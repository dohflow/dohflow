//! A replica derives its class-2 state (the transaction display read model, commitments,
//! recurring instances, the Money Inbox, the forecast) from the class-1 rows it installed, without
//! the operation log it does not hold (personal-cfo-1cltt.8).
//!
//! The comparison is between an origin and a replica built from that origin's snapshot. Which
//! columns the comparison leaves out, and why, is the one list in [`EXCLUDED`]: nothing else is
//! left out.

use std::sync::Arc;

use chrono::{DateTime, NaiveDate, Utc};
use core_ledger::TransactionId;
use core_money::{Currency, Money};
use rusqlite::types::Value;
use rusqlite::Connection;
use sync_core::snapshot::{verify, Verified};
use sync_core::status::EngineVersion;
use uuid::Uuid;

use super::{meta, seed_golden_vault, worker};
use crate::logical_export::{export_genesis_plaintext, ExportInput};
use crate::rebase::StepContext;
use crate::sync_bootstrap::{bootstrap_replica, ReplicaSource};
use crate::{
    AccountId, CommandMeta, DbWorker, Frequency, TransactionPageQuery, TransactionSortOrder,
    WriteCommand,
};

/// The class-2 tables compared row for row between an origin and a replica.
const COMPARED: &[&str] = &[
    "transaction_display_rows_read_model",
    "commitments",
    "recurring_event_instances",
    // The part of the inbox derived from class-1 state (the change journal's snooze and dismiss)
    // is compared; the part derived from staged imports is class 3, so it is empty on a fresh
    // replica and on this origin alike (no import is staged here), and is not what is compared.
    "money_inbox_read_model",
    "forecast_input_snapshots",
    "forecast_runs",
    "forecast_rows",
];

/// Every column the comparison leaves out, by table, each with its reason. A column named in no
/// row here is compared.
const EXCLUDED: &[(&str, &str, &str)] = &[
    // Stamped at the rebuild, so they hold the instant each device rebuilt.
    ("commitments", "created_at", "stamped at rebuild"),
    ("commitments", "updated_at", "stamped at rebuild"),
    (
        "recurring_event_instances",
        "created_at",
        "stamped at rebuild",
    ),
    (
        "forecast_input_snapshots",
        "created_at",
        "stamped when the snapshot is captured",
    ),
    (
        "forecast_input_snapshots",
        "ledger_cutoff_at",
        "the instant of the capture (the wall clock when the run was persisted)",
    ),
    // The device's own operation-log position (a watermark in this device's `op_seq` space).
    (
        "transaction_display_rows_read_model",
        "last_projected_op_seq",
        "this device's op_seq watermark",
    ),
    (
        "forecast_input_snapshots",
        "ledger_cutoff_op_seq",
        "this device's op_seq watermark",
    ),
    (
        "forecast_input_snapshots",
        "content_hash",
        "folds this device's op_seq in",
    ),
    (
        "forecast_runs",
        "assumptions_hash",
        "is the input snapshot's content_hash",
    ),
    // The ids of one forecast run and its rows are minted per run: device-local.
    ("forecast_input_snapshots", "id", "minted per capture"),
    ("forecast_runs", "id", "minted per run"),
    (
        "forecast_runs",
        "input_snapshot_id",
        "names a per-capture id",
    ),
    ("forecast_rows", "id", "minted per run"),
    ("forecast_rows", "forecast_run_id", "names a per-run id"),
];

fn engine() -> EngineVersion {
    EngineVersion::new("dohflow-engine-1").unwrap()
}

/// What a table holds with the excluded columns left out, as sorted text rows.
fn dump(conn: &Connection, table: &str) -> Vec<String> {
    let columns: Vec<String> = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .unwrap()
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .map(Result::unwrap)
        .filter(|name| {
            !EXCLUDED
                .iter()
                .any(|(t, c, _)| *t == table && *c == name.as_str())
        })
        .collect();
    assert!(!columns.is_empty(), "{table}");
    let sql = format!("SELECT {} FROM {table}", columns.join(", "));
    let mut stmt = conn.prepare(&sql).unwrap();
    let width = columns.len();
    let mut rows: Vec<String> = stmt
        .query_map([], |r| {
            let values: Vec<Value> = (0..width).map(|i| r.get(i)).collect::<Result<_, _>>()?;
            Ok(format!("{values:?}"))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    rows.sort();
    rows
}

fn dump_all(worker: &DbWorker) -> Vec<(&'static str, Vec<String>)> {
    let conn = worker.read_connection().unwrap();
    COMPARED
        .iter()
        .map(|table| (*table, dump(&conn, table)))
        .collect()
}

fn assert_class_2_equal(left: &DbWorker, right: &DbWorker, what: &str) {
    for ((table, a), (_, b)) in dump_all(left).into_iter().zip(dump_all(right)) {
        assert_eq!(a, b, "{what}: {table}");
    }
}

fn as_of() -> DateTime<Utc> {
    use chrono::TimeZone;
    Utc.with_ymd_and_hms(2026, 6, 20, 12, 0, 0).unwrap()
}

fn day(d: u32) -> DateTime<Utc> {
    NaiveDate::from_ymd_opt(2026, 6, d)
        .unwrap()
        .and_hms_opt(12, 0, 0)
        .unwrap()
        .and_utc()
}

fn record(
    account: AccountId,
    cents: i64,
    occurred_at: DateTime<Utc>,
) -> (CommandMeta, WriteCommand) {
    (
        meta(),
        WriteCommand::RecordTransaction {
            transaction_id: TransactionId::new(),
            account_id: account,
            amount: Money::new(cents, Currency::Usd),
            occurred_at,
        },
    )
}

/// An origin that has history, two transactions that tie on the instant they occurred, a voided one,
/// and the snapshot it would publish.
fn origin() -> (tempfile::TempDir, DbWorker, AccountId, Verified) {
    let (dir, worker) = worker();
    let (checking, _savings) = seed_golden_vault(&worker);
    for (cents, at) in [
        (-4_500, day(2)),
        (-1_250, day(9)),
        (-2_000, day(9)), // ties with the one before on the instant
        (-310, day(9)),   // and with this one
        (150_000, day(12)),
    ] {
        let (m, c) = record(checking, cents, at);
        worker.dispatch(m, c).unwrap();
    }
    let verified = verify(
        &export_genesis_plaintext(&worker.read_connection().unwrap(), &ExportInput::default())
            .unwrap(),
    )
    .unwrap();
    (dir, worker, checking, verified)
}

fn replica_of(origin: &DbWorker, verified: &Verified) -> (tempfile::TempDir, DbWorker) {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("vault.db");
    let dek = Arc::new(vault_crypto::generate_dek().unwrap());
    let (version, own) = (engine(), engine());
    let born = bootstrap_replica(
        &path,
        &dek,
        &ReplicaSource {
            vault_id: *origin.vault_id().unwrap().as_bytes(),
            root: 4,
            snapshot: verified,
            engine_version: &version,
            own_engine_version: &own,
            held_voids: &[],
        },
    )
    .unwrap();
    (dir, born.worker)
}

/// A server envelope applied to a replica: the ordinary apply, recorded in the replica's own
/// operation log with the server position it was applied on, and not queued for push.
fn pull(replica: &DbWorker, base: i64, meta: CommandMeta, command: &WriteCommand) {
    replica
        .dispatch_rebase_step(
            meta,
            command,
            StepContext {
                applied_base_seq: Some(base),
                rebased_from: None,
                record_outbox: false,
            },
        )
        .unwrap();
}

fn all_pages(worker: &DbWorker, sort: TransactionSortOrder) -> String {
    let page = worker
        .transaction_page(&TransactionPageQuery {
            sort,
            limit: 500,
            ..TransactionPageQuery::default()
        })
        .unwrap();
    format!("{:?}", page.rows)
}

fn every_sort_mode() -> [TransactionSortOrder; 4] {
    [
        TransactionSortOrder::NewestFirst,
        TransactionSortOrder::OldestFirst,
        TransactionSortOrder::AmountDesc,
        TransactionSortOrder::AmountAsc,
    ]
}

fn event_id(worker: &DbWorker) -> Uuid {
    worker
        .read_connection()
        .unwrap()
        .query_row("SELECT id FROM recurring_events LIMIT 1", [], |r| r.get(0))
        .unwrap()
}

fn user_posting_count(worker: &DbWorker) -> i64 {
    worker
        .read_connection()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM ledger_postings lp
             JOIN ledger_accounts la ON la.id = lp.ledger_account_id
             JOIN accounts a ON a.ledger_account_id = la.id",
            [],
            |r| r.get(0),
        )
        .unwrap()
}

/// Every origin row has the log row the old join needed, so dropping the join changes no origin
/// row set (AC9): the live list reads return what they returned before.
#[test]
fn dropping_the_operation_log_join_changes_no_origin_row_set() {
    let (_dir, origin, _checking, _verified) = origin();
    let conn = origin.read_connection().unwrap();
    let without_a_log_row: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM ledger_transactions lt
             WHERE NOT EXISTS (SELECT 1 FROM operation_log ol WHERE ol.command_id = lt.operation_id)",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(without_a_log_row, 0);
    let (with_join, without_join): (i64, i64) = (
        conn.query_row(
            "SELECT COUNT(*) FROM ledger_postings lp
             JOIN ledger_transactions lt ON lt.id = lp.transaction_id
             JOIN ledger_accounts la ON la.id = lp.ledger_account_id
             JOIN accounts a ON a.ledger_account_id = la.id
             JOIN operation_log ol ON ol.command_id = lt.operation_id",
            [],
            |r| r.get(0),
        )
        .unwrap(),
        conn.query_row(
            "SELECT COUNT(*) FROM ledger_postings lp
             JOIN ledger_transactions lt ON lt.id = lp.transaction_id
             JOIN ledger_accounts la ON la.id = lp.ledger_account_id
             JOIN accounts a ON a.ledger_account_id = la.id",
            [],
            |r| r.get(0),
        )
        .unwrap(),
    );
    assert_eq!(with_join, without_join);
    assert!(with_join > 0);
}

/// Transactions that tie on the instant they occurred are ordered by the command that created
/// them, newest command first, not by a device's log position (AC10).
#[test]
fn same_instant_transactions_order_by_creating_command() {
    let (_dir, origin, _checking, _verified) = origin();
    let rows = origin.recent_transactions(500).unwrap();
    let on_the_ninth: Vec<i64> = rows
        .iter()
        .filter(|r| r.occurred_at == day(9))
        .map(|r| r.amount.minor_units())
        .collect();
    // Recorded -1_250, then -2_000, then -310: newest command first.
    assert_eq!(on_the_ninth, [-310, -2_000, -1_250]);
}

/// AC3, AC9, AC11, AC12, AC13: a replica born from the origin's genesis holds the same class-2
/// state and shows the same screens, in the same order.
#[test]
fn a_replica_derives_the_class_2_state_its_origin_holds() {
    let (_odir, origin, _checking, verified) = origin();
    let (_rdir, replica) = replica_of(&origin, &verified);

    // The replica holds no operation log, and still has every transaction in its display model.
    let replica_log: i64 = replica.operation_count().unwrap().try_into().unwrap();
    assert_eq!(replica_log, 0);
    let postings = user_posting_count(&replica);
    assert!(postings >= 5);
    assert_eq!(
        i64::try_from(replica.transaction_display_rows().unwrap().len()).unwrap(),
        postings
    );
    assert_eq!(
        replica.transaction_projection_cursor().unwrap(),
        0,
        "the cursor is the replica's own MAX(op_seq): 0 on an empty log"
    );

    // The origin's own derivation, a full rebuild of what it holds, is the reference.
    origin.rebuild_class_2().unwrap();
    for worker in [&origin, &replica] {
        worker.persist_daily_forecast_at(as_of(), 90).unwrap();
    }
    assert_class_2_equal(&origin, &replica, "bootstrap");
    // The comparison is not vacuous: each table the fixture can fill holds rows.
    let replica_dump = dump_all(&replica);
    for table in [
        "transaction_display_rows_read_model",
        "commitments",
        "recurring_event_instances",
        "forecast_rows",
    ] {
        let rows = replica_dump.iter().find(|(t, _)| *t == table).unwrap();
        assert!(!rows.1.is_empty(), "{table} is empty on the replica");
    }

    // The live list: every sort mode, row for row and in order.
    for sort in every_sort_mode() {
        assert_eq!(
            all_pages(&origin, sort),
            all_pages(&replica, sort),
            "{sort:?}"
        );
    }
    assert_eq!(
        format!("{:?}", origin.recent_transactions(500).unwrap()),
        format!("{:?}", replica.recent_transactions(500).unwrap())
    );
}

/// AC7: on a replica, after each step (genesis, a local command, a pulled batch, a local command)
/// the incremental path leaves exactly what a fresh class-1 rebuild gives: no row skipped, none
/// applied twice, the cursor in the replica's own `op_seq` space.
#[test]
fn a_replicas_incremental_projection_equals_a_fresh_class_1_rebuild_after_every_step() {
    let (_odir, origin, checking, verified) = origin();
    let (_rdir, replica) = replica_of(&origin, &verified);

    let check = |step: &str, new_rows: Option<u64>| {
        let projected = replica.project_transaction_display_incremental().unwrap();
        if let Some(expected) = new_rows {
            assert_eq!(projected, expected, "{step}: exactly the new rows");
        }
        let incremental = dump_all(&replica);
        let cursor = replica.transaction_projection_cursor().unwrap();
        let log_head: i64 = replica
            .read_connection()
            .unwrap()
            .query_row(
                "SELECT COALESCE(MAX(op_seq), 0) FROM operation_log",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            cursor, log_head,
            "{step}: the cursor is this device's own op_seq"
        );
        replica.rebuild_class_2().unwrap();
        let rebuilt = dump_all(&replica);
        for ((table, a), (_, b)) in incremental.iter().zip(&rebuilt) {
            if *table == "transaction_display_rows_read_model" {
                assert_eq!(a, b, "{step}: {table}");
            }
        }
        assert_eq!(
            i64::try_from(replica.transaction_display_rows().unwrap().len()).unwrap(),
            user_posting_count(&replica),
            "{step}: one display row per user posting, none missing or doubled"
        );
        assert_eq!(
            replica.transaction_projection_cursor().unwrap(),
            log_head,
            "{step}: a full rebuild leaves the cursor at the device's own head"
        );
    };

    check("genesis", Some(0));
    let (m, c) = record(checking, -777, day(14));
    replica.dispatch(m, c).unwrap();
    check("a local command", Some(1));

    let (m, c) = record(checking, -888, day(15));
    pull(&replica, 4, m, &c);
    let (m, c) = record(checking, 9_999, day(16));
    pull(&replica, 5, m, &c);
    check("a pulled batch", Some(2));

    let (m, c) = record(checking, -42, day(17));
    replica.dispatch(m, c).unwrap();
    check("a local command after the batch", Some(1));
}

/// AC4: the replica and the origin apply the same further commands, including an edit and a void
/// of a transaction that came in the genesis, and their class-2 state still agrees.
#[test]
fn a_replica_and_its_origin_converge_after_further_commands_including_genesis_rows() {
    let (_odir, origin, checking, verified) = origin();
    let (_rdir, replica) = replica_of(&origin, &verified);

    let genesis: Vec<TransactionId> = origin
        .recent_transactions(500)
        .unwrap()
        .into_iter()
        .map(|r| r.transaction_id)
        .collect();
    let bill = event_id(&origin);
    let commands: Vec<WriteCommand> = vec![
        record(checking, -6_100, day(18)).1,
        WriteCommand::UpdateRecurringBill {
            event_id: crate::RecurringEventId::from_uuid(bill),
            name: "Rent (new lease)".to_owned(),
            amount: Money::new(190_000, Currency::Usd),
            bill_type: "rent_mortgage".to_owned(),
            frequency: Frequency::Monthly,
            anchor: NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
            autopay_account_id: Some(checking),
            description: None,
        },
        WriteCommand::VoidTransaction {
            transaction_id: genesis[0],
        },
    ];
    for (i, command) in commands.iter().enumerate() {
        let m = meta();
        origin.dispatch(m.clone(), command.clone()).unwrap();
        pull(&replica, 4 + i64::try_from(i).unwrap(), m, command);
    }

    origin.project_transaction_display_incremental().unwrap();
    replica.project_transaction_display_incremental().unwrap();
    origin.rebuild_class_2().unwrap();
    replica.rebuild_class_2().unwrap();
    for worker in [&origin, &replica] {
        worker.persist_daily_forecast_at(as_of(), 90).unwrap();
    }
    assert_class_2_equal(&origin, &replica, "after further commands");
    for sort in every_sort_mode() {
        assert_eq!(
            all_pages(&origin, sort),
            all_pages(&replica, sort),
            "{sort:?}"
        );
    }
}

/// AC1/AC5: the full display rebuild reads class 1 only. A replica's rows do not depend on the
/// operation log, so they survive a rebuild on a log with no rows at all.
#[test]
fn the_full_display_rebuild_keeps_a_row_for_every_transaction_whatever_the_log_holds() {
    let (_odir, origin, _checking, verified) = origin();
    let (_rdir, replica) = replica_of(&origin, &verified);
    let before = replica.transaction_display_rows().unwrap();
    assert!(!before.is_empty());
    assert!(
        before.iter().all(|r| r.last_projected_op_seq == 0),
        "the stamp is this device's own op_seq, 0 for a row with no log row"
    );
    assert_eq!(
        replica.rebuild_transaction_display().unwrap() as usize,
        before.len()
    );
    assert_eq!(replica.transaction_display_rows().unwrap(), before);
}
