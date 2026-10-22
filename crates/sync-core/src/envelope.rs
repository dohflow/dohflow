//! The command envelope's plaintext, the commitments that bind it to its clear header, the
//! envelope digest, and the checks a receiver runs after decryption
//! (`docs/architecture/vault-format.md` C.3, C.4, C.5; personal-cfo-t2s4b part 6b).
//!
//! Everything here is pure CBE1 and SHA-256. The idempotency-key tag is an HMAC under a vault
//! secret, so it is derived in `vault-crypto` and enters this module as an
//! [`IdempotencyKeyTag`]; nothing here holds a key.

use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::cbe::{Reader, WireError, Writer};
use crate::header::{
    ClearHeader, IdempotencyKeyTag, ManifestCommitment, MetadataCommitment, NonceDomain,
    ObjectKind, OBJECT_ENVELOPE,
};

/// The plaintext's label.
pub const PLAINTEXT_LABEL: &str = "dohflow/sync-envelope-plaintext/v1";
/// The most digests a delta manifest (or any manifest list) may hold.
pub const MAX_MANIFEST_ENTRIES: usize = 4096;
/// The longest payload, in bytes (`bytes(2^24)`).
pub const MAX_PAYLOAD: usize = 1 << 24;
/// The longest `actor_id`, in bytes.
pub const MAX_ACTOR_ID: usize = 256;

/// An idempotency key as it enters a Sync envelope: a UUID version 4 with the RFC 4122 variant,
/// drawn from a CSPRNG (C.5). Anything else (a version 7 key, a prefixed string, a non-UUID) is
/// not constructible, so a value of this type is always legal on the wire.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct IdempotencyKey([u8; 16]);

/// The idempotency key was not a UUID version 4 (C.5; the typed `InvalidIdempotencyKey`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("the idempotency key is not a canonical UUID version 4")]
pub struct InvalidIdempotencyKey;

impl IdempotencyKey {
    /// From the 16 raw bytes (the `canonical_idempotency_key`).
    ///
    /// # Errors
    /// [`InvalidIdempotencyKey`] unless the version nibble is 4 and the variant bits are `10`.
    pub fn from_bytes(bytes: [u8; 16]) -> Result<Self, InvalidIdempotencyKey> {
        if bytes[6] >> 4 == 4 && bytes[8] >> 6 == 0b10 {
            Ok(Self(bytes))
        } else {
            Err(InvalidIdempotencyKey)
        }
    }

    /// From the key's text: exactly the 36-character lowercase hyphenated form
    /// `xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx`. There is one text form per key, so the tag derived
    /// from it is stable; a prefix, braces, uppercase, a missing hyphen or a different version
    /// are refused.
    ///
    /// # Errors
    /// [`InvalidIdempotencyKey`].
    pub fn parse(text: &str) -> Result<Self, InvalidIdempotencyKey> {
        let bytes = text.as_bytes();
        if bytes.len() != 36 {
            return Err(InvalidIdempotencyKey);
        }
        let mut raw = [0_u8; 16];
        let mut index = 0;
        let mut position = 0;
        while position < 36 {
            if matches!(position, 8 | 13 | 18 | 23) {
                if bytes[position] != b'-' {
                    return Err(InvalidIdempotencyKey);
                }
                position += 1;
                continue;
            }
            let high = lower_hex(bytes[position]).ok_or(InvalidIdempotencyKey)?;
            let low = lower_hex(bytes[position + 1]).ok_or(InvalidIdempotencyKey)?;
            raw[index] = high << 4 | low;
            index += 1;
            position += 2;
        }
        Self::from_bytes(raw)
    }

    /// The 16 raw bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl core::fmt::Debug for IdempotencyKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // The key is a secret-ish identifier (the tag exists to avoid exposing it); never print it.
        f.write_str("IdempotencyKey(..)")
    }
}

fn lower_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

/// The complete original `CommandMeta` of a command, immutable for the life of the command
/// (C.4): what the metadata commitment binds and what a rebase reads back.
///
/// `Debug` prints the ids' presence and the issue time only, never the actor id.
#[derive(Clone, PartialEq, Eq)]
pub struct ImmutableMeta {
    /// The command's immutable id (may be a UUID version 7).
    pub command_id: [u8; 16],
    /// The correlation id.
    pub correlation_id: [u8; 16],
    /// The causing command, if any.
    pub causation_id: Option<[u8; 16]>,
    /// The actor type: `1` user, `2` system, `3` importer, `4` agent (C.1.3).
    pub actor_type: u8,
    /// The actor id, at most [`MAX_ACTOR_ID`] bytes.
    pub actor_id: String,
    /// The idempotency key.
    pub idempotency_key: IdempotencyKey,
    /// Microseconds since the Unix epoch, UTC.
    pub issued_at: i64,
    /// The device that issued the command.
    pub device_id: [u8; 16],
}

fn check_actor_type(actor_type: u8) -> Result<(), WireError> {
    if (1..=4).contains(&actor_type) {
        Ok(())
    } else {
        Err(WireError::UnknownActorType)
    }
}

impl core::fmt::Debug for ImmutableMeta {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ImmutableMeta")
            .field("actor_type", &self.actor_type)
            .field("has_causation_id", &self.causation_id.is_some())
            .field("issued_at", &self.issued_at)
            .finish_non_exhaustive()
    }
}

impl ImmutableMeta {
    fn write_fields(&self, w: &mut Writer) -> Result<(), WireError> {
        check_actor_type(self.actor_type)?;
        w.fixed(&self.command_id)
            .fixed(&self.correlation_id)
            .opt(self.causation_id.as_ref(), |w, id| {
                w.fixed(id);
            })
            .u8(self.actor_type)
            .text(MAX_ACTOR_ID, &self.actor_id)?
            .fixed(self.idempotency_key.as_bytes())
            .i64(self.issued_at);
        Ok(())
    }

    /// `SHA-256(label "dohflow/sync-meta-commit/v1" ‖ uuid command_id ‖ uuid correlation_id ‖
    /// opt(uuid) causation_id ‖ u8 actor_type ‖ text(256) actor_id ‖ uuid idempotency_key ‖
    /// i64 issued_at ‖ uuid device_id)` (C.3).
    ///
    /// # Errors
    /// [`WireError::UnknownActorType`] or [`WireError::TooLong`].
    pub fn commitment(&self) -> Result<MetadataCommitment, WireError> {
        let mut w = Writer::with_label("dohflow/sync-meta-commit/v1");
        self.write_fields(&mut w)?;
        w.fixed(&self.device_id);
        Ok(MetadataCommitment::from_derived(
            Sha256::digest(w.as_bytes()).into(),
        ))
    }
}

/// A set of artifact ids (C.9) in the one order the wire allows: strictly ascending bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactManifest(Vec<[u8; 32]>);

impl ArtifactManifest {
    /// Order `ids` and refuse a duplicate or more than [`MAX_MANIFEST_ENTRIES`].
    ///
    /// # Errors
    /// [`WireError::TooLong`], [`WireError::ListNotAscending`] (a duplicate).
    pub fn new(mut ids: Vec<[u8; 32]>) -> Result<Self, WireError> {
        if ids.len() > MAX_MANIFEST_ENTRIES {
            return Err(WireError::TooLong);
        }
        ids.sort_unstable();
        if ids.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(WireError::ListNotAscending);
        }
        Ok(Self(ids))
    }

    /// The ids, strictly ascending.
    #[must_use]
    pub fn ids(&self) -> &[[u8; 32]] {
        &self.0
    }

    /// Whether the manifest is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// `SHA-256(label "dohflow/sync-artifact-manifest/v1" ‖ list(digest, 4096))` (C.3).
    #[must_use]
    pub fn commitment(&self) -> ManifestCommitment {
        ManifestCommitment::from_derived(Sha256::digest(manifest_bytes(&self.0).as_slice()).into())
    }
}

fn manifest_bytes(ids: &[[u8; 32]]) -> Vec<u8> {
    let mut w = Writer::with_label("dohflow/sync-artifact-manifest/v1");
    w.list(MAX_MANIFEST_ENTRIES, ids, |w, id| {
        w.fixed(id);
    })
    .expect("a manifest of at most 4096 ids always fits");
    w.finish()
}

/// `EnvelopePlaintextV1` (C.4): what is encrypted under the envelope's AEAD key.
///
/// `Debug` prints sizes and a short prefix of the write-set hash, never the payload (canonical
/// JSON of financial data) or the actor id.
#[derive(Clone, PartialEq, Eq)]
pub struct EnvelopePlaintext {
    /// The command's complete immutable metadata.
    pub meta: ImmutableMeta,
    /// The class-1 write-set hash (C.4).
    pub write_set_hash: [u8; 32],
    /// The delta manifest, strictly ascending; empty when the command adds no artifact.
    pub delta_manifest: ArtifactManifest,
    /// The canonical-JSON command payload, at most [`MAX_PAYLOAD`] bytes.
    pub payload: Vec<u8>,
}

impl core::fmt::Debug for EnvelopePlaintext {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("EnvelopePlaintext")
            .field("payload_len", &self.payload.len())
            .field("delta_manifest_len", &self.delta_manifest.ids().len())
            .field(
                "write_set_hash",
                &format_args!(
                    "{:02x}{:02x}..",
                    self.write_set_hash[0], self.write_set_hash[1]
                ),
            )
            .finish_non_exhaustive()
    }
}

impl EnvelopePlaintext {
    /// The plaintext's canonical bytes: `label ‖ uuid command_id ‖ uuid correlation_id ‖
    /// opt(uuid) causation_id ‖ u8 actor_type ‖ text(256) actor_id ‖ uuid idempotency_key ‖
    /// i64 issued_at ‖ uuid device_id ‖ digest write_set_hash ‖ list(digest, 4096)
    /// delta_manifest ‖ bytes(2^24) payload`.
    ///
    /// # Errors
    /// [`WireError::UnknownActorType`], [`WireError::TooLong`].
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        let mut w = Writer::with_label(PLAINTEXT_LABEL);
        self.meta.write_fields(&mut w)?;
        w.fixed(&self.meta.device_id).fixed(&self.write_set_hash);
        w.list(MAX_MANIFEST_ENTRIES, self.delta_manifest.ids(), |w, id| {
            w.fixed(id);
        })?;
        w.bytes(MAX_PAYLOAD, &self.payload)?;
        Ok(w.finish())
    }

    /// Parse the plaintext from exactly `bytes`. A delta manifest that is not strictly
    /// ascending, an `actor_type` outside the registry, and a non-UUIDv4 idempotency key are
    /// refused.
    ///
    /// # Errors
    /// A [`WireError`]; an illegal idempotency key is [`WireError::InvalidIdempotencyKey`].
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut r = Reader::new(bytes);
        r.label(PLAINTEXT_LABEL)?;
        let command_id = r.fixed::<16>()?;
        let correlation_id = r.fixed::<16>()?;
        let causation_id = r.opt(|r| r.fixed::<16>())?;
        let actor_type = r.u8()?;
        check_actor_type(actor_type)?;
        let actor_id = r.text(MAX_ACTOR_ID)?.to_owned();
        let idempotency_key = IdempotencyKey::from_bytes(r.fixed::<16>()?)
            .map_err(|_| WireError::InvalidIdempotencyKey)?;
        let issued_at = r.i64()?;
        let device_id = r.fixed::<16>()?;
        let write_set_hash = r.fixed::<32>()?;
        let delta = r.list(MAX_MANIFEST_ENTRIES, 32, |r| r.fixed::<32>())?;
        if delta.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(WireError::ListNotAscending);
        }
        let payload = r.bytes(MAX_PAYLOAD)?.to_vec();
        r.finish()?;
        Ok(Self {
            meta: ImmutableMeta {
                command_id,
                correlation_id,
                causation_id,
                actor_type,
                actor_id,
                idempotency_key,
                issued_at,
                device_id,
            },
            write_set_hash,
            delta_manifest: ArtifactManifest(delta),
            payload,
        })
    }
}

/// `canonical_envelope_digest = SHA-256(label "dohflow/sync-envelope-digest/v1" ‖ u32 len ‖
/// header bytes ‖ u64 len ‖ ciphertext)` (C.3): two envelopes with the same digest are the same
/// bytes, so a pulled envelope that matches an outbox entry on `command_id` and digest is an
/// authenticated accepted retry, and one that matches the id but not the digest is a fork.
///
/// # Errors
/// [`WireError::TooLong`] if the header does not fit its `u32` length.
pub fn canonical_envelope_digest(
    header_bytes: &[u8],
    ciphertext: &[u8],
) -> Result<[u8; 32], WireError> {
    let header_len = u32::try_from(header_bytes.len()).map_err(|_| WireError::TooLong)?;
    let ciphertext_len = u64::try_from(ciphertext.len()).map_err(|_| WireError::TooLong)?;
    let mut hasher = Sha256::new();
    hasher.update(Writer::with_label("dohflow/sync-envelope-digest/v1").as_bytes());
    hasher.update(header_len.to_be_bytes());
    hasher.update(header_bytes);
    hasher.update(ciphertext_len.to_be_bytes());
    hasher.update(ciphertext);
    Ok(hasher.finalize().into())
}

/// Why a decrypted envelope failed the receiver's checks (C.4). Every variant fails closed:
/// nothing is applied, rebased, queued or indexed, and the caller keeps the ciphertext and a
/// diagnostic record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ReceiverError {
    /// The header is a snapshot header, not an envelope header.
    #[error("the header is not an envelope header")]
    NotAnEnvelope,
    /// The header's idempotency-key tag is not the tag recomputed from the plaintext's key.
    #[error("the idempotency-key tag does not match the plaintext")]
    TagMismatch,
    /// The header's metadata commitment is not the commitment recomputed from the plaintext.
    #[error("the metadata commitment does not match the plaintext")]
    MetadataCommitmentMismatch,
    /// The header's `object_id` is not the plaintext's `command_id`.
    #[error("the object id is not the command id")]
    ObjectIdMismatch,
    /// The header's `device_id` is not the plaintext's `device_id`.
    #[error("the header device is not the plaintext device")]
    DeviceMismatch,
    /// The header's nonce domain is not the one derived for its device key and object type.
    #[error("the nonce domain is not the derived one")]
    NonceDomainMismatch,
    /// The header's manifest commitment is present for an empty delta manifest, absent for a
    /// non-empty one, or not the manifest's commitment.
    #[error("the manifest commitment does not match the delta manifest")]
    ManifestCommitmentMismatch,
    /// The plaintext's own fields could not be committed (an unknown actor type, an oversize
    /// actor id).
    #[error("the plaintext metadata is not encodable")]
    UnencodableMetadata,
}

/// The checks of C.4 "Receiver checks" that need no membership chain, run **after**
/// authenticated decryption and **before** any apply, rebase, queue or accepted-index
/// reconciliation.
///
/// `tag_for` computes the idempotency-key tag of a key (HMAC under the vault's tag secret, in
/// `vault-crypto`). It is a function rather than a value so the only input to the comparison is
/// the plaintext's own key: a caller cannot satisfy check 1 by passing the header's tag back.
/// `sync-core` still holds no secret.
///
/// What this does **not** check, and the caller must, because it needs state `sync-core` does not
/// have: that `device_id` and `device_key_fingerprint` belong to one member of the membership
/// chain at the object's epoch; and that the AEAD key was derived from the receiver's expected
/// `vault_id`, the accepted `key_epoch` and the chain's fingerprint for `device_id`, **not** from
/// the header's own values (the header is only bound to the ciphertext because the key it was
/// opened with came from independent expectations).
///
/// # Errors
/// The first failing check, as a [`ReceiverError`]. All of them are tampering or a producer bug;
/// none is recoverable by retrying.
pub fn check_envelope(
    header: &ClearHeader,
    plaintext: &EnvelopePlaintext,
    tag_for: impl FnOnce(&IdempotencyKey) -> IdempotencyKeyTag,
) -> Result<(), ReceiverError> {
    let ObjectKind::Envelope {
        idempotency_key_tag,
        immutable_metadata_commitment,
        artifact_manifest_commitment,
    } = &header.object
    else {
        return Err(ReceiverError::NotAnEnvelope);
    };
    if header.object_id != plaintext.meta.command_id {
        return Err(ReceiverError::ObjectIdMismatch);
    }
    if header.device_id != plaintext.meta.device_id {
        return Err(ReceiverError::DeviceMismatch);
    }
    if *idempotency_key_tag != tag_for(&plaintext.meta.idempotency_key) {
        return Err(ReceiverError::TagMismatch);
    }
    let commitment = plaintext
        .meta
        .commitment()
        .map_err(|_| ReceiverError::UnencodableMetadata)?;
    if *immutable_metadata_commitment != commitment {
        return Err(ReceiverError::MetadataCommitmentMismatch);
    }
    let expected_domain = NonceDomain::derive(
        &header.vault_id,
        header.key_epoch,
        &header.device_key_fingerprint,
        OBJECT_ENVELOPE,
    );
    if header.nonce_domain != expected_domain {
        return Err(ReceiverError::NonceDomainMismatch);
    }
    // The manifest rule: the header commitment is present exactly when the decrypted delta
    // manifest is non-empty, and then it is that manifest's commitment. A present commitment for
    // an empty manifest would be a second valid encoding of the same envelope, with a different
    // digest, so it is refused.
    let manifest_ok = match artifact_manifest_commitment {
        None => plaintext.delta_manifest.is_empty(),
        Some(found) => {
            !plaintext.delta_manifest.is_empty() && *found == plaintext.delta_manifest.commitment()
        }
    };
    if !manifest_ok {
        return Err(ReceiverError::ManifestCommitmentMismatch);
    }
    Ok(())
}
