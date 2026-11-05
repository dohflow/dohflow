//! Publishing and fetching class-1b artifacts, and the receiver's local re-encryption
//! (`docs/architecture/vault-format.md` C.9; personal-cfo-0g528.2, part S2c).
//!
//! * [`publish`] seals an artifact, puts it through an [`ArtifactTransport`] and accepts the
//!   service's receipt **only if it equals the receipt computed locally** from the bytes that were
//!   sealed. A matching receipt is what lets a producer reference the artifact (receipt before
//!   reference, `sync_core::artifact_closure::build_delta`). Nothing here persists: the caller writes
//!   the row first and records the receipt after, in one transaction of its own.
//! * [`fetch`] gets an object and opens it from the **expected** address. A missing or unreachable
//!   object is a service fault (retried, never a basis to honor a void); an object that fails the
//!   outer AEAD is the same class (C.4 stage 9(a)); only a verified artifact reaches the closure
//!   check.
//! * [`materialize_attachment`] is the receiver's step for an attachment: derive its own DEK-scoped
//!   `StorageId_B`, generate a fresh content key, re-encrypt the bytes and wrap the key under the DEK.
//!   No origin storage id, wrapped key or nonce is read from, or written to, anything that crosses
//!   the wire.

use sync_core::artifact::{
    ArtifactId, ArtifactKind, ArtifactReceipt, LogicalBinding, MAX_ARTIFACT_CONTAINER,
};
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
    let SealedArtifact {
        container,
        receipt,
        plaintext_digest,
    } = seal_artifact(key, plaintext, fixity).map_err(PublishError::Seal)?;
    let answered = transport
        .put(
            key.vault_id(),
            key.key_epoch(),
            &plaintext.artifact_id,
            &container,
        )
        .map_err(PublishError::Transport)?;
    if answered != receipt {
        return Err(PublishError::ReceiptMismatch);
    }
    Ok(Published {
        receipt,
        plaintext_digest,
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
    /// The bytes fail the **outer** AEAD (or are not a container): a service or transport fault, the
    /// same class as `Missing`.
    #[error("the fetched object does not authenticate")]
    OuterAead,
    /// The object authenticates but its plaintext is invalid, or names another address: a fact about
    /// the artifact, not about the service. Carried to the closure check as a mismatch.
    #[error("{0}")]
    Invalid(OpenArtifactError),
}

/// Get the object at `(key_epoch, id)` and open it from the expected address.
///
/// # Errors
/// A [`FetchError`].
pub fn fetch(
    transport: &impl ArtifactTransport,
    key: &EpochKey,
    id: &ArtifactId,
) -> Result<OpenedArtifact, FetchError> {
    let container = transport
        .get(key.vault_id(), key.key_epoch(), id)
        .map_err(|e| match e {
            TransportError::NotFound => FetchError::Missing,
            _ => FetchError::Unavailable,
        })?;
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
    if plaintext.kind != ArtifactKind::Attachment {
        return Err(MaterializeError::NotAnAttachment);
    }
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
