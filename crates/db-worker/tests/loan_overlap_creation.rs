//! The loan-payment overlap on the two surfaces that come BEFORE a bill exists
//! (personal-cfo-6wk.20): a suggested recurring candidate, and a bill the user is about to create.
//! Fake data only.

mod common;

use chrono::{NaiveDate, TimeZone, Utc};
use common::*;
use core_ledger::{AccountId, CashflowRole};
use core_money::{Currency, Money};
use db_worker::*;
use pay_schedule::Frequency;
use rusqlite::params;
use uuid::Uuid;

struct World {
    checking: AccountId,
    loan: AccountId,
}

/// Checking plus a mortgage loan account: fixed $1,500 a month, due the 15th, paid from checking.
fn world(worker: &DbWorker) -> World {
    let checking = AccountId::new();
    create_role_account(worker, checking, "Checking", CashflowRole::LiquidCash);
    let loan = create_loan_with_terms(
        worker,
        "Test Mortgage",
        RepaymentPhilosophy::PayFixedAmount,
        Some(150_000),
        checking,
    );
    World { checking, loan }
}

/// Six monthly $1,500 debits to the payee, on the 15th, from checking.
fn plant_the_series(worker: &DbWorker, account: AccountId, payee: &str, day: u32) {
    for month in 1..=6u32 {
        record_with_counterparty(
            worker,
            account,
            -150_000,
            NaiveDate::from_ymd_opt(2026, month, day).unwrap(),
            payee,
        );
    }
}

/// A payment from `checking` INTO the loan (a recorded transfer) whose imported payee is `payee`.
fn pair_a_payment(worker: &DbWorker, world: &World, payee: &str) {
    pair_a_payment_into(worker, world.checking, world.loan, payee);
}

fn pair_a_payment_into(worker: &DbWorker, from: AccountId, loan: AccountId, payee: &str) {
    worker
        .dispatch(
            meta(),
            WriteCommand::Transfer {
                source_account_id: from,
                dest_account_id: loan,
                amount: Money::new(150_000, Currency::Usd),
                occurred_at: Utc.with_ymd_and_hms(2026, 7, 15, 12, 0, 0).unwrap(),
            },
        )
        .unwrap();
    let conn = worker.read_connection().unwrap();
    let transaction_id: Uuid = conn
        .query_row(
            "SELECT lp.transaction_id
               FROM ledger_postings lp
               JOIN accounts a ON a.ledger_account_id = lp.ledger_account_id
              WHERE a.id = ?1 AND lp.minor_units > 0",
            [loan.as_uuid()],
            |r| r.get(0),
        )
        .unwrap();
    conn.execute(
        "INSERT OR REPLACE INTO transaction_details
            (transaction_id, memo, counterparty, transaction_date, created_at)
         VALUES (?1, NULL, ?2, NULL, '2026-07-15T12:00:00+00:00')",
        params![transaction_id, payee],
    )
    .unwrap();
}

fn draft(
    name: &str,
    bill_type: &str,
    amount_minor: i64,
    frequency: Frequency,
    day: u32,
    from: AccountId,
) -> LoanOverlapDraft {
    LoanOverlapDraft {
        name: name.to_owned(),
        merchant_key: None,
        bill_type: bill_type.to_owned(),
        amount: Money::new(amount_minor, Currency::Usd),
        frequency,
        anchor: NaiveDate::from_ymd_opt(2026, 7, day).unwrap(),
        autopay_account_id: Some(from.as_uuid()),
    }
}

// ----- suggested recurring candidates (AC3) ------------------------------------------------------------

fn candidate<'a>(
    candidates: &'a [RecurringCandidateView],
    key: &str,
) -> &'a RecurringCandidateView {
    candidates
        .iter()
        .find(|c| c.candidate.merchant_key == key)
        .unwrap_or_else(|| panic!("no candidate {key}"))
}

/// The owner's scenario: imports only, so no payment was ever recorded INTO the loan. The suggested
/// payee has no paired-payment evidence, so it can only be a soft note, never a "this is the loan's
/// payment" cancel.
#[test]
fn a_suggestion_that_fits_the_loans_payment_but_has_no_paired_payments_is_only_possible() {
    let (_dir, worker) = worker();
    let w = world(&worker);
    plant_the_series(&worker, w.checking, "ACME HOME LOANS", 15);
    let candidates = worker.recurring_candidates().unwrap();
    let found = candidate(&candidates, "ACME HOME LOANS")
        .loan_match
        .clone()
        .unwrap();
    assert_eq!(found.loan_account_id, w.loan.as_uuid());
    assert_eq!(found.loan_name, "Test Mortgage");
    assert_eq!(found.strength, MatchStrength::Possible);
    assert_eq!(found.matched_by, None);
}

/// Once a payment into the loan has been recorded with the same payee, the suggestion IS the loan's
/// payment (the payee route): strong, so the UI will not offer to add it as a second bill.
#[test]
fn a_suggestion_with_the_payee_of_the_loans_paired_payments_is_strong() {
    let (_dir, worker) = worker();
    let w = world(&worker);
    plant_the_series(&worker, w.checking, "ACME HOME LOANS", 15);
    pair_a_payment(&worker, &w, "ACME HOME LOANS");
    let candidates = worker.recurring_candidates().unwrap();
    let found = candidate(&candidates, "ACME HOME LOANS")
        .loan_match
        .clone()
        .unwrap();
    assert_eq!(found.strength, MatchStrength::Strong);
    assert_eq!(found.matched_by, Some(MatchedBy::Payee));
}

#[test]
fn an_unrelated_suggestion_has_no_loan_match() {
    let (_dir, worker) = worker();
    let w = world(&worker);
    for month in 1..=6u32 {
        record_with_counterparty(
            &worker,
            w.checking,
            -1_099,
            NaiveDate::from_ymd_opt(2026, month, 5).unwrap(),
            "SPOTIFY",
        );
    }
    let candidates = worker.recurring_candidates().unwrap();
    assert!(candidate(&candidates, "SPOTIFY").loan_match.is_none());
}

/// The same $1,500 every month but on the 25th: ten days from the loan's due day, so not its payment.
#[test]
fn a_suggestion_on_another_day_of_the_month_has_no_loan_match() {
    let (_dir, worker) = worker();
    let w = world(&worker);
    plant_the_series(&worker, w.checking, "ACME HOME LOANS", 25);
    let candidates = worker.recurring_candidates().unwrap();
    assert!(candidate(&candidates, "ACME HOME LOANS")
        .loan_match
        .is_none());
}

// ----- a bill about to be created (AC4) -----------------------------------------------------------------

#[test]
fn a_loan_typed_bill_for_the_loans_payment_is_strong_before_it_is_created() {
    let (_dir, worker) = worker();
    let w = world(&worker);
    let found = worker
        .check_loan_overlap(&draft(
            "ACME HOME LOANS",
            "loan_payment",
            150_000,
            Frequency::Monthly,
            15,
            w.checking,
        ))
        .unwrap()
        .unwrap();
    assert_eq!(found.loan_account_id, w.loan.as_uuid());
    assert_eq!(found.strength, MatchStrength::Strong);
    assert_eq!(found.matched_by, Some(MatchedBy::LoanTyped));
}

#[test]
fn a_rent_or_mortgage_bill_with_escrow_is_still_strong() {
    let (_dir, worker) = worker();
    let w = world(&worker);
    let found = worker
        .check_loan_overlap(&draft(
            "Mortgage",
            "rent_mortgage",
            270_000,
            Frequency::Monthly,
            15,
            w.checking,
        ))
        .unwrap()
        .unwrap();
    assert_eq!(found.strength, MatchStrength::Strong);
}

/// The default type (what the form pre-selects) with no payee evidence: a soft note only.
#[test]
fn a_subscription_typed_bill_for_the_same_amount_is_only_possible() {
    let (_dir, worker) = worker();
    let w = world(&worker);
    let found = worker
        .check_loan_overlap(&draft(
            "ACME HOME LOANS",
            "subscription",
            150_000,
            Frequency::Monthly,
            15,
            w.checking,
        ))
        .unwrap()
        .unwrap();
    assert_eq!(found.strength, MatchStrength::Possible);
}

#[test]
fn a_bill_carrying_the_paired_payments_payee_is_strong_whatever_its_type() {
    let (_dir, worker) = worker();
    let w = world(&worker);
    pair_a_payment(&worker, &w, "ACME HOME LOANS");
    let found = worker
        .check_loan_overlap(&draft(
            "Acme Home Loans",
            "subscription",
            150_000,
            Frequency::Monthly,
            15,
            w.checking,
        ))
        .unwrap()
        .unwrap();
    assert_eq!(found.strength, MatchStrength::Strong);
    assert_eq!(found.matched_by, Some(MatchedBy::Payee));
}

#[test]
fn unrelated_or_off_cadence_bills_are_not_matches() {
    let (_dir, worker) = worker();
    let w = world(&worker);
    for (label, d) in [
        (
            "an unrelated subscription",
            draft(
                "Streaming",
                "subscription",
                1_599,
                Frequency::Monthly,
                15,
                w.checking,
            ),
        ),
        (
            "a weekly bill of the same amount",
            draft(
                "Contractor",
                "loan_payment",
                150_000,
                Frequency::Weekly,
                15,
                w.checking,
            ),
        ),
        (
            "the same amount on the 25th",
            draft(
                "Home payment",
                "loan_payment",
                150_000,
                Frequency::Monthly,
                25,
                w.checking,
            ),
        ),
        (
            "a 6x bill",
            draft(
                "Big bill",
                "loan_payment",
                900_000,
                Frequency::Monthly,
                15,
                w.checking,
            ),
        ),
    ] {
        assert!(worker.check_loan_overlap(&d).unwrap().is_none(), "{label}");
    }
}

/// The check only looks: it creates nothing and changes nothing (ADR 0018).
#[test]
fn the_check_is_read_only() {
    let (_dir, worker) = worker();
    let w = world(&worker);
    let before = worker.recurring_bill_views().unwrap().len();
    worker
        .check_loan_overlap(&draft(
            "ACME HOME LOANS",
            "loan_payment",
            150_000,
            Frequency::Monthly,
            15,
            w.checking,
        ))
        .unwrap();
    assert_eq!(worker.recurring_bill_views().unwrap().len(), before);
    assert!(worker.loan_double_count_warnings().unwrap().is_empty());
}

/// Nothing to compare with: no loan, no match.
#[test]
fn with_no_loan_there_is_never_a_match() {
    let (_dir, worker) = worker();
    let checking = AccountId::new();
    create_role_account(&worker, checking, "Checking", CashflowRole::LiquidCash);
    assert!(worker
        .check_loan_overlap(&draft(
            "ACME HOME LOANS",
            "loan_payment",
            150_000,
            Frequency::Monthly,
            15,
            checking
        ))
        .unwrap()
        .is_none());
}

/// With several loans the STRONGEST match wins, not merely the first by name: "Alpha" (first by name)
/// only possibly matches, "Zeta" matches through the payee of its own paired payments.
#[test]
fn the_strongest_loan_match_wins_over_an_earlier_weaker_one() {
    let (_dir, worker) = worker();
    let w = world(&worker);
    // "Test Mortgage" sorts after "Alpha Loan"; both are $1,500 on the 15th from checking.
    let alpha = create_loan_with_terms(
        &worker,
        "Alpha Loan",
        RepaymentPhilosophy::PayFixedAmount,
        Some(150_000),
        w.checking,
    );
    pair_a_payment_into(&worker, w.checking, w.loan, "ACME HOME LOANS");
    let found = worker
        .check_loan_overlap(&draft(
            "Acme Home Loans",
            "subscription",
            150_000,
            Frequency::Monthly,
            15,
            w.checking,
        ))
        .unwrap()
        .unwrap();
    assert_ne!(
        found.loan_account_id,
        alpha.as_uuid(),
        "the weaker Alpha Loan must not win"
    );
    assert_eq!(found.loan_account_id, w.loan.as_uuid());
    assert_eq!(found.strength, MatchStrength::Strong);
}

/// A bill in another currency than the forecast's is never compared with the loans: the forecast has
/// no offline exchange rate, so "the same amount" would mean nothing.
#[test]
fn a_bill_in_another_currency_is_never_a_match() {
    let (_dir, worker) = worker();
    let w = world(&worker);
    let mut euro = draft(
        "ACME HOME LOANS",
        "loan_payment",
        150_000,
        Frequency::Monthly,
        15,
        w.checking,
    );
    euro.amount = Money::new(150_000, Currency::Eur);
    assert!(worker.check_loan_overlap(&euro).unwrap().is_none());
    // The same bill in the forecast's currency does match, so the guard is what made the difference.
    euro.amount = Money::new(150_000, Currency::Usd);
    assert!(worker.check_loan_overlap(&euro).unwrap().is_some());
}
