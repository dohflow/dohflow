use std::fmt;

use crate::SecretError;

/// The longest public tag, in bytes. Room for a 32-byte fingerprint with a version prefix.
pub const MAX_TAG_BYTES: usize = 64;

/// A small NON-SECRET label stored beside an item, readable without any authentication prompt. Its
/// purpose is to let the owner of an item decide whether the item is still wanted (for example, by
/// comparing a fingerprint with the current state of the vault) and delete it, without a biometric.
///
/// **Never put a secret in it**: it is stored in an attribute the platform does not protect with the
/// item's access control (on Apple platforms, `kSecAttrGeneric`), and any process with the item's
/// access group can read it. `Debug` shows only the length.
#[derive(Clone, PartialEq, Eq)]
pub struct PublicTag(Vec<u8>);

impl PublicTag {
    /// # Errors
    /// [`SecretError::Empty`] for no bytes, [`SecretError::TooLarge`] past [`MAX_TAG_BYTES`].
    pub fn new(bytes: &[u8]) -> Result<Self, SecretError> {
        if bytes.is_empty() {
            return Err(SecretError::Empty);
        }
        if bytes.len() > MAX_TAG_BYTES {
            return Err(SecretError::TooLarge);
        }
        Ok(Self(bytes.to_vec()))
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for PublicTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PublicTag({} bytes)", self.0.len())
    }
}
