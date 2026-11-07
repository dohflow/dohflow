//! The class-1b artifact formats (`docs/architecture/vault-format.md` C.9; personal-cfo-0g528.2,
//! part S2b): the transport plaintext, the `DFSA` container, the receipt, the AAD, the logical
//! binding an artifact carries, and the closure that a snapshot commits to.
//!
//! This is the pure, key-free half. The addresses (C.3), the object key and the AEAD live in
//! `vault-crypto`, which derives them from `K_e`.
//!
//! * [`ArtifactId`] is the opaque service-facing address, `sync_artifact_id_e`.
//! * [`TransportPlaintext`] is `SyncArtifactTransportV1`: what is sealed. It borrows its parts so a
//!   decoded plaintext can stay in the caller's protected buffer.
//! * [`LogicalBinding`] is the logical id an artifact's encrypted `logical_metadata` names (an
//!   attachment, or one part of a source batch). A manifest is a sorted list of ids with nothing that
//!   says which id belongs to which row, so a receiver learns that only by opening the artifact (C.9).
//! * [`encode_container`] and [`parse_container`] are the `DFSA` container.
//! * [`ArtifactReceipt`] is `ArtifactReceiptV1`.
//! * [`ClosureManifest`] is a snapshot's full closure (at most 2^20 ids), committed to with the
//!   same label and bytes as an envelope's delta.

use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::canonical_json::{parse_canonical_node, Node};
use crate::cbe::{Reader, WireError, Writer};
use crate::envelope::manifest_bytes_bounded;
use crate::header::{ManifestCommitment, PROTOCOL_VERSION};

/// The label of the transport plaintext.
pub const TRANSPORT_LABEL: &str = "dohflow/sync-artifact-transport/v1";
/// The label of the AAD of an artifact's AEAD.
pub const AAD_LABEL: &str = "dohflow/sync-artifact-aad/v1";
/// The label of the receipt.
pub const RECEIPT_LABEL: &str = "dohflow/sync-receipt/v1";
/// The container magic.
pub const CONTAINER_MAGIC: &[u8; 4] = b"DFSA";
/// The one container version this build speaks.
pub const CONTAINER_VERSION: u16 = 1;
/// The AES-256-GCM tag length: the least a sealed object's ciphertext can be.
pub const MIN_CIPHERTEXT_LEN: usize = 16;
/// The most `logical_metadata` can hold (C.9).
pub const MAX_LOGICAL_METADATA: usize = 65_536;
/// The largest container the service stores (`max_artifact_bytes`, sync-service-protocol 6).
pub const MAX_ARTIFACT_CONTAINER: usize = 67_108_864;
/// The most ids in a snapshot's full closure (C.3).
pub const MAX_SNAPSHOT_CLOSURE: usize = 1 << 20;
/// The bytes of one id in a closure sidecar body (sync-service-protocol 4.14): the id's 32 raw bytes.
pub const CLOSURE_SIDECAR_ID_LEN: usize = 32;
/// The most bytes a closure sidecar body holds (sync-service-protocol 7.1): 2^20 ids of 32 bytes.
pub const MAX_CLOSURE_SIDECAR_LEN: usize = MAX_SNAPSHOT_CLOSURE * CLOSURE_SIDECAR_ID_LEN;
/// The most ids in an envelope's delta manifest (C.3).
pub const MAX_DELTA: usize = crate::envelope::MAX_MANIFEST_ENTRIES;
/// The most bytes in one source batch part's blob (C.9).
pub const MAX_PART_BLOB: usize = 1 << 22;
/// The most bytes in one source record's canonical JSON (C.9), refused at staging.
pub const MAX_SOURCE_RECORD: usize = 1 << 20;
/// The most bytes in a source batch's own columns plus all its parser runs (C.9).
pub const MAX_BATCH_HEADER: usize = 1 << 20;

/// Why an artifact object was refused. No variant carries a value from the object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ArtifactError {
    /// A CBE1 or container violation.
    #[error("the artifact is malformed: {0}")]
    Wire(WireError),
    /// `artifact_kind` 3 or 4: reserved, never produced (C.1.3).
    #[error("the artifact kind is reserved")]
    ReservedKind,
    /// `artifact_kind` outside the registry.
    #[error("unknown artifact kind")]
    UnknownKind,
    /// `logical_metadata` is not a canonical JSON object.
    #[error("the logical metadata is not a canonical JSON object")]
    MetadataNotCanonical,
    /// `logical_metadata` does not name the logical id its kind requires.
    #[error("the logical metadata does not name a valid logical binding")]
    BadLogicalMetadata,
    /// The descriptor's `plaintext_length` is not the blob's length.
    #[error("the descriptor's length is not the blob's")]
    DescriptorLength,
    /// The descriptor's `plaintext_digest` is not the blob's SHA-256.
    #[error("the descriptor's digest is not the blob's")]
    DescriptorDigest,
    /// The container is larger than the service stores.
    #[error("the artifact container is too large")]
    ContainerTooLarge,
    /// A closure holds more ids than its object type allows, or is not strictly ascending.
    #[error("the closure is too large or not strictly ascending")]
    BadClosure,
}

impl From<WireError> for ArtifactError {
    fn from(error: WireError) -> Self {
        Self::Wire(error)
    }
}

/// The opaque, epoch-scoped service address of an artifact, `sync_artifact_id_e` (C.3).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArtifactId([u8; 32]);

impl ArtifactId {
    /// Wrap a value that was **derived** as an artifact id (C.3). There is deliberately no
    /// `From<[u8; 32]>`: calling this is the explicit claim.
    #[must_use]
    pub const fn from_derived(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// The 32 raw bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl core::fmt::Debug for ArtifactId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // Public by construction (a keyed hash), but a short form keeps logs readable.
        write!(f, "ArtifactId({:02x}{:02x}..)", self.0[0], self.0[1])
    }
}

/// `artifact_kind` (C.1.3). Kinds 3 and 4 are reserved and refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactKind {
    /// An attachment's bytes.
    Attachment,
    /// One part of a source batch (C.9).
    SourceBatchPart,
}

impl ArtifactKind {
    /// The registry value.
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Attachment => 1,
            Self::SourceBatchPart => 2,
        }
    }

    /// The kind of a registry value.
    ///
    /// # Errors
    /// [`ArtifactError::ReservedKind`] for 3 and 4, [`ArtifactError::UnknownKind`] otherwise.
    pub const fn from_code(code: u8) -> Result<Self, ArtifactError> {
        match code {
            1 => Ok(Self::Attachment),
            2 => Ok(Self::SourceBatchPart),
            3 | 4 => Err(ArtifactError::ReservedKind),
            _ => Err(ArtifactError::UnknownKind),
        }
    }
}

/// The logical id an artifact binds (C.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LogicalBinding {
    /// An attachment.
    Attachment {
        /// The attachment's row id.
        attachment_id: [u8; 16],
    },
    /// One part of a source batch.
    BatchPart {
        /// The batch's id.
        batch_id: [u8; 16],
        /// This part's index, from 0.
        part_index: u32,
        /// How many parts the batch has.
        part_count: u32,
    },
}

fn parse_uuid(text: &str) -> Option<[u8; 16]> {
    let bytes = text.as_bytes();
    if bytes.len() != 36 {
        return None;
    }
    let mut out = [0_u8; 16];
    let mut at = 0;
    let mut i = 0;
    while i < 36 {
        if matches!(i, 8 | 13 | 18 | 23) {
            if bytes[i] != b'-' {
                return None;
            }
            i += 1;
            continue;
        }
        let hi = (bytes[i] as char).to_digit(16)?;
        let lo = (bytes[i + 1] as char).to_digit(16)?;
        if bytes[i].is_ascii_uppercase() || bytes[i + 1].is_ascii_uppercase() {
            return None;
        }
        out[at] = u8::try_from(hi * 16 + lo).ok()?;
        at += 1;
        i += 2;
    }
    Some(out)
}

/// A decimal string with no sign, no leading zero (except `"0"`), as C.1.2 writes an integer.
fn parse_decimal_u32(text: &str) -> Option<u32> {
    if text.is_empty() || (text.len() > 1 && text.starts_with('0')) {
        return None;
    }
    if !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

impl LogicalBinding {
    /// The binding that `logical_metadata` names for an artifact of `kind`.
    ///
    /// # Errors
    /// [`ArtifactError::MetadataNotCanonical`] if it is not a canonical JSON object,
    /// [`ArtifactError::BadLogicalMetadata`] if the member the kind requires is missing or malformed
    /// (an attachment needs `attachment_id`; a batch part needs `batch_id`, `part_count` of 1 to
    /// 2^20 and a `part_index` below it).
    pub fn of(kind: ArtifactKind, logical_metadata: &[u8]) -> Result<Self, ArtifactError> {
        let Node::Object(members) = parse_canonical_node(logical_metadata)
            .map_err(|_| ArtifactError::MetadataNotCanonical)?
        else {
            return Err(ArtifactError::MetadataNotCanonical);
        };
        let text = |name: &str| match members.get(name) {
            Some(Node::String(value)) => Some(value.as_str()),
            _ => None,
        };
        let bad = ArtifactError::BadLogicalMetadata;
        match kind {
            ArtifactKind::Attachment => Ok(Self::Attachment {
                attachment_id: text("attachment_id").and_then(parse_uuid).ok_or(bad)?,
            }),
            ArtifactKind::SourceBatchPart => {
                // Exactly the three members C.9 names: an unknown member is refused, so two
                // replicas cannot disagree about whether it matters.
                if members.len() != 3 {
                    return Err(bad);
                }
                let batch_id = text("batch_id").and_then(parse_uuid).ok_or(bad)?;
                let part_count = text("part_count").and_then(parse_decimal_u32).ok_or(bad)?;
                let part_index = text("part_index").and_then(parse_decimal_u32).ok_or(bad)?;
                if part_count == 0
                    || usize::try_from(part_count).map_or(true, |n| n > MAX_SNAPSHOT_CLOSURE)
                    || part_index >= part_count
                {
                    return Err(bad);
                }
                Ok(Self::BatchPart {
                    batch_id,
                    part_index,
                    part_count,
                })
            }
        }
    }
}

/// `SyncArtifactTransportV1` (C.9): `label ‖ digest sync_artifact_id ‖ u8 artifact_kind ‖ u64
/// plaintext_length ‖ digest plaintext_digest ‖ bytes(65536) logical_metadata ‖ blob`.
///
/// It borrows its parts. `plaintext_length` and `plaintext_digest` are derived from the blob when
/// it is encoded and checked against it when it is decoded, so an in-memory value cannot disagree
/// with its own descriptor.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct TransportPlaintext<'a> {
    /// The artifact's address in the epoch it is sealed for.
    pub artifact_id: ArtifactId,
    /// The kind.
    pub kind: ArtifactKind,
    /// Canonical JSON: only logical fields (C.9).
    pub logical_metadata: &'a [u8],
    /// The artifact's bytes.
    pub blob: &'a [u8],
}

impl core::fmt::Debug for TransportPlaintext<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // Sizes, never content: the blob is a document or financial data.
        f.debug_struct("TransportPlaintext")
            .field("artifact_id", &self.artifact_id)
            .field("kind", &self.kind)
            .field("metadata_len", &self.logical_metadata.len())
            .field("blob_len", &self.blob.len())
            .finish()
    }
}

impl<'a> TransportPlaintext<'a> {
    /// The canonical bytes.
    ///
    /// # Errors
    /// [`ArtifactError::MetadataNotCanonical`] if `logical_metadata` is not a canonical JSON
    /// object, [`ArtifactError::Wire`] if it is over 65,536 bytes.
    pub fn encode(&self) -> Result<Vec<u8>, ArtifactError> {
        if self.logical_metadata.len() > MAX_LOGICAL_METADATA {
            return Err(WireError::TooLong.into());
        }
        match parse_canonical_node(self.logical_metadata) {
            Ok(Node::Object(_)) => {}
            _ => return Err(ArtifactError::MetadataNotCanonical),
        }
        let mut w = Writer::with_label(TRANSPORT_LABEL);
        w.fixed(self.artifact_id.as_bytes()).u8(self.kind.code());
        w.u64(self.blob.len() as u64);
        w.fixed::<32>(&Sha256::digest(self.blob).into());
        w.bytes(MAX_LOGICAL_METADATA, self.logical_metadata)?;
        w.blob(usize::MAX, self.blob)?;
        Ok(w.finish())
    }

    /// The SHA-256 of the complete encoded plaintext: the value a client stores at first
    /// publication and checks before every seal (C.9 "Fixed plaintext").
    ///
    /// # Errors
    /// As [`Self::encode`].
    pub fn digest(&self) -> Result<[u8; 32], ArtifactError> {
        Ok(Sha256::digest(self.encode()?).into())
    }

    /// Decode and check `bytes`: the label, a kind from the registry (3 and 4 are refused),
    /// `plaintext_length` and `plaintext_digest` against the blob, `logical_metadata` as a canonical
    /// JSON object, and no trailing byte.
    ///
    /// # Errors
    /// An [`ArtifactError`].
    pub fn decode(bytes: &'a [u8]) -> Result<Self, ArtifactError> {
        let mut r = Reader::new(bytes);
        r.label(TRANSPORT_LABEL)?;
        let artifact_id = ArtifactId::from_derived(r.fixed::<32>()?);
        let kind = ArtifactKind::from_code(r.u8()?)?;
        let length = r.u64()?;
        let digest = r.fixed::<32>()?;
        let logical_metadata = r.bytes(MAX_LOGICAL_METADATA)?;
        let blob = r.blob(usize::MAX)?;
        r.finish()?;
        if u64::try_from(blob.len()).ok() != Some(length) {
            return Err(ArtifactError::DescriptorLength);
        }
        if <[u8; 32]>::from(Sha256::digest(blob)) != digest {
            return Err(ArtifactError::DescriptorDigest);
        }
        match parse_canonical_node(logical_metadata) {
            Ok(Node::Object(_)) => {}
            _ => return Err(ArtifactError::MetadataNotCanonical),
        }
        Ok(Self {
            artifact_id,
            kind,
            logical_metadata,
            blob,
        })
    }

    /// The logical binding this artifact names.
    ///
    /// # Errors
    /// As [`LogicalBinding::of`].
    pub fn binding(&self) -> Result<LogicalBinding, ArtifactError> {
        LogicalBinding::of(self.kind, self.logical_metadata)
    }
}

/// The AAD of an artifact's AEAD: `label ‖ u16 protocol_version ‖ uuid vault_id ‖ u64 key_epoch ‖
/// digest sync_artifact_id` (C.9). It binds the object to its address, so bytes presented under
/// another address do not authenticate.
#[must_use]
pub fn artifact_aad(vault_id: &[u8; 16], key_epoch: u64, id: &ArtifactId) -> Vec<u8> {
    let mut w = Writer::with_label(AAD_LABEL);
    w.u16(PROTOCOL_VERSION)
        .fixed(vault_id)
        .u64(key_epoch)
        .fixed(id.as_bytes());
    w.finish()
}

/// `magic "DFSA" ‖ u16 container_version = 1 ‖ u64 ciphertext_len ‖ ciphertext` (C.9).
#[must_use]
pub fn encode_container(ciphertext: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + 2 + 8 + ciphertext.len());
    out.extend_from_slice(CONTAINER_MAGIC);
    out.extend_from_slice(&CONTAINER_VERSION.to_be_bytes());
    out.extend_from_slice(&(ciphertext.len() as u64).to_be_bytes());
    out.extend_from_slice(ciphertext);
    out
}

/// The ciphertext of a container.
///
/// # Errors
/// [`ArtifactError::ContainerTooLarge`] above `max_len` (checked before anything is parsed),
/// [`WireError::BadMagic`], [`WireError::UnsupportedContainer`], [`WireError::Truncated`],
/// [`WireError::LengthExceedsInput`] or [`WireError::TrailingBytes`] for any length that is not
/// exactly what remains, and [`WireError::CiphertextTooShort`] under a GCM tag.
pub fn parse_container(bytes: &[u8], max_len: usize) -> Result<&[u8], ArtifactError> {
    if bytes.len() > max_len {
        return Err(ArtifactError::ContainerTooLarge);
    }
    if bytes.len() < 4 {
        return Err(WireError::Truncated.into());
    }
    if &bytes[..4] != CONTAINER_MAGIC {
        return Err(WireError::BadMagic.into());
    }
    let mut r = Reader::new(&bytes[4..]);
    if r.u16()? != CONTAINER_VERSION {
        return Err(WireError::UnsupportedContainer.into());
    }
    let length = usize::try_from(r.u64()?).map_err(|_| WireError::LengthExceedsInput)?;
    if length > r.remaining() {
        return Err(WireError::LengthExceedsInput.into());
    }
    if length < r.remaining() {
        return Err(WireError::TrailingBytes.into());
    }
    if length < MIN_CIPHERTEXT_LEN {
        return Err(WireError::CiphertextTooShort.into());
    }
    let start = bytes.len() - length;
    Ok(&bytes[start..])
}

/// `ArtifactReceiptV1` (C.9): `label ‖ uuid vault_id ‖ u64 key_epoch ‖ digest sync_artifact_id ‖
/// u64 ciphertext_length ‖ digest ciphertext_sha256`, over the AEAD output the container carries.
/// A durability acknowledgement bound to the address and the ciphertext, with no service signature
/// in v1: its integrity comes from the client re-verifying the outer AEAD whenever it fetches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArtifactReceipt {
    /// The vault.
    pub vault_id: [u8; 16],
    /// The key epoch of the address.
    pub key_epoch: u64,
    /// The artifact's address.
    pub artifact_id: ArtifactId,
    /// The ciphertext's length.
    pub ciphertext_length: u64,
    /// The ciphertext's SHA-256.
    pub ciphertext_sha256: [u8; 32],
}

impl ArtifactReceipt {
    /// The receipt a client computes for the ciphertext it sealed.
    #[must_use]
    pub fn for_ciphertext(
        vault_id: [u8; 16],
        key_epoch: u64,
        artifact_id: ArtifactId,
        ciphertext: &[u8],
    ) -> Self {
        Self {
            vault_id,
            key_epoch,
            artifact_id,
            ciphertext_length: ciphertext.len() as u64,
            ciphertext_sha256: Sha256::digest(ciphertext).into(),
        }
    }

    /// The canonical bytes.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::with_label(RECEIPT_LABEL);
        w.fixed(&self.vault_id)
            .u64(self.key_epoch)
            .fixed(self.artifact_id.as_bytes())
            .u64(self.ciphertext_length)
            .fixed(&self.ciphertext_sha256);
        w.finish()
    }

    /// Decode canonical bytes.
    ///
    /// # Errors
    /// An [`ArtifactError::Wire`].
    pub fn decode(bytes: &[u8]) -> Result<Self, ArtifactError> {
        let mut r = Reader::new(bytes);
        r.label(RECEIPT_LABEL)?;
        let receipt = Self {
            vault_id: r.fixed()?,
            key_epoch: r.u64()?,
            artifact_id: ArtifactId::from_derived(r.fixed()?),
            ciphertext_length: r.u64()?,
            ciphertext_sha256: r.fixed()?,
        };
        r.finish()?;
        Ok(receipt)
    }
}

/// Why a closure sidecar body (sync-service-protocol 4.14) is not a closure. Each maps to a protocol
/// code: [`Self::TooManyIds`] is `SnapshotTooLarge`, the others `MalformedObject`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ClosureSidecarError {
    /// The body is longer than 2^20 ids. Decided from the length alone, before a byte is read.
    #[error("the closure sidecar holds more than 2^20 ids")]
    TooManyIds,
    /// The body's length is not a whole number of 32-byte ids.
    #[error("the closure sidecar is not a whole number of ids")]
    NotWholeIds,
    /// An id is not greater than the one before it (a duplicate, or out of order).
    #[error("the closure sidecar is not strictly ascending")]
    NotAscending,
}

/// A snapshot's full artifact closure: strictly ascending, at most [`MAX_SNAPSHOT_CLOSURE`] ids.
/// Its commitment (header field 17 of a snapshot) is the same function of the list as an envelope
/// delta's, with the label and bytes of C.3; only the bound differs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosureManifest(Vec<[u8; 32]>);

impl ClosureManifest {
    /// Order `ids` and refuse a duplicate or more than [`MAX_SNAPSHOT_CLOSURE`].
    ///
    /// # Errors
    /// [`ArtifactError::BadClosure`].
    pub fn new(mut ids: Vec<[u8; 32]>) -> Result<Self, ArtifactError> {
        if ids.len() > MAX_SNAPSHOT_CLOSURE {
            return Err(ArtifactError::BadClosure);
        }
        ids.sort_unstable();
        if ids.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(ArtifactError::BadClosure);
        }
        Ok(Self(ids))
    }

    /// The ids, strictly ascending.
    #[must_use]
    pub fn ids(&self) -> &[[u8; 32]] {
        &self.0
    }

    /// The closure as a sidecar body (sync-service-protocol 4.14): the ids' raw bytes, strictly
    /// ascending, with no framing, so its length is 32 times the number of ids.
    #[must_use]
    pub fn to_sidecar(&self) -> Vec<u8> {
        self.0.concat()
    }

    /// A closure from a sidecar body. Strict: a body that is out of order is refused, never put in
    /// order, because the sidecar's order is part of what the commitment binds. The service and the
    /// client both call this, so they cannot disagree on what a well-formed sidecar is.
    ///
    /// # Errors
    /// [`ClosureSidecarError`], checked in the order of its variants: the length first.
    pub fn from_sidecar(bytes: &[u8]) -> Result<Self, ClosureSidecarError> {
        if bytes.len() > MAX_CLOSURE_SIDECAR_LEN {
            return Err(ClosureSidecarError::TooManyIds);
        }
        if !bytes.len().is_multiple_of(CLOSURE_SIDECAR_ID_LEN) {
            return Err(ClosureSidecarError::NotWholeIds);
        }
        let (whole, _) = bytes.as_chunks::<CLOSURE_SIDECAR_ID_LEN>();
        let ids: Vec<[u8; 32]> = whole.to_vec();
        if ids.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(ClosureSidecarError::NotAscending);
        }
        Ok(Self(ids))
    }

    /// `SHA-256(label "dohflow/sync-artifact-manifest/v1" ‖ list(digest, 2^20))` (C.3).
    #[must_use]
    pub fn commitment(&self) -> ManifestCommitment {
        ManifestCommitment::from_derived(
            Sha256::digest(manifest_bytes_bounded(&self.0, MAX_SNAPSHOT_CLOSURE)).into(),
        )
    }
}
