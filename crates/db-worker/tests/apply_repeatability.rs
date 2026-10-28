//! `apply` reads no clock (personal-cfo-32fmp slice 1; ADR 0013 Addendum A): the same command,
//! with the same metadata, applied to two identical copies of one vault, writes the same rows and
//! the same write-set hash, and every instant it stamps is the command's `issued_at`.
//!
//! Commands whose ids are minted inside `apply` are not in this set: their ids differ until the
//! derived-id pass (the next slice of 32fmp) lands, and that pass extends this test to them.

mod common;

use chrono::{DateTime, NaiveDate, Utc};
use common::{bill_cmd, KEY};
use core_ledger::{
    AccountId, BillContractId, CategoryId, IncomeSourceId, RecurringEventId, TagId, TransactionId,
};
use core_money::{Currency, Money};
use db_worker::{ActorType, CommandMeta, DbWorker, IssuedAt, WriteCommand};
use pay_schedule::Frequency;
use std::path::{Path, PathBuf};
use uuid::Uuid;

fn u(n: u128) -> Uuid {
    Uuid::from_u128(0x0198_c2c1_5f0a_7c3d_8a4e_9b1d_0000_0000 + n)
}
fn account(n: u128) -> AccountId {
    AccountId::from_uuid(u(n))
}
fn category(n: u128) -> CategoryId {
    CategoryId::from_uuid(u(n))
}
fn tag(n: u128) -> TagId {
    TagId::from_uuid(u(n))
}
fn txn(n: u128) -> TransactionId {
    TransactionId::from_uuid(u(n))
}
fn event(n: u128) -> RecurringEventId {
    RecurringEventId::from_uuid(u(n))
}
fn usd(minor: i64) -> Money {
    Money::new(minor, Currency::Usd)
}

/// A moment far from "now": a stamp equal to it can only have come from `issued_at`.
const ISSUED_MICROS: i64 = 1_000_000_000_000_000; // 2001-09-09T01:46:40Z

fn meta(n: u128) -> CommandMeta {
    CommandMeta {
        command_id: u(1_000 + n),
        correlation_id: u(2_000 + n),
        causation_id: None,
        actor_type: ActorType::User,
        actor_id: "tester".to_owned(),
        idempotency_key: format!("00000000-0000-4000-8000-{:012x}", 3_000 + n),
        issued_at: IssuedAt::from_unix_micros(ISSUED_MICROS),
    }
}

fn open(path: &Path) -> DbWorker {
    DbWorker::open(path, KEY).unwrap()
}

/// The commands under test. Each is applied on top of the base built by `base`.
fn commands() -> Vec<(&'static str, WriteCommand)> {
    vec![
        (
            "mark reviewed",
            WriteCommand::MarkReviewed {
                transaction_id: txn(10),
                reviewed: true,
            },
        ),
        (
            "set a note",
            WriteCommand::SetNote {
                transaction_id: txn(10),
                note: Some("coffee".into()),
            },
        ),
        (
            "create a tag",
            WriteCommand::CreateTag {
                id: tag(31),
                name: "trip".into(),
                color: None,
            },
        ),
        (
            "create a category",
            WriteCommand::CreateCategory {
                id: category(32),
                parent_id: None,
                name: "Fun".into(),
                category_type: "expense".into(),
                color: None,
                icon: None,
            },
        ),
        (
            "archive a category",
            WriteCommand::ArchiveCategory(category(32)),
        ),
        (
            "recategorize a transaction",
            WriteCommand::RecategorizeTransaction {
                transaction_id: txn(10),
                category_id: Some(category(32)),
            },
        ),
        (
            "create an income source",
            WriteCommand::CreateIncomeSource {
                id: IncomeSourceId::from_uuid(u(33)),
                name: "Paycheck".into(),
                net_amount: usd(300_000),
                frequency: Frequency::Biweekly,
                anchor: NaiveDate::from_ymd_opt(2026, 6, 5).unwrap(),
                deposit_account_id: Some(account(1)),
            },
        ),
        (
            "archive an income source",
            WriteCommand::ArchiveIncomeSource(IncomeSourceId::from_uuid(u(33))),
        ),
        (
            "create a recurring bill",
            match bill_cmd(Some(account(1))) {
                WriteCommand::CreateRecurringBill {
                    name,
                    amount,
                    bill_type,
                    frequency,
                    anchor,
                    autopay_account_id,
                    description,
                    source_merchant_key,
                    category_id,
                    tag_ids,
                    ..
                } => WriteCommand::CreateRecurringBill {
                    event_id: event(34),
                    contract_id: BillContractId::from_uuid(u(35)),
                    name,
                    amount,
                    bill_type,
                    frequency,
                    anchor,
                    autopay_account_id,
                    description,
                    source_merchant_key,
                    category_id,
                    tag_ids,
                },
                _ => unreachable!(),
            },
        ),
        (
            "delete the bill",
            WriteCommand::DeleteRecurringBill {
                event_id: event(34),
            },
        ),
        (
            "reinstate the bill",
            WriteCommand::ReinstateRecurringBill {
                event_id: event(34),
            },
        ),
        (
            "set the household timezone",
            WriteCommand::SetHouseholdTimezone {
                timezone: "America/Chicago".into(),
            },
        ),
    ]
}

/// Commands whose rows get ids minted inside `apply` today (`Uuid::now_v7()`), so two vaults
/// cannot agree on the write set until the derived-id pass. They still stamp `issued_at`.
fn minting_commands() -> Vec<(&'static str, WriteCommand)> {
    vec![
        (
            "snooze an inbox item",
            WriteCommand::SnoozeInboxItem {
                item_id: u(36),
                until: NaiveDate::from_ymd_opt(2026, 12, 25).unwrap(),
            },
        ),
        (
            "dismiss an inbox item",
            WriteCommand::DismissInboxItem {
                item_id: u(37),
                reason: "not mine".into(),
            },
        ),
    ]
}

/// A vault with an account and one recorded transaction, as a file two workers can start from.
fn base(dir: &Path) -> PathBuf {
    let path = dir.join("base.vault");
    let worker = open(&path);
    let mut setup = common::meta();
    setup.issued_at = IssuedAt::from_unix_micros(ISSUED_MICROS - 1_000_000);
    worker
        .dispatch(
            setup.clone(),
            common::liquid_account_cmd(account(1), core_money::Currency::Usd, 0),
        )
        .unwrap();
    let mut record = common::meta();
    record.issued_at = setup.issued_at;
    worker
        .dispatch(
            record,
            WriteCommand::RecordTransaction {
                transaction_id: txn(10),
                account_id: account(1),
                amount: usd(-4_250),
                occurred_at: DateTime::<Utc>::from_timestamp_micros(ISSUED_MICROS).unwrap(),
            },
        )
        .unwrap();
    worker.checkpoint_truncate().unwrap();
    drop(worker);
    path
}

#[test]
fn the_same_command_on_identical_vaults_writes_the_same_rows_and_hash() {
    let dir = tempfile::tempdir().unwrap();
    let base = base(dir.path());
    let (a_path, b_path) = (dir.path().join("a.vault"), dir.path().join("b.vault"));
    std::fs::copy(&base, &a_path).unwrap();
    std::fs::copy(&base, &b_path).unwrap();
    let (a, b) = (open(&a_path), open(&b_path));
    for (n, (label, command)) in commands().into_iter().enumerate() {
        // The two vaults hold the same state before and after every command: the ids come from
        // the payloads, and the instant from the metadata, so nothing else can differ.
        let m = meta(n as u128);
        a.dispatch(m.clone(), command_clone(&command))
            .unwrap_or_else(|e| panic!("{label}: {e}"));
        b.dispatch(m, command)
            .unwrap_or_else(|e| panic!("{label}: {e}"));
        let (wa, wb) = (a.last_write_set().unwrap(), b.last_write_set().unwrap());
        assert_eq!(wa.hash, wb.hash, "{label}: write-set hash");
        assert_eq!(wa.rows.len(), wb.rows.len(), "{label}: rows written");
    }
}

/// `WriteCommand` is not `Clone`; the payload codec is the one-to-one copy.
fn command_clone(command: &WriteCommand) -> WriteCommand {
    let prepared = db_worker::payload::prepare_payload(command).unwrap();
    db_worker::payload::decode_payload(db_worker::payload::PAYLOAD_SCHEMA_VERSION, &prepared.bytes)
        .unwrap()
}

/// Every `(table, column, rowid, value)` whose text value looks like an RFC 3339 instant.
fn instants(
    conn: &rusqlite::Connection,
) -> std::collections::BTreeSet<(String, String, i64, String)> {
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let mut found = std::collections::BTreeSet::new();
    for table in tables {
        let columns: Vec<String> = conn
            .prepare(&format!(
                "SELECT name FROM pragma_table_info('{table}') WHERE type LIKE '%TEXT%'"
            ))
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        for column in columns {
            let sql = format!(
                "SELECT rowid, \"{column}\" FROM \"{table}\" WHERE \"{column}\" LIKE '____-__-__T__:__:__%'"
            );
            // A WITHOUT ROWID table has no rowid; none of the instants live in one.
            let Ok(mut stmt) = conn.prepare(&sql) else {
                continue;
            };
            let rows = stmt
                .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
                .unwrap();
            for row in rows {
                let (rowid, value) = row.unwrap();
                found.insert((table.clone(), column.clone(), rowid, value));
            }
        }
    }
    found
}

#[test]
fn every_instant_apply_stamps_is_the_commands_issued_at() {
    let dir = tempfile::tempdir().unwrap();
    let base = base(dir.path());
    let path = dir.path().join("run.vault");
    std::fs::copy(&base, &path).unwrap();
    let worker = open(&path);
    let issued = DateTime::<Utc>::from_timestamp_micros(ISSUED_MICROS)
        .unwrap()
        .to_rfc3339();
    let before = instants(&worker.read_connection().unwrap());
    for (n, (label, command)) in commands().into_iter().chain(minting_commands()).enumerate() {
        worker
            .dispatch(meta(n as u128), command)
            .unwrap_or_else(|e| panic!("{label}: {e}"));
    }
    let after = instants(&worker.read_connection().unwrap());
    let now = Utc::now();
    let mut stamped = 0;
    for (table, column, rowid, value) in after.difference(&before) {
        // The device's own recording of the commands is not apply's output.
        if matches!(
            table.as_str(),
            "operation_log"
                | "command_idempotency_keys"
                | "projection_cursors"
                | "read_model_checksums"
        ) {
            continue;
        }
        if *value == issued {
            stamped += 1;
        } else if let Ok(at) = DateTime::parse_from_rfc3339(value) {
            let age = now
                .signed_duration_since(at.with_timezone(&Utc))
                .num_hours()
                .abs();
            assert!(
                age > 1,
                "{table}.{column} (row {rowid}) was stamped {value}, within an hour of the wall \
                 clock: apply must stamp the command's issued_at"
            );
        }
    }
    assert!(
        stamped >= 10,
        "the commands stamped issued_at in many places ({stamped})"
    );
}

#[test]
fn an_issued_at_outside_the_calendar_is_refused_before_anything_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let worker = open(&dir.path().join("v.vault"));
    let mut m = meta(0);
    m.issued_at = IssuedAt::from_unix_micros(i64::MAX);
    let err = worker
        .dispatch(
            m,
            WriteCommand::SetHouseholdTimezone {
                timezone: "America/Chicago".into(),
            },
        )
        .unwrap_err();
    assert!(
        matches!(err, db_worker::DbError::InvalidCommand(_)),
        "{err:?}"
    );
}
