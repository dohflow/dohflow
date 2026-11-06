//! The base image store (personal-cfo-bxsbz.3): capture and advance under the writer lock, the
//! secret strip, the restore rule and the pre-rebase scrub. Fake data only.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::*;
use crate::base_image::{
    base_image_path, observe, pre_rebase_path, sha256_file, BaseImageError, BaseImageStatus,
    RekeyedImage, SyncBaseState, SECRET_BEARING_TABLES,
};

fn base_row(worker: &DbWorker) -> (i64, Option<Vec<u8>>, String) {
    worker
        .lock()
        .conn
        .query_row(
            "SELECT base_seq, base_digest, state FROM sync_base WHERE singleton = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap()
}

fn count_in(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

fn open_image(worker: &DbWorker, path: &std::path::Path) -> Connection {
    worker
        .key
        .open_image(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .unwrap()
}

fn live_count(worker: &DbWorker, table: &str) -> i64 {
    count_in(&worker.lock().conn, table)
}

/// Pretend every outbox command was pushed and accepted: the rows leave the outbox.
fn clear_outbox(worker: &DbWorker) {
    worker
        .lock()
        .conn
        .execute("DELETE FROM sync_outbox", [])
        .unwrap();
}

fn files_in_sync_dir(worker: &DbWorker) -> Vec<String> {
    let dir = crate::base_image::sync_dir(&worker.path);
    let mut names: Vec<String> = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect(),
        Err(_) => Vec::new(),
    };
    names.sort();
    names
}

fn seed(worker: &DbWorker) {
    worker.dispatch(meta(), create_account_cmd()).unwrap();
}

fn connection(worker: &DbWorker, credential: &str) -> Uuid {
    let id = Uuid::now_v7();
    worker
        .create_connector_connection(id, "lunchflow", credential, Some("Checking"))
        .unwrap();
    id
}

#[test]
fn initializing_makes_the_base_from_the_vault_at_the_given_seq() {
    let (_dir, worker) = worker();
    seed(&worker);
    assert_eq!(
        worker.base_image_status().unwrap(),
        BaseImageStatus::SyncOff
    );

    let capture = worker.initialize_base_image(42).unwrap();

    let path = base_image_path(&worker.path);
    assert_eq!(capture.base_seq, 42);
    assert_eq!(capture.digest, sha256_file(&path).unwrap());
    assert_eq!(capture.image_bytes, std::fs::metadata(&path).unwrap().len());
    let (seq, digest, state) = base_row(&worker);
    assert_eq!((seq, state.as_str()), (42, "Ready"));
    assert_eq!(digest.as_deref(), Some(capture.digest.as_slice()));
    assert_eq!(
        worker.base_image_status().unwrap(),
        BaseImageStatus::Ready { base_seq: 42 }
    );
    // base.db IS the vault at that point: it opens under the vault's key and holds the same rows.
    let image = open_image(&worker, &path);
    assert_eq!(
        count_in(&image, "accounts"),
        live_count(&worker, "accounts")
    );
    assert!(live_count(&worker, "accounts") >= 1);
    assert_eq!(
        count_in(&image, "operation_log"),
        live_count(&worker, "operation_log")
    );
    drop(image);
    // One closed file: no temp file and no SQLite side files are left behind.
    assert_eq!(files_in_sync_dir(&worker), vec!["base.db".to_owned()]);
}

#[test]
fn initializing_turns_outbox_recording_on() {
    let (_dir, worker) = worker();
    worker.initialize_base_image(0).unwrap();
    worker.dispatch(meta(), create_account_cmd()).unwrap();
    assert_eq!(live_count(&worker, "sync_outbox"), 1);
}

#[test]
fn initializing_refuses_while_the_outbox_is_not_empty_and_writes_nothing() {
    let (_dir, worker) = worker();
    // A restored vault: Sync was on, the base is gone, and unpushed work remains.
    worker
        .lock()
        .conn
        .execute(
            "INSERT INTO sync_base (singleton, base_seq, base_digest, state, updated_at)
             VALUES (1, 3, NULL, 'NoBase', '2026-11-05T00:00:00Z')",
            [],
        )
        .unwrap();
    seed(&worker);
    assert_eq!(live_count(&worker, "sync_outbox"), 1);
    let before = base_row(&worker);

    let error = worker.initialize_base_image(9).unwrap_err();

    assert!(
        matches!(error, BaseImageError::OutboxNotEmpty { pending: 1 }),
        "{error:?}"
    );
    assert_eq!(base_row(&worker), before, "the record is unchanged");
    assert!(files_in_sync_dir(&worker).is_empty(), "no file is written");
}

#[test]
fn initializing_refuses_when_a_base_already_exists() {
    let (_dir, worker) = worker();
    worker.initialize_base_image(5).unwrap();
    let path = base_image_path(&worker.path);
    let before = std::fs::read(&path).unwrap();

    let error = worker.initialize_base_image(6).unwrap_err();

    assert!(
        matches!(error, BaseImageError::BaseAlreadyExists),
        "{error:?}"
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(base_row(&worker).0, 5);
}

#[test]
fn a_negative_seq_is_refused() {
    let (_dir, worker) = worker();
    assert!(matches!(
        worker.initialize_base_image(-1),
        Err(BaseImageError::NegativeSeq)
    ));
    worker.initialize_base_image(0).unwrap();
    assert!(matches!(
        worker.advance_base_image(-1),
        Err(BaseImageError::NegativeSeq)
    ));
}

#[test]
fn advancing_needs_an_empty_outbox_and_moves_the_base_forward() {
    let (_dir, worker) = worker();
    worker.initialize_base_image(5).unwrap();
    let path = base_image_path(&worker.path);
    let before = std::fs::read(&path).unwrap();

    // An unpushed command blocks the advance: the file and the record stay as they were.
    seed(&worker);
    let error = worker.advance_base_image(6).unwrap_err();
    assert!(
        matches!(error, BaseImageError::OutboxNotEmpty { pending: 1 }),
        "{error:?}"
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(base_row(&worker).0, 5);

    // Once the command is pushed and accepted (the row leaves the outbox), the base advances and
    // now holds that command's effects.
    clear_outbox(&worker);
    let capture = worker.advance_base_image(9).unwrap();
    assert_eq!(capture.base_seq, 9);
    assert_ne!(std::fs::read(&path).unwrap(), before);
    let (seq, digest, state) = base_row(&worker);
    assert_eq!((seq, state.as_str()), (9, "Ready"));
    assert_eq!(
        digest.as_deref(),
        Some(sha256_file(&path).unwrap().as_slice())
    );
    let image = open_image(&worker, &path);
    assert_eq!(
        count_in(&image, "accounts"),
        live_count(&worker, "accounts")
    );
    drop(image);
    assert_eq!(files_in_sync_dir(&worker), vec!["base.db".to_owned()]);
}

#[test]
fn advancing_needs_sync_on_a_usable_base_and_a_head_that_does_not_go_back() {
    let (_dir, worker) = worker();
    assert!(matches!(
        worker.advance_base_image(1),
        Err(BaseImageError::SyncNotEnabled)
    ));
    worker.initialize_base_image(10).unwrap();
    let error = worker.advance_base_image(9).unwrap_err();
    assert!(
        matches!(
            error,
            BaseImageError::SeqWentBack {
                current: 10,
                requested: 9
            }
        ),
        "{error:?}"
    );
    // The same head is fine: nothing new was pulled, the local state may have moved.
    worker.advance_base_image(10).unwrap();

    // A swap in flight, or a base that was lost, is not advanced.
    worker
        .lock()
        .conn
        .execute("UPDATE sync_base SET state = 'Rebasing'", [])
        .unwrap();
    assert!(matches!(
        worker.advance_base_image(11),
        Err(BaseImageError::NoUsableBase(SyncBaseState::Rebasing))
    ));
}

#[test]
fn the_outbox_check_and_the_copy_are_one_critical_section_and_the_strip_is_not() {
    let (_dir, worker) = worker();
    let worker = Arc::new(worker);
    worker.initialize_base_image(1).unwrap();
    let accounts_before = live_count(&worker, "accounts");

    // A writer on another thread is blocked at the check and at the end of the copy (both under
    // one hold of the writer lock), and has been let through by the time the strip starts: the
    // credentials are stripped and the file hashed with the writer free.
    let finished = Arc::new(AtomicBool::new(false));
    let writer: Arc<Mutex<Option<std::thread::JoinHandle<()>>>> = Arc::new(Mutex::new(None));
    let blocked_at: Arc<Mutex<Vec<(String, bool)>>> = Arc::new(Mutex::new(Vec::new()));
    {
        let (worker, finished, writer, blocked_at) = (
            Arc::clone(&worker),
            Arc::clone(&finished),
            Arc::clone(&writer),
            Arc::clone(&blocked_at),
        );
        observe::install(move |label| {
            let mut slot = writer.lock().unwrap();
            if slot.is_none() {
                let (worker, finished) = (Arc::clone(&worker), Arc::clone(&finished));
                *slot = Some(std::thread::spawn(move || {
                    worker.dispatch(meta(), create_account_cmd()).unwrap();
                    finished.store(true, Ordering::SeqCst);
                }));
            }
            std::thread::sleep(Duration::from_millis(150));
            blocked_at
                .lock()
                .unwrap()
                .push((label.to_owned(), !finished.load(Ordering::SeqCst)));
        });
    }
    worker.advance_base_image(2).unwrap();
    observe::clear();

    let seen = blocked_at.lock().unwrap().clone();
    assert_eq!(
        seen,
        vec![
            ("checked".to_owned(), true),
            ("copied".to_owned(), true),
            ("stripping".to_owned(), false),
        ]
    );
    writer.lock().unwrap().take().unwrap().join().unwrap();
    // The write landed after the copy, so the base does not contain it and the outbox holds it.
    let image = open_image(&worker, &base_image_path(&worker.path));
    assert_eq!(count_in(&image, "accounts"), accounts_before);
    drop(image);
    assert_eq!(live_count(&worker, "accounts"), accounts_before + 1);
    assert_eq!(live_count(&worker, "sync_outbox"), 1);
}

#[test]
fn a_base_never_holds_a_credential_and_the_copy_has_no_free_pages() {
    let (_dir, worker) = worker();
    let id = connection(&worker, "tok-live-secret");
    assert_eq!(live_count(&worker, "connector_connection_secrets"), 1);
    // A credential replaced in the live vault leaves the old value in the live file's free space.
    worker
        .set_connector_credential(id, "tok-rotated-secret")
        .unwrap();

    worker.initialize_base_image(1).unwrap();

    let image = open_image(&worker, &base_image_path(&worker.path));
    for table in SECRET_BEARING_TABLES {
        assert_eq!(count_in(&image, table), 0, "{table} is emptied");
    }
    let free: i64 = image
        .query_row("PRAGMA freelist_count", [], |r| r.get(0))
        .unwrap();
    assert_eq!(free, 0, "the copy is vacuumed");
    // The connection's identity (class 1) is carried; only the device-local secret is not.
    assert_eq!(count_in(&image, "connector_connections"), 1);
    drop(image);
    // The live vault keeps its credential.
    assert_eq!(live_count(&worker, "connector_connection_secrets"), 1);
}

#[test]
fn a_credential_the_live_vault_once_held_is_not_left_in_a_free_page_of_the_base() {
    let (_dir, worker) = worker();
    // Large credentials overflow into their own pages; forgetting the connections frees those
    // pages in the live file, where the old values stay readable under the key.
    let ids: Vec<Uuid> = (0..12)
        .map(|n| connection(&worker, &format!("tok-{n}-{}", "x".repeat(20_000))))
        .collect();
    for id in &ids {
        worker.delete_connector_connection(*id).unwrap();
    }
    let live_free: i64 = worker
        .lock()
        .conn
        .query_row("PRAGMA freelist_count", [], |r| r.get(0))
        .unwrap();
    assert!(live_free > 0, "the live file holds freed pages");
    worker.checkpoint_truncate().unwrap();
    let live_len = std::fs::metadata(&worker.path).unwrap().len();

    worker.initialize_base_image(1).unwrap();

    let path = base_image_path(&worker.path);
    let image = open_image(&worker, &path);
    let free: i64 = image
        .query_row("PRAGMA freelist_count", [], |r| r.get(0))
        .unwrap();
    assert_eq!(free, 0, "no freed page survives in the base");
    assert!(
        std::fs::metadata(&path).unwrap().len() < live_len,
        "the base is compacted"
    );
}

#[test]
fn a_restored_backup_has_no_base_and_never_rebases_on_a_stale_one() {
    let (dir, worker) = worker();
    seed(&worker);
    worker.initialize_base_image(7).unwrap();
    // A backup is vault.db: it carries the sync_base row and not the sync/ directory.
    let bytes = worker.snapshot_bytes().unwrap();

    let restored_path: PathBuf = dir.path().join("restored").join("test.vault");
    std::fs::create_dir_all(restored_path.parent().unwrap()).unwrap();
    std::fs::write(&restored_path, &bytes).unwrap();
    let restored = DbWorker::open(&restored_path, KEY).unwrap();

    assert_eq!(
        restored.base_image_status().unwrap(),
        BaseImageStatus::NoBase
    );
    // The open recorded it: no digest, and the state says there is no base.
    let (_, digest, state) = base_row(&restored);
    assert_eq!((digest, state.as_str()), (None, "NoBase"));
    assert!(matches!(
        restored.advance_base_image(8),
        Err(BaseImageError::NoUsableBase(SyncBaseState::NoBase))
    ));
    // It can be rebuilt (the pull-first rebootstrap's last step) once nothing is unpushed.
    restored.initialize_base_image(8).unwrap();
    assert_eq!(
        restored.base_image_status().unwrap(),
        BaseImageStatus::Ready { base_seq: 8 }
    );
}

#[test]
fn a_base_that_does_not_match_its_digest_is_no_base_on_open() {
    let (dir, worker) = worker();
    worker.initialize_base_image(7).unwrap();
    let path = worker.path.clone();
    drop(worker);

    // Another vault's base image copied over this one: same place, wrong bytes.
    let other = dir.path().join("other.vault");
    let other_worker = DbWorker::open(&other, KEY).unwrap();
    other_worker.initialize_base_image(1).unwrap();
    std::fs::copy(base_image_path(&other_worker.path), base_image_path(&path)).unwrap();
    drop(other_worker);

    let reopened = DbWorker::open(&path, KEY).unwrap();
    assert_eq!(
        reopened.base_image_status().unwrap(),
        BaseImageStatus::NoBase
    );
    assert_eq!(base_row(&reopened).2, "NoBase");
}

#[test]
fn an_image_left_beside_a_vault_with_sync_off_is_removed_on_open() {
    let (_dir, worker) = worker();
    let path = worker.path.clone();
    std::fs::create_dir_all(crate::base_image::sync_dir(&path)).unwrap();
    std::fs::write(base_image_path(&path), b"an abandoned capture").unwrap();
    drop(worker);

    let reopened = DbWorker::open(&path, KEY).unwrap();
    assert_eq!(
        reopened.base_image_status().unwrap(),
        BaseImageStatus::SyncOff
    );
    assert!(!base_image_path(&path).exists());
}

#[test]
fn the_pre_rebase_scrub_removes_only_the_deleted_connections_credential() {
    let (_dir, worker) = worker();
    // Large values overflow into pages of their own, so a scrub that only deleted the row would
    // leave the removed token in a free page.
    let gone = connection(&worker, &format!("tok-removed-{}", "x".repeat(20_000)));
    let kept = connection(&worker, "tok-kept-connection");
    // A rebase swap (bxsbz.7) keeps the previous live vault as pre-rebase.db.
    std::fs::create_dir_all(crate::base_image::sync_dir(&worker.path)).unwrap();
    std::fs::write(
        pre_rebase_path(&worker.path),
        worker.snapshot_bytes().unwrap(),
    )
    .unwrap();

    worker.delete_connector_connection(gone).unwrap();

    let image = open_image(&worker, &pre_rebase_path(&worker.path));
    let ids: Vec<Uuid> = image
        .prepare("SELECT connection_id FROM connector_connection_secrets")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(ids, vec![kept]);
    let free: i64 = image
        .query_row("PRAGMA freelist_count", [], |r| r.get(0))
        .unwrap();
    assert_eq!(free, 0, "the removed value is not left in a free page");
}

#[test]
fn the_tombstone_sweep_repeats_a_pre_rebase_scrub_that_did_not_happen() {
    let (_dir, worker) = worker();
    let gone = connection(&worker, "tok-removed-connection");
    worker.delete_connector_connection(gone).unwrap();
    // The image appears after the delete (a swap that completed later), still holding a token.
    let conn = worker.lock();
    conn.conn
        .execute(
            "INSERT INTO connector_connection_secrets (connection_id, credential, updated_at)
             VALUES (?1, 'tok-old', 't')",
            [gone],
        )
        .unwrap();
    drop(conn);
    std::fs::create_dir_all(crate::base_image::sync_dir(&worker.path)).unwrap();
    std::fs::write(
        pre_rebase_path(&worker.path),
        worker.snapshot_bytes().unwrap(),
    )
    .unwrap();
    worker.scrub_tombstoned_connector_secrets().unwrap();

    let image = open_image(&worker, &pre_rebase_path(&worker.path));
    assert_eq!(count_in(&image, "connector_connection_secrets"), 0);
}

/// A raw-key worker (the production keying) and the DEK a rotation would move it to.
fn raw_worker() -> (TempDir, DbWorker, Dek) {
    let dir = TempDir::new().unwrap();
    let worker = DbWorker::open_with_raw_key(
        dir.path().join("vault.db"),
        vault_crypto::generate_dek().unwrap(),
    )
    .unwrap();
    (dir, worker, vault_crypto::generate_dek().unwrap())
}

/// The rekeyed copy of the vault, as a rotation's prepare builds it, and its `sync_base` row.
fn rekeyed_vault(worker: &DbWorker, new_dek: &Dek, dir: &TempDir) -> PathBuf {
    let new_db = dir.path().join("vault.db.rekey-new");
    worker.prepare_rekeyed_copy(new_dek, &new_db).unwrap();
    new_db
}

fn recorded_in(new_db: &std::path::Path, new_dek: &Dek) -> (Option<Vec<u8>>, String) {
    let conn = crate::open_vault_connection(new_db, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    crate::key_raw(&conn, new_dek).unwrap();
    conn.query_row(
        "SELECT base_digest, state FROM sync_base WHERE singleton = 1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .unwrap()
}

#[test]
fn a_rekeyed_base_records_its_new_digest_in_the_rekeyed_vault() {
    let (dir, worker, new_dek) = raw_worker();
    let capture = worker.initialize_base_image(4).unwrap();
    let new_db = rekeyed_vault(&worker, &new_dek, &dir);

    let images = worker.prepare_rekeyed_images(&new_dek, &new_db).unwrap();

    let copy = crate::base_image::base_image_path(&worker.path).with_extension("db.rekey-new");
    let digest = sha256_file(&copy).unwrap();
    assert_eq!(
        images,
        vec![RekeyedImage {
            file: "base.db",
            new_sha256: Some(digest)
        }]
    );
    assert_ne!(digest, capture.digest, "re-encrypted bytes");
    assert_eq!(
        recorded_in(&new_db, &new_dek),
        (Some(digest.to_vec()), "Ready".to_owned())
    );
    // The live vault and its image are untouched until the swap.
    assert_eq!(
        base_row(&worker).1.as_deref(),
        Some(capture.digest.as_slice())
    );
    assert_eq!(
        sha256_file(&base_image_path(&worker.path)).unwrap(),
        capture.digest
    );
    // The copy is a vault under the NEW key only.
    let probe = |dek: &Dek| DbWorker::probe_raw_key(&copy, dek).unwrap().is_some();
    assert!(probe(&new_dek));
    assert!(!probe(worker.dek().unwrap()));
}

#[test]
fn a_base_the_vault_could_not_vouch_for_is_carried_over_but_stays_unvouched() {
    let (dir, worker, new_dek) = raw_worker();
    worker.initialize_base_image(4).unwrap();
    // The recorded digest does not match the file (a restore, or a half-finished swap).
    worker
        .lock()
        .conn
        .execute("UPDATE sync_base SET base_digest = zeroblob(32)", [])
        .unwrap();
    let new_db = rekeyed_vault(&worker, &new_dek, &dir);

    let images = worker.prepare_rekeyed_images(&new_dek, &new_db).unwrap();

    assert!(
        images[0].new_sha256.is_some(),
        "still re-encrypted: no old-key copy survives"
    );
    assert_eq!(
        recorded_in(&new_db, &new_dek),
        (Some(vec![0u8; 32]), "Ready".to_owned()),
        "a lost base is not made good by a rotation"
    );
}

#[test]
fn an_unreadable_base_is_dropped_and_the_rekeyed_vault_says_no_base() {
    let (dir, worker, new_dek) = raw_worker();
    worker.initialize_base_image(4).unwrap();
    let path = base_image_path(&worker.path);
    std::fs::write(&path, b"not a database").unwrap();
    worker
        .lock()
        .conn
        .execute(
            "UPDATE sync_base SET base_digest = ?1",
            [sha256_file(&path).unwrap().as_slice()],
        )
        .unwrap();
    let new_db = rekeyed_vault(&worker, &new_dek, &dir);

    let images = worker.prepare_rekeyed_images(&new_dek, &new_db).unwrap();

    assert_eq!(
        images,
        vec![RekeyedImage {
            file: "base.db",
            new_sha256: None
        }]
    );
    assert_eq!(recorded_in(&new_db, &new_dek), (None, "NoBase".to_owned()));
    assert!(!path.with_extension("db.rekey-new").exists());
}

#[test]
fn a_leftover_image_rekey_copy_is_refused_not_overwritten() {
    let (dir, worker, new_dek) = raw_worker();
    worker.initialize_base_image(4).unwrap();
    let new_db = rekeyed_vault(&worker, &new_dek, &dir);
    let leftover = base_image_path(&worker.path).with_extension("db.rekey-new");
    std::fs::write(&leftover, b"previous attempt").unwrap();

    assert!(matches!(
        worker.prepare_rekeyed_images(&new_dek, &new_db),
        Err(DbError::InvalidCommand(_))
    ));
    assert_eq!(std::fs::read(&leftover).unwrap(), b"previous attempt");
}

#[test]
fn a_passphrase_vault_cannot_be_rekeyed_so_its_images_are_left_alone() {
    let (_dir, worker) = worker();
    worker.initialize_base_image(4).unwrap();
    let new_dek = vault_crypto::generate_dek().unwrap();
    assert!(matches!(
        worker.prepare_rekeyed_images(&new_dek, std::path::Path::new("unused")),
        Err(DbError::KeyUnavailable)
    ));
}

#[test]
fn a_reader_that_pins_an_old_snapshot_refuses_the_copy_instead_of_missing_frames() {
    let (_dir, worker) = worker();
    worker.initialize_base_image(1).unwrap();
    let path = base_image_path(&worker.path);
    let before = std::fs::read(&path).unwrap();

    // Frames committed after the reader's snapshot cannot be folded into the main file, so a copy
    // of that file would silently lack them. The capture says so instead.
    let reader = worker.read_connection().unwrap();
    reader.execute_batch("BEGIN;").unwrap();
    let _: i64 = reader
        .query_row("SELECT COUNT(*) FROM accounts", [], |r| r.get(0))
        .unwrap();
    worker
        .set_device_setting("device.after_reader", "v")
        .unwrap();

    let error = worker.advance_base_image(2).unwrap_err();

    assert!(
        matches!(error, BaseImageError::Db(DbError::CheckpointBusy)),
        "{error:?}"
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(base_row(&worker).0, 1);
    assert_eq!(files_in_sync_dir(&worker), vec!["base.db".to_owned()]);
}

/// The per-push cost of keeping the base current, for bxsbz.7's perf budget. Not a check:
/// `cargo test -p db-worker --lib measure_the_base_copy_cost -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement, not a check"]
fn measure_the_base_copy_cost() {
    for megabytes in [10_i64, 50, 200] {
        let (_dir, worker) = worker();
        {
            let guard = worker.lock();
            guard
                .conn
                .execute_batch("CREATE TABLE perf_filler (id INTEGER PRIMARY KEY, payload BLOB);")
                .unwrap();
            let tx = guard.conn.unchecked_transaction().unwrap();
            for _ in 0..megabytes * 1024 {
                tx.execute(
                    "INSERT INTO perf_filler (payload) VALUES (randomblob(1024))",
                    [],
                )
                .unwrap();
            }
            tx.commit().unwrap();
        }
        let first = worker.initialize_base_image(1).unwrap();
        let steady = worker.advance_base_image(2).unwrap();
        println!(
            "vault ~{megabytes} MB -> base {} bytes | first: copy {:?}, strip {:?}, seal {:?} | \
             steady: copy {:?}, strip {:?}, seal {:?}",
            steady.image_bytes,
            first.copy,
            first.strip,
            first.seal,
            steady.copy,
            steady.strip,
            steady.seal
        );
    }
}

#[test]
fn a_base_record_that_moves_during_the_capture_abandons_it_and_leaves_no_file() {
    let (_dir, worker) = worker();
    let worker = Arc::new(worker);
    worker.initialize_base_image(1).unwrap();
    let path = base_image_path(&worker.path);
    let before = std::fs::read(&path).unwrap();
    {
        let worker = Arc::clone(&worker);
        observe::install(move |label| {
            if label == "stripping" {
                // The record moves while the copy is being stripped with the writer free.
                worker
                    .lock()
                    .conn
                    .execute("UPDATE sync_base SET base_seq = 50", [])
                    .unwrap();
            }
        });
    }
    let error = worker.advance_base_image(2).unwrap_err();
    observe::clear();

    assert!(
        matches!(error, BaseImageError::Db(DbError::InvalidCommand(_))),
        "{error:?}"
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "the base is untouched"
    );
    assert_eq!(files_in_sync_dir(&worker), vec!["base.db".to_owned()]);
    assert_eq!(base_row(&worker).0, 50);
}
