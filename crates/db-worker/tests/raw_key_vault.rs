//! Raw-key (DEK) vault open + opacity (personal-cfo-vhv).
//!
//! These exercise [`DbWorker::open_with_raw_key`] — the production keying path
//! where SQLCipher is keyed directly from the 256-bit DEK rather than a
//! passphrase. The full create→lock→unlock→read round-trip through the real
//! envelope lives in `finance-kernel`; here we isolate db-worker.

use db_worker::DbWorker;
use vault_crypto::{
    derive_kek, generate_dek, generate_salt, unwrap_dek, wrap_dek, Argon2Params, ALGORITHM,
};

/// A cheap KDF profile so the test does not pay the full interactive cost.
fn cheap_params() -> Argon2Params {
    Argon2Params {
        algorithm: ALGORITHM,
        memory_kib: 64,
        time_cost: 1,
        parallelism: 1,
        version: 1,
    }
}

#[test]
fn raw_keyed_vault_persists_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.db");

    // Mint two byte-identical DEKs by unwrapping the same wrapped DEK twice
    // (Dek is intentionally non-Clone).
    let salt = generate_salt().unwrap();
    let kek = derive_kek(b"password", &salt, &cheap_params()).unwrap();
    let dek = generate_dek().unwrap();
    let wrapped = wrap_dek(&kek, &dek).unwrap();
    let dek_open = unwrap_dek(&kek, &wrapped).unwrap();
    let dek_reopen = unwrap_dek(&kek, &wrapped).unwrap();

    // First session: create + read the vault identity, then drop (lock).
    let vault_id = {
        let worker = DbWorker::open_with_raw_key(&path, dek_open).unwrap();
        worker.vault_metadata().unwrap().vault_id
    };

    // Second session with the same key: the DB decrypts and the persisted
    // identity is unchanged — proves the DEK correctly keyed SQLCipher.
    let worker = DbWorker::open_with_raw_key(&path, dek_reopen).unwrap();
    assert_eq!(worker.vault_metadata().unwrap().vault_id, vault_id);
}

#[test]
fn raw_keyed_vault_is_opaque_without_the_key() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.db");

    {
        let dek = generate_dek().unwrap();
        let _worker = DbWorker::open_with_raw_key(&path, dek).unwrap();
    }

    // A keyless connection (stock-SQLite behaviour) cannot read the encrypted
    // pages: the header doesn't parse as plaintext SQLite.
    let plain = rusqlite::Connection::open(&path).unwrap();
    let read: Result<i64, _> =
        plain.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0));
    assert!(
        read.is_err(),
        "encrypted vault must not be readable without the key"
    );
}
