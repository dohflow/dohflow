//! Arming a founding device whose vault changed after its genesis was exported (the genesis gap;
//! `docs/architecture/vault-format.md` C.8 "The vault after the export"; `personal-cfo-1cltt.5`,
//! slice C).
//!
//! A device that published its genesis and then took commands before it was armed has effects in
//! its vault that no envelope will ever carry: the log says `EnabledWithLocalChanges`. Arming it by
//! capturing a base from the live vault would make those effects part of "what the server has
//! seen", and no other replica could ever reproduce them. [`reconcile_genesis_gap`] arms it so the
//! server's log and the device agree:
//!
//! 1. **Gates, before anything is written.** The genesis is `Published` and its snapshot identity
//!    is recorded (`sync_genesis`, migration 73); the device is not armed; the service's log head is
//!    still `0` and the snapshot it holds is the one this device published (C5); every command in
//!    the gap has its recorded issue time and its payload (C3).
//! 2. **The base is the genesis, fetched, never the live vault** (C1, R-b). The genesis container is
//!    streamed to a temporary file under a size bound and checked against the digest recorded when
//!    this device sent it, opened with the epoch key, installed into a throwaway replica
//!    ([`crate::sync_bootstrap`]) whose captured base image is the base this device is armed on.
//! 3. **The gap is replayed on a scratch copy of that base**, in log order, each command under its
//!    recorded ids and issue time (C3), and **every step is proved** (C2): the write-set hash the
//!    replay produces is the one the log recorded, and at the end the scratch vault's class-1 state
//!    is the live vault's, table by table. Any difference refuses arming and names the command, so a
//!    gap that cannot be reproduced is never sent.
//! 4. **Arming is one transaction** (C4): it re-reads the log's high-water mark (a command taken
//!    meanwhile aborts it), writes the replayed commands into the outbox in log order, puts the
//!    genesis base in place, and records the `sync_base` and `sync_position` rows. A crash leaves
//!    the device unarmed, with its scratch files to discard.
//!
//! The live vault is only read until step 4.

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use sync_core::header::parse_container;
use sync_core::membership::VerifiedState;
use sync_core::snapshot::{verify, SnapshotError, Verified};
use sync_core::snapshot_service::{
    copy_snapshot_container, CopyError, ServiceError, SnapshotService,
};
use sync_core::status::EngineVersion;
use thiserror::Error;
use uuid::Uuid;
use vault_crypto::sync_envelope::EpochKey;
use vault_crypto::sync_snapshot::{SnapshotOpenFailure, SnapshotOpener};

use crate::base_image::{
    base_image_path, base_image_temp_path, copy_file_synced, fsync_dir, sha256_file, sync_dir,
};
use crate::logical_export::{
    export_genesis_plaintext, export_genesis_with_mark, ExportError, ExportInput,
};
use crate::payload::decode_payload;
use crate::rebase::{command_envelope, RebaseError, StepContext};
use crate::sync_bootstrap::{bootstrap_replica, BootstrapError, ReplicaSource};
use crate::sync_genesis::{self, GenesisState, GenesisStoreError};
use crate::{
    ActorType, CommandMeta, DbError, DbWorker, IssuedAt, Outcome, WorkerState, WriteCommand,
};

/// The largest genesis container this client will accept: `max_snapshot_container_bytes` (protocol
/// 7.1), the largest well-formed `DFSY` container. A service that sends more is cut off.
pub const MAX_GENESIS_CONTAINER_BYTES: u64 = 1_073_742_137;

/// The directory the reconciliation builds in, beside the vault. A start that finds it discards it
/// first (it holds only copies).
#[must_use]
pub fn gap_dir(vault_path: &Path) -> PathBuf {
    let mut name = vault_path.as_os_str().to_owned();
    name.push(".genesis-gap");
    PathBuf::from(name)
}

/// What the caller supplies: the facts about the genesis that only the engine (which owns the key
/// custody and the verified chain) can vouch for.
pub struct GenesisProof<'a> {
    /// The epoch key of the genesis's epoch (`K_1`).
    pub epoch_key: &'a EpochKey,
    /// The verified membership state the genesis container is judged against.
    pub chain: &'a VerifiedState,
    /// This build's `engine_version`.
    pub own_engine_version: &'a EngineVersion,
}

/// One command of the gap, as it was recorded: everything a faithful replay needs.
#[derive(Debug, Clone)]
pub struct GapCommand {
    /// The command's metadata, original `issued_at` included (C3).
    pub meta: CommandMeta,
    /// The decoded command.
    pub cmd: WriteCommand,
    /// Its `op_seq` in the live vault.
    pub op_seq: i64,
    /// The write-set hash the live vault recorded for it.
    pub write_set_hash: [u8; 32],
}

/// How the reconciliation ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Armed {
    /// The gap commands now in the outbox, in log order.
    pub commands: usize,
    /// The log position the base image is of (`0`: the genesis).
    pub base_seq: i64,
}

/// Why the device was not armed. Every refusal leaves the live vault as it was: nothing is written
/// to it until the last transaction commits.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum GapError {
    /// There is no persisted genesis, or it is not `Published`.
    #[error("the genesis is not published")]
    GenesisNotPublished(Option<GenesisState>),
    /// The genesis is published, but this device did not record which snapshot it was (a vault
    /// that enabled Sync under an earlier build): the service's copy is not trusted.
    #[error("the genesis snapshot's identity was not recorded")]
    GenesisIdentityNotRecorded,
    /// The device already has a base.
    #[error("the device is already armed")]
    AlreadyArmed,
    /// A command taken after the export has no recorded issue time: it cannot be replayed (C3).
    #[error("the command {command_id} has no recorded issue time")]
    MissingIssuedAt {
        /// The command.
        command_id: Uuid,
    },
    /// A command taken after the export has no recorded payload this build can decode.
    #[error("the command {command_id} cannot be read back")]
    UnreadableCommand {
        /// The command.
        command_id: Uuid,
    },
    /// A command taken after the export cannot become an envelope (an `actor_id`, idempotency key
    /// or payload the envelope refuses): it would be queued and never sendable, so arming refuses
    /// by name before anything is written. `dispatch` does not bound these; a seal does.
    #[error("the command {command_id} cannot be sealed as an envelope")]
    UnsealableCommand {
        /// The command.
        command_id: Uuid,
    },
    /// The service sequenced envelopes after the genesis (C5).
    #[error("the service's log head is {head}, not the genesis")]
    ServiceMovedPastGenesis {
        /// The head it reports.
        head: u64,
    },
    /// The service's snapshot is not the one this device published.
    #[error("the service does not hold the genesis snapshot this device published")]
    ServiceGenesisIsNotOurs,
    /// The service refused or could not answer.
    #[error("the service: {0}")]
    Service(#[from] ServiceError),
    /// The container could not be fetched, or is not the one this device sent.
    #[error("the genesis container could not be fetched: {0}")]
    Fetch(#[from] CopyError),
    /// The container is not a snapshot of this vault, or cannot be opened.
    #[error("the genesis container cannot be opened: {0}")]
    Open(SnapshotOpenFailure),
    /// The opened plaintext is not a valid snapshot.
    #[error("the genesis snapshot is invalid: {0}")]
    Snapshot(#[from] SnapshotError),
    /// The genesis could not be installed into the base's vault.
    #[error("the genesis could not be installed: {0}")]
    Bootstrap(#[from] BootstrapError),
    /// The base refused a gap command, or reproduced it as other than the log recorded: the gap
    /// cannot be reproduced and is never sent (C2).
    #[error("the replay of the command {command_id} did not reproduce the log")]
    ReplayDiverged {
        /// The first command that did not.
        command_id: Uuid,
    },
    /// The replay ended in a class-1 state other than the live vault's (C2).
    #[error("the replayed state is not the live vault's: table {table}")]
    StateDiverged {
        /// The first table that differs.
        table: String,
    },
    /// The log moved while the reconciliation ran: a command was taken after it was read.
    #[error("the operation log moved from {expected} to {found}")]
    LogMoved {
        /// The high-water mark the reconciliation read.
        expected: u64,
        /// The mark at the commit.
        found: u64,
    },
    /// The vault is not in a state to be armed (an outbox row, or a worker that is not healthy).
    #[error("the vault is not ready to be armed")]
    NotReady,
    /// The recorded genesis could not be read.
    #[error(transparent)]
    Genesis(#[from] GenesisStoreError),
    /// The scratch vault failed.
    #[error(transparent)]
    Rebase(#[from] RebaseError),
    /// The state of the two vaults could not be exported to be compared.
    #[error(transparent)]
    Export(#[from] ExportError),
    /// The database failed.
    #[error(transparent)]
    Db(#[from] DbError),
    /// The filesystem failed.
    #[error("the filesystem failed: {0}")]
    Io(#[from] io::Error),
}

impl From<rusqlite::Error> for GapError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Db(error.into())
    }
}

/// The commands of the gap: every `operation_log` row after `mark`, up to and including
/// `high_water`, in log order, each with its recorded issue time.
///
/// # Errors
/// [`GapError::MissingIssuedAt`] for a row with no issue time, [`GapError::UnreadableCommand`] for
/// a row with no payload this build can decode, or [`GapError::Db`].
pub fn read_gap(
    conn: &Connection,
    mark: u64,
    high_water: u64,
) -> Result<Vec<GapCommand>, GapError> {
    #[allow(clippy::type_complexity)]
    type Row = (
        i64,
        Uuid,
        Uuid,
        Option<Uuid>,
        String,
        String,
        String,
        Option<Vec<u8>>,
        Option<u32>,
        Option<Vec<u8>>,
        Option<i64>,
    );
    let (mark, high_water) = (
        i64::try_from(mark).map_err(|_| GapError::NotReady)?,
        i64::try_from(high_water).map_err(|_| GapError::NotReady)?,
    );
    let mut stmt = conn.prepare(
        "SELECT op_seq, command_id, correlation_id, causation_id, actor_type, actor_id,
                idempotency_key, payload, payload_schema_version, write_set_hash, issued_at
           FROM operation_log WHERE op_seq > ?1 AND op_seq <= ?2 ORDER BY op_seq",
    )?;
    let rows = stmt
        .query_map(params![mark, high_water], |r| {
            Ok::<Row, rusqlite::Error>((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
                r.get(8)?,
                r.get(9)?,
                r.get(10)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut out = Vec::with_capacity(rows.len());
    for (
        op_seq,
        command_id,
        correlation_id,
        causation_id,
        actor,
        actor_id,
        key,
        payload,
        version,
        hash,
        issued,
    ) in rows
    {
        // The issue time first: it is the refusal the ruling names, and the one a row from before
        // the column has.
        let Some(issued) = issued else {
            return Err(GapError::MissingIssuedAt { command_id });
        };
        let unreadable = || GapError::UnreadableCommand { command_id };
        let (Some(payload), Some(version), Some(hash)) = (payload, version, hash) else {
            return Err(unreadable());
        };
        let cmd = decode_payload(version, &payload).map_err(|_| unreadable())?;
        let write_set_hash: [u8; 32] = hash.try_into().map_err(|_| unreadable())?;
        let actor_type = match actor.as_str() {
            "user" => ActorType::User,
            "system" => ActorType::System,
            "importer" => ActorType::Importer,
            "agent" => ActorType::Agent,
            _ => return Err(unreadable()),
        };
        let meta = CommandMeta {
            command_id,
            correlation_id,
            causation_id,
            actor_type,
            actor_id,
            idempotency_key: key,
            issued_at: IssuedAt::from_unix_micros(issued),
        };
        // The command must be sendable: the same conditions a seal refuses (the device id is not
        // one of them).
        if command_envelope(&meta, write_set_hash, payload, Uuid::nil()).is_none() {
            return Err(GapError::UnsealableCommand { command_id });
        }
        out.push(GapCommand {
            meta,
            cmd,
            op_seq,
            write_set_hash,
        });
    }
    Ok(out)
}

/// Fetch the genesis container to `dest` (streamed, bounded, and checked against the digest this
/// device recorded when it sent it), open it with the epoch key and verify the snapshot inside.
///
/// The container is held in memory once to be opened (the opener authenticates the whole of it),
/// after it has already been bounded and verified on disk.
///
/// # Errors
/// [`GapError::Service`], [`GapError::Fetch`], [`GapError::Open`], [`GapError::Snapshot`] or
/// [`GapError::Io`].
pub fn fetch_genesis<S: SnapshotService>(
    service: &S,
    vault_id: &[u8; 16],
    snapshot_id: &[u8; 16],
    container_sha256: &[u8; 32],
    proof: &GenesisProof<'_>,
    dest: &Path,
) -> Result<(Verified, EngineVersion), GapError> {
    {
        let mut reader = service.get_snapshot(vault_id, snapshot_id)?;
        let mut file = File::create(dest)?;
        copy_snapshot_container(
            &mut reader,
            &mut file,
            MAX_GENESIS_CONTAINER_BYTES,
            Some(container_sha256),
        )?;
        file.sync_all()?;
    }
    let mut bytes = Vec::new();
    File::open(dest)?.read_to_end(&mut bytes)?;
    let parsed =
        parse_container(&bytes).map_err(|_| GapError::Open(SnapshotOpenFailure::NotASnapshot))?;
    let plaintext = SnapshotOpener::new(proof.epoch_key, proof.chain)
        .open(&parsed)
        .map_err(GapError::Open)?;
    let engine = EngineVersion::new(&parsed.header.engine_version)
        .map_err(|_| GapError::Open(SnapshotOpenFailure::NotASnapshot))?;
    Ok((verify(&plaintext)?, engine))
}

/// Install the verified genesis into a throwaway replica at `replica_path` and return the path of
/// the base image it captured: the genesis, with its secrets stripped, at position `0`.
///
/// # Errors
/// [`GapError::Bootstrap`].
pub fn build_genesis_image(
    replica_path: &Path,
    dek: &Arc<vault_crypto::Dek>,
    vault_id: [u8; 16],
    snapshot: &Verified,
    engine: &EngineVersion,
    own: &EngineVersion,
) -> Result<PathBuf, GapError> {
    let source = ReplicaSource {
        vault_id,
        root: 0,
        snapshot,
        engine_version: engine,
        own_engine_version: own,
        held_voids: &[],
    };
    let bootstrapped = bootstrap_replica(replica_path, dek, &source)?;
    drop(bootstrapped);
    Ok(base_image_path(replica_path))
}

/// Replay `gap` on a scratch copy of `image` and prove it reproduces the live vault (C2): each
/// command's write-set hash is the one the log recorded, and the final class-1 state equals the
/// live vault's, table by table. Returns each command's `write_set_keys` as the scratch outbox
/// recorded them, in log order.
///
/// # Errors
/// [`GapError::LogMoved`] if the live log is no longer at `expected_high_water`,
/// [`GapError::ReplayDiverged`] naming the first command that did not reproduce,
/// [`GapError::StateDiverged`] naming the first table that differs, or a scratch/database failure.
pub fn replay_and_prove(
    live: &DbWorker,
    image: &Path,
    scratch_path: &Path,
    gap: &[GapCommand],
    expected_high_water: u64,
) -> Result<Vec<Vec<u8>>, GapError> {
    replay_and_prove_with(
        live,
        image,
        scratch_path,
        gap,
        expected_high_water,
        &mut || {},
    )
}

/// [`replay_and_prove`], with `between` run after the live log's high-water mark is read and before
/// the live state is exported: the moment a command taken on the live vault would race the check.
/// For tests; [`replay_and_prove`] passes a no-op.
///
/// # Errors
/// As [`replay_and_prove`].
#[doc(hidden)]
pub fn replay_and_prove_with(
    live: &DbWorker,
    image: &Path,
    scratch_path: &Path,
    gap: &[GapCommand],
    expected_high_water: u64,
    between: &mut dyn FnMut(),
) -> Result<Vec<Vec<u8>>, GapError> {
    let scratch = live.scratch_from_image(image, 0, scratch_path)?;
    let worker = scratch.worker();
    let mut keys = Vec::with_capacity(gap.len());
    for command in gap {
        let step = StepContext {
            applied_base_seq: Some(0),
            rebased_from: None,
            record_outbox: true,
        };
        let diverged = || GapError::ReplayDiverged {
            command_id: command.meta.command_id,
        };
        let outcome = match worker.dispatch_rebase_step(command.meta.clone(), &command.cmd, step) {
            Ok(outcome) => outcome,
            // The base refused it, or it applied differently: the same refusal either way.
            Err(RebaseError::Db(error)) if crate::rebase::is_command_refusal(&error) => {
                return Err(diverged())
            }
            Err(other) => return Err(other.into()),
        };
        let Outcome::Applied { op_seq } = outcome else {
            return Err(diverged());
        };
        let conn = worker.read_connection()?;
        let (hash, issued): (Vec<u8>, Option<i64>) = conn.query_row(
            "SELECT write_set_hash, issued_at FROM operation_log WHERE op_seq = ?1",
            [op_seq],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if hash != command.write_set_hash || issued != Some(command.meta.issued_at.as_unix_micros())
        {
            return Err(diverged());
        }
        keys.push(conn.query_row(
            "SELECT write_set_keys FROM sync_outbox WHERE command_id = ?1",
            [command.meta.command_id],
            |r| r.get::<_, Vec<u8>>(0),
        )?);
    }
    // The end state: the log's effects are the vault's, class 1, table by table. The live export
    // reads the log's high-water mark in the SAME read transaction as the rows
    // (`export_genesis_with_mark`), so the state compared is exactly the state at that mark. A
    // command taken since the gap was read is the log having moved (retry), never a divergence of
    // the replay.
    let moved = |found: u64| GapError::LogMoved {
        expected: expected_high_water,
        found,
    };
    between();
    let (plaintext, found) =
        export_genesis_with_mark(&live.read_connection()?, &ExportInput::default())?;
    if found != expected_high_water {
        return Err(moved(found));
    }
    let theirs = verify(&plaintext)?.manifest.tables;
    let ours = class_one_entries(&worker.read_connection()?)?;
    let differing = theirs
        .iter()
        .zip(&ours)
        .find(|(a, b)| a != b)
        .map(|(a, _)| a.name.clone())
        .or_else(|| (theirs.len() != ours.len()).then(|| "(the set of tables)".to_owned()));
    // A log that moved while the states were compared is the log moving, whatever the compare saw.
    let after: i64 = live.read_connection()?.query_row(
        "SELECT COALESCE(MAX(op_seq), 0) FROM operation_log",
        [],
        |r| r.get(0),
    )?;
    let after = u64::try_from(after).unwrap_or(0);
    if after != expected_high_water {
        return Err(moved(after));
    }
    if let Some(table) = differing {
        return Err(GapError::StateDiverged { table });
    }
    Ok(keys)
}

/// The per-table entries of a vault's class-1 logical export: name, columns, row count, state
/// digest and file hash. Two vaults with the same class-1 state have the same entries.
fn class_one_entries(conn: &Connection) -> Result<Vec<sync_core::snapshot::TableEntry>, GapError> {
    let plaintext = export_genesis_plaintext(conn, &ExportInput::default())?;
    Ok(verify(&plaintext)?.manifest.tables)
}

/// Arm the vault in one transaction (C4): re-read the log's high-water mark, write the replayed
/// commands into the outbox in log order, put the genesis base image in place, and record the
/// `sync_base` and `sync_position` rows.
///
/// # Errors
/// [`GapError::LogMoved`], [`GapError::AlreadyArmed`], [`GapError::NotReady`] or a database or
/// filesystem failure; on any of them nothing is written to the live vault.
pub fn arm(
    live: &DbWorker,
    image: &Path,
    expected_high_water: u64,
    gap: &[GapCommand],
    write_set_keys: &[Vec<u8>],
) -> Result<Armed, GapError> {
    if gap.len() != write_set_keys.len() {
        return Err(GapError::NotReady);
    }
    // One capture at a time: nothing else may place a base while this does.
    let _one_base = live
        .base_gate
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = sync_dir(live.db_path());
    let final_path = base_image_path(live.db_path());
    let temp = base_image_temp_path(live.db_path());
    fs::create_dir_all(&dir)?;
    match fs::remove_file(&temp) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error.into()),
        _ => {}
    }
    // The image is copied before the writer is taken: it can be a large file.
    copy_file_synced(image, &temp)?;
    let digest = sha256_file(&temp)?;

    let result = (|| -> Result<Armed, GapError> {
        let guard = live.lock();
        if guard.state != WorkerState::Healthy {
            return Err(GapError::NotReady);
        }
        let conn = &guard.conn;
        let tx = conn.unchecked_transaction()?;
        let found: i64 = tx.query_row(
            "SELECT COALESCE(MAX(op_seq), 0) FROM operation_log",
            [],
            |r| r.get(0),
        )?;
        let found = u64::try_from(found).unwrap_or(0);
        if found != expected_high_water {
            return Err(GapError::LogMoved {
                expected: expected_high_water,
                found,
            });
        }
        let armed: i64 = tx.query_row(
            "SELECT (SELECT count(*) FROM sync_base) + (SELECT count(*) FROM sync_outbox)",
            [],
            |r| r.get(0),
        )?;
        if armed > 0 {
            return Err(GapError::AlreadyArmed);
        }
        let now = Utc::now().to_rfc3339();
        for (command, keys) in gap.iter().zip(write_set_keys) {
            tx.execute(
                "INSERT INTO sync_outbox
                   (command_id, op_seq, state, write_set_keys, created_at, issued_at)
                 VALUES (?1, ?2, 'Pending', ?3, ?4, ?5)",
                params![
                    command.meta.command_id,
                    command.op_seq,
                    keys,
                    now,
                    command.meta.issued_at.as_unix_micros()
                ],
            )?;
        }
        tx.execute(
            "INSERT INTO sync_base (singleton, base_seq, base_digest, state, updated_at)
             VALUES (1, 0, ?1, 'Ready', ?2)",
            params![digest.as_slice(), now],
        )?;
        tx.execute(
            "INSERT INTO sync_position (singleton, snapshot_root, next_to_apply)
             VALUES (1, NULL, 1)",
            [],
        )?;
        fs::rename(&temp, &final_path)?;
        fsync_dir(&dir)?;
        tx.commit()?;
        Ok(Armed {
            commands: gap.len(),
            base_seq: 0,
        })
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// Arm a founding device whose vault changed after its genesis was exported, or confirm it has no
/// gap and arm it on its genesis. See the module comment for the order; nothing is written to the
/// live vault until the last transaction.
///
/// The scratch vaults are keyed like the live one, so the vault must have been opened with its raw
/// key (a passphrase-keyed vault is [`DbError::KeyUnavailable`]).
///
/// # Errors
/// A [`GapError`]; every one leaves the live vault as it was.
pub fn reconcile_genesis_gap<S: SnapshotService>(
    worker: &DbWorker,
    service: &S,
    proof: &GenesisProof<'_>,
) -> Result<Armed, GapError> {
    reconcile_genesis_gap_with(worker, service, proof, &mut || {})
}

/// [`reconcile_genesis_gap`], with `between` run at the moment a command on the live vault would
/// race the end-state check (see [`replay_and_prove_with`]). For tests.
///
/// # Errors
/// As [`reconcile_genesis_gap`].
#[doc(hidden)]
pub fn reconcile_genesis_gap_with<S: SnapshotService>(
    worker: &DbWorker,
    service: &S,
    proof: &GenesisProof<'_>,
    between: &mut dyn FnMut(),
) -> Result<Armed, GapError> {
    let vault_id = *worker.vault_id()?.as_bytes();
    let dek = worker.raw_dek()?;
    // 1. The gates.
    let genesis = worker.write_sync_state(|tx| sync_genesis::load(tx))?;
    let genesis = match genesis {
        Some(genesis) if genesis.state == GenesisState::Published => genesis,
        other => return Err(GapError::GenesisNotPublished(other.map(|g| g.state))),
    };
    let (Some(snapshot_id), Some(container_sha256)) = (
        genesis.genesis_snapshot_id,
        genesis.genesis_container_sha256,
    ) else {
        return Err(GapError::GenesisIdentityNotRecorded);
    };
    let mark = genesis
        .export_high_water
        .ok_or(GenesisStoreError::Corrupt)?;
    let (high_water, gap) = {
        let conn = worker.read_connection()?;
        let armed: i64 = conn.query_row(
            "SELECT (SELECT count(*) FROM sync_base) + (SELECT count(*) FROM sync_outbox)",
            [],
            |r| r.get(0),
        )?;
        if armed > 0 {
            return Err(GapError::AlreadyArmed);
        }
        let high_water: i64 = conn.query_row(
            "SELECT COALESCE(MAX(op_seq), 0) FROM operation_log",
            [],
            |r| r.get(0),
        )?;
        let high_water = u64::try_from(high_water).unwrap_or(0);
        (high_water, read_gap(&conn, mark, high_water)?)
    };
    // C5: nothing was sequenced after the genesis, and the snapshot the service holds is ours.
    let head = service.log_head(&vault_id)?;
    if head != 0 {
        return Err(GapError::ServiceMovedPastGenesis { head });
    }
    match service.latest_snapshot(&vault_id)? {
        Some(held) if held.snapshot_id == snapshot_id && held.base_seq == 0 => {}
        _ => return Err(GapError::ServiceGenesisIsNotOurs),
    }

    // 2. The base is the fetched genesis. The work directory is only ever copies.
    let work = gap_dir(worker.db_path());
    if work.exists() {
        fs::remove_dir_all(&work)?;
    }
    fs::create_dir_all(&work)?;
    let outcome = (|| -> Result<Armed, GapError> {
        let (verified, engine) = fetch_genesis(
            service,
            &vault_id,
            &snapshot_id,
            &container_sha256,
            proof,
            &work.join("genesis.container"),
        )?;
        let image = build_genesis_image(
            &work.join("genesis.db"),
            &dek,
            vault_id,
            &verified,
            &engine,
            proof.own_engine_version,
        )?;
        // 3. Replay and prove.
        let keys = replay_and_prove_with(
            worker,
            &image,
            &work.join("replay.db"),
            &gap,
            high_water,
            between,
        )?;
        // 4. Arm.
        arm(worker, &image, high_water, &gap, &keys)
    })();
    // The work directory held copies only; it goes whether or not arming happened.
    let _ = fs::remove_dir_all(&work);
    outcome
}

/// Whether `result` is the refusal that says the log moved (a caller may re-run on it).
#[must_use]
pub const fn is_log_moved(error: &GapError) -> bool {
    matches!(error, GapError::LogMoved { .. })
}

/// A connection-less check used by tests: the `sync_position` row an arming wrote.
#[doc(hidden)]
pub fn armed_position(conn: &Connection) -> rusqlite::Result<Option<(Option<i64>, i64)>> {
    conn.query_row(
        "SELECT snapshot_root, next_to_apply FROM sync_position WHERE singleton = 1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
}
