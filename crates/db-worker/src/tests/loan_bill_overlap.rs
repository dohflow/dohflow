//! A loan's payment counted twice (personal-cfo-6wk.20): the loan account's projected payment AND a
//! recurring bill for the same payment. Reproduction first, on fake data only.

use super::*;
use crate::MatchStrength;
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

/// CHANGED by planning's ruling (6wk.20): an amount alone never flags a bill any more (no exact amount
/// equality anywhere), whatever its type: the schedule has to line up. A `loan_payment` bill on
/// another cadence with an unrelated name is therefore NOT flagged; one whose name looks like the loan's
/// still is (the 6wk.11 name signal, a warning only).
#[test]
fn a_loan_payment_typed_bill_on_another_cadence_is_flagged_only_by_its_name() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    let weekly = |worker: &DbWorker, name: &str| {
        bill(
            worker,
            name,
            "loan_payment",
            150_000,
            Frequency::Weekly,
            NaiveDate::from_ymd_opt(2026, 7, 3).unwrap(),
            fixture.checking,
        )
    };
    weekly(&worker, "ACME HOME LOANS");
    assert!(
        worker.loan_double_count_warnings().unwrap().is_empty(),
        "the same amount alone is not enough"
    );
    weekly(&worker, "Test Mortgage payment");
    let warnings = worker.loan_double_count_warnings().unwrap();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].name_match);
    assert_eq!(warnings[0].strength, MatchStrength::Possible);
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

// ----- planning's ruling: strong vs possible, with a 95% floor, no ceiling, and no flicker --------------

/// Strength of the single warning for a monthly `subscription` bill of `amount_minor` on `day`,
/// paid from `from`; `None` when nothing is flagged.
fn strength_of(
    worker: &DbWorker,
    amount_minor: i64,
    day: u32,
    from: AccountId,
) -> Option<MatchStrength> {
    bill(
        worker,
        "Home payment",
        "subscription",
        amount_minor,
        Frequency::Monthly,
        NaiveDate::from_ymd_opt(2026, 7, day).unwrap(),
        from,
    );
    let warnings = worker.loan_double_count_warnings().unwrap();
    assert!(warnings.len() <= 1);
    warnings.first().map(|w| w.strength)
}

#[test]
fn the_amount_is_a_95_percent_floor_with_no_ceiling_from_the_funding_account() {
    // Payment $1,500. From the loan's own account on its due day:
    for (amount, expected) in [
        (150_000, Some(MatchStrength::Strong)),
        (142_500, Some(MatchStrength::Strong)), // exactly 95%
        (142_499, Some(MatchStrength::Possible)), // just under: still within 10%
        (135_000, Some(MatchStrength::Possible)), // exactly -10%
        (134_999, None),                        // just outside
        (180_000, Some(MatchStrength::Strong)), // escrow added: no upper bound
        (900_000, Some(MatchStrength::Strong)), // no upper bound at all
    ] {
        let (_dir, worker) = worker();
        let fixture = mortgage(&worker);
        assert_eq!(
            strength_of(&worker, amount, 15, fixture.checking),
            expected,
            "a ${:.2} bill against a $1,500 payment",
            amount as f64 / 100.0
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
            strength_of(&worker, amount, 15, savings),
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

    // Before any paired payment: a bill from checking is only a possible match.
    bill_on_the_15th(
        &worker,
        "Home payment",
        "subscription",
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
        "subscription",
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

/// The payee of the paired payments is an equally strong signal: a bill whose name is that payee
/// (here promoted from the same bank payee) is strong even when it is paid from another account.
#[test]
fn a_bill_carrying_the_paired_payments_payee_is_strong_even_from_another_account() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    let savings = other_account(&worker);
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
    // The imported payee on that payment.
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
    {
        let guard = worker.lock();
        record_transaction_detail(
            &guard.conn,
            transaction_id,
            None,
            Some("ACME HOME LOANS"),
            None,
        )
        .unwrap();
    }
    // Paid from savings (not the loan's source) but named like the payee: strong.
    bill(
        &worker,
        "Acme Home Loans",
        "subscription",
        150_000,
        Frequency::Monthly,
        the_15th(),
        savings,
    );
    let warnings = worker.loan_double_count_warnings().unwrap();
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].strength, MatchStrength::Strong);
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
    bill_on_the_15th(worker, "Home payment", "subscription", 150_000, from);
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
