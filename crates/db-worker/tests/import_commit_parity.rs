//! The import commit path on one fixed vault (fake data), pinned to a golden file written from the
//! code as it was BEFORE the commit was materialized and made replicable (personal-cfo-0g528.1).
//!
//! Three entry paths (a single-account file, a routed file, a connector sync) and the two Money
//! Inbox resolutions (import anyway, skip) are driven through the same sequence the kernel runs:
//! stage, decide each staged row, tally, advance the batch. Everything they leave behind is dumped
//! without ids or timestamps: the ledger transactions with their detail, category and provenance,
//! every dedupe decision, every staged row's status, the batch counts and the Money Inbox items. A
//! change of entry point edits only [`commit`] and [`skip`]. Regenerate with
//! `IMPORT_PARITY_UPDATE=1` only when a change is MEANT to move these values.

mod common;

use chrono::NaiveDate;
use common::*;
use core_ledger::{AccountId, CashflowRole, SourceBatchId, StagedTransactionId};
use core_money::{Currency, Money};
use db_worker::*;
use importer_core::{ParsedAccount, ParsedBatch, ParsedRecord, ParsedTransaction};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use uuid::Uuid;

const GOLDEN_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/import_commit_parity.golden.txt"
);
const GOLDEN: &str = include_str!("import_commit_parity.golden.txt");

/// The one place the test decides a staged row, so a change of entry point edits only this. The
/// Money Inbox is rebuilt by the caller once a batch (or a resolution) is done.
fn commit(worker: &DbWorker, staged: Uuid, force: bool) -> Result<(), DbError> {
    worker
        .commit_staged(meta(), StagedTransactionId::from_uuid(staged), force)
        .map(|_| ())
}

/// The Money Inbox "skip" resolution (and the silent skip of a certain connector duplicate).
fn skip(worker: &DbWorker, staged: Uuid) -> Result<(), DbError> {
    worker.skip_staged(StagedTransactionId::from_uuid(staged))
}

fn date(month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, month, day).unwrap()
}

/// One row. `source_hash` is unique per `n` unless the caller repeats it; `fingerprint` is the
/// dedupe key.
#[derive(Clone)]
struct Row {
    n: u32,
    fingerprint: String,
    when: NaiveDate,
    minor: i64,
    currency: Currency,
    merchant: String,
    category: Option<&'static str>,
    transaction_date: Option<NaiveDate>,
    external_account: Option<&'static str>,
    source_hash: Option<String>,
}

fn row(n: u32, fingerprint: &str, when: NaiveDate, minor: i64, merchant: &str) -> Row {
    Row {
        n,
        fingerprint: fingerprint.to_owned(),
        when,
        minor,
        currency: Currency::Usd,
        merchant: merchant.to_owned(),
        category: None,
        transaction_date: None,
        external_account: None,
        source_hash: None,
    }
}

fn parsed(source_format: &str, rows: &[Row]) -> ParsedBatch {
    let mut accounts: Vec<ParsedAccount> = Vec::new();
    for label in rows.iter().filter_map(|r| r.external_account) {
        if !accounts
            .iter()
            .any(|a| a.external_id.as_deref() == Some(label))
        {
            accounts.push(ParsedAccount {
                external_id: Some(label.to_owned()),
                external_name: Some(label.to_owned()),
                external_number_hash: None,
                proposed_subtype: None,
                currency: None,
            });
        }
    }
    ParsedBatch {
        source_format: source_format.to_owned(),
        accounts,
        records: rows
            .iter()
            .map(|r| ParsedRecord {
                external_id: (source_format == "simplefin").then(|| format!("prov-{}", r.n)),
                source_hash: r
                    .source_hash
                    .clone()
                    .unwrap_or_else(|| format!("{source_format}-{}", r.n)),
                normalized_json: "{}".to_owned(),
                parse_confidence_bps: Some(10_000),
                transaction: Some(ParsedTransaction {
                    posted_date: r.when,
                    transaction_date: r.transaction_date,
                    raw_date: r.when.to_string(),
                    date_confidence_bps: 10_000,
                    amount: Money::new(r.minor, r.currency),
                    description: Some(r.merchant.clone()),
                    category: r.category.map(str::to_owned),
                    normalized_merchant: Some(r.merchant.to_ascii_lowercase()),
                    external_account: r.external_account.map(str::to_owned),
                    txn_fingerprint: r.fingerprint.clone(),
                }),
                balance: None,
            })
            .collect(),
        warnings: Vec::new(),
        skipped: Vec::new(),
    }
}

fn open_batch(
    worker: &DbWorker,
    source_type: &str,
    name: &str,
    fingerprint: Option<&str>,
    connection: Option<Uuid>,
) -> SourceBatchId {
    let id = SourceBatchId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateSourceBatch {
                id,
                source_type: source_type.to_owned(),
                source_name: Some(name.to_owned()),
                file_fingerprint: fingerprint.map(str::to_owned),
                parser_version: Some("1.0.0".to_owned()),
                connector_connection_id: connection,
            },
        )
        .unwrap();
    id
}

fn finish_batch(worker: &DbWorker, batch: SourceBatchId) {
    let (total, committed, flagged) = worker.staged_commit_tally(batch.as_uuid()).unwrap();
    worker
        .dispatch(
            meta(),
            WriteCommand::UpdateBatchState {
                batch_id: batch,
                status: if flagged == 0 {
                    "committed"
                } else {
                    "partially_committed"
                }
                .to_owned(),
                staged_count: i64::from(total),
                committed_count: i64::from(committed),
                skipped_count: i64::from(flagged),
            },
        )
        .unwrap();
    worker.rebuild_money_inbox().unwrap();
}

/// A file import into one account, as the kernel runs it: file-level dedupe, then stage and decide
/// every row. Returns `false` for an exact re-upload.
fn import_file(worker: &DbWorker, name: &str, format: &str, rows: &[Row], to: AccountId) -> bool {
    if worker.batch_with_fingerprint(name).unwrap().is_some() {
        return false;
    }
    let batch = open_batch(worker, format, name, Some(name), None);
    let staged = worker
        .stage_parsed_batch(batch.as_uuid(), &parsed(format, rows), to.as_uuid())
        .unwrap();
    for id in staged {
        commit(worker, id, false).unwrap();
    }
    finish_batch(worker, batch);
    true
}

/// A routed file import (rows carry a source account label).
fn import_routed(
    worker: &DbWorker,
    name: &str,
    rows: &[Row],
    map: &BTreeMap<String, Uuid>,
) -> usize {
    let batch = open_batch(worker, "csv", name, Some(name), None);
    let (staged, not_routed) = worker
        .stage_parsed_batch_routed(batch.as_uuid(), &parsed("csv", rows), map)
        .unwrap();
    for id in staged {
        commit(worker, id, false).unwrap();
    }
    finish_batch(worker, batch);
    not_routed
}

/// A connector sync, as the kernel runs it: a certain duplicate is skipped silently, the rest are
/// decided.
fn sync(
    worker: &DbWorker,
    name: &str,
    connection: Option<Uuid>,
    rows: &[Row],
    map: &BTreeMap<String, Uuid>,
) {
    let batch = open_batch(worker, "simplefin", name, None, connection);
    let (staged, _unmapped) = worker
        .stage_sync_batch(batch.as_uuid(), &parsed("simplefin", rows), map)
        .unwrap();
    for staged in staged {
        if worker
            .txn_fingerprint_already_tracked(
                &staged.txn_fingerprint,
                staged.account_id,
                staged.staged_id,
                connection,
            )
            .unwrap()
        {
            skip(worker, staged.staged_id).unwrap();
        } else {
            commit(worker, staged.staged_id, false).unwrap();
        }
    }
    finish_batch(worker, batch);
}

fn waiting(worker: &DbWorker) -> Vec<MoneyInboxItem> {
    let mut items: Vec<MoneyInboxItem> = worker
        .money_inbox_list()
        .unwrap()
        .into_iter()
        .filter(|item| item.item_kind == "imported_waiting_commit")
        .collect();
    // Surfacing order is a timestamp; the dump sorts by content instead.
    items.sort_by(|a, b| a.payload_json.cmp(&b.payload_json));
    items
}

/// The waiting item whose payload mentions `needle`.
fn waiting_with(worker: &DbWorker, needle: &str) -> Uuid {
    let found: Vec<Uuid> = waiting(worker)
        .into_iter()
        .filter(|item| item.payload_json.contains(needle))
        .map(|item| item.target_id)
        .collect();
    assert_eq!(found.len(), 1, "exactly one waiting item mentions {needle}");
    found[0]
}

/// UUIDs and RFC 3339 instants in a payload are replaced with fixed tokens.
fn scrub(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    let is_uuid = |s: &[u8]| {
        s.len() == 36
            && s.iter().enumerate().all(|(k, c)| {
                if [8, 13, 18, 23].contains(&k) {
                    *c == b'-'
                } else {
                    c.is_ascii_hexdigit()
                }
            })
    };
    while i < bytes.len() {
        if i + 36 <= bytes.len() && is_uuid(&bytes[i..i + 36]) {
            out.push_str("<id>");
            i += 36;
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

fn fixture() -> (tempfile::TempDir, DbWorker, Fixture) {
    let (dir, worker) = worker();
    let checking = AccountId::new();
    let savings = AccountId::new();
    create_role_account(&worker, checking, "Checking", CashflowRole::LiquidCash);
    create_role_account(&worker, savings, "Savings", CashflowRole::LiquidCash);
    (dir, worker, Fixture { checking, savings })
}

struct Fixture {
    checking: AccountId,
    savings: AccountId,
}

/// Every table the commit path writes, as stable text.
fn dump(worker: &DbWorker) -> String {
    let conn = worker.read_connection().unwrap();
    let mut out = String::new();
    let mut section = |title: &str, sql: &str, columns: usize| {
        writeln!(out, "[{title}]").unwrap();
        let mut stmt = conn.prepare(sql).unwrap();
        let rows: Vec<String> = stmt
            .query_map([], |r| {
                let mut cells = Vec::new();
                for c in 0..columns {
                    cells.push(match r.get_ref(c)? {
                        rusqlite::types::ValueRef::Null => "-".to_owned(),
                        rusqlite::types::ValueRef::Integer(n) => n.to_string(),
                        rusqlite::types::ValueRef::Text(t) => {
                            String::from_utf8_lossy(t).into_owned()
                        }
                        rusqlite::types::ValueRef::Real(f) => f.to_string(),
                        rusqlite::types::ValueRef::Blob(_) => "<blob>".to_owned(),
                    });
                }
                Ok(cells.join(" | "))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        for line in rows {
            writeln!(out, "{line}").unwrap();
        }
    };

    section(
        "transactions",
        "SELECT a.name, lp.posting_date, lp.minor_units, lp.currency,
                lt.voided_at IS NOT NULL, td.memo, td.counterparty, td.transaction_date,
                c.name, tc.source, tc.confidence_bps,
                sb.source_name, sr.source_hash, spl.relationship
           FROM ledger_postings lp
           JOIN ledger_transactions lt ON lt.id = lp.transaction_id
           JOIN accounts a ON a.ledger_account_id = lp.ledger_account_id
           LEFT JOIN transaction_details td ON td.transaction_id = lt.id
           LEFT JOIN transaction_categorizations tc ON tc.transaction_id = lt.id
           LEFT JOIN categories c ON c.id = tc.category_id
           LEFT JOIN source_provenance_links spl
                  ON spl.entity_type = 'ledger_transaction' AND spl.entity_id = lt.id
           LEFT JOIN source_records sr ON sr.id = spl.source_record_id
           LEFT JOIN source_batches sb ON sb.id = sr.source_batch_id
          ORDER BY a.name, lp.posting_date, lp.minor_units, td.memo, sb.source_name, sr.source_hash",
        14,
    );
    section(
        "provenance_links",
        "SELECT spl.entity_type, spl.relationship, sb.source_name, sr.source_hash,
                (SELECT COUNT(*) FROM ledger_transactions lt WHERE lt.id = spl.entity_id)
           FROM source_provenance_links spl
           JOIN source_records sr ON sr.id = spl.source_record_id
           JOIN source_batches sb ON sb.id = sr.source_batch_id
          ORDER BY sb.source_name, sr.source_hash, spl.relationship",
        5,
    );
    section(
        "dedupe_decisions",
        "SELECT sb.source_name, dd.layer, dd.decision, dd.reason, dd.matched_entity_type,
                (SELECT td.memo FROM transaction_details td WHERE td.transaction_id = dd.matched_entity_id)
           FROM dedupe_decisions dd
           JOIN source_batches sb ON sb.id = dd.source_batch_id
          ORDER BY sb.source_name, dd.decision, dd.reason, 6",
        6,
    );
    // A flag is the staged row's own reason and counterpart (class 3), since a flagged decision is not
    // a row of the class-1 decisions table.
    section(
        "staged flags",
        "SELECT sb.source_name, st.commit_status, st.flag_reason,
                (SELECT td.memo FROM transaction_details td
                  WHERE td.transaction_id = st.flag_matched_transaction_id)
           FROM staged_transactions st
           JOIN source_records sr ON sr.id = st.source_record_id
           JOIN source_batches sb ON sb.id = sr.source_batch_id
          WHERE st.flag_reason IS NOT NULL
          ORDER BY sb.source_name, st.flag_reason, 4",
        4,
    );
    section(
        "staged_transactions",
        "SELECT sb.source_name, sr.source_hash, st.description, st.commit_status,
                st.dedupe_status, st.committed_transaction_id IS NOT NULL
           FROM staged_transactions st
           JOIN source_records sr ON sr.id = st.source_record_id
           JOIN source_batches sb ON sb.id = sr.source_batch_id
          ORDER BY sb.source_name, sr.source_hash, st.description, st.commit_status",
        6,
    );
    section(
        "batches",
        "SELECT source_name, source_type, status, staged_count, committed_count, skipped_count
           FROM source_batches ORDER BY source_name",
        6,
    );
    writeln!(out, "[money_inbox]").unwrap();
    for item in waiting(worker) {
        writeln!(
            out,
            "{} | {} | {}",
            item.item_kind,
            item.target_table,
            scrub(&item.payload_json)
        )
        .unwrap();
    }
    out
}

/// One long scenario over three entry paths and both inbox resolutions.
fn run_scenario(worker: &DbWorker, fx: &Fixture) {
    let groceries = category_names(worker)
        .into_iter()
        .next()
        .expect("a seeded category");

    // 1. A single-account file: a category from the source, a secondary date, a repeated
    //    fingerprint inside the file (flagged), two rows that share one source hash (identical
    //    content), and a negative and a positive amount.
    let mut with_category = row(1, "fp-cafe", date(6, 1), -450, "Corner Cafe");
    with_category.category = Some(Box::leak(groceries.clone().into_boxed_str()));
    with_category.transaction_date = Some(date(5, 30));
    let mut first_twin = row(4, "fp-twin-a", date(6, 4), -700, "Twin Shop");
    first_twin.source_hash = Some("csv-twin".to_owned());
    let mut second_twin = row(5, "fp-twin-b", date(6, 4), -700, "Twin Shop");
    second_twin.source_hash = Some("csv-twin".to_owned());
    let file_rows = vec![
        with_category,
        row(2, "fp-rent", date(6, 2), -120_000, "Landlord"),
        row(3, "fp-cafe", date(6, 1), -450, "Corner Cafe"),
        first_twin,
        second_twin,
        row(6, "fp-pay", date(6, 5), 250_000, "Employer"),
    ];
    assert!(import_file(
        worker,
        "statement-june.csv",
        "csv",
        &file_rows,
        fx.checking
    ));
    // An exact re-upload is file-deduped.
    assert!(!import_file(
        worker,
        "statement-june.csv",
        "csv",
        &file_rows,
        fx.checking
    ));

    // 2. A cross-source overlap: the same date and amount from OFX, with a different fingerprint.
    import_file(
        worker,
        "statement-june.ofx",
        "ofx",
        &[
            row(1, "ofx-cafe", date(6, 1), -450, "CORNER CAFE #12"),
            row(2, "ofx-fresh", date(6, 6), -1_000, "Fresh Rows"),
        ],
        fx.checking,
    );

    // 3. A routed file: two accounts, an unmapped one (counted, not staged).
    let mut to_savings = row(1, "fp-save", date(6, 7), 5_000, "Transfer In");
    to_savings.external_account = Some("Savings");
    let mut to_checking = row(2, "fp-chk", date(6, 7), -300, "Newsstand");
    to_checking.external_account = Some("Checking");
    let mut elsewhere = row(3, "fp-else", date(6, 7), -999, "Elsewhere");
    elsewhere.external_account = Some("Brokerage");
    let map = BTreeMap::from([
        ("Checking".to_owned(), fx.checking.as_uuid()),
        ("Savings".to_owned(), fx.savings.as_uuid()),
    ]);
    assert_eq!(
        import_routed(
            worker,
            "multi.csv",
            &[to_savings, to_checking, elsewhere],
            &map
        ),
        1
    );

    // 4. Rows the ledger cannot accept: a zero amount and a currency other than the account's.
    let mut euro = row(2, "fp-euro", date(6, 8), -2_500, "Euro Shop");
    euro.currency = Currency::Eur;
    import_file(
        worker,
        "odd-rows.csv",
        "csv",
        &[
            row(1, "fp-zero", date(6, 8), 0, "Zero Row"),
            euro,
            row(3, "fp-after", date(6, 9), -1_100, "After The Bad Rows"),
        ],
        fx.checking,
    );

    // 5. A connector sync, then the same sync again (a rewind re-fetch is skipped silently), plus a
    //    row that overlaps the file import on account, date and amount (flagged cross-source).
    let connection = Some(Uuid::now_v7());
    let mut provider = vec![
        row(1, "prov-1", date(6, 10), -4_200, "Provider Coffee"),
        row(2, "prov-2", date(6, 10), -4_200, "Provider Coffee"),
        row(3, "prov-3", date(6, 5), 250_000, "Payroll Direct"),
    ];
    for r in &mut provider {
        r.external_account = Some("acct-1");
    }
    let provider_map = BTreeMap::from([("acct-1".to_owned(), fx.checking.as_uuid())]);
    sync(worker, "sync-1", connection, &provider, &provider_map);
    sync(worker, "sync-2", connection, &provider, &provider_map);

    // 6. Money Inbox resolutions: import one flagged duplicate anyway, skip another, then refuse
    //    both resolutions on a row that is already committed.
    let flagged_cafe = waiting_with(worker, "Corner Cafe");
    commit(worker, flagged_cafe, true).unwrap();
    worker.rebuild_money_inbox().unwrap();
    let flagged_ofx = waiting_with(worker, "CORNER CAFE #12");
    skip(worker, flagged_ofx).unwrap();
    worker.rebuild_money_inbox().unwrap();
    assert!(commit(worker, flagged_cafe, true).is_err());
    assert!(skip(worker, flagged_cafe).is_err());
    // A forced commit of an invalid row is refused and changes nothing.
    let zero = waiting_with(worker, "Zero Row");
    assert!(commit(worker, zero, true).is_err());

    // 7. Voiding an imported transaction frees its fingerprint: the same row can be imported again.
    let voided = worker
        .recent_transactions(100)
        .unwrap()
        .into_iter()
        .find(|t| t.amount.minor_units() == -1_100)
        .expect("the row after the bad rows committed")
        .transaction_id;
    worker
        .dispatch(
            meta(),
            WriteCommand::VoidTransaction {
                transaction_id: voided,
            },
        )
        .unwrap();
    import_file(
        worker,
        "restore.csv",
        "csv",
        &[row(1, "fp-after", date(6, 9), -1_100, "After The Bad Rows")],
        fx.checking,
    );
}

fn category_names(worker: &DbWorker) -> Vec<String> {
    let conn = worker.read_connection().unwrap();
    let mut stmt = conn
        .prepare("SELECT name FROM categories ORDER BY name LIMIT 1")
        .unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn the_import_commit_path_writes_what_it_wrote_before_it_was_materialized() {
    let (_dir, worker, fx) = fixture();
    run_scenario(&worker, &fx);
    let text = dump(&worker);

    // The fixture must exercise what it claims to: a vacuous dump proves nothing.
    for needle in [
        "| import_alias | 5000 |",
        "duplicate of an already-committed transaction",
        "same date and amount already recorded from another source",
        "not imported: the amount is zero",
        "not imported: its currency differs from the account's currency",
        "user import-anyway override",
        "| skipped |",
        "| created_from |",
        "partially_committed",
    ] {
        assert!(
            text.contains(needle),
            "the scenario never produced {needle}:\n{text}"
        );
    }

    if std::env::var_os("IMPORT_PARITY_UPDATE").is_some() {
        std::fs::write(GOLDEN_PATH, &text).unwrap();
        return;
    }
    assert_eq!(
        text, GOLDEN,
        "the import commit path wrote different values; if that is intended, regenerate with \
         IMPORT_PARITY_UPDATE=1 and review the diff"
    );
}
