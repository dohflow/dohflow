//! What the Sync service can check about a pushed envelope without reading it
//! (`docs/architecture/vault-format.md` C.4 "Push and producer rules"; personal-cfo-g3m.17).
//!
//! The service holds the membership chain and the epoch in force, so a push can be refused at the
//! door for everything that depends on the header and the chain alone: the grammar, the vault, the
//! epoch, the sealing device and key, and the nonce domain. What it cannot check is anything that
//! needs the plaintext (the idempotency tag, the metadata commitment, `object_id` against
//! `command_id`, the manifest commitment): that is the producer's seal-then-verify
//! ([`crate::producer`]), which together with these checks leaves no class of inconsistency that
//! both sides miss.
//!
//! The order the service applies them is C.10's: the replay lookup comes **before** the epoch
//! check, because an old `key_epoch` must not turn an exact retry into a rejection. So the epoch
//! rule is a separate function, [`check_epoch_in_force`], called only for an unknown identity.

use thiserror::Error;

use crate::accepted::{IdentityError, PushIdentity};
use crate::cbe::WireError;
use crate::header::{
    parse_container_with_limit, ClearHeader, NonceDomain, MAX_ENVELOPE_CONTAINER_LEN,
    OBJECT_ENVELOPE,
};
use crate::membership::VerifiedState;

/// Why a push is refused at the door.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum PushRefusal {
    /// The container or header is not well formed (grammar, dual-identity shape, counter cap,
    /// `engine_version` grammar, ciphertext shorter than a tag, over the size cap). Unsupported
    /// protocol and container versions are distinguished by [`WireError::is_malformed`].
    #[error("the pushed object is not well formed: {0}")]
    Malformed(WireError),
    /// A snapshot header was pushed as a command envelope.
    #[error("the pushed object is not a command envelope")]
    NotAnEnvelope,
    /// The header names a vault other than the one the request was routed to.
    #[error("the pushed object names another vault")]
    WrongVault,
    /// The header's `device_id` and `device_key_fingerprint` are not one currently authorized
    /// member of the chain.
    #[error("the sealing device is not an authorized member")]
    UnknownDevice,
    /// The header's nonce domain is not the one derived for its epoch, device key and object type.
    #[error("the nonce domain is not the derived one")]
    NonceDomainMismatch,
    /// The header's `key_epoch` is not the epoch in force (`EpochRejected`).
    #[error("the key epoch is not the epoch in force")]
    EpochRejected,
    /// The identity could not be derived.
    #[error("the identity could not be derived: {0}")]
    Identity(IdentityError),
}

/// What the service knows when a push arrives.
#[derive(Debug, Clone, Copy)]
pub struct PushView<'a> {
    /// The vault the request was routed to (after credential authentication).
    pub routed_vault: [u8; 16],
    /// The verified membership chain and epoch records the service holds for that vault.
    pub chain: &'a VerifiedState,
}

/// The structural checks of C.4 on a pushed command envelope, **excluding the epoch rule** (see
/// the module docs for why it comes later). On success returns the push identity, with the digest
/// computed from the received bytes, ready for [`crate::accepted::decide_push`].
///
/// # Errors
/// A [`PushRefusal`].
pub fn check_push_structure(
    container_bytes: &[u8],
    view: &PushView<'_>,
) -> Result<PushIdentity, PushRefusal> {
    let container = parse_container_with_limit(container_bytes, MAX_ENVELOPE_CONTAINER_LEN)
        .map_err(PushRefusal::Malformed)?;
    let header = &container.header;
    if header.object_type() != OBJECT_ENVELOPE {
        return Err(PushRefusal::NotAnEnvelope);
    }
    if header.vault_id != view.routed_vault {
        return Err(PushRefusal::WrongVault);
    }
    let member = view
        .chain
        .authorized()
        .any(|m| m.device_id == header.device_id && m.fingerprint == header.device_key_fingerprint);
    if !member {
        return Err(PushRefusal::UnknownDevice);
    }
    let domain = NonceDomain::derive(
        &header.vault_id,
        header.key_epoch,
        &header.device_key_fingerprint,
        OBJECT_ENVELOPE,
    );
    if header.nonce_domain != domain {
        return Err(PushRefusal::NonceDomainMismatch);
    }
    PushIdentity::from_container(&container).map_err(PushRefusal::Identity)
}

/// The epoch rule (C.10 step 4): reject a `key_epoch` that is not the epoch in force. Call it only
/// for an identity [`crate::accepted::decide_push`] reported as unknown.
///
/// # Errors
/// [`PushRefusal::EpochRejected`].
pub fn check_epoch_in_force(header: &ClearHeader, epoch_in_force: u64) -> Result<(), PushRefusal> {
    if header.key_epoch == epoch_in_force {
        Ok(())
    } else {
        Err(PushRefusal::EpochRejected)
    }
}
