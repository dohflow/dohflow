//! Bounded in-process parser host (personal-cfo-hs9, ADR 0022 §5).
//!
//! R2 structured importers (CSV/OFX/QFX/QIF) have a small attack surface, so they
//! parse **in-process** behind hardened bounds rather than an OS subprocess: the
//! eay trait already keeps a plugin away from keys/DB/network/IPC, and this host
//! adds the resource limits + crash isolation around [`ImporterPlugin::parse`].
//! A hostile file therefore fails *cleanly* (a typed [`ParseError`]) instead of
//! hanging, OOMing, or crashing the app. Heavy, killable OS-subprocess / WASM
//! sandboxing is reserved for the documents/PDF tier (personal-cfo-h0il / -nqii),
//! where the attack surface is large.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crate::{ImporterPlugin, ParseError, ParsedBatch, ParserHints, ParserInput};

/// Resource limits enforced around a single parse (ADR 0022 §5).
#[derive(Debug, Clone)]
pub struct ParserLimits {
    /// Reject inputs larger than this (bytes) *before* parsing.
    pub max_bytes: usize,
    /// Reject a parse that produces more than this many records.
    pub max_records: usize,
    /// Best-effort wall-clock budget for a single parse (see [`run_bounded`]).
    pub timeout: Duration,
}

impl Default for ParserLimits {
    /// Conservative defaults for R2 structured importers: 64 MiB, 1,000,000
    /// records, 30 seconds.
    fn default() -> Self {
        Self {
            max_bytes: 64 * 1024 * 1024,
            max_records: 1_000_000,
            timeout: Duration::from_secs(30),
        }
    }
}

/// The outcome of a bounded run — maps onto `parser_runs.status` (personal-cfo-3bb).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    /// The parse completed within every limit.
    Ok,
    /// A size or record limit was exceeded; the parse failed cleanly.
    LimitExceeded,
    /// The parser returned an error, or panicked (contained — the app survived).
    ParseError,
    /// The parse exceeded its wall-clock budget.
    Timeout,
}

impl RunStatus {
    /// The `parser_runs.status` token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            RunStatus::Ok => "ok",
            RunStatus::LimitExceeded => "limit_exceeded",
            RunStatus::ParseError => "parse_error",
            RunStatus::Timeout => "timeout",
        }
    }
}

/// A record of one bounded parse — the `parser_run` provenance. The pipeline
/// (personal-cfo-cmx) persists it via the `record_parser_run` staging primitive;
/// the fields map 1:1 onto a `parser_runs` row.
#[derive(Debug, Clone)]
pub struct ParserRunReport {
    /// The plugin's stable id.
    pub plugin_id: &'static str,
    /// The plugin's version (`semver`), as a string.
    pub plugin_version: String,
    /// The run outcome.
    pub status: RunStatus,
    /// Bytes handed to the parser.
    pub bytes_in: usize,
    /// Records the parser produced (0 on failure).
    pub records_out: usize,
    /// Which bound was hit (`max_bytes` / `max_records` / `timeout`), if any.
    pub limit_hit: Option<String>,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u128,
}

/// Run `plugin.parse` under `limits`, isolating panics and bounding resources.
/// Returns the parsed batch (on success) plus a [`ParserRunReport`] for the
/// `parser_run` row.
///
/// Isolation (ADR 0022 §5, R2 structured importers):
/// - **Size** is checked before parsing; an oversize input fails immediately.
/// - **Panics** are caught ([`catch_unwind`]) so a hostile file cannot crash the
///   app — they surface as a `parse_error`.
/// - **Records** are capped after parsing.
/// - **Timeout** is *best-effort*: the parse runs on a worker thread and the
///   caller gets [`RunStatus::Timeout`] promptly, but a runaway parse that
///   ignores the budget leaks that thread until it finishes. A hard, killable
///   timeout needs an OS subprocess, which the documents/PDF tier graduates to
///   (personal-cfo-h0il / -nqii). The size cap keeps R2's bounded grammars well
///   inside this.
pub fn run_bounded(
    plugin: &'static dyn ImporterPlugin,
    input: ParserInput,
    hints: ParserHints,
    limits: &ParserLimits,
) -> (Result<ParsedBatch, ParseError>, ParserRunReport) {
    let plugin_id = plugin.id();
    let plugin_version = plugin.version().to_string();
    let bytes_in = input.bytes.len();
    let start = Instant::now();

    let report = |status: RunStatus, records_out: usize, limit_hit: Option<&str>| ParserRunReport {
        plugin_id,
        plugin_version: plugin_version.clone(),
        status,
        bytes_in,
        records_out,
        limit_hit: limit_hit.map(str::to_owned),
        duration_ms: start.elapsed().as_millis(),
    };

    if bytes_in > limits.max_bytes {
        let err = ParseError::LimitExceeded(format!(
            "input is {bytes_in} bytes, over the {} byte limit",
            limits.max_bytes
        ));
        return (
            Err(err),
            report(RunStatus::LimitExceeded, 0, Some("max_bytes")),
        );
    }

    // Parse on a worker thread so a slow parse can time out (best-effort) and a
    // panic is contained. The plugin is `&'static` and the input/hints are owned,
    // so they move into the thread cleanly.
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let outcome = catch_unwind(AssertUnwindSafe(|| plugin.parse(&input, &hints)));
        // The receiver may already be gone (the run timed out) — ignore that.
        let _ = tx.send(outcome);
    });

    match rx.recv_timeout(limits.timeout) {
        Ok(Ok(Ok(batch))) => {
            let records_out = batch.records.len();
            if records_out > limits.max_records {
                let err = ParseError::LimitExceeded(format!(
                    "parse produced {records_out} records, over the {} limit",
                    limits.max_records
                ));
                return (
                    Err(err),
                    report(RunStatus::LimitExceeded, records_out, Some("max_records")),
                );
            }
            (Ok(batch), report(RunStatus::Ok, records_out, None))
        }
        Ok(Ok(Err(err))) => (Err(err), report(RunStatus::ParseError, 0, None)),
        Ok(Err(_panic)) => (
            Err(ParseError::Malformed("parser panicked".to_owned())),
            report(RunStatus::ParseError, 0, None),
        ),
        Err(mpsc::RecvTimeoutError::Timeout) => (
            Err(ParseError::LimitExceeded(format!(
                "parse exceeded the {:?} budget",
                limits.timeout
            ))),
            report(RunStatus::Timeout, 0, Some("timeout")),
        ),
        Err(mpsc::RecvTimeoutError::Disconnected) => (
            Err(ParseError::Malformed(
                "parser worker stopped unexpectedly".to_owned(),
            )),
            report(RunStatus::ParseError, 0, None),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ParsedBatch, ParsedRecord};
    use semver::Version;

    fn rec() -> ParsedRecord {
        ParsedRecord {
            external_id: None,
            source_hash: "h".to_owned(),
            normalized_json: "{}".to_owned(),
            parse_confidence_bps: None,
            transaction: None,
            balance: None,
        }
    }

    fn empty_batch(records: Vec<ParsedRecord>) -> ParsedBatch {
        ParsedBatch {
            source_format: "csv".to_owned(),
            accounts: vec![],
            records,
            warnings: vec![],
        }
    }

    // Unit-struct plugins → `&Plugin` is a `&'static` reference.
    struct TwoRows;
    impl ImporterPlugin for TwoRows {
        fn id(&self) -> &'static str {
            "two-rows"
        }
        fn display_name(&self) -> &'static str {
            "Two Rows"
        }
        fn version(&self) -> Version {
            Version::new(1, 0, 0)
        }
        fn supported_extensions(&self) -> &'static [&'static str] {
            &["csv"]
        }
        fn detect_confidence(&self, _: &ParserInput) -> u16 {
            0
        }
        fn parse(&self, _: &ParserInput, _: &ParserHints) -> Result<ParsedBatch, ParseError> {
            Ok(empty_batch(vec![rec(), rec()]))
        }
    }

    struct Panics;
    impl ImporterPlugin for Panics {
        fn id(&self) -> &'static str {
            "panics"
        }
        fn display_name(&self) -> &'static str {
            "Panics"
        }
        fn version(&self) -> Version {
            Version::new(1, 0, 0)
        }
        fn supported_extensions(&self) -> &'static [&'static str] {
            &[]
        }
        fn detect_confidence(&self, _: &ParserInput) -> u16 {
            0
        }
        fn parse(&self, _: &ParserInput, _: &ParserHints) -> Result<ParsedBatch, ParseError> {
            panic!("hostile input")
        }
    }

    struct Slow;
    impl ImporterPlugin for Slow {
        fn id(&self) -> &'static str {
            "slow"
        }
        fn display_name(&self) -> &'static str {
            "Slow"
        }
        fn version(&self) -> Version {
            Version::new(1, 0, 0)
        }
        fn supported_extensions(&self) -> &'static [&'static str] {
            &[]
        }
        fn detect_confidence(&self, _: &ParserInput) -> u16 {
            0
        }
        fn parse(&self, _: &ParserInput, _: &ParserHints) -> Result<ParsedBatch, ParseError> {
            // Outlasts the test's tiny timeout; the leaked thread then exits.
            thread::sleep(Duration::from_secs(2));
            Ok(empty_batch(vec![]))
        }
    }

    #[test]
    fn ok_run_reports_records_and_ok_status() {
        let (res, report) = run_bounded(
            &TwoRows,
            ParserInput::new(b"a,b\n1,2\n".to_vec()),
            ParserHints::default(),
            &ParserLimits::default(),
        );
        assert!(res.is_ok());
        assert_eq!(report.status, RunStatus::Ok);
        assert_eq!(report.records_out, 2);
        assert_eq!(report.plugin_id, "two-rows");
        assert_eq!(report.plugin_version, "1.0.0");
        assert!(report.limit_hit.is_none());
    }

    #[test]
    fn oversize_input_is_rejected_before_parsing() {
        let limits = ParserLimits {
            max_bytes: 4,
            ..ParserLimits::default()
        };
        let (res, report) = run_bounded(
            &TwoRows,
            ParserInput::new(b"abcdef".to_vec()),
            ParserHints::default(),
            &limits,
        );
        assert!(matches!(res, Err(ParseError::LimitExceeded(_))));
        assert_eq!(report.status, RunStatus::LimitExceeded);
        assert_eq!(report.limit_hit.as_deref(), Some("max_bytes"));
        assert_eq!(report.records_out, 0);
    }

    #[test]
    fn a_panicking_parser_is_contained() {
        // The panic happens on the worker thread and is caught — this test
        // process (the "app") survives and gets a clean error + report.
        let (res, report) = run_bounded(
            &Panics,
            ParserInput::new(vec![]),
            ParserHints::default(),
            &ParserLimits::default(),
        );
        assert!(matches!(res, Err(ParseError::Malformed(_))));
        assert_eq!(report.status, RunStatus::ParseError);
    }

    #[test]
    fn too_many_records_exceeds_the_limit() {
        let limits = ParserLimits {
            max_records: 1,
            ..ParserLimits::default()
        };
        let (res, report) = run_bounded(
            &TwoRows,
            ParserInput::new(vec![]),
            ParserHints::default(),
            &limits,
        );
        assert!(matches!(res, Err(ParseError::LimitExceeded(_))));
        assert_eq!(report.status, RunStatus::LimitExceeded);
        assert_eq!(report.limit_hit.as_deref(), Some("max_records"));
        assert_eq!(report.records_out, 2);
    }

    #[test]
    fn a_slow_parser_times_out() {
        let limits = ParserLimits {
            timeout: Duration::from_millis(50),
            ..ParserLimits::default()
        };
        let (res, report) = run_bounded(
            &Slow,
            ParserInput::new(vec![]),
            ParserHints::default(),
            &limits,
        );
        assert!(matches!(res, Err(ParseError::LimitExceeded(_))));
        assert_eq!(report.status, RunStatus::Timeout);
        assert_eq!(report.limit_hit.as_deref(), Some("timeout"));
    }
}
