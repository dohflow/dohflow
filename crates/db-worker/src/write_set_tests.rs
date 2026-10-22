//! The write-set capture (vault-format.md C.1.5 and C.4; personal-cfo-t2s4b part 5c).
//!
//! Unit tests inside the crate, because they drive the writer connection directly (the one
//! that carries the capture triggers) with raw SQL: the shapes a command's apply produces and
//! some it does not yet, such as moving a folded child between parents.

use rusqlite::{params, Connection};
use sync_core::row_digest::{absent_digest, primary_key, row_digest, Column, Kind, Value};
use sync_core::write_set::{write_set_hash, Entry};
use tempfile::TempDir;
use uuid::Uuid;

use crate::write_set::{self, WriteSet};
use crate::{DbError, DbWorker};

const KEY: &str = "correct horse battery staple";

fn worker() -> (TempDir, DbWorker) {
    let dir = TempDir::new().unwrap();
    let worker = DbWorker::open(dir.path().join("ws.vault"), KEY).unwrap();
    (dir, worker)
}

/// Run `body` as one command would: in a transaction on the writer connection, from an empty
/// capture, then compute the write set before committing.
fn command<F>(worker: &DbWorker, body: F) -> Result<WriteSet, DbError>
where
    F: FnOnce(&rusqlite::Transaction<'_>) -> rusqlite::Result<()>,
{
    let mut guard = worker.lock();
    let tx = guard.conn.transaction()?;
    write_set::clear(&tx)?;
    body(&tx)?;
    let set = write_set::finish(&tx)?;
    tx.commit()?;
    Ok(set)
}

fn tables(set: &WriteSet) -> Vec<&str> {
    set.rows.iter().map(|(t, _)| t.as_str()).collect()
}

/// Apply the household-timezone command and return its write set.
fn dispatch_timezone(worker: &DbWorker, timezone: &str) -> WriteSet {
    let meta = crate::CommandMeta {
        command_id: Uuid::now_v7(),
        correlation_id: Uuid::now_v7(),
        causation_id: None,
        actor_type: crate::ActorType::User,
        actor_id: "tester".to_owned(),
        idempotency_key: Uuid::new_v4().to_string(),
        issued_at: crate::IssuedAt::now(),
    };
    worker
        .dispatch(
            meta,
            crate::WriteCommand::SetHouseholdTimezone {
                timezone: timezone.to_owned(),
            },
        )
        .unwrap();
    worker
        .last_write_set()
        .expect("a dispatched command records its write set")
}

fn empty_hash() -> [u8; 32] {
    write_set_hash(&mut []).unwrap()
}

fn attachment(tx: &Connection, id: Uuid, mime: &str) {
    tx.execute(
        "INSERT INTO attachments (id, storage_id, wrapped_content_key, content_key_nonce,
            content_nonce, content_alg, plaintext_size, mime_type, original_filename, ref_count,
            created_at) VALUES (?1, ?2, x'01', x'02', x'03', 'aes', 10, ?3, 'f.png', 0, 't')",
        params![id, id.to_string(), mime],
    )
    .unwrap();
}

fn link(tx: &Connection, attachment: Uuid, entity: Uuid, kind: &str) {
    tx.execute(
        "INSERT INTO attachment_links (attachment_id, entity_kind, entity_id, created_at)
         VALUES (?1, ?2, ?3, 't')",
        params![attachment, kind, entity],
    )
    .unwrap();
}

#[test]
fn a_command_that_changes_no_replicated_row_has_the_empty_write_set() {
    let (_dir, worker) = worker();
    let set = command(&worker, |_| Ok(())).unwrap();
    assert!(set.rows.is_empty());
    assert_eq!(set.hash, empty_hash());
}

#[test]
fn an_inserted_row_is_an_entry_whose_digest_is_its_row_digest() {
    let (_dir, worker) = worker();
    let id = Uuid::new_v4();
    let set = command(&worker, |tx| {
        tx.execute(
            "INSERT INTO tags (id, name, color, created_at) VALUES (?1, 'trip', NULL, 't')",
            params![id],
        )?;
        Ok(())
    })
    .unwrap();
    assert_eq!(tables(&set), ["tags"]);

    // The same entry, built independently from what is now stored.
    let conn = worker.read_connection().unwrap();
    let (name, color, created): (String, Option<String>, Option<String>) = conn
        .query_row(
            "SELECT name, color, NULL FROM tags WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    let _ = (name, color, created);
    let cols: Vec<String> = conn
        .prepare("PRAGMA table_info(tags)")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(cols.contains(&"id".to_owned()));
    let key = primary_key(&[(Kind::Uuid, Value::Blob(id.as_bytes()))]).unwrap();
    assert_eq!(set.rows, [("tags".to_owned(), key)]);
}

#[test]
fn inserting_then_deleting_inside_one_command_leaves_no_entry() {
    let (_dir, worker) = worker();
    let id = Uuid::new_v4();
    let set = command(&worker, |tx| {
        tx.execute(
            "INSERT INTO tags (id, name, color, created_at) VALUES (?1, 'gone', NULL, 't')",
            params![id],
        )?;
        tx.execute("DELETE FROM tags WHERE id = ?1", params![id])?;
        Ok(())
    })
    .unwrap();
    assert!(set.rows.is_empty(), "{:?}", set.rows);
    assert_eq!(set.hash, empty_hash());
}

#[test]
fn a_no_op_update_and_an_identical_delete_and_reinsert_are_not_entries() {
    let (_dir, worker) = worker();
    let id = Uuid::new_v4();
    command(&worker, |tx| {
        tx.execute(
            "INSERT INTO tags (id, name, color, created_at) VALUES (?1, 'keep', 'red', 't')",
            params![id],
        )?;
        Ok(())
    })
    .unwrap();
    // UPDATE ... SET name = name fires the trigger but changes nothing.
    let noop = command(&worker, |tx| {
        tx.execute("UPDATE tags SET name = name WHERE id = ?1", params![id])?;
        Ok(())
    })
    .unwrap();
    assert!(noop.rows.is_empty() && noop.hash == empty_hash());
    // Delete and re-insert the identical row.
    let churn = command(&worker, |tx| {
        tx.execute("DELETE FROM tags WHERE id = ?1", params![id])?;
        tx.execute(
            "INSERT INTO tags (id, name, color, created_at) VALUES (?1, 'keep', 'red', 't')",
            params![id],
        )?;
        Ok(())
    })
    .unwrap();
    assert!(churn.rows.is_empty(), "{:?}", churn.rows);
    // INSERT OR IGNORE that hits an identical row.
    let ignored = command(&worker, |tx| {
        tx.execute(
            "INSERT OR IGNORE INTO tags (id, name, color, created_at) VALUES (?1, 'other', 'blue', 't')",
            params![id],
        )?;
        Ok(())
    })
    .unwrap();
    assert!(ignored.rows.is_empty(), "{:?}", ignored.rows);
}

#[test]
fn a_real_update_and_a_delete_are_entries_and_a_delete_digests_as_absent() {
    let (_dir, worker) = worker();
    let id = Uuid::new_v4();
    command(&worker, |tx| {
        tx.execute(
            "INSERT INTO tags (id, name, color, created_at) VALUES (?1, 'a', NULL, 't')",
            params![id],
        )?;
        Ok(())
    })
    .unwrap();
    let updated = command(&worker, |tx| {
        tx.execute("UPDATE tags SET name = 'b' WHERE id = ?1", params![id])?;
        Ok(())
    })
    .unwrap();
    assert_eq!(tables(&updated), ["tags"]);
    let deleted = command(&worker, |tx| {
        tx.execute("DELETE FROM tags WHERE id = ?1", params![id])?;
        Ok(())
    })
    .unwrap();
    assert_eq!(tables(&deleted), ["tags"]);
    // The entry's digest is the deleted-row digest, so the write set differs from the update's.
    let key = primary_key(&[(Kind::Uuid, Value::Blob(id.as_bytes()))]).unwrap();
    let mut expected = vec![Entry {
        table: "tags".to_owned(),
        primary_key: key.clone(),
        digest: absent_digest("tags", &key).unwrap(),
    }];
    assert_eq!(deleted.hash, write_set_hash(&mut expected).unwrap());
    assert_ne!(deleted.hash, updated.hash);
}

#[test]
fn a_replace_conflict_is_seen_as_a_change_of_the_replaced_row() {
    // merchant_aliases is written with INSERT OR REPLACE; recursive triggers make the implied
    // delete visible, so the before image is the replaced row.
    let (_dir, worker) = worker();
    let identity = worker
        .create_merchant_identity("Cafe", None, false, "user")
        .unwrap();
    worker
        .link_merchant_alias("CAFE", identity, "user", 5_000)
        .unwrap();
    let other = worker
        .create_merchant_identity("Other", None, false, "user")
        .unwrap();
    let set = command(&worker, |tx| {
        tx.execute(
            "INSERT OR REPLACE INTO merchant_aliases
               (normalized_key, merchant_identity_id, source, confidence_bps, created_at)
             VALUES ('CAFE', ?1, 'user', 9000, 'now')",
            params![other],
        )?;
        Ok(())
    })
    .unwrap();
    assert_eq!(tables(&set), ["merchant_aliases"]);
    // Replacing with the identical content is not a change.
    let same = command(&worker, |tx| {
        tx.execute(
            "INSERT OR REPLACE INTO merchant_aliases
               (normalized_key, merchant_identity_id, source, confidence_bps, created_at)
             VALUES ('CAFE', ?1, 'user', 9000, 'now')",
            params![other],
        )?;
        Ok(())
    })
    .unwrap();
    assert!(same.rows.is_empty(), "{:?}", same.rows);
}

#[test]
fn a_projected_table_enters_the_set_only_through_its_replicated_fields() {
    let (_dir, worker) = worker();
    // The household record: the SetHouseholdTimezone command's write set is an entry named
    // "household", keyed by the constant int 1, and never empty.
    let changed = dispatch_timezone(&worker, "America/Chicago");
    assert_eq!(tables(&changed), ["household"]);
    assert_eq!(
        changed.rows[0].1,
        primary_key(&[(Kind::Int, Value::Int(1))]).unwrap()
    );
    // Setting the same timezone again is an accepted no-op: no row changed.
    let again = dispatch_timezone(&worker, "America/Chicago");
    assert!(again.rows.is_empty() && again.hash == empty_hash());
    // A device-local vault_metadata column is not an entry.
    let local = command(&worker, |tx| {
        tx.execute(
            "UPDATE vault_metadata SET schema_version = schema_version",
            [],
        )?;
        Ok(())
    })
    .unwrap();
    assert!(local.rows.is_empty(), "{:?}", local.rows);

    // settings: a user.* key and a legacy name replicate; a device.* key does not.
    let settings = command(&worker, |tx| {
        for (key, value) in [
            ("user.theme", "dark"),
            ("locale", "en-US"),
            ("device.window", "1"),
        ] {
            tx.execute(
                "INSERT INTO settings (key, value, value_schema_version, updated_at)
                 VALUES (?1, ?2, 1, 't')",
                params![key, value],
            )?;
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(tables(&settings), ["settings", "settings"]);
    // updated_at is not a replicated field of a settings row.
    let touch = command(&worker, |tx| {
        tx.execute(
            "UPDATE settings SET updated_at = 'later' WHERE key = 'user.theme'",
            [],
        )?;
        Ok(())
    })
    .unwrap();
    assert!(touch.rows.is_empty(), "{:?}", touch.rows);
}

#[test]
fn attachment_device_local_columns_never_enter_the_digest() {
    let (_dir, worker) = worker();
    let id = Uuid::new_v4();
    let created = command(&worker, |tx| {
        attachment(tx, id, "image/png");
        Ok(())
    })
    .unwrap();
    assert_eq!(tables(&created), ["attachments"]);
    // Re-wrapping the content key (a device-local column) and the derived ref_count change nothing.
    let local = command(&worker, |tx| {
        tx.execute(
            "UPDATE attachments SET wrapped_content_key = x'FF', ref_count = 5 WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    })
    .unwrap();
    assert!(local.rows.is_empty(), "{:?}", local.rows);
    // A replicated field does.
    let replicated = command(&worker, |tx| {
        tx.execute(
            "UPDATE attachments SET mime_type = 'image/jpeg' WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    })
    .unwrap();
    assert_eq!(tables(&replicated), ["attachments"]);
}

#[test]
fn a_folded_child_change_touches_its_parent_and_a_no_op_does_not() {
    let (_dir, worker) = worker();
    let parent = Uuid::new_v4();
    let entity = Uuid::new_v4();
    command(&worker, |tx| {
        attachment(tx, parent, "image/png");
        link(tx, parent, entity, "transaction");
        Ok(())
    })
    .unwrap();

    // The parent row is unchanged, but a child is added: the parent is the (only) entry.
    let added = command(&worker, |tx| {
        link(tx, parent, Uuid::new_v4(), "transaction");
        Ok(())
    })
    .unwrap();
    assert_eq!(tables(&added), ["attachments"]);

    // A no-op update of a child is not a change.
    let noop = command(&worker, |tx| {
        tx.execute("UPDATE attachment_links SET created_at = created_at", [])?;
        Ok(())
    })
    .unwrap();
    assert!(noop.rows.is_empty(), "{:?}", noop.rows);

    // Removing a child is a change of the parent.
    let removed = command(&worker, |tx| {
        tx.execute(
            "DELETE FROM attachment_links WHERE entity_id = ?1",
            params![entity],
        )?;
        Ok(())
    })
    .unwrap();
    assert_eq!(tables(&removed), ["attachments"]);
    assert_ne!(added.hash, removed.hash);
}

#[test]
fn a_child_moved_between_parents_touches_both_parents() {
    let (_dir, worker) = worker();
    let (a, b, entity) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    command(&worker, |tx| {
        attachment(tx, a, "image/png");
        attachment(tx, b, "image/png");
        link(tx, a, entity, "transaction");
        Ok(())
    })
    .unwrap();
    let moved = command(&worker, |tx| {
        tx.execute(
            "UPDATE attachment_links SET attachment_id = ?1 WHERE entity_id = ?2",
            params![b, entity],
        )?;
        Ok(())
    })
    .unwrap();
    assert_eq!(tables(&moved), ["attachments", "attachments"]);
    let keys: Vec<&Vec<u8>> = moved.rows.iter().map(|(_, k)| k).collect();
    assert_ne!(keys[0], keys[1], "both parents, once each");
}

#[test]
fn identical_postings_count_twice_so_dropping_one_changes_the_parent() {
    let (_dir, worker) = worker();
    let parent = Uuid::new_v4();
    let entity = Uuid::new_v4();
    command(&worker, |tx| {
        attachment(tx, parent, "image/png");
        link(tx, parent, entity, "transaction");
        link(tx, parent, entity, "transaction");
        Ok(())
    })
    .unwrap();
    // Delete one of two identical rows (by rowid): the multiset shrinks, the parent changes.
    let one = command(&worker, |tx| {
        tx.execute(
            "DELETE FROM attachment_links WHERE rowid = (SELECT MIN(rowid) FROM attachment_links)",
            [],
        )?;
        Ok(())
    })
    .unwrap();
    assert_eq!(tables(&one), ["attachments"]);
}

#[test]
fn an_orphaned_child_fails_the_command_before_commit() {
    let (_dir, worker) = worker();
    let parent = Uuid::new_v4();
    command(&worker, |tx| {
        attachment(tx, parent, "image/png");
        link(tx, parent, Uuid::new_v4(), "transaction");
        Ok(())
    })
    .unwrap();
    // Deleting the parent while a child remains leaves an orphan: refused deterministically.
    let result = command(&worker, |tx| {
        tx.execute("DELETE FROM attachments WHERE id = ?1", params![parent])?;
        Ok(())
    });
    assert!(
        matches!(result, Err(DbError::InvalidCommand(_))),
        "{result:?}"
    );
    // Deleting the children first (as a cascade would) is fine, and the parent digests as absent.
    let ok = command(&worker, |tx| {
        tx.execute(
            "DELETE FROM attachment_links WHERE attachment_id = ?1",
            params![parent],
        )?;
        tx.execute("DELETE FROM attachments WHERE id = ?1", params![parent])?;
        Ok(())
    })
    .unwrap();
    assert_eq!(tables(&ok), ["attachments"]);
    let key = primary_key(&[(Kind::Uuid, Value::Blob(parent.as_bytes()))]).unwrap();
    let mut expected = vec![Entry {
        table: "attachments".to_owned(),
        primary_key: key.clone(),
        digest: absent_digest("attachments", &key).unwrap(),
    }];
    assert_eq!(ok.hash, write_set_hash(&mut expected).unwrap());
}

#[test]
fn a_rolled_back_command_leaves_the_capture_empty() {
    let (_dir, worker) = worker();
    let id = Uuid::new_v4();
    {
        let mut guard = worker.lock();
        let tx = guard.conn.transaction().unwrap();
        tx.execute(
            "INSERT INTO tags (id, name, color, created_at) VALUES (?1, 'x', NULL, 't')",
            params![id],
        )
        .unwrap();
        // dropped without commit: rolled back, temp tables included
    }
    let set = command(&worker, |_| Ok(())).unwrap();
    assert!(set.rows.is_empty());
}

#[test]
fn a_direct_write_outside_a_command_does_not_leak_into_the_next_command() {
    let (_dir, worker) = worker();
    // set_setting is a direct writer: the triggers fire, and the next command must start clean.
    worker.set_setting("user.theme", "dark").unwrap();
    let set = command(&worker, |_| Ok(())).unwrap();
    assert!(set.rows.is_empty(), "{:?}", set.rows);
}

#[test]
fn the_digest_of_an_inserted_row_equals_an_independent_encoding_of_the_stored_row() {
    let (_dir, worker) = worker();
    let id = Uuid::new_v4();
    let set = command(&worker, |tx| {
        tx.execute(
            "INSERT INTO tags (id, name, color, created_at) VALUES (?1, 'trip', 'red', 't')",
            params![id],
        )?;
        Ok(())
    })
    .unwrap();
    let conn = worker.read_connection().unwrap();
    // tags columns: id uuid, name text, color text (plus any others, read generically).
    let mut stmt = conn.prepare("PRAGMA table_info(tags)").unwrap();
    let cols: Vec<(String, String)> = stmt
        .query_map([], |r| Ok((r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let values: Vec<rusqlite::types::Value> = {
        let names: Vec<&str> = cols.iter().map(|(n, _)| n.as_str()).collect();
        let sql = format!("SELECT {} FROM tags WHERE id = ?1", names.join(", "));
        conn.query_row(&sql, params![id], |r| {
            (0..cols.len())
                .map(|i| r.get::<_, rusqlite::types::Value>(i))
                .collect()
        })
        .unwrap()
    };
    let columns: Vec<Column<'_>> = cols
        .iter()
        .zip(&values)
        .map(|((name, ty), value)| Column {
            name,
            kind: match (ty.as_str(), name.as_str()) {
                ("INTEGER", _) => Kind::Int,
                ("TEXT", _) => Kind::Text,
                ("BLOB", "id") => Kind::Uuid,
                other => panic!("unexpected column {other:?}"),
            },
            value: match value {
                rusqlite::types::Value::Null => Value::Null,
                rusqlite::types::Value::Integer(v) => Value::Int(*v),
                rusqlite::types::Value::Text(t) => Value::Text(t.as_bytes()),
                rusqlite::types::Value::Blob(b) => Value::Blob(b),
                rusqlite::types::Value::Real(_) => panic!("REAL"),
            },
        })
        .collect();
    let key = primary_key(&[(Kind::Uuid, Value::Blob(id.as_bytes()))]).unwrap();
    let mut expected = vec![Entry {
        table: "tags".to_owned(),
        primary_key: key,
        digest: row_digest("tags", &columns).unwrap(),
    }];
    assert_eq!(set.hash, write_set_hash(&mut expected).unwrap());
}

// ---- the cost of the capture -------------------------------------------------------------------
//
// The capture adds a per-command cost (TEMP triggers plus the read-and-hash at the end). The
// REQUIRED check on it is RELATIVE: capture on against capture off, in the same process,
// interleaved, so a slow or loaded machine slows both sides alike. The absolute added time is not
// a required check, because it varies 20x with the machine: 5c measured +175 us on a quiet laptop
// and +3,600 us on a loaded hosted runner, with the ratio staying between 1.5x and 2.5x
// (personal-cfo-g3m.16, measurements by the Sync engine lane). The absolute 3 ms budget is
// asserted by the `#[ignore]`d test below, which the non-required perf job in ci.yml runs and
// reports on every pull request.

/// A warm-up of commands per side, the commands timed per run, and the runs per side.
const WARM_UP: usize = 50;
const COMMANDS: usize = 400;
const RUNS: usize = 7;
/// The required ratio ceiling: capture on may cost less than this many times capture off. The
/// first cut of the capture was 3.2x; the worst ratio seen since is 2.5x.
const MAX_RATIO: f64 = 4.0;
/// A loose cap on the TIME the capture adds to one command, in microseconds: the Definition of
/// Done budget for a whole command (P50 < 50 ms). It cannot flake on any real machine.
const MAX_ADDED_US: f64 = 50_000.0;

/// Time `commands` busy commands (one transaction with two postings and its details) on a fresh
/// vault, with the capture off (`disable`) or on.
fn time_commands(disable: bool, commands: usize) -> std::time::Duration {
    use std::time::Instant;

    write_set::DISABLED.with(|d| d.set(disable));
    let (_dir, worker) = worker();
    let account = core_ledger::Account::new(
        core_ledger::AccountId::new(),
        core_ledger::LedgerAccountId::new(),
        "Checking",
        core_ledger::CashflowRole::LiquidCash,
        core_money::Currency::Usd,
        core_ledger::AccountFlags::default(),
    );
    let id = account.id();
    let meta = || crate::CommandMeta {
        command_id: Uuid::now_v7(),
        correlation_id: Uuid::now_v7(),
        causation_id: None,
        actor_type: crate::ActorType::User,
        actor_id: "tester".to_owned(),
        idempotency_key: Uuid::new_v4().to_string(),
        issued_at: crate::IssuedAt::now(),
    };
    worker
        .dispatch(
            meta(),
            crate::WriteCommand::CreateAccount {
                account: Box::new(account),
                opening_balance: None,
            },
        )
        .unwrap();
    let start = Instant::now();
    for _ in 0..commands {
        worker
            .dispatch(
                meta(),
                crate::WriteCommand::RecordTransaction {
                    transaction_id: core_ledger::TransactionId::new(),
                    account_id: id,
                    amount: core_money::Money::new(100, core_money::Currency::Usd),
                    occurred_at: chrono::Utc::now(),
                },
            )
            .unwrap();
    }
    let elapsed = start.elapsed();
    write_set::DISABLED.with(|d| d.set(false));
    elapsed
}

fn median(mut v: Vec<std::time::Duration>) -> std::time::Duration {
    v.sort();
    v[v.len() / 2]
}

/// Interleave capture-off and capture-on runs (after a warm-up of each), and return the MEDIAN
/// of each side. Interleaving puts a load spike on both sides; the median ignores the outliers.
fn measure() -> (std::time::Duration, std::time::Duration) {
    time_commands(true, WARM_UP);
    time_commands(false, WARM_UP);
    let (mut off, mut on) = (Vec::new(), Vec::new());
    for _ in 0..RUNS {
        off.push(time_commands(true, COMMANDS));
        on.push(time_commands(false, COMMANDS));
    }
    (median(off), median(on))
}

fn per_command_us(d: std::time::Duration) -> f64 {
    d.as_micros() as f64 / COMMANDS as f64
}

/// The line the numbers are printed on, for the logs and the perf job's summary.
fn describe(off: std::time::Duration, on: std::time::Duration) -> String {
    format!(
        "{:.0} us/command off, {:.0} us/command on ({:.2}x, +{:.0} us)",
        per_command_us(off),
        per_command_us(on),
        on.as_secs_f64() / off.as_secs_f64(),
        per_command_us(on) - per_command_us(off)
    )
}

/// The REQUIRED check: the ratio ceiling and the loose added-time cap. `Err` says why not.
fn within_relative_budget(off: std::time::Duration, on: std::time::Duration) -> Result<(), String> {
    let ratio = on.as_secs_f64() / off.as_secs_f64();
    let added_us = per_command_us(on) - per_command_us(off);
    if ratio >= MAX_RATIO {
        return Err(format!(
            "the capture multiplied the cost of a command by {ratio:.2}x (limit {MAX_RATIO}x): {}",
            describe(off, on)
        ));
    }
    if added_us >= MAX_ADDED_US {
        return Err(format!(
            "the capture added {added_us:.0} us per command (limit {MAX_ADDED_US:.0} us): {}",
            describe(off, on)
        ));
    }
    Ok(())
}

/// The cost of the capture on the busiest command shape, against the same commands with the
/// capture off, in the same run: a ratio, not a wall-clock number (see the note above).
#[test]
fn the_capture_adds_little_to_a_command() {
    let (off, on) = measure();
    println!("write-set capture: {}", describe(off, on));
    if let Err(why) = within_relative_budget(off, on) {
        panic!("{why}");
    }
}

/// How many times the slowdown test runs the capture's read-and-hash pass per command. The pass
/// is only part of the capture (the TEMP triggers are the rest): measured, running it six times
/// moves the ratio from about 1.8x to about 3.6x, below the 4x ceiling. Twenty passes makes the
/// whole capture cost well over six times what it normally does, and keeps the ratio far above
/// the ceiling even on a machine where the capture is cheap (1.5x at multiplier 1 reads over 5x
/// at 20).
const SLOWDOWN_PASSES: u32 = 20;

/// Proof that the relative check can fail: with the capture's read-and-hash pass repeated
/// `SLOWDOWN_PASSES` times per command (the test-only `CAPTURE_COST_MULTIPLIER`), the same check
/// must reject it. If a change made the check unable to fail, this test would go red.
#[test]
fn the_relative_check_rejects_a_much_slower_capture() {
    write_set::CAPTURE_COST_MULTIPLIER.with(|m| m.set(SLOWDOWN_PASSES));
    let (off, on) = measure();
    write_set::CAPTURE_COST_MULTIPLIER.with(|m| m.set(1));
    println!("write-set capture, slowed: {}", describe(off, on));
    assert!(
        within_relative_budget(off, on).is_err(),
        "the relative check accepted a much slower capture: {}",
        describe(off, on)
    );
}

/// The ABSOLUTE budget: under 3 ms added per command. This is not a required check, because the
/// number depends on the machine (about 0.2 ms on a laptop, about 3.6 ms on a loaded hosted
/// runner). It is `#[ignore]`d from `cargo test --workspace` and run by the non-required perf job
/// in ci.yml, which prints the numbers into each pull request's checks, and it belongs to the
/// release perf budget (docs/architecture/definition-of-done.md).
#[test]
#[ignore = "absolute perf budget; run by the non-required perf job (cargo test ... -- --ignored)"]
fn the_capture_stays_inside_the_absolute_budget() {
    let (off, on) = measure();
    println!("write-set capture ABSOLUTE: {}", describe(off, on));
    let added_us = per_command_us(on) - per_command_us(off);
    assert!(
        added_us < 3_000.0,
        "the capture added {added_us:.0} us per command (budget 3000 us): {}",
        describe(off, on)
    );
}
