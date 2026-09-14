//! End-to-end restore (personal-cfo-au3, ADR 0024 §5): export a real vault with
//! user data, restore it into a **fresh** location, and verify the restored
//! vault opens on a fresh instance and reproduces the data. A wrong password is
//! refused **before any write**; an existing vault is never clobbered.

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

/// Create a vault at `dir/vault.db`, seed one account, and export a backup;
/// returns the package path.
fn seed_and_export(dir: &std::path::Path) -> std::path::PathBuf {
    let db_path = dir.join("vault.db");
    let kernel = Kernel::create_vault(&db_path, PASSWORD).expect("create vault");
    kernel
        .dispatch(CommandEnvelope::new(
            meta(),
            CreateAccount::new(account_named("Checking")),
        ))
        .expect("create account");
    assert_eq!(kernel.account_count().unwrap(), 1);
    // A non-default value (personal-cfo-q329): proves the backup carries the ACTUAL
    // stored timezone through, not merely that "UTC" survives by coincidence with the
    // untouched default.
    kernel
        .set_household_timezone("America/Los_Angeles")
        .expect("set household timezone");

    let pkg = dir.join("backup.pcfobk");
    kernel
        .export_backup(
            PASSWORD,
            &pkg,
            "0.1.0-test",
            "2026-06-20T00:00:00Z".into(),
            Uuid::from_bytes([3u8; 16]),
        )
        .expect("export backup");
    pkg
}

#[test]
fn restore_into_fresh_location_reproduces_the_data() {
    let src = TempDir::new().unwrap();
    let pkg = seed_and_export(src.path());

    let dest = TempDir::new().unwrap();
    let dest_db = dest.path().join("vault.db");
    let restored = Kernel::restore_backup(&pkg, PASSWORD, &dest_db).expect("restore");

    // The restored vault opens on a fresh instance and reproduces the account.
    assert_eq!(restored.account_count().unwrap(), 1);
    assert!(restored
        .account_views()
        .unwrap()
        .iter()
        .any(|a| a.name == "Checking"));
    // personal-cfo-q329: household_timezone round-trips through the backup, not just
    // reset to the "UTC" default a fresh vault would otherwise start at.
    assert_eq!(
        restored.vault_metadata().unwrap().household_timezone,
        "America/Los_Angeles"
    );
    // The vault files landed at the destination.
    assert!(dest_db.exists());
    assert!(dest.path().join("vault.db.envelope").exists());
}

#[test]
fn wrong_password_is_refused_before_writing() {
    let src = TempDir::new().unwrap();
    let pkg = seed_and_export(src.path());

    let dest = TempDir::new().unwrap();
    let dest_db = dest.path().join("vault.db");
    assert!(matches!(
        Kernel::restore_backup(&pkg, b"the wrong password", &dest_db),
        Err(KernelError::VaultUnlockFailed)
    ));
    // The verify-then-install order means nothing was written on failure.
    assert!(!dest_db.exists());
    assert!(!dest.path().join("vault.db.envelope").exists());
}

#[test]
fn refuses_to_clobber_an_existing_vault() {
    let src = TempDir::new().unwrap();
    let pkg = seed_and_export(src.path());

    // The source dir already holds a vault; restoring onto it is refused.
    let existing = src.path().join("vault.db");
    assert!(matches!(
        Kernel::restore_backup(&pkg, PASSWORD, &existing),
        Err(KernelError::VaultExists)
    ));
}
