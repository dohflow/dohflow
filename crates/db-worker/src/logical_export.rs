//! The logical class-1 export (`docs/architecture/vault-format.md` B.3, B.5 and C.8,
//! personal-cfo-1cltt.1): the ONE walker that turns the replicated rows of a vault into the
//! snapshot plaintext. The Sync genesis uses it here; the portable export (personal-cfo-vn91)
//! reuses it, so there is no second walker.
//!
//! **What it walks.** Every table the B.2 inventory gives a Sync-genesis disposition of `include`
//! or `fields`: the tables the write set covers (full, projection or folded), plus the import
//! provenance links (S4, whose write-set coverage is personal-cfo-0g528.2's). A projection
//! carries only its replicated fields (`attachments`, the `user.` keys of `settings`); a pseudo
//! table of the CLASS-0 manifest is not a table and gets no file. Tables go in ascending name
//! order, and every table gets a data file, empty ones included.
//!
//! **Order.** Rows are in ascending unsigned primary-key byte order (C.1.5). A folded child has no
//! key: its rows are ordered by their child-row bytes, the order its parent's digest uses.
//!
//! **Refusals.** A stored value whose storage class is not its declared kind, a primary key whose
//! encoding is over 256 bytes, a REAL, text that is not UTF-8, or a child with no parent stops
//! the export with a typed error naming the table and column. Nothing is coerced or truncated.
//!
//! **Consistency.** The walk runs inside one read transaction, so it sees one state of the vault.
//! No crypto, no file: the result is the plaintext that personal-cfo-1cltt.3 seals.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{Connection, Transaction, TransactionBehavior};
use sync_core::row_digest::{
    child_row_bytes, parent_row_digest, primary_key, row_digest, ChildTable, Column, Kind,
    RowDigestError, Value,
};
use sync_core::snapshot::{
    build, row_line, ColumnEntry, Header, RecordedVoid, SnapshotError, StateDigest, TableFile,
};

use crate::column_manifest::{Coverage, Manifest, Projection, TableSpec};
use crate::write_set::{digest_columns, replicated, to_columns, Owned, Row};

/// The version of the B.2 inventory the table set follows.
const INVENTORY_VERSION: &str = "1";

/// What a caller supplies beyond the vault's own rows.
#[derive(Debug, Clone, Copy, Default)]
pub struct ExportInput<'a> {
    /// The honored voids this replica holds, ascending and disjoint (C.10).
    pub voids: &'a [RecordedVoid],
    /// The closure manifest, ascending; empty until personal-cfo-1cltt.7.
    pub artifacts: &'a [[u8; 32]],
}

/// Why the export stopped. A name is a table or column name, never a value from a row.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ExportError {
    /// A stored value does not have its column's declared kind.
    #[error("table {table}, column {column}: the stored value does not match its declared kind")]
    KindMismatch {
        /// The table.
        table: String,
        /// The column.
        column: String,
    },
    /// A primary key's canonical encoding is over 256 bytes (never truncated).
    #[error("table {table}, column {column}: the primary key encoding exceeds 256 bytes")]
    KeyTooLong {
        /// The table.
        table: String,
        /// The key column that pushed it over.
        column: String,
    },
    /// Two rows have the same primary key, or a key column holds NULL.
    #[error("table {table}: a primary key is empty or repeated")]
    BadKey {
        /// The table.
        table: String,
    },
    /// A value cannot be written in the snapshot's canonical form.
    #[error("table {table}, column {column}: the value cannot be written in canonical form")]
    Unrepresentable {
        /// The table.
        table: String,
        /// The column.
        column: String,
    },
    /// A text or bytes value is longer than 2^24 bytes.
    #[error("table {table}, column {column}: the value exceeds the maximum length")]
    ValueTooLong {
        /// The table.
        table: String,
        /// The column.
        column: String,
    },
    /// A folded child row encodes to more than 65,536 bytes. The child table is named: the
    /// row's size is the sum of its columns, so no one column is at fault.
    #[error("table {table}: a folded child row exceeds the maximum length")]
    ChildRowTooLong {
        /// The folded child table.
        table: String,
    },
    /// The table's layout cannot be encoded at all (a name or column count out of bounds). That
    /// is a fault in the column manifest, not in a row, so no column is named.
    #[error("table {table}: the table layout cannot be encoded")]
    InvalidLayout {
        /// The table.
        table: String,
    },
    /// A folded child row names a parent that is not in the export.
    #[error("table {table}: a child row has no parent row")]
    OrphanChild {
        /// The child table.
        table: String,
    },
    /// The snapshot framing refused (including `TooLarge`, the typed `SnapshotTooLarge`).
    #[error(transparent)]
    Snapshot(#[from] SnapshotError),
    /// The database could not be read.
    #[error("the vault could not be read: {0}")]
    Db(#[from] rusqlite::Error),
}

/// The tables of the genesis, ascending by name.
pub(crate) fn genesis_tables(manifest: &Manifest) -> Vec<&TableSpec> {
    manifest
        .tables
        .values()
        .filter(|t| t.coverage != Coverage::None)
        .collect()
}

/// Build the snapshot plaintext of the vault `conn` reads (C.8).
///
/// # Errors
/// [`ExportError`] naming the table and column of the first value that cannot be exported.
pub fn export_genesis_plaintext(
    conn: &Connection,
    input: &ExportInput<'_>,
) -> Result<Vec<u8>, ExportError> {
    Ok(export_genesis_with_mark(conn, input)?.0)
}

/// [`export_genesis_plaintext`] and the `operation_log` high-water mark (the largest `op_seq`, `0`
/// for an empty log) of the **same** read transaction: the mark is the first statement of the
/// transaction, so it describes exactly the state the plaintext was built from. The enable sequence
/// persists it with the genesis (C.8) and compares the log against it once the genesis is published.
///
/// # Errors
/// [`ExportError`], as [`export_genesis_plaintext`].
pub fn export_genesis_with_mark(
    conn: &Connection,
    input: &ExportInput<'_>,
) -> Result<(Vec<u8>, u64), ExportError> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Deferred)?;
    let high_water: i64 = tx.query_row(
        "SELECT COALESCE(MAX(op_seq), 0) FROM operation_log",
        [],
        |r| r.get(0),
    )?;
    let high_water = u64::try_from(high_water).unwrap_or(0);
    let (created_at, schema_version): (String, i64) = tx.query_row(
        "SELECT created_at, schema_version FROM vault_metadata WHERE singleton = 1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let manifest = Manifest::get();
    let tables = genesis_tables(manifest);

    // Read every table once, so a parent can digest its children.
    let mut stored: BTreeMap<&str, Vec<Row>> = BTreeMap::new();
    for spec in &tables {
        let projection = manifest.projection_of(&spec.name);
        stored.insert(&spec.name, read_table(&tx, spec, projection)?);
    }

    let mut files = Vec::with_capacity(tables.len());
    for spec in &tables {
        files.push(table_file(manifest, spec, &stored)?);
    }
    let header = Header {
        schema_version: u64::try_from(schema_version).map_err(|_| {
            ExportError::Snapshot(SnapshotError::ManifestInvalid("a negative schema version"))
        })?,
        vault_created_at: &created_at,
        inventory_version: INVENTORY_VERSION,
        voids: input.voids,
        artifacts: input.artifacts,
    };
    Ok((build(&header, files)?, high_water))
}

/// Every exported row of a table, as stored, with the replicated-key filter applied.
fn read_table(
    conn: &Connection,
    spec: &TableSpec,
    projection: Option<&Projection>,
) -> Result<Vec<Row>, ExportError> {
    let columns = digest_columns(spec, projection);
    // A keyed table is read in full; the key columns are always part of `columns`.
    let names: Vec<&str> = columns.iter().map(|(n, _)| n.as_str()).collect();
    let sql = format!("SELECT {} FROM {}", names.join(", "), spec.name);
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        let mut map = Row::new();
        for (i, name) in names.iter().enumerate() {
            let value = row.get_ref(i)?;
            map.insert(
                (*name).to_owned(),
                Owned::from_ref(value).map_err(|_| ExportError::KindMismatch {
                    table: spec.name.clone(),
                    column: (*name).to_owned(),
                })?,
            );
        }
        if replicated(spec, projection, &map) {
            out.push(map);
        }
    }
    Ok(out)
}

/// Name what broke a row digest. A failure caused by one value names its table and column (found
/// by digesting each column alone), one caused by a folded child row names the child table, and a
/// failure no row can cause is the table's layout. No error carries an empty column.
fn digest_failure(
    table: &str,
    columns: &[Column<'_>],
    children: &[ChildTable<'_>],
    error: &RowDigestError,
) -> ExportError {
    let layout = || ExportError::InvalidLayout {
        table: table.to_owned(),
    };
    let culprit = |owner: &str, columns: &[Column<'_>]| -> Option<String> {
        columns
            .iter()
            .find(|column| row_digest(owner, &[**column]).as_ref().err() == Some(error))
            .map(|column| column.name.to_owned())
    };
    let named = |owner: &str, column: String| match error {
        RowDigestError::ValueTooLong => ExportError::ValueTooLong {
            table: owner.to_owned(),
            column,
        },
        _ => ExportError::KindMismatch {
            table: owner.to_owned(),
            column,
        },
    };
    match error {
        RowDigestError::KindMismatch
        | RowDigestError::BadUuidLength
        | RowDigestError::InvalidUtf8
        | RowDigestError::ValueTooLong => culprit(table, columns)
            .map(|column| named(table, column))
            .or_else(|| {
                children.iter().find_map(|child| {
                    child.rows.iter().find_map(|row| {
                        culprit(child.table, row).map(|column| named(child.table, column))
                    })
                })
            })
            .unwrap_or_else(layout),
        RowDigestError::ChildRowTooLong => children
            .iter()
            .find(|child| {
                child
                    .rows
                    .iter()
                    .any(|row| child_row_bytes(row) == Err(RowDigestError::ChildRowTooLong))
            })
            .map_or_else(layout, |child| ExportError::ChildRowTooLong {
                table: child.table.to_owned(),
            }),
        _ => layout(),
    }
}

/// The primary-key bytes of `row`, naming the key column that breaks a rule.
fn key_of(
    spec: &TableSpec,
    projection: Option<&Projection>,
    row: &Row,
) -> Result<Vec<u8>, ExportError> {
    if let Some(constant) = projection.and_then(|p| p.constant_key_int) {
        return primary_key(&[(Kind::Int, Value::Int(constant))])
            .map_err(|e| digest_failure(&spec.name, &[], &[], &e));
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
    for (i, column) in spec.key.iter().enumerate() {
        match primary_key(&parts[..=i]) {
            Ok(_) => {}
            Err(RowDigestError::KeyTooLong) => {
                return Err(ExportError::KeyTooLong {
                    table: spec.name.clone(),
                    column: column.clone(),
                })
            }
            Err(
                RowDigestError::KindMismatch
                | RowDigestError::BadUuidLength
                | RowDigestError::InvalidUtf8,
            ) => {
                return Err(ExportError::KindMismatch {
                    table: spec.name.clone(),
                    column: column.clone(),
                })
            }
            Err(_) => {
                return Err(ExportError::BadKey {
                    table: spec.name.clone(),
                })
            }
        }
    }
    if parts.iter().any(|(_, v)| matches!(v, Value::Null)) {
        return Err(ExportError::BadKey {
            table: spec.name.clone(),
        });
    }
    primary_key(&parts).map_err(|e| digest_failure(&spec.name, &[], &[], &e))
}

fn line_of(spec: &TableSpec, columns: &[Column<'_>]) -> Result<Vec<u8>, ExportError> {
    row_line(columns).map_err(|e| match e {
        SnapshotError::KindMismatch { column } => ExportError::KindMismatch {
            table: spec.name.clone(),
            column,
        },
        SnapshotError::Unrepresentable { column } => ExportError::Unrepresentable {
            table: spec.name.clone(),
            column,
        },
        other => ExportError::Snapshot(other),
    })
}

/// One table's file, its row count and, for a keyed table, its state digest.
fn table_file(
    manifest: &Manifest,
    spec: &TableSpec,
    stored: &BTreeMap<&str, Vec<Row>>,
) -> Result<TableFile, ExportError> {
    let projection = manifest.projection_of(&spec.name);
    let columns = digest_columns(spec, projection);
    let entries: Vec<ColumnEntry> = columns
        .iter()
        .map(|(name, kind)| ColumnEntry {
            name: name.clone(),
            kind: kind_name(*kind).to_owned(),
        })
        .collect();
    let rows = &stored[spec.name.as_str()];

    if spec.coverage == Coverage::Folded {
        return folded_child_file(manifest, spec, rows, &columns, entries, stored);
    }

    // A keyed table: ascending key order, one state digest over the row digests.
    let mut keyed: Vec<(Vec<u8>, &Row)> = Vec::with_capacity(rows.len());
    for row in rows {
        keyed.push((key_of(spec, projection, row)?, row));
    }
    keyed.sort_by(|a, b| a.0.cmp(&b.0));

    let children = children_of(manifest, spec, stored)?;
    let mut digest = StateDigest::new(&spec.name)?;
    let mut content = Vec::new();
    for (key, row) in &keyed {
        let cols = to_columns(&columns, row);
        content.extend_from_slice(&line_of(spec, &cols)?);
        let child_tables = if spec.children.is_empty() {
            Vec::new()
        } else {
            child_tables_of(manifest, spec, children.get(&parent_id_of(row)))
        };
        let row_digest = if spec.children.is_empty() {
            row_digest(&spec.name, &cols)
        } else {
            parent_row_digest(&spec.name, &cols, &child_tables)
        }
        .map_err(|e| digest_failure(&spec.name, &cols, &child_tables, &e))?;
        digest
            .push(key, row_digest)
            .map_err(|_| ExportError::BadKey {
                table: spec.name.clone(),
            })?;
    }
    Ok(TableFile {
        name: spec.name.clone(),
        columns: entries,
        row_count: keyed.len() as u64,
        state_digest: Some(digest.finish()?),
        content,
    })
}

/// The folded children of every row of a parent, by the parent's id: child table, then rows.
type ChildRows<'a> = BTreeMap<Vec<u8>, Vec<(&'a TableSpec, Vec<&'a Row>)>>;

fn parent_id_of(row: &Row) -> Vec<u8> {
    match row.get("id") {
        Some(Owned::Blob(bytes)) => bytes.clone(),
        _ => Vec::new(),
    }
}

fn children_of<'a>(
    manifest: &'a Manifest,
    parent: &TableSpec,
    stored: &'a BTreeMap<&str, Vec<Row>>,
) -> Result<ChildRows<'a>, ExportError> {
    let mut out: ChildRows<'a> = BTreeMap::new();
    for child_name in &parent.children {
        let child = &manifest.tables[child_name];
        let (_, column) = child
            .parent
            .as_ref()
            .expect("a folded child names its parent");
        let known: BTreeSet<Vec<u8>> = stored[parent.name.as_str()]
            .iter()
            .map(parent_id_of)
            .collect();
        let mut grouped: BTreeMap<Vec<u8>, Vec<&Row>> = BTreeMap::new();
        for row in &stored[child_name.as_str()] {
            let id = match row.get(column) {
                Some(Owned::Blob(bytes)) => bytes.clone(),
                _ => Vec::new(),
            };
            // A child with no parent is refused when the child's own file is built.
            grouped.entry(id).or_default().push(row);
        }
        // Every parent lists every declared child table, with no rows when it has none.
        for id in &known {
            out.entry(id.clone())
                .or_default()
                .push((child, grouped.remove(id).unwrap_or_default()));
        }
    }
    Ok(out)
}

fn child_tables_of<'a>(
    _manifest: &'a Manifest,
    parent: &'a TableSpec,
    owned: Option<&'a Vec<(&'a TableSpec, Vec<&'a Row>)>>,
) -> Vec<ChildTable<'a>> {
    let owned = owned.map_or(&[][..], Vec::as_slice);
    parent
        .children
        .iter()
        .map(|name| {
            let rows = owned
                .iter()
                .find(|(spec, _)| spec.name == *name)
                .map(|(spec, rows)| {
                    let columns: Vec<&(String, Kind)> = spec.columns.iter().collect();
                    rows.iter().map(|r| to_columns(&columns, r)).collect()
                })
                .unwrap_or_default();
            ChildTable { table: name, rows }
        })
        .collect()
}

fn folded_child_file(
    manifest: &Manifest,
    spec: &TableSpec,
    rows: &[Row],
    columns: &[&(String, Kind)],
    entries: Vec<ColumnEntry>,
    stored: &BTreeMap<&str, Vec<Row>>,
) -> Result<TableFile, ExportError> {
    let (parent_name, parent_column) = spec
        .parent
        .as_ref()
        .expect("a folded child names its parent");
    let parent = &manifest.tables[parent_name];
    let known: BTreeSet<Vec<u8>> = stored[parent.name.as_str()]
        .iter()
        .map(parent_id_of)
        .collect();
    let mut encoded: Vec<(Vec<u8>, Vec<u8>)> = Vec::with_capacity(rows.len());
    for row in rows {
        let id = match row.get(parent_column) {
            Some(Owned::Blob(bytes)) => bytes.clone(),
            _ => Vec::new(),
        };
        if !known.contains(&id) {
            return Err(ExportError::OrphanChild {
                table: spec.name.clone(),
            });
        }
        let cols = to_columns(columns, row);
        // The line first: it checks every column by its declared kind and names the one that
        // breaks a rule, which the child-row bytes cannot.
        let line = line_of(spec, &cols)?;
        let order = child_row_bytes(&cols).map_err(|e| match e {
            RowDigestError::ChildRowTooLong => ExportError::ChildRowTooLong {
                table: spec.name.clone(),
            },
            other => digest_failure(&spec.name, &cols, &[], &other),
        })?;
        encoded.push((order, line));
    }
    // The order of a parent's digest: bytewise by the child-row bytes, duplicates kept.
    encoded.sort_by(|a, b| a.0.cmp(&b.0));
    let content: Vec<u8> = encoded.iter().flat_map(|(_, line)| line.clone()).collect();
    Ok(TableFile {
        name: spec.name.clone(),
        columns: entries,
        row_count: encoded.len() as u64,
        state_digest: None,
        content,
    })
}

const fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Int => "int",
        Kind::Text => "text",
        Kind::Uuid => "uuid",
        Kind::Bytes => "bytes",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn child_with(value: Value<'static>) -> ChildTable<'static> {
        ChildTable {
            table: "ledger_postings",
            rows: vec![vec![Column {
                name: "currency",
                kind: Kind::Text,
                value,
            }]],
        }
    }

    #[test]
    fn a_failure_in_a_parents_digest_names_the_child_table_that_caused_it() {
        let leaked: &'static [u8] = Box::leak(vec![b'x'; 70_000].into_boxed_slice());
        let children = [child_with(Value::Text(leaked))];
        let error = digest_failure(
            "ledger_transactions",
            &[],
            &children,
            &RowDigestError::ChildRowTooLong,
        );
        assert!(
            matches!(&error, ExportError::ChildRowTooLong { table } if table == "ledger_postings"),
            "{error:?}"
        );
    }

    #[test]
    fn a_value_failure_found_only_in_a_child_row_names_the_child_table_and_column() {
        let bad = child_with(Value::Int(1)); // a text column holding an integer
        let error = digest_failure(
            "ledger_transactions",
            &[],
            &[bad],
            &RowDigestError::KindMismatch,
        );
        assert!(
            matches!(&error, ExportError::KindMismatch { table, column }
                if table == "ledger_postings" && column == "currency"),
            "{error:?}"
        );
    }

    #[test]
    fn a_failure_no_row_can_cause_is_the_layout_and_names_no_column() {
        for error in [
            RowDigestError::BadName,
            RowDigestError::DuplicateColumn,
            RowDigestError::TooManyColumns,
            RowDigestError::BadChildTables,
            RowDigestError::EmptyKey,
            RowDigestError::KeyTooLong,
        ] {
            let shown = digest_failure("accounts", &[], &[], &error);
            assert!(
                matches!(&shown, ExportError::InvalidLayout { table } if table == "accounts"),
                "{error:?} -> {shown:?}"
            );
        }
        // A value failure that no column reproduces is not blamed on an empty column either.
        let shown = digest_failure("accounts", &[], &[], &RowDigestError::ValueTooLong);
        assert!(matches!(shown, ExportError::InvalidLayout { .. }));
    }
}
