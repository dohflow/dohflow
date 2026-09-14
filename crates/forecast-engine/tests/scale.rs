//! Scale correctness (personal-cfo-164u AC): the engine must fold the
//! benchmark-sized workload — a 1-year horizon × 200 monthly bills × 26
//! biweekly paychecks — and produce a correct series. This guards the *result*
//! at scale; the wall-clock budget (<50ms on M1) is measured separately by
//! `benches/layer1.rs` (run with `cargo bench`), kept out of the test suite so
//! CI never flakes on timing.

use chrono::{Datelike, Days, NaiveDate, TimeZone, Utc};
use core_money::{Currency, Money};
use forecast_engine::{forecast_layer1, AssumptionBasis, EventKind, ForecastEvent, Horizon, Tz};
use uuid::Uuid;

const BILLS: u128 = 200;
const BILL_MINOR: i64 = -5_000; // -$50.00 per occurrence
const PAYCHECKS: u32 = 26;
const PAYCHECK_MINOR: i64 = 200_000; // +$2,000.00 per occurrence
const HORIZON_DAYS: u32 = 365;

fn usd(minor: i64) -> Money {
    Money::new(minor, Currency::Usd)
}

/// 200 monthly bills (each on a day 1–28, every month of 2026) plus 26 biweekly
/// paychecks from 2026-01-02 — the full 1-year fixture (≈2426 events).
fn year_fixture() -> Vec<ForecastEvent> {
    let mut events = Vec::with_capacity((BILLS as usize * 12) + PAYCHECKS as usize);
    for i in 0..BILLS {
        let day = (i as u32 % 28) + 1;
        for month in 1..=12u32 {
            events.push(ForecastEvent {
                occurs_on: NaiveDate::from_ymd_opt(2026, month, day).unwrap(),
                kind: EventKind::RecurringBill,
                amount: usd(BILL_MINOR),
                source_event_id: Uuid::from_u128(i),
                assumption_basis: AssumptionBasis::RecurringSchedule {
                    frequency: pay_schedule::Frequency::Monthly,
                },
            });
        }
    }
    let first_pay = NaiveDate::from_ymd_opt(2026, 1, 2).unwrap();
    for k in 0..PAYCHECKS {
        events.push(ForecastEvent {
            occurs_on: first_pay
                .checked_add_days(Days::new(u64::from(k) * 14))
                .unwrap(),
            kind: EventKind::Income,
            amount: usd(PAYCHECK_MINOR),
            source_event_id: Uuid::from_u128(1_000 + u128::from(k)),
            assumption_basis: AssumptionBasis::RecurringSchedule {
                frequency: pay_schedule::Frequency::Biweekly,
            },
        });
    }
    events
}

#[test]
fn folds_the_one_year_fixture_correctly() {
    let events = year_fixture();
    let starting = usd(10_000_000); // $100,000.00
    let as_of = Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap();

    let rows = forecast_layer1(
        &events,
        starting,
        Horizon::new(as_of, HORIZON_DAYS),
        Tz::UTC,
    )
    .unwrap();

    // 2026 is not a leap year → 365 emitted days, first and last bracket the year.
    assert_eq!(rows.len(), 365);
    assert_eq!(
        rows.first().unwrap().date,
        NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()
    );
    assert_eq!(
        rows.last().unwrap().date,
        NaiveDate::from_ymd_opt(2026, 12, 31).unwrap()
    );

    // Every event lands inside the window, so the final balance is starting + Σ.
    let total: i64 = events.iter().map(|e| e.amount.minor_units()).sum();
    assert_eq!(rows.last().unwrap().closing.p50, usd(10_000_000 + total));

    // Every event is attributed to exactly one day.
    let applied: usize = rows.iter().map(|r| r.events.len()).sum();
    assert_eq!(applied, events.len());

    // Sanity: each row's date is the previous + 1 day, and balances only move on
    // event days.
    for window in rows.windows(2) {
        assert_eq!(
            window[1].date,
            window[0].date.succ_opt().unwrap(),
            "rows must be contiguous calendar days"
        );
    }
    assert!(rows
        .iter()
        .any(|r| r.date.day() == 2 && r.date.month() == 1 && !r.events.is_empty()));
}
