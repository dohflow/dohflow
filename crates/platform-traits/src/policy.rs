use crate::SecretError;

/// What protection a stored secret gets. Both variants are **device-only** (never synchronized,
/// never restored to another device: ADR 0074 Decision 8, F3) and fail with
/// [`SecretError::Unavailable`] rather than weaken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AccessPolicy {
    /// Device-local and available only while the device has a passcode set. On Apple platforms:
    /// `kSecAttrAccessibleWhenPasscodeSetThisDeviceOnly` and `kSecAttrSynchronizable = false`.
    DeviceOnly,
    /// `DeviceOnly`, and the platform itself releases the secret only after a fresh biometric match
    /// and invalidates it when the enrolled biometrics change (`SecAccessControl`
    /// `.biometryCurrentSet`). **No passcode fallback**; the vault password is the only other way
    /// in, and that is the unlock path itself, not a store path.
    DeviceOnlyBiometryCurrentSet,
}

/// The longest prompt text, in characters.
pub const MAX_PROMPT_CHARS: usize = 120;

/// The reason text the operating system shows with an authentication prompt. Never contains a
/// secret; bounded and free of control characters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthPrompt(String);

impl AuthPrompt {
    /// # Errors
    /// [`SecretError::InvalidKey`] when `reason` is empty, longer than [`MAX_PROMPT_CHARS`], or
    /// contains a control character.
    pub fn new(reason: &str) -> Result<Self, SecretError> {
        if reason.trim().is_empty()
            || reason.chars().count() > MAX_PROMPT_CHARS
            || reason.chars().any(char::is_control)
        {
            return Err(SecretError::InvalidKey);
        }
        Ok(Self(reason.to_owned()))
    }

    #[must_use]
    pub fn reason(&self) -> &str {
        &self.0
    }
}
