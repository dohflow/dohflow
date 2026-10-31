//! Deterministic derived identifiers for rows minted while a command is applied
//! (ADR 0013 Addendum A, personal-cfo-1df8d; owned by personal-cfo-t2s4b).
//!
//! An id minted by `apply` must be a pure function of the command, so replaying the same
//! command on another device mints the same ids. A process-local counter cannot be part
//! of a replicated result; this module derives the id from the command's metadata, the
//! entity tag, the mint ordinal and a per-vault key instead:
//!
//! ```text
//! unix_ms = issued_at as Unix milliseconds
//! rand_a  = ordinal                               (12 bits, 0..=4095)
//! rand_b  = high 62 bits of HMAC-SHA256(id_key, command_id || u8(tag_len) || tag || u16_be(ordinal))
//! ```
//!
//! packed by `uuid::Builder::from_unix_timestamp_millis` into a UUIDv7. The byte grammar
//! is normative, and the ADR's two known-answer vectors are checked by this crate's tests
//! against the ADR text itself.
//!
//! The only way to mint is [`DerivedIds`], which hands out ordinals in order and never
//! repeats one. The function that derives an id from an explicit ordinal is private to
//! this crate: a public one would let an `apply` path pass a hand-picked ordinal and
//! bypass the counter, and a cloned allocator would restart from the same ordinal, so
//! `DerivedIds` is deliberately not `Clone` (both found by the adversarial review).
//!
//! The `id_key` is **not a secret**: it is `SHA-256("dohflow/derived-id-key/v1" ||
//! vault_id)`, an identifier-domain constant that every device derives from the vault id.
//! Within a command, ids keep creation order because the ordinal occupies the bits right
//! after the millisecond timestamp, so `ORDER BY id` reproduces mint order.

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::{Builder, Uuid};

use crate::IssuedAt;

/// The label hashed with the vault id to form the per-vault identifier key.
pub const ID_KEY_LABEL: &[u8] = b"dohflow/derived-id-key/v1";

/// The domain label of [`DerivedIdKey::keyed_id`].
const KEYED_ID_LABEL: &[u8] = b"dohflow/keyed-id/v1";

/// The domain label of [`DerivedIdKey::singleton_id`].
const SINGLETON_ID_LABEL: &[u8] = b"dohflow/singleton-id/v1";

/// How many ids one command may mint: ordinals `0..MAX_ORDINALS` fit `rand_a`'s 12 bits.
pub const MAX_ORDINALS: u32 = 4096;

/// The longest entity tag, in bytes: its length is a single octet in the HMAC message.
pub const MAX_TAG_LEN: usize = 255;

/// The largest `unix_ms` a UUIDv7 can carry (48 bits).
const MAX_UNIX_MILLIS: i64 = (1_i64 << 48) - 1;

/// Why an identifier could not be derived. All are validation errors raised before any
/// database write.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum DerivedIdError {
    /// The entity tag is empty.
    #[error("a derived-id tag must not be empty")]
    EmptyTag,
    /// The entity tag is longer than 255 bytes.
    #[error("a derived-id tag is at most {MAX_TAG_LEN} bytes, got {len}")]
    TagTooLong {
        /// The tag's length in bytes.
        len: usize,
    },
    /// The entity tag has a byte outside printable US-ASCII (`0x21..=0x7e`).
    #[error("a derived-id tag must be printable US-ASCII without spaces")]
    TagNotPrintableAscii,
    /// The command already minted its 4,096 ids (`DerivedIdOrdinalOverflow`).
    #[error(
        "command {command_id} minted more than {MAX_ORDINALS} ids (attempted ordinal {attempted})"
    )]
    OrdinalOverflow {
        /// The command that ran out of ordinals.
        command_id: Uuid,
        /// The ordinal that was refused (always 4096).
        attempted: u32,
    },
    /// `issued_at` is before the Unix epoch or past what 48 bits of milliseconds hold.
    #[error("issued_at is outside the range a UUIDv7 timestamp can carry")]
    IssuedAtOutOfRange,
    /// A genesis snapshot carried an id key that is not the one its vault id derives.
    #[error("the genesis id key does not match the key derived from the vault id")]
    GenesisKeyMismatch,
}

/// The per-vault, non-secret key that identifiers are derived under.
///
/// `SHA-256("dohflow/derived-id-key/v1" || vault_id.bytes)`. Genesis carries this value
/// beside `vault_id`, and a receiver recomputes it and rejects a mismatch.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct DerivedIdKey([u8; 32]);

impl DerivedIdKey {
    /// The key for a vault.
    #[must_use]
    pub fn for_vault(vault_id: &Uuid) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(ID_KEY_LABEL);
        hasher.update(vault_id.as_bytes());
        Self(hasher.finalize().into())
    }

    /// The key a genesis snapshot carries beside its `vault_id`, checked: a receiver
    /// recomputes the key from the vault id and refuses a mismatch before applying the
    /// snapshot. This is the only way to build a key from received bytes.
    ///
    /// # Errors
    /// [`DerivedIdError::GenesisKeyMismatch`] if `bytes` is not the key `vault_id` derives.
    pub fn from_genesis(vault_id: &Uuid, bytes: [u8; 32]) -> Result<Self, DerivedIdError> {
        let derived = Self::for_vault(vault_id);
        if derived.0 == bytes {
            Ok(derived)
        } else {
            Err(DerivedIdError::GenesisKeyMismatch)
        }
    }

    /// A KEYED id: the id of a row minted under a BASE-DEPENDENT branch of a command, derived
    /// from the command, an entity tag and the row's natural key rather than from an ordinal, so
    /// the number of ordinals a command consumes depends only on its payload (ADR 0013-A section
    /// 10(a)). Re-applying the same command on another base mints the same id for the same
    /// natural key; another command (a second reversal, a later scenario apply) mints another.
    ///
    /// ```text
    /// mac = HMAC-SHA256(id_key, "dohflow/keyed-id/v1" || command_id || u8(tag_len) || tag
    ///                           || for each part: u32_be(len) || part)
    /// id  = mac[0..16] with the version nibble set to 8 and the variant bits to 10
    /// ```
    ///
    /// Every part is length-prefixed, so `("ab", "c")` and `("a", "bc")` differ. A UUIDv8 layout,
    /// disjoint from the UUIDv7 ordinal ids.
    ///
    /// # Errors
    /// [`DerivedIdError`] for an invalid tag, or a part longer than `u32::MAX` bytes.
    pub fn keyed_id(
        &self,
        command_id: &Uuid,
        tag: &str,
        natural_key: &[&[u8]],
    ) -> Result<Uuid, DerivedIdError> {
        let tag = tag.as_bytes();
        validate_tag(tag)?;
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&self.0)
            .expect("HMAC accepts a key of any length");
        mac.update(KEYED_ID_LABEL);
        mac.update(command_id.as_bytes());
        mac.update(&[u8::try_from(tag.len()).expect("tag length at most 255")]);
        mac.update(tag);
        for part in natural_key {
            let length = u32::try_from(part.len())
                .map_err(|_| DerivedIdError::TagTooLong { len: part.len() })?;
            mac.update(&length.to_be_bytes());
            mac.update(part);
        }
        let out: [u8; 32] = mac.finalize().into_bytes().into();
        let mut bytes: [u8; 16] = out[..16].try_into().expect("sixteen bytes");
        bytes[6] = (bytes[6] & 0x0f) | 0x80;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        Ok(Uuid::from_bytes(bytes))
    }

    /// The id of a per-vault SINGLETON row (one household, one primary member, a system
    /// account): a pure function of the vault and a fixed label, so two devices that create
    /// or migrate the same vault independently agree, and a replica inherits it through
    /// bootstrap instead of seeding its own.
    ///
    /// ```text
    /// mac = HMAC-SHA256(id_key, "dohflow/singleton-id/v1" || u8(label_len) || label)
    /// id  = mac[0..16] with the version nibble set to 8 and the variant bits to 10
    /// ```
    ///
    /// A UUIDv8 (custom) layout, distinct from the UUIDv7 command-derived ids. The id is for
    /// LOCAL use: it is derived from the sync-visible `vault_id` and so must never be sent to
    /// the platform service, which would otherwise be able to link records by recomputing it.
    ///
    /// # Errors
    /// [`DerivedIdError`] for an empty, over-long or non-printable label.
    pub fn singleton_id(&self, label: &str) -> Result<Uuid, DerivedIdError> {
        let label = label.as_bytes();
        validate_tag(label)?;
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&self.0)
            .expect("HMAC accepts a key of any length");
        mac.update(SINGLETON_ID_LABEL);
        mac.update(&[u8::try_from(label.len()).expect("label length at most 255")]);
        mac.update(label);
        let out: [u8; 32] = mac.finalize().into_bytes().into();
        let mut bytes: [u8; 16] = out[..16].try_into().expect("sixteen bytes");
        bytes[6] = (bytes[6] & 0x0f) | 0x80;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        Ok(Uuid::from_bytes(bytes))
    }

    /// The 32 key bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl std::fmt::Debug for DerivedIdKey {
    // Not a secret, but there is no reason to print 32 bytes in a log line.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DerivedIdKey(..)")
    }
}

/// Everything derived from one set of inputs, for the known-answer tests.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct DerivationTrace {
    /// The HMAC input: `command_id || u8(tag_len) || tag || u16_be(ordinal)`.
    pub(crate) message: Vec<u8>,
    /// The full HMAC-SHA256 output.
    pub(crate) hmac: [u8; 32],
    /// The ten bytes handed to `uuid::Builder::from_unix_timestamp_millis`.
    pub(crate) counter_random_bytes: [u8; 10],
    /// The derived identifier.
    pub(crate) uuid: Uuid,
}

fn validate_tag(tag: &[u8]) -> Result<(), DerivedIdError> {
    if tag.is_empty() {
        return Err(DerivedIdError::EmptyTag);
    }
    if tag.len() > MAX_TAG_LEN {
        return Err(DerivedIdError::TagTooLong { len: tag.len() });
    }
    if !tag.iter().all(|b| (0x21..=0x7e).contains(b)) {
        return Err(DerivedIdError::TagNotPrintableAscii);
    }
    Ok(())
}

/// Derive an identifier and return every intermediate value.
///
/// Private to the crate on purpose: it takes an explicit ordinal, so exposing it would let
/// a caller bypass [`DerivedIds`]' counter.
///
/// # Errors
/// [`DerivedIdError`] for an invalid tag, an ordinal of 4096 or more, or an `issued_at`
/// outside the 48-bit millisecond range.
pub(crate) fn derive_id_traced(
    key: &DerivedIdKey,
    issued_at: IssuedAt,
    command_id: &Uuid,
    tag: &str,
    ordinal: u32,
) -> Result<DerivationTrace, DerivedIdError> {
    let tag = tag.as_bytes();
    validate_tag(tag)?;
    if ordinal >= MAX_ORDINALS {
        return Err(DerivedIdError::OrdinalOverflow {
            command_id: *command_id,
            attempted: ordinal,
        });
    }
    let unix_ms = issued_at.as_unix_millis();
    if !(0..=MAX_UNIX_MILLIS).contains(&unix_ms) {
        return Err(DerivedIdError::IssuedAtOutOfRange);
    }
    // Both conversions are in range: the ordinal is below 4096 and the tag length is at
    // most 255 (checked above).
    let ordinal = u16::try_from(ordinal).expect("ordinal below 4096");
    let tag_len = u8::try_from(tag.len()).expect("tag length at most 255");
    let unix_ms = u64::try_from(unix_ms).expect("unix_ms is non-negative");

    let mut message = Vec::with_capacity(16 + 1 + tag.len() + 2);
    message.extend_from_slice(command_id.as_bytes());
    message.push(tag_len);
    message.extend_from_slice(tag);
    message.extend_from_slice(&ordinal.to_be_bytes());

    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key.as_bytes())
        .expect("HMAC accepts a key of any length");
    mac.update(&message);
    let hmac: [u8; 32] = mac.finalize().into_bytes().into();

    // The high 62 bits of the digest as a right-aligned big-endian integer. Copying the
    // raw first eight digest bytes into the builder instead would be a different id.
    let high = u64::from_be_bytes(hmac[0..8].try_into().expect("eight bytes"));
    let rand_b = high >> 2;

    let mut counter_random_bytes = [0_u8; 10];
    counter_random_bytes[0] = u8::try_from((ordinal >> 8) & 0x0f).expect("masked to four bits");
    counter_random_bytes[1] = u8::try_from(ordinal & 0xff).expect("masked to eight bits");
    counter_random_bytes[2..10].copy_from_slice(&rand_b.to_be_bytes());

    let uuid = Builder::from_unix_timestamp_millis(unix_ms, &counter_random_bytes).into_uuid();
    Ok(DerivationTrace {
        message,
        hmac,
        counter_random_bytes,
        uuid,
    })
}

/// Derive the identifier for the `ordinal`-th entity of kind `tag` minted by a command.
///
/// A pure function of its arguments: the same command on any device, in any process,
/// derives the same id. Private to the crate, like [`derive_id_traced`].
///
/// # Errors
/// See [`derive_id_traced`].
pub(crate) fn derive_id(
    key: &DerivedIdKey,
    issued_at: IssuedAt,
    command_id: &Uuid,
    tag: &str,
    ordinal: u32,
) -> Result<Uuid, DerivedIdError> {
    derive_id_traced(key, issued_at, command_id, tag, ordinal).map(|trace| trace.uuid)
}

/// The id allocator one command's apply carries: the ordinal counter and the 4,096 cap.
///
/// Ordinals are handed out in the order an apply path would otherwise mint ids, so
/// `ORDER BY id` keeps creation order within the command. Once the cap is hit every
/// further call fails, so a command can never silently wrap or reuse an ordinal.
///
/// Not `Clone`: a copy would restart from the same ordinal and mint the same ids twice.
///
/// ```compile_fail,E0277
/// fn needs_clone<T: Clone>() {}
/// needs_clone::<core_ids::DerivedIds>();
/// ```
///
/// There is no public function that derives an id from an explicit ordinal either (this
/// guards the crate-root re-export; the function itself is `pub(crate)`):
///
/// ```compile_fail,E0425
/// let _ = core_ids::derive_id;
/// ```
#[derive(Debug)]
pub struct DerivedIds {
    key: DerivedIdKey,
    issued_at: IssuedAt,
    command_id: Uuid,
    next_ordinal: u32,
}

impl DerivedIds {
    /// An allocator for one command, starting at ordinal 0.
    #[must_use]
    pub const fn new(key: DerivedIdKey, issued_at: IssuedAt, command_id: Uuid) -> Self {
        Self {
            key,
            issued_at,
            command_id,
            next_ordinal: 0,
        }
    }

    /// Mint the next identifier, consuming one ordinal.
    ///
    /// # Errors
    /// [`DerivedIdError::OrdinalOverflow`] (`attempted: 4096`) for the 4,097th id, and the
    /// tag and range errors of [`derive_id`]. A failed call consumes no ordinal.
    pub fn next(&mut self, tag: &str) -> Result<Uuid, DerivedIdError> {
        let id = derive_id(
            &self.key,
            self.issued_at,
            &self.command_id,
            tag,
            self.next_ordinal,
        )?;
        self.next_ordinal += 1;
        Ok(id)
    }

    /// A keyed id for this command (see [`DerivedIdKey::keyed_id`]): consumes NO ordinal, so a
    /// mint under a base-dependent branch never shifts the ordinals of the ids after it.
    ///
    /// # Errors
    /// See [`DerivedIdKey::keyed_id`].
    pub fn keyed(&self, tag: &str, natural_key: &[&[u8]]) -> Result<Uuid, DerivedIdError> {
        self.key.keyed_id(&self.command_id, tag, natural_key)
    }

    /// How many identifiers this command has minted so far.
    #[must_use]
    pub const fn minted(&self) -> u32 {
        self.next_ordinal
    }

    /// How many more identifiers this command may mint.
    #[must_use]
    pub const fn remaining(&self) -> u32 {
        MAX_ORDINALS - self.next_ordinal
    }
}

#[cfg(test)]
mod tests;
