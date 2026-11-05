//! The logical class-1 export (vault-format.md B.3, B.5, C.8; personal-cfo-1cltt.1): the table
//! set follows the B.2 inventory, rows are in key order, a replica can recompute every state
//! digest from the file, nothing outside class 1 crosses, and a value that cannot be exported
//! stops genesis with a typed refusal naming the table and column.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use base64::Engine as _;
use chrono::NaiveDate;
use common::{liquid_account_cmd, meta, record_with_counterparty, worker};
use core_ledger::AccountId;
use core_money::Currency;
use db_worker::logical_export::{export_genesis_plaintext, ExportError, ExportInput};
use db_worker::DbWorker;
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use sync_core::row_digest::{
    parent_row_digest, primary_key, row_digest, ChildTable, Column, Kind, Value,
};
use sync_core::snapshot::{hex, verify, RecordedVoid, StateDigest, Verified};
use uuid::Uuid;

const INVENTORY: &str = include_str!("../../../docs/architecture/vault-format.md");
const CLASSES: &str = include_str!("../table_classes.toml");
const COLUMNS: &str = include_str!("../table_columns.toml");

/// A credential fixture that must never appear in a snapshot, in any spelling.
const SECRET: &str = "SENTINEL-credential-9f31c07a";

fn raw(worker: &DbWorker) -> Connection {
    let conn = worker.read_connection().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = OFF").unwrap();
    conn
}

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 3, d).unwrap()
}

/// A vault with an account, an opening balance, transactions (so postings exist), details, a
/// user setting and a device setting, an attachment with a link, and a connector secret.
fn populated() -> (tempfile::TempDir, DbWorker) {
    let (dir, worker) = worker();
    let account = AccountId::new();
    worker
        .dispatch(meta(), liquid_account_cmd(account, Currency::Usd, 250_000))
        .unwrap();
    for (i, cents) in [-1250, 99_000, -4_575].into_iter().enumerate() {
        record_with_counterparty(&worker, account, cents, day(i as u32 + 1), "Corner Shop");
    }
    let conn = raw(&worker);
    conn.execute(
        "INSERT INTO settings (key, value, value_schema_version, updated_at) VALUES
            ('user.theme', 'dark', 1, '2026-03-01T00:00:00Z'),
            ('device.window', 'x=10,y=20', 1, '2026-03-01T00:00:00Z')",
        [],
    )
    .unwrap();
    let attachment = Uuid::from_u128(0xA1);
    conn.execute(
        "INSERT INTO attachments (id, storage_id, wrapped_content_key, content_key_nonce,
                content_nonce, content_alg, plaintext_size, mime_type, original_filename,
                ref_count, created_at)
         VALUES (?1, 'storage-1', x'0102', x'0304', x'0506', 'aes-256-gcm', 42, 'image/png',
                 'receipt.png', 1, '2026-03-02T00:00:00Z')",
        [attachment],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO attachment_links (attachment_id, entity_kind, entity_id, created_at)
         VALUES (?1, 'transaction', ?2, '2026-03-02T00:00:00Z')",
        params![attachment, Uuid::from_u128(0xB1)],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO connector_connection_secrets (connection_id, credential, updated_at)
         VALUES (?1, ?2, '2026-03-02T00:00:00Z')",
        params![Uuid::from_u128(0xC1), SECRET],
    )
    .unwrap();
    (dir, worker)
}

fn export(worker: &DbWorker) -> Result<Vec<u8>, ExportError> {
    export_genesis_plaintext(&raw(worker), &ExportInput::default())
}

// -- the inventory -----------------------------------------------------------------------------

/// `(table, class, sync disposition)` for every B.2 row.
fn inventory() -> Vec<(String, String, String)> {
    let inventory = INVENTORY
        .split("<!-- inventory:start -->")
        .nth(1)
        .and_then(|rest| rest.split("<!-- inventory:end -->").next())
        .expect("the inventory markers");
    inventory
        .lines()
        .filter_map(|line| {
            let cells: Vec<&str> = line.split('|').map(str::trim).collect();
            let table = cells.get(1)?.strip_prefix('`')?.strip_suffix('`')?;
            Some((
                table.to_owned(),
                (*cells.get(2)?).to_owned(),
                cells.get(4)?.trim_matches('`').to_owned(),
            ))
        })
        .collect()
}

fn pseudo_tables() -> BTreeSet<String> {
    CLASSES
        .split("[[table]]")
        .filter(|block| block.contains("kind = \"pseudo\""))
        .filter_map(|block| {
            block
                .lines()
                .find_map(|l| l.strip_prefix("table = \""))
                .map(|l| l.trim_end_matches('"').to_owned())
        })
        .collect()
}

fn tables_of(verified: &Verified) -> BTreeSet<String> {
    verified
        .manifest
        .tables
        .iter()
        .map(|t| t.name.clone())
        .collect()
}

#[test]
fn the_exported_table_set_is_the_inventory_include_and_fields_rows_without_the_pseudo_rows() {
    let (_dir, worker) = populated();
    let verified = verify(&export(&worker).unwrap()).unwrap();
    let pseudo = pseudo_tables();
    assert!(pseudo.contains("settings_user_keys"));
    let wanted: BTreeSet<String> = inventory()
        .into_iter()
        .filter(|(_, _, sync)| sync == "include" || sync == "fields")
        .map(|(table, _, _)| table)
        .filter(|table| !pseudo.contains(table))
        .collect();
    assert_eq!(tables_of(&verified), wanted);
    // The skipped set equals the pseudo rows: every inventory include/fields row that has no file
    // is a pseudo row, and nothing else.
    let skipped: BTreeSet<String> = inventory()
        .into_iter()
        .filter(|(_, _, sync)| sync == "include" || sync == "fields")
        .map(|(table, _, _)| table)
        .filter(|table| !tables_of(&verified).contains(table))
        .collect();
    assert_eq!(skipped, pseudo.intersection(&skipped).cloned().collect());
    assert!(!skipped.is_empty());
}

#[test]
fn every_table_has_a_file_including_the_empty_ones() {
    let (_dir, worker) = populated();
    let verified = verify(&export(&worker).unwrap()).unwrap();
    let empties: Vec<&str> = verified
        .manifest
        .tables
        .iter()
        .filter(|t| t.row_count == "0")
        .map(|t| t.name.as_str())
        .collect();
    assert!(
        !empties.is_empty(),
        "a fresh vault has empty class-1 tables"
    );
    for name in empties {
        assert!(
            verified
                .files
                .iter()
                .any(|(n, c)| n == name && c.is_empty()),
            "{name}"
        );
    }
    assert!(verified.manifest.tables.iter().any(|t| t.row_count != "0"));
}

// -- order, determinism, the manifest header ---------------------------------------------------

#[test]
fn the_export_is_deterministic_and_carries_the_origin_creation_time() {
    let (_dir, worker) = populated();
    let a = export(&worker).unwrap();
    let b = export(&worker).unwrap();
    assert_eq!(a, b);
    let verified = verify(&a).unwrap();
    let created: String = raw(&worker)
        .query_row("SELECT created_at FROM vault_metadata", [], |r| r.get(0))
        .unwrap();
    assert_eq!(verified.manifest.vault_created_at, created);
    let version: i64 = raw(&worker)
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(verified.manifest.schema_version, version.to_string());
    assert_eq!(verified.manifest.format_version, "1");
}

#[test]
fn voids_and_the_closure_manifest_are_inputs() {
    let (_dir, worker) = populated();
    let voids = [RecordedVoid {
        void_id: [9; 32],
        first: 4,
        last: 7,
    }];
    let artifacts = [[1u8; 32], [2; 32]];
    let plaintext = export_genesis_plaintext(
        &raw(&worker),
        &ExportInput {
            voids: &voids,
            artifacts: &artifacts,
        },
    )
    .unwrap();
    let manifest = verify(&plaintext).unwrap().manifest;
    assert_eq!(manifest.voids[0].first, "4");
    assert_eq!(manifest.voids[0].void_id, hex(&[9; 32]));
    assert_eq!(manifest.artifacts.len(), 2);
}

// -- decoding a file back, as a replica does ----------------------------------------------------

struct Decoded {
    kind: Kind,
    name: String,
    value: Stored,
}

#[derive(Clone)]
enum Stored {
    Null,
    Int(i64),
    Text(String),
    Blob(Vec<u8>),
}

fn decode_rows(table: &sync_core::snapshot::TableEntry, content: &[u8]) -> Vec<Vec<Decoded>> {
    let text = std::str::from_utf8(content).unwrap();
    text.lines()
        .map(|line| {
            let object: serde_json::Value = serde_json::from_str(line).unwrap();
            table
                .columns
                .iter()
                .map(|column| {
                    let kind = match column.kind.as_str() {
                        "int" => Kind::Int,
                        "text" => Kind::Text,
                        "uuid" => Kind::Uuid,
                        "bytes" => Kind::Bytes,
                        other => panic!("{other}"),
                    };
                    let value = match &object[&column.name] {
                        serde_json::Value::Null => Stored::Null,
                        serde_json::Value::String(s) => match kind {
                            Kind::Int => Stored::Int(s.parse().unwrap()),
                            Kind::Text => Stored::Text(s.clone()),
                            Kind::Uuid => {
                                Stored::Blob(Uuid::parse_str(s).unwrap().as_bytes().to_vec())
                            }
                            Kind::Bytes => Stored::Blob(
                                base64::engine::general_purpose::STANDARD.decode(s).unwrap(),
                            ),
                        },
                        other => panic!("{other}"),
                    };
                    Decoded {
                        kind,
                        name: column.name.clone(),
                        value,
                    }
                })
                .collect()
        })
        .collect()
}

fn columns_of(row: &[Decoded]) -> Vec<Column<'_>> {
    row.iter()
        .map(|d| Column {
            name: &d.name,
            kind: d.kind,
            value: match &d.value {
                Stored::Null => Value::Null,
                Stored::Int(i) => Value::Int(*i),
                Stored::Text(t) => Value::Text(t.as_bytes()),
                Stored::Blob(b) => Value::Blob(b),
            },
        })
        .collect()
}

fn keys_of(table: &str) -> Vec<String> {
    let block = COLUMNS
        .split("[[table]]")
        .find(|b| b.contains(&format!("name = \"{table}\"")))
        .unwrap_or_else(|| panic!("no manifest block for {table}"));
    let line = block
        .lines()
        .find_map(|l| l.strip_prefix("primary_key = "))
        .unwrap();
    line.split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect()
}

fn children_of(table: &str) -> Vec<String> {
    let block = COLUMNS
        .split("[[table]]")
        .find(|b| b.contains(&format!("name = \"{table}\"")))
        .unwrap();
    block
        .lines()
        .find_map(|l| l.strip_prefix("children = "))
        .map(|l| l.split('"').skip(1).step_by(2).map(str::to_owned).collect())
        .unwrap_or_default()
}

fn parent_column(child: &str) -> String {
    let block = COLUMNS
        .split("[[table]]")
        .find(|b| b.contains(&format!("name = \"{child}\"")))
        .unwrap();
    block
        .lines()
        .find_map(|l| l.strip_prefix("parent_column = \""))
        .unwrap()
        .trim_end_matches('"')
        .to_owned()
}

/// What a bootstrapping replica does: from the file alone, the key bytes and row digests of each
/// row, in file order. A parent's digest includes the child section built from the child files.
fn replica_rows(verified: &Verified, table: &str) -> Vec<(Vec<u8>, [u8; 32])> {
    let entry = verified
        .manifest
        .tables
        .iter()
        .find(|t| t.name == table)
        .unwrap();
    let content = &verified.files.iter().find(|(n, _)| n == table).unwrap().1;
    let rows = decode_rows(entry, content);
    let keys = keys_of(table);
    let child_names = children_of(table);
    let child_rows: BTreeMap<String, Vec<Vec<Decoded>>> = child_names
        .iter()
        .map(|child| {
            let entry = verified
                .manifest
                .tables
                .iter()
                .find(|t| &t.name == child)
                .unwrap();
            let content = &verified.files.iter().find(|(n, _)| n == child).unwrap().1;
            (child.clone(), decode_rows(entry, content))
        })
        .collect();
    rows.iter()
        .map(|row| {
            let cols = columns_of(row);
            let key_parts: Vec<(Kind, Value<'_>)> = keys
                .iter()
                .map(|k| {
                    let c = cols.iter().find(|c| c.name == k).unwrap();
                    (c.kind, c.value)
                })
                .collect();
            let key = primary_key(&key_parts).unwrap();
            let digest = if child_names.is_empty() {
                row_digest(table, &cols).unwrap()
            } else {
                let id = &row.iter().find(|d| d.name == "id").unwrap().value;
                let Stored::Blob(id) = id else { panic!() };
                let tables: Vec<ChildTable<'_>> = child_names
                    .iter()
                    .map(|child| {
                        let column = parent_column(child);
                        ChildTable {
                            table: child,
                            rows: child_rows[child]
                                .iter()
                                .filter(|r| {
                                    matches!(&r.iter().find(|d| d.name == column).unwrap().value,
                                        Stored::Blob(b) if b == id)
                                })
                                .map(|r| columns_of(r))
                                .collect(),
                        }
                    })
                    .collect();
                parent_row_digest(table, &cols, &tables).unwrap()
            };
            (key, digest)
        })
        .collect()
}

#[test]
fn rows_are_in_ascending_key_order_and_a_replica_recomputes_every_state_digest() {
    let (_dir, worker) = populated();
    let verified = verify(&export(&worker).unwrap()).unwrap();
    let mut keyed = 0;
    let mut folded = 0;
    for entry in &verified.manifest.tables {
        let Some(want) = &entry.state_digest else {
            folded += 1;
            continue;
        };
        let rows = replica_rows(&verified, &entry.name);
        for pair in rows.windows(2) {
            assert!(pair[0].0 < pair[1].0, "{}: keys out of order", entry.name);
        }
        let mut digest = StateDigest::new(&entry.name).unwrap();
        for (key, row) in &rows {
            digest.push(key, *row).unwrap();
        }
        assert_eq!(&hex(&digest.finish().unwrap()), want, "{}", entry.name);
        keyed += 1;
    }
    // Two folded children (ledger_postings, attachment_links) keep a file and have no digest.
    assert_eq!(folded, 2);
    assert!(keyed > 30);
}

#[test]
fn a_changed_row_changes_its_tables_state_digest_and_no_other() {
    let (_dir, worker) = populated();
    let before = verify(&export(&worker).unwrap()).unwrap().manifest;
    raw(&worker)
        .execute("UPDATE accounts SET name = name || ' (renamed)'", [])
        .unwrap();
    let after = verify(&export(&worker).unwrap()).unwrap().manifest;
    let changed: Vec<&str> = before
        .tables
        .iter()
        .zip(&after.tables)
        .filter(|(a, b)| a.state_digest != b.state_digest)
        .map(|(a, _)| a.name.as_str())
        .collect();
    assert_eq!(changed, ["accounts"]);
}

#[test]
fn a_folded_child_row_changes_its_parents_state_digest() {
    let (_dir, worker) = populated();
    let before = verify(&export(&worker).unwrap()).unwrap().manifest;
    raw(&worker)
        .execute("UPDATE ledger_postings SET minor_units = minor_units + 1 WHERE rowid = (SELECT MIN(rowid) FROM ledger_postings)", [])
        .unwrap();
    let after = verify(&export(&worker).unwrap()).unwrap().manifest;
    let digest = |m: &sync_core::snapshot::Manifest, t: &str| {
        m.tables
            .iter()
            .find(|e| e.name == t)
            .unwrap()
            .state_digest
            .clone()
    };
    assert_ne!(
        digest(&before, "ledger_transactions"),
        digest(&after, "ledger_transactions")
    );
    assert_eq!(digest(&before, "ledger_postings"), None);
}

// -- what must not cross (AC2, AC4) --------------------------------------------------------------

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn no_table_outside_the_genesis_set_and_no_credential_byte_crosses() {
    let (_dir, worker) = populated();
    let plaintext = export(&worker).unwrap();
    let verified = verify(&plaintext).unwrap();
    let exported = tables_of(&verified);
    let mut checked = 0;
    for (table, _class, sync) in inventory() {
        if sync == "include" || sync == "fields" {
            continue;
        }
        // `exclude`, `rebuild` and `artifact` rows: not one is a file, a manifest entry or a path.
        assert!(!exported.contains(&table), "{table}");
        for needle in [
            format!("data/{table}.jsonl"),
            format!("\"name\":\"{table}\""),
        ] {
            assert!(!contains(&plaintext, needle.as_bytes()), "{needle}");
        }
        checked += 1;
    }
    assert!(checked > 30);

    // The connector secret, in every spelling a bundle could carry it.
    let b64 = base64::engine::general_purpose::STANDARD.encode(SECRET);
    for needle in [
        SECRET.to_owned(),
        b64,
        hex(SECRET.as_bytes()),
        "credential".to_owned(),
    ] {
        assert!(!contains(&plaintext, needle.as_bytes()), "{needle}");
    }
    for table in [
        "connector_connection_secrets",
        "connector_link_cursors",
        "command_idempotency_keys",
    ] {
        assert!(!contains(&plaintext, table.as_bytes()), "{table}");
    }
}

#[test]
fn only_the_user_settings_cross_and_no_device_key_or_value_appears() {
    let (_dir, worker) = populated();
    let plaintext = export(&worker).unwrap();
    assert!(contains(&plaintext, b"user.theme"));
    assert!(contains(&plaintext, b"dark"));
    for needle in ["device.window", "device.", "x=10,y=20"] {
        assert!(!contains(&plaintext, needle.as_bytes()), "{needle}");
    }
    let verified = verify(&plaintext).unwrap();
    let settings = verified
        .manifest
        .tables
        .iter()
        .find(|t| t.name == "settings")
        .unwrap();
    let columns: Vec<&str> = settings.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(columns, ["key", "value"]);
}

#[test]
fn an_attachment_carries_its_replicated_fields_and_never_its_keys_or_storage_id() {
    let (_dir, worker) = populated();
    let plaintext = export(&worker).unwrap();
    let verified = verify(&plaintext).unwrap();
    let attachments = verified
        .manifest
        .tables
        .iter()
        .find(|t| t.name == "attachments")
        .unwrap();
    let columns: Vec<&str> = attachments
        .columns
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(
        columns,
        [
            "id",
            "plaintext_size",
            "mime_type",
            "original_filename",
            "created_at"
        ]
    );
    assert!(contains(&plaintext, b"receipt.png"));
    for needle in [
        "storage-1",
        "aes-256-gcm",
        "wrapped_content_key",
        "ref_count",
    ] {
        assert!(!contains(&plaintext, needle.as_bytes()), "{needle}");
    }
}

#[test]
fn the_node_id_and_the_vault_metadata_row_are_not_carried() {
    let (_dir, worker) = populated();
    let plaintext = export(&worker).unwrap();
    let node: Uuid = raw(&worker)
        .query_row("SELECT node_id FROM vault_meta", [], |r| r.get(0))
        .unwrap();
    for needle in [node.to_string(), hex(node.as_bytes())] {
        assert!(!contains(&plaintext, needle.as_bytes()), "{needle}");
    }
    let verified = verify(&plaintext).unwrap();
    for table in ["vault_meta", "vault_metadata"] {
        assert!(!tables_of(&verified).contains(table));
    }
    // The vault's own id is not a row of any file either: it is the snapshot's header.
    let vault_id: Uuid = raw(&worker)
        .query_row("SELECT vault_id FROM vault_metadata", [], |r| r.get(0))
        .unwrap();
    assert!(!contains(&plaintext, vault_id.to_string().as_bytes()));
}

// -- declared-kind validation before genesis (AC3) ------------------------------------------------

fn refusal(worker: &DbWorker) -> ExportError {
    export(worker).expect_err("the export should refuse")
}

#[test]
fn a_storage_class_mismatch_stops_genesis_naming_the_table_and_column() {
    let (_dir, worker) = populated();
    raw(&worker)
        .execute("UPDATE accounts SET retirement = 'not a number'", [])
        .unwrap();
    match refusal(&worker) {
        ExportError::KindMismatch { table, column } => {
            assert_eq!(
                (table.as_str(), column.as_str()),
                ("accounts", "retirement")
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_folded_child_row_with_a_bad_value_names_its_table_and_column() {
    let (_dir, worker) = populated();
    let conn = raw(&worker);
    // A folded child's rows are checked first (its file sorts before its parent's); the refusal
    // must name the column, never an empty one. The balance triggers would refuse the update.
    let triggers: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'trigger' AND tbl_name = 'ledger_postings'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    for trigger in triggers {
        conn.execute_batch(&format!("DROP TRIGGER \"{trigger}\""))
            .unwrap();
    }
    conn.execute(
        "UPDATE ledger_postings SET minor_units = 'not a number'",
        [],
    )
    .unwrap();
    match refusal(&worker) {
        ExportError::KindMismatch { table, column } => assert_eq!(
            (table.as_str(), column.as_str()),
            ("ledger_postings", "minor_units")
        ),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_folded_childs_uuid_column_that_is_not_sixteen_bytes_names_its_table_and_column() {
    let (_dir, worker) = populated();
    raw(&worker)
        .execute("UPDATE attachment_links SET entity_id = x'0102'", [])
        .unwrap();
    match refusal(&worker) {
        ExportError::KindMismatch { table, column } => assert_eq!(
            (table.as_str(), column.as_str()),
            ("attachment_links", "entity_id")
        ),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_real_value_is_refused_not_rounded() {
    let (_dir, worker) = populated();
    raw(&worker)
        .execute("UPDATE accounts SET retirement = 1.5", [])
        .unwrap();
    assert!(matches!(
        refusal(&worker),
        ExportError::KindMismatch { table, column } if table == "accounts" && column == "retirement"
    ));
}

#[test]
fn a_uuid_column_that_is_not_sixteen_bytes_is_refused() {
    let (_dir, worker) = populated();
    raw(&worker)
        .execute("UPDATE accounts SET ledger_account_id = x'0102'", [])
        .unwrap();
    assert!(matches!(
        refusal(&worker),
        ExportError::KindMismatch { table, column } if table == "accounts" && column == "ledger_account_id"
    ));
}

#[test]
fn text_that_is_not_utf8_is_refused() {
    let (_dir, worker) = populated();
    raw(&worker)
        .execute("UPDATE accounts SET name = CAST(x'ff' AS TEXT)", [])
        .unwrap();
    assert!(matches!(
        refusal(&worker),
        ExportError::KindMismatch { table, column } if table == "accounts" && column == "name"
    ));
}

#[test]
fn a_primary_key_over_256_bytes_is_refused_and_never_truncated() {
    let (_dir, worker) = populated();
    let key = format!("user.{}", "k".repeat(300));
    raw(&worker)
        .execute(
            "INSERT INTO settings (key, value, value_schema_version, updated_at)
             VALUES (?1, 'v', 1, '2026-03-01T00:00:00Z')",
            [key],
        )
        .unwrap();
    match refusal(&worker) {
        ExportError::KeyTooLong { table, column } => {
            assert_eq!((table.as_str(), column.as_str()), ("settings", "key"));
        }
        other => panic!("{other:?}"),
    }
    // A key of exactly 256 bytes of encoding is still fine: the tag byte, the length prefix and
    // the text come to 256.
    raw(&worker)
        .execute("DELETE FROM settings WHERE length(key) > 200", [])
        .unwrap();
    let fits = format!("user.{}", "k".repeat(256 - 5 - 4 - 1));
    raw(&worker)
        .execute(
            "INSERT INTO settings (key, value, value_schema_version, updated_at)
             VALUES (?1, 'v', 1, '2026-03-01T00:00:00Z')",
            [fits],
        )
        .unwrap();
    assert!(export(&worker).is_ok());
}

#[test]
fn a_folded_child_with_no_parent_is_refused() {
    let (_dir, worker) = populated();
    raw(&worker)
        .execute(
            "INSERT INTO ledger_postings
                (transaction_id, ledger_account_id, minor_units, currency, currency_exponent, posting_date)
             VALUES (?1, ?2, 5, 'USD', 2, '2026-03-01')",
            params![Uuid::from_u128(0xDEAD), Uuid::from_u128(0xBEEF)],
        )
        .unwrap();
    assert!(matches!(
        refusal(&worker),
        ExportError::OrphanChild { table } if table == "ledger_postings"
    ));
}

#[test]
fn a_value_over_two_to_the_24_bytes_is_refused_naming_its_table_and_column() {
    let (_dir, worker) = populated();
    // 8,388,609 bytes hex to 16,777,218 characters, two over the limit.
    raw(&worker)
        .execute("UPDATE accounts SET notes = hex(zeroblob(8388609))", [])
        .unwrap();
    match refusal(&worker) {
        ExportError::ValueTooLong { table, column } => {
            assert_eq!((table.as_str(), column.as_str()), ("accounts", "notes"));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_folded_child_row_over_65536_bytes_is_refused_naming_the_child_table() {
    let (_dir, worker) = populated();
    // No one column is too long (each is far under 2^24); the row is.
    raw(&worker)
        .execute(
            "UPDATE attachment_links SET entity_kind = hex(zeroblob(40000))",
            [],
        )
        .unwrap();
    match refusal(&worker) {
        ExportError::ChildRowTooLong { table } => assert_eq!(table, "attachment_links"),
        other => panic!("{other:?}"),
    }
}

/// The table and column an error names, if it names a column.
fn named_column(error: &ExportError) -> Option<&str> {
    match error {
        ExportError::KindMismatch { column, .. }
        | ExportError::KeyTooLong { column, .. }
        | ExportError::Unrepresentable { column, .. }
        | ExportError::ValueTooLong { column, .. } => Some(column),
        _ => None,
    }
}

#[test]
fn no_refusal_names_an_empty_table_or_column() {
    let scenarios: &[&str] = &[
        "UPDATE accounts SET retirement = 'not a number'",
        "UPDATE accounts SET retirement = 1.5",
        "UPDATE accounts SET ledger_account_id = x'0102'",
        "UPDATE accounts SET name = CAST(x'ff' AS TEXT)",
        "UPDATE accounts SET notes = hex(zeroblob(8388609))",
        "UPDATE attachment_links SET entity_id = x'0102'",
        "UPDATE attachment_links SET entity_kind = hex(zeroblob(40000))",
    ];
    for sql in scenarios {
        let (_dir, worker) = populated();
        raw(&worker).execute(sql, []).unwrap();
        let error = refusal(&worker);
        let message = error.to_string();
        if let Some(column) = named_column(&error) {
            assert!(!column.is_empty(), "{sql}: {message}");
        }
        assert!(
            !message.contains("table ,") && !message.contains("column :"),
            "{sql}: {message}"
        );
        assert!(!message.contains("SENTINEL"));
    }
}

#[test]
fn a_refusal_names_a_table_and_a_column_and_never_a_value() {
    let (_dir, worker) = populated();
    raw(&worker)
        .execute("UPDATE accounts SET retirement = 'SENTINEL-value-77'", [])
        .unwrap();
    let message = refusal(&worker).to_string();
    assert!(message.contains("accounts") && message.contains("retirement"));
    assert!(!message.contains("SENTINEL"));
}

#[test]
fn the_state_digest_of_a_known_table_matches_an_independent_hash() {
    // The same digest, computed here from the label by hand, for a table the fixture leaves empty.
    let (_dir, worker) = populated();
    let verified = verify(&export(&worker).unwrap()).unwrap();
    let entry = verified
        .manifest
        .tables
        .iter()
        .find(|t| t.row_count == "0" && t.state_digest.is_some())
        .expect("an empty keyed table");
    let mut message = Vec::new();
    message.extend_from_slice(&[0, 28]);
    message.extend_from_slice(b"dohflow/sync-state-digest/v1");
    message.extend_from_slice(&u32::try_from(entry.name.len()).unwrap().to_be_bytes());
    message.extend_from_slice(entry.name.as_bytes());
    message.extend_from_slice(&0u64.to_be_bytes());
    assert_eq!(
        entry.state_digest.as_deref().unwrap(),
        hex(&Sha256::digest(&message))
    );
}

#[test]
fn a_folded_childs_rows_are_in_child_row_byte_order_duplicates_kept() {
    let (_dir, worker) = populated();
    // Two identical postings count twice (the digest is a multiset), whatever their rowids.
    let conn = raw(&worker);
    // The balance triggers refuse an unbalanced posting; this scratch vault drops them.
    let triggers: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'trigger' AND tbl_name = 'ledger_postings'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    for trigger in triggers {
        conn.execute_batch(&format!("DROP TRIGGER {trigger}"))
            .unwrap();
    }
    conn.execute_batch(
        "INSERT INTO ledger_postings SELECT * FROM ledger_postings
            WHERE rowid = (SELECT MAX(rowid) FROM ledger_postings);
         INSERT INTO ledger_postings SELECT * FROM ledger_postings
            WHERE rowid = (SELECT MIN(rowid) FROM ledger_postings);",
    )
    .unwrap();
    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM ledger_postings", [], |r| r.get(0))
        .unwrap();
    let verified = verify(&export(&worker).unwrap()).unwrap();
    for table in ["ledger_postings", "attachment_links"] {
        let entry = verified
            .manifest
            .tables
            .iter()
            .find(|t| t.name == table)
            .unwrap();
        let content = &verified.files.iter().find(|(n, _)| n == table).unwrap().1;
        let rows = decode_rows(entry, content);
        if table == "ledger_postings" {
            assert_eq!(rows.len() as i64, total);
        }
        let encoded: Vec<Vec<u8>> = rows
            .iter()
            .map(|row| sync_core::row_digest::child_row_bytes(&columns_of(row)).unwrap())
            .collect();
        assert!(encoded.windows(2).all(|w| w[0] <= w[1]), "{table}");
    }
}

#[test]
fn a_column_that_never_leaves_is_not_validated_or_read_into_the_snapshot() {
    // `attachments.wrapped_content_key` is a local key and is not part of the projection: whatever
    // it holds, it neither stops the export nor reaches the plaintext.
    let (_dir, worker) = populated();
    raw(&worker)
        .execute(
            "UPDATE attachments SET wrapped_content_key = 'SENTINEL-local-key'",
            [],
        )
        .unwrap();
    let plaintext = export(&worker).unwrap();
    assert!(!contains(&plaintext, b"SENTINEL-local-key"));
}
