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

use super::{liquid_subtype_cmd, meta, seed_golden_vault, worker};
use crate::logical_export::{export_genesis_plaintext, ExportInput};
use crate::rebase::StepContext;
use crate::sync_bootstrap::{bootstrap_replica, ReplicaSource};
use crate::{
    AccountId, AccountSubtype, CommandMeta, DbWorker, Frequency, TransactionPageQuery,
    TransactionSortOrder, WriteCommand,
};

/// The class-2 tables compared row for row between an origin and a replica.
const COMPARED: &[&str] = &[
    "transaction_display_rows_read_model",
    "commitments",
    "recurring_event_instances",
    // The stored inbox table is the staging-derived part (class 3 input): the change journal's
    // snooze and dismiss only touch staged items, so with nothing staged this compares nothing and
    // the table is in UNTESTED. The class-1 part is derived on read and is compared through
    // `money_inbox_list()` (see `assert_the_inbox_lists_are_equal`).
    "money_inbox_read_model",
    "read_model_checksums",
    "forecast_input_snapshots",
    "forecast_runs",
    "forecast_rows",
    "forecast_actuals",
    "forecast_backtest_results",
    "forecast_dependency_edges",
    "forecast_diffs",
    "forecast_quality_scores",
    "risk_flags",
    "forecast_dirty_ranges",
    "model_registry",
];

/// Class-2 tables the comparison names and does not compare, each with its reason.
const NAMED_EXCLUDED_TABLES: &[(&str, &str)] = &[(
    "recurring_suggestion_suppressions",
    "AC12: class 2 in the manifest but written by a user command (DismissRecurringSuggestion), so it \
     cannot be derived from class-1 rows; its classification is a separate decision \
     (personal-cfo-5ymg8.10) and this change does not touch the table",
)];

/// Tables the fixture fills through their write seam on both devices, because no code path of the
/// forecast writes them: `record_dependency_edges` is called only by tests (the forecast engine
/// does not attribute its rows yet), and nothing writes `risk_flags` at all. The rows are the same
/// on both devices by construction, so this exercises the comparison (that a row attributed to the
/// wrong run or row fails, through [`REFERENCES`]), not a derivation by the bootstrap.
const SEEDED_THROUGH_A_SEAM: &[&str] = &["forecast_dependency_edges", "risk_flags"];

/// The compared tables the fixture leaves empty on both devices. They are NOT counted as equal: they
/// are untested here, because nothing the bootstrap or a forecast run does writes them (a caller of
/// the forecast engine does, or no writer exists yet).
const UNTESTED: &[&str] = &[
    "money_inbox_read_model",
    "forecast_diffs",
    "forecast_quality_scores",
    "forecast_dirty_ranges",
];

/// Every column the comparison leaves out, by table, each with its reason. A column named in no
/// row here is compared, or, where [`REFERENCES`] names it, compared through the natural key of the
/// row it points at.
const EXCLUDED: &[(&str, &str, &str)] = &[
    // Stamped at the rebuild or the capture, so they hold the instant each device ran.
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
        "stamped at capture",
    ),
    ("model_registry", "created_at", "stamped at registration"),
    (
        "forecast_dependency_edges",
        "created_at",
        "stamped when the edge is recorded",
    ),
    (
        "forecast_actuals",
        "created_at",
        "class 2, rebuilt locally and never shipped: stamped with the day the actualization ran, so \
         two devices that run either side of midnight UTC differ",
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
    (
        "read_model_checksums",
        "computed_at",
        "stamped when the checksum is computed",
    ),
    (
        "read_model_checksums",
        "current_checksum",
        "the display checksum folds last_projected_op_seq (this device's op_seq space); each device's stored checksum is instead asserted equal to a fresh recomputation on that device (`assert_the_checksums_are_fresh`)",
    ),
    // Minted per capture, run and row with `Uuid::now_v7()` (forecast_persist.rs), or derived from
    // such ids: device-local. Every row is compared through its other columns, and every column that
    // points at one of these rows is compared through that row's natural key ([`REFERENCES`]).
    ("forecast_input_snapshots", "id", "minted per capture"),
    ("forecast_runs", "id", "minted per run"),
    ("forecast_rows", "id", "minted per run"),
    (
        "forecast_actuals",
        "id",
        "derived from the per-run run id and row id (actual_id)",
    ),
];

/// Columns that name a row of another compared table whose own id is excluded. The value compared
/// is the referenced row's natural key (its other columns, with its own references resolved the same
/// way), so a row attributed to the wrong run or row still fails, and no column is dropped.
const REFERENCES: &[(&str, &str, &str)] = &[
    (
        "forecast_runs",
        "input_snapshot_id",
        "forecast_input_snapshots",
    ),
    ("forecast_rows", "forecast_run_id", "forecast_runs"),
    ("forecast_actuals", "forecast_run_id", "forecast_runs"),
    ("forecast_actuals", "forecast_row_id", "forecast_rows"),
    (
        "forecast_dependency_edges",
        "forecast_run_id",
        "forecast_runs",
    ),
    ("forecast_dependency_edges", "to_row_id", "forecast_rows"),
    ("forecast_diffs", "forecast_run_id", "forecast_runs"),
    ("forecast_diffs", "prior_run_id", "forecast_runs"),
    (
        "forecast_quality_scores",
        "forecast_run_id",
        "forecast_runs",
    ),
    ("risk_flags", "forecast_run_id", "forecast_runs"),
];

fn engine() -> EngineVersion {
    EngineVersion::new("dohflow-engine-1").unwrap()
}

fn is_excluded(table: &str, column: &str) -> bool {
    EXCLUDED.iter().any(|(t, c, _)| *t == table && *c == column)
}

fn columns_of(conn: &Connection, table: &str) -> Vec<String> {
    conn.prepare(&format!("PRAGMA table_info({table})"))
        .unwrap()
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// One row as text: every column that is not excluded, a column in [`REFERENCES`] shown as the
/// natural key of the row it points at.
fn row_text(conn: &Connection, table: &str, columns: &[String], row: &rusqlite::Row<'_>) -> String {
    let mut parts = Vec::new();
    for (i, column) in columns.iter().enumerate() {
        if is_excluded(table, column) {
            continue;
        }
        let value: Value = row.get(i).unwrap();
        let reference = REFERENCES
            .iter()
            .find(|(t, c, _)| *t == table && c == column);
        match (reference, &value) {
            (Some((_, _, target)), v) if *v != Value::Null => {
                parts.push(format!("{column}=>{}", natural_key(conn, target, v)));
            }
            _ => parts.push(format!("{column}={value:?}")),
        }
    }
    parts.join("|")
}

/// The natural key of the row of `table` whose `id` is `id`: its non-excluded columns.
fn natural_key(conn: &Connection, table: &str, id: &Value) -> String {
    let columns = columns_of(conn, table);
    let sql = format!("SELECT {} FROM {table} WHERE id = ?1", columns.join(", "));
    let mut stmt = conn.prepare(&sql).unwrap();
    let mut rows = stmt.query([id]).unwrap();
    let row = rows.next().unwrap().expect("a referenced row exists");
    format!("{table}[{}]", row_text(conn, table, &columns, row))
}

/// What a table holds, as sorted text rows (see [`row_text`]).
fn dump(conn: &Connection, table: &str) -> Vec<String> {
    let columns = columns_of(conn, table);
    let sql = format!("SELECT {} FROM {table}", columns.join(", "));
    let mut stmt = conn.prepare(&sql).unwrap();
    let mut rows: Vec<String> = stmt
        .query_map([], |r| Ok(row_text(conn, table, &columns, r)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    rows.sort();
    // A table whose own id is excluded is matched through its other columns, so those must tell
    // its rows apart: each row then has exactly one counterpart, and excluding the id drops none.
    if is_excluded(table, "id") {
        let mut distinct = rows.clone();
        distinct.dedup();
        assert_eq!(distinct.len(), rows.len(), "{table}: rows are not distinct");
    }
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

/// The tables `rebuild_class_2` derives, which a bootstrap must leave filled on its own: compared
/// BEFORE any forecast or other read that would derive them again.
const THE_REBUILT_TABLES: &[&str] = &[
    "transaction_display_rows_read_model",
    "commitments",
    "recurring_event_instances",
];

fn assert_the_rebuilt_tables_equal(origin: &DbWorker, replica: &DbWorker, what: &str) {
    let (o, r) = (
        origin.read_connection().unwrap(),
        replica.read_connection().unwrap(),
    );
    for table in THE_REBUILT_TABLES {
        let (a, b) = (dump(&o, table), dump(&r, table));
        assert!(!b.is_empty(), "{what}: {table} is empty on the replica");
        assert_eq!(a, b, "{what}: {table}");
    }
}

/// The Money Inbox's class-1 part is derived when the list is read (stale balances, unreviewed and
/// low-confidence transactions). It is what a screen shows, so it is compared, and not vacuous.
fn assert_the_inbox_lists_are_equal(origin: &DbWorker, replica: &DbWorker, what: &str) {
    let (a, b) = (
        format!("{:?}", origin.money_inbox_list().unwrap()),
        format!("{:?}", replica.money_inbox_list().unwrap()),
    );
    assert_eq!(a, b, "{what}: the Money Inbox list");
    assert_ne!(a, "[]", "{what}: the Money Inbox list is not vacuous");
}

/// Each device's stored checksum equals a fresh recomputation on that device. (The checksums are
/// not compared across devices: the display checksum folds this device's op_seq stamps in.)
fn assert_the_checksums_are_fresh(worker: &DbWorker, what: &str) {
    let conn = worker.read_connection().unwrap();
    let stored: Vec<(String, i64)> = conn
        .prepare("SELECT read_model_name, current_checksum FROM read_model_checksums")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(!stored.is_empty(), "{what}: no checksum is stored");
    for (name, value) in stored {
        let fresh = match name.as_str() {
            "transaction_display" => crate::projection::checksum(&conn).unwrap(),
            "commitments" => crate::commitments::checksum(&conn).unwrap(),
            "money_inbox" => crate::money_inbox::checksum(&conn).unwrap(),
            other => panic!("{what}: an unknown read model {other}"),
        };
        assert_eq!(
            value, fresh as i64,
            "{what}: the stored {name} checksum is stale"
        );
    }
}

/// Every class-2 table the manifest lists is either compared or named, and nothing else is: a new
/// class-2 table cannot slip past the comparison.
#[test]
fn every_class_2_table_is_compared_or_named() {
    let manifest = include_str!("../../table_classes.toml");
    let mut class_2: Vec<&str> = Vec::new();
    for block in manifest.split("[[table]]").skip(1) {
        let field = |key: &str| {
            block.lines().find_map(|l| {
                l.strip_prefix(key)?
                    .strip_prefix(" = \"")?
                    .strip_suffix('"')
            })
        };
        if field("class") == Some("2") {
            class_2.push(field("table").expect("a table name"));
        }
    }
    let mut accounted: Vec<&str> = COMPARED
        .iter()
        .copied()
        .chain(NAMED_EXCLUDED_TABLES.iter().map(|(t, _)| *t))
        .collect();
    class_2.sort_unstable();
    accounted.sort_unstable();
    assert_eq!(class_2, accounted, "class-2 tables vs the comparison");
}

/// Persist a forecast run at the fixed as-of date, then score it against the instances and backtest
/// it. Exactly one run exists afterwards, and every forecast row belongs to it, so the rows are
/// compared within their run.
fn run_the_forecast(worker: &DbWorker) {
    worker.persist_daily_forecast_at(as_of(), 90).unwrap();
    worker.actualize_forecasts().unwrap();
    worker.backtest_forecasts().unwrap();
    let conn = worker.read_connection().unwrap();
    let runs: i64 = conn
        .query_row("SELECT COUNT(*) FROM forecast_runs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(runs, 1, "one run per device");
    let (rows, in_the_run): (i64, i64) = conn
        .query_row(
            "SELECT COUNT(*), COUNT(*) FILTER (WHERE forecast_run_id = (SELECT id FROM forecast_runs))
               FROM forecast_rows",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(rows > 0);
    assert_eq!(rows, in_the_run, "every forecast row belongs to the run");
    drop(conn);
    seed_the_write_seams(worker);
}

/// Fill the two tables whose only writers are test seams (see [`SEEDED_THROUGH_A_SEAM`]): one edge
/// from the bill to every forecast row that bill produced, and a risk flag on the run. The ids are
/// fixed, so they are the same on every device and are compared.
fn seed_the_write_seams(worker: &DbWorker) {
    let (run, bill, rows): (Uuid, Uuid, Vec<Uuid>) = {
        let conn = worker.read_connection().unwrap();
        let run = conn
            .query_row("SELECT id FROM forecast_runs", [], |r| r.get(0))
            .unwrap();
        let bill = conn
            .query_row("SELECT id FROM recurring_events LIMIT 1", [], |r| r.get(0))
            .unwrap();
        let rows = conn
            .prepare("SELECT id FROM forecast_rows WHERE source_id = ?1 ORDER BY date")
            .unwrap()
            .query_map([bill], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        (run, bill, rows)
    };
    assert!(!rows.is_empty(), "the bill produced forecast rows");
    let edges: Vec<crate::DependencyEdge> = rows
        .iter()
        .enumerate()
        .map(|(i, row)| crate::DependencyEdge {
            id: Uuid::from_u128(0x1000 + i as u128),
            forecast_run_id: run,
            from_event_id: bill,
            to_row_id: *row,
            edge_type: crate::EdgeType::Affects,
        })
        .collect();
    worker.record_dependency_edges(&edges).unwrap();
    worker
        .lock()
        .conn
        .execute(
            "INSERT INTO risk_flags
                (id, forecast_run_id, flag_type, severity, projected_impact_minor, evidence_json,
                 readiness_required_bps, created_at)
             VALUES (?1, ?2, 'low_balance', 'warning', -150000, '{}', 0, '2026-06-20T12:00:00+00:00')",
            rusqlite::params![Uuid::from_u128(0x2000), run],
        )
        .unwrap();
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
    let (checking, savings) = seed_golden_vault(&worker);
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
    // Two transfers at the same instant as the transactions on the ninth: each is one transaction
    // with two user postings, so its two legs tie on every key but the account.
    for (i, cents) in [7_000, 12_500, 3_300, 41_000, 900, 6_100]
        .into_iter()
        .enumerate()
    {
        // Half run the other way, so the order the postings were written in is not the account order.
        let (source_account_id, dest_account_id) = if i % 2 == 0 {
            (checking, savings)
        } else {
            (savings, checking)
        };
        worker
            .dispatch(
                meta(),
                WriteCommand::Transfer {
                    source_account_id,
                    dest_account_id,
                    amount: Money::new(cents, Currency::Usd),
                    occurred_at: day(9),
                },
            )
            .unwrap();
    }
    // Two accounts created in the opposite order to their ids (the higher id first), so the order
    // their rows were written in is not their id order, and transfers between them.
    let (high, low) = (
        AccountId::from_uuid(Uuid::from_u128(u128::MAX - 3)),
        AccountId::from_uuid(Uuid::from_u128(3)),
    );
    for (id, name) in [(high, "High id"), (low, "Low id")] {
        worker
            .dispatch(
                meta(),
                liquid_subtype_cmd(id, name, Some(AccountSubtype::Checking), 0),
            )
            .unwrap();
    }
    for (source, dest) in [(high, low), (low, high)] {
        worker
            .dispatch(
                meta(),
                WriteCommand::Transfer {
                    source_account_id: source,
                    dest_account_id: dest,
                    amount: Money::new(2_500, Currency::Usd),
                    occurred_at: day(11),
                },
            )
            .unwrap();
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
        .filter(|r| r.occurred_at == day(9) && r.counter_account_id.is_none())
        .map(|r| r.amount.minor_units())
        .collect();
    // Recorded -1_250, then -2_000, then -310: newest command first.
    assert_eq!(on_the_ninth, [-310, -2_000, -1_250]);
}

/// A transaction whose id sorts against its command's order is still ordered by the command: the
/// id only breaks a tie the command leaves (AC10).
#[test]
fn a_transaction_id_that_sorts_against_command_order_does_not_decide_the_order() {
    let (_dir, origin, checking, _verified) = origin();
    // The first command makes the transaction with the HIGHER id; the second the lower.
    for id in [u128::MAX - 7, 7] {
        origin
            .dispatch(
                meta(),
                WriteCommand::RecordTransaction {
                    transaction_id: TransactionId::from_uuid(Uuid::from_u128(id)),
                    account_id: checking,
                    amount: Money::new(-1, Currency::Usd),
                    occurred_at: day(10),
                },
            )
            .unwrap();
    }
    let ids = |sort| -> Vec<u128> {
        origin
            .transaction_page(&TransactionPageQuery {
                sort,
                limit: 500,
                ..TransactionPageQuery::default()
            })
            .unwrap()
            .rows
            .iter()
            .filter(|r| r.occurred_at == day(10))
            .map(|r| r.transaction_id.as_uuid().as_u128())
            .collect()
    };
    assert_eq!(ids(TransactionSortOrder::NewestFirst), [7, u128::MAX - 7]);
    assert_eq!(ids(TransactionSortOrder::OldestFirst), [u128::MAX - 7, 7]);
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

    // The origin's own derivation, a full rebuild of what it holds, is the reference. The tables the
    // bootstrap derives are compared first, before any forecast or other read derives them again.
    origin.rebuild_class_2().unwrap();
    assert_the_rebuilt_tables_equal(&origin, &replica, "straight after the bootstrap");
    assert_the_inbox_lists_are_equal(&origin, &replica, "straight after the bootstrap");
    assert_the_checksums_are_fresh(&replica, "straight after the bootstrap");
    for worker in [&origin, &replica] {
        run_the_forecast(worker);
    }
    assert_class_2_equal(&origin, &replica, "bootstrap");
    assert_the_inbox_lists_are_equal(&origin, &replica, "after the forecast");
    for worker in [&origin, &replica] {
        assert_the_checksums_are_fresh(worker, "after the forecast");
    }
    // The comparison is not vacuous: a compared table holds rows on both devices, or it is named
    // in UNTESTED (and so not counted as equal).
    let mut empty_on_both = Vec::new();
    for ((table, o), (_, r)) in dump_all(&origin).into_iter().zip(dump_all(&replica)) {
        if o.is_empty() && r.is_empty() {
            empty_on_both.push(table);
        } else {
            assert!(!o.is_empty() && !r.is_empty(), "{table}: {o:?} / {r:?}");
        }
    }
    assert_eq!(
        empty_on_both, UNTESTED,
        "the tables the fixture leaves empty"
    );
    for table in SEEDED_THROUGH_A_SEAM {
        assert!(!empty_on_both.contains(table), "{table} is seeded");
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

    // Before any full rebuild: the display model the incremental path left, and the commitments
    // each command's own apply refreshed.
    origin.project_transaction_display_incremental().unwrap();
    replica.project_transaction_display_incremental().unwrap();
    {
        let (o, r) = (
            origin.read_connection().unwrap(),
            replica.read_connection().unwrap(),
        );
        for table in ["transaction_display_rows_read_model", "commitments"] {
            assert_eq!(
                dump(&o, table),
                dump(&r, table),
                "before a rebuild: {table}"
            );
        }
    }
    assert_the_inbox_lists_are_equal(&origin, &replica, "before a rebuild");
    origin.rebuild_class_2().unwrap();
    replica.rebuild_class_2().unwrap();
    for worker in [&origin, &replica] {
        run_the_forecast(worker);
    }
    assert_class_2_equal(&origin, &replica, "after further commands");
    assert_the_inbox_lists_are_equal(&origin, &replica, "after further commands");
    for worker in [&origin, &replica] {
        assert_the_checksums_are_fresh(worker, "after further commands");
    }
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

/// Every page of `sort`, `size` rows at a time, as (transaction, account) pairs.
fn walk(worker: &DbWorker, sort: TransactionSortOrder, size: u32) -> Vec<(Uuid, Uuid)> {
    let mut out = Vec::new();
    let mut offset = 0;
    loop {
        let page = worker
            .transaction_page(&TransactionPageQuery {
                sort,
                limit: size,
                offset,
                ..TransactionPageQuery::default()
            })
            .unwrap();
        if page.rows.is_empty() {
            return out;
        }
        out.extend(
            page.rows
                .iter()
                .map(|r| (r.transaction_id.as_uuid(), r.account_id.as_uuid())),
        );
        offset += size;
    }
}

/// A transfer is one transaction with two user postings, so it is two rows that tie on the sort
/// key, the creating command and the transaction. The account id separates them, so paging a list
/// that holds transfers shows each leg exactly once, in the same order on an origin and a replica,
/// whatever the page size.
#[test]
fn the_two_legs_of_a_transfer_page_once_each_and_in_the_same_order_on_both_devices() {
    let (_odir, origin, _checking, verified) = origin();
    let (_rdir, replica) = replica_of(&origin, &verified);
    for sort in every_sort_mode() {
        let whole = walk(&origin, sort, 500);
        let transfer_legs = whole
            .iter()
            .filter(|(txn, _)| whole.iter().filter(|(t, _)| t == txn).count() == 2)
            .count();
        assert_eq!(transfer_legs, 16, "{sort:?}: each transfer has two legs");
        let mut unique = whole.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(
            unique.len(),
            whole.len(),
            "{sort:?}: (transaction, account) is unique"
        );
        // The legs of one transfer tie on everything but the account, so the account id orders
        // them: descending in the newest-first list, ascending in the oldest-first one.
        let ascending = match sort {
            TransactionSortOrder::OldestFirst => Some(true),
            TransactionSortOrder::NewestFirst => Some(false),
            _ => None,
        };
        if let Some(ascending) = ascending {
            for pair in whole.windows(2).filter(|w| w[0].0 == w[1].0) {
                assert_eq!(pair[0].1 < pair[1].1, ascending, "{sort:?}: leg order");
            }
        }
        for size in [1, 2, 3] {
            let paged = walk(&origin, sort, size);
            assert_eq!(paged, whole, "{sort:?}: pages of {size} on the origin");
            assert_eq!(
                walk(&replica, sort, size),
                whole,
                "{sort:?}: pages of {size} on the replica"
            );
        }
    }
}
