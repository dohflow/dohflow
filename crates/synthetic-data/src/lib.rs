//! Deterministic synthetic household data generator (personal-cfo-9ujs, ADR 0026 §7).
//!
//! The local-only privacy model forbids developing or validating forecast / learning
//! models on real user data, so they are built and tested against **synthetic**
//! households generated here. The headline invariant is **determinism**: the same
//! `(persona, seed, start, months)` always produces byte-identical output, so a fixture
//! is reproducible and a model's accuracy is a stable, regression-testable number.
//!
//! This first slice ships the generator core + the **couple / household** persona — the
//! one with the back-to-school / summer seasonality the adaptive spend-learning model
//! (`personal-cfo-pezm`) and the Layer-2 statistical forecast (`personal-cfo-9h1s`) are
//! built to deduce. The remaining personas (contractor `0ff1`, hourly `5z09`, power-user
//! `1o1x`) and the encrypted golden fixture vault (`66x9`) are follow-on slices.

use chrono::{Datelike, Months, NaiveDate};
use core_money::{Currency, Money};
use serde::{Deserialize, Serialize};

/// A small SplitMix64 PRNG. Inlined (no `rand`) so the output is byte-identical across
/// platforms and toolchains — a `rand` `StdRng` is explicitly *not* stable across
/// versions, which would break the fixture-reproducibility invariant.
struct SplitMix64(u64);

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A uniform `f64` in `[0, 1)`.
    fn unit(&mut self) -> f64 {
        // 53 bits of mantissa precision.
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// A multiplicative jitter factor in `[1 - spread, 1 + spread)`.
    fn jitter(&mut self, spread: f64) -> f64 {
        1.0 + (self.unit() * 2.0 - 1.0) * spread
    }

    /// A day-of-month in `1..=28` (avoids month-length edge cases).
    fn day(&mut self) -> u32 {
        1 + (self.next_u64() % 28) as u32
    }
}

/// The discretionary spending categories the Layer-2 model forecasts (`9h1s`). Iterated
/// in declaration order during generation, so the PRNG draw sequence is fixed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpendCategory {
    Groceries,
    Restaurants,
    Shopping,
    Gas,
    Entertainment,
    Healthcare,
    HouseholdSupplies,
    Travel,
}

/// One synthetic spending transaction (amount is negative — money out).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyntheticTransaction {
    pub date: NaiveDate,
    pub amount: Money,
    pub category: SpendCategory,
    pub merchant: String,
}

/// A synthetic income event (a paycheck; amount is positive).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyntheticIncome {
    pub date: NaiveDate,
    pub amount: Money,
    pub source: String,
}

/// A synthetic recurring bill (amount is negative).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyntheticBill {
    pub date: NaiveDate,
    pub amount: Money,
    pub name: String,
}

/// A synthetic balance observation (the checking-account balance on a date).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyntheticBalance {
    pub date: NaiveDate,
    pub balance: Money,
}

/// A fully-generated synthetic household over the requested horizon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyntheticHousehold {
    pub transactions: Vec<SyntheticTransaction>,
    pub income: Vec<SyntheticIncome>,
    pub bills: Vec<SyntheticBill>,
    pub balances: Vec<SyntheticBalance>,
}

/// Which household to generate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Persona {
    /// Dual-income couple/household with school-age seasonality (the flagship).
    CoupleHousehold,
}

/// Per-category spending shape: a base monthly budget (cents), a 12-month seasonal
/// multiplier (index 0 = January), and how many transactions to spread it across.
struct CategorySpec {
    category: SpendCategory,
    base_monthly_cents: i64,
    seasonal: [f64; 12],
    txns_per_month: u32,
    merchant: &'static str,
}

/// The couple/household spending model. Groceries / restaurants / gas peak in summer
/// (kids home); shopping spikes for back-to-school (Aug) and the holidays (Nov–Dec);
/// travel is lumpy (big summer + December trips) — the "extraordinary" spend the
/// learning model must keep out of the baseline.
const COUPLE_SPEND: &[CategorySpec] = &[
    CategorySpec {
        category: SpendCategory::Groceries,
        base_monthly_cents: 80_000,
        seasonal: [
            1.00, 1.00, 1.00, 1.02, 1.05, 1.18, 1.25, 1.30, 1.10, 1.00, 1.02, 1.08,
        ],
        txns_per_month: 13,
        merchant: "Whole Foods Market",
    },
    CategorySpec {
        category: SpendCategory::Restaurants,
        base_monthly_cents: 40_000,
        seasonal: [
            0.90, 0.95, 1.00, 1.00, 1.10, 1.20, 1.22, 1.15, 1.00, 1.00, 1.10, 1.30,
        ],
        txns_per_month: 9,
        merchant: "Local Bistro",
    },
    CategorySpec {
        category: SpendCategory::Shopping,
        base_monthly_cents: 30_000,
        seasonal: [
            0.80, 0.80, 0.90, 0.90, 0.90, 0.95, 0.95, 1.30, 1.00, 1.00, 1.55, 1.90,
        ],
        txns_per_month: 6,
        merchant: "Target",
    },
    CategorySpec {
        category: SpendCategory::Gas,
        base_monthly_cents: 20_000,
        seasonal: [
            0.90, 0.90, 1.00, 1.00, 1.10, 1.20, 1.30, 1.25, 1.10, 1.00, 0.95, 1.00,
        ],
        txns_per_month: 8,
        merchant: "Shell",
    },
    CategorySpec {
        category: SpendCategory::Entertainment,
        base_monthly_cents: 15_000,
        seasonal: [
            0.90, 0.90, 1.00, 1.00, 1.10, 1.30, 1.30, 1.20, 1.00, 1.00, 1.00, 1.40,
        ],
        txns_per_month: 5,
        merchant: "AMC Theatres",
    },
    CategorySpec {
        category: SpendCategory::Healthcare,
        base_monthly_cents: 10_000,
        seasonal: [
            1.00, 1.05, 1.00, 1.00, 1.00, 1.00, 1.00, 1.05, 1.10, 1.00, 1.00, 1.10,
        ],
        txns_per_month: 2,
        merchant: "CVS Pharmacy",
    },
    CategorySpec {
        category: SpendCategory::HouseholdSupplies,
        base_monthly_cents: 12_000,
        seasonal: [
            1.00, 1.00, 1.00, 1.00, 1.05, 1.10, 1.10, 1.15, 1.05, 1.00, 1.00, 1.05,
        ],
        txns_per_month: 4,
        merchant: "Home Depot",
    },
    CategorySpec {
        category: SpendCategory::Travel,
        base_monthly_cents: 6_000,
        seasonal: [
            0.30, 0.30, 0.50, 0.60, 0.80, 2.50, 4.00, 2.00, 0.60, 0.50, 1.00, 3.00,
        ],
        txns_per_month: 1,
        merchant: "Delta Air Lines",
    },
];

/// Generate a synthetic household for `persona`, reproducible from `(seed, start, months)`.
/// `start` is anchored to the first of its month; `months` is the horizon length.
#[must_use]
pub fn generate(persona: Persona, seed: u64, start: NaiveDate, months: u32) -> SyntheticHousehold {
    match persona {
        Persona::CoupleHousehold => couple_household(seed, start, months),
    }
}

fn couple_household(seed: u64, start: NaiveDate, months: u32) -> SyntheticHousehold {
    let mut rng = SplitMix64::new(seed);
    let start = start.with_day(1).unwrap_or(start);
    let end = add_months(start, months);

    // Income: two biweekly paychecks, the second offset by a week. Salaried → fixed
    // amounts (no jitter), so recurrence is cleanly detectable.
    let mut income = Vec::new();
    push_biweekly(&mut income, start, end, 240_000, "Employer A payroll");
    push_biweekly(
        &mut income,
        add_days(start, 7),
        end,
        190_000,
        "Employer B payroll",
    );

    let mut bills = Vec::new();
    let mut transactions = Vec::new();

    let mut month = start;
    while month < end {
        let m0 = month.month0() as usize;

        // Recurring bills on fixed days. Utilities swing with the season (heat / AC).
        push_bill(&mut bills, month, 1, -220_000, "Rent");
        let utilities = (12_000.0 * (1.0 + 0.5 * seasonal_swing(m0))) as i64;
        push_bill(&mut bills, month, 5, -utilities, "Utilities");
        push_bill(&mut bills, month, 10, -8_000, "Internet");
        push_bill(&mut bills, month, 15, -4_500, "Subscriptions");
        push_bill(&mut bills, month, 20, -9_000, "Phone");

        // Variable spending: each category's monthly budget = base × seasonal factor,
        // spread across its transactions on pseudo-random days with per-txn jitter.
        for spec in COUPLE_SPEND {
            let budget = (spec.base_monthly_cents as f64 * spec.seasonal[m0]) as i64;
            let per_txn = budget / i64::from(spec.txns_per_month.max(1));
            for _ in 0..spec.txns_per_month {
                let day = rng.day();
                let amount = -((per_txn as f64 * rng.jitter(0.25)).round() as i64);
                if let Some(date) = month.with_day(day) {
                    transactions.push(SyntheticTransaction {
                        date,
                        amount: Money::new(amount, Currency::Usd),
                        category: spec.category,
                        merchant: spec.merchant.to_owned(),
                    });
                }
            }
        }

        month = add_months(month, 1);
    }

    let balances = monthly_balances(&income, &bills, &transactions, start, end, 500_000);

    SyntheticHousehold {
        transactions,
        income,
        bills,
        balances,
    }
}

/// A symmetric −1..1 seasonal swing peaking mid-summer and mid-winter (for utilities).
fn seasonal_swing(month0: usize) -> f64 {
    // Two peaks/year: |cos| over the year, scaled to roughly [0, 1].
    let phase = (month0 as f64 + 0.5) / 12.0 * std::f64::consts::TAU;
    phase.cos().abs()
}

fn add_months(date: NaiveDate, months: u32) -> NaiveDate {
    date.checked_add_months(Months::new(months)).unwrap_or(date)
}

fn add_days(date: NaiveDate, days: i64) -> NaiveDate {
    date.checked_add_signed(chrono::Duration::days(days))
        .unwrap_or(date)
}

fn push_biweekly(
    out: &mut Vec<SyntheticIncome>,
    first: NaiveDate,
    end: NaiveDate,
    cents: i64,
    source: &str,
) {
    let mut date = first;
    while date < end {
        out.push(SyntheticIncome {
            date,
            amount: Money::new(cents, Currency::Usd),
            source: source.to_owned(),
        });
        date = add_days(date, 14);
    }
}

fn push_bill(out: &mut Vec<SyntheticBill>, month: NaiveDate, day: u32, cents: i64, name: &str) {
    if let Some(date) = month.with_day(day) {
        out.push(SyntheticBill {
            date,
            amount: Money::new(cents, Currency::Usd),
            name: name.to_owned(),
        });
    }
}

/// A checking-balance observation at the first of each month: the opening balance plus
/// every flow (income +, bills/spend −) dated before that month. Flows are summed in a
/// stable date order, so the series is deterministic.
fn monthly_balances(
    income: &[SyntheticIncome],
    bills: &[SyntheticBill],
    transactions: &[SyntheticTransaction],
    start: NaiveDate,
    end: NaiveDate,
    opening_cents: i64,
) -> Vec<SyntheticBalance> {
    let mut flows: Vec<(NaiveDate, i64)> = Vec::new();
    flows.extend(income.iter().map(|i| (i.date, i.amount.minor_units())));
    flows.extend(bills.iter().map(|b| (b.date, b.amount.minor_units())));
    flows.extend(
        transactions
            .iter()
            .map(|t| (t.date, t.amount.minor_units())),
    );
    flows.sort_by_key(|f| f.0); // stable → ties keep insertion order

    let mut balances = Vec::new();
    let mut month = start;
    while month <= end {
        let bal: i64 = opening_cents
            + flows
                .iter()
                .filter(|(date, _)| *date < month)
                .map(|(_, cents)| cents)
                .sum::<i64>();
        balances.push(SyntheticBalance {
            date: month,
            balance: Money::new(bal, Currency::Usd),
        });
        month = add_months(month, 1);
    }
    balances
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start() -> NaiveDate {
        NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()
    }

    /// The headline invariant: same `(persona, seed, start, months)` → identical output.
    #[test]
    fn generation_is_deterministic() {
        let a = generate(Persona::CoupleHousehold, 42, start(), 24);
        let b = generate(Persona::CoupleHousehold, 42, start(), 24);
        assert_eq!(a, b, "same seed must reproduce byte-identically");

        let c = generate(Persona::CoupleHousehold, 99, start(), 24);
        assert_ne!(
            a.transactions, c.transactions,
            "a different seed must vary the draws"
        );
    }

    /// 24 months of history is produced across all streams (what 9h1s ingests).
    #[test]
    fn produces_two_years_of_history() {
        let h = generate(Persona::CoupleHousehold, 7, start(), 24);
        // Biweekly × 2 earners over ~24 months ≈ 52 paychecks each.
        assert!(h.income.len() >= 100, "income: {}", h.income.len());
        // 5 bills × 24 months.
        assert_eq!(h.bills.len(), 5 * 24);
        // Hundreds of categorized spending transactions.
        assert!(h.transactions.len() > 500, "txns: {}", h.transactions.len());
        assert!(h.transactions.iter().all(|t| t.amount.minor_units() <= 0));
        assert!(h.income.iter().all(|i| i.amount.minor_units() > 0));
    }

    /// Seasonality is present and detectable — the whole point. Shopping spikes for the
    /// holidays, so December shopping spend far exceeds a flat month's (February).
    #[test]
    fn shopping_is_seasonal_holidays_peak() {
        let h = generate(Persona::CoupleHousehold, 7, start(), 24);
        let spend = |month: u32| -> i64 {
            h.transactions
                .iter()
                .filter(|t| t.category == SpendCategory::Shopping && t.date.month() == month)
                .map(|t| -t.amount.minor_units())
                .sum()
        };
        let december = spend(12);
        let february = spend(2);
        assert!(
            december > february * 2,
            "December shopping {december} should dwarf February {february}"
        );
    }

    /// Travel is lumpy: a summer-month total is many times a winter-month total — the
    /// extraordinary spend the learning model must keep out of the baseline.
    #[test]
    fn travel_is_lumpy_summer_heavy() {
        let h = generate(Persona::CoupleHousehold, 7, start(), 24);
        let spend = |month: u32| -> i64 {
            h.transactions
                .iter()
                .filter(|t| t.category == SpendCategory::Travel && t.date.month() == month)
                .map(|t| -t.amount.minor_units())
                .sum()
        };
        assert!(spend(7) > spend(1) * 3, "July travel should dwarf January");
    }
}
