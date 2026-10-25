use crate::{AuthPrompt, SecretError};

/// The outcome of a successful pre-flight prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreflightResult {
    Matched,
}

/// Biometric availability and user experience. **It never releases a secret.**
///
/// A standalone biometric match is a screen gate, not cryptography: code that did "match, then call
/// `load_secret` for a `DeviceOnly` item" would protect nothing. A secret that needs a biometric uses
/// [`crate::AccessPolicy::DeviceOnlyBiometryCurrentSet`], and the platform's own access control
/// releases it. No [`crate::SecretStore`] call may be conditioned on the result of
/// [`BiometricAuth::preflight_prompt`] (a repository test scans for that).
pub trait BiometricAuth: Send + Sync {
    /// Whether a biometric can be used right now (hardware present and enrolled).
    fn is_available(&self) -> bool;

    /// Optionally asks the user to authenticate before a flow that will later use the store, so the
    /// prompt appears at a sensible moment. Its result gates **nothing** in the store.
    ///
    /// # Errors
    /// [`SecretError::BiometryNotEnrolled`], [`SecretError::BiometryLockedOut`],
    /// [`SecretError::AuthCancelled`], [`SecretError::AuthFailed`] or [`SecretError::Unavailable`].
    fn preflight_prompt(&self, prompt: &AuthPrompt) -> Result<PreflightResult, SecretError>;
}
