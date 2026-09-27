//! Restore-as-new IPC regressions (personal-cfo-g3m.3). Every test uses synthetic vaults and
//! real backup crypto; the original slot is snapshotted to catch unintended writes.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use app_lib::ipc::commands::{
    create_account_impl, create_vault_named_impl, export_backup_impl, list_vaults_impl,
    lock_vault_impl, restore_backup_as_new_vault_impl, switch_vault_impl, unlock_vault_impl,
};
use app_lib::ipc::dto::{
    AccountFlagsDto, CashflowRoleDto, CreateAccountInput, RestoreRecoveryStatusDto, VaultStateDto,
};
use app_lib::ipc::IpcError;
use app_lib::vault_registry::{VaultEntry, VaultRegistry};
use app_lib::AppState;
use finance_kernel::{
    Account, AccountFlags, AccountId, ActorType, AttachmentId, CashflowRole, CommandEnvelope,
    CommandMeta, CreateAccount, Currency, LedgerAccountId, Money, RecordTransaction, TransactionId,
    VaultController,
};
use tempfile::TempDir;
use uuid::Uuid;

const PASSWORD: &str = "synthetic restore password";
const V1_PASSWORD: &str = "checked-in v1 compatibility fixture password";
const V1_FIXTURE: &[u8] =
    include_bytes!("../../../../crates/finance-kernel/tests/fixtures/backup-v1-minimal.pcfobk");

fn state(root: &Path) -> AppState {
    let registry = VaultRegistry::load(root);
    let path = registry
        .active_path(root)
        .unwrap_or_else(|| root.join("vault.db"));
    AppState::with_registry(VaultController::open(path), registry, root.to_path_buf())
}

fn account_input(name: &str) -> CreateAccountInput {
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

fn meta() -> CommandMeta {
    CommandMeta {
        command_id: Uuid::now_v7(),
        correlation_id: Uuid::now_v7(),
        causation_id: None,
        actor_type: ActorType::User,
        actor_id: "restore test".to_owned(),
        idempotency_key: Uuid::now_v7().to_string(),
    }
}

fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, current: &Path, result: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(current).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                walk(root, &path, result);
            } else {
                result.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    walk(dir, dir, &mut result);
    result
}

fn original_slot(root: &Path) -> (Uuid, PathBuf) {
    let registry = VaultRegistry::load(root);
    let id = registry.active.unwrap();
    let path = registry.active_path(root).unwrap();
    (id, path.parent().unwrap().to_path_buf())
}

fn make_v2_source() -> (
    TempDir,
    AppState,
    PathBuf,
    Uuid,
    PathBuf,
    AttachmentId,
    Vec<u8>,
) {
    let dir = TempDir::new().unwrap();
    let state = state(dir.path());
    create_vault_named_impl(&state, "Original".to_owned(), PASSWORD.to_owned()).unwrap();
    let (original_id, original_slot) = original_slot(dir.path());
    let pdf = b"%PDF-1.4\nsynthetic-restore-attachment\n%%EOF\n".to_vec();
    let attachment_id = {
        let guard = state.lock_controller().unwrap();
        let kernel = guard.kernel().unwrap();
        let account = Account::new(
            AccountId::new(),
            LedgerAccountId::new(),
            "Checking",
            CashflowRole::LiquidCash,
            Currency::Usd,
            AccountFlags::default(),
        );
        let account_id = account.id();
        kernel
            .dispatch(CommandEnvelope::new(
                meta(),
                CreateAccount::with_opening_balance(account, Money::new(50_000, Currency::Usd)),
            ))
            .unwrap();
        let transaction_id = TransactionId::new();
        kernel
            .dispatch(CommandEnvelope::new(
                meta(),
                RecordTransaction::new(
                    transaction_id,
                    account_id,
                    Money::new(-1_234, Currency::Usd),
                    "2026-09-01T00:00:00Z".parse().unwrap(),
                ),
            ))
            .unwrap();
        kernel
            .attach_document(
                transaction_id,
                &pdf,
                Some("application/pdf"),
                Some("receipt.pdf"),
            )
            .unwrap()
            .id
    };
    let package = dir.path().join("source.pcfobk");
    export_backup_impl(&state, package.to_string_lossy().into_owned()).unwrap();
    lock_vault_impl(&state).unwrap();
    (
        dir,
        state,
        package,
        original_id,
        original_slot,
        attachment_id,
        pdf,
    )
}

#[test]
fn locked_v2_restore_registers_a_new_unlocked_vault_and_preserves_original_bytes() {
    let (dir, state, package, original_id, original_slot, attachment_id, pdf) = make_v2_source();
    let before = snapshot(&original_slot);
    let old_registry = VaultRegistry::load(dir.path());

    let restored = restore_backup_as_new_vault_impl(
        &state,
        package.to_string_lossy().into_owned(),
        PASSWORD.to_owned(),
        "  Recovered  ".to_owned(),
    )
    .unwrap();
    assert_eq!(restored.state, VaultStateDto::Unlocked);
    assert_eq!(restored.account_count, Some(1));
    assert!(
        snapshot(&original_slot) == before,
        "original DB, envelope and blobs changed"
    );

    let registry = VaultRegistry::load(dir.path());
    assert_eq!(registry.vaults.len(), 2);
    assert_eq!(registry.vaults[0], old_registry.vaults[0]);
    assert_ne!(registry.active, Some(original_id));
    assert_eq!(registry.vaults[1].name, "Recovered");
    let list = list_vaults_impl(&state).unwrap();
    assert_eq!(
        list.vaults.iter().filter(|entry| entry.is_active).count(),
        1
    );
    assert_eq!(
        list.restore_recovery_status,
        RestoreRecoveryStatusDto::Clear
    );
    let restored_logical_id = {
        let guard = state.lock_controller().unwrap();
        let kernel = guard.kernel().unwrap();
        assert_eq!(kernel.read_attachment_bytes(attachment_id).unwrap(), pdf);
        assert_eq!(kernel.transactions(10).unwrap().len(), 2);
        kernel.vault_metadata().unwrap().vault_id
    };

    // Switching drops the restored key. Writing to the original cannot leak into the restored
    // read model; switching back requires the restored vault's password again.
    assert_eq!(
        switch_vault_impl(&state, original_id.to_string())
            .unwrap()
            .state,
        VaultStateDto::Locked
    );
    unlock_vault_impl(&state, PASSWORD.to_owned()).unwrap();
    let original_logical_id = state
        .lock_controller()
        .unwrap()
        .kernel()
        .unwrap()
        .vault_metadata()
        .unwrap()
        .vault_id;
    assert_eq!(
        restored_logical_id, original_logical_id,
        "backup restore changed the logical vault id"
    );
    create_account_impl(&state, account_input("Original only")).unwrap();
    assert_eq!(
        switch_vault_impl(&state, registry.active.unwrap().to_string())
            .unwrap()
            .state,
        VaultStateDto::Locked
    );
    assert_eq!(
        unlock_vault_impl(&state, PASSWORD.to_owned())
            .unwrap()
            .account_count,
        Some(1)
    );
}

#[test]
fn wrong_password_and_invalid_packages_leave_selection_and_original_unchanged() {
    let (dir, state, package, original_id, original_slot, _, _) = make_v2_source();
    let before = snapshot(&original_slot);
    let registry_bytes = fs::read(dir.path().join("vaults.json")).unwrap();
    let attempt = |path: &Path, password: &str| {
        restore_backup_as_new_vault_impl(
            &state,
            path.to_string_lossy().into_owned(),
            password.to_owned(),
            "Retry".to_owned(),
        )
    };
    assert!(matches!(
        attempt(&package, "wrong password"),
        Err(IpcError::VaultUnlockFailed)
    ));
    let malformed = dir.path().join("malformed.pcfobk");
    fs::write(&malformed, b"not a backup").unwrap();
    assert!(attempt(&malformed, PASSWORD).is_err());
    let newer = dir.path().join("newer.pcfobk");
    let mut newer_bytes = V1_FIXTURE.to_vec();
    newer_bytes[6..8].copy_from_slice(&u16::MAX.to_be_bytes());
    fs::write(&newer, newer_bytes).unwrap();
    assert!(attempt(&newer, V1_PASSWORD).is_err());
    let tampered = dir.path().join("tampered.pcfobk");
    let mut tampered_bytes = fs::read(&package).unwrap();
    let last = tampered_bytes.len() - 1;
    tampered_bytes[last] ^= 1;
    fs::write(&tampered, tampered_bytes).unwrap();
    assert!(attempt(&tampered, PASSWORD).is_err());

    assert!(
        snapshot(&original_slot) == before,
        "original files changed after rejected packages"
    );
    assert_eq!(
        fs::read(dir.path().join("vaults.json")).unwrap(),
        registry_bytes
    );
    assert_eq!(VaultRegistry::load(dir.path()).active, Some(original_id));
    assert_eq!(
        state.lock_controller().unwrap().state(),
        finance_kernel::VaultState::Locked
    );
    assert_eq!(
        list_vaults_impl(&state).unwrap().restore_recovery_status,
        RestoreRecoveryStatusDto::Clear
    );
}

#[test]
fn registry_save_failure_rolls_back_the_new_slot_and_selection() {
    let (dir, state, package, original_id, original_slot, _, _) = make_v2_source();
    let before = snapshot(&original_slot);
    let registry_file = dir.path().join("vaults.json");
    let saved = dir.path().join("vaults-before.json");
    fs::rename(&registry_file, &saved).unwrap();
    fs::create_dir(&registry_file).unwrap(); // deterministic registry-save failure

    assert!(matches!(
        restore_backup_as_new_vault_impl(
            &state,
            package.to_string_lossy().into_owned(),
            PASSWORD.to_owned(),
            "Retry".to_owned()
        ),
        Err(IpcError::Persistence(_))
    ));
    assert!(
        snapshot(&original_slot) == before,
        "original files changed on registry failure"
    );
    assert_eq!(state.lock_registry().unwrap().active, Some(original_id));
    assert_eq!(
        state.lock_controller().unwrap().state(),
        finance_kernel::VaultState::Locked
    );
    fs::remove_dir(&registry_file).unwrap();
    fs::rename(&saved, &registry_file).unwrap();
    assert_eq!(VaultRegistry::load(dir.path()).active, Some(original_id));
    assert_eq!(
        list_vaults_impl(&state).unwrap().restore_recovery_status,
        RestoreRecoveryStatusDto::Clear
    );
}

#[test]
fn unavailable_destination_leaves_legacy_original_and_selection_untouched() {
    let dir = TempDir::new().unwrap();
    let database = dir.path().join("vault.db");
    let mut controller = VaultController::open(&database);
    controller.create(PASSWORD.as_bytes()).unwrap();
    controller.lock().unwrap();
    let id = Uuid::now_v7();
    let registry = VaultRegistry {
        vaults: vec![VaultEntry {
            id,
            name: "Legacy".to_owned(),
            path: "vault.db".into(),
            created_at: "2026-09-27T00:00:00Z".to_owned(),
        }],
        active: Some(id),
    };
    registry.save(dir.path()).unwrap();
    let state = AppState::with_registry(controller, registry, dir.path().to_path_buf());
    let before_db = fs::read(&database).unwrap();
    let before_envelope = fs::read(dir.path().join("vault.db.envelope")).unwrap();
    let before_registry = fs::read(dir.path().join("vaults.json")).unwrap();
    fs::write(dir.path().join("vaults"), b"occupied app-managed root").unwrap();

    assert!(matches!(
        restore_backup_as_new_vault_impl(
            &state,
            "unused.pcfobk".to_owned(),
            PASSWORD.to_owned(),
            "Restored".to_owned(),
        ),
        Err(IpcError::Persistence(_))
    ));
    assert_eq!(fs::read(&database).unwrap(), before_db);
    assert_eq!(
        fs::read(dir.path().join("vault.db.envelope")).unwrap(),
        before_envelope
    );
    assert_eq!(
        fs::read(dir.path().join("vaults.json")).unwrap(),
        before_registry
    );
    assert_eq!(
        fs::read(dir.path().join("vaults")).unwrap(),
        b"occupied app-managed root"
    );
    assert_eq!(state.lock_registry().unwrap().active, Some(id));
    assert_eq!(
        state.lock_controller().unwrap().state(),
        finance_kernel::VaultState::Locked
    );
}

#[test]
fn damaged_originals_restore_v1_without_replacing_the_remaining_files() {
    for missing in ["vault.db", "vault.db.envelope"] {
        let dir = TempDir::new().unwrap();
        let original = state(dir.path());
        create_vault_named_impl(&original, "Damaged".to_owned(), PASSWORD.to_owned()).unwrap();
        lock_vault_impl(&original).unwrap();
        let (original_id, original_slot) = original_slot(dir.path());
        fs::remove_file(original_slot.join(missing)).unwrap();
        let before = snapshot(&original_slot);
        drop(original);
        let restarted = state(dir.path());
        assert_eq!(
            restarted.lock_controller().unwrap().state(),
            finance_kernel::VaultState::CorruptNeedsRecovery
        );
        let package = dir.path().join("historical-v1.pcfobk");
        fs::write(&package, V1_FIXTURE).unwrap();

        let status = restore_backup_as_new_vault_impl(
            &restarted,
            package.to_string_lossy().into_owned(),
            V1_PASSWORD.to_owned(),
            "Historical".to_owned(),
        )
        .unwrap();
        assert_eq!(status.state, VaultStateDto::Unlocked);
        assert_eq!(status.account_count, Some(0));
        assert!(
            snapshot(&original_slot) == before,
            "damaged original changed: {missing}"
        );
        assert_ne!(VaultRegistry::load(dir.path()).active, Some(original_id));
        assert_eq!(list_vaults_impl(&restarted).unwrap().vaults.len(), 2);
    }
}

#[test]
fn envelope_only_legacy_recovery_works_without_a_preexisting_registry_entry() {
    let dir = TempDir::new().unwrap();
    let original_envelope = dir.path().join("vault.db.envelope");
    fs::write(&original_envelope, b"incomplete legacy envelope").unwrap();
    let state = AppState::with_registry(
        VaultController::open(dir.path().join("vault.db")),
        VaultRegistry::default(),
        dir.path().to_path_buf(),
    );
    assert_eq!(
        state.lock_controller().unwrap().state(),
        finance_kernel::VaultState::CorruptNeedsRecovery
    );
    let package = dir.path().join("historical-v1.pcfobk");
    fs::write(&package, V1_FIXTURE).unwrap();

    let status = restore_backup_as_new_vault_impl(
        &state,
        package.to_string_lossy().into_owned(),
        V1_PASSWORD.to_owned(),
        "Recovered legacy".to_owned(),
    )
    .unwrap();
    assert_eq!(status.state, VaultStateDto::Unlocked);
    assert_eq!(
        fs::read(original_envelope).unwrap(),
        b"incomplete legacy envelope"
    );
    assert!(!dir.path().join("vault.db").exists());
    let registry = VaultRegistry::load(dir.path());
    assert_eq!(registry.vaults.len(), 1);
    assert_eq!(registry.active, Some(registry.vaults[0].id));
    assert_eq!(registry.vaults[0].name, "Recovered legacy");
}

#[test]
fn restart_reports_an_unregistered_attempt_without_opening_or_removing_it() {
    let (dir, original, _, original_id, _, _, _) = make_v2_source();
    let id = Uuid::now_v7();
    let slot = dir.path().join("vaults").join(id.to_string());
    fs::create_dir(&slot).unwrap();
    fs::write(slot.join(".restore-in-progress"), id.to_string()).unwrap();
    fs::write(slot.join("vault.db"), b"incomplete synthetic data").unwrap();
    drop(original);

    let restarted = state(dir.path());
    assert_eq!(VaultRegistry::load(dir.path()).active, Some(original_id));
    assert_eq!(
        restarted.lock_controller().unwrap().state(),
        finance_kernel::VaultState::Locked
    );
    assert_eq!(
        list_vaults_impl(&restarted)
            .unwrap()
            .restore_recovery_status,
        RestoreRecoveryStatusDto::Interrupted
    );
    assert_eq!(
        fs::read(slot.join("vault.db")).unwrap(),
        b"incomplete synthetic data"
    );
}

#[test]
fn invalid_name_or_unlocked_source_is_rejected_before_a_slot_is_created() {
    let (dir, state, package, original_id, original_slot, _, _) = make_v2_source();
    let before = snapshot(&original_slot);
    assert!(matches!(
        restore_backup_as_new_vault_impl(
            &state,
            package.to_string_lossy().into_owned(),
            PASSWORD.to_owned(),
            "   ".to_owned()
        ),
        Err(IpcError::Validation(_))
    ));
    assert!(
        snapshot(&original_slot) == before,
        "blank name changed original files"
    );
    unlock_vault_impl(&state, PASSWORD.to_owned()).unwrap();
    let unlocked_before = snapshot(&original_slot);
    assert!(matches!(
        restore_backup_as_new_vault_impl(
            &state,
            package.to_string_lossy().into_owned(),
            PASSWORD.to_owned(),
            "Other".to_owned()
        ),
        Err(IpcError::Validation(_))
    ));
    assert!(
        snapshot(&original_slot) == unlocked_before,
        "rejected unlocked restore changed files"
    );
    assert_eq!(VaultRegistry::load(dir.path()).active, Some(original_id));
}
