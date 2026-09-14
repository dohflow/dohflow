//! Debt-payoff strategy projections (ADR 0036 `debt_payoff`, personal-cfo-od07).
//!
//! Pure + deterministic, like the rest of the crate: integer-cent math, no `f64` / clock / IO.
//! Given a household's debts (balance, APR, minimum rule) and a monthly **extra** budget, a
//! strategy projects the month-by-month paydown — interest accrues, every debt's current minimum
//! is paid, and the committed budget's surplus (the extra plus the minimums freed as debts shrink
//! or clear — the "snowball" roll-over) is applied to a target debt chosen by the strategy
//! (snowball → smallest balance, avalanche → highest APR). It reports the debt-free month + total
//! interest so the Debt sub-view can compare strategies descriptively (ADR 0018 — never
//! "best"/"recommended").

use std::cmp::Reverse;

use crate::revolving::minimum_due;

/// How far to simulate before giving up (a minimum below the monthly interest never amortizes).
const HORIZON_CAP_MONTHS: u32 = 600;

/// One debt to pay down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PayoffDebt {
    /// Current owed balance in minor units.
    pub balance_cents: i64,
    /// Annual percentage rate in basis points.
    pub apr_bps: i64,
    /// Minimum-payment percent-of-balance in basis points.
    pub min_percent_bps: i64,
    /// Minimum-payment floor in minor units.
    pub min_floor_cents: i64,
    /// A fixed required payment (a loan's amortization amount); `0` uses the percent/floor rule.
    pub fixed_payment_cents: i64,
}

/// The paydown strategy — the crate-neutral order in which the surplus budget is applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayoffStrategy {
    /// Pay only each debt's minimum — no extra, and freed minimums are **not** rolled over.
    MinimumOnly,
    /// Roll the extra + freed minimums onto the **smallest balance** first (debt snowball).
    Snowball,
    /// Roll the extra + freed minimums onto the **highest APR** first (debt avalanche).
    Avalanche,
}

/// The projected outcome of a strategy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayoffResult {
    /// Months until every debt reaches zero, or `None` if not within the horizon cap (a
    /// minimum below the interest never amortizes).
    pub debt_free_month: Option<u32>,
    /// Total interest paid across the paydown, in minor units.
    pub total_interest_cents: i64,
    /// The total owed balance at the end of each month (personal-cfo-6wk.16): index `0` is the
    /// starting total, index `i` the total after month `i`. Ends at `0` on the debt-free month
    /// (length = `debt_free_month + 1`), else runs to the horizon cap. Feeds the burndown chart.
    pub monthly_total_owed: Vec<i64>,
    /// Per-debt owed balances at each month (personal-cfo-6wk.17): `monthly_owed_by_debt[m][d]` is
    /// debt `d`'s balance after month `m`, in the input order (same length + month-indexing as
    /// `monthly_total_owed`; each row sums to it). Lets the burndown chart break down per debt.
    pub monthly_owed_by_debt: Vec<Vec<i64>>,
}

/// The current monthly minimum for a debt at `balance` (its fixed payment, or the percent/floor
/// rule), never more than the balance.
fn required_minimum(debt: &PayoffDebt, balance: i64) -> i64 {
    if balance <= 0 {
        return 0;
    }
    let m = if debt.fixed_payment_cents > 0 {
        debt.fixed_payment_cents
    } else {
        minimum_due(balance, debt.min_percent_bps, debt.min_floor_cents)
    };
    // A percent-only minimum rounds to 0 once the balance is a few cents — clear that residual
    // so a paying-down debt actually reaches zero (else MinimumOnly reports "never debt-free").
    if m == 0 && debt.min_percent_bps > 0 {
        return balance;
    }
    m.min(balance)
}

/// One month's interest on `balance` at `apr_bps` (monthly = apr / 12), integer cents.
fn monthly_interest(balance: i64, apr_bps: i64) -> i64 {
    if balance <= 0 || apr_bps <= 0 {
        return 0;
    }
    (i128::from(balance) * i128::from(apr_bps) / (10_000 * 12)) as i64
}

/// The index of the debt the strategy targets with the surplus — the smallest balance
/// (`Snowball`) or the highest APR (`Avalanche`) among active debts, ties broken by lowest
/// index for determinism. `None` for `MinimumOnly` or when no debt is active.
fn target_debt(balances: &[i64], debts: &[PayoffDebt], strategy: PayoffStrategy) -> Option<usize> {
    let active = (0..balances.len()).filter(|&i| balances[i] > 0);
    match strategy {
        PayoffStrategy::MinimumOnly => None,
        PayoffStrategy::Snowball => active.min_by_key(|&i| (balances[i], i)),
        PayoffStrategy::Avalanche => active.min_by_key(|&i| (Reverse(debts[i].apr_bps), i)),
    }
}

/// Simulate paying down `debts` under `strategy` with `extra_budget_cents` of extra each month.
///
/// For a strategy (not `MinimumOnly`) every debt's **current** minimum is funded first (these are
/// contractual and are never starved), then the committed budget's surplus — `sum(initial
/// minimums) + extra` less the minimums actually paid this month — cascades onto the target debt,
/// so a cleared or shrunk debt's minimum keeps working (the snowball). Interest accrues first.
///
/// Note: with percent-of-balance minimums avalanche is only *generally* lower-interest than
/// snowball — the shrinking minimums make the freed surplus depend on which debt is targeted — so
/// this crate never claims one strategy dominates; the UI compares them descriptively.
#[must_use]
pub fn simulate_payoff(
    debts: &[PayoffDebt],
    strategy: PayoffStrategy,
    extra_budget_cents: i64,
) -> PayoffResult {
    let mut balances: Vec<i64> = debts.iter().map(|d| d.balance_cents.max(0)).collect();
    let mut total_interest: i64 = 0;
    let total_owed = |bals: &[i64]| bals.iter().fold(0i64, |a, &b| a.saturating_add(b.max(0)));
    let clamp = |bals: &[i64]| bals.iter().map(|&b| b.max(0)).collect::<Vec<i64>>();
    // index 0 is the starting total owed; one entry is pushed per simulated month.
    let mut monthly_total_owed: Vec<i64> = vec![total_owed(&balances)];
    let mut monthly_owed_by_debt: Vec<Vec<i64>> = vec![clamp(&balances)];

    // The committed monthly outlay for a rolling strategy: initial minimums + the extra.
    let initial_min_sum: i64 = (0..debts.len())
        .map(|i| required_minimum(&debts[i], balances[i]))
        .fold(0i64, i64::saturating_add);
    let constant_budget = initial_min_sum.saturating_add(extra_budget_cents.max(0));

    let mut month = 0u32;
    while balances.iter().any(|&b| b > 0) && month < HORIZON_CAP_MONTHS {
        month += 1;
        // 1. Accrue interest on every active debt.
        for i in 0..balances.len() {
            let interest = monthly_interest(balances[i], debts[i].apr_bps);
            balances[i] = balances[i].saturating_add(interest);
            total_interest = total_interest.saturating_add(interest);
        }
        // 2. Allocate payments.
        if strategy == PayoffStrategy::MinimumOnly {
            for i in 0..balances.len() {
                balances[i] -= required_minimum(&debts[i], balances[i]);
            }
        } else {
            // Fund every active debt's CURRENT minimum first — contractual, never starved by the
            // frozen budget (a growing balance's percent-minimum can exceed its initial value).
            let mut paid_min = 0i64;
            for i in 0..balances.len() {
                let pay = required_minimum(&debts[i], balances[i]);
                balances[i] -= pay;
                paid_min = paid_min.saturating_add(pay);
            }
            // The committed budget's surplus (extra + freed minimums) cascades onto the target.
            let mut surplus = constant_budget.saturating_sub(paid_min).max(0);
            while surplus > 0 {
                let Some(t) = target_debt(&balances, debts, strategy) else {
                    break;
                };
                let pay = balances[t].min(surplus);
                if pay == 0 {
                    break;
                }
                balances[t] -= pay;
                surplus -= pay;
            }
        }
        monthly_total_owed.push(total_owed(&balances));
        monthly_owed_by_debt.push(clamp(&balances));
    }

    PayoffResult {
        debt_free_month: balances.iter().all(|&b| b <= 0).then_some(month),
        total_interest_cents: total_interest,
        monthly_total_owed,
        monthly_owed_by_debt,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn debt(balance: i64, apr_bps: i64, fixed: i64) -> PayoffDebt {
        PayoffDebt {
            balance_cents: balance,
            apr_bps,
            min_percent_bps: 200, // 2%
            min_floor_cents: 2_500,
            fixed_payment_cents: fixed,
        }
    }

    /// A hand table: $1,000 at 0% APR paying a fixed $100/month clears in exactly 10 months
    /// with no interest.
    #[test]
    fn zero_apr_fixed_payment_matches_a_hand_table() {
        let r = simulate_payoff(&[debt(100_000, 0, 10_000)], PayoffStrategy::MinimumOnly, 0);
        assert_eq!(r.debt_free_month, Some(10));
        assert_eq!(r.total_interest_cents, 0);
    }

    /// The burndown trajectory (personal-cfo-6wk.16) starts at the total owed, steps down each
    /// month, and ends at exactly 0 on the debt-free month.
    #[test]
    fn trajectory_walks_from_total_owed_down_to_zero() {
        let r = simulate_payoff(&[debt(100_000, 0, 10_000)], PayoffStrategy::MinimumOnly, 0);
        assert_eq!(
            r.monthly_total_owed,
            vec![
                100_000, 90_000, 80_000, 70_000, 60_000, 50_000, 40_000, 30_000, 20_000, 10_000, 0
            ]
        );
        // length = debt_free_month + 1, first = starting total, last = 0.
        assert_eq!(
            r.monthly_total_owed.len(),
            r.debt_free_month.unwrap() as usize + 1
        );
        assert_eq!(r.monthly_total_owed[0], 100_000);
        assert_eq!(*r.monthly_total_owed.last().unwrap(), 0);
        // A never-clearing plan still records a bounded, non-empty trajectory.
        let stuck = PayoffDebt {
            balance_cents: 1_000_000,
            apr_bps: 3_600,
            min_percent_bps: 100,
            min_floor_cents: 0,
            fixed_payment_cents: 0,
        };
        let s = simulate_payoff(&[stuck], PayoffStrategy::MinimumOnly, 0);
        assert_eq!(s.debt_free_month, None);
        assert_eq!(s.monthly_total_owed[0], 1_000_000);
        assert!(*s.monthly_total_owed.last().unwrap() > 0);
    }

    /// The per-debt trajectory (personal-cfo-6wk.17) has one entry per debt each month, aligned
    /// with the aggregate trajectory, and every month's per-debt balances sum to the aggregate.
    #[test]
    fn per_debt_balances_sum_to_the_aggregate_each_month() {
        let debts = [debt(50_000, 500, 5_000), debt(100_000, 2_400, 5_000)];
        let r = simulate_payoff(&debts, PayoffStrategy::Avalanche, 10_000);
        assert_eq!(r.monthly_owed_by_debt.len(), r.monthly_total_owed.len());
        assert_eq!(
            r.monthly_owed_by_debt[0],
            vec![50_000, 100_000],
            "starts at the balances"
        );
        for (m, row) in r.monthly_owed_by_debt.iter().enumerate() {
            assert_eq!(row.len(), 2, "one entry per debt");
            assert_eq!(
                row.iter().sum::<i64>(),
                r.monthly_total_owed[m],
                "per-debt sums to the aggregate at month {m}",
            );
        }
    }

    /// With FIXED payments the surplus rolled onto the target is identical regardless of which
    /// debt it targets, so avalanche (highest APR first) never pays more interest than snowball.
    /// (Under percent-of-balance minimums this holds only *generally* — the shrinking minimums
    /// make the freed surplus strategy-dependent — so the invariant is asserted on fixed payments.)
    #[test]
    fn avalanche_beats_snowball_on_interest_for_fixed_payments() {
        // A: small balance, low APR. B: large balance, high APR.
        let debts = [debt(50_000, 500, 5_000), debt(100_000, 2_400, 5_000)];
        let snow = simulate_payoff(&debts, PayoffStrategy::Snowball, 10_000);
        let aval = simulate_payoff(&debts, PayoffStrategy::Avalanche, 10_000);
        assert!(
            aval.total_interest_cents <= snow.total_interest_cents,
            "avalanche should not pay more interest: aval={} snow={}",
            aval.total_interest_cents,
            snow.total_interest_cents
        );
        assert!(aval.debt_free_month.is_some() && snow.debt_free_month.is_some());
    }

    /// Percent-of-balance minimums (the card path, `fixed_payment_cents = 0`): both strategies
    /// still clear the debts within the horizon (the surplus funds the target either way).
    #[test]
    fn percent_minimum_debts_pay_off_under_both_strategies() {
        let debts = [
            PayoffDebt {
                balance_cents: 300_000,
                apr_bps: 1_800,
                min_percent_bps: 200,
                min_floor_cents: 2_500,
                fixed_payment_cents: 0,
            },
            PayoffDebt {
                balance_cents: 150_000,
                apr_bps: 2_400,
                min_percent_bps: 200,
                min_floor_cents: 2_500,
                fixed_payment_cents: 0,
            },
        ];
        for strat in [PayoffStrategy::Snowball, PayoffStrategy::Avalanche] {
            let r = simulate_payoff(&debts, strat, 20_000);
            assert!(
                r.debt_free_month.is_some(),
                "{strat:?} should clear percent-min debts"
            );
        }
    }

    /// More extra clears the debt sooner (and never later).
    #[test]
    fn more_extra_is_never_slower() {
        let debts = [debt(200_000, 1_800, 5_000)];
        let low = simulate_payoff(&debts, PayoffStrategy::Avalanche, 5_000);
        let high = simulate_payoff(&debts, PayoffStrategy::Avalanche, 50_000);
        assert!(high.debt_free_month.unwrap() <= low.debt_free_month.unwrap());
        assert!(high.total_interest_cents <= low.total_interest_cents);
    }

    /// A minimum below the monthly interest never amortizes → no debt-free month.
    #[test]
    fn a_minimum_below_interest_never_pays_off() {
        // $10,000 at 36% APR (3%/mo) paying a 1% minimum ($100 < $300 interest) → grows forever.
        let d = PayoffDebt {
            balance_cents: 1_000_000,
            apr_bps: 3_600,
            min_percent_bps: 100,
            min_floor_cents: 0,
            fixed_payment_cents: 0,
        };
        let r = simulate_payoff(&[d], PayoffStrategy::MinimumOnly, 0);
        assert_eq!(r.debt_free_month, None);
    }

    /// A percent-only minimum (no floor) rounds to 0 on the last few cents; the residual guard
    /// clears it so a paying-down debt reaches zero instead of reporting "never debt-free".
    #[test]
    fn floorless_percent_minimum_still_pays_off() {
        let d = PayoffDebt {
            balance_cents: 500_000,
            apr_bps: 0,
            min_percent_bps: 200,
            min_floor_cents: 0,
            fixed_payment_cents: 0,
        };
        let r = simulate_payoff(&[d], PayoffStrategy::MinimumOnly, 0);
        assert!(
            r.debt_free_month.is_some(),
            "the floorless residual must clear"
        );
        assert_eq!(r.total_interest_cents, 0);
    }

    /// A runaway high-APR debt (minimum below interest) is still funded its full minimum every
    /// month — it must not starve a second debt or loop forever; the plan reports None.
    #[test]
    fn a_growing_minimum_does_not_starve_or_loop() {
        let grow = PayoffDebt {
            balance_cents: 2_000_000,
            apr_bps: 3_600, // 3%/mo on a 2% min → grows
            min_percent_bps: 200,
            min_floor_cents: 0,
            fixed_payment_cents: 0,
        };
        let other = PayoffDebt {
            balance_cents: 300_000,
            apr_bps: 2_900,
            min_percent_bps: 200,
            min_floor_cents: 2_500,
            fixed_payment_cents: 0,
        };
        let r = simulate_payoff(&[grow, other], PayoffStrategy::Snowball, 0);
        assert_eq!(r.debt_free_month, None);
        assert!(r.total_interest_cents > 0);
    }

    /// No debts → already debt-free, no interest. Determinism holds.
    #[test]
    fn no_debts_and_determinism() {
        let empty = simulate_payoff(&[], PayoffStrategy::Snowball, 10_000);
        assert_eq!(empty.debt_free_month, Some(0));
        assert_eq!(empty.total_interest_cents, 0);

        let debts = [debt(50_000, 500, 5_000), debt(100_000, 2_400, 5_000)];
        let a = simulate_payoff(&debts, PayoffStrategy::Avalanche, 12_345);
        let b = simulate_payoff(&debts, PayoffStrategy::Avalanche, 12_345);
        assert_eq!(a, b);
    }
}
