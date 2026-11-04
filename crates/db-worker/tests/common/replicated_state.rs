//! The independent full-scan view of the replicated state (personal-cfo-vpza7, shared by
//! personal-cfo-32fmp.4): every write-set row digested from a plain scan of the live tables, written
//! independently of `write_set.rs` (no triggers, no shadow tables), plus the class-1b artifact tables
//! the write set does not cover. One implementation serves the write-set oracle, the two-vault replay
//! property and the engine pin.

use std::collections::BTreeMap;

use rusqlite::types::ValueRef;
use sync_core::row_digest::{
    absent_digest, parent_row_digest, primary_key, row_digest, ChildTable, Column, Kind, Value,
};
use sync_core::write_set::{write_set_hash, Entry};

const MANIFEST: &str = include_str!("../../table_columns.toml");

pub struct TableSpec {
    pub key: Vec<String>,
    pub columns: Vec<(String, Kind)>,
    pub class: String,
    pub write_set: String,
    /// Folded child tables of this parent, if any.
    pub children: Vec<String>,
    /// For a folded child: its parent table and the column naming the parent's `id`.
    pub parent: Option<(String, String)>,
}

pub fn parse_kind(kind: &str) -> Kind {
    match kind {
        "int" => Kind::Int,
        "text" => Kind::Text,
        "uuid" => Kind::Uuid,
        "bytes" => Kind::Bytes,
        other => panic!("unknown kind {other}"),
    }
}

pub fn quoted_list(line: &str) -> Vec<String> {
    line.split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect()
}

/// The manifest's tables, parsed from the checked-in file.
pub fn manifest_tables() -> BTreeMap<String, TableSpec> {
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
                class: quoted_list(&line("class")).remove(0),
                write_set: quoted_list(&line("write_set")).remove(0),
                children: optional("children").unwrap_or_default(),
                parent,
            },
        );
    }
    out
}

pub fn value_of(value: ValueRef<'_>) -> Value<'_> {
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(v) => Value::Int(v),
        ValueRef::Text(bytes) => Value::Text(bytes),
        ValueRef::Blob(bytes) => Value::Blob(bytes),
        ValueRef::Real(_) => panic!("a REAL value in a covered table"),
    }
}

pub fn as_value(value: &rusqlite::types::Value) -> Value<'_> {
    match value {
        rusqlite::types::Value::Null => Value::Null,
        rusqlite::types::Value::Integer(v) => Value::Int(*v),
        rusqlite::types::Value::Text(t) => Value::Text(t.as_bytes()),
        rusqlite::types::Value::Blob(b) => Value::Blob(b),
        rusqlite::types::Value::Real(_) => panic!("a REAL value in a covered table"),
    }
}

pub type Row = BTreeMap<String, rusqlite::types::Value>;

pub fn all_rows(conn: &rusqlite::Connection, table: &str, spec: &TableSpec) -> Vec<Row> {
    let names: Vec<&str> = spec.columns.iter().map(|(n, _)| n.as_str()).collect();
    let mut stmt = conn
        .prepare(&format!("SELECT {} FROM {table}", names.join(", ")))
        .unwrap();
    stmt.query_map([], |row| {
        names
            .iter()
            .enumerate()
            .map(|(i, n)| Ok(((*n).to_owned(), row.get::<_, rusqlite::types::Value>(i)?)))
            .collect()
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

pub fn cols_of<'a>(spec: &'a TableSpec, only: Option<&[String]>, row: &'a Row) -> Vec<Column<'a>> {
    spec.columns
        .iter()
        .filter(|(n, _)| only.is_none_or(|o| o.contains(n)))
        .map(|(n, k)| Column {
            name: n,
            kind: *k,
            value: as_value(&row[n]),
        })
        .collect()
}

pub fn key_of(spec: &TableSpec, row: &Row) -> Vec<u8> {
    let key: Vec<(Kind, Value<'_>)> = spec
        .key
        .iter()
        .map(|k| {
            (
                spec.columns.iter().find(|(n, _)| n == k).unwrap().1,
                as_value(&row[k]),
            )
        })
        .collect();
    primary_key(&key).unwrap()
}

/// Every replicated row's digest, keyed by (logical table, primary key), from a full scan.
/// Written independently of `write_set.rs`: no triggers, no shadow tables.
pub fn replicated_state(conn: &rusqlite::Connection) -> BTreeMap<(String, Vec<u8>), [u8; 32]> {
    let specs = manifest_tables();
    let mut out = BTreeMap::new();
    for (table, spec) in &specs {
        match spec.write_set.as_str() {
            "full" => {
                let children: Vec<(&String, &TableSpec)> =
                    spec.children.iter().map(|c| (c, &specs[c])).collect();
                for row in all_rows(conn, table, spec) {
                    let key = key_of(spec, &row);
                    let digest = if children.is_empty() {
                        row_digest(table, &cols_of(spec, None, &row)).unwrap()
                    } else {
                        let id = match &row["id"] {
                            rusqlite::types::Value::Blob(b) => b.clone(),
                            other => panic!("{other:?}"),
                        };
                        let child_rows: Vec<(String, Vec<Row>)> = children
                            .iter()
                            .map(|(name, cspec)| {
                                let column = &cspec.parent.as_ref().unwrap().1;
                                let rows = all_rows(conn, name, cspec)
                                    .into_iter()
                                    .filter(|r| {
                                        r[column] == rusqlite::types::Value::Blob(id.clone())
                                    })
                                    .collect();
                                ((*name).clone(), rows)
                            })
                            .collect();
                        let tables: Vec<ChildTable<'_>> = child_rows
                            .iter()
                            .map(|(name, rows)| ChildTable {
                                table: name,
                                rows: rows
                                    .iter()
                                    .map(|r| cols_of(&specs[name], None, r))
                                    .collect(),
                            })
                            .collect();
                        parent_row_digest(table, &cols_of(spec, None, &row), &tables).unwrap()
                    };
                    out.insert((table.clone(), key), digest);
                }
            }
            "projection" => {
                let p = manifest_projection(table);
                for row in all_rows(conn, table, spec) {
                    if let Some(prefixes) = &p.key_prefixes {
                        let rusqlite::types::Value::Text(k) = &row[&spec.key[0]] else {
                            continue;
                        };
                        if !prefixes.iter().any(|x| k.starts_with(x)) {
                            continue;
                        }
                    }
                    let key = match p.constant_key {
                        Some(c) => primary_key(&[(Kind::Int, Value::Int(c))]).unwrap(),
                        None => key_of(spec, &row),
                    };
                    let digest = if spec.children.is_empty() {
                        row_digest(&p.logical, &cols_of(spec, Some(&p.columns), &row)).unwrap()
                    } else {
                        let id = row["id"].clone();
                        let kids: Vec<Row> =
                            all_rows(conn, &spec.children[0], &specs[&spec.children[0]])
                                .into_iter()
                                .filter(|r| {
                                    r[&specs[&spec.children[0]].parent.as_ref().unwrap().1] == id
                                })
                                .collect();
                        let cspec = &specs[&spec.children[0]];
                        let tables = vec![ChildTable {
                            table: &spec.children[0],
                            rows: kids.iter().map(|r| cols_of(cspec, None, r)).collect(),
                        }];
                        parent_row_digest(
                            &p.logical,
                            &cols_of(spec, Some(&p.columns), &row),
                            &tables,
                        )
                        .unwrap()
                    };
                    out.insert((p.logical.clone(), key), digest);
                }
            }
            _ => {}
        }
    }
    out
}

pub struct ProjectionInfo {
    pub logical: String,
    pub columns: Vec<String>,
    pub constant_key: Option<i64>,
    pub key_prefixes: Option<Vec<String>>,
}

pub fn manifest_projection(source: &str) -> ProjectionInfo {
    let block = MANIFEST
        .split("[[projection]]")
        .skip(1)
        .find(|b| b.contains(&format!("source = \"{source}\"")))
        .unwrap();
    let field = |key: &str| {
        block
            .lines()
            .find(|l| l.starts_with(&format!("{key} = ")))
            .map(str::to_owned)
    };
    ProjectionInfo {
        logical: quoted_list(&field("logical_table").unwrap()).remove(0),
        columns: quoted_list(&field("columns").unwrap()),
        constant_key: field("constant_key_int")
            .map(|l| l.split_once('=').unwrap().1.trim().parse().unwrap()),
        key_prefixes: field("key_prefixes").map(|l| quoted_list(&l)),
    }
}

/// The state-difference write set computed from two full dumps.
pub fn oracle(
    before: &BTreeMap<(String, Vec<u8>), [u8; 32]>,
    after: &BTreeMap<(String, Vec<u8>), [u8; 32]>,
) -> [u8; 32] {
    let mut entries = Vec::new();
    for (key, digest) in after {
        if before.get(key) != Some(digest) {
            entries.push(Entry {
                table: key.0.clone(),
                primary_key: key.1.clone(),
                digest: *digest,
            });
        }
    }
    for key in before.keys().filter(|k| !after.contains_key(*k)) {
        entries.push(Entry {
            table: key.0.clone(),
            primary_key: key.1.clone(),
            digest: absent_digest(&key.0, &key.1).unwrap(),
        });
    }
    write_set_hash(&mut entries).unwrap()
}

/// The class-1b artifact tables the write set does not cover (`write_set = none`: the source
/// batches, records, parser runs and provenance links), digested like any other row so a replica's
/// copy can be compared. The write-set oracle must NOT use this (the capture does not include them).
pub fn artifact_state(conn: &rusqlite::Connection) -> BTreeMap<(String, Vec<u8>), [u8; 32]> {
    let mut out = BTreeMap::new();
    for (table, spec) in &manifest_tables() {
        if spec.class != "1b" || spec.write_set != "none" || spec.key.is_empty() {
            continue;
        }
        for row in all_rows(conn, table, spec) {
            out.insert(
                (table.clone(), key_of(spec, &row)),
                row_digest(table, &cols_of(spec, None, &row)).unwrap(),
            );
        }
    }
    out
}

/// Everything two replicas must agree on: the write-set rows and the class-1b artifact rows. The
/// operation log is deliberately absent: `operation_log.created_at`, the HLC and the node id are
/// the recording device's own (personal-cfo-32fmp.4 AC2).
pub fn comparable_state(conn: &rusqlite::Connection) -> BTreeMap<(String, Vec<u8>), [u8; 32]> {
    let mut state = replicated_state(conn);
    state.extend(artifact_state(conn));
    state
}
