//! Descriptive comfort-band drift attribution (personal-cfo-5ie.8, ADR 0018 §915.1).
//!
//! A pure, deterministic engine that explains *why* the Future Cash projection is set to cross
//! **below** the comfort band: it attributes the crossing to the spending categories whose
//! baseline has **risen recently** (within the current run's own history — no cross-run baseline
//! versioning, per the 2026-07-02 decision). The output carries only facts (category + amounts);
//! the descriptive copy that renders it (and the non-advice-phrase boundary) lives in the UI.
//!
//! Timing (ADR 0018 addendum): only a **far-horizon** crossing is treated as drift here; a nearer
//! shortfall is the user-steered "cover it" tool's job. `NEAR_HORIZON_DAYS` is that documented line.
//!
//! The `DbWorker::band_drift` read runs this over the real forecast + spend history and enriches
//! the category ids with names for the UI; a durable `cash_band_breach` risk_flag on every
//! forecast run is a later durability add (the schema is ready, migration v34).

use chrono::{Datelike, Duration, NaiveDate};
use std::collections::{BTreeMap, BTreeSet};

/// Crossings sooner than this are the "cover it" tool's job, not drift attribution (ADR 0018
/// timing addendum). A small documented constant, not a modelled quantity.
const NEAR_HORIZON_DAYS: i64 = 45;
/// The recent window whose per-category spend rate is compared against the preceding equal window.
const WINDOW_DAYS: i64 = 90;
/// At most this many contributing categories are attributed (the largest risers).
const MAX_FACTORS: usize = 3;
/// A category must recur in at least this many distinct recent months to count as a *baseline*
/// rise — so a one-off spike (a single month) isn't mistaken for a sustained shift. Upstream
/// already drops extraordinary/lumpy spend (ADR 0038); this hardens against the rest.
const MIN_RECENT_MONTHS: usize = 2;

/// Which band edge the projection crosses. Drift attribution covers the **lower** edge only —
/// crossing *above* the upper edge is excess (the absence of spend), not a rising-baseline drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BandEdge {
    Lower,
}

/// A category's contribution to a lower-band drift: its recent monthly spend and how much that is
/// **up** versus the preceding window. `delta_minor` is positive by construction (risers only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContributingFactor {
    pub category_id: String,
    pub recent_monthly_minor: i64,
    pub delta_minor: i64,
}

/// A descriptive band-drift signal: the projection crosses the band on `crossing_date`, reaching
/// `magnitude_minor` past the edge at its worst, attributed to `contributing_factors`. Facts only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BandDriftSignal {
    pub crossing_date: NaiveDate,
    pub edge: BandEdge,
    pub magnitude_minor: i64,
    pub contributing_factors: Vec<ContributingFactor>,
}

/// Deterministic within-run band-drift attribution (5ie.8). `days` is the projected series as
/// `(date, net closing p50 minor units)`, ascending; `spend` is categorized ordinary variable
/// spend as `(date, category_id, amount minor units)`.
///
/// Emits `Some` only when **both**: (1) the projection closes below `lower_minor` on a day at
/// least `NEAR_HORIZON_DAYS` past `as_of` (a far-horizon crossing), and (2) at least one category's
/// baseline has risen (its recent monthly rate exceeds the preceding window's). If nothing rose,
/// the crossing is not attributable to a spending drift — the plain crossing signal (3v6d) covers
/// it — so this returns `None`. Same inputs → same output (integer math, sorted attribution).
pub(crate) fn compute_band_drift_signal(
    days: &[(NaiveDate, i64)],
    lower_minor: i64,
    spend: &[(NaiveDate, String, i64)],
    as_of: NaiveDate,
) -> Option<BandDriftSignal> {
    let near_cutoff = as_of + Duration::days(NEAR_HORIZON_DAYS);
    let far: Vec<&(NaiveDate, i64)> = days.iter().filter(|(d, _)| *d >= near_cutoff).collect();

    // The first far-horizon day that closes below the lower edge.
    let crossing_date = far.iter().find(|(_, net)| *net < lower_minor)?.0;
    // The worst (lowest) far-horizon point sets the magnitude past the edge.
    let lowest = far.iter().map(|(_, net)| *net).min()?;
    let magnitude_minor = lower_minor - lowest;

    // Attribute to rising-spend categories; no risers means the crossing isn't a spending drift.
    let contributing_factors = rising_factors(spend, as_of);
    if contributing_factors.is_empty() {
        return None;
    }

    Some(BandDriftSignal {
        crossing_date,
        edge: BandEdge::Lower,
        magnitude_minor,
        contributing_factors,
    })
}

/// Per-category recent-vs-preceding monthly spend, risers only, largest rise first (ties by
/// category id for determinism), capped at [`MAX_FACTORS`]. Recent = the `WINDOW_DAYS` before
/// `as_of`; preceding = the equal window before that. Each category's monthly rate is its window
/// sum divided by the number of **distinct calendar months it actually appeared in** — so uneven
/// data (a steady category whose postings don't split evenly across the two windows) isn't read as
/// a rise; only a genuinely higher monthly rate is.
fn rising_factors(spend: &[(NaiveDate, String, i64)], as_of: NaiveDate) -> Vec<ContributingFactor> {
    let recent_start = as_of - Duration::days(WINDOW_DAYS);
    let prior_start = as_of - Duration::days(2 * WINDOW_DAYS);
    // category → (sum, distinct (year, month) it appeared in) per window.
    type Bucket<'a> = BTreeMap<&'a str, (i64, BTreeSet<(i32, u32)>)>;
    let mut recent: Bucket = BTreeMap::new();
    let mut prior: Bucket = BTreeMap::new();
    for (date, category, amount) in spend {
        let ym = (date.year(), date.month());
        if *date >= recent_start && *date < as_of {
            let e = recent.entry(category.as_str()).or_default();
            e.0 += amount;
            e.1.insert(ym);
        } else if *date >= prior_start && *date < recent_start {
            let e = prior.entry(category.as_str()).or_default();
            e.0 += amount;
            e.1.insert(ym);
        }
    }

    let monthly = |(sum, months): &(i64, BTreeSet<(i32, u32)>)| -> i64 {
        sum / i64::try_from(months.len().max(1)).unwrap_or(1)
    };
    let mut factors: Vec<ContributingFactor> = recent
        .iter()
        .filter_map(|(category, bucket)| {
            // Ignore one-offs: a baseline rise must recur across recent months.
            if bucket.1.len() < MIN_RECENT_MONTHS {
                return None;
            }
            let recent_monthly = monthly(bucket);
            let prior_monthly = prior.get(category).map_or(0, monthly);
            let delta = recent_monthly - prior_monthly;
            (delta > 0).then(|| ContributingFactor {
                category_id: (*category).to_owned(),
                recent_monthly_minor: recent_monthly,
                delta_minor: delta,
            })
        })
        .collect();
    factors.sort_by(|a, b| {
        b.delta_minor
            .cmp(&a.delta_minor)
            .then(a.category_id.cmp(&b.category_id))
    });
    factors.truncate(MAX_FACTORS);
    factors
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    /// Build a flat monthly spend series for a category: `count` postings of `amount` on the 15th
    /// of each month, ending in the month before `end` and walking backwards.
    fn monthly_spend(
        category: &str,
        amount: i64,
        months: &[&str],
    ) -> Vec<(NaiveDate, String, i64)> {
        months
            .iter()
            .map(|m| (d(&format!("{m}-15")), category.to_owned(), amount))
            .collect()
    }

    /// GOLDEN FIXTURE (5ie.8 AC): a sustained dining increase lifts the baseline, the projection
    /// crosses below the lower band on a known far-horizon date, and the signal attributes it to
    /// dining with the right magnitude — as facts, no prose.
    #[test]
    fn golden_sustained_dining_increase_attributes_the_lower_band_crossing() {
        let as_of = d("2026-07-01");
        // Projection: comfortably above the $5,000 floor near-term, dips below on Sep 20 (far).
        let days = vec![
            (d("2026-07-10"), 700_000),
            (d("2026-08-10"), 560_000),
            (d("2026-09-20"), 480_000), // first far-horizon day below the 500_000 floor
            (d("2026-10-01"), 430_000), // the worst point
        ];
        // Dining ran ~$400/mo for the prior window, then ~$700/mo in the recent 90 days; groceries
        // held steady.
        let mut spend = monthly_spend("dining", 40_000, &["2026-02", "2026-03", "2026-04"]);
        spend.extend(monthly_spend("dining", 70_000, &["2026-05", "2026-06"]));
        spend.extend(monthly_spend(
            "groceries",
            50_000,
            &["2026-02", "2026-03", "2026-04", "2026-05", "2026-06"],
        ));

        let signal = compute_band_drift_signal(&days, 500_000, &spend, as_of)
            .expect("a far-horizon below-band crossing with a rising baseline emits a signal");

        assert_eq!(signal.crossing_date, d("2026-09-20"));
        assert_eq!(signal.edge, BandEdge::Lower);
        assert_eq!(signal.magnitude_minor, 500_000 - 430_000); // depth at the worst point
                                                               // Dining is the (only) riser: recent ~700/mo vs prior ~400/mo → +~300/mo.
        assert_eq!(signal.contributing_factors.len(), 1);
        let dining = &signal.contributing_factors[0];
        assert_eq!(dining.category_id, "dining");
        assert!(dining.delta_minor > 0);
        // The structured evidence carries no prose — nothing for the advice scan to catch.
    }

    #[test]
    fn no_signal_when_the_crossing_is_near_term() {
        let as_of = d("2026-07-01");
        // Below the floor, but only 10 days out — the cover-it tool's job, not drift.
        let days = vec![(d("2026-07-11"), 400_000)];
        let spend = monthly_spend("dining", 90_000, &["2026-05", "2026-06"]);
        assert!(compute_band_drift_signal(&days, 500_000, &spend, as_of).is_none());
    }

    #[test]
    fn no_signal_when_no_category_rose() {
        let as_of = d("2026-07-01");
        let days = vec![(d("2026-09-20"), 400_000)]; // far + below
                                                     // Flat dining — no rise → the crossing is from bills/income, not a spending drift.
        let spend = monthly_spend(
            "dining",
            40_000,
            &["2026-02", "2026-03", "2026-04", "2026-05", "2026-06"],
        );
        assert!(compute_band_drift_signal(&days, 500_000, &spend, as_of).is_none());
    }

    #[test]
    fn a_one_off_recent_spike_is_not_attributed_as_a_baseline_rise() {
        let as_of = d("2026-07-01");
        let days = vec![(d("2026-09-20"), 400_000)]; // far + below
                                                     // Steady dining, plus a single large one-off "shopping" posting in one recent month only.
        let mut spend = monthly_spend(
            "dining",
            40_000,
            &["2026-02", "2026-03", "2026-04", "2026-05", "2026-06"],
        );
        spend.push((d("2026-06-10"), "shopping".to_owned(), 300_000));
        // Dining is flat (no rise) and shopping is a one-off (single month) → nothing sustained rose.
        assert!(compute_band_drift_signal(&days, 500_000, &spend, as_of).is_none());
    }

    #[test]
    fn no_signal_when_the_projection_stays_in_band() {
        let as_of = d("2026-07-01");
        let days = vec![(d("2026-09-20"), 700_000)]; // far but above the floor
        let spend = monthly_spend("dining", 90_000, &["2026-05", "2026-06"]);
        assert!(compute_band_drift_signal(&days, 500_000, &spend, as_of).is_none());
    }

    #[test]
    fn attribution_is_deterministic_and_ranked_by_rise() {
        let as_of = d("2026-07-01");
        let days = vec![(d("2026-09-20"), 400_000)];
        // Two risers: dining +larger, shopping +smaller → dining first, both present.
        let mut spend = monthly_spend("dining", 20_000, &["2026-02", "2026-03"]);
        spend.extend(monthly_spend("dining", 80_000, &["2026-05", "2026-06"]));
        spend.extend(monthly_spend("shopping", 10_000, &["2026-02", "2026-03"]));
        spend.extend(monthly_spend("shopping", 30_000, &["2026-05", "2026-06"]));

        let a = compute_band_drift_signal(&days, 500_000, &spend, as_of).unwrap();
        let b = compute_band_drift_signal(&days, 500_000, &spend, as_of).unwrap();
        assert_eq!(a, b, "same inputs → identical output");
        let cats: Vec<&str> = a
            .contributing_factors
            .iter()
            .map(|f| f.category_id.as_str())
            .collect();
        assert_eq!(cats, vec!["dining", "shopping"]); // biggest rise first
    }
}
