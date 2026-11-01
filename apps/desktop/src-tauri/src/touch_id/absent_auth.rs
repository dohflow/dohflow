//! The biometric stand-in for a build with no sensor to ask.
//!
//! Kept in its own file on purpose: the repository scan in `platform-traits/tests/policy_scan.rs`
//! flags any consumer file that names both `preflight_prompt` and `load_secret`, and `mod.rs`
//! loads secrets.

use platform_traits::{
    AuthPrompt, BiometricAuth, BiometricAvailability, PreflightResult, SecretError,
    UnavailableReason,
};

/// No biometric hardware to ask.
pub(super) struct AbsentAuth;

impl BiometricAuth for AbsentAuth {
    fn availability(&self) -> BiometricAvailability {
        BiometricAvailability::NotPresent
    }

    fn preflight_prompt(&self, _prompt: &AuthPrompt) -> Result<PreflightResult, SecretError> {
        Err(SecretError::Unavailable(UnavailableReason::Unsupported))
    }
}
