use crate::{AccessPolicy, AuthPrompt, PublicTag, SecretError, SecretKey, SecretValue};

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
    /// This store IMPLEMENTS the biometric access control
    /// ([`AccessPolicy::DeviceOnlyBiometryCurrentSet`]). It says nothing about whether THIS device can
    /// create such an item right now (no enrolled biometrics, no sensor, locked out): that is
    /// [`crate::BiometricAuth::availability`], a runtime fact a caller asks before storing.
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
    ) -> Result<(), SecretError> {
        self.store_secret_tagged(key, value, policy, None)
    }

    /// [`SecretStore::store_secret`] with an optional [`PublicTag`] stored beside the item. A store
    /// REPLACES the tag with the new one (or removes it when `None`), so a tag can never describe a
    /// different value than the one stored with it.
    ///
    /// A biometric-gated item (`DeviceOnlyBiometryCurrentSet`) is refused with the typed error for
    /// the device's current [`crate::BiometricAvailability`] **before** anything is replaced, when the
    /// implementation can tell (a store with no biometric knowledge may leave it to the platform).
    ///
    /// # Errors
    /// As [`SecretStore::store_secret`].
    fn store_secret_tagged(
        &self,
        key: &SecretKey,
        value: &SecretValue,
        policy: AccessPolicy,
        tag: Option<&PublicTag>,
    ) -> Result<(), SecretError>;

    /// Reads the item's [`PublicTag`] **without authentication**: never shows a prompt, never
    /// releases the secret, works for a biometric-gated item. `Ok(None)` for an item stored without a
    /// tag. This is how an owner sweeps items it no longer wants without asking the user to match a
    /// finger.
    ///
    /// # Errors
    /// [`SecretError::NotFound`] when there is no item, [`SecretError::Unavailable`] or
    /// [`SecretError::Platform`].
    fn public_tag(&self, key: &SecretKey) -> Result<Option<PublicTag>, SecretError>;

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
