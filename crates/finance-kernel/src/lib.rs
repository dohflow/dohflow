//! The Finance Kernel: the single internal boundary for all financial state
//! changes (plan §2.6, §9.1.2; ADR 0006).
//!
//! Every mutation enters through [`Kernel::dispatch`] as a typed, **sealed**
//! [`KernelCommand`] wrapped in a [`CommandEnvelope`] carrying §9.1.2 provenance
//! metadata. The kernel validates the command, emits a tracing span at the
//! boundary, lowers it onto a [`db_worker`] write command, and lets the
//! db-worker apply it atomically with its operation-log entry.
//!
//! Boundaries this crate holds:
//! - **Sealed command set.** `KernelCommand` cannot be implemented outside this
//!   crate, so the frontend cannot inject arbitrary commands — they must come
//!   through the typed commands defined here (and, in production, the Tauri IPC
//!   layer that builds them).
//! - **No leaked database types.** `rusqlite` does not appear anywhere in this
//!   crate's public API (or its dependencies); reads go through typed kernel
//!   methods that return plain domain values.
//!
//! Commands so far cover the account lifecycle ([`CreateAccount`] with optional
//! opening-balance equity posting, [`UpdateAccount`], [`ArchiveAccount`],
//! [`ReinstateAccount`]); further commands arrive with their feature beads.

use std::path::Path;

use chrono::{DateTime, NaiveDate, Utc};
use thiserror::Error;

pub use core_ledger::{
    Account, AccountFlags, AccountId, AccountKind, AccountSubtype, AttachmentId, BillContractId,
    CashflowRole, CategoryId, IncomeSourceId, LedgerAccount, LedgerAccountId, NormalBalance,
    RecurringEventId, RecurringTransferId, SourceBatchId, SourceRecordId, SplitLineId,
    StagedTransactionId, TagId, TransactionId,
};
pub use core_money::{Currency, Money};
pub use db_worker::{
    sqlite_version, AccountAvailability, AccountHistoryView, AccountSeriesView, AccountView,
    ActorType, AssumptionBasis, AssumptionEventView, AssumptionParams, AttachmentMeta, Band,
    BandDriftView, CandidateObservation, CapabilityUnlock, CardCycleView,
    CardStatementForecastView, CardStatementHistoryView, CashAvailability, CashFlowHistory,
    CashTiers, CategoryFilter, CategorySource, CategorySpend, CategoryView, ComfortBand,
    CommandMeta, CommitmentView, ConnectorConnectionRow, ConnectorLinkRow, DayBalance,
    DebtTermsInput, DebtTermsView, DriftFactorView, ForecastAssumptionSpec, ForecastDayView,
    ForecastEventView, ForecastReadiness, ForecastView, GroupSeriesView, HistoryDay,
    ImportedTransactionFields, IncomeSourceView, LoanDoubleCount, ManualEntry, MoneyInboxItem,
    MultiSeriesForecast, NewScenario, Outcome, PayoffDebtSeries, PayoffPlanView, ReadinessFactor,
    RecurringBillView, RecurringCandidateView, RecurringInstanceRow, RecurringTransferView,
    RepaymentPhilosophy, ReviewStatus, ScenarioStatus, ScenarioView, SpendBreakdown, SpendFilters,
    SplitLineInput, SplitLineView, TagView, TransactionDisplayRow, TransactionPage,
    TransactionPageQuery, TransactionRow, TransactionSortOrder, UnconfirmedOccurrence,
    VaultMetadata, WorkerState, AUTO_CATEGORIZE_ON_IMPORT_KEY, COMFORT_BAND_UPPER_KEY,
    CURRENT_SCHEMA_VERSION, FUTURE_CASH_SERIES_KEY, MINIMUM_CASH_FLOOR_KEY, REPORTING_CURRENCY_KEY,
};
pub use importer_core::{
    all_presets, content_fingerprint, detect_best, plugin_by_id, preset_by_id, run_bounded,
    AccountHandling, CategoryHandling, ColumnMapping, ImporterPlugin, ParseError, ParseWarning,
    ParsedAccount, ParsedBalance, ParsedBatch, ParsedRecord, ParsedTransaction, ParserHints,
    ParserInput, ParserLimits, ParserRunReport, RunStatus, SignConvention, SourcePreset,
    SourceQuirk,
};
pub use pay_schedule::{Frequency, PaySchedule};
// Canonical onboarding warning (ADR 0002 / personal-cfo-n7bo): re-exported so the
// IPC layer surfaces the single source of truth without depending on vault-crypto.
pub use vault_crypto::CANONICAL_NO_RESET_WARNING;

/// Audit-event type for the onboarding no-reset-warning acknowledgement
/// (personal-cfo-n7bo), stored in `audit_events.event_type`.
pub const NO_RESET_WARNING_ACKNOWLEDGED: &str = "no_reset_warning_acknowledged";

use db_worker::{DbError, DbWorker, WriteCommand};
use uuid::Uuid;

mod vault;
pub use vault::{classify_vault, VaultController, VaultHealth, VaultState};

pub mod backup;

mod sealed {
    /// Private supertrait that seals [`KernelCommand`](super::KernelCommand):
    /// only types in this crate can name it, so only this crate can implement
    /// the public trait.
    pub trait Sealed {}
}

/// A typed, validated financial command.
///
/// `KernelCommand` is **sealed** — it cannot be implemented outside this crate.
/// That keeps the set of mutations closed and auditable and prevents the
/// frontend from constructing arbitrary commands.
///
/// ```compile_fail
/// // A type outside this crate cannot implement the sealed trait:
/// use finance_kernel::KernelCommand;
/// struct Rogue;
/// impl KernelCommand for Rogue {
///     fn kind(&self) -> &'static str { "rogue" }
/// }
/// ```
pub trait KernelCommand: sealed::Sealed {
    /// A stable, non-sensitive identifier for the command kind (used in spans
    /// and the op-log).
    fn kind(&self) -> &'static str;

    /// Validate domain invariants before the command touches persistence.
    ///
    /// # Errors
    /// Returns [`KernelError::Validation`] if the command is not valid.
    #[doc(hidden)]
    fn validate(&self) -> Result<(), KernelError>;

    /// Lower this kernel command onto a db-worker write command.
    #[doc(hidden)]
    fn lower(self) -> WriteCommand;
}

/// A command plus the §9.1.2 provenance metadata required to apply it.
#[derive(Debug, Clone)]
pub struct CommandEnvelope<C: KernelCommand> {
    /// Provenance + idempotency metadata.
    pub meta: CommandMeta,
    /// The typed command.
    pub command: C,
}

impl<C: KernelCommand> CommandEnvelope<C> {
    /// Pair a command with its metadata.
    pub fn new(meta: CommandMeta, command: C) -> Self {
        Self { meta, command }
    }
}

/// Create a user account, optionally with an opening balance (recorded as an
/// equity posting, never a column).
#[derive(Debug, Clone)]
pub struct CreateAccount {
    account: Account,
    opening_balance: Option<Money>,
}

impl CreateAccount {
    /// Create an account with no opening balance.
    #[must_use]
    pub fn new(account: Account) -> Self {
        Self {
            account,
            opening_balance: None,
        }
    }

    /// Create an account with an opening balance (currency must match the
    /// account's currency).
    #[must_use]
    pub fn with_opening_balance(account: Account, opening_balance: Money) -> Self {
        Self {
            account,
            opening_balance: Some(opening_balance),
        }
    }
}

impl sealed::Sealed for CreateAccount {}

impl KernelCommand for CreateAccount {
    fn kind(&self) -> &'static str {
        "create_account"
    }

    fn validate(&self) -> Result<(), KernelError> {
        if let Some(opening) = self.opening_balance {
            if opening.currency() != self.account.currency() {
                return Err(KernelError::Validation(
                    "opening balance currency must match the account currency".to_owned(),
                ));
            }
        }
        // A subtype must belong to the account's cashflow role (ADR 0028) — the
        // schema CHECK only constrains the token set, not the role match.
        if let Some(subtype) = self.account.subtype() {
            if subtype.role() != self.account.cashflow_role() {
                return Err(KernelError::Validation(format!(
                    "account subtype '{}' does not belong to this account type",
                    subtype.as_str()
                )));
            }
        }
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::CreateAccount {
            account: Box::new(self.account),
            opening_balance: self.opening_balance,
        }
    }
}

/// Record a manual transaction: a signed money movement against an account,
/// balanced by a system income/expense counter-account (personal-cfo-6wgi).
#[derive(Debug, Clone, Copy)]
pub struct RecordTransaction {
    /// The id the new ledger transaction is persisted under — supplied by the caller
    /// (personal-cfo-4d8.24.2.1) so it can be surfaced back (the inline add-transaction
    /// flow attaches category/tags/notes to this id once the record succeeds).
    transaction_id: TransactionId,
    account_id: AccountId,
    amount: Money,
    occurred_at: DateTime<Utc>,
}

impl RecordTransaction {
    /// Build a `RecordTransaction`. `amount` is signed: positive is money in
    /// (income), negative is money out (expense); its currency must match the
    /// account's. `transaction_id` is the caller-minted id the transaction is stored
    /// under (mint one with [`TransactionId::new`] and keep it to reference the row).
    #[must_use]
    pub const fn new(
        transaction_id: TransactionId,
        account_id: AccountId,
        amount: Money,
        occurred_at: DateTime<Utc>,
    ) -> Self {
        Self {
            transaction_id,
            account_id,
            amount,
            occurred_at,
        }
    }
}

impl sealed::Sealed for RecordTransaction {}

impl KernelCommand for RecordTransaction {
    fn kind(&self) -> &'static str {
        "record_transaction"
    }

    fn validate(&self) -> Result<(), KernelError> {
        if self.amount.is_zero() {
            return Err(KernelError::Validation(
                "transaction amount must be non-zero".to_owned(),
            ));
        }
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::RecordTransaction {
            transaction_id: self.transaction_id,
            account_id: self.account_id,
            amount: self.amount,
            occurred_at: self.occurred_at,
        }
    }
}

/// Record a one-off transfer between two of the user's own accounts
/// (personal-cfo-npoe): a balanced debit-source / credit-destination ledger
/// transaction, aggregate cash unchanged. The db-worker validates the accounts
/// exist, are distinct liquid-cash accounts, and share `amount`'s currency.
#[derive(Debug, Clone, Copy)]
pub struct Transfer {
    source_account_id: AccountId,
    dest_account_id: AccountId,
    amount: Money,
    occurred_at: DateTime<Utc>,
}

impl Transfer {
    /// Build a `Transfer` of `amount` (positive) from `source` to `dest`.
    #[must_use]
    pub const fn new(
        source_account_id: AccountId,
        dest_account_id: AccountId,
        amount: Money,
        occurred_at: DateTime<Utc>,
    ) -> Self {
        Self {
            source_account_id,
            dest_account_id,
            amount,
            occurred_at,
        }
    }
}

impl sealed::Sealed for Transfer {}

impl KernelCommand for Transfer {
    fn kind(&self) -> &'static str {
        "transfer"
    }

    fn validate(&self) -> Result<(), KernelError> {
        if self.source_account_id == self.dest_account_id {
            return Err(KernelError::Validation(
                "a transfer needs two different accounts".to_owned(),
            ));
        }
        if self.amount.is_zero() || self.amount.minor_units() < 0 {
            return Err(KernelError::Validation(
                "transfer amount must be positive".to_owned(),
            ));
        }
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::Transfer {
            source_account_id: self.source_account_id,
            dest_account_id: self.dest_account_id,
            amount: self.amount,
            occurred_at: self.occurred_at,
        }
    }
}

/// Delete a transaction by voiding it (personal-cfo-4d8.11, ADR 0007 §9). The
/// db-worker posts a reversing entry and hides both the original and the reversal, so
/// balances + forecast update as if it never happened while the ledger stays
/// append-only and auditable. The db-worker rejects an unknown or already-voided id.
#[derive(Debug, Clone, Copy)]
pub struct VoidTransaction {
    transaction_id: TransactionId,
}

impl VoidTransaction {
    /// Void (delete) the transaction `transaction_id`.
    #[must_use]
    pub const fn new(transaction_id: TransactionId) -> Self {
        Self { transaction_id }
    }
}

impl sealed::Sealed for VoidTransaction {}

impl KernelCommand for VoidTransaction {
    fn kind(&self) -> &'static str {
        "void_transaction"
    }

    fn validate(&self) -> Result<(), KernelError> {
        // Existence + not-already-voided are checked in the db-worker apply, which
        // holds the row.
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::VoidTransaction {
            transaction_id: self.transaction_id,
        }
    }
}

/// Promote a scenario's active assumption events into base (ADR 0055 §1,
/// personal-cfo-4d8.27.6.3).
///
/// On the command bus rather than the direct assumption-write path, exceptionally: this
/// is the one assumption-layer operation that changes what the household's real forecast
/// says, and the operation id it earns is stored as the reversal handle (ADR 0055 §3).
/// Existence, already-applied, and has-active-events are checked in the db-worker apply,
/// which holds the rows.
#[derive(Debug, Clone, Copy)]
pub struct ApplyScenario {
    scenario_id: Uuid,
}

impl ApplyScenario {
    /// Apply `scenario_id` onto base.
    #[must_use]
    pub const fn new(scenario_id: Uuid) -> Self {
        Self { scenario_id }
    }
}

impl sealed::Sealed for ApplyScenario {}

impl KernelCommand for ApplyScenario {
    fn kind(&self) -> &'static str {
        "apply_scenario"
    }

    fn validate(&self) -> Result<(), KernelError> {
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::ApplyScenario {
            scenario_id: self.scenario_id,
        }
    }
}

/// Undo an [`ApplyScenario`] (ADR 0055 §5): clear the promoted base events and restore
/// whatever they superseded. Reverting a scenario that is not applied is an error, not a
/// no-op — checked in the db-worker apply.
#[derive(Debug, Clone, Copy)]
pub struct RevertScenarioApply {
    scenario_id: Uuid,
}

impl RevertScenarioApply {
    /// Revert the apply of `scenario_id`.
    #[must_use]
    pub const fn new(scenario_id: Uuid) -> Self {
        Self { scenario_id }
    }
}

impl sealed::Sealed for RevertScenarioApply {}

impl KernelCommand for RevertScenarioApply {
    fn kind(&self) -> &'static str {
        "revert_scenario_apply"
    }

    fn validate(&self) -> Result<(), KernelError> {
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::RevertScenarioApply {
            scenario_id: self.scenario_id,
        }
    }
}

/// Mark a transaction reviewed or unreviewed (personal-cfo-4d8.7, ADR 0032 §2). Records
/// the user's explicit override; the default (no override) is import = unreviewed,
/// manual = reviewed. The db-worker rejects an unknown transaction id.
#[derive(Debug, Clone, Copy)]
pub struct MarkReviewed {
    transaction_id: TransactionId,
    reviewed: bool,
}

impl MarkReviewed {
    /// Mark `transaction_id` reviewed (`true`) or unreviewed (`false`).
    #[must_use]
    pub const fn new(transaction_id: TransactionId, reviewed: bool) -> Self {
        Self {
            transaction_id,
            reviewed,
        }
    }
}

impl sealed::Sealed for MarkReviewed {}

impl KernelCommand for MarkReviewed {
    fn kind(&self) -> &'static str {
        "mark_reviewed"
    }

    fn validate(&self) -> Result<(), KernelError> {
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::MarkReviewed {
            transaction_id: self.transaction_id,
            reviewed: self.reviewed,
        }
    }
}

/// Convert an account's unexplained balance adjustment into one real transaction
/// (ADR 0027 §8, personal-cfo-dyy4) — the residual plug becomes a ledger posting
/// dated at the latest assertion, so the plug goes to zero. The db-worker computes
/// the residual + date; a no-op (rejected) when the balance is already explained.
#[derive(Debug, Clone, Copy)]
pub struct ConvertUnexplainedToTransaction {
    account_id: AccountId,
}

impl ConvertUnexplainedToTransaction {
    /// Build a `ConvertUnexplainedToTransaction` for `account_id`.
    #[must_use]
    pub const fn new(account_id: AccountId) -> Self {
        Self { account_id }
    }
}

impl sealed::Sealed for ConvertUnexplainedToTransaction {}

impl KernelCommand for ConvertUnexplainedToTransaction {
    fn kind(&self) -> &'static str {
        "convert_unexplained_to_transaction"
    }

    fn validate(&self) -> Result<(), KernelError> {
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::ConvertUnexplainedToTransaction {
            account_id: self.account_id,
        }
    }
}

/// Create a recurring account-to-account transfer (ADR 0026 §14, personal-cfo-npoe):
/// a scheduled money movement projected per-account (source −, destination +). The
/// db-worker validates the accounts are distinct liquid-cash accounts sharing the
/// amount's currency.
#[derive(Debug, Clone, Copy)]
pub struct CreateRecurringTransfer {
    id: RecurringTransferId,
    source_account_id: AccountId,
    dest_account_id: AccountId,
    amount: Money,
    frequency: Frequency,
    anchor: NaiveDate,
}

impl CreateRecurringTransfer {
    /// Build a `CreateRecurringTransfer`. The caller mints `id`; `amount` is the
    /// positive amount moved each occurrence.
    #[must_use]
    pub const fn new(
        id: RecurringTransferId,
        source_account_id: AccountId,
        dest_account_id: AccountId,
        amount: Money,
        frequency: Frequency,
        anchor: NaiveDate,
    ) -> Self {
        Self {
            id,
            source_account_id,
            dest_account_id,
            amount,
            frequency,
            anchor,
        }
    }
}

impl sealed::Sealed for CreateRecurringTransfer {}

impl KernelCommand for CreateRecurringTransfer {
    fn kind(&self) -> &'static str {
        "create_recurring_transfer"
    }

    fn validate(&self) -> Result<(), KernelError> {
        if self.source_account_id == self.dest_account_id {
            return Err(KernelError::Validation(
                "a transfer needs two different accounts".to_owned(),
            ));
        }
        if self.amount.is_zero() || self.amount.minor_units() < 0 {
            return Err(KernelError::Validation(
                "transfer amount must be positive".to_owned(),
            ));
        }
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::CreateRecurringTransfer {
            id: self.id,
            source_account_id: self.source_account_id,
            dest_account_id: self.dest_account_id,
            amount: self.amount,
            frequency: self.frequency,
            anchor: self.anchor,
        }
    }
}

/// Delete a recurring transfer (personal-cfo-npoe). Stops future projection; any
/// already-posted one-off transfers are untouched.
#[derive(Debug, Clone, Copy)]
pub struct DeleteRecurringTransfer {
    id: RecurringTransferId,
}

impl DeleteRecurringTransfer {
    /// Build a `DeleteRecurringTransfer` for `id`.
    #[must_use]
    pub const fn new(id: RecurringTransferId) -> Self {
        Self { id }
    }
}

impl sealed::Sealed for DeleteRecurringTransfer {}

impl KernelCommand for DeleteRecurringTransfer {
    fn kind(&self) -> &'static str {
        "delete_recurring_transfer"
    }

    fn validate(&self) -> Result<(), KernelError> {
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::DeleteRecurringTransfer(self.id)
    }
}

/// Create a recurring net-pay income source (personal-cfo-le79): a take-home
/// amount on a [`PaySchedule`] cadence, optionally deposited into an account.
#[derive(Debug, Clone)]
pub struct CreateIncomeSource {
    name: String,
    net_amount: Money,
    frequency: Frequency,
    anchor: NaiveDate,
    deposit_account_id: Option<AccountId>,
}

impl CreateIncomeSource {
    /// Build a `CreateIncomeSource`. `net_amount` is the positive take-home pay
    /// per occurrence; its currency must match the deposit account if one is set.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        net_amount: Money,
        frequency: Frequency,
        anchor: NaiveDate,
        deposit_account_id: Option<AccountId>,
    ) -> Self {
        Self {
            name: name.into(),
            net_amount,
            frequency,
            anchor,
            deposit_account_id,
        }
    }
}

impl sealed::Sealed for CreateIncomeSource {}

impl KernelCommand for CreateIncomeSource {
    fn kind(&self) -> &'static str {
        "create_income_source"
    }

    fn validate(&self) -> Result<(), KernelError> {
        if self.name.trim().is_empty() {
            return Err(KernelError::Validation(
                "income source name must not be empty".to_owned(),
            ));
        }
        if self.net_amount.is_zero() || self.net_amount.minor_units() < 0 {
            return Err(KernelError::Validation(
                "income source net amount must be positive".to_owned(),
            ));
        }
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::CreateIncomeSource {
            id: IncomeSourceId::new(),
            name: self.name,
            net_amount: self.net_amount,
            frequency: self.frequency,
            anchor: self.anchor,
            deposit_account_id: self.deposit_account_id,
        }
    }
}

/// Edit an existing net-pay income source (personal-cfo-tch0).
#[derive(Debug, Clone)]
pub struct UpdateIncomeSource {
    id: IncomeSourceId,
    name: String,
    net_amount: Money,
    frequency: Frequency,
    anchor: NaiveDate,
    deposit_account_id: Option<AccountId>,
}

impl UpdateIncomeSource {
    /// Build an `UpdateIncomeSource` command. Same field rules as create.
    #[must_use]
    pub fn new(
        id: IncomeSourceId,
        name: impl Into<String>,
        net_amount: Money,
        frequency: Frequency,
        anchor: NaiveDate,
        deposit_account_id: Option<AccountId>,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            net_amount,
            frequency,
            anchor,
            deposit_account_id,
        }
    }
}

impl sealed::Sealed for UpdateIncomeSource {}

impl KernelCommand for UpdateIncomeSource {
    fn kind(&self) -> &'static str {
        "update_income_source"
    }

    fn validate(&self) -> Result<(), KernelError> {
        if self.name.trim().is_empty() {
            return Err(KernelError::Validation(
                "income source name must not be empty".to_owned(),
            ));
        }
        if self.net_amount.is_zero() || self.net_amount.minor_units() < 0 {
            return Err(KernelError::Validation(
                "income source net amount must be positive".to_owned(),
            ));
        }
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::UpdateIncomeSource {
            id: self.id,
            name: self.name,
            net_amount: self.net_amount,
            frequency: self.frequency,
            anchor: self.anchor,
            deposit_account_id: self.deposit_account_id,
        }
    }
}

/// Delete a net-pay income source (personal-cfo-tch0).
#[derive(Debug, Clone, Copy)]
pub struct DeleteIncomeSource {
    id: IncomeSourceId,
}

impl DeleteIncomeSource {
    /// Build a `DeleteIncomeSource` command for `id`.
    #[must_use]
    pub const fn new(id: IncomeSourceId) -> Self {
        Self { id }
    }
}

impl sealed::Sealed for DeleteIncomeSource {}

impl KernelCommand for DeleteIncomeSource {
    fn kind(&self) -> &'static str {
        "delete_income_source"
    }

    fn validate(&self) -> Result<(), KernelError> {
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::DeleteIncomeSource(self.id)
    }
}

/// Archive a net-pay income source (personal-cfo-tch0): excluded from the
/// forecast, retained with its archive date, restorable.
#[derive(Debug, Clone, Copy)]
pub struct ArchiveIncomeSource {
    id: IncomeSourceId,
}

impl ArchiveIncomeSource {
    /// Build an `ArchiveIncomeSource` command for `id`.
    #[must_use]
    pub const fn new(id: IncomeSourceId) -> Self {
        Self { id }
    }
}

impl sealed::Sealed for ArchiveIncomeSource {}

impl KernelCommand for ArchiveIncomeSource {
    fn kind(&self) -> &'static str {
        "archive_income_source"
    }

    fn validate(&self) -> Result<(), KernelError> {
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::ArchiveIncomeSource(self.id)
    }
}

/// Restore a previously archived income source (personal-cfo-tch0).
#[derive(Debug, Clone, Copy)]
pub struct RestoreIncomeSource {
    id: IncomeSourceId,
}

impl RestoreIncomeSource {
    /// Build a `RestoreIncomeSource` command for `id`.
    #[must_use]
    pub const fn new(id: IncomeSourceId) -> Self {
        Self { id }
    }
}

impl sealed::Sealed for RestoreIncomeSource {}

impl KernelCommand for RestoreIncomeSource {
    fn kind(&self) -> &'static str {
        "restore_income_source"
    }

    fn validate(&self) -> Result<(), KernelError> {
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::RestoreIncomeSource(self.id)
    }
}

/// Create a manual recurring bill (personal-cfo-esmy): an expected outflow on a
/// [`PaySchedule`] cadence, optionally paid from an autopay account. Lowering it
/// writes a `recurring_event` + a linked `bill_contract` and refreshes the
/// commitments projection.
#[derive(Debug, Clone)]
pub struct CreateRecurringBill {
    event_id: RecurringEventId,
    name: String,
    amount: Money,
    bill_type: String,
    frequency: Frequency,
    anchor: NaiveDate,
    autopay_account_id: Option<AccountId>,
    description: Option<String>,
    source_merchant_key: Option<String>,
    category_id: Option<CategoryId>,
    tag_ids: Vec<TagId>,
}

impl CreateRecurringBill {
    /// Build a `CreateRecurringBill` with a caller-supplied `event_id` (so the caller can
    /// reference the new bill afterwards, e.g. to set autopay). `amount` is the positive expected
    /// outflow per occurrence; its currency must match the autopay account if one is set.
    /// `bill_type` is a validated `bill_contracts.type` token; `description` is an optional note.
    #[must_use]
    // A recurring bill legitimately carries this many fields (id + name + amount + type + schedule +
    // autopay + note); a builder-args struct would add ceremony without clarity.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        event_id: RecurringEventId,
        name: impl Into<String>,
        amount: Money,
        bill_type: impl Into<String>,
        frequency: Frequency,
        anchor: NaiveDate,
        autopay_account_id: Option<AccountId>,
        description: Option<String>,
    ) -> Self {
        Self {
            event_id,
            name: name.into(),
            amount,
            bill_type: bill_type.into(),
            frequency,
            anchor,
            autopay_account_id,
            description,
            source_merchant_key: None,
            category_id: None,
            tag_ids: Vec::new(),
        }
    }

    /// Record the normalized merchant key of the recurring candidate this bill was
    /// promoted from (personal-cfo-5n4.8), so the suggestion stays suppressed even
    /// after the bill is renamed. A blank key is treated as absent.
    #[must_use]
    pub fn with_source_merchant_key(mut self, key: Option<String>) -> Self {
        self.source_merchant_key = key.filter(|k| !k.trim().is_empty());
        self
    }

    /// Set the bill's category (personal-cfo-4d8.24.5) — the category chosen when
    /// promoting a suggestion, written to `recurring_events.category_id`.
    #[must_use]
    pub fn with_category_id(mut self, category_id: Option<CategoryId>) -> Self {
        self.category_id = category_id;
        self
    }

    /// Set the bill's tags (personal-cfo-4d8.24.5.1) — the tags chosen when promoting a
    /// suggestion, written to `recurring_event_tags`.
    #[must_use]
    pub fn with_tag_ids(mut self, tag_ids: Vec<TagId>) -> Self {
        self.tag_ids = tag_ids;
        self
    }
}

impl sealed::Sealed for CreateRecurringBill {}

impl KernelCommand for CreateRecurringBill {
    fn kind(&self) -> &'static str {
        "create_recurring_bill"
    }

    fn validate(&self) -> Result<(), KernelError> {
        if self.name.trim().is_empty() {
            return Err(KernelError::Validation(
                "recurring bill name must not be empty".to_owned(),
            ));
        }
        if self.amount.is_zero() || self.amount.minor_units() < 0 {
            return Err(KernelError::Validation(
                "recurring bill amount must be positive".to_owned(),
            ));
        }
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::CreateRecurringBill {
            event_id: self.event_id,
            contract_id: BillContractId::new(),
            name: self.name,
            amount: self.amount,
            bill_type: self.bill_type,
            frequency: self.frequency,
            anchor: self.anchor,
            autopay_account_id: self.autopay_account_id,
            description: self.description,
            source_merchant_key: self.source_merchant_key,
            category_id: self.category_id,
            tag_ids: self.tag_ids,
        }
    }
}

/// Set whether a recurring bill autopays (ADR 0041, personal-cfo-mc7f). Autopay is intent
/// metadata: it does NOT change what the forecast projects (autopay bills still project as
/// outflows on their due date) — it drives the UI distinction (badge + "confirm it cleared" vs
/// "mark paid"). Orthogonal to the autopay *account* ("which account"). Keyed by recurring-event id.
#[derive(Debug, Clone, Copy)]
pub struct SetBillAutopay {
    event_id: RecurringEventId,
    autopay: bool,
}

impl SetBillAutopay {
    /// Mark the bill `event_id` as autopay (`true`) or manual (`false`).
    #[must_use]
    pub const fn new(event_id: RecurringEventId, autopay: bool) -> Self {
        Self { event_id, autopay }
    }
}

impl sealed::Sealed for SetBillAutopay {}

impl KernelCommand for SetBillAutopay {
    fn kind(&self) -> &'static str {
        "set_bill_autopay"
    }

    fn validate(&self) -> Result<(), KernelError> {
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::SetBillAutopay {
            event_id: self.event_id,
            autopay: self.autopay,
        }
    }
}

/// Edit an existing manual recurring bill (personal-cfo-zl1l): change any of its
/// fields — name, amount, type, schedule (frequency + anchor), autopay account, or
/// description — keyed by its recurring-event id. The commitments projection is
/// refreshed in the same write.
#[derive(Debug, Clone)]
pub struct UpdateRecurringBill {
    event_id: RecurringEventId,
    name: String,
    amount: Money,
    bill_type: String,
    frequency: Frequency,
    anchor: NaiveDate,
    autopay_account_id: Option<AccountId>,
    description: Option<String>,
}

impl UpdateRecurringBill {
    /// Build an `UpdateRecurringBill`. `event_id` is the bill's recurring-event id
    /// (the `id` on its `RecurringBillView`); `amount` is the positive expected
    /// outflow per occurrence, its currency matching the autopay account if set.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        event_id: RecurringEventId,
        name: impl Into<String>,
        amount: Money,
        bill_type: impl Into<String>,
        frequency: Frequency,
        anchor: NaiveDate,
        autopay_account_id: Option<AccountId>,
        description: Option<String>,
    ) -> Self {
        Self {
            event_id,
            name: name.into(),
            amount,
            bill_type: bill_type.into(),
            frequency,
            anchor,
            autopay_account_id,
            description,
        }
    }
}

impl sealed::Sealed for UpdateRecurringBill {}

impl KernelCommand for UpdateRecurringBill {
    fn kind(&self) -> &'static str {
        "update_recurring_bill"
    }

    fn validate(&self) -> Result<(), KernelError> {
        if self.name.trim().is_empty() {
            return Err(KernelError::Validation(
                "recurring bill name must not be empty".to_owned(),
            ));
        }
        if self.amount.is_zero() || self.amount.minor_units() < 0 {
            return Err(KernelError::Validation(
                "recurring bill amount must be positive".to_owned(),
            ));
        }
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::UpdateRecurringBill {
            event_id: self.event_id,
            name: self.name,
            amount: self.amount,
            bill_type: self.bill_type,
            frequency: self.frequency,
            anchor: self.anchor,
            autopay_account_id: self.autopay_account_id,
            description: self.description,
        }
    }
}

/// Delete a manual recurring bill (personal-cfo-zl1l): remove its `recurring_event`
/// and linked `bill_contract`, keyed by the recurring-event id, and refresh the
/// commitments projection. Only the bill's forward-looking schedule is dropped — no
/// posted transactions are affected. Richer archive-with-history is personal-cfo-4d8.2.
#[derive(Debug, Clone)]
pub struct DeleteRecurringBill {
    event_id: RecurringEventId,
}

impl DeleteRecurringBill {
    /// Build a `DeleteRecurringBill` from the bill's recurring-event id.
    #[must_use]
    pub fn new(event_id: RecurringEventId) -> Self {
        Self { event_id }
    }
}

impl sealed::Sealed for DeleteRecurringBill {}

impl KernelCommand for DeleteRecurringBill {
    fn kind(&self) -> &'static str {
        "delete_recurring_bill"
    }

    fn validate(&self) -> Result<(), KernelError> {
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::DeleteRecurringBill {
            event_id: self.event_id,
        }
    }
}

/// Archive (soft-hide) a recurring bill (personal-cfo-4d8.2). Non-destructive: the
/// bill and its history are kept; it leaves the active list and the forecast.
#[derive(Debug, Clone, Copy)]
pub struct ArchiveRecurringBill {
    event_id: RecurringEventId,
}

impl ArchiveRecurringBill {
    /// Build an `ArchiveRecurringBill` for the bill's recurring-event id.
    #[must_use]
    pub const fn new(event_id: RecurringEventId) -> Self {
        Self { event_id }
    }
}

impl sealed::Sealed for ArchiveRecurringBill {}

impl KernelCommand for ArchiveRecurringBill {
    fn kind(&self) -> &'static str {
        "archive_recurring_bill"
    }

    fn validate(&self) -> Result<(), KernelError> {
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::ArchiveRecurringBill(self.event_id)
    }
}

/// Restore a previously archived recurring bill (personal-cfo-4d8.2).
#[derive(Debug, Clone, Copy)]
pub struct RestoreRecurringBill {
    event_id: RecurringEventId,
}

impl RestoreRecurringBill {
    /// Build a `RestoreRecurringBill` for the bill's recurring-event id.
    #[must_use]
    pub const fn new(event_id: RecurringEventId) -> Self {
        Self { event_id }
    }
}

impl sealed::Sealed for RestoreRecurringBill {}

impl KernelCommand for RestoreRecurringBill {
    fn kind(&self) -> &'static str {
        "restore_recurring_bill"
    }

    fn validate(&self) -> Result<(), KernelError> {
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::RestoreRecurringBill(self.event_id)
    }
}

/// Rename an account.
#[derive(Debug, Clone)]
pub struct UpdateAccount {
    id: AccountId,
    name: String,
}

impl UpdateAccount {
    /// Build an `UpdateAccount` command.
    #[must_use]
    pub fn new(id: AccountId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
        }
    }
}

impl sealed::Sealed for UpdateAccount {}

impl KernelCommand for UpdateAccount {
    fn kind(&self) -> &'static str {
        "update_account"
    }

    fn validate(&self) -> Result<(), KernelError> {
        if self.name.trim().is_empty() {
            return Err(KernelError::Validation(
                "account name must not be empty".to_owned(),
            ));
        }
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::UpdateAccount {
            id: self.id,
            name: self.name,
        }
    }
}

/// Archive (soft-hide) an account. Non-destructive: postings are preserved.
#[derive(Debug, Clone, Copy)]
pub struct ArchiveAccount {
    id: AccountId,
}

impl ArchiveAccount {
    /// Build an `ArchiveAccount` command for `id`.
    #[must_use]
    pub const fn new(id: AccountId) -> Self {
        Self { id }
    }
}

impl sealed::Sealed for ArchiveAccount {}

impl KernelCommand for ArchiveAccount {
    fn kind(&self) -> &'static str {
        "archive_account"
    }

    fn validate(&self) -> Result<(), KernelError> {
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::ArchiveAccount(self.id)
    }
}

/// Reinstate a previously archived account.
#[derive(Debug, Clone, Copy)]
pub struct ReinstateAccount {
    id: AccountId,
}

impl ReinstateAccount {
    /// Build a `ReinstateAccount` command for `id`.
    #[must_use]
    pub const fn new(id: AccountId) -> Self {
        Self { id }
    }
}

impl sealed::Sealed for ReinstateAccount {}

impl KernelCommand for ReinstateAccount {
    fn kind(&self) -> &'static str {
        "reinstate_account"
    }

    fn validate(&self) -> Result<(), KernelError> {
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::ReinstateAccount(self.id)
    }
}

/// Set (or clear) an account's subtype (ADR 0028, personal-cfo-9dgg). `None`
/// clears it. The role↔subtype match is validated in the worker, which has the
/// account's current cashflow role.
#[derive(Debug, Clone, Copy)]
pub struct SetAccountSubtype {
    id: AccountId,
    subtype: Option<AccountSubtype>,
}

impl SetAccountSubtype {
    /// Build a `SetAccountSubtype` command (`None` clears the subtype).
    #[must_use]
    pub const fn new(id: AccountId, subtype: Option<AccountSubtype>) -> Self {
        Self { id, subtype }
    }
}

impl sealed::Sealed for SetAccountSubtype {}

impl KernelCommand for SetAccountSubtype {
    fn kind(&self) -> &'static str {
        "set_account_subtype"
    }

    fn validate(&self) -> Result<(), KernelError> {
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::SetAccountSubtype {
            id: self.id,
            subtype: self.subtype,
        }
    }
}

/// Set (or clear) an account's free-text note (ADR 0044, personal-cfo-4d8.22.4).
#[derive(Debug, Clone)]
pub struct SetAccountNote {
    id: AccountId,
    note: Option<String>,
}

impl SetAccountNote {
    /// Build a `SetAccountNote` command (`None` clears the note).
    #[must_use]
    pub const fn new(id: AccountId, note: Option<String>) -> Self {
        Self { id, note }
    }
}

impl sealed::Sealed for SetAccountNote {}

impl KernelCommand for SetAccountNote {
    fn kind(&self) -> &'static str {
        "set_account_note"
    }

    fn validate(&self) -> Result<(), KernelError> {
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::SetAccountNote {
            id: self.id,
            note: self.note,
        }
    }
}

/// Link a real asset to the liability that finances it, or clear the link (ADR 0044 §5,
/// personal-cfo-4d8.22.3). The real-asset/liability role checks need another account's
/// role, so they are validated in the worker, not here.
#[derive(Debug, Clone, Copy)]
pub struct SetAccountLink {
    asset_id: AccountId,
    liability_id: Option<AccountId>,
}

impl SetAccountLink {
    /// Build a `SetAccountLink` command (`None` clears the asset's link).
    #[must_use]
    pub const fn new(asset_id: AccountId, liability_id: Option<AccountId>) -> Self {
        Self {
            asset_id,
            liability_id,
        }
    }
}

impl sealed::Sealed for SetAccountLink {}

impl KernelCommand for SetAccountLink {
    fn kind(&self) -> &'static str {
        "set_account_link"
    }

    fn validate(&self) -> Result<(), KernelError> {
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::SetAccountLink {
            asset_id: self.asset_id,
            liability_id: self.liability_id,
        }
    }
}

/// Upsert a liability account's debt terms (ADR 0035 §5, personal-cfo-6wk.6). The
/// liability-target and liquid-paying-source rules need another account's role, so they are
/// validated in the worker, not here.
#[derive(Debug, Clone, Copy)]
pub struct SetDebtTerms {
    account_id: AccountId,
    terms: DebtTermsInput,
}

impl SetDebtTerms {
    /// Build a `SetDebtTerms` command.
    #[must_use]
    pub const fn new(account_id: AccountId, terms: DebtTermsInput) -> Self {
        Self { account_id, terms }
    }
}

impl sealed::Sealed for SetDebtTerms {}

impl KernelCommand for SetDebtTerms {
    fn kind(&self) -> &'static str {
        "set_debt_terms"
    }

    fn validate(&self) -> Result<(), KernelError> {
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::SetDebtTerms {
            account_id: self.account_id,
            terms: self.terms,
        }
    }
}

/// Record — or clear — a card statement's REAL balance for one cycle (feedback
/// 2026-07-03). The credit-card role check needs the account row, so it is validated in
/// the worker, not here.
#[derive(Debug, Clone, Copy)]
pub struct SetCardStatementBalance {
    account_id: AccountId,
    cycle_close: chrono::NaiveDate,
    statement_balance_minor: Option<i64>,
}

impl SetCardStatementBalance {
    /// Build a `SetCardStatementBalance` command (`None` clears the assertion).
    #[must_use]
    pub const fn new(
        account_id: AccountId,
        cycle_close: chrono::NaiveDate,
        statement_balance_minor: Option<i64>,
    ) -> Self {
        Self {
            account_id,
            cycle_close,
            statement_balance_minor,
        }
    }
}

impl sealed::Sealed for SetCardStatementBalance {}

impl KernelCommand for SetCardStatementBalance {
    fn kind(&self) -> &'static str {
        "set_card_statement_balance"
    }

    fn validate(&self) -> Result<(), KernelError> {
        if self.statement_balance_minor.is_some_and(|v| v < 0) {
            return Err(KernelError::Validation(
                "a statement balance cannot be negative".to_owned(),
            ));
        }
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::SetCardStatementBalance {
            account_id: self.account_id,
            cycle_close: self.cycle_close,
            statement_balance_minor: self.statement_balance_minor,
        }
    }
}

/// Open an ingestion source batch (ADR 0008, personal-cfo-3bb). The id is minted
/// by the caller so the importer can attach records before the batch is staged;
/// importers stage records under a batch and never write the ledger directly.
#[derive(Debug, Clone)]
pub struct CreateSourceBatch {
    id: SourceBatchId,
    source_type: String,
    source_name: Option<String>,
    file_fingerprint: Option<String>,
    parser_version: Option<String>,
}

impl CreateSourceBatch {
    /// Build a `CreateSourceBatch`. `source_type` is a `source_batches.source_type`
    /// token (`csv`/`ofx`/`manual`/…); `file_fingerprint` is the whole-file content
    /// hash used for file-level dedupe (ADR 0014) when the source is a file.
    #[must_use]
    pub fn new(
        id: SourceBatchId,
        source_type: impl Into<String>,
        source_name: Option<String>,
        file_fingerprint: Option<String>,
        parser_version: Option<String>,
    ) -> Self {
        Self {
            id,
            source_type: source_type.into(),
            source_name,
            file_fingerprint,
            parser_version,
        }
    }
}

impl sealed::Sealed for CreateSourceBatch {}

impl KernelCommand for CreateSourceBatch {
    fn kind(&self) -> &'static str {
        "create_source_batch"
    }

    fn validate(&self) -> Result<(), KernelError> {
        if self.source_type.trim().is_empty() {
            return Err(KernelError::Validation(
                "source batch source_type must not be empty".to_owned(),
            ));
        }
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::CreateSourceBatch {
            id: self.id,
            source_type: self.source_type,
            source_name: self.source_name,
            file_fingerprint: self.file_fingerprint,
            parser_version: self.parser_version,
        }
    }
}

/// Attach a parsed source record to a batch (ADR 0008, personal-cfo-3bb).
/// Idempotent on `(batch, source_hash)` — re-attaching the same content adds no
/// new record (shred-after-parse: the hash + normalized fields persist, not bytes).
#[derive(Debug, Clone)]
pub struct AttachSourceRecord {
    id: SourceRecordId,
    batch_id: SourceBatchId,
    external_id: Option<String>,
    source_hash: String,
    normalized_json: String,
    parse_confidence_bps: Option<i64>,
}

impl AttachSourceRecord {
    /// Build an `AttachSourceRecord`. `source_hash` is the record's content
    /// fingerprint (the dedupe key); `normalized_json` is the extracted, normalized
    /// fields — never the raw uploaded bytes (ADR 0014 §4).
    #[must_use]
    pub fn new(
        id: SourceRecordId,
        batch_id: SourceBatchId,
        external_id: Option<String>,
        source_hash: impl Into<String>,
        normalized_json: impl Into<String>,
        parse_confidence_bps: Option<i64>,
    ) -> Self {
        Self {
            id,
            batch_id,
            external_id,
            source_hash: source_hash.into(),
            normalized_json: normalized_json.into(),
            parse_confidence_bps,
        }
    }
}

impl sealed::Sealed for AttachSourceRecord {}

impl KernelCommand for AttachSourceRecord {
    fn kind(&self) -> &'static str {
        "attach_source_record"
    }

    fn validate(&self) -> Result<(), KernelError> {
        if self.source_hash.trim().is_empty() {
            return Err(KernelError::Validation(
                "source record source_hash must not be empty".to_owned(),
            ));
        }
        if self.normalized_json.trim().is_empty() {
            return Err(KernelError::Validation(
                "source record normalized_json must not be empty".to_owned(),
            ));
        }
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::AttachSourceRecord {
            id: self.id,
            batch_id: self.batch_id,
            external_id: self.external_id,
            source_hash: self.source_hash,
            normalized_json: self.normalized_json,
            parse_confidence_bps: self.parse_confidence_bps,
        }
    }
}

/// Advance a source batch's lifecycle status + progress counts (ADR 0008,
/// personal-cfo-3bb): `parsing → staged → committed | partially_committed |
/// discarded | failed`, plus `superseded`.
#[derive(Debug, Clone)]
pub struct UpdateBatchState {
    batch_id: SourceBatchId,
    status: String,
    staged_count: i64,
    committed_count: i64,
    skipped_count: i64,
}

impl UpdateBatchState {
    /// Build an `UpdateBatchState`. `status` is one of the ADR 0008 lifecycle
    /// tokens; the counts are cumulative progress (non-negative).
    #[must_use]
    pub fn new(
        batch_id: SourceBatchId,
        status: impl Into<String>,
        staged_count: i64,
        committed_count: i64,
        skipped_count: i64,
    ) -> Self {
        Self {
            batch_id,
            status: status.into(),
            staged_count,
            committed_count,
            skipped_count,
        }
    }
}

impl sealed::Sealed for UpdateBatchState {}

impl KernelCommand for UpdateBatchState {
    fn kind(&self) -> &'static str {
        "update_batch_state"
    }

    fn validate(&self) -> Result<(), KernelError> {
        const VALID: [&str; 7] = [
            "parsing",
            "staged",
            "committed",
            "partially_committed",
            "discarded",
            "failed",
            "superseded",
        ];
        if !VALID.contains(&self.status.as_str()) {
            return Err(KernelError::Validation(format!(
                "invalid source batch status '{}'",
                self.status
            )));
        }
        if self.staged_count < 0 || self.committed_count < 0 || self.skipped_count < 0 {
            return Err(KernelError::Validation(
                "source batch counts must not be negative".to_owned(),
            ));
        }
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::UpdateBatchState {
            batch_id: self.batch_id,
            status: self.status,
            staged_count: self.staged_count,
            committed_count: self.committed_count,
            skipped_count: self.skipped_count,
        }
    }
}

/// Commit one staged transaction to the ledger (ADR 0008/0014, personal-cfo-cmx):
/// promote it to a balanced ledger transaction + FK-strict import provenance, or
/// — if its fingerprint duplicates an already-committed import — flag it for the
/// Money Inbox (never a silent drop). Idempotent via the command bus.
#[derive(Debug, Clone, Copy)]
pub struct CommitStaged {
    staged_transaction_id: StagedTransactionId,
    force: bool,
}

impl CommitStaged {
    /// Build a `CommitStaged` for the staged transaction `id` — the normal
    /// pipeline path, where the transaction-level dedupe check applies.
    #[must_use]
    pub const fn new(staged_transaction_id: StagedTransactionId) -> Self {
        Self {
            staged_transaction_id,
            force: false,
        }
    }

    /// Commit `id` unconditionally, skipping the dedupe check — the Money Inbox
    /// "import anyway" resolution for a flagged duplicate (ADR 0014 §7,
    /// personal-cfo-asqy).
    #[must_use]
    pub const fn import_anyway(staged_transaction_id: StagedTransactionId) -> Self {
        Self {
            staged_transaction_id,
            force: true,
        }
    }
}

impl sealed::Sealed for CommitStaged {}

impl KernelCommand for CommitStaged {
    fn kind(&self) -> &'static str {
        "commit_staged"
    }

    fn validate(&self) -> Result<(), KernelError> {
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::CommitStaged {
            staged_transaction_id: self.staged_transaction_id,
            force: self.force,
        }
    }
}

/// Skip a flagged staged transaction without committing it — the Money Inbox
/// "skip" resolution (ADR 0014 §7, personal-cfo-asqy). Marks the row `skipped`
/// so the next inbox rebuild drops it; no ledger write. Idempotent via the bus.
#[derive(Debug, Clone, Copy)]
pub struct SkipStaged {
    staged_transaction_id: StagedTransactionId,
}

impl SkipStaged {
    /// Build a `SkipStaged` for the staged transaction `id`.
    #[must_use]
    pub const fn new(staged_transaction_id: StagedTransactionId) -> Self {
        Self {
            staged_transaction_id,
        }
    }
}

impl sealed::Sealed for SkipStaged {}

impl KernelCommand for SkipStaged {
    fn kind(&self) -> &'static str {
        "skip_staged"
    }

    fn validate(&self) -> Result<(), KernelError> {
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::SkipStaged {
            staged_transaction_id: self.staged_transaction_id,
        }
    }
}

/// Confirm a recurring bill occurrence as paid early (personal-cfo-5ie.9, ADR 5ie.7): post a real
/// outflow from `paying_account_id` dated `actual_date` for the occurrence scheduled on
/// `scheduled_date`, so the balance moves now and the forecast stops projecting that occurrence.
/// Idempotent on `(recurring_event_id, scheduled_date)`. v1 is the liquid-paid bill path; the
/// card/loan liability-crediting path awaits mixed-role transfers (personal-cfo-r7sb).
#[derive(Debug, Clone, Copy)]
pub struct ConfirmObligationEarly {
    recurring_event_id: RecurringEventId,
    scheduled_date: NaiveDate,
    actual_amount: Money,
    actual_date: DateTime<Utc>,
    paying_account_id: AccountId,
}

impl ConfirmObligationEarly {
    /// Build a `ConfirmObligationEarly`. `actual_amount` is the amount paid — non-negative; zero
    /// means nothing was due this cycle (the occurrence is still cleared from the forecast).
    #[must_use]
    pub const fn new(
        recurring_event_id: RecurringEventId,
        scheduled_date: NaiveDate,
        actual_amount: Money,
        actual_date: DateTime<Utc>,
        paying_account_id: AccountId,
    ) -> Self {
        Self {
            recurring_event_id,
            scheduled_date,
            actual_amount,
            actual_date,
            paying_account_id,
        }
    }
}

impl sealed::Sealed for ConfirmObligationEarly {}

impl KernelCommand for ConfirmObligationEarly {
    fn kind(&self) -> &'static str {
        "confirm_obligation_early"
    }

    fn validate(&self) -> Result<(), KernelError> {
        // Zero is valid — "nothing due this cycle" still clears the occurrence from the forecast.
        // Only a negative amount is rejected.
        if self.actual_amount.minor_units() < 0 {
            return Err(KernelError::Validation(
                "confirmed amount can't be negative".to_owned(),
            ));
        }
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::ConfirmObligationEarly {
            recurring_event_id: self.recurring_event_id,
            scheduled_date: self.scheduled_date,
            actual_amount: self.actual_amount,
            actual_date: self.actual_date,
            paying_account_id: self.paying_account_id,
        }
    }
}

/// Reverse an early confirm (personal-cfo-5ie.9): void the transaction it posted and drop the
/// fulfillment record, so the forecast projects the occurrence again. Idempotent — a no-op if the
/// occurrence was never confirmed. Keyed on `(recurring_event_id, scheduled_date)`.
#[derive(Debug, Clone, Copy)]
pub struct UnconfirmObligation {
    recurring_event_id: RecurringEventId,
    scheduled_date: NaiveDate,
}

impl UnconfirmObligation {
    /// Build an `UnconfirmObligation` for the occurrence of `recurring_event_id` scheduled on
    /// `scheduled_date`.
    #[must_use]
    pub const fn new(recurring_event_id: RecurringEventId, scheduled_date: NaiveDate) -> Self {
        Self {
            recurring_event_id,
            scheduled_date,
        }
    }
}

impl sealed::Sealed for UnconfirmObligation {}

impl KernelCommand for UnconfirmObligation {
    fn kind(&self) -> &'static str {
        "unconfirm_obligation"
    }

    fn validate(&self) -> Result<(), KernelError> {
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::UnconfirmObligation {
            recurring_event_id: self.recurring_event_id,
            scheduled_date: self.scheduled_date,
        }
    }
}

/// The accepted dismiss reasons (ADR 0014 §7, personal-cfo-ci71).
pub const INBOX_DISMISS_REASONS: [&str; 4] =
    ["not_relevant", "already_handled", "incorrect", "other"];

/// Snooze a Money Inbox item until `until` (ADR 0014 §7, personal-cfo-ci71) — a
/// soft action recorded in `change_journal_entries` and re-applied on rebuild.
#[derive(Debug, Clone, Copy)]
pub struct SnoozeInboxItem {
    item_id: Uuid,
    until: NaiveDate,
}

impl SnoozeInboxItem {
    /// Build a `SnoozeInboxItem` hiding `item_id` until `until`.
    #[must_use]
    pub const fn new(item_id: Uuid, until: NaiveDate) -> Self {
        Self { item_id, until }
    }
}

impl sealed::Sealed for SnoozeInboxItem {}

impl KernelCommand for SnoozeInboxItem {
    fn kind(&self) -> &'static str {
        "snooze_inbox_item"
    }

    fn validate(&self) -> Result<(), KernelError> {
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::SnoozeInboxItem {
            item_id: self.item_id,
            until: self.until,
        }
    }
}

/// Dismiss a Money Inbox item with a typed `reason` (ADR 0014 §7,
/// personal-cfo-ci71). The item stays hidden across rebuilds.
#[derive(Debug, Clone)]
pub struct DismissInboxItem {
    item_id: Uuid,
    reason: String,
}

impl DismissInboxItem {
    /// Build a `DismissInboxItem`. `reason` must be one of [`INBOX_DISMISS_REASONS`].
    #[must_use]
    pub fn new(item_id: Uuid, reason: String) -> Self {
        Self { item_id, reason }
    }
}

impl sealed::Sealed for DismissInboxItem {}

impl KernelCommand for DismissInboxItem {
    fn kind(&self) -> &'static str {
        "dismiss_inbox_item"
    }

    fn validate(&self) -> Result<(), KernelError> {
        if !INBOX_DISMISS_REASONS.contains(&self.reason.as_str()) {
            return Err(KernelError::Validation(format!(
                "invalid dismiss reason: {}",
                self.reason
            )));
        }
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::DismissInboxItem {
            item_id: self.item_id,
            reason: self.reason,
        }
    }
}

/// Dismiss a recurring-bill SUGGESTION (ADR 0046, personal-cfo-4d8.24.6): record a
/// suppression keyed on `(merchant_key, currency)` at the dismissed `amount_minor` +
/// `frequency`, so detection stops offering it until the pattern materially changes.
/// Latest-dismiss-wins (the apply upserts on the key).
#[derive(Debug, Clone)]
pub struct DismissRecurringSuggestion {
    merchant_key: String,
    currency: String,
    amount_minor: i64,
    frequency: String,
    reason: Option<String>,
}

impl DismissRecurringSuggestion {
    /// Build a `DismissRecurringSuggestion` from a suggestion's `(merchant_key, currency,
    /// amount_minor, frequency)` and an optional free-text `reason`.
    #[must_use]
    pub fn new(
        merchant_key: impl Into<String>,
        currency: impl Into<String>,
        amount_minor: i64,
        frequency: impl Into<String>,
        reason: Option<String>,
    ) -> Self {
        Self {
            merchant_key: merchant_key.into(),
            currency: currency.into(),
            amount_minor,
            frequency: frequency.into(),
            reason,
        }
    }
}

impl sealed::Sealed for DismissRecurringSuggestion {}

impl KernelCommand for DismissRecurringSuggestion {
    fn kind(&self) -> &'static str {
        "dismiss_recurring_suggestion"
    }

    fn validate(&self) -> Result<(), KernelError> {
        if self.merchant_key.trim().is_empty() {
            return Err(KernelError::Validation(
                "recurring dismissal needs a merchant key".to_owned(),
            ));
        }
        if self.currency.trim().is_empty() {
            return Err(KernelError::Validation(
                "recurring dismissal needs a currency".to_owned(),
            ));
        }
        if self.frequency.trim().is_empty() {
            return Err(KernelError::Validation(
                "recurring dismissal needs a frequency".to_owned(),
            ));
        }
        if self.amount_minor <= 0 {
            return Err(KernelError::Validation(
                "recurring dismissal amount must be a positive magnitude".to_owned(),
            ));
        }
        if let Some(reason) = &self.reason {
            if reason.chars().count() > 280 {
                return Err(KernelError::Validation(
                    "recurring dismissal reason is too long (max 280 chars)".to_owned(),
                ));
            }
        }
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::DismissRecurringSuggestion {
            merchant_key: self.merchant_key,
            currency: self.currency,
            amount_minor: self.amount_minor,
            frequency: self.frequency,
            reason: self.reason,
        }
    }
}

/// Create a user category in the taxonomy (ADR 0030, personal-cfo-bac).
#[derive(Debug, Clone)]
pub struct CreateCategory {
    id: CategoryId,
    parent_id: Option<CategoryId>,
    name: String,
    category_type: String,
    color: Option<String>,
    icon: Option<String>,
}

impl CreateCategory {
    /// Build a `CreateCategory`. The caller mints `id`; `category_type` is one of
    /// `income` / `expense` / `transfer` / `adjustment`. `icon` is an optional emoji
    /// set at creation (personal-cfo-kogu).
    #[must_use]
    pub fn new(
        id: CategoryId,
        parent_id: Option<CategoryId>,
        name: String,
        category_type: String,
        color: Option<String>,
        icon: Option<String>,
    ) -> Self {
        Self {
            id,
            parent_id,
            name,
            category_type,
            color,
            icon,
        }
    }
}

impl sealed::Sealed for CreateCategory {}

impl KernelCommand for CreateCategory {
    fn kind(&self) -> &'static str {
        "create_category"
    }

    fn validate(&self) -> Result<(), KernelError> {
        if self.name.trim().is_empty() {
            return Err(KernelError::Validation(
                "category name must not be empty".to_owned(),
            ));
        }
        if !matches!(
            self.category_type.as_str(),
            "income" | "expense" | "transfer" | "adjustment"
        ) {
            return Err(KernelError::Validation(format!(
                "invalid category type: {}",
                self.category_type
            )));
        }
        // Bound the icon (an emoji or short token); 16 scalar values fits a ZWJ emoji
        // sequence while keeping the stored value small (personal-cfo-kogu / 4d8.24.10).
        if let Some(icon) = &self.icon {
            if icon.trim().chars().count() > 16 {
                return Err(KernelError::Validation(
                    "category icon must be at most 16 characters".to_owned(),
                ));
            }
        }
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        // Normalize an empty/whitespace icon to `None` so an untouched field stays null
        // rather than storing "".
        let icon = self.icon.and_then(|value| {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        });
        WriteCommand::CreateCategory {
            id: self.id,
            parent_id: self.parent_id,
            name: self.name,
            category_type: self.category_type,
            color: self.color,
            icon,
        }
    }
}

/// Create a user-defined tag (ADR 0033, personal-cfo-2ryf). The caller mints `id`.
#[derive(Debug, Clone)]
pub struct CreateTag {
    id: TagId,
    name: String,
    color: Option<String>,
}

impl CreateTag {
    /// Build a `CreateTag`. The caller mints `id`.
    #[must_use]
    pub fn new(id: TagId, name: String, color: Option<String>) -> Self {
        Self { id, name, color }
    }
}

impl sealed::Sealed for CreateTag {}

impl KernelCommand for CreateTag {
    fn kind(&self) -> &'static str {
        "create_tag"
    }

    fn validate(&self) -> Result<(), KernelError> {
        if self.name.trim().is_empty() {
            return Err(KernelError::Validation(
                "tag name must not be empty".to_owned(),
            ));
        }
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::CreateTag {
            id: self.id,
            name: self.name,
            color: self.color,
        }
    }
}

/// Replace a transaction's tag set (ADR 0033, personal-cfo-hmt): pass the full desired
/// set to add/remove. The db-worker validates the transaction + tags exist.
#[derive(Debug, Clone)]
pub struct SetTags {
    transaction_id: TransactionId,
    tag_ids: Vec<TagId>,
}

impl SetTags {
    /// Build a `SetTags` setting `transaction_id`'s tags to exactly `tag_ids`.
    #[must_use]
    pub fn new(transaction_id: TransactionId, tag_ids: Vec<TagId>) -> Self {
        Self {
            transaction_id,
            tag_ids,
        }
    }
}

impl sealed::Sealed for SetTags {}

impl KernelCommand for SetTags {
    fn kind(&self) -> &'static str {
        "set_tags"
    }

    fn validate(&self) -> Result<(), KernelError> {
        Ok(()) // existence checked in the db-worker apply (it holds the rows)
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::SetTags {
            transaction_id: self.transaction_id,
            tag_ids: self.tag_ids,
        }
    }
}

/// Set (or clear, with `None`) a transaction's free-text note (ADR 0033 §3, ≤ 4096).
#[derive(Debug, Clone)]
pub struct SetNote {
    transaction_id: TransactionId,
    note: Option<String>,
}

impl SetNote {
    /// Build a `SetNote` for `transaction_id` (`None` clears the note).
    #[must_use]
    pub fn new(transaction_id: TransactionId, note: Option<String>) -> Self {
        Self {
            transaction_id,
            note,
        }
    }
}

impl sealed::Sealed for SetNote {}

impl KernelCommand for SetNote {
    fn kind(&self) -> &'static str {
        "set_note"
    }

    fn validate(&self) -> Result<(), KernelError> {
        if let Some(note) = &self.note {
            if note.chars().count() > 4096 {
                return Err(KernelError::Validation(
                    "note must be 4096 characters or fewer".to_owned(),
                ));
            }
        }
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::SetNote {
            transaction_id: self.transaction_id,
            note: self.note,
        }
    }
}

/// Replace a transaction's split set (ADR 0034, personal-cfo-e7i). The lines must sum to
/// the transaction amount; that + existence are validated in the db-worker apply (which
/// holds the rows). An empty `lines` un-splits the transaction.
#[derive(Debug, Clone)]
pub struct SetSplits {
    transaction_id: TransactionId,
    lines: Vec<SplitLineInput>,
}

impl SetSplits {
    /// Build a `SetSplits` setting `transaction_id`'s splits to exactly `lines`.
    #[must_use]
    pub fn new(transaction_id: TransactionId, lines: Vec<SplitLineInput>) -> Self {
        Self {
            transaction_id,
            lines,
        }
    }
}

impl sealed::Sealed for SetSplits {}

impl KernelCommand for SetSplits {
    fn kind(&self) -> &'static str {
        "set_splits"
    }

    fn validate(&self) -> Result<(), KernelError> {
        Ok(()) // sum + existence checked in the db-worker apply (it holds the rows)
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::SetSplits {
            transaction_id: self.transaction_id,
            lines: self.lines,
        }
    }
}

/// Archive (hide) a category — system or user (ADR 0030). Idempotent via the bus.
#[derive(Debug, Clone, Copy)]
pub struct ArchiveCategory {
    id: CategoryId,
}

impl ArchiveCategory {
    /// Build an `ArchiveCategory` for `id`.
    #[must_use]
    pub const fn new(id: CategoryId) -> Self {
        Self { id }
    }
}

impl sealed::Sealed for ArchiveCategory {}

impl KernelCommand for ArchiveCategory {
    fn kind(&self) -> &'static str {
        "archive_category"
    }

    fn validate(&self) -> Result<(), KernelError> {
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::ArchiveCategory(self.id)
    }
}

/// Un-archive a previously hidden category (ADR 0030).
#[derive(Debug, Clone, Copy)]
pub struct ReinstateCategory {
    id: CategoryId,
}

impl ReinstateCategory {
    /// Build a `ReinstateCategory` for `id`.
    #[must_use]
    pub const fn new(id: CategoryId) -> Self {
        Self { id }
    }
}

impl sealed::Sealed for ReinstateCategory {}

impl KernelCommand for ReinstateCategory {
    fn kind(&self) -> &'static str {
        "reinstate_category"
    }

    fn validate(&self) -> Result<(), KernelError> {
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::ReinstateCategory(self.id)
    }
}

/// Rename and/or recolor a category (ADR 0030, personal-cfo-bac). A *user* category
/// updates name + color + icon; a *system* ("Default") category updates its appearance
/// (color + icon) only — the db-worker preserves its name and ignores any submitted name,
/// so identity stays immutable (ADR 0030 amendment, personal-cfo-kogu).
#[derive(Debug, Clone)]
pub struct UpdateCategory {
    id: CategoryId,
    name: String,
    color: Option<String>,
    /// A short display icon (an emoji), or `None` to clear it (personal-cfo-4d8.24.10).
    icon: Option<String>,
}

impl UpdateCategory {
    /// Build an `UpdateCategory` setting `name`, `color`, and `icon` for `id`.
    #[must_use]
    pub fn new(id: CategoryId, name: String, color: Option<String>, icon: Option<String>) -> Self {
        Self {
            id,
            name,
            color,
            icon,
        }
    }
}

impl sealed::Sealed for UpdateCategory {}

impl KernelCommand for UpdateCategory {
    fn kind(&self) -> &'static str {
        "update_category"
    }

    fn validate(&self) -> Result<(), KernelError> {
        if self.name.trim().is_empty() {
            return Err(KernelError::Validation(
                "category name must not be empty".to_owned(),
            ));
        }
        // Bound the icon (an emoji or short token); 16 scalar values fits a ZWJ emoji
        // sequence while keeping the stored value small (personal-cfo-4d8.24.10).
        if let Some(icon) = &self.icon {
            if icon.trim().chars().count() > 16 {
                return Err(KernelError::Validation(
                    "category icon must be at most 16 characters".to_owned(),
                ));
            }
        }
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        // Normalize an empty/whitespace icon to `None` so clearing the field clears the
        // column (rather than storing "").
        let icon = self.icon.and_then(|value| {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        });
        WriteCommand::UpdateCategory {
            id: self.id,
            name: self.name,
            color: self.color,
            icon,
        }
    }
}

/// Re-parent a *user* category, or make it a top-level group (`new_parent_id`
/// `None`) (ADR 0030, personal-cfo-bac). A system category cannot be re-parented (its
/// identity is fixed); the db-worker validates parent existence and rejects
/// re-parenting cycles.
#[derive(Debug, Clone, Copy)]
pub struct MoveCategory {
    id: CategoryId,
    new_parent_id: Option<CategoryId>,
}

impl MoveCategory {
    /// Build a `MoveCategory` re-parenting `id` under `new_parent_id` (`None` for a
    /// top-level group).
    #[must_use]
    pub const fn new(id: CategoryId, new_parent_id: Option<CategoryId>) -> Self {
        Self { id, new_parent_id }
    }
}

impl sealed::Sealed for MoveCategory {}

impl KernelCommand for MoveCategory {
    fn kind(&self) -> &'static str {
        "move_category"
    }

    fn validate(&self) -> Result<(), KernelError> {
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::MoveCategory {
            id: self.id,
            new_parent_id: self.new_parent_id,
        }
    }
}

/// Set or clear a transaction's category (ADR 0030, personal-cfo-bac). A manual
/// assignment (`source = user`, `confidence = 100%`); `category_id` `None` clears
/// it. The db-worker validates that the transaction and category exist.
#[derive(Debug, Clone, Copy)]
pub struct RecategorizeTransaction {
    transaction_id: TransactionId,
    category_id: Option<CategoryId>,
}

impl RecategorizeTransaction {
    /// Build a `RecategorizeTransaction` assigning `category_id` (or `None` to
    /// clear) to `transaction_id`.
    #[must_use]
    pub const fn new(transaction_id: TransactionId, category_id: Option<CategoryId>) -> Self {
        Self {
            transaction_id,
            category_id,
        }
    }
}

impl sealed::Sealed for RecategorizeTransaction {}

impl KernelCommand for RecategorizeTransaction {
    fn kind(&self) -> &'static str {
        "recategorize_transaction"
    }

    fn validate(&self) -> Result<(), KernelError> {
        Ok(())
    }

    fn lower(self) -> WriteCommand {
        WriteCommand::RecategorizeTransaction {
            transaction_id: self.transaction_id,
            category_id: self.category_id,
        }
    }
}

/// The outcome of an [`Kernel::ingest_batch`] run (personal-cfo-cmx).
#[derive(Debug, Clone)]
pub struct BatchResult {
    /// The source batch id (display string), if a batch was created.
    pub source_batch_id: Option<String>,
    /// Terminal status: `committed` / `partially_committed` / `failed` /
    /// `already_imported`.
    pub status: String,
    /// Staged transactions in the batch.
    pub staged: u32,
    /// Transactions committed to the ledger.
    pub committed: u32,
    /// Transactions flagged as suspected duplicates (surfaced in the Money Inbox).
    pub flagged: u32,
    /// Transactions auto-categorized from merchant memory after the import (ADR 0030
    /// addendum, personal-cfo-5n4.2). `0` when the setting is off or nothing matched.
    pub auto_categorized: u32,
}

/// [`Kernel::ingest_sync_batch`]'s result: the standard batch outcome plus
/// the count of records skipped because their external account is unmapped
/// (personal-cfo-gglk).
#[derive(Debug, Clone)]
pub struct SyncBatchResult {
    pub batch: BatchResult,
    /// Records not staged: their connector account key has no mapped account.
    pub skipped_unmapped: usize,
}

/// Derive a fresh per-command [`CommandMeta`] from an ingest `seed`: a new command
/// id + idempotency key, the seed's correlation + actor, and the seed command as
/// causation — so all of a batch's commands share one correlation.
fn next_meta(seed: &CommandMeta) -> CommandMeta {
    CommandMeta {
        command_id: Uuid::now_v7(),
        correlation_id: seed.correlation_id,
        causation_id: Some(seed.command_id),
        actor_type: seed.actor_type,
        actor_id: seed.actor_id.clone(),
        idempotency_key: Uuid::now_v7().to_string(),
    }
}

/// Errors from the kernel boundary. Intentionally free of any `rusqlite` types:
/// persistence failures are flattened to a message so no database type leaks.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum KernelError {
    /// A command failed domain validation.
    #[error("command validation failed: {0}")]
    Validation(String),
    /// A required metadata field was empty.
    #[error("missing required command metadata: {0}")]
    MissingMetadata(&'static str),
    /// The kernel's persistence is not accepting writes.
    #[error("kernel unavailable for writes: {0:?}")]
    Unavailable(WorkerState),
    /// A writer panic rolled back the transaction; recovery is required.
    #[error("writer panicked; persistence marked for recovery")]
    WriterPanicked,
    /// Any other persistence failure, flattened to keep database types out of
    /// the public API.
    #[error("persistence error: {0}")]
    Persistence(String),
    /// `create_vault` was asked to create a vault whose envelope already exists.
    #[error("a vault already exists at this location")]
    VaultExists,
    /// `unlock_vault` found no vault envelope at the given location.
    #[error("no vault exists at this location")]
    VaultNotFound,
    /// The vault could not be unlocked: the password was wrong (the wrapped DEK
    /// failed to authenticate). Deliberately carries no detail — there is no
    /// oracle distinguishing wrong-password from a tampered envelope.
    #[error("vault unlock failed")]
    VaultUnlockFailed,
    /// A vault envelope/sidecar I/O or non-unlock crypto failure, flattened to a
    /// message (never embeds key, password, or salt material).
    #[error("vault error: {0}")]
    Vault(String),
    /// A vault state transition not permitted by the §6.2.1 state machine
    /// (`personal-cfo-tg5`).
    #[error("illegal vault transition from {from:?} to {to:?}")]
    IllegalVaultTransition {
        /// The state the controller was in.
        from: VaultState,
        /// The state that was illegally requested.
        to: VaultState,
    },
}

impl From<vault_crypto::VaultCryptoError> for KernelError {
    fn from(error: vault_crypto::VaultCryptoError) -> Self {
        use vault_crypto::VaultCryptoError as E;
        match error {
            // Wrong password derives a wrong KEK → the AEAD tag fails to verify.
            E::KeyUnwrap => KernelError::VaultUnlockFailed,
            // Everything else is an internal/format failure; flatten to text.
            other => KernelError::Vault(other.to_string()),
        }
    }
}

impl From<DbError> for KernelError {
    fn from(error: DbError) -> Self {
        match error {
            DbError::MissingMetadata(field) => KernelError::MissingMetadata(field),
            DbError::WorkerUnavailable(state) => KernelError::Unavailable(state),
            DbError::WriterPanicked => KernelError::WriterPanicked,
            DbError::InvalidCommand(message) => KernelError::Validation(message),
            DbError::SelfTestFailed(message) => KernelError::Persistence(message),
            // Bind the inner error without naming `rusqlite`; flatten to text.
            DbError::Sqlite(inner) => KernelError::Persistence(inner.to_string()),
            // Attachment store (ADR 0023). Generic messages — no key bytes,
            // plaintext, or vault paths cross the boundary (§6.6).
            DbError::Io(_) => KernelError::Persistence("attachment blob I/O failed".into()),
            DbError::Crypto(_) => KernelError::Persistence("attachment cryptography failed".into()),
            DbError::KeyUnavailable => {
                KernelError::Persistence("operation requires an unlocked raw vault key".into())
            }
            DbError::AttachmentNotFound => KernelError::Validation("attachment not found".into()),
        }
    }
}

/// The Finance Kernel. Owns the persistence boundary; the only way to mutate
/// financial state.
pub struct Kernel {
    worker: DbWorker,
}

impl Kernel {
    /// Open a kernel backed by the encrypted vault at `path`.
    ///
    /// # Errors
    /// Returns [`KernelError`] if the underlying vault cannot be opened or fails
    /// its startup self-test.
    pub fn open(path: impl AsRef<Path>, key: &str) -> Result<Self, KernelError> {
        Ok(Self {
            worker: DbWorker::open(path, key)?,
        })
    }

    /// Build a kernel over an already-open db-worker.
    #[must_use]
    pub fn with_worker(worker: DbWorker) -> Self {
        Self { worker }
    }

    /// The health of the underlying writer.
    #[must_use]
    pub fn state(&self) -> WorkerState {
        self.worker.state()
    }

    /// Dispatch a command: validate, emit a boundary span, lower, and apply
    /// atomically (mutation + op-log) via the db-worker.
    ///
    /// # Errors
    /// Returns [`KernelError`] if validation fails, metadata is missing, the
    /// worker is unavailable, the write panics, or a persistence error occurs.
    pub fn dispatch<C: KernelCommand>(
        &self,
        envelope: CommandEnvelope<C>,
    ) -> Result<Outcome, KernelError> {
        let CommandEnvelope { meta, command } = envelope;
        let kind = command.kind();

        // Boundary span (plan §2.1). Only non-sensitive identifiers are
        // recorded — never monetary amounts or account contents.
        let span = tracing::info_span!(
            "kernel.dispatch",
            command.kind = kind,
            command.id = %meta.command_id,
            actor.type = ?meta.actor_type,
        );
        let _entered = span.enter();

        command.validate()?;
        tracing::info!(command.kind = kind, "dispatching kernel command");

        let write = command.lower();
        let outcome = self.worker.dispatch(meta, write)?;

        tracing::info!(?outcome, "kernel command applied");
        Ok(outcome)
    }

    /// Run an importer end to end (personal-cfo-cmx): parse the bytes in the
    /// bounded host (ADR 0022 §5), stage the result, dedupe, commit the clean
    /// transactions, and advance the batch state — the single path any source
    /// takes to the ledger. `target_account` is the account the import lands in;
    /// `meta` supplies the actor + correlation (each internal command gets a fresh
    /// idempotency key, sharing the correlation).
    ///
    /// # Errors
    /// Returns [`KernelError`] if a command fails to apply or persistence errors.
    pub fn ingest_batch(
        &self,
        plugin: &'static dyn ImporterPlugin,
        input: ParserInput,
        hints: &ParserHints,
        target_account: AccountId,
        limits: &ParserLimits,
        meta: &CommandMeta,
    ) -> Result<BatchResult, KernelError> {
        let fingerprint = content_fingerprint(&input.bytes);

        // File-level dedupe (ADR 0014 §3): an exact re-upload is skipped without
        // re-parsing.
        if let Some(existing) = self.worker.batch_with_fingerprint(&fingerprint)? {
            return Ok(BatchResult {
                source_batch_id: Some(existing.to_string()),
                status: "already_imported".to_owned(),
                staged: 0,
                committed: 0,
                flagged: 0,
                auto_categorized: 0,
            });
        }

        let source_name = input.filename.clone();
        let declared = input.declared_format.clone();
        let (parsed, report) = run_bounded(plugin, input, hints.clone(), limits);
        let source_type = parsed
            .as_ref()
            .map(|batch| batch.source_format.clone())
            .unwrap_or_else(|_| declared.unwrap_or_else(|| "other".to_owned()));

        let batch_id = SourceBatchId::new();
        self.dispatch(CommandEnvelope::new(
            next_meta(meta),
            CreateSourceBatch::new(
                batch_id,
                source_type,
                source_name,
                Some(fingerprint),
                Some(report.plugin_version.clone()),
            ),
        ))?;
        self.worker.record_parser_run(batch_id.as_uuid(), &report)?;

        let parsed = match parsed {
            Ok(batch) => batch,
            Err(_err) => {
                self.dispatch(CommandEnvelope::new(
                    next_meta(meta),
                    UpdateBatchState::new(batch_id, "failed", 0, 0, 0),
                ))?;
                return Ok(BatchResult {
                    source_batch_id: Some(batch_id.to_string()),
                    status: "failed".to_owned(),
                    staged: 0,
                    committed: 0,
                    flagged: 0,
                    auto_categorized: 0,
                });
            }
        };

        // Bulk-stage, then commit each clean transaction (duplicates self-flag).
        let staged_ids = self.worker.stage_parsed_batch(
            batch_id.as_uuid(),
            &parsed,
            target_account.as_uuid(),
        )?;
        for staged_id in &staged_ids {
            self.dispatch(CommandEnvelope::new(
                next_meta(meta),
                CommitStaged::new(StagedTransactionId::from_uuid(*staged_id)),
            ))?;
        }

        let (total, committed, flagged) = self.worker.staged_commit_tally(batch_id.as_uuid())?;
        let status = if flagged == 0 {
            "committed"
        } else {
            "partially_committed"
        };
        self.dispatch(CommandEnvelope::new(
            next_meta(meta),
            UpdateBatchState::new(
                batch_id,
                status,
                i64::from(total),
                i64::from(committed),
                i64::from(flagged),
            ),
        ))?;

        // Auto-apply merchant memory to the freshly imported rows (ADR 0030 addendum,
        // personal-cfo-5n4.2) when the setting is on and something committed. Reuses the
        // idempotent, uncategorized-only apply, so it never overwrites a user assignment.
        let auto_categorized = if committed > 0 && self.auto_categorize_on_import()? {
            self.worker.apply_merchant_memory()?
        } else {
            0
        };
        // Freshen the reconciliation seams (ADR 0026 addendum, xtz5): newly
        // committed rows may link projected occurrences and one-off entries,
        // and the forecast reads those projections without rebuilding them.
        // Best-effort: the rows are already committed, the projections are
        // derived and idempotent, and the next queue read heals a miss —
        // failing an otherwise-successful ingest here would buy nothing.
        if committed > 0 {
            if let Err(err) = self.worker.rebuild_recurring_instances() {
                tracing::warn!(error = %err, "post-ingest seam rebuild failed");
            }
        }

        Ok(BatchResult {
            source_batch_id: Some(batch_id.to_string()),
            status: status.to_owned(),
            staged: total,
            committed,
            flagged,
            auto_categorized,
        })
    }

    /// Ingest an already-parsed connector sync batch (personal-cfo-gglk):
    /// the connector-tier sibling of [`Self::ingest_batch`]. There is no file
    /// and no parse step — the adapter produced staged candidates — so the
    /// batch is created with **no** file fingerprint (schema-sanctioned for
    /// connector sources; per-transaction fingerprints carry all dedupe
    /// weight, which is what overlapping sync windows need), and the
    /// `parser_runs` provenance row records the adapter id + version exactly
    /// as importer plugin runs do (plan §8.1.3).
    ///
    /// `account_map` resolves connector account keys onto real accounts;
    /// records with no mapping are counted in `skipped_unmapped`, never
    /// silently dropped.
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence or dispatch failure.
    #[allow(clippy::too_many_lines)]
    pub fn ingest_sync_batch(
        &self,
        adapter_id: &str,
        adapter_version: &str,
        source_name: &str,
        parsed: &ParsedBatch,
        account_map: &std::collections::BTreeMap<String, uuid::Uuid>,
        meta: &CommandMeta,
    ) -> Result<SyncBatchResult, KernelError> {
        let batch_id = SourceBatchId::new();
        self.dispatch(CommandEnvelope::new(
            next_meta(meta),
            CreateSourceBatch::new(
                batch_id,
                adapter_id.to_owned(),
                Some(source_name.to_owned()),
                None,
                Some(adapter_version.to_owned()),
            ),
        ))?;
        self.worker.record_connector_run(
            batch_id.as_uuid(),
            adapter_id,
            adapter_version,
            parsed.records.len(),
        )?;

        let (staged_rows, skipped_unmapped) =
            self.worker
                .stage_sync_batch(batch_id.as_uuid(), parsed, account_map)?;
        for row in &staged_rows {
            // Connector overlap re-fetches are BY DESIGN (the since rewind),
            // so an exact fingerprint match against a row that is committed,
            // flagged, or skipped is a certain duplicate — skip it silently
            // instead of flagging it into the Money Inbox (which would flood
            // on every re-sync; a still-unresolved collision counts, tevp).
            let duplicate = self.worker.txn_fingerprint_already_tracked(
                &row.txn_fingerprint,
                row.account_id,
                row.staged_id,
            )?;
            let staged = StagedTransactionId::from_uuid(row.staged_id);
            if duplicate {
                self.dispatch(CommandEnvelope::new(
                    next_meta(meta),
                    SkipStaged::new(staged),
                ))?;
            } else {
                self.dispatch(CommandEnvelope::new(
                    next_meta(meta),
                    CommitStaged::new(staged),
                ))?;
            }
        }

        let (total, committed, flagged) = self.worker.staged_commit_tally(batch_id.as_uuid())?;
        let status = if flagged == 0 {
            "committed"
        } else {
            "partially_committed"
        };
        self.dispatch(CommandEnvelope::new(
            next_meta(meta),
            UpdateBatchState::new(
                batch_id,
                status,
                i64::from(total),
                i64::from(committed),
                i64::from(flagged),
            ),
        ))?;

        let auto_categorized = if committed > 0 && self.auto_categorize_on_import()? {
            self.worker.apply_merchant_memory()?
        } else {
            0
        };
        // Freshen the reconciliation seams (ADR 0026 addendum, xtz5): newly
        // committed rows may link projected occurrences and one-off entries,
        // and the forecast reads those projections without rebuilding them.
        // Best-effort: the rows are already committed, the projections are
        // derived and idempotent, and the next queue read heals a miss —
        // failing an otherwise-successful ingest here would buy nothing.
        if committed > 0 {
            if let Err(err) = self.worker.rebuild_recurring_instances() {
                tracing::warn!(error = %err, "post-ingest seam rebuild failed");
            }
        }

        Ok(SyncBatchResult {
            batch: BatchResult {
                source_batch_id: Some(batch_id.to_string()),
                status: status.to_owned(),
                staged: total,
                committed,
                flagged,
                auto_categorized,
            },
            skipped_unmapped,
        })
    }

    /// Connector-connection store passthroughs (personal-cfo-gglk):
    /// configuration, not ledger mutations — direct worker writes, no op-log.
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn create_connector_connection(
        &self,
        id: uuid::Uuid,
        adapter_id: &str,
        credential: &str,
        display_hint: Option<&str>,
    ) -> Result<(), KernelError> {
        Ok(self
            .worker
            .create_connector_connection(id, adapter_id, credential, display_hint)?)
    }

    /// Every connector connection (credential omitted by construction).
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn connector_connections(
        &self,
    ) -> Result<Vec<db_worker::ConnectorConnectionRow>, KernelError> {
        Ok(self.worker.connector_connections()?)
    }

    /// Whether a connection exists (credential never touched).
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn connector_connection_exists(&self, id: uuid::Uuid) -> Result<bool, KernelError> {
        Ok(self.worker.connector_connection_exists(id)?)
    }

    /// `(source_type, parser_name, parser_version)` for a sync batch.
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn source_batch_provenance(
        &self,
        batch_id: uuid::Uuid,
    ) -> Result<Option<(String, String, String)>, KernelError> {
        Ok(self.worker.source_batch_provenance(batch_id)?)
    }

    /// The stored credential for one connection — treat as a secret at once.
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn connector_credential(&self, id: uuid::Uuid) -> Result<Option<String>, KernelError> {
        Ok(self.worker.connector_credential(id)?)
    }

    /// Forget a connection and its account links (past synced data stays).
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn delete_connector_connection(&self, id: uuid::Uuid) -> Result<(), KernelError> {
        Ok(self.worker.delete_connector_connection(id)?)
    }

    /// Upsert a discovered external account for a connection.
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn upsert_connector_link(
        &self,
        connection_id: uuid::Uuid,
        external_id: &str,
        external_name: Option<&str>,
    ) -> Result<(), KernelError> {
        Ok(self
            .worker
            .upsert_connector_link(connection_id, external_id, external_name)?)
    }

    /// Map (or unmap) an external account onto a real account.
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn set_connector_link_account(
        &self,
        connection_id: uuid::Uuid,
        external_id: &str,
        account_id: Option<uuid::Uuid>,
    ) -> Result<(), KernelError> {
        Ok(self
            .worker
            .set_connector_link_account(connection_id, external_id, account_id)?)
    }

    /// Every account link for a connection.
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn connector_links(
        &self,
        connection_id: uuid::Uuid,
    ) -> Result<Vec<db_worker::ConnectorLinkRow>, KernelError> {
        Ok(self.worker.connector_links(connection_id)?)
    }

    /// Record a sync attempt's outcome on the connection row.
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn record_connector_sync(
        &self,
        connection_id: uuid::Uuid,
        error: Option<&str>,
    ) -> Result<(), KernelError> {
        Ok(self.worker.record_connector_sync(connection_id, error)?)
    }

    /// Advance the listed links' since-watermarks (the caller computes the
    /// advance set: phase-1 mapped ∩ provider response − retry-held).
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn advance_connector_watermarks(
        &self,
        connection_id: uuid::Uuid,
        synced_on: &str,
        advance_external_ids: &[String],
    ) -> Result<(), KernelError> {
        Ok(self.worker.advance_connector_watermarks(
            connection_id,
            synced_on,
            advance_external_ids,
        )?)
    }

    /// Clear a stale connection error without stamping `last_synced_at`.
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn clear_connector_error(&self, connection_id: uuid::Uuid) -> Result<(), KernelError> {
        Ok(self.worker.clear_connector_error(connection_id)?)
    }

    /// Record a sync failure without consuming the debounce window.
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn record_connector_error(
        &self,
        connection_id: uuid::Uuid,
        error: &str,
    ) -> Result<(), KernelError> {
        Ok(self.worker.record_connector_error(connection_id, error)?)
    }

    /// Total number of accounts (typed read view; no database types exposed).
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn account_count(&self) -> Result<u64, KernelError> {
        Ok(self.worker.account_count()?)
    }

    /// The linked SQLCipher version (`PRAGMA cipher_version`), e.g.
    /// `"4.5.7 community"`. Pair with [`sqlite_version`] for the full engine
    /// fingerprint. See [`db_worker::DbWorker::cipher_version`].
    ///
    /// # Errors
    /// Returns [`KernelError`] if the version cannot be read.
    pub fn cipher_version(&self) -> Result<String, KernelError> {
        Ok(self.worker.cipher_version()?)
    }

    /// Record the user's acknowledgement of the no-password-reset warning as an
    /// immutable audit event (ADR 0002 / personal-cfo-n7bo). Called during
    /// onboarding once the user checks the confirmation box; the `meta` carries
    /// the command + correlation provenance for the audit record.
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn acknowledge_no_reset_warning(&self, meta: &CommandMeta) -> Result<(), KernelError> {
        self.worker
            .record_audit_event(meta, NO_RESET_WARNING_ACKNOWLEDGED)?;
        Ok(())
    }

    /// The number of recorded audit events of `event_type` (personal-cfo-n7bo).
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn audit_event_count(&self, event_type: &str) -> Result<u64, KernelError> {
        Ok(self.worker.audit_event_count(event_type)?)
    }

    /// Read an app-level setting's value, or `None` when unset (personal-cfo-p5g).
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn get_setting(&self, key: &str) -> Result<Option<String>, KernelError> {
        Ok(self.worker.get_setting(key)?)
    }

    /// Upsert an app-level setting (personal-cfo-p5g). Settings are configuration,
    /// not ledger writes, so they bypass the [`KernelCommand`] bus.
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), KernelError> {
        self.worker.set_setting(key, value)?;
        Ok(())
    }

    /// Whether an account with `id` exists.
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn account_exists(&self, id: AccountId) -> Result<bool, KernelError> {
        Ok(self.worker.account_exists(id)?)
    }

    /// The balance of an account, derived from its ledger postings. `None` if
    /// the account does not exist.
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn account_balance(&self, id: AccountId) -> Result<Option<Money>, KernelError> {
        Ok(self.worker.account_balance(id)?)
    }

    /// Record a manual balance assertion (ADR 0027, personal-cfo-ueg6): set the
    /// account's balance directly as of a date, with no transaction. The currency
    /// must match the account's. A forecast input, so it bypasses the
    /// [`KernelCommand`] bus.
    ///
    /// # Errors
    /// Returns [`KernelError`] if the account does not exist, the currency
    /// mismatches, or on a persistence failure.
    pub fn record_balance_assertion(
        &self,
        id: Uuid,
        account_id: AccountId,
        amount: Money,
        as_of: NaiveDate,
    ) -> Result<(), KernelError> {
        self.worker
            .record_balance_assertion(id, account_id, amount, as_of)?;
        Ok(())
    }

    /// The derived auto-reconciling adjustment ("plug", ADR 0027) for an account:
    /// the still-unexplained amount of its latest assertion, or `None` if it has
    /// no assertion.
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn account_unexplained(&self, account_id: AccountId) -> Result<Option<Money>, KernelError> {
        Ok(self.worker.account_unexplained(account_id)?)
    }

    /// The read-model row for an account, rebuilt from canonical tables. `None`
    /// if the account does not exist.
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn account_view(&self, id: AccountId) -> Result<Option<AccountView>, KernelError> {
        Ok(self.worker.account_view(id)?)
    }

    /// Every account's read-model row, ordered by name. Backs the accounts list
    /// UI (personal-cfo-0eft).
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn account_views(&self) -> Result<Vec<AccountView>, KernelError> {
        Ok(self.worker.account_views()?)
    }

    /// The type-based cash-tier rollups (ADR 0028, personal-cfo-9dgg): spendable /
    /// reserve / net cash derived from liquid accounts' subtypes.
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure or mixed liquid currencies.
    pub fn cash_tiers(&self) -> Result<CashTiers, KernelError> {
        Ok(self.worker.cash_tiers()?)
    }

    /// The household cash-availability snapshot (ADR 0029, personal-cfo-fqbm):
    /// ledger / available / pending / committed / headroom per liquid account plus
    /// the net rollup + minimum-cash-floor status.
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure, mixed liquid currencies, or
    /// a forecast arithmetic failure.
    pub fn cash_availability(&self) -> Result<CashAvailability, KernelError> {
        Ok(self.worker.cash_availability()?)
    }

    /// The household liquid-cash comfort band (ADR 0018 addendum 915.1, personal-cfo-3v6d).
    ///
    /// # Errors
    /// Returns [`KernelError`] on a read failure.
    pub fn comfort_band(&self) -> Result<ComfortBand, KernelError> {
        Ok(self.worker.comfort_band()?)
    }

    /// The current comfort-band drift signal — why the projection is set to cross below the band
    /// (personal-cfo-5ie.8, ADR 0018 §915.1). `as_of` is the caller's clock read.
    ///
    /// # Errors
    /// Returns [`KernelError`] on a read/forecast failure.
    pub fn band_drift(
        &self,
        as_of: DateTime<Utc>,
        horizon_days: u32,
    ) -> Result<Option<BandDriftView>, KernelError> {
        Ok(self.worker.band_drift(as_of, horizon_days)?)
    }

    /// The R1 Forecast Readiness score (ADR 0026 §13, personal-cfo-6vj9): a 0–100
    /// data-maturity indicator (coverage + balance freshness + explained ratio) with
    /// a per-factor breakdown.
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn forecast_readiness(&self) -> Result<ForecastReadiness, KernelError> {
        Ok(self.worker.forecast_readiness()?)
    }

    /// Forecast capabilities that have self-activated but whose one-time unlock notice the
    /// user has not yet acknowledged (ADR 0026 §10, personal-cfo-egon).
    ///
    /// # Errors
    /// Returns [`KernelError`] on a read failure.
    pub fn pending_capability_unlocks(&self) -> Result<Vec<CapabilityUnlock>, KernelError> {
        Ok(self.worker.pending_capability_unlocks()?)
    }

    /// Record the user's acknowledgement of a capability-unlock notice, so it fires exactly
    /// once (ADR 0026 §10). `meta` carries the command + correlation provenance.
    ///
    /// # Errors
    /// Returns [`KernelError`] for an unknown capability key or a persistence failure.
    pub fn acknowledge_capability(&self, meta: &CommandMeta, key: &str) -> Result<(), KernelError> {
        self.worker.acknowledge_capability(meta, key)?;
        Ok(())
    }

    /// Apply merchant-memory auto-categorization (ADR 0030 addendum, personal-cfo-7yh0):
    /// learn merchant→category from manual categorizations and fill uncategorized
    /// transactions of the same merchant. Returns the number newly categorized.
    ///
    /// # Errors
    /// Returns [`KernelError`] on a read/write failure.
    pub fn apply_merchant_memory(&self) -> Result<u32, KernelError> {
        Ok(self.worker.apply_merchant_memory()?)
    }

    /// Whether merchant memory auto-applies after an import (ADR 0030 addendum,
    /// personal-cfo-5n4.2). Defaults to `true` when the setting has never been set.
    ///
    /// # Errors
    /// Returns [`KernelError`] on a read failure.
    pub fn auto_categorize_on_import(&self) -> Result<bool, KernelError> {
        Ok(self
            .worker
            .get_setting(AUTO_CATEGORIZE_ON_IMPORT_KEY)?
            .is_none_or(|value| value == "true"))
    }

    /// Set whether merchant memory auto-applies after an import (ADR 0030 addendum,
    /// personal-cfo-5n4.2).
    ///
    /// # Errors
    /// Returns [`KernelError`] on a write failure.
    pub fn set_auto_categorize_on_import(&self, enabled: bool) -> Result<(), KernelError> {
        self.worker.set_setting(
            AUTO_CATEGORIZE_ON_IMPORT_KEY,
            if enabled { "true" } else { "false" },
        )?;
        Ok(())
    }

    /// The Future Cash chart's stored series-selection preference — an opaque JSON
    /// string the frontend serializes (personal-cfo-4d8.25.26), or `None` when never
    /// set (the frontend then applies its default of the three aggregate tiers).
    ///
    /// # Errors
    /// Returns [`KernelError`] on a read failure.
    pub fn future_cash_series_selection(&self) -> Result<Option<String>, KernelError> {
        Ok(self.worker.get_setting(FUTURE_CASH_SERIES_KEY)?)
    }

    /// Persist the Future Cash chart's series-selection preference (opaque JSON).
    ///
    /// # Errors
    /// Returns [`KernelError`] on a write failure.
    pub fn set_future_cash_series_selection(&self, selection: &str) -> Result<(), KernelError> {
        self.worker.set_setting(FUTURE_CASH_SERIES_KEY, selection)?;
        Ok(())
    }

    /// Attach a document to a transaction (ADR 0023): encrypt + store `bytes`,
    /// link the resulting attachment to the transaction, and return its metadata.
    ///
    /// # Errors
    /// [`KernelError`] if the vault key is unavailable or storage fails.
    pub fn attach_document(
        &self,
        transaction_id: TransactionId,
        bytes: &[u8],
        mime_type: Option<&str>,
        original_filename: Option<&str>,
    ) -> Result<AttachmentMeta, KernelError> {
        let id = self
            .worker
            .import_attachment(bytes, mime_type, original_filename)?;
        self.worker
            .link_attachment(id, "transaction", transaction_id.as_uuid())?;
        self.worker
            .attachments_for("transaction", transaction_id.as_uuid())?
            .into_iter()
            .find(|m| m.id == id)
            .ok_or_else(|| KernelError::Persistence("attachment missing after attach".into()))
    }

    /// The documents attached to a transaction (metadata only).
    ///
    /// # Errors
    /// [`KernelError`] on a persistence failure.
    pub fn transaction_attachments(
        &self,
        transaction_id: TransactionId,
    ) -> Result<Vec<AttachmentMeta>, KernelError> {
        Ok(self
            .worker
            .attachments_for("transaction", transaction_id.as_uuid())?)
    }

    /// The raw imported source fields behind a committed transaction (ADR 0045 §2):
    /// every column the importer captured, resolved through its provenance link.
    /// `None` for a manually-entered transaction.
    pub fn imported_transaction_fields(
        &self,
        transaction_id: TransactionId,
    ) -> Result<Option<ImportedTransactionFields>, KernelError> {
        Ok(self
            .worker
            .imported_transaction_fields(transaction_id.as_uuid())?)
    }

    /// Detach a document from a transaction; removing the last link
    /// crypto-shreds the attachment (ADR 0023).
    ///
    /// # Errors
    /// [`KernelError`] on a persistence failure.
    pub fn remove_attachment(
        &self,
        attachment_id: AttachmentId,
        transaction_id: TransactionId,
    ) -> Result<(), KernelError> {
        Ok(self
            .worker
            .unlink_attachment(attachment_id, "transaction", transaction_id.as_uuid())?)
    }

    /// The most recent transactions (newest first, capped at `limit`), for the
    /// transactions list UI (personal-cfo-idsd).
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn transactions(&self, limit: u32) -> Result<Vec<TransactionRow>, KernelError> {
        Ok(self.worker.recent_transactions(limit)?)
    }

    /// One filtered, ordered page of transactions plus the total match count
    /// (personal-cfo-3fdd.1) — server-side search / filter / sort / paging for the
    /// transactions list and the global search palette.
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn transaction_page(
        &self,
        query: &TransactionPageQuery,
    ) -> Result<TransactionPage, KernelError> {
        Ok(self.worker.transaction_page(query)?)
    }

    /// Every income source with its next pay date (personal-cfo-le79).
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn income_source_views(&self) -> Result<Vec<IncomeSourceView>, KernelError> {
        Ok(self.worker.income_source_views()?)
    }

    /// Every manual recurring bill with its next due date (personal-cfo-esmy).
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn recurring_bill_views(&self) -> Result<Vec<RecurringBillView>, KernelError> {
        Ok(self.worker.recurring_bill_views()?)
    }

    /// An account's debt terms (ADR 0035 §5, personal-cfo-6wk.6), or `None` if unset.
    ///
    /// # Errors
    /// Returns [`KernelError`] if the read fails.
    pub fn debt_terms(&self, account_id: AccountId) -> Result<Option<DebtTermsView>, KernelError> {
        Ok(self.worker.debt_terms(account_id)?)
    }

    /// Debt terms for every account that has them, optionally scoped to an account set
    /// (personal-cfo-4d17). Accounts without terms are omitted rather than returned empty:
    /// "no rate recorded" and "no interest" are different facts.
    ///
    /// # Errors
    /// Returns [`KernelError`] if the read fails.
    pub fn debt_terms_list(
        &self,
        account_ids: &[uuid::Uuid],
    ) -> Result<Vec<DebtTermsView>, KernelError> {
        Ok(self.worker.debt_terms_list(account_ids)?)
    }

    /// Every recurring transfer with its next occurrence (ADR 0026 §14,
    /// personal-cfo-npoe).
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn recurring_transfer_views(&self) -> Result<Vec<RecurringTransferView>, KernelError> {
        Ok(self.worker.recurring_transfer_views()?)
    }

    /// Every commitment — the forecast-facing obligation projection consumed by
    /// the Future Cash ledger (personal-cfo-rxw).
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn commitment_views(&self) -> Result<Vec<CommitmentView>, KernelError> {
        Ok(self.worker.commitment_views()?)
    }

    /// The full category taxonomy (plan §9.6, ADR 0030, personal-cfo-bac).
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn category_views(&self) -> Result<Vec<CategoryView>, KernelError> {
        Ok(self.worker.category_views()?)
    }

    /// Every tag, for the tag picker (ADR 0033, personal-cfo-2ryf).
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn tag_views(&self) -> Result<Vec<TagView>, KernelError> {
        Ok(self.worker.tag_views()?)
    }

    /// A transaction's split lines, ordered (ADR 0034, personal-cfo-kr9).
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn transaction_splits(
        &self,
        transaction_id: TransactionId,
    ) -> Result<Vec<SplitLineView>, KernelError> {
        Ok(self.worker.transaction_splits(transaction_id)?)
    }

    /// The committed counterpart(s) a flagged staged transaction may duplicate, for the
    /// duplicate Review panel (ADR 0032 §4, personal-cfo-4d8.20).
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn duplicate_candidates(
        &self,
        staged_transaction_id: StagedTransactionId,
    ) -> Result<Vec<TransactionRow>, KernelError> {
        Ok(self.worker.duplicate_candidates(staged_transaction_id)?)
    }

    /// Stable content checksum of the commitments projection — used by the
    /// restore-drill regression to prove a restored vault reproduces the
    /// read model byte-for-byte (personal-cfo-7pfu).
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn commitments_checksum(&self) -> Result<u64, KernelError> {
        Ok(self.worker.commitments_checksum()?)
    }

    /// Stable content checksum of the transaction-display read model
    /// (personal-cfo-7pfu).
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn transaction_display_checksum(&self) -> Result<u64, KernelError> {
        Ok(self.worker.transaction_display_checksum()?)
    }

    /// Decrypt an attachment's bytes in memory (ADR 0023) — used by the restore
    /// drill to prove a restored blob still decrypts to the original
    /// (personal-cfo-7pfu).
    ///
    /// # Errors
    /// Returns [`KernelError`] if the id is unknown or decryption fails.
    pub fn read_attachment_bytes(
        &self,
        attachment_id: AttachmentId,
    ) -> Result<Vec<u8>, KernelError> {
        Ok(self.worker.read_attachment_bytes(attachment_id)?)
    }

    /// The deterministic Future Cash forecast over the next `horizon_days`
    /// (plan §13.3, personal-cfo-164u): liquid-cash opening balance folded with
    /// the income + recurring-obligation event stream, one row per day. Backs
    /// the dashboard's Future Cash widgets (personal-cfo-1vd7).
    ///
    /// `scenario` selects the overlay: `None` is the base forecast; `Some(id)`
    /// layers that scenario's scoped events over the base (personal-cfo-6zep).
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure or a forecast
    /// computation error (malformed schedule, mixed-currency liquid accounts).
    pub fn future_cash_forecast(
        &self,
        horizon_days: u32,
        scenarios: &[Uuid],
    ) -> Result<ForecastView, KernelError> {
        Ok(self.worker.future_cash_forecast(horizon_days, scenarios)?)
    }

    /// The per-account and per-group Future Cash projection (ADR 0026 §12,
    /// personal-cfo-l8oh) — the multi-series foundation for the chart + table.
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure, a malformed schedule,
    /// mixed-currency liquid accounts, or a forecast arithmetic failure.
    pub fn future_cash_by_account(
        &self,
        horizon_days: u32,
        scenarios: &[Uuid],
    ) -> Result<MultiSeriesForecast, KernelError> {
        Ok(self
            .worker
            .future_cash_by_account(horizon_days, scenarios)?)
    }

    /// The realized cash-flow HISTORY over the trailing `lookback_days` (cf-history,
    /// personal-cfo-4d8.27.5.2) — each liquid account's actual daily closing balance,
    /// folded backward and clamped to its earliest real data. Backs the Account Detail chart.
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure or mixed-currency liquid accounts.
    pub fn cash_flow_history(&self, lookback_days: u32) -> Result<CashFlowHistory, KernelError> {
        Ok(self.worker.cash_flow_history(lookback_days)?)
    }

    /// The per-card credit-card statement + payment forecast (ADR 0039 §2,
    /// personal-cfo-4lhm).
    ///
    /// # Errors
    /// Returns [`KernelError`] on a read failure or a malformed stored schedule.
    pub fn card_statement_forecast(&self) -> Result<Vec<CardStatementForecastView>, KernelError> {
        Ok(self.worker.card_statement_forecast()?)
    }

    /// The past billing-cycle windows for one card with derived-from-imports charge totals
    /// and any recorded actual statements — the statement-history capture surface
    /// (ADR 0039 addendum 2026-07-10 §2, personal-cfo-4d8.25.4).
    ///
    /// # Errors
    /// Returns [`KernelError`] on a read failure.
    pub fn card_statement_history(
        &self,
        account_id: AccountId,
    ) -> Result<Vec<CardStatementHistoryView>, KernelError> {
        Ok(self.worker.card_statement_history(account_id)?)
    }

    /// Compare debt-paydown strategies (minimum-only / snowball / avalanche) for the current
    /// debts at `extra_budget_minor` extra per month (ADR 0036 debt_payoff, personal-cfo-od07).
    ///
    /// `account_ids` scopes the comparison; **empty means every debt**. The scope reaches the
    /// SIMULATION, not its output: snowball and avalanche order the debts and route the extra
    /// budget among them, so filtering results afterwards would report a payoff order and a
    /// debt-free month the selected debts do not have (personal-cfo-4d8.27.9.7).
    ///
    /// # Errors
    /// Returns [`KernelError`] on a read failure.
    /// Obligations whose scheduled date has passed with nothing recorded against them
    /// (personal-cfo-4d8.27.7.6, ADR 0058). Confirmation is proved by
    /// `confirmed_obligations`, not by the instance row's status — see the db-worker doc.
    ///
    /// # Errors
    /// Returns [`KernelError`] if the read fails.
    pub fn unconfirmed_past_due(&self) -> Result<Vec<UnconfirmedOccurrence>, KernelError> {
        Ok(self.worker.unconfirmed_past_due()?)
    }

    pub fn debt_payoff_comparison(
        &self,
        extra_budget_minor: i64,
        account_ids: &[uuid::Uuid],
    ) -> Result<Vec<PayoffPlanView>, KernelError> {
        Ok(self
            .worker
            .debt_payoff_comparison(extra_budget_minor, account_ids)?)
    }

    /// Suspected loan double-counts — a loan tracked as both a `loan_liability` account with
    /// payment terms and an active recurring `loan_payment` bill (personal-cfo-6wk.11). A
    /// descriptive warning (ADR 0018); the app never auto-removes either side.
    ///
    /// # Errors
    /// Returns [`KernelError`] on a read failure.
    pub fn loan_double_count_warnings(&self) -> Result<Vec<LoanDoubleCount>, KernelError> {
        Ok(self.worker.loan_double_count_warnings()?)
    }

    /// Candidate recurring bills detected from realized outflows (personal-cfo-98ql) — a merchant
    /// that recurs at a consistent cadence + amount. Suggestions only; the user confirms before a
    /// recurring event is created.
    ///
    /// # Errors
    /// Returns [`KernelError`] on a read failure.
    pub fn recurring_candidates(&self) -> Result<Vec<RecurringCandidateView>, KernelError> {
        Ok(self.worker.recurring_candidates()?)
    }

    /// Recurring inbound deposits not yet modeled as income sources (gmnk).
    ///
    /// # Errors
    /// Returns [`KernelError`] on a read failure.
    pub fn income_candidates(&self) -> Result<Vec<RecurringCandidateView>, KernelError> {
        Ok(self.worker.income_candidates()?)
    }

    /// Activate the persisted forecast pipeline on vault open (ADR 0026 §15,
    /// personal-cfo-5ie.3): persist at most one reproducible run per input-state
    /// per day, so the actualization loop has a time-spanning run history. A no-op
    /// (`Ok(None)`) when a run already exists for today's inputs. Best-effort at the
    /// unlock seam — callers log failures rather than failing the open.
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure or a forecast arithmetic failure.
    pub fn persist_daily_forecast(&self) -> Result<Option<Uuid>, KernelError> {
        Ok(self.worker.persist_daily_forecast()?)
    }

    /// Number of persisted forecast runs (ADR 0026 §3/§15, personal-cfo-5ie.3) —
    /// the observability seam for daily-on-open activation.
    ///
    /// # Errors
    /// Returns [`KernelError`] on a read failure.
    pub fn persisted_forecast_run_count(&self) -> Result<u64, KernelError> {
        Ok(self.worker.persisted_forecast_run_count()?)
    }

    /// Rebuild the recurring-instance read model + link realized transactions
    /// (ADR 0026 §9, personal-cfo-5ie.4/-5ie.5). Returns the instance count.
    ///
    /// # Errors
    /// Returns [`KernelError`] if the projection fails or a schedule is malformed.
    pub fn rebuild_recurring_instances(&self) -> Result<u64, KernelError> {
        Ok(self.worker.rebuild_recurring_instances()?)
    }

    /// Read the projected recurring instances (ADR 0026 §9). Rebuild first when
    /// fresh data is required.
    ///
    /// # Errors
    /// Returns [`KernelError`] if the read fails.
    pub fn recurring_instances(&self) -> Result<Vec<RecurringInstanceRow>, KernelError> {
        Ok(self.worker.recurring_instances()?)
    }

    /// One recurring bill's projected instances after refreshing the linking seam — the
    /// retro-attach surface (ADR 0047 §1, personal-cfo-4d8.25.8).
    ///
    /// # Errors
    /// Returns [`KernelError`] if the rebuild or read fails.
    pub fn recurring_bill_history(
        &self,
        event_id: RecurringEventId,
    ) -> Result<Vec<RecurringInstanceRow>, KernelError> {
        Ok(self.worker.recurring_bill_history(event_id)?)
    }

    /// Actualize the persisted forecast runs against realized transactions (ADR 0026
    /// §9/§17, personal-cfo-46jq): refresh the recurring-instance seam, then
    /// recompute `forecast_actuals` (exact/matched/missed/superseded). Returns the
    /// number of actuals written.
    ///
    /// # Errors
    /// Returns [`KernelError`] if the projection or scoring fails.
    pub fn actualize_forecasts(&self) -> Result<u64, KernelError> {
        Ok(self.worker.actualize_forecasts()?)
    }

    /// Number of `forecast_actuals` rows (ADR 0026 §9) — the actualization
    /// observability seam the readiness factors (A3) read.
    ///
    /// # Errors
    /// Returns [`KernelError`] on a read failure.
    pub fn forecast_actuals_count(&self) -> Result<u64, KernelError> {
        Ok(self.worker.forecast_actuals_count()?)
    }

    /// Backtest the persisted forecast runs against realized actuals (ADR 0026 §18,
    /// personal-cfo-nxgx): record the per-vault MAPE in `forecast_backtest_results`,
    /// feeding the "Forecast accuracy" readiness factor. Returns rows written (0 or 1).
    ///
    /// # Errors
    /// Returns [`KernelError`] if the read/write fails.
    pub fn backtest_forecasts(&self) -> Result<u64, KernelError> {
        Ok(self.worker.backtest_forecasts()?)
    }

    /// Record a manual future entry — a one-time signed cash event on a date
    /// (personal-cfo-q6gh). A forecast input, so it bypasses the [`KernelCommand`]
    /// bus.
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn record_manual_entry(
        &self,
        id: Uuid,
        amount: Money,
        occurs_on: NaiveDate,
        label: &str,
        account_id: Option<Uuid>,
    ) -> Result<(), KernelError> {
        self.worker
            .record_manual_entry(id, amount, occurs_on, label, account_id)?;
        Ok(())
    }

    /// The active base manual future entries (personal-cfo-q6gh).
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure or malformed stored params.
    pub fn manual_entries(&self) -> Result<Vec<ManualEntry>, KernelError> {
        Ok(self.worker.manual_entries()?)
    }

    /// Supersede an assumption event with a replacement (personal-cfo-5u2) — the
    /// "edit" of a manual entry: no silent mutation, history retained.
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn supersede_assumption_event(
        &self,
        id: Uuid,
        superseded_by: Uuid,
    ) -> Result<(), KernelError> {
        self.worker.supersede_assumption_event(id, superseded_by)?;
        Ok(())
    }

    /// Clear an assumption event (personal-cfo-5u2) — the "delete" of a manual
    /// entry: the row is retained, just deactivated.
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn clear_assumption_event(&self, id: Uuid) -> Result<(), KernelError> {
        self.worker.clear_assumption_event(id)?;
        Ok(())
    }

    /// Record a typed forecast assumption event (personal-cfo-6zep) — the
    /// generalized creator behind `create_forecast_assumption` (additions,
    /// amount/date modifications, removals), base or scenario-scoped.
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn record_forecast_assumption(
        &self,
        spec: &ForecastAssumptionSpec,
    ) -> Result<(), KernelError> {
        self.worker.record_forecast_assumption(spec)?;
        Ok(())
    }

    /// The active assumption events for a scenario (personal-cfo-5u2): `None` =
    /// the base assumptions; `Some(id)` = exactly that scenario's events.
    ///
    /// # Errors
    /// Returns [`KernelError`] on a read failure or an unrecognized stored token.
    pub fn active_assumption_events(
        &self,
        scenario: Option<Uuid>,
    ) -> Result<Vec<AssumptionEventView>, KernelError> {
        Ok(self.worker.active_assumption_events(scenario)?)
    }

    /// Create a scenario (status `draft`) — a named overlay on the base forecast
    /// (ADR 0026 §5, personal-cfo-0mg/6zep).
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn create_scenario(&self, scenario: &NewScenario) -> Result<(), KernelError> {
        self.worker.create_scenario(scenario)?;
        Ok(())
    }

    /// Read a scenario by id (personal-cfo-6zep).
    ///
    /// # Errors
    /// Returns [`KernelError`] on a read failure or an unrecognized stored token.
    pub fn scenario(&self, id: Uuid) -> Result<Option<ScenarioView>, KernelError> {
        Ok(self.worker.scenario(id)?)
    }

    /// All scenarios, oldest first (personal-cfo-6zep).
    ///
    /// # Errors
    /// Returns [`KernelError`] on a read failure or an unrecognized stored token.
    pub fn list_scenarios(&self) -> Result<Vec<ScenarioView>, KernelError> {
        Ok(self.worker.list_scenarios()?)
    }

    /// Update a scenario's lifecycle status (draft → active → archived)
    /// (personal-cfo-6zep).
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn set_scenario_status(&self, id: Uuid, status: ScenarioStatus) -> Result<(), KernelError> {
        self.worker.set_scenario_status(id, status)?;
        Ok(())
    }

    /// Rename a scenario (personal-cfo-vru6).
    ///
    /// # Errors
    /// Returns [`KernelError`] on an empty name or a persistence failure.
    pub fn rename_scenario(&self, id: Uuid, name: &str) -> Result<(), KernelError> {
        self.worker.rename_scenario(id, name)?;
        Ok(())
    }

    /// Delete a scenario and its overlay events, permanently (ADR 0051 §1).
    ///
    /// Destructive by design and distinct from [`Self::archive_scenario`], which keeps
    /// everything. Scoped to the scenario's own overlay — base events are never touched.
    ///
    /// # Errors
    /// Returns [`KernelError`] if no such scenario exists, or on a persistence failure.
    pub fn delete_scenario(&self, id: Uuid) -> Result<(), KernelError> {
        self.worker.delete_scenario(id)?;
        Ok(())
    }

    /// Spend rolled up by category over a date range, at the children of `parent`,
    /// narrowed by the transaction list's facets (ADR 0052 §2 — the chart and the list
    /// read one filter state, or a drill-down disagrees with the bar it came from).
    ///
    /// # Errors
    /// Returns [`KernelError`] on a read failure.
    pub fn spend_by_category(
        &self,
        from: chrono::NaiveDate,
        to: chrono::NaiveDate,
        parent: Option<Uuid>,
        currency: &str,
        filters: &SpendFilters,
    ) -> Result<SpendBreakdown, KernelError> {
        Ok(self
            .worker
            .spend_by_category(from, to, parent, currency, filters)?)
    }

    /// Archive a scenario, keeping every event (ADR 0051 §1). Reversible.
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn archive_scenario(&self, id: Uuid) -> Result<(), KernelError> {
        self.worker.archive_scenario(id)?;
        Ok(())
    }

    /// Clone a scenario into a new draft carrying a copy of its active events
    /// (ADR 0051 §2), returning the new scenario's id.
    ///
    /// # Errors
    /// Returns [`KernelError`] if the source does not exist or the name is blank.
    pub fn clone_scenario(&self, id: Uuid, name: &str) -> Result<Uuid, KernelError> {
        let new_id = Uuid::now_v7();
        self.worker.clone_scenario(id, new_id, name)?;
        Ok(new_id)
    }

    /// Set (or clear) a scenario's expiry date (ADR 0051 §3).
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn set_scenario_expiry(
        &self,
        id: Uuid,
        expires_on: Option<&str>,
    ) -> Result<(), KernelError> {
        self.worker.set_scenario_expiry(id, expires_on)?;
        Ok(())
    }

    /// The vault's identity + configuration (plan §9.2, personal-cfo-2lm).
    ///
    /// # Errors
    /// Returns [`KernelError`] if the metadata cannot be read (e.g. the
    /// singleton row is missing).
    pub fn vault_metadata(&self) -> Result<VaultMetadata, KernelError> {
        Ok(self.worker.vault_metadata()?)
    }

    /// Set the household's IANA timezone (`personal-cfo-q329`) — ADR 0021 §1's
    /// calendar-boundary authority, previously unsettable after vault creation.
    ///
    /// # Errors
    /// Returns [`KernelError`] if `tz` is not a valid IANA timezone name, or the write
    /// fails.
    pub fn set_household_timezone(&self, tz: &str) -> Result<(), KernelError> {
        self.worker.set_household_timezone(tz)?;
        Ok(())
    }

    /// Rebuild the transaction-display read model from canonical tables.
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn rebuild_transaction_display(&self) -> Result<u64, KernelError> {
        Ok(self.worker.rebuild_transaction_display()?)
    }

    /// Rebuild the commitments read model from canonical tables — the other repair
    /// the recovery wizard offers for read-model drift (personal-cfo-5ivp).
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn rebuild_commitments(&self) -> Result<u64, KernelError> {
        Ok(self.worker.rebuild_commitments()?)
    }

    /// Every active Money Inbox item — the triage surface for the import/commit
    /// exceptions the pipeline could not auto-resolve (ADR 0014 §7, personal-cfo-dsq).
    ///
    /// # Errors
    /// Returns [`KernelError`] if the read fails.
    pub fn money_inbox_list(&self) -> Result<Vec<MoneyInboxItem>, KernelError> {
        Ok(self.worker.money_inbox_list()?)
    }

    /// Rebuild the Money Inbox read model from canonical state — the repair path
    /// for read-model drift (the commit pipeline refreshes it incrementally).
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn rebuild_money_inbox(&self) -> Result<u64, KernelError> {
        Ok(self.worker.rebuild_money_inbox()?)
    }

    /// Bulk-accept every transaction in the low-confidence-category review queue (ADR 0030
    /// addendum, personal-cfo-j5ij): mark each reviewed, which keeps its `source = rule`
    /// category but drops it from the queue. Dispatched one [`MarkReviewed`] per row through
    /// the command path (like the import commit loop), so each is event-sourced + audited;
    /// `seed` provides the correlation/actor. Returns the number accepted.
    ///
    /// # Errors
    /// Returns [`KernelError`] on a read or write failure.
    pub fn accept_low_confidence_categories(&self, seed: &CommandMeta) -> Result<u32, KernelError> {
        let ids = self.worker.low_confidence_category_transaction_ids()?;
        for id in &ids {
            self.dispatch(CommandEnvelope::new(
                next_meta(seed),
                MarkReviewed::new(TransactionId::from_uuid(*id), true),
            ))?;
        }
        Ok(u32::try_from(ids.len()).unwrap_or(u32::MAX))
    }

    /// Bulk mark an explicit transaction set reviewed (personal-cfo-4d8.25.16): one
    /// [`MarkReviewed`] per id through the command path — each event-sourced + audited,
    /// all sharing `seed`'s correlation id as the batch key — exactly the
    /// [`Self::accept_low_confidence_categories`] shape, so a whole-inbox sweep is one
    /// IPC round-trip instead of a frontend fan-out. Returns the number marked.
    ///
    /// # Errors
    /// Returns [`KernelError`] on the first failed dispatch (earlier marks remain
    /// applied and audited; the caller re-reads the inbox either way).
    pub fn mark_transactions_reviewed_bulk(
        &self,
        seed: &CommandMeta,
        ids: &[TransactionId],
    ) -> Result<u32, KernelError> {
        for id in ids {
            self.dispatch(CommandEnvelope::new(
                next_meta(seed),
                MarkReviewed::new(*id, true),
            ))?;
        }
        Ok(u32::try_from(ids.len()).unwrap_or(u32::MAX))
    }

    /// The row DTOs for an explicit id set (personal-cfo-4d8.25.15).
    ///
    /// # Errors
    /// Returns [`KernelError`] on a read failure.
    pub fn transactions_by_ids(
        &self,
        ids: &[TransactionId],
    ) -> Result<Vec<TransactionRow>, KernelError> {
        Ok(self.worker.transactions_by_ids(ids)?)
    }

    /// Incrementally project new operations into the transaction-display read
    /// model.
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn project_transaction_display_incremental(&self) -> Result<u64, KernelError> {
        Ok(self.worker.project_transaction_display_incremental()?)
    }

    /// All transaction-display rows (read model).
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn transaction_display_rows(&self) -> Result<Vec<TransactionDisplayRow>, KernelError> {
        Ok(self.worker.transaction_display_rows()?)
    }

    /// Number of entries in the operation log.
    ///
    /// # Errors
    /// Returns [`KernelError`] on a persistence failure.
    pub fn operation_count(&self) -> Result<u64, KernelError> {
        Ok(self.worker.operation_count()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use tracing_test::traced_test;
    use uuid::Uuid;

    fn sample_account() -> Account {
        Account::new(
            AccountId::new(),
            LedgerAccountId::new(),
            "Checking",
            CashflowRole::LiquidCash,
            Currency::Usd,
            AccountFlags::default(),
        )
    }

    fn meta() -> CommandMeta {
        CommandMeta {
            command_id: Uuid::now_v7(),
            correlation_id: Uuid::now_v7(),
            causation_id: None,
            actor_type: ActorType::User,
            actor_id: "tester".to_owned(),
            idempotency_key: Uuid::now_v7().to_string(),
        }
    }

    // A unit test (not an integration test) so the emitting crate and the
    // tracing-test capture filter are the same crate; otherwise the library's
    // events are filtered out (plan §2.1 boundary instrumentation).
    #[traced_test]
    #[test]
    fn dispatch_emits_boundary_instrumentation() {
        let dir = TempDir::new().unwrap();
        let kernel = Kernel::open(dir.path().join("vault.db"), "key").unwrap();

        kernel
            .dispatch(CommandEnvelope::new(
                meta(),
                CreateAccount::new(sample_account()),
            ))
            .unwrap();

        assert!(logs_contain("dispatching kernel command"));
        assert!(logs_contain("kernel command applied"));
        assert!(logs_contain("create_account"));
    }

    #[test]
    fn create_account_rejects_a_subtype_from_a_different_role() {
        // A `savings` subtype on a credit-facility account is invalid (ADR 0028):
        // the schema CHECK allows the token, but the kernel enforces the role match.
        let mismatched = Account::new(
            AccountId::new(),
            LedgerAccountId::new(),
            "Card",
            CashflowRole::CreditFacility,
            Currency::Usd,
            AccountFlags::default(),
        )
        .with_subtype(Some(AccountSubtype::Savings));
        assert!(matches!(
            CreateAccount::new(mismatched).validate(),
            Err(KernelError::Validation(_)),
        ));

        // The matching subtype validates, and no subtype is always fine.
        let matching = sample_account().with_subtype(Some(AccountSubtype::Checking));
        assert!(CreateAccount::new(matching).validate().is_ok());
        assert!(CreateAccount::new(sample_account()).validate().is_ok());
    }

    #[test]
    fn cash_tiers_roll_up_through_the_kernel() {
        let dir = TempDir::new().unwrap();
        let kernel = Kernel::open(dir.path().join("vault.db"), "key").unwrap();
        let checking = sample_account().with_subtype(Some(AccountSubtype::Checking));
        kernel
            .dispatch(CommandEnvelope::new(
                meta(),
                CreateAccount::with_opening_balance(checking, Money::new(120_000, Currency::Usd)),
            ))
            .unwrap();
        let tiers = kernel.cash_tiers().unwrap();
        assert_eq!(tiers.spendable, Money::new(120_000, Currency::Usd));
        assert_eq!(tiers.reserve, Money::zero(Currency::Usd));
        assert_eq!(tiers.net, Money::new(120_000, Currency::Usd));
    }
}
