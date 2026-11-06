//! The seam between the client and the Sync service's artifact endpoints
//! (`docs/architecture/sync-service-protocol.md` 4.10; personal-cfo-0g528.2, part S2c).
//!
//! [`ArtifactTransport`] is `putArtifact`, `getArtifact` and `getArtifactStatus`. **There is no
//! production implementor in this tree**: the service is `personal-cfo-v98vd` and the client engine
//! that owns the network is `personal-cfo-elciw`. The only implementor is the in-memory fake in the
//! vault-crypto tests, and a source-scan test fails any other until the engine supplies one, so no
//! code can start talking to a service by accident.

use thiserror::Error;

use crate::artifact::{ArtifactId, ArtifactReceipt};

/// Why the service did not do what was asked. A transport fault is never a statement about an
/// artifact; the client retries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum TransportError {
    /// A different byte string already exists for this `(key_epoch, artifact_id)` (`ObjectConflict`).
    #[error("a different object already exists at this address")]
    ObjectConflict,
    /// No object at this address (`ArtifactNotFound`).
    #[error("no object at this address")]
    NotFound,
    /// The container is larger than the service stores (`PayloadTooLarge`).
    #[error("the object is larger than the service stores")]
    PayloadTooLarge,
    /// The service could not be reached or answered with a server error.
    #[error("the service is unavailable")]
    Unavailable,
}

/// What `getArtifactStatus` says about one id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArtifactStatus {
    /// The id asked about.
    pub artifact_id: ArtifactId,
    /// Whether the service holds it.
    pub present: bool,
    /// Whether a retained snapshot or an accepted envelope references it.
    pub pinned: bool,
    /// The stored ciphertext's length and SHA-256, if present.
    pub receipt: Option<ArtifactReceipt>,
}

/// The artifact endpoints, from the client's side.
pub trait ArtifactTransport {
    /// `putArtifact`: store `container` at `(key_epoch, artifact_id)` and return the receipt. An exact
    /// retry returns the same receipt; different bytes for the address are
    /// [`TransportError::ObjectConflict`].
    ///
    /// # Errors
    /// A [`TransportError`].
    fn put(
        &self,
        vault_id: &[u8; 16],
        key_epoch: u64,
        artifact_id: &ArtifactId,
        container: &[u8],
    ) -> Result<ArtifactReceipt, TransportError>;

    /// `getArtifact`: the container bytes.
    ///
    /// # Errors
    /// [`TransportError::NotFound`] or [`TransportError::Unavailable`].
    fn get(
        &self,
        vault_id: &[u8; 16],
        key_epoch: u64,
        artifact_id: &ArtifactId,
    ) -> Result<Vec<u8>, TransportError>;

    /// `getArtifactStatus` for up to 4096 ids, in request order.
    ///
    /// # Errors
    /// [`TransportError::Unavailable`].
    fn status(
        &self,
        vault_id: &[u8; 16],
        key_epoch: u64,
        ids: &[ArtifactId],
    ) -> Result<Vec<ArtifactStatus>, TransportError>;
}
