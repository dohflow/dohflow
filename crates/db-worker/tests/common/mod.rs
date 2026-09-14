//! Shared fixtures for the db-worker integration tests (split out of the
//! old in-file `lib.rs` tests module). Pub API only; each test file pulls
//! what it needs, so unused items here are expected.
#![allow(dead_code)]

use chrono::NaiveDate;
use core_ledger::{
    Account, AccountFlags, AccountId, AccountSubtype, BillContractId, CashflowRole, CategoryId,
    IncomeSourceId, LedgerAccountId, RecurringEventId, TransactionId,
};
use core_money::{Currency, Money};
use db_worker::*;
use pay_schedule::Frequency;
use rusqlite::params;
use tempfile::TempDir;
use uuid::Uuid;

pub const KEY: &str = "correct horse battery staple";

pub fn worker() -> (TempDir, DbWorker) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.vault");
    let worker = DbWorker::open(&path, KEY).unwrap();
    (dir, worker)
}

pub fn meta() -> CommandMeta {
    meta_with_key(&Uuid::now_v7().to_string())
}

pub fn meta_with_key(idempotency_key: &str) -> CommandMeta {
    CommandMeta {
        command_id: Uuid::now_v7(),
        correlation_id: Uuid::now_v7(),
        causation_id: None,
        actor_type: ActorType::User,
        actor_id: "tester".to_owned(),
        idempotency_key: idempotency_key.to_owned(),
    }
}

pub fn sample_account() -> Account {
    Account::new(
        AccountId::new(),
        LedgerAccountId::new(),
        "Checking",
        CashflowRole::LiquidCash,
        Currency::Usd,
        AccountFlags::default(),
    )
}

pub fn create_account_cmd() -> WriteCommand {
    WriteCommand::CreateAccount {
        account: Box::new(sample_account()),
        opening_balance: None,
    }
}

pub fn cmd_of(variant: u8) -> WriteCommand {
    match variant % 4 {
        0 => create_account_cmd(),
        1 => WriteCommand::UpdateAccount {
            id: AccountId::new(),
            name: "Renamed".to_owned(),
        },
        2 => WriteCommand::ArchiveAccount(AccountId::new()),
        _ => WriteCommand::ReinstateAccount(AccountId::new()),
    }
}

pub fn income_cmd(deposit: Option<AccountId>) -> WriteCommand {
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
pub fn variable_category(worker: &DbWorker) -> CategoryId {
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
pub fn plant_categorized(
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

pub fn checking(account_id: AccountId) -> Account {
    Account::new(
        account_id,
        LedgerAccountId::new(),
        "Checking",
        CashflowRole::LiquidCash,
        Currency::Usd,
        AccountFlags::default(),
    )
}

pub fn loan_terms(
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

/// Create a `loan_payment` recurring bill paid from `paying` for `amount` minor units.
pub fn create_loan_payment_bill(worker: &DbWorker, name: &str, amount: i64, paying: AccountId) {
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateRecurringBill {
                event_id: RecurringEventId::new(),
                contract_id: BillContractId::new(),
                name: name.to_owned(),
                amount: Money::new(amount, Currency::Usd),
                bill_type: "loan_payment".to_owned(),
                frequency: Frequency::Monthly,
                anchor: NaiveDate::from_ymd_opt(2026, 7, 15).unwrap(),
                autopay_account_id: Some(paying),
                description: None,
                source_merchant_key: None,
                category_id: None,
                tag_ids: Vec::new(),
            },
        )
        .unwrap();
}

/// Create an active `loan_liability` account with payment terms and a $5,000 owed balance
/// (so it actually emits a forecast payment — the detection ignores paid-off loans).
pub fn create_loan_with_terms(
    worker: &DbWorker,
    name: &str,
    philosophy: RepaymentPhilosophy,
    fixed: Option<i64>,
    paying: AccountId,
) -> AccountId {
    let id = AccountId::new();
    create_role_account(worker, id, name, CashflowRole::LoanLiability);
    worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: id,
                terms: loan_terms(philosophy, fixed, paying),
            },
        )
        .unwrap();
    // A liability owes when its balance is negative (ADR 0027).
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            id,
            Money::new(-500_000, Currency::Usd),
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        )
        .unwrap();
    id
}

pub fn bill_cmd(autopay: Option<AccountId>) -> WriteCommand {
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

pub fn new_event(id: Uuid, kind: AssumptionKind, scenario_id: Option<Uuid>) -> NewAssumptionEvent {
    NewAssumptionEvent {
        id,
        kind,
        target_entity_type: Some("income_source".to_owned()),
        target_entity_id: Some(Uuid::now_v7()),
        params_json: r#"{"net_minor":500000}"#.to_owned(),
        source: AssumptionSource::UserOverride,
        scenario_id,
        origin_run_id: None,
    }
}

// ===== Type-based cash-tier rollups (ADR 0028, personal-cfo-9dgg) =====

pub fn liquid_subtype_cmd(
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

/// Record a spend transaction and attach a raw merchant `counterparty` (as imports do),
/// returning its id. Callers plant in increasing date order so the row is locatable.
pub fn record_with_counterparty(
    worker: &DbWorker,
    account_id: AccountId,
    cents: i64,
    date: NaiveDate,
    counterparty: &str,
) -> TransactionId {
    worker
        .dispatch(
            meta(),
            WriteCommand::RecordTransaction {
                transaction_id: TransactionId::new(),
                account_id,
                amount: Money::new(cents, Currency::Usd),
                occurred_at: date.and_hms_opt(12, 0, 0).unwrap().and_utc(),
            },
        )
        .unwrap();
    let txn = worker
        .recent_transactions(500)
        .unwrap()
        .iter()
        .find(|t| t.occurred_at.date_naive() == date)
        .expect("just-planted txn")
        .transaction_id;
    worker
        .read_connection()
        .unwrap()
        .execute(
            "INSERT OR REPLACE INTO transaction_details
                    (transaction_id, memo, counterparty, created_at)
                 VALUES (?1, NULL, ?2, ?3)",
            params![txn.as_uuid(), counterparty, "2026-01-01T00:00:00Z"],
        )
        .unwrap();
    txn
}

/// The `(category_id, source)` assigned to a transaction, or `None` if uncategorized.
pub fn category_of(worker: &DbWorker, txn: TransactionId) -> Option<(Uuid, String)> {
    worker
        .read_connection()
        .unwrap()
        .query_row(
            "SELECT category_id, source FROM transaction_categorizations WHERE transaction_id = ?1",
            [txn.as_uuid()],
            |r| Ok((r.get::<_, Uuid>(0)?, r.get::<_, String>(1)?)),
        )
        .ok()
}

pub fn liquid_account_cmd(id: AccountId, currency: Currency, opening_minor: i64) -> WriteCommand {
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

pub fn create_role_account(worker: &DbWorker, id: AccountId, name: &str, role: CashflowRole) {
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

pub fn sample_debt_terms(paying: Option<AccountId>) -> DebtTermsInput {
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
