//! Database side of vault key rotation (ADR 0083 §2–§3, personal-cfo-2y8): the
//! re-encrypted copy is one closed file under the new DEK, carries the live
//! vault's version marker and rows, and re-wraps and re-addresses every
//! attachment so it reads back under the new key and new name.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{create_account_cmd, meta};
use db_worker::{verify_rekeyed_copy, DbWorker};
use vault_crypto::{derive_kek, generate_dek, generate_salt, unwrap_dek, wrap_dek, Dek, Profile};

const PDF: &[u8] = b"%PDF-1.4\nrekey attachment one\n%%EOF\n";
const PNG: &[u8] = b"\x89PNG\r\n\x1a\nrekey attachment two";

/// A second handle on the same DEK bytes, via the production wrap/unwrap path
/// (`Dek` is deliberately non-`Clone`).
fn duplicate(dek: &Dek) -> Dek {
    let salt = generate_salt().unwrap();
    let kek = derive_kek(
        b"test-duplicate",
        &salt,
        &Profile::LegacyCompatibility.params(),
    )
    .unwrap();
    unwrap_dek(&kek, &wrap_dek(&kek, dek).unwrap()).unwrap()
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut os = path.as_os_str().to_owned();
    os.push(suffix);
    PathBuf::from(os)
}

/// A raw-keyed vault with an account and two attachments, reopened through the
/// unlock path (`open_existing_with_raw_key`, whose writer has no
/// SQLITE_OPEN_CREATE flag — the state a real rotation starts from).
fn seeded() -> (
    tempfile::TempDir,
    PathBuf,
    Dek,
    DbWorker,
    Vec<core_ledger::AttachmentId>,
) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.db");
    let dek = generate_dek().unwrap();
    drop(DbWorker::open_with_raw_key(&path, duplicate(&dek)).unwrap());
    let worker = DbWorker::open_existing_with_raw_key(&path, duplicate(&dek)).unwrap();
    worker.dispatch(meta(), create_account_cmd()).unwrap();
    let a = worker
        .import_attachment(PDF, Some("application/pdf"), Some("one.pdf"))
        .unwrap();
    let b = worker
        .import_attachment(PNG, Some("image/png"), Some("two.png"))
        .unwrap();
    (dir, path, dek, worker, vec![a, b])
}

#[test]
fn rekeyed_copy_is_one_closed_file_with_markers_rows_and_readdressed_attachments() {
    let (dir, path, old_dek, worker, ids) = seeded();
    let copy = with_suffix(&path, ".rekey-new");
    let new_dek = generate_dek().unwrap();
    let user_version = worker.user_version().unwrap();
    let accounts = worker.account_count().unwrap();
    let operations = worker.operation_count().unwrap();

    let renames = worker.prepare_rekeyed_copy(&new_dek, &copy).unwrap();

    // One closed file: no side files beside the copy.
    for suffix in ["-wal", "-shm", "-journal"] {
        assert!(!with_suffix(&copy, suffix).exists(), "copy left {suffix}");
    }
    // Every attachment is renamed, and every new name differs from the old.
    assert_eq!(renames.len(), 2);
    let blobs = dir.path().join("blobs");
    for rename in &renames {
        assert_ne!(rename.old, rename.new);
        assert!(blobs.join(&rename.old).is_file(), "old blob untouched");
        assert!(!blobs.join(&rename.new).exists(), "prepare links nothing");
    }
    // Before links exist, verification refuses (a row names a missing blob).
    assert!(verify_rekeyed_copy(&copy, &new_dek, &blobs, user_version).is_err());
    // The live vault is untouched and still opens under the old key.
    assert_eq!(worker.account_count().unwrap(), accounts);
    assert_eq!(worker.read_attachment_bytes(ids[0]).unwrap(), PDF);

    for rename in &renames {
        fs::hard_link(blobs.join(&rename.old), blobs.join(&rename.new)).unwrap();
    }
    verify_rekeyed_copy(&copy, &new_dek, &blobs, user_version).unwrap();
    // A wrong key or a different version marker fails verification.
    assert!(verify_rekeyed_copy(&copy, &old_dek, &blobs, user_version).is_err());
    assert!(verify_rekeyed_copy(&copy, &new_dek, &blobs, user_version + 1).is_err());
    for suffix in ["-wal", "-shm", "-journal"] {
        assert!(!with_suffix(&copy, suffix).exists(), "verify left {suffix}");
    }

    // Swap the copy in (the kernel's apply step, by hand) and reopen under the
    // new key: same rows, same marker, attachments read back under new names.
    drop(worker);
    for suffix in ["-wal", "-shm"] {
        let _ = fs::remove_file(with_suffix(&path, suffix));
    }
    fs::rename(&copy, &path).unwrap();
    for rename in &renames {
        fs::remove_file(blobs.join(&rename.old)).unwrap();
    }
    let reopened = DbWorker::open_existing_with_raw_key(&path, duplicate(&new_dek)).unwrap();
    assert_eq!(reopened.user_version().unwrap(), user_version);
    assert_eq!(reopened.account_count().unwrap(), accounts);
    assert_eq!(reopened.operation_count().unwrap(), operations);
    assert_eq!(reopened.read_attachment_bytes(ids[0]).unwrap(), PDF);
    assert_eq!(reopened.read_attachment_bytes(ids[1]).unwrap(), PNG);
    assert!(reopened.attachments_consistent().unwrap());
    assert!(reopened.integrity_ok());
    // Importing an identical file after rotation dedupes against the existing
    // (re-addressed) attachment instead of storing a second copy.
    assert_eq!(
        reopened
            .import_attachment(PDF, Some("application/pdf"), None)
            .unwrap(),
        ids[0]
    );
    drop(reopened);

    // The old key no longer opens the rotated database.
    assert!(DbWorker::open_existing_with_raw_key(&path, old_dek).is_err());
}

#[test]
fn prepare_refuses_an_existing_destination_and_a_vault_without_attachments_works() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.db");
    let worker = DbWorker::open_with_raw_key(&path, generate_dek().unwrap()).unwrap();
    worker.dispatch(meta(), create_account_cmd()).unwrap();
    let copy = with_suffix(&path, ".rekey-new");
    fs::write(&copy, b"stale").unwrap();
    assert!(worker
        .prepare_rekeyed_copy(&generate_dek().unwrap(), &copy)
        .is_err());
    assert_eq!(fs::read(&copy).unwrap(), b"stale", "never overwritten");
    fs::remove_file(&copy).unwrap();

    let new_dek = generate_dek().unwrap();
    let renames = worker.prepare_rekeyed_copy(&new_dek, &copy).unwrap();
    assert!(renames.is_empty());
    verify_rekeyed_copy(
        &copy,
        &new_dek,
        &dir.path().join("blobs"),
        worker.user_version().unwrap(),
    )
    .unwrap();
}
