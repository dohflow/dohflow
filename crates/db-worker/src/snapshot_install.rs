//! Installing a snapshot into a new vault (`docs/architecture/vault-format.md` C.8;
//! `personal-cfo-1cltt.5`, slice A): the exact inverse of [`crate::logical_export`].
//!
//! A replica is built from a snapshot, never from the seed a new vault gets (`seed_new_vault`): the
//! origin's `vault_id` and creation time are written first, then every row of every table the
//! snapshot lists, so a legacy origin's ids are kept and nothing is derived twice.
//!
//! **What is checked, in order, before anything is written:** the vault is at the snapshot's schema
//! version and holds no identity and no replicated row; the snapshot names exactly this build's
//! tables with exactly its columns; and it needs no artifacts (a class-1b closure, or an attachment
//! row, is `personal-cfo-1cltt.7`'s). **What is checked before it commits:** every data-file line is
//! the canonical encoding of its row ([`sync_core::snapshot::parse_row_line`]), and the installed
//! state, exported again inside the same transaction, equals the snapshot table by table: the same
//! file hash, row count and state digest. A snapshot that fails any of these installs nothing.
//!
//! Foreign keys are deferred to the commit, and tables go in the snapshot's ascending order, which
//! puts a ledger transaction's postings in before its header, as the balance trigger requires.

use rusqlite::types::Value as SqlValue;
use rusqlite::Connection;
use sync_core::snapshot::{
    parse_row_line, verify, ColumnEntry, OwnedValue, SnapshotError, TableEntry, Verified,
};
use thiserror::Error;
use uuid::Uuid;

use crate::column_manifest::{Coverage, Manifest};
use crate::logical_export::{export_within, genesis_tables, kind_name, ExportError, ExportInput};
use crate::write_set::digest_columns;
use crate::DbError;

/// What an install wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstalledSnapshot {
    /// The tables installed.
    pub tables: usize,
    /// The rows installed, over all tables.
    pub rows: u64,
}

/// Why a snapshot was not installed. Nothing was written in any case, and no variant carries a
/// value from a row (only table and column names).
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum InstallError {
    /// The snapshot, or one of its rows, is not valid.
    #[error("the snapshot is not valid: {0}")]
    Snapshot(#[from] SnapshotError),
    /// The vault is not at the snapshot's schema version. The creation path migrates a new vault to
    /// the snapshot's version before it installs, then to this build's (the caller's job); a
    /// snapshot of a **newer** schema is update-required and is never installed.
    #[error("the snapshot is of schema version {snapshot} and the vault is at {vault}")]
    SchemaMismatch {
        /// The snapshot's schema version.
        snapshot: u64,
        /// The vault's.
        vault: i64,
    },
    /// The vault already has an identity (`vault_metadata`): it is not new.
    #[error("the vault already has an identity")]
    HasIdentity,
    /// A table the snapshot installs already holds rows.
    #[error("table {table} is not empty")]
    NotEmpty {
        /// The table.
        table: String,
    },
    /// The snapshot names a table this build does not replicate.
    #[error("the snapshot names a table this build does not know: {table}")]
    UnknownTable {
        /// The table.
        table: String,
    },
    /// A table this build replicates is not in the snapshot.
    #[error("the snapshot has no file for table {table}")]
    MissingTable {
        /// The table.
        table: String,
    },
    /// A table's columns are not this build's, in names, kinds or order.
    #[error("table {table}: the snapshot's columns are not this build's")]
    ColumnsMismatch {
        /// The table.
        table: String,
    },
    /// The snapshot needs artifacts this slice does not install: a class-1b closure, or an
    /// attachment row (its bytes are an artifact). `personal-cfo-1cltt.7`.
    #[error("the snapshot needs artifacts, which are not installed here")]
    NeedsArtifacts,
    /// The installed rows do not re-export to the snapshot's table: its file, count or digest.
    #[error("table {table}: the installed rows do not re-export to the snapshot's")]
    DigestMismatch {
        /// The table.
        table: String,
    },
    /// The vault could not be read or written.
    #[error("the vault could not be written: {0}")]
    Db(#[from] DbError),
    /// The installed vault's identity header (its creation time or schema version) is not the
    /// snapshot's.
    #[error("the installed vault's header is not the snapshot's")]
    HeaderMismatch,
    /// The re-export that proves the install failed.
    #[error("the installed state could not be exported again: {0}")]
    Export(#[from] ExportError),
}

impl From<rusqlite::Error> for InstallError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Db(DbError::from(error))
    }
}

fn sql_value(value: OwnedValue) -> SqlValue {
    match value {
        OwnedValue::Null => SqlValue::Null,
        OwnedValue::Int(n) => SqlValue::Integer(n),
        OwnedValue::Text(t) => SqlValue::Text(t),
        OwnedValue::Blob(b) => SqlValue::Blob(b),
    }
}

/// This build's columns of `table`, as the snapshot's manifest spells them.
fn expected_columns(table: &str) -> Vec<ColumnEntry> {
    let manifest = Manifest::get();
    let spec = &manifest.tables[table];
    digest_columns(spec, manifest.projection_of(table))
        .into_iter()
        .map(|(name, kind)| ColumnEntry {
            name: name.clone(),
            kind: kind_name(*kind).to_owned(),
        })
        .collect()
}

/// Install `verified` into the new, migrated vault `conn`, as the vault identified by `vault_id`.
///
/// `conn` must be at the snapshot's schema version and hold no identity and no row of any table the
/// snapshot installs. Everything happens in one transaction: it either commits the whole snapshot
/// or leaves the vault as it was.
///
/// # Errors
/// An [`InstallError`]; the vault is unchanged.
pub fn install_snapshot(
    conn: &mut Connection,
    vault_id: [u8; 16],
    verified: &Verified,
) -> Result<InstalledSnapshot, InstallError> {
    install_inner(conn, vault_id, verified, false).map(|(installed, _)| installed)
}

/// One attachment row of a snapshot: the columns that travel. The local crypto fields (`storage_id`,
/// the wrapped content key and the nonces) never travel; a replica makes them when it materializes
/// the attachment's artifact, and creates the row from this and those.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotAttachment {
    /// The attachment's id.
    pub id: [u8; 16],
    /// The plaintext size the origin recorded.
    pub plaintext_size: i64,
    /// The media type of the **row** (a later class-1 edit lives only there).
    pub mime_type: Option<String>,
    /// The original filename.
    pub original_filename: Option<String>,
    /// When the row was created.
    pub created_at: String,
}

/// [`install_snapshot`] for a snapshot that carries a class-1b closure (`personal-cfo-1cltt.7`,
/// slice D2). It differs in three ways, each because the closure's artifacts are not here yet:
///
/// * a snapshot with artifacts and attachment rows is accepted, not refused;
/// * the `attachments` rows are **not inserted**: they are parsed and returned, because their
///   local crypto fields come from materializing the artifacts, and the caller creates the rows
///   then;
/// * the re-export proof is **not run here**: attachment links hang under attachment rows (they are
///   folded children of their parent in the export), so the state cannot be exported until the rows
///   exist. The caller runs [`prove_installed`] once they do. (A snapshot with a closure is of the
///   schema that introduced closures or later, so the build carries it forward before the proof.)
///
/// The caller sets `PRAGMA foreign_keys = OFF` on `conn` before calling (it cannot change inside the
/// transaction) and checks the whole database with `foreign_key_check` once the artifacts are
/// materialized: a provenance link names a source record that only a batch artifact makes.
///
/// # Errors
/// An [`InstallError`]; the vault is unchanged.
pub fn install_snapshot_with_closure(
    conn: &mut Connection,
    vault_id: [u8; 16],
    verified: &Verified,
) -> Result<(InstalledSnapshot, Vec<SnapshotAttachment>), InstallError> {
    install_inner(conn, vault_id, verified, true)
}

fn install_inner(
    conn: &mut Connection,
    vault_id: [u8; 16],
    verified: &Verified,
    with_closure: bool,
) -> Result<(InstalledSnapshot, Vec<SnapshotAttachment>), InstallError> {
    let manifest = &verified.manifest;
    let snapshot_schema: u64 = manifest.schema_version.parse().map_err(|_| {
        InstallError::Snapshot(SnapshotError::ManifestInvalid("the schema version"))
    })?;
    let vault_schema: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if u64::try_from(vault_schema).ok() != Some(snapshot_schema) {
        return Err(InstallError::SchemaMismatch {
            snapshot: snapshot_schema,
            vault: vault_schema,
        });
    }
    if !with_closure && !manifest.artifacts.is_empty() {
        return Err(InstallError::NeedsArtifacts);
    }

    let own = Manifest::get();
    let ours: Vec<&str> = genesis_tables(own)
        .iter()
        .map(|t| t.name.as_str())
        .collect();
    for entry in &manifest.tables {
        if !ours.contains(&entry.name.as_str()) {
            return Err(InstallError::UnknownTable {
                table: entry.name.clone(),
            });
        }
        if entry.columns != expected_columns(&entry.name) {
            return Err(InstallError::ColumnsMismatch {
                table: entry.name.clone(),
            });
        }
    }
    for table in &ours {
        if !manifest.tables.iter().any(|entry| entry.name == *table) {
            return Err(InstallError::MissingTable {
                table: (*table).to_owned(),
            });
        }
    }
    // An attachment row's bytes are an artifact.
    if !with_closure
        && manifest
            .tables
            .iter()
            .any(|t| t.name == "attachments" && t.row_count != "0")
    {
        return Err(InstallError::NeedsArtifacts);
    }

    let tx = conn.transaction()?;
    let has_identity: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM vault_metadata WHERE singleton = 1)",
        [],
        |r| r.get(0),
    )?;
    if has_identity {
        return Err(InstallError::HasIdentity);
    }
    for table in &ours {
        let occupied: bool =
            tx.query_row(&format!("SELECT EXISTS(SELECT 1 FROM {table})"), [], |r| {
                r.get(0)
            })?;
        if occupied {
            return Err(InstallError::NotEmpty {
                table: (*table).to_owned(),
            });
        }
    }

    // The checks above read; from here on the transaction writes, and a failure drops it.
    tx.execute_batch("PRAGMA defer_foreign_keys = ON")?;
    crate::insert_vault_metadata(
        &tx,
        &Uuid::from_bytes(vault_id),
        i64::try_from(snapshot_schema).map_err(|_| {
            InstallError::Snapshot(SnapshotError::ManifestInvalid("the schema version"))
        })?,
        &manifest.vault_created_at,
    )?;

    let mut rows = 0_u64;
    let mut attachments = Vec::new();
    for (entry, (name, content)) in manifest.tables.iter().zip(&verified.files) {
        debug_assert_eq!(&entry.name, name);
        let holds_attachment_rows = with_closure && entry.name == "attachments";
        let names: Vec<&str> = entry.columns.iter().map(|c| c.name.as_str()).collect();
        let sql = format!(
            "INSERT INTO {} ({}) VALUES ({})",
            entry.name,
            names.join(", "),
            vec!["?"; names.len()].join(", ")
        );
        let mut insert = tx.prepare(&sql)?;
        for line in content.split_inclusive(|b| *b == b'\n') {
            let values = parse_row_line(line, &entry.columns)?;
            if holds_attachment_rows {
                attachments.push(attachment_of(&entry.columns, values)?);
            } else {
                insert.execute(rusqlite::params_from_iter(
                    values.into_iter().map(sql_value),
                ))?;
            }
            rows += 1;
        }
    }

    // The proof: what is installed exports to the snapshot, table for table. With a closure the
    // attachment rows do not exist yet, so the proof waits for them (`prove_installed`).
    if !with_closure {
        prove_in(&tx, verified, true)?;
    }
    tx.commit()?;
    Ok((
        InstalledSnapshot {
            tables: manifest.tables.len(),
            rows,
        },
        attachments,
    ))
}

/// The attachment row a snapshot's `attachments` line holds.
fn attachment_of(
    columns: &[ColumnEntry],
    values: Vec<OwnedValue>,
) -> Result<SnapshotAttachment, InstallError> {
    let malformed = || InstallError::Snapshot(SnapshotError::MalformedRow);
    let mut id = None;
    let mut plaintext_size = None;
    let mut mime_type = None;
    let mut original_filename = None;
    let mut created_at = None;
    for (column, value) in columns.iter().zip(values) {
        match (column.name.as_str(), value) {
            ("id", OwnedValue::Blob(bytes)) => {
                id = Some(<[u8; 16]>::try_from(bytes).map_err(|_| malformed())?);
            }
            ("plaintext_size", OwnedValue::Int(n)) => plaintext_size = Some(n),
            ("mime_type", OwnedValue::Text(t)) => mime_type = Some(t),
            ("mime_type", OwnedValue::Null) => mime_type = None,
            ("original_filename", OwnedValue::Text(t)) => original_filename = Some(t),
            ("original_filename", OwnedValue::Null) => original_filename = None,
            ("created_at", OwnedValue::Text(t)) => created_at = Some(t),
            _ => {}
        }
    }
    Ok(SnapshotAttachment {
        id: id.ok_or_else(malformed)?,
        plaintext_size: plaintext_size.ok_or_else(malformed)?,
        mime_type,
        original_filename,
        created_at: created_at.ok_or_else(malformed)?,
    })
}

/// The proof that what is installed exports to the snapshot, table for table (the same file hash, row
/// count and state digest, and the same header), run inside `tx`.
fn prove_in(tx: &Connection, verified: &Verified, check_schema: bool) -> Result<(), InstallError> {
    let manifest = &verified.manifest;
    let (again, _) = export_within(tx, &ExportInput::default())?;
    let reexported = verify(&again)?;
    if reexported.manifest.vault_created_at != manifest.vault_created_at
        || (check_schema && reexported.manifest.schema_version != manifest.schema_version)
    {
        return Err(InstallError::HeaderMismatch);
    }
    for entry in &manifest.tables {
        let other: Option<&TableEntry> = reexported
            .manifest
            .tables
            .iter()
            .find(|t| t.name == entry.name);
        if other != Some(entry) {
            return Err(InstallError::DigestMismatch {
                table: entry.name.clone(),
            });
        }
    }
    Ok(())
}

/// The proof of [`install_snapshot_with_closure`], once the attachment rows exist: the installed state
/// exports to the snapshot's tables, table for table (the same file hash, row count and state
/// digest), and the vault's creation time is the snapshot's. The schema version is not compared: the
/// build has carried the vault to its own schema since, and the tables' columns were checked equal
/// to this build's at install.
///
/// # Errors
/// [`InstallError::DigestMismatch`] naming the first table that differs, or
/// [`InstallError::HeaderMismatch`].
pub fn prove_installed(conn: &Connection, verified: &Verified) -> Result<(), InstallError> {
    // One read transaction, so the proof reads one view.
    let tx = conn.unchecked_transaction()?;
    prove_in(&tx, verified, false)
}

/// The tables a snapshot installs, for a caller that wants to know before it creates a vault.
#[must_use]
pub fn installed_table_names() -> Vec<String> {
    genesis_tables(Manifest::get())
        .iter()
        .filter(|t| t.coverage != Coverage::None)
        .map(|t| t.name.clone())
        .collect()
}
