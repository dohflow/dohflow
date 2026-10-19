//! The column-kind manifest and the schema obligations of vault-format.md C.1.5
//! (personal-cfo-t2s4b part 5b).
//!
//! `table_columns.toml` lists every column of every table with the declared kind that tags its
//! values in the canonical row encoding (`int`, `text`, `uuid`, `bytes`, aligned with the
//! portable export), each table's primary key, and what the Sync write set covers. It is
//! **generated from the live migrated schema** plus the CLASS-0 manifest and checked in, so a
//! schema change shows up as a reviewed diff: this test fails until the file is regenerated
//! (`COLUMN_KINDS_UPDATE=1 cargo test -p db-worker --test column_kinds`) and the kinds of any
//! new column are reviewed.
//!
//! The schema obligations C.1.5 promises are tested here:
//! * no covered table has a REAL column (no table has one at all);
//! * every covered table declares an explicit, command-supplied primary key, and none relies
//!   on a SQLite-assigned rowid or `AUTOINCREMENT`;
//! * every covered key fits the 256-byte limit by construction;
//! * a field-split table is covered only through a projection of its replicated fields.
//!
//! Two replicated tables have no key and are **folded into a parent** (C.1.5 "Folded
//! children"); a third, `provenance_links`, is outside the write set (B.2 `exclude`). Three
//! text keys have no stored bound; they are named below as explicit exceptions (a ratchet that
//! may only shrink), to be closed by a write-time check or a documented construction bound.

mod common;

use std::collections::BTreeMap;
use std::fmt::Write as _;

use common::worker;

const CLASSES: &str = include_str!("../table_classes.toml");
const MANIFEST: &str = include_str!("../table_columns.toml");

/// Keyless replicated tables folded into a parent: `(child, parent, child column naming the
/// parent's `id`)`. Their rows are part of the parent's digest and never write-set entries.
const FOLDED: &[(&str, &str, &str)] = &[
    ("ledger_postings", "ledger_transactions", "transaction_id"),
    ("attachment_links", "attachments", "attachment_id"),
];

/// Tables the write set never covers although CLASS-0 gives them class 1: the command's own log
/// row carries the write-set hash, and `provenance_links` is excluded by B.2.
const NOT_REPLICATED: &[&str] = &["operation_log", "provenance_links"];

/// Covered text key columns with no bound enforced by the schema. Each needs a bound
/// (a length CHECK, or a write-time validation) so the 256-byte key limit is a schema-time
/// guarantee.
const UNBOUNDED_TEXT_KEYS: &[(&str, &str)] =
    &[("credit_card_cycles", "cycle_close"), ("settings", "key")];

/// Covered text keys the schema does not bound but the writer does, with a typed error and a
/// test (`merchant_alias_key_width.rs`): `merchant_aliases.normalized_key`.
const WRITER_BOUNDED_TEXT_KEYS: &[(&str, &str)] = &[("merchant_aliases", "normalized_key")];

/// BLOB columns that hold a 16-byte identifier although their name does not say so.
const UUID_BLOBS: &[(&str, &str)] = &[
    ("forecast_assumption_events", "superseded_by"),
    ("staged_balances", "account_ref"),
];

/// The tables the write set projects, and what each projection keeps.
struct Projection {
    source: &'static str,
    logical_table: &'static str,
    columns: &'static [&'static str],
    key: &'static [&'static str],
    /// A constant key for a singleton source row (the household record), as `int`.
    constant_key: Option<i64>,
    /// `settings` only: the row filter, a key prefix and the legacy unprefixed key names.
    key_prefixes: &'static [&'static str],
}

const PROJECTIONS: &[Projection] = &[
    Projection {
        source: "attachments",
        logical_table: "attachments",
        columns: &[
            "id",
            "plaintext_size",
            "mime_type",
            "original_filename",
            "created_at",
        ],
        key: &["id"],
        constant_key: None,
        key_prefixes: &[],
    },
    Projection {
        source: "settings",
        logical_table: "settings",
        columns: &["key", "value"],
        key: &["key"],
        constant_key: None,
        key_prefixes: &["user."],
    },
    Projection {
        source: "vault_metadata",
        logical_table: "household",
        columns: &["household_timezone"],
        key: &[],
        constant_key: Some(1),
        key_prefixes: &[],
    },
];

struct Col {
    name: String,
    declared: String,
    pk: i64,
}

/// `(table, class)` from CLASS-0, skipping rows that are not real tables.
fn classes() -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for block in CLASSES.split("[[table]]").skip(1) {
        let field = |key: &str| -> Option<String> {
            block.lines().find_map(|l| {
                let (k, v) = l.split_once('=')?;
                (k.trim() == key).then(|| v.trim().trim_matches('"').to_owned())
            })
        };
        if matches!(field("kind").as_deref(), Some("pseudo") | Some("planned")) {
            continue;
        }
        out.insert(field("table").unwrap(), field("class").unwrap());
    }
    out
}

fn legacy_settings_keys() -> Vec<String> {
    let line = CLASSES
        .lines()
        .find(|l| l.starts_with("legacy_persisted_keys"))
        .expect("legacy_persisted_keys");
    let value = line.split_once('=').unwrap().1.trim().trim_matches('"');
    value.split(',').map(str::to_owned).collect()
}

fn live_schema() -> BTreeMap<String, (Vec<Col>, bool)> {
    let (_dir, worker) = worker();
    let conn = worker.read_connection().unwrap();
    let names: Vec<String> = conn
        .prepare(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
             ORDER BY name",
        )
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let mut out = BTreeMap::new();
    for table in names {
        let ddl: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name = ?1",
                [&table],
                |r| r.get(0),
            )
            .unwrap();
        let autoincrement = ddl.to_uppercase().contains("AUTOINCREMENT");
        let cols: Vec<Col> = conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap()
            .query_map([], |r| {
                Ok(Col {
                    name: r.get(1)?,
                    declared: r.get::<_, String>(2)?.to_uppercase(),
                    pk: r.get(5)?,
                })
            })
            .unwrap()
            .map(Result::unwrap)
            .collect();
        out.insert(table, (cols, autoincrement));
    }
    out
}

fn kind_of(table: &str, col: &Col) -> &'static str {
    match col.declared.as_str() {
        "INTEGER" => "int",
        "TEXT" => "text",
        "BLOB" => {
            let uuid = col.name == "id"
                || col.name.ends_with("_id")
                || UUID_BLOBS.contains(&(table, col.name.as_str()));
            if uuid {
                "uuid"
            } else {
                "bytes"
            }
        }
        other => panic!("{table}.{}: declared type {other} has no kind", col.name),
    }
}

/// What the write set covers for a table.
fn write_set(table: &str, class: &str) -> &'static str {
    if PROJECTIONS.iter().any(|p| p.source == table) {
        return "projection";
    }
    if FOLDED.iter().any(|(child, _, _)| *child == table) {
        return "folded";
    }
    if NOT_REPLICATED.contains(&table) {
        return "none";
    }
    match class {
        "1" | "4" => "full",
        _ => "none",
    }
}

fn generate() -> String {
    let classes = classes();
    let schema = live_schema();
    let mut out = String::new();
    out.push_str(
        "# Column-kind manifest (vault-format.md C.1.5; personal-cfo-t2s4b part 5b).\n\
         #\n\
         # GENERATED by `COLUMN_KINDS_UPDATE=1 cargo test -p db-worker --test column_kinds` from the\n\
         # migrated schema and table_classes.toml; the test fails until this file matches. Review\n\
         # the kind of every new column in the diff: `uuid` is a BLOB of exactly 16 bytes, `bytes`\n\
         # any other BLOB. `write_set` is what the Sync write set covers for the table: `full` (all\n\
         # columns), `projection` (see [[projection]]), `folded` (a keyless child whose rows are part\n\
         # of its `parent`'s digest, C.1.5) or `none`.\n\n",
    );
    for (table, (cols, _)) in &schema {
        let class = classes
            .get(table)
            .unwrap_or_else(|| panic!("{table} is not classified"));
        let mut key: Vec<&Col> = cols.iter().filter(|c| c.pk > 0).collect();
        key.sort_by_key(|c| c.pk);
        let key_names: Vec<String> = key.iter().map(|c| format!("\"{}\"", c.name)).collect();
        let columns: Vec<String> = cols
            .iter()
            .map(|c| format!("\"{}:{}\"", c.name, kind_of(table, c)))
            .collect();
        writeln!(out, "[[table]]").unwrap();
        writeln!(out, "name = \"{table}\"").unwrap();
        writeln!(out, "class = \"{class}\"").unwrap();
        writeln!(out, "write_set = \"{}\"", write_set(table, class)).unwrap();
        writeln!(out, "primary_key = [{}]", key_names.join(", ")).unwrap();
        if let Some((_, parent, column)) = FOLDED.iter().find(|(child, _, _)| child == table) {
            writeln!(out, "parent = \"{parent}\"").unwrap();
            writeln!(out, "parent_column = \"{column}\"").unwrap();
        }
        let children: Vec<String> = FOLDED
            .iter()
            .filter(|(_, parent, _)| parent == table)
            .map(|(child, _, _)| format!("\"{child}\""))
            .collect();
        if !children.is_empty() {
            writeln!(out, "children = [{}]", children.join(", ")).unwrap();
        }
        writeln!(out, "columns = [{}]\n", columns.join(", ")).unwrap();
    }
    let legacy = legacy_settings_keys();
    for p in PROJECTIONS {
        writeln!(out, "[[projection]]").unwrap();
        writeln!(out, "source = \"{}\"", p.source).unwrap();
        writeln!(out, "logical_table = \"{}\"", p.logical_table).unwrap();
        let cols: Vec<String> = p.columns.iter().map(|c| format!("\"{c}\"")).collect();
        writeln!(out, "columns = [{}]", cols.join(", ")).unwrap();
        let key: Vec<String> = p.key.iter().map(|c| format!("\"{c}\"")).collect();
        writeln!(out, "key = [{}]", key.join(", ")).unwrap();
        if let Some(k) = p.constant_key {
            writeln!(out, "constant_key_int = {k}").unwrap();
        }
        if !p.key_prefixes.is_empty() {
            let prefixes: Vec<String> = p.key_prefixes.iter().map(|c| format!("\"{c}\"")).collect();
            writeln!(out, "key_prefixes = [{}]", prefixes.join(", ")).unwrap();
            let names: Vec<String> = legacy.iter().map(|c| format!("\"{c}\"")).collect();
            writeln!(out, "legacy_key_names = [{}]", names.join(", ")).unwrap();
        }
        out.push('\n');
    }
    // No trailing blank line (`git diff --check`).
    while out.ends_with("\n\n") {
        out.pop();
    }
    out
}

#[test]
fn the_checked_in_manifest_is_the_generated_one() {
    let generated = generate();
    if std::env::var_os("COLUMN_KINDS_UPDATE").is_some() {
        std::fs::write(
            concat!(env!("CARGO_MANIFEST_DIR"), "/table_columns.toml"),
            &generated,
        )
        .unwrap();
        return;
    }
    assert_eq!(
        MANIFEST, generated,
        "table_columns.toml is stale: regenerate with COLUMN_KINDS_UPDATE=1 and review the kinds"
    );
}

#[test]
fn every_classified_table_is_in_the_manifest_and_nothing_else_is() {
    let manifest_tables: Vec<String> = MANIFEST
        .lines()
        .filter_map(|l| l.strip_prefix("name = \""))
        .map(|l| l.trim_end_matches('"').to_owned())
        .collect();
    let mut expected: Vec<String> = classes().keys().cloned().collect();
    expected.sort();
    let mut actual = manifest_tables;
    actual.sort();
    assert_eq!(actual, expected);
}

#[test]
fn no_table_has_a_real_column_or_an_untagged_declared_type() {
    for (table, (cols, _)) in live_schema() {
        for col in cols {
            assert!(
                matches!(col.declared.as_str(), "INTEGER" | "TEXT" | "BLOB"),
                "{table}.{}: declared type {} has no tag (REAL, NUMERIC and the like are refused)",
                col.name,
                col.declared
            );
        }
    }
}

#[test]
fn covered_tables_declare_explicit_command_supplied_keys() {
    let classes = classes();
    let schema = live_schema();
    let mut keyless = Vec::new();
    for (table, (cols, autoincrement)) in &schema {
        let covered = write_set(table, &classes[table]) != "none";
        if !covered {
            continue;
        }
        assert!(
            !autoincrement,
            "{table}: an AUTOINCREMENT key is allocated by SQLite"
        );
        let key: Vec<&Col> = cols.iter().filter(|c| c.pk > 0).collect();
        if PROJECTIONS.iter().any(|p| p.source == table.as_str()) {
            continue;
        }
        if key.is_empty() {
            keyless.push(table.as_str());
            continue;
        }
        // A single INTEGER PRIMARY KEY is a rowid alias that SQLite assigns when omitted.
        assert!(
            !(key.len() == 1 && key[0].declared == "INTEGER"),
            "{table}.{}: an INTEGER PRIMARY KEY is assigned by SQLite when omitted",
            key[0].name
        );
    }
    // Every covered table without a key is a folded child and every folded child is keyless:
    // a table that gains a key stops being folded, and a new keyless table must be folded.
    let mut folded: Vec<&str> = FOLDED.iter().map(|(child, _, _)| *child).collect();
    folded.sort_unstable();
    keyless.sort_unstable();
    assert_eq!(
        keyless, folded,
        "the keyless covered tables and FOLDED must be the same set"
    );
    for (child, parent, column) in FOLDED {
        let (child_cols, _) = &schema[*child];
        assert!(
            child_cols
                .iter()
                .any(|c| c.name == *column && kind_of(child, c) == "uuid"),
            "{child}.{column} must be a uuid column naming the parent"
        );
        let (parent_cols, _) = &schema[*parent];
        let key: Vec<&str> = parent_cols
            .iter()
            .filter(|c| c.pk > 0)
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(
            key,
            ["id"],
            "{parent} must have the single key `id` that {child} folds into"
        );
        assert!(
            matches!(write_set(parent, &classes[*parent]), "full" | "projection"),
            "{parent} must be a write-set entry"
        );
    }
    assert_eq!(
        write_set("provenance_links", &classes["provenance_links"]),
        "none"
    );
}

#[test]
fn covered_key_widths_fit_the_cap_by_construction() {
    let classes = classes();
    let schema = live_schema();
    let mut unbounded = Vec::new();
    for (table, (cols, _)) in &schema {
        if write_set(table, &classes[table]) == "none" {
            continue;
        }
        let key: Vec<&Col> = {
            let mut k: Vec<&Col> = cols.iter().filter(|c| c.pk > 0).collect();
            k.sort_by_key(|c| c.pk);
            k
        };
        let mut width = 0;
        for col in key {
            match kind_of(table, col) {
                "int" => width += 9,
                "uuid" => width += 17,
                "text" => unbounded.push((table.as_str(), col.name.as_str())),
                other => panic!("{table}.{}: a {other} key has no bound", col.name),
            }
        }
        assert!(width <= 256, "{table}: fixed key width {width} exceeds 256");
    }
    unbounded.retain(|key| !WRITER_BOUNDED_TEXT_KEYS.contains(key));
    assert_eq!(
        unbounded, UNBOUNDED_TEXT_KEYS,
        "the set of covered text keys with no schema bound changed: it may only shrink"
    );
}

#[test]
fn projections_name_real_columns_and_the_field_split_tables_are_all_projected() {
    let schema = live_schema();
    for p in PROJECTIONS {
        let (cols, _) = &schema[p.source];
        let names: Vec<&str> = cols.iter().map(|c| c.name.as_str()).collect();
        for column in p.columns.iter().chain(p.key) {
            assert!(
                names.contains(column),
                "{}: projection names missing column {column}",
                p.source
            );
        }
        // The device-local columns of the attachments row never enter a digest.
        if p.source == "attachments" {
            for local in [
                "storage_id",
                "wrapped_content_key",
                "content_key_nonce",
                "content_nonce",
                "ref_count",
                "content_alg",
            ] {
                assert!(
                    !p.columns.contains(&local),
                    "{local} is device-local or derived"
                );
            }
        }
        if p.source == "settings" {
            assert!(!p.key_prefixes.iter().any(|x| x.starts_with("device.")));
        }
    }
    // Every table whose CLASS-0 row names canonical/receiver-local fields is projected or full.
    for block in CLASSES.split("[[table]]").skip(1) {
        if block.contains("receiver_local_fields = \"")
            && !block.contains("receiver_local_fields = \"\"")
        {
            let table = block
                .lines()
                .find_map(|l| l.strip_prefix("table = \""))
                .unwrap()
                .trim_end_matches('"');
            assert!(
                PROJECTIONS.iter().any(|p| p.source == table),
                "{table} has receiver-local fields, so it must be covered only through a projection"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The encoder over a populated vault
// ---------------------------------------------------------------------------

use common::{bill_cmd, create_account_cmd, income_cmd, liquid_account_cmd, meta};
use rusqlite::types::ValueRef;
use sync_core::row_digest::{
    parent_row_digest, primary_key, row_digest, ChildTable, Column, Kind, Value,
};

struct TableSpec {
    key: Vec<String>,
    columns: Vec<(String, Kind)>,
    write_set: String,
    /// Folded child tables of this parent, if any.
    children: Vec<String>,
    /// For a folded child: its parent table and the column naming the parent's `id`.
    parent: Option<(String, String)>,
}

fn parse_kind(kind: &str) -> Kind {
    match kind {
        "int" => Kind::Int,
        "text" => Kind::Text,
        "uuid" => Kind::Uuid,
        "bytes" => Kind::Bytes,
        other => panic!("unknown kind {other}"),
    }
}

fn quoted_list(line: &str) -> Vec<String> {
    line.split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect()
}

/// The manifest's tables, parsed from the checked-in file.
fn manifest_tables() -> BTreeMap<String, TableSpec> {
    let mut out = BTreeMap::new();
    for block in MANIFEST.split("[[table]]").skip(1) {
        let block = block.split("[[projection]]").next().unwrap();
        let line = |key: &str| {
            block
                .lines()
                .find(|l| l.starts_with(&format!("{key} = ")))
                .unwrap_or_else(|| panic!("missing {key}"))
                .to_owned()
        };
        let name = quoted_list(&line("name")).remove(0);
        let columns = quoted_list(&line("columns"))
            .into_iter()
            .map(|c| {
                let (n, k) = c.split_once(':').unwrap();
                (n.to_owned(), parse_kind(k))
            })
            .collect();
        let optional = |key: &str| {
            block
                .lines()
                .find(|l| l.starts_with(&format!("{key} = ")))
                .map(quoted_list)
        };
        let parent = optional("parent").map(|mut p| {
            let column = optional("parent_column").unwrap().remove(0);
            (p.remove(0), column)
        });
        out.insert(
            name,
            TableSpec {
                key: quoted_list(&line("primary_key")),
                columns,
                write_set: quoted_list(&line("write_set")).remove(0),
                children: optional("children").unwrap_or_default(),
                parent,
            },
        );
    }
    out
}

fn value_of(value: ValueRef<'_>) -> Value<'_> {
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(v) => Value::Int(v),
        ValueRef::Text(bytes) => Value::Text(bytes),
        ValueRef::Blob(bytes) => Value::Blob(bytes),
        ValueRef::Real(_) => panic!("a REAL value in a covered table"),
    }
}

#[test]
fn every_row_of_a_populated_vault_encodes_under_its_declared_kinds() {
    let (_dir, worker) = worker();
    let account = create_account_cmd();
    worker.dispatch(meta(), account).unwrap();
    worker.dispatch(meta(), income_cmd(None)).unwrap();
    worker.dispatch(meta(), bill_cmd(None)).unwrap();
    // An account with an opening balance writes a ledger transaction with postings.
    worker
        .dispatch(
            meta(),
            liquid_account_cmd(
                core_ledger::AccountId::new(),
                core_money::Currency::Usd,
                125_000,
            ),
        )
        .unwrap();

    let specs = manifest_tables();
    let conn = worker.read_connection().unwrap();
    let mut encoded: BTreeMap<String, usize> = BTreeMap::new();
    for (table, spec) in &specs {
        // A projection encodes only its replicated columns and a folded child is part of its
        // parent's digest; this loop covers every full-width, keyed table.
        if matches!(spec.write_set.as_str(), "none" | "projection" | "folded") {
            continue;
        }
        let names: Vec<&str> = spec.columns.iter().map(|(n, _)| n.as_str()).collect();
        let sql = format!("SELECT {} FROM {table}", names.join(", "));
        let mut stmt = conn.prepare(&sql).unwrap();
        let mut rows = stmt.query([]).unwrap();
        while let Some(row) = rows.next().unwrap() {
            let columns: Vec<Column<'_>> = spec
                .columns
                .iter()
                .enumerate()
                .map(|(i, (name, kind))| Column {
                    name,
                    kind: *kind,
                    value: value_of(row.get_ref(i).unwrap()),
                })
                .collect();
            row_digest(table, &columns).unwrap_or_else(|e| panic!("{table}: {e}"));
            if !spec.key.is_empty() {
                let key: Vec<(Kind, Value<'_>)> = spec
                    .key
                    .iter()
                    .map(|k| {
                        let column = columns.iter().find(|c| c.name == k).unwrap();
                        (column.kind, column.value)
                    })
                    .collect();
                primary_key(&key).unwrap_or_else(|e| panic!("{table} key: {e}"));
            }
            *encoded.entry(table.clone()).or_default() += 1;
        }
    }
    // The workload populated the core tables; if one stays empty the check above proved
    // nothing for it.
    for table in [
        "accounts",
        "ledger_accounts",
        "income_sources",
        "recurring_events",
        "categories",
    ] {
        assert!(
            encoded.get(table).copied().unwrap_or(0) > 0,
            "{table} was not exercised"
        );
    }
}

/// Read every row of `table` (all manifest columns) as owned values, in the given SQL order.
fn read_rows(
    conn: &rusqlite::Connection,
    spec: &TableSpec,
    table: &str,
    filter: &str,
    order: &str,
    parent_id: &[u8],
) -> Vec<Vec<(String, Kind, rusqlite::types::Value)>> {
    let names: Vec<&str> = spec.columns.iter().map(|(n, _)| n.as_str()).collect();
    let sql = format!(
        "SELECT {} FROM {table} WHERE {filter} = ?1 ORDER BY {order}",
        names.join(", ")
    );
    let mut stmt = conn.prepare(&sql).unwrap();
    stmt.query_map([parent_id], |row| {
        spec.columns
            .iter()
            .enumerate()
            .map(|(i, (name, kind))| {
                Ok((
                    name.clone(),
                    *kind,
                    row.get::<_, rusqlite::types::Value>(i)?,
                ))
            })
            .collect()
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

fn as_value(value: &rusqlite::types::Value) -> Value<'_> {
    match value {
        rusqlite::types::Value::Null => Value::Null,
        rusqlite::types::Value::Integer(v) => Value::Int(*v),
        rusqlite::types::Value::Text(t) => Value::Text(t.as_bytes()),
        rusqlite::types::Value::Blob(b) => Value::Blob(b),
        rusqlite::types::Value::Real(_) => panic!("a REAL value in a covered table"),
    }
}

fn as_columns(row: &[(String, Kind, rusqlite::types::Value)]) -> Vec<Column<'_>> {
    row.iter()
        .map(|(n, k, v)| Column {
            name: n,
            kind: *k,
            value: as_value(v),
        })
        .collect()
}

#[test]
fn a_parent_digest_with_folded_children_is_independent_of_row_order_and_counts_duplicates() {
    let (_dir, worker) = worker();
    worker
        .dispatch(
            meta(),
            liquid_account_cmd(
                core_ledger::AccountId::new(),
                core_money::Currency::Usd,
                125_000,
            ),
        )
        .unwrap();
    let specs = manifest_tables();
    let conn = worker.read_connection().unwrap();
    let parent_spec = &specs["ledger_transactions"];
    assert_eq!(parent_spec.children, ["ledger_postings"]);
    let (child_parent, child_column) = specs["ledger_postings"].parent.clone().unwrap();
    assert_eq!(
        (child_parent.as_str(), child_column.as_str()),
        ("ledger_transactions", "transaction_id")
    );

    // Every ledger transaction, with its postings read in both rowid orders.
    let ids: Vec<Vec<u8>> = conn
        .prepare("SELECT id FROM ledger_transactions")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(!ids.is_empty());
    let mut checked_children = 0;
    for id in ids {
        let parent_rows = read_rows(&conn, parent_spec, "ledger_transactions", "id", "id", &id);
        assert_eq!(parent_rows.len(), 1);
        let digest = |order: &str, drop_last: bool| {
            let mut child_rows = read_rows(
                &conn,
                &specs["ledger_postings"],
                "ledger_postings",
                &child_column,
                order,
                &id,
            );
            if drop_last {
                child_rows.pop();
            }
            let children = vec![ChildTable {
                table: "ledger_postings",
                rows: child_rows.iter().map(|r| as_columns(r)).collect(),
            }];
            parent_row_digest(
                "ledger_transactions",
                &as_columns(&parent_rows[0]),
                &children,
            )
            .unwrap()
        };
        let forward = digest("rowid ASC", false);
        assert_eq!(
            forward,
            digest("rowid DESC", false),
            "child order is not semantic"
        );
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM ledger_postings WHERE transaction_id = ?1",
                [&id],
                |r| r.get(0),
            )
            .unwrap();
        checked_children += count;
        if count > 0 {
            assert_ne!(
                forward,
                digest("rowid ASC", true),
                "removing a child changes the parent digest"
            );
        }
    }
    assert!(
        checked_children >= 2,
        "the opening balance wrote postings ({checked_children})"
    );
}
