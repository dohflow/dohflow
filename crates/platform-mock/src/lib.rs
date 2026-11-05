//! In-memory platform implementations for tests (`personal-cfo-1zb`). Never linked into the app:
//! it keeps secrets in ordinary memory.
//!
//! The mock store's biometric behavior ([`BiometryScript`]) stands for the platform's OWN access
//! control, and is deliberately separate from [`MockBiometricAuth`] (the pre-flight prompt): a test
//! can show that a matched pre-flight prompt changes nothing about what the store releases.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use platform_traits::{
    AccessPolicy, AuthPrompt, BiometricAuth, BiometricAvailability, BootClock, BootClockError,
    BootId, CounterContext, PreflightResult, Progress, PublicTag, Roaming, RollbackWitness,
    SecretError, SecretKey, SecretStore, SecretValue, StoreCapabilities, UnavailableReason,
    VerifiedMark, WitnessError,
};

/// The simulated device's biometric state, SHARED by a [`MockSecretStore`] and a [`MockBiometricAuth`]
/// so a test changes it once and both agree, as one real device does (enrolling a fingerprint, or the
/// sensor locking out, changes what the store can create and what availability reports).
#[derive(Clone)]
pub struct BiometryDevice(Arc<Mutex<BiometricAvailability>>);

impl BiometryDevice {
    #[must_use]
    pub fn new(state: BiometricAvailability) -> Self {
        Self(Arc::new(Mutex::new(state)))
    }

    /// A device with an enrolled, usable biometric.
    #[must_use]
    pub fn available() -> Self {
        Self::new(BiometricAvailability::Available)
    }

    pub fn set(&self, state: BiometricAvailability) {
        *self.0.lock().unwrap() = state;
    }

    #[must_use]
    pub fn get(&self) -> BiometricAvailability {
        *self.0.lock().unwrap()
    }
}

/// What the (simulated) platform access control does when a biometric-gated item is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BiometryScript {
    /// The user matches.
    Match,
    /// The user cancels the prompt.
    Cancel,
    /// The match fails.
    Fail,
    /// No biometric is enrolled.
    NotEnrolled,
    /// The enrolled set changed since the item was stored: the item is invalidated and reads as
    /// not found (the platform need not give a distinct code; ADR 0002 addendum, decision 6).
    EnrollmentChanged,
}

struct Item {
    value: SecretValue,
    policy: AccessPolicy,
    tag: Option<PublicTag>,
}

/// An in-memory [`SecretStore`].
pub struct MockSecretStore {
    items: Mutex<HashMap<SecretKey, Item>>,
    script: Mutex<BiometryScript>,
    biometry_supported: bool,
    unavailable: Option<UnavailableReason>,
    device: BiometryDevice,
}

impl MockSecretStore {
    /// A store that supports both policies; biometric reads `Match` until scripted otherwise.
    #[must_use]
    pub fn new() -> Self {
        Self::on_device(BiometryDevice::available())
    }

    /// A store on a device whose biometric state the test controls (and shares with a
    /// [`MockBiometricAuth::on`]).
    #[must_use]
    pub fn on_device(device: BiometryDevice) -> Self {
        Self {
            items: Mutex::new(HashMap::new()),
            script: Mutex::new(BiometryScript::Match),
            biometry_supported: true,
            unavailable: None,
            device,
        }
    }

    /// A store with no biometric access control (storing that policy is refused as `Unsupported`).
    #[must_use]
    pub fn without_biometry() -> Self {
        Self {
            biometry_supported: false,
            ..Self::new()
        }
    }

    /// A store that fails every call with `reason`, as an unentitled macOS build or a Linux machine
    /// with no Secret Service does: for testing that callers keep the app password-only.
    #[must_use]
    pub fn unavailable(reason: UnavailableReason) -> Self {
        Self {
            unavailable: Some(reason),
            ..Self::new()
        }
    }

    pub fn script_biometry(&self, script: BiometryScript) {
        *self.script.lock().unwrap() = script;
    }

    /// How many items are stored (for assertions that nothing was written).
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.lock().unwrap().len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn check_available(&self) -> Result<(), SecretError> {
        match self.unavailable {
            Some(reason) => Err(SecretError::Unavailable(reason)),
            None => Ok(()),
        }
    }
}

impl Default for MockSecretStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretStore for MockSecretStore {
    fn store_secret_tagged(
        &self,
        key: &SecretKey,
        value: &SecretValue,
        policy: AccessPolicy,
        tag: Option<&PublicTag>,
    ) -> Result<(), SecretError> {
        self.check_available()?;
        if policy == AccessPolicy::DeviceOnlyBiometryCurrentSet {
            if !self.biometry_supported {
                return Err(SecretError::Unavailable(UnavailableReason::Unsupported));
            }
            // Refused for the device's CURRENT biometric state, before anything is replaced.
            if let Some(refusal) = self.device.get().store_refusal() {
                return Err(refusal);
            }
        }
        self.items.lock().unwrap().insert(
            key.clone(),
            Item {
                value: value.duplicate(),
                policy,
                tag: tag.cloned(),
            },
        );
        Ok(())
    }

    fn update_public_tag(
        &self,
        key: &SecretKey,
        tag: Option<&PublicTag>,
    ) -> Result<(), SecretError> {
        self.check_available()?;
        match self.items.lock().unwrap().get_mut(key) {
            Some(item) => {
                item.tag = tag.cloned();
                Ok(())
            }
            None => Err(SecretError::NotFound),
        }
    }

    fn public_tag(&self, key: &SecretKey) -> Result<Option<PublicTag>, SecretError> {
        self.check_available()?;
        self.items
            .lock()
            .unwrap()
            .get(key)
            .map(|item| item.tag.clone())
            .ok_or(SecretError::NotFound)
    }

    fn load_secret(
        &self,
        key: &SecretKey,
        _prompt: &AuthPrompt,
    ) -> Result<SecretValue, SecretError> {
        self.check_available()?;
        let mut items = self.items.lock().unwrap();
        let Some(item) = items.get(key) else {
            return Err(SecretError::NotFound);
        };
        if item.policy == AccessPolicy::DeviceOnlyBiometryCurrentSet {
            // The platform's own access control decides: independent of any pre-flight prompt.
            match *self.script.lock().unwrap() {
                BiometryScript::Match => {}
                BiometryScript::Cancel => return Err(SecretError::AuthCancelled),
                BiometryScript::Fail => return Err(SecretError::AuthFailed),
                BiometryScript::NotEnrolled => return Err(SecretError::BiometryNotEnrolled),
                BiometryScript::EnrollmentChanged => {
                    items.remove(key);
                    return Err(SecretError::NotFound);
                }
            }
        }
        Ok(items[key].value.duplicate())
    }

    fn delete_secret(&self, key: &SecretKey) -> Result<(), SecretError> {
        self.check_available()?;
        self.items.lock().unwrap().remove(key);
        Ok(())
    }

    fn capabilities(&self) -> StoreCapabilities {
        StoreCapabilities {
            device_only_class: true,
            biometry_current_set: self.biometry_supported,
            roams_to_other_device: Roaming::Never,
            rollback_resistant: false,
        }
    }
}

/// A scriptable pre-flight prompt. Its answer never reaches [`MockSecretStore`].
pub struct MockBiometricAuth {
    device: BiometryDevice,
    outcome: Mutex<Result<PreflightResult, SecretError>>,
    prompts: Mutex<u32>,
}

impl MockBiometricAuth {
    #[must_use]
    pub fn matching() -> Self {
        Self::on(BiometryDevice::available())
    }

    /// Prompt behavior on a device whose state the test controls (shared with a
    /// [`MockSecretStore::on_device`]).
    #[must_use]
    pub fn on(device: BiometryDevice) -> Self {
        Self {
            device,
            outcome: Mutex::new(Ok(PreflightResult::Matched)),
            prompts: Mutex::new(0),
        }
    }

    /// A device with no biometric hardware.
    #[must_use]
    pub fn unavailable() -> Self {
        let auth = Self::on(BiometryDevice::new(BiometricAvailability::NotPresent));
        auth.script(Err(SecretError::Unavailable(
            UnavailableReason::Unsupported,
        )));
        auth
    }

    pub fn script(&self, outcome: Result<PreflightResult, SecretError>) {
        *self.outcome.lock().unwrap() = outcome;
    }

    /// How many times the prompt was shown.
    #[must_use]
    pub fn prompts_shown(&self) -> u32 {
        *self.prompts.lock().unwrap()
    }
}

impl BiometricAuth for MockBiometricAuth {
    fn availability(&self) -> BiometricAvailability {
        self.device.get()
    }

    fn preflight_prompt(&self, _prompt: &AuthPrompt) -> Result<PreflightResult, SecretError> {
        *self.prompts.lock().unwrap() += 1;
        self.outcome.lock().unwrap().clone()
    }
}

/// A scriptable [`BootClock`]: the test restarts the "device", moves time forward, makes the clock go
/// backwards (as a wall clock can) or makes it fail, and the code under test sees exactly that.
pub struct MockBootClock {
    state: Mutex<ClockState>,
}

struct ClockState {
    boot: [u8; 16],
    now_ns: u64,
    failing: bool,
}

impl MockBootClock {
    /// Boot number 1, one second after boot.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Mutex::new(ClockState {
                boot: Self::boot_bytes(1),
                now_ns: 1_000_000_000,
                failing: false,
            }),
        }
    }

    fn boot_bytes(number: u64) -> [u8; 16] {
        let mut bytes = [0_u8; 16];
        bytes[..8].copy_from_slice(&number.to_le_bytes());
        bytes
    }

    /// Time passes (the Mac may sleep: the clock counts it).
    pub fn advance(&self, nanoseconds: u64) {
        self.state.lock().unwrap().now_ns += nanoseconds;
    }

    /// Time passes, in days.
    pub fn advance_days(&self, days: u64) {
        self.advance(days * 24 * 60 * 60 * 1_000_000_000);
    }

    /// The Mac restarts: a new boot id and a clock that counts from the new boot.
    pub fn restart(&self) {
        let mut state = self.state.lock().unwrap();
        let next = u64::from_le_bytes(state.boot[..8].try_into().unwrap()) + 1;
        state.boot = Self::boot_bytes(next);
        state.now_ns = 1_000_000_000;
    }

    /// The clock reads `nanoseconds` (a test can move it backwards, which a monotonic clock never
    /// does; this lets a test prove the rules do not depend on that).
    pub fn set_now(&self, nanoseconds: u64) {
        self.state.lock().unwrap().now_ns = nanoseconds;
    }

    /// Every call fails until `false`.
    pub fn fail(&self, failing: bool) {
        self.state.lock().unwrap().failing = failing;
    }
}

impl Default for MockBootClock {
    fn default() -> Self {
        Self::new()
    }
}

impl BootClock for MockBootClock {
    fn boot_id(&self) -> Result<BootId, BootClockError> {
        let state = self.state.lock().unwrap();
        if state.failing {
            return Err(BootClockError);
        }
        Ok(BootId::new(state.boot))
    }

    fn monotonic_ns(&self) -> Result<u64, BootClockError> {
        let state = self.state.lock().unwrap();
        if state.failing {
            return Err(BootClockError);
        }
        Ok(state.now_ns)
    }
}

/// An in-memory [`RollbackWitness`] for tests. **The only implementation in the tree until the
/// Sync client engine supplies the platform one** (`personal-cfo-elciw`); a source-scan test fails
/// any other. It keeps the contract of the trait (monotone, never invents a value) and lets a
/// test script the failures a real witness can have.
///
/// Clones share state, as a real witness does across the code that holds it: a test keeps one clone
/// to inspect or tamper with while the code under test holds another.
#[derive(Clone, Default)]
pub struct MockWitness(Arc<Mutex<WitnessState>>);

#[derive(Default)]
struct WitnessState {
    counters: HashMap<CounterContext, u64>,
    marks: HashMap<[u8; 16], VerifiedMark>,
    unavailable: bool,
    /// How many of the next `advance_counter` calls fail with `Unavailable` before one succeeds.
    fail_counter_advances: usize,
    /// How many `advance_counter` calls succeed before the scripted failures start.
    skip_before_fail: usize,
    counter_advances: usize,
}

impl MockWitness {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Make every call fail with [`WitnessError::Unavailable`] (or stop doing so).
    pub fn set_unavailable(&self, unavailable: bool) {
        self.0.lock().unwrap().unavailable = unavailable;
    }

    /// Make the next `count` counter advances fail with [`WitnessError::Unavailable`].
    pub fn fail_next_counter_advances(&self, count: usize) {
        self.0.lock().unwrap().fail_counter_advances = count;
    }

    /// Let the next `succeed` counter advances through, then fail the `count` after them with
    /// [`WitnessError::Unavailable`]: a crash at a chosen step of a longer sequence.
    pub fn fail_counter_advances_after(&self, succeed: usize, count: usize) {
        let mut state = self.0.lock().unwrap();
        state.skip_before_fail = succeed;
        state.fail_counter_advances = count;
    }

    /// How many counter advances succeeded (a no-op advance to the held value counts).
    #[must_use]
    pub fn counter_advances(&self) -> usize {
        self.0.lock().unwrap().counter_advances
    }

    /// The witnessed value, bypassing the trait (for assertions).
    #[must_use]
    pub fn peek_counter(&self, context: &CounterContext) -> Option<u64> {
        self.0.lock().unwrap().counters.get(context).copied()
    }

    /// Set a counter to any value, forwards or backwards: the way a test injects a witness that is
    /// ahead of a rolled-back register. A real witness cannot be told to do this.
    pub fn force_counter(&self, context: &CounterContext, value: u64) {
        self.0.lock().unwrap().counters.insert(*context, value);
    }

    /// Forget a counter entirely: a witness that has lost its record of a context.
    pub fn forget_counter(&self, context: &CounterContext) {
        self.0.lock().unwrap().counters.remove(context);
    }

    /// Set the verified mark to any value (for the same reason as [`Self::force_counter`]).
    pub fn force_mark(&self, vault_id: &[u8; 16], mark: VerifiedMark) {
        self.0.lock().unwrap().marks.insert(*vault_id, mark);
    }

    /// Forget a vault's mark entirely.
    pub fn forget_mark(&self, vault_id: &[u8; 16]) {
        self.0.lock().unwrap().marks.remove(vault_id);
    }
}

impl RollbackWitness for MockWitness {
    fn counter(&self, context: &CounterContext) -> Result<Option<u64>, WitnessError> {
        let state = self.0.lock().unwrap();
        if state.unavailable {
            return Err(WitnessError::Unavailable);
        }
        Ok(state.counters.get(context).copied())
    }

    fn advance_counter(
        &self,
        context: &CounterContext,
        next_counter: u64,
    ) -> Result<(), WitnessError> {
        let mut state = self.0.lock().unwrap();
        if state.unavailable {
            return Err(WitnessError::Unavailable);
        }
        if state.skip_before_fail > 0 {
            state.skip_before_fail -= 1;
        } else if state.fail_counter_advances > 0 {
            state.fail_counter_advances -= 1;
            return Err(WitnessError::Unavailable);
        }
        if state
            .counters
            .get(context)
            .is_some_and(|held| *held > next_counter)
        {
            return Err(WitnessError::WouldRegress);
        }
        state.counters.insert(*context, next_counter);
        state.counter_advances += 1;
        Ok(())
    }

    fn verified_mark(&self, vault_id: &[u8; 16]) -> Result<Option<VerifiedMark>, WitnessError> {
        let state = self.0.lock().unwrap();
        if state.unavailable {
            return Err(WitnessError::Unavailable);
        }
        Ok(state.marks.get(vault_id).copied())
    }

    fn advance_verified_mark(
        &self,
        vault_id: &[u8; 16],
        mark: &VerifiedMark,
    ) -> Result<(), WitnessError> {
        let mut state = self.0.lock().unwrap();
        if state.unavailable {
            return Err(WitnessError::Unavailable);
        }
        if let Some(held) = state.marks.get(vault_id) {
            match mark.cmp_progress(held) {
                Progress::Equal | Progress::Ahead => {}
                Progress::Behind | Progress::Diverged => return Err(WitnessError::WouldRegress),
            }
        }
        state.marks.insert(*vault_id, *mark);
        Ok(())
    }
}
