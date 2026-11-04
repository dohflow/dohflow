//! The key material of Sync (`docs/architecture/vault-format.md` C.2, C.3 and C.6;
//! personal-cfo-1cltt.2): the epoch root `K_e`, the tag secret `S`, their commitments, and a
//! device's own signing and agreement keys.
//!
//! * [`generate_secret`] draws 32 bytes from the operating system's random source straight into a
//!   protected buffer. `K_e` and `S` are each one such draw, and a failed draw is an error, never a
//!   zero key.
//! * [`epoch_key_commitment`] and [`tag_key_commitment`] are the two commitments an epoch record
//!   carries. They are the **only** place either is computed: [`crate::sync_envelope::EpochKey`]
//!   and [`crate::sync::TagSecret`] are built through them, so a key that failed its commitment
//!   cannot be used by type.
//! * [`DeviceKeys`] is a device's Ed25519 signing key and its X25519 agreement key. They are
//!   generated independently and are not derived from a common seed (C.2). The private halves
//!   live in [`SecretBytes`]; no type here derives `Debug` over them or exposes them except to the
//!   sibling modules that sign and unseal.
//!
//! Nothing in this module logs, formats or returns private bytes in an error.

use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};
use sync_core::header::Fingerprint;
use sync_core::keys::{ed25519_public_key_acceptable, x25519_public_key_acceptable};
use sync_core::membership::{device_fingerprint, KEM_ID, SUITE_ID};
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroize;

use crate::secret::SecretBytes;
use crate::sync::label_bytes;
use crate::VaultCryptoError;

/// The label of the epoch-key commitment.
pub const EPOCH_KEY_COMMITMENT_LABEL: &str = "dohflow/sync-epoch-key-commit/v1";
/// The label of the tag-secret commitment.
pub const TAG_KEY_COMMITMENT_LABEL: &str = "dohflow/sync-tag-key-commit/v1";

/// 32 fresh random bytes from the operating system, in a protected buffer. Used for `K_e`, `S` and
/// the two device private keys.
///
/// # Errors
/// [`VaultCryptoError::Rng`] if the system source fails. No partial or zero key is returned.
pub fn generate_secret() -> Result<SecretBytes<32>, VaultCryptoError> {
    SecretBytes::<32>::init(|out| getrandom::getrandom(out).map_err(|_| VaultCryptoError::Rng))
}

fn commit(label: &str, key: &SecretBytes<32>) -> [u8; 32] {
    // Hash incrementally: building `label ‖ key` in a Vec would leave an unscrubbed copy of the
    // key on the heap (and a second one if it reallocated).
    let mut hash = Sha256::new();
    hash.update(label_bytes(label));
    hash.update(key.expose());
    hash.finalize().into()
}

/// `SHA-256(label "dohflow/sync-epoch-key-commit/v1" ‖ key32 K_e)` (C.3).
#[must_use]
pub fn epoch_key_commitment(key: &SecretBytes<32>) -> [u8; 32] {
    commit(EPOCH_KEY_COMMITMENT_LABEL, key)
}

/// `SHA-256(label "dohflow/sync-tag-key-commit/v1" ‖ key32 S)` (C.3).
#[must_use]
pub fn tag_key_commitment(secret: &SecretBytes<32>) -> [u8; 32] {
    commit(TAG_KEY_COMMITMENT_LABEL, secret)
}

/// Why a device's keys were not produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum DeviceKeyError {
    /// The system random source failed.
    #[error("the system random source failed")]
    Rng,
    /// The signing key's public key is one the chain would refuse (C.6 key acceptance). With a
    /// random key this is astronomically unlikely, and with a stored one it means the stored bytes
    /// are damaged.
    #[error("the signing key's public key is not acceptable to the membership chain")]
    UnacceptableSigningKey,
    /// The agreement key's public key is one the chain would refuse (see above).
    #[error("the agreement key's public key is not acceptable to the membership chain")]
    UnacceptableAgreementKey,
}

impl From<VaultCryptoError> for DeviceKeyError {
    fn from(_: VaultCryptoError) -> Self {
        Self::Rng
    }
}

/// A device's public keys and the fingerprint that commits to both (C.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DevicePublic {
    /// The Ed25519 verifying key.
    pub ed25519_pk: [u8; 32],
    /// The X25519 agreement public key (the suite-1 KEM key).
    pub kem_public_key: [u8; 32],
    /// `SHA-256(label ‖ suite ‖ kem ‖ ed25519_pk ‖ kem_public_key)`.
    pub fingerprint: Fingerprint,
}

/// A device's own keys: an Ed25519 signing key and an X25519 agreement key, independent of each
/// other (C.2). They never leave the device and never cross the IPC boundary.
pub struct DeviceKeys {
    signing: SecretBytes<32>,
    agreement: SecretBytes<32>,
}

impl DeviceKeys {
    /// Generate a fresh pair of independent keys.
    ///
    /// # Errors
    /// [`DeviceKeyError`].
    pub fn generate() -> Result<Self, DeviceKeyError> {
        Self::from_secrets(generate_secret()?, generate_secret()?)
    }

    /// Rebuild a device's keys from the two private values a store held. Both public keys are
    /// re-screened, so damaged storage cannot become an enrollment.
    ///
    /// # Errors
    /// [`DeviceKeyError::UnacceptableSigningKey`] or [`DeviceKeyError::UnacceptableAgreementKey`].
    pub fn from_secrets(
        signing: SecretBytes<32>,
        agreement: SecretBytes<32>,
    ) -> Result<Self, DeviceKeyError> {
        let keys = Self { signing, agreement };
        let public = keys.public();
        if !ed25519_public_key_acceptable(&public.ed25519_pk) {
            return Err(DeviceKeyError::UnacceptableSigningKey);
        }
        if !x25519_public_key_acceptable(&public.kem_public_key) {
            return Err(DeviceKeyError::UnacceptableAgreementKey);
        }
        Ok(keys)
    }

    /// The Ed25519 seed. For the custody module that stores it, and nothing else.
    pub(crate) const fn signing_secret(&self) -> &SecretBytes<32> {
        &self.signing
    }

    /// The X25519 private key. For the custody module and the unsealer, and nothing else.
    pub(crate) const fn agreement_secret(&self) -> &SecretBytes<32> {
        &self.agreement
    }

    /// The public keys and the fingerprint.
    #[must_use]
    pub fn public(&self) -> DevicePublic {
        let ed25519_pk = SigningKey::from_bytes(self.signing.expose())
            .verifying_key()
            .to_bytes();
        let mut scalar = *self.agreement.expose();
        let kem_public_key = PublicKey::from(&StaticSecret::from(scalar)).to_bytes();
        scalar.zeroize();
        DevicePublic {
            ed25519_pk,
            kem_public_key,
            fingerprint: device_fingerprint(SUITE_ID, KEM_ID, &ed25519_pk, &kem_public_key),
        }
    }

    /// An Ed25519 signature over `message` (RFC 8032, deterministic).
    #[must_use]
    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        SigningKey::from_bytes(self.signing.expose())
            .sign(message)
            .to_bytes()
    }
}

impl core::fmt::Debug for DeviceKeys {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("DeviceKeys([REDACTED])")
    }
}
