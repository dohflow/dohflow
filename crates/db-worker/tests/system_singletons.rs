//! System singletons and seeds (personal-cfo-32fmp.3; ADR 0074 Addendum G): the
//! `(role, currency)` map is class 1 and looked up first, a new pair gets a content-derived id,
//! and the seeders run only in the transaction that creates the vault.

mod common;

use common::KEY;
use core_ids::DerivedIdKey;
use core_ledger::{Account, AccountFlags, AccountId, CashflowRole, LedgerAccountId};
use core_money::{Currency, Money};
use db_worker::{ActorType, CommandMeta, DbWorker, IssuedAt, WriteCommand};
use rusqlite::params;
use std::collections::BTreeMap;
use std::path::Path;
use uuid::Uuid;

fn u(n: u128) -> Uuid {
    Uuid::from_u128(0x0198_c2c1_5f0a_7c3d_8a4e_9b1d_0000_0000 + n)
}

fn meta(n: u128, offset: i64) -> CommandMeta {
    CommandMeta {
        command_id: u(1_000 + n),
        correlation_id: u(2_000 + n),
        causation_id: None,
        actor_type: ActorType::User,
        actor_id: "tester".to_owned(),
        idempotency_key: format!("00000000-0000-4000-8000-{:012x}", 3_000 + n),
        issued_at: IssuedAt::from_unix_micros(1_792_454_400_000_000 + offset * 1_000_000),
    }
}

/// A checking account with an opening balance in `currency` (its equity posting needs the
/// system account for that currency).
fn open_account(n: u128, currency: Currency) -> WriteCommand {
    WriteCommand::CreateAccount {
        account: Box::new(Account::new(
            AccountId::from_uuid(u(n)),
            LedgerAccountId::from_uuid(u(500 + n)),
            "Checking",
            CashflowRole::LiquidCash,
            currency,
            AccountFlags::default(),
        )),
        opening_balance: Some(Money::new(10_000, currency)),
    }
}

fn vault_id(worker: &DbWorker) -> Uuid {
    worker.vault_id().unwrap()
}

fn system_map(worker: &DbWorker) -> BTreeMap<(String, String), Uuid> {
    let conn = worker.read_connection().unwrap();
    let mut stmt = conn
        .prepare("SELECT role, currency, ledger_account_id FROM system_ledger_accounts")
        .unwrap();
    let rows = stmt
        .query_map([], |r| Ok(((r.get(0)?, r.get(1)?), r.get(2)?)))
        .unwrap();
    rows.map(Result::unwrap).collect()
}

fn count(worker: &DbWorker, table: &str) -> i64 {
    worker
        .read_connection()
        .unwrap()
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

/// Every table's row count, to show that opening a vault writes no row.
fn all_counts(worker: &DbWorker) -> BTreeMap<String, i64> {
    let conn = worker.read_connection().unwrap();
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
        .unwrap();
    let names: Vec<String> = stmt
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    names
        .into_iter()
        .map(|name| {
            let n = count(worker, &name);
            (name, n)
        })
        .collect()
}

/// The category rows (id, name, parent) in a stable order.
fn categories(worker: &DbWorker) -> Vec<(Uuid, String, Option<Uuid>, String)> {
    let conn = worker.read_connection().unwrap();
    let mut stmt = conn
        .prepare("SELECT id, name, parent_id, created_at FROM categories ORDER BY id")
        .unwrap();
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap();
    rows.map(Result::unwrap).collect()
}

#[test]
fn a_new_vault_seeds_derived_system_accounts_and_categories_and_one_creation_time() {
    let dir = tempfile::tempdir().unwrap();
    let worker = DbWorker::open(dir.path().join("v.vault"), KEY).unwrap();
    let key = DerivedIdKey::for_vault(&vault_id(&worker));
    let map = system_map(&worker);
    assert_eq!(map.len(), 3, "the USD trio");
    for ((role, currency), id) in &map {
        assert_eq!(
            *id,
            key.content_id(
                "system-ledger-account",
                &[role.as_bytes(), currency.as_bytes()]
            )
            .unwrap(),
            "{role} {currency}"
        );
    }
    // A seeded category is a content id of its names, and every seed row carries the vault's
    // creation time, never the clock.
    let created: String = worker
        .read_connection()
        .unwrap()
        .query_row(
            "SELECT created_at FROM vault_metadata WHERE singleton = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let rows = categories(&worker);
    assert!(rows.len() > 20);
    let income = key.content_id("category.seed", &[b"Income"]).unwrap();
    let salary = key
        .content_id("category.seed", &[b"Income", b"Salary"])
        .unwrap();
    assert!(rows.iter().any(|r| r.0 == income && r.2.is_none()));
    assert!(rows
        .iter()
        .any(|r| r.0 == salary && r.1 == "Salary" && r.2 == Some(income)));
    assert!(rows.iter().all(|r| r.3 == created));
}

#[test]
fn opening_an_existing_vault_writes_no_row_whether_its_ids_are_derived_or_legacy() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.vault");
    {
        let worker = DbWorker::open(&path, KEY).unwrap();
        worker.checkpoint_truncate().unwrap();
    }
    let derived = {
        let worker = DbWorker::open(&path, KEY).unwrap();
        (
            all_counts(&worker),
            system_map(&worker),
            categories(&worker),
        )
    };
    // A legacy vault: random system ids, as every vault created before this change holds.
    {
        let worker = DbWorker::open(&path, KEY).unwrap();
        let conn = worker.read_connection().unwrap();
        for (n, ((role, currency), old)) in derived.1.iter().enumerate() {
            let legacy = Uuid::from_u128(0x7000 + n as u128);
            conn.execute(
                "UPDATE ledger_accounts SET id = ?1 WHERE id = ?2",
                params![legacy, old],
            )
            .unwrap();
            conn.execute(
                "UPDATE system_ledger_accounts SET ledger_account_id = ?1
                  WHERE role = ?2 AND currency = ?3",
                params![legacy, role, currency],
            )
            .unwrap();
        }
    }
    let legacy_before = {
        let worker = DbWorker::open(&path, KEY).unwrap();
        (all_counts(&worker), system_map(&worker))
    };
    let worker = DbWorker::open(&path, KEY).unwrap();
    assert_eq!(all_counts(&worker), legacy_before.0, "an open wrote a row");
    assert_eq!(
        system_map(&worker),
        legacy_before.1,
        "an open rewrote a legacy system id"
    );
    assert_eq!(all_counts(&worker), derived.0);
    assert_eq!(categories(&worker), derived.2, "an open changed the seeds");
}

fn copy_of(dir: &Path, base: &Path, name: &str) -> DbWorker {
    let path = dir.join(name);
    std::fs::copy(base, &path).unwrap();
    DbWorker::open(&path, KEY).unwrap()
}

fn equity_posting_account(worker: &DbWorker, currency: &str) -> Uuid {
    worker
        .read_connection()
        .unwrap()
        .query_row(
            "SELECT ledger_account_id FROM system_ledger_accounts
              WHERE role = 'opening_balance_equity' AND currency = ?1",
            [currency],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn a_replica_of_a_legacy_origin_posts_to_the_origins_ids_with_an_identical_write_set() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("origin.vault");
    // A legacy origin: random system ids.
    {
        let worker = DbWorker::open(&base, KEY).unwrap();
        let conn = worker.read_connection().unwrap();
        for (n, ((role, currency), old)) in system_map(&worker).iter().enumerate() {
            let legacy = Uuid::from_u128(0x7000 + n as u128);
            conn.execute(
                "UPDATE ledger_accounts SET id = ?1 WHERE id = ?2",
                params![legacy, old],
            )
            .unwrap();
            conn.execute(
                "UPDATE system_ledger_accounts SET ledger_account_id = ?1
                  WHERE role = ?2 AND currency = ?3",
                params![legacy, role, currency],
            )
            .unwrap();
        }
        worker.checkpoint_truncate().unwrap();
    }
    let origin = copy_of(dir.path(), &base, "a.vault");
    // The replica starts from the origin's genesis: the class-1 rows, including the map.
    let replica = copy_of(dir.path(), &base, "b.vault");
    let ledgers_before = count(&replica, "ledger_accounts");
    assert_eq!(
        ledgers_before,
        count(&origin, "ledger_accounts"),
        "opening the replica created no ledger account"
    );
    assert_eq!(
        system_map(&replica),
        system_map(&origin),
        "the replica holds the origin's map"
    );
    let legacy_equity = equity_posting_account(&origin, "USD");
    origin
        .dispatch(meta(1, 0), open_account(1, Currency::Usd))
        .unwrap();
    replica
        .dispatch(meta(1, 0), open_account(1, Currency::Usd))
        .unwrap();
    assert_eq!(
        origin.last_write_set().unwrap(),
        replica.last_write_set().unwrap(),
        "the same command has the same write set on the replica"
    );
    assert_eq!(equity_posting_account(&replica, "USD"), legacy_equity);
    assert_eq!(
        count(&replica, "ledger_accounts"),
        ledgers_before + 1,
        "only the account's own row"
    );
}

#[test]
fn two_devices_creating_one_new_pair_converge_and_a_rebased_command_finds_the_row() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("v.vault");
    {
        let worker = DbWorker::open(&base, KEY).unwrap();
        worker.checkpoint_truncate().unwrap();
    }
    let a = copy_of(dir.path(), &base, "a.vault");
    let b = copy_of(dir.path(), &base, "b.vault");
    // Both devices open an EUR account: the EUR system pair does not exist on either base.
    assert!(!system_map(&a).contains_key(&("opening_balance_equity".into(), "EUR".into())));
    a.dispatch(meta(1, 0), open_account(1, Currency::Eur))
        .unwrap();
    b.dispatch(meta(2, 5), open_account(2, Currency::Eur))
        .unwrap();
    let eur_a = equity_posting_account(&a, "EUR");
    assert_eq!(
        eur_a,
        equity_posting_account(&b, "EUR"),
        "derived, so the same id"
    );
    // A's command rebased onto B finds the row: no second system account, and its own ids are
    // unchanged.
    let before = count(&b, "ledger_accounts");
    b.dispatch(meta(3, 10), open_account(3, Currency::Eur))
        .unwrap();
    assert_eq!(
        count(&b, "ledger_accounts"),
        before + 1,
        "only the new account's own row"
    );
    assert_eq!(
        b.read_connection()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM system_ledger_accounts WHERE currency = 'EUR'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1,
        "exactly one EUR system account on the rebased base"
    );
    assert_eq!(equity_posting_account(&b, "EUR"), eur_a);
}

/// Known answers for the two content-id tags under a fixed vault id, so a change to either tag,
/// to the parts or to the derivation shows up as a diff here and not as drifted ids.
#[test]
fn known_answers_for_the_system_account_and_category_seed_ids() {
    let key = DerivedIdKey::for_vault(&Uuid::from_u128(0x0198_c2c1_5f0a_7c3d_8a4e_9b1d_0000_0001));
    let got = [
        key.content_id(
            "system-ledger-account",
            &[b"opening_balance_equity", b"USD"],
        )
        .unwrap(),
        key.content_id(
            "system-ledger-account",
            &[b"opening_balance_equity", b"EUR"],
        )
        .unwrap(),
        key.content_id("category.seed", &[b"Income"]).unwrap(),
        key.content_id("category.seed", &[b"Income", b"Salary"])
            .unwrap(),
    ]
    .map(|id| id.to_string());
    assert_eq!(
        got,
        [
            "3b5911f8-2c74-82b5-9042-9c6ae7fbffb0",
            "fd3381f5-8ab1-849b-9adb-e4344c975d60",
            "00dd66eb-5f7f-80af-b2d3-b82ef87254d6",
            "449a8034-d76c-85e1-8e05-0b0631141eb4",
        ]
    );
}
