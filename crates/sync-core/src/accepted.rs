//! The accepted-envelope index and the retry protocol of `docs/architecture/vault-format.md`
//! C.10 (personal-cfo-t2s4b part 6c).
//!
//! Pure decision logic: no storage, no transport, no clock. The service's index is authoritative
//! and recovery-durable; this module fixes the record's bytes and the two decisions made over it,
//! one on the service when a push arrives ([`decide_push`]) and one on the client when it sees an
//! envelope for a command it still holds in its outbox ([`match_pulled`]). The service cannot
//! decrypt, so it only ever compares identities (ids, tag, digest, commitment); the checks that
//! need the plaintext are `crate::envelope::check_envelope`'s.
//!
//! # Identities come from bytes, never from claims
//!
//! A [`PushIdentity`], an [`OutboxEntry`] and a [`PulledEnvelope`] have no public field
//! constructor: each is built by `from_container` from a parsed container, and the envelope digest
//! is **computed** from the header bytes and ciphertext, not read from a field. A pusher cannot
//! declare a matching digest without sending the matching bytes, so a replay requires resubmitting
//! the exact accepted envelope. (The client's outbox keeps the sealed container bytes for the
//! exact-byte retry of ADR 0074 decision 8, so one derivation serves both sides, and a producer
//! can seal, parse its own container, decrypt, run `check_envelope` and derive its
//! [`OutboxEntry`] in a few lines.)
//!
//! # The service's order around [`decide_push`]
//!
//! 1. authenticate the credential (a revoked device stops here);
//! 2. route the vault, and require the header's `vault_id` to be the routed vault;
//! 3. [`decide_push`] against that vault's index;
//! 4. only for [`PushDecision::Unknown`]: reject an old `key_epoch` (`EpochRejected`), then the
//!    compare-and-swap on `base_seq`, then append and insert the new record.
//!
//! Nothing here replays or requeues a command. A match returns the recorded `seq` and result so
//! the caller can *bind* its outbox entry to them; the sealed bytes are never re-sent as a new
//! command and never re-encrypted.

use std::collections::HashMap;

use thiserror::Error;

use crate::cbe::{Reader, WireError, Writer};
use crate::envelope::canonical_envelope_digest;
use crate::header::{IdempotencyKeyTag, MetadataCommitment, ObjectKind, ParsedContainer};

/// The record's label.
pub const ACCEPTED_LABEL: &str = "dohflow/sync-accepted/v1";
/// The longest opaque `result_ref`, in bytes.
pub const MAX_RESULT_REF: usize = 64;

/// One accepted command in the service's index (C.10). It holds no plaintext key and no payload;
/// the original command metadata is bound by `metadata_commitment`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedRecord {
    /// The vault.
    pub vault_id: [u8; 16],
    /// The command's immutable id.
    pub command_id: [u8; 16],
    /// The tag of the command's idempotency key.
    pub idempotency_key_tag: IdempotencyKeyTag,
    /// The `canonical_envelope_digest` of the accepted envelope.
    pub envelope_digest: [u8; 32],
    /// The commitment to the command's complete immutable metadata.
    pub metadata_commitment: MetadataCommitment,
    /// The key epoch the envelope was sealed under.
    pub key_epoch: u64,
    /// The sequence number the service assigned.
    pub seq: u64,
    /// An opaque, service-chosen result reference (never client data): absent, or 1 to
    /// [`MAX_RESULT_REF`] bytes. An empty one is refused, because it would be a second encoding of
    /// "no result".
    pub result_ref: Option<Vec<u8>>,
}

impl AcceptedRecord {
    /// `label ‖ uuid vault_id ‖ uuid command_id ‖ digest tag ‖ digest envelope_digest ‖ digest
    /// metadata_commitment ‖ u64 key_epoch ‖ u64 seq ‖ opt(bytes(64)) result_ref`.
    ///
    /// # Errors
    /// [`WireError::TooLong`] if `result_ref` is over 64 bytes, [`WireError::EmptyOptionalBytes`]
    /// if it is present and empty.
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        let mut w = Writer::with_label(ACCEPTED_LABEL);
        w.fixed(&self.vault_id)
            .fixed(&self.command_id)
            .fixed(self.idempotency_key_tag.as_bytes())
            .fixed(&self.envelope_digest)
            .fixed(self.metadata_commitment.as_bytes())
            .u64(self.key_epoch)
            .u64(self.seq);
        match &self.result_ref {
            None => {
                w.u8(0);
            }
            Some(reference) => {
                if reference.is_empty() {
                    return Err(WireError::EmptyOptionalBytes);
                }
                w.u8(1);
                w.bytes(MAX_RESULT_REF, reference)?;
            }
        }
        Ok(w.finish())
    }

    /// Parse a record from exactly `bytes`.
    ///
    /// # Errors
    /// A [`WireError`] for any CBE1 violation.
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut r = Reader::new(bytes);
        r.label(ACCEPTED_LABEL)?;
        let vault_id = r.fixed::<16>()?;
        let command_id = r.fixed::<16>()?;
        let tag = r.fixed::<32>()?;
        let envelope_digest = r.fixed::<32>()?;
        let commitment = r.fixed::<32>()?;
        let key_epoch = r.u64()?;
        let seq = r.u64()?;
        let result_ref = r.opt(|r| {
            let bytes = r.bytes(MAX_RESULT_REF)?;
            if bytes.is_empty() {
                Err(WireError::EmptyOptionalBytes)
            } else {
                Ok(bytes.to_vec())
            }
        })?;
        r.finish()?;
        Ok(Self {
            vault_id,
            command_id,
            idempotency_key_tag: IdempotencyKeyTag::from_derived(tag),
            envelope_digest,
            metadata_commitment: MetadataCommitment::from_derived(commitment),
            key_epoch,
            seq,
            result_ref,
        })
    }
}

/// Why an identity could not be taken from a container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum IdentityError {
    /// The container's header is a snapshot header; only command envelopes are indexed.
    #[error("the container is not a command envelope")]
    NotAnEnvelope,
    /// The digest could not be computed (a length does not fit its field).
    #[error("the envelope digest could not be computed")]
    Digest,
}

/// The identity fields of a command envelope, read from its parsed container with the digest
/// **computed** from the exact header bytes and ciphertext.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Identity {
    vault_id: [u8; 16],
    command_id: [u8; 16],
    idempotency_key_tag: IdempotencyKeyTag,
    envelope_digest: [u8; 32],
    metadata_commitment: MetadataCommitment,
    device_id: [u8; 16],
}

impl Identity {
    fn from_container(container: &ParsedContainer<'_>) -> Result<Self, IdentityError> {
        let ObjectKind::Envelope {
            idempotency_key_tag,
            immutable_metadata_commitment,
            ..
        } = container.header.object
        else {
            return Err(IdentityError::NotAnEnvelope);
        };
        let envelope_digest =
            canonical_envelope_digest(container.header_bytes, container.ciphertext)
                .map_err(|_| IdentityError::Digest)?;
        Ok(Self {
            vault_id: container.header.vault_id,
            command_id: container.header.object_id,
            idempotency_key_tag,
            envelope_digest,
            metadata_commitment: immutable_metadata_commitment,
            device_id: container.header.device_id,
        })
    }
}

/// What a push carries, as the service computes it from the received bytes: everything the
/// service can see without decrypting. There is no public constructor from fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PushIdentity(Identity);

impl PushIdentity {
    /// From a parsed container. The digest is computed here.
    ///
    /// # Errors
    /// [`IdentityError`].
    pub fn from_container(container: &ParsedContainer<'_>) -> Result<Self, IdentityError> {
        Identity::from_container(container).map(Self)
    }

    /// The vault the header names. The caller requires it to equal the routed vault **before**
    /// [`decide_push`].
    #[must_use]
    pub const fn vault_id(&self) -> &[u8; 16] {
        &self.0.vault_id
    }

    /// The command's immutable id.
    #[must_use]
    pub const fn command_id(&self) -> &[u8; 16] {
        &self.0.command_id
    }

    /// The header's idempotency-key tag.
    #[must_use]
    pub const fn idempotency_key_tag(&self) -> &IdempotencyKeyTag {
        &self.0.idempotency_key_tag
    }

    /// The computed `canonical_envelope_digest`.
    #[must_use]
    pub const fn envelope_digest(&self) -> &[u8; 32] {
        &self.0.envelope_digest
    }

    /// The header's metadata commitment.
    #[must_use]
    pub const fn metadata_commitment(&self) -> &MetadataCommitment {
        &self.0.metadata_commitment
    }

    /// The record the service stores once it appends this envelope.
    #[must_use]
    pub fn accepted(
        &self,
        key_epoch: u64,
        seq: u64,
        result_ref: Option<Vec<u8>>,
    ) -> AcceptedRecord {
        AcceptedRecord {
            vault_id: self.0.vault_id,
            command_id: self.0.command_id,
            idempotency_key_tag: self.0.idempotency_key_tag,
            envelope_digest: self.0.envelope_digest,
            metadata_commitment: self.0.metadata_commitment,
            key_epoch,
            seq,
            result_ref,
        }
    }
}

/// Lookups over the service's accepted-envelope index. An implementation is per vault.
///
/// The contract: at most one record per `command_id` and at most one per tag, and a record is
/// inserted only when [`decide_push`] returned [`PushDecision::Unknown`] and the append succeeded.
/// [`InMemoryIndex`] is the reference and refuses an insert that would break it.
pub trait AcceptedIndex {
    /// The record for a `command_id`, if any.
    fn by_command_id(&self, command_id: &[u8; 16]) -> Option<&AcceptedRecord>;
    /// The record for an idempotency-key tag, if any.
    fn by_tag(&self, tag: &IdempotencyKeyTag) -> Option<&AcceptedRecord>;
}

/// The outcome of the first steps of a push (C.10 steps 1 to 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushDecision<'a> {
    /// Both identities are already accepted as this exact envelope: return the record's `seq` and
    /// `result_ref`, even when the request's `base_seq` is stale and even when its `key_epoch` is
    /// old. Nothing is appended.
    Replay(&'a AcceptedRecord),
    /// An identity is bound to a different command, tag, envelope or metadata, or the two
    /// lookups disagree. Reject with `ProtocolFork`.
    ProtocolFork,
    /// Neither identity is known. The caller goes on to reject an old `key_epoch`
    /// (`EpochRejected`) and then applies the compare-and-swap on `base_seq`.
    Unknown,
}

/// The index returned a record that cannot belong to this lookup or this vault. That is a service
/// bug or corruption, not a client fault, and is reported apart from `ProtocolFork`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("the accepted-envelope index is inconsistent")]
pub struct IndexInconsistent;

/// C.10, on a push: look up **both** `command_id` and the tag, then decide in the spec's order.
///
/// A replay needs both lookups to land on the **same** record and that record's digest and
/// commitment to equal the push's (the tag and command id are then equal by construction). If
/// only one identity is known, or the two lookups land on different records, or a bound value
/// differs, it is a fork. The caller has already required the header's `vault_id` to be the routed
/// vault; a record of another vault, or one that does not carry the key it was looked up by, is
/// the index's fault.
///
/// # Errors
/// [`IndexInconsistent`].
pub fn decide_push<'a>(
    index: &'a impl AcceptedIndex,
    push: &PushIdentity,
) -> Result<PushDecision<'a>, IndexInconsistent> {
    let by_command = index.by_command_id(push.command_id());
    let by_tag = index.by_tag(push.idempotency_key_tag());
    for record in [by_command, by_tag].into_iter().flatten() {
        if record.vault_id != *push.vault_id() {
            return Err(IndexInconsistent);
        }
    }
    if by_command.is_some_and(|record| record.command_id != *push.command_id())
        || by_tag.is_some_and(|record| record.idempotency_key_tag != *push.idempotency_key_tag())
    {
        return Err(IndexInconsistent);
    }
    Ok(match (by_command, by_tag) {
        (None, None) => PushDecision::Unknown,
        (Some(record), Some(other))
            if record == other
                && record.envelope_digest == *push.envelope_digest()
                && record.metadata_commitment == *push.metadata_commitment() =>
        {
            PushDecision::Replay(record)
        }
        _ => PushDecision::ProtocolFork,
    })
}

/// A reference [`AcceptedIndex`] over two hash maps, for one vault.
#[derive(Debug, Default)]
pub struct InMemoryIndex {
    by_command: HashMap<[u8; 16], AcceptedRecord>,
    by_tag: HashMap<[u8; 32], AcceptedRecord>,
}

/// An insert that would put a second record under a `command_id` or a tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("an accepted record already exists for this command id or tag")]
pub struct DuplicateIdentity;

impl InMemoryIndex {
    /// An empty index.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a record accepted after [`PushDecision::Unknown`].
    ///
    /// # Errors
    /// [`DuplicateIdentity`] if the `command_id` or the tag is already present (nothing is
    /// inserted).
    pub fn insert(&mut self, record: AcceptedRecord) -> Result<(), DuplicateIdentity> {
        if self.by_command.contains_key(&record.command_id)
            || self
                .by_tag
                .contains_key(record.idempotency_key_tag.as_bytes())
        {
            return Err(DuplicateIdentity);
        }
        self.by_command.insert(record.command_id, record.clone());
        self.by_tag
            .insert(*record.idempotency_key_tag.as_bytes(), record);
        Ok(())
    }
}

impl AcceptedIndex for InMemoryIndex {
    fn by_command_id(&self, command_id: &[u8; 16]) -> Option<&AcceptedRecord> {
        self.by_command.get(command_id)
    }

    fn by_tag(&self, tag: &IdempotencyKeyTag) -> Option<&AcceptedRecord> {
        self.by_tag.get(tag.as_bytes())
    }
}

/// What a client holds for a command it sealed and has not yet seen accepted: the identity of
/// the saved sealed container, derived from those exact bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutboxEntry(Identity);

impl OutboxEntry {
    /// From the sealed container the outbox keeps for exact-byte retry. The digest is computed.
    ///
    /// # Errors
    /// [`IdentityError`].
    pub fn from_container(container: &ParsedContainer<'_>) -> Result<Self, IdentityError> {
        Identity::from_container(container).map(Self)
    }
}

/// An envelope pulled from the log, **already authenticated and checked** (AEAD opened,
/// `check_envelope` passed, membership verified), with what the service assigned to it. The
/// `seq` and `result_ref` come from the service's response and index, never from the AAD.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PulledEnvelope {
    identity: Identity,
    seq: u64,
    result_ref: Option<Vec<u8>>,
}

impl PulledEnvelope {
    /// From the pulled container and the service's response for it.
    ///
    /// # Errors
    /// [`IdentityError`].
    pub fn from_container(
        container: &ParsedContainer<'_>,
        seq: u64,
        result_ref: Option<Vec<u8>>,
    ) -> Result<Self, IdentityError> {
        Ok(Self {
            identity: Identity::from_container(container)?,
            seq,
            result_ref,
        })
    }

    /// The device that sealed it, for the diagnostic record of a fork.
    #[must_use]
    pub const fn device_id(&self) -> &[u8; 16] {
        &self.identity.device_id
    }
}

/// The client's view of a pulled envelope against one outbox entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PullMatch {
    /// The pulled envelope **is** the outbox entry, byte for byte: bind the entry to this `seq`
    /// and result. Never replay it, requeue it or re-encrypt it.
    BindToAccepted {
        /// The service-assigned sequence number.
        seq: u64,
        /// The service's result reference.
        result_ref: Option<Vec<u8>>,
    },
    /// The same command id or tag carries a different envelope. Both envelopes are retained for
    /// diagnosis (with each header's `device_id` and fingerprint, so the member chain and the UI
    /// can name who sealed what) and sync halts for this vault (`ProtocolFork`).
    ProtocolFork,
    /// An unrelated command.
    NotMine,
}

/// C.10, on the client: does a pulled envelope settle this outbox entry?
///
/// Equal digests imply equal headers (device, fingerprint, commitment, tag, command id, every
/// field), so a different device sealing the same command id necessarily differs in digest and is
/// a fork without any further comparison.
#[must_use]
pub fn match_pulled(outbox: &OutboxEntry, pulled: &PulledEnvelope) -> PullMatch {
    let mine = &outbox.0;
    let theirs = &pulled.identity;
    let same_command = mine.command_id == theirs.command_id;
    let same_tag = mine.idempotency_key_tag == theirs.idempotency_key_tag;
    if !same_command && !same_tag {
        return PullMatch::NotMine;
    }
    if same_command
        && same_tag
        && mine.envelope_digest == theirs.envelope_digest
        && mine.metadata_commitment == theirs.metadata_commitment
    {
        PullMatch::BindToAccepted {
            seq: pulled.seq,
            result_ref: pulled.result_ref.clone(),
        }
    } else {
        PullMatch::ProtocolFork
    }
}
