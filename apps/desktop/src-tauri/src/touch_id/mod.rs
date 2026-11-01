//! Touch ID unlock, the app-side half (`personal-cfo-xuu`, PR 3 of 4).
//!
//! One keychain item per vault holds the released unlock material, protected by the platform's own
//! access control (`biometryCurrentSet`, device-only, never synchronized). Next to it, in the item's
//! non-secret public tag, sit the markers that decide WHEN Touch ID is offered (`tag`, `policy`): the
//! owner's rule is that the VAULT password is required after a restart, after 3 failed or cancelled
//! prompts and 30 days after the last password unlock, all of it app-side.
//!
//! **Key-bearing code lives here and never in `src/ipc`** (`tests/dek_boundary.rs` forbids the key types
//! and the word "dek" there). The IPC commands call the plain functions at the bottom of this file,
//! which return small DTO-shaped values and never a key.
//!
//! Rules this module keeps (each has a test against the mock platform):
//!
//! * The password is the only way in that this module cannot weaken. Touch ID is an OFFER; every
//!   failure, cancel, lockout or unreadable marker ends at the password field, with no error loop.
//! * `LAContext`-style authentication never gates the release: the item's own access control does, and
//!   `BiometricAuth::preflight_prompt` is never called here.
//! * The failed-prompt counter is incremented BEFORE the prompt and restored after a success, so three
//!   successful unlocks never force the password and three cancels do; only a password unlock zeroes it.
//! * A retired item (a password change or a key rotation) is deleted before and after the change and
//!   swept at launch and at every unlock.

mod absent_auth;
pub mod app;
mod policy;
mod tag;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use finance_kernel::KernelError;
use platform_traits::{
    AccessPolicy, AuthPrompt, BiometricAuth, BootClock, BootClockError, BootId, PublicTag,
    SecretError, SecretKey, SecretStore, SecretValue, UnavailableReason,
};
use vault_crypto::{SecretBytes, RELEASED_LEN};

use absent_auth::AbsentAuth;
pub use policy::{reason_for, Reason, MAX_FAILED_PROMPTS, MAX_PASSWORD_AGE_NS};
use tag::Markers;

/// The keychain service name of the item (`SecretKey` requires the `app.dohflow.` prefix).
const SERVICE: &str = "app.dohflow.touch-id-unlock";
/// The account used when the host has no vault registry id (a single-vault test host).
const DEFAULT_ACCOUNT: &str = "default";
/// The reason text the system shows with the Touch ID prompt: fixed, no vault data in it.
const PROMPT_REASON: &str = "Unlock DohFlow";
/// The fingerprint of the wrapped DEK sits at this range of the released plaintext
/// (`vault_crypto::released`: version 1 byte, vault id 16, fingerprint 32, key 32).
const FINGERPRINT: std::ops::Range<usize> = 17..49;

/// What the settings card and the unlock screen need to know.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Touch ID cannot be used here, and why (a development build, no sensor, nothing enrolled, ...).
    Unavailable(Reason),
    /// Touch ID is not enabled for this vault.
    Off,
    /// Touch ID is enabled. `offered` is whether the unlock screen shows it right now.
    On {
        /// `true` when no rule requires the vault password.
        offered: bool,
    },
}

/// The status of Touch ID for one vault.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Status {
    pub state: State,
    /// Why Touch ID is not offered, when it is not.
    pub reason: Option<Reason>,
    /// Failed or cancelled prompts since the last password unlock (0 when there is no readable item).
    pub failed_prompts: u8,
}

/// How a Touch ID unlock attempt ended.
#[derive(Debug)]
pub enum Attempt {
    /// The vault is unlocked.
    Unlocked,
    /// Not unlocked; the vault password is needed, for this reason. The caller shows the password field.
    PasswordRequired(Reason),
    /// The unlock itself failed in a way the password path would also report (a damaged vault, a
    /// persistence error): the item was kept and the counter restored.
    Failed(KernelError),
}

/// The platform pieces, injected: the mock in tests, the macOS ones in the app.
#[derive(Clone)]
pub struct TouchId {
    store: Arc<dyn SecretStore>,
    auth: Arc<dyn BiometricAuth>,
    clock: Arc<dyn BootClock>,
    /// Serializes each read-then-write of the markers (the counter and the stamp). Held for one such
    /// step only, never across a prompt, so a password unlock is not held up behind a Touch ID sheet.
    markers: Arc<Mutex<()>>,
    /// Set while a Touch ID attempt runs (from the counter increment to its outcome). A second attempt
    /// is refused with no prompt and no count.
    attempt_running: Arc<AtomicBool>,
}

/// Clears `attempt_running` when the attempt ends, however it ends.
struct AttemptGuard(Arc<AtomicBool>);

impl Drop for AttemptGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl TouchId {
    #[must_use]
    pub fn new(
        store: Arc<dyn SecretStore>,
        auth: Arc<dyn BiometricAuth>,
        clock: Arc<dyn BootClock>,
    ) -> Self {
        Self {
            store,
            auth,
            clock,
            markers: Arc::new(Mutex::new(())),
            attempt_running: Arc::new(AtomicBool::new(false)),
        }
    }

    /// No Touch ID on this build: every call is the typed `Unavailable(reason)` and the app stays
    /// password-only. Used where there is no platform implementation, and for a development build that
    /// carries no keychain entitlement.
    #[must_use]
    pub fn absent(reason: UnavailableReason) -> Self {
        Self::new(
            Arc::new(AbsentStore(reason)),
            Arc::new(AbsentAuth),
            Arc::new(AbsentClock),
        )
    }

    /// The platform implementation for this build. On macOS the keychain access group is read from the
    /// app's own entitlement; a build that does not carry `<TeamID>.<bundle_id>` is the typed
    /// `MissingEntitlement` path (password-only, and the settings card says a signed build is needed).
    #[must_use]
    pub fn production(bundle_id: &str) -> Self {
        #[cfg(target_os = "macos")]
        {
            let store: Arc<dyn SecretStore> = match platform_macos::own_access_group(bundle_id) {
                Some(group) => Arc::new(platform_macos::MacSecretStore::new(group)),
                None => Arc::new(AbsentStore(UnavailableReason::MissingEntitlement)),
            };
            Self::new(
                store,
                Arc::new(platform_macos::MacBiometricAuth::new()),
                Arc::new(platform_macos::MacBootClock::new()),
            )
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = bundle_id;
            Self::absent(UnavailableReason::Unsupported)
        }
    }

    fn serialized(&self) -> MutexGuard<'_, ()> {
        self.markers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn key(account: &str) -> Option<SecretKey> {
        SecretKey::new(SERVICE, account).ok()
    }

    /// Reads the markers without any prompt. `Ok(None)`: there is no item. `Err`: a reason the markers
    /// cannot be used (the caller fails closed).
    fn read_markers(&self, key: &SecretKey) -> Result<Option<Markers>, Reason> {
        match self.store.public_tag(key) {
            Ok(Some(tag)) => Markers::decode(&tag)
                .map(Some)
                .ok_or(Reason::MarkersUnreadable),
            Ok(None) => Err(Reason::MarkersUnreadable),
            Err(SecretError::NotFound) => Ok(None),
            Err(SecretError::Unavailable(UnavailableReason::MissingEntitlement)) => {
                Err(Reason::SignedBuildNeeded)
            }
            Err(SecretError::Unavailable(_)) => Err(Reason::StoreUnavailable),
            Err(_) => Err(Reason::MarkersUnreadable),
        }
    }

    /// The status for the vault `account`, whose envelope currently has `fingerprint` (`None` when it
    /// cannot be read: the item is then not offered).
    #[must_use]
    pub fn status(&self, account: &str, fingerprint: Option<[u8; 32]>) -> Status {
        let unavailable = |reason| Status {
            state: State::Unavailable(reason),
            reason: Some(reason),
            failed_prompts: 0,
        };
        let Some(key) = Self::key(account) else {
            return unavailable(Reason::MarkersUnreadable);
        };
        let availability = self.auth.availability();
        match self.read_markers(&key) {
            Err(Reason::SignedBuildNeeded) => unavailable(Reason::SignedBuildNeeded),
            Err(Reason::StoreUnavailable) => unavailable(Reason::StoreUnavailable),
            Ok(None) => match reason_for(availability) {
                Some(reason) => unavailable(reason),
                None => Status {
                    state: State::Off,
                    reason: Some(Reason::NotEnabled),
                    failed_prompts: 0,
                },
            },
            Err(reason) => Status {
                state: State::On { offered: false },
                reason: Some(reason),
                failed_prompts: 0,
            },
            Ok(Some(markers)) => {
                let verdict = if fingerprint == Some(markers.fingerprint) {
                    policy::eligibility(
                        &markers,
                        self.clock.boot_id(),
                        self.clock.monotonic_ns(),
                        availability,
                    )
                } else {
                    Err(Reason::Retired)
                };
                Status {
                    state: State::On {
                        offered: verdict.is_ok(),
                    },
                    reason: verdict.err(),
                    failed_prompts: markers.failed_prompts,
                }
            }
        }
    }

    /// Stores the unlock material for `account` and starts the markers (AC C, G, J). The caller has
    /// already verified the vault password. Biometric availability is checked FIRST (AC A); a failure
    /// leaves no item, so the vault stays password-only.
    ///
    /// # Errors
    /// The [`Reason`] Touch ID could not be enabled for.
    pub fn install(
        &self,
        account: &str,
        material: &SecretBytes<RELEASED_LEN>,
    ) -> Result<(), Reason> {
        let key = Self::key(account).ok_or(Reason::MarkersUnreadable)?;
        let _step = self.serialized();
        let outcome = self.try_install(&key, material);
        if outcome.is_err() {
            // A failed enable is password-only, whatever the cause: an older item is removed too (a
            // refused store keeps the item it would have replaced, and an item that no longer matches
            // what the user just did must not linger).
            let _ = self.store.delete_secret(&key);
        }
        outcome
    }

    fn try_install(
        &self,
        key: &SecretKey,
        material: &SecretBytes<RELEASED_LEN>,
    ) -> Result<(), Reason> {
        if let Some(reason) = reason_for(self.auth.availability()) {
            return Err(reason);
        }
        let boot = self
            .clock
            .boot_id()
            .map_err(|_| Reason::MarkersUnreadable)?;
        let now = self
            .clock
            .monotonic_ns()
            .map_err(|_| Reason::MarkersUnreadable)?;
        let mut fingerprint = [0_u8; 32];
        fingerprint.copy_from_slice(&material.expose()[FINGERPRINT]);
        let markers = Markers {
            fingerprint,
            boot,
            password_stamp_ns: now,
            failed_prompts: 0,
        };
        let value = SecretValue::new(material.expose()).map_err(|_| Reason::StoreUnavailable)?;
        self.store
            .store_secret_tagged(
                key,
                &value,
                AccessPolicy::DeviceOnlyBiometryCurrentSet,
                Some(&markers.encode()),
            )
            .map_err(|error| reason_for_store_error(&error))
    }

    /// Turns Touch ID off: the item (and its markers) is deleted.
    ///
    /// # Errors
    /// The store's error when the item cannot be removed.
    pub fn disable(&self, account: &str) -> Result<(), SecretError> {
        let key = Self::key(account).ok_or(SecretError::InvalidKey)?;
        let _step = self.serialized();
        self.store.delete_secret(&key)
    }

    /// Whether an item can be removed for a change of the vault's key material. A store that is
    /// unavailable (no entitlement, no keychain) holds no item, so there is nothing to remove.
    ///
    /// # Errors
    /// The store's error when an item exists and cannot be removed.
    pub fn retire(&self, account: &str) -> Result<(), SecretError> {
        match self.disable(account) {
            Ok(()) | Err(SecretError::Unavailable(_)) => Ok(()),
            Err(error) => Err(error),
        }
    }

    /// After a successful vault PASSWORD unlock: re-stamp the markers (this boot, now, no failed
    /// prompts), or delete the item when it no longer matches the envelope or the clock cannot be read.
    pub fn note_password_unlock(&self, account: &str, fingerprint: Option<[u8; 32]>) {
        let Some(key) = Self::key(account) else {
            return;
        };
        let _step = self.serialized();
        let Ok(Some(markers)) = self.read_markers(&key) else {
            // No item, an unusable store, or unreadable markers: unreadable ones are swept below.
            self.sweep_key(&key, fingerprint);
            return;
        };
        if fingerprint != Some(markers.fingerprint) {
            let _ = self.store.delete_secret(&key);
            return;
        }
        let stamped = match (self.clock.boot_id(), self.clock.monotonic_ns()) {
            (Ok(boot), Ok(now)) => Some(Markers {
                boot,
                password_stamp_ns: now,
                failed_prompts: 0,
                ..markers
            }),
            _ => None,
        };
        let updated = stamped.and_then(|markers| {
            self.store
                .update_public_tag(&key, Some(&markers.encode()))
                .ok()
        });
        if updated.is_none() {
            // Cannot re-stamp: the old stamp would be wrong in the user's favor or against them, and
            // either way the markers cannot be trusted. Password-only until Touch ID is enabled again.
            let _ = self.store.delete_secret(&key);
        }
    }

    /// Deletes the item when its markers cannot be read or its fingerprint no longer matches the
    /// vault's envelope. Needs no prompt and no unlock. Run at launch and after every unlock.
    pub fn sweep(&self, account: &str, fingerprint: Option<[u8; 32]>) {
        let Some(key) = Self::key(account) else {
            return;
        };
        let _step = self.serialized();
        self.sweep_key(&key, fingerprint);
    }

    /// `sweep` for a caller that already holds the markers step.
    fn sweep_key(&self, key: &SecretKey, fingerprint: Option<[u8; 32]>) {
        let stale = match self.store.public_tag(key) {
            Ok(Some(tag)) => match Markers::decode(&tag) {
                Some(markers) => fingerprint.is_some_and(|current| current != markers.fingerprint),
                None => true,
            },
            Ok(None) => true,
            Err(_) => false,
        };
        if stale {
            let _ = self.store.delete_secret(key);
        }
    }

    /// One Touch ID unlock attempt. `unlock_with` performs the controller step with the released
    /// material and returns the kernel's verdict.
    pub fn unlock(
        &self,
        account: &str,
        fingerprint: Option<[u8; 32]>,
        unlock_with: impl FnOnce(SecretBytes<RELEASED_LEN>) -> Result<(), KernelError>,
    ) -> Attempt {
        if self.attempt_running.swap(true, Ordering::SeqCst) {
            // Another attempt is running: refuse this one, with no prompt and no count.
            return Attempt::PasswordRequired(Reason::AttemptInProgress);
        }
        let _attempt = AttemptGuard(Arc::clone(&self.attempt_running));
        let Some(key) = Self::key(account) else {
            return Attempt::PasswordRequired(Reason::MarkersUnreadable);
        };

        // The check, the read and the increment are one step: nothing else reads the markers between
        // them and writes them back.
        let (before, counted) = {
            let _step = self.serialized();
            let status = self.status(account, fingerprint);
            match status.state {
                State::On { offered: true } => {}
                State::On { offered: false } => {
                    return Attempt::PasswordRequired(
                        status.reason.unwrap_or(Reason::MarkersUnreadable),
                    )
                }
                State::Off => return Attempt::PasswordRequired(Reason::NotEnabled),
                State::Unavailable(reason) => return Attempt::PasswordRequired(reason),
            }
            let Ok(Some(before)) = self.read_markers(&key) else {
                return Attempt::PasswordRequired(Reason::MarkersUnreadable);
            };
            // The counter goes up BEFORE the prompt, so a prompt that never returns (a crash, a killed
            // app) still counts. If it cannot be written the password is required and nothing is shown.
            let counted = before.with_failed_prompts(before.failed_prompts.saturating_add(1));
            if self
                .store
                .update_public_tag(&key, Some(&counted.encode()))
                .is_err()
            {
                return Attempt::PasswordRequired(Reason::MarkersUnreadable);
            }
            (before, counted)
        };

        let Ok(prompt) = AuthPrompt::new(PROMPT_REASON) else {
            self.restore(&key, &before, &counted);
            return Attempt::PasswordRequired(Reason::StoreUnavailable);
        };
        let value = match self.store.load_secret(&key, &prompt) {
            Ok(value) => value,
            // A cancelled prompt counts like a failed one (the increment above stays): the third one in
            // a row, with no password unlock between, requires the password.
            Err(SecretError::AuthCancelled) => {
                return Attempt::PasswordRequired(failed_prompt_reason(counted.failed_prompts))
            }
            Err(error) => return self.after_load_error(&key, &before, &counted, error),
        };
        let Some(released) = released_from(&value) else {
            // Not 81 bytes: an item this build cannot read. Retire it.
            let _step = self.serialized();
            let _ = self.store.delete_secret(&key);
            return Attempt::PasswordRequired(Reason::Retired);
        };
        drop(value);

        match unlock_with(released) {
            Ok(()) => {
                // A successful unlock is not a failed prompt: put the counter back as it was.
                self.restore(&key, &before, &counted);
                Attempt::Unlocked
            }
            Err(KernelError::ReleasedUnlock(cause)) => {
                self.restore(&key, &before, &counted);
                if cause.requires_reenrollment() {
                    // The item no longer matches the vault, or cannot be read: retire it.
                    let _step = self.serialized();
                    let _ = self.store.delete_secret(&key);
                    Attempt::PasswordRequired(Reason::Retired)
                } else {
                    // The item matched the envelope: keep it (it may be the user's only practical
                    // copy), ask for the password once, and never say the key is wrong.
                    Attempt::PasswordRequired(Reason::UnlockUnavailable)
                }
            }
            Err(other) => {
                // The password path would report the same (a damaged vault, a persistence error):
                // keep the item, restore the counter, surface the error.
                self.restore(&key, &before, &counted);
                Attempt::Failed(other)
            }
        }
    }

    /// Puts the counter back to `before`, but only if the markers are still the ones this attempt
    /// wrote (`counted`). If a password unlock re-stamped them meanwhile, or the item is gone, that
    /// newer state stands: a late restore never overwrites it.
    fn restore(&self, key: &SecretKey, before: &Markers, counted: &Markers) {
        let _step = self.serialized();
        let unchanged = matches!(
            self.store.public_tag(key),
            Ok(Some(tag)) if Markers::decode(&tag) == Some(*counted)
        );
        if unchanged {
            // Best effort: if this fails the counter stays one higher, which only moves toward the
            // password.
            let _ = self.store.update_public_tag(key, Some(&before.encode()));
        }
    }

    /// What a failed `load_secret` means (AC I; 03b's M3).
    fn after_load_error(
        &self,
        key: &SecretKey,
        before: &Markers,
        counted: &Markers,
        error: SecretError,
    ) -> Attempt {
        match error {
            // A wrong finger. Keep the item (anyone at the keyboard could otherwise turn Touch ID off by
            // failing it) and keep the incremented counter: it is a failed prompt.
            SecretError::AuthFailed => {
                Attempt::PasswordRequired(failed_prompt_reason(counted.failed_prompts))
            }
            // Too many failed matches: macOS wants its own passcode. The password is required at once.
            SecretError::BiometryLockedOut => {
                let locked = before.with_failed_prompts(MAX_FAILED_PROMPTS);
                let _step = self.serialized();
                let _ = self.store.update_public_tag(key, Some(&locked.encode()));
                Attempt::PasswordRequired(Reason::BiometryLockedOut)
            }
            // The enrolled set changed (or was removed): the item is invalidated by the platform. Delete
            // it (a no-op if it is already gone) and offer enrollment again after the password.
            SecretError::NotFound => {
                let _step = self.serialized();
                let _ = self.store.delete_secret(key);
                Attempt::PasswordRequired(Reason::Retired)
            }
            SecretError::BiometryNotEnrolled => {
                self.restore(key, before, counted);
                Attempt::PasswordRequired(Reason::BiometryNotEnrolled)
            }
            SecretError::Unavailable(UnavailableReason::MissingEntitlement) => {
                self.restore(key, before, counted);
                Attempt::PasswordRequired(Reason::SignedBuildNeeded)
            }
            // Locked, no keychain, a platform error: not the user's finger. Restore and ask for the
            // password.
            _ => {
                self.restore(key, before, counted);
                Attempt::PasswordRequired(Reason::StoreUnavailable)
            }
        }
    }
}

/// The reason after a failed or cancelled prompt: the third one is the rule, the first two just fall back.
fn failed_prompt_reason(failed_prompts: u8) -> Reason {
    if failed_prompts >= MAX_FAILED_PROMPTS {
        Reason::TooManyFailedPrompts
    } else {
        // Below the limit Touch ID is still offered next time; the reason is only "this prompt did not
        // unlock", which the unlock screen shows as a plain "use your password".
        Reason::PromptNotCompleted
    }
}

fn reason_for_store_error(error: &SecretError) -> Reason {
    match error {
        SecretError::BiometryNotEnrolled => Reason::BiometryNotEnrolled,
        SecretError::BiometryLockedOut => Reason::BiometryLockedOut,
        SecretError::Unavailable(UnavailableReason::MissingEntitlement) => {
            Reason::SignedBuildNeeded
        }
        SecretError::Unavailable(UnavailableReason::Unsupported) => Reason::BiometryNotPresent,
        _ => Reason::StoreUnavailable,
    }
}

/// The released plaintext from a loaded secret, built in place in the protected buffer. `None` unless it
/// is exactly [`RELEASED_LEN`] bytes.
fn released_from(value: &SecretValue) -> Option<SecretBytes<RELEASED_LEN>> {
    let bytes = value.expose();
    if bytes.len() != RELEASED_LEN {
        return None;
    }
    SecretBytes::init::<std::convert::Infallible>(|out| {
        out.copy_from_slice(bytes);
        Ok(())
    })
    .ok()
}

// ---- a build with no Touch ID --------------------------------------------------------------------

/// A store that answers every call with `Unavailable(reason)`.
struct AbsentStore(UnavailableReason);

impl SecretStore for AbsentStore {
    fn store_secret_tagged(
        &self,
        _key: &SecretKey,
        _value: &SecretValue,
        _policy: AccessPolicy,
        _tag: Option<&PublicTag>,
    ) -> Result<(), SecretError> {
        Err(SecretError::Unavailable(self.0))
    }

    fn load_secret(
        &self,
        _key: &SecretKey,
        _prompt: &AuthPrompt,
    ) -> Result<SecretValue, SecretError> {
        Err(SecretError::Unavailable(self.0))
    }

    fn update_public_tag(
        &self,
        _key: &SecretKey,
        _tag: Option<&PublicTag>,
    ) -> Result<(), SecretError> {
        Err(SecretError::Unavailable(self.0))
    }

    fn delete_secret(&self, _key: &SecretKey) -> Result<(), SecretError> {
        Err(SecretError::Unavailable(self.0))
    }

    fn public_tag(&self, _key: &SecretKey) -> Result<Option<PublicTag>, SecretError> {
        Err(SecretError::Unavailable(self.0))
    }

    fn capabilities(&self) -> platform_traits::StoreCapabilities {
        platform_traits::StoreCapabilities {
            device_only_class: true,
            biometry_current_set: false,
            roams_to_other_device: platform_traits::Roaming::Never,
            rollback_resistant: false,
        }
    }
}

/// A clock that cannot answer (the rules then fail closed).
struct AbsentClock;

impl BootClock for AbsentClock {
    fn boot_id(&self) -> Result<BootId, BootClockError> {
        Err(BootClockError)
    }

    fn monotonic_ns(&self) -> Result<u64, BootClockError> {
        Err(BootClockError)
    }
}

/// The account name used for a vault with no registry id.
#[must_use]
pub const fn default_account() -> &'static str {
    DEFAULT_ACCOUNT
}
