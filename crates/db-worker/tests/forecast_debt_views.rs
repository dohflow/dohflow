//! Future Cash forecast views: cards, loans, payoff, readiness, availability, and forecast persistence
//! (moved verbatim from the old in-file `lib.rs` tests module).

mod common;

use chrono::{NaiveDate, Utc};
use common::*;
use core_ledger::{
    Account, AccountFlags, AccountId, BillContractId, CashflowRole, LedgerAccountId,
    RecurringEventId, TransactionId,
};
use core_money::{Currency, Money};
use db_worker::*;
use pay_schedule::Frequency;
use rusqlite::params;
use uuid::Uuid;

/// ADR 0039 (xcq): the credit-card cycle/statement schema round-trips, defaults sanely,
/// and the cycle `status` CHECK rejects an unknown token.
#[test]
fn credit_card_cycle_schema_round_trips_and_checks_status() {
    let (_dir, worker) = worker();
    let account_id = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(account_id)),
                opening_balance: None,
            },
        )
        .unwrap();
    let conn = worker.read_connection().unwrap();

    // A cycle row round-trips and the money columns default to 0.
    conn.execute(
        "INSERT INTO credit_card_cycles
                (account_id, cycle_close, payment_due, currency, status, created_at, updated_at)
             VALUES (?1, '2026-07-01', '2026-07-21', 'USD', 'open',
                     '2026-06-30T00:00:00Z', '2026-06-30T00:00:00Z')",
        [account_id.as_uuid()],
    )
    .unwrap();
    let (status, carried): (String, i64) = conn
        .query_row(
            "SELECT status, carried_opening_balance_minor FROM credit_card_cycles
                 WHERE account_id = ?1",
            [account_id.as_uuid()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(status, "open");
    assert_eq!(carried, 0, "carried_opening_balance_minor defaults to 0");

    // An unknown status is rejected by the CHECK.
    assert!(
        conn.execute(
            "INSERT INTO credit_card_cycles
                    (account_id, cycle_close, currency, status, created_at, updated_at)
                 VALUES (?1, '2026-08-01', 'USD', 'bogus', 'x', 'x')",
            [account_id.as_uuid()],
        )
        .is_err(),
        "an unknown cycle status must be rejected by the CHECK"
    );

    // The statements table exists.
    let statements: i64 = conn
        .query_row("SELECT count(*) FROM credit_card_statements", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(statements, 0);
}

/// ADR 0039 §2 (4lhm): the per-card statement forecast buckets known card-charged bills
/// into each cycle and selects the payment by the repayment philosophy.
#[test]
fn card_statement_forecast_projects_known_bill_charges_and_payment() {
    use chrono::Datelike;
    let (_dir, worker) = worker();
    let checking_id = AccountId::new();
    let card = AccountId::new();
    create_role_account(&worker, checking_id, "Checking", CashflowRole::LiquidCash);
    create_role_account(&worker, card, "Visa", CashflowRole::CreditFacility);
    // close 5 / due 25, PayStatementBalance, min 1% or $25.
    worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: card,
                terms: sample_debt_terms(Some(checking_id)),
            },
        )
        .unwrap();
    // A $10 monthly subscription on the card.
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateRecurringBill {
                event_id: RecurringEventId::new(),
                contract_id: BillContractId::new(),
                name: "Spotify".to_owned(),
                amount: Money::new(1_000, Currency::Usd),
                bill_type: "subscription".to_owned(),
                frequency: Frequency::Monthly,
                anchor: NaiveDate::from_ymd_opt(2026, 7, 10).unwrap(),
                autopay_account_id: Some(card),
                description: None,
                source_merchant_key: None,
                category_id: None,
                tag_ids: Vec::new(),
            },
        )
        .unwrap();

    let as_of = NaiveDate::from_ymd_opt(2026, 7, 2)
        .unwrap()
        .and_hms_opt(12, 0, 0)
        .unwrap()
        .and_utc();
    let forecast = worker.card_statement_forecast_at(as_of).unwrap();
    assert_eq!(forecast.len(), 1, "one card with a derivable cycle");
    let card_fc = &forecast[0];
    assert_eq!(card_fc.account_name, "Visa");
    assert_eq!(card_fc.repayment_philosophy, "pay_statement_balance");
    assert_eq!(
        card_fc.credit_limit_minor, 500_000,
        "credit limit flows through for utilization"
    );
    assert_eq!(card_fc.cycles.len(), 3);
    for cycle in &card_fc.cycles {
        assert_eq!(cycle.close_date.day(), 5);
        assert_eq!(cycle.due_date.day(), 25);
        assert_eq!(cycle.projected_variable_minor, 0, "no variable history yet");
        // The current (partial) cycle projects only charges still to come, so it may hold
        // 0 or the $10; a full future cycle always holds exactly one $10 monthly charge.
        assert!(cycle.known_charges_minor <= 1_000);
    }
    // The two full future cycles each carry one $10 monthly charge, paid in full.
    for cycle in &card_fc.cycles[1..] {
        assert_eq!(
            cycle.known_charges_minor, 1_000,
            "one $10 monthly charge per full cycle"
        );
        assert_eq!(cycle.statement_balance_minor, 1_000);
        assert_eq!(cycle.forecast_payment_minor, 1_000);
        assert_eq!(cycle.minimum_due_minor, 1_000);
    }
}

/// ADR 0039 §2 (4lhm): the projected statement also includes the card's own ordinary
/// variable spend (pezm.1 baseline), as a per-cycle monthly average.
#[test]
fn card_statement_forecast_includes_projected_variable_card_spend() {
    let (_dir, worker) = worker();
    let checking_id = AccountId::new();
    let card = AccountId::new();
    create_role_account(&worker, checking_id, "Checking", CashflowRole::LiquidCash);
    create_role_account(&worker, card, "Visa", CashflowRole::CreditFacility);
    worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: card,
                terms: sample_debt_terms(Some(checking_id)),
            },
        )
        .unwrap();
    let category = variable_category(&worker);
    // ~$200/month of categorized variable spend ON THE CARD over 3 distinct months, plus a
    // much larger amount on CHECKING — the card's projection must reflect only the card's
    // spend (proving read_variable_spend_history(Some(card)) excludes other accounts).
    let today = NaiveDate::from_ymd_opt(2026, 7, 2).unwrap();
    for m in 1..=3u32 {
        let date = today.checked_sub_months(chrono::Months::new(m)).unwrap();
        plant_categorized(&worker, card, -20_000, date, category);
        plant_categorized(&worker, checking_id, -99_900, date, category);
    }

    let as_of = NaiveDate::from_ymd_opt(2026, 7, 2)
        .unwrap()
        .and_hms_opt(12, 0, 0)
        .unwrap()
        .and_utc();
    let forecast = worker.card_statement_forecast_at(as_of).unwrap();
    let card_fc = &forecast[0];
    // A full future cycle carries the whole $200 monthly average (the card's spend only,
    // not checking's); the current partial cycle is prorated, so ≤ $200.
    for cycle in &card_fc.cycles[1..] {
        assert_eq!(
            cycle.projected_variable_minor, 20_000,
            "projected variable card spend must reflect ONLY the card's spend, not checking's"
        );
    }
    assert!(card_fc.cycles[0].projected_variable_minor <= 20_000);
}

/// ADR 0039 §2 (4lhm review): a card-charged bill hidden from the forecast
/// (`include_in_forecast = 0`) contributes nothing to the card statement balance, matching
/// every other forecast surface.
#[test]
fn card_statement_forecast_excludes_bills_hidden_from_the_forecast() {
    let (_dir, worker) = worker();
    let checking_id = AccountId::new();
    let card = AccountId::new();
    create_role_account(&worker, checking_id, "Checking", CashflowRole::LiquidCash);
    create_role_account(&worker, card, "Visa", CashflowRole::CreditFacility);
    worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: card,
                terms: sample_debt_terms(Some(checking_id)),
            },
        )
        .unwrap();
    let event_id = RecurringEventId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateRecurringBill {
                event_id,
                contract_id: BillContractId::new(),
                name: "Spotify".to_owned(),
                amount: Money::new(1_000, Currency::Usd),
                bill_type: "subscription".to_owned(),
                frequency: Frequency::Monthly,
                anchor: NaiveDate::from_ymd_opt(2026, 7, 10).unwrap(),
                autopay_account_id: Some(card),
                description: None,
                source_merchant_key: None,
                category_id: None,
                tag_ids: Vec::new(),
            },
        )
        .unwrap();
    // Hide it from the forecast.
    worker
        .read_connection()
        .unwrap()
        .execute(
            "UPDATE recurring_events SET include_in_forecast = 0 WHERE id = ?1",
            [event_id.as_uuid()],
        )
        .unwrap();

    let as_of = NaiveDate::from_ymd_opt(2026, 7, 2)
        .unwrap()
        .and_hms_opt(12, 0, 0)
        .unwrap()
        .and_utc();
    let forecast = worker.card_statement_forecast_at(as_of).unwrap();
    assert!(
        forecast[0]
            .cycles
            .iter()
            .all(|c| c.known_charges_minor == 0),
        "a bill hidden from the forecast must not inflate the card statement"
    );
}

/// ADR 0039 §2 (4lhm review): at a late close day (28th) a month-end-anchored monthly bill
/// (the 31st, clamped) lands exactly once per cycle — not 2× in one and 0 in the next.
#[test]
fn card_statement_forecast_does_not_double_count_month_end_bills() {
    use chrono::Datelike;
    let (_dir, worker) = worker();
    let checking_id = AccountId::new();
    let card = AccountId::new();
    create_role_account(&worker, checking_id, "Checking", CashflowRole::LiquidCash);
    create_role_account(&worker, card, "Visa", CashflowRole::CreditFacility);
    let mut terms = sample_debt_terms(Some(checking_id));
    terms.statement_close_day = Some(28);
    terms.payment_due_day = Some(18);
    worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: card,
                terms,
            },
        )
        .unwrap();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateRecurringBill {
                event_id: RecurringEventId::new(),
                contract_id: BillContractId::new(),
                name: "Rent share".to_owned(),
                amount: Money::new(5_000, Currency::Usd),
                bill_type: "subscription".to_owned(),
                frequency: Frequency::Monthly,
                anchor: NaiveDate::from_ymd_opt(2026, 1, 31).unwrap(),
                autopay_account_id: Some(card),
                description: None,
                source_merchant_key: None,
                category_id: None,
                tag_ids: Vec::new(),
            },
        )
        .unwrap();

    let as_of = NaiveDate::from_ymd_opt(2026, 7, 20)
        .unwrap()
        .and_hms_opt(12, 0, 0)
        .unwrap()
        .and_utc();
    let forecast = worker.card_statement_forecast_at(as_of).unwrap();
    let card_fc = &forecast[0];
    assert_eq!(card_fc.cycles.len(), 3);
    // No cycle is double-counted (≤ one $50 charge each); the full future cycles carry
    // exactly one — never 2× in one cycle and 0 in the next (the month-end bucketing bug).
    for cycle in &card_fc.cycles {
        assert_eq!(cycle.close_date.day(), 28);
        assert!(
            cycle.known_charges_minor <= 5_000,
            "no cycle may double-count a monthly bill; got {} on {}",
            cycle.known_charges_minor,
            cycle.close_date
        );
    }
    for cycle in &card_fc.cycles[1..] {
        assert_eq!(
            cycle.known_charges_minor, 5_000,
            "each full future cycle holds exactly one $50 charge, got {} on {}",
            cycle.known_charges_minor, cycle.close_date
        );
    }
}

/// ADR 0035 §4 (llx5): a card carrying an owed balance under a partial (minimum) payment
/// accrues interest on the carried balance and revolves it into the next cycle.
#[test]
fn card_statement_forecast_accrues_interest_on_a_carried_balance() {
    let (_dir, worker) = worker();
    let checking_id = AccountId::new();
    let card = AccountId::new();
    create_role_account(&worker, checking_id, "Checking", CashflowRole::LiquidCash);
    create_role_account(&worker, card, "Visa", CashflowRole::CreditFacility);
    // close 5 / due 25, but pay_minimum + 24% APR so a carried balance revolves.
    let mut terms = sample_debt_terms(Some(checking_id));
    terms.repayment_philosophy = RepaymentPhilosophy::PayMinimum;
    terms.apr_bps = Some(2_400);
    worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: card,
                terms,
            },
        )
        .unwrap();
    // The card currently owes $1,000 — a liability's stored balance is negative.
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            card,
            Money::new(-100_000, Currency::Usd),
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        )
        .unwrap();

    let as_of = NaiveDate::from_ymd_opt(2026, 7, 2)
        .unwrap()
        .and_hms_opt(12, 0, 0)
        .unwrap()
        .and_utc();
    let forecast = worker.card_statement_forecast_at(as_of).unwrap();
    let card_fc = &forecast[0];
    assert_eq!(
        card_fc.cycles[0].carried_opening_balance_minor, 100_000,
        "the owed balance opens the projection"
    );
    assert!(
        card_fc.cycles[0].accrued_interest_minor > 0,
        "interest accrues on the carried balance"
    );
    assert_eq!(
        card_fc.cycles[0].statement_balance_minor,
        100_000 + card_fc.cycles[0].accrued_interest_minor,
        "statement = carried opening + interest (no new charges here)"
    );
    assert!(
        card_fc.cycles[1].carried_opening_balance_minor > 0,
        "a minimum-only payment leaves a balance revolving into the next cycle"
    );
}

/// ADR 0036 debt_payoff (od07): the payoff comparison ranks the three strategies over the
/// household's current debts; avalanche never pays more interest than snowball.
#[test]
fn debt_payoff_comparison_ranks_strategies_for_the_household_debts() {
    let (_dir, worker) = worker();
    let checking_id = AccountId::new();
    let card = AccountId::new();
    let loan = AccountId::new();
    create_role_account(&worker, checking_id, "Checking", CashflowRole::LiquidCash);
    create_role_account(&worker, card, "Visa", CashflowRole::CreditFacility);
    create_role_account(&worker, loan, "Auto loan", CashflowRole::LoanLiability);
    // Card owes $2,000 at ~22% APR, pay_minimum.
    let mut card_terms = sample_debt_terms(Some(checking_id));
    card_terms.repayment_philosophy = RepaymentPhilosophy::PayMinimum;
    worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: card,
                terms: card_terms,
            },
        )
        .unwrap();
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            card,
            Money::new(-200_000, Currency::Usd),
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        )
        .unwrap();
    // Loan owes $5,000 at 6% APR, $200/mo fixed.
    let mut lt = loan_terms(
        RepaymentPhilosophy::PayFixedAmount,
        Some(20_000),
        checking_id,
    );
    lt.apr_bps = Some(600);
    worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: loan,
                terms: lt,
            },
        )
        .unwrap();
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            loan,
            Money::new(-500_000, Currency::Usd),
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        )
        .unwrap();

    let plans = worker.debt_payoff_comparison(20_000, &[]).unwrap(); // $200/mo extra
    assert_eq!(plans.len(), 3);
    assert_eq!(plans[0].strategy, "minimum_only");
    assert_eq!(plans[1].strategy, "snowball");
    assert_eq!(plans[2].strategy, "avalanche");
    // With the extra budget, snowball and avalanche clear the debts within the horizon.
    assert!(
        plans[1].debt_free_month.is_some(),
        "snowball clears with extra"
    );
    assert!(
        plans[2].debt_free_month.is_some(),
        "avalanche clears with extra"
    );
    // Avalanche is interest-optimal: it never pays more interest than snowball.
    assert!(plans[2].total_interest_minor <= plans[1].total_interest_minor);
    // The burndown trajectory (6wk.16) starts at the total owed ($2,000 + $5,000) and, for a
    // plan that clears within the horizon, ends at zero.
    assert_eq!(plans[1].monthly_total_owed_minor[0], 700_000);
    assert_eq!(*plans[1].monthly_total_owed_minor.last().unwrap(), 0);
    // 6wk.13: the comparison carries the reference currency (USD by default).
    assert_eq!(plans[1].currency, "USD");
    // 6wk.17: one per-debt series per debt, labelled, each summing to the aggregate per month.
    assert_eq!(plans[1].per_debt.len(), 2, "the card + the loan");

    // Scoping to ONE debt (personal-cfo-4d8.27.9.7). The scope must reach the SIMULATION,
    // not its output — so this asserts things a post-filter could not produce:
    let card_only = worker
        .debt_payoff_comparison(20_000, &[card.as_uuid()])
        .unwrap();
    assert_eq!(card_only[1].per_debt.len(), 1, "just the card");
    // 1. the burndown STARTS at the card's $2,000, not the household's $7,000. A filtered
    //    output would still carry the full starting total.
    assert_eq!(card_only[1].monthly_total_owed_minor[0], 200_000);
    // 2. the payoff MATH changes, not just the rows shown — the assertion a post-filter
    //    cannot satisfy. Scope to the LOAN, not the card: snowball pays the SMALLEST
    //    balance first, so the card already had the whole extra budget in the household
    //    plan and clears at the same month either way. The loan is the debt that was
    //    waiting its turn, so giving it the budget from month 0 clears it strictly sooner.
    let loan_only = worker
        .debt_payoff_comparison(20_000, &[loan.as_uuid()])
        .unwrap();
    let cleared_month = |series: &[i64]| series.iter().position(|&owed| owed == 0);
    let shared = cleared_month(
        &plans[1]
            .per_debt
            .iter()
            .find(|d| d.label == "Auto loan")
            .expect("the loan has a series in the household plan")
            .monthly_owed_minor,
    );
    let alone = cleared_month(&loan_only[1].per_debt[0].monthly_owed_minor);
    assert!(
        alone.is_some() && (shared.is_none() || alone < shared),
        "the loan clears sooner with the extra budget to itself: alone={alone:?} shared={shared:?}",
    );

    // An empty scope still means EVERY debt — inverting that would silently report the
    // household has nothing to pay off.
    assert_eq!(
        worker.debt_payoff_comparison(20_000, &[]).unwrap()[1]
            .per_debt
            .len(),
        2,
    );
    let labels: Vec<&str> = plans[1].per_debt.iter().map(|s| s.label.as_str()).collect();
    assert!(labels.contains(&"Visa") && labels.contains(&"Auto loan"));
    for m in 0..plans[1].monthly_total_owed_minor.len() {
        let per_debt_sum: i64 = plans[1]
            .per_debt
            .iter()
            .map(|s| s.monthly_owed_minor[m])
            .sum();
        assert_eq!(
            per_debt_sum, plans[1].monthly_total_owed_minor[m],
            "per-debt sums to the aggregate at month {m}",
        );
    }
}

/// 6wk.13: a mixed-currency household compares only its reference-currency debts — a
/// foreign-currency debt is excluded rather than summed across currencies.
#[test]
fn debt_payoff_comparison_is_single_currency() {
    let (_dir, worker) = worker();
    let checking_id = AccountId::new();
    let usd_card = AccountId::new();
    let eur_loan = AccountId::new();
    create_role_account(&worker, checking_id, "Checking", CashflowRole::LiquidCash);
    create_role_account(&worker, usd_card, "Visa", CashflowRole::CreditFacility);
    // A EUR loan alongside the USD card (reporting currency defaults to USD).
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(Account::new(
                    eur_loan,
                    LedgerAccountId::new(),
                    "Euro loan",
                    CashflowRole::LoanLiability,
                    Currency::Eur,
                    AccountFlags::default(),
                )),
                opening_balance: None,
            },
        )
        .unwrap();
    let mut card_terms = sample_debt_terms(Some(checking_id));
    card_terms.repayment_philosophy = RepaymentPhilosophy::PayMinimum;
    worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: usd_card,
                terms: card_terms,
            },
        )
        .unwrap();
    worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: eur_loan,
                terms: loan_terms(
                    RepaymentPhilosophy::PayFixedAmount,
                    Some(20_000),
                    checking_id,
                ),
            },
        )
        .unwrap();
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            usd_card,
            Money::new(-200_000, Currency::Usd),
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        )
        .unwrap();
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            eur_loan,
            Money::new(-500_000, Currency::Eur),
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        )
        .unwrap();

    let plans = worker.debt_payoff_comparison(0, &[]).unwrap();
    assert_eq!(plans.len(), 3);
    assert_eq!(plans[0].currency, "USD");
    // Only the USD card is included; the EUR loan is excluded.
    assert_eq!(plans[0].per_debt.len(), 1, "only the USD debt");
    assert_eq!(plans[0].per_debt[0].label, "Visa");
}

/// od07: no debts → an empty comparison (nothing to project).
#[test]
fn debt_payoff_comparison_is_empty_without_debts() {
    let (_dir, worker) = worker();
    let checking_id = AccountId::new();
    create_role_account(&worker, checking_id, "Checking", CashflowRole::LiquidCash);
    assert!(worker
        .debt_payoff_comparison(10_000, &[])
        .unwrap()
        .is_empty());
}

/// 6wk.11: a loan tracked as both a loan account with terms AND a same-named `loan_payment`
/// bill is flagged (name match) so the user can remove one and avoid the double-count.
#[test]
fn loan_double_count_flags_a_same_named_loan_and_bill() {
    let (_dir, worker) = worker();
    let checking_id = AccountId::new();
    create_role_account(&worker, checking_id, "Checking", CashflowRole::LiquidCash);
    let loan = create_loan_with_terms(
        &worker,
        "Car Loan",
        RepaymentPhilosophy::PayMinimum,
        None,
        checking_id,
    );
    create_loan_payment_bill(&worker, "Car-Loan Payment", 45_000, checking_id);

    let warnings = worker.loan_double_count_warnings().unwrap();
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].loan_account_id, loan.as_uuid());
    assert!(warnings[0].name_match, "same-named loan and bill");
    assert!(
        !warnings[0].amount_match,
        "loan has no fixed amount to match"
    );
}

/// 6wk.11: a differently-named loan and bill are still flagged when the loan's fixed monthly
/// payment equals the bill amount (same currency).
#[test]
fn loan_double_count_flags_a_matching_fixed_amount() {
    let (_dir, worker) = worker();
    let checking_id = AccountId::new();
    create_role_account(&worker, checking_id, "Checking", CashflowRole::LiquidCash);
    create_loan_with_terms(
        &worker,
        "Auto Financing",
        RepaymentPhilosophy::PayFixedAmount,
        Some(45_000),
        checking_id,
    );
    create_loan_payment_bill(&worker, "Toyota", 45_000, checking_id);

    let warnings = worker.loan_double_count_warnings().unwrap();
    assert_eq!(warnings.len(), 1);
    assert!(!warnings[0].name_match, "names do not overlap");
    assert!(
        warnings[0].amount_match,
        "the $450 fixed payment matches the bill"
    );
}

/// 6wk.11: unrelated loans/bills do not warn, and a same-named non-`loan_payment` bill
/// (e.g. a subscription) is never flagged — only the double-count case is.
#[test]
fn loan_double_count_ignores_unrelated_and_non_loan_bills() {
    let (_dir, worker) = worker();
    let checking_id = AccountId::new();
    create_role_account(&worker, checking_id, "Checking", CashflowRole::LiquidCash);
    create_loan_with_terms(
        &worker,
        "Car Loan",
        RepaymentPhilosophy::PayMinimum,
        None,
        checking_id,
    );
    // A loan_payment bill for a different loan (no name/amount overlap) → not the same loan.
    create_loan_payment_bill(&worker, "Mortgage", 120_000, checking_id);
    // A same-named bill that is NOT a loan_payment (a subscription) → not a double-count.
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateRecurringBill {
                event_id: RecurringEventId::new(),
                contract_id: BillContractId::new(),
                name: "Car Loan Payment".to_owned(),
                amount: Money::new(45_000, Currency::Usd),
                bill_type: "subscription".to_owned(),
                frequency: Frequency::Monthly,
                anchor: NaiveDate::from_ymd_opt(2026, 7, 15).unwrap(),
                autopay_account_id: Some(checking_id),
                description: None,
                source_merchant_key: None,
                category_id: None,
                tag_ids: Vec::new(),
            },
        )
        .unwrap();

    assert!(worker.loan_double_count_warnings().unwrap().is_empty());
}

/// 6wk.11 (review): a paid-off loan (owed = 0) emits no forecast payment, so pairing it with a
/// matching bill would not double-count anything — it is not flagged.
#[test]
fn loan_double_count_ignores_a_paid_off_loan() {
    let (_dir, worker) = worker();
    let checking_id = AccountId::new();
    create_role_account(&worker, checking_id, "Checking", CashflowRole::LiquidCash);
    // A loan account with terms but NO owed balance (never asserted → owed 0).
    let id = AccountId::new();
    create_role_account(&worker, id, "Car Loan", CashflowRole::LoanLiability);
    worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: id,
                terms: loan_terms(RepaymentPhilosophy::PayMinimum, None, checking_id),
            },
        )
        .unwrap();
    create_loan_payment_bill(&worker, "Car-Loan Payment", 45_000, checking_id);

    assert!(
        worker.loan_double_count_warnings().unwrap().is_empty(),
        "a paid-off loan contributes no payment, so there is nothing to double-count"
    );
}

/// od07 (review #4): full-payment cards (paid off each cycle) are excluded from the payoff
/// set, and a carry-debt with no configured minimum falls back to the default so it amortizes.
#[test]
fn debt_payoff_excludes_full_payment_cards_and_defaults_missing_minimums() {
    let (_dir, worker) = worker();
    let checking = AccountId::new();
    let payer = AccountId::new();
    let carry = AccountId::new();
    create_role_account(&worker, checking, "Checking", CashflowRole::LiquidCash);
    create_role_account(&worker, payer, "Amex", CashflowRole::CreditFacility);
    create_role_account(&worker, carry, "Visa", CashflowRole::CreditFacility);
    // A full-payment card (pay_statement_balance) with a balance → not carry-debt.
    worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: payer,
                terms: sample_debt_terms(Some(checking)),
            },
        )
        .unwrap();
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            payer,
            Money::new(-100_000, Currency::Usd),
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        )
        .unwrap();
    assert!(
        worker.debt_payoff_comparison(0, &[]).unwrap().is_empty(),
        "a full-payment card is not carry-debt"
    );

    // A pay_minimum card with NO explicit minimum terms → defaulted, amortizes.
    let mut cm = sample_debt_terms(Some(checking));
    cm.repayment_philosophy = RepaymentPhilosophy::PayMinimum;
    cm.min_payment_percent_bps = None;
    cm.min_payment_floor_minor = None;
    worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: carry,
                terms: cm,
            },
        )
        .unwrap();
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            carry,
            Money::new(-50_000, Currency::Usd), // owes $500
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        )
        .unwrap();
    let plans = worker.debt_payoff_comparison(0, &[]).unwrap();
    assert_eq!(plans.len(), 3);
    assert!(
        plans[0].debt_free_month.is_some(),
        "the defaulted minimum amortizes the carry card"
    );
}

/// 3v6d/ADR 0018 addendum 915.1: the comfort band's lower edge IS the minimum-cash floor
/// (wrapped, not replaced); the upper edge is a separate, optional setting; both read in the
/// reporting currency.
#[test]
fn comfort_band_wraps_the_floor_and_reads_an_optional_upper_edge() {
    let (_dir, worker) = worker();

    // Default: floor 0 as the lower edge, no upper edge.
    let band = worker.comfort_band().unwrap();
    assert_eq!(band.currency, Currency::Usd);
    assert_eq!(band.lower, Money::new(0, Currency::Usd));
    assert_eq!(band.upper, None);

    // The lower edge tracks the shipped floor; the upper edge is its own setting.
    worker
        .set_setting(MINIMUM_CASH_FLOOR_KEY, "500000")
        .unwrap();
    worker
        .set_setting(COMFORT_BAND_UPPER_KEY, "1000000")
        .unwrap();
    worker.set_setting("reporting_currency", "EUR").unwrap();
    let band = worker.comfort_band().unwrap();
    assert_eq!(band.currency, Currency::Eur);
    assert_eq!(band.lower, Money::new(500_000, Currency::Eur));
    assert_eq!(band.upper, Some(Money::new(1_000_000, Currency::Eur)));

    // Clearing the upper edge (blank) reads back as None; the floor is untouched.
    worker.set_setting(COMFORT_BAND_UPPER_KEY, "").unwrap();
    let band = worker.comfort_band().unwrap();
    assert_eq!(band.upper, None);
    assert_eq!(band.lower, Money::new(500_000, Currency::Eur));
}

/// 5ie.8/ADR 0018 §915.1 migration v34: risk_flags gains the `cash_band_breach` flag type and
/// the descriptive `contributing_factors_json` column (renamed from `suggested_actions_json`).
#[test]
fn risk_flags_accepts_cash_band_breach_and_contributing_factors() {
    let (_dir, worker) = worker();
    let conn = worker.read_connection().unwrap();
    // The new flag type + the renamed descriptive-evidence column both work.
    conn.execute(
            "INSERT INTO risk_flags
                (id, flag_type, severity, contributing_factors_json, readiness_required_bps, created_at)
             VALUES (?1, 'cash_band_breach', 'warning', '[{\"category\":\"dining\"}]', 0, 'now')",
            params![Uuid::now_v7()],
        )
        .unwrap();
    // The original flag types still validate.
    conn.execute(
        "INSERT INTO risk_flags
                (id, flag_type, severity, readiness_required_bps, created_at)
             VALUES (?1, 'low_balance', 'info', 0, 'now')",
        params![Uuid::now_v7()],
    )
    .unwrap();
    // The pre-915.1 column name is gone.
    assert!(
            conn.execute(
                "INSERT INTO risk_flags
                    (id, flag_type, severity, suggested_actions_json, readiness_required_bps, created_at)
                 VALUES (?1, 'low_balance', 'info', '[]', 0, 'now')",
                params![Uuid::now_v7()],
            )
            .is_err(),
            "suggested_actions_json was renamed to contributing_factors_json (915.1)"
        );
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM risk_flags", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 2);
}

/// 5ie.8: the band_drift read plumbs the forecast + band + spend history into the engine and
/// returns None when the projection never crosses below the band (the engine's attribution
/// itself is unit-tested in band_drift.rs).
#[test]
fn band_drift_is_none_without_a_below_band_crossing() {
    let (_dir, worker) = worker();
    let checking = AccountId::new();
    create_role_account(&worker, checking, "Checking", CashflowRole::LiquidCash);
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            checking,
            Money::new(1_000_000, Currency::Usd), // $10,000
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        )
        .unwrap();
    // A $5,000 floor with a flat $10,000 projection (no bills/spend) never crosses below.
    worker
        .set_setting(MINIMUM_CASH_FLOOR_KEY, "500000")
        .unwrap();
    assert!(worker.band_drift(Utc::now(), 180).unwrap().is_none());
}

#[test]
fn forecast_starts_from_the_asserted_balance() {
    let (_dir, worker) = worker();
    let account = AccountId::new();
    worker
        .dispatch(meta(), liquid_account_cmd(account, Currency::Usd, 0))
        .unwrap();
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            account,
            Money::new(425_000, Currency::Usd),
            NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
        )
        .unwrap();
    // No income/bills → a flat line anchored at the asserted balance.
    let view = worker.future_cash_forecast(30, &[]).unwrap();
    assert_eq!(view.starting_balance, Money::new(425_000, Currency::Usd));
}

#[test]
fn acknowledge_capability_rejects_unknown_key() {
    let (_dir, worker) = worker();
    assert!(worker
        .acknowledge_capability(&meta(), "not_a_capability")
        .is_err());
}

/// 6wk.6 (ADR 0035 §5): SetDebtTerms upserts on a liability account and reads back, with
/// a liquid paying source accepted.
#[test]
fn set_debt_terms_round_trips_on_a_liability() {
    let (_dir, worker) = worker();
    let card = AccountId::new();
    let checking_id = AccountId::new();
    create_role_account(&worker, card, "Visa", CashflowRole::CreditFacility);
    create_role_account(&worker, checking_id, "Checking", CashflowRole::LiquidCash);

    worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: card,
                terms: sample_debt_terms(Some(checking_id)),
            },
        )
        .unwrap();

    let terms = worker.debt_terms(card).unwrap().expect("terms set");
    assert_eq!(terms.apr_bps, Some(2199));
    assert_eq!(terms.payment_due_day, Some(25));
    assert_eq!(
        terms.repayment_philosophy,
        RepaymentPhilosophy::PayStatementBalance
    );
    assert_eq!(terms.paying_source_account_id, Some(checking_id));
    // Unset accounts read as None.
    assert!(worker.debt_terms(checking_id).unwrap().is_none());
}

/// The list read (personal-cfo-4d17): terms for many debts in one call.
#[test]
fn debt_terms_list_returns_a_row_per_account_with_terms() {
    let (_dir, worker) = worker();
    let card = AccountId::new();
    let loan = AccountId::new();
    let bare = AccountId::new();
    let checking_id = AccountId::new();
    create_role_account(&worker, card, "Visa", CashflowRole::CreditFacility);
    create_role_account(&worker, loan, "Auto loan", CashflowRole::LoanLiability);
    create_role_account(&worker, bare, "Store card", CashflowRole::CreditFacility);
    create_role_account(&worker, checking_id, "Checking", CashflowRole::LiquidCash);

    for account in [card, loan] {
        worker
            .dispatch(
                meta(),
                WriteCommand::SetDebtTerms {
                    account_id: account,
                    terms: sample_debt_terms(Some(checking_id)),
                },
            )
            .unwrap();
    }

    let all = worker.debt_terms_list(&[]).unwrap();
    assert_eq!(all.len(), 2, "one row per account WITH terms");
    // A debt with no terms recorded is OMITTED, not returned with null fields. "No rate
    // recorded" and "no interest" are different facts, and a row of nulls erases the
    // difference — a caller averaging APRs would silently fold in a 0%.
    assert!(all.iter().all(|t| t.account_id != bare));
    // …and the liquid account is not a debt at all.
    assert!(all.iter().all(|t| t.account_id != checking_id));

    // The rows carry the same values the single-account read gives, because both map
    // through one helper — a second hand-written 11-field mapping is what would drift.
    let from_list = all
        .iter()
        .find(|t| t.account_id == card)
        .expect("the card is listed");
    let from_single = worker.debt_terms(card).unwrap().expect("terms set");
    assert_eq!(from_list.apr_bps, from_single.apr_bps);
    assert_eq!(from_list.payment_due_day, from_single.payment_due_day);
    assert_eq!(
        from_list.repayment_philosophy,
        from_single.repayment_philosophy
    );
    assert_eq!(
        from_list.paying_source_account_id,
        from_single.paying_source_account_id
    );

    // Scoping narrows; an empty scope means everything (NOT nothing — inverting that
    // would report a household has no debt terms at all).
    let scoped = worker.debt_terms_list(&[loan.as_uuid()]).unwrap();
    assert_eq!(scoped.len(), 1);
    assert_eq!(scoped[0].account_id, loan);
}

/// 6wk.6: debt terms can only be set on a liability account.
#[test]
fn set_debt_terms_rejects_a_non_liability_target() {
    let (_dir, worker) = worker();
    let checking_id = AccountId::new();
    create_role_account(&worker, checking_id, "Checking", CashflowRole::LiquidCash);
    assert!(
        worker
            .dispatch(
                meta(),
                WriteCommand::SetDebtTerms {
                    account_id: checking_id,
                    terms: sample_debt_terms(None),
                },
            )
            .is_err(),
        "a liquid account is not a valid debt-terms target"
    );
}

/// 6wk.6 (ADR 0035 §2): the paying source must be a liquid-cash account.
#[test]
fn set_debt_terms_rejects_a_non_liquid_paying_source() {
    let (_dir, worker) = worker();
    let card = AccountId::new();
    let brokerage = AccountId::new();
    create_role_account(&worker, card, "Visa", CashflowRole::CreditFacility);
    create_role_account(
        &worker,
        brokerage,
        "Brokerage",
        CashflowRole::InvestmentAsset,
    );
    assert!(
        worker
            .dispatch(
                meta(),
                WriteCommand::SetDebtTerms {
                    account_id: card,
                    terms: sample_debt_terms(Some(brokerage)),
                },
            )
            .is_err(),
        "a brokerage account cannot pay a debt"
    );
    assert!(
        worker.debt_terms(card).unwrap().is_none(),
        "nothing was written"
    );
}

/// 6wk.6: a day-of-month out of 1..=31 is rejected.
#[test]
fn set_debt_terms_rejects_an_out_of_range_day() {
    let (_dir, worker) = worker();
    let card = AccountId::new();
    create_role_account(&worker, card, "Visa", CashflowRole::CreditFacility);
    let mut terms = sample_debt_terms(None);
    terms.payment_due_day = Some(40);
    assert!(worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: card,
                terms
            }
        )
        .is_err());
}

/// 6wk.6: non-negative / bounded domain — a negative APR, an over-100% minimum percent,
/// or a negative money amount is rejected before it can corrupt the forecast math.
#[test]
fn set_debt_terms_rejects_out_of_domain_amounts() {
    let (_dir, worker) = worker();
    let card = AccountId::new();
    create_role_account(&worker, card, "Visa", CashflowRole::CreditFacility);
    let reject = |mutate: fn(&mut DebtTermsInput)| {
        let mut terms = sample_debt_terms(None);
        mutate(&mut terms);
        worker
            .dispatch(
                meta(),
                WriteCommand::SetDebtTerms {
                    account_id: card,
                    terms,
                },
            )
            .is_err()
    };
    assert!(reject(|t| t.apr_bps = Some(-500)), "negative APR");
    assert!(
        reject(|t| t.min_payment_percent_bps = Some(25_000)),
        "minimum > 100%"
    );
    assert!(
        reject(|t| t.credit_limit_minor = Some(-100)),
        "negative limit"
    );
    assert!(reject(|t| t.grace_period_days = Some(-1)), "negative grace");
}

#[test]
fn future_cash_forecast_on_empty_vault_is_flat_zero() {
    let (_dir, worker) = worker();
    // Clock-driven entry point: no accounts/income/bills → a flat USD-0 line.
    let view = worker.future_cash_forecast(30, &[]).unwrap();
    assert_eq!(view.days.len(), 30);
    assert_eq!(view.currency, Currency::Usd);
    assert_eq!(view.starting_balance, Money::zero(Currency::Usd));
    assert!(view
        .days
        .iter()
        .all(|d| d.closing.p50 == Money::zero(Currency::Usd) && d.events.is_empty()));
}

#[test]
fn future_cash_forecast_on_empty_vault_uses_the_base_currency_setting() {
    let (_dir, worker) = worker();
    // With no liquid account to pin a currency, the forecast follows the
    // vault's base-currency setting instead of hardcoding USD
    // (personal-cfo-4n3x). This is what makes a EUR vault's Future Cash and
    // scenario amount labels read in EUR before any account exists.
    worker.set_setting("reporting_currency", "EUR").unwrap();
    let view = worker.future_cash_forecast(30, &[]).unwrap();
    assert_eq!(view.currency, Currency::Eur);
    assert_eq!(view.starting_balance, Money::zero(Currency::Eur));
}

#[test]
fn forecast_run_rows_and_snapshot_round_trip() {
    // personal-cfo-63t: a snapshot -> run -> rows insert reads back in order.
    let (_dir, worker) = worker();
    let conn = worker.read_connection().unwrap();
    let snapshot_id = Uuid::now_v7();
    let run_id = Uuid::now_v7();

    conn.execute(
        "INSERT INTO forecast_input_snapshots
                (id, household_id, created_at, ledger_cutoff_at,
                 included_entity_hashes_json, source_freshness_json, schema_version)
             VALUES (?1, NULL, 'now', 'now', '{}', NULL, 6)",
        params![snapshot_id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO forecast_runs
                (id, generated_at, horizon_days, starting_cash_minor,
                 input_snapshot_id, assumptions_hash, deterministic_run)
             VALUES (?1, 'now', 30, 100000, ?2, 'abc', 1)",
        params![run_id, snapshot_id],
    )
    .unwrap();
    // Insert two rows out of date order; the index query returns them ordered.
    for (date, p50, bal) in [
        ("2026-06-09", 200_i64, 99_800_i64),
        ("2026-06-08", -100, 99_900),
    ] {
        conn.execute(
            "INSERT INTO forecast_rows
                    (id, forecast_run_id, date, amount_p10_minor, amount_p50_minor,
                     amount_p90_minor, running_balance_p10_minor,
                     running_balance_p50_minor, running_balance_p90_minor,
                     source_type, computation_mode)
                 VALUES (?1, ?2, ?3, ?4, ?4, ?4, ?5, ?5, ?5, 'income',
                         'deterministic_incremental')",
            params![Uuid::now_v7(), run_id, date, p50, bal],
        )
        .unwrap();
    }

    let rows: Vec<(String, i64, i64)> = conn
        .prepare(
            "SELECT date, amount_p50_minor, running_balance_p50_minor
                 FROM forecast_rows WHERE forecast_run_id = ?1 ORDER BY date",
        )
        .unwrap()
        .query_map(params![run_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        rows,
        vec![
            ("2026-06-08".to_owned(), -100, 99_900),
            ("2026-06-09".to_owned(), 200, 99_800),
        ]
    );

    // The run links back to its snapshot (per-row explanation support).
    let linked: Uuid = conn
        .query_row(
            "SELECT input_snapshot_id FROM forecast_runs WHERE id = ?1",
            params![run_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(linked, snapshot_id);
}

#[test]
fn forecast_rows_reject_invalid_enum_tokens() {
    let (_dir, worker) = worker();
    let conn = worker.read_connection().unwrap();
    let insert = |source_type: &str, computation_mode: &str| {
        conn.execute(
            "INSERT INTO forecast_rows
                    (id, forecast_run_id, date, amount_p10_minor, amount_p50_minor,
                     amount_p90_minor, running_balance_p10_minor,
                     running_balance_p50_minor, running_balance_p90_minor,
                     source_type, computation_mode)
                 VALUES (?1, ?2, 'now', 0, 0, 0, 0, 0, 0, ?3, ?4)",
            params![
                Uuid::now_v7(),
                Uuid::now_v7(),
                source_type,
                computation_mode
            ],
        )
    };
    assert!(insert("not_a_source", "full_batch").is_err());
    assert!(insert("income", "not_a_mode").is_err());
    assert!(insert("income", "full_batch").is_ok());
}

/// ADR 0039 addendum 2026-07-10 §1 (personal-cfo-4d8.25.2 + .3), the owner scenario:
/// owed 17,082.23; close 22 / due 17; the real Jun-22 statement of 13,873.08 recorded, plus a
/// STALE row keyed to the next (Jul-22, still-future) close — recorded under a shifted
/// derivation. The stale row must be inert (Aug 17 must NOT repeat 13,873.08), the paid
/// statement must carry the 3,209.15 of post-close charges into the next cycle instead of
/// dropping them, and every stored row must be visible for management.
#[test]
fn stale_future_keyed_statement_override_is_inert_and_post_close_charges_carry() {
    let (_dir, worker) = worker();
    let checking = AccountId::new();
    let card = AccountId::new();
    create_role_account(&worker, checking, "Checking", CashflowRole::LiquidCash);
    create_role_account(&worker, card, "Venture X", CashflowRole::CreditFacility);
    let mut terms = sample_debt_terms(Some(checking));
    terms.statement_close_day = Some(22);
    terms.payment_due_day = Some(17);
    terms.apr_bps = Some(0); // keep the carry arithmetic exact
    worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: card,
                terms,
            },
        )
        .unwrap();
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            card,
            Money::new(-1_708_223, Currency::Usd),
            NaiveDate::from_ymd_opt(2026, 7, 8).unwrap(),
        )
        .unwrap();
    // The real Jun-22 statement, recorded through the guarded write path (a past close).
    worker
        .dispatch(
            meta(),
            WriteCommand::SetCardStatementBalance {
                account_id: card,
                cycle_close: NaiveDate::from_ymd_opt(2026, 6, 22).unwrap(),
                statement_balance_minor: Some(1_387_308),
            },
        )
        .unwrap();
    // The stale row keyed to the NEXT close — legacy data recorded before the closed-cycles-only
    // write guard existed, seeded directly since the guard now (correctly) rejects it.
    let conn = worker.read_connection().unwrap();
    conn.execute(
        "INSERT INTO credit_card_statements
            (id, account_id, cycle_close, statement_balance_minor, minimum_due_minor,
             payment_due, currency, created_at)
         VALUES (?1, ?2, '2026-07-22', 1387308, NULL, '', 'USD', '2026-07-03T00:00:00Z')",
        params![Uuid::now_v7(), card.as_uuid()],
    )
    .unwrap();

    let as_of = NaiveDate::from_ymd_opt(2026, 7, 9)
        .unwrap()
        .and_hms_opt(12, 0, 0)
        .unwrap()
        .and_utc();
    let forecast = worker.card_statement_forecast_at(as_of).unwrap();
    let card_fc = &forecast[0];

    // Cycle 0 = the just-closed Jun-22 statement, due Jul 17: the recorded actual.
    let first = &card_fc.cycles[0];
    assert_eq!(
        first.close_date,
        NaiveDate::from_ymd_opt(2026, 6, 22).unwrap()
    );
    assert_eq!(
        first.due_date,
        NaiveDate::from_ymd_opt(2026, 7, 17).unwrap()
    );
    assert!(first.statement_is_actual);
    assert_eq!(first.statement_balance_minor, 1_387_308);
    assert_eq!(first.forecast_payment_minor, 1_387_308);

    // Cycle 1 (close Jul 22, due Aug 17): the stale row must NOT replay — and the cycle
    // opens from the 3,209.15 of post-close charges the override didn't bill.
    let second = &card_fc.cycles[1];
    assert_eq!(
        second.close_date,
        NaiveDate::from_ymd_opt(2026, 7, 22).unwrap()
    );
    assert_eq!(
        second.due_date,
        NaiveDate::from_ymd_opt(2026, 8, 17).unwrap()
    );
    assert!(
        !second.statement_is_actual,
        "a row on a not-yet-closed cycle must not badge as actual"
    );
    assert_ne!(
        second.statement_balance_minor, 1_387_308,
        "the stale future-keyed override must not replay the old statement"
    );
    assert_eq!(
        second.carried_opening_balance_minor, 320_915,
        "post-close charges (owed - statement) open the next cycle"
    );
    assert_eq!(second.statement_balance_minor, 320_915);

    // Every stored row is visible for management, newest first, with the applied flag
    // mirroring the projection gate (the future-keyed stale row is marked inert).
    assert_eq!(
        card_fc.stored_statements,
        vec![
            StoredStatementView {
                close_date: NaiveDate::from_ymd_opt(2026, 7, 22).unwrap(),
                statement_balance_minor: 1_387_308,
                applied: false,
            },
            StoredStatementView {
                close_date: NaiveDate::from_ymd_opt(2026, 6, 22).unwrap(),
                statement_balance_minor: 1_387_308,
                applied: true,
            },
        ]
    );
    // The is_closed flags mirror the same household-day gate.
    assert!(first.is_closed);
    assert!(!second.is_closed);
}

/// ADR 0039 addendum 2026-07-10 §1: an actual statement can only be recorded for a cycle
/// that has already closed; clearing is always allowed.
#[test]
fn statement_balance_writes_reject_an_unclosed_cycle() {
    let (_dir, worker) = worker();
    let card = AccountId::new();
    create_role_account(&worker, card, "Visa", CashflowRole::CreditFacility);
    let future = Utc::now().date_naive() + chrono::Days::new(40);
    let err = worker.dispatch(
        meta(),
        WriteCommand::SetCardStatementBalance {
            account_id: card,
            cycle_close: future,
            statement_balance_minor: Some(10_000),
        },
    );
    assert!(
        err.is_err(),
        "recording against an unclosed cycle must fail"
    );
    // Clearing an (even future-keyed) row is always allowed — that's the repair path.
    worker
        .dispatch(
            meta(),
            WriteCommand::SetCardStatementBalance {
                account_id: card,
                cycle_close: future,
                statement_balance_minor: None,
            },
        )
        .unwrap();
    // A past close records fine.
    worker
        .dispatch(
            meta(),
            WriteCommand::SetCardStatementBalance {
                account_id: card,
                cycle_close: NaiveDate::from_ymd_opt(2026, 6, 22).unwrap(),
                statement_balance_minor: Some(10_000),
            },
        )
        .unwrap();
}

/// ADR 0039 addendum 2026-07-10 §3 (personal-cfo-4d8.23.10): a cycle-less card WITH a charged
/// bill folds the bill into due-day pseudo-cycles — the bill stops emitting a charge-date
/// liquid outflow, the first payment covers the owed balance plus the folded bill, and every
/// later payment is exactly the bill: the money is modeled once.
#[test]
fn cycle_less_card_with_charged_bills_counts_the_money_once() {
    use chrono::{Datelike, Days};
    let (_dir, worker) = worker();
    let checking = AccountId::new();
    let card = AccountId::new();
    create_role_account(&worker, checking, "Checking", CashflowRole::LiquidCash);
    create_role_account(&worker, card, "Quicksilver", CashflowRole::CreditFacility);
    let today = Utc::now().date_naive();
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            checking,
            Money::new(1_000_000, Currency::Usd),
            today - Days::new(1),
        )
        .unwrap();
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            card,
            Money::new(-50_000, Currency::Usd),
            today - Days::new(1),
        )
        .unwrap();
    // Due day ~12 days out, NO close day: the cycle-less naive shape.
    let due_day = i64::from((today + Days::new(12)).day().min(28));
    let mut terms = sample_debt_terms(Some(checking));
    terms.statement_close_day = None;
    terms.payment_due_day = Some(due_day);
    terms.apr_bps = None;
    worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: card,
                terms,
            },
        )
        .unwrap();
    // A $90 monthly bill charged to the card, first occurrence in 5 days (before the due day).
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateRecurringBill {
                event_id: RecurringEventId::new(),
                contract_id: BillContractId::new(),
                name: "Cox Internet".to_owned(),
                amount: Money::new(9_000, Currency::Usd),
                bill_type: "utility".to_owned(),
                frequency: Frequency::Monthly,
                anchor: today + Days::new(5),
                autopay_account_id: Some(card),
                description: None,
                source_merchant_key: None,
                category_id: None,
                tag_ids: Vec::new(),
            },
        )
        .unwrap();

    let view = worker.future_cash_forecast(60, &[]).unwrap();
    let events: Vec<_> = view.days.iter().flat_map(|d| d.events.iter()).collect();
    assert!(
        !events.iter().any(|e| e.kind == "recurring_bill"),
        "the charged bill must not emit its own charge-date liquid outflow"
    );
    let payments: Vec<i64> = events
        .iter()
        .filter(|e| e.kind == "loan_payment")
        .map(|e| e.amount.minor_units())
        .collect();
    assert!(
        payments.len() >= 2,
        "at least the first two pseudo-cycle payments land inside 60 days: {payments:?}"
    );
    assert_eq!(
        payments[0], -59_000,
        "the first payment covers the owed balance plus the folded bill"
    );
    assert!(
        payments[1..].iter().all(|&p| p == -9_000),
        "each later payment is exactly the folded monthly bill: {payments:?}"
    );
}

/// Adversarial review of 4d8.23.10: a pay_minimum card with NEITHER minimum term configured
/// must not project perpetual $0 payments while its charged bills are suppressed — the
/// ADR 0035 §5 default rule (1% / $25) applies on the cycle path, exactly like the naive
/// loan path, so the suppressed bill's money is always carried by a real payment.
#[test]
fn pseudo_path_pay_minimum_without_terms_defaults_to_the_minimum_rule() {
    use chrono::{Datelike, Days};
    let (_dir, worker) = worker();
    let checking = AccountId::new();
    let card = AccountId::new();
    create_role_account(&worker, checking, "Checking", CashflowRole::LiquidCash);
    create_role_account(&worker, card, "Slate", CashflowRole::CreditFacility);
    let today = Utc::now().date_naive();
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            checking,
            Money::new(1_000_000, Currency::Usd),
            today - Days::new(1),
        )
        .unwrap();
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            card,
            Money::new(-50_000, Currency::Usd),
            today - Days::new(1),
        )
        .unwrap();
    let due_day = i64::from((today + Days::new(12)).day().min(28));
    let mut terms = sample_debt_terms(Some(checking));
    terms.statement_close_day = None;
    terms.payment_due_day = Some(due_day);
    terms.apr_bps = None;
    terms.repayment_philosophy = RepaymentPhilosophy::PayMinimum;
    terms.min_payment_percent_bps = None;
    terms.min_payment_floor_minor = None;
    worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: card,
                terms,
            },
        )
        .unwrap();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateRecurringBill {
                event_id: RecurringEventId::new(),
                contract_id: BillContractId::new(),
                name: "Streaming".to_owned(),
                amount: Money::new(9_000, Currency::Usd),
                bill_type: "subscription".to_owned(),
                frequency: Frequency::Monthly,
                anchor: today + Days::new(5),
                autopay_account_id: Some(card),
                description: None,
                source_merchant_key: None,
                category_id: None,
                tag_ids: Vec::new(),
            },
        )
        .unwrap();

    let view = worker.future_cash_forecast(60, &[]).unwrap();
    let events: Vec<_> = view.days.iter().flat_map(|d| d.events.iter()).collect();
    assert!(
        !events.iter().any(|e| e.kind == "recurring_bill"),
        "the charged bill is suppressed (carried by the payment)"
    );
    let payments: Vec<i64> = events
        .iter()
        .filter(|e| e.kind == "loan_payment")
        .map(|e| e.amount.minor_units())
        .collect();
    assert!(
        !payments.is_empty(),
        "a minimum-paying card with unset terms must still project payments"
    );
    assert!(
        payments.iter().all(|&p| p <= -2_500),
        "every payment is at least the ADR 0035 s5 default $25 floor: {payments:?}"
    );
}

/// ADR 0039 addendum 2026-07-10 §2 T1 (personal-cfo-4d8.25.5): with imported card history —
/// uncategorized included — future statements track the median per-window charges instead of
/// collapsing to bills-only. Payments (positive postings) are sign-excluded, and windows not
/// fully covered by history are ignored.
#[test]
fn estimator_t1_projects_future_statements_from_uncategorized_card_history() {
    let (_dir, worker) = worker();
    let checking = AccountId::new();
    let card = AccountId::new();
    create_role_account(&worker, checking, "Checking", CashflowRole::LiquidCash);
    create_role_account(&worker, card, "Venture X", CashflowRole::CreditFacility);
    let mut terms = sample_debt_terms(Some(checking));
    terms.statement_close_day = Some(22);
    terms.payment_due_day = Some(17);
    terms.apr_bps = Some(0);
    worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: card,
                terms,
            },
        )
        .unwrap();
    let d = |y, m, day| NaiveDate::from_ymd_opt(y, m, day).unwrap();
    let plant = |cents: i64, date: NaiveDate| {
        worker
            .dispatch(
                meta(),
                WriteCommand::RecordTransaction {
                    transaction_id: TransactionId::new(),
                    account_id: card,
                    amount: Money::new(cents, Currency::Usd),
                    occurred_at: date.and_hms_opt(12, 0, 0).unwrap().and_utc(),
                },
            )
            .unwrap();
    };
    // Earliest posting Feb 25 → the (Feb 22, Mar 22) window is NOT fully covered and must
    // be excluded; the three later windows are complete: totals 30_000 / 50_000 / 40_000.
    plant(-1_000, d(2026, 2, 25));
    plant(-30_000, d(2026, 4, 1)); // (Mar 22, Apr 22)
    plant(-50_000, d(2026, 5, 1)); // (Apr 22, May 22)
    plant(-40_000, d(2026, 6, 1)); // (May 22, Jun 22)
                                   // A PAYMENT — sign-excluded from charges; also extends the coverage span past the most
                                   // recent close so the (May 22, Jun 22) window counts as fully covered.
    plant(60_000, d(2026, 6, 25));

    let as_of = d(2026, 7, 9).and_hms_opt(12, 0, 0).unwrap().and_utc();
    let forecast = worker.card_statement_forecast_at(as_of).unwrap();
    let card_fc = &forecast[0];
    assert_eq!(card_fc.estimate_basis, "card_history");
    assert_eq!(card_fc.estimate_sample_cycles, 3);
    // The first FULL future cycle (close Aug 22) carries the whole median estimate:
    // median(40_000, 50_000, 30_000) = 40_000 — not zero, not a bills-only figure.
    let full_cycle = &card_fc.cycles[2];
    assert_eq!(full_cycle.close_date, d(2026, 8, 22));
    assert_eq!(full_cycle.projected_variable_minor, 40_000);
    assert_eq!(full_cycle.statement_balance_minor, 40_000);
}

/// ADR 0039 addendum 2026-07-10 §2 T2 (personal-cfo-4d8.25.5): with NO transaction detail at
/// all, recorded statement history alone drives the projection (full-payment cards).
#[test]
fn estimator_t2_projects_from_recorded_statements_without_any_transactions() {
    let (_dir, worker) = worker();
    let checking = AccountId::new();
    let card = AccountId::new();
    create_role_account(&worker, checking, "Checking", CashflowRole::LiquidCash);
    create_role_account(&worker, card, "Sapphire", CashflowRole::CreditFacility);
    let mut terms = sample_debt_terms(Some(checking));
    terms.statement_close_day = Some(22);
    terms.payment_due_day = Some(17);
    terms.apr_bps = Some(0);
    worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: card,
                terms,
            },
        )
        .unwrap();
    let d = |y, m, day| NaiveDate::from_ymd_opt(y, m, day).unwrap();
    for (close, minor) in [(d(2026, 4, 22), 40_000i64), (d(2026, 5, 22), 50_000)] {
        worker
            .dispatch(
                meta(),
                WriteCommand::SetCardStatementBalance {
                    account_id: card,
                    cycle_close: close,
                    statement_balance_minor: Some(minor),
                },
            )
            .unwrap();
    }

    let as_of = d(2026, 7, 9).and_hms_opt(12, 0, 0).unwrap().and_utc();
    let forecast = worker.card_statement_forecast_at(as_of).unwrap();
    let card_fc = &forecast[0];
    assert_eq!(card_fc.estimate_basis, "statement_history");
    assert_eq!(card_fc.estimate_sample_cycles, 2);
    // First full future cycle: the median recorded statement (45_000), no bills to subtract.
    let full_cycle = &card_fc.cycles[2];
    assert_eq!(full_cycle.projected_variable_minor, 45_000);
    assert_eq!(full_cycle.statement_balance_minor, 45_000);
}

/// The statement-history capture surface (personal-cfo-4d8.25.4): past windows list derived
/// import totals where covered, recorded statements where present, and recording a past
/// statement feeds the projection + the estimator.
#[test]
fn card_statement_history_lists_derived_and_stored_values() {
    let (_dir, worker) = worker();
    let checking = AccountId::new();
    let card = AccountId::new();
    create_role_account(&worker, checking, "Checking", CashflowRole::LiquidCash);
    create_role_account(&worker, card, "Venture X", CashflowRole::CreditFacility);
    let mut terms = sample_debt_terms(Some(checking));
    terms.statement_close_day = Some(22);
    terms.payment_due_day = Some(17);
    worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: card,
                terms,
            },
        )
        .unwrap();
    let d = |y, m, day| NaiveDate::from_ymd_opt(y, m, day).unwrap();
    // History spans Feb 1 – Jun 21: windows inside that span are fully covered; the
    // (Jan 22, Feb 22) window is only partially covered and must report no derived value.
    for (cents, date) in [(-500i64, d(2026, 2, 1)), (-25_000, d(2026, 6, 21))] {
        worker
            .dispatch(
                meta(),
                WriteCommand::RecordTransaction {
                    transaction_id: TransactionId::new(),
                    account_id: card,
                    amount: Money::new(cents, Currency::Usd),
                    occurred_at: date.and_hms_opt(12, 0, 0).unwrap().and_utc(),
                },
            )
            .unwrap();
    }
    worker
        .dispatch(
            meta(),
            WriteCommand::SetCardStatementBalance {
                account_id: card,
                cycle_close: d(2026, 5, 22),
                statement_balance_minor: Some(31_500),
            },
        )
        .unwrap();

    let as_of = d(2026, 7, 9).and_hms_opt(12, 0, 0).unwrap().and_utc();
    let history = worker.card_statement_history_at(card, as_of).unwrap();
    assert!(!history.is_empty());
    // Newest first: the (May 22, Jun 22) window holds the planted charge as a derived total.
    let jun = history
        .iter()
        .find(|w| w.close_date == d(2026, 6, 22))
        .expect("June window present");
    assert_eq!(jun.window_open, d(2026, 5, 22));
    assert_eq!(jun.derived_charges_minor, Some(25_000));
    assert_eq!(jun.stored_statement_minor, None);
    // The recorded May statement shows on its window.
    let may = history
        .iter()
        .find(|w| w.close_date == d(2026, 5, 22))
        .expect("May window present");
    assert_eq!(may.stored_statement_minor, Some(31_500));
    // A window only partially covered by history has no derived value (its open predates
    // the earliest posting).
    let feb = history
        .iter()
        .find(|w| w.close_date == d(2026, 2, 22))
        .expect("Feb window present");
    assert_eq!(feb.derived_charges_minor, None);
    // An account with no debt terms yields an empty history, not an error.
    assert!(worker
        .card_statement_history_at(checking, as_of)
        .unwrap()
        .is_empty());
}

/// The owner-scenario walk-forward backtest (personal-cfo-4d8.25.6): 12 months of synthetic
/// varying card history + a charged bill. The next projected statement must track the
/// history (within the recorded 25% tolerance of the true per-window mean), never replay the
/// prior actual statement, and never collapse to the bills-only sum; the estimator's
/// walk-forward MAPE is recorded by the backtest run.
#[test]
fn statement_backtest_owner_scenario_tracks_history_and_records_the_metric() {
    let (_dir, worker) = worker();
    let checking = AccountId::new();
    let card = AccountId::new();
    create_role_account(&worker, checking, "Checking", CashflowRole::LiquidCash);
    create_role_account(&worker, card, "Venture X", CashflowRole::CreditFacility);
    let mut terms = sample_debt_terms(Some(checking));
    terms.statement_close_day = Some(22);
    terms.payment_due_day = Some(17);
    terms.apr_bps = Some(0);
    worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: card,
                terms,
            },
        )
        .unwrap();
    let d = |y, m, day| NaiveDate::from_ymd_opt(y, m, day).unwrap();
    // A $90 monthly bill charged to the card (the deterministic backbone).
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateRecurringBill {
                event_id: RecurringEventId::new(),
                contract_id: BillContractId::new(),
                name: "Youtube TV".to_owned(),
                amount: Money::new(9_000, Currency::Usd),
                bill_type: "subscription".to_owned(),
                frequency: Frequency::Monthly,
                anchor: d(2026, 7, 12),
                autopay_account_id: Some(card),
                description: None,
                source_merchant_key: None,
                category_id: None,
                tag_ids: Vec::new(),
            },
        )
        .unwrap();
    // 12 months of variable charges: one posting mid-window, totals oscillating around
    // $1,500 (120_000..=180_000 minor) in a deterministic pattern.
    let amounts: [i64; 12] = [
        150_000, 165_000, 135_000, 180_000, 120_000, 150_000, 172_500, 142_500, 157_500, 127_500,
        165_000, 150_000,
    ];
    let mut date = d(2025, 7, 25);
    for amount in amounts {
        worker
            .dispatch(
                meta(),
                WriteCommand::RecordTransaction {
                    transaction_id: TransactionId::new(),
                    account_id: card,
                    amount: Money::new(-amount, Currency::Usd),
                    occurred_at: date.and_hms_opt(12, 0, 0).unwrap().and_utc(),
                },
            )
            .unwrap();
        date = date.checked_add_months(chrono::Months::new(1)).unwrap();
    }
    // The real Jun-22 statement, recorded.
    worker
        .dispatch(
            meta(),
            WriteCommand::SetCardStatementBalance {
                account_id: card,
                cycle_close: d(2026, 6, 22),
                statement_balance_minor: Some(160_000),
            },
        )
        .unwrap();

    let as_of = d(2026, 7, 9).and_hms_opt(12, 0, 0).unwrap().and_utc();
    let forecast = worker.card_statement_forecast_at(as_of).unwrap();
    let card_fc = &forecast[0];
    assert_eq!(card_fc.estimate_basis, "card_history");
    // The first FULL future cycle (close Aug 22): bills + estimated variable spend.
    let projected = card_fc.cycles[2].statement_balance_minor;
    let bills_only = 9_000;
    assert_ne!(
        projected, 160_000,
        "must not replay the prior actual statement"
    );
    assert_ne!(
        projected, bills_only,
        "must not collapse to the bills-only sum"
    );
    // Exact contract: the bill was created TODAY, so the dedupe floor keeps it out of the
    // historical windows (it never posted there) — median of the 10 covered raw totals
    // (153_750) plus the known bill (9_000). An unfloored dedupe would cancel the new bill
    // back to 153_750 (adversarial review of 4d8.25.5 — money vanishing).
    assert_eq!(projected, 162_750, "history median + newly-created bill");
    // Tolerance recorded with this baseline (ADR 0039 addendum 2026-07-10 §2): within 25%
    // of the true per-window mean (~150_000 variable + 9_000 bill).
    let expected = 159_000i64;
    let err_bps = (projected - expected).abs() * 10_000 / expected;
    assert!(
        err_bps <= 2_500,
        "projected {projected} vs expected {expected}: {err_bps} bps > 2500"
    );
    // The walk-forward metric is recorded by the backtest run.
    worker.backtest_forecasts().unwrap();
    let conn = worker.read_connection().unwrap();
    let (score, sample): (i64, i64) = conn
        .query_row(
            "SELECT score_bps, sample_size FROM forecast_backtest_results
             WHERE model_id = 'card_statement_estimator_v1' AND metric_type = 'mape'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(sample >= 3, "walk-forward pairs recorded: {sample}");
    // The fixture oscillates ±20% around a stable median, so a correct walk-forward error
    // stays well under 50% — a degenerate metric (e.g. an inverted window span reading zero
    // realized charges) scores ~100% and must fail here.
    assert!(
        (0..5_000).contains(&score),
        "walk-forward MAPE {score} bps out of the sane range for the fixture"
    );
}
