//! Determinism (personal-cfo-164u AC): a fixed input fixture must always
//! produce byte-identical output. A regression in the fold, ordering, or
//! day-emission logic would change the `insta` snapshot and fail CI.

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use core_money::{Currency, Money};
use forecast_engine::{forecast_layer1, AssumptionBasis, EventKind, ForecastEvent, Horizon, Tz};
use uuid::Uuid;

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn usd(minor: i64) -> Money {
    Money::new(minor, Currency::Usd)
}

fn recurring(
    id: u128,
    on: NaiveDate,
    kind: EventKind,
    minor: i64,
    freq: pay_schedule::Frequency,
) -> ForecastEvent {
    ForecastEvent {
        occurs_on: on,
        kind,
        amount: usd(minor),
        source_event_id: Uuid::from_u128(id),
        assumption_basis: AssumptionBasis::RecurringSchedule { frequency: freq },
    }
}

/// A compact but realistic two-week window: an opening balance, a paycheck, rent
/// on day 0, and two same-day bills (which exercise the same-day tie-break).
/// Events are supplied out of order on purpose — the engine sorts them.
fn month_fixture() -> (Vec<ForecastEvent>, Money, Horizon, Tz) {
    let events = vec![
        recurring(
            3,
            date(2026, 6, 10),
            EventKind::RecurringBill,
            -4_500,
            pay_schedule::Frequency::Monthly,
        ), // utility
        recurring(
            1,
            date(2026, 6, 1),
            EventKind::RecurringBill,
            -120_000,
            pay_schedule::Frequency::Monthly,
        ), // rent
        recurring(
            2,
            date(2026, 6, 10),
            EventKind::RecurringBill,
            -1_599,
            pay_schedule::Frequency::Monthly,
        ), // subscription
        recurring(
            4,
            date(2026, 6, 5),
            EventKind::Income,
            150_000,
            pay_schedule::Frequency::Biweekly,
        ), // paycheck
    ];
    let as_of: DateTime<Utc> = Utc.with_ymd_and_hms(2026, 6, 1, 12, 0, 0).unwrap();
    (events, usd(200_000), Horizon::new(as_of, 14), Tz::UTC)
}

#[test]
fn month_fixture_is_byte_identical() {
    let (events, starting, horizon, tz) = month_fixture();
    let rows = forecast_layer1(&events, starting, horizon, tz).unwrap();
    insta::assert_debug_snapshot!(rows);
}

#[test]
fn same_input_yields_equal_output() {
    let (events, starting, horizon, tz) = month_fixture();
    let a = forecast_layer1(&events, starting, horizon, tz).unwrap();
    let b = forecast_layer1(&events, starting, horizon, tz).unwrap();
    assert_eq!(a, b);
}
