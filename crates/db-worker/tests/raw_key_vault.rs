//! Raw-key (DEK) vault open + opacity (personal-cfo-vhv).
//!
//! These exercise [`DbWorker::open_with_raw_key`] — the production keying path
//! where SQLCipher is keyed directly from the 256-bit DEK rather than a
//! passphrase. The full create→lock→unlock→read round-trip through the real
//! envelope lives in `finance-kernel`; here we isolate db-worker.

use db_worker::{DbError, DbWorker, CURRENT_SCHEMA_VERSION};
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

#[test]
fn raw_key_existing_open_refuses_a_newer_stamp_without_lowering_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.db");
    let salt = generate_salt().unwrap();
    let kek = derive_kek(b"password", &salt, &cheap_params()).unwrap();
    let wrapped = wrap_dek(&kek, &generate_dek().unwrap()).unwrap();
    let future = CURRENT_SCHEMA_VERSION + 1;
    let worker = DbWorker::open_with_raw_key(&path, unwrap_dek(&kek, &wrapped).unwrap()).unwrap();
    worker
        .read_connection()
        .unwrap()
        .pragma_update(None, "user_version", future)
        .unwrap();
    drop(worker);

    assert!(matches!(
        DbWorker::open_existing_with_raw_key(&path, unwrap_dek(&kek, &wrapped).unwrap()),
        Err(DbError::NewerSchema { observed, supported })
            if observed == future && supported == CURRENT_SCHEMA_VERSION
    ));
    assert!(matches!(
        DbWorker::open_with_raw_key(&path, unwrap_dek(&kek, &wrapped).unwrap()),
        Err(DbError::NewerSchema { observed, supported })
            if observed == future && supported == CURRENT_SCHEMA_VERSION
    ));
}

#[test]
fn existing_raw_key_open_does_not_create_a_missing_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing.db");
    assert!(matches!(
        DbWorker::open_existing_with_raw_key(&path, generate_dek().unwrap()),
        Err(DbError::UnsupportedSchema)
    ));
    assert!(!path.exists());
}

/// SQLCipher's memory security must be on for a raw-keyed connection (personal-cfo-rsjud): it
/// wipes freed memory, including the copy of the key pragma's SQL text that SQLite keeps.
#[test]
fn a_raw_keyed_connection_has_cipher_memory_security_on() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.db");
    let worker = DbWorker::open_with_raw_key(&path, generate_dek().unwrap()).unwrap();
    let conn = worker.read_connection().unwrap();
    let on: String = conn
        .query_row("PRAGMA cipher_memory_security", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        on, "1",
        "memory security must be ON on every raw-keyed connection"
    );
}

/// The cost of memory security on the command path, so a regression shows up: 1,000 commands on a
/// raw-keyed vault. Printed for the PR; the guard is generous.
#[test]
fn memory_security_keeps_a_command_burst_inside_budget() {
    use std::time::Instant;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.db");
    let worker = DbWorker::open_with_raw_key(&path, generate_dek().unwrap()).unwrap();
    let start = Instant::now();
    for i in 0..1_000 {
        worker.set_setting(&format!("user.k{i}"), "v").unwrap();
    }
    let elapsed = start.elapsed();
    println!("1000 settings writes with memory security on: {elapsed:?}");
    assert!(elapsed.as_secs() < 5, "{elapsed:?}");
}

#[test]
fn a_passphrase_keyed_connection_has_cipher_memory_security_on() {
    let dir = tempfile::tempdir().unwrap();
    let worker =
        DbWorker::open(dir.path().join("vault.db"), "correct horse battery staple").unwrap();
    let on: String = worker
        .read_connection()
        .unwrap()
        .query_row("PRAGMA cipher_memory_security", [], |row| row.get(0))
        .unwrap();
    assert_eq!(on, "1");
}

/// No code outside the two keying helpers sends a key to SQLCipher, so the memory-security pragma
/// cannot be skipped at a new keying site.
#[test]
fn only_the_keying_helpers_send_a_key_pragma() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut hits = Vec::new();
    for entry in std::fs::read_dir(&src).unwrap().flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        for (n, line) in std::fs::read_to_string(&path).unwrap().lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            if code.contains("pragma_update(None, \"key\"") || code.contains("PRAGMA key") {
                hits.push(format!(
                    "{}:{}",
                    path.file_name().unwrap().to_string_lossy(),
                    n + 1
                ));
            }
        }
    }
    // Exactly the two senders: key_passphrase and key_raw, both in lib.rs.
    hits.sort();
    assert_eq!(
        hits.len(),
        2,
        "key pragmas outside the keying helpers: {hits:?}"
    );
    assert!(hits.iter().all(|h| h.starts_with("lib.rs:")), "{hits:?}");
}
