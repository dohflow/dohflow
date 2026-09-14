//! Timezone + DST edge cases (personal-cfo-164u AC; aligned with the
//! cross-cutting suite personal-cfo-rp1r).
//!
//! Two things must hold:
//! 1. The horizon **start date follows the household zone**, not UTC / the
//!    machine's local zone — including half-hour offsets and 30-minute DST.
//! 2. An event's **local calendar date is preserved** across a DST transition:
//!    a bill on a spring-forward / fall-back day still lands on that local date.
//!
//! Zones exercised: `America/Los_Angeles` (1h DST), `Europe/London` (1h DST),
//! `Asia/Kolkata` (UTC+5:30, no DST), `Australia/Lord_Howe` (UTC+10:30 / 30-min
//! DST).

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use core_money::{Currency, Money};
use forecast_engine::{forecast_layer1, AssumptionBasis, EventKind, ForecastEvent, Horizon, Tz};
use uuid::Uuid;

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
}

fn usd(minor: i64) -> Money {
    Money::new(minor, Currency::Usd)
}

fn event(on: NaiveDate, kind: EventKind, minor: i64) -> ForecastEvent {
    ForecastEvent {
        occurs_on: on,
        kind,
        amount: usd(minor),
        source_event_id: Uuid::nil(),
        assumption_basis: AssumptionBasis::ManualOneOff,
    }
}

fn start_date_in(tz: Tz, as_of: DateTime<Utc>) -> NaiveDate {
    forecast_layer1(&[], usd(0), Horizon::new(as_of, 1), tz).unwrap()[0].date
}

#[test]
fn start_date_follows_los_angeles_dst() {
    // 2026-03-08 is the spring-forward day in LA. At 18:00Z it is 11:00 PDT
    // (UTC-7) on the 8th — start date is 2026-03-08.
    assert_eq!(
        start_date_in(Tz::America__Los_Angeles, at(2026, 3, 8, 18, 0)),
        date(2026, 3, 8)
    );
}

#[test]
fn start_date_follows_london_bst_across_midnight() {
    // June → BST (UTC+1). 23:30Z is 00:30 on the next local day.
    assert_eq!(
        start_date_in(Tz::Europe__London, at(2026, 6, 1, 23, 30)),
        date(2026, 6, 2)
    );
}

#[test]
fn start_date_follows_kolkata_half_hour_offset() {
    // IST is UTC+5:30 year-round. 19:00Z is 00:30 IST the next local day.
    assert_eq!(
        start_date_in(Tz::Asia__Kolkata, at(2026, 6, 1, 19, 0)),
        date(2026, 6, 2)
    );
}

#[test]
fn start_date_reflects_lord_howe_30_minute_dst() {
    // Same wall-clock UTC instant (13:20Z), two seasons. Lord Howe is on DST
    // (UTC+11) in January but standard time (UTC+10:30) in July — the 30-minute
    // difference flips which calendar day "now" falls on.
    assert_eq!(
        start_date_in(Tz::Australia__Lord_Howe, at(2026, 1, 15, 13, 20)),
        date(2026, 1, 16), // +11:00 → 00:20 next day
    );
    assert_eq!(
        start_date_in(Tz::Australia__Lord_Howe, at(2026, 7, 15, 13, 20)),
        date(2026, 7, 15), // +10:30 → 23:50 same day
    );
}

#[test]
fn bill_on_spring_forward_day_keeps_its_local_date() {
    // Start 2026-03-07 (PST), horizon covers the 2026-03-08 spring-forward day.
    // A bill dated on the transition day must land on that local date's row.
    let bill = event(date(2026, 3, 8), EventKind::RecurringBill, -5_000);
    let rows = forecast_layer1(
        &[bill],
        usd(100_000),
        Horizon::new(at(2026, 3, 7, 18, 0), 3),
        Tz::America__Los_Angeles,
    )
    .unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].date, date(2026, 3, 7));
    assert_eq!(rows[1].date, date(2026, 3, 8));
    assert_eq!(rows[1].events.len(), 1);
    assert_eq!(rows[1].closing.p50, usd(95_000));
}

#[test]
fn ordering_is_preserved_on_a_fall_back_day() {
    // 2026-11-01 is the LA fall-back day. Income and a bill share that date;
    // income must still apply before the bill (same-day ordering holds).
    let income = event(date(2026, 11, 1), EventKind::Income, 200_000);
    let bill = event(date(2026, 11, 1), EventKind::RecurringBill, -50_000);
    let rows = forecast_layer1(
        &[bill, income],
        usd(0),
        Horizon::new(at(2026, 11, 1, 17, 0), 1),
        Tz::America__Los_Angeles,
    )
    .unwrap();
    assert_eq!(rows[0].date, date(2026, 11, 1));
    let kinds: Vec<EventKind> = rows[0].events.iter().map(|e| e.kind).collect();
    assert_eq!(kinds, vec![EventKind::Income, EventKind::RecurringBill]);
    assert_eq!(rows[0].closing.p50, usd(150_000));
}
