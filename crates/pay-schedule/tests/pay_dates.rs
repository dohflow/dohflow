//! Public-API + persistence tests for the pay-schedule engine (personal-cfo-82q9).

use chrono::NaiveDate;
use pay_schedule::{Frequency, PaySchedule};

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

#[test]
fn biweekly_paycheck_over_a_90_day_forecast_horizon() {
    let schedule = PaySchedule::new(Frequency::Biweekly, d(2026, 6, 5));
    let dates = schedule.pay_dates(d(2026, 6, 1), d(2026, 8, 30));
    assert_eq!(
        dates,
        vec![
            d(2026, 6, 5),
            d(2026, 6, 19),
            d(2026, 7, 3),
            d(2026, 7, 17),
            d(2026, 7, 31),
            d(2026, 8, 14),
            d(2026, 8, 28),
        ],
    );
    // The next paycheck mid-cycle is the following occurrence.
    assert_eq!(schedule.next_pay_date(d(2026, 6, 6)), Some(d(2026, 6, 19)));
}

#[test]
fn frequency_tokens_round_trip() {
    for freq in [
        Frequency::Weekly,
        Frequency::Biweekly,
        Frequency::SemiMonthly,
        Frequency::Monthly,
        Frequency::Quarterly,
        Frequency::Annual,
    ] {
        assert_eq!(Frequency::from_token(&freq.token()), Some(freq));
    }
    assert_eq!(Frequency::SemiMonthly.token(), "semi_monthly");
    assert_eq!(Frequency::from_token("nonsense"), None);
}

#[test]
fn schedule_round_trips_through_serde_with_snake_case_frequency() {
    let schedule = PaySchedule::new(Frequency::SemiMonthly, d(2026, 6, 15));
    let json = serde_json::to_string(&schedule).unwrap();
    assert!(json.contains("\"semi_monthly\""));
    assert!(json.contains("\"2026-06-15\""));

    let restored: PaySchedule = serde_json::from_str(&json).unwrap();
    assert_eq!(restored, schedule);
    // The restored schedule produces the same dates.
    assert_eq!(
        restored.pay_dates(d(2026, 6, 1), d(2026, 6, 30)),
        schedule.pay_dates(d(2026, 6, 1), d(2026, 6, 30)),
    );
}
