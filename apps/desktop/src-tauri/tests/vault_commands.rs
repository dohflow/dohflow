//! Integration tests for the vault lifecycle commands (personal-cfo-8v2 / -3ry).
//!
//! These drive the real `*_impl` functions against an `AppState` whose
//! `VaultController` points at a temp-dir vault path — no webview, no mocked
//! crypto. They prove create → unlock → lock transitions, persistence across a
//! lock, and that a wrong password is rejected without opening the vault.

use app_lib::ipc::commands::{
    account_count_impl, change_password_impl, create_account_impl, create_vault_impl,
    create_vault_named_impl, delete_vault_impl, household_timezone_impl, list_vaults_impl,
    lock_vault_impl, rename_vault_impl, switch_vault_impl, unlock_vault_impl, vault_status_impl,
};
use app_lib::ipc::dto::{
    AccountFlagsDto, CashflowRoleDto, ChangePasswordInput, CreateAccountInput, VaultStateDto,
};
use app_lib::ipc::IpcError;
use app_lib::vault_registry::{VaultEntry, VaultRegistry};
use app_lib::AppState;
use finance_kernel::VaultController;
use tempfile::TempDir;
use uuid::Uuid;

const PASSWORD: &str = "correct horse battery staple";
const NEW_PASSWORD: &str = "an entirely different phrase";

fn change_input(old: &str, new: &str) -> ChangePasswordInput {
    ChangePasswordInput {
        old_password: old.to_owned(),
        new_password: new.to_owned(),
    }
}

/// An `AppState` over a fresh temp path with no vault yet (state: `NoVault`).
fn empty_state() -> (TempDir, AppState) {
    let dir = TempDir::new().expect("temp dir");
    let controller = VaultController::open(dir.path().join("vault.db"));
    (dir, AppState::new(controller))
}

/// An `AppState` with a real app-data root + an empty registry (multi-vault mode).
fn registry_state() -> (TempDir, AppState) {
    let dir = TempDir::new().expect("temp dir");
    let controller = VaultController::open(dir.path().join("vault.db"));
    let state = AppState::with_registry(
        controller,
        VaultRegistry::default(),
        dir.path().to_path_buf(),
    );
    (dir, state)
}

fn create_input(name: &str) -> CreateAccountInput {
    CreateAccountInput {
        name: name.to_owned(),
        cashflow_role: CashflowRoleDto::LiquidCash,
        currency: "USD".to_owned(),
        flags: Some(AccountFlagsDto {
            retirement: false,
            tax_advantaged: false,
            joint: false,
            business: false,
        }),
        opening_balance: None,
        subtype: None,
        idempotency_key: String::new(),
    }
}

#[test]
fn status_of_an_empty_location_is_no_vault() {
    let (_dir, state) = empty_state();
    let status = vault_status_impl(&state).unwrap();
    assert_eq!(status.state, VaultStateDto::NoVault);
    assert_eq!(status.account_count, None);
}

#[test]
fn list_vaults_returns_registered_vaults_with_the_active_flagged() {
    let dir = TempDir::new().unwrap();
    let real = Uuid::now_v7();
    let test = Uuid::now_v7();
    let registry = VaultRegistry {
        vaults: vec![
            VaultEntry {
                id: real,
                name: "Real".to_owned(),
                path: "vault.db".into(),
                created_at: "2026-07-03T00:00:00Z".to_owned(),
            },
            VaultEntry {
                id: test,
                name: "Test".to_owned(),
                path: "vaults/b/vault.db".into(),
                created_at: "2026-07-03T00:00:00Z".to_owned(),
            },
        ],
        active: Some(test),
    };
    let controller = VaultController::open(dir.path().join("vault.db"));
    let state = AppState::with_registry(controller, registry, dir.path().to_path_buf());

    let list = list_vaults_impl(&state).unwrap();
    assert_eq!(list.vaults.len(), 2);
    assert_eq!(list.vaults[0].name, "Real");
    assert!(!list.vaults[0].is_active);
    assert_eq!(list.vaults[1].name, "Test");
    assert!(list.vaults[1].is_active, "the active vault is flagged");
}

#[test]
fn create_named_vaults_switch_between_them_and_delete_falls_back() {
    let (_dir, state) = registry_state();

    // Create "Real" (unlocked) with an account.
    let real = create_vault_named_impl(&state, "Real".to_owned(), PASSWORD.to_owned()).unwrap();
    assert_eq!(real.state, VaultStateDto::Unlocked);
    create_account_impl(&state, create_input("Checking")).unwrap();

    // Create "Test" — this locks Real and switches to a fresh, empty vault.
    let test = create_vault_named_impl(&state, "Test".to_owned(), PASSWORD.to_owned()).unwrap();
    assert_eq!(test.state, VaultStateDto::Unlocked);
    assert_eq!(test.account_count, Some(0));

    // Both are registered; exactly one (Test) is active.
    let list = list_vaults_impl(&state).unwrap();
    assert_eq!(list.vaults.len(), 2);
    assert_eq!(list.vaults.iter().filter(|v| v.is_active).count(), 1);
    let real_id = list
        .vaults
        .iter()
        .find(|v| v.name == "Real")
        .unwrap()
        .id
        .clone();

    // Switch back to Real → it comes up locked; unlocking reveals its own (isolated) account.
    let switched = switch_vault_impl(&state, real_id).unwrap();
    assert_eq!(switched.state, VaultStateDto::Locked);
    let unlocked = unlock_vault_impl(&state, PASSWORD.to_owned()).unwrap();
    assert_eq!(unlocked.account_count, Some(1), "Real's data, not Test's");

    // Deleting the active vault (Real) drops its entry and falls back to Test (locked).
    let after = delete_vault_impl(&state).unwrap();
    assert_eq!(after.state, VaultStateDto::Locked);
    let list2 = list_vaults_impl(&state).unwrap();
    assert_eq!(list2.vaults.len(), 1);
    assert_eq!(list2.vaults[0].name, "Test");
    assert!(list2.vaults[0].is_active);
}

#[test]
fn rename_vault_updates_the_registry_and_rejects_a_blank_name() {
    let (_dir, state) = registry_state();
    create_vault_named_impl(&state, "Real".to_owned(), PASSWORD.to_owned()).unwrap();
    let id = list_vaults_impl(&state).unwrap().vaults[0].id.clone();

    let updated = rename_vault_impl(&state, id.clone(), "Renamed".to_owned()).unwrap();
    assert_eq!(updated.vaults[0].name, "Renamed");

    assert!(
        rename_vault_impl(&state, id, "   ".to_owned()).is_err(),
        "a blank name is rejected"
    );
    assert_eq!(list_vaults_impl(&state).unwrap().vaults[0].name, "Renamed");
}

#[test]
fn delete_will_not_fall_back_to_a_vault_whose_files_are_missing() {
    let (_dir, state) = registry_state();
    create_vault_named_impl(&state, "Real".to_owned(), PASSWORD.to_owned()).unwrap();
    // A stale registry entry pointing at a vault that isn't on disk.
    {
        let mut registry = state.lock_registry().unwrap();
        registry.vaults.push(VaultEntry {
            id: Uuid::now_v7(),
            name: "Ghost".to_owned(),
            path: "vaults/ghost/vault.db".into(),
            created_at: "2026-07-03T00:00:00Z".to_owned(),
        });
    }
    // Deleting the active vault finds no *present* fallback, so it lands on the create screen.
    let after = delete_vault_impl(&state).unwrap();
    assert_eq!(
        after.state,
        VaultStateDto::NoVault,
        "won't switch to a phantom vault with no files"
    );
}

#[test]
fn delete_wipes_the_vault_and_returns_to_no_vault() {
    let (dir, state) = empty_state();
    create_vault_impl(&state, PASSWORD.to_owned()).unwrap();
    create_account_impl(&state, create_input("Checking")).unwrap();
    let db = dir.path().join("vault.db");
    assert!(db.exists());

    let status = delete_vault_impl(&state).unwrap();
    assert_eq!(status.state, VaultStateDto::NoVault);
    assert!(!db.exists(), "the vault file is gone");

    // The location is genuinely empty — a fresh vault can be created over it.
    let recreated = create_vault_impl(&state, PASSWORD.to_owned()).unwrap();
    assert_eq!(recreated.state, VaultStateDto::Unlocked);
    assert_eq!(recreated.account_count, Some(0));
}

#[test]
fn create_unlocks_and_reports_zero_accounts() {
    let (_dir, state) = empty_state();
    let status = create_vault_impl(&state, PASSWORD.to_owned()).unwrap();
    assert_eq!(status.state, VaultStateDto::Unlocked);
    assert_eq!(status.account_count, Some(0));

    // The unlocked vault accepts writes, and the count flows back through status.
    create_account_impl(&state, create_input("Checking")).unwrap();
    assert_eq!(vault_status_impl(&state).unwrap().account_count, Some(1));
}

/// personal-cfo-q329, ADR 0021 addendum: a freshly created vault captures the MACHINE's
/// IANA zone as its initial `household_timezone` (an acceptable initial default, never the
/// authoritative value — that's the stored column from here on) rather than sitting at the
/// bare `UTC` default forever. Compares against `iana_time_zone::get_timezone()` called
/// directly — the same OS call `create_vault_impl` makes internally — so this test's
/// expectation is whatever this machine actually reports, not a hardcoded zone name.
#[test]
fn create_captures_the_machine_timezone_as_the_initial_household_timezone() {
    let (_dir, state) = empty_state();
    create_vault_impl(&state, PASSWORD.to_owned()).unwrap();
    let stored = household_timezone_impl(&state).unwrap();

    match iana_time_zone::get_timezone() {
        Ok(machine_tz) => assert_eq!(
            stored, machine_tz,
            "a successful OS capture must be written as the initial timezone"
        ),
        // No IANA zone nameable on this machine/CI runner — the documented fallback.
        Err(_) => assert_eq!(stored, "UTC"),
    }
}

#[test]
fn create_lock_unlock_round_trips_and_persists() {
    let (_dir, state) = empty_state();
    create_vault_impl(&state, PASSWORD.to_owned()).unwrap();
    create_account_impl(&state, create_input("Savings")).unwrap();

    let locked = lock_vault_impl(&state).unwrap();
    assert_eq!(locked.state, VaultStateDto::Locked);
    assert_eq!(locked.account_count, None);
    // While locked, account commands are refused.
    assert!(matches!(
        account_count_impl(&state).unwrap_err(),
        IpcError::VaultLocked
    ));

    let unlocked = unlock_vault_impl(&state, PASSWORD.to_owned()).unwrap();
    assert_eq!(unlocked.state, VaultStateDto::Unlocked);
    // The account written before locking survived the round trip.
    assert_eq!(unlocked.account_count, Some(1));
}

#[test]
fn wrong_password_is_rejected_and_leaves_the_vault_locked() {
    let (_dir, state) = empty_state();
    create_vault_impl(&state, PASSWORD.to_owned()).unwrap();
    lock_vault_impl(&state).unwrap();

    let err = unlock_vault_impl(&state, "the wrong password".to_owned()).unwrap_err();
    assert!(matches!(err, IpcError::VaultUnlockFailed));

    // Still locked — a failed unlock never opens the vault.
    assert_eq!(
        vault_status_impl(&state).unwrap().state,
        VaultStateDto::Locked
    );

    // The correct password still works afterwards.
    let status = unlock_vault_impl(&state, PASSWORD.to_owned()).unwrap();
    assert_eq!(status.state, VaultStateDto::Unlocked);
}

#[test]
fn change_password_rotates_the_unlock_credential() {
    let (_dir, state) = empty_state();
    create_vault_impl(&state, PASSWORD.to_owned()).unwrap();
    create_account_impl(&state, create_input("Checking")).unwrap();

    // The change keeps the vault open (same DEK, nothing re-encrypted).
    let status = change_password_impl(&state, change_input(PASSWORD, NEW_PASSWORD)).unwrap();
    assert_eq!(status.state, VaultStateDto::Unlocked);
    assert_eq!(status.account_count, Some(1));

    // After a lock: the old password fails, the new one opens the same data.
    lock_vault_impl(&state).unwrap();
    assert!(matches!(
        unlock_vault_impl(&state, PASSWORD.to_owned()).unwrap_err(),
        IpcError::VaultUnlockFailed
    ));
    let unlocked = unlock_vault_impl(&state, NEW_PASSWORD.to_owned()).unwrap();
    assert_eq!(unlocked.account_count, Some(1));
}

#[test]
fn change_password_rejects_a_wrong_current_password() {
    let (_dir, state) = empty_state();
    create_vault_impl(&state, PASSWORD.to_owned()).unwrap();

    let err =
        change_password_impl(&state, change_input("not the password", NEW_PASSWORD)).unwrap_err();
    assert!(matches!(err, IpcError::VaultUnlockFailed));

    // Untouched: still unlocked now, and the original password still works.
    assert_eq!(
        vault_status_impl(&state).unwrap().state,
        VaultStateDto::Unlocked
    );
    lock_vault_impl(&state).unwrap();
    unlock_vault_impl(&state, PASSWORD.to_owned()).unwrap();
}

#[test]
fn change_password_rejects_a_short_new_password() {
    let (_dir, state) = empty_state();
    create_vault_impl(&state, PASSWORD.to_owned()).unwrap();

    let err = change_password_impl(&state, change_input(PASSWORD, "short")).unwrap_err();
    assert!(matches!(err, IpcError::Validation(_)));

    // Refused before touching the envelope: the original password still works.
    lock_vault_impl(&state).unwrap();
    unlock_vault_impl(&state, PASSWORD.to_owned()).unwrap();
}

#[test]
fn change_password_requires_an_unlocked_vault() {
    let (_dir, state) = empty_state();
    create_vault_impl(&state, PASSWORD.to_owned()).unwrap();
    lock_vault_impl(&state).unwrap();

    // An illegal transition surfaces as a (non-leaky) persistence error.
    assert!(matches!(
        change_password_impl(&state, change_input(PASSWORD, NEW_PASSWORD)).unwrap_err(),
        IpcError::Persistence(_)
    ));
    unlock_vault_impl(&state, PASSWORD.to_owned()).unwrap();
}

/// Number of persisted forecast runs, read through the controller's kernel.
fn persisted_run_count(state: &AppState) -> u64 {
    let guard = state.lock_controller().unwrap();
    guard
        .kernel()
        .unwrap()
        .persisted_forecast_run_count()
        .unwrap()
}

#[test]
fn unlock_persists_a_daily_forecast_run_then_dedups() {
    // Daily-on-open activation (ADR 0026 §15, personal-cfo-5ie.3): unlocking the
    // vault persists exactly one forecast run for the day; re-opening with the same
    // inputs the same day dedups to a no-op.
    let (_dir, state) = empty_state();
    create_vault_impl(&state, PASSWORD.to_owned()).unwrap();
    // `create` does not trigger the on-open persist — only `unlock` does.
    assert_eq!(persisted_run_count(&state), 0);
    create_account_impl(&state, create_input("Checking")).unwrap();

    lock_vault_impl(&state).unwrap();
    unlock_vault_impl(&state, PASSWORD.to_owned()).unwrap();
    assert_eq!(
        persisted_run_count(&state),
        1,
        "unlock should persist one forecast run"
    );

    // A second open the same day with unchanged inputs writes no new run.
    lock_vault_impl(&state).unwrap();
    unlock_vault_impl(&state, PASSWORD.to_owned()).unwrap();
    assert_eq!(
        persisted_run_count(&state),
        1,
        "re-open with unchanged inputs same day should dedup"
    );
}
