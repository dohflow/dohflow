//! Concurrency: long-running reads on separate connections do not block the
//! single writer (WAL), and a burst of writes all succeed (plan §3.2, DoD §1.2).

use std::sync::Arc;
use std::thread;

use core_ledger::{Account, AccountFlags, AccountId, CashflowRole, LedgerAccountId};
use core_money::Currency;
use db_worker::{
    AccountQuery, ActorType, CommandMeta, DbWorker, LedgerQuery, Outcome, WriteCommand,
};
use tempfile::TempDir;
use uuid::Uuid;

const KEY: &str = "correct horse battery staple";
const WRITES: usize = 100;

fn meta() -> CommandMeta {
    CommandMeta {
        command_id: Uuid::now_v7(),
        correlation_id: Uuid::now_v7(),
        causation_id: None,
        actor_type: ActorType::User,
        actor_id: "stress".to_owned(),
        idempotency_key: Uuid::now_v7().to_string(),
    }
}

fn create_account() -> WriteCommand {
    WriteCommand::CreateAccount {
        account: Box::new(Account::new(
            AccountId::new(),
            LedgerAccountId::new(),
            "Checking",
            CashflowRole::LiquidCash,
            Currency::Usd,
            AccountFlags::default(),
        )),
        opening_balance: None,
    }
}

#[test]
fn long_reads_do_not_block_writes() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("vault.db");
    let worker = Arc::new(DbWorker::open(&path, KEY).unwrap());

    // Reader thread: hammer reads on its own connection while writes happen.
    let reader = {
        let worker = Arc::clone(&worker);
        thread::spawn(move || {
            let conn = worker.read_connection().unwrap();
            let mut seen = 0u64;
            for _ in 0..1000 {
                seen = conn
                    .count_accounts()
                    .expect("read should never be blocked out");
            }
            seen
        })
    };

    for _ in 0..WRITES {
        let outcome = worker
            .dispatch(meta(), create_account())
            .expect("write should succeed");
        assert!(matches!(outcome, Outcome::Applied { .. }));
    }

    reader.join().expect("reader thread panicked");

    let conn = worker.read_connection().unwrap();
    assert_eq!(conn.count_accounts().unwrap(), WRITES as u64);
    assert_eq!(conn.operation_count().unwrap(), WRITES as u64);

    // personal-cfo-0s0: op-log sequence ids are strictly increasing and
    // contiguous (1..=WRITES) — monotonic per vault, no gaps.
    let seqs: Vec<i64> = conn
        .prepare("SELECT op_seq FROM operation_log ORDER BY op_seq")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(seqs, (1..=WRITES as i64).collect::<Vec<_>>());
}
