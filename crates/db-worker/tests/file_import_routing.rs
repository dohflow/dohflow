//! File-import account routing (personal-cfo-tulv): a parsed file spanning
//! several source accounts stages each row against the real account its
//! label maps to, records that answer on each staged account, and leaves an
//! unmapped account's rows unstaged — counted, never folded into another
//! account. The single-target path keeps its original behavior.

mod common;

use chrono::{NaiveDate, Utc};
use common::*;
use core_ledger::AccountId;
use core_money::{Currency, Money};
use db_worker::*;
use importer_core::{ParsedAccount, ParsedBatch, ParsedRecord, ParsedTransaction};
use rusqlite::params;
use std::collections::BTreeMap;
use uuid::Uuid;

fn account(worker: &DbWorker) -> AccountId {
    let id = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(id)),
                opening_balance: None,
            },
        )
        .unwrap();
    id
}

fn row(label: &str, n: i64) -> ParsedRecord {
    ParsedRecord {
        external_id: None,
        source_hash: format!("sha256:{label}-{n}"),
        normalized_json: "{}".to_owned(),
        parse_confidence_bps: Some(10_000),
        transaction: Some(ParsedTransaction {
            posted_date: NaiveDate::from_ymd_opt(2026, 6, 1).unwrap(),
            transaction_date: None,
            raw_date: "06/01/2026".to_owned(),
            date_confidence_bps: 10_000,
            amount: Money::new(-100 * n, Currency::Usd),
            description: Some(format!("payee {n}")),
            category: None,
            normalized_merchant: Some(format!("payee {n}")),
            external_account: Some(label.to_owned()),
            txn_fingerprint: format!("2026-06-01|{}|payee {n}", -100 * n),
        }),
        balance: None,
    }
}

fn three_account_batch() -> ParsedBatch {
    let labels = ["Checking", "Credit Card", "Savings"];
    ParsedBatch {
        source_format: "csv".to_owned(),
        accounts: labels
            .iter()
            .map(|label| ParsedAccount {
                external_id: Some((*label).to_owned()),
                external_name: Some((*label).to_owned()),
                external_number_hash: None,
                proposed_subtype: None,
                currency: None,
            })
            .collect(),
        records: vec![
            row("Checking", 1),
            row("Credit Card", 2),
            row("Savings", 3),
            row("Checking", 4),
        ],
        warnings: Vec::new(),
        skipped: Vec::new(),
    }
}

fn new_batch(worker: &DbWorker) -> Uuid {
    let batch_id = Uuid::now_v7();
    worker
        .read_connection()
        .unwrap()
        .execute(
            "INSERT INTO source_batches
                (id, source_type, source_name, status, created_at, updated_at)
             VALUES (?1, 'csv', 'test file', 'staged', ?2, ?2)",
            params![batch_id, Utc::now().to_rfc3339()],
        )
        .unwrap();
    batch_id
}

/// `(external_name, matched_account_id)` per staged account, by name.
fn staged_accounts(worker: &DbWorker, batch_id: Uuid) -> Vec<(String, Option<Uuid>)> {
    let conn = worker.read_connection().unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT external_name, matched_account_id FROM staged_accounts
              WHERE source_batch_id = ?1 ORDER BY external_name",
        )
        .unwrap();
    stmt.query_map([batch_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// The proposed account of each staged transaction, in staging order.
fn proposed(worker: &DbWorker, ids: &[Uuid]) -> Vec<Uuid> {
    let conn = worker.read_connection().unwrap();
    ids.iter()
        .map(|id| {
            conn.query_row(
                "SELECT proposed_account_id FROM staged_transactions WHERE id = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap()
        })
        .collect()
}

#[test]
fn each_row_stages_against_its_mapped_account_and_unmapped_rows_are_counted() {
    let (_dir, worker) = worker();
    let checking = account(&worker).as_uuid();
    let card = account(&worker).as_uuid();
    let batch_id = new_batch(&worker);
    let map = BTreeMap::from([
        ("Checking".to_owned(), checking),
        ("Credit Card".to_owned(), card),
    ]);

    let (staged, not_routed) = worker
        .stage_parsed_batch_routed(batch_id, &three_account_batch(), &map)
        .unwrap();

    assert_eq!(not_routed, 1, "the Savings row is left out and counted");
    assert_eq!(proposed(&worker, &staged), vec![checking, card, checking]);
    assert_eq!(
        staged_accounts(&worker, batch_id),
        vec![
            ("Checking".to_owned(), Some(checking)),
            ("Credit Card".to_owned(), Some(card)),
            ("Savings".to_owned(), None),
        ],
        "each staged account records its own answer, not the batch's"
    );
    // No source record exists for the unmapped row.
    let records: i64 = worker
        .read_connection()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM source_records WHERE source_batch_id = ?1",
            [batch_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(records, 3);
}

#[test]
fn the_single_target_path_still_stages_every_row_into_its_one_account() {
    let (_dir, worker) = worker();
    let target = account(&worker).as_uuid();
    let batch_id = new_batch(&worker);

    let staged = worker
        .stage_parsed_batch(batch_id, &three_account_batch(), target)
        .unwrap();

    assert_eq!(proposed(&worker, &staged), vec![target; 4]);
}
