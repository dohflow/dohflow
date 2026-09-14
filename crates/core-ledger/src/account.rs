//! User-visible account domain types (plan §9.3).
//!
//! A user-facing [`Account`] is distinct from its double-entry substrate: the
//! [`LedgerAccount`](crate::LedgerAccount) it posts against. This module adds the
//! classification ([`CashflowRole`], [`NormalBalance`]) and the flags the
//! product needs, while the ledger primitives stay in the crate root.

use chrono::{DateTime, Utc};
use core_money::Currency;
use serde::{Deserialize, Serialize};

use crate::{AccountId, AccountKind, LedgerAccountId};

/// The side an account's balance normally sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NormalBalance {
    /// Assets and expenses.
    Debit,
    /// Liabilities, equity, and income.
    Credit,
}

impl NormalBalance {
    /// Lowercase storage token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            NormalBalance::Debit => "debit",
            NormalBalance::Credit => "credit",
        }
    }
}

impl AccountKind {
    /// The normal balance side implied by this accounting kind.
    #[must_use]
    pub const fn normal_balance(self) -> NormalBalance {
        match self {
            AccountKind::Asset | AccountKind::Expense => NormalBalance::Debit,
            _ => NormalBalance::Credit,
        }
    }

    /// Lowercase storage token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            AccountKind::Asset => "asset",
            AccountKind::Liability => "liability",
            AccountKind::Equity => "equity",
            AccountKind::Income => "income",
            AccountKind::Expense => "expense",
        }
    }
}

/// How an account participates in household cashflow (plan §9.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum CashflowRole {
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
    /// In-transit / clearing/suspense.
    ExternalClearing,
    /// Virtual income/expense categorization account.
    IncomeExpenseVirtual,
}

impl CashflowRole {
    /// The double-entry kind the backing ledger account takes.
    #[must_use]
    pub const fn account_kind(self) -> AccountKind {
        match self {
            CashflowRole::LiquidCash
            | CashflowRole::InvestmentAsset
            | CashflowRole::RealAsset
            | CashflowRole::ExternalClearing => AccountKind::Asset,
            CashflowRole::CreditFacility | CashflowRole::LoanLiability => AccountKind::Liability,
            CashflowRole::IncomeExpenseVirtual => AccountKind::Income,
        }
    }

    /// The normal balance side for this role.
    #[must_use]
    pub const fn normal_balance(self) -> NormalBalance {
        self.account_kind().normal_balance()
    }

    /// Lowercase storage token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            CashflowRole::LiquidCash => "liquid_cash",
            CashflowRole::CreditFacility => "credit_facility",
            CashflowRole::LoanLiability => "loan_liability",
            CashflowRole::InvestmentAsset => "investment_asset",
            CashflowRole::RealAsset => "real_asset",
            CashflowRole::ExternalClearing => "external_clearing",
            CashflowRole::IncomeExpenseVirtual => "income_expense_virtual",
        }
    }
}

/// A finer classification within a [`CashflowRole`] (ADR 0028).
///
/// Optional — an account may carry no subtype. Each subtype belongs to exactly
/// one role (see [`AccountSubtype::role`]); the kernel rejects a subtype whose
/// role does not match the account. Only the liquid subtypes drive cash tiers
/// today; the credit / loan / investment subtypes are foundation for later
/// grouping (e.g. `personal-cfo-xuer`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum AccountSubtype {
    /// `LiquidCash` — a checking account.
    Checking,
    /// `LiquidCash` — a savings account.
    Savings,
    /// `LiquidCash` — a money-market account.
    MoneyMarket,
    /// `LiquidCash` — physical cash.
    Cash,
    /// `CreditFacility` — a credit card.
    CreditCard,
    /// `CreditFacility` — a line of credit.
    LineOfCredit,
    /// `LoanLiability` — a mortgage.
    Mortgage,
    /// `LoanLiability` — an auto loan.
    AutoLoan,
    /// `LoanLiability` — a student loan.
    StudentLoan,
    /// `InvestmentAsset` — a taxable brokerage account.
    Brokerage,
    /// `InvestmentAsset` — a retirement account.
    Retirement,
    /// `InvestmentAsset` — a health savings account, often investable (ADR 0028
    /// addendum 2026-07-11). Distinct from the `AccountFlags::tax_advantaged` flag.
    Hsa,
    /// `InvestmentAsset` — a cryptocurrency holding (bitcoin, ethereum, …), tracked
    /// as a manually-valued investment balance (ADR 0028 addendum 2026-07-11).
    Crypto,
    /// `RealAsset` — a home, land, or other real estate (ADR 0044).
    Property,
    /// `RealAsset` — a car, boat, or other vehicle (ADR 0044).
    Vehicle,
    /// `RealAsset` — anything else owned that carries value (ADR 0044).
    OtherRealAsset,
}

impl AccountSubtype {
    /// Every subtype, in declaration order (taxonomy source of truth — the schema
    /// `CHECK` and the IPC layer derive their token sets from this).
    pub const ALL: [AccountSubtype; 16] = [
        AccountSubtype::Checking,
        AccountSubtype::Savings,
        AccountSubtype::MoneyMarket,
        AccountSubtype::Cash,
        AccountSubtype::CreditCard,
        AccountSubtype::LineOfCredit,
        AccountSubtype::Mortgage,
        AccountSubtype::AutoLoan,
        AccountSubtype::StudentLoan,
        AccountSubtype::Brokerage,
        AccountSubtype::Retirement,
        AccountSubtype::Hsa,
        AccountSubtype::Crypto,
        AccountSubtype::Property,
        AccountSubtype::Vehicle,
        AccountSubtype::OtherRealAsset,
    ];

    /// Lowercase storage token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            AccountSubtype::Checking => "checking",
            AccountSubtype::Savings => "savings",
            AccountSubtype::MoneyMarket => "money_market",
            AccountSubtype::Cash => "cash",
            AccountSubtype::CreditCard => "credit_card",
            AccountSubtype::LineOfCredit => "line_of_credit",
            AccountSubtype::Mortgage => "mortgage",
            AccountSubtype::AutoLoan => "auto_loan",
            AccountSubtype::StudentLoan => "student_loan",
            AccountSubtype::Brokerage => "brokerage",
            AccountSubtype::Retirement => "retirement",
            AccountSubtype::Hsa => "hsa",
            AccountSubtype::Crypto => "crypto",
            AccountSubtype::Property => "property",
            AccountSubtype::Vehicle => "vehicle",
            AccountSubtype::OtherRealAsset => "other_real_asset",
        }
    }

    /// Parse a storage token, or `None` if unrecognized.
    #[must_use]
    pub fn from_token(token: &str) -> Option<Self> {
        AccountSubtype::ALL
            .into_iter()
            .find(|subtype| subtype.as_str() == token)
    }

    /// The single [`CashflowRole`] this subtype belongs to.
    #[must_use]
    pub const fn role(self) -> CashflowRole {
        match self {
            AccountSubtype::Checking
            | AccountSubtype::Savings
            | AccountSubtype::MoneyMarket
            | AccountSubtype::Cash => CashflowRole::LiquidCash,
            AccountSubtype::CreditCard | AccountSubtype::LineOfCredit => {
                CashflowRole::CreditFacility
            }
            AccountSubtype::Mortgage | AccountSubtype::AutoLoan | AccountSubtype::StudentLoan => {
                CashflowRole::LoanLiability
            }
            AccountSubtype::Brokerage
            | AccountSubtype::Retirement
            | AccountSubtype::Hsa
            | AccountSubtype::Crypto => CashflowRole::InvestmentAsset,
            AccountSubtype::Property | AccountSubtype::Vehicle | AccountSubtype::OtherRealAsset => {
                CashflowRole::RealAsset
            }
        }
    }

    /// The cash tier a liquid subtype rolls up into (ADR 0028), or `None` for a
    /// non-liquid subtype. Unclassified liquid accounts (no subtype) are bucketed
    /// as [`CashTier::Spendable`] by the rollup, not here.
    #[must_use]
    pub const fn cash_tier(self) -> Option<CashTier> {
        match self {
            AccountSubtype::Checking | AccountSubtype::Cash => Some(CashTier::Spendable),
            AccountSubtype::Savings | AccountSubtype::MoneyMarket => Some(CashTier::Reserve),
            _ => None,
        }
    }
}

/// A liquid-cash rollup tier (ADR 0028): money to spend now vs money set aside.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CashTier {
    /// Spendable now — checking, cash, and unclassified liquid.
    Spendable,
    /// Set aside — savings, money market.
    Reserve,
}

/// Boolean classification flags carried by a user account (plan §9.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountFlags {
    /// A retirement account.
    pub retirement: bool,
    /// Tax-advantaged (HSA/FSA/529/etc.).
    pub tax_advantaged: bool,
    /// Jointly held.
    pub joint: bool,
    /// A business account.
    pub business: bool,
    /// Whether the account is active (false once archived).
    pub active: bool,
}

impl Default for AccountFlags {
    fn default() -> Self {
        Self {
            retirement: false,
            tax_advantaged: false,
            joint: false,
            business: false,
            active: true,
        }
    }
}

/// A user-visible account and the ledger account it posts against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    id: AccountId,
    ledger_account_id: LedgerAccountId,
    name: String,
    cashflow_role: CashflowRole,
    currency: Currency,
    flags: AccountFlags,
    /// Optional finer classification (ADR 0028). `#[serde(default)]` so commands /
    /// oplog entries serialized before this field deserialize as `None`.
    #[serde(default)]
    subtype: Option<AccountSubtype>,
    last_synced: Option<DateTime<Utc>>,
    manual_balance_at: Option<DateTime<Utc>>,
}

impl Account {
    /// Create a user account (active, no timestamps). The ledger account id is
    /// the double-entry substrate this account posts against.
    #[must_use]
    pub fn new(
        id: AccountId,
        ledger_account_id: LedgerAccountId,
        name: impl Into<String>,
        cashflow_role: CashflowRole,
        currency: Currency,
        flags: AccountFlags,
    ) -> Self {
        Self {
            id,
            ledger_account_id,
            name: name.into(),
            cashflow_role,
            currency,
            flags,
            subtype: None,
            last_synced: None,
            manual_balance_at: None,
        }
    }

    /// Set the optional subtype (ADR 0028), returning the account. The caller is
    /// responsible for role↔subtype validation (the kernel does this on create /
    /// update); this is a plain builder.
    #[must_use]
    pub fn with_subtype(mut self, subtype: Option<AccountSubtype>) -> Self {
        self.subtype = subtype;
        self
    }

    /// The user account id.
    #[must_use]
    pub const fn id(&self) -> AccountId {
        self.id
    }

    /// The backing ledger account id.
    #[must_use]
    pub const fn ledger_account_id(&self) -> LedgerAccountId {
        self.ledger_account_id
    }

    /// The display name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The cashflow role.
    #[must_use]
    pub const fn cashflow_role(&self) -> CashflowRole {
        self.cashflow_role
    }

    /// The optional finer subtype (ADR 0028).
    #[must_use]
    pub const fn subtype(&self) -> Option<AccountSubtype> {
        self.subtype
    }

    /// The normal balance side (derived from the role).
    #[must_use]
    pub const fn normal_balance(&self) -> NormalBalance {
        self.cashflow_role.normal_balance()
    }

    /// The currency.
    #[must_use]
    pub const fn currency(&self) -> Currency {
        self.currency
    }

    /// The classification flags.
    #[must_use]
    pub const fn flags(&self) -> AccountFlags {
        self.flags
    }
}

#[cfg(test)]
mod tests {
    use super::{AccountSubtype, CashTier, CashflowRole};

    #[test]
    fn subtype_token_round_trips_for_every_variant() {
        for subtype in AccountSubtype::ALL {
            assert_eq!(
                AccountSubtype::from_token(subtype.as_str()),
                Some(subtype),
                "token round-trip failed for {subtype:?}",
            );
        }
        assert_eq!(AccountSubtype::from_token("not_a_subtype"), None);
    }

    #[test]
    fn every_subtype_maps_to_one_role() {
        assert_eq!(AccountSubtype::Checking.role(), CashflowRole::LiquidCash);
        assert_eq!(AccountSubtype::Cash.role(), CashflowRole::LiquidCash);
        assert_eq!(
            AccountSubtype::CreditCard.role(),
            CashflowRole::CreditFacility,
        );
        assert_eq!(AccountSubtype::Mortgage.role(), CashflowRole::LoanLiability);
        assert_eq!(
            AccountSubtype::Retirement.role(),
            CashflowRole::InvestmentAsset,
        );
        // Investment subtypes added by the ADR 0028 addendum (2026-07-11).
        assert_eq!(AccountSubtype::Hsa.role(), CashflowRole::InvestmentAsset);
        assert_eq!(AccountSubtype::Crypto.role(), CashflowRole::InvestmentAsset);
        assert_eq!(AccountSubtype::Hsa.cash_tier(), None);
        assert_eq!(AccountSubtype::Crypto.cash_tier(), None);
        // Real-asset subtypes (ADR 0044).
        assert_eq!(AccountSubtype::Property.role(), CashflowRole::RealAsset);
        assert_eq!(AccountSubtype::Vehicle.role(), CashflowRole::RealAsset);
        assert_eq!(
            AccountSubtype::OtherRealAsset.role(),
            CashflowRole::RealAsset,
        );
    }

    #[test]
    fn only_liquid_subtypes_carry_a_cash_tier() {
        // Spendable: immediately accessible.
        assert_eq!(
            AccountSubtype::Checking.cash_tier(),
            Some(CashTier::Spendable)
        );
        assert_eq!(AccountSubtype::Cash.cash_tier(), Some(CashTier::Spendable));
        // Reserve: set aside.
        assert_eq!(AccountSubtype::Savings.cash_tier(), Some(CashTier::Reserve));
        assert_eq!(
            AccountSubtype::MoneyMarket.cash_tier(),
            Some(CashTier::Reserve),
        );
        // Non-liquid subtypes never roll into a cash tier.
        for subtype in AccountSubtype::ALL {
            if subtype.role() == CashflowRole::LiquidCash {
                assert!(subtype.cash_tier().is_some());
            } else {
                assert_eq!(subtype.cash_tier(), None, "{subtype:?} should have no tier");
            }
        }
    }
}
