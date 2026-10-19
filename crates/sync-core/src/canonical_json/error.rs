//! The errors of the canonical JSON codec.
//!
//! Payloads carry financial values, so no error message embeds a value taken from the
//! document: the variants name the rule that was broken, and the one place a serde
//! message could carry a value ([`CanonicalJsonError::Decode`]) is redacted from both
//! `Display` and `Debug`.

use std::fmt;

use thiserror::Error;

/// A message that is available to tests and debuggers but never printed.
#[derive(Clone, PartialEq, Eq)]
pub struct Redacted(String);

impl Redacted {
    pub(crate) fn new(message: impl fmt::Display) -> Self {
        Self(message.to_string())
    }

    /// The underlying message. It may contain a value from the document, so callers must
    /// not log it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Redacted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// Why a document could not be encoded or decoded.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum CanonicalJsonError {
    /// The document contains a JSON number. Every integer is a decimal string (C.1.2).
    #[error("a JSON number is not allowed: every integer is a decimal string")]
    NumberNotAllowed,
    /// A floating-point value was serialized; none may appear (C.1.2).
    #[error("a floating-point value is not allowed")]
    Float,
    /// A value of a kind the format has no representation for (for example raw bytes).
    #[error("unsupported value kind: {0}")]
    Unsupported(&'static str),
    /// An integer string does not match `-?(0|[1-9][0-9]*)`.
    #[error("an integer string is malformed")]
    BadInteger,
    /// An integer string is well formed but outside the range of its field.
    #[error("an integer is out of range for its field")]
    IntegerOutOfRange,
    /// Two members of one object have the same name.
    #[error("an object has two members with the same name")]
    DuplicateMember,
    /// A member name is not ASCII `[a-z0-9_]`.
    #[error("a member name is not ASCII [a-z0-9_]")]
    BadMemberName,
    /// The text is not valid JSON, or breaks a string rule (invalid UTF-8, a lone
    /// surrogate escape, a raw control character in a string).
    #[error("malformed JSON at byte {offset}: {reason}")]
    Malformed {
        /// Byte offset of the problem.
        offset: usize,
        /// Which rule was broken.
        reason: &'static str,
    },
    /// The document nests deeper than the limit.
    #[error("the document is nested more than {0} levels deep")]
    TooDeep(usize),
    /// The document is valid but is not the canonical form of the value it decodes to.
    #[error("the document is not in canonical form")]
    NotCanonical,
    /// The document does not have the structure the target type expects.
    #[error("the document does not have the expected structure")]
    Decode(Redacted),
}

impl serde::ser::Error for CanonicalJsonError {
    fn custom<T: fmt::Display>(message: T) -> Self {
        Self::Decode(Redacted::new(message))
    }
}

impl serde::de::Error for CanonicalJsonError {
    fn custom<T: fmt::Display>(message: T) -> Self {
        Self::Decode(Redacted::new(message))
    }
}
