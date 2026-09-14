//! Deterministic Future Cash ledger — Layer 1 (plan §13.3, personal-cfo-164u).
//!
//! [`forecast_layer1`] folds a stream of dated, signed cash events over a
//! starting balance into a **per-day balance series**. It is the deterministic
//! core of DohFlow's forecast: the product wedge that turns the
//! obligations a household has entered (paychecks, rent, subscriptions, loan
//! payments) into "how much cash will I have on each day for the next N days".
//!
//! # Purity (enforced at the crate boundary)
//! This crate is a pure function over value types. It performs **no** database
//! access, IO, network, RNG, or clock reads — none of those crates are even in
//! its dependency tree (CI's "pure-types crate boundaries" check rejects
//! `tokio`/`tauri`/`rusqlite`). The caller reads the wall clock once and hands
//! the resulting instant in via [`Horizon::as_of`]; the fold itself only does
//! integer money arithmetic and calendar-date stepping. That is what makes the
//! output **deterministic**: identical inputs always yield byte-identical
//! output (asserted by the `insta` snapshot test).
//!
//! # Same-day ordering
//! When several events land on the same calendar day the running balance must
//! be applied in a fixed order, or the intra-day balances (and any min-cash
//! reasoning downstream) would depend on input order. The order is
//! **income → bills → transfers → manual one-offs** ([`EventKind::priority`]),
//! with `(source_event_id, amount)` as a final deterministic tie-break. Two
//! distinct events therefore always sort to a fixed position regardless of the
//! order they were supplied in (the "commutativity" property test).
//!
//! # Timezone / DST
//! Every event carries a **local calendar date** ([`ForecastEvent::occurs_on`]) —
//! the pay-schedule engine and the commitments projection both already produce
//! timezone-free `NaiveDate`s, because "due on June 5" is the same date in every
//! zone. The fold therefore **buckets by local calendar date**, which makes it
//! immune to DST by construction: a bill scheduled for 00:30 local on a
//! spring-forward day still falls on that local date. (Converting each event to
//! a UTC instant and bucketing by UTC day — one reading of the original design
//! sketch — would *reintroduce* that bug, so we deliberately do not.) The
//! household [`Tz`] is load-bearing in exactly one place: resolving the
//! horizon's `as_of` instant to the household-local **start date**, so "today"
//! and the horizon window follow the household zone rather than the machine's
//! local zone (personal-cfo-rp1r).
//!
//! # Layer 2 (`layer2`)
//! The statistical spend model that widens Layer-1's deterministic line into a P10/P50/P90
//! band learned from history (ADR 0026 §7, personal-cfo-9h1s) lives in [`layer2`].

pub mod layer2;
pub mod payoff;
pub mod revolving;

use chrono::{DateTime, Days, NaiveDate, Utc};
use core_money::{Currency, Money, MoneyError};
use thiserror::Error;
use uuid::Uuid;

/// Re-exported so callers can name the household timezone type without taking a
/// direct `chrono-tz` dependency.
pub use chrono_tz::Tz;

/// What kind of cash event a [`ForecastEvent`] is. Drives the same-day ordering
/// and maps 1:1 to the persisted `forecast_rows.source_type` enum (§9.10), so a
/// row's provenance survives into the database unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventKind {
    /// A paycheck / income deposit (a positive cashflow).
    Income,
    /// A recurring bill occurrence (rent, subscription, utility, …).
    RecurringBill,
    /// A scheduled loan / debt payment.
    LoanPayment,
    /// A scheduled transfer between accounts.
    Transfer,
    /// A one-off manually entered future event.
    ManualOneOff,
}

impl EventKind {
    /// Same-day tie-break priority: **income (0) < bills (1) < transfers (2) <
    /// manual (3)**. Lower applies first within a calendar day. `RecurringBill`
    /// and `LoanPayment` share the "bills" rank; `(source_event_id, amount)`
    /// breaks any remaining tie deterministically.
    #[must_use]
    pub const fn priority(self) -> u8 {
        match self {
            EventKind::Income => 0,
            EventKind::RecurringBill | EventKind::LoanPayment => 1,
            EventKind::Transfer => 2,
            EventKind::ManualOneOff => 3,
        }
    }

    /// The persisted `forecast_rows.source_type` token (schema enum, §9.10).
    #[must_use]
    pub const fn source_type_token(self) -> &'static str {
        match self {
            EventKind::Income => "income",
            EventKind::RecurringBill => "recurring_bill",
            EventKind::LoanPayment => "loan_payment",
            EventKind::Transfer => "transfer",
            EventKind::ManualOneOff => "manual_entry",
        }
    }
}

/// Why an event is in the forecast — the evidence a downstream "explain this
/// row" feature (personal-cfo-vkge) renders without re-deriving it. Carried on
/// every [`DayEvent`] so the explanation needs no second pass over the inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssumptionBasis {
    /// Projected from a recurring schedule; carries the typed cadence — rendered
    /// to its token (`"monthly"`, `"every_6_weeks"`) only at the display/DTO edge
    /// (ADR 0048).
    RecurringSchedule {
        /// The pay-schedule cadence.
        frequency: pay_schedule::Frequency,
    },
    /// A single, non-recurring future entry the user typed in.
    ManualOneOff,
}

/// A single dated cash event to fold into the forecast.
///
/// `amount` is **signed** in the forecast currency: inflows positive (income),
/// outflows negative (bills, loan payments). The engine treats the sign as
/// authoritative and `kind` as ordering/provenance metadata — it does not
/// re-derive the sign from the kind, so transfers and manual entries may be
/// either direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForecastEvent {
    /// The household-local calendar date the event occurs on.
    pub occurs_on: NaiveDate,
    /// What kind of event this is (drives same-day ordering + provenance).
    pub kind: EventKind,
    /// Signed amount in the forecast currency (inflow positive, outflow
    /// negative). Must match the starting balance's currency.
    pub amount: Money,
    /// The id of the source entity this occurrence was projected from (a
    /// commitment / income source / manual entry id). Provenance for the UI and
    /// a deterministic same-day tie-break.
    pub source_event_id: Uuid,
    /// Why this event is assumed (for row explanation).
    pub assumption_basis: AssumptionBasis,
}

/// One applied event as recorded on a [`DailyBalance`] — the evidence behind
/// that day's balance change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DayEvent {
    /// The source entity id (see [`ForecastEvent::source_event_id`]).
    pub source_event_id: Uuid,
    /// The event kind.
    pub kind: EventKind,
    /// The signed amount applied to the running balance.
    pub amount: Money,
    /// Why this event is assumed.
    pub assumption_basis: AssumptionBasis,
}

/// The forecast window: an "as of" instant the caller stamps from the wall clock
/// (so the engine stays clock-free) plus a length in calendar days.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Horizon {
    /// The instant the forecast is computed "as of". Resolved to the
    /// household-local **start date** via the [`Tz`] (the first emitted day).
    pub as_of: DateTime<Utc>,
    /// Number of calendar days to project, **inclusive** of the start day. A
    /// 30-day horizon emits 30 rows. Zero emits none.
    pub days: u32,
}

impl Horizon {
    /// Build a horizon.
    #[must_use]
    pub const fn new(as_of: DateTime<Utc>, days: u32) -> Self {
        Self { as_of, days }
    }

    /// The inclusive local-date window `[start, end]` this horizon covers in
    /// `tz` — `start` is the household-local "today" resolved from [`as_of`],
    /// `end` is `start + days - 1`. Returns `None` when `days` is zero.
    ///
    /// Both the fold and the DB input adapter resolve their date range through
    /// this one method, so the days the engine emits and the days the schedules
    /// are expanded over can never disagree.
    ///
    /// [`as_of`]: Horizon::as_of
    #[must_use]
    pub fn window(&self, tz: Tz) -> Option<(NaiveDate, NaiveDate)> {
        if self.days == 0 {
            return None;
        }
        let start = self.as_of.with_timezone(&tz).date_naive();
        // The horizon length is caller-bounded; the saturate only guards the
        // absurd year-9999 calendar edge (no real forecast reaches it).
        let end = start
            .checked_add_days(Days::new(u64::from(self.days - 1)))
            .unwrap_or(NaiveDate::MAX);
        Some((start, end))
    }
}

/// A forecast value as a confidence band — the 10th / 50th / 90th percentile of
/// the projected balance (ADR 0026 §1). Layer-1 is deterministic, so it emits a
/// **collapsed** band (`p10 == p50 == p90`) via [`Band::point`]; the statistical
/// layers widen it later without changing this shape, so the chart, IPC DTO, and
/// persistence are built once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Band {
    /// 10th-percentile (pessimistic) projected balance.
    pub p10: Money,
    /// 50th-percentile (median / deterministic point) projected balance.
    pub p50: Money,
    /// 90th-percentile (optimistic) projected balance.
    pub p90: Money,
}

impl Band {
    /// A collapsed band — all three percentiles equal — for a deterministic point
    /// estimate (Layer 1). The statistical layers replace this with a real spread.
    #[must_use]
    pub const fn point(value: Money) -> Self {
        Self {
            p10: value,
            p50: value,
            p90: value,
        }
    }
}

/// One day of the forecast: the closing cash balance (as a band) and the events
/// that moved it. Emitted for **every** day in the horizon — days with no events
/// still appear, carrying the previous balance forward (empty `events`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DailyBalance {
    /// The household-local calendar date.
    pub date: NaiveDate,
    /// Projected cash balance at end of day, as a P10/P50/P90 band (collapsed for
    /// the deterministic Layer-1 engine: all three percentiles equal).
    pub closing: Band,
    /// The events applied on this day, in the canonical same-day order.
    pub events: Vec<DayEvent>,
}

/// Why a forecast could not be computed. The happy path is a clean balance
/// series; these are the (rare) money-arithmetic failures, surfaced rather than
/// silently swallowed — financial code must never drop or mis-sum an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ForecastError {
    /// An event's currency did not match the starting balance's currency.
    /// Cross-currency forecasting is out of scope for Layer 1 (a converter is a
    /// later layer); the caller must pre-convert or partition by currency.
    #[error("event currency {found} does not match forecast currency {expected}")]
    CurrencyMismatch {
        /// The forecast currency (from the starting balance).
        expected: Currency,
        /// The offending event's currency.
        found: Currency,
    },
    /// The running balance overflowed `i64` minor units.
    #[error("forecast arithmetic overflowed the i64 minor-unit range")]
    Overflow,
}

impl From<MoneyError> for ForecastError {
    fn from(error: MoneyError) -> Self {
        match error {
            MoneyError::CurrencyMismatch { left, right } => ForecastError::CurrencyMismatch {
                expected: left,
                found: right,
            },
            // Overflow is the only other failure `checked_add` can return here;
            // any future variant collapses to the same "cannot compute" signal.
            _ => ForecastError::Overflow,
        }
    }
}

/// Project the Layer-1 deterministic Future Cash balance series.
///
/// Folds `events` over `starting_balance` across the `horizon`, one row per
/// calendar day in the household `tz`. See the [module docs](crate) for the
/// purity, ordering, and timezone guarantees.
///
/// # Errors
/// Returns [`ForecastError::CurrencyMismatch`] if any in-horizon event's
/// currency differs from `starting_balance`'s, or [`ForecastError::Overflow`]
/// if the running balance exceeds the `i64` minor-unit range.
pub fn forecast_layer1(
    events: &[ForecastEvent],
    starting_balance: Money,
    horizon: Horizon,
    tz: Tz,
) -> Result<Vec<DailyBalance>, ForecastError> {
    // Resolve the local-date window once (shared with the DB input adapter).
    let Some((start, end)) = horizon.window(tz) else {
        return Ok(Vec::new());
    };
    let currency = starting_balance.currency();

    // Keep only events inside the window, validating currency up front so the
    // fold's arithmetic cannot mix currencies.
    let mut scheduled: Vec<&ForecastEvent> = Vec::with_capacity(events.len());
    for event in events {
        if event.occurs_on < start || event.occurs_on > end {
            continue;
        }
        if event.amount.currency() != currency {
            return Err(ForecastError::CurrencyMismatch {
                expected: currency,
                found: event.amount.currency(),
            });
        }
        scheduled.push(event);
    }

    // Canonical order: by day, then same-day priority, then a total tie-break on
    // (source id, amount) so distinct events never depend on input order.
    scheduled.sort_by(|a, b| {
        a.occurs_on
            .cmp(&b.occurs_on)
            .then_with(|| a.kind.priority().cmp(&b.kind.priority()))
            .then_with(|| a.source_event_id.cmp(&b.source_event_id))
            .then_with(|| a.amount.minor_units().cmp(&b.amount.minor_units()))
    });

    // Single forward pass: walk every calendar day, draining the events that
    // land on it, and emit the closing balance (carried forward on empty days).
    let mut rows = Vec::with_capacity(horizon.days as usize);
    let mut running = starting_balance;
    let mut next = 0usize;
    let mut day = start;
    for _ in 0..horizon.days {
        let mut applied = Vec::new();
        while let Some(event) = scheduled.get(next) {
            if event.occurs_on != day {
                break;
            }
            running = running.checked_add(event.amount)?;
            applied.push(DayEvent {
                source_event_id: event.source_event_id,
                kind: event.kind,
                amount: event.amount,
                assumption_basis: event.assumption_basis,
            });
            next += 1;
        }
        rows.push(DailyBalance {
            date: day,
            closing: Band::point(running),
            events: applied,
        });
        // The final iteration may sit on the calendar's last representable day;
        // there is no row after it, so a failed step is simply the loop's end.
        match day.checked_add_days(Days::new(1)) {
            Some(tomorrow) => day = tomorrow,
            None => break,
        }
    }

    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn usd(minor: i64) -> Money {
        Money::new(minor, Currency::Usd)
    }

    fn as_of_utc(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, 12, 0, 0).unwrap()
    }

    fn event(occurs_on: NaiveDate, kind: EventKind, minor: i64) -> ForecastEvent {
        ForecastEvent {
            occurs_on,
            kind,
            amount: usd(minor),
            source_event_id: Uuid::nil(),
            assumption_basis: AssumptionBasis::ManualOneOff,
        }
    }

    #[test]
    fn empty_horizon_emits_no_rows() {
        let rows = forecast_layer1(
            &[],
            usd(1_000),
            Horizon::new(as_of_utc(2026, 6, 1), 0),
            Tz::UTC,
        )
        .unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn every_day_in_horizon_emits_one_row_carrying_balance_forward() {
        let events = [event(date(2026, 6, 3), EventKind::Income, 5_000)];
        let rows = forecast_layer1(
            &events,
            usd(1_000),
            Horizon::new(as_of_utc(2026, 6, 1), 5),
            Tz::UTC,
        )
        .unwrap();
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[0].date, date(2026, 6, 1));
        assert_eq!(rows[4].date, date(2026, 6, 5));
        // Balance carries forward until the income lands on the 3rd.
        assert_eq!(rows[0].closing.p50, usd(1_000));
        assert_eq!(rows[1].closing.p50, usd(1_000));
        assert_eq!(rows[2].closing.p50, usd(6_000));
        assert_eq!(rows[4].closing.p50, usd(6_000));
        // Deterministic Layer-1 emits a collapsed band (p10 == p50 == p90).
        assert_eq!(rows[2].closing, Band::point(usd(6_000)));
        assert!(rows[0].events.is_empty());
        assert_eq!(rows[2].events.len(), 1);
    }

    #[test]
    fn events_outside_the_window_are_ignored() {
        let events = [
            event(date(2026, 5, 31), EventKind::Income, 9_999), // before start
            event(date(2026, 6, 10), EventKind::Income, 8_888), // after end
            event(date(2026, 6, 2), EventKind::RecurringBill, -500),
        ];
        let rows = forecast_layer1(
            &events,
            usd(1_000),
            Horizon::new(as_of_utc(2026, 6, 1), 3),
            Tz::UTC,
        )
        .unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[2].closing.p50, usd(500));
        assert!(rows.iter().all(|r| r
            .events
            .iter()
            .all(|e| e.amount != usd(9_999) && e.amount != usd(8_888))));
    }

    #[test]
    fn same_day_events_apply_income_before_bills_before_transfers_before_manual() {
        // All on day 0, supplied out of canonical order.
        let day0 = date(2026, 6, 1);
        let events = [
            event(day0, EventKind::ManualOneOff, -100),
            event(day0, EventKind::Transfer, -10),
            event(day0, EventKind::RecurringBill, -1_000),
            event(day0, EventKind::Income, 5_000),
        ];
        let rows = forecast_layer1(
            &events,
            usd(0),
            Horizon::new(as_of_utc(2026, 6, 1), 1),
            Tz::UTC,
        )
        .unwrap();
        let kinds: Vec<EventKind> = rows[0].events.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![
                EventKind::Income,
                EventKind::RecurringBill,
                EventKind::Transfer,
                EventKind::ManualOneOff
            ]
        );
        assert_eq!(rows[0].closing.p50, usd(3_890));
    }

    #[test]
    fn mismatched_currency_is_rejected() {
        let events = [ForecastEvent {
            occurs_on: date(2026, 6, 1),
            kind: EventKind::Income,
            amount: Money::new(100, Currency::Eur),
            source_event_id: Uuid::nil(),
            assumption_basis: AssumptionBasis::ManualOneOff,
        }];
        let err = forecast_layer1(
            &events,
            usd(0),
            Horizon::new(as_of_utc(2026, 6, 1), 1),
            Tz::UTC,
        )
        .unwrap_err();
        assert_eq!(
            err,
            ForecastError::CurrencyMismatch {
                expected: Currency::Usd,
                found: Currency::Eur,
            }
        );
    }

    #[test]
    fn start_date_follows_household_tz_not_utc() {
        // 2026-06-01T03:00Z is still 2026-05-31 in Los Angeles (UTC-7 in June).
        let as_of = Utc.with_ymd_and_hms(2026, 6, 1, 3, 0, 0).unwrap();
        let rows = forecast_layer1(
            &[],
            usd(0),
            Horizon::new(as_of, 1),
            Tz::America__Los_Angeles,
        )
        .unwrap();
        assert_eq!(rows[0].date, date(2026, 5, 31));
    }
}
