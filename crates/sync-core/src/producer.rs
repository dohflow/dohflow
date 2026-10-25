//! The producer's seal-then-verify rule (`docs/architecture/vault-format.md` C.4, "Producer
//! rule"; personal-cfo-g3m.17).
//!
//! The service stores ciphertext it cannot read, so it cannot tell a consistently sealed envelope
//! from one a buggy or partly failed producer sealed inconsistently (a wrong tag, a stale
//! commitment, a manifest commitment for an empty manifest). Such an envelope would be accepted at
//! a sequence number, and every replica would then fail closed there. So the producer runs the
//! receiver's own checks on the **exact bytes it is about to upload**, and uploads only if they
//! pass: [`verify_before_upload`]. On any failure nothing is uploaded and the command stays in the
//! outbox with a typed [`ProducerError`] the user can be told about.
//!
//! The AEAD key never enters this crate: the caller supplies an [`EnvelopeOpener`]. That opener
//! must be the **receiver path** (the key derived from the *expected* vault id, key epoch and this
//! device's fingerprint from the chain, and the nonce built from the header's own domain and
//! counter), never the sealing code's key object and never the plaintext the producer still holds
//! in memory. Opening through the sealing key, or handing back the producer's own plaintext, would
//! hide exactly the seal bugs this check exists to catch (a wrong AAD, a wrong key derivation, a
//! wrong nonce).

use thiserror::Error;

use crate::accepted::{IdentityError, OutboxEntry, PushIdentity};
use crate::cbe::WireError;
use crate::envelope::{
    check_envelope, EnvelopePlaintext, IdempotencyKey, ReceiverError, MAX_PAYLOAD,
};
use crate::header::{
    parse_container_with_limit, IdempotencyKeyTag, ParsedContainer, MAX_ENVELOPE_CONTAINER_LEN,
};

/// Why an envelope must not be uploaded. Every variant leaves the command in the outbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ProducerError {
    /// The sealed bytes are not a well-formed container (including one over the size cap).
    #[error("the sealed object is not well formed: {0}")]
    Unparseable(WireError),
    /// The ciphertext does not open under the key and header the producer holds: it was not
    /// sealed with them.
    #[error("the sealed object does not open")]
    NotOpenable,
    /// The opened plaintext is not a well-formed envelope plaintext.
    #[error("the opened plaintext is not well formed: {0}")]
    PlaintextInvalid(WireError),
    /// The header and plaintext disagree: the receiver's checks would refuse this envelope.
    #[error("the envelope is inconsistently sealed: {0}")]
    Inconsistent(ReceiverError),
    /// The identity could not be derived from the verified bytes.
    #[error("the envelope identity could not be derived: {0}")]
    Identity(IdentityError),
}

/// Opens a sealed envelope the way a **receiver** would. The one implementation that may be used
/// to approve an upload derives the AEAD key from the expected vault id, key epoch and this
/// device's fingerprint from the membership chain (C.3), builds the nonce from the header's nonce
/// domain and invocation counter, and uses the header bytes as the AAD. Nothing that returns a
/// plaintext the caller already holds is an opener.
pub trait EnvelopeOpener {
    /// The plaintext of `container`, or `None` if it does not open.
    fn open(&self, container: &ParsedContainer<'_>) -> Option<Vec<u8>>;
}

/// An envelope that passed the receiver's checks as sealed. It **owns the verified bytes** and the
/// identities derived from them, and is constructible only by [`verify_before_upload`]. The
/// transport's push takes an `UploadApproved`, never raw bytes plus an approval, so "upload exactly
/// what was verified" holds by type; an exact-byte retry reuses the same value, which is persisted
/// with the outbox entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadApproved {
    bytes: Vec<u8>,
    outbox: OutboxEntry,
    push: PushIdentity,
}

impl UploadApproved {
    /// The verified container bytes: the only bytes to upload.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The outbox entry for the verified bytes (their digest is computed from them).
    #[must_use]
    pub const fn outbox(&self) -> &OutboxEntry {
        &self.outbox
    }

    /// The identity the push will carry; equal by construction to what the service computes from
    /// the same bytes.
    #[must_use]
    pub const fn push(&self) -> &PushIdentity {
        &self.push
    }
}

/// Seal-then-verify: open the sealed container the client is about to upload with the receiver
/// path ([`EnvelopeOpener`]), decode it, and run [`check_envelope`] on header and plaintext. The
/// verified bytes are exactly the bytes passed in, the result owns them and derives its
/// identities from them, so the producer cannot approve one envelope and upload another.
///
/// A [`ProducerError::NotOpenable`] or [`ProducerError::Inconsistent`] is a producer bug, not a
/// transient error: keep the command in the outbox, surface a typed "stuck, needs an update or a
/// report" state, and do not retry in a loop.
///
/// The membership check (the sealing device's key belongs to a member at the epoch) is the
/// caller's, as for [`check_envelope`].
///
/// # Errors
/// A [`ProducerError`]; on any error the envelope must not be uploaded.
pub fn verify_before_upload(
    container_bytes: &[u8],
    opener: &impl EnvelopeOpener,
    tag_for: impl FnOnce(&IdempotencyKey) -> IdempotencyKeyTag,
) -> Result<UploadApproved, ProducerError> {
    let container = parse_container_with_limit(container_bytes, MAX_ENVELOPE_CONTAINER_LEN)
        .map_err(ProducerError::Unparseable)?;
    let plaintext_bytes = opener.open(&container).ok_or(ProducerError::NotOpenable)?;
    // A plaintext larger than its bound cannot be an envelope plaintext.
    if plaintext_bytes.len() > MAX_PAYLOAD + 64 * 1024 {
        return Err(ProducerError::PlaintextInvalid(WireError::TooLong));
    }
    let plaintext =
        EnvelopePlaintext::decode(&plaintext_bytes).map_err(ProducerError::PlaintextInvalid)?;
    check_envelope(&container.header, &plaintext, tag_for).map_err(ProducerError::Inconsistent)?;
    let outbox = OutboxEntry::from_container(&container).map_err(ProducerError::Identity)?;
    let push = PushIdentity::from_container(&container).map_err(ProducerError::Identity)?;
    Ok(UploadApproved {
        bytes: container_bytes.to_vec(),
        outbox,
        push,
    })
}
