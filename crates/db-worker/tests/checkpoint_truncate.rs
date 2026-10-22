//! `DbWorker::checkpoint_truncate` (personal-cfo-7ie.14): folds the write-ahead log into the main
//! file and truncates it, and says so honestly when it cannot.

mod common;

use std::fs;
use std::path::Path;

use common::*;
use db_worker::{DbError, DbWorker};
use vault_crypto::generate_dek;

fn wal_len(path: &Path) -> u64 {
    let mut os = path.as_os_str().to_owned();
    os.push("-wal");
    fs::metadata(std::path::PathBuf::from(os)).map_or(0, |m| m.len())
}

fn raw_keyed() -> (tempfile::TempDir, std::path::PathBuf, DbWorker) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.db");
    let worker = DbWorker::open_with_raw_key(&path, generate_dek().unwrap()).unwrap();
    (dir, path, worker)
}

#[test]
fn a_checkpoint_empties_the_wal() {
    let (_dir, path, worker) = raw_keyed();
    for n in 0..5 {
        worker.set_setting(&format!("user.k{n}"), "v").unwrap();
    }
    assert!(wal_len(&path) > 0, "the test must start with a live -wal");
    worker.checkpoint_truncate().unwrap();
    assert_eq!(wal_len(&path), 0);
}

#[test]
fn a_reader_that_pins_an_old_snapshot_makes_the_checkpoint_say_busy_not_clean() {
    let (_dir, path, worker) = raw_keyed();
    worker.dispatch(meta(), create_account_cmd()).unwrap();

    // A reader outside the worker holds a read transaction, so frames written after its snapshot
    // cannot be folded into the main file and the log cannot be truncated.
    let reader = worker.read_connection().unwrap();
    reader.execute_batch("BEGIN;").unwrap();
    let _: i64 = reader
        .query_row("SELECT COUNT(*) FROM accounts", [], |row| row.get(0))
        .unwrap();
    worker.set_setting("user.after_reader", "v").unwrap();

    match worker.checkpoint_truncate() {
        Err(DbError::CheckpointBusy) => {}
        other => panic!("expected CheckpointBusy, got {other:?}"),
    }
    assert!(wal_len(&path) > 0, "the log still holds the newer frames");

    // Once the reader is gone, the same call completes and the log is empty.
    reader.execute_batch("COMMIT;").unwrap();
    drop(reader);
    worker.checkpoint_truncate().unwrap();
    assert_eq!(wal_len(&path), 0);
}

// -- integrity_check_with_raw_key (personal-cfo-7ie.14) -------------------------------------------

use db_worker::IntegrityOutcome;
use vault_crypto::{unwrap_dek, wrap_dek, Dek};

/// A second handle on the same DEK bytes (`Dek` is not `Clone`), via the production wrap path.
fn duplicate(dek: &Dek) -> Dek {
    let salt = vault_crypto::generate_salt().unwrap();
    let kek = vault_crypto::derive_kek(
        b"test-duplicate",
        &salt,
        &vault_crypto::Profile::LegacyCompatibility.params(),
    )
    .unwrap();
    unwrap_dek(&kek, &wrap_dek(&kek, dek).unwrap()).unwrap()
}

fn sealed_vault() -> (tempfile::TempDir, std::path::PathBuf, Dek) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.db");
    let dek = generate_dek().unwrap();
    let worker = DbWorker::open_with_raw_key(&path, duplicate(&dek)).unwrap();
    for n in 0..30 {
        worker
            .set_setting(&format!("user.k{n}"), &"v".repeat(200))
            .unwrap();
    }
    worker.checkpoint_truncate().unwrap();
    drop(worker);
    (dir, path, dek)
}

#[test]
fn an_intact_vault_reports_intact_and_a_wrong_key_reports_wrong_key() {
    let (_dir, path, dek) = sealed_vault();
    assert_eq!(
        DbWorker::integrity_check_with_raw_key(&path, &dek).unwrap(),
        IntegrityOutcome::Intact
    );
    assert_eq!(
        DbWorker::integrity_check_with_raw_key(&path, &generate_dek().unwrap()).unwrap(),
        IntegrityOutcome::WrongKey
    );
}

#[test]
fn damage_to_page_one_reads_as_a_wrong_key() {
    let (_dir, path, dek) = sealed_vault();
    let mut bytes = fs::read(&path).unwrap();
    for byte in &mut bytes[64..192] {
        *byte ^= 0xA5;
    }
    fs::write(&path, &bytes).unwrap();
    assert_eq!(
        DbWorker::integrity_check_with_raw_key(&path, &dek).unwrap(),
        IntegrityOutcome::WrongKey
    );
}

/// Whatever SQLite surfaces for a bad page (corrupt, not-a-database, or a bare SQL logic error,
/// depending on what the page holds), the check must call it damage, never a persistence error:
/// damage each page after the first on its own and require a verdict every time.
#[test]
fn damage_to_any_later_page_is_damage_never_an_error() {
    let (_dir, path, dek) = sealed_vault();
    let pristine = fs::read(&path).unwrap();
    let pages = pristine.len() / 4096;
    assert!(pages > 4, "{pages} pages");
    let mut damaged = 0;
    for page in 1..pages {
        let mut bytes = pristine.clone();
        for byte in &mut bytes[page * 4096 + 64..page * 4096 + 192] {
            *byte ^= 0xA5;
        }
        fs::write(&path, &bytes).unwrap();
        match DbWorker::integrity_check_with_raw_key(&path, &dek) {
            Ok(IntegrityOutcome::Damaged { problems }) => {
                assert!(problems >= 1);
                damaged += 1;
            }
            // A page the check never reads (a free page) may go unnoticed.
            Ok(IntegrityOutcome::Intact) => {}
            other => panic!("page {page}: expected a verdict, got {other:?}"),
        }
    }
    assert!(
        damaged >= pages / 2,
        "only {damaged} of {pages} pages read as damage"
    );
    // The check never writes: put the file back and it still verifies.
    fs::write(&path, &pristine).unwrap();
    assert_eq!(
        DbWorker::integrity_check_with_raw_key(&path, &dek).unwrap(),
        IntegrityOutcome::Intact
    );
}
