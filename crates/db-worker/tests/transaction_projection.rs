//! Transaction-display read model (beads personal-cfo-9x4 + lxj): the read model
//! is derived only by the projection runner, rebuild == incremental, and rebuild
//! is fast.

use std::time::Instant;

use core_ledger::{Account, AccountFlags, AccountId, CashflowRole, LedgerAccountId};
use core_money::{Currency, Money};
use db_worker::{ActorType, CommandMeta, DbWorker, WriteCommand};
use tempfile::TempDir;
use uuid::Uuid;

const KEY: &str = "correct horse battery staple";

fn meta() -> CommandMeta {
    CommandMeta {
        command_id: Uuid::now_v7(),
        correlation_id: Uuid::now_v7(),
        causation_id: None,
        actor_type: ActorType::User,
        actor_id: "tester".to_owned(),
        idempotency_key: Uuid::now_v7().to_string(),
    }
}

fn worker() -> (TempDir, DbWorker) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("vault.db");
    let worker = DbWorker::open(&path, KEY).unwrap();
    (dir, worker)
}

fn create_with_opening_balance(worker: &DbWorker, name: &str, cents: i64) -> AccountId {
    let account = Account::new(
        AccountId::new(),
        LedgerAccountId::new(),
        name,
        CashflowRole::LiquidCash,
        Currency::Usd,
        AccountFlags::default(),
    );
    let id = account.id();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(account),
                opening_balance: Some(Money::new(cents, Currency::Usd)),
            },
        )
        .unwrap();
    id
}

#[test]
fn command_dispatch_does_not_populate_the_read_model() {
    let (_dir, worker) = worker();
    create_with_opening_balance(&worker, "Checking", 10_000);
    // The command path writes canonical tables only; the read model stays empty
    // until the projection runner runs.
    assert_eq!(worker.transaction_display_rows().unwrap().len(), 0);
}

#[test]
fn rebuild_projects_one_row_per_user_posting() {
    let (_dir, worker) = worker();
    create_with_opening_balance(&worker, "Checking", 10_000);
    create_with_opening_balance(&worker, "Savings", 25_000);

    let written = worker.rebuild_transaction_display().unwrap();
    assert_eq!(
        written, 2,
        "one display row per user-account opening posting"
    );

    let rows = worker.transaction_display_rows().unwrap();
    assert_eq!(rows.len(), 2);
    // The system (equity) postings are excluded; amounts reflect the openings.
    let mut amounts: Vec<i64> = rows.iter().map(|r| r.amount.minor_units()).collect();
    amounts.sort_unstable();
    assert_eq!(amounts, vec![10_000, 25_000]);
}

#[test]
fn account_without_opening_balance_yields_no_rows() {
    let (_dir, worker) = worker();
    let account = Account::new(
        AccountId::new(),
        LedgerAccountId::new(),
        "Empty",
        CashflowRole::LiquidCash,
        Currency::Usd,
        AccountFlags::default(),
    );
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(account),
                opening_balance: None,
            },
        )
        .unwrap();
    worker.rebuild_transaction_display().unwrap();
    assert_eq!(worker.transaction_display_rows().unwrap().len(), 0);
}

#[test]
fn incremental_matches_full_rebuild_checksum() {
    let (_dir, worker) = worker();

    // Build the read model incrementally as canonical data arrives.
    create_with_opening_balance(&worker, "Checking", 10_000);
    worker.project_transaction_display_incremental().unwrap();
    create_with_opening_balance(&worker, "Savings", 25_000);
    worker.project_transaction_display_incremental().unwrap();

    let incremental = worker.transaction_display_checksum().unwrap();

    // A full rebuild from canonical must produce identical content.
    worker.rebuild_transaction_display().unwrap();
    let rebuilt = worker.transaction_display_checksum().unwrap();

    assert_eq!(
        incremental, rebuilt,
        "incremental projection must not drift from rebuild"
    );
    // The cursor advanced to the op-log head.
    assert!(worker.transaction_projection_cursor().unwrap() > 0);
}

#[test]
fn rebuild_is_fast() {
    // Perf sanity. The §lxj 50k-transaction golden-fixture baseline is deferred
    // to when the synthetic-data generator (personal-cfo-9ujs) lands; here we
    // assert a modest set rebuilds well under the 2s budget.
    let (_dir, worker) = worker();
    for i in 0..300 {
        create_with_opening_balance(&worker, &format!("Account {i}"), 1_000 + i);
    }
    let start = Instant::now();
    let written = worker.rebuild_transaction_display().unwrap();
    let elapsed = start.elapsed();
    assert_eq!(written, 300);
    assert!(elapsed.as_secs() < 2, "rebuild took {elapsed:?}");
}
