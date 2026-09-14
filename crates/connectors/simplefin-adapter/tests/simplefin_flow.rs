//! The SimpleFIN adapter end to end on a fixture transport (personal-cfo-w3gh):
//! claim flow (incl. the single-use 403), chunked sync with overlap dedupe,
//! error triage, the 1k-transaction batch, and the no-credential-leak rule.

use std::sync::Mutex;

use chrono::NaiveDate;
use connector_core::{
    Connection, ConnectorAdapter, ConnectorError, Credential, HealthStatus, LinkInput, LinkSession,
};
use simplefin_adapter::transport::{HttpResponse, Transport, TransportError};
use simplefin_adapter::SimpleFinAdapter;

const ACCESS_URL: &str = "https://demo:CFO-CANARY-9f2d7c1e@bridge.example/simplefin";

/// Serves queued responses and records every request for assertions.
#[derive(Default)]
struct FixtureTransport {
    responses: Mutex<Vec<HttpResponse>>,
    requests: Mutex<Vec<RecordedRequest>>,
}

#[derive(Debug, Clone)]
struct RecordedRequest {
    method: &'static str,
    url: String,
    basic_user: Option<String>,
    query: Vec<(String, String)>,
}

impl FixtureTransport {
    fn queue(responses: Vec<(u16, String)>) -> Self {
        let mut list: Vec<HttpResponse> = responses
            .into_iter()
            .map(|(status, body)| HttpResponse { status, body })
            .collect();
        list.reverse(); // pop() serves in queue order
        Self {
            responses: Mutex::new(list),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn recorded(&self) -> Vec<RecordedRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl Transport for FixtureTransport {
    fn post_empty(&self, url: &str) -> Result<HttpResponse, TransportError> {
        self.requests.lock().unwrap().push(RecordedRequest {
            method: "POST",
            url: url.to_owned(),
            basic_user: None,
            query: Vec::new(),
        });
        self.responses
            .lock()
            .unwrap()
            .pop()
            .ok_or_else(|| TransportError("fixture exhausted".to_owned()))
    }

    fn get(
        &self,
        url: &str,
        basic: Option<(&str, &str)>,
        query: &[(String, String)],
    ) -> Result<HttpResponse, TransportError> {
        self.requests.lock().unwrap().push(RecordedRequest {
            method: "GET",
            url: url.to_owned(),
            basic_user: basic.map(|(u, _)| u.to_owned()),
            query: query.to_vec(),
        });
        self.responses
            .lock()
            .unwrap()
            .pop()
            .ok_or_else(|| TransportError("fixture exhausted".to_owned()))
    }
}

fn fixed_now() -> i64 {
    NaiveDate::from_ymd_opt(2026, 8, 22)
        .unwrap()
        .and_hms_opt(12, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp()
}

fn mk_adapter(transport: &FixtureTransport) -> SimpleFinAdapter<&FixtureTransport> {
    SimpleFinAdapter::new(transport, fixed_now)
}

fn conn() -> Connection {
    Connection {
        credential: Credential::new(ACCESS_URL),
    }
}

fn account_json(id: &str, txns: &str) -> String {
    format!(
        r#"{{"id":"{id}","name":"Checking","currency":"USD","balance":"100.23",
            "available-balance":"100.23","balance-date":1755000000,
            "org":{{"domain":"demo.bank","name":"Demo Bank"}},
            "holdings":[],"transactions":[{txns}]}}"#
    )
}

fn txn_json(id: &str, posted: i64, amount: &str) -> String {
    format!(
        r#"{{"id":"{id}","posted":{posted},"amount":"{amount}",
            "description":"Purchase {id}","payee":"X","mcc":"5812"}}"#
    )
}

fn set_json(accounts: &[String]) -> String {
    format!(r#"{{"errors":[],"accounts":[{}]}}"#, accounts.join(","))
}

// ===========================================================================
// Claim flow
// ===========================================================================

#[test]
fn a_valid_setup_token_claims_an_access_url() {
    use base64::Engine as _;
    let token =
        base64::engine::general_purpose::STANDARD.encode("https://bridge.example/claim/ONE-TIME");
    let transport = FixtureTransport::queue(vec![(200, format!("{ACCESS_URL}\n"))]);
    let adapter = mk_adapter(&transport);

    let session = adapter
        .link(&LinkInput {
            user_token: Some(Credential::new(format!("  {token}  "))), // pasted with whitespace
            ..LinkInput::default()
        })
        .unwrap();

    let LinkSession::Established { credential, .. } = session else {
        panic!("user-token tier must establish directly");
    };
    assert_eq!(credential.expose_secret(), ACCESS_URL);

    let requests = transport.recorded();
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].url, "https://bridge.example/claim/ONE-TIME");
}

#[test]
fn a_reused_setup_token_is_the_re_link_prompt() {
    use base64::Engine as _;
    let token =
        base64::engine::general_purpose::STANDARD.encode("https://bridge.example/claim/USED");
    let transport = FixtureTransport::queue(vec![(
        403,
        "Forbidden (was it already claimed?)".to_owned(),
    )]);
    let err = mk_adapter(&transport)
        .link(&LinkInput {
            user_token: Some(Credential::new(token)),
            ..LinkInput::default()
        })
        .unwrap_err();
    assert!(
        matches!(&err, ConnectorError::NeedsUserAction { code, .. } if code == "setup_token_used")
    );
}

#[test]
fn garbage_tokens_fail_without_touching_the_network() {
    let transport = FixtureTransport::queue(vec![]);
    let adapter = mk_adapter(&transport);
    for bad in ["not base64!!!", "aHR0cDovL2luc2VjdXJlLmV4YW1wbGU="] {
        // second: base64 of an http:// URL
        let err = adapter
            .link(&LinkInput {
                user_token: Some(Credential::new(bad)),
                ..LinkInput::default()
            })
            .unwrap_err();
        assert!(
            matches!(&err, ConnectorError::NeedsUserAction { code, .. } if code == "token.invalid")
        );
    }
    assert!(transport.recorded().is_empty());
}

// ===========================================================================
// Sync
// ===========================================================================

#[test]
fn sync_maps_accounts_transactions_and_one_balance_per_account() {
    let body = set_json(&[account_json(
        "ACT-1",
        &[
            txn_json("TXN-1", 1_755_000_000, "-5.00"),
            txn_json("TXN-2", 1_755_100_000, "1250.00"),
        ]
        .join(","),
    )]);
    let since = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap(); // single chunk
    let transport = FixtureTransport::queue(vec![(200, body)]);
    let adapter = mk_adapter(&transport);
    let synced = adapter.sync(&conn(), Some(since)).unwrap();

    assert_eq!(synced.adapter_id, "simplefin");
    assert_eq!(synced.batch.source_format, "simplefin");
    assert_eq!(synced.batch.accounts.len(), 1);
    let account = &synced.batch.accounts[0];
    assert_eq!(account.external_id.as_deref(), Some("ACT-1"));
    assert_eq!(account.external_name.as_deref(), Some("Demo Bank Checking"));

    let txns: Vec<_> = synced
        .batch
        .records
        .iter()
        .filter_map(|r| r.transaction.as_ref())
        .collect();
    assert_eq!(txns.len(), 2);
    assert_eq!(txns[0].amount.minor_units(), -500);
    assert_eq!(txns[0].external_account.as_deref(), Some("ACT-1"));
    assert!(txns[0].txn_fingerprint.ends_with("|sfin:TXN-1"));

    let balances: Vec<_> = synced
        .batch
        .records
        .iter()
        .filter_map(|r| r.balance.as_ref())
        .collect();
    assert_eq!(balances.len(), 1, "one balance per account, not per chunk");
    assert_eq!(balances[0].amount.minor_units(), 10_023);

    // The request carried Basic auth split from the URL — no userinfo in it.
    let requests = transport.recorded();
    assert_eq!(requests[0].url, "https://bridge.example/simplefin/accounts");
    assert_eq!(requests[0].basic_user.as_deref(), Some("demo"));
    assert!(requests[0]
        .query
        .iter()
        .any(|(k, v)| k == "version" && v == "2"));
}

#[test]
fn overlapping_chunks_dedupe_by_account_and_txn_id() {
    // ~180-day range → 3 chunks; the same transaction appears in each.
    let since = NaiveDate::from_ymd_opt(2026, 2, 23).unwrap();
    let dup = account_json("ACT-1", &txn_json("TXN-DUP", 1_755_000_000, "-5.00"));
    let transport = FixtureTransport::queue(vec![
        (200, set_json(std::slice::from_ref(&dup))),
        (200, set_json(std::slice::from_ref(&dup))),
        (200, set_json(std::slice::from_ref(&dup))),
        (200, set_json(std::slice::from_ref(&dup))),
        (200, set_json(&[dup])),
    ]);
    let adapter = mk_adapter(&transport);
    let synced = adapter.sync(&conn(), Some(since)).unwrap();

    let txn_count = synced
        .batch
        .records
        .iter()
        .filter(|r| r.transaction.is_some())
        .count();
    assert_eq!(txn_count, 1, "overlap must dedupe by (account, txn id)");
    assert_eq!(synced.batch.accounts.len(), 1);
    assert_eq!(
        transport.recorded().len(),
        5,
        "five 45-day chunks over the span"
    );
}

#[test]
fn a_403_mid_sync_is_expired_and_a_gen_auth_aborts() {
    let since = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();

    let transport = FixtureTransport::queue(vec![(
        403,
        r#"{"errors":["Forbidden"],"accounts":[]}"#.to_owned(),
    )]);
    let err = mk_adapter(&transport)
        .sync(&conn(), Some(since))
        .unwrap_err();
    assert!(matches!(err, ConnectorError::Expired(_)));

    let transport = FixtureTransport::queue(vec![(
        200,
        r#"{"errlist":[{"code":"gen.auth","msg":"Forbidden"}],
            "accounts":[],"connections":[]}"#
            .to_owned(),
    )]);
    let err = mk_adapter(&transport)
        .sync(&conn(), Some(since))
        .unwrap_err();
    assert!(matches!(&err, ConnectorError::NeedsUserAction { code, .. } if code == "gen.auth"));
}

#[test]
fn a_con_auth_does_not_abort_sync_and_healthy_institutions_still_stage() {
    // One institution needs re-auth; the other's data must survive the sync.
    let since = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
    let body = format!(
        r#"{{"errlist":[{{"code":"con.auth","msg":"Reauthorize Demo Bank","conn_id":"C1"}}],
            "connections":[{{"conn_id":"C2","name":"Other Bank"}}],
            "accounts":[{}]}}"#,
        account_json("ACT-2", &txn_json("TXN-9", 1_755_000_000, "-7.00"))
    );
    let transport = FixtureTransport::queue(vec![(200, body)]);
    let synced = mk_adapter(&transport).sync(&conn(), Some(since)).unwrap();

    assert_eq!(
        synced
            .batch
            .records
            .iter()
            .filter(|r| r.transaction.is_some())
            .count(),
        1,
        "the healthy institution's data stages"
    );
    assert!(synced
        .batch
        .warnings
        .iter()
        .any(|w| w.message.contains("requires reauthorization")));
}

#[test]
fn a_redirect_is_never_misread_as_a_credential_failure() {
    // The transport is configured with redirects(0); a Bridge host migration
    // must surface as itself, not as Expired (which would tell the user to
    // re-link a perfectly good credential).
    let since = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
    let transport = FixtureTransport::queue(vec![(301, String::new())]);
    let err = mk_adapter(&transport)
        .sync(&conn(), Some(since))
        .unwrap_err();
    assert!(
        matches!(&err, ConnectorError::Provider(msg) if msg.contains("redirected")),
        "got {err:?}"
    );

    use base64::Engine as _;
    let token =
        base64::engine::general_purpose::STANDARD.encode("https://bridge.example/claim/MOVED");
    let transport = FixtureTransport::queue(vec![(302, String::new())]);
    let err = mk_adapter(&transport)
        .link(&LinkInput {
            user_token: Some(Credential::new(token)),
            ..LinkInput::default()
        })
        .unwrap_err();
    assert!(matches!(&err, ConnectorError::Provider(msg) if msg.contains("redirected")));
}

#[test]
fn a_429_is_the_healthy_rate_limited_state() {
    let since = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
    let transport = FixtureTransport::queue(vec![(429, String::new())]);
    let err = mk_adapter(&transport)
        .sync(&conn(), Some(since))
        .unwrap_err();
    assert!(matches!(err, ConnectorError::RateLimited(_)));

    let transport = FixtureTransport::queue(vec![(429, String::new())]);
    assert_eq!(
        mk_adapter(&transport).health(&conn()).unwrap(),
        HealthStatus::RateLimited
    );
}

#[test]
fn v2_accounts_are_connection_scoped_and_named_from_their_connection() {
    // Two institutions expose the SAME account id — they must stay distinct.
    let since = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
    let body = r#"{"errlist":[],
        "connections":[{"conn_id":"C1","name":"First Bank"},
                       {"conn_id":"C2","name":"Second Bank"}],
        "accounts":[
          {"id":"ACT-1","conn_id":"C1","name":"Checking","currency":"USD",
           "balance":"10.00","balance-date":1755000000,"transactions":[]},
          {"id":"ACT-1","conn_id":"C2","name":"Checking","currency":"USD",
           "balance":"20.00","balance-date":1755000000,"transactions":[]}
        ]}"#;
    let transport = FixtureTransport::queue(vec![(200, body.to_owned())]);
    let synced = mk_adapter(&transport).sync(&conn(), Some(since)).unwrap();

    let ids: Vec<_> = synced
        .batch
        .accounts
        .iter()
        .filter_map(|a| a.external_id.as_deref())
        .collect();
    assert_eq!(ids, vec!["C1/ACT-1", "C2/ACT-1"]);
    assert_eq!(
        synced.batch.accounts[0].external_name.as_deref(),
        Some("First Bank Checking")
    );
    let balances = synced
        .batch
        .records
        .iter()
        .filter(|r| r.balance.is_some())
        .count();
    assert_eq!(
        balances, 2,
        "same-id accounts across connections stay distinct"
    );
}

#[test]
fn fetch_transactions_filters_when_the_server_ignores_the_account_param() {
    // account= is a v2 addition a v1 server may ignore — the adapter must
    // filter the response itself.
    let body = set_json(&[
        account_json("ACT-1", &txn_json("TXN-1", 1_755_000_000, "-5.00")),
        account_json("ACT-2", &txn_json("TXN-2", 1_755_000_000, "-9.00")),
    ]);
    let since = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
    let transport = FixtureTransport::queue(vec![(200, body)]);
    let records = mk_adapter(&transport)
        .fetch_transactions(&conn(), "ACT-1", Some(since))
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0]
            .transaction
            .as_ref()
            .unwrap()
            .external_account
            .as_deref(),
        Some("ACT-1")
    );
}

#[test]
fn non_fatal_provider_errors_become_warnings_and_bad_records_are_never_silent() {
    let since = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
    let mixed = format!(
        r#"{{"errors":["Requested date range exceeds limit of 90 days and was capped."],
            "accounts":[{},{}]}}"#,
        // Unsupported currency: whole account's records become warnings.
        account_json("ACT-1", &txn_json("TXN-1", 1_755_000_000, "-5.00"))
            .replace("\"USD\"", "\"ZMW\""),
        // Unparseable amount on one txn, and a zero-amount row (the live
        // Bridge emits 0.00 rows — found by the demo drill).
        account_json(
            "ACT-2",
            &[
                txn_json("TXN-2", 1_755_000_000, "1.2.3"),
                txn_json("TXN-3", 1_755_000_000, "0.00"),
            ]
            .join(","),
        ),
    );
    let transport = FixtureTransport::queue(vec![(200, mixed)]);
    let synced = mk_adapter(&transport).sync(&conn(), Some(since)).unwrap();

    assert_eq!(
        synced
            .batch
            .records
            .iter()
            .filter(|r| r.transaction.is_some())
            .count(),
        0
    );
    let warnings = &synced.batch.warnings;
    assert!(warnings.iter().any(|w| w.message.contains("was capped")));
    assert!(warnings
        .iter()
        .any(|w| w.message.contains("unsupported currency")));
    assert!(warnings
        .iter()
        .any(|w| w.message.contains("unparseable amount")));
    assert!(warnings
        .iter()
        .any(|w| w.message.contains("zero-amount transaction")));
}

#[test]
fn a_1k_transaction_batch_stages_every_record_with_unique_hashes() {
    let txns: Vec<String> = (0..1000)
        .map(|i| txn_json(&format!("TXN-{i}"), 1_755_000_000 + i * 60, "-1.00"))
        .collect();
    let body = set_json(&[account_json("ACT-1", &txns.join(","))]);
    let since = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
    let transport = FixtureTransport::queue(vec![(200, body)]);
    let synced = mk_adapter(&transport).sync(&conn(), Some(since)).unwrap();

    let records: Vec<_> = synced
        .batch
        .records
        .iter()
        .filter(|r| r.transaction.is_some())
        .collect();
    assert_eq!(records.len(), 1000);
    let hashes: std::collections::HashSet<_> =
        records.iter().map(|r| r.source_hash.as_str()).collect();
    assert_eq!(hashes.len(), 1000, "source hashes must be unique");
    let fingerprints: std::collections::HashSet<_> = records
        .iter()
        .map(|r| r.transaction.as_ref().unwrap().txn_fingerprint.as_str())
        .collect();
    assert_eq!(fingerprints.len(), 1000, "fingerprints must be unique");
}

// ===========================================================================
// Health + leak rule + registry
// ===========================================================================

#[test]
fn health_maps_the_taxonomy() {
    let transport = FixtureTransport::queue(vec![(200, set_json(&[]))]);
    assert_eq!(
        mk_adapter(&transport).health(&conn()).unwrap(),
        HealthStatus::Healthy
    );

    let transport =
        FixtureTransport::queue(vec![(403, r#"{"errors":[],"accounts":[]}"#.to_owned())]);
    assert_eq!(
        mk_adapter(&transport).health(&conn()).unwrap(),
        HealthStatus::Expired
    );

    let transport = FixtureTransport::queue(vec![]); // exhausted → transport error
    assert!(matches!(
        mk_adapter(&transport).health(&conn()).unwrap(),
        HealthStatus::Unreachable { .. }
    ));
}

#[test]
fn nothing_the_adapter_emits_contains_the_credential() {
    let secret = "CFO-CANARY-9f2d7c1e";
    let body = set_json(&[account_json(
        "ACT-1",
        &txn_json("TXN-1", 1_755_000_000, "-5.00"),
    )]);
    let since = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
    let transport = FixtureTransport::queue(vec![(200, body)]);
    let adapter = mk_adapter(&transport);
    let synced = adapter.sync(&conn(), Some(since)).unwrap();

    let json = serde_json::to_string(&synced).unwrap();
    assert!(!json.contains(secret), "credential in persisted output");
    assert!(
        !format!("{synced:?}").contains(secret),
        "credential in Debug output"
    );

    // Error paths (incl. the malformed-credential message) never echo it.
    let bad_conn = Connection {
        credential: Credential::new(format!("https://{secret}.example/nope")),
    };
    let transport = FixtureTransport::queue(vec![]);
    let err = mk_adapter(&transport)
        .fetch_accounts(&bad_conn)
        .unwrap_err();
    assert!(!format!("{err}").contains(secret), "credential in error");

    // Requests carry the credential only as split Basic auth, never in URLs.
    for request in transport.recorded() {
        assert!(!request.url.contains(secret), "credential in request URL");
    }
}

#[test]
fn the_adapter_is_registered_under_its_schema_token() {
    let adapter = connector_core::connector_by_id("simplefin").expect("registered");
    assert_eq!(adapter.display_name(), "SimpleFIN");
    assert!(adapter
        .capabilities()
        .supports(connector_core::Capability::Transactions));
    assert!(!adapter
        .capabilities()
        .supports(connector_core::Capability::Holdings));
}

#[test]
fn retry_required_errors_populate_the_structured_hold_fields() {
    let since = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
    let body = format!(
        r#"{{"errlist":[
            {{"code":"act.missingdata","msg":"Incomplete transaction listing.","account_id":"ACT-1"}},
            {{"code":"act.failed","msg":"Failed to get account information."}}
        ],"connections":[],"accounts":[{}]}}"#,
        account_json("ACT-2", &txn_json("TXN-1", 1_755_000_000, "-5.00"))
    );
    let transport = FixtureTransport::queue(vec![(200, body)]);
    let synced = mk_adapter(&transport).sync(&conn(), Some(since)).unwrap();

    assert_eq!(synced.held_account_ids, vec!["ACT-1".to_owned()]);
    assert!(
        synced.hold_all_watermarks,
        "an unscoped act.failed holds everything"
    );
    // The prose warnings still surface for humans.
    assert!(synced
        .batch
        .warnings
        .iter()
        .any(|w| w.message.starts_with("retry required")));
}
