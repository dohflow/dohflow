//! Merchant grouping and merchant memory on one fixed vault (fake data), pinned to a golden file
//! written from the code as it was before the merchant writes moved onto the command bus
//! (personal-cfo-5ymg8.5). Everything the passes write is dumped without timestamps: the seeded and
//! earned merchant identities, their aliases, and every transaction's category with its source and
//! confidence. Regenerate with `MERCHANT_PARITY_UPDATE=1` only when a change is MEANT to move
//! these values.

mod common;

use chrono::{Duration, NaiveDate};
use common::*;
use core_ledger::{AccountId, CashflowRole, CategoryId, TransactionId};
use db_worker::*;
use std::fmt::Write as _;
use uuid::Uuid;

const GOLDEN_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/merchant_memory_parity.golden.txt"
);
const GOLDEN: &str = include_str!("merchant_memory_parity.golden.txt");

/// The one place the test runs the merchant passes, so a change of entry point edits only this.
fn run_merchant_memory(worker: &DbWorker) -> u32 {
    worker.apply_merchant_memory().unwrap()
}

fn category_ids(worker: &DbWorker) -> (CategoryId, CategoryId) {
    let conn = worker.read_connection().unwrap();
    let mut stmt = conn
        .prepare("SELECT id FROM categories ORDER BY name LIMIT 2")
        .unwrap();
    let ids: Vec<Uuid> = stmt
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    (CategoryId::from_uuid(ids[0]), CategoryId::from_uuid(ids[1]))
}

struct Planter<'a> {
    worker: &'a DbWorker,
    account: AccountId,
    next: NaiveDate,
}

impl Planter<'_> {
    /// A spend with `counterparty` on its own date, categorized by the user when `category` is set.
    fn plant(&mut self, counterparty: &str, category: Option<CategoryId>) -> TransactionId {
        self.next += Duration::days(1);
        let txn =
            record_with_counterparty(self.worker, self.account, -2_500, self.next, counterparty);
        if let Some(category) = category {
            self.worker
                .dispatch(
                    meta(),
                    WriteCommand::RecategorizeTransaction {
                        transaction_id: txn,
                        category_id: Some(category),
                    },
                )
                .unwrap();
        }
        txn
    }
}

/// Everything the passes can write, as stable text (no ids, no timestamps).
fn dump(worker: &DbWorker, applied: u32) -> String {
    let conn = worker.read_connection().unwrap();
    let mut out = String::new();
    writeln!(out, "applied {applied}").unwrap();

    out.push_str("[identities]\n");
    let mut stmt = conn
        .prepare(
            "SELECT display_name, source, is_ambiguous, default_category_id IS NOT NULL
               FROM merchant_identities ORDER BY display_name, source",
        )
        .unwrap();
    for row in stmt
        .query_map([], |r| {
            Ok(format!(
                "{} | {} | ambiguous {} | default category {}",
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?
            ))
        })
        .unwrap()
    {
        writeln!(out, "{}", row.unwrap()).unwrap();
    }

    out.push_str("[aliases]\n");
    let mut stmt = conn
        .prepare(
            "SELECT ma.normalized_key, mi.display_name, ma.source, ma.confidence_bps, ma.match_type
               FROM merchant_aliases ma
               JOIN merchant_identities mi ON mi.id = ma.merchant_identity_id
              ORDER BY ma.normalized_key",
        )
        .unwrap();
    for row in stmt
        .query_map([], |r| {
            Ok(format!(
                "{} -> {} | {} | {} bps | {}",
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, String>(4)?
            ))
        })
        .unwrap()
    {
        writeln!(out, "{}", row.unwrap()).unwrap();
    }

    out.push_str("[categorizations]\n");
    let mut stmt = conn
        .prepare(
            "SELECT lp.posting_date, td.counterparty, c.name, tc.source, tc.confidence_bps
               FROM ledger_transactions lt
               JOIN ledger_postings lp ON lp.transaction_id = lt.id
               JOIN accounts a ON a.ledger_account_id = lp.ledger_account_id
               JOIN transaction_details td ON td.transaction_id = lt.id
               LEFT JOIN transaction_categorizations tc ON tc.transaction_id = lt.id
               LEFT JOIN categories c ON c.id = tc.category_id
              WHERE a.name = 'Checking'
              ORDER BY lp.posting_date",
        )
        .unwrap();
    for row in stmt
        .query_map([], |r| {
            Ok(format!(
                "{} | {} | {} | {} | {}",
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?
                    .unwrap_or_else(|| "-".to_owned()),
                r.get::<_, Option<String>>(3)?
                    .unwrap_or_else(|| "-".to_owned()),
                r.get::<_, Option<i64>>(4)?
                    .map_or("-".to_owned(), |b| b.to_string()),
            ))
        })
        .unwrap()
    {
        writeln!(out, "{}", row.unwrap()).unwrap();
    }
    out
}

fn fixture() -> (tempfile::TempDir, DbWorker) {
    let (dir, worker) = worker();
    let checking = AccountId::new();
    create_role_account(&worker, checking, "Checking", CashflowRole::LiquidCash);
    let (a, b) = category_ids(&worker);
    let mut p = Planter {
        worker: &worker,
        account: checking,
        next: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
    };

    // A merchant the user always puts in A: the uncategorized visits are filled at 100%.
    for _ in 0..3 {
        p.plant("CORNER CAFE", Some(a));
    }
    p.plant("CORNER CAFE", None);
    p.plant("CORNER CAFE", None);

    // A conflicted merchant at exactly 60% (3 of 5): still applied, at 6000 bps.
    for _ in 0..3 {
        p.plant("MARKET HALL", Some(a));
    }
    for _ in 0..2 {
        p.plant("MARKET HALL", Some(b));
    }
    p.plant("MARKET HALL", None);

    // A merchant at 50%: below the threshold, left for review.
    p.plant("DELI COUNTER", Some(a));
    p.plant("DELI COUNTER", Some(b));
    p.plant("DELI COUNTER", None);

    // A chain seen at two locations with no curated seed: minted as an earned brand, then the
    // category learned at one location fills the other.
    p.plant("RITUAL COFFEE SF", Some(b));
    p.plant("RITUAL COFFEE SF", Some(b));
    p.plant("RITUAL COFFEE OAKLAND", None);

    // A seeded ambiguous merchant: its category is never propagated.
    for _ in 0..3 {
        p.plant("AMAZON.COM", Some(a));
    }
    p.plant("AMZN MKTP US", None);

    // A merchant the user mapped themselves (a raw insert; the user-correction API is not part of
    // what this pins): the earned-brand pass must leave their mapping alone.
    {
        let conn = worker.read_connection().unwrap();
        let contoso = Uuid::from_u128(0xC0);
        conn.execute(
            "INSERT INTO merchant_identities
                 (id, display_name, default_category_id, is_ambiguous, source, created_at)
             VALUES (?1, 'Contoso', NULL, 0, 'user', '2026-01-01T00:00:00+00:00')",
            [contoso],
        )
        .unwrap();
        for raw in ["CONTOSO HQ", "CONTOSO STORE 42"] {
            let key = categorization::normalize_merchant(raw);
            conn.execute(
                "INSERT INTO merchant_aliases
                     (normalized_key, merchant_identity_id, source, confidence_bps, created_at, match_type)
                 VALUES (?1, ?2, 'user', 10000, '2026-01-01T00:00:00+00:00', 'exact')",
                rusqlite::params![key, contoso],
            )
            .unwrap();
        }
    }
    p.plant("CONTOSO HQ", Some(b));
    p.plant("CONTOSO HQ", Some(b));
    p.plant("CONTOSO STORE 42", None);

    (dir, worker)
}

#[test]
fn merchant_grouping_and_memory_write_what_they_wrote_before_they_moved_onto_the_bus() {
    let (_dir, worker) = fixture();
    let applied = run_merchant_memory(&worker);
    let text = dump(&worker, applied);

    // The fixture must exercise what it claims to: a vacuous dump proves nothing.
    assert!(
        text.contains("Ritual Coffee | auto"),
        "an earned brand is minted:\n{text}"
    );
    assert!(
        text.contains("CORNER CAFE | ") && text.contains("| rule | 10000"),
        "a unanimous merchant fills at 100%:\n{text}"
    );
    assert!(
        text.contains("| rule | 6000"),
        "a 60% merchant still applies:\n{text}"
    );
    assert!(applied > 0, "the pass applied something");
    assert!(
        text.contains("CONTOSO STORE 42 | Auto Insurance | rule | 10000"),
        "a user-mapped merchant's other alias is filled through the identity:\n{text}"
    );
    assert!(
        text.contains("AMZN MKTP US | - | - | -"),
        "an ambiguous merchant is not filled"
    );

    // A second pass finds nothing new (idempotent).
    assert_eq!(
        run_merchant_memory(&worker),
        0,
        "a re-run categorizes nothing"
    );
    assert_eq!(
        dump(&worker, 0).replacen("applied 0", &format!("applied {applied}"), 1),
        text
    );

    if std::env::var_os("MERCHANT_PARITY_UPDATE").is_some() {
        std::fs::write(GOLDEN_PATH, &text).unwrap();
        return;
    }
    assert_eq!(
        text, GOLDEN,
        "merchant grouping or memory wrote different values; if that is intended, regenerate \
         with MERCHANT_PARITY_UPDATE=1 and review the diff"
    );
}
