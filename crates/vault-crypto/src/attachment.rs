//! Attachment cryptography (ADR 0023, personal-cfo-bcj).
//!
//! Realizes ADR 0002 §5's per-attachment key model. Each attachment's bytes are
//! encrypted with a fresh random **content key** (AES-256-GCM); that content key
//! is wrapped under the vault **DEK** and stored as metadata. The on-disk blob
//! filename — the **storage ID** — is a *keyed* content hash: `HMAC-SHA256` over
//! the plaintext under a subkey `HKDF`-derived from the DEK. This gives dedup
//! *within a vault* (identical bytes → identical storage ID) without an on-disk
//! name an attacker could match against a guessed document.
//!
//! Pure crate: no `rusqlite`, async runtime, Tauri, or filesystem. The blob store
//! I/O and metadata live in `db-worker`; this module only transforms
//! bytes ⇄ ciphertext and derives IDs. AEAD is AES-256-GCM with a fresh random
//! 96-bit nonce per operation, matching the DEK envelope (ADR 0002).

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use zeroize::Zeroize;

use crate::envelope::Dek;
use crate::{SecretBytes, VaultCryptoError};

/// Length of an attachment content key, in bytes (256-bit, AES-256-GCM).
pub const CONTENT_KEY_LEN: usize = 32;

/// AES-GCM nonce length, in bytes (96-bit, the standard for GCM).
const NONCE_LEN: usize = 12;

/// HKDF `info` for the content-addressing subkey. Versioned for domain
/// separation: bump the suffix only if the addressing scheme changes.
const ADDR_INFO: &[u8] = b"personal-cfo/attachment-addressing/v1";

/// The opaque on-disk blob name: hex of `HMAC-SHA256(addr_subkey, plaintext)`.
/// Deterministic for a given vault + plaintext (so dedup works), but reveals
/// nothing about the plaintext without the DEK-derived subkey.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StorageId(String);

impl StorageId {
    /// The hex string used as the on-disk filename.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A per-attachment content key. Like [`Dek`]/[`crate::Kek`] it is backed by
/// [`SecretBytes`] (mlock'd, zeroize-on-drop, non-`Clone`) with a redacted
/// `Debug` (§6.6). Its raw bytes never leave this module.
pub struct ContentKey(SecretBytes<CONTENT_KEY_LEN>);

impl ContentKey {
    fn expose(&self) -> &[u8; CONTENT_KEY_LEN] {
        self.0.expose()
    }

    /// Move `bytes` into a protected allocation, scrubbing the caller's copy.
    fn from_array(mut bytes: [u8; CONTENT_KEY_LEN]) -> Self {
        let key = ContentKey(SecretBytes::new(bytes));
        bytes.zeroize();
        key
    }
}

impl Zeroize for ContentKey {
    fn zeroize(&mut self) {
        self.0.zeroize();
    }
}

impl core::fmt::Debug for ContentKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ContentKey([REDACTED])")
    }
}

/// A content key wrapped (AES-256-GCM) under the DEK. Ciphertext + nonce — not
/// secret; stored in the `attachments` metadata row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WrappedContentKey {
    /// The per-wrap random AES-GCM nonce.
    pub nonce: [u8; NONCE_LEN],
    /// AES-256-GCM ciphertext of the content key (includes the auth tag).
    pub ciphertext: Vec<u8>,
}

/// Encrypted attachment bytes + the nonce used. Ciphertext — not secret; this is
/// what `db-worker` writes to the `blobs/` file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncryptedBlob {
    /// The per-blob random AES-GCM nonce.
    pub nonce: [u8; NONCE_LEN],
    /// AES-256-GCM ciphertext of the attachment plaintext (includes the tag).
    pub ciphertext: Vec<u8>,
}

/// Derive the content-addressing subkey from the DEK via HKDF-SHA256. No salt is
/// used: the DEK is already a uniformly-random 256-bit key, so HKDF-Extract adds
/// nothing, and `ADDR_INFO` provides domain separation from any other DEK use.
fn addressing_subkey(dek: &Dek) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(None, dek.expose_bytes());
    let mut subkey = [0u8; 32];
    hk.expand(ADDR_INFO, &mut subkey)
        .expect("32 bytes is within HKDF-SHA256's 255*32 output limit");
    subkey
}

/// Compute the keyed-hash [`StorageId`] for `plaintext` under `dek`. Deterministic
/// per vault + plaintext (enables dedup); requires the unlocked DEK.
#[must_use]
pub fn storage_id(dek: &Dek, plaintext: &[u8]) -> StorageId {
    let mut subkey = addressing_subkey(dek);
    let mut mac =
        <Hmac<Sha256> as Mac>::new_from_slice(&subkey).expect("HMAC accepts a key of any length");
    mac.update(plaintext);
    let tag = mac.finalize().into_bytes();
    subkey.zeroize();
    StorageId(to_hex(&tag))
}

/// Generate a fresh random content key from the OS CSPRNG.
///
/// # Errors
/// [`VaultCryptoError::Rng`] if the OS random source is unavailable.
pub fn generate_content_key() -> Result<ContentKey, VaultCryptoError> {
    let mut bytes = [0u8; CONTENT_KEY_LEN];
    getrandom::getrandom(&mut bytes).map_err(|_| VaultCryptoError::Rng)?;
    Ok(ContentKey::from_array(bytes))
}

/// Encrypt `plaintext` under `key` with AES-256-GCM and a fresh random nonce.
///
/// # Errors
/// [`VaultCryptoError::Rng`] if the nonce cannot be sampled, or
/// [`VaultCryptoError::KeyWrap`] if the AEAD encryption fails.
pub fn encrypt_blob(key: &ContentKey, plaintext: &[u8]) -> Result<EncryptedBlob, VaultCryptoError> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key.expose()));
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::getrandom(&mut nonce).map_err(|_| VaultCryptoError::Rng)?;
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext)
        .map_err(|_| VaultCryptoError::KeyWrap)?;
    Ok(EncryptedBlob { nonce, ciphertext })
}

/// Decrypt an [`EncryptedBlob`] under `key`.
///
/// # Errors
/// [`VaultCryptoError::KeyUnwrap`] if the AEAD tag does not verify (wrong key or
/// tampered ciphertext).
pub fn decrypt_blob(key: &ContentKey, blob: &EncryptedBlob) -> Result<Vec<u8>, VaultCryptoError> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key.expose()));
    cipher
        .decrypt(Nonce::from_slice(&blob.nonce), blob.ciphertext.as_ref())
        .map_err(|_| VaultCryptoError::KeyUnwrap)
}

/// Wrap a content key under the `dek` (AES-256-GCM, fresh nonce) for storage.
///
/// # Errors
/// [`VaultCryptoError::Rng`] / [`VaultCryptoError::KeyWrap`] as for the DEK wrap.
pub fn wrap_content_key(
    dek: &Dek,
    key: &ContentKey,
) -> Result<WrappedContentKey, VaultCryptoError> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(dek.expose_bytes()));
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::getrandom(&mut nonce).map_err(|_| VaultCryptoError::Rng)?;
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), key.expose().as_ref())
        .map_err(|_| VaultCryptoError::KeyWrap)?;
    Ok(WrappedContentKey { nonce, ciphertext })
}

/// Unwrap a [`WrappedContentKey`] under the `dek`.
///
/// # Errors
/// [`VaultCryptoError::KeyUnwrap`] if the tag does not verify or the recovered
/// key is the wrong length.
pub fn unwrap_content_key(
    dek: &Dek,
    wrapped: &WrappedContentKey,
) -> Result<ContentKey, VaultCryptoError> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(dek.expose_bytes()));
    let mut plaintext = cipher
        .decrypt(
            Nonce::from_slice(&wrapped.nonce),
            wrapped.ciphertext.as_ref(),
        )
        .map_err(|_| VaultCryptoError::KeyUnwrap)?;
    if plaintext.len() != CONTENT_KEY_LEN {
        plaintext.zeroize();
        return Err(VaultCryptoError::KeyUnwrap);
    }
    let mut arr = [0u8; CONTENT_KEY_LEN];
    arr.copy_from_slice(&plaintext);
    plaintext.zeroize();
    Ok(ContentKey::from_array(arr))
}

/// Lower-case hex, no allocation beyond the output string.
fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate_dek;

    #[test]
    fn blob_encrypt_then_decrypt_round_trips() {
        let key = generate_content_key().unwrap();
        let plaintext = b"a bank statement PDF's bytes";
        let blob = encrypt_blob(&key, plaintext).unwrap();
        assert_ne!(blob.ciphertext, plaintext);
        assert_eq!(decrypt_blob(&key, &blob).unwrap(), plaintext);
    }

    #[test]
    fn content_key_wrap_then_unwrap_round_trips() {
        let dek = generate_dek().unwrap();
        let key = generate_content_key().unwrap();
        let plaintext = b"hello";
        let blob = encrypt_blob(&key, plaintext).unwrap();

        let wrapped = wrap_content_key(&dek, &key).unwrap();
        let recovered = unwrap_content_key(&dek, &wrapped).unwrap();
        // The recovered key decrypts the blob the original key encrypted.
        assert_eq!(decrypt_blob(&recovered, &blob).unwrap(), plaintext);
    }

    #[test]
    fn unwrap_with_wrong_dek_fails() {
        let dek = generate_dek().unwrap();
        let other = generate_dek().unwrap();
        let key = generate_content_key().unwrap();
        let wrapped = wrap_content_key(&dek, &key).unwrap();
        assert!(matches!(
            unwrap_content_key(&other, &wrapped),
            Err(VaultCryptoError::KeyUnwrap)
        ));
    }

    #[test]
    fn storage_id_is_deterministic_per_vault_and_plaintext() {
        let dek = generate_dek().unwrap();
        let a = storage_id(&dek, b"same bytes");
        let b = storage_id(&dek, b"same bytes");
        assert_eq!(a, b, "dedup relies on a stable id for identical bytes");
        // 32-byte HMAC → 64 hex chars.
        assert_eq!(a.as_str().len(), 64);
        assert!(a.as_str().bytes().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn storage_id_differs_for_different_plaintext() {
        let dek = generate_dek().unwrap();
        assert_ne!(storage_id(&dek, b"one"), storage_id(&dek, b"two"));
    }

    #[test]
    fn storage_id_is_vault_specific() {
        // The same plaintext under a different DEK yields a different id — the
        // on-disk name is keyed, not a bare content hash.
        let plaintext = b"a 1099 form";
        assert_ne!(
            storage_id(&generate_dek().unwrap(), plaintext),
            storage_id(&generate_dek().unwrap(), plaintext)
        );
    }

    #[test]
    fn each_encrypt_uses_a_fresh_nonce() {
        let key = generate_content_key().unwrap();
        let a = encrypt_blob(&key, b"x").unwrap();
        let b = encrypt_blob(&key, b"x").unwrap();
        assert_ne!(a.nonce, b.nonce);
        assert_ne!(a.ciphertext, b.ciphertext);
    }

    #[test]
    fn content_key_debug_is_redacted() {
        let key = generate_content_key().unwrap();
        assert_eq!(format!("{key:?}"), "ContentKey([REDACTED])");
    }

    #[test]
    fn tampered_blob_ciphertext_is_rejected() {
        let key = generate_content_key().unwrap();
        let mut blob = encrypt_blob(&key, b"sensitive").unwrap();
        blob.ciphertext[0] ^= 0xff;
        assert!(matches!(
            decrypt_blob(&key, &blob),
            Err(VaultCryptoError::KeyUnwrap)
        ));
    }
}
