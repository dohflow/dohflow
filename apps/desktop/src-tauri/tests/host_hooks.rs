//! The generic host hooks (personal-cfo-i93n3.13, i93n3.14): `on_ready`, `on_unlocked`,
//! `unlock_integrity_check` on `HostOptions`, the host-callable `lock_active_vault`, and the typed
//! `VaultIntegrityCheckFailed` error. Synthetic vaults only; nothing here names a platform.

use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use app_lib::ipc::commands::{
    active_vault_id, call_if_unlocked, create_vault_impl, create_vault_named_impl,
    export_backup_impl, list_vaults_impl, lock_active_vault_in, lock_vault_impl,
    restore_backup_as_new_vault_impl, restore_backup_impl, switch_vault_impl, unlock_vault_impl,
    unlock_vault_with_options_impl, vault_status_impl,
};
use app_lib::ipc::dto::VaultStateDto;
use app_lib::ipc::IpcError;
use app_lib::vault_registry::VaultRegistry;
use app_lib::{call_host_hook, AppState, HostOptions, HostRegistry};
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
    // `call_host_hook` is the function the app uses for `on_ready` and (through
    // `call_if_unlocked`) `on_unlocked`.
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
fn the_app_calls_its_hooks_through_those_functions_and_only_from_the_places_planned() {
    // on_ready: once, in setup.
    assert_eq!(LIB_RS.matches("call_host_hook(options.on_ready").count(), 1);
    let commands = include_str!("../src/ipc/commands.rs");
    // on_unlocked: from each of the five commands that can end the vault Unlocked, and no other.
    assert_eq!(
        commands.matches("call_if_unlocked(host.on_unlocked").count(),
        5,
        "create_vault, unlock_vault, create_vault_named, restore_backup, restore_backup_as_new_vault"
    );
    for wrapper in [
        "pub fn create_vault(",
        "pub fn unlock_vault(",
        "pub fn create_vault_named(",
        "pub fn restore_backup(",
        "pub fn restore_backup_as_new_vault(",
    ] {
        let start = commands
            .find(wrapper)
            .unwrap_or_else(|| panic!("{wrapper} exists"));
        let body_end = commands[start..]
            .find("\n}\n")
            .map_or(commands.len(), |e| start + e);
        assert!(
            commands[start..body_end].contains("call_if_unlocked(host.on_unlocked"),
            "{wrapper} must route its outcome through call_if_unlocked"
        );
    }
    // The unlock command asks the host before it unlocks.
    assert!(commands.contains("host.unlock_options(active_vault_id(state.inner()).as_deref())"));
}

// ---- on_unlocked: exactly once per transition to Unlocked, never on a failure ----------------

thread_local! {
    static UNLOCKED_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

thread_local! {
    static UNLOCKED_IDS: std::cell::RefCell<Vec<Option<String>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

fn bump(_: &u32, vault_id: Option<&str>) {
    UNLOCKED_CALLS.with(|calls| calls.set(calls.get() + 1));
    UNLOCKED_IDS.with(|ids| ids.borrow_mut().push(vault_id.map(str::to_owned)));
}

fn seen_ids() -> Vec<Option<String>> {
    UNLOCKED_IDS.with(|ids| ids.borrow().clone())
}

fn calls() -> usize {
    UNLOCKED_CALLS.with(std::cell::Cell::get)
}

/// What the commands do with their result: route it through the one helper.
fn fire(outcome: &Result<app_lib::ipc::dto::VaultStatusDto, IpcError>) {
    call_if_unlocked(
        Some(bump as fn(&u32, Option<&str>)),
        &0_u32,
        outcome,
        || Some("a-vault-id".to_owned()),
    );
}

fn status(state: VaultStateDto) -> Result<app_lib::ipc::dto::VaultStatusDto, IpcError> {
    Ok(app_lib::ipc::dto::VaultStatusDto {
        state,
        account_count: None,
    })
}

#[test]
fn on_unlocked_fires_only_for_a_successful_transition_to_unlocked() {
    fire(&status(VaultStateDto::Unlocked));
    assert_eq!(calls(), 1, "an Unlocked result fires once");
    for other in [
        VaultStateDto::NoVault,
        VaultStateDto::Locked,
        VaultStateDto::CorruptNeedsRecovery,
        VaultStateDto::RestoringBackup,
    ] {
        fire(&status(other));
    }
    for failure in [
        IpcError::VaultUnlockFailed,
        IpcError::VaultIntegrityCheckFailed,
        IpcError::VaultLocked,
        IpcError::Validation("x".to_owned()),
    ] {
        fire(&Err(failure));
    }
    assert_eq!(calls(), 1, "no other state and no failure fires it");
    // And inert when the host set none.
    call_if_unlocked::<u32>(None, &0, &status(VaultStateDto::Unlocked), || None);
    assert_eq!(calls(), 1);
}

fn export_package(dir: &Path) -> (AppState, String) {
    let state = state_at(&dir.join("vault.db"));
    create_vault_impl(&state, PASSWORD.to_owned()).expect("create source");
    let package = dir.join("backup.pcfobk");
    export_backup_impl(&state, package.to_string_lossy().into_owned()).expect("export");
    (state, package.to_string_lossy().into_owned())
}

#[test]
fn create_and_unlock_fire_once_each_and_a_failed_unlock_fires_nothing() {
    let dir = tempdir().unwrap();
    let state = state_at(&dir.path().join("vault.db"));

    fire(&create_vault_impl(&state, PASSWORD.to_owned()));
    assert_eq!(calls(), 1, "create");
    lock_vault_impl(&state).unwrap();

    fire(&unlock_vault_with_options_impl(
        &state,
        "wrong password".to_owned(),
        UnlockOptions::default(),
    ));
    assert_eq!(calls(), 1, "a wrong password fires nothing");

    fire(&unlock_vault_impl(&state, PASSWORD.to_owned()));
    assert_eq!(calls(), 2, "a password unlock");
}

#[test]
fn a_failed_integrity_check_fires_nothing() {
    let dir = tempdir().unwrap();
    let state = locked_vault(dir.path());
    damage_a_later_page(&dir.path().join("vault.db"));
    fire(&unlock_vault_with_options_impl(
        &state,
        PASSWORD.to_owned(),
        UnlockOptions::with_integrity_check(),
    ));
    assert_eq!(calls(), 0);
}

#[test]
fn a_fresh_restore_fires_once_and_a_failed_restore_fires_nothing() {
    let source = tempdir().unwrap();
    let (_state, package) = export_package(source.path());

    let target = tempdir().unwrap();
    let fresh = state_at(&target.path().join("vault.db"));
    fire(&restore_backup_impl(
        &fresh,
        package.clone(),
        "wrong password".to_owned(),
    ));
    assert_eq!(calls(), 0, "a wrong backup password fires nothing");

    fire(&restore_backup_impl(&fresh, package, PASSWORD.to_owned()));
    assert_eq!(calls(), 1, "a restore into a fresh location");
}

#[test]
fn a_named_vault_and_a_restore_as_new_fire_once_and_a_failed_restore_fires_nothing() {
    let dir = tempdir().unwrap();
    let registry = VaultRegistry::load(dir.path());
    let path = registry
        .active_path(dir.path())
        .unwrap_or_else(|| dir.path().join("vault.db"));
    let state = AppState::with_registry(
        VaultController::open(path),
        registry,
        dir.path().to_path_buf(),
    );

    fire(&create_vault_named_impl(
        &state,
        "Original".to_owned(),
        PASSWORD.to_owned(),
    ));
    assert_eq!(calls(), 1, "create_vault_named ends Unlocked");

    let package = dir.path().join("named-backup.pcfobk");
    export_backup_impl(&state, package.to_string_lossy().into_owned()).expect("export");
    lock_vault_impl(&state).unwrap();

    fire(&restore_backup_as_new_vault_impl(
        &state,
        package.to_string_lossy().into_owned(),
        "wrong password".to_owned(),
        "Recovered".to_owned(),
    ));
    assert_eq!(calls(), 1, "a failed restore-as-new fires nothing");

    fire(&restore_backup_as_new_vault_impl(
        &state,
        package.to_string_lossy().into_owned(),
        PASSWORD.to_owned(),
        "Recovered".to_owned(),
    ));
    assert_eq!(calls(), 2, "restore-as-new from the locked state");
}

#[test]
fn switching_vaults_does_not_fire_it_because_the_target_comes_up_locked() {
    let dir = tempdir().unwrap();
    let registry = VaultRegistry::load(dir.path());
    let path = registry
        .active_path(dir.path())
        .unwrap_or_else(|| dir.path().join("vault.db"));
    let state = AppState::with_registry(
        VaultController::open(path),
        registry,
        dir.path().to_path_buf(),
    );
    create_vault_named_impl(&state, "One".to_owned(), PASSWORD.to_owned()).unwrap();
    create_vault_named_impl(&state, "Two".to_owned(), PASSWORD.to_owned()).unwrap();
    let first = list_vaults_impl(&state).unwrap().vaults[0].id.clone();
    let switched = switch_vault_impl(&state, first);
    assert!(
        matches!(&switched, Ok(s) if s.state == VaultStateDto::Locked),
        "{switched:?}"
    );
    fire(&switched);
    assert_eq!(calls(), 0);
}

// ---- the integrity-check decision ---------------------------------------------------

fn always(_: Option<&str>) -> bool {
    true
}
fn never(_: Option<&str>) -> bool {
    false
}

#[test]
fn the_unlock_options_follow_the_hosts_answer() {
    let none = HostOptions::default();
    assert!(
        !none.unlock_options(Some("any")).integrity_check,
        "desktop: never checks"
    );

    let yes = HostOptions {
        unlock_integrity_check: Some(always),
        ..HostOptions::default()
    };
    assert!(yes.unlock_options(Some("any")).integrity_check);
    assert_eq!(
        yes.unlock_options(None),
        UnlockOptions::with_integrity_check()
    );

    let no = HostOptions {
        unlock_integrity_check: Some(never),
        ..HostOptions::default()
    };
    assert!(!no.unlock_options(Some("any")).integrity_check);
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

// ---- register_host_options (personal-cfo-i93n3.15) ----------------------------------------

fn marked(device: &'static str) -> HostOptions {
    HostOptions {
        system_backup_device: Some(device),
        ..HostOptions::default()
    }
}

#[test]
fn registered_options_are_what_run_resolves_and_the_default_when_none() {
    let none = HostRegistry::new();
    assert!(
        none.resolve().system_backup_device.is_none(),
        "default when none"
    );

    let some = HostRegistry::new();
    some.register(marked("Test Device"))
        .expect("the first registration");
    assert_eq!(some.resolve().system_backup_device, Some("Test Device"));
}

#[test]
fn a_second_registration_is_refused_and_changes_nothing() {
    let registry = HostRegistry::new();
    registry.register(marked("First")).expect("first");
    let refused = registry
        .register(marked("Second"))
        .expect_err("second is refused");
    assert_eq!(
        refused.system_backup_device,
        Some("Second"),
        "handed back unchanged"
    );
    assert_eq!(registry.resolve().system_backup_device, Some("First"));
}

#[test]
fn a_registration_after_run_resolved_its_options_is_refused() {
    let registry = HostRegistry::new();
    let resolved = registry.resolve();
    assert!(resolved.system_backup_device.is_none());
    assert!(
        registry.register(marked("Late")).is_err(),
        "too late: the options are in use"
    );
    // And it stays the defaults afterwards.
    assert!(registry.resolve().system_backup_device.is_none());
}

#[test]
fn run_resolves_through_the_registry_and_the_desktop_never_registers() {
    // `run` is the only caller of resolve; nothing but a host calls register_host_options.
    assert_eq!(LIB_RS.matches("HOST_REGISTRY.resolve()").count(), 1);
    assert_eq!(
        LIB_RS.matches("register_host_options(").count(),
        1,
        "only its definition"
    );
}

// ---- the hooks carry the vault id (personal-cfo-i93n3.18) ---------------------------------

fn registry_state(root: &Path) -> AppState {
    let registry = VaultRegistry::load(root);
    let path = registry
        .active_path(root)
        .unwrap_or_else(|| root.join("vault.db"));
    AppState::with_registry(VaultController::open(path), registry, root.to_path_buf())
}

fn vault_file(root: &Path, id: &str) -> std::path::PathBuf {
    let registry = VaultRegistry::load(root);
    let entry = registry
        .vaults
        .iter()
        .find(|v| v.id.to_string() == id)
        .expect("the vault is registered");
    root.join(&entry.path)
}

thread_local! {
    static TO_CHECK: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// A host that checks exactly the vaults it was told to, and every vault it cannot identify.
fn check_listed_or_unknown(vault_id: Option<&str>) -> bool {
    match vault_id {
        None => true,
        Some(id) => TO_CHECK.with(|list| list.borrow().iter().any(|listed| listed == id)),
    }
}

#[test]
fn on_unlocked_receives_each_vaults_own_registry_id_across_two_vaults() {
    let dir = tempdir().unwrap();
    let state = registry_state(dir.path());

    fire_with_id(
        &state,
        create_vault_named_impl(&state, "One".to_owned(), PASSWORD.to_owned()),
    );
    let one = active_vault_id(&state).expect("a registered vault has an id");
    fire_with_id(
        &state,
        create_vault_named_impl(&state, "Two".to_owned(), PASSWORD.to_owned()),
    );
    let two = active_vault_id(&state).expect("a registered vault has an id");
    assert_ne!(one, two, "two vaults, two ids");

    let package = dir.path().join("one.pcfobk");
    switch_vault_impl(&state, one.clone()).unwrap();
    unlock_vault_impl(&state, PASSWORD.to_owned()).unwrap();
    export_backup_impl(&state, package.to_string_lossy().into_owned()).unwrap();
    lock_vault_impl(&state).unwrap();
    fire_with_id(
        &state,
        restore_backup_as_new_vault_impl(
            &state,
            package.to_string_lossy().into_owned(),
            PASSWORD.to_owned(),
            "Restored".to_owned(),
        ),
    );
    let restored = active_vault_id(&state).expect("a registered vault has an id");
    assert!(
        restored != one && restored != two,
        "the restored vault is a third vault"
    );

    // Each transition delivered the id of the vault that became unlocked, in order.
    assert_eq!(
        seen_ids(),
        vec![Some(one.clone()), Some(two.clone()), Some(restored)]
    );
    // And it is the picker's identifier, a UUID in text form: not a path or a name.
    for id in [&one, &two] {
        assert!(uuid::Uuid::parse_str(id).is_ok(), "{id}");
        assert!(!id.contains('/') && !id.contains("One") && !id.contains("Two"));
    }
}

fn fire_with_id(state: &AppState, outcome: Result<app_lib::ipc::dto::VaultStatusDto, IpcError>) {
    call_if_unlocked(
        Some(bump as fn(&u32, Option<&str>)),
        &0_u32,
        &outcome,
        || active_vault_id(state),
    );
}

#[test]
fn the_integrity_decision_is_per_vault_and_reaches_the_kernel() {
    let dir = tempdir().unwrap();
    let state = registry_state(dir.path());
    create_vault_named_impl(&state, "One".to_owned(), PASSWORD.to_owned()).unwrap();
    let one = active_vault_id(&state).unwrap();
    create_vault_named_impl(&state, "Two".to_owned(), PASSWORD.to_owned()).unwrap();
    let two = active_vault_id(&state).unwrap();
    lock_vault_impl(&state).unwrap();
    // Damage BOTH vaults' files; the host asks for a check on One only.
    damage_a_later_page(&vault_file(dir.path(), &one));
    damage_a_later_page(&vault_file(dir.path(), &two));
    TO_CHECK.with(|list| *list.borrow_mut() = vec![one.clone()]);
    let host = HostOptions {
        unlock_integrity_check: Some(check_listed_or_unknown),
        ..HostOptions::default()
    };

    switch_vault_impl(&state, one).unwrap();
    let id = active_vault_id(&state);
    assert!(host.unlock_options(id.as_deref()).integrity_check);
    let refused = unlock_vault_with_options_impl(
        &state,
        PASSWORD.to_owned(),
        host.unlock_options(id.as_deref()),
    );
    assert!(
        matches!(refused, Err(IpcError::VaultIntegrityCheckFailed)),
        "{refused:?}"
    );

    switch_vault_impl(&state, two).unwrap();
    let id = active_vault_id(&state);
    assert!(
        !host.unlock_options(id.as_deref()).integrity_check,
        "the host did not ask for Two, so the kernel is not told to check it"
    );
    let opened = unlock_vault_with_options_impl(
        &state,
        PASSWORD.to_owned(),
        host.unlock_options(id.as_deref()),
    );
    assert!(
        !matches!(opened, Err(IpcError::VaultIntegrityCheckFailed)),
        "no check was requested for Two: {opened:?}"
    );
}

#[test]
fn a_vault_with_no_registry_id_reports_none_and_a_host_that_checks_unknowns_checks_it() {
    // The legacy layout (no registry entry): the id is None, and the documented host rule is that
    // None means "run the check".
    let dir = tempdir().unwrap();
    let state = locked_vault(dir.path());
    assert_eq!(active_vault_id(&state), None);
    damage_a_later_page(&dir.path().join("vault.db"));
    let host = HostOptions {
        unlock_integrity_check: Some(check_listed_or_unknown),
        ..HostOptions::default()
    };
    let options = host.unlock_options(active_vault_id(&state).as_deref());
    assert!(options.integrity_check, "None is checked");
    let refused = unlock_vault_with_options_impl(&state, PASSWORD.to_owned(), options);
    assert!(
        matches!(refused, Err(IpcError::VaultIntegrityCheckFailed)),
        "{refused:?}"
    );
    // And the hook receives None for such a vault.
    fire_with_id(
        &state,
        Ok(app_lib::ipc::dto::VaultStatusDto {
            state: VaultStateDto::Unlocked,
            account_count: None,
        }),
    );
    assert_eq!(seen_ids().last(), Some(&None));
}
