//! Local diagnostic capture (personal-cfo-lyd).
//!
//! The contract is `docs/security/logging-policy.md` §5–§9 (merged in
//! `df9ea87`, personal-cfo-vkda) under ADR 0066-A §5:
//!
//! * **Session-only, bounded, in memory.** A [`Diagnostics`] handle belongs to one
//!   unlocked-vault session (the Finance Kernel owns it), holds at most
//!   [`CAPACITY`] fixed-size [`Record`]s, evicts oldest-first with per-metric
//!   dropped counters, and is never written anywhere automatically. Dropping the
//!   handle — vault lock, vault switch, app exit — drops everything in it,
//!   including a pending preview.
//! * **Admission is by type.** A [`Record`] holds a closed [`Metric`], a whole
//!   number of session seconds, and a closed [`Value`]. There is no string, path
//!   or message field anywhere, so a sensitive value has nowhere to go. A value
//!   of the wrong shape for its metric is rejected and counted, never stored.
//! * **One export: a previewed, redacted bundle.** [`Diagnostics::preview`]
//!   freezes an immutable [`Snapshot`] whose bytes are exactly what a save writes;
//!   records captured afterwards never enter it. The bytes also pass through
//!   [`crate::redact`] as defense in depth, and a bundle the redactor would change
//!   is refused rather than shown.
//! * **Honest completeness.** Every [`Bundle`] carries what it holds, what was
//!   dropped by capacity, what was rejected at admission, and a fixed statement
//!   of what it does not cover. [`parse_bundle`] round-trips it for tests and for
//!   `personal-cfo-ryjx`.
//!
//! There is deliberately no persistence API, no network code and no unredacted
//! mode in this module.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Most records one session holds (logging-policy §6.2).
pub const CAPACITY: usize = 2_000;
/// The ring's memory must stay under this (logging-policy §6.2).
pub const MEMORY_BUDGET_BYTES: usize = 256 * 1024;
/// Counts saturate here (logging-policy §7), so no large number — an account
/// number included — can be carried by a count.
pub const COUNT_CEILING: u32 = 1_000_000;
/// The bundle's `format` field.
pub const FORMAT: &str = "dohflow-diagnostics";
/// The bundle's `version` field.
pub const FORMAT_VERSION: u32 = 1;
/// The fixed statement of what a bundle does not cover (logging-policy §7).
pub const COVERAGE: &str = "Covers only the current unlocked session since its last unlock. \
Earlier sessions, other vaults and anything before a crash are not included. \
Oldest records are dropped once the session holds 2000.";

/// The metrics plan §6.6.1 allows (logging-policy §7).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    JobDuration,
    ImportRows,
    DedupeCandidates,
    ForecastDuration,
    QueryDuration,
    UiRenderTiming,
    ConnectorStatus,
    RetryCount,
    MigrationDuration,
    BackupOutcome,
    RestoreOutcome,
}

/// The shape of value a metric admits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    Duration,
    Count,
    Status,
    Outcome,
}

impl Metric {
    /// Every metric, in declaration order.
    pub const ALL: [Self; 11] = [
        Self::JobDuration,
        Self::ImportRows,
        Self::DedupeCandidates,
        Self::ForecastDuration,
        Self::QueryDuration,
        Self::UiRenderTiming,
        Self::ConnectorStatus,
        Self::RetryCount,
        Self::MigrationDuration,
        Self::BackupOutcome,
        Self::RestoreOutcome,
    ];

    const fn shape(self) -> Shape {
        match self {
            Self::JobDuration
            | Self::ForecastDuration
            | Self::QueryDuration
            | Self::UiRenderTiming
            | Self::MigrationDuration => Shape::Duration,
            Self::ImportRows | Self::DedupeCandidates | Self::RetryCount => Shape::Count,
            Self::ConnectorStatus => Shape::Status,
            Self::BackupOutcome | Self::RestoreOutcome => Shape::Outcome,
        }
    }
}

/// Coarse duration buckets (logging-policy §7).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DurationBucket {
    #[serde(rename = "<10ms")]
    Under10ms,
    #[serde(rename = "10-100ms")]
    From10To100ms,
    #[serde(rename = "100ms-1s")]
    From100msTo1s,
    #[serde(rename = "1-10s")]
    From1To10s,
    #[serde(rename = ">10s")]
    Over10s,
}

impl DurationBucket {
    /// The bucket a measured duration falls in.
    #[must_use]
    pub fn of(duration: Duration) -> Self {
        match duration.as_millis() {
            0..=9 => Self::Under10ms,
            10..=99 => Self::From10To100ms,
            100..=999 => Self::From100msTo1s,
            1_000..=9_999 => Self::From1To10s,
            _ => Self::Over10s,
        }
    }
}

/// Fixed connector status categories, mapped from typed errors — never a raw
/// HTTP status line, body, URL or message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorStatus {
    Ok,
    AuthFailed,
    RateLimited,
    ProviderError,
    NetworkUnavailable,
}

/// Fixed failure categories for an outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureCategory {
    Validation,
    Storage,
    Vault,
    PermissionDenied,
    DiskFull,
    Io,
    Unavailable,
    Internal,
}

/// How an operation ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Success,
    Failure(FailureCategory),
}

/// The value of one record: exactly one of four closed shapes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Value {
    Duration(DurationBucket),
    Count(u32),
    Status(ConnectorStatus),
    Outcome(Outcome),
}

impl Value {
    const fn shape(self) -> Shape {
        match self {
            Self::Duration(_) => Shape::Duration,
            Self::Count(_) => Shape::Count,
            Self::Status(_) => Shape::Status,
            Self::Outcome(_) => Shape::Outcome,
        }
    }
}

/// One captured record. Fixed-size and `Copy`; no field can hold text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    /// Whole seconds since this capture session began (no wall-clock time).
    pub at_s: u32,
    pub metric: Metric,
    pub value: Value,
}

/// App-controlled facts for the bundle header. Each is validated to a narrow
/// character set, so no caller can smuggle free text into a bundle through it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundleHeader {
    /// The app version, e.g. `0.2.0`.
    pub build_version: String,
    /// The build channel, e.g. `release`.
    pub build_channel: String,
    /// The OS family, e.g. `macos`.
    pub platform: String,
    /// The creation date, `YYYY-MM-DD` (UTC day, no time).
    pub created_on: String,
}

/// Why a preview could not be made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewError {
    /// A header field is outside its allowed character set.
    InvalidHeader,
    /// The redactor would change the bundle, so it holds something it must not.
    RedactionAltered,
}

/// An immutable, previewed bundle. Its bytes are exactly what a save writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    id: u64,
    bytes: Vec<u8>,
    records: usize,
}

impl Snapshot {
    /// Identifies this snapshot; a save names the snapshot it was shown.
    #[must_use]
    pub const fn id(&self) -> u64 {
        self.id
    }

    /// The exact bundle bytes (UTF-8 JSON).
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The exact bundle text.
    #[must_use]
    pub fn text(&self) -> &str {
        // Built from `serde_json::to_string_pretty`, always UTF-8.
        std::str::from_utf8(&self.bytes).unwrap_or_default()
    }

    /// How many records the bundle holds.
    #[must_use]
    pub const fn records(&self) -> usize {
        self.records
    }
}

/// The saved bundle's structure (logging-policy §7).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bundle {
    pub format: String,
    pub version: u32,
    pub created_on: String,
    pub build: BuildInfo,
    pub platform: String,
    pub coverage: String,
    pub records_retained: u32,
    pub dropped_by_capacity: BTreeMap<Metric, u64>,
    pub rejected_at_admission: u64,
    pub records: Vec<Record>,
}

/// Build identity in the bundle header.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildInfo {
    pub version: String,
    pub channel: String,
}

/// Why a bundle did not parse.
#[derive(Debug, PartialEq, Eq)]
pub enum ParseError {
    /// Not JSON of the bundle's shape, or an unknown field.
    Malformed(String),
    /// Not a DohFlow diagnostics bundle of a supported version.
    UnsupportedFormat,
    /// `records_retained` disagrees with the records present, or exceeds
    /// [`CAPACITY`].
    Inconsistent,
}

struct Inner {
    started: Instant,
    records: VecDeque<Record>,
    dropped: BTreeMap<Metric, u64>,
    rejected: u64,
    pending: Option<Snapshot>,
    next_snapshot_id: u64,
}

/// One unlocked session's capture. Cheap to clone (shared handle); dropping the
/// last clone drops every record and any pending preview.
#[derive(Clone)]
pub struct Diagnostics {
    inner: Arc<Mutex<Inner>>,
}

impl std::fmt::Debug for Diagnostics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Counts only; the records themselves are for the previewed bundle.
        let inner = self.lock();
        f.debug_struct("Diagnostics")
            .field("records", &inner.records.len())
            .field("rejected", &inner.rejected)
            .field("pending_preview", &inner.pending.is_some())
            .finish()
    }
}

impl Default for Diagnostics {
    fn default() -> Self {
        Self::new()
    }
}

impl Diagnostics {
    /// Start a new, empty session.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                started: Instant::now(),
                records: VecDeque::with_capacity(CAPACITY),
                dropped: BTreeMap::new(),
                rejected: 0,
                pending: None,
                next_snapshot_id: 1,
            })),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A poisoned lock only means a panic elsewhere mid-push; the ring's
        // invariants (bounded length, counters) hold at every await-free step.
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Admit one value for `metric`. A value of the wrong shape is rejected and
    /// counted, never stored.
    pub fn record(&self, metric: Metric, value: Value) {
        let mut inner = self.lock();
        if metric.shape() != value.shape() {
            inner.rejected = inner.rejected.saturating_add(1);
            return;
        }
        let value = match value {
            Value::Count(n) => Value::Count(n.min(COUNT_CEILING)),
            other => other,
        };
        let at_s = u32::try_from(inner.started.elapsed().as_secs()).unwrap_or(u32::MAX);
        if inner.records.len() >= CAPACITY {
            if let Some(evicted) = inner.records.pop_front() {
                *inner.dropped.entry(evicted.metric).or_default() += 1;
            }
        }
        inner.records.push_back(Record {
            at_s,
            metric,
            value,
        });
    }

    /// Admit a measured duration.
    pub fn record_duration(&self, metric: Metric, duration: Duration) {
        self.record(metric, Value::Duration(DurationBucket::of(duration)));
    }

    /// Admit a count (saturating at [`COUNT_CEILING`]).
    pub fn record_count(&self, metric: Metric, count: u64) {
        self.record(
            metric,
            Value::Count(u32::try_from(count).unwrap_or(u32::MAX)),
        );
    }

    /// Admit an outcome.
    pub fn record_outcome(&self, metric: Metric, outcome: Outcome) {
        self.record(metric, Value::Outcome(outcome));
    }

    /// Records currently held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.lock().records.len()
    }

    /// Whether nothing is held.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Freeze an immutable snapshot of everything held now, render it to the
    /// exact bundle bytes, and keep it as the pending preview (replacing any
    /// earlier one). Records captured afterwards never enter this snapshot.
    ///
    /// # Errors
    /// [`PreviewError::InvalidHeader`] if a header field is outside its
    /// character set; [`PreviewError::RedactionAltered`] if the redactor would
    /// change the bundle.
    pub fn preview(&self, header: &BundleHeader) -> Result<Snapshot, PreviewError> {
        validate_header(header)?;
        let mut inner = self.lock();
        let bundle = Bundle {
            format: FORMAT.to_owned(),
            version: FORMAT_VERSION,
            created_on: header.created_on.clone(),
            build: BuildInfo {
                version: header.build_version.clone(),
                channel: header.build_channel.clone(),
            },
            platform: header.platform.clone(),
            coverage: COVERAGE.to_owned(),
            records_retained: u32::try_from(inner.records.len()).unwrap_or(u32::MAX),
            dropped_by_capacity: inner.dropped.clone(),
            rejected_at_admission: inner.rejected,
            records: inner.records.iter().copied().collect(),
        };
        let mut text =
            serde_json::to_string_pretty(&bundle).map_err(|_| PreviewError::RedactionAltered)?;
        text.push('\n');
        // Defense in depth: typed records cannot carry a secret, so the redactor
        // must be a no-op. If it is not, something is wrong — refuse to show it.
        if crate::redact(&text) != text {
            return Err(PreviewError::RedactionAltered);
        }
        let snapshot = Snapshot {
            id: inner.next_snapshot_id,
            bytes: text.into_bytes(),
            records: bundle.records.len(),
        };
        inner.next_snapshot_id += 1;
        inner.pending = Some(snapshot.clone());
        Ok(snapshot)
    }

    /// The pending preview, if it is snapshot `id`. The caller writes exactly
    /// these bytes; nothing is rebuilt at save time.
    #[must_use]
    pub fn pending(&self, id: u64) -> Option<Snapshot> {
        self.lock().pending.clone().filter(|s| s.id == id)
    }

    /// Forget the pending preview if it is snapshot `id` (cancel or close).
    pub fn discard(&self, id: u64) {
        let mut inner = self.lock();
        if inner.pending.as_ref().is_some_and(|s| s.id == id) {
            inner.pending = None;
        }
    }
}

fn validate_header(header: &BundleHeader) -> Result<(), PreviewError> {
    let token = |s: &str, max: usize| {
        !s.is_empty()
            && s.len() <= max
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'))
    };
    let date = header.created_on.len() == 10
        && header.created_on.char_indices().all(|(i, c)| {
            if i == 4 || i == 7 {
                c == '-'
            } else {
                c.is_ascii_digit()
            }
        });
    if token(&header.build_version, 32)
        && token(&header.build_channel, 16)
        && token(&header.platform, 16)
        && date
    {
        Ok(())
    } else {
        Err(PreviewError::InvalidHeader)
    }
}

/// Parse a saved bundle, checking its format, version and internal
/// consistency. Unknown fields are rejected.
///
/// # Errors
/// See [`ParseError`].
pub fn parse_bundle(bytes: &[u8]) -> Result<Bundle, ParseError> {
    let bundle: Bundle =
        serde_json::from_slice(bytes).map_err(|e| ParseError::Malformed(e.to_string()))?;
    if bundle.format != FORMAT || bundle.version != FORMAT_VERSION || bundle.coverage != COVERAGE {
        return Err(ParseError::UnsupportedFormat);
    }
    if bundle.records.len() > CAPACITY
        || usize::try_from(bundle.records_retained).ok() != Some(bundle.records.len())
    {
        return Err(ParseError::Inconsistent);
    }
    Ok(bundle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header() -> BundleHeader {
        BundleHeader {
            build_version: "0.2.0".into(),
            build_channel: "release".into(),
            platform: "macos".into(),
            created_on: "2026-09-28".into(),
        }
    }

    #[test]
    fn a_record_is_fixed_size_and_the_full_ring_fits_the_memory_budget() {
        let per_record = std::mem::size_of::<Record>();
        assert!(per_record <= 16, "Record grew to {per_record} bytes");
        assert!(
            per_record * CAPACITY < MEMORY_BUDGET_BYTES,
            "{CAPACITY} records × {per_record} B exceeds {MEMORY_BUDGET_BYTES} B"
        );
    }

    #[test]
    fn capacity_is_enforced_oldest_first_with_per_metric_drop_counters() {
        let d = Diagnostics::new();
        for _ in 0..CAPACITY {
            d.record_duration(Metric::JobDuration, Duration::from_millis(5));
        }
        for _ in 0..3 {
            d.record_count(Metric::ImportRows, 7);
        }
        assert_eq!(d.len(), CAPACITY);
        let bundle = parse_bundle(d.preview(&header()).expect("preview").bytes()).expect("parse");
        assert_eq!(bundle.records.len(), CAPACITY);
        assert_eq!(
            bundle.dropped_by_capacity.get(&Metric::JobDuration),
            Some(&3)
        );
        // The newest three are the counts; the oldest three durations were evicted.
        assert!(bundle.records[CAPACITY - 3..]
            .iter()
            .all(|r| r.metric == Metric::ImportRows));
    }

    #[test]
    fn a_value_of_the_wrong_shape_is_rejected_and_counted_not_stored() {
        let d = Diagnostics::new();
        d.record(Metric::JobDuration, Value::Count(5));
        d.record(
            Metric::BackupOutcome,
            Value::Duration(DurationBucket::Over10s),
        );
        assert!(d.is_empty());
        let bundle = parse_bundle(d.preview(&header()).expect("preview").bytes()).expect("parse");
        assert_eq!(bundle.rejected_at_admission, 2);
        assert!(bundle.records.is_empty());
    }

    #[test]
    fn counts_saturate_so_no_large_number_can_be_carried() {
        let d = Diagnostics::new();
        // An identifier-sized value handed in as a "count".
        d.record_count(Metric::ImportRows, 98_765_432_109);
        d.record_count(Metric::RetryCount, 3);
        let snapshot = d.preview(&header()).expect("preview");
        assert!(!snapshot.text().contains("98765432109"));
        let bundle = parse_bundle(snapshot.bytes()).expect("parse");
        assert_eq!(bundle.records[0].value, Value::Count(COUNT_CEILING));
        assert_eq!(bundle.records[1].value, Value::Count(3));
    }

    #[test]
    fn the_snapshot_is_immutable_and_later_records_never_enter_it() {
        let d = Diagnostics::new();
        d.record_outcome(Metric::BackupOutcome, Outcome::Success);
        let snapshot = d.preview(&header()).expect("preview");
        d.record_outcome(
            Metric::BackupOutcome,
            Outcome::Failure(FailureCategory::DiskFull),
        );
        let pending = d.pending(snapshot.id()).expect("still pending");
        assert_eq!(
            pending, snapshot,
            "the pending bytes are exactly the previewed bytes"
        );
        assert_eq!(
            parse_bundle(pending.bytes()).expect("parse").records.len(),
            1
        );
    }

    #[test]
    fn a_new_preview_replaces_the_old_and_discard_forgets_it() {
        let d = Diagnostics::new();
        let first = d.preview(&header()).expect("first");
        let second = d.preview(&header()).expect("second");
        assert!(
            d.pending(first.id()).is_none(),
            "an older snapshot can no longer be saved"
        );
        assert!(d.pending(second.id()).is_some());
        d.discard(first.id()); // not pending: no effect
        assert!(d.pending(second.id()).is_some());
        d.discard(second.id());
        assert!(d.pending(second.id()).is_none());
    }

    #[test]
    fn dropping_the_session_drops_records_and_the_pending_preview() {
        // What vault lock / switch / exit do: the owning kernel drops its handle.
        let d = Diagnostics::new();
        d.record_duration(Metric::JobDuration, Duration::from_secs(2));
        let weak = Arc::downgrade(&d.inner);
        let _ = d.preview(&header()).expect("preview");
        drop(d);
        assert!(
            weak.upgrade().is_none(),
            "nothing outlives the session handle"
        );
        // And a new session starts empty.
        assert!(Diagnostics::new().is_empty());
    }

    #[test]
    fn the_bundle_states_its_limits_and_round_trips() {
        let d = Diagnostics::new();
        d.record_duration(Metric::MigrationDuration, Duration::from_millis(250));
        d.record(
            Metric::ConnectorStatus,
            Value::Status(ConnectorStatus::RateLimited),
        );
        d.record_outcome(
            Metric::RestoreOutcome,
            Outcome::Failure(FailureCategory::Vault),
        );
        let snapshot = d.preview(&header()).expect("preview");
        let bundle = parse_bundle(snapshot.bytes()).expect("parse");
        assert_eq!(bundle.format, FORMAT);
        assert_eq!(bundle.coverage, COVERAGE);
        assert_eq!(bundle.records_retained, 3);
        assert_eq!(bundle.build.version, "0.2.0");
        assert_eq!(
            bundle.records.iter().map(|r| r.value).collect::<Vec<_>>(),
            vec![
                Value::Duration(DurationBucket::From100msTo1s),
                Value::Status(ConnectorStatus::RateLimited),
                Value::Outcome(Outcome::Failure(FailureCategory::Vault)),
            ]
        );
        assert_eq!(snapshot.records(), 3);
    }

    #[test]
    fn header_fields_cannot_smuggle_free_text() {
        let d = Diagnostics::new();
        for bad in [
            BundleHeader {
                build_version: "0.2.0 password=SENTINEL-PASS".into(),
                ..header()
            },
            BundleHeader {
                platform: "/Users/jane/Library".into(),
                ..header()
            },
            BundleHeader {
                created_on: "2026-09-28T10:00".into(),
                ..header()
            },
            BundleHeader {
                build_channel: String::new(),
                ..header()
            },
        ] {
            assert_eq!(d.preview(&bad), Err(PreviewError::InvalidHeader), "{bad:?}");
        }
    }

    #[test]
    fn parse_rejects_unknown_fields_other_formats_and_inconsistency() {
        let d = Diagnostics::new();
        d.record_count(Metric::RetryCount, 1);
        let text = d.preview(&header()).expect("preview").text().to_owned();

        let smuggled = text.replacen(
            "\"platform\"",
            "\"note\": \"acct 4111\",\n  \"platform\"",
            1,
        );
        assert!(matches!(
            parse_bundle(smuggled.as_bytes()),
            Err(ParseError::Malformed(_))
        ));

        let record_field = text.replacen("\"at_s\"", "\"message\": \"x\",\n      \"at_s\"", 1);
        assert!(matches!(
            parse_bundle(record_field.as_bytes()),
            Err(ParseError::Malformed(_))
        ));

        let other = text.replacen(FORMAT, "someone-else", 1);
        assert_eq!(
            parse_bundle(other.as_bytes()),
            Err(ParseError::UnsupportedFormat)
        );

        let lying = text.replacen("\"records_retained\": 1", "\"records_retained\": 5", 1);
        assert_eq!(
            parse_bundle(lying.as_bytes()),
            Err(ParseError::Inconsistent)
        );
    }

    #[test]
    fn duration_buckets_cover_the_boundaries() {
        use DurationBucket::*;
        for (ms, bucket) in [
            (0, Under10ms),
            (9, Under10ms),
            (10, From10To100ms),
            (99, From10To100ms),
            (100, From100msTo1s),
            (999, From100msTo1s),
            (1_000, From1To10s),
            (9_999, From1To10s),
            (10_000, Over10s),
        ] {
            assert_eq!(
                DurationBucket::of(Duration::from_millis(ms)),
                bucket,
                "{ms} ms"
            );
        }
    }

    #[test]
    fn every_metric_admits_exactly_one_shape() {
        let samples = [
            Value::Duration(DurationBucket::Under10ms),
            Value::Count(1),
            Value::Status(ConnectorStatus::Ok),
            Value::Outcome(Outcome::Success),
        ];
        for metric in Metric::ALL {
            let admitted = samples
                .iter()
                .filter(|value| {
                    let d = Diagnostics::new();
                    d.record(metric, **value);
                    d.len() == 1
                })
                .count();
            assert_eq!(admitted, 1, "{metric:?}");
        }
    }
}
