//! `SetHouseholdTimezone` (personal-cfo-g3m.8; `docs/architecture/vault-format.md`
//! B.2 rule 6): the household timezone changes the forecast horizon boundary, so it is
//! a class-1 logical field that changes only through a logged command.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::*;
use db_worker::{new_idempotency_key, DbError, Outcome, WriteCommand};
use rusqlite::params;

const EVENT: &str = "household_timezone_changed";

fn set(timezone: &str) -> WriteCommand {
    WriteCommand::SetHouseholdTimezone {
        timezone: timezone.to_owned(),
    }
}

#[test]
fn a_change_goes_through_the_bus_and_logs_an_operation_and_an_audit_event() {
    let (_dir, worker) = worker();
    assert_eq!(worker.vault_metadata().unwrap().household_timezone, "UTC");
    let before = worker.operation_count().unwrap();

    let m = meta();
    let outcome = worker.dispatch(m.clone(), set("America/Chicago")).unwrap();
    assert!(matches!(outcome, Outcome::Applied { .. }), "{outcome:?}");

    let metadata = worker.vault_metadata().unwrap();
    assert_eq!(metadata.household_timezone, "America/Chicago");
    assert_eq!(worker.operation_count().unwrap(), before + 1);
    assert_eq!(worker.audit_event_count(EVENT).unwrap(), 1);

    // The operation-log row carries the full CommandMeta and names the vault row.
    let conn = worker.read_connection().unwrap();
    let (operation_type, command_id, actor_id, key, entities): (
        String,
        uuid::Uuid,
        String,
        String,
        Vec<u8>,
    ) = conn
        .query_row(
            "SELECT operation_type, command_id, actor_id, idempotency_key, affected_entities
             FROM operation_log ORDER BY rowid DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap();
    assert_eq!(operation_type, "set_household_timezone");
    assert_eq!(command_id, m.command_id);
    assert_eq!(actor_id, m.actor_id);
    assert_eq!(key, m.idempotency_key);
    let entities = String::from_utf8(entities).unwrap();
    assert!(entities.contains("\"table\":\"households\""), "{entities}");
    assert!(
        entities.contains(&db_worker::households::household_id(&metadata.vault_id).to_string()),
        "the entity is the household: {entities}"
    );

    // The audit event is tied to the same command and correlation ids.
    let (audit_command, audit_correlation): (uuid::Uuid, uuid::Uuid) = conn
        .query_row(
            "SELECT command_id, correlation_id FROM audit_events WHERE event_type = ?1",
            params![EVENT],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(audit_command, m.command_id);
    assert_eq!(audit_correlation, m.correlation_id);
}

#[test]
fn each_change_is_its_own_logged_operation() {
    let (_dir, worker) = worker();
    for zone in ["Asia/Tokyo", "Europe/Lisbon", "UTC"] {
        worker.dispatch(meta(), set(zone)).unwrap();
        assert_eq!(worker.vault_metadata().unwrap().household_timezone, zone);
    }
    assert_eq!(worker.operation_count().unwrap(), 3);
    assert_eq!(worker.audit_event_count(EVENT).unwrap(), 3);
}

#[test]
fn setting_the_current_value_is_an_accepted_logged_no_op() {
    let (_dir, worker) = worker();
    worker.dispatch(meta(), set("UTC")).unwrap();
    assert_eq!(worker.vault_metadata().unwrap().household_timezone, "UTC");
    assert_eq!(worker.operation_count().unwrap(), 1);
}

#[test]
fn a_retry_with_the_same_key_replays_and_writes_nothing_new() {
    let (_dir, worker) = worker();
    let key = new_idempotency_key();
    let first = worker
        .dispatch(meta_with_key(&key), set("Asia/Tokyo"))
        .unwrap();
    let second = worker
        .dispatch(meta_with_key(&key), set("Asia/Tokyo"))
        .unwrap();
    assert!(matches!(first, Outcome::Applied { .. }));
    assert!(matches!(second, Outcome::Replayed { .. }));
    assert_eq!(worker.operation_count().unwrap(), 1);
    assert_eq!(worker.audit_event_count(EVENT).unwrap(), 1);
}

#[test]
fn an_unrecognized_name_is_rejected_and_nothing_is_written() {
    let (_dir, worker) = worker();
    for bad in [
        "Not/A_Real_Zone",
        "",
        " ",
        "America/Chicago ",
        "GMT+5",
        "../../etc/passwd",
    ] {
        let result = worker.dispatch(meta(), set(bad));
        assert!(
            matches!(result, Err(DbError::InvalidCommand(_))),
            "{bad:?}: {result:?}"
        );
    }
    assert_eq!(worker.vault_metadata().unwrap().household_timezone, "UTC");
    assert_eq!(worker.operation_count().unwrap(), 0, "no operation logged");
    assert_eq!(
        worker.audit_event_count(EVENT).unwrap(),
        0,
        "no audit event"
    );
    assert_eq!(worker.idempotency_key_count().unwrap(), 0, "no key stored");
}

// ---------------------------------------------------------------------------
// AC2: no other code path writes household_timezone
// ---------------------------------------------------------------------------

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root")
}

fn production_sources() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if path.is_dir() {
                if matches!(name.as_ref(), "target" | "node_modules" | ".git" | "tests") {
                    continue;
                }
                walk(&path, out);
            } else if name.ends_with(".rs") {
                out.push(path);
            }
        }
    }
    let root = repo_root();
    let mut files = Vec::new();
    walk(&root.join("crates"), &mut files);
    walk(&root.join("apps/desktop/src-tauri/src"), &mut files);
    files
}

/// Whether a string literal (a SQL statement) writes `household_timezone`. Whitespace is
/// collapsed so a statement split across lines is still matched.
fn writes_household_timezone(statement: &str) -> bool {
    let sql = statement
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    if !sql.contains("household_timezone") {
        return false;
    }
    sql.starts_with("update ")
        || sql.starts_with("insert ")
        || sql.starts_with("replace ")
        || sql.contains(" set household_timezone")
        || sql.contains(", household_timezone =")
        || sql.contains("on conflict")
}

fn string_literals(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = source.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' && (i == 0 || bytes[i - 1] != b'\\') {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && !(bytes[j] == b'"' && bytes[j - 1] != b'\\') {
                j += 1;
            }
            if j > start {
                out.push(source[start..j].replace("\\\n", " "));
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }
    out
}

/// The only raw write of `household_timezone` is inside `apply/`, in the one function
/// the command runs. The genesis seed of the household row (`households::seed`, schema version
/// 61 and later) and the bootstrap `INSERT` into `vault_metadata` for a database caught at a
/// pre-61 prefix are the other sites: they run before any command exists, and every later change
/// is this command.
#[test]
fn no_other_code_path_writes_household_timezone() {
    let root = repo_root();
    let mut writers = Vec::new();
    for path in production_sources() {
        let rel = path.strip_prefix(&root).unwrap().display().to_string();
        let source = fs::read_to_string(&path).unwrap();
        for literal in string_literals(&source) {
            if writes_household_timezone(&literal) {
                writers.push(rel.clone());
            }
        }
    }
    writers.sort();
    assert_eq!(
        writers,
        vec![
            "crates/db-worker/src/apply/household.rs".to_owned(),
            "crates/db-worker/src/households.rs".to_owned(), // the genesis seed, see above
            "crates/db-worker/src/lib.rs".to_owned(),        // the pre-61 prefix INSERT, see above
            "crates/db-worker/src/migrations.rs".to_owned(), // migration 61's up and down SQL
            "crates/db-worker/src/migrations.rs".to_owned(),
        ],
        "household_timezone must be written only by the SetHouseholdTimezone apply function \
         (and the bootstrap seed row)"
    );

    // The bootstrap site is the INSERT of the default, not an UPDATE.
    let lib = fs::read_to_string(root.join("crates/db-worker/src/lib.rs")).unwrap();
    let update_in_lib = string_literals(&lib).into_iter().any(|literal| {
        let sql = literal
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        sql.starts_with("update vault_metadata") && sql.contains("household_timezone")
    });
    assert!(!update_in_lib, "lib.rs must not UPDATE household_timezone");
}

#[test]
fn the_scanner_recognizes_the_writes_it_exists_to_catch() {
    for sql in [
        "UPDATE vault_metadata SET household_timezone = ?1 WHERE singleton = 1",
        "update vault_metadata\n   set household_timezone = 'UTC'",
        "INSERT INTO vault_metadata (singleton, household_timezone) VALUES (1, ?1)",
        "INSERT OR REPLACE INTO vault_metadata (household_timezone) VALUES (?1)",
    ] {
        assert!(writes_household_timezone(sql), "{sql}");
    }
    for sql in [
        "SELECT household_timezone FROM vault_metadata WHERE singleton = 1",
        "SELECT vault_id FROM vault_metadata",
        "UPDATE vault_metadata SET schema_version = ?1 WHERE singleton = 1",
    ] {
        assert!(!writes_household_timezone(sql), "{sql}");
    }
}
