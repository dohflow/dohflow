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
