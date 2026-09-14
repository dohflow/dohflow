//! Wire DTOs for the IPC boundary (personal-cfo-40t).
//!
//! These types are the serde + [`specta::Type`] surface the frontend sees. They
//! live here, in the desktop crate, so `specta` never becomes a dependency of
//! the pure domain crates (`core-money`, `core-ledger`) — which would breach the
//! "pure-types crate boundaries" CI check. Each DTO carries an explicit
//! conversion to/from the kernel domain type; UUIDs cross the wire as strings,
//! money as integer minor units.

use chrono::{DateTime, NaiveDate, Utc};
use finance_kernel::{
    Account, AccountAvailability, AccountFlags, AccountId, AccountSeriesView, AccountSubtype,
    AccountView, AssumptionBasis, AssumptionEventView, AssumptionParams, AttachmentId,
    AttachmentMeta, Band, BandDriftView, BatchResult, CapabilityUnlock, CardCycleView,
    CardStatementForecastView, CardStatementHistoryView, CashAvailability, CashFlowHistory,
    CashTiers, CashflowRole, CategoryFilter, CategoryId, CategoryView, ColumnMapping, ComfortBand,
    Currency, DayBalance, DebtTermsInput, DebtTermsView, DriftFactorView, ForecastAssumptionSpec,
    ForecastDayView, ForecastEventView, ForecastReadiness, ForecastView, Frequency,
    GroupSeriesView, ImportedTransactionFields, IncomeSourceId, IncomeSourceView, LedgerAccountId,
    LoanDoubleCount, ManualEntry, Money, MoneyInboxItem, MultiSeriesForecast, Outcome,
    PayoffDebtSeries, PayoffPlanView, ReadinessFactor, RecordTransaction, RecurringBillView,
    RecurringCandidateView, RecurringEventId, RecurringInstanceRow, RecurringTransferId,
    RecurringTransferView, RepaymentPhilosophy, ScenarioView, SourceBatchId, SplitLineInput,
    SplitLineView, StagedTransactionId, TagId, TagView, TransactionId, TransactionPage,
    TransactionPageQuery, TransactionRow, TransactionSortOrder, Transfer, VaultHealth, VaultState,
};
use serde::{Deserialize, Serialize};
use specta::Type;
use specta_typescript::Number;
use uuid::Uuid;

use super::IpcError;

/// Parse a UUID string into a typed [`AccountId`], mapping a bad value to a
/// validation error.
pub fn parse_account_id(raw: &str) -> Result<AccountId, IpcError> {
    uuid::Uuid::parse_str(raw.trim())
        .map(AccountId::from_uuid)
        .map_err(|_| IpcError::Validation(format!("not a valid account id: {raw:?}")))
}

/// Parse a UUID string into a typed [`TransactionId`].
pub fn parse_transaction_id(raw: &str) -> Result<TransactionId, IpcError> {
    uuid::Uuid::parse_str(raw.trim())
        .map(TransactionId::from_uuid)
        .map_err(|_| IpcError::Validation(format!("not a valid transaction id: {raw:?}")))
}

/// Parse a UUID string into a typed [`AttachmentId`].
pub fn parse_attachment_id(raw: &str) -> Result<AttachmentId, IpcError> {
    uuid::Uuid::parse_str(raw.trim())
        .map(AttachmentId::from_uuid)
        .map_err(|_| IpcError::Validation(format!("not a valid attachment id: {raw:?}")))
}

/// Parse a UUID string into a typed [`RecurringEventId`] — a recurring bill's id.
pub fn parse_recurring_event_id(raw: &str) -> Result<RecurringEventId, IpcError> {
    uuid::Uuid::parse_str(raw.trim())
        .map(RecurringEventId::from_uuid)
        .map_err(|_| IpcError::Validation(format!("not a valid recurring bill id: {raw:?}")))
}

/// Parse a UUID string into a typed [`RecurringTransferId`].
pub fn parse_recurring_transfer_id(raw: &str) -> Result<RecurringTransferId, IpcError> {
    uuid::Uuid::parse_str(raw.trim())
        .map(RecurringTransferId::from_uuid)
        .map_err(|_| IpcError::Validation(format!("not a valid recurring transfer id: {raw:?}")))
}

/// Parse a UUID string into a typed [`TagId`].
pub fn parse_tag_id(raw: &str) -> Result<TagId, IpcError> {
    uuid::Uuid::parse_str(raw.trim())
        .map(TagId::from_uuid)
        .map_err(|_| IpcError::Validation(format!("not a valid tag id: {raw:?}")))
}

/// Parse a UUID string into a typed [`SourceBatchId`] — an ingestion batch's id.
pub fn parse_source_batch_id(raw: &str) -> Result<SourceBatchId, IpcError> {
    uuid::Uuid::parse_str(raw.trim())
        .map(SourceBatchId::from_uuid)
        .map_err(|_| IpcError::Validation(format!("not a valid source batch id: {raw:?}")))
}

/// Parse a UUID string into a typed [`StagedTransactionId`] — a staged import
/// row's id, also the Money Inbox item id for the imported-waiting-commit kind.
pub fn parse_staged_transaction_id(raw: &str) -> Result<StagedTransactionId, IpcError> {
    uuid::Uuid::parse_str(raw.trim())
        .map(StagedTransactionId::from_uuid)
        .map_err(|_| IpcError::Validation(format!("not a valid staged transaction id: {raw:?}")))
}

/// Parse a UUID string into a typed [`CategoryId`].
pub fn parse_category_id(raw: &str) -> Result<CategoryId, IpcError> {
    uuid::Uuid::parse_str(raw.trim())
        .map(CategoryId::from_uuid)
        .map_err(|_| IpcError::Validation(format!("not a valid category id: {raw:?}")))
}

/// Parse a UUID string into a typed [`IncomeSourceId`].
pub fn parse_income_source_id(raw: &str) -> Result<IncomeSourceId, IpcError> {
    uuid::Uuid::parse_str(raw.trim())
        .map(IncomeSourceId::from_uuid)
        .map_err(|_| IpcError::Validation(format!("not a valid income source id: {raw:?}")))
}

/// Normalize an optional free-text field: trim, and treat blank as absent.
fn normalize_description(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// A stored attachment's display metadata on the wire (ADR 0023): listing only —
/// no key material and no bytes cross this boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct AttachmentDto {
    /// Attachment id (UUID string).
    pub id: String,
    /// IANA media type, if known at import.
    pub mime_type: Option<String>,
    /// Original filename (kept only inside the encrypted vault).
    pub original_filename: Option<String>,
    /// Plaintext size in bytes (exported as a TS `number`).
    #[specta(type = Number)]
    pub plaintext_size: u64,
    /// RFC 3339 creation instant.
    pub created_at: String,
}

impl From<AttachmentMeta> for AttachmentDto {
    fn from(meta: AttachmentMeta) -> Self {
        Self {
            id: meta.id.to_string(),
            mime_type: meta.mime_type,
            original_filename: meta.original_filename,
            plaintext_size: meta.plaintext_size,
            created_at: meta.created_at,
        }
    }
}

/// One captured source field of an imported transaction (ADR 0045 §2): the source
/// column header and its raw value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct ImportedFieldDto {
    pub key: String,
    pub value: String,
}

/// The raw imported source fields behind a committed transaction
/// (personal-cfo-4d8.24.1.4): every column the importer captured (nothing dropped —
/// ADR 0045 §2), plus which source produced them and when. `null` for a
/// manually-entered transaction (no import provenance).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct ImportedTransactionFieldsDto {
    /// The source format token, e.g. `csv` / `ofx`.
    pub source_type: String,
    /// When the batch was imported (RFC 3339), if recorded.
    pub imported_at: Option<String>,
    /// Every captured source field as key/value, key-sorted.
    pub fields: Vec<ImportedFieldDto>,
}

impl From<ImportedTransactionFields> for ImportedTransactionFieldsDto {
    fn from(f: ImportedTransactionFields) -> Self {
        Self {
            source_type: f.source_type,
            imported_at: f.imported_at,
            fields: f
                .fields
                .into_iter()
                .map(|(key, value)| ImportedFieldDto { key, value })
                .collect(),
        }
    }
}

/// Parse an ISO-4217 alphabetic code into a supported [`Currency`].
pub fn parse_currency(code: &str) -> Result<Currency, IpcError> {
    match code.trim().to_ascii_uppercase().as_str() {
        "USD" => Ok(Currency::Usd),
        "EUR" => Ok(Currency::Eur),
        other => Err(IpcError::Validation(format!(
            "unsupported currency: {other}"
        ))),
    }
}

/// A monetary amount on the wire: integer minor units + ISO currency code.
///
/// `minor_units` is an `i64` cast to a TypeScript `number` via
/// `#[specta(type = Number)]` on the field. A personal vault's balances stay
/// far below 2^53 minor units, so no precision is lost.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct MoneyDto {
    /// Signed amount in the currency's minor units (e.g. cents). Exported to
    /// TypeScript as `number` (lossless for any realistic personal balance).
    #[specta(type = Number)]
    pub minor_units: i64,
    /// ISO-4217 alphabetic code, e.g. `"USD"`.
    pub currency: String,
}

impl MoneyDto {
    /// Convert to a kernel [`Money`], validating the currency code.
    pub fn to_money(&self) -> Result<Money, IpcError> {
        Ok(Money::new(
            self.minor_units,
            parse_currency(&self.currency)?,
        ))
    }
}

impl From<Money> for MoneyDto {
    fn from(money: Money) -> Self {
        Self {
            minor_units: money.minor_units(),
            currency: money.currency().code().to_owned(),
        }
    }
}

/// How an account participates in household cashflow. Mirrors the kernel
/// [`CashflowRole`] so the generated bindings expose a clean string union.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum CashflowRoleDto {
    /// Checking, savings — spendable now.
    LiquidCash,
    /// Credit cards and lines of credit.
    CreditFacility,
    /// Term loans, mortgages.
    LoanLiability,
    /// Brokerage, retirement balances.
    InvestmentAsset,
    /// Property, vehicles.
    RealAsset,
    /// In-transit / clearing / suspense.
    ExternalClearing,
    /// Virtual income/expense categorization account.
    IncomeExpenseVirtual,
}

impl CashflowRoleDto {
    /// Map to the kernel role.
    #[must_use]
    pub fn to_core(self) -> CashflowRole {
        match self {
            CashflowRoleDto::LiquidCash => CashflowRole::LiquidCash,
            CashflowRoleDto::CreditFacility => CashflowRole::CreditFacility,
            CashflowRoleDto::LoanLiability => CashflowRole::LoanLiability,
            CashflowRoleDto::InvestmentAsset => CashflowRole::InvestmentAsset,
            CashflowRoleDto::RealAsset => CashflowRole::RealAsset,
            CashflowRoleDto::ExternalClearing => CashflowRole::ExternalClearing,
            CashflowRoleDto::IncomeExpenseVirtual => CashflowRole::IncomeExpenseVirtual,
        }
    }
}

/// Boolean classification flags for an account. Mirrors [`AccountFlags`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct AccountFlagsDto {
    /// A retirement account.
    pub retirement: bool,
    /// Tax-advantaged (HSA/FSA/529/etc.).
    pub tax_advantaged: bool,
    /// Jointly held.
    pub joint: bool,
    /// A business account.
    pub business: bool,
}

impl AccountFlagsDto {
    /// Map to kernel flags (always created `active`).
    #[must_use]
    pub fn to_core(self) -> AccountFlags {
        AccountFlags {
            retirement: self.retirement,
            tax_advantaged: self.tax_advantaged,
            joint: self.joint,
            business: self.business,
            active: true,
        }
    }
}

/// Input to create an account.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct CreateAccountInput {
    /// Display name (must be non-empty).
    pub name: String,
    /// Cashflow role.
    pub cashflow_role: CashflowRoleDto,
    /// ISO-4217 currency code, e.g. `"USD"`.
    pub currency: String,
    /// Optional classification flags (defaults to all-false, active).
    pub flags: Option<AccountFlagsDto>,
    /// Optional opening balance (currency must match `currency`); recorded as a
    /// balanced equity posting, never a stored column.
    pub opening_balance: Option<MoneyDto>,
    /// Optional subtype storage token (ADR 0028), e.g. `"checking"`. Must belong to
    /// `cashflow_role` (validated by the kernel).
    pub subtype: Option<String>,
    /// Idempotency key for safe retries. If empty, the server generates one.
    pub idempotency_key: String,
}

impl CreateAccountInput {
    /// Build the kernel [`Account`] with freshly generated identities.
    pub fn to_account(&self) -> Result<Account, IpcError> {
        if self.name.trim().is_empty() {
            return Err(IpcError::Validation(
                "account name must not be empty".to_owned(),
            ));
        }
        let currency = parse_currency(&self.currency)?;
        let flags = self.flags.map(AccountFlagsDto::to_core).unwrap_or_default();
        let subtype = parse_subtype(self.subtype.as_deref())?;
        Ok(Account::new(
            AccountId::new(),
            LedgerAccountId::new(),
            self.name.clone(),
            self.cashflow_role.to_core(),
            currency,
            flags,
        )
        .with_subtype(subtype))
    }
}

/// Parse an optional subtype storage token into an [`AccountSubtype`] (ADR 0028).
/// `None`/absent stays `None`; an unrecognized token is a validation error. The
/// role↔subtype match is enforced by the kernel, which knows the account's role.
pub fn parse_subtype(token: Option<&str>) -> Result<Option<AccountSubtype>, IpcError> {
    match token.map(str::trim).filter(|t| !t.is_empty()) {
        None => Ok(None),
        Some(t) => AccountSubtype::from_token(t)
            .map(Some)
            .ok_or_else(|| IpcError::Validation(format!("not a valid account subtype: {t:?}"))),
    }
}

/// Input to rename an account.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct UpdateAccountInput {
    /// The account id (UUID string).
    pub account_id: String,
    /// The new display name.
    pub name: String,
    /// Idempotency key for safe retries. If empty, the server generates one.
    pub idempotency_key: String,
}

/// How a liability is repaid (ADR 0035 §1), on the wire as a snake_case token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum RepaymentPhilosophyDto {
    /// Pay the full balance.
    PayInFull,
    /// Pay the projected statement balance.
    PayStatementBalance,
    /// Pay the current owed balance.
    PayCurrentBalance,
    /// Pay the computed minimum.
    PayMinimum,
    /// Pay a stored fixed amount.
    PayFixedAmount,
    /// Not set — the forecast assumes the minimum.
    Unknown,
}

impl RepaymentPhilosophyDto {
    fn to_core(self) -> RepaymentPhilosophy {
        match self {
            Self::PayInFull => RepaymentPhilosophy::PayInFull,
            Self::PayStatementBalance => RepaymentPhilosophy::PayStatementBalance,
            Self::PayCurrentBalance => RepaymentPhilosophy::PayCurrentBalance,
            Self::PayMinimum => RepaymentPhilosophy::PayMinimum,
            Self::PayFixedAmount => RepaymentPhilosophy::PayFixedAmount,
            Self::Unknown => RepaymentPhilosophy::Unknown,
        }
    }

    fn from_core(p: RepaymentPhilosophy) -> Self {
        match p {
            RepaymentPhilosophy::PayInFull => Self::PayInFull,
            RepaymentPhilosophy::PayStatementBalance => Self::PayStatementBalance,
            RepaymentPhilosophy::PayCurrentBalance => Self::PayCurrentBalance,
            RepaymentPhilosophy::PayMinimum => Self::PayMinimum,
            RepaymentPhilosophy::PayFixedAmount => Self::PayFixedAmount,
            RepaymentPhilosophy::Unknown => Self::Unknown,
        }
    }
}

/// Input to set a liability account's debt terms (ADR 0035 §5, personal-cfo-6wk.2). An
/// upsert — absent (`null`) fields are stored as unset. Money is integer minor units; rates
/// and percentages are basis points.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SetDebtTermsInput {
    /// The liability account id (UUID string).
    pub account_id: String,
    /// APR in basis points.
    #[specta(type = Option<Number>)]
    pub apr_bps: Option<i64>,
    /// Statement close day-of-month (1–31).
    #[specta(type = Option<Number>)]
    pub statement_close_day: Option<i64>,
    /// Payment due day-of-month (1–31).
    #[specta(type = Option<Number>)]
    pub payment_due_day: Option<i64>,
    /// Grace-period length in days.
    #[specta(type = Option<Number>)]
    pub grace_period_days: Option<i64>,
    /// Credit limit in minor units.
    #[specta(type = Option<Number>)]
    pub credit_limit_minor: Option<i64>,
    /// How the liability is repaid.
    pub repayment_philosophy: RepaymentPhilosophyDto,
    /// Fixed payment for `pay_fixed_amount`, in minor units.
    #[specta(type = Option<Number>)]
    pub fixed_amount_minor: Option<i64>,
    /// Minimum-payment percent-of-balance, in basis points.
    #[specta(type = Option<Number>)]
    pub min_payment_percent_bps: Option<i64>,
    /// Minimum-payment floor, in minor units.
    #[specta(type = Option<Number>)]
    pub min_payment_floor_minor: Option<i64>,
    /// The liquid account that pays this debt (UUID string), or `null`.
    pub paying_source_account_id: Option<String>,
    /// The loan's original principal in minor units (ADR 0044), or `null`.
    #[specta(type = Option<Number>)]
    pub original_principal_minor: Option<i64>,
    /// Idempotency key for safe retries. If empty, the server generates one.
    pub idempotency_key: String,
}

impl SetDebtTermsInput {
    /// Lower to the kernel input, parsing the paying-source id.
    pub fn to_core(&self) -> Result<DebtTermsInput, IpcError> {
        let paying_source_account_id = match self.paying_source_account_id.as_deref() {
            Some(s) if !s.trim().is_empty() => Some(parse_account_id(s)?),
            _ => None,
        };
        Ok(DebtTermsInput {
            apr_bps: self.apr_bps,
            statement_close_day: self.statement_close_day,
            payment_due_day: self.payment_due_day,
            grace_period_days: self.grace_period_days,
            credit_limit_minor: self.credit_limit_minor,
            repayment_philosophy: self.repayment_philosophy.to_core(),
            fixed_amount_minor: self.fixed_amount_minor,
            min_payment_percent_bps: self.min_payment_percent_bps,
            min_payment_floor_minor: self.min_payment_floor_minor,
            paying_source_account_id,
            original_principal_minor: self.original_principal_minor,
        })
    }
}

/// A liability account's debt terms for display.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct DebtTermsDto {
    /// The liability account id (UUID string).
    pub account_id: String,
    /// APR in basis points.
    #[specta(type = Option<Number>)]
    pub apr_bps: Option<i64>,
    /// Statement close day-of-month.
    #[specta(type = Option<Number>)]
    pub statement_close_day: Option<i64>,
    /// Payment due day-of-month.
    #[specta(type = Option<Number>)]
    pub payment_due_day: Option<i64>,
    /// Grace-period days.
    #[specta(type = Option<Number>)]
    pub grace_period_days: Option<i64>,
    /// Credit limit in minor units.
    #[specta(type = Option<Number>)]
    pub credit_limit_minor: Option<i64>,
    /// How the liability is repaid.
    pub repayment_philosophy: RepaymentPhilosophyDto,
    /// Fixed payment amount in minor units.
    #[specta(type = Option<Number>)]
    pub fixed_amount_minor: Option<i64>,
    /// Minimum-payment percent in basis points.
    #[specta(type = Option<Number>)]
    pub min_payment_percent_bps: Option<i64>,
    /// Minimum-payment floor in minor units.
    #[specta(type = Option<Number>)]
    pub min_payment_floor_minor: Option<i64>,
    /// The paying liquid account (UUID string), or `null`.
    pub paying_source_account_id: Option<String>,
    /// The loan's original principal in minor units (ADR 0044), or `null`.
    #[specta(type = Option<Number>)]
    pub original_principal_minor: Option<i64>,
}

impl From<DebtTermsView> for DebtTermsDto {
    fn from(v: DebtTermsView) -> Self {
        Self {
            account_id: v.account_id.as_uuid().to_string(),
            apr_bps: v.apr_bps,
            statement_close_day: v.statement_close_day,
            payment_due_day: v.payment_due_day,
            grace_period_days: v.grace_period_days,
            credit_limit_minor: v.credit_limit_minor,
            repayment_philosophy: RepaymentPhilosophyDto::from_core(v.repayment_philosophy),
            fixed_amount_minor: v.fixed_amount_minor,
            min_payment_percent_bps: v.min_payment_percent_bps,
            min_payment_floor_minor: v.min_payment_floor_minor,
            paying_source_account_id: v.paying_source_account_id.map(|a| a.as_uuid().to_string()),
            original_principal_minor: v.original_principal_minor,
        }
    }
}

/// A read-model view of an account for display.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct AccountViewDto {
    /// The account id (UUID string).
    pub id: String,
    /// Display name.
    pub name: String,
    /// Cashflow-role storage token (e.g. `"liquid_cash"`).
    pub cashflow_role: String,
    /// Optional subtype storage token (ADR 0028), e.g. `"checking"`; `null` when
    /// unspecified.
    pub subtype: Option<String>,
    /// Whether the account is active (not archived).
    pub active: bool,
    /// Balance, summed from the account's ledger postings.
    pub balance: MoneyDto,
    /// Free-text note (ADR 0044); `null` when unset.
    pub notes: Option<String>,
    /// The financing liability this real asset points at (UUID string, ADR 0044 §5), or
    /// `null`. Only ever set on a real-asset account; used to prefill the editor's Link.
    pub linked_account_id: Option<String>,
    /// The display name of this account's link partner (ADR 0044 §5) — the liability a
    /// real asset points at, or the asset that points at a liability. `null` when unlinked.
    pub linked_account_name: Option<String>,
}

impl From<AccountView> for AccountViewDto {
    fn from(view: AccountView) -> Self {
        Self {
            id: view.id.to_string(),
            name: view.name,
            cashflow_role: view.cashflow_role,
            subtype: view.subtype,
            active: view.active,
            balance: view.balance.into(),
            notes: view.notes,
            linked_account_id: view.linked_account_id.map(|a| a.as_uuid().to_string()),
            linked_account_name: view.linked_account_name,
        }
    }
}

/// The type-based cash-tier rollups for display (ADR 0028, personal-cfo-9dgg).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct CashTiersDto {
    /// Spendable now: checking + cash + unclassified liquid.
    pub spendable: MoneyDto,
    /// Set aside: savings + money market.
    pub reserve: MoneyDto,
    /// Net cash = spendable + reserve = every liquid account.
    pub net: MoneyDto,
}

impl From<CashTiers> for CashTiersDto {
    fn from(tiers: CashTiers) -> Self {
        Self {
            spendable: tiers.spendable.into(),
            reserve: tiers.reserve.into(),
            net: tiers.net.into(),
        }
    }
}

/// The five cash numbers for one liquid account on the wire (ADR 0029,
/// personal-cfo-fqbm).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct AccountAvailabilityDto {
    /// The liquid account id (UUID string).
    pub account_id: String,
    /// Display name.
    pub name: String,
    /// Canonical (assertion-anchored) balance.
    pub ledger: MoneyDto,
    /// Uncleared holds (0 in manual mode).
    pub pending: MoneyDto,
    /// `ledger − pending`.
    pub available: MoneyDto,
    /// Projected outflows over the next 30 days attributed to the account.
    pub committed: MoneyDto,
    /// `available − committed`.
    pub headroom: MoneyDto,
}

impl From<AccountAvailability> for AccountAvailabilityDto {
    fn from(a: AccountAvailability) -> Self {
        Self {
            account_id: a.account_id.to_string(),
            name: a.name,
            ledger: a.ledger.into(),
            pending: a.pending.into(),
            available: a.available.into(),
            committed: a.committed.into(),
            headroom: a.headroom.into(),
        }
    }
}

/// The household cash-availability snapshot on the wire (ADR 0029): per-account
/// numbers + the net rollup + the minimum-cash-floor status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct CashAvailabilityDto {
    /// ISO-4217 currency code.
    pub currency: String,
    /// One entry per liquid account.
    pub accounts: Vec<AccountAvailabilityDto>,
    /// `Σ available`.
    pub net_available: MoneyDto,
    /// `Σ committed` (incl. un-attributable outflows).
    pub net_committed: MoneyDto,
    /// `net_available − net_committed` — the household safe-to-spend.
    pub net_headroom: MoneyDto,
    /// The household minimum-cash-floor setting.
    pub floor: MoneyDto,
    /// Whether net headroom is below the floor (the forward-looking alert).
    pub below_floor: bool,
}

impl From<CashAvailability> for CashAvailabilityDto {
    fn from(a: CashAvailability) -> Self {
        Self {
            currency: a.currency.code().to_owned(),
            accounts: a
                .accounts
                .into_iter()
                .map(AccountAvailabilityDto::from)
                .collect(),
            net_available: a.net_available.into(),
            net_committed: a.net_committed.into(),
            net_headroom: a.net_headroom.into(),
            floor: a.floor.into(),
            below_floor: a.below_floor,
        }
    }
}

/// The household liquid-cash comfort band on the wire (ADR 0018 addendum 915.1,
/// personal-cfo-3v6d): the lower edge (minimum-cash floor) + an optional upper edge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct ComfortBandDto {
    /// ISO-4217 currency code.
    pub currency: String,
    /// The lower edge — the minimum-cash floor.
    pub lower: MoneyDto,
    /// The upper edge, if the user has set one.
    pub upper: Option<MoneyDto>,
}

impl From<ComfortBand> for ComfortBandDto {
    fn from(b: ComfortBand) -> Self {
        Self {
            currency: b.currency.code().to_owned(),
            lower: b.lower.into(),
            upper: b.upper.map(MoneyDto::from),
        }
    }
}

/// A rising-spend category contributing to a band drift, on the wire (personal-cfo-5ie.8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct DriftFactorDto {
    pub category_id: String,
    pub category_name: String,
    /// The category's recent monthly spend.
    pub recent_monthly: MoneyDto,
    /// How much that is up versus the preceding window (positive).
    pub delta: MoneyDto,
}

/// The descriptive comfort-band drift signal on the wire (personal-cfo-5ie.8, ADR 0018 §915.1):
/// the crossing + the rising-spend categories that attribute it. Facts only; the descriptive copy
/// (and the non-advice boundary) is applied in the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct BandDriftSignalDto {
    /// The first far-horizon day the projection closes below the lower edge, `YYYY-MM-DD`.
    pub crossing_date: String,
    /// How far below the edge the projection reaches at its worst.
    pub magnitude: MoneyDto,
    /// The rising-spend categories, largest rise first.
    pub factors: Vec<DriftFactorDto>,
}

impl From<BandDriftView> for BandDriftSignalDto {
    fn from(v: BandDriftView) -> Self {
        Self {
            crossing_date: v.crossing_date.to_string(),
            magnitude: v.magnitude.into(),
            factors: v.factors.into_iter().map(DriftFactorDto::from).collect(),
        }
    }
}

impl From<DriftFactorView> for DriftFactorDto {
    fn from(f: DriftFactorView) -> Self {
        Self {
            category_id: f.category_id,
            category_name: f.category_name,
            recent_monthly: f.recent_monthly.into(),
            delta: f.delta.into(),
        }
    }
}

/// One Forecast Readiness factor on the wire (ADR 0026 §13, personal-cfo-6vj9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct ReadinessFactorDto {
    /// Stable key: `coverage` | `freshness` | `explained`.
    pub key: String,
    /// Display label.
    pub label: String,
    /// This factor's score, 0–100.
    pub score: u8,
    /// One-line explanation / the action that improves it.
    pub detail: String,
}

impl From<ReadinessFactor> for ReadinessFactorDto {
    fn from(f: ReadinessFactor) -> Self {
        Self {
            key: f.key,
            label: f.label,
            score: f.score,
            detail: f.detail,
        }
    }
}

/// The R1 Forecast Readiness score on the wire (ADR 0026 §13, personal-cfo-6vj9):
/// a 0–100 data-maturity indicator with a per-factor breakdown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct ForecastReadinessDto {
    /// Overall score, 0–100.
    pub score: u8,
    /// Per-factor breakdown (coverage, freshness, explained).
    pub factors: Vec<ReadinessFactorDto>,
}

impl From<ForecastReadiness> for ForecastReadinessDto {
    fn from(r: ForecastReadiness) -> Self {
        Self {
            score: r.score,
            factors: r
                .factors
                .into_iter()
                .map(ReadinessFactorDto::from)
                .collect(),
        }
    }
}

/// A self-activated forecast capability whose one-time unlock notice the user has not yet
/// acknowledged (ADR 0026 §10, personal-cfo-egon).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct CapabilityUnlockDto {
    /// Stable capability key (e.g. `forecast_band`) — pass back to `acknowledge_capability`.
    pub key: String,
    /// Headline shown to the user.
    pub title: String,
    /// One-line explanation of what unlocked and why.
    pub body: String,
    /// The readiness factor whose threshold unlocked this capability.
    pub factor_key: String,
}

impl From<CapabilityUnlock> for CapabilityUnlockDto {
    fn from(c: CapabilityUnlock) -> Self {
        Self {
            key: c.key,
            title: c.title,
            body: c.body,
            factor_key: c.factor_key,
        }
    }
}

/// A recent transaction for the transactions list (personal-cfo-idsd). The
/// signed `amount` is from the account's perspective (positive in, negative out).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct TransactionRowDto {
    /// The ledger transaction id (UUID string).
    pub transaction_id: String,
    /// The user account id (UUID string).
    pub account_id: String,
    /// The account's display name.
    pub account_name: String,
    /// The far side of a transfer (personal-cfo-4d8.27.8.1) — the other user account this
    /// transaction moved money against. Null for an ordinary income/expense, which has no
    /// second user account. Read with `amount`'s sign for direction: negative means the
    /// money left `account_name` for this one.
    pub counter_account_id: Option<String>,
    pub counter_account_name: Option<String>,
    /// When the transaction occurred — the POSTED date (primary, RFC 3339, ADR 0045).
    pub occurred_at: String,
    /// The secondary transaction / authorization date an import carried (`YYYY-MM-DD`),
    /// when distinct from the posted date; null otherwise (ADR 0045, personal-cfo-4d8.24.1).
    pub transaction_date: Option<String>,
    /// The account's balance immediately after this transaction, over the FULL ledger;
    /// `null` unless the caller asked for it (personal-cfo-ttuy).
    #[specta(type = Option<Number>)]
    pub balance_after_minor: Option<i64>,
    /// Signed amount from the account's perspective.
    pub amount: MoneyDto,
    /// Free-text detail (personal-cfo-byxe): an import's description or a user memo.
    pub memo: Option<String>,
    /// The merchant / payee, when known.
    pub counterparty: Option<String>,
    /// The assigned category id (UUID string), or null if uncategorized (ADR 0030,
    /// personal-cfo-bac). The frontend resolves the name from the taxonomy.
    pub category_id: Option<String>,
    /// Whether the user has reviewed this transaction (ADR 0032, personal-cfo-4d8.7).
    /// Imported transactions default unreviewed; manual ones default reviewed.
    pub reviewed: bool,
    /// The user's free-text note (ADR 0033, personal-cfo-hmt), or null.
    pub note: Option<String>,
    /// The assigned tag ids (UUID strings); empty when untagged (ADR 0033).
    pub tag_ids: Vec<String>,
    /// Number of split lines (ADR 0034); 0 when not split. Drives the expand affordance.
    #[specta(type = Number)]
    pub split_count: i64,
    /// How the category was assigned (ADR 0030 merchant-memory addendum, personal-cfo-5n4.1):
    /// `user` / `rule` / `model` / `import_alias`, or null when uncategorized. Drives the row's
    /// provenance badge so auto-categorized ("rule") tags are visible and trustable.
    pub category_source: Option<String>,
    /// The categorizer's confidence in basis points (0..=10000), or null when uncategorized.
    #[specta(type = Option<Number>)]
    pub category_confidence_bps: Option<i64>,
}

impl From<TransactionRow> for TransactionRowDto {
    fn from(row: TransactionRow) -> Self {
        Self {
            transaction_id: row.transaction_id.to_string(),
            account_id: row.account_id.to_string(),
            account_name: row.account_name,
            counter_account_id: row.counter_account_id.map(|id| id.to_string()),
            counter_account_name: row.counter_account_name,
            occurred_at: row.occurred_at.to_rfc3339(),
            transaction_date: row.transaction_date.map(|d| d.to_string()),
            balance_after_minor: row.balance_after_minor,
            amount: row.amount.into(),
            memo: row.memo,
            counterparty: row.counterparty,
            category_id: row.category_id.map(|id| id.to_string()),
            reviewed: row.reviewed,
            note: row.note,
            tag_ids: row.tag_ids.iter().map(ToString::to_string).collect(),
            split_count: row.split_count,
            category_source: row.category_source,
            category_confidence_bps: row.category_confidence_bps,
        }
    }
}

/// The category-filter sentinel meaning "only uncategorized rows" in a
/// [`TransactionPageInput`] (personal-cfo-3fdd.1).
const UNCATEGORIZED_SENTINEL: &str = "uncategorized";

/// Input for the paged transaction read (personal-cfo-3fdd.1): the filter bar's
/// constraints plus sort + window. Null / empty fields mean "no constraint".
#[derive(Debug, Clone, Deserialize, Type)]
pub struct TransactionPageInput {
    /// Case-insensitive substring over memo / counterparty / note / account name.
    pub query: Option<String>,
    /// Restrict to these accounts (UUID strings); **empty means no constraint**. A set,
    /// not a single id, because the Debt page scopes its embedded list to a
    /// multi-account selection (ADR 0057 §3) — the filter bar's single-account facet is
    /// the one-element case of the same field, not a second mechanism.
    pub account_ids: Vec<String>,
    /// Ask for each row's `balance_after_minor` (personal-cfo-ttuy). Off by default: it
    /// costs an extra bounded read, and a running balance is only meaningful in DATE order.
    #[serde(default)]
    pub with_balances: bool,
    /// A category UUID, or the sentinel `"uncategorized"` for rows without one.
    pub category_id: Option<String>,
    /// Only transactions carrying this tag (UUID string).
    pub tag_id: Option<String>,
    /// Only transactions confirmed as paying this recurring bill (UUID string) —
    /// a bill's linked-payment history (personal-cfo-4d8.24.7.1).
    pub recurring_event_id: Option<String>,
    /// Inclusive `YYYY-MM-DD` lower bound on the occurred-at day.
    pub from_date: Option<String>,
    /// Inclusive `YYYY-MM-DD` upper bound on the occurred-at day.
    pub to_date: Option<String>,
    /// Only unreviewed transactions.
    pub unreviewed_only: bool,
    /// `newest` (the default order) / `oldest` / `amount_desc` / `amount_asc`.
    pub sort: String,
    /// Page size (rows per fetch).
    pub limit: u32,
    /// Row offset of the window.
    pub offset: u32,
}

impl TransactionPageInput {
    /// Convert to the kernel [`TransactionPageQuery`], parsing ids, dates, and the
    /// sort token. Rust stays authoritative for validation (ADR 0003).
    pub fn into_query(self) -> Result<TransactionPageQuery, IpcError> {
        // Treat empty / whitespace-only strings like null, so the frontend's
        // ""-means-unset filter state round-trips harmlessly.
        let non_empty = |raw: Option<String>| -> Option<String> {
            raw.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty())
        };
        let parse_day = |raw: &str| -> Result<NaiveDate, IpcError> {
            NaiveDate::parse_from_str(raw, "%Y-%m-%d")
                .map_err(|_| IpcError::Validation(format!("not a valid date: {raw:?}")))
        };
        let category = match non_empty(self.category_id).as_deref() {
            None => None,
            Some(UNCATEGORIZED_SENTINEL) => Some(CategoryFilter::Uncategorized),
            Some(raw) => Some(CategoryFilter::Category(parse_category_id(raw)?)),
        };
        let sort = match self.sort.trim() {
            "" | "newest" => TransactionSortOrder::NewestFirst,
            "oldest" => TransactionSortOrder::OldestFirst,
            "amount_desc" => TransactionSortOrder::AmountDesc,
            "amount_asc" => TransactionSortOrder::AmountAsc,
            other => {
                return Err(IpcError::Validation(format!(
                    "not a valid sort order: {other:?}"
                )))
            }
        };
        Ok(TransactionPageQuery {
            query: non_empty(self.query),
            // Blank/whitespace entries are dropped rather than rejected, matching how
            // every other facet treats the frontend's ""-means-unset state.
            with_balances: self.with_balances,
            account_ids: self
                .account_ids
                .iter()
                .map(|raw| raw.trim())
                .filter(|raw| !raw.is_empty())
                .map(parse_account_id)
                .collect::<Result<Vec<_>, _>>()?,
            category,
            tag_id: non_empty(self.tag_id)
                .as_deref()
                .map(parse_tag_id)
                .transpose()?,
            recurring_event_id: non_empty(self.recurring_event_id)
                .as_deref()
                .map(parse_recurring_event_id)
                .transpose()?,
            from: non_empty(self.from_date)
                .as_deref()
                .map(parse_day)
                .transpose()?,
            to: non_empty(self.to_date)
                .as_deref()
                .map(parse_day)
                .transpose()?,
            unreviewed_only: self.unreviewed_only,
            sort,
            limit: self.limit,
            offset: self.offset,
        })
    }
}

/// One page of the filtered transaction list plus the total match count
/// (personal-cfo-3fdd.1), so the UI renders real page controls over all history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct TransactionPageDto {
    /// The requested window of rows.
    pub rows: Vec<TransactionRowDto>,
    /// Total rows matching the filter, across all pages.
    pub total: u32,
}

impl From<TransactionPage> for TransactionPageDto {
    fn from(page: TransactionPage) -> Self {
        Self {
            rows: page.rows.into_iter().map(TransactionRowDto::from).collect(),
            total: page.total,
        }
    }
}

/// A split line for display (ADR 0034, personal-cfo-kr9).
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct SplitLineDto {
    /// The split line id (UUID string).
    pub id: String,
    /// The slice amount.
    pub amount: MoneyDto,
    /// The line's category id (UUID string), or null.
    pub category_id: Option<String>,
    /// The line's note, or null.
    pub note: Option<String>,
    /// The line's tag ids (UUID strings).
    pub tag_ids: Vec<String>,
    /// Display order within the transaction.
    #[specta(type = Number)]
    pub sort_order: i64,
}

impl From<SplitLineView> for SplitLineDto {
    fn from(line: SplitLineView) -> Self {
        Self {
            id: line.id.to_string(),
            amount: line.amount.into(),
            category_id: line.category_id.map(|id| id.to_string()),
            note: line.note,
            tag_ids: line.tag_ids.iter().map(ToString::to_string).collect(),
            sort_order: line.sort_order,
        }
    }
}

/// One line of a split, as supplied by the UI to `set_splits` (ADR 0034).
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct SplitLineInputDto {
    /// The slice amount (same sign + currency as the transaction).
    pub amount: MoneyDto,
    /// The line's category id (UUID string), or null.
    pub category_id: Option<String>,
    /// The line's note, or null.
    pub note: Option<String>,
    /// The line's tag ids (UUID strings).
    pub tag_ids: Vec<String>,
}

impl SplitLineInputDto {
    /// Convert to the kernel [`SplitLineInput`], parsing the ids + amount.
    pub fn into_input(self) -> Result<SplitLineInput, IpcError> {
        Ok(SplitLineInput {
            amount: self.amount.to_money()?,
            category_id: self
                .category_id
                .as_deref()
                .map(parse_category_id)
                .transpose()?,
            note: self.note,
            tag_ids: self
                .tag_ids
                .iter()
                .map(|t| parse_tag_id(t))
                .collect::<Result<Vec<_>, _>>()?,
        })
    }
}

/// A tag for display (ADR 0033, personal-cfo-2ryf).
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct TagViewDto {
    /// The tag id (UUID string).
    pub id: String,
    /// Display name.
    pub name: String,
    /// Optional display color.
    pub color: Option<String>,
    /// Whether the tag is archived (soft-deleted).
    pub archived: bool,
}

impl From<TagView> for TagViewDto {
    fn from(tag: TagView) -> Self {
        Self {
            id: tag.id.to_string(),
            name: tag.name,
            color: tag.color,
            archived: tag.archived,
        }
    }
}

/// Result of creating a tag (ADR 0033): the new tag's id + the mutation outcome.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct CreateTagResult {
    /// The created tag's id (UUID string).
    pub tag_id: String,
    /// The op-log mutation outcome.
    pub mutation: MutationResult,
}

/// A category in the household taxonomy (plan §9.6, ADR 0030, personal-cfo-bac).
/// The frontend builds the tree from `parent_id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct CategoryDto {
    /// Category id (UUID string).
    pub id: String,
    /// The parent category id (UUID string), or null for a top-level group.
    pub parent_id: Option<String>,
    pub name: String,
    /// `income` / `expense` / `transfer` / `adjustment`.
    pub category_type: String,
    pub icon: Option<String>,
    pub color: Option<String>,
    /// A seeded default — its identity (name/parent) is fixed, but its appearance
    /// (color + icon) is user-editable, and it can be archived (ADR 0030 amendment, kogu).
    pub is_system: bool,
    /// `deterministic` / `variable_regular` / `variable_lumpy` / `ignore_cashflow`
    /// / `income`.
    pub forecast_behavior: String,
    /// Whether the category is archived (hidden from pickers; ADR 0030 soft-delete).
    pub archived: bool,
}

impl From<CategoryView> for CategoryDto {
    fn from(view: CategoryView) -> Self {
        Self {
            id: view.id.to_string(),
            parent_id: view.parent_id.map(|id| id.to_string()),
            name: view.name,
            category_type: view.category_type,
            icon: view.icon,
            color: view.color,
            is_system: view.is_system,
            forecast_behavior: view.forecast_behavior,
            archived: view.archived,
        }
    }
}

/// Input to create a user category (ADR 0030, personal-cfo-bac).
#[derive(Debug, Clone, Deserialize, Type)]
pub struct CreateCategoryInput {
    /// Parent category id (UUID string), or null for a top-level group.
    pub parent_id: Option<String>,
    pub name: String,
    /// `income` / `expense` / `transfer` / `adjustment`.
    pub category_type: String,
    pub color: Option<String>,
    /// Optional display icon (an emoji) set at creation (personal-cfo-kogu); null for none.
    pub icon: Option<String>,
    /// Idempotency key; empty → generated server-side.
    pub idempotency_key: String,
}

/// Result of creating a category: the new id plus the mutation outcome.
#[derive(Debug, Clone, Serialize, Type)]
pub struct CreateCategoryResult {
    /// The new category id (UUID string).
    pub category_id: String,
    pub mutation: MutationResult,
}

/// Input to edit a category (ADR 0030, personal-cfo-bac). A user category updates
/// name + color + icon; for a system ("Default") category only the appearance
/// (color + icon) is applied and the submitted name is ignored (its identity is
/// preserved — ADR 0030 amendment, personal-cfo-kogu).
#[derive(Debug, Clone, Deserialize, Type)]
pub struct UpdateCategoryInput {
    /// The category to edit (UUID string).
    pub id: String,
    /// New display name.
    pub name: String,
    /// New display color, or null to clear it.
    pub color: Option<String>,
    /// New display icon (an emoji), or null to clear it (personal-cfo-4d8.24.10).
    pub icon: Option<String>,
    /// Idempotency key; empty → generated server-side.
    pub idempotency_key: String,
}

/// Input to re-parent a user category (ADR 0030, personal-cfo-bac). `new_parent_id`
/// null makes it a top-level group; cycles are rejected.
#[derive(Debug, Clone, Deserialize, Type)]
pub struct MoveCategoryInput {
    /// The category to move (UUID string).
    pub id: String,
    /// New parent category id (UUID string), or null for a top-level group.
    pub new_parent_id: Option<String>,
    /// Idempotency key; empty → generated server-side.
    pub idempotency_key: String,
}

/// A Money Inbox triage item for the frontend (ADR 0014 §7, personal-cfo-dsq).
/// The read model is generic across all item kinds; `payload_json` is a JSON
/// object string holding the kind-specific display detail, which the frontend
/// parses per `item_kind` (for `imported_waiting_commit`: merchant, amount,
/// posted_at, account, source filename, dedupe reason, suspected counterpart).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct MoneyInboxItemDto {
    /// Stable item id (UUID string).
    pub item_id: String,
    /// Item-kind token (e.g. `imported_waiting_commit`).
    pub item_kind: String,
    /// The canonical table this item points at (e.g. `staged_transactions`).
    pub target_table: String,
    /// The canonical row id this item points at (UUID string).
    pub target_id: String,
    /// Sort weight; lower surfaces first. Exported as TS `number`.
    #[specta(type = Number)]
    pub priority: i64,
    /// When the item first appeared (RFC 3339 / ISO timestamp).
    pub surfaced_at: String,
    /// Hidden-until timestamp set by a snooze action (null until snoozed).
    pub snoozed_until: Option<String>,
    /// Set when the user dismisses the item (null while active).
    pub dismissed_at: Option<String>,
    /// Set when the item is resolved (null while active).
    pub resolved_at: Option<String>,
    /// Kind-specific display detail as a JSON object string.
    pub payload_json: String,
}

impl From<MoneyInboxItem> for MoneyInboxItemDto {
    fn from(item: MoneyInboxItem) -> Self {
        Self {
            item_id: item.item_id.to_string(),
            item_kind: item.item_kind,
            target_table: item.target_table,
            target_id: item.target_id.to_string(),
            priority: item.priority,
            surfaced_at: item.surfaced_at,
            snoozed_until: item.snoozed_until,
            dismissed_at: item.dismissed_at,
            resolved_at: item.resolved_at,
            payload_json: item.payload_json,
        }
    }
}

/// Result of a successful mutating command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct MutationResult {
    /// The op-log sequence number of the application. Exported as TS `number`.
    #[specta(type = Number)]
    pub op_seq: i64,
    /// Whether this was a replay of an already-applied idempotency key (no new
    /// rows were written).
    pub replayed: bool,
}

/// Result of creating a recurring bill: the mutation outcome plus the new bill's id, so
/// the caller can immediately fetch its retro-attached history (ADR 0047 §1,
/// personal-cfo-4d8.25.8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct CreateRecurringBillResult {
    /// The op-log sequence number of the application. Exported as TS `number`.
    #[specta(type = Number)]
    pub op_seq: i64,
    /// Whether this was a replay of an already-applied idempotency key.
    pub replayed: bool,
    /// The created recurring event id (UUID string).
    pub event_id: String,
}

impl From<Outcome> for MutationResult {
    fn from(outcome: Outcome) -> Self {
        match outcome {
            Outcome::Applied { op_seq } => Self {
                op_seq,
                replayed: false,
            },
            Outcome::Replayed { op_seq } => Self {
                op_seq,
                replayed: true,
            },
        }
    }
}

/// Result of creating an account: the new id plus the mutation outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct CreateAccountResult {
    /// The new account's id (UUID string).
    pub account_id: String,
    /// The mutation outcome (op-log sequence + replay flag).
    pub mutation: MutationResult,
}

/// Input for opening an ingestion source batch (personal-cfo-3bb). The batch id
/// is minted server-side and returned in [`CreateSourceBatchResult`].
#[derive(Debug, Clone, Deserialize, Type)]
pub struct CreateSourceBatchInput {
    /// Source-type token (`csv`/`ofx`/`manual`/…).
    pub source_type: String,
    /// Display name (e.g. the filename), if any.
    pub source_name: Option<String>,
    /// Whole-file content fingerprint for file-level dedupe (ADR 0014), if any.
    pub file_fingerprint: Option<String>,
    /// Parser version, if known at open time.
    pub parser_version: Option<String>,
    /// Idempotency key; empty → generated server-side.
    pub idempotency_key: String,
}

/// Result of opening a source batch: the new id plus the mutation outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct CreateSourceBatchResult {
    /// The new batch's id (UUID string).
    pub source_batch_id: String,
    /// The mutation outcome (op-log sequence + replay flag).
    pub mutation: MutationResult,
}

/// Input for attaching a parsed source record to a batch (personal-cfo-3bb).
#[derive(Debug, Clone, Deserialize, Type)]
pub struct AttachSourceRecordInput {
    /// The batch (UUID string) to attach the record under.
    pub source_batch_id: String,
    /// Provider/external id, when the source carries one.
    pub external_id: Option<String>,
    /// Content fingerprint of the record (the dedupe key).
    pub source_hash: String,
    /// The extracted, normalized fields as JSON — never the raw bytes (ADR 0014 §4).
    pub normalized_json: String,
    /// Parser confidence in basis points (0..=10000), if scored.
    #[specta(type = Option<Number>)]
    pub parse_confidence_bps: Option<i64>,
    /// Idempotency key; empty → generated server-side.
    pub idempotency_key: String,
}

/// Result of attaching a source record: the record id plus the mutation outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct AttachSourceRecordResult {
    /// The attached record's id (UUID string). On a content-duplicate the existing
    /// record is reused — this is the id this attach minted.
    pub source_record_id: String,
    /// The mutation outcome (op-log sequence + replay flag).
    pub mutation: MutationResult,
}

/// Input for advancing a source batch's lifecycle status + counts (personal-cfo-3bb).
#[derive(Debug, Clone, Deserialize, Type)]
pub struct UpdateBatchStateInput {
    /// The batch (UUID string) to advance.
    pub source_batch_id: String,
    /// The new lifecycle status token (ADR 0008).
    pub status: String,
    /// Records staged so far.
    #[specta(type = Number)]
    pub staged_count: i64,
    /// Records committed to the ledger so far.
    #[specta(type = Number)]
    pub committed_count: i64,
    /// Records skipped so far.
    #[specta(type = Number)]
    pub skipped_count: i64,
    /// Idempotency key; empty → generated server-side.
    pub idempotency_key: String,
}

/// Source-column → field mapping for a CSV import (personal-cfo-cu8), by header
/// name. Any field may be omitted (the importer auto-detects common headers).
#[derive(Debug, Clone, Default, Deserialize, Type)]
pub struct ColumnMappingDto {
    pub date: Option<String>,
    pub description: Option<String>,
    pub amount: Option<String>,
    pub debit: Option<String>,
    pub credit: Option<String>,
    pub account: Option<String>,
    pub category: Option<String>,
    pub currency: Option<String>,
    pub memo: Option<String>,
}

impl ColumnMappingDto {
    /// Lower to the importer's `ColumnMapping`.
    #[must_use]
    pub fn into_mapping(self) -> ColumnMapping {
        ColumnMapping {
            date: self.date,
            description: self.description,
            amount: self.amount,
            debit: self.debit,
            credit: self.credit,
            account: self.account,
            category: self.category,
            currency: self.currency,
            memo: self.memo,
        }
    }
}

/// Input for importing a file through the ingestion pipeline (personal-cfo-cu8).
/// The raw bytes cross the wire as a `number[]` and are parsed in the bounded
/// host; nothing is persisted unencrypted (ADR 0014 shred-after-parse).
#[derive(Debug, Clone, Deserialize, Type)]
pub struct ImportBatchInput {
    /// The raw file bytes.
    pub data: Vec<u8>,
    /// Original filename — drives plugin detection + the batch name.
    pub filename: Option<String>,
    /// The account (UUID string) to import the transactions into.
    pub target_account_id: String,
    /// An explicit importer plugin id; if omitted, the best-detected one is used.
    pub plugin_id: Option<String>,
    /// Optional column mapping (CSV).
    pub column_mapping: Option<ColumnMappingDto>,
    /// Default currency code (e.g. `"USD"`) for amounts without one.
    pub default_currency: Option<String>,
    /// An explicit date format (e.g. `"%m/%d/%Y"`) to disambiguate dates.
    pub date_format: Option<String>,
    /// Idempotency key; empty → generated server-side.
    pub idempotency_key: String,
}

/// The outcome of an import (personal-cfo-cmx `BatchResult`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct BatchResultDto {
    /// The source batch id (UUID string), if a batch was created.
    pub source_batch_id: Option<String>,
    /// Terminal status: `committed` / `partially_committed` / `failed` /
    /// `already_imported`.
    pub status: String,
    /// Transactions staged.
    pub staged: u32,
    /// Transactions committed to the ledger.
    pub committed: u32,
    /// Transactions flagged as suspected duplicates (the Money Inbox).
    pub flagged: u32,
    /// Transactions auto-categorized from merchant memory after the import (ADR 0030
    /// addendum, personal-cfo-5n4.2). 0 when the setting is off or nothing matched.
    pub auto_categorized: u32,
}

impl From<BatchResult> for BatchResultDto {
    fn from(result: BatchResult) -> Self {
        Self {
            source_batch_id: result.source_batch_id,
            status: result.status,
            staged: result.staged,
            committed: result.committed,
            flagged: result.flagged,
            auto_categorized: result.auto_categorized,
        }
    }
}

/// Input for recording a manual transaction (personal-cfo-6wgi).
#[derive(Debug, Clone, Deserialize, Type)]
pub struct RecordTransactionInput {
    /// The account (UUID string) the money moves against.
    pub account_id: String,
    /// Signed amount: positive is money in (income), negative is money out
    /// (expense). Currency must match the account's.
    pub amount: MoneyDto,
    /// When the transaction occurred, RFC 3339 (e.g. `"2026-06-07T00:00:00Z"`).
    pub occurred_at: String,
    /// Idempotency key for safe retries. If empty, the server generates one.
    pub idempotency_key: String,
}

impl RecordTransactionInput {
    /// Parse and validate into a kernel [`RecordTransaction`] command.
    ///
    /// # Errors
    /// Returns [`IpcError::Validation`] on a bad account id, unsupported
    /// currency, or a non-RFC-3339 timestamp.
    pub fn to_command(&self, transaction_id: TransactionId) -> Result<RecordTransaction, IpcError> {
        let account_id = parse_account_id(&self.account_id)?;
        let amount = self.amount.to_money()?;
        let occurred_at = DateTime::parse_from_rfc3339(self.occurred_at.trim())
            .map(|dt| dt.with_timezone(&Utc))
            .map_err(|_| {
                IpcError::Validation(format!(
                    "not a valid RFC 3339 timestamp: {:?}",
                    self.occurred_at
                ))
            })?;
        Ok(RecordTransaction::new(
            transaction_id,
            account_id,
            amount,
            occurred_at,
        ))
    }
}

/// The result of [`record_transaction`](crate::ipc::commands::record_transaction_impl):
/// the mutation outcome plus the id the new transaction was stored under
/// (personal-cfo-4d8.24.2.1), so the caller can attach category/tags/notes to it inline.
#[derive(Debug, Clone, Serialize, Type)]
pub struct RecordTransactionResult {
    /// The new transaction's id (UUID string).
    pub transaction_id: String,
    /// The op-sequence + replayed flag (idempotency), identical to other mutations.
    pub result: MutationResult,
}

/// Input to record a one-off transfer between two of the user's accounts
/// (personal-cfo-npoe).
#[derive(Debug, Clone, Deserialize, Type)]
pub struct RecordTransferInput {
    /// The account (UUID string) money moves out of.
    pub source_account_id: String,
    /// The account (UUID string) money moves into.
    pub dest_account_id: String,
    /// The positive amount moved; currency must match both accounts.
    pub amount: MoneyDto,
    /// When the transfer occurred, RFC 3339.
    pub occurred_at: String,
    /// Idempotency key for safe retries. If empty, the server generates one.
    pub idempotency_key: String,
}

impl RecordTransferInput {
    /// Parse and validate into a kernel [`Transfer`] command.
    ///
    /// # Errors
    /// Returns [`IpcError::Validation`] on a bad account id, unsupported currency,
    /// or a non-RFC-3339 timestamp.
    pub fn to_command(&self) -> Result<Transfer, IpcError> {
        let source_account_id = parse_account_id(&self.source_account_id)?;
        let dest_account_id = parse_account_id(&self.dest_account_id)?;
        let amount = self.amount.to_money()?;
        let occurred_at = DateTime::parse_from_rfc3339(self.occurred_at.trim())
            .map(|dt| dt.with_timezone(&Utc))
            .map_err(|_| {
                IpcError::Validation(format!(
                    "not a valid RFC 3339 timestamp: {:?}",
                    self.occurred_at
                ))
            })?;
        Ok(Transfer::new(
            source_account_id,
            dest_account_id,
            amount,
            occurred_at,
        ))
    }
}

/// The vault lifecycle state on the wire (mirrors `finance_kernel::VaultState`,
/// §6.2.1). Serializes as the variant name (e.g. `"Locked"`), which the frontend
/// routes on. Kept here so `specta` stays out of the kernel crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
pub enum VaultStateDto {
    NoVault,
    CreatingVault,
    Locked,
    Unlocking,
    Unlocked,
    Locking,
    Rekeying,
    Migrating,
    RestoringBackup,
    CorruptNeedsRecovery,
}

impl From<VaultState> for VaultStateDto {
    fn from(state: VaultState) -> Self {
        match state {
            VaultState::NoVault => Self::NoVault,
            VaultState::CreatingVault => Self::CreatingVault,
            VaultState::Locked => Self::Locked,
            VaultState::Unlocking => Self::Unlocking,
            VaultState::Unlocked => Self::Unlocked,
            VaultState::Locking => Self::Locking,
            VaultState::Rekeying => Self::Rekeying,
            VaultState::Migrating => Self::Migrating,
            VaultState::RestoringBackup => Self::RestoringBackup,
            VaultState::CorruptNeedsRecovery => Self::CorruptNeedsRecovery,
        }
    }
}

/// What the frontend needs to render the right screen: the current vault state
/// plus, when `Unlocked`, the account count. Carries **no key material** (§6.2 #19).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
pub struct VaultStatusDto {
    /// The current lifecycle state.
    pub state: VaultStateDto,
    /// Number of accounts — present only while the vault is unlocked.
    pub account_count: Option<u32>,
}

/// Input to change the vault master password (personal-cfo-zxq). Carries both
/// passwords across the IPC boundary; the command wraps each in `Zeroizing` on
/// use, mirroring create/unlock. `Debug` is redacted so neither password can
/// ever reach a log line (§6.6).
#[derive(Clone, Deserialize, Type)]
pub struct ChangePasswordInput {
    /// The current master password, verified against the envelope on disk.
    pub old_password: String,
    /// The replacement master password (min 8 characters, checked at the boundary).
    pub new_password: String,
}

impl std::fmt::Debug for ChangePasswordInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ChangePasswordInput([REDACTED])")
    }
}

/// One known vault in the multi-vault registry (personal-cfo-j0cg.6). Carries no key material —
/// just the id, display name, and whether it's the active vault.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct VaultSummaryDto {
    pub id: String,
    pub name: String,
    pub is_active: bool,
    /// When the vault was registered (RFC 3339) — the launch picker's date line.
    pub created_at: String,
}

/// The known vaults (personal-cfo-j0cg.6, ADR 0042).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct VaultListDto {
    pub vaults: Vec<VaultSummaryDto>,
}

/// The vault health-check result on the wire (personal-cfo-n9w): one boolean per
/// coherence check + the overall verdict. Drives the recovery wizard (5ivp).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
pub struct VaultHealthDto {
    /// The writer is in its normal state.
    pub writer_healthy: bool,
    /// WAL/journal pragmas match policy.
    pub wal_configured: bool,
    /// The migrated schema version matches the vault metadata stamp.
    pub schema_coherent: bool,
    /// `PRAGMA integrity_check` reports `ok`.
    pub integrity_ok: bool,
    /// Every stored attachment's encrypted blob is present on disk.
    pub attachments_consistent: bool,
    /// Materialized read models match a fresh compute (informational — rebuildable).
    pub read_models_current: bool,
    /// Whether the vault is healthy enough to use (read-model drift excluded).
    pub is_healthy: bool,
}

impl From<VaultHealth> for VaultHealthDto {
    fn from(h: VaultHealth) -> Self {
        Self {
            writer_healthy: h.writer_healthy,
            wal_configured: h.wal_configured,
            schema_coherent: h.schema_coherent,
            integrity_ok: h.integrity_ok,
            attachments_consistent: h.attachments_consistent,
            read_models_current: h.read_models_current,
            is_healthy: h.is_healthy(),
        }
    }
}

/// Input to create a recurring net-pay income source (personal-cfo-le79).
#[derive(Debug, Clone, Deserialize, Type)]
pub struct CreateIncomeSourceInput {
    /// Display name (e.g. the employer).
    pub name: String,
    /// Net (take-home) pay per occurrence.
    pub net_amount: MoneyDto,
    /// Pay-frequency token (`weekly` / `biweekly` / `semi_monthly` / `monthly` /
    /// `quarterly` / `annual`).
    pub frequency: String,
    /// Anchor pay date, `YYYY-MM-DD`.
    pub anchor_date: String,
    /// Optional deposit account id (UUID string).
    pub deposit_account_id: Option<String>,
    /// Idempotency key; blank is replaced with a fresh one.
    pub idempotency_key: String,
}

impl CreateIncomeSourceInput {
    /// Parse the net amount into typed [`Money`].
    pub fn net_amount(&self) -> Result<Money, IpcError> {
        self.net_amount.to_money()
    }

    /// Parse the frequency token.
    pub fn frequency(&self) -> Result<Frequency, IpcError> {
        Frequency::from_token(self.frequency.trim()).ok_or_else(|| {
            IpcError::Validation(format!("unsupported pay frequency: {}", self.frequency))
        })
    }

    /// Parse the anchor date (`YYYY-MM-DD`).
    pub fn anchor(&self) -> Result<NaiveDate, IpcError> {
        NaiveDate::parse_from_str(self.anchor_date.trim(), "%Y-%m-%d").map_err(|_| {
            IpcError::Validation(format!(
                "not a valid date (YYYY-MM-DD): {}",
                self.anchor_date
            ))
        })
    }

    /// Parse the optional deposit account id (a blank string is treated as none).
    pub fn deposit_account_id(&self) -> Result<Option<AccountId>, IpcError> {
        match self
            .deposit_account_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(raw) => Ok(Some(parse_account_id(raw)?)),
            None => Ok(None),
        }
    }
}

/// A recurring income source on the wire, with its computed next pay date.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct IncomeSourceDto {
    /// Income source id (UUID string).
    pub id: String,
    /// Display name.
    pub name: String,
    /// Net pay per occurrence.
    pub net_amount: MoneyDto,
    /// Pay-frequency token.
    pub frequency: String,
    /// Anchor pay date, `YYYY-MM-DD`.
    pub anchor_date: String,
    /// Deposit account id, if linked.
    pub deposit_account_id: Option<String>,
    /// Deposit account name, if linked.
    pub deposit_account_name: Option<String>,
    /// Next pay date on or after today, `YYYY-MM-DD`, if representable.
    pub next_pay_date: Option<String>,
    /// Whether the source is active (archived sources are excluded from the
    /// forecast). personal-cfo-tch0.
    pub active: bool,
    /// When the source was created (RFC 3339).
    pub created_at: String,
    /// When the source was archived (RFC 3339), if archived.
    pub archived_at: Option<String>,
}

impl From<IncomeSourceView> for IncomeSourceDto {
    fn from(view: IncomeSourceView) -> Self {
        Self {
            id: view.id.to_string(),
            name: view.name,
            net_amount: MoneyDto::from(view.net_amount),
            frequency: view.frequency.token(),
            anchor_date: view.anchor.to_string(),
            deposit_account_id: view.deposit_account_id.map(|a| a.to_string()),
            deposit_account_name: view.deposit_account_name,
            next_pay_date: view.next_pay_date.map(|d| d.to_string()),
            active: view.active,
            created_at: view.created_at,
            archived_at: view.archived_at,
        }
    }
}

/// Input to edit an existing net-pay income source (personal-cfo-tch0).
#[derive(Debug, Clone, Deserialize, Type)]
pub struct UpdateIncomeSourceInput {
    /// The income source id (UUID string) — the `id` on its `IncomeSourceDto`.
    pub income_source_id: String,
    /// New display name.
    pub name: String,
    /// New net pay per occurrence.
    pub net_amount: MoneyDto,
    /// New pay-frequency token.
    pub frequency: String,
    /// New anchor pay date, `YYYY-MM-DD`.
    pub anchor_date: String,
    /// New optional deposit account id (UUID string).
    pub deposit_account_id: Option<String>,
    /// Idempotency key; blank is replaced with a fresh one.
    pub idempotency_key: String,
}

impl UpdateIncomeSourceInput {
    /// Parse the income source id into a typed [`IncomeSourceId`].
    pub fn income_source_id(&self) -> Result<IncomeSourceId, IpcError> {
        parse_income_source_id(&self.income_source_id)
    }

    /// Parse the net amount into typed [`Money`].
    pub fn net_amount(&self) -> Result<Money, IpcError> {
        self.net_amount.to_money()
    }

    /// Parse the frequency token.
    pub fn frequency(&self) -> Result<Frequency, IpcError> {
        Frequency::from_token(self.frequency.trim()).ok_or_else(|| {
            IpcError::Validation(format!("unsupported pay frequency: {}", self.frequency))
        })
    }

    /// Parse the anchor date (`YYYY-MM-DD`).
    pub fn anchor(&self) -> Result<NaiveDate, IpcError> {
        NaiveDate::parse_from_str(self.anchor_date.trim(), "%Y-%m-%d").map_err(|_| {
            IpcError::Validation(format!(
                "not a valid date (YYYY-MM-DD): {}",
                self.anchor_date
            ))
        })
    }

    /// Parse the optional deposit account id (a blank string is treated as none).
    pub fn deposit_account_id(&self) -> Result<Option<AccountId>, IpcError> {
        match self
            .deposit_account_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(raw) => Ok(Some(parse_account_id(raw)?)),
            None => Ok(None),
        }
    }
}

/// The `bill_contracts.type` tokens accepted at the IPC boundary (mirrors the
/// table's CHECK constraint, so a bad value is a friendly validation error
/// rather than a raw SQLite failure).
const BILL_TYPES: &[&str] = &[
    "utility",
    "rent_mortgage",
    "insurance",
    "subscription",
    "loan_payment",
    "tax",
    "membership",
    "childcare",
    "other",
];

/// Input to dismiss a recurring-bill suggestion (ADR 0046, personal-cfo-4d8.24.6): the
/// suggestion's identity + inferred pattern, recorded so detection stops offering it until
/// the pattern materially changes.
#[derive(Debug, Clone, Deserialize, Type)]
pub struct DismissRecurringSuggestionInput {
    /// The suggestion's normalized merchant key (detection's grouping key).
    pub merchant_key: String,
    /// The suggestion's currency (part of the suppression key).
    pub currency: String,
    /// The dismissed amount (positive magnitude, minor units).
    #[specta(type = Number)]
    pub amount_minor: i64,
    /// The dismissed cadence token (`weekly` … `annual`).
    pub frequency: String,
    /// Optional free-text reason (≤ 280 chars).
    pub reason: Option<String>,
    /// Idempotency key; blank is replaced with a fresh one.
    pub idempotency_key: String,
}

/// Input to create a manual recurring bill (personal-cfo-esmy).
#[derive(Debug, Clone, Deserialize, Type)]
pub struct CreateRecurringBillInput {
    /// Display name (e.g. the merchant).
    pub name: String,
    /// Expected outflow per occurrence (positive).
    pub amount: MoneyDto,
    /// Bill-type token (`utility` / `rent_mortgage` / `insurance` / `subscription`
    /// / `loan_payment` / `tax` / `membership` / `childcare` / `other`).
    pub bill_type: String,
    /// Pay-frequency token (`weekly` / `biweekly` / `semi_monthly` / `monthly` /
    /// `quarterly` / `annual`).
    pub frequency: String,
    /// Anchor due date, `YYYY-MM-DD`.
    pub anchor_date: String,
    /// Optional autopay account id (UUID string).
    pub autopay_account_id: Option<String>,
    /// Whether the bill autopays (ADR 0041, personal-cfo-mc7f). `None`/`false` → manual.
    pub autopay: Option<bool>,
    /// Optional free-text description.
    pub description: Option<String>,
    /// The normalized merchant key of the recurring candidate this bill is promoted from
    /// (personal-cfo-5n4.8), when created from a suggestion — persisted so the suggestion
    /// stays suppressed after a rename. `None` for a manually-created bill.
    pub source_merchant_key: Option<String>,
    /// The bill's category id (UUID string), set when promoting (personal-cfo-4d8.24.5).
    /// `None`/blank leaves the bill uncategorized.
    pub category_id: Option<String>,
    /// Tag ids (UUID strings) to apply to the bill (personal-cfo-4d8.24.5.1); empty for
    /// an untagged bill. Each must be an existing tag.
    pub tag_ids: Vec<String>,
    /// Idempotency key; blank is replaced with a fresh one.
    pub idempotency_key: String,
}

impl CreateRecurringBillInput {
    /// Parse the amount into typed [`Money`].
    pub fn amount(&self) -> Result<Money, IpcError> {
        self.amount.to_money()
    }

    /// Validate the bill-type token against the accepted set.
    pub fn bill_type(&self) -> Result<String, IpcError> {
        let token = self.bill_type.trim();
        if BILL_TYPES.contains(&token) {
            Ok(token.to_owned())
        } else {
            Err(IpcError::Validation(format!(
                "unsupported bill type: {}",
                self.bill_type
            )))
        }
    }

    /// Parse the frequency token.
    pub fn frequency(&self) -> Result<Frequency, IpcError> {
        Frequency::from_token(self.frequency.trim()).ok_or_else(|| {
            IpcError::Validation(format!("unsupported pay frequency: {}", self.frequency))
        })
    }

    /// Parse the anchor date (`YYYY-MM-DD`).
    pub fn anchor(&self) -> Result<NaiveDate, IpcError> {
        NaiveDate::parse_from_str(self.anchor_date.trim(), "%Y-%m-%d").map_err(|_| {
            IpcError::Validation(format!(
                "not a valid date (YYYY-MM-DD): {}",
                self.anchor_date
            ))
        })
    }

    /// Parse the optional autopay account id (a blank string is treated as none).
    pub fn autopay_account_id(&self) -> Result<Option<AccountId>, IpcError> {
        match self
            .autopay_account_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(raw) => Ok(Some(parse_account_id(raw)?)),
            None => Ok(None),
        }
    }

    /// The normalized description — trimmed; blank becomes `None`.
    pub fn description(&self) -> Option<String> {
        normalize_description(self.description.as_deref())
    }
}

/// Input to confirm a recurring bill occurrence paid early (personal-cfo-5ie.9).
#[derive(Debug, Clone, Deserialize, Type)]
pub struct ConfirmObligationEarlyInput {
    pub recurring_event_id: String,
    /// The occurrence's scheduled due date, `YYYY-MM-DD`.
    pub scheduled_date: String,
    /// The amount actually paid — non-negative; zero means nothing was due this cycle.
    pub actual_amount: MoneyDto,
    /// When it was actually paid, `YYYY-MM-DD` (must be on or before today).
    pub actual_date: String,
    /// The liquid account it was paid from.
    pub paying_account_id: String,
    pub idempotency_key: String,
}

impl ConfirmObligationEarlyInput {
    pub fn recurring_event_id(&self) -> Result<RecurringEventId, IpcError> {
        parse_recurring_event_id(&self.recurring_event_id)
    }
    pub fn paying_account_id(&self) -> Result<AccountId, IpcError> {
        parse_account_id(&self.paying_account_id)
    }
    pub fn scheduled_date(&self) -> Result<NaiveDate, IpcError> {
        NaiveDate::parse_from_str(self.scheduled_date.trim(), "%Y-%m-%d")
            .map_err(|_| IpcError::Validation("scheduled_date must be YYYY-MM-DD".to_owned()))
    }
    /// The actual pay date at noon UTC (a stable time-of-day; the kernel compares by date).
    pub fn actual_date(&self) -> Result<DateTime<Utc>, IpcError> {
        let day = NaiveDate::parse_from_str(self.actual_date.trim(), "%Y-%m-%d")
            .map_err(|_| IpcError::Validation("actual_date must be YYYY-MM-DD".to_owned()))?;
        Ok(day.and_hms_opt(12, 0, 0).unwrap_or_default().and_utc())
    }
}

/// Input to reverse an early confirm (personal-cfo-5ie.9).
#[derive(Debug, Clone, Deserialize, Type)]
pub struct UnconfirmObligationInput {
    pub recurring_event_id: String,
    pub scheduled_date: String,
    pub idempotency_key: String,
}

impl UnconfirmObligationInput {
    pub fn recurring_event_id(&self) -> Result<RecurringEventId, IpcError> {
        parse_recurring_event_id(&self.recurring_event_id)
    }
    pub fn scheduled_date(&self) -> Result<NaiveDate, IpcError> {
        NaiveDate::parse_from_str(self.scheduled_date.trim(), "%Y-%m-%d")
            .map_err(|_| IpcError::Validation("scheduled_date must be YYYY-MM-DD".to_owned()))
    }
}

/// Input to edit an existing manual recurring bill (personal-cfo-zl1l).
#[derive(Debug, Clone, Deserialize, Type)]
pub struct UpdateRecurringBillInput {
    /// The recurring bill id (UUID string) — the `id` on its `RecurringBillDto`.
    pub bill_id: String,
    /// New display name.
    pub name: String,
    /// New expected outflow per occurrence (positive).
    pub amount: MoneyDto,
    /// New bill-type token.
    pub bill_type: String,
    /// New pay-frequency token.
    pub frequency: String,
    /// New anchor due date, `YYYY-MM-DD`.
    pub anchor_date: String,
    /// New optional autopay account id (UUID string).
    pub autopay_account_id: Option<String>,
    /// Whether the bill autopays (ADR 0041, personal-cfo-mc7f). `None` leaves it unchanged.
    pub autopay: Option<bool>,
    /// New optional free-text description.
    pub description: Option<String>,
    /// Idempotency key; blank is replaced with a fresh one.
    pub idempotency_key: String,
}

impl UpdateRecurringBillInput {
    /// Parse the bill id into a typed [`RecurringEventId`].
    pub fn bill_id(&self) -> Result<RecurringEventId, IpcError> {
        parse_recurring_event_id(&self.bill_id)
    }

    /// Parse the amount into typed [`Money`].
    pub fn amount(&self) -> Result<Money, IpcError> {
        self.amount.to_money()
    }

    /// Validate the bill-type token against the accepted set.
    pub fn bill_type(&self) -> Result<String, IpcError> {
        let token = self.bill_type.trim();
        if BILL_TYPES.contains(&token) {
            Ok(token.to_owned())
        } else {
            Err(IpcError::Validation(format!(
                "unsupported bill type: {}",
                self.bill_type
            )))
        }
    }

    /// Parse the frequency token.
    pub fn frequency(&self) -> Result<Frequency, IpcError> {
        Frequency::from_token(self.frequency.trim()).ok_or_else(|| {
            IpcError::Validation(format!("unsupported pay frequency: {}", self.frequency))
        })
    }

    /// Parse the anchor date (`YYYY-MM-DD`).
    pub fn anchor(&self) -> Result<NaiveDate, IpcError> {
        NaiveDate::parse_from_str(self.anchor_date.trim(), "%Y-%m-%d").map_err(|_| {
            IpcError::Validation(format!(
                "not a valid date (YYYY-MM-DD): {}",
                self.anchor_date
            ))
        })
    }

    /// Parse the optional autopay account id (a blank string is treated as none).
    pub fn autopay_account_id(&self) -> Result<Option<AccountId>, IpcError> {
        match self
            .autopay_account_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(raw) => Ok(Some(parse_account_id(raw)?)),
            None => Ok(None),
        }
    }

    /// The normalized description — trimmed; blank becomes `None`.
    pub fn description(&self) -> Option<String> {
        normalize_description(self.description.as_deref())
    }
}

/// A manual recurring bill on the wire, with its computed next due date.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct RecurringBillDto {
    /// Recurring bill id (UUID string).
    pub id: String,
    /// Display name.
    pub name: String,
    /// Bill-type token.
    pub bill_type: String,
    /// Expected outflow per occurrence.
    pub amount: MoneyDto,
    /// Pay-frequency token.
    pub frequency: String,
    /// Anchor due date, `YYYY-MM-DD`.
    pub anchor_date: String,
    /// Autopay account id, if linked.
    pub autopay_account_id: Option<String>,
    /// Autopay account name, if linked.
    pub autopay_account_name: Option<String>,
    /// Whether the bill is marked autopay (ADR 0041, personal-cfo-mc7f).
    pub autopay_enabled: bool,
    /// Next due date on or after today, `YYYY-MM-DD`, if representable.
    pub next_due_date: Option<String>,
    /// Optional free-text description.
    pub description: Option<String>,
    /// Whether the bill is active (archived bills are excluded from the forecast).
    pub active: bool,
    /// When the bill was created (RFC 3339).
    pub created_at: String,
    /// When the bill was archived (RFC 3339), if it is archived.
    pub archived_at: Option<String>,
    /// The bill's category id (UUID string), if categorized (personal-cfo-4d8.24.5).
    pub category_id: Option<String>,
    /// The bill's tag ids (UUID strings); empty when untagged (personal-cfo-4d8.24.5.1).
    pub tag_ids: Vec<String>,
}

impl From<RecurringBillView> for RecurringBillDto {
    fn from(view: RecurringBillView) -> Self {
        Self {
            id: view.id.to_string(),
            name: view.name,
            bill_type: view.bill_type,
            amount: MoneyDto::from(view.amount),
            frequency: view.frequency.token(),
            anchor_date: view.anchor.to_string(),
            autopay_account_id: view.autopay_account_id.map(|a| a.to_string()),
            autopay_account_name: view.autopay_account_name,
            autopay_enabled: view.autopay_enabled,
            next_due_date: view.next_due_date.map(|d| d.to_string()),
            description: view.description,
            active: view.active,
            created_at: view.created_at,
            archived_at: view.archived_at,
            category_id: view.category_id.map(|c| c.to_string()),
            tag_ids: view.tag_ids.iter().map(ToString::to_string).collect(),
        }
    }
}

/// Input to mark a recurring bill autopay or manual (ADR 0041, personal-cfo-mc7f).
#[derive(Debug, Clone, Deserialize, Type)]
pub struct SetBillAutopayInput {
    pub event_id: String,
    /// `true` = autopay, `false` = manual.
    pub autopay: bool,
    pub idempotency_key: String,
}

impl SetBillAutopayInput {
    pub fn event_id(&self) -> Result<RecurringEventId, IpcError> {
        parse_recurring_event_id(&self.event_id)
    }
}

/// A recurring transfer for the list UI (ADR 0026 §14, personal-cfo-npoe).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct RecurringTransferDto {
    /// Recurring transfer id (UUID string).
    pub id: String,
    /// The source account id (UUID string).
    pub source_account_id: String,
    /// The source account's display name.
    pub source_account_name: String,
    /// The destination account id (UUID string).
    pub dest_account_id: String,
    /// The destination account's display name.
    pub dest_account_name: String,
    /// The amount moved each occurrence.
    pub amount: MoneyDto,
    /// Pay-frequency token.
    pub frequency: String,
    /// Anchor occurrence date, `YYYY-MM-DD`.
    pub anchor_date: String,
    /// Next occurrence on or after today, `YYYY-MM-DD`, if representable.
    pub next_date: Option<String>,
    /// When the transfer was created (RFC 3339).
    pub created_at: String,
}

impl From<RecurringTransferView> for RecurringTransferDto {
    fn from(view: RecurringTransferView) -> Self {
        Self {
            id: view.id.to_string(),
            source_account_id: view.source_account_id.to_string(),
            source_account_name: view.source_account_name,
            dest_account_id: view.dest_account_id.to_string(),
            dest_account_name: view.dest_account_name,
            amount: MoneyDto::from(view.amount),
            frequency: view.frequency.token(),
            anchor_date: view.anchor.to_string(),
            next_date: view.next_date.map(|d| d.to_string()),
            created_at: view.created_at,
        }
    }
}

/// Input to create a recurring transfer (ADR 0026 §14, personal-cfo-npoe).
#[derive(Debug, Clone, Deserialize, Type)]
pub struct CreateRecurringTransferInput {
    /// The source account id (UUID string).
    pub source_account_id: String,
    /// The destination account id (UUID string).
    pub dest_account_id: String,
    /// The positive amount moved each occurrence.
    pub amount: MoneyDto,
    /// Pay-frequency token (`weekly` / `biweekly` / `semi_monthly` / `monthly` /
    /// `quarterly` / `annual`).
    pub frequency: String,
    /// Anchor occurrence date, `YYYY-MM-DD`.
    pub anchor_date: String,
    /// Idempotency key; blank is replaced with a fresh one.
    pub idempotency_key: String,
}

impl CreateRecurringTransferInput {
    /// Parse the frequency token into a typed [`Frequency`].
    pub fn frequency(&self) -> Result<Frequency, IpcError> {
        Frequency::from_token(self.frequency.trim())
            .ok_or_else(|| IpcError::Validation(format!("unknown frequency: {:?}", self.frequency)))
    }

    /// Parse the anchor date.
    pub fn anchor(&self) -> Result<NaiveDate, IpcError> {
        NaiveDate::parse_from_str(self.anchor_date.trim(), "%Y-%m-%d")
            .map_err(|_| IpcError::Validation(format!("not a valid date: {:?}", self.anchor_date)))
    }
}

/// Result of creating a recurring transfer: the new id plus the mutation outcome.
#[derive(Debug, Clone, Serialize, Type)]
pub struct CreateRecurringTransferResult {
    /// The new recurring transfer id (UUID string).
    pub recurring_transfer_id: String,
    pub mutation: MutationResult,
}

/// Why a forecast event is assumed — provenance on the wire (ADR 0026 §1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct AssumptionBasisDto {
    /// `recurring_schedule` | `manual_one_off`.
    pub kind: String,
    /// The cadence token for a recurring schedule (e.g. `monthly`), else null.
    pub frequency: Option<String>,
}

impl From<AssumptionBasis> for AssumptionBasisDto {
    fn from(basis: AssumptionBasis) -> Self {
        match basis {
            AssumptionBasis::RecurringSchedule { frequency } => Self {
                kind: "recurring_schedule".to_owned(),
                frequency: Some(frequency.token()),
            },
            AssumptionBasis::ManualOneOff => Self {
                kind: "manual_one_off".to_owned(),
                frequency: None,
            },
        }
    }
}

/// A projected cash event on a forecast day (personal-cfo-164u).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct ForecastEventDto {
    /// The source entity id (UUID string) this event was projected from.
    pub source_event_id: String,
    /// The source entity's display name (e.g. `"Rent"`, `"Acme Corp"`).
    pub name: String,
    /// Source-type token: `income` / `recurring_bill` / `loan_payment` /
    /// `transfer` / `manual_entry`.
    pub kind: String,
    /// Signed amount applied to the running balance (inflow +, outflow −).
    pub amount: MoneyDto,
    /// Why this event is assumed (provenance for row explanation).
    pub assumption_basis: AssumptionBasisDto,
}

impl From<ForecastEventView> for ForecastEventDto {
    fn from(view: ForecastEventView) -> Self {
        Self {
            source_event_id: view.source_event_id.to_string(),
            name: view.name,
            kind: view.kind,
            amount: MoneyDto::from(view.amount),
            assumption_basis: AssumptionBasisDto::from(view.assumption_basis),
        }
    }
}

/// A forecast value as a P10/P50/P90 band on the wire (ADR 0026 §1). Collapsed
/// (all three equal) for the deterministic Layer-1 engine; widened by the
/// statistical layers later — so the chart is built once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct BandDto {
    /// 10th-percentile (pessimistic) projected balance.
    pub p10: MoneyDto,
    /// 50th-percentile (median / deterministic) projected balance.
    pub p50: MoneyDto,
    /// 90th-percentile (optimistic) projected balance.
    pub p90: MoneyDto,
}

impl From<Band> for BandDto {
    fn from(band: Band) -> Self {
        Self {
            p10: MoneyDto::from(band.p10),
            p50: MoneyDto::from(band.p50),
            p90: MoneyDto::from(band.p90),
        }
    }
}

/// One day of the Future Cash series: the closing balance (as a band) and the
/// events that moved it (empty on quiet days).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct ForecastDayDto {
    /// The household-local calendar date, `YYYY-MM-DD`.
    pub date: String,
    /// Projected liquid-cash balance at end of day, as a P10/P50/P90 band.
    pub closing: BandDto,
    /// Events applied on this day, in canonical same-day order.
    pub events: Vec<ForecastEventDto>,
}

impl From<ForecastDayView> for ForecastDayDto {
    fn from(view: ForecastDayView) -> Self {
        Self {
            date: view.date.to_string(),
            closing: BandDto::from(view.closing),
            events: view
                .events
                .into_iter()
                .map(ForecastEventDto::from)
                .collect(),
        }
    }
}

/// The Future Cash forecast (personal-cfo-164u): the opening liquid balance plus
/// the per-day projected series. Backs the dashboard's Future Cash widgets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct ForecastViewDto {
    /// ISO-4217 currency code the forecast is computed in (e.g. `"USD"`).
    pub currency: String,
    /// Liquid-cash balance at the start of the horizon ("today").
    pub starting_balance: MoneyDto,
    /// The first projected day (household-local "today"), `YYYY-MM-DD`.
    pub start_date: String,
    /// Number of days projected (the requested horizon). Exported as TS `number`.
    pub horizon_days: u32,
    /// One row per calendar day in the horizon, ascending.
    pub days: Vec<ForecastDayDto>,
}

impl From<ForecastView> for ForecastViewDto {
    fn from(view: ForecastView) -> Self {
        Self {
            currency: view.currency.code().to_owned(),
            starting_balance: MoneyDto::from(view.starting_balance),
            start_date: view.start_date.to_string(),
            horizon_days: view.horizon_days,
            days: view.days.into_iter().map(ForecastDayDto::from).collect(),
        }
    }
}

/// One projected credit-card billing cycle (ADR 0039 §2, personal-cfo-4lhm). Money fields are
/// minor units in the card's currency (carried on [`CardStatementForecastDto`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct CardCycleDto {
    /// Statement close date, `YYYY-MM-DD`.
    pub close_date: String,
    /// Payment due date, `YYYY-MM-DD`.
    pub due_date: String,
    /// Carried (owed) balance at the start of this cycle (minor units).
    #[specta(type = Number)]
    pub carried_opening_balance_minor: i64,
    /// Known card-charged bill charges posting this cycle (minor units).
    #[specta(type = Number)]
    pub known_charges_minor: i64,
    /// Projected ordinary variable card spend this cycle (minor units).
    #[specta(type = Number)]
    pub projected_variable_minor: i64,
    /// Projected finance charge (interest) accrued this cycle (minor units).
    #[specta(type = Number)]
    pub accrued_interest_minor: i64,
    /// Statement balance = carried opening + known charges + projected spend + interest.
    #[specta(type = Number)]
    pub statement_balance_minor: i64,
    /// Minimum payment due (minor units).
    #[specta(type = Number)]
    pub minimum_due_minor: i64,
    /// Full-statement payoff (minor units).
    #[specta(type = Number)]
    pub full_pay_minor: i64,
    /// The payment the repayment philosophy selects on the due date (minor units).
    #[specta(type = Number)]
    pub forecast_payment_minor: i64,
    /// Whether the statement balance is the user's recorded REAL statement rather than an
    /// estimate (feedback 2026-07-03).
    pub statement_is_actual: bool,
    /// Whether this cycle has closed as of the HOUSEHOLD calendar day (ADR 0039 addendum
    /// 2026-07-10 §1) — the UI gates the record-statement affordance on this, never on the
    /// browser's local day.
    pub is_closed: bool,
}

impl From<CardCycleView> for CardCycleDto {
    fn from(v: CardCycleView) -> Self {
        Self {
            close_date: v.close_date.to_string(),
            due_date: v.due_date.to_string(),
            carried_opening_balance_minor: v.carried_opening_balance_minor,
            known_charges_minor: v.known_charges_minor,
            projected_variable_minor: v.projected_variable_minor,
            accrued_interest_minor: v.accrued_interest_minor,
            statement_balance_minor: v.statement_balance_minor,
            minimum_due_minor: v.minimum_due_minor,
            full_pay_minor: v.full_pay_minor,
            forecast_payment_minor: v.forecast_payment_minor,
            statement_is_actual: v.statement_is_actual,
            is_closed: v.is_closed,
        }
    }
}

/// Input recording (or clearing) a card statement's REAL balance for one cycle
/// (feedback 2026-07-03).
#[derive(Debug, Clone, Deserialize, Type)]
pub struct SetCardStatementBalanceInput {
    /// The credit-card account id (UUID string).
    pub account_id: String,
    /// The statement close date identifying the cycle, `YYYY-MM-DD`.
    pub cycle_close: String,
    /// The actual statement balance owed in minor units (≥ 0), or `null` to clear the
    /// assertion and fall back to the estimate.
    #[specta(type = Option<Number>)]
    pub statement_balance_minor: Option<i64>,
    /// Idempotency key; blank is replaced with a fresh one.
    pub idempotency_key: String,
}

/// A credit card's projected statement + payment forecast over its next cycles
/// (personal-cfo-4lhm). Backs the credit-card view (`4piy`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct CardStatementForecastDto {
    /// The credit-card account id.
    pub account_id: String,
    /// The account display name.
    pub account_name: String,
    /// ISO-4217 currency code (e.g. `"USD"`).
    pub currency: String,
    /// The credit limit in minor units (for utilization); `0` when unset.
    #[specta(type = Number)]
    pub credit_limit_minor: i64,
    /// The repayment philosophy token (ADR 0035 §1).
    pub repayment_philosophy: String,
    /// The upcoming projected cycles, soonest first.
    pub cycles: Vec<CardCycleDto>,
    /// Every user-recorded statement row for the card, newest first — the management list
    /// (ADR 0039 addendum 2026-07-10 §1): stale rows keyed to a close the derivation no
    /// longer leads with stay visible and clearable (personal-cfo-4d8.25.2).
    pub stored_statements: Vec<StoredStatementDto>,
    /// Which signal tier produced the projected-spend estimate (ADR 0039 addendum
    /// 2026-07-10 §2): `card_history` / `statement_history` / `categorized_average` / `none`
    /// — surfaced for forecast explainability.
    pub estimate_basis: String,
    /// The statement estimator's walk-forward MAPE (bps), `0` when none is recorded yet —
    /// sizes the owed-balance uncertainty band on the Account Detail chart (ADR 0050).
    #[specta(type = Number)]
    pub estimate_mape_bps: i64,
    /// How many samples (cycle windows or recorded statements) the estimate is fitted on.
    #[specta(type = Number)]
    pub estimate_sample_cycles: i64,
}

/// One user-recorded statement row `(cycle close, amount)` on a card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct StoredStatementDto {
    /// The cycle-close date the row is keyed to, `YYYY-MM-DD`.
    pub close_date: String,
    /// The recorded statement balance (minor units, ≥ 0).
    #[specta(type = Number)]
    pub statement_balance_minor: i64,
    /// Whether the projection applies this row (close on/before the household calendar day).
    /// A future-keyed stale row is inert and the UI flags it — computed server-side so the
    /// badge can never disagree with the projection gate.
    pub applied: bool,
}

impl From<CardStatementForecastView> for CardStatementForecastDto {
    fn from(v: CardStatementForecastView) -> Self {
        Self {
            account_id: v.account_id.to_string(),
            account_name: v.account_name,
            currency: v.currency.code().to_owned(),
            credit_limit_minor: v.credit_limit_minor,
            repayment_philosophy: v.repayment_philosophy,
            cycles: v.cycles.into_iter().map(CardCycleDto::from).collect(),
            stored_statements: v
                .stored_statements
                .into_iter()
                .map(|row| StoredStatementDto {
                    close_date: row.close_date.to_string(),
                    statement_balance_minor: row.statement_balance_minor,
                    applied: row.applied,
                })
                .collect(),
            estimate_basis: v.estimate_basis,
            estimate_mape_bps: v.estimate_mape_bps,
            estimate_sample_cycles: v.estimate_sample_cycles,
        }
    }
}

/// One past billing-cycle window for a card: the derived-from-imports charge total and any
/// user-recorded actual statement — the statement-history capture surface (ADR 0039 addendum
/// 2026-07-10 §2, personal-cfo-4d8.25.4). Newest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct CardStatementHistoryDto {
    /// The window's open date (`YYYY-MM-DD`, the previous close).
    pub window_open: String,
    /// The cycle-close date the window (and any recorded statement) is keyed to, `YYYY-MM-DD`.
    pub close_date: String,
    /// Sum of imported charges in the window (minor units), or `null` when the card's
    /// transaction history does not fully cover it.
    #[specta(type = Option<Number>)]
    pub derived_charges_minor: Option<i64>,
    /// The user-recorded actual statement for this close (minor units), if any.
    #[specta(type = Option<Number>)]
    pub stored_statement_minor: Option<i64>,
}

impl From<CardStatementHistoryView> for CardStatementHistoryDto {
    fn from(v: CardStatementHistoryView) -> Self {
        Self {
            window_open: v.window_open.to_string(),
            close_date: v.close_date.to_string(),
            derived_charges_minor: v.derived_charges_minor,
            stored_statement_minor: v.stored_statement_minor,
        }
    }
}

/// One projected occurrence of a recurring bill with its realized link — the retro-attach
/// surface (ADR 0047 §1, personal-cfo-4d8.25.8): a `paid` status with a linked transaction
/// means a real historical posting matched this occurrence of the schedule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct RecurringBillOccurrenceDto {
    /// The occurrence's scheduled date, `YYYY-MM-DD`.
    pub scheduled_date: String,
    /// The expected amount (positive magnitude, minor units).
    #[specta(type = Number)]
    pub expected_amount_minor: i64,
    /// ISO-4217 currency code.
    pub currency: String,
    /// `scheduled` | `paid` | `skipped` | `overridden`.
    pub status: String,
    /// The realized transaction this occurrence linked to, if any.
    pub linked_transaction_id: Option<String>,
}

impl From<RecurringInstanceRow> for RecurringBillOccurrenceDto {
    fn from(v: RecurringInstanceRow) -> Self {
        Self {
            scheduled_date: v.scheduled_date,
            expected_amount_minor: v.expected_amount_minor,
            currency: v.currency,
            status: v.status,
            linked_transaction_id: v.linked_transaction_id.map(|id| id.to_string()),
        }
    }
}

/// One debt-paydown plan's projected outcome (personal-cfo-od07, ADR 0036 debt_payoff). Backs
/// the Debt sub-view's descriptive strategy compare (ADR 0018 — never "best"/"recommended").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct DebtPayoffPlanDto {
    /// The strategy token: `minimum_only` / `snowball` / `avalanche`.
    pub strategy: String,
    /// Months until debt-free, or `null` if not within the horizon (a minimum below the
    /// interest never amortizes).
    pub debt_free_month: Option<u32>,
    /// Total interest paid across the paydown (minor units).
    #[specta(type = Number)]
    pub total_interest_minor: i64,
    /// The household reference currency the amounts are in (personal-cfo-6wk.13).
    pub currency: String,
    /// The total owed balance at the end of each month (personal-cfo-6wk.16): index `0` is the
    /// current total, index `i` the total after month `i` — the burndown-chart series.
    #[specta(type = Vec<Number>)]
    pub monthly_total_owed_minor: Vec<i64>,
    /// Per-debt owed-balance-over-time (personal-cfo-6wk.17): one labelled series per debt, each
    /// month-indexed like `monthly_total_owed_minor` and summing to it — the per-debt breakdown.
    pub per_debt: Vec<PayoffDebtSeriesDto>,
}

/// One debt's owed-balance-over-time within a payoff plan (personal-cfo-6wk.17).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct PayoffDebtSeriesDto {
    /// The debt's account label.
    pub label: String,
    /// Owed balance at the end of each month, minor units (index `0` = current).
    #[specta(type = Vec<Number>)]
    pub monthly_owed_minor: Vec<i64>,
}

impl From<PayoffDebtSeries> for PayoffDebtSeriesDto {
    fn from(v: PayoffDebtSeries) -> Self {
        Self {
            label: v.label,
            monthly_owed_minor: v.monthly_owed_minor,
        }
    }
}

impl From<PayoffPlanView> for DebtPayoffPlanDto {
    fn from(v: PayoffPlanView) -> Self {
        Self {
            strategy: v.strategy,
            debt_free_month: v.debt_free_month,
            total_interest_minor: v.total_interest_minor,
            currency: v.currency,
            monthly_total_owed_minor: v.monthly_total_owed_minor,
            per_debt: v
                .per_debt
                .into_iter()
                .map(PayoffDebtSeriesDto::from)
                .collect(),
        }
    }
}

/// A suspected loan double-count (personal-cfo-6wk.11): a loan tracked as both a
/// `loan_liability` account with payment terms and an active recurring `loan_payment` bill,
/// which double-counts its payment in the liquid forecast. Descriptive only (ADR 0018).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct LoanDoubleCountWarningDto {
    /// The `loan_liability` account (plain UUID, matches `AccountViewDto.account_id`).
    pub loan_account_id: String,
    pub loan_name: String,
    /// The `loan_payment` recurring bill (plain UUID, matches the recurring-bill view id).
    pub bill_event_id: String,
    pub bill_name: String,
    /// The normalized names look like the same loan.
    pub name_match: bool,
    /// The loan's fixed monthly payment equals the bill amount (same currency).
    pub amount_match: bool,
}

impl From<LoanDoubleCount> for LoanDoubleCountWarningDto {
    fn from(v: LoanDoubleCount) -> Self {
        Self {
            loan_account_id: v.loan_account_id.to_string(),
            loan_name: v.loan_name,
            bill_event_id: v.bill_event_id.to_string(),
            bill_name: v.bill_name,
            name_match: v.name_match,
            amount_match: v.amount_match,
        }
    }
}

/// The in-app update check result (personal-cfo-1ik.3). `checked` is false when the check
/// could not run (no source checkout / no upstream / git unavailable), with `error` set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct UpdateStatusDto {
    /// The running app's semver (`CARGO_PKG_VERSION`).
    pub current_version: String,
    /// The commit the running binary was built from (short hash, or `"unknown"`).
    pub current_commit: String,
    /// Which build this is: `dev` | `release` (or an explicit build channel) —
    /// personal-cfo-4d8.27.3.1, so the app can say what you are running.
    pub build_channel: String,
    /// When this binary was built (RFC 3339 UTC).
    pub build_time: String,
    /// Whether tracked files were modified in the worktree at build time.
    pub build_dirty: bool,
    /// The upstream's latest commit (short hash), when the check ran.
    pub latest_commit: Option<String>,
    /// How many commits behind the upstream the built commit is (`null` if not determinable).
    pub commits_behind: Option<u32>,
    /// The upstream commit's date (`YYYY-MM-DD`), when known.
    pub latest_date: Option<String>,
    /// True only when the check ran and the built commit is at the upstream tip.
    pub up_to_date: bool,
    /// Whether the check actually ran (vs. degraded to `error`).
    pub checked: bool,
    /// Why the check could not run, when `checked` is false.
    pub error: Option<String>,
}

impl From<crate::update::UpdateStatus> for UpdateStatusDto {
    fn from(v: crate::update::UpdateStatus) -> Self {
        Self {
            current_version: v.current_version,
            current_commit: v.current_commit,
            build_channel: v.build_channel,
            build_time: v.build_time,
            build_dirty: v.build_dirty,
            latest_commit: v.latest_commit,
            commits_behind: v.commits_behind,
            latest_date: v.latest_date,
            up_to_date: v.up_to_date,
            checked: v.checked,
            error: v.error,
        }
    }
}

/// The running binary's build identity (personal-cfo-4d8.27.3.2). Compile-time constants
/// only — NO git, no network — so the app can answer "which build is this?" instantly, at
/// launch, and while offline. The update *check* (`check_for_update`) is a separate,
/// network-bound concern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct BuildInfoDto {
    /// The app's semver (`CARGO_PKG_VERSION`).
    pub version: String,
    /// The commit the binary was built from (short hash, or `"unknown"`).
    pub commit: String,
    /// `dev` | `release` (or an explicit `PCFO_BUILD_CHANNEL`, e.g. `beta`).
    pub channel: String,
    /// When the binary was built (RFC 3339 UTC).
    pub built_at: String,
    /// Whether tracked files were modified in the worktree at build time.
    pub dirty: bool,
}

/// The outcome of applying an in-app update (personal-cfo-1ik.4): whether the rebuild+reinstall
/// succeeded, and the tail of its output to surface on failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct ApplyUpdateResultDto {
    pub ok: bool,
    pub output_tail: String,
}

impl From<crate::update::ApplyResult> for ApplyUpdateResultDto {
    fn from(v: crate::update::ApplyResult) -> Self {
        Self {
            ok: v.ok,
            output_tail: v.output_tail,
        }
    }
}

/// A detected recurring-bill candidate (personal-cfo-98ql): a merchant that recurs at a
/// consistent cadence + amount in the realized history. A suggestion for the user to confirm
/// (ADR 0018 — never auto-created); backs the "Suggested recurring" review surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct RecurringCandidateDto {
    /// The normalized merchant key (the grouping key).
    pub merchant_key: String,
    /// A human label (the most recent observation's raw payee/memo).
    pub display: String,
    /// The representative (median) charge magnitude, minor units.
    #[specta(type = Number)]
    pub amount_minor: i64,
    /// The smallest observed charge magnitude, minor units (personal-cfo-4d8.24.5). Equal to
    /// `amount_max_minor` for a fixed-amount bill; a range for a variable one.
    #[specta(type = Number)]
    pub amount_min_minor: i64,
    /// The largest observed charge magnitude, minor units.
    #[specta(type = Number)]
    pub amount_max_minor: i64,
    pub currency: String,
    /// The inferred cadence as a frequency token (`weekly` … `annual`).
    pub frequency: String,
    /// The most recent observed charge date, `YYYY-MM-DD` (personal-cfo-4d8.24.5).
    pub last_seen: String,
    /// The next expected charge date, `YYYY-MM-DD`.
    pub next_date: String,
    /// How many matching charges were observed.
    pub occurrence_count: u32,
    /// Detection confidence in basis points (0..=10000).
    #[specta(type = Number)]
    pub confidence_bps: i64,
    /// The dominant category across the merchant's observed charges (UUID string), or null
    /// when none were categorized — pre-fills the promote form (personal-cfo-4d8.24.5).
    pub dominant_category_id: Option<String>,
    /// Display names of the accounts the observations posted to, most-frequent first —
    /// the "always on Venture X" provenance (personal-cfo-4d8.25.11).
    pub source_account_names: Vec<String>,
    /// The single account ALL observations posted to (UUID string), when there is exactly
    /// one — the ADR 0047 §4 autopay/pay-from prefill; null for a multi-account series.
    pub source_account_id: Option<String>,
    /// The median observed day-of-month for month-stepped cadences ("roughly the 19th"),
    /// else null.
    #[specta(type = Option<Number>)]
    pub typical_day_of_month: Option<u32>,
    /// The observed charges behind the suggestion, most recent first (capped) — the
    /// proof expander's rows.
    pub observations: Vec<CandidateObservationDto>,
}

/// One observed charge behind a recurring candidate (personal-cfo-4d8.25.11).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct CandidateObservationDto {
    /// The charge date, `YYYY-MM-DD`.
    pub date: String,
    /// The charge magnitude, minor units (positive).
    #[specta(type = Number)]
    pub amount_minor: i64,
    /// The account the charge posted to.
    pub account_name: String,
}

impl From<RecurringCandidateView> for RecurringCandidateDto {
    fn from(view: RecurringCandidateView) -> Self {
        let candidate = view.candidate;
        Self {
            merchant_key: candidate.merchant_key,
            display: candidate.display,
            amount_minor: candidate.amount_minor,
            amount_min_minor: candidate.amount_min_minor,
            amount_max_minor: candidate.amount_max_minor,
            currency: candidate.currency,
            frequency: candidate.frequency.to_owned(),
            last_seen: candidate.last_seen.to_string(),
            next_date: candidate.next_date.to_string(),
            occurrence_count: u32::try_from(candidate.occurrence_count).unwrap_or(u32::MAX),
            confidence_bps: candidate.confidence_bps,
            dominant_category_id: view.dominant_category_id.map(|id| id.to_string()),
            source_account_names: view.source_account_names,
            source_account_id: view.source_account_id.map(|id| id.to_string()),
            typical_day_of_month: view.typical_day_of_month,
            observations: view
                .observations
                .into_iter()
                .map(|o| CandidateObservationDto {
                    date: o.date.to_string(),
                    amount_minor: o.amount_minor,
                    account_name: o.account_name,
                })
                .collect(),
        }
    }
}

/// One projected series — a liquid account or the synthetic "Unallocated cash"
/// bucket (`account_id == null`) — for the multi-series chart + table (ADR 0026
/// §12, personal-cfo-l8oh).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct AccountSeriesDto {
    /// The liquid account id (UUID string), or `null` for the Unallocated series.
    pub account_id: Option<String>,
    /// Display name (account name, or `"Unallocated cash"`).
    pub name: String,
    /// The account's subtype storage token, if any (ADR 0028).
    pub subtype: Option<String>,
    /// The cash tier this series rolls into: `spendable` / `reserve` /
    /// `unallocated`.
    pub tier: String,
    /// Per-day projected balance + the events attributed to this series.
    pub days: Vec<ForecastDayDto>,
}

impl From<AccountSeriesView> for AccountSeriesDto {
    fn from(view: AccountSeriesView) -> Self {
        Self {
            account_id: view.account_id.map(|id| id.to_string()),
            name: view.name,
            subtype: view.subtype,
            tier: view.tier,
            days: view.days.into_iter().map(ForecastDayDto::from).collect(),
        }
    }
}

/// A single day's group closing balance (`YYYY-MM-DD` + band).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct DayBalanceDto {
    /// The household-local calendar date, `YYYY-MM-DD`.
    pub date: String,
    /// Projected closing balance band for the group on that day.
    pub closing: BandDto,
}

impl From<DayBalance> for DayBalanceDto {
    fn from(view: DayBalance) -> Self {
        Self {
            date: view.date.to_string(),
            closing: BandDto::from(view.closing),
        }
    }
}

/// A per-tier group series (`spendable` / `reserve` / `unallocated` / `net`): the
/// summed running balance of its member accounts (ADR 0026 §12).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct GroupSeriesDto {
    /// `spendable` / `reserve` / `unallocated` / `net`.
    pub tier: String,
    /// The per-day closing balance for the group.
    pub closings: Vec<DayBalanceDto>,
}

impl From<GroupSeriesView> for GroupSeriesDto {
    fn from(view: GroupSeriesView) -> Self {
        Self {
            tier: view.tier,
            closings: view.closings.into_iter().map(DayBalanceDto::from).collect(),
        }
    }
}

/// The per-account and per-group Future Cash projection (personal-cfo-l8oh): one
/// series per liquid account (+ Unallocated) and the cash-tier rollups, all
/// reconciling to the aggregate's `net`. Backs the multi-series chart + table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct MultiSeriesForecastDto {
    /// ISO-4217 currency code the projection is computed in.
    pub currency: String,
    /// The first projected day (household-local "today"), `YYYY-MM-DD`.
    pub start_date: String,
    /// Number of days projected. Exported as TS `number`.
    pub horizon_days: u32,
    /// One series per liquid account, plus Unallocated when it has flows.
    pub accounts: Vec<AccountSeriesDto>,
    /// The tier rollups: spendable, reserve, (unallocated if any), net.
    pub groups: Vec<GroupSeriesDto>,
}

impl From<MultiSeriesForecast> for MultiSeriesForecastDto {
    fn from(view: MultiSeriesForecast) -> Self {
        Self {
            currency: view.currency.code().to_owned(),
            start_date: view.start_date.to_string(),
            horizon_days: view.horizon_days,
            accounts: view
                .accounts
                .into_iter()
                .map(AccountSeriesDto::from)
                .collect(),
            groups: view.groups.into_iter().map(GroupSeriesDto::from).collect(),
        }
    }
}

/// A single realized historical closing balance (cf-history): `YYYY-MM-DD` + amount.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct HistoryDayDto {
    /// The household-local calendar date, `YYYY-MM-DD`.
    pub date: String,
    /// The account's realized closing balance that day.
    pub closing: MoneyDto,
}

/// One liquid account's realized historical balance series (cf-history, personal-cfo-4d8.27.5.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct AccountHistoryDto {
    /// The spending account id (UUID string).
    pub account_id: String,
    /// Display name.
    pub name: String,
    /// The account's subtype storage token, if any (ADR 0028).
    pub subtype: Option<String>,
    /// The cash tier this series rolls into (`spendable` / `reserve`), or `card` for a
    /// credit card's owed-balance history (stored signed, negative when owed).
    pub tier: String,
    /// The realized closing balance each day, oldest first.
    pub days: Vec<HistoryDayDto>,
}

/// The realized cash-flow HISTORY: one daily-closing series per liquid account, each honest back
/// only as far as its own real data (personal-cfo-4d8.27.5.2). Backs the Account Detail chart's
/// realized line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct CashFlowHistoryDto {
    /// ISO-4217 currency code.
    pub currency: String,
    /// The earliest day any account has data for, `YYYY-MM-DD` (clamped to real data).
    pub start_date: String,
    /// The last realized day (household-local "today"), `YYYY-MM-DD`.
    pub end_date: String,
    /// One realized series per liquid account.
    pub accounts: Vec<AccountHistoryDto>,
}

impl From<CashFlowHistory> for CashFlowHistoryDto {
    fn from(view: CashFlowHistory) -> Self {
        let currency = view.currency.code().to_owned();
        Self {
            currency: currency.clone(),
            start_date: view.start_date.to_string(),
            end_date: view.end_date.to_string(),
            accounts: view
                .accounts
                .into_iter()
                .map(|a| AccountHistoryDto {
                    account_id: a.account_id.to_string(),
                    name: a.name,
                    subtype: a.subtype,
                    tier: a.tier,
                    days: a
                        .days
                        .into_iter()
                        .map(|d| HistoryDayDto {
                            date: d.date.to_string(),
                            closing: MoneyDto {
                                minor_units: d.closing_minor,
                                currency: currency.clone(),
                            },
                        })
                        .collect(),
                })
                .collect(),
        }
    }
}

/// Input to create a manual future entry (personal-cfo-q6gh). The `amount`'s sign
/// is the direction (inflow +, outflow −).
#[derive(Debug, Clone, Deserialize, Type)]
pub struct CreateManualFutureEntryInput {
    /// Signed amount (minor units + currency).
    pub amount: MoneyDto,
    /// The date the entry lands on, `YYYY-MM-DD`.
    pub date: String,
    /// A short user label, e.g. `"Bonus"`.
    pub label: String,
    /// The liquid account the flow comes from / goes to (UUID string), if chosen
    /// (personal-cfo-4d8.24.3). `None`/blank ⇒ Unallocated.
    pub account_id: Option<String>,
}

impl CreateManualFutureEntryInput {
    /// Parse the entry date (`YYYY-MM-DD`).
    pub fn occurs_on(&self) -> Result<NaiveDate, IpcError> {
        parse_iso_date(&self.date)
    }
}

/// Input to edit a manual future entry (personal-cfo-q6gh) — records a replacement
/// that supersedes the prior one.
#[derive(Debug, Clone, Deserialize, Type)]
pub struct UpdateManualFutureEntryInput {
    /// The id (UUID string) of the entry to replace.
    pub id: String,
    pub amount: MoneyDto,
    /// `YYYY-MM-DD`.
    pub date: String,
    pub label: String,
    /// The liquid account the flow is attributed to (UUID string), if chosen
    /// (personal-cfo-4d8.24.3). `None`/blank ⇒ Unallocated.
    pub account_id: Option<String>,
}

impl UpdateManualFutureEntryInput {
    /// Parse the entry date (`YYYY-MM-DD`).
    pub fn occurs_on(&self) -> Result<NaiveDate, IpcError> {
        parse_iso_date(&self.date)
    }
}

/// A stored manual future entry (personal-cfo-q6gh).
#[derive(Debug, Clone, Serialize, Type)]
pub struct ManualFutureEntryDto {
    /// The assumption-event id (UUID string) — pass to update/delete.
    pub id: String,
    pub amount: MoneyDto,
    /// `YYYY-MM-DD`.
    pub date: String,
    pub label: String,
    /// The attributed liquid account (UUID string), if any (personal-cfo-4d8.24.3).
    pub account_id: Option<String>,
    /// The committed transaction the matcher linked this entry to (UUID
    /// string), if any — a matched entry no longer projects (ADR 0026
    /// addendum 2026-09-02, personal-cfo-xtz5).
    pub matched_transaction_id: Option<String>,
}

impl From<ManualEntry> for ManualFutureEntryDto {
    fn from(entry: ManualEntry) -> Self {
        Self {
            id: entry.id.to_string(),
            amount: MoneyDto::from(entry.amount),
            date: entry.occurs_on.to_string(),
            label: entry.label,
            account_id: entry.account_id.map(|id| id.to_string()),
            matched_transaction_id: entry.matched_transaction_id.map(|id| id.to_string()),
        }
    }
}

/// Parse a `YYYY-MM-DD` calendar date, mapping a failure to a validation error.
pub(crate) fn parse_iso_date(raw: &str) -> Result<NaiveDate, IpcError> {
    NaiveDate::parse_from_str(raw.trim(), "%Y-%m-%d")
        .map_err(|_| IpcError::Validation(format!("not a valid date (YYYY-MM-DD): {raw}")))
}

/// Parse an optional UUID string id: `None` or blank → `None`; a non-empty value
/// must be a valid [`Uuid`].
pub fn parse_opt_uuid(raw: Option<&str>) -> Result<Option<Uuid>, IpcError> {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => Uuid::parse_str(s)
            .map(Some)
            .map_err(|_| IpcError::Validation(format!("not a valid id: {s}"))),
        None => Ok(None),
    }
}

/// Require a target entity for a modification/exclusion kind.
fn require_target(target: Option<Uuid>, kind: &str) -> Result<(), IpcError> {
    if target.is_none() {
        return Err(IpcError::Validation(format!(
            "{kind} needs a target_entity_id"
        )));
    }
    Ok(())
}

// ===== Scenarios + forecast assumption events (ADR 0026, personal-cfo-6zep) =====

/// A stored scenario definition — a named overlay on the base forecast
/// (ADR 0026 §5, personal-cfo-0mg/6zep).
#[derive(Debug, Clone, Serialize, Type)]
pub struct ScenarioDto {
    /// The scenario id (UUID string) — pass to update/delete and to the forecast.
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    /// Lifecycle token: `draft` / `active` / `archived`.
    pub status: String,
    pub created_at: String,
    /// When it last changed — the manager's "updated" column.
    pub updated_at: String,
    /// User-set expiry (`YYYY-MM-DD`), or `None` (ADR 0051 §3). Past this date the
    /// scenario is not selectable and is not applied by a run.
    pub expires_on: Option<String>,
    /// Active overlay events, so the manager can show that archiving kept them.
    pub event_count: u32,
    /// When this scenario's events were promoted into base (ADR 0055), or `None`.
    /// A timestamp rather than a status token: applied-ness is orthogonal to the
    /// lifecycle, so an applied scenario can still be archived.
    pub applied_at: Option<String>,
}

impl From<ScenarioView> for ScenarioDto {
    fn from(view: ScenarioView) -> Self {
        Self {
            id: view.id.to_string(),
            name: view.name,
            description: view.description,
            status: view.status.as_token().to_owned(),
            created_at: view.created_at,
            updated_at: view.updated_at,
            expires_on: view.expires_on,
            event_count: view.event_count,
            applied_at: view.applied_at,
        }
    }
}

/// Input to create a scenario (personal-cfo-6zep). Starts in `draft`.
#[derive(Debug, Clone, Deserialize, Type)]
pub struct CreateScenarioInput {
    pub name: String,
    pub description: Option<String>,
}

/// Input to update a scenario's lifecycle status and/or name (personal-cfo-6zep / vru6).
#[derive(Debug, Clone, Deserialize, Type)]
pub struct UpdateScenarioInput {
    /// The scenario id (UUID string).
    pub id: String,
    /// New lifecycle token: `draft` / `active` / `archived`.
    pub status: String,
    /// A new name, if renaming (personal-cfo-vru6); `None` leaves the name unchanged.
    pub name: Option<String>,
}

/// One category's spend over a range, with its subtree total (ADR 0052,
/// personal-cfo-4d8.27.8.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct CategorySpendDto {
    /// The category id (UUID string).
    pub category_id: String,
    pub name: String,
    /// The parent this sits under (UUID string), or null at the requested root level.
    pub parent_id: Option<String>,
    /// Net outflow in minor units, POSITIVE for money spent (refunds subtract).
    /// Includes every descendant — this is the figure the chart draws.
    #[specta(type = Number)]
    pub total_minor: i64,
    /// Spend on this category itself, excluding descendants.
    #[specta(type = Number)]
    pub own_minor: i64,
    /// Whether the cell can be drilled into.
    pub has_children: bool,
}

impl From<finance_kernel::CategorySpend> for CategorySpendDto {
    fn from(row: finance_kernel::CategorySpend) -> Self {
        Self {
            category_id: row.category_id.to_string(),
            name: row.name,
            parent_id: row.parent_id.map(|id| id.to_string()),
            total_minor: row.total_minor,
            own_minor: row.own_minor,
            has_children: row.has_children,
        }
    }
}

/// Input to the spend-by-category read (ADR 0052). The date range is inclusive; `parent`
/// is the level to roll up to (`None` = the taxonomy roots).
///
/// The facets below mirror `TransactionPageInput` because ADR 0052 §2 makes the chart and
/// the list read ONE filter state: a bar the user clicks has to list exactly the rows it
/// counted. There is deliberately **no category facet** — a category selection is the
/// chart's drill level (`parent_id`), not a filter over the aggregate.
#[derive(Debug, Clone, Deserialize, Type)]
pub struct SpendByCategoryInput {
    /// Inclusive start, `YYYY-MM-DD`.
    pub from: String,
    /// Inclusive end, `YYYY-MM-DD`.
    pub to: String,
    /// The parent whose children to return (UUID string); `None` = roots.
    pub parent_id: Option<String>,
    /// Free-text over memo / counterparty / note / account name; `None` = no constraint.
    pub query: Option<String>,
    /// Restrict to these accounts (UUID strings); **empty means no constraint**. Mirrors
    /// `TransactionPageInput::account_ids` exactly — a multi-card Debt selection must
    /// narrow the chart the same way it narrows the list below it (ADR 0057 §2).
    pub account_ids: Vec<String>,
    /// Restrict to transactions carrying one tag (UUID string).
    pub tag_id: Option<String>,
    /// Only unreviewed transactions (ADR 0032 derived-reviewed semantics).
    pub unreviewed_only: bool,
}

/// Input to clone a scenario into a new draft (ADR 0051 §2).
#[derive(Debug, Clone, Deserialize, Type)]
pub struct CloneScenarioInput {
    /// The scenario to copy (UUID string).
    pub id: String,
    /// The new scenario's name.
    pub name: String,
}

/// Input to set or clear a scenario's expiry (ADR 0051 §3).
#[derive(Debug, Clone, Deserialize, Type)]
pub struct SetScenarioExpiryInput {
    /// The scenario id (UUID string).
    pub id: String,
    /// `YYYY-MM-DD`, or `None` to clear the expiry.
    pub expires_on: Option<String>,
}

/// A stored forecast assumption event (ADR 0026 §4, personal-cfo-5u2/6zep). The
/// typed parameters live in `params_json` (its shape varies by `kind`); the
/// frontend parses it per kind when rendering an overlay.
#[derive(Debug, Clone, Serialize, Type)]
pub struct AssumptionEventDto {
    /// The event id (UUID string) — pass to delete (clear).
    pub id: String,
    /// Kind token, e.g. `one_time_event` / `bill_amount` / `exclusion`.
    pub kind: String,
    /// The base income/bill this targets, for a modification or exclusion.
    pub target_entity_id: Option<String>,
    /// The owning scenario, or `None` for a base assumption.
    pub scenario_id: Option<String>,
    /// Event parameters as a JSON string (shape keyed by `kind`).
    pub params_json: String,
    pub created_at: String,
    /// Set when this base event exists because a scenario was APPLIED (ADR 0055), naming
    /// the scenario responsible. `None` for an override the user set directly — the two
    /// need different explanations, so the surface has to be able to tell them apart.
    pub promoted_from_scenario_id: Option<String>,
}

impl From<AssumptionEventView> for AssumptionEventDto {
    fn from(view: AssumptionEventView) -> Self {
        Self {
            id: view.id.to_string(),
            kind: view.kind.as_token().to_owned(),
            target_entity_id: view.target_entity_id.map(|id| id.to_string()),
            scenario_id: view.scenario_id.map(|id| id.to_string()),
            params_json: view.params_json,
            created_at: view.created_at,
            promoted_from_scenario_id: view.promoted_from_scenario_id.map(|id| id.to_string()),
        }
    }
}

/// Input to create a forecast assumption event (personal-cfo-6zep) — the
/// generalized creator. `kind` selects the shape; the relevant optional fields are
/// required for that kind (validated in [`to_spec`](Self::to_spec)).
#[derive(Debug, Clone, Default, Deserialize, Type)]
pub struct CreateForecastAssumptionInput {
    /// Kind token: `one_time_event` / `income_amount` / `bill_amount` /
    /// `income_date` / `bill_date` / `exclusion` / `recurring_debt_payment`.
    pub kind: String,
    /// The owning scenario (UUID string); `None`/blank = a base assumption.
    pub scenario_id: Option<String>,
    /// The base income/bill targeted by a modification or exclusion (UUID string).
    pub target_entity_id: Option<String>,
    /// `one_time_event`: the signed amount (inflow +, outflow −).
    pub amount: Option<MoneyDto>,
    /// `one_time_event`: the date it lands, `YYYY-MM-DD`.
    pub date: Option<String>,
    /// `one_time_event`: a short user label.
    pub label: Option<String>,
    /// `income_amount` / `bill_amount`: the replacement amount, in minor units.
    #[specta(type = Option<Number>)]
    pub new_amount_minor: Option<i64>,
    /// `income_date` / `bill_date`: the new schedule anchor, `YYYY-MM-DD`.
    pub new_anchor_date: Option<String>,
    /// `income_amount` / `bill_amount` / `exclusion`: apply only on/after this date
    /// (`YYYY-MM-DD`); omit for the whole horizon.
    pub effective_date: Option<String>,
    /// `income_amount` / `bill_amount`: stop applying after this date (`YYYY-MM-DD`);
    /// omit for an open-ended override. Multiple windows compose (ADR 0026 §4).
    pub end_date: Option<String>,
}

impl CreateForecastAssumptionInput {
    /// Validate the wire input for its `kind` and lower it to a typed
    /// [`ForecastAssumptionSpec`] with the given id.
    pub fn to_spec(&self, id: Uuid) -> Result<ForecastAssumptionSpec, IpcError> {
        let scenario_id = parse_opt_uuid(self.scenario_id.as_deref())?;
        let target_entity_id = parse_opt_uuid(self.target_entity_id.as_deref())?;
        let effective_date = match self.effective_date.as_deref() {
            Some(d) => Some(parse_iso_date(d)?),
            None => None,
        };
        let end_date = match self.end_date.as_deref() {
            Some(d) => Some(parse_iso_date(d)?),
            None => None,
        };
        let params = match self.kind.as_str() {
            "one_time_event" => AssumptionParams::OneTimeEvent {
                amount: self
                    .amount
                    .as_ref()
                    .ok_or_else(|| {
                        IpcError::Validation("one_time_event needs an amount".to_owned())
                    })?
                    .to_money()?,
                date: parse_iso_date(self.date.as_deref().ok_or_else(|| {
                    IpcError::Validation("one_time_event needs a date".to_owned())
                })?)?,
                label: self.label.clone().unwrap_or_default(),
            },
            "income_amount" | "bill_amount" => {
                require_target(target_entity_id, &self.kind)?;
                let new_amount_minor = self.new_amount_minor.ok_or_else(|| {
                    IpcError::Validation(format!("{} needs new_amount_minor", self.kind))
                })?;
                if self.kind == "income_amount" {
                    AssumptionParams::IncomeAmount {
                        new_amount_minor,
                        effective_date,
                        end_date,
                    }
                } else {
                    AssumptionParams::BillAmount {
                        new_amount_minor,
                        effective_date,
                        end_date,
                    }
                }
            }
            "income_date" | "bill_date" => {
                require_target(target_entity_id, &self.kind)?;
                let new_anchor_date =
                    parse_iso_date(self.new_anchor_date.as_deref().ok_or_else(|| {
                        IpcError::Validation(format!("{} needs new_anchor_date", self.kind))
                    })?)?;
                if self.kind == "income_date" {
                    AssumptionParams::IncomeDate { new_anchor_date }
                } else {
                    AssumptionParams::BillDate { new_anchor_date }
                }
            }
            "exclusion" => {
                require_target(target_entity_id, &self.kind)?;
                AssumptionParams::Exclusion { effective_date }
            }
            "recurring_debt_payment" => {
                // An extra monthly debt payment (personal-cfo-6wk.15): a free-standing recurring
                // outflow. `amount` is the positive payment magnitude (the fold negates it);
                // `new_anchor_date` is the monthly anchor; `end_date` optionally bounds it.
                let amount = self
                    .amount
                    .as_ref()
                    .ok_or_else(|| {
                        IpcError::Validation("recurring_debt_payment needs an amount".to_owned())
                    })?
                    .to_money()?;
                if amount.minor_units() <= 0 {
                    return Err(IpcError::Validation(
                        "recurring_debt_payment amount must be positive".to_owned(),
                    ));
                }
                let anchor_date =
                    parse_iso_date(self.new_anchor_date.as_deref().ok_or_else(|| {
                        IpcError::Validation(
                            "recurring_debt_payment needs new_anchor_date".to_owned(),
                        )
                    })?)?;
                AssumptionParams::RecurringDebtPayment {
                    amount,
                    anchor_date,
                    end_date,
                    label: self.label.clone().unwrap_or_default(),
                }
            }
            "variable_spend_override" => {
                // A planned change to one CATEGORY's discretionary spend
                // (personal-cfo-4d8.27.6.2). `target_entity_id` is the category;
                // `new_amount_minor` carries the SIGNED monthly delta (negative = spend
                // less), reusing the existing field rather than widening the input.
                require_target(target_entity_id, &self.kind)?;
                let delta = self.new_amount_minor.ok_or_else(|| {
                    IpcError::Validation(
                        "variable_spend_override needs new_amount_minor (the signed monthly change)"
                            .to_owned(),
                    )
                })?;
                if delta == 0 {
                    return Err(IpcError::Validation(
                        "a spend change of zero has no effect".to_owned(),
                    ));
                }
                // Bounded so the per-account apportionment (a multiply) cannot overflow
                // and so a slipped keystroke cannot produce a nonsense projection.
                // $10M/month is far beyond any household's discretionary spend.
                const MAX_MONTHLY_DELTA_MINOR: i64 = 1_000_000_000;
                if delta.abs() > MAX_MONTHLY_DELTA_MINOR {
                    return Err(IpcError::Validation(
                        "that monthly spend change is implausibly large".to_owned(),
                    ));
                }
                if let (Some(from), Some(to)) = (effective_date, end_date) {
                    if to < from {
                        return Err(IpcError::Validation(
                            "the end date must be on or after the start date".to_owned(),
                        ));
                    }
                }
                AssumptionParams::VariableSpendOverride {
                    delta_minor_per_month: delta,
                    effective_date,
                    end_date,
                }
            }
            other => {
                return Err(IpcError::Validation(format!(
                    "unsupported assumption kind: {other}"
                )))
            }
        };
        Ok(ForecastAssumptionSpec {
            id,
            scenario_id,
            target_entity_id,
            params,
        })
    }
}

// ===== Balance assertions (ADR 0027, personal-cfo-ueg6) =====

/// Input to assert an account balance directly — the additive set-balance.
#[derive(Debug, Clone, Deserialize, Type)]
pub struct AssertBalanceInput {
    /// The account id (UUID string).
    pub account_id: String,
    /// The asserted balance (must match the account's currency).
    pub amount: MoneyDto,
    /// The "as of" date the balance applies to, `YYYY-MM-DD`.
    pub as_of_date: String,
}

impl AssertBalanceInput {
    /// Parse the target account id.
    pub fn account(&self) -> Result<AccountId, IpcError> {
        parse_account_id(&self.account_id)
    }

    /// Parse the as-of date (`YYYY-MM-DD`).
    pub fn as_of(&self) -> Result<NaiveDate, IpcError> {
        parse_iso_date(&self.as_of_date)
    }
}

/// The result of asserting a balance: the new (assertion-anchored) balance and the
/// still-unexplained adjustment (the "plug"), `None` when fully explained or absent.
#[derive(Debug, Clone, Serialize, Type)]
pub struct AssertBalanceResult {
    pub balance: MoneyDto,
    pub unexplained: Option<MoneyDto>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn money(minor_units: i64) -> MoneyDto {
        MoneyDto {
            minor_units,
            currency: "USD".to_owned(),
        }
    }

    /// personal-cfo-6wk.15: the wire input for the new kind lowers to the typed
    /// `RecurringDebtPayment` spec (amount + anchor + scenario).
    #[test]
    fn recurring_debt_payment_lowers_to_the_typed_spec() {
        let scenario = Uuid::now_v7();
        let input = CreateForecastAssumptionInput {
            kind: "recurring_debt_payment".to_owned(),
            scenario_id: Some(scenario.to_string()),
            amount: Some(money(30_000)),
            new_anchor_date: Some("2026-08-01".to_owned()),
            label: Some("Extra debt payment".to_owned()),
            ..Default::default()
        };
        let spec = input.to_spec(Uuid::now_v7()).unwrap();
        assert_eq!(spec.scenario_id, Some(scenario));
        assert!(matches!(
            spec.params,
            AssumptionParams::RecurringDebtPayment { .. }
        ));
    }

    #[test]
    fn recurring_debt_payment_rejects_non_positive_amount_and_missing_anchor() {
        let no_anchor = CreateForecastAssumptionInput {
            kind: "recurring_debt_payment".to_owned(),
            amount: Some(money(30_000)),
            new_anchor_date: None,
            ..Default::default()
        };
        assert!(no_anchor.to_spec(Uuid::now_v7()).is_err());

        let zero_amount = CreateForecastAssumptionInput {
            kind: "recurring_debt_payment".to_owned(),
            amount: Some(money(0)),
            new_anchor_date: Some("2026-08-01".to_owned()),
            ..Default::default()
        };
        assert!(zero_amount.to_spec(Uuid::now_v7()).is_err());
    }
}

/// One past-due obligation still waiting for the user to say what happened
/// (personal-cfo-4d8.27.7.6, ADR 0058).
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct UnconfirmedOccurrenceDto {
    pub recurring_event_id: String,
    pub name: String,
    pub scheduled_date: String,
    #[specta(type = Number)]
    pub expected_amount_minor: i64,
    pub currency: String,
    #[specta(type = Number)]
    pub days_overdue: i64,
}

impl From<finance_kernel::UnconfirmedOccurrence> for UnconfirmedOccurrenceDto {
    fn from(v: finance_kernel::UnconfirmedOccurrence) -> Self {
        Self {
            recurring_event_id: v.recurring_event_id.to_string(),
            name: v.name,
            scheduled_date: v.scheduled_date,
            expected_amount_minor: v.expected_amount_minor,
            currency: v.currency,
            days_overdue: v.days_overdue,
        }
    }
}

/// The spend chart's rows plus what the same query excluded (personal-cfo-90eg).
///
/// The exclusions travel WITH the rows so the surface can explain the gap between this
/// chart and the list beside it, rather than letting the two silently disagree.
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct SpendBreakdownDto {
    pub rows: Vec<CategorySpendDto>,
    #[specta(type = Number)]
    pub uncategorized_minor: i64,
    #[specta(type = Number)]
    pub transfers_minor: i64,
}

impl From<finance_kernel::SpendBreakdown> for SpendBreakdownDto {
    fn from(v: finance_kernel::SpendBreakdown) -> Self {
        Self {
            rows: v.rows.into_iter().map(CategorySpendDto::from).collect(),
            uncategorized_minor: v.uncategorized_minor,
            transfers_minor: v.transfers_minor,
        }
    }
}

// ---- connectors (personal-cfo-gglk, ADR 0060) ------------------------------

/// Link a new aggregator connection from a user-pasted setup token.
///
/// LEAK RULE: `setup_token` is a one-time secret — the manual `Debug` impl
/// redacts it, mirroring `connector_core::Credential`'s posture.
// NO Serialize: the token must stay one derive away from unserializable
// (pinned by a static assertion in the commands tests).
#[derive(Clone, PartialEq, Eq, Deserialize, Type)]
pub struct ConnectorLinkInput {
    /// Registry id of the adapter (`"simplefin"`).
    pub adapter_id: String,
    pub setup_token: String,
}

impl std::fmt::Debug for ConnectorLinkInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectorLinkInput")
            .field("adapter_id", &self.adapter_id)
            .field("setup_token", &"[redacted]")
            .finish()
    }
}

/// An external account the provider exposes on a connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct ConnectorExternalAccountDto {
    /// Connector-core account key — the stable, connection-scoped id.
    pub external_id: String,
    pub external_name: Option<String>,
}

/// The outcome of linking: the stored connection plus the accounts discovered
/// by the post-claim fetch (empty, with `fetch_error` set, if that best-effort
/// fetch failed — the credential is stored regardless, because the setup token
/// is single-use and must never be wasted).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct ConnectorLinkResultDto {
    pub connection_id: String,
    pub display_hint: Option<String>,
    pub accounts: Vec<ConnectorExternalAccountDto>,
    pub fetch_error: Option<String>,
}

/// One external-account → real-account mapping, with its sync watermark.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct ConnectorAccountLinkDto {
    pub external_id: String,
    pub external_name: Option<String>,
    pub account_id: Option<String>,
    /// `YYYY-MM-DD`; `None` = never synced (next sync fetches full history).
    pub last_synced_on: Option<String>,
}

/// A stored connection for listing — the credential never crosses the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct ConnectorConnectionDto {
    pub id: String,
    pub adapter_id: String,
    pub display_hint: Option<String>,
    pub last_synced_at: Option<String>,
    pub last_error: Option<String>,
    pub links: Vec<ConnectorAccountLinkDto>,
}

/// Map (or unmap, with `None`) an external account onto a real account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ConnectorSetAccountLinkInput {
    pub connection_id: String,
    pub external_id: String,
    pub account_id: Option<String>,
}

/// Run one sync for a connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ConnectorSyncInput {
    pub connection_id: String,
    pub idempotency_key: String,
}

/// A sync's outcome. `status` is one of `synced` / `partially_committed` /
/// `rate_limited` (healthy — retry later, ADR 0060 §5) / `expired` (re-link) /
/// `needs_user_action` / `failed` / `skipped_debounced` / `no_mapped_accounts`
/// / `sync_in_progress` (a concurrent sync holds this connection's slot) /
/// `discovered_accounts` (an unmapped sync fetched the account list instead
/// of walking transactions — map, then sync).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct ConnectorSyncResultDto {
    pub connection_id: String,
    pub status: String,
    pub staged: u32,
    pub committed: u32,
    pub flagged: u32,
    /// Records skipped because their external account has no mapping yet.
    pub skipped_unmapped: u32,
    /// Provider/batch warnings (already wire-sanitized by the adapter).
    pub warnings: Vec<String>,
    /// Human-facing detail for non-`synced` statuses.
    pub message: Option<String>,
}

/// Forget a stored connection (its past synced data stays in the ledger).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ConnectorForgetInput {
    pub connection_id: String,
}
