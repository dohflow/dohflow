//! The live closure a replica holds (`vault-format.md` B.2 `sync_live_closure`, C.9;
//! `personal-cfo-1cltt.7`, slice D1).
//!
//! The live closure at the preceding log position is "exactly what a snapshot at that position
//! would carry, as a set of `(id, ciphertext_sha256)` pairs", a function of replicated state. A
//! replica cannot rebuild it from class-1 rows alone (an id is keyed under the producer's DEK and
//! its hash is the sealed bytes'), so it keeps it: seeded from the snapshot it bootstrapped from,
//! and maintained by each accepted envelope's delta. The materializer and the stage-9 verdicts take
//! it ([`load`]) as their `LiveClosure` argument; the `sync_artifacts` mapping is a cache and never
//! decides one.
//!
//! **The invariant.** The table is the live closure at the vault's *accepted* position, and it is
//! written only where the vault's class-1 state equals the accepted state at that position:
//!
//! * [`apply_delta`], by the transaction that applies an accepted envelope, **after** the command's
//!   writes (the prune must see post-apply state);
//! * [`seed`], at bootstrap;
//! * [`rebuild_for_epoch`], at a key epoch.
//!
//! **Nothing local ever writes it**: no local command and no local-command replay. A prune evaluated
//! in a vault that holds an unpushed delete would drop an object the accepted log still holds live,
//! and a later rebinding would pass on this device and fail on a replica that tailed the log. So
//! each writer refuses ([`LiveClosureError::LocalChangesPending`]) while the outbox is not empty.
//!
//! The functions take the **caller's transaction**: the engine (`personal-cfo-elciw`) calls
//! [`apply_delta`] in the transaction that applies the envelope (the direct apply, each pulled
//! envelope on the rebase scratch, the push path's outbox-emptying transaction), so a failed apply
//! rolls the table back with the command.

use rusqlite::{params, Connection};
use sync_core::artifact::ArtifactId;
use sync_core::artifact_closure::{LiveClosure, LogicalRef};
use thiserror::Error;

use crate::sync_produce::{closure_sources_in, ProduceError};
use crate::{DbError, DbWorker};

/// One row of the live closure: an artifact id, the hash the snapshot at that position binds to it,
/// and the logical attachment or batch it belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct LiveEntry {
    /// `sync_artifact_id_e`.
    pub id: ArtifactId,
    /// The SHA-256 of the artifact's AEAD output.
    pub ciphertext_sha256: [u8; 32],
    /// What it belongs to.
    pub logical: LogicalRef,
}

/// Why the live closure was not read or written.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum LiveClosureError {
    /// The vault holds an unpushed local command (an outbox row), so its class-1 state is not the
    /// accepted state: writing the live closure from it could drop an object the accepted log still
    /// holds live.
    #[error("the vault holds unpushed local commands: its state is not the accepted state")]
    LocalChangesPending,
    /// A stored row is not a well-formed entry.
    #[error("the live closure holds a malformed row")]
    Malformed,
    /// The reachability read failed.
    #[error("{0}")]
    Sources(#[from] ProduceError),
    /// The database failed, or the worker does not accept writes.
    #[error("{0}")]
    Db(#[from] DbError),
}

impl From<rusqlite::Error> for LiveClosureError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Db(DbError::from(error))
    }
}

fn kind_of(logical: &LogicalRef) -> (&'static str, [u8; 16]) {
    match logical {
        LogicalRef::Attachment(id) => ("attachment", *id),
        LogicalRef::Batch(id) => ("batch", *id),
    }
}

fn epoch_of(key_epoch: u64) -> Result<i64, LiveClosureError> {
    i64::try_from(key_epoch).map_err(|_| LiveClosureError::Malformed)
}

/// Refuse while the vault holds an unpushed local command (see the module comment).
fn require_accepted_state(conn: &Connection) -> Result<(), LiveClosureError> {
    let pending: bool =
        conn.query_row("SELECT EXISTS(SELECT 1 FROM sync_outbox)", [], |r| r.get(0))?;
    if pending {
        Err(LiveClosureError::LocalChangesPending)
    } else {
        Ok(())
    }
}

/// The live closure of `key_epoch`, as the materializer and the stage-9 verdicts take it: each
/// logical object with its ids and hashes, and each source record of a live batch with the batch
/// that holds it (the records are the vault's own `source_records` rows).
///
/// # Errors
/// [`LiveClosureError`].
pub fn load(conn: &Connection, key_epoch: u64) -> Result<LiveClosure, LiveClosureError> {
    let epoch = epoch_of(key_epoch)?;
    let mut live = LiveClosure::new();
    let mut stmt = conn.prepare(
        "SELECT sync_artifact_id, ciphertext_sha256, logical_kind, logical_id
           FROM sync_live_closure WHERE key_epoch = ?1 ORDER BY sync_artifact_id",
    )?;
    let rows = stmt.query_map([epoch], |r| {
        Ok((
            r.get::<_, Vec<u8>>(0)?,
            r.get::<_, Vec<u8>>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, Vec<u8>>(3)?,
        ))
    })?;
    for row in rows {
        let (id, hash, kind, logical) = row?;
        let id = <[u8; 32]>::try_from(id).map_err(|_| LiveClosureError::Malformed)?;
        let hash = <[u8; 32]>::try_from(hash).map_err(|_| LiveClosureError::Malformed)?;
        let logical = <[u8; 16]>::try_from(logical).map_err(|_| LiveClosureError::Malformed)?;
        let logical = match kind.as_str() {
            "attachment" => LogicalRef::Attachment(logical),
            "batch" => LogicalRef::Batch(logical),
            _ => return Err(LiveClosureError::Malformed),
        };
        live.insert(logical, ArtifactId::from_derived(id), hash);
    }
    let mut records = conn.prepare(
        "SELECT r.source_batch_id, r.id FROM source_records r
          WHERE r.source_batch_id IN (SELECT logical_id FROM sync_live_closure
                                       WHERE key_epoch = ?1 AND logical_kind = 'batch')",
    )?;
    let pairs = records.query_map([epoch], |r| {
        Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?))
    })?;
    for pair in pairs {
        let (batch, record) = pair?;
        let batch = <[u8; 16]>::try_from(batch).map_err(|_| LiveClosureError::Malformed)?;
        let record = <[u8; 16]>::try_from(record).map_err(|_| LiveClosureError::Malformed)?;
        live.insert_record(batch, record);
    }
    Ok(live)
}

fn insert(
    conn: &Connection,
    key_epoch: i64,
    entries: &[LiveEntry],
) -> Result<(), LiveClosureError> {
    for entry in entries {
        let (kind, logical) = kind_of(&entry.logical);
        conn.execute(
            "INSERT INTO sync_live_closure
                 (key_epoch, sync_artifact_id, ciphertext_sha256, logical_kind, logical_id)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (key_epoch, sync_artifact_id) DO UPDATE SET
                 ciphertext_sha256 = excluded.ciphertext_sha256,
                 logical_kind = excluded.logical_kind,
                 logical_id = excluded.logical_id",
            params![
                key_epoch,
                entry.id.as_bytes().as_slice(),
                entry.ciphertext_sha256.as_slice(),
                kind,
                logical.as_slice(),
            ],
        )?;
    }
    Ok(())
}

/// Bring the live closure to the position after an accepted envelope: add the delta's `entries`,
/// then remove every logical object that class-1 reachability no longer holds (an attachment row
/// that is gone, a batch no provenance link reaches: the genesis closure's rule, from the same
/// function). `conn` is the transaction that applies the envelope, and this runs **after** the
/// command's writes in it, so the prune sees the state after the apply; a failed apply rolls the
/// table back with the command.
///
/// # Errors
/// [`LiveClosureError::LocalChangesPending`] while the outbox is not empty, otherwise
/// [`LiveClosureError`].
pub fn apply_delta(
    conn: &Connection,
    key_epoch: u64,
    entries: &[LiveEntry],
) -> Result<(), LiveClosureError> {
    require_accepted_state(conn)?;
    let epoch = epoch_of(key_epoch)?;
    insert(conn, epoch, entries)?;
    // The prune: every logical object of this epoch that reachability no longer holds.
    let sources = closure_sources_in(conn)?;
    let mut objects: Vec<(String, Vec<u8>)> = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT DISTINCT logical_kind, logical_id FROM sync_live_closure WHERE key_epoch = ?1",
        )?;
        let rows = stmt.query_map([epoch], |r| Ok((r.get(0)?, r.get(1)?)))?;
        for row in rows {
            objects.push(row?);
        }
    }
    for (kind, logical) in objects {
        let held = match kind.as_str() {
            "attachment" => sources
                .attachments
                .iter()
                .any(|id| id.as_slice() == logical),
            "batch" => sources.batches.iter().any(|id| id.as_slice() == logical),
            _ => return Err(LiveClosureError::Malformed),
        };
        if !held {
            conn.execute(
                "DELETE FROM sync_live_closure
                  WHERE key_epoch = ?1 AND logical_kind = ?2 AND logical_id = ?3",
                params![epoch, kind, logical],
            )?;
        }
    }
    Ok(())
}

/// Seed the live closure of `key_epoch` from a snapshot's closure, at bootstrap: the epoch's rows
/// are replaced by `entries`. The snapshot is the accepted state, so there is no prune.
///
/// # Errors
/// [`LiveClosureError`].
pub fn seed(
    conn: &Connection,
    key_epoch: u64,
    entries: &[LiveEntry],
) -> Result<(), LiveClosureError> {
    require_accepted_state(conn)?;
    let epoch = epoch_of(key_epoch)?;
    conn.execute(
        "DELETE FROM sync_live_closure WHERE key_epoch = ?1",
        [epoch],
    )?;
    insert(conn, epoch, entries)
}

/// Rebuild the live closure at a key epoch from the re-addressed closure: every row of every other
/// epoch is removed, and `key_epoch`'s rows are `entries`. (A past epoch's closure is read by
/// nothing: a verdict is judged against the epoch in force.)
///
/// # Errors
/// [`LiveClosureError`].
pub fn rebuild_for_epoch(
    conn: &Connection,
    key_epoch: u64,
    entries: &[LiveEntry],
) -> Result<(), LiveClosureError> {
    require_accepted_state(conn)?;
    let epoch = epoch_of(key_epoch)?;
    conn.execute("DELETE FROM sync_live_closure", [])?;
    insert(conn, epoch, entries)
}

impl DbWorker {
    /// The live closure of `key_epoch` this vault holds, for [`crate::sync_receive::receive_delta`]
    /// and [`DbWorker::materialize_batch`]. During a rebase, the vault the envelope applies to is the
    /// scratch, so the caller loads it from there.
    ///
    /// # Errors
    /// [`LiveClosureError`].
    pub fn live_closure(&self, key_epoch: u64) -> Result<LiveClosure, LiveClosureError> {
        load(&self.read_connection()?, key_epoch)
    }

    /// [`seed`] in its own transaction, refused when the worker does not accept writes.
    ///
    /// # Errors
    /// [`LiveClosureError`].
    pub fn seed_live_closure(
        &self,
        key_epoch: u64,
        entries: &[LiveEntry],
    ) -> Result<(), LiveClosureError> {
        self.write_sync_state(|tx| seed(tx, key_epoch, entries))
    }

    /// [`rebuild_for_epoch`] in its own transaction, refused when the worker does not accept writes.
    ///
    /// # Errors
    /// [`LiveClosureError`].
    pub fn rebuild_live_closure_for_epoch(
        &self,
        key_epoch: u64,
        entries: &[LiveEntry],
    ) -> Result<(), LiveClosureError> {
        self.write_sync_state(|tx| rebuild_for_epoch(tx, key_epoch, entries))
    }
}
