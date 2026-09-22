//! Small, local-only durable job runtime primitives.
//!
//! The runtime deliberately does not know about SQLCipher, vault keys, or any
//! financial value.  A [`JobStore`] supplies durable state and a
//! [`JobExecutor`] supplies the actual work.  This keeps the scheduler
//! testable with an in-memory store while [`db-worker`] remains the only crate
//! that owns the encrypted database connection.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration as StdDuration, Instant};

use chrono::{DateTime, Duration, Utc};
use thiserror::Error;
use tracing::info_span;
use uuid::Uuid;

/// The cancellation service-level objective for a cooperative job handler.
pub const CANCELLATION_SLA: StdDuration = StdDuration::from_millis(500);
/// Poll interval for a DB-backed cancellation request while a handler runs.
pub const CANCELLATION_POLL_INTERVAL: StdDuration = StdDuration::from_millis(25);

/// A wall-clock source injected into scheduling tests and hosts that need
/// deterministic time. Production callers use [`SystemClock`].
pub trait Clock: Send + Sync {
    /// Return the current UTC instant.
    fn now(&self) -> DateTime<Utc>;
}

/// Production clock backed by `Utc::now()`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// A schedule persisted in a durable job row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Schedule {
    /// A job that runs once and then remains in `succeeded` or `failed`.
    Once,
    /// A fixed interval in whole seconds.  The interval must be positive.
    Interval { seconds: i64 },
    /// A calendar-day cadence.  UTC is used by the runtime; callers choose the
    /// household timezone when calculating the initial due time.
    Daily,
    /// A seven-day cadence.
    Weekly,
}

impl Schedule {
    /// Parse the compact database representation.
    pub fn parse(value: &str) -> Result<Self, ScheduleError> {
        match value {
            "once" => Ok(Self::Once),
            "daily" => Ok(Self::Daily),
            "weekly" => Ok(Self::Weekly),
            value => value
                .strip_prefix("interval:")
                .ok_or_else(|| ScheduleError::Invalid(value.to_owned()))
                .and_then(|seconds| {
                    let seconds = seconds
                        .parse::<i64>()
                        .map_err(|_| ScheduleError::Invalid(value.to_owned()))?;
                    if seconds <= 0 {
                        return Err(ScheduleError::NonPositiveInterval);
                    }
                    Ok(Self::Interval { seconds })
                }),
        }
    }

    /// Return the compact database representation.
    #[must_use]
    pub fn as_token(&self) -> String {
        match self {
            Self::Once => "once".to_owned(),
            Self::Daily => "daily".to_owned(),
            Self::Weekly => "weekly".to_owned(),
            Self::Interval { seconds } => format!("interval:{seconds}"),
        }
    }

    /// Calculate the next due time after a successful run.
    #[must_use]
    pub fn next_due_after(&self, completed_at: DateTime<Utc>) -> Option<DateTime<Utc>> {
        match self {
            Self::Once => None,
            Self::Interval { seconds } => Some(completed_at + Duration::seconds(*seconds)),
            Self::Daily => Some(completed_at + Duration::days(1)),
            Self::Weekly => Some(completed_at + Duration::weeks(1)),
        }
    }
}

/// Errors in a persisted schedule token.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ScheduleError {
    /// The token is not one of the supported schedule forms.
    #[error("invalid job schedule {0:?}")]
    Invalid(String),
    /// An interval must advance time.
    #[error("job interval must be positive")]
    NonPositiveInterval,
}

/// Retry policy for one job class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackoffPolicy {
    /// Initial delay after the first retryable failure.
    pub initial: Duration,
    /// Multiplier in basis points. `20_000` means 2x.
    pub multiplier_bps: u32,
}

impl Default for BackoffPolicy {
    fn default() -> Self {
        Self {
            initial: Duration::minutes(1),
            multiplier_bps: 20_000,
        }
    }
}

impl BackoffPolicy {
    /// Calculate the delay before the next attempt.  The exponent is capped
    /// so malformed or very old rows cannot overflow a duration.
    #[must_use]
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let mut seconds = self.initial.num_seconds().max(0) as i128;
        let multiplier = i128::from(self.multiplier_bps.max(10_000));
        for _ in 1..attempt.min(31) {
            seconds = seconds.saturating_mul(multiplier) / 10_000;
        }
        let seconds = seconds.min(i128::from(i64::MAX));
        Duration::seconds(seconds as i64)
    }
}

/// The specification stored for a job class/instance.
#[derive(Clone, PartialEq, Eq)]
pub struct JobSpec {
    /// Stable identity of this scheduled job.
    pub id: Uuid,
    /// Safe, non-sensitive job-kind token used in tracing and UI labels.
    pub kind: String,
    /// Cadence for this job.
    pub schedule: Schedule,
    /// The first (or next) time at which the job is eligible.
    pub next_due_at: DateTime<Utc>,
    /// Maximum attempts for one due run, including the initial attempt.
    pub max_attempts: u32,
    /// Retry delay policy for retryable failures.
    pub backoff: BackoffPolicy,
    /// Whether the job is eligible to run.
    pub enabled: bool,
    /// Jobs such as connector work must have an explicit user setting checked
    /// by their executor before they are allowed to run.
    pub requires_explicit_opt_in: bool,
    /// Opaque encrypted-vault configuration.  The runtime never logs it.
    pub payload_json: Option<String>,
}

impl fmt::Debug for JobSpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("JobSpec")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("schedule", &self.schedule)
            .field("next_due_at", &self.next_due_at)
            .field("max_attempts", &self.max_attempts)
            .field("backoff", &self.backoff)
            .field("enabled", &self.enabled)
            .field("requires_explicit_opt_in", &self.requires_explicit_opt_in)
            .field(
                "payload_json",
                &self.payload_json.as_ref().map(|_| "[redacted]"),
            )
            .finish()
    }
}

impl JobSpec {
    /// Validate values before they are persisted.
    pub fn validate(&self) -> Result<(), JobSpecError> {
        validate_job_kind(&self.kind)?;
        if self.max_attempts == 0 {
            return Err(JobSpecError::ZeroAttempts);
        }
        if self.backoff.initial < Duration::zero() {
            return Err(JobSpecError::NegativeBackoff);
        }
        if self.backoff.multiplier_bps < 10_000 {
            return Err(JobSpecError::BackoffMultiplierTooSmall);
        }
        if let Schedule::Interval { seconds } = &self.schedule {
            if *seconds <= 0 {
                return Err(JobSpecError::NonPositiveInterval);
            }
        }
        Ok(())
    }
}

fn validate_job_kind(kind: &str) -> Result<(), JobSpecError> {
    if kind.trim().is_empty() {
        return Err(JobSpecError::EmptyKind);
    }
    if kind.len() > 64
        || !kind
            .bytes()
            .all(|character| character.is_ascii_alphanumeric() || b"._-".contains(&character))
    {
        return Err(JobSpecError::InvalidKind);
    }
    Ok(())
}

/// Validation failures for a job specification.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum JobSpecError {
    /// A stable kind is required for routing and safe logs.
    #[error("job kind cannot be empty")]
    EmptyKind,
    /// Job kinds are routing tokens, not free-form user text.
    #[error("job kind must be an ASCII token no longer than 64 characters")]
    InvalidKind,
    /// Jobs must have at least one attempt.
    #[error("job max_attempts must be greater than zero")]
    ZeroAttempts,
    /// Backoff delays cannot move backwards.
    #[error("job backoff cannot be negative")]
    NegativeBackoff,
    /// A multiplier below 1x would make retries happen sooner each time.
    #[error("job backoff multiplier must be at least 1x")]
    BackoffMultiplierTooSmall,
    /// An interval must advance time.
    #[error("job interval must be positive")]
    NonPositiveInterval,
}

/// Durable lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    /// Eligible or waiting for its due time.
    Queued,
    /// Claimed by one runtime invocation.
    Running,
    /// A one-shot job completed successfully, or a recurring job has no
    /// pending attempt after a successful run.
    Succeeded,
    /// The job exhausted attempts or returned a non-retryable failure.
    Failed,
    /// The job was cooperatively cancelled.
    Cancelled,
}

impl JobState {
    /// Database token for this state.
    #[must_use]
    pub const fn as_token(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    /// Parse a database token.
    pub fn parse(value: &str) -> Result<Self, JobStateError> {
        match value {
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            other => Err(JobStateError::Invalid(other.to_owned())),
        }
    }
}

/// Invalid durable state token.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum JobStateError {
    /// The database contains an unknown state.
    #[error("invalid durable job state {0:?}")]
    Invalid(String),
}

/// A durable row loaded by a store.
#[derive(Clone, PartialEq, Eq)]
pub struct JobRecord {
    /// Stable job identity.
    pub id: Uuid,
    /// Safe routing kind.
    pub kind: String,
    /// Cadence.
    pub schedule: Schedule,
    /// Current due time. `None` means the one-shot job has completed.
    pub next_due_at: Option<DateTime<Utc>>,
    /// Current lifecycle state.
    pub state: JobState,
    /// Number of attempts made for the current due run.
    pub attempts: u32,
    /// Attempt ceiling.
    pub max_attempts: u32,
    /// Retry policy.
    pub backoff: BackoffPolicy,
    /// Whether this row is enabled.
    pub enabled: bool,
    /// Explicit opt-in requirement passed to the executor.
    pub requires_explicit_opt_in: bool,
    /// Opaque encrypted-vault configuration.
    pub payload_json: Option<String>,
    /// Last invocation timestamp.
    pub last_run_at: Option<DateTime<Utc>>,
    /// Last durable outcome token.
    pub last_outcome: Option<JobOutcome>,
    /// Sanitized plain-language failure reason, if any.
    pub last_error: Option<String>,
    /// Unlock window in which the last attempt was claimed.
    pub last_unlock_window: Option<String>,
    /// Whether a user has requested cancellation of a running invocation.
    pub cancel_requested: bool,
}

impl fmt::Debug for JobRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("JobRecord")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("schedule", &self.schedule)
            .field("next_due_at", &self.next_due_at)
            .field("state", &self.state)
            .field("attempts", &self.attempts)
            .field("max_attempts", &self.max_attempts)
            .field("backoff", &self.backoff)
            .field("enabled", &self.enabled)
            .field("requires_explicit_opt_in", &self.requires_explicit_opt_in)
            .field(
                "payload_json",
                &self.payload_json.as_ref().map(|_| "[redacted]"),
            )
            .field("last_run_at", &self.last_run_at)
            .field("last_outcome", &self.last_outcome)
            .field(
                "last_error",
                &self.last_error.as_ref().map(|_| "[redacted]"),
            )
            .field("last_unlock_window", &self.last_unlock_window)
            .field("cancel_requested", &self.cancel_requested)
            .finish()
    }
}

/// Durable outcome token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobOutcome {
    /// Invocation completed.
    Succeeded,
    /// Invocation failed.
    Failed,
    /// Invocation was cancelled.
    Cancelled,
    /// Invocation was skipped by an executor policy.
    Skipped,
}

impl JobOutcome {
    /// Database token.
    #[must_use]
    pub const fn as_token(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Skipped => "skipped",
        }
    }

    /// Parse a database token.
    pub fn parse(value: &str) -> Result<Self, JobOutcomeError> {
        match value {
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            "skipped" => Ok(Self::Skipped),
            other => Err(JobOutcomeError::Invalid(other.to_owned())),
        }
    }
}

/// Invalid durable outcome token.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum JobOutcomeError {
    /// The database contains an unknown outcome.
    #[error("invalid durable job outcome {0:?}")]
    Invalid(String),
}

/// A failure returned by an executor.  The message is sanitized by the store
/// before it reaches durable state or a UI surface.
#[derive(Clone, PartialEq, Eq)]
pub struct JobFailure {
    /// Plain-language reason suitable for the Money Inbox.
    pub reason: String,
    /// Whether the same due run may be retried after backoff.
    pub retryable: bool,
}

impl fmt::Debug for JobFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("JobFailure")
            .field("reason", &"[redacted]")
            .field("retryable", &self.retryable)
            .finish()
    }
}

impl JobFailure {
    /// Construct a retryable failure.
    #[must_use]
    pub fn retryable(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            retryable: true,
        }
    }

    /// Construct a terminal failure.
    #[must_use]
    pub fn permanent(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            retryable: false,
        }
    }
}

/// Result of one executor invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobExecution {
    /// Work committed successfully.
    Succeeded,
    /// Work failed with a durable reason.
    Failed(JobFailure),
    /// Work observed cancellation and committed nothing.
    Cancelled,
    /// Work was deliberately not run (for example, an opt-in setting is off).
    Skipped,
}

/// Result of an atomic terminal transition.
///
/// A store returns [`Self::Cancelled`] when a cancellation request won the
/// race with a success/failure/skip write.  The runner then records the
/// cancellation outcome instead of claiming that the handler completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobFinishResult {
    /// The requested terminal state was persisted.
    Applied,
    /// A cancellation request was observed before the terminal write.
    Cancelled,
}

/// Cooperative cancellation shared by the runtime and a handler.
#[derive(Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl fmt::Debug for CancellationToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CancellationToken")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

impl CancellationToken {
    /// Create a new non-cancelled token.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Request cancellation.  The request is lock-free and visible to all
    /// clones immediately.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    /// Whether cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    /// Return an error when a handler reaches a cooperative checkpoint.
    pub fn check(&self) -> Result<(), CancellationError> {
        if self.is_cancelled() {
            Err(CancellationError)
        } else {
            Ok(())
        }
    }

    /// Sleep in short checkpoints so a handler can honor cancellation without
    /// waiting for a long OS sleep to finish.  Returns `false` when cancelled.
    pub fn sleep(&self, duration: StdDuration) -> bool {
        let start = Instant::now();
        while start.elapsed() < duration {
            if self.is_cancelled() {
                return false;
            }
            let remaining = duration.saturating_sub(start.elapsed());
            std::thread::sleep(remaining.min(CANCELLATION_SLA / 4));
        }
        !self.is_cancelled()
    }
}

/// Error returned at a cooperative cancellation checkpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("job cancelled")]
pub struct CancellationError;

/// A durable state store.  Implementations must make claim and terminal-state
/// transitions atomic so two unlock workers cannot run the same row.
pub trait JobStore {
    /// Store-specific error type.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Recover rows left `running` by a crash and return them to the queue.
    fn recover_running(&self, now: DateTime<Utc>) -> Result<u64, Self::Error>;
    /// List rows due at `now` that have not been claimed in `unlock_window`.
    fn due_jobs(
        &self,
        now: DateTime<Utc>,
        unlock_window: &str,
    ) -> Result<Vec<JobRecord>, Self::Error>;
    /// Atomically claim a row for `attempt` in an unlock window.
    fn claim_job(
        &self,
        id: Uuid,
        attempt: u32,
        started_at: DateTime<Utc>,
        unlock_window: &str,
    ) -> Result<bool, Self::Error>;
    /// Check a durable cancellation request for a running row.
    fn cancellation_requested(&self, _id: Uuid) -> Result<bool, Self::Error> {
        Ok(false)
    }
    /// Record a successful invocation and its next schedule state.
    fn finish_success(
        &self,
        job: &JobRecord,
        completed_at: DateTime<Utc>,
        next_due_at: Option<DateTime<Utc>>,
    ) -> Result<JobFinishResult, Self::Error>;
    /// Record a failure. `retry_at` is `Some` only when another attempt is due.
    fn finish_failure(
        &self,
        job: &JobRecord,
        failed_at: DateTime<Utc>,
        failure: &JobFailure,
        retry_at: Option<DateTime<Utc>>,
    ) -> Result<JobFinishResult, Self::Error>;
    /// Record a cancellation without claiming success.
    fn finish_cancelled(
        &self,
        job: &JobRecord,
        cancelled_at: DateTime<Utc>,
        reason: &str,
    ) -> Result<(), Self::Error>;
    /// Record a policy skip while leaving the job eligible for a later unlock.
    fn finish_skipped(
        &self,
        job: &JobRecord,
        skipped_at: DateTime<Utc>,
    ) -> Result<JobFinishResult, Self::Error>;
}

/// A registered consumer for one durable job kind.
pub trait JobHandler<C: ?Sized>: Send + Sync {
    /// Stable job-kind token used to route a row to this handler.
    fn kind(&self) -> &'static str;

    /// Decide whether the handler is enabled by current user settings.
    fn should_run(&self, job: &JobRecord, _context: &C) -> bool {
        !job.requires_explicit_opt_in
    }

    /// Execute one claimed job.
    fn execute(
        &self,
        job: &JobRecord,
        cancellation: &CancellationToken,
        context: &C,
    ) -> JobExecution;
}

/// Process-local dispatcher that routes durable rows to registered consumers.
///
/// The dispatcher is intentionally empty by default: a job with no registered
/// consumer is skipped (and remains queued for a later unlock), while an
/// opt-in job still fails closed through the handler policy. Feature beads add
/// their handlers without changing the scheduler or unlock lifecycle.
pub struct JobDispatcher<C: ?Sized> {
    handlers: RwLock<BTreeMap<String, Arc<dyn JobHandler<C>>>>,
}

impl<C: ?Sized> Default for JobDispatcher<C> {
    fn default() -> Self {
        Self {
            handlers: RwLock::new(BTreeMap::new()),
        }
    }
}

impl<C: ?Sized> JobDispatcher<C> {
    /// Create an empty dispatcher.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register or replace a handler for its stable kind token.
    ///
    /// The token is validated with the same constraints as [`JobSpec::kind`]
    /// so it is safe to use in SQL rows, tracing fields, and UI labels.
    pub fn register(&self, handler: Arc<dyn JobHandler<C>>) -> Result<(), JobSpecError> {
        validate_job_kind(handler.kind())?;
        if let Ok(mut handlers) = self.handlers.write() {
            handlers.insert(handler.kind().to_owned(), handler);
        }
        Ok(())
    }

    /// Whether a handler for `kind` is currently registered.
    #[must_use]
    pub fn contains(&self, kind: &str) -> bool {
        self.handlers
            .read()
            .map(|handlers| handlers.contains_key(kind))
            .unwrap_or(false)
    }
}

impl<C: ?Sized> JobExecutor<C> for JobDispatcher<C> {
    fn should_run(&self, job: &JobRecord, context: &C) -> bool {
        self.handlers
            .read()
            .ok()
            .and_then(|handlers| handlers.get(&job.kind).cloned())
            .is_some_and(|handler| handler.should_run(job, context))
    }

    fn execute(
        &self,
        job: &JobRecord,
        cancellation: &CancellationToken,
        context: &C,
    ) -> JobExecution {
        self.handlers
            .read()
            .ok()
            .and_then(|handlers| handlers.get(&job.kind).cloned())
            .map_or(JobExecution::Skipped, |handler| {
                handler.execute(job, cancellation, context)
            })
    }
}

/// A job handler.  The context type lets the host pass its safe kernel façade
/// without coupling this crate to the finance kernel or SQLCipher.
pub trait JobExecutor<C: ?Sized>: Send + Sync {
    /// Decide whether this job is allowed to run under current settings.
    fn should_run(&self, job: &JobRecord, _context: &C) -> bool {
        // Opt-in jobs are fail-closed unless their consumer explicitly checks
        // its setting and overrides this method.
        !job.requires_explicit_opt_in
    }

    /// Execute one claimed job.  The handler owns the domain transaction and
    /// must checkpoint the token before and during long work.
    fn execute(
        &self,
        job: &JobRecord,
        cancellation: &CancellationToken,
        context: &C,
    ) -> JobExecution;
}

/// Summary of one unlock-window sweep.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct JobRunReport {
    /// Rows recovered from an interrupted prior invocation.
    pub recovered: u64,
    /// Rows claimed and executed.
    pub attempted: u64,
    /// Successful executions.
    pub succeeded: u64,
    /// Failed executions.
    pub failed: u64,
    /// Cancellations.
    pub cancelled: u64,
    /// Policy skips.
    pub skipped: u64,
}

/// Errors raised by the runner/store boundary.
#[derive(Debug, Error)]
pub enum JobRunnerError<E: std::error::Error + Send + Sync + 'static> {
    /// Durable storage failed.
    #[error("job store failed: {0}")]
    Store(#[source] E),
}

/// Stateless runner for one unlocked sweep.
pub struct JobRunner<'store, S> {
    store: &'store S,
}

impl<'store, S> JobRunner<'store, S>
where
    S: JobStore + Sync,
{
    /// Construct a runner over a durable store.
    #[must_use]
    pub fn new(store: &'store S) -> Self {
        Self { store }
    }

    /// Run all rows due in one unlock window.  The method is synchronous by
    /// design; desktop callers put it on `spawn_blocking` after unlock.
    pub fn run_due<C: ?Sized, E: JobExecutor<C> + ?Sized>(
        &self,
        now: DateTime<Utc>,
        unlock_window: &str,
        context: &C,
        executor: &E,
        cancellation: &CancellationToken,
    ) -> Result<JobRunReport, JobRunnerError<S::Error>> {
        let clock = SystemClock;
        self.run_due_with_clock(now, unlock_window, context, executor, cancellation, &clock)
    }

    /// Clock-injected variant used by deterministic hosts and tests.
    pub fn run_due_with_clock<C: ?Sized, E: JobExecutor<C> + ?Sized>(
        &self,
        now: DateTime<Utc>,
        unlock_window: &str,
        context: &C,
        executor: &E,
        cancellation: &CancellationToken,
        clock: &dyn Clock,
    ) -> Result<JobRunReport, JobRunnerError<S::Error>> {
        let recovered = self
            .store
            .recover_running(now)
            .map_err(JobRunnerError::Store)?;
        let jobs = self
            .store
            .due_jobs(now, unlock_window)
            .map_err(JobRunnerError::Store)?;
        let mut report = JobRunReport {
            recovered,
            ..JobRunReport::default()
        };

        for job in jobs {
            if cancellation.is_cancelled() {
                break;
            }
            let attempt = job.attempts.saturating_add(1);
            if attempt > job.max_attempts {
                let failure = JobFailure::permanent("maximum attempts reached");
                let finish = self
                    .store
                    .finish_failure(&job, now, &failure, None)
                    .map_err(JobRunnerError::Store)?;
                if finish == JobFinishResult::Cancelled {
                    self.store
                        .finish_cancelled(&job, clock.now(), "job cancelled")
                        .map_err(JobRunnerError::Store)?;
                    report.cancelled += 1;
                } else {
                    report.failed += 1;
                }
                continue;
            }
            if !self
                .store
                .claim_job(job.id, attempt, now, unlock_window)
                .map_err(JobRunnerError::Store)?
            {
                continue;
            }

            if self
                .store
                .cancellation_requested(job.id)
                .map_err(JobRunnerError::Store)?
            {
                self.store
                    .finish_cancelled(&job, clock.now(), "job cancelled")
                    .map_err(JobRunnerError::Store)?;
                report.cancelled += 1;
                continue;
            }

            if !executor.should_run(&job, context) {
                let finish = self
                    .store
                    .finish_skipped(&job, clock.now())
                    .map_err(JobRunnerError::Store)?;
                if finish == JobFinishResult::Cancelled {
                    self.store
                        .finish_cancelled(&job, clock.now(), "job cancelled")
                        .map_err(JobRunnerError::Store)?;
                    report.cancelled += 1;
                } else {
                    report.skipped += 1;
                }
                tracing::info!(job.kind = %job.kind, outcome = "skipped", "durable job policy skipped invocation");
                continue;
            }
            report.attempted += 1;

            let span = info_span!(
                "job.invocation",
                job.kind = %job.kind,
                attempt,
                outcome = tracing::field::Empty,
            );
            let _entered = span.enter();
            let job_cancellation = CancellationToken::new();
            if cancellation.is_cancelled() {
                job_cancellation.cancel();
            }
            let (execution, was_cancelled) = std::thread::scope(|scope| {
                let watcher_token = job_cancellation.clone();
                let global_token = cancellation.clone();
                let watcher_stop = CancellationToken::new();
                let watcher_stop_for_thread = watcher_stop.clone();
                let store = self.store;
                let job_id = job.id;
                let _watcher = scope.spawn(move || {
                    while !watcher_stop_for_thread.is_cancelled() && !watcher_token.is_cancelled() {
                        let cancellation_requested = match store.cancellation_requested(job_id) {
                            Ok(requested) => requested,
                            Err(_) => {
                                // A store read failure must not turn into an
                                // accidental successful commit. The handler
                                // will see cancellation and the terminal
                                // transition will report any follow-up store
                                // error to the caller.
                                watcher_token.cancel();
                                break;
                            }
                        };
                        if global_token.is_cancelled() || cancellation_requested {
                            watcher_token.cancel();
                            break;
                        }
                        std::thread::sleep(CANCELLATION_POLL_INTERVAL);
                    }
                });
                let execution = executor.execute(&job, &job_cancellation, context);
                // Stop the scoped DB poller before joining the scope.  A
                // handler that returned normally no longer needs a watcher.
                watcher_stop.cancel();
                (execution, job_cancellation.is_cancelled())
            });
            let execution = if was_cancelled {
                // Cancellation wins even if a handler races with the watcher
                // and returns a late success/failure result. This is the
                // no-partial-commit boundary for cooperative consumers.
                JobExecution::Cancelled
            } else {
                execution
            };
            let completed_at = clock.now();
            match execution {
                JobExecution::Succeeded => {
                    span.record("outcome", "succeeded");
                    let finish = self
                        .store
                        .finish_success(
                            &job,
                            completed_at,
                            job.schedule.next_due_after(completed_at),
                        )
                        .map_err(JobRunnerError::Store)?;
                    if finish == JobFinishResult::Cancelled {
                        self.store
                            .finish_cancelled(&job, completed_at, "job cancelled")
                            .map_err(JobRunnerError::Store)?;
                        span.record("outcome", "cancelled");
                        report.cancelled += 1;
                    } else {
                        report.succeeded += 1;
                    }
                }
                JobExecution::Failed(failure) => {
                    span.record("outcome", "failed");
                    let retry_at = (failure.retryable && attempt < job.max_attempts)
                        .then(|| completed_at + job.backoff.delay_for_attempt(attempt));
                    let finish = self
                        .store
                        .finish_failure(&job, completed_at, &failure, retry_at)
                        .map_err(JobRunnerError::Store)?;
                    if finish == JobFinishResult::Cancelled {
                        self.store
                            .finish_cancelled(&job, completed_at, "job cancelled")
                            .map_err(JobRunnerError::Store)?;
                        span.record("outcome", "cancelled");
                        report.cancelled += 1;
                    } else {
                        report.failed += 1;
                    }
                }
                JobExecution::Cancelled => {
                    span.record("outcome", "cancelled");
                    self.store
                        .finish_cancelled(&job, completed_at, "job cancelled")
                        .map_err(JobRunnerError::Store)?;
                    report.cancelled += 1;
                }
                JobExecution::Skipped => {
                    span.record("outcome", "skipped");
                    self.store
                        .finish_skipped(&job, completed_at)
                        .map_err(JobRunnerError::Store)?;
                    report.skipped += 1;
                }
            }
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex};

    use tracing_subscriber::fmt::format::FmtSpan;

    use super::*;

    #[derive(Clone, Copy)]
    struct FixedClock(DateTime<Utc>);

    impl Clock for FixedClock {
        fn now(&self) -> DateTime<Utc> {
            self.0
        }
    }

    #[derive(Debug, Error)]
    #[error("fake store")]
    struct FakeError;

    #[derive(Default)]
    struct FakeStore {
        jobs: Mutex<HashMap<Uuid, JobRecord>>,
    }

    impl JobStore for FakeStore {
        type Error = FakeError;

        fn recover_running(&self, _now: DateTime<Utc>) -> Result<u64, Self::Error> {
            let mut jobs = self.jobs.lock().unwrap();
            let mut count = 0;
            for job in jobs.values_mut() {
                if job.state == JobState::Running {
                    job.state = JobState::Queued;
                    count += 1;
                }
            }
            Ok(count)
        }

        fn due_jobs(
            &self,
            now: DateTime<Utc>,
            window: &str,
        ) -> Result<Vec<JobRecord>, Self::Error> {
            Ok(self
                .jobs
                .lock()
                .unwrap()
                .values()
                .filter(|job| {
                    job.enabled
                        && job.state == JobState::Queued
                        && job.next_due_at.is_some_and(|due| due <= now)
                        && job.last_unlock_window.as_deref() != Some(window)
                })
                .cloned()
                .collect())
        }

        fn claim_job(
            &self,
            id: Uuid,
            attempt: u32,
            started_at: DateTime<Utc>,
            window: &str,
        ) -> Result<bool, Self::Error> {
            let mut jobs = self.jobs.lock().unwrap();
            let job = jobs.get_mut(&id).unwrap();
            if job.state != JobState::Queued || job.attempts + 1 != attempt {
                return Ok(false);
            }
            job.state = JobState::Running;
            job.attempts = attempt;
            job.last_run_at = Some(started_at);
            job.last_unlock_window = Some(window.to_owned());
            Ok(true)
        }

        fn cancellation_requested(&self, id: Uuid) -> Result<bool, Self::Error> {
            Ok(self.jobs.lock().unwrap()[&id].cancel_requested)
        }

        fn finish_success(
            &self,
            job: &JobRecord,
            completed_at: DateTime<Utc>,
            next_due_at: Option<DateTime<Utc>>,
        ) -> Result<JobFinishResult, Self::Error> {
            let mut jobs = self.jobs.lock().unwrap();
            let row = jobs.get_mut(&job.id).unwrap();
            if row.cancel_requested {
                return Ok(JobFinishResult::Cancelled);
            }
            row.state = JobState::Succeeded;
            row.attempts = 0;
            row.last_outcome = Some(JobOutcome::Succeeded);
            row.last_run_at = Some(completed_at);
            row.next_due_at = next_due_at;
            Ok(JobFinishResult::Applied)
        }

        fn finish_failure(
            &self,
            job: &JobRecord,
            _failed_at: DateTime<Utc>,
            failure: &JobFailure,
            retry_at: Option<DateTime<Utc>>,
        ) -> Result<JobFinishResult, Self::Error> {
            let mut jobs = self.jobs.lock().unwrap();
            let row = jobs.get_mut(&job.id).unwrap();
            if row.cancel_requested {
                return Ok(JobFinishResult::Cancelled);
            }
            row.state = if retry_at.is_some() {
                JobState::Queued
            } else {
                JobState::Failed
            };
            row.last_outcome = Some(JobOutcome::Failed);
            row.last_error = Some(failure.reason.clone());
            row.next_due_at = retry_at.or(row.next_due_at);
            Ok(JobFinishResult::Applied)
        }

        fn finish_cancelled(
            &self,
            job: &JobRecord,
            _cancelled_at: DateTime<Utc>,
            reason: &str,
        ) -> Result<(), Self::Error> {
            let mut jobs = self.jobs.lock().unwrap();
            let row = jobs.get_mut(&job.id).unwrap();
            row.state = JobState::Cancelled;
            row.last_outcome = Some(JobOutcome::Cancelled);
            row.last_error = Some(reason.to_owned());
            Ok(())
        }

        fn finish_skipped(
            &self,
            job: &JobRecord,
            _skipped_at: DateTime<Utc>,
        ) -> Result<JobFinishResult, Self::Error> {
            let mut jobs = self.jobs.lock().unwrap();
            let row = jobs.get_mut(&job.id).unwrap();
            if row.cancel_requested {
                return Ok(JobFinishResult::Cancelled);
            }
            row.state = JobState::Queued;
            row.last_outcome = Some(JobOutcome::Skipped);
            Ok(JobFinishResult::Applied)
        }
    }

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

    struct SensitiveFailure;

    impl JobExecutor<()> for SensitiveFailure {
        fn execute(
            &self,
            _job: &JobRecord,
            _cancellation: &CancellationToken,
            _context: &(),
        ) -> JobExecution {
            JobExecution::Failed(JobFailure::permanent(
                "provider failure: account=acct-sensitive-42 payload=secret-token",
            ))
        }
    }

    #[derive(Clone)]
    struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for CaptureWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn job(schedule: Schedule) -> JobRecord {
        JobRecord {
            id: Uuid::now_v7(),
            kind: "test".to_owned(),
            schedule,
            next_due_at: Some(
                DateTime::from_timestamp(1_700_000_000, 0).unwrap() - Duration::seconds(1),
            ),
            state: JobState::Queued,
            attempts: 0,
            max_attempts: 3,
            backoff: BackoffPolicy::default(),
            enabled: true,
            requires_explicit_opt_in: false,
            payload_json: None,
            last_run_at: None,
            last_outcome: None,
            last_error: None,
            last_unlock_window: None,
            cancel_requested: false,
        }
    }

    #[test]
    fn schedule_tokens_and_next_due_are_stable() {
        for (schedule, token) in [
            (Schedule::Once, "once"),
            (Schedule::Daily, "daily"),
            (Schedule::Weekly, "weekly"),
            (Schedule::Interval { seconds: 90 }, "interval:90"),
        ] {
            assert_eq!(Schedule::parse(token).unwrap(), schedule);
            assert_eq!(schedule.as_token(), token);
        }
        assert!(Schedule::parse("interval:0").is_err());
    }

    #[test]
    fn opt_in_jobs_are_fail_closed_until_the_consumer_allows_them() {
        let mut job = job(Schedule::Once);
        job.requires_explicit_opt_in = true;
        assert!(!Succeed.should_run(&job, &()));
    }

    struct TestHandler;

    impl JobHandler<()> for TestHandler {
        fn kind(&self) -> &'static str {
            "test"
        }

        fn execute(
            &self,
            _job: &JobRecord,
            _cancellation: &CancellationToken,
            _context: &(),
        ) -> JobExecution {
            JobExecution::Succeeded
        }
    }

    #[test]
    fn dispatcher_routes_registered_kinds_and_skips_unknown_jobs() {
        let dispatcher = JobDispatcher::new();
        assert!(!dispatcher.should_run(&job(Schedule::Once), &()));
        dispatcher.register(Arc::new(TestHandler)).unwrap();
        assert!(dispatcher.contains("test"));

        let registered = job(Schedule::Once);
        assert!(dispatcher.should_run(&registered, &()));
        assert_eq!(
            dispatcher.execute(&registered, &CancellationToken::new(), &()),
            JobExecution::Succeeded
        );

        let mut unknown = registered;
        unknown.kind = "other".to_owned();
        assert!(!dispatcher.should_run(&unknown, &()));
        assert_eq!(
            dispatcher.execute(&unknown, &CancellationToken::new(), &()),
            JobExecution::Skipped
        );
    }

    #[test]
    fn dispatcher_rejects_unsafe_handler_kinds() {
        struct UnsafeHandler;
        impl JobHandler<()> for UnsafeHandler {
            fn kind(&self) -> &'static str {
                "user supplied reason"
            }

            fn execute(
                &self,
                _job: &JobRecord,
                _cancellation: &CancellationToken,
                _context: &(),
            ) -> JobExecution {
                JobExecution::Succeeded
            }
        }

        let dispatcher = JobDispatcher::new();
        assert_eq!(
            dispatcher.register(Arc::new(UnsafeHandler)).unwrap_err(),
            JobSpecError::InvalidKind
        );
    }

    #[test]
    fn debug_output_redacts_payloads_and_failure_reasons() {
        let mut spec = JobSpec {
            id: Uuid::now_v7(),
            kind: "backup_export".to_owned(),
            schedule: Schedule::Once,
            next_due_at: DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
            max_attempts: 1,
            backoff: BackoffPolicy::default(),
            enabled: true,
            requires_explicit_opt_in: false,
            payload_json: Some("account-number-123456789".to_owned()),
        };
        let spec_debug = format!("{spec:?}");
        assert!(spec_debug.contains("[redacted]"));
        assert!(!spec_debug.contains("123456789"));

        let failure = JobFailure::permanent("provider token account-number-123456789");
        let failure_debug = format!("{failure:?}");
        assert!(failure_debug.contains("[redacted]"));
        assert!(!failure_debug.contains("123456789"));

        spec.payload_json = None;
        assert!(format!("{spec:?}").contains("payload_json: None"));
    }

    #[test]
    fn runner_tracing_omits_sensitive_payload_and_failure_text() {
        let store = FakeStore::default();
        let mut scheduled = job(Schedule::Once);
        scheduled.payload_json =
            Some("{\"account\":\"acct-sensitive-42\",\"token\":\"secret-token\"}".to_owned());
        let id = scheduled.id;
        store.jobs.lock().unwrap().insert(id, scheduled);

        let captured = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_span_events(FmtSpan::CLOSE)
            .with_writer({
                let captured = Arc::clone(&captured);
                move || CaptureWriter(Arc::clone(&captured))
            })
            .finish();
        let now = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let clock = FixedClock(now);
        let report = tracing::subscriber::with_default(subscriber, || {
            JobRunner::new(&store).run_due_with_clock(
                now,
                "unlock-sensitive",
                &(),
                &SensitiveFailure,
                &CancellationToken::new(),
                &clock,
            )
        })
        .unwrap();

        assert_eq!(report.failed, 1);
        let logged = String::from_utf8(captured.lock().unwrap().clone()).unwrap();
        assert!(logged.contains("job.invocation"));
        assert!(logged.contains("outcome=\"failed\""));
        assert!(!logged.contains("acct-sensitive-42"));
        assert!(!logged.contains("secret-token"));
        assert!(!logged.contains("provider failure"));
    }

    #[test]
    fn runner_recoveries_and_debounces_one_unlock_window() {
        let store = FakeStore::default();
        let first = job(Schedule::Once);
        let id = first.id;
        store.jobs.lock().unwrap().insert(id, first);
        let runner = JobRunner::new(&store);
        let now = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let clock = FixedClock(now);
        let report = runner
            .run_due_with_clock(
                now,
                "unlock-1",
                &(),
                &Succeed,
                &CancellationToken::new(),
                &clock,
            )
            .unwrap();
        assert_eq!(report.attempted, 1);
        assert_eq!(report.succeeded, 1);
        let second = runner
            .run_due_with_clock(
                now,
                "unlock-1",
                &(),
                &Succeed,
                &CancellationToken::new(),
                &clock,
            )
            .unwrap();
        assert_eq!(second.attempted, 0);
        assert_eq!(store.jobs.lock().unwrap()[&id].state, JobState::Succeeded);
    }

    #[test]
    fn cancellation_is_visible_to_a_handler_without_sleeping_for_the_full_sla() {
        let token = CancellationToken::new();
        let copy = token.clone();
        std::thread::spawn(move || {
            std::thread::sleep(StdDuration::from_millis(5));
            copy.cancel();
        });
        let started = Instant::now();
        assert!(!token.sleep(StdDuration::from_secs(5)));
        assert!(started.elapsed() < CANCELLATION_SLA);
    }
}
