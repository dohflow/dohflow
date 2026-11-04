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

use common::replicated_state::*;
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

/// How each covered text key column is bounded, since SQLite bounds nothing. `(table, column,
/// maximum UTF-8 bytes, mechanism)`. The manifest records the maximum, and a test per key
/// proves the mechanism: a writer check with a typed error, or a bound by construction.
/// Every covered text key must appear here (no unbounded key remains), and the widest
/// possible key must fit the 256-byte limit of C.1.5.
const TEXT_KEY_BOUNDS: &[(&str, &str, usize, &str)] = &[
    // `MintMerchantAnchors` rejects a longer key (merchant_alias_key_width.rs).
    ("merchant_aliases", "normalized_key", 251, "writer"),
    // `set_device_setting` rejects a longer key; every user.* key is a fixed registry name
    // (settings_key_width.rs).
    ("settings", "key", 128, "writer"),
    // `NaiveDate::to_string()` is `YYYY-MM-DD`, or at most 13 bytes for the extreme years
    // (settings_key_width.rs checks the extremes).
    ("credit_card_cycles", "cycle_close", 16, "construction"),
    // `upsert_connector_link` rejects a longer provider account id (connector_link_key_width.rs);
    // the primary key is (connection_id uuid, external_id text): 256 - 17 - 5 = 234.
    ("connector_account_links", "external_id", 234, "writer"),
    // The role is one of the system-account constants (`opening_balance_equity`,
    // `unmatched_expense`, `unmatched_income`: at most 22 bytes) and the currency an ISO code
    // from the `Currency` enum (3 bytes); no caller passes free text.
    ("system_ledger_accounts", "role", 32, "construction"),
    ("system_ledger_accounts", "currency", 8, "construction"),
];

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
    /// A constant key for a singleton source row, as `int` (none today; the household record is
    /// the class-1 `households` table since schema version 61).
    constant_key: Option<i64>,
    /// `settings` only: the row filter, the replicated key prefixes.
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
    for (table, column, max, mechanism) in TEXT_KEY_BOUNDS {
        writeln!(out, "[[key_bound]]").unwrap();
        writeln!(out, "table = \"{table}\"").unwrap();
        writeln!(out, "column = \"{column}\"").unwrap();
        writeln!(out, "max_bytes = {max}").unwrap();
        writeln!(out, "mechanism = \"{mechanism}\"\n").unwrap();
    }
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
    // Every covered text key has a recorded bound, and the bound plus the other key columns
    // fits the 256-byte cap (a text key is a tag byte, a u32 length and the bytes).
    unbounded.sort_unstable();
    let mut bounded: Vec<(&str, &str)> = TEXT_KEY_BOUNDS
        .iter()
        .map(|(t, c, _, _)| (*t, *c))
        .collect();
    bounded.sort_unstable();
    assert_eq!(
        unbounded, bounded,
        "every covered text key needs an entry in TEXT_KEY_BOUNDS"
    );
    for (table, column, max, _) in TEXT_KEY_BOUNDS {
        let (cols, _) = &schema[*table];
        let mut width = 0;
        for col in cols.iter().filter(|c| c.pk > 0) {
            width += match kind_of(table, col) {
                "int" => 9,
                "uuid" => 17,
                "text" => 5 + if col.name == *column { *max } else { 0 },
                other => panic!("{other} key"),
            };
        }
        assert!(
            width <= 256,
            "{table}.{column}: widest key {width} exceeds 256"
        );
    }
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
use sync_core::row_digest::{
    parent_row_digest, primary_key, row_digest, ChildTable, Column, Kind, Value,
};

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

// ---------------------------------------------------------------------------
// The write set against an independent oracle: a full before/after dump
// ---------------------------------------------------------------------------

use db_worker::WriteCommand;

#[test]
fn the_write_set_of_real_commands_equals_the_state_difference_of_full_dumps() {
    let (_dir, worker) = worker();
    let account = core_ledger::AccountId::new();
    let txn = core_ledger::TransactionId::new();
    let commands: Vec<(&str, WriteCommand)> = vec![
        (
            "create account with opening balance",
            liquid_account_cmd(account, core_money::Currency::Usd, 125_000),
        ),
        ("create income source", income_cmd(Some(account))),
        ("create recurring bill", bill_cmd(Some(account))),
        (
            "record a transaction",
            WriteCommand::RecordTransaction {
                transaction_id: txn,
                account_id: account,
                amount: core_money::Money::new(-4_250, core_money::Currency::Usd),
                occurred_at: chrono::Utc::now(),
            },
        ),
        (
            "set a note",
            WriteCommand::SetNote {
                transaction_id: txn,
                note: Some("coffee".to_owned()),
            },
        ),
        (
            "set the household timezone",
            WriteCommand::SetHouseholdTimezone {
                timezone: "America/Chicago".to_owned(),
            },
        ),
        (
            "void the transaction",
            WriteCommand::VoidTransaction {
                transaction_id: txn,
            },
        ),
    ];
    let mut seen_nonempty = 0;
    for (label, command) in commands {
        let before = replicated_state(&worker.read_connection().unwrap());
        worker
            .dispatch(meta(), command)
            .unwrap_or_else(|e| panic!("{label}: {e}"));
        let after = replicated_state(&worker.read_connection().unwrap());
        let captured = worker.last_write_set().expect("a write set");
        assert_eq!(
            captured.hash,
            oracle(&before, &after),
            "{label}: the capture disagrees with the dump diff"
        );
        if !captured.rows.is_empty() {
            seen_nonempty += 1;
        }
    }
    assert!(
        seen_nonempty >= 6,
        "the commands exercised the capture ({seen_nonempty})"
    );
}

// ---------------------------------------------------------------------------
// The oracle for EVERY command variant (personal-cfo-vpza7)
// ---------------------------------------------------------------------------
//
// The test above proves the write-set capture for seven hand-picked commands. This one runs the
// deterministic sample of every `WriteCommand` variant (`common::samples`, the payload_v1 samples)
// in dependency order on one vault, and for each compares the capture's hash with the hash of the
// difference between two full dumps of the replicated state taken with no triggers. A variant whose
// apply writes a replicated row the capture mishandles (a cascade, a REPLACE, a folded child, a key
// change) disagrees with its dump diff. A variant with no setup fails by name: it is never exempt.

/// The fully captured tables, from the checked-in manifest.
fn full_capture_tables() -> Vec<String> {
    manifest_tables()
        .iter()
        .filter(|(_, spec)| spec.write_set == "full")
        .map(|(name, _)| name.clone())
        .collect()
}

/// The (table, write kind) pairs that none of the 56 command variants performs on a fresh sample
/// run, so the all-variants oracle does not prove the capture trigger for them (`bi` insert, `bu`
/// update, `bd` delete). Why, by group:
///
/// * delete (`bd`) is absent where the table is only ever tombstoned or archived (an UPDATE) or
///   never deleted by a command;
/// * the rows of `category_aliases`, `credit_card_cycles`,
///   `manual_entry_links` is written by raw (non-command) APIs today and reaches the command bus
///   with `personal-cfo-5ymg8`; `merchant_aliases` and `merchant_identities` are only ever
///   inserted (a mint never updates or deletes one);
/// * the rest need a second command on the same row (an update after a create).
///
/// `ledger_transactions` insert IS exercised but its trigger is an equivalent mutant: the folded
/// `ledger_postings` child marks the parent touched, so dropping the parent's INSERT trigger
/// changes nothing observable.
const UNEXERCISED: &[(&str, &[&str])] = &[
    ("accounts", &["bd"]),
    ("balance_observations", &["bd"]),
    ("bill_contracts", &["bd"]),
    ("categories", &["bd"]),
    ("category_aliases", &["bi", "bu", "bd"]),
    ("change_journal_entries", &["bu", "bd"]),
    ("confirmed_obligations", &["bu"]),
    ("connector_account_links", &["bd"]),
    ("connector_connections", &["bd"]),
    ("credit_card_cycles", &["bi", "bu", "bd"]),
    ("credit_card_statements", &["bu", "bd"]),
    ("debt_terms", &["bu", "bd"]),
    ("dedupe_decisions", &["bu", "bd"]),
    ("forecast_assumption_events", &["bd"]),
    ("household_members", &["bd"]),
    ("households", &["bi", "bd"]),
    // The map is insert-only: a command meets a (role, currency) pair no seed created (a euro
    // account with an opening balance) and inserts it; nothing updates or deletes a row.
    ("system_ledger_accounts", &["bu", "bd"]),
    ("income_sources", &["bd"]),
    ("ledger_accounts", &["bu", "bd"]),
    ("ledger_transactions", &["bd"]),
    ("manual_entry_links", &["bi", "bu", "bd"]),
    ("merchant_aliases", &["bu", "bd"]),
    ("merchant_identities", &["bu", "bd"]),
    ("profiles", &["bu", "bd"]),
    ("recurring_event_tags", &["bu", "bd"]),
    ("recurring_events", &["bd"]),
    ("recurring_transfers", &["bd"]),
    ("scenarios", &["bd"]),
    ("split_line_tags", &["bu", "bd"]),
    ("split_lines", &["bu", "bd"]),
    ("tags", &["bu", "bd"]),
    ("transaction_categorizations", &["bd"]),
    ("transaction_details", &["bu", "bd"]),
    ("transaction_reviews", &["bu", "bd"]),
    ("transaction_tags", &["bu", "bd"]),
];

mod all_variants {
    use super::common::samples::{samples, txn, BEYOND_DOUBLE};
    use super::common::variant_runner::*;
    use super::{full_capture_tables, meta, oracle, replicated_state, worker, UNEXERCISED};
    use core_ledger::{CategoryId, RecurringEventId, TagId};
    use db_worker::{DbWorker, WriteCommand};

    fn run_checked(
        worker: &DbWorker,
        label: &str,
        command: WriteCommand,
        exercised: &mut std::collections::BTreeSet<(String, &'static str)>,
    ) -> bool {
        let before = replicated_state(&worker.read_connection().unwrap());
        worker
            .dispatch(meta(), command)
            .unwrap_or_else(|e| panic!("{label}: the command did not apply: {e}"));
        let after = replicated_state(&worker.read_connection().unwrap());
        let captured = worker.last_write_set().expect("a write set");
        assert_eq!(
            captured.hash,
            oracle(&before, &after),
            "{label}: the capture disagrees with the dump diff"
        );
        for (key, digest) in &after {
            match before.get(key) {
                None => exercised.insert((key.0.clone(), "bi")),
                Some(d) if d != digest => exercised.insert((key.0.clone(), "bu")),
                _ => false,
            };
        }
        for key in before.keys().filter(|k| !after.contains_key(*k)) {
            exercised.insert((key.0.clone(), "bd"));
        }
        !captured.rows.is_empty()
    }

    #[test]
    fn the_write_set_of_every_command_variant_equals_the_state_difference_of_full_dumps() {
        // The order covers exactly the samples: no variant is left out and none is extra.
        let mut ordered: Vec<&str> = ORDER.to_vec();
        ordered.sort_unstable();
        let mut sampled: Vec<&str> = samples().iter().map(|(n, _)| *n).collect();
        sampled.sort_unstable();
        assert_eq!(
            ordered, sampled,
            "ORDER must place every WriteCommand variant exactly once"
        );

        let (_dir, worker) = worker();
        let mut rt = Runtime {
            primary: Some(db_worker::households::primary_member_id(
                &worker.vault_metadata().unwrap().vault_id,
            )),
            ..Runtime::default()
        };
        let mut by_name: std::collections::BTreeMap<&str, WriteCommand> =
            samples().into_iter().collect();
        let mut exercised = std::collections::BTreeSet::new();
        let mut checked = 0;
        let mut nonempty = 0;
        for name in ORDER {
            let sample = by_name.remove(name).expect("a sample per variant");
            for (label, setup) in before(name, &worker, &mut rt) {
                nonempty += usize::from(run_checked(&worker, label, setup, &mut exercised));
            }
            let command = command_for(name, sample, &rt);
            nonempty += usize::from(run_checked(&worker, name, command, &mut exercised));
            checked += 1;
        }
        assert_eq!(checked, 84, "every variant ran");
        assert!(by_name.is_empty());
        assert!(
            nonempty >= 40,
            "the capture was exercised across the variants ({nonempty})"
        );
        // Which kinds of write the 56 variants actually performed, per fully captured table (the
        // net effect of a command on a row: created, changed, removed; they are what the capture
        // triggers for INSERT, UPDATE and DELETE decide). A kind no variant performs is not proven
        // by this test, so the set of them is pinned: a command that starts performing one fails
        // here until the list shrinks, and a new table fails until it is listed or exercised.
        let mut unexercised: Vec<String> = full_capture_tables()
            .iter()
            .flat_map(|t| ["bi", "bu", "bd"].map(|k| (t.clone(), k)))
            .filter(|pair| !exercised.contains(pair))
            .map(|(t, k)| format!("{t}:{k}"))
            .collect();
        unexercised.sort();
        let mut pinned: Vec<String> = UNEXERCISED
            .iter()
            .flat_map(|(t, kinds)| kinds.iter().map(move |k| format!("{t}:{k}")))
            .collect();
        pinned.sort();
        assert_eq!(
            unexercised, pinned,
            "the write kinds no command variant performs changed: update UNEXERCISED"
        );
        // Unused helpers keep the import list honest for the next variant that needs them.
        let _ = (
            txn(1),
            BEYOND_DOUBLE,
            CategoryId::new(),
            RecurringEventId::new(),
            TagId::new(),
        );
    }
}
