//! A loan's payment counted twice (personal-cfo-6wk.20): the loan account's projected payment AND a
//! recurring bill for the same payment. Reproduction first, on fake data only.

use super::*;
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
fn a_subscription_typed_bill_for_the_loans_payment_is_flagged() {
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
    // KNOWN GAP until the forecast-level link (the second phase of 6wk.20): the warning is
    // descriptive (ADR 0018), so the forecast still counts both until the user removes one.
    assert_eq!(outflow_on_the_15th(&worker), 300_000);
}

#[test]
fn a_rent_mortgage_typed_bill_is_flagged() {
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
}

/// A `loan_payment` bill is suspected on the name or the amount alone, as before 6wk.20 (its own type
/// says it is loan-shaped), even when its schedule differs.
#[test]
fn a_loan_payment_typed_bill_is_flagged_whatever_its_schedule() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    bill(
        &worker,
        "ACME HOME LOANS",
        "loan_payment",
        150_000,
        Frequency::Weekly,
        NaiveDate::from_ymd_opt(2026, 7, 3).unwrap(),
        fixture.checking,
    );
    let warnings = worker.loan_double_count_warnings().unwrap();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].amount_match);
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
    // Two days after the 15th, paid from an account that is not the loan's source.
    assert!(flagged_when_on_day(&worker, 17, savings));
}

#[test]
fn the_same_paying_account_alone_is_enough_on_a_distant_day() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    // The 2nd is 13 days from the 15th, but the bill is paid from the loan's own source.
    assert!(flagged_when_on_day(&worker, 2, fixture.checking));
}

#[test]
fn the_due_day_tolerance_is_three_days_either_side() {
    for (day, expected) in [(12, true), (15, true), (18, true), (11, false), (19, false)] {
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
