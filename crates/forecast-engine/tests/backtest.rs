//! Layer-2 model backtest (personal-cfo-5ie.1): is the learned spend band any good?
//!
//! After the band went live (9h1s — the forecast now widens with learned variable
//! spend), this measures whether it actually *predicts* a household's held-out future.
//! Pure: it trains [`SpendModel`] on the early months of a deterministic synthetic
//! household and scores the held-out tail — no DB, no live persistence, no encrypted
//! fixtures, just the synthetic-data library + the pure forecast-engine model.
//!
//! Two metrics, the standard pair for a probabilistic forecast:
//! - **P50 MAPE** (accuracy) — how far the central estimate is from the realized total.
//! - **[P10,P90] coverage** (calibration) — the share of realized totals that land
//!   inside the band (nominal 80%).
//!
//! The measured numbers are recorded in the asserts as the baseline a model change must
//! not regress past.

use std::collections::BTreeMap;

use chrono::{Datelike, NaiveDate};
use forecast_engine::layer2::{SpendModel, SpendObservation};
use synthetic_data::{generate, Persona, SyntheticTransaction};

/// Categories the couple persona spends on *regularly* (predictable season to season) —
/// held to a tighter accuracy envelope than the lumpy ones (travel, holiday shopping).
const REGULAR: &[&str] = &[
    "Groceries",
    "Restaurants",
    "Gas",
    "Healthcare",
    "HouseholdSupplies",
    "Entertainment",
];

/// Sum spend magnitude into `(category, year, calendar-month)` totals.
fn monthly_totals(txns: &[&SyntheticTransaction]) -> BTreeMap<(String, i32, u32), i64> {
    let mut totals = BTreeMap::new();
    for t in txns {
        *totals
            .entry((format!("{:?}", t.category), t.date.year(), t.date.month()))
            .or_insert(0) += -t.amount.minor_units();
    }
    totals
}

#[test]
fn layer2_backtest_predicts_held_out_spend() {
    let start = NaiveDate::from_ymd_opt(2020, 1, 1).unwrap();
    let household = generate(Persona::CoupleHousehold, 7, start, 36);
    // Train on the first two years; hold out the third.
    let cutoff = NaiveDate::from_ymd_opt(2022, 1, 1).unwrap();

    let train: Vec<SpendObservation> = household
        .transactions
        .iter()
        .filter(|t| t.date < cutoff)
        .map(|t| SpendObservation {
            date: t.date,
            category: format!("{:?}", t.category),
            amount_cents: -t.amount.minor_units(),
        })
        .collect();
    let model = SpendModel::fit(&train);

    let test_txns: Vec<&SyntheticTransaction> = household
        .transactions
        .iter()
        .filter(|t| t.date >= cutoff)
        .collect();
    let actuals = monthly_totals(&test_txns);

    let mut ape_by_cat: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    let (mut covered, mut total) = (0u32, 0u32);
    for ((cat, _year, month), actual) in &actuals {
        let Some(band) = model.band(cat, *month) else {
            continue;
        };
        #[allow(clippy::cast_precision_loss)]
        let ape = (actual - band.p50_cents).abs() as f64 / *actual as f64;
        ape_by_cat.entry(cat.clone()).or_default().push(ape);
        total += 1;
        if band.p10_cents <= *actual && *actual <= band.p90_cents {
            covered += 1;
        }
    }

    #[allow(clippy::cast_precision_loss)]
    let coverage = f64::from(covered) / f64::from(total);
    let (mut worst_regular, mut worst_overall) = (0.0f64, 0.0f64);
    eprintln!("=== Layer-2 backtest (couple, seed 7, train 24mo, test 12mo) ===");
    for (cat, apes) in &ape_by_cat {
        #[allow(clippy::cast_precision_loss)]
        let mape = apes.iter().sum::<f64>() / apes.len() as f64;
        worst_overall = worst_overall.max(mape);
        let tag = if REGULAR.contains(&cat.as_str()) {
            worst_regular = worst_regular.max(mape);
            "regular"
        } else {
            "lumpy"
        };
        eprintln!("  {cat:<18} {tag:<8} MAPE {:>5.1}%", mape * 100.0);
    }
    eprintln!(
        "  coverage [P10,P90]: {:.0}% of {total} predictions",
        coverage * 100.0
    );

    // --- Baseline envelopes (couple/seed 7, deterministic). A model change must not
    // regress past these; an *improvement* (esp. to coverage) tightens them. ---
    assert!(
        total >= 80,
        "expected ~8 categories x 12 months, got {total}"
    );

    // ACCURACY is the model's strong suit: the P50 tracks held-out spend well. Measured
    // worst regular-category MAPE = 11.4% (Healthcare); worst overall = 16.4% (Travel).
    assert!(
        worst_regular < 0.15,
        "regular-category P50 MAPE regressed past 15%: {worst_regular:.3}"
    );
    assert!(
        worst_overall < 0.20,
        "worst-category P50 MAPE regressed past 20%: {worst_overall:.3}"
    );

    // CALIBRATION: the Student-t prediction band (personal-cfo-5ie.2) is well-calibrated.
    // Measured coverage is 82% — right at the nominal 80%, up from 26% for the naive
    // empirical band on the same two years of history. (The undercoverage was structural,
    // not a bug: empirical P10/P90 from ~2 samples per bucket is far too narrow; the t
    // interval restores honest width that narrows toward 1.28·σ as history accrues.) Guard
    // that it stays calibrated — neither collapsing (overconfident) nor ballooning
    // (uninformative).
    assert!(
        (0.70..=0.95).contains(&coverage),
        "band coverage left the calibrated 70-95% envelope: {coverage:.3}"
    );
}
