//! Versioned command payloads (personal-cfo-t2s4b part 3b; `docs/architecture/vault-format.md`
//! C.1.2 and the `EnvelopePlaintextV1` payload field).
//!
//! A [`WriteCommand`] is the unit a Sync replica replays, so it needs a stable, versioned
//! serialization. The encoding is the restricted canonical JSON of the pure `sync-core`
//! crate: every integer a decimal string, no floats, ASCII snake-case member names, and
//! externally tagged enums (`{"set_account_note": {...}}`, `"unit_variant"`).
//!
//! # The schema version
//!
//! [`PAYLOAD_SCHEMA_VERSION`] names the shape of the whole encoding: every variant, every
//! field, every nested type, and the spelling of every token. It is part of the envelope's
//! clear header and of the operation-log row (part 4). **Any change to a payload's shape
//! requires bumping it**, and the previous version's frozen fixtures
//! (`crates/db-worker/tests/fixtures/payload_v1/`) stay in the repository as the
//! compatibility record. The tests make that mechanical: they re-encode a sample of every
//! variant and compare the bytes against the frozen fixtures and a pinned digest of them,
//! so a field added, renamed or retyped fails the test until the version is bumped on
//! purpose.
//!
//! Decoding is strict in the two ways that matter for replay. A payload of a version this
//! build does not know is refused with [`PayloadError::UnsupportedVersion`], loudly, never
//! decoded as if it were current. And a document that is not the canonical encoding of the
//! command it decodes to (an unknown member, a different spelling) is refused too, so two
//! byte strings can never mean the same command.
//!
//! Payload types may use `#[derive(Serialize, Deserialize)]` with externally tagged,
//! snake-case enums. `#[serde(flatten)]`, `untagged` and the `tag` / `content` attributes
//! are not allowed: they buffer values and would read integers back out of the buffer as
//! numbers. A source-scan test enforces this.

use sync_core::canonical_json::{from_canonical_slice, to_canonical_vec, CanonicalJsonError};
use thiserror::Error;

use crate::WriteCommand;

/// The version of the command-payload encoding this build writes and reads.
pub const PAYLOAD_SCHEMA_VERSION: u32 = 1;

/// Why a payload could not be encoded or decoded.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum PayloadError {
    /// The payload was written under a schema version this build does not read.
    #[error(
        "payload schema version {found} is not supported (this build reads version {supported})"
    )]
    UnsupportedVersion {
        /// The version the payload claims.
        found: u32,
        /// The version this build reads.
        supported: u32,
    },
    /// The canonical JSON codec refused the document or the value.
    #[error("payload encoding error: {0}")]
    Codec(#[from] CanonicalJsonError),
}

/// Encode a command as its current-version payload bytes (canonical JSON).
///
/// # Errors
/// [`PayloadError::Codec`] if the command holds a value the format cannot carry (a
/// float, raw bytes, or a member name outside `[a-z0-9_]`).
pub fn encode_payload(command: &WriteCommand) -> Result<Vec<u8>, PayloadError> {
    Ok(to_canonical_vec(command)?)
}

/// Decode a payload written under `version`.
///
/// # Errors
/// [`PayloadError::UnsupportedVersion`] if `version` is not [`PAYLOAD_SCHEMA_VERSION`];
/// [`PayloadError::Codec`] if the bytes are not the canonical encoding of a command.
pub fn decode_payload(version: u32, bytes: &[u8]) -> Result<WriteCommand, PayloadError> {
    if version != PAYLOAD_SCHEMA_VERSION {
        return Err(PayloadError::UnsupportedVersion {
            found: version,
            supported: PAYLOAD_SCHEMA_VERSION,
        });
    }
    Ok(from_canonical_slice(bytes)?)
}

/// A command together with the exact payload bytes replicas will decode it from.
#[derive(Debug, Clone)]
pub struct PreparedPayload {
    /// The canonical payload bytes (what the envelope carries and the write-set hash binds).
    pub bytes: Vec<u8>,
    /// The command **decoded from those bytes**. The origin applies this, not the value it
    /// was given, so it applies exactly what every replica will apply (an encoding that is
    /// not one-to-one would otherwise let the origin and the replicas diverge).
    pub command: WriteCommand,
}

/// Encode a command and decode it straight back, returning both.
///
/// # Errors
/// [`PayloadError`] if the command cannot be encoded, or if the bytes do not decode to a
/// command (which would mean the encoding is not round-trippable).
pub fn prepare_payload(command: &WriteCommand) -> Result<PreparedPayload, PayloadError> {
    let bytes = encode_payload(command)?;
    let command = decode_payload(PAYLOAD_SCHEMA_VERSION, &bytes)?;
    Ok(PreparedPayload { bytes, command })
}
