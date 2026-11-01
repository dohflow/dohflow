use thiserror::Error;

/// Why a store cannot be used at all right now. Fixed vocabulary, no free text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum UnavailableReason {
    /// The app lacks the entitlement the platform requires for this storage (macOS data-protection
    /// keychain without the keychain entitlements and their provisioning profile).
    MissingEntitlement,
    /// The device has no passcode set, and the requested class needs one.
    NoPasscode,
    /// No secret-service daemon is reachable (Linux, headless or minimal systems).
    NoSecretService,
    /// The store is locked or the app is in the background and may not interact (for example
    /// `errSecInteractionNotAllowed`).
    Locked,
    /// This platform has no implementation of the requested protection.
    Unsupported,
}

/// A store or biometric failure. **Every `Display` and `Debug` is a fixed string**: no variant can
/// carry a secret value, and `Platform` carries only the platform's numeric status.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum SecretError {
    #[error("no secret is stored under that key")]
    NotFound,
    #[error("the secret key is not valid")]
    InvalidKey,
    /// The authentication prompt text is empty, too long or contains a control character.
    #[error("the authentication prompt text is not valid")]
    InvalidPrompt,
    #[error("the secret is too large for the platform store")]
    TooLarge,
    #[error("an empty secret cannot be stored")]
    Empty,
    #[error("the user cancelled the authentication prompt")]
    AuthCancelled,
    #[error("authentication failed")]
    AuthFailed,
    #[error("no biometric is enrolled on this device")]
    BiometryNotEnrolled,
    #[error("biometric authentication is locked out")]
    BiometryLockedOut,
    #[error("the secret store is not available: {0:?}")]
    Unavailable(UnavailableReason),
    /// A platform status code (an `OSStatus` on Apple platforms) that has no more specific meaning
    /// here. Never a message: platform messages can echo what was passed in.
    #[error("the platform secret store failed (status {code})")]
    Platform { code: i64 },
}
