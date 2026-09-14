//! Household-local "today" resolution (ADR 0021 §1) for the six call sites
//! `personal-cfo-5ie.11`'s fix deliberately left out of scope: `money_inbox_list`'s
//! snooze filter, `recurring_candidates`/`income_candidates`' next-date projection,
//! and the next pay/due/occurrence date on income sources, recurring bills, and
//! recurring transfers (`personal-cfo-m8x2r`).
//!
//! None of the six take an explicit as-of instant — they all resolve
//! `forecast::household_today(conn)` -> `Utc::now()` internally — so the only lever
//! available to a test is the household TIMEZONE itself: pick a zone whose local
//! calendar date differs from UTC's at the moment the test runs, and prove the
//! site's output reflects the LOCAL date, not UTC's. This mirrors the "sanity: the
//! zones actually disagree" pattern `5ie.11`'s own tests
//! (`crates/db-worker/src/tests/mod.rs`) already use.

mod common;

use chrono::{Duration, NaiveDate, Timelike, Utc};
use common::*;
use core_ledger::{
    AccountId, BillContractId, CashflowRole, IncomeSourceId, RecurringEventId, RecurringTransferId,
};
use core_money::{Currency, Money};
use db_worker::*;
use pay_schedule::Frequency;
use rusqlite::params;
use uuid::Uuid;

/// A household timezone whose local calendar date is guaranteed to differ from
/// UTC's *right now*, plus that local date. Both zones are fixed-offset and
/// DST-free (verified against chrono-tz 0.10.4), so the arithmetic below is exact
/// and this test crate never needs a chrono-tz dependency of its own.
///
/// Covers all 24 UTC hours: `Pacific/Niue` (UTC−11) disagrees with UTC whenever
/// the UTC hour is `< 11` (its local midnight falls at UTC 11:00, so before that
/// its local date is still UTC's *yesterday*); `Pacific/Kiritimati` (UTC+14)
/// disagrees whenever the UTC hour is `>= 10` (its local midnight falls at UTC
/// 10:00 the *prior* day, so from then on its local date is already UTC's
/// *tomorrow*). `[0, 11)` ∪ `[10, 24)` = every hour — no gap.
fn zone_that_disagrees_with_utc(now: chrono::DateTime<Utc>) -> (&'static str, NaiveDate) {
    if now.hour() < 11 {
        ("Pacific/Niue", (now - Duration::hours(11)).date_naive())
    } else {
        (
            "Pacific/Kiritimati",
            (now + Duration::hours(14)).date_naive(),
        )
    }
}

/// Sets the household timezone to one guaranteed to disagree with UTC right now,
/// and returns `(household-local today, UTC today)`. Asserts the sanity
/// precondition that they really do disagree, so every test built on this can
/// never pass vacuously (the same guard `5ie.11`'s own tests use).
fn disagreeing_household(worker: &DbWorker) -> (NaiveDate, NaiveDate) {
    let utc_today = Utc::now().date_naive();
    let (zone, local_today) = zone_that_disagrees_with_utc(Utc::now());
    worker.set_household_timezone(zone).unwrap();
    assert_ne!(
        local_today, utc_today,
        "sanity: the chosen zone ({zone}) must actually disagree with UTC right now"
    );
    (local_today, utc_today)
}

// ===========================================================================
// (1) money_inbox_list — snooze expiry (lib.rs's `until <= today` string compare)
// ===========================================================================

/// ci71's snooze filter (`money_inbox_list`) must judge `snoozed_until` against
/// the household-local date, not UTC's. `snoozed_until` is set to
/// `max(local, utc)`: under the household-correct resolution the item is visible
/// only when `local > utc`; under the UTC-buggy resolution it would be visible
/// only when `utc > local` — the two never agree, so this discriminates in
/// every case `disagreeing_household` can produce.
#[test]
fn money_inbox_list_snooze_expiry_uses_household_local_today() {
    let (_dir, worker) = worker();
    let (local_today, utc_today) = disagreeing_household(&worker);

    let id = Uuid::now_v7();
    let until = local_today.max(utc_today);
    {
        let conn = worker.read_connection().unwrap();
        conn.execute(
            "INSERT INTO money_inbox_read_model
                    (item_id, item_kind, target_table, target_id, priority, surfaced_at,
                     snoozed_until, dismissed_at, resolved_at, payload_json)
                 VALUES (?1, 'imported_waiting_commit', 'staged_transactions', ?1, 20, ?2,
                         ?3, NULL, NULL, '{}')",
            params![id, Utc::now().to_rfc3339(), until.to_string()],
        )
        .unwrap();
    }

    let visible = worker
        .money_inbox_list()
        .unwrap()
        .iter()
        .any(|item| item.item_id == id);
    assert_eq!(
        visible,
        local_today > utc_today,
        "snooze expiry must be judged against the household-local date ({local_today}), \
         not UTC's ({utc_today}) — snoozed_until={until}"
    );
}

// ===========================================================================
// (2)-(4) The three view readers: next pay/due/occurrence date
// ===========================================================================
//
// All three feed `today` into `PaySchedule::new(frequency, anchor).next_pay_date(today)`.
// `Frequency::EveryNDays(1)` makes every calendar date a schedule date, so
// `next_pay_date(today) == Some(today)` for ANY anchor — the returned date becomes
// a direct readout of whichever "today" the function actually resolved.

#[test]
fn income_source_next_pay_date_uses_household_local_today() {
    let (_dir, worker) = worker();
    let (local_today, _utc_today) = disagreeing_household(&worker);

    worker
        .dispatch(
            meta(),
            WriteCommand::CreateIncomeSource {
                id: IncomeSourceId::new(),
                name: "Acme Corp".to_owned(),
                net_amount: Money::new(300_000, Currency::Usd),
                frequency: Frequency::EveryNDays(1),
                anchor: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
                deposit_account_id: None,
            },
        )
        .unwrap();

    let views = worker.income_source_views().unwrap();
    assert_eq!(views.len(), 1);
    assert_eq!(
        views[0].next_pay_date,
        Some(local_today),
        "next pay date must be anchored on the household-local today, not UTC's"
    );
}

#[test]
fn recurring_bill_next_due_date_uses_household_local_today() {
    let (_dir, worker) = worker();
    let (local_today, _utc_today) = disagreeing_household(&worker);

    worker
        .dispatch(
            meta(),
            WriteCommand::CreateRecurringBill {
                event_id: RecurringEventId::new(),
                contract_id: BillContractId::new(),
                name: "Rent".to_owned(),
                amount: Money::new(180_000, Currency::Usd),
                bill_type: "rent_mortgage".to_owned(),
                frequency: Frequency::EveryNDays(1),
                anchor: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
                autopay_account_id: None,
                description: None,
                source_merchant_key: None,
                category_id: None,
                tag_ids: Vec::new(),
            },
        )
        .unwrap();

    let views = worker.recurring_bill_views().unwrap();
    assert_eq!(views.len(), 1);
    assert_eq!(
        views[0].next_due_date,
        Some(local_today),
        "next due date must be anchored on the household-local today, not UTC's"
    );
}

#[test]
fn recurring_transfer_next_date_uses_household_local_today() {
    let (_dir, worker) = worker();
    let (local_today, _utc_today) = disagreeing_household(&worker);

    let source = AccountId::new();
    let dest = AccountId::new();
    create_role_account(&worker, source, "Checking", CashflowRole::LiquidCash);
    create_role_account(&worker, dest, "Savings", CashflowRole::LiquidCash);
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateRecurringTransfer {
                id: RecurringTransferId::new(),
                source_account_id: source,
                dest_account_id: dest,
                amount: Money::new(30_000, Currency::Usd),
                frequency: Frequency::EveryNDays(1),
                anchor: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            },
        )
        .unwrap();

    let views = worker.recurring_transfer_views().unwrap();
    assert_eq!(views.len(), 1);
    assert_eq!(
        views[0].next_date,
        Some(local_today),
        "next occurrence date must be anchored on the household-local today, not UTC's"
    );
}

// ===========================================================================
// (5)-(6) recurring_candidates / income_candidates — the roll_forward boundary
// ===========================================================================
//
// The candidate SET is today-independent (categorization::detect_recurring's
// only today-sensitive line is `roll_forward(next, today, interval)` inside its
// next-date projection), so these two tests target `candidate.next_date`
// specifically, not inclusion/exclusion. Three biweekly observations ending 14
// days before `boundary = min(local, utc)` put `next_raw` (= last + 14 days)
// exactly ON `boundary`: whichever "today" is actually `boundary` sees
// `next_raw >= today` and keeps it; the other, later "today" rolls it forward a
// full 14-day cadence step. The two answers always differ by exactly 14 days.

/// Plants a 4-observation biweekly series (one extra beyond the 3-occurrence
/// detection minimum, for margin) ending 14 days before `boundary`, on `account`
/// with `cents` per occurrence (negative for an outflow/bill candidate, positive
/// for an inflow/income candidate).
fn plant_biweekly_series(worker: &DbWorker, account: AccountId, cents: i64, boundary: NaiveDate) {
    for back in [56i64, 42, 28, 14] {
        record_with_counterparty(
            worker,
            account,
            cents,
            boundary - Duration::days(back),
            "ACME RECURRING",
        );
    }
}

/// Asserts `candidate.next_date` matches the household-local resolution and
/// differs from what the UTC-buggy resolution would have produced — proving
/// the assertion is not vacuous, the same "control" discipline
/// `scripts/tests/*.test.sh` use elsewhere in this repo.
fn assert_next_date_is_household_local(
    next_date: NaiveDate,
    boundary: NaiveDate,
    local_today: NaiveDate,
    utc_today: NaiveDate,
) {
    let expected = if local_today <= boundary {
        boundary
    } else {
        boundary + Duration::days(14)
    };
    let buggy = if utc_today <= boundary {
        boundary
    } else {
        boundary + Duration::days(14)
    };
    assert_ne!(
        expected, buggy,
        "sanity: the fixture's boundary must actually discriminate local vs. UTC"
    );
    assert_eq!(
        next_date, expected,
        "next_date must roll forward against the household-local today ({local_today}), \
         not UTC's ({utc_today}) — boundary={boundary}"
    );
}

#[test]
fn recurring_candidate_next_date_uses_household_local_today() {
    let (_dir, worker) = worker();
    let (local_today, utc_today) = disagreeing_household(&worker);
    let boundary = local_today.min(utc_today);

    let checking = AccountId::new();
    create_role_account(&worker, checking, "Checking", CashflowRole::LiquidCash);
    plant_biweekly_series(&worker, checking, -1_099, boundary);

    let candidates = worker.recurring_candidates().unwrap();
    assert_eq!(candidates.len(), 1, "{candidates:?}");
    let c = &candidates[0].candidate;
    assert_eq!(c.frequency, "biweekly");
    assert_eq!(c.last_seen, boundary - Duration::days(14));
    assert_next_date_is_household_local(c.next_date, boundary, local_today, utc_today);
}

#[test]
fn income_candidate_next_date_uses_household_local_today() {
    let (_dir, worker) = worker();
    let (local_today, utc_today) = disagreeing_household(&worker);
    let boundary = local_today.min(utc_today);

    let checking = AccountId::new();
    create_role_account(&worker, checking, "Checking", CashflowRole::LiquidCash);
    plant_biweekly_series(&worker, checking, 250_000, boundary);

    let candidates = worker.income_candidates().unwrap();
    assert_eq!(candidates.len(), 1, "{candidates:?}");
    let c = &candidates[0].candidate;
    assert_eq!(c.frequency, "biweekly");
    assert_eq!(c.last_seen, boundary - Duration::days(14));
    assert_next_date_is_household_local(c.next_date, boundary, local_today, utc_today);
}

// ===========================================================================
// Self-check on the zone-selection helper itself
// ===========================================================================

/// The `[0, 11)` ∪ `[10, 24)` hour-coverage claim in `zone_that_disagrees_with_utc`'s
/// doc comment, checked directly rather than trusted — every hour of the day picks
/// a zone whose arithmetic actually disagrees with that same hour's UTC date.
#[test]
fn zone_selection_covers_every_utc_hour() {
    let base = Utc::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc();
    for hour in 0..24 {
        let now = base + Duration::hours(hour) + Duration::minutes(30);
        let (zone, local_date) = zone_that_disagrees_with_utc(now);
        assert_ne!(
            local_date,
            now.date_naive(),
            "hour {hour} UTC: {zone}'s computed local date must differ from UTC's"
        );
    }
}

/// `zone_that_disagrees_with_utc`'s date arithmetic must actually track calendar
/// day boundaries, not just always return "yesterday"/"tomorrow" — checked
/// against `Datelike` so a future refactor that breaks the day-rollover math
/// (e.g. an off-by-one in the hour thresholds) fails loudly here rather than
/// only in the household-timezone tests above.
#[test]
fn zone_selection_date_arithmetic_is_a_real_day_offset() {
    let now = Utc::now();
    let (_, local_date) = zone_that_disagrees_with_utc(now);
    let diff_days = (local_date - now.date_naive()).num_days();
    assert!(
        diff_days == 1 || diff_days == -1,
        "expected exactly a one-day offset in either direction, got {diff_days} \
         (local={local_date}, utc={})",
        now.date_naive()
    );
    // And the offset direction matches which zone was picked.
    let (zone, _) = zone_that_disagrees_with_utc(now);
    if zone == "Pacific/Kiritimati" {
        assert_eq!(
            diff_days, 1,
            "Kiritimati (UTC+14) must be one day AHEAD of UTC"
        );
    } else {
        assert_eq!(diff_days, -1, "Niue (UTC-11) must be one day BEHIND UTC");
    }
}
