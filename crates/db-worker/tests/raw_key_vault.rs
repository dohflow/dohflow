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

/// A passphrase that needs SQL escaping still opens the vault it created: the statement text is
/// built by hand, so quotes (and non-ASCII) must round-trip.
#[test]
fn a_passphrase_with_quotes_and_unicode_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.db");
    for passphrase in [
        "it's a \"quoted\" pass'phrase",
        "''",
        "'",
        "caf\u{e9} \u{1f512} 'x'",
        "a;b--c /* d */",
    ] {
        let _ = std::fs::remove_file(&path);
        {
            let worker = DbWorker::open(&path, passphrase).unwrap();
            worker.set_setting("user.round_trip", "yes").unwrap();
        }
        let reopened =
            DbWorker::open(&path, passphrase).unwrap_or_else(|e| panic!("{passphrase:?}: {e}"));
        assert!(reopened.read_connection().is_ok());
        // A different passphrase does not open it.
        assert!(DbWorker::open(&path, &format!("{passphrase}x")).is_err());
    }
}

/// Every statement that carries a key to SQLCipher is known, and the memory-security setting
/// cannot be skipped at a new one. A key-bearing statement is any of:
///
/// * `PRAGMA key` / `PRAGMA <schema>.key` / `PRAGMA rekey` / `PRAGMA <schema>.rekey`, as SQL text;
/// * `ATTACH DATABASE ... KEY ...`, which keys the attached database;
/// * `pragma_update(None, "key" | "rekey", ..)`, which builds the key into an unscrubbed `String`
///   and so is never allowed.
///
/// The allowed sites, exactly: the two keying helpers (`key_passphrase` and `raw_key_pragma`, both
/// in `lib.rs`) and the rekey copy's `ATTACH` in `rekey.rs`, which runs on the live writer
/// connection (already keyed through `key_raw`, so memory security is on for the statement text)
/// and builds its key literal in a `Zeroizing` buffer. Anything else, a new `PRAGMA rekey` or a
/// second `ATTACH ... KEY` included, fails here and gets reviewed. The scan covers `src/` and its
/// subdirectories (test modules excluded).
#[test]
fn only_the_known_statements_carry_a_key() {
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }
    /// The token after each `PRAGMA ` in `code`, up to whitespace, `=`, `(` or `;`.
    fn pragma_names(code: &str) -> Vec<String> {
        let upper = code.to_ascii_uppercase();
        let mut names = Vec::new();
        let mut from = 0;
        while let Some(at) = upper[from..].find("PRAGMA ") {
            let start = from + at + "PRAGMA ".len();
            let name: String = code[start..]
                .trim_start()
                .chars()
                .take_while(|c| !c.is_whitespace() && !matches!(c, '=' | '(' | ';' | '"' | '\\'))
                .collect();
            names.push(name.to_ascii_lowercase());
            from = start;
        }
        names
    }
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    walk(&src, &mut files);
    assert!(files.len() > 20, "found {} source files", files.len());
    let (mut keys, mut attaches, mut pragma_update_key) = (Vec::new(), Vec::new(), Vec::new());
    for path in files {
        let name = path.strip_prefix(&src).unwrap().display().to_string();
        if name.starts_with("tests") || name.ends_with("_tests.rs") {
            continue;
        }
        for (n, line) in std::fs::read_to_string(&path).unwrap().lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            let site = format!("{name}:{}", n + 1);
            if code.contains("pragma_update(None, \"key\"")
                || code.contains("pragma_update(None, \"rekey\"")
            {
                pragma_update_key.push(site.clone());
            }
            for pragma in pragma_names(code) {
                if pragma == "key"
                    || pragma == "rekey"
                    || pragma.ends_with(".key")
                    || pragma.ends_with(".rekey")
                {
                    keys.push(format!("{site} (PRAGMA {pragma})"));
                }
            }
            let upper = code.to_ascii_uppercase();
            if upper.contains("ATTACH DATABASE")
                && upper
                    .split(|c: char| !c.is_ascii_alphanumeric())
                    .any(|word| word == "KEY")
            {
                attaches.push(site);
            }
        }
    }
    assert!(
        pragma_update_key.is_empty(),
        "pragma_update builds the key into an unscrubbed String: {pragma_update_key:?}"
    );
    keys.sort();
    assert_eq!(
        keys.len(),
        2,
        "PRAGMA key statements outside the keying helpers: {keys:?}"
    );
    assert!(
        keys.iter()
            .all(|h| h.starts_with("lib.rs:") && h.ends_with("(PRAGMA key)")),
        "{keys:?}"
    );
    assert_eq!(
        attaches.len(),
        1,
        "ATTACH ... KEY outside the rekey copy: {attaches:?}"
    );
    assert!(attaches[0].starts_with("rekey.rs:"), "{attaches:?}");
}

/// Memory security makes SQLCipher warn, per allocation, when `mlock` is refused (a small
/// `RLIMIT_MEMLOCK`); the warning flood starved a CI runner. Every keyed connection runs with the
/// log threshold at ERROR, so that best-effort warning is not printed while real errors still are.
#[test]
fn a_keyed_connection_does_not_log_sqlcipher_warnings() {
    let dir = tempfile::tempdir().unwrap();
    let raw =
        DbWorker::open_with_raw_key(dir.path().join("raw.vault"), generate_dek().unwrap()).unwrap();
    let pass = DbWorker::open(
        dir.path().join("pass.vault"),
        "correct horse battery staple",
    )
    .unwrap();
    for (label, worker) in [("raw", &raw), ("passphrase", &pass)] {
        let level: String = worker
            .read_connection()
            .unwrap()
            .query_row("PRAGMA cipher_log_level", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            level, "ERROR",
            "{label}: WARN is the SQLCipher default and floods stderr"
        );
    }
}
