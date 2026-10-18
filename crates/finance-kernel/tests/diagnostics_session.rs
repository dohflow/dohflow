//! Local diagnostic capture on the kernel's session (personal-cfo-lyd,
//! `docs/security/logging-policy.md` §6–§7).
//!
//! The kernel owns its session's `Diagnostics`, so the lifecycle rules hold by
//! construction: a vault lock drops the kernel and with it every record and any
//! pending preview; a new unlock starts empty. These tests drive real vault
//! operations (backup, restore, lock/unlock) and read the capture back through
//! the same previewed-bundle path the UI uses.

use finance_kernel::{
    failure_category, Account, AccountFlags, AccountId, ActorType, CashflowRole, CommandEnvelope,
    CommandMeta, CreateAccount, Currency, Kernel, KernelError, LedgerAccountId, VaultController,
};
use observability::diagnostics::{
    parse_bundle, BundleHeader, FailureCategory, Metric, Outcome, Value,
};
use tempfile::TempDir;
use uuid::Uuid;

const PASSWORD: &[u8] = b"correct horse battery staple";

fn header() -> BundleHeader {
    BundleHeader {
        build_version: "0.2.0".into(),
        build_channel: "dev".into(),
        platform: "macos".into(),
        created_on: "2026-09-28".into(),
    }
}

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

fn outcomes(kernel: &Kernel, metric: Metric) -> Vec<Value> {
    let bundle = parse_bundle(
        kernel
            .diagnostics()
            .preview(&header())
            .expect("preview")
            .bytes(),
    )
    .expect("bundle parses");
    bundle
        .records
        .into_iter()
        .filter(|r| r.metric == metric)
        .map(|r| r.value)
        .collect()
}

#[test]
fn a_new_session_starts_empty() {
    let dir = TempDir::new().unwrap();
    let kernel = Kernel::create_vault(dir.path().join("vault.db"), PASSWORD).unwrap();
    assert!(kernel.diagnostics().is_empty());
}

#[test]
fn backup_outcomes_are_recorded_as_fixed_categories_never_paths_or_messages() {
    let dir = TempDir::new().unwrap();
    let kernel = Kernel::create_vault(dir.path().join("vault.db"), PASSWORD).unwrap();

    // Success.
    kernel
        .export_manual_backup(&dir.path().join("ok.pcfobk"), "0.2.0")
        .expect("manual backup");

    // Failure, through a path that would leak if anything copied it: an email,
    // an account-number sentinel and a folder name.
    let hostile_dir = dir.path().join("jane.doe@example.com-ACCT-SENTINEL-771");
    let hostile = hostile_dir.join("missing").join("backup.pcfobk");
    let error = kernel
        .export_manual_backup(&hostile, "0.2.0")
        .expect_err("destination folder does not exist");
    assert_eq!(failure_category(&error), FailureCategory::Validation);

    assert_eq!(
        outcomes(&kernel, Metric::BackupOutcome),
        vec![
            Value::Outcome(Outcome::Success),
            Value::Outcome(Outcome::Failure(FailureCategory::Validation)),
        ]
    );
    let text = kernel
        .diagnostics()
        .preview(&header())
        .expect("preview")
        .text()
        .to_owned();
    for leak in [
        "jane.doe",
        "example.com",
        "ACCT-SENTINEL-771",
        "missing",
        "pcfobk",
    ] {
        assert!(!text.contains(leak), "bundle leaked `{leak}`:\n{text}");
    }
}

#[test]
fn a_successful_restore_is_the_first_record_of_the_restored_session() {
    let src = TempDir::new().unwrap();
    let source = Kernel::create_vault(src.path().join("vault.db"), PASSWORD).unwrap();
    source
        .dispatch(CommandEnvelope::new(
            meta(),
            CreateAccount::new(Account::new(
                AccountId::new(),
                LedgerAccountId::new(),
                "Checking",
                CashflowRole::LiquidCash,
                Currency::Usd,
                AccountFlags::default(),
            )),
        ))
        .unwrap();
    let pkg = src.path().join("b.pcfobk");
    source
        .export_unattended(&pkg, "0.2.0", "2026-09-28T00:00:00Z".into(), Uuid::now_v7())
        .unwrap();

    let dest = TempDir::new().unwrap();
    let restored = Kernel::restore_backup(&pkg, PASSWORD, &dest.path().join("vault.db")).unwrap();
    assert_eq!(
        outcomes(&restored, Metric::RestoreOutcome),
        vec![Value::Outcome(Outcome::Success)]
    );
    // The SOURCE vault's session is separate and saw no restore.
    assert!(outcomes(&source, Metric::RestoreOutcome).is_empty());
}

#[test]
fn locking_the_vault_drops_the_capture_and_the_pending_preview() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("vault.db");
    Kernel::create_vault(&path, PASSWORD).unwrap().lock();

    let mut controller = VaultController::open(&path);
    controller.unlock(PASSWORD).unwrap();
    let kernel = controller.kernel().expect("unlocked");
    kernel
        .export_manual_backup(&dir.path().join("one.pcfobk"), "0.2.0")
        .unwrap();
    let snapshot = kernel.diagnostics().preview(&header()).expect("preview");
    assert_eq!(snapshot.records(), 1);
    let snapshot_id = snapshot.id();

    controller.lock().unwrap();
    assert!(
        controller.kernel().is_none(),
        "the session is gone with the kernel"
    );

    controller.unlock(PASSWORD).unwrap();
    let kernel = controller.kernel().expect("unlocked again");
    assert!(kernel.diagnostics().is_empty(), "nothing survives a lock");
    assert!(
        kernel.diagnostics().pending(snapshot_id).is_none(),
        "a preview from before the lock cannot be saved"
    );
}

#[test]
fn every_kernel_error_maps_to_a_fixed_category() {
    // The category is chosen from the variant alone; the message never matters.
    for (error, category) in [
        (
            KernelError::Validation("acct ACCT-SENTINEL-771".into()),
            FailureCategory::Validation,
        ),
        (
            KernelError::Persistence("disk /Users/jane".into()),
            FailureCategory::Storage,
        ),
        (
            KernelError::Vault("password=SENTINEL-PASS".into()),
            FailureCategory::Vault,
        ),
        (KernelError::VaultUnlockFailed, FailureCategory::Vault),
        (KernelError::WriterPanicked, FailureCategory::Internal),
    ] {
        assert_eq!(failure_category(&error), category, "{error:?}");
    }
}
