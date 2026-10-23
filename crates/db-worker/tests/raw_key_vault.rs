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

/// SQLCipher's broad memory security (`cipher_memory_security`) is never turned on
/// (personal-cfo-7ie.17, owner decision 2026-10-20): it cost 4x to 11x, it cannot be switched back
/// off once on, and it is not needed because a key never passes through SQL text. `PRAGMA
/// cipher_memory_security` reads 0 on a connection that never enabled it.
fn memory_security(worker: &DbWorker) -> String {
    worker
        .read_connection()
        .unwrap()
        .query_row("PRAGMA cipher_memory_security", [], |row| row.get(0))
        .unwrap()
}

#[test]
fn memory_security_is_never_enabled_on_a_raw_keyed_vault() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.db");
    let worker = DbWorker::open_with_raw_key(&path, generate_dek().unwrap()).unwrap();
    assert_eq!(memory_security(&worker), "0");
    // Real work on the vault does not turn it on either.
    worker.set_setting("user.theme", "dark").unwrap();
    assert_eq!(memory_security(&worker), "0");
}

#[test]
fn memory_security_is_never_enabled_on_a_passphrase_vault() {
    let dir = tempfile::tempdir().unwrap();
    let worker =
        DbWorker::open(dir.path().join("vault.db"), "correct horse battery staple").unwrap();
    assert_eq!(memory_security(&worker), "0");
}

/// A passphrase with quotes, non-ASCII text and SQL syntax round-trips: it is passed to SQLCipher
/// as bytes through `sqlite3_key_v2`, never as SQL text, so nothing needs escaping.
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

/// No key ever reaches SQL (personal-cfo-7ie.17): a key goes to SQLCipher only through
/// `sqlite3_key_v2`, called from exactly one place, so there is no statement text holding a key
/// for SQLite to copy and free unscrubbed, and no bound parameter either. The scan covers `src/`
/// and its subdirectories (test modules excluded) and fails on:
///
/// * `PRAGMA key` / `PRAGMA rekey`, with or without a schema qualifier, as SQL text;
/// * `ATTACH ... KEY`, with or without the `DATABASE` word, over one line or several;
/// * `pragma_update(None, "key" | "rekey", ..)`;
/// * the C calls `sqlite3_key(`, `sqlite3_rekey(` and `sqlite3_rekey_v2(`;
/// * anything mentioning `cipher_memory_security` (it must never be set);
/// * more or fewer than one call of `sqlite3_key_v2`, which must be in `lib.rs`.
#[test]
fn no_key_reaches_sql_and_only_one_place_calls_sqlite3_key_v2() {
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
    /// The token after each `PRAGMA` in `code`, up to whitespace, `=`, `(`, `;` or a quote.
    fn pragma_names(code: &str) -> Vec<String> {
        let upper = code.to_ascii_uppercase();
        let mut names = Vec::new();
        let mut from = 0;
        while let Some(at) = upper[from..].find("PRAGMA") {
            let start = from + at + "PRAGMA".len();
            let name: String = code[start..]
                .trim_start()
                .chars()
                .take_while(|c| !c.is_whitespace() && !matches!(c, '=' | '(' | ';' | '"' | '\\'))
                .collect();
            if !name.is_empty() {
                names.push(name.to_ascii_lowercase());
            }
            from = start;
        }
        names
    }
    /// The number of `ATTACH` statements in `code` with a `KEY` word before the literal ends.
    fn attach_key_statements(code: &str) -> usize {
        let upper = code.to_ascii_uppercase();
        let (mut count, mut from) = (0, 0);
        while let Some(at) = upper[from..].find("ATTACH") {
            let begin = from + at;
            let start = begin + "ATTACH".len();
            from = start;
            // A statement keyword, not part of an identifier like `attach_source_record`.
            let standalone = !upper[..begin]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
                && upper[start..].starts_with(' ');
            if !standalone {
                continue;
            }
            let statement: String = upper[start..].chars().take_while(|c| *c != '"').collect();
            if statement
                .split(|c: char| !c.is_ascii_alphanumeric())
                .any(|word| word == "KEY")
            {
                count += 1;
            }
        }
        count
    }
    // Workspace-wide: every crate's `src/` (finance-kernel and the desktop app depend on rusqlite
    // too) and the desktop app's own `src-tauri/src`, so "exactly one caller" holds everywhere.
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for entry in std::fs::read_dir(workspace.join("crates"))
        .unwrap()
        .flatten()
    {
        let src = entry.path().join("src");
        if src.is_dir() {
            walk(&src, &mut files);
        }
    }
    walk(&workspace.join("apps/desktop/src-tauri/src"), &mut files);
    assert!(files.len() > 100, "found {} source files", files.len());
    let mut findings = Vec::new();
    let mut key_v2_calls = Vec::new();
    for path in files {
        let relative = path.strip_prefix(&workspace).unwrap();
        let name = relative.display().to_string();
        let file_in_crate = relative
            .components()
            .skip(1)
            .collect::<std::path::PathBuf>();
        if file_in_crate.starts_with("src/tests")
            || name.ends_with("_tests.rs")
            || name.contains("/tests/")
        {
            continue;
        }
        // Comments out, then every run of whitespace (tabs and newlines included) to one space.
        let code: String = std::fs::read_to_string(&path)
            .unwrap()
            .lines()
            .map(|line| line.split("//").next().unwrap_or(""))
            .collect::<Vec<_>>()
            .join(" ")
            // A `\t`, `\n` or `\r` escape inside a string literal is whitespace in the SQL.
            .replace("\\t", " ")
            .replace("\\n", " ")
            .replace("\\r", " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        for banned in [
            "pragma_update(None, \"key\"",
            "pragma_update(None, \"rekey\"",
            "sqlite3_key(",
            "sqlite3_rekey(",
            "sqlite3_rekey_v2(",
            "cipher_memory_security",
        ] {
            if code.contains(banned) {
                findings.push(format!("{name}: {banned}"));
            }
        }
        for pragma in pragma_names(&code) {
            if pragma == "key"
                || pragma == "rekey"
                || pragma.ends_with(".key")
                || pragma.ends_with(".rekey")
            {
                findings.push(format!("{name}: PRAGMA {pragma}"));
            }
        }
        for _ in 0..attach_key_statements(&code) {
            findings.push(format!("{name}: ATTACH ... KEY"));
        }
        for _ in code.matches("sqlite3_key_v2(") {
            key_v2_calls.push(name.clone());
        }
    }
    assert!(findings.is_empty(), "a key could reach SQL: {findings:?}");
    assert_eq!(
        key_v2_calls,
        ["crates/db-worker/src/lib.rs"],
        "sqlite3_key_v2 must be called from exactly one place, key_connection in db-worker's lib.rs"
    );
}

/// Every connection to a vault is opened through `open_vault_connection`, which first waits for
/// SQLCipher to finish its start-up (personal-cfo-g3m.20); a new open path that bypassed it would
/// bring the cold-start race back. In non-test `src/` code (an inline `#[cfg(test)] mod ... {`
/// ends the scan of a file), `Connection::open_with_flags(` appears exactly once, inside that
/// function, and `Connection::open_in_memory(` exactly once, in the readiness probe itself. Plain
/// `Connection::open(` and any other opener appear nowhere.
#[test]
fn every_vault_connection_is_opened_through_the_readiness_gate() {
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
    /// The file's code up to its inline test module, with line comments removed.
    fn non_test_code(source: &str) -> String {
        let lines: Vec<&str> = source.lines().collect();
        let mut kept = Vec::new();
        for (n, line) in lines.iter().enumerate() {
            if *line == "#[cfg(test)]" {
                let next = lines.get(n + 1).copied().unwrap_or("");
                if next.starts_with("mod ") && next.trim_end().ends_with('{') {
                    break;
                }
            }
            kept.push(line.split("//").next().unwrap_or(""));
        }
        kept.join("\n")
    }
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    walk(&src, &mut files);
    let (mut with_flags, mut in_memory) = (Vec::new(), Vec::new());
    let mut other = Vec::new();
    for path in files {
        let name = path.strip_prefix(&src).unwrap().display().to_string();
        if name.starts_with("tests") || name.ends_with("_tests.rs") {
            continue;
        }
        let code = non_test_code(&std::fs::read_to_string(&path).unwrap());
        for _ in code.matches("Connection::open_with_flags(") {
            with_flags.push(name.clone());
        }
        for _ in code.matches("Connection::open_in_memory(") {
            in_memory.push(name.clone());
        }
        // Any other way to open: plain `Connection::open(`, a VFS variant, or a builder.
        for needle in [
            "Connection::open(",
            "Connection::open_with_flags_and_vfs(",
            "Connection::open_with_vfs(",
            "Connection::open_in_memory_with_flags(",
        ] {
            if code.contains(needle) {
                other.push(format!("{name}: {needle}"));
            }
        }
    }
    assert!(
        other.is_empty(),
        "a connection is opened outside the gate: {other:?}"
    );
    assert_eq!(
        with_flags,
        ["lib.rs"],
        "open_with_flags must appear only in open_vault_connection"
    );
    assert_eq!(
        in_memory,
        ["lib.rs"],
        "open_in_memory must appear only in the readiness probe"
    );
}
