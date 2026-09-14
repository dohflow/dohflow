//! WAL/SHM/temp-file plaintext-leak test (personal-cfo-zxvl, §6.4 / §20.1).
//!
//! Release-blocking: proves no plaintext §6.6 sensitive value escapes the
//! SQLCipher-encrypted vault into the side files SQLite or the OS may create
//! alongside it — the WAL, the shared-memory index, the rollback journal, and the
//! temp store. Unique sentinels are seeded via the Finance Kernel command bus
//! (never raw SQL); a write workload exercises the WAL plus a read-model rebuild
//! and an attachment; then every file under the vault directory — and any new OS
//! temp file — is scanned for a sentinel byte sequence. A match fails with the
//! leaking path and class.
//!
//! macOS preview / QuickLook caches (PreviewCache, QuickLookCache) are out of
//! scope until in-app attachment preview lands (ADR 0022): nothing renders an
//! attachment yet, so nothing can be cached there. That scan attaches with the
//! preview feature.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDate, Utc};
use finance_kernel::{
    Account, AccountFlags, AccountId, ActorType, CashflowRole, CommandEnvelope, CommandMeta,
    CreateAccount, CreateIncomeSource, Currency, Frequency, Kernel, LedgerAccountId, Money,
    RecordTransaction, TransactionId,
};
use uuid::Uuid;

const PW: &[u8] = b"sentinel vault password";

// Unique sentinels, one per §6.6 sensitive class. Any byte match outside the
// encrypted `.db` is definitive.
const ACCOUNT_SENTINEL: &str = "ACME-SENTINEL-INC-5999000099990000";
const INCOME_SENTINEL: &str = "SENTINEL-EMPLOYER-PAYSTUB-9999";
const FILENAME_SENTINEL: &str = "sentinel-DEADBEEF-CAFE-1234.pdf";
const ATTACHMENT_CANARY: &[u8] = b"ZXVL-SENTINEL-PLAINTEXT-CANARY-0F0F";

fn sentinels() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("account name", ACCOUNT_SENTINEL.as_bytes().to_vec()),
        ("income source name", INCOME_SENTINEL.as_bytes().to_vec()),
        ("attachment filename", FILENAME_SENTINEL.as_bytes().to_vec()),
        ("attachment bytes", ATTACHMENT_CANARY.to_vec()),
    ]
}

fn meta() -> CommandMeta {
    CommandMeta {
        command_id: Uuid::now_v7(),
        correlation_id: Uuid::now_v7(),
        causation_id: None,
        actor_type: ActorType::User,
        actor_id: "zxvl".to_owned(),
        idempotency_key: Uuid::now_v7().to_string(),
    }
}

fn at(rfc3339: &str) -> DateTime<Utc> {
    rfc3339.parse().unwrap()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && needle.len() <= haystack.len() && {
        haystack.windows(needle.len()).any(|w| w == needle)
    }
}

/// Scan every file under `dir` (recursively) for each sentinel, recording leaks.
fn scan_dir(dir: &Path, sents: &[(&'static str, Vec<u8>)], leaks: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_dir(&path, sents, leaks);
        } else if let Ok(bytes) = fs::read(&path) {
            for (class, needle) in sents {
                if contains(&bytes, needle) {
                    leaks.push(format!("LEAK: {} matched {class}", path.display()));
                }
            }
        }
    }
}

fn temp_entries() -> HashSet<PathBuf> {
    fs::read_dir(std::env::temp_dir())
        .map(|rd| rd.flatten().map(|e| e.path()).collect())
        .unwrap_or_default()
}

#[test]
fn no_sentinel_leaks_into_side_files() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("vault.db");
    let temp_before = temp_entries();

    let kernel = Kernel::create_vault(&db_path, PW).unwrap();

    // Seed sentinel data through the command bus (no raw inserts).
    let acct = Account::new(
        AccountId::new(),
        LedgerAccountId::new(),
        ACCOUNT_SENTINEL,
        CashflowRole::LiquidCash,
        Currency::Usd,
        AccountFlags::default(),
    );
    let acct_id = acct.id();
    kernel
        .dispatch(CommandEnvelope::new(
            meta(),
            CreateAccount::with_opening_balance(acct, Money::new(1_337_142, Currency::Usd)),
        ))
        .unwrap();
    // A workload that accumulates WAL frames.
    for i in 1..=20 {
        kernel
            .dispatch(CommandEnvelope::new(
                meta(),
                RecordTransaction::new(
                    TransactionId::new(),
                    acct_id,
                    Money::new(-(i * 137), Currency::Usd),
                    at("2026-06-05T00:00:00Z"),
                ),
            ))
            .unwrap();
    }
    kernel
        .dispatch(CommandEnvelope::new(
            meta(),
            CreateIncomeSource::new(
                INCOME_SENTINEL,
                Money::new(900_000, Currency::Usd),
                Frequency::Biweekly,
                "2026-06-01".parse::<NaiveDate>().unwrap(),
                Some(acct_id),
            ),
        ))
        .unwrap();
    kernel.rebuild_transaction_display().unwrap(); // exercises sort/temp paths

    // Attach a document with a sentinel filename + canary bytes (the blob store).
    let txn_id = kernel
        .transactions(1)
        .unwrap()
        .first()
        .unwrap()
        .transaction_id;
    let pdf = [b"%PDF-1.4\n".as_slice(), ATTACHMENT_CANARY, b"\n%%EOF\n"].concat();
    kernel
        .attach_document(
            txn_id,
            &pdf,
            Some("application/pdf"),
            Some(FILENAME_SENTINEL),
        )
        .unwrap();

    // The WAL exists while the worker is alive — the same un-checkpointed state a
    // crash would leave. Sanity-check the workload actually exercised the WAL.
    let wal = PathBuf::from(format!("{}-wal", db_path.display()));
    assert!(
        wal.exists(),
        "expected a WAL side file at {}",
        wal.display()
    );

    // Nothing under the vault dir (vault.db, -wal, -shm, journal, blobs/) may hold
    // a sentinel in the clear...
    let sents = sentinels();
    let mut leaks = Vec::new();
    scan_dir(dir.path(), &sents, &mut leaks);
    // ...nor any NEW file in the OS temp dir (SQLite temp store / staging).
    for path in temp_entries().difference(&temp_before) {
        if path.is_file() {
            if let Ok(bytes) = fs::read(path) {
                for (class, needle) in &sents {
                    if contains(&bytes, needle) {
                        leaks.push(format!(
                            "LEAK: {} (OS temp) matched {class}",
                            path.display()
                        ));
                    }
                }
            }
        }
    }

    assert!(
        leaks.is_empty(),
        "plaintext leaked into side files:\n{}",
        leaks.join("\n")
    );

    drop(kernel);
}
