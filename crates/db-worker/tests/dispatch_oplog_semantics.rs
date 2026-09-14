//! Dispatch, idempotency, op-log, and write-path property tests
//! (moved verbatim from the old in-file `lib.rs` tests module).

mod common;

use chrono::Utc;
use common::*;
use core_ledger::{Account, AccountFlags, AccountId, CashflowRole, LedgerAccountId, TransactionId};
use core_money::{Currency, Money};
use db_worker::*;
use proptest::prelude::*;
use uuid::Uuid;

#[test]
fn audit_events_are_append_only() {
    let (_dir, worker) = worker();
    worker.record_audit_event(&meta(), "test_event").unwrap();
    assert_eq!(worker.audit_event_count("test_event").unwrap(), 1);
    // Counts are per event type.
    assert_eq!(worker.audit_event_count("other").unwrap(), 0);

    // The table is immutable (append-only triggers, like the operation log):
    // UPDATE and DELETE abort even on a connection that could otherwise write.
    let conn = worker.read_connection().unwrap();
    assert!(
        conn.execute("UPDATE audit_events SET event_type = 'x'", [])
            .is_err(),
        "audit_events UPDATE must be blocked by the append-only trigger"
    );
    assert!(
        conn.execute("DELETE FROM audit_events", []).is_err(),
        "audit_events DELETE must be blocked by the append-only trigger"
    );
}

#[test]
fn missing_metadata_is_rejected() {
    let (_dir, worker) = worker();
    let mut m = meta();
    m.idempotency_key = "  ".to_owned();
    assert!(matches!(
        worker.dispatch(m, create_account_cmd()),
        Err(DbError::MissingMetadata("idempotency_key"))
    ));
}

#[test]
fn dispatch_applies_and_records_oplog() {
    let (_dir, worker) = worker();
    let outcome = worker.dispatch(meta(), create_account_cmd()).unwrap();
    assert!(matches!(outcome, Outcome::Applied { .. }));
    let conn = worker.read_connection().unwrap();
    assert_eq!(conn.count_accounts().unwrap(), 1);
    assert_eq!(conn.operation_count().unwrap(), 1);
}

#[test]
fn same_command_id_replays_without_new_rows() {
    let (_dir, worker) = worker();
    let m = meta();
    let first = worker.dispatch(m.clone(), create_account_cmd()).unwrap();
    let second = worker.dispatch(m, create_account_cmd()).unwrap();
    assert!(matches!(first, Outcome::Applied { .. }));
    assert!(matches!(second, Outcome::Replayed { .. }));
    let conn = worker.read_connection().unwrap();
    assert_eq!(conn.count_accounts().unwrap(), 1, "replay must not write");
    assert_eq!(conn.operation_count().unwrap(), 1);
}

#[test]
fn replay_is_a_noop_for_each_command_type() {
    // Exercises the idempotency mechanism across all command types
    // (plan personal-cfo-02i: "proptest covers >= 3 command types").
    for variant in 0u8..4 {
        let (_dir, worker) = worker();
        let cmd = cmd_of(variant);
        let key = Uuid::now_v7().to_string();

        let first = worker.dispatch(meta_with_key(&key), cmd.clone()).unwrap();
        // A retry: different command_id, SAME idempotency_key.
        let second = worker.dispatch(meta_with_key(&key), cmd.clone()).unwrap();

        assert!(
            matches!(first, Outcome::Applied { .. }),
            "variant {variant}"
        );
        assert!(
            matches!(second, Outcome::Replayed { .. }),
            "variant {variant}"
        );
        // Replay wrote nothing new.
        let conn = worker.read_connection().unwrap();
        assert_eq!(conn.operation_count().unwrap(), 1, "variant {variant}");
        assert_eq!(
            worker.idempotency_key_count().unwrap(),
            1,
            "variant {variant}"
        );
    }
}

#[test]
fn replayed_outcome_returns_the_original_op_seq() {
    let (_dir, worker) = worker();
    let key = Uuid::now_v7().to_string();
    let first = worker
        .dispatch(meta_with_key(&key), create_account_cmd())
        .unwrap();
    let second = worker
        .dispatch(meta_with_key(&key), create_account_cmd())
        .unwrap();
    let (Outcome::Applied { op_seq: a } | Outcome::Replayed { op_seq: a }) = first;
    let (Outcome::Applied { op_seq: b } | Outcome::Replayed { op_seq: b }) = second;
    assert_eq!(a, b, "replay must return the original result ref");
}

#[test]
fn expired_idempotency_keys_reap_without_touching_oplog() {
    let (_dir, worker) = worker();
    worker.dispatch(meta(), create_account_cmd()).unwrap();
    assert_eq!(worker.idempotency_key_count().unwrap(), 1);

    // Reap everything (cutoff far in the future).
    let cutoff = (Utc::now() + chrono::Duration::days(3650)).to_rfc3339();
    let reaped = worker.gc_idempotency_keys_before(&cutoff).unwrap();
    assert_eq!(reaped, 1);
    assert_eq!(worker.idempotency_key_count().unwrap(), 0);

    // Referential integrity: the op-log and the account are untouched.
    let conn = worker.read_connection().unwrap();
    assert_eq!(conn.operation_count().unwrap(), 1);
    assert_eq!(conn.count_accounts().unwrap(), 1);
}

#[test]
fn property_mixed_commands_keep_the_book_balanced_and_rebuild_is_deterministic() {
    let (_dir, worker) = worker();
    // Deterministic LCG so the run is reproducible (no `rand` dependency).
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = move || {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (seed >> 33) as u32
    };

    let mut account_ids: Vec<AccountId> = Vec::new();
    for _ in 0..1_000 {
        if account_ids.is_empty() || next() % 5 == 0 {
            let account = Account::new(
                AccountId::new(),
                LedgerAccountId::new(),
                "acct",
                CashflowRole::LiquidCash,
                Currency::Usd,
                AccountFlags::default(),
            );
            account_ids.push(account.id());
            worker
                .dispatch(
                    meta(),
                    WriteCommand::CreateAccount {
                        account: Box::new(account),
                        opening_balance: None,
                    },
                )
                .unwrap();
        } else {
            let idx = next() as usize % account_ids.len();
            let mut cents = i64::from(next() % 100_001) - 50_000;
            if cents == 0 {
                cents = 1;
            }
            worker
                .dispatch(
                    meta(),
                    WriteCommand::RecordTransaction {
                        transaction_id: TransactionId::new(),
                        account_id: account_ids[idx],
                        amount: Money::new(cents, Currency::Usd),
                        occurred_at: Utc::now(),
                    },
                )
                .unwrap();
        }
    }

    let conn = worker.read_connection().unwrap();
    // (a) Every transaction's postings net to zero.
    let unbalanced: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM (SELECT transaction_id FROM ledger_postings \
                 GROUP BY transaction_id HAVING SUM(minor_units) <> 0)",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(unbalanced, 0, "every transaction must balance");
    // (b) Book-wide double-entry invariant.
    let book: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(minor_units),0) FROM ledger_postings",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(book, 0, "the whole book must net to zero");

    // (c) The read-model rebuild is deterministic over this transaction set.
    worker.rebuild_transaction_display().unwrap();
    let first = worker.transaction_display_checksum().unwrap();
    worker.rebuild_transaction_display().unwrap();
    let second = worker.transaction_display_checksum().unwrap();
    assert_eq!(first, second, "rebuild must be deterministic");
}

#[test]
fn one_thousand_command_burst_completes_under_budget() {
    use std::time::{Duration, Instant};

    let (_dir, worker) = worker();
    let account = sample_account();
    let id = account.id();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(account),
                opening_balance: None,
            },
        )
        .unwrap();

    let start = Instant::now();
    for _ in 0..1_000 {
        worker
            .dispatch(
                meta(),
                WriteCommand::RecordTransaction {
                    transaction_id: TransactionId::new(),
                    account_id: id,
                    amount: Money::new(100, Currency::Usd),
                    occurred_at: Utc::now(),
                },
            )
            .unwrap();
    }
    let elapsed = start.elapsed();
    // §27 target is < 5s on dev hardware; assert a generous CI-safe ceiling
    // as the regression guard.
    assert!(
        elapsed < Duration::from_secs(30),
        "1k-command burst took {elapsed:?}"
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    #[test]
    fn replay_with_same_idempotency_key_writes_nothing_new(
        variant in 0u8..4,
        key in "[a-zA-Z0-9]{8,24}",
    ) {
        let (_dir, worker) = worker();
        let cmd = cmd_of(variant);

        let first = worker.dispatch(meta_with_key(&key), cmd.clone()).unwrap();
        let second = worker.dispatch(meta_with_key(&key), cmd.clone()).unwrap();

        let first_applied = matches!(first, Outcome::Applied { .. });
        let second_replayed = matches!(second, Outcome::Replayed { .. });
        prop_assert!(first_applied);
        prop_assert!(second_replayed);

        let conn = worker.read_connection().unwrap();
        prop_assert_eq!(conn.operation_count().unwrap(), 1);
        prop_assert_eq!(worker.idempotency_key_count().unwrap(), 1);
    }
}
