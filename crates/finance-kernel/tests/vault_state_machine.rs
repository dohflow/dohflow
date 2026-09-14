//! Vault state machine + startup self-test (personal-cfo-tg5): legal/illegal
//! transitions, on-disk classification, fault-injection (recovery always lands
//! in Locked or CorruptNeedsRecovery), and the post-unlock health check.

use finance_kernel::{
    classify_vault, Account, AccountFlags, AccountId, ActorType, CashflowRole, CommandEnvelope,
    CommandMeta, CreateAccount, Currency, KernelError, LedgerAccountId, VaultController,
    VaultState,
};
use tempfile::TempDir;
use uuid::Uuid;

const PASSWORD: &[u8] = b"correct horse battery staple";
const NEW_PASSWORD: &[u8] = b"an entirely different phrase";

fn meta() -> CommandMeta {
    CommandMeta {
        command_id: Uuid::now_v7(),
        correlation_id: Uuid::now_v7(),
        causation_id: None,
        actor_type: ActorType::User,
        actor_id: "tester".to_owned(),
        idempotency_key: Uuid::now_v7().to_string(),
    }
}

fn account_named(name: &str) -> Account {
    Account::new(
        AccountId::new(),
        LedgerAccountId::new(),
        name,
        CashflowRole::LiquidCash,
        Currency::Usd,
        AccountFlags::default(),
    )
}

#[test]
fn transition_graph_allows_legal_edges_and_rejects_illegal_ones() {
    use VaultState::*;

    // Representative legal edges (the happy path + universal fault edge).
    assert!(NoVault.can_transition_to(CreatingVault));
    assert!(CreatingVault.can_transition_to(Unlocked));
    assert!(Locked.can_transition_to(Unlocking));
    assert!(Unlocking.can_transition_to(Unlocked));
    assert!(Unlocking.can_transition_to(Locked));
    assert!(Unlocked.can_transition_to(Locking));
    assert!(Locking.can_transition_to(Locked));
    // A fault can be detected from anywhere.
    for from in [
        NoVault,
        CreatingVault,
        Locked,
        Unlocking,
        Unlocked,
        Migrating,
    ] {
        assert!(from.can_transition_to(CorruptNeedsRecovery));
    }

    // Illegal shortcuts.
    assert!(!NoVault.can_transition_to(Unlocked)); // must pass through CreatingVault
    assert!(!Locked.can_transition_to(Unlocked)); // must pass through Unlocking
    assert!(!Unlocked.can_transition_to(NoVault));
    assert!(!Locked.can_transition_to(Locking));
}

#[test]
fn classify_reflects_what_is_on_disk() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("vault.db");
    let sidecar = dir.path().join("vault.db.envelope");

    // Nothing yet.
    assert_eq!(classify_vault(&path), VaultState::NoVault);

    // A real vault → both files present → Locked.
    let mut controller = VaultController::open(&path);
    controller.create(PASSWORD).unwrap();
    controller.lock().unwrap();
    assert!(path.exists() && sidecar.exists());
    assert_eq!(classify_vault(&path), VaultState::Locked);

    // Sidecar without DB (interrupted create / lost DB) → needs recovery.
    std::fs::remove_file(&path).unwrap();
    assert_eq!(classify_vault(&path), VaultState::CorruptNeedsRecovery);

    // DB without sidecar (lost envelope) → needs recovery.
    std::fs::write(&path, b"db bytes").unwrap();
    std::fs::remove_file(&sidecar).unwrap();
    assert_eq!(classify_vault(&path), VaultState::CorruptNeedsRecovery);
}

#[test]
fn create_lock_unlock_walks_the_expected_states_and_persists() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("vault.db");

    let mut controller = VaultController::open(&path);
    assert_eq!(controller.state(), VaultState::NoVault);

    controller.create(PASSWORD).unwrap();
    assert_eq!(controller.state(), VaultState::Unlocked);
    let kernel = controller.kernel().expect("unlocked kernel");
    kernel
        .dispatch(CommandEnvelope::new(
            meta(),
            CreateAccount::new(account_named("Checking")),
        ))
        .unwrap();
    assert_eq!(kernel.account_count().unwrap(), 1);

    controller.lock().unwrap();
    assert_eq!(controller.state(), VaultState::Locked);
    assert!(controller.kernel().is_none());

    controller.unlock(PASSWORD).unwrap();
    assert_eq!(controller.state(), VaultState::Unlocked);
    assert_eq!(controller.kernel().unwrap().account_count().unwrap(), 1);
}

#[test]
fn wrong_password_unlock_returns_to_locked_never_unlocked() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("vault.db");

    let mut controller = VaultController::open(&path);
    controller.create(PASSWORD).unwrap();
    controller.lock().unwrap();

    let error = controller.unlock(b"the wrong password").unwrap_err();
    assert!(matches!(error, KernelError::VaultUnlockFailed));
    assert_eq!(controller.state(), VaultState::Locked);
    assert!(controller.kernel().is_none());
}

#[test]
fn interrupted_create_is_recovered_as_corrupt_not_silently_lost() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("vault.db");

    // Build a real vault, then simulate an interrupted create by removing the DB
    // but leaving the envelope sidecar.
    VaultController::open(&path).create(PASSWORD).unwrap();
    std::fs::remove_file(&path).unwrap();

    // A fresh controller classifies the half state as needing recovery — never
    // NoVault (which would silently discard the orphaned envelope).
    assert_eq!(
        VaultController::open(&path).state(),
        VaultState::CorruptNeedsRecovery
    );
}

#[test]
fn lock_from_the_wrong_state_is_an_illegal_transition() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("vault.db");

    // Locking a vault that was never opened is not a legal edge.
    let mut controller = VaultController::open(&path); // NoVault
    let error = controller.lock().unwrap_err();
    assert!(matches!(
        error,
        KernelError::IllegalVaultTransition {
            from: VaultState::NoVault,
            to: VaultState::Locking,
        }
    ));
}

// ---- change password (personal-cfo-zxq) ------------------------------------

#[test]
fn change_password_rewraps_the_envelope_and_keeps_the_data() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("vault.db");

    let mut controller = VaultController::open(&path);
    controller.create(PASSWORD).unwrap();
    controller
        .kernel()
        .unwrap()
        .dispatch(CommandEnvelope::new(
            meta(),
            CreateAccount::new(account_named("Checking")),
        ))
        .unwrap();

    // The change happens in place: the vault stays unlocked and usable.
    controller.change_password(PASSWORD, NEW_PASSWORD).unwrap();
    assert_eq!(controller.state(), VaultState::Unlocked);
    assert_eq!(controller.kernel().unwrap().account_count().unwrap(), 1);

    // After a lock, the old password no longer opens the vault…
    controller.lock().unwrap();
    let error = controller.unlock(PASSWORD).unwrap_err();
    assert!(matches!(error, KernelError::VaultUnlockFailed));
    assert_eq!(controller.state(), VaultState::Locked);

    // …the new one does, and the account data is intact (same DEK, same DB).
    controller.unlock(NEW_PASSWORD).unwrap();
    assert_eq!(controller.kernel().unwrap().account_count().unwrap(), 1);
}

#[test]
fn change_password_with_a_wrong_old_password_leaves_the_vault_untouched() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("vault.db");

    let mut controller = VaultController::open(&path);
    controller.create(PASSWORD).unwrap();

    let error = controller
        .change_password(b"not the password", NEW_PASSWORD)
        .unwrap_err();
    assert!(matches!(error, KernelError::VaultUnlockFailed));
    // Still open — the refused change never dropped the kernel.
    assert_eq!(controller.state(), VaultState::Unlocked);
    assert!(controller.kernel().is_some());

    // The envelope on disk is untouched: the original password still unlocks.
    controller.lock().unwrap();
    controller.unlock(PASSWORD).unwrap();
    assert_eq!(controller.state(), VaultState::Unlocked);
}

#[test]
fn change_password_requires_an_unlocked_vault() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("vault.db");

    let mut controller = VaultController::open(&path);
    controller.create(PASSWORD).unwrap();
    controller.lock().unwrap(); // Locked, not Unlocked

    let error = controller
        .change_password(PASSWORD, NEW_PASSWORD)
        .unwrap_err();
    assert!(matches!(
        error,
        KernelError::IllegalVaultTransition {
            from: VaultState::Locked,
            to: VaultState::Rekeying,
        }
    ));
    // The refusal changed nothing: the original password still unlocks.
    controller.unlock(PASSWORD).unwrap();
}

#[test]
fn a_preexisting_garbage_tmp_does_not_corrupt_a_password_change() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("vault.db");
    let tmp = dir.path().join("vault.db.envelope.tmp");

    let mut controller = VaultController::open(&path);
    controller.create(PASSWORD).unwrap();
    // A leftover tmp from some earlier interrupted change (fault injection).
    std::fs::write(&tmp, b"garbage that is not an envelope").unwrap();

    controller.change_password(PASSWORD, NEW_PASSWORD).unwrap();
    assert!(!tmp.exists(), "the tmp was consumed by the rename");

    controller.lock().unwrap();
    controller.unlock(NEW_PASSWORD).unwrap();
    assert_eq!(controller.state(), VaultState::Unlocked);
}

#[test]
fn an_interrupted_change_leaves_the_old_password_valid() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("vault.db");
    let sidecar = dir.path().join("vault.db.envelope");
    let tmp = dir.path().join("vault.db.envelope.tmp");

    // Build the two real envelopes (old and new) by running a full change, then
    // reconstruct the crash window on disk: tmp written, rename never ran.
    let mut controller = VaultController::open(&path);
    controller.create(PASSWORD).unwrap();
    controller
        .kernel()
        .unwrap()
        .dispatch(CommandEnvelope::new(
            meta(),
            CreateAccount::new(account_named("Checking")),
        ))
        .unwrap();
    let old_envelope = std::fs::read(&sidecar).unwrap();
    controller.change_password(PASSWORD, NEW_PASSWORD).unwrap();
    let new_envelope = std::fs::read(&sidecar).unwrap();
    controller.lock().unwrap();
    std::fs::write(&sidecar, &old_envelope).unwrap();
    std::fs::write(&tmp, &new_envelope).unwrap();

    // The crash state classifies as a normal locked vault, and the OLD password
    // (the one matching the installed sidecar) still unlocks it, data intact.
    let mut recovered = VaultController::open(&path);
    assert_eq!(recovered.state(), VaultState::Locked);
    recovered.unlock(PASSWORD).unwrap();
    assert_eq!(recovered.kernel().unwrap().account_count().unwrap(), 1);
}

#[test]
fn delete_wipes_the_vault_files_and_returns_to_no_vault() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("vault.db");
    let sidecar = dir.path().join("vault.db.envelope");
    let blobs = dir.path().join("blobs");

    let mut controller = VaultController::open(&path);
    controller.create(PASSWORD).unwrap();
    controller
        .kernel()
        .unwrap()
        .dispatch(CommandEnvelope::new(
            meta(),
            CreateAccount::new(account_named("Checking")),
        ))
        .unwrap();
    // A blob store to prove the whole instance is cleaned, not just the DB.
    std::fs::create_dir_all(&blobs).unwrap();
    std::fs::write(blobs.join("blob"), b"x").unwrap();
    assert!(path.exists() && sidecar.exists());

    controller.delete().unwrap();

    assert_eq!(controller.state(), VaultState::NoVault);
    assert!(controller.kernel().is_none());
    assert!(!path.exists(), "db removed");
    assert!(!sidecar.exists(), "envelope removed");
    assert!(!blobs.exists(), "blob store removed");
    assert_eq!(classify_vault(&path), VaultState::NoVault);
}

#[test]
fn delete_requires_an_unlocked_vault() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("vault.db");

    let mut controller = VaultController::open(&path);
    controller.create(PASSWORD).unwrap();
    controller.lock().unwrap(); // Locked, not Unlocked

    let error = controller.delete().unwrap_err();
    assert!(matches!(
        error,
        KernelError::IllegalVaultTransition {
            from: VaultState::Locked,
            to: VaultState::NoVault,
        }
    ));
    assert!(path.exists(), "a refused delete leaves the vault intact");
}

#[test]
fn an_encrypted_backup_respawns_a_deleted_vault() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("vault.db");
    let backup = dir.path().join("backup.pcfo");

    let mut controller = VaultController::open(&path);
    controller.create(PASSWORD).unwrap();
    controller
        .kernel()
        .unwrap()
        .dispatch(CommandEnvelope::new(
            meta(),
            CreateAccount::new(account_named("Checking")),
        ))
        .unwrap();
    controller
        .kernel()
        .unwrap()
        .export_backup(
            PASSWORD,
            &backup,
            "test",
            "2026-07-02T00:00:00Z".to_owned(),
            Uuid::now_v7(),
        )
        .unwrap();

    controller.delete().unwrap();
    assert_eq!(controller.state(), VaultState::NoVault);

    // The backup restores the wiped instance and its data.
    controller.restore(&backup, PASSWORD).unwrap();
    assert_eq!(controller.state(), VaultState::Unlocked);
    assert_eq!(controller.kernel().unwrap().account_count().unwrap(), 1);
}

#[test]
fn switch_to_locks_the_current_vault_repoints_and_keeps_data_isolated() {
    let dir = TempDir::new().unwrap();
    let path_a = dir.path().join("a/vault.db");
    let path_b = dir.path().join("b/vault.db");
    std::fs::create_dir_all(path_a.parent().unwrap()).unwrap();
    std::fs::create_dir_all(path_b.parent().unwrap()).unwrap();

    // Vault A: created + one account.
    let mut controller = VaultController::open(&path_a);
    controller.create(PASSWORD).unwrap();
    controller
        .kernel()
        .unwrap()
        .dispatch(CommandEnvelope::new(
            meta(),
            CreateAccount::new(account_named("A-checking")),
        ))
        .unwrap();

    // Switch to a fresh location B: A is locked (its DEK dropped), B classifies as NoVault.
    controller.switch_to(path_b.clone()).unwrap();
    assert!(
        controller.kernel().is_none(),
        "the previous vault's kernel is gone"
    );
    assert_eq!(controller.state(), VaultState::NoVault);

    // Create B there with its own data.
    controller.create(PASSWORD).unwrap();
    controller
        .kernel()
        .unwrap()
        .dispatch(CommandEnvelope::new(
            meta(),
            CreateAccount::new(account_named("B-savings")),
        ))
        .unwrap();
    assert_eq!(controller.kernel().unwrap().account_count().unwrap(), 1);

    // Switch back to A: B locks, A is an existing (Locked) vault; unlocking reveals A's data only.
    controller.switch_to(path_a.clone()).unwrap();
    assert_eq!(controller.state(), VaultState::Locked);
    controller.unlock(PASSWORD).unwrap();
    assert_eq!(
        controller.kernel().unwrap().account_count().unwrap(),
        1,
        "A holds its own single account — data never crossed from B"
    );
}

#[test]
fn health_check_is_green_on_a_freshly_created_vault() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("vault.db");

    let mut controller = VaultController::open(&path);
    controller.create(PASSWORD).unwrap();

    let health = controller.health_check().unwrap();
    assert!(health.writer_healthy);
    assert!(health.wal_configured);
    assert!(health.schema_coherent);
    assert!(health.read_models_current);
    assert!(health.is_healthy());

    // Health check requires an unlocked vault.
    controller.lock().unwrap();
    assert!(controller.health_check().is_err());
}
