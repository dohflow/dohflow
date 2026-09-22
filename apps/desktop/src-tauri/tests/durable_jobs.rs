//! Desktop lifecycle coverage for the local durable-job dispatcher (personal-cfo-ati).
//!
//! These tests exercise the same AppState and command implementation used by
//! Tauri. They keep the handler local and deterministic; backup/connector
//! consumers register their own handlers in their owning beads.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::{Duration as StdDuration, Instant};

use app_lib::ipc::commands::{
    create_vault_impl, lock_vault_impl, run_due_jobs_on_unlock_impl,
    spawn_due_jobs_on_unlock_for_state, unlock_vault_impl,
};
use app_lib::AppState;
use finance_kernel::{
    BackoffPolicy, CancellationToken, JobExecution, JobHandler, JobRecord, JobSpec, JobState,
    Kernel, Schedule, VaultController,
};
use tempfile::TempDir;
use uuid::Uuid;

const PASSWORD: &str = "correct horse battery staple";

fn due_spec(id: Uuid, kind: &str) -> JobSpec {
    JobSpec {
        id,
        kind: kind.to_owned(),
        schedule: Schedule::Once,
        next_due_at: chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
        max_attempts: 1,
        backoff: BackoffPolicy::default(),
        enabled: true,
        requires_explicit_opt_in: false,
        payload_json: Some("opaque payload stays in the encrypted vault".to_owned()),
    }
}

struct Complete;

impl JobHandler<Kernel> for Complete {
    fn kind(&self) -> &'static str {
        "desktop_test"
    }

    fn execute(
        &self,
        _job: &JobRecord,
        _cancellation: &CancellationToken,
        _context: &Kernel,
    ) -> JobExecution {
        JobExecution::Succeeded
    }
}

struct BlockingComplete {
    started: Arc<AtomicBool>,
    release: Arc<AtomicBool>,
}

impl JobHandler<Kernel> for BlockingComplete {
    fn kind(&self) -> &'static str {
        "desktop_blocking_test"
    }

    fn execute(
        &self,
        _job: &JobRecord,
        cancellation: &CancellationToken,
        _context: &Kernel,
    ) -> JobExecution {
        self.started.store(true, Ordering::Release);
        while !self.release.load(Ordering::Acquire) {
            if cancellation.is_cancelled() {
                return JobExecution::Cancelled;
            }
            thread::sleep(StdDuration::from_millis(2));
        }
        JobExecution::Succeeded
    }
}

fn fresh_state() -> (TempDir, AppState) {
    let dir = TempDir::new().expect("temp dir");
    let controller = VaultController::open(dir.path().join("vault.db"));
    (dir, AppState::new(controller))
}

fn schedule(state: &AppState, spec: &JobSpec) {
    state
        .lock_controller()
        .expect("controller lock")
        .kernel()
        .expect("unlocked kernel")
        .schedule_job(spec)
        .expect("schedule durable job");
}

fn read_job(state: &AppState, id: Uuid) -> finance_kernel::DurableJobView {
    state
        .lock_controller()
        .expect("controller lock")
        .kernel()
        .expect("unlocked kernel")
        .durable_job(id)
        .expect("read durable job")
        .expect("durable row")
}

#[test]
fn production_dispatcher_is_nonblocking_and_debounces_one_unlock_window() {
    let (_dir, state) = fresh_state();
    create_vault_impl(&state, PASSWORD.to_owned()).expect("create vault");
    let id = Uuid::now_v7();
    schedule(&state, &due_spec(id, "desktop_blocking_test"));

    let started = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    state
        .register_job_handler(Arc::new(BlockingComplete {
            started: Arc::clone(&started),
            release: Arc::clone(&release),
        }))
        .expect("register desktop test handler");
    let state = Arc::new(state);
    let dispatcher = state.job_dispatcher();

    let dispatch_started = Instant::now();
    let handle = spawn_due_jobs_on_unlock_for_state(
        Arc::clone(&state),
        dispatcher,
        "unlock-desktop-test".to_owned(),
    );
    assert!(
        dispatch_started.elapsed() < StdDuration::from_millis(100),
        "post-unlock dispatch must return without waiting for the handler"
    );

    let wait_started = Instant::now();
    while !started.load(Ordering::Acquire) {
        assert!(
            wait_started.elapsed() < StdDuration::from_secs(2),
            "blocking handler should be scheduled"
        );
        thread::sleep(StdDuration::from_millis(2));
    }
    release.store(true, Ordering::Release);
    let report = tauri::async_runtime::block_on(handle)
        .expect("scheduler task joins")
        .expect("scheduler succeeds");
    assert_eq!(report.attempted, 1);
    assert_eq!(report.succeeded, 1);

    let same_window = run_due_jobs_on_unlock_impl(
        &state,
        state.job_dispatcher().as_ref(),
        "unlock-desktop-test",
    )
    .expect("same-window scheduler sweep");
    assert_eq!(same_window.attempted, 0);
    assert_eq!(read_job(&state, id).state, JobState::Succeeded);
}

#[test]
fn unlock_reopen_dispatch_persists_the_terminal_outcome() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("vault.db");
    let first = AppState::new(VaultController::open(&path));
    create_vault_impl(&first, PASSWORD.to_owned()).expect("create vault");
    let id = Uuid::now_v7();
    schedule(&first, &due_spec(id, "desktop_test"));
    lock_vault_impl(&first).expect("lock vault");
    drop(first);

    let reopened = AppState::new(VaultController::open(&path));
    unlock_vault_impl(&reopened, PASSWORD.to_owned()).expect("unlock vault");
    reopened
        .register_job_handler(Arc::new(Complete))
        .expect("register desktop test handler");
    let dispatcher = reopened.job_dispatcher();
    let report = run_due_jobs_on_unlock_impl(&reopened, dispatcher.as_ref(), "unlock-reopened")
        .expect("run reopened scheduler");

    assert_eq!(report.attempted, 1);
    assert_eq!(report.succeeded, 1);
    let row = read_job(&reopened, id);
    assert_eq!(row.state, JobState::Succeeded);
    assert_eq!(
        row.last_outcome,
        Some(finance_kernel::JobOutcome::Succeeded)
    );
    assert!(row.last_run_at.is_some());
}
