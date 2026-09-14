//! The db-worker: sole owner of the SQLCipher connection (plan §2.6, §3.2,
//! §9.1.2; ADR 0011).
//!
//! Responsibilities:
//! - **Single writer.** One `rusqlite::Connection` for writes, serialized behind
//!   a mutex. Reads use separate connections so long reads never block writes
//!   (WAL mode).
//! - **Atomic op-log.** Every successful [`WriteCommand`] commits its domain
//!   mutation *and* its [`operation_log`](operation_log) row in a single SQLite
//!   transaction. A panic between the two rolls back both — no partial state.
//! - **Fail safe, not silent.** A writer panic marks the worker
//!   [`WorkerState::CorruptNeedsRecovery`] and refuses further writes, rather
//!   than risking silent corruption.
//! - **Required provenance.** Every command carries §9.1.2 metadata; missing
//!   metadata is rejected.
//!
//! This crate is the **only** one that may depend on `rusqlite` (CI-enforced).
//! The full domain schema and command set arrive via the schema beads and the
//! Finance Kernel; db-worker provides the mechanism plus a minimal representative
//! command (`CreateAccount`) used to prove the invariants.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use chrono::{DateTime, Datelike, NaiveDate, Utc};
use core_ledger::{
    Account, AccountId, AccountKind, AccountSubtype, BillContractId, CashTier, CategoryId,
    IncomeSourceId, LedgerAccountId, LedgerTransaction, OperationId, Posting, RecurringEventId,
    RecurringTransferId, SourceBatchId, SourceRecordId, SplitLineId, StagedTransactionId, TagId,
    TransactionId,
};
use core_money::{Currency, Money, MoneyError};
use importer_core::{ParsedBatch, ParserRunReport};
use pay_schedule::{Frequency, PaySchedule};
use rusqlite::{params, Connection, OptionalExtension};
use thiserror::Error;
use uuid::Uuid;
use vault_crypto::Dek;
use zeroize::{Zeroize, Zeroizing};

mod apply;
mod assumptions;
mod attachments;
mod band_drift;
mod categories;
mod connectors;
pub use connectors::{ConnectorConnectionRow, ConnectorLinkRow};
mod commitments;
mod debt;
mod forecast;
mod forecast_actualize;
mod forecast_backtest;
mod forecast_events;
mod forecast_overrides;
mod forecast_persist;
mod ingestion;
mod loan_overlap;
mod manual_entry;
mod merchant_grouping;
mod merchant_identity;
mod merchant_memory;
mod migrations;
mod money_inbox;
mod projection;
mod recurring_debt;
mod recurring_detection;
mod recurring_instances;
mod scenarios;
mod schedule_sources;
mod spend_by_category;

pub use assumptions::{
    AssumptionEventView, AssumptionKind, AssumptionSource, AssumptionStatus, DependencyEdge,
    DirtyRange, DirtyReason, EdgeType, NewAssumptionEvent,
};
pub use attachments::AttachmentMeta;
pub use categories::CategoryView;
pub use commitments::CommitmentView;
pub use money_inbox::MoneyInboxItem;

/// The schema version a fully-migrated vault is at — the highest migration this
/// build knows. A backup whose `schema_version` exceeds this is from a newer app
/// and cannot be restored (personal-cfo-au3, ADR 0024 §5).
pub const CURRENT_SCHEMA_VERSION: i64 = migrations::CURRENT_VERSION;
pub use forecast::{
    AccountAvailability, AccountHistoryView, AccountSeriesView, CapabilityUnlock, CardCycleView,
    CardStatementForecastView, CardStatementHistoryView, CashAvailability, CashFlowHistory,
    DayBalance, ForecastDayView, ForecastEventView, ForecastReadiness, ForecastView,
    GroupSeriesView, HistoryDay, MultiSeriesForecast, PayoffDebtSeries, PayoffPlanView,
    ReadinessFactor, StoredStatementView,
};

/// The `settings` key holding the household minimum-cash-floor in minor units — the
/// buffer the user wants to keep (ADR 0029, personal-cfo-fqbm). Default `0`. This is the
/// comfort band's LOWER edge (ADR 0018 addendum 915.1 wraps, not replaces, the floor).
pub const MINIMUM_CASH_FLOOR_KEY: &str = "minimum_cash_floor_minor";
/// The `settings` key holding the comfort band's UPPER edge in minor units — cash above it
/// is excess the user may choose to deploy (ADR 0018 addendum 915.1, personal-cfo-3v6d).
/// Absent means no upper edge is set.
pub const COMFORT_BAND_UPPER_KEY: &str = "comfort_band_upper_minor";
/// The `settings` key gating auto-apply of merchant memory after an import (ADR 0030
/// addendum, personal-cfo-5n4.2). `"true"`/`"false"`; **absent means on** (default ON).
pub const AUTO_CATEGORIZE_ON_IMPORT_KEY: &str = "auto_categorize_on_import";
/// The `settings` key holding the Future Cash chart's series-selection preference
/// (personal-cfo-4d8.25.26): an opaque JSON array of series keys the frontend
/// serializes. Absent means the default (the three aggregate tiers).
pub const FUTURE_CASH_SERIES_KEY: &str = "future_cash_series_selection";
pub use categorization::RecurringCandidate;
pub use debt::{DebtTermsInput, DebtTermsView, RepaymentPhilosophy};
pub use forecast_engine::{AssumptionBasis, Band};
pub use forecast_events::{AssumptionParams, ForecastAssumptionSpec};
pub use forecast_persist::{
    ForecastDiffView, InputSnapshotView, ModelRegistryView, NewForecastDiff,
};
pub use loan_overlap::LoanDoubleCount;
pub use manual_entry::ManualEntry;
pub use merchant_identity::MerchantIdentity;
pub use projection::{
    project_transaction_display, CategorySource, ReviewStatus, TransactionDisplayInput,
    TransactionDisplayRow,
};
pub use recurring_detection::{CandidateObservation, RecurringCandidateView};
pub use recurring_instances::{RecurringInstanceRow, UnconfirmedOccurrence};
pub use scenarios::{NewScenario, ScenarioStatus, ScenarioView};
pub use spend_by_category::{CategorySpend, SpendBreakdown, SpendFilters};

/// 64 MiB cap on the WAL so it cannot grow without bound (plan §3.2).
const JOURNAL_SIZE_LIMIT: i64 = 64 * 1024 * 1024;
/// How long a blocked connection waits on a lock before erroring.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
// The schema version stamped into `PRAGMA user_version` and each op-log row is
// `migrations::CURRENT_VERSION` (the highest applied migration). The version
// history now lives in `migrations::MIGRATIONS` (personal-cfo-wkn).
/// How long an idempotency key is honoured before it may be reaped.
const IDEMPOTENCY_TTL_DAYS: i64 = 30;

/// Health of the worker's writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerState {
    /// Normal operation.
    Healthy,
    /// A backup restore is in progress; writes are refused.
    RestoringBackup,
    /// A writer panic left the worker in an unknown state; writes are refused
    /// until recovery.
    CorruptNeedsRecovery,
}

/// Who issued a command (plan §9.1.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ActorType {
    /// A human user.
    User,
    /// The system itself (migrations, maintenance).
    System,
    /// An importer pipeline.
    Importer,
    /// An AI agent.
    Agent,
}

fn actor_type_str(actor: ActorType) -> &'static str {
    match actor {
        ActorType::User => "user",
        ActorType::System => "system",
        ActorType::Importer => "importer",
        ActorType::Agent => "agent",
    }
}

/// Provenance + idempotency metadata required on every command (plan §9.1.2).
#[derive(Debug, Clone)]
pub struct CommandMeta {
    /// Unique id of this command instance (op-log primary identity).
    pub command_id: Uuid,
    /// Groups commands belonging to one logical user action.
    pub correlation_id: Uuid,
    /// The command that caused this one, if any.
    pub causation_id: Option<Uuid>,
    /// Who issued the command.
    pub actor_type: ActorType,
    /// Stable id of the actor.
    pub actor_id: String,
    /// Key for idempotent retries.
    pub idempotency_key: String,
}

impl CommandMeta {
    fn validate(&self) -> Result<(), DbError> {
        if self.actor_id.trim().is_empty() {
            return Err(DbError::MissingMetadata("actor_id"));
        }
        if self.idempotency_key.trim().is_empty() {
            return Err(DbError::MissingMetadata("idempotency_key"));
        }
        Ok(())
    }
}

/// One line of a [`WriteCommand::SetSplits`] (ADR 0034, personal-cfo-e7i): a slice of
/// a transaction's amount with its own category / note / tags. A plain data carrier —
/// the apply mints the [`SplitLineId`], assigns `sort_order` from the vec position, and
/// validates that the lines' amounts sum to the transaction amount.
#[derive(Debug, Clone)]
pub struct SplitLineInput {
    /// The slice amount (same sign + currency as the transaction).
    pub amount: Money,
    /// The line's category (`None` = uncategorized).
    pub category_id: Option<CategoryId>,
    /// The line's note (`None` = no note).
    pub note: Option<String>,
    /// The line's tags.
    pub tag_ids: Vec<TagId>,
}

/// A mutating command. The enum is the exhaustive write surface, so the worker's
/// dispatch is compile-checked. More variants arrive with their feature beads.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum WriteCommand {
    /// Create a user account (with its backing ledger account). An optional
    /// opening balance is recorded as a balanced equity posting, never a column.
    CreateAccount {
        /// The account to create.
        account: Box<Account>,
        /// Optional opening balance, posted against opening-balance equity.
        opening_balance: Option<Money>,
    },
    /// Rename an account.
    UpdateAccount {
        /// The account to update.
        id: AccountId,
        /// The new display name.
        name: String,
    },
    /// Archive (soft-hide) an account. Non-destructive: postings are preserved.
    ArchiveAccount(AccountId),
    /// Reinstate a previously archived account.
    ReinstateAccount(AccountId),
    /// Set (or clear) an account's subtype (ADR 0028). `None` clears it.
    SetAccountSubtype {
        /// The account to classify.
        id: AccountId,
        /// The new subtype, or `None` to clear it.
        subtype: Option<AccountSubtype>,
    },
    /// Set (or clear) an account's free-text note (ADR 0044). `None` clears it.
    SetAccountNote {
        /// The account to annotate.
        id: AccountId,
        /// The new note, or `None` to clear it.
        note: Option<String>,
    },
    /// Set (or clear, with `None`) a real asset's link to the liability that finances it
    /// (ADR 0044 §5). The source must be a real-asset account and the target a liability
    /// (validated on apply); the link is stored on the asset row. Display-only.
    SetAccountLink {
        /// The real-asset account that owns the link.
        asset_id: AccountId,
        /// The financing liability, or `None` to clear the link.
        liability_id: Option<AccountId>,
    },
    /// Upsert a liability account's debt terms (ADR 0035 §5, personal-cfo-6wk.6). The target
    /// must be a liability account and any paying source a liquid account (validated on apply).
    SetDebtTerms {
        /// The liability account these terms apply to.
        account_id: AccountId,
        /// The settable attributes (an upsert replaces the row).
        terms: DebtTermsInput,
    },
    /// Record a card statement's REAL balance for one cycle (feedback 2026-07-03): the user
    /// knows the actual statement, so the forecast stops estimating that cycle and carries
    /// forward from the asserted number. `None` clears the assertion back to the estimate.
    SetCardStatementBalance {
        /// The credit-card account the statement belongs to (must be a `credit_facility`).
        account_id: AccountId,
        /// The statement close date identifying the cycle.
        cycle_close: chrono::NaiveDate,
        /// The actual statement balance owed (≥ 0), or `None` to clear the assertion.
        statement_balance_minor: Option<i64>,
    },
    /// Record a manual transaction: a signed money movement against an account,
    /// balanced by a system income/expense counter-account.
    RecordTransaction {
        /// The caller-minted id the new transaction is persisted under
        /// (personal-cfo-4d8.24.2.1).
        transaction_id: TransactionId,
        /// The user account the money moves against.
        account_id: AccountId,
        /// Signed amount — positive is money in (income), negative is money out
        /// (expense). Currency must match the account.
        amount: Money,
        /// When the transaction occurred.
        occurred_at: DateTime<Utc>,
    },
    /// Confirm a recurring bill occurrence paid early (personal-cfo-5ie.9, ADR 5ie.7):
    /// post a real outflow from `paying_account_id` dated `actual_date` and record the
    /// occurrence as fulfilled, so the forecast stops projecting it. Idempotent on
    /// `(recurring_event_id, scheduled_date)`. v1 = liquid-paid bill path only.
    ConfirmObligationEarly {
        /// The recurring bill whose occurrence is being confirmed.
        recurring_event_id: RecurringEventId,
        /// The occurrence's scheduled due date.
        scheduled_date: NaiveDate,
        /// The positive amount actually paid.
        actual_amount: Money,
        /// When it was actually paid.
        actual_date: DateTime<Utc>,
        /// The liquid account the payment came from.
        paying_account_id: AccountId,
    },
    /// Reverse an early confirm (personal-cfo-5ie.9): void the posted transaction and drop
    /// the fulfillment record, so the forecast projects the occurrence again. Idempotent —
    /// a no-op when the occurrence was never confirmed.
    UnconfirmObligation {
        /// The recurring bill whose confirmed occurrence is being reversed.
        recurring_event_id: RecurringEventId,
        /// The occurrence's scheduled due date.
        scheduled_date: NaiveDate,
    },
    /// Convert an account's unexplained balance adjustment (the additive-balance
    /// "plug", ADR 0027 §8, personal-cfo-dyy4) into one real transaction dated at
    /// the latest assertion, so the plug goes to zero. No-op when already explained.
    ConvertUnexplainedToTransaction {
        /// The account whose residual plug is converted.
        account_id: AccountId,
    },
    /// Record a one-off transfer between two of the user's own accounts
    /// (personal-cfo-npoe): a balanced ledger transaction debiting the source and
    /// crediting the destination — no system counter-account, aggregate cash
    /// unchanged. v1 is liquid-cash ↔ liquid-cash.
    Transfer {
        /// The account money moves out of.
        source_account_id: AccountId,
        /// The account money moves into.
        dest_account_id: AccountId,
        /// The (positive) amount moved; currency must match both accounts.
        amount: Money,
        /// When the transfer occurred.
        occurred_at: DateTime<Utc>,
    },
    /// Create a recurring account-to-account transfer (ADR 0026 §14,
    /// personal-cfo-npoe): a scheduled money movement projected in the per-account
    /// forecast (source −, destination +). v1 is liquid-cash ↔ liquid-cash.
    CreateRecurringTransfer {
        /// Caller-minted id.
        id: RecurringTransferId,
        /// The account money moves out of.
        source_account_id: AccountId,
        /// The account money moves into.
        dest_account_id: AccountId,
        /// The (positive) amount moved each occurrence.
        amount: Money,
        /// Transfer cadence.
        frequency: Frequency,
        /// The schedule anchor (a known occurrence date).
        anchor: NaiveDate,
    },
    /// Delete a recurring transfer (personal-cfo-npoe). Stops future projection;
    /// any already-posted one-off transfers are untouched.
    DeleteRecurringTransfer(RecurringTransferId),
    /// Create a recurring net-pay income source (personal-cfo-le79).
    CreateIncomeSource {
        /// The new income source's id.
        id: IncomeSourceId,
        /// Display name (e.g. the employer).
        name: String,
        /// Net (take-home) pay per occurrence, in minor units.
        net_amount: Money,
        /// Pay cadence.
        frequency: Frequency,
        /// A known pay date the cadence is anchored on.
        anchor: NaiveDate,
        /// Optional account the pay is deposited into (currency must match).
        deposit_account_id: Option<AccountId>,
    },
    /// Edit an existing net-pay income source (personal-cfo-tch0): rewrite its
    /// `income_sources` row by id.
    UpdateIncomeSource {
        /// The income source to edit.
        id: IncomeSourceId,
        /// New display name.
        name: String,
        /// New net pay per occurrence, in minor units (positive).
        net_amount: Money,
        /// New pay cadence.
        frequency: Frequency,
        /// New schedule anchor (a known pay date).
        anchor: NaiveDate,
        /// New optional deposit account (currency must match).
        deposit_account_id: Option<AccountId>,
    },
    /// Delete a net-pay income source (personal-cfo-tch0): drop its row by id.
    DeleteIncomeSource(IncomeSourceId),
    /// Archive a net-pay income source (personal-cfo-tch0): set `active = 0` +
    /// `archived_at`, dropping it from the forecast while keeping its history.
    ArchiveIncomeSource(IncomeSourceId),
    /// Restore a previously archived income source (personal-cfo-tch0): set
    /// `active = 1` + clear `archived_at`, returning it to the forecast.
    RestoreIncomeSource(IncomeSourceId),
    /// Create a manual recurring bill (plan §9.8, personal-cfo-esmy): a
    /// `recurring_event` (the schedule) plus a linked `bill_contract` (its type
    /// and due rule). The commitments projection is refreshed in the same write.
    CreateRecurringBill {
        /// The new recurring event's id (also the derived commitment id).
        event_id: RecurringEventId,
        /// The new bill contract's id.
        contract_id: BillContractId,
        /// Display name (e.g. the merchant).
        name: String,
        /// Expected outflow per occurrence, in minor units (positive).
        amount: Money,
        /// Bill-contract type token (validated at the IPC boundary).
        bill_type: String,
        /// Pay cadence.
        frequency: Frequency,
        /// A known due date the cadence is anchored on.
        anchor: NaiveDate,
        /// Optional autopay account (currency must match).
        autopay_account_id: Option<AccountId>,
        /// Optional free-text description.
        description: Option<String>,
        /// The normalized merchant key of the recurring candidate this bill was promoted
        /// from, if any (personal-cfo-5n4.8). Persisted so the suggestion is durably
        /// suppressed even after the bill is renamed — the exclusion keys on this, not
        /// the (mutable) name. `None` for a manually-created bill.
        source_merchant_key: Option<String>,
        /// The bill's category, set when promoting (personal-cfo-4d8.24.5); `None` if
        /// uncategorized. Written to `recurring_events.category_id`.
        category_id: Option<CategoryId>,
        /// Tags to apply to the bill, set when promoting (personal-cfo-4d8.24.5.1);
        /// written to `recurring_event_tags`. Empty for an untagged bill.
        tag_ids: Vec<TagId>,
    },
    /// Set whether a recurring bill autopays (ADR 0041, personal-cfo-mc7f). Intent metadata only —
    /// it does not change the projection; it drives the autopay-vs-manual UI distinction.
    SetBillAutopay {
        /// The recurring bill to flag.
        event_id: RecurringEventId,
        /// `true` = autopay, `false` = manual.
        autopay: bool,
    },
    /// Edit an existing manual recurring bill (personal-cfo-zl1l): rewrite its
    /// `recurring_event` + `bill_contract` rows by recurring-event id, then refresh
    /// the commitments projection in the same write.
    UpdateRecurringBill {
        /// The recurring event id identifying the bill to edit.
        event_id: RecurringEventId,
        /// New display name.
        name: String,
        /// New expected outflow per occurrence, in minor units (positive).
        amount: Money,
        /// New bill-contract type token (validated at the IPC boundary).
        bill_type: String,
        /// New pay cadence.
        frequency: Frequency,
        /// New schedule anchor (a known due date).
        anchor: NaiveDate,
        /// New optional autopay account (currency must match).
        autopay_account_id: Option<AccountId>,
        /// New optional free-text description.
        description: Option<String>,
    },
    /// Delete a manual recurring bill (personal-cfo-zl1l): drop its `recurring_event`
    /// and `bill_contract` rows by recurring-event id, then refresh the commitments
    /// projection. Only the forward-looking schedule is removed.
    DeleteRecurringBill {
        /// The recurring event id identifying the bill to delete.
        event_id: RecurringEventId,
    },
    /// Archive a manual recurring bill (personal-cfo-4d8.2): set `is_active = 0` +
    /// `archived_at`, dropping it from the commitments projection / forecast while
    /// keeping the bill and its history. Keyed by recurring-event id.
    ArchiveRecurringBill(RecurringEventId),
    /// Restore a previously archived recurring bill (personal-cfo-4d8.2): set
    /// `is_active = 1` + clear `archived_at`, returning it to the forecast.
    RestoreRecurringBill(RecurringEventId),
    /// Open an ingestion source batch (ADR 0008, personal-cfo-3bb). Status starts
    /// at `parsing`; importers stage records under it, then advance its state.
    CreateSourceBatch {
        /// The caller-minted batch id (so the importer knows it up front).
        id: SourceBatchId,
        /// Source type token (`csv`/`ofx`/`manual`/…, CHECK-constrained).
        source_type: String,
        /// Display name (e.g. the filename), if any.
        source_name: Option<String>,
        /// Whole-file content fingerprint for file-level dedupe, if any.
        file_fingerprint: Option<String>,
        /// Parser version, if known at open time.
        parser_version: Option<String>,
    },
    /// Attach a parsed `source_record` to a batch (ADR 0008, personal-cfo-3bb).
    /// Idempotent on `(batch, source_hash)`: re-attaching the same content adds no
    /// new record (shred-after-parse keeps the hash + normalized fields, not bytes).
    AttachSourceRecord {
        /// The caller-minted record id (used only if the content is new).
        id: SourceRecordId,
        /// The batch this record belongs to.
        batch_id: SourceBatchId,
        /// Provider/external id, when the source carries one.
        external_id: Option<String>,
        /// Content fingerprint of the record (the dedupe key).
        source_hash: String,
        /// The extracted, normalized fields as JSON (never the raw bytes).
        normalized_json: String,
        /// Parser confidence in basis points (0..=10000), if scored.
        parse_confidence_bps: Option<i64>,
    },
    /// Advance a source batch's lifecycle status + progress counts (ADR 0008,
    /// personal-cfo-3bb): `parsing → staged → committed | partially_committed |
    /// discarded | failed`, plus `superseded`.
    UpdateBatchState {
        /// The batch to advance.
        batch_id: SourceBatchId,
        /// The new lifecycle status (CHECK-constrained).
        status: String,
        /// Records staged so far.
        staged_count: i64,
        /// Records committed to the ledger so far.
        committed_count: i64,
        /// Records skipped (duplicates, etc.) so far.
        skipped_count: i64,
    },
    /// Commit one staged transaction to the ledger (ADR 0008/0014, personal-cfo-cmx):
    /// promote it to a balanced double-entry transaction + FK-strict import
    /// provenance, marking the staged row committed. A transaction-fingerprint
    /// duplicate is flagged for the Money Inbox instead — never silently dropped.
    CommitStaged {
        /// The staged transaction to commit.
        staged_transaction_id: StagedTransactionId,
        /// Skip the transaction-level dedupe check and commit unconditionally —
        /// the Money Inbox "import anyway" resolution for a flagged row (ADR 0014
        /// §7, personal-cfo-asqy). The pipeline always passes `false`.
        force: bool,
    },
    /// Skip a staged transaction without committing it — the Money Inbox "skip"
    /// resolution (ADR 0014 §7, personal-cfo-asqy). Marks the row `skipped` so the
    /// next inbox rebuild drops it; no ledger write.
    SkipStaged {
        /// The staged transaction to skip.
        staged_transaction_id: StagedTransactionId,
    },
    /// Snooze a Money Inbox item until `until` (ADR 0014 §7, personal-cfo-ci71): a
    /// soft action recorded in `change_journal_entries` and re-applied to the read
    /// model on rebuild. The item hides from the default list until the date passes.
    SnoozeInboxItem {
        /// The inbox item id (`money_inbox_read_model.item_id`).
        item_id: Uuid,
        /// Hide the item until this date (`YYYY-MM-DD`).
        until: NaiveDate,
    },
    /// Dismiss a Money Inbox item with a typed `reason` (ADR 0014 §7,
    /// personal-cfo-ci71): a soft action recorded in `change_journal_entries`; the
    /// item stays hidden across rebuilds.
    DismissInboxItem {
        /// The inbox item id (`money_inbox_read_model.item_id`).
        item_id: Uuid,
        /// Why it was dismissed (a token: `not_relevant` / `already_handled` /
        /// `incorrect` / `other`).
        reason: String,
    },
    /// Dismiss a recurring-bill SUGGESTION (ADR 0046, personal-cfo-4d8.24.6): upsert a
    /// suppression keyed on `(merchant_key, currency)` so detection stops offering it
    /// until the pattern materially changes (amount outside the band / different cadence).
    DismissRecurringSuggestion {
        /// The suggestion's normalized merchant key (detection's grouping key).
        merchant_key: String,
        /// The suggestion's currency (part of the suppression key).
        currency: String,
        /// The dismissed amount (positive magnitude) — the re-surface band anchors on it.
        amount_minor: i64,
        /// The dismissed cadence token — a different inferred cadence re-surfaces it.
        frequency: String,
        /// Optional free-text reason.
        reason: Option<String>,
    },
    /// Create a user category in the taxonomy (ADR 0030, personal-cfo-bac). Always
    /// a user category (`is_system = 0`); the forecast behavior is derived from the
    /// type.
    CreateCategory {
        /// Caller-minted id.
        id: CategoryId,
        /// Parent category, or `None` for a top-level group.
        parent_id: Option<CategoryId>,
        /// Display name.
        name: String,
        /// `income` / `expense` / `transfer` / `adjustment`.
        category_type: String,
        /// Optional display color.
        color: Option<String>,
        /// Optional display icon (an emoji).
        icon: Option<String>,
    },
    /// Rename and/or recolor a category (ADR 0030). A *user* category updates
    /// name + color + icon. A *system* ("Default") category updates its APPEARANCE
    /// (color + icon) only — its name is preserved and any submitted name is ignored,
    /// keeping identity immutable (ADR 0030 amendment, personal-cfo-kogu).
    UpdateCategory {
        /// The category to edit.
        id: CategoryId,
        /// New display name (must be non-empty). Ignored for a system category.
        name: String,
        /// New display color, or `None` to clear it.
        color: Option<String>,
        /// New display icon (an emoji), or `None` to clear it (personal-cfo-4d8.24.10).
        icon: Option<String>,
    },
    /// Re-parent a *user* category, or make it a top-level group (`new_parent_id`
    /// `None`) (ADR 0030). A system category cannot be re-parented (its identity is
    /// fixed); re-parenting cycles are pre-checked for a clean error and blocked by the
    /// `categories_no_parent_cycle` trigger as the hard backstop.
    MoveCategory {
        /// The category to move.
        id: CategoryId,
        /// New parent, or `None` for a top-level group.
        new_parent_id: Option<CategoryId>,
    },
    /// Archive (soft-delete / hide) a category — system or user (ADR 0030). Keeps
    /// historical assignments valid; the category drops from pickers.
    ArchiveCategory(CategoryId),
    /// Un-archive a previously hidden category (ADR 0030).
    ReinstateCategory(CategoryId),
    /// Set or clear a transaction's category (ADR 0030, personal-cfo-bac). A manual
    /// assignment: `source = user`, `confidence = 100%`. `category_id` `None` clears
    /// it (uncategorize). Latest assignment wins (upsert on `transaction_id`).
    RecategorizeTransaction {
        /// The transaction to (re)categorize.
        transaction_id: TransactionId,
        /// The category to assign, or `None` to clear the assignment.
        category_id: Option<CategoryId>,
    },
    /// Delete a transaction by voiding it (personal-cfo-4d8.11, ADR 0007 §9): post a
    /// reversing entry and hide both the original and the reversal from every view, so
    /// balances + forecast update as if it never happened while the ledger stays
    /// append-only and auditable.
    VoidTransaction {
        /// The transaction to void.
        transaction_id: TransactionId,
    },
    /// Mark a transaction reviewed or unreviewed (personal-cfo-4d8.7, ADR 0032 §2):
    /// records the user's explicit override in `transaction_reviews`. The default (no
    /// override) is derived from import provenance — imported = unreviewed, manual =
    /// reviewed.
    MarkReviewed {
        /// The transaction to (un)review.
        transaction_id: TransactionId,
        /// `true` = reviewed, `false` = unreviewed.
        reviewed: bool,
    },
    /// Promote a scenario's active assumption events into base (ADR 0055 §1).
    ///
    /// On the command bus, exceptionally: assumption events are normally forecast inputs
    /// written outside it, but apply is the one that changes what the household's real
    /// forecast says, and this command's `command_id` is stored as the reversal handle
    /// (ADR 0055 §3).
    ApplyScenario {
        /// The scenario whose active events become base events.
        scenario_id: Uuid,
    },
    /// Undo an [`WriteCommand::ApplyScenario`] (ADR 0055 §5).
    RevertScenarioApply {
        /// The applied scenario to roll back.
        scenario_id: Uuid,
    },
    /// Create a user-defined tag (ADR 0033, personal-cfo-2ryf).
    CreateTag {
        /// Caller-minted tag id.
        id: TagId,
        /// Tag name (unique among non-archived tags).
        name: String,
        /// Optional display color.
        color: Option<String>,
    },
    /// Replace a transaction's tag set (ADR 0033, personal-cfo-hmt).
    SetTags {
        /// The transaction to tag.
        transaction_id: TransactionId,
        /// The full desired tag set.
        tag_ids: Vec<TagId>,
    },
    /// Set or clear a transaction's free-text note (ADR 0033 §3).
    SetNote {
        /// The transaction to annotate.
        transaction_id: TransactionId,
        /// The note, or `None` to clear it.
        note: Option<String>,
    },
    /// Replace a transaction's split set (ADR 0034, personal-cfo-e7i).
    SetSplits {
        /// The transaction to split (empty `lines` un-splits it).
        transaction_id: TransactionId,
        /// The full desired set of split lines (sum must equal the txn amount).
        lines: Vec<SplitLineInput>,
    },
}

impl WriteCommand {
    fn kind(&self) -> &'static str {
        match self {
            WriteCommand::CreateAccount { .. } => "create_account",
            WriteCommand::UpdateAccount { .. } => "update_account",
            WriteCommand::ArchiveAccount(_) => "archive_account",
            WriteCommand::ReinstateAccount(_) => "reinstate_account",
            WriteCommand::SetAccountSubtype { .. } => "set_account_subtype",
            WriteCommand::SetAccountNote { .. } => "set_account_note",
            WriteCommand::SetAccountLink { .. } => "set_account_link",
            WriteCommand::SetDebtTerms { .. } => "set_debt_terms",
            WriteCommand::SetCardStatementBalance { .. } => "set_card_statement_balance",
            WriteCommand::RecordTransaction { .. } => "record_transaction",
            WriteCommand::ConfirmObligationEarly { .. } => "confirm_obligation_early",
            WriteCommand::UnconfirmObligation { .. } => "unconfirm_obligation",
            WriteCommand::ConvertUnexplainedToTransaction { .. } => {
                "convert_unexplained_to_transaction"
            }
            WriteCommand::Transfer { .. } => "transfer",
            WriteCommand::CreateRecurringTransfer { .. } => "create_recurring_transfer",
            WriteCommand::DeleteRecurringTransfer(_) => "delete_recurring_transfer",
            WriteCommand::CreateIncomeSource { .. } => "create_income_source",
            WriteCommand::UpdateIncomeSource { .. } => "update_income_source",
            WriteCommand::DeleteIncomeSource(_) => "delete_income_source",
            WriteCommand::ArchiveIncomeSource(_) => "archive_income_source",
            WriteCommand::RestoreIncomeSource(_) => "restore_income_source",
            WriteCommand::CreateRecurringBill { .. } => "create_recurring_bill",
            WriteCommand::SetBillAutopay { .. } => "set_bill_autopay",
            WriteCommand::UpdateRecurringBill { .. } => "update_recurring_bill",
            WriteCommand::DeleteRecurringBill { .. } => "delete_recurring_bill",
            WriteCommand::ArchiveRecurringBill(_) => "archive_recurring_bill",
            WriteCommand::RestoreRecurringBill(_) => "restore_recurring_bill",
            WriteCommand::CreateSourceBatch { .. } => "create_source_batch",
            WriteCommand::AttachSourceRecord { .. } => "attach_source_record",
            WriteCommand::UpdateBatchState { .. } => "update_batch_state",
            WriteCommand::CommitStaged { .. } => "commit_staged",
            WriteCommand::SkipStaged { .. } => "skip_staged",
            WriteCommand::SnoozeInboxItem { .. } => "snooze_inbox_item",
            WriteCommand::DismissInboxItem { .. } => "dismiss_inbox_item",
            WriteCommand::DismissRecurringSuggestion { .. } => "dismiss_recurring_suggestion",
            WriteCommand::CreateCategory { .. } => "create_category",
            WriteCommand::UpdateCategory { .. } => "update_category",
            WriteCommand::MoveCategory { .. } => "move_category",
            WriteCommand::ArchiveCategory(_) => "archive_category",
            WriteCommand::ReinstateCategory(_) => "reinstate_category",
            WriteCommand::RecategorizeTransaction { .. } => "recategorize_transaction",
            WriteCommand::VoidTransaction { .. } => "void_transaction",
            WriteCommand::MarkReviewed { .. } => "mark_reviewed",
            WriteCommand::ApplyScenario { .. } => "apply_scenario",
            WriteCommand::RevertScenarioApply { .. } => "revert_scenario_apply",
            WriteCommand::CreateTag { .. } => "create_tag",
            WriteCommand::SetTags { .. } => "set_tags",
            WriteCommand::SetNote { .. } => "set_note",
            WriteCommand::SetSplits { .. } => "set_splits",
        }
    }
}

/// The result of a dispatched command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The command was applied; carries its op-log sequence number.
    Applied {
        /// Monotonic op-log sequence number.
        op_seq: i64,
    },
    /// A command with this `idempotency_key` was already applied; no new rows
    /// were written. Carries the original op-log sequence number.
    Replayed {
        /// Op-log sequence number of the original application.
        op_seq: i64,
    },
}

/// A read-model row for a user account, rebuildable from canonical tables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountView {
    /// The account id.
    pub id: AccountId,
    /// Display name.
    pub name: String,
    /// Cashflow-role storage token.
    pub cashflow_role: String,
    /// Optional finer-subtype storage token (ADR 0028); `None` when unspecified.
    pub subtype: Option<String>,
    /// Whether the account is active (not archived).
    pub active: bool,
    /// Balance, summed from the account's ledger postings (canonical).
    pub balance: Money,
    /// Free-text note (ADR 0044); `None` when unset.
    pub notes: Option<String>,
    /// The real-asset -> financing-liability link stored on THIS row (ADR 0044 §5).
    /// Only ever set on a real-asset account (it points at its mortgage/auto-loan);
    /// `None` otherwise. Display-only — net-worth math is unaffected.
    pub linked_account_id: Option<AccountId>,
    /// The display name of this account's link partner (ADR 0044 §5), resolved for
    /// BOTH sides: for a real asset it's the liability it points at; for a liability
    /// it's the asset that points at it (reverse lookup). `None` when unlinked.
    pub linked_account_name: Option<String>,
}

/// Type-based cash-tier rollups (ADR 0028): liquid balances bucketed into
/// spendable vs reserve, plus their net. Single-currency like the forecast
/// (`personal-cfo-4n3x`). Derived on read from account subtype + assertion-anchored
/// balances; nothing is persisted. Consumed by the per-account/group forecast
/// (`personal-cfo-l8oh`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CashTiers {
    /// Spendable now: checking + cash + any unclassified liquid account.
    pub spendable: Money,
    /// Set aside: savings + money market.
    pub reserve: Money,
    /// Net cash = spendable + reserve = every liquid account.
    pub net: Money,
}

/// A recent transaction as seen from one user account (personal-cfo-idsd). Read
/// directly from the canonical ledger (always fresh), one row per user-account
/// posting; the signed amount is from that account's perspective.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionRow {
    /// The ledger transaction id.
    pub transaction_id: TransactionId,
    /// The user account the money moved against.
    pub account_id: AccountId,
    /// The account's display name (denormalized for the list).
    pub account_name: String,
    /// The far side of a two-account movement (personal-cfo-4d8.27.8.1): a transfer's
    /// other account. `None` for an ordinary income/expense, which touches one user
    /// account and a system counter-account. Read with `amount`'s sign to render a
    /// direction — negative means money left `account_name` FOR this one.
    pub counter_account_id: Option<AccountId>,
    pub counter_account_name: Option<String>,
    /// When the transaction occurred — the POSTED date (primary, ADR 0045).
    pub occurred_at: DateTime<Utc>,
    /// The secondary transaction / authorization date an import carried, when distinct
    /// from the posted date (ADR 0045, personal-cfo-4d8.24.1); `None` otherwise.
    pub transaction_date: Option<NaiveDate>,
    /// The account's balance immediately after this transaction, over the **full ledger**
    /// (personal-cfo-ttuy).
    ///
    /// `None` when the caller did not ask for it — see [`TransactionPageQuery::with_balances`].
    /// Deliberately not derivable from the page: filtering the list must never change what
    /// an account held at a moment, so this is a cumulative sum at the transaction's
    /// position in the whole ledger, not a running total over the returned rows.
    pub balance_after_minor: Option<i64>,
    /// Signed amount: positive is money in, negative is money out.
    pub amount: Money,
    /// Free-text detail (personal-cfo-byxe): an import's original description, or a
    /// user memo. `None` for a bare transaction.
    pub memo: Option<String>,
    /// The merchant / payee (normalized), when known. `None` for a bare transaction.
    pub counterparty: Option<String>,
    /// The assigned category (ADR 0030, personal-cfo-bac), or `None` if
    /// uncategorized. The frontend resolves the display name from the taxonomy.
    pub category_id: Option<CategoryId>,
    /// Whether the user has reviewed this transaction (ADR 0032, personal-cfo-4d8.7).
    /// Defaults by source: imported = unreviewed, manual = reviewed.
    pub reviewed: bool,
    /// The user's free-text note (ADR 0033, personal-cfo-hmt), or `None`.
    pub note: Option<String>,
    /// The assigned tag ids (ADR 0033, personal-cfo-2ryf); empty when untagged.
    pub tag_ids: Vec<TagId>,
    /// Number of split lines (ADR 0034); `0` when the transaction is not split. Lets the
    /// list show an expand affordance without fetching the lines.
    pub split_count: i64,
    /// How the category was assigned (ADR 0030 merchant-memory addendum, personal-cfo-5n4.1):
    /// one of `user` / `rule` / `model` / `import_alias`, or `None` when uncategorized. Drives
    /// the row's provenance badge ("you" vs. auto-categorized) so learned tags are visible.
    pub category_source: Option<String>,
    /// The categorizer's confidence in basis points (0..=10000), or `None` when uncategorized.
    /// For `rule` rows this is the merchant-memory agreement ratio.
    pub category_confidence_bps: Option<i64>,
}

/// A recurring net-pay income source (personal-cfo-le79), with its next pay date
/// computed from the schedule at read time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomeSourceView {
    /// The income source id.
    pub id: IncomeSourceId,
    /// Display name (e.g. the employer).
    pub name: String,
    /// Net (take-home) pay per occurrence.
    pub net_amount: Money,
    /// Pay cadence.
    pub frequency: Frequency,
    /// The schedule anchor date.
    pub anchor: NaiveDate,
    /// Optional deposit account id.
    pub deposit_account_id: Option<AccountId>,
    /// The deposit account's name, if one is linked.
    pub deposit_account_name: Option<String>,
    /// The next pay date on or after today, if representable.
    pub next_pay_date: Option<NaiveDate>,
    /// Whether the source is active (archived sources are excluded from the
    /// forecast). personal-cfo-tch0.
    pub active: bool,
    /// When the source was created (RFC 3339).
    pub created_at: String,
    /// When the source was archived (RFC 3339), if it is archived.
    pub archived_at: Option<String>,
}

/// The household liquid-cash comfort band (ADR 0018 addendum 915.1, personal-cfo-3v6d): the
/// target range projected cash should sit within, in the reporting currency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComfortBand {
    /// The currency both edges are in.
    pub currency: Currency,
    /// The lower edge — the shipped minimum-cash floor (default `0`).
    pub lower: Money,
    /// The upper edge — excess above it the user may choose to deploy. `None` until set.
    pub upper: Option<Money>,
}

/// A contributing category in a band drift (personal-cfo-5ie.8), enriched with its display name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriftFactorView {
    /// The category id (UUID string).
    pub category_id: String,
    /// The category's display name.
    pub category_name: String,
    /// The category's recent monthly spend.
    pub recent_monthly: Money,
    /// How much that is up versus the preceding window (positive).
    pub delta: Money,
}

/// The descriptive comfort-band drift signal for the UI (personal-cfo-5ie.8, ADR 0018 §915.1): why
/// the projection is set to cross below the band, with the rising-spend categories that attribute
/// it. Facts only — the descriptive copy that renders it lives in the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BandDriftView {
    /// The first far-horizon day the projection closes below the lower edge.
    pub crossing_date: NaiveDate,
    /// How far below the edge the projection reaches at its worst.
    pub magnitude: Money,
    /// The rising-spend categories, largest rise first.
    pub factors: Vec<DriftFactorView>,
}

/// A manual recurring bill (personal-cfo-esmy): a `recurring_event` joined to its
/// `bill_contract`, with the next due date computed from the schedule at read
/// time (the same way income computes its next pay date).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecurringBillView {
    /// The recurring event id.
    pub id: RecurringEventId,
    /// Display name (e.g. the merchant).
    pub name: String,
    /// Bill-contract type token (e.g. `subscription`).
    pub bill_type: String,
    /// Expected outflow per occurrence.
    pub amount: Money,
    /// Pay cadence.
    pub frequency: Frequency,
    /// The schedule anchor (a known due date).
    pub anchor: NaiveDate,
    /// Optional autopay account id.
    pub autopay_account_id: Option<AccountId>,
    /// The autopay account's name, if one is linked.
    pub autopay_account_name: Option<String>,
    /// Whether the bill is marked autopay (ADR 0041, personal-cfo-mc7f). Legacy bills read `false`.
    pub autopay_enabled: bool,
    /// The next due date on or after today, if representable.
    pub next_due_date: Option<NaiveDate>,
    /// Optional free-text description.
    pub description: Option<String>,
    /// Whether the bill is active (archived bills are excluded from the forecast).
    pub active: bool,
    /// When the bill was created (RFC 3339).
    pub created_at: String,
    /// When the bill was archived (RFC 3339), if it is archived.
    pub archived_at: Option<String>,
    /// The bill's category, set when promoting (personal-cfo-4d8.24.5); `None` if
    /// uncategorized. The frontend resolves the display name from the taxonomy.
    pub category_id: Option<CategoryId>,
    /// The bill's tags (personal-cfo-4d8.24.5.1); empty when untagged.
    pub tag_ids: Vec<TagId>,
}

/// A recurring account-to-account transfer (ADR 0026 §14, personal-cfo-npoe), with
/// its next occurrence computed from the schedule at read time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecurringTransferView {
    /// The recurring transfer id.
    pub id: RecurringTransferId,
    /// The account money moves out of.
    pub source_account_id: AccountId,
    /// The source account's display name.
    pub source_account_name: String,
    /// The account money moves into.
    pub dest_account_id: AccountId,
    /// The destination account's display name.
    pub dest_account_name: String,
    /// The amount moved each occurrence.
    pub amount: Money,
    /// Transfer cadence.
    pub frequency: Frequency,
    /// The schedule anchor (a known occurrence date).
    pub anchor: NaiveDate,
    /// The next occurrence on or after today, if representable.
    pub next_date: Option<NaiveDate>,
    /// When the recurring transfer was created (RFC 3339).
    pub created_at: String,
}

/// Vault-level identity and configuration (plan §9.2, personal-cfo-2lm).
///
/// The KDF fields hold the Argon2id parameters used to derive the vault key.
/// In a freshly created vault these are placeholder defaults; the vault module
/// (`personal-cfo-vhv`) writes the real values at vault creation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultMetadata {
    /// Stable identity of this vault (UUIDv7).
    pub vault_id: Uuid,
    /// The schema version the vault was last written at.
    pub schema_version: i64,
    /// The on-disk crypto envelope version (ADR 0002).
    pub envelope_version: i64,
    /// KDF algorithm token (e.g. `"argon2id"`).
    pub kdf_algorithm: String,
    /// Argon2id memory cost in KiB.
    pub kdf_memory_kib: i64,
    /// Argon2id time cost (iterations).
    pub kdf_time_cost: i64,
    /// Argon2id parallelism (lanes).
    pub kdf_parallelism: i64,
    /// IANA household timezone (consumed by `personal-cfo-rr0`).
    pub household_timezone: String,
    /// Optional pointer to the encrypted attachment manifest.
    pub manifest_pointer: Option<String>,
}

/// What the startup self-test observed.
#[derive(Debug, Clone)]
pub struct SelfTestReport {
    /// Effective journal mode (must be `wal`).
    pub journal_mode: String,
    /// Effective busy timeout in milliseconds.
    pub busy_timeout_ms: i64,
    /// Effective WAL size limit in bytes.
    pub journal_size_limit: i64,
}

/// Errors from the db-worker.
#[derive(Debug, Error)]
pub enum DbError {
    /// A required metadata field was empty.
    #[error("missing required command metadata: {0}")]
    MissingMetadata(&'static str),
    /// The worker is not in a state that accepts writes.
    #[error("worker unavailable for writes: {0:?}")]
    WorkerUnavailable(WorkerState),
    /// A panic occurred mid-write; the transaction rolled back and the worker is
    /// marked for recovery.
    #[error("writer panicked; transaction rolled back and worker marked for recovery")]
    WriterPanicked,
    /// The startup self-test found a misconfigured connection.
    #[error("startup self-test failed: {0}")]
    SelfTestFailed(String),
    /// A command was structurally invalid (e.g. unbalanced opening posting,
    /// unknown currency, missing system account).
    #[error("invalid command: {0}")]
    InvalidCommand(String),
    /// An underlying SQLite/SQLCipher error.
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    /// Blob filesystem I/O failed (attachment store, ADR 0023). The message is
    /// deliberately generic — it never embeds plaintext or key material (§6.6).
    #[error("attachment blob I/O failed")]
    Io(#[from] std::io::Error),
    /// Attachment cryptography failed (wrap/unwrap/encrypt/decrypt). Generic
    /// message — never embeds key bytes (§6.6).
    #[error("attachment cryptography failed")]
    Crypto(#[from] vault_crypto::VaultCryptoError),
    /// An operation needs the raw DEK, but the worker was opened in passphrase
    /// mode. Attachments require a raw-key vault (ADR 0002 / ADR 0023).
    #[error("operation requires an unlocked raw vault key")]
    KeyUnavailable,
    /// The referenced attachment id does not exist.
    #[error("attachment not found")]
    AttachmentNotFound,
}

impl From<MoneyError> for DbError {
    fn from(error: MoneyError) -> Self {
        DbError::InvalidCommand(error.to_string())
    }
}

#[derive(Clone, Copy)]
enum Fault {
    None,
    #[cfg_attr(not(test), allow(dead_code))]
    PanicAt(FaultPoint),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FaultPoint {
    AfterMutation,
    AfterOpLog,
}

impl Fault {
    fn maybe_panic(self, point: FaultPoint) {
        if let Fault::PanicAt(p) = self {
            if p == point {
                panic!("injected fault at {point:?}");
            }
        }
    }
}

struct Inner {
    conn: Connection,
    state: WorkerState,
    /// Monotonic hybrid-logical-clock counter (sync-readiness, §9.1.2).
    last_hlc: i64,
}

impl Inner {
    /// Next monotonic HLC value: never less than wall-clock millis, always
    /// strictly increasing within the process.
    fn next_hlc(&mut self) -> i64 {
        let wall = Utc::now().timestamp_millis();
        self.last_hlc = (self.last_hlc + 1).max(wall);
        self.last_hlc
    }
}

/// How the SQLCipher connection is keyed. `Passphrase` is SQLCipher's own
/// KDF path (used by tests and any passphrase-mode vault); `Raw` feeds a
/// pre-derived 256-bit key (the DEK from `personal-cfo-vhv`) directly. Both
/// forms are held in zeroizing storage so the key is scrubbed when the worker
/// is dropped (lock zeroizes the DEK, ADR 0002).
enum KeyMaterial {
    Passphrase(Zeroizing<String>),
    Raw(Dek),
}

/// The database worker. Cheap to share across threads behind an `Arc`.
pub struct DbWorker {
    inner: Mutex<Inner>,
    path: PathBuf,
    key: KeyMaterial,
    /// Stable per-vault node identity (UUIDv7) stamped on every op-log row
    /// (§9.1.2; stored as a BLOB per ADR 0011).
    node_id: Uuid,
    /// Schema version stamped on every op-log row (§9.1.2).
    schema_version: i64,
}

impl DbWorker {
    /// Open (or create) the encrypted vault at `path`, configure it, bootstrap
    /// the schema, and run the startup self-test.
    ///
    /// # Errors
    /// Returns [`DbError`] if the connection cannot be opened/keyed/configured,
    /// the schema cannot be created, or the self-test fails.
    pub fn open(path: impl AsRef<Path>, key: &str) -> Result<Self, DbError> {
        let path = path.as_ref().to_path_buf();
        let conn = open_keyed(&path, key)?;
        Self::bootstrap(
            path,
            conn,
            KeyMaterial::Passphrase(Zeroizing::new(key.to_owned())),
        )
    }

    /// Open (or create) the encrypted vault at `path`, keyed directly from the
    /// 256-bit `dek` — the raw SQLCipher key that `vault-crypto` unwraps from
    /// the envelope (`personal-cfo-vhv`). This is the production vault path; the
    /// passphrase [`open`](Self::open) is for tests and passphrase-mode vaults.
    ///
    /// The `dek` is moved into the worker and zeroized when the worker is
    /// dropped, so locking the vault (dropping the worker) scrubs the key
    /// (ADR 0002).
    ///
    /// # Errors
    /// Returns [`DbError`] if the connection cannot be opened/keyed/configured,
    /// the schema cannot be created, or the self-test fails.
    pub fn open_with_raw_key(path: impl AsRef<Path>, dek: Dek) -> Result<Self, DbError> {
        let path = path.as_ref().to_path_buf();
        let conn = open_keyed_raw(&path, &dek)?;
        Self::bootstrap(path, conn, KeyMaterial::Raw(dek))
    }

    /// Shared post-keying bootstrap: migrate, seed singletons, self-test. Run
    /// before the worker is usable so no domain query ever touches a
    /// half-migrated vault (personal-cfo-wkn).
    fn bootstrap(path: PathBuf, mut conn: Connection, key: KeyMaterial) -> Result<Self, DbError> {
        migrations::run_migrations(&mut conn)?;
        let node_id = ensure_node_id(&conn)?;
        ensure_vault_metadata(&conn, migrations::CURRENT_VERSION)?;
        ensure_system_accounts(&conn)?;
        ensure_default_categories(&conn)?;
        merchant_identity::ensure_seed_merchants(&conn)?;
        let last_hlc = current_max_hlc(&conn)?;
        let worker = Self {
            inner: Mutex::new(Inner {
                conn,
                state: WorkerState::Healthy,
                last_hlc,
            }),
            path,
            key,
            node_id,
            schema_version: migrations::CURRENT_VERSION,
        };
        worker.self_test()?;
        Ok(worker)
    }

    /// The current writer health.
    #[must_use]
    pub fn state(&self) -> WorkerState {
        self.lock().state
    }

    /// Open a fresh read-only-style connection to the same vault. Reads through
    /// these never block the single writer (WAL).
    ///
    /// # Errors
    /// Returns [`DbError`] if the connection cannot be opened or keyed.
    pub fn read_connection(&self) -> Result<Connection, DbError> {
        match &self.key {
            KeyMaterial::Passphrase(passphrase) => open_keyed(&self.path, passphrase.as_str()),
            KeyMaterial::Raw(dek) => open_keyed_raw(&self.path, dek),
        }
    }

    /// A consistent snapshot of the encrypted `vault.db` bytes, for backup
    /// (personal-cfo-ef3, ADR 0024). Takes the writer lock so no write
    /// interleaves, folds the WAL into the main database
    /// (`wal_checkpoint(TRUNCATE)` — the checkpoint copies every committed frame
    /// into the main file regardless of whether the truncate step can run), then
    /// reads the file. The bytes are SQLCipher ciphertext, safe to copy into a
    /// backup as-is.
    ///
    /// # Errors
    /// [`DbError::WorkerUnavailable`] if the worker is not healthy, or
    /// [`DbError`] if the checkpoint or file read fails.
    pub fn snapshot_bytes(&self) -> Result<Vec<u8>, DbError> {
        let guard = self.lock();
        if guard.state != WorkerState::Healthy {
            return Err(DbError::WorkerUnavailable(guard.state));
        }
        guard
            .conn
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        Ok(std::fs::read(&self.path)?)
    }

    /// The vault database path (`vault.db`). Used by the backup module to locate
    /// the envelope sidecar and blob store (personal-cfo-ef3).
    #[must_use]
    pub fn db_path(&self) -> &Path {
        &self.path
    }

    /// The attachment blob directory (`<vault>/blobs/`, ADR 0023).
    #[must_use]
    pub fn blobs_dir(&self) -> PathBuf {
        attachments::blobs_dir(&self.path)
    }

    /// Total number of accounts. Typed read view so callers (e.g. the Finance
    /// Kernel) never need to touch a `rusqlite::Connection`.
    ///
    /// # Errors
    /// Returns [`DbError`] if the read connection cannot be opened or the query
    /// fails.
    pub fn account_count(&self) -> Result<u64, DbError> {
        self.read_connection()?.count_accounts()
    }

    /// Whether an account with `id` exists.
    ///
    /// # Errors
    /// Returns [`DbError`] if the read connection cannot be opened or the query
    /// fails.
    pub fn account_exists(&self, id: AccountId) -> Result<bool, DbError> {
        self.read_connection()?.account_exists(id)
    }

    /// The balance of an account (ADR 0027): the latest manual balance assertion
    /// plus postings after it, or the plain posting sum when none. `None` if the
    /// account does not exist.
    ///
    /// # Errors
    /// Returns [`DbError`] if the read fails.
    pub fn account_balance(&self, id: AccountId) -> Result<Option<Money>, DbError> {
        read_account_balance(&self.read_connection()?, id)
    }

    /// Record a manual balance assertion (ADR 0027, personal-cfo-ueg6): "account =
    /// `amount` as of `as_of`", stored as a `manual` balance observation. It is
    /// **not** a ledger posting — the assertion anchors the balance, and any
    /// transactions stay optional. The currency must match the account's.
    ///
    /// # Errors
    /// [`DbError::InvalidCommand`] if the account does not exist or the currency
    /// mismatches; [`DbError::Sqlite`] on a write failure.
    pub fn record_balance_assertion(
        &self,
        id: Uuid,
        account_id: AccountId,
        amount: Money,
        as_of: NaiveDate,
    ) -> Result<(), DbError> {
        let guard = self.lock();
        let account_currency: Option<String> = guard
            .conn
            .query_row(
                "SELECT currency FROM accounts WHERE id = ?1",
                [account_id.as_uuid()],
                |r| r.get(0),
            )
            .optional()?;
        let Some(account_currency) = account_currency else {
            return Err(DbError::InvalidCommand("no such account".to_owned()));
        };
        if amount.currency() != currency_from_code(&account_currency)? {
            return Err(DbError::InvalidCommand(
                "assertion currency must match the account currency".to_owned(),
            ));
        }
        let now = Utc::now().to_rfc3339();
        guard.conn.execute(
            "INSERT INTO balance_observations
                (id, account_id, observed_at, balance_amount_minor, balance_currency,
                 source, source_record_id, reconciliation_session_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'manual', NULL, NULL, ?6)",
            params![
                id,
                account_id.as_uuid(),
                as_of.to_string(),
                amount.minor_units(),
                amount.currency().code(),
                now,
            ],
        )?;
        Ok(())
    }

    /// The derived auto-reconciling adjustment ("plug", ADR 0027) for an account:
    /// the still-unexplained amount of its latest assertion, or `None` if it has no
    /// assertion. Recomputed on read; shrinks as real postings explain the gap.
    ///
    /// # Errors
    /// Returns [`DbError`] if the read fails.
    pub fn account_unexplained(&self, account_id: AccountId) -> Result<Option<Money>, DbError> {
        let conn = self.read_connection()?;
        let row: Option<(Uuid, String)> = conn
            .query_row(
                "SELECT ledger_account_id, currency FROM accounts WHERE id = ?1",
                [account_id.as_uuid()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((ledger_account_id, currency_code)) = row else {
            return Ok(None);
        };
        let currency = currency_from_code(&currency_code)?;
        Ok(
            unexplained_adjustment(&conn, account_id.as_uuid(), ledger_account_id)?
                .map(|minor| Money::new(minor, currency)),
        )
    }

    /// The read-model row for an account, rebuilt from canonical tables. `None`
    /// if the account does not exist.
    ///
    /// # Errors
    /// Returns [`DbError`] if the read fails.
    pub fn account_view(&self, id: AccountId) -> Result<Option<AccountView>, DbError> {
        read_account_view(&self.read_connection()?, id)
    }

    /// Every account's read-model row, ordered by name. Backs the accounts list
    /// UI (personal-cfo-0eft).
    ///
    /// # Errors
    /// Returns [`DbError`] if the read fails.
    pub fn account_views(&self) -> Result<Vec<AccountView>, DbError> {
        read_account_views(&self.read_connection()?)
    }

    /// The type-based cash-tier rollups (ADR 0028, personal-cfo-9dgg): spendable /
    /// reserve / net cash derived from liquid accounts' subtypes and their
    /// assertion-anchored balances. No user setup — it works off account type out
    /// of the box.
    ///
    /// # Errors
    /// Returns [`DbError`] if the read fails or liquid accounts span currencies.
    pub fn cash_tiers(&self) -> Result<CashTiers, DbError> {
        cash_tier_rollups(&self.read_connection()?)
    }

    /// The household cash-availability snapshot (ADR 0029, personal-cfo-fqbm):
    /// ledger / available / pending / committed / headroom per liquid account, plus
    /// the net rollup and the minimum-cash-floor status. The wall clock is read here
    /// (the impure boundary) and passed to the deterministic computation.
    ///
    /// # Errors
    /// Returns [`DbError`] on a read failure, mixed-currency liquid accounts, or a
    /// forecast arithmetic failure.
    pub fn cash_availability(&self) -> Result<CashAvailability, DbError> {
        let floor_minor = self
            .get_setting(MINIMUM_CASH_FLOOR_KEY)?
            .and_then(|raw| raw.trim().parse::<i64>().ok())
            .unwrap_or(0);
        forecast::compute_cash_availability(&self.read_connection()?, Utc::now(), floor_minor)
    }

    /// The household liquid-cash comfort band (ADR 0018 addendum 915.1, personal-cfo-3v6d): the
    /// target range projected cash should sit within. The LOWER edge is the shipped
    /// minimum-cash-floor (wrapped, not replaced); the UPPER edge is a separate setting, absent
    /// until the user sets one. Both are interpreted in the reporting currency. Pure settings read.
    pub fn comfort_band(&self) -> Result<ComfortBand, DbError> {
        let conn = self.read_connection()?;
        let currency = forecast::reporting_currency(&conn)?.unwrap_or(Currency::Usd);
        let read = |key: &str| -> Result<Option<i64>, DbError> {
            Ok(self
                .get_setting(key)?
                .and_then(|raw| raw.trim().parse::<i64>().ok()))
        };
        Ok(ComfortBand {
            currency,
            lower: Money::new(read(MINIMUM_CASH_FLOOR_KEY)?.unwrap_or(0), currency),
            upper: read(COMFORT_BAND_UPPER_KEY)?.map(|m| Money::new(m, currency)),
        })
    }

    /// The current comfort-band drift signal (personal-cfo-5ie.8, ADR 0018 §915.1): why the Future
    /// Cash projection is set to cross **below** the band, if it is — the rising-spend categories
    /// that attribute it (within-run). `None` when there's no far-horizon below-band crossing
    /// attributable to a spending rise (the plain crossing signal covers those). The clock is read
    /// here and passed down, keeping the underlying attribution deterministic.
    pub fn band_drift(
        &self,
        as_of: DateTime<Utc>,
        horizon_days: u32,
    ) -> Result<Option<BandDriftView>, DbError> {
        let conn = self.read_connection()?;
        let band = self.comfort_band()?;
        let forecast = forecast::compute(&conn, as_of, horizon_days, &[])?;
        let today = as_of.date_naive();
        // 180 days back covers the engine's recent + preceding attribution windows.
        let window_start = today - chrono::Duration::days(180);
        let spend = forecast::variable_spend_points(&conn, window_start, today)?;
        let days: Vec<(NaiveDate, i64)> = forecast
            .days
            .iter()
            .map(|d| (d.date, d.closing.p50.minor_units()))
            .collect();

        let Some(signal) =
            band_drift::compute_band_drift_signal(&days, band.lower.minor_units(), &spend, today)
        else {
            return Ok(None);
        };

        let currency = band.currency;
        let factors = signal
            .contributing_factors
            .iter()
            .map(|f| {
                Ok(DriftFactorView {
                    category_name: category_display_name(&conn, &f.category_id)?,
                    category_id: f.category_id.clone(),
                    recent_monthly: Money::new(f.recent_monthly_minor, currency),
                    delta: Money::new(f.delta_minor, currency),
                })
            })
            .collect::<Result<Vec<_>, DbError>>()?;
        Ok(Some(BandDriftView {
            crossing_date: signal.crossing_date,
            magnitude: Money::new(signal.magnitude_minor, currency),
            factors,
        }))
    }

    /// The R1 Forecast Readiness score (ADR 0026 §13, personal-cfo-6vj9): a 0–100
    /// data-maturity indicator (coverage + balance freshness + explained ratio) with
    /// a per-factor breakdown. The wall clock is read here and passed to the
    /// deterministic computation.
    ///
    /// # Errors
    /// Returns [`DbError`] on a read failure or a malformed stored schedule.
    pub fn forecast_readiness(&self) -> Result<ForecastReadiness, DbError> {
        forecast::compute_forecast_readiness(&self.read_connection()?, Utc::now())
    }

    /// Forecast capabilities that have self-activated but whose one-time unlock notice the
    /// user has not yet acknowledged (ADR 0026 §10, personal-cfo-egon).
    ///
    /// # Errors
    /// Returns [`DbError`] on a read failure or a malformed stored schedule.
    pub fn pending_capability_unlocks(&self) -> Result<Vec<CapabilityUnlock>, DbError> {
        forecast::pending_capability_unlocks(&self.read_connection()?, Utc::now())
    }

    /// Record the user's acknowledgement of a capability-unlock notice, so it fires exactly
    /// once (an append-only `audit_events` row, mirroring the no-reset-warning ack).
    /// Rejects unknown capability keys so only real capabilities can be acknowledged.
    ///
    /// # Errors
    /// [`DbError::InvalidCommand`] for an unknown `key`; [`DbError::Sqlite`] on a write
    /// failure.
    pub fn acknowledge_capability(&self, meta: &CommandMeta, key: &str) -> Result<(), DbError> {
        if !forecast::is_known_capability(key) {
            return Err(DbError::InvalidCommand(format!(
                "unknown forecast capability: {key}"
            )));
        }
        self.record_audit_event(meta, &forecast::capability_ack_event_type(key))
    }

    /// Apply merchant-memory auto-categorization (ADR 0030 addendum, personal-cfo-7yh0):
    /// learn merchant→category from the user's manual categorizations and fill
    /// **uncategorized** transactions of the same normalized merchant (`source = 'rule'`,
    /// never overwriting a user assignment). Returns the number newly categorized.
    ///
    /// # Errors
    /// [`DbError`] on a read/write failure.
    pub fn apply_merchant_memory(&self) -> Result<u32, DbError> {
        let guard = self.lock();
        merchant_memory::apply(&guard.conn)
    }

    /// Mint canonical identities for unseeded multi-location chains discovered in the user's
    /// own transactions (ADR 0030 addendum, personal-cfo-5n4.4), returning the count minted.
    /// `apply_merchant_memory` runs this first; exposed separately for the import path + tests.
    ///
    /// # Errors
    /// Returns [`DbError`] if the read/write fails.
    pub fn apply_merchant_grouping(&self) -> Result<u32, DbError> {
        let guard = self.lock();
        merchant_grouping::mint_anchors(&guard.conn)
    }

    /// Create a canonical merchant identity (ADR 0030 addendum, personal-cfo-zrpg),
    /// returning its id. `source` is `seed` | `user` | `auto`. The write seam the
    /// merchant-entity consumers (seeding, the ambiguous-merchant beads) build on.
    ///
    /// # Errors
    /// Returns [`DbError`] if the insert fails (e.g. an out-of-domain `source`).
    pub fn create_merchant_identity(
        &self,
        display_name: &str,
        default_category_id: Option<CategoryId>,
        is_ambiguous: bool,
        source: &str,
    ) -> Result<Uuid, DbError> {
        let guard = self.lock();
        merchant_identity::create_identity(
            &guard.conn,
            display_name,
            default_category_id.map(CategoryId::as_uuid),
            is_ambiguous,
            source,
        )
    }

    /// Point a normalized merchant key (`categorization::normalize_merchant` output) at an
    /// identity (ADR 0030 addendum, personal-cfo-zrpg). Re-linking a key moves it.
    ///
    /// # Errors
    /// Returns [`DbError`] if the insert fails.
    pub fn link_merchant_alias(
        &self,
        normalized_key: &str,
        merchant_identity_id: Uuid,
        source: &str,
        confidence_bps: i64,
    ) -> Result<(), DbError> {
        let guard = self.lock();
        merchant_identity::link_alias(
            &guard.conn,
            normalized_key,
            merchant_identity_id,
            source,
            confidence_bps,
        )
    }

    /// Resolve a normalized merchant key to its canonical identity, if mapped (ADR 0030
    /// addendum, personal-cfo-zrpg) — the read seam for read-model population + grouping.
    ///
    /// # Errors
    /// Returns [`DbError`] if the read fails.
    pub fn resolve_merchant_identity(
        &self,
        normalized_key: &str,
    ) -> Result<Option<MerchantIdentity>, DbError> {
        merchant_identity::resolve(&self.read_connection()?, normalized_key)
    }

    /// Number of canonical merchant identities (ADR 0030 addendum) — observability seam.
    ///
    /// # Errors
    /// Returns [`DbError`] if the read fails.
    pub fn merchant_identity_count(&self) -> Result<u64, DbError> {
        merchant_identity::identity_count(&self.read_connection()?)
    }

    /// The most recent transactions (newest first, capped at `limit`), read
    /// directly from canonical tables so the list is always current regardless
    /// of read-model rebuild state. Backs the transactions list UI
    /// (personal-cfo-idsd).
    ///
    /// # Errors
    /// Returns [`DbError`] if the read fails.
    pub fn recent_transactions(&self, limit: u32) -> Result<Vec<TransactionRow>, DbError> {
        read_recent_transactions(&self.read_connection()?, limit)
    }

    /// The row DTOs for an explicit id set, newest first (personal-cfo-4d8.25.15) —
    /// resolves Money Inbox items that fall outside the recent-window list. Unknown or
    /// voided ids are absent from the result.
    ///
    /// # Errors
    /// Returns [`DbError`] if the read fails.
    pub fn transactions_by_ids(
        &self,
        ids: &[TransactionId],
    ) -> Result<Vec<TransactionRow>, DbError> {
        let uuids: Vec<Uuid> = ids.iter().map(|id| id.as_uuid()).collect();
        read_transactions_by_ids(&self.read_connection()?, &uuids)
    }

    /// One filtered, ordered page of transactions plus the total match count
    /// (personal-cfo-3fdd.1): the server-side search / filter / sort / paging read
    /// behind the Transactions tab and the global search palette, replacing the
    /// client-side scan of a 200-row window so all history is reachable.
    ///
    /// # Errors
    /// Returns [`DbError`] if the read fails.
    pub fn transaction_page(
        &self,
        query: &TransactionPageQuery,
    ) -> Result<TransactionPage, DbError> {
        read_transaction_page(&self.read_connection()?, query)
    }

    /// The committed ledger transactions that look like duplicates of a flagged staged
    /// transaction (same fingerprint, already committed) — the counterpart(s) for the
    /// duplicate Review panel (ADR 0032 §4, personal-cfo-4d8.20).
    ///
    /// # Errors
    /// Returns [`DbError`] if the read fails.
    pub fn duplicate_candidates(
        &self,
        staged_transaction_id: StagedTransactionId,
    ) -> Result<Vec<TransactionRow>, DbError> {
        read_duplicate_candidates(&self.read_connection()?, staged_transaction_id)
    }

    /// Every income source, with its next pay date computed from the schedule.
    /// Backs the income list UI (personal-cfo-le79).
    ///
    /// # Errors
    /// Returns [`DbError`] if the read fails.
    pub fn income_source_views(&self) -> Result<Vec<IncomeSourceView>, DbError> {
        read_income_source_views(&self.read_connection()?)
    }

    /// Every manual recurring bill, with its next due date computed from the
    /// schedule. Backs the bills list UI (personal-cfo-esmy / -apso).
    ///
    /// # Errors
    /// Returns [`DbError`] if the read fails.
    pub fn recurring_bill_views(&self) -> Result<Vec<RecurringBillView>, DbError> {
        read_recurring_bill_views(&self.read_connection()?)
    }

    /// An account's debt terms (ADR 0035 §5, personal-cfo-6wk.6), or `None` if unset.
    ///
    /// # Errors
    /// Returns [`DbError`] if the read fails.
    pub fn debt_terms(&self, account_id: AccountId) -> Result<Option<DebtTermsView>, DbError> {
        debt::read_debt_terms(&self.read_connection()?, account_id)
    }

    /// Debt terms for every account that has them, optionally scoped to an account set
    /// (personal-cfo-4d17). Accounts without terms are omitted — see the db-worker doc for
    /// why that is not the same as returning them empty.
    ///
    /// # Errors
    /// Returns [`DbError`] if the read fails.
    pub fn debt_terms_list(&self, account_ids: &[Uuid]) -> Result<Vec<DebtTermsView>, DbError> {
        debt::read_debt_terms_list(&self.read_connection()?, account_ids)
    }

    /// Every recurring transfer, with its next occurrence computed from the
    /// schedule (ADR 0026 §14, personal-cfo-npoe). Backs the transfers list UI.
    ///
    /// # Errors
    /// Returns [`DbError`] if the read fails.
    pub fn recurring_transfer_views(&self) -> Result<Vec<RecurringTransferView>, DbError> {
        read_recurring_transfer_views(&self.read_connection()?)
    }

    /// Every tag, ordered by name (ADR 0033, personal-cfo-2ryf). Backs the tag picker.
    ///
    /// # Errors
    /// Returns [`DbError`] if the read fails.
    pub fn tag_views(&self) -> Result<Vec<TagView>, DbError> {
        read_tag_views(&self.read_connection()?)
    }

    /// A transaction's split lines, ordered (ADR 0034, personal-cfo-kr9). Empty when the
    /// transaction is not split. Backs the expandable parent row.
    ///
    /// # Errors
    /// Returns [`DbError`] if the read fails.
    pub fn transaction_splits(
        &self,
        transaction_id: TransactionId,
    ) -> Result<Vec<SplitLineView>, DbError> {
        read_transaction_splits(&self.read_connection()?, transaction_id)
    }

    /// Rebuild the commitments projection from the canonical `recurring_events`
    /// and `bill_contracts` tables, returning the number of commitments written
    /// (plan §9.8, personal-cfo-rxw).
    ///
    /// # Errors
    /// Returns [`DbError`] if the projection fails.
    pub fn rebuild_commitments(&self) -> Result<u64, DbError> {
        let mut conn = self.read_connection()?;
        commitments::rebuild(&mut conn)
    }

    /// Every commitment (the forecast-facing obligation projection), ordered by
    /// name. (personal-cfo-rxw.)
    ///
    /// # Errors
    /// Returns [`DbError`] if the read fails.
    pub fn commitment_views(&self) -> Result<Vec<CommitmentView>, DbError> {
        commitments::read_rows(&self.read_connection()?)
    }

    /// The full category taxonomy (plan §9.6, ADR 0030, personal-cfo-bac) — the
    /// seeded defaults plus any user categories, for the management UI + pickers.
    ///
    /// # Errors
    /// Returns [`DbError`] if the read fails.
    pub fn category_views(&self) -> Result<Vec<CategoryView>, DbError> {
        categories::read_views(&self.read_connection()?)
    }

    /// Every active Money Inbox item (the triage surface), in default sort order
    /// (ADR 0014 §7, personal-cfo-dsq). Derived; written only by the projection.
    ///
    /// # Errors
    /// Returns [`DbError`] if the read fails.
    pub fn money_inbox_list(&self) -> Result<Vec<MoneyInboxItem>, DbError> {
        let conn = self.read_connection()?;
        // Household-local "today" (ADR 0021 §1), not UTC's — the snooze expiry
        // below is a calendar-boundary comparison (personal-cfo-m8x2r).
        let today = forecast::household_today(&conn)?.to_string();
        // The materialized event-driven items, minus those snoozed until a future
        // date (the snooze *expiry* is evaluated here, at read time, so the rebuild
        // checksum stays clock-independent; personal-cfo-ci71), plus the
        // time-derived items computed on read (stale balances, personal-cfo-r52x).
        let mut items: Vec<MoneyInboxItem> = money_inbox::read_rows(&conn)?
            .into_iter()
            .filter(|item| {
                item.snoozed_until
                    .as_deref()
                    .is_none_or(|until| until <= today.as_str())
            })
            .collect();
        items.extend(money_inbox::connector_error_items(&conn)?);
        items.extend(money_inbox::stale_balance_items(&conn, Utc::now())?);
        // The low-confidence-category review items (ADR 0030 addendum, personal-cfo-j5ij/
        // -uc95): auto-categorized below the confidence threshold + unreviewed, computed on
        // read from canonical state so accepting/recategorizing one drops it next read.
        let low_confidence = money_inbox::low_confidence_category_items(
            &conn,
            money_inbox::LOW_CONFIDENCE_CATEGORY_THRESHOLD_BPS,
        )?;
        // One item per transaction: a low-confidence row is also unreviewed, so suppress its
        // generic unreviewed item — the more actionable category card wins.
        let low_confidence_targets: std::collections::HashSet<Uuid> =
            low_confidence.iter().map(|item| item.target_id).collect();
        // The unreviewed-transaction review items (ADR 0032 §3, personal-cfo-4d8.7), also
        // computed on read from canonical state so reviewing one simply drops it next read.
        items.extend(
            money_inbox::unreviewed_transaction_items(&conn)?
                .into_iter()
                .filter(|item| !low_confidence_targets.contains(&item.target_id)),
        );
        items.extend(low_confidence);
        Ok(items)
    }

    /// The transaction ids in the low-confidence-category review queue (ADR 0030 addendum,
    /// personal-cfo-j5ij). The kernel's bulk-accept marks each reviewed via the command path.
    ///
    /// # Errors
    /// Returns [`DbError`] on a read failure.
    pub fn low_confidence_category_transaction_ids(&self) -> Result<Vec<Uuid>, DbError> {
        money_inbox::low_confidence_category_transaction_ids(
            &self.read_connection()?,
            money_inbox::LOW_CONFIDENCE_CATEGORY_THRESHOLD_BPS,
        )
    }

    /// Rebuild the Money Inbox read model from canonical state (the repair path;
    /// the commit pipeline refreshes it incrementally per staged-row change).
    /// Returns the number of items written (personal-cfo-dsq).
    ///
    /// # Errors
    /// Returns [`DbError`] if the projection fails.
    pub fn rebuild_money_inbox(&self) -> Result<u64, DbError> {
        let mut conn = self.read_connection()?;
        money_inbox::rebuild(&mut conn)
    }

    /// Rebuild the recurring-instance read model (ADR 0026 §9, personal-cfo-5ie.4/
    /// -5ie.5): project one instance per scheduled occurrence of an active recurring
    /// schedule over the window, then link each to the realized liquid-account
    /// posting that satisfies it. Rebuilt on demand by its consumer (the
    /// actualization loop, `46jq`); deterministic given today's date. Returns the
    /// number of instances written.
    ///
    /// # Errors
    /// Returns [`DbError`] if the projection fails or a schedule is malformed.
    /// Obligations whose scheduled date has passed with nothing recorded against them
    /// (personal-cfo-4d8.27.7.6, ADR 0058). Refreshes the instance projection first so the
    /// answer reflects today rather than whenever it was last rebuilt.
    ///
    /// # Errors
    /// Returns [`DbError`] if the rebuild or the read fails.
    pub fn unconfirmed_past_due(&self) -> Result<Vec<UnconfirmedOccurrence>, DbError> {
        let mut conn = self.read_connection()?;
        let today = forecast::household_today(&conn)?;
        recurring_instances::rebuild(&mut conn, today)?;
        recurring_instances::read_unconfirmed_past_due(&conn, today)
    }

    pub fn rebuild_recurring_instances(&self) -> Result<u64, DbError> {
        let mut conn = self.read_connection()?;
        let today = forecast::household_today(&conn)?;
        recurring_instances::rebuild(&mut conn, today)
    }

    /// One recurring bill's projected instances after refreshing the linking seam — the
    /// retro-attach surface (ADR 0047 §1, personal-cfo-4d8.25.8): right after a bill is
    /// created/approved, the caller shows which historical transactions matched its
    /// schedule (status `paid` + a `linked_transaction_id`). The rebuild is idempotent
    /// and deterministic, so refreshing on read is safe.
    ///
    /// # Errors
    /// Returns [`DbError`] if the rebuild or read fails.
    pub fn recurring_bill_history(
        &self,
        event_id: RecurringEventId,
    ) -> Result<Vec<RecurringInstanceRow>, DbError> {
        let mut conn = self.read_connection()?;
        let today = forecast::household_today(&conn)?;
        recurring_instances::rebuild(&mut conn, today)?;
        recurring_instances::read_rows_for_event(&conn, event_id.as_uuid())
    }

    /// Read the projected recurring instances (ADR 0026 §9). Caller is responsible
    /// for rebuilding first when fresh data is required.
    ///
    /// # Errors
    /// Returns [`DbError`] if the read fails.
    pub fn recurring_instances(
        &self,
    ) -> Result<Vec<recurring_instances::RecurringInstanceRow>, DbError> {
        recurring_instances::read_rows(&self.read_connection()?)
    }

    /// Actualize the persisted forecast runs (ADR 0026 §9/§17, personal-cfo-46jq):
    /// refresh the recurring-instance seam, then score every actualizable forecast
    /// row dated on or before today against the linked instances, recomputing
    /// `forecast_actuals` (exact / matched / missed / superseded). Idempotent.
    /// Returns the number of actuals written.
    ///
    /// # Errors
    /// Returns [`DbError`] if the projection or scoring fails.
    pub fn actualize_forecasts(&self) -> Result<u64, DbError> {
        let mut conn = self.read_connection()?;
        let today = forecast::household_today(&conn)?;
        recurring_instances::rebuild(&mut conn, today)?;
        let tx = conn.transaction()?;
        let count = forecast_actualize::actualize(&tx, today)?;
        tx.commit()?;
        Ok(count)
    }

    /// Number of `forecast_actuals` rows (ADR 0026 §9) — the observability seam for
    /// actualization and the input the readiness factors (A3) read.
    ///
    /// # Errors
    /// Returns [`DbError`] if the read fails.
    pub fn forecast_actuals_count(&self) -> Result<u64, DbError> {
        forecast_actualize::count(&self.read_connection()?)
    }

    /// Backtest the persisted forecast runs against realized actuals (ADR 0026 §18,
    /// personal-cfo-nxgx): aggregate the predicted↔realized pairs in `forecast_actuals`
    /// into a per-vault MAPE, plus the card statement estimator's walk-forward error
    /// (ADR 0039 addendum 2026-07-10 §2, personal-cfo-4d8.25.6), recording each in
    /// `forecast_backtest_results` when there's enough history. Returns the number of
    /// rows written (0..=2). Call after [`Self::actualize_forecasts`].
    ///
    /// # Errors
    /// Returns [`DbError`] if the read/write fails.
    pub fn backtest_forecasts(&self) -> Result<u64, DbError> {
        let conn = self.read_connection()?;
        let today = forecast::household_today(&conn)?;
        forecast_backtest::run_backtest(&conn, today, Self::DAILY_PERSIST_HORIZON_DAYS)
    }

    /// The deterministic Future Cash forecast over the next `horizon_days`
    /// (plan §13.3, personal-cfo-164u): the liquid-cash opening balance folded
    /// with the income + recurring-obligation event stream, one row per day.
    ///
    /// `scenario` selects the overlay: `None` is the base forecast (base
    /// assumption events only); `Some(id)` layers that scenario's scoped events
    /// (additions, modifications, removals) over the base (personal-cfo-6zep). The
    /// engine honors override/exclusion events via [`forecast::compute`].
    ///
    /// The wall clock is read here (the impure boundary) and passed to the pure
    /// engine as the horizon's "as of" instant, so the engine stays clock-free.
    ///
    /// # Errors
    /// Returns [`DbError`] on a read failure, a malformed stored schedule,
    /// mixed-currency liquid accounts, or a forecast arithmetic failure.
    pub fn future_cash_forecast(
        &self,
        horizon_days: u32,
        scenarios: &[Uuid],
    ) -> Result<ForecastView, DbError> {
        forecast::compute(
            &self.read_connection()?,
            Utc::now(),
            horizon_days,
            scenarios,
        )
    }

    /// The per-account and per-group Future Cash projection (ADR 0026 §12,
    /// personal-cfo-l8oh): one running-balance series per liquid account (plus an
    /// "Unallocated cash" series for un-attributable flows) and the cash-tier
    /// group rollups, reconciling to [`future_cash_forecast`]. Backs the
    /// multi-series chart + spreadsheet table.
    ///
    /// # Errors
    /// Returns [`DbError`] on a read failure, a malformed stored schedule,
    /// mixed-currency liquid accounts, or a forecast arithmetic failure.
    pub fn future_cash_by_account(
        &self,
        horizon_days: u32,
        scenarios: &[Uuid],
    ) -> Result<MultiSeriesForecast, DbError> {
        forecast::compute_by_account(
            &self.read_connection()?,
            Utc::now(),
            horizon_days,
            scenarios,
        )
    }

    /// The realized cash-flow HISTORY over the trailing `lookback_days` (cf-history,
    /// personal-cfo-4d8.27.5.2): each liquid account's actual daily closing balance,
    /// folded backward from today over the ledger postings and clamped to its
    /// earliest real data. Backs the Account Detail chart's realized line.
    ///
    /// # Errors
    /// Returns [`DbError`] on a read failure or mixed-currency liquid accounts.
    pub fn cash_flow_history(&self, lookback_days: u32) -> Result<CashFlowHistory, DbError> {
        forecast::compute_cash_flow_history(&self.read_connection()?, Utc::now(), lookback_days)
    }

    /// The per-card credit-card statement + payment forecast (ADR 0039 §2,
    /// personal-cfo-4lhm): each card's upcoming cycles with the projected statement
    /// balance (known card-charged bills + projected ordinary variable card spend),
    /// minimum due, full-pay amount, and the payment its repayment philosophy selects.
    ///
    /// # Errors
    /// Returns [`DbError`] on a read failure or a malformed stored schedule.
    pub fn card_statement_forecast(&self) -> Result<Vec<CardStatementForecastView>, DbError> {
        self.card_statement_forecast_at(Utc::now())
    }

    /// [`Self::card_statement_forecast`] evaluated at an explicit `as_of` instant — the clock
    /// seam for deterministic tests. The cycle layout depends on where `as_of` falls in the
    /// billing month (e.g. the grace window between statement close and payment due, ADR 0039
    /// §2, personal-cfo-4d8.23.1), so pinning `as_of` keeps those tests stable across run dates.
    pub fn card_statement_forecast_at(
        &self,
        as_of: DateTime<Utc>,
    ) -> Result<Vec<CardStatementForecastView>, DbError> {
        forecast::card_statement_forecast(&self.read_connection()?, as_of)
    }

    /// The past billing-cycle windows for one card with derived-from-imports charge totals
    /// and any user-recorded actual statements — the statement-history capture surface
    /// (ADR 0039 addendum 2026-07-10 §2, personal-cfo-4d8.25.4).
    ///
    /// # Errors
    /// Returns [`DbError`] on a read failure or malformed stored dates.
    pub fn card_statement_history(
        &self,
        account_id: AccountId,
    ) -> Result<Vec<CardStatementHistoryView>, DbError> {
        self.card_statement_history_at(account_id, Utc::now())
    }

    /// [`Self::card_statement_history`] evaluated at an explicit `as_of` instant — the
    /// clock seam for deterministic tests.
    pub fn card_statement_history_at(
        &self,
        account_id: AccountId,
        as_of: DateTime<Utc>,
    ) -> Result<Vec<CardStatementHistoryView>, DbError> {
        forecast::card_statement_history(&self.read_connection()?, account_id.as_uuid(), as_of)
    }

    /// Compare debt-paydown strategies (minimum-only / snowball / avalanche) for the current
    /// debts at `extra_budget_minor` extra per month (ADR 0036 debt_payoff, personal-cfo-od07):
    /// each plan's debt-free month + total interest, for the Debt sub-view's descriptive compare.
    ///
    /// # Errors
    /// Returns [`DbError`] on a read failure.
    pub fn debt_payoff_comparison(
        &self,
        extra_budget_minor: i64,
        account_ids: &[Uuid],
    ) -> Result<Vec<PayoffPlanView>, DbError> {
        forecast::debt_payoff_comparison(&self.read_connection()?, extra_budget_minor, account_ids)
    }

    /// Suspected loan double-counts: loans tracked as both a `loan_liability` account
    /// with payment terms AND an active recurring `loan_payment` bill, which
    /// double-counts the payment in the liquid forecast (personal-cfo-6wk.11). A
    /// heuristic, descriptive warning (ADR 0018); the app never auto-removes either.
    ///
    /// # Errors
    /// Returns [`DbError`] if a read fails.
    pub fn loan_double_count_warnings(&self) -> Result<Vec<LoanDoubleCount>, DbError> {
        loan_overlap::detect_loan_double_counts(&self.read_connection()?)
    }

    /// Candidate recurring bills detected from realized outflows (personal-cfo-98ql):
    /// merchants that recur at a consistent cadence within a stable amount band, with an
    /// inferred amount/frequency/next-date + confidence. Suggestions only — the user
    /// confirms before any recurring event is created; already-tracked merchants are excluded.
    ///
    /// # Errors
    /// Returns [`DbError`] if a read fails.
    pub fn recurring_candidates(&self) -> Result<Vec<RecurringCandidateView>, DbError> {
        let conn = self.read_connection()?;
        // Household-local "today" (ADR 0021 §1), not UTC's — the detector's next-date
        // projection and cadence window are calendar-boundary comparisons (personal-cfo-m8x2r).
        let today = forecast::household_today(&conn)?;
        recurring_detection::recurring_candidates(&conn, today)
    }

    /// Recurring inbound deposits not yet modeled as income sources
    /// (personal-cfo-gmnk): the same detector and suppression store as bills.
    ///
    /// # Errors
    /// Returns [`DbError`] if the read fails.
    pub fn income_candidates(&self) -> Result<Vec<RecurringCandidateView>, DbError> {
        let conn = self.read_connection()?;
        // Household-local "today" (ADR 0021 §1), not UTC's — same reasoning as
        // `recurring_candidates` (personal-cfo-m8x2r).
        let today = forecast::household_today(&conn)?;
        recurring_detection::income_candidates(&conn, today)
    }

    /// Compute the Future Cash forecast and persist it as a **reproducible run**
    /// (ADR 0026 §3, personal-cfo-eqfw): in one transaction, capture a
    /// content-addressed input snapshot, fold the deterministic Layer-1 forecast,
    /// and write the `forecast_run` + its `forecast_rows`. Returns the run id.
    ///
    /// Reproducible by construction: the same vault state + `as_of` dedups to the
    /// same snapshot and writes byte-identical row data (only the run/row ids and
    /// `generated_at` differ). Today's dashboard still reads the on-demand
    /// [`Self::future_cash_forecast`]; the persisted runs feed the reproducibility
    /// guarantee + later consumers (the chart, backtest, invalidation).
    ///
    /// # Errors
    /// [`DbError`] on a read/write failure or a forecast arithmetic failure.
    pub fn generate_and_persist_forecast(&self, horizon_days: u32) -> Result<Uuid, DbError> {
        self.persist_forecast_at(Utc::now(), horizon_days)
    }

    /// [`Self::generate_and_persist_forecast`] with an explicit `as_of` — the
    /// reproducibility seam (a fixed instant gives a deterministic run).
    fn persist_forecast_at(
        &self,
        as_of: DateTime<Utc>,
        horizon_days: u32,
    ) -> Result<Uuid, DbError> {
        let mut guard = self.lock();
        let tx = guard.conn.transaction()?;
        let (snapshot_id, content_hash) = forecast_persist::capture_snapshot(&tx)?;
        let view = forecast::compute(&tx, as_of, horizon_days, &[])?;
        let run_id = forecast_persist::persist_run_and_rows(
            &tx,
            &as_of.to_rfc3339(),
            horizon_days,
            snapshot_id,
            &content_hash,
            "layer1_deterministic",
            &view,
        )?;
        tx.commit()?;
        Ok(run_id)
    }

    /// The horizon for the daily-on-open persisted run (ADR 0026 §15): the maximum
    /// user-facing horizon (1Y), so the snapshot holds the fullest forward record
    /// for actualization to score any sub-window against. Independent of the
    /// dashboard default (90d) and not part of the dedup key.
    const DAILY_PERSIST_HORIZON_DAYS: u32 = 365;

    /// Activate the persisted forecast pipeline on vault open (ADR 0026 §15,
    /// personal-cfo-5ie.3). Persists at most **one** run per `(input content_hash,
    /// calendar day)`: if a run already exists today for the current input state it
    /// is a no-op. Returns the new run id, or `None` when deduped. Called once after
    /// unlock so the actualization loop (§9, `46jq`) accrues the time-spanning run
    /// history it scores against; the on-demand dashboard forecast is untouched.
    ///
    /// # Errors
    /// [`DbError`] on a read/write failure or a forecast arithmetic failure.
    pub fn persist_daily_forecast(&self) -> Result<Option<Uuid>, DbError> {
        self.persist_daily_forecast_at(Utc::now(), Self::DAILY_PERSIST_HORIZON_DAYS)
    }

    /// Count persisted forecast runs (ADR 0026 §3/§15). The observability seam for
    /// the daily-on-open activation and the read entry point the actualization loop
    /// (`46jq`) builds on.
    ///
    /// # Errors
    /// [`DbError`] on a read failure.
    pub fn persisted_forecast_run_count(&self) -> Result<u64, DbError> {
        let conn = self.read_connection()?;
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM forecast_runs", [], |r| r.get(0))?;
        Ok(u64::try_from(count).unwrap_or(0))
    }

    /// [`Self::persist_daily_forecast`] with an explicit `as_of` + horizon — the
    /// deterministic test seam. The dedup guard and the write share one transaction
    /// so a concurrent open cannot slip a duplicate run between the check and insert.
    fn persist_daily_forecast_at(
        &self,
        as_of: DateTime<Utc>,
        horizon_days: u32,
    ) -> Result<Option<Uuid>, DbError> {
        let mut guard = self.lock();
        let tx = guard.conn.transaction()?;
        let (snapshot_id, content_hash) = forecast_persist::capture_snapshot(&tx)?;
        // Run-level dedup: skip when a run for this input-state already exists today.
        // `generated_at` is RFC 3339, so its first 10 chars are the `YYYY-MM-DD` day.
        let today = as_of.format("%Y-%m-%d").to_string();
        let already: bool = tx.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM forecast_runs
                 WHERE assumptions_hash = ?1 AND substr(generated_at, 1, 10) = ?2
             )",
            params![content_hash, today],
            |r| r.get(0),
        )?;
        if already {
            return Ok(None);
        }
        let view = forecast::compute(&tx, as_of, horizon_days, &[])?;
        let run_id = forecast_persist::persist_run_and_rows(
            &tx,
            &as_of.to_rfc3339(),
            horizon_days,
            snapshot_id,
            &content_hash,
            "layer1_deterministic",
            &view,
        )?;
        tx.commit()?;
        Ok(Some(run_id))
    }

    /// Import `bytes` as an encrypted attachment (ADR 0023): derive the keyed
    /// storage id, encrypt under a fresh per-blob key wrapped by the DEK, write
    /// the ciphertext to `<vault>/blobs/` (never the OS temp dir), and record the
    /// metadata. Identical bytes dedup to the existing attachment. The bytes are
    /// not linked to any entity yet — call [`link_attachment`](Self::link_attachment).
    ///
    /// # Errors
    /// [`DbError::KeyUnavailable`] in passphrase mode; [`DbError::Crypto`] /
    /// [`DbError::Io`] / [`DbError::Sqlite`] on encrypt / write / insert.
    pub fn import_attachment(
        &self,
        bytes: &[u8],
        mime_type: Option<&str>,
        original_filename: Option<&str>,
    ) -> Result<core_ledger::AttachmentId, DbError> {
        let dek = self.dek()?;
        let storage_id = vault_crypto::storage_id(dek, bytes);
        let dir = attachments::blobs_dir(&self.path);
        let guard = self.lock();
        if guard.state != WorkerState::Healthy {
            return Err(DbError::WorkerUnavailable(guard.state));
        }
        // Live dedup is decided on metadata (not file existence) under the writer
        // lock, so the check and the insert cannot race.
        if let Some(existing) =
            attachments::find_id_by_storage_id(&guard.conn, storage_id.as_str())?
        {
            return Ok(existing);
        }
        let content_key = vault_crypto::generate_content_key()?;
        let blob = vault_crypto::encrypt_blob(&content_key, bytes)?;
        let wrapped = vault_crypto::wrap_content_key(dek, &content_key)?;
        attachments::write_blob(&dir, &storage_id, &blob.ciphertext)?;
        let id = core_ledger::AttachmentId::new();
        attachments::insert_attachment(
            &guard.conn,
            id,
            &storage_id,
            &wrapped,
            &blob.nonce,
            bytes.len() as u64,
            mime_type,
            original_filename,
        )?;
        Ok(id)
    }

    /// Link an imported attachment to a domain entity (many-to-many), bumping its
    /// reference count.
    ///
    /// # Errors
    /// [`DbError`] if the worker is unavailable or the write fails.
    pub fn link_attachment(
        &self,
        attachment_id: core_ledger::AttachmentId,
        entity_kind: &str,
        entity_id: uuid::Uuid,
    ) -> Result<(), DbError> {
        let mut guard = self.lock();
        if guard.state != WorkerState::Healthy {
            return Err(DbError::WorkerUnavailable(guard.state));
        }
        let tx = guard.conn.transaction()?;
        attachments::insert_link(&tx, attachment_id, entity_kind, entity_id)?;
        attachments::adjust_ref_count(&tx, attachment_id, 1)?;
        tx.commit()?;
        Ok(())
    }

    /// The attachments linked to a domain entity (metadata only — no bytes).
    ///
    /// # Errors
    /// [`DbError`] if the read fails.
    pub fn attachments_for(
        &self,
        entity_kind: &str,
        entity_id: uuid::Uuid,
    ) -> Result<Vec<AttachmentMeta>, DbError> {
        attachments::list_for(&self.read_connection()?, entity_kind, entity_id)
    }

    /// The raw imported source fields behind a committed transaction (ADR 0045 §2):
    /// every column the importer captured, resolved through the `created_from`
    /// provenance link to its `source_record`'s `normalized_json`. `None` for a
    /// manually-entered transaction (no import provenance). Fields are key-sorted
    /// (the `normalized_json` object is deserialized into a `BTreeMap`).
    pub fn imported_transaction_fields(
        &self,
        transaction_id: uuid::Uuid,
    ) -> Result<Option<ImportedTransactionFields>, DbError> {
        let conn = self.read_connection()?;
        let row = conn
            .query_row(
                "SELECT sr.normalized_json, sb.source_type, sb.imported_at
                   FROM source_provenance_links spl
                   JOIN source_records sr ON sr.id = spl.source_record_id
                   JOIN source_batches sb ON sb.id = sr.source_batch_id
                  WHERE spl.entity_type = 'ledger_transaction'
                    AND spl.entity_id = ?1
                    AND spl.relationship = 'created_from'
                  ORDER BY spl.id
                  LIMIT 1",
                params![transaction_id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()?;
        let Some((normalized_json, source_type, imported_at)) = row else {
            return Ok(None);
        };
        Ok(Some(ImportedTransactionFields {
            source_type,
            imported_at,
            fields: parse_normalized_fields(&normalized_json),
        }))
    }

    /// Decrypt and return an attachment's plaintext bytes **in memory** — the
    /// plaintext never touches disk outside the vault (ADR 0023).
    ///
    /// # Errors
    /// [`DbError::KeyUnavailable`] in passphrase mode;
    /// [`DbError::AttachmentNotFound`] if the id is unknown; [`DbError::Crypto`] /
    /// [`DbError::Io`] on unwrap / read / decrypt.
    pub fn read_attachment_bytes(
        &self,
        attachment_id: core_ledger::AttachmentId,
    ) -> Result<Vec<u8>, DbError> {
        let dek = self.dek()?;
        let dir = attachments::blobs_dir(&self.path);
        let fields = attachments::crypto_fields(&self.read_connection()?, attachment_id)?
            .ok_or(DbError::AttachmentNotFound)?;
        let content_key = vault_crypto::unwrap_content_key(dek, &fields.wrapped)?;
        let blob = attachments::read_blob(&dir, &fields.storage_id, fields.content_nonce)?;
        Ok(vault_crypto::decrypt_blob(&content_key, &blob)?)
    }

    /// Remove one link from a domain entity to an attachment. When the **last**
    /// link is removed the attachment is crypto-shredded: the metadata row (the
    /// only wrapped content key) is deleted — making the blob permanently
    /// undecryptable — and the file is then unlinked.
    ///
    /// # Errors
    /// [`DbError`] if the worker is unavailable or a write fails.
    pub fn unlink_attachment(
        &self,
        attachment_id: core_ledger::AttachmentId,
        entity_kind: &str,
        entity_id: uuid::Uuid,
    ) -> Result<(), DbError> {
        let dir = attachments::blobs_dir(&self.path);
        let mut guard = self.lock();
        if guard.state != WorkerState::Healthy {
            return Err(DbError::WorkerUnavailable(guard.state));
        }
        let tx = guard.conn.transaction()?;
        let removed = attachments::delete_link(&tx, attachment_id, entity_kind, entity_id)?;
        let shredded_storage_id =
            if removed > 0 && attachments::adjust_ref_count(&tx, attachment_id, -1)? == 0 {
                attachments::take_storage_id_and_delete(&tx, attachment_id)?
            } else {
                None
            };
        tx.commit()?;
        // After the row (and its wrapped key) is gone the blob is already
        // undecryptable; unlink the ciphertext best-effort.
        if let Some(storage_id) = shredded_storage_id {
            attachments::remove_blob(&dir, &storage_id)?;
        }
        Ok(())
    }

    /// Borrow the raw DEK, or fail if the vault was opened in passphrase mode —
    /// attachments require the raw key (ADR 0002 / ADR 0023).
    fn dek(&self) -> Result<&Dek, DbError> {
        match &self.key {
            KeyMaterial::Raw(dek) => Ok(dek),
            KeyMaterial::Passphrase(_) => Err(DbError::KeyUnavailable),
        }
    }

    /// A stable content checksum of the commitments projection (proves the
    /// rebuild is deterministic).
    ///
    /// # Errors
    /// Returns [`DbError`] if the read fails.
    pub fn commitments_checksum(&self) -> Result<u64, DbError> {
        commitments::checksum(&self.read_connection()?)
    }

    /// The vault's identity + configuration (plan §9.2, personal-cfo-2lm).
    ///
    /// # Errors
    /// Returns [`DbError`] if the read connection cannot be opened, or the
    /// singleton `vault_metadata` row is missing or unreadable (a corrupt-vault
    /// signal the state machine, `personal-cfo-tg5`, will consume).
    pub fn vault_metadata(&self) -> Result<VaultMetadata, DbError> {
        read_vault_metadata(&self.read_connection()?)
    }

    /// Set the household's IANA timezone (`personal-cfo-q329`) — the calendar-boundary
    /// authority ADR 0021 §1 already named, previously unsettable after vault creation.
    /// Configuration on the singleton `vault_metadata` row, not a ledger write, so this
    /// goes through the single-writer connection directly rather than `WriteCommand`
    /// (mirrors `set_setting`'s "settings, not ledger writes" rationale).
    ///
    /// # Errors
    /// Returns [`DbError::InvalidCommand`] if `tz` is not a valid IANA timezone name, or
    /// [`DbError`] if the write fails.
    pub fn set_household_timezone(&self, tz: &str) -> Result<(), DbError> {
        let guard = self.lock();
        forecast::write_household_tz(&guard.conn, tz)
    }

    /// Number of entries in the operation log.
    ///
    /// # Errors
    /// Returns [`DbError`] if the read connection cannot be opened or the query
    /// fails.
    pub fn operation_count(&self) -> Result<u64, DbError> {
        self.read_connection()?.operation_count()
    }

    /// Full rebuild of the transaction-display read model from canonical tables.
    /// Returns the number of rows written.
    ///
    /// # Errors
    /// Returns [`DbError`] if the projection fails.
    pub fn rebuild_transaction_display(&self) -> Result<u64, DbError> {
        let mut conn = self.read_connection()?;
        projection::rebuild(&mut conn)
    }

    /// Incrementally project new operations into the transaction-display read
    /// model. Returns the number of rows upserted.
    ///
    /// # Errors
    /// Returns [`DbError`] if the projection fails.
    pub fn project_transaction_display_incremental(&self) -> Result<u64, DbError> {
        let mut conn = self.read_connection()?;
        projection::incremental(&mut conn)
    }

    /// All transaction-display rows in deterministic order.
    ///
    /// # Errors
    /// Returns [`DbError`] if the read fails.
    pub fn transaction_display_rows(&self) -> Result<Vec<TransactionDisplayRow>, DbError> {
        projection::read_rows(&self.read_connection()?)
    }

    /// A stable content checksum of the transaction-display read model.
    ///
    /// # Errors
    /// Returns [`DbError`] if the read fails.
    pub fn transaction_display_checksum(&self) -> Result<u64, DbError> {
        projection::checksum(&self.read_connection()?)
    }

    /// The transaction-display projection cursor (last applied op-log sequence).
    ///
    /// # Errors
    /// Returns [`DbError`] if the read fails.
    pub fn transaction_projection_cursor(&self) -> Result<i64, DbError> {
        projection::cursor(&self.read_connection()?)
    }

    /// Verify the writer connection is configured per policy.
    ///
    /// # Errors
    /// Returns [`DbError::SelfTestFailed`] if journal mode is not WAL, or a
    /// SQLite error if a pragma cannot be read.
    pub fn self_test(&self) -> Result<SelfTestReport, DbError> {
        let guard = self.lock();
        let journal_mode: String = guard
            .conn
            .pragma_query_value(None, "journal_mode", |r| r.get(0))?;
        let busy_timeout_ms: i64 = guard
            .conn
            .pragma_query_value(None, "busy_timeout", |r| r.get(0))?;
        let journal_size_limit: i64 =
            guard
                .conn
                .pragma_query_value(None, "journal_size_limit", |r| r.get(0))?;
        let report = SelfTestReport {
            journal_mode,
            busy_timeout_ms,
            journal_size_limit,
        };
        if !report.journal_mode.eq_ignore_ascii_case("wal") {
            return Err(DbError::SelfTestFailed(format!(
                "journal_mode is {}, expected wal",
                report.journal_mode
            )));
        }
        Ok(report)
    }

    /// Whether `PRAGMA integrity_check` reports `ok` — the vault database is not
    /// corrupt. Part of the vault health check (personal-cfo-n9w); the strongest
    /// signal for a vault holding real data. A query failure reads as not-ok.
    #[must_use]
    pub fn integrity_ok(&self) -> bool {
        let guard = self.lock();
        guard
            .conn
            .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .map(|result| result.eq_ignore_ascii_case("ok"))
            .unwrap_or(false)
    }

    /// Whether every stored attachment's encrypted blob is present under `blobs/` —
    /// the attachment manifest matches the blob store (personal-cfo-n9w). A missing
    /// blob (deleted/lost file) means an attachment can no longer be decrypted.
    ///
    /// # Errors
    /// Returns [`DbError`] on a read failure.
    pub fn attachments_consistent(&self) -> Result<bool, DbError> {
        let dir = self.blobs_dir();
        let guard = self.lock();
        let mut stmt = guard.conn.prepare("SELECT storage_id FROM attachments")?;
        let storage_ids: Vec<String> = stmt
            .query_map([], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        drop(stmt);
        Ok(storage_ids.iter().all(|sid| dir.join(sid).exists()))
    }

    /// The schema version this worker migrated the vault to (the highest applied
    /// migration). Used by the vault health check to assert schema coherence.
    #[must_use]
    pub fn schema_version(&self) -> i64 {
        self.schema_version
    }

    /// The linked SQLCipher version string (`PRAGMA cipher_version`), e.g.
    /// `"4.5.7 community"`. Pinned in `docs/architecture/stack.md`; the
    /// cross-version vault test (personal-cfo-7igv) asserts it so a silent
    /// SQLCipher upgrade that could break vault compatibility (risk
    /// personal-cfo-aia0) fails CI instead of shipping.
    ///
    /// # Errors
    /// [`DbError::SelfTestFailed`] if the pragma returns no row (a non-SQLCipher
    /// SQLite build), or [`DbError::Sqlite`] if it cannot be read.
    pub fn cipher_version(&self) -> Result<String, DbError> {
        let guard = self.lock();
        let version: Option<String> = guard
            .conn
            .query_row("PRAGMA cipher_version", [], |r| r.get(0))
            .optional()?;
        version.ok_or_else(|| {
            DbError::SelfTestFailed(
                "PRAGMA cipher_version returned no row (not a SQLCipher build)".to_owned(),
            )
        })
    }

    /// Append an immutable audit event (personal-cfo-n7bo). For non-financial
    /// security/compliance events — e.g. the onboarding no-reset-warning
    /// acknowledgement — which do not belong in the entity-scoped financial
    /// `operation_log`. Carries the command + correlation provenance from `meta`
    /// and a timestamp; the table is append-only (UPDATE/DELETE abort).
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a write failure.
    pub fn record_audit_event(&self, meta: &CommandMeta, event_type: &str) -> Result<(), DbError> {
        let guard = self.lock();
        guard.conn.execute(
            "INSERT INTO audit_events (id, event_type, command_id, correlation_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                Uuid::now_v7(),
                event_type,
                meta.command_id,
                meta.correlation_id,
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    /// The number of recorded audit events of `event_type`. Lets onboarding (and
    /// tests) confirm the no-reset-warning acknowledgement was persisted.
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a read failure.
    pub fn audit_event_count(&self, event_type: &str) -> Result<u64, DbError> {
        let n: i64 = self.read_connection()?.query_row(
            "SELECT COUNT(*) FROM audit_events WHERE event_type = ?1",
            [event_type],
            |r| r.get(0),
        )?;
        Ok(u64::try_from(n).unwrap_or(0))
    }

    /// Upsert an app-level setting (personal-cfo-p5g). `value` is a JSON or scalar
    /// string. Settings are configuration, not ledger mutations, so — like
    /// `vault_metadata` and `audit_events` — they bypass the `WriteCommand` bus.
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a write failure.
    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), DbError> {
        let guard = self.lock();
        guard.conn.execute(
            "INSERT INTO settings (key, value, value_schema_version, updated_at)
             VALUES (?1, ?2, 1, ?3)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value,
                 updated_at = excluded.updated_at",
            params![key, value, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    /// File-level dedupe (ADR 0014 §3): the id of an already-committed batch with
    /// this whole-file `fingerprint`, if any — an exact re-upload can be skipped.
    ///
    /// A batch stops counting once none of its committed rows survive in the ledger
    /// (every one voided): the user deleted that import, and re-uploading the same
    /// file is a deliberate restore, not an accidental duplicate (feedback 2026-07-03).
    pub fn batch_with_fingerprint(&self, fingerprint: &str) -> Result<Option<Uuid>, DbError> {
        self.read_connection()?
            .query_row(
                "SELECT sb.id FROM source_batches sb
                  WHERE sb.file_fingerprint = ?1
                    AND sb.status IN ('committed', 'partially_committed')
                    AND EXISTS (
                      SELECT 1 FROM staged_transactions st
                      JOIN source_records sr ON sr.id = st.source_record_id
                      JOIN ledger_transactions lt ON lt.id = st.committed_transaction_id
                       WHERE sr.source_batch_id = sb.id
                         AND st.commit_status = 'committed'
                         AND lt.voided_at IS NULL
                    )
                  LIMIT 1",
                params![fingerprint],
                |r| r.get::<_, Uuid>(0),
            )
            .optional()
            .map_err(DbError::from)
    }

    /// Persist a parser run for `batch_id` from a [`ParserRunReport`]
    /// (personal-cfo-hs9). Scratch metadata — like settings/audit events it
    /// bypasses the command bus.
    pub fn record_parser_run(
        &self,
        batch_id: Uuid,
        report: &ParserRunReport,
    ) -> Result<(), DbError> {
        let now = Utc::now().to_rfc3339();
        let guard = self.lock();
        ingestion::record_parser_run(
            &guard.conn,
            &ingestion::NewParserRun {
                source_batch_id: batch_id,
                parser_name: report.plugin_id,
                parser_version: &report.plugin_version,
                bytes_in: i64::try_from(report.bytes_in).unwrap_or(i64::MAX),
                records_out: i64::try_from(report.records_out).unwrap_or(i64::MAX),
                status: report.status.as_str(),
                limit_hit: report.limit_hit.as_deref(),
                finished_at: Some(&now),
            },
        )?;
        Ok(())
    }

    /// Bulk-stage a parsed batch (personal-cfo-cmx): insert its source_records +
    /// staged transactions/accounts/balances in one transaction, all proposed
    /// against `target_account`. Returns the new staged-transaction ids (commit
    /// them with `CommitStaged`). Staging is scratch, so this bypasses the command
    /// bus; the audited writes are the per-transaction commits.
    pub fn stage_parsed_batch(
        &self,
        batch_id: Uuid,
        batch: &ParsedBatch,
        target_account: Uuid,
    ) -> Result<Vec<Uuid>, DbError> {
        let guard = self.lock();
        let tx = guard.conn.unchecked_transaction()?;
        for acct in &batch.accounts {
            ingestion::stage_account(
                &tx,
                &ingestion::NewStagedAccount {
                    source_batch_id: batch_id,
                    external_name: acct.external_name.as_deref(),
                    external_number_hash: acct.external_number_hash.as_deref(),
                    proposed_subtype: acct.proposed_subtype.as_deref(),
                    matched_account_id: Some(target_account),
                },
            )?;
        }
        let mut staged = Vec::new();
        for rec in &batch.records {
            // insert_source_record dedupes on (batch, source_hash) and returns
            // the EFFECTIVE id — staged rows must reference that, never a
            // freshly-minted id that was not inserted.
            let record_id = ingestion::insert_source_record(
                &tx,
                Uuid::now_v7(),
                &ingestion::NewSourceRecord {
                    source_batch_id: batch_id,
                    external_id: rec.external_id.as_deref(),
                    source_hash: &rec.source_hash,
                    normalized_json: &rec.normalized_json,
                    parse_confidence_bps: rec.parse_confidence_bps.map(i64::from),
                },
            )?;
            if let Some(txn) = &rec.transaction {
                let posted = txn.posted_date.to_string();
                let transaction_date = txn.transaction_date.map(|d| d.to_string());
                let id = ingestion::stage_transaction(
                    &tx,
                    &ingestion::NewStagedTransaction {
                        source_record_id: record_id,
                        proposed_account_id: Some(target_account),
                        posted_at: &posted,
                        transaction_date: transaction_date.as_deref(),
                        amount_minor: txn.amount.minor_units(),
                        currency: txn.amount.currency().code(),
                        normalized_merchant: txn.normalized_merchant.as_deref(),
                        description: txn.description.as_deref(),
                        imported_category: txn.category.as_deref(),
                        txn_fingerprint: &txn.txn_fingerprint,
                    },
                )?;
                staged.push(id);
            }
            if let Some(bal) = &rec.balance {
                let observed = bal.observed_at.to_string();
                ingestion::stage_balance(
                    &tx,
                    &ingestion::NewStagedBalance {
                        source_record_id: record_id,
                        account_ref: Some(target_account),
                        observed_at: &observed,
                        balance_minor: bal.amount.minor_units(),
                        currency: bal.amount.currency().code(),
                    },
                )?;
            }
        }
        tx.commit()?;
        Ok(staged)
    }

    /// Bulk-stage a connector sync batch (personal-cfo-gglk): like
    /// [`Self::stage_parsed_batch`], but each record resolves its account
    /// through `account_map` — connector-core account key → real account —
    /// instead of a single target. Records whose external account is unmapped
    /// (or absent) are **not staged**; their count is returned so the caller
    /// surfaces them rather than silently dropping data (ADR 0014 §3 ethos).
    /// Staged accounts carry the map's answer as `matched_account_id`
    /// (`None` = discovered but unmapped).
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a write failure.
    pub fn stage_sync_batch(
        &self,
        batch_id: Uuid,
        batch: &ParsedBatch,
        account_map: &std::collections::BTreeMap<String, Uuid>,
    ) -> Result<(Vec<connectors::StagedSyncTxn>, usize), DbError> {
        let guard = self.lock();
        let tx = guard.conn.unchecked_transaction()?;
        for acct in &batch.accounts {
            let mapped = acct
                .external_id
                .as_deref()
                .and_then(|key| account_map.get(key))
                .copied();
            ingestion::stage_account(
                &tx,
                &ingestion::NewStagedAccount {
                    source_batch_id: batch_id,
                    external_name: acct.external_name.as_deref(),
                    external_number_hash: acct.external_number_hash.as_deref(),
                    proposed_subtype: acct.proposed_subtype.as_deref(),
                    matched_account_id: mapped,
                },
            )?;
        }
        let mut staged = Vec::new();
        let mut skipped_unmapped = 0_usize;
        for rec in &batch.records {
            // Resolve the record's account up front: a record with no mapped
            // account cannot be staged (the commit path requires one).
            let record_account =
                |external: Option<&str>| external.and_then(|key| account_map.get(key)).copied();
            let txn_account = rec
                .transaction
                .as_ref()
                .and_then(|t| record_account(t.external_account.as_deref()));
            let bal_account = rec
                .balance
                .as_ref()
                .and_then(|b| record_account(b.external_account.as_deref()));
            if rec.transaction.as_ref().is_some() && txn_account.is_none()
                || rec.balance.as_ref().is_some() && bal_account.is_none()
            {
                skipped_unmapped += 1;
                continue;
            }

            let record_id = ingestion::insert_source_record(
                &tx,
                Uuid::now_v7(),
                &ingestion::NewSourceRecord {
                    source_batch_id: batch_id,
                    external_id: rec.external_id.as_deref(),
                    source_hash: &rec.source_hash,
                    normalized_json: &rec.normalized_json,
                    parse_confidence_bps: rec.parse_confidence_bps.map(i64::from),
                },
            )?;
            if let (Some(txn), Some(account)) = (&rec.transaction, txn_account) {
                let posted = txn.posted_date.to_string();
                let transaction_date = txn.transaction_date.map(|d| d.to_string());
                let id = ingestion::stage_transaction(
                    &tx,
                    &ingestion::NewStagedTransaction {
                        source_record_id: record_id,
                        proposed_account_id: Some(account),
                        posted_at: &posted,
                        transaction_date: transaction_date.as_deref(),
                        amount_minor: txn.amount.minor_units(),
                        currency: txn.amount.currency().code(),
                        normalized_merchant: txn.normalized_merchant.as_deref(),
                        description: txn.description.as_deref(),
                        imported_category: txn.category.as_deref(),
                        txn_fingerprint: &txn.txn_fingerprint,
                    },
                )?;
                staged.push(connectors::StagedSyncTxn {
                    staged_id: id,
                    txn_fingerprint: txn.txn_fingerprint.clone(),
                    account_id: account,
                });
            }
            if let (Some(bal), Some(account)) = (&rec.balance, bal_account) {
                // Stage, then promote in the SAME transaction (ADR 0027
                // addendum, personal-cfo-yl53): a mapped account's provider
                // balance becomes a balance_observation immediately —
                // last-wins per (account, day), provenance via the source
                // record, staged row consumed. Provider balances commit
                // regardless of how the batch's transactions later triage.
                // The provider's epoch collapses to a UTC date upstream; an
                // evening sync west of UTC lands "tomorrow". A balance cannot
                // be observed in the household's future, so clamp to the
                // local today — keeping the same-day tie honest (a manual
                // assertion recorded later still wins on created_at).
                let local_today = forecast::read_household_tz(&tx)
                    .map(|tz| Utc::now().with_timezone(&tz).date_naive())
                    .unwrap_or_else(|_| Utc::now().date_naive());
                let observed = bal.observed_at.min(local_today).to_string();
                let staged_bal_id = ingestion::stage_balance(
                    &tx,
                    &ingestion::NewStagedBalance {
                        source_record_id: record_id,
                        account_ref: Some(account),
                        observed_at: &bal.observed_at.to_string(),
                        balance_minor: bal.amount.minor_units(),
                        currency: bal.amount.currency().code(),
                    },
                )?;
                // Currency guard (parity with record_balance_assertion and the
                // txn commit path): a cross-currency provider balance must not
                // silently anchor the account. It stays staged, un-promoted.
                let account_currency: Option<String> = tx
                    .query_row(
                        "SELECT currency FROM accounts WHERE id = ?1",
                        [account],
                        |r| r.get(0),
                    )
                    .optional()?;
                if account_currency.as_deref() != Some(bal.amount.currency().code()) {
                    continue;
                }
                tx.execute(
                    "DELETE FROM balance_observations
                      WHERE account_id = ?1 AND observed_at = ?2
                        AND source = 'connector_sync'",
                    params![account, observed],
                )?;
                tx.execute(
                    "INSERT INTO balance_observations
                        (id, account_id, observed_at, balance_amount_minor,
                         balance_currency, source, source_record_id,
                         reconciliation_session_id, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, 'connector_sync', ?6, NULL, ?7)",
                    params![
                        Uuid::now_v7(),
                        account,
                        observed,
                        bal.amount.minor_units(),
                        bal.amount.currency().code(),
                        record_id,
                        Utc::now().to_rfc3339(),
                    ],
                )?;
                tx.execute(
                    "DELETE FROM staged_balances WHERE id = ?1",
                    params![staged_bal_id],
                )?;
            }
        }
        tx.commit()?;
        Ok((staged, skipped_unmapped))
    }

    /// Whether an exact `txn_fingerprint` is already committed (non-voided) at
    /// `account`, excluding `staged_id` itself — the connector re-sync
    /// pre-check (personal-cfo-gglk): a provider-id fingerprint match on an
    /// overlap re-fetch is a CERTAIN duplicate, silently skipped rather than
    /// flagged into the Money Inbox.
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a read failure.
    pub fn txn_fingerprint_already_committed(
        &self,
        txn_fingerprint: &str,
        account: Uuid,
        excluding_staged_id: Uuid,
    ) -> Result<bool, DbError> {
        let conn = self.read_connection()?;
        ingestion::fingerprint_already_committed(
            &conn,
            txn_fingerprint,
            Some(account),
            excluding_staged_id,
        )
    }

    /// Like [`Self::txn_fingerprint_already_committed`], but also true for a
    /// fingerprint sitting flagged or skipped from an earlier batch — the
    /// connector pre-check (tevp): a rewind re-fetch of an UNRESOLVED (or
    /// deliberately skipped) collision must not mint a fresh inbox item on
    /// every sync.
    pub fn txn_fingerprint_already_tracked(
        &self,
        txn_fingerprint: &str,
        account: Uuid,
        excluding_staged_id: Uuid,
    ) -> Result<bool, DbError> {
        let conn = self.read_connection()?;
        ingestion::fingerprint_already_tracked(
            &conn,
            txn_fingerprint,
            Some(account),
            excluding_staged_id,
        )
    }

    /// `(total, committed, flagged)` staged-transaction counts for a batch
    /// (personal-cfo-cmx) — used to set the batch's terminal state + the result.
    pub fn staged_commit_tally(&self, batch_id: Uuid) -> Result<(u32, u32, u32), DbError> {
        let conn = self.read_connection()?;
        let (total, committed, flagged): (i64, i64, i64) = conn.query_row(
            "SELECT COUNT(*),
                    COALESCE(SUM(CASE WHEN t.commit_status = 'committed' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN t.commit_status = 'flagged' THEN 1 ELSE 0 END), 0)
               FROM staged_transactions t
               JOIN source_records r ON r.id = t.source_record_id
              WHERE r.source_batch_id = ?1",
            params![batch_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        Ok((
            u32::try_from(total).unwrap_or(u32::MAX),
            u32::try_from(committed).unwrap_or(u32::MAX),
            u32::try_from(flagged).unwrap_or(u32::MAX),
        ))
    }

    /// Read an app-level setting's value, or `None` when it has never been set
    /// (personal-cfo-p5g).
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a read failure.
    pub fn get_setting(&self, key: &str) -> Result<Option<String>, DbError> {
        Ok(self
            .read_connection()?
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()?)
    }

    // ===== Forecast assumption events (ADR 0026 §4, personal-cfo-5u2) =====
    // Forecast inputs, not ledger mutations — written directly (the `settings`
    // shape), bypassing the `WriteCommand` bus. Events are run-independent: they
    // persist across recomputes, gated by `status`, so a user override survives
    // a re-run until explicitly cleared or superseded. See [`assumptions`].

    /// Record a new assumption event (status `active`, timestamps "now").
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a write failure.
    pub fn record_assumption_event(&self, ev: &NewAssumptionEvent) -> Result<(), DbError> {
        let now = Utc::now().to_rfc3339();
        self.lock().conn.execute(
            "INSERT INTO forecast_assumption_events
                (id, kind, target_entity_type, target_entity_id, params_json,
                 source, scenario_id, status, superseded_by, origin_run_id,
                 created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'active', NULL, ?8, ?9, ?9)",
            params![
                ev.id,
                ev.kind.as_token(),
                ev.target_entity_type,
                ev.target_entity_id,
                ev.params_json,
                ev.source.as_token(),
                ev.scenario_id,
                ev.origin_run_id,
                now,
            ],
        )?;
        Ok(())
    }

    /// Record a typed forecast assumption event (personal-cfo-6zep) — the
    /// generalized creator behind `create_forecast_assumption`. Builds the
    /// canonical `params_json` for the [`ForecastAssumptionSpec`]'s shape
    /// (additions, amount/date modifications, removals) and records it as a
    /// `user_override` event, base or scenario-scoped.
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a write failure.
    pub fn record_forecast_assumption(&self, spec: &ForecastAssumptionSpec) -> Result<(), DbError> {
        self.record_assumption_event(&spec.as_event())
    }

    /// List the `active` assumption events for a scenario: `None` returns the
    /// base assumptions (`scenario_id IS NULL`); `Some(id)` returns exactly that
    /// scenario's events. Composing base + scenario overlays is the caller's job.
    ///
    /// # Errors
    /// [`DbError`] on a read failure or an unrecognized stored token.
    pub fn active_assumption_events(
        &self,
        scenario: Option<Uuid>,
    ) -> Result<Vec<AssumptionEventView>, DbError> {
        let conn = self.read_connection()?;
        let mut stmt = conn.prepare(
            "SELECT id, kind, target_entity_type, target_entity_id, params_json,
                    source, scenario_id, status, superseded_by, origin_run_id,
                    created_at, updated_at, promoted_from_scenario_id
             FROM forecast_assumption_events
             WHERE status = 'active'
               AND ((?1 IS NULL AND scenario_id IS NULL) OR scenario_id = ?1)
             ORDER BY created_at, id",
        )?;
        let mut out = Vec::new();
        let mut rows = stmt.query(params![scenario])?;
        while let Some(row) = rows.next()? {
            out.push(AssumptionEventView::from_row(row)?);
        }
        Ok(out)
    }

    /// Supersede an assumption event with another (status `superseded`; the row
    /// is retained for history, `superseded_by` points at the replacement).
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a write failure.
    pub fn supersede_assumption_event(&self, id: Uuid, superseded_by: Uuid) -> Result<(), DbError> {
        self.lock().conn.execute(
            "UPDATE forecast_assumption_events
             SET status = 'superseded', superseded_by = ?2, updated_at = ?3
             WHERE id = ?1",
            params![id, superseded_by, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    /// Clear (deactivate) an assumption event without deleting it.
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a write failure.
    pub fn clear_assumption_event(&self, id: Uuid) -> Result<(), DbError> {
        self.lock().conn.execute(
            "UPDATE forecast_assumption_events
             SET status = 'cleared', updated_at = ?2
             WHERE id = ?1",
            params![id, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    /// Record per-run attribution edges (assumption event → forecast row).
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a write failure.
    pub fn record_dependency_edges(&self, edges: &[DependencyEdge]) -> Result<(), DbError> {
        let now = Utc::now().to_rfc3339();
        let guard = self.lock();
        for e in edges {
            guard.conn.execute(
                "INSERT INTO forecast_dependency_edges
                    (id, forecast_run_id, from_event_id, to_row_id, edge_type, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    e.id,
                    e.forecast_run_id,
                    e.from_event_id,
                    e.to_row_id,
                    e.edge_type.as_token(),
                    now,
                ],
            )?;
        }
        Ok(())
    }

    /// The dependency edges pointing at a forecast row — the assumption events
    /// that shaped it (the join behind per-row explanation).
    ///
    /// # Errors
    /// [`DbError`] on a read failure or an unrecognized stored token.
    pub fn dependency_edges_for_row(&self, row_id: Uuid) -> Result<Vec<DependencyEdge>, DbError> {
        let conn = self.read_connection()?;
        let mut stmt = conn.prepare(
            "SELECT id, forecast_run_id, from_event_id, to_row_id, edge_type
             FROM forecast_dependency_edges WHERE to_row_id = ?1 ORDER BY id",
        )?;
        let mut out = Vec::new();
        let mut rows = stmt.query(params![row_id])?;
        while let Some(row) = rows.next()? {
            out.push(DependencyEdge::from_row(row)?);
        }
        Ok(out)
    }

    /// Enqueue a date range for recompute.
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a write failure.
    pub fn enqueue_dirty_range(&self, range: &DirtyRange) -> Result<(), DbError> {
        self.lock().conn.execute(
            "INSERT INTO forecast_dirty_ranges
                (id, from_date, to_date, reason, triggering_event_id, created_at, resolved_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)",
            params![
                range.id,
                range.from_date.to_string(),
                range.to_date.to_string(),
                range.reason.as_token(),
                range.triggering_event_id,
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    /// The open (unresolved) dirty ranges, oldest first.
    ///
    /// # Errors
    /// [`DbError`] on a read failure or an unrecognized stored token.
    pub fn open_dirty_ranges(&self) -> Result<Vec<DirtyRange>, DbError> {
        let conn = self.read_connection()?;
        let mut stmt = conn.prepare(
            "SELECT id, from_date, to_date, reason, triggering_event_id
             FROM forecast_dirty_ranges WHERE resolved_at IS NULL
             ORDER BY created_at, id",
        )?;
        let mut out = Vec::new();
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            out.push(DirtyRange::from_row(row)?);
        }
        Ok(out)
    }

    /// Mark a dirty range resolved (recomputed); it leaves the open set.
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a write failure.
    pub fn resolve_dirty_range(&self, id: Uuid) -> Result<(), DbError> {
        self.lock().conn.execute(
            "UPDATE forecast_dirty_ranges SET resolved_at = ?2 WHERE id = ?1",
            params![id, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    // ===== Reproducible forecast persistence (ADR 0026 §3, personal-cfo-eqfw) =====
    // Content-addressed input snapshots, the model registry, and run diffs — the
    // storage layer beneath persisted, reproducible forecasts. See
    // [`forecast_persist`].

    /// Capture a content-addressed snapshot of the current forecast inputs and
    /// return its id. **Idempotent by content:** if a snapshot with the same
    /// content hash already exists, its id is returned and no new row is written —
    /// so identical inputs map to the same snapshot id.
    ///
    /// # Errors
    /// [`DbError`] on a read/write failure.
    pub fn capture_input_snapshot(&self) -> Result<Uuid, DbError> {
        let guard = self.lock();
        Ok(forecast_persist::capture_snapshot(&guard.conn)?.0)
    }

    /// Read a captured input snapshot by id (its content-addressed fields).
    ///
    /// # Errors
    /// [`DbError`] on a read failure.
    pub fn input_snapshot(&self, id: Uuid) -> Result<Option<InputSnapshotView>, DbError> {
        let conn = self.read_connection()?;
        let mut stmt = conn.prepare(
            "SELECT id, content_hash, created_at, ledger_cutoff_op_seq,
                    recurring_events_snapshot_json, income_sources_snapshot_json,
                    manual_assumption_events_json, scenario_overlay_ids_json
             FROM forecast_input_snapshots WHERE id = ?1",
        )?;
        let mut rows = stmt.query(params![id])?;
        match rows.next()? {
            Some(row) => Ok(Some(InputSnapshotView::from_row(row)?)),
            None => Ok(None),
        }
    }

    /// Register (idempotent upsert) a forecast model version. Re-registering the
    /// same `model_id` updates its version + parameters in place.
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a write failure.
    pub fn register_model(
        &self,
        model_id: &str,
        version: &str,
        parameters_json: Option<&str>,
    ) -> Result<(), DbError> {
        self.lock().conn.execute(
            "INSERT INTO model_registry (model_id, version, parameters_json, created_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(model_id) DO UPDATE SET version = excluded.version,
                 parameters_json = excluded.parameters_json",
            params![model_id, version, parameters_json, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    /// Read a registered model by id.
    ///
    /// # Errors
    /// [`DbError`] on a read failure.
    pub fn model(&self, model_id: &str) -> Result<Option<ModelRegistryView>, DbError> {
        let conn = self.read_connection()?;
        let mut stmt = conn.prepare(
            "SELECT model_id, version, parameters_json, created_at
             FROM model_registry WHERE model_id = ?1",
        )?;
        let mut rows = stmt.query(params![model_id])?;
        match rows.next()? {
            Some(row) => Ok(Some(ModelRegistryView::from_row(row)?)),
            None => Ok(None),
        }
    }

    /// Record a run-to-run forecast diff.
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a write failure.
    pub fn record_forecast_diff(&self, diff: &NewForecastDiff) -> Result<(), DbError> {
        self.lock().conn.execute(
            "INSERT INTO forecast_diffs
                (id, forecast_run_id, prior_run_id, summary_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                diff.id,
                diff.forecast_run_id,
                diff.prior_run_id,
                diff.summary_json,
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    /// The diffs recorded against a forecast run, oldest first.
    ///
    /// # Errors
    /// [`DbError`] on a read failure.
    pub fn forecast_diffs_for_run(&self, run_id: Uuid) -> Result<Vec<ForecastDiffView>, DbError> {
        let conn = self.read_connection()?;
        let mut stmt = conn.prepare(
            "SELECT id, forecast_run_id, prior_run_id, summary_json, created_at
             FROM forecast_diffs WHERE forecast_run_id = ?1 ORDER BY created_at, id",
        )?;
        let mut out = Vec::new();
        let mut rows = stmt.query(params![run_id])?;
        while let Some(row) = rows.next()? {
            out.push(ForecastDiffView::from_row(row)?);
        }
        Ok(out)
    }

    // ===== Scenarios (ADR 0026 §5, personal-cfo-0mg) =====
    // The named scenario definition + lifecycle. A scenario's overlay events are
    // scenario-scoped assumption events (5u2 `scenario_id`), not a separate store —
    // run a scenario via `active_assumption_events(Some(id))`. See [`scenarios`].

    /// Create a scenario (status `draft`, timestamps "now").
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a write failure.
    pub fn create_scenario(&self, scenario: &NewScenario) -> Result<(), DbError> {
        let now = Utc::now().to_rfc3339();
        self.lock().conn.execute(
            "INSERT INTO scenarios
                (id, name, description, status, base_run_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'draft', ?4, ?5, ?5)",
            params![
                scenario.id,
                scenario.name,
                scenario.description,
                scenario.base_run_id,
                now,
            ],
        )?;
        Ok(())
    }

    /// Read a scenario by id.
    ///
    /// # Errors
    /// [`DbError`] on a read failure or an unrecognized stored token.
    pub fn scenario(&self, id: Uuid) -> Result<Option<ScenarioView>, DbError> {
        let conn = self.read_connection()?;
        let mut stmt = conn.prepare(
            "SELECT s.id, s.name, s.description, s.status, s.base_run_id, s.created_at,
                    s.updated_at, s.expires_on,
                    (SELECT COUNT(*) FROM forecast_assumption_events e
                      WHERE e.scenario_id = s.id AND e.status = 'active'),
                    s.applied_at
             FROM scenarios s WHERE s.id = ?1",
        )?;
        let mut rows = stmt.query(params![id])?;
        match rows.next()? {
            Some(row) => Ok(Some(ScenarioView::from_row(row)?)),
            None => Ok(None),
        }
    }

    /// All scenarios, oldest first.
    ///
    /// # Errors
    /// [`DbError`] on a read failure or an unrecognized stored token.
    pub fn list_scenarios(&self) -> Result<Vec<ScenarioView>, DbError> {
        let conn = self.read_connection()?;
        let mut stmt = conn.prepare(
            "SELECT s.id, s.name, s.description, s.status, s.base_run_id, s.created_at,
                    s.updated_at, s.expires_on,
                    (SELECT COUNT(*) FROM forecast_assumption_events e
                      WHERE e.scenario_id = s.id AND e.status = 'active'),
                    s.applied_at
             FROM scenarios s ORDER BY s.created_at, s.id",
        )?;
        let mut out = Vec::new();
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            out.push(ScenarioView::from_row(row)?);
        }
        Ok(out)
    }

    /// Update a scenario's lifecycle status (draft → active → archived).
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a write failure.
    pub fn set_scenario_status(&self, id: Uuid, status: ScenarioStatus) -> Result<(), DbError> {
        self.lock().conn.execute(
            "UPDATE scenarios SET status = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, status.as_token(), Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    /// Rename a scenario (personal-cfo-vru6).
    ///
    /// # Errors
    /// [`DbError::InvalidCommand`] if the trimmed name is empty; [`DbError::Sqlite`] on a write
    /// failure.
    pub fn rename_scenario(&self, id: Uuid, name: &str) -> Result<(), DbError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(DbError::InvalidCommand(
                "a scenario needs a name".to_owned(),
            ));
        }
        self.lock().conn.execute(
            "UPDATE scenarios SET name = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, name, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    /// Spend rolled up by category over `[from, to]`, at the children of `parent`
    /// (`None` = the taxonomy roots), narrowed by the transaction list's facets —
    /// ADR 0052, personal-cfo-4d8.27.8.2 / -4d8.27.8.4.
    ///
    /// # Errors
    /// [`DbError`] on a read failure.
    pub fn spend_by_category(
        &self,
        from: NaiveDate,
        to: NaiveDate,
        parent: Option<Uuid>,
        currency: &str,
        filters: &SpendFilters,
    ) -> Result<SpendBreakdown, DbError> {
        spend_by_category::spend_by_category(
            &self.read_connection()?,
            from,
            to,
            parent,
            currency,
            filters,
        )
    }

    /// Archive a scenario, **keeping every event** (ADR 0051 §1).
    ///
    /// Archiving is a filing action, not a destructive one: the scenario drops out of
    /// the selector (so it can never reach a forecast run — a run only loads the
    /// selected scenario's events) but its overlay survives intact, so restoring
    /// returns it whole. This deliberately no longer clears events the way the old
    /// `delete_scenario` did; that clearing was compensating for the absence of a real
    /// delete, which now exists.
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a write failure.
    pub fn archive_scenario(&self, id: Uuid) -> Result<(), DbError> {
        self.set_scenario_status(id, ScenarioStatus::Archived)
    }

    /// Delete a scenario **and its overlay events**, permanently (ADR 0051 §1).
    ///
    /// Matches the house style for user-authored entities (recurring bills, income
    /// sources both hard-delete, with archive as the separate soft path). The cascade
    /// runs in one transaction so no event is ever left pointing at a scenario that no
    /// longer exists.
    ///
    /// Scoped to this scenario's own overlay: the `scenario_id = ?1` predicate cannot
    /// match a base event (`scenario_id IS NULL`), so a delete can never touch the
    /// user's real forecast.
    ///
    /// # Errors
    /// [`DbError::InvalidCommand`] if no such scenario exists; [`DbError::Sqlite`] on a
    /// write failure.
    pub fn delete_scenario(&self, id: Uuid) -> Result<(), DbError> {
        let mut guard = self.lock();
        let tx = guard.conn.transaction()?;
        // Edges first: they reference the events by `from_event_id` with no FK, so
        // removing events without them would strand per-row explanation edges pointing
        // at rows that no longer exist (they are read back by `explanation_edges`).
        tx.execute(
            "DELETE FROM forecast_dependency_edges
              WHERE from_event_id IN
                (SELECT id FROM forecast_assumption_events WHERE scenario_id = ?1)",
            params![id],
        )?;
        tx.execute(
            "DELETE FROM forecast_assumption_events WHERE scenario_id = ?1",
            params![id],
        )?;
        let removed = tx.execute("DELETE FROM scenarios WHERE id = ?1", params![id])?;
        if removed == 0 {
            return Err(DbError::InvalidCommand(
                "scenario does not exist".to_owned(),
            ));
        }
        tx.commit()?;
        Ok(())
    }

    /// Set (or clear, with `None`) a scenario's expiry date (ADR 0051 §3).
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a write failure.
    pub fn set_scenario_expiry(&self, id: Uuid, expires_on: Option<&str>) -> Result<(), DbError> {
        // Validate rather than storing arbitrary text: `expires_on` silently controls
        // whether the scenario applies, so a malformed or implausible date (a partially
        // typed year like `0002-12-31`) would quietly disable someone's plan.
        if let Some(date) = expires_on {
            let parsed = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|_| {
                DbError::InvalidCommand(format!("expiry must be YYYY-MM-DD: {date}"))
            })?;
            if parsed.year() < 1900 || parsed.year() > 9999 {
                return Err(DbError::InvalidCommand(format!(
                    "expiry year is out of range: {date}"
                )));
            }
        }
        self.lock().conn.execute(
            "UPDATE scenarios SET expires_on = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, expires_on, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    /// Clone a scenario into a new draft carrying a fresh copy of its active events
    /// (ADR 0051 §2), returning the new scenario's id.
    ///
    /// Copied: name (suffixed), description, and every ACTIVE event with a fresh id.
    /// Deliberately not copied: status (a copy of an active plan is not itself active),
    /// `base_run_id` (the clone forks from the current base, not the source's
    /// historical run), expiry, and cleared/superseded events (undo history, not plan).
    ///
    /// # Errors
    /// [`DbError::InvalidCommand`] if no such scenario exists; [`DbError::Sqlite`] on a
    /// write failure.
    pub fn clone_scenario(&self, id: Uuid, new_id: Uuid, name: &str) -> Result<(), DbError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(DbError::InvalidCommand(
                "a scenario needs a name".to_owned(),
            ));
        }
        let now = Utc::now().to_rfc3339();
        let mut guard = self.lock();
        let tx = guard.conn.transaction()?;
        let copied = tx.execute(
            "INSERT INTO scenarios (id, name, description, status, base_run_id, created_at, updated_at)
             SELECT ?2, ?3, description, 'draft', NULL, ?4, ?4 FROM scenarios WHERE id = ?1",
            params![id, new_id, name, now],
        )?;
        if copied == 0 {
            return Err(DbError::InvalidCommand(
                "scenario does not exist".to_owned(),
            ));
        }
        // Each copied event needs its OWN id; SQLite has no UUID generator, so the ids
        // are minted here and the rows inserted one by one.
        let sources: Vec<Uuid> = {
            let mut stmt = tx.prepare(
                "SELECT id FROM forecast_assumption_events
                  WHERE scenario_id = ?1 AND status = 'active' ORDER BY created_at, id",
            )?;
            let mut rows = stmt.query(params![id])?;
            let mut out = Vec::new();
            while let Some(row) = rows.next()? {
                out.push(row.get(0)?);
            }
            out
        };
        for source in sources {
            tx.execute(
                // Every NOT NULL column must be carried: `source` and `kind` are
                // CHECK-constrained and non-null, so copying them from the source row
                // (rather than defaulting) keeps the clone a faithful overlay. The
                // clone is a fresh plan, so `superseded_by`/`origin_run_id` are not
                // copied — they describe the SOURCE's undo and run history.
                // `created_at` is COPIED, not restamped: `entity_overrides` orders base
                // and scenario events together by (created_at, id) and lets the later one
                // win, so restamping would change how the copy resolves against a base
                // override created in between — two supposedly-identical scenarios would
                // project different amounts. The clone timestamp lives in `updated_at`.
                "INSERT INTO forecast_assumption_events
                    (id, kind, target_entity_type, target_entity_id, params_json,
                     source, scenario_id, status, created_at, updated_at)
                 SELECT ?2, kind, target_entity_type, target_entity_id, params_json,
                        source, ?3, 'active', created_at, ?4
                   FROM forecast_assumption_events WHERE id = ?1",
                params![source, Uuid::now_v7(), new_id, now],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    // ===== Manual future entries (ADR 0026, personal-cfo-q6gh) =====
    // User-added one-time cash events, stored as `5u2` assumption events
    // (kind = one_time_event, source = user_override). The forecast folds the
    // active base entries in; edit = supersede, delete = clear (never silent
    // mutation). See [`manual_entry`].

    /// Record a manual future entry — a one-time signed cash event on a date
    /// (inflow +, outflow −).
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a write failure.
    pub fn record_manual_entry(
        &self,
        id: Uuid,
        amount: Money,
        occurs_on: NaiveDate,
        label: &str,
        account_id: Option<Uuid>,
    ) -> Result<(), DbError> {
        self.record_assumption_event(&NewAssumptionEvent {
            id,
            kind: AssumptionKind::OneTimeEvent,
            target_entity_type: None,
            target_entity_id: None,
            params_json: manual_entry::build_params_json(amount, occurs_on, label, account_id),
            source: AssumptionSource::UserOverride,
            scenario_id: None,
            origin_run_id: None,
        })
    }

    /// The active base manual future entries, oldest first.
    ///
    /// # Errors
    /// [`DbError`] on a read failure or malformed stored params.
    pub fn manual_entries(&self) -> Result<Vec<ManualEntry>, DbError> {
        manual_entry::active_manual_entries(&self.read_connection()?, &[])
    }

    /// Whether the materialized read models are coherent with the canonical
    /// tables: the stored authoritative checksum matches a freshly computed one.
    ///
    /// Returns `true` when they match, or when no checksum has been recorded yet
    /// (the read model has not been materialized; it rebuilds deterministically
    /// on demand). A `false` means drift was detected — recoverable by a rebuild,
    /// not corruption.
    ///
    /// # Errors
    /// Returns [`DbError`] if the read fails.
    pub fn read_models_current(&self) -> Result<bool, DbError> {
        let conn = self.read_connection()?;
        let stored: Option<i64> = conn
            .query_row(
                "SELECT current_checksum FROM read_model_checksums \
                 WHERE read_model_name = 'transaction_display'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        match stored {
            None => Ok(true),
            Some(stored) => Ok(stored == projection::checksum(&conn)? as i64),
        }
    }

    /// Dispatch a write command. The mutation and its op-log entry commit
    /// atomically.
    ///
    /// # Errors
    /// Returns [`DbError`] if metadata is missing, the worker is unavailable,
    /// the write panics, or a SQLite error occurs.
    pub fn dispatch(&self, meta: CommandMeta, cmd: WriteCommand) -> Result<Outcome, DbError> {
        self.dispatch_inner(meta, cmd, Fault::None)
    }

    /// Reap idempotency keys whose `expires_at` is strictly before `cutoff`
    /// (RFC 3339). Returns the number removed. Safe to run anytime: it only
    /// removes the memoization, never op-log or domain rows.
    ///
    /// # Errors
    /// Returns [`DbError`] if the delete fails.
    pub fn gc_idempotency_keys_before(&self, cutoff: &str) -> Result<u64, DbError> {
        let guard = self.lock();
        let removed = guard.conn.execute(
            "DELETE FROM command_idempotency_keys WHERE expires_at < ?1",
            [cutoff],
        )?;
        Ok(u64::try_from(removed).unwrap_or(0))
    }

    /// Number of live idempotency keys.
    ///
    /// # Errors
    /// Returns [`DbError`] if the query fails.
    pub fn idempotency_key_count(&self) -> Result<u64, DbError> {
        let guard = self.lock();
        let n: i64 =
            guard
                .conn
                .query_row("SELECT COUNT(*) FROM command_idempotency_keys", [], |r| {
                    r.get(0)
                })?;
        Ok(u64::try_from(n).unwrap_or(0))
    }

    fn dispatch_inner(
        &self,
        meta: CommandMeta,
        cmd: WriteCommand,
        fault: Fault,
    ) -> Result<Outcome, DbError> {
        meta.validate()?;
        let node_id = self.node_id;
        let schema_version = self.schema_version;
        let mut guard = self.lock();
        if guard.state != WorkerState::Healthy {
            return Err(DbError::WorkerUnavailable(guard.state));
        }

        // Idempotent replay: an existing idempotency key returns the stored
        // result and writes nothing new (plan §9.1.2; bead personal-cfo-02i).
        if let Some(op_seq) = find_by_idempotency_key(&guard.conn, &meta.idempotency_key)? {
            return Ok(Outcome::Replayed { op_seq });
        }

        let result = catch_unwind(AssertUnwindSafe(|| {
            apply_command(&mut guard, &meta, &cmd, node_id, schema_version, fault)
        }));

        match result {
            Ok(outcome) => outcome,
            Err(_panic) => {
                // The transaction was dropped during unwind -> rolled back.
                // Mark the worker for recovery instead of risking silent damage.
                guard.state = WorkerState::CorruptNeedsRecovery;
                Err(DbError::WriterPanicked)
            }
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        // Recover the guard even if a previous holder panicked; the worker's
        // own `state` field tracks health, so a poisoned mutex is not fatal.
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Typed read-only account queries (plan §2.6 read views).
pub trait AccountQuery {
    /// Total number of accounts.
    ///
    /// # Errors
    /// Returns a SQLite error if the query fails.
    fn count_accounts(&self) -> Result<u64, DbError>;

    /// Whether an account with `id` exists.
    ///
    /// # Errors
    /// Returns a SQLite error if the query fails.
    fn account_exists(&self, id: AccountId) -> Result<bool, DbError>;
}

impl AccountQuery for Connection {
    fn count_accounts(&self) -> Result<u64, DbError> {
        let n: i64 = self.query_row("SELECT COUNT(*) FROM accounts", [], |r| r.get(0))?;
        Ok(u64::try_from(n).unwrap_or(0))
    }

    fn account_exists(&self, id: AccountId) -> Result<bool, DbError> {
        let n: i64 = self.query_row(
            "SELECT COUNT(*) FROM accounts WHERE id = ?1",
            [id.as_uuid()],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }
}

/// Typed read-only operation-log queries.
pub trait LedgerQuery {
    /// Number of entries in the operation log.
    ///
    /// # Errors
    /// Returns a SQLite error if the query fails.
    fn operation_count(&self) -> Result<u64, DbError>;
}

impl LedgerQuery for Connection {
    fn operation_count(&self) -> Result<u64, DbError> {
        let n: i64 = self.query_row("SELECT COUNT(*) FROM operation_log", [], |r| r.get(0))?;
        Ok(u64::try_from(n).unwrap_or(0))
    }
}

/// Open a SQLCipher connection keyed with a **passphrase** (SQLCipher runs its
/// own KDF over it). Used by [`DbWorker::open`].
fn open_keyed(path: &Path, key: &str) -> Result<Connection, DbError> {
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "key", key)?;
    configure_conn(&conn)?;
    Ok(conn)
}

/// Open a SQLCipher connection keyed with a **raw** 256-bit key (no SQLCipher
/// KDF — the DEK is used verbatim). Used by [`DbWorker::open_with_raw_key`].
///
/// SQLCipher recognizes a raw key only from the literal `x'<64 hex>'` PRAGMA
/// text, so this must be issued via `execute_batch` rather than a bound
/// `pragma_update` (which would quote the value and be treated as a passphrase).
/// The PRAGMA string holds the key in hex and is zeroized immediately after use.
fn open_keyed_raw(path: &Path, dek: &Dek) -> Result<Connection, DbError> {
    use std::fmt::Write as _;

    let conn = Connection::open(path)?;
    let mut pragma = String::with_capacity(80);
    pragma.push_str("PRAGMA key = \"x'");
    for byte in dek.expose_bytes() {
        // Infallible write into a String.
        let _ = write!(pragma, "{byte:02x}");
    }
    pragma.push_str("'\";");
    let result = conn.execute_batch(&pragma);
    pragma.zeroize();
    result?;
    configure_conn(&conn)?;
    Ok(conn)
}

/// Apply the non-key connection pragmas shared by both keying paths (WAL,
/// busy-timeout, WAL size cap — plan §3.2).
fn configure_conn(conn: &Connection) -> Result<(), DbError> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    conn.pragma_update(None, "journal_size_limit", JOURNAL_SIZE_LIMIT)?;
    // Keep transient query data (sorts, rebuilds, temp B-trees) in RAM so no temp
    // file is ever written beside the vault (personal-cfo-zxvl, §6.4 WAL/SHM/temp
    // policy). SQLCipher also encrypts any temp file it does create; MEMORY is
    // belt-and-suspenders against a plaintext temp leak.
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    Ok(())
}

/// The bundled SQLite version string (`rusqlite::version()`), e.g. `"3.45.3"`.
/// Pinned in `docs/architecture/stack.md` and asserted by the cross-version vault
/// test (personal-cfo-7igv); see also [`DbWorker::cipher_version`].
#[must_use]
pub fn sqlite_version() -> &'static str {
    rusqlite::version()
}

/// The baseline schema (migration `0001`, personal-cfo-wkn). Defined as a `&str`
/// const consumed by [`migrations::MIGRATIONS`]; `CREATE TABLE IF NOT EXISTS`
/// keeps it a safe no-op on a pre-framework vault that already has these tables.
///
/// operation_log (ADR 0011, personal-cfo-0s0): immutable append-only audit
/// trail. Provenance UUIDs are 16-byte BLOBs; idempotency_key stays TEXT (an
/// arbitrary caller string, not a UUID) and hlc_timestamp stays INTEGER
/// (single-device HLC is a counter; BLOB encoding is deferred).
/// affected_entities is a JSON list of {table, id} stored as a BLOB; the
/// normalized per-entity index lives in provenance_links (both coexist per
/// ADR 0011). UPDATE/DELETE are blocked by the triggers below.
pub(crate) const BASELINE_UP: &str = "CREATE TABLE IF NOT EXISTS operation_log (
            op_seq               INTEGER PRIMARY KEY AUTOINCREMENT,
            command_id           BLOB NOT NULL UNIQUE,
            correlation_id       BLOB NOT NULL,
            causation_id         BLOB,
            actor_type           TEXT NOT NULL,
            actor_id             TEXT NOT NULL,
            idempotency_key      TEXT NOT NULL,
            operation_type       TEXT NOT NULL,
            affected_entities    BLOB NOT NULL,
            metadata             BLOB,
            node_id              BLOB NOT NULL,
            vault_schema_version INTEGER NOT NULL,
            hlc_timestamp        INTEGER NOT NULL,
            created_at           TEXT NOT NULL
        );
        -- The operation log is immutable history (ADR 0011): only INSERT is
        -- permitted. These triggers make UPDATE/DELETE hard errors.
        CREATE TRIGGER IF NOT EXISTS operation_log_no_update
        BEFORE UPDATE ON operation_log
        BEGIN
            SELECT RAISE(ABORT, 'operation_log is append-only');
        END;
        CREATE TRIGGER IF NOT EXISTS operation_log_no_delete
        BEFORE DELETE ON operation_log
        BEGIN
            SELECT RAISE(ABORT, 'operation_log is append-only');
        END;
        CREATE TABLE IF NOT EXISTS ledger_accounts (
            id             BLOB PRIMARY KEY,
            kind           TEXT NOT NULL,
            normal_balance TEXT NOT NULL,
            currency       TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS accounts (
            id                BLOB PRIMARY KEY,
            ledger_account_id BLOB NOT NULL,
            name              TEXT NOT NULL,
            cashflow_role     TEXT NOT NULL,
            normal_balance    TEXT NOT NULL,
            currency          TEXT NOT NULL,
            retirement        INTEGER NOT NULL DEFAULT 0,
            tax_advantaged    INTEGER NOT NULL DEFAULT 0,
            joint             INTEGER NOT NULL DEFAULT 0,
            business          INTEGER NOT NULL DEFAULT 0,
            active            INTEGER NOT NULL DEFAULT 1,
            last_synced       TEXT,
            manual_balance_at TEXT
        );
        CREATE TABLE IF NOT EXISTS system_ledger_accounts (
            role              TEXT NOT NULL,
            currency          TEXT NOT NULL,
            ledger_account_id BLOB NOT NULL,
            PRIMARY KEY (role, currency)
        );
        CREATE TABLE IF NOT EXISTS ledger_transactions (
            id           BLOB PRIMARY KEY,
            -- operation_id is the operation provenance back-reference; it joins
            -- operation_log.command_id, which is a BLOB (ADR 0011, 0s0).
            operation_id BLOB NOT NULL,
            occurred_at  TEXT NOT NULL,
            currency     TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS ledger_postings (
            transaction_id    BLOB NOT NULL,
            ledger_account_id BLOB NOT NULL,
            minor_units       INTEGER NOT NULL,
            currency          TEXT NOT NULL,
            currency_exponent INTEGER NOT NULL,
            posting_date      TEXT NOT NULL
        );
        -- Balance invariant (ADR 0007): per transaction, signed minor_units sum
        -- to zero (debit positive, credit negative). Enforced DB-side at insert
        -- time. The header is inserted last (see persist_transaction), so this
        -- fires once all postings are present.
        CREATE TRIGGER IF NOT EXISTS ledger_transaction_balances
        AFTER INSERT ON ledger_transactions
        BEGIN
            SELECT CASE
                WHEN (SELECT COALESCE(SUM(minor_units), 0)
                      FROM ledger_postings WHERE transaction_id = NEW.id) != 0
                THEN RAISE(ABORT, 'ledger transaction postings do not balance to zero')
            END;
        END;
        -- Defend the invariant after finalization: a posting added once the
        -- header exists must keep the transaction balanced.
        CREATE TRIGGER IF NOT EXISTS ledger_posting_keeps_balance
        AFTER INSERT ON ledger_postings
        WHEN EXISTS (SELECT 1 FROM ledger_transactions WHERE id = NEW.transaction_id)
        BEGIN
            SELECT CASE
                WHEN (SELECT COALESCE(SUM(minor_units), 0)
                      FROM ledger_postings WHERE transaction_id = NEW.transaction_id) != 0
                THEN RAISE(ABORT, 'posting would unbalance a finalized ledger transaction')
            END;
        END;
        CREATE INDEX IF NOT EXISTS idx_ledger_postings_account_date
            ON ledger_postings (ledger_account_id, posting_date);
        CREATE INDEX IF NOT EXISTS idx_ledger_postings_transaction
            ON ledger_postings (transaction_id);
        CREATE INDEX IF NOT EXISTS idx_ledger_transactions_date
            ON ledger_transactions (occurred_at);
        CREATE TABLE IF NOT EXISTS provenance_links (
            op_seq      INTEGER NOT NULL,
            entity_type TEXT NOT NULL,
            entity_id   BLOB NOT NULL
        );
        CREATE TABLE IF NOT EXISTS command_idempotency_keys (
            idempotency_key TEXT PRIMARY KEY,
            command_id      BLOB NOT NULL,
            command_type    TEXT NOT NULL,
            result_ref_json TEXT NOT NULL,
            op_seq          INTEGER NOT NULL,
            expires_at      TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS vault_meta (
            node_id BLOB NOT NULL
        );
        -- Vault-level identity + configuration (plan §9.2, personal-cfo-2lm).
        -- Exactly one row, enforced by the singleton CHECK. KDF columns hold the
        -- Argon2id parameters; values seeded here are placeholders that the
        -- vault module (personal-cfo-vhv) overwrites at real vault creation and
        -- personal-cfo-0sqk calibrates.
        CREATE TABLE IF NOT EXISTS vault_metadata (
            singleton          INTEGER PRIMARY KEY CHECK (singleton = 1),
            vault_id           BLOB    NOT NULL,
            schema_version     INTEGER NOT NULL,
            envelope_version   INTEGER NOT NULL,
            kdf_algorithm      TEXT    NOT NULL,
            kdf_memory_kib     INTEGER NOT NULL,
            kdf_time_cost      INTEGER NOT NULL,
            kdf_parallelism    INTEGER NOT NULL,
            household_timezone TEXT    NOT NULL,
            manifest_pointer   TEXT,
            created_at         TEXT    NOT NULL
        );
        CREATE TABLE IF NOT EXISTS transaction_display_rows_read_model (
            transaction_id          BLOB NOT NULL,
            account_id              BLOB NOT NULL,
            occurred_at             TEXT NOT NULL,
            minor_units             INTEGER NOT NULL,
            currency                TEXT NOT NULL,
            merchant_identity_id    TEXT,
            primary_category_id     TEXT,
            category_confidence_bps INTEGER NOT NULL DEFAULT 0,
            category_source         TEXT NOT NULL,
            recurring_event_id      TEXT,
            review_status           TEXT NOT NULL,
            last_projected_op_seq   INTEGER NOT NULL,
            PRIMARY KEY (transaction_id, account_id)
        );
        CREATE TABLE IF NOT EXISTS projection_cursors (
            read_model_name     TEXT PRIMARY KEY,
            last_applied_op_seq INTEGER NOT NULL,
            last_rebuild_at     TEXT,
            content_checksum    INTEGER
        );
        -- Drift detection: the authoritative content checksum per read model
        -- (personal-cfo-0s0). Compared against a freshly computed checksum to
        -- detect divergence between the materialized projection and canonical
        -- state (reconciliation is personal-cfo-xn7).
        CREATE TABLE IF NOT EXISTS read_model_checksums (
            read_model_name  TEXT PRIMARY KEY,
            current_checksum INTEGER NOT NULL,
            computed_at      TEXT NOT NULL
        );
        -- Category taxonomy (plan §9.6, personal-cfo-d3p). Hierarchical via
        -- parent_id; type + forecast_behavior are constrained token sets. The
        -- default taxonomy is seeded at vault create (ensure_default_categories).
        CREATE TABLE IF NOT EXISTS categories (
            id                   BLOB PRIMARY KEY,
            household_id         BLOB,
            parent_id            BLOB,
            name                 TEXT NOT NULL,
            type                 TEXT NOT NULL
                CHECK (type IN ('income', 'expense', 'transfer', 'adjustment')),
            icon                 TEXT,
            color                TEXT,
            is_system            INTEGER NOT NULL DEFAULT 0,
            budget_default_minor INTEGER,
            forecast_behavior    TEXT NOT NULL
                CHECK (forecast_behavior IN (
                    'deterministic', 'variable_regular', 'variable_lumpy',
                    'ignore_cashflow', 'income'
                )),
            created_at           TEXT NOT NULL,
            updated_at           TEXT NOT NULL,
            -- A category cannot be its own parent (1-cycle). Deeper re-parenting
            -- cycles are blocked by the trigger below.
            CHECK (parent_id IS NULL OR parent_id <> id)
        );
        CREATE INDEX IF NOT EXISTS idx_categories_parent ON categories (parent_id);
        -- Prevent re-parenting cycles: if NEW.parent_id's ancestor chain reaches
        -- NEW.id, abort. (INSERT cannot form a multi-level cycle — a new id is
        -- not yet referenced — so only UPDATE OF parent_id needs guarding.)
        CREATE TRIGGER IF NOT EXISTS categories_no_parent_cycle
        BEFORE UPDATE OF parent_id ON categories
        WHEN NEW.parent_id IS NOT NULL
        BEGIN
            SELECT CASE WHEN (
                WITH RECURSIVE ancestors(id) AS (
                    SELECT NEW.parent_id
                    UNION ALL
                    SELECT c.parent_id FROM categories c
                    JOIN ancestors a ON c.id = a.id
                    WHERE c.parent_id IS NOT NULL
                )
                SELECT COUNT(*) FROM ancestors WHERE id = NEW.id
            ) > 0 THEN RAISE(ABORT, 'category parent cycle') END;
        END;
        -- Alternative names that map to a category (import/merchant matching).
        CREATE TABLE IF NOT EXISTS category_aliases (
            id          BLOB PRIMARY KEY,
            category_id BLOB NOT NULL,
            alias       TEXT NOT NULL UNIQUE,
            created_at  TEXT NOT NULL
        );
        -- Forecast persistence (plan §9.10, personal-cfo-63t). A forecast_runs
        -- header records what makes the run reproducible (input_snapshot_id,
        -- assumptions_hash, random_seed, deterministic_run); forecast_rows hold
        -- the per-date P10/P50/P90 projection + running balances; input snapshots
        -- capture the inputs a run was computed against. These tables are written
        -- by the forecast engine (personal-cfo-164u), not seeded at vault create.
        CREATE TABLE IF NOT EXISTS forecast_input_snapshots (
            id                          BLOB PRIMARY KEY,
            household_id                BLOB,
            created_at                  TEXT NOT NULL,
            ledger_cutoff_at            TEXT NOT NULL,
            included_entity_hashes_json TEXT NOT NULL,
            source_freshness_json       TEXT,
            schema_version              INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS forecast_runs (
            id                        BLOB PRIMARY KEY,
            household_id              BLOB,
            generated_at              TEXT NOT NULL,
            horizon_days              INTEGER NOT NULL,
            starting_cash_minor       INTEGER NOT NULL,
            model_version             TEXT,
            model_registry_json       TEXT,
            input_snapshot_id         BLOB,
            assumptions_hash          TEXT,
            assumptions_version       INTEGER,
            scenario_overlay_ids_json TEXT,
            random_seed               INTEGER,
            deterministic_run         INTEGER NOT NULL DEFAULT 1,
            p10_min_cash_minor        INTEGER,
            p50_min_cash_minor        INTEGER,
            p90_min_cash_minor        INTEGER,
            summary_json              TEXT
        );
        CREATE TABLE IF NOT EXISTS forecast_rows (
            id                        BLOB PRIMARY KEY,
            forecast_run_id           BLOB NOT NULL,
            date                      TEXT NOT NULL,
            description               TEXT,
            amount_p10_minor          INTEGER NOT NULL,
            amount_p50_minor          INTEGER NOT NULL,
            amount_p90_minor          INTEGER NOT NULL,
            running_balance_p10_minor INTEGER NOT NULL,
            running_balance_p50_minor INTEGER NOT NULL,
            running_balance_p90_minor INTEGER NOT NULL,
            source_type               TEXT NOT NULL CHECK (source_type IN (
                'starting_balance', 'income', 'recurring_bill', 'variable_spend',
                'credit_card_payment', 'transfer', 'scenario_event',
                'manual_entry', 'loan_payment', 'investment_cashflow'
            )),
            source_id                 BLOB,
            confidence_bps            INTEGER NOT NULL DEFAULT 0,
            explanation_json          TEXT,
            is_user_adjusted          INTEGER NOT NULL DEFAULT 0,
            computation_mode          TEXT NOT NULL CHECK (computation_mode IN (
                'deterministic_incremental', 'full_batch'
            )),
            stale_since               TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_forecast_rows_run_date
            ON forecast_rows (forecast_run_id, date);";

/// Build the op-log `affected_entities` JSON (a list of `{table, id}` pairs)
/// for a single affected entity, as the BLOB bytes stored on the row. Inputs
/// are a static table name and a UUID, so manual JSON is safe (no escaping) and
/// keeps `serde_json` out of the write path.
fn affected_entities_json(table: &str, id: Uuid) -> Vec<u8> {
    format!("[{{\"table\":\"{table}\",\"id\":\"{id}\"}}]").into_bytes()
}

fn ensure_node_id(conn: &Connection) -> Result<Uuid, DbError> {
    let existing: Option<Uuid> = conn
        .query_row("SELECT node_id FROM vault_meta LIMIT 1", [], |r| r.get(0))
        .optional()?;
    if let Some(node_id) = existing {
        return Ok(node_id);
    }
    let node_id = Uuid::now_v7();
    conn.execute("INSERT INTO vault_meta (node_id) VALUES (?1)", [node_id])?;
    Ok(node_id)
}

/// Default Argon2id `interactive_default` parameters seeded into a fresh vault.
///
/// These are **placeholders**: the vault module (`personal-cfo-vhv`) writes the
/// real parameters at vault creation, and `personal-cfo-0sqk` calibrates the
/// per-device-class profile and enforces a CI floor. They exist so a freshly
/// opened vault always has a complete, readable `vault_metadata` row.
const DEFAULT_KDF_ALGORITHM: &str = "argon2id";
const DEFAULT_KDF_MEMORY_KIB: i64 = 65_536;
const DEFAULT_KDF_TIME_COST: i64 = 3;
const DEFAULT_KDF_PARALLELISM: i64 = 1;

/// Seed the singleton `vault_metadata` row if it is absent (mirrors
/// [`ensure_node_id`]), and keep its `schema_version` stamp in step with the
/// migrated schema. Migrations advance `PRAGMA user_version` + the
/// `schema_migrations` tracker but never re-stamp this row, so without this
/// update a vault created under an older build reports schema-incoherent on
/// every open even after migrating cleanly (personal-cfo-4d8.27.1.3).
/// Idempotent across re-opens.
fn ensure_vault_metadata(conn: &Connection, schema_version: i64) -> Result<(), DbError> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM vault_metadata WHERE singleton = 1)",
        [],
        |r| r.get(0),
    )?;
    if exists {
        // Advance the write-once stamp to the version we just migrated to. Only
        // move it FORWARD: if the stored stamp is already higher (a vault created
        // by a newer build, opened here by an older one), leave it so the health
        // check still surfaces the genuine "opened by a newer version" mismatch
        // instead of masking it.
        conn.execute(
            "UPDATE vault_metadata SET schema_version = ?1
             WHERE singleton = 1 AND schema_version < ?1",
            params![schema_version],
        )?;
        return Ok(());
    }
    conn.execute(
        "INSERT INTO vault_metadata (
            singleton, vault_id, schema_version, envelope_version,
            kdf_algorithm, kdf_memory_kib, kdf_time_cost, kdf_parallelism,
            household_timezone, manifest_pointer, created_at
        ) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, ?9)",
        params![
            Uuid::now_v7(),
            schema_version,
            VAULT_ENVELOPE_VERSION,
            DEFAULT_KDF_ALGORITHM,
            DEFAULT_KDF_MEMORY_KIB,
            DEFAULT_KDF_TIME_COST,
            DEFAULT_KDF_PARALLELISM,
            DEFAULT_HOUSEHOLD_TIMEZONE,
            Utc::now().to_rfc3339(),
        ],
    )?;
    Ok(())
}

/// The on-disk vault crypto envelope version (plan §6.2; ADR 0002). v1 is the
/// initial layout; `personal-cfo-vhv` bumps it when the envelope format changes.
const VAULT_ENVELOPE_VERSION: i64 = 1;

/// Default household timezone for a fresh vault. `personal-cfo-rr0` (date/tz
/// policy) reads and, with the onboarding flow, sets this.
const DEFAULT_HOUSEHOLD_TIMEZONE: &str = "UTC";

fn current_max_hlc(conn: &Connection) -> Result<i64, DbError> {
    let max: i64 = conn.query_row(
        "SELECT COALESCE(MAX(hlc_timestamp), 0) FROM operation_log",
        [],
        |r| r.get(0),
    )?;
    Ok(max)
}

fn find_by_idempotency_key(conn: &Connection, key: &str) -> Result<Option<i64>, DbError> {
    let op_seq = conn
        .query_row(
            "SELECT op_seq FROM command_idempotency_keys WHERE idempotency_key = ?1",
            [key],
            |r| r.get(0),
        )
        .optional()?;
    Ok(op_seq)
}

fn currency_from_code(code: &str) -> Result<Currency, DbError> {
    match code {
        "USD" => Ok(Currency::Usd),
        "EUR" => Ok(Currency::Eur),
        other => Err(DbError::InvalidCommand(format!("unknown currency {other}"))),
    }
}

/// Auto-create the system ledger accounts at vault init (plan §9.3): an
/// opening-balance equity account and catch-alls for unmatched income/expense.
fn ensure_system_accounts(conn: &Connection) -> Result<(), DbError> {
    ensure_system_ledger_account(
        conn,
        "opening_balance_equity",
        Currency::Usd,
        AccountKind::Equity,
    )?;
    ensure_system_ledger_account(conn, "unmatched_income", Currency::Usd, AccountKind::Income)?;
    ensure_system_ledger_account(
        conn,
        "unmatched_expense",
        Currency::Usd,
        AccountKind::Expense,
    )?;
    Ok(())
}

/// A top-level group in the default category taxonomy: its accounting `type`,
/// its own `forecast_behavior`, and its leaf categories (each with a forecast
/// behavior; leaves inherit the group's `type`).
struct CategoryGroup {
    name: &'static str,
    category_type: &'static str,
    forecast_behavior: &'static str,
    leaves: &'static [(&'static str, &'static str)],
}

/// The default, user-editable category taxonomy seeded into a fresh vault
/// (plan §9.6). The per-category `forecast_behavior` is a heuristic default:
/// fixed recurring bills are `deterministic`, regular variable spend is
/// `variable_regular`, irregular spend is `variable_lumpy`, transfers are
/// `ignore_cashflow`, and income is `income`. Users re-classify later (bac).
const DEFAULT_TAXONOMY: &[CategoryGroup] = &[
    CategoryGroup {
        name: "Income",
        category_type: "income",
        forecast_behavior: "income",
        leaves: &[
            ("Salary", "income"),
            ("Hourly Wages", "income"),
            ("Contractor/Freelance", "income"),
            ("Bonus/Commission", "income"),
            ("Rental Income", "income"),
            ("Investment Income", "income"),
            ("Benefits", "income"),
        ],
    },
    CategoryGroup {
        name: "Housing",
        category_type: "expense",
        forecast_behavior: "variable_regular",
        leaves: &[
            ("Rent/Mortgage", "deterministic"),
            ("Property Tax", "variable_lumpy"),
            ("HOA", "deterministic"),
            ("Utilities", "variable_regular"),
            ("Internet/Phone", "deterministic"),
            ("Insurance", "deterministic"),
            ("Maintenance", "variable_lumpy"),
        ],
    },
    CategoryGroup {
        name: "Food and Drink",
        category_type: "expense",
        forecast_behavior: "variable_regular",
        leaves: &[
            ("Groceries", "variable_regular"),
            ("Restaurants", "variable_regular"),
            ("Coffee", "variable_regular"),
            ("Alcohol/Bars", "variable_lumpy"),
        ],
    },
    CategoryGroup {
        name: "Transportation",
        category_type: "expense",
        forecast_behavior: "variable_regular",
        leaves: &[
            ("Gas", "variable_regular"),
            ("Auto Payment", "deterministic"),
            ("Auto Insurance", "deterministic"),
            ("Maintenance", "variable_lumpy"),
            ("Parking/Tolls", "variable_lumpy"),
            ("Public Transit", "variable_regular"),
            ("Rideshare", "variable_lumpy"),
        ],
    },
    CategoryGroup {
        name: "Debt",
        category_type: "expense",
        forecast_behavior: "deterministic",
        // "Credit Card Payment" is intentionally NOT here — a card payment is money
        // movement (checking -> card), not spend, so it lives under "Transfers"
        // (ADR 0030 addendum 2026-07-11, personal-cfo-4d8.25.21).
        leaves: &[
            ("Student Loan", "deterministic"),
            ("Auto Loan", "deterministic"),
            ("Personal Loan", "deterministic"),
        ],
    },
    CategoryGroup {
        name: "Health",
        category_type: "expense",
        forecast_behavior: "variable_regular",
        leaves: &[
            ("Insurance Premiums", "deterministic"),
            ("Medical", "variable_lumpy"),
            ("Dental", "variable_lumpy"),
            ("Pharmacy", "variable_regular"),
        ],
    },
    CategoryGroup {
        name: "Family",
        category_type: "expense",
        forecast_behavior: "variable_regular",
        leaves: &[
            ("Childcare", "variable_regular"),
            ("Education", "variable_lumpy"),
            ("Pet Care", "variable_lumpy"),
        ],
    },
    CategoryGroup {
        name: "Lifestyle",
        category_type: "expense",
        forecast_behavior: "variable_lumpy",
        leaves: &[
            ("Shopping", "variable_lumpy"),
            ("Travel", "variable_lumpy"),
            ("Entertainment", "variable_lumpy"),
            ("Subscriptions", "deterministic"),
            ("Fitness", "variable_regular"),
            ("Gifts/Donations", "variable_lumpy"),
        ],
    },
    CategoryGroup {
        name: "Savings and Investments",
        category_type: "transfer",
        forecast_behavior: "ignore_cashflow",
        leaves: &[
            ("Emergency Fund", "variable_regular"),
            ("Brokerage Transfer", "variable_lumpy"),
            ("Retirement Contribution", "deterministic"),
            ("HSA Contribution", "deterministic"),
        ],
    },
    CategoryGroup {
        name: "Transfers",
        category_type: "transfer",
        forecast_behavior: "ignore_cashflow",
        leaves: &[
            ("Internal Transfer", "ignore_cashflow"),
            // A credit-card payment moves cash from a liquid account to the card
            // (ADR 0030 addendum 2026-07-11) — a transfer, excluded from spend totals
            // and recurring-bill detection; the card's statement forecast models it.
            ("Credit Card Payment", "ignore_cashflow"),
            ("Reimbursement", "ignore_cashflow"),
            ("Cash Withdrawal", "ignore_cashflow"),
        ],
    },
    CategoryGroup {
        name: "Taxes",
        category_type: "expense",
        forecast_behavior: "variable_regular",
        leaves: &[
            ("Federal Tax", "variable_regular"),
            ("State Tax", "variable_regular"),
            ("Local Tax", "variable_regular"),
            ("Quarterly Estimated Tax", "variable_lumpy"),
        ],
    },
];

/// Seed the default category taxonomy (plan §9.6, personal-cfo-d3p) into a fresh
/// vault. Idempotent: if any system category already exists, do nothing. Mirrors
/// [`ensure_system_accounts`]. Each row is `is_system = 1`; leaves point at their
/// group via `parent_id`.
/// The default forecast behavior for a new category of `category_type` (ADR 0030):
/// income tracks as income; transfers/adjustments are cashflow-neutral; everything
/// else (expenses) varies regularly. A user can refine it later.
fn default_forecast_behavior(category_type: &str) -> &'static str {
    match category_type {
        "income" => "income",
        "transfer" | "adjustment" => "ignore_cashflow",
        _ => "variable_regular",
    }
}

/// Require that `id` names an existing **user** category, for identity-changing
/// commands (re-parent). A system category's identity is fixed — only its appearance
/// (color + icon) is editable (ADR 0030 amendment, kogu) — so a system row, or a
/// missing one, is rejected with a clean [`DbError::InvalidCommand`].
fn require_user_category(tx: &Connection, id: CategoryId) -> Result<(), DbError> {
    let is_system: Option<bool> = tx
        .query_row(
            "SELECT is_system FROM categories WHERE id = ?1",
            [id.as_uuid()],
            |r| r.get::<_, i64>(0).map(|v| v != 0),
        )
        .optional()?;
    match is_system {
        None => Err(DbError::InvalidCommand(
            "category does not exist".to_owned(),
        )),
        Some(true) => Err(DbError::InvalidCommand(
            "a system category cannot be re-parented (its identity is fixed)".to_owned(),
        )),
        Some(false) => Ok(()),
    }
}

/// True if making `moving` a child of `new_parent` would create a cycle — i.e.
/// `moving` already appears in `new_parent`'s ancestor chain (this also covers
/// `new_parent == moving`). Mirrors the `categories_no_parent_cycle` trigger so
/// the command can reject with a clean error before the DB-level backstop fires.
fn would_create_category_cycle(
    tx: &Connection,
    new_parent: Uuid,
    moving: Uuid,
) -> Result<bool, DbError> {
    let count: i64 = tx.query_row(
        "WITH RECURSIVE ancestors(id) AS (
            SELECT ?1
            UNION ALL
            SELECT c.parent_id FROM categories c
            JOIN ancestors a ON c.id = a.id
            WHERE c.parent_id IS NOT NULL
        )
        SELECT COUNT(*) FROM ancestors WHERE id = ?2",
        params![new_parent, moving],
        |r| r.get(0),
    )?;
    Ok(count > 0)
}

fn ensure_default_categories(conn: &Connection) -> Result<(), DbError> {
    let already_seeded: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM categories WHERE is_system = 1)",
        [],
        |r| r.get(0),
    )?;
    if already_seeded {
        return Ok(());
    }
    let now = Utc::now().to_rfc3339();
    for group in DEFAULT_TAXONOMY {
        let group_id = Uuid::now_v7();
        insert_system_category(
            conn,
            group_id,
            None,
            group.name,
            group.category_type,
            group.forecast_behavior,
            &now,
        )?;
        for (leaf_name, leaf_behavior) in group.leaves {
            insert_system_category(
                conn,
                Uuid::now_v7(),
                Some(group_id),
                leaf_name,
                group.category_type,
                leaf_behavior,
                &now,
            )?;
        }
    }
    Ok(())
}

fn insert_system_category(
    conn: &Connection,
    id: Uuid,
    parent_id: Option<Uuid>,
    name: &str,
    category_type: &str,
    forecast_behavior: &str,
    now: &str,
) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO categories (
            id, household_id, parent_id, name, type, icon, color, is_system,
            budget_default_minor, forecast_behavior, created_at, updated_at
        ) VALUES (?1, NULL, ?2, ?3, ?4, NULL, NULL, 1, NULL, ?5, ?6, ?6)",
        params![id, parent_id, name, category_type, forecast_behavior, now],
    )?;
    Ok(())
}

/// Find (or create) the system ledger account for `(role, currency)`.
fn ensure_system_ledger_account(
    conn: &Connection,
    role: &str,
    currency: Currency,
    kind: AccountKind,
) -> Result<LedgerAccountId, DbError> {
    let existing: Option<Uuid> = conn
        .query_row(
            "SELECT ledger_account_id FROM system_ledger_accounts WHERE role = ?1 AND currency = ?2",
            params![role, currency.code()],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(uuid) = existing {
        return Ok(LedgerAccountId::from_uuid(uuid));
    }
    let ledger_account_id = LedgerAccountId::new();
    insert_ledger_account(conn, ledger_account_id, kind, currency)?;
    conn.execute(
        "INSERT INTO system_ledger_accounts (role, currency, ledger_account_id) VALUES (?1, ?2, ?3)",
        params![role, currency.code(), ledger_account_id.as_uuid()],
    )?;
    Ok(ledger_account_id)
}

fn insert_ledger_account(
    conn: &Connection,
    id: LedgerAccountId,
    kind: AccountKind,
    currency: Currency,
) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO ledger_accounts (id, kind, normal_balance, currency) VALUES (?1, ?2, ?3, ?4)",
        params![
            id.as_uuid(),
            kind.as_str(),
            kind.normal_balance().as_str(),
            currency.code(),
        ],
    )?;
    Ok(())
}

/// A category's display name, or the id itself if it can't be resolved (defensive — used only for
/// the band-drift signal's UI copy, personal-cfo-5ie.8).
fn category_display_name(conn: &Connection, category_id: &str) -> Result<String, DbError> {
    let Ok(uuid) = Uuid::parse_str(category_id) else {
        return Ok(category_id.to_owned());
    };
    Ok(conn
        .query_row("SELECT name FROM categories WHERE id = ?1", [uuid], |r| {
            r.get::<_, String>(0)
        })
        .optional()?
        .unwrap_or_else(|| category_id.to_owned()))
}

/// Persist a balanced ledger transaction. Postings are inserted first and the
/// header last, so the DB-side balance trigger (ADR 0007) fires once the full
/// transaction is present and rejects any imbalance at insert time.
fn persist_transaction(conn: &Connection, txn: &LedgerTransaction) -> Result<(), DbError> {
    let posting_date = txn.occurred_at().date_naive().to_string();
    for posting in txn.postings() {
        conn.execute(
            "INSERT INTO ledger_postings (
                transaction_id, ledger_account_id, minor_units, currency, currency_exponent, posting_date
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                txn.id().as_uuid(),
                posting.account().as_uuid(),
                posting.amount().minor_units(),
                posting.amount().currency().code(),
                i64::from(posting.amount().currency_exponent()),
                posting_date,
            ],
        )?;
    }
    conn.execute(
        "INSERT INTO ledger_transactions (id, operation_id, occurred_at, currency) VALUES (?1, ?2, ?3, ?4)",
        params![
            txn.id().as_uuid(),
            // operation_id joins operation_log.command_id, a BLOB (ADR 0011, 0s0).
            txn.operation_id().as_uuid(),
            txn.occurred_at().to_rfc3339(),
            txn.currency().code(),
        ],
    )?;
    Ok(())
}

/// Reverse `txn_uuid` (invert every posting, dated at `occurred_at`) and hide both the original
/// and the reversal from every view — so balances net to zero while the ledger stays
/// append-only (ADR 0007 §9). The caller has verified the transaction exists and is not already
/// voided. Shared by `VoidTransaction` and `UnconfirmObligation`.
fn reverse_and_hide_transaction(
    conn: &Connection,
    txn_uuid: Uuid,
    currency: Currency,
    occurred_at: DateTime<Utc>,
    command_id: Uuid,
) -> Result<(), DbError> {
    let mut stmt = conn.prepare(
        "SELECT ledger_account_id, minor_units FROM ledger_postings WHERE transaction_id = ?1",
    )?;
    let legs: Vec<(Uuid, i64)> = stmt
        .query_map([txn_uuid], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    drop(stmt);
    let mut reversal_postings = Vec::with_capacity(legs.len());
    for (account_uuid, minor) in legs {
        reversal_postings.push(Posting::new(
            LedgerAccountId::from_uuid(account_uuid),
            Money::new(minor, currency).checked_neg()?,
        ));
    }
    let reversal_id = TransactionId::new();
    let reversal = LedgerTransaction::new(
        reversal_id,
        OperationId::from_uuid(command_id),
        occurred_at,
        reversal_postings,
    )
    .map_err(|e| DbError::InvalidCommand(e.to_string()))?;
    persist_transaction(conn, &reversal)?;
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE ledger_transactions SET voided_at = ?1 WHERE id IN (?2, ?3)",
        params![now, txn_uuid, reversal_id.as_uuid()],
    )?;
    Ok(())
}

/// Record a committed transaction's display detail (personal-cfo-byxe): a free-text
/// `memo` + `counterparty`, kept beside the pure ledger. A no-op when both are
/// absent, so a detail-less manual transaction leaves no row (the list LEFT JOINs).
/// The raw imported source fields behind a committed transaction (ADR 0045 §2,
/// personal-cfo-4d8.24.1.4) — every column the importer captured, plus which
/// source produced them and when it was imported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedTransactionFields {
    /// The source format/type token, e.g. `csv` / `ofx` (from `source_batches`).
    pub source_type: String,
    /// When the batch was imported (RFC 3339), if recorded.
    pub imported_at: Option<String>,
    /// Every captured source field as `(key, value)`, key-sorted.
    pub fields: Vec<(String, String)>,
}

/// Parse a `source_records.normalized_json` object into key-sorted `(key, value)`
/// pairs. Values are strings as written; a non-string JSON value is stringified
/// (importers write string values, so this is defensive). A malformed/non-object
/// JSON yields an empty list rather than an error (the row is still shown).
fn parse_normalized_fields(json: &str) -> Vec<(String, String)> {
    serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(json)
        .map(|map| {
            map.into_iter()
                .map(|(key, value)| {
                    let text = match value {
                        serde_json::Value::String(s) => s,
                        serde_json::Value::Null => String::new(),
                        other => other.to_string(),
                    };
                    (key, text)
                })
                .collect()
        })
        .unwrap_or_default()
}

fn record_transaction_detail(
    conn: &Connection,
    transaction_id: Uuid,
    memo: Option<&str>,
    counterparty: Option<&str>,
    transaction_date: Option<&str>,
) -> Result<(), DbError> {
    if memo.is_none() && counterparty.is_none() && transaction_date.is_none() {
        return Ok(());
    }
    conn.execute(
        "INSERT OR REPLACE INTO transaction_details
            (transaction_id, memo, counterparty, transaction_date, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            transaction_id,
            memo,
            counterparty,
            transaction_date,
            Utc::now().to_rfc3339()
        ],
    )?;
    Ok(())
}

/// The assertion-anchored balance (ADR 0027, personal-cfo-ueg6): the latest
/// balance observation — a manual assertion or a connector-synced balance
/// (2026-09-02 addendum, yl53) — plus the postings dated strictly after it.
/// With no observation it is the plain posting sum (backward compatible). `account_id` is the account
/// UUID (assertions key on it); `ledger_account_id` is its ledger account.
pub(crate) fn assertion_anchored_balance(
    conn: &Connection,
    account_id: Uuid,
    ledger_account_id: Uuid,
) -> Result<i64, DbError> {
    let latest: Option<(i64, String)> = conn
        .query_row(
            "SELECT balance_amount_minor, observed_at FROM balance_observations
             WHERE account_id = ?1 AND source IN ('manual', 'connector_sync')
             ORDER BY observed_at DESC, created_at DESC LIMIT 1",
            [account_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    match latest {
        Some((asserted, observed_at)) => {
            let after: i64 = conn.query_row(
                "SELECT COALESCE(SUM(minor_units), 0) FROM ledger_postings
                 WHERE ledger_account_id = ?1 AND posting_date > ?2",
                params![ledger_account_id, observed_at],
                |r| r.get(0),
            )?;
            asserted
                .checked_add(after)
                .ok_or_else(|| DbError::InvalidCommand("balance overflow".to_owned()))
        }
        None => Ok(conn.query_row(
            "SELECT COALESCE(SUM(minor_units), 0) FROM ledger_postings
             WHERE ledger_account_id = ?1",
            [ledger_account_id],
            |r| r.get(0),
        )?),
    }
}

/// The derived auto-reconciling adjustment ("plug", ADR 0027) for an account: the
/// still-unexplained amount of its latest observation (manual or connector-
/// synced, ADR 0027 addendum) = asserted − (prior observation + real postings
/// in the window since it). `None` when there is no observation.
/// Recomputed on read, so it shrinks as real postings explain the gap.
pub(crate) fn unexplained_adjustment(
    conn: &Connection,
    account_id: Uuid,
    ledger_account_id: Uuid,
) -> Result<Option<i64>, DbError> {
    let latest: Option<(i64, String, String)> = conn
        .query_row(
            "SELECT balance_amount_minor, observed_at, source FROM balance_observations
             WHERE account_id = ?1 AND source IN ('manual', 'connector_sync')
             ORDER BY observed_at DESC, created_at DESC LIMIT 1",
            [account_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((asserted, observed_at, source)) = latest else {
        return Ok(None);
    };
    // Observations compare ACROSS days, not within one: a same-day sibling is
    // superseded by last-wins, and a posting (date-granular) could never land
    // inside a degenerate same-day window (yl53 review). The prior is the
    // newest observation on an EARLIER day.
    let prior: Option<(i64, String)> = conn
        .query_row(
            "SELECT balance_amount_minor, observed_at FROM balance_observations
             WHERE account_id = ?1 AND source IN ('manual', 'connector_sync')
               AND observed_at < ?2
             ORDER BY observed_at DESC, created_at DESC LIMIT 1",
            params![account_id, observed_at],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    // A FIRST-ever observation that came from a connector is the provider's
    // baseline truth, not a user-visible discrepancy: without it these
    // accounts had no plug at all, and a plug equal to the pre-history
    // balance would tank Forecast Readiness while asking the user to explain
    // something no action can (ADR 0027 addendum).
    if prior.is_none() && source == "connector_sync" {
        return Ok(None);
    }
    // Postings explaining the gap: those in (prior assertion date, this date], or
    // everything up to this date when there is no prior assertion.
    let in_window: i64 = match &prior {
        Some((_, prior_date)) => conn.query_row(
            "SELECT COALESCE(SUM(minor_units), 0) FROM ledger_postings
             WHERE ledger_account_id = ?1 AND posting_date > ?2 AND posting_date <= ?3",
            params![ledger_account_id, prior_date, observed_at],
            |r| r.get(0),
        )?,
        None => conn.query_row(
            "SELECT COALESCE(SUM(minor_units), 0) FROM ledger_postings
             WHERE ledger_account_id = ?1 AND posting_date <= ?2",
            params![ledger_account_id, observed_at],
            |r| r.get(0),
        )?,
    };
    let explained = prior
        .map_or(0, |(b, _)| b)
        .checked_add(in_window)
        .ok_or_else(|| DbError::InvalidCommand("adjustment overflow".to_owned()))?;
    let unexplained = asserted
        .checked_sub(explained)
        .ok_or_else(|| DbError::InvalidCommand("adjustment overflow".to_owned()))?;
    Ok(Some(unexplained))
}

fn read_account_balance(conn: &Connection, id: AccountId) -> Result<Option<Money>, DbError> {
    let row: Option<(Uuid, String)> = conn
        .query_row(
            "SELECT ledger_account_id, currency FROM accounts WHERE id = ?1",
            [id.as_uuid()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((ledger_account_id, currency_code)) = row else {
        return Ok(None);
    };
    let currency = currency_from_code(&currency_code)?;
    let sum = assertion_anchored_balance(conn, id.as_uuid(), ledger_account_id)?;
    Ok(Some(Money::new(sum, currency)))
}

/// The stored columns behind an [`AccountView`]: `(name, cashflow_role, subtype, active,
/// notes, linked_account_id)`. Named to keep the `query_row` type out of
/// `type_complexity`'s way.
type AccountRow = (
    String,
    String,
    Option<String>,
    i64,
    Option<String>,
    Option<Uuid>,
);

fn read_account_view(conn: &Connection, id: AccountId) -> Result<Option<AccountView>, DbError> {
    let row: Option<AccountRow> = conn
        .query_row(
            "SELECT name, cashflow_role, subtype, active, notes, linked_account_id
             FROM accounts WHERE id = ?1",
            [id.as_uuid()],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .optional()?;
    let Some((name, cashflow_role, subtype, active, notes, linked_uuid)) = row else {
        return Ok(None);
    };
    let balance = read_account_balance(conn, id)?
        .ok_or_else(|| DbError::InvalidCommand("account row without balance".to_owned()))?;
    let linked_account_id = linked_uuid.map(AccountId::from_uuid);
    let linked_account_name = read_link_partner_name(conn, id, linked_uuid)?;
    Ok(Some(AccountView {
        id,
        name,
        cashflow_role,
        subtype,
        active: active != 0,
        balance,
        notes,
        linked_account_id,
        linked_account_name,
    }))
}

/// Resolve an account's link-partner name for display (ADR 0044 §5), working both
/// directions: if THIS account points at a liability (`forward` is `Some`), return that
/// liability's name; otherwise return the name of the asset that points at THIS account
/// (the reverse lookup a liability uses). Only an ACTIVE partner surfaces a chip, so an
/// archived partner leaves no stale "Linked to …" on the live account. `None` when
/// unlinked (or the partner is archived). One-to-one is enforced on write, so at most one
/// asset points at a given liability; the `ORDER BY` keeps the read deterministic anyway.
fn read_link_partner_name(
    conn: &Connection,
    id: AccountId,
    forward: Option<Uuid>,
) -> Result<Option<String>, DbError> {
    if let Some(target) = forward {
        return conn
            .query_row(
                "SELECT name FROM accounts WHERE id = ?1 AND active = 1",
                [target],
                |r| r.get(0),
            )
            .optional()
            .map_err(Into::into);
    }
    conn.query_row(
        "SELECT name FROM accounts WHERE linked_account_id = ?1 AND active = 1
         ORDER BY name COLLATE NOCASE, id LIMIT 1",
        [id.as_uuid()],
        |r| r.get(0),
    )
    .optional()
    .map_err(Into::into)
}

/// Read every account as a [`AccountView`], ordered by name (case-insensitive).
/// The balance is summed per account from canonical postings, like the
/// single-account read; at personal-vault account counts the per-row balance
/// query is negligible.
fn read_account_views(conn: &Connection) -> Result<Vec<AccountView>, DbError> {
    let mut stmt = conn.prepare("SELECT id FROM accounts ORDER BY name COLLATE NOCASE, id")?;
    let ids: Vec<AccountId> = stmt
        .query_map([], |r| r.get::<_, Uuid>(0).map(AccountId::from_uuid))?
        .collect::<Result<_, _>>()?;
    let mut views = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(view) = read_account_view(conn, id)? {
            views.push(view);
        }
    }
    Ok(views)
}

/// Compute the type-based cash-tier rollups (ADR 0028, personal-cfo-9dgg).
///
/// Every `LiquidCash` account's assertion-anchored balance (ADR 0027) is bucketed
/// by its subtype — checking/cash (and any unclassified liquid) into spendable,
/// savings/money_market into reserve — so `net = spendable + reserve` is exactly
/// the sum of all liquid accounts. Single-currency like the forecast: the liquid
/// accounts' shared currency, or the base-currency setting (then USD) when there
/// are none. Mixed-currency liquid accounts are rejected, as in the forecast.
fn cash_tier_rollups(conn: &Connection) -> Result<CashTiers, DbError> {
    let mut stmt = conn.prepare(
        // `active = 1`: an archived account has left the household's forecast
        // (ADR 0056). This filter MUST move in lockstep with the other five liquid
        // reads — see the ADR; splitting them breaks the reconciliation invariant.
        "SELECT id, currency, ledger_account_id, subtype
         FROM accounts WHERE cashflow_role = 'liquid_cash' AND active = 1",
    )?;
    let accounts: Vec<(Uuid, String, Uuid, Option<String>)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<Result<_, _>>()?;

    // Single currency, mirroring `liquid_starting_balance`.
    let mut currency_code: Option<String> = None;
    for (_, code, _, _) in &accounts {
        match &currency_code {
            None => currency_code = Some(code.clone()),
            Some(existing) if existing != code => {
                return Err(DbError::InvalidCommand(
                    "Cash tiers do not support mixed-currency liquid accounts yet".to_owned(),
                ));
            }
            Some(_) => {}
        }
    }
    let currency = match &currency_code {
        Some(code) => currency_from_code(code)?,
        None => forecast::reporting_currency(conn)?.unwrap_or(Currency::Usd),
    };

    // Batched assertion-anchored balances (personal-cfo-3fdd.3c): one query for
    // every liquid account's latest manual assertion, one for the posting sums
    // after each account's anchor (the full sum when unasserted) — replacing the
    // per-account `assertion_anchored_balance` query pair while producing exactly
    // its results.
    let mut assertions: std::collections::HashMap<Uuid, i64> = std::collections::HashMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT account_id, balance_amount_minor
             FROM (SELECT account_id, balance_amount_minor,
                          ROW_NUMBER() OVER (PARTITION BY account_id
                                             ORDER BY observed_at DESC, created_at DESC) AS rn
                     FROM balance_observations WHERE source IN ('manual', 'connector_sync'))
             WHERE rn = 1",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, Uuid>(0)?, r.get::<_, i64>(1)?)))?;
        for row in rows {
            let (account_id, asserted) = row?;
            assertions.insert(account_id, asserted);
        }
    }
    let mut posting_sums: std::collections::HashMap<Uuid, i64> = std::collections::HashMap::new();
    {
        // Per liquid account: the postings dated strictly after its latest
        // observation (manual or connector-synced), or all postings when it
        // has none (ADR 0027 anchoring + 2026-09-02 addendum).
        let mut stmt = conn.prepare(
            "WITH latest AS (
                 SELECT account_id, observed_at
                 FROM (SELECT account_id, observed_at,
                              ROW_NUMBER() OVER (PARTITION BY account_id
                                                 ORDER BY observed_at DESC, created_at DESC) AS rn
                         FROM balance_observations WHERE source IN ('manual', 'connector_sync'))
                 WHERE rn = 1
             )
             SELECT a.id, COALESCE(SUM(lp.minor_units), 0)
             FROM accounts a
             LEFT JOIN latest l ON l.account_id = a.id
             LEFT JOIN ledger_postings lp
                    ON lp.ledger_account_id = a.ledger_account_id
                   AND (l.observed_at IS NULL OR lp.posting_date > l.observed_at)
             WHERE a.cashflow_role = 'liquid_cash' AND a.active = 1
             GROUP BY a.id",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, Uuid>(0)?, r.get::<_, i64>(1)?)))?;
        for row in rows {
            let (account_id, sum) = row?;
            posting_sums.insert(account_id, sum);
        }
    }

    let overflow = || DbError::InvalidCommand("cash tier overflow".to_owned());
    let mut spendable: i64 = 0;
    let mut reserve: i64 = 0;
    for (account_id, _, _ledger_account_id, subtype) in &accounts {
        let balance = assertions
            .get(account_id)
            .copied()
            .unwrap_or(0)
            .checked_add(posting_sums.get(account_id).copied().unwrap_or(0))
            .ok_or_else(|| DbError::InvalidCommand("balance overflow".to_owned()))?;
        // Unclassified liquid (NULL or an unknown token) folds into spendable, so
        // the net always equals the sum of every liquid account.
        let tier = subtype
            .as_deref()
            .and_then(AccountSubtype::from_token)
            .and_then(AccountSubtype::cash_tier)
            .unwrap_or(CashTier::Spendable);
        match tier {
            CashTier::Spendable => {
                spendable = spendable.checked_add(balance).ok_or_else(overflow)?
            }
            CashTier::Reserve => reserve = reserve.checked_add(balance).ok_or_else(overflow)?,
        }
    }
    let net = spendable.checked_add(reserve).ok_or_else(overflow)?;
    Ok(CashTiers {
        spendable: Money::new(spendable, currency),
        reserve: Money::new(reserve, currency),
        net: Money::new(net, currency),
    })
}

/// Parse a `group_concat(lower(hex(tag_id)))` string into a tag-id list (empty when
/// the transaction has no tags, i.e. the aggregate is `NULL`).
fn parse_tag_ids(concat: Option<&str>) -> Vec<TagId> {
    concat
        .map(|s| {
            s.split(',')
                .filter(|hex| !hex.is_empty())
                .filter_map(|hex| Uuid::parse_str(hex).ok())
                .map(TagId::from_uuid)
                .collect()
        })
        .unwrap_or_default()
}

/// The `TransactionRow` projection's SELECT columns. Shared by the recent-list read,
/// the filtered page read, and the duplicate-candidates read so a row is built
/// identically everywhere (the join to `accounts` in `TXN_ROW_FROM` restricts to
/// user-account postings). Tags / split counts / import provenance come from the
/// pre-aggregated LEFT JOINs in `TXN_ROW_FROM` (personal-cfo-3fdd.3a) — each side
/// table is scanned once per statement instead of once per result row.
///
/// The reviewed expression (ADR 0032: explicit override, else imported = 0 /
/// manual = 1 derived from provenance) is duplicated in `read_transaction_page`'s
/// `unreviewed_only` filter — keep the two in sync.
const TXN_ROW_COLUMNS: &str = "lt.id, a.id, a.name, lt.occurred_at, lp.minor_units, lp.currency,
                td.memo, td.counterparty, tc.category_id,
                COALESCE(tr.reviewed, CASE WHEN prov.entity_id IS NULL THEN 1 ELSE 0 END),
                td.note,
                tags.tag_concat,
                COALESCE(splits.line_count, 0),
                tc.source, tc.confidence_bps,
                td.transaction_date,
                -- The OTHER user account this transaction moved money against
                -- (personal-cfo-4d8.27.8.1) — see the `counter` join below. Appended
                -- last: the tuple is read positionally.
                counter.acct_id, counter.acct_name";

/// The `TransactionRow` projection's FROM + JOINs (user-account postings only).
const TXN_ROW_FROM: &str = "FROM ledger_postings lp
         JOIN ledger_transactions lt ON lt.id = lp.transaction_id
         JOIN ledger_accounts la ON la.id = lp.ledger_account_id
         JOIN accounts a ON a.ledger_account_id = la.id
         JOIN operation_log ol ON ol.command_id = lt.operation_id
         LEFT JOIN transaction_details td ON td.transaction_id = lt.id
         LEFT JOIN transaction_categorizations tc ON tc.transaction_id = lt.id
         LEFT JOIN transaction_reviews tr ON tr.transaction_id = lt.id
         LEFT JOIN (SELECT transaction_id, group_concat(lower(hex(tag_id))) AS tag_concat
                      FROM transaction_tags GROUP BY transaction_id) tags
                ON tags.transaction_id = lt.id
         LEFT JOIN (SELECT transaction_id, COUNT(*) AS line_count
                      FROM split_lines GROUP BY transaction_id) splits
                ON splits.transaction_id = lt.id
         LEFT JOIN (SELECT entity_id FROM source_provenance_links
                     WHERE entity_type = 'ledger_transaction' GROUP BY entity_id) prov
                ON prov.entity_id = lt.id
         -- The far side of a transfer (personal-cfo-4d8.27.8.1): the transaction's OTHER
         -- user-account posting. A JOINED derived table rather than a correlated
         -- subquery, matching `tags`/`splits`/`prov` above — `txn_row_reads_have_no_
         -- correlated_subqueries` (personal-cfo-3fdd.3a) fails the build otherwise.
         --
         -- An ordinary income/expense has one user posting and one SYSTEM posting, and
         -- system ledger accounts have no `accounts` row, so nothing matches and the
         -- LEFT JOIN yields NULL. A transfer has exactly two user postings, so exactly
         -- one matches: no row multiplication. (Nothing writes a third user posting; a
         -- future multi-leg command would need this revisited.)
         -- Restricted to transactions that HAVE a second user posting (i.e. transfers).
         -- Without the HAVING, SQLite materializes a projection of the whole postings
         -- table on every list read; with it, only transfers materialize, which is a
         -- small fraction of a vault.
         LEFT JOIN (SELECT lp2.transaction_id AS txn_id, a2.id AS acct_id,
                           a2.name AS acct_name
                      FROM ledger_postings lp2
                      JOIN ledger_accounts la2 ON la2.id = lp2.ledger_account_id
                      JOIN accounts a2 ON a2.ledger_account_id = la2.id
                     WHERE lp2.transaction_id IN (
                           SELECT lp3.transaction_id
                             FROM ledger_postings lp3
                             JOIN ledger_accounts la3 ON la3.id = lp3.ledger_account_id
                             JOIN accounts a3 ON a3.ledger_account_id = la3.id
                            GROUP BY lp3.transaction_id
                           HAVING COUNT(*) > 1)) counter
                ON counter.txn_id = lt.id AND counter.acct_id <> a.id";

/// A category and every category beneath it (ADR 0052 §2).
///
/// Read whole and walked in memory rather than expressed as a recursive CTE: the taxonomy
/// is a small, bounded table (~64 seeded rows) that every filtered page read would
/// otherwise re-walk in SQL. The `seen` set makes the walk safe even against a vault
/// whose parent chain was hand-edited past the schema's cycle trigger.
fn category_subtree(conn: &Connection, root: Uuid) -> Result<Vec<Uuid>, DbError> {
    let mut stmt = conn.prepare("SELECT id, parent_id FROM categories")?;
    let mut edges: Vec<(Uuid, Option<Uuid>)> = Vec::new();
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        edges.push((row.get(0)?, row.get(1)?));
    }
    let mut out = vec![root];
    let mut frontier = vec![root];
    while let Some(parent) = frontier.pop() {
        for (id, p) in &edges {
            if *p == Some(parent) && !out.contains(id) {
                out.push(*id);
                frontier.push(*id);
            }
        }
    }
    Ok(out)
}

/// The raw column tuple read from a `TXN_ROW_COLUMNS` row, before typed construction.
type TxnRowTuple = (
    Uuid,
    Uuid,
    String,
    String,
    i64,
    String,
    Option<String>,
    Option<String>,
    Option<Uuid>,
    i64,
    Option<String>,
    Option<String>,
    i64,
    Option<String>,
    Option<i64>,
    // 15: td.transaction_date (ADR 0045) — the secondary transaction date, if any.
    Option<String>,
    // 16/17: the counter account (personal-cfo-4d8.27.8.1) — `None` unless this
    // transaction has a second USER posting (i.e. a transfer).
    Option<Uuid>,
    Option<String>,
);

/// Read one `TXN_ROW_COLUMNS` row into its raw tuple (the `query_map` closure).
fn read_txn_row_tuple(r: &rusqlite::Row) -> rusqlite::Result<TxnRowTuple> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
        r.get(6)?,
        r.get(7)?,
        r.get(8)?,
        r.get(9)?,
        r.get(10)?,
        r.get(11)?,
        r.get(12)?,
        r.get(13)?,
        r.get(14)?,
        r.get(15)?,
        r.get(16)?,
        r.get(17)?,
    ))
}

/// Build a typed [`TransactionRow`] from a raw `TXN_ROW_COLUMNS` tuple.
fn build_txn_row(t: TxnRowTuple) -> Result<TransactionRow, DbError> {
    let (
        txn_id,
        account_id,
        account_name,
        occurred_at,
        minor_units,
        currency_code,
        memo,
        counterparty,
        category_id,
        reviewed_int,
        note,
        tags_concat,
        split_count,
        category_source,
        category_confidence_bps,
        transaction_date,
        counter_account_id,
        counter_account_name,
    ) = t;
    Ok(TransactionRow {
        // Filled in afterwards by `attach_balances`, and only when asked for: the page
        // query itself must stay free of per-row cumulative sums.
        balance_after_minor: None,
        transaction_id: TransactionId::from_uuid(txn_id),
        account_id: AccountId::from_uuid(account_id),
        account_name,
        counter_account_id: counter_account_id.map(AccountId::from_uuid),
        counter_account_name,
        occurred_at: DateTime::parse_from_rfc3339(&occurred_at)
            .map(|dt| dt.with_timezone(&Utc))
            .map_err(|e| DbError::InvalidCommand(e.to_string()))?,
        // The stored secondary date is a plain `YYYY-MM-DD`; a malformed value is
        // dropped rather than failing the whole row read.
        transaction_date: transaction_date
            .as_deref()
            .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()),
        amount: Money::new(minor_units, currency_from_code(&currency_code)?),
        memo,
        counterparty,
        category_id: category_id.map(CategoryId::from_uuid),
        reviewed: reviewed_int != 0,
        note,
        tag_ids: parse_tag_ids(tags_concat.as_deref()),
        split_count,
        category_source,
        category_confidence_bps,
    })
}

/// Read the most recent transactions (newest first, capped at `limit`) directly
/// from canonical tables. The join to `accounts` restricts rows to user-account
/// postings (one per manual transaction), excluding the system counter-posting;
/// the signed amount is from the account's perspective.
fn read_recent_transactions(conn: &Connection, limit: u32) -> Result<Vec<TransactionRow>, DbError> {
    let sql = format!(
        "SELECT {TXN_ROW_COLUMNS} {TXN_ROW_FROM}
         WHERE lt.voided_at IS NULL
         ORDER BY lt.occurred_at DESC, ol.op_seq DESC
         LIMIT ?1"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([limit], read_txn_row_tuple)?;
    rows.map(|r| build_txn_row(r?)).collect()
}

/// The row DTOs for an explicit id set, newest first — the Money Inbox's row-resolution
/// read (personal-cfo-4d8.25.15): inbox items outside the recent-`TRANSACTION_LIST_LIMIT`
/// window still need full rows for selection + bulk actions + Card Review. Unknown/voided
/// ids are silently absent from the result (the caller reconciles). Chunked IN-lists keep
/// the statement under SQLite's bind-parameter cap.
fn read_transactions_by_ids(
    conn: &Connection,
    ids: &[Uuid],
) -> Result<Vec<TransactionRow>, DbError> {
    let mut out = Vec::with_capacity(ids.len());
    for chunk in ids.chunks(500) {
        let placeholders = vec!["?"; chunk.len()].join(",");
        let sql = format!(
            "SELECT {TXN_ROW_COLUMNS} {TXN_ROW_FROM}
             WHERE lt.voided_at IS NULL AND lt.id IN ({placeholders})
             ORDER BY lt.occurred_at DESC, ol.op_seq DESC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(chunk.iter()), read_txn_row_tuple)?;
        for row in rows {
            out.push(build_txn_row(row?)?);
        }
    }
    Ok(out)
}

/// How a transaction page is ordered (personal-cfo-3fdd.1). Every order is total —
/// ties break on the op-log sequence — so consecutive windows never overlap or skip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TransactionSortOrder {
    /// Newest first (the default; the recent list's order).
    #[default]
    NewestFirst,
    /// Oldest first.
    OldestFirst,
    /// Largest signed amount first.
    AmountDesc,
    /// Smallest signed amount first.
    AmountAsc,
}

impl TransactionSortOrder {
    /// The ORDER BY column list for this order (fixed strings, never user input).
    const fn order_by(self) -> &'static str {
        match self {
            Self::NewestFirst => "lt.occurred_at DESC, ol.op_seq DESC",
            Self::OldestFirst => "lt.occurred_at ASC, ol.op_seq ASC",
            // Amount orders keep newest-first between equal amounts, matching the
            // stable client-side sort they replace.
            Self::AmountDesc => "lp.minor_units DESC, lt.occurred_at DESC, ol.op_seq DESC",
            Self::AmountAsc => "lp.minor_units ASC, lt.occurred_at DESC, ol.op_seq DESC",
        }
    }
}

/// The category constraint of a [`TransactionPageQuery`]: one specific category, or
/// only rows with no assignment (the UI's "uncategorized" sentinel).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CategoryFilter {
    /// Only transactions with no category assigned.
    Uncategorized,
    /// Only transactions assigned this category.
    Category(CategoryId),
}

/// A filtered, ordered window over the transaction list (personal-cfo-3fdd.1): the
/// SQL-side counterpart of the UI's filter bar + pager, so search/filter/sort span
/// ALL history instead of a client-side scan of the recent 200 rows. `None` /
/// `false` fields mean "no constraint".
#[derive(Debug, Clone, Default)]
pub struct TransactionPageQuery {
    /// Case-insensitive substring over memo / counterparty / note / account name.
    pub query: Option<String>,
    /// Restrict to these accounts. **Empty means no constraint** — a set rather than an
    /// `Option<AccountId>` because the Debt page scopes its embedded list to a
    /// multi-account selection (ADR 0057 §3), and the filter bar's single-account facet
    /// is just the one-element case of the same idea rather than a second mechanism.
    pub account_ids: Vec<AccountId>,
    /// Compute `balance_after_minor` for the returned rows (personal-cfo-ttuy).
    ///
    /// Off by default: it costs an extra bounded read, and only the transactions list
    /// shows the column. A running balance is only meaningful in DATE order, so callers
    /// must not set this for an amount-sorted page — the row values would each be correct
    /// and the column as a whole would be nonsense.
    pub with_balances: bool,
    /// Only this category — or only uncategorized rows.
    pub category: Option<CategoryFilter>,
    /// Only transactions carrying this tag.
    pub tag_id: Option<TagId>,
    /// Only transactions confirmed as paying this recurring bill (personal-cfo-4d8.24.7.1):
    /// the linked-payment history a bill's detail panel shows. Matches via
    /// `confirmed_obligations`.
    pub recurring_event_id: Option<RecurringEventId>,
    /// Inclusive calendar-day lower bound on the occurred-at date.
    pub from: Option<NaiveDate>,
    /// Inclusive calendar-day upper bound on the occurred-at date.
    pub to: Option<NaiveDate>,
    /// Only unreviewed transactions (ADR 0032 derived-reviewed semantics).
    pub unreviewed_only: bool,
    /// Row order (a deterministic total order, so windows never overlap).
    pub sort: TransactionSortOrder,
    /// Window size (rows per page).
    pub limit: u32,
    /// Window start within the filtered, ordered set.
    pub offset: u32,
}

/// One page of the filtered transaction list plus the total match count
/// (personal-cfo-3fdd.1), so the UI can render real page controls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionPage {
    /// The requested window of rows.
    pub rows: Vec<TransactionRow>,
    /// How many rows match the filter in total, across all pages.
    pub total: u32,
}

/// Escape SQL LIKE wildcards in user input (`\` is the ESCAPE character), so a
/// search for "100%" or "big_sur" matches literally instead of as a pattern.
pub(crate) fn escape_like(raw: &str) -> String {
    raw.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Read one filtered, ordered page of transactions plus the total match count
/// (personal-cfo-3fdd.1). Every user-supplied value binds as a parameter — nothing
/// from the query is interpolated into the SQL (the ORDER BY strings are fixed).
/// With no filters, newest-first, offset 0 this returns exactly the head of
/// `read_recent_transactions`.
/// Fill in each row's `balance_after_minor` from the full ledger.
///
/// A SEPARATE, bounded read rather than a column on the page query, for two reasons:
///
/// 1. The page query's plan is guarded against correlated per-row subqueries
///    (`txn_row_reads_have_no_correlated_subqueries`), and a per-row cumulative sum is
///    exactly that. Windowing inside the page query instead would drag the whole ledger
///    into every read.
/// 2. The window here is PARTITIONED to the accounts actually on the page, so the work is
///    bounded by those accounts' postings rather than by the vault.
///
/// The balance is a cumulative sum at the transaction's position in the account's own
/// ledger — filters never enter it, so narrowing the list cannot change what an account
/// held at a moment.
fn attach_balances(
    conn: &Connection,
    mut rows: Vec<TransactionRow>,
) -> Result<Vec<TransactionRow>, DbError> {
    if rows.is_empty() {
        return Ok(rows);
    }
    let mut accounts: Vec<Uuid> = rows.iter().map(|r| r.account_id.as_uuid()).collect();
    accounts.sort_unstable();
    accounts.dedup();

    let placeholders = vec!["?"; accounts.len()].join(", ");
    let sql = format!(
        "SELECT transaction_id, account_id, running FROM (
             SELECT lp.transaction_id AS transaction_id,
                    a.id              AS account_id,
                    SUM(lp.minor_units) OVER (
                        PARTITION BY a.id
                        ORDER BY lt.occurred_at, lt.id
                        ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW
                    ) AS running
               FROM ledger_postings lp
               JOIN ledger_transactions lt ON lt.id = lp.transaction_id
               JOIN ledger_accounts la ON la.id = lp.ledger_account_id
               JOIN accounts a ON a.ledger_account_id = la.id
              WHERE lt.voided_at IS NULL
                AND a.id IN ({placeholders}))"
    );
    let mut stmt = conn.prepare(&sql)?;
    let binds: Vec<&dyn rusqlite::ToSql> = accounts
        .iter()
        .map(|id| id as &dyn rusqlite::ToSql)
        .collect();
    let mut found: std::collections::HashMap<(Uuid, Uuid), i64> = std::collections::HashMap::new();
    let mut cursor = stmt.query(rusqlite::params_from_iter(binds))?;
    while let Some(row) = cursor.next()? {
        found.insert(
            (row.get::<_, Uuid>(0)?, row.get::<_, Uuid>(1)?),
            row.get::<_, i64>(2)?,
        );
    }
    for row in &mut rows {
        row.balance_after_minor = found
            .get(&(row.transaction_id.as_uuid(), row.account_id.as_uuid()))
            .copied();
    }
    Ok(rows)
}

fn read_transaction_page(
    conn: &Connection,
    query: &TransactionPageQuery,
) -> Result<TransactionPage, DbError> {
    let mut clauses: Vec<String> = vec!["lt.voided_at IS NULL".to_owned()];
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    // The tag filter is an extra INNER JOIN, not a subquery: (transaction_id,
    // tag_id) is the table's PK, so it matches at most once per row (no grain
    // change) and the plan stays subquery-free. Its `?` precedes the WHERE
    // parameters in the SQL text, so it binds first.
    let tag_join = if let Some(tag_id) = query.tag_id {
        params.push(Box::new(tag_id.as_uuid()));
        "JOIN transaction_tags tagf ON tagf.transaction_id = lt.id AND tagf.tag_id = ?"
    } else {
        ""
    };
    // Free-text search: the same fields the client-side `matchesQuery` spanned
    // (memo / counterparty / note / account name; NULL fields simply don't match).
    if let Some(needle) = query
        .query
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let pattern = format!("%{}%", escape_like(needle));
        clauses.push(
            "(td.memo LIKE ? ESCAPE '\\' OR td.counterparty LIKE ? ESCAPE '\\'
              OR td.note LIKE ? ESCAPE '\\' OR a.name LIKE ? ESCAPE '\\')"
                .to_owned(),
        );
        for _ in 0..4 {
            params.push(Box::new(pattern.clone()));
        }
    }
    if !query.account_ids.is_empty() {
        // A bound IN-list, one placeholder per id — never interpolated. The set is small
        // and caller-supplied (the accounts a household owns), so this stays a LIST
        // SUBQUERY-free equality test on an indexed column.
        let placeholders = vec!["?"; query.account_ids.len()].join(", ");
        clauses.push(format!("a.id IN ({placeholders})"));
        for account_id in &query.account_ids {
            params.push(Box::new(account_id.as_uuid()));
        }
    }
    // Only the transactions confirmed as paying this recurring bill
    // (personal-cfo-4d8.24.7.1). The inner set is computed once via the
    // `UNIQUE(recurring_event_id, scheduled_date)` index on `confirmed_obligations`
    // (few rows per bill), then `lt.id IN (...)` filters the page.
    if let Some(recurring_event_id) = query.recurring_event_id {
        clauses.push(
            "lt.id IN (SELECT transaction_id FROM confirmed_obligations \
             WHERE recurring_event_id = ?)"
                .to_owned(),
        );
        params.push(Box::new(recurring_event_id.as_uuid()));
    }
    match query.category {
        // Uncategorized means nothing classifies it — including its split lines. A split
        // with categorized lines is NOT uncategorized, or it would show both here and
        // under those categories.
        Some(CategoryFilter::Uncategorized) => clauses.push(
            "(tc.category_id IS NULL
              AND lt.id NOT IN (SELECT transaction_id FROM split_lines
                                 WHERE category_id IS NOT NULL))"
                .to_owned(),
        ),
        Some(CategoryFilter::Category(id)) => {
            // Filtering to a category means its WHOLE SUBTREE, and it must also reach
            // transactions whose SPLIT LINES carry the category (ADR 0052 §2). Without
            // the first, picking "Food" hides everything filed under "Food / Dining";
            // without the second, a $200 trip itemized into Groceries is invisible when
            // filtering to Groceries — and both would make the spend chart's drill-down
            // show fewer rows than the cell it came from.
            //
            // The subtree is resolved in Rust (the taxonomy is small and bounded) and
            // bound as an IN-list; the split match is a NON-correlated `IN (SELECT …)`,
            // the same idiom as the recurring-bill filter above, so it plans as a cached
            // LIST SUBQUERY rather than tripping the no-correlated-subquery guard.
            let subtree = category_subtree(conn, id.as_uuid())?;
            let placeholders = vec!["?"; subtree.len()].join(", ");
            // Splits WIN over the parent categorization, exactly as the spend aggregate
            // treats them (ADR 0052 §3). Without the `NOT IN split_lines` guard the two
            // disagree in the other direction: a $200 charge auto-filed under Shopping and
            // then split into Groceries/Household still matches a Shopping filter, while
            // the chart counts $0 of Shopping from it — so clicking the Shopping cell
            // would list a row that cell counted nothing of.
            clauses.push(format!(
                "((lt.id NOT IN (SELECT transaction_id FROM split_lines)
                   AND tc.category_id IN ({placeholders}))
                  OR lt.id IN (SELECT sl.transaction_id FROM split_lines sl
                                WHERE sl.category_id IN ({placeholders})))"
            ));
            for id in &subtree {
                params.push(Box::new(*id));
            }
            for id in &subtree {
                params.push(Box::new(*id));
            }
        }
        None => {}
    }
    // Date bounds compare the stored RFC 3339 text against the bare `YYYY-MM-DD`
    // day, keeping the occurred_at index usable: any timestamped value on `from`'s
    // day sorts after the bare date, and `<` the day after `to` includes all of
    // `to`'s day (both bounds inclusive, like the filter bar).
    if let Some(from) = query.from {
        clauses.push("lt.occurred_at >= ?".to_owned());
        params.push(Box::new(from.to_string()));
    }
    if let Some(to) = query.to {
        if let Some(day_after) = to.succ_opt() {
            clauses.push("lt.occurred_at < ?".to_owned());
            params.push(Box::new(day_after.to_string()));
        }
    }
    if query.unreviewed_only {
        // Keep in sync with TXN_ROW_COLUMNS' reviewed expression (ADR 0032).
        clauses.push(
            "COALESCE(tr.reviewed, CASE WHEN prov.entity_id IS NULL THEN 1 ELSE 0 END) = 0"
                .to_owned(),
        );
    }
    let where_sql = clauses.join(" AND ");

    let count_sql = format!("SELECT COUNT(*) {TXN_ROW_FROM} {tag_join} WHERE {where_sql}");
    let total: u32 = conn.query_row(
        &count_sql,
        rusqlite::params_from_iter(params.iter().map(AsRef::as_ref)),
        |r| r.get(0),
    )?;

    let rows_sql = format!(
        "SELECT {TXN_ROW_COLUMNS} {TXN_ROW_FROM} {tag_join}
         WHERE {where_sql}
         ORDER BY {order_by}
         LIMIT ? OFFSET ?",
        order_by = query.sort.order_by(),
    );
    params.push(Box::new(query.limit));
    params.push(Box::new(query.offset));
    let mut stmt = conn.prepare(&rows_sql)?;
    let rows = stmt.query_map(
        rusqlite::params_from_iter(params.iter().map(AsRef::as_ref)),
        read_txn_row_tuple,
    )?;
    let rows = rows
        .map(|r| build_txn_row(r?))
        .collect::<Result<Vec<_>, _>>()?;
    let rows = if query.with_balances {
        attach_balances(conn, rows)?
    } else {
        rows
    };
    Ok(TransactionPage { rows, total })
}

/// The committed ledger transactions that look like duplicates of the given flagged
/// staged transaction — they share its `txn_fingerprint` and are already committed
/// (ADR 0032 §4, personal-cfo-4d8.20). Returns 1 for the common case, N for the
/// multi-match case, 0 when none; backs the duplicate Review panel's counterpart column.
fn read_duplicate_candidates(
    conn: &Connection,
    staged_transaction_id: StagedTransactionId,
) -> Result<Vec<TransactionRow>, DbError> {
    let sql = format!(
        "SELECT {TXN_ROW_COLUMNS} {TXN_ROW_FROM}
         WHERE lt.voided_at IS NULL AND lt.id IN (
             SELECT c.committed_transaction_id
             FROM staged_transactions c
             JOIN staged_transactions s ON s.id = ?1
             WHERE c.txn_fingerprint = s.txn_fingerprint
               AND c.commit_status = 'committed'
               AND c.committed_transaction_id IS NOT NULL
             UNION
             SELECT d.matched_entity_id
             FROM dedupe_decisions d
             WHERE d.staged_transaction_id = ?1 AND d.decision = 'flagged'
               AND d.matched_entity_type = 'ledger_transaction'
               AND d.matched_entity_id IS NOT NULL
         )
         ORDER BY lt.occurred_at DESC, ol.op_seq DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([staged_transaction_id.as_uuid()], read_txn_row_tuple)?;
    rows.map(|r| build_txn_row(r?)).collect()
}

/// A tag for display (ADR 0033, personal-cfo-2ryf).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagView {
    /// The tag id.
    pub id: TagId,
    /// Display name.
    pub name: String,
    /// Optional display color, or `None`.
    pub color: Option<String>,
    /// Whether the tag is archived (soft-deleted).
    pub archived: bool,
}

/// Read every tag for the picker, ordered by name (ADR 0033).
fn read_tag_views(conn: &Connection) -> Result<Vec<TagView>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, name, color, archived_at FROM tags ORDER BY name COLLATE NOCASE, id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, Uuid>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, Option<String>>(3)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, name, color, archived_at) = row?;
        out.push(TagView {
            id: TagId::from_uuid(id),
            name,
            color,
            archived: archived_at.is_some(),
        });
    }
    Ok(out)
}

/// One line of a split transaction for display (ADR 0034, personal-cfo-kr9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitLineView {
    /// The split line id.
    pub id: SplitLineId,
    /// The slice amount (same sign + currency as the transaction).
    pub amount: Money,
    /// The line's category (`None` = uncategorized).
    pub category_id: Option<CategoryId>,
    /// The line's note (`None` = no note).
    pub note: Option<String>,
    /// The line's tags.
    pub tag_ids: Vec<TagId>,
    /// Display order within the transaction.
    pub sort_order: i64,
}

/// Read a transaction's split lines, ordered (ADR 0034). Empty when the transaction is
/// not split.
fn read_transaction_splits(
    conn: &Connection,
    transaction_id: TransactionId,
) -> Result<Vec<SplitLineView>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, amount_minor, currency, category_id, note, sort_order,
                (SELECT group_concat(lower(hex(slt.tag_id)))
                   FROM split_line_tags slt WHERE slt.split_line_id = split_lines.id)
         FROM split_lines WHERE transaction_id = ?1 ORDER BY sort_order, id",
    )?;
    let rows = stmt.query_map([transaction_id.as_uuid()], |r| {
        Ok((
            r.get::<_, Uuid>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, Option<Uuid>>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, i64>(5)?,
            r.get::<_, Option<String>>(6)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, minor, currency, category_id, note, sort_order, tags_concat) = row?;
        out.push(SplitLineView {
            id: SplitLineId::from_uuid(id),
            amount: Money::new(minor, currency_from_code(&currency)?),
            category_id: category_id.map(CategoryId::from_uuid),
            note,
            tag_ids: parse_tag_ids(tags_concat.as_deref()),
            sort_order,
        });
    }
    Ok(out)
}

/// Read every income source as an [`IncomeSourceView`], ordered by name, with the
/// next pay date derived from its schedule (relative to today, household-local
/// calendar — ADR 0021 §1, personal-cfo-m8x2r).
pub(crate) fn read_income_source_views(
    conn: &Connection,
) -> Result<Vec<IncomeSourceView>, DbError> {
    let today = forecast::household_today(conn)?;
    let mut stmt = conn.prepare(
        "SELECT s.id, s.name, s.net_minor_units, s.currency, s.frequency, s.anchor_date,
                s.deposit_account_id, a.name, s.active, s.created_at, s.archived_at
         FROM income_sources s
         LEFT JOIN accounts a ON a.id = s.deposit_account_id
         ORDER BY s.name COLLATE NOCASE, s.id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, Uuid>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, String>(5)?,
            r.get::<_, Option<Uuid>>(6)?,
            r.get::<_, Option<String>>(7)?,
            r.get::<_, i64>(8)?,
            r.get::<_, String>(9)?,
            r.get::<_, Option<String>>(10)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (
            id,
            name,
            minor_units,
            currency_code,
            freq_token,
            anchor_str,
            deposit,
            deposit_name,
            active,
            created_at,
            archived_at,
        ) = row?;
        let frequency = Frequency::from_token(&freq_token).ok_or_else(|| {
            DbError::InvalidCommand(format!("unknown income frequency token: {freq_token}"))
        })?;
        let anchor = NaiveDate::parse_from_str(&anchor_str, "%Y-%m-%d")
            .map_err(|e| DbError::InvalidCommand(e.to_string()))?;
        let next_pay_date = PaySchedule::new(frequency, anchor).next_pay_date(today);
        out.push(IncomeSourceView {
            id: IncomeSourceId::from_uuid(id),
            name,
            net_amount: Money::new(minor_units, currency_from_code(&currency_code)?),
            frequency,
            anchor,
            deposit_account_id: deposit.map(AccountId::from_uuid),
            deposit_account_name: deposit_name,
            next_pay_date,
            active: active != 0,
            created_at,
            archived_at,
        });
    }
    Ok(out)
}

/// Read every manual recurring bill as a [`RecurringBillView`], ordered by name,
/// with the next due date derived from its schedule (relative to today,
/// household-local calendar — ADR 0021 §1, personal-cfo-m8x2r). The schedule
/// lives on `recurring_events`; the `bill_contract` supplies the type.
pub(crate) fn read_recurring_bill_views(
    conn: &Connection,
) -> Result<Vec<RecurringBillView>, DbError> {
    let today = forecast::household_today(conn)?;
    let mut stmt = conn.prepare(
        "SELECT e.id, e.name, b.type, e.amount_expected_minor, e.currency, e.frequency,
                e.next_expected_date, e.autopay_account_id, a.name, b.description,
                e.is_active, e.created_at, e.archived_at, COALESCE(e.autopay_enabled, 0),
                e.category_id, rt.tag_concat
         FROM recurring_events e
         JOIN bill_contracts b ON b.recurring_event_id = e.id
         LEFT JOIN accounts a ON a.id = e.autopay_account_id
         LEFT JOIN (SELECT recurring_event_id, group_concat(lower(hex(tag_id))) AS tag_concat
                      FROM recurring_event_tags GROUP BY recurring_event_id) rt
                ON rt.recurring_event_id = e.id
         WHERE e.source = 'manual'
         ORDER BY e.name COLLATE NOCASE, e.id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, Uuid>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, String>(5)?,
            r.get::<_, Option<String>>(6)?,
            r.get::<_, Option<Uuid>>(7)?,
            r.get::<_, Option<String>>(8)?,
            r.get::<_, Option<String>>(9)?,
            r.get::<_, i64>(10)?,
            r.get::<_, String>(11)?,
            r.get::<_, Option<String>>(12)?,
            r.get::<_, i64>(13)?,
            r.get::<_, Option<Uuid>>(14)?,
            r.get::<_, Option<String>>(15)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (
            id,
            name,
            bill_type,
            minor_units,
            currency_code,
            freq_token,
            anchor_opt,
            autopay,
            autopay_name,
            description,
            is_active,
            created_at,
            archived_at,
            autopay_enabled,
            category_id,
            tag_concat,
        ) = row?;
        let frequency = Frequency::from_token(&freq_token).ok_or_else(|| {
            DbError::InvalidCommand(format!("unknown bill frequency token: {freq_token}"))
        })?;
        let anchor_str = anchor_opt.ok_or_else(|| {
            DbError::InvalidCommand("recurring bill is missing its anchor date".to_owned())
        })?;
        let anchor = NaiveDate::parse_from_str(&anchor_str, "%Y-%m-%d")
            .map_err(|e| DbError::InvalidCommand(e.to_string()))?;
        let next_due_date = PaySchedule::new(frequency, anchor).next_pay_date(today);
        out.push(RecurringBillView {
            id: RecurringEventId::from_uuid(id),
            name,
            bill_type,
            amount: Money::new(minor_units, currency_from_code(&currency_code)?),
            frequency,
            anchor,
            autopay_account_id: autopay.map(AccountId::from_uuid),
            autopay_account_name: autopay_name,
            autopay_enabled: autopay_enabled != 0,
            next_due_date,
            description,
            active: is_active != 0,
            created_at,
            archived_at,
            category_id: category_id.map(CategoryId::from_uuid),
            tag_ids: parse_tag_ids(tag_concat.as_deref()),
        });
    }
    Ok(out)
}

/// Read every recurring transfer with its next occurrence (ADR 0026 §14,
/// personal-cfo-npoe). Joins both accounts for display names.
pub(crate) fn read_recurring_transfer_views(
    conn: &Connection,
) -> Result<Vec<RecurringTransferView>, DbError> {
    // Household-local "today" (ADR 0021 §1), not UTC's — the next-occurrence
    // projection below is a calendar-boundary comparison (personal-cfo-m8x2r).
    let today = forecast::household_today(conn)?;
    let mut stmt = conn.prepare(
        "SELECT t.id, t.source_account_id, sa.name, t.dest_account_id, da.name,
                t.amount_minor, t.currency, t.frequency, t.anchor_date, t.created_at
         FROM recurring_transfers t
         JOIN accounts sa ON sa.id = t.source_account_id
         JOIN accounts da ON da.id = t.dest_account_id
         ORDER BY t.created_at, t.id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, Uuid>(0)?,
            r.get::<_, Uuid>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, Uuid>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, i64>(5)?,
            r.get::<_, String>(6)?,
            r.get::<_, String>(7)?,
            r.get::<_, String>(8)?,
            r.get::<_, String>(9)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (
            id,
            source_id,
            source_name,
            dest_id,
            dest_name,
            amount_minor,
            code,
            freq_token,
            anchor_str,
            created_at,
        ) = row?;
        let frequency = Frequency::from_token(&freq_token).ok_or_else(|| {
            DbError::InvalidCommand(format!("unknown transfer frequency token: {freq_token}"))
        })?;
        let anchor = NaiveDate::parse_from_str(&anchor_str, "%Y-%m-%d")
            .map_err(|e| DbError::InvalidCommand(e.to_string()))?;
        let next_date = PaySchedule::new(frequency, anchor).next_pay_date(today);
        out.push(RecurringTransferView {
            id: RecurringTransferId::from_uuid(id),
            source_account_id: AccountId::from_uuid(source_id),
            source_account_name: source_name,
            dest_account_id: AccountId::from_uuid(dest_id),
            dest_account_name: dest_name,
            amount: Money::new(amount_minor, currency_from_code(&code)?),
            frequency,
            anchor,
            next_date,
            created_at,
        });
    }
    Ok(out)
}

fn read_vault_metadata(conn: &Connection) -> Result<VaultMetadata, DbError> {
    conn.query_row(
        "SELECT vault_id, schema_version, envelope_version, kdf_algorithm,
                kdf_memory_kib, kdf_time_cost, kdf_parallelism, household_timezone,
                manifest_pointer
         FROM vault_metadata WHERE singleton = 1",
        [],
        |r| {
            Ok(VaultMetadata {
                vault_id: r.get(0)?,
                schema_version: r.get(1)?,
                envelope_version: r.get(2)?,
                kdf_algorithm: r.get(3)?,
                kdf_memory_kib: r.get(4)?,
                kdf_time_cost: r.get(5)?,
                kdf_parallelism: r.get(6)?,
                household_timezone: r.get(7)?,
                manifest_pointer: r.get(8)?,
            })
        },
    )
    .map_err(|error| match error {
        rusqlite::Error::QueryReturnedNoRows => {
            DbError::SelfTestFailed("vault_metadata row is missing".to_owned())
        }
        other => other.into(),
    })
}

/// Apply one command together with its provenance link, operation-log row, and
/// idempotency memo — all inside a single transaction (plan §2.6, §9.1.2). A
/// panic at any point drops the transaction, rolling back the whole set. The
/// `fault` parameter is `Fault::None` outside tests.
fn apply_command(
    inner: &mut Inner,
    meta: &CommandMeta,
    cmd: &WriteCommand,
    node_id: Uuid,
    schema_version: i64,
    fault: Fault,
) -> Result<Outcome, DbError> {
    // Advance the HLC before borrowing the connection for the transaction.
    let hlc = inner.next_hlc();
    let tx = inner.conn.transaction()?;

    // 1. Domain mutation. Each arm yields the affected entity id for provenance.
    let entity_id = match cmd {
        WriteCommand::CreateAccount {
            account,
            opening_balance,
        } => apply::accounts::apply_create_account(&tx, meta, account, opening_balance)?,
        WriteCommand::UpdateAccount { id, name } => {
            apply::accounts::apply_update_account(&tx, id, name)?
        }
        WriteCommand::ArchiveAccount(id) => apply::accounts::apply_archive_account(&tx, id)?,
        WriteCommand::ReinstateAccount(id) => apply::accounts::apply_reinstate_account(&tx, id)?,
        WriteCommand::SetAccountSubtype { id, subtype } => {
            apply::accounts::apply_set_account_subtype(&tx, id, subtype)?
        }
        WriteCommand::SetAccountNote { id, note } => {
            apply::accounts::apply_set_account_note(&tx, id, note.as_deref())?
        }
        WriteCommand::SetAccountLink {
            asset_id,
            liability_id,
        } => apply::accounts::apply_set_account_link(&tx, asset_id, liability_id.as_ref())?,
        WriteCommand::SetDebtTerms { account_id, terms } => {
            debt::apply_set_debt_terms(&tx, *account_id, *terms)?;
            account_id.as_uuid()
        }
        WriteCommand::SetCardStatementBalance {
            account_id,
            cycle_close,
            statement_balance_minor,
        } => {
            debt::apply_set_card_statement_balance(
                &tx,
                *account_id,
                *cycle_close,
                *statement_balance_minor,
            )?;
            account_id.as_uuid()
        }
        WriteCommand::RecordTransaction {
            transaction_id,
            account_id,
            amount,
            occurred_at,
        } => apply::transactions::apply_record_transaction(
            &tx,
            meta,
            transaction_id,
            account_id,
            amount,
            occurred_at,
        )?,
        WriteCommand::ConfirmObligationEarly {
            recurring_event_id,
            scheduled_date,
            actual_amount,
            actual_date,
            paying_account_id,
        } => apply::transactions::apply_confirm_obligation_early(
            &tx,
            meta,
            recurring_event_id,
            scheduled_date,
            actual_amount,
            actual_date,
            paying_account_id,
        )?,
        WriteCommand::UnconfirmObligation {
            recurring_event_id,
            scheduled_date,
        } => apply::transactions::apply_unconfirm_obligation(
            &tx,
            meta,
            recurring_event_id,
            scheduled_date,
        )?,
        WriteCommand::ConvertUnexplainedToTransaction { account_id } => {
            apply::transactions::apply_convert_unexplained_to_transaction(&tx, meta, account_id)?
        }
        WriteCommand::Transfer {
            source_account_id,
            dest_account_id,
            amount,
            occurred_at,
        } => apply::transactions::apply_transfer(
            &tx,
            meta,
            source_account_id,
            dest_account_id,
            amount,
            occurred_at,
        )?,
        WriteCommand::CreateRecurringTransfer {
            id,
            source_account_id,
            dest_account_id,
            amount,
            frequency,
            anchor,
        } => apply::recurring::apply_create_recurring_transfer(
            &tx,
            id,
            source_account_id,
            dest_account_id,
            amount,
            frequency,
            anchor,
        )?,
        WriteCommand::DeleteRecurringTransfer(id) => {
            apply::recurring::apply_delete_recurring_transfer(&tx, id)?
        }
        WriteCommand::CreateIncomeSource {
            id,
            name,
            net_amount,
            frequency,
            anchor,
            deposit_account_id,
        } => apply::recurring::apply_create_income_source(
            &tx,
            id,
            name,
            net_amount,
            frequency,
            anchor,
            deposit_account_id,
        )?,
        WriteCommand::UpdateIncomeSource {
            id,
            name,
            net_amount,
            frequency,
            anchor,
            deposit_account_id,
        } => apply::recurring::apply_update_income_source(
            &tx,
            id,
            name,
            net_amount,
            frequency,
            anchor,
            deposit_account_id,
        )?,
        WriteCommand::DeleteIncomeSource(id) => {
            apply::recurring::apply_delete_income_source(&tx, id)?
        }
        WriteCommand::ArchiveIncomeSource(id) => {
            apply::recurring::apply_archive_income_source(&tx, id)?
        }
        WriteCommand::RestoreIncomeSource(id) => {
            apply::recurring::apply_restore_income_source(&tx, id)?
        }
        WriteCommand::CreateRecurringBill {
            event_id,
            contract_id,
            name,
            amount,
            bill_type,
            frequency,
            anchor,
            autopay_account_id,
            description,
            source_merchant_key,
            category_id,
            tag_ids,
        } => apply::recurring::apply_create_recurring_bill(
            &tx,
            event_id,
            contract_id,
            name,
            amount,
            bill_type,
            frequency,
            anchor,
            autopay_account_id,
            description,
            source_merchant_key,
            category_id,
            tag_ids,
        )?,
        WriteCommand::SetBillAutopay { event_id, autopay } => {
            apply::recurring::apply_set_bill_autopay(&tx, event_id, autopay)?
        }
        WriteCommand::UpdateRecurringBill {
            event_id,
            name,
            amount,
            bill_type,
            frequency,
            anchor,
            autopay_account_id,
            description,
        } => apply::recurring::apply_update_recurring_bill(
            &tx,
            event_id,
            name,
            amount,
            bill_type,
            frequency,
            anchor,
            autopay_account_id,
            description,
        )?,
        WriteCommand::DeleteRecurringBill { event_id } => {
            apply::recurring::apply_delete_recurring_bill(&tx, event_id)?
        }
        WriteCommand::ArchiveRecurringBill(event_id) => {
            apply::recurring::apply_archive_recurring_bill(&tx, event_id)?
        }
        WriteCommand::RestoreRecurringBill(event_id) => {
            apply::recurring::apply_restore_recurring_bill(&tx, event_id)?
        }
        WriteCommand::CreateSourceBatch {
            id,
            source_type,
            source_name,
            file_fingerprint,
            parser_version,
        } => apply::ingestion_cmds::apply_create_source_batch(
            &tx,
            id,
            source_type,
            source_name,
            file_fingerprint,
            parser_version,
        )?,
        WriteCommand::AttachSourceRecord {
            id,
            batch_id,
            external_id,
            source_hash,
            normalized_json,
            parse_confidence_bps,
        } => ingestion::insert_source_record(
            &tx,
            id.as_uuid(),
            &ingestion::NewSourceRecord {
                source_batch_id: batch_id.as_uuid(),
                external_id: external_id.as_deref(),
                source_hash: source_hash.as_str(),
                normalized_json: normalized_json.as_str(),
                parse_confidence_bps: *parse_confidence_bps,
            },
        )?,
        WriteCommand::UpdateBatchState {
            batch_id,
            status,
            staged_count,
            committed_count,
            skipped_count,
        } => apply::ingestion_cmds::apply_update_batch_state(
            &tx,
            batch_id,
            status,
            staged_count,
            committed_count,
            skipped_count,
        )?,
        WriteCommand::CommitStaged {
            staged_transaction_id,
            force,
        } => apply::ingestion_cmds::apply_commit_staged(&tx, meta, staged_transaction_id, force)?,
        WriteCommand::SkipStaged {
            staged_transaction_id,
        } => apply::ingestion_cmds::apply_skip_staged(&tx, staged_transaction_id)?,
        WriteCommand::SnoozeInboxItem { item_id, until } => {
            apply::inbox::apply_snooze_inbox_item(&tx, item_id, until)?
        }
        WriteCommand::DismissInboxItem { item_id, reason } => {
            apply::inbox::apply_dismiss_inbox_item(&tx, item_id, reason)?
        }
        WriteCommand::DismissRecurringSuggestion {
            merchant_key,
            currency,
            amount_minor,
            frequency,
            reason,
        } => apply::recurring::apply_dismiss_recurring_suggestion(
            &tx,
            merchant_key,
            currency,
            *amount_minor,
            frequency,
            reason,
        )?,
        WriteCommand::CreateCategory {
            id,
            parent_id,
            name,
            category_type,
            color,
            icon,
        } => apply::categorization_cmds::apply_create_category(
            &tx,
            id,
            parent_id,
            name,
            category_type,
            color,
            icon,
        )?,
        WriteCommand::UpdateCategory {
            id,
            name,
            color,
            icon,
        } => apply::categorization_cmds::apply_update_category(&tx, id, name, color, icon)?,
        WriteCommand::MoveCategory { id, new_parent_id } => {
            apply::categorization_cmds::apply_move_category(&tx, id, new_parent_id)?
        }
        WriteCommand::ArchiveCategory(id) => {
            apply::categorization_cmds::apply_archive_category(&tx, id)?
        }
        WriteCommand::ReinstateCategory(id) => {
            apply::categorization_cmds::apply_reinstate_category(&tx, id)?
        }
        WriteCommand::RecategorizeTransaction {
            transaction_id,
            category_id,
        } => apply::categorization_cmds::apply_recategorize_transaction(
            &tx,
            transaction_id,
            category_id,
        )?,
        WriteCommand::VoidTransaction { transaction_id } => {
            apply::transactions::apply_void_transaction(&tx, meta, transaction_id)?
        }
        WriteCommand::MarkReviewed {
            transaction_id,
            reviewed,
        } => apply::transactions::apply_mark_reviewed(&tx, transaction_id, reviewed)?,
        // The command_id is stored on the scenario as the reversal handle (ADR 0055 §3),
        // which is why apply is on this bus at all.
        WriteCommand::ApplyScenario { scenario_id } => apply::scenario_apply::apply_scenario(
            &tx,
            *scenario_id,
            meta.command_id,
            &Utc::now().to_rfc3339(),
        )?,
        WriteCommand::RevertScenarioApply { scenario_id } => {
            apply::scenario_apply::revert_scenario_apply(
                &tx,
                *scenario_id,
                &Utc::now().to_rfc3339(),
            )?
        }
        WriteCommand::CreateTag { id, name, color } => {
            apply::transactions::apply_create_tag(&tx, id, name, color)?
        }
        WriteCommand::SetTags {
            transaction_id,
            tag_ids,
        } => apply::transactions::apply_set_tags(&tx, transaction_id, tag_ids)?,
        WriteCommand::SetNote {
            transaction_id,
            note,
        } => apply::transactions::apply_set_note(&tx, transaction_id, note)?,
        WriteCommand::SetSplits {
            transaction_id,
            lines,
        } => apply::transactions::apply_set_splits(&tx, transaction_id, lines)?,
    };

    fault.maybe_panic(FaultPoint::AfterMutation);

    // 2. Operation-log row (full §9.1.2 / ADR 0011 metadata, system-stamped).
    //    Provenance UUIDs bind as BLOBs; affected_entities is JSON bytes. Most
    //    commands affect an account row; income-source creation affects its own
    //    table.
    let entity_table = match cmd {
        WriteCommand::CreateIncomeSource { .. }
        | WriteCommand::UpdateIncomeSource { .. }
        | WriteCommand::DeleteIncomeSource(_)
        | WriteCommand::ArchiveIncomeSource(_)
        | WriteCommand::RestoreIncomeSource(_) => "income_sources",
        WriteCommand::CreateRecurringBill { .. }
        | WriteCommand::UpdateRecurringBill { .. }
        | WriteCommand::DeleteRecurringBill { .. }
        | WriteCommand::ArchiveRecurringBill(_)
        | WriteCommand::RestoreRecurringBill(_) => "recurring_events",
        WriteCommand::CreateSourceBatch { .. } | WriteCommand::UpdateBatchState { .. } => {
            "source_batches"
        }
        WriteCommand::AttachSourceRecord { .. } => "source_records",
        // Apply/revert affect the scenario row itself — the promoted assumption events
        // are recoverable from it via `promoted_from_scenario_id` (ADR 0055).
        WriteCommand::ApplyScenario { .. } | WriteCommand::RevertScenarioApply { .. } => {
            "scenarios"
        }
        // The committed case affects a ledger_transaction; a flagged duplicate
        // (rare) affects only the staged row — the authoritative record there is
        // the dedupe_decisions entry, not this op-log hint.
        WriteCommand::CommitStaged { .. } => "ledger_transactions",
        WriteCommand::RecategorizeTransaction { .. } => "ledger_transactions",
        WriteCommand::VoidTransaction { .. } => "ledger_transactions",
        WriteCommand::MarkReviewed { .. } => "transaction_reviews",
        WriteCommand::CreateTag { .. } => "tags",
        WriteCommand::SetTags { .. } => "transaction_tags",
        WriteCommand::SetNote { .. } => "transaction_details",
        WriteCommand::SetSplits { .. } => "split_lines",
        WriteCommand::CreateRecurringTransfer { .. } | WriteCommand::DeleteRecurringTransfer(_) => {
            "recurring_transfers"
        }
        WriteCommand::SkipStaged { .. } => "staged_transactions",
        WriteCommand::SnoozeInboxItem { .. } | WriteCommand::DismissInboxItem { .. } => {
            "money_inbox_read_model"
        }
        WriteCommand::DismissRecurringSuggestion { .. } => "recurring_suggestion_suppressions",
        WriteCommand::SetCardStatementBalance { .. } => "credit_card_statements",
        WriteCommand::CreateCategory { .. }
        | WriteCommand::UpdateCategory { .. }
        | WriteCommand::MoveCategory { .. }
        | WriteCommand::ArchiveCategory(_)
        | WriteCommand::ReinstateCategory(_) => "categories",
        // Account-affecting commands, including the postings paths (a recorded
        // transaction / transfer / early-confirm moves an account's balance) and
        // SetBillAutopay, which keep their historical "accounts" hint. Exhaustive
        // on purpose (personal-cfo-3fdd.4): a new WriteCommand variant must pick
        // its op-log entity table here or it will not compile.
        WriteCommand::CreateAccount { .. }
        | WriteCommand::UpdateAccount { .. }
        | WriteCommand::ArchiveAccount(_)
        | WriteCommand::ReinstateAccount(_)
        | WriteCommand::SetAccountSubtype { .. }
        | WriteCommand::SetAccountNote { .. }
        | WriteCommand::SetAccountLink { .. }
        | WriteCommand::SetDebtTerms { .. }
        | WriteCommand::RecordTransaction { .. }
        | WriteCommand::ConfirmObligationEarly { .. }
        | WriteCommand::UnconfirmObligation { .. }
        | WriteCommand::ConvertUnexplainedToTransaction { .. }
        | WriteCommand::Transfer { .. }
        | WriteCommand::SetBillAutopay { .. } => "accounts",
    };
    let affected_entities = affected_entities_json(entity_table, entity_id);
    tx.execute(
        "INSERT INTO operation_log (
            command_id, correlation_id, causation_id, actor_type, actor_id,
            idempotency_key, operation_type, affected_entities, metadata, node_id,
            vault_schema_version, hlc_timestamp, created_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            meta.command_id,
            meta.correlation_id,
            meta.causation_id,
            actor_type_str(meta.actor_type),
            meta.actor_id,
            meta.idempotency_key,
            cmd.kind(),
            affected_entities,
            Option::<Vec<u8>>::None,
            node_id,
            schema_version,
            hlc,
            Utc::now().to_rfc3339(),
        ],
    )?;
    let op_seq = tx.last_insert_rowid();

    // 3. Provenance link from the affected entity to this operation.
    tx.execute(
        "INSERT INTO provenance_links (op_seq, entity_type, entity_id) VALUES (?1, ?2, ?3)",
        params![op_seq, "account", entity_id],
    )?;

    // 4. Idempotency memo so a retry with this key returns the same result.
    let result_ref_json = format!("{{\"op_seq\":{op_seq}}}");
    let expires_at = (Utc::now() + chrono::Duration::days(IDEMPOTENCY_TTL_DAYS)).to_rfc3339();
    tx.execute(
        "INSERT INTO command_idempotency_keys (
            idempotency_key, command_id, command_type, result_ref_json, op_seq, expires_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            meta.idempotency_key,
            meta.command_id,
            cmd.kind(),
            result_ref_json,
            op_seq,
            expires_at,
        ],
    )?;

    fault.maybe_panic(FaultPoint::AfterOpLog);

    tx.commit()?;
    Ok(Outcome::Applied { op_seq })
}

#[cfg(test)]
mod tests;
