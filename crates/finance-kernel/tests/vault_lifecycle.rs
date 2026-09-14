//! Vault lifecycle end-to-end (personal-cfo-vhv): create → write → lock →
//! unlock → read persists; wrong password is rejected; create/unlock guard
//! against missing/duplicate vaults.
//!
//! Opacity of the encrypted file (stock SQLite cannot read it) is proven in
//! `db-worker`'s `raw_keyed_vault_is_opaque_without_the_key`, on the same
//! raw-keyed vault `create_vault` produces — finance-kernel must not depend on
//! `rusqlite` (the db-worker boundary), so it is not re-tested here.

use finance_kernel::{
    Account, AccountFlags, AccountId, ActorType, CashflowRole, CommandEnvelope, CommandMeta,
    CreateAccount, Currency, Kernel, KernelError, LedgerAccountId,
};
use tempfile::TempDir;
use uuid::Uuid;

const PASSWORD: &[u8] = b"correct horse battery staple";

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
fn create_write_lock_unlock_read_round_trips() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("vault.db");

    // Create the vault and write an account into it.
    let kernel = Kernel::create_vault(&path, PASSWORD).expect("create vault");
    let account = account_named("Checking");
    let id = account.id();
    kernel
        .dispatch(CommandEnvelope::new(meta(), CreateAccount::new(account)))
        .expect("create account");
    assert_eq!(kernel.account_count().unwrap(), 1);

    // Lock (drop the kernel → DEK zeroized).
    kernel.lock();

    // Unlock with the right password and confirm the data persisted.
    let reopened = Kernel::unlock_vault(&path, PASSWORD).expect("unlock vault");
    assert_eq!(reopened.account_count().unwrap(), 1);
    assert!(reopened.account_exists(id).unwrap());
}

#[test]
fn wrong_password_is_rejected_and_never_opens_the_vault() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("vault.db");

    Kernel::create_vault(&path, PASSWORD)
        .expect("create vault")
        .lock();

    match Kernel::unlock_vault(&path, b"not the password") {
        Err(KernelError::VaultUnlockFailed) => {}
        Err(other) => panic!("expected VaultUnlockFailed, got {other:?}"),
        Ok(_) => panic!("wrong password must never open the vault"),
    }
}

#[test]
fn create_refuses_to_clobber_an_existing_vault() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("vault.db");

    Kernel::create_vault(&path, PASSWORD)
        .expect("first create")
        .lock();

    let again = Kernel::create_vault(&path, PASSWORD);
    assert!(matches!(again, Err(KernelError::VaultExists)));
}

#[test]
fn unlock_missing_vault_reports_not_found() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("vault.db");

    let result = Kernel::unlock_vault(&path, PASSWORD);
    assert!(matches!(result, Err(KernelError::VaultNotFound)));
}
