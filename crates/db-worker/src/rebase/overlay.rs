//! The class-3 preservation overlay (personal-cfo-bxsbz.6; ADR 0074 decision 2, ADR 0073
//! Addendum D).
//!
//! A scratch vault is a copy of the base image: its class-3 tables hold what the device had when
//! the base was taken, and a credential never (`base_image::SECRET_BEARING_TABLES`). The live
//! vault's class-3 state has moved on since, and it is the authoritative copy: the credential, the
//! watermark, a staged import, `device.*` settings, a durable job, the audit trail, backup history
//! and the queue exist only on this device. The overlay carries them across, per the CLASS-0
//! manifest's `rebase_phase`:
//!
//! ```text
//! DbWorker::capture_preservation()   one critical section: the live class-3 rows and the unpushed
//!                                    commands (the immutable plan), read together
//! rollback_to(base_seq)              records the keys of the class-3 rows the base itself carries
//! apply_pulled / rebase_local        the class-1 state is rebuilt; replay writes its own memos,
//!                                    outbox rows and replay-owned audit rows
//! Scratch::overlay_class3(image)     in one transaction: drop ONLY the base-inherited rows, insert
//!                                    the stable live rows, validate their references against the
//!                                    rebuilt class-1 state, commit or leave the scratch untouched
//! ```
//!
//! **Only base-inherited rows are dropped.** A row the pulled envelopes or the replay wrote has a
//! key the base did not carry, so it stays unless the live vault holds the same key with other
//! content, which is a [`BlockCause::KeyCollision`] and not a silent choice. (A pulled command's
//! class-3 effect on a base-inherited row is re-run after the overlay by the registry of
//! bxsbz.6's next slice.)
//!
//! **Withheld, not overlaid.** The memo, the outbox and the replay-owned audit rows are rebuilt by
//! the replay or the reconciliation, under the new `op_seq`s; copying the live rows would point at
//! rows that no longer exist. A live memo that is not tied to any planned or queued command and is
//! not already in the scratch vault cannot be remapped, so it blocks.
//!
//! **A failure keeps everything.** The live vault is only read: the image is captured under its
//! writer lock and every write goes to the scratch vault, inside a transaction that is dropped
//! unless every reference holds. A [`RebaseBlockedLocalState`] names the table and the cause (row
//! counts and tags, never a value) and the scratch path, which the caller keeps as the evidence.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use rusqlite::types::Value;
use rusqlite::{params_from_iter, Connection};
use sha2::{Digest, Sha256};
use zeroize::Zeroize;

use super::execute::read_local_commands_on;
use super::{LocalCommand, RebaseError, Scratch};
use crate::base_image::SECRET_BEARING_TABLES;
use crate::{DbError, DbWorker, WorkerState};

/// What a missing reference does to a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Missing {
    /// The rebase is blocked: the person's local state names something the rebuilt vault lacks.
    Block,
    /// The row is a stale notice and is dropped (the manifest: "never blocks the swap").
    Drop,
}

/// One column that must name a row of a class-1 table in the rebuilt vault.
struct Ref {
    column: &'static str,
    target_table: &'static str,
    target_column: &'static str,
    missing: Missing,
}

/// How a table is carried across.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Drop the base-inherited keys, then insert the live rows.
    Keyed,
    /// A table with no primary key (one row): replace its contents with the live rows.
    ReplaceAll,
}

struct Spec {
    table: &'static str,
    /// A SQL predicate selecting the rows this overlay owns (the table holds other scopes too).
    filter: Option<&'static str>,
    kind: Kind,
    refs: &'static [Ref],
}

const fn connection_ref(missing: Missing) -> Ref {
    Ref {
        column: "connection_id",
        target_table: "connector_connections",
        target_column: "id",
        missing,
    }
}

const fn source_record_ref() -> Ref {
    Ref {
        column: "source_record_id",
        target_table: "source_records",
        target_column: "id",
        missing: Missing::Block,
    }
}

const fn account_ref(column: &'static str) -> Ref {
    Ref {
        column,
        target_table: "accounts",
        target_column: "id",
        missing: Missing::Block,
    }
}

/// The class-3 tables whose rows are `rebase_phase = "stable"`, in insert order. A tombstoned
/// connection counts as present (the identity row survives a delete).
const OVERLAID: &[Spec] = &[
    Spec {
        table: "sync_base",
        filter: None,
        kind: Kind::Keyed,
        refs: &[],
    },
    Spec {
        table: "vault_meta",
        filter: None,
        kind: Kind::ReplaceAll,
        refs: &[],
    },
    Spec {
        table: "vault_metadata",
        filter: None,
        kind: Kind::Keyed,
        refs: &[],
    },
    Spec {
        table: "outbox_queue",
        filter: None,
        kind: Kind::Keyed,
        refs: &[],
    },
    Spec {
        table: "settings",
        filter: Some("substr(key, 1, 7) = 'device.'"),
        kind: Kind::Keyed,
        refs: &[],
    },
    Spec {
        table: "connector_connection_secrets",
        filter: None,
        kind: Kind::Keyed,
        refs: &[connection_ref(Missing::Block)],
    },
    Spec {
        table: "connector_link_cursors",
        filter: None,
        kind: Kind::Keyed,
        refs: &[connection_ref(Missing::Block)],
    },
    Spec {
        table: "connector_attention",
        filter: None,
        kind: Kind::Keyed,
        refs: &[connection_ref(Missing::Drop)],
    },
    Spec {
        table: "staged_accounts",
        filter: None,
        kind: Kind::Keyed,
        refs: &[
            Ref {
                column: "source_batch_id",
                target_table: "source_batches",
                target_column: "id",
                missing: Missing::Block,
            },
            account_ref("matched_account_id"),
        ],
    },
    Spec {
        table: "staged_balances",
        filter: None,
        kind: Kind::Keyed,
        refs: &[source_record_ref(), account_ref("account_ref")],
    },
    Spec {
        table: "staged_transactions",
        filter: None,
        kind: Kind::Keyed,
        refs: &[source_record_ref(), account_ref("proposed_account_id")],
    },
    Spec {
        table: "durable_jobs",
        filter: None,
        kind: Kind::Keyed,
        refs: &[],
    },
    Spec {
        table: "backup_history",
        filter: None,
        kind: Kind::Keyed,
        refs: &[],
    },
];

/// Class-3 tables the overlay reads but carries by their own rule.
const AUDIT: Spec = Spec {
    table: "audit_events",
    filter: None,
    kind: Kind::Keyed,
    refs: &[],
};
const MEMOS: Spec = Spec {
    table: "command_idempotency_keys",
    filter: None,
    kind: Kind::Keyed,
    refs: &[],
};
const MIGRATIONS: Spec = Spec {
    table: "schema_migrations",
    filter: None,
    kind: Kind::Keyed,
    refs: &[],
};

/// The audit event types written by a planned command's apply arm: the replay writes them again
/// under its own plan, so the live row is withheld (ADR 0073 Addendum D). Every other event type is
/// a stable security or capability acknowledgement, or a token-removal record, and is never
/// dropped.
const REPLAY_OWNED_AUDIT: &[&str] = &[crate::apply::household::HOUSEHOLD_TIMEZONE_CHANGED];

/// Every class-3 table the rebase accounts for, with the rule that carries it: overlaid, or
/// withheld/validated/rebuilt (`sync_outbox`, `projection_cursors`, and the three read by their
/// own rule). `tests` fails when the CLASS-0 manifest gains a class-3 table that is not here.
#[cfg(test)]
pub(crate) fn class_three_tables_handled() -> Vec<&'static str> {
    OVERLAID
        .iter()
        .map(|spec| spec.table)
        .chain([
            AUDIT.table,
            MEMOS.table,
            MIGRATIONS.table,
            "sync_outbox",
            "projection_cursors",
        ])
        .collect()
}

/// The base-inherited keys of every [`Kind::Keyed`] overlay table, captured when the scratch vault
/// is rolled back to the base.
pub(super) type Inherited = BTreeMap<&'static str, Vec<Vec<Value>>>;

/// Why a rebase is blocked by local state. Counts and tags only: a value never appears.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockCause {
    /// Live rows name a row of `target` that the rebuilt class-1 state lacks.
    MissingReference {
        /// The referencing column.
        column: &'static str,
        /// The class-1 table it names.
        target: &'static str,
        /// The rows that do.
        rows: Offending,
    },
    /// A row the pulled envelopes or the replay wrote has the key of a live row with other content.
    KeyCollision {
        /// The rows that collide.
        rows: Offending,
    },
    /// A scratch row cannot be dropped because class-1 state still refers to it.
    StillReferenced {
        /// The rows.
        rows: Offending,
    },
    /// A live memo belongs to no planned or queued command and is not in the scratch vault.
    UnplannedMemo {
        /// The memos.
        rows: Offending,
    },
    /// A stable audit row has a command or correlation id that is not 16 bytes.
    MalformedAudit {
        /// The rows.
        rows: Offending,
    },
    /// The live and the scratch vault differ in a table's columns: they are not the same schema.
    SchemaDiffers,
    /// The applied migrations of the scratch vault are not the live vault's.
    MigrationHistoryDiffers,
    /// An outbox entry of the rebuilt vault names no operation-log row.
    OutboxWithoutLog {
        /// The entries.
        rows: Offending,
    },
    /// A projection cursor is ahead of the rebuilt operation log.
    CursorAhead {
        /// The cursors.
        rows: Offending,
    },
}

/// How many rows offended, and a few tags to tell them apart in the evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offending {
    /// The number of rows.
    pub count: usize,
    /// Up to five tags: the first 6 bytes of the SHA-256 of the row key, hex.
    pub sample: Vec<String>,
}

impl Offending {
    fn from_keys(keys: &[Vec<u8>]) -> Self {
        Self {
            count: keys.len(),
            sample: keys.iter().take(5).map(|key| tag(key)).collect(),
        }
    }
}

impl BlockCause {
    fn describe(&self) -> String {
        match self {
            Self::MissingReference {
                column,
                target,
                rows,
            } => format!(
                "{} row(s) name a {target} the rebuilt vault does not have ({column})",
                rows.count
            ),
            Self::KeyCollision { rows } => {
                format!("{} row(s) collide with a row the rebase wrote", rows.count)
            }
            Self::StillReferenced { rows } => {
                format!(
                    "{} row(s) are still referenced by rebuilt state",
                    rows.count
                )
            }
            Self::UnplannedMemo { rows } => format!(
                "{} idempotency memo(s) belong to no planned or queued command",
                rows.count
            ),
            Self::MalformedAudit { rows } => {
                format!(
                    "{} audit row(s) have a malformed command or correlation id",
                    rows.count
                )
            }
            Self::SchemaDiffers => {
                "the live and the scratch vault are not the same schema".to_owned()
            }
            Self::MigrationHistoryDiffers => {
                "the scratch vault's migration history is not the live vault's".to_owned()
            }
            Self::OutboxWithoutLog { rows } => {
                format!("{} outbox entr(ies) name no operation-log row", rows.count)
            }
            Self::CursorAhead { rows } => {
                format!(
                    "{} projection cursor(s) are ahead of the operation log",
                    rows.count
                )
            }
        }
    }
}

/// The typed block (`RebaseBlockedLocalState`): the person's device-local state does not fit the
/// rebuilt vault. The live vault is untouched and `scratch` is kept as the evidence; nothing was
/// overlaid onto it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebaseBlockedLocalState {
    /// The class-3 table.
    pub table: &'static str,
    /// What does not fit.
    pub cause: BlockCause,
    /// The scratch vault, kept for diagnosis.
    pub scratch: PathBuf,
}

impl RebaseBlockedLocalState {
    /// A sentence that names the table and the cause, never a value.
    #[must_use]
    pub fn describe(&self) -> String {
        format!(
            "the rebase is blocked by local state in {}: {}",
            self.table,
            self.cause.describe()
        )
    }
}

/// One table as read: its columns, its primary-key columns and its rows, sorted.
struct TableImage {
    table: &'static str,
    columns: Vec<String>,
    pk: Vec<usize>,
    rows: Vec<Vec<Value>>,
}

impl std::fmt::Debug for TableImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TableImage")
            .field("table", &self.table)
            .field("rows", &self.rows.len())
            .finish_non_exhaustive()
    }
}

impl TableImage {
    fn column(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c == name)
    }

    fn key_of(&self, row: &[Value]) -> Vec<Value> {
        self.pk.iter().map(|&i| row[i].clone()).collect()
    }
}

impl Drop for TableImage {
    fn drop(&mut self) {
        if !SECRET_BEARING_TABLES.contains(&self.table) {
            return;
        }
        for value in self.rows.iter_mut().flatten() {
            match value {
                Value::Text(text) => text.zeroize(),
                Value::Blob(bytes) => bytes.zeroize(),
                _ => {}
            }
        }
    }
}

/// The live vault's class-3 state and unpushed commands, captured together. Immutable: the
/// controller replays from [`Preservation::local`], not from a second read of the live vault.
pub struct Preservation {
    local: Vec<LocalCommand>,
    tables: Vec<TableImage>,
    audit: TableImage,
    memos: TableImage,
    migrations: TableImage,
    digest: [u8; 32],
}

impl std::fmt::Debug for Preservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Preservation")
            .field("local_commands", &self.local.len())
            .field("tables", &self.tables)
            .field("digest", &self.digest)
            .finish_non_exhaustive()
    }
}

impl Preservation {
    /// The unpushed commands as of the capture, in local order: the immutable plan.
    #[must_use]
    pub fn local(&self) -> &[LocalCommand] {
        &self.local
    }

    /// A digest of everything captured. A controller that captured under the writer lock compares
    /// it with a second capture under the lock that installs the swap: unequal means the live
    /// vault moved on and the rebase starts over.
    #[must_use]
    pub const fn digest(&self) -> [u8; 32] {
        self.digest
    }

    /// The command ids a rebase plans or has queued: their memo and audit rows are withheld.
    fn planned_ids(&self) -> BTreeSet<Vec<u8>> {
        let mut ids: BTreeSet<Vec<u8>> = self
            .local
            .iter()
            .map(|c| c.meta.command_id.as_bytes().to_vec())
            .collect();
        if let Some(queue) = self.tables.iter().find(|t| t.table == "outbox_queue") {
            if let Some(column) = queue.column("command_id") {
                for row in &queue.rows {
                    if let Value::Blob(id) = &row[column] {
                        ids.insert(id.clone());
                    }
                }
            }
        }
        ids
    }
}

impl DbWorker {
    /// Capture the preservation image: under the writer lock, in one read transaction, every
    /// stable class-3 row and the unpushed commands. The live vault is only read.
    ///
    /// # Errors
    /// [`RebaseError::Db`], or whatever [`DbWorker::read_local_commands`] refuses.
    pub fn capture_preservation(&self) -> Result<Preservation, RebaseError> {
        let guard = self.lock();
        if guard.state != WorkerState::Healthy {
            return Err(DbError::WorkerUnavailable(guard.state).into());
        }
        let tx = guard.conn.unchecked_transaction()?;
        let local = read_local_commands_on(&tx)?;
        let tables = OVERLAID
            .iter()
            .map(|spec| read_table(&tx, spec))
            .collect::<Result<Vec<_>, _>>()?;
        let audit = read_table(&tx, &AUDIT)?;
        let memos = read_table(&tx, &MEMOS)?;
        let migrations = read_table(&tx, &MIGRATIONS)?;
        drop(tx);
        let mut hasher = Sha256::new();
        for command in &local {
            hasher.update(command.meta.command_id.as_bytes());
        }
        for table in tables.iter().chain([&audit, &memos, &migrations]) {
            hasher.update(table.table.as_bytes());
            for row in &table.rows {
                let mut buf = Vec::new();
                for value in row {
                    encode_value(&mut buf, value);
                }
                hasher.update(u64::try_from(buf.len()).unwrap_or(u64::MAX).to_be_bytes());
                hasher.update(&buf);
            }
        }
        Ok(Preservation {
            local,
            tables,
            audit,
            memos,
            migrations,
            digest: hasher.finalize().into(),
        })
    }
}

/// The keys of the class-3 rows the freshly opened scratch vault carries, per overlay table.
pub(super) fn inherited_keys(worker: &DbWorker) -> Result<Inherited, RebaseError> {
    let guard = worker.lock();
    let mut out = Inherited::new();
    for spec in OVERLAID.iter().filter(|s| s.kind == Kind::Keyed) {
        let image = read_table(&guard.conn, spec)?;
        let keys = image.rows.iter().map(|row| image.key_of(row)).collect();
        out.insert(spec.table, keys);
    }
    Ok(out)
}

/// What an overlay did to one table.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OverlayTable {
    /// The table.
    pub table: &'static str,
    /// Base-inherited rows dropped.
    pub removed: usize,
    /// Live rows inserted.
    pub inserted: usize,
    /// Live rows already present, byte for byte.
    pub unchanged: usize,
    /// Live rows dropped because the thing they name is gone (a stale notice).
    pub stale: usize,
    /// Live rows deliberately not carried: rebuilt by the replay.
    pub withheld: usize,
}

/// What [`Scratch::overlay_class3`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OverlayReport {
    /// One entry per table, in the order they were carried.
    pub tables: Vec<OverlayTable>,
}

impl OverlayReport {
    /// What happened to `table`.
    #[must_use]
    pub fn table(&self, table: &str) -> Option<&OverlayTable> {
        self.tables.iter().find(|t| t.table == table)
    }
}

impl Scratch<'_> {
    /// Carry the live vault's stable class-3 state onto the rebuilt scratch vault, once, after
    /// [`Scratch::rebase_local`]. The whole overlay is one transaction: if any reference does not
    /// hold, nothing is written and the typed [`RebaseError::BlockedLocalState`] names why.
    ///
    /// # Errors
    /// [`RebaseError::OverlayBeforeReplay`], [`RebaseError::AlreadyOverlaid`],
    /// [`RebaseError::BlockedLocalState`], or [`RebaseError::Db`].
    pub fn overlay_class3(&mut self, image: &Preservation) -> Result<OverlayReport, RebaseError> {
        if !self.rebased {
            return Err(RebaseError::OverlayBeforeReplay);
        }
        if self.overlaid {
            return Err(RebaseError::AlreadyOverlaid);
        }
        let report = {
            let guard = self.worker.lock();
            overlay_in(&guard.conn, image, &self.inherited, &self.path)?
        };
        self.overlaid = true;
        Ok(report)
    }
}

fn overlay_in(
    conn: &Connection,
    image: &Preservation,
    inherited: &Inherited,
    scratch: &Path,
) -> Result<OverlayReport, RebaseError> {
    let blocked = |table: &'static str, cause: BlockCause| {
        RebaseError::BlockedLocalState(Box::new(RebaseBlockedLocalState {
            table,
            cause,
            scratch: scratch.to_path_buf(),
        }))
    };
    let tx = conn.unchecked_transaction()?;

    for table in image
        .tables
        .iter()
        .chain([&image.audit, &image.memos, &image.migrations])
    {
        if columns_of(&tx, table.table)? != table.columns {
            return Err(blocked(table.table, BlockCause::SchemaDiffers));
        }
    }
    let scratch_migrations = read_table(&tx, &MIGRATIONS)?;
    if migration_set(&scratch_migrations) != migration_set(&image.migrations) {
        return Err(blocked(
            MIGRATIONS.table,
            BlockCause::MigrationHistoryDiffers,
        ));
    }

    // Drop first, insert second: a key may be dropped and written back in the same overlay.
    let removed = OVERLAID
        .iter()
        .map(|spec| drop_inherited(&tx, spec, inherited, &blocked))
        .collect::<Result<Vec<_>, _>>()?;
    let mut report = OverlayReport::default();
    for ((spec, table), removed) in OVERLAID.iter().zip(&image.tables).zip(removed) {
        let mut entry = OverlayTable {
            table: spec.table,
            removed,
            ..OverlayTable::default()
        };
        let rows: Vec<&Vec<Value>> = table.rows.iter().collect();
        place(&tx, spec, table, &rows, &mut entry, &blocked)?;
        report.tables.push(entry);
    }

    let mut audit = OverlayTable {
        table: AUDIT.table,
        ..OverlayTable::default()
    };
    overlay_audit(&tx, image, &mut audit, &blocked)?;
    report.tables.push(audit);

    let mut memos = OverlayTable {
        table: MEMOS.table,
        ..OverlayTable::default()
    };
    check_memos(&tx, image, &mut memos, &blocked)?;
    report.tables.push(memos);

    check_outbox(&tx, &blocked)?;
    check_cursors(&tx, &blocked)?;
    tx.commit()?;
    Ok(report)
}

/// Delete the rows of `spec` the base carried. Returns how many.
fn drop_inherited(
    tx: &Connection,
    spec: &Spec,
    inherited: &Inherited,
    blocked: &dyn Fn(&'static str, BlockCause) -> RebaseError,
) -> Result<usize, RebaseError> {
    if spec.kind == Kind::ReplaceAll {
        return Ok(tx.execute(&format!("DELETE FROM {}", quote(spec.table)), [])?);
    }
    let pk = pk_columns(tx, spec.table)?;
    let predicate = pk
        .iter()
        .enumerate()
        .map(|(i, column)| format!("{} = ?{}", quote(column), i + 1))
        .collect::<Vec<_>>()
        .join(" AND ");
    let sql = format!("DELETE FROM {} WHERE {predicate}", quote(spec.table));
    let mut removed = 0;
    let mut still_referenced = Vec::new();
    for key in inherited.get(spec.table).into_iter().flatten() {
        match tx.execute(&sql, params_from_iter(key.iter())) {
            Ok(n) => removed += n,
            Err(rusqlite::Error::SqliteFailure(error, _))
                if error.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_FOREIGNKEY =>
            {
                still_referenced.push(key_bytes(key));
            }
            Err(other) => return Err(other.into()),
        }
    }
    if still_referenced.is_empty() {
        Ok(removed)
    } else {
        Err(blocked(
            spec.table,
            BlockCause::StillReferenced {
                rows: Offending::from_keys(&still_referenced),
            },
        ))
    }
}

/// Insert `rows` of `table` into the scratch vault: references checked first, then each row is
/// skipped if it is already there byte for byte and refused if its key holds other content.
fn place(
    tx: &Connection,
    spec: &Spec,
    table: &TableImage,
    rows: &[&Vec<Value>],
    entry: &mut OverlayTable,
    blocked: &dyn Fn(&'static str, BlockCause) -> RebaseError,
) -> Result<(), RebaseError> {
    let mut keep: Vec<&Vec<Value>> = rows.to_vec();
    for reference in spec.refs {
        let Some(column) = table.column(reference.column) else {
            return Err(blocked(spec.table, BlockCause::SchemaDiffers));
        };
        let lookup = format!(
            "SELECT EXISTS (SELECT 1 FROM {} WHERE {} = ?1)",
            quote(reference.target_table),
            quote(reference.target_column)
        );
        let mut missing = Vec::new();
        keep.retain(|row| {
            let value = &row[column];
            if matches!(value, Value::Null) {
                return true;
            }
            let found: bool = tx
                .query_row(&lookup, [value], |r| r.get(0))
                .unwrap_or(false);
            if found {
                return true;
            }
            match reference.missing {
                Missing::Block => missing.push(key_bytes(&table.key_of(row))),
                Missing::Drop => entry.stale += 1,
            }
            false
        });
        if !missing.is_empty() {
            return Err(blocked(
                spec.table,
                BlockCause::MissingReference {
                    column: reference.column,
                    target: reference.target_table,
                    rows: Offending::from_keys(&missing),
                },
            ));
        }
    }

    let columns = table
        .columns
        .iter()
        .map(|c| quote(c))
        .collect::<Vec<_>>()
        .join(", ");
    let marks = (1..=table.columns.len())
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(", ");
    let insert = format!(
        "INSERT INTO {} ({columns}) VALUES ({marks})",
        quote(spec.table)
    );
    let mut collisions = Vec::new();
    for row in keep {
        if spec.kind == Kind::Keyed {
            if let Some(existing) = row_by_key(tx, table, &table.key_of(row))? {
                if existing == **row {
                    entry.unchanged += 1;
                } else {
                    collisions.push(key_bytes(&table.key_of(row)));
                }
                continue;
            }
        }
        tx.execute(&insert, params_from_iter(row.iter()))?;
        entry.inserted += 1;
    }
    if collisions.is_empty() {
        Ok(())
    } else {
        Err(blocked(
            spec.table,
            BlockCause::KeyCollision {
                rows: Offending::from_keys(&collisions),
            },
        ))
    }
}

/// Carry the stable audit rows; withhold the replay-owned ones. Audit is append-only, so nothing
/// is ever dropped: a base-inherited row is also a live row, and is found already in place.
fn overlay_audit(
    tx: &Connection,
    image: &Preservation,
    entry: &mut OverlayTable,
    blocked: &dyn Fn(&'static str, BlockCause) -> RebaseError,
) -> Result<(), RebaseError> {
    let table = &image.audit;
    let (Some(kind), Some(command), Some(correlation)) = (
        table.column("event_type"),
        table.column("command_id"),
        table.column("correlation_id"),
    ) else {
        return Err(blocked(AUDIT.table, BlockCause::SchemaDiffers));
    };
    let mut stable = Vec::new();
    let mut malformed = Vec::new();
    for row in &table.rows {
        if let Value::Text(event) = &row[kind] {
            if REPLAY_OWNED_AUDIT.contains(&event.as_str()) {
                entry.withheld += 1;
                continue;
            }
        }
        let well_formed = |v: &Value| matches!(v, Value::Blob(b) if b.len() == 16);
        if well_formed(&row[command]) && well_formed(&row[correlation]) {
            stable.push(row);
        } else {
            malformed.push(key_bytes(&table.key_of(row)));
        }
    }
    if !malformed.is_empty() {
        return Err(blocked(
            AUDIT.table,
            BlockCause::MalformedAudit {
                rows: Offending::from_keys(&malformed),
            },
        ));
    }
    place(tx, &AUDIT, table, &stable, entry, blocked)
}

/// A live memo is withheld when its command is planned or queued (the replay rebuilt it or
/// queued it without one), and otherwise must already be in the scratch vault, unchanged.
fn check_memos(
    tx: &Connection,
    image: &Preservation,
    entry: &mut OverlayTable,
    blocked: &dyn Fn(&'static str, BlockCause) -> RebaseError,
) -> Result<(), RebaseError> {
    let table = &image.memos;
    let Some(command) = table.column("command_id") else {
        return Err(blocked(MEMOS.table, BlockCause::SchemaDiffers));
    };
    let planned = image.planned_ids();
    let mut unplanned = Vec::new();
    for row in &table.rows {
        if matches!(&row[command], Value::Blob(id) if planned.contains(id)) {
            entry.withheld += 1;
            continue;
        }
        let key = table.key_of(row);
        match row_by_key(tx, table, &key)? {
            Some(existing) if existing == *row => entry.unchanged += 1,
            _ => unplanned.push(key_bytes(&key)),
        }
    }
    if unplanned.is_empty() {
        Ok(())
    } else {
        Err(blocked(
            MEMOS.table,
            BlockCause::UnplannedMemo {
                rows: Offending::from_keys(&unplanned),
            },
        ))
    }
}

/// Every outbox entry of the rebuilt vault names its operation-log row (the manifest's
/// `reference_validation` for `sync_outbox`).
fn check_outbox(
    tx: &Connection,
    blocked: &dyn Fn(&'static str, BlockCause) -> RebaseError,
) -> Result<(), RebaseError> {
    let mut stmt = tx.prepare(
        "SELECT o.command_id FROM sync_outbox o
          WHERE NOT EXISTS (SELECT 1 FROM operation_log l
                             WHERE l.command_id = o.command_id AND l.op_seq = o.op_seq)",
    )?;
    let orphans = stmt
        .query_map([], |r| r.get::<_, Vec<u8>>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    if orphans.is_empty() {
        Ok(())
    } else {
        Err(blocked(
            "sync_outbox",
            BlockCause::OutboxWithoutLog {
                rows: Offending::from_keys(&orphans),
            },
        ))
    }
}

/// The rebuild wrote the projection cursors; one ahead of the rebuilt log is not valid evidence.
fn check_cursors(
    tx: &Connection,
    blocked: &dyn Fn(&'static str, BlockCause) -> RebaseError,
) -> Result<(), RebaseError> {
    let mut stmt = tx.prepare(
        "SELECT read_model_name FROM projection_cursors
          WHERE last_applied_op_seq > (SELECT COALESCE(MAX(op_seq), 0) FROM operation_log)",
    )?;
    let ahead = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    if ahead.is_empty() {
        Ok(())
    } else {
        let keys: Vec<Vec<u8>> = ahead.into_iter().map(String::into_bytes).collect();
        Err(blocked(
            "projection_cursors",
            BlockCause::CursorAhead {
                rows: Offending::from_keys(&keys),
            },
        ))
    }
}

fn migration_set(table: &TableImage) -> BTreeSet<Vec<u8>> {
    let (Some(version), Some(hash)) = (table.column("version"), table.column("content_hash"))
    else {
        return BTreeSet::new();
    };
    table
        .rows
        .iter()
        .map(|row| key_bytes(&[row[version].clone(), row[hash].clone()]))
        .collect()
}

fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn columns_of(conn: &Connection, table: &str) -> Result<Vec<String>, rusqlite::Error> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({})", quote(table)))?;
    let columns = stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<Result<_, _>>();
    columns
}

/// The primary-key column names of `table`, in key order.
fn pk_columns(conn: &Connection, table: &str) -> Result<Vec<String>, rusqlite::Error> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({})", quote(table)))?;
    let mut keyed = stmt
        .query_map([], |r| Ok((r.get::<_, i64>(5)?, r.get::<_, String>(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    keyed.retain(|(position, _)| *position > 0);
    keyed.sort();
    Ok(keyed.into_iter().map(|(_, name)| name).collect())
}

fn read_table(conn: &Connection, spec: &Spec) -> Result<TableImage, rusqlite::Error> {
    let columns = columns_of(conn, spec.table)?;
    let pk_names = pk_columns(conn, spec.table)?;
    let pk = pk_names
        .iter()
        .filter_map(|name| columns.iter().position(|c| c == name))
        .collect();
    let select = columns
        .iter()
        .map(|c| quote(c))
        .collect::<Vec<_>>()
        .join(", ");
    let filter = spec
        .filter
        .map_or_else(String::new, |f| format!(" WHERE {f}"));
    let mut stmt = conn.prepare(&format!(
        "SELECT {select} FROM {}{filter}",
        quote(spec.table)
    ))?;
    let width = columns.len();
    let mut rows = stmt
        .query_map([], |r| {
            (0..width)
                .map(|i| r.get::<_, Value>(i))
                .collect::<Result<Vec<_>, _>>()
        })?
        .collect::<Result<Vec<_>, _>>()?;
    rows.sort_by_key(|row| key_bytes(row));
    Ok(TableImage {
        table: spec.table,
        columns,
        pk,
        rows,
    })
}

/// The row of `table` with primary key `key`, in the scratch vault.
fn row_by_key(
    conn: &Connection,
    table: &TableImage,
    key: &[Value],
) -> Result<Option<Vec<Value>>, rusqlite::Error> {
    if table.pk.is_empty() {
        return Ok(None);
    }
    let select = table
        .columns
        .iter()
        .map(|c| quote(c))
        .collect::<Vec<_>>()
        .join(", ");
    let predicate = table
        .pk
        .iter()
        .enumerate()
        .map(|(i, &column)| format!("{} = ?{}", quote(&table.columns[column]), i + 1))
        .collect::<Vec<_>>()
        .join(" AND ");
    let width = table.columns.len();
    let mut stmt = conn.prepare(&format!(
        "SELECT {select} FROM {} WHERE {predicate}",
        quote(table.table)
    ))?;
    let mut rows = stmt.query(params_from_iter(key.iter()))?;
    match rows.next()? {
        Some(r) => Ok(Some(
            (0..width)
                .map(|i| r.get::<_, Value>(i))
                .collect::<Result<_, _>>()?,
        )),
        None => Ok(None),
    }
}

/// A canonical, injective encoding of one value.
fn encode_value(buf: &mut Vec<u8>, value: &Value) {
    match value {
        Value::Null => buf.push(0),
        Value::Integer(n) => {
            buf.push(1);
            buf.extend_from_slice(&n.to_be_bytes());
        }
        Value::Real(x) => {
            buf.push(2);
            buf.extend_from_slice(&x.to_bits().to_be_bytes());
        }
        Value::Text(text) => {
            buf.push(3);
            buf.extend_from_slice(&u64::try_from(text.len()).unwrap_or(u64::MAX).to_be_bytes());
            buf.extend_from_slice(text.as_bytes());
        }
        Value::Blob(bytes) => {
            buf.push(4);
            buf.extend_from_slice(&u64::try_from(bytes.len()).unwrap_or(u64::MAX).to_be_bytes());
            buf.extend_from_slice(bytes);
        }
    }
}

fn key_bytes(values: &[Value]) -> Vec<u8> {
    let mut buf = Vec::new();
    for value in values {
        encode_value(&mut buf, value);
    }
    buf
}

/// A short, value-free tag for a row key: enough to tell rows apart in evidence.
fn tag(key: &[u8]) -> String {
    Sha256::digest(key)[..6]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
