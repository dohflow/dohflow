//! No-plaintext-leak, round-trip, dedup, and crypto-shred tests for the
//! encrypted attachment store (personal-cfo-bcj, ADR 0023). The leak test is the
//! Week-8 gate's "no plaintext attachment artifacts outside the vault" criterion:
//! it plants a known PDF + PNG, imports them, and asserts their plaintext bytes
//! never appear at rest (the SQLCipher DB, the ciphertext blobs, the envelope) or
//! in the OS temp dir — including after a simulated crash (an unclean drop).

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use db_worker::DbWorker;
use uuid::Uuid;
use vault_crypto::generate_dek;

/// A canary embedded in each plaintext document; it must never appear at rest.
const CANARY: &[u8] = b"PLAINTEXT-LEAK-CANARY-3f9a2b1c8d7e";

fn planted_pdf() -> Vec<u8> {
    let mut v = b"%PDF-1.4\n1 0 obj<<>>endobj\n".to_vec();
    v.extend_from_slice(CANARY);
    v.extend_from_slice(b"\ntrailer<<>>\n%%EOF\n");
    v
}

fn planted_png() -> Vec<u8> {
    let mut v = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    v.extend_from_slice(CANARY);
    v.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, b'I', b'E', b'N', b'D']);
    v
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// True if any file anywhere under `dir` contains `needle`.
fn dir_contains(dir: &Path, needle: &[u8]) -> bool {
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if dir_contains(&path, needle) {
                return true;
            }
        } else if fs::read(&path)
            .map(|b| contains(&b, needle))
            .unwrap_or(false)
        {
            return true;
        }
    }
    false
}

fn temp_entries() -> HashSet<PathBuf> {
    fs::read_dir(std::env::temp_dir())
        .map(|rd| rd.flatten().map(|e| e.path()).collect())
        .unwrap_or_default()
}

fn count_files(dir: &Path) -> usize {
    fs::read_dir(dir)
        .map(|rd| rd.flatten().filter(|e| e.path().is_file()).count())
        .unwrap_or(0)
}

#[test]
fn attachments_never_leak_plaintext_at_rest_or_in_temp() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("vault.db");
    let entity = Uuid::from_bytes([7u8; 16]);

    let temp_before = temp_entries();
    let worker = DbWorker::open_with_raw_key(&db_path, generate_dek().unwrap()).unwrap();

    let pdf = planted_pdf();
    let png = planted_png();
    let pdf_id = worker
        .import_attachment(&pdf, Some("application/pdf"), Some("statement.pdf"))
        .unwrap();
    let png_id = worker
        .import_attachment(&png, Some("image/png"), Some("receipt.png"))
        .unwrap();
    worker
        .link_attachment(pdf_id, "transaction", entity)
        .unwrap();
    worker
        .link_attachment(png_id, "transaction", entity)
        .unwrap();

    // Round-trip: the ciphertext decrypts back to exactly the original bytes.
    assert_eq!(worker.read_attachment_bytes(pdf_id).unwrap(), pdf);
    assert_eq!(worker.read_attachment_bytes(png_id).unwrap(), png);

    // Dedup: re-importing identical bytes returns the same id.
    let again = worker
        .import_attachment(&pdf, Some("application/pdf"), None)
        .unwrap();
    assert_eq!(again, pdf_id);

    // Listing surfaces both (metadata only).
    assert_eq!(
        worker.attachments_for("transaction", entity).unwrap().len(),
        2
    );

    // The blobs/ dir holds exactly two ciphertext files and no leftover `.tmp`.
    let blobs = dir.path().join("blobs");
    assert_eq!(count_files(&blobs), 2, "expected two blob files");
    let has_tmp = fs::read_dir(&blobs)
        .unwrap()
        .flatten()
        .any(|e| e.path().extension().is_some_and(|x| x == "tmp"));
    assert!(!has_tmp, "a .tmp blob was left behind");

    // No plaintext anywhere under the vault dir (SQLCipher DB + ciphertext blobs).
    assert!(
        !dir_contains(dir.path(), CANARY),
        "plaintext leaked at rest in the vault directory"
    );
    // No NEW file in the OS temp dir contains the plaintext (no temp staging).
    let leaked: Vec<_> = temp_entries()
        .difference(&temp_before)
        .filter(|p| p.is_file() && fs::read(p).map(|b| contains(&b, CANARY)).unwrap_or(false))
        .cloned()
        .collect();
    assert!(
        leaked.is_empty(),
        "plaintext leaked to the OS temp dir: {leaked:?}"
    );

    // Crash simulation: an unclean drop (scrubs the DEK) must leave nothing in
    // plaintext on disk.
    drop(worker);
    assert!(
        !dir_contains(dir.path(), CANARY),
        "plaintext appeared at rest after an unclean drop"
    );
}

#[test]
fn crypto_shred_on_last_unlink_makes_bytes_unrecoverable() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("vault.db");
    let worker = DbWorker::open_with_raw_key(&db_path, generate_dek().unwrap()).unwrap();
    let blobs = dir.path().join("blobs");
    let txn = Uuid::from_bytes([1u8; 16]);
    let bill = Uuid::from_bytes([2u8; 16]);

    let bytes = planted_pdf();
    let id = worker.import_attachment(&bytes, None, None).unwrap();
    worker.link_attachment(id, "transaction", txn).unwrap();
    worker.link_attachment(id, "bill", bill).unwrap();
    assert_eq!(count_files(&blobs), 1);

    // One of two links removed: still referenced — blob survives, bytes readable.
    worker.unlink_attachment(id, "transaction", txn).unwrap();
    assert_eq!(count_files(&blobs), 1);
    assert_eq!(worker.read_attachment_bytes(id).unwrap(), bytes);

    // Last link removed: crypto-shred — the row (and the only wrapped key) is
    // deleted, the blob is unlinked, and the bytes are unrecoverable.
    worker.unlink_attachment(id, "bill", bill).unwrap();
    assert_eq!(
        count_files(&blobs),
        0,
        "blob should be unlinked after the last link"
    );
    assert!(
        worker.read_attachment_bytes(id).is_err(),
        "a crypto-shredded attachment must not be readable"
    );
    assert!(!dir_contains(dir.path(), CANARY));
}
