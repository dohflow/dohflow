//! Debt-payoff strategy comparison: minimum-only vs snowball vs avalanche via
//! the pure payoff simulator (moved verbatim from `forecast.rs`).

use core_money::Currency;
use forecast_engine::payoff::{simulate_payoff, PayoffDebt, PayoffStrategy};
use rusqlite::Connection;
use uuid::Uuid;

use super::card_cycles::{DEFAULT_MIN_FLOOR_MINOR, DEFAULT_MIN_PERCENT_BPS};
use super::events::reporting_currency;
use crate::DbError;

// ===== Debt-payoff strategy comparison (ADR 0036 debt_payoff, personal-cfo-od07) =====

/// One debt-paydown plan's projected outcome (descriptive, ADR 0018).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayoffPlanView {
    /// The strategy token: `minimum_only` / `snowball` / `avalanche`.
    pub strategy: String,
    /// Months until every debt reaches zero, or `None` if not within the horizon (a minimum
    /// below the interest never amortizes).
    pub debt_free_month: Option<u32>,
    /// Total interest paid across the paydown, in minor units.
    pub total_interest_minor: i64,
    /// The household reference currency the amounts are in (personal-cfo-6wk.13).
    pub currency: String,
    /// The total owed balance at the end of each month (personal-cfo-6wk.16): index `0` is the
    /// current total, index `i` the total after month `i` — the burndown-chart series.
    pub monthly_total_owed_minor: Vec<i64>,
    /// Per-debt owed-balance-over-time, one series per debt (personal-cfo-6wk.17): each carries
    /// the account label + its month-indexed balances (aligned with `monthly_total_owed_minor`,
    /// summing to it). Lets the burndown chart break the aggregate down per debt.
    pub per_debt: Vec<PayoffDebtSeries>,
}

/// One debt's owed-balance-over-time within a payoff plan (personal-cfo-6wk.17).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayoffDebtSeries {
    /// The debt's account label (name).
    pub label: String,
    /// The debt's owed balance at the end of each month, minor units (index `0` = current).
    pub monthly_owed_minor: Vec<i64>,
}

/// Every active liability account (card or loan) with `debt_terms` and an owed balance, as a
/// [`PayoffDebt`]. The per-debt minimum is its fixed payment (`pay_fixed_amount`) or the
/// percent/floor rule; the balance is the current owed amount (`-assertion_anchored_balance`).
/// Load the debts a payoff comparison should simulate.
///
/// `account_ids` scopes the set; **empty means every debt** (personal-cfo-4d8.27.9.7).
/// The scope belongs HERE rather than on the output: snowball and avalanche order the
/// debts and route the extra budget among them, so simulating four debts and then hiding
/// two would report a payoff order and a debt-free month that the selected debts do not
/// actually have.
fn read_all_debts_for_payoff(
    conn: &Connection,
    account_ids: &[Uuid],
) -> Result<(Vec<PayoffDebt>, Vec<String>, Currency), DbError> {
    // The comparison sums interest across debts in raw minor units, so it is single-currency: keep
    // only debts in the household reference currency (personal-cfo-6wk.13) — a mixed-currency
    // household compares its base-currency debts, not a meaningless cross-currency sum.
    let reference = reporting_currency(conn)?.unwrap_or(Currency::Usd);
    // One bound placeholder per selected account, matching how the transaction and spend
    // reads scope themselves (personal-cfo-4d8.27.9.4). Binding the ids as VALUES keeps
    // them BLOB-typed like the stored column — a JSON/text scope would compare a string
    // against a BLOB and silently match nothing, which on this surface would quietly
    // report "no debts to pay off".
    let mut sql = String::from(
        "SELECT a.id, a.ledger_account_id, a.name, COALESCE(dt.apr_bps, 0),
                COALESCE(dt.min_payment_percent_bps, 0), COALESCE(dt.min_payment_floor_minor, 0),
                dt.repayment_philosophy, COALESCE(dt.fixed_amount_minor, 0)
         FROM accounts a
         JOIN debt_terms dt ON dt.account_id = a.id
         WHERE a.cashflow_role IN ('credit_facility', 'loan_liability')
           AND a.active = 1
           AND a.currency = ?1",
    );
    if !account_ids.is_empty() {
        let placeholders = vec!["?"; account_ids.len()].join(", ");
        sql.push_str(&format!(" AND a.id IN ({placeholders})"));
    }
    sql.push_str(" ORDER BY a.name COLLATE NOCASE, a.id");
    let mut stmt = conn.prepare(&sql)?;

    let mut binds: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(reference.code().to_owned())];
    for id in account_ids {
        binds.push(Box::new(*id));
    }
    let rows = stmt
        .query_map(rusqlite::params_from_iter(binds.iter()), |r| {
            Ok((
                r.get::<_, Uuid>(0)?,
                r.get::<_, Uuid>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, i64>(7)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut debts = Vec::new();
    let mut labels = Vec::new();
    for (id, ledger, name, apr_bps, min_pct, min_floor, philosophy, fixed) in rows {
        // Full-payment philosophies are paid off each cycle — not revolving carry-debt to pay
        // down, and (having no minimum rule) they would poison the baseline to "never debt-free".
        if matches!(
            philosophy.as_str(),
            "pay_in_full" | "pay_statement_balance" | "pay_current_balance"
        ) {
            continue;
        }
        let owed = crate::assertion_anchored_balance(conn, id, ledger)?
            .saturating_neg()
            .max(0);
        if owed <= 0 {
            continue;
        }
        let fixed_payment_cents = if philosophy == "pay_fixed_amount" {
            fixed
        } else {
            0
        };
        // A carry-debt with no configured minimum falls back to the ADR 0035 §5 default (1%/$25)
        // so the baseline actually amortizes rather than stalling on a $0 minimum.
        let (min_percent_bps, min_floor_cents) =
            if fixed_payment_cents == 0 && min_pct == 0 && min_floor == 0 {
                (DEFAULT_MIN_PERCENT_BPS, DEFAULT_MIN_FLOOR_MINOR)
            } else {
                (min_pct, min_floor)
            };
        debts.push(PayoffDebt {
            balance_cents: owed,
            apr_bps,
            min_percent_bps,
            min_floor_cents,
            fixed_payment_cents,
        });
        labels.push(name);
    }
    Ok((debts, labels, reference))
}

/// Compare debt-paydown strategies for the household's current debts (ADR 0036 debt_payoff,
/// personal-cfo-od07): minimum-only (baseline), snowball, and avalanche — each at
/// `extra_budget_minor` extra per month (the baseline ignores the extra). Returns the debt-free
/// month + total interest per strategy for the Debt sub-view's descriptive compare (ADR 0018).
///
/// # Errors
/// Returns [`DbError`] on a read failure.
pub(crate) fn debt_payoff_comparison(
    conn: &Connection,
    extra_budget_minor: i64,
    account_ids: &[Uuid],
) -> Result<Vec<PayoffPlanView>, DbError> {
    let (debts, labels, currency) = read_all_debts_for_payoff(conn, account_ids)?;
    if debts.is_empty() {
        return Ok(Vec::new());
    }
    let currency_code = currency.code().to_owned();
    let extra = extra_budget_minor.max(0);
    let plans = [
        ("minimum_only", PayoffStrategy::MinimumOnly, 0),
        ("snowball", PayoffStrategy::Snowball, extra),
        ("avalanche", PayoffStrategy::Avalanche, extra),
    ];
    Ok(plans
        .iter()
        .map(|&(strategy, strat, ex)| {
            let result = simulate_payoff(&debts, strat, ex);
            // Transpose the engine's [month][debt] snapshots into one month-indexed series per
            // debt, labelled by account (personal-cfo-6wk.17).
            let months = result.monthly_owed_by_debt.len();
            let per_debt = (0..debts.len())
                .map(|d| PayoffDebtSeries {
                    label: labels[d].clone(),
                    monthly_owed_minor: (0..months)
                        .map(|m| result.monthly_owed_by_debt[m][d])
                        .collect(),
                })
                .collect();
            PayoffPlanView {
                strategy: strategy.to_owned(),
                debt_free_month: result.debt_free_month,
                total_interest_minor: result.total_interest_cents,
                currency: currency_code.clone(),
                monthly_total_owed_minor: result.monthly_total_owed,
                per_debt,
            }
        })
        .collect())
}
