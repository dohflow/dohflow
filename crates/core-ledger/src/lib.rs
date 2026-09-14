//! Pure double-entry ledger primitives for DohFlow (plan §9.4).
//!
//! This crate defines the value types the Finance Kernel operates on —
//! [`Posting`], [`LedgerTransaction`], [`LedgerAccount`], and the identity
//! primitive [`OperationId`] (plan §9.1.2). It is **pure**: types and invariants
//! only, no database access, no async runtime, no Tauri, no I/O.
//!
//! The headline invariant is double-entry balance: a [`LedgerTransaction`] whose
//! postings do not sum to zero (in a single shared currency) is
//! *unconstructable* — [`LedgerTransaction::new`] rejects it, and so does
//! deserialization. Property-tested in `tests/invariants.rs`.
//!
//! Multi-currency events (FX conversion, cross-currency transfers) are modeled
//! as two single-currency transactions linked downstream; a single
//! `LedgerTransaction` is always one currency.

use chrono::{DateTime, Utc};
use core_money::{Currency, Money, MoneyError};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use core_ids::uuid_id;

mod account;
pub use account::{Account, AccountFlags, AccountSubtype, CashTier, CashflowRole, NormalBalance};

// The `uuid_id!` macro and the UUIDv7/display-id machinery live in `core-ids`
// (personal-cfo-i2t) so every schema crate shares one ID foundation. These four
// types are the ledger's identities; the macro gives each `new()` (monotonic
// UUIDv7), `as_uuid`/`as_bytes`, `display_id`, and serde-transparent encoding.
uuid_id! {
    /// Identity of an operation in the operation log (plan §9.1.2). Carried on a
    /// [`LedgerTransaction`] as the provenance back-reference to the operation
    /// that produced it.
    OperationId
}

uuid_id! {
    /// Identity of a ledger transaction.
    TransactionId
}

uuid_id! {
    /// Identity of a ledger account (the double-entry substrate).
    LedgerAccountId
}

uuid_id! {
    /// Identity of a user-visible account (plan §9.3).
    AccountId
}

uuid_id! {
    /// Identity of a recurring income source (plan §9.7, personal-cfo-le79).
    IncomeSourceId
}

uuid_id! {
    /// Identity of a recurring event — a detected or configured recurrence
    /// pattern (the rule/schedule); plan §9.8, personal-cfo-rxw.
    RecurringEventId
}

uuid_id! {
    /// Identity of a materialised occurrence of a recurring event (plan §9.8,
    /// personal-cfo-rxw).
    RecurringEventInstanceId
}

uuid_id! {
    /// Identity of a bill contract — merchant/contract metadata attached to a
    /// recurring obligation (plan §9.8, personal-cfo-rxw).
    BillContractId
}

uuid_id! {
    /// Identity of a commitment — a forecast-facing obligation projected from
    /// recurring events + bill contracts (plan §9.8, personal-cfo-rxw).
    CommitmentId
}

uuid_id! {
    /// Identity of a stored attachment — an encrypted document blob plus its
    /// metadata (ADR 0023, personal-cfo-bcj). Domain entities reference it
    /// through the `attachment_links` join.
    AttachmentId
}

uuid_id! {
    /// Identity of an ingestion source batch — one import / sync event (ADR 0008,
    /// personal-cfo-ihe / 3bb). Importers stage records under a batch and never
    /// write the canonical ledger directly.
    SourceBatchId
}

uuid_id! {
    /// Identity of a staged source record — one parsed row / provider object
    /// within a [`SourceBatchId`] batch (ADR 0008, personal-cfo-3bb).
    SourceRecordId
}

uuid_id! {
    /// Identity of a staged transaction — a proposed transaction awaiting commit
    /// to the ledger (ADR 0008, personal-cfo-cmx). The commit pipeline promotes it
    /// to a [`TransactionId`] + provenance.
    StagedTransactionId
}

uuid_id! {
    /// Identity of a category in the household taxonomy (plan §9.6, ADR 0030,
    /// personal-cfo-d3p/-bac). Hierarchical via a parent `CategoryId`.
    CategoryId
}

uuid_id! {
    /// Identity of a recurring transfer — a scheduled account-to-account money
    /// movement (ADR 0026 §14, personal-cfo-npoe).
    RecurringTransferId
}

uuid_id! {
    /// Identity of a user-defined tag — a many-to-many label on transactions,
    /// orthogonal to the 1:1 category (ADR 0033, personal-cfo-2ryf).
    TagId
}

uuid_id! {
    /// Identity of a split line — one slice of a split transaction's amount, carrying
    /// its own category / note / tags (ADR 0034, personal-cfo-kr9). A side-table
    /// decomposition; the ledger postings are unchanged.
    SplitLineId
}

/// The accounting nature of an account, which fixes its normal balance side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum AccountKind {
    /// Resources owned (cash, bank, receivables).
    Asset,
    /// Obligations owed (credit cards, loans, payables).
    Liability,
    /// Residual ownership (opening balances book here).
    Equity,
    /// Inflows (salary, interest).
    Income,
    /// Outflows (bills, purchases).
    Expense,
}

/// A ledger account primitive: identity, accounting kind, and currency. The
/// full account record (name, metadata, archival) lives in the accounts schema;
/// this is the minimum the ledger itself needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerAccount {
    id: LedgerAccountId,
    kind: AccountKind,
    currency: Currency,
}

impl LedgerAccount {
    /// Create a ledger account.
    #[must_use]
    pub const fn new(id: LedgerAccountId, kind: AccountKind, currency: Currency) -> Self {
        Self { id, kind, currency }
    }

    /// The account's identity.
    #[must_use]
    pub const fn id(self) -> LedgerAccountId {
        self.id
    }

    /// The account's accounting kind.
    #[must_use]
    pub const fn kind(self) -> AccountKind {
        self.kind
    }

    /// The account's currency.
    #[must_use]
    pub const fn currency(self) -> Currency {
        self.currency
    }
}

/// A single movement against one account. The signed [`Money`] amount is a debit
/// (positive) or credit (negative); a transaction's postings sum to zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Posting {
    account: LedgerAccountId,
    amount: Money,
}

impl Posting {
    /// Create a posting against `account` for `amount`.
    #[must_use]
    pub const fn new(account: LedgerAccountId, amount: Money) -> Self {
        Self { account, amount }
    }

    /// The account this posting moves.
    #[must_use]
    pub const fn account(self) -> LedgerAccountId {
        self.account
    }

    /// The signed amount.
    #[must_use]
    pub const fn amount(self) -> Money {
        self.amount
    }
}

/// Why a set of postings could not form a valid [`LedgerTransaction`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum LedgerError {
    /// Fewer than two postings — double entry needs at least two.
    #[error("a ledger transaction needs at least two postings, got {0}")]
    TooFewPostings(usize),
    /// Postings span more than one currency.
    #[error("postings mix currencies: expected {expected}, found {found}")]
    MixedCurrency {
        /// The currency established by the first posting.
        expected: Currency,
        /// The first differing currency encountered.
        found: Currency,
    },
    /// Postings do not sum to zero.
    #[error("postings do not balance to zero; residual {residual}")]
    Unbalanced {
        /// The non-zero residual sum.
        residual: Money,
    },
    /// Arithmetic error while summing postings (e.g. overflow).
    #[error(transparent)]
    Money(#[from] MoneyError),
}

/// A balanced, single-currency double-entry transaction.
///
/// Construct via [`LedgerTransaction::new`], which enforces the invariants;
/// deserialization runs the same validation, so an invalid transaction cannot
/// exist. Fields are private.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "LedgerTransactionRaw")]
pub struct LedgerTransaction {
    id: TransactionId,
    operation_id: OperationId,
    occurred_at: DateTime<Utc>,
    postings: Vec<Posting>,
}

impl LedgerTransaction {
    /// Build a transaction, enforcing double-entry invariants:
    /// at least two postings, a single shared currency, and postings summing to
    /// exactly zero.
    ///
    /// # Errors
    /// Returns [`LedgerError`] if fewer than two postings are given, the
    /// postings mix currencies, summing overflows, or the postings do not
    /// balance to zero.
    pub fn new(
        id: TransactionId,
        operation_id: OperationId,
        occurred_at: DateTime<Utc>,
        postings: Vec<Posting>,
    ) -> Result<Self, LedgerError> {
        if postings.len() < 2 {
            return Err(LedgerError::TooFewPostings(postings.len()));
        }

        let currency = postings[0].amount().currency();
        let mut sum = Money::zero(currency);
        for posting in &postings {
            let amount = posting.amount();
            if amount.currency() != currency {
                return Err(LedgerError::MixedCurrency {
                    expected: currency,
                    found: amount.currency(),
                });
            }
            sum = sum.checked_add(amount)?;
        }

        if !sum.is_zero() {
            return Err(LedgerError::Unbalanced { residual: sum });
        }

        Ok(Self {
            id,
            operation_id,
            occurred_at,
            postings,
        })
    }

    /// The transaction's identity.
    #[must_use]
    pub const fn id(&self) -> TransactionId {
        self.id
    }

    /// The operation that produced this transaction (op-log provenance).
    #[must_use]
    pub const fn operation_id(&self) -> OperationId {
        self.operation_id
    }

    /// When the transaction occurred.
    #[must_use]
    pub const fn occurred_at(&self) -> DateTime<Utc> {
        self.occurred_at
    }

    /// The transaction's postings (always balanced, single-currency).
    #[must_use]
    pub fn postings(&self) -> &[Posting] {
        &self.postings
    }

    /// The transaction's currency (shared by all postings).
    #[must_use]
    pub fn currency(&self) -> Currency {
        // Invariant: non-empty and homogeneous, guaranteed by construction.
        self.postings[0].amount().currency()
    }
}

/// Unvalidated mirror used so that deserialization is forced through
/// [`LedgerTransaction::new`] — the invariant holds on every construction path.
#[derive(Deserialize)]
struct LedgerTransactionRaw {
    id: TransactionId,
    operation_id: OperationId,
    occurred_at: DateTime<Utc>,
    postings: Vec<Posting>,
}

impl TryFrom<LedgerTransactionRaw> for LedgerTransaction {
    type Error = LedgerError;

    fn try_from(raw: LedgerTransactionRaw) -> Result<Self, Self::Error> {
        LedgerTransaction::new(raw.id, raw.operation_id, raw.occurred_at, raw.postings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at_epoch() -> DateTime<Utc> {
        DateTime::from_timestamp(0, 0).expect("epoch is valid")
    }

    fn usd(units: i64) -> Money {
        Money::new(units, Currency::Usd)
    }

    fn balanced_pair(amount: i64) -> Vec<Posting> {
        vec![
            Posting::new(LedgerAccountId::new(), usd(amount)),
            Posting::new(LedgerAccountId::new(), usd(-amount)),
        ]
    }

    #[test]
    fn rejects_fewer_than_two_postings() {
        assert_eq!(
            LedgerTransaction::new(TransactionId::new(), OperationId::new(), at_epoch(), vec![]),
            Err(LedgerError::TooFewPostings(0))
        );
        assert_eq!(
            LedgerTransaction::new(
                TransactionId::new(),
                OperationId::new(),
                at_epoch(),
                vec![Posting::new(LedgerAccountId::new(), usd(0))],
            ),
            Err(LedgerError::TooFewPostings(1))
        );
    }

    #[test]
    fn accepts_balanced_transaction() {
        let tx = LedgerTransaction::new(
            TransactionId::new(),
            OperationId::new(),
            at_epoch(),
            balanced_pair(2_500),
        )
        .expect("balanced");
        assert_eq!(tx.postings().len(), 2);
        assert_eq!(tx.currency(), Currency::Usd);
    }

    #[test]
    fn rejects_unbalanced_transaction() {
        let postings = vec![
            Posting::new(LedgerAccountId::new(), usd(100)),
            Posting::new(LedgerAccountId::new(), usd(-99)),
        ];
        assert_eq!(
            LedgerTransaction::new(
                TransactionId::new(),
                OperationId::new(),
                at_epoch(),
                postings
            ),
            Err(LedgerError::Unbalanced { residual: usd(1) })
        );
    }

    #[test]
    fn rejects_mixed_currency() {
        let postings = vec![
            Posting::new(LedgerAccountId::new(), Money::new(100, Currency::Usd)),
            Posting::new(LedgerAccountId::new(), Money::new(-100, Currency::Eur)),
        ];
        assert_eq!(
            LedgerTransaction::new(
                TransactionId::new(),
                OperationId::new(),
                at_epoch(),
                postings
            ),
            Err(LedgerError::MixedCurrency {
                expected: Currency::Usd,
                found: Currency::Eur,
            })
        );
    }

    #[test]
    fn serde_round_trip_preserves_a_valid_transaction() {
        let tx = LedgerTransaction::new(
            TransactionId::new(),
            OperationId::new(),
            at_epoch(),
            balanced_pair(4_200),
        )
        .expect("balanced");
        let json = serde_json::to_string(&tx).expect("serialize");
        let back: LedgerTransaction = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(tx, back);
    }

    #[test]
    fn deserialization_rejects_an_unbalanced_transaction() {
        let tx = LedgerTransaction::new(
            TransactionId::new(),
            OperationId::new(),
            at_epoch(),
            balanced_pair(1_000),
        )
        .expect("balanced");
        // Tamper with the serialized form so the postings no longer balance.
        let mut value = serde_json::to_value(&tx).expect("to_value");
        value["postings"][0]["amount"]["minor_units"] = serde_json::json!(1_234);
        let result: Result<LedgerTransaction, _> = serde_json::from_value(value);
        assert!(result.is_err(), "invalid transaction must not deserialize");
    }

    #[test]
    fn operation_ids_are_time_ordered() {
        let a = OperationId::new();
        let b = OperationId::new();
        // UUIDv7 is monotonic in generation order at this resolution.
        assert!(a <= b);
    }
}
