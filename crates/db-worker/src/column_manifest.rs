//! The column-kind manifest (`table_columns.toml`, vault-format.md C.1.5), parsed once at runtime.
//!
//! The file is generated from the migrated schema and checked in (see
//! `tests/column_kinds.rs`); the parser here reads exactly the shapes the generator writes and
//! panics on anything else, because a malformed embedded manifest is a build defect, not a
//! runtime condition.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use sync_core::row_digest::Kind;

const MANIFEST: &str = include_str!("../table_columns.toml");

/// What the Sync write set covers for a table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Coverage {
    Full,
    Projection,
    Folded,
    None,
}

/// One table of the manifest.
#[derive(Debug, Clone)]
pub(crate) struct TableSpec {
    pub name: String,
    pub coverage: Coverage,
    /// Primary-key column names in declared order (empty for a keyless table).
    pub key: Vec<String>,
    /// Every column in table order, with its declared kind.
    pub columns: Vec<(String, Kind)>,
    /// For a folded child: its parent table and the column naming the parent's `id`.
    pub parent: Option<(String, String)>,
    /// For a parent: its folded child tables.
    pub children: Vec<String>,
}

impl TableSpec {
    pub fn kind_of(&self, column: &str) -> Kind {
        self.columns
            .iter()
            .find(|(name, _)| name == column)
            .unwrap_or_else(|| panic!("{}: no column {column}", self.name))
            .1
    }
}

/// A projection of a field-split table onto its replicated fields.
#[derive(Debug, Clone)]
pub(crate) struct Projection {
    pub source: String,
    pub logical_table: String,
    pub columns: Vec<String>,
    #[allow(dead_code)] // parsed so the struct mirrors the manifest; the table's own key is used
    pub key: Vec<String>,
    /// A constant integer key for a singleton source row.
    pub constant_key_int: Option<i64>,
    pub key_prefixes: Vec<String>,
    pub legacy_key_names: Vec<String>,
}

impl Projection {
    /// Whether the source row with key text `key` is replicated (only meaningful for a
    /// projection with a key filter: the settings table).
    pub fn replicates_key(&self, key: &str) -> bool {
        if self.key_prefixes.is_empty() {
            return true;
        }
        self.key_prefixes
            .iter()
            .any(|p| key.starts_with(p.as_str()))
            || self.legacy_key_names.iter().any(|n| n == key)
    }
}

/// The parsed manifest.
#[derive(Debug)]
pub(crate) struct Manifest {
    pub tables: BTreeMap<String, TableSpec>,
    pub projections: Vec<Projection>,
}

impl Manifest {
    pub fn get() -> &'static Manifest {
        static PARSED: OnceLock<Manifest> = OnceLock::new();
        PARSED.get_or_init(|| parse(MANIFEST))
    }

    pub fn projection_of(&self, table: &str) -> Option<&Projection> {
        self.projections.iter().find(|p| p.source == table)
    }
}

fn quoted(line: &str) -> Vec<String> {
    line.split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect()
}

fn kind(text: &str) -> Kind {
    match text {
        "int" => Kind::Int,
        "text" => Kind::Text,
        "uuid" => Kind::Uuid,
        "bytes" => Kind::Bytes,
        other => panic!("table_columns.toml: unknown kind {other}"),
    }
}

fn parse(text: &str) -> Manifest {
    let mut tables = BTreeMap::new();
    let mut projections = Vec::new();
    for block in text.split("\n[[").skip(1) {
        let (header, body) = block.split_once("]]").expect("table header");
        let field = |key: &str| -> Option<&str> {
            body.lines()
                .find_map(|l| l.strip_prefix(&format!("{key} = ")))
        };
        match header {
            "table" => {
                let name = quoted(field("name").unwrap()).remove(0);
                let coverage = match quoted(field("write_set").unwrap()).remove(0).as_str() {
                    "full" => Coverage::Full,
                    "projection" => Coverage::Projection,
                    "folded" => Coverage::Folded,
                    "none" => Coverage::None,
                    other => panic!("table_columns.toml: unknown write_set {other}"),
                };
                let columns = quoted(field("columns").unwrap())
                    .into_iter()
                    .map(|c| {
                        let (n, k) = c.split_once(':').expect("name:kind");
                        (n.to_owned(), kind(k))
                    })
                    .collect();
                let parent = field("parent").map(|p| {
                    (
                        quoted(p).remove(0),
                        quoted(field("parent_column").expect("parent_column")).remove(0),
                    )
                });
                tables.insert(
                    name.clone(),
                    TableSpec {
                        name,
                        coverage,
                        key: quoted(field("primary_key").unwrap()),
                        columns,
                        parent,
                        children: field("children").map(quoted).unwrap_or_default(),
                    },
                );
            }
            "projection" => projections.push(Projection {
                source: quoted(field("source").unwrap()).remove(0),
                logical_table: quoted(field("logical_table").unwrap()).remove(0),
                columns: quoted(field("columns").unwrap()),
                key: quoted(field("key").unwrap()),
                constant_key_int: field("constant_key_int").map(|v| v.trim().parse().unwrap()),
                key_prefixes: field("key_prefixes").map(quoted).unwrap_or_default(),
                legacy_key_names: field("legacy_key_names").map(quoted).unwrap_or_default(),
            }),
            _ => {}
        }
    }
    Manifest {
        tables,
        projections,
    }
}
