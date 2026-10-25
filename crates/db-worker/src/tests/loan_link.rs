//! A recurring bill linked to the loan it pays (personal-cfo-6wk.22; ADR 0035 addendum 2026-10-25):
//! the forecast counts the obligation once, and every way the link can stop holding brings the
//! loan's own payment back. Fake data only.

use super::loan_bill_overlap::{
    bill, bill_on_the_15th, mortgage, outflow_on_the_15th, the_15th, Fixture,
};
use super::*;
use chrono::TimeZone;

fn link(worker: &DbWorker, event: RecurringEventId, loan: AccountId) -> Result<(), crate::DbError> {
    worker
        .dispatch(
            meta(),
            WriteCommand::LinkBillToLoan {
                event_id: event,
                loan_account_id: loan,
            },
        )
        .map(|_| ())
}

fn unlink(worker: &DbWorker, event: RecurringEventId) {
    worker
        .dispatch(meta(), WriteCommand::UnlinkBillFromLoan { event_id: event })
        .unwrap();
}

/// The monthly $1,500 bill on the 15th from checking, as the form creates it (`subscription`).
fn the_bill(worker: &DbWorker, fixture: &Fixture, amount_minor: i64) -> RecurringEventId {
    bill_on_the_15th(
        worker,
        "ACME HOME LOANS",
        "subscription",
        amount_minor,
        fixture.checking,
    )
}

/// The names of the events on 2026-07-15 in the aggregate forecast.
fn events_on_the_15th(worker: &DbWorker) -> Vec<(String, String, i64)> {
    let as_of = chrono::Utc.with_ymd_and_hms(2026, 7, 1, 12, 0, 0).unwrap();
    let conn = worker.read_connection().unwrap();
    let view = crate::forecast::compute(&conn, as_of, 45, &[]).unwrap();
    view.days
        .iter()
        .filter(|d| d.date == the_15th())
        .flat_map(|d| d.events.iter())
        .map(|e| (e.kind.clone(), e.name.clone(), e.amount.minor_units()))
        .collect()
}

/// How the bill's link stands, read through the same view the UI uses.
fn status_of(worker: &DbWorker, event: RecurringEventId) -> Option<crate::BillLoanLinkView> {
    worker
        .recurring_bill_views()
        .unwrap()
        .into_iter()
        .find(|b| b.id == event)
        .and_then(|b| b.loan_link)
}

fn stored_link(worker: &DbWorker, event: RecurringEventId) -> Option<Uuid> {
    worker
        .read_connection()
        .unwrap()
        .query_row(
            "SELECT linked_loan_account_id FROM recurring_events WHERE id = ?1",
            [event.as_uuid()],
            |r| r.get(0),
        )
        .unwrap()
}

// ----- the forecast counts the obligation once --------------------------------------------------------

#[test]
fn before_linking_both_rows_count_and_after_linking_there_is_one() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    let bill_id = the_bill(&worker, &fixture, 150_000);
    assert_eq!(
        outflow_on_the_15th(&worker),
        300_000,
        "the reported double count"
    );

    link(&worker, bill_id, fixture.loan).unwrap();
    assert_eq!(
        outflow_on_the_15th(&worker),
        150_000,
        "one outflow once linked"
    );
    let events = events_on_the_15th(&worker);
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].1, "ACME HOME LOANS", "the bill carries the cash");
}

#[test]
fn an_escrow_bill_shows_the_observed_debit_not_the_loans_principal_and_interest() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    let bill_id = the_bill(&worker, &fixture, 270_000); // $1,500 P&I plus $1,200 escrow
    link(&worker, bill_id, fixture.loan).unwrap();

    assert_eq!(
        outflow_on_the_15th(&worker),
        270_000,
        "the observed debit, escrow included"
    );
    let status = status_of(&worker, bill_id).unwrap();
    assert!(status.in_force);
    assert_eq!(
        status.scheduled_payment,
        Some(Money::new(150_000, Currency::Usd))
    );
    assert_eq!(
        status.other,
        Some(Money::new(120_000, Currency::Usd)),
        "the split: what is beyond principal and interest"
    );
    assert!(!status.below_scheduled);
}

#[test]
fn unlinking_restores_both_rows() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    let bill_id = the_bill(&worker, &fixture, 150_000);
    link(&worker, bill_id, fixture.loan).unwrap();
    unlink(&worker, bill_id);
    assert_eq!(stored_link(&worker, bill_id), None);
    assert_eq!(outflow_on_the_15th(&worker), 300_000);
}

#[test]
fn the_per_account_series_still_reconcile_with_the_aggregate_when_linked() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    let bill_id = the_bill(&worker, &fixture, 150_000);
    link(&worker, bill_id, fixture.loan).unwrap();
    let as_of = chrono::Utc.with_ymd_and_hms(2026, 7, 1, 12, 0, 0).unwrap();
    let conn = worker.read_connection().unwrap();
    let aggregate = crate::forecast::compute(&conn, as_of, 45, &[]).unwrap();
    let multi = crate::forecast::compute_by_account(&conn, as_of, 45, &[]).unwrap();
    let checking = multi
        .accounts
        .iter()
        .find(|s| s.account_id == Some(fixture.checking.as_uuid()))
        .expect("checking series");
    assert_eq!(
        checking.days.last().unwrap().closing.p50.minor_units(),
        aggregate.days.last().unwrap().closing.p50.minor_units(),
        "checking is the only liquid account, so its series equals the aggregate"
    );
}

// ----- every way the link can stop holding brings the loan's payment back ------------------------------

#[test]
fn archiving_the_linked_bill_brings_the_loans_own_payment_back() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    let bill_id = the_bill(&worker, &fixture, 150_000);
    link(&worker, bill_id, fixture.loan).unwrap();
    worker
        .dispatch(meta(), WriteCommand::ArchiveRecurringBill(bill_id))
        .unwrap();
    // The counterexample that rules out several links: the loan's $1,500 must not vanish.
    assert_eq!(
        outflow_on_the_15th(&worker),
        150_000,
        "only the loan's own row remains"
    );
    assert_eq!(events_on_the_15th(&worker)[0].0, "loan_payment");
    assert_eq!(
        stored_link(&worker, bill_id),
        None,
        "an archived bill carries no link"
    );

    // Restoring the bill brings it back UNLINKED: both count until the user links it again.
    worker
        .dispatch(meta(), WriteCommand::RestoreRecurringBill(bill_id))
        .unwrap();
    assert_eq!(outflow_on_the_15th(&worker), 300_000);
}

#[test]
fn deleting_the_linked_bill_brings_the_loans_own_payment_back() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    let bill_id = the_bill(&worker, &fixture, 150_000);
    link(&worker, bill_id, fixture.loan).unwrap();
    worker
        .dispatch(
            meta(),
            WriteCommand::DeleteRecurringBill { event_id: bill_id },
        )
        .unwrap();
    assert_eq!(outflow_on_the_15th(&worker), 150_000);
    worker
        .dispatch(
            meta(),
            WriteCommand::ReinstateRecurringBill { event_id: bill_id },
        )
        .unwrap();
    assert_eq!(
        stored_link(&worker, bill_id),
        None,
        "a reinstated bill comes back unlinked"
    );
    assert_eq!(outflow_on_the_15th(&worker), 300_000);
}

#[test]
fn archiving_the_loan_clears_the_link_and_a_reinstated_loan_comes_back_unlinked() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    let bill_id = the_bill(&worker, &fixture, 150_000);
    link(&worker, bill_id, fixture.loan).unwrap();

    worker
        .dispatch(meta(), WriteCommand::ArchiveAccount(fixture.loan))
        .unwrap();
    assert_eq!(
        stored_link(&worker, bill_id),
        None,
        "cleared in the same command"
    );
    // The archived loan emits nothing; the bill carries the cash.
    assert_eq!(outflow_on_the_15th(&worker), 150_000);

    worker
        .dispatch(meta(), WriteCommand::ReinstateAccount(fixture.loan))
        .unwrap();
    assert_eq!(
        stored_link(&worker, bill_id),
        None,
        "the reinstated loan is unlinked"
    );
    // No silent suppression: the bill and the loan's own payment both count again.
    assert_eq!(outflow_on_the_15th(&worker), 300_000);
}

#[test]
fn archiving_a_loan_leaves_other_bills_links_alone() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    let bill_id = the_bill(&worker, &fixture, 150_000);
    link(&worker, bill_id, fixture.loan).unwrap();
    let other_loan = {
        let id = AccountId::new();
        create_role_account(&worker, id, "Other Loan", CashflowRole::LoanLiability);
        id
    };
    worker
        .dispatch(meta(), WriteCommand::ArchiveAccount(other_loan))
        .unwrap();
    assert_eq!(stored_link(&worker, bill_id), Some(fixture.loan.as_uuid()));
}

#[test]
fn a_cadence_change_brings_the_loans_row_back_with_the_status_and_restoring_it_restores_the_link() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    let bill_id = the_bill(&worker, &fixture, 150_000);
    link(&worker, bill_id, fixture.loan).unwrap();

    let edit = |frequency: Frequency| {
        worker
            .dispatch(
                meta(),
                WriteCommand::UpdateRecurringBill {
                    event_id: bill_id,
                    name: "ACME HOME LOANS".to_owned(),
                    amount: Money::new(150_000, Currency::Usd),
                    bill_type: "subscription".to_owned(),
                    frequency,
                    anchor: the_15th(),
                    autopay_account_id: Some(fixture.checking),
                    description: None,
                },
            )
            .unwrap();
    };
    edit(Frequency::Weekly);
    // The link is still stored but no longer in force: BOTH now count, and the status says why.
    assert_eq!(stored_link(&worker, bill_id), Some(fixture.loan.as_uuid()));
    let status = status_of(&worker, bill_id).unwrap();
    assert!(status.cadence_changed && !status.in_force);
    assert_eq!(outflow_on_the_15th(&worker), 300_000);

    edit(Frequency::Monthly);
    assert!(status_of(&worker, bill_id).unwrap().in_force);
    assert_eq!(
        outflow_on_the_15th(&worker),
        150_000,
        "restoring the cadence restores the link"
    );
}

#[test]
fn an_amount_cut_below_95_percent_keeps_the_link_and_flags_it() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    let bill_id = the_bill(&worker, &fixture, 150_000);
    link(&worker, bill_id, fixture.loan).unwrap();
    for (amount, below) in [(142_500, false), (142_499, true), (100_000, true)] {
        worker
            .dispatch(
                meta(),
                WriteCommand::UpdateRecurringBill {
                    event_id: bill_id,
                    name: "ACME HOME LOANS".to_owned(),
                    amount: Money::new(amount, Currency::Usd),
                    bill_type: "subscription".to_owned(),
                    frequency: Frequency::Monthly,
                    anchor: the_15th(),
                    autopay_account_id: Some(fixture.checking),
                    description: None,
                },
            )
            .unwrap();
        let status = status_of(&worker, bill_id).unwrap();
        assert!(
            status.in_force,
            "{amount}: the amount stays the user's truth, the link stays"
        );
        assert_eq!(status.below_scheduled, below, "{amount}");
        assert_eq!(
            outflow_on_the_15th(&worker),
            amount,
            "{amount}: one row, the bill's amount"
        );
    }
}

#[test]
fn a_loan_that_becomes_pay_in_full_stops_being_carried_by_the_bill() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    let bill_id = the_bill(&worker, &fixture, 150_000);
    link(&worker, bill_id, fixture.loan).unwrap();
    // The loan is switched to a single payoff: it no longer projects a recurring payment.
    let mut terms = loan_terms(RepaymentPhilosophy::PayInFull, None, fixture.checking);
    terms.payment_due_day = Some(15);
    worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: fixture.loan,
                terms,
            },
        )
        .unwrap();
    let status = status_of(&worker, bill_id).unwrap();
    assert!(!status.in_force, "a single payoff is not a monthly payment");
    // The loan's payoff row returns beside the bill: nothing silently vanishes.
    assert!(events_on_the_15th(&worker)
        .iter()
        .any(|e| e.0 == "loan_payment"));
}

// ----- what linking refuses -----------------------------------------------------------------------------

#[test]
fn a_second_bill_for_the_same_loan_is_refused_naming_the_first() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    let first = the_bill(&worker, &fixture, 150_000);
    link(&worker, first, fixture.loan).unwrap();
    let second = bill_on_the_15th(
        &worker,
        "Extra principal",
        "subscription",
        20_000,
        fixture.checking,
    );
    // A TYPED error that names the existing bill (its Display text stays fixed: bill names are user
    // data and stay out of logs).
    match link(&worker, second, fixture.loan).unwrap_err() {
        crate::DbError::LoanAlreadyLinked { bill_id, bill_name } => {
            assert_eq!(bill_id, first.as_uuid());
            assert_eq!(bill_name, "ACME HOME LOANS");
        }
        other => panic!("expected LoanAlreadyLinked, got {other:?}"),
    }
    assert_eq!(
        link(&worker, second, fixture.loan).unwrap_err().to_string(),
        "this loan already has a linked bill",
        "the Display text carries no bill name"
    );
    assert_eq!(stored_link(&worker, second), None, "nothing was written");
    assert_eq!(stored_link(&worker, first), Some(fixture.loan.as_uuid()));
}

#[test]
fn re_linking_the_same_bill_to_the_same_loan_is_a_no_op_and_moving_it_replaces_the_link() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    let bill_id = the_bill(&worker, &fixture, 150_000);
    link(&worker, bill_id, fixture.loan).unwrap();
    link(&worker, bill_id, fixture.loan).unwrap();
    assert_eq!(stored_link(&worker, bill_id), Some(fixture.loan.as_uuid()));
}

#[test]
fn a_link_needs_a_loan_a_recurring_payment_the_same_currency_and_an_active_bill() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    let bill_id = the_bill(&worker, &fixture, 150_000);

    // Not a loan account.
    assert!(link(&worker, bill_id, fixture.checking).is_err());
    // Missing account.
    assert!(link(&worker, bill_id, AccountId::new()).is_err());
    // A loan with no payment due day set has no payment to link.
    let bare = AccountId::new();
    create_role_account(&worker, bare, "Bare Loan", CashflowRole::LoanLiability);
    assert!(link(&worker, bill_id, bare).is_err());
    // A loan paid in full once is not a monthly payment.
    let payoff = AccountId::new();
    create_role_account(&worker, payoff, "Payoff Loan", CashflowRole::LoanLiability);
    worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: payoff,
                terms: loan_terms(RepaymentPhilosophy::PayInFull, None, fixture.checking),
            },
        )
        .unwrap();
    assert!(link(&worker, bill_id, payoff).is_err());
    // An archived loan.
    let archived = AccountId::new();
    create_role_account(&worker, archived, "Old Loan", CashflowRole::LoanLiability);
    worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: archived,
                terms: loan_terms(
                    RepaymentPhilosophy::PayFixedAmount,
                    Some(1_000),
                    fixture.checking,
                ),
            },
        )
        .unwrap();
    worker
        .dispatch(meta(), WriteCommand::ArchiveAccount(archived))
        .unwrap();
    assert!(link(&worker, bill_id, archived).is_err());
    // An archived bill.
    let archived_bill = the_bill(&worker, &fixture, 150_000);
    worker
        .dispatch(meta(), WriteCommand::ArchiveRecurringBill(archived_bill))
        .unwrap();
    assert!(link(&worker, archived_bill, fixture.loan).is_err());
    // Nothing was written by any refusal.
    assert_eq!(stored_link(&worker, bill_id), None);
    assert_eq!(stored_link(&worker, archived_bill), None);
}

#[test]
fn a_foreign_currency_loan_cannot_be_linked_to_a_dollar_bill() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    let bill_id = the_bill(&worker, &fixture, 150_000);
    let euro = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(Account::new(
                    euro,
                    LedgerAccountId::new(),
                    "Euro Loan",
                    CashflowRole::LoanLiability,
                    Currency::Eur,
                    AccountFlags::default(),
                )),
                opening_balance: None,
            },
        )
        .unwrap();
    worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: euro,
                terms: loan_terms(
                    RepaymentPhilosophy::PayFixedAmount,
                    Some(150_000),
                    fixture.checking,
                ),
            },
        )
        .unwrap();
    assert!(link(&worker, bill_id, euro).is_err());
}

#[test]
fn unlinking_and_linking_a_missing_or_unlinked_bill() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    let bill_id = the_bill(&worker, &fixture, 150_000);
    // Unlinking a bill that is not linked is an accepted no-op.
    unlink(&worker, bill_id);
    // A bill that does not exist is refused.
    assert!(worker
        .dispatch(
            meta(),
            WriteCommand::UnlinkBillFromLoan {
                event_id: RecurringEventId::new()
            },
        )
        .is_err());
    assert!(link(&worker, RecurringEventId::new(), fixture.loan).is_err());
}

// ----- "No, it's a different bill" is remembered ---------------------------------------------------------

fn stored_not_payment(worker: &DbWorker, event: RecurringEventId) -> Option<Uuid> {
    worker
        .read_connection()
        .unwrap()
        .query_row(
            "SELECT not_payment_of_loan_account_id FROM recurring_events WHERE id = ?1",
            [event.as_uuid()],
            |r| r.get(0),
        )
        .unwrap()
}

fn mark_not(
    worker: &DbWorker,
    event: RecurringEventId,
    loan: Option<AccountId>,
) -> Result<(), crate::DbError> {
    worker
        .dispatch(
            meta(),
            WriteCommand::SetBillNotLoanPayment {
                event_id: event,
                loan_account_id: loan,
            },
        )
        .map(|_| ())
}

#[test]
fn the_detector_skips_a_bill_the_user_said_is_not_the_loans_payment_and_clearing_it_restores_the_note(
) {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    let bill_id = the_bill(&worker, &fixture, 150_000);
    assert_eq!(
        worker.loan_double_count_warnings().unwrap().len(),
        1,
        "the soft note"
    );

    mark_not(&worker, bill_id, Some(fixture.loan)).unwrap();
    assert_eq!(
        stored_not_payment(&worker, bill_id),
        Some(fixture.loan.as_uuid())
    );
    assert!(
        worker.loan_double_count_warnings().unwrap().is_empty(),
        "the note stops after the user said it is a different bill"
    );

    mark_not(&worker, bill_id, None).unwrap();
    assert_eq!(stored_not_payment(&worker, bill_id), None);
    assert_eq!(
        worker.loan_double_count_warnings().unwrap().len(),
        1,
        "cleared: the note returns"
    );
}

#[test]
fn the_answer_is_scoped_to_that_loan() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    // A second loan with the same payment, due the same day, from the same account.
    let other = AccountId::new();
    create_role_account(&worker, other, "Zeta Loan", CashflowRole::LoanLiability);
    worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: other,
                terms: loan_terms(
                    RepaymentPhilosophy::PayFixedAmount,
                    Some(150_000),
                    fixture.checking,
                ),
            },
        )
        .unwrap();
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            other,
            Money::new(-20_000_000, Currency::Usd),
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        )
        .unwrap();
    let bill_id = the_bill(&worker, &fixture, 150_000);
    assert_eq!(worker.loan_double_count_warnings().unwrap().len(), 2);
    mark_not(&worker, bill_id, Some(fixture.loan)).unwrap();
    let remaining = worker.loan_double_count_warnings().unwrap();
    assert_eq!(remaining.len(), 1, "only the OTHER loan still warns");
    assert_eq!(remaining[0].loan_account_id, other.as_uuid());
}

#[test]
fn saying_not_the_payment_of_the_linked_loan_unlinks_and_linking_forgets_the_answer() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    let bill_id = the_bill(&worker, &fixture, 150_000);
    link(&worker, bill_id, fixture.loan).unwrap();
    mark_not(&worker, bill_id, Some(fixture.loan)).unwrap();
    assert_eq!(
        stored_link(&worker, bill_id),
        None,
        "the two answers contradict"
    );
    assert_eq!(outflow_on_the_15th(&worker), 300_000);

    link(&worker, bill_id, fixture.loan).unwrap();
    assert_eq!(
        stored_not_payment(&worker, bill_id),
        None,
        "linking forgets the earlier No"
    );
    assert_eq!(stored_link(&worker, bill_id), Some(fixture.loan.as_uuid()));
}

#[test]
fn the_answer_needs_a_real_loan_and_a_real_bill() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    let bill_id = the_bill(&worker, &fixture, 150_000);
    assert!(
        mark_not(&worker, bill_id, Some(fixture.checking)).is_err(),
        "not a loan"
    );
    assert!(mark_not(&worker, RecurringEventId::new(), Some(fixture.loan)).is_err());
    assert_eq!(stored_not_payment(&worker, bill_id), None);
}

// ----- a linked pair is no longer an overlap warning -----------------------------------------------------

#[test]
fn a_linked_pair_is_no_longer_an_overlap_warning() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    let bill_id = bill(
        &worker,
        "ACME HOME LOANS",
        "loan_payment",
        150_000,
        Frequency::Monthly,
        the_15th(),
        fixture.checking,
    );
    assert_eq!(
        worker.loan_double_count_warnings().unwrap().len(),
        1,
        "a strong match first"
    );
    link(&worker, bill_id, fixture.loan).unwrap();
    assert!(
        worker.loan_double_count_warnings().unwrap().is_empty(),
        "the double count it described is resolved"
    );
    unlink(&worker, bill_id);
    assert_eq!(worker.loan_double_count_warnings().unwrap().len(), 1);
}

// ----- the input snapshot ---------------------------------------------------------------------------------

#[test]
fn the_input_snapshot_carries_the_link_only_when_it_is_set() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    let bill_id = the_bill(&worker, &fixture, 150_000);
    let snapshot = |w: &DbWorker| {
        crate::forecast_persist::gather_inputs(&w.read_connection().unwrap())
            .unwrap()
            .recurring_events_json
    };
    let unlinked = snapshot(&worker);
    assert!(
        !unlinked.contains("linked_loan_account_id"),
        "an unlinked bill serializes exactly as before: {unlinked}"
    );
    link(&worker, bill_id, fixture.loan).unwrap();
    let linked = snapshot(&worker);
    assert!(
        linked.contains(&format!(
            "\"linked_loan_account_id\":\"{}\"",
            fixture.loan.as_uuid()
        )),
        "{linked}"
    );
    // Apart from the link, the bill serializes identically.
    assert_eq!(
        linked.replace(
            &format!(",\"linked_loan_account_id\":\"{}\"", fixture.loan.as_uuid()),
            ""
        ),
        unlinked
    );
    unlink(&worker, bill_id);
    assert_eq!(
        snapshot(&worker),
        unlinked,
        "unlinking restores the exact content"
    );
}

// ----- migration 59 ----------------------------------------------------------------------------------------

fn raw_bill(conn: &rusqlite::Connection, n: u8) {
    conn.execute(
        "INSERT INTO recurring_events (id, name, amount_expected_minor, currency, frequency,
             created_at, updated_at) VALUES (?1, 'Rent', 100, 'USD', 'monthly', 't', 't')",
        [vec![n; 16]],
    )
    .unwrap();
}

#[test]
fn migration_59_adds_two_nullable_uuid_columns_to_recurring_events() {
    let (_dir, worker) = worker();
    let conn = worker.read_connection().unwrap();
    for column in ["linked_loan_account_id", "not_payment_of_loan_account_id"] {
        assert_eq!(
            column_of(&conn, "recurring_events", column),
            Some(("BLOB".to_owned(), false)),
            "{column}: a nullable BLOB, so every existing bill is unlinked"
        );
    }
}

#[test]
fn migration_59_leaves_existing_bills_unlinked_and_rolls_back_and_forward() {
    let (_dir, worker) = worker();
    let mut conn = worker.read_connection().unwrap();
    migrations::migrate_down(&mut conn, 58).unwrap();
    assert_eq!(user_version(&conn), 58);
    for column in ["linked_loan_account_id", "not_payment_of_loan_account_id"] {
        assert_eq!(
            column_of(&conn, "recurring_events", column),
            None,
            "{column}"
        );
    }
    raw_bill(&conn, 2);

    migrations::run_migrations(&mut conn).unwrap();
    assert_eq!(user_version(&conn), migrations::CURRENT_VERSION);
    let (rows, linked, answered): (i64, i64, i64) = conn
        .query_row(
            "SELECT COUNT(*), COUNT(linked_loan_account_id), COUNT(not_payment_of_loan_account_id)
               FROM recurring_events",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!((rows, linked, answered), (1, 0, 0), "kept, and unlinked");
    let integrity: String = conn
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");
}

/// Rolling 59 back would quietly forget the user's links and answers, so the down refuses while any
/// exists (a live link, a live answer, or a deleted bill's answer), atomically, and works once cleared.
#[test]
fn migration_59_down_refuses_while_a_link_or_answer_exists_and_is_atomic() {
    for (label, set) in [
        ("a live link", "linked_loan_account_id = zeroblob(16)"),
        (
            "a live answer",
            "not_payment_of_loan_account_id = zeroblob(16)",
        ),
        (
            "a deleted bill's answer",
            "not_payment_of_loan_account_id = zeroblob(16), deleted_at = 't'",
        ),
    ] {
        let (_dir, worker) = worker();
        let mut conn = worker.read_connection().unwrap();
        raw_bill(&conn, 2);
        conn.execute(&format!("UPDATE recurring_events SET {set}"), [])
            .unwrap();

        assert!(migrations::migrate_down(&mut conn, 58).is_err(), "{label}");
        // Atomic: still at the current version, the columns and the data intact.
        assert_eq!(user_version(&conn), migrations::CURRENT_VERSION, "{label}");
        assert!(column_of(&conn, "recurring_events", "linked_loan_account_id").is_some());
        let kept: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM recurring_events
                  WHERE linked_loan_account_id IS NOT NULL
                     OR not_payment_of_loan_account_id IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(kept, 1, "{label}: nothing was lost");

        conn.execute(
            "UPDATE recurring_events
                SET linked_loan_account_id = NULL, not_payment_of_loan_account_id = NULL",
            [],
        )
        .unwrap();
        migrations::migrate_down(&mut conn, 58).unwrap();
        assert_eq!(
            user_version(&conn),
            58,
            "{label}: cleared, so it rolls back"
        );
        assert!(column_of(&conn, "recurring_events", "linked_loan_account_id").is_none());
    }
}

// ----- write-set capture: the link, and what clears it, replicate ---------------------------------------------

fn last_tables(worker: &DbWorker) -> Vec<String> {
    worker
        .last_write_set()
        .expect("a dispatched command records its write set")
        .rows
        .into_iter()
        .map(|(table, _)| table)
        .collect()
}

fn last_touches(worker: &DbWorker, table: &str, id: Uuid) -> bool {
    let wanted = id.as_bytes().to_vec();
    worker
        .last_write_set()
        .expect("write set")
        .rows
        .iter()
        .any(|(t, key)| t == table && key.windows(16).any(|w| w == wanted.as_slice()))
}

#[test]
fn linking_and_unlinking_write_exactly_the_bills_row_and_a_no_op_writes_nothing() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    let bill_id = the_bill(&worker, &fixture, 150_000);

    link(&worker, bill_id, fixture.loan).unwrap();
    assert_eq!(last_tables(&worker), ["recurring_events"]);
    assert!(last_touches(&worker, "recurring_events", bill_id.as_uuid()));

    // Linking the same pair again changes no row: an empty write set.
    link(&worker, bill_id, fixture.loan).unwrap();
    assert!(
        last_tables(&worker).is_empty(),
        "{:?}",
        last_tables(&worker)
    );

    unlink(&worker, bill_id);
    assert_eq!(last_tables(&worker), ["recurring_events"]);
    unlink(&worker, bill_id);
    assert!(
        last_tables(&worker).is_empty(),
        "unlinking an unlinked bill writes nothing"
    );
}

#[test]
fn archiving_a_linked_loan_puts_the_bills_cleared_link_in_that_commands_write_set() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    let bill_id = the_bill(&worker, &fixture, 150_000);

    // No link: archiving a loan touches only the account.
    let lonely = AccountId::new();
    create_role_account(&worker, lonely, "Lonely Loan", CashflowRole::LoanLiability);
    worker
        .dispatch(meta(), WriteCommand::ArchiveAccount(lonely))
        .unwrap();
    assert_eq!(last_tables(&worker), ["accounts"]);

    link(&worker, bill_id, fixture.loan).unwrap();
    worker
        .dispatch(meta(), WriteCommand::ArchiveAccount(fixture.loan))
        .unwrap();
    // The cleared link is part of THIS command's write set, so it replicates with it.
    assert!(last_touches(&worker, "accounts", fixture.loan.as_uuid()));
    assert!(last_touches(&worker, "recurring_events", bill_id.as_uuid()));
}

#[test]
fn archiving_or_deleting_a_linked_bill_puts_its_cleared_link_in_the_write_set() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    let bill_id = the_bill(&worker, &fixture, 150_000);
    link(&worker, bill_id, fixture.loan).unwrap();
    worker
        .dispatch(meta(), WriteCommand::ArchiveRecurringBill(bill_id))
        .unwrap();
    assert!(last_touches(&worker, "recurring_events", bill_id.as_uuid()));

    let other = the_bill(&worker, &fixture, 150_000);
    link(&worker, other, fixture.loan).unwrap();
    worker
        .dispatch(
            meta(),
            WriteCommand::DeleteRecurringBill { event_id: other },
        )
        .unwrap();
    assert!(last_touches(&worker, "recurring_events", other.as_uuid()));
    assert_eq!(stored_link(&worker, other), None);
}

#[test]
fn remembering_and_clearing_the_answer_write_the_bills_row_and_repeating_writes_nothing() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    let bill_id = the_bill(&worker, &fixture, 150_000);
    mark_not(&worker, bill_id, Some(fixture.loan)).unwrap();
    assert_eq!(last_tables(&worker), ["recurring_events"]);
    mark_not(&worker, bill_id, Some(fixture.loan)).unwrap();
    assert!(
        last_tables(&worker).is_empty(),
        "the same answer again changes nothing"
    );
    mark_not(&worker, bill_id, None).unwrap();
    assert_eq!(last_tables(&worker), ["recurring_events"]);
    mark_not(&worker, bill_id, None).unwrap();
    assert!(last_tables(&worker).is_empty());
}

// ----- gaps found by mutation testing ----------------------------------------------------------------------

/// No command turns `include_in_forecast` off after creation, so this is defensive: a linked bill
/// that is somehow excluded from the forecast must not suppress the loan's row (cash would vanish).
#[test]
fn a_linked_bill_that_is_not_in_the_forecast_does_not_carry_the_loan() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    let bill_id = the_bill(&worker, &fixture, 150_000);
    link(&worker, bill_id, fixture.loan).unwrap();
    assert_eq!(outflow_on_the_15th(&worker), 150_000);

    worker
        .read_connection()
        .unwrap()
        .execute(
            "UPDATE recurring_events SET include_in_forecast = 0 WHERE id = ?1",
            [bill_id.as_uuid()],
        )
        .unwrap();
    assert!(!status_of(&worker, bill_id).unwrap().in_force);
    // The bill is out of the forecast and the link is not in force, so the loan's own row is the
    // only one: the obligation is still counted exactly once.
    assert_eq!(outflow_on_the_15th(&worker), 150_000);
    assert_eq!(events_on_the_15th(&worker)[0].0, "loan_payment");
}

/// The part of the bill beyond the loan's payment is only a "split" while the link carries the
/// cash. Once the cadence changes both rows count, and no split is claimed.
#[test]
fn no_split_is_shown_when_the_link_is_not_in_force() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    let bill_id = the_bill(&worker, &fixture, 270_000);
    link(&worker, bill_id, fixture.loan).unwrap();
    assert!(status_of(&worker, bill_id).unwrap().other.is_some());

    worker
        .dispatch(
            meta(),
            WriteCommand::UpdateRecurringBill {
                event_id: bill_id,
                name: "ACME HOME LOANS".to_owned(),
                amount: Money::new(270_000, Currency::Usd),
                bill_type: "subscription".to_owned(),
                frequency: Frequency::Weekly,
                anchor: the_15th(),
                autopay_account_id: Some(fixture.checking),
                description: None,
            },
        )
        .unwrap();
    let status = status_of(&worker, bill_id).unwrap();
    assert!(!status.in_force && status.cadence_changed);
    assert_eq!(status.other, None, "no split while both rows count");
}

/// A credit card WITH payment terms is still not a loan: the role is what refuses it.
#[test]
fn a_credit_card_with_payment_terms_cannot_be_linked() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    let bill_id = the_bill(&worker, &fixture, 150_000);
    let card = AccountId::new();
    create_role_account(&worker, card, "Visa", CashflowRole::CreditFacility);
    worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: card,
                terms: loan_terms(
                    RepaymentPhilosophy::PayFixedAmount,
                    Some(150_000),
                    fixture.checking,
                ),
            },
        )
        .unwrap();
    let refused = link(&worker, bill_id, card).unwrap_err().to_string();
    assert!(
        refused.contains("loan account"),
        "refused because of the role: {refused}"
    );
    assert_eq!(stored_link(&worker, bill_id), None);
}

/// `UpdateRecurringBill` rewrites a bill's currency. A linked dollar bill edited into euros is no
/// longer the dollar loan's payment, so the link must stop being in force and must stop suppressing the
/// loan's row (04b's F2: removing the same-currency condition passed every test). Checked on the link
/// status and on the suppression set itself: a euro bill in a dollar household already makes the whole
/// forecast refuse to build (an existing rule, independent of links), so the forecast total cannot
/// show this.
#[test]
fn editing_the_linked_bills_currency_stops_it_carrying_the_loan() {
    let (_dir, worker) = worker();
    let fixture = mortgage(&worker);
    let bill_id = the_bill(&worker, &fixture, 150_000);
    link(&worker, bill_id, fixture.loan).unwrap();
    let carried = |w: &DbWorker| {
        crate::forecast::loans_with_link_in_force(&w.read_connection().unwrap()).unwrap()
    };
    assert!(carried(&worker).contains(&fixture.loan.as_uuid()));

    worker
        .dispatch(
            meta(),
            WriteCommand::UpdateRecurringBill {
                event_id: bill_id,
                name: "ACME HOME LOANS".to_owned(),
                amount: Money::new(150_000, Currency::Eur),
                bill_type: "subscription".to_owned(),
                frequency: Frequency::Monthly,
                anchor: the_15th(),
                autopay_account_id: None,
                description: None,
            },
        )
        .unwrap();
    // The link is still stored, but the euro bill is not the dollar loan's payment.
    assert_eq!(stored_link(&worker, bill_id), Some(fixture.loan.as_uuid()));
    assert!(!status_of(&worker, bill_id).unwrap().in_force);
    assert!(
        carried(&worker).is_empty(),
        "the loan's own row is no longer suppressed by a bill in another currency"
    );

    // Restoring the currency restores the link.
    worker
        .dispatch(
            meta(),
            WriteCommand::UpdateRecurringBill {
                event_id: bill_id,
                name: "ACME HOME LOANS".to_owned(),
                amount: Money::new(150_000, Currency::Usd),
                bill_type: "subscription".to_owned(),
                frequency: Frequency::Monthly,
                anchor: the_15th(),
                autopay_account_id: None,
                description: None,
            },
        )
        .unwrap();
    assert!(status_of(&worker, bill_id).unwrap().in_force);
}
