//! `lunchflow-adapter` — the LunchFlow connector (personal-cfo-r2pow, ADR 0076),
//! the second adapter on the `connector-core` contract.
//!
//! Built against LunchFlow's documented **Personal API**
//! (lunchflow.app/docs/api, read 2026-09-27;
//! docs/research/lunchflow-feasibility.md): the user creates an "API
//! destination" in their own LunchFlow dashboard and pastes its key; every
//! request sends it as an `x-api-key` header to the pinned
//! [`BASE_URL`]. That is ADR 0004 §2's user-token tier — no relay, no project
//! secret, the same shape as SimpleFIN. The Platform API (OAuth, a
//! project-owned client secret) is never used.
//!
//! Endpoints: `GET /accounts`, `GET /accounts/{id}/transactions`
//! (`from`/`to`/`include_pending`), `GET /accounts/{id}/balance`. There is no
//! documented pagination, rate limit, or revoke endpoint.
//!
//! Ships **disabled** in the registry (ADR 0076 decisions 2–3): `connector_link`
//! refuses it until the v0.3.0 release bead flips `enabled`.
//!
//! LEAK RULE: the API key lives in [`connector_core::Credential`] and travels
//! only in a request header. No error message, warning or log carries it, and
//! every provider-controlled string entering a message is sanitized first.

use chrono::{Days, NaiveDate};
use connector_core::{
    balance_record, register_connector, review_date, AccountType, BillingPeriod, CapabilitySet,
    Connection, ConnectorAdapter, ConnectorEconomics, ConnectorError, ConnectorMetadata,
    ConnectorReferral, Credential, CredentialTier, DisclosureText, HealthStatus, LinkInput,
    LinkSession, Payer,
};
use importer_core::{
    content_fingerprint, ParseWarning, ParsedAccount, ParsedBalance, ParsedBatch, ParsedRecord,
    ParsedTransaction,
};
use semver::Version;
use serde::Serialize;

pub mod transport;
pub mod wire;

use transport::{HttpResponse, Transport, UreqTransport};
use wire::{
    AccountList, BalanceEnvelope, ErrorBody, TransactionList, WireAccount, WireTransaction,
};

/// The pinned Personal API base. LunchFlow's docs show `https://lunchflow.app`,
/// but that host answers every API path with a permanent 308 redirect to
/// `www.lunchflow.app` (seen by the owner's live drill, 2026-09-27). The
/// adapter follows no redirects, so the key header is never replayed to
/// another host, which means it must name the canonical host directly. The
/// threat model's TB3 egress row names this host.
pub const BASE_URL: &str = "https://www.lunchflow.app/api/v1";

/// First-refresh lookback when the caller passes `since: None`. Open-banking
/// providers generally cap history at 24 months; asking for that much lets
/// the provider return whatever it has instead of a narrower default window.
const FULL_HISTORY_LOOKBACK_DAYS: u64 = 730;
/// An explicit `since` is rewound so consecutive refreshes overlap: late-
/// posting transactions near the watermark are re-fetched and downstream
/// dedupe absorbs the repeats (the SimpleFIN adapter's margin).
const SINCE_REWIND_DAYS: u64 = 5;

/// The LunchFlow adapter, generic over its HTTP seam so every test runs on
/// fixtures. The production instance is `LunchFlowAdapter<UreqTransport>`,
/// registered at compile time below.
pub struct LunchFlowAdapter<T: Transport = UreqTransport> {
    transport: T,
    /// Injected clock (DoD: time is mocked only via injection) — epoch seconds.
    now_epoch: fn() -> i64,
}

fn real_now_epoch() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

static LUNCHFLOW: LunchFlowAdapter = LunchFlowAdapter::new(UreqTransport::new(), real_now_epoch);

/// LunchFlow's registry entry (ADR 0015 + its 2026-09-27 addendum; ADR 0076).
///
/// **Disabled** until the v0.3.0 release bead (personal-cfo-p3f7r) flips it
/// once ADR 0076 decision 3's ship conditions hold.
///
/// Cost: the Individual plan's **annual** option — $34.99/year, 2 connections
/// included, $10.00 per extra connection per year — checked live on
/// lunchflow.app on the cost review date and matching
/// docs/research/lunchflow-feasibility.md §d. The schema holds one plan; the
/// monthly option ($5.49/month, 4 included, $1.00/month per extra,
/// owner-verified 2026-09-18, same doc) is recorded here only. Terms: read in
/// full by the owner on 2026-09-18 (§b; the pages render client-side).
///
/// Regions: the countries LunchFlow's coverage docs name individually for
/// bank accounts (lunchflow.app/docs/guides/connections, read 2026-09-27).
/// Its docs also cover "Europe" (GoCardless) and "Pacific Asia" (Finverse)
/// without naming countries, so those are not listed; re-check before the
/// enable flip.
///
/// Disclosure: LunchFlow's own wording. DohFlow has a referral relationship
/// with LunchFlow (ADR 0076 §5, D15), so unlike SimpleFIN it is **not**
/// described as unaffiliated; the referral carries the FTC sentence
/// verbatim.
pub const LUNCHFLOW_METADATA: ConnectorMetadata = ConnectorMetadata {
    tier: CredentialTier::UserToken,
    account_types: &[
        AccountType::Depository,
        AccountType::Credit,
        AccountType::Investment,
    ],
    regions: &["US", "CA", "GB", "NZ", "BR"],
    economics: ConnectorEconomics {
        payer: Payer::UserDirect,
        base_cost_minor_units: Some(3499),
        currency: Some("USD"),
        billing_period: Some(BillingPeriod::Annual),
        included_connections: Some(2),
        extra_connection_cost_minor_units: Some(1000),
        extra_connection_period: Some(BillingPeriod::Annual),
        cost_reviewed_at: review_date(2026, 9, 27),
        terms_url: Some("https://www.lunchflow.app/terms"),
        terms_reviewed_at: review_date(2026, 9, 18),
        history_depth_expectation: "Varies by bank and by LunchFlow's own data provider.",
    },
    disclosure: DisclosureText {
        independent_party: "LunchFlow is a separate company, not run by DohFlow. DohFlow has a \
                            referral relationship with LunchFlow, explained in the note below.",
        handles_credentials: "LunchFlow connects to your banks and gives this app read-only \
                              account and transaction data through an API key you create in \
                              your LunchFlow dashboard. Your bank credentials are given to \
                              LunchFlow, never to this app.",
        cost_summary: "It costs money. LunchFlow charges its own plan fee, paid to them \
                       \u{2014} nothing here is billed by this app.",
        optional: "It is optional. Everything in this app works with manual entry and file \
                   imports, including LunchFlow's own CSV and OFX exports; a connection only \
                   saves the typing.",
    },
    referral: Some(ConnectorReferral {
        url: "https://www.lunchflow.app/?atp=dohflow",
        disclosure: "DohFlow may earn a commission if you sign up for LunchFlow through the \
                     link above \u{2014} this does not affect what LunchFlow charges you, and \
                     DohFlow works the same whether or not you use it.",
    }),
    enabled: false,
};

register_connector!(LUNCHFLOW, LUNCHFLOW_METADATA);

impl<T: Transport> LunchFlowAdapter<T> {
    #[must_use]
    pub const fn new(transport: T, now_epoch: fn() -> i64) -> Self {
        Self {
            transport,
            now_epoch,
        }
    }

    fn today(&self) -> NaiveDate {
        wire::epoch_to_date((self.now_epoch)()).unwrap_or(NaiveDate::MAX)
    }

    /// One authenticated GET against the pinned base, triaged into the
    /// connector error taxonomy.
    fn get(
        &self,
        conn: &Connection,
        path: &str,
        query: &[(String, String)],
    ) -> Result<String, ConnectorError> {
        let key = conn.credential.expose_secret();
        if key.is_empty() {
            return Err(ConnectorError::Expired(
                "stored LunchFlow key is empty — re-link required".to_owned(),
            ));
        }
        let response = self
            .transport
            .get(&format!("{BASE_URL}{path}"), key, query)
            .map_err(|e| ConnectorError::Network(scrub(&e.0, key)))?;
        triage(&response, key)
    }

    fn accounts(&self, conn: &Connection) -> Result<Vec<WireAccount>, ConnectorError> {
        let body = self.get(conn, "/accounts", &[])?;
        let list: AccountList = serde_json::from_str(&body).map_err(|_| {
            ConnectorError::Provider("LunchFlow returned an unreadable account list".to_owned())
        })?;
        Ok(list.accounts)
    }

    /// One account's transactions over the refresh window, plus whether the
    /// provider reported more matches than it returned.
    fn transactions(
        &self,
        conn: &Connection,
        account_id: &str,
        since: Option<NaiveDate>,
    ) -> Result<(Vec<WireTransaction>, bool), ConnectorError> {
        let today = self.today();
        let from = match since {
            Some(since) => since.checked_sub_days(Days::new(SINCE_REWIND_DAYS)),
            None => today.checked_sub_days(Days::new(FULL_HISTORY_LOOKBACK_DAYS)),
        }
        .unwrap_or(NaiveDate::MIN);
        let query = [
            ("from".to_owned(), from.to_string()),
            ("to".to_owned(), today.to_string()),
            // Posted-only staging, as with SimpleFIN: pending rows change or
            // vanish, so they are not requested at all.
            ("include_pending".to_owned(), "false".to_owned()),
        ];
        let body = self.get(conn, &account_path(account_id, "transactions"), &query)?;
        let list: TransactionList = serde_json::from_str(&body).map_err(|_| {
            ConnectorError::Provider(scrub(
                &format!(
                    "LunchFlow returned unreadable transactions for account {}",
                    sanitize(account_id)
                ),
                conn.credential.expose_secret(),
            ))
        })?;
        let truncated = list
            .total
            .is_some_and(|total| total > list.transactions.len() as u64);
        Ok((list.transactions, truncated))
    }

    fn balance(
        &self,
        conn: &Connection,
        account_id: &str,
    ) -> Result<Result<ParsedBalance, String>, ConnectorError> {
        let body = self.get(conn, &account_path(account_id, "balance"), &[])?;
        let envelope: BalanceEnvelope = serde_json::from_str(&body).map_err(|_| {
            ConnectorError::Provider(scrub(
                &format!(
                    "LunchFlow returned an unreadable balance for account {}",
                    sanitize(account_id)
                ),
                conn.credential.expose_secret(),
            ))
        })?;
        Ok(map_balance(account_id, &envelope, self.today()))
    }
}

/// `/accounts/{id}/{what}` with the id percent-encoded: it came from the
/// provider and is hostile input to a URL path.
fn account_path(account_id: &str, what: &str) -> String {
    let mut encoded = String::with_capacity(account_id.len());
    for byte in account_id.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    format!("/accounts/{encoded}/{what}")
}

// ===========================================================================
// Provider-string hygiene
// ===========================================================================

/// Sanitize a provider-controlled string before it enters any warning or
/// error message: control characters and the bidi/zero-width forgery set are
/// stripped, and the result is truncated (simplefin-adapter's floor).
fn sanitize(raw: &str) -> String {
    let forged = |c: &char| {
        c.is_control()
            || matches!(
                c,
                '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}'
            )
    };
    let cleaned: String = raw.chars().filter(|c| !forged(c)).take(200).collect();
    if raw.chars().count() > 200 {
        format!("{cleaned}…")
    } else {
        cleaned
    }
}

// ===========================================================================
// Error triage
// ===========================================================================

/// Replace every occurrence of the connection's key in `text`. Provider
/// text is hostile input and may echo the key it was sent (an error body
/// quoting the rejected key, say); anything that reaches an error, a warning,
/// `last_error` or a log passes through here first.
fn scrub(text: &str, key: &str) -> String {
    if key.is_empty() {
        text.to_owned()
    } else {
        text.replace(key, "[redacted]")
    }
}

/// The provider's own `{error, message}` text — sanitized, and scrubbed of
/// the key — for enriching a status-level error.
fn provider_detail(body: &str, key: &str) -> String {
    serde_json::from_str::<ErrorBody>(body)
        .ok()
        .and_then(|e| e.message.or(e.error))
        .map(|m| format!(" (LunchFlow says: {})", scrub(&sanitize(&m), key)))
        .unwrap_or_default()
}

/// Map one exchange onto the ADR 0060 §5 taxonomy. A 429 is `RateLimited`,
/// a **healthy** state. A 401 (no key) or a 403 (a key LunchFlow refuses —
/// observed live for a wrong key, 2026-09-27; also a deleted API destination
/// or a lapsed plan) both mean this key no longer works: re-link.
///
/// An auth failure carries NO provider text at all: a body answering a
/// rejected key is the likeliest place for that key to be echoed back, and
/// the message flows into `last_error`, the connection DTO and logs. Other
/// statuses keep the provider's detail, scrubbed of `key`.
fn triage(response: &HttpResponse, key: &str) -> Result<String, ConnectorError> {
    match response.status {
        200..=299 => Ok(response.body.clone()),
        401 | 403 => Err(ConnectorError::Expired(
            "LunchFlow refused this API key — it may have been deleted, or your LunchFlow \
             plan may have lapsed; paste a new key or check your plan"
                .to_owned(),
        )),
        429 => Err(ConnectorError::RateLimited(
            "LunchFlow asked the app to slow down".to_owned(),
        )),
        status @ 300..=399 => Err(ConnectorError::Provider(format!(
            "LunchFlow redirected the request (status {status}) — possible API move; update the app"
        ))),
        status @ (500 | 502 | 503 | 504) => Err(ConnectorError::Network(format!(
            "LunchFlow is unavailable (status {status})"
        ))),
        status => Err(ConnectorError::Provider(format!(
            "unexpected status {status} from LunchFlow{}",
            provider_detail(&response.body, key)
        ))),
    }
}

// ===========================================================================
// Mapping — wire → staged candidates
// ===========================================================================

fn map_account(account: &WireAccount) -> ParsedAccount {
    let name = account.name.as_deref().map(sanitize).unwrap_or_default();
    let institution = account.institution_name.as_deref().map(sanitize);
    ParsedAccount {
        external_id: Some(account.id.clone()),
        external_name: Some(match institution {
            Some(institution) if !institution.is_empty() => {
                format!("{institution} {name}").trim().to_owned()
            }
            _ => name,
        }),
        external_number_hash: None,
        // LunchFlow reports no account type or subtype; the user chooses one
        // when mapping.
        proposed_subtype: None,
        currency: account.currency.as_deref().and_then(wire::iso_code),
    }
}

/// The normalized fields kept on the `source_record` (shred-after-parse, ADR
/// 0014 §4): enough to replay the row, nothing the key could hide in.
#[derive(Serialize)]
struct NormalizedTxn<'a> {
    account_id: &'a str,
    txn_id: &'a str,
    date: &'a str,
    amount: &'a str,
    currency: Option<&'a str>,
    merchant: Option<&'a str>,
    description: Option<&'a str>,
}

/// Map one wire transaction, or explain why it cannot be staged. `index`
/// feeds the house `source_hash` contract.
fn map_transaction(
    account: &WireAccount,
    txn: &WireTransaction,
    index: usize,
) -> Result<ParsedRecord, String> {
    let id = sanitize(&txn.id);
    if txn.is_pending {
        return Err(format!(
            "pending transaction {id} not staged (posted-only scope)"
        ));
    }
    let posted_date = wire::parse_date(&txn.date)
        .ok_or_else(|| format!("transaction {id} has no usable date"))?;
    let account_code = account.currency.as_deref().and_then(wire::iso_code);
    let row_code = txn.currency.as_deref().and_then(wire::iso_code);
    if let (Some(account_code), Some(row_code)) = (&account_code, &row_code) {
        if account_code != row_code {
            return Err(format!(
                "transaction {id} is in {row_code}, not its account's {account_code}"
            ));
        }
    }
    let code = row_code
        .or(account_code)
        .ok_or_else(|| format!("transaction {id} states no currency"))?;
    let currency = wire::currency_from_code(&code)
        .ok_or_else(|| format!("transaction {id} uses unsupported currency {code}"))?;
    let amount_text = wire::amount_text(&txn.amount);
    let provider_minor = wire::parse_amount_minor(amount_text, currency.exponent())
        .ok_or_else(|| format!("transaction {id} has an unparseable amount"))?;
    if provider_minor == 0 {
        // The staging contract requires a non-zero amount.
        return Err(format!("zero-amount transaction {id} not staged"));
    }
    let minor = to_ledger_sign(provider_minor);

    let description = txn
        .description
        .as_deref()
        .filter(|d| !d.trim().is_empty())
        .or_else(|| txn.merchant.as_deref().filter(|m| !m.trim().is_empty()))
        .map(sanitize);
    let normalized = NormalizedTxn {
        account_id: &account.id,
        txn_id: &txn.id,
        date: &txn.date,
        amount: amount_text,
        currency: Some(&code),
        merchant: txn.merchant.as_deref(),
        description: txn.description.as_deref(),
    };
    let normalized_json = serde_json::to_string(&normalized)
        .map_err(|_| format!("transaction {id} failed normalization"))?;

    Ok(ParsedRecord {
        external_id: Some(txn.id.clone()),
        source_hash: content_fingerprint(format!("{index}:{normalized_json}").as_bytes()),
        normalized_json,
        parse_confidence_bps: Some(10_000),
        transaction: Some(ParsedTransaction {
            posted_date,
            // The Personal API carries a single date (ADR 0045: no secondary
            // transaction date to record).
            transaction_date: None,
            raw_date: txn.date.clone(),
            date_confidence_bps: 10_000,
            amount: core_money::Money::new(minor, currency),
            description,
            category: None,
            normalized_merchant: None,
            external_account: Some(account.id.clone()),
            // Provider-id fingerprint, mirroring the OFX `fitid:` and
            // SimpleFIN `sfin:` conventions (ADR 0014 §3).
            txn_fingerprint: format!("{posted_date}|{minor}|lflow:{}", txn.id),
        }),
        balance: None,
    })
}

/// LunchFlow's amount sign → the ledger's (inflow +, outflow −). The API docs
/// do not state LunchFlow's convention; the owner confirmed on a live account
/// (2026-09-27, recorded on personal-cfo-r2pow) that purchases come back
/// negative, the ledger's own convention, so this is the identity. Kept as
/// one named function so the convention is stated, tested, and changeable in
/// exactly one place.
const fn to_ledger_sign(provider_minor: i64) -> i64 {
    provider_minor
}

/// The balance as of the refresh date: the Personal API returns a current
/// balance with no as-of timestamp, so the observation is dated `today` (the
/// injected clock).
fn map_balance(
    account_id: &str,
    envelope: &BalanceEnvelope,
    today: NaiveDate,
) -> Result<ParsedBalance, String> {
    let id = sanitize(account_id);
    let code = envelope
        .balance
        .currency
        .as_deref()
        .and_then(wire::iso_code)
        .ok_or_else(|| format!("balance for account {id} states no currency"))?;
    let currency = wire::currency_from_code(&code)
        .ok_or_else(|| format!("balance for account {id} uses unsupported currency {code}"))?;
    let minor = wire::parse_amount_minor(
        wire::amount_text(&envelope.balance.amount),
        currency.exponent(),
    )
    .ok_or_else(|| format!("balance for account {id} is unparseable"))?;
    Ok(ParsedBalance {
        observed_at: today,
        amount: core_money::Money::new(minor, currency),
        external_account: Some(account_id.to_owned()),
    })
}

/// The currency an account's transactions are in: the first row stating a
/// valid ISO code (a row that disagrees is warned about when it is mapped).
fn rows_currency(rows: &[WireTransaction]) -> Option<String> {
    rows.iter()
        .find_map(|t| t.currency.as_deref().and_then(wire::iso_code))
}

/// The ISO currency code a balance response states, if any.
fn balance_currency(body: &str) -> Option<String> {
    serde_json::from_str::<BalanceEnvelope>(body)
        .ok()
        .and_then(|e| e.balance.currency.as_deref().and_then(wire::iso_code))
}

fn needs_attention(account: &WireAccount) -> String {
    format!(
        "LunchFlow reports account {} as {} — reconnect it in your LunchFlow dashboard",
        sanitize(&account.id),
        sanitize(account.status.as_deref().unwrap_or("inactive"))
    )
}

// ===========================================================================
// The ConnectorAdapter impl
// ===========================================================================

impl<T: Transport> ConnectorAdapter for LunchFlowAdapter<T> {
    fn id(&self) -> &'static str {
        // A `source_batches.source_type` schema token (migration 53, ADR 0076
        // decisions 8–9) — never prefixed, never renamed.
        "lunchflow"
    }

    fn display_name(&self) -> &'static str {
        "LunchFlow"
    }

    fn version(&self) -> Version {
        Version::new(0, 1, 0)
    }

    fn capabilities(&self) -> CapabilitySet {
        CapabilitySet {
            accounts: true,
            transactions: true,
            balances: true,
            // LunchFlow serves holdings for some providers, but no staged
            // holdings shape exists yet (personal-cfo-kmw5) — declaring it
            // would be a lie.
            holdings: false,
            liabilities: false,
        }
    }

    /// Validate a pasted Personal API key with one `GET /accounts`. The key
    /// itself is the credential — the base URL is pinned, so a refresh needs
    /// nothing else.
    fn link(&self, input: &LinkInput) -> Result<LinkSession, ConnectorError> {
        let pasted = input
            .user_token
            .as_ref()
            .ok_or_else(|| ConnectorError::NeedsUserAction {
                code: "key.missing".to_owned(),
                message: "paste a LunchFlow API key to connect".to_owned(),
                help_url: None,
            })?;
        // Strip ALL whitespace: pasted keys pick up line wraps and padding.
        let key = zeroize::Zeroizing::new(
            pasted
                .expose_secret()
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>(),
        );
        if key.is_empty() {
            return Err(ConnectorError::NeedsUserAction {
                code: "key.missing".to_owned(),
                message: "paste a LunchFlow API key to connect".to_owned(),
                help_url: None,
            });
        }
        let credential = Credential::new(key.as_str());
        let accounts = match self.accounts(&Connection {
            credential: credential.clone(),
        }) {
            Ok(accounts) => accounts,
            // At link time a refused key is the user's to fix: say so.
            Err(ConnectorError::Expired(_)) => {
                return Err(ConnectorError::NeedsUserAction {
                    code: "key.invalid".to_owned(),
                    message: "LunchFlow did not accept that API key — copy it again from the \
                              API destination in your LunchFlow dashboard (or create a new \
                              one), and check that your LunchFlow plan is active"
                        .to_owned(),
                    help_url: None,
                })
            }
            Err(other) => return Err(other),
        };
        let hint = match accounts.len() {
            1 => "LunchFlow connection, 1 account".to_owned(),
            n => format!("LunchFlow connection, {n} accounts"),
        };
        Ok(LinkSession::Established {
            credential,
            display_hint: Some(hint),
        })
    }

    /// Discovery. LunchFlow's live account objects carry no `currency`
    /// (documented, but absent — observed 2026-09-27), so an account without
    /// one takes its balance's currency: one extra request per such account,
    /// because the mapping surface needs the currency to refuse one the app
    /// cannot hold (ADR 0076 decision 7). A failed lookup leaves it `None`.
    /// A user's balance-currency override in LunchFlow shows up here as the
    /// override; refresh corrects it from the transactions themselves and
    /// never stages a balance whose currency disagrees with them.
    fn fetch_accounts(&self, conn: &Connection) -> Result<Vec<ParsedAccount>, ConnectorError> {
        let mut accounts = self.accounts(conn)?;
        for account in accounts.iter_mut().filter(|a| a.currency.is_none()) {
            match self.get(conn, &account_path(&account.id, "balance"), &[]) {
                Ok(body) => account.currency = balance_currency(&body),
                Err(err) if is_connection_level(&err) => return Err(err),
                Err(_) => {}
            }
        }
        Ok(accounts.iter().map(map_account).collect())
    }

    fn fetch_transactions(
        &self,
        conn: &Connection,
        account_external_id: &str,
        since: Option<NaiveDate>,
    ) -> Result<Vec<ParsedRecord>, ConnectorError> {
        // The account's currency is needed to check each row's; one extra
        // request, same as the listing sync() already holds.
        let accounts = self.accounts(conn)?;
        let Some(account) = accounts.iter().find(|a| a.id == account_external_id) else {
            return Err(ConnectorError::Provider(scrub(
                &format!("LunchFlow has no account {}", sanitize(account_external_id)),
                conn.credential.expose_secret(),
            )));
        };
        let (transactions, _) = self.transactions(conn, &account.id, since)?;
        let mut seen = std::collections::HashSet::new();
        // Unmappable rows are dropped without a channel here (the fetch_*
        // signatures have none); sync() reports every reason.
        Ok(transactions
            .iter()
            .filter(|t| seen.insert(t.id.clone()))
            .enumerate()
            .filter_map(|(index, txn)| map_transaction(account, txn, index).ok())
            .collect())
    }

    fn fetch_balances(
        &self,
        conn: &Connection,
        account_external_id: &str,
    ) -> Result<Vec<ParsedBalance>, ConnectorError> {
        Ok(self
            .balance(conn, account_external_id)?
            .into_iter()
            .collect())
    }

    fn health(&self, conn: &Connection) -> Result<HealthStatus, ConnectorError> {
        match self.accounts(conn) {
            Ok(accounts) => Ok(match accounts.iter().find(|a| !a.is_active()) {
                Some(account) => HealthStatus::NeedsUserAction {
                    code: "lunchflow.account_status".to_owned(),
                    message: needs_attention(account),
                    help_url: None,
                },
                None => HealthStatus::Healthy,
            }),
            Err(ConnectorError::Expired(_)) => Ok(HealthStatus::Expired),
            Err(ConnectorError::RateLimited(_)) => Ok(HealthStatus::RateLimited),
            Err(ConnectorError::Network(message)) => Ok(HealthStatus::Unreachable { message }),
            Err(ConnectorError::NeedsUserAction {
                code,
                message,
                help_url,
            }) => Ok(HealthStatus::NeedsUserAction {
                code,
                message,
                help_url,
            }),
            Err(other) => Err(other),
        }
    }

    /// The Personal API has no revoke endpoint: forgetting the connection
    /// deletes the local key, and the user revokes it server-side by deleting
    /// the API destination in their LunchFlow dashboard.
    fn revoke(&self, _conn: &Connection) -> Result<(), ConnectorError> {
        Ok(())
    }

    /// One refresh: the account list, then each account's transactions and
    /// balance (`1 + 2N` requests). Connection-level failures (a refused key,
    /// a throttle) abort; a failure scoped to one account becomes a warning
    /// and holds that account's watermark, so the next refresh re-asks for
    /// the same window. Every row that cannot be staged is reported, never
    /// silently dropped.
    fn sync(
        &self,
        conn: &Connection,
        since: Option<NaiveDate>,
    ) -> Result<connector_core::SyncBatch, ConnectorError> {
        let wire_accounts = self.accounts(conn)?;
        let today = self.today();
        let mut accounts = Vec::new();
        let mut records = Vec::new();
        let mut warnings = Vec::new();
        let mut held_account_ids = Vec::new();
        let mut txn_index = 0_usize;
        let mut balance_index = 0_usize;
        let warn = |warnings: &mut Vec<ParseWarning>, message: String| {
            warnings.push(ParseWarning { row: None, message });
        };

        for listed in &wire_accounts {
            // The balance first: it is also where an account without a
            // stated currency learns one (see fetch_accounts).
            let mut account = listed.clone();
            let balance = match self.get(conn, &account_path(&account.id, "balance"), &[]) {
                Ok(body) => match serde_json::from_str::<BalanceEnvelope>(&body) {
                    Ok(envelope) => Some(envelope),
                    Err(_) => {
                        warn(
                            &mut warnings,
                            format!(
                                "LunchFlow returned an unreadable balance for account {}",
                                sanitize(&account.id)
                            ),
                        );
                        None
                    }
                },
                Err(err) if is_connection_level(&err) => return Err(err),
                Err(err) => {
                    warn(
                        &mut warnings,
                        format!(
                            "the balance for account {} was not refreshed: {err}",
                            sanitize(&account.id)
                        ),
                    );
                    None
                }
            };
            // Rows first, then decide the account's currency: the API states
            // none on live accounts, and a balance can carry a currency the
            // user chose in LunchFlow's balance override (Configure >
            // Balance, observed 2026-09-27). The transactions' own currency
            // is the ground truth; the balance's is only a fallback.
            let fetched = match self.transactions(conn, &account.id, since) {
                Ok(fetched) => Some(fetched),
                Err(err) if is_connection_level(&err) => return Err(err),
                Err(err) => {
                    warn(
                        &mut warnings,
                        format!(
                            "transactions for account {} were not refreshed: {err}",
                            sanitize(&account.id)
                        ),
                    );
                    held_account_ids.push(account.id.clone());
                    None
                }
            };
            let balance_code = balance
                .as_ref()
                .and_then(|e| e.balance.currency.as_deref().and_then(wire::iso_code));
            if account.currency.is_none() {
                account.currency = fetched
                    .as_ref()
                    .and_then(|(rows, _)| rows_currency(rows))
                    .or_else(|| balance_code.clone());
            }
            let account = account;
            accounts.push(map_account(&account));
            if !account.is_active() {
                warn(&mut warnings, needs_attention(&account));
                held_account_ids.push(account.id.clone());
            }

            if let Some((transactions, truncated)) = fetched {
                if truncated {
                    warn(
                        &mut warnings,
                        format!(
                            "LunchFlow returned only part of account {}'s transactions; \
                             the next refresh asks again",
                            sanitize(&account.id)
                        ),
                    );
                    held_account_ids.push(account.id.clone());
                }
                let mut seen = std::collections::HashSet::new();
                for txn in &transactions {
                    if !seen.insert(&txn.id) {
                        continue;
                    }
                    match map_transaction(&account, txn, txn_index) {
                        Ok(record) => {
                            records.push(record);
                            txn_index += 1;
                        }
                        Err(reason) => warn(&mut warnings, reason),
                    }
                }
            }

            // A balance in a different currency from the account's
            // transactions is a display override, not the account's
            // currency: report it and leave the balance out rather than
            // anchor the account with it.
            let balance = match (&balance_code, &account.currency) {
                (Some(balance_code), Some(account_code)) if balance_code != account_code => {
                    warn(
                        &mut warnings,
                        format!(
                            "the balance for account {} is in {balance_code}, but its \
                             transactions are in {account_code} — a currency override in \
                             LunchFlow's balance settings? The balance was not refreshed",
                            sanitize(&account.id)
                        ),
                    );
                    None
                }
                _ => balance,
            };

            if let Some(envelope) = balance {
                match map_balance(&account.id, &envelope, today) {
                    Ok(balance) => {
                        records.push(balance_record(balance_index, balance));
                        balance_index += 1;
                    }
                    Err(reason) => warn(&mut warnings, reason),
                }
            }
        }

        // Every warning quotes provider-controlled text (ids, status strings,
        // error details): scrub the key out of all of it before it leaves.
        let key = conn.credential.expose_secret();
        for warning in &mut warnings {
            warning.message = scrub(&warning.message, key);
        }
        held_account_ids.sort();
        held_account_ids.dedup();
        Ok(connector_core::SyncBatch {
            adapter_id: self.id().to_owned(),
            adapter_version: self.version().to_string(),
            since,
            held_account_ids,
            hold_all_watermarks: false,
            batch: ParsedBatch {
                source_format: self.id().to_owned(),
                accounts,
                records,
                warnings,
            },
        })
    }
}

/// Errors that describe the whole connection (the key, the plan, a throttle)
/// rather than one account.
fn is_connection_level(err: &ConnectorError) -> bool {
    matches!(
        err,
        ConnectorError::Expired(_)
            | ConnectorError::NeedsUserAction { .. }
            | ConnectorError::RateLimited(_)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_strings_are_sanitized() {
        let forged = "Coffee\u{202E}evil\nnext\u{200B}line";
        assert_eq!(sanitize(forged), "Coffeeevilnextline");
        let long = "x".repeat(300);
        assert_eq!(sanitize(&long).chars().count(), 201);
    }

    #[test]
    fn account_ids_are_percent_encoded_into_paths() {
        assert_eq!(account_path("4242", "balance"), "/accounts/4242/balance");
        assert_eq!(
            account_path("../x?y", "transactions"),
            "/accounts/..%2Fx%3Fy/transactions"
        );
    }

    #[test]
    fn the_pinned_base_is_the_canonical_https_host() {
        // The bare `lunchflow.app` host 308-redirects every API path to
        // `www`; with redirects off, pinning the bare host breaks every call.
        assert_eq!(BASE_URL, "https://www.lunchflow.app/api/v1");
    }

    #[test]
    fn the_ledger_sign_is_the_providers() {
        assert_eq!(to_ledger_sign(-1234), -1234);
        assert_eq!(to_ledger_sign(500), 500);
    }

    fn triage_(response: &HttpResponse) -> Result<String, ConnectorError> {
        triage(response, "test-key")
    }

    #[test]
    fn auth_failures_carry_no_provider_text_and_others_are_scrubbed() {
        let echo = r#"{"error":"Forbidden","message":"invalid key lf-SECRET-123"}"#;
        for status in [401, 403] {
            let err = triage(
                &HttpResponse {
                    status,
                    body: echo.to_owned(),
                },
                "lf-SECRET-123",
            )
            .unwrap_err();
            let text = format!("{err} {err:?}");
            assert!(
                !text.contains("SECRET") && !text.contains("invalid key"),
                "{text}"
            );
        }
        let err = triage(
            &HttpResponse {
                status: 400,
                body: echo.to_owned(),
            },
            "lf-SECRET-123",
        )
        .unwrap_err();
        let text = format!("{err} {err:?}");
        assert!(!text.contains("SECRET"), "{text}");
        assert!(
            text.contains("[redacted]"),
            "detail kept, key scrubbed: {text}"
        );
    }

    #[test]
    fn statuses_triage_into_the_taxonomy() {
        let r = |status: u16, body: &str| HttpResponse {
            status,
            body: body.to_owned(),
        };
        assert!(matches!(
            triage_(&r(401, r#"{"error":"Unauthorized"}"#)),
            Err(ConnectorError::Expired(_))
        ));
        assert!(matches!(
            triage_(&r(403, r#"{"error":"Forbidden","message":"invalid key"}"#)),
            Err(ConnectorError::Expired(_))
        ));
        assert!(matches!(
            triage_(&r(429, "")),
            Err(ConnectorError::RateLimited(_))
        ));
        assert!(matches!(
            triage_(&r(503, "")),
            Err(ConnectorError::Network(_))
        ));
        assert!(matches!(
            triage_(&r(302, "")),
            Err(ConnectorError::Provider(_))
        ));
        let Err(ConnectorError::Provider(message)) =
            triage_(&r(400, r#"{"message":"bad\u001b[31m range"}"#))
        else {
            panic!("400 is a provider error");
        };
        assert!(
            !message.contains('\u{1b}'),
            "control char leaked: {message}"
        );
    }
}
