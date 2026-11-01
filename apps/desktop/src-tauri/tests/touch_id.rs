//! Touch ID unlock, the app-side rules (`personal-cfo-xuu`, PR 3), against the platform mock.
//!
//! The owner's rule (2026-11-01, matching the iPhone): the VAULT password is required, and Touch ID is not
//! offered, after a restart (K1), after three failed or cancelled prompts (K2) and 30 days after the last
//! password unlock (K3), all of it app-side, failing closed. One test per rule, one per lifecycle edge
//! and one per branch of the error policy. What a real sensor does is the owner's Touch ID run (AC B).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use app_lib::ipc::commands::{
    change_password_impl, create_vault_impl, delete_vault_impl, lock_vault_impl,
    rotate_vault_key_impl, touch_id_disable_impl, touch_id_enable_impl, touch_id_status_impl,
    touch_id_unlock_with_options_impl, unlock_vault_impl,
};
use app_lib::ipc::dto::{
    ChangePasswordInput, RotateVaultKeyInput, TouchIdEnableInput, TouchIdStateDto,
    TouchIdStatusDto, TouchIdUnlockDto, VaultStateDto,
};
use app_lib::ipc::IpcError;
use app_lib::touch_id::{app as touch, TouchId, MAX_FAILED_PROMPTS};
use app_lib::AppState;
use finance_kernel::{
    Account, AccountFlags, AccountId, ActorType, CashflowRole, CommandEnvelope, CommandMeta,
    CreateAccount, Currency, LedgerAccountId, UnlockOptions, VaultController,
};
use platform_mock::{
    BiometryDevice, BiometryScript, MockBiometricAuth, MockBootClock, MockSecretStore,
};
use platform_traits::{
    AccessPolicy, AuthPrompt, BiometricAvailability, PublicTag, SecretError, SecretKey,
    SecretStore, SecretValue, StoreCapabilities, UnavailableReason,
};
use tempfile::TempDir;
use uuid::Uuid;

const PASSWORD: &str = "test-only touch id password";
const SERVICE: &str = "app.dohflow.touch-id-unlock";
const ACCOUNT: &str = "default";
const COUNTER: usize = 57;

/// A store that delegates to the mock and can be told to spy on a prompt, fail a delete or fail a load.
struct Spy {
    inner: MockSecretStore,
    counter_seen_at_prompt: Mutex<Vec<u8>>,
    fail_delete: AtomicBool,
    load_error: Mutex<Option<SecretError>>,
}

impl SecretStore for Spy {
    fn store_secret_tagged(
        &self,
        key: &SecretKey,
        value: &SecretValue,
        policy: AccessPolicy,
        tag: Option<&PublicTag>,
    ) -> Result<(), SecretError> {
        self.inner.store_secret_tagged(key, value, policy, tag)
    }

    fn load_secret(
        &self,
        key: &SecretKey,
        prompt: &AuthPrompt,
    ) -> Result<SecretValue, SecretError> {
        // What the item's tag says WHILE the prompt would be showing.
        if let Ok(Some(tag)) = self.inner.public_tag(key) {
            self.counter_seen_at_prompt
                .lock()
                .unwrap()
                .push(tag.as_bytes()[COUNTER]);
        }
        if let Some(error) = self.load_error.lock().unwrap().clone() {
            return Err(error);
        }
        self.inner.load_secret(key, prompt)
    }

    fn update_public_tag(
        &self,
        key: &SecretKey,
        tag: Option<&PublicTag>,
    ) -> Result<(), SecretError> {
        self.inner.update_public_tag(key, tag)
    }

    fn delete_secret(&self, key: &SecretKey) -> Result<(), SecretError> {
        if self.fail_delete.load(Ordering::SeqCst) {
            return Err(SecretError::Platform { code: 1 });
        }
        self.inner.delete_secret(key)
    }

    fn public_tag(&self, key: &SecretKey) -> Result<Option<PublicTag>, SecretError> {
        self.inner.public_tag(key)
    }

    fn capabilities(&self) -> StoreCapabilities {
        self.inner.capabilities()
    }
}

struct Rig {
    dir: TempDir,
    state: AppState,
    store: Arc<Spy>,
    clock: Arc<MockBootClock>,
    auth: Arc<MockBiometricAuth>,
    device: BiometryDevice,
}

impl Rig {
    /// An unlocked vault, Touch ID not yet enabled, on a device with an enrolled fingerprint.
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let device = BiometryDevice::available();
        let store = Arc::new(Spy {
            inner: MockSecretStore::on_device(device.clone()),
            counter_seen_at_prompt: Mutex::default(),
            fail_delete: AtomicBool::new(false),
            load_error: Mutex::default(),
        });
        let clock = Arc::new(MockBootClock::new());
        let auth = Arc::new(MockBiometricAuth::on(device.clone()));
        let controller = VaultController::open(dir.path().join("vault.db"));
        let state = AppState::new(controller).with_touch_id(TouchId::new(
            store.clone(),
            auth.clone(),
            clock.clone(),
        ));
        create_vault_impl(&state, PASSWORD.to_owned()).expect("create vault");
        Self {
            dir,
            state,
            store,
            clock,
            auth,
            device,
        }
    }

    fn key() -> SecretKey {
        SecretKey::new(SERVICE, ACCOUNT).unwrap()
    }

    fn enable(&self) {
        let outcome = touch_id_enable_impl(&self.state, enable_input(PASSWORD)).expect("enable");
        assert!(outcome.enabled, "{outcome:?}");
    }

    fn lock(&self) {
        let status = lock_vault_impl(&self.state).expect("lock");
        assert_eq!(status.state, VaultStateDto::Locked);
    }

    fn password_unlock(&self) {
        let status = unlock_vault_impl(&self.state, PASSWORD.to_owned()).expect("password unlock");
        assert_eq!(status.state, VaultStateDto::Unlocked);
    }

    fn touch(&self) -> TouchIdUnlockDto {
        touch_id_unlock_with_options_impl(&self.state, UnlockOptions::default()).expect("attempt")
    }

    fn status(&self) -> TouchIdStatusDto {
        touch_id_status_impl(&self.state)
    }

    fn script(&self, script: BiometryScript) {
        self.store.inner.script_biometry(script);
    }

    fn item_exists(&self) -> bool {
        self.store.inner.public_tag(&Self::key()).is_ok()
    }

    /// The failed-prompt counter as the item's public tag holds it.
    fn counter(&self) -> u8 {
        self.store
            .inner
            .public_tag(&Self::key())
            .unwrap()
            .unwrap()
            .as_bytes()[COUNTER]
    }

    fn vault_path(&self) -> std::path::PathBuf {
        self.dir.path().join("vault.db")
    }
}

fn enable_input(password: &str) -> TouchIdEnableInput {
    TouchIdEnableInput {
        password: password.to_owned(),
    }
}

fn reason(dto: &TouchIdUnlockDto) -> &str {
    dto.reason.as_deref().expect("a reason")
}

// ---- enable / disable ----------------------------------------------------------------------------

#[test]
fn enabling_then_unlocking_with_touch_id_opens_the_vault_and_counts_no_failure() {
    let rig = Rig::new();
    assert_eq!(rig.status().state, TouchIdStateDto::Off);
    rig.enable();
    assert_eq!(rig.status().state, TouchIdStateDto::On);
    rig.lock();
    assert!(rig.status().offered);

    rig.script(BiometryScript::Match);
    let outcome = rig.touch();
    assert!(outcome.unlocked, "{outcome:?}");
    assert_eq!(outcome.vault.unwrap().state, VaultStateDto::Unlocked);
    assert_eq!(rig.counter(), 0);
    assert_eq!(
        rig.auth.prompts_shown(),
        0,
        "no pre-flight prompt gates the release"
    );
}

#[test]
fn enabling_needs_the_vault_password_again_and_a_wrong_one_changes_nothing() {
    let rig = Rig::new();
    let error = touch_id_enable_impl(&rig.state, enable_input("not the password")).unwrap_err();
    assert!(matches!(error, IpcError::VaultUnlockFailed), "{error:?}");
    assert!(!rig.item_exists());
    assert_eq!(rig.status().state, TouchIdStateDto::Off);
}

#[test]
fn enabling_on_a_device_that_cannot_store_it_stays_password_only_with_the_reason() {
    for (state, code) in [
        (BiometricAvailability::NotEnrolled, "biometry_not_enrolled"),
        (BiometricAvailability::NotPresent, "biometry_not_present"),
        (BiometricAvailability::LockedOut, "biometry_locked_out"),
        (BiometricAvailability::Unknown, "biometry_unknown"),
    ] {
        let rig = Rig::new();
        rig.device.set(state);
        let outcome = touch_id_enable_impl(&rig.state, enable_input(PASSWORD)).unwrap();
        assert!(!outcome.enabled);
        assert_eq!(outcome.reason.as_deref(), Some(code));
        assert!(!rig.item_exists(), "{state:?}");
        // The vault is untouched and the password still opens it.
        rig.lock();
        rig.password_unlock();
    }
}

#[test]
fn a_failed_re_enable_removes_the_older_item() {
    let rig = Rig::new();
    rig.enable();
    assert!(rig.item_exists());
    rig.device.set(BiometricAvailability::NotEnrolled);
    let outcome = touch_id_enable_impl(&rig.state, enable_input(PASSWORD)).unwrap();
    assert!(!outcome.enabled);
    assert!(
        !rig.item_exists(),
        "password-only, and no older item lingers"
    );
}

#[test]
fn disabling_removes_the_item_and_is_idempotent() {
    let rig = Rig::new();
    rig.enable();
    assert_eq!(
        touch_id_disable_impl(&rig.state).unwrap().state,
        TouchIdStateDto::Off
    );
    assert!(!rig.item_exists());
    assert_eq!(
        touch_id_disable_impl(&rig.state).unwrap().state,
        TouchIdStateDto::Off
    );
}

#[test]
fn an_item_removed_behind_the_apps_back_leaves_the_password_working() {
    let rig = Rig::new();
    rig.enable();
    rig.lock();
    rig.store.inner.delete_secret(&Rig::key()).unwrap();
    assert_eq!(rig.status().state, TouchIdStateDto::Off);
    let outcome = rig.touch();
    assert!(!outcome.unlocked);
    assert_eq!(reason(&outcome), "not_enabled");
    rig.password_unlock();
}

// ---- K1: a restart --------------------------------------------------------------------------------

#[test]
fn k1_after_a_restart_touch_id_is_not_offered_and_no_prompt_is_shown() {
    let rig = Rig::new();
    rig.enable();
    rig.lock();
    rig.clock.restart();

    let status = rig.status();
    assert!(!status.offered);
    assert_eq!(status.reason.as_deref(), Some("after_restart"));
    let outcome = rig.touch();
    assert!(!outcome.unlocked);
    assert_eq!(reason(&outcome), "after_restart");
    assert!(
        rig.store.counter_seen_at_prompt.lock().unwrap().is_empty(),
        "the item was not even asked for"
    );
    assert_eq!(rig.counter(), 0, "a restart is not a failed prompt");
    assert!(rig.item_exists(), "the item is kept");
}

#[test]
fn k1_the_password_unlock_after_a_restart_restamps_and_touch_id_is_offered_again() {
    let rig = Rig::new();
    rig.enable();
    rig.lock();
    rig.clock.restart();
    rig.password_unlock();
    rig.lock();
    assert!(rig.status().offered);
    rig.script(BiometryScript::Match);
    assert!(rig.touch().unlocked);
}

#[test]
fn k1_an_unreadable_boot_id_fails_closed() {
    let rig = Rig::new();
    rig.enable();
    rig.lock();
    rig.clock.fail(true);
    let outcome = rig.touch();
    assert!(!outcome.unlocked);
    assert_eq!(reason(&outcome), "markers_unreadable");
    rig.clock.fail(false);
    rig.password_unlock();
}

// ---- K2: failed or cancelled prompts --------------------------------------------------------------

#[test]
fn k2_three_cancelled_prompts_require_the_password_and_only_a_password_unlock_resets_it() {
    let rig = Rig::new();
    rig.enable();
    rig.lock();
    rig.script(BiometryScript::Cancel);

    assert_eq!(reason(&rig.touch()), "prompt_not_completed");
    assert_eq!(rig.counter(), 1);
    assert_eq!(reason(&rig.touch()), "prompt_not_completed");
    assert_eq!(rig.counter(), 2);
    assert_eq!(reason(&rig.touch()), "too_many_failed_prompts");
    assert_eq!(rig.counter(), MAX_FAILED_PROMPTS);

    // The fourth time Touch ID is not offered, and nothing is asked of the platform.
    let before = rig.store.counter_seen_at_prompt.lock().unwrap().len();
    let status = rig.status();
    assert!(!status.offered);
    assert_eq!(status.reason.as_deref(), Some("too_many_failed_prompts"));
    rig.script(BiometryScript::Match);
    let outcome = rig.touch();
    assert!(!outcome.unlocked, "even a matching finger is not asked for");
    assert_eq!(reason(&outcome), "too_many_failed_prompts");
    assert_eq!(
        rig.store.counter_seen_at_prompt.lock().unwrap().len(),
        before
    );

    rig.password_unlock();
    rig.lock();
    assert_eq!(rig.counter(), 0, "a password unlock zeroes the count");
    assert!(rig.status().offered);
}

#[test]
fn k2_failed_matches_count_like_cancels() {
    let rig = Rig::new();
    rig.enable();
    rig.lock();
    rig.script(BiometryScript::Fail);
    for _ in 0..2 {
        assert!(!rig.touch().unlocked);
    }
    assert_eq!(reason(&rig.touch()), "too_many_failed_prompts");
    assert!(rig.item_exists(), "a wrong finger never deletes the item");
}

#[test]
fn k2_three_successful_unlocks_never_force_the_password() {
    let rig = Rig::new();
    rig.enable();
    rig.script(BiometryScript::Match);
    for _ in 0..5 {
        rig.lock();
        assert!(rig.touch().unlocked);
        assert_eq!(rig.counter(), 0, "a success restores the pre-prompt value");
    }
    rig.lock();
    assert!(rig.status().offered);
}

#[test]
fn k2_cancel_success_cancel_cancel_forces_the_password() {
    let rig = Rig::new();
    rig.enable();
    rig.lock();
    rig.script(BiometryScript::Cancel);
    assert!(!rig.touch().unlocked); // 1
    rig.script(BiometryScript::Match);
    assert!(rig.touch().unlocked); // a success: back to 1, not zero
    assert_eq!(rig.counter(), 1);
    rig.lock();
    rig.script(BiometryScript::Cancel);
    assert!(!rig.touch().unlocked); // 2
    assert_eq!(reason(&rig.touch()), "too_many_failed_prompts"); // 3
    assert!(!rig.status().offered);
}

#[test]
fn k2_the_counter_is_raised_before_the_prompt_is_shown() {
    let rig = Rig::new();
    rig.enable();
    rig.lock();
    rig.script(BiometryScript::Match);
    assert!(rig.touch().unlocked);
    // While the platform would have been asking the user, the tag already said one prompt was spent: a
    // crash during the prompt still counts.
    assert_eq!(*rig.store.counter_seen_at_prompt.lock().unwrap(), vec![1]);
}

#[test]
fn k2_a_lockout_requires_the_password_at_once() {
    let rig = Rig::new();
    rig.enable();
    rig.lock();
    *rig.store.load_error.lock().unwrap() = Some(SecretError::BiometryLockedOut);
    let outcome = rig.touch();
    assert!(!outcome.unlocked);
    assert_eq!(reason(&outcome), "biometry_locked_out");
    assert_eq!(rig.counter(), MAX_FAILED_PROMPTS);
    assert!(rig.item_exists());
    *rig.store.load_error.lock().unwrap() = None;
    rig.script(BiometryScript::Match);
    assert!(!rig.status().offered);
}

#[test]
fn k2_an_unwritable_counter_means_the_password_and_no_prompt() {
    let rig = Rig::new();
    rig.enable();
    rig.lock();
    // The item's tag is replaced by one the app does not understand: nothing can be counted.
    rig.store
        .inner
        .update_public_tag(&Rig::key(), Some(&PublicTag::new(b"junk").unwrap()))
        .unwrap();
    let outcome = rig.touch();
    assert!(!outcome.unlocked);
    assert_eq!(reason(&outcome), "markers_unreadable");
    assert!(rig.store.counter_seen_at_prompt.lock().unwrap().is_empty());
}

// ---- K3: 30 days ----------------------------------------------------------------------------------

#[test]
fn k3_thirty_days_after_the_last_password_unlock_the_password_is_required() {
    let rig = Rig::new();
    rig.enable();
    rig.lock();
    rig.clock.advance_days(29);
    assert!(rig.status().offered, "day 29");
    rig.clock.advance_days(1);
    let status = rig.status();
    assert!(!status.offered, "day 30");
    assert_eq!(status.reason.as_deref(), Some("password_too_old"));
    let outcome = rig.touch();
    assert!(!outcome.unlocked);
    assert_eq!(reason(&outcome), "password_too_old");

    rig.password_unlock();
    rig.lock();
    assert!(
        rig.status().offered,
        "the password unlock restarts the 30 days"
    );
}

#[test]
fn k3_a_touch_id_unlock_does_not_extend_the_thirty_days() {
    let rig = Rig::new();
    rig.enable();
    rig.script(BiometryScript::Match);
    rig.lock();
    rig.clock.advance_days(20);
    assert!(rig.touch().unlocked);
    rig.lock();
    rig.clock.advance_days(10);
    assert!(
        !rig.status().offered,
        "30 days since the PASSWORD, not since the last unlock"
    );
}

#[test]
fn k3_a_clock_that_reads_earlier_than_the_stamp_fails_closed() {
    let rig = Rig::new();
    rig.enable();
    rig.lock();
    rig.clock.set_now(1);
    let status = rig.status();
    assert!(!status.offered);
    assert_eq!(status.reason.as_deref(), Some("markers_unreadable"));
    rig.clock.fail(true);
    assert!(!rig.status().offered);
}

// ---- the error policy (03a's M3, AC I) -----------------------------------------------------------

#[test]
fn a_changed_fingerprint_set_deletes_the_item_and_offers_enrollment_after_the_password() {
    let rig = Rig::new();
    rig.enable();
    rig.lock();
    rig.script(BiometryScript::EnrollmentChanged);
    let outcome = rig.touch();
    assert!(!outcome.unlocked);
    assert_eq!(reason(&outcome), "retired");
    assert!(!rig.item_exists());
    assert_eq!(rig.status().state, TouchIdStateDto::Off);
    rig.password_unlock();
}

#[test]
fn an_item_that_is_not_released_material_is_deleted() {
    let rig = Rig::new();
    rig.enable();
    rig.lock();
    // Right tag, wrong value length: an item this build cannot read.
    let tag = rig.store.inner.public_tag(&Rig::key()).unwrap().unwrap();
    rig.store
        .inner
        .store_secret_tagged(
            &Rig::key(),
            &SecretValue::new(b"short").unwrap(),
            AccessPolicy::DeviceOnlyBiometryCurrentSet,
            Some(&tag),
        )
        .unwrap();
    rig.script(BiometryScript::Match);
    let outcome = rig.touch();
    assert!(!outcome.unlocked);
    assert_eq!(reason(&outcome), "retired");
    assert!(!rig.item_exists());
}

#[test]
fn a_damaged_vault_keeps_the_item_restores_the_counter_and_reports_the_same_error_as_the_password()
{
    let rig = Rig::new();
    // Spread the vault over many pages, so damage to the last ones is damage to real content.
    {
        let guard = rig.state.lock_controller().unwrap();
        let kernel = guard.kernel().unwrap();
        for n in 0..400 {
            kernel
                .dispatch(CommandEnvelope::new(
                    meta(),
                    CreateAccount::new(Account::new(
                        AccountId::new(),
                        LedgerAccountId::new(),
                        format!("Account number {n} with a reasonably long name to fill pages"),
                        CashflowRole::LiquidCash,
                        Currency::Usd,
                        AccountFlags::default(),
                    )),
                ))
                .unwrap();
        }
    }
    rig.enable();
    rig.lock();
    let path = rig.vault_path();
    let mut bytes = std::fs::read(&path).unwrap();
    let pages = bytes.len() / 4096;
    assert!(pages > 8);
    for page in pages - 3..pages {
        for byte in &mut bytes[page * 4096 + 64..page * 4096 + 192] {
            *byte ^= 0xA5;
        }
    }
    std::fs::write(&path, &bytes).unwrap();

    rig.script(BiometryScript::Match);
    let error =
        touch_id_unlock_with_options_impl(&rig.state, UnlockOptions::with_integrity_check())
            .unwrap_err();
    assert!(
        matches!(error, IpcError::VaultIntegrityCheckFailed),
        "{error:?}"
    );
    assert!(
        rig.item_exists(),
        "a damaged vault is not a reason to forget the key"
    );
    assert_eq!(rig.counter(), 0, "and it is not a failed prompt");
}

fn meta() -> CommandMeta {
    CommandMeta {
        command_id: Uuid::now_v7(),
        correlation_id: Uuid::now_v7(),
        causation_id: None,
        actor_type: ActorType::User,
        actor_id: "tester".to_owned(),
        idempotency_key: Uuid::new_v4().to_string(),
        issued_at: finance_kernel::IssuedAt::now(),
    }
}

#[test]
fn a_store_that_says_the_build_is_unsigned_is_password_only_and_says_why() {
    let dir = TempDir::new().unwrap();
    let state = AppState::new(VaultController::open(dir.path().join("vault.db")))
        .with_touch_id(TouchId::absent(UnavailableReason::MissingEntitlement));
    create_vault_impl(&state, PASSWORD.to_owned()).unwrap();
    let status = touch_id_status_impl(&state);
    assert_eq!(status.state, TouchIdStateDto::Unavailable);
    assert_eq!(status.reason.as_deref(), Some("signed_build_needed"));
    let outcome = touch_id_enable_impl(&state, enable_input(PASSWORD)).unwrap();
    assert!(!outcome.enabled);
    // Nothing to remove, so the key changes are not blocked by it.
    change_password_impl(
        &state,
        ChangePasswordInput {
            old_password: PASSWORD.to_owned(),
            new_password: "another long password".to_owned(),
        },
    )
    .unwrap();
}

// ---- lifecycle: the item never outlives the key it unwraps ----------------------------------------

#[test]
fn a_password_change_removes_the_item() {
    let rig = Rig::new();
    rig.enable();
    change_password_impl(
        &rig.state,
        ChangePasswordInput {
            old_password: PASSWORD.to_owned(),
            new_password: "a brand new password".to_owned(),
        },
    )
    .unwrap();
    assert!(!rig.item_exists());
    assert_eq!(rig.status().state, TouchIdStateDto::Off);
}

#[test]
fn a_key_rotation_removes_the_item() {
    let rig = Rig::new();
    rig.enable();
    rotate_vault_key_impl(
        &rig.state,
        RotateVaultKeyInput {
            password: PASSWORD.to_owned(),
        },
    )
    .unwrap();
    assert!(!rig.item_exists());
}

#[test]
fn deleting_the_vault_removes_the_item() {
    let rig = Rig::new();
    rig.enable();
    delete_vault_impl(&rig.state).unwrap();
    assert!(!rig.item_exists());
}

#[test]
fn a_change_does_not_start_when_the_item_cannot_be_removed_first() {
    let rig = Rig::new();
    rig.enable();
    rig.store.fail_delete.store(true, Ordering::SeqCst);

    let changed = change_password_impl(
        &rig.state,
        ChangePasswordInput {
            old_password: PASSWORD.to_owned(),
            new_password: "a brand new password".to_owned(),
        },
    );
    assert!(
        matches!(changed, Err(IpcError::Persistence(_))),
        "{changed:?}"
    );
    let rotated = rotate_vault_key_impl(
        &rig.state,
        RotateVaultKeyInput {
            password: PASSWORD.to_owned(),
        },
    );
    assert!(
        matches!(rotated, Err(IpcError::Persistence(_))),
        "{rotated:?}"
    );
    assert!(matches!(
        delete_vault_impl(&rig.state),
        Err(IpcError::Persistence(_))
    ));

    // Nothing changed: the old password still opens the vault and the item is still there.
    rig.lock();
    rig.password_unlock();
    assert!(rig.item_exists());
}

#[test]
fn a_crash_between_the_change_and_the_removal_is_swept_at_launch() {
    let rig = Rig::new();
    rig.enable();
    // The password changes in the kernel, and the app dies before it can remove the item.
    {
        let mut guard = rig.state.lock_controller().unwrap();
        guard
            .change_password(PASSWORD.as_bytes(), b"changed behind the apps back")
            .unwrap();
        guard.lock().unwrap();
    }
    assert!(rig.item_exists());
    let status = rig.status();
    assert_eq!(status.reason.as_deref(), Some("retired"));
    assert!(!status.offered);
    let outcome = rig.touch();
    assert!(!outcome.unlocked, "a stale item is never asked for");

    touch::sweep(&rig.state);
    assert!(!rig.item_exists(), "the launch sweep removes it");
}

#[test]
fn a_password_unlock_removes_an_item_that_no_longer_matches_the_vault() {
    let rig = Rig::new();
    rig.enable();
    {
        let mut guard = rig.state.lock_controller().unwrap();
        guard
            .change_password(PASSWORD.as_bytes(), b"changed behind the apps back")
            .unwrap();
        guard.lock().unwrap();
    }
    let status =
        unlock_vault_impl(&rig.state, "changed behind the apps back".to_owned()).expect("unlock");
    assert_eq!(status.state, VaultStateDto::Unlocked);
    assert!(!rig.item_exists());
}

#[test]
fn an_item_whose_markers_are_unreadable_is_swept_and_never_offered() {
    let rig = Rig::new();
    rig.enable();
    rig.lock();
    rig.store
        .inner
        .update_public_tag(&Rig::key(), Some(&PublicTag::new(b"not markers").unwrap()))
        .unwrap();
    assert!(!rig.status().offered);
    touch::sweep(&rig.state);
    assert!(!rig.item_exists());
}

#[test]
fn nothing_the_commands_return_carries_key_material_or_error_text() {
    let rig = Rig::new();
    rig.enable();
    rig.lock();
    rig.script(BiometryScript::Cancel);
    let outcome = rig.touch();
    // Only fixed codes: lowercase words and underscores.
    for code in [outcome.reason.as_deref(), rig.status().reason.as_deref()]
        .into_iter()
        .flatten()
    {
        assert!(
            code.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
            "{code}"
        );
    }
}
