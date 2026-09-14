//! `simplefin-adapter` — the SimpleFIN connector (personal-cfo-w3gh, ADR 0060
//! §1), first real adapter on the `connector-core` contract.
//!
//! Clean-room from the SimpleFIN protocol spec (v1.0.7 + v2.0.0-draft;
//! summarized in docs/research/simplefin-feasibility.md) — no third-party
//! code. The flow, verbatim from the spec:
//!
//! 1. The user pastes a **setup token** — base64 of a one-time claim URL.
//! 2. `POST` that URL once (empty body, explicit `Content-Length: 0`) →
//!    `200 text/plain` whose body is the **access URL**, with HTTP Basic
//!    credentials in the URL userinfo. A reused token → `403` → re-link.
//! 3. `GET {access}/accounts` with the credentials as a Basic header
//!    (never trusted to URL userinfo), `?version=2` for structured errors —
//!    while still parsing the v1 envelope a default-configured server sends —
//!    `start-date`/`end-date` in epoch seconds, ranges capped at 90 days,
//!    walked in sub-cap chunks with overlap and seen-id dedupe.
//!
//! LEAK RULE: the access URL **is** the credential. It lives in
//! [`connector_core::Credential`], and no URL ever appears in an error
//! message, warning, or log — transports report failure *kinds* only, and
//! every provider-controlled string entering a message is sanitized
//! (control-stripped + truncated) first.
//!
//! Rate budget (Bridge policy, ~24 req/day): one request per chunk in
//! [`ConnectorAdapter::sync`], single requests elsewhere; scheduling/debounce
//! is the caller's job (sync on vault open + manual refresh, ADR 0060 §4).
//! A 429 maps to [`ConnectorError::RateLimited`] — a *healthy* state; the
//! Bridge's soft over-budget notices arrive as `errors`-array warnings.

use chrono::{Days, NaiveDate};
use connector_core::{
    balance_record, register_connector, CapabilitySet, Connection, ConnectorAdapter,
    ConnectorError, Credential, HealthStatus, LinkInput, LinkSession,
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
use wire::{AccountSet, WireAccount, WireTransaction};

/// First-sync lookback when the caller passes `since: None`. Bridge history
/// depth at link time is ~2–6 months (feasibility doc), so a year captures
/// everything available without an unbounded walk.
const FULL_HISTORY_LOOKBACK_DAYS: u64 = 365;
/// The Bridge caps ranges at 90 days and, as of 2026-08 (observed live by
/// the demo drill), ADVISES ≤45: "Requested date range exceeds recommended
/// range of 45 days. In the future, this may be capped." Sit inside the
/// recommendation so a future cap changes nothing.
const CHUNK_DAYS: u64 = 45;
/// Bridge best practice overlaps windows by ~5 days, so chunks step 40…
const CHUNK_STEP_DAYS: u64 = 40;
/// …and an explicit `since` is rewound by the same margin, so *consecutive
/// syncs* also overlap (late-posting transactions near the watermark are
/// re-fetched; downstream dedupe absorbs the repeats).
const SINCE_REWIND_DAYS: u64 = 5;

/// The SimpleFIN adapter, generic over its HTTP seam so every test runs on
/// fixtures. The production instance is `SimpleFinAdapter<UreqTransport>`,
/// registered at compile time below.
pub struct SimpleFinAdapter<T: Transport = UreqTransport> {
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

static SIMPLEFIN: SimpleFinAdapter = SimpleFinAdapter::new(UreqTransport::new(), real_now_epoch);
register_connector!(SIMPLEFIN);

impl<T: Transport> SimpleFinAdapter<T> {
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

    /// One authenticated `GET {access}/accounts` with `?version=2` plus
    /// `extra` query params, mapped through the shared error triage.
    fn get_accounts(
        &self,
        conn: &Connection,
        extra: &[(String, String)],
    ) -> Result<AccountSet, ConnectorError> {
        let access = AccessUrl::parse(conn.credential.expose_secret())?;
        let mut query = vec![("version".to_owned(), "2".to_owned())];
        query.extend_from_slice(extra);
        let response = self
            .transport
            .get(
                &format!("{}/accounts", access.base),
                Some((&access.user, &access.password)),
                &query,
            )
            .map_err(|e| ConnectorError::Network(e.0))?;
        triage(&response)
    }
}

// ===========================================================================
// Provider-string hygiene
// ===========================================================================

/// Sanitize a provider-controlled string before it enters any warning or
/// error message: control characters (newline forgery, ANSI escapes) are
/// stripped and the result is truncated. Display-side sanitization still
/// applies (spec: "you must sanitize the strings"); this is the wire-side
/// floor.
fn sanitize(raw: &str) -> String {
    // Control chars (Cc) AND the format-char forgery set (Cf): bidi overrides
    // and isolates, zero-width characters, BOM — all usable to spoof what a
    // string visually says in the UI or a log line.
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
// Access URL — parsing the credential, never printing it
// ===========================================================================

/// The decomposed access URL. Exists only transiently inside a request; the
/// stored credential stays the intact URL string inside [`Credential`].
struct AccessUrl {
    /// `scheme://host[:port]/path` with the userinfo removed.
    base: String,
    user: zeroize::Zeroizing<String>,
    password: zeroize::Zeroizing<String>,
}

// The parsed credential must be as unprintable as the stored one.
impl std::fmt::Debug for AccessUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AccessUrl([redacted])")
    }
}

impl AccessUrl {
    fn parse(url: &str) -> Result<Self, ConnectorError> {
        let malformed = || {
            ConnectorError::Expired(
                "stored access credential is malformed — re-link required".to_owned(),
            )
        };
        let (scheme, rest) = url.split_once("://").ok_or_else(malformed)?;
        if scheme != "https" {
            return Err(malformed());
        }
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        let (userinfo, host) = authority.rsplit_once('@').ok_or_else(malformed)?;
        let (user, password) = userinfo.split_once(':').ok_or_else(malformed)?;
        if host.is_empty() || user.is_empty() {
            return Err(malformed());
        }
        let path = path.trim_end_matches('/');
        let base = if path.is_empty() {
            format!("{scheme}://{host}")
        } else {
            format!("{scheme}://{host}/{path}")
        };
        Ok(Self {
            base,
            user: zeroize::Zeroizing::new(percent_decode(user).ok_or_else(malformed)?),
            password: zeroize::Zeroizing::new(percent_decode(password).ok_or_else(malformed)?),
        })
    }
}

/// Minimal `%XX` decoder for URL userinfo. `None` on malformed escapes.
fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3)?;
            let hi = char::from(hex[0]).to_digit(16)?;
            let lo = char::from(hex[1]).to_digit(16)?;
            out.push(u8::try_from(hi * 16 + lo).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

// ===========================================================================
// Error triage — the spec's status codes + both error channels
// ===========================================================================

/// A short sanitized summary of a body's error channels, for enriching
/// status-level error messages.
fn body_error_detail(body: &str) -> Option<String> {
    let set: AccountSet = serde_json::from_str(body).ok()?;
    set.errlist
        .first()
        .map(|e| format!("{}: {}", sanitize(&e.code), sanitize(&e.msg)))
        .or_else(|| set.errors.first().map(|m| sanitize(m)))
}

/// Decide fatal-vs-parseable for one `/accounts` exchange. A Bridge 403 body
/// is itself a valid AccountSet, and a 200 can still carry errors (the 90-day
/// cap warning), so bodies are parsed on every status — on fatal statuses the
/// parse enriches the error message.
fn triage(response: &HttpResponse) -> Result<AccountSet, ConnectorError> {
    let detail = || {
        body_error_detail(&response.body)
            .map(|d| format!(" (provider says — {d})"))
            .unwrap_or_default()
    };
    match response.status {
        200 => serde_json::from_str(&response.body)
            .map_err(|_| ConnectorError::Provider("unparseable /accounts response".to_owned())),
        // No-redirect policy (transport doc): a 3xx here most likely means a
        // Bridge host migration — surface it as itself, never as a bogus
        // credential failure.
        status @ 300..=399 => Err(ConnectorError::Provider(format!(
            "provider redirected (status {status}) — possible host migration; update the app \
             or re-link"
        ))),
        // Spec: "Authentication failed. This could be because access has been
        // revoked or if the credentials are incorrect." → re-link.
        403 => Err(ConnectorError::Expired(format!(
            "access revoked or credentials no longer valid — re-link required{}",
            detail()
        ))),
        // Spec: 402 "Payment required" — the user's Bridge subscription.
        402 => Err(ConnectorError::NeedsUserAction {
            code: "payment_required".to_owned(),
            message: format!(
                "the SimpleFIN Bridge subscription requires payment{}",
                detail()
            ),
            help_url: None,
        }),
        // Throttled — a *healthy* state (ADR 0060 §5): retry later.
        429 => Err(ConnectorError::RateLimited(format!(
            "provider throttled the request — retry later{}",
            detail()
        ))),
        status => Err(ConnectorError::Provider(format!(
            "unexpected status {status} from provider"
        ))),
    }
}

/// The three channels a parsed set's errors split into.
struct SplitOutcome {
    /// Connection-scoped credential failure (`gen.auth`) — abort, re-link.
    fatal: Option<ConnectorError>,
    /// Institution-scoped re-auth (`con.auth`) — the *other* institutions'
    /// data is still good; sync continues and this surfaces as a warning +
    /// the health state.
    reauth: Option<ConnectorError>,
    warnings: Vec<ParseWarning>,
    /// Structured retry-required scopes (`act.failed`/`act.missingdata`):
    /// provider account ids whose windows are incomplete. `None` = unscoped.
    retry_required: Vec<Option<String>>,
}

/// Split a parsed set's error channels. Spec code grammar: `prefix.[subcode]`
/// with prefixes `gen`/`con`/`act`; unknown subcodes degrade to the naked
/// prefix. `act.failed`/`act.missingdata` are retry conditions ("Try again
/// later") — flagged with a `retry required` marker so the integration layer
/// can hold back its since-watermark instead of recording a complete sync.
/// Repeated auth errors land in warnings — nothing is silently dropped.
fn split_errors(set: &AccountSet) -> SplitOutcome {
    let mut outcome = SplitOutcome {
        fatal: None,
        reauth: None,
        warnings: Vec::new(),
        retry_required: Vec::new(),
    };
    for err in &set.errlist {
        let code = sanitize(&err.code);
        let msg = sanitize(&err.msg);
        let is_auth = err.code.ends_with(".auth");
        let prefix = err.code.split('.').next().unwrap_or("");
        if prefix == "gen" && is_auth && outcome.fatal.is_none() {
            outcome.fatal = Some(ConnectorError::NeedsUserAction {
                code,
                message: msg,
                help_url: None,
            });
        } else if prefix == "con" && is_auth && outcome.reauth.is_none() {
            outcome.reauth = Some(ConnectorError::NeedsUserAction {
                code,
                message: msg,
                help_url: None,
            });
        } else if err.code == "act.failed" || err.code == "act.missingdata" {
            outcome
                .retry_required
                .push(err.account_id.as_deref().map(sanitize));
            outcome.warnings.push(ParseWarning {
                row: None,
                message: format!(
                    "retry required ({code}): {msg} — account {}",
                    err.account_id.as_deref().map(sanitize).unwrap_or_default()
                ),
            });
        } else {
            outcome.warnings.push(ParseWarning {
                row: None,
                message: format!("provider reported ({code}): {msg}"),
            });
        }
    }
    for msg in &set.errors {
        outcome.warnings.push(ParseWarning {
            row: None,
            message: format!("provider reported: {}", sanitize(msg)),
        });
    }
    outcome
}

// ===========================================================================
// Chunked date walk — the 90-day cap
// ===========================================================================

/// `[start, end)` epoch-second windows covering `since..=today`, each ≤89
/// days, stepping 85 for intra-walk overlap. An explicit `since` is rewound
/// [`SINCE_REWIND_DAYS`] so consecutive syncs overlap too; `None` = the
/// full-history lookback.
fn chunk_ranges(since: Option<NaiveDate>, today: NaiveDate) -> Vec<(i64, i64)> {
    let start = match since {
        Some(date) => date
            .checked_sub_days(Days::new(SINCE_REWIND_DAYS))
            .unwrap_or(date),
        None => today
            .checked_sub_days(Days::new(FULL_HISTORY_LOOKBACK_DAYS))
            .unwrap_or(today),
    };
    // end-date is exclusive, so the walk's end bound is tomorrow.
    let walk_end = today.checked_add_days(Days::new(1)).unwrap_or(today);
    let mut ranges = Vec::new();
    let mut cursor = start.min(walk_end);
    loop {
        let chunk_end = cursor
            .checked_add_days(Days::new(CHUNK_DAYS))
            .unwrap_or(walk_end)
            .min(walk_end);
        ranges.push((epoch_at_midnight(cursor), epoch_at_midnight(chunk_end)));
        if chunk_end >= walk_end {
            return ranges;
        }
        cursor = cursor
            .checked_add_days(Days::new(CHUNK_STEP_DAYS))
            .unwrap_or(walk_end);
    }
}

fn epoch_at_midnight(date: NaiveDate) -> i64 {
    date.and_hms_opt(0, 0, 0)
        .map_or(0, |dt| dt.and_utc().timestamp())
}

fn date_range_query(start: i64, end: i64) -> Vec<(String, String)> {
    vec![
        ("start-date".to_owned(), start.to_string()),
        ("end-date".to_owned(), end.to_string()),
    ]
}

// ===========================================================================
// Account identity — connection-scoped
// ===========================================================================

/// Account ids are unique only *within* a connection (spec), so the staged
/// identity key is `"{conn_id}/{id}"` when the v2 `conn_id` is present, and
/// the bare id for v1 responses. [`split_account_key`] recovers the raw id
/// for the `account=` query parameter.
fn account_key(account: &WireAccount) -> String {
    match &account.conn_id {
        Some(conn) => format!("{conn}/{}", account.id),
        None => account.id.clone(),
    }
}

/// `"{conn_id}/{id}"` → `(Some(conn_id), id)`; bare ids pass through.
fn split_account_key(key: &str) -> (Option<&str>, &str) {
    match key.split_once('/') {
        Some((conn, id)) => (Some(conn), id),
        None => (None, key),
    }
}

/// Does this wire account match a requested composite key? A v1 response has
/// no `conn_id`, so a composite request degrades to matching the raw id.
fn account_matches(account: &WireAccount, requested_key: &str) -> bool {
    let (conn, id) = split_account_key(requested_key);
    account.id == id
        && match (conn, &account.conn_id) {
            (Some(want), Some(have)) => want == have,
            _ => true,
        }
}

// ===========================================================================
// Wire → staged-candidate mapping
// ===========================================================================

/// The normalized fields persisted as a record's `normalized_json` (ADR 0014
/// §4 shred-after-parse: this JSON survives, the wire body does not).
#[derive(Serialize)]
struct NormalizedTxn<'a> {
    account_key: &'a str,
    txn_id: &'a str,
    posted: i64,
    amount: &'a str,
    description: &'a str,
    transacted_at: Option<i64>,
}

fn map_account(account: &WireAccount, set: &AccountSet) -> ParsedAccount {
    // Institution display name: the v2 Connection's name, else the v1 org.
    let institution = account
        .conn_id
        .as_ref()
        .and_then(|conn_id| {
            set.connections
                .iter()
                .find(|c| &c.conn_id == conn_id)
                .and_then(|c| c.name.clone().or_else(|| c.org_name.clone()))
        })
        .or_else(|| {
            account
                .org
                .as_ref()
                .and_then(|org| org.name.clone().or_else(|| org.domain.clone()))
        });
    ParsedAccount {
        // Connection-scoped identity — the key every fetch is made with.
        external_id: Some(account_key(account)),
        external_name: Some(match institution {
            Some(institution) => {
                format!("{} {}", sanitize(&institution), sanitize(&account.name))
            }
            None => sanitize(&account.name),
        }),
        external_number_hash: None,
        proposed_subtype: None,
    }
}

/// Map one wire transaction, or explain why it cannot be staged. `index`
/// feeds the house `source_hash` contract (`sha256:` fingerprint of
/// `"{index}:{normalized_json}"`).
fn map_transaction(
    account: &WireAccount,
    txn: &WireTransaction,
    index: usize,
) -> Result<ParsedRecord, String> {
    let key = account_key(account);
    if txn.pending {
        return Err(format!(
            "pending transaction {} not staged (posted-only v1 scope)",
            sanitize(&txn.id)
        ));
    }
    let posted_date = wire::epoch_to_date(txn.posted).ok_or_else(|| {
        format!(
            "transaction {} has no usable posted date",
            sanitize(&txn.id)
        )
    })?;
    let currency = wire::currency_from_code(&account.currency).ok_or_else(|| {
        format!(
            "account {} uses unsupported currency {}",
            sanitize(&account.id),
            sanitize(&account.currency)
        )
    })?;
    let minor = wire::parse_amount_minor(&txn.amount, currency.exponent())
        .ok_or_else(|| format!("transaction {} has unparseable amount", sanitize(&txn.id)))?;
    if minor == 0 {
        // The ledger's staging contract requires a non-zero amount, and the
        // live Bridge does emit 0.00 rows (found by the demo drill) — surface
        // them as warnings, never abort a sync over one.
        return Err(format!(
            "zero-amount transaction {} not staged",
            sanitize(&txn.id)
        ));
    }

    let normalized = NormalizedTxn {
        account_key: &key,
        txn_id: &txn.id,
        posted: txn.posted,
        amount: &txn.amount,
        description: &txn.description,
        transacted_at: txn.transacted_at,
    };
    let normalized_json = serde_json::to_string(&normalized)
        .map_err(|_| format!("transaction {} failed normalization", sanitize(&txn.id)))?;

    Ok(ParsedRecord {
        external_id: Some(txn.id.clone()),
        source_hash: content_fingerprint(format!("{index}:{normalized_json}").as_bytes()),
        normalized_json,
        parse_confidence_bps: Some(10_000),
        transaction: Some(ParsedTransaction {
            posted_date,
            transaction_date: txn.transacted_at.and_then(wire::epoch_to_date),
            raw_date: txn.posted.to_string(),
            date_confidence_bps: 10_000,
            amount: core_money::Money::new(minor, currency),
            description: Some(txn.description.clone()),
            category: None,
            normalized_merchant: None,
            external_account: Some(key),
            // Provider-id fingerprint, mirroring the OFX importer's
            // `fitid:` convention (ADR 0014 §3).
            txn_fingerprint: format!("{posted_date}|{minor}|sfin:{}", txn.id),
        }),
        balance: None,
    })
}

fn map_balance(account: &WireAccount) -> Result<ParsedBalance, String> {
    let observed_at = wire::epoch_to_date(account.balance_date).ok_or_else(|| {
        format!(
            "account {} has no usable balance date",
            sanitize(&account.id)
        )
    })?;
    let currency = wire::currency_from_code(&account.currency).ok_or_else(|| {
        format!(
            "account {} uses unsupported currency {}",
            sanitize(&account.id),
            sanitize(&account.currency)
        )
    })?;
    let minor = wire::parse_amount_minor(&account.balance, currency.exponent())
        .ok_or_else(|| format!("account {} has unparseable balance", sanitize(&account.id)))?;
    Ok(ParsedBalance {
        observed_at,
        amount: core_money::Money::new(minor, currency),
        external_account: Some(account_key(account)),
    })
}

// ===========================================================================
// The ConnectorAdapter impl
// ===========================================================================

impl<T: Transport> ConnectorAdapter for SimpleFinAdapter<T> {
    fn id(&self) -> &'static str {
        // A `source_batches.source_type` schema token — never prefixed.
        "simplefin"
    }

    fn display_name(&self) -> &'static str {
        "SimpleFIN"
    }

    fn version(&self) -> Version {
        Version::new(0, 1, 0)
    }

    fn capabilities(&self) -> CapabilitySet {
        CapabilitySet {
            accounts: true,
            transactions: true,
            balances: true,
            // The Bridge emits holdings, but no staged shape exists yet
            // (personal-cfo-kmw5) — declaring it would be a lie.
            holdings: false,
            liabilities: false,
        }
    }

    fn link(&self, input: &LinkInput) -> Result<LinkSession, ConnectorError> {
        use base64::Engine as _;
        let token = input
            .user_token
            .as_ref()
            .ok_or_else(|| ConnectorError::NeedsUserAction {
                code: "token.missing".to_owned(),
                message: "paste a SimpleFIN setup token to connect".to_owned(),
                help_url: None,
            })?;
        // Strip ALL whitespace, not just the ends — long tokens pick up line
        // wraps when pasted from email or a terminal.
        let compact: String = token
            .expose_secret()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        let claim_url = base64::engine::general_purpose::STANDARD
            .decode(&compact)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .filter(|url| url.starts_with("https://"))
            .ok_or_else(|| ConnectorError::NeedsUserAction {
                code: "token.invalid".to_owned(),
                message: "that does not look like a SimpleFIN setup token — copy it again"
                    .to_owned(),
                help_url: None,
            })?;

        let response = self
            .transport
            .post_empty(&claim_url)
            .map_err(|e| ConnectorError::Network(e.0))?;
        match response.status {
            200 => {
                let trimmed = response.body.trim();
                // Validate now so a bad claim body fails at link time, not at
                // first sync. The only owned copy goes straight into the
                // zeroizing Credential; the transport-owned response body is
                // the accepted residual.
                AccessUrl::parse(trimmed).map_err(|_| {
                    ConnectorError::Provider("claim returned an unusable access URL".to_owned())
                })?;
                Ok(LinkSession::Established {
                    credential: Credential::new(trimmed),
                    display_hint: Some("SimpleFIN Bridge connection".to_owned()),
                })
            }
            // Spec: 403 = "does not exist or has already been claimed" — and
            // possibly compromised; the required checklist says to tell the
            // user so they can disable the token.
            403 => Err(ConnectorError::NeedsUserAction {
                code: "setup_token_used".to_owned(),
                message: "this setup token was already claimed or is invalid — it may be \
                          compromised, so disable it at the Bridge and generate a fresh one"
                    .to_owned(),
                help_url: None,
            }),
            status @ 300..=399 => Err(ConnectorError::Provider(format!(
                "claim redirected (status {status}) — possible host migration; update the app"
            ))),
            status => Err(ConnectorError::Provider(format!(
                "unexpected status {status} from claim"
            ))),
        }
    }

    fn fetch_accounts(&self, conn: &Connection) -> Result<Vec<ParsedAccount>, ConnectorError> {
        let set = self.get_accounts(conn, &[("balances-only".to_owned(), "1".to_owned())])?;
        let outcome = split_errors(&set);
        if let Some(fatal) = outcome.fatal.or(outcome.reauth) {
            return Err(fatal);
        }
        // NOTE: outcome.warnings are dropped here — the fetch_* trait
        // signatures have no warnings channel yet (connector-core follow-up
        // bead); sync() is the surfacing path.
        Ok(set.accounts.iter().map(|a| map_account(a, &set)).collect())
    }

    fn fetch_transactions(
        &self,
        conn: &Connection,
        account_external_id: &str,
        since: Option<NaiveDate>,
    ) -> Result<Vec<ParsedRecord>, ConnectorError> {
        let (_, raw_id) = split_account_key(account_external_id);
        let mut records = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut index = 0_usize;
        for (start, end) in chunk_ranges(since, self.today()) {
            let mut query = date_range_query(start, end);
            // `account=` is a v2 addition; a v1 server may ignore it, so the
            // response is filtered again below.
            query.push(("account".to_owned(), raw_id.to_owned()));
            let set = self.get_accounts(conn, &query)?;
            let outcome = split_errors(&set);
            if let Some(fatal) = outcome.fatal.or(outcome.reauth) {
                return Err(fatal);
            }
            for account in set
                .accounts
                .iter()
                .filter(|a| account_matches(a, account_external_id))
            {
                for txn in &account.transactions {
                    if !seen.insert(txn.id.clone()) {
                        continue;
                    }
                    // Unmappable records are dropped without a channel here
                    // (see fetch_accounts note); sync() reports reasons.
                    if let Ok(record) = map_transaction(account, txn, index) {
                        records.push(record);
                        index += 1;
                    }
                }
            }
        }
        Ok(records)
    }

    fn fetch_balances(
        &self,
        conn: &Connection,
        account_external_id: &str,
    ) -> Result<Vec<ParsedBalance>, ConnectorError> {
        let (_, raw_id) = split_account_key(account_external_id);
        let set = self.get_accounts(
            conn,
            &[
                ("balances-only".to_owned(), "1".to_owned()),
                ("account".to_owned(), raw_id.to_owned()),
            ],
        )?;
        let outcome = split_errors(&set);
        if let Some(fatal) = outcome.fatal.or(outcome.reauth) {
            return Err(fatal);
        }
        Ok(set
            .accounts
            .iter()
            .filter(|a| account_matches(a, account_external_id))
            // Unparseable balances are dropped without a channel here (see
            // fetch_accounts note); sync() reports reasons.
            .filter_map(|a| map_balance(a).ok())
            .collect())
    }

    fn health(&self, conn: &Connection) -> Result<HealthStatus, ConnectorError> {
        match self.get_accounts(conn, &[("balances-only".to_owned(), "1".to_owned())]) {
            Ok(set) => {
                let outcome = split_errors(&set);
                Ok(match outcome.fatal.or(outcome.reauth) {
                    Some(ConnectorError::NeedsUserAction {
                        code,
                        message,
                        help_url,
                    }) => HealthStatus::NeedsUserAction {
                        code,
                        message,
                        help_url,
                    },
                    Some(_) | None => HealthStatus::Healthy,
                })
            }
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

    /// The single-walk override ADR 0060 anticipated: one chunked walk with
    /// **no** account filter returns every account's transactions and
    /// balances together, so a full sync costs `ceil(range/85d)` requests
    /// regardless of account count — the right shape for a ≤24 req/day
    /// budget.
    ///
    /// Partial-failure posture: `gen.auth` (connection-scoped) aborts;
    /// `con.auth` (one institution needs re-auth at the Bridge) does NOT —
    /// the healthy institutions' data still stages, and the re-auth surfaces
    /// as a warning here plus `NeedsUserAction` from [`Self::health`].
    fn sync(
        &self,
        conn: &Connection,
        since: Option<NaiveDate>,
    ) -> Result<connector_core::SyncBatch, ConnectorError> {
        let mut accounts: Vec<ParsedAccount> = Vec::new();
        let mut account_keys = std::collections::HashSet::new();
        let mut records = Vec::new();
        let mut warnings = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut txn_index = 0_usize;
        let mut latest_balance: std::collections::BTreeMap<String, ParsedBalance> =
            std::collections::BTreeMap::new();

        let mut held_account_ids: Vec<String> = Vec::new();
        let mut hold_all_watermarks = false;
        for (start, end) in chunk_ranges(since, self.today()) {
            let set = self.get_accounts(conn, &date_range_query(start, end))?;
            let outcome = split_errors(&set);
            if let Some(fatal) = outcome.fatal {
                return Err(fatal);
            }
            for scope in &outcome.retry_required {
                match scope {
                    Some(account_id) => held_account_ids.push(account_id.clone()),
                    None => hold_all_watermarks = true,
                }
            }
            warnings.extend(outcome.warnings);
            if let Some(ConnectorError::NeedsUserAction { code, message, .. }) = outcome.reauth {
                warnings.push(ParseWarning {
                    row: None,
                    message: format!(
                        "an institution requires reauthorization at the Bridge ({code}): {message}"
                    ),
                });
            }

            for account in &set.accounts {
                let key = account_key(account);
                if account_keys.insert(key.clone()) {
                    accounts.push(map_account(account, &set));
                }
                match map_balance(account) {
                    // Every chunk repeats the current balance; keep one.
                    Ok(balance) => {
                        latest_balance.insert(key.clone(), balance);
                    }
                    Err(reason) => warnings.push(ParseWarning {
                        row: None,
                        message: reason,
                    }),
                }
                for txn in &account.transactions {
                    if !seen.insert((key.clone(), txn.id.clone())) {
                        continue;
                    }
                    match map_transaction(account, txn, txn_index) {
                        Ok(record) => {
                            records.push(record);
                            txn_index += 1;
                        }
                        Err(reason) => warnings.push(ParseWarning {
                            row: None,
                            message: reason,
                        }),
                    }
                }
            }
        }

        for (balance_index, balance) in latest_balance.into_values().enumerate() {
            records.push(balance_record(balance_index, balance));
        }

        held_account_ids.sort();
        held_account_ids.dedup();
        Ok(connector_core::SyncBatch {
            adapter_id: self.id().to_owned(),
            adapter_version: self.version().to_string(),
            since,
            held_account_ids,
            hold_all_watermarks,
            batch: ParsedBatch {
                source_format: self.id().to_owned(),
                accounts,
                records,
                warnings,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_url_parses_and_never_echoes_the_credential() {
        let url = "https://demo:s3cret@beta-bridge.simplefin.org/simplefin";
        let access = AccessUrl::parse(url).unwrap();
        assert_eq!(access.base, "https://beta-bridge.simplefin.org/simplefin");
        assert_eq!(access.user.as_str(), "demo");
        assert_eq!(access.password.as_str(), "s3cret");

        // Percent-encoded userinfo decodes; a ':' in the password survives.
        let access = AccessUrl::parse("https://u%40x:p%3Aw:extra@host/simplefin").unwrap();
        assert_eq!(access.user.as_str(), "u@x");
        assert_eq!(access.password.as_str(), "p:w:extra");

        for bad in [
            "http://demo:demo@host/simplefin", // not https
            "https://host/simplefin",          // no userinfo
            "not a url",
            "https://demo:pw%zz@host/x", // bad escape
        ] {
            let err = AccessUrl::parse(bad).unwrap_err();
            assert!(!format!("{err}").contains("host"), "URL leaked: {err}");
        }
    }

    #[test]
    fn chunk_ranges_cover_the_span_with_overlap_and_exclusive_end() {
        let today = NaiveDate::from_ymd_opt(2026, 8, 22).unwrap();
        let since = NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(); // ~202 days
        let ranges = chunk_ranges(Some(since), today);
        assert_eq!(ranges.len(), 6);
        // Every chunk sits inside the Bridge's 45-day recommendation.
        for (start, end) in &ranges {
            assert!(end - start <= 45 * 86_400);
        }
        // The explicit since is rewound 5 days so consecutive SYNCS overlap.
        let rewound = since.checked_sub_days(Days::new(5)).unwrap();
        assert_eq!(ranges[0].0, epoch_at_midnight(rewound));
        assert_eq!(ranges[1].0 - ranges[0].0, 40 * 86_400, "40-day step");
        // Final end bound is tomorrow (end-date is exclusive).
        let tomorrow = today.checked_add_days(Days::new(1)).unwrap();
        assert_eq!(ranges.last().unwrap().1, epoch_at_midnight(tomorrow));

        // A short range is a single chunk; None = the 365-day lookback.
        assert_eq!(chunk_ranges(Some(today), today).len(), 1);
        assert_eq!(chunk_ranges(None, today).len(), 10);
    }

    #[test]
    fn provider_strings_are_sanitized() {
        assert_eq!(sanitize("plain text"), "plain text");
        assert_eq!(
            sanitize("line1\nline2\x1b[31mred\x1b[0m"),
            "line1line2[31mred[0m"
        );
        // Bidi overrides / zero-width forgery characters are stripped too.
        assert_eq!(sanitize("a\u{202E}b\u{200B}c\u{2066}d\u{FEFF}"), "abcd");
        let long = "x".repeat(500);
        let cleaned = sanitize(&long);
        assert!(cleaned.chars().count() <= 201); // 200 + ellipsis
        assert!(cleaned.ends_with('…'));
    }

    #[test]
    fn account_keys_are_connection_scoped() {
        let v1 = WireAccount {
            id: "ACT-1".to_owned(),
            name: "Checking".to_owned(),
            currency: "USD".to_owned(),
            balance: "1.00".to_owned(),
            balance_date: 1_755_000_000,
            transactions: vec![],
            org: None,
            conn_id: None,
        };
        assert_eq!(account_key(&v1), "ACT-1");
        let mut v2 = v1.clone();
        v2.conn_id = Some("C9".to_owned());
        assert_eq!(account_key(&v2), "C9/ACT-1");
        assert_eq!(split_account_key("C9/ACT-1"), (Some("C9"), "ACT-1"));
        assert_eq!(split_account_key("ACT-1"), (None, "ACT-1"));
        assert!(account_matches(&v2, "C9/ACT-1"));
        assert!(!account_matches(&v2, "C8/ACT-1"));
        assert!(account_matches(&v1, "ACT-1"));
    }
}
