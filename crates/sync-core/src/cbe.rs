//! CBE1, the canonical binary encoding of `docs/architecture/vault-format.md` C.1.1
//! (personal-cfo-t2s4b part 6a).
//!
//! Every value that is hashed, signed, authenticated as AAD, or used as HKDF `info` is encoded
//! with this one scheme, so each logical value has exactly one byte string. [`Writer`] and
//! [`Reader`] are the only way structures are built or parsed, and one field table per structure
//! drives both.
//!
//! | Kind | Encoding |
//! | --- | --- |
//! | `label` | `u16` length (1 to 64), then ASCII bytes from `[a-z0-9/_.-]` |
//! | `u8`..`u64`, `i64` | fixed-width, big-endian (two's complement for `i64`) |
//! | `uuid`, `digest`, `fingerprint` | 16 / 32 / 32 raw bytes |
//! | `bytes(max)` | `u32` length no greater than `max`, then the bytes |
//! | `text(max)` | `bytes(max)` whose content is valid UTF-8 (`max` counts bytes) |
//! | `opt(T)` | presence byte `0` or `1`, then `T` if present |
//! | `list(T, max)` | `u32` count no greater than `max`, then the items |
//!
//! A [`Reader`] rejects: trailing bytes after the last field ([`Reader::finish`]), a length above
//! its stated maximum, a presence byte other than `0` or `1`, invalid UTF-8 in a `text`, an unknown
//! or mismatched label, and truncation. It never skips an unknown trailing field, and it never
//! allocates from a length it has not first checked against the bytes actually present, so a
//! hostile length cannot make it reserve memory (this code runs before authentication).

use thiserror::Error;

/// Why a wire structure was refused. Every variant fails closed: the parser returns nothing
/// partial, and the message names a rule, never a value from the input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum WireError {
    /// The input ended before the structure did.
    #[error("the object is truncated")]
    Truncated,
    /// Bytes remain after the last field.
    #[error("trailing bytes follow the object")]
    TrailingBytes,
    /// A length or count is above its stated maximum.
    #[error("a length is above its maximum")]
    TooLong,
    /// A length claims more bytes than the input holds.
    #[error("a length exceeds the bytes present")]
    LengthExceedsInput,
    /// A presence byte is neither `0` nor `1`.
    #[error("a presence byte is neither 0 nor 1")]
    BadPresence,
    /// A `text` is not valid UTF-8.
    #[error("a text field is not valid UTF-8")]
    InvalidUtf8,
    /// A label is malformed (length 1 to 64, ASCII from `[a-z0-9/_.-]`).
    #[error("a label is malformed")]
    BadLabel,
    /// A well-formed label is not the one this structure starts with.
    #[error("the object's label is not the expected one")]
    LabelMismatch,
    /// The `object_type` byte is neither `1` (envelope) nor `2` (snapshot).
    #[error("unknown object type")]
    UnknownObjectType,
    /// The dual-identity fields (tag, commitment) do not match the object type.
    #[error("the idempotency tag and metadata commitment do not match the object type")]
    ObjectFieldsMismatch,
    /// A snapshot header carries no manifest commitment (it is always present for a snapshot).
    #[error("a snapshot header must carry its manifest commitment")]
    SnapshotWithoutManifest,
    /// The invocation counter is above the per-key cap of `2^32 - 1`.
    #[error("the invocation counter is above the per-key cap")]
    CounterAboveCap,
    /// The container magic is not `DFSY`.
    #[error("bad container magic")]
    BadMagic,
    /// The protocol version is not one this build supports.
    #[error("unsupported protocol version")]
    UnsupportedProtocol,
    /// The container version is not one this build supports.
    #[error("unsupported container version")]
    UnsupportedContainer,
}

impl WireError {
    /// Whether this is the typed `MalformedObject` failure of C.11 (any CBE1 or container
    /// violation), as opposed to `UnsupportedProtocol`.
    #[must_use]
    pub const fn is_malformed(&self) -> bool {
        !matches!(self, Self::UnsupportedProtocol | Self::UnsupportedContainer)
    }
}

fn valid_label(label: &str) -> bool {
    (1..=64).contains(&label.len())
        && label
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"/_.-".contains(&b))
}

/// Builds one CBE1 structure. Structures start with their label; the rest is appended in field
/// order.
#[derive(Debug, Default)]
pub struct Writer(Vec<u8>);

impl Writer {
    /// A writer that starts with `label`.
    ///
    /// # Panics
    /// If `label` is not a valid label. Labels are constants of this crate, so a bad one is a
    /// programming error, caught by every test that builds the structure.
    #[must_use]
    pub fn with_label(label: &str) -> Self {
        assert!(valid_label(label), "bad CBE1 label {label:?}");
        let mut writer = Self::default();
        writer
            .0
            .extend_from_slice(&(label.len() as u16).to_be_bytes());
        writer.0.extend_from_slice(label.as_bytes());
        writer
    }

    /// Append a `u8`.
    pub fn u8(&mut self, value: u8) -> &mut Self {
        self.0.push(value);
        self
    }

    /// Append a big-endian `u16`.
    pub fn u16(&mut self, value: u16) -> &mut Self {
        self.0.extend_from_slice(&value.to_be_bytes());
        self
    }

    /// Append a big-endian `u32`.
    pub fn u32(&mut self, value: u32) -> &mut Self {
        self.0.extend_from_slice(&value.to_be_bytes());
        self
    }

    /// Append a big-endian `u64`.
    pub fn u64(&mut self, value: u64) -> &mut Self {
        self.0.extend_from_slice(&value.to_be_bytes());
        self
    }

    /// Append a big-endian two's-complement `i64`.
    pub fn i64(&mut self, value: i64) -> &mut Self {
        self.0.extend_from_slice(&value.to_be_bytes());
        self
    }

    /// Append a fixed-width raw field (`uuid`, `digest`, `fingerprint`, a 4-byte nonce domain).
    pub fn fixed<const N: usize>(&mut self, value: &[u8; N]) -> &mut Self {
        self.0.extend_from_slice(value);
        self
    }

    /// Append `bytes(max)`.
    ///
    /// # Errors
    /// [`WireError::TooLong`] if `value` is longer than `max` (or than a `u32`).
    pub fn bytes(&mut self, max: usize, value: &[u8]) -> Result<&mut Self, WireError> {
        if value.len() > max {
            return Err(WireError::TooLong);
        }
        let length = u32::try_from(value.len()).map_err(|_| WireError::TooLong)?;
        self.0.extend_from_slice(&length.to_be_bytes());
        self.0.extend_from_slice(value);
        Ok(self)
    }

    /// Append `text(max)`; `max` counts bytes.
    ///
    /// # Errors
    /// [`WireError::TooLong`] if the UTF-8 encoding is longer than `max`.
    pub fn text(&mut self, max: usize, value: &str) -> Result<&mut Self, WireError> {
        self.bytes(max, value.as_bytes())
    }

    /// Append `opt(T)`: `0` when absent, else `1` and whatever `write` appends.
    pub fn opt<T>(&mut self, value: Option<&T>, write: impl FnOnce(&mut Self, &T)) -> &mut Self {
        match value {
            None => self.0.push(0),
            Some(inner) => {
                self.0.push(1);
                write(self, inner);
            }
        }
        self
    }

    /// The bytes written so far.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Finish and take the bytes.
    #[must_use]
    pub fn finish(self) -> Vec<u8> {
        self.0
    }
}

/// Parses one CBE1 structure from a byte slice, strictly.
#[derive(Debug, Clone, Copy)]
pub struct Reader<'a> {
    rest: &'a [u8],
}

impl<'a> Reader<'a> {
    /// A reader over `input`.
    #[must_use]
    pub const fn new(input: &'a [u8]) -> Self {
        Self { rest: input }
    }

    /// How many bytes are left.
    #[must_use]
    pub const fn remaining(&self) -> usize {
        self.rest.len()
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], WireError> {
        if self.rest.len() < n {
            return Err(WireError::Truncated);
        }
        let (head, tail) = self.rest.split_at(n);
        self.rest = tail;
        Ok(head)
    }

    /// Read and check the structure's label.
    ///
    /// # Errors
    /// [`WireError::BadLabel`] for a malformed label, [`WireError::LabelMismatch`] for a
    /// well-formed label that is not `expected`, [`WireError::Truncated`].
    pub fn label(&mut self, expected: &str) -> Result<(), WireError> {
        let length = usize::from(self.u16()?);
        if !(1..=64).contains(&length) {
            return Err(WireError::BadLabel);
        }
        let bytes = self.take(length)?;
        let text = std::str::from_utf8(bytes).map_err(|_| WireError::BadLabel)?;
        if !valid_label(text) {
            return Err(WireError::BadLabel);
        }
        if text == expected {
            Ok(())
        } else {
            Err(WireError::LabelMismatch)
        }
    }

    /// Read a `u8`.
    ///
    /// # Errors
    /// [`WireError::Truncated`].
    pub fn u8(&mut self) -> Result<u8, WireError> {
        Ok(self.take(1)?[0])
    }

    /// Read a big-endian `u16`.
    ///
    /// # Errors
    /// [`WireError::Truncated`].
    pub fn u16(&mut self) -> Result<u16, WireError> {
        Ok(u16::from_be_bytes(self.fixed()?))
    }

    /// Read a big-endian `u32`.
    ///
    /// # Errors
    /// [`WireError::Truncated`].
    pub fn u32(&mut self) -> Result<u32, WireError> {
        Ok(u32::from_be_bytes(self.fixed()?))
    }

    /// Read a big-endian `u64`.
    ///
    /// # Errors
    /// [`WireError::Truncated`].
    pub fn u64(&mut self) -> Result<u64, WireError> {
        Ok(u64::from_be_bytes(self.fixed()?))
    }

    /// Read a big-endian two's-complement `i64`.
    ///
    /// # Errors
    /// [`WireError::Truncated`].
    pub fn i64(&mut self) -> Result<i64, WireError> {
        Ok(i64::from_be_bytes(self.fixed()?))
    }

    /// Read a fixed-width raw field.
    ///
    /// # Errors
    /// [`WireError::Truncated`].
    pub fn fixed<const N: usize>(&mut self) -> Result<[u8; N], WireError> {
        let mut out = [0_u8; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    /// Read `bytes(max)`.
    ///
    /// # Errors
    /// [`WireError::TooLong`] if the stated length is above `max`,
    /// [`WireError::LengthExceedsInput`] if it claims more bytes than remain.
    pub fn bytes(&mut self, max: usize) -> Result<&'a [u8], WireError> {
        let length = usize::try_from(self.u32()?).map_err(|_| WireError::TooLong)?;
        if length > max {
            return Err(WireError::TooLong);
        }
        if length > self.rest.len() {
            return Err(WireError::LengthExceedsInput);
        }
        self.take(length)
    }

    /// Read `text(max)`; `max` counts bytes.
    ///
    /// # Errors
    /// As [`Reader::bytes`], plus [`WireError::InvalidUtf8`].
    pub fn text(&mut self, max: usize) -> Result<&'a str, WireError> {
        std::str::from_utf8(self.bytes(max)?).map_err(|_| WireError::InvalidUtf8)
    }

    /// Read `opt(T)`: a presence byte of `0` or `1`, then `read` if present.
    ///
    /// # Errors
    /// [`WireError::BadPresence`] for any other presence byte, plus whatever `read` returns.
    pub fn opt<T>(
        &mut self,
        read: impl FnOnce(&mut Self) -> Result<T, WireError>,
    ) -> Result<Option<T>, WireError> {
        match self.u8()? {
            0 => Ok(None),
            1 => read(self).map(Some),
            _ => Err(WireError::BadPresence),
        }
    }

    /// Require that nothing is left.
    ///
    /// # Errors
    /// [`WireError::TrailingBytes`].
    pub fn finish(self) -> Result<(), WireError> {
        if self.rest.is_empty() {
            Ok(())
        } else {
            Err(WireError::TrailingBytes)
        }
    }
}
