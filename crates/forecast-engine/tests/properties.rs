//! Property tests (personal-cfo-164u AC): invariants that must hold for any
//! input, checked with `proptest`.
//!
//! - **Running-sum invariant:** each day's closing balance equals the starting
//!   balance plus every event amount dated on or before that day.
//! - **Same-day commutativity:** the output does not depend on the order events
//!   are supplied in (the engine imposes a total same-day order).
//! - **Monotonicity / non-negativity:** when no event is an outflow and the
//!   opening balance is non-negative, balances never decrease and stay ≥ 0
//!   ("cash stays non-negative when bills don't exceed inflows").

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use core_money::{Currency, Money};
use forecast_engine::{forecast_layer1, AssumptionBasis, EventKind, ForecastEvent, Horizon, Tz};
use proptest::prelude::*;
use uuid::Uuid;

const BASE_YEAR: i32 = 2026;
const HORIZON_DAYS: u32 = 40;

fn usd(minor: i64) -> Money {
    Money::new(minor, Currency::Usd)
}

fn as_of() -> DateTime<Utc> {
    // Noon UTC on 2026-01-01 → start date 2026-01-01 in UTC.
    Utc.with_ymd_and_hms(BASE_YEAR, 1, 1, 12, 0, 0).unwrap()
}

fn start_date() -> NaiveDate {
    NaiveDate::from_ymd_opt(BASE_YEAR, 1, 1).unwrap()
}

fn kind_of(tag: u8) -> EventKind {
    match tag % 5 {
        0 => EventKind::Income,
        1 => EventKind::RecurringBill,
        2 => EventKind::LoanPayment,
        3 => EventKind::Transfer,
        _ => EventKind::ManualOneOff,
    }
}

prop_compose! {
    /// An event within ~50 days of the start, with a bounded ±$100k amount so
    /// no realistic multiset can overflow `i64` minor units.
    fn any_event()(
        day_offset in 0u32..50,
        tag in any::<u8>(),
        minor in -10_000_000i64..=10_000_000,
        id in any::<u128>(),
    ) -> ForecastEvent {
        let occurs_on = start_date()
            .checked_add_days(chrono::Days::new(u64::from(day_offset)))
            .unwrap();
        ForecastEvent {
            occurs_on,
            kind: kind_of(tag),
            amount: usd(minor),
            source_event_id: Uuid::from_u128(id),
            assumption_basis: AssumptionBasis::ManualOneOff,
        }
    }
}

prop_compose! {
    /// A non-negative-only event (inflows / zero), for the monotonicity property.
    fn inflow_event()(
        day_offset in 0u32..50,
        minor in 0i64..=10_000_000,
        id in any::<u128>(),
    ) -> ForecastEvent {
        let occurs_on = start_date()
            .checked_add_days(chrono::Days::new(u64::from(day_offset)))
            .unwrap();
        ForecastEvent {
            occurs_on,
            kind: EventKind::Income,
            amount: usd(minor),
            source_event_id: Uuid::from_u128(id),
            assumption_basis: AssumptionBasis::ManualOneOff,
        }
    }
}

proptest! {
    #[test]
    fn closing_balance_is_the_running_prefix_sum(
        events in prop::collection::vec(any_event(), 0..60),
        starting in -1_000_000i64..=100_000_000,
    ) {
        let rows = forecast_layer1(&events, usd(starting), Horizon::new(as_of(), HORIZON_DAYS), Tz::UTC)
            .expect("single-currency fixture never errors");
        prop_assert_eq!(rows.len(), HORIZON_DAYS as usize);

        for row in &rows {
            // Independently sum every event dated on or before this row's day.
            let expected: i64 = events
                .iter()
                .filter(|e| e.occurs_on <= row.date)
                .map(|e| e.amount.minor_units())
                .sum();
            prop_assert_eq!(row.closing.p50, usd(starting + expected));
        }
    }

    #[test]
    fn output_is_independent_of_input_order(
        events in prop::collection::vec(any_event(), 0..60),
        starting in -1_000_000i64..=100_000_000,
    ) {
        let horizon = Horizon::new(as_of(), HORIZON_DAYS);
        let canonical = forecast_layer1(&events, usd(starting), horizon, Tz::UTC).unwrap();

        let mut reversed = events.clone();
        reversed.reverse();
        let from_reversed = forecast_layer1(&reversed, usd(starting), horizon, Tz::UTC).unwrap();
        prop_assert_eq!(&canonical, &from_reversed);

        // A rotation is a different permutation again.
        let mut rotated = events.clone();
        if !rotated.is_empty() {
            rotated.rotate_left(1);
        }
        let from_rotated = forecast_layer1(&rotated, usd(starting), horizon, Tz::UTC).unwrap();
        prop_assert_eq!(&canonical, &from_rotated);
    }

    #[test]
    fn inflows_only_keep_balance_non_decreasing_and_non_negative(
        events in prop::collection::vec(inflow_event(), 0..60),
        starting in 0i64..=100_000_000,
    ) {
        let rows = forecast_layer1(&events, usd(starting), Horizon::new(as_of(), HORIZON_DAYS), Tz::UTC)
            .unwrap();
        let mut previous = usd(starting);
        for row in &rows {
            prop_assert!(row.closing.p50.minor_units() >= 0);
            prop_assert!(row.closing.p50.minor_units() >= previous.minor_units());
            previous = row.closing.p50;
        }
    }
}
