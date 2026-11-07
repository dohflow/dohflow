//! The NF-1 Future Cash golden (personal-cfo-i93n3.27 slice E, acceptance criterion 2).
//!
//! The iPhone's Future Cash must equal the desktop's for one known fixture, and both are held to ONE
//! checked-in expected result, `tests/fixtures/nf1_future_cash.json`. That file is derived by arithmetic
//! from the fixture (an opening balance, two one-off events, a closing balance per group and account, no
//! uncertainty range), not recorded from a run. This test seeds NF-1 through the DESKTOP command path
//! (`create_account_impl`, `create_manual_future_entry_impl`) and reads it back through
//! `future_cash_forecast_impl` and `future_cash_by_account_impl`. The iOS harness builds the same fixture
//! through IPC from the page and compares the in-app result to the same file.
//!
//! The events are one-offs at fixed offsets from "today" (+7 and +10, no recurrence), so a calendar
//! boundary (a month length, a day of the month) cannot change the result. The forecast reads the wall
//! clock inside the worker, so "today" cannot be pinned; instead the household timezone is set to UTC
//! explicitly, "today" is read before seeding and after the result, and the run is retried ONCE if the
//! date rolled over in between and fails loudly if it rolls over twice.

use app_lib::ipc::commands::{
    create_account_impl, create_manual_future_entry_impl, future_cash_by_account_impl,
    future_cash_forecast_impl, set_household_timezone_impl,
};
use app_lib::ipc::dto::{
    AccountFlagsDto, CashflowRoleDto, CreateAccountInput, CreateManualFutureEntryInput,
    ForecastDayDto, ForecastViewDto, MoneyDto, MultiSeriesForecastDto,
};
use app_lib::AppState;
use chrono::{Duration, NaiveDate, Utc};
use finance_kernel::VaultController;
use serde_json::Value;
use tempfile::TempDir;

const EXPECTED: &str = include_str!("fixtures/nf1_future_cash.json");

fn usd(minor_units: i64) -> MoneyDto {
    MoneyDto {
        minor_units,
        currency: "USD".to_owned(),
    }
}

fn utc_today() -> NaiveDate {
    Utc::now().date_naive()
}

struct Run {
    today: NaiveDate,
    forecast: ForecastViewDto,
    by_account: MultiSeriesForecastDto,
    expected: Value,
}

/// Seed NF-1 through the desktop command path and read the forecast back.
fn run_once() -> (NaiveDate, Run, NaiveDate) {
    let expected: Value = serde_json::from_str(EXPECTED).expect("expected file is JSON");
    let horizon = expected["horizon_days"].as_u64().expect("horizon") as u32;

    let before = utc_today();
    let dir = TempDir::new().expect("temp dir");
    let mut controller = VaultController::open(dir.path().join("vault.db"));
    controller.create(b"test-key").expect("create vault");
    let state = AppState::new(controller);
    set_household_timezone_impl(&state, "UTC".to_owned()).expect("household timezone");

    let mut ids = std::collections::HashMap::new();
    for account in expected["accounts"].as_array().expect("accounts") {
        let name = account["name"].as_str().expect("name");
        let id = create_account_impl(
            &state,
            CreateAccountInput {
                name: name.to_owned(),
                cashflow_role: CashflowRoleDto::LiquidCash,
                currency: "USD".to_owned(),
                flags: Some(AccountFlagsDto {
                    retirement: false,
                    tax_advantaged: false,
                    joint: false,
                    business: false,
                }),
                opening_balance: Some(usd(account["opening_minor"].as_i64().expect("opening"))),
                subtype: Some(account["subtype"].as_str().expect("subtype").to_owned()),
                idempotency_key: String::new(),
            },
        )
        .expect("account")
        .account_id;
        ids.insert(name.to_owned(), id);
    }
    for event in expected["events"].as_array().expect("events") {
        let offset = event["offset"].as_i64().expect("offset");
        create_manual_future_entry_impl(
            &state,
            CreateManualFutureEntryInput {
                amount: usd(event["amount_minor"].as_i64().expect("amount")),
                date: (before + Duration::days(offset)).to_string(),
                label: event["label"].as_str().expect("label").to_owned(),
                account_id: Some(ids[event["account"].as_str().expect("account")].clone()),
            },
        )
        .expect("manual entry");
    }
    let forecast = future_cash_forecast_impl(&state, horizon, vec![]).expect("forecast");
    let by_account = future_cash_by_account_impl(&state, horizon, vec![]).expect("by account");
    let after = utc_today();
    (
        before,
        Run {
            today: before,
            forecast,
            by_account,
            expected,
        },
        after,
    )
}

/// Run, retrying once if the UTC date changed while the run was in flight.
fn run() -> Run {
    for attempt in 0..2 {
        let (before, run, after) = run_once();
        if before == after {
            return run;
        }
        eprintln!("the date rolled over during attempt {attempt}: {before} -> {after}; retrying");
    }
    panic!("the UTC date rolled over twice in a row while seeding NF-1: re-run the test");
}

/// The expected closing at `offset`: the last listed closing at or before it.
fn expected_at(expected: &Value, offset: i64) -> &Value {
    expected["closings"]
        .as_array()
        .expect("closings")
        .iter()
        .rfind(|c| c["offset"].as_i64().expect("offset") <= offset)
        .expect("a closing at or before every offset")
}

fn offset_of(today: NaiveDate, date: &str) -> i64 {
    (NaiveDate::parse_from_str(date, "%Y-%m-%d").expect("date") - today).num_days()
}

fn assert_no_range(day: &ForecastDayDto) {
    assert_eq!(
        day.closing.p10, day.closing.p50,
        "no range: p10 == p50 on {}",
        day.date
    );
    assert_eq!(
        day.closing.p90, day.closing.p50,
        "no range: p90 == p50 on {}",
        day.date
    );
}

#[test]
fn nf1_future_cash_matches_the_checked_in_arithmetic() {
    let Run {
        today,
        forecast,
        by_account,
        expected,
    } = run();

    // The starting balance is the two openings.
    assert_eq!(forecast.currency, "USD");
    assert_eq!(forecast.start_date, today.to_string());
    assert_eq!(
        forecast.starting_balance.minor_units,
        expected_at(&expected, 0)["net"].as_i64().unwrap()
    );
    // One row per calendar day in the horizon, ascending (today and the days after it, `horizon_days` rows in all), and the per-group and
    // per-account series cover exactly the same days: a series that is short would otherwise pass by comparing less.
    let expected_days = expected["horizon_days"].as_u64().expect("horizon") as usize;
    assert_eq!(
        forecast.days.len(),
        expected_days,
        "one forecast row per day of the horizon"
    );
    for window in forecast.days.windows(2) {
        assert!(window[0].date < window[1].date, "days ascend");
    }

    // Net (the plain forecast): every day's closing is the arithmetic closing at its offset.
    for day in &forecast.days {
        let offset = offset_of(today, &day.date);
        assert!(
            (0..=i64::from(forecast.horizon_days)).contains(&offset),
            "day {} inside the horizon",
            day.date
        );
        assert_no_range(day);
        assert_eq!(
            day.closing.p50.minor_units,
            expected_at(&expected, offset)["net"].as_i64().unwrap(),
            "net closing at +{offset} ({})",
            day.date
        );
    }

    // Both events land on their dates, once each, with their signed amounts.
    for event in expected["events"].as_array().unwrap() {
        let date = (today + Duration::days(event["offset"].as_i64().unwrap())).to_string();
        let day = forecast
            .days
            .iter()
            .find(|d| d.date == date)
            .unwrap_or_else(|| panic!("a day for {date}"));
        let hits: Vec<_> = day
            .events
            .iter()
            .filter(|e| e.name == event["label"].as_str().unwrap())
            .collect();
        assert_eq!(hits.len(), 1, "exactly one {} on {date}", event["label"]);
        assert_eq!(
            hits[0].amount.minor_units,
            event["amount_minor"].as_i64().unwrap()
        );
    }
    let total_events: usize = forecast.days.iter().map(|d| d.events.len()).sum();
    assert_eq!(
        total_events,
        expected["events"].as_array().unwrap().len(),
        "no other events"
    );

    // Per group: spendable, reserve, net.
    for group in &by_account.groups {
        let key = match group.tier.as_str() {
            "spendable" | "reserve" | "net" => group.tier.as_str(),
            other => panic!("unexpected tier {other}: NF-1 has no unallocated cash"),
        };
        for closing in &group.closings {
            let offset = offset_of(today, &closing.date);
            assert_eq!(
                closing.closing.p50.minor_units,
                expected_at(&expected, offset)[key].as_i64().unwrap(),
                "{key} closing at +{offset}"
            );
            assert_eq!(closing.closing.p10, closing.closing.p50);
            assert_eq!(closing.closing.p90, closing.closing.p50);
        }
    }
    assert_eq!(by_account.groups.len(), 3, "spendable, reserve and net");
    for group in &by_account.groups {
        assert_eq!(
            group.closings.len(),
            forecast.days.len(),
            "{} covers every day",
            group.tier
        );
    }

    // Per account, with its tier.
    for account in expected["accounts"].as_array().unwrap() {
        let name = account["name"].as_str().unwrap();
        let series = by_account
            .accounts
            .iter()
            .find(|a| a.name == name)
            .unwrap_or_else(|| panic!("a series for {name}"));
        assert_eq!(
            series.tier,
            account["tier"].as_str().unwrap(),
            "{name}'s tier"
        );
        for day in &series.days {
            let offset = offset_of(today, &day.date);
            assert_no_range(day);
            assert_eq!(
                day.closing.p50.minor_units,
                expected_at(&expected, offset)["accounts"][name]
                    .as_i64()
                    .unwrap(),
                "{name} closing at +{offset}"
            );
        }
    }
    assert_eq!(
        by_account.accounts.len(),
        2,
        "two accounts and nothing unallocated"
    );
    for series in &by_account.accounts {
        assert_eq!(
            series.days.len(),
            forecast.days.len(),
            "{} covers every day",
            series.name
        );
    }
}
