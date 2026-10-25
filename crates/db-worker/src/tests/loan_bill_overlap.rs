//! A loan's payment counted twice (personal-cfo-6wk.20): the loan account's projected payment AND a
//! recurring bill for the same payment. Reproduction first, on fake data only.

use super::*;
use crate::{MatchStrength, MatchedBy};
use chrono::TimeZone;

struct Fixture {
    checking: AccountId,
    loan: AccountId,
}

/// The owner's scenario, on fake data: checking, and a mortgage tracked as a loan account with payment
/// terms (due day 15, fixed $1,500 a month, paid from checking), owing $200,000.
fn mortgage(worker: &DbWorker) -> Fixture {
    let checking_id = AccountId::new();
    let loan = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(checking_id)),
                opening_balance: Some(Money::new(500_000, Currency::Usd)),
            },
        )
        .unwrap();
    create_role_account(worker, loan, "Test Mortgage", CashflowRole::LoanLiability);
    worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: loan,
                terms: loan_terms(
                    RepaymentPhilosophy::PayFixedAmount,
                    Some(150_000),
                    checking_id,
                ),
            },
        )
        .unwrap();
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            loan,
            Money::new(-20_000_000, Currency::Usd),
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        )
        .unwrap();
    Fixture {
        checking: checking_id,
        loan,
    }
}

/// A bill as Add bill / Make recurring / promoting a suggested recurring candidate create it (the
/// default type is `subscription`).
fn bill(
    worker: &DbWorker,
    name: &str,
    bill_type: &str,
    amount_minor: i64,
    frequency: Frequency,
    anchor: NaiveDate,
    autopay: AccountId,
) -> RecurringEventId {
    let event_id = RecurringEventId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateRecurringBill {
                event_id,
                contract_id: BillContractId::new(),
                name: name.to_owned(),
                amount: Money::new(amount_minor, Currency::Usd),
                bill_type: bill_type.to_owned(),
                frequency,
                anchor,
                autopay_account_id: Some(autopay),
                description: None,
                source_merchant_key: None,
                category_id: None,
                tag_ids: Vec::new(),
            },
        )
        .unwrap();
    event_id
}

fn the_15th() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 7, 15).unwrap()
}

/// A monthly bill on the loan's due day (the 15th), paid from `checking`.
fn bill_on_the_15th(
    worker: &DbWorker,
    name: &str,
    bill_type: &str,
    amount_minor: i64,
    checking: AccountId,
) -> RecurringEventId {
    bill(
        worker,
        name,
        bill_type,
        amount_minor,
        Frequency::Monthly,
        the_15th(),
        checking,
    )
}

/// How much leaves the liquid balance on the due date 2026-07-15, in the aggregate forecast.
fn outflow_on_the_15th(worker: &DbWorker) -> i64 {
    let as_of = chrono::Utc.with_ymd_and_hms(2026, 7, 1, 12, 0, 0).unwrap();
    let conn = worker.read_connection().unwrap();
    let view = crate::forecast::compute(&conn, as_of, 45, &[]).unwrap();
    let target = NaiveDate::from_ymd_opt(2026, 7, 15).unwrap();
    let mut previous = view.starting_balance.minor_units();
    for day in &view.days {
        let closing = day.closing.p50.minor_units();
        if day.date == target {
            return previous - closing;
        }
        previous = closing;
    }
    panic!("2026-07-15 is not in the forecast window");
}

#[test]
fn the_loan_alone_projects_one_payment() {
    let (_dir, worker) = worker();
    mortgage(&worker);
    assert_eq!(outflow_on_the_15th(&worker), 150_000);
}

/// The owner's report (06-dogfood #7): the bill suggested for the mortgage's payment is added with its
/// default type, `subscription`. Before 6wk.20 the detector only looked at `loan_payment` bills and was
/// silent; it now flags a bill of ANY type whose schedule and amount line up with the loan's payment.
#[test]
fn a_subscription_typed_bill_for_the_loans_payment_is_flagged_as_possible() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    bill_on_the_15th(
        &worker,
        "ACME HOME LOANS",
        "subscription",
        150_000,
        fixture.checking,
    );

    let warnings = worker.loan_double_count_warnings().unwrap();
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].loan_account_id, fixture.loan.as_uuid());
    assert_eq!(warnings[0].bill_name, "ACME HOME LOANS");
    assert!(
        warnings[0].amount_match,
        "the $1,500 payment matches the bill"
    );
    assert!(
        !warnings[0].name_match,
        "the payee name has nothing in common with the loan's"
    );
    // A generic bill has no real evidence it is the loan's payment (no payee in common with the
    // loan's paired payments, not loan-typed): a soft note, never "counted twice" or a link offer.
    assert_eq!(warnings[0].strength, MatchStrength::Possible);
    // KNOWN GAP until the forecast-level link (the second phase of 6wk.20): the warning is
    // descriptive (ADR 0018), so the forecast still counts both until the user removes one.
    assert_eq!(outflow_on_the_15th(&worker), 300_000);
}

#[test]
fn a_rent_mortgage_typed_bill_from_the_funding_account_is_strong() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    bill_on_the_15th(
        &worker,
        "Test Mortgage",
        "rent_mortgage",
        150_000,
        fixture.checking,
    );
    let warnings = worker.loan_double_count_warnings().unwrap();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].name_match && warnings[0].amount_match);
    assert_eq!(
        warnings[0].strength,
        MatchStrength::Strong,
        "loan-typed, from the funding account, on the due day, at 100%"
    );
}

/// A bill must be MONTHLY to be the loan's payment: a weekly `loan_payment` bill is not flagged, whatever
/// its amount or name (names are no evidence at all).
#[test]
fn a_loan_payment_typed_bill_on_another_cadence_is_not_flagged() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    for name in ["ACME HOME LOANS", "Test Mortgage payment"] {
        bill(
            &worker,
            name,
            "loan_payment",
            150_000,
            Frequency::Weekly,
            NaiveDate::from_ymd_opt(2026, 7, 3).unwrap(),
            fixture.checking,
        );
    }
    assert!(worker.loan_double_count_warnings().unwrap().is_empty());
}

/// A name that resembles the loan's, on its own, is not evidence: a monthly bill on the due day with a
/// much larger amount is no match even if it is called "Test Mortgage".
#[test]
fn a_matching_name_alone_is_no_evidence() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    bill_on_the_15th(
        &worker,
        "Test Mortgage",
        "loan_payment",
        900_000,
        fixture.checking,
    );
    assert!(worker.loan_double_count_warnings().unwrap().is_empty());
}

/// A minimum-payment loan has no fixed amount, but the forecast derives one (1% of the balance, or the
/// $25 floor); a bill equal to that derived payment is the same payment.
#[test]
fn a_minimum_payment_loan_is_matched_on_its_derived_payment() {
    let (_dir, worker) = worker();
    let checking_id = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(checking_id)),
                opening_balance: Some(Money::new(500_000, Currency::Usd)),
            },
        )
        .unwrap();
    let loan = AccountId::new();
    create_role_account(&worker, loan, "Student Debt", CashflowRole::LoanLiability);
    worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: loan,
                terms: loan_terms(RepaymentPhilosophy::PayMinimum, None, checking_id),
            },
        )
        .unwrap();
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            loan,
            Money::new(-5_000_000, Currency::Usd), // owes $50,000: the default minimum is 1% = $500
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        )
        .unwrap();
    bill_on_the_15th(&worker, "NELNET", "subscription", 50_000, checking_id);
    let warnings = worker.loan_double_count_warnings().unwrap();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].amount_match);
    assert_eq!(
        outflow_on_the_15th(&worker),
        100_000,
        "the loan's $500 and the bill's $500"
    );
}

// ----- what must NOT be flagged -----------------------------------------------------------------

#[test]
fn an_unrelated_subscription_is_not_flagged() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    bill_on_the_15th(
        &worker,
        "Streaming service",
        "subscription",
        1_599,
        fixture.checking,
    );
    assert!(worker.loan_double_count_warnings().unwrap().is_empty());
}

#[test]
fn an_equal_amount_on_a_different_cadence_is_not_flagged_for_a_generic_bill() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    // $1,500 every week is not the monthly payment, even from the same account.
    bill(
        &worker,
        "Contractor",
        "subscription",
        150_000,
        Frequency::Weekly,
        the_15th(),
        fixture.checking,
    );
    assert!(worker.loan_double_count_warnings().unwrap().is_empty());
}

#[test]
fn a_coincidentally_equal_amount_on_another_day_from_another_account_is_not_flagged() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    let savings = AccountId::new();
    create_role_account(&worker, savings, "Savings", CashflowRole::LiquidCash);
    bill(
        &worker,
        "Tuition",
        "subscription",
        150_000,
        Frequency::Monthly,
        NaiveDate::from_ymd_opt(2026, 7, 2).unwrap(),
        savings,
    );
    let _ = fixture;
    assert!(worker.loan_double_count_warnings().unwrap().is_empty());
}

#[test]
fn a_shared_word_in_the_name_alone_does_not_flag_a_generic_bill_on_another_schedule() {
    let (_dir, worker) = worker();
    let checking_id = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(checking_id)),
                opening_balance: Some(Money::new(500_000, Currency::Usd)),
            },
        )
        .unwrap();
    let savings = AccountId::new();
    create_role_account(&worker, savings, "Savings", CashflowRole::LiquidCash);
    let loan = AccountId::new();
    create_role_account(&worker, loan, "Toyota Corolla", CashflowRole::LoanLiability);
    worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: loan,
                terms: loan_terms(
                    RepaymentPhilosophy::PayFixedAmount,
                    Some(45_000),
                    checking_id,
                ),
            },
        )
        .unwrap();
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            loan,
            Money::new(-1_500_000, Currency::Usd),
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        )
        .unwrap();
    // "Toyota Corolla insurance" shares the loan's whole name, but it is monthly on the 3rd from
    // another account at another amount: not the loan's payment.
    bill(
        &worker,
        "Toyota Corolla insurance",
        "subscription",
        12_000,
        Frequency::Monthly,
        NaiveDate::from_ymd_opt(2026, 7, 3).unwrap(),
        savings,
    );
    assert!(worker.loan_double_count_warnings().unwrap().is_empty());
}

// ----- the schedule corroboration, one signal at a time -------------------------------------------
//
// Every fixture above puts a bill on the due day AND on the paying account, so neither signal was
// proven on its own (04b's mutation review of the first version of this change). Each test below
// varies exactly one of them for a generic (`subscription`) bill whose amount equals the payment.

/// A second liquid account that is NOT the loan's paying source.
fn other_account(worker: &DbWorker) -> AccountId {
    let id = AccountId::new();
    create_role_account(worker, id, "Savings", CashflowRole::LiquidCash);
    id
}

/// The mortgage fixture's loan is due on the 15th and paid from checking; flag state of a monthly
/// $1,500 `subscription` bill anchored on `day` of July and paid from `from`.
fn flagged_when_on_day(worker: &DbWorker, day: u32, from: AccountId) -> bool {
    bill(
        worker,
        "Home payment",
        "subscription",
        150_000,
        Frequency::Monthly,
        NaiveDate::from_ymd_opt(2026, 7, day).unwrap(),
        from,
    );
    !worker.loan_double_count_warnings().unwrap().is_empty()
}

#[test]
fn near_the_due_day_alone_is_enough_even_from_another_account() {
    let (_dir, worker) = worker();
    mortgage(&worker);
    let savings = other_account(&worker);
    // Two days after the 15th, paid from an account that is not the loan's source: only POSSIBLE.
    assert!(flagged_when_on_day(&worker, 17, savings));
    assert_eq!(
        worker.loan_double_count_warnings().unwrap()[0].strength,
        MatchStrength::Possible
    );
}

/// Ruling: the funding account counts only together with the date. A bill from the loan's own account
/// but 13 days from the due day is some other obligation.
#[test]
fn the_same_paying_account_without_the_date_is_not_enough() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    assert!(!flagged_when_on_day(&worker, 2, fixture.checking));
}

#[test]
fn the_due_day_tolerance_is_seven_days_either_side() {
    for (day, expected) in [(8, true), (15, true), (22, true), (7, false), (23, false)] {
        let (_dir, worker) = worker();
        mortgage(&worker);
        let savings = other_account(&worker);
        assert_eq!(
            flagged_when_on_day(&worker, day, savings),
            expected,
            "a bill on the {day}th from another account (loan due the 15th)"
        );
    }
}

#[test]
fn the_due_day_comparison_wraps_across_the_month_end() {
    // A loan due on the 1st; a bill on the 30th is two days away across the month end.
    let (_dir, worker) = worker();
    let checking_id = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(checking_id)),
                opening_balance: Some(Money::new(500_000, Currency::Usd)),
            },
        )
        .unwrap();
    let savings = other_account(&worker);
    let loan = AccountId::new();
    create_role_account(&worker, loan, "Test Mortgage", CashflowRole::LoanLiability);
    let mut terms = loan_terms(
        RepaymentPhilosophy::PayFixedAmount,
        Some(150_000),
        checking_id,
    );
    terms.payment_due_day = Some(1);
    worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: loan,
                terms,
            },
        )
        .unwrap();
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            loan,
            Money::new(-20_000_000, Currency::Usd),
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        )
        .unwrap();
    assert!(
        flagged_when_on_day(&worker, 30, savings),
        "the 30th is within 3 days of the 1st"
    );
}

// ----- planning's revised ruling: strong needs real evidence; possible is a soft note ---------------------

/// Strength of the single warning for a monthly bill of `bill_type` and `amount_minor` on `day`, paid
/// from `from`; `None` when nothing is flagged.
fn strength_of(
    worker: &DbWorker,
    bill_type: &str,
    amount_minor: i64,
    day: u32,
    from: AccountId,
) -> Option<MatchStrength> {
    bill(
        worker,
        "Home payment",
        bill_type,
        amount_minor,
        Frequency::Monthly,
        NaiveDate::from_ymd_opt(2026, 7, day).unwrap(),
        from,
    );
    let warnings = worker.loan_double_count_warnings().unwrap();
    assert!(warnings.len() <= 1);
    warnings.first().map(|w| w.strength)
}

/// A loan-typed bill from the funding account on the due day: STRONG from 95% to 200% of the scheduled
/// payment ($1,500), and not beyond.
#[test]
fn a_loan_typed_bill_is_strong_from_95_to_200_percent_of_the_payment() {
    for (amount, expected) in [
        (150_000, Some(MatchStrength::Strong)),
        (142_500, Some(MatchStrength::Strong)), // exactly 95%
        (142_499, Some(MatchStrength::Possible)), // just under 95%, still within 10%
        (135_000, Some(MatchStrength::Possible)), // exactly -10%
        (134_999, None),                        // just outside
        (270_000, Some(MatchStrength::Strong)), // 1.8x: escrow, extra principal
        (300_000, Some(MatchStrength::Strong)), // exactly 200%
        (300_001, None),                        // beyond 200% and beyond the possible band
        (375_000, None),                        // 2.5x
    ] {
        let (_dir, worker) = worker();
        let fixture = mortgage(&worker);
        assert_eq!(
            strength_of(&worker, "loan_payment", amount, 15, fixture.checking),
            expected,
            "a loan_payment bill of ${:.2} against a $1,500 payment",
            amount as f64 / 100.0
        );
    }
}

/// A GENERIC (subscription) bill from the funding account is never strong: possible from 95% to 150%
/// (and within 10%), no match beyond.
#[test]
fn a_generic_bill_from_the_funding_account_is_never_strong() {
    for (amount, expected) in [
        (150_000, Some(MatchStrength::Possible)),
        (142_500, Some(MatchStrength::Possible)),
        (134_999, None),
        (225_000, Some(MatchStrength::Possible)), // exactly 150%
        (225_001, None),
        (900_000, None), // a large unrelated bill near the due day is no match at all
    ] {
        let (_dir, worker) = worker();
        let fixture = mortgage(&worker);
        assert_eq!(
            strength_of(&worker, "subscription", amount, 15, fixture.checking),
            expected,
            "a subscription of ${:.2} against a $1,500 payment",
            amount as f64 / 100.0
        );
    }
}

/// A large unrelated loan-typed bill on the due day from the funding account (2.5x or more) must never
/// be STRONG: that was the over-flagging the first draft of the rule had.
#[test]
fn a_large_unrelated_bill_near_the_due_day_is_never_strong() {
    for bill_type in ["subscription", "loan_payment", "rent_mortgage", "utility"] {
        let (_dir, worker) = worker();
        let fixture = mortgage(&worker);
        let strength = strength_of(&worker, bill_type, 900_000, 15, fixture.checking);
        assert_ne!(strength, Some(MatchStrength::Strong), "{bill_type}");
        assert_eq!(
            strength, None,
            "{bill_type}: 6x the payment is no match at all"
        );
    }
}

#[test]
fn from_another_account_the_ceiling_is_ten_percent_and_it_is_only_possible() {
    for (amount, expected) in [
        (150_000, Some(MatchStrength::Possible)),
        (165_000, Some(MatchStrength::Possible)), // exactly +10%
        (165_001, None),
    ] {
        let (_dir, worker) = worker();
        mortgage(&worker);
        let savings = other_account(&worker);
        assert_eq!(
            strength_of(&worker, "subscription", amount, 15, savings),
            expected,
            "{amount}"
        );
    }
}

/// A minimum-payment loan's payment moves with its balance. A bill set to last month's minimum must
/// keep matching as the balance, and so the minimum, drifts: the flicker exact equality caused.
#[test]
fn a_changing_minimum_payment_does_not_flicker() {
    let (_dir, worker) = worker();
    let checking_id = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(checking_id)),
                opening_balance: Some(Money::new(500_000, Currency::Usd)),
            },
        )
        .unwrap();
    let loan = AccountId::new();
    create_role_account(&worker, loan, "Student Debt", CashflowRole::LoanLiability);
    worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: loan,
                terms: loan_terms(RepaymentPhilosophy::PayMinimum, None, checking_id),
            },
        )
        .unwrap();
    // The bill is last month's minimum: $480.
    bill_on_the_15th(&worker, "NELNET", "subscription", 48_000, checking_id);

    for (owed_minor, label) in [
        (-5_000_000, "owes $50,000: the minimum is $500"),
        (
            -4_800_000,
            "owes $48,000: the minimum is $480, an exact match",
        ),
        (
            -4_000_000,
            "owes $40,000: the minimum is $400, the bill is 120% of it",
        ),
        (
            -5_200_000,
            "owes $52,000: the minimum is $520, the bill is 92% of it",
        ),
    ] {
        worker
            .record_balance_assertion(
                Uuid::now_v7(),
                loan,
                Money::new(owed_minor, Currency::Usd),
                NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            )
            .unwrap();
        let warnings = worker.loan_double_count_warnings().unwrap();
        assert_eq!(warnings.len(), 1, "{label}: still flagged, no flicker");
    }
}

/// The loan's funding account can come from payments already paired INTO the loan, not only from its
/// terms: a loan with no paying source, but paid in the past from checking, still recognizes a bill
/// paid from checking as strong, and only possible when it is paid from elsewhere.
#[test]
fn the_source_of_payments_already_paired_into_the_loan_counts_as_its_funding_account() {
    let (_dir, worker) = worker();
    let checking_id = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(checking_id)),
                opening_balance: Some(Money::new(2_000_000, Currency::Usd)),
            },
        )
        .unwrap();
    let savings = other_account(&worker);
    let loan = AccountId::new();
    create_role_account(&worker, loan, "Test Mortgage", CashflowRole::LoanLiability);
    let mut terms = loan_terms(
        RepaymentPhilosophy::PayFixedAmount,
        Some(150_000),
        checking_id,
    );
    terms.paying_source_account_id = None; // no declared funding account
    worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: loan,
                terms,
            },
        )
        .unwrap();
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            loan,
            Money::new(-20_000_000, Currency::Usd),
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        )
        .unwrap();

    // Before any paired payment: a loan-typed bill from checking is only a possible match.
    bill_on_the_15th(
        &worker,
        "Home payment",
        "loan_payment",
        150_000,
        checking_id,
    );
    assert_eq!(
        worker.loan_double_count_warnings().unwrap()[0].strength,
        MatchStrength::Possible
    );

    // A payment from checking into the loan is recorded: checking is now its funding account.
    worker
        .dispatch(
            meta(),
            WriteCommand::Transfer {
                source_account_id: checking_id,
                dest_account_id: loan,
                amount: Money::new(150_000, Currency::Usd),
                occurred_at: chrono::Utc.with_ymd_and_hms(2026, 6, 15, 12, 0, 0).unwrap(),
            },
        )
        .unwrap();
    assert_eq!(
        worker.loan_double_count_warnings().unwrap()[0].strength,
        MatchStrength::Strong
    );
    // The same payment tracked from savings (not a funding account of the loan) stays only possible.
    bill(
        &worker,
        "Other payment",
        "loan_payment",
        150_000,
        Frequency::Monthly,
        NaiveDate::from_ymd_opt(2026, 7, 16).unwrap(),
        savings,
    );
    let warnings = worker.loan_double_count_warnings().unwrap();
    let strength = |name: &str| {
        warnings
            .iter()
            .find(|w| w.bill_name == name)
            .map(|w| w.strength)
    };
    assert_eq!(strength("Home payment"), Some(MatchStrength::Strong));
    assert_eq!(strength("Other payment"), Some(MatchStrength::Possible));
}

/// The mortgage fixture plus a payment already paired into the loan from checking whose imported payee
/// is "ACME HOME LOANS"; returns a second account the loan is NOT paid from.
fn mortgage_with_a_paired_payee(worker: &DbWorker) -> AccountId {
    let fixture = mortgage(worker);
    let savings = other_account(worker);
    worker
        .dispatch(
            meta(),
            WriteCommand::Transfer {
                source_account_id: fixture.checking,
                dest_account_id: fixture.loan,
                amount: Money::new(150_000, Currency::Usd),
                occurred_at: chrono::Utc.with_ymd_and_hms(2026, 6, 15, 12, 0, 0).unwrap(),
            },
        )
        .unwrap();
    // The transfer's transaction: the one with a positive posting on the loan account.
    let transaction_id: Uuid = worker
        .read_connection()
        .unwrap()
        .query_row(
            "SELECT lp.transaction_id
               FROM ledger_postings lp
               JOIN accounts a ON a.ledger_account_id = lp.ledger_account_id
              WHERE a.id = ?1 AND lp.minor_units > 0",
            [fixture.loan.as_uuid()],
            |r| r.get(0),
        )
        .unwrap();
    let guard = worker.lock();
    record_transaction_detail(
        &guard.conn,
        transaction_id,
        None,
        Some("ACME HOME LOANS"),
        None,
    )
    .unwrap();
    savings
}

/// Strength of the one warning for a bill named like the paired payments' payee.
fn acme_bill(
    worker: &DbWorker,
    amount_minor: i64,
    frequency: Frequency,
    day: u32,
    from: AccountId,
) -> Option<MatchStrength> {
    bill(
        worker,
        "Acme Home Loans",
        "subscription",
        amount_minor,
        frequency,
        NaiveDate::from_ymd_opt(2026, 7, day).unwrap(),
        from,
    );
    worker
        .loan_double_count_warnings()
        .unwrap()
        .first()
        .map(|w| w.strength)
}

/// The payee of the loan's paired payments is real evidence, whatever the bill's type and whichever
/// account pays it: a generic bill carrying that payee, monthly, on the due day, is STRONG.
#[test]
fn a_bill_carrying_the_paired_payments_payee_is_strong_even_from_another_account() {
    let (_dir, worker) = worker();
    let savings = mortgage_with_a_paired_payee(&worker);
    assert_eq!(
        acme_bill(&worker, 150_000, Frequency::Monthly, 15, savings),
        Some(MatchStrength::Strong)
    );
}

/// An escrow mortgage: the payee route waives the amount ceiling (only the 95% floor applies).
#[test]
fn a_payee_matched_escrow_mortgage_is_strong_with_no_amount_ceiling() {
    for amount in [180_000, 300_000, 900_000] {
        let (_dir, worker) = worker();
        let savings = mortgage_with_a_paired_payee(&worker);
        assert_eq!(
            acme_bill(&worker, amount, Frequency::Monthly, 15, savings),
            Some(MatchStrength::Strong),
            "{amount}"
        );
    }
}

/// The payee route still needs the 95% floor: a payee-matched bill far BELOW the payment is not strong.
#[test]
fn a_payee_matched_bill_below_the_floor_is_not_strong() {
    let (_dir, worker) = worker();
    let savings = mortgage_with_a_paired_payee(&worker);
    assert_ne!(
        acme_bill(&worker, 50_000, Frequency::Monthly, 15, savings),
        Some(MatchStrength::Strong)
    );
}

/// One payee can carry two loans, so the payee route still needs the loan's cadence (monthly) and a
/// date within 7 days of the due day (planning's clarification); only the amount ceiling is waived.
#[test]
fn the_payee_route_still_needs_the_loans_cadence_and_date() {
    // (frequency, day of July, expected)
    for (frequency, day, expected) in [
        (Frequency::Weekly, 15, None),
        (Frequency::Monthly, 22, Some(MatchStrength::Strong)), // 7 days: the edge
        (Frequency::Monthly, 23, None),                        // 8 days
        (Frequency::Monthly, 8, Some(MatchStrength::Strong)),
        (Frequency::Monthly, 7, None),
    ] {
        let (_dir, worker) = worker();
        let savings = mortgage_with_a_paired_payee(&worker);
        assert_eq!(
            acme_bill(&worker, 900_000, frequency, day, savings),
            expected,
            "{frequency:?} on the {day}th"
        );
    }
}

// ----- what must NOT count as a funding account (04b's F1 on the paired-payments query) -------------------

/// A loan with no declared funding account and a $200,000 balance, plus the accounts a test pays it from.
fn loan_with_no_declared_source(worker: &DbWorker) -> AccountId {
    let loan = AccountId::new();
    create_role_account(worker, loan, "Test Mortgage", CashflowRole::LoanLiability);
    let mut terms = loan_terms(
        RepaymentPhilosophy::PayFixedAmount,
        Some(150_000),
        AccountId::new(),
    );
    terms.paying_source_account_id = None;
    worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: loan,
                terms,
            },
        )
        .unwrap();
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            loan,
            Money::new(-20_000_000, Currency::Usd),
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        )
        .unwrap();
    loan
}

fn funded_account(
    worker: &DbWorker,
    name: &str,
    role: CashflowRole,
    opening_minor: i64,
) -> AccountId {
    let id = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(Account::new(
                    id,
                    LedgerAccountId::new(),
                    name,
                    role,
                    Currency::Usd,
                    AccountFlags::default(),
                )),
                opening_balance: Some(Money::new(opening_minor, Currency::Usd)),
            },
        )
        .unwrap();
    id
}

fn pay_the_loan_from(
    worker: &DbWorker,
    source: AccountId,
    loan: AccountId,
) -> Result<(), crate::DbError> {
    worker
        .dispatch(
            meta(),
            WriteCommand::Transfer {
                source_account_id: source,
                dest_account_id: loan,
                amount: Money::new(150_000, Currency::Usd),
                occurred_at: chrono::Utc.with_ymd_and_hms(2026, 6, 15, 12, 0, 0).unwrap(),
            },
        )
        .map(|_| ())
}

fn strength_for_a_bill_paid_from(worker: &DbWorker, from: AccountId) -> Option<MatchStrength> {
    bill_on_the_15th(worker, "Home payment", "loan_payment", 150_000, from);
    worker
        .loan_double_count_warnings()
        .unwrap()
        .first()
        .map(|w| w.strength)
}

/// CONTROL: a live payment from savings makes savings a funding account (the query does count it).
#[test]
fn a_live_payment_from_another_liquid_account_makes_it_a_funding_account() {
    let (_dir, worker) = worker();
    let savings = funded_account(&worker, "Savings", CashflowRole::LiquidCash, 2_000_000);
    let loan = loan_with_no_declared_source(&worker);
    pay_the_loan_from(&worker, savings, loan).unwrap();
    assert_eq!(
        strength_for_a_bill_paid_from(&worker, savings),
        Some(MatchStrength::Strong)
    );
}

/// A VOIDED payment never happened: its source must not count as the loan's funding account.
#[test]
fn a_voided_payment_does_not_make_its_source_a_funding_account() {
    let (_dir, worker) = worker();
    let savings = funded_account(&worker, "Savings", CashflowRole::LiquidCash, 2_000_000);
    let loan = loan_with_no_declared_source(&worker);
    pay_the_loan_from(&worker, savings, loan).unwrap();
    let transaction_id: Uuid = worker
        .read_connection()
        .unwrap()
        .query_row(
            "SELECT lp.transaction_id
               FROM ledger_postings lp
               JOIN accounts a ON a.ledger_account_id = lp.ledger_account_id
              WHERE a.id = ?1 AND lp.minor_units > 0",
            [loan.as_uuid()],
            |r| r.get(0),
        )
        .unwrap();
    worker
        .dispatch(
            meta(),
            WriteCommand::VoidTransaction {
                transaction_id: TransactionId::from_uuid(transaction_id),
            },
        )
        .unwrap();
    assert_eq!(
        strength_for_a_bill_paid_from(&worker, savings),
        Some(MatchStrength::Possible),
        "the equal amount on the due day is still a possible match, but savings is not a funding account"
    );
}

/// Only a LIQUID account can be the funding account. The ledger refuses a transfer into a loan from an
/// investment, card or loan account, so a payment with such a source leg is planted directly (a balanced
/// pair of postings): the query's liquid-only guard is what keeps it from counting.
#[test]
fn a_payment_from_a_non_liquid_account_does_not_make_it_a_funding_account() {
    let (_dir, worker) = worker();
    let brokerage = funded_account(
        &worker,
        "Brokerage",
        CashflowRole::InvestmentAsset,
        2_000_000,
    );
    let loan = loan_with_no_declared_source(&worker);
    let ledger_of = |account: AccountId| -> Uuid {
        worker
            .read_connection()
            .unwrap()
            .query_row(
                "SELECT ledger_account_id FROM accounts WHERE id = ?1",
                [account.as_uuid()],
                |r| r.get(0),
            )
            .unwrap()
    };
    let (loan_ledger, brokerage_ledger) = (ledger_of(loan), ledger_of(brokerage));
    {
        let guard = worker.lock();
        let transaction_id = Uuid::now_v7();
        for (ledger_account, minor) in [(loan_ledger, 150_000_i64), (brokerage_ledger, -150_000)] {
            guard
                .conn
                .execute(
                    "INSERT INTO ledger_postings
                        (transaction_id, ledger_account_id, minor_units, currency, currency_exponent, posting_date)
                     VALUES (?1, ?2, ?3, 'USD', 2, '2026-06-15')",
                    rusqlite::params![transaction_id, ledger_account, minor],
                )
                .unwrap();
        }
        guard
            .conn
            .execute(
                "INSERT INTO ledger_transactions (id, operation_id, occurred_at, currency)
                 VALUES (?1, ?2, '2026-06-15T12:00:00+00:00', 'USD')",
                rusqlite::params![transaction_id, Uuid::now_v7()],
            )
            .unwrap();
    }
    assert_eq!(
        strength_for_a_bill_paid_from(&worker, brokerage),
        Some(MatchStrength::Possible),
        "the equal amount on the due day is possible, but the brokerage is not a funding account"
    );
}

// ----- which evidence made a strong match, for the explanation ------------------------------------------

#[test]
fn a_loan_typed_strong_match_says_so_and_names_the_account_it_is_paid_from() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    bill_on_the_15th(
        &worker,
        "ACME HOME LOANS",
        "loan_payment",
        150_000,
        fixture.checking,
    );
    let warnings = worker.loan_double_count_warnings().unwrap();
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].strength, MatchStrength::Strong);
    assert_eq!(warnings[0].matched_by, Some(MatchedBy::LoanTyped));
    assert_eq!(warnings[0].bill_type.as_deref(), Some("loan_payment"));
    assert_eq!(
        warnings[0].paid_from_account_name.as_deref(),
        Some("Checking")
    );
}

#[test]
fn a_payee_strong_match_says_so() {
    let (_dir, worker) = worker();
    let savings = mortgage_with_a_paired_payee(&worker);
    acme_bill(&worker, 150_000, Frequency::Monthly, 15, savings);
    let warnings = worker.loan_double_count_warnings().unwrap();
    assert_eq!(warnings[0].matched_by, Some(MatchedBy::Payee));
    assert_eq!(
        warnings[0].paid_from_account_name.as_deref(),
        Some("Savings")
    );
}

/// When both routes hold (a loan-typed bill from the funding account that also carries the payee), the
/// payee is reported: it is the stronger evidence.
#[test]
fn the_payee_is_reported_when_both_routes_hold() {
    let (_dir, worker) = worker();
    let _savings = mortgage_with_a_paired_payee(&worker);
    let checking_id: Uuid = worker
        .read_connection()
        .unwrap()
        .query_row("SELECT id FROM accounts WHERE name = 'Checking'", [], |r| {
            r.get(0)
        })
        .unwrap();
    bill_on_the_15th(
        &worker,
        "Acme Home Loans",
        "loan_payment",
        150_000,
        AccountId::from_uuid(checking_id),
    );
    let warnings = worker.loan_double_count_warnings().unwrap();
    assert_eq!(warnings[0].matched_by, Some(MatchedBy::Payee));
}

/// For an EXISTING bill the declared-terms route is still only POSSIBLE (the soft note): the user's
/// terms are enough to ask while a bill is being added, never to say it is counted twice.
#[test]
fn a_declared_terms_match_is_only_possible_for_an_existing_bill() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    bill_on_the_15th(
        &worker,
        "Home payment",
        "subscription",
        150_000,
        fixture.checking,
    );
    let warnings = worker.loan_double_count_warnings().unwrap();
    assert_eq!(warnings[0].strength, MatchStrength::Possible);
    assert_eq!(warnings[0].matched_by, Some(MatchedBy::DeclaredTerms));
}

/// A possible match that is NOT from the declared paying account (here: another account, within 10%)
/// carries no route at all, so nothing asks about it.
#[test]
fn a_possible_match_from_another_account_carries_no_evidence_route() {
    let (_dir, worker) = worker();
    mortgage(&worker);
    let savings = other_account(&worker);
    bill_on_the_15th(&worker, "Home payment", "subscription", 150_000, savings);
    let warnings = worker.loan_double_count_warnings().unwrap();
    assert_eq!(warnings[0].strength, MatchStrength::Possible);
    assert_eq!(warnings[0].matched_by, None);
}

/// The ask needs the DECLARED paying account. A source that only appears because a payment was once
/// recorded from it (a paired source) makes a generic bill possible, but does not make it ask.
#[test]
fn a_paired_source_that_is_not_the_declared_account_does_not_make_the_ask() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker); // declared paying account: checking
    let savings = funded_account(&worker, "Savings", CashflowRole::LiquidCash, 2_000_000);
    pay_the_loan_from(&worker, savings, fixture.loan).unwrap();
    bill_on_the_15th(&worker, "Home payment", "subscription", 150_000, savings);
    let warnings = worker.loan_double_count_warnings().unwrap();
    assert_eq!(warnings[0].strength, MatchStrength::Possible);
    assert_eq!(
        warnings[0].matched_by, None,
        "savings is a paired source, not the declared account"
    );
}
