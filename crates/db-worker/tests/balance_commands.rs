//! Balance observations through the bus (personal-cfo-5ymg8.3; ADR 0027): manual assertions are
//! never last-wins, and connector readings are a monotone in-place upsert whose row id is a
//! content id and which names no device-local source record.

mod common;

use chrono::NaiveDate;
use common::KEY;
use core_ledger::{Account, AccountFlags, AccountId, CashflowRole, LedgerAccountId};
use core_money::{Currency, Money};
use db_worker::{ActorType, CommandMeta, DbError, DbWorker, IssuedAt, WriteCommand};
use uuid::Uuid;

fn u(n: u128) -> Uuid {
    Uuid::from_u128(0x0198_c2c1_5f0a_7c3d_8a4e_9b1d_0000_0000 + n)
}
fn usd(minor: i64) -> Money {
    Money::new(minor, Currency::Usd)
}
fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// 2026-10-20T00:00:00Z in microseconds, plus `offset` seconds.
fn at(offset: i64) -> IssuedAt {
    IssuedAt::from_unix_micros(1_792_454_400_000_000 + offset * 1_000_000)
}

fn meta(n: u128, issued_at: IssuedAt) -> CommandMeta {
    CommandMeta {
        command_id: u(1_000 + n),
        correlation_id: u(2_000 + n),
        causation_id: None,
        actor_type: ActorType::Importer,
        actor_id: "tester".to_owned(),
        idempotency_key: format!("00000000-0000-4000-8000-{:012x}", 3_000 + n),
        issued_at,
    }
}

fn vault() -> (tempfile::TempDir, DbWorker, AccountId) {
    let dir = tempfile::tempdir().unwrap();
    let worker = DbWorker::open(dir.path().join("v.vault"), KEY).unwrap();
    let account = AccountId::from_uuid(u(1));
    worker
        .dispatch(
            meta(900, at(-10)),
            WriteCommand::CreateAccount {
                account: Box::new(Account::new(
                    account,
                    LedgerAccountId::from_uuid(u(501)),
                    "Checking",
                    CashflowRole::LiquidCash,
                    Currency::Usd,
                    AccountFlags::default(),
                )),
                opening_balance: None,
            },
        )
        .unwrap();
    (dir, worker, account)
}

fn connector(account: AccountId, observed: NaiveDate, minor: i64) -> WriteCommand {
    WriteCommand::RecordConnectorBalance {
        account_id: account,
        observed_at: observed,
        amount: usd(minor),
    }
}

type Row = (String, i64, String, Option<Uuid>, String);

/// `(id hex, minor, source, source_record_id, created_at)` of the observations of a day.
fn rows(worker: &DbWorker, observed: &str) -> Vec<Row> {
    let conn = worker.read_connection().unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT hex(id), balance_amount_minor, source, source_record_id, created_at
               FROM balance_observations WHERE observed_at = ?1 ORDER BY source, id",
        )
        .unwrap();
    stmt.query_map([observed], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

#[test]
fn a_connector_reading_updates_its_row_in_place_and_never_deletes_it() {
    let (_dir, worker, account) = vault();
    worker
        .dispatch(
            meta(1, at(0)),
            connector(account, day(2026, 10, 19), 100_000),
        )
        .unwrap();
    let first = rows(&worker, "2026-10-19");
    assert_eq!(first.len(), 1);
    assert_eq!(
        (first[0].1, first[0].2.as_str()),
        (100_000, "connector_sync")
    );
    assert!(
        first[0].3.is_none(),
        "no device-local source record is named"
    );
    assert_eq!(worker.last_write_set().unwrap().rows.len(), 1);

    // A newer reading for the same day: the same row, new amount, newer created_at.
    worker
        .dispatch(
            meta(2, at(60)),
            connector(account, day(2026, 10, 19), 101_500),
        )
        .unwrap();
    let second = rows(&worker, "2026-10-19");
    assert_eq!(second.len(), 1, "one row per account, day and connector");
    assert_eq!(second[0].0, first[0].0, "the id is stable");
    assert_eq!(second[0].1, 101_500);
    assert!(second[0].4 > first[0].4);
}

#[test]
fn a_stale_connector_command_cannot_overwrite_a_newer_reading() {
    let (_dir, worker, account) = vault();
    worker
        .dispatch(
            meta(1, at(100)),
            connector(account, day(2026, 10, 19), 200_000),
        )
        .unwrap();
    let before = rows(&worker, "2026-10-19");
    // An offline device's refresh, issued earlier and applied later.
    worker
        .dispatch(
            meta(2, at(10)),
            connector(account, day(2026, 10, 19), 150_000),
        )
        .unwrap();
    assert!(
        worker.last_write_set().unwrap().rows.is_empty(),
        "an older command is a no-op"
    );
    assert_eq!(rows(&worker, "2026-10-19"), before);
}

#[test]
fn the_row_id_depends_on_the_vault_account_and_day_and_not_on_the_command() {
    let (dir, worker, account) = vault();
    worker.checkpoint_truncate().unwrap();
    let base = dir.path().join("v.vault");
    let copy = |name: &str| {
        let path = dir.path().join(name);
        std::fs::copy(&base, &path).unwrap();
        DbWorker::open(&path, KEY).unwrap()
    };
    // Two copies of one vault, two DIFFERENT commands for the same account and day.
    let (a, b) = (copy("a.vault"), copy("b.vault"));
    a.dispatch(meta(1, at(0)), connector(account, day(2026, 10, 19), 1))
        .unwrap();
    b.dispatch(meta(2, at(500)), connector(account, day(2026, 10, 19), 9))
        .unwrap();
    assert_eq!(rows(&a, "2026-10-19")[0].0, rows(&b, "2026-10-19")[0].0);
    // Another day is another row.
    a.dispatch(meta(3, at(1)), connector(account, day(2026, 10, 18), 1))
        .unwrap();
    assert_ne!(rows(&a, "2026-10-18")[0].0, rows(&a, "2026-10-19")[0].0);
}

#[test]
fn a_manual_assertion_is_never_last_wins_and_the_later_one_anchors() {
    let (_dir, worker, account) = vault();
    worker
        .record_balance_assertion(u(70), account, usd(500_000), day(2026, 10, 19))
        .unwrap();
    worker
        .record_balance_assertion(u(71), account, usd(510_000), day(2026, 10, 19))
        .unwrap();
    let both = rows(&worker, "2026-10-19");
    assert_eq!(both.len(), 2, "both assertions are kept");
    assert!(both.iter().all(|r| r.2 == "manual" && r.3.is_none()));
    // A reused id is refused.
    let again = worker.record_balance_assertion(u(70), account, usd(1), day(2026, 10, 19));
    assert!(
        matches!(again, Err(DbError::InvalidCommand(_))),
        "{again:?}"
    );
}

#[test]
fn a_manual_assertion_and_a_connector_reading_for_one_day_both_stay() {
    let (_dir, worker, account) = vault();
    worker
        .dispatch(
            meta(1, at(0)),
            connector(account, day(2026, 10, 19), 100_000),
        )
        .unwrap();
    worker
        .record_balance_assertion(u(70), account, usd(99_000), day(2026, 10, 19))
        .unwrap();
    let both = rows(&worker, "2026-10-19");
    assert_eq!(both.len(), 2);
    assert_eq!(
        both.iter().map(|r| r.2.as_str()).collect::<Vec<_>>(),
        ["connector_sync", "manual"]
    );
}

#[test]
fn refusals_are_a_function_of_the_command_and_the_base() {
    let (_dir, worker, account) = vault();
    // An unknown account and a currency other than the account's.
    let unknown = worker.dispatch(
        meta(1, at(0)),
        connector(AccountId::from_uuid(u(99)), day(2026, 10, 19), 1),
    );
    assert!(
        matches!(unknown, Err(DbError::InvalidCommand(_))),
        "{unknown:?}"
    );
    let euro = worker.dispatch(
        meta(2, at(0)),
        WriteCommand::RecordConnectorBalance {
            account_id: account,
            observed_at: day(2026, 10, 19),
            amount: Money::new(1, Currency::Eur),
        },
    );
    assert!(matches!(euro, Err(DbError::InvalidCommand(_))), "{euro:?}");
    // A reading dated after the household-local date of the command's issued_at.
    let future = worker.dispatch(meta(3, at(0)), connector(account, day(2026, 10, 25), 1));
    assert!(
        matches!(future, Err(DbError::InvalidCommand(_))),
        "{future:?}"
    );
    // The same date is fine under a LATER issued_at: the verdict reads the command, not the clock.
    worker
        .dispatch(
            meta(4, at(86_400 * 10)),
            connector(account, day(2026, 10, 25), 1),
        )
        .unwrap();
    assert!(
        rows(&worker, "2026-10-19").is_empty(),
        "the refusals wrote nothing"
    );
}

#[test]
fn a_legacy_connector_row_with_a_random_id_is_adopted_not_duplicated() {
    let (_dir, worker, account) = vault();
    // A row written before the command existed: a random id, a source record.
    worker
        .read_connection()
        .unwrap()
        .execute(
            "INSERT INTO balance_observations
                (id, account_id, observed_at, balance_amount_minor, balance_currency,
                 source, source_record_id, reconciliation_session_id, created_at)
             VALUES (?1, ?2, '2026-10-19', 5, 'USD', 'connector_sync', ?3, NULL,
                     '2026-10-19T00:00:00+00:00')",
            rusqlite::params![u(77), account.as_uuid(), u(78)],
        )
        .unwrap();
    worker
        .dispatch(meta(1, at(0)), connector(account, day(2026, 10, 19), 42))
        .unwrap();
    let after = rows(&worker, "2026-10-19");
    assert_eq!(after.len(), 1, "adopted in place, not duplicated");
    assert_eq!(after[0].1, 42);
    assert!(
        after[0].3.is_none(),
        "the source record is dropped from the replicated row"
    );
}
