//! `VaultLease` (personal-cfo-bxsbz.2): process-exclusive ownership of a vault, which a worker
//! holds for its life and which can be kept across a close and handed to the next worker.

use db_worker::{DbError, DbWorker, VaultLease};
use vault_crypto::{generate_dek, unwrap_dek, wrap_dek, Dek};

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

fn in_use<T>(result: Result<T, DbError>) -> bool {
    matches!(result, Err(DbError::VaultInUse))
}

#[test]
fn a_lease_is_exclusive_and_released_on_drop() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.db");
    let lease = VaultLease::acquire(&path).unwrap();
    assert_eq!(lease.path(), path);
    assert!(in_use(VaultLease::acquire(&path)));
    drop(lease);
    assert!(VaultLease::acquire(&path).is_ok());
}

#[test]
fn a_worker_holds_the_lease_for_its_life() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.db");
    let worker = DbWorker::open_with_raw_key(&path, generate_dek().unwrap()).unwrap();
    assert!(in_use(VaultLease::acquire(&path)));
    drop(worker);
    assert!(VaultLease::acquire(&path).is_ok());
}

#[test]
fn closing_into_a_lease_keeps_the_vault_owned_until_the_lease_is_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.db");
    let dek = generate_dek().unwrap();
    let worker = DbWorker::open_with_raw_key(&path, duplicate(&dek)).unwrap();
    let lease = worker.close_into_lease();
    assert!(in_use(VaultLease::acquire(&path)));
    assert!(in_use(DbWorker::open_existing_with_raw_key(
        &path,
        duplicate(&dek)
    )));
    drop(lease);
    assert!(DbWorker::open_existing_with_raw_key(&path, dek).is_ok());
}

#[test]
fn a_held_lease_is_handed_to_the_next_worker_without_a_release() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.db");
    let dek = generate_dek().unwrap();
    let lease = DbWorker::open_with_raw_key(&path, duplicate(&dek))
        .unwrap()
        .close_into_lease();
    let worker = DbWorker::open_existing_with_raw_key_leased(&path, dek, lease).unwrap();
    // The new worker owns the vault: still refused to everyone else.
    assert!(in_use(VaultLease::acquire(&path)));
    drop(worker);
    assert!(VaultLease::acquire(&path).is_ok());
}

#[test]
fn a_lease_for_another_vault_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (path, other) = (dir.path().join("a.db"), dir.path().join("b.db"));
    let dek = generate_dek().unwrap();
    drop(DbWorker::open_with_raw_key(&path, duplicate(&dek)).unwrap());
    let wrong = VaultLease::acquire(&other).unwrap();
    match DbWorker::open_existing_with_raw_key_leased(&path, dek, wrong) {
        Err(DbError::InvalidCommand(_)) => {}
        Err(other) => panic!("expected InvalidCommand, got {other:?}"),
        Ok(_) => panic!("a lease for another vault must not open this one"),
    }
}

#[test]
fn a_lock_file_left_behind_never_blocks_a_new_lease() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.db");
    drop(VaultLease::acquire(&path).unwrap());
    let mut lock = path.as_os_str().to_owned();
    lock.push(".runner.lock");
    assert!(
        std::path::Path::new(&lock).exists(),
        "the file stays after release"
    );
    assert!(VaultLease::acquire(&path).is_ok());
}
