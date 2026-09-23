//! End-to-end scheduled and on-demand backup exports against a real test vault.

use std::fs;

use finance_kernel::{BackupCadence, BackupHistoryKind, Kernel};
use tempfile::tempdir;

#[test]
fn scheduled_exports_verify_record_history_and_never_prune_unowned_files() {
    let vault_dir = tempdir().unwrap();
    let backup_dir = vault_dir.path().join("backups");
    fs::create_dir(&backup_dir).unwrap();
    let kernel =
        Kernel::create_vault(vault_dir.path().join("vault.db"), b"correct horse battery").unwrap();

    let defaults = kernel.backup_schedule_settings().unwrap();
    assert_eq!(defaults.cadence, BackupCadence::Weekly);
    assert_eq!(defaults.destination, None);
    assert_eq!(defaults.keep_last, None);

    let configured = kernel
        .configure_backup_schedule(BackupCadence::Weekly, Some(&backup_dir), Some(2))
        .unwrap();
    assert_eq!(configured.cadence, BackupCadence::Weekly);
    assert_eq!(configured.keep_last, Some(2));
    assert!(configured.next_due_at.is_some());

    let first = kernel.run_scheduled_backup("0.2.0-test").unwrap();
    let second = kernel.run_scheduled_backup("0.2.0-test").unwrap();
    let third = kernel.run_scheduled_backup("0.2.0-test").unwrap();
    assert!(first.verified && second.verified && third.verified);
    assert_eq!(first.kind, BackupHistoryKind::Scheduled);
    assert!(!std::path::Path::new(&first.destination).exists());
    assert!(std::path::Path::new(&second.destination).exists());
    assert!(std::path::Path::new(&third.destination).exists());

    let foreign = backup_dir.join("foreign.pcfobk");
    fs::write(&foreign, b"not a backup created by this job").unwrap();
    let fourth = kernel.run_scheduled_backup("0.2.0-test").unwrap();
    assert!(std::path::Path::new(&fourth.destination).exists());
    assert!(foreign.exists());
    assert_eq!(
        fs::read(&foreign).unwrap(),
        b"not a backup created by this job"
    );

    // A recorded file that no longer verifies is never removed by retention.
    fs::write(&third.destination, b"tampered after export").unwrap();
    let fifth = kernel.run_scheduled_backup("0.2.0-test").unwrap();
    assert!(std::path::Path::new(&third.destination).exists());
    assert!(std::path::Path::new(&fifth.destination).exists());

    let manual_path = backup_dir.join("manual.pcfobk");
    let manual = kernel
        .export_manual_backup(&manual_path, "0.2.0-test")
        .unwrap();
    assert_eq!(manual.kind, BackupHistoryKind::Manual);
    assert!(manual.verified);
    assert!(manual_path.exists());

    let on_demand = kernel.run_backup_now("0.2.0-test").unwrap();
    assert_eq!(on_demand.kind, BackupHistoryKind::Manual);
    assert!(on_demand.verified);
    assert!(std::path::Path::new(&on_demand.destination).exists());

    let history = kernel.backup_history().unwrap();
    assert_eq!(history.len(), 7);
    assert!(history.iter().all(|entry| entry.vault_id == first.vault_id));
    assert!(history.iter().any(
        |entry| entry.backup_id == manual.backup_id && entry.kind == BackupHistoryKind::Manual
    ));
    assert!(history
        .iter()
        .any(|entry| entry.backup_id == on_demand.backup_id
            && entry.kind == BackupHistoryKind::Manual));
}
