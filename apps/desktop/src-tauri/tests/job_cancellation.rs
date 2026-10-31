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
//! * A cancel during a provider fetch is recorded as terminal, and the refresh's own commit is all
//!   or nothing against an uncancelled control.
//! * The terminal state is durable: after the vault is closed and reopened the job is still
//!   cancelled and is not claimed again.
//! * The latency of the cancel path stays well under the 500 ms SLA (the wall-clock assertion
//!   lives in the `#[ignore]`d perf cases, like the db-worker's, because absolute timings fail
//!   under load on shared runners).
//!
//! It does NOT prove that a running export or fetch stops early, and it does not assert the
//! "cancelled means nothing was committed" rule, because neither holds today: the backup handler
//! checks its token once, before the export starts, and the connector refresh is handed no token
//! at all, so work in flight runs to completion and the job is then recorded as cancelled. Those
//! two gaps are the `#[ignore]`d cases below, each naming the bead that fixes it
//! (personal-cfo-g3m.29 for the backup, personal-cfo-pxi.25 for the connector refresh); each
//! bead un-ignores its case.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use app_lib::connector_refresh::{refresh_job_id, run_due_connector_refresh, RefreshSweepReport};
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
    BackoffPolicy, BackupCadence, CancellationToken, Clock, JobExecutor, JobSpec, JobState, Kernel,
    ParsedAccount, ParsedBalance, ParsedRecord, Schedule, VaultController, BACKUP_JOB_ID,
    BACKUP_JOB_KIND, CONNECTOR_REFRESH_JOB_KIND, LOCK_RELEASING_JOB_KINDS,
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
#[ignore = "absolute latency SLA; run by the non-required perf job (cargo test ... -- --ignored)"]
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
#[test]
#[ignore = "personal-cfo-g3m.29: the backup export has no cancellation checkpoints, so a cancel that lands mid-export lets it finish and commit, then records the job as cancelled"]
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
        while !*open {
            open = released.wait(open).unwrap();
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

/// What a cancel during the provider fetch left behind.
struct FetchCancel {
    /// How long the cancel request took (the vault lock is released during a fetch).
    request: Duration,
    /// How long the sweep took to return after the request, with the provider released after
    /// `release_after`.
    to_return: Duration,
    imported: usize,
    state: JobState,
    dir: TempDir,
    app: AppState,
    connection: Uuid,
}

/// A cancel during the provider fetch, with the vault lock released, as production runs it. The
/// provider is released `release_after` after the request (a test with no deadline would hang on
/// a fix that makes the fetch stop early, so the release is on a timer).
fn cancel_during_fetch(window: &str, release_after: Duration) -> FetchCancel {
    let _gate = serialize_blocking();
    let (dir, app) = open_state();
    let connection = linked(&app);
    let mut request = Duration::ZERO;
    let mut to_return = Duration::ZERO;
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
        let report = refresh.join().expect("sweep joins");
        to_return = requested_at.elapsed();
        release.join().expect("release joins");
        assert_eq!(report.failed, 0, "{report:?}");
    });
    let imported = transaction_count(&app);
    let state = job_state(&app, connection);
    FetchCancel {
        request,
        to_return,
        imported,
        state,
        dir,
        app,
        connection,
    }
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
#[ignore = "absolute latency SLA; run by the non-required perf job (cargo test ... -- --ignored)"]
fn connector_refresh_cancel_request_is_under_the_sla() {
    let cancelled = cancel_during_fetch("perf", Duration::from_millis(50));
    assert!(
        cancelled.request < SLA,
        "the cancel request during a fetch took {:?}",
        cancelled.request
    );
}

/// The contract for a cancel that lands during a slow fetch: the cancellation reaches the fetch,
/// the sweep returns within the SLA without waiting for the provider, and a job recorded as
/// cancelled committed nothing.
#[test]
#[ignore = "personal-cfo-pxi.25: connector_refresh is handed no cancellation token, so a cancel during a fetch waits for the fetch, commits its whole batch, then records the job as cancelled"]
fn a_connector_refresh_cancelled_mid_fetch_stops_within_the_sla_and_commits_nothing() {
    let cancelled = cancel_during_fetch("slow", Duration::from_secs(3));
    assert!(
        cancelled.to_return < SLA,
        "the sweep took {:?} to return after the cancel",
        cancelled.to_return
    );
    if cancelled.state == JobState::Cancelled {
        assert_eq!(
            cancelled.imported, 0,
            "the job is recorded as cancelled but its batch was committed"
        );
    }
}

#[test]
fn the_p99_helper_is_the_slowest_of_fewer_than_a_hundred_samples() {
    let samples: Vec<Duration> = (1..=24).map(Duration::from_millis).collect();
    assert_eq!(p99(&samples), Duration::from_millis(24));
    assert_eq!(p99(&[Duration::from_millis(7)]), Duration::from_millis(7));
    let hundred: Vec<Duration> = (1..=100).map(Duration::from_millis).collect();
    assert_eq!(p99(&hundred), Duration::from_millis(99));
}
