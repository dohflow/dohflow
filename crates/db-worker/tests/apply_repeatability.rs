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
            "set debt terms",
            WriteCommand::SetDebtTerms {
                account_id: account(28),
                terms: db_worker::DebtTermsInput {
                    apr_bps: Some(2_499),
                    statement_close_day: Some(15),
                    payment_due_day: Some(10),
                    grace_period_days: Some(25),
                    credit_limit_minor: Some(500_000),
                    repayment_philosophy: db_worker::RepaymentPhilosophy::PayStatementBalance,
                    fixed_amount_minor: None,
                    min_payment_percent_bps: Some(200),
                    min_payment_floor_minor: Some(2_500),
                    paying_source_account_id: Some(account(1)),
                    original_principal_minor: None,
                },
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
            "record a statement balance for a closed cycle",
            WriteCommand::SetCardStatementBalance {
                account_id: account(28),
                cycle_close: NaiveDate::from_ymd_opt(2001, 8, 15).unwrap(),
                statement_balance_minor: Some(184_500),
            },
        ),
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
    let mut card = common::meta();
    card.issued_at = setup.issued_at;
    worker
        .dispatch(
            card,
            WriteCommand::CreateAccount {
                account: Box::new(core_ledger::Account::new(
                    account(28),
                    core_ledger::LedgerAccountId::from_uuid(u(128)),
                    "Visa",
                    core_ledger::CashflowRole::CreditFacility,
                    Currency::Usd,
                    core_ledger::AccountFlags::default(),
                )),
                opening_balance: None,
            },
        )
        .unwrap();
    let mut subtype = common::meta();
    subtype.issued_at = setup.issued_at;
    worker
        .dispatch(
            subtype,
            WriteCommand::SetAccountSubtype {
                id: account(28),
                subtype: Some(core_ledger::AccountSubtype::CreditCard),
            },
        )
        .unwrap();
    worker.checkpoint_truncate().unwrap();
    drop(worker);
    path
}

/// Apply the payload-id commands to a copy of `base`, returning `label hash` lines.
fn hashes_on_a_copy(base: &Path, dir: &Path) -> Vec<String> {
    let path = dir.join("child.vault");
    std::fs::copy(base, &path).unwrap();
    let worker = open(&path);
    let mut lines = Vec::new();
    for (n, (label, command)) in commands().into_iter().enumerate() {
        worker
            .dispatch(meta(n as u128), command)
            .unwrap_or_else(|e| panic!("{label}: {e}"));
        let hash: String = worker
            .last_write_set()
            .unwrap()
            .hash
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        lines.push(format!("HASH {label}: {hash}"));
    }
    lines
}

/// The child half of the time-zone test: run only when the parent re-invokes this binary with
/// `TZ` set and `DOHFLOW_TZ_BASE` naming the base vault.
#[test]
#[ignore = "run by apply_is_independent_of_the_device_time_zone in a child process"]
fn child_apply_under_the_inherited_time_zone() {
    let base = PathBuf::from(std::env::var("DOHFLOW_TZ_BASE").expect("run by the parent test"));
    let dir = tempfile::tempdir().unwrap();
    for line in hashes_on_a_copy(&base, dir.path()) {
        println!("{line}");
    }
}

#[test]
fn apply_is_independent_of_the_device_time_zone() {
    // The device's zone must not change what apply writes. Each zone runs in its own child
    // process (a zone set in-process would race the other tests), on a copy of the same base.
    let dir = tempfile::tempdir().unwrap();
    let base = base(dir.path());
    let run = |zone: &str| -> Vec<String> {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "child_apply_under_the_inherited_time_zone",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("TZ", zone)
            .env("DOHFLOW_TZ_BASE", &base)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "child under {zone} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let lines: Vec<String> = String::from_utf8_lossy(&output.stdout)
            .lines()
            // The harness prints "test name ... " before the first line of output.
            .filter_map(|l| l.find("HASH ").map(|at| l[at..].to_owned()))
            .collect();
        assert_eq!(lines.len(), commands().len(), "under {zone}: {lines:?}");
        lines
    };
    let chicago = run("America/Chicago");
    let auckland = run("Pacific/Auckland");
    assert_eq!(
        chicago, auckland,
        "the write sets differ between two device time zones"
    );
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

#[test]
fn a_statement_for_a_cycle_not_yet_closed_at_issued_at_is_refused_whatever_the_wall_clock() {
    // The close date is in the "future" of the command's issued_at (2001) but long past by the
    // wall clock (2026): a verdict that depended on the clock would differ between the device that
    // issued the command and a replica applying it later. Both refuse.
    let dir = tempfile::tempdir().unwrap();
    let base = base(dir.path());
    let (a_path, b_path) = (dir.path().join("a.vault"), dir.path().join("b.vault"));
    std::fs::copy(&base, &a_path).unwrap();
    std::fs::copy(&base, &b_path).unwrap();
    for path in [a_path, b_path] {
        let worker = open(&path);
        let err = worker
            .dispatch(
                meta(0),
                WriteCommand::SetCardStatementBalance {
                    account_id: account(28),
                    cycle_close: NaiveDate::from_ymd_opt(2001, 12, 1).unwrap(),
                    statement_balance_minor: Some(100),
                },
            )
            .unwrap_err();
        assert!(
            err.to_string().contains("already closed"),
            "refused on the issued_at date: {err}"
        );
    }
}
