//! The generic host hooks (personal-cfo-i93n3.13): `on_ready`, `after_unlock`,
//! `unlock_integrity_check` on `HostOptions`, the host-callable `lock_active_vault`, and the typed
//! `VaultIntegrityCheckFailed` error. Synthetic vaults only; nothing here names a platform.

use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use app_lib::ipc::commands::{
    create_vault_impl, lock_active_vault_in, lock_vault_impl, unlock_vault_impl,
    unlock_vault_with_options_impl, vault_status_impl,
};
use app_lib::ipc::dto::VaultStateDto;
use app_lib::ipc::IpcError;
use app_lib::{call_host_hook, AppState, HostOptions};
use finance_kernel::{UnlockOptions, VaultController};
use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri::Manager;
use tempfile::tempdir;

const PASSWORD: &str = "synthetic host hook password";
const LIB_RS: &str = include_str!("../src/lib.rs");

fn state_at(db: &Path) -> AppState {
    AppState::new(VaultController::open(db))
}

fn locked_vault(dir: &Path) -> AppState {
    let state = state_at(&dir.join("vault.db"));
    create_vault_impl(&state, PASSWORD.to_owned()).expect("create");
    lock_vault_impl(&state).expect("lock");
    state
}

/// Damage every page after the first (as the engine's own test does): page 1 carries the header
/// and schema root, so damaging it reads as a wrong key, and which failure SQLite reports for the
/// others depends on what each page holds.
fn damage_a_later_page(db: &Path) {
    let mut bytes = fs::read(db).unwrap();
    assert!(bytes.len() > 4096 * 4, "the vault has several pages");
    let mut page = 1;
    while (page + 1) * 4096 <= bytes.len() {
        for byte in &mut bytes[page * 4096 + 64..page * 4096 + 192] {
            *byte ^= 0xA5;
        }
        page += 1;
    }
    fs::write(db, bytes).unwrap();
}

// ---- the hooks fire when set and do nothing when None --------------------------------

static READY_CALLS: AtomicUsize = AtomicUsize::new(0);
static UNLOCK_CALLS: AtomicUsize = AtomicUsize::new(0);

fn count_ready(_: &u32) {
    READY_CALLS.fetch_add(1, Ordering::SeqCst);
}
fn count_unlock(_: &u32) {
    UNLOCK_CALLS.fetch_add(1, Ordering::SeqCst);
}

#[test]
fn a_host_hook_fires_when_set_and_nothing_happens_when_it_is_none() {
    // `call_host_hook` is the function the app uses for `on_ready` and `after_unlock`.
    let argument = 7_u32;
    let none: Option<fn(&u32)> = None;
    call_host_hook(none, &argument);
    assert_eq!(READY_CALLS.load(Ordering::SeqCst), 0);
    assert_eq!(UNLOCK_CALLS.load(Ordering::SeqCst), 0);

    call_host_hook(Some(count_ready as fn(&u32)), &argument);
    assert_eq!(READY_CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(UNLOCK_CALLS.load(Ordering::SeqCst), 0);
    call_host_hook(Some(count_unlock as fn(&u32)), &argument);
    assert_eq!(UNLOCK_CALLS.load(Ordering::SeqCst), 1);
}

#[test]
fn the_app_calls_its_hooks_through_that_function_and_only_from_the_places_planned() {
    // on_ready: once, in setup. after_unlock: in the unlock command (checked in commands.rs).
    assert_eq!(LIB_RS.matches("call_host_hook(options.on_ready").count(), 1);
    let commands = include_str!("../src/ipc/commands.rs");
    assert_eq!(
        commands.matches("call_host_hook(host.after_unlock").count(),
        1
    );
    // The unlock command asks the host before it unlocks.
    assert!(commands.contains("host.unlock_options()"));
}

// ---- the integrity-check decision ---------------------------------------------------

fn always() -> bool {
    true
}
fn never() -> bool {
    false
}

#[test]
fn the_unlock_options_follow_the_hosts_answer() {
    let none = HostOptions::default();
    assert!(
        !none.unlock_options().integrity_check,
        "desktop: never checks"
    );

    let yes = HostOptions {
        unlock_integrity_check: Some(always),
        ..HostOptions::default()
    };
    assert!(yes.unlock_options().integrity_check);
    assert_eq!(yes.unlock_options(), UnlockOptions::with_integrity_check());

    let no = HostOptions {
        unlock_integrity_check: Some(never),
        ..HostOptions::default()
    };
    assert!(!no.unlock_options().integrity_check);
}

#[test]
fn the_integrity_check_reaches_the_kernel_and_a_damaged_vault_is_refused_untouched() {
    let dir = tempdir().unwrap();
    let state = locked_vault(dir.path());
    let db = dir.path().join("vault.db");
    damage_a_later_page(&db);
    let before = fs::read(&db).unwrap();

    let refused = unlock_vault_with_options_impl(
        &state,
        PASSWORD.to_owned(),
        UnlockOptions::with_integrity_check(),
    );
    assert!(
        matches!(refused, Err(IpcError::VaultIntegrityCheckFailed)),
        "{refused:?}"
    );
    // Nothing was opened or written, and the vault is still locked.
    assert_eq!(fs::read(&db).unwrap(), before, "the file is byte-identical");
    assert_eq!(
        vault_status_impl(&state).unwrap().state,
        VaultStateDto::Locked
    );
}

#[test]
fn without_the_option_the_same_damage_is_not_reported_as_an_integrity_failure() {
    // The control: the check runs only when the host asks, so desktop behavior is unchanged.
    let dir = tempdir().unwrap();
    let state = locked_vault(dir.path());
    damage_a_later_page(&dir.path().join("vault.db"));
    let result = unlock_vault_impl(&state, PASSWORD.to_owned());
    assert!(
        !matches!(result, Err(IpcError::VaultIntegrityCheckFailed)),
        "{result:?}"
    );
}

#[test]
fn a_sound_vault_passes_the_check_and_unlocks() {
    let dir = tempdir().unwrap();
    let state = locked_vault(dir.path());
    let status = unlock_vault_with_options_impl(
        &state,
        PASSWORD.to_owned(),
        UnlockOptions::with_integrity_check(),
    )
    .expect("a sound vault unlocks with the check on");
    assert_eq!(status.state, VaultStateDto::Unlocked);
}

#[test]
fn a_wrong_password_with_the_check_on_is_still_a_wrong_password() {
    let dir = tempdir().unwrap();
    let state = locked_vault(dir.path());
    let refused = unlock_vault_with_options_impl(
        &state,
        "not the password".to_owned(),
        UnlockOptions::with_integrity_check(),
    );
    assert!(
        matches!(refused, Err(IpcError::VaultUnlockFailed)),
        "{refused:?}"
    );
}

// ---- lock_active_vault --------------------------------------------------------------

fn mock_app_with(state: AppState) -> tauri::App<tauri::test::MockRuntime> {
    let app = mock_builder()
        .build(mock_context(noop_assets()))
        .expect("mock app");
    app.manage(state);
    app
}

#[test]
fn lock_active_vault_locks_leaves_no_wal_content_and_is_idempotent() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("vault.db");
    let state = state_at(&db);
    create_vault_impl(&state, PASSWORD.to_owned()).expect("create");
    let app = mock_app_with(state);
    let handle = app.handle();

    lock_active_vault_in(handle).expect("locks an unlocked vault");
    let state = handle.state::<AppState>();
    assert_eq!(
        vault_status_impl(state.inner()).unwrap().state,
        VaultStateDto::Locked
    );
    let wal = dir.path().join("vault.db-wal");
    let wal_len = fs::metadata(&wal).map_or(0, |m| m.len());
    assert_eq!(wal_len, 0, "no -wal content after the host lock");

    // A host may call it at every lifecycle event: a vault that is not unlocked is left alone.
    lock_active_vault_in(handle).expect("a second call is a no-op");
    assert_eq!(
        vault_status_impl(state.inner()).unwrap().state,
        VaultStateDto::Locked
    );
}

#[test]
fn lock_active_vault_is_not_an_ipc_command() {
    // It is a plain function: not in the command list, so no webview can call it and no
    // capability can grant it.
    assert!(!LIB_RS.contains("ipc::commands::lock_active_vault,"));
    let permissions = include_str!("../permissions/app-commands.toml");
    let destructive = include_str!("../permissions/destructive-commands.toml");
    assert!(!permissions.contains("lock_active_vault"));
    assert!(!destructive.contains("lock_active_vault"));
}
