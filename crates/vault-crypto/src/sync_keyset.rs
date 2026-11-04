//! Sealing and unsealing a key set to one device (`docs/architecture/vault-format.md` C.6, "Sealing
//! the key set (F1)"; personal-cfo-1cltt.2).
//!
//! For each recipient the sealer runs HPKE (RFC 9180) base mode `Seal` to the recipient's X25519
//! key: DHKEM(X25519, HKDF-SHA256), HKDF-SHA256, AES-256-GCM, the one suite of C.1.4. The plaintext
//! is `label "dohflow/sync-sealed-keys/v1" ‖ key32 K_e ‖ key32 S`, 93 bytes, so the sealed blob is a
//! 32-byte `enc` and a 109-byte ciphertext. The `info` and `aad` come from
//! [`sync_core::membership::EpochBody::hpke_info`] and
//! [`sync_core::membership::EpochBody::hpke_aad`]: they bind the epoch header, the recipient and the
//! sealer, so a sealed entry cannot be moved to another record or another recipient.
//!
//! # Randomness
//!
//! `hpke` 0.13 draws randomness in exactly one place per seal: a 32-byte seed from which it derives
//! the ephemeral key. This module draws that seed itself from the operating system (a fallible
//! call, so a failed source is an error and never a zero key) and hands the crate a [`SeedRng`],
//! a generator that yields exactly those 32 bytes once. Asking it for one byte more panics: that
//! can only mean `hpke` changed how it draws, and a loud stop beats silent reuse. `SeedRng` is
//! `CryptoRng` only here and is not a general generator. The known-answer tests feed the RFC's
//! `ikmE` through the same type, so they pin the code that runs in production.
//!
//! # Secret handling
//!
//! The plaintext is built in, and opened into, a zeroizing buffer. The recipient's private key is a
//! [`SecretBytes`]; the `hpke` private-key type built from it scrubs on drop because the workspace
//! turns on `x25519-dalek`'s `zeroize` feature (guarded by `tests/x25519_zeroize.rs`). No error
//! here carries a key, a plaintext, an `enc` or a context.

use hpke::aead::AesGcm256;
use hpke::kdf::HkdfSha256;
use hpke::kem::X25519HkdfSha256;
use hpke::rand_core::{CryptoRng, RngCore};
use hpke::{single_shot_open, single_shot_seal, Deserializable, OpModeR, OpModeS, Serializable};
use sync_core::keys::x25519_public_key_acceptable;
use sync_core::membership::SEALED_KEYS_CIPHERTEXT_LEN;
use zeroize::Zeroizing;

use crate::secret::SecretBytes;
use crate::sync::label_bytes;
use crate::sync_keys::{generate_secret, DeviceKeys};

/// The label of the sealed-keys plaintext.
pub const SEALED_KEYS_LABEL: &str = "dohflow/sync-sealed-keys/v1";
/// The sealed-keys plaintext: a CBE1 label (2 + 27 bytes), `K_e` and `S`.
pub const SEALED_KEYS_PLAINTEXT_LEN: usize = 93;

type Kem = X25519HkdfSha256;
type Kdf = HkdfSha256;
type Aead = AesGcm256;

/// Why a key set was not sealed or not unsealed. Never carries key material.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum KeySetError {
    /// The system random source failed.
    #[error("the system random source failed")]
    Rng,
    /// The recipient's key is one the chain refuses (a low-order or malformed X25519 key).
    #[error("the recipient's agreement key is not acceptable")]
    UnacceptableRecipientKey,
    /// HPKE refused to seal.
    #[error("the key set could not be sealed")]
    Seal,
    /// HPKE could not open the blob: wrong key, wrong `info`, wrong `aad`, or a damaged blob.
    #[error("the sealed key set could not be opened")]
    Open,
    /// The blob opened but is not a key set: a wrong length or a wrong label.
    #[error("the opened bytes are not a key set")]
    NotAKeySet,
}

/// One recipient's sealed key set: the HPKE encapsulated key and the ciphertext.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealedKeySet {
    /// The HPKE `enc`, 32 bytes in suite 1.
    pub enc: [u8; 32],
    /// The ciphertext, [`SEALED_KEYS_CIPHERTEXT_LEN`] bytes: the plaintext and the 16-byte tag.
    pub ciphertext: Vec<u8>,
}

/// A generator that yields exactly one 32-byte seed, once. See the module doc.
pub struct SeedRng {
    seed: Zeroizing<[u8; 32]>,
    taken: usize,
}

impl SeedRng {
    /// A generator over `seed`.
    #[must_use]
    pub fn new(seed: &SecretBytes<32>) -> Self {
        Self {
            seed: Zeroizing::new(*seed.expose()),
            taken: 0,
        }
    }
}

impl RngCore for SeedRng {
    fn next_u32(&mut self) -> u32 {
        let mut bytes = [0_u8; 4];
        self.fill_bytes(&mut bytes);
        u32::from_le_bytes(bytes)
    }

    fn next_u64(&mut self) -> u64 {
        let mut bytes = [0_u8; 8];
        self.fill_bytes(&mut bytes);
        u64::from_le_bytes(bytes)
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        let end = self.taken + dest.len();
        // Unreachable by construction: `hpke` 0.13 asks for exactly its 32-byte private-key size,
        // once per seal. If that ever changes, stop rather than reuse or invent bytes.
        assert!(
            end <= self.seed.len(),
            "SeedRng asked for more than its seed"
        );
        dest.copy_from_slice(&self.seed[self.taken..end]);
        self.taken = end;
    }
}

impl CryptoRng for SeedRng {}

/// HPKE base-mode single-shot seal of `plaintext` to `recipient_public_key`, with the ephemeral key
/// derived from `seed`. Low level: it exists so the known-answer tests can fix the seed. Production
/// code seals through [`seal_key_set`], which draws a fresh seed.
///
/// # Errors
/// [`KeySetError::UnacceptableRecipientKey`] or [`KeySetError::Seal`].
#[doc(hidden)]
pub fn seal_with_seed(
    recipient_public_key: &[u8; 32],
    info: &[u8],
    aad: &[u8],
    plaintext: &[u8],
    seed: &SecretBytes<32>,
) -> Result<([u8; 32], Vec<u8>), KeySetError> {
    if !x25519_public_key_acceptable(recipient_public_key) {
        return Err(KeySetError::UnacceptableRecipientKey);
    }
    let public = <Kem as hpke::Kem>::PublicKey::from_bytes(recipient_public_key)
        .map_err(|_| KeySetError::UnacceptableRecipientKey)?;
    let mut rng = SeedRng::new(seed);
    let (encapped, ciphertext) = single_shot_seal::<Aead, Kdf, Kem, _>(
        &OpModeS::Base,
        &public,
        info,
        plaintext,
        aad,
        &mut rng,
    )
    .map_err(|_| KeySetError::Seal)?;
    let enc: [u8; 32] = encapped
        .to_bytes()
        .as_slice()
        .try_into()
        .map_err(|_| KeySetError::Seal)?;
    Ok((enc, ciphertext))
}

/// HPKE base-mode single-shot open. The plaintext comes back in a zeroizing buffer.
///
/// # Errors
/// [`KeySetError::Open`].
#[doc(hidden)]
pub fn open_with_key(
    recipient_private_key: &SecretBytes<32>,
    enc: &[u8; 32],
    info: &[u8],
    aad: &[u8],
    ciphertext: &[u8],
) -> Result<Zeroizing<Vec<u8>>, KeySetError> {
    let private = <Kem as hpke::Kem>::PrivateKey::from_bytes(recipient_private_key.expose())
        .map_err(|_| KeySetError::Open)?;
    let encapped =
        <Kem as hpke::Kem>::EncappedKey::from_bytes(enc).map_err(|_| KeySetError::Open)?;
    single_shot_open::<Aead, Kdf, Kem>(&OpModeR::Base, &private, &encapped, info, ciphertext, aad)
        .map(Zeroizing::new)
        .map_err(|_| KeySetError::Open)
}

/// Seal `K_e` and `S` to one recipient (C.6, F1) with a fresh ephemeral key.
///
/// `info` and `aad` are the recipient's own, from the epoch header being built.
///
/// # Errors
/// [`KeySetError`].
pub fn seal_key_set(
    recipient_public_key: &[u8; 32],
    info: &[u8],
    aad: &[u8],
    epoch_root: &SecretBytes<32>,
    tag_secret: &SecretBytes<32>,
) -> Result<SealedKeySet, KeySetError> {
    let mut plaintext = Zeroizing::new(Vec::with_capacity(SEALED_KEYS_PLAINTEXT_LEN));
    plaintext.extend_from_slice(&label_bytes(SEALED_KEYS_LABEL));
    plaintext.extend_from_slice(epoch_root.expose());
    plaintext.extend_from_slice(tag_secret.expose());
    debug_assert_eq!(plaintext.len(), SEALED_KEYS_PLAINTEXT_LEN);
    let seed = generate_secret().map_err(|_| KeySetError::Rng)?;
    let (enc, ciphertext) = seal_with_seed(recipient_public_key, info, aad, &plaintext, &seed)?;
    if ciphertext.len() != SEALED_KEYS_CIPHERTEXT_LEN {
        return Err(KeySetError::Seal);
    }
    Ok(SealedKeySet { enc, ciphertext })
}

/// A key set as unsealed, before either key has been checked against a commitment. The two values
/// become usable only through [`crate::sync_envelope::EpochKey::from_verified`] and
/// [`crate::sync::TagSecret::from_verified`].
pub struct UnsealedKeySet {
    /// The candidate `K_e`.
    pub epoch_root: SecretBytes<32>,
    /// The candidate `S`.
    pub tag_secret: SecretBytes<32>,
}

impl core::fmt::Debug for UnsealedKeySet {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("UnsealedKeySet([REDACTED])")
    }
}

/// Open this device's entry of an epoch record (C.6, F1).
///
/// # Errors
/// [`KeySetError::Open`] for any HPKE failure, [`KeySetError::NotAKeySet`] for a wrong length or
/// label.
pub fn unseal_key_set(
    recipient: &DeviceKeys,
    sealed: &SealedKeySet,
    info: &[u8],
    aad: &[u8],
) -> Result<UnsealedKeySet, KeySetError> {
    if sealed.ciphertext.len() != SEALED_KEYS_CIPHERTEXT_LEN {
        return Err(KeySetError::NotAKeySet);
    }
    let plaintext = open_with_key(
        recipient.agreement_secret(),
        &sealed.enc,
        info,
        aad,
        &sealed.ciphertext,
    )?;
    let label = label_bytes(SEALED_KEYS_LABEL);
    if plaintext.len() != SEALED_KEYS_PLAINTEXT_LEN || plaintext[..label.len()] != label[..] {
        return Err(KeySetError::NotAKeySet);
    }
    let keys = &plaintext[label.len()..];
    let epoch_root = SecretBytes::<32>::from_slice(&keys[..32]).ok_or(KeySetError::NotAKeySet)?;
    let tag_secret = SecretBytes::<32>::from_slice(&keys[32..]).ok_or(KeySetError::NotAKeySet)?;
    Ok(UnsealedKeySet {
        epoch_root,
        tag_secret,
    })
}
