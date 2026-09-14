//! `connector-core` — the `ConnectorAdapter` contract + a compile-time adapter
//! registry (personal-cfo-x6dr, ADR 0004 / ADR 0060, plan §8.3).
//!
//! A connector is *just another importer* with a network on the far side: every
//! adapter emits **staged candidates** — [`importer_core::ParsedAccount`] /
//! [`importer_core::ParsedRecord`] / [`importer_core::ParsedBalance`] — that
//! enter the exact same staged-ingestion pipeline (ADR 0008) as file-importer
//! output. An adapter never writes ledger rows; this crate does not (and must
//! never) depend on `db-worker`, so the compile graph enforces it (CI:
//! "Enforce connector-core compile barrier").
//!
//! Provider neutrality (plan §8.3): no SimpleFIN/Teller/Plaid concept appears
//! in these types. Credentials are **user-owned** (ADR 0004 §2) and wrapped in
//! [`Credential`], which zeroizes on drop and cannot be Debug-printed,
//! Display-formatted, or serialized — the §6.6 never-log-tokens rule enforced
//! at the type level (pinned by static assertions in the unit tests).
//!
//! The error taxonomy here is ADR 0060 §5's: `NeedsUserAction`, `Expired`, and
//! — crucially — [`ConnectorError::RateLimited`], which is a **healthy**
//! connection state a caller must not surface as broken.
//!
//! Adapters register at compile time with [`register_connector!`] (built on
//! `inventory`), mirroring `importer-core`'s registry: no runtime dynamic
//! loading (ADR 0022), no runtime registration map to mistype.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use importer_core::{
    content_fingerprint, ParseWarning, ParsedAccount, ParsedBalance, ParsedBatch, ParsedRecord,
};
use semver::Version;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

#[cfg(any(test, feature = "mock"))]
pub mod mock;

// ===========================================================================
// Credentials — user-owned, never printable, zeroized on drop
// ===========================================================================

/// A user-owned connector credential (a SimpleFIN access URL, a claimed setup
/// token, …). Exists to make leaking hard: the secret zeroizes on drop (clones
/// included), no `Debug`/`Display` renders the value, and there is
/// deliberately **no** serde support — persisting the secret is an explicit
/// act via [`Credential::expose_secret`], done only by the vault-storage path
/// (ADR 0060 §1) and never by a log or a staged row.
///
/// No equality: nothing compares credentials today, so the non-constant-time
/// footgun is simply absent. If vault dedupe ever needs equality, implement it
/// with a constant-time comparison at that point.
#[derive(Clone)]
pub struct Credential(Zeroizing<String>);

impl Credential {
    #[must_use]
    pub fn new(secret: impl Into<String>) -> Self {
        Self(Zeroizing::new(secret.into()))
    }

    /// The raw secret. Callers other than vault storage and the adapter's own
    /// HTTP layer have no business calling this (greppable by design).
    #[must_use]
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Credential([redacted])")
    }
}

// ===========================================================================
// Capabilities
// ===========================================================================

/// One thing an adapter can do. Feature code checks these before calling the
/// corresponding `fetch_*` — a mismatch is a typed error, never a crash.
///
/// `Holdings` and `Liabilities` are **forward declarations** for the
/// capability/cost/terms registry (personal-cfo-5jjz): no `fetch_*` method
/// exists for them yet — those land with the investments arc (see the
/// follow-up bead under epic personal-cfo-pxi). An adapter must not declare a
/// capability it cannot serve.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Accounts,
    Transactions,
    Balances,
    Holdings,
    Liabilities,
}

/// What an adapter supports. Also the shape recorded (per provider) by the
/// capability/cost/terms registry (personal-cfo-5jjz, plan §8.2.1).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilitySet {
    pub accounts: bool,
    pub transactions: bool,
    pub balances: bool,
    pub holdings: bool,
    pub liabilities: bool,
}

impl CapabilitySet {
    #[must_use]
    pub fn supports(&self, capability: Capability) -> bool {
        match capability {
            Capability::Accounts => self.accounts,
            Capability::Transactions => self.transactions,
            Capability::Balances => self.balances,
            Capability::Holdings => self.holdings,
            Capability::Liabilities => self.liabilities,
        }
    }

    /// A typed [`ConnectorError::CapabilityMissing`] when `capability` is
    /// absent (personal-cfo-x6dr AC), for feature code to check up front.
    ///
    /// # Errors
    /// [`ConnectorError::CapabilityMissing`] when the capability is absent.
    pub fn ensure(&self, capability: Capability) -> Result<(), ConnectorError> {
        if self.supports(capability) {
            Ok(())
        } else {
            Err(ConnectorError::CapabilityMissing(capability))
        }
    }
}

// ===========================================================================
// Linking
// ===========================================================================

/// What the user supplies to establish a connection. For the user-token tier
/// (ADR 0004 §2) `user_token` carries the pasted token.
///
/// INVARIANT: `params` must never carry secret material — anything secret goes
/// through a [`Credential`]. `params` is for provider-neutral, non-secret
/// extras a link flow needs (institution, country, …) and is printed in full
/// by `Debug`. When a BYO adapter needs a second secret (a cert passphrase, a
/// user's own client secret), extend [`LinkInput`] with a
/// `Credential`-valued field — do not widen `params`.
#[derive(Default)]
pub struct LinkInput {
    pub user_token: Option<Credential>,
    pub params: BTreeMap<String, String>,
}

impl std::fmt::Debug for LinkInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LinkInput")
            .field(
                "user_token",
                &self.user_token.as_ref().map(|_| "[redacted]"),
            )
            .field("params", &self.params)
            .finish()
    }
}

/// The outcome of [`ConnectorAdapter::link`].
///
/// The relay tier (ADR 0004 §3) will complete `ExternalAuth` sessions via the
/// relay design (`personal-cfo-nizb`); the user-token tier returns
/// `Established` directly. An adapter must encode everything a later sync
/// needs into the credential itself — [`Connection`] carries nothing else.
pub enum LinkSession {
    /// Credential established — store it in the vault (ADR 0060 §1) and sync.
    Established {
        credential: Credential,
        /// Something safe to show ("Bridge connection, 3 institutions").
        display_hint: Option<String>,
    },
    /// The user must finish auth in the **system browser** (ADR 0004 §4 —
    /// never the app WebView). `auth_url` may embed one-time tokens in its
    /// query string, so `Debug` prints host-only and truncates `session_id`;
    /// treat both as sensitive until the relay design (`nizb`) rules otherwise.
    ExternalAuth {
        auth_url: String,
        session_id: String,
    },
}

impl std::fmt::Debug for LinkSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Established {
                credential,
                display_hint,
            } => f
                .debug_struct("Established")
                .field("credential", credential)
                .field("display_hint", display_hint)
                .finish(),
            Self::ExternalAuth {
                auth_url,
                session_id,
            } => f
                .debug_struct("ExternalAuth")
                .field("auth_url", &host_only(auth_url))
                .field("session_id", &truncate_id(session_id))
                .finish(),
        }
    }
}

/// `scheme://host` of a URL-shaped string — everything after the authority's
/// host (path, query, userinfo) is dropped for safe display.
fn host_only(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return "[non-url]".to_owned();
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    // Strip userinfo if present — never render `user:pass@`.
    let host = authority.rsplit('@').next().unwrap_or("");
    format!("{scheme}://{host}/…")
}

/// The first few characters of an opaque id, for log-safe display.
fn truncate_id(id: &str) -> String {
    let head: String = id.chars().take(8).collect();
    if id.chars().count() > 8 {
        format!("{head}…")
    } else {
        head
    }
}

/// An established connection an adapter syncs against. Deliberately minimal:
/// the credential must be self-contained (see [`LinkSession`]).
#[derive(Debug, Clone)]
pub struct Connection {
    pub credential: Credential,
}

// ===========================================================================
// Health + errors — the ADR 0060 §5 taxonomy
// ===========================================================================

/// Connection health, mapping 1:1 onto UI states (`personal-cfo-ul5d`).
///
/// LEAK RULE for every `message`/`help_url` in this enum: provider messaging
/// only — never the credential, never a request URL (a user-token request URL
/// *is* the credential), never URL userinfo. Host-only or a provider error
/// code is always enough.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum HealthStatus {
    Healthy,
    /// Throttled — still healthy (ADR 0060 §5). Retry later; do not re-auth.
    RateLimited,
    /// The user must act at the provider (re-authorize an institution, claim a
    /// fresh token). Carries provider messaging, never a credential.
    NeedsUserAction {
        code: String,
        message: String,
        help_url: Option<String>,
    },
    /// The stored credential no longer works — re-link required.
    Expired,
    /// Network-level failure; nothing wrong with the credential.
    Unreachable {
        message: String,
    },
}

/// Why a connector call failed. `RateLimited` is a **healthy** state: callers
/// keep the connection active and retry later — conflating throttled with
/// broken is the documented trap this taxonomy exists to avoid (ADR 0060 §5,
/// docs/research/simplefin-feasibility.md).
///
/// LEAK RULE for every `String` payload below: the message must never contain
/// the credential, the request URL, or URL userinfo — for the user-token tier
/// the request URL *is* the credential. Pass host-only (see how `Debug` for
/// [`LinkSession`] does it) or a provider error code.
#[derive(Debug, thiserror::Error)]
pub enum ConnectorError {
    /// The user must act at the provider; surface `message` + `help_url`.
    #[error("user action required ({code}): {message}")]
    NeedsUserAction {
        code: String,
        message: String,
        help_url: Option<String>,
    },
    /// The credential is no longer valid — the caller offers the re-link flow.
    #[error("connection expired: {0}")]
    Expired(String),
    /// Throttled by the provider. Healthy; retry after the provider's window.
    #[error("rate limited: {0}")]
    RateLimited(String),
    /// The feature asked for something this adapter cannot do
    /// (personal-cfo-x6dr AC: typed, never a crash).
    #[error("adapter does not support capability {0:?}")]
    CapabilityMissing(Capability),
    /// Network-level failure (DNS, TLS, timeout). Describe the failure class,
    /// not the request — transport errors love to embed full URLs.
    #[error("network error: {0}")]
    Network(String),
    /// The provider answered with something unexpected.
    #[error("provider error: {0}")]
    Provider(String),
}

// ===========================================================================
// Sync output — provenance-stamped staged candidates
// ===========================================================================

/// One sync's output: a [`ParsedBatch`] of staged candidates stamped with the
/// adapter's id + version. The pipeline records these on the `source_batch` /
/// `parser_run` rows it creates (plan §8.1.3; row-level persistence is owned
/// by the SimpleFIN arc — see personal-cfo-w3gh's AC; test here:
/// `tests/adapter_provenance.rs`), exactly as it records importer plugin
/// provenance today.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncBatch {
    /// [`ConnectorAdapter::id`] of the producing adapter.
    pub adapter_id: String,
    /// [`ConnectorAdapter::version`] of the producing adapter, as a string.
    pub adapter_version: String,
    /// The `since` bound this sync used; `None` = full history (first sync).
    pub since: Option<NaiveDate>,
    /// Staged candidates — same shape file importers produce (ADR 0008).
    /// `batch.source_format` is the adapter's [`ConnectorAdapter::id`], which
    /// must be one of the `source_batches.source_type` schema tokens
    /// (`simplefin`/`teller`/`plaid`/`relay`/…).
    pub batch: ParsedBatch,
    /// Provider account ids whose windows the provider reported incomplete
    /// (retry-required: `act.failed` / `act.missingdata`) — the caller must
    /// NOT advance those accounts' since-watermarks. Structured on purpose:
    /// watermark holds must never depend on parsing warning prose.
    #[serde(default)]
    pub held_account_ids: Vec<String>,
    /// A retry-required condition with no account scope — hold every
    /// watermark for this sync.
    #[serde(default)]
    pub hold_all_watermarks: bool,
}

/// Wrap a staged balance observation into the [`ParsedRecord`] shape the
/// staging pipeline expects, following the house `source_record` contract:
/// `normalized_json` is the record's normalized fields (shred-after-parse,
/// ADR 0014 §4) and `source_hash` is a `sha256:` content fingerprint of
/// `"{index}:{normalized_json}"` — `index` is the record's position in the
/// sync, so two identical-looking balances stay distinct records instead of
/// silently collapsing in `insert_source_record`'s per-batch dedupe.
///
/// `pub` so adapters that override [`ConnectorAdapter::sync`] (single-call
/// providers like SimpleFIN) compose the exact same wrapping.
#[must_use]
pub fn balance_record(index: usize, balance: ParsedBalance) -> ParsedRecord {
    let normalized_json = serde_json::to_string(&balance)
        .unwrap_or_else(|_| "{\"balance_serialization_failed\":true}".to_owned());
    ParsedRecord {
        external_id: None,
        source_hash: content_fingerprint(format!("{index}:{normalized_json}").as_bytes()),
        normalized_json,
        parse_confidence_bps: Some(10_000),
        transaction: None,
        balance: Some(balance),
    }
}

// ===========================================================================
// The trait
// ===========================================================================

/// A statically-registered bank-sync adapter (personal-cfo-x6dr, ADR 0060).
/// Implementations are stateless singletons registered as
/// `&'static dyn ConnectorAdapter` via [`register_connector!`].
///
/// Rules every adapter inherits (ADR 0004 / ADR 0008 / plan §8.3):
/// - output is staged candidates only — the ledger is written by the ingestion
///   pipeline after dedupe + review, never by an adapter;
/// - credentials are user-owned, arrive via [`LinkInput`]/[`Connection`], and
///   are never logged or serialized;
/// - no provider concept leaks out of the adapter (`normalized_json` carries
///   provider payloads as *data*, typed fields stay provider-neutral).
pub trait ConnectorAdapter: Sync {
    /// Stable, unique id (e.g. `"simplefin"`), recorded on every
    /// `source_batch`/`parser_run` this adapter feeds. Must be one of the
    /// `source_batches.source_type` schema tokens. Never changes.
    fn id(&self) -> &'static str;

    /// Human-facing name.
    fn display_name(&self) -> &'static str;

    /// Adapter version, recorded alongside [`Self::id`] for provenance.
    fn version(&self) -> Version;

    /// What this adapter can fetch. Feature code gates on this via
    /// [`CapabilitySet::ensure`] before calling the matching `fetch_*`.
    fn capabilities(&self) -> CapabilitySet;

    /// Establish a connection from user-supplied input (paste-token claim for
    /// the user-token tier; a system-browser URL for relay-tier providers).
    ///
    /// # Errors
    /// Any [`ConnectorError`]; a reused single-use token surfaces as
    /// [`ConnectorError::NeedsUserAction`] prompting a fresh token.
    fn link(&self, input: &LinkInput) -> Result<LinkSession, ConnectorError>;

    /// External accounts visible on this connection. Adapters set
    /// [`ParsedAccount::external_id`] to the provider's stable account id —
    /// it is the key the fetch methods below are called with.
    ///
    /// # Errors
    /// Any [`ConnectorError`].
    fn fetch_accounts(&self, conn: &Connection) -> Result<Vec<ParsedAccount>, ConnectorError>;

    /// Transactions for one external account as staged candidate records.
    /// `account_external_id` matches [`ParsedAccount::external_id`] from
    /// [`Self::fetch_accounts`] (falling back to `external_name` only for
    /// providers with no stable id). `since` of `None` means full history —
    /// the first sync.
    ///
    /// # Errors
    /// Any [`ConnectorError`].
    fn fetch_transactions(
        &self,
        conn: &Connection,
        account_external_id: &str,
        since: Option<NaiveDate>,
    ) -> Result<Vec<ParsedRecord>, ConnectorError>;

    /// Current balance observations for one external account (keyed like
    /// [`Self::fetch_transactions`]).
    ///
    /// # Errors
    /// Any [`ConnectorError`].
    fn fetch_balances(
        &self,
        conn: &Connection,
        account_external_id: &str,
    ) -> Result<Vec<ParsedBalance>, ConnectorError>;

    /// Connection health, for the health surface (`personal-cfo-ul5d`).
    ///
    /// # Errors
    /// Any [`ConnectorError`] — but transport-level trouble should map into a
    /// returned [`HealthStatus`] where possible, not an `Err`.
    fn health(&self, conn: &Connection) -> Result<HealthStatus, ConnectorError>;

    /// Invalidate/forget the provider side of this connection where the
    /// provider supports it. The user-token tier may have nothing to revoke
    /// remotely — the local credential is simply forgotten (default: `Ok`).
    ///
    /// # Errors
    /// Any [`ConnectorError`].
    fn revoke(&self, _conn: &Connection) -> Result<(), ConnectorError> {
        Ok(())
    }

    /// One full sync: accounts, then per-account transactions and balances
    /// (each gated on [`Self::capabilities`]), merged into a
    /// provenance-stamped [`SyncBatch`]. `since` of `None` = full history.
    ///
    /// Accounts without an id (`external_id` and `external_name` both `None`)
    /// cannot be fetched against; they are surfaced as batch warnings, never
    /// silently dropped (ADR 0014's replayable-never-silent ethos).
    ///
    /// Adapters whose provider returns everything in one call (SimpleFIN's
    /// `/accounts`) should override this with the single-request version —
    /// [`balance_record`] is public so the wrapping stays identical.
    ///
    /// # Errors
    /// Any [`ConnectorError`] from the underlying fetches.
    fn sync(
        &self,
        conn: &Connection,
        since: Option<NaiveDate>,
    ) -> Result<SyncBatch, ConnectorError> {
        let capabilities = self.capabilities();
        capabilities.ensure(Capability::Accounts)?;
        let accounts = self.fetch_accounts(conn)?;

        let mut records = Vec::new();
        let mut warnings = Vec::new();
        let mut balance_index = 0_usize;
        for account in &accounts {
            let Some(key) = account
                .external_id
                .as_deref()
                .or(account.external_name.as_deref())
            else {
                warnings.push(ParseWarning {
                    row: None,
                    message: "account with no external id or name was skipped during sync"
                        .to_owned(),
                });
                continue;
            };
            if capabilities.supports(Capability::Transactions) {
                records.extend(self.fetch_transactions(conn, key, since)?);
            }
            if capabilities.supports(Capability::Balances) {
                for balance in self.fetch_balances(conn, key)? {
                    records.push(balance_record(balance_index, balance));
                    balance_index += 1;
                }
            }
        }

        Ok(SyncBatch {
            adapter_id: self.id().to_owned(),
            adapter_version: self.version().to_string(),
            since,
            held_account_ids: Vec::new(),
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

// ===========================================================================
// Registry — compile-time, via `inventory` (mirrors importer-core)
// ===========================================================================

/// A compile-time adapter registration. Created by [`register_connector!`] —
/// never constructed by hand.
pub struct ConnectorRegistration {
    pub adapter: &'static dyn ConnectorAdapter,
}

inventory::collect!(ConnectorRegistration);

// Re-export `inventory` so [`register_connector!`] resolves it without the
// downstream crate having to name `inventory` itself.
pub use inventory;

/// Register a connector adapter at compile time.
///
/// ```ignore
/// use connector_core::{register_connector, ConnectorAdapter};
/// struct SimpleFin;
/// impl ConnectorAdapter for SimpleFin { /* … */ }
/// register_connector!(SimpleFin);
/// ```
#[macro_export]
macro_rules! register_connector {
    ($adapter:expr) => {
        $crate::inventory::submit! {
            $crate::ConnectorRegistration { adapter: &$adapter }
        }
    };
}

/// Every registered adapter, in registration order.
pub fn all_connectors() -> impl Iterator<Item = &'static dyn ConnectorAdapter> {
    inventory::iter::<ConnectorRegistration>
        .into_iter()
        .map(|r| r.adapter)
}

/// Look up an adapter by its stable id.
#[must_use]
pub fn connector_by_id(id: &str) -> Option<&'static dyn ConnectorAdapter> {
    all_connectors().find(|a| a.id() == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Pin the type-level redaction guarantees: a future "helpful" derive of
    // Serialize/Display on Credential would silently void the crate's §6.6
    // claim — make it a compile error instead.
    static_assertions::assert_not_impl_any!(Credential: serde::Serialize, serde::de::DeserializeOwned, std::fmt::Display);
    static_assertions::assert_not_impl_any!(Connection: serde::Serialize, serde::de::DeserializeOwned);
    static_assertions::assert_not_impl_any!(LinkSession: serde::Serialize, serde::de::DeserializeOwned);
    static_assertions::assert_not_impl_any!(LinkInput: serde::Serialize, serde::de::DeserializeOwned);

    const CANARY: &str = "CFO-CANARY-9f2d7c1e";

    #[test]
    fn credential_bearing_types_never_debug_print_the_secret() {
        let credential = Credential::new(CANARY);
        assert_eq!(format!("{credential:?}"), "Credential([redacted])");

        let input = LinkInput {
            user_token: Some(credential.clone()),
            params: BTreeMap::from([("institution".to_owned(), "demo".to_owned())]),
        };
        assert!(!format!("{input:?}").contains(CANARY));

        let conn = Connection {
            credential: credential.clone(),
        };
        assert!(!format!("{conn:?}").contains(CANARY));

        let session = LinkSession::Established {
            credential,
            display_hint: Some("2 accounts".to_owned()),
        };
        assert!(!format!("{session:?}").contains(CANARY));
    }

    #[test]
    fn external_auth_debug_is_host_only_and_truncated() {
        let session = LinkSession::ExternalAuth {
            auth_url: format!("https://user:{CANARY}@link.example/session?token={CANARY}"),
            session_id: format!("sess-{CANARY}"),
        };
        let debug = format!("{session:?}");
        assert!(!debug.contains(CANARY), "leaked: {debug}");
        assert!(debug.contains("https://link.example/…"), "got: {debug}");
    }

    #[test]
    fn capability_ensure_is_a_typed_error_not_a_crash() {
        let caps = CapabilitySet {
            accounts: true,
            transactions: true,
            ..CapabilitySet::default()
        };
        assert!(caps.ensure(Capability::Transactions).is_ok());
        let err = caps.ensure(Capability::Holdings).unwrap_err();
        assert!(matches!(
            err,
            ConnectorError::CapabilityMissing(Capability::Holdings)
        ));
    }

    #[test]
    fn default_sync_composes_and_stamps_provenance() {
        let adapter = mock::MockConnector::with_fixture();
        let conn = Connection {
            credential: Credential::new("mock-access-url"),
        };
        let since = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let synced = adapter.sync(&conn, Some(since)).unwrap();

        assert_eq!(synced.adapter_id, adapter.id());
        assert_eq!(synced.adapter_version, adapter.version().to_string());
        assert_eq!(synced.since, Some(since));
        assert_eq!(synced.batch.source_format, adapter.id());
        assert!(!synced.batch.accounts.is_empty());
        assert!(synced.batch.warnings.is_empty());
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
        assert!(txns > 0, "fixture transactions expected");
        assert!(balances > 0, "fixture balances expected");
    }

    #[test]
    fn balance_records_follow_the_house_source_record_contract() {
        use core_money::{Currency, Money};
        let balance = ParsedBalance {
            observed_at: NaiveDate::from_ymd_opt(2026, 8, 4).unwrap(),
            amount: Money::new(1000, Currency::Usd),
            external_account: None,
        };
        let first = balance_record(0, balance.clone());
        let second = balance_record(1, balance);
        assert!(first.source_hash.starts_with("sha256:"));
        assert_ne!(
            first.source_hash, second.source_hash,
            "identical balances at different positions must stay distinct records"
        );
        let parsed: serde_json::Value = serde_json::from_str(&first.normalized_json).unwrap();
        assert!(parsed.get("observed_at").is_some());
    }

    #[test]
    fn accounts_without_any_id_surface_as_warnings_not_silent_drops() {
        let adapter = mock::MockConnector::with_unnamed_account();
        let conn = Connection {
            credential: Credential::new("mock-access-url"),
        };
        let synced = adapter.sync(&conn, None).unwrap();
        assert_eq!(synced.since, None);
        assert!(
            synced
                .batch
                .warnings
                .iter()
                .any(|w| w.message.contains("skipped during sync")),
            "expected a skip warning, got {:?}",
            synced.batch.warnings
        );
    }
}
