//! The replica's sync status (`docs/architecture/vault-format.md` C.6 "Detection and recovery",
//! C.11; personal-cfo-g3m.17): a pure state machine that says whether the replica may apply the
//! next log position, and why not.
//!
//! Three different things stop a replica, and they are different on purpose:
//!
//! * **Halted**: a fork or an equivocation was found (both records are retained, keys from the
//!   forked branch are quarantined). The recovery is re-enrollment or a re-bootstrap.
//! * **EnvelopeInvalid**: an *authenticated* envelope at log position `N` fails the receiver
//!   checks. The service cannot detect this (it cannot decrypt), so every honest replica reaches
//!   it and stops at `N`: nothing at or past `N` is applied. It is not a fork, and it is not
//!   transient: retrying changes nothing. It needs a defined recovery (see the ADR 0074
//!   addendum); until then the status is sticky.
//! * **UpdateRequired** (R40): an envelope whose `engine_version` this build does not equal. The
//!   replica never evaluates or applies it (versions are opaque and equality-only), so it stops
//!   at that position without having judged it. It is not a verdict on the envelope. The exits
//!   are a build whose `engine_version` equals the envelope's, or a re-bootstrap from a snapshot
//!   past the position (ADR 0074 Addendum E).
//! * Retryable conditions (a coupled epoch record not yet delivered, a transient network error)
//!   are not in this machine at all: they change nothing about what may be applied.
//!
//! The outbox is not part of this state, so it is preserved by construction.
//!
//! **Persistence.** `EnvelopeInvalid` can be recomputed: a restored device re-pulls the position,
//! recomputes the same digest and runs the same deterministic checks. `Halted` evidence cannot:
//! the service may stop showing the conflicting record, so `Halted` MUST be persisted under the
//! same anti-rollback witness as the rest of the verified state.
//!
//! **What this machine cannot see.** It keeps no digest for positions that applied cleanly, so a
//! different digest at an already-applied position (the service showing two histories) is not
//! visible here. The engine's applied-log check owns that and should feed the same
//! [`HaltEvidence::SeqConflict`], so the halt class stays complete.

use std::num::NonZeroU16;

use crate::cbe::WireError;
use crate::envelope::ReceiverError;
use crate::header::{ClearHeader, Fingerprint};
use crate::membership::ChainError;
use crate::void::{ForkSuspected, HonoredVoid, VoidLedger};

/// What a fork or an equivocation looked like (both ids, retained for diagnosis).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HaltEvidence {
    /// Two membership records for one version.
    Fork {
        /// The version.
        version: u64,
        /// The record already accepted.
        accepted: [u8; 32],
        /// The conflicting, otherwise valid record.
        conflicting: [u8; 32],
    },
    /// The log position holds a different envelope than the one first seen there: the same
    /// position with a different digest is a fork of the log, not a re-observation.
    SeqConflict {
        /// The log position.
        seq: u64,
        /// The digest first seen at that position.
        first: [u8; 32],
        /// The different digest seen later.
        second: [u8; 32],
    },
    /// The replica applied an envelope and its write-set hash is not the one the envelope
    /// carries. That is a fork of the apply semantics (ADR 0074), recovered through a
    /// re-bootstrap. It is NEVER an apply failure and a void never papers over it.
    WriteSetMismatch {
        /// The log position.
        seq: u64,
        /// The write-set hash the envelope carries.
        expected: [u8; 32],
        /// The write-set hash this replica computed.
        computed: [u8; 32],
    },
    /// A position the replica says it applied is now found invalid (and no honored void skips
    /// it): the cursor was overstated or determinism broke. Either way the replica cannot trust
    /// its own state.
    AppliedPositionInvalid {
        /// The log position.
        seq: u64,
        /// The digest of the envelope found invalid.
        envelope_digest: [u8; 32],
    },
    /// A void already honored at one position was offered at another: the log forked.
    VoidAtTwoPositions {
        /// The void's id.
        void_id: [u8; 32],
        /// The position it was honored at.
        honored_at: u64,
        /// The position it is offered at now.
        offered_at: u64,
    },
    /// Two epoch records after one `prev_epoch_id`.
    Equivocation {
        /// The record already accepted.
        accepted: [u8; 32],
        /// The conflicting, otherwise valid record.
        conflicting: [u8; 32],
    },
}

/// An authenticated envelope that failed the receiver checks, and where.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidEnvelope {
    /// The log position the service assigned.
    pub seq: u64,
    /// The `canonical_envelope_digest` of the envelope at that position: its identity. A replica
    /// that restarts re-pulls `seq`, recomputes the same digest, and lands in the same state.
    pub envelope_digest: [u8; 32],
    /// The command's id (the header's `object_id`).
    pub command_id: [u8; 16],
    /// The device that sealed it, for the audit trail and the member chain.
    pub device_id: [u8; 16],
    /// The fingerprint of the key that sealed it.
    pub device_key_fingerprint: Fingerprint,
    /// Why the replica stopped here.
    pub reason: StopReason,
}

/// Why a replica stopped at an authenticated envelope. Audit only: honoring a void never depends
/// on the reason, only on the stop's position, digest and command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// The envelope failed a receiver check (C.4).
    ReceiverCheck(ReceiverError),
    /// The envelope passed them and then failed to apply for a deterministic reason (C.10).
    Apply(ApplyFailureCode),
}

/// A stable code for a deterministic apply rejection. The registry of codes is C.10's
/// classification; the code is audit-only here (never consulted by honoring), so a new code
/// never changes consensus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApplyFailureCode(NonZeroU16);

impl ApplyFailureCode {
    /// A code; `0` is not a code.
    #[must_use]
    pub const fn new(code: u16) -> Option<Self> {
        match NonZeroU16::new(code) {
            Some(code) => Some(Self(code)),
            None => None,
        }
    }

    /// The numeric code.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0.get()
    }
}

/// An `engine_version`: opaque printable ASCII (`0x21..=0x7e`), 1 to 64 bytes, compared byte for
/// byte and never ordered (C.13).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct EngineVersion {
    len: u8,
    bytes: [u8; 64],
}

impl std::fmt::Debug for EngineVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "EngineVersion({:?})", self.as_str())
    }
}

impl EngineVersion {
    /// Parse a value; anything but 1 to 64 printable ASCII bytes is refused.
    ///
    /// # Errors
    /// [`WireError::BadEngineVersion`].
    pub fn new(value: &str) -> Result<Self, WireError> {
        let raw = value.as_bytes();
        if raw.is_empty() || raw.len() > 64 || !raw.iter().all(|b| (0x21..=0x7e).contains(b)) {
            return Err(WireError::BadEngineVersion);
        }
        let mut bytes = [0_u8; 64];
        bytes[..raw.len()].copy_from_slice(raw);
        Ok(Self {
            len: u8::try_from(raw.len()).map_err(|_| WireError::BadEngineVersion)?,
            bytes,
        })
    }

    /// The value as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        // The constructor admits only printable ASCII.
        std::str::from_utf8(&self.bytes[..usize::from(self.len)]).unwrap_or("")
    }
}

/// The header's `engine_version` as a value. A decoded header always holds a valid one (decode
/// refuses anything else); a hand-built header with a bad value is treated as a foreign version.
fn header_engine_version(header: &ClearHeader) -> EngineVersion {
    EngineVersion::new(&header.engine_version)
        .unwrap_or_else(|_| EngineVersion::new("unparseable").expect("a valid literal"))
}

/// An envelope at `seq` carries an `engine_version` this build does not equal (R40).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpdateRequired {
    /// The log position.
    pub seq: u64,
    /// The `canonical_envelope_digest` of the envelope there.
    pub envelope_digest: [u8; 32],
    /// The `engine_version` the envelope carries.
    pub envelope_engine_version: EngineVersion,
    /// This build's `engine_version`.
    pub own_engine_version: EngineVersion,
}

/// What the engine verified about a snapshot it is about to install (its provenance: membership,
/// epoch, signature, is the engine's existing snapshot verification and is not repeated here).
#[derive(Debug, Clone, Copy)]
pub struct SnapshotFacts<'a> {
    /// The snapshot's root: the last log position it covers.
    pub root: u64,
    /// The snapshot's `engine_version`.
    pub engine_version: &'a EngineVersion,
    /// The honored voids the snapshot records.
    pub recorded_voids: &'a [HonoredVoid],
    /// The ranges `(seq, V - 1)` of every verified void this replica holds, honored or not. A
    /// snapshot rooted inside any of them was built on poisoned state and is never a root.
    pub held_voids: &'a [(u64, u64)],
}

/// Proof that a snapshot is a valid way out of the replica's CURRENT stop. Only
/// [`SyncStatus::rebootstrap_permit`] can build one, and [`SyncStatus::observe_rebootstrapped`]
/// accepts nothing else, so a stop cannot be cleared with an unverified snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RebootstrapPermit {
    root: u64,
    stop_seq: u64,
}

/// The replica's status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncStatus {
    /// Applying normally.
    Healthy,
    /// A fork or an equivocation was detected.
    Halted(HaltEvidence),
    /// An authenticated envelope at `seq` failed the receiver checks, or failed to apply for a
    /// deterministic reason.
    EnvelopeInvalid(InvalidEnvelope),
    /// An envelope at `seq` carries an `engine_version` this build does not equal: nothing at or
    /// past `seq` is evaluated or applied (R40).
    UpdateRequired(UpdateRequired),
}

/// Something the UI and the audit log should show, emitted once per change of status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncEvent {
    /// Sync halted on a fork or equivocation.
    Halted(HaltEvidence),
    /// Sync stopped at an authenticated-but-invalid envelope.
    EnvelopeInvalid(InvalidEnvelope),
    /// Sync stopped at an envelope of another `engine_version` (R40): update, or re-bootstrap.
    UpdateRequired(UpdateRequired),
    /// A snapshot past the stop was installed; the replica applies again from after `root`.
    Rebootstrapped {
        /// The snapshot's root position.
        root: u64,
    },
    /// An honored void skipped the invalid position; the replica applies again from the position
    /// after `after` (the last skipped position).
    Resumed {
        /// The last skipped log position.
        after: u64,
    },
}

impl SyncStatus {
    /// Whether the replica may apply log position `seq`: always when healthy; never when halted;
    /// only positions **before** the invalid one when stopped at an invalid envelope.
    #[must_use]
    pub const fn can_apply(&self, seq: u64) -> bool {
        match self {
            Self::Healthy => true,
            Self::Halted(_) => false,
            Self::EnvelopeInvalid(invalid) => seq < invalid.seq,
            Self::UpdateRequired(update) => seq < update.seq,
        }
    }

    /// Whether the replica may push: only when healthy. A replica stopped at an invalid envelope
    /// cannot push either: any push needs a base at or past the invalid position, and nothing
    /// there applies. Nothing is built on an invalid position; only a void or a re-bootstrap from
    /// a snapshot past it moves on.
    #[must_use]
    pub const fn can_push(&self) -> bool {
        matches!(self, Self::Healthy)
    }

    /// Whether the replica is healthy.
    #[must_use]
    pub const fn is_healthy(&self) -> bool {
        matches!(self, Self::Healthy)
    }

    /// Record a membership or epoch verification failure. Only the halt class changes the status;
    /// anything else (including a retryable missing epoch record) is not this machine's concern.
    /// The halt class **takes precedence** over a stop at an invalid envelope (a fork found while
    /// stopped must still quarantine keys, flag exposure and start re-enrollment); only a status
    /// that is already `Halted` absorbs it.
    pub fn observe_chain_error(&mut self, error: &ChainError) -> Option<SyncEvent> {
        if matches!(self, Self::Halted(_)) {
            return None;
        }
        let evidence = match *error {
            ChainError::Fork {
                version,
                accepted,
                conflicting,
            } => HaltEvidence::Fork {
                version,
                accepted,
                conflicting,
            },
            ChainError::EpochEquivocation {
                accepted,
                conflicting,
            } => HaltEvidence::Equivocation {
                accepted,
                conflicting,
            },
            _ => return None,
        };
        *self = Self::Halted(evidence);
        Some(SyncEvent::Halted(evidence))
    }

    /// Record that a void was **honored** (see [`crate::void`]): the replica is stopped at exactly
    /// the void's `seq` (the honor rule requires it), so the stop clears and the replica applies
    /// again from after the range. A stop anywhere else stays (a void for another position does
    /// not move it), and a halted replica stays halted: a void is never an answer to a fork.
    pub fn observe_honored(&mut self, honored: &HonoredVoid) -> Option<SyncEvent> {
        match self {
            Self::EnvelopeInvalid(invalid) if invalid.seq == honored.first => {
                *self = Self::Healthy;
                Some(SyncEvent::Resumed {
                    after: honored.last,
                })
            }
            _ => None,
        }
    }

    /// Halt on a suspected fork reported by the void honor rule (ADR 0074 Addendum E): a void
    /// naming another digest than the one held at the stop is the `SeqConflict` class; the same
    /// void honored at two positions is its own evidence. Halting is the rule, so every engine
    /// gets it from here instead of hand-rolling it. A status that is already `Halted` absorbs it.
    pub fn observe_fork_suspected(&mut self, fork: &ForkSuspected) -> Option<SyncEvent> {
        if matches!(self, Self::Halted(_)) {
            return None;
        }
        let evidence = match *fork {
            ForkSuspected::DigestAtStop { seq, ours, theirs } => HaltEvidence::SeqConflict {
                seq,
                first: ours,
                second: theirs,
            },
            ForkSuspected::VoidAtTwoPositions {
                void_id,
                honored_at,
                offered_at,
            } => HaltEvidence::VoidAtTwoPositions {
                void_id,
                honored_at,
                offered_at,
            },
        };
        *self = Self::Halted(evidence);
        Some(SyncEvent::Halted(evidence))
    }

    /// [`Self::observe_envelope`] for an engine that also holds the void ledger and its cursor:
    /// a position an honored void skips is not evaluated again, and neither is a **valid** result
    /// for one below `next_to_apply` (already applied). Re-verifying such a position (after a
    /// restart, a re-pull, or a verifier running ahead) must not re-open a stop the replica has
    /// already moved past; the "found invalid" fact is recomputable only for the positions at or
    /// after the frontier that no honored void skips. A **failed** result for a position below
    /// the frontier that no void skips contradicts the replica's own state (an overstated cursor
    /// or broken determinism): it halts, with [`HaltEvidence::AppliedPositionInvalid`].
    #[allow(clippy::too_many_arguments)] // the position, its envelope, the engine's ledger and cursor
    pub fn observe_envelope_in_context(
        &mut self,
        ledger: &VoidLedger,
        next_to_apply: u64,
        seq: u64,
        envelope_digest: [u8; 32],
        header: &ClearHeader,
        own_engine_version: &EngineVersion,
        outcome: Result<(), ReceiverError>,
    ) -> Option<SyncEvent> {
        if ledger.skips(seq) {
            return None;
        }
        if seq < next_to_apply {
            if outcome.is_ok() || matches!(self, Self::Halted(_)) {
                return None;
            }
            let evidence = HaltEvidence::AppliedPositionInvalid {
                seq,
                envelope_digest,
            };
            *self = Self::Halted(evidence);
            return Some(SyncEvent::Halted(evidence));
        }
        self.observe_envelope(seq, envelope_digest, header, own_engine_version, outcome)
    }

    /// Record the outcome of the receiver checks on the authenticated envelope the service placed
    /// at `seq` (with its `canonical_envelope_digest`). A failure stops the replica at `seq`; if
    /// it is already stopped at a later position (envelopes can be verified out of order) the
    /// earlier one wins; seeing the same `(seq, digest)` again is idempotent; a **different**
    /// digest at a position already recorded is a fork of the log and halts. A halted replica
    /// stays halted.
    ///
    /// The "found invalid" fact needs no persistence: it is recomputable from the log by running
    /// the same deterministic checks on the same bytes, and a replica must evaluate a position
    /// itself before it considers any void naming it. An engine that keeps a void ledger MUST call
    /// [`Self::observe_envelope_in_context`] instead, so a position already skipped or applied is
    /// not evaluated again.
    pub fn observe_envelope(
        &mut self,
        seq: u64,
        envelope_digest: [u8; 32],
        header: &ClearHeader,
        own_engine_version: &EngineVersion,
        outcome: Result<(), ReceiverError>,
    ) -> Option<SyncEvent> {
        self.observe_stop(
            seq,
            envelope_digest,
            header,
            own_engine_version,
            outcome.map_err(StopReason::ReceiverCheck),
        )
    }

    /// Record that applying the envelope at `seq` failed for a **deterministic** reason (C.10's
    /// classification; transient errors never reach here). The same rules as
    /// [`Self::observe_envelope`]: the earlier position wins, the same `(seq, digest)` is
    /// idempotent, another digest at the position halts. An apply stop is never found out of
    /// order (applying `seq` needs `seq - 1` applied), so a receiver-check stop at a later
    /// position simply loses to it.
    pub fn observe_apply_failure(
        &mut self,
        seq: u64,
        envelope_digest: [u8; 32],
        header: &ClearHeader,
        own_engine_version: &EngineVersion,
        outcome: Result<(), ApplyFailureCode>,
    ) -> Option<SyncEvent> {
        self.observe_stop(
            seq,
            envelope_digest,
            header,
            own_engine_version,
            outcome.map_err(StopReason::Apply),
        )
    }

    /// The version gate (C.4 stage 1c, R40) is a precondition of every verdict, so it lives here
    /// and cannot be skipped: an envelope whose `engine_version` is not this build's is routed to
    /// [`Self::observe_engine_version`] whatever the outcome says, because an outcome formed at a
    /// foreign version is not a verdict.
    fn observe_stop(
        &mut self,
        seq: u64,
        envelope_digest: [u8; 32],
        header: &ClearHeader,
        own_engine_version: &EngineVersion,
        outcome: Result<(), StopReason>,
    ) -> Option<SyncEvent> {
        let envelope_version = header_engine_version(header);
        if envelope_version != *own_engine_version {
            return self.observe_engine_version(
                seq,
                envelope_digest,
                envelope_version,
                *own_engine_version,
            );
        }
        if let Some(event) = self.same_position_other_digest(seq, envelope_digest) {
            return Some(event);
        }
        let Err(reason) = outcome else {
            return None;
        };
        let invalid = InvalidEnvelope {
            seq,
            envelope_digest,
            command_id: header.object_id,
            device_id: header.device_id,
            device_key_fingerprint: header.device_key_fingerprint,
            reason,
        };
        match self {
            Self::Halted(_) => None,
            Self::EnvelopeInvalid(InvalidEnvelope { seq: existing, .. })
            | Self::UpdateRequired(UpdateRequired { seq: existing, .. })
                if *existing <= seq =>
            {
                None
            }
            _ => {
                *self = Self::EnvelopeInvalid(invalid);
                Some(SyncEvent::EnvelopeInvalid(invalid))
            }
        }
    }

    /// A different digest at a position already recorded as a stop is a fork of the log.
    fn same_position_other_digest(
        &mut self,
        seq: u64,
        envelope_digest: [u8; 32],
    ) -> Option<SyncEvent> {
        let (recorded_seq, recorded_digest) = match self {
            Self::EnvelopeInvalid(i) => (i.seq, i.envelope_digest),
            Self::UpdateRequired(u) => (u.seq, u.envelope_digest),
            _ => return None,
        };
        if recorded_seq != seq || recorded_digest == envelope_digest {
            return None;
        }
        let evidence = HaltEvidence::SeqConflict {
            seq,
            first: recorded_digest,
            second: envelope_digest,
        };
        *self = Self::Halted(evidence);
        Some(SyncEvent::Halted(evidence))
    }

    /// Record the `engine_version` of the envelope the service placed at `seq`. One this build
    /// does not equal (versions are opaque; equality only) puts the replica in the R40
    /// update-required state at `seq`: the envelope is neither evaluated nor applied, and no
    /// verdict on it is formed. The earlier position wins and the same `(seq, digest)` is
    /// idempotent; another digest at the position halts; a halted replica stays halted. An equal
    /// version changes nothing.
    pub fn observe_engine_version(
        &mut self,
        seq: u64,
        envelope_digest: [u8; 32],
        envelope_engine_version: EngineVersion,
        own_engine_version: EngineVersion,
    ) -> Option<SyncEvent> {
        if let Some(event) = self.same_position_other_digest(seq, envelope_digest) {
            return Some(event);
        }
        if envelope_engine_version == own_engine_version {
            return None;
        }
        let update = UpdateRequired {
            seq,
            envelope_digest,
            envelope_engine_version,
            own_engine_version,
        };
        match self {
            Self::Halted(_) => None,
            // At the SAME position (same digest: another digest was a fork above) the version
            // gate beats any verdict: one formed at a foreign version is not a verdict.
            Self::EnvelopeInvalid(InvalidEnvelope { seq: existing, .. }) if *existing == seq => {
                *self = Self::UpdateRequired(update);
                Some(SyncEvent::UpdateRequired(update))
            }
            Self::EnvelopeInvalid(InvalidEnvelope { seq: existing, .. })
            | Self::UpdateRequired(UpdateRequired { seq: existing, .. })
                if *existing <= seq =>
            {
                None
            }
            _ => {
                *self = Self::UpdateRequired(update);
                Some(SyncEvent::UpdateRequired(update))
            }
        }
    }

    /// The build now running has `own_engine_version`: an update-required stop whose envelope
    /// carries exactly that version ends (the replica evaluates the position normally). It is
    /// recomputed on every start, so nothing is persisted. Any other status is unchanged.
    pub fn observe_own_engine_version(&mut self, own_engine_version: EngineVersion) {
        if matches!(self, Self::UpdateRequired(u) if u.envelope_engine_version == own_engine_version)
        {
            *self = Self::Healthy;
        }
    }

    /// Halt on a successful apply whose write-set hash is not the envelope's: a fork of the apply
    /// semantics (ADR 0074), never an apply failure and never voidable. A status that is already
    /// `Halted` absorbs it.
    pub fn observe_write_set_mismatch(
        &mut self,
        seq: u64,
        expected: [u8; 32],
        computed: [u8; 32],
    ) -> Option<SyncEvent> {
        if matches!(self, Self::Halted(_)) || expected == computed {
            return None;
        }
        let evidence = HaltEvidence::WriteSetMismatch {
            seq,
            expected,
            computed,
        };
        *self = Self::Halted(evidence);
        Some(SyncEvent::Halted(evidence))
    }

    /// Whether `snapshot` is a valid way out of the current stop, and if so the permit that
    /// [`Self::observe_rebootstrapped`] requires:
    ///
    /// * a stop at an invalid envelope or an apply failure: the snapshot records an honored void
    ///   whose range covers the stop and is rooted past it (never inside a voided range);
    /// * an update-required stop: the snapshot covers the position, is of THIS build's
    ///   `engine_version` (any other would be update-required itself), and is not rooted inside
    ///   any void range this replica holds (it cannot judge those voids, so it is conservative);
    /// * a fork of the apply semantics (a write-set mismatch, or an applied position found
    ///   invalid): a snapshot rooted at or past the position (ADR 0074: the recovery is a
    ///   re-bootstrap);
    /// * no other halt: a fork, an equivocation, a position conflict or a void at two positions
    ///   is key quarantine and re-enrollment, never a snapshot.
    #[must_use]
    pub fn rebootstrap_permit(&self, snapshot: &SnapshotFacts<'_>) -> Option<RebootstrapPermit> {
        let (stop_seq, ok) = match self {
            Self::EnvelopeInvalid(stop) => (
                stop.seq,
                crate::void::snapshot_passes_stop(stop.seq, snapshot.root, snapshot.recorded_voids),
            ),
            Self::UpdateRequired(update) => (
                update.seq,
                snapshot.root >= update.seq
                    && *snapshot.engine_version == update.own_engine_version
                    && !snapshot
                        .held_voids
                        .iter()
                        .any(|(first, last)| *first <= snapshot.root && snapshot.root <= *last),
            ),
            Self::Halted(
                HaltEvidence::WriteSetMismatch { seq, .. }
                | HaltEvidence::AppliedPositionInvalid { seq, .. },
            ) => (*seq, snapshot.root >= *seq),
            _ => return None,
        };
        ok.then_some(RebootstrapPermit {
            root: snapshot.root,
            stop_seq,
        })
    }

    /// Install the snapshot a [`RebootstrapPermit`] vouches for: the stop it was issued for ends.
    /// A permit for another stop (the status moved on since) changes nothing.
    pub fn observe_rebootstrapped(&mut self, permit: RebootstrapPermit) -> Option<SyncEvent> {
        let current = match self {
            Self::EnvelopeInvalid(i) => i.seq,
            Self::UpdateRequired(u) => u.seq,
            Self::Halted(
                HaltEvidence::WriteSetMismatch { seq, .. }
                | HaltEvidence::AppliedPositionInvalid { seq, .. },
            ) => *seq,
            _ => return None,
        };
        if current != permit.stop_seq {
            return None;
        }
        *self = Self::Healthy;
        Some(SyncEvent::Rebootstrapped { root: permit.root })
    }
}
