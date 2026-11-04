//! The import commit is replicable (personal-cfo-0g528.1, SYNC-2c, AC5): an ORIGIN stages a file and
//! commits its rows; a REPLICA that has no staging rows applies the same envelopes and ends with
//! byte-identical canonical and provenance rows. Both start as copies of one base vault, so they share
//! the vault id (and so the content-id key) and the class-1 and class-1b state a genesis snapshot
//! would carry: the accounts, the category, and the source batch and records of the file.
//!
//! The origin's own bookkeeping (the staged rows, the flagged decision) is class 3 and class 4 today
//! and is not compared; every row the COMMAND writes is.

mod common;

use chrono::{DateTime, NaiveDate, Utc};
use common::*;
use core_ledger::{
    Account, AccountFlags, AccountId, CashflowRole, CategoryId, LedgerAccountId, SourceBatchId,
    SourceRecordId, StagedTransactionId,
};
use core_money::{Currency, Money};
use db_worker::payload::{decode_payload, PAYLOAD_SCHEMA_VERSION};
use db_worker::*;
use importer_core::{ParsedAccount, ParsedBatch, ParsedRecord, ParsedTransaction};
use rusqlite::Connection;
use std::fmt::Write as _;
use std::path::Path;
use uuid::Uuid;

fn u(n: u128) -> Uuid {
    Uuid::from_u128(0x0198_c2c1_5f0a_7c3d_8a4e_9b1d_0000_0000 + n)
}

const BATCH: u128 = 50;
const COFFEE: u128 = 60;

fn rows() -> Vec<(u128, &'static str, i64, &'static str, Option<&'static str>)> {
    // (n, fingerprint, minor, merchant, category). Rows 3 and 4 share a fingerprint: row 4 is flagged
    // on the origin and never travels. Rows 5 and 6 are identical in content (one source hash) and
    // stay two transactions.
    vec![
        (1, "fp-1", -450, "Corner Cafe", Some("Coffee Shops")),
        (2, "fp-2", -120_000, "Landlord", None),
        (3, "fp-3", -700, "Twin Shop", None),
        (4, "fp-3", -700, "Twin Shop", None),
        (5, "fp-5", -900, "Same Row", None),
        (6, "fp-6", -900, "Same Row", None),
    ]
}

fn source_hash(n: u128) -> String {
    // Rows 5 and 6 hash to the same record.
    format!("sha256:row-{}", if n == 6 { 5 } else { n })
}

fn parsed() -> ParsedBatch {
    ParsedBatch {
        source_format: "csv".to_owned(),
        accounts: vec![ParsedAccount {
            external_id: None,
            external_name: None,
            external_number_hash: None,
            proposed_subtype: None,
            currency: None,
        }],
        records: rows()
            .into_iter()
            .map(|(n, fingerprint, minor, merchant, category)| ParsedRecord {
                external_id: None,
                source_hash: source_hash(n),
                normalized_json: "{}".to_owned(),
                parse_confidence_bps: Some(10_000),
                transaction: Some(ParsedTransaction {
                    posted_date: NaiveDate::from_ymd_opt(2026, 6, u32::try_from(n).unwrap())
                        .unwrap(),
                    transaction_date: None,
                    raw_date: "2026-06-01".to_owned(),
                    date_confidence_bps: 10_000,
                    amount: Money::new(minor, Currency::Usd),
                    description: Some(merchant.to_owned()),
                    category: category.map(str::to_owned),
                    normalized_merchant: Some(merchant.to_ascii_lowercase()),
                    external_account: None,
                    txn_fingerprint: fingerprint.to_owned(),
                }),
                balance: None,
            })
            .collect(),
        warnings: Vec::new(),
        skipped: Vec::new(),
    }
}

fn account() -> AccountId {
    AccountId::from_uuid(u(1))
}

/// The base vault: the class-1 accounts and category, and the class-1b batch and records of the file,
/// all written through commands so a copy carries them.
fn base_vault(path: &Path) {
    let worker = DbWorker::open(path, KEY).unwrap();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(Account::new(
                    account(),
                    LedgerAccountId::from_uuid(u(101)),
                    "Checking",
                    CashflowRole::LiquidCash,
                    Currency::Usd,
                    AccountFlags::default(),
                )),
                opening_balance: None,
            },
        )
        .unwrap();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateCategory {
                id: CategoryId::from_uuid(u(COFFEE)),
                parent_id: None,
                name: "Coffee Shops".to_owned(),
                category_type: "expense".to_owned(),
                color: None,
                icon: None,
            },
        )
        .unwrap();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateSourceBatch {
                id: SourceBatchId::from_uuid(u(BATCH)),
                source_type: "csv".to_owned(),
                source_name: Some("june.csv".to_owned()),
                file_fingerprint: Some("fp-file".to_owned()),
                parser_version: Some("1.0.0".to_owned()),
                connector_connection_id: None,
            },
        )
        .unwrap();
    for (index, n) in [1_u128, 2, 3, 4, 5].into_iter().enumerate() {
        worker
            .dispatch(
                meta(),
                WriteCommand::AttachSourceRecord {
                    id: SourceRecordId::from_uuid(u(70 + index as u128)),
                    batch_id: SourceBatchId::from_uuid(u(BATCH)),
                    external_id: None,
                    source_hash: source_hash(n),
                    normalized_json: "{}".to_owned(),
                    parse_confidence_bps: Some(10_000),
                },
            )
            .unwrap();
    }
    worker.checkpoint_truncate().unwrap();
}

fn copy_of(base: &Path, to: &Path) -> DbWorker {
    std::fs::copy(base, to).unwrap();
    DbWorker::open(to, KEY).unwrap()
}

/// Every row of the tables the command writes, as text (blobs in hex), for byte comparison.
fn canonical_rows(conn: &Connection) -> String {
    let mut out = String::new();
    for (table, order) in [
        ("ledger_transactions", "id"),
        ("ledger_postings", "transaction_id, ledger_account_id"),
        ("transaction_details", "transaction_id"),
        ("transaction_categorizations", "transaction_id"),
        ("source_provenance_links", "id"),
        ("system_ledger_accounts", "role, currency"),
    ] {
        writeln!(out, "[{table}]").unwrap();
        let sql = format!("SELECT * FROM {table} ORDER BY {order}");
        let mut stmt = conn.prepare(&sql).unwrap();
        let columns = stmt.column_count();
        let lines: Vec<String> = stmt
            .query_map([], |r| {
                let mut cells = Vec::new();
                for c in 0..columns {
                    cells.push(match r.get_ref(c)? {
                        rusqlite::types::ValueRef::Null => "-".to_owned(),
                        rusqlite::types::ValueRef::Integer(n) => n.to_string(),
                        rusqlite::types::ValueRef::Real(f) => f.to_string(),
                        rusqlite::types::ValueRef::Text(t) => {
                            String::from_utf8_lossy(t).into_owned()
                        }
                        rusqlite::types::ValueRef::Blob(b) => {
                            b.iter().map(|byte| format!("{byte:02x}")).collect()
                        }
                    });
                }
                Ok(cells.join(" | "))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        for line in lines {
            writeln!(out, "{line}").unwrap();
        }
    }
    // The committed decisions are about the transaction; the flagged ones name a staging row and stay
    // on the device that flagged them (class 4 until personal-cfo-5ymg8.6).
    writeln!(out, "[dedupe_decisions committed]").unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT id, source_batch_id, layer, staged_transaction_id, decision, reason, decided_at
               FROM dedupe_decisions WHERE decision = 'committed' ORDER BY id",
        )
        .unwrap();
    let lines: Vec<String> = stmt
        .query_map([], |r| {
            Ok(format!(
                "{:?} | {:?} | {} | {:?} | {} | {} | {}",
                r.get::<_, Vec<u8>>(0)?,
                r.get::<_, Vec<u8>>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<Vec<u8>>>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?
            ))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for line in lines {
        writeln!(out, "{line}").unwrap();
    }
    out
}

fn envelope_meta(n: u128) -> CommandMeta {
    CommandMeta {
        command_id: u(1_000 + n),
        correlation_id: u(2_000 + n),
        causation_id: None,
        actor_type: ActorType::Importer,
        actor_id: "tester".to_owned(),
        idempotency_key: format!("00000000-0000-4000-8000-{:012x}", 3_000 + n),
        issued_at: IssuedAt::from_unix_micros(1_000_000_000_000_000 + i64::try_from(n).unwrap()),
    }
}

/// The command the origin logged under `meta`, decoded from its stored payload.
fn logged_command(conn: &Connection, command_id: Uuid) -> Option<WriteCommand> {
    let (kind, payload, version): (String, Vec<u8>, i64) = conn
        .query_row(
            "SELECT operation_type, payload, payload_schema_version FROM operation_log
              WHERE command_id = ?1",
            [command_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .ok()?;
    assert_eq!(kind, "commit_imported_transaction");
    assert_eq!(version, i64::from(PAYLOAD_SCHEMA_VERSION));
    Some(decode_payload(u32::try_from(version).unwrap(), &payload).unwrap())
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

#[test]
fn a_replica_with_no_staging_applies_the_origins_envelopes_to_identical_rows() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base.vault");
    base_vault(&base);
    let origin = copy_of(&base, &dir.path().join("origin.vault"));
    let replica = copy_of(&base, &dir.path().join("replica.vault"));

    // The origin stages the file against the pre-seeded records and decides every row.
    let staged = origin
        .stage_parsed_batch(u(BATCH), &parsed(), account().as_uuid())
        .unwrap();
    assert_eq!(staged.len(), 6);
    let mut envelopes: Vec<(CommandMeta, WriteCommand)> = Vec::new();
    for (index, id) in staged.iter().enumerate() {
        let meta = envelope_meta(index as u128);
        let decision = origin
            .commit_staged(meta.clone(), StagedTransactionId::from_uuid(*id), false)
            .unwrap();
        if matches!(decision, StagedDecision::Committed(_)) {
            let command = logged_command(&origin.read_connection().unwrap(), meta.command_id)
                .expect("a committed row logs its command");
            envelopes.push((meta, command));
        }
    }
    // Row 4 repeats row 3's fingerprint, so it was flagged: one fewer envelope than rows.
    assert_eq!(envelopes.len(), 5, "five rows committed, one was flagged");

    // The envelope names no staging row: its JSON mentions neither a staged id nor a digest.
    for (_, command) in &envelopes {
        let json = serde_json::to_string(command).unwrap();
        assert!(json.contains("commit_imported_transaction"), "{json}");
        assert!(!json.contains("staged"), "{json}");
    }

    // The replica has no staging rows at all, and no transaction yet.
    let replica_conn = replica.read_connection().unwrap();
    assert_eq!(
        count(&replica_conn, "SELECT COUNT(*) FROM staged_transactions"),
        0
    );
    assert_eq!(
        count(&replica_conn, "SELECT COUNT(*) FROM ledger_transactions"),
        0
    );

    for (meta, command) in &envelopes {
        let on_origin = origin.dispatch(meta.clone(), command.clone());
        // Re-applying on the origin is a replay of its own key, never a second write.
        assert!(
            matches!(on_origin, Ok(Outcome::Replayed { .. })),
            "{on_origin:?}"
        );
        let applied = replica.dispatch(meta.clone(), command.clone()).unwrap();
        assert!(matches!(applied, Outcome::Applied { .. }));
        // The command's write set is the same on both devices.
        let origin_hash: Vec<u8> = origin
            .read_connection()
            .unwrap()
            .query_row(
                "SELECT write_set_hash FROM operation_log WHERE command_id = ?1",
                [meta.command_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            replica.last_write_set().unwrap().hash.to_vec(),
            origin_hash,
            "the replica's write set matches the origin's"
        );
    }

    // Byte-identical canonical and provenance rows.
    let origin_rows = canonical_rows(&origin.read_connection().unwrap());
    let replica_rows = canonical_rows(&replica.read_connection().unwrap());
    assert_eq!(origin_rows, replica_rows);
    assert!(origin_rows.contains("[source_provenance_links]"));

    // m8f2's invariant holds on the replica: every committed import posting has a provenance link,
    // and the link names a record that exists.
    let replica_conn = replica.read_connection().unwrap();
    let unlinked = count(
        &replica_conn,
        "SELECT COUNT(*) FROM ledger_transactions lt
          WHERE NOT EXISTS (SELECT 1 FROM source_provenance_links spl
                             WHERE spl.entity_type = 'ledger_transaction' AND spl.entity_id = lt.id)",
    );
    assert_eq!(
        unlinked, 0,
        "every imported transaction has a provenance link"
    );
    let dangling = count(
        &replica_conn,
        "SELECT COUNT(*) FROM source_provenance_links spl
          WHERE NOT EXISTS (SELECT 1 FROM source_records sr WHERE sr.id = spl.source_record_id)",
    );
    assert_eq!(dangling, 0);
    assert_eq!(
        count(&replica_conn, "SELECT COUNT(*) FROM ledger_transactions"),
        5
    );
    // The twin rows (one source hash, two ordinals) are two distinct transactions with one record.
    assert_eq!(
        count(
            &replica_conn,
            "SELECT COUNT(DISTINCT source_record_id) FROM source_provenance_links"
        ),
        4
    );

    // The category the source named travelled as an id and was written as an import alias.
    assert_eq!(
        count(
            &replica_conn,
            "SELECT COUNT(*) FROM transaction_categorizations
              WHERE source = 'import_alias' AND confidence_bps = 5000"
        ),
        1
    );

    // Replay is a no-op: the same envelopes again, as the same commands under new keys (a second
    // delivery), write nothing.
    let before = canonical_rows(&replica_conn);
    for (index, (meta, command)) in envelopes.iter().enumerate() {
        let mut again = envelope_meta(100 + index as u128);
        again.issued_at = IssuedAt::from_unix_micros(2_000_000_000_000_000);
        let outcome = replica.dispatch(again, command.clone()).unwrap();
        assert!(matches!(outcome, Outcome::Applied { .. }), "{meta:?}");
    }
    assert_eq!(canonical_rows(&replica.read_connection().unwrap()), before);
}

#[test]
fn a_crash_between_the_command_and_the_bookkeeping_heals_without_a_second_write() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base.vault");
    base_vault(&base);
    let origin = copy_of(&base, &dir.path().join("origin.vault"));
    let staged = origin
        .stage_parsed_batch(u(BATCH), &parsed(), account().as_uuid())
        .unwrap();

    let first = staged[1];
    origin
        .commit_staged(
            envelope_meta(1),
            StagedTransactionId::from_uuid(first),
            false,
        )
        .unwrap();
    let conn = origin.read_connection().unwrap();
    let ops_before = count(&conn, "SELECT COUNT(*) FROM operation_log");
    let rows_before = canonical_rows(&conn);

    // The crash: the command committed, the staged row was never marked.
    conn.execute(
        "UPDATE staged_transactions
            SET commit_status = 'staged', dedupe_status = 'pending', committed_transaction_id = NULL
          WHERE id = ?1",
        [first],
    )
    .unwrap();

    let decision = origin
        .commit_staged(
            envelope_meta(2),
            StagedTransactionId::from_uuid(first),
            false,
        )
        .unwrap();
    assert_eq!(decision, StagedDecision::AlreadyCommitted);
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM operation_log"),
        ops_before,
        "no second command was dispatched"
    );
    assert_eq!(
        canonical_rows(&conn),
        rows_before,
        "no row was written twice"
    );
    let (status, linked): (String, bool) = conn
        .query_row(
            "SELECT commit_status, committed_transaction_id IS NOT NULL
               FROM staged_transactions WHERE id = ?1",
            [first],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((status.as_str(), linked), ("committed", true));
}

#[test]
fn the_same_row_ordinal_in_another_batch_is_another_transaction() {
    // Two batches, each with a row at ordinal 1, in one vault: four distinct ids, none from a digest.
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base.vault");
    base_vault(&base);
    let origin = copy_of(&base, &dir.path().join("origin.vault"));
    let account_id = account().as_uuid();
    let first = origin
        .stage_parsed_batch(u(BATCH), &parsed(), account_id)
        .unwrap();
    // A re-upload of the same content is a new batch with new records.
    let second_batch = SourceBatchId::from_uuid(u(BATCH + 1));
    origin
        .dispatch(
            meta(),
            WriteCommand::CreateSourceBatch {
                id: second_batch,
                source_type: "csv".to_owned(),
                source_name: Some("june-again.csv".to_owned()),
                file_fingerprint: Some("fp-file-2".to_owned()),
                parser_version: Some("1.0.0".to_owned()),
                connector_connection_id: None,
            },
        )
        .unwrap();
    let second = origin
        .stage_parsed_batch(second_batch.as_uuid(), &parsed(), account_id)
        .unwrap();
    for (index, id) in first.iter().chain(second.iter()).enumerate() {
        // Force: the second batch repeats the first's fingerprints, which would otherwise flag.
        origin
            .commit_staged(
                envelope_meta(200 + index as u128),
                StagedTransactionId::from_uuid(*id),
                true,
            )
            .unwrap();
    }
    let conn = origin.read_connection().unwrap();
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM ledger_transactions"),
        12,
        "twelve rows, twelve transactions: twins and re-uploads stay distinct"
    );
    assert_eq!(
        count(&conn, "SELECT COUNT(DISTINCT id) FROM ledger_transactions"),
        12
    );
    let _ = (DateTime::<Utc>::MIN_UTC,);
}

#[test]
fn a_retry_under_the_same_key_after_a_crash_finishes_the_bookkeeping() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base.vault");
    base_vault(&base);
    let origin = copy_of(&base, &dir.path().join("origin.vault"));
    let staged = origin
        .stage_parsed_batch(u(BATCH), &parsed(), account().as_uuid())
        .unwrap();
    let row = staged[1];
    let meta = envelope_meta(7);
    origin
        .commit_staged(meta.clone(), StagedTransactionId::from_uuid(row), false)
        .unwrap();
    let conn = origin.read_connection().unwrap();
    // The crash: the command committed, the staged row was never marked.
    conn.execute(
        "UPDATE staged_transactions
            SET commit_status = 'staged', dedupe_status = 'pending', committed_transaction_id = NULL
          WHERE id = ?1",
        [row],
    )
    .unwrap();
    let ops = count(&conn, "SELECT COUNT(*) FROM operation_log");
    // The retry carries the same idempotency key, so it is a replay, and it still heals the row.
    let decision = origin
        .commit_staged(meta, StagedTransactionId::from_uuid(row), false)
        .unwrap();
    assert!(matches!(
        decision,
        StagedDecision::Committed(Outcome::Replayed { .. })
    ));
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM operation_log"), ops);
    let status: String = conn
        .query_row(
            "SELECT commit_status FROM staged_transactions WHERE id = ?1",
            [row],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(status, "committed");
}

#[test]
fn a_second_staging_pass_continues_the_numbering_and_a_repeated_ordinal_is_loud() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base.vault");
    base_vault(&base);
    let origin = copy_of(&base, &dir.path().join("origin.vault"));
    let first = origin
        .stage_parsed_batch(u(BATCH), &parsed(), account().as_uuid())
        .unwrap();
    // A second pass over the same batch (new rows) numbers on from the first.
    let mut more = parsed();
    for (index, record) in more.records.iter_mut().enumerate() {
        record.source_hash = format!("sha256:second-pass-{index}");
        if let Some(txn) = record.transaction.as_mut() {
            txn.txn_fingerprint = format!("second-fp-{index}");
        }
    }
    let second = origin
        .stage_parsed_batch(u(BATCH), &more, account().as_uuid())
        .unwrap();
    let conn = origin.read_connection().unwrap();
    let ordinals: Vec<i64> = first
        .iter()
        .chain(second.iter())
        .map(|id| {
            conn.query_row(
                "SELECT row_ordinal FROM staged_transactions WHERE id = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap()
        })
        .collect();
    assert_eq!(ordinals, (1..=12).collect::<Vec<i64>>());

    // A restart of the numbering would give two different source records the same ordinal. The
    // second of them must NOT be treated as already committed.
    origin
        .commit_staged(
            envelope_meta(1),
            StagedTransactionId::from_uuid(first[0]),
            false,
        )
        .unwrap();
    conn.execute(
        "UPDATE staged_transactions SET row_ordinal = 1 WHERE id = ?1",
        [second[0]],
    )
    .unwrap();
    let refused = origin
        .commit_staged(
            envelope_meta(2),
            StagedTransactionId::from_uuid(second[0]),
            true,
        )
        .expect_err("a repeated ordinal on another record is refused");
    assert!(
        matches!(&refused, DbError::InvalidCommand(m) if m.contains("row ordinal")),
        "{refused:?}"
    );
}
