//! The device-local Sync state (personal-cfo-bxsbz.1): the outbox is recorded in the operation-log
//! transaction while Sync is enabled, and not at all while it is off. Fake data only.

use super::*;
use crate::sync_state::{
    decode_row_keys, outbox_issued_at, OutboxIssuedAtError, DEVICE_LOCAL_OPERATION_LOG_COLUMNS,
};

/// Enable Sync on this device the way the base image will (a `sync_base` row), at server seq 7.
fn enable_sync(worker: &DbWorker, base_seq: i64) {
    worker
        .lock()
        .conn
        .execute(
            "INSERT INTO sync_base (singleton, base_seq, base_digest, state, updated_at)
             VALUES (1, ?1, zeroblob(32), 'Ready', '2026-11-04T00:00:00Z')",
            [base_seq],
        )
        .unwrap();
}

fn count(worker: &DbWorker, table: &str) -> i64 {
    worker
        .lock()
        .conn
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

/// A mixed run: two accounts, an income source, a deposit and a refused command.
fn mixed_run(worker: &DbWorker) {
    worker.dispatch(meta(), create_account_cmd()).unwrap();
    let account = AccountId::new();
    worker
        .dispatch(meta(), liquid_account_cmd(account, Currency::Usd, 50_000))
        .unwrap();
    worker.dispatch(meta(), income_cmd(Some(account))).unwrap();
    worker
        .dispatch(
            meta(),
            deposit_cmd(account, 2_500, NaiveDate::from_ymd_opt(2026, 7, 1).unwrap()),
        )
        .unwrap();
    // A command the kernel refuses writes nothing, in the log or the outbox.
    assert!(worker
        .dispatch(
            meta(),
            deposit_cmd(
                AccountId::new(),
                100,
                NaiveDate::from_ymd_opt(2026, 7, 2).unwrap()
            ),
        )
        .is_err());
}

#[test]
fn a_vault_that_never_enabled_sync_has_no_outbox_rows_after_a_mixed_run() {
    let (_dir, worker) = worker();
    mixed_run(&worker);
    assert!(count(&worker, "operation_log") >= 4);
    assert_eq!(count(&worker, "sync_base"), 0);
    assert_eq!(count(&worker, "sync_outbox"), 0);
    assert_eq!(count(&worker, "outbox_queue"), 0);
    let stamped: i64 = worker
        .lock()
        .conn
        .query_row(
            "SELECT count(*) FROM operation_log WHERE applied_base_seq IS NOT NULL OR rebased_from IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stamped, 0, "with Sync off nothing is stamped");
}

#[test]
fn while_sync_is_enabled_each_apply_records_one_pending_entry_with_its_write_set_keys() {
    let (_dir, worker) = worker();
    enable_sync(&worker, 7);
    let m = meta();
    let command_id = m.command_id;
    let Outcome::Applied { op_seq } = worker.dispatch(m, create_account_cmd()).unwrap() else {
        panic!("applied");
    };
    let rows = worker.last_write_set().unwrap().rows;
    assert!(!rows.is_empty());

    let (state, stored_op, keys, sealed, bound): (
        String,
        i64,
        Vec<u8>,
        Option<Vec<u8>>,
        Option<i64>,
    ) = worker
        .lock()
        .conn
        .query_row(
            "SELECT state, op_seq, write_set_keys, sealed_container, bound_seq
                   FROM sync_outbox WHERE command_id = ?1",
            [command_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap();
    assert_eq!(state, "Pending");
    assert_eq!(stored_op, op_seq);
    assert_eq!(
        decode_row_keys(&keys).unwrap(),
        rows,
        "the keys are the write set's, in order"
    );
    assert!(sealed.is_none() && bound.is_none());

    // The log row says which base it was applied on.
    let applied: Option<i64> = worker
        .lock()
        .conn
        .query_row(
            "SELECT applied_base_seq FROM operation_log WHERE op_seq = ?1",
            [op_seq],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(applied, Some(7));

    mixed_run(&worker);
    // One entry per applied command, none for the refused one.
    assert_eq!(
        count(&worker, "sync_outbox"),
        count(&worker, "operation_log")
    );
}

#[test]
fn a_replayed_idempotency_key_records_no_second_entry() {
    let (_dir, worker) = worker();
    enable_sync(&worker, 0);
    let key = Uuid::new_v4().to_string();
    let first = worker
        .dispatch(meta_with_key(&key), create_account_cmd())
        .unwrap();
    let again = worker
        .dispatch(meta_with_key(&key), create_account_cmd())
        .unwrap();
    assert!(matches!(first, Outcome::Applied { .. }));
    assert!(matches!(again, Outcome::Replayed { .. }));
    assert_eq!(count(&worker, "sync_outbox"), 1);
    assert_eq!(count(&worker, "operation_log"), 1);
}

#[test]
fn the_outbox_row_and_the_log_row_commit_or_roll_back_together() {
    let (_dir, worker) = worker();
    enable_sync(&worker, 3);
    let outcome = worker.dispatch_inner(
        meta(),
        create_account_cmd(),
        Fault::PanicAt(FaultPoint::AfterOpLog),
    );
    assert!(matches!(outcome, Err(DbError::WriterPanicked)));
    assert_eq!(count(&worker, "operation_log"), 0);
    assert_eq!(
        count(&worker, "sync_outbox"),
        0,
        "no outbox row without its log row"
    );
}

#[test]
fn the_operation_log_stays_append_only_with_the_new_columns() {
    let (_dir, worker) = worker();
    worker.dispatch(meta(), create_account_cmd()).unwrap();
    let conn = &worker.lock().conn;
    for statement in [
        "UPDATE operation_log SET applied_base_seq = 1",
        "UPDATE operation_log SET rebased_from = 1",
        "DELETE FROM operation_log",
    ] {
        let error = conn.execute(statement, []).unwrap_err().to_string();
        assert!(error.contains("append-only"), "{statement}: {error}");
    }
}

#[test]
fn the_new_log_columns_are_nullable_and_checked() {
    let (_dir, worker) = worker();
    let conn = worker.read_connection().unwrap();
    let columns = operation_log_columns(&conn);
    let tail: Vec<(&str, &str, bool)> = columns
        .iter()
        .rev()
        .take(2)
        .rev()
        .map(|(n, t, nn)| (n.as_str(), t.as_str(), *nn))
        .collect();
    assert_eq!(
        tail,
        vec![
            ("applied_base_seq", "INTEGER", false),
            ("rebased_from", "INTEGER", false)
        ]
    );
    // A negative base or a zero origin is refused when the row is written.
    let mut conn = worker.lock();
    for (column, value) in [("applied_base_seq", -1), ("rebased_from", 0)] {
        let error = conn
            .conn
            .execute(
                &format!(
                    "INSERT INTO operation_log
                       (command_id, correlation_id, actor_type, actor_id, idempotency_key,
                        operation_type, affected_entities, node_id, vault_schema_version,
                        hlc_timestamp, created_at, {column})
                     VALUES (zeroblob(16), zeroblob(16), 'user', 'u', 'k', 'test', zeroblob(1),
                             zeroblob(16), 66, 1, '2026-11-04T00:00:00Z', {value})"
                ),
                [],
            )
            .unwrap_err()
            .to_string();
        assert!(error.contains("CHECK"), "{column}: {error}");
    }
    let _ = &mut conn;
}

#[test]
fn the_outbox_states_carry_exactly_the_columns_they_need() {
    let (_dir, worker) = worker();
    let conn = &worker.lock().conn;
    let insert = |state: &str, sealed: &str, bound: &str, result: &str| {
        conn.execute(
            &format!(
                "INSERT INTO sync_outbox (command_id, op_seq, state, write_set_keys, sealed_container,
                                         bound_seq, result_ref, created_at)
                 VALUES (randomblob(16), abs(random()), '{state}', zeroblob(0), {sealed}, {bound}, {result},
                         '2026-11-04T00:00:00Z')"
            ),
            [],
        )
    };
    assert!(insert("Pending", "NULL", "NULL", "NULL").is_ok());
    assert!(insert("Approved", "zeroblob(4)", "NULL", "NULL").is_ok());
    assert!(insert("Bound", "zeroblob(4)", "9", "zeroblob(1)").is_ok());
    // Each state refuses the columns of another, and an unknown state is refused.
    assert!(insert("Pending", "zeroblob(4)", "NULL", "NULL").is_err());
    assert!(insert("Approved", "NULL", "NULL", "NULL").is_err());
    assert!(insert("Approved", "zeroblob(4)", "9", "NULL").is_err());
    // An accepted command may have no result (bxsbz.5, migration 68), and a present one is 1 to 64
    // bytes.
    assert!(insert("Bound", "zeroblob(4)", "9", "NULL").is_ok());
    assert!(insert("Bound", "zeroblob(4)", "9", "zeroblob(0)").is_err());
    assert!(insert("Bound", "zeroblob(4)", "9", "zeroblob(65)").is_err());
    assert!(insert("Bound", "zeroblob(4)", "NULL", "NULL").is_err());
    assert!(insert("Bound", "NULL", "9", "zeroblob(1)").is_err());
    assert!(insert("Done", "NULL", "NULL", "NULL").is_err());
}

#[test]
fn the_base_state_matches_its_digest_and_the_queue_takes_only_adr_0073_reasons() {
    let (_dir, worker) = worker();
    let conn = &worker.lock().conn;
    let base = |state: &str, digest: &str| {
        conn.execute("DELETE FROM sync_base", []).unwrap();
        conn.execute(
            &format!(
                "INSERT INTO sync_base (singleton, base_seq, base_digest, state, updated_at)
                 VALUES (1, 0, {digest}, '{state}', '2026-11-04T00:00:00Z')"
            ),
            [],
        )
    };
    assert!(base("NoBase", "NULL").is_ok());
    assert!(base("Ready", "zeroblob(32)").is_ok());
    assert!(base("Rebasing", "zeroblob(32)").is_ok());
    assert!(
        base("NoBase", "zeroblob(32)").is_err(),
        "NoBase has no digest"
    );
    assert!(base("Ready", "NULL").is_err(), "a base has a digest");
    assert!(
        base("Ready", "zeroblob(31)").is_err(),
        "the digest is 32 bytes"
    );
    assert!(base("Done", "zeroblob(32)").is_err());
    assert!(
        conn.execute(
            "INSERT INTO sync_base (singleton, base_seq, base_digest, state, updated_at)
                  VALUES (2, 0, NULL, 'NoBase', 'x')",
            []
        )
        .is_err(),
        "a singleton"
    );

    let queue = |reason: &str| {
        conn.execute(
            &format!(
                "INSERT INTO outbox_queue (command_id, correlation_id, reason, origin_node_id, queued_at,
                                          envelope, audit_evidence)
                 VALUES (randomblob(16), randomblob(16), '{reason}', randomblob(16), 'x', zeroblob(1), zeroblob(1))"
            ),
            [],
        )
    };
    assert!(queue("queue_choose").is_ok());
    assert!(queue("queue_edit").is_ok());
    assert!(queue("auto").is_err(), "an auto command is never queued");
}

#[test]
fn row_keys_round_trip_and_a_truncated_list_is_not_guessed_at() {
    let rows = vec![
        ("accounts".to_owned(), vec![1u8; 16]),
        ("ledger_transactions".to_owned(), vec![9, 8, 7]),
        ("settings".to_owned(), b"user.reporting_currency".to_vec()),
    ];
    let bytes = crate::sync_state::encode_row_keys(&rows);
    assert_eq!(decode_row_keys(&bytes).unwrap(), rows);
    assert_eq!(
        decode_row_keys(&[]).unwrap(),
        Vec::<(String, Vec<u8>)>::new()
    );
    for cut in 1..bytes.len() {
        let decoded = decode_row_keys(&bytes[..cut]);
        // A prefix that happens to end on an entry boundary decodes to fewer entries; any other
        // cut is refused outright.
        if let Some(prefix) = decoded {
            assert!(prefix.len() < rows.len());
            assert_eq!(prefix, rows[..prefix.len()]);
        }
    }
    assert!(
        decode_row_keys(&[0, 2, 0xff, 0xfe, 0, 0, 0, 0]).is_none(),
        "a table name is UTF-8"
    );
}

#[test]
fn the_device_local_log_columns_exist_and_are_listed() {
    let (_dir, worker) = worker();
    let conn = worker.read_connection().unwrap();
    let present: Vec<String> = operation_log_columns(&conn)
        .into_iter()
        .map(|c| c.0)
        .collect();
    for column in DEVICE_LOCAL_OPERATION_LOG_COLUMNS {
        assert!(
            present.iter().any(|p| p == column),
            "{column} is an operation_log column"
        );
    }
}

// ---- the issue time of an unpushed command (AC0 of personal-cfo-bxsbz.4) ---------------------

const FIXED_ISSUED_AT: i64 = 1_790_000_000_123_456;

fn meta_issued_at(micros: i64) -> CommandMeta {
    let mut m = meta();
    m.issued_at = IssuedAt::from_unix_micros(micros);
    m
}

#[test]
fn the_outbox_row_records_the_commands_original_issue_time() {
    let (_dir, worker) = worker();
    enable_sync(&worker, 7);
    let m = meta_issued_at(FIXED_ISSUED_AT);
    worker.dispatch(m.clone(), create_account_cmd()).unwrap();

    let read = outbox_issued_at(&worker.lock().conn, &m.command_id).unwrap();
    assert_eq!(read.as_unix_micros(), FIXED_ISSUED_AT);
}

#[test]
fn the_issue_time_column_is_nullable_and_bounded_like_an_envelopes() {
    let (_dir, worker) = worker();
    let conn = worker.lock();
    let insert = |value: &str| {
        conn.conn.execute(
            &format!(
                "INSERT INTO sync_outbox
                   (command_id, op_seq, state, write_set_keys, created_at, issued_at)
                 VALUES (randomblob(16), abs(random()), 'Pending', zeroblob(0),
                         '2026-11-06T00:00:00Z', {value})"
            ),
            [],
        )
    };
    assert!(insert("NULL").is_ok(), "a row from before the column");
    assert!(insert("0").is_ok());
    assert!(insert("253402300799999999").is_ok());
    for refused in ["-1", "253402300800000000"] {
        assert!(insert(refused).is_err(), "{refused}");
    }
}

#[test]
fn a_row_with_no_recorded_issue_time_is_refused_by_name_and_so_is_a_missing_row() {
    let (_dir, worker) = worker();
    let id = Uuid::now_v7();
    worker
        .lock()
        .conn
        .execute(
            "INSERT INTO sync_outbox (command_id, op_seq, state, write_set_keys, created_at)
             VALUES (?1, 1, 'Pending', zeroblob(0), '2026-11-06T00:00:00Z')",
            [id],
        )
        .unwrap();
    let conn = &worker.lock().conn;
    assert!(matches!(
        outbox_issued_at(conn, &id),
        Err(OutboxIssuedAtError::Missing { command_id }) if command_id == id
    ));
    let other = Uuid::now_v7();
    assert!(matches!(
        outbox_issued_at(conn, &other),
        Err(OutboxIssuedAtError::NotInOutbox { command_id }) if command_id == other
    ));
}

/// The commitment an envelope would bind for `meta`, with the device id of the issuing vault.
fn commitment_of(meta: &CommandMeta, device: Uuid) -> sync_core::header::MetadataCommitment {
    sync_core::envelope::ImmutableMeta {
        command_id: *meta.command_id.as_bytes(),
        correlation_id: *meta.correlation_id.as_bytes(),
        causation_id: meta.causation_id.map(|id| *id.as_bytes()),
        actor_type: 1,
        actor_id: meta.actor_id.clone(),
        idempotency_key: sync_core::envelope::IdempotencyKey::parse(&meta.idempotency_key).unwrap(),
        issued_at: meta.issued_at.as_unix_micros(),
        device_id: *device.as_bytes(),
    }
    .commitment()
    .unwrap()
}

#[test]
fn a_command_replayed_from_its_recorded_issue_time_mints_the_same_ids_and_stamps_and_commits_the_same_meta(
) {
    let (dir, live) = worker();
    enable_sync(&live, 7);
    // State the command is applied on: an income source to tombstone.
    let income = income_cmd(None);
    let WriteCommand::CreateIncomeSource { id: income_id, .. } = &income else {
        panic!("an income source command");
    };
    let income_id = *income_id;
    live.dispatch(meta(), income).unwrap();
    // The device at the same state, standing in for the vault rolled back to its base.
    let rolled_back = dir.path().join("rolled-back.vault");
    std::fs::write(&rolled_back, live.snapshot_bytes().unwrap()).unwrap();
    let replica = DbWorker::open(&rolled_back, KEY).unwrap();
    // A control at the same state that replays under a DIFFERENT issue time: if the issue time did
    // not matter, the check below would prove nothing.
    let wrong_path = dir.path().join("wrong-time.vault");
    std::fs::write(&wrong_path, std::fs::read(&rolled_back).unwrap()).unwrap();
    let wrong = DbWorker::open(&wrong_path, KEY).unwrap();

    // Two commands that read the issue time: one mints ids (an opening balance), one stamps
    // `deleted_at`.
    let account = liquid_account_cmd(AccountId::new(), Currency::Usd, 12_345);
    let tombstone = WriteCommand::DeleteIncomeSource(income_id);
    for cmd in [account, tombstone] {
        let wrong_cmd = cmd.clone();
        let original = meta_issued_at(FIXED_ISSUED_AT);
        let before_seal = commitment_of(&original, live.node_id);
        live.dispatch(original.clone(), cmd.clone()).unwrap();
        let recorded_write_set = live.last_write_set().unwrap();

        // Rebuild the metadata from what the vault kept, as a rebase must.
        let kept = outbox_issued_at(&live.lock().conn, &original.command_id).unwrap();
        let rebuilt = CommandMeta {
            issued_at: kept,
            ..original.clone()
        };
        replica.dispatch(rebuilt.clone(), cmd).unwrap();

        // Every derived id and every stamp is inside the write-set hash.
        assert_eq!(replica.last_write_set().unwrap(), recorded_write_set);
        let elsewhere = CommandMeta {
            issued_at: IssuedAt::from_unix_micros(FIXED_ISSUED_AT + 86_400_000_000),
            ..original.clone()
        };
        wrong.dispatch(elsewhere, wrong_cmd).unwrap();
        assert_ne!(
            wrong.last_write_set().unwrap().hash,
            recorded_write_set.hash,
            "another issue time mints other ids or stamps"
        );
        assert_eq!(
            commitment_of(&rebuilt, live.node_id),
            before_seal,
            "the replay's metadata commits to what a seal made before the rollback bound"
        );
    }
}
