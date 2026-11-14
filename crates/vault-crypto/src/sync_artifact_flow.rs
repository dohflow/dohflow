//! Publishing and fetching class-1b artifacts, and the receiver's local re-encryption
//! (`docs/architecture/vault-format.md` C.9; personal-cfo-0g528.2, part S2c).
//!
//! * [`publish`] seals an artifact, puts it through an [`ArtifactTransport`] and accepts the
//!   service's receipt **only if it equals the receipt computed locally** from the bytes that were
//!   sealed. A matching receipt is what lets a producer reference the artifact (receipt before
//!   reference, `sync_core::artifact_closure::build_delta`). Nothing here persists: the caller writes
//!   the row first and records the receipt after, in one transaction of its own.
//! * [`fetch`] gets an object, checks its ciphertext against the **hash the envelope bound to the
//!   id**, and only then opens it from the **expected** address. A missing or unreachable object, or
//!   bytes that do not hash to the bound value, is a service or transport fault (retried, never a
//!   basis to honor a void: C.4 stage 9(a)). Bytes that do hash to it and then fail the outer AEAD or
//!   the descriptor are the producer's fault (stage 9(b), voidable). Only a verified artifact reaches
//!   the closure check.
//! * [`materialize_attachment`] is the receiver's step for an attachment: derive its own DEK-scoped
//!   `StorageId_B`, generate a fresh content key, re-encrypt the bytes and wrap the key under the DEK.
//!   No origin storage id, wrapped key or nonce is read from, or written to, anything that crosses
//!   the wire.

use sha2::Digest;
use sync_core::artifact::{ArtifactId, ArtifactReceipt, LogicalBinding, MAX_ARTIFACT_CONTAINER};
use sync_core::artifact_closure::Mismatch;
use sync_core::artifact_transport::{ArtifactTransport, TransportError};
use thiserror::Error;

use crate::attachment::{
    encrypt_blob, generate_content_key, storage_id, wrap_content_key, EncryptedBlob, StorageId,
    WrappedContentKey,
};
use crate::envelope::Dek;
use crate::sync_artifact::{
    open_artifact, seal_artifact, Fixity, OpenArtifactError, OpenedArtifact, SealArtifactError,
    SealedArtifact,
};
use crate::sync_envelope::EpochKey;
use crate::VaultCryptoError;
use sync_core::artifact::TransportPlaintext;

/// Why an artifact was not published. Nothing the caller may reference came out of any of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum PublishError {
    /// The object could not be sealed (including `ArtifactDescriptorChanged`).
    #[error("{0}")]
    Seal(SealArtifactError),
    /// The service refused or failed the put (`ObjectConflict`, `PayloadTooLarge`, unavailable).
    #[error("{0}")]
    Transport(TransportError),
    /// The service answered with a receipt that is not the one these bytes produce. It is not
    /// recorded, so no reference can be built on it.
    #[error("the service's receipt does not match the sealed object")]
    ReceiptMismatch,
}

/// A published artifact: what the caller records beside its row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Published {
    /// The receipt, equal to the locally computed one.
    pub receipt: ArtifactReceipt,
    /// SHA-256 of the complete transport plaintext (the fixity to store at first publication).
    pub plaintext_digest: [u8; 32],
}

/// Seal `plaintext`, put it, and verify the receipt against the one computed from the sealed bytes.
///
/// # Errors
/// A [`PublishError`]. A retry after a crash rebuilds identical bytes (encryption is deterministic),
/// so an exact retry gets the original receipt.
pub fn publish(
    transport: &impl ArtifactTransport,
    key: &EpochKey,
    plaintext: &TransportPlaintext<'_>,
    fixity: Fixity,
) -> Result<Published, PublishError> {
    let sealed = seal_artifact(key, plaintext, fixity).map_err(PublishError::Seal)?;
    put_sealed(transport, key, &plaintext.artifact_id, &sealed)
}

/// Put an already sealed artifact and verify the receipt against the one computed from its bytes.
/// [`publish`] is [`seal_artifact`] and this; a producer that records what it sealed **before** the
/// put (the transport plaintext digest, C.9) seals, records, then calls this.
///
/// # Errors
/// [`PublishError::Transport`] or [`PublishError::ReceiptMismatch`].
pub fn put_sealed(
    transport: &impl ArtifactTransport,
    key: &EpochKey,
    artifact_id: &ArtifactId,
    sealed: &SealedArtifact,
) -> Result<Published, PublishError> {
    let answered = transport
        .put(
            key.vault_id(),
            key.key_epoch(),
            artifact_id,
            &sealed.container,
        )
        .map_err(PublishError::Transport)?;
    if answered != sealed.receipt {
        return Err(PublishError::ReceiptMismatch);
    }
    Ok(Published {
        receipt: sealed.receipt,
        plaintext_digest: sealed.plaintext_digest,
    })
}

/// Why an artifact is not (yet) held and verified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum FetchError {
    /// The service does not hold the id (`ArtifactMissing`). Retried; may end in the device state
    /// `ArtifactRebootstrapRequired`; never a basis to honor a void (C.4 stage 9(a)).
    #[error("the service has no object at this address")]
    Missing,
    /// The service could not be reached. Retried.
    #[error("the service is unavailable")]
    Unavailable,
    /// The fetched bytes are not the artifact the envelope bound to this id: they are not a
    /// well-formed container, or their ciphertext does not hash to the bound `ciphertext_sha256`.
    /// A fault of the service or the transport, the same class as `Missing` (C.4 stage 9(a)).
    #[error("the fetched bytes are not the artifact the envelope bound to this id")]
    HashMismatch,
    /// The bytes hash to the bound value but fail the **outer** AEAD: the producer bound a hash of an
    /// object that does not open, which is its fault (C.4 stage 9(b)).
    #[error("the bound object does not authenticate")]
    OuterAead,
    /// The object authenticates but its plaintext is invalid, or names another address: a fact about
    /// the artifact, not about the service. Carried to the closure check as a mismatch.
    #[error("{0}")]
    Invalid(OpenArtifactError),
}

impl FetchError {
    /// The stage-9(b) verdict this failure stands for, if it is a fact about the artifact. Bytes
    /// that hash to the bound value are one fixed object that every replica judges alike, so a
    /// failure to open it ([`FetchError::OuterAead`]) or to validate it ([`FetchError::Invalid`]) is
    /// the producer's. Every other class is a service or transport fault and leaves the envelope not
    /// determinable (stage 9(a)), so it has no verdict.
    #[must_use]
    pub fn mismatch(self, id: ArtifactId) -> Option<Mismatch> {
        match self {
            Self::Invalid(_) | Self::OuterAead => Some(Mismatch::ArtifactInvalid(id)),
            Self::Missing | Self::Unavailable | Self::HashMismatch => None,
        }
    }
}

/// Get the object at `(key_epoch, id)`, check its ciphertext against `bound_sha256` (the hash the
/// envelope bound to the id), and open it from the expected address.
///
/// # Errors
/// A [`FetchError`].
pub fn fetch(
    transport: &impl ArtifactTransport,
    key: &EpochKey,
    id: &ArtifactId,
    bound_sha256: &[u8; 32],
) -> Result<OpenedArtifact, FetchError> {
    let container = transport
        .get(key.vault_id(), key.key_epoch(), id)
        .map_err(|e| match e {
            TransportError::NotFound => FetchError::Missing,
            _ => FetchError::Unavailable,
        })?;
    // The bytes the envelope bound are the AEAD output a container carries. Bytes that are not a
    // container, or whose ciphertext hashes to something else, are not them.
    let ciphertext = sync_core::artifact::parse_container(&container, MAX_ARTIFACT_CONTAINER)
        .map_err(|_| FetchError::HashMismatch)?;
    if sha2::Sha256::digest(ciphertext).as_slice() != bound_sha256 {
        return Err(FetchError::HashMismatch);
    }
    open_artifact(key, id, &container, MAX_ARTIFACT_CONTAINER).map_err(|e| match e {
        OpenArtifactError::Container(_) | OpenArtifactError::NotOpenable => FetchError::OuterAead,
        other => FetchError::Invalid(other),
    })
}

/// An attachment as the receiver stores it: its own address, its own content key (wrapped under its
/// own DEK) and the re-encrypted bytes. Nothing in it came from the origin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalAttachment {
    /// The attachment's row id.
    pub attachment_id: [u8; 16],
    /// `StorageId_B`: the keyed hash of the bytes under the receiver's DEK.
    pub storage_id: StorageId,
    /// The fresh content key, wrapped under the receiver's DEK.
    pub wrapped_key: WrappedContentKey,
    /// The bytes encrypted under that content key.
    pub blob: EncryptedBlob,
}

/// Why an artifact was not materialized as an attachment.
#[derive(Debug, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum MaterializeError {
    /// The artifact is not an attachment, or its metadata names no attachment id.
    #[error("the artifact is not a valid attachment")]
    NotAnAttachment,
    /// Local encryption failed.
    #[error("local encryption failed")]
    Crypto(VaultCryptoError),
}

/// Re-encrypt a verified attachment artifact for this device: `StorageId_B` under `dek`, a fresh
/// content key, the bytes encrypted under it, the key wrapped under `dek`. The caller writes the blob
/// file (temp, fsync, rename), then the mapping row in one transaction, and removes the blob if the
/// transaction fails (D7).
///
/// # Errors
/// A [`MaterializeError`].
pub fn materialize_attachment(
    dek: &Dek,
    opened: &OpenedArtifact,
) -> Result<LocalAttachment, MaterializeError> {
    let plaintext = opened.plaintext();
    // The binding is read according to the artifact's kind, so only an attachment with a valid
    // `attachment_id` yields `LogicalBinding::Attachment`.
    let Ok(LogicalBinding::Attachment { attachment_id }) = plaintext.binding() else {
        return Err(MaterializeError::NotAnAttachment);
    };
    let content_key = generate_content_key().map_err(MaterializeError::Crypto)?;
    let blob = encrypt_blob(&content_key, plaintext.blob).map_err(MaterializeError::Crypto)?;
    let wrapped_key = wrap_content_key(dek, &content_key).map_err(MaterializeError::Crypto)?;
    Ok(LocalAttachment {
        attachment_id,
        storage_id: storage_id(dek, plaintext.blob),
        wrapped_key,
        blob,
    })
}
