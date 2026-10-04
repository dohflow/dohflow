//! Connector refresh on the durable job runtime (personal-cfo-lqk; ADR 0060
//! addendum 2026-10-04): one job per connection, a per-connection cadence, a
//! sweep that never holds the vault across a provider fetch, and provider
//! failures that surface where they already did. Real vaults, the in-memory
//! mock connector, an injected clock; no network.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::Duration;

use app_lib::connector_refresh::{refresh_job_id, run_due_connector_refresh, RefreshSweepReport};
use app_lib::ipc::commands::{
    account_list_impl, connector_connections_impl, connector_forget_impl, connector_link_impl,
    connector_set_account_link_impl, connector_set_refresh_cadence_impl, connector_sync_impl,
    create_account_impl, money_inbox_list_impl, run_due_jobs_on_unlock_impl,
};
use app_lib::ipc::dto::{
    AccountFlagsDto, CashflowRoleDto, ConnectorForgetInput, ConnectorLinkInput,
    ConnectorSetAccountLinkInput, ConnectorSetRefreshCadenceInput, ConnectorSyncInput,
    CreateAccountInput,
};
use app_lib::ipc::IpcError;
use app_lib::AppState;
use chrono::{DateTime, Utc};
use connector_core::mock::{FailureMode, MockConnector};
use connector_core::{
    CapabilitySet, Connection, ConnectorAdapter, ConnectorError, HealthStatus, LinkInput,
    LinkSession, SyncBatch,
};
use finance_kernel::{
    Clock, JobOutcome, JobState, ParsedAccount, ParsedBalance, ParsedRecord, VaultController,
};
use semver::Version;
use tempfile::TempDir;
use uuid::Uuid;

struct At(DateTime<Utc>);
impl Clock for At {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

fn hours(n: i64) -> chrono::Duration {
    chrono::Duration::hours(n)
}

fn healthy() -> &'static MockConnector {
    static ADAPTER: OnceLock<MockConnector> = OnceLock::new();
    ADAPTER.get_or_init(|| MockConnector::with_fixture().with_id("other"))
}

fn resolve_healthy(_: &str) -> Option<&'static dyn ConnectorAdapter> {
    Some(healthy())
}

fn resolve_expired(_: &str) -> Option<&'static dyn ConnectorAdapter> {
    static ADAPTER: OnceLock<MockConnector> = OnceLock::new();
    Some(ADAPTER.get_or_init(|| MockConnector::failing_with(FailureMode::Expired).with_id("other")))
}

fn open_state() -> (TempDir, AppState) {
    let dir = TempDir::new().expect("temp dir");
    let mut controller = VaultController::open(dir.path().join("vault.db"));
    controller.create(b"test-key").expect("create vault");
    (dir, AppState::new(controller))
}

/// Link a connection and map its checking account, as a user would.
fn linked(state: &AppState) -> String {
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
    connection_id
}

fn cadence(state: &AppState, connection_id: &str) -> String {
    connector_connections_impl(state)
        .unwrap()
        .into_iter()
        .find(|c| c.id == connection_id)
        .expect("connection")
        .refresh_cadence
}

fn set_cadence(state: &AppState, connection_id: &str, cadence: &str) -> Result<(), IpcError> {
    connector_set_refresh_cadence_impl(
        state,
        ConnectorSetRefreshCadenceInput {
            connection_id: connection_id.to_owned(),
            cadence: cadence.to_owned(),
        },
    )
}

fn last_refreshed(state: &AppState, connection_id: &str) -> Option<String> {
    connector_connections_impl(state)
        .unwrap()
        .into_iter()
        .find(|c| c.id == connection_id)
        .and_then(|c| c.last_synced_at)
}

fn job(state: &AppState, connection_id: &str) -> Option<finance_kernel::JobRecord> {
    let id = refresh_job_id(Uuid::parse_str(connection_id).unwrap());
    state
        .lock_controller()
        .unwrap()
        .kernel()
        .unwrap()
        .durable_job(id)
        .unwrap()
}

fn sweep(
    state: &AppState,
    resolve: app_lib::ipc::commands::ConnectorResolver,
    window: &str,
    now: DateTime<Utc>,
) -> RefreshSweepReport {
    run_due_connector_refresh(state, resolve, window, &At(now)).expect("sweep")
}

#[test]
fn linking_creates_one_refresh_job_at_the_providers_suggested_cadence() {
    let (_dir, state) = open_state();
    let connection = linked(&state);
    assert_eq!(cadence(&state, &connection), "every_open");
    let job = job(&state, &connection).expect("a refresh job");
    assert_eq!(job.kind, "connector_refresh");
    assert!(job.enabled && job.requires_explicit_opt_in);
    assert_eq!(
        job.payload_json.as_deref(),
        Some(format!("{{\"connection_id\":\"{connection}\"}}").as_str()),
        "the payload is the connection id only"
    );

    // The main post-unlock sweep leaves it to its own sweep.
    let report =
        run_due_jobs_on_unlock_impl(&state, state.job_dispatcher().as_ref(), "unlock-main")
            .unwrap();
    assert_eq!((report.attempted, report.skipped), (0, 0));
    let untouched = crate::job(&state, &connection).unwrap();
    assert_eq!(untouched.state, JobState::Queued);
    assert_eq!(
        untouched.last_outcome, None,
        "not even skipped by the main sweep"
    );
    assert_eq!(untouched.last_unlock_window, None);
}

fn job_state(state: &AppState, connection: &str) -> JobState {
    job(state, connection).unwrap().state
}

#[test]
fn a_due_connection_refreshes_once_per_interval_and_a_manual_refresh_counts() {
    let (_dir, state) = open_state();
    let connection = linked(&state);
    let now = Utc::now();

    let first = sweep(&state, resolve_healthy, "w1", now);
    assert_eq!(
        first,
        RefreshSweepReport {
            refreshed: 1,
            skipped: 0,
            failed: 0
        }
    );
    assert!(last_refreshed(&state, &connection).is_some());
    let row = job(&state, &connection).unwrap();
    assert_eq!(row.last_outcome, Some(JobOutcome::Succeeded));

    // Same unlock window: never twice. One hour later: not due yet.
    assert_eq!(
        sweep(&state, resolve_healthy, "w1", now),
        RefreshSweepReport::default()
    );
    assert_eq!(
        sweep(&state, resolve_healthy, "w2", now + hours(1)),
        RefreshSweepReport::default()
    );
    // Seven hours later: due, and refreshed.
    assert_eq!(
        sweep(&state, resolve_healthy, "w3", now + hours(7)).refreshed,
        1
    );

    // Daily: a manual refresh resets the clock, so the due run is skipped.
    set_cadence(&state, &connection, "daily").unwrap();
    connector_sync_impl(
        &state,
        healthy(),
        ConnectorSyncInput {
            connection_id: connection.clone(),
            idempotency_key: Uuid::now_v7().to_string(),
        },
    )
    .unwrap();
    let after_manual = sweep(&state, resolve_healthy, "w4", Utc::now() + hours(2));
    assert_eq!(
        after_manual,
        RefreshSweepReport {
            refreshed: 0,
            skipped: 1,
            failed: 0
        }
    );
    assert_eq!(
        sweep(&state, resolve_healthy, "w5", Utc::now() + hours(25)).refreshed,
        1,
        "a day after the last refresh"
    );
}

#[test]
fn manual_only_never_refreshes_on_its_own() {
    let (_dir, state) = open_state();
    let connection = linked(&state);
    set_cadence(&state, &connection, "manual").unwrap();
    assert_eq!(cadence(&state, &connection), "manual");
    let later = Utc::now() + chrono::Duration::days(30);
    assert_eq!(
        sweep(&state, resolve_healthy, "w1", later),
        RefreshSweepReport::default()
    );
    assert!(last_refreshed(&state, &connection).is_none());
}

#[test]
fn the_cadence_round_trips_and_rejects_unknown_values() {
    let (_dir, state) = open_state();
    let connection = linked(&state);
    for token in ["daily", "weekly", "manual", "every_open"] {
        set_cadence(&state, &connection, token).unwrap();
        assert_eq!(cadence(&state, &connection), token);
    }
    assert!(matches!(
        set_cadence(&state, &connection, "hourly"),
        Err(IpcError::Validation(_))
    ));
    assert!(matches!(
        set_cadence(&state, &Uuid::now_v7().to_string(), "daily"),
        Err(IpcError::Validation(_))
    ));
}

#[test]
fn a_provider_failure_is_a_completed_run_not_a_job_failure() {
    let (_dir, state) = open_state();
    let connection = linked(&state);
    let report = sweep(&state, resolve_expired, "w1", Utc::now());
    assert_eq!(
        report.refreshed, 1,
        "the refresh ran and recorded the outcome"
    );
    let row = job(&state, &connection).unwrap();
    assert_eq!(row.last_outcome, Some(JobOutcome::Succeeded));

    // The outcome surfaced on the connection and in the Money Inbox, once.
    let health = connector_connections_impl(&state).unwrap().remove(0);
    assert!(health.last_error.is_some());
    let kinds: Vec<String> = money_inbox_list_impl(&state)
        .unwrap()
        .into_iter()
        .map(|item| item.item_kind)
        .collect();
    assert!(kinds.contains(&"connector_error".to_owned()), "{kinds:?}");
    assert!(!kinds.contains(&"job_failure".to_owned()), "{kinds:?}");
}

#[test]
fn forgetting_removes_the_job_and_a_missing_job_is_backfilled() {
    let (_dir, state) = open_state();
    let connection = linked(&state);

    // A connection linked before refresh jobs existed has none: backfilled.
    let id = refresh_job_id(Uuid::parse_str(&connection).unwrap());
    state
        .lock_controller()
        .unwrap()
        .kernel()
        .unwrap()
        .delete_job(id)
        .unwrap();
    assert!(job(&state, &connection).is_none());
    sweep(&state, resolve_healthy, "w1", Utc::now());
    assert!(
        job(&state, &connection).is_some(),
        "backfilled at the sweep"
    );

    connector_forget_impl(
        &state,
        ConnectorForgetInput {
            connection_id: connection.clone(),
        },
    )
    .unwrap();
    assert!(job(&state, &connection).is_none());
}

#[test]
fn a_refresh_interrupted_by_an_exit_runs_after_the_next_unlock() {
    let (_dir, state) = open_state();
    let connection = linked(&state);
    let now = Utc::now();
    // Claimed in one unlock, never finished: the app exited mid-refresh.
    {
        let guard = state.lock_controller().unwrap();
        let (claimed, _) = guard
            .kernel()
            .unwrap()
            .claim_due_jobs_of_kind("connector_refresh", now, "before-exit", &At(now))
            .unwrap();
        assert_eq!(claimed.len(), 1);
    }
    assert_eq!(job_state(&state, &connection), JobState::Running);

    // Next unlock: the main sweep's recovery requeues it, then it refreshes.
    run_due_jobs_on_unlock_impl(&state, state.job_dispatcher().as_ref(), "after-exit").unwrap();
    assert_eq!(job_state(&state, &connection), JobState::Queued);
    assert_eq!(
        sweep(&state, resolve_healthy, "after-exit", now).refreshed,
        1
    );
}

// ---- review I2 on PR 66: the sweep never blocks the vault ---------------------

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

fn resolve_blocking(_: &str) -> Option<&'static dyn ConnectorAdapter> {
    Some(blocking())
}

#[test]
fn the_vault_stays_usable_while_a_provider_fetch_is_in_flight() {
    let (_dir, state) = open_state();
    linked(&state);
    std::thread::scope(|scope| {
        let refresh = scope.spawn(|| sweep(&state, resolve_blocking, "w1", Utc::now()));
        // Wait until the provider fetch is under way…
        while !blocking().entered.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(5));
        }
        // …then read the vault from another thread. With the lock held across
        // the fetch this would wait forever.
        let (sent, received) = std::sync::mpsc::channel();
        let vault = &state;
        scope.spawn(move || {
            let _ = sent.send(account_list_impl(vault).map(|accounts| accounts.len()));
        });
        let read = received
            .recv_timeout(Duration::from_secs(10))
            .expect("the vault answered while the fetch was in flight");
        assert_eq!(read.expect("account list"), 1);

        // Release the provider; the refresh completes.
        let (lock, released) = &blocking().gate;
        *lock.lock().unwrap() = true;
        released.notify_all();
        assert_eq!(refresh.join().unwrap().refreshed, 1);
    });
}
