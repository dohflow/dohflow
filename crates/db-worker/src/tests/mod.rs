//! Unit tests that reach crate internals (`open_keyed`, fault injection,
//! `dispatch_inner`, `crate::forecast` / `crate::ingestion` internals, and
//! private `DbWorker` methods). Pub-API-only tests live in `tests/*.rs`;
//! shared helpers for those live in `tests/common/mod.rs`.

use super::*;
use core_ledger::{AccountFlags, CashflowRole};
use core_money::Currency;
use tempfile::TempDir;

const KEY: &str = "correct horse battery staple";

fn worker() -> (TempDir, DbWorker) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.vault");
    let worker = DbWorker::open(&path, KEY).unwrap();
    (dir, worker)
}

fn meta() -> CommandMeta {
    meta_with_key(&Uuid::now_v7().to_string())
}

fn meta_with_key(idempotency_key: &str) -> CommandMeta {
    CommandMeta {
        command_id: Uuid::now_v7(),
        correlation_id: Uuid::now_v7(),
        causation_id: None,
        actor_type: ActorType::User,
        actor_id: "tester".to_owned(),
        idempotency_key: idempotency_key.to_owned(),
    }
}

fn sample_account() -> Account {
    Account::new(
        AccountId::new(),
        LedgerAccountId::new(),
        "Checking",
        CashflowRole::LiquidCash,
        Currency::Usd,
        AccountFlags::default(),
    )
}

fn create_account_cmd() -> WriteCommand {
    WriteCommand::CreateAccount {
        account: Box::new(sample_account()),
        opening_balance: None,
    }
}

fn income_cmd(deposit: Option<AccountId>) -> WriteCommand {
    WriteCommand::CreateIncomeSource {
        id: IncomeSourceId::new(),
        name: "Acme Corp".to_owned(),
        net_amount: Money::new(300_000, Currency::Usd),
        frequency: Frequency::Biweekly,
        anchor: NaiveDate::from_ymd_opt(2026, 6, 5).unwrap(),
        deposit_account_id: deposit,
    }
}

/// A seeded `variable_regular` category id (the kind Layer-2 models).
fn variable_category(worker: &DbWorker) -> CategoryId {
    let conn = worker.read_connection().unwrap();
    let id: Uuid = conn
        .query_row(
            "SELECT id FROM categories WHERE forecast_behavior = 'variable_regular' LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    CategoryId::from_uuid(id)
}

/// Record a dated spend transaction and assign it `category`. Callers plant in
/// increasing date order; the transaction is then located by its (unique) date.
fn plant_categorized(
    worker: &DbWorker,
    account_id: AccountId,
    cents: i64,
    date: NaiveDate,
    category: CategoryId,
) {
    let occurred = date.and_hms_opt(12, 0, 0).unwrap().and_utc();
    worker
        .dispatch(
            meta(),
            WriteCommand::RecordTransaction {
                transaction_id: TransactionId::new(),
                account_id,
                amount: Money::new(cents, Currency::Usd),
                occurred_at: occurred,
            },
        )
        .unwrap();
    let txns = worker.recent_transactions(500).unwrap();
    let txn_id = txns
        .iter()
        .find(|t| t.occurred_at.date_naive() == date)
        .expect("just-planted txn")
        .transaction_id;
    worker
        .dispatch(
            meta(),
            WriteCommand::RecategorizeTransaction {
                transaction_id: txn_id,
                category_id: Some(category),
            },
        )
        .unwrap();
}

fn checking(account_id: AccountId) -> Account {
    Account::new(
        account_id,
        LedgerAccountId::new(),
        "Checking",
        CashflowRole::LiquidCash,
        Currency::Usd,
        AccountFlags::default(),
    )
}

fn loan_terms(
    philosophy: RepaymentPhilosophy,
    fixed: Option<i64>,
    paying: AccountId,
) -> DebtTermsInput {
    DebtTermsInput {
        apr_bps: None,
        statement_close_day: None,
        payment_due_day: Some(15),
        grace_period_days: None,
        credit_limit_minor: None,
        repayment_philosophy: philosophy,
        fixed_amount_minor: fixed,
        min_payment_percent_bps: None,
        min_payment_floor_minor: None,
        paying_source_account_id: Some(paying),
        original_principal_minor: None,
    }
}

fn bill_cmd(autopay: Option<AccountId>) -> WriteCommand {
    WriteCommand::CreateRecurringBill {
        event_id: RecurringEventId::new(),
        contract_id: BillContractId::new(),
        name: "Rent".to_owned(),
        amount: Money::new(180_000, Currency::Usd),
        bill_type: "rent_mortgage".to_owned(),
        frequency: Frequency::Monthly,
        anchor: NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
        autopay_account_id: autopay,
        description: None,
        source_merchant_key: None,
        category_id: None,
        tag_ids: Vec::new(),
    }
}

// ===== Type-based cash-tier rollups (ADR 0028, personal-cfo-9dgg) =====

fn liquid_subtype_cmd(
    id: AccountId,
    name: &str,
    subtype: Option<AccountSubtype>,
    opening_minor: i64,
) -> WriteCommand {
    WriteCommand::CreateAccount {
        account: Box::new(
            Account::new(
                id,
                LedgerAccountId::new(),
                name,
                CashflowRole::LiquidCash,
                Currency::Usd,
                AccountFlags::default(),
            )
            .with_subtype(subtype),
        ),
        opening_balance: Some(Money::new(opening_minor, Currency::Usd)),
    }
}

/// Seed a deterministic "golden" vault (personal-cfo-rdg9 / -tv3w): a checking
/// ($5,000) and savings ($10,000) account, each with a balance asserted at the
/// golden as_of (so balances are exact and fresh), a $3,000 biweekly paycheck
/// (anchor 2026-06-05) into checking, and an $1,800 monthly rent (anchor
/// 2026-07-01) autopaid from checking. Accounts open at 0 so the asserted
/// balance is the whole balance (no real-clock opening posting confounds it).
fn seed_golden_vault(worker: &DbWorker) -> (AccountId, AccountId) {
    let as_of_date = NaiveDate::from_ymd_opt(2026, 6, 20).unwrap();
    let checking = AccountId::new();
    let savings = AccountId::new();
    worker
        .dispatch(
            meta(),
            liquid_subtype_cmd(checking, "Checking", Some(AccountSubtype::Checking), 0),
        )
        .unwrap();
    worker
        .dispatch(
            meta(),
            liquid_subtype_cmd(savings, "Savings", Some(AccountSubtype::Savings), 0),
        )
        .unwrap();
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            checking,
            Money::new(500_000, Currency::Usd),
            as_of_date,
        )
        .unwrap();
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            savings,
            Money::new(1_000_000, Currency::Usd),
            as_of_date,
        )
        .unwrap();
    worker.dispatch(meta(), income_cmd(Some(checking))).unwrap();
    worker.dispatch(meta(), bill_cmd(Some(checking))).unwrap();
    (checking, savings)
}

/// The golden as_of: noon UTC on 2026-06-20 (a fixed clock for stable values).
fn golden_as_of() -> DateTime<Utc> {
    use chrono::TimeZone;
    Utc.with_ymd_and_hms(2026, 6, 20, 12, 0, 0).unwrap()
}

fn liquid_account_no_opening(id: AccountId) -> WriteCommand {
    WriteCommand::CreateAccount {
        account: Box::new(Account::new(
            id,
            LedgerAccountId::new(),
            "Checking",
            CashflowRole::LiquidCash,
            Currency::Usd,
            AccountFlags::default(),
        )),
        opening_balance: None,
    }
}

fn readiness_factor<'a>(r: &'a ForecastReadiness, key: &str) -> &'a ReadinessFactor {
    r.factors.iter().find(|f| f.key == key).unwrap()
}

fn liquid_account_cmd(id: AccountId, currency: Currency, opening_minor: i64) -> WriteCommand {
    WriteCommand::CreateAccount {
        account: Box::new(Account::new(
            id,
            LedgerAccountId::new(),
            "Checking",
            CashflowRole::LiquidCash,
            currency,
            AccountFlags::default(),
        )),
        opening_balance: Some(Money::new(opening_minor, currency)),
    }
}

/// Count persisted forecast runs (the daily-on-open activation, ADR 0026 §15).
fn forecast_run_count(worker: &DbWorker) -> u64 {
    worker.persisted_forecast_run_count().unwrap()
}

/// Actualize with a controlled `today` (the deterministic seam): refresh
/// instances then score. Returns `(predicted_date, match_status, realized_minor,
/// has_txn)` per forecast_actuals row, ordered by date.
fn actualize_and_read(worker: &DbWorker, today: NaiveDate) -> Vec<(String, String, i64, bool)> {
    let mut conn = worker.read_connection().unwrap();
    recurring_instances::rebuild(&mut conn, today).unwrap();
    forecast_actualize::actualize(&conn, today).unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT fr.date, fa.match_status, fa.realized_amount_minor,
                        fa.matched_transaction_id IS NOT NULL
                 FROM forecast_actuals fa
                 JOIN forecast_rows fr ON fr.id = fa.forecast_row_id
                 ORDER BY fr.date, fa.match_status",
        )
        .unwrap();
    let out = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)? != 0,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    out
}

/// Seed a liquid account + biweekly income (Acme, +3,000 from 2026-06-05) and
/// persist a forecast run as of `as_of`. Returns the account id.
fn seed_income_and_persist(worker: &DbWorker, as_of: DateTime<Utc>) -> AccountId {
    let account_id = AccountId::new();
    worker
        .dispatch(meta(), liquid_account_cmd(account_id, Currency::Usd, 0))
        .unwrap();
    worker
        .dispatch(meta(), income_cmd(Some(account_id)))
        .unwrap();
    worker.persist_daily_forecast_at(as_of, 365).unwrap();
    account_id
}

fn deposit_cmd(account_id: AccountId, minor: i64, on: NaiveDate) -> WriteCommand {
    WriteCommand::RecordTransaction {
        transaction_id: TransactionId::new(),
        account_id,
        amount: Money::new(minor, Currency::Usd),
        occurred_at: on.and_hms_opt(0, 0, 0).unwrap().and_utc(),
    }
}

/// Refresh instances, actualize, then backtest at a controlled `today`. Returns the
/// number of `forecast_backtest_results` rows written (0 or 1).
fn actualize_and_backtest(worker: &DbWorker, today: NaiveDate) -> u64 {
    let mut conn = worker.read_connection().unwrap();
    recurring_instances::rebuild(&mut conn, today).unwrap();
    forecast_actualize::actualize(&conn, today).unwrap();
    forecast_backtest::run_backtest(&conn, today, 365).unwrap()
}

fn create_role_account(worker: &DbWorker, id: AccountId, name: &str, role: CashflowRole) {
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
                opening_balance: None,
            },
        )
        .unwrap();
}

fn sample_debt_terms(paying: Option<AccountId>) -> DebtTermsInput {
    DebtTermsInput {
        apr_bps: Some(2199),
        statement_close_day: Some(5),
        payment_due_day: Some(25),
        grace_period_days: Some(21),
        credit_limit_minor: Some(500_000),
        repayment_philosophy: RepaymentPhilosophy::PayStatementBalance,
        fixed_amount_minor: None,
        min_payment_percent_bps: Some(100),
        min_payment_floor_minor: Some(2500),
        paying_source_account_id: paying,
        original_principal_minor: None,
    }
}

/// Stage an account-matched transaction ready to commit. PR-B adds the
/// bulk-stage command; here we stage directly so `CommitStaged` is testable
/// in isolation.
fn stage_a_transaction(
    worker: &DbWorker,
    account_id: AccountId,
    amount_minor: i64,
    fingerprint: &str,
) -> StagedTransactionId {
    stage_a_transaction_dated(worker, account_id, amount_minor, fingerprint, None)
}

/// As [`stage_a_transaction`], but with an explicit secondary transaction date
/// (ADR 0045) so the dual-date surfacing can be exercised end-to-end.
fn stage_a_transaction_dated(
    worker: &DbWorker,
    account_id: AccountId,
    amount_minor: i64,
    fingerprint: &str,
    transaction_date: Option<&str>,
) -> StagedTransactionId {
    let batch_id = SourceBatchId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateSourceBatch {
                id: batch_id,
                source_type: "csv".to_owned(),
                source_name: None,
                file_fingerprint: None,
                parser_version: None,
            },
        )
        .unwrap();
    let record_id = SourceRecordId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::AttachSourceRecord {
                id: record_id,
                batch_id,
                external_id: None,
                source_hash: format!("hash-{fingerprint}"),
                normalized_json: "{}".to_owned(),
                parse_confidence_bps: None,
            },
        )
        .unwrap();
    let conn = worker.read_connection().unwrap();
    let staged = crate::ingestion::stage_transaction(
        &conn,
        &crate::ingestion::NewStagedTransaction {
            source_record_id: record_id.as_uuid(),
            proposed_account_id: Some(account_id.as_uuid()),
            posted_at: "2026-06-20",
            transaction_date,
            amount_minor,
            currency: "USD",
            normalized_merchant: Some("merchant"),
            description: None,
            imported_category: None,
            txn_fingerprint: fingerprint,
        },
    )
    .unwrap();
    StagedTransactionId::from_uuid(staged)
}

// ---- migration framework (personal-cfo-wkn) ----

fn user_version(conn: &Connection) -> i64 {
    conn.pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap()
}

/// 9h1s pipeline: with enough variable-spend history the forecast band activates —
/// personal-cfo-4d8.27.6.2: a scenario that plans to spend LESS in a category raises the
/// projected balance, and does so by adjusting the modelled draw — not by inventing a
/// deterministic event, which would double-count spend the band already carries.
#[test]
fn a_category_spend_override_raises_the_projected_balance() {
    let (_dir, worker) = worker();
    let account_id = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(account_id)),
                opening_balance: Some(Money::new(5_000_000, Currency::Usd)),
            },
        )
        .unwrap();
    let category = variable_category(&worker);

    let as_of = Utc::now();
    let today = as_of.date_naive();
    for m in (1..=24u32).rev() {
        let date = today.checked_sub_months(chrono::Months::new(m)).unwrap();
        let cents = -(50_000 + i64::from(m <= 12) * 10_000);
        plant_categorized(&worker, account_id, cents, date, category);
    }

    let conn = || worker.read_connection().unwrap();
    let ending = |scenario: Option<Uuid>| {
        let sel: Vec<Uuid> = scenario.into_iter().collect();
        crate::forecast::compute(&conn(), as_of, 90, &sel)
            .unwrap()
            .days
            .last()
            .expect("a 90-day horizon")
            .closing
            .p50
            .minor_units()
    };
    let base_end = ending(None);

    let scenario = Uuid::now_v7();
    worker
        .create_scenario(&NewScenario {
            id: scenario,
            name: "Spend less".to_owned(),
            description: None,
            base_run_id: None,
        })
        .unwrap();
    worker
        .record_forecast_assumption(&ForecastAssumptionSpec {
            id: Uuid::now_v7(),
            scenario_id: Some(scenario),
            target_entity_id: Some(category.as_uuid()),
            params: AssumptionParams::VariableSpendOverride {
                delta_minor_per_month: -20_000, // spend $200/mo less
                effective_date: None,
                end_date: None,
            },
        })
        .unwrap();

    let scenario_end = ending(scenario.into());
    assert!(
        scenario_end > base_end,
        "planning to spend less must project MORE cash: {scenario_end} vs {base_end}",
    );
    // …and the base forecast is untouched — the overlay is scenario-scoped.
    assert_eq!(ending(None), base_end, "base must not move");

    // No fabricated events: the change comes from the modelled draw, so the scenario's
    // event stream is identical to base (this is what prevents double-counting).
    let events_of = |scenario: Option<Uuid>| {
        let sel: Vec<Uuid> = scenario.into_iter().collect();
        crate::forecast::compute(&conn(), as_of, 90, &sel)
            .unwrap()
            .days
            .iter()
            .flat_map(|d| &d.events)
            .count()
    };
    assert_eq!(
        events_of(scenario.into()),
        events_of(None),
        "a spend override must not add projected events",
    );
}

/// personal-cfo-4d8.27.6.2: a household-level spend cut is APPORTIONED across the
/// accounts that actually spend in that category, so the per-account views and the
/// aggregate agree about what one plan does. Without apportionment each account would
/// apply the whole cut and the per-account total would overstate the saving.
#[test]
fn a_category_spend_override_is_apportioned_across_accounts() {
    let (_dir, worker) = worker();
    let a = AccountId::new();
    let b = AccountId::new();
    for id in [a, b] {
        worker
            .dispatch(
                meta(),
                WriteCommand::CreateAccount {
                    account: Box::new(checking(id)),
                    opening_balance: Some(Money::new(5_000_000, Currency::Usd)),
                },
            )
            .unwrap();
    }
    let category = variable_category(&worker);
    let as_of = Utc::now();
    let today = as_of.date_naive();
    // BOTH accounts spend heavily in the category, so an un-apportioned cut would be
    // applied twice over.
    for m in (1..=24u32).rev() {
        let date = today.checked_sub_months(chrono::Months::new(m)).unwrap();
        let cents = -(50_000 + i64::from(m <= 12) * 10_000);
        plant_categorized(&worker, a, cents, date, category);
        plant_categorized(
            &worker,
            b,
            cents,
            date.checked_add_days(chrono::Days::new(1)).unwrap(),
            category,
        );
    }

    let conn = || worker.read_connection().unwrap();
    let per_account_total = |scenario: Option<Uuid>| -> i64 {
        let sel: Vec<Uuid> = scenario.into_iter().collect();
        crate::forecast::compute_by_account(&conn(), as_of, 90, &sel)
            .unwrap()
            .accounts
            .iter()
            .map(|s| s.days.last().unwrap().closing.p50.minor_units())
            .sum()
    };
    let aggregate = |scenario: Option<Uuid>| -> i64 {
        let sel: Vec<Uuid> = scenario.into_iter().collect();
        crate::forecast::compute(&conn(), as_of, 90, &sel)
            .unwrap()
            .days
            .last()
            .unwrap()
            .closing
            .p50
            .minor_units()
    };

    let scenario = Uuid::now_v7();
    worker
        .create_scenario(&NewScenario {
            id: scenario,
            name: "Spend less".to_owned(),
            description: None,
            base_run_id: None,
        })
        .unwrap();
    worker
        .record_forecast_assumption(&ForecastAssumptionSpec {
            id: Uuid::now_v7(),
            scenario_id: Some(scenario),
            target_entity_id: Some(category.as_uuid()),
            params: AssumptionParams::VariableSpendOverride {
                delta_minor_per_month: -20_000,
                effective_date: None,
                end_date: None,
            },
        })
        .unwrap();

    let per_account_saving = per_account_total(scenario.into()) - per_account_total(None);
    let aggregate_saving = aggregate(scenario.into()) - aggregate(None);
    assert!(
        per_account_saving > 0 && aggregate_saving > 0,
        "both views save"
    );
    // The same plan must not save twice as much just because the spend is split over two
    // accounts. Allow a small rounding tolerance (per-day integer division).
    let drift = (per_account_saving - aggregate_saving).abs();
    assert!(
        drift <= aggregate_saving / 10,
        "per-account saving {per_account_saving} should track the aggregate {aggregate_saving}",
    );
}

/// the deterministic line widens into a real P10/P90 spread (which the chart draws).
#[test]
fn layer2_widens_the_forecast_with_enough_variable_spend_history() {
    let (_dir, worker) = worker();
    let account_id = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(account_id)),
                opening_balance: Some(Money::new(5_000_000, Currency::Usd)),
            },
        )
        .unwrap();
    let category = variable_category(&worker);

    let as_of = Utc::now();
    let today = as_of.date_naive();
    // 24 months of variable spend, oldest first; the year-over-year step ($500 → $600)
    // gives each seasonal bucket the variance that becomes the band's spread.
    for m in (1..=24u32).rev() {
        let date = today.checked_sub_months(chrono::Months::new(m)).unwrap();
        let cents = -(50_000 + i64::from(m <= 12) * 10_000);
        plant_categorized(&worker, account_id, cents, date, category);
    }

    let conn = worker.read_connection().unwrap();
    let view = crate::forecast::compute(&conn, as_of, 90, &[]).unwrap();
    let last = view.days.last().expect("a 90-day horizon");
    assert!(
        last.closing.p10.minor_units() < last.closing.p90.minor_units(),
        "Layer-2 should widen the band, got {:?}",
        last.closing
    );
}

/// Below the history gate, the forecast stays the trustworthy collapsed line.
#[test]
fn layer2_stays_collapsed_without_enough_history() {
    let (_dir, worker) = worker();
    let account_id = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(account_id)),
                opening_balance: Some(Money::new(5_000_000, Currency::Usd)),
            },
        )
        .unwrap();
    let category = variable_category(&worker);

    let today = Utc::now().date_naive();
    for m in (1..=3u32).rev() {
        let date = today.checked_sub_months(chrono::Months::new(m)).unwrap();
        plant_categorized(&worker, account_id, -50_000, date, category);
    }

    let conn = worker.read_connection().unwrap();
    let view = crate::forecast::compute(&conn, Utc::now(), 90, &[]).unwrap();
    let last = view.days.last().expect("a 90-day horizon");
    assert_eq!(
        last.closing.p10.minor_units(),
        last.closing.p90.minor_units(),
        "below the gate the band stays collapsed"
    );
}

/// ADR 0038 (pezm.1): a one-off vacation charge is excluded from the Layer-2 baseline
/// history, while the steady ordinary spend in the same category is kept.
#[test]
fn layer2_history_excludes_extraordinary_one_off_spend() {
    let (_dir, worker) = worker();
    let account_id = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(account_id)),
                opening_balance: Some(Money::new(5_000_000, Currency::Usd)),
            },
        )
        .unwrap();
    let category = variable_category(&worker);

    let today = Utc::now().date_naive();
    // Six months of steady ordinary spend (four postings each month).
    for m in (1..=6u32).rev() {
        let date = today.checked_sub_months(chrono::Months::new(m)).unwrap();
        for cents in [-40_000, -45_000, -38_000, -42_000] {
            plant_categorized(&worker, account_id, cents, date, category);
        }
    }
    // One $3,000 vacation charge in the same category.
    let spike_date = today.checked_sub_months(chrono::Months::new(3)).unwrap();
    plant_categorized(&worker, account_id, -300_000, spike_date, category);

    let conn = worker.read_connection().unwrap();
    let start = today.checked_sub_months(chrono::Months::new(24)).unwrap();
    let history = crate::forecast::read_variable_spend_history(&conn, start, today, None).unwrap();

    assert!(
        history.iter().all(|o| o.amount_cents != 300_000),
        "the extraordinary spike must be excluded from the baseline"
    );
    assert!(
        history.iter().any(|o| o.amount_cents == 40_000),
        "ordinary spend must remain"
    );
    assert_eq!(
        history.len(),
        24,
        "24 ordinary postings kept, the single spike dropped"
    );
}

/// ADR 0039 §3 (6wk.8): a recurring bill paid from a credit card (full-payment
/// philosophy, derivable cycle) leaves liquid cash on the card's payment DUE date, never
/// the charge date.
#[test]
fn card_charged_bill_is_paid_on_the_card_due_date_not_the_charge_date() {
    use chrono::{Datelike, TimeZone};
    let (_dir, worker) = worker();
    let checking_id = AccountId::new();
    let card = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(checking_id)),
                opening_balance: Some(Money::new(100_000, Currency::Usd)),
            },
        )
        .unwrap();
    create_role_account(&worker, card, "Visa", CashflowRole::CreditFacility);
    // Statements close on the 5th, payment due on the 25th, paid in full from checking
    // (sample_debt_terms uses PayStatementBalance — a full-payment philosophy).
    worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: card,
                terms: sample_debt_terms(Some(checking_id)),
            },
        )
        .unwrap();
    // A $10 subscription charged to the card on the 10th of each month.
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

    let as_of = Utc.with_ymd_and_hms(2026, 7, 1, 12, 0, 0).unwrap();
    let conn = worker.read_connection().unwrap();
    let view = crate::forecast::compute(&conn, as_of, 150, &[]).unwrap();

    // Every day the balance drops is a card-payment due date (the 25th), never the 10th.
    let mut prev = view.starting_balance.minor_units();
    let mut drops = 0;
    for day in &view.days {
        let bal = day.closing.p50.minor_units();
        if bal < prev {
            assert_eq!(
                day.date.day(),
                25,
                "a card-charged bill must only hit cash on the due day (25th), got {}",
                day.date
            );
            drops += 1;
        }
        prev = bal;
    }
    assert!(
        drops >= 2,
        "the card-charged subscription should produce payments; got {drops}"
    );
}

/// ADR 0039 §3 gating: a bill charged to a card with NO derivable cycle falls back to a
/// liquid outflow on its charge date — the obligation never silently vanishes.
#[test]
fn card_charged_bill_without_cycle_terms_stays_a_liquid_outflow() {
    use chrono::{Datelike, TimeZone};
    let (_dir, worker) = worker();
    let checking_id = AccountId::new();
    let card = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(checking_id)),
                opening_balance: Some(Money::new(100_000, Currency::Usd)),
            },
        )
        .unwrap();
    create_role_account(&worker, card, "Visa", CashflowRole::CreditFacility);
    // No SetDebtTerms → no derivable cycle → fallback to today's behavior.
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

    let as_of = Utc.with_ymd_and_hms(2026, 7, 1, 12, 0, 0).unwrap();
    let conn = worker.read_connection().unwrap();
    let view = crate::forecast::compute(&conn, as_of, 100, &[]).unwrap();

    let mut prev = view.starting_balance.minor_units();
    let mut drops = 0;
    for day in &view.days {
        let bal = day.closing.p50.minor_units();
        if bal < prev {
            assert_eq!(
                day.date.day(),
                10,
                "fallback: a card bill with no cycle hits cash on the charge date (10th), got {}",
                day.date
            );
            drops += 1;
        }
        prev = bal;
    }
    assert!(
        drops >= 2,
        "the bill must still hit liquid cash (no vanish); got {drops}"
    );
}

/// ADR 0039 §2 / personal-cfo-6wk.10: a card-charged bill on a card with a derivable cycle —
/// even a revolving (pay_minimum) one — has its liquid impact modeled as the card PAYMENT on
/// the due date (the 25th), not a charge-date outflow (the 10th). The per-charge outflow is
/// suppressed; cash still moves (the payment), so nothing silently vanishes.
#[test]
fn card_charged_bill_on_a_cycle_card_becomes_a_card_payment_on_the_due_date() {
    use chrono::{Datelike, TimeZone};
    let (_dir, worker) = worker();
    let checking_id = AccountId::new();
    let card = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(checking_id)),
                opening_balance: Some(Money::new(100_000, Currency::Usd)),
            },
        )
        .unwrap();
    create_role_account(&worker, card, "Visa", CashflowRole::CreditFacility);
    // Derivable cycle (close 5 / due 25), pay_minimum → revolving: the card projects a
    // per-cycle minimum PAYMENT on the due date (6wk.10), superseding per-bill retiming.
    let mut terms = sample_debt_terms(Some(checking_id));
    terms.repayment_philosophy = RepaymentPhilosophy::PayMinimum;
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

    let as_of = Utc.with_ymd_and_hms(2026, 7, 1, 12, 0, 0).unwrap();
    let conn = worker.read_connection().unwrap();
    let view = crate::forecast::compute(&conn, as_of, 100, &[]).unwrap();

    let mut prev = view.starting_balance.minor_units();
    let mut drops = 0;
    for day in &view.days {
        let bal = day.closing.p50.minor_units();
        if bal < prev {
            assert_eq!(
                day.date.day(),
                25,
                "the card payment lands on the due date (25th), not the charge date (10th); got {}",
                day.date
            );
            drops += 1;
        }
        prev = bal;
    }
    assert!(
        drops >= 1,
        "the card payment must still move liquid cash; got {drops}"
    );
}

/// personal-cfo-6wk.10: a bill charged to a full-pay card with a cycle is SUPPRESSED (no
/// charge-date outflow) and paid as the card payment on the due date (25th), from the paying
/// source — with per-account == aggregate reconciliation.
#[test]
fn a_card_charged_bill_is_paid_on_the_due_date_from_the_paying_source() {
    use chrono::{Datelike, TimeZone};
    let (_dir, worker) = worker();
    let checking_id = AccountId::new();
    let card = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(checking_id)),
                opening_balance: Some(Money::new(500_000, Currency::Usd)),
            },
        )
        .unwrap();
    create_role_account(&worker, card, "Visa", CashflowRole::CreditFacility);
    // Full-pay card (sample = pay_statement_balance), cycle close 5 / due 25, paid from checking.
    worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: card,
                terms: sample_debt_terms(Some(checking_id)),
            },
        )
        .unwrap();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateRecurringBill {
                event_id: RecurringEventId::new(),
                contract_id: BillContractId::new(),
                name: "Insurance".to_owned(),
                amount: Money::new(9_000, Currency::Usd),
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

    let as_of = Utc.with_ymd_and_hms(2026, 7, 1, 12, 0, 0).unwrap();
    let conn = worker.read_connection().unwrap();
    let view = crate::forecast::compute(&conn, as_of, 100, &[]).unwrap();
    // No charge-date (10th) outflow; the bill is paid as the card payment on the due date.
    let mut prev = view.starting_balance.minor_units();
    let mut drops = 0;
    for day in &view.days {
        let bal = day.closing.p50.minor_units();
        if bal < prev {
            assert_eq!(
                day.date.day(),
                25,
                "card payment on the due date; got {}",
                day.date
            );
            drops += 1;
        }
        prev = bal;
    }
    assert!(
        drops >= 1,
        "the card payment must move liquid cash; got {drops}"
    );
    // Reconciliation: checking is the only liquid account, so its series equals the aggregate
    // — i.e. the card payment attributes to the paying source, not Unallocated.
    let multi = crate::forecast::compute_by_account(&conn, as_of, 100, &[]).unwrap();
    let checking_series = multi
        .accounts
        .iter()
        .find(|s| s.account_id == Some(checking_id.as_uuid()))
        .expect("checking series");
    assert_eq!(
        checking_series
            .days
            .last()
            .unwrap()
            .closing
            .p50
            .minor_units(),
        view.days.last().unwrap().closing.p50.minor_units(),
        "the card payment attributes to checking (per-account == aggregate)",
    );
}

/// personal-cfo-6wk.10 gate: a bill charged to a credit card WITHOUT a derivable cycle has no
/// card payment to model, so it keeps its charge-date (10th) liquid outflow — cash never
/// silently vanishes.
#[test]
fn a_bill_on_a_card_without_a_cycle_keeps_its_charge_date_outflow() {
    use chrono::{Datelike, TimeZone};
    let (_dir, worker) = worker();
    let checking_id = AccountId::new();
    let card = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(checking_id)),
                opening_balance: Some(Money::new(500_000, Currency::Usd)),
            },
        )
        .unwrap();
    // A credit-card account with NO debt_terms → no derivable cycle.
    create_role_account(&worker, card, "Store card", CashflowRole::CreditFacility);
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateRecurringBill {
                event_id: RecurringEventId::new(),
                contract_id: BillContractId::new(),
                name: "Netflix".to_owned(),
                amount: Money::new(1_500, Currency::Usd),
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

    let as_of = Utc.with_ymd_and_hms(2026, 7, 1, 12, 0, 0).unwrap();
    let conn = worker.read_connection().unwrap();
    let view = crate::forecast::compute(&conn, as_of, 100, &[]).unwrap();
    let mut prev = view.starting_balance.minor_units();
    let mut drops = 0;
    for day in &view.days {
        let bal = day.closing.p50.minor_units();
        if bal < prev {
            assert_eq!(
                day.date.day(),
                10,
                "no cycle → charge-date outflow; got {}",
                day.date
            );
            drops += 1;
        }
        prev = bal;
    }
    assert!(
        drops >= 1,
        "the bill must still move liquid cash; got {drops}"
    );
}

/// personal-cfo-6wk.10 review: a base amount override on a card-charged bill flows through to
/// the card PAYMENT (not the raw amount) — the card path honors overrides like the bill path.
#[test]
fn card_payment_honors_an_override_on_a_card_charged_bill() {
    use chrono::TimeZone;
    let (_dir, worker) = worker();
    let checking_id = AccountId::new();
    let card = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(checking_id)),
                opening_balance: Some(Money::new(500_000, Currency::Usd)),
            },
        )
        .unwrap();
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
    let bill = RecurringEventId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateRecurringBill {
                event_id: bill,
                contract_id: BillContractId::new(),
                name: "Electric".to_owned(),
                amount: Money::new(15_000, Currency::Usd), // $150 base
                bill_type: "utility".to_owned(),
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
    // Base override: the electric bill is $50 across the horizon, not $150.
    worker
        .record_forecast_assumption(&ForecastAssumptionSpec {
            id: Uuid::now_v7(),
            scenario_id: None,
            target_entity_id: Some(bill.as_uuid()),
            params: AssumptionParams::BillAmount {
                new_amount_minor: 5_000,
                effective_date: None,
                end_date: None,
            },
        })
        .unwrap();

    let as_of = Utc.with_ymd_and_hms(2026, 7, 1, 12, 0, 0).unwrap();
    let conn = worker.read_connection().unwrap();
    let view = crate::forecast::compute(&conn, as_of, 100, &[]).unwrap();
    let mut prev = view.starting_balance.minor_units();
    let mut saw = false;
    for day in &view.days {
        let bal = day.closing.p50.minor_units();
        if bal < prev {
            assert_eq!(
                prev - bal,
                5_000,
                "the card payment reflects the $50 override, not the $150 base; got {}",
                prev - bal
            );
            saw = true;
        }
        prev = bal;
    }
    assert!(saw, "a card payment should appear");
}

/// ADR 0035 §4 (llx5 review): a charge that already posted earlier this cycle lives in the
/// card's owed balance and must NOT be re-counted in the current cycle's known charges —
/// the forward-from-today projection prevents that double-count.
#[test]
fn card_statement_forecast_does_not_double_count_this_cycles_elapsed_charges() {
    use chrono::TimeZone;
    let (_dir, worker) = worker();
    let checking_id = AccountId::new();
    let card = AccountId::new();
    create_role_account(&worker, checking_id, "Checking", CashflowRole::LiquidCash);
    create_role_account(&worker, card, "Visa", CashflowRole::CreditFacility);
    // close 5 / due 25.
    worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: card,
                terms: sample_debt_terms(Some(checking_id)),
            },
        )
        .unwrap();
    // A $10 bill charging on the 10th of each month.
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
                anchor: NaiveDate::from_ymd_opt(2026, 1, 10).unwrap(),
                autopay_account_id: Some(card),
                description: None,
                source_merchant_key: None,
                category_id: None,
                tag_ids: Vec::new(),
            },
        )
        .unwrap();
    // The card owes $10 as of the 20th — this cycle's 10th charge already posted.
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            card,
            Money::new(-1_000, Currency::Usd),
            NaiveDate::from_ymd_opt(2026, 7, 20).unwrap(),
        )
        .unwrap();

    // As of the 20th (close 5 / due 25), the July statement has closed (the 5th) but is not yet
    // due (the 25th), so cycle 0 is that just-closed unpaid statement (personal-cfo-4d8.23.1);
    // its 10th charge already elapsed and lives in the owed balance.
    let as_of = Utc.with_ymd_and_hms(2026, 7, 20, 12, 0, 0).unwrap();
    let conn = worker.read_connection().unwrap();
    let forecast = crate::forecast::card_statement_forecast(&conn, as_of).unwrap();
    let card_fc = &forecast[0];
    // The elapsed charge lives in the owed balance (opening), NOT re-counted in the imminent cycle.
    assert_eq!(card_fc.cycles[0].carried_opening_balance_minor, 1_000);
    assert_eq!(
        card_fc.cycles[0].known_charges_minor, 0,
        "the just-closed statement's already-posted charge must not be double-counted"
    );
    // Cycle 1 spans [Jul 5, Aug 5): its Jul 10 charge already elapsed (before the 20th), so it is
    // not re-counted either; the next FUTURE $10 charge (Aug 10) lands in cycle 2.
    assert_eq!(card_fc.cycles[1].known_charges_minor, 0);
    assert_eq!(card_fc.cycles[2].known_charges_minor, 1_000);
}

/// personal-cfo-4d8.23.1: a card whose statement has already closed this month but whose
/// payment is not yet due (grace window: close_day < today <= due_day) must forecast that
/// imminent payment on THIS month's due date, not skip it to next month. Close day 3, due day
/// 17, evaluated 2026-07-06 — the July 17 payment must be projected (was mis-dated Aug 17).
#[test]
fn card_next_due_uses_the_just_closed_unpaid_statement_not_next_month() {
    use chrono::{Datelike, TimeZone};
    let (_dir, worker) = worker();
    let checking_id = AccountId::new();
    let card = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(checking_id)),
                opening_balance: Some(Money::new(500_000, Currency::Usd)),
            },
        )
        .unwrap();
    create_role_account(&worker, card, "Visa", CashflowRole::CreditFacility);
    // Close 3 / due 17 (the reported card), pay the minimum from checking.
    let mut terms = sample_debt_terms(Some(checking_id));
    terms.statement_close_day = Some(3);
    terms.payment_due_day = Some(17);
    terms.repayment_philosophy = RepaymentPhilosophy::PayMinimum;
    worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: card,
                terms,
            },
        )
        .unwrap();
    // The card owes $500 as of the 5th — the July statement (closed the 3rd) is unpaid.
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            card,
            Money::new(-50_000, Currency::Usd),
            NaiveDate::from_ymd_opt(2026, 7, 5).unwrap(),
        )
        .unwrap();

    let as_of = Utc.with_ymd_and_hms(2026, 7, 6, 12, 0, 0).unwrap();
    let conn = worker.read_connection().unwrap();

    // Statement view: the soonest cycle is due 2026-07-17 with a positive projected payment.
    let forecast = crate::forecast::card_statement_forecast(&conn, as_of).unwrap();
    let cycle0 = &forecast[0].cycles[0];
    assert_eq!(
        cycle0.due_date,
        NaiveDate::from_ymd_opt(2026, 7, 17).unwrap(),
        "the just-closed unpaid statement is due Jul 17, not Aug 17"
    );
    assert!(
        cycle0.forecast_payment_minor > 0,
        "a payment is projected for it"
    );

    // End-to-end: the first liquid drop (the card payment) lands on 2026-07-17, not Aug 17.
    let view = crate::forecast::compute(&conn, as_of, 90, &[]).unwrap();
    let first_drop = view
        .days
        .iter()
        .find(|day| day.closing.p50.minor_units() < view.starting_balance.minor_units())
        .expect("the card payment must move liquid cash within the window");
    assert_eq!(
        (first_drop.date.month(), first_drop.date.day()),
        (7, 17),
        "the first card payment lands on Jul 17; got {}",
        first_drop.date
    );
}

/// personal-cfo-4d8.23.2 (ADR 0039 addendum): a credit card entered with ONLY a due day (no
/// statement_close_day, no APR/limit/grace/philosophy) still projects an upcoming payment — the
/// naive minimum (ADR 0035 §5 default: 1%-of-balance / $25) on its due day, like a loan.
#[test]
fn a_card_with_only_a_due_day_projects_a_naive_payment() {
    use chrono::{Datelike, TimeZone};
    let (_dir, worker) = worker();
    let checking_id = AccountId::new();
    let card = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(checking_id)),
                opening_balance: Some(Money::new(500_000, Currency::Usd)),
            },
        )
        .unwrap();
    create_role_account(&worker, card, "Visa", CashflowRole::CreditFacility);
    // ONLY a due day — no close day, no APR/limit/grace, philosophy left Unknown, no min terms.
    let mut terms = sample_debt_terms(Some(checking_id));
    terms.statement_close_day = None;
    terms.payment_due_day = Some(17);
    terms.repayment_philosophy = RepaymentPhilosophy::Unknown;
    terms.apr_bps = None;
    terms.credit_limit_minor = None;
    terms.grace_period_days = None;
    terms.min_payment_percent_bps = None;
    terms.min_payment_floor_minor = None;
    terms.fixed_amount_minor = None;
    worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: card,
                terms,
            },
        )
        .unwrap();
    // Owes $1,000 — the naive default minimum is max(1% = $10, $25 floor) = $25.
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            card,
            Money::new(-100_000, Currency::Usd),
            NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
        )
        .unwrap();

    let as_of = Utc.with_ymd_and_hms(2026, 7, 6, 12, 0, 0).unwrap();
    let conn = worker.read_connection().unwrap();
    let view = crate::forecast::compute(&conn, as_of, 40, &[]).unwrap();

    let mut prev = view.starting_balance.minor_units();
    let first_drop = view
        .days
        .iter()
        .find_map(|day| {
            let bal = day.closing.p50.minor_units();
            let dropped = bal < prev;
            let delta = prev - bal;
            prev = bal;
            dropped.then_some((day.date, delta))
        })
        .expect("a naive card with a due day must project a payment");
    assert_eq!(
        first_drop.0.day(),
        17,
        "the naive payment lands on the due day"
    );
    assert_eq!(
        first_drop.1, 2_500,
        "the naive minimum is the $25 default floor"
    );
}

/// personal-cfo-4d8.23.2: a card with NEITHER a close day NOR a due day has nothing to date a
/// payment on, so it contributes no outflow (naive projection needs at least a due day).
#[test]
fn a_card_with_no_due_day_projects_nothing() {
    use chrono::TimeZone;
    let (_dir, worker) = worker();
    let checking_id = AccountId::new();
    let card = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(checking_id)),
                opening_balance: Some(Money::new(500_000, Currency::Usd)),
            },
        )
        .unwrap();
    create_role_account(&worker, card, "Visa", CashflowRole::CreditFacility);
    let mut terms = sample_debt_terms(Some(checking_id));
    terms.statement_close_day = None;
    terms.payment_due_day = None;
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
            Money::new(-100_000, Currency::Usd),
            NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
        )
        .unwrap();

    let as_of = Utc.with_ymd_and_hms(2026, 7, 6, 12, 0, 0).unwrap();
    let conn = worker.read_connection().unwrap();
    let view = crate::forecast::compute(&conn, as_of, 60, &[]).unwrap();
    let start = view.starting_balance.minor_units();
    assert!(
        view.days
            .iter()
            .all(|day| day.closing.p50.minor_units() >= start),
        "a card with no due day must not project any payment outflow"
    );
}

/// personal-cfo-4d8.23.2: a naive (cycle-less) card payment must attribute to the card's liquid
/// paying source in the per-account series — not fall to Unallocated. With checking the only
/// liquid account, its per-account series must equal the aggregate.
#[test]
fn a_naive_card_payment_attributes_to_its_paying_source() {
    use chrono::TimeZone;
    let (_dir, worker) = worker();
    let checking_id = AccountId::new();
    let card = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(checking_id)),
                opening_balance: Some(Money::new(500_000, Currency::Usd)),
            },
        )
        .unwrap();
    create_role_account(&worker, card, "Visa", CashflowRole::CreditFacility);
    let mut terms = sample_debt_terms(Some(checking_id));
    terms.statement_close_day = None;
    terms.payment_due_day = Some(17);
    terms.repayment_philosophy = RepaymentPhilosophy::PayMinimum;
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
            Money::new(-100_000, Currency::Usd),
            NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
        )
        .unwrap();

    let as_of = Utc.with_ymd_and_hms(2026, 7, 6, 12, 0, 0).unwrap();
    let conn = worker.read_connection().unwrap();
    let view = crate::forecast::compute(&conn, as_of, 100, &[]).unwrap();
    let multi = crate::forecast::compute_by_account(&conn, as_of, 100, &[]).unwrap();
    let checking_series = multi
        .accounts
        .iter()
        .find(|s| s.account_id == Some(checking_id.as_uuid()))
        .expect("checking series");
    assert_eq!(
        checking_series
            .days
            .last()
            .unwrap()
            .closing
            .p50
            .minor_units(),
        view.days.last().unwrap().closing.p50.minor_units(),
        "the naive card payment attributes to checking (per-account == aggregate), not Unallocated"
    );
    // Sanity: the payment actually left checking (it is below its $5,000 opening).
    assert!(
        checking_series
            .days
            .last()
            .unwrap()
            .closing
            .p50
            .minor_units()
            < 500_000,
        "the naive card payment must debit the paying source"
    );
}

/// personal-cfo-4d8.24.3: a manual future entry attributed to a liquid account moves THAT
/// account's per-account series (not Unallocated); an unattributed entry lands in Unallocated.
/// Either way the per-account closings still sum to the aggregate (ADR 0026 §12).
#[test]
fn a_manual_entry_attributes_to_its_chosen_account() {
    use chrono::TimeZone;
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
    // A -$1,000 one-time outflow next week, attributed to checking.
    worker
        .record_manual_entry(
            Uuid::now_v7(),
            Money::new(-100_000, Currency::Usd),
            NaiveDate::from_ymd_opt(2026, 7, 13).unwrap(),
            "Insurance",
            Some(checking_id.as_uuid()),
        )
        .unwrap();

    let as_of = Utc.with_ymd_and_hms(2026, 7, 6, 12, 0, 0).unwrap();
    let conn = worker.read_connection().unwrap();
    let aggregate = crate::forecast::compute(&conn, as_of, 100, &[]).unwrap();
    let multi = crate::forecast::compute_by_account(&conn, as_of, 100, &[]).unwrap();

    let checking_closing = multi
        .accounts
        .iter()
        .find(|s| s.account_id == Some(checking_id.as_uuid()))
        .expect("checking series")
        .days
        .last()
        .unwrap()
        .closing
        .p50
        .minor_units();
    // The entry moved checking (its closing carries the outflow), and there is no Unallocated
    // series holding it — the attributed flow == the aggregate flow onto checking.
    assert_eq!(
        checking_closing,
        aggregate.days.last().unwrap().closing.p50.minor_units(),
        "the attributed manual entry lands on checking, not Unallocated"
    );
    assert!(
        !multi.accounts.iter().any(|s| s.account_id.is_none()),
        "no Unallocated series when the only flow is account-attributed"
    );

    // Reconciliation invariant (ADR 0026 §12): Σ per-account closings == aggregate closing.
    let per_account_sum: i64 = multi
        .accounts
        .iter()
        .map(|s| s.days.last().unwrap().closing.p50.minor_units())
        .sum();
    assert_eq!(
        per_account_sum,
        aggregate.days.last().unwrap().closing.p50.minor_units(),
    );
}

/// ADR 0035 §3 (6wk.4): a loan set pay_fixed_amount projects a fixed monthly outflow on its
/// due day from the paying source — in the aggregate AND that account's series.
#[test]
fn loan_payment_projects_a_fixed_monthly_outflow_from_the_paying_source() {
    use chrono::{Datelike, TimeZone};
    let (_dir, worker) = worker();
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
    create_role_account(&worker, loan, "Auto loan", CashflowRole::LoanLiability);
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
    // The loan owes $10,000 (a liability balance is negative).
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            loan,
            Money::new(-1_000_000, Currency::Usd),
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        )
        .unwrap();

    let as_of = Utc.with_ymd_and_hms(2026, 7, 1, 12, 0, 0).unwrap();
    let conn = worker.read_connection().unwrap();
    let view = crate::forecast::compute(&conn, as_of, 100, &[]).unwrap();
    let mut prev = view.starting_balance.minor_units();
    let mut drops = 0;
    for day in &view.days {
        let bal = day.closing.p50.minor_units();
        if bal < prev {
            assert_eq!(
                day.date.day(),
                15,
                "loan payment on the 15th, got {}",
                day.date
            );
            assert_eq!(prev - bal, 150_000, "the fixed $1,500 payment");
            drops += 1;
        }
        prev = bal;
    }
    assert!(drops >= 2, "recurring monthly loan payments; got {drops}");

    // Reconciliation: checking is the only liquid account, so its per-account series must
    // equal the aggregate — i.e. the loan payment attributes to checking, not Unallocated.
    let multi = crate::forecast::compute_by_account(&conn, as_of, 100, &[]).unwrap();
    let checking_series = multi
        .accounts
        .iter()
        .find(|s| s.account_id == Some(checking_id.as_uuid()))
        .expect("checking series");
    assert_eq!(
        checking_series
            .days
            .last()
            .unwrap()
            .closing
            .p50
            .minor_units(),
        view.days.last().unwrap().closing.p50.minor_units(),
        "the loan payment attributes to checking (per-account == aggregate)"
    );
}

/// ADR 0035 §3 (6wk.4): a loan set pay_in_full projects a single owed-balance payoff on the
/// next due date, not a recurring outflow.
#[test]
fn loan_pay_in_full_projects_a_single_owed_balance_payoff() {
    use chrono::{Datelike, TimeZone};
    let (_dir, worker) = worker();
    let checking_id = AccountId::new();
    let loan = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(checking_id)),
                opening_balance: Some(Money::new(2_000_000, Currency::Usd)),
            },
        )
        .unwrap();
    create_role_account(&worker, loan, "Personal loan", CashflowRole::LoanLiability);
    worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: loan,
                terms: loan_terms(RepaymentPhilosophy::PayInFull, None, checking_id),
            },
        )
        .unwrap();
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            loan,
            Money::new(-1_000_000, Currency::Usd), // owes $10,000
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        )
        .unwrap();

    let as_of = Utc.with_ymd_and_hms(2026, 7, 1, 12, 0, 0).unwrap();
    let conn = worker.read_connection().unwrap();
    let view = crate::forecast::compute(&conn, as_of, 100, &[]).unwrap();
    let mut prev = view.starting_balance.minor_units();
    let mut drops = 0;
    for day in &view.days {
        let bal = day.closing.p50.minor_units();
        if bal < prev {
            assert_eq!(day.date.day(), 15);
            assert_eq!(prev - bal, 1_000_000, "the full owed-balance payoff");
            drops += 1;
        }
        prev = bal;
    }
    assert_eq!(
        drops, 1,
        "pay-in-full is a single payoff, not recurring; got {drops}"
    );
}

/// ADR 0035 §3 (6wk.4 review): a loan denominated in a different currency from the liquid
/// forecast is skipped — it must NOT break the whole projection (no offline FX in v1).
#[test]
fn a_foreign_currency_loan_is_skipped_not_a_forecast_failure() {
    use chrono::TimeZone;
    let (_dir, worker) = worker();
    let checking_id = AccountId::new();
    let loan = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(checking_id)), // USD
                opening_balance: Some(Money::new(500_000, Currency::Usd)),
            },
        )
        .unwrap();
    // A EUR loan alongside the USD liquid account.
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(Account::new(
                    loan,
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
            Money::new(-1_000_000, Currency::Eur),
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        )
        .unwrap();

    let as_of = Utc.with_ymd_and_hms(2026, 7, 1, 12, 0, 0).unwrap();
    let conn = worker.read_connection().unwrap();
    // The forecast must succeed; the EUR loan is simply not modeled.
    let view = crate::forecast::compute(&conn, as_of, 100, &[]).unwrap();
    let start = view.starting_balance.minor_units();
    assert!(
        view.days
            .iter()
            .all(|d| d.closing.p50.minor_units() == start),
        "a foreign-currency loan is skipped, so the USD forecast stays flat"
    );
}

/// ADR 0035 §5 (6wk.4 review): a pay_minimum loan with no explicit minimum terms projects the
/// default minimum (1%-of-balance / $25), not $0.
#[test]
fn a_pay_minimum_loan_without_min_terms_projects_the_default_minimum() {
    use chrono::{Datelike, TimeZone};
    let (_dir, worker) = worker();
    let checking_id = AccountId::new();
    let loan = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(checking_id)),
                opening_balance: Some(Money::new(5_000_000, Currency::Usd)),
            },
        )
        .unwrap();
    create_role_account(&worker, loan, "Student loan", CashflowRole::LoanLiability);
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
            Money::new(-1_000_000, Currency::Usd), // owes $10,000 → default min = 1% = $100
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        )
        .unwrap();

    let as_of = Utc.with_ymd_and_hms(2026, 7, 1, 12, 0, 0).unwrap();
    let conn = worker.read_connection().unwrap();
    let view = crate::forecast::compute(&conn, as_of, 100, &[]).unwrap();
    let mut prev = view.starting_balance.minor_units();
    let mut drops = 0;
    for day in &view.days {
        let bal = day.closing.p50.minor_units();
        if bal < prev {
            assert_eq!(day.date.day(), 15);
            assert_eq!(
                prev - bal,
                10_000,
                "the default 1% minimum on $10,000 = $100"
            );
            drops += 1;
        }
        prev = bal;
    }
    assert!(
        drops >= 2,
        "a default-minimum loan projects a payment, not $0; got {drops}"
    );
}

/// ADR 0035 §3 (6wk.4 review): a loan with no paying source still depletes cash in the
/// aggregate and lands in the Unallocated per-account series.
#[test]
fn a_loan_without_a_paying_source_lands_in_unallocated() {
    use chrono::TimeZone;
    let (_dir, worker) = worker();
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
    create_role_account(&worker, loan, "Auto loan", CashflowRole::LoanLiability);
    let mut terms = loan_terms(
        RepaymentPhilosophy::PayFixedAmount,
        Some(150_000),
        checking_id,
    );
    terms.paying_source_account_id = None; // no paying source
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
            Money::new(-1_000_000, Currency::Usd),
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        )
        .unwrap();

    let as_of = Utc.with_ymd_and_hms(2026, 7, 1, 12, 0, 0).unwrap();
    let conn = worker.read_connection().unwrap();
    // Aggregate still carries the outflow.
    let view = crate::forecast::compute(&conn, as_of, 100, &[]).unwrap();
    assert!(
        view.days.last().unwrap().closing.p50.minor_units() < view.starting_balance.minor_units(),
        "an unattributed loan payment still depletes the aggregate liquid forecast"
    );
    // Per-account: it lands in the Unallocated series (account_id None), not checking.
    let multi = crate::forecast::compute_by_account(&conn, as_of, 100, &[]).unwrap();
    let unallocated = multi
        .accounts
        .iter()
        .find(|s| s.account_id.is_none())
        .expect("an Unallocated series for the unattributed loan payment");
    assert!(
        unallocated.days.last().unwrap().closing.p50.minor_units() < 0,
        "the loan payment lands in Unallocated when there's no paying source"
    );
}

/// personal-cfo-6wk.19: a scenario-scoped recurring extra-debt-payment overlay projects a
/// monthly liquid outflow in that scenario's forecast, leaving the base forecast unchanged.
#[test]
fn a_scenario_recurring_debt_payment_reduces_projected_liquid_cash() {
    use chrono::TimeZone;
    let (_dir, worker) = worker();
    let checking_id = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(checking_id)),
                opening_balance: Some(Money::new(1_000_000, Currency::Usd)),
            },
        )
        .unwrap();

    let scenario = Uuid::now_v7();
    // The scenario must EXIST to be applied (ADR 0051 §1: `effective_scenario` treats a
    // missing row as not-selectable, which is what makes a stale selection fall back to
    // base after a delete). Every production path creates the row first — the payoff
    // flow calls `create_scenario` before attaching its overlay.
    worker
        .create_scenario(&NewScenario {
            id: scenario,
            name: "Snowball".to_owned(),
            description: None,
            base_run_id: None,
        })
        .unwrap();
    worker
        .record_forecast_assumption(&ForecastAssumptionSpec {
            id: Uuid::now_v7(),
            scenario_id: Some(scenario),
            target_entity_id: None,
            params: AssumptionParams::RecurringDebtPayment {
                amount: Money::new(30_000, Currency::Usd), // $300/mo extra
                anchor_date: NaiveDate::from_ymd_opt(2026, 8, 1).unwrap(),
                end_date: None,
                label: "Snowball extra".to_owned(),
            },
        })
        .unwrap();

    let as_of = Utc.with_ymd_and_hms(2026, 7, 1, 12, 0, 0).unwrap();
    let conn = worker.read_connection().unwrap();
    let base = crate::forecast::compute(&conn, as_of, 100, &[]).unwrap();
    let scen = crate::forecast::compute(&conn, as_of, 100, &[scenario]).unwrap();

    // The base forecast is untouched by the scenario overlay (flat $10,000, no payments).
    let start = base.starting_balance.minor_units();
    assert_eq!(base.days.last().unwrap().closing.p50.minor_units(), start);
    assert!(
        base.days
            .iter()
            .flat_map(|d| &d.events)
            .all(|e| e.amount.minor_units() != -30_000),
        "the scenario-scoped overlay must not leak into the base forecast",
    );
    // The scenario projects one −$300 outflow per month; the liquid drop equals their sum.
    let payments = scen
        .days
        .iter()
        .flat_map(|d| &d.events)
        .filter(|e| e.amount.minor_units() == -30_000)
        .count();
    assert!(
        payments >= 3,
        "≥3 monthly extra payments in the 100-day window; got {payments}"
    );
    assert_eq!(
        start - scen.days.last().unwrap().closing.p50.minor_units(),
        payments as i64 * 30_000,
        "the projected liquid drop equals the sum of the extra debt payments",
    );
}

/// personal-cfo-6wk.15 review: an overlay in a currency other than the liquid forecast
/// currency is skipped (no offline FX), so the scenario forecast can't hard-error.
#[test]
fn a_foreign_currency_recurring_debt_payment_is_skipped() {
    use chrono::TimeZone;
    let (_dir, worker) = worker();
    let checking_id = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(checking_id)), // USD liquid
                opening_balance: Some(Money::new(1_000_000, Currency::Usd)),
            },
        )
        .unwrap();

    let scenario = Uuid::now_v7();
    // The scenario must EXIST to be applied (ADR 0051 §1: `effective_scenario` treats a
    // missing row as not-selectable, which is what makes a stale selection fall back to
    // base after a delete). Every production path creates the row first — the payoff
    // flow calls `create_scenario` before attaching its overlay.
    worker
        .create_scenario(&NewScenario {
            id: scenario,
            name: "Snowball".to_owned(),
            description: None,
            base_run_id: None,
        })
        .unwrap();
    worker
        .record_forecast_assumption(&ForecastAssumptionSpec {
            id: Uuid::now_v7(),
            scenario_id: Some(scenario),
            target_entity_id: None,
            params: AssumptionParams::RecurringDebtPayment {
                amount: Money::new(30_000, Currency::Eur), // EUR overlay vs a USD forecast
                anchor_date: NaiveDate::from_ymd_opt(2026, 8, 1).unwrap(),
                end_date: None,
                label: "Euro extra".to_owned(),
            },
        })
        .unwrap();

    let as_of = Utc.with_ymd_and_hms(2026, 7, 1, 12, 0, 0).unwrap();
    let conn = worker.read_connection().unwrap();
    // The scenario forecast must succeed (not a CurrencyMismatch) and stay flat like the base.
    let scen = crate::forecast::compute(&conn, as_of, 100, &[scenario]).unwrap();
    let start = scen.starting_balance.minor_units();
    assert!(
        scen.days
            .iter()
            .all(|d| d.closing.p50.minor_units() == start),
        "a foreign-currency overlay is skipped, so the USD forecast stays flat",
    );
}

#[test]
fn persisted_forecast_is_reproducible() {
    use chrono::TimeZone;

    let (_dir, worker) = worker();
    let seed = worker.read_connection().unwrap();
    seed.execute(
        "INSERT INTO recurring_events
                (id, name, amount_expected_minor, currency, frequency,
                 next_expected_date, created_at, updated_at)
             VALUES (?1, 'Rent', 180000, 'USD', 'monthly', '2026-07-01', 'now', 'now')",
        params![Uuid::now_v7()],
    )
    .unwrap();
    seed.execute(
        "INSERT INTO income_sources
                (id, name, net_minor_units, currency, frequency, anchor_date, created_at)
             VALUES (?1, 'Acme', 500000, 'USD', 'monthly', '2026-06-15', 'now')",
        params![Uuid::now_v7()],
    )
    .unwrap();

    // Persist twice from identical inputs at a fixed instant.
    let as_of = Utc.with_ymd_and_hms(2026, 6, 20, 12, 0, 0).unwrap();
    let run_a = worker.persist_forecast_at(as_of, 60).unwrap();
    let run_b = worker.persist_forecast_at(as_of, 60).unwrap();
    assert_ne!(run_a, run_b, "each run gets a fresh id");

    let rows = |run: Uuid| -> Vec<(String, i64, i64, String)> {
        let conn = worker.read_connection().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT date, amount_p50_minor, running_balance_p50_minor, source_type
                     FROM forecast_rows WHERE forecast_run_id = ?1 ORDER BY id",
            )
            .unwrap();
        stmt.query_map(params![run], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
    };
    let rows_a = rows(run_a);
    let rows_b = rows(run_b);
    assert!(
        rows_a.len() > 1,
        "a starting balance plus income/bill events produce rows"
    );
    assert_eq!(
        rows_a[0].3, "starting_balance",
        "first row is the opening position"
    );
    assert_eq!(
        rows_a, rows_b,
        "re-running from identical inputs is byte-identical"
    );

    // Identical inputs dedup to one content-addressed snapshot.
    let snapshot = |run: Uuid| -> Uuid {
        worker
            .read_connection()
            .unwrap()
            .query_row(
                "SELECT input_snapshot_id FROM forecast_runs WHERE id = ?1",
                params![run],
                |r| r.get(0),
            )
            .unwrap()
    };
    assert_eq!(
        snapshot(run_a),
        snapshot(run_b),
        "identical inputs share one snapshot"
    );
}

#[test]
fn manual_entry_folds_into_forecast_and_survives_rerun() {
    use chrono::TimeZone;

    let (_dir, worker) = worker();
    // A liquid account gives the forecast a starting balance + currency.
    worker
        .dispatch(
            meta(),
            liquid_account_cmd(AccountId::new(), Currency::Usd, 100_000),
        )
        .unwrap();

    // A manual entry: +$5,000 on 2026-08-01.
    let entry = Uuid::now_v7();
    let date = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
    worker
        .record_manual_entry(
            entry,
            Money::new(500_000, Currency::Usd),
            date,
            "Bonus",
            None,
        )
        .unwrap();

    let as_of = Utc.with_ymd_and_hms(2026, 7, 1, 12, 0, 0).unwrap();
    let view = forecast::compute(&worker.read_connection().unwrap(), as_of, 90, &[]).unwrap();

    // It lands on its day as a manual_entry event with the manual_one_off basis.
    let day = view
        .days
        .iter()
        .find(|d| d.date == date)
        .expect("the manual entry's day is in the horizon");
    let event = day
        .events
        .iter()
        .find(|e| e.source_event_id == entry)
        .expect("the manual entry is folded in");
    assert_eq!(event.kind, "manual_entry");
    assert_eq!(event.amount.minor_units(), 500_000);
    assert_eq!(event.name, "Bonus");
    assert_eq!(event.assumption_basis, AssumptionBasis::ManualOneOff);
    // It lifts the closing balance: 100_000 opening + 500_000 = 600_000.
    assert_eq!(view.days.last().unwrap().closing.p50.minor_units(), 600_000);

    // Survives a re-run (deterministic).
    let again = forecast::compute(&worker.read_connection().unwrap(), as_of, 90, &[]).unwrap();
    assert_eq!(
        again.days.last().unwrap().closing.p50,
        view.days.last().unwrap().closing.p50,
    );

    // Editing supersedes (no silent mutation): the old entry leaves, the new one lands.
    let replacement = Uuid::now_v7();
    worker
        .record_manual_entry(
            replacement,
            Money::new(700_000, Currency::Usd),
            date,
            "Bigger bonus",
            None,
        )
        .unwrap();
    worker
        .supersede_assumption_event(entry, replacement)
        .unwrap();
    let after = forecast::compute(&worker.read_connection().unwrap(), as_of, 90, &[]).unwrap();
    let day = after.days.iter().find(|d| d.date == date).unwrap();
    assert!(
        day.events.iter().all(|e| e.source_event_id != entry),
        "the superseded entry is no longer folded in",
    );
    assert!(
        day.events.iter().any(|e| e.source_event_id == replacement),
        "the replacement entry is folded in",
    );
    assert_eq!(
        after.days.last().unwrap().closing.p50.minor_units(),
        800_000
    );
}

#[test]
fn scenario_overlay_applies_additions_overrides_and_exclusions() {
    use chrono::TimeZone;

    let (_dir, worker) = worker();
    worker
        .dispatch(
            meta(),
            liquid_account_cmd(AccountId::new(), Currency::Usd, 1_000_000),
        )
        .unwrap();
    // A base monthly bill (Rent, -$1,800).
    let bill = Uuid::now_v7();
    worker
        .read_connection()
        .unwrap()
        .execute(
            "INSERT INTO recurring_events
                    (id, name, amount_expected_minor, currency, frequency,
                     next_expected_date, created_at, updated_at)
                 VALUES (?1, 'Rent', 180000, 'USD', 'monthly', '2026-07-01', 'now', 'now')",
            params![bill],
        )
        .unwrap();

    let as_of = Utc.with_ymd_and_hms(2026, 6, 20, 12, 0, 0).unwrap();
    let conn = || worker.read_connection().unwrap();
    let base_end = forecast::compute(&conn(), as_of, 90, &[])
        .unwrap()
        .days
        .last()
        .unwrap()
        .closing
        .p50
        .minor_units();

    // A scenario: a +$5,000 addition + a rent override to -$2,500, scenario-scoped.
    let scenario = Uuid::now_v7();
    worker
        .create_scenario(&NewScenario {
            id: scenario,
            name: "Raise + rent hike".to_owned(),
            description: None,
            base_run_id: None,
        })
        .unwrap();
    worker
        .record_assumption_event(&NewAssumptionEvent {
            id: Uuid::now_v7(),
            kind: AssumptionKind::OneTimeEvent,
            target_entity_type: None,
            target_entity_id: None,
            params_json:
                r#"{"amount_minor":500000,"currency":"USD","date":"2026-08-01","label":"Bonus"}"#
                    .to_owned(),
            source: AssumptionSource::UserOverride,
            scenario_id: Some(scenario),
            origin_run_id: None,
        })
        .unwrap();
    worker
        .record_assumption_event(&NewAssumptionEvent {
            id: Uuid::now_v7(),
            kind: AssumptionKind::BillAmount,
            target_entity_type: Some("recurring_event".to_owned()),
            target_entity_id: Some(bill),
            params_json: r#"{"new_amount_minor":250000}"#.to_owned(),
            source: AssumptionSource::UserOverride,
            scenario_id: Some(scenario),
            origin_run_id: None,
        })
        .unwrap();

    // Base is unaffected — scenario events do not leak.
    let base2 = forecast::compute(&conn(), as_of, 90, &[]).unwrap();
    assert_eq!(
        base2.days.last().unwrap().closing.p50.minor_units(),
        base_end,
        "base forecast is unaffected by scenario-scoped events",
    );

    // The scenario forecast folds the addition and overrides the rent amount.
    let scen = forecast::compute(&conn(), as_of, 90, &[scenario]).unwrap();
    let scen_events: Vec<_> = scen.days.iter().flat_map(|d| &d.events).collect();
    assert!(
        scen_events
            .iter()
            .any(|e| e.kind == "manual_entry" && e.amount.minor_units() == 500_000),
        "the scenario addition is folded in",
    );
    assert!(
        scen_events
            .iter()
            .any(|e| e.source_event_id == bill && e.amount.minor_units() == -250_000),
        "the scenario overrides the rent amount",
    );
    assert!(
        !scen_events
            .iter()
            .any(|e| e.source_event_id == bill && e.amount.minor_units() == -180_000),
        "the base rent amount is replaced under the scenario",
    );

    // A base exclusion drops the bill entirely.
    worker
        .record_assumption_event(&NewAssumptionEvent {
            id: Uuid::now_v7(),
            kind: AssumptionKind::Exclusion,
            target_entity_type: Some("recurring_event".to_owned()),
            target_entity_id: Some(bill),
            params_json: "{}".to_owned(),
            source: AssumptionSource::UserOverride,
            scenario_id: None,
            origin_run_id: None,
        })
        .unwrap();
    let excluded = forecast::compute(&conn(), as_of, 90, &[]).unwrap();
    assert!(
        !excluded
            .days
            .iter()
            .flat_map(|d| &d.events)
            .any(|e| e.source_event_id == bill),
        "an exclusion drops the targeted bill from the projection",
    );
}

#[test]
fn zero_income_override_zeroes_that_income_under_the_scenario() {
    use chrono::TimeZone;
    let (_dir, worker) = worker();
    worker.dispatch(meta(), income_cmd(None)).unwrap();
    let income_id = worker.income_source_views().unwrap()[0].id.as_uuid();

    // A scenario that zeroes the income (e.g. unpaid leave) — new_amount_minor = 0
    // is a valid override, not an exclusion (personal-cfo-yiau).
    let scenario = Uuid::now_v7();
    worker
        .create_scenario(&NewScenario {
            id: scenario,
            name: "Unpaid leave".to_owned(),
            description: None,
            base_run_id: None,
        })
        .unwrap();
    worker
        .record_assumption_event(&NewAssumptionEvent {
            id: Uuid::now_v7(),
            kind: AssumptionKind::IncomeAmount,
            target_entity_type: Some("income_source".to_owned()),
            target_entity_id: Some(income_id),
            params_json: r#"{"new_amount_minor":0}"#.to_owned(),
            source: AssumptionSource::UserOverride,
            scenario_id: Some(scenario),
            origin_run_id: None,
        })
        .unwrap();

    let as_of = Utc.with_ymd_and_hms(2026, 6, 20, 12, 0, 0).unwrap();
    let conn = || worker.read_connection().unwrap();

    // Base: the income projects its full amount.
    let base = forecast::compute(&conn(), as_of, 90, &[]).unwrap();
    assert!(
        base.days
            .iter()
            .flat_map(|d| &d.events)
            .any(|e| e.source_event_id == income_id && e.amount.minor_units() == 300_000),
        "base income projects its full amount",
    );

    // Scenario: the occurrences are still projected, but zeroed (not excluded).
    let scen = forecast::compute(&conn(), as_of, 90, &[scenario]).unwrap();
    let income_events: Vec<_> = scen
        .days
        .iter()
        .flat_map(|d| &d.events)
        .filter(|e| e.source_event_id == income_id)
        .collect();
    assert!(
        !income_events.is_empty(),
        "the 0 occurrence is still projected"
    );
    assert!(
        income_events.iter().all(|e| e.amount.minor_units() == 0),
        "the scenario zeroes the income",
    );
}

#[test]
fn record_forecast_assumption_drives_the_scenario_forecast() {
    use chrono::TimeZone;

    let (_dir, worker) = worker();
    worker
        .dispatch(
            meta(),
            liquid_account_cmd(AccountId::new(), Currency::Usd, 1_000_000),
        )
        .unwrap();
    // A base monthly bill (Rent, -$1,800).
    let bill = Uuid::now_v7();
    worker
        .read_connection()
        .unwrap()
        .execute(
            "INSERT INTO recurring_events
                    (id, name, amount_expected_minor, currency, frequency,
                     next_expected_date, created_at, updated_at)
                 VALUES (?1, 'Rent', 180000, 'USD', 'monthly', '2026-07-01', 'now', 'now')",
            params![bill],
        )
        .unwrap();
    let as_of = Utc.with_ymd_and_hms(2026, 6, 20, 12, 0, 0).unwrap();
    let conn = || worker.read_connection().unwrap();

    let scenario = Uuid::now_v7();
    worker
        .create_scenario(&NewScenario {
            id: scenario,
            name: "Raise + rent hike".to_owned(),
            description: None,
            base_run_id: None,
        })
        .unwrap();

    // Build the overlay through the TYPED creator (the create_forecast_assumption
    // path): a +$5,000 addition + a rent override to -$2,500, scenario-scoped.
    worker
        .record_forecast_assumption(&ForecastAssumptionSpec {
            id: Uuid::now_v7(),
            scenario_id: Some(scenario),
            target_entity_id: None,
            params: AssumptionParams::OneTimeEvent {
                amount: Money::new(500_000, Currency::Usd),
                date: NaiveDate::from_ymd_opt(2026, 8, 1).unwrap(),
                label: "Bonus".to_owned(),
            },
        })
        .unwrap();
    worker
        .record_forecast_assumption(&ForecastAssumptionSpec {
            id: Uuid::now_v7(),
            scenario_id: Some(scenario),
            target_entity_id: Some(bill),
            params: AssumptionParams::BillAmount {
                new_amount_minor: 250_000,
                effective_date: None,
                end_date: None,
            },
        })
        .unwrap();

    // The base keeps the original rent and has no addition (events are scoped).
    let base = forecast::compute(&conn(), as_of, 90, &[]).unwrap();
    let base_events: Vec<_> = base.days.iter().flat_map(|d| &d.events).collect();
    assert!(
        base_events
            .iter()
            .any(|e| e.source_event_id == bill && e.amount.minor_units() == -180_000),
        "base keeps the original rent amount",
    );
    assert!(
        base_events.iter().all(|e| e.kind != "manual_entry"),
        "base has no scenario addition",
    );

    // The scenario forecast reflects both typed events.
    let scen = forecast::compute(&conn(), as_of, 90, &[scenario]).unwrap();
    let scen_events: Vec<_> = scen.days.iter().flat_map(|d| &d.events).collect();
    assert!(
        scen_events
            .iter()
            .any(|e| e.kind == "manual_entry" && e.amount.minor_units() == 500_000),
        "the typed addition is folded into the scenario forecast",
    );
    assert!(
        scen_events
            .iter()
            .any(|e| e.source_event_id == bill && e.amount.minor_units() == -250_000),
        "the typed bill override changes the projected amount under the scenario",
    );
}

/// w6o9 end-to-end: two windowed income overrides written through the typed
/// creator parse back via `entity_overrides` and compose by window (each applies
/// in its range; the base resumes outside them) — the family-leave step-down.
#[test]
fn windowed_income_overrides_parse_and_compose() {
    let (_dir, worker) = worker();
    let income = Uuid::now_v7();
    worker
        .record_forecast_assumption(&ForecastAssumptionSpec {
            id: Uuid::now_v7(),
            scenario_id: None,
            target_entity_id: Some(income),
            params: AssumptionParams::IncomeAmount {
                new_amount_minor: 200_000,
                effective_date: Some(NaiveDate::from_ymd_opt(2026, 10, 1).unwrap()),
                end_date: Some(NaiveDate::from_ymd_opt(2026, 11, 30).unwrap()),
            },
        })
        .unwrap();
    worker
        .record_forecast_assumption(&ForecastAssumptionSpec {
            id: Uuid::now_v7(),
            scenario_id: None,
            target_entity_id: Some(income),
            params: AssumptionParams::IncomeAmount {
                new_amount_minor: 0,
                effective_date: Some(NaiveDate::from_ymd_opt(2026, 12, 1).unwrap()),
                end_date: None,
            },
        })
        .unwrap();

    let conn = worker.read_connection().unwrap();
    let overrides = crate::forecast_overrides::entity_overrides(&conn, &[]).unwrap();
    let over = overrides.get(&income).expect("override present");
    let base = 400_000;
    let day = |s: &str| NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();
    assert_eq!(over.amount_for(day("2026-09-15"), base), Some(base));
    assert_eq!(over.amount_for(day("2026-10-15"), base), Some(200_000));
    assert_eq!(over.amount_for(day("2026-12-15"), base), Some(0));
}

#[test]
fn archive_and_expiry_stop_a_scenario_without_destroying_it() {
    // ADR 0051: archive KEEPS the overlay but must still stop it reaching a run, and
    // expiry does the same on a date. Both are enforced on the READ path
    // (`effective_scenario`) — before ADR 0051 nothing consulted a scenario's status,
    // so `delete_scenario` cleared its events to make archiving stick, which is exactly
    // the data loss the archive/delete split exists to prevent.
    use chrono::TimeZone;

    let (_dir, worker) = worker();
    worker
        .dispatch(
            meta(),
            liquid_account_cmd(AccountId::new(), Currency::Usd, 1_000_000),
        )
        .unwrap();
    let as_of = Utc.with_ymd_and_hms(2026, 6, 20, 12, 0, 0).unwrap();
    let conn = || worker.read_connection().unwrap();
    let applies = |scenario: Uuid| {
        forecast::compute(&conn(), as_of, 90, &[scenario])
            .unwrap()
            .days
            .iter()
            .flat_map(|d| &d.events)
            .any(|e| e.amount.minor_units() == 500_000)
    };

    let scenario = Uuid::now_v7();
    worker
        .create_scenario(&NewScenario {
            id: scenario,
            name: "What if".to_owned(),
            description: None,
            base_run_id: None,
        })
        .unwrap();
    worker
        .record_forecast_assumption(&ForecastAssumptionSpec {
            id: Uuid::now_v7(),
            scenario_id: Some(scenario),
            target_entity_id: None,
            params: AssumptionParams::OneTimeEvent {
                amount: Money::new(500_000, Currency::Usd),
                date: NaiveDate::from_ymd_opt(2026, 8, 1).unwrap(),
                label: "Bonus".to_owned(),
            },
        })
        .unwrap();
    assert!(applies(scenario), "a draft scenario applies when selected");

    // ARCHIVE: the overlay survives, but the run no longer picks it up.
    worker.archive_scenario(scenario).unwrap();
    assert_eq!(
        worker
            .active_assumption_events(Some(scenario))
            .unwrap()
            .len(),
        1,
        "archiving must keep the scenario's events",
    );
    assert!(
        !applies(scenario),
        "an archived scenario must not reach a forecast run even if its id is passed in",
    );

    // RESTORE: it comes back whole.
    worker
        .set_scenario_status(scenario, ScenarioStatus::Draft)
        .unwrap();
    assert!(applies(scenario), "restoring returns the scenario intact");

    // EXPIRY: a past date gates it; a future date does not; clearing it restores.
    worker
        .set_scenario_expiry(scenario, Some("2026-06-19"))
        .unwrap();
    assert!(!applies(scenario), "an expired scenario does not apply");
    worker
        .set_scenario_expiry(scenario, Some("2026-06-20"))
        .unwrap();
    assert!(
        applies(scenario),
        "expiry is inclusive — a scenario expiring today still applies today",
    );
    worker.set_scenario_expiry(scenario, None).unwrap();
    assert!(applies(scenario));

    // DELETE: scenario and overlay are gone, and a stale selection degrades to base
    // rather than erroring or resurrecting the overlay.
    worker.delete_scenario(scenario).unwrap();
    assert!(
        worker.scenario(scenario).unwrap().is_none(),
        "delete removes the scenario row",
    );
    assert!(!applies(scenario));
}

#[test]
fn golden_vault_forecast_matches_known_baseline() {
    // personal-cfo-rdg9: a fixture vault with known forecast / projection /
    // availability / readiness outputs — a regression baseline.
    let (_dir, worker) = worker();
    let (checking, savings) = seed_golden_vault(&worker);
    let as_of = golden_as_of();
    let conn = worker.read_connection().unwrap();

    // --- Aggregate Future Cash forecast over 30 days ---
    let forecast = forecast::compute(&conn, as_of, 30, &[]).unwrap();
    assert_eq!(
        forecast.starting_balance,
        Money::new(1_500_000, Currency::Usd)
    );
    // Two biweekly paychecks (2026-07-03, -07-17) and one monthly rent (-07-01).
    let income_total: i64 = forecast
        .days
        .iter()
        .flat_map(|d| &d.events)
        .filter(|e| e.kind == "income")
        .map(|e| e.amount.minor_units())
        .sum();
    let bill_total: i64 = forecast
        .days
        .iter()
        .flat_map(|d| &d.events)
        .filter(|e| e.kind == "recurring_bill")
        .map(|e| e.amount.minor_units())
        .sum();
    assert_eq!(income_total, 600_000); // 2 × $3,000
    assert_eq!(bill_total, -180_000); // 1 × $1,800 outflow
    assert_eq!(
        forecast.days.last().unwrap().closing.p50,
        Money::new(1_920_000, Currency::Usd) // 15,000 + 6,000 − 1,800
    );

    // --- Cash availability (ADR 0029) ---
    let avail = forecast::compute_cash_availability(&conn, as_of, 0).unwrap();
    assert_eq!(avail.net_available, Money::new(1_500_000, Currency::Usd));
    assert_eq!(avail.net_committed, Money::new(180_000, Currency::Usd));
    assert_eq!(avail.net_headroom, Money::new(1_320_000, Currency::Usd));
    let checking_view = avail
        .accounts
        .iter()
        .find(|a| a.account_id == checking.as_uuid())
        .unwrap();
    assert_eq!(checking_view.committed, Money::new(180_000, Currency::Usd));
    assert_eq!(checking_view.headroom, Money::new(320_000, Currency::Usd));
    let savings_view = avail
        .accounts
        .iter()
        .find(|a| a.account_id == savings.as_uuid())
        .unwrap();
    assert_eq!(savings_view.ledger, Money::new(1_000_000, Currency::Usd));
    assert_eq!(savings_view.committed, Money::new(0, Currency::Usd));

    // --- Forecast readiness (ADR 0026 §8 / §18): full coverage + fresh + no txns.
    //     Caps at 75, not 100: the two earned-over-time factors are still 0 — no
    //     categorized spending history (spending-history, 0.15) and no realized actuals
    //     (recurrence-actuals, 0.10); backtest-accuracy is neutral (nothing to score
    //     yet). The re-pinned seven-factor weighted sum (nxgx). ---
    let readiness = forecast::compute_forecast_readiness(&conn, as_of).unwrap();
    assert_eq!(readiness.score, 75);
}

#[test]
fn golden_vault_forecast_is_deterministic() {
    // personal-cfo-tv3w: same vault + same as_of → identical output, and the
    // per-account projection reconciles to the aggregate every day.
    let (_dir, worker) = worker();
    seed_golden_vault(&worker);
    let as_of = golden_as_of();
    let conn = worker.read_connection().unwrap();

    let first = forecast::compute(&conn, as_of, 90, &[]).unwrap();
    let second = forecast::compute(&conn, as_of, 90, &[]).unwrap();
    assert_eq!(
        first, second,
        "the forecast must be deterministic for a fixed as_of"
    );

    let multi_first = forecast::compute_by_account(&conn, as_of, 90, &[]).unwrap();
    let multi_second = forecast::compute_by_account(&conn, as_of, 90, &[]).unwrap();
    assert_eq!(multi_first, multi_second);

    // Reconciliation invariant (ADR 0026 §12): Σ per-account closing (including
    // the Unallocated series) == the aggregate closing, every day.
    for (day, aggregate) in first.days.iter().enumerate() {
        let summed: i64 = multi_first
            .accounts
            .iter()
            .map(|series| series.days[day].closing.p50.minor_units())
            .sum();
        assert_eq!(
            summed,
            aggregate.closing.p50.minor_units(),
            "per-account closings must sum to the aggregate on day {day}"
        );
    }
}

/// personal-cfo-4d8.27.5.7.2: a liquid account with enough of its OWN categorized variable
/// spend gets a per-account variance cone (monotonically widening); an account with no spend
/// history stays a deterministic line. Card variance is NOT injected here (that is the later
/// card-lump work), so this exercises the CASH-account continuous cone only.
#[test]
fn per_account_cash_cone_widens_with_the_accounts_own_spend() {
    use chrono::TimeZone;
    let as_of = Utc.with_ymd_and_hms(2026, 6, 20, 12, 0, 0).unwrap();
    let today = as_of.date_naive();

    let (_dir, worker) = worker();
    let checking_id = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(checking_id)),
                opening_balance: Some(Money::new(5_000_000, Currency::Usd)),
            },
        )
        .unwrap();
    // A second liquid account with NO spend history → stays a deterministic line.
    let savings_id = AccountId::new();
    worker
        .dispatch(
            meta(),
            liquid_subtype_cmd(
                savings_id,
                "Savings",
                Some(AccountSubtype::Savings),
                1_000_000,
            ),
        )
        .unwrap();
    let category = variable_category(&worker);
    for m in (1..=8u32).rev() {
        let date = today.checked_sub_months(chrono::Months::new(m)).unwrap();
        plant_categorized(&worker, checking_id, -50_000, date, category);
    }

    let conn = worker.read_connection().unwrap();
    let multi = forecast::compute_by_account(&conn, as_of, 180, &[]).unwrap();

    let checking_series = multi
        .accounts
        .iter()
        .find(|a| a.account_id == Some(checking_id.as_uuid()))
        .expect("checking series");
    let savings_series = multi
        .accounts
        .iter()
        .find(|a| a.account_id == Some(savings_id.as_uuid()))
        .expect("savings series");

    // The spending account carries a cone that widens over the horizon and stays ordered.
    let ck = &checking_series.days;
    let width = |i: usize| ck[i].closing.p90.minor_units() - ck[i].closing.p10.minor_units();
    let last = ck.len() - 1;
    assert!(width(last) > 0, "the spending account has a cone");
    assert!(width(last) >= width(30), "the cone widens over the horizon");
    for d in ck {
        assert!(d.closing.p10.minor_units() <= d.closing.p50.minor_units());
        assert!(d.closing.p50.minor_units() <= d.closing.p90.minor_units());
    }
    // The no-spend account stays a deterministic line (collapsed band).
    for d in &savings_series.days {
        assert_eq!(d.closing.p10.minor_units(), d.closing.p50.minor_units());
        assert_eq!(d.closing.p50.minor_units(), d.closing.p90.minor_units());
    }

    // Deterministic: identical inputs → byte-identical cones.
    let again = forecast::compute_by_account(&conn, as_of, 180, &[]).unwrap();
    assert_eq!(multi, again, "per-account cones are deterministic");
}

/// Build a vault with one checking account funding one credit card (given repayment philosophy),
/// 3 months of categorized card spend, and optionally a recorded card-estimator MAPE, then return
/// the paying account's cone width `(early, late)` over a 120-day projection. Checking has no cash
/// spend of its own, so any width is purely the card lump (personal-cfo-4d8.27.5.7.3).
fn card_lump_checking_widths(philosophy: RepaymentPhilosophy, with_mape: bool) -> (i64, i64) {
    let (_dir, worker) = worker();
    let checking_id = AccountId::new();
    let card = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(checking_id)),
                opening_balance: Some(Money::new(5_000_000, Currency::Usd)),
            },
        )
        .unwrap();
    create_role_account(&worker, card, "Visa", CashflowRole::CreditFacility);
    let mut terms = sample_debt_terms(Some(checking_id));
    terms.repayment_philosophy = philosophy;
    worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: card,
                terms,
            },
        )
        .unwrap();
    // 3 months of $200/mo categorized spend ON THE CARD → a non-zero projected variable charge.
    let today = NaiveDate::from_ymd_opt(2026, 7, 2).unwrap();
    let category = variable_category(&worker);
    for m in 1..=3u32 {
        let date = today.checked_sub_months(chrono::Months::new(m)).unwrap();
        plant_categorized(&worker, card, -20_000, date, category);
    }
    let conn = worker.read_connection().unwrap();
    if with_mape {
        // A recorded card-estimator MAPE (15%) — the seam that sizes the lump.
        conn.execute(
            "INSERT OR REPLACE INTO forecast_backtest_results
                (id, model_id, as_of_date, horizon_days, metric_type, score_bps, sample_size, created_at)
             VALUES (?1, 'card_statement_estimator_v1', ?2, 30, 'mape', 1500, 5, ?2)",
            rusqlite::params![Uuid::now_v7(), today.to_string()],
        )
        .unwrap();
    }
    let as_of = today.and_hms_opt(12, 0, 0).unwrap().and_utc();
    let multi = forecast::compute_by_account(&conn, as_of, 120, &[]).unwrap();
    let again = forecast::compute_by_account(&conn, as_of, 120, &[]).unwrap();
    assert_eq!(
        multi, again,
        "per-account cones (incl. card lumps) are deterministic"
    );
    let ck = multi
        .accounts
        .iter()
        .find(|a| a.account_id == Some(checking_id.as_uuid()))
        .expect("checking series");
    let width =
        |i: usize| ck.days[i].closing.p90.minor_units() - ck.days[i].closing.p10.minor_units();
    (width(5), width(ck.days.len() - 1))
}

/// personal-cfo-4d8.27.5.7.3: a full-payer card's variable statement uncertainty lands as a lump
/// on the paying account's cone at each future payment date; a revolver or a missing MAPE injects
/// nothing (revolvers are the path-dependent MC follow-up).
#[test]
fn full_payer_card_lump_widens_the_paying_accounts_cone() {
    // Full-payer + a recorded card MAPE → the paying account's cone widens by the horizon end
    // (no width yet before the first card payment, since checking has no cash cone of its own).
    let (early, late) = card_lump_checking_widths(RepaymentPhilosophy::PayStatementBalance, true);
    assert_eq!(early, 0, "no width before the first card payment date");
    assert!(
        late > 0,
        "a full-payer card lump widens the paying account's cone"
    );

    // No recorded card MAPE → nothing to size the spread → no lump.
    let (_e, late_no_mape) =
        card_lump_checking_widths(RepaymentPhilosophy::PayStatementBalance, false);
    assert_eq!(late_no_mape, 0, "no card MAPE → no lump");

    // A revolver (minimum payer) → no analytic lump (path-dependent, deferred to the MC follow-up).
    let (_e2, late_revolver) = card_lump_checking_widths(RepaymentPhilosophy::PayMinimum, true);
    assert_eq!(
        late_revolver, 0,
        "a revolver injects no analytic lump (deferred to the MC follow-up)"
    );
}

/// personal-cfo-4d8.27.5.2 (cf-history): the realized balance history folds backward from
/// today's balance over the ledger postings and is clamped to the account's earliest real data
/// (never fabricating a balance before the account had activity).
#[test]
fn cash_flow_history_folds_realized_balances_backward() {
    use chrono::TimeZone;
    let ymd = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();

    let (_dir, worker) = worker();
    let checking_id = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(checking_id)),
                opening_balance: None,
            },
        )
        .unwrap();
    // Controlled dated postings: a deposit opens the account, a spend, then another deposit.
    let plant = |cents: i64, date: NaiveDate| {
        worker
            .dispatch(
                meta(),
                WriteCommand::RecordTransaction {
                    transaction_id: TransactionId::new(),
                    account_id: checking_id,
                    amount: Money::new(cents, Currency::Usd),
                    occurred_at: date.and_hms_opt(12, 0, 0).unwrap().and_utc(),
                },
            )
            .unwrap();
    };
    plant(100_000, ymd(2026, 6, 1));
    plant(-30_000, ymd(2026, 6, 6));
    plant(20_000, ymd(2026, 6, 16));

    let as_of = Utc.with_ymd_and_hms(2026, 7, 1, 12, 0, 0).unwrap();
    let conn = worker.read_connection().unwrap();
    let hist = forecast::compute_cash_flow_history(&conn, as_of, 90).unwrap();

    let series = hist
        .accounts
        .iter()
        .find(|a| a.account_id == checking_id.as_uuid())
        .expect("checking history");
    let at = |d: NaiveDate| {
        series
            .days
            .iter()
            .find(|x| x.date == d)
            .unwrap_or_else(|| panic!("no history for {d}"))
            .closing_minor
    };
    assert_eq!(at(ymd(2026, 7, 1)), 90_000, "today = full posting sum");
    assert_eq!(at(ymd(2026, 6, 1)), 100_000, "the opening deposit day");
    assert_eq!(
        at(ymd(2026, 6, 10)),
        70_000,
        "after the -30k spend, before the +20k"
    );
    assert_eq!(at(ymd(2026, 6, 20)), 90_000, "after the +20k deposit");
    // Honesty clamp: nothing before the account's first real posting.
    assert_eq!(
        series.days.first().unwrap().date,
        ymd(2026, 6, 1),
        "clamped to earliest data"
    );
    assert_eq!(hist.start_date, ymd(2026, 6, 1));
    assert_eq!(hist.end_date, ymd(2026, 7, 1));

    // A credit card is a spending account too (the Account Detail owed-balance history):
    // its series appears with tier "card", stored-signed (negative when owed).
    let card_id = AccountId::new();
    create_role_account(&worker, card_id, "Venture X", CashflowRole::CreditFacility);
    worker
        .dispatch(
            meta(),
            WriteCommand::RecordTransaction {
                transaction_id: TransactionId::new(),
                account_id: card_id,
                amount: Money::new(-40_000, Currency::Usd),
                occurred_at: ymd(2026, 6, 10).and_hms_opt(12, 0, 0).unwrap().and_utc(),
            },
        )
        .unwrap();
    let with_card = forecast::compute_cash_flow_history(&conn, as_of, 90).unwrap();
    let card_series = with_card
        .accounts
        .iter()
        .find(|a| a.account_id == card_id.as_uuid())
        .expect("card history series");
    assert_eq!(card_series.tier, "card");
    assert_eq!(
        card_series.days.last().unwrap().closing_minor,
        -40_000,
        "card history is stored-signed (negative owed)"
    );
    assert_eq!(
        card_series.days.first().unwrap().date,
        ymd(2026, 6, 10),
        "card series clamps to its earliest posting"
    );

    // A posting dated AFTER today must not shift the history — it hasn't happened yet, so every
    // historical day stays `Σ(postings ≤ D)` (adversarial-review F1: bal_today counts it, so the
    // fold seeds `running` with the after-today sum to cancel it out).
    plant(-9_999, ymd(2026, 8, 1));
    let hist2 = forecast::compute_cash_flow_history(&conn, as_of, 90).unwrap();
    let s2 = hist2
        .accounts
        .iter()
        .find(|a| a.account_id == checking_id.as_uuid())
        .unwrap();
    let at2 = |d: NaiveDate| s2.days.iter().find(|x| x.date == d).unwrap().closing_minor;
    assert_eq!(
        at2(ymd(2026, 7, 1)),
        90_000,
        "a future-dated posting must not shift today's history"
    );
    assert_eq!(at2(ymd(2026, 6, 10)), 70_000, "…nor any past day");

    // Deterministic: two fresh computes over the same state are identical.
    assert_eq!(
        forecast::compute_cash_flow_history(&conn, as_of, 90).unwrap(),
        forecast::compute_cash_flow_history(&conn, as_of, 90).unwrap()
    );
}

#[test]
fn per_account_series_reconcile_to_the_aggregate_with_attribution() {
    use chrono::TimeZone;
    let (_dir, worker) = worker();
    let checking = AccountId::new();
    let savings = AccountId::new();
    worker
        .dispatch(
            meta(),
            liquid_subtype_cmd(
                checking,
                "Checking",
                Some(AccountSubtype::Checking),
                100_000,
            ),
        )
        .unwrap();
    worker
        .dispatch(
            meta(),
            liquid_subtype_cmd(savings, "Savings", Some(AccountSubtype::Savings), 500_000),
        )
        .unwrap();
    // Income deposits to checking; the bill autopays from checking.
    worker.dispatch(meta(), income_cmd(Some(checking))).unwrap();
    worker.dispatch(meta(), bill_cmd(Some(checking))).unwrap();
    // An account-agnostic manual entry → the Unallocated series.
    worker
        .record_manual_entry(
            Uuid::now_v7(),
            Money::new(50_000, Currency::Usd),
            NaiveDate::from_ymd_opt(2026, 7, 15).unwrap(),
            "Gift",
            None,
        )
        .unwrap();

    let conn = worker.read_connection().unwrap();
    let as_of = Utc.with_ymd_and_hms(2026, 6, 1, 12, 0, 0).unwrap();
    let aggregate = forecast::compute(&conn, as_of, 90, &[]).unwrap();
    let multi = forecast::compute_by_account(&conn, as_of, 90, &[]).unwrap();

    // The reconciliation invariant: per-account closings (incl. Unallocated)
    // sum to the aggregate, every day (ADR 0026 §12).
    assert_eq!(aggregate.days.len(), 90);
    for (i, agg_day) in aggregate.days.iter().enumerate() {
        let summed: i64 = multi
            .accounts
            .iter()
            .map(|s| s.days[i].closing.p50.minor_units())
            .sum();
        assert_eq!(summed, agg_day.closing.p50.minor_units(), "day {i}");
    }

    // Attribution lands in the right series.
    let series = |id: Option<AccountId>| {
        multi
            .accounts
            .iter()
            .find(|s| s.account_id == id.map(AccountId::as_uuid))
            .unwrap()
    };
    let event_count = |s: &AccountSeriesView| s.days.iter().map(|d| d.events.len()).sum::<usize>();
    assert!(event_count(series(Some(checking))) > 0); // income + bill
    assert_eq!(event_count(series(Some(savings))), 0); // nothing attributed
    assert_eq!(event_count(series(None)), 1); // the manual entry

    // Tiers follow the subtypes (ADR 0028).
    assert_eq!(series(Some(checking)).tier, "spendable");
    assert_eq!(series(Some(savings)).tier, "reserve");
    assert_eq!(series(None).tier, "unallocated");

    // The `net` group equals the aggregate day by day.
    let net = multi.groups.iter().find(|g| g.tier == "net").unwrap();
    for (i, agg_day) in aggregate.days.iter().enumerate() {
        assert_eq!(net.closings[i].closing.p50, agg_day.closing.p50);
    }
}

#[test]
fn cash_availability_components_invariant_committed_and_floor() {
    use chrono::TimeZone;
    let (_dir, worker) = worker();
    let checking = AccountId::new();
    worker
        .dispatch(
            meta(),
            liquid_subtype_cmd(
                checking,
                "Checking",
                Some(AccountSubtype::Checking),
                500_000,
            ),
        )
        .unwrap();
    // A bill autopaying from checking, due inside the 30-day committed window.
    worker.dispatch(meta(), bill_cmd(Some(checking))).unwrap();

    let conn = worker.read_connection().unwrap();
    // Fixed clock so the monthly rent (due 2026-07-01) lands in [as_of, +30d).
    let as_of = Utc.with_ymd_and_hms(2026, 6, 20, 12, 0, 0).unwrap();
    let avail = forecast::compute_cash_availability(&conn, as_of, 100_000).unwrap();

    assert_eq!(avail.accounts.len(), 1);
    let acct = &avail.accounts[0];
    assert_eq!(acct.account_id, checking.as_uuid());
    assert_eq!(acct.ledger, Money::new(500_000, Currency::Usd));
    assert_eq!(acct.pending, Money::new(0, Currency::Usd)); // manual mode
    assert_eq!(acct.available, Money::new(500_000, Currency::Usd));
    assert_eq!(acct.committed, Money::new(180_000, Currency::Usd)); // the rent
    assert_eq!(acct.headroom, Money::new(320_000, Currency::Usd)); // 500k - 180k
                                                                   // The per-account invariant (ADR 0029): ledger >= available >= headroom.
    assert!(acct.ledger.minor_units() >= acct.available.minor_units());
    assert!(acct.available.minor_units() >= acct.headroom.minor_units());

    // Net rollup + floor: net headroom 320k is above a 100k floor.
    assert_eq!(avail.net_available, Money::new(500_000, Currency::Usd));
    assert_eq!(avail.net_committed, Money::new(180_000, Currency::Usd));
    assert_eq!(avail.net_headroom, Money::new(320_000, Currency::Usd));
    assert_eq!(avail.floor, Money::new(100_000, Currency::Usd));
    assert!(!avail.below_floor);

    // A higher floor than the headroom flips the forward-looking alert.
    let strict = forecast::compute_cash_availability(&conn, as_of, 400_000).unwrap();
    assert!(strict.below_floor, "headroom 320k is below a 400k floor");
}

#[test]
fn forecast_readiness_reflects_coverage_freshness_and_explained() {
    use chrono::TimeZone;
    // Fixed clock so a balance asserted "that day" reads as fresh.
    let as_of = Utc.with_ymd_and_hms(2026, 6, 20, 12, 0, 0).unwrap();
    let today = NaiveDate::from_ymd_opt(2026, 6, 20).unwrap();
    let five_hundred = Money::new(500_000, Currency::Usd);

    // 1. An empty vault (no liquid account) is a hard 0.
    let (_d0, empty) = worker();
    let r0 =
        forecast::compute_forecast_readiness(&empty.read_connection().unwrap(), as_of).unwrap();
    assert_eq!(r0.score, 0);
    assert_eq!(r0.factors.len(), 7); // coverage, freshness, explained, categorization, spending_history, recurrence_actuals, backtest_mape
    assert_eq!(readiness_factor(&r0, "coverage").score, 0);

    // 2. Account only, balance asserted today, no income/bills, no transactions.
    //    The asserted balance is a full unexplained plug (no postings back it),
    //    yet the explained factor stays NEUTRAL (100) — the assert-only manual
    //    workflow (ADR 0027) is never penalized. Freshness is full (asserted
    //    today); coverage is partial (income + bills missing).
    let (_d1, solo) = worker();
    let acct = AccountId::new();
    solo.dispatch(meta(), liquid_account_no_opening(acct))
        .unwrap();
    solo.record_balance_assertion(Uuid::now_v7(), acct, five_hundred, today)
        .unwrap();
    let r1 = forecast::compute_forecast_readiness(&solo.read_connection().unwrap(), as_of).unwrap();
    assert!(r1.score > 0);
    assert_eq!(readiness_factor(&r1, "freshness").score, 100);
    assert_eq!(readiness_factor(&r1, "explained").score, 100);
    assert!(readiness_factor(&r1, "coverage").score < 100);

    // 3. Full coverage (account + income + bills) with a fresh balance scores high.
    let (_d2, full) = worker();
    let acct2 = AccountId::new();
    full.dispatch(meta(), liquid_account_no_opening(acct2))
        .unwrap();
    full.dispatch(meta(), income_cmd(Some(acct2))).unwrap();
    full.dispatch(meta(), bill_cmd(Some(acct2))).unwrap();
    full.record_balance_assertion(Uuid::now_v7(), acct2, five_hundred, today)
        .unwrap();
    let r2 = forecast::compute_forecast_readiness(&full.read_connection().unwrap(), as_of).unwrap();
    assert_eq!(readiness_factor(&r2, "coverage").score, 100);
    assert!(
        r2.score >= 70,
        "full fresh vault should score high, got {}",
        r2.score
    );
    assert!(r2.score > r1.score);

    // 4. A balance asserted > 6 weeks ago drives freshness to 0 and lowers the score.
    let (_d3, stale) = worker();
    let acct3 = AccountId::new();
    stale
        .dispatch(meta(), liquid_account_no_opening(acct3))
        .unwrap();
    stale.dispatch(meta(), income_cmd(Some(acct3))).unwrap();
    stale.dispatch(meta(), bill_cmd(Some(acct3))).unwrap();
    let stale_date = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(); // ~111 days before
    stale
        .record_balance_assertion(Uuid::now_v7(), acct3, five_hundred, stale_date)
        .unwrap();
    let r3 =
        forecast::compute_forecast_readiness(&stale.read_connection().unwrap(), as_of).unwrap();
    assert_eq!(readiness_factor(&r3, "freshness").score, 0);
    assert!(r3.score < r2.score, "a stale balance lowers readiness");
}

/// personal-cfo-4d8.27.1.1: an account whose balance came from an opening balance
/// (never an explicit assertion) reads as FRESH the day it is created — not the old
/// synthetic 45-day-stale floor; an account with no balance evidence at all reads
/// as needing a balance; and the freshness account set matches the forecast's
/// (archived-but-still-projected accounts drag freshness as they anchor the forecast).
#[test]
fn forecast_readiness_freshness_counts_opening_balance_and_matches_the_forecast_set() {
    // Real clock: the opening-balance posting is dated "now", so aligning the
    // horizon (`as_of`) to now keeps the anchor and the evaluation on the same day.
    let as_of = Utc::now();

    // 1. A liquid account with an opening balance and NO manual assertion is fresh
    //    (anchored by its opening posting, not the old 45-day synthetic floor).
    let (_d0, w0) = worker();
    let a0 = AccountId::new();
    w0.dispatch(meta(), liquid_account_cmd(a0, Currency::Usd, 500_000))
        .unwrap();
    let r0 = forecast::compute_forecast_readiness(&w0.read_connection().unwrap(), as_of).unwrap();
    assert!(
        readiness_factor(&r0, "freshness").score >= 90,
        "an opening-balance account is fresh the day it is created, got {}",
        readiness_factor(&r0, "freshness").score
    );

    // 2. A liquid account with no opening balance, no assertion, and no postings has
    //    no balance evidence at all → prompt to set balances (0), not vacuously fresh.
    let (_d1, w1) = worker();
    let bare = AccountId::new();
    w1.dispatch(meta(), liquid_account_no_opening(bare))
        .unwrap();
    let r1 = forecast::compute_forecast_readiness(&w1.read_connection().unwrap(), as_of).unwrap();
    assert_eq!(
        readiness_factor(&r1, "freshness").score,
        0,
        "a liquid account with no balance evidence reads as needing a balance"
    );

    // 3. Freshness covers exactly the accounts the forecast's starting balance sums.
    //    Under ADR 0056 that set is the ACTIVE liquid accounts, so an archived account
    //    drops out of both together: its stale 2020 balance no longer drags freshness,
    //    because it no longer anchors the forecast either.
    //
    //    The invariant this protects is the COUPLING, not the direction. It previously
    //    asserted the opposite — archived-still-anchors, archived-still-drags — and
    //    flipping only one side is what it exists to catch.
    let (_d2, w2) = worker();
    let fresh = AccountId::new();
    w2.dispatch(meta(), liquid_account_cmd(fresh, Currency::Usd, 500_000))
        .unwrap();
    let old = AccountId::new();
    w2.dispatch(meta(), liquid_account_no_opening(old)).unwrap();
    let long_ago = NaiveDate::from_ymd_opt(2020, 1, 1).unwrap();
    w2.record_balance_assertion(
        Uuid::now_v7(),
        old,
        Money::new(100_000, Currency::Usd),
        long_ago,
    )
    .unwrap();
    w2.dispatch(meta(), WriteCommand::ArchiveAccount(old))
        .unwrap();
    let r2 = forecast::compute_forecast_readiness(&w2.read_connection().unwrap(), as_of).unwrap();
    assert!(
        readiness_factor(&r2, "freshness").score > 0,
        "an archived liquid account has left the forecast (ADR 0056), so its 2020 balance \
         must no longer drag freshness — the household cannot fix a staleness caused by an \
         account it deliberately retired",
    );

    // …and it left the PROJECTION in the same breath. Asserting both in one fixture is
    // the point: the failure mode is one side moving without the other.
    let start = forecast::compute(&w2.read_connection().unwrap(), as_of, 30, &[])
        .unwrap()
        .days
        .first()
        .unwrap()
        .closing
        .p50
        .minor_units();
    assert_eq!(
        start, 500_000,
        "only the active account's 500,000 anchors the projection; the archived \
         account's 100,000 is gone",
    );
}

/// nxgx (ADR 0026 §13a): the spending-history factor IS the Layer-2 band's activation
/// predicate, surfaced — and categorized spend raises readiness.
#[test]
fn forecast_readiness_spending_factors_track_categorized_history() {
    use chrono::TimeZone;
    let as_of = Utc.with_ymd_and_hms(2026, 6, 20, 12, 0, 0).unwrap();
    let today = as_of.date_naive();

    let (_dir, worker) = worker();
    let account_id = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(account_id)),
                opening_balance: Some(Money::new(5_000_000, Currency::Usd)),
            },
        )
        .unwrap();
    let category = variable_category(&worker);

    // Before any spending: the projected range is locked.
    let before =
        forecast::compute_forecast_readiness(&worker.read_connection().unwrap(), as_of).unwrap();
    assert_eq!(readiness_factor(&before, "spending_history").score, 0);

    // Eight months of categorized variable spend → the range unlocks (≥ 6-month gate).
    for m in (1..=8u32).rev() {
        let date = today.checked_sub_months(chrono::Months::new(m)).unwrap();
        plant_categorized(&worker, account_id, -50_000, date, category);
    }
    let after =
        forecast::compute_forecast_readiness(&worker.read_connection().unwrap(), as_of).unwrap();
    assert_eq!(
        readiness_factor(&after, "spending_history").score,
        100,
        "8 months of categorized spend should unlock the range"
    );
    assert_eq!(
        readiness_factor(&after, "categorization").score,
        100,
        "all planted spend is categorized"
    );
    assert!(after.score > before.score, "spending data raises readiness");
}

/// personal-cfo-4d8.27.1.2: the spending-history indicator counts categorized variable spend
/// on CREDIT-CARD accounts (interim ADR 0050 fix), so a card-based household is not stuck
/// reading 0 despite hundreds of categorized transactions.
#[test]
fn forecast_readiness_spending_history_counts_credit_card_spend() {
    use chrono::TimeZone;
    let as_of = Utc.with_ymd_and_hms(2026, 6, 20, 12, 0, 0).unwrap();
    let today = as_of.date_naive();

    let (_dir, worker) = worker();
    // A liquid account gives the forecast a starting balance (clears the hard gate); it holds
    // no spend — all discretionary spending happens on the card below.
    let checking_id = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(checking_id)),
                opening_balance: Some(Money::new(5_000_000, Currency::Usd)),
            },
        )
        .unwrap();
    let card = AccountId::new();
    create_role_account(&worker, card, "Venture X", CashflowRole::CreditFacility);
    let category = variable_category(&worker);

    // No card spend yet → the range is locked.
    let before =
        forecast::compute_forecast_readiness(&worker.read_connection().unwrap(), as_of).unwrap();
    assert_eq!(readiness_factor(&before, "spending_history").score, 0);

    // Eight months of categorized variable spend ON THE CARD. Before the interim fix this read
    // 0 (the indicator was liquid-only); now the card months are credited.
    for m in (1..=8u32).rev() {
        let date = today.checked_sub_months(chrono::Months::new(m)).unwrap();
        plant_categorized(&worker, card, -50_000, date, category);
    }
    let after =
        forecast::compute_forecast_readiness(&worker.read_connection().unwrap(), as_of).unwrap();
    let sh = readiness_factor(&after, "spending_history");
    assert_eq!(
        sh.score, 100,
        "categorized variable spend on a credit card should count toward spending history"
    );
    // ...but the liquid-only band is NOT active (all spend is on the card), so the app must not
    // announce a range that is not drawn: the detail must not claim active, and the
    // forecast_band unlock notice must not fire (adversarial review of personal-cfo-4d8.27.1.2).
    assert_ne!(
        sh.detail, "Projected spending range is active.",
        "the range is not active when all spend is card-only"
    );
    let conn = worker.read_connection().unwrap();
    assert!(
        forecast::pending_capability_unlocks(&conn, as_of)
            .unwrap()
            .iter()
            .all(|c| c.key != "forecast_band"),
        "the forecast_band notice must not fire while the liquid-only band is inactive"
    );
}

/// egon (ADR 0026 §10): the capability-unlock notice fires once — pending while the band
/// is active and unacknowledged, gone after acknowledgement, absent while it's locked.
#[test]
fn capability_unlock_fires_once_when_the_band_activates() {
    use chrono::TimeZone;
    let as_of = Utc.with_ymd_and_hms(2026, 6, 20, 12, 0, 0).unwrap();
    let today = as_of.date_naive();

    let (_dir, worker) = worker();
    let account_id = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(account_id)),
                opening_balance: Some(Money::new(5_000_000, Currency::Usd)),
            },
        )
        .unwrap();
    let category = variable_category(&worker);

    // Band locked (no spending history) → nothing to announce.
    {
        let conn = worker.read_connection().unwrap();
        assert!(forecast::pending_capability_unlocks(&conn, as_of)
            .unwrap()
            .is_empty());
    }

    // Enough categorized history → the band activates → the unlock is pending.
    for m in (1..=8u32).rev() {
        let date = today.checked_sub_months(chrono::Months::new(m)).unwrap();
        plant_categorized(&worker, account_id, -50_000, date, category);
    }
    {
        let conn = worker.read_connection().unwrap();
        let pending = forecast::pending_capability_unlocks(&conn, as_of).unwrap();
        // Key-scoped: the same fixture also satisfies other capabilities' predicates
        // (its months of postings give cash_flow_history its 30 days), so assert on
        // the band's entry rather than the whole list (personal-cfo-4d8.27.5.5).
        let band = pending
            .iter()
            .find(|c| c.key == "forecast_band")
            .expect("band unlock pending");
        assert_eq!(band.factor_key, "spending_history");
    }

    // Acknowledge → fires exactly once.
    worker
        .acknowledge_capability(&meta(), "forecast_band")
        .unwrap();
    {
        let conn = worker.read_connection().unwrap();
        assert!(
            forecast::pending_capability_unlocks(&conn, as_of)
                .unwrap()
                .iter()
                .all(|c| c.key != "forecast_band"),
            "an acknowledged capability is no longer pending"
        );
    }
}

/// personal-cfo-4d8.27.5.5: the cash_flow_history capability announces itself once the
/// earliest LIQUID posting is ≥30 days old — an announcement, not a gate. Card-only
/// history does not trigger it, and acknowledging dismisses it for good.
#[test]
fn cash_flow_history_capability_announces_after_thirty_days_of_liquid_history() {
    use chrono::TimeZone;
    let as_of = Utc.with_ymd_and_hms(2026, 6, 20, 12, 0, 0).unwrap();
    let today = as_of.date_naive();

    let (_dir, worker) = worker();
    let checking_id = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(checking_id)),
                opening_balance: None,
            },
        )
        .unwrap();
    let card = AccountId::new();
    create_role_account(&worker, card, "Visa", CashflowRole::CreditFacility);
    let plant = |account: AccountId, cents: i64, days_ago: u64| {
        worker
            .dispatch(
                meta(),
                WriteCommand::RecordTransaction {
                    transaction_id: TransactionId::new(),
                    account_id: account,
                    amount: Money::new(cents, Currency::Usd),
                    occurred_at: today
                        .checked_sub_days(chrono::Days::new(days_ago))
                        .unwrap()
                        .and_hms_opt(12, 0, 0)
                        .unwrap()
                        .and_utc(),
                },
            )
            .unwrap();
    };
    let has_history_unlock = || {
        let conn = worker.read_connection().unwrap();
        forecast::pending_capability_unlocks(&conn, as_of)
            .unwrap()
            .iter()
            .any(|c| c.key == "cash_flow_history")
    };

    // 10 days of liquid history + 60 days of CARD history → not yet: the predicate is
    // liquid-only (the history chart is the liquid chart).
    plant(checking_id, -5_000, 10);
    plant(card, -20_000, 60);
    assert!(
        !has_history_unlock(),
        "10 liquid days (card history does not count) must not announce"
    );

    // A liquid posting 40 days back crosses the 30-day depth → the notice is pending.
    plant(checking_id, 100_000, 40);
    assert!(has_history_unlock(), "40 days of liquid history announces");

    // Acknowledge → dismissed for good.
    worker
        .acknowledge_capability(&meta(), "cash_flow_history")
        .unwrap();
    assert!(!has_history_unlock(), "acknowledged → no longer pending");
}

/// 5ie.9: confirming a bill occurrence early posts a real outflow, moves the balance, and
/// suppresses exactly that occurrence from the forecast (no double-count) — even when the
/// confirm is made further ahead than the ±7-day instance-linking tolerance. Re-confirm is a
/// no-op; future occurrences still project.
#[test]
fn confirming_a_bill_early_posts_and_suppresses_without_double_count() {
    let (_dir, worker) = worker();
    let checking = AccountId::new();
    create_role_account(&worker, checking, "Checking", CashflowRole::LiquidCash);
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            checking,
            Money::new(500_000, Currency::Usd),
            NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
        )
        .unwrap();

    let bill = RecurringEventId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateRecurringBill {
                event_id: bill,
                contract_id: BillContractId::new(),
                name: "Rent".to_owned(),
                amount: Money::new(20_000, Currency::Usd),
                bill_type: "rent_mortgage".to_owned(),
                frequency: Frequency::Monthly,
                anchor: NaiveDate::from_ymd_opt(2026, 7, 20).unwrap(),
                autopay_account_id: Some(checking),
                description: None,
                source_merchant_key: None,
                category_id: None,
                tag_ids: Vec::new(),
            },
        )
        .unwrap();

    let confirm = |worker: &DbWorker| {
        worker
            .dispatch(
                meta(),
                WriteCommand::ConfirmObligationEarly {
                    recurring_event_id: bill,
                    scheduled_date: NaiveDate::from_ymd_opt(2026, 7, 20).unwrap(),
                    actual_amount: Money::new(20_000, Currency::Usd),
                    actual_date: NaiveDate::from_ymd_opt(2026, 7, 2)
                        .unwrap()
                        .and_hms_opt(12, 0, 0)
                        .unwrap()
                        .and_utc(),
                    paying_account_id: checking,
                },
            )
            .unwrap();
    };

    // Confirm the 2026-07-20 occurrence on 2026-07-02 — 18 days early, well past the 7-day
    // linking tolerance.
    confirm(&worker);
    assert_eq!(
        worker.account_balance(checking).unwrap().unwrap(),
        Money::new(480_000, Currency::Usd),
        "the outflow posted",
    );

    let as_of = NaiveDate::from_ymd_opt(2026, 7, 2)
        .unwrap()
        .and_hms_opt(9, 0, 0)
        .unwrap()
        .and_utc();
    let conn = worker.read_connection().unwrap();
    let forecast = crate::forecast::compute(&conn, as_of, 90, &[]).unwrap();
    let rent_dates: Vec<NaiveDate> = forecast
        .days
        .iter()
        .flat_map(|d| d.events.iter().map(move |e| (d.date, e.source_event_id)))
        .filter(|(_, id)| *id == bill.as_uuid())
        .map(|(date, _)| date)
        .collect();
    assert!(
        !rent_dates.contains(&NaiveDate::from_ymd_opt(2026, 7, 20).unwrap()),
        "the confirmed 7/20 occurrence is suppressed",
    );
    assert!(
        rent_dates.contains(&NaiveDate::from_ymd_opt(2026, 8, 20).unwrap()),
        "future occurrences still project",
    );
    // Starting balance already reflects the posting — so across 7/2..7/20 there is exactly
    // one $200 hit, not two.
    assert_eq!(
        forecast.starting_balance,
        Money::new(480_000, Currency::Usd)
    );

    // …and the "needs confirmation" queue must agree with that suppression
    // (personal-cfo-4d8.27.7.6, ADR 0058 §2). THIS is the assertion that keeps the queue
    // from manufacturing a double-payment: after 7/20 has passed, the instance row is
    // still `status = 'scheduled'` because the confirm landed 18 days early and outside
    // the ±7-day link tolerance (personal-cfo-vn6b). A status-driven queue would list the
    // occurrence as unresolved, the user would confirm it again, and a SECOND $200 would
    // post. Reading `confirmed_obligations` is what makes that unreachable.
    {
        let after = NaiveDate::from_ymd_opt(2026, 7, 25).unwrap();
        let mut c = worker.read_connection().unwrap();
        crate::recurring_instances::rebuild(&mut c, after).unwrap();

        let stale_status: String = c
            .query_row(
                "SELECT status FROM recurring_event_instances
                  WHERE recurring_event_id = ?1 AND scheduled_date = '2026-07-20'",
                [bill.as_uuid()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            stale_status, "scheduled",
            "vn6b: the far-early confirm leaves the instance status stale — the trap this \
             query has to avoid",
        );

        let pending = crate::recurring_instances::read_unconfirmed_past_due(&c, after).unwrap();
        assert!(
            !pending.iter().any(|o| o.scheduled_date == "2026-07-20"),
            "the confirmed occurrence is NOT in the queue despite its stale status",
        );
    }

    // The positive half — without it the assertion above could pass on a query that always
    // returns nothing, which is the vacuous-guard trap. A bill nobody confirmed DOES
    // surface, with the figures the section needs to describe it.
    {
        let unpaid = RecurringEventId::new();
        worker
            .dispatch(
                meta(),
                WriteCommand::CreateRecurringBill {
                    event_id: unpaid,
                    contract_id: BillContractId::new(),
                    name: "Gym".to_owned(),
                    amount: Money::new(4_500, Currency::Usd),
                    bill_type: "subscription".to_owned(),
                    frequency: Frequency::Monthly,
                    anchor: NaiveDate::from_ymd_opt(2026, 7, 10).unwrap(),
                    autopay_account_id: Some(checking),
                    description: None,
                    source_merchant_key: None,
                    category_id: None,
                    tag_ids: Vec::new(),
                },
            )
            .unwrap();

        let after = NaiveDate::from_ymd_opt(2026, 7, 25).unwrap();
        let mut c = worker.read_connection().unwrap();
        crate::recurring_instances::rebuild(&mut c, after).unwrap();
        let pending = crate::recurring_instances::read_unconfirmed_past_due(&c, after).unwrap();

        let gym = pending
            .iter()
            .find(|o| o.scheduled_date == "2026-07-10")
            .expect("an unpaid past-due bill is in the queue");
        assert_eq!(gym.name, "Gym", "carries the bill's name for the surface");
        assert_eq!(gym.expected_amount_minor, 4_500);
        assert_eq!(gym.days_overdue, 15, "7/10 → 7/25");
        // Still disjoint from the confirmed one.
        assert!(!pending.iter().any(|o| o.scheduled_date == "2026-07-20"));
    }

    // Re-confirming the same occurrence posts nothing new.
    confirm(&worker);
    assert_eq!(
        worker.account_balance(checking).unwrap().unwrap(),
        Money::new(480_000, Currency::Usd),
        "re-confirm is a no-op",
    );
}

/// 5ie.10: a bill entered today with a year-old anchor must not dump a year of spurious
/// past-due rows on Cash Flow. Uses real relative dates (`Utc::now()`), not the fictional
/// 2026-07 literals the rest of this file uses, because the bug and its fix are about
/// `recurring_events.created_at` (always stamped at real wall-clock time) versus `today` —
/// a fictional "today" that predates real "now" would make every occurrence look
/// pre-creation regardless of the fix.
#[test]
fn a_bill_created_today_with_a_year_old_anchor_shows_only_its_newest_occurrence() {
    let (_dir, worker) = worker();
    let today = Utc::now().date_naive();
    let anchor = today - chrono::Duration::days(370);
    let bill = RecurringEventId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateRecurringBill {
                event_id: bill,
                contract_id: BillContractId::new(),
                name: "Piano Lessons".to_owned(),
                amount: Money::new(11_250, Currency::Usd),
                bill_type: "other".to_owned(),
                frequency: Frequency::Monthly,
                anchor,
                autopay_account_id: None,
                description: None,
                source_merchant_key: None,
                category_id: None,
                tag_ids: Vec::new(),
            },
        )
        .unwrap();

    let mut c = worker.read_connection().unwrap();
    crate::recurring_instances::rebuild(&mut c, today).unwrap();
    let pending = crate::recurring_instances::read_unconfirmed_past_due(&c, today).unwrap();
    let this_bill: Vec<_> = pending
        .iter()
        .filter(|o| o.recurring_event_id == bill.as_uuid())
        .collect();

    assert_eq!(
        this_bill.len(),
        1,
        "a year-old anchor must not surface a year of past-due rows: {this_bill:?}",
    );
    // The one row that DOES surface is the most recent occurrence — the genuinely
    // actionable one, not an arbitrary survivor. A monthly cadence puts it within the
    // last ~31 days; a year-old anchor's earlier occurrences would be 300+ days overdue.
    assert!(
        this_bill[0].days_overdue < 32,
        "the surviving row is the newest occurrence, not one from a year back: {:?}",
        this_bill[0],
    );
}

/// 5ie.10 (review #1): confirming the one pre-creation row that surfaces must NOT walk the
/// next-older occurrence into its place. An earlier version of the fix computed "the bill's
/// newest occurrence" over only the still-unresolved candidate rows, so each confirm moved
/// that maximum one occurrence further back — a cooperative user who kept confirming what
/// the queue showed would eventually post a year of dated payments for occurrences the
/// household never tracked in the app. The corrected query fixes the "newest occurrence"
/// reference over the bill's full history (resolved or not), so it never moves. Confirms via
/// the real `ConfirmObligationEarly` path (the actual `MarkObligationPaid` control, ADR 0058
/// consequences) rather than inserting into `confirmed_obligations` directly, so the posting
/// side is exercised exactly as a user's confirm would trigger it.
#[test]
fn confirming_the_one_pre_creation_row_does_not_surface_the_next_older_one() {
    let (_dir, worker) = worker();
    let checking = AccountId::new();
    create_role_account(&worker, checking, "Checking", CashflowRole::LiquidCash);
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            checking,
            Money::new(500_000, Currency::Usd),
            Utc::now().date_naive(),
        )
        .unwrap();

    let today = Utc::now().date_naive();
    let anchor = today - chrono::Duration::days(370);
    let bill = RecurringEventId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateRecurringBill {
                event_id: bill,
                contract_id: BillContractId::new(),
                name: "Piano Lessons".to_owned(),
                amount: Money::new(11_250, Currency::Usd),
                bill_type: "other".to_owned(),
                frequency: Frequency::Monthly,
                anchor,
                autopay_account_id: Some(checking),
                description: None,
                source_merchant_key: None,
                category_id: None,
                tag_ids: Vec::new(),
            },
        )
        .unwrap();

    let mut c = worker.read_connection().unwrap();
    crate::recurring_instances::rebuild(&mut c, today).unwrap();
    let before = crate::recurring_instances::read_unconfirmed_past_due(&c, today).unwrap();
    let surfaced: Vec<_> = before
        .iter()
        .filter(|o| o.recurring_event_id == bill.as_uuid())
        .collect();
    assert_eq!(
        surfaced.len(),
        1,
        "sanity: exactly one row before any confirm"
    );
    let scheduled_date: NaiveDate = surfaced[0].scheduled_date.parse().unwrap();

    worker
        .dispatch(
            meta(),
            WriteCommand::ConfirmObligationEarly {
                recurring_event_id: bill,
                scheduled_date,
                actual_amount: Money::new(11_250, Currency::Usd),
                actual_date: today.and_hms_opt(9, 0, 0).unwrap().and_utc(),
                paying_account_id: checking,
            },
        )
        .unwrap();

    let mut c2 = worker.read_connection().unwrap();
    crate::recurring_instances::rebuild(&mut c2, today).unwrap();
    let after = crate::recurring_instances::read_unconfirmed_past_due(&c2, today).unwrap();
    let remaining: Vec<_> = after
        .iter()
        .filter(|o| o.recurring_event_id == bill.as_uuid())
        .collect();
    assert!(
        remaining.is_empty(),
        "confirming the one surfaced row must not walk an older pre-creation occurrence \
         into its place: {remaining:?}",
    );
}

/// 5ie.10: the fix must not cap a GENUINE multi-occurrence backlog that accrued after the
/// bill was created — ADR 0058's "never hide a genuine backlog" consequence still applies
/// once the household has actually been tracking the bill. `rebuild`'s `today` is
/// independently controllable from `created_at` (always real `Utc::now()`), so this
/// simulates "the household added the bill, then missed several weeks" by rebuilding at a
/// `today` well after the bill's real creation moment.
#[test]
fn a_genuine_post_creation_backlog_is_not_collapsed_to_one_row() {
    let (_dir, worker) = worker();
    let created_on = Utc::now().date_naive();
    let anchor = created_on + chrono::Duration::days(3);
    let bill = RecurringEventId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateRecurringBill {
                event_id: bill,
                contract_id: BillContractId::new(),
                name: "Weekly Cleaner".to_owned(),
                amount: Money::new(8_000, Currency::Usd),
                bill_type: "other".to_owned(),
                frequency: Frequency::Weekly,
                anchor,
                autopay_account_id: None,
                description: None,
                source_merchant_key: None,
                category_id: None,
                tag_ids: Vec::new(),
            },
        )
        .unwrap();

    // Four weekly occurrences after creation (anchor+3, +10, +17, +24), all still
    // genuinely unresolved by the time "today" is 25 days past creation.
    let much_later = created_on + chrono::Duration::days(25);
    let mut c = worker.read_connection().unwrap();
    crate::recurring_instances::rebuild(&mut c, much_later).unwrap();
    let pending = crate::recurring_instances::read_unconfirmed_past_due(&c, much_later).unwrap();
    let this_bill: Vec<_> = pending
        .iter()
        .filter(|o| o.recurring_event_id == bill.as_uuid())
        .collect();

    assert_eq!(
        this_bill.len(),
        4,
        "a genuine backlog accrued after creation must show in full, not collapse to \
         the newest row: {this_bill:?}",
    );
}

/// Construct "23:30 on `local_date`" in `tz`, expressed as the precise UTC instant
/// (DST-correct via `chrono-tz`'s IANA rules, rather than a hardcoded offset).
fn at_2330_local(tz: forecast_engine::Tz, local_date: NaiveDate) -> chrono::DateTime<Utc> {
    use chrono::TimeZone;
    tz.from_local_datetime(&local_date.and_hms_opt(23, 30, 0).unwrap())
        .single()
        .expect("23:30 is not a DST-transition instant for any date this suite uses")
        .with_timezone(&Utc)
}

/// personal-cfo-5ie.11: `household_today_at` resolves "today" via the household timezone,
/// not UTC — the already-accepted ADR 0021 §1 policy the kernel's various "today" call
/// sites now follow. Pins the exact scenario from the bug report: 23:30 Pacific is still
/// the SAME calendar day locally even though UTC has already rolled to the next one.
/// Anchored to real `Utc::now()`'s LA-local date (not a fictional literal), so this test
/// stays valid regardless of when it runs and regardless of DST.
#[test]
fn household_today_at_resolves_the_local_date_not_utc() {
    let (_dir, worker) = worker();
    let conn = worker.read_connection().unwrap();
    conn.execute(
        "UPDATE vault_metadata SET household_timezone = ?1 WHERE singleton = 1",
        ["America/Los_Angeles"],
    )
    .unwrap();
    let tz = crate::forecast::read_household_tz(&conn).unwrap();

    let la_today = Utc::now().with_timezone(&tz).date_naive();
    let as_of = at_2330_local(tz, la_today);
    assert_eq!(
        as_of.date_naive(),
        la_today + chrono::Duration::days(1),
        "sanity: 23:30 Pacific is always past midnight UTC (LA is UTC-7/-8) — the bug this fixes",
    );

    let household_today = crate::forecast::household_today_at(&conn, as_of).unwrap();
    assert_eq!(
        household_today, la_today,
        "23:30 Pacific is still today locally, not tomorrow",
    );
}

/// personal-cfo-5ie.11: a bill due on the household-local calendar date must not be shown
/// as past due before local midnight, even though the finance kernel's old UTC-based
/// "today" would already have rolled to the next day. Anchored to real `Utc::now()` (not
/// a fictional 2026-07 literal, unlike most of this file's other tests) — `created_at` on
/// the bill this test creates is always real wall-clock, and a fictional "today" earlier
/// than that would trip the `personal-cfo-5ie.10` most-recent-pre-creation-occurrence rule
/// instead of testing the timezone fix this test is actually for. Uses a 366-day interval
/// (pay-schedule's max, ADR 0048 §3) so the schedule has exactly one occurrence anywhere
/// near the lookback window — a shorter cadence would always have a "last cycle" sibling
/// occurrence, which 5ie.10's separate (and separately tested) rule correctly surfaces and
/// would muddy this test's assertion.
#[test]
fn a_bill_due_today_local_is_not_past_due_at_2330_local() {
    let (_dir, worker) = worker();
    let conn = worker.read_connection().unwrap();
    conn.execute(
        "UPDATE vault_metadata SET household_timezone = ?1 WHERE singleton = 1",
        ["America/Los_Angeles"],
    )
    .unwrap();
    let tz = crate::forecast::read_household_tz(&conn).unwrap();

    let la_today = Utc::now().with_timezone(&tz).date_naive();
    let as_of = at_2330_local(tz, la_today);
    let today_local = crate::forecast::household_today_at(&conn, as_of).unwrap();
    let today_utc_buggy = as_of.date_naive();
    assert_ne!(
        today_local, today_utc_buggy,
        "sanity: the zones actually disagree at this instant",
    );

    let bill = RecurringEventId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateRecurringBill {
                event_id: bill,
                contract_id: BillContractId::new(),
                name: "Electric".to_owned(),
                amount: Money::new(9_000, Currency::Usd),
                bill_type: "utility".to_owned(),
                frequency: Frequency::EveryNDays(366),
                anchor: today_local,
                autopay_account_id: None,
                description: None,
                source_merchant_key: None,
                category_id: None,
                tag_ids: Vec::new(),
            },
        )
        .unwrap();

    let mut c = worker.read_connection().unwrap();
    // The correctly-resolved household-local "today": the bill is due TODAY, not past due.
    crate::recurring_instances::rebuild(&mut c, today_local).unwrap();
    let pending = crate::recurring_instances::read_unconfirmed_past_due(&c, today_local).unwrap();
    assert!(
        !pending
            .iter()
            .any(|o| o.recurring_event_id == bill.as_uuid()),
        "a bill due today (household-local) must not read as past due: {pending:?}",
    );

    // The OLD buggy value — what "today" would have been using the kernel's raw UTC
    // date — DOES show it as past due. Proves the assertion above is not vacuous.
    crate::recurring_instances::rebuild(&mut c, today_utc_buggy).unwrap();
    let pending_buggy =
        crate::recurring_instances::read_unconfirmed_past_due(&c, today_utc_buggy).unwrap();
    assert!(
        pending_buggy
            .iter()
            .any(|o| o.recurring_event_id == bill.as_uuid()),
        "sanity: the UTC-date bug this fixes really would have surfaced the bill: \
         {pending_buggy:?}",
    );
}

/// 5ie.9 (review #2): a confirmed occurrence still suppresses its projection after the bill's
/// due date is shifted a few days (the confirmed date matches the nearest projected occurrence
/// within tolerance), so an anchor edit after a confirm can't double-count.
#[test]
fn confirming_matches_a_shifted_occurrence_within_tolerance() {
    let (_dir, worker) = worker();
    let checking = AccountId::new();
    create_role_account(&worker, checking, "Checking", CashflowRole::LiquidCash);
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            checking,
            Money::new(500_000, Currency::Usd),
            NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
        )
        .unwrap();
    // The bill is due on the 22nd; the user confirmed the occurrence they knew as the 20th.
    let bill = RecurringEventId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateRecurringBill {
                event_id: bill,
                contract_id: BillContractId::new(),
                name: "Rent".to_owned(),
                amount: Money::new(20_000, Currency::Usd),
                bill_type: "rent_mortgage".to_owned(),
                frequency: Frequency::Monthly,
                anchor: NaiveDate::from_ymd_opt(2026, 7, 22).unwrap(),
                autopay_account_id: Some(checking),
                description: None,
                source_merchant_key: None,
                category_id: None,
                tag_ids: Vec::new(),
            },
        )
        .unwrap();
    worker
        .dispatch(
            meta(),
            WriteCommand::ConfirmObligationEarly {
                recurring_event_id: bill,
                scheduled_date: NaiveDate::from_ymd_opt(2026, 7, 20).unwrap(),
                actual_amount: Money::new(20_000, Currency::Usd),
                actual_date: NaiveDate::from_ymd_opt(2026, 7, 2)
                    .unwrap()
                    .and_hms_opt(12, 0, 0)
                    .unwrap()
                    .and_utc(),
                paying_account_id: checking,
            },
        )
        .unwrap();

    let conn = worker.read_connection().unwrap();
    let forecast = crate::forecast::compute(
        &conn,
        NaiveDate::from_ymd_opt(2026, 7, 2)
            .unwrap()
            .and_hms_opt(9, 0, 0)
            .unwrap()
            .and_utc(),
        90,
        &[],
    )
    .unwrap();
    let rent: Vec<NaiveDate> = forecast
        .days
        .iter()
        .flat_map(|d| d.events.iter().map(move |e| (d.date, e.source_event_id)))
        .filter(|(_, id)| *id == bill.as_uuid())
        .map(|(date, _)| date)
        .collect();
    assert!(
        !rent.contains(&NaiveDate::from_ymd_opt(2026, 7, 22).unwrap()),
        "the shifted-but-confirmed July occurrence is suppressed",
    );
    assert!(
        rent.contains(&NaiveDate::from_ymd_opt(2026, 8, 22).unwrap()),
        "the unconfirmed August occurrence still projects",
    );
}

/// 5ie.9: a $0 confirm ("nothing due this cycle" — e.g. a variable bill or zero statement
/// balance) clears the occurrence from the forecast without moving cash; a negative amount is
/// still rejected.
#[test]
fn confirming_a_zero_amount_clears_the_occurrence_without_moving_cash() {
    let (_dir, worker) = worker();
    let checking = AccountId::new();
    create_role_account(&worker, checking, "Checking", CashflowRole::LiquidCash);
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            checking,
            Money::new(500_000, Currency::Usd),
            NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
        )
        .unwrap();
    let bill = RecurringEventId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateRecurringBill {
                event_id: bill,
                contract_id: BillContractId::new(),
                name: "Water".to_owned(),
                amount: Money::new(20_000, Currency::Usd),
                bill_type: "utility".to_owned(),
                frequency: Frequency::Monthly,
                anchor: NaiveDate::from_ymd_opt(2026, 7, 20).unwrap(),
                autopay_account_id: Some(checking),
                description: None,
                source_merchant_key: None,
                category_id: None,
                tag_ids: Vec::new(),
            },
        )
        .unwrap();
    let due = NaiveDate::from_ymd_opt(2026, 7, 20).unwrap();
    let paid_at = NaiveDate::from_ymd_opt(2026, 7, 2)
        .unwrap()
        .and_hms_opt(12, 0, 0)
        .unwrap()
        .and_utc();
    // A negative amount is rejected.
    assert!(worker
        .dispatch(
            meta(),
            WriteCommand::ConfirmObligationEarly {
                recurring_event_id: bill,
                scheduled_date: due,
                actual_amount: Money::new(-1, Currency::Usd),
                actual_date: paid_at,
                paying_account_id: checking,
            },
        )
        .is_err());
    // A $0 confirm succeeds — nothing was due.
    worker
        .dispatch(
            meta(),
            WriteCommand::ConfirmObligationEarly {
                recurring_event_id: bill,
                scheduled_date: due,
                actual_amount: Money::new(0, Currency::Usd),
                actual_date: paid_at,
                paying_account_id: checking,
            },
        )
        .unwrap();
    // Cash didn't move.
    assert_eq!(
        worker.account_balance(checking).unwrap().unwrap(),
        Money::new(500_000, Currency::Usd),
        "a $0 confirm moves no cash",
    );
    // The occurrence is cleared from the forecast.
    let conn = worker.read_connection().unwrap();
    let forecast = crate::forecast::compute(
        &conn,
        NaiveDate::from_ymd_opt(2026, 7, 2)
            .unwrap()
            .and_hms_opt(9, 0, 0)
            .unwrap()
            .and_utc(),
        60,
        &[],
    )
    .unwrap();
    let still_projects = forecast
        .days
        .iter()
        .flat_map(|d| d.events.iter().map(move |e| (d.date, e.source_event_id)))
        .any(|(date, id)| id == bill.as_uuid() && date == due);
    assert!(!still_projects, "the $0-confirmed occurrence is suppressed");
}

/// 5ie.9: unconfirming an early-confirmed occurrence reverses the payment (balance restored)
/// and the forecast projects the occurrence again; unconfirming an unconfirmed occurrence is a
/// no-op.
#[test]
fn unconfirming_reverses_the_payment_and_reprojects() {
    let (_dir, worker) = worker();
    let checking = AccountId::new();
    create_role_account(&worker, checking, "Checking", CashflowRole::LiquidCash);
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            checking,
            Money::new(500_000, Currency::Usd),
            NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
        )
        .unwrap();
    let bill = RecurringEventId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateRecurringBill {
                event_id: bill,
                contract_id: BillContractId::new(),
                name: "Rent".to_owned(),
                amount: Money::new(20_000, Currency::Usd),
                bill_type: "rent_mortgage".to_owned(),
                frequency: Frequency::Monthly,
                anchor: NaiveDate::from_ymd_opt(2026, 7, 20).unwrap(),
                autopay_account_id: Some(checking),
                description: None,
                source_merchant_key: None,
                category_id: None,
                tag_ids: Vec::new(),
            },
        )
        .unwrap();
    let due = NaiveDate::from_ymd_opt(2026, 7, 20).unwrap();
    worker
        .dispatch(
            meta(),
            WriteCommand::ConfirmObligationEarly {
                recurring_event_id: bill,
                scheduled_date: due,
                actual_amount: Money::new(20_000, Currency::Usd),
                actual_date: NaiveDate::from_ymd_opt(2026, 7, 2)
                    .unwrap()
                    .and_hms_opt(12, 0, 0)
                    .unwrap()
                    .and_utc(),
                paying_account_id: checking,
            },
        )
        .unwrap();
    assert_eq!(
        worker.account_balance(checking).unwrap().unwrap(),
        Money::new(480_000, Currency::Usd),
    );

    let unconfirm = |worker: &DbWorker| {
        worker
            .dispatch(
                meta(),
                WriteCommand::UnconfirmObligation {
                    recurring_event_id: bill,
                    scheduled_date: due,
                },
            )
            .unwrap();
    };
    unconfirm(&worker);
    assert_eq!(
        worker.account_balance(checking).unwrap().unwrap(),
        Money::new(500_000, Currency::Usd),
        "unconfirm reverses the payment",
    );
    let conn = worker.read_connection().unwrap();
    let forecast = crate::forecast::compute(
        &conn,
        NaiveDate::from_ymd_opt(2026, 7, 2)
            .unwrap()
            .and_hms_opt(9, 0, 0)
            .unwrap()
            .and_utc(),
        60,
        &[],
    )
    .unwrap();
    let reprojects = forecast
        .days
        .iter()
        .flat_map(|d| d.events.iter().map(move |e| (d.date, e.source_event_id)))
        .any(|(date, id)| id == bill.as_uuid() && date == due);
    assert!(reprojects, "the occurrence projects again after unconfirm");

    // Unconfirming again changes nothing.
    unconfirm(&worker);
    assert_eq!(
        worker.account_balance(checking).unwrap().unwrap(),
        Money::new(500_000, Currency::Usd),
        "re-unconfirm is a no-op",
    );
}

#[test]
fn rebuild_read_models_repairs_drift() {
    let (_dir, worker) = worker();
    // Materialize the read model so an authoritative checksum is stored.
    worker.rebuild_transaction_display().unwrap();
    assert!(worker.read_models_current().unwrap());

    // Fault-inject drift: corrupt the stored checksum behind the projection's
    // back, so it no longer matches a fresh compute (personal-cfo-5ivp).
    {
        let guard = worker.lock();
        guard
            .conn
            .execute(
                "UPDATE read_model_checksums SET current_checksum = current_checksum + 1 \
                     WHERE read_model_name = 'transaction_display'",
                [],
            )
            .unwrap();
    }
    assert!(
        !worker.read_models_current().unwrap(),
        "a tampered checksum reads as drift"
    );

    // The rebuild repair restores coherence (deterministic, non-destructive).
    worker.rebuild_transaction_display().unwrap();
    assert!(worker.read_models_current().unwrap());
}

#[test]
fn future_cash_forecast_projects_income_and_bills() {
    use chrono::TimeZone;
    let (_dir, worker) = worker();
    let account_id = AccountId::new();
    worker
        .dispatch(
            meta(),
            liquid_account_cmd(account_id, Currency::Usd, 100_000),
        )
        .unwrap();
    // Biweekly income (Acme, +3,000 from 2026-06-05) and a monthly bill
    // (Rent, -1,800 from 2026-07-01).
    worker
        .dispatch(meta(), income_cmd(Some(account_id)))
        .unwrap();
    worker.dispatch(meta(), bill_cmd(Some(account_id))).unwrap();

    let conn = worker.read_connection().unwrap();
    let as_of = Utc.with_ymd_and_hms(2026, 6, 1, 12, 0, 0).unwrap();
    let view = forecast::compute(&conn, as_of, 400, &[]).unwrap();

    assert_eq!(view.currency, Currency::Usd);
    assert_eq!(view.starting_balance, Money::new(100_000, Currency::Usd));
    assert_eq!(
        view.start_date,
        NaiveDate::from_ymd_opt(2026, 6, 1).unwrap()
    );
    assert_eq!(view.days.len(), 400);

    let events: Vec<&ForecastEventView> = view.days.iter().flat_map(|d| d.events.iter()).collect();
    assert!(events.iter().any(|e| e.kind == "income"
        && e.name == "Acme Corp"
        && e.amount == Money::new(300_000, Currency::Usd)));
    assert!(events.iter().any(|e| e.kind == "recurring_bill"
        && e.name == "Rent"
        && e.amount == Money::new(-180_000, Currency::Usd)));
    // Provenance is threaded through (ADR 0026 §1): scheduled events carry
    // their recurring-schedule basis rather than being dropped.
    assert!(events.iter().all(|e| matches!(
        e.assumption_basis,
        AssumptionBasis::RecurringSchedule { .. }
    )));
    // Deterministic Layer-1 emits a collapsed band on every day.
    assert!(view
        .days
        .iter()
        .all(|d| d.closing.p10 == d.closing.p50 && d.closing.p50 == d.closing.p90));

    // The final balance equals the opening balance plus every projected event.
    let total: i64 = events.iter().map(|e| e.amount.minor_units()).sum();
    assert_eq!(
        view.days.last().unwrap().closing.p50,
        Money::new(100_000 + total, Currency::Usd)
    );
}

#[test]
fn persist_daily_forecast_writes_a_run_then_dedups_within_the_day() {
    use chrono::TimeZone;
    let (_dir, worker) = worker();
    let account_id = AccountId::new();
    worker
        .dispatch(
            meta(),
            liquid_account_cmd(account_id, Currency::Usd, 100_000),
        )
        .unwrap();
    worker
        .dispatch(meta(), income_cmd(Some(account_id)))
        .unwrap();

    let day = Utc.with_ymd_and_hms(2026, 6, 1, 9, 0, 0).unwrap();
    // (1) First open persists a run + its rows.
    let run = worker.persist_daily_forecast_at(day, 365).unwrap();
    assert!(run.is_some(), "first open should persist a run");
    assert_eq!(forecast_run_count(&worker), 1);
    let rows: i64 = worker
        .read_connection()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM forecast_rows", [], |r| r.get(0))
        .unwrap();
    assert!(rows > 0, "the run should have persisted forecast rows");

    // (2) A second open later the same day, inputs unchanged, dedups to a no-op.
    let again = worker
        .persist_daily_forecast_at(
            day.with_time(chrono::NaiveTime::from_hms_opt(18, 0, 0).unwrap())
                .unwrap(),
            365,
        )
        .unwrap();
    assert!(
        again.is_none(),
        "same inputs same day should not persist again"
    );
    assert_eq!(forecast_run_count(&worker), 1);
}

#[test]
fn persist_daily_forecast_writes_a_new_run_after_a_forecast_change() {
    use chrono::TimeZone;
    let (_dir, worker) = worker();
    let account_id = AccountId::new();
    worker
        .dispatch(
            meta(),
            liquid_account_cmd(account_id, Currency::Usd, 100_000),
        )
        .unwrap();

    let day = Utc.with_ymd_and_hms(2026, 6, 1, 9, 0, 0).unwrap();
    assert!(worker
        .persist_daily_forecast_at(day, 365)
        .unwrap()
        .is_some());
    assert_eq!(forecast_run_count(&worker), 1);

    // A forecast-affecting change (adding a bill) shifts the input content hash,
    // so a new run persists the same day despite the dedup guard.
    worker.dispatch(meta(), bill_cmd(Some(account_id))).unwrap();
    let after = worker
        .persist_daily_forecast_at(
            day.with_time(chrono::NaiveTime::from_hms_opt(10, 0, 0).unwrap())
                .unwrap(),
            365,
        )
        .unwrap();
    assert!(
        after.is_some(),
        "a changed input should persist a fresh run"
    );
    assert_eq!(forecast_run_count(&worker), 2);
}

#[test]
fn persist_daily_forecast_writes_a_new_run_on_a_new_day() {
    use chrono::TimeZone;
    let (_dir, worker) = worker();
    let account_id = AccountId::new();
    worker
        .dispatch(
            meta(),
            liquid_account_cmd(account_id, Currency::Usd, 100_000),
        )
        .unwrap();

    // Same inputs, two different calendar days → one run each (the daily series).
    let day1 = Utc.with_ymd_and_hms(2026, 6, 1, 9, 0, 0).unwrap();
    let day2 = Utc.with_ymd_and_hms(2026, 6, 2, 9, 0, 0).unwrap();
    assert!(worker
        .persist_daily_forecast_at(day1, 365)
        .unwrap()
        .is_some());
    assert!(
        worker
            .persist_daily_forecast_at(day2, 365)
            .unwrap()
            .is_some(),
        "unchanged inputs on a new day should still persist (daily cadence)"
    );
    assert_eq!(forecast_run_count(&worker), 2);
}

#[test]
fn recurring_instances_match_forecast_occurrences() {
    // The instance projection (5ie.4) must agree with the forecast's own
    // occurrence expansion: every scheduled income/bill forecast event has an
    // instance with the same entity, date, and magnitude.
    let (_dir, worker) = worker();
    let account_id = AccountId::new();
    worker
        .dispatch(
            meta(),
            liquid_account_cmd(account_id, Currency::Usd, 100_000),
        )
        .unwrap();
    worker
        .dispatch(meta(), income_cmd(Some(account_id)))
        .unwrap();
    worker.dispatch(meta(), bill_cmd(Some(account_id))).unwrap();

    let today = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
    let mut conn = worker.read_connection().unwrap();
    recurring_instances::rebuild(&mut conn, today).unwrap();
    drop(conn);

    let instances = worker.recurring_instances().unwrap();
    let conn = worker.read_connection().unwrap();
    let as_of = today.and_hms_opt(12, 0, 0).unwrap().and_utc();
    let view = forecast::compute(&conn, as_of, 180, &[]).unwrap();
    let mut checked = 0;
    for day in &view.days {
        for ev in &day.events {
            if !matches!(
                ev.kind.as_str(),
                "income" | "recurring_bill" | "loan_payment"
            ) {
                continue;
            }
            let found = instances.iter().any(|i| {
                i.recurring_event_id == ev.source_event_id
                    && i.scheduled_date == day.date.to_string()
                    && i.expected_amount_minor == ev.amount.minor_units().abs()
            });
            assert!(found, "no instance for {} on {}", ev.kind, day.date);
            checked += 1;
        }
    }
    assert!(checked > 0, "expected scheduled forecast events to verify");
}

#[test]
fn recurring_instance_links_a_realized_transaction() {
    // A realized deposit on a pay date links its income instance (5ie.5).
    let (_dir, worker) = worker();
    let account_id = AccountId::new();
    worker
        .dispatch(meta(), liquid_account_cmd(account_id, Currency::Usd, 0))
        .unwrap();
    worker
        .dispatch(meta(), income_cmd(Some(account_id)))
        .unwrap(); // +3,000 biweekly from 2026-06-05
    worker
        .dispatch(
            meta(),
            WriteCommand::RecordTransaction {
                transaction_id: TransactionId::new(),
                account_id,
                amount: Money::new(300_000, Currency::Usd),
                occurred_at: NaiveDate::from_ymd_opt(2026, 6, 5)
                    .unwrap()
                    .and_hms_opt(0, 0, 0)
                    .unwrap()
                    .and_utc(),
            },
        )
        .unwrap();
    let txn_id = worker.recent_transactions(50).unwrap()[0].transaction_id;

    let today = NaiveDate::from_ymd_opt(2026, 6, 10).unwrap();
    let mut conn = worker.read_connection().unwrap();
    recurring_instances::rebuild(&mut conn, today).unwrap();
    drop(conn);

    let instances = worker.recurring_instances().unwrap();
    let paid: Vec<_> = instances.iter().filter(|i| i.status == "paid").collect();
    assert_eq!(
        paid.len(),
        1,
        "only the 2026-06-05 occurrence should be paid"
    );
    assert_eq!(paid[0].scheduled_date, "2026-06-05");
    assert_eq!(paid[0].linked_transaction_id, Some(txn_id.as_uuid()));
}

#[test]
fn recurring_instance_rebuild_is_idempotent() {
    let (_dir, worker) = worker();
    let account_id = AccountId::new();
    worker
        .dispatch(
            meta(),
            liquid_account_cmd(account_id, Currency::Usd, 100_000),
        )
        .unwrap();
    worker
        .dispatch(meta(), income_cmd(Some(account_id)))
        .unwrap();
    worker.dispatch(meta(), bill_cmd(Some(account_id))).unwrap();

    let today = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
    let mut conn = worker.read_connection().unwrap();
    recurring_instances::rebuild(&mut conn, today).unwrap();
    let first = worker.recurring_instances().unwrap();
    recurring_instances::rebuild(&mut conn, today).unwrap();
    let second = worker.recurring_instances().unwrap();
    assert!(!first.is_empty());
    assert_eq!(first, second, "a re-run yields byte-identical instances");
}

#[test]
fn empty_vault_has_no_recurring_instances() {
    let (_dir, worker) = worker();
    let mut conn = worker.read_connection().unwrap();
    let n = recurring_instances::rebuild(&mut conn, NaiveDate::from_ymd_opt(2026, 6, 1).unwrap())
        .unwrap();
    assert_eq!(n, 0);
}

#[test]
fn actualization_marks_an_on_time_exact_payment() {
    use chrono::TimeZone;
    let (_dir, worker) = worker();
    let as_of = Utc.with_ymd_and_hms(2026, 6, 1, 12, 0, 0).unwrap();
    let account_id = seed_income_and_persist(&worker, as_of);
    // The paycheck lands on the predicted date for the predicted amount.
    worker
        .dispatch(
            meta(),
            deposit_cmd(
                account_id,
                300_000,
                NaiveDate::from_ymd_opt(2026, 6, 5).unwrap(),
            ),
        )
        .unwrap();

    let actuals = actualize_and_read(&worker, NaiveDate::from_ymd_opt(2026, 6, 6).unwrap());
    assert_eq!(actuals.len(), 1, "only the 2026-06-05 row is past");
    let (date, status, realized, has_txn) = &actuals[0];
    assert_eq!(date, "2026-06-05");
    assert_eq!(status, "exact");
    assert_eq!(*realized, 300_000);
    assert!(has_txn);
}

#[test]
fn actualization_marks_an_off_amount_payment_matched() {
    use chrono::TimeZone;
    let (_dir, worker) = worker();
    let as_of = Utc.with_ymd_and_hms(2026, 6, 1, 12, 0, 0).unwrap();
    let account_id = seed_income_and_persist(&worker, as_of);
    // +2,900: within the 5% link tolerance but outside the 1% exact band.
    worker
        .dispatch(
            meta(),
            deposit_cmd(
                account_id,
                290_000,
                NaiveDate::from_ymd_opt(2026, 6, 5).unwrap(),
            ),
        )
        .unwrap();

    let actuals = actualize_and_read(&worker, NaiveDate::from_ymd_opt(2026, 6, 6).unwrap());
    assert_eq!(actuals.len(), 1);
    assert_eq!(actuals[0].1, "matched");
    assert_eq!(actuals[0].2, 290_000);
}

#[test]
fn actualization_marks_an_unpaid_past_occurrence_missed() {
    use chrono::TimeZone;
    let (_dir, worker) = worker();
    let as_of = Utc.with_ymd_and_hms(2026, 6, 1, 12, 0, 0).unwrap();
    seed_income_and_persist(&worker, as_of); // no deposit recorded

    let actuals = actualize_and_read(&worker, NaiveDate::from_ymd_opt(2026, 6, 6).unwrap());
    assert_eq!(actuals.len(), 1);
    assert_eq!(actuals[0].1, "missed");
    assert_eq!(actuals[0].2, 0);
    assert!(
        !actuals[0].3,
        "a missed occurrence has no matched transaction"
    );
}

#[test]
fn actualization_supersedes_an_older_runs_prediction() {
    use chrono::TimeZone;
    let (_dir, worker) = worker();
    // Two runs for the same inputs on different days both predict 2026-06-05;
    // only the newer run's row is scored, the older is superseded.
    let account_id =
        seed_income_and_persist(&worker, Utc.with_ymd_and_hms(2026, 6, 1, 12, 0, 0).unwrap());
    worker
        .persist_daily_forecast_at(Utc.with_ymd_and_hms(2026, 6, 2, 12, 0, 0).unwrap(), 365)
        .unwrap();
    assert_eq!(worker.persisted_forecast_run_count().unwrap(), 2);
    worker
        .dispatch(
            meta(),
            deposit_cmd(
                account_id,
                300_000,
                NaiveDate::from_ymd_opt(2026, 6, 5).unwrap(),
            ),
        )
        .unwrap();

    let actuals = actualize_and_read(&worker, NaiveDate::from_ymd_opt(2026, 6, 6).unwrap());
    let statuses: Vec<&str> = actuals.iter().map(|a| a.1.as_str()).collect();
    assert_eq!(actuals.len(), 2, "one row per run for 2026-06-05");
    assert_eq!(statuses.iter().filter(|s| **s == "exact").count(), 1);
    assert_eq!(statuses.iter().filter(|s| **s == "superseded").count(), 1);
}

#[test]
fn actualization_is_idempotent() {
    use chrono::TimeZone;
    let (_dir, worker) = worker();
    let account_id =
        seed_income_and_persist(&worker, Utc.with_ymd_and_hms(2026, 6, 1, 12, 0, 0).unwrap());
    worker
        .dispatch(
            meta(),
            deposit_cmd(
                account_id,
                300_000,
                NaiveDate::from_ymd_opt(2026, 6, 5).unwrap(),
            ),
        )
        .unwrap();
    let today = NaiveDate::from_ymd_opt(2026, 6, 6).unwrap();
    let first = actualize_and_read(&worker, today);
    let second = actualize_and_read(&worker, today);
    assert_eq!(worker.forecast_actuals_count().unwrap(), 1);
    assert_eq!(first, second);
}

#[test]
fn recurrence_actuals_factor_is_neutral_without_recurring_events() {
    use chrono::TimeZone;
    // An account but no income/bills: nothing to verify, so the factor is neutral (100)
    // rather than dragging the score down (mirrors the categorization-when-no-spend rule).
    let (_dir, worker) = worker();
    let account_id = AccountId::new();
    worker
        .dispatch(meta(), liquid_account_cmd(account_id, Currency::Usd, 0))
        .unwrap();
    let r = forecast::compute_forecast_readiness(
        &worker.read_connection().unwrap(),
        Utc.with_ymd_and_hms(2026, 6, 6, 12, 0, 0).unwrap(),
    )
    .unwrap();
    assert_eq!(readiness_factor(&r, "recurrence_actuals").score, 100);
}

#[test]
fn recurrence_actuals_factor_is_zero_with_events_but_no_actuals() {
    use chrono::TimeZone;
    // Income exists but nothing has been actualized yet → the factor is 0 (real history
    // is what earns it), and it pulls the blended score below the no-events case.
    let (_dir, worker) = worker();
    seed_income_and_persist(&worker, Utc.with_ymd_and_hms(2026, 6, 1, 12, 0, 0).unwrap());
    let r = forecast::compute_forecast_readiness(
        &worker.read_connection().unwrap(),
        Utc.with_ymd_and_hms(2026, 6, 6, 12, 0, 0).unwrap(),
    )
    .unwrap();
    assert_eq!(readiness_factor(&r, "recurrence_actuals").score, 0);
}

#[test]
fn recurrence_actuals_factor_fills_as_payments_are_verified() {
    use chrono::TimeZone;
    // Three biweekly paychecks (2026-06-05 / -19 / 07-03) land on time and get
    // actualized → the single income event clears the ≥3-actuals bar → factor 100.
    let (_dir, worker) = worker();
    let account_id =
        seed_income_and_persist(&worker, Utc.with_ymd_and_hms(2026, 6, 1, 12, 0, 0).unwrap());
    for day in [
        NaiveDate::from_ymd_opt(2026, 6, 5).unwrap(),
        NaiveDate::from_ymd_opt(2026, 6, 19).unwrap(),
        NaiveDate::from_ymd_opt(2026, 7, 3).unwrap(),
    ] {
        worker
            .dispatch(meta(), deposit_cmd(account_id, 300_000, day))
            .unwrap();
    }
    // Actualize as of a date past all three occurrences.
    let today = NaiveDate::from_ymd_opt(2026, 7, 4).unwrap();
    actualize_and_read(&worker, today);

    let r = forecast::compute_forecast_readiness(
        &worker.read_connection().unwrap(),
        Utc.with_ymd_and_hms(2026, 7, 4, 12, 0, 0).unwrap(),
    )
    .unwrap();
    assert_eq!(readiness_factor(&r, "recurrence_actuals").score, 100);
}

#[test]
fn backtest_records_low_mape_when_forecasts_are_accurate() {
    use chrono::TimeZone;
    // Three biweekly paychecks land exactly as forecast → MAPE ≈ 0 → the forecast-accuracy
    // factor is full (100), and a forecast_backtest_results row is written.
    let (_dir, worker) = worker();
    let account_id =
        seed_income_and_persist(&worker, Utc.with_ymd_and_hms(2026, 6, 1, 12, 0, 0).unwrap());
    for day in [
        NaiveDate::from_ymd_opt(2026, 6, 5).unwrap(),
        NaiveDate::from_ymd_opt(2026, 6, 19).unwrap(),
        NaiveDate::from_ymd_opt(2026, 7, 3).unwrap(),
    ] {
        worker
            .dispatch(meta(), deposit_cmd(account_id, 300_000, day))
            .unwrap();
    }
    let written = actualize_and_backtest(&worker, NaiveDate::from_ymd_opt(2026, 7, 4).unwrap());
    assert_eq!(written, 1, "enough history → a backtest row is recorded");

    let conn = worker.read_connection().unwrap();
    let (mape_bps, sample) = forecast_backtest::latest_mape(&conn).unwrap().unwrap();
    assert_eq!(sample, 3);
    assert!(
        mape_bps < 100,
        "accurate forecasts → MAPE under 1%, got {mape_bps} bps"
    );

    let r = forecast::compute_forecast_readiness(
        &conn,
        Utc.with_ymd_and_hms(2026, 7, 4, 12, 0, 0).unwrap(),
    )
    .unwrap();
    assert_eq!(readiness_factor(&r, "backtest_mape").score, 100);
}

#[test]
fn backtest_factor_is_neutral_without_enough_history() {
    use chrono::TimeZone;
    // One realized paycheck (< the 3-sample floor) → no MAPE recorded → the factor stays
    // neutral (100): accuracy we can't yet measure shouldn't penalize the vault.
    let (_dir, worker) = worker();
    let account_id =
        seed_income_and_persist(&worker, Utc.with_ymd_and_hms(2026, 6, 1, 12, 0, 0).unwrap());
    worker
        .dispatch(
            meta(),
            deposit_cmd(
                account_id,
                300_000,
                NaiveDate::from_ymd_opt(2026, 6, 5).unwrap(),
            ),
        )
        .unwrap();
    let written = actualize_and_backtest(&worker, NaiveDate::from_ymd_opt(2026, 6, 6).unwrap());
    assert_eq!(written, 0, "too few samples → nothing recorded");

    let conn = worker.read_connection().unwrap();
    assert!(forecast_backtest::latest_mape(&conn).unwrap().is_none());
    let r = forecast::compute_forecast_readiness(
        &conn,
        Utc.with_ymd_and_hms(2026, 6, 6, 12, 0, 0).unwrap(),
    )
    .unwrap();
    assert_eq!(readiness_factor(&r, "backtest_mape").score, 100);
}

#[test]
fn backtest_penalizes_forecasts_with_missed_events() {
    use chrono::TimeZone;
    // Two paychecks land, one predicted paycheck never does (missed = 100% error). The
    // aggregate MAPE leaves the envelope → the forecast-accuracy factor drops below full.
    let (_dir, worker) = worker();
    let account_id =
        seed_income_and_persist(&worker, Utc.with_ymd_and_hms(2026, 6, 1, 12, 0, 0).unwrap());
    for day in [
        NaiveDate::from_ymd_opt(2026, 6, 5).unwrap(),
        NaiveDate::from_ymd_opt(2026, 6, 19).unwrap(),
        // 2026-07-03 is deliberately NOT deposited → a missed prediction.
    ] {
        worker
            .dispatch(meta(), deposit_cmd(account_id, 300_000, day))
            .unwrap();
    }
    actualize_and_backtest(&worker, NaiveDate::from_ymd_opt(2026, 7, 4).unwrap());

    let conn = worker.read_connection().unwrap();
    let (mape_bps, sample) = forecast_backtest::latest_mape(&conn).unwrap().unwrap();
    assert_eq!(sample, 3, "two exact + one missed");
    assert!(
        mape_bps > 1_000,
        "a missed event pushes MAPE past 10%, got {mape_bps} bps"
    );
    let r = forecast::compute_forecast_readiness(
        &conn,
        Utc.with_ymd_and_hms(2026, 7, 4, 12, 0, 0).unwrap(),
    )
    .unwrap();
    assert!(
        readiness_factor(&r, "backtest_mape").score < 100,
        "misses lower the forecast-accuracy factor"
    );
}

/// 6p6b: the canonical-merchant seed is idempotent — re-running it (as every vault open
/// does) adds no duplicate identities or aliases.
#[test]
fn seed_merchants_is_idempotent() {
    let (_dir, worker) = worker();
    let count = worker.merchant_identity_count().unwrap();
    assert!(count >= 1, "at least Amazon is seeded");
    merchant_identity::ensure_seed_merchants(&worker.read_connection().unwrap()).unwrap();
    assert_eq!(
        worker.merchant_identity_count().unwrap(),
        count,
        "re-seeding adds nothing"
    );
}

#[test]
fn future_cash_forecast_rejects_mixed_currency_liquid_accounts() {
    use chrono::TimeZone;
    let (_dir, worker) = worker();
    worker
        .dispatch(
            meta(),
            liquid_account_cmd(AccountId::new(), Currency::Usd, 100_000),
        )
        .unwrap();
    worker
        .dispatch(
            meta(),
            liquid_account_cmd(AccountId::new(), Currency::Eur, 50_000),
        )
        .unwrap();
    let conn = worker.read_connection().unwrap();
    let as_of = Utc.with_ymd_and_hms(2026, 6, 1, 12, 0, 0).unwrap();
    assert!(matches!(
        forecast::compute(&conn, as_of, 30, &[]).unwrap_err(),
        DbError::InvalidCommand(_)
    ));
}

/// cmx: committing a staged transaction posts a balanced ledger transaction,
/// links FK-strict import provenance, and marks the staged row committed.
#[test]
fn committing_a_staged_transaction_posts_to_the_ledger_with_provenance() {
    let (_dir, worker) = worker();
    let account = sample_account();
    let account_id = account.id();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(account),
                opening_balance: None,
            },
        )
        .unwrap();

    let staged = stage_a_transaction(&worker, account_id, -1299, "fp-coffee");
    worker
        .dispatch(
            meta(),
            WriteCommand::CommitStaged {
                staged_transaction_id: staged,
                force: false,
            },
        )
        .unwrap();

    assert_eq!(
        worker.account_balance(account_id).unwrap(),
        Some(Money::new(-1299, Currency::Usd))
    );
    let conn = worker.read_connection().unwrap();
    let txns: i64 = conn
        .query_row("SELECT COUNT(*) FROM ledger_transactions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(txns, 1);
    let prov: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM source_provenance_links
                  WHERE entity_type = 'ledger_transaction' AND relationship = 'created_from'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(prov, 1);
    let status: String = conn
        .query_row(
            "SELECT commit_status FROM staged_transactions WHERE id = ?1",
            rusqlite::params![staged.as_uuid()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(status, "committed");
}

/// ADR 0045 (personal-cfo-4d8.24.1.3): a staged transaction carrying a secondary
/// transaction/authorization date lands it on the committed transaction's details
/// and surfaces it in the read model — the posted date stays the primary occurred_at.
#[test]
fn committing_a_staged_transaction_surfaces_the_secondary_date() {
    let (_dir, worker) = worker();
    let account = sample_account();
    let account_id = account.id();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(account),
                opening_balance: None,
            },
        )
        .unwrap();

    let staged =
        stage_a_transaction_dated(&worker, account_id, -1299, "fp-dated", Some("2026-06-18"));
    worker
        .dispatch(
            meta(),
            WriteCommand::CommitStaged {
                staged_transaction_id: staged,
                force: false,
            },
        )
        .unwrap();

    let conn = worker.read_connection().unwrap();
    let rows = crate::read_recent_transactions(&conn, 10).unwrap();
    let row = rows.first().expect("one committed transaction");
    // Posted date is the primary occurred_at (2026-06-20 noon UTC); the secondary
    // transaction date is surfaced separately.
    assert_eq!(
        row.occurred_at.date_naive(),
        chrono::NaiveDate::from_ymd_opt(2026, 6, 20).unwrap()
    );
    assert_eq!(
        row.transaction_date,
        Some(chrono::NaiveDate::from_ymd_opt(2026, 6, 18).unwrap())
    );

    // A single-date import leaves the secondary date null.
    let plain = stage_a_transaction(&worker, account_id, -500, "fp-plain");
    worker
        .dispatch(
            meta(),
            WriteCommand::CommitStaged {
                staged_transaction_id: plain,
                force: false,
            },
        )
        .unwrap();
    let conn = worker.read_connection().unwrap();
    let rows = crate::read_recent_transactions(&conn, 10).unwrap();
    assert!(
        rows.iter()
            .any(|r| r.amount.minor_units() == -500 && r.transaction_date.is_none()),
        "a single-date import has no secondary transaction date"
    );
}

/// ADR 0045 §2 (personal-cfo-4d8.24.1.4): the raw imported fields behind a committed
/// transaction are retrievable through the provenance link — every captured source
/// column, key-sorted, with the source type. A non-imported id resolves to None.
#[test]
fn imported_transaction_fields_returns_the_captured_source_columns() {
    let (_dir, worker) = worker();
    let account = sample_account();
    let account_id = account.id();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(account),
                opening_balance: None,
            },
        )
        .unwrap();

    let batch_id = SourceBatchId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateSourceBatch {
                id: batch_id,
                source_type: "csv".to_owned(),
                source_name: None,
                file_fingerprint: None,
                parser_version: None,
            },
        )
        .unwrap();
    let record_id = SourceRecordId::new();
    let normalized = r#"{"Posted Date":"2026-06-20","Card No.":"1234","Category":"Dining","Description":"Coffee"}"#;
    worker
        .dispatch(
            meta(),
            WriteCommand::AttachSourceRecord {
                id: record_id,
                batch_id,
                external_id: None,
                source_hash: "hash-imp".to_owned(),
                normalized_json: normalized.to_owned(),
                parse_confidence_bps: None,
            },
        )
        .unwrap();
    let staged = {
        let conn = worker.read_connection().unwrap();
        crate::ingestion::stage_transaction(
            &conn,
            &crate::ingestion::NewStagedTransaction {
                source_record_id: record_id.as_uuid(),
                proposed_account_id: Some(account_id.as_uuid()),
                posted_at: "2026-06-20",
                transaction_date: None,
                amount_minor: -1299,
                currency: "USD",
                normalized_merchant: Some("coffee"),
                description: Some("Coffee"),
                imported_category: None,
                txn_fingerprint: "fp-imp",
            },
        )
        .unwrap()
    };
    let staged_id = StagedTransactionId::from_uuid(staged);
    worker
        .dispatch(
            meta(),
            WriteCommand::CommitStaged {
                staged_transaction_id: staged_id,
                force: false,
            },
        )
        .unwrap();
    let txn_id: uuid::Uuid = {
        let conn = worker.read_connection().unwrap();
        conn.query_row(
            "SELECT committed_transaction_id FROM staged_transactions WHERE id = ?1",
            rusqlite::params![staged_id.as_uuid()],
            |r| r.get(0),
        )
        .unwrap()
    };

    let imported = worker
        .imported_transaction_fields(txn_id)
        .unwrap()
        .expect("committed import has provenance");
    assert_eq!(imported.source_type, "csv");
    // Every source column is present, key-sorted (BTreeMap ordering).
    let keys: Vec<&str> = imported.fields.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(
        keys,
        vec!["Card No.", "Category", "Description", "Posted Date"]
    );
    assert_eq!(
        imported
            .fields
            .iter()
            .find(|(k, _)| k == "Card No.")
            .map(|(_, v)| v.as_str()),
        Some("1234")
    );

    // A transaction with no import provenance resolves to None (not an error).
    assert!(worker
        .imported_transaction_fields(uuid::Uuid::now_v7())
        .unwrap()
        .is_none());
}

/// The `normalized_json` parser is key-sorted, string-valued, and tolerant of junk.
#[test]
fn parse_normalized_fields_is_sorted_and_lenient() {
    let fields = crate::parse_normalized_fields(r#"{"b":"2","a":"1","c":""}"#);
    assert_eq!(
        fields,
        vec![
            ("a".to_owned(), "1".to_owned()),
            ("b".to_owned(), "2".to_owned()),
            ("c".to_owned(), String::new()),
        ]
    );
    // Malformed / non-object JSON yields no fields rather than an error.
    assert!(crate::parse_normalized_fields("not json").is_empty());
    assert!(crate::parse_normalized_fields("[1,2,3]").is_empty());
}

/// Stage a transaction carrying an imported category string (its own batch/record).
fn stage_with_imported_category(
    worker: &DbWorker,
    account_id: AccountId,
    amount_minor: i64,
    fingerprint: &str,
    imported_category: Option<&str>,
) -> StagedTransactionId {
    let batch_id = SourceBatchId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateSourceBatch {
                id: batch_id,
                source_type: "csv".to_owned(),
                source_name: None,
                file_fingerprint: None,
                parser_version: None,
            },
        )
        .unwrap();
    let record_id = SourceRecordId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::AttachSourceRecord {
                id: record_id,
                batch_id,
                external_id: None,
                source_hash: format!("hash-{fingerprint}"),
                normalized_json: "{}".to_owned(),
                parse_confidence_bps: None,
            },
        )
        .unwrap();
    let conn = worker.read_connection().unwrap();
    let staged = crate::ingestion::stage_transaction(
        &conn,
        &crate::ingestion::NewStagedTransaction {
            source_record_id: record_id.as_uuid(),
            proposed_account_id: Some(account_id.as_uuid()),
            posted_at: "2026-06-20",
            transaction_date: None,
            amount_minor,
            currency: "USD",
            normalized_merchant: Some("merchant"),
            description: None,
            imported_category,
            txn_fingerprint: fingerprint,
        },
    )
    .unwrap();
    StagedTransactionId::from_uuid(staged)
}

/// ADR 0045 §3 (personal-cfo-4d8.24.1.1): an imported category that names a real
/// category prefills it as a reduced-confidence `import_alias` categorization
/// (case-insensitive match); a non-matching name leaves the transaction uncategorized.
#[test]
fn imported_category_prefills_a_matching_category_as_import_alias() {
    let (_dir, worker) = worker();
    let account = sample_account();
    let account_id = account.id();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(account),
                opening_balance: None,
            },
        )
        .unwrap();
    // A seeded, UNIQUELY-named category to match by name (case-insensitively). The
    // uniqueness filter matters: some seeded names (e.g. "Maintenance") repeat, and an
    // ambiguous name must NOT prefill (asserted below).
    let (cat_id, cat_name): (Uuid, String) = {
        let conn = worker.read_connection().unwrap();
        conn.query_row(
            "SELECT id, name FROM categories c
              WHERE archived_at IS NULL
                AND (SELECT COUNT(*) FROM categories c2
                      WHERE c2.name = c.name COLLATE NOCASE AND c2.archived_at IS NULL) = 1
              ORDER BY id LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
    };

    let staged = stage_with_imported_category(
        &worker,
        account_id,
        -1299,
        "fp-cat",
        Some(&cat_name.to_uppercase()),
    );
    worker
        .dispatch(
            meta(),
            WriteCommand::CommitStaged {
                staged_transaction_id: staged,
                force: false,
            },
        )
        .unwrap();
    let txn_id: Uuid = {
        let conn = worker.read_connection().unwrap();
        conn.query_row(
            "SELECT committed_transaction_id FROM staged_transactions WHERE id = ?1",
            rusqlite::params![staged.as_uuid()],
            |r| r.get(0),
        )
        .unwrap()
    };
    let (category_id, source, conf): (Uuid, String, i64) = {
        let conn = worker.read_connection().unwrap();
        conn.query_row(
            "SELECT category_id, source, confidence_bps
               FROM transaction_categorizations WHERE transaction_id = ?1",
            rusqlite::params![txn_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap()
    };
    assert_eq!(category_id, cat_id);
    assert_eq!(source, "import_alias");
    assert_eq!(conf, 5000);

    // A user re-categorization supersedes the import_alias prefill (source -> user).
    worker
        .dispatch(
            meta(),
            WriteCommand::RecategorizeTransaction {
                transaction_id: TransactionId::from_uuid(txn_id),
                category_id: Some(CategoryId::from_uuid(cat_id)),
            },
        )
        .unwrap();
    let source_after: String = {
        let conn = worker.read_connection().unwrap();
        conn.query_row(
            "SELECT source FROM transaction_categorizations WHERE transaction_id = ?1",
            rusqlite::params![txn_id],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(source_after, "user");

    // An AMBIGUOUS imported category — a name shared by two seeded categories
    // ("Maintenance" is seeded under both Housing and Transportation) — is NOT
    // prefilled; we skip rather than guess the wrong subcategory (regression for the
    // 4d8.24.1.1 adversarial-review finding).
    let dup_count: i64 = {
        let conn = worker.read_connection().unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM categories
              WHERE name = 'Maintenance' COLLATE NOCASE AND archived_at IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert!(
        dup_count >= 2,
        "the seed must have a duplicate category name to exercise ambiguity"
    );
    let staged_amb =
        stage_with_imported_category(&worker, account_id, -400, "fp-amb", Some("Maintenance"));
    worker
        .dispatch(
            meta(),
            WriteCommand::CommitStaged {
                staged_transaction_id: staged_amb,
                force: false,
            },
        )
        .unwrap();
    let txn_amb: Uuid = {
        let conn = worker.read_connection().unwrap();
        conn.query_row(
            "SELECT committed_transaction_id FROM staged_transactions WHERE id = ?1",
            rusqlite::params![staged_amb.as_uuid()],
            |r| r.get(0),
        )
        .unwrap()
    };
    let amb_count: i64 = {
        let conn = worker.read_connection().unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM transaction_categorizations WHERE transaction_id = ?1",
            rusqlite::params![txn_amb],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(
        amb_count, 0,
        "an ambiguous imported category is not prefilled"
    );

    // A non-matching imported category leaves the transaction uncategorized.
    let staged2 = stage_with_imported_category(
        &worker,
        account_id,
        -700,
        "fp-nocat",
        Some("No Such Category Xyz"),
    );
    worker
        .dispatch(
            meta(),
            WriteCommand::CommitStaged {
                staged_transaction_id: staged2,
                force: false,
            },
        )
        .unwrap();
    let txn2: Uuid = {
        let conn = worker.read_connection().unwrap();
        conn.query_row(
            "SELECT committed_transaction_id FROM staged_transactions WHERE id = ?1",
            rusqlite::params![staged2.as_uuid()],
            |r| r.get(0),
        )
        .unwrap()
    };
    let count: i64 = {
        let conn = worker.read_connection().unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM transaction_categorizations WHERE transaction_id = ?1",
            rusqlite::params![txn2],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(
        count, 0,
        "a non-matching imported category does not categorize"
    );
}

/// cmx: re-committing with the same idempotency key is a replay — no second
/// ledger transaction (the command bus dedupes).
#[test]
fn committing_a_staged_transaction_is_idempotent() {
    let (_dir, worker) = worker();
    let account = sample_account();
    let account_id = account.id();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(account),
                opening_balance: None,
            },
        )
        .unwrap();
    let staged = stage_a_transaction(&worker, account_id, 5_000, "fp-pay");

    worker
        .dispatch(
            meta_with_key("commit-pay"),
            WriteCommand::CommitStaged {
                staged_transaction_id: staged,
                force: false,
            },
        )
        .unwrap();
    worker
        .dispatch(
            meta_with_key("commit-pay"),
            WriteCommand::CommitStaged {
                staged_transaction_id: staged,
                force: false,
            },
        )
        .unwrap();

    let conn = worker.read_connection().unwrap();
    let txns: i64 = conn
        .query_row("SELECT COUNT(*) FROM ledger_transactions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        txns, 1,
        "a replay must not write a second ledger transaction"
    );
}

/// cmx: a staged transaction whose fingerprint duplicates an already-committed
/// one is flagged for the Money Inbox — not committed, not dropped (ADR 0014 §3).
#[test]
fn a_duplicate_fingerprint_is_flagged_not_committed() {
    let (_dir, worker) = worker();
    let account = sample_account();
    let account_id = account.id();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(account),
                opening_balance: None,
            },
        )
        .unwrap();

    let first = stage_a_transaction(&worker, account_id, -1299, "same-fp");
    let second = stage_a_transaction(&worker, account_id, -1299, "same-fp");
    worker
        .dispatch(
            meta(),
            WriteCommand::CommitStaged {
                staged_transaction_id: first,
                force: false,
            },
        )
        .unwrap();
    worker
        .dispatch(
            meta(),
            WriteCommand::CommitStaged {
                staged_transaction_id: second,
                force: false,
            },
        )
        .unwrap();

    let conn = worker.read_connection().unwrap();
    let txns: i64 = conn
        .query_row("SELECT COUNT(*) FROM ledger_transactions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        txns, 1,
        "the duplicate must not post a second ledger transaction"
    );
    let status: String = conn
        .query_row(
            "SELECT commit_status FROM staged_transactions WHERE id = ?1",
            rusqlite::params![second.as_uuid()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(status, "flagged");
    let flagged: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM dedupe_decisions WHERE decision = 'flagged'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(flagged, 1);
}

/// dsq: a flagged staged duplicate surfaces exactly one Money Inbox item of
/// kind `imported_waiting_commit`, pointing at the flagged staged row and
/// carrying the dedupe reason in its payload. A cleanly-committed row is not an
/// item (resolution is intrinsic to the staged commit_status).
/// 4d8.20: `duplicate_candidates` returns the committed ledger transaction(s) a
/// flagged staged duplicate matches by fingerprint — the panel's counterpart column.
#[test]
fn duplicate_candidates_returns_the_committed_counterpart() {
    let (_dir, worker) = worker();
    let account = sample_account();
    let account_id = account.id();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(account),
                opening_balance: None,
            },
        )
        .unwrap();

    let first = stage_a_transaction(&worker, account_id, -1299, "same-fp");
    let second = stage_a_transaction(&worker, account_id, -1299, "same-fp");
    worker
        .dispatch(
            meta(),
            WriteCommand::CommitStaged {
                staged_transaction_id: first,
                force: false,
            },
        )
        .unwrap();
    worker
        .dispatch(
            meta(),
            WriteCommand::CommitStaged {
                staged_transaction_id: second,
                force: false,
            },
        )
        .unwrap();

    // `second` is flagged; its counterpart is the committed transaction from `first`.
    let candidates = worker.duplicate_candidates(second).unwrap();
    assert_eq!(
        candidates.len(),
        1,
        "one committed counterpart by fingerprint"
    );
    assert_eq!(candidates[0].amount, Money::new(-1299, Currency::Usd));

    // A freshly-staged row with a unique fingerprint has no committed counterpart.
    let lonely = stage_a_transaction(&worker, account_id, -500, "unique-fp");
    assert!(worker.duplicate_candidates(lonely).unwrap().is_empty());
}

#[test]
fn flagging_a_staged_duplicate_surfaces_a_money_inbox_item() {
    let (_dir, worker) = worker();
    let account = sample_account();
    let account_id = account.id();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(account),
                opening_balance: None,
            },
        )
        .unwrap();

    // No exceptions yet -> empty inbox (explicit empty state).
    assert!(worker.money_inbox_list().unwrap().is_empty());

    let first = stage_a_transaction(&worker, account_id, -1299, "same-fp");
    let second = stage_a_transaction(&worker, account_id, -1299, "same-fp");
    worker
        .dispatch(
            meta(),
            WriteCommand::CommitStaged {
                staged_transaction_id: first,
                force: false,
            },
        )
        .unwrap();
    assert!(
        worker
            .money_inbox_list()
            .unwrap()
            .iter()
            .all(|i| i.item_kind != "imported_waiting_commit"),
        "a cleanly committed row is not a flagged-duplicate inbox item \
             (it is now an unreviewed-transaction item, personal-cfo-4d8.7)"
    );

    worker
        .dispatch(
            meta(),
            WriteCommand::CommitStaged {
                staged_transaction_id: second,
                force: false,
            },
        )
        .unwrap();

    let items = worker.money_inbox_list().unwrap();
    let flagged: Vec<_> = items
        .iter()
        .filter(|i| i.item_kind == "imported_waiting_commit")
        .collect();
    assert_eq!(flagged.len(), 1, "the flagged duplicate is one inbox item");
    let item = flagged[0];
    assert_eq!(item.item_kind, "imported_waiting_commit");
    assert_eq!(item.target_table, "staged_transactions");
    assert_eq!(item.target_id, second.as_uuid());
    assert_eq!(item.item_id, second.as_uuid());
    assert!(
        item.payload_json
            .contains("duplicate of an already-committed transaction"),
        "payload carries the dedupe reason: {}",
        item.payload_json
    );
    assert!(
        item.payload_json.contains("\"merchant\":\"merchant\""),
        "payload carries the merchant: {}",
        item.payload_json
    );
}

/// dsq: the Money Inbox projection is deterministic — rebuilding from canonical
/// state twice yields byte-identical items (proves no agent writes / no clock
/// leakage into the item content).
#[test]
fn money_inbox_rebuild_is_deterministic() {
    let (_dir, worker) = worker();
    let account = sample_account();
    let account_id = account.id();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(account),
                opening_balance: None,
            },
        )
        .unwrap();
    let first = stage_a_transaction(&worker, account_id, -1299, "dup-fp");
    let second = stage_a_transaction(&worker, account_id, -1299, "dup-fp");
    worker
        .dispatch(
            meta(),
            WriteCommand::CommitStaged {
                staged_transaction_id: first,
                force: false,
            },
        )
        .unwrap();
    worker
        .dispatch(
            meta(),
            WriteCommand::CommitStaged {
                staged_transaction_id: second,
                force: false,
            },
        )
        .unwrap();

    let from_hook = worker.money_inbox_list().unwrap();
    let n1 = worker.rebuild_money_inbox().unwrap();
    let after_one = worker.money_inbox_list().unwrap();
    let n2 = worker.rebuild_money_inbox().unwrap();
    let after_two = worker.money_inbox_list().unwrap();

    assert_eq!(n1, 1);
    assert_eq!(n2, 1);
    assert_eq!(
        from_hook, after_one,
        "an explicit rebuild reproduces the pipeline-hook projection"
    );
    assert_eq!(after_one, after_two, "a second rebuild is byte-identical");
}

/// asqy: "import anyway" (force) commits a flagged duplicate to the ledger; the
/// inbox item then clears, the override is recorded, and a replay is a no-op.
#[test]
fn import_anyway_force_commits_a_flagged_duplicate() {
    let (_dir, worker) = worker();
    let account = sample_account();
    let account_id = account.id();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(account),
                opening_balance: None,
            },
        )
        .unwrap();

    let first = stage_a_transaction(&worker, account_id, -1299, "same-fp");
    let second = stage_a_transaction(&worker, account_id, -1299, "same-fp");
    worker
        .dispatch(
            meta(),
            WriteCommand::CommitStaged {
                staged_transaction_id: first,
                force: false,
            },
        )
        .unwrap();
    worker
        .dispatch(
            meta(),
            WriteCommand::CommitStaged {
                staged_transaction_id: second,
                force: false,
            },
        )
        .unwrap();
    assert_eq!(
        worker
            .money_inbox_list()
            .unwrap()
            .iter()
            .filter(|i| i.item_kind == "imported_waiting_commit")
            .count(),
        1,
    );

    // Import anyway (twice with the same key — the replay must be a no-op).
    worker
        .dispatch(
            meta_with_key("import-anyway"),
            WriteCommand::CommitStaged {
                staged_transaction_id: second,
                force: true,
            },
        )
        .unwrap();
    worker
        .dispatch(
            meta_with_key("import-anyway"),
            WriteCommand::CommitStaged {
                staged_transaction_id: second,
                force: true,
            },
        )
        .unwrap();

    assert!(
        worker
            .money_inbox_list()
            .unwrap()
            .iter()
            .all(|i| i.item_kind != "imported_waiting_commit"),
        "the committed row is no longer a flagged-duplicate inbox item"
    );
    assert_eq!(
        worker.account_balance(account_id).unwrap(),
        Some(Money::new(-2598, Currency::Usd)),
        "both -12.99 charges are now in the ledger"
    );
    let conn = worker.read_connection().unwrap();
    let txns: i64 = conn
        .query_row("SELECT COUNT(*) FROM ledger_transactions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        txns, 2,
        "force commit posts the second txn; replay adds none"
    );
    let status: String = conn
        .query_row(
            "SELECT commit_status FROM staged_transactions WHERE id = ?1",
            rusqlite::params![second.as_uuid()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(status, "committed");
    let overrides: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM dedupe_decisions
                  WHERE reason = 'user import-anyway override'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(overrides, 1, "the override intent is in the audit trail");
}

/// asqy: "skip" marks a flagged duplicate skipped with no ledger write; the
/// inbox item clears.
#[test]
fn skip_clears_a_flagged_duplicate_without_a_ledger_write() {
    let (_dir, worker) = worker();
    let account = sample_account();
    let account_id = account.id();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(account),
                opening_balance: None,
            },
        )
        .unwrap();

    let first = stage_a_transaction(&worker, account_id, -1299, "same-fp");
    let second = stage_a_transaction(&worker, account_id, -1299, "same-fp");
    worker
        .dispatch(
            meta(),
            WriteCommand::CommitStaged {
                staged_transaction_id: first,
                force: false,
            },
        )
        .unwrap();
    worker
        .dispatch(
            meta(),
            WriteCommand::CommitStaged {
                staged_transaction_id: second,
                force: false,
            },
        )
        .unwrap();
    assert_eq!(
        worker
            .money_inbox_list()
            .unwrap()
            .iter()
            .filter(|i| i.item_kind == "imported_waiting_commit")
            .count(),
        1,
    );

    worker
        .dispatch(
            meta(),
            WriteCommand::SkipStaged {
                staged_transaction_id: second,
            },
        )
        .unwrap();

    assert!(
        worker
            .money_inbox_list()
            .unwrap()
            .iter()
            .all(|i| i.item_kind != "imported_waiting_commit"),
        "the skipped row is no longer a flagged-duplicate inbox item"
    );
    assert_eq!(
        worker.account_balance(account_id).unwrap(),
        Some(Money::new(-1299, Currency::Usd)),
        "skip writes no ledger transaction, so only the first charge counts"
    );
    let conn = worker.read_connection().unwrap();
    let txns: i64 = conn
        .query_row("SELECT COUNT(*) FROM ledger_transactions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(txns, 1);
    let status: String = conn
        .query_row(
            "SELECT commit_status FROM staged_transactions WHERE id = ?1",
            rusqlite::params![second.as_uuid()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(status, "skipped");
}

/// byxe: a committed import carries its source detail onto the transactions
/// list — the original description as the memo, the normalized merchant as the
/// counterparty — so an imported row isn't a bare amount.
#[test]
fn a_committed_import_shows_its_detail_in_the_transactions_list() {
    let (_dir, worker) = worker();
    let account = sample_account();
    let account_id = account.id();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(account),
                opening_balance: None,
            },
        )
        .unwrap();

    let batch_id = SourceBatchId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateSourceBatch {
                id: batch_id,
                source_type: "csv".to_owned(),
                source_name: None,
                file_fingerprint: None,
                parser_version: None,
            },
        )
        .unwrap();
    let record_id = SourceRecordId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::AttachSourceRecord {
                id: record_id,
                batch_id,
                external_id: None,
                source_hash: "h-detail".to_owned(),
                normalized_json: "{}".to_owned(),
                parse_confidence_bps: None,
            },
        )
        .unwrap();
    let staged = {
        let conn = worker.read_connection().unwrap();
        crate::ingestion::stage_transaction(
            &conn,
            &crate::ingestion::NewStagedTransaction {
                source_record_id: record_id.as_uuid(),
                proposed_account_id: Some(account_id.as_uuid()),
                posted_at: "2026-06-20",
                transaction_date: None,
                amount_minor: -1299,
                currency: "USD",
                normalized_merchant: Some("starbucks"),
                description: Some("STARBUCKS STORE 123"),
                imported_category: None,
                txn_fingerprint: "fp-detail",
            },
        )
        .unwrap()
    };
    worker
        .dispatch(
            meta(),
            WriteCommand::CommitStaged {
                staged_transaction_id: StagedTransactionId::from_uuid(staged),
                force: false,
            },
        )
        .unwrap();

    let rows = worker.recent_transactions(10).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].memo.as_deref(), Some("STARBUCKS STORE 123"));
    assert_eq!(rows[0].counterparty.as_deref(), Some("starbucks"));
}

#[test]
fn entity_ids_are_stored_as_16_byte_blobs() {
    // personal-cfo-i2t: entity primary keys are stored as the compact 16-byte
    // UUID BLOB, not a 36-char TEXT string.
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.vault");
    let worker = DbWorker::open(&path, KEY).unwrap();
    worker.dispatch(meta(), create_account_cmd()).unwrap();

    let reader = open_keyed(&path, KEY).unwrap();
    for (table, column) in [
        ("accounts", "id"),
        ("accounts", "ledger_account_id"),
        ("ledger_accounts", "id"),
    ] {
        let (storage_class, len): (String, i64) = reader
            .query_row(
                &format!("SELECT typeof({column}), length({column}) FROM {table} LIMIT 1"),
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(storage_class, "blob", "{table}.{column} must be a BLOB");
        assert_eq!(len, 16, "{table}.{column} must be a 16-byte UUID");
    }
}

#[test]
fn vault_metadata_is_seeded_on_open_with_defaults() {
    // personal-cfo-2lm: a fresh vault always has a complete, readable
    // vault_metadata row with documented placeholder KDF defaults.
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.vault");
    let worker = DbWorker::open(&path, KEY).unwrap();

    let meta = worker.vault_metadata().unwrap();
    assert!(!meta.vault_id.is_nil());
    assert_eq!(meta.schema_version, migrations::CURRENT_VERSION);
    assert_eq!(meta.envelope_version, VAULT_ENVELOPE_VERSION);
    assert_eq!(meta.kdf_algorithm, DEFAULT_KDF_ALGORITHM);
    assert_eq!(meta.kdf_memory_kib, DEFAULT_KDF_MEMORY_KIB);
    assert_eq!(meta.kdf_time_cost, DEFAULT_KDF_TIME_COST);
    assert_eq!(meta.kdf_parallelism, DEFAULT_KDF_PARALLELISM);
    assert_eq!(meta.household_timezone, DEFAULT_HOUSEHOLD_TIMEZONE);
    assert_eq!(meta.manifest_pointer, None);

    // vault_id is the compact 16-byte BLOB, decryptable post-open.
    let reader = open_keyed(&path, KEY).unwrap();
    let (storage_class, len): (String, i64) = reader
        .query_row(
            "SELECT typeof(vault_id), length(vault_id) FROM vault_metadata WHERE singleton = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(storage_class, "blob");
    assert_eq!(len, 16);
}

/// personal-cfo-q329: `set_household_timezone` persists a valid IANA name, and setting it
/// actually shifts what `household_today` resolves to — proof this isn't merely a column
/// write, but reaches the same resolver 5ie.11's past-due queue and ku2hn's confirm-early
/// guard already consult. Also covers `personal-cfo-v53f`'s core assertion ("changing tz
/// re-anchors calendar dates") for the "today" boundary specifically — v53f's fuller AC
/// (mid-year audit retention, schedule reanchoring) stays open, as q329's design brief notes.
#[test]
fn set_household_timezone_persists_and_shifts_household_today() {
    let (_dir, worker) = worker();
    let conn = worker.read_connection().unwrap();

    assert_eq!(worker.vault_metadata().unwrap().household_timezone, "UTC");

    // 05:00 UTC on Jan 15 is still 21:00 Jan 14 in Los Angeles (winter, UTC-8) but
    // already 14:00 Jan 15 in Tokyo (UTC+9) — same instant, three different calendar
    // days depending which zone is authoritative.
    let as_of = chrono::DateTime::parse_from_rfc3339("2026-01-15T05:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let under_utc = crate::forecast::household_today_at(&conn, as_of).unwrap();
    assert_eq!(under_utc.to_string(), "2026-01-15");

    worker
        .set_household_timezone("America/Los_Angeles")
        .unwrap();
    assert_eq!(
        worker.vault_metadata().unwrap().household_timezone,
        "America/Los_Angeles",
    );
    let conn = worker.read_connection().unwrap();
    let under_la = crate::forecast::household_today_at(&conn, as_of).unwrap();
    assert_eq!(
        under_la.to_string(),
        "2026-01-14",
        "the same instant now resolves to the PREVIOUS calendar day under the new zone",
    );

    worker.set_household_timezone("Asia/Tokyo").unwrap();
    let conn = worker.read_connection().unwrap();
    let under_tokyo = crate::forecast::household_today_at(&conn, as_of).unwrap();
    assert_eq!(under_tokyo.to_string(), "2026-01-15");
}

/// personal-cfo-q329: `read_household_tz`'s downstream callers fail loudly on a bad
/// timezone (`crates/db-worker/src/forecast/events.rs`) — this is the gate that keeps a
/// typo or an unrecognized zone name out of the vault before it can ever reach them.
#[test]
fn set_household_timezone_rejects_an_unrecognized_iana_name() {
    let (_dir, worker) = worker();
    let result = worker.set_household_timezone("Not/A_Real_Zone");
    assert!(result.is_err(), "an invalid IANA name must be rejected");
    // The stored value is untouched — still the vault's original default.
    assert_eq!(worker.vault_metadata().unwrap().household_timezone, "UTC");
}

#[test]
fn vault_metadata_is_a_singleton() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.vault");
    let _worker = DbWorker::open(&path, KEY).unwrap();

    let conn = open_keyed(&path, KEY).unwrap();
    let result = conn.execute(
        "INSERT INTO vault_metadata (
                singleton, vault_id, schema_version, envelope_version,
                kdf_algorithm, kdf_memory_kib, kdf_time_cost, kdf_parallelism,
                household_timezone, manifest_pointer, created_at
            ) VALUES (1, ?1, 1, 1, 'argon2id', 1, 1, 1, 'UTC', NULL, 'now')",
        params![Uuid::now_v7()],
    );
    assert!(
        result.is_err(),
        "a second vault_metadata row must be rejected"
    );
}

#[test]
fn operation_log_is_append_only() {
    // personal-cfo-0s0: the op-log is immutable history (ADR 0011).
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.vault");
    let worker = DbWorker::open(&path, KEY).unwrap();
    worker.dispatch(meta(), create_account_cmd()).unwrap();

    let conn = open_keyed(&path, KEY).unwrap();
    assert!(
        conn.execute("UPDATE operation_log SET actor_id = 'x'", [])
            .is_err(),
        "UPDATE on operation_log must be rejected"
    );
    assert!(
        conn.execute("DELETE FROM operation_log", []).is_err(),
        "DELETE on operation_log must be rejected"
    );
}

#[test]
fn op_log_records_affected_entities_json() {
    // personal-cfo-0s0: every op-log row names the entities it touched.
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.vault");
    let worker = DbWorker::open(&path, KEY).unwrap();
    let account = sample_account();
    let account_id = account.id();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(account),
                opening_balance: None,
            },
        )
        .unwrap();

    let conn = open_keyed(&path, KEY).unwrap();
    let bytes: Vec<u8> = conn
        .query_row(
            "SELECT affected_entities FROM operation_log LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let json = String::from_utf8(bytes).unwrap();
    assert_eq!(
        json,
        format!(
            "[{{\"table\":\"accounts\",\"id\":\"{}\"}}]",
            account_id.as_uuid()
        )
    );
}

#[test]
fn projection_writes_matching_drift_checksums() {
    // personal-cfo-0s0: rebuild records a content checksum in both
    // projection_cursors and read_model_checksums, matching a fresh compute.
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.vault");
    let worker = DbWorker::open(&path, KEY).unwrap();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(sample_account()),
                opening_balance: Some(Money::new(1_000, Currency::Usd)),
            },
        )
        .unwrap();
    worker.rebuild_transaction_display().unwrap();

    let conn = open_keyed(&path, KEY).unwrap();
    let expected = projection::checksum(&conn).unwrap() as i64;
    let cursor_checksum: i64 = conn
            .query_row(
                "SELECT content_checksum FROM projection_cursors WHERE read_model_name = 'transaction_display'",
                [],
                |r| r.get(0),
            )
            .unwrap();
    let drift_checksum: i64 = conn
            .query_row(
                "SELECT current_checksum FROM read_model_checksums WHERE read_model_name = 'transaction_display'",
                [],
                |r| r.get(0),
            )
            .unwrap();
    assert_eq!(cursor_checksum, expected);
    assert_eq!(drift_checksum, expected);
}

#[test]
fn migrations_are_recorded_with_content_hash() {
    let (_dir, worker) = worker();
    let conn = worker.read_connection().unwrap();
    assert_eq!(user_version(&conn), migrations::CURRENT_VERSION);
    for m in migrations::MIGRATIONS {
        let hash: String = conn
            .query_row(
                "SELECT content_hash FROM schema_migrations WHERE version = ?1",
                [m.version],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hash, migrations::content_hash(m.up), "{}", m.name);
    }
}

#[test]
fn reopen_applies_no_new_migrations() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.vault");
    let count = || {
        DbWorker::open(&path, KEY)
            .unwrap()
            .read_connection()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap()
    };
    let first = count();
    assert_eq!(first, migrations::MIGRATIONS.len() as i64);
    assert_eq!(count(), first, "re-open must not re-apply migrations");
}

#[test]
fn down_then_up_reaches_current_with_integrity() {
    let (_dir, worker) = worker();
    let mut conn = worker.read_connection().unwrap();

    migrations::migrate_down(&mut conn, 1).unwrap();
    assert_eq!(user_version(&conn), 1);
    // The 0002 index is gone after rolling back to the baseline.
    let idx: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
                 WHERE type='index' AND name='idx_operation_log_correlation'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(idx, 0);

    let applied = migrations::run_migrations(&mut conn).unwrap();
    assert_eq!(
        applied,
        u64::try_from(migrations::CURRENT_VERSION - 1).unwrap(),
        "re-applies every migration above the rolled-back baseline",
    );
    assert_eq!(user_version(&conn), migrations::CURRENT_VERSION);
    let integrity: String = conn
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");
}

#[test]
fn down_migration_preserves_canonical_data() {
    // A realistic (non-empty) vault: a created account survives a down→up.
    let (_dir, worker) = worker();
    worker.dispatch(meta(), create_account_cmd()).unwrap();
    let mut conn = worker.read_connection().unwrap();

    migrations::migrate_down(&mut conn, 1).unwrap();
    migrations::run_migrations(&mut conn).unwrap();

    let accounts: i64 = conn
        .query_row("SELECT COUNT(*) FROM accounts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        accounts, 1,
        "canonical rows must survive a down/up round-trip"
    );
    let integrity: String = conn
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");
}

#[test]
fn rebuild_flag_triggers_read_model_rebuild() {
    let (_dir, worker) = worker();
    let mut conn = worker.read_connection().unwrap();
    // A synthetic migration flagged to rebuild read models (benign up SQL).
    let synthetic = [migrations::Migration {
        version: 9001,
        name: "test_rebuild_hook",
        up: "SELECT 1;",
        down: None,
        rebuilds_read_models: true,
    }];
    migrations::apply(&mut conn, &synthetic).unwrap();
    // The rebuild wrote the transaction-display cursor + checksum.
    let cursor: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM projection_cursors WHERE read_model_name = 'transaction_display'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        cursor, 1,
        "rebuilds_read_models must run the projection rebuild"
    );
}

#[test]
fn tampered_migration_hash_is_detected() {
    let (_dir, worker) = worker();
    let mut conn = worker.read_connection().unwrap();
    conn.execute(
        "UPDATE schema_migrations SET content_hash = 'tampered' WHERE version = 2",
        [],
    )
    .unwrap();
    assert!(
        migrations::run_migrations(&mut conn).is_err(),
        "a changed content hash on an applied migration must be rejected"
    );
}

#[test]
fn current_version_matches_highest_migration_and_downs_present() {
    let highest = migrations::MIGRATIONS
        .iter()
        .map(|m| m.version)
        .max()
        .unwrap();
    assert_eq!(migrations::CURRENT_VERSION, highest);
    // Every non-baseline migration is reversible (one-step rollback).
    for m in migrations::MIGRATIONS {
        if m.version > 1 {
            assert!(m.down.is_some(), "migration {} needs a down", m.name);
        }
    }
}

#[test]
fn panic_after_mutation_rolls_back_both_rows() {
    let (_dir, worker) = worker();
    // Force a panic between the domain mutation and the op-log insert.
    let outcome = worker.dispatch_inner(
        meta(),
        create_account_cmd(),
        Fault::PanicAt(FaultPoint::AfterMutation),
    );
    assert!(matches!(outcome, Err(DbError::WriterPanicked)));

    // Atomicity: the account, op-log, and idempotency rows all roll back.
    let conn = worker.read_connection().unwrap();
    assert_eq!(conn.count_accounts().unwrap(), 0, "mutation must roll back");
    assert_eq!(conn.operation_count().unwrap(), 0, "op-log must roll back");
    assert_eq!(
        worker.idempotency_key_count().unwrap(),
        0,
        "idempotency memo must roll back"
    );

    // Recovery: the worker refuses further writes until recovered.
    assert_eq!(worker.state(), WorkerState::CorruptNeedsRecovery);
    let after = worker.dispatch(meta(), create_account_cmd());
    assert!(matches!(
        after,
        Err(DbError::WorkerUnavailable(
            WorkerState::CorruptNeedsRecovery
        ))
    ));
}

#[test]
fn panic_after_oplog_also_rolls_back() {
    let (_dir, worker) = worker();
    let outcome = worker.dispatch_inner(
        meta(),
        create_account_cmd(),
        Fault::PanicAt(FaultPoint::AfterOpLog),
    );
    assert!(matches!(outcome, Err(DbError::WriterPanicked)));
    let conn = worker.read_connection().unwrap();
    // Commit never ran, so every insert rolls back together.
    assert_eq!(conn.count_accounts().unwrap(), 0);
    assert_eq!(conn.operation_count().unwrap(), 0);
    assert_eq!(worker.idempotency_key_count().unwrap(), 0);
}

/// The `TransactionRow` reads carry no correlated per-row subqueries
/// (personal-cfo-3fdd.3a): tags / split counts / import provenance are
/// pre-aggregated LEFT JOINs (and the tag *filter* is a PK-backed INNER JOIN),
/// so EXPLAIN QUERY PLAN shows no `CORRELATED SCALAR SUBQUERY` for the recent
/// list or the fully-filtered page.
#[test]
fn txn_row_reads_have_no_correlated_subqueries() {
    let (_dir, worker) = worker();
    let conn = worker.read_connection().unwrap();

    // EXPLAIN still requires every `?` bound; the values are irrelevant to the plan.
    let plan_of = |sql: &str, params: &[&dyn rusqlite::ToSql]| -> String {
        let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
        let lines: Vec<String> = stmt
            .query_map(params, |r| r.get::<_, String>(3))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        lines.join("\n")
    };

    let recent = format!(
        "SELECT {TXN_ROW_COLUMNS} {TXN_ROW_FROM}
         WHERE lt.voided_at IS NULL
         ORDER BY lt.occurred_at DESC, ol.op_seq DESC
         LIMIT 200"
    );
    let plan = plan_of(&recent, &[]);
    assert!(
        !plan.contains("CORRELATED SCALAR SUBQUERY"),
        "recent-transactions plan still has a correlated scalar subquery:\n{plan}"
    );

    // The fully-loaded page query (every filter active): still no correlated
    // scalar subquery, and the ORDER BY rides the occurred_at index (no sort).
    let paged = format!(
        "SELECT {TXN_ROW_COLUMNS} {TXN_ROW_FROM}
         JOIN transaction_tags tagf ON tagf.transaction_id = lt.id AND tagf.tag_id = ?
         WHERE lt.voided_at IS NULL
           AND (td.memo LIKE ? ESCAPE '\\' OR td.counterparty LIKE ? ESCAPE '\\'
                OR td.note LIKE ? ESCAPE '\\' OR a.name LIKE ? ESCAPE '\\')
           AND a.id = ?
           AND (tc.category_id IN (?, ?)
                OR lt.id IN (SELECT sl.transaction_id FROM split_lines sl
                              WHERE sl.category_id IN (?, ?)))
           AND lt.occurred_at >= ? AND lt.occurred_at < ?
         ORDER BY lt.occurred_at DESC, ol.op_seq DESC
         LIMIT ? OFFSET ?"
    );
    let uuid = Uuid::nil();
    let plan = plan_of(
        &paged,
        &[
            &uuid,
            &"%x%",
            &"%x%",
            &"%x%",
            &"%x%",
            &uuid,
            &uuid,
            &uuid,
            &uuid,
            &uuid,
            &"2026-06-01",
            &"2026-07-01",
            &10i64,
            &0i64,
        ],
    );
    assert!(
        !plan.contains("CORRELATED SCALAR SUBQUERY"),
        "transaction-page plan still has a correlated scalar subquery:\n{plan}"
    );

    // The total-count companion query (whole filtered set, no LIMIT) as well.
    let count = format!("SELECT COUNT(*) {TXN_ROW_FROM} WHERE lt.voided_at IS NULL");
    let plan = plan_of(&count, &[]);
    assert!(
        !plan.contains("CORRELATED SCALAR SUBQUERY"),
        "transaction-count plan still has a correlated scalar subquery:\n{plan}"
    );
}

/// personal-cfo-4d8.27.8.2 (ADR 0052 §3/§4): the spend rollup sums a category's whole
/// subtree, attributes a SPLIT transaction by its lines rather than its parent
/// categorization, nets refunds, and excludes transfers (internal movement, and two
/// postings that would otherwise be counted twice).
#[test]
fn spend_by_category_rolls_up_splits_refunds_and_excludes_transfers() {
    let (_dir, worker) = worker();
    let checking = AccountId::new();
    let savings = AccountId::new();
    for (id, name) in [(checking, "Checking"), (savings, "Savings")] {
        worker
            .dispatch(
                meta(),
                WriteCommand::CreateAccount {
                    account: Box::new(Account::new(
                        id,
                        LedgerAccountId::new(),
                        name,
                        CashflowRole::LiquidCash,
                        Currency::Usd,
                        AccountFlags::default(),
                    )),
                    opening_balance: Some(Money::new(1_000_000, Currency::Usd)),
                },
            )
            .unwrap();
    }

    // A 3-level slice of the taxonomy: root → child → grandchild.
    let conn = worker.read_connection().unwrap();
    let root: Uuid = conn
        .query_row(
            "SELECT id FROM categories WHERE parent_id IS NULL AND type = 'expense' LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let child: Uuid = conn
        .query_row(
            "SELECT id FROM categories WHERE parent_id = ?1 LIMIT 1",
            [root],
            |r| r.get(0),
        )
        .unwrap();
    drop(conn);
    let grandchild = CategoryId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateCategory {
                id: grandchild,
                parent_id: Some(CategoryId::from_uuid(child)),
                name: "Deep leaf".to_owned(),
                category_type: "expense".to_owned(),
                color: None,
                icon: None,
            },
        )
        .unwrap();

    let day = NaiveDate::from_ymd_opt(2026, 6, 15).unwrap();
    // $100 on the child, $300 on the grandchild, and a $50 refund against it.
    plant_categorized(
        &worker,
        checking,
        -10_000,
        day,
        CategoryId::from_uuid(child),
    );
    plant_categorized(
        &worker,
        checking,
        -30_000,
        day.succ_opt().unwrap(),
        grandchild,
    );
    plant_categorized(
        &worker,
        checking,
        5_000,
        day.succ_opt().unwrap().succ_opt().unwrap(),
        grandchild,
    );

    // A SPLIT transaction: $200 out, itemized $150 to the child and $50 to the
    // grandchild. Its own categorization points at the CHILD, so an implementation that
    // ignores split lines would put the whole $200 there — this is the case ADR 0052 §3
    // exists for, and the one users bother to itemize.
    plant_categorized(
        &worker,
        checking,
        -20_000,
        NaiveDate::from_ymd_opt(2026, 6, 20).unwrap(),
        CategoryId::from_uuid(child),
    );
    let split_txn = worker
        .recent_transactions(50)
        .unwrap()
        .into_iter()
        .find(|t| t.amount.minor_units() == -20_000)
        .expect("the split transaction")
        .transaction_id;
    worker
        .dispatch(
            meta(),
            WriteCommand::SetSplits {
                transaction_id: split_txn,
                lines: vec![
                    SplitLineInput {
                        amount: Money::new(-15_000, Currency::Usd),
                        category_id: Some(CategoryId::from_uuid(child)),
                        note: None,
                        tag_ids: vec![],
                    },
                    SplitLineInput {
                        amount: Money::new(-5_000, Currency::Usd),
                        category_id: Some(grandchild),
                        note: None,
                        tag_ids: vec![],
                    },
                ],
            },
        )
        .unwrap();

    // A transfer between the user's own accounts must not read as spending.
    worker
        .dispatch(
            meta(),
            WriteCommand::Transfer {
                source_account_id: checking,
                dest_account_id: savings,
                amount: Money::new(50_000, Currency::Usd),
                occurred_at: day.and_hms_opt(12, 0, 0).unwrap().and_utc(),
            },
        )
        .unwrap();

    let from = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
    let to = NaiveDate::from_ymd_opt(2026, 6, 30).unwrap();
    let breakdown = worker
        .spend_by_category(from, to, None, "USD", &SpendFilters::default())
        .unwrap();

    // The exclusions the chart could not place, reported by the SAME query that dropped
    // them (personal-cfo-90eg). Without this the chart and the list beside it simply
    // disagree, and the reader cannot tell a rule from a bug.
    assert_eq!(
        breakdown.transfers_minor, 50_000,
        "the $500 self-transfer is reported as excluded, not silently dropped",
    );

    let roots = breakdown.rows;
    let root_row = roots
        .iter()
        .find(|r| r.category_id == root)
        .expect("the root category");
    // 100 (child) + 300 (grandchild) − 50 (refund) + 200 (the split) = 550;
    // the transfer contributes 0.
    assert_eq!(
        root_row.total_minor, 55_000,
        "subtree total nets the refund and counts the split's lines"
    );
    assert!(root_row.has_children);

    // Drilling in: the child carries its own spend PLUS the grandchild's.
    let children = worker
        .spend_by_category(from, to, Some(root), "USD", &SpendFilters::default())
        .unwrap()
        .rows;
    let child_row = children
        .iter()
        .find(|r| r.category_id == child)
        .expect("the child category");
    assert_eq!(child_row.total_minor, 55_000);
    // 100 direct + 150 from the split's child line. A SPLIT-BLIND implementation would
    // read 300 here (the whole split against the parent's own categorization) and give
    // the grandchild nothing extra, so this is what pins ADR 0052 §3.
    assert_eq!(
        child_row.own_minor, 25_000,
        "own spend excludes descendants but includes this category's split lines"
    );

    // …and the grandchild carries its $50 split line on top of its direct spend.
    let leaves = worker
        .spend_by_category(from, to, Some(child), "USD", &SpendFilters::default())
        .unwrap()
        .rows;
    let leaf = leaves
        .iter()
        .find(|r| r.category_id == grandchild.as_uuid())
        .expect("the grandchild");
    assert_eq!(
        leaf.total_minor, 30_000,
        "300 direct - 50 refund + 50 from the split line"
    );

    // No transfer leaked in as a category total anywhere.
    let transfer_leak: i64 = roots.iter().map(|r| r.total_minor).sum::<i64>() - 55_000;
    assert_eq!(transfer_leak, 0, "a transfer is not spending");
}

/// personal-cfo-4d8.27.8.4 (ADR 0052 §2): the chart counts exactly the rows the LIST
/// would show for the same facets.
///
/// This is the promise the whole surface rests on — a user clicks a bar, gets a filtered
/// list, and the two must agree or neither is trustworthy. The aggregate is a UNION of
/// two shapes (postings for un-split transactions, `split_lines` for split ones) and the
/// split side joins neither `accounts` nor the review tables, so the filters are resolved
/// ONCE into a shared id set rather than reimplemented per branch. These assertions are
/// what would catch that set drifting from `read_transaction_page`.
#[test]
fn spend_by_category_facets_match_the_transaction_list() {
    let (_dir, worker) = worker();
    let checking = AccountId::new();
    let savings = AccountId::new();
    for (id, name) in [(checking, "Checking"), (savings, "Savings")] {
        worker
            .dispatch(
                meta(),
                WriteCommand::CreateAccount {
                    account: Box::new(Account::new(
                        id,
                        LedgerAccountId::new(),
                        name,
                        CashflowRole::LiquidCash,
                        Currency::Usd,
                        AccountFlags::default(),
                    )),
                    opening_balance: Some(Money::new(1_000_000, Currency::Usd)),
                },
            )
            .unwrap();
    }
    let conn = worker.read_connection().unwrap();
    let category: Uuid = conn
        .query_row(
            "SELECT id FROM categories WHERE parent_id IS NULL AND type = 'expense' LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    drop(conn);

    let day = NaiveDate::from_ymd_opt(2026, 6, 15).unwrap();
    let from = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
    let to = NaiveDate::from_ymd_opt(2026, 6, 30).unwrap();
    // $100 + $300 on Checking, $50 on Savings — all the same expense category, so any
    // difference in the totals below is the FILTER, never the taxonomy.
    plant_categorized(
        &worker,
        checking,
        -10_000,
        day,
        CategoryId::from_uuid(category),
    );
    plant_categorized(
        &worker,
        checking,
        -30_000,
        day.succ_opt().unwrap(),
        CategoryId::from_uuid(category),
    );
    plant_categorized(
        &worker,
        savings,
        -5_000,
        day,
        CategoryId::from_uuid(category),
    );

    let total_of = |filters: &SpendFilters| -> i64 {
        worker
            .spend_by_category(from, to, None, "USD", filters)
            .unwrap()
            .rows
            .iter()
            .map(|r| r.total_minor)
            .sum()
    };
    let page_query = |filters: &SpendFilters| TransactionPageQuery {
        query: filters.query.clone(),
        account_ids: filters
            .account_ids
            .iter()
            .map(|id| AccountId::from_uuid(*id))
            .collect(),
        category: None,
        tag_id: filters.tag_id.map(TagId::from_uuid),
        recurring_event_id: None,
        with_balances: false,
        from: Some(from),
        to: Some(to),
        unreviewed_only: filters.unreviewed_only,
        sort: TransactionSortOrder::NewestFirst,
        limit: 500,
        offset: 0,
    };
    // The list's own total for the same facets — every fixture row is a simple
    // single-currency expense, so negating the amounts is the honest comparison.
    let list_total_of = |filters: &SpendFilters| -> i64 {
        worker
            .transaction_page(&page_query(filters))
            .unwrap()
            .rows
            .iter()
            .map(|r| -r.amount.minor_units())
            .sum()
    };

    // Unfiltered: everything.
    let unfiltered = SpendFilters::default();
    assert_eq!(total_of(&unfiltered), 45_000);
    assert_eq!(total_of(&unfiltered), list_total_of(&unfiltered));

    // Account facet: Checking's two rows only.
    let by_account = SpendFilters {
        account_ids: vec![checking.as_uuid()],
        ..SpendFilters::default()
    };
    assert_eq!(total_of(&by_account), 40_000);

    // The SET case (personal-cfo-4d8.27.9.4, ADR 0057 §2). A multi-card Debt selection
    // must narrow the chart exactly as it narrows the list — an aggregate that honoured
    // only the FIRST id would pass every single-account assertion above while charting
    // one card of the several the list showed.
    let both_accounts = SpendFilters {
        account_ids: vec![checking.as_uuid(), savings.as_uuid()],
        ..SpendFilters::default()
    };
    assert_eq!(total_of(&both_accounts), 45_000);
    assert_eq!(total_of(&both_accounts), list_total_of(&both_accounts));
    assert!(
        total_of(&both_accounts) > total_of(&by_account),
        "two accounts total strictly more than one",
    );
    assert_eq!(
        total_of(&by_account),
        list_total_of(&by_account),
        "the chart's account total must equal the rows the list shows for it"
    );

    // Tag facet: tag exactly one $300 row.
    let tag = TagId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateTag {
                id: tag,
                name: "Holiday".to_owned(),
                color: None,
            },
        )
        .unwrap();
    let tagged = worker
        .recent_transactions(500)
        .unwrap()
        .into_iter()
        .find(|t| t.amount.minor_units() == -30_000)
        .unwrap();
    worker
        .dispatch(
            meta(),
            WriteCommand::SetTags {
                transaction_id: tagged.transaction_id,
                tag_ids: vec![tag],
            },
        )
        .unwrap();
    let by_tag = SpendFilters {
        tag_id: Some(tag.as_uuid()),
        ..SpendFilters::default()
    };
    assert_eq!(total_of(&by_tag), 30_000);
    assert_eq!(total_of(&by_tag), list_total_of(&by_tag));

    // Search facet spans the account name, exactly as the list does — so searching
    // "Savings" finds the row on that account even with nothing in its memo.
    let by_query = SpendFilters {
        query: Some("Savings".to_owned()),
        ..SpendFilters::default()
    };
    assert_eq!(total_of(&by_query), 5_000);
    assert_eq!(total_of(&by_query), list_total_of(&by_query));

    // Unreviewed-only: every fixture row is hand-entered, and the derived rule
    // (ADR 0032) treats a row with no import provenance as reviewed — so this is zero,
    // not "all of them". Getting the COALESCE backwards would show the full 45,000.
    let unreviewed = SpendFilters {
        unreviewed_only: true,
        ..SpendFilters::default()
    };
    assert_eq!(total_of(&unreviewed), 0);
    assert_eq!(total_of(&unreviewed), list_total_of(&unreviewed));
}

/// personal-cfo-4d8.27.8.2 (ADR 0052 §4): only EXPENSE categories are spend.
///
/// The two-user-posting shape alone is not enough. The dominant transfer in an
/// import-first app is a ONE-SIDED row — a "Credit Card Payment" the importer matched by
/// name — which has a system counter-posting and so slips past the transfer guard; it
/// would read as spending on top of the card's own charges, counting the same money
/// twice. A categorized paycheck is the mirror failure: with no sign filter it lands as
/// NEGATIVE spend, which would drag any percentage-of-total the chart draws.
#[test]
fn spend_by_category_ignores_transfer_and_income_categories() {
    let (_dir, worker) = worker();
    let checking = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(Account::new(
                    checking,
                    LedgerAccountId::new(),
                    "Checking",
                    CashflowRole::LiquidCash,
                    Currency::Usd,
                    AccountFlags::default(),
                )),
                opening_balance: Some(Money::new(1_000_000, Currency::Usd)),
            },
        )
        .unwrap();

    let conn = worker.read_connection().unwrap();
    let by_type = |kind: &str| -> Uuid {
        conn.query_row(
            "SELECT id FROM categories WHERE type = ?1 AND is_system = 1 LIMIT 1",
            [kind],
            |r| r.get(0),
        )
        .unwrap()
    };
    let transfer_category = by_type("transfer");
    let income_category = by_type("income");
    drop(conn);

    let day = NaiveDate::from_ymd_opt(2026, 6, 10).unwrap();
    // A one-sided card payment, categorized to a TRANSFER category (the import path).
    plant_categorized(
        &worker,
        checking,
        -50_000,
        day,
        CategoryId::from_uuid(transfer_category),
    );
    // A paycheck, categorized to an INCOME category.
    plant_categorized(
        &worker,
        checking,
        500_000,
        day.succ_opt().unwrap(),
        CategoryId::from_uuid(income_category),
    );

    let roots = worker
        .spend_by_category(
            NaiveDate::from_ymd_opt(2026, 6, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 6, 30).unwrap(),
            None,
            "USD",
            &SpendFilters::default(),
        )
        .unwrap();
    let total: i64 = roots.rows.iter().map(|r| r.total_minor).sum();
    assert_eq!(
        total, 0,
        "neither a transfer-categorized payment nor income is spending",
    );
    assert!(
        roots.rows.iter().all(|r| r.total_minor >= 0),
        "no category may report negative spend: {roots:?}",
    );
}

/// ADR 0055 / personal-cfo-4d8.27.6.3: apply promotes a scenario's events into base, and
/// revert restores base EXACTLY.
///
/// The round trip is the assertion that matters. Apply only ever INSERTs, so revert has
/// nothing to reconstruct from memory — it clears what was inserted and un-supersedes
/// what was displaced. If that is true, the base assumption set after revert is
/// indistinguishable from the set before apply.
#[test]
fn apply_scenario_promotes_events_and_revert_restores_base_exactly() {
    let (_dir, worker) = worker();

    // Base's active events as a comparable fingerprint: (kind, target, params).
    let base_fingerprint = |w: &DbWorker| -> Vec<(String, Option<Uuid>, String)> {
        let mut rows: Vec<_> = w
            .active_assumption_events(None)
            .unwrap()
            .into_iter()
            .map(|e| {
                (
                    e.kind.as_token().to_owned(),
                    e.target_entity_id,
                    e.params_json,
                )
            })
            .collect();
        rows.sort();
        rows
    };

    let bill = Uuid::now_v7();
    // A base override already in place, which the scenario will collide with.
    worker
        .record_assumption_event(&NewAssumptionEvent {
            id: Uuid::now_v7(),
            kind: AssumptionKind::BillAmount,
            target_entity_type: Some("recurring_event".to_owned()),
            target_entity_id: Some(bill),
            params_json: r#"{"new_amount_minor":100000}"#.to_owned(),
            source: AssumptionSource::UserOverride,
            scenario_id: None,
            origin_run_id: None,
        })
        .unwrap();
    let before = base_fingerprint(&worker);
    assert_eq!(before.len(), 1);

    let scenario = Uuid::now_v7();
    worker
        .create_scenario(&NewScenario {
            id: scenario,
            name: "Rent hike".to_owned(),
            description: None,
            base_run_id: None,
        })
        .unwrap();
    // One colliding event (same kind + target as base) and one untargeted, so the
    // NULL-target supersede path is exercised too — `= NULL` is never true, so a `=`
    // instead of `IS` would silently never supersede untargeted kinds.
    worker
        .record_assumption_event(&NewAssumptionEvent {
            id: Uuid::now_v7(),
            kind: AssumptionKind::BillAmount,
            target_entity_type: Some("recurring_event".to_owned()),
            target_entity_id: Some(bill),
            params_json: r#"{"new_amount_minor":250000}"#.to_owned(),
            source: AssumptionSource::UserOverride,
            scenario_id: Some(scenario),
            origin_run_id: None,
        })
        .unwrap();
    worker
        .record_assumption_event(&NewAssumptionEvent {
            id: Uuid::now_v7(),
            kind: AssumptionKind::InflationRate,
            target_entity_type: None,
            target_entity_id: None,
            params_json: r#"{"annual_bps":400}"#.to_owned(),
            source: AssumptionSource::UserOverride,
            scenario_id: Some(scenario),
            origin_run_id: None,
        })
        .unwrap();

    // ── apply ────────────────────────────────────────────────────────────────────
    worker
        .dispatch(
            meta(),
            WriteCommand::ApplyScenario {
                scenario_id: scenario,
            },
        )
        .unwrap();

    let applied = base_fingerprint(&worker);
    assert_eq!(
        applied.len(),
        2,
        "both scenario events are promoted; the colliding base override is superseded",
    );
    assert!(
        applied
            .iter()
            .any(|(_, _, params)| params.contains("250000")),
        "the scenario's amount is what base now uses",
    );
    assert!(
        !applied
            .iter()
            .any(|(_, _, params)| params.contains("100000")),
        "the superseded base override is no longer active",
    );
    let view = worker.scenario(scenario).unwrap().unwrap();
    assert!(view.applied_at.is_some(), "the scenario is marked applied");

    // Applying twice would promote a second copy of everything, and the handle would only
    // undo the newer set.
    assert!(worker
        .dispatch(
            meta(),
            WriteCommand::ApplyScenario {
                scenario_id: scenario
            },
        )
        .is_err());

    // ── revert ───────────────────────────────────────────────────────────────────
    worker
        .dispatch(
            meta(),
            WriteCommand::RevertScenarioApply {
                scenario_id: scenario,
            },
        )
        .unwrap();

    assert_eq!(
        base_fingerprint(&worker),
        before,
        "base is byte-identical to before the apply",
    );
    assert!(worker
        .scenario(scenario)
        .unwrap()
        .unwrap()
        .applied_at
        .is_none());
    // The scenario itself is untouched — its own events are still there to re-apply.
    assert_eq!(worker.scenario(scenario).unwrap().unwrap().event_count, 2);

    // Reverting again is an error, not a quiet success: a revert that reports success
    // without reverting anything is how a user comes to believe a change was undone.
    assert!(worker
        .dispatch(
            meta(),
            WriteCommand::RevertScenarioApply {
                scenario_id: scenario
            },
        )
        .is_err());
}

/// Apply → revert → apply again works.
///
/// Guards a defect found reviewing the IPC layer: the apply command originally carried an
/// idempotency key derived from the scenario id, which is STABLE — and `dispatch` replays
/// a known key rather than re-running it. So the second apply would have been memoized as
/// a replay and silently done nothing, leaving the user with a button that appeared dead
/// after they had once undone. The key is now fresh per invocation; double-apply is
/// prevented by the domain guard instead, which is where it belongs.
#[test]
fn a_reverted_scenario_can_be_applied_again() {
    let (_dir, worker) = worker();
    let scenario = Uuid::now_v7();
    worker
        .create_scenario(&NewScenario {
            id: scenario,
            name: "Re-appliable".to_owned(),
            description: None,
            base_run_id: None,
        })
        .unwrap();
    worker
        .record_assumption_event(&NewAssumptionEvent {
            id: Uuid::now_v7(),
            kind: AssumptionKind::InflationRate,
            target_entity_type: None,
            target_entity_id: None,
            params_json: r#"{"annual_bps":400}"#.to_owned(),
            source: AssumptionSource::UserOverride,
            scenario_id: Some(scenario),
            origin_run_id: None,
        })
        .unwrap();

    let apply = || WriteCommand::ApplyScenario {
        scenario_id: scenario,
    };
    let revert = || WriteCommand::RevertScenarioApply {
        scenario_id: scenario,
    };

    worker.dispatch(meta(), apply()).unwrap();
    worker.dispatch(meta(), revert()).unwrap();
    worker.dispatch(meta(), apply()).unwrap();

    assert!(worker
        .scenario(scenario)
        .unwrap()
        .unwrap()
        .applied_at
        .is_some());
    let base = worker.active_assumption_events(None).unwrap();
    assert_eq!(
        base.len(),
        1,
        "exactly one active promoted event — the first apply's copy stayed cleared",
    );
    assert!(base[0].params_json.contains("400"));
}

/// A scenario with nothing active does not become "applied" — marking it so would offer
/// a revert that undoes nothing (ADR 0055 §5's reasoning).
#[test]
fn apply_scenario_rejects_a_scenario_with_no_active_events() {
    let (_dir, worker) = worker();
    let scenario = Uuid::now_v7();
    worker
        .create_scenario(&NewScenario {
            id: scenario,
            name: "Empty".to_owned(),
            description: None,
            base_run_id: None,
        })
        .unwrap();
    assert!(worker
        .dispatch(
            meta(),
            WriteCommand::ApplyScenario {
                scenario_id: scenario
            },
        )
        .is_err());
    assert!(worker
        .scenario(scenario)
        .unwrap()
        .unwrap()
        .applied_at
        .is_none());
}

/// personal-cfo-abhr: a promoted base event names the scenario it came from.
///
/// Without this, an override created by APPLYING a scenario is indistinguishable from one
/// the user typed directly — and the two need different explanations on a bill. The
/// column exists from migration v47; this asserts it survives the apply and reaches the
/// read the UI uses.
#[test]
fn a_promoted_base_event_carries_the_scenario_it_came_from() {
    let (_dir, worker) = worker();
    let bill = Uuid::now_v7();
    let scenario = Uuid::now_v7();
    worker
        .create_scenario(&NewScenario {
            id: scenario,
            name: "Rent hike".to_owned(),
            description: None,
            base_run_id: None,
        })
        .unwrap();
    worker
        .record_assumption_event(&NewAssumptionEvent {
            id: Uuid::now_v7(),
            kind: AssumptionKind::BillAmount,
            target_entity_type: Some("recurring_event".to_owned()),
            target_entity_id: Some(bill),
            params_json: r#"{"new_amount_minor":250000}"#.to_owned(),
            source: AssumptionSource::UserOverride,
            scenario_id: Some(scenario),
            origin_run_id: None,
        })
        .unwrap();
    // A base override the user set directly, on a different bill.
    let typed_bill = Uuid::now_v7();
    worker
        .record_assumption_event(&NewAssumptionEvent {
            id: Uuid::now_v7(),
            kind: AssumptionKind::BillAmount,
            target_entity_type: Some("recurring_event".to_owned()),
            target_entity_id: Some(typed_bill),
            params_json: r#"{"new_amount_minor":9900}"#.to_owned(),
            source: AssumptionSource::UserOverride,
            scenario_id: None,
            origin_run_id: None,
        })
        .unwrap();

    worker
        .dispatch(
            meta(),
            WriteCommand::ApplyScenario {
                scenario_id: scenario,
            },
        )
        .unwrap();

    let base = worker.active_assumption_events(None).unwrap();
    let promoted = base
        .iter()
        .find(|e| e.target_entity_id == Some(bill))
        .expect("the promoted event is a base event");
    assert_eq!(
        promoted.promoted_from_scenario_id,
        Some(scenario),
        "a promoted event names the scenario responsible",
    );
    let typed = base
        .iter()
        .find(|e| e.target_entity_id == Some(typed_bill))
        .expect("the hand-set override is still base");
    assert_eq!(
        typed.promoted_from_scenario_id, None,
        "an override the user set directly is NOT attributed to a scenario",
    );
}

/// Composing several scenarios (personal-cfo-4d8.27.6.4, ADR 0059).
///
/// Two halves, because they fail differently: non-overlapping scenarios must BOTH apply
/// (a set that silently used only the first would pass any single-scenario test), and an
/// overlapping pair must resolve by the user's stacking order rather than by creation
/// time.
#[test]
fn several_scenarios_compose_with_selection_order_as_precedence() {
    let (_dir, worker) = worker();
    let checking = AccountId::new();
    create_role_account(&worker, checking, "Checking", CashflowRole::LiquidCash);

    let rent = RecurringEventId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateRecurringBill {
                event_id: rent,
                contract_id: BillContractId::new(),
                name: "Rent".to_owned(),
                amount: Money::new(200_000, Currency::Usd),
                bill_type: "rent_mortgage".to_owned(),
                frequency: Frequency::Monthly,
                anchor: NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(),
                autopay_account_id: Some(checking),
                description: None,
                source_merchant_key: None,
                category_id: None,
                tag_ids: Vec::new(),
            },
        )
        .unwrap();
    let gym = RecurringEventId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateRecurringBill {
                event_id: gym,
                contract_id: BillContractId::new(),
                name: "Gym".to_owned(),
                amount: Money::new(5_000, Currency::Usd),
                bill_type: "subscription".to_owned(),
                frequency: Frequency::Monthly,
                anchor: NaiveDate::from_ymd_opt(2026, 9, 5).unwrap(),
                autopay_account_id: Some(checking),
                description: None,
                source_merchant_key: None,
                category_id: None,
                tag_ids: Vec::new(),
            },
        )
        .unwrap();

    let make = |id: Uuid, name: &str| {
        worker
            .create_scenario(&NewScenario {
                id,
                name: name.to_owned(),
                description: None,
                base_run_id: None,
            })
            .unwrap();
    };
    let a = Uuid::now_v7();
    let b = Uuid::now_v7();
    make(a, "Rent hike");
    make(b, "Rent hike, worse");

    // A: rent → $2,500. Authored FIRST.
    worker
        .record_forecast_assumption(&ForecastAssumptionSpec {
            id: Uuid::now_v7(),
            scenario_id: Some(a),
            target_entity_id: Some(rent.as_uuid()),
            params: AssumptionParams::BillAmount {
                new_amount_minor: 250_000,
                effective_date: None,
                end_date: None,
            },
        })
        .unwrap();
    // A also cuts the gym — a field B never touches, so it must survive composition.
    worker
        .record_forecast_assumption(&ForecastAssumptionSpec {
            id: Uuid::now_v7(),
            scenario_id: Some(a),
            target_entity_id: Some(gym.as_uuid()),
            params: AssumptionParams::BillAmount {
                new_amount_minor: 0,
                effective_date: None,
                end_date: None,
            },
        })
        .unwrap();
    // B: rent → $3,000. Authored SECOND, so creation order and selection order can be
    // made to disagree below.
    worker
        .record_forecast_assumption(&ForecastAssumptionSpec {
            id: Uuid::now_v7(),
            scenario_id: Some(b),
            target_entity_id: Some(rent.as_uuid()),
            params: AssumptionParams::BillAmount {
                new_amount_minor: 300_000,
                effective_date: None,
                end_date: None,
            },
        })
        .unwrap();

    let conn = worker.read_connection().unwrap();
    let day = NaiveDate::from_ymd_opt(2026, 9, 15).unwrap();
    let rent_under = |sel: &[Uuid]| {
        crate::forecast_overrides::entity_overrides(&conn, sel)
            .unwrap()
            .get(&rent.as_uuid())
            .and_then(|o| o.amount_for(day, 200_000))
    };
    let gym_under = |sel: &[Uuid]| {
        crate::forecast_overrides::entity_overrides(&conn, sel)
            .unwrap()
            .get(&gym.as_uuid())
            .and_then(|o| o.amount_for(day, 5_000))
    };

    // 1. Non-overlapping: selecting A alone changes BOTH of its targets.
    assert_eq!(rent_under(&[a]), Some(250_000));
    assert_eq!(gym_under(&[a]), Some(0));

    // 2. Composed: B's rent wins over A's, AND A's gym cut survives — the half a
    //    "last scenario replaces everything" implementation would silently drop.
    assert_eq!(rent_under(&[a, b]), Some(300_000), "later-stacked wins");
    assert_eq!(
        gym_under(&[a, b]),
        Some(0),
        "A's untouched field still applies"
    );

    // 3. THE assertion that separates selection-order from creation-order: reverse the
    //    stack and A wins, even though B was authored later. Under the old
    //    ORDER BY created_at rule this would still return B's 300,000.
    assert_eq!(
        rent_under(&[b, a]),
        Some(250_000),
        "selection order is the precedence, not creation time",
    );

    // 4. Empty selection is base only — inverting that would apply every scenario at once.
    assert_eq!(rent_under(&[]), None);
}
