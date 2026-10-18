//! The LunchFlow adapter end to end on an in-process fixture transport
//! (personal-cfo-r2pow): link, discovery, refresh windows, error triage,
//! partial failures, and the no-key-leak rule. No network.

use std::collections::BTreeMap;
use std::sync::Mutex;

use chrono::NaiveDate;
use connector_core::{
    Connection, ConnectorAdapter, ConnectorError, Credential, HealthStatus, LinkInput, LinkSession,
};
use lunchflow_adapter::transport::{HttpResponse, Transport, TransportError};
use lunchflow_adapter::{LunchFlowAdapter, BASE_URL};

const KEY: &str = "lf-CFO-CANARY-9f2d7c1e";

/// Serves a response per request path and records every request. Unknown
/// paths answer 404, like the provider.
#[derive(Default)]
struct FixtureTransport {
    routes: BTreeMap<String, (u16, String)>,
    requests: Mutex<Vec<Recorded>>,
}

#[derive(Debug, Clone)]
struct Recorded {
    path: String,
    api_key: String,
    query: Vec<(String, String)>,
}

impl FixtureTransport {
    fn with(routes: &[(&str, u16, &str)]) -> Self {
        Self {
            routes: routes
                .iter()
                .map(|(path, status, body)| ((*path).to_owned(), (*status, (*body).to_owned())))
                .collect(),
            requests: Mutex::new(Vec::new()),
        }
    }

    /// Both fixture accounts with their transactions and balances.
    fn standard() -> Self {
        Self::with(&[
            ("/accounts", 200, include_str!("fixtures/accounts.json")),
            (
                "/accounts/101/transactions",
                200,
                include_str!("fixtures/transactions_101.json"),
            ),
            (
                "/accounts/101/balance",
                200,
                include_str!("fixtures/balance_101.json"),
            ),
            (
                "/accounts/102/transactions",
                200,
                include_str!("fixtures/transactions_102.json"),
            ),
            (
                "/accounts/102/balance",
                200,
                include_str!("fixtures/balance_102.json"),
            ),
        ])
    }

    fn recorded(&self) -> Vec<Recorded> {
        self.requests.lock().unwrap().clone()
    }

    fn query_for(&self, path: &str) -> BTreeMap<String, String> {
        self.recorded()
            .into_iter()
            .find(|r| r.path == path)
            .map(|r| r.query.into_iter().collect())
            .unwrap_or_default()
    }
}

impl Transport for FixtureTransport {
    fn get(
        &self,
        url: &str,
        api_key: &str,
        query: &[(String, String)],
    ) -> Result<HttpResponse, TransportError> {
        let path = url
            .strip_prefix(BASE_URL)
            .ok_or_else(|| TransportError(format!("request left the pinned base: {url}")))?;
        self.requests.lock().unwrap().push(Recorded {
            path: path.to_owned(),
            api_key: api_key.to_owned(),
            query: query.to_vec(),
        });
        let (status, body) = self.routes.get(path).cloned().unwrap_or((
            404,
            r#"{"error":"Not Found","message":"no such account"}"#.to_owned(),
        ));
        Ok(HttpResponse { status, body })
    }
}

/// 2026-09-27T12:00:00Z — the injected clock.
fn fixed_now() -> i64 {
    1_790_510_400
}

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 9, 27).unwrap()
}

fn adapter(transport: &FixtureTransport) -> LunchFlowAdapter<&FixtureTransport> {
    LunchFlowAdapter::new(transport, fixed_now)
}

fn conn() -> Connection {
    Connection {
        credential: Credential::new(KEY),
    }
}

fn link_with(transport: &FixtureTransport, pasted: &str) -> Result<LinkSession, ConnectorError> {
    adapter(transport).link(&LinkInput {
        user_token: Some(Credential::new(pasted)),
        params: BTreeMap::new(),
    })
}

// --- link -------------------------------------------------------------------

#[test]
fn a_valid_key_links_after_one_authenticated_call() {
    let transport = FixtureTransport::standard();
    let LinkSession::Established {
        credential,
        display_hint,
    } = link_with(&transport, KEY).unwrap()
    else {
        panic!("user-token tier links directly");
    };
    assert_eq!(credential.expose_secret(), KEY);
    assert_eq!(
        display_hint.as_deref(),
        Some("LunchFlow connection, 2 accounts")
    );
    let requests = transport.recorded();
    assert_eq!(requests.len(), 1, "one validating request");
    assert_eq!(requests[0].path, "/accounts");
    assert_eq!(requests[0].api_key, KEY, "the key travels in the header");
}

#[test]
fn a_whitespace_padded_paste_is_accepted() {
    let transport = FixtureTransport::standard();
    let padded = format!("  {}\n{}\t ", &KEY[..6], &KEY[6..]);
    let LinkSession::Established { credential, .. } = link_with(&transport, &padded).unwrap()
    else {
        panic!("links");
    };
    assert_eq!(credential.expose_secret(), KEY);
    assert_eq!(transport.recorded()[0].api_key, KEY);
}

#[test]
fn a_bad_or_revoked_key_is_a_user_action_at_link_time() {
    // A wrong key answers 403 live (2026-09-27); a missing one 401.
    for (status, code) in [(401, "key.invalid"), (403, "key.invalid")] {
        let transport = FixtureTransport::with(&[(
            "/accounts",
            status,
            include_str!("fixtures/unauthorized.json"),
        )]);
        match link_with(&transport, KEY) {
            Err(ConnectorError::NeedsUserAction { code: got, .. }) => {
                assert_eq!(got, code, "status {status}");
            }
            other => panic!("status {status}: expected NeedsUserAction, got {other:?}"),
        }
    }
}

#[test]
fn a_missing_or_blank_key_never_touches_the_network() {
    let transport = FixtureTransport::standard();
    let blank = link_with(&transport, " \n ");
    assert!(matches!(
        blank,
        Err(ConnectorError::NeedsUserAction { ref code, .. }) if code == "key.missing"
    ));
    let missing = adapter(&transport).link(&LinkInput::default());
    assert!(matches!(
        missing,
        Err(ConnectorError::NeedsUserAction { .. })
    ));
    assert!(transport.recorded().is_empty());
}

// --- discovery --------------------------------------------------------------

#[test]
fn accounts_carry_stable_ids_names_and_their_native_currency() {
    let transport = FixtureTransport::standard();
    let accounts = adapter(&transport).fetch_accounts(&conn()).unwrap();
    assert_eq!(accounts.len(), 2);
    assert_eq!(accounts[0].external_id.as_deref(), Some("101"));
    assert_eq!(
        accounts[0].external_name.as_deref(),
        Some("Fixture Bank Everyday Checking")
    );
    // Account 101 states no currency (as live accounts don't): it takes its
    // balance's, at the cost of one request.
    assert_eq!(accounts[0].currency.as_deref(), Some("USD"));
    let paths: Vec<String> = transport.recorded().into_iter().map(|r| r.path).collect();
    assert_eq!(paths, ["/accounts", "/accounts/101/balance"]);
    // A currency the app cannot hold yet is still reported, raw, so the
    // mapping surface can refuse it (ADR 0076 decision 7).
    assert_eq!(accounts[1].currency.as_deref(), Some("GBP"));
    // LunchFlow reports no account type: nothing is guessed.
    assert!(accounts.iter().all(|a| a.proposed_subtype.is_none()));
}

#[test]
fn a_changed_response_shape_fails_loudly_instead_of_reading_as_empty() {
    // A 200 that lacks the documented array must never look like "no
    // accounts" or "no transactions" — that would link a key that sees
    // nothing, or silently skip a refresh window.
    let transport =
        FixtureTransport::with(&[("/accounts", 200, r#"{"data": [{"id": 1}], "total": 1}"#)]);
    assert!(matches!(
        link_with(&transport, KEY),
        Err(ConnectorError::Provider(_))
    ));

    let transport = FixtureTransport::with(&[
        ("/accounts", 200, include_str!("fixtures/accounts.json")),
        (
            "/accounts/101/transactions",
            200,
            r#"{"items": [], "total": 0}"#,
        ),
    ]);
    assert!(adapter(&transport)
        .fetch_transactions(&conn(), "101", None)
        .is_err());
}

// --- transactions -----------------------------------------------------------

#[test]
fn transactions_carry_provider_ids_one_date_and_the_ledger_sign() {
    let transport = FixtureTransport::standard();
    let records = adapter(&transport)
        .fetch_transactions(&conn(), "101", None)
        .unwrap();
    assert_eq!(records.len(), 3);
    for record in &records {
        assert!(
            record.external_id.is_some(),
            "every record has its provider id"
        );
        let txn = record.transaction.as_ref().unwrap();
        assert!(txn.transaction_date.is_none(), "the API carries one date");
        assert!(txn.txn_fingerprint.contains("|lflow:"));
    }
    let by_id = |id: &str| {
        records
            .iter()
            .find(|r| r.external_id.as_deref() == Some(id))
            .and_then(|r| r.transaction.clone())
            .unwrap()
    };
    let grocer = by_id("txn_a1");
    assert_eq!(grocer.amount.minor_units(), -4510, "outflow stays negative");
    assert_eq!(
        grocer.posted_date,
        NaiveDate::from_ymd_opt(2026, 9, 20).unwrap()
    );
    assert_eq!(grocer.description.as_deref(), Some("FIXTURE GROCER #12"));
    assert_eq!(
        by_id("txn_a2").amount.minor_units(),
        250_000,
        "inflow positive"
    );
    let cafe = by_id("txn_a3");
    assert_eq!(cafe.amount.minor_units(), -350);
    assert_eq!(
        cafe.description.as_deref(),
        Some("Fixture Cafe"),
        "an empty description falls back to the merchant"
    );
}

#[test]
fn the_first_refresh_asks_for_full_history_and_later_ones_overlap() {
    let transport = FixtureTransport::standard();
    adapter(&transport)
        .fetch_transactions(&conn(), "101", None)
        .unwrap();
    let first = transport.query_for("/accounts/101/transactions");
    assert_eq!(first["from"], "2024-09-27", "730-day lookback");
    assert_eq!(first["to"], today().to_string());
    assert_eq!(first["include_pending"], "false", "posted-only");

    let transport = FixtureTransport::standard();
    let since = NaiveDate::from_ymd_opt(2026, 9, 10).unwrap();
    adapter(&transport)
        .fetch_transactions(&conn(), "101", Some(since))
        .unwrap();
    let later = transport.query_for("/accounts/101/transactions");
    assert_eq!(later["from"], "2026-09-05", "since rewound five days");
}

#[test]
fn rows_that_cannot_be_staged_are_reported_never_silent() {
    let odd = r#"{"transactions": [
        {"id": "p1", "amount": -1.00, "currency": "USD", "date": "2026-09-20", "isPending": true},
        {"id": "z1", "amount": 0, "currency": "USD", "date": "2026-09-20"},
        {"id": "f1", "amount": -1.005, "currency": "USD", "date": "2026-09-20"},
        {"id": "e1", "amount": 1e2, "currency": "USD", "date": "2026-09-20"},
        {"id": "c1", "amount": -1.00, "currency": "EUR", "date": "2026-09-20"},
        {"id": "d1", "amount": -1.00, "currency": "USD", "date": "20/09/2026"},
        {"id": "ok", "amount": -1.00, "currency": "USD", "date": "2026-09-20"},
        {"id": "ok", "amount": -1.00, "currency": "USD", "date": "2026-09-20"}
    ], "total": 8}"#;
    let transport = FixtureTransport::with(&[
        ("/accounts", 200, include_str!("fixtures/accounts.json")),
        ("/accounts/101/transactions", 200, odd),
        (
            "/accounts/101/balance",
            200,
            include_str!("fixtures/balance_101.json"),
        ),
        (
            "/accounts/102/transactions",
            200,
            include_str!("fixtures/transactions_102.json"),
        ),
        (
            "/accounts/102/balance",
            200,
            include_str!("fixtures/balance_102.json"),
        ),
    ]);
    let synced = adapter(&transport).sync(&conn(), None).unwrap();
    let warnings: Vec<&str> = synced
        .batch
        .warnings
        .iter()
        .map(|w| w.message.as_str())
        .collect();
    let has = |needle: &str| warnings.iter().any(|w| w.contains(needle));
    assert!(has("pending transaction p1"), "{warnings:?}");
    assert!(has("zero-amount transaction z1"), "{warnings:?}");
    assert!(
        has("transaction f1 has an unparseable amount"),
        "{warnings:?}"
    );
    assert!(
        has("transaction e1 has an unparseable amount"),
        "{warnings:?}"
    );
    assert!(
        has("transaction c1 is in EUR, not its account's USD"),
        "{warnings:?}"
    );
    assert!(has("transaction d1 has no usable date"), "{warnings:?}");
    // The GBP account's rows and balance can't be held yet: warned, not staged.
    assert!(
        has("transaction txn_b1 uses unsupported currency GBP"),
        "{warnings:?}"
    );
    assert!(
        has("balance for account 102 uses unsupported currency GBP"),
        "{warnings:?}"
    );
    let staged_ids: Vec<&str> = synced
        .batch
        .records
        .iter()
        .filter(|r| r.transaction.is_some())
        .filter_map(|r| r.external_id.as_deref())
        .collect();
    assert_eq!(staged_ids, ["ok"], "the duplicate id stages once");
}

#[test]
fn a_truncated_response_holds_that_accounts_watermark() {
    let truncated = r#"{"transactions": [
        {"id": "t1", "amount": -1.00, "currency": "USD", "date": "2026-09-20"}
    ], "total": 500}"#;
    let transport = FixtureTransport::with(&[
        ("/accounts", 200, include_str!("fixtures/accounts.json")),
        ("/accounts/101/transactions", 200, truncated),
        (
            "/accounts/101/balance",
            200,
            include_str!("fixtures/balance_101.json"),
        ),
        (
            "/accounts/102/transactions",
            200,
            include_str!("fixtures/transactions_102.json"),
        ),
        (
            "/accounts/102/balance",
            200,
            include_str!("fixtures/balance_102.json"),
        ),
    ]);
    let synced = adapter(&transport).sync(&conn(), None).unwrap();
    assert_eq!(synced.held_account_ids, ["101"]);
    assert!(synced
        .batch
        .warnings
        .iter()
        .any(|w| w.message.contains("only part of account 101")));
}

#[test]
fn a_balance_currency_override_never_relabels_the_account() {
    // LunchFlow lets a user display an account's balance in another currency
    // (Configure > Balance). The transactions stay in the account's real
    // currency, so they decide it; the overridden balance is reported and
    // left out rather than anchoring the account.
    let transport = FixtureTransport::with(&[
        ("/accounts", 200, include_str!("fixtures/accounts.json")),
        (
            "/accounts/101/transactions",
            200,
            include_str!("fixtures/transactions_101.json"),
        ),
        (
            "/accounts/101/balance",
            200,
            r#"{ "balance": { "amount": 1100.00, "currency": "EUR" } }"#,
        ),
        (
            "/accounts/102/transactions",
            200,
            include_str!("fixtures/transactions_102.json"),
        ),
        (
            "/accounts/102/balance",
            200,
            include_str!("fixtures/balance_102.json"),
        ),
    ]);
    let synced = adapter(&transport).sync(&conn(), None).unwrap();
    assert_eq!(synced.batch.accounts[0].currency.as_deref(), Some("USD"));
    let staged_txns = synced
        .batch
        .records
        .iter()
        .filter(|r| r.transaction.is_some())
        .count();
    assert_eq!(staged_txns, 3, "the USD rows all stage");
    assert!(
        !synced.batch.records.iter().any(|r| r.balance.is_some()),
        "the EUR-labelled balance does not stage"
    );
    assert!(synced.batch.warnings.iter().any(|w| w
        .message
        .contains("is in EUR, but its transactions are in USD")));
}

// --- balances ---------------------------------------------------------------

#[test]
fn balances_are_dated_the_refresh_day() {
    let transport = FixtureTransport::standard();
    let balances = adapter(&transport).fetch_balances(&conn(), "101").unwrap();
    assert_eq!(balances.len(), 1);
    assert_eq!(balances[0].observed_at, today());
    assert_eq!(balances[0].amount.minor_units(), 123_456);
    assert_eq!(balances[0].external_account.as_deref(), Some("101"));
}

// --- refresh ----------------------------------------------------------------

#[test]
fn a_refresh_stages_every_account_with_provenance() {
    let transport = FixtureTransport::standard();
    let synced = adapter(&transport).sync(&conn(), None).unwrap();
    assert_eq!(synced.adapter_id, "lunchflow");
    assert_eq!(synced.adapter_version, "0.1.0");
    assert_eq!(synced.batch.source_format, "lunchflow");
    assert_eq!(synced.batch.accounts.len(), 2);
    let txns = synced
        .batch
        .records
        .iter()
        .filter(|r| r.transaction.is_some())
        .count();
    let balances = synced
        .batch
        .records
        .iter()
        .filter(|r| r.balance.is_some())
        .count();
    assert_eq!(txns, 3, "account 101's rows; the GBP row is warned");
    assert_eq!(balances, 1, "account 101's balance; the GBP one is warned");
    assert!(synced.held_account_ids.is_empty());
    let mut hashes: Vec<&str> = synced
        .batch
        .records
        .iter()
        .map(|r| r.source_hash.as_str())
        .collect();
    hashes.sort_unstable();
    hashes.dedup();
    assert_eq!(
        hashes.len(),
        synced.batch.records.len(),
        "unique source hashes"
    );
    assert_eq!(transport.recorded().len(), 5, "1 + 2 per account");
}

#[test]
fn one_accounts_failure_holds_it_while_the_others_stage() {
    let transport = FixtureTransport::with(&[
        ("/accounts", 200, include_str!("fixtures/accounts.json")),
        ("/accounts/101/transactions", 500, "{}"),
        (
            "/accounts/101/balance",
            200,
            include_str!("fixtures/balance_101.json"),
        ),
        (
            "/accounts/102/transactions",
            200,
            include_str!("fixtures/transactions_102.json"),
        ),
        (
            "/accounts/102/balance",
            200,
            include_str!("fixtures/balance_102.json"),
        ),
    ]);
    let synced = adapter(&transport).sync(&conn(), None).unwrap();
    assert_eq!(synced.held_account_ids, ["101"]);
    assert!(synced.batch.warnings.iter().any(|w| w
        .message
        .contains("transactions for account 101 were not refreshed")));
    assert!(
        synced.batch.records.iter().any(|r| r.balance.is_some()),
        "account 101's balance still stages"
    );
}

#[test]
fn a_refused_key_or_throttle_mid_refresh_aborts_it() {
    for (status, expect_rate_limited) in [(401, false), (429, true)] {
        let transport = FixtureTransport::with(&[
            ("/accounts", 200, include_str!("fixtures/accounts.json")),
            ("/accounts/101/transactions", status, "{}"),
        ]);
        let err = adapter(&transport).sync(&conn(), None).unwrap_err();
        if expect_rate_limited {
            assert!(matches!(err, ConnectorError::RateLimited(_)), "{err:?}");
        } else {
            assert!(matches!(err, ConnectorError::Expired(_)), "{err:?}");
        }
    }
}

#[test]
fn an_inactive_account_is_warned_held_and_flagged_by_health() {
    let accounts = include_str!("fixtures/accounts.json").replacen(
        r#""status": "ACTIVE""#,
        r#""status": "DISCONNECTED""#,
        1,
    );
    let transport = FixtureTransport::with(&[
        ("/accounts", 200, &accounts),
        (
            "/accounts/101/transactions",
            200,
            include_str!("fixtures/transactions_101.json"),
        ),
        (
            "/accounts/101/balance",
            200,
            include_str!("fixtures/balance_101.json"),
        ),
        (
            "/accounts/102/transactions",
            200,
            include_str!("fixtures/transactions_102.json"),
        ),
        (
            "/accounts/102/balance",
            200,
            include_str!("fixtures/balance_102.json"),
        ),
    ]);
    let synced = adapter(&transport).sync(&conn(), None).unwrap();
    assert_eq!(synced.held_account_ids, ["101"]);
    assert!(synced.batch.warnings.iter().any(|w| w
        .message
        .contains("reconnect it in your LunchFlow dashboard")));
    assert!(matches!(
        adapter(&transport).health(&conn()).unwrap(),
        HealthStatus::NeedsUserAction { ref code, .. } if code == "lunchflow.account_status"
    ));
}

// --- health + revoke ---------------------------------------------------------

#[test]
fn health_maps_the_taxonomy_and_a_throttle_is_healthy_not_broken() {
    let health = |status: u16, body: &str| {
        let transport = FixtureTransport::with(&[("/accounts", status, body)]);
        adapter(&transport).health(&conn()).unwrap()
    };
    assert_eq!(
        health(200, include_str!("fixtures/accounts.json")),
        HealthStatus::Healthy
    );
    assert_eq!(health(429, "{}"), HealthStatus::RateLimited);
    assert_eq!(
        health(401, include_str!("fixtures/unauthorized.json")),
        HealthStatus::Expired
    );
    assert_eq!(health(403, "{}"), HealthStatus::Expired);
    assert!(matches!(
        health(503, "{}"),
        HealthStatus::Unreachable { .. }
    ));
}

#[test]
fn revoke_is_a_local_forget_with_no_request() {
    let transport = FixtureTransport::standard();
    adapter(&transport).revoke(&conn()).unwrap();
    assert!(transport.recorded().is_empty());
}

// --- leak rule + registration ------------------------------------------------

#[test]
fn nothing_the_adapter_emits_contains_the_key() {
    let transport = FixtureTransport::standard();
    let synced = adapter(&transport).sync(&conn(), None).unwrap();
    let rendered = format!("{synced:?}");
    assert!(
        !rendered.contains("CFO-CANARY"),
        "key leaked into the batch"
    );

    let session = link_with(&transport, KEY).unwrap();
    assert!(!format!("{session:?}").contains("CFO-CANARY"));
    // Requests carry the key only in the header, never in a path or query.
    let recorded = transport.recorded();
    assert!(recorded.len() > 1);
    for request in recorded {
        assert!(!request.path.contains("CFO-CANARY"));
        assert!(!request.query.iter().any(|(_, v)| v.contains("CFO-CANARY")));
    }

    // A provider that ECHOES the key in its error body, at every status the
    // adapter triages — on discovery, link, health, and the balance and
    // transactions paths (review F1, 2026-09-27).
    let echo = format!(r#"{{"error":"Rejected","message":"key {KEY} is not valid"}}"#);
    for status in [300, 302, 400, 401, 403, 404, 409, 429, 500, 502, 503] {
        let transport = FixtureTransport::with(&[("/accounts", status, &echo)]);
        let err = adapter(&transport).fetch_accounts(&conn()).unwrap_err();
        assert!(
            !format!("{err} {err:?}").contains("CFO-CANARY"),
            "status {status}"
        );
        let link = link_with(&transport, KEY).unwrap_err();
        assert!(
            !format!("{link} {link:?}").contains("CFO-CANARY"),
            "link {status}"
        );
        let health = adapter(&transport).health(&conn());
        assert!(
            !format!("{health:?}").contains("CFO-CANARY"),
            "health {status}"
        );

        let per_account = FixtureTransport::with(&[
            ("/accounts", 200, include_str!("fixtures/accounts.json")),
            ("/accounts/101/balance", status, &echo),
            ("/accounts/101/transactions", status, &echo),
            ("/accounts/102/balance", status, &echo),
            ("/accounts/102/transactions", status, &echo),
        ]);
        let refreshed = format!("{:?}", adapter(&per_account).sync(&conn(), None));
        assert!(
            !refreshed.contains("CFO-CANARY"),
            "sync {status}: {refreshed}"
        );
        for result in [
            format!("{:?}", adapter(&per_account).fetch_balances(&conn(), "101")),
            format!(
                "{:?}",
                adapter(&per_account).fetch_transactions(&conn(), "101", None)
            ),
        ] {
            assert!(!result.contains("CFO-CANARY"), "status {status}: {result}");
        }
    }
}

/// No run of 8 or more characters of the key survives in `text`.
fn assert_no_key_fragment(text: &str) {
    let key: Vec<char> = KEY.chars().collect();
    for window in key.windows(8) {
        let fragment: String = window.iter().collect();
        assert!(
            !text.contains(&fragment),
            "fragment {fragment:?} leaked: {text}"
        );
    }
}

#[test]
fn provider_ids_echoing_the_key_never_reach_warnings_or_stored_ids() {
    // personal-cfo-pxi.4: ids and statuses are provider-controlled. An
    // account id that embeds the key, plainly or hidden behind zero-width
    // characters, is refused (ids are used verbatim, so they can't be
    // scrubbed). A status that quotes an obfuscated key across the
    // 200-character cut is quoted in a warning, and never leaks.
    let hidden: String = KEY.chars().flat_map(|c| [c, '\u{200B}']).collect();
    let straddling = format!("{}{hidden}{}", "x".repeat(195), "y".repeat(20));
    let accounts = serde_json::json!({
        "accounts": [
            { "id": format!("acct-{KEY}"), "name": "Echo", "status": "ACTIVE" },
            { "id": format!("zw-{hidden}"), "name": "Hidden", "status": "ACTIVE" },
            { "id": "acct-3", "name": "Checking", "status": straddling }
        ],
        "total": 3
    })
    .to_string();
    // Per-account paths are unrouted, so they 404 and their warnings quote ids.
    let transport = FixtureTransport::with(&[("/accounts", 200, &accounts)]);
    let synced = adapter(&transport).sync(&conn(), None).unwrap();

    let messages: Vec<&str> = synced
        .batch
        .warnings
        .iter()
        .map(|w| w.message.as_str())
        .collect();
    assert!(
        messages
            .iter()
            .any(|m| m.contains("2 account(s) from LunchFlow had an id containing your API key")),
        "{messages:?}"
    );
    assert_eq!(
        synced.batch.accounts.len(),
        1,
        "both key-bearing accounts are refused"
    );
    assert!(
        messages
            .iter()
            .any(|m| m.contains("reconnect it in your LunchFlow dashboard")),
        "the long status is quoted: {messages:?}"
    );
    assert_no_key_fragment(&format!("{synced:?}"));

    // Discovery refuses it too.
    let discovered = adapter(&transport).fetch_accounts(&conn()).unwrap();
    assert_eq!(discovered.len(), 1);
    assert_no_key_fragment(&format!("{discovered:?}"));
}

#[test]
fn a_transaction_id_bearing_the_key_is_refused_not_stored() {
    let rows = serde_json::json!({
        "transactions": [
            { "id": format!("t-{KEY}"), "amount": -1.00, "currency": "USD", "date": "2026-09-20" },
            { "id": "ok-1", "amount": -2.00, "currency": "USD", "date": "2026-09-20" }
        ],
        "total": 2
    })
    .to_string();
    let transport = FixtureTransport::with(&[
        ("/accounts", 200, include_str!("fixtures/accounts.json")),
        ("/accounts/101/transactions", 200, &rows),
        (
            "/accounts/101/balance",
            200,
            include_str!("fixtures/balance_101.json"),
        ),
    ]);
    let synced = adapter(&transport).sync(&conn(), None).unwrap();
    let staged: Vec<&str> = synced
        .batch
        .records
        .iter()
        .filter(|r| r.transaction.is_some())
        .filter_map(|r| r.external_id.as_deref())
        .collect();
    assert_eq!(staged, ["ok-1"]);
    assert!(synced
        .batch
        .warnings
        .iter()
        .any(|w| w.message.contains("id contains your API key")));
    assert_no_key_fragment(&format!("{synced:?}"));
}

#[test]
fn multi_megabyte_provider_ids_are_refused_quickly_and_never_quoted() {
    // personal-cfo-pxi.6: an absurdly long id costs a bounded amount of
    // work, is refused like a key-bearing one, and the warning quotes none
    // of it.
    let huge_account = "q".repeat(3_000_000);
    let huge_txn = "r".repeat(3_000_000);
    let accounts = serde_json::json!({
        "accounts": [
            { "id": huge_account, "name": "Huge", "status": "ACTIVE" },
            { "id": 101, "name": "Everyday Checking", "status": "ACTIVE" }
        ],
        "total": 2
    })
    .to_string();
    let rows = serde_json::json!({
        "transactions": [
            { "id": huge_txn, "amount": -1.00, "currency": "USD", "date": "2026-09-20" },
            { "id": "ok-1", "amount": -2.00, "currency": "USD", "date": "2026-09-20" }
        ],
        "total": 2
    })
    .to_string();
    let transport = FixtureTransport::with(&[
        ("/accounts", 200, &accounts),
        ("/accounts/101/transactions", 200, &rows),
        (
            "/accounts/101/balance",
            200,
            include_str!("fixtures/balance_101.json"),
        ),
    ]);

    let started = std::time::Instant::now();
    let synced = adapter(&transport).sync(&conn(), None).unwrap();
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "took {:?}",
        started.elapsed()
    );

    assert_eq!(
        synced.batch.accounts.len(),
        1,
        "the huge account id is refused"
    );
    let staged: Vec<&str> = synced
        .batch
        .records
        .iter()
        .filter(|r| r.transaction.is_some())
        .filter_map(|r| r.external_id.as_deref())
        .collect();
    assert_eq!(staged, ["ok-1"], "the huge transaction id is refused");
    let messages: Vec<&str> = synced
        .batch
        .warnings
        .iter()
        .map(|w| w.message.as_str())
        .collect();
    assert!(
        messages
            .contains(&"1 account(s) from LunchFlow had an id too long to use and were skipped"),
        "{messages:?}"
    );
    assert!(
        messages.contains(&"a transaction with an id too long to use was not staged"),
        "{messages:?}"
    );
    for message in &messages {
        assert!(
            !message.contains("qqqq") && !message.contains("rrrr"),
            "{message}"
        );
    }

    // Discovery refuses the huge account id too.
    assert_eq!(
        adapter(&transport).fetch_accounts(&conn()).unwrap().len(),
        1
    );
}

#[test]
fn the_adapter_is_registered_disabled_under_its_schema_token() {
    let registration =
        connector_core::registration_by_id("lunchflow").expect("lunchflow registered");
    assert_eq!(registration.adapter.display_name(), "LunchFlow");
    assert!(!registration.metadata.enabled);
    assert!(!registration.adapter.capabilities().holdings);
}
