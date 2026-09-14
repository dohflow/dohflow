//! Auto-reconciliation of projections against real transactions
//! (personal-cfo-xtz5, ADR 0026 addendum 2026-09-02): linked instances
//! suppress forecast projections like confirms, and the deterministic
//! one-off entry matcher clears matched manual entries.

mod common;

use chrono::{Days, Utc};
use common::*;
use core_ledger::{
    Account, AccountFlags, AccountId, BillContractId, CashflowRole, LedgerAccountId,
    RecurringEventId, TransactionId,
};
use core_money::{Currency, Money};
use db_worker::*;
use pay_schedule::Frequency;
use uuid::Uuid;

fn checking(worker: &DbWorker) -> AccountId {
    let account_id = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(Account::new(
                    account_id,
                    LedgerAccountId::new(),
                    "Checking",
                    CashflowRole::LiquidCash,
                    Currency::Usd,
                    AccountFlags::default(),
                )),
                opening_balance: Some(Money::new(500_000, Currency::Usd)),
            },
        )
        .unwrap();
    account_id
}

fn record(worker: &DbWorker, account_id: AccountId, minor: i64, date: chrono::NaiveDate) {
    worker
        .dispatch(
            meta(),
            WriteCommand::RecordTransaction {
                transaction_id: TransactionId::new(),
                account_id,
                amount: Money::new(minor, Currency::Usd),
                occurred_at: date.and_hms_opt(12, 0, 0).unwrap().and_utc(),
            },
        )
        .unwrap();
}

/// The exact double-count the addendum kills: a payment lands (synced or
/// typed) days before its due date; the instance seam links it; the forecast
/// must consume the projection instead of subtracting the bill again.
#[test]
fn linked_instance_suppresses_the_projection_like_a_confirm() {
    let (_dir, worker) = worker();
    let account = checking(&worker);
    let today = Utc::now().date_naive();
    let due = today.checked_add_days(Days::new(2)).unwrap();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateRecurringBill {
                event_id: RecurringEventId::new(),
                contract_id: BillContractId::new(),
                name: "Rent".to_owned(),
                amount: Money::new(180_000, Currency::Usd),
                bill_type: "subscription".to_owned(),
                frequency: Frequency::Monthly,
                anchor: due,
                autopay_account_id: Some(account),
                description: None,
                source_merchant_key: None,
                category_id: None,
                tag_ids: Vec::new(),
            },
        )
        .unwrap();

    // Before any payment: the projection drops the closing balance at +2.
    let before = worker.future_cash_forecast(10, &[]).unwrap();
    let start = before.days.first().unwrap().closing.p50.minor_units();
    let end_before = before.days.last().unwrap().closing.p50.minor_units();
    assert_eq!(
        start - end_before,
        180_000,
        "unpaid bill projects one outflow"
    );

    // The payment arrives a day early; the seam links it on rebuild.
    record(
        &worker,
        account,
        -180_000,
        today.checked_sub_days(Days::new(1)).unwrap(),
    );
    worker.rebuild_recurring_instances().unwrap();

    let after = worker.future_cash_forecast(10, &[]).unwrap();
    let start_after = after.days.first().unwrap().closing.p50.minor_units();
    let end_after = after.days.last().unwrap().closing.p50.minor_units();
    // The real posting is in the starting balance; the projection is consumed
    // — no second subtraction.
    assert_eq!(start_after, start - 180_000, "payment is in the balance");
    assert_eq!(
        end_after, start_after,
        "no double-count: projection suppressed by the link"
    );
}

/// The one-off matcher: strict v1 rules, deterministic, one claim per txn,
/// instances win over entries.
#[test]
fn manual_entry_matcher_is_strict_and_deterministic() {
    let (_dir, worker) = worker();
    let account = checking(&worker);
    let today = Utc::now().date_naive();
    let entry_date = today.checked_sub_days(Days::new(3)).unwrap();

    let attributed = Uuid::now_v7();
    worker
        .record_manual_entry(
            attributed,
            Money::new(-260_000, Currency::Usd),
            entry_date,
            "Celso final payment",
            Some(account.as_uuid().to_owned()),
        )
        .unwrap();
    let unattributed = Uuid::now_v7();
    worker
        .record_manual_entry(
            unattributed,
            Money::new(-260_000, Currency::Usd),
            entry_date,
            "No account chosen",
            None,
        )
        .unwrap();

    // Wrong amount and out-of-window txns never match.
    record(&worker, account, -260_001, entry_date);
    record(
        &worker,
        account,
        -260_000,
        entry_date.checked_sub_days(Days::new(20)).unwrap(),
    );
    worker.rebuild_recurring_instances().unwrap();
    let entries = worker.manual_entries().unwrap();
    assert!(
        entries.iter().all(|e| e.matched_transaction_id.is_none()),
        "{entries:?}"
    );

    // An exact-amount txn one day off matches the ATTRIBUTED entry only.
    record(
        &worker,
        account,
        -260_000,
        entry_date.checked_add_days(Days::new(1)).unwrap(),
    );
    worker.rebuild_recurring_instances().unwrap();
    let entries = worker.manual_entries().unwrap();
    let matched = entries.iter().find(|e| e.id == attributed).unwrap();
    let unmatched = entries.iter().find(|e| e.id == unattributed).unwrap();
    let linked = matched.matched_transaction_id.expect("attributed matches");
    assert!(
        unmatched.matched_transaction_id.is_none(),
        "no account, no auto-match"
    );

    // Rebuilds are stable, and one transaction is never claimed twice.
    worker.rebuild_recurring_instances().unwrap();
    let again = worker.manual_entries().unwrap();
    assert_eq!(
        again
            .iter()
            .find(|e| e.id == attributed)
            .unwrap()
            .matched_transaction_id,
        Some(linked),
        "byte-stable rebuild"
    );

    // A matched past entry no longer projects — and since it is in the past,
    // the forecast simply must not error; the FutureCashEntries surface shows
    // the matched state (frontend test).
    worker.future_cash_forecast(10, &[]).unwrap();
}

fn weekly_bill(
    worker: &DbWorker,
    account: AccountId,
    anchor: chrono::NaiveDate,
) -> RecurringEventId {
    let event_id = RecurringEventId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateRecurringBill {
                event_id,
                contract_id: BillContractId::new(),
                name: "Weekly cleaner".to_owned(),
                amount: Money::new(15_000, Currency::Usd),
                bill_type: "subscription".to_owned(),
                frequency: Frequency::Weekly,
                anchor,
                autopay_account_id: Some(account),
                description: None,
                source_merchant_key: None,
                category_id: None,
                tag_ids: Vec::new(),
            },
        )
        .unwrap();
    event_id
}

fn drop_over(worker: &DbWorker, horizon: u32) -> i64 {
    let f = worker.future_cash_forecast(horizon, &[]).unwrap();
    f.days.first().unwrap().closing.p50.minor_units()
        - f.days.last().unwrap().closing.p50.minor_units()
}

/// Review blocker, variant B: a weekly bill routinely paid ON TIME last week
/// must still project next week — the past linked occurrence sits exactly one
/// tolerance-width away and the old nearest-match ate the upcoming one.
#[test]
fn weekly_bill_paid_last_week_still_projects_next_week() {
    let (_dir, worker) = worker();
    let account = checking(&worker);
    let today = Utc::now().date_naive();
    weekly_bill(
        &worker,
        account,
        today.checked_sub_days(Days::new(4)).unwrap(),
    );
    // Paid on time last week; the seam links it.
    record(
        &worker,
        account,
        -15_000,
        today.checked_sub_days(Days::new(4)).unwrap(),
    );
    worker.rebuild_recurring_instances().unwrap();
    // Horizon 8 holds exactly one upcoming occurrence (today+3) — it must
    // still drop the balance by one payment.
    assert_eq!(drop_over(&worker, 8), 15_000);
}

/// Review blocker, variant A: the same real payment confirmed (5ie.9) AND
/// seam-linked must consume exactly ONE projected occurrence, not two.
#[test]
fn confirmed_and_linked_payment_consumes_one_occurrence() {
    let (_dir, worker) = worker();
    let account = checking(&worker);
    let today = Utc::now().date_naive();
    let due = today.checked_add_days(Days::new(2)).unwrap();
    let event_id = weekly_bill(&worker, account, due);
    worker
        .dispatch(
            meta(),
            WriteCommand::ConfirmObligationEarly {
                recurring_event_id: event_id,
                scheduled_date: due,
                actual_amount: Money::new(15_000, Currency::Usd),
                actual_date: Utc::now(),
                paying_account_id: account,
            },
        )
        .unwrap();
    worker.rebuild_recurring_instances().unwrap();
    // Horizon 12 holds due (+2, paid) and the next (+9): exactly one must
    // still project.
    assert_eq!(drop_over(&worker, 12), 15_000);
}

/// Income gets the same treatment: an early-arriving paycheck is in the
/// balance and must not project again — but NEXT week's must.
#[test]
fn linked_income_suppresses_only_its_own_occurrence() {
    let (_dir, worker) = worker();
    let account = checking(&worker);
    let today = Utc::now().date_naive();
    let payday = today.checked_add_days(Days::new(2)).unwrap();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateIncomeSource {
                id: core_ledger::IncomeSourceId::new(),
                name: "Weekly pay".to_owned(),
                net_amount: Money::new(90_000, Currency::Usd),
                frequency: Frequency::Weekly,
                anchor: payday,
                deposit_account_id: Some(account),
            },
        )
        .unwrap();
    // The deposit landed a day early; the seam links it.
    record(
        &worker,
        account,
        90_000,
        today.checked_add_days(Days::new(1)).unwrap(),
    );
    worker.rebuild_recurring_instances().unwrap();
    let f = worker.future_cash_forecast(12, &[]).unwrap();
    let rise = f.days.last().unwrap().closing.p50.minor_units()
        - f.days.first().unwrap().closing.p50.minor_units();
    // Only the +9 payday still projects; the realized +2 does not double.
    assert_eq!(rise, 90_000);
}
