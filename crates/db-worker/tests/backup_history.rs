//! Device-local backup receipt persistence (personal-cfo-8qh).

mod common;

use db_worker::{BackupHistoryEntry, BackupHistoryKind};
use uuid::Uuid;

use common::worker;

fn entry(
    backup_id: Uuid,
    vault_id: Uuid,
    kind: BackupHistoryKind,
    created_at: &str,
) -> BackupHistoryEntry {
    BackupHistoryEntry {
        backup_id,
        vault_id,
        created_at: created_at.to_owned(),
        kind,
        destination: "/tmp/dohflow-backups/test.pcfobk".to_owned(),
        format_version: 2,
        size_bytes: 512,
        verified: true,
        error: None,
    }
}

#[test]
fn history_persists_manual_and_scheduled_receipts_for_only_the_owning_vault() {
    let (_dir, worker) = worker();
    let vault_id = worker.vault_metadata().unwrap().vault_id;
    let manual = entry(
        Uuid::now_v7(),
        vault_id,
        BackupHistoryKind::Manual,
        "2026-09-21T12:00:00Z",
    );
    let scheduled = entry(
        Uuid::now_v7(),
        vault_id,
        BackupHistoryKind::Scheduled,
        "2026-09-22T12:00:00Z",
    );

    worker.record_backup_history(&manual).unwrap();
    worker.record_backup_history(&scheduled).unwrap();

    let history = worker.backup_history(vault_id).unwrap();
    assert_eq!(history, vec![scheduled, manual]);
    assert_eq!(history[0].kind, BackupHistoryKind::Scheduled);
    assert!(history
        .iter()
        .all(|row| row.verified && row.error.is_none()));
}

#[test]
fn history_rejects_other_vaults_and_inconsistent_verification_receipts() {
    let (_dir, worker) = worker();
    let vault_id = worker.vault_metadata().unwrap().vault_id;

    let mut foreign = entry(
        Uuid::now_v7(),
        Uuid::now_v7(),
        BackupHistoryKind::Manual,
        "2026-09-22T12:00:00Z",
    );
    assert!(worker.record_backup_history(&foreign).is_err());

    foreign.vault_id = vault_id;
    foreign.destination = "relative/folder/test.pcfobk".to_owned();
    assert!(worker.record_backup_history(&foreign).is_err());

    foreign.destination = "/tmp/private-account-nickname/test.pcfobk".to_owned();
    foreign.verified = false;
    assert!(worker.record_backup_history(&foreign).is_err());

    let failed = BackupHistoryEntry {
        error: Some("Could not verify the backup file.".to_owned()),
        ..foreign
    };
    worker.record_backup_history(&failed).unwrap();
    let history = worker.backup_history(vault_id).unwrap();
    assert_eq!(history.len(), 1);
    assert!(!history[0].verified);
    assert_eq!(
        history[0].error.as_deref(),
        Some("Could not verify the backup file.")
    );
}

#[test]
fn debug_output_redacts_destination_and_error_text() {
    let entry = BackupHistoryEntry {
        error: Some("private error details".to_owned()),
        verified: false,
        ..entry(
            Uuid::now_v7(),
            Uuid::now_v7(),
            BackupHistoryKind::Manual,
            "2026-09-22T12:00:00Z",
        )
    };
    let debug = format!("{entry:?}");
    assert!(!debug.contains("test.pcfobk"));
    assert!(!debug.contains("private-account-nickname"));
    assert!(!debug.contains("private error details"));
    assert!(debug.contains("[REDACTED]"));
}
