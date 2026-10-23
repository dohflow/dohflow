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
        idempotency_key: Uuid::new_v4().to_string(),
        issued_at: db_worker::IssuedAt::now(),
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

/// Create `n` accounts with an opening balance each (a worker with `n` rows to project).
fn worker_with_accounts(n: i64) -> (TempDir, DbWorker) {
    let (dir, worker) = worker();
    for i in 0..n {
        create_with_opening_balance(&worker, &format!("Account {i}"), 1_000 + i);
    }
    (dir, worker)
}

/// The fastest of `runs` full rebuilds, and the rows each wrote. The minimum ignores load spikes.
fn fastest_rebuild(worker: &DbWorker, runs: usize) -> (u64, std::time::Duration) {
    let mut best = std::time::Duration::MAX;
    let mut written = 0;
    for _ in 0..runs {
        let start = Instant::now();
        written = worker.rebuild_transaction_display().unwrap();
        best = best.min(start.elapsed());
    }
    (written, best)
}

/// Rebuild is fast, judged by how it SCALES in the same run: rebuilding 300 accounts' rows must
/// cost about 5 times what rebuilding 60 does (linear), and a quadratic rebuild would cost 25
/// times. Both sides are the same CPU-bound work, so a slow or loaded machine slows them alike.
/// The old `< 2 s` wall-clock assertion failed once under load (personal-cfo-g3m.19), and
/// comparing against the time to create the rows was no better (creating is disk-bound, rebuild
/// is CPU-bound, so load moves them differently). The absolute budget is
/// `rebuild_stays_inside_the_absolute_budget`, an `#[ignore]`d test run by the non-required perf
/// job; the last bound here is a generous hang guard.
#[test]
fn rebuild_is_fast() {
    // Perf sanity. The §lxj 50k-transaction golden-fixture baseline is deferred to when the
    // synthetic-data generator (personal-cfo-9ujs) lands.
    let (_small_dir, small) = worker_with_accounts(60);
    let (_large_dir, large) = worker_with_accounts(300);
    // Interleave the two sizes so a load spike lands on both.
    let mut small_best = std::time::Duration::MAX;
    let mut large_best = std::time::Duration::MAX;
    for _ in 0..3 {
        let (written, t) = fastest_rebuild(&small, 3);
        assert_eq!(written, 60);
        small_best = small_best.min(t);
        let (written, t) = fastest_rebuild(&large, 3);
        assert_eq!(written, 300);
        large_best = large_best.min(t);
    }
    let ratio = large_best.as_secs_f64() / small_best.as_secs_f64();
    println!(
        "rebuild: 60 rows {small_best:?}, 300 rows {large_best:?} ({ratio:.1}x for 5x the rows)"
    );
    assert!(
        ratio < 12.0,
        "rebuilding 5x the rows cost {ratio:.1}x (linear is 5x, quadratic 25x): \
         {small_best:?} -> {large_best:?}"
    );
    assert!(
        large_best.as_secs() < 60,
        "rebuild did not finish in a minute: {large_best:?}"
    );
}

/// The ABSOLUTE budget: rebuilding the read model for 300 accounts takes under 2 s. Not a
/// required check, because an absolute wall-clock number varies with the machine and its load;
/// run by the non-required perf job in ci.yml (`cargo test ... -- --ignored`).
#[test]
#[ignore = "absolute perf budget; run by the non-required perf job (cargo test ... -- --ignored)"]
fn rebuild_stays_inside_the_absolute_budget() {
    let (_dir, worker) = worker_with_accounts(300);
    let (written, took) = fastest_rebuild(&worker, 1);
    assert_eq!(written, 300);
    println!("transaction rebuild ABSOLUTE: {took:?}");
    assert!(took.as_secs() < 2, "rebuild took {took:?} (budget 2 s)");
}
