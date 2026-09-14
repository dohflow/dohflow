//! Pay-schedule recurrence engine (personal-cfo-82q9).
//!
//! Turns a `(frequency, anchor date)` into the **calendar pay dates** that fall
//! in a horizon. Pure and database-free; consumed by income sources
//! (`personal-cfo-j8of`) and the Future Cash forecast (`personal-cfo-164u`).
//!
//! Pay dates are [`NaiveDate`] — calendar dates with no time-of-day — so DST and
//! timezones never enter this crate. "Paid on June 5" is the same date in every
//! zone; placing that date on a wall-clock timeline (with the household
//! timezone) is the forecast's job. Month-end is clamped: a schedule anchored on
//! the 31st pays on the 30th/28th/29th in shorter months.

use chrono::{Datelike, Days, Months, NaiveDate};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A calendar pay date (no time-of-day; timezone/DST-independent).
pub type PayDate = NaiveDate;

/// Interval-frequency bounds (ADR 0048 §3): `n == 0` is invalid; upper bounds keep
/// horizons meaningful.
const MAX_EVERY_N_DAYS: u32 = 366;
const MAX_EVERY_N_WEEKS: u32 = 52;
const MAX_EVERY_N_MONTHS: u32 = 36;

/// How often a pay schedule recurs.
///
/// The regular cadences plus parameterized intervals (ADR 0048) — explicit
/// irregular / manually-listed pay dates are stored by the income source, not
/// generated here. Serialized as the flat snake_case token everywhere
/// (`"monthly"`, `"every_6_weeks"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frequency {
    /// Every 7 days from the anchor.
    Weekly,
    /// Every 14 days from the anchor.
    Biweekly,
    /// Twice a month: the **15th and the last day** of the month (the last day is
    /// clamped per month — Feb → 28/29). Two fixed days, independent of the anchor
    /// (personal-cfo-n2ek); per-source configurable days are a follow-up.
    SemiMonthly,
    /// Same day-of-month as the anchor, every month (month-end clamped).
    Monthly,
    /// Same day-of-month as the anchor, every 3 months (month-end clamped).
    Quarterly,
    /// Same day-of-month as the anchor, every 12 months (Feb 29 → Feb 28).
    Annual,
    /// Every `n` days from the anchor (ADR 0048; `1..=366`).
    EveryNDays(u32),
    /// Every `n` weeks (`7·n` days) from the anchor (ADR 0048; `1..=52`).
    EveryNWeeks(u32),
    /// The anchor's day-of-month every `n` months, month-end clamped
    /// (ADR 0048; `1..=36`).
    EveryNMonths(u32),
}

impl Frequency {
    /// The stable snake_case token, for database persistence and the IPC wire
    /// (`"monthly"`, `"every_6_weeks"`). Matches the serde representation.
    #[must_use]
    pub fn token(self) -> String {
        match self {
            Frequency::Weekly => "weekly".to_owned(),
            Frequency::Biweekly => "biweekly".to_owned(),
            Frequency::SemiMonthly => "semi_monthly".to_owned(),
            Frequency::Monthly => "monthly".to_owned(),
            Frequency::Quarterly => "quarterly".to_owned(),
            Frequency::Annual => "annual".to_owned(),
            Frequency::EveryNDays(n) => format!("every_{n}_days"),
            Frequency::EveryNWeeks(n) => format!("every_{n}_weeks"),
            Frequency::EveryNMonths(n) => format!("every_{n}_months"),
        }
    }

    /// Parse a [`token`](Frequency::token) string back into a [`Frequency`].
    /// Parameterized tokens are bounds-checked (ADR 0048 §3): `every_0_days`,
    /// malformed counts, and out-of-range intervals parse to `None`.
    #[must_use]
    pub fn from_token(token: &str) -> Option<Self> {
        match token {
            "weekly" => Some(Frequency::Weekly),
            "biweekly" => Some(Frequency::Biweekly),
            "semi_monthly" => Some(Frequency::SemiMonthly),
            "monthly" => Some(Frequency::Monthly),
            "quarterly" => Some(Frequency::Quarterly),
            "annual" => Some(Frequency::Annual),
            _ => {
                let rest = token.strip_prefix("every_")?;
                let (count, unit) = rest.split_once('_')?;
                let n: u32 = count.parse().ok()?;
                match unit {
                    "days" if (1..=MAX_EVERY_N_DAYS).contains(&n) => Some(Frequency::EveryNDays(n)),
                    "weeks" if (1..=MAX_EVERY_N_WEEKS).contains(&n) => {
                        Some(Frequency::EveryNWeeks(n))
                    }
                    "months" if (1..=MAX_EVERY_N_MONTHS).contains(&n) => {
                        Some(Frequency::EveryNMonths(n))
                    }
                    _ => None,
                }
            }
        }
    }
}

// Serde as the flat token string in both directions, so the six classic tokens keep
// their exact pre-ADR-0048 serialized form and parameterized tokens stay flat
// (`"every_6_weeks"`, never an object).
impl Serialize for Frequency {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.token())
    }
}

impl<'de> Deserialize<'de> for Frequency {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let token = String::deserialize(deserializer)?;
        Frequency::from_token(&token)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown frequency token: {token}")))
    }
}

/// A recurring pay schedule: a cadence anchored on a known pay date.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaySchedule {
    /// The recurrence cadence.
    pub frequency: Frequency,
    /// A known pay date the cadence is measured from (may be in the past).
    pub anchor: NaiveDate,
}

impl PaySchedule {
    /// Build a schedule.
    #[must_use]
    pub const fn new(frequency: Frequency, anchor: NaiveDate) -> Self {
        Self { frequency, anchor }
    }

    /// All pay dates in the inclusive `[from, to]` horizon, ascending and
    /// deduplicated. Empty when `from > to`. Deterministic: identical inputs
    /// always yield an identical `Vec`.
    #[must_use]
    pub fn pay_dates(&self, from: NaiveDate, to: NaiveDate) -> Vec<PayDate> {
        if from > to {
            return Vec::new();
        }
        match self.frequency {
            Frequency::Weekly => self.fixed_interval_dates(7, from, to),
            Frequency::Biweekly => self.fixed_interval_dates(14, from, to),
            Frequency::SemiMonthly => self.semimonthly_dates(from, to),
            Frequency::Monthly => self.monthly_step_dates(1, from, to),
            Frequency::Quarterly => self.monthly_step_dates(3, from, to),
            Frequency::Annual => self.monthly_step_dates(12, from, to),
            // ADR 0048 §2: intervals reuse the two existing generators — no new
            // date math. `.max(1)` is defense in depth; `from_token` rejects 0.
            Frequency::EveryNDays(n) => self.fixed_interval_dates(i64::from(n.max(1)), from, to),
            Frequency::EveryNWeeks(n) => {
                self.fixed_interval_dates(7 * i64::from(n.max(1)), from, to)
            }
            Frequency::EveryNMonths(n) => self.monthly_step_dates(i64::from(n.max(1)), from, to),
        }
    }

    /// The first pay date on or after `on_or_after`, if one is representable
    /// within ~13 months (covers every cadence). Used by the income UI to show
    /// the next paycheck.
    #[must_use]
    pub fn next_pay_date(&self, on_or_after: NaiveDate) -> Option<PayDate> {
        let to = on_or_after.checked_add_months(Months::new(13))?;
        self.pay_dates(on_or_after, to).into_iter().next()
    }

    /// Fixed-day-interval cadences (weekly, biweekly). The first occurrence on or
    /// after `from` is found in O(1) via floor division, then enumerated.
    fn fixed_interval_dates(&self, interval: i64, from: NaiveDate, to: NaiveDate) -> Vec<PayDate> {
        let mut out = Vec::new();
        // Align to the latest occurrence on or before `from`, then step forward.
        let k = (from - self.anchor).num_days().div_euclid(interval);
        let Some(mut date) = shift_days(self.anchor, k * interval) else {
            return out;
        };
        while date < from {
            match shift_days(date, interval) {
                Some(next) => date = next,
                None => return out,
            }
        }
        while date <= to {
            out.push(date);
            match shift_days(date, interval) {
                Some(next) => date = next,
                None => break,
            }
        }
        out
    }

    /// Month-stepping cadences (monthly, quarterly, annual). Each occurrence is
    /// recomputed from the anchor (`anchor + k·step months`) so month-end intent
    /// is preserved — a 31st schedule keeps trying the 31st, clamped per month,
    /// rather than sticking at a once-clamped day.
    fn monthly_step_dates(&self, step: i64, from: NaiveDate, to: NaiveDate) -> Vec<PayDate> {
        let mut out = Vec::new();
        // Estimate the starting multiple, then correct by a step or two.
        let months_from_anchor = i64::from(
            (from.year() - self.anchor.year()) * 12
                + (from.month() as i32 - self.anchor.month() as i32),
        );
        let mut k = months_from_anchor.div_euclid(step) - 1;
        let Some(mut date) = shift_months(self.anchor, k * step) else {
            return out;
        };
        while date < from {
            k += 1;
            match shift_months(self.anchor, k * step) {
                Some(next) => date = next,
                None => return out,
            }
        }
        while date <= to {
            out.push(date);
            k += 1;
            match shift_months(self.anchor, k * step) {
                Some(next) => date = next,
                None => break,
            }
        }
        out
    }

    /// Semimonthly: the **15th and the last day** of each month (personal-cfo-n2ek).
    /// These are two fixed days, *not* "anchor day + 15" — the old model produced
    /// duplicate/late-month dates for a near-month-end anchor (a 30th anchor gave
    /// `30 + (30+15 clamped to last)`). The anchor no longer affects the pay days;
    /// per-source configurable days (e.g. 1st + 15th) are a follow-up.
    fn semimonthly_dates(&self, from: NaiveDate, to: NaiveDate) -> Vec<PayDate> {
        let mut out = Vec::new();
        // Walk month-by-month from `from`'s month through `to`.
        let Some(mut month_start) = NaiveDate::from_ymd_opt(from.year(), from.month(), 1) else {
            return out;
        };
        while month_start <= to {
            let (year, month) = (month_start.year(), month_start.month());
            let Some(last) = last_day_of_month(year, month) else {
                break;
            };
            for day in [15, last] {
                if let Some(date) = NaiveDate::from_ymd_opt(year, month, day) {
                    if date >= from && date <= to {
                        out.push(date);
                    }
                }
            }
            match month_start.checked_add_months(Months::new(1)) {
                Some(next) => month_start = next,
                None => break,
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// Shift a date by a signed number of days (negative steps backward).
fn shift_days(date: NaiveDate, days: i64) -> Option<NaiveDate> {
    if days >= 0 {
        date.checked_add_days(Days::new(days as u64))
    } else {
        date.checked_sub_days(Days::new(days.unsigned_abs()))
    }
}

/// Shift a date by a signed number of months (month-end clamped by chrono).
fn shift_months(date: NaiveDate, months: i64) -> Option<NaiveDate> {
    let magnitude = u32::try_from(months.unsigned_abs()).ok()?;
    if months >= 0 {
        date.checked_add_months(Months::new(magnitude))
    } else {
        date.checked_sub_months(Months::new(magnitude))
    }
}

/// The last day-of-month for `(year, month)`.
fn last_day_of_month(year: i32, month: u32) -> Option<u32> {
    let first = NaiveDate::from_ymd_opt(year, month, 1)?;
    let next = first.checked_add_months(Months::new(1))?;
    Some(next.pred_opt()?.day())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    /// ADR 0047 §3: the anchor is "a real occurrence the cadence counts from" — ANY
    /// occurrence of the series is an equivalent anchor. Sweep every frequency across a
    /// grid of anchors (incl. month-end clamp territory) and assert the expanded lattice
    /// is identical when the anchor advances by whole intervals — past or future — so a
    /// candidate promoted on its last OBSERVED date projects exactly as one promoted on
    /// its next EXPECTED date.
    #[test]
    fn any_occurrence_is_an_equivalent_anchor() {
        let start = d(2025, 11, 1);
        let end = d(2027, 3, 1);
        for freq in [
            Frequency::Weekly,
            Frequency::Biweekly,
            Frequency::SemiMonthly,
            Frequency::Monthly,
            Frequency::Quarterly,
            Frequency::Annual,
        ] {
            for anchor_day in [1u32, 15, 28, 29, 30, 31] {
                let Some(anchor) = NaiveDate::from_ymd_opt(2026, 1, anchor_day) else {
                    continue;
                };
                let base = PaySchedule::new(freq, anchor).pay_dates(start, end);
                // Advance the anchor to each of the first few occurrences it generated:
                // every UNCLAMPED one must reproduce the same lattice. (A month-end
                // CLAMPED occurrence — e.g. a day-29 schedule's Feb 28 — carries a
                // different day-of-month, so re-anchoring on it legitimately shifts the
                // series; ADR 0047 §3 documents that caveat.)
                // Day-interval frequencies are interval-exact (every occurrence is an
                // equivalent anchor); month-stepped ones require the same day-of-month.
                let day_preserving = matches!(
                    freq,
                    Frequency::Weekly | Frequency::Biweekly | Frequency::SemiMonthly
                );
                for shifted_anchor in base
                    .iter()
                    .filter(|date| day_preserving || date.day() == anchor.day())
                    .take(4)
                {
                    let shifted = PaySchedule::new(freq, *shifted_anchor).pay_dates(start, end);
                    assert_eq!(
                        base, shifted,
                        "{freq:?} anchored {anchor} vs {shifted_anchor}: lattices must match"
                    );
                }
            }
        }
    }

    #[test]
    fn weekly_and_biweekly_step_from_the_anchor() {
        let weekly = PaySchedule::new(Frequency::Weekly, d(2026, 6, 5));
        assert_eq!(
            weekly.pay_dates(d(2026, 6, 1), d(2026, 6, 30)),
            vec![
                d(2026, 6, 5),
                d(2026, 6, 12),
                d(2026, 6, 19),
                d(2026, 6, 26)
            ],
        );

        let biweekly = PaySchedule::new(Frequency::Biweekly, d(2026, 6, 5));
        assert_eq!(
            biweekly.pay_dates(d(2026, 6, 1), d(2026, 7, 5)),
            vec![d(2026, 6, 5), d(2026, 6, 19), d(2026, 7, 3)],
        );
    }

    #[test]
    fn anchor_in_the_past_aligns_into_the_horizon() {
        let weekly = PaySchedule::new(Frequency::Weekly, d(2020, 1, 1));
        // 2020-01-01 is a Wednesday; +7-day steps land on Wednesdays.
        assert_eq!(
            weekly.pay_dates(d(2026, 6, 1), d(2026, 6, 21)),
            vec![d(2026, 6, 3), d(2026, 6, 10), d(2026, 6, 17)],
        );
    }

    #[test]
    fn monthly_clamps_month_end() {
        let monthly = PaySchedule::new(Frequency::Monthly, d(2026, 1, 31));
        assert_eq!(
            monthly.pay_dates(d(2026, 1, 1), d(2026, 4, 30)),
            vec![
                d(2026, 1, 31),
                d(2026, 2, 28),
                d(2026, 3, 31),
                d(2026, 4, 30)
            ],
        );
    }

    #[test]
    fn annual_handles_leap_day() {
        let annual = PaySchedule::new(Frequency::Annual, d(2024, 2, 29));
        assert_eq!(
            annual.pay_dates(d(2024, 1, 1), d(2027, 12, 31)),
            vec![
                d(2024, 2, 29),
                d(2025, 2, 28),
                d(2026, 2, 28),
                d(2027, 2, 28)
            ],
        );
    }

    #[test]
    fn quarterly_steps_three_months() {
        let quarterly = PaySchedule::new(Frequency::Quarterly, d(2026, 1, 15));
        assert_eq!(
            quarterly.pay_dates(d(2026, 1, 1), d(2026, 12, 31)),
            vec![
                d(2026, 1, 15),
                d(2026, 4, 15),
                d(2026, 7, 15),
                d(2026, 10, 15)
            ],
        );
    }

    #[test]
    fn semimonthly_pays_the_15th_and_last_day() {
        let s = PaySchedule::new(Frequency::SemiMonthly, d(2026, 6, 1));
        assert_eq!(
            s.pay_dates(d(2026, 6, 1), d(2026, 7, 31)),
            vec![
                d(2026, 6, 15),
                d(2026, 6, 30),
                d(2026, 7, 15),
                d(2026, 7, 31),
            ],
        );

        // The anchor day no longer changes the pay days — always the 15th + last.
        let anchored_late = PaySchedule::new(Frequency::SemiMonthly, d(2026, 6, 22));
        assert_eq!(
            anchored_late.pay_dates(d(2026, 6, 1), d(2026, 6, 30)),
            vec![d(2026, 6, 15), d(2026, 6, 30)],
        );
    }

    #[test]
    fn semimonthly_near_month_end_anchor_does_not_duplicate() {
        // Regression (personal-cfo-n2ek): a near-month-end anchor previously gave
        // "30th + (30 + 15 clamped to last)" → duplicate/late dates ("Jul 30, Jul
        // 31 again"). Now every full month is exactly the 15th + the last day.
        let s = PaySchedule::new(Frequency::SemiMonthly, d(2026, 6, 30));
        assert_eq!(
            s.pay_dates(d(2026, 6, 16), d(2026, 8, 31)),
            vec![
                d(2026, 6, 30),
                d(2026, 7, 15),
                d(2026, 7, 31),
                d(2026, 8, 15),
                d(2026, 8, 31),
            ],
        );
    }

    #[test]
    fn semimonthly_in_february_is_the_15th_and_the_last_day() {
        let s = PaySchedule::new(Frequency::SemiMonthly, d(2026, 1, 1));
        // 2026 is not a leap year → Feb 28.
        assert_eq!(
            s.pay_dates(d(2026, 2, 1), d(2026, 2, 28)),
            vec![d(2026, 2, 15), d(2026, 2, 28)],
        );
    }

    #[test]
    fn empty_horizon_and_inverted_range() {
        let weekly = PaySchedule::new(Frequency::Weekly, d(2026, 6, 5));
        assert!(weekly.pay_dates(d(2026, 7, 1), d(2026, 6, 1)).is_empty());
        // A horizon with no occurrence.
        assert!(weekly.pay_dates(d(2026, 6, 6), d(2026, 6, 11)).is_empty());
    }

    #[test]
    fn next_pay_date_finds_the_upcoming_occurrence() {
        let monthly = PaySchedule::new(Frequency::Monthly, d(2026, 1, 15));
        assert_eq!(monthly.next_pay_date(d(2026, 6, 16)), Some(d(2026, 7, 15)));
        assert_eq!(monthly.next_pay_date(d(2026, 6, 15)), Some(d(2026, 6, 15)));
    }

    #[test]
    fn output_is_deterministic() {
        let s = PaySchedule::new(Frequency::Biweekly, d(2026, 3, 13));
        assert_eq!(
            s.pay_dates(d(2026, 1, 1), d(2026, 12, 31)),
            s.pay_dates(d(2026, 1, 1), d(2026, 12, 31)),
        );
    }

    // ===== Interval frequencies (ADR 0048, personal-cfo-4d8.25.14) =====

    /// every_n_days steps exactly n days; alignment from a past anchor works like weekly.
    #[test]
    fn every_n_days_steps_exactly() {
        let s = PaySchedule::new(Frequency::EveryNDays(45), d(2026, 1, 10));
        assert_eq!(
            s.pay_dates(d(2026, 1, 1), d(2026, 6, 30)),
            vec![
                d(2026, 1, 10),
                d(2026, 2, 24),
                d(2026, 4, 10),
                d(2026, 5, 25)
            ],
        );
        // Past-anchor alignment via floor division, like weekly.
        let aligned = PaySchedule::new(Frequency::EveryNDays(45), d(2024, 1, 10));
        let dates = aligned.pay_dates(d(2026, 1, 1), d(2026, 6, 30));
        assert!(!dates.is_empty());
        for pair in dates.windows(2) {
            assert_eq!((pair[1] - pair[0]).num_days(), 45);
        }
    }

    /// every_n_weeks is 7·n days — the every-6-weeks bill from the owner feedback.
    #[test]
    fn every_n_weeks_is_seven_n_days() {
        let s = PaySchedule::new(Frequency::EveryNWeeks(6), d(2026, 1, 2));
        assert_eq!(
            s.pay_dates(d(2026, 1, 1), d(2026, 7, 1)),
            vec![
                d(2026, 1, 2),
                d(2026, 2, 13),
                d(2026, 3, 27),
                d(2026, 5, 8),
                d(2026, 6, 19)
            ],
        );
    }

    /// every_n_months keeps the anchor's day-of-month with the same month-end clamp
    /// discipline as monthly: a day-31 anchor keeps trying the 31st.
    #[test]
    fn every_n_months_clamps_month_end_like_monthly() {
        let s = PaySchedule::new(Frequency::EveryNMonths(2), d(2025, 12, 31));
        assert_eq!(
            s.pay_dates(d(2025, 12, 1), d(2026, 8, 31)),
            vec![
                d(2025, 12, 31),
                d(2026, 2, 28),
                d(2026, 4, 30),
                d(2026, 6, 30),
                d(2026, 8, 31),
            ],
        );
    }

    /// Degenerate aliases expand identically to their classic counterparts (ADR 0048 §3:
    /// permitted, not normalized).
    #[test]
    fn degenerate_aliases_match_the_classic_lattices() {
        let horizon = (d(2026, 1, 1), d(2026, 12, 31));
        for (alias, classic) in [
            (Frequency::EveryNDays(7), Frequency::Weekly),
            (Frequency::EveryNDays(14), Frequency::Biweekly),
            (Frequency::EveryNWeeks(1), Frequency::Weekly),
            (Frequency::EveryNWeeks(2), Frequency::Biweekly),
            (Frequency::EveryNMonths(1), Frequency::Monthly),
            (Frequency::EveryNMonths(3), Frequency::Quarterly),
            (Frequency::EveryNMonths(12), Frequency::Annual),
        ] {
            let anchor = d(2026, 1, 31);
            assert_eq!(
                PaySchedule::new(alias, anchor).pay_dates(horizon.0, horizon.1),
                PaySchedule::new(classic, anchor).pay_dates(horizon.0, horizon.1),
                "{alias:?} must expand exactly like {classic:?}"
            );
        }
    }

    /// ADR 0047 §3 anchor equivalence extends to the interval variants: any unclamped
    /// occurrence re-anchors to the identical lattice.
    #[test]
    fn interval_frequencies_keep_anchor_equivalence() {
        let start = d(2025, 11, 1);
        let end = d(2027, 3, 1);
        for freq in [
            Frequency::EveryNDays(45),
            Frequency::EveryNWeeks(6),
            Frequency::EveryNMonths(2),
        ] {
            let day_preserving = !matches!(freq, Frequency::EveryNMonths(_));
            for anchor_day in [1u32, 15, 30, 31] {
                let Some(anchor) = NaiveDate::from_ymd_opt(2026, 1, anchor_day) else {
                    continue;
                };
                let base = PaySchedule::new(freq, anchor).pay_dates(start, end);
                for shifted_anchor in base
                    .iter()
                    .filter(|date| day_preserving || date.day() == anchor.day())
                    .take(4)
                {
                    let shifted = PaySchedule::new(freq, *shifted_anchor).pay_dates(start, end);
                    assert_eq!(
                        base, shifted,
                        "{freq:?} anchored {anchor} vs {shifted_anchor}: lattices must match"
                    );
                }
            }
        }
    }

    /// Token round-trip for both families; bounds and malformed forms rejected.
    #[test]
    fn token_round_trip_and_bounds() {
        for freq in [
            Frequency::Weekly,
            Frequency::Biweekly,
            Frequency::SemiMonthly,
            Frequency::Monthly,
            Frequency::Quarterly,
            Frequency::Annual,
            Frequency::EveryNDays(45),
            Frequency::EveryNWeeks(6),
            Frequency::EveryNMonths(2),
        ] {
            assert_eq!(Frequency::from_token(&freq.token()), Some(freq));
        }
        for bad in [
            "every_0_days",
            "every_367_days",
            "every_53_weeks",
            "every_37_months",
            "every__weeks",
            "every_6_fortnights",
            "every_6weeks",
            "every_-2_days",
            "fortnightly",
        ] {
            assert_eq!(Frequency::from_token(bad), None, "{bad} must not parse");
        }
    }

    /// Serde stays the flat token string in both directions — classic tokens keep their
    /// exact pre-ADR-0048 form; parameterized tokens are flat strings, never objects.
    #[test]
    fn serde_is_the_flat_token_string() {
        assert_eq!(
            serde_json::to_string(&Frequency::Monthly).unwrap(),
            "\"monthly\""
        );
        assert_eq!(
            serde_json::to_string(&Frequency::EveryNWeeks(6)).unwrap(),
            "\"every_6_weeks\""
        );
        assert_eq!(
            serde_json::from_str::<Frequency>("\"every_45_days\"").unwrap(),
            Frequency::EveryNDays(45)
        );
        assert!(serde_json::from_str::<Frequency>("\"every_0_days\"").is_err());
        // The schedule struct embeds the same flat form.
        let s = PaySchedule::new(Frequency::EveryNMonths(2), d(2026, 1, 31));
        assert_eq!(
            serde_json::to_string(&s).unwrap(),
            "{\"frequency\":\"every_2_months\",\"anchor\":\"2026-01-31\"}"
        );
    }
}
