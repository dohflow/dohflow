//! Vault lifecycle end-to-end (personal-cfo-vhv): create → write → lock →
//! unlock → read persists; wrong password is rejected; create/unlock guard
//! against missing/duplicate vaults.
//!
//! Opacity of the encrypted file (stock SQLite cannot read it) is proven in
//! `db-worker`'s `raw_keyed_vault_is_opaque_without_the_key`, on the same
//! raw-keyed vault `create_vault` produces — finance-kernel must not depend on
//! `rusqlite` (the db-worker boundary), so it is not re-tested here.

use db_worker::DbError;
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
        idempotency_key: Uuid::new_v4().to_string(),
        issued_at: finance_kernel::IssuedAt::now(),
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

#[test]
fn newer_schema_error_keeps_its_identity_across_the_kernel_boundary() {
    let error = KernelError::from(DbError::NewerSchema {
        observed: db_worker::CURRENT_SCHEMA_VERSION + 1,
        supported: db_worker::CURRENT_SCHEMA_VERSION,
    });
    assert!(matches!(
        error,
        KernelError::NewerVaultSchema { observed, supported }
            if observed == supported + 1
    ));
}

#[test]
fn existing_envelope_with_missing_db_never_creates_a_replacement_on_unlock() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("vault.db");
    Kernel::create_vault(&path, PASSWORD).unwrap().lock();
    std::fs::rename(&path, dir.path().join("preserved.db")).unwrap();
    assert!(matches!(
        Kernel::unlock_vault(&path, PASSWORD),
        Err(KernelError::UnsupportedVaultSchema)
    ));
    assert!(!path.exists());
    assert!(dir.path().join("preserved.db").exists());
}

/// A `-wal` with a page size this app never uses (personal-cfo-5vto5, RUSTSEC-2026-0459) is refused at
/// unlock with its own typed error, and the vault and its log are left exactly as they were.
#[test]
fn unlock_refuses_a_mismatched_wal_with_a_typed_error_and_touches_nothing() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("vault.db");
    let kernel = Kernel::create_vault(&path, PASSWORD).expect("create vault");
    kernel
        .dispatch(CommandEnvelope::new(
            meta(),
            CreateAccount::new(account_named("Checking")),
        ))
        .expect("create account");
    // While the vault is open its `-wal` is real: keep a copy of both files, as a crash or a copied
    // folder would leave them.
    let mut wal = path.as_os_str().to_owned();
    wal.push("-wal");
    let wal = std::path::PathBuf::from(wal);
    let copy_dir = TempDir::new().unwrap();
    let copy = copy_dir.path().join("vault.db");
    std::fs::copy(&path, &copy).unwrap();
    // The key envelope sits beside the vault.
    let mut envelope = path.as_os_str().to_owned();
    envelope.push(".envelope");
    let mut copy_envelope = copy.as_os_str().to_owned();
    copy_envelope.push(".envelope");
    std::fs::copy(&envelope, &copy_envelope).unwrap();
    let mut copy_wal = copy.as_os_str().to_owned();
    copy_wal.push("-wal");
    let copy_wal = std::path::PathBuf::from(copy_wal);
    std::fs::copy(&wal, &copy_wal).unwrap();
    kernel.lock();

    let mut bytes = std::fs::read(&copy_wal).unwrap();
    bytes[8..12].copy_from_slice(&8192_u32.to_be_bytes());
    std::fs::write(&copy_wal, &bytes).unwrap();
    let main_before = std::fs::read(&copy).unwrap();

    let refused = Kernel::unlock_vault(&copy, PASSWORD)
        .err()
        .expect("refused");
    assert!(
        matches!(
            refused,
            KernelError::WalHeaderMismatch {
                found_page_size: 8192,
                expected_page_size: 4096
            }
        ),
        "{refused:?}"
    );
    assert_eq!(
        std::fs::read(&copy_wal).unwrap(),
        bytes,
        "the -wal is untouched"
    );
    assert_eq!(
        std::fs::read(&copy).unwrap(),
        main_before,
        "the vault is untouched"
    );
    let text = refused.to_string();
    assert!(text.contains("8192") && text.contains("4096") && !text.contains("vault.db"));
}
