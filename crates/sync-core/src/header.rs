//! The clear header and the container of `docs/architecture/vault-format.md` C.4
//! (personal-cfo-t2s4b part 6a).
//!
//! The header is the AAD of an envelope's or snapshot's AEAD. [`ClearHeader`] has one encoding
//! per value ([`ClearHeader::encode`]) and one parser ([`ClearHeader::decode`]) that accepts
//! exactly those bytes. The object type is the [`ObjectKind`] variant, so the dual-identity rule
//! of C.4 (an envelope carries its idempotency tag and metadata commitment, a snapshot carries
//! neither and always carries its manifest commitment) holds by construction and a parser cannot
//! produce a header that breaks it.
//!
//! The service-assigned `seq` is **not** a header field: it does not exist until the service has
//! accepted the sealed object, so it can never be in the AAD.
//!
//! The values that feed the header are derived elsewhere (the tag and the nonce domain in
//! `vault-crypto`, the commitments in 6b). They enter as the distinct newtypes below, which have
//! no `From<[u8; 32]>`: a local storage id or a bare plaintext digest cannot be passed where a
//! service-bound field is expected.
//!
//! ```compile_fail
//! use sync_core::header::IdempotencyKeyTag;
//! let storage_id: [u8; 32] = [0x5a; 32]; // a device-local StorageId, or a bare plaintext digest
//! let _tag: IdempotencyKeyTag = storage_id.into(); // there is no such conversion
//! ```
//!
//! ```compile_fail
//! use sync_core::header::{ManifestCommitment, MetadataCommitment};
//! let commitment = MetadataCommitment::from_derived([1; 32]);
//! let _manifest: ManifestCommitment = commitment; // one kind of digest never stands in for another
//! ```

use sha2::{Digest, Sha256};

use crate::cbe::{Reader, WireError, Writer};

/// The one protocol version this build speaks.
pub const PROTOCOL_VERSION: u16 = 1;
/// The header's label.
pub const HEADER_LABEL: &str = "dohflow/sync-header/v1";
/// The highest `invocation_counter` a derived key may use (`2^32 - 1`).
pub const MAX_INVOCATION_COUNTER: u64 = (1 << 32) - 1;
/// The longest `engine_version`, in bytes.
pub const MAX_ENGINE_VERSION: usize = 64;

/// The `object_type` byte of a command envelope.
pub const OBJECT_ENVELOPE: u8 = 1;
/// The `object_type` byte of a snapshot.
pub const OBJECT_SNAPSHOT: u8 = 2;

/// Whether `value` is a legal `engine_version` (C.13): opaque printable ASCII (`0x21..=0x7e`),
/// 1 to [`MAX_ENGINE_VERSION`] bytes, compared byte for byte.
#[must_use]
pub fn is_valid_engine_version(value: &str) -> bool {
    (1..=MAX_ENGINE_VERSION).contains(&value.len())
        && value.bytes().all(|b| (0x21..=0x7e).contains(&b))
}

macro_rules! digest_newtype {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name([u8; 32]);

        impl $name {
            /// Wrap a value that was **derived** as this kind of field. There is deliberately no
            /// `From<[u8; 32]>`: calling this is an explicit claim that the bytes are what the
            /// name says, and is the one place to look for a misuse.
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

        impl core::fmt::Debug for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                // Public by construction (a hash or a keyed tag), but a short form keeps logs readable.
                write!(f, concat!(stringify!($name), "({:02x}{:02x}..)"), self.0[0], self.0[1])
            }
        }
    };
}

digest_newtype!(
    /// The tag of an envelope's idempotency key (C.5). Present in an envelope header, absent in a
    /// snapshot header. Derived in `vault-crypto`, which holds the key.
    IdempotencyKeyTag
);
digest_newtype!(
    /// The commitment to an envelope's complete immutable `CommandMeta` (C.4, derived in 6b).
    MetadataCommitment
);
digest_newtype!(
    /// The commitment to an artifact manifest (C.9): an envelope's delta manifest (present only
    /// when it is non-empty), or a snapshot's full closure (always present).
    ManifestCommitment
);
digest_newtype!(
    /// The fingerprint of a device's key (C.2).
    Fingerprint
);

/// The 4-byte nonce domain of a device's key for an object type (C.3).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct NonceDomain([u8; 4]);

impl NonceDomain {
    /// Wrap a derived nonce domain.
    #[must_use]
    pub const fn from_derived(bytes: [u8; 4]) -> Self {
        Self(bytes)
    }

    /// The 4 raw bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 4] {
        &self.0
    }

    /// The domain of a device key for an object type (C.3): the first 4 bytes of
    /// `SHA-256(label "dohflow/sync-nonce/v1" ‖ uuid vault_id ‖ u64 key_epoch ‖ fingerprint ‖
    /// u8 object_type)`. A receiver recomputes it and refuses a header that names another.
    #[must_use]
    pub fn derive(
        vault_id: &[u8; 16],
        key_epoch: u64,
        fingerprint: &Fingerprint,
        object_type: u8,
    ) -> Self {
        let mut w = Writer::with_label("dohflow/sync-nonce/v1");
        w.fixed(vault_id)
            .u64(key_epoch)
            .fixed(&fingerprint.0)
            .u8(object_type);
        let digest = Sha256::digest(w.as_bytes());
        Self([digest[0], digest[1], digest[2], digest[3]])
    }
}

impl core::fmt::Debug for NonceDomain {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "NonceDomain({:02x}{:02x}..)", self.0[0], self.0[1])
    }
}

/// Which kind of sealed object a header belongs to, with the fields that differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
    /// A command envelope (`object_type` 1). `object_id` is the immutable `command_id`.
    Envelope {
        /// The tag of the command's idempotency key.
        idempotency_key_tag: IdempotencyKeyTag,
        /// The commitment to the command's complete immutable metadata.
        immutable_metadata_commitment: MetadataCommitment,
        /// The commitment to the delta manifest, present only when that manifest is non-empty.
        artifact_manifest_commitment: Option<ManifestCommitment>,
    },
    /// A genesis or bootstrap snapshot (`object_type` 2). `object_id` is the snapshot id.
    Snapshot {
        /// The commitment to the snapshot's full closure, always present (even for an empty one).
        artifact_manifest_commitment: ManifestCommitment,
    },
}

/// The clear header of a sealed object (C.4): authenticated, never encrypted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClearHeader {
    /// The vault this object belongs to.
    pub vault_id: [u8; 16],
    /// The key epoch it was sealed under.
    pub key_epoch: u64,
    /// Envelope or snapshot, with that kind's own fields.
    pub object: ObjectKind,
    /// The immutable `command_id`, or the snapshot id.
    pub object_id: [u8; 16],
    /// The sealing device.
    pub device_id: [u8; 16],
    /// The fingerprint of the sealing device's key; belongs to `device_id`'s member (C.6).
    pub device_key_fingerprint: Fingerprint,
    /// The nonce domain for this device key and object type.
    pub nonce_domain: NonceDomain,
    /// The AEAD invocation counter, at most [`MAX_INVOCATION_COUNTER`].
    pub invocation_counter: u64,
    /// The server sequence number the client built on.
    pub base_seq: u64,
    /// The payload schema version (`operation_log.payload_schema_version`).
    pub payload_schema_version: u32,
    /// The apply-semantics generation (C.13), at most [`MAX_ENGINE_VERSION`] bytes.
    pub engine_version: String,
    /// The producer's vault schema version.
    pub schema_version: u32,
}

impl ClearHeader {
    /// The `object_type` byte: `1` envelope, `2` snapshot.
    #[must_use]
    pub const fn object_type(&self) -> u8 {
        match self.object {
            ObjectKind::Envelope { .. } => OBJECT_ENVELOPE,
            ObjectKind::Snapshot { .. } => OBJECT_SNAPSHOT,
        }
    }

    /// The header's canonical bytes: the AAD of the object's AEAD.
    ///
    /// # Errors
    /// [`WireError::CounterAboveCap`] if the counter is above `2^32 - 1`, [`WireError::TooLong`]
    /// if `engine_version` is longer than 64 bytes.
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        if self.invocation_counter > MAX_INVOCATION_COUNTER {
            return Err(WireError::CounterAboveCap);
        }
        if !is_valid_engine_version(&self.engine_version) {
            return Err(WireError::BadEngineVersion);
        }
        let mut w = Writer::with_label(HEADER_LABEL);
        w.u16(PROTOCOL_VERSION)
            .fixed(&self.vault_id)
            .u64(self.key_epoch)
            .u8(self.object_type())
            .fixed(&self.object_id);
        let (tag, commitment, manifest) = match &self.object {
            ObjectKind::Envelope {
                idempotency_key_tag,
                immutable_metadata_commitment,
                artifact_manifest_commitment,
            } => (
                Some(idempotency_key_tag.0),
                Some(immutable_metadata_commitment.0),
                artifact_manifest_commitment.map(|m| m.0),
            ),
            ObjectKind::Snapshot {
                artifact_manifest_commitment,
            } => (None, None, Some(artifact_manifest_commitment.0)),
        };
        w.opt(tag.as_ref(), |w, v| {
            w.fixed(v);
        })
        .opt(commitment.as_ref(), |w, v| {
            w.fixed(v);
        })
        .fixed(&self.device_id)
        .fixed(&self.device_key_fingerprint.0)
        .fixed(&self.nonce_domain.0)
        .u64(self.invocation_counter)
        .u64(self.base_seq)
        .u32(self.payload_schema_version)
        .text(MAX_ENGINE_VERSION, &self.engine_version)?
        .u32(self.schema_version)
        .opt(manifest.as_ref(), |w, v| {
            w.fixed(v);
        });
        Ok(w.finish())
    }

    /// Parse a header from exactly `bytes`: the grammar, the protocol version, the object-type
    /// rules for the dual-identity fields and the counter cap. Nothing is returned for a header
    /// that breaks any of them.
    ///
    /// # Errors
    /// A [`WireError`]: [`WireError::UnsupportedProtocol`] for a protocol version other than `1`
    /// (read right after the label, before anything else is trusted), otherwise a malformed-object
    /// rule.
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut r = Reader::new(bytes);
        r.label(HEADER_LABEL)?;
        if r.u16()? != PROTOCOL_VERSION {
            return Err(WireError::UnsupportedProtocol);
        }
        let vault_id = r.fixed::<16>()?;
        let key_epoch = r.u64()?;
        let object_type = r.u8()?;
        let object_id = r.fixed::<16>()?;
        let tag = r.opt(|r| r.fixed::<32>())?;
        let commitment = r.opt(|r| r.fixed::<32>())?;
        let device_id = r.fixed::<16>()?;
        let fingerprint = r.fixed::<32>()?;
        let nonce_domain = r.fixed::<4>()?;
        let invocation_counter = r.u64()?;
        let base_seq = r.u64()?;
        let payload_schema_version = r.u32()?;
        let engine_version = r.text(MAX_ENGINE_VERSION)?.to_owned();
        let schema_version = r.u32()?;
        let manifest = r.opt(|r| r.fixed::<32>())?;
        r.finish()?;

        let object = match (object_type, tag, commitment, manifest) {
            (OBJECT_ENVELOPE, Some(tag), Some(commitment), manifest) => ObjectKind::Envelope {
                idempotency_key_tag: IdempotencyKeyTag(tag),
                immutable_metadata_commitment: MetadataCommitment(commitment),
                artifact_manifest_commitment: manifest.map(ManifestCommitment),
            },
            (OBJECT_ENVELOPE, _, _, _)
            | (OBJECT_SNAPSHOT, Some(_), _, _)
            | (OBJECT_SNAPSHOT, _, Some(_), _) => return Err(WireError::ObjectFieldsMismatch),
            (OBJECT_SNAPSHOT, None, None, Some(manifest)) => ObjectKind::Snapshot {
                artifact_manifest_commitment: ManifestCommitment(manifest),
            },
            (OBJECT_SNAPSHOT, None, None, None) => return Err(WireError::SnapshotWithoutManifest),
            _ => return Err(WireError::UnknownObjectType),
        };
        if invocation_counter > MAX_INVOCATION_COUNTER {
            return Err(WireError::CounterAboveCap);
        }
        if !is_valid_engine_version(&engine_version) {
            return Err(WireError::BadEngineVersion);
        }
        Ok(Self {
            vault_id,
            key_epoch,
            object,
            object_id,
            device_id,
            device_key_fingerprint: Fingerprint(fingerprint),
            nonce_domain: NonceDomain(nonce_domain),
            invocation_counter,
            base_seq,
            payload_schema_version,
            engine_version,
            schema_version,
        })
    }
}

/// The container magic.
pub const CONTAINER_MAGIC: &[u8; 4] = b"DFSY";
/// The one container version this build speaks.
pub const CONTAINER_VERSION: u16 = 1;
/// The AES-256-GCM tag length: the least a sealed object's ciphertext can be.
pub const MIN_CIPHERTEXT_LEN: usize = 16;

/// A parsed container: the header, the header bytes exactly as received (the AAD), and the
/// ciphertext (AES-256-GCM output including its tag), which is never interpreted here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedContainer<'a> {
    /// The decoded header.
    pub header: ClearHeader,
    /// The header's bytes, borrowed from the input. They are the AAD; they equal
    /// `header.encode()` because the header grammar has one encoding per value.
    pub header_bytes: &'a [u8],
    /// The ciphertext, borrowed from the input.
    pub ciphertext: &'a [u8],
}

/// `magic "DFSY" ‖ u16 container_version ‖ u32 header_len ‖ header ‖ u64 ciphertext_len ‖
/// ciphertext`.
///
/// # Errors
/// As [`ClearHeader::encode`], plus [`WireError::TooLong`] if the header or ciphertext length
/// does not fit its length field.
pub fn encode_container(header: &ClearHeader, ciphertext: &[u8]) -> Result<Vec<u8>, WireError> {
    let header_bytes = header.encode()?;
    let header_len = u32::try_from(header_bytes.len()).map_err(|_| WireError::TooLong)?;
    let ciphertext_len = u64::try_from(ciphertext.len()).map_err(|_| WireError::TooLong)?;
    let mut out = Vec::with_capacity(4 + 2 + 4 + header_bytes.len() + 8 + ciphertext.len());
    out.extend_from_slice(CONTAINER_MAGIC);
    out.extend_from_slice(&CONTAINER_VERSION.to_be_bytes());
    out.extend_from_slice(&header_len.to_be_bytes());
    out.extend_from_slice(&header_bytes);
    out.extend_from_slice(&ciphertext_len.to_be_bytes());
    out.extend_from_slice(ciphertext);
    Ok(out)
}

/// Parse a container. Every length is checked against the bytes actually present before it is
/// used, nothing is allocated from an unchecked length, and any trailing byte is refused.
///
/// # Errors
/// [`WireError::BadMagic`], [`WireError::UnsupportedContainer`], [`WireError::Truncated`],
/// [`WireError::LengthExceedsInput`], [`WireError::CiphertextTooShort`] (under a 16-byte GCM
/// tag), [`WireError::TrailingBytes`], and anything
/// [`ClearHeader::decode`] returns (it must consume exactly `header_len` bytes).
pub fn parse_container(bytes: &[u8]) -> Result<ParsedContainer<'_>, WireError> {
    parse_container_inner(bytes)
}

/// The most a command envelope's container can hold: the header (labels, ids, digests and a
/// 64-byte engine version come to well under 512 bytes), the plaintext's payload bound of 2^24,
/// its delta manifest of at most 4096 digests, its metadata (a 256-byte actor id and the fixed
/// fields, under 1 KiB), the length fields, and the 16-byte GCM tag, rounded up to 4 KiB of
/// slack. A transport reads at most this many bytes of an envelope before it parses anything.
pub const MAX_ENVELOPE_CONTAINER_LEN: usize = (1 << 24) + 4096 * 32 + 4096;

/// [`parse_container`] for an object with a known size cap (every command envelope): a container
/// longer than `max_len` is refused before a byte of it is read, so a hostile service cannot make
/// the client parse, or buffer, an arbitrarily large object.
///
/// # Errors
/// [`WireError::ObjectTooLarge`] if `bytes.len() > max_len`, otherwise as [`parse_container`].
pub fn parse_container_with_limit(
    bytes: &[u8],
    max_len: usize,
) -> Result<ParsedContainer<'_>, WireError> {
    if bytes.len() > max_len {
        return Err(WireError::ObjectTooLarge);
    }
    parse_container_inner(bytes)
}

fn parse_container_inner(bytes: &[u8]) -> Result<ParsedContainer<'_>, WireError> {
    let mut r = Reader::new(bytes);
    if &r.fixed::<4>()? != CONTAINER_MAGIC {
        return Err(WireError::BadMagic);
    }
    if r.u16()? != CONTAINER_VERSION {
        return Err(WireError::UnsupportedContainer);
    }
    let header_len = usize::try_from(r.u32()?).map_err(|_| WireError::LengthExceedsInput)?;
    if header_len > r.remaining() {
        return Err(WireError::LengthExceedsInput);
    }
    let header_start = bytes.len() - r.remaining();
    let header_bytes = &bytes[header_start..header_start + header_len];
    let header = ClearHeader::decode(header_bytes)?;
    let mut r = Reader::new(&bytes[header_start + header_len..]);
    let ciphertext_len = usize::try_from(r.u64()?).map_err(|_| WireError::LengthExceedsInput)?;
    if ciphertext_len > r.remaining() {
        return Err(WireError::LengthExceedsInput);
    }
    let ciphertext_start = bytes.len() - r.remaining();
    if ciphertext_len < MIN_CIPHERTEXT_LEN {
        return Err(WireError::CiphertextTooShort);
    }
    let ciphertext = &bytes[ciphertext_start..ciphertext_start + ciphertext_len];
    Reader::new(&bytes[ciphertext_start + ciphertext_len..]).finish()?;
    Ok(ParsedContainer {
        header,
        header_bytes,
        ciphertext,
    })
}
