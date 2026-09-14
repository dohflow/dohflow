//! Income-source candidates from recurring inbound deposits (personal-cfo-gmnk,
//! onboarding epic 5fp6): the recurring detector pointed at the inflow side,
//! sharing the bill suggestions' suppression store.

mod common;

use chrono::{Days, NaiveDate, Utc};
use common::*;
use core_ledger::{
    Account, AccountFlags, AccountId, BillContractId, CashflowRole, IncomeSourceId,
    LedgerAccountId, RecurringEventId,
};
use core_money::{Currency, Money};
use db_worker::*;
use pay_schedule::Frequency;

fn liquid(worker: &DbWorker, name: &str) -> AccountId {
    let account_id = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(Account::new(
                    account_id,
                    LedgerAccountId::new(),
                    name,
                    CashflowRole::LiquidCash,
                    Currency::Usd,
                    AccountFlags::default(),
                )),
                opening_balance: None,
            },
        )
        .unwrap();
    account_id
}

/// Biweekly deposits `count` times, ending today.
fn deposits(worker: &DbWorker, account: AccountId, payee: &str, cents: i64, count: u64) {
    let today = Utc::now().date_naive();
    for i in 0..count {
        let date: NaiveDate = today
            .checked_sub_days(Days::new(14 * (count - 1 - i)))
            .unwrap();
        record_with_counterparty(worker, account, cents, date, payee);
    }
}

#[test]
fn recurring_deposits_become_income_candidates_with_their_account() {
    let (_dir, worker) = worker();
    let checking = liquid(&worker, "Checking");
    deposits(&worker, checking, "ACME PAYROLL", 250_000, 4);
    deposits(&worker, checking, "RIVERSIDE TENANT", 180_000, 4);
    // Outflows never become income, however regular.
    deposits(&worker, checking, "CITY WATER", -6_000, 4);

    let candidates = worker.income_candidates().unwrap();
    let names: Vec<&str> = candidates
        .iter()
        .map(|c| c.candidate.merchant_key.as_str())
        .collect();
    assert_eq!(candidates.len(), 2, "{names:?}");
    let payroll = candidates
        .iter()
        .find(|c| c.candidate.display.contains("PAYROLL"))
        .unwrap();
    assert_eq!(payroll.candidate.amount_minor, 250_000);
    assert_eq!(payroll.candidate.frequency, "biweekly");
    // Single-account series → the deposit-account prefill (ADR 0047 §4 idiom).
    assert_eq!(
        payroll.source_account_id,
        Some(checking.as_uuid().to_owned())
    );
    assert!(!names.iter().any(|n| n.contains("water")));

    // And the bill detector still ignores the deposits.
    let bills = worker.recurring_candidates().unwrap();
    assert!(
        bills
            .iter()
            .all(|c| !c.candidate.display.contains("PAYROLL")),
        "{bills:?}"
    );
}

#[test]
fn an_existing_income_source_consumes_its_candidate() {
    let (_dir, worker) = worker();
    let checking = liquid(&worker, "Checking");
    deposits(&worker, checking, "ACME PAYROLL", 250_000, 4);
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateIncomeSource {
                id: IncomeSourceId::new(),
                name: "Acme Payroll".to_owned(),
                net_amount: Money::new(250_000, Currency::Usd),
                frequency: Frequency::Biweekly,
                anchor: Utc::now().date_naive(),
                deposit_account_id: Some(checking),
            },
        )
        .unwrap();
    assert!(worker.income_candidates().unwrap().is_empty());
}

#[test]
fn a_dismissal_suppresses_the_income_candidate_too() {
    let (_dir, worker) = worker();
    let checking = liquid(&worker, "Checking");
    deposits(&worker, checking, "RIVERSIDE TENANT", 180_000, 4);
    let before = worker.income_candidates().unwrap();
    assert_eq!(before.len(), 1);
    let c = &before[0].candidate;
    worker
        .dispatch(
            meta(),
            WriteCommand::DismissRecurringSuggestion {
                merchant_key: c.merchant_key.clone(),
                currency: c.currency.clone(),
                amount_minor: c.amount_minor,
                frequency: c.frequency.to_owned(),
                reason: None,
            },
        )
        .unwrap();
    assert!(worker.income_candidates().unwrap().is_empty());
}

fn card(worker: &DbWorker) -> AccountId {
    let account_id = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(Account::new(
                    account_id,
                    LedgerAccountId::new(),
                    "Card",
                    CashflowRole::CreditFacility,
                    Currency::Usd,
                    AccountFlags::default(),
                )),
                opening_balance: None,
            },
        )
        .unwrap();
    account_id
}

/// A card refund stream is not income: only liquid accounts feed the inflow
/// detector.
#[test]
fn credit_facility_inflows_are_not_income() {
    let (_dir, worker) = worker();
    let card = card(&worker);
    deposits(&worker, card, "MERCHANT REFUND", 4_000, 4);
    assert!(worker.income_candidates().unwrap().is_empty());
}

/// gmnk review: the bill detector's evidence keys are BILL identities and must
/// never consume an income candidate — a payee that both bills you and pays
/// you keeps its income suggestion.
#[test]
fn a_bills_linked_evidence_does_not_consume_the_income_candidate() {
    let (_dir, worker) = worker();
    let checking = liquid(&worker, "Checking");
    let today = Utc::now().date_naive();
    // An active monthly bill with three linked payments (the evidence seam).
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateRecurringBill {
                event_id: RecurringEventId::new(),
                contract_id: BillContractId::new(),
                name: "Riverside HOA".to_owned(),
                amount: Money::new(30_000, Currency::Usd),
                bill_type: "subscription".to_owned(),
                frequency: Frequency::Monthly,
                anchor: today.checked_sub_days(Days::new(60)).unwrap(),
                autopay_account_id: Some(checking),
                description: None,
                source_merchant_key: None,
                category_id: None,
                tag_ids: Vec::new(),
            },
        )
        .unwrap();
    for back in [60u64, 30, 0] {
        record_with_counterparty(
            &worker,
            checking,
            -30_000,
            today.checked_sub_days(Days::new(back)).unwrap(),
            "RIVERSIDE HOA",
        );
    }
    worker.rebuild_recurring_instances().unwrap();
    // The same payee also deposits on a schedule (a recurring reimbursement).
    deposits(&worker, checking, "RIVERSIDE HOA", 12_500, 4);

    let candidates = worker.income_candidates().unwrap();
    assert!(
        candidates
            .iter()
            .any(|c| c.candidate.display.eq_ignore_ascii_case("RIVERSIDE HOA")),
        "bill evidence must not consume the income candidate: {candidates:?}"
    );
}
