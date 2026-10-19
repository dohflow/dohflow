//! The operation log records what a replica needs to replay a command (personal-cfo-t2s4b,
//! part 4b; ADR 0074 decision 3): the versioned payload, the payload schema version, the
//! write-set hash of the rows the command changed, and the apply-semantics generation.
//!
//! Pre-v2 rows (written before migration 56) keep NULL in all four columns and are audit
//! history; every row this build writes carries all four, and the table's CHECK forbids a
//! partial set.

mod common;

use chrono::NaiveDate;
use common::*;
use core_ledger::{IncomeSourceId, TransactionId};
use core_money::{Currency, Money};
use db_worker::payload::{decode_payload, encode_payload, PAYLOAD_SCHEMA_VERSION};
use db_worker::*;
use pay_schedule::Frequency;
use rusqlite::Connection;
use sync_core::write_set::write_set_hash;

struct Row {
    operation_type: String,
    payload: Vec<u8>,
    version: i64,
    hash: Vec<u8>,
    engine: String,
}

fn last_row(conn: &Connection) -> Row {
    conn.query_row(
        "SELECT operation_type, payload, payload_schema_version, write_set_hash, engine_version
         FROM operation_log ORDER BY op_seq DESC LIMIT 1",
        [],
        |r| {
            Ok(Row {
                operation_type: r.get(0)?,
                payload: r.get(1)?,
                version: r.get(2)?,
                hash: r.get(3)?,
                engine: r.get(4)?,
            })
        },
    )
    .unwrap()
}

fn empty_set_hash() -> Vec<u8> {
    write_set_hash(&mut []).unwrap().to_vec()
}

/// A varied run of real commands over a live vault.
fn commands() -> Vec<WriteCommand> {
    let account = sample_account();
    let id = account.id();
    let txn = TransactionId::new();
    let (income_id, WriteCommand::CreateIncomeSource { .. }) =
        (IncomeSourceId::new(), income_cmd(None))
    else {
        unreachable!()
    };
    vec![
        WriteCommand::CreateAccount {
            account: Box::new(account),
            opening_balance: Some(Money::new(250_000, Currency::Usd)),
        },
        WriteCommand::RecordTransaction {
            transaction_id: txn,
            account_id: id,
            amount: Money::new(-4_250, Currency::Usd),
            occurred_at: chrono::Utc::now(),
        },
        WriteCommand::SetNote {
            transaction_id: txn,
            note: Some("coffee with a \"quoted\" word and unicode \u{e9}\u{1f512}".to_owned()),
        },
        WriteCommand::CreateIncomeSource {
            id: income_id,
            name: "Pay".to_owned(),
            net_amount: Money::new(300_000, Currency::Usd),
            frequency: Frequency::Biweekly,
            anchor: NaiveDate::from_ymd_opt(2026, 6, 5).unwrap(),
            deposit_account_id: Some(id),
        },
        bill_cmd(None),
        WriteCommand::ArchiveIncomeSource(income_id),
        WriteCommand::RestoreIncomeSource(income_id),
        WriteCommand::DeleteIncomeSource(income_id),
        WriteCommand::VoidTransaction {
            transaction_id: txn,
        },
    ]
}

#[test]
fn every_command_records_its_payload_version_hash_and_engine() {
    let (_dir, worker) = worker();
    let reader = worker.read_connection().unwrap();
    for command in commands() {
        let expected_payload = encode_payload(&command).unwrap();
        worker.dispatch(meta(), command).unwrap();
        let row = last_row(&reader);
        let kind = row.operation_type.as_str();
        // The payload is keyed by the same snake_case name the log row carries.
        assert!(
            row.payload.starts_with(format!("{{\"{kind}\":").as_bytes()),
            "{kind}: {}",
            String::from_utf8_lossy(&row.payload)
                .chars()
                .take(40)
                .collect::<String>()
        );
        assert_eq!(
            row.payload, expected_payload,
            "{kind}: the stored payload is exactly the encoded command"
        );
        assert_eq!(row.version, i64::from(PAYLOAD_SCHEMA_VERSION), "{kind}");
        assert_eq!(row.engine, ENGINE_VERSION, "{kind}");
        assert_eq!(row.hash.len(), 32, "{kind}");
        assert_eq!(
            row.hash,
            worker.last_write_set().unwrap().hash.to_vec(),
            "{kind}: the stored hash is the write set of this command"
        );
    }
}

#[test]
fn a_stored_payload_decodes_back_to_the_same_command() {
    let (_dir, worker) = worker();
    let reader = worker.read_connection().unwrap();
    for command in commands() {
        worker.dispatch(meta(), command).unwrap();
        let row = last_row(&reader);
        let kind = row.operation_type.as_str();
        let version = u32::try_from(row.version).unwrap();
        let decoded = decode_payload(version, &row.payload).unwrap();
        assert_eq!(
            encode_payload(&decoded).unwrap(),
            row.payload,
            "{kind}: decode then encode is the identity"
        );
    }
}

#[test]
fn the_hash_differs_between_commands_that_change_different_rows_and_is_empty_for_none() {
    let (_dir, worker) = worker();
    let reader = worker.read_connection().unwrap();
    worker.dispatch(meta(), create_account_cmd()).unwrap();
    let create = last_row(&reader).hash;
    assert_ne!(create, empty_set_hash(), "creating an account changes rows");

    // Restoring an active income source changes nothing: it is logged with the empty write set.
    let (_, income) = (IncomeSourceId::new(), income_cmd(None));
    let WriteCommand::CreateIncomeSource { id, .. } = income.clone() else {
        unreachable!()
    };
    worker.dispatch(meta(), income).unwrap();
    let income_hash = last_row(&reader).hash;
    assert_ne!(income_hash, create);
    worker
        .dispatch(meta(), WriteCommand::RestoreIncomeSource(id))
        .unwrap();
    assert_eq!(last_row(&reader).hash, empty_set_hash());
}

#[test]
fn a_command_the_format_cannot_carry_is_refused_before_anything_is_written() {
    let (_dir, worker) = worker();
    let reader = worker.read_connection().unwrap();
    let before = worker.operation_count().unwrap();
    let state = |conn: &Connection| -> (i64, i64) {
        (
            conn.query_row("SELECT COUNT(*) FROM income_sources", [], |r| r.get(0))
                .unwrap(),
            conn.query_row("SELECT COUNT(*) FROM command_idempotency_keys", [], |r| {
                r.get(0)
            })
            .unwrap(),
        )
    };
    let untouched = state(&reader);

    let secret = "Quiet \u{0}Pay";
    let command = WriteCommand::CreateIncomeSource {
        id: IncomeSourceId::new(),
        name: secret.to_owned(),
        net_amount: Money::new(1, Currency::Usd),
        frequency: Frequency::Monthly,
        anchor: NaiveDate::from_ymd_opt(2026, 6, 5).unwrap(),
        deposit_account_id: None,
    };
    let error = worker.dispatch(meta(), command).unwrap_err();
    let DbError::InvalidCommand(message) = &error else {
        panic!("expected a refusal, got {error:?}");
    };
    assert!(message.contains("U+0000"), "{message}");
    assert!(
        !message.contains("Quiet") && !message.contains("Pay"),
        "the refusal names the kind of value, never the value: {message}"
    );
    assert_eq!(worker.operation_count().unwrap(), before, "no op-log row");
    assert_eq!(state(&reader), untouched, "no row and no idempotency memo");
}

#[test]
fn a_replay_with_the_same_key_adds_no_row_and_keeps_the_original_record() {
    let (_dir, worker) = worker();
    let reader = worker.read_connection().unwrap();
    let key = uuid::Uuid::new_v4().to_string();
    let command = create_account_cmd();
    worker
        .dispatch(meta_with_key(&key), command.clone())
        .unwrap();
    let first = last_row(&reader);
    let count = worker.operation_count().unwrap();
    let outcome = worker.dispatch(meta_with_key(&key), command).unwrap();
    assert!(matches!(outcome, Outcome::Replayed { .. }), "{outcome:?}");
    assert_eq!(worker.operation_count().unwrap(), count);
    let again = last_row(&reader);
    assert_eq!(
        (again.payload, again.hash, again.version),
        (first.payload, first.hash, first.version)
    );
}

#[test]
fn rows_written_before_the_v2_columns_stay_audit_history() {
    // Pre-v2 rows have NULL in all four columns; this build's rows never do (the CHECK on the
    // table forbids a partial set, and migration 56's tests cover the old rows themselves).
    let (_dir, worker) = worker();
    worker.dispatch(meta(), create_account_cmd()).unwrap();
    let conn = worker.read_connection().unwrap();
    let (replayable, audit_only): (i64, i64) = conn
        .query_row(
            "SELECT SUM(payload_schema_version IS NOT NULL), SUM(payload_schema_version IS NULL)
             FROM operation_log",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(replayable >= 1);
    assert_eq!(
        audit_only, 0,
        "every row this build wrote carries the v2 columns"
    );
}
