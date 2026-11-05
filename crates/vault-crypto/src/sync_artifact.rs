//! Class-1b artifact addresses, sealing and opening (`docs/architecture/vault-format.md` C.3 and C.9;
//! `personal-cfo-0g528.2`, part S2b).
//!
//! An artifact (an attachment's bytes, or one part of a source batch) is published as an immutable
//! object under an opaque, epoch-scoped address:
//!
//! * [`first_artifact_id`] is the id at an artifact's **first publication**, keyed under `K_e` from
//!   its 32-byte local address (C.3). Only the device that first publishes an artifact can compute
//!   it, because the address is keyed under that device's own DEK.
//! * [`readdress_id`] is the id at a **key-changing epoch**, chained from the previous id and keyed
//!   under `K_{e+1}`. Every member of the new epoch holds the previous id, so any member can run a
//!   cutover (ADR 0074 Addendum I).
//! * [`seal_artifact`] seals a transport plaintext under the per-artifact object key with the fixed
//!   zero nonce, and [`open_artifact`] opens it from the **expected** address, never from the
//!   container, with the address in the AAD so bytes presented under another address do not
//!   authenticate.
//!
//! # The zero nonce
//!
//! The nonce is twelve zero bytes. That is safe only because the object key is unique per
//! `(vault, key_epoch, sync_artifact_id)` and invoked once, and only while **the plaintext sealed
//! under an address never changes**. [`Fixity`] enforces the second half: the first seal returns the
//! SHA-256 of the complete transport plaintext, the caller stores it, and every later seal passes it
//! back, so a changed descriptor, a changed blob or a changed id is refused before encryption with
//! `ArtifactDescriptorChanged`. Encryption is deterministic, so a retry rebuilds identical bytes.
//!
//! # Secret handling
//!
//! Keys live in [`SecretBytes`]. The plaintext an open returns lives in a zeroizing buffer
//! ([`OpenedArtifact`]); a re-address reseals in memory and the buffer is zeroized when it drops.
//! Nothing here puts a key, a plaintext or a blob in an error, a `Debug` output or a log.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use sync_core::artifact::{
    artifact_aad, encode_container, parse_container, ArtifactError, ArtifactId, ArtifactReceipt,
    TransportPlaintext, MAX_ARTIFACT_CONTAINER,
};
use sync_core::header::PROTOCOL_VERSION;
use thiserror::Error;
use zeroize::Zeroizing;

use crate::secret::SecretBytes;
use crate::sync::label_bytes;
use crate::sync_envelope::EpochKey;

/// The label of the first-publication address derivation.
pub const ADDRESS_LABEL: &str = "dohflow/sync-artifact-address/v1";
/// The label of the chained re-address derivation.
pub const READDRESS_LABEL: &str = "dohflow/sync-artifact-readdress/v1";
/// The label of the per-artifact object key derivation.
pub const OBJECT_LABEL: &str = "dohflow/sync-artifact-object/v1";

/// `Expand(K, info)`: HKDF-SHA256-Expand with PRK `K`, 32 bytes out (C.3).
fn expand(key: &EpochKey, info: &[u8]) -> SecretBytes<32> {
    let hk = Hkdf::<Sha256>::from_prk(key.secret().expose()).expect("a 32-byte key is a valid PRK");
    SecretBytes::<32>::init(|out| hk.expand(info, out).map_err(|_| ()))
        .expect("32 bytes is within HKDF-SHA256's output limit")
}

fn hmac_sha256(key: &SecretBytes<32>, message: &[u8]) -> [u8; 32] {
    let mut mac =
        <Hmac<Sha256> as Mac>::new_from_slice(key.expose()).expect("HMAC takes a key of any size");
    mac.update(message);
    mac.finalize().into_bytes().into()
}

/// The id of an artifact at its **first publication** in epoch `key.key_epoch()` (C.3):
/// `HMAC-SHA256(Expand(K_e, label ‖ uuid vault_id), storage_id_raw)`. `storage_id_raw` is the
/// artifact's 32 raw address bytes, never the text of a hex string.
#[must_use]
pub fn first_artifact_id(key: &EpochKey, storage_id_raw: &[u8; 32]) -> ArtifactId {
    let mut info = label_bytes(ADDRESS_LABEL);
    info.extend_from_slice(key.vault_id());
    ArtifactId::from_derived(hmac_sha256(&expand(key, &info), storage_id_raw))
}

/// The id of an artifact in the next key-changing epoch (C.3):
/// `HMAC-SHA256(Expand(K_{e+1}, label ‖ uuid vault_id ‖ u64 key_epoch_{e+1}), previous_id)`.
///
/// It needs only the previous id and `K_{e+1}`, so any member of the new epoch can compute it
/// without the origin's DEK and without fetching anything; a device that knows the previous id and
/// not `K_{e+1}` cannot link the two ids.
#[must_use]
pub fn readdress_id(next_key: &EpochKey, previous: &ArtifactId) -> ArtifactId {
    let mut info = label_bytes(READDRESS_LABEL);
    info.extend_from_slice(next_key.vault_id());
    info.extend_from_slice(&next_key.key_epoch().to_be_bytes());
    ArtifactId::from_derived(hmac_sha256(&expand(next_key, &info), previous.as_bytes()))
}

/// The per-artifact object key (C.3): `Expand(K_e, label ‖ u16 protocol_version ‖ uuid vault_id ‖
/// u64 key_epoch ‖ digest sync_artifact_id)`.
fn object_key(key: &EpochKey, id: &ArtifactId) -> SecretBytes<32> {
    let mut info = label_bytes(OBJECT_LABEL);
    info.extend_from_slice(&PROTOCOL_VERSION.to_be_bytes());
    info.extend_from_slice(key.vault_id());
    info.extend_from_slice(&key.key_epoch().to_be_bytes());
    info.extend_from_slice(id.as_bytes());
    expand(key, &info)
}

/// The nonce of every artifact object: twelve zero bytes. Safe only under C.9's fixed-plaintext rule
/// (see the module comment): the key is unique per address and invoked once.
const ZERO_NONCE: [u8; 12] = [0; 12];

fn encrypt(aead: &SecretBytes<32>, aad: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, ()> {
    Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(aead.expose()))
        .encrypt(
            Nonce::from_slice(&ZERO_NONCE),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| ())
}

fn decrypt(aead: &SecretBytes<32>, aad: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, ()> {
    Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(aead.expose()))
        .decrypt(
            Nonce::from_slice(&ZERO_NONCE),
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .map_err(|_| ())
}

/// Whether a seal is the first for an address or must reproduce what was sealed before.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fixity {
    /// The first seal under this `(key_epoch, sync_artifact_id)`; the caller stores the returned
    /// [`SealedArtifact::plaintext_digest`].
    First,
    /// A later seal: the stored SHA-256 of the complete transport plaintext. Any difference is
    /// refused before encryption.
    Expected([u8; 32]),
}

/// Why an artifact was not sealed. Nothing was encrypted in any of these cases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum SealArtifactError {
    /// The transport plaintext differs from the one first sealed under this address (C.11
    /// `ArtifactDescriptorChanged`): a changed descriptor, blob or id.
    #[error("the artifact's plaintext differs from the one first sealed under its address")]
    ArtifactDescriptorChanged,
    /// The transport plaintext cannot be encoded.
    #[error("{0}")]
    Format(ArtifactError),
    /// The container would be larger than the service stores.
    #[error("the sealed artifact is too large")]
    ContainerTooLarge,
    /// The AEAD refused to encrypt.
    #[error("encryption failed")]
    Encryption,
}

/// A sealed artifact: the container to publish, the receipt the service must answer with, and the
/// digest of the plaintext it was sealed from.
#[derive(Clone, PartialEq, Eq)]
pub struct SealedArtifact {
    /// The `DFSA` container: the only bytes to publish.
    pub container: Vec<u8>,
    /// The receipt a faithful service returns for these bytes.
    pub receipt: ArtifactReceipt,
    /// SHA-256 of the complete transport plaintext (store it; pass it back as [`Fixity::Expected`]).
    pub plaintext_digest: [u8; 32],
}

impl core::fmt::Debug for SealedArtifact {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // Up to 64 MiB of ciphertext: the size and the address, not the bytes.
        f.debug_struct("SealedArtifact")
            .field("len", &self.container.len())
            .field("artifact_id", &self.receipt.artifact_id)
            .finish_non_exhaustive()
    }
}

/// Seal `plaintext` under `key`'s epoch and its own address (the address is `plaintext.artifact_id`,
/// which the caller derived for this epoch).
///
/// # Errors
/// A [`SealArtifactError`]; nothing is encrypted unless the plaintext matches `fixity`.
pub fn seal_artifact(
    key: &EpochKey,
    plaintext: &TransportPlaintext<'_>,
    fixity: Fixity,
) -> Result<SealedArtifact, SealArtifactError> {
    let encoded = Zeroizing::new(plaintext.encode().map_err(SealArtifactError::Format)?);
    let digest: [u8; 32] = Sha256::digest(encoded.as_slice()).into();
    if let Fixity::Expected(expected) = fixity {
        if expected != digest {
            return Err(SealArtifactError::ArtifactDescriptorChanged);
        }
    }
    let id = plaintext.artifact_id;
    let aad = artifact_aad(key.vault_id(), key.key_epoch(), &id);
    let aead = object_key(key, &id);
    let ciphertext =
        encrypt(&aead, &aad, encoded.as_slice()).map_err(|()| SealArtifactError::Encryption)?;
    let container = encode_container(&ciphertext);
    if container.len() > MAX_ARTIFACT_CONTAINER {
        return Err(SealArtifactError::ContainerTooLarge);
    }
    Ok(SealedArtifact {
        receipt: ArtifactReceipt::for_ciphertext(*key.vault_id(), key.key_epoch(), id, &ciphertext),
        container,
        plaintext_digest: digest,
    })
}

/// Why an artifact was not opened. Each is deterministic from the bytes, the key and the address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum OpenArtifactError {
    /// The container is malformed or too large.
    #[error("{0}")]
    Container(ArtifactError),
    /// The ciphertext does not authenticate under the key and address (a flipped byte, a wrong
    /// key, or bytes presented under another address).
    #[error("the artifact does not open")]
    NotOpenable,
    /// It authenticates, but the plaintext fails its own descriptor (length, digest, kind or
    /// metadata).
    #[error("{0}")]
    Plaintext(ArtifactError),
    /// It authenticates and decodes, but names another address than the one it was fetched under.
    #[error("the artifact names another address")]
    WrongAddress,
}

/// A verified, decrypted artifact. The plaintext sits in a zeroizing buffer and is zeroized when
/// this value drops.
pub struct OpenedArtifact {
    bytes: Zeroizing<Vec<u8>>,
}

impl core::fmt::Debug for OpenedArtifact {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OpenedArtifact")
            .field("len", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

impl OpenedArtifact {
    /// The verified transport plaintext, borrowed from the protected buffer.
    ///
    /// # Panics
    /// Never: the bytes were decoded and checked when the artifact was opened.
    #[must_use]
    pub fn plaintext(&self) -> TransportPlaintext<'_> {
        TransportPlaintext::decode(&self.bytes).expect("checked when the artifact was opened")
    }
}

/// Open `container` as the artifact at `expected_id` in `key`'s epoch: parse the container, derive
/// the key from the **expected** address (never from anything the container claims), authenticate,
/// then check the plaintext's descriptor and that it names `expected_id`. `max_len` bounds the
/// container before anything is parsed.
///
/// # Errors
/// An [`OpenArtifactError`].
pub fn open_artifact(
    key: &EpochKey,
    expected_id: &ArtifactId,
    container: &[u8],
    max_len: usize,
) -> Result<OpenedArtifact, OpenArtifactError> {
    let ciphertext = parse_container(container, max_len).map_err(OpenArtifactError::Container)?;
    let aad = artifact_aad(key.vault_id(), key.key_epoch(), expected_id);
    let aead = object_key(key, expected_id);
    let bytes = Zeroizing::new(
        decrypt(&aead, &aad, ciphertext).map_err(|()| OpenArtifactError::NotOpenable)?,
    );
    let plaintext = TransportPlaintext::decode(&bytes).map_err(OpenArtifactError::Plaintext)?;
    if &plaintext.artifact_id != expected_id {
        return Err(OpenArtifactError::WrongAddress);
    }
    Ok(OpenedArtifact { bytes })
}

/// Why an artifact was not re-addressed. Nothing was published.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ReaddressError {
    /// The two keys are not consecutive epochs of one vault.
    #[error("the keys are not consecutive epochs of one vault")]
    NotTheNextEpoch,
    /// The old object does not open (the fetch path).
    #[error("{0}")]
    Open(OpenArtifactError),
    /// The resealed object was refused.
    #[error("{0}")]
    Seal(SealArtifactError),
}

/// An artifact resealed for the next key-changing epoch.
#[derive(Debug)]
pub struct Readdressed {
    /// Its new address, `sync_artifact_id_{e+1}`.
    pub artifact_id: ArtifactId,
    /// The sealed object, its receipt and its plaintext digest.
    pub sealed: SealedArtifact,
}

fn require_next_epoch(old_key: &EpochKey, new_key: &EpochKey) -> Result<(), ReaddressError> {
    if old_key.vault_id() != new_key.vault_id()
        || old_key.key_epoch().checked_add(1) != Some(new_key.key_epoch())
    {
        return Err(ReaddressError::NotTheNextEpoch);
    }
    Ok(())
}

/// Re-address one artifact for the next epoch from a verified plaintext the caller already holds
/// (a local materialized copy whose digest matched, or [`OpenedArtifact::plaintext`]): the new id
/// is chained from `old_id`, the plaintext is the same except for the id it carries, and it is
/// sealed in memory under the new epoch's object key. Nothing is materialized.
///
/// # Errors
/// [`ReaddressError`].
pub fn readdress_plaintext(
    old_key: &EpochKey,
    new_key: &EpochKey,
    old_id: &ArtifactId,
    plaintext: &TransportPlaintext<'_>,
    fixity: Fixity,
) -> Result<Readdressed, ReaddressError> {
    require_next_epoch(old_key, new_key)?;
    let artifact_id = readdress_id(new_key, old_id);
    let moved = TransportPlaintext {
        artifact_id,
        ..*plaintext
    };
    let sealed = seal_artifact(new_key, &moved, fixity).map_err(ReaddressError::Seal)?;
    Ok(Readdressed {
        artifact_id,
        sealed,
    })
}

/// Re-address one artifact from the epoch-`e` object the service holds: open it under `K_e` at
/// `old_id`, then [`readdress_plaintext`]. The opened plaintext is zeroized when this returns.
///
/// # Errors
/// [`ReaddressError`]; an object that cannot be opened is [`ReaddressError::Open`], and nothing is
/// published.
pub fn readdress_container(
    old_key: &EpochKey,
    new_key: &EpochKey,
    old_id: &ArtifactId,
    old_container: &[u8],
    fixity: Fixity,
) -> Result<Readdressed, ReaddressError> {
    require_next_epoch(old_key, new_key)?;
    let opened = open_artifact(old_key, old_id, old_container, MAX_ARTIFACT_CONTAINER)
        .map_err(ReaddressError::Open)?;
    readdress_plaintext(old_key, new_key, old_id, &opened.plaintext(), fixity)
}
