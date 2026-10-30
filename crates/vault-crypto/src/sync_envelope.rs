//! Sealing and opening Sync envelopes (`docs/architecture/vault-format.md` C.3 and C.4;
//! personal-cfo-g3m.22).
//!
//! An envelope is an AES-256-GCM object whose AAD is its clear header. The AEAD key is
//! `Expand(K_e, "dohflow/sync-aead/v1" ‖ protocol_version ‖ vault_id ‖ key_epoch ‖ fingerprint ‖
//! object_type)`, so each device has its own key per epoch and object type, and the GCM nonce is the
//! header's 4-byte nonce domain followed by the 8-byte invocation counter. Two devices can therefore
//! never produce the same (key, nonce), **provided each seals only under its own identity and its
//! own counter**. That proviso is what the types here enforce.
//!
//! * [`EpochKey`] is the verified key of one vault's one epoch; it can only be built after the
//!   key is checked against the epoch record's commitment, and it carries its vault and epoch.
//! * [`seal_envelope`] takes the sealer's own identity ([`SealerIdentity`]) and refuses any header
//!   that names another vault, epoch, device, fingerprint or nonce domain, **before** it encrypts.
//!   The invocation counter is the caller's: it must have persisted the advance first (C.4).
//! * [`ReceiverOpener`] is the receiver path (and the producer's own seal-then-verify path): the
//!   key is derived from the **expected** vault and epoch and from the fingerprint the membership
//!   chain holds for the header's device **during the envelope's epoch**, never from the header's
//!   claims; the header must restate exactly those; the AAD is the received header bytes. It is the
//!   only implementation of `sync_core::producer::EnvelopeOpener` allowed outside tests (a
//!   source-scan test guards this), so "verified by opening" can never mean "handed back the
//!   plaintext the producer still held".
//!
//! # Secret handling
//!
//! Keys live in [`SecretBytes`] (locked, zeroized on drop). Nothing here puts a key, nonce, counter
//! or plaintext in an error, a `Debug` output or a log. The plaintext an opener returns is the
//! caller's (it is a command payload); `sync-core`'s decoded `EnvelopePlaintext` does not zeroize
//! its payload, a limitation tracked separately.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use hkdf::Hkdf;
use sha2::{Digest, Sha256};
use sync_core::envelope::ReceiverError;
use sync_core::header::{
    encode_container, ClearHeader, Fingerprint, NonceDomain, ParsedContainer,
    MAX_ENVELOPE_CONTAINER_LEN, OBJECT_ENVELOPE, OBJECT_SNAPSHOT, PROTOCOL_VERSION,
};
use sync_core::membership::{EpochAtPosition, VerifiedState};
use sync_core::producer::EnvelopeOpener;
use thiserror::Error;

use crate::secret::SecretBytes;
use crate::sync::label_bytes;

/// The label of the AEAD-key derivation.
pub const AEAD_KEY_LABEL: &str = "dohflow/sync-aead/v1";
/// The label of the epoch-key commitment.
pub const EPOCH_KEY_COMMITMENT_LABEL: &str = "dohflow/sync-epoch-key-commit/v1";

/// Which kind of object a key is for. A key for one is never a key for the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectType {
    /// A command envelope.
    Envelope,
    /// A snapshot.
    Snapshot,
}

impl ObjectType {
    const fn code(self) -> u8 {
        match self {
            Self::Envelope => OBJECT_ENVELOPE,
            Self::Snapshot => OBJECT_SNAPSHOT,
        }
    }
}

/// Why an epoch key was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum KeyError {
    /// The key does not match the commitment in the epoch record.
    #[error("the key does not match the epoch record's commitment")]
    CommitmentMismatch,
}

/// The verified key `K_e` of one vault's one key epoch. It cannot be built from a raw key alone:
/// the key must match the epoch record's `key_commitment_ke`, so an `EpochKey` is always the real
/// key of that vault's epoch, and it cannot be paired with another epoch or vault by mistake.
pub struct EpochKey {
    vault_id: [u8; 16],
    key_epoch: u64,
    bytes: SecretBytes<32>,
}

impl EpochKey {
    /// Accept `key` as the key of `key_epoch` of `vault_id` if its commitment equals
    /// `commitment` (the `key_commitment_ke` of the verified epoch record).
    ///
    /// # Errors
    /// [`KeyError::CommitmentMismatch`].
    pub fn from_verified(
        vault_id: [u8; 16],
        key_epoch: u64,
        key: SecretBytes<32>,
        commitment: &[u8; 32],
    ) -> Result<Self, KeyError> {
        // Hash incrementally: building `label ‖ K_e` in a Vec would leave an unscrubbed copy of the
        // epoch root on the heap (and a second one if it reallocated).
        let mut hash = Sha256::new();
        hash.update(label_bytes(EPOCH_KEY_COMMITMENT_LABEL));
        hash.update(key.expose());
        let computed: [u8; 32] = hash.finalize().into();
        if &computed != commitment {
            return Err(KeyError::CommitmentMismatch);
        }
        Ok(Self {
            vault_id,
            key_epoch,
            bytes: key,
        })
    }

    /// The vault this key belongs to.
    #[must_use]
    pub const fn vault_id(&self) -> &[u8; 16] {
        &self.vault_id
    }

    /// The epoch this key belongs to.
    #[must_use]
    pub const fn key_epoch(&self) -> u64 {
        self.key_epoch
    }
}

impl core::fmt::Debug for EpochKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("EpochKey")
            .field("key_epoch", &self.key_epoch)
            .finish_non_exhaustive()
    }
}

/// The AEAD key of one device for one object type in one epoch, in a protected buffer.
///
/// # Panics
/// Never in practice: a 32-byte HKDF-SHA256 output is far inside its limit.
#[must_use]
pub fn aead_key(key: &EpochKey, fingerprint: &Fingerprint, object: ObjectType) -> SecretBytes<32> {
    let hk = Hkdf::<Sha256>::from_prk(key.bytes.expose()).expect("a 32-byte key is a valid PRK");
    let mut info = label_bytes(AEAD_KEY_LABEL);
    info.extend_from_slice(&PROTOCOL_VERSION.to_be_bytes());
    info.extend_from_slice(&key.vault_id);
    info.extend_from_slice(&key.key_epoch.to_be_bytes());
    info.extend_from_slice(fingerprint.as_bytes());
    info.push(object.code());
    SecretBytes::<32>::init(|out| hk.expand(&info, out).map_err(|_| ()))
        .expect("32 bytes is within HKDF-SHA256's output limit")
}

/// The GCM nonce: the header's nonce domain, then the invocation counter (C.3).
fn gcm_nonce(domain: &NonceDomain, counter: u64) -> [u8; 12] {
    let mut nonce = [0_u8; 12];
    nonce[..4].copy_from_slice(domain.as_bytes());
    nonce[4..].copy_from_slice(&counter.to_be_bytes());
    nonce
}

/// The sealing device's own identity, from its own enrollment state. [`seal_envelope`] refuses a
/// header that names anyone else: sealing under another device's fingerprint would reuse that
/// device's (key, nonce domain) with this device's counter, which is AES-GCM nonce reuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SealerIdentity {
    /// The vault.
    pub vault_id: [u8; 16],
    /// This device's id.
    pub device_id: [u8; 16],
    /// This device's fingerprint.
    pub fingerprint: Fingerprint,
}

/// Why an envelope was not sealed. Nothing was encrypted in any of these cases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum SealError {
    /// The header is not an envelope header.
    #[error("the header is not an envelope header")]
    NotAnEnvelope,
    /// The header names a vault other than the key's and the sealer's.
    #[error("the header names another vault")]
    WrongVault,
    /// The header names an epoch other than the key's.
    #[error("the header names another key epoch")]
    WrongEpoch,
    /// The header names a device or fingerprint other than the sealer's own.
    #[error("the header names another device")]
    WrongDevice,
    /// The header's nonce domain is not the one derived for this device and epoch.
    #[error("the header's nonce domain is not the derived one")]
    WrongNonceDomain,
    /// The header or the sealed object cannot be encoded within the bounds.
    #[error("the object cannot be encoded")]
    Unencodable,
    /// The AEAD refused to encrypt.
    #[error("encryption failed")]
    Encryption,
}

/// Seal `plaintext` as the envelope `header` describes, with `key` as the epoch's key and `sealer`
/// as this device. Returns the container bytes.
///
/// The caller owns the invocation counter: it must have persisted the advance **before** calling
/// this (C.4), and the counter is whatever the header carries. This function never chooses one.
///
/// # Errors
/// A [`SealError`]; nothing is encrypted unless every identity check passes.
pub fn seal_envelope(
    key: &EpochKey,
    sealer: &SealerIdentity,
    header: &ClearHeader,
    plaintext: &[u8],
) -> Result<Vec<u8>, SealError> {
    if header.object_type() != OBJECT_ENVELOPE {
        return Err(SealError::NotAnEnvelope);
    }
    if header.vault_id != key.vault_id || header.vault_id != sealer.vault_id {
        return Err(SealError::WrongVault);
    }
    if header.key_epoch != key.key_epoch {
        return Err(SealError::WrongEpoch);
    }
    if header.device_id != sealer.device_id || header.device_key_fingerprint != sealer.fingerprint {
        return Err(SealError::WrongDevice);
    }
    let domain = NonceDomain::derive(
        &header.vault_id,
        header.key_epoch,
        &header.device_key_fingerprint,
        OBJECT_ENVELOPE,
    );
    if header.nonce_domain != domain {
        return Err(SealError::WrongNonceDomain);
    }
    let aad = header.encode().map_err(|_| SealError::Unencodable)?;
    let aead = aead_key(key, &sealer.fingerprint, ObjectType::Envelope);
    let ciphertext = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(aead.expose()))
        .encrypt(
            Nonce::from_slice(&gcm_nonce(&domain, header.invocation_counter)),
            Payload {
                msg: plaintext,
                aad: &aad,
            },
        )
        .map_err(|_| SealError::Encryption)?;
    let container = encode_container(header, &ciphertext).map_err(|_| SealError::Unencodable)?;
    if container.len() > MAX_ENVELOPE_CONTAINER_LEN {
        return Err(SealError::Unencodable);
    }
    Ok(container)
}

/// The receiver path: opens an envelope the way every replica does, and the way a producer
/// verifies what it is about to upload.
///
/// The key is derived from the expected vault and epoch (carried by the [`EpochKey`]) and from the
/// fingerprint the membership chain holds for the header's device **as a member of that epoch**
/// (`VerifiedState::authorized_member_in_epoch`), never from the header's own claims. The header
/// must then restate exactly the vault, epoch, fingerprint and derived nonce domain. The AAD is the
/// received header bytes. Any failure is `None`, uniformly: nothing here depends on a secret in a
/// way a caller could observe.
pub struct ReceiverOpener<'a> {
    key: &'a EpochKey,
    chain: &'a VerifiedState,
    window: MemberWindow,
}

/// Which membership a receiver judges a header's device against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MemberWindow {
    /// The producer's own check: the last head of the key's epoch (it is sealing at the head).
    InEpoch,
    /// A replica processing the log in order: the membership version in force at the position.
    AtVersion(u64),
}

impl<'a> ReceiverOpener<'a> {
    /// An opener for envelopes of `key`'s epoch, checked against `chain`, for the **producer's**
    /// own seal-then-verify ONLY: the device must be a member as of the last head of the epoch,
    /// since it seals at the head. A replica judging a log position uses
    /// [`Self::at_position`]; a source-scan test keeps this constructor out of replica code.
    #[must_use]
    pub const fn for_producer(key: &'a EpochKey, chain: &'a VerifiedState) -> Self {
        Self {
            key,
            chain,
            window: MemberWindow::InEpoch,
        }
    }

    /// An opener for a **replica** judging the envelope at one log position: the device must be a
    /// member as of the membership version in force at that position, so the verdict does not
    /// depend on how far past the position the replica has verified.
    #[must_use]
    pub const fn at_position(
        key: &'a EpochKey,
        chain: &'a VerifiedState,
        position: &EpochAtPosition,
    ) -> Self {
        Self {
            key,
            chain,
            window: MemberWindow::AtVersion(position.membership_head_version()),
        }
    }
}

impl core::fmt::Debug for ReceiverOpener<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ReceiverOpener")
            .field("key_epoch", &self.key.key_epoch)
            .finish_non_exhaustive()
    }
}

/// Why the receiver path refused to open an envelope. Each is deterministic from the bytes, the
/// verified key and the verified chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum OpenFailure {
    /// The header is not an envelope header.
    #[error("not an envelope header")]
    NotAnEnvelope,
    /// The header names another vault than the key's.
    #[error("another vault")]
    WrongVault,
    /// The header names another key epoch than the key's.
    #[error("another key epoch")]
    WrongEpoch,
    /// The header's device is not a member at the position (or, for the producer's own check, of
    /// the epoch).
    #[error("the device is not a member of the epoch")]
    DeviceOutsideEpoch,
    /// The header's fingerprint is not the one the chain holds for its device.
    #[error("the fingerprint is not the chain's")]
    IdentityMismatch,
    /// The header's nonce domain is not the derived one.
    #[error("the nonce domain is not the derived one")]
    NonceDomain,
    /// The ciphertext does not open under the derived key and the header as AAD.
    #[error("the ciphertext does not open")]
    NotOpenable,
}

/// The receiver path with its reasons. [`ReceiverOpener`] and [`classify_envelope`] both use it,
/// so there is a single implementation of the checks.
fn open_typed(
    key: &EpochKey,
    chain: &VerifiedState,
    window: MemberWindow,
    container: &ParsedContainer<'_>,
) -> Result<Vec<u8>, OpenFailure> {
    let header = &container.header;
    if header.object_type() != OBJECT_ENVELOPE {
        return Err(OpenFailure::NotAnEnvelope);
    }
    if header.vault_id != key.vault_id {
        return Err(OpenFailure::WrongVault);
    }
    if header.key_epoch != key.key_epoch {
        return Err(OpenFailure::WrongEpoch);
    }
    // The fingerprint comes from the chain, for the epoch the envelope claims and the key
    // belongs to; the header must restate it.
    let member = match window {
        MemberWindow::InEpoch => chain.authorized_member_in_epoch(&header.device_id, key.key_epoch),
        MemberWindow::AtVersion(version) => {
            chain.authorized_member_at_version(&header.device_id, version)
        }
    }
    .ok_or(OpenFailure::DeviceOutsideEpoch)?;
    if member.fingerprint != header.device_key_fingerprint {
        return Err(OpenFailure::IdentityMismatch);
    }
    let domain = NonceDomain::derive(
        &key.vault_id,
        key.key_epoch,
        &member.fingerprint,
        OBJECT_ENVELOPE,
    );
    if header.nonce_domain != domain {
        return Err(OpenFailure::NonceDomain);
    }
    let aead = aead_key(key, &member.fingerprint, ObjectType::Envelope);
    Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(aead.expose()))
        .decrypt(
            Nonce::from_slice(&gcm_nonce(&domain, header.invocation_counter)),
            Payload {
                msg: container.ciphertext,
                aad: container.header_bytes,
            },
        )
        .map_err(|_| OpenFailure::NotOpenable)
}

impl EnvelopeOpener for ReceiverOpener<'_> {
    fn open(&self, container: &ParsedContainer<'_>) -> Option<Vec<u8>> {
        open_typed(self.key, self.chain, self.window, container).ok()
    }
}

/// Why an envelope is **invalid** (C.4 "When an envelope is found invalid"). The verdict class is
/// the contract; the reason is informative and for diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum InvalidReason {
    /// The container or header does not parse (no key or chain condition).
    Unparseable,
    /// The header names another vault than the replica's own (no key condition: the replica
    /// knows its vault from its chain).
    WrongVault,
    /// The header's `key_epoch` is not the epoch in force at the envelope's log position (C.7).
    EpochNotInForce,
    /// The open step refused it, for the reason inside.
    Open(OpenFailure),
    /// The opened plaintext does not decode.
    PlaintextMalformed,
    /// A receiver check of `check_envelope` rejected it.
    Check(ReceiverError),
}

/// Why a replica cannot yet judge an envelope. Never a basis to honor a void.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Wait {
    /// The replica has not processed the log in order up to this position: the epoch record that
    /// fixes the epoch and membership in force there is not verified yet.
    ChainBehind,
    /// The replica holds no verified key for the header's epoch.
    NoVerifiedKey,
    /// The header's `engine_version` is not this build's (C.13, R40): the replica forms no
    /// verdict at all, because every later stage runs checks that are bound to the engine
    /// version. It waits for a build of the envelope's version, or a snapshot past it.
    UpdateRequired,
}

/// A replica's verdict on the envelope at one log position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Authentic, consistent and checked: apply it.
    Valid,
    /// Found invalid: voidable.
    Invalid(InvalidReason),
    /// Cannot be judged yet: wait, retry, fetch more. A void for it is inert.
    NotDeterminable(Wait),
}

/// Classify the envelope at one log position the way C.4 "When an envelope is found invalid"
/// defines it: **invalid** only with the evidence, **not determinable** while the replica's key or
/// log position is missing, otherwise valid.
///
/// `own_engine_version` is this build's `engine_version` (C.13): an envelope of any other value
/// is not determinable ([`Wait::UpdateRequired`]) before anything else but the parse and the vault
/// are judged. `keys` are the replica's *verified* epoch keys. `position` is the epoch and membership in force
/// **at this position**, built from the newest epoch record positioned before it (see
/// [`EpochAtPosition`]); `None` means the replica has not processed the log in order up to this
/// position (that record is not verified yet). The verdict then does not depend on how far past
/// the position the replica has verified. `tag_for` is the receiver's idempotency-key tagger, as
/// for `check_envelope`.
#[must_use]
pub fn classify_envelope(
    container_bytes: &[u8],
    own_engine_version: &str,
    keys: &[EpochKey],
    chain: &VerifiedState,
    position: Option<&EpochAtPosition>,
    tag_for: impl FnOnce(&sync_core::envelope::IdempotencyKey) -> sync_core::header::IdempotencyKeyTag,
) -> Verdict {
    use sync_core::envelope::{check_envelope, EnvelopePlaintext};
    // 1. A container or header that does not parse is invalid with no key or chain condition.
    let Ok(container) =
        sync_core::header::parse_container_with_limit(container_bytes, MAX_ENVELOPE_CONTAINER_LEN)
    else {
        return Verdict::Invalid(InvalidReason::Unparseable);
    };
    let header = &container.header;
    // 1b. A header that names another vault is invalid with no key or position condition: the
    //     replica knows its own vault from its chain.
    if header.vault_id != *chain.vault_id() {
        return Verdict::Invalid(InvalidReason::WrongVault);
    }
    // 1c. The version gate (C.13, R40): the receiver checks and the apply semantics are bound to
    //     the engine version, so a header of another version is not judged at all. Without this
    //     a replica could find invalid what the sealing version finds valid, and the vault would
    //     split by version on a void.
    if header.engine_version != own_engine_version {
        return Verdict::NotDeterminable(Wait::UpdateRequired);
    }
    // 2. Not judgeable until the log has been processed in order up to this position.
    let Some(position) = position else {
        return Verdict::NotDeterminable(Wait::ChainBehind);
    };
    // 3. Any epoch other than the one in force at this position, past or future, is invalid by
    //    position alone and needs no key (C.7).
    if header.key_epoch != position.key_epoch() {
        return Verdict::Invalid(InvalidReason::EpochNotInForce);
    }
    // 4. Judging the rest needs the verified key for this epoch.
    let Some(key) = keys
        .iter()
        .find(|k| k.key_epoch == header.key_epoch && k.vault_id == header.vault_id)
    else {
        return Verdict::NotDeterminable(Wait::NoVerifiedKey);
    };
    // 5 and 6. Device membership at the position, restated identity, nonce domain, AEAD.
    let plaintext_bytes = match open_typed(
        key,
        chain,
        MemberWindow::AtVersion(position.membership_head_version()),
        &container,
    ) {
        Ok(bytes) => bytes,
        Err(failure) => return Verdict::Invalid(InvalidReason::Open(failure)),
    };
    // 7. The opened plaintext must decode.
    let Ok(plaintext) = EnvelopePlaintext::decode(&plaintext_bytes) else {
        return Verdict::Invalid(InvalidReason::PlaintextMalformed);
    };
    // 8. The receiver checks.
    match check_envelope(header, &plaintext, tag_for) {
        Ok(_) => Verdict::Valid,
        Err(error) => Verdict::Invalid(InvalidReason::Check(error)),
    }
}
