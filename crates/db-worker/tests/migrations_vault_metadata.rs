//! Migrations, vault metadata, and startup health
//! (moved verbatim from the old in-file `lib.rs` tests module).

mod common;

use common::*;
use db_worker::*;
use tempfile::TempDir;

#[test]
fn health_checks_integrity_and_attachment_consistency() {
    // Raw-key open so attachments (which need the DEK) can be imported.
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("raw.vault");
    let dek = vault_crypto::generate_dek().unwrap();
    let worker = DbWorker::open_with_raw_key(&path, dek).unwrap();

    // A fresh, healthy vault: integrity ok, no attachments to be missing.
    assert!(worker.integrity_ok());
    assert!(worker.attachments_consistent().unwrap());

    // Import an attachment → its blob is present, so the manifest matches.
    worker
        .import_attachment(b"%PDF-1.4 hello", Some("application/pdf"), Some("t.pdf"))
        .unwrap();
    assert!(worker.attachments_consistent().unwrap());

    // Lose the blob → the manifest no longer matches the store.
    for entry in std::fs::read_dir(worker.blobs_dir()).unwrap() {
        std::fs::remove_file(entry.unwrap().path()).unwrap();
    }
    assert!(!worker.attachments_consistent().unwrap());
    // Integrity is independent of the blob store.
    assert!(worker.integrity_ok());
}

#[test]
fn vault_metadata_seed_is_idempotent_across_reopen() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.vault");
    let first = DbWorker::open(&path, KEY)
        .unwrap()
        .vault_metadata()
        .unwrap()
        .vault_id;
    let second = DbWorker::open(&path, KEY)
        .unwrap()
        .vault_metadata()
        .unwrap()
        .vault_id;
    assert_eq!(first, second, "re-opening must not reseed vault_metadata");
}
