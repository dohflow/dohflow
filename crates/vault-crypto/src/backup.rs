//! Backup payload sealing (ADR 0024, personal-cfo-ef3).
//!
//! The encrypted-backup container's payload (manifest + vault envelope + DB +
//! attachment blobs) is AES-256-GCM-sealed under a random per-backup **DEK**,
//! which is itself wrapped under a **backup KEK** derived from the unlocked
//! vault DEK with HKDF-SHA256 and a dedicated domain (ADR 0024-A). The backup
//! DEK comes from [`crate::generate_dek`] / [`crate::wrap_dek`]; this module
//! also provides the **payload AEAD**.
//!
//! Container assembly (the plaintext header, the manifest, content hashes, and
//! bundling the vault files) lives in the backup module that can read the vault
//! on disk — this crate stays pure (no filesystem).

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::Zeroize;

use crate::envelope::Dek;
use crate::VaultCryptoError;

/// AES-GCM nonce length, in bytes (96-bit).
const NONCE_LEN: usize = 12;

/// Domain-separated from attachment addressing and every other DEK-derived key.
const BACKUP_KEK_INFO: &[u8] = b"dohflow-backup-kek-v2";

/// Derive the per-backup KEK from the unlocked vault DEK.
///
/// The 16-byte salt must be fresh for every backup. HKDF's info value is a
/// cryptographic domain boundary and must not be reused for another purpose.
/// The returned key is protected and zeroized on drop like an Argon2-derived
/// [`crate::Kek`].
pub fn derive_backup_kek(
    dek: &crate::Dek,
    salt: &[u8; crate::SALT_LEN],
) -> Result<crate::Kek, crate::VaultCryptoError> {
    let hkdf = Hkdf::<Sha256>::new(Some(salt), dek.expose_bytes());
    let mut bytes = [0u8; crate::KEK_LEN];
    if hkdf.expand(BACKUP_KEK_INFO, &mut bytes).is_err() {
        bytes.zeroize();
        return Err(crate::VaultCryptoError::Derivation);
    }
    Ok(crate::Kek::from_array(bytes))
}

/// AES-256-GCM-sealed bytes: ciphertext (with auth tag) + the nonce used. Not
/// secret — this is what the backup container stores as its payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SealedPayload {
    /// The per-seal random AES-GCM nonce.
    pub nonce: [u8; NONCE_LEN],
    /// AES-256-GCM ciphertext of the payload (includes the authentication tag).
    pub ciphertext: Vec<u8>,
}

/// Seal `plaintext` under `dek` with AES-256-GCM and a fresh random nonce.
///
/// # Errors
/// [`VaultCryptoError::Rng`] if the nonce cannot be sampled, or
/// [`VaultCryptoError::KeyWrap`] if the AEAD encryption fails.
pub fn seal(dek: &Dek, plaintext: &[u8]) -> Result<SealedPayload, VaultCryptoError> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(dek.expose_bytes()));
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::getrandom(&mut nonce).map_err(|_| VaultCryptoError::Rng)?;
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext)
        .map_err(|_| VaultCryptoError::KeyWrap)?;
    Ok(SealedPayload { nonce, ciphertext })
}

/// Open a [`SealedPayload`] under `dek`.
///
/// # Errors
/// [`VaultCryptoError::KeyUnwrap`] if the AEAD tag does not verify (wrong DEK ⇐
/// wrong password, or tampered ciphertext).
pub fn open(dek: &Dek, sealed: &SealedPayload) -> Result<Vec<u8>, VaultCryptoError> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(dek.expose_bytes()));
    cipher
        .decrypt(Nonce::from_slice(&sealed.nonce), sealed.ciphertext.as_ref())
        .map_err(|_| VaultCryptoError::KeyUnwrap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate_dek;

    #[test]
    fn seal_then_open_round_trips() {
        let dek = generate_dek().unwrap();
        let payload = b"manifest + vault.db + blobs, as one payload";
        let sealed = seal(&dek, payload).unwrap();
        assert_ne!(sealed.ciphertext, payload);
        assert_eq!(open(&dek, &sealed).unwrap(), payload);
    }

    #[test]
    fn open_with_wrong_dek_fails() {
        let sealed = seal(&generate_dek().unwrap(), b"secret").unwrap();
        assert!(matches!(
            open(&generate_dek().unwrap(), &sealed),
            Err(VaultCryptoError::KeyUnwrap)
        ));
    }

    #[test]
    fn each_seal_uses_a_fresh_nonce() {
        let dek = generate_dek().unwrap();
        let a = seal(&dek, b"x").unwrap();
        let b = seal(&dek, b"x").unwrap();
        assert_ne!(a.nonce, b.nonce);
        assert_ne!(a.ciphertext, b.ciphertext);
    }

    #[test]
    fn tampered_ciphertext_is_rejected() {
        let dek = generate_dek().unwrap();
        let mut sealed = seal(&dek, b"sensitive").unwrap();
        sealed.ciphertext[0] ^= 0xff;
        assert!(matches!(
            open(&dek, &sealed),
            Err(VaultCryptoError::KeyUnwrap)
        ));
    }

    #[test]
    fn empty_payload_round_trips() {
        let dek = generate_dek().unwrap();
        let sealed = seal(&dek, b"").unwrap();
        assert_eq!(open(&dek, &sealed).unwrap(), b"");
    }
}
