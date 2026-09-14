//! Recurring-obligation detection v1 (personal-cfo-98ql, plan §9.8 / §13.1).
//!
//! Pure + deterministic: given a household's realized outflows (already grouped by a
//! normalized merchant key via [`crate::normalize_merchant`]), surface the merchants that
//! recur at a consistent cadence within a stable amount band as **candidate** recurring
//! bills — each with an inferred `{amount, frequency, next date}` and a confidence. These
//! are only ever *suggestions*: the caller shows them for review and the user confirms
//! before any `recurring_event` is created (never auto-created).
//!
//! Confidence is integer basis points (no `f64`, so the whole crate stays deterministic and
//! float-free): more occurrences + tighter cadence + tighter amounts → higher confidence.

use chrono::{Duration, NaiveDate};

/// Fewest occurrences before a merchant can be a candidate — 3 gives ≥2 intervals, the
/// minimum to establish a cadence rather than infer one from a single gap.
const MIN_OCCURRENCES: usize = 3;
/// A candidate must have at least this share (bps) of its intervals matching the inferred
/// cadence, else it is too irregular to call recurring.
const MIN_CADENCE_REGULARITY_BPS: i64 = 5000;
/// The amount band around the median: an occurrence counts as on-amount within ±10%, floored
/// so tiny bills (a few dollars) still tolerate a cent or two of drift.
const AMOUNT_BAND_BPS: i64 = 1000;
const AMOUNT_BAND_FLOOR_MINOR: i64 = 500;

/// The half-width (minor units) of the amount band around `reference_minor` — an amount within
/// `±amount_band_minor` of the reference is "the same" for recurring purposes (±10%, floored at
/// $5). Shared so the detector and the suggestion-suppression re-surface rule (ADR 0046) agree on
/// what a materially-different amount is. `reference_minor` is treated as a magnitude; the i128
/// intermediate avoids overflow for very large amounts.
#[must_use]
pub fn amount_band_minor(reference_minor: i64) -> i64 {
    let scaled = i128::from(reference_minor.abs()) * i128::from(AMOUNT_BAND_BPS) / 10_000;
    i64::try_from(scaled)
        .unwrap_or(i64::MAX)
        .max(AMOUNT_BAND_FLOOR_MINOR)
}

/// One realized outflow for a merchant. `amount_minor` is the positive magnitude; the caller
/// has already normalized `merchant_key` and picked a human `display` label.
#[derive(Debug, Clone)]
pub struct Observation {
    pub merchant_key: String,
    pub display: String,
    pub date: NaiveDate,
    pub amount_minor: i64,
    pub currency: String,
}

/// A detected recurring candidate — a *suggestion* the user confirms before promotion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecurringCandidate {
    pub merchant_key: String,
    /// A representative human label (the most recent observation's display name).
    pub display: String,
    /// The representative (median) charge magnitude, minor units.
    pub amount_minor: i64,
    /// The smallest observed charge magnitude in the group (minor units) — with
    /// `amount_max_minor` this is the range the user sees before promoting a variable
    /// bill (personal-cfo-4d8.24.5).
    pub amount_min_minor: i64,
    /// The largest observed charge magnitude in the group (minor units).
    pub amount_max_minor: i64,
    pub currency: String,
    /// The inferred cadence as the app's frequency token (`weekly` … `annual`).
    pub frequency: &'static str,
    /// The most recent observed charge date (personal-cfo-4d8.24.5).
    pub last_seen: NaiveDate,
    /// The next expected charge date (last seen + one cadence interval).
    pub next_date: NaiveDate,
    pub occurrence_count: usize,
    /// Confidence in basis points (0..=10000).
    pub confidence_bps: i64,
}

/// A cadence the detector recognizes: its frequency token, the inclusive day-interval band a
/// median gap must fall in, and the canonical interval used to project the next date.
struct Cadence {
    token: &'static str,
    lo: i64,
    hi: i64,
    interval_days: i64,
}

/// Recognized cadences, widest-frequency first. Bands are disjoint; semi-monthly is omitted
/// (its ~15-day gap is indistinguishable from biweekly by interval alone in v1).
const CADENCES: &[Cadence] = &[
    Cadence {
        token: "weekly",
        lo: 6,
        hi: 8,
        interval_days: 7,
    },
    Cadence {
        token: "biweekly",
        lo: 12,
        hi: 16,
        interval_days: 14,
    },
    Cadence {
        token: "monthly",
        lo: 26,
        hi: 35,
        interval_days: 30,
    },
    Cadence {
        token: "quarterly",
        lo: 82,
        hi: 100,
        interval_days: 91,
    },
    Cadence {
        token: "annual",
        lo: 350,
        hi: 380,
        interval_days: 365,
    },
];

/// The median of a non-empty, already-sortable slice (lower-middle for an even count, so it is
/// deterministic and needs no averaging / float).
fn median_i64(sorted: &[i64]) -> i64 {
    sorted[(sorted.len() - 1) / 2]
}

/// Detect recurring candidates from `observations`, as of `today`. Groups by
/// `(merchant_key, currency)`; a group qualifies when it recurs ≥ [`MIN_OCCURRENCES`] times at a
/// recognized, majority-consistent cadence. Deterministic: output is sorted by descending
/// confidence, then merchant key, then currency.
#[must_use]
pub fn detect_recurring(observations: &[Observation], today: NaiveDate) -> Vec<RecurringCandidate> {
    // Group by (merchant_key, currency), preserving nothing order-dependent.
    let mut groups: std::collections::BTreeMap<(String, String), Vec<&Observation>> =
        std::collections::BTreeMap::new();
    for obs in observations {
        groups
            .entry((obs.merchant_key.clone(), obs.currency.clone()))
            .or_default()
            .push(obs);
    }

    let mut candidates = Vec::new();
    for ((merchant_key, currency), mut group) in groups {
        if group.len() < MIN_OCCURRENCES {
            continue;
        }
        // Chronological order for interval + next-date reasoning; ties broken by amount so the
        // ordering is total and deterministic.
        group.sort_by(|a, b| {
            a.date
                .cmp(&b.date)
                .then(a.amount_minor.cmp(&b.amount_minor))
        });

        let intervals: Vec<i64> = group
            .windows(2)
            .map(|w| (w[1].date - w[0].date).num_days())
            .collect();
        let mut sorted_intervals = intervals.clone();
        sorted_intervals.sort_unstable();
        let median_interval = median_i64(&sorted_intervals);

        let Some(cadence) = CADENCES
            .iter()
            .find(|c| median_interval >= c.lo && median_interval <= c.hi)
        else {
            continue; // no recognized cadence
        };

        // Cadence regularity: the share of intervals that fall in the cadence band.
        let matching = intervals
            .iter()
            .filter(|&&d| d >= cadence.lo && d <= cadence.hi)
            .count();
        let cadence_bps = i64::try_from(matching).unwrap_or(0) * 10_000
            / i64::try_from(intervals.len()).unwrap_or(1);
        if cadence_bps < MIN_CADENCE_REGULARITY_BPS {
            continue; // too irregular to call recurring
        }

        let mut amounts: Vec<i64> = group.iter().map(|o| o.amount_minor).collect();
        amounts.sort_unstable();
        let median_amount = median_i64(&amounts);
        let band = amount_band_minor(median_amount);
        let on_amount = amounts
            .iter()
            .filter(|&&a| (a - median_amount).abs() <= band)
            .count();
        let amount_bps = i64::try_from(on_amount).unwrap_or(0) * 10_000
            / i64::try_from(amounts.len()).unwrap_or(1);

        let n = group.len();
        // More occurrences → more confidence; 3 → 2500, 6+ → 10000.
        let count_bps = (i64::try_from(n).unwrap_or(0) - 2)
            .max(0)
            .saturating_mul(2500)
            .min(10_000);
        // Cadence regularity is the backbone; amount noise scales it down (perfect amounts →
        // no reduction, wildly varying amounts → halved).
        let confidence_bps = (count_bps * cadence_bps / 10_000) * (5_000 + amount_bps / 2) / 10_000;

        // The most recent observation drives the display label + the next-date projection.
        let last = group[group.len() - 1];
        let next_date = last.date + Duration::days(cadence.interval_days);
        // Roll a stale projection forward so a candidate built from older history still points
        // at a future date.
        let next_date = roll_forward(next_date, today, cadence.interval_days);

        candidates.push(RecurringCandidate {
            merchant_key,
            display: last.display.clone(),
            amount_minor: median_amount,
            // `amounts` is sorted ascending above, so the ends are the min/max magnitude.
            amount_min_minor: *amounts.first().unwrap_or(&median_amount),
            amount_max_minor: *amounts.last().unwrap_or(&median_amount),
            currency,
            frequency: cadence.token,
            last_seen: last.date,
            next_date,
            occurrence_count: n,
            confidence_bps,
        });
    }

    candidates.sort_by(|a, b| {
        b.confidence_bps
            .cmp(&a.confidence_bps)
            .then(a.merchant_key.cmp(&b.merchant_key))
            .then(a.currency.cmp(&b.currency))
    });
    candidates
}

/// Advance `date` by whole `interval_days` steps until it is `>= today`. Computed analytically
/// (no loop / cap) so it always reaches the future even for very stale history.
fn roll_forward(date: NaiveDate, today: NaiveDate, interval_days: i64) -> NaiveDate {
    if date >= today || interval_days <= 0 {
        return date;
    }
    // Ceil-divide the gap by the interval (both positive) without the unstable `div_ceil`.
    let gap = (today - date).num_days();
    let steps = (gap + interval_days - 1) / interval_days;
    date + Duration::days(steps * interval_days)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn obs(key: &str, date: NaiveDate, amount: i64) -> Observation {
        Observation {
            merchant_key: key.to_owned(),
            display: key.to_owned(),
            date,
            amount_minor: amount,
            currency: "USD".to_owned(),
        }
    }

    #[test]
    fn amount_range_and_last_seen_reflect_the_observations() {
        // A variable monthly bill: the amounts drift, so min < median < max, and the
        // range is what the user sees before promoting (personal-cfo-4d8.24.5).
        let amounts = [1_000, 1_099, 1_050, 1_099, 1_150, 1_099];
        let obs: Vec<_> = amounts
            .iter()
            .enumerate()
            .map(|(i, &amt)| obs("SPOTIFY", ymd(2026, 1 + i as u32, 5), amt))
            .collect();
        let c = &detect_recurring(&obs, ymd(2026, 7, 1))[0];
        assert_eq!(c.amount_min_minor, 1_000);
        assert_eq!(c.amount_max_minor, 1_150);
        assert_eq!(
            c.amount_minor, 1_099,
            "median unchanged by the new range fields"
        );
        assert_eq!(
            c.last_seen,
            ymd(2026, 6, 5),
            "the most recent observed charge"
        );
    }

    #[test]
    fn a_fixed_amount_bill_reports_an_equal_min_and_max() {
        let obs: Vec<_> = (0..6)
            .map(|i| obs("RENT", ymd(2026, 1 + i, 1), 180_000))
            .collect();
        let c = &detect_recurring(&obs, ymd(2026, 7, 1))[0];
        assert_eq!(c.amount_min_minor, 180_000);
        assert_eq!(c.amount_max_minor, 180_000);
    }

    #[test]
    fn six_months_of_spotify_is_a_high_confidence_monthly_candidate() {
        let obs: Vec<_> = (0..6)
            .map(|i| obs("SPOTIFY", ymd(2026, 1 + i, 5), 1_099))
            .collect();
        let out = detect_recurring(&obs, ymd(2026, 7, 1));
        assert_eq!(out.len(), 1);
        let c = &out[0];
        assert_eq!(c.frequency, "monthly");
        assert_eq!(c.amount_minor, 1_099);
        assert_eq!(c.occurrence_count, 6);
        assert!(
            c.confidence_bps >= 9_000,
            "clean 6-month history is high confidence"
        );
        assert!(
            c.next_date >= ymd(2026, 7, 1),
            "next date is rolled into the future"
        );
    }

    #[test]
    fn a_one_off_is_not_a_candidate() {
        let out = detect_recurring(&[obs("ACME", ymd(2026, 3, 3), 5_000)], ymd(2026, 7, 1));
        assert!(out.is_empty());
    }

    #[test]
    fn an_irregular_cadence_is_not_surfaced() {
        // Scattered gaps (5, 40, 12, 90 days) with no consistent cadence.
        let obs = vec![
            obs("RANDO", ymd(2026, 1, 1), 2_000),
            obs("RANDO", ymd(2026, 1, 6), 2_000),
            obs("RANDO", ymd(2026, 2, 15), 2_000),
            obs("RANDO", ymd(2026, 2, 27), 2_000),
            obs("RANDO", ymd(2026, 5, 27), 2_000),
        ];
        let out = detect_recurring(&obs, ymd(2026, 7, 1));
        assert!(out.is_empty(), "no consistent cadence → not surfaced");
    }

    #[test]
    fn weekly_and_amount_noise_lower_confidence_but_still_detect() {
        // Weekly gym, amounts drift a little.
        let obs = vec![
            obs("GYM", ymd(2026, 6, 1), 1_500),
            obs("GYM", ymd(2026, 6, 8), 1_600),
            obs("GYM", ymd(2026, 6, 15), 1_400),
            obs("GYM", ymd(2026, 6, 22), 1_500),
        ];
        let out = detect_recurring(&obs, ymd(2026, 7, 1));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].frequency, "weekly");
    }

    #[test]
    fn next_date_is_future_even_for_decade_stale_weekly_history() {
        // A long-dead weekly merchant: the next-date projection must still land on/after today.
        let obs: Vec<_> = (0..4)
            .map(|i| obs("OLDGYM", ymd(2005, 1, 3) + Duration::days(i * 7), 1_500))
            .collect();
        let out = detect_recurring(&obs, ymd(2026, 7, 1));
        assert_eq!(out.len(), 1);
        assert!(
            out[0].next_date >= ymd(2026, 7, 1),
            "next_date rolled to the future"
        );
    }

    #[test]
    fn same_inputs_yield_the_same_candidates() {
        let obs: Vec<_> = (0..5)
            .map(|i| obs("NETFLIX", ymd(2026, 1 + i, 20), 1_599))
            .collect();
        let a = detect_recurring(&obs, ymd(2026, 7, 1));
        let b = detect_recurring(&obs, ymd(2026, 7, 1));
        assert_eq!(a, b);
    }
}
