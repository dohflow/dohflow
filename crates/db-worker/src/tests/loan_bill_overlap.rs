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

/// A monthly bill on the 15th paid from `checking`, as Add bill / Make recurring / promoting a suggested
/// recurring candidate create it (the default type is `subscription`).
fn bill_on_the_15th(
    worker: &DbWorker,
    name: &str,
    bill_type: &str,
    amount_minor: i64,
    checking: AccountId,
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
                frequency: Frequency::Monthly,
                anchor: NaiveDate::from_ymd_opt(2026, 7, 15).unwrap(),
                autopay_account_id: Some(checking),
                description: None,
                source_merchant_key: None,
                category_id: None,
                tag_ids: Vec::new(),
            },
        )
        .unwrap();
    event_id
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

/// REPRODUCTION of the owner's report (06-dogfood #7): the bill suggested for the mortgage's payment is
/// added with its default type, `subscription`. Cash Flow now shows $3,000 leaving on the 15th, and the
/// existing detector (6wk.11, which only looks at `loan_payment` bills) finds nothing.
#[test]
fn reproduction_a_subscription_typed_bill_for_the_loans_payment_double_counts_today() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    bill_on_the_15th(&worker, "ACME HOME LOANS", "subscription", 150_000, fixture.checking);

    assert_eq!(
        outflow_on_the_15th(&worker),
        300_000,
        "the loan's $1,500 and the bill's $1,500 both leave on the 15th: the double count"
    );
    assert!(
        worker.loan_double_count_warnings().unwrap().is_empty(),
        "the 6wk.11 detector only considers bill_type == loan_payment, so it is silent here"
    );
}

#[test]
fn reproduction_a_rent_mortgage_typed_bill_is_invisible_to_the_detector_too() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    bill_on_the_15th(&worker, "Test Mortgage", "rent_mortgage", 150_000, fixture.checking);
    assert_eq!(outflow_on_the_15th(&worker), 300_000);
    assert!(worker.loan_double_count_warnings().unwrap().is_empty());
}

/// The variant in the report: with the bill typed Loan payment the detector DOES warn, but the forecast
/// still counts both (the warning is descriptive only, ADR 0018).
#[test]
fn reproduction_a_loan_payment_typed_bill_is_flagged_but_still_double_counted() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    bill_on_the_15th(&worker, "ACME HOME LOANS", "loan_payment", 150_000, fixture.checking);
    let warnings = worker.loan_double_count_warnings().unwrap();
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].loan_account_id, fixture.loan.as_uuid());
    assert!(warnings[0].amount_match);
    assert_eq!(outflow_on_the_15th(&worker), 300_000, "flagged, yet counted twice");
}
