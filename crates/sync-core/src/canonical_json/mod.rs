//! Canonical JSON (restricted RFC 8785) for Sync documents: command payloads, manifests
//! and logical metadata (`docs/architecture/vault-format.md` C.1.2).
//!
//! The format is RFC 8785 JSON under restrictions that make JCS's IEEE-754 number rules
//! irrelevant:
//!
//! * **no JSON number appears at all**: every integer, of any width, is a decimal string
//!   `-?(0|[1-9][0-9]*)`, and no floating-point value exists;
//! * member names are ASCII `[a-z0-9_]`, sorted by UTF-16 code units (byte order for
//!   ASCII), with no insignificant whitespace;
//! * strings use the JCS escaping rules; invalid UTF-8, lone surrogate escapes and
//!   duplicate member names are rejected; no Unicode normalization is applied.
//!
//! The byte-for-byte RFC 8785 output (ordering, escaping) comes from the pinned
//! `serde_json_canonicalizer` crate, called only through `to_vec`. Everything the crate
//! does not enforce is enforced here: numbers and floats never reach it, because
//! [`to_canonical_vec`] turns every integer into a string first and refuses floats; and
//! [`from_canonical_slice`] parses with a strict parser that rejects duplicates, numbers
//! and bad strings, decodes integers out of their strings, and finally **re-encodes the
//! decoded value and requires the original bytes**, so a document that is valid but not
//! canonical (whitespace, member order, an alternative escape spelling, an unknown
//! member, `"-0"`) is refused rather than accepted under a different spelling.
//!
//! **Hashes and signatures are always over the bytes that were received**, never over a
//! re-encoding of what was parsed. [`parse_node_lenient`] accepts any whitespace and any
//! escape spelling (it still enforces every content rule), so using its result to produce
//! a hash would give different wire bytes the same hash; [`parse_canonical_node`] and
//! [`from_canonical_slice`] are the entry points for a document that arrives from outside.
//!
//! One value, one encoding: an `Option` whose content itself encodes as `null` (`Some(())`,
//! `Some(None)`) is refused, because it would encode like `None`; strings never contain
//! U+0000; nesting is at most [`MAX_DEPTH`] levels, on encoding as on parsing. User data is
//! never a map key (keys follow the member-name rule), and binary data is a base64 string
//! newtype, not a byte sequence.
//!
//! Payload types may use `#[derive(Serialize, Deserialize)]` with externally tagged,
//! snake-case enums. They may not use `#[serde(flatten)]`, `untagged` or the `tag`
//! attributes, which buffer values and would read integers out of the buffer as numbers.

mod de;
mod error;
mod node;
mod parse;
mod ser;

use serde::de::DeserializeOwned;
use serde::Serialize;

pub use error::{CanonicalJsonError, Redacted};
pub use node::MAX_DEPTH;
pub use node::{is_member_name, Node};
pub use parse::{parse_canonical_node, parse_node_lenient};

/// Encode a value as canonical JSON bytes.
///
/// # Errors
/// [`CanonicalJsonError`] if the value contains a float or raw bytes, has a member name
/// outside `[a-z0-9_]`, or has two members with the same name.
pub fn to_canonical_vec<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, CanonicalJsonError> {
    let node = value.serialize(ser::NodeSerializer)?;
    node_to_canonical_vec(&node)
}

/// Encode an already built [`Node`] as canonical JSON bytes.
///
/// # Errors
/// [`CanonicalJsonError::BadMemberName`] if any object has a member name outside
/// `[a-z0-9_]` (a [`Node`] can be built by hand, bypassing the checks of the parser and
/// the serializer); [`CanonicalJsonError::Decode`] if the encoder itself fails, which it
/// cannot for a number-free tree.
pub fn node_to_canonical_vec(node: &Node) -> Result<Vec<u8>, CanonicalJsonError> {
    node.validate()?;
    serde_json_canonicalizer::to_vec(node).map_err(|e| CanonicalJsonError::Decode(Redacted::new(e)))
}

/// Decode canonical JSON bytes into a value, rejecting any document that is not the
/// canonical encoding of that value.
///
/// # Errors
/// [`CanonicalJsonError`] naming the rule that was broken, including
/// [`CanonicalJsonError::NotCanonical`] for a valid document in a non-canonical spelling.
pub fn from_canonical_slice<T: DeserializeOwned + Serialize>(
    bytes: &[u8],
) -> Result<T, CanonicalJsonError> {
    let node = parse_node_lenient(bytes)?;
    let value = T::deserialize(de::NodeDeserializer(node))?;
    if to_canonical_vec(&value)? != bytes {
        return Err(CanonicalJsonError::NotCanonical);
    }
    Ok(value)
}
