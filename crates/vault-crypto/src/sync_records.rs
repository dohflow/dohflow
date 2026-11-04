//! Building and accepting the signed records of Sync's key custody
//! (`docs/architecture/vault-format.md` C.6; personal-cfo-1cltt.2).
//!
//! Two directions, and one path between them:
//!
//! * **Building.** [`create_genesis`] makes the founding device's membership record (C.6 chain
//!   rule 1: version 1, action `add`, signed by the device it adds) and the reason-1 epoch record
//!   that seals fresh `K_1` and `S` to that device alone. [`seal_epoch_record`] is the general
//!   sealer (recipients sorted ascending by fingerprint, a fresh ephemeral key for each, never a
//!   sender context reused across recipients) for the grants and cutovers that later beads drive.
//!   [`sign_membership`] and [`sign_epoch`] frame a body as a signed record.
//! * **Accepting.** [`accept_records`] is the recipient's path of C.6 ("A recipient accepts an epoch
//!   record only when it ..."): first `sync_core::membership::extend` (strict signature, the
//!   sealer authorized at its own named head, the reason table, exactly the right recipients, `S`
//!   unchanged, a fresh `K_e` for a key-changing reason), **then** this device's entry is unsealed,
//!   and **then** both keys are checked against the record's commitments. A record that fails the
//!   first step never reaches HPKE. Nothing is returned for the keys unless every record of the
//!   walk passed, so a caller installs all of them or none.
//! * **The one path.** The device that builds a record also accepts it ([`create_genesis`] runs its
//!   own output through [`accept_records`] before returning it): the founding device installs `K_1`
//!   and `S` by accepting its own genesis record, never from the values it generated. That is the
//!   epoch-record counterpart of the producer's seal-then-verify rule.
//!
//! Nothing here logs. Errors name a record by its position and carry no key, `enc` or plaintext.

use sha2::{Digest, Sha256};
use sync_core::membership::{
    extend, Action, ChainError, EpochBody, FailureKind, MembershipBody, Reason, Recipient,
    SignedRecord, VerifiedState, KEM_ID, MAX_RECIPIENTS, SUITE_ID,
};

use crate::sync::TagSecret;
use crate::sync_envelope::EpochKey;
use crate::sync_keys::{
    epoch_key_commitment, generate_secret, tag_key_commitment, DeviceKeys, DevicePublic,
};
use crate::sync_keyset::{seal_key_set, unseal_key_set, KeySetError, SealedKeySet};

/// An epoch body is at most a header and 1,024 recipients of at most 2,364 bytes each; the same
/// bound `sync-core` parses under.
const MAX_EPOCH_BODY: usize = 512 + MAX_RECIPIENTS * 2364;

/// Why a record was not built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum BuildError {
    /// The system random source failed.
    #[error("the system random source failed")]
    Rng,
    /// A recipient's agreement key is one the chain refuses.
    #[error("a recipient's agreement key is not acceptable")]
    UnacceptableRecipientKey,
    /// HPKE refused to seal.
    #[error("a key set could not be sealed")]
    Seal,
    /// Two recipients share a fingerprint, or there are more than the format allows.
    #[error("the recipient list is not valid")]
    BadRecipients,
    /// The record cannot be encoded within the format's bounds.
    #[error("the record cannot be encoded")]
    Unencodable,
    /// The sealer's own recipient path refused the record it just built: a bug, or a request that
    /// no verifier would accept. Nothing is offered.
    #[error("the sealer's own acceptance of its record failed: {0}")]
    NotAcceptedBySelf(AcceptError),
}

impl From<KeySetError> for BuildError {
    fn from(error: KeySetError) -> Self {
        match error {
            KeySetError::Rng => Self::Rng,
            KeySetError::UnacceptableRecipientKey => Self::UnacceptableRecipientKey,
            _ => Self::Seal,
        }
    }
}

/// Why a record or a walk of records was not accepted. Nothing from a rejected walk is installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AcceptError {
    /// The chain itself failed verification (`ChainError::kind` is the C.11 name). Raised before any
    /// key is unsealed.
    #[error("the membership or epoch chain failed verification: {0}")]
    Chain(ChainError),
    /// The device's entry did not open, was not a key set, or was missing from a record that must
    /// list the device (C.11 `EpochRecordInvalid`).
    #[error("epoch record {at}: the sealed key set is invalid")]
    EpochRecordInvalid {
        /// Which epoch record of this walk.
        at: usize,
    },
    /// A key opened, and it is not the one the record commits to (C.11 `KeyCommitmentMismatch`).
    #[error("epoch record {at}: a key does not match its commitment")]
    KeyCommitmentMismatch {
        /// Which epoch record of this walk.
        at: usize,
    },
}

impl AcceptError {
    /// The C.11 failure name.
    #[must_use]
    pub fn name(&self) -> String {
        match self {
            Self::Chain(error) => error.name(),
            Self::EpochRecordInvalid { .. } => FailureKind::EpochRecordInvalid.name().to_owned(),
            Self::KeyCommitmentMismatch { .. } => "KeyCommitmentMismatch".to_owned(),
        }
    }

    /// Whether the failure is the halt class (a fork or an equivocation).
    #[must_use]
    pub fn halts_sync(&self) -> bool {
        matches!(self, Self::Chain(error) if error.halts_sync())
    }
}

impl From<ChainError> for AcceptError {
    fn from(error: ChainError) -> Self {
        Self::Chain(error)
    }
}

/// A signed membership record: `u32 body_len ‖ body ‖ signature`, by `keys`.
///
/// # Errors
/// [`BuildError::Unencodable`].
pub fn sign_membership(keys: &DeviceKeys, body: &MembershipBody) -> Result<Vec<u8>, BuildError> {
    sign(keys, &body.encode().map_err(|_| BuildError::Unencodable)?)
}

/// A signed epoch record, by `keys`.
///
/// # Errors
/// [`BuildError::Unencodable`].
pub fn sign_epoch(keys: &DeviceKeys, body: &EpochBody) -> Result<Vec<u8>, BuildError> {
    sign(keys, &body.encode().map_err(|_| BuildError::Unencodable)?)
}

fn sign(keys: &DeviceKeys, body: &[u8]) -> Result<Vec<u8>, BuildError> {
    SignedRecord {
        body: body.to_vec(),
        signature: keys.sign(body),
    }
    .encode()
    .map_err(|_| BuildError::Unencodable)
}

/// A device an epoch record seals to: its id and public keys, from its `add` record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecipientKeys {
    /// The recipient's device id.
    pub device_id: [u8; 16],
    /// Its public keys and fingerprint.
    pub public: DevicePublic,
}

/// What an epoch record is for and what it seals. The vault and the key epoch come from the
/// verified key, so a record cannot seal one epoch's key under another's number.
pub struct SealSpec<'a> {
    /// Why the record exists.
    pub reason: Reason,
    /// The id of the newest membership record the record covers.
    pub membership_head: [u8; 32],
    /// The previous epoch record's id; absent only for the genesis.
    pub prev_epoch_id: Option<[u8; 32]>,
    /// The sealing device.
    pub sealer: &'a DeviceKeys,
    /// `K_e` of the epoch the record establishes.
    pub epoch_key: &'a EpochKey,
    /// `S`.
    pub tag_secret: &'a TagSecret,
    /// The recipients, in any order (the record sorts them).
    pub recipients: &'a [RecipientKeys],
}

/// Build and sign one epoch record sealing `K_e` and `S` to each recipient (C.6).
///
/// # Errors
/// [`BuildError`].
pub fn seal_epoch_record(spec: &SealSpec<'_>) -> Result<Vec<u8>, BuildError> {
    let mut recipients: Vec<&RecipientKeys> = spec.recipients.iter().collect();
    recipients.sort_by(|a, b| {
        a.public
            .fingerprint
            .as_bytes()
            .cmp(b.public.fingerprint.as_bytes())
    });
    if recipients.len() > MAX_RECIPIENTS
        || recipients.windows(2).any(|pair| {
            pair[0].public.fingerprint.as_bytes() == pair[1].public.fingerprint.as_bytes()
        })
    {
        return Err(BuildError::BadRecipients);
    }
    // The header does not depend on the recipient list, so its hash (which every entry binds) is
    // known before any entry is sealed.
    let mut body = EpochBody {
        suite_id: SUITE_ID,
        kem_id: KEM_ID,
        vault_id: *spec.epoch_key.vault_id(),
        key_epoch: spec.epoch_key.key_epoch(),
        reason: spec.reason,
        membership_head: spec.membership_head,
        prev_epoch_id: spec.prev_epoch_id,
        key_commitment_ke: epoch_key_commitment(spec.epoch_key.secret()),
        key_commitment_s: tag_key_commitment(spec.tag_secret.secret()),
        sealer_fingerprint: spec.sealer.public().fingerprint,
        recipients: Vec::new(),
    };
    let mut entries = Vec::with_capacity(recipients.len());
    for recipient in recipients {
        let fingerprint = recipient.public.fingerprint;
        // A fresh ephemeral key per recipient: `seal_key_set` draws its own seed each call.
        let sealed = seal_key_set(
            &recipient.public.kem_public_key,
            &body.hpke_info(&recipient.device_id, &fingerprint),
            &body.hpke_aad(&fingerprint),
            spec.epoch_key.secret(),
            spec.tag_secret.secret(),
        )?;
        entries.push(Recipient {
            device_id: recipient.device_id,
            fingerprint,
            suite_id: SUITE_ID,
            kem_id: KEM_ID,
            enc: sealed.enc,
            ciphertext: sealed.ciphertext,
        });
    }
    body.recipients = entries;
    sign_epoch(spec.sealer, &body)
}

/// The two records of a founding device (C.6: "The genesis membership record is installed with the
/// genesis epoch record").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenesisRecords {
    /// The signed membership record (version 1, `add`).
    pub membership: Vec<u8>,
    /// The signed reason-1 epoch record, sealed to the founding device only.
    pub epoch: Vec<u8>,
}

/// Create a vault's Sync genesis on the founding device: generate `K_1` and `S`, sign the device's
/// own `add`, seal the key set to it, and run the result through [`accept_records`] before
/// returning it. The generated keys do not leave this function; the caller installs them by
/// accepting the returned records.
///
/// # Errors
/// [`BuildError`].
pub fn create_genesis(
    vault_id: [u8; 16],
    device_id: [u8; 16],
    keys: &DeviceKeys,
    issued_at: i64,
) -> Result<GenesisRecords, BuildError> {
    let public = keys.public();
    let membership_body = MembershipBody {
        suite_id: SUITE_ID,
        kem_id: KEM_ID,
        vault_id,
        version: 1,
        prev_id: None,
        action: Action::Add,
        signer_fingerprint: public.fingerprint,
        device_id,
        ed25519_pk: public.ed25519_pk,
        kem_public_key: public.kem_public_key,
        issued_at,
    };
    let membership = sign_membership(keys, &membership_body)?;
    let head = record_id(&membership).ok_or(BuildError::Unencodable)?;
    let epoch_root = generate_secret().map_err(|_| BuildError::Rng)?;
    let epoch_commitment = epoch_key_commitment(&epoch_root);
    let epoch_key = EpochKey::from_verified(vault_id, 1, epoch_root, &epoch_commitment)
        .map_err(|_| BuildError::Rng)?;
    let tag_root = generate_secret().map_err(|_| BuildError::Rng)?;
    let tag_commitment = tag_key_commitment(&tag_root);
    let tag_secret =
        TagSecret::from_verified(tag_root, &tag_commitment).map_err(|_| BuildError::Rng)?;
    let epoch = seal_epoch_record(&SealSpec {
        reason: Reason::Genesis,
        membership_head: head,
        prev_epoch_id: None,
        sealer: keys,
        epoch_key: &epoch_key,
        tag_secret: &tag_secret,
        recipients: &[RecipientKeys { device_id, public }],
    })?;
    accept_records(
        &VerifiedState::empty(vault_id),
        &[&membership],
        &[&epoch],
        keys,
    )
    .map_err(BuildError::NotAcceptedBySelf)?;
    Ok(GenesisRecords { membership, epoch })
}

/// The id of a signed record's bytes: `SHA-256(body)`.
fn record_id(signed: &[u8]) -> Option<[u8; 32]> {
    SignedRecord::parse(signed, MAX_EPOCH_BODY)
        .ok()
        .map(|record| record.id())
}

/// The key of one epoch that this device holds a sealed entry for, with the id of the record that
/// carried it.
#[derive(Debug)]
pub struct EpochInstall {
    /// The id of the epoch record that sealed it.
    pub record_id: [u8; 32],
    /// `K_e`, built only through its commitment.
    pub epoch_key: EpochKey,
}

/// A verified walk: the new chain state, and every key this device unsealed.
#[derive(Debug)]
pub struct Accepted {
    /// The state after every record of the walk passed.
    pub state: VerifiedState,
    /// `K_e` of each epoch record of the walk that listed this device, in chain order. A record
    /// that did not list it (a grant to someone else) yields none: the device already holds that
    /// epoch's key.
    pub installs: Vec<EpochInstall>,
    /// `S`, if any record of the walk listed this device. It never changes, so every record that
    /// yields it yields the same value.
    pub tag_secret: Option<TagSecret>,
}

/// The recipient's acceptance of a suffix of the chain (C.6): verify every record, then unseal this
/// device's entries and check both keys against the commitments. See the module doc for the order.
///
/// `state` is the last verified state (empty for a device that has none); `memberships` and
/// `epochs` are the complete suffix after it, in chain order. A failure anywhere returns the error
/// and nothing else: no key, no state.
///
/// # Errors
/// [`AcceptError`].
pub fn accept_records(
    state: &VerifiedState,
    memberships: &[&[u8]],
    epochs: &[&[u8]],
    device: &DeviceKeys,
) -> Result<Accepted, AcceptError> {
    // Step 1, with no key in play: signatures, linkage, authorization, reasons, recipients.
    let verified = extend(state, memberships, epochs)?;
    let my_fingerprint = device.public().fingerprint;
    let mut installs = Vec::new();
    let mut tag_secret = None;
    for (at, bytes) in epochs.iter().enumerate() {
        let invalid = AcceptError::EpochRecordInvalid { at };
        let record = SignedRecord::parse(bytes, MAX_EPOCH_BODY).map_err(|_| invalid)?;
        let body = EpochBody::decode(&record.body).map_err(|_| invalid)?;
        debug_assert_eq!(body.suite_id, SUITE_ID);
        let head_version = verified
            .membership_version_of(&body.membership_head)
            .ok_or(invalid)?;
        let me =
            verified.authorized_member_by_fingerprint_at_version(&my_fingerprint, head_version);
        let entry = body
            .recipients
            .iter()
            .find(|r| r.fingerprint == my_fingerprint);
        let Some(entry) = entry else {
            // A grant lists only the devices it adds; every other reason lists every member. A
            // device that must be listed and is not is an invalid record, never a silent skip.
            let required = match body.reason {
                Reason::Grant => me.is_some_and(|m| m.added_at_version == head_version),
                _ => me.is_some(),
            };
            if required {
                return Err(invalid);
            }
            continue;
        };
        // Step 2: unseal. `info` and `aad` are rebuilt from the verified header, never taken from
        // the entry.
        let unsealed = unseal_key_set(
            device,
            &SealedKeySet {
                enc: entry.enc,
                ciphertext: entry.ciphertext.clone(),
            },
            &body.hpke_info(&entry.device_id, &entry.fingerprint),
            &body.hpke_aad(&entry.fingerprint),
        )
        .map_err(|_| invalid)?;
        // Step 3: the keys are usable only through their commitments.
        let mismatch = AcceptError::KeyCommitmentMismatch { at };
        let epoch_key = EpochKey::from_verified(
            body.vault_id,
            body.key_epoch,
            unsealed.epoch_root,
            &body.key_commitment_ke,
        )
        .map_err(|_| mismatch)?;
        tag_secret = Some(
            TagSecret::from_verified(unsealed.tag_secret, &body.key_commitment_s)
                .map_err(|_| mismatch)?,
        );
        installs.push(EpochInstall {
            record_id: Sha256::digest(&record.body).into(),
            epoch_key,
        });
    }
    Ok(Accepted {
        state: verified,
        installs,
        tag_secret,
    })
}
