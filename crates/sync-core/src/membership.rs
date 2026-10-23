//! The membership chain and epoch records of `docs/architecture/vault-format.md` C.6, and a pure
//! reference verifier for them (personal-cfo-g3m.21).
//!
//! The service stores and enforces the chain; every client re-verifies it; a TypeScript device
//! registry and the Sync server pin their behavior to this crate's public conformance vectors
//! (`docs/architecture/vectors/membership-chain-v1.json`). So the rules here are the rules of C.6,
//! applied in the order the spec gives them, and every refusal is a typed [`ChainError`] that names
//! the C.11 failure it is.
//!
//! What is verified: strict Ed25519 signatures, `prev_id` linkage and versions, the signer being an
//! authorized member **in the state after `prev_id`** (its key comes from the chain, never from
//! the record being verified), fingerprint recomputation and no reuse, `revoke` repeating its
//! `add`, and the epoch side: each membership change coupled to exactly one epoch record, the
//! reason table, the sealer authorized at its own named head, the recipient sets, and `S`
//! unchanged. What is not: HPKE unsealing and key commitments against real keys (they need
//! keys), snapshots and artifacts, C.7 epoch-position rules, persistence and rollback protection.

use std::collections::BTreeSet;

use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::cbe::{Reader, WireError, Writer};
use crate::header::{Fingerprint, PROTOCOL_VERSION};
use crate::keys::{
    ed25519_public_key_acceptable, ed25519_verify_strict, x25519_public_key_acceptable,
};

/// The one cryptographic suite (C.1.4): Ed25519, X25519/HKDF-SHA256 HPKE, AES-256-GCM.
pub const SUITE_ID: u16 = 1;
/// The one KEM (`DHKEM(X25519, HKDF-SHA256)`).
pub const KEM_ID: u16 = 0x0020;
/// In suite 1 a KEM public key is exactly this many bytes (X25519).
pub const KEM_PUBLIC_KEY_LEN: usize = 32;
/// The most recipients an epoch record may list.
pub const MAX_RECIPIENTS: usize = 1024;
/// The most memberships a verified chain may hold (a bound on work, not a rule of the spec).
pub const MAX_CHAIN_LEN: usize = 4096;
/// The most authorized members at any version: the next cutover must list every one of them.
pub const MAX_MEMBERS: usize = MAX_RECIPIENTS;
/// The exact length of a recipient's sealed key set in suite 1: the 93-byte plaintext (`label
/// "dohflow/sync-sealed-keys/v1" ‖ key32 K_e ‖ key32 S`) plus the 16-byte AES-GCM tag.
pub const SEALED_KEYS_CIPHERTEXT_LEN: usize = 109;

const MEMBERSHIP_LABEL: &str = "dohflow/sync-membership/v1";
const EPOCH_LABEL: &str = "dohflow/sync-epoch-record/v1";
const FINGERPRINT_LABEL: &str = "dohflow/device-fpr/v1";
/// A membership body is at most a label, fixed fields and a 32-byte KEM key: well under this.
const MAX_MEMBERSHIP_BODY: usize = 2560;
/// An epoch body: a header and at most 1024 recipients of at most 2,364 bytes each.
const MAX_EPOCH_BODY: usize = 512 + MAX_RECIPIENTS * 2364;
const SIGNATURE_LEN: usize = 64;

/// A fingerprint of a device's keys (C.3): `SHA-256(label "dohflow/device-fpr/v1" ‖ u16 suite_id ‖
/// u16 kem_id ‖ key32 ed25519_pk ‖ bytes(2048) kem_public_key)`.
///
/// # Panics
/// Never: the key lengths are bounded by the caller's types.
#[must_use]
pub fn device_fingerprint(
    suite_id: u16,
    kem_id: u16,
    ed25519_pk: &[u8; 32],
    kem_public_key: &[u8],
) -> Fingerprint {
    let mut w = Writer::with_label(FINGERPRINT_LABEL);
    w.u16(suite_id).u16(kem_id).fixed(ed25519_pk);
    w.bytes(2048, kem_public_key)
        .expect("a KEM public key is far below 2048 bytes");
    Fingerprint::from_derived(Sha256::digest(w.as_bytes()).into())
}

/// A signed record: `u32 body_len ‖ body ‖ 64-byte Ed25519 signature over the body bytes`.
/// Identified by `SHA-256(body)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedRecord {
    /// The body bytes, exactly as signed.
    pub body: Vec<u8>,
    /// The signature over `body`.
    pub signature: [u8; 64],
}

impl SignedRecord {
    /// Parse a container, requiring `body_len` to be at most `max_body` and nothing to follow the
    /// signature.
    ///
    /// # Errors
    /// A [`WireError`] for truncation, an oversize body or trailing bytes.
    pub fn parse(bytes: &[u8], max_body: usize) -> Result<Self, WireError> {
        let mut r = Reader::new(bytes);
        let body = r.bytes(max_body)?.to_vec();
        let signature = r.fixed::<SIGNATURE_LEN>()?;
        r.finish()?;
        Ok(Self { body, signature })
    }

    /// The container's bytes.
    ///
    /// # Errors
    /// [`WireError::TooLong`] if the body is over `u32` bytes.
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        let mut w = Writer::default();
        w.bytes(usize::MAX, &self.body)?;
        w.fixed(&self.signature);
        Ok(w.finish())
    }

    /// The record's id, `SHA-256(body)`.
    #[must_use]
    pub fn id(&self) -> [u8; 32] {
        Sha256::digest(&self.body).into()
    }
}

/// What a membership record does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// `1`: add a device.
    Add,
    /// `2`: revoke a device.
    Revoke,
}

/// A membership record body (C.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MembershipBody {
    /// The suite (must be [`SUITE_ID`]).
    pub suite_id: u16,
    /// The KEM (must be [`KEM_ID`]).
    pub kem_id: u16,
    /// The vault.
    pub vault_id: [u8; 16],
    /// Position in the chain, starting at 1.
    pub version: u64,
    /// The previous record's id; absent only for the first.
    pub prev_id: Option<[u8; 32]>,
    /// Add or revoke.
    pub action: Action,
    /// The fingerprint of the signing device.
    pub signer_fingerprint: Fingerprint,
    /// The device added or revoked.
    pub device_id: [u8; 16],
    /// The device's Ed25519 public key.
    pub ed25519_pk: [u8; 32],
    /// The device's KEM public key (exactly 32 bytes in suite 1).
    pub kem_public_key: [u8; 32],
    /// Microseconds since the Unix epoch.
    pub issued_at: i64,
}

impl MembershipBody {
    /// The body's canonical bytes.
    ///
    /// # Errors
    /// Never in practice.
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        let mut w = Writer::with_label(MEMBERSHIP_LABEL);
        w.u16(PROTOCOL_VERSION)
            .u16(self.suite_id)
            .u16(self.kem_id)
            .fixed(&self.vault_id)
            .u64(self.version)
            .opt(self.prev_id.as_ref(), |w, id| {
                w.fixed(id);
            })
            .u8(match self.action {
                Action::Add => 1,
                Action::Revoke => 2,
            })
            .fixed(self.signer_fingerprint.as_bytes())
            .fixed(&self.device_id)
            .fixed(&self.ed25519_pk);
        w.bytes(2048, &self.kem_public_key)?;
        w.i64(self.issued_at);
        Ok(w.finish())
    }

    /// Parse a body from exactly `bytes`. A KEM public key that is not 32 bytes, an unknown action
    /// and a protocol version other than 1 are refused.
    ///
    /// # Errors
    /// A [`WireError`].
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut r = Reader::new(bytes);
        r.label(MEMBERSHIP_LABEL)?;
        if r.u16()? != PROTOCOL_VERSION {
            return Err(WireError::UnsupportedProtocol);
        }
        let suite_id = r.u16()?;
        let kem_id = r.u16()?;
        let vault_id = r.fixed::<16>()?;
        let version = r.u64()?;
        let prev_id = r.opt(|r| r.fixed::<32>())?;
        let action = match r.u8()? {
            1 => Action::Add,
            2 => Action::Revoke,
            _ => return Err(WireError::UnknownObjectType),
        };
        let signer_fingerprint = Fingerprint::from_derived(r.fixed::<32>()?);
        let device_id = r.fixed::<16>()?;
        let ed25519_pk = r.fixed::<32>()?;
        let kem = r.bytes(2048)?;
        let kem_public_key: [u8; KEM_PUBLIC_KEY_LEN] =
            kem.try_into().map_err(|_| WireError::TooLong)?;
        let issued_at = r.i64()?;
        r.finish()?;
        Ok(Self {
            suite_id,
            kem_id,
            vault_id,
            version,
            prev_id,
            action,
            signer_fingerprint,
            device_id,
            ed25519_pk,
            kem_public_key,
            issued_at,
        })
    }

    /// The fingerprint recomputed from this record's own keys.
    #[must_use]
    pub fn fingerprint(&self) -> Fingerprint {
        device_fingerprint(
            self.suite_id,
            self.kem_id,
            &self.ed25519_pk,
            &self.kem_public_key,
        )
    }
}

/// Why an epoch record exists (C.6 reason table).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// `1`: the first record, founding device.
    Genesis,
    /// `2`: a revocation cutover (fresh key).
    Cutover,
    /// `3`: a grant to a newly added device (same key).
    Grant,
    /// `4`: a forced rotation (fresh key, no membership change).
    Rotation,
}

/// One sealed key set entry of an epoch record. The ciphertext is opaque here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recipient {
    /// The recipient device.
    pub device_id: [u8; 16],
    /// The recipient's fingerprint.
    pub fingerprint: Fingerprint,
    /// The suite.
    pub suite_id: u16,
    /// The KEM.
    pub kem_id: u16,
    /// The HPKE encapsulated key (exactly 32 bytes in suite 1).
    pub enc: [u8; 32],
    /// The sealed key set, at most 256 bytes.
    pub ciphertext: Vec<u8>,
}

/// An epoch record body (C.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpochBody {
    /// The suite.
    pub suite_id: u16,
    /// The KEM.
    pub kem_id: u16,
    /// The vault.
    pub vault_id: [u8; 16],
    /// The key epoch the record establishes.
    pub key_epoch: u64,
    /// Why the record exists.
    pub reason: Reason,
    /// The id of the newest membership record this epoch record covers.
    pub membership_head: [u8; 32],
    /// The previous epoch record's id; absent only for the first.
    pub prev_epoch_id: Option<[u8; 32]>,
    /// The commitment to `K_e`.
    pub key_commitment_ke: [u8; 32],
    /// The commitment to `S`.
    pub key_commitment_s: [u8; 32],
    /// The sealing device's fingerprint.
    pub sealer_fingerprint: Fingerprint,
    /// The sealed key sets.
    pub recipients: Vec<Recipient>,
}

impl EpochBody {
    /// The body's canonical bytes.
    ///
    /// # Errors
    /// A [`WireError`] for an over-long list or ciphertext.
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        let mut w = Writer::with_label(EPOCH_LABEL);
        w.u16(PROTOCOL_VERSION)
            .u16(self.suite_id)
            .u16(self.kem_id)
            .fixed(&self.vault_id)
            .u64(self.key_epoch)
            .u8(match self.reason {
                Reason::Genesis => 1,
                Reason::Cutover => 2,
                Reason::Grant => 3,
                Reason::Rotation => 4,
            })
            .fixed(&self.membership_head)
            .opt(self.prev_epoch_id.as_ref(), |w, id| {
                w.fixed(id);
            })
            .fixed(&self.key_commitment_ke)
            .fixed(&self.key_commitment_s)
            .fixed(self.sealer_fingerprint.as_bytes());
        if self.recipients.len() > MAX_RECIPIENTS {
            return Err(WireError::TooLong);
        }
        // One canonical order: strictly ascending by fingerprint, so one set has one encoding.
        if self
            .recipients
            .windows(2)
            .any(|pair| pair[0].fingerprint.as_bytes() >= pair[1].fingerprint.as_bytes())
        {
            return Err(WireError::ListNotAscending);
        }
        w.u32(u32::try_from(self.recipients.len()).map_err(|_| WireError::TooLong)?);
        for recipient in &self.recipients {
            if recipient.ciphertext.len() != SEALED_KEYS_CIPHERTEXT_LEN {
                return Err(WireError::TooLong);
            }
            w.fixed(&recipient.device_id)
                .fixed(recipient.fingerprint.as_bytes())
                .u16(recipient.suite_id)
                .u16(recipient.kem_id);
            w.bytes(2048, &recipient.enc)?;
            w.bytes(256, &recipient.ciphertext)?;
        }
        Ok(w.finish())
    }

    /// Parse a body from exactly `bytes`.
    ///
    /// # Errors
    /// A [`WireError`].
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut r = Reader::new(bytes);
        r.label(EPOCH_LABEL)?;
        if r.u16()? != PROTOCOL_VERSION {
            return Err(WireError::UnsupportedProtocol);
        }
        let suite_id = r.u16()?;
        let kem_id = r.u16()?;
        let vault_id = r.fixed::<16>()?;
        let key_epoch = r.u64()?;
        let reason = match r.u8()? {
            1 => Reason::Genesis,
            2 => Reason::Cutover,
            3 => Reason::Grant,
            4 => Reason::Rotation,
            _ => return Err(WireError::UnknownObjectType),
        };
        let membership_head = r.fixed::<32>()?;
        let prev_epoch_id = r.opt(|r| r.fixed::<32>())?;
        let key_commitment_ke = r.fixed::<32>()?;
        let key_commitment_s = r.fixed::<32>()?;
        let sealer_fingerprint = Fingerprint::from_derived(r.fixed::<32>()?);
        // A recipient is at least 16 + 32 + 2 + 2 + 4 + 4 bytes on the wire.
        let recipients = r.list(MAX_RECIPIENTS, 60, |r| {
            let device_id = r.fixed::<16>()?;
            let fingerprint = Fingerprint::from_derived(r.fixed::<32>()?);
            let suite_id = r.u16()?;
            let kem_id = r.u16()?;
            let enc: [u8; 32] = r.bytes(2048)?.try_into().map_err(|_| WireError::TooLong)?;
            let ciphertext = r.bytes(256)?.to_vec();
            if ciphertext.len() != SEALED_KEYS_CIPHERTEXT_LEN {
                return Err(WireError::TooLong);
            }
            Ok(Recipient {
                device_id,
                fingerprint,
                suite_id,
                kem_id,
                enc,
                ciphertext,
            })
        })?;
        r.finish()?;
        if recipients
            .windows(2)
            .any(|pair| pair[0].fingerprint.as_bytes() >= pair[1].fingerprint.as_bytes())
        {
            return Err(WireError::ListNotAscending);
        }
        Ok(Self {
            suite_id,
            kem_id,
            vault_id,
            key_epoch,
            reason,
            membership_head,
            prev_epoch_id,
            key_commitment_ke,
            key_commitment_s,
            sealer_fingerprint,
            recipients,
        })
    }
}

/// Why a chain failed verification. `kind` names the C.11 failure; positions are zero-based
/// indices into the records passed to one call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ChainError {
    /// A record is malformed (any CBE1 or container violation).
    #[error("record {at}: {error}")]
    Malformed {
        /// Which record.
        at: usize,
        /// Why.
        error: WireError,
    },
    /// A protocol version this build does not implement (update required, not corrupt).
    #[error("record {at}: unsupported protocol version")]
    UnsupportedProtocol {
        /// Which record.
        at: usize,
    },
    /// A suite or KEM this build does not implement.
    #[error("record {at}: unsupported suite")]
    UnsupportedSuite {
        /// Which record.
        at: usize,
    },
    /// The chain is longer than [`MAX_CHAIN_LEN`].
    #[error("the membership chain is too long")]
    TooLong,
    /// A record names another vault.
    #[error("record {at}: another vault")]
    VaultMismatch {
        /// Which record.
        at: usize,
    },
    /// The first record breaks chain rule 1.
    #[error("record {at}: the genesis record breaks rule 1")]
    BadGenesis {
        /// Which record.
        at: usize,
    },
    /// A record's version is not the previous plus one.
    #[error("record {at}: wrong version")]
    WrongVersion {
        /// Which record.
        at: usize,
    },
    /// A record's `prev_id` is not the previous record's id.
    #[error("record {at}: wrong prev_id")]
    WrongPrevId {
        /// Which record.
        at: usize,
    },
    /// The signature does not verify strictly under the signer's key from the chain.
    #[error("record {at}: bad signature")]
    BadSignature {
        /// Which record.
        at: usize,
    },
    /// The signer's fingerprint is not that of any member of the chain.
    #[error("record {at}: the signer is not a member")]
    SignerUnknown {
        /// Which record.
        at: usize,
    },
    /// The signer was revoked before this record.
    #[error("record {at}: the signer is revoked")]
    SignerRevoked {
        /// Which record.
        at: usize,
    },
    /// A genesis `signer_fingerprint` that is not the one recomputed from the record's keys.
    #[error("record {at}: fingerprint mismatch")]
    FingerprintMismatch {
        /// Which record.
        at: usize,
    },
    /// An Ed25519 or KEM public key that is not acceptable (invalid, non-canonical or small
    /// order).
    #[error("record {at}: unacceptable public key")]
    KeyUnacceptable {
        /// Which record.
        at: usize,
    },
    /// An `add` of a fingerprint already in the chain.
    #[error("record {at}: fingerprint reused")]
    FingerprintReused {
        /// Which record.
        at: usize,
    },
    /// An `add` of a `device_id` already in the chain (even a revoked one).
    #[error("record {at}: device id reused")]
    DeviceIdReused {
        /// Which record.
        at: usize,
    },
    /// An `add` that reuses another member's Ed25519 or KEM public key under a new fingerprint.
    #[error("record {at}: public key reused")]
    KeyReused {
        /// Which record.
        at: usize,
    },
    /// An `add` that would leave more than [`MAX_MEMBERS`] authorized members.
    #[error("record {at}: too many members")]
    TooManyMembers {
        /// Which record.
        at: usize,
    },
    /// A `revoke` that does not repeat its `add`'s device id and keys exactly.
    #[error("record {at}: revoke does not repeat the add")]
    RevokeMismatch {
        /// Which record.
        at: usize,
    },
    /// A `revoke` of a fingerprint that is not currently authorized.
    #[error("record {at}: revoke of a device that is not authorized")]
    RevokeOfUnauthorized {
        /// Which record.
        at: usize,
    },
    /// A `revoke` that would leave no authorized member (its cutover could never be sealed).
    #[error("record {at}: revoke would leave no authorized member")]
    WouldEmptyChain {
        /// Which record.
        at: usize,
    },
    /// Two different membership records for one version (`MembershipChainFork`). Reported only
    /// when the second record would itself have been valid at that position (right `prev_id`,
    /// signer authorized there, signature valid under the chain's key), so garbage injected at an
    /// accepted position cannot trigger the halt class. Both ids are carried so the caller can
    /// retain and show both records.
    #[error("two membership records share version {version}")]
    Fork {
        /// The version.
        version: u64,
        /// The id of the record already accepted at that version.
        accepted: [u8; 32],
        /// The id of the conflicting, otherwise valid record.
        conflicting: [u8; 32],
    },
    /// A membership record that no epoch record names (`EpochRecordMissing`).
    #[error("membership version {version} has no epoch record")]
    EpochRecordMissing {
        /// The version.
        version: u64,
    },
    /// Two different epoch records follow one `prev_epoch_id` (`EpochEquivocation`). Reported only
    /// when the second record's sealer is authorized at its named head and its signature is valid.
    #[error("epoch equivocation")]
    EpochEquivocation {
        /// The id of the epoch record already accepted.
        accepted: [u8; 32],
        /// The id of the conflicting, otherwise valid record.
        conflicting: [u8; 32],
    },
    /// A cutover epoch record that does not name a `revoke` record.
    #[error("epoch {at}: a cutover must name a revoke")]
    CutoverHeadNotRevoke {
        /// Which epoch record.
        at: usize,
    },
    /// A grant epoch record that does not name an `add` record.
    #[error("epoch {at}: a grant must name an add")]
    GrantHeadNotAdd {
        /// Which epoch record.
        at: usize,
    },
    /// An epoch record out of sequence: `prev_epoch_id`, `key_epoch`, `reason`, head, or the `S`
    /// and `K_e` commitments do not follow the reason table.
    #[error("epoch {at}: bad epoch sequence")]
    BadEpochSequence {
        /// Which epoch record.
        at: usize,
    },
    /// An epoch record names a membership head that is not in the verified chain.
    #[error("epoch {at}: unknown membership head")]
    UnknownHead {
        /// Which epoch record.
        at: usize,
    },
    /// The recipient list is not the one the reason requires (extra, missing, or wrong values).
    #[error("epoch {at}: recipients do not match the reason")]
    RecipientsMismatch {
        /// Which epoch record.
        at: usize,
    },
    /// The sealer is not an authorized member at the record's own named head.
    #[error("epoch {at}: the sealer is not authorized at its head")]
    SealerNotAuthorized {
        /// Which epoch record.
        at: usize,
    },
    /// The epoch record's signature does not verify strictly.
    #[error("epoch {at}: bad signature")]
    EpochBadSignature {
        /// Which epoch record.
        at: usize,
    },
}

/// The C.11 name of a failure: the only thing conformance vectors pin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// `MalformedObject`.
    MalformedObject,
    /// `UnsupportedProtocol`.
    UnsupportedProtocol,
    /// `UnsupportedSuite`.
    UnsupportedSuite,
    /// `MembershipChainInvalid`.
    MembershipChainInvalid,
    /// `MembershipChainFork`.
    MembershipChainFork,
    /// `EpochRecordMissing`.
    EpochRecordMissing,
    /// `EpochEquivocation`.
    EpochEquivocation,
    /// `CutoverHeadNotRevoke`.
    CutoverHeadNotRevoke,
    /// `GrantHeadNotAdd`.
    GrantHeadNotAdd,
    /// `BadEpochSequence`.
    BadEpochSequence,
    /// `EpochRecordInvalid`.
    EpochRecordInvalid,
}

impl FailureKind {
    /// The C.11 spelling.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::MalformedObject => "MalformedObject",
            Self::UnsupportedProtocol => "UnsupportedProtocol",
            Self::UnsupportedSuite => "UnsupportedSuite",
            Self::MembershipChainInvalid => "MembershipChainInvalid",
            Self::MembershipChainFork => "MembershipChainFork",
            Self::EpochRecordMissing => "EpochRecordMissing",
            Self::EpochEquivocation => "EpochEquivocation",
            Self::CutoverHeadNotRevoke => "CutoverHeadNotRevoke",
            Self::GrantHeadNotAdd => "GrantHeadNotAdd",
            Self::BadEpochSequence => "BadEpochSequence",
            Self::EpochRecordInvalid => "EpochRecordInvalid",
        }
    }
}

impl ChainError {
    /// The C.11 failure this error is.
    ///
    /// A fork or an equivocation is the **halt class**: [`ChainError::halts_sync`] is true, the
    /// caller keeps both records, installs nothing and enters `SyncHalted` (C.6 "Detection and
    /// recovery"), instead of treating it as ordinary invalid input.
    #[must_use]
    pub const fn kind(&self) -> FailureKind {
        match self {
            Self::Malformed { .. } | Self::TooLong => FailureKind::MalformedObject,
            Self::UnsupportedProtocol { .. } => FailureKind::UnsupportedProtocol,
            Self::UnsupportedSuite { .. } => FailureKind::UnsupportedSuite,
            Self::Fork { .. } => FailureKind::MembershipChainFork,
            Self::EpochRecordMissing { .. } => FailureKind::EpochRecordMissing,
            Self::EpochEquivocation { .. } => FailureKind::EpochEquivocation,
            Self::CutoverHeadNotRevoke { .. } => FailureKind::CutoverHeadNotRevoke,
            Self::GrantHeadNotAdd { .. } => FailureKind::GrantHeadNotAdd,
            Self::BadEpochSequence { .. } | Self::UnknownHead { .. } => {
                FailureKind::BadEpochSequence
            }
            Self::RecipientsMismatch { .. }
            | Self::SealerNotAuthorized { .. }
            | Self::EpochBadSignature { .. } => FailureKind::EpochRecordInvalid,
            _ => FailureKind::MembershipChainInvalid,
        }
    }

    /// A short stable name for the variant, for diagnostics and tests; conformance vectors pin
    /// only [`ChainError::kind`] and the position.
    #[must_use]
    pub fn name(&self) -> String {
        let debug = format!("{self:?}");
        debug.split([' ', '{', '(']).next().unwrap_or("").to_owned()
    }

    /// Whether the caller should fetch more records and try again rather than treat the input as
    /// bad: the service has not (yet) delivered the epoch record that names the newest membership
    /// record. [`extend`] cannot tell a withheld coupled epoch record from a stale anchor, so both
    /// are this; a client engine that was shown an explicit anchor naming an older head reports it
    /// as `StaleMembershipHead`.
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        matches!(self, Self::EpochRecordMissing { .. })
    }

    /// Whether this is the halt class (`MembershipChainFork`, `EpochEquivocation`).
    #[must_use]
    pub const fn halts_sync(&self) -> bool {
        matches!(self, Self::Fork { .. } | Self::EpochEquivocation { .. })
    }

    /// The position of the offending record in the call's input, where the error has one.
    #[must_use]
    pub const fn position(&self) -> Option<usize> {
        match self {
            Self::Malformed { at, .. }
            | Self::UnsupportedProtocol { at }
            | Self::UnsupportedSuite { at }
            | Self::VaultMismatch { at }
            | Self::BadGenesis { at }
            | Self::WrongVersion { at }
            | Self::WrongPrevId { at }
            | Self::BadSignature { at }
            | Self::SignerUnknown { at }
            | Self::SignerRevoked { at }
            | Self::FingerprintMismatch { at }
            | Self::KeyUnacceptable { at }
            | Self::FingerprintReused { at }
            | Self::DeviceIdReused { at }
            | Self::KeyReused { at }
            | Self::TooManyMembers { at }
            | Self::RevokeMismatch { at }
            | Self::RevokeOfUnauthorized { at }
            | Self::WouldEmptyChain { at }
            | Self::CutoverHeadNotRevoke { at }
            | Self::GrantHeadNotAdd { at }
            | Self::BadEpochSequence { at }
            | Self::UnknownHead { at }
            | Self::RecipientsMismatch { at }
            | Self::SealerNotAuthorized { at }
            | Self::EpochBadSignature { at } => Some(*at),
            Self::TooLong
            | Self::Fork { .. }
            | Self::EpochRecordMissing { .. }
            | Self::EpochEquivocation { .. } => None,
        }
    }
}

/// A device in the chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    /// The device.
    pub device_id: [u8; 16],
    /// Its fingerprint (recomputed from its keys).
    pub fingerprint: Fingerprint,
    /// Its Ed25519 public key.
    pub ed25519_pk: [u8; 32],
    /// Its KEM public key.
    pub kem_public_key: [u8; 32],
    /// The version of the `add` record.
    pub added_at_version: u64,
    /// The version of the `revoke` record, if revoked.
    pub revoked_at_version: Option<u64>,
}

impl Member {
    /// Whether the member is authorized in the state after membership version `version`.
    #[must_use]
    pub fn authorized_at(&self, version: u64) -> bool {
        self.added_at_version <= version && self.revoked_at_version.is_none_or(|r| r > version)
    }
}

/// One verified membership record, kept to answer "who was authorized at version v" and to detect
/// a different record at an accepted position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StepRef {
    id: [u8; 32],
    action: Action,
}

/// One verified epoch record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EpochRef {
    id: [u8; 32],
    prev_epoch_id: Option<[u8; 32]>,
    key_epoch: u64,
    reason: Reason,
    /// Zero-based index of the membership record it names.
    head_index: usize,
    key_commitment_ke: [u8; 32],
}

/// Everything verified so far, and everything later checks need: the persisted state a client
/// keeps (under its anti-rollback witness, C.6 rule 5) between pulls. A new state is returned only
/// when every record passed; nothing is partially applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedState {
    vault_id: [u8; 16],
    steps: Vec<StepRef>,
    members: Vec<Member>,
    epochs: Vec<EpochRef>,
    key_commitment_s: Option<[u8; 32]>,
}

impl VerifiedState {
    /// The state before any record.
    #[must_use]
    pub fn empty(vault_id: [u8; 16]) -> Self {
        Self {
            vault_id,
            steps: Vec::new(),
            members: Vec::new(),
            epochs: Vec::new(),
            key_commitment_s: None,
        }
    }

    /// The vault.
    #[must_use]
    pub const fn vault_id(&self) -> &[u8; 16] {
        &self.vault_id
    }

    /// The id of the newest membership record, if any.
    #[must_use]
    pub fn head_id(&self) -> Option<[u8; 32]> {
        self.steps.last().map(|s| s.id)
    }

    /// The version of the newest membership record (0 for an empty state).
    #[must_use]
    pub fn head_version(&self) -> u64 {
        self.steps.len() as u64
    }

    /// The id of the newest epoch record (the anchor), if any.
    #[must_use]
    pub fn epoch_head_id(&self) -> Option<[u8; 32]> {
        self.epochs.last().map(|e| e.id)
    }

    /// The key epoch in force (0 for an empty state).
    #[must_use]
    pub fn key_epoch(&self) -> u64 {
        self.epochs.last().map_or(0, |e| e.key_epoch)
    }

    /// Every device ever added, in order, with its status.
    #[must_use]
    pub fn members(&self) -> &[Member] {
        &self.members
    }

    /// The devices currently authorized.
    pub fn authorized(&self) -> impl Iterator<Item = &Member> {
        let version = self.head_version();
        self.members
            .iter()
            .filter(move |m| m.authorized_at(version))
    }

    fn authorized_member(&self, fingerprint: &Fingerprint, version: u64) -> Option<&Member> {
        self.members
            .iter()
            .find(|m| &m.fingerprint == fingerprint && m.authorized_at(version))
    }

    /// The state's canonical bytes, for persistence.
    ///
    /// # Errors
    /// A [`WireError`] if a list is over its bound.
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        let mut w = Writer::with_label(STATE_LABEL);
        w.fixed(&self.vault_id);
        w.list(MAX_CHAIN_LEN, &self.steps, |w, step| {
            w.fixed(&step.id).u8(match step.action {
                Action::Add => 1,
                Action::Revoke => 2,
            });
        })?;
        w.list(MAX_CHAIN_LEN, &self.members, |w, m| {
            w.fixed(&m.device_id)
                .fixed(m.fingerprint.as_bytes())
                .fixed(&m.ed25519_pk)
                .fixed(&m.kem_public_key)
                .u64(m.added_at_version)
                .opt(m.revoked_at_version.as_ref(), |w, v| {
                    w.u64(*v);
                });
        })?;
        w.list(MAX_CHAIN_LEN, &self.epochs, |w, e| {
            w.fixed(&e.id)
                .opt(e.prev_epoch_id.as_ref(), |w, id| {
                    w.fixed(id);
                })
                .u64(e.key_epoch)
                .u8(reason_byte(e.reason))
                .u64(e.head_index as u64)
                .fixed(&e.key_commitment_ke);
        })?;
        w.opt(self.key_commitment_s.as_ref(), |w, s| {
            w.fixed(s);
        });
        Ok(w.finish())
    }

    /// Parse a persisted state, checking that it is internally consistent (every index in range,
    /// every status within the chain), so a damaged or tampered file is refused.
    ///
    /// # Errors
    /// A [`WireError`], or [`WireError::ObjectFieldsMismatch`] for an inconsistent state.
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut r = Reader::new(bytes);
        r.label(STATE_LABEL)?;
        let vault_id = r.fixed::<16>()?;
        let steps = r.list(MAX_CHAIN_LEN, 33, |r| {
            let id = r.fixed::<32>()?;
            let action = match r.u8()? {
                1 => Action::Add,
                2 => Action::Revoke,
                _ => return Err(WireError::UnknownObjectType),
            };
            Ok(StepRef { id, action })
        })?;
        let members = r.list(MAX_CHAIN_LEN, 105, |r| {
            Ok(Member {
                device_id: r.fixed::<16>()?,
                fingerprint: Fingerprint::from_derived(r.fixed::<32>()?),
                ed25519_pk: r.fixed::<32>()?,
                kem_public_key: r.fixed::<32>()?,
                added_at_version: r.u64()?,
                revoked_at_version: r.opt(|r| r.u64())?,
            })
        })?;
        let epochs = r.list(MAX_CHAIN_LEN, 83, |r| {
            let id = r.fixed::<32>()?;
            let prev_epoch_id = r.opt(|r| r.fixed::<32>())?;
            let key_epoch = r.u64()?;
            let reason = reason_from_byte(r.u8()?)?;
            let head_index = usize::try_from(r.u64()?).map_err(|_| WireError::TooLong)?;
            let key_commitment_ke = r.fixed::<32>()?;
            Ok(EpochRef {
                id,
                prev_epoch_id,
                key_epoch,
                reason,
                head_index,
                key_commitment_ke,
            })
        })?;
        let key_commitment_s = r.opt(|r| r.fixed::<32>())?;
        r.finish()?;
        let version_limit = steps.len() as u64;
        let consistent = members.iter().all(|m| {
            (1..=version_limit).contains(&m.added_at_version)
                && m.revoked_at_version
                    .is_none_or(|v| v > m.added_at_version && v <= version_limit)
        }) && epochs.iter().all(|e| e.head_index < steps.len())
            && key_commitment_s.is_some() == !epochs.is_empty();
        if !consistent {
            return Err(WireError::ObjectFieldsMismatch);
        }
        Ok(Self {
            vault_id,
            steps,
            members,
            epochs,
            key_commitment_s,
        })
    }
}

const STATE_LABEL: &str = "dohflow/sync-verified-state/v1";

fn reason_byte(reason: Reason) -> u8 {
    match reason {
        Reason::Genesis => 1,
        Reason::Cutover => 2,
        Reason::Grant => 3,
        Reason::Rotation => 4,
    }
}

fn reason_from_byte(byte: u8) -> Result<Reason, WireError> {
    match byte {
        1 => Ok(Reason::Genesis),
        2 => Ok(Reason::Cutover),
        3 => Ok(Reason::Grant),
        4 => Ok(Reason::Rotation),
        _ => Err(WireError::UnknownObjectType),
    }
}

fn malformed(at: usize) -> impl Fn(WireError) -> ChainError {
    move |error| match error {
        WireError::UnsupportedProtocol => ChainError::UnsupportedProtocol { at },
        error => ChainError::Malformed { at, error },
    }
}

fn check_suite(suite_id: u16, kem_id: u16, at: usize) -> Result<(), ChainError> {
    if suite_id == SUITE_ID && kem_id == KEM_ID {
        Ok(())
    } else {
        Err(ChainError::UnsupportedSuite { at })
    }
}

/// Verify a whole chain from nothing: `extend` applied to the empty state.
///
/// # Errors
/// A [`ChainError`]; [`ChainError::kind`] gives the C.11 name.
pub fn verify_chain(
    vault_id: &[u8; 16],
    memberships: &[&[u8]],
    epochs: &[&[u8]],
) -> Result<VerifiedState, ChainError> {
    extend(&VerifiedState::empty(*vault_id), memberships, epochs)
}

/// Verify the records that follow `state` and return the new state, or an error and no change.
///
/// `memberships` and `epochs` are the **complete suffix** from the last accepted record, in chain
/// order; every record in them is verified in full (a superset of C.6's backward walk from the
/// anchor, which exists to skip work a forward pass has already paid for). A record that repeats
/// one already accepted at its position is idempotent; a different record at an accepted position
/// is [`ChainError::Fork`] or [`ChainError::EpochEquivocation`] **only if it would itself have been
/// valid at that position**; unauthenticated bytes are an ordinary error, never the halt class.
/// If the newest epoch record does not name the newest membership record the result is
/// [`ChainError::EpochRecordMissing`], which is retryable ([`ChainError::is_retryable`]): `extend`
/// cannot tell a withheld coupled epoch record from a stale anchor, and a client engine that was
/// shown an explicit anchor naming an older head reports it as `StaleMembershipHead`.
/// Afterwards the newest epoch record
/// must name the newest membership record: every membership change after the genesis is installed
/// with exactly one epoch record that names it, so anything else is
/// [`ChainError::EpochRecordMissing`].
///
/// # Errors
/// A [`ChainError`]; [`ChainError::kind`] gives the C.11 name.
pub fn extend(
    state: &VerifiedState,
    memberships: &[&[u8]],
    epochs: &[&[u8]],
) -> Result<VerifiedState, ChainError> {
    let mut next = state.clone();
    for (at, bytes) in memberships.iter().enumerate() {
        apply_membership(&mut next, at, bytes)?;
    }
    for (at, bytes) in epochs.iter().enumerate() {
        apply_epoch(&mut next, at, bytes)?;
    }
    if next.steps.is_empty() {
        return Ok(next);
    }
    match next.epochs.last() {
        None => Err(ChainError::EpochRecordMissing { version: 1 }),
        Some(last) if last.head_index + 1 < next.steps.len() => {
            Err(ChainError::EpochRecordMissing {
                version: (last.head_index + 2) as u64,
            })
        }
        Some(_) => Ok(next),
    }
}

fn apply_membership(state: &mut VerifiedState, at: usize, bytes: &[u8]) -> Result<(), ChainError> {
    if state.steps.len() >= MAX_CHAIN_LEN {
        return Err(ChainError::TooLong);
    }
    let record = SignedRecord::parse(bytes, MAX_MEMBERSHIP_BODY).map_err(malformed(at))?;
    let body = MembershipBody::decode(&record.body).map_err(malformed(at))?;
    check_suite(body.suite_id, body.kem_id, at)?;
    if body.vault_id != state.vault_id {
        return Err(ChainError::VaultMismatch { at });
    }
    let id = record.id();

    // A record at a position already accepted: the same record is idempotent, another is a fork.
    if let Ok(position) = usize::try_from(body.version) {
        if position >= 1 && position <= state.steps.len() {
            return if state.steps[position - 1].id == id {
                Ok(())
            } else {
                Err(conflicting_membership(
                    state, at, position, &body, &record, id,
                ))
            };
        }
    }

    let recomputed = body.fingerprint();
    let Some(previous) = state.steps.last().copied() else {
        // Rule 1: the genesis record, signed by the device it adds.
        if body.version != 1
            || body.prev_id.is_some()
            || body.action != Action::Add
            || body.signer_fingerprint != recomputed
        {
            return Err(ChainError::BadGenesis { at });
        }
        if !ed25519_public_key_acceptable(&body.ed25519_pk)
            || !x25519_public_key_acceptable(&body.kem_public_key)
        {
            return Err(ChainError::KeyUnacceptable { at });
        }
        if !ed25519_verify_strict(&body.ed25519_pk, &record.body, &record.signature) {
            return Err(ChainError::BadSignature { at });
        }
        state.members.push(Member {
            device_id: body.device_id,
            fingerprint: recomputed,
            ed25519_pk: body.ed25519_pk,
            kem_public_key: body.kem_public_key,
            added_at_version: 1,
            revoked_at_version: None,
        });
        state.steps.push(StepRef {
            id,
            action: Action::Add,
        });
        return Ok(());
    };

    // Rule 2: version and linkage.
    let previous_version = state.steps.len() as u64;
    if body.version != previous_version + 1 {
        return Err(ChainError::WrongVersion { at });
    }
    if body.prev_id != Some(previous.id) {
        return Err(ChainError::WrongPrevId { at });
    }
    // Rule 3: the signer is an authorized, unrevoked member in the state after prev_id, and the
    // key that verifies the signature is the one the CHAIN holds for it, never the record's own.
    let Some(signer) = state.authorized_member(&body.signer_fingerprint, previous_version) else {
        return Err(
            if state
                .members
                .iter()
                .any(|m| m.fingerprint == body.signer_fingerprint)
            {
                ChainError::SignerRevoked { at }
            } else {
                ChainError::SignerUnknown { at }
            },
        );
    };
    if !ed25519_verify_strict(&signer.ed25519_pk, &record.body, &record.signature) {
        return Err(ChainError::BadSignature { at });
    }
    match body.action {
        Action::Add => {
            if !ed25519_public_key_acceptable(&body.ed25519_pk)
                || !x25519_public_key_acceptable(&body.kem_public_key)
            {
                return Err(ChainError::KeyUnacceptable { at });
            }
            // Rule 4, extended: a fingerprint, a device id or a public key is never reused,
            // including by a revoked device.
            if state.members.iter().any(|m| m.fingerprint == recomputed) {
                return Err(ChainError::FingerprintReused { at });
            }
            if state.members.iter().any(|m| m.device_id == body.device_id) {
                return Err(ChainError::DeviceIdReused { at });
            }
            if state
                .members
                .iter()
                .any(|m| m.ed25519_pk == body.ed25519_pk || m.kem_public_key == body.kem_public_key)
            {
                return Err(ChainError::KeyReused { at });
            }
            if state.authorized().count() >= MAX_MEMBERS {
                return Err(ChainError::TooManyMembers { at });
            }
            state.members.push(Member {
                device_id: body.device_id,
                fingerprint: recomputed,
                ed25519_pk: body.ed25519_pk,
                kem_public_key: body.kem_public_key,
                added_at_version: body.version,
                revoked_at_version: None,
            });
        }
        Action::Revoke => {
            // Rule 6: it repeats the add exactly and revokes a currently authorized fingerprint.
            let authorized_now = state.authorized().count();
            let Some(target) = state
                .members
                .iter_mut()
                .find(|m| m.fingerprint == recomputed)
            else {
                return Err(ChainError::RevokeMismatch { at });
            };
            if target.device_id != body.device_id
                || target.ed25519_pk != body.ed25519_pk
                || target.kem_public_key != body.kem_public_key
            {
                return Err(ChainError::RevokeMismatch { at });
            }
            if target.revoked_at_version.is_some() {
                return Err(ChainError::RevokeOfUnauthorized { at });
            }
            if authorized_now <= 1 {
                return Err(ChainError::WouldEmptyChain { at });
            }
            target.revoked_at_version = Some(body.version);
        }
    }
    state.steps.push(StepRef {
        id,
        action: body.action,
    });
    Ok(())
}

/// A membership record that conflicts with the one accepted at its version: a [`ChainError::Fork`]
/// only if it would itself have been valid there, otherwise the ordinary error it earns, so that
/// unauthenticated bytes can never put a device into the halt class.
fn conflicting_membership(
    state: &VerifiedState,
    at: usize,
    position: usize,
    body: &MembershipBody,
    record: &SignedRecord,
    id: [u8; 32],
) -> ChainError {
    let valid_there = if position == 1 {
        if body.version != 1
            || body.prev_id.is_some()
            || body.action != Action::Add
            || body.signer_fingerprint != body.fingerprint()
        {
            return ChainError::BadGenesis { at };
        }
        if !ed25519_public_key_acceptable(&body.ed25519_pk)
            || !x25519_public_key_acceptable(&body.kem_public_key)
        {
            return ChainError::KeyUnacceptable { at };
        }
        // A competing genesis is evidence of a fork only if the ACCEPTED founding device signed
        // it. A self-signed genesis needs no secret (the vault id is routing data), so one from
        // anyone else has no authority over the chain and is just a bad genesis.
        let founder = &state.members[0];
        if body.signer_fingerprint != founder.fingerprint {
            return ChainError::BadGenesis { at };
        }
        ed25519_verify_strict(&founder.ed25519_pk, &record.body, &record.signature)
    } else {
        if body.prev_id != Some(state.steps[position - 2].id) {
            return ChainError::WrongPrevId { at };
        }
        let Some(signer) = state.authorized_member(&body.signer_fingerprint, (position - 1) as u64)
        else {
            return if state
                .members
                .iter()
                .any(|m| m.fingerprint == body.signer_fingerprint)
            {
                ChainError::SignerRevoked { at }
            } else {
                ChainError::SignerUnknown { at }
            };
        };
        ed25519_verify_strict(&signer.ed25519_pk, &record.body, &record.signature)
    };
    if valid_there {
        ChainError::Fork {
            version: body.version,
            accepted: state.steps[position - 1].id,
            conflicting: id,
        }
    } else {
        ChainError::BadSignature { at }
    }
}

/// An epoch record that conflicts with the one accepted after the same `prev_epoch_id`: an
/// [`ChainError::EpochEquivocation`] only if its sealer is authorized at its named head and its
/// signature is valid under the chain's key, otherwise the ordinary error it earns.
fn conflicting_epoch(
    state: &VerifiedState,
    at: usize,
    body: &EpochBody,
    record: &SignedRecord,
    accepted: [u8; 32],
    conflicting: [u8; 32],
) -> ChainError {
    let Some(head_index) = state
        .steps
        .iter()
        .position(|s| s.id == body.membership_head)
    else {
        return ChainError::UnknownHead { at };
    };
    let Some(sealer) = state.authorized_member(&body.sealer_fingerprint, (head_index + 1) as u64)
    else {
        return ChainError::SealerNotAuthorized { at };
    };
    if ed25519_verify_strict(&sealer.ed25519_pk, &record.body, &record.signature) {
        ChainError::EpochEquivocation {
            accepted,
            conflicting,
        }
    } else {
        ChainError::EpochBadSignature { at }
    }
}

fn apply_epoch(state: &mut VerifiedState, at: usize, bytes: &[u8]) -> Result<(), ChainError> {
    if state.epochs.len() >= MAX_CHAIN_LEN {
        return Err(ChainError::TooLong);
    }
    let record = SignedRecord::parse(bytes, MAX_EPOCH_BODY).map_err(malformed(at))?;
    let body = EpochBody::decode(&record.body).map_err(malformed(at))?;
    check_suite(body.suite_id, body.kem_id, at)?;
    if body.vault_id != state.vault_id {
        return Err(ChainError::VaultMismatch { at });
    }
    let id = record.id();

    // Equivocation: another record already follows this prev_epoch_id. The same record is
    // idempotent.
    if let Some(existing) = state
        .epochs
        .iter()
        .find(|e| e.prev_epoch_id == body.prev_epoch_id)
    {
        let accepted = existing.id;
        return if accepted == id {
            Ok(())
        } else {
            Err(conflicting_epoch(state, at, &body, &record, accepted, id))
        };
    }

    let head_index = state
        .steps
        .iter()
        .position(|s| s.id == body.membership_head)
        .ok_or(ChainError::UnknownHead { at })?;
    let head_version = (head_index + 1) as u64;

    // The sealer is authorized at the record's OWN named head, and its signature verifies under
    // the key the chain holds for it.
    let sealer = state
        .authorized_member(&body.sealer_fingerprint, head_version)
        .ok_or(ChainError::SealerNotAuthorized { at })?;
    if !ed25519_verify_strict(&sealer.ed25519_pk, &record.body, &record.signature) {
        return Err(ChainError::EpochBadSignature { at });
    }

    // Sequence and the reason table.
    match (state.epochs.last(), body.reason) {
        (None, Reason::Genesis) => {
            if body.key_epoch != 1 || body.prev_epoch_id.is_some() || head_index != 0 {
                return Err(ChainError::BadEpochSequence { at });
            }
            state.key_commitment_s = Some(body.key_commitment_s);
        }
        (None, _) | (Some(_), Reason::Genesis) => {
            return Err(ChainError::BadEpochSequence { at });
        }
        (Some(previous), reason) => {
            if body.prev_epoch_id != Some(previous.id)
                || state.key_commitment_s != Some(body.key_commitment_s)
            {
                return Err(ChainError::BadEpochSequence { at });
            }
            let fresh_key = !state
                .epochs
                .iter()
                .any(|e| e.key_commitment_ke == body.key_commitment_ke);
            match reason {
                Reason::Cutover | Reason::Grant => {
                    // The head advances by exactly one membership record: every record after the
                    // genesis has its own epoch record.
                    if head_index > previous.head_index + 1 {
                        return Err(ChainError::EpochRecordMissing {
                            version: (previous.head_index + 2) as u64,
                        });
                    }
                    if head_index != previous.head_index + 1 {
                        return Err(ChainError::BadEpochSequence { at });
                    }
                    let wanted = if reason == Reason::Cutover {
                        Action::Revoke
                    } else {
                        Action::Add
                    };
                    if state.steps[head_index].action != wanted {
                        return Err(if reason == Reason::Cutover {
                            ChainError::CutoverHeadNotRevoke { at }
                        } else {
                            ChainError::GrantHeadNotAdd { at }
                        });
                    }
                    if reason == Reason::Cutover {
                        // A cutover must publish a key never used before in this chain.
                        if body.key_epoch != previous.key_epoch + 1 || !fresh_key {
                            return Err(ChainError::BadEpochSequence { at });
                        }
                    } else if body.key_epoch != previous.key_epoch
                        || body.key_commitment_ke != previous.key_commitment_ke
                    {
                        return Err(ChainError::BadEpochSequence { at });
                    }
                }
                Reason::Rotation => {
                    if head_index != previous.head_index
                        || body.key_epoch != previous.key_epoch + 1
                        || !fresh_key
                    {
                        return Err(ChainError::BadEpochSequence { at });
                    }
                }
                Reason::Genesis => unreachable!("handled above"),
            }
        }
    }

    // Recipients: exactly the set the reason names, with the values recomputed from the chain.
    let expected: Vec<&Member> = match body.reason {
        Reason::Genesis | Reason::Cutover | Reason::Rotation => state
            .members
            .iter()
            .filter(|m| m.authorized_at(head_version))
            .collect(),
        Reason::Grant => state
            .members
            .iter()
            .filter(|m| m.added_at_version == head_version)
            .collect(),
    };
    let mut listed = BTreeSet::new();
    for recipient in &body.recipients {
        let known = expected
            .iter()
            .any(|m| m.device_id == recipient.device_id && m.fingerprint == recipient.fingerprint);
        if recipient.suite_id != SUITE_ID
            || recipient.kem_id != KEM_ID
            || !known
            || !listed.insert(*recipient.fingerprint.as_bytes())
        {
            return Err(ChainError::RecipientsMismatch { at });
        }
    }
    if listed.len() != expected.len() {
        return Err(ChainError::RecipientsMismatch { at });
    }
    state.epochs.push(EpochRef {
        id,
        prev_epoch_id: body.prev_epoch_id,
        key_epoch: body.key_epoch,
        reason: body.reason,
        head_index,
        key_commitment_ke: body.key_commitment_ke,
    });
    Ok(())
}
