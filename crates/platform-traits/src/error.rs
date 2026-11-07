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

/// The step of a store that the platform refused. A fixed vocabulary, so a log line or a test failure can
/// say where a store stopped without carrying anything the platform said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum StoreStep {
    /// Creating the access control the item is protected by (biometry, device-only).
    AccessControl,
    /// Removing the item a replacement store would supersede.
    Delete,
    /// Adding the item.
    Add,
}

impl StoreStep {
    /// The fixed label for logs: `access_control`, `delete` or `add`.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::AccessControl => "access_control",
            Self::Delete => "delete",
            Self::Add => "add",
        }
    }
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
    /// A store stopped at `step` with the platform's numeric status `code` (an `OSStatus` on Apple
    /// platforms, `0` when the platform refused without giving one). Never a message. It is returned
    /// for every refusal at a store step except the ones with a typed meaning of their own (a missing
    /// entitlement), so a caller can name the step and the code and not guess a cause.
    #[error("the platform secret store refused the {} step (status {code})", .step.label())]
    StoreFailed { step: StoreStep, code: i64 },
}
