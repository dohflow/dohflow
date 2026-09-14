//! Canonical ledger schema (bead personal-cfo-19s; ADR 0007): the balance
//! invariant is enforced DB-side at insert time, money is stored as integer
//! minor units + currency + exponent, the documented indexes exist, and the
//! schema is deterministic.

use db_worker::DbWorker;
use proptest::prelude::*;
use rusqlite::{params, Connection};
use tempfile::TempDir;
use uuid::Uuid;

const KEY: &str = "correct horse battery staple";

fn worker() -> (TempDir, DbWorker) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("vault.db");
    let worker = DbWorker::open(&path, KEY).unwrap();
    (dir, worker)
}

fn acct() -> String {
    Uuid::now_v7().to_string()
}

/// Insert a ledger transaction with the given `(ledger_account_id, minor_units)`
/// postings. Postings are written first and the header last, so the balance
/// trigger fires once the transaction is complete. The whole thing runs in one
/// SQLite transaction, so a trigger ABORT leaves no orphan rows.
fn insert_transaction(conn: &mut Connection, postings: &[(String, i64)]) -> rusqlite::Result<()> {
    let txn_id = Uuid::now_v7().to_string();
    let op_id = Uuid::now_v7().to_string();
    let tx = conn.transaction()?;
    for (account, minor_units) in postings {
        tx.execute(
            "INSERT INTO ledger_postings (
                transaction_id, ledger_account_id, minor_units, currency, currency_exponent, posting_date
            ) VALUES (?1, ?2, ?3, 'USD', 2, '2026-01-01')",
            params![txn_id, account, minor_units],
        )?;
    }
    tx.execute(
        "INSERT INTO ledger_transactions (id, operation_id, occurred_at, currency)
         VALUES (?1, ?2, '2026-01-01T00:00:00Z', 'USD')",
        params![txn_id, op_id],
    )?;
    tx.commit()?;
    Ok(())
}

fn count_transactions(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM ledger_transactions", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn balanced_transaction_is_accepted() {
    let (_dir, worker) = worker();
    let mut conn = worker.read_connection().unwrap();
    // Simple expense: credit cash, debit an expense account.
    insert_transaction(&mut conn, &[(acct(), -5_000), (acct(), 5_000)]).unwrap();
    assert_eq!(count_transactions(&conn), 1);
}

#[test]
fn unbalanced_transaction_is_rejected_at_insert_time() {
    let (_dir, worker) = worker();
    let mut conn = worker.read_connection().unwrap();
    let result = insert_transaction(&mut conn, &[(acct(), -5_000), (acct(), 4_000)]);
    assert!(result.is_err(), "DB must reject an unbalanced transaction");
    // And nothing was persisted (clean rollback).
    assert_eq!(count_transactions(&conn), 0);
}

#[test]
fn all_canonical_transaction_shapes_balance() {
    // The §9.4 shapes are all just balanced multi-posting transactions.
    let (cash, exp, savings, cc, exp2) = (acct(), acct(), acct(), acct(), acct());
    let shapes: Vec<(&str, Vec<(String, i64)>)> = vec![
        (
            "simple expense",
            vec![(cash.clone(), -5_000), (exp.clone(), 5_000)],
        ),
        (
            "transfer",
            vec![(cash.clone(), -10_000), (savings.clone(), 10_000)],
        ),
        (
            "cc purchase",
            vec![(cc.clone(), 5_000), (exp.clone(), -5_000)],
        ),
        (
            "cc payment",
            vec![(cash.clone(), -10_000), (cc.clone(), 10_000)],
        ),
        (
            "split",
            vec![
                (cash.clone(), -10_000),
                (exp.clone(), 6_000),
                (exp2.clone(), 4_000),
            ],
        ),
        (
            "reversal",
            vec![(cash.clone(), 5_000), (exp.clone(), -5_000)],
        ),
        (
            "amendment",
            vec![(cash.clone(), -1_000), (exp.clone(), 1_000)],
        ),
    ];
    for (name, postings) in shapes {
        let (_dir, worker) = worker();
        let mut conn = worker.read_connection().unwrap();
        insert_transaction(&mut conn, &postings).unwrap_or_else(|e| panic!("{name} failed: {e}"));
        assert_eq!(count_transactions(&conn), 1, "{name}");
    }
}

#[test]
fn money_is_stored_as_integer_minor_units_with_exponent() {
    let (_dir, worker) = worker();
    let mut conn = worker.read_connection().unwrap();
    insert_transaction(&mut conn, &[(acct(), -5_000), (acct(), 5_000)]).unwrap();
    let (units, exponent, currency): (i64, i64, String) = conn
        .query_row(
            "SELECT minor_units, currency_exponent, currency FROM ledger_postings LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(exponent, 2);
    assert_eq!(currency, "USD");
    assert!(units == -5_000 || units == 5_000);
}

#[test]
fn account_date_index_is_used() {
    let (_dir, worker) = worker();
    let conn = worker.read_connection().unwrap();
    let mut stmt = conn
        .prepare(
            "EXPLAIN QUERY PLAN
             SELECT minor_units FROM ledger_postings
             WHERE ledger_account_id = ?1 AND posting_date >= ?2",
        )
        .unwrap();
    let detail: Vec<String> = stmt
        .query_map(params!["acct", "2020-01-01"], |r| r.get::<_, String>(3))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let plan = detail.join(" ");
    assert!(
        plan.contains("idx_ledger_postings_account_date"),
        "expected index use, plan was: {plan}"
    );
}

fn schema_fingerprint(conn: &Connection) -> String {
    let mut stmt = conn
        .prepare(
            "SELECT type || '|' || name || '|' || COALESCE(sql, '')
             FROM sqlite_master ORDER BY type, name",
        )
        .unwrap();
    let rows: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    rows.join("\n")
}

#[test]
fn schema_is_deterministic() {
    let (_d1, w1) = worker();
    let (_d2, w2) = worker();
    assert_eq!(
        schema_fingerprint(&w1.read_connection().unwrap()),
        schema_fingerprint(&w2.read_connection().unwrap()),
        "fresh vaults must have an identical schema fingerprint"
    );
    // Note: the full migration upgrade/downgrade roundtrip (19s AC) is deferred
    // to the migration framework (personal-cfo-wkn).
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    #[test]
    fn balanced_postings_accepted_unbalanced_rejected(
        amounts in prop::collection::vec(-1_000_000i64..1_000_000, 1..6),
        imbalance in prop::sample::select(vec![0i64, 1, -1, 100, -100]),
    ) {
        let (_dir, worker) = worker();
        let mut conn = worker.read_connection().unwrap();

        let balancer: i64 = amounts.iter().sum();
        let mut postings: Vec<(String, i64)> = amounts.iter().map(|&a| (acct(), a)).collect();
        // Close the transaction out, then nudge by `imbalance`.
        postings.push((acct(), -balancer + imbalance));

        let result = insert_transaction(&mut conn, &postings);
        if imbalance == 0 {
            prop_assert!(result.is_ok(), "balanced transaction must be accepted");
            prop_assert_eq!(count_transactions(&conn), 1);
        } else {
            prop_assert!(result.is_err(), "unbalanced transaction must be rejected");
            prop_assert_eq!(count_transactions(&conn), 0);
        }
    }
}
