//! Capturing the rows a command changed, and hashing them (vault-format.md C.1.5 and C.4;
//! personal-cfo-t2s4b part 5c).
//!
//! The write set is a **state difference**: exactly the replicated rows whose final state
//! differs from their state before the command. SQLite offers no cheap way to ask for that
//! afterwards, so the writer connection carries `TEMP` triggers, generated from the column-kind
//! manifest, that snapshot a row's *before image* the first time a command touches it:
//!
//! * a `BEFORE INSERT/UPDATE/DELETE` trigger on every replicated table copies the old row (or a
//!   "did not exist" marker for a key that is about to appear) into a per-table temp shadow
//!   table, guarded by `NOT EXISTS` so only the first touch counts (never `OR IGNORE`: the
//!   conflict policy of the statement that fires a trigger, an UPSERT or a `REPLACE`,
//!   overrides the one written inside it);
//! * a keyless **folded child** (`ledger_postings`, `attachment_links`) snapshots all of its
//!   parent's children the first time any of them changes, and marks the parent as touched, since
//!   the children are part of the parent's digest;
//! * `PRAGMA recursive_triggers` is on, so a `REPLACE` conflict resolution fires the delete
//!   trigger too, and foreign-key actions (cascades) fire them as for any write.
//!
//! [`finish`] runs at the end of the command's transaction, before the operation-log row: it
//! compares each touched row's before digest with its after digest, keeps the ones that
//! differ, hashes them with [`sync_core::write_set::write_set_hash`] and empties the shadow
//! tables. `TEMP` objects live in the connection, not in the encrypted file, and take part in
//! the transaction, so a rolled-back command leaves them empty. Class-2 and class-3 tables
//! have no triggers, so a rebuild is excluded structurally, not by a filter.
//!
//! Why triggers and not SQLite's preupdate hook: the hook needs `libsqlite3-sys`'s
//! `preupdate_hook` feature, which forces `bindgen` and libclang into every build, including the
//! iOS cross-compile.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use rusqlite::types::{ToSqlOutput, ValueRef};
use rusqlite::Connection;
use sync_core::row_digest::{
    absent_digest, parent_row_digest, primary_key, row_digest, ChildTable, Column, Kind, Value,
};
use sync_core::write_set::{write_set_hash, Entry};

use crate::column_manifest::{Coverage, Manifest, Projection, TableSpec};
use crate::DbError;

#[cfg(test)]
thread_local! {
    /// Test-only switch that turns the capture off on the calling thread, so a test can time a
    /// command with and without it. The worker applies a command on the caller's thread.
    pub(crate) static DISABLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn disabled() -> bool {
    #[cfg(test)]
    {
        DISABLED.with(std::cell::Cell::get)
    }
    #[cfg(not(test))]
    {
        false
    }
}

/// The result of a command's capture: the hash and the entries it covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteSet {
    /// The write-set hash (C.4).
    pub hash: [u8; 32],
    /// The `(table, primary_key)` of every changed replicated row, in canonical order.
    pub rows: Vec<(String, Vec<u8>)>,
}

fn shadow(table: &str) -> String {
    format!("ws_old_{table}")
}

fn ident_list(cols: &[&str]) -> String {
    cols.join(", ")
}

/// The tables that get a before-image shadow and triggers: every table the write set covers
/// that is not a folded child.
fn keyed_tables(manifest: &Manifest) -> Vec<&TableSpec> {
    manifest
        .tables
        .values()
        .filter(|t| matches!(t.coverage, Coverage::Full | Coverage::Projection))
        .collect()
}

fn folded_children(manifest: &Manifest) -> Vec<&TableSpec> {
    manifest
        .tables
        .values()
        .filter(|t| t.coverage == Coverage::Folded)
        .collect()
}

/// Every capture table, in the form the dirty check needs.
fn capture_tables(manifest: &Manifest) -> Vec<String> {
    let mut names = Vec::new();
    for table in keyed_tables(manifest) {
        names.push(shadow(&table.name));
        if !table.children.is_empty() {
            names.push(format!("ws_pt_{}", table.name));
        }
    }
    for child in folded_children(manifest) {
        names.push(format!("ws_cb_{}", child.name));
        names.push(format!("ws_cs_{}", child.name));
    }
    names
}

/// One statement that names every capture table currently holding a row. A command touches a
/// handful of the ~30 tables, so both [`finish`] and [`clear`] work from this list instead of
/// visiting every table (the visit was most of the capture's cost).
fn dirty_sql(manifest: &Manifest) -> &'static str {
    static SQL: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    SQL.get_or_init(|| {
        capture_tables(manifest)
            .iter()
            .map(|t| format!("SELECT '{t}' WHERE EXISTS (SELECT 1 FROM temp.{t})"))
            .collect::<Vec<_>>()
            .join(" UNION ALL ")
    })
}

fn dirty_tables(conn: &Connection, manifest: &Manifest) -> Result<BTreeSet<String>, DbError> {
    let mut stmt = conn.prepare_cached(dirty_sql(manifest))?;
    let names = stmt.query_map([], |r| r.get::<_, String>(0))?;
    Ok(names.collect::<Result<_, _>>()?)
}

/// Create the temp shadow tables and triggers on the writer connection. Must run after the
/// migrations, because the triggers name the migrated tables.
pub(crate) fn install(conn: &Connection) -> Result<(), DbError> {
    if disabled() {
        return Ok(());
    }
    let manifest = Manifest::get();
    let mut sql = String::from("PRAGMA recursive_triggers = ON;\n");
    for table in keyed_tables(manifest) {
        let t = &table.name;
        let cols: Vec<&str> = table.columns.iter().map(|(n, _)| n.as_str()).collect();
        let key: Vec<&str> = table.key.iter().map(String::as_str).collect();
        let all = ident_list(&cols);
        let old_values = cols
            .iter()
            .map(|c| format!("OLD.{c}"))
            .collect::<Vec<_>>()
            .join(", ");
        let new_key_match = key
            .iter()
            .map(|k| format!("{k} = NEW.{k}"))
            .collect::<Vec<_>>()
            .join(" AND ");
        let old_key_match = key
            .iter()
            .map(|k| format!("{k} = OLD.{k}"))
            .collect::<Vec<_>>()
            .join(" AND ");
        let new_key_values = key
            .iter()
            .map(|k| format!("NEW.{k}"))
            .collect::<Vec<_>>()
            .join(", ");
        let shadow = shadow(t);
        writeln!(
            sql,
            "DROP TABLE IF EXISTS temp.{shadow};
             CREATE TEMP TABLE {shadow} ({all}, ws_existed INTEGER NOT NULL, PRIMARY KEY ({keys}));",
            keys = ident_list(&key),
        )
        .unwrap();
        // Only the first touch of a key counts, so every snapshot is guarded by NOT EXISTS
        // rather than OR IGNORE: the conflict policy of the statement that fires a trigger
        // (an UPSERT, an INSERT OR REPLACE) overrides the one written inside it, and an
        // unguarded insert would abort the command.
        //
        // A key about to appear: snapshot the row if it is already there (a REPLACE or an
        // IGNORE hitting an existing row), else mark it as not having existed.
        let not_seen_new = format!("NOT EXISTS (SELECT 1 FROM {shadow} WHERE {new_key_match})");
        let not_seen_old = format!("NOT EXISTS (SELECT 1 FROM {shadow} WHERE {old_key_match})");
        let on_new_key = format!(
            "INSERT INTO {shadow} ({all}, ws_existed)
               SELECT {all}, 1 FROM main.{t} WHERE {new_key_match} AND {not_seen_new};
             INSERT INTO {shadow} ({keys}, ws_existed)
               SELECT {new_key_values}, 0
               WHERE NOT EXISTS (SELECT 1 FROM main.{t} WHERE {new_key_match}) AND {not_seen_new};",
            keys = ident_list(&key),
        );
        let on_old_row = format!(
            "INSERT INTO {shadow} ({all}, ws_existed) SELECT {old_values}, 1 WHERE {not_seen_old};"
        );
        writeln!(
            sql,
            "DROP TRIGGER IF EXISTS temp.ws_{t}_bi;
             CREATE TEMP TRIGGER ws_{t}_bi BEFORE INSERT ON main.{t} BEGIN {on_new_key} END;
             DROP TRIGGER IF EXISTS temp.ws_{t}_bu;
             CREATE TEMP TRIGGER ws_{t}_bu BEFORE UPDATE ON main.{t} BEGIN
               {on_old_row}
               {on_new_key}
             END;
             DROP TRIGGER IF EXISTS temp.ws_{t}_bd;
             CREATE TEMP TRIGGER ws_{t}_bd BEFORE DELETE ON main.{t} BEGIN
               {on_old_row}
             END;"
        )
        .unwrap();
        if !table.children.is_empty() {
            writeln!(
                sql,
                "DROP TABLE IF EXISTS temp.ws_pt_{t};
                 CREATE TEMP TABLE ws_pt_{t} (pk BLOB PRIMARY KEY);"
            )
            .unwrap();
        }
    }
    for child in folded_children(manifest) {
        let c = &child.name;
        let (parent, parent_col) = child.parent.as_ref().expect("a folded child has a parent");
        let cols: Vec<&str> = child.columns.iter().map(|(n, _)| n.as_str()).collect();
        let all = ident_list(&cols);
        writeln!(
            sql,
            "DROP TABLE IF EXISTS temp.ws_cb_{c};
             CREATE TEMP TABLE ws_cb_{c} ({all});
             DROP TABLE IF EXISTS temp.ws_cs_{c};
             CREATE TEMP TABLE ws_cs_{c} (parent BLOB PRIMARY KEY);"
        )
        .unwrap();
        // The first change to any child of a parent snapshots all that parent's children and
        // marks the parent touched. `{side}` is OLD or NEW.
        let touch = |side: &str| {
            format!(
                "INSERT INTO ws_cb_{c} ({all})
                   SELECT {all} FROM main.{c} WHERE {parent_col} = {side}.{parent_col}
                     AND NOT EXISTS (SELECT 1 FROM temp.ws_cs_{c} WHERE parent = {side}.{parent_col});
                 INSERT INTO ws_cs_{c} (parent)
                   SELECT {side}.{parent_col}
                   WHERE NOT EXISTS (SELECT 1 FROM temp.ws_cs_{c} WHERE parent = {side}.{parent_col});
                 INSERT INTO ws_pt_{parent} (pk)
                   SELECT {side}.{parent_col}
                   WHERE NOT EXISTS (SELECT 1 FROM temp.ws_pt_{parent} WHERE pk = {side}.{parent_col});"
            )
        };
        writeln!(
            sql,
            "DROP TRIGGER IF EXISTS temp.ws_{c}_bi;
             CREATE TEMP TRIGGER ws_{c}_bi BEFORE INSERT ON main.{c} BEGIN {new} END;
             DROP TRIGGER IF EXISTS temp.ws_{c}_bu;
             CREATE TEMP TRIGGER ws_{c}_bu BEFORE UPDATE ON main.{c} BEGIN {old} {new} END;
             DROP TRIGGER IF EXISTS temp.ws_{c}_bd;
             CREATE TEMP TRIGGER ws_{c}_bd BEFORE DELETE ON main.{c} BEGIN {old} END;",
            new = touch("NEW"),
            old = touch("OLD"),
        )
        .unwrap();
    }
    conn.execute_batch(&sql)?;
    Ok(())
}

/// An owned stored value.
#[derive(Debug, Clone, PartialEq)]
enum Owned {
    Null,
    Int(i64),
    Text(Vec<u8>),
    Blob(Vec<u8>),
}

impl Owned {
    fn from_ref(value: ValueRef<'_>) -> Result<Self, DbError> {
        Ok(match value {
            ValueRef::Null => Owned::Null,
            ValueRef::Integer(v) => Owned::Int(v),
            ValueRef::Text(b) => Owned::Text(b.to_vec()),
            ValueRef::Blob(b) => Owned::Blob(b.to_vec()),
            ValueRef::Real(_) => {
                return Err(DbError::InvalidCommand(
                    "a replicated table holds a REAL value".to_owned(),
                ))
            }
        })
    }

    fn as_value(&self) -> Value<'_> {
        match self {
            Owned::Null => Value::Null,
            Owned::Int(v) => Value::Int(*v),
            Owned::Text(b) => Value::Text(b),
            Owned::Blob(b) => Value::Blob(b),
        }
    }
}

/// A row: manifest column name to stored value.
type Row = BTreeMap<String, Owned>;

fn encode_error(error: sync_core::row_digest::RowDigestError) -> DbError {
    // The message names the rule broken and never a value.
    DbError::InvalidCommand(format!("a replicated row cannot be encoded: {error}"))
}

/// Read rows of `table` for the given SQL `where` clause and parameters.
fn read_rows(
    conn: &Connection,
    table: &str,
    spec: &TableSpec,
    where_sql: &str,
    params: &[&dyn rusqlite::ToSql],
) -> Result<Vec<Row>, DbError> {
    let names: Vec<&str> = spec.columns.iter().map(|(n, _)| n.as_str()).collect();
    let sql = format!("SELECT {} FROM {table} WHERE {where_sql}", names.join(", "));
    let mut stmt = conn.prepare_cached(&sql)?;
    let mut rows = stmt.query(params)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        let mut map = Row::new();
        for (i, name) in names.iter().enumerate() {
            map.insert((*name).to_owned(), Owned::from_ref(row.get_ref(i)?)?);
        }
        out.push(map);
    }
    Ok(out)
}

/// The columns a table (or its projection) contributes to a digest, with kinds.
fn digest_columns<'a>(
    spec: &'a TableSpec,
    projection: Option<&'a Projection>,
) -> Vec<&'a (String, Kind)> {
    match projection {
        Some(p) => spec
            .columns
            .iter()
            .filter(|(n, _)| p.columns.contains(n))
            .collect(),
        None => spec.columns.iter().collect(),
    }
}

fn to_columns<'a>(cols: &[&'a (String, Kind)], row: &'a Row) -> Vec<Column<'a>> {
    cols.iter()
        .map(|(name, kind)| Column {
            name,
            kind: *kind,
            value: row.get(name).unwrap_or(&Owned::Null).as_value(),
        })
        .collect()
}

/// The primary key bytes of a row (or of a projection's logical key).
fn key_bytes(
    spec: &TableSpec,
    projection: Option<&Projection>,
    row: &Row,
) -> Result<Vec<u8>, DbError> {
    if let Some(constant) = projection.and_then(|p| p.constant_key_int) {
        return primary_key(&[(Kind::Int, Value::Int(constant))]).map_err(encode_error);
    }
    let parts: Vec<(Kind, Value<'_>)> = spec
        .key
        .iter()
        .map(|k| {
            (
                spec.kind_of(k),
                row.get(k).unwrap_or(&Owned::Null).as_value(),
            )
        })
        .collect();
    primary_key(&parts).map_err(encode_error)
}

/// Whether a row of a projected table is replicated at all (settings keys are filtered).
fn replicated(spec: &TableSpec, projection: Option<&Projection>, row: &Row) -> bool {
    let Some(p) = projection else { return true };
    if p.key_prefixes.is_empty() {
        return true;
    }
    let Some(Owned::Text(key)) = spec.key.first().and_then(|k| row.get(k)) else {
        return false;
    };
    std::str::from_utf8(key).is_ok_and(|k| p.replicates_key(k))
}

/// The digest of a row's state, or of its absence.
fn state_digest(
    spec: &TableSpec,
    projection: Option<&Projection>,
    table_name: &str,
    row: Option<&Row>,
    children: Option<&[ChildTable<'_>]>,
    key: &[u8],
) -> Result<[u8; 32], DbError> {
    let Some(row) = row else {
        return absent_digest(table_name, key).map_err(encode_error);
    };
    let cols = digest_columns(spec, projection);
    let columns = to_columns(&cols, row);
    let digest = match children {
        Some(children) => parent_row_digest(table_name, &columns, children),
        None => row_digest(table_name, &columns),
    };
    digest.map_err(encode_error)
}

/// Build the child tables of one parent from owned child rows.
fn child_tables<'a>(
    children: &'a [(&'a TableSpec, Vec<Row>)],
    storage: &'a [Vec<Vec<&'a (String, Kind)>>],
) -> Vec<ChildTable<'a>> {
    children
        .iter()
        .zip(storage)
        .map(|((spec, rows), cols)| ChildTable {
            table: &spec.name,
            rows: rows
                .iter()
                .zip(cols)
                .map(|(row, c)| to_columns(c, row))
                .collect(),
        })
        .collect()
}

/// Read a parent's current children from the live child tables.
fn current_children<'a>(
    conn: &Connection,
    manifest: &'a Manifest,
    parent: &TableSpec,
    parent_key: &[u8],
) -> Result<Vec<(&'a TableSpec, Vec<Row>)>, DbError> {
    let mut out = Vec::new();
    for child in &parent.children {
        let spec = &manifest.tables[child];
        let (_, column) = spec.parent.as_ref().expect("folded child");
        let rows = read_rows(conn, child, spec, &format!("{column} = ?1"), &[&parent_key])?;
        out.push((spec, rows));
    }
    Ok(out)
}

/// The parent's id is the single uuid key `id`; a parent key is its 16 raw bytes.
fn parent_id(row_key: &[u8]) -> Option<Vec<u8>> {
    // primary_key for a uuid is the tag byte 3 then 16 bytes.
    (row_key.len() == 17 && row_key[0] == 3).then(|| row_key[1..].to_vec())
}

/// Compute the write set of the command that just ran in the current transaction, then clear
/// the capture tables.
pub(crate) fn finish(conn: &Connection) -> Result<WriteSet, DbError> {
    if disabled() {
        return Ok(WriteSet {
            hash: [0; 32],
            rows: Vec::new(),
        });
    }
    let manifest = Manifest::get();
    let mut entries: Vec<Entry> = Vec::new();
    let dirty = dirty_tables(conn, manifest)?;

    for table in keyed_tables(manifest) {
        // A table the command did not touch (no before image, no touched-parent mark) has no
        // entry; most tables of most commands are skipped here.
        let touched_directly = dirty.contains(&shadow(&table.name));
        let touched_via_children = dirty.contains(&format!("ws_pt_{}", table.name));
        if !touched_directly && !touched_via_children {
            continue;
        }
        let projection = manifest.projection_of(&table.name);
        let logical = projection.map_or(table.name.as_str(), |p| p.logical_table.as_str());
        let shadow_name = format!("temp.{}", shadow(&table.name));

        // Every key the command touched: the shadow's rows, plus (for a parent) the keys
        // touched only through their children.
        let mut touched: BTreeMap<Vec<u8>, (Row, Option<Row>)> = BTreeMap::new();
        let mut before_rows = {
            let names: Vec<&str> = table.columns.iter().map(|(n, _)| n.as_str()).collect();
            let sql = format!("SELECT {}, ws_existed FROM {shadow_name}", names.join(", "));
            let mut stmt = conn.prepare_cached(&sql)?;
            let mut rows = stmt.query([])?;
            let mut out: Vec<(Row, bool)> = Vec::new();
            while let Some(row) = rows.next()? {
                let mut map = Row::new();
                for (i, name) in names.iter().enumerate() {
                    map.insert((*name).to_owned(), Owned::from_ref(row.get_ref(i)?)?);
                }
                let existed: i64 = row.get(names.len())?;
                out.push((map, existed == 1));
            }
            out
        };
        // `touched` maps a key to (key-only row, before row if it existed).
        for (row, existed) in before_rows.drain(..) {
            let key_row: Row = table
                .key
                .iter()
                .map(|k| (k.clone(), row.get(k).cloned().unwrap_or(Owned::Null)))
                .collect();
            let kb = key_bytes(table, projection, &row)?;
            touched.insert(kb, (key_row, existed.then_some(row)));
        }
        let mut parent_only: BTreeSet<Vec<u8>> = BTreeSet::new();
        if !table.children.is_empty() {
            let mut stmt =
                conn.prepare_cached(&format!("SELECT pk FROM temp.ws_pt_{}", table.name))?;
            let ids: Vec<Vec<u8>> = stmt
                .query_map([], |r| r.get(0))?
                .collect::<Result<_, _>>()?;
            for id in ids {
                let key_row: Row = [(table.key[0].clone(), Owned::Blob(id))]
                    .into_iter()
                    .collect();
                let kb = key_bytes(table, projection, &key_row)?;
                if let std::collections::btree_map::Entry::Vacant(slot) = touched.entry(kb.clone())
                {
                    parent_only.insert(kb);
                    slot.insert((key_row, None));
                }
            }
        }

        for (kb, (key_row, before_row)) in touched {
            // The current (after) row, found by key.
            let where_sql = table
                .key
                .iter()
                .enumerate()
                .map(|(i, k)| format!("{k} = ?{}", i + 1))
                .collect::<Vec<_>>()
                .join(" AND ");
            // Bind the key exactly as it was captured: a text key goes in as its original bytes,
            // never re-decoded, so a key that is not valid UTF-8 still finds its row.
            let key_params: Vec<ToSqlOutput<'_>> = table
                .key
                .iter()
                .map(|k| match key_row.get(k).unwrap_or(&Owned::Null) {
                    Owned::Null => ToSqlOutput::Borrowed(ValueRef::Null),
                    Owned::Int(v) => ToSqlOutput::Borrowed(ValueRef::Integer(*v)),
                    Owned::Text(b) => ToSqlOutput::Borrowed(ValueRef::Text(b)),
                    Owned::Blob(b) => ToSqlOutput::Borrowed(ValueRef::Blob(b)),
                })
                .collect();
            let params: Vec<&dyn rusqlite::ToSql> = key_params
                .iter()
                .map(|v| v as &dyn rusqlite::ToSql)
                .collect();
            let after_row = read_rows(conn, &table.name, table, &where_sql, &params)?
                .into_iter()
                .next();

            // A row of a projected table that Sync does not replicate (a device setting) is
            // never an entry, before or after.
            let probe = after_row
                .as_ref()
                .or(before_row.as_ref())
                .unwrap_or(&key_row);
            if !replicated(table, projection, probe) {
                continue;
            }

            // A parent compares its row *and its children*; the before children are the
            // snapshot if any child changed, else the current children.
            let (before_digest, after_digest) = if table.children.is_empty() {
                (
                    state_digest(table, projection, logical, before_row.as_ref(), None, &kb)?,
                    state_digest(table, projection, logical, after_row.as_ref(), None, &kb)?,
                )
            } else {
                let id = parent_id(&kb)
                    .ok_or_else(|| DbError::InvalidCommand("a parent key is not a uuid".into()))?;
                let after_children = current_children(conn, manifest, table, &id)?;
                let mut before_children = Vec::new();
                for child in &table.children {
                    let spec = &manifest.tables[child];
                    let seen: i64 = conn.query_row(
                        &format!("SELECT COUNT(*) FROM temp.ws_cs_{child} WHERE parent = ?1"),
                        [&id],
                        |r| r.get(0),
                    )?;
                    let (_, column) = spec.parent.as_ref().expect("folded child");
                    let rows = if seen > 0 {
                        // The snapshot lives in the child's temp table with the same columns.
                        let names: Vec<&str> =
                            spec.columns.iter().map(|(n, _)| n.as_str()).collect();
                        let sql = format!(
                            "SELECT {} FROM temp.ws_cb_{child} WHERE {column} = ?1",
                            names.join(", ")
                        );
                        let mut stmt = conn.prepare_cached(&sql)?;
                        let mut rs = stmt.query([&id])?;
                        let mut out = Vec::new();
                        while let Some(r) = rs.next()? {
                            let mut map = Row::new();
                            for (i, n) in names.iter().enumerate() {
                                map.insert((*n).to_owned(), Owned::from_ref(r.get_ref(i)?)?);
                            }
                            out.push(map);
                        }
                        out
                    } else {
                        read_rows(conn, child, spec, &format!("{column} = ?1"), &[&id])?
                    };
                    before_children.push((spec, rows));
                }
                // A parent only touched through its children is unchanged itself.
                let before_row_effective = match (&before_row, parent_only.contains(&kb)) {
                    (_, true) => after_row.clone(),
                    (b, false) => b.clone(),
                };
                let storage_a: Vec<Vec<Vec<&(String, Kind)>>> = after_children
                    .iter()
                    .map(|(s, rows)| rows.iter().map(|_| s.columns.iter().collect()).collect())
                    .collect();
                let storage_b: Vec<Vec<Vec<&(String, Kind)>>> = before_children
                    .iter()
                    .map(|(s, rows)| rows.iter().map(|_| s.columns.iter().collect()).collect())
                    .collect();
                let after_ct = child_tables(&after_children, &storage_a);
                let before_ct = child_tables(&before_children, &storage_b);
                // An orphan: children left behind a parent that no longer exists.
                if after_row.is_none() && after_children.iter().any(|(_, rows)| !rows.is_empty()) {
                    return Err(DbError::InvalidCommand(
                        "a folded child row has no parent after the command".to_owned(),
                    ));
                }
                (
                    state_digest(
                        table,
                        projection,
                        logical,
                        before_row_effective.as_ref(),
                        Some(&before_ct),
                        &kb,
                    )?,
                    state_digest(
                        table,
                        projection,
                        logical,
                        after_row.as_ref(),
                        Some(&after_ct),
                        &kb,
                    )?,
                )
            };
            if before_digest != after_digest {
                entries.push(Entry {
                    table: logical.to_owned(),
                    primary_key: kb,
                    digest: after_digest,
                });
            }
        }
    }

    clear_tables(conn, &dirty)?;
    let hash = write_set_hash(&mut entries)
        .map_err(|e| DbError::InvalidCommand(format!("write set: {e}")))?;
    let rows = entries
        .iter()
        .map(|e| (e.table.clone(), e.primary_key.clone()))
        .collect();
    Ok(WriteSet { hash, rows })
}

/// Empty the capture tables that hold rows (inside the command's transaction).
pub(crate) fn clear(conn: &Connection) -> Result<(), DbError> {
    if disabled() {
        return Ok(());
    }
    let manifest = Manifest::get();
    let dirty = dirty_tables(conn, manifest)?;
    clear_tables(conn, &dirty)
}

fn clear_tables(conn: &Connection, dirty: &BTreeSet<String>) -> Result<(), DbError> {
    for table in dirty {
        conn.prepare_cached(&format!("DELETE FROM temp.{table}"))?
            .execute([])?;
    }
    Ok(())
}
