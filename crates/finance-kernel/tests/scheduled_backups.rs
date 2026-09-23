//! End-to-end scheduled and on-demand backup exports against a real test vault.

use std::fs;

use finance_kernel::{BackupCadence, BackupHistoryKind, Kernel};
use tempfile::tempdir;

#[test]
fn scheduled_exports_keep_prior_backups_and_never_delete_foreign_files() {
    let vault_dir = tempdir().unwrap();
    let backup_dir = vault_dir.path().join("backups");
    fs::create_dir(&backup_dir).unwrap();
    let kernel =
        Kernel::create_vault(vault_dir.path().join("vault.db"), b"correct horse battery").unwrap();

    let defaults = kernel.backup_schedule_settings().unwrap();
    assert_eq!(defaults.cadence, BackupCadence::Weekly);
    assert_eq!(defaults.destination, None);

    let configured = kernel
        .configure_backup_schedule(BackupCadence::Weekly, Some(&backup_dir))
        .unwrap();
    assert_eq!(configured.cadence, BackupCadence::Weekly);
    assert!(configured.next_due_at.is_some());

    let first = kernel.run_scheduled_backup("0.2.0-test").unwrap();
    let second = kernel.run_scheduled_backup("0.2.0-test").unwrap();
    let third = kernel.run_scheduled_backup("0.2.0-test").unwrap();
    assert!(first.verified && second.verified && third.verified);
    assert_eq!(first.kind, BackupHistoryKind::Scheduled);
    let second_contents = fs::read(&second.destination).unwrap();
    let third_contents = fs::read(&third.destination).unwrap();
    assert!(std::path::Path::new(&first.destination).exists());
    assert!(std::path::Path::new(&second.destination).exists());
    assert!(std::path::Path::new(&third.destination).exists());

    let foreign = backup_dir.join("foreign.pcfobk");
    let foreign_contents = b"not a backup created by this job";
    fs::write(&foreign, foreign_contents).unwrap();

    // A previously recorded path can be replaced by a different file. Later
    // scheduled runs must not remove or rewrite that foreign content.
    let replacement_contents = b"foreign content at a recorded backup path";
    fs::write(&first.destination, replacement_contents).unwrap();
    let fourth = kernel.run_scheduled_backup("0.2.0-test").unwrap();
    let fifth = kernel.run_scheduled_backup("0.2.0-test").unwrap();
    let sixth = kernel.run_scheduled_backup("0.2.0-test").unwrap();
    for entry in [&first, &second, &third, &fourth, &fifth, &sixth] {
        assert!(std::path::Path::new(&entry.destination).exists());
    }
    assert_eq!(fs::read(&first.destination).unwrap(), replacement_contents);
    assert_eq!(fs::read(&second.destination).unwrap(), second_contents);
    assert_eq!(fs::read(&third.destination).unwrap(), third_contents);
    assert!(std::path::Path::new(&fourth.destination).exists());
    assert!(foreign.exists());
    assert_eq!(fs::read(&foreign).unwrap(), foreign_contents);

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
    assert_eq!(history.len(), 8);
    assert!(history.iter().all(|entry| entry.vault_id == first.vault_id));
    assert!(history.iter().any(
        |entry| entry.backup_id == manual.backup_id && entry.kind == BackupHistoryKind::Manual
    ));
    assert!(history
        .iter()
        .any(|entry| entry.backup_id == on_demand.backup_id
            && entry.kind == BackupHistoryKind::Manual));
}
