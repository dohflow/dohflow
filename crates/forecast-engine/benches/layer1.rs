//! Layer-1 performance baseline (personal-cfo-164u AC): the deterministic fold
//! must complete in **<50ms on M1 dev** for a 1-year horizon × 200 monthly bills
//! × 26 biweekly paychecks (≈2426 events).
//!
//! Dependency-free harness (`harness = false`): a plain `main` that times the
//! pure function with `std::time::Instant`. Run with:
//!
//! ```sh
//! cargo bench -p forecast-engine
//! ```

use std::hint::black_box;
use std::time::Instant;

use chrono::{Days, NaiveDate, TimeZone, Utc};
use core_money::{Currency, Money};
use forecast_engine::{forecast_layer1, AssumptionBasis, EventKind, ForecastEvent, Horizon, Tz};
use uuid::Uuid;

const BILLS: u128 = 200;
const PAYCHECKS: u32 = 26;
const HORIZON_DAYS: u32 = 365;
const ITERATIONS: u32 = 200;

fn usd(minor: i64) -> Money {
    Money::new(minor, Currency::Usd)
}

fn year_fixture() -> Vec<ForecastEvent> {
    let mut events = Vec::with_capacity((BILLS as usize * 12) + PAYCHECKS as usize);
    for i in 0..BILLS {
        let day = (i as u32 % 28) + 1;
        for month in 1..=12u32 {
            events.push(ForecastEvent {
                occurs_on: NaiveDate::from_ymd_opt(2026, month, day).unwrap(),
                kind: EventKind::RecurringBill,
                amount: usd(-5_000),
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
            amount: usd(200_000),
            source_event_id: Uuid::from_u128(1_000 + u128::from(k)),
            assumption_basis: AssumptionBasis::RecurringSchedule {
                frequency: pay_schedule::Frequency::Biweekly,
            },
        });
    }
    events
}

fn main() {
    let events = year_fixture();
    let starting = usd(10_000_000);
    let as_of = Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap();
    let horizon = Horizon::new(as_of, HORIZON_DAYS);

    // Warm up (and assert the fixture is well-formed before timing).
    let rows = forecast_layer1(&events, starting, horizon, Tz::UTC).unwrap();
    assert_eq!(rows.len(), HORIZON_DAYS as usize);

    let mut best = std::time::Duration::MAX;
    let mut total = std::time::Duration::ZERO;
    for _ in 0..ITERATIONS {
        let start = Instant::now();
        let out = forecast_layer1(
            black_box(&events),
            black_box(starting),
            black_box(horizon),
            Tz::UTC,
        )
        .unwrap();
        let elapsed = start.elapsed();
        black_box(out);
        best = best.min(elapsed);
        total += elapsed;
    }

    let avg = total / ITERATIONS;
    println!(
        "forecast_layer1: {} events × {} days — best {:?}, avg {:?} over {} iters (budget <50ms)",
        events.len(),
        HORIZON_DAYS,
        best,
        avg,
        ITERATIONS,
    );
}
