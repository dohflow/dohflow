//! Already-accepted reconciliation and the re-seal rule (personal-cfo-bxsbz.5; ADR 0074 decision 8,
//! `docs/architecture/vault-format.md` C.10).
//!
//! A command the service accepted whose response was lost is still in this device's outbox. The
//! rebase must neither apply it a second time nor queue it for a decision that, once kept, would
//! re-seal an accepted command (a `ProtocolFork`). So **before anything is applied to the scratch
//! vault**, [`reconcile`] matches every outbox entry with every pulled envelope and with the
//! accepted-index probe results the client engine (elciw) obtained by re-pushing the retained
//! container byte for byte.
//!
//! ```text
//! reconcile(pulled, outbox, evidence)   pure: nothing is applied, nothing is written
//!   -> Err(ProtocolFork)                both envelopes kept; the caller halts the vault's sync
//!   -> Ok(Reconciliation)               which entries are already accepted, which are replayable
//! Scratch::apply_reconciled(..)         the pulled tail, with the device's own commands remapped
//! Scratch::rebase_local(unsettled)      replay and queue what the service does not know
//! ```
//!
//! **What counts as the same envelope.** Equal `canonical_envelope_digest` (sync-core's
//! `match_pulled`: command id, tag, header and ciphertext), and the pulled plaintext equals the
//! outbox command's complete [`CommandMeta`], payload and payload schema version. A digest match
//! whose plaintext disagrees with the local record is a fork too: the retained bytes and the
//! local row cannot both be that command.
//!
//! **The evidence** per entry, in the order it is read:
//!
//! * the pulled tail (authenticated and checked: [`PulledLogEntry`] holds a `CheckedEnvelope`);
//! * for an `Approved` entry the tail does not settle, the [`Probe`] the engine got. `Replay`
//!   binds, `ProtocolFork` halts, `Unknown` means the service does not know either identity and
//!   the entry may be re-sealed; no probe is an error, never a guess;
//! * a `Pending` entry was never approved, so it is unknown by construction and needs no probe.
//!
//! **The re-seal rule.** An auto-replayed entry is written back as `Pending` (its container
//! dropped, its row keys and write-set hash those of the new base) only when its evidence said
//! "unknown". The engine seals it at push. A test proves an accepted command is never written back
//! as `Pending`.
//!
//! **Not decided here.** Whether a bound position later appears with the same digest in the tail
//! the client pulls, and persisting a halt under the anti-rollback witness, belong to the client
//! engine (elciw). `sync_base.state` is never used for halts.

use std::collections::BTreeMap;

use chrono::Utc;
use rusqlite::params;
use sync_core::accepted::{
    match_pulled, OutboxEntry, PullMatch, PulledEnvelope as AcceptedPulled, PushDecision,
};
use sync_core::envelope::{canonical_envelope_digest, EnvelopePlaintext};
use sync_core::header::{parse_container, ClearHeader};
use sync_core::status::{HaltEvidence, SyncEvent, SyncStatus};
use uuid::Uuid;

use super::{
    canonical_key_text, command_meta, LocalCommand, OutboxEntryState, PulledEnvelope, RebaseError,
    RowKey, Scratch,
};
use crate::sync_state::encode_row_keys;
use crate::DbError;

/// What re-pushing a retained container byte for byte told the client engine about the service's
/// accepted-envelope index (C.10, `decide_push`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Probe {
    /// Both identities are already accepted as this exact envelope.
    Replay {
        /// The recorded sequence number.
        seq: u64,
        /// The recorded result reference.
        result_ref: Option<Vec<u8>>,
    },
    /// An identity is bound to a different command, tag, envelope or metadata.
    ProtocolFork,
    /// The service knows neither identity.
    Unknown,
}

impl Probe {
    /// The probe result for the service's push decision.
    #[must_use]
    pub fn from_decision(decision: &PushDecision<'_>) -> Self {
        match decision {
            PushDecision::Replay(record) => Self::Replay {
                seq: record.seq,
                result_ref: record.result_ref.clone(),
            },
            PushDecision::ProtocolFork => Self::ProtocolFork,
            PushDecision::Unknown => Self::Unknown,
        }
    }
}

/// The probe results for the outbox's sealed entries, by command id. The pulled tail is the other
/// half of the accepted evidence and arrives as [`PulledLogEntry`] values.
#[derive(Debug, Clone, Default)]
pub struct AcceptedEvidence {
    probes: BTreeMap<Uuid, Probe>,
}

impl AcceptedEvidence {
    /// No probe results.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the probe result for the entry `command_id`.
    pub fn record(&mut self, command_id: Uuid, probe: Probe) -> &mut Self {
        self.probes.insert(command_id, probe);
        self
    }

    /// The probe result for `command_id`, if one was obtained.
    #[must_use]
    pub fn probe(&self, command_id: &Uuid) -> Option<&Probe> {
        self.probes.get(command_id)
    }
}

/// A server envelope offered to a reconciliation: the checked envelope with its position (the
/// rebase applies it), the container bytes it came in (for the digest, and kept as diagnostics on
/// a fork), and the service's result reference for it.
///
/// It holds a [`CheckedEnvelope`](sync_core::envelope::CheckedEnvelope), which only
/// `check_envelope` can build, so the tag and commitment checks have run before anything is
/// reconciled or applied:
///
/// ```compile_fail
/// use sync_core::envelope::{CheckedEnvelope, EnvelopePlaintext};
/// use sync_core::header::ClearHeader;
/// fn forge<'a>(header: &'a ClearHeader, plaintext: &'a EnvelopePlaintext) {
///     // The fields are private: there is no way to claim a check that did not run.
///     let _ = db_worker::rebase::PulledEnvelope {
///         seq: 1,
///         checked: CheckedEnvelope { header, plaintext },
///     };
/// }
/// ```
#[derive(Debug, Clone)]
pub struct PulledLogEntry<'a> {
    /// The checked envelope and its server seq.
    pub envelope: PulledEnvelope<'a>,
    /// The exact container bytes the service returned.
    pub container: &'a [u8],
    /// The service's result reference for it.
    pub result_ref: Option<Vec<u8>>,
}

/// One retained envelope, kept whole for diagnosis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetainedEnvelope {
    /// Its `canonical_envelope_digest`.
    pub digest: [u8; 32],
    /// The container bytes, ciphertext included.
    pub container: Vec<u8>,
    /// The server seq it was seen at, if it came from the log.
    pub seq: Option<u64>,
}

/// Where a fork was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForkSource {
    /// An envelope pulled from the log.
    Tail,
    /// The accepted-envelope index refused the retained container as a fork.
    Index,
}

/// The evidence of a `ProtocolFork`: one command identity claimed by two envelopes. Both are kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolForkEvidence {
    /// The command whose identity is claimed twice.
    pub command_id: Uuid,
    /// This device's retained envelope; `None` for a `Pending` entry, which was never sealed.
    pub local: Option<RetainedEnvelope>,
    /// The other envelope; `None` when the index refused ours without naming it.
    pub other: Option<RetainedEnvelope>,
    /// Where the fork was found.
    pub source: ForkSource,
}

impl ProtocolForkEvidence {
    /// The halt the replica's `SyncStatus` records: the command and both digests.
    #[must_use]
    pub fn halt_evidence(&self) -> HaltEvidence {
        HaltEvidence::ProtocolFork {
            command_id: *self.command_id.as_bytes(),
            local_digest: self.local.as_ref().map(|e| e.digest),
            other_digest: self.other.as_ref().map(|e| e.digest),
        }
    }

    /// Feed the fork to `status`. Persisting the halt is the engine's, under the anti-rollback
    /// witness.
    pub fn observe(&self, status: &mut SyncStatus) -> Option<SyncEvent> {
        status.observe_protocol_fork(
            *self.command_id.as_bytes(),
            self.local.as_ref().map(|e| e.digest),
            self.other.as_ref().map(|e| e.digest),
        )
    }
}

/// How the evidence about one accepted command contradicts itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Disagreement {
    /// Two sources put the command at different positions, or give it different results.
    Position {
        /// The position (and result) the outbox or the probe gives.
        recorded: (u64, Option<Vec<u8>>),
        /// The position (and result) the pulled tail gives.
        pulled: (u64, Option<Vec<u8>>),
    },
    /// The tail holds the exact envelope, and the probe says the service does not know it.
    ProbeUnknownButPulled,
    /// The tail holds the exact envelope, and the probe says the service called it a fork.
    ProbeForkButPulled,
}

/// What settled an already-accepted entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettledBy {
    /// The exact envelope is in the pulled tail.
    Tail,
    /// The accepted-envelope index said `Replay`; the envelope is not in the tail (it was
    /// truncated, or a snapshot bootstrap carries its effects).
    Index,
    /// The outbox already held the entry `Bound`.
    Bound,
}

/// An outbox entry the service already accepted: bound to its position, and neither replayed nor
/// queued.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settled {
    /// The command.
    pub command_id: Uuid,
    /// Its `op_seq` in the live vault before the rebase.
    pub old_op_seq: i64,
    /// The sequence number the service assigned.
    pub seq: u64,
    /// The service's result reference.
    pub result_ref: Option<Vec<u8>>,
    /// The retained sealed container, kept on the bound entry.
    pub sealed: Vec<u8>,
    /// The class-1 rows the command wrote locally, kept when the scratch vault has no better.
    pub write_set_keys: Vec<RowKey>,
    /// What settled it.
    pub by: SettledBy,
}

/// The result of reconciling the outbox with the accepted evidence.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reconciliation {
    /// The entries already accepted, in local order.
    pub settled: Vec<Settled>,
    /// The `Approved` entries whose probe said the service knows neither identity: they may be
    /// re-sealed after the replay.
    pub unknown: Vec<Uuid>,
}

impl Reconciliation {
    /// The entries still to rebase: everything the service does not know, in local order. The
    /// accepted ones are bound, not replayed.
    #[must_use]
    pub fn unsettled(&self, local: &[LocalCommand]) -> Vec<LocalCommand> {
        local
            .iter()
            .filter(|c| {
                !self
                    .settled
                    .iter()
                    .any(|s| s.command_id == c.meta.command_id)
            })
            .cloned()
            .collect()
    }

    fn remap(&self) -> BTreeMap<Uuid, i64> {
        self.settled
            .iter()
            .filter(|s| s.by == SettledBy::Tail)
            .map(|s| (s.command_id, s.old_op_seq))
            .collect()
    }
}

/// A pulled entry with its identity derived from its bytes.
struct View<'a> {
    entry: &'a PulledLogEntry<'a>,
    identity: AcceptedPulled,
    digest: [u8; 32],
    seq: u64,
    command_id: [u8; 16],
}

impl<'a> View<'a> {
    fn new(entry: &'a PulledLogEntry<'a>) -> Result<Self, RebaseError> {
        let seq = entry.envelope.seq;
        let mismatch = || RebaseError::PulledContainerMismatch { seq };
        let container = parse_container(entry.container).map_err(|_| mismatch())?;
        let identity = AcceptedPulled::from_container(
            &container,
            &entry.envelope.checked,
            seq,
            entry.result_ref.clone(),
        )
        .map_err(|_| mismatch())?;
        let digest = canonical_envelope_digest(container.header_bytes, container.ciphertext)
            .map_err(|_| mismatch())?;
        Ok(Self {
            entry,
            identity,
            digest,
            seq,
            command_id: entry.envelope.checked.plaintext().meta.command_id,
        })
    }

    fn retained(&self) -> RetainedEnvelope {
        RetainedEnvelope {
            digest: self.digest,
            container: self.entry.container.to_vec(),
            seq: Some(self.seq),
        }
    }
}

/// The outbox entry's retained container, read back as that command's envelope.
struct Seal {
    identity: OutboxEntry,
    digest: [u8; 32],
    bytes: Vec<u8>,
}

impl Seal {
    fn read(command: &LocalCommand) -> Result<Option<Self>, RebaseError> {
        let Some(bytes) = command.entry.sealed() else {
            return Ok(None);
        };
        let unreadable = || RebaseError::UnreadableSealed {
            command_id: command.meta.command_id,
        };
        let container = parse_container(bytes).map_err(|_| unreadable())?;
        if container.header.object_id != *command.meta.command_id.as_bytes() {
            return Err(unreadable());
        }
        let identity = OutboxEntry::from_container(&container).map_err(|_| unreadable())?;
        let digest = canonical_envelope_digest(container.header_bytes, container.ciphertext)
            .map_err(|_| unreadable())?;
        Ok(Some(Self {
            identity,
            digest,
            bytes: bytes.to_vec(),
        }))
    }

    fn retained(&self) -> RetainedEnvelope {
        RetainedEnvelope {
            digest: self.digest,
            container: self.bytes.clone(),
            seq: None,
        }
    }
}

/// How one pulled envelope relates to one outbox entry.
enum Relation {
    NotMine,
    Bind,
    Fork,
}

/// Whether the pulled plaintext is the outbox command's: the complete [`CommandMeta`], the payload
/// and the payload schema version.
fn same_command(local: &LocalCommand, header: &ClearHeader, plaintext: &EnvelopePlaintext) -> bool {
    let Ok(pulled) = command_meta(&plaintext.meta) else {
        return false;
    };
    let mine = &local.meta;
    pulled.command_id == mine.command_id
        && pulled.correlation_id == mine.correlation_id
        && pulled.causation_id == mine.causation_id
        && pulled.actor_type == mine.actor_type
        && pulled.actor_id == mine.actor_id
        && pulled.idempotency_key == mine.idempotency_key
        && pulled.issued_at == mine.issued_at
        && plaintext.payload == local.payload
        && header.payload_schema_version == local.payload_version
}

fn relate(local: &LocalCommand, seal: Option<&Seal>, view: &View<'_>) -> Relation {
    let plaintext = view.entry.envelope.checked.plaintext();
    match seal {
        Some(seal) => match match_pulled(&seal.identity, &view.identity) {
            PullMatch::NotMine => Relation::NotMine,
            PullMatch::ProtocolFork => Relation::Fork,
            PullMatch::BindToAccepted { .. } => {
                if same_command(local, view.entry.envelope.checked.header(), plaintext) {
                    Relation::Bind
                } else {
                    Relation::Fork
                }
            }
        },
        // Never sealed: no digest to compare. The same command id or the same idempotency key in
        // the log is another envelope for this identity.
        None => {
            let same_id = view.command_id == *local.meta.command_id.as_bytes();
            let same_key =
                canonical_key_text(&plaintext.meta.idempotency_key) == local.meta.idempotency_key;
            if same_id || same_key {
                Relation::Fork
            } else {
                Relation::NotMine
            }
        }
    }
}

fn fork(
    local: &LocalCommand,
    seal: Option<&Seal>,
    other: Option<&View<'_>>,
    source: ForkSource,
) -> RebaseError {
    RebaseError::ProtocolFork(Box::new(ProtocolForkEvidence {
        command_id: local.meta.command_id,
        local: seal.map(Seal::retained),
        other: other.map(View::retained),
        source,
    }))
}

/// Reconcile the outbox with the accepted evidence, before anything is applied.
///
/// `pulled` is every envelope the rebase will apply, checked; `local` is the whole outbox in local
/// order. Every entry is matched with every pulled envelope.
///
/// # Errors
/// [`RebaseError::ProtocolFork`] with both envelopes kept; [`RebaseError::ProbeRequired`] for an
/// `Approved` entry nothing settles; [`RebaseError::EvidenceDisagrees`];
/// [`RebaseError::PulledContainerMismatch`]; [`RebaseError::UnreadableSealed`].
pub fn reconcile(
    pulled: &[PulledLogEntry<'_>],
    local: &[LocalCommand],
    evidence: &AcceptedEvidence,
) -> Result<Reconciliation, RebaseError> {
    let views = pulled
        .iter()
        .map(View::new)
        .collect::<Result<Vec<_>, _>>()?;
    let mut out = Reconciliation::default();
    for command in local {
        let command_id = command.meta.command_id;
        let seal = Seal::read(command)?;
        let mut binds: Vec<&View<'_>> = Vec::new();
        for view in &views {
            match relate(command, seal.as_ref(), view) {
                Relation::NotMine => {}
                Relation::Bind => binds.push(view),
                Relation::Fork => {
                    return Err(fork(command, seal.as_ref(), Some(view), ForkSource::Tail))
                }
            }
        }
        // The service holds each command once: the same envelope twice in the log is a fork.
        if let [_, second, ..] = binds.as_slice() {
            return Err(fork(command, seal.as_ref(), Some(second), ForkSource::Tail));
        }
        let probe = evidence.probe(&command_id);
        let settled = |seq: u64, result_ref: Option<Vec<u8>>, by: SettledBy| -> Settled {
            Settled {
                command_id,
                old_op_seq: command.old_op_seq,
                seq,
                result_ref,
                sealed: seal.as_ref().map(|s| s.bytes.clone()).unwrap_or_default(),
                write_set_keys: command.command.write_set_keys.clone(),
                by,
            }
        };
        let pulled_at = |view: &View<'_>| (view.seq, view.entry.result_ref.clone());
        let disagrees = |found| RebaseError::EvidenceDisagrees { command_id, found };
        match (&command.entry, binds.first()) {
            (OutboxEntryState::Pending, _) => {}
            (
                OutboxEntryState::Bound {
                    seq, result_ref, ..
                },
                Some(view),
            ) => {
                if pulled_at(view) != (*seq, result_ref.clone()) {
                    return Err(disagrees(Disagreement::Position {
                        recorded: (*seq, result_ref.clone()),
                        pulled: pulled_at(view),
                    }));
                }
                out.settled
                    .push(settled(*seq, result_ref.clone(), SettledBy::Tail));
            }
            (
                OutboxEntryState::Bound {
                    seq, result_ref, ..
                },
                None,
            ) => {
                out.settled
                    .push(settled(*seq, result_ref.clone(), SettledBy::Bound));
            }
            (OutboxEntryState::Approved { .. }, Some(view)) => {
                match probe {
                    None => {}
                    Some(Probe::Replay { seq, result_ref })
                        if (*seq, result_ref.clone()) == pulled_at(view) => {}
                    Some(Probe::Replay { seq, result_ref }) => {
                        return Err(disagrees(Disagreement::Position {
                            recorded: (*seq, result_ref.clone()),
                            pulled: pulled_at(view),
                        }));
                    }
                    Some(Probe::Unknown) => {
                        return Err(disagrees(Disagreement::ProbeUnknownButPulled));
                    }
                    Some(Probe::ProtocolFork) => {
                        return Err(disagrees(Disagreement::ProbeForkButPulled));
                    }
                }
                out.settled.push(settled(
                    view.seq,
                    view.entry.result_ref.clone(),
                    SettledBy::Tail,
                ));
            }
            (OutboxEntryState::Approved { .. }, None) => match probe {
                None => return Err(RebaseError::ProbeRequired { command_id }),
                Some(Probe::Replay { seq, result_ref }) => {
                    out.settled
                        .push(settled(*seq, result_ref.clone(), SettledBy::Index));
                }
                Some(Probe::ProtocolFork) => {
                    return Err(fork(command, seal.as_ref(), None, ForkSource::Index));
                }
                Some(Probe::Unknown) => out.unknown.push(command_id),
            },
        }
    }
    Ok(out)
}

/// The `sync_outbox.result_ref` column for a result: a `Bound` row requires it non-null, so an
/// absent result is stored as the empty blob. The service's result reference is never empty
/// (`AcceptedRecord` refuses that), so the two readings cannot be confused.
pub(super) fn result_to_column(result_ref: &Option<Vec<u8>>) -> Vec<u8> {
    result_ref.clone().unwrap_or_default()
}

/// The inverse of [`result_to_column`].
pub(super) fn result_from_column(column: Vec<u8>) -> Option<Vec<u8>> {
    (!column.is_empty()).then_some(column)
}

impl Scratch<'_> {
    /// Apply the server's envelopes with the device's own accepted commands recognized, bind
    /// those, and clear the entries whose evidence says the service does not know them for the
    /// replay.
    ///
    /// * An envelope that is one of the device's own commands (`reconciliation.settled`, by the
    ///   tail) is applied like any other, records the `op_seq` it replaces, and does not count as
    ///   touched by the server: a later local command that builds on it is not a conflict.
    /// * Each settled entry becomes a `Bound` outbox row in the scratch vault, keeping its sealed
    ///   container and carrying the accepted position and result. It is never written `Pending`.
    ///
    /// # Errors
    /// As [`Scratch::apply_pulled`], and [`RebaseError::AcceptedButAbsent`] when the evidence puts
    /// a command in the log that the scratch vault does not hold.
    pub fn apply_reconciled(
        &mut self,
        envelopes: &[PulledEnvelope<'_>],
        reconciliation: &Reconciliation,
    ) -> Result<(), RebaseError> {
        self.apply_pulled_remapping(envelopes, &reconciliation.remap())?;
        self.bind_settled(&reconciliation.settled)?;
        self.cleared.extend(reconciliation.unknown.iter().copied());
        Ok(())
    }

    fn bind_settled(&mut self, settled: &[Settled]) -> Result<(), RebaseError> {
        let guard = self.worker.lock();
        let tx = guard.conn.unchecked_transaction()?;
        let now = Utc::now().to_rfc3339();
        for entry in settled {
            let op_seq: Option<i64> = tx
                .query_row(
                    "SELECT op_seq FROM operation_log WHERE command_id = ?1",
                    params![entry.command_id],
                    |r| r.get(0),
                )
                .map(Some)
                .or_else(|error| match error {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })?;
            let Some(op_seq) = op_seq else {
                return Err(RebaseError::AcceptedButAbsent {
                    command_id: entry.command_id,
                    seq: entry.seq,
                });
            };
            let keys = self
                .pulled_rows
                .get(&entry.command_id)
                .unwrap_or(&entry.write_set_keys);
            let seq = i64::try_from(entry.seq).map_err(|_| {
                RebaseError::Db(DbError::SelfTestFailed(
                    "an accepted position does not fit".to_owned(),
                ))
            })?;
            tx.execute(
                "INSERT INTO sync_outbox
                   (command_id, op_seq, state, write_set_keys, sealed_container, bound_seq,
                    result_ref, created_at)
                 VALUES (?1, ?2, 'Bound', ?3, ?4, ?5, ?6, ?7)",
                params![
                    entry.command_id,
                    op_seq,
                    encode_row_keys(keys),
                    entry.sealed,
                    seq,
                    result_to_column(&entry.result_ref),
                    now,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

/// Reconcile, apply the server's envelopes, and rebase what the service does not know: one rebase
/// start to finish on the scratch vault, with the accepted evidence. Public for the Sync
/// simulator.
///
/// # Errors
/// As [`reconcile`], [`Scratch::apply_reconciled`] and [`Scratch::rebase_local`]. A
/// [`RebaseError::ProtocolFork`] is returned before the scratch vault is written.
pub fn reconcile_and_replay(
    scratch: &mut Scratch<'_>,
    pulled: &[PulledLogEntry<'_>],
    local: &[LocalCommand],
    evidence: &AcceptedEvidence,
) -> Result<(Reconciliation, super::Executed), RebaseError> {
    let reconciliation = reconcile(pulled, local, evidence)?;
    let envelopes: Vec<PulledEnvelope<'_>> = pulled.iter().map(|p| p.envelope).collect();
    scratch.apply_reconciled(&envelopes, &reconciliation)?;
    let executed = scratch.rebase_local(&reconciliation.unsettled(local))?;
    Ok((reconciliation, executed))
}
