//! The proof a device makes to claim its Sync credential (`docs/architecture/sync-service-protocol.md`
//! 4.5; personal-cfo-v98vd AC26).
//!
//! A device that the membership chain authorizes proves it holds its signing key by signing a short
//! CBE1 body naming the vault, the device, its fingerprint and the time, and the service verifies
//! that signature **strictly, under the key the chain holds for the device** (never one the caller
//! supplies). The body and the check live here so that the client that builds the proof and the
//! service that checks it run the same code and cannot drift (the single-implementation rule the
//! void verdict also follows).
//!
//! The clock is not here. The service compares `issued_at` with its own clock (the protocol's
//! `claim_skew_seconds`); this module only builds and checks the signed bytes.

use thiserror::Error;

use crate::cbe::Writer;
use crate::header::Fingerprint;
use crate::keys::ed25519_verify_strict;
use crate::membership::VerifiedState;

/// The label of the claim body.
pub const CLAIM_LABEL: &str = "dohflow/sync-credential-claim/v1";

/// The body a device signs: `label ‖ uuid vault_id ‖ uuid device_id ‖ fingerprint ‖ i64 issued_at`
/// (CBE1, `vault-format.md` C.1.1; integers big-endian). `issued_at` is microseconds since the Unix
/// epoch (UTC).
#[must_use]
pub fn claim_body(
    vault_id: &[u8; 16],
    device_id: &[u8; 16],
    fingerprint: &Fingerprint,
    issued_at: i64,
) -> Vec<u8> {
    let mut w = Writer::with_label(CLAIM_LABEL);
    w.fixed(vault_id)
        .fixed(device_id)
        .fixed(fingerprint.as_bytes())
        .i64(issued_at);
    w.finish()
}

/// Why a claim was refused. Neither is a halt: a bad claim is ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ClaimError {
    /// The device is not currently authorized in the chain (never added, or revoked).
    #[error("the device is not currently authorized in the chain")]
    DeviceNotFound,
    /// The signature does not verify strictly under the chain's key for the device.
    #[error("the claim proof does not verify")]
    ProofInvalid,
}

/// Check a claim: `device_id` must be a device `state` currently authorizes, and `proof` must be
/// its Ed25519 signature, verified strictly under the public key the chain holds for it, over
/// [`claim_body`] for this vault, that device and its fingerprint, and `issued_at`.
///
/// # Errors
/// A [`ClaimError`].
pub fn verify_claim(
    state: &VerifiedState,
    device_id: &[u8; 16],
    issued_at: i64,
    proof: &[u8; 64],
) -> Result<(), ClaimError> {
    let member = state
        .authorized()
        .find(|m| &m.device_id == device_id)
        .ok_or(ClaimError::DeviceNotFound)?;
    let body = claim_body(
        state.vault_id(),
        &member.device_id,
        &member.fingerprint,
        issued_at,
    );
    if ed25519_verify_strict(&member.ed25519_pk, &body, proof) {
        Ok(())
    } else {
        Err(ClaimError::ProofInvalid)
    }
}
