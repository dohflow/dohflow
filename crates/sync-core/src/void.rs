//! The void record and the rule by which a replica honors it (ADR 0074 Addendum D, personal-cfo-
//! g3m.24; `docs/architecture/vault-format.md` C.10 "Void records"; personal-cfo-g3m.17 part C).
//!
//! Every honest replica stops at a log position whose envelope authenticates but is invalid, and a
//! snapshot cannot pass it. A member-signed **void** lets the vault move on, with two properties:
//!
//! * the signature is **not authority over the outcome**: a replica skips a position only if it
//!   has itself evaluated that position, found it invalid, and sees the same `command_id` and
//!   envelope digest in the void (so a malicious member cannot void a valid envelope, and a void
//!   for one the replica cannot yet judge is inert); and
//! * coverage comes only from **honored** voids: an inert void covers nothing, so an inert void
//!   over a valid range never blocks a later genuine void inside it.
//!
//! This module is pure format and rule code. The replica's own verdict on a position comes from the
//! pipeline in `vault-crypto` (`classify_envelope`), which reports only its class here
//! ([`Evaluated`]); the service's checks and the client engine's rollback and re-issue are
//! `personal-cfo-v98vd` and `personal-cfo-elciw`.

use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::cbe::{Reader, WireError, Writer};
use crate::header::{Fingerprint, PROTOCOL_VERSION};
use crate::keys::ed25519_verify_strict;
use crate::membership::{EpochAtPosition, SignedRecord, VerifiedState, SUITE_ID};
use crate::status::SyncStatus;

/// The label of a void body.
pub const VOID_LABEL: &str = "dohflow/sync-void/v1";
/// The longest void body: the label, the fixed fields and nothing else (well under this).
pub const MAX_VOID_BODY: usize = 256;

/// Why the voider says the position is poisoned. Audit only. Only reason `1` exists in this
/// version: reason `2` (an envelope that passes the receiver checks but cannot be applied
/// deterministically) is version-scoped and is added with its `engine_version` body field by
/// `personal-cfo-g3m.26`. Until then a producer must not emit it and a replica refuses it as a
/// malformed object, so no reason-2 body lives in a log as a legacy form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoidReason {
    /// `1`: the envelope fails the receiver checks.
    ReceiverCheck,
}

/// A void record's body (C.10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoidBody {
    /// The suite (must be [`SUITE_ID`]).
    pub suite_id: u16,
    /// The vault.
    pub vault_id: [u8; 16],
    /// The poisoned log position.
    pub seq: u64,
    /// The `command_id` of the envelope the signer saw there.
    pub command_id: [u8; 16],
    /// The `canonical_envelope_digest` of that envelope.
    pub envelope_digest: [u8; 32],
    /// Why the signer says it is poisoned (audit only).
    pub reason: VoidReason,
    /// The signing device's fingerprint.
    pub signer_fingerprint: Fingerprint,
    /// Microseconds since the Unix epoch. Audit only: never compared with a clock.
    pub issued_at: i64,
}

impl VoidBody {
    /// The body's canonical bytes.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::with_label(VOID_LABEL);
        w.u16(PROTOCOL_VERSION)
            .u16(self.suite_id)
            .fixed(&self.vault_id)
            .u64(self.seq)
            .fixed(&self.command_id)
            .fixed(&self.envelope_digest)
            .u8(match self.reason {
                VoidReason::ReceiverCheck => 1,
            })
            .fixed(self.signer_fingerprint.as_bytes())
            .i64(self.issued_at);
        w.finish()
    }

    /// Parse a body from exactly `bytes`. The protocol version is read first; an unknown reason
    /// byte is a malformed object.
    ///
    /// # Errors
    /// A [`WireError`].
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut r = Reader::new(bytes);
        r.label(VOID_LABEL)?;
        if r.u16()? != PROTOCOL_VERSION {
            return Err(WireError::UnsupportedProtocol);
        }
        let suite_id = r.u16()?;
        let vault_id = r.fixed::<16>()?;
        let seq = r.u64()?;
        let command_id = r.fixed::<16>()?;
        let envelope_digest = r.fixed::<32>()?;
        let reason = match r.u8()? {
            1 => VoidReason::ReceiverCheck,
            _ => return Err(WireError::UnknownObjectType),
        };
        let signer_fingerprint = Fingerprint::from_derived(r.fixed::<32>()?);
        let issued_at = r.i64()?;
        r.finish()?;
        Ok(Self {
            suite_id,
            vault_id,
            seq,
            command_id,
            envelope_digest,
            reason,
            signer_fingerprint,
            issued_at,
        })
    }

    /// The void's id, `SHA-256(body)`.
    #[must_use]
    pub fn id(&self) -> [u8; 32] {
        Sha256::digest(self.encode()).into()
    }
}

/// Why a void record was refused. None of these puts a replica in the halt class: a void is an
/// ordinary log object, and a bad one is ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum VoidError {
    /// The record or its body is not well formed.
    #[error("the void record is malformed: {0}")]
    Malformed(WireError),
    /// A protocol version this build does not support.
    #[error("unsupported protocol version")]
    UnsupportedProtocol,
    /// A suite other than suite 1.
    #[error("unsupported suite")]
    UnsupportedSuite,
    /// The void names another vault.
    #[error("the void names another vault")]
    WrongVault,
    /// The signer is not an authorized member at the membership version in force at the void's
    /// position.
    #[error("the signer is not an authorized member at the void's position")]
    SignerNotAuthorized,
    /// The signature does not verify strictly under the chain's key for the signer.
    #[error("bad signature")]
    BadSignature,
}

/// A void record whose signature and signer were verified against the chain, **bound to the log
/// position it was pulled at** (`V`). There is exactly one `V`: the same value built the epoch
/// position (`at_void`) that judged the signer and bounds the range the void may skip, so the two
/// cannot disagree. `V` comes only from the replica's own in-order pull, never from an index-replay
/// "void claimed by" response (those claims never feed the honor rule).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedVoid {
    id: [u8; 32],
    position: u64,
    body: VoidBody,
}

impl VerifiedVoid {
    /// The void's id, `SHA-256(body)`.
    #[must_use]
    pub const fn id(&self) -> &[u8; 32] {
        &self.id
    }

    /// The log position the void itself occupies (`V`).
    #[must_use]
    pub const fn position(&self) -> u64 {
        self.position
    }

    /// The verified body.
    #[must_use]
    pub const fn body(&self) -> &VoidBody {
        &self.body
    }
}

/// Verify a void record pulled at log position `position`: parse, protocol, suite and vault, then
/// the signer — an authorized member **at the membership version in force at that position**
/// (`at_void`, built from the newest epoch record positioned before it), whose key is the one the
/// **chain** holds, never one carried in the record — and the strict Ed25519 signature. A member
/// revoked after `position` is still a valid signer for it, as for envelopes. The service MUST use
/// this same function for "authorized at `V`", or it refuses voids that replicas would honor.
///
/// `issued_at` is audit only: it is never compared with a clock.
///
/// A halted replica must keep verifying membership and epoch records past its stop, or `at_void`
/// cannot be built and the void stays undeterminable.
///
/// # Errors
/// A [`VoidError`]; the record is then ignored.
pub fn verify_void(
    state: &VerifiedState,
    at_void: &EpochAtPosition,
    position: u64,
    record: &[u8],
) -> Result<VerifiedVoid, VoidError> {
    let signed = SignedRecord::parse(record, MAX_VOID_BODY).map_err(VoidError::Malformed)?;
    let body = VoidBody::decode(&signed.body).map_err(|e| match e {
        WireError::UnsupportedProtocol => VoidError::UnsupportedProtocol,
        other => VoidError::Malformed(other),
    })?;
    if body.suite_id != SUITE_ID {
        return Err(VoidError::UnsupportedSuite);
    }
    if body.vault_id != *state.vault_id() {
        return Err(VoidError::WrongVault);
    }
    let signer = state
        .authorized_member_by_fingerprint_at_version(
            &body.signer_fingerprint,
            at_void.membership_head_version(),
        )
        .ok_or(VoidError::SignerNotAuthorized)?;
    if !ed25519_verify_strict(&signer.ed25519_pk, &signed.body, &signed.signature) {
        return Err(VoidError::BadSignature);
    }
    Ok(VerifiedVoid {
        id: signed.id(),
        position,
        body,
    })
}

/// A range of log positions an honored void skips.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HonoredVoid {
    /// The void's id.
    pub void_id: [u8; 32],
    /// The first skipped position (the void's `seq`).
    pub first: u64,
    /// The last skipped position (the position just before the void).
    pub last: u64,
}

/// Why a void is inert (skips nothing, changes nothing). Check [`Inert::is_transient`]: a
/// transient void must be offered again, in `V` order, when the frontier or the key state changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Inert {
    /// The void is not positioned after the position it names. Permanent.
    VoidNotAfterTarget,
    /// An earlier **honored** void already covers the position. Permanent.
    AlreadyCovered,
    /// The named position is at or below the root of the snapshot this replica bootstrapped from:
    /// it never evaluated it. Permanent for this replica, but the engine must treat the root as
    /// suspect (check the snapshot's recorded voids, evaluate the position from retained
    /// evidence, or re-bootstrap from a snapshot at or after `V`).
    PrecedesRoot,
    /// The replica applied the position (it found it valid). Permanent.
    FoundValid,
    /// The replica has not reached the position in log order, or has not found it invalid.
    /// Transient.
    NotYetEvaluated,
    /// The replica is at the position and cannot judge it (no verified key yet). Transient.
    NotDeterminable,
    /// The digests agree but the void names another `command_id` (the digest binds the command id,
    /// so this is a signer fault). Permanent.
    CommandMismatch,
}

impl Inert {
    /// Whether the void may become honorable later (offer it again, in `V` order, when the
    /// frontier or the key state changes).
    #[must_use]
    pub const fn is_transient(self) -> bool {
        matches!(self, Self::NotYetEvaluated | Self::NotDeterminable)
    }
}

/// A void that contradicts what the replica holds: the log or the ledger forked. The engine
/// **halts** on it as a log fork, the `SeqConflict` class (ADR 0074 Addendum E); sync-core only
/// reports it, and makes sure the contradiction is never a silent inert.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForkSuspected {
    /// An authorized member saw another digest at the stop's position than this replica did. An
    /// honest service refuses such a void (it checks the digest against its own index), so only a
    /// service that equivocated or skipped its check can serve one.
    DigestAtStop {
        /// The log position.
        seq: u64,
        /// The digest this replica holds there.
        ours: [u8; 32],
        /// The digest the void names.
        theirs: [u8; 32],
    },
    /// The void id was already honored at another position: the same void at two positions.
    VoidAtTwoPositions {
        /// The void's id.
        void_id: [u8; 32],
        /// The position it was honored at.
        honored_at: u64,
        /// The position it is offered at now.
        offered_at: u64,
    },
}

/// The outcome of offering a void to the honor rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Honor {
    /// Skip the range: the void was honored **now**. Rollback and re-issue of the commands in the
    /// range are driven off this outcome, exactly once per void id.
    Honored(HonoredVoid),
    /// The same void was already honored at this position (an offer repeated after a restart or a
    /// re-pull). Nothing changes, and it must **not** drive rollback or re-issue again: that would
    /// issue the voided commands a second time with new command ids.
    AlreadyHonored(HonoredVoid),
    /// Skip nothing.
    Inert(Inert),
    /// Skip nothing, and the engine MUST surface a possible fork.
    ForkSuspected(ForkSuspected),
}

/// A refused offer: the engine supplied an inconsistent replica state, or voids out of order. The
/// honor rule never guesses (a wrong `FoundValid` would be permanent and split the vault).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum OfferError {
    /// The replica is stopped at a position below `next_to_apply`: the cursor must never pass the
    /// stop (chain records verified past the stop do not advance it).
    #[error("the frontier has passed the stop")]
    StopBehindFrontier,
    /// A snapshot root at or above `next_to_apply`: a bootstrapped replica's cursor starts at
    /// `root + 1`.
    #[error("the frontier is not above the snapshot root")]
    RootNotBelowFrontier,
    /// An honored range reaches `next_to_apply` or beyond: after honoring a void the cursor moves
    /// past its range.
    #[error("the frontier is inside an honored range")]
    FrontierInsideHonoredRange,
    /// The voids are not in strictly ascending position order. Order decides which of two voids
    /// for one position wins, so it is enforced here, not left to the engine.
    #[error("the voids are not in ascending position order")]
    VoidsNotAscending,
}

/// What the replica itself holds when a void is offered. The stop comes from the status (not from
/// a free-form "I saw this"): a replica honors a void only for the position it is **stopped at, in
/// log order**, so two replicas that see the same log in any order build the same ledger.
#[derive(Debug, Clone, Copy)]
pub struct HonorContext<'a> {
    /// The replica's status; the void is honored only against a stop at exactly its `seq`.
    pub status: &'a SyncStatus,
    /// The lowest log position this replica has neither applied nor skipped: every position below
    /// it is done. A void is honored only when this equals the void's `seq`. It **never passes the
    /// stop** (chain records verified past a stop do not count), starts at `root + 1` for a
    /// bootstrapped replica, and moves past a range once a void for it is honored; an offer with
    /// a state that contradicts this is refused ([`OfferError`]).
    pub next_to_apply: u64,
    /// The root (last position) of the snapshot the replica bootstrapped from, if it did.
    pub snapshot_root: Option<u64>,
    /// Whether the replica is at the position but cannot judge it yet (no verified key).
    pub target_not_determinable: bool,
}

/// The honored voids so far. Coverage comes only from here: an inert void covers nothing.
///
/// Persist it in the same transaction as the applied cursor, under the anti-rollback witness.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VoidLedger {
    honored: Vec<HonoredVoid>,
}

impl VoidLedger {
    /// An empty ledger.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The honored voids, in the order they were honored.
    #[must_use]
    pub fn honored(&self) -> &[HonoredVoid] {
        &self.honored
    }

    /// Whether an honored void skips log position `seq`: the engine does not apply it.
    #[must_use]
    pub fn skips(&self, seq: u64) -> bool {
        self.honored.iter().any(|v| v.first <= seq && seq <= v.last)
    }

    /// Offer the verified voids known for this replica, **in ascending position order**, and
    /// return the outcome of each until the first one that changes the replica (a void honored
    /// now, or a suspected fork): after that the context is stale and the engine re-evaluates and
    /// offers again. Order decides which of two voids for one position wins, so it is enforced
    /// here: positions must be strictly ascending.
    ///
    /// A void is honored only if **all** hold, checked in this order (the first failure decides
    /// the outcome, which makes it the same on every replica):
    ///
    /// - `V > seq`;
    /// - no honored void already covers `seq`;
    /// - `seq` is above the snapshot root;
    /// - `seq >= next_to_apply` (a position the replica applied was found valid);
    /// - everything before `seq` is done (`next_to_apply == seq`);
    /// - the replica is **stopped at exactly `seq`** (a stop at an invalid envelope) and can
    ///   judge it;
    /// - the void names the digest the replica holds there (a different one is a suspected fork,
    ///   never a silent inert);
    /// - and the same `command_id`.
    ///
    /// The void's reason byte and signer play no part beyond verification. The same void at the
    /// same position again is [`Honor::AlreadyHonored`]; at another position it is a suspected
    /// fork. An honored void is recorded; an inert one changes nothing.
    ///
    /// # Errors
    /// [`OfferError`] for an inconsistent context (checked first, before any void) or voids out of
    /// order.
    pub fn offer_in_order(
        &mut self,
        voids: &[VerifiedVoid],
        ctx: &HonorContext<'_>,
    ) -> Result<Vec<Honor>, OfferError> {
        if let SyncStatus::EnvelopeInvalid(stop) = ctx.status {
            if stop.seq < ctx.next_to_apply {
                return Err(OfferError::StopBehindFrontier);
            }
        }
        if ctx
            .snapshot_root
            .is_some_and(|root| root >= ctx.next_to_apply)
        {
            return Err(OfferError::RootNotBelowFrontier);
        }
        if self.honored.iter().any(|h| h.last >= ctx.next_to_apply) {
            return Err(OfferError::FrontierInsideHonoredRange);
        }
        if voids.windows(2).any(|w| w[0].position() >= w[1].position()) {
            return Err(OfferError::VoidsNotAscending);
        }
        let mut outcomes = Vec::new();
        for void in voids {
            let outcome = self.offer_one(void, ctx);
            let changes = matches!(outcome, Honor::Honored(_) | Honor::ForkSuspected(_));
            outcomes.push(outcome);
            if changes {
                break;
            }
        }
        Ok(outcomes)
    }

    fn offer_one(&mut self, void: &VerifiedVoid, ctx: &HonorContext<'_>) -> Honor {
        let body = void.body();
        let position = void.position();
        if let Some(existing) = self.honored.iter().find(|h| &h.void_id == void.id()) {
            return if existing.last.checked_add(1) == Some(position) {
                Honor::AlreadyHonored(*existing)
            } else {
                Honor::ForkSuspected(ForkSuspected::VoidAtTwoPositions {
                    void_id: *void.id(),
                    honored_at: existing.last.saturating_add(1),
                    offered_at: position,
                })
            };
        }
        if position <= body.seq {
            return Honor::Inert(Inert::VoidNotAfterTarget);
        }
        if self.skips(body.seq) {
            return Honor::Inert(Inert::AlreadyCovered);
        }
        if ctx.snapshot_root.is_some_and(|root| body.seq <= root) {
            return Honor::Inert(Inert::PrecedesRoot);
        }
        if body.seq < ctx.next_to_apply {
            return Honor::Inert(Inert::FoundValid);
        }
        if ctx.next_to_apply < body.seq {
            return Honor::Inert(Inert::NotYetEvaluated);
        }
        // Here the replica is at the position. Stopped at it: judge the void. Otherwise it is
        // either unable to judge it (no verified key) or has not found it invalid yet; a halted
        // replica applies nothing and judges nothing.
        let stop = match ctx.status {
            SyncStatus::EnvelopeInvalid(stop) if stop.seq == body.seq => stop,
            SyncStatus::Halted(_) => return Honor::Inert(Inert::NotYetEvaluated),
            _ => {
                return Honor::Inert(if ctx.target_not_determinable {
                    Inert::NotDeterminable
                } else {
                    Inert::NotYetEvaluated
                });
            }
        };
        if stop.envelope_digest != body.envelope_digest {
            return Honor::ForkSuspected(ForkSuspected::DigestAtStop {
                seq: body.seq,
                ours: stop.envelope_digest,
                theirs: body.envelope_digest,
            });
        }
        if stop.command_id != body.command_id {
            return Honor::Inert(Inert::CommandMismatch);
        }
        let honored = HonoredVoid {
            void_id: *void.id(),
            first: body.seq,
            last: position - 1,
        };
        self.honored.push(honored);
        Honor::Honored(honored)
    }

    /// The ledger's canonical bytes, for persistence under the anti-rollback witness: a CBE1 list
    /// of `(void id, first, last)`.
    ///
    /// # Errors
    /// [`WireError::TooLong`] for an absurd number of entries.
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        let mut w = Writer::with_label("dohflow/sync-void-ledger/v1");
        w.list(MAX_LEDGER_ENTRIES, &self.honored, |w, h| {
            w.fixed(&h.void_id).u64(h.first).u64(h.last);
        })?;
        Ok(w.finish())
    }

    /// Parse a ledger. A damaged one is refused: a range that runs backwards, or one that does not
    /// start strictly after the previous range ends. Honoring only ever produces strictly
    /// ascending, disjoint ranges, so the encoding of a set of ranges is unique (snapshot
    /// metadata records it).
    ///
    /// # Errors
    /// A [`WireError`].
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut r = Reader::new(bytes);
        r.label("dohflow/sync-void-ledger/v1")?;
        let honored: Vec<HonoredVoid> = r.list(MAX_LEDGER_ENTRIES, 48, |r| {
            Ok(HonoredVoid {
                void_id: r.fixed::<32>()?,
                first: r.u64()?,
                last: r.u64()?,
            })
        })?;
        r.finish()?;
        for (i, h) in honored.iter().enumerate() {
            if h.first > h.last {
                return Err(WireError::ListNotAscending);
            }
            if i > 0 && h.first <= honored[i - 1].last {
                return Err(WireError::ListNotAscending);
            }
        }
        Ok(Self { honored })
    }
}

/// The most entries a ledger holds (one per honored void).
pub const MAX_LEDGER_ENTRIES: usize = 4096;
