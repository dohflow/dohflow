//! Connector refresh on the durable job runtime (personal-cfo-lqk; ADR 0060
//! addendum 2026-10-04).
//!
//! Every connection has exactly one `connector_refresh` job: created when the
//! connection is linked (or at the next unlock for one linked before this
//! existed), removed when it is forgotten. Its payload is the connection id
//! only. Its cadence is the user's choice per connection, defaulting to the
//! provider's registry suggestion.
//!
//! The runtime's post-unlock sweep holds the vault lock while handlers run, and
//! a provider fetch can take seconds, so these jobs run in their own sweep in
//! three steps: claim under the lock, refresh with it released (the refresh
//! takes the lock only for its own short phases), record the outcome under the
//! lock. The sweep runs after the main one, whose crash recovery requeues a
//! refresh an exit interrupted.
//!
//! Provider outcomes (expired link, rate limit, provider or network error) are
//! recorded by the refresh itself, on the connection's health and as a
//! `connector_error` Money Inbox item, so for the job they are a completed run,
//! never a second `job_failure` item. Only a failure inside DohFlow is a job
//! failure.

use chrono::{DateTime, Utc};
use connector_core::RefreshCadence;
use finance_kernel::{
    BackoffPolicy, ClaimedJob, Clock, JobExecution, JobFailure, JobSpec, Kernel, KernelError,
    Schedule, CONNECTOR_REFRESH_JOB_KIND,
};
use uuid::Uuid;

use crate::ipc::commands::{connector_sync_impl, with_kernel, ConnectorResolver};
use crate::ipc::dto::ConnectorSyncInput;
use crate::ipc::IpcError;
use crate::AppState;

/// Namespace for refresh job ids: one stable id per connection.
const REFRESH_JOB_NAMESPACE: Uuid = Uuid::from_u128(0x6c71_6b5f_7265_6672_6573_685f_6a6f_6273);

/// A refresh that hit a DohFlow-internal problem is retried a couple of times.
const REFRESH_MAX_ATTEMPTS: u32 = 3;

/// The connection's refresh job id.
#[must_use]
pub fn refresh_job_id(connection_id: Uuid) -> Uuid {
    Uuid::new_v5(&REFRESH_JOB_NAMESPACE, connection_id.as_bytes())
}

/// The registry's suggested cadence for a provider; every-open when unknown.
#[must_use]
pub fn suggested_cadence(adapter_id: &str) -> RefreshCadence {
    connector_core::registration_by_id(adapter_id)
        .map_or(RefreshCadence::EveryOpen, |registration| {
            registration.metadata.suggested_refresh
        })
}

fn spec(connection_id: Uuid, cadence: RefreshCadence, next_due_at: DateTime<Utc>) -> JobSpec {
    // Manual keeps an interval so the row stays valid; it is simply disabled.
    let seconds = cadence
        .min_interval_seconds()
        .or_else(|| RefreshCadence::EveryOpen.min_interval_seconds())
        .unwrap_or(6 * 60 * 60);
    JobSpec {
        id: refresh_job_id(connection_id),
        kind: CONNECTOR_REFRESH_JOB_KIND.to_owned(),
        schedule: Schedule::Interval { seconds },
        next_due_at,
        max_attempts: REFRESH_MAX_ATTEMPTS,
        backoff: BackoffPolicy::default(),
        enabled: cadence != RefreshCadence::Manual,
        requires_explicit_opt_in: true,
        payload_json: Some(format!("{{\"connection_id\":\"{connection_id}\"}}")),
    }
}

/// The connection's current cadence, read back from its job.
///
/// # Errors
/// [`KernelError`] on a read failure.
pub fn cadence_of(
    kernel: &Kernel,
    connection_id: Uuid,
) -> Result<Option<RefreshCadence>, KernelError> {
    let Some(job) = kernel.durable_job(refresh_job_id(connection_id))? else {
        return Ok(None);
    };
    if !job.enabled {
        return Ok(Some(RefreshCadence::Manual));
    }
    let Schedule::Interval { seconds } = job.schedule else {
        return Ok(Some(RefreshCadence::EveryOpen));
    };
    Ok(Some(
        RefreshCadence::ALL
            .into_iter()
            .find(|cadence| cadence.min_interval_seconds() == Some(seconds))
            .unwrap_or(RefreshCadence::EveryOpen),
    ))
}

/// Give a connection its refresh job if it has none, with the provider's
/// suggested cadence, due now (the refresh's own recency check decides).
///
/// # Errors
/// [`KernelError`] on a read or write failure.
pub fn ensure_refresh_job(
    kernel: &Kernel,
    connection_id: Uuid,
    adapter_id: &str,
    now: DateTime<Utc>,
) -> Result<(), KernelError> {
    if kernel.durable_job(refresh_job_id(connection_id))?.is_some() {
        return Ok(());
    }
    kernel.schedule_job(&spec(connection_id, suggested_cadence(adapter_id), now))
}

/// Change a connection's cadence. The job becomes due now; the recency check
/// then waits for the new interval since the last refresh.
///
/// # Errors
/// [`KernelError`] on a write failure, or while the job is running.
pub fn set_refresh_cadence(
    kernel: &Kernel,
    connection_id: Uuid,
    cadence: RefreshCadence,
    now: DateTime<Utc>,
) -> Result<(), KernelError> {
    kernel.reconfigure_job(&spec(connection_id, cadence, now))
}

/// Remove a forgotten connection's refresh job.
///
/// # Errors
/// [`KernelError`] on a write failure.
pub fn remove_refresh_job(kernel: &Kernel, connection_id: Uuid) -> Result<(), KernelError> {
    kernel.delete_job(refresh_job_id(connection_id))?;
    Ok(())
}

/// What one refresh sweep did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RefreshSweepReport {
    /// Connections refreshed (whatever the provider answered).
    pub refreshed: u32,
    /// Due jobs not run: refreshed too recently, or the connection or its
    /// provider is gone.
    pub skipped: u32,
    /// Refreshes that failed inside DohFlow.
    pub failed: u32,
}

/// Run the due `connector_refresh` jobs after unlock, never holding the vault
/// lock across a provider fetch. Call it after the main durable-job sweep.
///
/// # Errors
/// [`IpcError`] if the vault cannot be read to claim or record jobs.
pub fn run_due_connector_refresh(
    state: &AppState,
    resolve: ConnectorResolver,
    unlock_window: &str,
    clock: &dyn Clock,
) -> Result<RefreshSweepReport, IpcError> {
    let now = clock.now();
    // Step 1, under the lock: backfill missing jobs, then claim the due ones.
    let (claimed, connections) = with_kernel(state, |kernel| {
        let connections = kernel.connector_connections()?;
        for connection in &connections {
            ensure_refresh_job(kernel, connection.id, &connection.adapter_id, now)?;
        }
        let (claimed, _) =
            kernel.claim_due_jobs_of_kind(CONNECTOR_REFRESH_JOB_KIND, now, unlock_window, clock)?;
        Ok((claimed, connections))
    })?;

    let mut report = RefreshSweepReport::default();
    for job in &claimed {
        // Step 2, lock released: decide, then refresh.
        let execution = refresh_one(state, resolve, job, &connections, now);
        match &execution {
            JobExecution::Succeeded => report.refreshed += 1,
            JobExecution::Failed(_) => report.failed += 1,
            JobExecution::Skipped | JobExecution::Cancelled => report.skipped += 1,
        }
        // Step 3, under the lock: record the outcome.
        with_kernel(state, |kernel| {
            kernel.finish_claimed_job(job, execution, clock)?;
            Ok(())
        })?;
    }
    if report != RefreshSweepReport::default() {
        tracing::info!(
            refreshed = report.refreshed,
            skipped = report.skipped,
            failed = report.failed,
            "connector refresh sweep"
        );
    }
    Ok(report)
}

fn refresh_one(
    state: &AppState,
    resolve: ConnectorResolver,
    claimed: &ClaimedJob,
    connections: &[finance_kernel::ConnectorConnectionRow],
    now: DateTime<Utc>,
) -> JobExecution {
    let Some(connection) = connection_of(claimed)
        .and_then(|id| connections.iter().find(|connection| connection.id == id))
    else {
        return JobExecution::Skipped;
    };
    // Measured from the last refresh of any kind, so a manual refresh counts.
    if let (Some(last), Schedule::Interval { seconds }) =
        (connection.last_synced_at.as_deref(), &claimed.job.schedule)
    {
        let recent = DateTime::parse_from_rfc3339(last).is_ok_and(|last| {
            now.signed_duration_since(last.with_timezone(&Utc))
                < chrono::Duration::seconds(*seconds)
        });
        if recent {
            return JobExecution::Skipped;
        }
    }
    let Some(adapter) = resolve(&connection.adapter_id) else {
        tracing::warn!(adapter = %connection.adapter_id, "no adapter for a scheduled refresh");
        return JobExecution::Skipped;
    };
    let input = ConnectorSyncInput {
        connection_id: connection.id.to_string(),
        idempotency_key: format!("connector-refresh-{}", Uuid::now_v7()),
    };
    match connector_sync_impl(state, adapter, input) {
        // Every provider outcome is recorded by the refresh itself.
        Ok(_) => JobExecution::Succeeded,
        Err(error) => execution_for_internal_error(&error),
    }
}

/// A refresh that failed inside DohFlow: retried, then a `job_failure`. The
/// reason is fixed text; the error's own message never reaches the job row.
fn execution_for_internal_error(error: &IpcError) -> JobExecution {
    let reason = match error {
        IpcError::VaultLocked => "the vault was locked during the refresh",
        _ => "the refresh could not be saved",
    };
    JobExecution::Failed(JobFailure::retryable(reason))
}

fn connection_of(claimed: &ClaimedJob) -> Option<Uuid> {
    let payload: serde_json::Value =
        serde_json::from_str(claimed.job.payload_json.as_deref()?).ok()?;
    Uuid::parse_str(payload.get("connection_id")?.as_str()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_ids_are_stable_per_connection() {
        let a = Uuid::now_v7();
        assert_eq!(refresh_job_id(a), refresh_job_id(a));
        assert_ne!(refresh_job_id(a), refresh_job_id(Uuid::now_v7()));
    }

    #[test]
    fn a_spec_carries_only_the_connection_id_and_disables_manual() {
        let connection = Uuid::now_v7();
        let now = Utc::now();
        let daily = spec(connection, RefreshCadence::Daily, now);
        assert_eq!(daily.schedule, Schedule::Interval { seconds: 86_400 });
        assert!(daily.enabled && daily.requires_explicit_opt_in);
        assert_eq!(
            daily.payload_json.as_deref(),
            Some(format!("{{\"connection_id\":\"{connection}\"}}").as_str())
        );
        assert!(!spec(connection, RefreshCadence::Manual, now).enabled);
    }

    #[test]
    fn internal_errors_retry_with_fixed_text() {
        let JobExecution::Failed(failure) =
            execution_for_internal_error(&IpcError::Validation("/Users/jane/secret".to_owned()))
        else {
            panic!("a failure");
        };
        assert!(failure.retryable);
        assert!(!failure.reason.contains("jane"));
    }
}
