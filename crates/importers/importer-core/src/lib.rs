//! `importer-core` — the `ImporterPlugin` contract + a compile-time plugin
//! registry (personal-cfo-eay).
//!
//! Every importer (CSV/OFX/QFX/QIF now; more later) implements
//! [`ImporterPlugin`]: it receives **untrusted bytes** and returns a typed
//! [`ParsedBatch`] of *staged candidates* — never committed ledger rows. Per
//! ADR 0022 a plugin holds **no keys, DB handles, network, or IPC**; the
//! isolation harness (personal-cfo-hs9) enforces the hard resource bounds
//! around [`ImporterPlugin::parse`]. The pipeline (personal-cfo-cmx) takes the
//! [`ParsedBatch`], stages it into the `ihe` schema, dedupes, and commits.
//!
//! Plugins are registered **at compile time** with [`register_importer!`] (built
//! on the `inventory` crate). There is no runtime registration map to populate,
//! and ADR 0022 forbids runtime dynamic loading (libloading/dlopen) — the plugin
//! roster is fixed when the binary is built.
//!
//! A second, smaller registry — [`SourcePreset`]/[`register_preset!`] — declares
//! per-source-app export shapes (YNAB, Monarch Money, …) applied by the generic
//! CSV importer before a user's own column mapping (personal-cfo-gvidg). A
//! preset is a set of hints over the existing importer architecture, not a new
//! parser: most personal-finance apps export CSV with stable headers, so for
//! most sources a preset is the whole job.

use chrono::NaiveDate;
use core_money::{Currency, Money};
use semver::Version;
use serde::{Deserialize, Serialize};

mod host;
pub use host::{run_bounded, ParserLimits, ParserRunReport, RunStatus};

/// The whole-file content fingerprint used for file-level dedupe (ADR 0014 §3):
/// `"sha256:<hex>"` of the raw bytes. An exact re-upload yields the same
/// fingerprint, so the pipeline can skip it without re-parsing.
#[must_use]
pub fn content_fingerprint(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write as _;
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(2 + digest.len() * 2);
    hex.push_str("sha256:");
    for byte in digest {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

// ===========================================================================
// Input
// ===========================================================================

/// Untrusted bytes handed to a parser, plus minimal context. A parser receives
/// **bytes** and returns typed records (ADR 0022); the hard limits
/// (size/memory/time/rows) are enforced by the isolation harness around
/// [`ImporterPlugin::parse`], not inside the plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParserInput {
    /// Original filename, when known (its extension drives auto-detection).
    pub filename: Option<String>,
    /// Declared source type / format hint (from the user or the source_type).
    pub declared_format: Option<String>,
    /// The raw file bytes — untrusted.
    pub bytes: Vec<u8>,
}

impl ParserInput {
    /// A bare input over `bytes` (no filename/format hint).
    #[must_use]
    pub fn new(bytes: Vec<u8>) -> Self {
        Self {
            filename: None,
            declared_format: None,
            bytes,
        }
    }

    #[must_use]
    pub fn with_filename(mut self, name: impl Into<String>) -> Self {
        self.filename = Some(name.into());
        self
    }

    #[must_use]
    pub fn with_declared_format(mut self, fmt: impl Into<String>) -> Self {
        self.declared_format = Some(fmt.into());
        self
    }

    /// The lowercased file extension (no dot), if the filename has one.
    #[must_use]
    pub fn extension(&self) -> Option<String> {
        self.filename
            .as_ref()
            .and_then(|f| f.rsplit_once('.'))
            .map(|(_, ext)| ext.to_ascii_lowercase())
    }
}

/// User/template guidance for a parse: the CSV column mapping, an explicit date
/// format, a default currency, an institution hint.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ParserHints {
    pub column_mapping: Option<ColumnMapping>,
    /// An explicit date format (e.g. `%m/%d/%Y`) chosen per source to disambiguate.
    pub date_format: Option<String>,
    /// Currency to assume when the source does not carry one per row.
    pub default_currency: Option<Currency>,
    pub institution: Option<String>,
}

/// Maps source CSV columns (by header name) onto canonical transaction fields.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ColumnMapping {
    pub date: Option<String>,
    pub description: Option<String>,
    pub amount: Option<String>,
    pub debit: Option<String>,
    pub credit: Option<String>,
    pub account: Option<String>,
    pub category: Option<String>,
    /// A second, coarser category column some sources export alongside
    /// `category` (e.g. YNAB's "Category Group"), combined as
    /// `"{category_group}: {category}"` when both resolve on a row
    /// (personal-cfo-gvidg). `None` for sources with a single category
    /// column — the existing `category`-only behavior is unchanged.
    pub category_group: Option<String>,
    pub currency: Option<String>,
    pub memo: Option<String>,
}

// ===========================================================================
// Output — staged candidates, mapping onto the `ihe` staging schema
// ===========================================================================

/// The normalized result of one parse: staged candidates, not committed rows.
/// Maps onto the `ihe` staging schema — `accounts` → `staged_accounts`, each
/// [`ParsedRecord`] → a `source_record` (+ an optional `staged_transaction` /
/// `staged_balance`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedBatch {
    /// The plugin's source-format token (e.g. `"csv"`, `"ofx"`).
    pub source_format: String,
    /// External accounts observed in the source, to be matched to real accounts.
    pub accounts: Vec<ParsedAccount>,
    /// One entry per parsed row / object.
    pub records: Vec<ParsedRecord>,
    /// Non-fatal issues a human may want to review (ambiguous date, odd row).
    pub warnings: Vec<ParseWarning>,
}

/// One parsed row / provider object → a `source_record`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedRecord {
    /// Provider/external id, when the source carries one.
    pub external_id: Option<String>,
    /// Content fingerprint of this record (the `source_record` dedupe key).
    pub source_hash: String,
    /// The normalized fields as JSON. This is persisted; the raw bytes are
    /// **not** (ADR 0014 §4 shred-after-parse).
    pub normalized_json: String,
    /// Parser confidence for this record, in basis points (0..=10000).
    pub parse_confidence_bps: Option<u16>,
    /// A proposed transaction, when this record is one.
    pub transaction: Option<ParsedTransaction>,
    /// A proposed balance observation, when this record is one (ADR 0027).
    pub balance: Option<ParsedBalance>,
}

/// A proposed transaction → a `staged_transaction`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedTransaction {
    /// Parsed **posted** date — when the money moved. The PRIMARY date (ADR 0045):
    /// the account's `occurred_at`, the list's main date, and the dedupe key.
    pub posted_date: NaiveDate,
    /// The transaction / authorization date, when the source carries a distinct one
    /// (ADR 0045). Secondary + descriptive — shown in the detail view, never used for
    /// math or dedupe. `None` when the source has a single date column.
    #[serde(default)]
    pub transaction_date: Option<NaiveDate>,
    /// The raw date string from the source, preserved for audit / ambiguity.
    pub raw_date: String,
    /// Confidence of the date parse, in basis points (0..=10000).
    pub date_confidence_bps: u16,
    /// Signed amount (inflow +, outflow −).
    pub amount: Money,
    pub description: Option<String>,
    /// The source's own category for this row, when present (e.g. a CapitalOne
    /// `Category` column) — a PREFILL, not a user choice (ADR 0045 §3). `None` when
    /// the source has no category. The full raw row is also in `normalized_json`.
    #[serde(default)]
    pub category: Option<String>,
    /// Normalized merchant (part of the transaction dedupe fingerprint).
    pub normalized_merchant: Option<String>,
    /// External account label this row belongs to (matched to a real account later).
    pub external_account: Option<String>,
    /// date + amount + normalized merchant + account (ADR 0014 §3 dedupe key).
    pub txn_fingerprint: String,
}

/// An external account observed in the source → a `staged_account`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedAccount {
    /// The source's **stable, opaque account id** when it has one (an OFX
    /// `ACCTID` hash-safe token, a connector provider's account id). Distinct
    /// from `external_name`, which is a display label and neither unique nor
    /// stable. Connectors key per-account fetches on this (personal-cfo-x6dr);
    /// file importers may leave it `None`.
    #[serde(default)]
    pub external_id: Option<String>,
    pub external_name: Option<String>,
    /// A hash of the account number — never the number itself.
    pub external_number_hash: Option<String>,
    /// Proposed subtype token (`checking`/`savings`/`credit_card`/…), matched later.
    pub proposed_subtype: Option<String>,
}

/// An observed ending balance → a `staged_balance` (becomes a balance assertion
/// on commit, ADR 0027).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedBalance {
    pub observed_at: NaiveDate,
    pub amount: Money,
    pub external_account: Option<String>,
}

/// A non-fatal parse issue surfaced for human review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParseWarning {
    /// Source row number, when applicable.
    pub row: Option<usize>,
    pub message: String,
}

// ===========================================================================
// Errors
// ===========================================================================

/// Why a parse failed. A failure leaves a `parser_run` record and no staged
/// rows (ADR 0008 / personal-cfo-hs9).
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    /// A resource bound was exceeded (rows/size/etc.) — fail cleanly, never OOM.
    #[error("input exceeds a parser limit: {0}")]
    LimitExceeded(String),
    /// The bytes are not valid for this format.
    #[error("malformed input: {0}")]
    Malformed(String),
    /// The input is structurally valid but this plugin cannot handle it.
    #[error("unsupported input: {0}")]
    Unsupported(String),
}

// ===========================================================================
// The trait
// ===========================================================================

/// A statically-registered importer. Implementations are stateless singletons
/// (registered as `&'static dyn ImporterPlugin` via [`register_importer!`]) that
/// turn untrusted bytes into a [`ParsedBatch`]. They must hold **no** keys, DB
/// handles, network, or IPC (ADR 0022); the isolation harness
/// (personal-cfo-hs9) enforces the hard bounds around [`parse`](Self::parse).
pub trait ImporterPlugin: Sync {
    /// Stable, unique id, recorded on every `parser_run`. Never changes.
    fn id(&self) -> &'static str;

    /// Human-facing name.
    fn display_name(&self) -> &'static str;

    /// Plugin version, recorded on every `parser_run` alongside the id.
    fn version(&self) -> Version;

    /// File extensions this plugin handles (lowercase, no dot), used for detection.
    fn supported_extensions(&self) -> &'static [&'static str];

    /// `0..=10000` bps: how confident this plugin is that it can parse `input`.
    /// `0` means "not mine". The registry offers the highest-confidence match.
    fn detect_confidence(&self, input: &ParserInput) -> u16;

    /// Source column headers the plugin understands, to seed the mapping UI.
    fn column_mapping_hints(&self) -> &'static [&'static str] {
        &[]
    }

    /// The actual source column headers in `input`, in source order, for a
    /// column-mapping UI (personal-cfo-4d8.24.1.2). These are the exact strings the
    /// plugin's [`parse`](Self::parse) will match a [`ColumnMapping`] against, so a UI
    /// built from them maps reliably. Empty for formats without user-mappable columns
    /// (e.g. OFX, whose fields are fixed tags). Default: none.
    fn preview_columns(&self, _input: &ParserInput) -> Vec<String> {
        Vec::new()
    }

    /// The institution this plugin is specialized for, if any.
    fn institution_hint(&self) -> Option<&'static str> {
        None
    }

    /// Parse untrusted bytes into staged candidates, or a typed [`ParseError`].
    fn parse(&self, input: &ParserInput, hints: &ParserHints) -> Result<ParsedBatch, ParseError>;
}

// ===========================================================================
// Registry — compile-time, via `inventory`
// ===========================================================================

/// A compile-time plugin registration. Created by [`register_importer!`] — never
/// constructed by hand. `inventory` gathers every submission into a static set,
/// so the plugin roster is fixed at compile time: there is no runtime
/// registration map to populate (and thus none to mistype), and ADR 0022's ban
/// on runtime dynamic loading is upheld by construction.
pub struct PluginRegistration {
    pub plugin: &'static dyn ImporterPlugin,
}

inventory::collect!(PluginRegistration);

// Re-export `inventory` so [`register_importer!`] resolves it without the
// downstream crate having to name `inventory` itself.
pub use inventory;

/// Register an importer plugin at compile time.
///
/// ```ignore
/// use importer_core::{register_importer, ImporterPlugin};
/// struct MyCsv;
/// impl ImporterPlugin for MyCsv { /* … */ }
/// register_importer!(MyCsv);
/// ```
#[macro_export]
macro_rules! register_importer {
    ($plugin:expr) => {
        $crate::inventory::submit! {
            $crate::PluginRegistration { plugin: &$plugin }
        }
    };
}

/// Every registered plugin, in registration order.
pub fn all_plugins() -> impl Iterator<Item = &'static dyn ImporterPlugin> {
    inventory::iter::<PluginRegistration>
        .into_iter()
        .map(|r| r.plugin)
}

/// Look up a plugin by its stable id.
#[must_use]
pub fn plugin_by_id(id: &str) -> Option<&'static dyn ImporterPlugin> {
    all_plugins().find(|p| p.id() == id)
}

/// The highest-confidence plugin for `input`, if any claims it
/// (`detect_confidence > 0`). Ties resolve to the first registered.
#[must_use]
pub fn detect_best(input: &ParserInput) -> Option<&'static dyn ImporterPlugin> {
    all_plugins()
        .map(|p| (p.detect_confidence(input), p))
        .filter(|(confidence, _)| *confidence > 0)
        .max_by_key(|(confidence, _)| *confidence)
        .map(|(_, plugin)| plugin)
}

// ===========================================================================
// Source presets — per-app export-shape declarations (personal-cfo-gvidg)
// ===========================================================================

/// How a source app's export encodes amount direction. Documentation only —
/// the CSV importer's `resolve_amount` already handles a signed `amount`
/// column and a debit/credit split generically from whichever columns a
/// preset's [`ColumnMapping`] points at; this names which shape a given
/// source actually uses, for the preset author and the migrate guide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignConvention {
    /// One amount column, negative = outflow (DohFlow's own convention).
    SignedAmount,
    /// Separate outflow/inflow columns (mapped as `debit`/`credit`).
    SeparateOutflowInflow,
    /// One amount column where the source's own sign means the opposite of
    /// DohFlow's convention (rare; not handled automatically — a preset with
    /// this convention must document the flip in its quirks for now).
    NegativeIsCredit,
}

/// Whether a source's category column(s) become a transaction's category
/// prefill, and how.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CategoryHandling {
    /// The source has no category column.
    None,
    /// One category column, mapped via `ColumnMapping::category`.
    SingleColumn,
    /// Two columns — a coarser group and a specific category — combined via
    /// `ColumnMapping::category_group` + `category`.
    GroupAndCategory,
}

/// Whether a source's account column (when present) should split one export
/// into multiple [`ParsedAccount`]s, or the source is assumed one-file-per-
/// account (today's behavior for every source with no account column).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountHandling {
    /// No account column; the whole file targets one account (today's only
    /// behavior for `generic-csv`, unchanged).
    OneFilePerAccount,
    /// An account column present — the importer stages one [`ParsedAccount`]
    /// per distinct value and stamps each row's `external_account`.
    /// Routing those into DIFFERENT real DohFlow accounts at commit time is
    /// NOT part of this preset mechanism (see this crate's module docs) —
    /// today every row still commits into whichever single account the user
    /// picked, the same as `OneFilePerAccount`; only the *parsed* data is
    /// richer.
    AccountColumn,
}

/// A known quirk in a source's export, surfaced in the migrate guide and
/// worth a dedicated row in the preset's test fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceQuirk {
    /// Amounts use a thousands separator (already handled generically by
    /// the CSV importer's `parse_minor_units` — named here so a preset's
    /// fixture exercises it deliberately, not by accident).
    ThousandsSeparator,
    /// Negative amounts are written in parentheses, accounting-style
    /// (also already handled generically — same rationale).
    ParenthesesNegative,
    /// The export carries a memo/notes column distinct from the main
    /// description.
    MemoColumn,
    /// The export includes transfer rows (moves between the user's own
    /// accounts), which a migrate guide should tell the user to expect and
    /// review rather than treat as external income/spending.
    TransferRows,
    /// The export includes split-transaction rows (one purchase, multiple
    /// categories) that don't have a DohFlow-side equivalent on first
    /// import and need manual re-splitting after.
    SplitRows,
    /// The export marks pending/uncleared rows distinctly from posted ones.
    PendingFlag,
}

/// A compile-time declared preset for one source app's export shape, applied
/// by the generic CSV importer's caller *before* the user's own column
/// mapping (which can still override any field the preset guessed) —
/// personal-cfo-gvidg, mirroring [`register_importer!`]'s trait+registry
/// shape exactly. A trait, not a plain data struct: `inventory::submit!`
/// requires const-evaluable statics, which rules out a struct literal
/// holding `Option<String>` fields directly — the same reason
/// [`ImporterPlugin`]/`ConnectorAdapter` are traits returning owned data
/// from methods rather than struct literals.
///
/// SCOPE NOTE on [`AccountHandling::AccountColumn`]: this mechanism parses
/// and stages an account column's distinct values as [`ParsedAccount`]s and
/// stamps `external_account` on each row — but does not itself route
/// different rows into different *real* DohFlow accounts at commit time.
/// `crates/db-worker`'s `stage_parsed_batch` (the file-import commit path)
/// still targets one account for the whole batch, same as today;
/// `stage_sync_batch` (the connector-sync path) already supports true
/// per-row account routing via an external-key → real-account map, and is
/// the natural mechanism to reuse for this — deliberately left to whichever
/// bead does the first `AccountColumn`-handling preset's full end-to-end
/// commit path (e.g. `personal-cfo-tulv`'s YNAB guide), not built here.
pub trait SourcePreset: Sync {
    /// Stable, unique id (e.g. `"ynab"`). Never changes.
    fn id(&self) -> &'static str;

    /// Human-facing name (e.g. `"YNAB"`).
    fn display_name(&self) -> &'static str;

    /// The source app's own homepage or export-docs URL, shown in the
    /// migrate guide and the "Import from" picker.
    fn source_app_url(&self) -> &'static str;

    /// The hints applied before the user's own mapping. Built fresh per
    /// call (not stored) since [`ParserHints`]/[`ColumnMapping`] hold owned
    /// `String`s.
    fn hints(&self) -> ParserHints;

    fn sign_convention(&self) -> SignConvention;
    fn category_handling(&self) -> CategoryHandling;
    fn account_handling(&self) -> AccountHandling;

    /// Known quirks this source's export has. Default: none.
    fn quirks(&self) -> &'static [SourceQuirk] {
        &[]
    }

    /// The export headers this preset declares, as verified against the
    /// source app's own documentation — `"<what was checked> — <date>"`,
    /// e.g. `"YNAB Register export docs, support.ynab.com — 2026-09-19"`.
    /// A one-time provenance stamp per preset (re-stamped only when the
    /// export shape is re-verified), distinct from ADR 0015's cost/terms
    /// review cadence, which this preset mechanism has no equivalent of —
    /// an export column layout doesn't carry a cost or terms to go stale.
    fn verified_against(&self) -> &'static str;

    /// Slug of the migrate guide this preset links to: renders as
    /// `/help/migrate/<help_slug>` once `personal-cfo-y0o0x`'s migrate
    /// collection (or a flat help article, its own stated fallback) carries
    /// that page. Not validated against a real URL by this crate — the
    /// frontend's own link-check gates do that.
    fn help_slug(&self) -> &'static str;

    /// A synthesized fixture (never a real user file) exercising this
    /// preset's known shape and quirks, in the source's own documented
    /// export format — required so the preset test harness
    /// (personal-cfo-gvidg) can generically assert every registered preset
    /// has one, and so `parse()` applied through this preset's `hints()`
    /// has something to run against without per-preset harness boilerplate.
    fn fixture_csv(&self) -> &'static str;
}

/// A compile-time preset registration. Created by [`register_preset!`] —
/// never constructed by hand.
pub struct PresetRegistration {
    pub preset: &'static dyn SourcePreset,
}

inventory::collect!(PresetRegistration);

/// Register a source preset at compile time.
///
/// ```ignore
/// use importer_core::{register_preset, SourcePreset};
/// struct Ynab;
/// impl SourcePreset for Ynab { /* … */ }
/// register_preset!(Ynab);
/// ```
#[macro_export]
macro_rules! register_preset {
    ($preset:expr) => {
        $crate::inventory::submit! {
            $crate::PresetRegistration { preset: &$preset }
        }
    };
}

/// Every registered preset, in registration order.
pub fn all_presets() -> impl Iterator<Item = &'static dyn SourcePreset> {
    inventory::iter::<PresetRegistration>
        .into_iter()
        .map(|r| r.preset)
}

/// Look up a preset by its stable id.
#[must_use]
pub fn preset_by_id(id: &str) -> Option<&'static dyn SourcePreset> {
    all_presets().find(|p| p.id() == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_is_lowercased_and_dotless() {
        let input = ParserInput::new(vec![]).with_filename("Statement.CSV");
        assert_eq!(input.extension().as_deref(), Some("csv"));
        assert_eq!(ParserInput::new(vec![]).extension(), None);
    }

    #[test]
    fn parsed_batch_round_trips_through_serde() {
        let batch = ParsedBatch {
            source_format: "csv".to_owned(),
            accounts: vec![ParsedAccount {
                external_id: None,
                external_name: Some("Checking".to_owned()),
                external_number_hash: None,
                proposed_subtype: Some("checking".to_owned()),
            }],
            records: vec![ParsedRecord {
                external_id: None,
                source_hash: "h".to_owned(),
                normalized_json: "{}".to_owned(),
                parse_confidence_bps: Some(10_000),
                transaction: Some(ParsedTransaction {
                    posted_date: NaiveDate::from_ymd_opt(2026, 6, 20).unwrap(),
                    transaction_date: Some(NaiveDate::from_ymd_opt(2026, 6, 18).unwrap()),
                    raw_date: "06/20/2026".to_owned(),
                    date_confidence_bps: 10_000,
                    amount: Money::new(-1299, Currency::Usd),
                    description: Some("CAFE".to_owned()),
                    category: Some("Dining".to_owned()),
                    normalized_merchant: Some("cafe".to_owned()),
                    external_account: Some("Checking".to_owned()),
                    txn_fingerprint: "2026-06-20|-1299|cafe|Checking".to_owned(),
                }),
                balance: None,
            }],
            warnings: vec![],
        };
        let json = serde_json::to_string(&batch).unwrap();
        let back: ParsedBatch = serde_json::from_str(&json).unwrap();
        assert_eq!(batch, back);
    }
}
