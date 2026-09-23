//! The desktop adapter for the scheduled backup job.

use finance_kernel::{
    BackupCadence, JobExecution, JobFailure, JobHandler, JobRecord, Kernel, BACKUP_JOB_ID,
    BACKUP_JOB_KIND,
};
use job_runtime::CancellationToken;

pub(crate) struct BackupJobHandler;

impl JobHandler<Kernel> for BackupJobHandler {
    fn kind(&self) -> &'static str {
        BACKUP_JOB_KIND
    }

    fn should_run(&self, job: &JobRecord, context: &Kernel) -> bool {
        if job.id != BACKUP_JOB_ID {
            return false;
        }
        context.backup_schedule_settings().is_ok_and(|settings| {
            settings.cadence != BackupCadence::Off && settings.destination.is_some()
        })
    }

    fn execute(
        &self,
        job: &JobRecord,
        cancellation: &CancellationToken,
        context: &Kernel,
    ) -> JobExecution {
        if cancellation.is_cancelled() {
            return JobExecution::Cancelled;
        }
        if job.id != BACKUP_JOB_ID {
            return JobExecution::Failed(JobFailure::permanent(
                "Invalid scheduled backup job configuration.",
            ));
        }

        match context.run_scheduled_backup(env!("CARGO_PKG_VERSION")) {
            Ok(_) => JobExecution::Succeeded,
            Err(_) => {
                // Kernel errors may contain a local path. Keep the durable
                // failure, Money Inbox copy, and logs path-free.
                tracing::warn!(outcome = "failed", "scheduled backup export failed");
                JobExecution::Failed(JobFailure::retryable(
                    "Could not create or verify the scheduled backup. Check the destination folder in Settings.",
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use chrono::{DateTime, Duration, Utc};
    use finance_kernel::{
        BackoffPolicy, BackupCadence, BackupHistoryKind, Clock, JobSpec, JobState, Schedule,
        VaultController,
    };
    use tempfile::tempdir;

    use crate::{ipc::commands::run_due_jobs_on_unlock_with_clock_impl, state::AppState};

    struct FixedClock(DateTime<Utc>);

    impl Clock for FixedClock {
        fn now(&self) -> DateTime<Utc> {
            self.0
        }
    }

    #[test]
    fn due_backup_runs_after_unlock_once_and_records_verified_receipt() {
        let directory = tempdir().unwrap();
        let destination = directory.path().join("cloud-sync");
        let now = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let clock = FixedClock(now);
        fs::create_dir(&destination).unwrap();
        let mut controller = VaultController::open(directory.path().join("vault.db"));
        controller.create(b"test-only backup password").unwrap();
        let state = AppState::new(controller);

        {
            let guard = state.lock_controller().unwrap();
            let kernel = guard.kernel().unwrap();
            kernel
                .configure_backup_schedule(BackupCadence::Weekly, Some(&destination))
                .unwrap();
            let job = kernel.durable_job(super::BACKUP_JOB_ID).unwrap().unwrap();
            kernel
                .schedule_job(&JobSpec {
                    id: job.id,
                    kind: job.kind,
                    schedule: Schedule::Weekly,
                    next_due_at: now - Duration::seconds(1),
                    max_attempts: job.max_attempts,
                    backoff: BackoffPolicy::default(),
                    enabled: job.enabled,
                    requires_explicit_opt_in: job.requires_explicit_opt_in,
                    payload_json: job.payload_json,
                })
                .unwrap();
        }

        let dispatcher = state.job_dispatcher();
        assert!(dispatcher.contains(super::BACKUP_JOB_KIND));
        let first = run_due_jobs_on_unlock_with_clock_impl(
            &state,
            dispatcher.as_ref(),
            "unlock-test-1",
            &clock,
        )
        .unwrap();
        assert_eq!(first.attempted, 1);
        assert_eq!(first.succeeded, 1);

        let repeated = run_due_jobs_on_unlock_with_clock_impl(
            &state,
            dispatcher.as_ref(),
            "unlock-test-1",
            &clock,
        )
        .unwrap();
        assert_eq!(repeated.attempted, 0, "one unlock window runs at most once");

        let guard = state.lock_controller().unwrap();
        let history = guard.kernel().unwrap().backup_history().unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].kind, BackupHistoryKind::Scheduled);
        assert!(history[0].verified);
        assert!(std::path::Path::new(&history[0].destination).exists());
        assert!(std::path::Path::new(&history[0].destination)
            .starts_with(destination.canonicalize().unwrap()));
    }

    #[test]
    fn terminal_scheduled_failure_reaches_money_inbox_without_destination_path() {
        let directory = tempdir().unwrap();
        let destination = directory.path().join("cloud-sync");
        fs::create_dir(&destination).unwrap();
        let mut controller = VaultController::open(directory.path().join("vault.db"));
        controller.create(b"test-only backup password").unwrap();
        let state = AppState::new(controller);
        let now = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let mut clock = FixedClock(now);

        {
            let guard = state.lock_controller().unwrap();
            let kernel = guard.kernel().unwrap();
            kernel
                .configure_backup_schedule(BackupCadence::Weekly, Some(&destination))
                .unwrap();
            let job = kernel.durable_job(super::BACKUP_JOB_ID).unwrap().unwrap();
            kernel
                .schedule_job(&JobSpec {
                    id: job.id,
                    kind: job.kind,
                    schedule: Schedule::Weekly,
                    next_due_at: now - Duration::seconds(1),
                    max_attempts: job.max_attempts,
                    backoff: BackoffPolicy::default(),
                    enabled: job.enabled,
                    requires_explicit_opt_in: job.requires_explicit_opt_in,
                    payload_json: job.payload_json,
                })
                .unwrap();
        }

        fs::remove_dir(&destination).unwrap();
        let dispatcher = state.job_dispatcher();
        for (attempt, elapsed_seconds) in [0, 60, 180].into_iter().enumerate() {
            clock.0 = now + Duration::seconds(elapsed_seconds);
            let report = run_due_jobs_on_unlock_with_clock_impl(
                &state,
                dispatcher.as_ref(),
                &format!("unlock-failure-{attempt}"),
                &clock,
            )
            .unwrap();
            assert_eq!(report.attempted, 1);
            assert_eq!(report.failed, 1);
        }

        let guard = state.lock_controller().unwrap();
        let kernel = guard.kernel().unwrap();
        let job = kernel.durable_job(super::BACKUP_JOB_ID).unwrap().unwrap();
        assert_eq!(job.state, JobState::Failed);
        assert_eq!(
            job.last_error.as_deref(),
            Some("Could not create or verify the scheduled backup. Check the destination folder in Settings.")
        );
        let inbox_item = kernel
            .money_inbox_list()
            .unwrap()
            .into_iter()
            .find(|item| item.item_kind == "job_failure")
            .expect("terminal scheduled backup failure is actionable in Money Inbox");
        assert!(inbox_item
            .payload_json
            .contains("Could not create or verify"));
        assert!(!inbox_item
            .payload_json
            .contains(destination.to_str().unwrap()));

        let history = kernel.backup_history().unwrap();
        assert_eq!(history.len(), 3);
        assert!(history.iter().all(|entry| !entry.verified));
    }
}
