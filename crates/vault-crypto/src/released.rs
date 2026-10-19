//! The unlock material a platform biometric store releases (personal-cfo-7ie.10, ADR 0002
//! decision 6).
//!
//! Touch ID and the phone's biometric unlock keep a *wrapped* copy of the vault's DEK in the
//! platform keystore. When the user authenticates, the platform layer (Rust, never the
//! WebView) decrypts that item into a [`SecretBytes`] and hands it to the kernel, which
//! checks it against the vault's envelope **before** SQLCipher is keyed. This module is the
//! pure part: the released plaintext's format, its parser, and the wrapped-DEK fingerprint.
//!
//! # Released plaintext, version 1 (81 bytes, fixed)
//!
//! ```text
//! offset  len  field
//!      0    1  version        = 1
//!      1   16  vault_id       the vault's Sync vault_id (UUID, raw bytes)
//!     17   32  fingerprint    dek_fingerprint(wrapped DEK on disk at enrollment)
//!     49   32  dek            the data-encrypting key (the SQLCipher raw key)
//! ```
//!
//! # Fingerprint
//!
//! `fingerprint = SHA-256("dohflow/dek-fingerprint/v1" ‖ nonce(12) ‖ ciphertext)` over the
//! wrapped DEK exactly as it sits in the sidecar envelope. A password change re-wraps the
//! DEK under a new KEK with a fresh nonce, and a rekey (ADR 0083) changes the DEK itself,
//! so either changes the fingerprint and the old item stops matching. Known-answer vector:
//! `nonce = 00 01 .. 0b`, `ciphertext = a0 a1 .. cf` (48 bytes) gives
//! `b701053f4fa0aae083aa0c20477413151e2b7dea1ddee0012f84fce5f1644842`.

use sha2::{Digest, Sha256};

use crate::envelope::DEK_LEN;
use crate::{Dek, SecretBytes, WrappedDek};

/// The released-plaintext format version this build reads.
pub const RELEASED_VERSION: u8 = 1;

/// Length of the released plaintext, in bytes.
pub const RELEASED_LEN: usize = 1 + 16 + 32 + DEK_LEN;

/// The domain-separation label of the wrapped-DEK fingerprint.
pub const DEK_FINGERPRINT_LABEL: &[u8] = b"dohflow/dek-fingerprint/v1";

/// The fingerprint of a wrapped DEK, as stored in the released plaintext.
#[must_use]
pub fn dek_fingerprint(wrapped: &WrappedDek) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(DEK_FINGERPRINT_LABEL);
    hasher.update(wrapped.nonce);
    hasher.update(&wrapped.ciphertext);
    hasher.finalize().into()
}

/// Why released material could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReleasedParseError {
    /// The version byte is not [`RELEASED_VERSION`]. The platform item was written by a
    /// build this one cannot read, so the caller deletes it and asks for the password.
    UnknownVersion(u8),
}

/// Parsed released material. The DEK is held in a protected allocation, and `Debug`
/// shows no byte of it.
pub struct ReleasedUnlockMaterial {
    /// The vault's Sync `vault_id`, raw UUID bytes.
    pub vault_id: [u8; 16],
    /// The wrapped-DEK fingerprint recorded at enrollment.
    pub fingerprint: [u8; 32],
    /// The data-encrypting key.
    pub dek: Dek,
}

impl core::fmt::Debug for ReleasedUnlockMaterial {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ReleasedUnlockMaterial([REDACTED])")
    }
}

impl ReleasedUnlockMaterial {
    /// Parse the released plaintext. The length is fixed by the type, so only the version
    /// can be wrong.
    ///
    /// # Errors
    /// [`ReleasedParseError::UnknownVersion`] for a version other than 1.
    pub fn parse(released: &SecretBytes<RELEASED_LEN>) -> Result<Self, ReleasedParseError> {
        let bytes = released.expose();
        if bytes[0] != RELEASED_VERSION {
            return Err(ReleasedParseError::UnknownVersion(bytes[0]));
        }
        let mut vault_id = [0_u8; 16];
        vault_id.copy_from_slice(&bytes[1..17]);
        let mut fingerprint = [0_u8; 32];
        fingerprint.copy_from_slice(&bytes[17..49]);
        // The DEK is copied straight from the protected buffer into its own protected
        // allocation: no stack array ever holds it (personal-cfo-xzpra).
        let dek = Dek::from_slice(&bytes[49..49 + DEK_LEN])
            .expect("the slice is DEK_LEN bytes by construction");
        Ok(Self {
            vault_id,
            fingerprint,
            dek,
        })
    }

    /// Build the released plaintext for enrollment: what the platform layer seals into its
    /// keystore after a successful password unlock.
    #[must_use]
    pub fn encode(
        vault_id: [u8; 16],
        wrapped: &WrappedDek,
        dek: &Dek,
    ) -> SecretBytes<RELEASED_LEN> {
        // Built in place in the protected buffer, so the sealed plaintext never exists in an
        // unscrubbed stack array.
        SecretBytes::init::<core::convert::Infallible>(|out| {
            out[0] = RELEASED_VERSION;
            out[1..17].copy_from_slice(&vault_id);
            out[17..49].copy_from_slice(&dek_fingerprint(wrapped));
            out[49..].copy_from_slice(dek.expose_bytes());
            Ok(())
        })
        .unwrap_or_else(|never| match never {})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wrapped() -> WrappedDek {
        WrappedDek {
            nonce: core::array::from_fn(|i| u8::try_from(i).unwrap()),
            ciphertext: (0..48_u8).map(|i| 0xA0 + i).collect(),
        }
    }

    #[test]
    fn the_fingerprint_matches_the_independent_known_answer() {
        assert_eq!(
            hex(&dek_fingerprint(&wrapped())),
            "b701053f4fa0aae083aa0c20477413151e2b7dea1ddee0012f84fce5f1644842"
        );
    }

    #[test]
    fn the_fingerprint_changes_with_the_nonce_or_the_ciphertext() {
        let base = dek_fingerprint(&wrapped());
        let mut other = wrapped();
        other.nonce[0] ^= 1;
        assert_ne!(dek_fingerprint(&other), base);
        let mut other = wrapped();
        other.ciphertext[47] ^= 1;
        assert_ne!(dek_fingerprint(&other), base);
        // The label separates this hash from a bare hash of the same bytes.
        let mut bare = Sha256::new();
        bare.update(wrapped().nonce);
        bare.update(wrapped().ciphertext);
        assert_ne!(<[u8; 32]>::from(bare.finalize()), base);
    }

    #[test]
    fn released_material_has_the_documented_layout_and_round_trips() {
        let dek = {
            let bytes: [u8; DEK_LEN] = core::array::from_fn(|i| 0x40 + u8::try_from(i).unwrap());
            Dek::from_slice(&bytes).unwrap()
        };
        let vault_id: [u8; 16] = [
            0x01, 0x98, 0xc2, 0xc1, 0x5f, 0x0a, 0x7c, 0x3d, 0x8a, 0x4e, 0x9b, 0x1d, 0, 0, 0, 0x11,
        ];
        let sealed = ReleasedUnlockMaterial::encode(vault_id, &wrapped(), &dek);
        assert_eq!(
            hex(sealed.expose()),
            "010198c2c15f0a7c3d8a4e9b1d00000011b701053f4fa0aae083aa0c20477413151e2b7dea1ddee0012f84fce5f1644842404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f"
        );
        assert_eq!(sealed.expose().len(), RELEASED_LEN);
        let parsed = ReleasedUnlockMaterial::parse(&sealed).unwrap();
        assert_eq!(parsed.vault_id, vault_id);
        assert_eq!(parsed.fingerprint, dek_fingerprint(&wrapped()));
        assert_eq!(parsed.dek.expose_bytes(), dek.expose_bytes());
    }

    #[test]
    fn an_unknown_version_is_refused_and_debug_shows_no_key() {
        for version in [0_u8, 2, 255] {
            let mut bytes = [7_u8; RELEASED_LEN];
            bytes[0] = version;
            let sealed = SecretBytes::from_slice(&bytes).unwrap();
            assert_eq!(
                ReleasedUnlockMaterial::parse(&sealed).unwrap_err(),
                ReleasedParseError::UnknownVersion(version)
            );
        }
        let mut bytes = [0xAB_u8; RELEASED_LEN];
        bytes[0] = RELEASED_VERSION;
        let parsed =
            ReleasedUnlockMaterial::parse(&SecretBytes::from_slice(&bytes).unwrap()).unwrap();
        let shown = format!("{parsed:?} {:?}", parsed.dek);
        assert!(!shown.to_lowercase().contains("ab"), "{shown}");
        assert!(shown.contains("REDACTED"));
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
}
