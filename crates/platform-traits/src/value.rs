use std::fmt;

use zeroize::Zeroizing;

use crate::SecretError;

/// The largest secret a platform store is asked to hold. Keychains are for small wrapping material
/// (ADR 0002 "Rejected alternatives": not multi-megabyte key material).
pub const MAX_SECRET_BYTES: usize = 64 * 1024;

/// Secret bytes. Zeroized on drop, **no `Display`, no `Clone`, no `Serialize`**, and a `Debug` that
/// shows only the length. Copy deliberately with [`SecretValue::duplicate`].
///
/// Residue the platform keeps (for example a framework's own copy) cannot be wiped from here; see
/// the ADR addendum.
pub struct SecretValue(Zeroizing<Vec<u8>>);

impl SecretValue {
    /// Copies `bytes` into an exactly sized, zeroizing buffer.
    ///
    /// # Errors
    /// [`SecretError::Empty`] for no bytes, [`SecretError::TooLarge`] above [`MAX_SECRET_BYTES`].
    pub fn new(bytes: &[u8]) -> Result<Self, SecretError> {
        if bytes.is_empty() {
            return Err(SecretError::Empty);
        }
        if bytes.len() > MAX_SECRET_BYTES {
            return Err(SecretError::TooLarge);
        }
        Ok(Self(Zeroizing::new(bytes.to_vec())))
    }

    /// Takes ownership of `bytes` (zeroized on drop) without copying.
    ///
    /// # Errors
    /// [`SecretError::Empty`] / [`SecretError::TooLarge`] (the buffer is still zeroized).
    pub fn from_vec(bytes: Vec<u8>) -> Result<Self, SecretError> {
        let wrapped = Zeroizing::new(bytes);
        if wrapped.is_empty() {
            return Err(SecretError::Empty);
        }
        if wrapped.len() > MAX_SECRET_BYTES {
            return Err(SecretError::TooLarge);
        }
        Ok(Self(wrapped))
    }

    #[must_use]
    pub fn expose(&self) -> &[u8] {
        &self.0
    }

    /// Always at least 1: an empty secret cannot be constructed (`SecretError::Empty`), so there is
    /// deliberately no `is_empty`.
    #[allow(clippy::len_without_is_empty)]
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// An explicit second copy (also zeroized on drop).
    #[must_use]
    pub fn duplicate(&self) -> Self {
        Self(Zeroizing::new(self.0.to_vec()))
    }
}

impl PartialEq for SecretValue {
    /// Constant-time-ish by length and content scan without short-circuit on the first difference.
    fn eq(&self, other: &Self) -> bool {
        if self.0.len() != other.0.len() {
            return false;
        }
        self.0
            .iter()
            .zip(other.0.iter())
            .fold(0_u8, |acc, (a, b)| acc | (a ^ b))
            == 0
    }
}

impl Eq for SecretValue {}

impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SecretValue(<redacted, {} bytes>)", self.0.len())
    }
}
