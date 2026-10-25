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
//! The order the service applies them is C.10's, and it matters: **parse, then the replay lookup,
//! then the rest**. [`parse_push`] does the minimum the lookup needs (the strict parse, the size
//! cap, the envelope type, the routed vault, and the digest computed from the received bytes) and
//! returns the identity for [`crate::accepted::decide_push`]. Only for an identity the index does
//! not know does [`check_new_push`] run the checks that depend on the chain and the epoch in force:
//! an authorized member, the derived nonce domain, and the epoch. An exact retry of an accepted
//! envelope must return its original `seq` even if its device has since been revoked or replaced
//! (after `NonceStateLost`, or a restore), and even with an old `key_epoch`.
//!
//! The nonce-reuse backstop of C.4 (the first `(key_epoch, fingerprint, object_type, nonce)`
//! the service sees) is service state, not a property of one push; it belongs with the index, in
//! the Sync server (`personal-cfo-v98vd`), and runs in the same "new identities only" step.

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
    /// The key epoch in force.
    pub epoch_in_force: u64,
}

/// A pushed envelope after the minimum parse: its identity (digest computed from the received
/// bytes) and its header, for [`check_new_push`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedPush {
    /// The identity for the replay lookup.
    pub identity: PushIdentity,
    /// The decoded header.
    pub header: ClearHeader,
}

/// The first step of a push: the strict parse and size cap, the envelope type, the routed vault,
/// and the identity with its digest computed from the received bytes. Everything the replay
/// lookup needs and nothing that depends on the chain or the epoch.
///
/// # Errors
/// A [`PushRefusal`].
pub fn parse_push(
    container_bytes: &[u8],
    routed_vault: &[u8; 16],
) -> Result<ParsedPush, PushRefusal> {
    let container = parse_container_with_limit(container_bytes, MAX_ENVELOPE_CONTAINER_LEN)
        .map_err(PushRefusal::Malformed)?;
    if container.header.object_type() != OBJECT_ENVELOPE {
        return Err(PushRefusal::NotAnEnvelope);
    }
    if container.header.vault_id != *routed_vault {
        return Err(PushRefusal::WrongVault);
    }
    let identity = PushIdentity::from_container(&container).map_err(PushRefusal::Identity)?;
    Ok(ParsedPush {
        identity,
        header: container.header,
    })
}

/// The checks for an identity the index does **not** know ([`crate::accepted::PushDecision::
/// Unknown`]): the sealing device and key are one currently authorized member, the nonce domain is
/// the derived one, and the epoch is the epoch in force (`EpochRejected`).
///
/// # Errors
/// A [`PushRefusal`].
pub fn check_new_push(header: &ClearHeader, view: &PushView<'_>) -> Result<(), PushRefusal> {
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
    if header.key_epoch != view.epoch_in_force {
        return Err(PushRefusal::EpochRejected);
    }
    Ok(())
}
