use crate::{AuthPrompt, SecretError};

/// Whether a biometric can be used on this device RIGHT NOW. A runtime fact, not a property of the
/// store: enrolling a fingerprint later, removing every fingerprint, or too many failed matches
/// each change it without a restart, so callers ask each time and never cache it (xuu, AC A).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BiometricAvailability {
    /// Hardware is present, a biometric is enrolled and the sensor is usable.
    Available,
    /// Hardware is present and nothing is enrolled.
    NotEnrolled,
    /// This device has no biometric hardware (or the platform says it is not available here).
    NotPresent,
    /// Too many failed matches: the platform wants the device passcode before biometrics work.
    LockedOut,
    /// The platform answered with something this build does not map. Treated as "not available" for
    /// the user, but a store call is still allowed to try and report its own typed error.
    Unknown,
}

impl BiometricAvailability {
    /// The typed error a store must return when asked to create a biometric-gated item while the
    /// device cannot, **before** it replaces anything (a refusal never destroys the old secret).
    /// `None` for `Available` and for `Unknown` (the platform decides and reports its own error).
    #[must_use]
    pub fn store_refusal(self) -> Option<SecretError> {
        match self {
            Self::Available | Self::Unknown => None,
            Self::NotEnrolled => Some(SecretError::BiometryNotEnrolled),
            Self::LockedOut => Some(SecretError::BiometryLockedOut),
            Self::NotPresent => Some(SecretError::Unavailable(
                crate::UnavailableReason::Unsupported,
            )),
        }
    }
}

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
    /// Whether a biometric can be used right now, and if not, why. Asked each time; never cached.
    fn availability(&self) -> BiometricAvailability;

    /// Whether a biometric can be used right now (hardware present and enrolled).
    fn is_available(&self) -> bool {
        self.availability() == BiometricAvailability::Available
    }

    /// Optionally asks the user to authenticate before a flow that will later use the store, so the
    /// prompt appears at a sensible moment. Its result gates **nothing** in the store.
    ///
    /// # Errors
    /// [`SecretError::BiometryNotEnrolled`], [`SecretError::BiometryLockedOut`],
    /// [`SecretError::AuthCancelled`], [`SecretError::AuthFailed`] or [`SecretError::Unavailable`].
    fn preflight_prompt(&self, prompt: &AuthPrompt) -> Result<PreflightResult, SecretError>;
}
