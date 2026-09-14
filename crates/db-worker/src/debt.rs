//! Per-account debt terms (ADR 0035 §5, personal-cfo-6wk.6) — the shared attributes both
//! cards and loans carry, plus the `SetDebtTerms` apply + read seam. The paying-source and
//! liability-target rules are validated here (not a column CHECK), since they read another
//! account's `cashflow_role`.

use chrono::Utc;
use core_ledger::AccountId;
use rusqlite::{params, Connection, OptionalExtension};

use crate::DbError;

/// How a liability is repaid (ADR 0035 §1) — one model for cards and loans. The forecast maps
/// each token to a projected payment amount; `Unknown` defaults to the contractual minimum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepaymentPhilosophy {
    /// Pay the full statement balance (revolving) / owed balance (installment).
    PayInFull,
    /// Pay the projected statement balance specifically.
    PayStatementBalance,
    /// Pay the current owed balance (incl. post-statement activity).
    PayCurrentBalance,
    /// Pay the computed minimum.
    PayMinimum,
    /// Pay a stored fixed amount (a loan's scheduled payment, or a user figure).
    PayFixedAmount,
    /// Not set — the forecast assumes the minimum (ADR 0035 §1).
    Unknown,
}

impl RepaymentPhilosophy {
    /// The stored token (matches the `debt_terms` CHECK set).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PayInFull => "pay_in_full",
            Self::PayStatementBalance => "pay_statement_balance",
            Self::PayCurrentBalance => "pay_current_balance",
            Self::PayMinimum => "pay_minimum",
            Self::PayFixedAmount => "pay_fixed_amount",
            Self::Unknown => "unknown",
        }
    }

    /// Parse a stored token.
    ///
    /// # Errors
    /// Returns [`DbError::InvalidCommand`] for an unknown token.
    pub fn from_token(token: &str) -> Result<Self, DbError> {
        Ok(match token {
            "pay_in_full" => Self::PayInFull,
            "pay_statement_balance" => Self::PayStatementBalance,
            "pay_current_balance" => Self::PayCurrentBalance,
            "pay_minimum" => Self::PayMinimum,
            "pay_fixed_amount" => Self::PayFixedAmount,
            "unknown" => Self::Unknown,
            other => {
                return Err(DbError::InvalidCommand(format!(
                    "unknown repayment_philosophy '{other}'"
                )))
            }
        })
    }
}

/// The settable per-account debt attributes (ADR 0035 §5). `None` fields clear/leave unset;
/// `SetDebtTerms` is an upsert that replaces the row.
#[derive(Debug, Clone, Copy)]
pub struct DebtTermsInput {
    /// Annual percentage rate, in basis points.
    pub apr_bps: Option<i64>,
    /// Statement close day-of-month (1–31; month-end clamp at projection time).
    pub statement_close_day: Option<i64>,
    /// Payment due day-of-month (1–31).
    pub payment_due_day: Option<i64>,
    /// Grace-period length in days.
    pub grace_period_days: Option<i64>,
    /// Credit limit (cards / lines), in minor units.
    pub credit_limit_minor: Option<i64>,
    /// How the liability is repaid.
    pub repayment_philosophy: RepaymentPhilosophy,
    /// The fixed payment for `PayFixedAmount`, in minor units.
    pub fixed_amount_minor: Option<i64>,
    /// Minimum-payment percent-of-balance, in basis points.
    pub min_payment_percent_bps: Option<i64>,
    /// Minimum-payment floor, in minor units.
    pub min_payment_floor_minor: Option<i64>,
    /// The liquid account that pays this debt (ADR 0035 §2). Validated liquid here.
    pub paying_source_account_id: Option<AccountId>,
    /// The loan's original principal, in minor units (ADR 0044) — descriptive; does
    /// not affect the forecast (which projects from the current owed balance).
    pub original_principal_minor: Option<i64>,
}

/// The read view of an account's debt terms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebtTermsView {
    /// The liability account these terms belong to.
    pub account_id: AccountId,
    /// APR in basis points.
    pub apr_bps: Option<i64>,
    /// Statement close day-of-month.
    pub statement_close_day: Option<i64>,
    /// Payment due day-of-month.
    pub payment_due_day: Option<i64>,
    /// Grace-period days.
    pub grace_period_days: Option<i64>,
    /// Credit limit in minor units.
    pub credit_limit_minor: Option<i64>,
    /// Repayment philosophy.
    pub repayment_philosophy: RepaymentPhilosophy,
    /// Fixed payment amount in minor units.
    pub fixed_amount_minor: Option<i64>,
    /// Minimum-payment percent in basis points.
    pub min_payment_percent_bps: Option<i64>,
    /// Minimum-payment floor in minor units.
    pub min_payment_floor_minor: Option<i64>,
    /// The paying liquid account.
    pub paying_source_account_id: Option<AccountId>,
    /// The loan's original principal in minor units (ADR 0044).
    pub original_principal_minor: Option<i64>,
}

/// The cashflow role of `account_id`, or `None` if it does not exist.
fn account_role(tx: &Connection, account_id: AccountId) -> Result<Option<String>, DbError> {
    Ok(tx
        .query_row(
            "SELECT cashflow_role FROM accounts WHERE id = ?1",
            [account_id.as_uuid()],
            |r| r.get::<_, String>(0),
        )
        .optional()?)
}

/// Validate a day-of-month field is in `1..=31`.
fn check_day(day: Option<i64>, what: &str) -> Result<(), DbError> {
    if let Some(d) = day {
        if !(1..=31).contains(&d) {
            return Err(DbError::InvalidCommand(format!(
                "{what} must be between 1 and 31"
            )));
        }
    }
    Ok(())
}

/// Validate an optional value falls in `lo..=hi`. Used to keep non-negative / bounded debt
/// attributes (APR, money amounts, the minimum-payment percent) from feeding garbage into the
/// forecast's payment + finance-charge math (ADR 0035 §4/§5).
fn check_range(value: Option<i64>, lo: i64, hi: i64, what: &str) -> Result<(), DbError> {
    if let Some(v) = value {
        if v < lo || v > hi {
            return Err(DbError::InvalidCommand(format!(
                "{what} must be between {lo} and {hi}"
            )));
        }
    }
    Ok(())
}

/// Apply `SetDebtTerms` (ADR 0035 §5): the target must be a liability account, and any
/// `paying_source_account_id` must be a liquid-cash account. Upserts the row.
///
/// # Errors
/// Returns [`DbError::InvalidCommand`] if the target is missing / not a liability, the paying
/// source is missing / not liquid, or a day field is out of range; [`DbError`] on a write fail.
pub(crate) fn apply_set_debt_terms(
    tx: &Connection,
    account_id: AccountId,
    input: DebtTermsInput,
) -> Result<(), DbError> {
    let Some(role) = account_role(tx, account_id)? else {
        return Err(DbError::InvalidCommand("account does not exist".to_owned()));
    };
    if role != "credit_facility" && role != "loan_liability" {
        return Err(DbError::InvalidCommand(
            "debt terms can only be set on a liability account".to_owned(),
        ));
    }
    if let Some(source) = input.paying_source_account_id {
        match account_role(tx, source)? {
            None => {
                return Err(DbError::InvalidCommand(
                    "paying source account does not exist".to_owned(),
                ))
            }
            Some(source_role) if source_role != "liquid_cash" => {
                return Err(DbError::InvalidCommand(
                    "paying source must be a liquid-cash account".to_owned(),
                ))
            }
            Some(_) => {}
        }
    }
    check_day(input.statement_close_day, "statement_close_day")?;
    check_day(input.payment_due_day, "payment_due_day")?;
    // Domain bounds: these are non-negative quantities; a percent is 0–100% (bps), grace is a
    // sane number of days. A negative APR or an over-100% minimum would corrupt the forecast.
    check_range(input.apr_bps, 0, i64::MAX, "apr_bps")?;
    check_range(input.grace_period_days, 0, 366, "grace_period_days")?;
    check_range(input.credit_limit_minor, 0, i64::MAX, "credit_limit_minor")?;
    check_range(input.fixed_amount_minor, 0, i64::MAX, "fixed_amount_minor")?;
    check_range(
        input.min_payment_percent_bps,
        0,
        10_000,
        "min_payment_percent_bps",
    )?;
    check_range(
        input.min_payment_floor_minor,
        0,
        i64::MAX,
        "min_payment_floor_minor",
    )?;

    tx.execute(
        "INSERT OR REPLACE INTO debt_terms
            (account_id, apr_bps, statement_close_day, payment_due_day, grace_period_days,
             credit_limit_minor, repayment_philosophy, fixed_amount_minor,
             min_payment_percent_bps, min_payment_floor_minor, paying_source_account_id,
             original_principal_minor, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            account_id.as_uuid(),
            input.apr_bps,
            input.statement_close_day,
            input.payment_due_day,
            input.grace_period_days,
            input.credit_limit_minor,
            input.repayment_philosophy.as_str(),
            input.fixed_amount_minor,
            input.min_payment_percent_bps,
            input.min_payment_floor_minor,
            input.paying_source_account_id.map(|a| a.as_uuid()),
            input.original_principal_minor,
            Utc::now().to_rfc3339(),
        ],
    )?;
    Ok(())
}

/// Apply `SetCardStatementBalance` (feedback 2026-07-03): record — or clear — the REAL
/// statement balance for one of a card's cycles. The target must be a credit-card
/// account; the amount must be non-negative. The forecast prefers this over its own
/// estimate for that cycle and carries the owed balance forward from it.
///
/// # Errors
/// Returns [`DbError::InvalidCommand`] if the target is missing / not a credit card or the
/// amount is negative; [`DbError`] on a write fail.
pub(crate) fn apply_set_card_statement_balance(
    tx: &Connection,
    account_id: AccountId,
    cycle_close: chrono::NaiveDate,
    statement_balance_minor: Option<i64>,
) -> Result<(), DbError> {
    let Some(role) = account_role(tx, account_id)? else {
        return Err(DbError::InvalidCommand("account does not exist".to_owned()));
    };
    if role != "credit_facility" {
        return Err(DbError::InvalidCommand(
            "a statement balance can only be recorded on a credit-card account".to_owned(),
        ));
    }
    let Some(balance) = statement_balance_minor else {
        tx.execute(
            "DELETE FROM credit_card_statements WHERE account_id = ?1 AND cycle_close = ?2",
            params![account_id.as_uuid(), cycle_close.to_string()],
        )?;
        return Ok(());
    };
    if balance < 0 {
        return Err(DbError::InvalidCommand(
            "a statement balance cannot be negative".to_owned(),
        ));
    }
    // An actual statement can only exist for a cycle that has CLOSED (ADR 0039 addendum
    // 2026-07-10 §1): recording against a future close would key the row to a shifting
    // derived date and silently replay into an upcoming cycle (personal-cfo-4d8.25.2).
    // Clearing (the `None` branch above) is always allowed. Replay-safe: a write accepted
    // when its close was past stays past on any later op-log replay.
    let today = Utc::now()
        .with_timezone(&crate::forecast::read_household_tz(tx)?)
        .date_naive();
    if cycle_close > today {
        return Err(DbError::InvalidCommand(
            "a statement balance can only be recorded for a cycle that has already closed"
                .to_owned(),
        ));
    }
    let currency: String = tx.query_row(
        "SELECT currency FROM accounts WHERE id = ?1",
        [account_id.as_uuid()],
        |r| r.get(0),
    )?;
    tx.execute(
        "INSERT INTO credit_card_statements
            (id, account_id, cycle_close, statement_balance_minor, minimum_due_minor,
             payment_due, currency, created_at)
         VALUES (?1, ?2, ?3, ?4, NULL, '', ?5, ?6)
         ON CONFLICT (account_id, cycle_close)
         DO UPDATE SET statement_balance_minor = excluded.statement_balance_minor",
        params![
            uuid::Uuid::now_v7(),
            account_id.as_uuid(),
            cycle_close.to_string(),
            balance,
            currency,
            Utc::now().to_rfc3339(),
        ],
    )?;
    Ok(())
}

/// The user-recorded statement balances for a card, keyed by cycle-close date
/// (feedback 2026-07-03). Empty when the user never asserted one.
pub(crate) fn read_card_statement_balances(
    conn: &Connection,
    account_id: uuid::Uuid,
) -> Result<std::collections::HashMap<chrono::NaiveDate, i64>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT cycle_close, statement_balance_minor FROM credit_card_statements
          WHERE account_id = ?1",
    )?;
    let rows = stmt.query_map([account_id], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    })?;
    let mut map = std::collections::HashMap::new();
    for row in rows {
        let (close, balance) = row?;
        let date = close.parse::<chrono::NaiveDate>().map_err(|_| {
            DbError::InvalidCommand(format!("bad cycle_close date in store: {close}"))
        })?;
        map.insert(date, balance);
    }
    Ok(map)
}

/// The stored term columns, in the order [`terms_from_row`] expects them.
///
/// Shared by the single-account and list reads so the two cannot drift into disagreeing
/// about what a debt's terms are — the failure mode a second hand-written 11-field mapping
/// invites.
const TERMS_COLUMNS: &str = "apr_bps, statement_close_day, payment_due_day, grace_period_days,
     credit_limit_minor, repayment_philosophy, fixed_amount_minor,
     min_payment_percent_bps, min_payment_floor_minor, paying_source_account_id,
     original_principal_minor";

/// Map one [`TERMS_COLUMNS`] row (indices 0..=10) to a view.
fn terms_from_row(account_id: AccountId, r: &rusqlite::Row<'_>) -> Result<DebtTermsView, DbError> {
    let philosophy: String = r.get(5)?;
    Ok(DebtTermsView {
        account_id,
        apr_bps: r.get(0)?,
        statement_close_day: r.get(1)?,
        payment_due_day: r.get(2)?,
        grace_period_days: r.get(3)?,
        credit_limit_minor: r.get(4)?,
        repayment_philosophy: RepaymentPhilosophy::from_token(&philosophy)?,
        fixed_amount_minor: r.get(6)?,
        min_payment_percent_bps: r.get(7)?,
        min_payment_floor_minor: r.get(8)?,
        paying_source_account_id: r.get::<_, Option<uuid::Uuid>>(9)?.map(AccountId::from_uuid),
        original_principal_minor: r.get(10)?,
    })
}

/// Read an account's debt terms, if set.
///
/// # Errors
/// Returns [`DbError`] if the read fails or a stored token is invalid.
pub(crate) fn read_debt_terms(
    conn: &Connection,
    account_id: AccountId,
) -> Result<Option<DebtTermsView>, DbError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {TERMS_COLUMNS} FROM debt_terms WHERE account_id = ?1"
    ))?;
    let mut rows = stmt.query([account_id.as_uuid()])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    Ok(Some(terms_from_row(account_id, row)?))
}

/// Read debt terms for every account that has them, optionally scoped to an account set
/// (personal-cfo-4d17).
///
/// The Debt page shows APR, statement dates and minimum rules for every debt in scope; one
/// round-trip per debt would mean N loading states to reconcile against each other, which is
/// how a page ends up rendering a half-loaded total as if it were final.
///
/// **Accounts without terms are omitted, not returned empty.** A debt with no APR on record
/// is not a debt at 0% — the caller has to be able to tell "no rate recorded" from "no
/// interest", and a row of nulls would erase that distinction.
///
/// An empty `account_ids` means every debt account, matching how the transaction and spend
/// reads scope themselves (personal-cfo-4d8.27.9.4).
///
/// # Errors
/// Returns [`DbError`] if the read fails or a stored token is invalid.
pub(crate) fn read_debt_terms_list(
    conn: &Connection,
    account_ids: &[uuid::Uuid],
) -> Result<Vec<DebtTermsView>, DbError> {
    // `account_id` trails the shared columns so `terms_from_row`'s indices stay valid for
    // both reads.
    let mut sql = format!("SELECT {TERMS_COLUMNS}, account_id FROM debt_terms");
    if !account_ids.is_empty() {
        let placeholders = vec!["?"; account_ids.len()].join(", ");
        sql.push_str(&format!(" WHERE account_id IN ({placeholders})"));
    }
    sql.push_str(" ORDER BY account_id");

    let mut stmt = conn.prepare(&sql)?;
    let binds: Vec<&dyn rusqlite::ToSql> = account_ids
        .iter()
        .map(|id| id as &dyn rusqlite::ToSql)
        .collect();
    let mut rows = stmt.query(rusqlite::params_from_iter(binds))?;

    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        let account_id = AccountId::from_uuid(row.get::<_, uuid::Uuid>(11)?);
        out.push(terms_from_row(account_id, row)?);
    }
    Ok(out)
}
