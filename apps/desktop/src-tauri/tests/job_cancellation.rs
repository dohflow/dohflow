//! Cross-cutting cancellation suite for the durable job runtime (personal-cfo-gdsz).
//!
//! Every job kind the production app registers today is covered here: `backup_export` (the
//! dispatcher's scheduled backup, personal-cfo-8qh) and `connector_refresh` (the lock-releasing
//! kind, personal-cfo-lqk). A registry guard fails when a third kind is registered without a case
//! in this file, so a new consumer cannot ship without deciding what cancelling it means.
//!
//! Cancellation is driven through `Kernel::cancel_job`, the db-worker's request path. There is no
//! Tauri or IPC command that cancels a job today, so there is no UI hop to test; the suite says so
//! rather than inventing one.
//!
//! What it proves, and what it does not:
//! * A cancel of a queued job is terminal at once and the job never runs.
//! * A cancel racing a real scheduled backup, at many offsets, leaves only whole artifacts: every
//!   file in the destination is a complete `.pcfobk` with a matching verified receipt, no temp
//!   leftovers, and nothing at all when the job never ran.
//! * A cancel during a provider fetch stops the refresh within the SLA without waiting for the
//!   provider, writes nothing (no rows, stamp, cursor or notice change) and is recorded as
//!   cancelled; the abandoned fetch's late result never reaches the commit and the connection's
//!   sync slot is held until the fetch ends. A cancel that lands as the fetch returns, or after
//!   the commit, is decided at the commit: before it, nothing is written; after it, the job stays
//!   succeeded (personal-cfo-pxi.25).
//! * A crash between the commit and the terminal write, with NO cancel request pending, recovers to
//!   a truthful outcome and the re-run imports nothing twice. **With a cancel request also pending,
//!   recovery still records the job as cancelled although the batch is imported**, and a failure
//!   after the writes began, with a cancel pending, can be recorded as cancelled too: both need the
//!   durable commit marker of personal-cfo-g3m.31 and are the `#[ignore]`d cases of that bead below.
//! * The terminal state is durable: after the vault is closed and reopened the job is still
//!   cancelled and is not claimed again.
//! * Queued cancels are terminal at once and their P99 is under the 500 ms SLA (live: one indexed
//!   write, about a hundred times under the bound). The latency of the cancel path around a RACE
//!   is the two `#[ignore]`d perf cases, because absolute timings fail under load. Nothing in CI
//!   runs them (the perf-budget job builds only db-worker and job-runtime): run them by hand, one
//!   at a time, `cargo test --test job_cancellation -- --ignored --exact --test-threads=1 <name>`;
//!   in parallel they contend for the vault and can miss the bound.
//!
//! The backup export reads the job's cancellation at its checkpoints (personal-cfo-g3m.29): between
//! blob reads, after the snapshot, between the manifest and frame steps, around the seal, each
//! megabyte written, and before the read-back, the verification and the receipt. The race case below
//! cancels a real run at many offsets, but a small vault finishes inside a few milliseconds, so the
//! proof that EVERY checkpoint leaves the destination untouched is
//! `finance-kernel/tests/backup_cancel.rs`, which cancels at each one in turn. A backup that has
//! completed (written, verified, receipted) is reported as `Committed`, so a cancel that arrives after
//! it is too late and the job stays succeeded (the case below that lands the cancel in that window).

use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use app_lib::backup_job::backup_job_execution_observed;
use app_lib::connector_attention::{encode, AttentionEntry, AttentionKind};
use app_lib::connector_refresh::{
    refresh_job_id, run_due_connector_refresh, run_due_connector_refresh_observed,
    RefreshSweepReport,
};
use app_lib::ipc::commands::{
    connector_link_impl, connector_set_account_link_impl, create_account_impl,
    transaction_list_impl, unlock_vault_impl,
};
use app_lib::ipc::dto::{
    AccountFlagsDto, CashflowRoleDto, ConnectorLinkInput, ConnectorSetAccountLinkInput,
    CreateAccountInput,
};
use app_lib::AppState;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use connector_core::mock::MockConnector;
use connector_core::{
    CapabilitySet, Connection, ConnectorAdapter, ConnectorError, HealthStatus, LinkInput,
    LinkSession, SyncBatch,
};
use finance_kernel::{
    BackoffPolicy, BackupCadence, CancellationToken, Clock, JobDispatcher, JobExecution,
    JobExecutor, JobHandler, JobRecord, JobSpec, JobState, Kernel, ParsedAccount, ParsedBalance,
    ParsedRecord, Schedule, VaultController, BACKUP_JOB_ID, BACKUP_JOB_KIND,
    CONNECTOR_REFRESH_JOB_KIND, LOCK_RELEASING_JOB_KINDS,
};
use semver::Version;
use tempfile::TempDir;
use uuid::Uuid;

const PASSWORD: &str = "test-only cancellation password";

/// The kinds this suite has cases for. Adding a kind to the app means adding it here and writing
/// its cases below; the guard test fails until both are done.
const COVERED_KINDS: &[&str] = &[BACKUP_JOB_KIND, CONNECTOR_REFRESH_JOB_KIND];

/// The cancellation SLA of the job runtime (plan section 19.7).
const SLA: Duration = Duration::from_millis(500);

struct FixedClock(DateTime<Utc>);

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

fn base_time() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).unwrap()
}

/// The slowest of the samples, which is the 99th percentile for fewer than 100 of them.
fn p99(samples: &[Duration]) -> Duration {
    let mut sorted = samples.to_vec();
    sorted.sort();
    let index = ((sorted.len() * 99).div_ceil(100)).saturating_sub(1);
    sorted[index]
}

// ---- the registry guard ----------------------------------------------------------------------

#[test]
fn every_kind_the_app_registers_has_cancellation_cases_here() {
    let dir = TempDir::new().unwrap();
    let state = AppState::new(VaultController::open(dir.path().join("vault.db")));
    let mut registered = state.job_dispatcher().kinds();
    registered.extend(
        LOCK_RELEASING_JOB_KINDS
            .iter()
            .map(|kind| (*kind).to_owned()),
    );
    registered.sort();
    registered.dedup();

    let mut covered: Vec<String> = COVERED_KINDS
        .iter()
        .map(|kind| (*kind).to_owned())
        .collect();
    covered.sort();
    assert_eq!(
        registered, covered,
        "a job kind was added or removed: give it cancellation cases in tests/job_cancellation.rs \
         and list it in COVERED_KINDS (personal-cfo-gdsz)"
    );
}

// ---- backup_export: the dispatcher's kind ----------------------------------------------------

struct BackupFixture {
    dir: TempDir,
    destination: PathBuf,
    state: AppState,
}

impl BackupFixture {
    fn vault_path(&self) -> PathBuf {
        self.dir.path().join("vault.db")
    }
}

/// A real vault with a due scheduled backup pointed at a fresh folder.
fn backup_fixture() -> BackupFixture {
    let dir = TempDir::new().expect("temp dir");
    let destination = dir.path().join("backups");
    fs::create_dir(&destination).unwrap();
    let mut controller = VaultController::open(dir.path().join("vault.db"));
    controller.create(PASSWORD.as_bytes()).unwrap();
    let state = AppState::new(controller);
    {
        let guard = state.lock_controller().unwrap();
        let kernel = guard.kernel().unwrap();
        kernel
            .configure_backup_schedule(BackupCadence::Weekly, Some(&destination))
            .unwrap();
        let job = kernel.durable_job(BACKUP_JOB_ID).unwrap().unwrap();
        kernel
            .schedule_job(&JobSpec {
                id: job.id,
                kind: job.kind,
                schedule: Schedule::Weekly,
                next_due_at: base_time() - ChronoDuration::seconds(1),
                max_attempts: job.max_attempts,
                backoff: BackoffPolicy::default(),
                enabled: job.enabled,
                requires_explicit_opt_in: job.requires_explicit_opt_in,
                payload_json: job.payload_json,
            })
            .unwrap();
    }
    BackupFixture {
        dir,
        destination,
        state,
    }
}

fn run_due(
    kernel: &Kernel,
    executor: &dyn JobExecutor<Kernel>,
    window: &str,
) -> finance_kernel::JobRunReport {
    let clock = FixedClock(base_time());
    kernel
        .run_due_jobs_with_clock(
            clock.now(),
            window,
            executor,
            &CancellationToken::new(),
            &clock,
        )
        .expect("run due jobs")
}

fn destination_files(destination: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(destination)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    files.sort();
    files
}

/// The no-partial-commit invariant for the backup kind: the destination holds only whole backups,
/// each with exactly one verified receipt of the same size, and nothing else (no temp file, no
/// unverified receipt).
fn assert_whole_or_nothing(kernel: &Kernel, destination: &Path) -> usize {
    let files = destination_files(destination);
    let history = kernel.backup_history().unwrap();
    for file in &files {
        assert_eq!(
            file.extension().and_then(|ext| ext.to_str()),
            Some("pcfobk"),
            "a leftover or temp file in the destination: {file:?}"
        );
    }
    assert_eq!(
        files.len(),
        history.len(),
        "every artifact has one receipt and every receipt has its artifact"
    );
    for entry in &history {
        assert!(entry.verified, "an unverified receipt is a partial commit");
        let file = Path::new(&entry.destination);
        let on_disk = fs::metadata(file)
            .unwrap_or_else(|_| panic!("a receipt without its artifact: {file:?}"))
            .len();
        assert_eq!(
            on_disk, entry.size_bytes,
            "the artifact is not the size its receipt recorded"
        );
    }
    files.len()
}

#[test]
fn a_queued_backup_cancelled_before_it_runs_never_runs_and_writes_nothing() {
    let fixture = backup_fixture();
    let dispatcher = fixture.state.job_dispatcher();
    let guard = fixture.state.lock_controller().unwrap();
    let kernel = guard.kernel().unwrap();

    assert!(
        kernel.cancel_job(BACKUP_JOB_ID).unwrap(),
        "the request is accepted"
    );
    let row = kernel.durable_job(BACKUP_JOB_ID).unwrap().unwrap();
    assert_eq!(
        row.state,
        JobState::Cancelled,
        "a queued job is terminal at once"
    );

    let report = run_due(kernel, dispatcher.as_ref(), "after-cancel");
    assert_eq!((report.attempted, report.succeeded), (0, 0), "{report:?}");
    assert_eq!(assert_whole_or_nothing(kernel, &fixture.destination), 0);
    assert!(
        !kernel.cancel_job(BACKUP_JOB_ID).unwrap(),
        "cancelling an already cancelled job is a no-op, not an error"
    );
}

/// What one race observed.
struct RaceOutcome {
    /// How long the `cancel_job` call itself took.
    call: Duration,
    /// How long the runner took to return after the cancel was requested.
    to_terminal: Duration,
    /// Whether the scheduler claimed and started the backup.
    attempted: bool,
    /// Whether the scheduler recorded the running job as cancelled.
    recorded_cancelled_run: bool,
    /// Whole artifacts left in the destination.
    artifacts: usize,
}

/// One race: a runner thread executes the real scheduled backup while the main thread cancels it
/// `offset` after the runner started. Everything that must hold at every offset is asserted here;
/// what a caller wants beyond that comes back in the outcome.
fn race_one_backup(offset: Duration, window: &str) -> RaceOutcome {
    let fixture = backup_fixture();
    let dispatcher = fixture.state.job_dispatcher();
    let (call, to_terminal, report) = {
        let guard = fixture.state.lock_controller().unwrap();
        let kernel = guard.kernel().unwrap();
        let executor: &dyn JobExecutor<Kernel> = dispatcher.as_ref();
        thread::scope(|scope| {
            let (done_tx, done_rx) = std::sync::mpsc::channel::<Instant>();
            let runner = scope.spawn(move || {
                let report = run_due(kernel, executor, window);
                let _ = done_tx.send(Instant::now());
                report
            });
            thread::sleep(offset);
            let requested_at = Instant::now();
            kernel.cancel_job(BACKUP_JOB_ID).expect("cancel request");
            let call = requested_at.elapsed();
            let report = runner.join().expect("runner joins");
            let finished_at = done_rx.recv().expect("runner reported");
            (
                call,
                finished_at.saturating_duration_since(requested_at),
                report,
            )
        })
    };
    let attempted = report.attempted > 0;

    let guard = fixture.state.lock_controller().unwrap();
    let kernel = guard.kernel().unwrap();
    let row = kernel.durable_job(BACKUP_JOB_ID).unwrap().unwrap();
    assert_eq!(
        row.state,
        JobState::Cancelled,
        "the request wins, whenever it landed"
    );
    let artifacts = assert_whole_or_nothing(kernel, &fixture.destination);
    if !attempted {
        assert_eq!(artifacts, 0, "a job that never ran left something behind");
    }
    drop(guard);

    // Durable: close the vault, reopen it, and the job is still cancelled and not claimed again.
    let path = fixture.vault_path();
    let destination = fixture.destination.clone();
    let before = destination_files(&destination);
    drop(fixture.state);
    let reopened = AppState::new(VaultController::open(&path));
    unlock_vault_impl(&reopened, PASSWORD.to_owned()).expect("unlock reopened vault");
    let dispatcher = reopened.job_dispatcher();
    let guard = reopened.lock_controller().unwrap();
    let kernel = guard.kernel().unwrap();
    let row = kernel.durable_job(BACKUP_JOB_ID).unwrap().unwrap();
    assert_eq!(row.state, JobState::Cancelled, "the checkpoint is durable");
    assert_eq!(
        row.last_outcome,
        Some(finance_kernel::JobOutcome::Cancelled)
    );
    let again = run_due(kernel, dispatcher.as_ref(), "after-reopen");
    assert_eq!(
        again.attempted, 0,
        "a cancelled job is not claimed again: {again:?}"
    );
    assert_eq!(
        destination_files(&destination),
        before,
        "the reopen wrote nothing"
    );
    assert_whole_or_nothing(kernel, &destination);
    RaceOutcome {
        call,
        to_terminal,
        attempted,
        recorded_cancelled_run: report.cancelled > 0,
        artifacts,
    }
}

/// Offsets that straddle the real export: before it starts, while it runs, after it finished.
const RACE_OFFSETS_MS: &[u64] = &[0, 1, 2, 4, 8, 16, 32, 64];

#[test]
fn a_backup_cancelled_at_any_moment_leaves_only_whole_artifacts_and_a_durable_cancelled_job() {
    let mut ran = 0;
    for (index, offset) in RACE_OFFSETS_MS.iter().enumerate() {
        let outcome = race_one_backup(Duration::from_millis(*offset), &format!("race-{index}"));
        if outcome.attempted {
            ran += 1;
        }
    }
    // The sweep is only meaningful if it covered the start of the work.
    assert!(
        ran > 0,
        "no offset let the backup start, so the whole-artifact check never saw an artifact"
    );
}

/// Queued cancels take effect when `cancel_job` returns, and quickly: one write, measured over
/// many rows so a single slow call cannot hide behind the others. The bound is the SLA itself,
/// roughly a hundred times what a single indexed update needs.
#[test]
fn queued_cancels_are_terminal_when_the_call_returns_and_p99_is_under_the_sla() {
    let fixture = backup_fixture();
    let guard = fixture.state.lock_controller().unwrap();
    let kernel = guard.kernel().unwrap();
    let mut samples = Vec::new();
    for _ in 0..25 {
        let id = Uuid::now_v7();
        kernel
            .schedule_job(&JobSpec {
                id,
                kind: "gdsz_probe".to_owned(),
                schedule: Schedule::Once,
                next_due_at: base_time(),
                max_attempts: 1,
                backoff: BackoffPolicy::default(),
                enabled: true,
                requires_explicit_opt_in: false,
                payload_json: None,
            })
            .unwrap();
        let requested_at = Instant::now();
        assert!(kernel.cancel_job(id).unwrap());
        samples.push(requested_at.elapsed());
        assert_eq!(
            kernel.durable_job(id).unwrap().unwrap().state,
            JobState::Cancelled,
            "terminal as soon as the call returns"
        );
    }
    assert!(
        p99(&samples) < SLA,
        "the slowest of {} queued cancels took {:?}",
        samples.len(),
        p99(&samples)
    );
}

#[test]
#[ignore = "absolute latency SLA; run by hand, one at a time: cargo test --test job_cancellation -- --ignored --exact --test-threads=1 <name> (parallel runs contend for the vault and miss the bound)"]
fn backup_cancel_path_latency_is_under_the_sla_at_p99() {
    let mut calls = Vec::new();
    let mut to_terminal = Vec::new();
    for round in 0..3 {
        for (index, offset) in RACE_OFFSETS_MS.iter().enumerate() {
            let outcome = race_one_backup(
                Duration::from_millis(*offset),
                &format!("perf-{round}-{index}"),
            );
            calls.push(outcome.call);
            to_terminal.push(outcome.to_terminal);
        }
    }
    assert!(
        p99(&calls) < SLA,
        "the cancel request took {:?} at P99 over {} samples",
        p99(&calls),
        calls.len()
    );
    // On a small vault the export finishes inside the SLA, so this bounds the whole path. It does
    // not show a long export stops early: it cannot, today.
    assert!(
        p99(&to_terminal) < SLA,
        "the runner took {:?} to return after the cancel at P99 over {} samples",
        p99(&to_terminal),
        to_terminal.len()
    );
}

/// The contract for a cancel that lands while the backup export is running: the runner returns
/// within the SLA and, if it recorded the running job as cancelled, nothing was committed.
/// A handler for the real backup kind that lands a cancel request exactly after the export has
/// completed and before its result is reported, then waits until the runner's watcher has raised the
/// job's token (so the runner's cancellation override would fire for anything but a committed result).
struct CancelsAfterTheExport;

impl JobHandler<Kernel> for CancelsAfterTheExport {
    fn kind(&self) -> &'static str {
        BACKUP_JOB_KIND
    }

    fn should_run(&self, job: &JobRecord, _context: &Kernel) -> bool {
        job.id == BACKUP_JOB_ID
    }

    fn execute(
        &self,
        job: &JobRecord,
        cancellation: &CancellationToken,
        kernel: &Kernel,
    ) -> JobExecution {
        backup_job_execution_observed(job, cancellation, kernel, &|| {
            assert!(
                kernel.cancel_job(BACKUP_JOB_ID).unwrap(),
                "a running job accepts the request"
            );
            let deadline = Instant::now() + Duration::from_secs(5);
            while !cancellation.is_cancelled() {
                assert!(
                    Instant::now() < deadline,
                    "the watcher never raised the token"
                );
                thread::sleep(Duration::from_millis(2));
            }
        })
    }
}

/// A cancel that arrives after the backup completed is too late: the bundle and its receipt exist, so
/// the job is recorded as succeeded, never as cancelled (personal-cfo-g3m.29, the "committed outcome is
/// the truth" rule of the job runtime).
#[test]
fn a_backup_cancelled_after_it_completed_is_recorded_succeeded_with_its_bundle_and_receipt() {
    let fixture = backup_fixture();
    let dispatcher = JobDispatcher::<Kernel>::new();
    dispatcher
        .register(Arc::new(CancelsAfterTheExport))
        .unwrap();
    let guard = fixture.state.lock_controller().unwrap();
    let kernel = guard.kernel().unwrap();

    let report = run_due(kernel, &dispatcher, "cancel-after-export");

    assert_eq!((report.succeeded, report.cancelled), (1, 0), "{report:?}");
    let row = kernel.durable_job(BACKUP_JOB_ID).unwrap().unwrap();
    assert_eq!(row.state, JobState::Succeeded);
    assert_eq!(
        row.last_outcome,
        Some(finance_kernel::JobOutcome::Succeeded)
    );
    assert_eq!(
        assert_whole_or_nothing(kernel, &fixture.destination),
        1,
        "the completed bundle and its receipt are there"
    );
    assert!(
        !kernel.job_cancellation_requested(BACKUP_JOB_ID).unwrap(),
        "the late request is cleared, not left pending"
    );
}

#[test]
fn a_backup_cancelled_mid_export_stops_within_the_sla_and_commits_nothing() {
    for (index, offset) in RACE_OFFSETS_MS.iter().enumerate() {
        let outcome = race_one_backup(Duration::from_millis(*offset), &format!("mid-{index}"));
        if outcome.recorded_cancelled_run {
            assert_eq!(
                outcome.artifacts, 0,
                "offset {offset} ms: the run was recorded as cancelled but its backup was committed"
            );
            assert!(
                outcome.to_terminal < SLA,
                "offset {offset} ms: the cancelled run took {:?} to return",
                outcome.to_terminal
            );
        }
    }
}

// ---- connector_refresh: the lock-releasing kind ---------------------------------------------

/// A provider whose fetch blocks until the test releases it.
struct BlockingProvider {
    inner: MockConnector,
    entered: AtomicBool,
    gate: (Mutex<bool>, Condvar),
}

impl ConnectorAdapter for BlockingProvider {
    fn id(&self) -> &'static str {
        self.inner.id()
    }
    fn display_name(&self) -> &'static str {
        self.inner.display_name()
    }
    fn version(&self) -> Version {
        self.inner.version()
    }
    fn capabilities(&self) -> CapabilitySet {
        self.inner.capabilities()
    }
    fn link(&self, input: &LinkInput) -> Result<LinkSession, ConnectorError> {
        self.inner.link(input)
    }
    fn fetch_accounts(&self, conn: &Connection) -> Result<Vec<ParsedAccount>, ConnectorError> {
        self.inner.fetch_accounts(conn)
    }
    fn fetch_transactions(
        &self,
        conn: &Connection,
        account: &str,
        since: Option<chrono::NaiveDate>,
    ) -> Result<Vec<ParsedRecord>, ConnectorError> {
        self.inner.fetch_transactions(conn, account, since)
    }
    fn fetch_balances(
        &self,
        conn: &Connection,
        account: &str,
    ) -> Result<Vec<ParsedBalance>, ConnectorError> {
        self.inner.fetch_balances(conn, account)
    }
    fn health(&self, conn: &Connection) -> Result<HealthStatus, ConnectorError> {
        self.inner.health(conn)
    }
    fn sync(
        &self,
        conn: &Connection,
        since: Option<chrono::NaiveDate>,
    ) -> Result<SyncBatch, ConnectorError> {
        self.entered.store(true, Ordering::SeqCst);
        let (lock, released) = &self.gate;
        let mut open = lock.lock().unwrap();
        // A deadline, so a fix that never lets the test release the provider (a mutated fetch that is
        // never polled) fails the suite instead of hanging it.
        let deadline = Instant::now() + Duration::from_secs(10);
        while !*open {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                break;
            };
            open = released.wait_timeout(open, left).unwrap().0;
        }
        drop(open);
        self.inner.sync(conn, since)
    }
}

fn blocking() -> &'static BlockingProvider {
    static ADAPTER: OnceLock<BlockingProvider> = OnceLock::new();
    ADAPTER.get_or_init(|| BlockingProvider {
        inner: MockConnector::with_fixture().with_id("other"),
        entered: AtomicBool::new(false),
        gate: (Mutex::new(false), Condvar::new()),
    })
}

/// The blocking provider is one process-wide gate, so the cases that use it run one at a time and
/// each starts it closed.
fn serialize_blocking() -> std::sync::MutexGuard<'static, ()> {
    static SERIAL: Mutex<()> = Mutex::new(());
    let guard = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    blocking().entered.store(false, Ordering::SeqCst);
    *blocking().gate.0.lock().unwrap() = false;
    guard
}

fn resolve_blocking(_: &str) -> Option<&'static dyn ConnectorAdapter> {
    Some(blocking())
}

fn healthy() -> &'static MockConnector {
    static ADAPTER: OnceLock<MockConnector> = OnceLock::new();
    ADAPTER.get_or_init(|| MockConnector::with_fixture().with_id("other"))
}

fn resolve_healthy(_: &str) -> Option<&'static dyn ConnectorAdapter> {
    Some(healthy())
}

/// A provider that runs a hook inside its fetch, which a test uses to cancel at that instant.
type OnSync = Box<dyn Fn() + Send>;
static ON_SYNC: Mutex<Option<OnSync>> = Mutex::new(None);
/// When set, the cancelling provider answers its fetch with a provider error (after the hook).
static FAIL_FETCH: AtomicBool = AtomicBool::new(false);
static HOOK_SERIAL: Mutex<()> = Mutex::new(());

/// Holds the cancelling provider's hook and failure mode for one test and clears them after, so the
/// tests that use it run one at a time.
struct HookGuard(#[allow(dead_code)] std::sync::MutexGuard<'static, ()>);

fn hook_guard(hook: impl Fn() + Send + 'static, fail: bool) -> HookGuard {
    let guard = HOOK_SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *ON_SYNC.lock().unwrap() = Some(Box::new(hook));
    FAIL_FETCH.store(fail, Ordering::SeqCst);
    HookGuard(guard)
}

impl Drop for HookGuard {
    fn drop(&mut self) {
        *ON_SYNC.lock().unwrap() = None;
        FAIL_FETCH.store(false, Ordering::SeqCst);
    }
}

/// The hook, then the failure when asked: what the cancelling provider does at either fetch.
fn fetch_hook() -> Result<(), ConnectorError> {
    if let Some(hook) = ON_SYNC.lock().unwrap().as_ref() {
        hook();
    }
    if FAIL_FETCH.load(Ordering::SeqCst) {
        return Err(ConnectorError::Provider("test failure".to_owned()));
    }
    Ok(())
}

struct CancellingProvider(MockConnector);

impl ConnectorAdapter for CancellingProvider {
    fn id(&self) -> &'static str {
        self.0.id()
    }
    fn display_name(&self) -> &'static str {
        self.0.display_name()
    }
    fn version(&self) -> Version {
        self.0.version()
    }
    fn capabilities(&self) -> CapabilitySet {
        self.0.capabilities()
    }
    fn link(&self, input: &LinkInput) -> Result<LinkSession, ConnectorError> {
        self.0.link(input)
    }
    fn fetch_accounts(&self, conn: &Connection) -> Result<Vec<ParsedAccount>, ConnectorError> {
        fetch_hook()?;
        self.0.fetch_accounts(conn)
    }
    fn fetch_transactions(
        &self,
        conn: &Connection,
        account: &str,
        since: Option<chrono::NaiveDate>,
    ) -> Result<Vec<ParsedRecord>, ConnectorError> {
        self.0.fetch_transactions(conn, account, since)
    }
    fn fetch_balances(
        &self,
        conn: &Connection,
        account: &str,
    ) -> Result<Vec<ParsedBalance>, ConnectorError> {
        self.0.fetch_balances(conn, account)
    }
    fn health(&self, conn: &Connection) -> Result<HealthStatus, ConnectorError> {
        self.0.health(conn)
    }
    fn sync(
        &self,
        conn: &Connection,
        since: Option<chrono::NaiveDate>,
    ) -> Result<SyncBatch, ConnectorError> {
        fetch_hook()?;
        self.0.sync(conn, since)
    }
}

fn resolve_cancelling(_: &str) -> Option<&'static dyn ConnectorAdapter> {
    static ADAPTER: OnceLock<CancellingProvider> = OnceLock::new();
    Some(ADAPTER.get_or_init(|| CancellingProvider(MockConnector::with_fixture().with_id("other"))))
}

fn open_state() -> (TempDir, AppState) {
    let dir = TempDir::new().expect("temp dir");
    let mut controller = VaultController::open(dir.path().join("vault.db"));
    controller
        .create(PASSWORD.as_bytes())
        .expect("create vault");
    (dir, AppState::new(controller))
}

/// Link a connection and map its checking account, as a user would. Returns the connection id.
fn linked(state: &AppState) -> Uuid {
    let connection_id = connector_link_impl(
        state,
        healthy(),
        ConnectorLinkInput {
            adapter_id: "other".to_owned(),
            setup_token: "mock-setup-token".to_owned(),
        },
    )
    .expect("link")
    .connection_id;
    let account = create_account_impl(
        state,
        CreateAccountInput {
            name: "Checking".to_owned(),
            cashflow_role: CashflowRoleDto::LiquidCash,
            currency: "USD".to_owned(),
            flags: Some(AccountFlagsDto {
                retirement: false,
                tax_advantaged: false,
                joint: false,
                business: false,
            }),
            opening_balance: None,
            subtype: None,
            idempotency_key: String::new(),
        },
    )
    .expect("account")
    .account_id;
    connector_set_account_link_impl(
        state,
        ConnectorSetAccountLinkInput {
            connection_id: connection_id.clone(),
            external_id: "mock-acct-checking".to_owned(),
            account_id: Some(account),
            allow_shared_feed: false,
        },
    )
    .expect("map");
    Uuid::parse_str(&connection_id).unwrap()
}

fn job_state(state: &AppState, connection: Uuid) -> JobState {
    state
        .lock_controller()
        .unwrap()
        .kernel()
        .unwrap()
        .durable_job(refresh_job_id(connection))
        .unwrap()
        .expect("the connection's refresh job")
        .state
}

fn cancel(state: &AppState, connection: Uuid) -> (bool, Duration) {
    let requested_at = Instant::now();
    let accepted = state
        .lock_controller()
        .unwrap()
        .kernel()
        .unwrap()
        .cancel_job(refresh_job_id(connection))
        .unwrap();
    (accepted, requested_at.elapsed())
}

fn sweep(
    state: &AppState,
    resolve: app_lib::ipc::commands::ConnectorResolver,
    window: &str,
) -> RefreshSweepReport {
    run_due_connector_refresh(state, resolve, window, &FixedClock(Utc::now())).expect("sweep")
}

fn transaction_count(state: &AppState) -> usize {
    transaction_list_impl(state).expect("transactions").len()
}

#[test]
fn a_queued_connector_refresh_cancelled_before_the_sweep_never_fetches() {
    let (_dir, state) = open_state();
    let connection = linked(&state);

    let (accepted, _) = cancel(&state, connection);
    assert!(accepted);
    assert_eq!(job_state(&state, connection), JobState::Cancelled);

    let report = sweep(&state, resolve_healthy, "w1");
    assert_eq!(report, RefreshSweepReport::default(), "nothing was claimed");
    assert_eq!(transaction_count(&state), 0, "nothing was imported");
}

/// Everything a refresh may change on a connection besides the imported rows, so a cancelled run
/// can be shown to change none of it.
#[derive(Debug, PartialEq, Eq)]
struct ConnectionSnapshot {
    last_synced_at: Option<String>,
    last_error: Option<String>,
    attention: Option<String>,
    watermarks: Vec<Option<String>>,
}

fn snapshot(state: &AppState, connection: Uuid) -> ConnectionSnapshot {
    let controller = state.lock_controller().unwrap();
    let kernel = controller.kernel().unwrap();
    let row = kernel
        .connector_connections()
        .unwrap()
        .into_iter()
        .find(|row| row.id == connection)
        .expect("the connection");
    ConnectionSnapshot {
        last_synced_at: row.last_synced_at,
        last_error: row.last_error,
        attention: row.attention,
        watermarks: kernel
            .connector_links(connection)
            .unwrap()
            .into_iter()
            .map(|link| link.last_synced_on)
            .collect(),
    }
}

/// Give the connection a re-authentication notice, which a healthy refresh would clear.
fn set_notice(state: &AppState, connection: Uuid) {
    let notice = encode(&[AttentionEntry {
        kind: AttentionKind::ReauthInstitution,
        institution: Some("Example Bank".to_owned()),
    }]);
    state
        .lock_controller()
        .unwrap()
        .kernel()
        .unwrap()
        .set_connector_attention(connection, &notice)
        .unwrap();
}

/// What a cancel during the provider fetch left behind.
struct FetchCancel {
    /// How long the cancel request took (the vault lock is released during a fetch).
    request: Duration,
    /// How long the sweep took to return after the request, with the provider released after
    /// `release_after`.
    to_return: Duration,
    report: RefreshSweepReport,
    imported: usize,
    state: JobState,
    before: ConnectionSnapshot,
    after: ConnectionSnapshot,
    dir: TempDir,
    app: AppState,
    connection: Uuid,
}

/// A cancel during the provider fetch, with the vault lock released, as production runs it. The
/// provider is released `release_after` after the request (a test with no deadline would hang on
/// a fix that makes the fetch stop early, so the release is on a timer). The returned snapshot is
/// taken once the provider has been released, so a late result that reached the commit would show.
fn cancel_during_fetch(window: &str, release_after: Duration) -> FetchCancel {
    let _gate = serialize_blocking();
    let (dir, app) = open_state();
    let connection = linked(&app);
    set_notice(&app, connection);
    let before = snapshot(&app, connection);
    let mut request = Duration::ZERO;
    let mut to_return = Duration::ZERO;
    let mut report = RefreshSweepReport::default();
    thread::scope(|scope| {
        let state = &app;
        let refresh = scope.spawn(move || sweep(state, resolve_blocking, window));
        while !blocking().entered.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(2));
        }
        let (accepted, took) = cancel(&app, connection);
        assert!(accepted, "a running job accepts the request");
        request = took;
        let requested_at = Instant::now();
        let release = scope.spawn(move || {
            thread::sleep(release_after);
            let (lock, released) = &blocking().gate;
            *lock.lock().unwrap() = true;
            released.notify_all();
        });
        report = refresh.join().expect("sweep joins");
        to_return = requested_at.elapsed();
        release.join().expect("release joins");
        assert_eq!(report.failed, 0, "{report:?}");
    });
    // The abandoned fetch ends on its own thread; wait until it has released the sync slot.
    wait_for_sync_slot(&app, connection);
    let imported = transaction_count(&app);
    let state = job_state(&app, connection);
    let after = snapshot(&app, connection);
    FetchCancel {
        request,
        to_return,
        report,
        imported,
        state,
        before,
        after,
        dir,
        app,
        connection,
    }
}

/// Block until the connection's sync slot can be claimed (an abandoned fetch has ended).
fn wait_for_sync_slot(app: &AppState, connection: Uuid) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !app.claim_connector_sync(connection) {
        assert!(
            Instant::now() < deadline,
            "the sync slot was never released"
        );
        thread::sleep(Duration::from_millis(5));
    }
    app.release_connector_sync(connection);
}

#[test]
fn a_connector_refresh_cancelled_mid_fetch_is_accepted_all_or_nothing_and_durable() {
    // The control: the same refresh, never cancelled, imports the full fixture.
    let (_control_dir, control) = open_state();
    linked(&control);
    sweep(&control, resolve_healthy, "control");
    let full = transaction_count(&control);
    assert!(full > 0, "the fixture imports something");

    let cancelled = cancel_during_fetch("w1", Duration::from_millis(50));
    assert!(
        cancelled.imported == 0 || cancelled.imported == full,
        "a partial import: {} of {full}",
        cancelled.imported
    );

    // Durable: close and reopen, and the job row, and the data, are exactly as they were.
    let FetchCancel {
        dir,
        app,
        connection,
        state: before,
        imported,
        ..
    } = cancelled;
    let path = dir.path().join("vault.db");
    drop(app);
    let reopened = AppState::new(VaultController::open(&path));
    unlock_vault_impl(&reopened, PASSWORD.to_owned()).expect("unlock reopened vault");
    assert_eq!(
        job_state(&reopened, connection),
        before,
        "the checkpoint is durable"
    );
    assert_eq!(
        transaction_count(&reopened),
        imported,
        "the reopen changed no data"
    );
}

#[test]
#[ignore = "absolute latency SLA; run by hand, one at a time: cargo test --test job_cancellation -- --ignored --exact --test-threads=1 <name> (parallel runs contend for the vault and miss the bound)"]
fn connector_refresh_cancel_request_is_under_the_sla() {
    let cancelled = cancel_during_fetch("perf", Duration::from_millis(50));
    assert!(
        cancelled.request < SLA,
        "the cancel request during a fetch took {:?}",
        cancelled.request
    );
}

/// The contract for a cancel that lands during a slow fetch: the cancellation reaches the fetch,
/// the sweep returns within the SLA without waiting for the provider, the job is recorded as
/// cancelled and committed nothing, and the provider's late result (released 3 s after the cancel)
/// never reaches the commit.
#[test]
fn a_connector_refresh_cancelled_mid_fetch_stops_within_the_sla_and_commits_nothing() {
    let cancelled = cancel_during_fetch("slow", Duration::from_secs(3));
    assert!(
        cancelled.to_return < SLA,
        "the sweep took {:?} to return after the cancel",
        cancelled.to_return
    );
    assert_eq!(cancelled.state, JobState::Cancelled);
    assert_eq!(cancelled.report.cancelled, 1, "{:?}", cancelled.report);
    assert_eq!(cancelled.report.refreshed, 0, "{:?}", cancelled.report);
    assert_eq!(cancelled.imported, 0, "no row was imported");
    assert_eq!(
        cancelled.after, cancelled.before,
        "the last-refreshed stamp, the cursors and the re-authentication notice are unchanged"
    );
    assert!(
        cancelled.before.attention.is_some() && cancelled.before.last_synced_at.is_none(),
        "the snapshot would have shown a refresh: a healthy one clears the notice and stamps the time"
    );
}

/// The same release timing with no cancel, as the control for "a late result never commits": an
/// uncancelled refresh of the same blocking provider imports the fixture and clears the notice.
#[test]
fn the_uncancelled_control_commits_and_changes_what_the_cancelled_run_must_not() {
    let _gate = serialize_blocking();
    let (_dir, app) = open_state();
    let connection = linked(&app);
    set_notice(&app, connection);
    let before = snapshot(&app, connection);
    *blocking().gate.0.lock().unwrap() = true;
    let report = sweep(&app, resolve_blocking, "control");
    assert_eq!(report.refreshed, 1, "{report:?}");
    assert!(transaction_count(&app) > 0);
    let after = snapshot(&app, connection);
    assert_ne!(after.last_synced_at, before.last_synced_at);
    assert_ne!(after.attention, before.attention);
    assert_eq!(job_state(&app, connection), JobState::Succeeded);
}

#[test]
fn an_abandoned_fetch_keeps_the_connections_sync_slot_until_it_ends() {
    let _gate = serialize_blocking();
    let (_dir, app) = open_state();
    let connection = linked(&app);
    thread::scope(|scope| {
        let state = &app;
        let refresh = scope.spawn(move || sweep(state, resolve_blocking, "slot"));
        while !blocking().entered.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(2));
        }
        assert!(
            !app.claim_connector_sync(connection),
            "the running refresh holds the slot"
        );
        assert!(cancel(&app, connection).0);
        let report = refresh.join().expect("sweep joins");
        assert_eq!(report.cancelled, 1, "{report:?}");
        // The sweep has returned and the job is cancelled, but the provider call is still in
        // flight: one sync per connection means a second walk must not start.
        assert!(
            !app.claim_connector_sync(connection),
            "the abandoned fetch still holds the slot"
        );
        let (lock, released) = &blocking().gate;
        *lock.lock().unwrap() = true;
        released.notify_all();
    });
    wait_for_sync_slot(&app, connection);
    assert_eq!(transaction_count(&app), 0, "the late result was dropped");
}

/// A cancel that lands exactly as the provider returns: the request is decided in the section
/// that commits, so nothing is written and the job is cancelled.
#[test]
fn a_cancel_that_lands_as_the_fetch_returns_commits_nothing() {
    let (_dir, app) = open_state();
    let app: &'static AppState = Box::leak(Box::new(app));
    let connection = linked(app);
    set_notice(app, connection);
    let before = snapshot(app, connection);
    let _hook = hook_guard(move || assert!(cancel(app, connection).0), false);
    let report = sweep(app, resolve_cancelling, "as-it-returns");

    assert_eq!(report.cancelled, 1, "{report:?}");
    assert_eq!(job_state(app, connection), JobState::Cancelled);
    assert_eq!(transaction_count(app), 0);
    assert_eq!(snapshot(app, connection), before);
}

/// Link a connection and leave every account unmapped, so a refresh takes the discovery path.
fn linked_unmapped(state: &AppState) -> Uuid {
    let id = connector_link_impl(
        state,
        healthy(),
        ConnectorLinkInput {
            adapter_id: "other".to_owned(),
            setup_token: "mock-setup-token".to_owned(),
        },
    )
    .expect("link")
    .connection_id;
    Uuid::parse_str(&id).unwrap()
}

/// The discovery path (a connection with nothing mapped yet) has its own commit: it upserts the
/// provider's accounts as links and clears the connection's error and re-authentication notice. A
/// cancel that lands as the discovery fetch returns must stop it there, inside the commit section,
/// exactly as the main sync does. (Linking already discovered the accounts, so the notice, which a
/// discovery clears, is what shows a write.)
#[test]
fn a_cancel_that_lands_as_the_discovery_fetch_returns_writes_nothing() {
    let (_dir, app) = open_state();
    let app: &'static AppState = Box::leak(Box::new(app));
    let connection = linked_unmapped(app);
    set_notice(app, connection);
    let before = snapshot(app, connection);
    assert!(before.attention.is_some());
    {
        let _hook = hook_guard(move || assert!(cancel(app, connection).0), false);
        let report = sweep(app, resolve_cancelling, "discovery-cancel");
        assert_eq!(report.cancelled, 1, "{report:?}");
    }
    assert_eq!(job_state(app, connection), JobState::Cancelled);
    assert_eq!(
        snapshot(app, connection),
        before,
        "discovery wrote (cleared the notice) after the cancel"
    );

    // The control: the same discovery, never cancelled, clears the notice.
    let (_control_dir, control) = open_state();
    let control_connection = linked_unmapped(&control);
    set_notice(&control, control_connection);
    let _hook = hook_guard(|| {}, false);
    let report = sweep(&control, resolve_cancelling, "discovery-control");
    assert_eq!(report.refreshed, 1, "{report:?}");
    assert_eq!(snapshot(&control, control_connection).attention, None);
}

/// A provider that fails after the cancel: nothing may be recorded (no error on the connection, no
/// change to the notice), because the job is cancelled. Run for both fetch sites, with a control
/// that shows the same failure IS recorded when nobody cancelled.
fn a_cancel_before_a_provider_failure_is_recorded(unmapped: bool) {
    let (_dir, app) = open_state();
    let app: &'static AppState = Box::leak(Box::new(app));
    let connection = if unmapped {
        linked_unmapped(app)
    } else {
        linked(app)
    };
    set_notice(app, connection);
    let before = snapshot(app, connection);
    {
        let _hook = hook_guard(move || assert!(cancel(app, connection).0), true);
        let report = sweep(app, resolve_cancelling, "error-cancel");
        assert_eq!(report.cancelled, 1, "unmapped={unmapped}: {report:?}");
    }
    assert_eq!(job_state(app, connection), JobState::Cancelled);
    assert_eq!(
        snapshot(app, connection),
        before,
        "unmapped={unmapped}: the failure was recorded on a cancelled run"
    );
    assert_eq!(before.last_error, None);

    let (_control_dir, control) = open_state();
    let control_connection = if unmapped {
        linked_unmapped(&control)
    } else {
        linked(&control)
    };
    let _hook = hook_guard(|| {}, true);
    sweep(&control, resolve_cancelling, "error-control");
    assert!(
        snapshot(&control, control_connection).last_error.is_some(),
        "unmapped={unmapped}: the control's failure was not recorded"
    );
}

#[test]
fn a_cancel_before_a_sync_failure_is_recorded_leaves_health_and_notice_untouched() {
    a_cancel_before_a_provider_failure_is_recorded(false);
}

#[test]
fn a_cancel_before_a_discovery_failure_is_recorded_leaves_health_and_notice_untouched() {
    a_cancel_before_a_provider_failure_is_recorded(true);
}

/// A cancel that lands after the commit and before the job's outcome is recorded is too late: the
/// refresh's rows are in, so the job says it succeeded.
#[test]
fn a_cancel_after_the_commit_leaves_the_job_succeeded() {
    let (_dir, app) = open_state();
    let connection = linked(&app);
    let (_control_dir, control) = open_state();
    linked(&control);
    sweep(&control, resolve_healthy, "control");
    let full = transaction_count(&control);

    let accepted = std::sync::atomic::AtomicBool::new(false);
    let report = run_due_connector_refresh_observed(
        &app,
        resolve_healthy,
        "after-commit",
        &FixedClock(Utc::now()),
        &|_| {
            accepted.store(cancel(&app, connection).0, Ordering::SeqCst);
        },
    )
    .expect("sweep");

    assert!(accepted.load(Ordering::SeqCst), "the cancel was accepted");
    assert_eq!((report.refreshed, report.cancelled), (1, 0), "{report:?}");
    assert_eq!(job_state(&app, connection), JobState::Succeeded);
    assert_eq!(transaction_count(&app), full, "the whole batch is in");
    assert!(
        !job_cancel_pending(&app, connection),
        "the late request is cleared"
    );
}

fn job_cancel_pending(state: &AppState, connection: Uuid) -> bool {
    state
        .lock_controller()
        .unwrap()
        .kernel()
        .unwrap()
        .job_cancellation_requested(refresh_job_id(connection))
        .unwrap()
}

/// Known gap, personal-cfo-g3m.31: a crash after the commit WITH a cancel request pending. Recovery
/// terminalizes any running row that has a cancel request as `Cancelled`, although the batch is
/// imported, because nothing durable says the commit happened. The correct outcome (what this
/// asserts) is that the job is not recorded as cancelled once its rows are in; g3m.31's durable
/// commit marker delivers it and un-ignores this case.
#[test]
#[ignore = "personal-cfo-g3m.31: needs the durable commit marker; today recovery records this job as cancelled with the batch imported"]
fn a_crash_after_the_commit_with_a_cancel_pending_is_not_recorded_as_cancelled() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("vault.db");
    let mut controller = VaultController::open(&path);
    controller
        .create(PASSWORD.as_bytes())
        .expect("create vault");
    let app = AppState::new(controller);
    let connection = linked(&app);

    // The refresh commits, a cancel request arrives, and the process dies before the outcome.
    let crashed = catch_unwind(AssertUnwindSafe(|| {
        run_due_connector_refresh_observed(
            &app,
            resolve_healthy,
            "crashes-with-cancel",
            &FixedClock(Utc::now()),
            &|_| {
                assert!(cancel(&app, connection).0);
                panic!("the process died here");
            },
        )
    }));
    assert!(crashed.is_err());
    let imported = transaction_count(&app);
    assert!(imported > 0, "the refresh committed before the crash");
    drop(app);

    let reopened = AppState::new(VaultController::open(&path));
    unlock_vault_impl(&reopened, PASSWORD.to_owned()).expect("unlock reopened vault");
    {
        let guard = reopened.lock_controller().unwrap();
        run_due(
            guard.kernel().unwrap(),
            reopened.job_dispatcher().as_ref(),
            "recover-with-cancel",
        );
    }
    assert_eq!(
        transaction_count(&reopened),
        imported,
        "the rows are still there"
    );
    assert_ne!(
        job_state(&reopened, connection),
        JobState::Cancelled,
        "recovery recorded a cancelled job whose batch is imported"
    );
}

/// A crash between the commit and the terminal write leaves the job `running`. Recovery requeues
/// it; the re-run is skipped while the committed refresh is recent, and when it does run again
/// (past the cadence) the dedupe imports nothing twice. Never cancelled.
#[test]
fn a_crash_between_the_commit_and_the_outcome_recovers_without_a_duplicate_import() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("vault.db");
    let mut controller = VaultController::open(&path);
    controller
        .create(PASSWORD.as_bytes())
        .expect("create vault");
    let app = AppState::new(controller);
    let connection = linked(&app);

    // The process dies after the refresh committed and before the job's outcome was written.
    let crashed = catch_unwind(AssertUnwindSafe(|| {
        run_due_connector_refresh_observed(
            &app,
            resolve_healthy,
            "crashes",
            &FixedClock(Utc::now()),
            &|_| panic!("the process died here"),
        )
    }));
    assert!(crashed.is_err());
    let imported = transaction_count(&app);
    assert!(imported > 0, "the refresh committed before the crash");
    assert_eq!(job_state(&app, connection), JobState::Running);
    drop(app);

    // Reopen. The main sweep's recovery requeues the interrupted job.
    let reopened = AppState::new(VaultController::open(&path));
    unlock_vault_impl(&reopened, PASSWORD.to_owned()).expect("unlock reopened vault");
    {
        let guard = reopened.lock_controller().unwrap();
        run_due(
            guard.kernel().unwrap(),
            reopened.job_dispatcher().as_ref(),
            "recover",
        );
    }
    assert_eq!(job_state(&reopened, connection), JobState::Queued);

    // The next sweep, a day later: the dedupe makes the re-run import nothing a second time.
    let report = run_due_connector_refresh(
        &reopened,
        resolve_healthy,
        "after-recovery",
        &FixedClock(Utc::now() + ChronoDuration::hours(25)),
    )
    .expect("sweep");
    assert_eq!(report.refreshed, 1, "{report:?}");
    assert_eq!(report.cancelled, 0);
    assert_eq!(
        transaction_count(&reopened),
        imported,
        "nothing imported twice"
    );
    assert_eq!(job_state(&reopened, connection), JobState::Succeeded);
}

#[test]
fn the_p99_helper_is_the_slowest_of_fewer_than_a_hundred_samples() {
    let samples: Vec<Duration> = (1..=24).map(Duration::from_millis).collect();
    assert_eq!(p99(&samples), Duration::from_millis(24));
    assert_eq!(p99(&[Duration::from_millis(7)]), Duration::from_millis(7));
    let hundred: Vec<Duration> = (1..=100).map(Duration::from_millis).collect();
    assert_eq!(p99(&hundred), Duration::from_millis(99));
}
