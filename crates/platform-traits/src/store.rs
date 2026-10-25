use crate::{AccessPolicy, AuthPrompt, SecretError, SecretKey, SecretValue};

/// Whether items can leave this device in a backup or by sync. Reported per implementation; a
/// store that cannot prove "never" reports `Unknown`, not `Never`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Roaming {
    Never,
    May,
    Unknown,
}

/// What a store can honestly promise. Two separate facts on purpose (ADR 0002 addendum): moving to
/// another device and being rolled back on this one are different failures, and a secret store only
/// ever addresses the first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreCapabilities {
    /// Items get the device-only class ([`AccessPolicy::DeviceOnly`]).
    pub device_only_class: bool,
    /// The platform's own access control can require a current-set biometric
    /// ([`AccessPolicy::DeviceOnlyBiometryCurrentSet`]).
    pub biometry_current_set: bool,
    /// Whether an item can move to another device.
    pub roams_to_other_device: Roaming,
    /// Whether the store can detect that this device was rolled back to an older state. **Always
    /// `false` for a keychain**: it is never the anti-rollback witness (ADR 0017).
    pub rollback_resistant: bool,
}

/// A device-local secret store (plan §3.3.1). Implementations must pass
/// [`crate::conformance::run`].
pub trait SecretStore: Send + Sync {
    /// Stores `value` under `key`, replacing any existing item.
    ///
    /// A store that is **refused** (an unsupported policy, a protection that cannot be created) fails
    /// before anything is replaced: the previous secret is untouched. A store that fails **after**
    /// the replacement has begun (the platform rejects the final write) MAY leave no item at all, so a
    /// caller that cannot tolerate losing the old secret keeps its own copy until the call returns
    /// `Ok`.
    ///
    /// # Errors
    /// [`SecretError::Unavailable`] when the requested protection cannot be provided (never a
    /// weaker store), [`SecretError::TooLarge`], or [`SecretError::Platform`].
    fn store_secret(
        &self,
        key: &SecretKey,
        value: &SecretValue,
        policy: AccessPolicy,
    ) -> Result<(), SecretError>;

    /// Loads the secret. `prompt` is the text shown if the platform asks the user to authenticate
    /// (ignored for items that need no authentication), so a call site never branches on policy.
    ///
    /// # Errors
    /// [`SecretError::NotFound`] (also what an item invalidated by a biometric change may look
    /// like), [`SecretError::AuthCancelled`] / [`SecretError::AuthFailed`], or
    /// [`SecretError::Unavailable`].
    fn load_secret(&self, key: &SecretKey, prompt: &AuthPrompt)
        -> Result<SecretValue, SecretError>;

    /// Deletes the secret. **Idempotent**: deleting what is not there succeeds.
    ///
    /// # Errors
    /// [`SecretError::Unavailable`] or [`SecretError::Platform`].
    fn delete_secret(&self, key: &SecretKey) -> Result<(), SecretError>;

    fn capabilities(&self) -> StoreCapabilities;
}
