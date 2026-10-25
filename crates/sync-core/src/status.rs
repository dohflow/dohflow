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

use crate::envelope::ReceiverError;
use crate::header::{ClearHeader, Fingerprint};
use crate::membership::ChainError;

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
    /// Which receiver check failed.
    pub reason: ReceiverError,
}

/// The replica's status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncStatus {
    /// Applying normally.
    Healthy,
    /// A fork or an equivocation was detected.
    Halted(HaltEvidence),
    /// An authenticated envelope at `seq` failed the receiver checks.
    EnvelopeInvalid(InvalidEnvelope),
}

/// Something the UI and the audit log should show, emitted once per change of status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncEvent {
    /// Sync halted on a fork or equivocation.
    Halted(HaltEvidence),
    /// Sync stopped at an authenticated-but-invalid envelope.
    EnvelopeInvalid(InvalidEnvelope),
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

    /// Record the outcome of the receiver checks on the authenticated envelope the service placed
    /// at `seq` (with its `canonical_envelope_digest`). A failure stops the replica at `seq`; if
    /// it is already stopped at a later position (envelopes can be verified out of order) the
    /// earlier one wins; seeing the same `(seq, digest)` again is idempotent; a **different**
    /// digest at a position already recorded is a fork of the log and halts. A halted replica
    /// stays halted.
    ///
    /// The "found invalid" fact needs no persistence: it is recomputable from the log by running
    /// the same deterministic checks on the same bytes, and a replica must evaluate a position
    /// itself before it considers any void naming it.
    pub fn observe_envelope(
        &mut self,
        seq: u64,
        envelope_digest: [u8; 32],
        header: &ClearHeader,
        outcome: Result<(), ReceiverError>,
    ) -> Option<SyncEvent> {
        if let Self::EnvelopeInvalid(existing) = self {
            if existing.seq == seq && existing.envelope_digest != envelope_digest {
                let evidence = HaltEvidence::SeqConflict {
                    seq,
                    first: existing.envelope_digest,
                    second: envelope_digest,
                };
                *self = Self::Halted(evidence);
                return Some(SyncEvent::Halted(evidence));
            }
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
            Self::EnvelopeInvalid(existing) if existing.seq <= seq => None,
            _ => {
                *self = Self::EnvelopeInvalid(invalid);
                Some(SyncEvent::EnvelopeInvalid(invalid))
            }
        }
    }
}
