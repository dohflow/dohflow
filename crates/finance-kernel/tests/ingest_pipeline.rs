//! End-to-end ingestion pipeline (personal-cfo-cmx). Drives a real `Kernel`
//! (temp vault) through `ingest_batch` with a test `ImporterPlugin`: parse in the
//! bounded host → stage → dedupe → commit the clean rows → advance batch state.
//! Asserts the unique transactions hit the ledger, the duplicate is flagged (not
//! posted), and an exact re-upload is file-deduped. The first importer (`cu8`) and
//! the IPC entry point plug into this same `ingest_batch` path.

use chrono::NaiveDate;
use finance_kernel::{
    Account, AccountFlags, AccountId, ActorType, CashflowRole, CommandEnvelope, CommandMeta,
    CreateAccount, Currency, ImporterPlugin, Kernel, LedgerAccountId, Money, ParseError,
    ParsedBatch, ParsedRecord, ParsedTransaction, ParserHints, ParserInput, ParserLimits,
    RecategorizeTransaction,
};
use semver::Version;
use uuid::Uuid;

const PW: &[u8] = b"ingest pipeline correct horse";

fn meta() -> CommandMeta {
    CommandMeta {
        command_id: Uuid::now_v7(),
        correlation_id: Uuid::now_v7(),
        causation_id: None,
        actor_type: ActorType::User,
        actor_id: "tester".to_owned(),
        idempotency_key: Uuid::now_v7().to_string(),
    }
}

fn account(name: &str) -> Account {
    Account::new(
        AccountId::new(),
        LedgerAccountId::new(),
        name,
        CashflowRole::LiquidCash,
        Currency::Usd,
        AccountFlags::default(),
    )
}

/// One parsed record carrying a transaction. `source_hash` is unique per row (so
/// all rows become distinct source_records); `fingerprint` is the dedupe key.
fn record(row: usize, fingerprint: &str, amount_minor: i64, merchant: &str) -> ParsedRecord {
    ParsedRecord {
        external_id: None,
        source_hash: format!("row-{row}"),
        normalized_json: "{}".to_owned(),
        parse_confidence_bps: Some(10_000),
        transaction: Some(ParsedTransaction {
            posted_date: NaiveDate::from_ymd_opt(2026, 6, 20).unwrap(),
            transaction_date: None,
            raw_date: "2026-06-20".to_owned(),
            date_confidence_bps: 10_000,
            amount: Money::new(amount_minor, Currency::Usd),
            description: Some(merchant.to_owned()),
            category: None,
            normalized_merchant: Some(merchant.to_ascii_lowercase()),
            external_account: None,
            txn_fingerprint: fingerprint.to_owned(),
        }),
        balance: None,
    }
}

/// A test importer producing three transactions — the third duplicates the first
/// (same fingerprint), so it is flagged rather than committed.
struct ThreeRowCsv;

impl ImporterPlugin for ThreeRowCsv {
    fn id(&self) -> &'static str {
        "three-row-csv"
    }
    fn display_name(&self) -> &'static str {
        "Three Row CSV"
    }
    fn version(&self) -> Version {
        Version::new(1, 0, 0)
    }
    fn supported_extensions(&self) -> &'static [&'static str] {
        &["csv"]
    }
    fn detect_confidence(&self, _: &ParserInput) -> u16 {
        10_000
    }
    fn parse(&self, _: &ParserInput, _: &ParserHints) -> Result<ParsedBatch, ParseError> {
        Ok(ParsedBatch {
            source_format: "csv".to_owned(),
            accounts: vec![],
            records: vec![
                record(0, "fp-a", -1299, "Coffee"),
                record(1, "fp-b", -4200, "Lunch"),
                record(2, "fp-a", -1299, "Coffee"), // duplicate of row 0
            ],
            warnings: vec![],
        })
    }
}

fn csv_input() -> ParserInput {
    ParserInput::new(b"date,amount\n2026-06-20,-12.99\n".to_vec()).with_filename("statement.csv")
}

/// A test importer producing a single transaction at the given `merchant` +
/// `fingerprint`. Drives the auto-apply-on-import test (personal-cfo-5n4.2).
struct OneRowCsv {
    fingerprint: &'static str,
    merchant: &'static str,
}

impl ImporterPlugin for OneRowCsv {
    fn id(&self) -> &'static str {
        "one-row-csv"
    }
    fn display_name(&self) -> &'static str {
        "One Row CSV"
    }
    fn version(&self) -> Version {
        Version::new(1, 0, 0)
    }
    fn supported_extensions(&self) -> &'static [&'static str] {
        &["csv"]
    }
    fn detect_confidence(&self, _: &ParserInput) -> u16 {
        10_000
    }
    fn parse(&self, _: &ParserInput, _: &ParserHints) -> Result<ParsedBatch, ParseError> {
        Ok(ParsedBatch {
            source_format: "csv".to_owned(),
            accounts: vec![],
            records: vec![record(0, self.fingerprint, -1299, self.merchant)],
            warnings: vec![],
        })
    }
}

/// Distinct bytes per phase so each import is a new file (file-dedupe is by content).
fn tagged_input(tag: &str) -> ParserInput {
    ParserInput::new(format!("date,amount,tag\n2026-06-20,-12.99,{tag}\n").into_bytes())
        .with_filename("statement.csv")
}

/// Auto-apply merchant memory after an import (ADR 0030 addendum, personal-cfo-5n4.2):
/// once the user has categorized a merchant, a later import of the same merchant is
/// auto-categorized (`source=rule`) when the setting is on, and left uncategorized when
/// off.
#[test]
fn ingest_batch_auto_categorizes_from_merchant_memory_when_enabled() {
    let dir = tempfile::tempdir().unwrap();
    let kernel = Kernel::create_vault(dir.path().join("vault.db"), PW).unwrap();
    let checking = account("Checking");
    let account_id = checking.id();
    kernel
        .dispatch(CommandEnvelope::new(meta(), CreateAccount::new(checking)))
        .unwrap();

    // Default ON. Phase 1: import one Coffee. No memory yet, so nothing auto-applies.
    let first = kernel
        .ingest_batch(
            &OneRowCsv {
                fingerprint: "fp-1",
                merchant: "Coffee",
            },
            tagged_input("a"),
            &ParserHints::default(),
            account_id,
            &ParserLimits::default(),
            &meta(),
        )
        .unwrap();
    assert_eq!(first.committed, 1);
    assert_eq!(first.auto_categorized, 0, "no learned merchant yet");

    // The user categorizes that Coffee — the training signal (source=user).
    let category = kernel
        .category_views()
        .unwrap()
        .into_iter()
        .find(|c| c.category_type == "expense" && !c.archived)
        .expect("a seeded expense category");
    let coffee_id = kernel.transactions(50).unwrap()[0].transaction_id;
    kernel
        .dispatch(CommandEnvelope::new(
            meta(),
            RecategorizeTransaction::new(coffee_id, Some(category.id)),
        ))
        .unwrap();

    // Phase 2: import another Coffee. On commit it is auto-categorized as source=rule.
    let second = kernel
        .ingest_batch(
            &OneRowCsv {
                fingerprint: "fp-2",
                merchant: "Coffee",
            },
            tagged_input("b"),
            &ParserHints::default(),
            account_id,
            &ParserLimits::default(),
            &meta(),
        )
        .unwrap();
    assert_eq!(second.committed, 1);
    assert_eq!(
        second.auto_categorized, 1,
        "the new Coffee is filled from memory"
    );
    let rule_rows = kernel
        .transactions(50)
        .unwrap()
        .into_iter()
        .filter(|r| r.category_source.as_deref() == Some("rule"))
        .count();
    assert_eq!(
        rule_rows, 1,
        "exactly the freshly imported Coffee is source=rule"
    );

    // Turn the setting off. Phase 3: a third Coffee imports uncategorized.
    kernel.set_auto_categorize_on_import(false).unwrap();
    assert!(!kernel.auto_categorize_on_import().unwrap());
    let third = kernel
        .ingest_batch(
            &OneRowCsv {
                fingerprint: "fp-3",
                merchant: "Coffee",
            },
            tagged_input("c"),
            &ParserHints::default(),
            account_id,
            &ParserLimits::default(),
            &meta(),
        )
        .unwrap();
    assert_eq!(third.committed, 1);
    assert_eq!(
        third.auto_categorized, 0,
        "auto-apply is off, so nothing is filled"
    );
}

/// Bulk-accept the low-confidence review queue (ADR 0030 addendum, personal-cfo-j5ij):
/// a merchant taught a 2-of-3 split (66% agreement, below the 70% threshold) yields a
/// low-confidence auto-categorization on the next import, which surfaces in the Money
/// Inbox and is cleared by `accept_low_confidence_categories`.
#[test]
fn accept_low_confidence_categories_clears_the_review_queue() {
    let dir = tempfile::tempdir().unwrap();
    let kernel = Kernel::create_vault(dir.path().join("vault.db"), PW).unwrap();
    let checking = account("Checking");
    let account_id = checking.id();
    kernel
        .dispatch(CommandEnvelope::new(meta(), CreateAccount::new(checking)))
        .unwrap();

    // Three ShopMart imports (no memory yet → no auto-apply), then teach a 2-of-3 split so
    // the merchant's agreement is 6666 bps — below the 7000 review threshold. The plugin is
    // a `&'static` literal each time (rvalue static promotion) to satisfy `ingest_batch`.
    let hints = ParserHints::default();
    let limits = ParserLimits::default();
    kernel
        .ingest_batch(
            &OneRowCsv {
                fingerprint: "sm-1",
                merchant: "ShopMart",
            },
            tagged_input("1"),
            &hints,
            account_id,
            &limits,
            &meta(),
        )
        .unwrap();
    kernel
        .ingest_batch(
            &OneRowCsv {
                fingerprint: "sm-2",
                merchant: "ShopMart",
            },
            tagged_input("2"),
            &hints,
            account_id,
            &limits,
            &meta(),
        )
        .unwrap();
    kernel
        .ingest_batch(
            &OneRowCsv {
                fingerprint: "sm-3",
                merchant: "ShopMart",
            },
            tagged_input("3"),
            &hints,
            account_id,
            &limits,
            &meta(),
        )
        .unwrap();
    let cats: Vec<_> = kernel
        .category_views()
        .unwrap()
        .into_iter()
        .filter(|c| c.category_type == "expense" && !c.archived)
        .take(2)
        .collect();
    assert!(cats.len() >= 2, "need two seeded expense categories");
    let ids: Vec<_> = kernel
        .transactions(50)
        .unwrap()
        .into_iter()
        .filter(|r| r.counterparty.as_deref() == Some("shopmart"))
        .map(|r| r.transaction_id)
        .collect();
    assert_eq!(ids.len(), 3);
    for (txn, cat) in [
        (ids[0], cats[0].id),
        (ids[1], cats[0].id),
        (ids[2], cats[1].id),
    ] {
        kernel
            .dispatch(CommandEnvelope::new(
                meta(),
                RecategorizeTransaction::new(txn, Some(cat)),
            ))
            .unwrap();
    }

    // A 4th ShopMart import auto-applies (default on) at the 6666 agreement → low-confidence.
    let fourth = kernel
        .ingest_batch(
            &OneRowCsv {
                fingerprint: "sm-4",
                merchant: "ShopMart",
            },
            tagged_input("4"),
            &hints,
            account_id,
            &limits,
            &meta(),
        )
        .unwrap();
    assert_eq!(fourth.auto_categorized, 1);

    let low_count = || {
        kernel
            .money_inbox_list()
            .unwrap()
            .into_iter()
            .filter(|i| i.item_kind == "low_confidence_category")
            .count()
    };
    assert_eq!(
        low_count(),
        1,
        "the auto-applied 4th ShopMart is queued for review"
    );

    // Bulk-accept clears the queue (marks reviewed, keeping the rule category).
    assert_eq!(kernel.accept_low_confidence_categories(&meta()).unwrap(), 1);
    assert_eq!(low_count(), 0, "accepted items leave the queue");
}

#[test]
fn ingest_batch_parses_stages_commits_and_dedupes() {
    let dir = tempfile::tempdir().unwrap();
    let kernel = Kernel::create_vault(dir.path().join("vault.db"), PW).unwrap();

    let checking = account("Checking");
    let account_id = checking.id();
    kernel
        .dispatch(CommandEnvelope::new(meta(), CreateAccount::new(checking)))
        .unwrap();

    let result = kernel
        .ingest_batch(
            &ThreeRowCsv,
            csv_input(),
            &ParserHints::default(),
            account_id,
            &ParserLimits::default(),
            &meta(),
        )
        .unwrap();

    assert_eq!(result.staged, 3);
    assert_eq!(result.committed, 2);
    assert_eq!(result.flagged, 1);
    assert_eq!(result.status, "partially_committed");

    // The two unique transactions posted to the ledger (−12.99 + −42.00); the
    // duplicate did not. Balance is summed from postings, so it reflects exactly
    // the committed movements.
    assert_eq!(
        kernel.account_balance(account_id).unwrap(),
        Some(Money::new(-5499, Currency::Usd))
    );

    // Re-importing identical bytes is file-deduped — skipped without re-parsing,
    // and the ledger is unchanged.
    let again = kernel
        .ingest_batch(
            &ThreeRowCsv,
            csv_input(),
            &ParserHints::default(),
            account_id,
            &ParserLimits::default(),
            &meta(),
        )
        .unwrap();
    assert_eq!(again.status, "already_imported");
    assert_eq!(
        kernel.account_balance(account_id).unwrap(),
        Some(Money::new(-5499, Currency::Usd))
    );
}

/// Feedback 2026-07-03: deleting (voiding) imported transactions must let the SAME file
/// re-import cleanly — the ghosts of voided rows can't keep blocking the restore.
#[test]
fn voided_transactions_do_not_block_reimporting_the_same_file() {
    let dir = tempfile::tempdir().unwrap();
    let kernel = Kernel::create_vault(dir.path().join("vault.db"), PW).unwrap();

    let checking = account("Checking");
    let account_id = checking.id();
    kernel
        .dispatch(CommandEnvelope::new(meta(), CreateAccount::new(checking)))
        .unwrap();

    let first = kernel
        .ingest_batch(
            &ThreeRowCsv,
            csv_input(),
            &ParserHints::default(),
            account_id,
            &ParserLimits::default(),
            &meta(),
        )
        .unwrap();
    assert_eq!(first.committed, 2);

    // The user deletes everything the import created (bulk delete → void).
    for row in kernel.transactions(100).unwrap() {
        kernel
            .dispatch(CommandEnvelope::new(
                meta(),
                finance_kernel::VoidTransaction::new(row.transaction_id),
            ))
            .unwrap();
    }
    assert_eq!(
        kernel.account_balance(account_id).unwrap(),
        Some(Money::new(0, Currency::Usd)),
        "everything voided"
    );

    // Re-importing the identical bytes now restores the rows instead of refusing:
    // neither the file-level fingerprint nor the per-row fingerprints of voided
    // transactions count as duplicates any more.
    let again = kernel
        .ingest_batch(
            &ThreeRowCsv,
            csv_input(),
            &ParserHints::default(),
            account_id,
            &ParserLimits::default(),
            &meta(),
        )
        .unwrap();
    assert_ne!(
        again.status, "already_imported",
        "file-level dedupe released"
    );
    assert_eq!(again.committed, 2, "the unique rows commit again");
    assert_eq!(
        kernel.account_balance(account_id).unwrap(),
        Some(Money::new(-5499, Currency::Usd)),
        "the restore lands the same balance as the original import"
    );
}
