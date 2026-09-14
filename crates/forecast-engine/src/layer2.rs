//! Layer-2 statistical spend model (ADR 0026 §7, personal-cfo-9h1s).
//!
//! Where Layer-1 ([`crate::forecast_layer1`]) folds the obligations a household has
//! *entered* into a deterministic per-day balance line, Layer-2 widens the
//! variable-spend portion into a **band** — P10/P50/P90 — learned from the household's
//! own history. This is the defensible *baseline* per ADR 0026 §7: empirical quantiles
//! of historical spend, bucketed by category and calendar month so seasonality (summer /
//! back-to-school / holidays) is reflected. A tuned model (exponential smoothing,
//! Holt-Winters, day-of-week patterns) earns its own ADR when designed against real data;
//! this prototype is validated on synthetic households, never real user data (local-only
//! privacy).
//!
//! Pure + deterministic like the rest of the crate: identical observations in →
//! byte-identical model out (`BTreeMap` ordering + integer-cent quantiles; no RNG / clock
//! / IO).

use std::collections::BTreeMap;

use chrono::{Datelike, NaiveDate};
use core_money::Money;

use crate::{Band, DailyBalance};

/// One historical spending observation: a dated amount in a category. `amount_cents` is
/// the spend **magnitude** (positive minor units); the caller negates ledger debits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpendObservation {
    pub date: NaiveDate,
    pub category: String,
    pub amount_cents: i64,
}

/// A P10/P50/P90 **spend** band in minor units (cents) — distinct from the crate's
/// balance [`crate::Band`] (`Money`). Currency-agnostic; the overlay wraps it in the
/// household `Money` when it widens a forecast row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpendBand {
    pub p10_cents: i64,
    pub p50_cents: i64,
    pub p90_cents: i64,
}

/// The fitted Layer-2 spend model: per-category monthly bands, both **seasonal**
/// (category × calendar-month) and **overall** (category — the fallback for a thin
/// season).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpendModel {
    seasonal: BTreeMap<(String, u32), SpendBand>,
    overall: BTreeMap<String, SpendBand>,
}

impl SpendModel {
    /// Fit the model from a household's spend history. Spend is summed into per
    /// `(category, year-month)` totals; the band for a `(category, calendar-month)` is the
    /// empirical quantiles of that bucket's monthly totals across the years of history.
    #[must_use]
    pub fn fit(history: &[SpendObservation]) -> Self {
        // 1. Monthly totals per (category, year, calendar-month).
        let mut monthly: BTreeMap<(String, i32, u32), i64> = BTreeMap::new();
        for obs in history {
            let key = (obs.category.clone(), obs.date.year(), obs.date.month());
            *monthly.entry(key).or_default() += obs.amount_cents;
        }

        // 2. Regroup the monthly totals into seasonal + overall sample sets.
        let mut seasonal_samples: BTreeMap<(String, u32), Vec<i64>> = BTreeMap::new();
        let mut overall_samples: BTreeMap<String, Vec<i64>> = BTreeMap::new();
        for ((category, _year, month), total) in monthly {
            seasonal_samples
                .entry((category.clone(), month))
                .or_default()
                .push(total);
            overall_samples.entry(category).or_default().push(total);
        }

        // 3. Quantize each sample set into a band.
        let seasonal = seasonal_samples
            .into_iter()
            .map(|(key, mut samples)| (key, band_of(&mut samples)))
            .collect();
        let overall = overall_samples
            .into_iter()
            .map(|(key, mut samples)| (key, band_of(&mut samples)))
            .collect();
        Self { seasonal, overall }
    }

    /// The forward spend band for `category` in `calendar_month` (1–12). Uses the seasonal
    /// bucket, falling back to the category's overall distribution when that season has no
    /// history.
    #[must_use]
    pub fn band(&self, category: &str, calendar_month: u32) -> Option<SpendBand> {
        self.seasonal
            .get(&(category.to_owned(), calendar_month))
            .or_else(|| self.overall.get(category))
            .copied()
    }

    /// This category's typical monthly spend (the overall, non-seasonal P50), or `None`
    /// when the model never saw it. Used to APPORTION a household-level planned change
    /// across the per-account models, so the same plan means the same thing on the
    /// aggregate chart and the per-account charts (personal-cfo-4d8.27.6.2).
    #[must_use]
    pub fn expected_monthly(&self, category: &str) -> Option<i64> {
        self.overall.get(category).map(|b| b.p50_cents)
    }

    /// The categories the model has learned, in deterministic (sorted) order.
    pub fn categories(&self) -> impl Iterator<Item = &str> {
        self.overall.keys().map(String::as_str)
    }
}

/// Fit a forward spend band from a bucket's historical monthly totals. The **P50** is the
/// empirical median — an accurate point estimate (backtest personal-cfo-5ie.1). The
/// **P10/P90** form an ~80% *prediction* interval for next period's total via a Student-t
/// prediction interval ([`prediction_half_width`]): wide when the sample is tiny (with two
/// years of history the naive empirical band covered only ~26% of held-out actuals; the t
/// interval widens it toward the nominal 80%, personal-cfo-5ie.2) and narrowing toward the
/// normal `1.28·σ` as evidence accrues. Sorts in place; an empty set collapses to zero.
fn band_of(samples: &mut [i64]) -> SpendBand {
    samples.sort_unstable();
    let p50 = quantile(samples, 0.50);
    let half = prediction_half_width(samples);
    SpendBand {
        p10_cents: (p50 - half).max(0), // a spend total can't go negative
        p50_cents: p50,
        p90_cents: p50 + half,
    }
}

/// Linear-interpolation quantile of a pre-sorted slice.
fn quantile(sorted: &[i64], q: f64) -> i64 {
    match sorted.len() {
        0 => 0,
        1 => sorted[0],
        n => {
            let pos = q * (n - 1) as f64;
            let lo = pos.floor() as usize;
            let hi = pos.ceil() as usize;
            let frac = pos - lo as f64;
            #[allow(clippy::cast_precision_loss)]
            let interp = sorted[lo] as f64 + (sorted[hi] as f64 - sorted[lo] as f64) * frac;
            interp.round() as i64
        }
    }
}

/// One-observation bucket: no spread to estimate, so assume a wide "we don't know yet"
/// band of ±50% of the lone value.
const SINGLE_SAMPLE_CV: f64 = 0.5;

/// Half-width of the ~80% central prediction interval (P10..P90) for *next period's* total,
/// from a small historical sample. A Student-t prediction interval —
/// `t_{0.90, n-1} · s · √(1 + 1/n)` — which is fat for a tiny sample (genuine ignorance
/// about the spread) and narrows toward `1.28·s` as `n` grows (personal-cfo-5ie.2).
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn prediction_half_width(sorted: &[i64]) -> i64 {
    let n = sorted.len();
    if n == 0 {
        return 0;
    }
    if n == 1 {
        return (sorted[0] as f64 * SINGLE_SAMPLE_CV).round() as i64;
    }
    let mean = sorted.iter().sum::<i64>() as f64 / n as f64;
    let variance = sorted
        .iter()
        .map(|&x| {
            let d = x as f64 - mean;
            d * d
        })
        .sum::<f64>()
        / (n as f64 - 1.0); // Bessel-corrected sample variance
    let std = variance.sqrt();
    let inflation = (1.0 + 1.0 / n as f64).sqrt(); // parameter-uncertainty term
    (t_quantile_90(n - 1) * std * inflation).round() as i64
}

/// The 0.90 quantile of the Student-t distribution with `df` degrees of freedom (a standard
/// table). `df > 30` uses the normal limit `z_{0.90} = 1.2816`.
fn t_quantile_90(df: usize) -> f64 {
    const T90: [f64; 30] = [
        3.078, 1.886, 1.638, 1.533, 1.476, 1.440, 1.415, 1.397, 1.383, 1.372, //
        1.363, 1.356, 1.350, 1.345, 1.341, 1.337, 1.333, 1.330, 1.328, 1.325, //
        1.323, 1.321, 1.319, 1.318, 1.316, 1.315, 1.314, 1.313, 1.311, 1.310,
    ];
    match df {
        0 => T90[0],
        d => T90.get(d - 1).copied().unwrap_or(1.2816),
    }
}

/// A planned change to one category's spend over an optional date window
/// (personal-cfo-4d8.27.6.2) — "reduce Dining by $200/month from August".
///
/// `delta_cents_per_month` is signed: negative reduces spend (and so raises projected
/// cash), positive increases it. It is prorated across the days of each month, exactly
/// as the model's own bands are, so a partial month is handled without special-casing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpendAdjustment {
    /// The model's category key (the category id, as the observations carry it).
    pub category: String,
    /// Signed monthly change in cents; negative = spend less.
    pub delta_cents_per_month: i64,
    /// Applies on/after this date; `None` = from the start of the horizon.
    pub start: Option<NaiveDate>,
    /// Applies through this date (inclusive); `None` = to the end of the horizon.
    pub end: Option<NaiveDate>,
}

impl SpendAdjustment {
    fn covers(&self, date: NaiveDate) -> bool {
        self.start.is_none_or(|s| date >= s) && self.end.is_none_or(|e| date <= e)
    }
}

/// Widen a Layer-1 balance series into a Layer-2 band by overlaying the projected
/// variable spend the [`SpendModel`] learned (ADR 0026 §7, personal-cfo-9h1s), with no
/// planned changes — see [`widen_with_spend_adjusted`] to apply those.
///
/// Each day's **P50** drops by the cumulative expected discretionary spend to date — the
/// Layer-1 line folds only *entered* obligations, so this makes the projection honestly
/// less optimistic. **P10/P90** invert the cumulative spend band (high spend → low cash),
/// so the balance band **widens monotonically** across the horizon as spend uncertainty
/// accrues. The returned series keeps each day's date + events; only `closing` changes.
#[must_use]
pub fn widen_with_spend(layer1: &[DailyBalance], model: &SpendModel) -> Vec<DailyBalance> {
    widen_with_spend_adjusted(layer1, model, &[])
}

/// As [`widen_with_spend`], with per-category planned changes applied to the projected
/// draw (personal-cfo-4d8.27.6.2).
///
/// The adjustment moves the **expected draw**, not the fitted spread: committing to
/// spend $200 less on dining does not by itself make the outcome more certain, so p10
/// and p90 shift with p50 rather than narrowing toward it. A category's adjusted draw is
/// floored at zero — a reduction larger than the modelled spend means "about nothing",
/// never a projected inflow.
#[must_use]
pub fn widen_with_spend_adjusted(
    layer1: &[DailyBalance],
    model: &SpendModel,
    adjustments: &[SpendAdjustment],
) -> Vec<DailyBalance> {
    let Some(first) = layer1.first() else {
        return Vec::new();
    };
    let currency = first.closing.p50.currency();
    let categories: Vec<String> = model.categories().map(str::to_owned).collect();

    let (mut cum_p10, mut cum_p50, mut cum_p90) = (0i64, 0i64, 0i64);
    layer1
        .iter()
        .map(|day| {
            // This day's expected spend band = each category's month band / its day count.
            let days = days_in_month(day.date).max(1);
            let month = day.date.month();
            for category in &categories {
                if let Some(band) = model.band(category, month) {
                    // A planned change shifts the whole band for the days it covers;
                    // prorated per day like the band itself. Summed across a month it
                    // lands within rounding of the stated monthly delta.
                    let delta = adjustments
                        .iter()
                        .filter(|a| a.category == *category && a.covers(day.date))
                        .map(|a| a.delta_cents_per_month)
                        .sum::<i64>()
                        / days;
                    cum_p10 += (band.p10_cents / days + delta).max(0);
                    cum_p50 += (band.p50_cents / days + delta).max(0);
                    cum_p90 += (band.p90_cents / days + delta).max(0);
                }
            }
            // Subtract cumulative spend from the Layer-1 point; high spend → low cash.
            let base = day.closing.p50.minor_units();
            DailyBalance {
                date: day.date,
                closing: Band {
                    p10: Money::new(base - cum_p90, currency),
                    p50: Money::new(base - cum_p50, currency),
                    p90: Money::new(base - cum_p10, currency),
                },
                events: day.events.clone(),
            }
        })
        .collect()
}

/// A discrete uncertainty "lump" injected at a specific date — the analytic per‑payment card
/// lump (ADR 0050): on a pay‑in‑full card the variable part of a future statement is uncertain,
/// and that uncertainty lands on cash as a symmetric spread around the (deterministic) payment
/// on its due date. `half_width_cents` is the one‑sided 80% spread (`≥ 0`); it is sized on the
/// impure side (db‑worker) from the statement estimator's walk‑forward MAPE, so this engine stays
/// integer + RNG‑free.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LumpInjection {
    pub date: NaiveDate,
    pub half_width_cents: i64,
}

/// Widen a balance series by discrete lump injections (ADR 0050, analytic full‑payer card lump).
///
/// Each lump adds a symmetric `± half_width_cents` to that day's P10/P90 (P50 is unchanged — the
/// mean payment is deterministic) for every day **on or after** its date, so the band steps wider
/// at each payment and stays wider (the realized uncertainty persists). Lumps accumulate linearly,
/// matching [`widen_with_spend`]'s comonotonic convention, so the total band is monotonically
/// non‑decreasing. Pure + integer: the caller precomputes the widths. An empty list or all‑zero
/// widths returns the series unchanged (a closed statement injects nothing → the deterministic
/// line, the "no width for a known amount" invariant).
#[must_use]
pub fn widen_with_lumps(series: &[DailyBalance], lumps: &[LumpInjection]) -> Vec<DailyBalance> {
    if lumps.iter().all(|l| l.half_width_cents == 0) {
        return series.to_vec();
    }
    series
        .iter()
        .map(|day| {
            let cum: i64 = lumps
                .iter()
                .filter(|l| l.date <= day.date)
                .map(|l| l.half_width_cents)
                .sum();
            if cum == 0 {
                return day.clone();
            }
            let currency = day.closing.p50.currency();
            DailyBalance {
                date: day.date,
                closing: Band {
                    p10: Money::new(day.closing.p10.minor_units() - cum, currency),
                    p50: day.closing.p50,
                    p90: Money::new(day.closing.p90.minor_units() + cum, currency),
                },
                events: day.events.clone(),
            }
        })
        .collect()
}

/// The number of days in `date`'s calendar month.
fn days_in_month(date: NaiveDate) -> i64 {
    let (year, month) = (date.year(), date.month());
    let first = NaiveDate::from_ymd_opt(year, month, 1);
    let next = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)
    };
    match (first, next) {
        (Some(start), Some(end)) => (end - start).num_days(),
        _ => 30,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_money::Currency;
    use synthetic_data::{generate, Persona};

    /// A flat Layer-1 series (collapsed bands) — the deterministic line the overlay widens.
    fn flat_layer1(start: NaiveDate, days: u32, opening_cents: i64) -> Vec<DailyBalance> {
        (0..u64::from(days))
            .map(|d| DailyBalance {
                date: start.checked_add_days(chrono::Days::new(d)).unwrap(),
                closing: Band::point(Money::new(opening_cents, Currency::Usd)),
                events: Vec::new(),
            })
            .collect()
    }

    /// `months` of couple/household spend history, mapped to Layer-2 observations.
    fn history(months: u32) -> Vec<SpendObservation> {
        let start = NaiveDate::from_ymd_opt(2021, 1, 1).unwrap();
        generate(Persona::CoupleHousehold, 7, start, months)
            .transactions
            .iter()
            .map(|t| SpendObservation {
                date: t.date,
                category: format!("{:?}", t.category),
                amount_cents: -t.amount.minor_units(), // ledger debit → positive magnitude
            })
            .collect()
    }

    #[test]
    fn fit_is_deterministic() {
        assert_eq!(SpendModel::fit(&history(24)), SpendModel::fit(&history(24)));
    }

    #[test]
    fn band_is_ordered_p10_le_p50_le_p90() {
        let model = SpendModel::fit(&history(48));
        let cats: Vec<String> = model.categories().map(str::to_owned).collect();
        for cat in cats {
            for month in 1..=12 {
                if let Some(b) = model.band(&cat, month) {
                    assert!(
                        b.p10_cents <= b.p50_cents && b.p50_cents <= b.p90_cents,
                        "{cat} m{month}: {b:?} not ordered"
                    );
                }
            }
        }
    }

    /// 5ie.2 calibration: the prediction band tracks *uncertainty* — fewer samples of the
    /// same spread yield a wider band (the Student-t multiplier shrinks as `n` grows).
    #[test]
    fn fewer_samples_yield_a_wider_band() {
        let width = |mut samples: Vec<i64>| {
            let b = band_of(&mut samples);
            b.p90_cents - b.p10_cents
        };
        let thin = width(vec![100_000, 200_000]); // n = 2
        let thick = width(vec![100_000, 200_000, 100_000, 200_000, 100_000, 200_000]); // n = 6, same spread
        assert!(
            thin > thick,
            "a 2-sample band should be wider than a 6-sample band of the same spread: {thin} vs {thick}"
        );
        assert!(thick > 0, "a multi-sample band must still have width");
    }

    /// 9h1s AC: with synthetic seasonal data, P50 tracks the bucket mean within budget.
    /// 48 months → 4 yearly samples per (category, calendar-month) bucket.
    #[test]
    fn p50_tracks_the_seasonal_mean() {
        let hist = history(48);
        let model = SpendModel::fit(&hist);
        let (cat, month) = ("Groceries", 8); // the back-to-school peak
        let band = model.band(cat, month).unwrap();

        // Recompute the bucket's per-year monthly totals + their mean directly.
        let mut totals: BTreeMap<i32, i64> = BTreeMap::new();
        for o in hist
            .iter()
            .filter(|o| o.category == cat && o.date.month() == month)
        {
            *totals.entry(o.date.year()).or_default() += o.amount_cents;
        }
        let mean = totals.values().sum::<i64>() / i64::try_from(totals.len()).unwrap();
        let off = (band.p50_cents - mean).abs() as f64 / mean as f64;
        assert!(
            off < 0.20,
            "P50 {} vs mean {} → {:.1}% off",
            band.p50_cents,
            mean,
            off * 100.0
        );
    }

    /// Seasonality is captured: the holiday Shopping band sits well above a flat month.
    #[test]
    fn shopping_band_reflects_seasonality() {
        let model = SpendModel::fit(&history(48));
        let dec = model.band("Shopping", 12).unwrap();
        let feb = model.band("Shopping", 2).unwrap();
        assert!(
            dec.p50_cents > feb.p50_cents * 2,
            "Dec shopping P50 {} should dwarf Feb {}",
            dec.p50_cents,
            feb.p50_cents
        );
    }

    #[test]
    fn widen_lowers_the_line_and_widens_the_band() {
        let model = SpendModel::fit(&history(48));
        let start = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
        let layer1 = flat_layer1(start, 365, 5_000_000); // a flat $50k Layer-1 line
        let out = widen_with_spend(&layer1, &model);

        assert_eq!(out.len(), layer1.len());
        // Every day's band is ordered P10 ≤ P50 ≤ P90.
        for day in &out {
            assert!(day.closing.p10.minor_units() <= day.closing.p50.minor_units());
            assert!(day.closing.p50.minor_units() <= day.closing.p90.minor_units());
        }
        let p50 = |i: usize| out[i].closing.p50.minor_units();
        let width = |i: usize| out[i].closing.p90.minor_units() - out[i].closing.p10.minor_units();
        // P50 falls below the Layer-1 line (variable spend is now subtracted) and keeps
        // falling as cumulative spend accrues.
        assert!(
            p50(30) < 5_000_000,
            "P50 should drop below the line within a month"
        );
        assert!(
            p50(364) < p50(30),
            "P50 should keep falling over the horizon"
        );
        // The band widens further out (uncertainty grows).
        assert!(
            width(364) > width(30) && width(30) > 0,
            "band should widen over time"
        );
    }

    #[test]
    fn widen_is_deterministic() {
        let model = SpendModel::fit(&history(24));
        let start = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
        let layer1 = flat_layer1(start, 200, 4_000_000);
        assert_eq!(
            widen_with_spend(&layer1, &model),
            widen_with_spend(&layer1, &model)
        );
    }

    #[test]
    fn widen_of_empty_series_is_empty() {
        let model = SpendModel::fit(&history(24));
        assert!(widen_with_spend(&[], &model).is_empty());
    }

    #[test]
    fn lumps_widen_the_band_from_their_date_forward() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let series = flat_layer1(start, 60, 1_000_000); // collapsed point bands
        let d = NaiveDate::from_ymd_opt(2026, 1, 20).unwrap();
        let out = widen_with_lumps(
            &series,
            &[LumpInjection {
                date: d,
                half_width_cents: 5_000,
            }],
        );
        // Before the lump date: still a point (no width).
        let before = &out[10]; // 2026-01-11
        assert_eq!(before.closing.p10, before.closing.p90);
        // On/after the lump date: ±5000 around the unchanged P50.
        let on = &out[19]; // 2026-01-20
        assert_eq!(on.closing.p50.minor_units(), 1_000_000);
        assert_eq!(on.closing.p10.minor_units(), 1_000_000 - 5_000);
        assert_eq!(on.closing.p90.minor_units(), 1_000_000 + 5_000);
        // A second, later lump accumulates linearly.
        let out2 = widen_with_lumps(
            &series,
            &[
                LumpInjection {
                    date: d,
                    half_width_cents: 5_000,
                },
                LumpInjection {
                    date: NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
                    half_width_cents: 3_000,
                },
            ],
        );
        let last = out2.last().unwrap();
        assert_eq!(
            last.closing.p90.minor_units() - last.closing.p10.minor_units(),
            2 * (5_000 + 3_000)
        );
    }

    #[test]
    fn empty_or_zero_lumps_pass_through() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let series = flat_layer1(start, 30, 1_000_000);
        assert_eq!(widen_with_lumps(&series, &[]), series);
        let zero = [LumpInjection {
            date: start,
            half_width_cents: 0,
        }];
        assert_eq!(widen_with_lumps(&series, &zero), series);
    }

    #[test]
    fn widen_with_lumps_is_deterministic() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let series = flat_layer1(start, 90, 2_000_000);
        let lumps = [LumpInjection {
            date: NaiveDate::from_ymd_opt(2026, 2, 15).unwrap(),
            half_width_cents: 7_000,
        }];
        assert_eq!(
            widen_with_lumps(&series, &lumps),
            widen_with_lumps(&series, &lumps)
        );
    }

    /// 12 months of a single category at a steady monthly total.
    fn single_category_history(category: &str, monthly_cents: i64) -> Vec<SpendObservation> {
        (0..12)
            .map(|m: u32| SpendObservation {
                date: NaiveDate::from_ymd_opt(2025, m + 1, 15).unwrap(),
                category: category.to_owned(),
                amount_cents: monthly_cents,
            })
            .collect()
    }

    /// personal-cfo-4d8.27.6.2: a planned reduction lowers the projected draw, so the
    /// balance ends HIGHER — and the reduction only applies inside its window.
    #[test]
    fn a_spend_adjustment_shifts_the_projected_draw() {
        let start = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        let layer1 = flat_layer1(start, 60, 1_000_000);
        let model = SpendModel::fit(&single_category_history("dining", 30_000));

        let plain = widen_with_spend(&layer1, &model);
        let reduced = widen_with_spend_adjusted(
            &layer1,
            &model,
            &[SpendAdjustment {
                category: "dining".to_owned(),
                delta_cents_per_month: -10_000,
                start: None,
                end: None,
            }],
        );
        let plain_end = plain.last().unwrap().closing.p50.minor_units();
        let reduced_end = reduced.last().unwrap().closing.p50.minor_units();
        assert!(
            reduced_end > plain_end,
            "spending less must project MORE cash: {reduced_end} vs {plain_end}"
        );

        // Windowed: an adjustment that ends before the horizon starts changes nothing.
        let outside = widen_with_spend_adjusted(
            &layer1,
            &model,
            &[SpendAdjustment {
                category: "dining".to_owned(),
                delta_cents_per_month: -10_000,
                start: None,
                end: Some(NaiveDate::from_ymd_opt(2026, 7, 31).unwrap()),
            }],
        );
        assert_eq!(
            outside.last().unwrap().closing.p50.minor_units(),
            plain_end,
            "an adjustment outside the horizon must not move the projection",
        );
    }

    /// A reduction larger than the modelled spend means "about nothing", never an inflow.
    #[test]
    fn an_over_large_reduction_floors_at_zero_spend() {
        let start = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        let layer1 = flat_layer1(start, 60, 1_000_000);
        let model = SpendModel::fit(&single_category_history("dining", 30_000));
        let zeroed = widen_with_spend_adjusted(
            &layer1,
            &model,
            &[SpendAdjustment {
                category: "dining".to_owned(),
                delta_cents_per_month: -10_000_000,
                start: None,
                end: None,
            }],
        );
        let start = layer1.first().unwrap().closing.p50.minor_units();
        assert_eq!(
            zeroed.last().unwrap().closing.p50.minor_units(),
            start,
            "every category floored to zero leaves the deterministic line exactly flat \
             (a `<=` assertion here would also pass for a sign inversion)",
        );
    }
}
