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
    foreign.destination = "   ".to_owned();
    assert!(worker.record_backup_history(&foreign).is_err());
    foreign.destination = "/tmp/line\nbreak/test.pcfobk".to_owned();
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

#[test]
fn history_accepts_a_non_path_destination_label() {
    // A destination that is not a folder supplies its own label (personal-cfo-7ie.15).
    let (_dir, worker) = worker();
    let vault_id = worker.vault_metadata().unwrap().vault_id;
    let mut receipt = entry(
        Uuid::now_v7(),
        vault_id,
        BackupHistoryKind::Scheduled,
        "2026-09-22T12:00:00Z",
    );
    receipt.destination = "memory://test/one.pcfobk".to_owned();
    worker.record_backup_history(&receipt).unwrap();
    let history = worker.backup_history(vault_id).unwrap();
    assert_eq!(history[0].destination, "memory://test/one.pcfobk");
}

/// personal-cfo-g3m.31: a scheduled receipt and its job's commit marker are one transaction. A fault
/// between them (here a trigger that refuses the marker write) leaves neither; without it, both.
#[test]
fn a_receipt_and_its_jobs_commit_marker_are_atomic() {
    let (_dir, worker) = worker();
    let vault_id = worker.vault_metadata().unwrap().vault_id;
    let job = Uuid::now_v7();
    let conn = worker.read_connection().unwrap();
    conn.execute(
        "INSERT INTO durable_jobs (id, kind, cadence, state, attempt_count, max_attempts,
                                   created_at, updated_at)
         VALUES (?1, 'backup_export', 'weekly', 'running', 1, 3, 't', 't')",
        [job],
    )
    .unwrap();
    let marker = || -> Option<String> {
        conn.query_row(
            "SELECT committed_at FROM durable_jobs WHERE id = ?1",
            [job],
            |row| row.get(0),
        )
        .unwrap()
    };
    let receipt = entry(
        Uuid::now_v7(),
        vault_id,
        BackupHistoryKind::Scheduled,
        "2026-11-01T12:00:00Z",
    );

    conn.execute_batch(
        "CREATE TRIGGER refuse_marker BEFORE UPDATE OF committed_at ON durable_jobs
         BEGIN SELECT RAISE(ABORT, 'injected fault between the receipt and the marker'); END;",
    )
    .unwrap();
    assert!(worker
        .record_backup_history_for_job(&receipt, Some((job, "The scheduled backup failed.")))
        .is_err());
    assert!(
        worker.backup_history(vault_id).unwrap().is_empty(),
        "no receipt"
    );
    assert!(marker().is_none(), "no marker");

    conn.execute_batch("DROP TRIGGER refuse_marker").unwrap();
    worker
        .record_backup_history_for_job(&receipt, Some((job, "The scheduled backup failed.")))
        .unwrap();
    assert_eq!(worker.backup_history(vault_id).unwrap(), vec![receipt]);
    assert!(marker().is_some(), "both");
}

/// A receipt with no job (a manual backup) writes no marker anywhere.
#[test]
fn a_manual_receipt_writes_no_marker() {
    let (_dir, worker) = worker();
    let vault_id = worker.vault_metadata().unwrap().vault_id;
    let job = Uuid::now_v7();
    let conn = worker.read_connection().unwrap();
    conn.execute(
        "INSERT INTO durable_jobs (id, kind, cadence, state, attempt_count, max_attempts,
                                   created_at, updated_at)
         VALUES (?1, 'backup_export', 'weekly', 'running', 1, 3, 't', 't')",
        [job],
    )
    .unwrap();
    worker
        .record_backup_history(&entry(
            Uuid::now_v7(),
            vault_id,
            BackupHistoryKind::Manual,
            "2026-11-01T12:00:00Z",
        ))
        .unwrap();
    let marked: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM durable_jobs WHERE committed_at IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(marked, 0);
}

/// What a receipt commits is recorded with the marker: a verified receipt is a success, and a receipt that
/// records a failure is a failure whose reason lands in the job's `last_error` in the same transaction.
#[test]
fn a_failed_receipt_marks_the_job_failed_with_its_reason_in_the_same_transaction() {
    let (_dir, worker) = worker();
    let vault_id = worker.vault_metadata().unwrap().vault_id;
    let conn = worker.read_connection().unwrap();
    let running = |job: Uuid| {
        conn.execute(
            "INSERT INTO durable_jobs (id, kind, cadence, state, attempt_count, max_attempts,
                                       created_at, updated_at)
             VALUES (?1, 'backup_export', 'weekly', 'running', 1, 3, 't', 't')",
            [job],
        )
        .unwrap();
    };
    let read = |job: Uuid| -> (Option<String>, Option<String>) {
        conn.query_row(
            "SELECT committed_outcome, last_error FROM durable_jobs WHERE id = ?1",
            [job],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
    };
    let reason = "Could not create or verify the scheduled backup.\nCheck the folder in Settings.";

    let verified_job = Uuid::now_v7();
    running(verified_job);
    worker
        .record_backup_history_for_job(
            &entry(
                Uuid::now_v7(),
                vault_id,
                BackupHistoryKind::Scheduled,
                "2026-11-01T12:00:00Z",
            ),
            Some((verified_job, reason)),
        )
        .unwrap();
    assert_eq!(read(verified_job), (Some("succeeded".to_owned()), None));

    let failed_job = Uuid::now_v7();
    running(failed_job);
    let mut failed = entry(
        Uuid::now_v7(),
        vault_id,
        BackupHistoryKind::Scheduled,
        "2026-11-01T12:01:00Z",
    );
    failed.verified = false;
    failed.error = Some("Could not verify the backup file.".to_owned());
    worker
        .record_backup_history_for_job(&failed, Some((failed_job, reason)))
        .unwrap();
    let (outcome, last_error) = read(failed_job);
    assert_eq!(outcome.as_deref(), Some("failed"));
    assert_eq!(
        last_error.as_deref(),
        Some("Could not create or verify the scheduled backup.Check the folder in Settings."),
        "the job's reason, sanitized as the runner's own failure write is"
    );
}
