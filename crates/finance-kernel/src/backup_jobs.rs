//! Vault-local configuration and execution for scheduled backup exports.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use db_worker::{BackupHistoryEntry, BackupHistoryKind};
use job_runtime::{BackoffPolicy, JobSpec, Schedule};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::backup_sink::{BackupSink, BackupSinkError, FolderSink, PutMode, PutOutcome};
use crate::{Kernel, KernelError};

/// Stable row identity for the one scheduled-backup consumer in each vault.
pub const BACKUP_JOB_ID: Uuid = Uuid::from_u128(0x8c9d_6d13_1e2a_4b0a_8f5e_0000_0000_0001);
/// Stable local durable-job kind token.
pub const BACKUP_JOB_KIND: &str = "backup_export";

/// User-facing backup cadence. `Off` is stored in the encrypted job payload;
/// its runtime row remains disabled while retaining a valid schedule token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupCadence {
    /// Do not run automatically; the user may still create a manual backup.
    Off,
    /// Run once per calendar day after the due time.
    Daily,
    /// Run once per seven-day interval (the default).
    Weekly,
    /// Run once per calendar month after the due time.
    Monthly,
}

impl BackupCadence {
    fn schedule(self) -> Schedule {
        match self {
            Self::Off | Self::Weekly => Schedule::Weekly,
            Self::Daily => Schedule::Daily,
            Self::Monthly => Schedule::Monthly,
        }
    }

    fn enabled(self, destination: Option<&str>) -> bool {
        self != Self::Off && destination.is_some()
    }
}

/// How a cancellable scheduled backup ended (personal-cfo-g3m.29).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackupRun {
    /// The backup was written, verified and its receipt recorded. A cancel request that arrives
    /// after this point is too late.
    Done(BackupHistoryEntry),
    /// A cancel request was seen before the receipt was recorded. Nothing remains: no partial or
    /// temporary file in the destination, no receipt, nothing else.
    Cancelled,
}

/// Local Settings view of this vault's backup configuration and latest job state.
#[derive(Clone, PartialEq, Eq)]
pub struct BackupScheduleSettings {
    /// Configured cadence (weekly is the default before a destination is chosen).
    pub cadence: BackupCadence,
    /// Canonical local destination folder, if selected.
    pub destination: Option<PathBuf>,
    /// Persisted runtime due time, if a job row exists.
    pub next_due_at: Option<DateTime<Utc>>,
    /// Last durable-job attempt time.
    pub last_run_at: Option<DateTime<Utc>>,
    /// Path-free plain-language reason from a terminal failed run.
    pub last_error: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BackupJobPayload {
    cadence: BackupCadence,
    destination: Option<String>,
    // Accept the field written by an unshipped preview, but never persist or
    // act on it. S2a-0 keeps every backup; a future retention decision is
    // tracked separately.
    #[serde(default, rename = "keep_last", skip_serializing)]
    _legacy_keep_last: Option<u32>,
}

impl Kernel {
    /// Read configuration from this vault's durable backup job.
    pub fn backup_schedule_settings(&self) -> Result<BackupScheduleSettings, KernelError> {
        let Some(job) = self.durable_job(BACKUP_JOB_ID)? else {
            return Ok(BackupScheduleSettings {
                cadence: BackupCadence::Weekly,
                destination: None,
                next_due_at: None,
                last_run_at: None,
                last_error: None,
            });
        };
        if job.kind != BACKUP_JOB_KIND {
            return Err(KernelError::Persistence(
                "backup job identity is invalid".to_owned(),
            ));
        }
        let payload = job
            .payload_json
            .as_deref()
            .and_then(|raw| serde_json::from_str::<BackupJobPayload>(raw).ok())
            .ok_or_else(|| {
                KernelError::Persistence("backup job configuration is invalid".to_owned())
            })?;
        if payload
            .destination
            .as_deref()
            .is_some_and(|path| !Path::new(path).is_absolute())
        {
            return Err(KernelError::Persistence(
                "backup job configuration is invalid".to_owned(),
            ));
        }

        Ok(BackupScheduleSettings {
            cadence: payload.cadence,
            destination: payload.destination.map(PathBuf::from),
            next_due_at: job.next_due_at,
            last_run_at: job.last_run_at,
            last_error: job.last_error,
        })
    }

    /// Persist the user-selected folder and cadence in the encrypted job row
    /// for this vault. Scheduled backups are keep-all; this payload contains no
    /// automatic-retention policy.
    pub fn configure_backup_schedule(
        &self,
        cadence: BackupCadence,
        destination: Option<&Path>,
    ) -> Result<BackupScheduleSettings, KernelError> {
        let destination = destination
            .map(|path| {
                if !path.is_absolute() {
                    return Err(KernelError::Validation(
                        "backup destination must be an absolute folder".to_owned(),
                    ));
                }
                let canonical = std::fs::canonicalize(path).map_err(|_| {
                    KernelError::Validation("backup folder is unavailable".to_owned())
                })?;
                if !canonical.is_dir() {
                    return Err(KernelError::Validation(
                        "backup destination must be a folder".to_owned(),
                    ));
                }
                canonical.to_str().map(str::to_owned).ok_or_else(|| {
                    KernelError::Validation("backup folder path is not supported".to_owned())
                })
            })
            .transpose()?;

        let payload = BackupJobPayload {
            cadence,
            destination,
            _legacy_keep_last: None,
        };
        let payload_json = serde_json::to_string(&payload).map_err(|_| {
            KernelError::Persistence("backup configuration could not be saved".to_owned())
        })?;
        let schedule = cadence.schedule();
        let now = Utc::now();
        let next_due_at = schedule
            .next_due_after(now)
            .unwrap_or_else(|| now + Duration::weeks(1));
        let enabled = cadence.enabled(payload.destination.as_deref());
        let spec = JobSpec {
            id: BACKUP_JOB_ID,
            kind: BACKUP_JOB_KIND.to_owned(),
            schedule,
            next_due_at,
            max_attempts: 3,
            backoff: BackoffPolicy::default(),
            enabled,
            requires_explicit_opt_in: false,
            payload_json: Some(payload_json),
        };
        self.worker.reconfigure_job(&spec).map_err(|_| {
            KernelError::Persistence("backup settings could not be saved".to_owned())
        })?;
        self.backup_schedule_settings()
    }

    /// Local history for the active vault only.
    pub fn backup_history(&self) -> Result<Vec<BackupHistoryEntry>, KernelError> {
        let vault_id = self.vault_metadata()?.vault_id;
        Ok(self.worker.backup_history(vault_id)?)
    }

    /// Export a user-selected backup file and record its verified receipt.
    pub fn export_manual_backup(
        &self,
        package_path: &Path,
        app_version: &str,
    ) -> Result<BackupHistoryEntry, KernelError> {
        let result = validate_backup_file_path(package_path).and_then(|()| {
            // Validated above: absolute, `.pcfobk`, an existing parent, UTF-8.
            let name = package_path.file_name().and_then(|value| value.to_str());
            let folder = package_path.parent();
            match (folder, name) {
                (Some(folder), Some(name)) => self.perform_backup_export(
                    &FolderSink::new(folder),
                    name,
                    app_version,
                    BackupHistoryKind::Manual,
                    PutMode::Replace,
                ),
                _ => Err(KernelError::Validation(
                    "backup file path is not supported".to_owned(),
                )),
            }
        });
        self.record_backup_outcome(&result);
        result
    }

    /// Run the shared backup exporter immediately using the configured folder.
    pub fn run_backup_now(&self, app_version: &str) -> Result<BackupHistoryEntry, KernelError> {
        let result = self.run_backup_now_inner(app_version);
        self.record_backup_outcome(&result);
        result
    }

    fn run_backup_now_inner(&self, app_version: &str) -> Result<BackupHistoryEntry, KernelError> {
        let settings = self.backup_schedule_settings()?;
        let folder = settings.destination.ok_or_else(|| {
            KernelError::Validation("choose a backup folder in Settings first".to_owned())
        })?;
        self.backup_to_sink(
            &FolderSink::new(folder),
            app_version,
            BackupHistoryKind::Manual,
        )
    }

    /// Execute one durable scheduled-backup invocation.
    pub fn run_scheduled_backup(
        &self,
        app_version: &str,
    ) -> Result<BackupHistoryEntry, KernelError> {
        let result = self.run_scheduled_backup_inner(app_version);
        self.record_backup_outcome(&result);
        result
    }

    /// [`Kernel::run_scheduled_backup`] that stops for a cancel request: `cancelled` is read
    /// between the export's steps (blob reads, the snapshot, the manifest and frame steps, the
    /// seal, each megabyte written, and before the read-back, the verification and the receipt),
    /// and a request seen before the receipt is recorded removes what the run wrote and returns
    /// [`BackupRun::Cancelled`] (personal-cfo-g3m.29). The seal is one call and cannot be
    /// interrupted; the longest uninterrupted step is that call over the whole vault.
    ///
    /// # Errors
    /// As [`Kernel::run_scheduled_backup`].
    pub fn run_scheduled_backup_cancellable(
        &self,
        app_version: &str,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<BackupRun, KernelError> {
        match self.scheduled_backup_target() {
            Ok(folder) => self.run_backup_to_sink_cancellable(
                &FolderSink::new(folder),
                app_version,
                BackupHistoryKind::Scheduled,
                cancelled,
            ),
            Err(error) => {
                let result: Result<(), KernelError> = Err(error);
                self.record_backup_outcome(&result);
                result.map(|()| BackupRun::Cancelled)
            }
        }
    }

    /// [`Kernel::run_backup_to_sink`] that stops for a cancel request, as
    /// [`Kernel::run_scheduled_backup_cancellable`] does.
    ///
    /// # Errors
    /// As [`Kernel::run_backup_to_sink`].
    pub fn run_backup_to_sink_cancellable(
        &self,
        sink: &dyn BackupSink,
        app_version: &str,
        kind: BackupHistoryKind,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<BackupRun, KernelError> {
        let result = self.vault_metadata().and_then(|metadata| {
            let name = scheduled_backup_name(metadata.vault_id, Utc::now());
            self.perform_backup_export_cancellable(
                sink,
                &name,
                app_version,
                kind,
                PutMode::CreateNew,
                cancelled,
                |kernel, package, backup_id| {
                    kernel
                        .verify_unattended_backup_bytes(package, backup_id)
                        .map(|_| ())
                },
            )
        });
        // A cancelled run is neither a success nor a failure for the diagnostics counter.
        if !matches!(result, Ok(None)) {
            self.record_backup_outcome(&result);
        }
        result.map(|entry| entry.map_or(BackupRun::Cancelled, BackupRun::Done))
    }

    fn scheduled_backup_target(&self) -> Result<PathBuf, KernelError> {
        let settings = self.backup_schedule_settings()?;
        if settings.cadence == BackupCadence::Off {
            return Err(KernelError::Validation(
                "scheduled backups are turned off".to_owned(),
            ));
        }
        settings.destination.ok_or_else(|| {
            KernelError::Validation("choose a backup folder in Settings first".to_owned())
        })
    }

    fn run_scheduled_backup_inner(
        &self,
        app_version: &str,
    ) -> Result<BackupHistoryEntry, KernelError> {
        let settings = self.backup_schedule_settings()?;
        if settings.cadence == BackupCadence::Off {
            return Err(KernelError::Validation(
                "scheduled backups are turned off".to_owned(),
            ));
        }
        let folder = settings.destination.ok_or_else(|| {
            KernelError::Validation("choose a backup folder in Settings first".to_owned())
        })?;
        self.backup_to_sink(
            &FolderSink::new(folder),
            app_version,
            BackupHistoryKind::Scheduled,
        )
    }

    /// Write one new verified backup to `sink` and record its receipt.
    ///
    /// The container is assembled from the unlocked vault's in-memory key (ADR
    /// 0024-A), stored under a fresh timestamped name that can never replace an
    /// earlier backup ([`PutMode::CreateNew`]), read back from the sink and verified
    /// against that same key, and only then recorded as verified. This is the one
    /// path the folder destination and every other destination share; nothing is
    /// ever deleted (keep-all). A failure records a path-free receipt and returns the
    /// same fixed error text as a folder failure always did.
    ///
    /// # Errors
    /// [`KernelError::Vault`] with a fixed message if the write or the verification
    /// fails; [`KernelError::Persistence`] if the receipt cannot be saved.
    pub fn run_backup_to_sink(
        &self,
        sink: &dyn BackupSink,
        app_version: &str,
        kind: BackupHistoryKind,
    ) -> Result<BackupHistoryEntry, KernelError> {
        let result = self.backup_to_sink(sink, app_version, kind);
        self.record_backup_outcome(&result);
        result
    }

    fn backup_to_sink(
        &self,
        sink: &dyn BackupSink,
        app_version: &str,
        kind: BackupHistoryKind,
    ) -> Result<BackupHistoryEntry, KernelError> {
        let name = scheduled_backup_name(self.vault_metadata()?.vault_id, Utc::now());
        self.perform_backup_export(sink, &name, app_version, kind, PutMode::CreateNew)
    }

    fn perform_backup_export(
        &self,
        sink: &dyn BackupSink,
        name: &str,
        app_version: &str,
        kind: BackupHistoryKind,
        mode: PutMode,
    ) -> Result<BackupHistoryEntry, KernelError> {
        self.perform_backup_export_with_verifier(
            sink,
            name,
            app_version,
            kind,
            mode,
            |kernel, package, backup_id| {
                kernel
                    .verify_unattended_backup_bytes(package, backup_id)
                    .map(|_| ())
            },
        )
    }

    fn perform_backup_export_with_verifier(
        &self,
        sink: &dyn BackupSink,
        name: &str,
        app_version: &str,
        kind: BackupHistoryKind,
        mode: PutMode,
        verify: impl FnOnce(&Self, &[u8], Uuid) -> Result<(), KernelError>,
    ) -> Result<BackupHistoryEntry, KernelError> {
        // Never cancelled: the manual and on-demand paths have no cancel request to read.
        self.perform_backup_export_cancellable(
            sink,
            name,
            app_version,
            kind,
            mode,
            &|| false,
            verify,
        )
        .and_then(|entry| entry.ok_or_else(|| KernelError::Vault("backup cancelled".to_owned())))
    }

    /// The export, with checkpoints. `Ok(None)` is a cancel seen before the receipt was recorded,
    /// with the destination left exactly as it was.
    #[allow(clippy::too_many_arguments)]
    fn perform_backup_export_cancellable(
        &self,
        sink: &dyn BackupSink,
        name: &str,
        app_version: &str,
        kind: BackupHistoryKind,
        mode: PutMode,
        cancelled: &dyn Fn() -> bool,
        verify: impl FnOnce(&Self, &[u8], Uuid) -> Result<(), KernelError>,
    ) -> Result<Option<BackupHistoryEntry>, KernelError> {
        let vault_id = self.vault_metadata()?.vault_id;
        let backup_id = Uuid::now_v7();
        let created_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        let assembled = self.assemble_unattended_backup_cancellable(
            app_version,
            created_at.clone(),
            backup_id,
            cancelled,
        );
        let stored = match assembled {
            Ok(None) => return Ok(None),
            Ok(Some(package)) => match sink.put_cancellable(name, &package, mode, cancelled) {
                Ok(PutOutcome::Cancelled) => return Ok(None),
                Ok(PutOutcome::Stored(size)) => Ok(size),
                Err(error) => Err(error),
            },
            Err(_) => Err(BackupSinkError),
        };
        // Past the write: a cancel from here removes the item this run created, so the
        // destination is as it was. A sink that cannot remove it finishes the run instead.
        let undo = |sink: &dyn BackupSink| -> bool {
            mode == PutMode::CreateNew && sink.discard_created(name).is_ok()
        };
        if stored.is_ok() && cancelled() && undo(sink) {
            return Ok(None);
        }
        let failure = match &stored {
            Err(_) => Some("Could not write the backup file."),
            Ok(_) => {
                let read = sink
                    .get(name)
                    .map_err(|_| KernelError::Vault("reading backup failed".to_owned()));
                if read.is_ok() && cancelled() && undo(sink) {
                    return Ok(None);
                }
                read.and_then(|package| verify(self, &package, backup_id))
                    .err()
                    .map(|_| "Could not verify the backup file.")
            }
        };
        // The last look before the receipt, which is the commit: a request seen here undoes the
        // run; one that arrives after the receipt is too late.
        if failure.is_none() && cancelled() && undo(sink) {
            return Ok(None);
        }
        let entry = BackupHistoryEntry {
            backup_id,
            vault_id,
            created_at,
            kind,
            destination: sink.describe(name),
            format_version: 2,
            size_bytes: stored.unwrap_or(0),
            verified: failure.is_none(),
            error: failure.map(str::to_owned),
        };
        // A scheduled receipt is the scheduled job's durable effect: its commit marker goes in the same
        // transaction as the receipt (personal-cfo-g3m.31).
        let job = matches!(entry.kind, BackupHistoryKind::Scheduled).then_some(BACKUP_JOB_ID);
        self.worker
            .record_backup_history_for_job(&entry, job)
            .map_err(|_| {
                KernelError::Persistence("backup history could not be saved".to_owned())
            })?;
        if let Some(error) = entry.error.as_deref() {
            return Err(KernelError::Vault(error.to_owned()));
        }
        Ok(Some(entry))
    }
}

fn validate_backup_file_path(path: &Path) -> Result<(), KernelError> {
    if !path.is_absolute() || path.extension().and_then(|value| value.to_str()) != Some("pcfobk") {
        return Err(KernelError::Validation(
            "choose an absolute .pcfobk backup file".to_owned(),
        ));
    }
    if path.parent().is_none_or(|parent| !parent.is_dir()) {
        return Err(KernelError::Validation(
            "backup destination folder is unavailable".to_owned(),
        ));
    }
    if path.to_str().is_none() {
        return Err(KernelError::Validation(
            "backup file path is not supported".to_owned(),
        ));
    }
    Ok(())
}

fn scheduled_backup_name(vault_id: Uuid, now: DateTime<Utc>) -> String {
    let timestamp = format!(
        "{}{:09}Z",
        now.format("%Y%m%dT%H%M%S"),
        now.timestamp_subsec_nanos()
    );
    format!("{vault_id}-{timestamp}.pcfobk")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn legacy_retention_value_is_ignored_and_not_serialized() {
        let payload: BackupJobPayload = serde_json::from_str(
            r#"{"cadence":"weekly","destination":"/tmp/backups","keep_last":3}"#,
        )
        .unwrap();

        assert_eq!(payload.cadence, BackupCadence::Weekly);
        assert_eq!(payload.destination.as_deref(), Some("/tmp/backups"));
        let rewritten = serde_json::to_string(&payload).unwrap();
        assert!(!rewritten.contains("keep_last"));
    }

    #[test]
    fn a_cancel_that_arrives_during_verification_stops_before_the_receipt_and_removes_the_file() {
        let root = tempdir().unwrap();
        let backup_dir = root.path().join("backups");
        std::fs::create_dir(&backup_dir).unwrap();
        let kernel =
            Kernel::create_vault(root.path().join("vault.db"), b"correct horse battery").unwrap();
        let verified = std::cell::Cell::new(false);

        let result = kernel
            .perform_backup_export_cancellable(
                &FolderSink::new(&backup_dir),
                "during-verification.pcfobk",
                "0.2.0-test",
                BackupHistoryKind::Scheduled,
                PutMode::CreateNew,
                &|| verified.get(),
                |_, _, _| {
                    // The request lands while the read-back is being verified.
                    verified.set(true);
                    Ok(())
                },
            )
            .unwrap();

        assert!(result.is_none(), "the run was cancelled before its receipt");
        assert!(std::fs::read_dir(&backup_dir).unwrap().next().is_none());
        assert!(kernel.backup_history().unwrap().is_empty());
    }

    #[test]
    fn verification_failure_after_write_preserves_output_and_records_safe_receipt() {
        let root = tempdir().unwrap();
        let backup_dir = root.path().join("backups");
        std::fs::create_dir(&backup_dir).unwrap();
        let kernel =
            Kernel::create_vault(root.path().join("vault.db"), b"correct horse battery").unwrap();
        let package_path = backup_dir.join("failed-verification.pcfobk");

        let error = kernel
            .perform_backup_export_with_verifier(
                &FolderSink::new(&backup_dir),
                "failed-verification.pcfobk",
                "0.2.0-test",
                BackupHistoryKind::Scheduled,
                PutMode::CreateNew,
                |_, _, _| Err(KernelError::Vault("injected verifier failure".to_owned())),
            )
            .unwrap_err();

        assert!(matches!(
            error,
            KernelError::Vault(ref message) if message == "Could not verify the backup file."
        ));
        assert!(package_path.is_file());
        assert!(std::fs::metadata(&package_path).unwrap().len() > 0);

        let history = kernel.backup_history().unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].destination, package_path.to_string_lossy());
        assert_eq!(history[0].kind, BackupHistoryKind::Scheduled);
        assert!(!history[0].verified);
        assert_eq!(
            history[0].error.as_deref(),
            Some("Could not verify the backup file.")
        );
    }
}
