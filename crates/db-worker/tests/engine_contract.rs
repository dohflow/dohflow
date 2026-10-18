//! Linked engine provenance, complete frozen SQL state, and deterministic WAL
//! checkpoint boundaries. No real data, timing race or stress-loop assertion.
mod common;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use common::{create_account_cmd, meta, worker};
use db_worker::{DbWorker, WriteCommand};
use rusqlite::{types::ValueRef, Connection, OpenFlags};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{fs, io::Write, path::Path};
use vault_crypto::{derive_kek, unwrap_dek, VaultEnvelope};

/// The linked engine's `sqlite_source_id()` — SQLite 3.51.3 as packaged in
/// SQLCipher 4.14.0 community (`libsqlite3-sys` 0.38.2; amalgamation SHA-256
/// `ea0bf0b08f688ca5d9312b2e33e7f81b3f4ae54b5016fb062ae1f2632a30a1b9`). Must
/// match `docs/architecture/stack.md`.
const SOURCE_ID: &str =
    "2026-03-13 10:38:09 737ae4a34738ffa0c3ff7f9bb18df914dd1cad163f28fd6b6e114a344fe6alt1";
const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../finance-kernel/tests/fixtures/sqlcipher-4.5.7.json"
);
const STATE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/sqlcipher-4.5.7-state.json"
);

fn decode(artifact: &Value) -> Vec<u8> {
    let bytes = STANDARD
        .decode(artifact["base64"].as_str().unwrap())
        .unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(&bytes)),
        artifact["sha256"].as_str().unwrap()
    );
    bytes
}

fn fixture_connection(root: &Path, with_wal: bool) -> Connection {
    let corpus: Value = serde_json::from_slice(&fs::read(FIXTURE).unwrap()).unwrap();
    let files = corpus["files"].as_object().unwrap();
    fs::write(root.join("vault.db"), decode(&files["vault.db"])).unwrap();
    if with_wal {
        fs::write(root.join("vault.db-wal"), decode(&files["vault.db-wal"])).unwrap();
    }
    let envelope = VaultEnvelope::from_bytes(&decode(&files["vault.db.envelope"])).unwrap();
    let kek = derive_kek(
        b"synthetic outgoing engine compatibility password",
        &envelope.salt,
        &envelope.kdf,
    )
    .unwrap();
    let dek = unwrap_dek(&kek, &envelope.wrapped).unwrap();
    let key: String = dek
        .expose_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let conn = Connection::open_with_flags(root.join("vault.db"), OpenFlags::SQLITE_OPEN_READ_ONLY)
        .unwrap();
    conn.pragma_update(None, "key", format!("x'{key}'"))
        .unwrap();
    conn
}

fn sql_state(conn: &Connection) -> Value {
    let tables: Vec<String> = conn.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap()
        .query_map([], |row| row.get(0)).unwrap().map(Result::unwrap).collect();
    let mut state = serde_json::Map::new();
    for table in tables {
        let name = table.replace('"', "\"\"");
        let mut stmt = conn.prepare(&format!("SELECT * FROM \"{name}\"")).unwrap();
        let columns = stmt
            .column_names()
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>();
        let mut records: Vec<Value> = stmt
            .query_map([], |row| {
                let values: Vec<Value> = (0..columns.len())
                    .map(|column| match row.get_ref(column).unwrap() {
                        ValueRef::Null => json!(["null"]),
                        ValueRef::Integer(n) => json!(["integer", n]),
                        ValueRef::Real(n) => json!(["real", n]),
                        ValueRef::Text(bytes) => json!(["text", STANDARD.encode(bytes)]),
                        ValueRef::Blob(bytes) => json!(["blob", STANDARD.encode(bytes)]),
                    })
                    .collect();
                Ok(json!(values))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect();
        records.sort_by_cached_key(Value::to_string);
        state.insert(table, json!({"columns": columns, "records": records}));
    }
    json!({"tables": state, "user_version": conn.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0)).unwrap()})
}

#[test]
fn linked_sqlcipher_source_and_provider_match_the_pin() {
    let (_dir, worker) = worker();
    let conn = worker.read_connection().unwrap();
    let source: String = conn
        .query_row("SELECT sqlite_source_id()", [], |row| row.get(0))
        .unwrap();
    assert_eq!(source, SOURCE_ID);
    let provider: String = conn
        .pragma_query_value(None, "cipher_provider", |row| row.get(0))
        .unwrap();
    assert_eq!(provider, "openssl");
    println!(
        "Linked SQLCipher={}, SQLite={}, source={}, provider={}",
        worker.cipher_version().unwrap(),
        rusqlite::version(),
        source,
        provider
    );
}

#[test]
#[ignore = "manual outgoing SQL-state capture; requires G3M5_STATE_PATH; never overwrites"]
fn capture_outgoing_sql_state() {
    assert_eq!(rusqlite::version(), "3.45.3");
    let mut state = serde_json::Map::new();
    for (name, wal) in [("settled", false), ("committed_wal", true)] {
        let dir = tempfile::tempdir().unwrap();
        let conn = fixture_connection(dir.path(), wal);
        assert_eq!(
            conn.query_row("SELECT sqlite_source_id()", [], |row| row
                .get::<_, String>(0))
                .unwrap(),
            SOURCE_ID
        );
        state.insert(name.into(), sql_state(&conn));
    }
    let output = std::env::var_os("G3M5_STATE_PATH").expect("explicit output path required");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .unwrap();
    file.write_all(&serde_json::to_vec_pretty(&state).unwrap())
        .unwrap();
    file.sync_all().unwrap();
}

#[test]
fn frozen_canonical_tables_audit_and_migration_history_are_identical() {
    let expected: Value = serde_json::from_slice(&fs::read(STATE).unwrap()).unwrap();
    for (name, wal) in [("settled", false), ("committed_wal", true)] {
        let dir = tempfile::tempdir().unwrap();
        let conn = fixture_connection(dir.path(), wal);
        assert_eq!(sql_state(&conn), expected[name]);
        let integrity: String = conn
            .pragma_query_value(None, "integrity_check", |row| row.get(0))
            .unwrap();
        assert_eq!(integrity, "ok");
        assert!(!conn
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .exists([])
            .unwrap());
    }
}

#[test]
fn a_reader_snapshot_bounds_checkpoint_without_losing_commits() {
    let (dir, worker) = worker();
    let command = create_account_cmd();
    let id = match &command {
        WriteCommand::CreateAccount { account, .. } => account.id(),
        _ => unreachable!(),
    };
    worker.dispatch(meta(), command).unwrap();
    let checkpoint = worker.read_connection().unwrap();
    checkpoint
        .execute_batch("PRAGMA wal_autocheckpoint=0; PRAGMA wal_checkpoint(TRUNCATE)")
        .unwrap();
    let reader = worker.read_connection().unwrap();
    reader.execute_batch("BEGIN DEFERRED").unwrap();
    assert_eq!(
        reader
            .query_row(
                "SELECT name FROM accounts WHERE id=?1",
                [id.as_uuid()],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "Checking"
    );
    worker
        .dispatch(
            meta(),
            WriteCommand::UpdateAccount {
                id,
                name: "Synthetic WAL commit".into(),
            },
        )
        .unwrap();
    // PASSIVE cannot backfill the commit while this earlier read snapshot lives.
    let (busy, frames, copied): (i64, i64, i64) = checkpoint
        .query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .unwrap();
    assert_eq!(busy, 0);
    assert!(frames > copied);
    assert_eq!(
        reader
            .query_row(
                "SELECT name FROM accounts WHERE id=?1",
                [id.as_uuid()],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "Checking"
    );
    reader.execute_batch("ROLLBACK").unwrap();
    let fresh = worker.read_connection().unwrap();
    assert_eq!(
        fresh
            .query_row(
                "SELECT name FROM accounts WHERE id=?1",
                [id.as_uuid()],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "Synthetic WAL commit"
    );
    let before = sql_state(&fresh);
    let result: (i64, i64, i64) = checkpoint
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .unwrap();
    assert_eq!(result, (0, 0, 0));
    assert_eq!(sql_state(&fresh), before);
    drop(fresh);
    drop(checkpoint);
    drop(reader);
    drop(worker);
    let reopened = DbWorker::open(dir.path().join("test.vault"), common::KEY).unwrap();
    assert!(reopened.integrity_ok());
    assert_eq!(sql_state(&reopened.read_connection().unwrap()), before);
}
