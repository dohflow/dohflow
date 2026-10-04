//! End-to-end ingestion pipeline (personal-cfo-cmx). Drives a real `Kernel`
//! (temp vault) through `ingest_batch` with a test `ImporterPlugin`: parse in the
//! bounded host → stage → dedupe → commit the clean rows → advance batch state.
//! Asserts the unique transactions hit the ledger, the duplicate is flagged (not
//! posted), and an exact re-upload is file-deduped. The first importer (`cu8`) and
//! the IPC entry point plug into this same `ingest_batch` path.

use chrono::NaiveDate;
use finance_kernel::{
    Account, AccountFlags, AccountId, ActorType, CashflowRole, CommandEnvelope, CommandMeta,
    CommitStaged, CreateAccount, Currency, ImportWarning, ImporterPlugin, Kernel, KernelError,
    LedgerAccountId, Money, ParseError, ParseWarning, ParsedBatch, ParsedRecord, ParsedTransaction,
    ParserHints, ParserInput, ParserLimits, RecategorizeTransaction, StagedTransactionId,
    MAX_IMPORT_WARNINGS,
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
            skipped: Vec::new(),
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
            skipped: Vec::new(),
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

/// A test importer that stages `good` rows and reports `skips` rows it could
/// not use, plus one note on a staged row (personal-cfo-pxi.10).
struct SkippingCsv {
    good: usize,
    skips: usize,
}

impl ImporterPlugin for SkippingCsv {
    fn id(&self) -> &'static str {
        "skipping-csv"
    }
    fn display_name(&self) -> &'static str {
        "Skipping CSV"
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
            records: (0..self.good)
                .map(|i| record(i, &format!("fp-good-{i}"), -100, "Store"))
                .collect(),
            warnings: vec![ParseWarning {
                row: Some(0),
                message: "ambiguous date (assumed US M/D/Y)".to_owned(),
            }],
            skipped: (0..self.skips)
                .map(|i| ParseWarning {
                    row: Some(self.good + i),
                    message: "unparseable / missing amount".to_owned(),
                })
                .collect(),
        })
    }
}

#[test]
fn ingest_batch_reports_skipped_rows_before_notes() {
    let dir = tempfile::tempdir().unwrap();
    let kernel = Kernel::create_vault(dir.path().join("vault.db"), PW).unwrap();
    let checking = account("Checking");
    let account_id = checking.id();
    kernel
        .dispatch(CommandEnvelope::new(meta(), CreateAccount::new(checking)))
        .unwrap();

    let result = kernel
        .ingest_batch(
            &SkippingCsv { good: 2, skips: 1 },
            tagged_input("skips"),
            &ParserHints::default(),
            account_id,
            &ParserLimits::default(),
            &meta(),
        )
        .unwrap();
    assert_eq!(result.status, "committed");
    assert_eq!(result.committed, 2);
    assert_eq!(result.skipped_rows, 1);
    assert_eq!(
        result.warnings,
        vec![
            ImportWarning {
                row: Some(2),
                message: "unparseable / missing amount".to_owned(),
                skipped: true,
            },
            ImportWarning {
                row: Some(0),
                message: "ambiguous date (assumed US M/D/Y)".to_owned(),
                skipped: false,
            },
        ]
    );
}

#[test]
fn ingest_batch_bounds_the_warning_list_but_counts_every_skip() {
    let dir = tempfile::tempdir().unwrap();
    let kernel = Kernel::create_vault(dir.path().join("vault.db"), PW).unwrap();
    let checking = account("Checking");
    let account_id = checking.id();
    kernel
        .dispatch(CommandEnvelope::new(meta(), CreateAccount::new(checking)))
        .unwrap();

    let skips = MAX_IMPORT_WARNINGS + 5;
    let result = kernel
        .ingest_batch(
            &SkippingCsv {
                good: 1,
                skips: MAX_IMPORT_WARNINGS + 5,
            },
            tagged_input("many-skips"),
            &ParserHints::default(),
            account_id,
            &ParserLimits::default(),
            &meta(),
        )
        .unwrap();
    assert_eq!(
        result.skipped_rows as usize, skips,
        "the count is never truncated"
    );
    assert_eq!(result.warnings.len(), MAX_IMPORT_WARNINGS);
    assert!(
        result.warnings.iter().all(|w| w.skipped),
        "skips fill the list first"
    );
    assert_eq!(result.warnings[0].row, Some(1));

    // A re-upload of the same file reports nothing new.
    let again = kernel
        .ingest_batch(
            &SkippingCsv {
                good: 1,
                skips: MAX_IMPORT_WARNINGS + 5,
            },
            tagged_input("many-skips"),
            &ParserHints::default(),
            account_id,
            &ParserLimits::default(),
            &meta(),
        )
        .unwrap();
    assert_eq!(again.status, "already_imported");
    assert_eq!((again.skipped_rows, again.warnings.len()), (0, 0));
}

/// Three rows whose middle one the ledger cannot accept (personal-cfo-pxi.9):
/// `MiddleRow(0)` has a zero amount, `MiddleRow(1)` a currency other than the
/// account's. A parser would normally skip a zero row; this stub reaches the
/// pipeline with it, which is what a connector or a future plugin could do.
struct MiddleRow(u8);

fn three_with_bad_middle(kind: u8) -> Vec<ParsedRecord> {
    let mut middle = record(1, "fp-mid", -1, "Middle");
    let txn = middle.transaction.as_mut().unwrap();
    txn.amount = match kind {
        0 => Money::new(0, Currency::Usd),
        _ => Money::new(-2500, Currency::Eur),
    };
    vec![
        record(0, "fp-first", -1000, "First"),
        middle,
        record(2, "fp-third", -2000, "Third"),
    ]
}

impl ImporterPlugin for MiddleRow {
    fn id(&self) -> &'static str {
        "middle-row-csv"
    }
    fn display_name(&self) -> &'static str {
        "Middle Row CSV"
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
            records: three_with_bad_middle(self.0),
            warnings: vec![],
            skipped: Vec::new(),
        })
    }
}

/// The Money Inbox items waiting on an imported row (other item kinds, such
/// as uncategorized transactions, are not this test's concern).
fn waiting_rows(kernel: &Kernel) -> Vec<finance_kernel::MoneyInboxItem> {
    kernel
        .money_inbox_list()
        .unwrap()
        .into_iter()
        .filter(|item| item.item_kind == "imported_waiting_commit")
        .collect()
}

fn checking_kernel() -> (tempfile::TempDir, Kernel, AccountId) {
    let dir = tempfile::tempdir().unwrap();
    let kernel = Kernel::create_vault(dir.path().join("vault.db"), PW).unwrap();
    let checking = account("Checking");
    let account_id = checking.id();
    kernel
        .dispatch(CommandEnvelope::new(meta(), CreateAccount::new(checking)))
        .unwrap();
    (dir, kernel, account_id)
}

fn import_middle(
    kernel: &Kernel,
    plugin: &'static MiddleRow,
    account_id: AccountId,
) -> finance_kernel::BatchResult {
    kernel
        .ingest_batch(
            plugin,
            tagged_input(&format!("middle-{}", plugin.0)),
            &ParserHints::default(),
            account_id,
            &ParserLimits::default(),
            &meta(),
        )
        .expect("one bad row never fails the import")
}

#[test]
fn an_invalid_row_is_flagged_and_the_rest_of_the_batch_commits() {
    for (plugin, reason) in [
        (&MiddleRow(0), "not imported: the amount is zero"),
        (
            &MiddleRow(1),
            "not imported: its currency differs from the account's currency",
        ),
    ] {
        let (_dir, kernel, account_id) = checking_kernel();
        let result = import_middle(&kernel, plugin, account_id);

        // Rows 1 and 3 commit; row 2 is flagged; the batch is terminal.
        assert_eq!(
            (result.staged, result.committed, result.flagged),
            (3, 2, 1),
            "{reason}"
        );
        assert_eq!(result.status, "partially_committed");
        assert_eq!(
            kernel.account_balance(account_id).unwrap(),
            Some(Money::new(-3000, Currency::Usd)),
            "only the two valid rows posted"
        );

        // The flagged row waits in the Money Inbox with its reason.
        let inbox = waiting_rows(&kernel);
        assert_eq!(inbox.len(), 1, "{reason}");
        assert_eq!(inbox[0].item_kind, "imported_waiting_commit");
        assert!(
            inbox[0].payload_json.contains(reason),
            "{}",
            inbox[0].payload_json
        );

        // Re-importing the identical file is file-deduped and adds nothing.
        let again = import_middle(&kernel, plugin, account_id);
        assert_eq!(again.status, "already_imported");
        assert_eq!(waiting_rows(&kernel).len(), 1);
        assert_eq!(
            kernel.account_balance(account_id).unwrap(),
            Some(Money::new(-3000, Currency::Usd))
        );
    }
}

#[test]
fn import_anyway_on_an_invalid_row_is_refused_and_changes_nothing() {
    for (plugin, error) in [
        (&MiddleRow(0), "staged transaction amount must be non-zero"),
        (
            &MiddleRow(1),
            "staged transaction currency does not match its account",
        ),
    ] {
        let (_dir, kernel, account_id) = checking_kernel();
        import_middle(&kernel, plugin, account_id);
        let before = waiting_rows(&kernel);
        assert_eq!(before.len(), 1);
        let staged = StagedTransactionId::from_uuid(before[0].target_id);

        let refused = kernel
            .dispatch(CommandEnvelope::new(
                meta(),
                CommitStaged::import_anyway(staged),
            ))
            .expect_err("an invalid row cannot be forced into the ledger");
        assert!(
            matches!(&refused, KernelError::Validation(message) if message == error),
            "{refused:?}"
        );

        // Nothing moved: same balance, the row still waits with its reason.
        assert_eq!(
            kernel.account_balance(account_id).unwrap(),
            Some(Money::new(-3000, Currency::Usd))
        );
        let after = waiting_rows(&kernel);
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].target_id, before[0].target_id);
        assert_eq!(after[0].payload_json, before[0].payload_json);
    }
}

#[test]
fn a_sync_batch_with_an_invalid_row_still_reaches_a_terminal_state() {
    // ingest_sync_batch commits through the same CommitStaged path.
    let (_dir, kernel, account_id) = checking_kernel();
    let mut records = three_with_bad_middle(0);
    for record in &mut records {
        record.external_id = Some(record.source_hash.clone());
        record.transaction.as_mut().unwrap().external_account = Some("acct-1".to_owned());
    }
    let parsed = ParsedBatch {
        source_format: "simplefin".to_owned(),
        accounts: vec![],
        records,
        warnings: vec![],
        skipped: Vec::new(),
    };
    let account_map =
        std::collections::BTreeMap::from([("acct-1".to_owned(), account_id.as_uuid())]);
    let synced = kernel
        .ingest_sync_batch("simplefin", "1.0.0", "Test", &parsed, &account_map, &meta())
        .expect("one bad row never fails the sync");
    assert_eq!((synced.batch.committed, synced.batch.flagged), (2, 1));
    assert_eq!(synced.batch.status, "partially_committed");
    assert_eq!(
        kernel.account_balance(account_id).unwrap(),
        Some(Money::new(-3000, Currency::Usd))
    );
}
