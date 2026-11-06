//! The rebase engine core (personal-cfo-bxsbz.4; ADR 0074 decision 2, ADR 0073).
//!
//! A device with unpushed commands cannot apply the server's envelopes on top of them: the first
//! create-if-missing forks. A rebase therefore rolls back to the base image, applies the server's
//! envelopes there, and re-applies the local commands on that new base. It runs on a **scratch
//! vault**, a second [`DbWorker`] on a copy of `sync/base.db`; the live vault is never written. The
//! swap that installs a finished scratch vault is a later child of the bead.
//!
//! ```text
//! rollback_to(base_seq) -> Scratch
//! Scratch::apply_pulled(envelopes)   server envelopes in seq order, each write-set hash verified
//! plan(outbox, touched)              untouched? -> replay, otherwise queue the correlation group
//! Scratch::execute(plan)             replay the planned commands, queue the rest
//! ```
//!
//! **Pulled envelopes.** [`PulledEnvelope`] holds a [`CheckedEnvelope`]: its checks (`check_envelope`)
//! ran and passed on exactly the header and plaintext it carries, so an envelope that was never
//! checked cannot reach the scratch vault. Each is applied through the ordinary command path with
//! the origin's complete [`CommandMeta`], and the write-set hash the apply produced is compared
//! with the envelope's. A mismatch is a **fork**, not a conflict (ADR 0074 decision 3): the rebase
//! stops, the live vault is untouched, and [`ForkEvidence`] carries what the replica's
//! `SyncStatus` needs to show it.
//!
//! **Already-accepted commands.** An unpushed command the service already accepted (the response
//! was lost) must not be applied or queued again: [`reconcile`] matches every local outbox entry
//! with every pulled envelope and the probe results before anything is applied, binds the exact
//! matches to their accepted position, and halts on a `ProtocolFork`. The re-seal rule: only a
//! command whose evidence says the service knows neither of its identities is replayed, and it is
//! written back `Pending` with its new row keys and no container. There is no decrypting, sealing
//! or key material anywhere in the rebase.
//!
//! **Rolling back reaches the base and no further.** [`DbWorker::rollback_to`] accepts only the
//! recorded `base_seq`; any other target is a typed error, never a partial rollback.

mod execute;
mod plan;
mod reconcile;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use sync_core::envelope::{CheckedEnvelope, ImmutableMeta};
use sync_core::status::HaltEvidence;
use uuid::Uuid;

use crate::base_image::{base_image_path, copy_file_synced, BaseImageStatus};
use crate::payload::{decode_payload, PayloadError};
use crate::{
    apply_command, ActorType, CommandMeta, DbError, DbWorker, Fault, IssuedAt, KeyMaterial,
    Outcome, WorkerState,
};

/// How one command is applied during a rebase: it replaces what [`DbWorker::dispatch`] would read
/// from the vault, because the scratch vault is not a Sync-enabled live vault.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepContext {
    /// The server seq of the base this operation-log row is applied on: the seq before a server
    /// envelope, the new head for a re-applied local command.
    pub applied_base_seq: Option<i64>,
    /// The `op_seq` this row replaces when a local command is re-applied, otherwise `None`.
    pub rebased_from: Option<i64>,
    /// Whether the command is an unpushed local one: a re-applied command is recorded as a pending
    /// outbox entry on the rebuilt vault, a server envelope is not.
    pub record_outbox: bool,
}

/// Why a rebase stopped. The live vault is untouched by every one of these.
#[derive(Debug, thiserror::Error)]
pub enum RebaseError {
    /// [`DbWorker::rollback_to`] reaches `base_seq` and no further: any other target is refused.
    #[error("a rebase can only roll back to the base (seq {base_seq}), not to seq {requested}")]
    TargetNotTheBase {
        /// The seq the caller asked for.
        requested: i64,
        /// The seq of the base image.
        base_seq: i64,
    },
    /// There is no base to roll back to: the vault rebootstraps pull-first instead.
    #[error("there is no usable base image to roll back to")]
    NoUsableBase(BaseImageStatus),
    /// The scratch vault's path is taken.
    #[error("the scratch vault already exists")]
    ScratchExists,
    /// Pulled envelopes must arrive in seq order starting right after the base.
    #[error("expected the envelope at seq {expected}, got seq {got}")]
    OutOfOrder {
        /// The seq that had to come next.
        expected: u64,
        /// The seq that was offered.
        got: u64,
    },
    /// The envelope's metadata or payload cannot be applied by this build.
    #[error("the envelope at seq {seq} cannot be decoded")]
    Undecodable {
        /// The envelope's server seq.
        seq: u64,
        /// What was wrong with it.
        cause: Undecodable,
    },
    /// A server envelope was applied and produced another write-set hash than it carries.
    #[error("the envelope at seq {} forks from this replica", .0.seq)]
    Fork(ForkEvidence),
    /// A server envelope was refused by the command's own validation; the replica stops at its
    /// position (a deterministic apply failure, vault-format.md C.10).
    #[error("the envelope at seq {seq} was refused when applied")]
    PulledEnvelopeRefused {
        /// The envelope's server seq.
        seq: u64,
        /// The refusal.
        error: DbError,
    },
    /// The idempotency memo of a command being applied already exists in the scratch vault. The
    /// executor requires it to be absent; an already-accepted command is reconciled (bxsbz.5), not
    /// applied again.
    #[error("the command {command_id} already has an idempotency memo in the scratch vault")]
    MemoPresent {
        /// The command that would have been applied twice.
        command_id: Uuid,
    },
    /// The local commands were already rebased onto this scratch vault: replaying the same outbox
    /// twice would apply every command twice.
    #[error("the local commands were already rebased onto this scratch vault")]
    AlreadyRebased,
    /// A local command's metadata cannot be preserved as an envelope.
    #[error("the local command {command_id} cannot be preserved")]
    UnsealableLocalCommand {
        /// The command.
        command_id: Uuid,
    },
    /// An unpushed command has no recorded issue time (a row from before the outbox kept it): a
    /// replay under any other time would mint other ids, so the rebase refuses by name.
    #[error("the unpushed command {command_id} has no recorded issue time")]
    MissingIssuedAt {
        /// The command.
        command_id: Uuid,
    },
    /// An unpushed command's recorded row cannot be decoded by this build.
    #[error("the unpushed command {command_id} cannot be read back")]
    UnreadableLocalCommand {
        /// The command.
        command_id: Uuid,
    },
    /// One command identity is bound to two different envelopes (C.10 `ProtocolFork`). Both are
    /// kept in the evidence; the caller feeds it to the replica's `SyncStatus` and the vault stops.
    /// Nothing was applied to the scratch vault when this is returned from `reconcile`.
    #[error("the command {} is bound to two different envelopes", .0.command_id)]
    ProtocolFork(Box<ProtocolForkEvidence>),
    /// An `Approved` entry the tail does not settle has no probe result: whether the service
    /// accepted it is unknown, and the rebase will not guess (a replay of an accepted command is a
    /// fork, and so is a queue that is later kept).
    #[error("the sealed command {command_id} has no accepted-index probe result")]
    ProbeRequired {
        /// The command.
        command_id: Uuid,
    },
    /// The tail and the probe, or the outbox and the tail, disagree about one accepted command.
    /// That is a fork-consistency failure of the service, not a bind: nothing is bound.
    #[error("the evidence about the accepted command {command_id} disagrees")]
    EvidenceDisagrees {
        /// The command.
        command_id: Uuid,
        /// How.
        found: Disagreement,
    },
    /// The evidence says the service accepted the command at `seq`, but the scratch vault holds no
    /// such command: the pulled tail does not reach it, and the base does not contain it.
    #[error("the accepted command {command_id} (seq {seq}) is not in the scratch vault")]
    AcceptedButAbsent {
        /// The command.
        command_id: Uuid,
        /// The sequence number the evidence gives it.
        seq: u64,
    },
    /// The entry was handed to the replay without its reconciliation: a `Bound` entry is never
    /// replayed, and an `Approved` one only once the evidence said the service does not know it.
    #[error("the command {command_id} was not reconciled before the replay")]
    NotReconciled {
        /// The command.
        command_id: Uuid,
    },
    /// A pulled entry's container is not the envelope that was checked.
    #[error("the container at seq {seq} is not the checked envelope")]
    PulledContainerMismatch {
        /// The envelope's server seq.
        seq: u64,
    },
    /// An outbox entry's retained sealed container cannot be read back as that command's envelope.
    #[error("the retained envelope of the command {command_id} cannot be read")]
    UnreadableSealed {
        /// The command.
        command_id: Uuid,
    },
    /// The database or the filesystem failed.
    #[error(transparent)]
    Db(#[from] DbError),
}

impl From<rusqlite::Error> for RebaseError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Db(error.into())
    }
}

impl From<std::io::Error> for RebaseError {
    fn from(error: std::io::Error) -> Self {
        Self::Db(error.into())
    }
}

/// What made an envelope undecodable.
#[derive(Debug)]
pub enum Undecodable {
    /// The actor type byte is not one this build knows.
    ActorType(u8),
    /// The payload is not a command this build can decode.
    Payload(PayloadError),
}

/// The evidence of a fork: the replica's own apply of the envelope at `seq` produced `computed`,
/// not the `expected` hash the envelope carries. Nothing here is written to the live vault; the
/// caller feeds [`ForkEvidence::halt_evidence`] to the replica's `SyncStatus`, which shows it and
/// persists it under the anti-rollback witness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForkEvidence {
    /// The envelope's server seq.
    pub seq: u64,
    /// The write-set hash the envelope carries.
    pub expected: [u8; 32],
    /// The write-set hash this replica computed.
    pub computed: [u8; 32],
}

impl ForkEvidence {
    /// The halt `SyncStatus::observe_write_set_mismatch` records for this fork.
    #[must_use]
    pub const fn halt_evidence(&self) -> HaltEvidence {
        HaltEvidence::WriteSetMismatch {
            seq: self.seq,
            expected: self.expected,
            computed: self.computed,
        }
    }
}

/// A server envelope offered to a rebase: its position and the proof that `check_envelope` passed
/// on it.
#[derive(Debug, Clone, Copy)]
pub struct PulledEnvelope<'a> {
    /// The envelope's server seq.
    pub seq: u64,
    /// The checked header and plaintext.
    pub checked: CheckedEnvelope<'a>,
}

pub use execute::{Executed, LocalCommand, OutboxEntryState, Replayed};
pub use plan::{
    auto_iff, plan, Disposition, OutboxCommand, Plan, PlannedCommand, QueueCause, QueueReason,
};
pub use reconcile::{
    reconcile, reconcile_and_replay, AcceptedEvidence, Disagreement, ForkSource, Probe,
    ProtocolForkEvidence, PulledLogEntry, Reconciliation, RetainedEnvelope, Settled, SettledBy,
};

/// A table row a command wrote: the table name and its canonical primary key
/// (`WriteSet::rows`).
pub type RowKey = (String, Vec<u8>);

/// A scratch vault: a copy of the base image with the server's envelopes applied. It borrows the
/// live worker, so the key it uses cannot outlive the live vault's.
pub struct Scratch<'a> {
    live: &'a DbWorker,
    worker: DbWorker,
    path: PathBuf,
    base_seq: i64,
    head_seq: i64,
    touched: BTreeSet<RowKey>,
    /// The rows each applied server envelope wrote, by command: what a reconciled entry's outbox
    /// row is remapped to.
    pulled_rows: BTreeMap<Uuid, Vec<RowKey>>,
    /// The `Approved` entries whose evidence said the service does not know them, so that
    /// replaying them cannot duplicate an accepted command.
    cleared: BTreeSet<Uuid>,
    /// Whether the local commands were already rebased onto this scratch vault.
    rebased: bool,
}

impl std::fmt::Debug for Scratch<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scratch")
            .field("path", &self.path)
            .field("base_seq", &self.base_seq)
            .field("head_seq", &self.head_seq)
            .field("touched_rows", &self.touched.len())
            .finish_non_exhaustive()
    }
}

impl DbWorker {
    /// Roll back to the base image: copy `sync/base.db` to `scratch_path` and open the copy as a
    /// scratch vault under this vault's key. `target_seq` must be the recorded `base_seq`; the base
    /// image is the only restore point, so an earlier (or later) target is a typed error and
    /// nothing is created. The live vault is only read.
    ///
    /// # Errors
    /// [`RebaseError::NoUsableBase`], [`RebaseError::TargetNotTheBase`],
    /// [`RebaseError::ScratchExists`], or [`RebaseError::Db`].
    pub fn rollback_to(
        &self,
        target_seq: i64,
        scratch_path: &Path,
    ) -> Result<Scratch<'_>, RebaseError> {
        let base_seq = match self.base_image_status()? {
            BaseImageStatus::Ready { base_seq } => base_seq,
            other => return Err(RebaseError::NoUsableBase(other)),
        };
        if target_seq != base_seq {
            return Err(RebaseError::TargetNotTheBase {
                requested: target_seq,
                base_seq,
            });
        }
        if scratch_path.exists() {
            return Err(RebaseError::ScratchExists);
        }
        if let Some(dir) = scratch_path.parent() {
            fs::create_dir_all(dir)?;
        }
        if let Err(error) = copy_file_synced(&base_image_path(&self.path), scratch_path) {
            let _ = fs::remove_file(scratch_path);
            return Err(error.into());
        }
        let worker = match self.open_scratch(scratch_path) {
            Ok(worker) => worker,
            Err(error) => {
                let _ = fs::remove_file(scratch_path);
                return Err(error.into());
            }
        };
        Ok(Scratch {
            live: self,
            worker,
            path: scratch_path.to_path_buf(),
            base_seq,
            head_seq: base_seq,
            touched: BTreeSet::new(),
            pulled_rows: BTreeMap::new(),
            cleared: BTreeSet::new(),
            rebased: false,
        })
    }

    /// Open the image at `path` as a worker keyed like this one. It takes its own runner lock on
    /// `path` and reconciles nothing: it is a scratch copy, not a vault with a base of its own.
    fn open_scratch(&self, path: &Path) -> Result<DbWorker, DbError> {
        let runner_lock = crate::acquire_runner_lock(path)?;
        let (conn, key) = match &self.key {
            KeyMaterial::Raw(dek) => (
                crate::open_existing_keyed_raw(path, dek)?,
                KeyMaterial::Raw(std::sync::Arc::clone(dek)),
            ),
            KeyMaterial::Passphrase(passphrase) => (
                crate::open_existing_keyed(path, passphrase)?,
                KeyMaterial::Passphrase(passphrase.clone()),
            ),
        };
        DbWorker::bootstrap_scratch(path.to_path_buf(), conn, key, runner_lock)
    }

    /// Apply one command the way a rebase does: the memo must be absent, the step context
    /// replaces what the vault would say about Sync, and the command otherwise goes through the
    /// ordinary validated apply.
    pub(crate) fn dispatch_rebase_step(
        &self,
        meta: CommandMeta,
        cmd: &crate::WriteCommand,
        step: StepContext,
    ) -> Result<Outcome, RebaseError> {
        meta.validate()?;
        let node_id = self.node_id;
        let schema_version = self.schema_version;
        let mut guard = self.lock();
        if guard.state != WorkerState::Healthy {
            return Err(DbError::WorkerUnavailable(guard.state).into());
        }
        if crate::find_by_idempotency_key(&guard.conn, &meta.idempotency_key)?.is_some() {
            return Err(RebaseError::MemoPresent {
                command_id: meta.command_id,
            });
        }
        guard.rebase = Some(step);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            apply_command(
                &mut guard,
                &meta,
                cmd,
                node_id,
                schema_version,
                Fault::None,
                None,
            )
        }));
        guard.rebase = None;
        match result {
            Ok(outcome) => Ok(outcome?),
            Err(_panic) => {
                guard.state = WorkerState::CorruptNeedsRecovery;
                Err(DbError::WriterPanicked.into())
            }
        }
    }
}

impl Scratch<'_> {
    /// The seq of the base this scratch vault was rolled back to.
    #[must_use]
    pub const fn base_seq(&self) -> i64 {
        self.base_seq
    }

    /// The server seq of the last envelope applied (the base seq before any).
    #[must_use]
    pub const fn head_seq(&self) -> i64 {
        self.head_seq
    }

    /// The class-1 rows the server envelopes wrote so far: what an unpushed local command's
    /// recorded write set must not touch.
    #[must_use]
    pub const fn touched(&self) -> &BTreeSet<RowKey> {
        &self.touched
    }

    /// The scratch vault's worker, for reading its state.
    #[must_use]
    pub const fn worker(&self) -> &DbWorker {
        &self.worker
    }

    /// The scratch vault's path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Apply the server's envelopes in seq order, each verified against the write-set hash it
    /// carries. Stops at the first problem; the scratch vault is then abandoned by the caller and
    /// the live vault was never written.
    ///
    /// # Errors
    /// [`RebaseError::OutOfOrder`], [`RebaseError::Undecodable`],
    /// [`RebaseError::PulledEnvelopeRefused`], [`RebaseError::MemoPresent`], or
    /// [`RebaseError::Fork`] when a replica's write set differs from the envelope's.
    pub fn apply_pulled(&mut self, envelopes: &[PulledEnvelope<'_>]) -> Result<(), RebaseError> {
        self.apply_pulled_remapping(envelopes, &BTreeMap::new())
    }

    /// [`Self::apply_pulled`], where `own` names the envelopes that are this device's own accepted
    /// commands (command id to the `op_seq` it had locally). Such a row records the `op_seq` it
    /// replaces, and its rows do not count as touched by the server: the command is the device's
    /// own, so a later local command that builds on it must not be mistaken for a conflict.
    pub(crate) fn apply_pulled_remapping(
        &mut self,
        envelopes: &[PulledEnvelope<'_>],
        own: &BTreeMap<Uuid, i64>,
    ) -> Result<(), RebaseError> {
        for envelope in envelopes {
            let expected = u64::try_from(self.head_seq + 1).unwrap_or(u64::MAX);
            if envelope.seq != expected {
                return Err(RebaseError::OutOfOrder {
                    expected,
                    got: envelope.seq,
                });
            }
            let plaintext = envelope.checked.plaintext();
            let seq = envelope.seq;
            let undecodable = |cause| RebaseError::Undecodable { seq, cause };
            let meta = command_meta(&plaintext.meta).map_err(undecodable)?;
            let command = decode_payload(
                envelope.checked.header().payload_schema_version,
                &plaintext.payload,
            )
            .map_err(|error| undecodable(Undecodable::Payload(error)))?;
            let applied_on = self.head_seq;
            let command_id = meta.command_id;
            let step = StepContext {
                applied_base_seq: Some(applied_on),
                rebased_from: own.get(&command_id).copied(),
                record_outbox: false,
            };
            match self.worker.dispatch_rebase_step(meta, &command, step) {
                Ok(_) => {}
                Err(RebaseError::Db(error)) if is_command_refusal(&error) => {
                    return Err(RebaseError::PulledEnvelopeRefused { seq, error });
                }
                Err(other) => return Err(other),
            }
            let write_set = self.worker.last_write_set().ok_or(DbError::SelfTestFailed(
                "an applied command has no write set".to_owned(),
            ))?;
            if write_set.hash != plaintext.write_set_hash {
                return Err(RebaseError::Fork(ForkEvidence {
                    seq,
                    expected: plaintext.write_set_hash,
                    computed: write_set.hash,
                }));
            }
            if !own.contains_key(&command_id) {
                self.touched.extend(write_set.rows.iter().cloned());
            }
            self.pulled_rows.insert(command_id, write_set.rows);
            self.head_seq =
                i64::try_from(seq).map_err(|_| RebaseError::OutOfOrder { expected, got: seq })?;
        }
        Ok(())
    }

    /// The live worker this scratch vault was rolled back from.
    #[must_use]
    pub const fn live(&self) -> &DbWorker {
        self.live
    }
}

/// The original [`CommandMeta`] of a command, rebuilt from its envelope metadata.
fn command_meta(meta: &ImmutableMeta) -> Result<CommandMeta, Undecodable> {
    let actor_type = match meta.actor_type {
        1 => ActorType::User,
        2 => ActorType::System,
        3 => ActorType::Importer,
        4 => ActorType::Agent,
        other => return Err(Undecodable::ActorType(other)),
    };
    Ok(CommandMeta {
        command_id: Uuid::from_bytes(meta.command_id),
        correlation_id: Uuid::from_bytes(meta.correlation_id),
        causation_id: meta.causation_id.map(Uuid::from_bytes),
        actor_type,
        actor_id: meta.actor_id.clone(),
        idempotency_key: canonical_key_text(&meta.idempotency_key),
        issued_at: IssuedAt::from_unix_micros(meta.issued_at),
    })
}

/// Whether `error` is the command's own refusal (its validation said no) rather than a failure of
/// the database or the filesystem. A refusal on the new base queues a local command and stops a
/// replica at a server envelope; an infrastructure failure aborts the rebase, because retrying
/// may succeed and nothing may be queued on its account.
///
/// Every [`DbError`] variant is listed, so a new one cannot be filed on the wrong side silently.
#[must_use]
pub fn is_command_refusal(error: &DbError) -> bool {
    // The alias keeps this exhaustive match out of the rejection-site ledger: it classifies errors,
    // it raises none.
    use DbError as E;
    match error {
        E::MissingMetadata(_)
        | E::InvalidIdempotencyKey
        | E::InvalidCommand(_)
        | E::Rejected { .. }
        | E::LoanAlreadyLinked { .. }
        | E::AttachmentNotFound
        | E::ReplicatedKeyTooLong { .. }
        | E::SettingScope => true,
        E::CipherUnavailable
        | E::CheckpointBusy
        | E::VaultInUse
        | E::NewerSchema { .. }
        | E::UnsupportedSchema
        | E::WorkerUnavailable(_)
        | E::WriterPanicked
        | E::SelfTestFailed(_)
        | E::Sqlite(_)
        | E::Io(_)
        | E::Crypto(_)
        | E::KeyUnavailable
        | E::QueuedForDisposition { .. } => false,
    }
}

/// Whether a rebase queued the command `command_id` for the person's decision.
pub(crate) fn is_queued(
    conn: &rusqlite::Connection,
    command_id: &Uuid,
) -> Result<bool, rusqlite::Error> {
    conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM outbox_queue WHERE command_id = ?1)",
        rusqlite::params![command_id],
        |r| r.get(0),
    )
}

/// Roll `live` back to its base image as a scratch vault at `scratch_path` (see
/// [`DbWorker::rollback_to`]). Public for the Sync simulator, which drives the rollback and the
/// replay between interleavings.
///
/// # Errors
/// As [`DbWorker::rollback_to`].
pub fn rollback_to<'a>(
    live: &'a DbWorker,
    target_seq: i64,
    scratch_path: &Path,
) -> Result<Scratch<'a>, RebaseError> {
    live.rollback_to(target_seq, scratch_path)
}

/// Apply the server's `envelopes` to `scratch`, then re-apply or queue the unpushed `local`
/// commands: one rebase, start to finish, on the scratch vault. Public for the Sync simulator.
///
/// # Errors
/// As [`Scratch::apply_pulled`] and [`Scratch::rebase_local`].
pub fn replay(
    scratch: &mut Scratch<'_>,
    envelopes: &[PulledEnvelope<'_>],
    local: &[LocalCommand],
) -> Result<Executed, RebaseError> {
    scratch.apply_pulled(envelopes)?;
    scratch.rebase_local(local)
}

/// The key's canonical text, the one form there is (C.5): lowercase, hyphenated, 36 characters.
/// It is a received key being written back out, never a minted one.
fn canonical_key_text(key: &sync_core::envelope::IdempotencyKey) -> String {
    Uuid::from_bytes(*key.as_bytes()).hyphenated().to_string()
}
