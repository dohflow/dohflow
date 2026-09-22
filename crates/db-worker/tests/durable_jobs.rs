//! Durable local job runtime integration tests (personal-cfo-ati).
//!
//! These exercise the real SQLCipher-backed worker rather than an in-memory
//! replacement: scheduling, claim/debounce, retry state, crash recovery, and
//! cancellation all survive the same persistence boundary the desktop uses.

mod common;

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration as StdDuration, Instant};

use chrono::{DateTime, Duration, Utc};
use common::worker;
use db_worker::DbWorker;
use db_worker::JobStoreError;
use job_runtime::{
    BackoffPolicy, CancellationToken, Clock, JobExecution, JobExecutor, JobFinishResult, JobRecord,
    JobRunner, JobSpec, JobState, JobStore, Schedule,
};
use rusqlite::params;
use uuid::Uuid;

struct Succeed;

impl JobExecutor<()> for Succeed {
    fn execute(
        &self,
        _job: &JobRecord,
        _cancellation: &CancellationToken,
        _context: &(),
    ) -> JobExecution {
        JobExecution::Succeeded
    }
}

struct FailThenSucceed {
    calls: AtomicU32,
}

impl JobExecutor<()> for FailThenSucceed {
    fn execute(
        &self,
        _job: &JobRecord,
        _cancellation: &CancellationToken,
        _context: &(),
    ) -> JobExecution {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            JobExecution::Failed(job_runtime::JobFailure::retryable(
                "temporary provider failure",
            ))
        } else {
            JobExecution::Succeeded
        }
    }
}

struct Cancel;

impl JobExecutor<()> for Cancel {
    fn execute(
        &self,
        _job: &JobRecord,
        cancellation: &CancellationToken,
        _context: &(),
    ) -> JobExecution {
        cancellation.cancel();
        JobExecution::Cancelled
    }
}

struct PermanentFailure;

impl JobExecutor<()> for PermanentFailure {
    fn execute(
        &self,
        _job: &JobRecord,
        _cancellation: &CancellationToken,
        _context: &(),
    ) -> JobExecution {
        JobExecution::Failed(job_runtime::JobFailure::permanent(
            "backup destination is unavailable",
        ))
    }
}

struct Skip;

impl JobExecutor<()> for Skip {
    fn execute(
        &self,
        _job: &JobRecord,
        _cancellation: &CancellationToken,
        _context: &(),
    ) -> JobExecution {
        JobExecution::Skipped
    }
}

struct WaitForCancellation;

impl JobExecutor<()> for WaitForCancellation {
    fn execute(
        &self,
        _job: &JobRecord,
        cancellation: &CancellationToken,
        _context: &(),
    ) -> JobExecution {
        while !cancellation.is_cancelled() {
            std::thread::sleep(StdDuration::from_millis(5));
        }
        JobExecution::Cancelled
    }
}

/// Inject a cancellation immediately before the durable terminal write. This
/// models the final-poll-to-terminal-write race without relying on scheduler
/// timing or sleeps.
struct CancelAtTerminalWrite {
    worker: Arc<DbWorker>,
}

impl JobStore for CancelAtTerminalWrite {
    type Error = JobStoreError;

    fn recover_running(&self, now: DateTime<Utc>) -> Result<u64, Self::Error> {
        self.worker.recover_running(now)
    }

    fn due_jobs(
        &self,
        now: DateTime<Utc>,
        unlock_window: &str,
    ) -> Result<Vec<JobRecord>, Self::Error> {
        self.worker.due_jobs(now, unlock_window)
    }

    fn claim_job(
        &self,
        id: Uuid,
        attempt: u32,
        started_at: DateTime<Utc>,
        unlock_window: &str,
    ) -> Result<bool, Self::Error> {
        self.worker
            .claim_job(id, attempt, started_at, unlock_window)
    }

    fn cancellation_requested(&self, id: Uuid) -> Result<bool, Self::Error> {
        self.worker.cancellation_requested(id)
    }

    fn finish_success(
        &self,
        job: &JobRecord,
        completed_at: DateTime<Utc>,
        next_due_at: Option<DateTime<Utc>>,
    ) -> Result<JobFinishResult, Self::Error> {
        self.worker.cancel_job(job.id)?;
        self.worker.finish_success(job, completed_at, next_due_at)
    }

    fn finish_failure(
        &self,
        job: &JobRecord,
        failed_at: DateTime<Utc>,
        failure: &job_runtime::JobFailure,
        retry_at: Option<DateTime<Utc>>,
    ) -> Result<JobFinishResult, Self::Error> {
        self.worker.cancel_job(job.id)?;
        self.worker
            .finish_failure(job, failed_at, failure, retry_at)
    }

    fn finish_cancelled(
        &self,
        job: &JobRecord,
        cancelled_at: DateTime<Utc>,
        reason: &str,
    ) -> Result<(), Self::Error> {
        self.worker.finish_cancelled(job, cancelled_at, reason)
    }

    fn finish_skipped(
        &self,
        job: &JobRecord,
        skipped_at: DateTime<Utc>,
    ) -> Result<JobFinishResult, Self::Error> {
        self.worker.cancel_job(job.id)?;
        self.worker.finish_skipped(job, skipped_at)
    }
}

#[derive(Clone, Copy)]
struct FixedClock(DateTime<Utc>);

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).expect("fixed test timestamp is valid")
}

fn spec(id: Uuid, schedule: Schedule) -> JobSpec {
    JobSpec {
        id,
        kind: "test_job".to_owned(),
        schedule,
        next_due_at: now() - Duration::seconds(1),
        max_attempts: 3,
        backoff: BackoffPolicy {
            initial: Duration::seconds(30),
            multiplier_bps: 20_000,
        },
        enabled: true,
        requires_explicit_opt_in: false,
        payload_json: Some(r#"{"safe":"opaque"}"#.to_owned()),
    }
}

fn set_due_now(worker: &DbWorker, id: Uuid) {
    let conn = worker.read_connection().unwrap();
    conn.execute(
        "UPDATE durable_jobs SET next_due_at = ?2 WHERE id = ?1",
        params![id, now().to_rfc3339()],
    )
    .unwrap();
}

#[test]
fn one_shot_job_claims_once_per_unlock_and_persists_terminal_state() {
    let (dir, worker) = worker();
    let id = Uuid::now_v7();
    worker.schedule_job(&spec(id, Schedule::Once)).unwrap();
    let clock = FixedClock(now());

    let first = worker
        .run_due_jobs_with_clock(
            now(),
            "unlock-1",
            &(),
            &Succeed,
            &CancellationToken::new(),
            &clock,
        )
        .unwrap();
    assert_eq!(first.attempted, 1);
    assert_eq!(first.succeeded, 1);
    assert_eq!(
        worker.durable_job(id).unwrap().unwrap().state,
        JobState::Succeeded
    );

    let same_window = worker
        .run_due_jobs_with_clock(
            now(),
            "unlock-1",
            &(),
            &Succeed,
            &CancellationToken::new(),
            &clock,
        )
        .unwrap();
    assert_eq!(same_window.attempted, 0);

    drop(worker);
    let reopened = DbWorker::open(dir.path().join("test.vault"), common::KEY).unwrap();
    let persisted = reopened.durable_job(id).unwrap().unwrap();
    assert_eq!(persisted.state, JobState::Succeeded);
    assert_eq!(
        persisted.last_outcome,
        Some(job_runtime::JobOutcome::Succeeded)
    );
    assert!(persisted.next_due_at.is_none(), "once jobs do not recur");
}

#[test]
fn retryable_failure_uses_backoff_and_then_succeeds() {
    let (_dir, worker) = worker();
    let id = Uuid::now_v7();
    worker.schedule_job(&spec(id, Schedule::Once)).unwrap();
    let executor = FailThenSucceed {
        calls: AtomicU32::new(0),
    };
    let clock = FixedClock(now());

    let first = worker
        .run_due_jobs_with_clock(
            now(),
            "unlock-1",
            &(),
            &executor,
            &CancellationToken::new(),
            &clock,
        )
        .unwrap();
    assert_eq!(first.failed, 1);
    let after_failure = worker.durable_job(id).unwrap().unwrap();
    assert_eq!(after_failure.state, JobState::Queued);
    assert_eq!(
        after_failure.last_outcome,
        Some(job_runtime::JobOutcome::Failed)
    );
    assert!(after_failure.next_due_at.unwrap() > now());

    set_due_now(&worker, id);
    let second = worker
        .run_due_jobs_with_clock(
            now(),
            "unlock-2",
            &(),
            &executor,
            &CancellationToken::new(),
            &clock,
        )
        .unwrap();
    assert_eq!(second.succeeded, 1);
    assert_eq!(executor.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn running_rows_are_requeued_after_a_crash_and_cancellation_is_terminal() {
    let (_dir, worker) = worker();
    let clock = FixedClock(now());
    let crashed = Uuid::now_v7();
    worker.schedule_job(&spec(crashed, Schedule::Once)).unwrap();
    let conn = worker.read_connection().unwrap();
    conn.execute(
        "UPDATE durable_jobs SET state = 'running', attempt_count = 1 WHERE id = ?1",
        params![crashed],
    )
    .unwrap();

    let recovered = worker
        .run_due_jobs_with_clock(
            now(),
            "unlock-after-crash",
            &(),
            &Succeed,
            &CancellationToken::new(),
            &clock,
        )
        .unwrap();
    assert_eq!(recovered.recovered, 1);
    assert_eq!(recovered.succeeded, 1);

    let cancelled = Uuid::now_v7();
    worker
        .schedule_job(&spec(cancelled, Schedule::Once))
        .unwrap();
    let report = worker
        .run_due_jobs_with_clock(
            now(),
            "unlock-cancel",
            &(),
            &Cancel,
            &CancellationToken::new(),
            &clock,
        )
        .unwrap();
    assert_eq!(report.cancelled, 1);
    assert_eq!(
        worker.durable_job(cancelled).unwrap().unwrap().state,
        JobState::Cancelled
    );
}

#[test]
fn terminal_failure_is_readable_as_a_plain_money_inbox_reason() {
    let (_dir, worker) = worker();
    let clock = FixedClock(now());
    let id = Uuid::now_v7();
    worker.schedule_job(&spec(id, Schedule::Once)).unwrap();
    worker
        .run_due_jobs_with_clock(
            now(),
            "unlock-failure",
            &(),
            &PermanentFailure,
            &CancellationToken::new(),
            &clock,
        )
        .unwrap();

    let item = worker
        .money_inbox_list()
        .unwrap()
        .into_iter()
        .find(|item| item.target_id == id)
        .expect("failed durable job is actionable in Money Inbox");
    assert_eq!(item.item_kind, "job_failure");
    assert!(item
        .payload_json
        .contains("backup destination is unavailable"));
    assert!(!item.payload_json.contains("opaque"));
}

#[test]
fn db_cancel_request_reaches_a_running_handler_within_the_sla() {
    let (_dir, worker) = worker();
    let worker = Arc::new(worker);
    let id = Uuid::now_v7();
    worker.schedule_job(&spec(id, Schedule::Once)).unwrap();
    let task_worker = Arc::clone(&worker);
    let clock = FixedClock(now());
    let task = std::thread::spawn(move || {
        task_worker
            .run_due_jobs_with_clock(
                now(),
                "unlock-db-cancel",
                &(),
                &WaitForCancellation,
                &CancellationToken::new(),
                &clock,
            )
            .unwrap()
    });

    let mut running = false;
    for _ in 0..100 {
        if worker
            .durable_job(id)
            .unwrap()
            .is_some_and(|job| job.state == JobState::Running)
        {
            running = true;
            break;
        }
        std::thread::sleep(StdDuration::from_millis(2));
    }
    assert!(running, "job should be running before cancellation");
    let started = Instant::now();
    worker.cancel_job(id).unwrap();
    let report = task.join().unwrap();
    assert_eq!(report.cancelled, 1);
    assert!(started.elapsed() < StdDuration::from_millis(500));
}

#[test]
fn cancellation_wins_the_final_poll_to_success_write_race() {
    let (_dir, worker) = worker();
    let worker = Arc::new(worker);
    let id = Uuid::now_v7();
    worker.schedule_job(&spec(id, Schedule::Once)).unwrap();
    let store = CancelAtTerminalWrite {
        worker: Arc::clone(&worker),
    };
    let report = JobRunner::new(&store)
        .run_due_with_clock(
            now(),
            "unlock-terminal-race",
            &(),
            &Succeed,
            &CancellationToken::new(),
            &FixedClock(now()),
        )
        .unwrap();

    assert_eq!(report.succeeded, 0);
    assert_eq!(report.cancelled, 1);
    assert_eq!(
        worker.durable_job(id).unwrap().unwrap().state,
        JobState::Cancelled
    );
}

#[test]
fn cancellation_wins_the_final_poll_to_failure_write_race() {
    let (_dir, worker) = worker();
    let worker = Arc::new(worker);
    let id = Uuid::now_v7();
    worker.schedule_job(&spec(id, Schedule::Once)).unwrap();
    let store = CancelAtTerminalWrite {
        worker: Arc::clone(&worker),
    };
    let report = JobRunner::new(&store)
        .run_due_with_clock(
            now(),
            "unlock-terminal-failure-race",
            &(),
            &PermanentFailure,
            &CancellationToken::new(),
            &FixedClock(now()),
        )
        .unwrap();

    assert_eq!(report.failed, 0);
    assert_eq!(report.cancelled, 1);
    assert_eq!(
        worker.durable_job(id).unwrap().unwrap().state,
        JobState::Cancelled
    );
}

#[test]
fn cancellation_wins_the_final_poll_to_executor_skipped_write_race() {
    let (_dir, worker) = worker();
    let worker = Arc::new(worker);
    let id = Uuid::now_v7();
    worker.schedule_job(&spec(id, Schedule::Once)).unwrap();
    let store = CancelAtTerminalWrite {
        worker: Arc::clone(&worker),
    };
    let report = JobRunner::new(&store)
        .run_due_with_clock(
            now(),
            "unlock-terminal-skipped-race",
            &(),
            &Skip,
            &CancellationToken::new(),
            &FixedClock(now()),
        )
        .unwrap();

    assert_eq!(report.skipped, 0);
    assert_eq!(report.cancelled, 1);
    let row = worker.durable_job(id).unwrap().unwrap();
    assert_eq!(row.state, JobState::Cancelled);
    assert_eq!(row.last_outcome, Some(job_runtime::JobOutcome::Cancelled));
}
