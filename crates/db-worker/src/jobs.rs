//! SQLCipher persistence for the local durable job runtime (personal-cfo-ati).
//!
//! This module is deliberately the only place that translates durable job
//! records to SQL.  The scheduler and handler contract live in the pure
//! `job-runtime` crate; callers never receive a `rusqlite::Connection`.

use chrono::{DateTime, Utc};
use job_runtime::{
    BackoffPolicy, CancellationToken, Clock, JobExecutor, JobFailure, JobFinishResult, JobOutcome,
    JobRecord, JobRunReport, JobRunner, JobRunnerError, JobSpec, JobState, JobStore,
};
use rusqlite::{params, OptionalExtension};
use thiserror::Error;
use uuid::Uuid;

use crate::{DbError, DbWorker};

/// A read-only durable-job view for callers that do not need runtime internals.
pub type DurableJobView = JobRecord;

/// Errors returned while decoding durable job rows.
#[derive(Debug, Error)]
pub enum JobStoreError {
    /// The row contains an invalid timestamp, schedule, state, or outcome.
    #[error("invalid durable job row: {0}")]
    InvalidRow(String),
    /// The underlying encrypted database failed.
    #[error(transparent)]
    Db(#[from] DbError),
    /// A direct SQL operation failed while claiming or finalizing a row.
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}

impl From<JobStoreError> for DbError {
    fn from(error: JobStoreError) -> Self {
        match error {
            JobStoreError::InvalidRow(message) => DbError::SelfTestFailed(message),
            JobStoreError::Db(error) => error,
            JobStoreError::Sqlite(error) => DbError::Sqlite(error),
        }
    }
}

fn parse_timestamp(
    value: Option<String>,
    field: &str,
) -> Result<Option<DateTime<Utc>>, JobStoreError> {
    value
        .map(|raw| {
            DateTime::parse_from_rfc3339(&raw)
                .map(|value| value.with_timezone(&Utc))
                .map_err(|error| JobStoreError::InvalidRow(format!("{field}: {error}")))
        })
        .transpose()
}

fn sanitize_error(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|character| {
            !character.is_control()
                && !matches!(
                    character,
                    '\u{200B}'..='\u{200F}'
                        | '\u{202A}'..='\u{202E}'
                        | '\u{2066}'..='\u{2069}'
                        | '\u{FEFF}'
                )
        })
        .take(300)
        .collect();
    if raw.chars().count() > 300 {
        format!("{cleaned}…")
    } else {
        cleaned
    }
}

#[allow(clippy::too_many_arguments)]
fn row_from_parts(
    id: Uuid,
    kind: String,
    cadence: String,
    next_due_at: Option<String>,
    state: String,
    attempt_count: i64,
    max_attempts: i64,
    backoff_initial_seconds: i64,
    backoff_multiplier_bps: i64,
    enabled: i64,
    requires_explicit_opt_in: i64,
    payload_json: Option<String>,
    last_run_at: Option<String>,
    last_outcome: Option<String>,
    last_error: Option<String>,
    last_unlock_window: Option<String>,
    cancel_requested: i64,
) -> Result<JobRecord, JobStoreError> {
    if kind.trim().is_empty()
        || kind.len() > 64
        || !kind
            .bytes()
            .all(|character| character.is_ascii_alphanumeric() || b"._-".contains(&character))
    {
        return Err(JobStoreError::InvalidRow(
            "job kind is not a safe routing token".to_owned(),
        ));
    }
    let schedule = job_runtime::Schedule::parse(&cadence)
        .map_err(|error| JobStoreError::InvalidRow(error.to_string()))?;
    let state =
        JobState::parse(&state).map_err(|error| JobStoreError::InvalidRow(error.to_string()))?;
    let last_outcome = last_outcome
        .map(|value| {
            JobOutcome::parse(&value).map_err(|error| JobStoreError::InvalidRow(error.to_string()))
        })
        .transpose()?;
    let attempts = u32::try_from(attempt_count)
        .map_err(|_| JobStoreError::InvalidRow("attempt_count is out of range".to_owned()))?;
    let max_attempts = u32::try_from(max_attempts)
        .map_err(|_| JobStoreError::InvalidRow("max_attempts is out of range".to_owned()))?;
    let initial = chrono::Duration::seconds(backoff_initial_seconds);
    if backoff_initial_seconds < 0 || backoff_multiplier_bps < 10_000 || max_attempts == 0 {
        return Err(JobStoreError::InvalidRow(
            "invalid retry policy in durable job row".to_owned(),
        ));
    }
    Ok(JobRecord {
        id,
        kind,
        schedule,
        next_due_at: parse_timestamp(next_due_at, "next_due_at")?,
        state,
        attempts,
        max_attempts,
        backoff: BackoffPolicy {
            initial,
            multiplier_bps: u32::try_from(backoff_multiplier_bps).map_err(|_| {
                JobStoreError::InvalidRow("backoff multiplier is out of range".to_owned())
            })?,
        },
        enabled: enabled != 0,
        requires_explicit_opt_in: requires_explicit_opt_in != 0,
        payload_json,
        last_run_at: parse_timestamp(last_run_at, "last_run_at")?,
        last_outcome,
        last_error: last_error.map(|value| sanitize_error(&value)),
        last_unlock_window,
        cancel_requested: cancel_requested != 0,
    })
}

const SELECT_COLUMNS: &str = "
    id, kind, cadence, next_due_at, state, attempt_count, max_attempts,
    backoff_initial_seconds, backoff_multiplier_bps, enabled,
    requires_explicit_opt_in, payload_json, last_run_at, last_outcome,
    last_error, last_unlock_window, cancel_requested";

impl DbWorker {
    /// Insert or update a durable schedule.  Runtime state and attempt history
    /// are preserved on updates so changing a cadence cannot silently replay a
    /// currently-running job.
    pub fn schedule_job(&self, spec: &JobSpec) -> Result<(), DbError> {
        spec.validate()
            .map_err(|error| DbError::InvalidCommand(error.to_string()))?;
        let now = Utc::now().to_rfc3339();
        let guard = self.lock();
        guard.conn.execute(
            "INSERT INTO durable_jobs (
                id, kind, cadence, next_due_at, state, attempt_count,
                max_attempts, backoff_initial_seconds, backoff_multiplier_bps,
                enabled, requires_explicit_opt_in, payload_json, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, 'queued', 0, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)
             ON CONFLICT(id) DO UPDATE SET
                kind = excluded.kind,
                cadence = excluded.cadence,
                next_due_at = excluded.next_due_at,
                max_attempts = excluded.max_attempts,
                backoff_initial_seconds = excluded.backoff_initial_seconds,
                backoff_multiplier_bps = excluded.backoff_multiplier_bps,
                enabled = excluded.enabled,
                requires_explicit_opt_in = excluded.requires_explicit_opt_in,
                payload_json = excluded.payload_json,
                updated_at = excluded.updated_at",
            params![
                spec.id,
                spec.kind,
                spec.schedule.as_token(),
                spec.next_due_at.to_rfc3339(),
                i64::from(spec.max_attempts),
                spec.backoff.initial.num_seconds(),
                i64::from(spec.backoff.multiplier_bps),
                if spec.enabled { 1 } else { 0 },
                if spec.requires_explicit_opt_in { 1 } else { 0 },
                spec.payload_json,
                now,
            ],
        )?;
        Ok(())
    }

    /// Replace user-owned configuration for a durable job, resetting terminal
    /// failure/cancellation state so the new settings can take effect. An
    /// active handler is never reconfigured underneath its invocation.
    pub fn reconfigure_job(&self, spec: &JobSpec) -> Result<(), DbError> {
        spec.validate()
            .map_err(|error| DbError::InvalidCommand(error.to_string()))?;
        let now = Utc::now().to_rfc3339();
        let mut guard = self.lock();
        let tx = guard.conn.transaction()?;
        let running: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM durable_jobs WHERE id = ?1 AND state = 'running')",
            params![spec.id],
            |row| row.get(0),
        )?;
        if running {
            return Err(DbError::InvalidCommand(
                "cannot reconfigure a running job".to_owned(),
            ));
        }
        tx.execute(
            "INSERT INTO durable_jobs (
                id, kind, cadence, next_due_at, state, attempt_count,
                max_attempts, backoff_initial_seconds, backoff_multiplier_bps,
                enabled, requires_explicit_opt_in, payload_json, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, 'queued', 0, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)
             ON CONFLICT(id) DO UPDATE SET
                kind = excluded.kind,
                cadence = excluded.cadence,
                next_due_at = excluded.next_due_at,
                state = 'queued',
                attempt_count = 0,
                max_attempts = excluded.max_attempts,
                backoff_initial_seconds = excluded.backoff_initial_seconds,
                backoff_multiplier_bps = excluded.backoff_multiplier_bps,
                enabled = excluded.enabled,
                requires_explicit_opt_in = excluded.requires_explicit_opt_in,
                payload_json = excluded.payload_json,
                last_outcome = NULL,
                last_error = NULL,
                last_unlock_window = NULL,
                cancel_requested = 0,
                updated_at = excluded.updated_at",
            params![
                spec.id,
                spec.kind,
                spec.schedule.as_token(),
                spec.next_due_at.to_rfc3339(),
                i64::from(spec.max_attempts),
                spec.backoff.initial.num_seconds(),
                i64::from(spec.backoff.multiplier_bps),
                if spec.enabled { 1 } else { 0 },
                if spec.requires_explicit_opt_in { 1 } else { 0 },
                spec.payload_json,
                now,
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Read one durable row.
    pub fn durable_job(&self, id: Uuid) -> Result<Option<DurableJobView>, DbError> {
        let conn = self.read_connection()?;
        let sql = format!("SELECT {SELECT_COLUMNS} FROM durable_jobs WHERE id = ?1");
        conn.query_row(&sql, params![id], row_from_query)
            .optional()?
            .transpose()
            .map_err(DbError::from)
    }

    /// Read all durable rows in stable order for Settings/history surfaces.
    pub fn durable_jobs(&self) -> Result<Vec<DurableJobView>, DbError> {
        let conn = self.read_connection()?;
        let sql = format!("SELECT {SELECT_COLUMNS} FROM durable_jobs ORDER BY next_due_at, id");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map([], row_from_query)?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|row| row.map_err(DbError::from))
            .collect()
    }

    /// Request cancellation.  A queued row becomes terminal immediately;
    /// running rows retain their claim and expose the request to cooperative
    /// handlers through [`Self::job_cancellation_requested`].
    pub fn cancel_job(&self, id: Uuid) -> Result<bool, DbError> {
        let now = Utc::now().to_rfc3339();
        let guard = self.lock();
        let changed = guard.conn.execute(
            "UPDATE durable_jobs
                SET state = CASE WHEN state = 'running' THEN state ELSE 'cancelled' END,
                    last_outcome = CASE WHEN state = 'running' THEN last_outcome ELSE 'cancelled' END,
                    last_error = CASE WHEN state = 'running' THEN last_error ELSE 'cancelled by user' END,
                    cancel_requested = 1,
                    updated_at = ?2
              WHERE id = ?1 AND state NOT IN ('failed', 'cancelled')",
            params![id, now],
        )?;
        Ok(changed != 0)
    }

    /// Return whether cancellation was requested for a running job.
    pub fn job_cancellation_requested(&self, id: Uuid) -> Result<bool, DbError> {
        Ok(self
            .read_connection()?
            .query_row(
                "SELECT cancel_requested FROM durable_jobs WHERE id = ?1",
                params![id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .unwrap_or(0)
            != 0)
    }

    /// Run due jobs after unlock.  The caller supplies a context-aware handler;
    /// this method never waits on the unlock mutex itself and is intended for a
    /// desktop `spawn_blocking` task.
    pub fn run_due_jobs<C: ?Sized, E: JobExecutor<C> + ?Sized>(
        &self,
        now: DateTime<Utc>,
        unlock_window: &str,
        context: &C,
        executor: &E,
        cancellation: &CancellationToken,
    ) -> Result<JobRunReport, JobRunnerError<JobStoreError>> {
        let _runner_guard = self
            .runner_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        JobRunner::new(self).run_due(now, unlock_window, context, executor, cancellation)
    }

    /// Clock-injected scheduler entry point for deterministic tests and
    /// hosts with an explicit time source.
    pub fn run_due_jobs_with_clock<C: ?Sized, E: JobExecutor<C> + ?Sized>(
        &self,
        now: DateTime<Utc>,
        unlock_window: &str,
        context: &C,
        executor: &E,
        cancellation: &CancellationToken,
        clock: &dyn Clock,
    ) -> Result<JobRunReport, JobRunnerError<JobStoreError>> {
        let _runner_guard = self
            .runner_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        JobRunner::new(self).run_due_with_clock(
            now,
            unlock_window,
            context,
            executor,
            cancellation,
            clock,
        )
    }
}

fn row_from_query(
    row: &rusqlite::Row<'_>,
) -> Result<Result<JobRecord, JobStoreError>, rusqlite::Error> {
    Ok(row_from_parts(
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
        row.get(11)?,
        row.get(12)?,
        row.get(13)?,
        row.get(14)?,
        row.get(15)?,
        row.get(16)?,
    ))
}

impl JobStore for DbWorker {
    type Error = JobStoreError;

    fn recover_running(&self, now: DateTime<Utc>) -> Result<u64, Self::Error> {
        let mut guard = self.lock();
        let tx = guard.conn.transaction()?;
        let now = now.to_rfc3339();
        let cancelled = tx.execute(
            "UPDATE durable_jobs
                SET state = 'cancelled', last_outcome = 'cancelled',
                    last_error = 'cancelled before recovery',
                    cancel_requested = 0, updated_at = ?1
              WHERE state = 'running' AND cancel_requested = 1",
            params![now],
        )?;
        let requeued = tx.execute(
            "UPDATE durable_jobs
                SET state = 'queued',
                    next_due_at = CASE
                        WHEN next_due_at IS NULL OR next_due_at > ?1 THEN ?1
                        ELSE next_due_at
                    END,
                    cancel_requested = 0,
                    updated_at = ?1
              WHERE state = 'running' AND cancel_requested = 0",
            params![now],
        )?;
        tx.commit()?;
        Ok((cancelled + requeued) as u64)
    }

    fn due_jobs(
        &self,
        now: DateTime<Utc>,
        unlock_window: &str,
    ) -> Result<Vec<JobRecord>, Self::Error> {
        let conn = self.read_connection()?;
        let sql = format!(
            "SELECT {SELECT_COLUMNS} FROM durable_jobs
              WHERE enabled = 1
                AND state IN ('queued', 'succeeded')
                AND next_due_at IS NOT NULL
                AND next_due_at <= ?1
                AND (last_unlock_window IS NULL OR last_unlock_window <> ?2)
              ORDER BY next_due_at, id"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![now.to_rfc3339(), unlock_window], row_from_query)?;
        rows.collect::<Result<Vec<_>, _>>()?.into_iter().collect()
    }

    fn claim_job(
        &self,
        id: Uuid,
        attempt: u32,
        started_at: DateTime<Utc>,
        unlock_window: &str,
    ) -> Result<bool, Self::Error> {
        let guard = self.lock();
        let changed = guard.conn.execute(
            "UPDATE durable_jobs
                SET state = 'running', attempt_count = ?2, last_run_at = ?3,
                    last_unlock_window = ?4, cancel_requested = 0, updated_at = ?3
              WHERE id = ?1
                AND state IN ('queued', 'succeeded')
                AND attempt_count = ?2 - 1
                AND (last_unlock_window IS NULL OR last_unlock_window <> ?4)",
            params![
                id,
                i64::from(attempt),
                started_at.to_rfc3339(),
                unlock_window
            ],
        )?;
        Ok(changed != 0)
    }

    fn cancellation_requested(&self, id: Uuid) -> Result<bool, Self::Error> {
        self.job_cancellation_requested(id)
            .map_err(JobStoreError::Db)
    }

    fn finish_success(
        &self,
        job: &JobRecord,
        completed_at: DateTime<Utc>,
        next_due_at: Option<DateTime<Utc>>,
    ) -> Result<JobFinishResult, Self::Error> {
        let guard = self.lock();
        let changed = guard.conn.execute(
            "UPDATE durable_jobs
                SET state = 'succeeded', attempt_count = 0,
                    next_due_at = ?2, last_outcome = 'succeeded',
                    last_error = NULL, cancel_requested = 0, updated_at = ?3
              WHERE id = ?1 AND state = 'running' AND cancel_requested = 0",
            params![
                job.id,
                next_due_at.map(|value| value.to_rfc3339()),
                completed_at.to_rfc3339()
            ],
        )?;
        Ok(if changed == 0 {
            JobFinishResult::Cancelled
        } else {
            JobFinishResult::Applied
        })
    }

    fn finish_failure(
        &self,
        job: &JobRecord,
        failed_at: DateTime<Utc>,
        failure: &JobFailure,
        retry_at: Option<DateTime<Utc>>,
    ) -> Result<JobFinishResult, Self::Error> {
        let state = if retry_at.is_some() {
            "queued"
        } else {
            "failed"
        };
        let guard = self.lock();
        let changed = guard.conn.execute(
            "UPDATE durable_jobs
                SET state = ?2, next_due_at = COALESCE(?3, next_due_at),
                    last_outcome = 'failed', last_error = ?4,
                    cancel_requested = 0, updated_at = ?5
              WHERE id = ?1 AND state IN ('running', 'queued', 'succeeded')
                AND cancel_requested = 0",
            params![
                job.id,
                state,
                retry_at.map(|value| value.to_rfc3339()),
                sanitize_error(&failure.reason),
                failed_at.to_rfc3339(),
            ],
        )?;
        Ok(if changed == 0 {
            JobFinishResult::Cancelled
        } else {
            JobFinishResult::Applied
        })
    }

    fn finish_cancelled(
        &self,
        job: &JobRecord,
        cancelled_at: DateTime<Utc>,
        reason: &str,
    ) -> Result<(), Self::Error> {
        let guard = self.lock();
        guard.conn.execute(
            "UPDATE durable_jobs
                SET state = 'cancelled', last_outcome = 'cancelled',
                    last_error = ?2, updated_at = ?3
              WHERE id = ?1 AND state = 'running'",
            params![job.id, sanitize_error(reason), cancelled_at.to_rfc3339()],
        )?;
        Ok(())
    }

    fn finish_skipped(
        &self,
        job: &JobRecord,
        skipped_at: DateTime<Utc>,
    ) -> Result<JobFinishResult, Self::Error> {
        let guard = self.lock();
        let changed = guard.conn.execute(
            "UPDATE durable_jobs
                SET state = 'queued', last_outcome = 'skipped',
                    cancel_requested = 0, updated_at = ?2
              WHERE id = ?1 AND state = 'running' AND cancel_requested = 0",
            params![job.id, skipped_at.to_rfc3339()],
        )?;
        Ok(if changed == 0 {
            JobFinishResult::Cancelled
        } else {
            JobFinishResult::Applied
        })
    }
}
