//! Adversarial round trip of the one diagnostic export (personal-cfo-ryjx).
//!
//! Contract: `docs/security/logging-policy.md` §6–§8 under ADR 0066-A §5. The
//! export is a user-initiated, previewed, redacted bundle; there is no second
//! exporter and no unredacted mode, so this suite exercises exactly the
//! production path the Settings card uses:
//!
//! capture (real kernel sources and `Kernel::diagnostics` admission)
//!   → redaction/allowlist (`Diagnostics::preview`, typed records + `redact()`)
//!   → preview (`diagnostics_preview_impl`)
//!   → packaging (`diagnostics_save_impl` writing the pending snapshot)
//!   → parse (`parse_bundle`, plus an independent JSON walk).
//!
//! What it proves, and what it does not:
//! * A seeded corpus of synthetic sensitive values — account numbers,
//!   descriptions, balances, credentials, a provider body, paths and raw error
//!   text — is pushed through real vault operations, and none of it reaches the
//!   preview, the saved file, the preview DTO or any other file. That is a
//!   check against a **finite corpus**, not a claim of universal secret
//!   detection. The stronger guarantee is the allowlist walk: every key and
//!   every string in a bundle must come from the closed sets of §7, written out
//!   here independently of the Rust enums, so widening an enum without changing
//!   the policy fails this suite.
//! * The retained records equal the approved `insta` snapshots, and eviction,
//!   admission rejections and post-preview records are accounted for exactly.
//! * Empty sessions, unknown and duplicate fields, Unicode and escaping,
//!   truncation boundaries, cancel, write failure and vault lock/switch leave
//!   no diagnostic artifact behind and reveal nothing.
//!
//! Every operation is local: temp folders, a synthetic SQLCipher vault and the
//! in-memory mock connector. No test here opens a socket.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use app_lib::ipc::commands::{
    account_list_impl, connector_link_impl, create_account_impl, create_vault_impl,
    create_vault_named_impl, diagnostics_discard_impl, diagnostics_preview_impl,
    diagnostics_save_impl, export_backup_impl, import_batch_impl, lock_vault_impl,
    run_due_jobs_on_unlock_impl, unlock_vault_impl,
};
use app_lib::ipc::dto::{
    AccountFlagsDto, CashflowRoleDto, ConnectorLinkInput, CreateAccountInput,
    DiagnosticsPreviewDto, DiagnosticsSaveResult, ImportBatchInput, MoneyDto,
};
use app_lib::ipc::IpcError;
use app_lib::vault_registry::VaultRegistry;
use app_lib::AppState;
use connector_core::mock::MockConnector;
use finance_kernel::{
    BackoffPolicy, CancellationToken, JobExecution, JobFailure, JobHandler, JobRecord, JobSpec,
    Kernel, Schedule, VaultController,
};
use observability::diagnostics::{
    parse_bundle, BundleHeader, ConnectorStatus, Diagnostics, DurationBucket, FailureCategory,
    Metric, Outcome, ParseError, PreviewError, Value, CAPACITY, COUNT_CEILING, COVERAGE, FORMAT,
};
use serde_json::{json, Value as Json};
use tempfile::TempDir;
use uuid::Uuid;

// ---- the seeded corpus ---------------------------------------------------------
//
// Synthetic only. Each value is fed into a real input below; none may appear in
// anything the export produces.

/// The vault password itself.
const PASSWORD: &str = "synthetic SENTINEL-PASS-7731 password";
/// A synthetic 12-digit account number (deliberately not card-shaped, so the
/// repository's own value scan has nothing to flag).
const ACCOUNT_NUMBER: &str = "551200349871";
const ACCOUNT_NAME: &str = "Checking acct 551200349871";
/// Opening balance in minor units, and its rendered forms.
const OPENING_BALANCE_MINOR: i64 = 9_876_543;
const DESCRIPTION: &str = "SENTINEL-MERCHANT Café Zoë";
const CSV_AMOUNT: &str = "-1234.56";
const SETUP_TOKEN: &str = "SENTINEL-SETUP-TOKEN-5521";
const PROVIDER_BODY: &str = r#"{"error":"invalid_grant","error_description":"SENTINEL-PROVIDER-BODY","access_token":"SENTINEL-ACCESS-TOKEN"}"#;
const RAW_ERROR: &str = "SENTINEL-RAW-ERROR: ENOENT open /Users/jane.doe/Private/ledger.db";
const JOB_PAYLOAD: &str = "SENTINEL-JOB-PAYLOAD account=551200349871";
const JOB_KIND_OK: &str = "ryjx_sentinel_ok";
const JOB_KIND_FAIL: &str = "ryjx_sentinel_fail";
/// A folder name that is an email, an account sentinel and non-ASCII text.
const HOSTILE_FOLDER: &str = "jane.doe@example.com ACCT-SENTINEL-771 Zoë";
/// The folder a user saves into: spaces, quotes and Unicode.
const SAVE_FOLDER: &str = "Private \"Folder\" ✓ jane";

/// Fragments that must be absent from every byte the export produces. Each is
/// a distinctive piece of a seeded value (or a seeded value whole).
const FORBIDDEN: &[&str] = &[
    "SENTINEL",
    ACCOUNT_NUMBER,
    "Checking",
    "9876543",
    "98765.43",
    "98,765.43",
    "1234.56",
    "123456",
    "Café",
    "Zoë",
    "Zo\\u00eb",
    "invalid_grant",
    "setup_token_used",
    "claimed",
    "error_description",
    "access_token",
    "ENOENT",
    "ledger.db",
    "jane",
    "example.com",
    "Private",
    "Folder",
    "pcfobk",
    "vault.db",
    JOB_KIND_OK,
    JOB_KIND_FAIL,
    "mock",
    "/Users",
    "/tmp",
    "/var",
    "/private",
];

// ---- the independent allowlist (logging-policy §7) ----------------------------
//
// Written out by hand on purpose: these are the policy's closed sets, not a
// reflection of the Rust enums. A new metric, status, category or field must
// change §7 and this list together.

const BUNDLE_KEYS: &[&str] = &[
    "format",
    "version",
    "created_on",
    "build",
    "platform",
    "coverage",
    "records_retained",
    "dropped_by_capacity",
    "rejected_at_admission",
    "records",
];
const BUILD_KEYS: &[&str] = &["version", "channel"];
const RECORD_KEYS: &[&str] = &["at_s", "metric", "value"];
const METRICS: &[&str] = &[
    "job_duration",
    "import_rows",
    "dedupe_candidates",
    "forecast_duration",
    "query_duration",
    "ui_render_timing",
    "connector_status",
    "retry_count",
    "migration_duration",
    "backup_outcome",
    "restore_outcome",
];
const DURATION_METRICS: &[&str] = &[
    "job_duration",
    "forecast_duration",
    "query_duration",
    "ui_render_timing",
    "migration_duration",
];
const COUNT_METRICS: &[&str] = &["import_rows", "dedupe_candidates", "retry_count"];
const STATUS_METRICS: &[&str] = &["connector_status"];
const OUTCOME_METRICS: &[&str] = &["backup_outcome", "restore_outcome"];
const DURATION_BUCKETS: &[&str] = &["<10ms", "10-100ms", "100ms-1s", "1-10s", ">10s"];
const CONNECTOR_STATUSES: &[&str] = &[
    "ok",
    "auth_failed",
    "rate_limited",
    "provider_error",
    "network_unavailable",
];
const FAILURE_CATEGORIES: &[&str] = &[
    "validation",
    "storage",
    "vault",
    "permission_denied",
    "disk_full",
    "io",
    "unavailable",
    "internal",
];
const SAVE_RESULTS: &[&str] = &[
    "saved",
    "preview_expired",
    "invalid_destination",
    "permission_denied",
    "disk_full",
    "failed",
];

/// The header values the app is approved to state (§7 "Honest completeness").
struct ApprovedHeader {
    dates: Vec<String>,
    max_at_s: u64,
}

impl ApprovedHeader {
    /// Valid for a bundle previewed between `started` and now.
    fn since(started: Instant) -> Self {
        Self {
            dates: today_utc(),
            max_at_s: started.elapsed().as_secs() + 1,
        }
    }
}

/// Today's UTC date, and tomorrow's in case the test straddles midnight.
fn today_utc() -> Vec<String> {
    let now = chrono::Utc::now();
    vec![
        now.format("%Y-%m-%d").to_string(),
        (now + chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string(),
    ]
}

/// A named edit that inserts forbidden content into a bundle.
type Mutation = (&'static str, Box<dyn Fn(&mut Json)>);

fn object_keys(value: &Json) -> Option<BTreeSet<&str>> {
    value
        .as_object()
        .map(|o| o.keys().map(String::as_str).collect())
}

fn exactly<'a>(keys: &[&'a str]) -> BTreeSet<&'a str> {
    keys.iter().copied().collect()
}

/// Every way `bundle` departs from the §7 allowlist. Empty means allowed.
fn allowlist_violations(bundle: &Json, approved: &ApprovedHeader) -> Vec<String> {
    let mut v = Vec::new();
    let mut need = |ok: bool, what: String| {
        if !ok {
            v.push(what);
        }
    };

    need(
        object_keys(bundle) == Some(exactly(BUNDLE_KEYS)),
        format!("bundle keys {:?}", object_keys(bundle)),
    );
    need(bundle["format"] == json!(FORMAT), "format".into());
    need(bundle["version"] == json!(1), "version".into());
    need(
        bundle["created_on"]
            .as_str()
            .is_some_and(|d| approved.dates.iter().any(|t| t == d)),
        format!("created_on {}", bundle["created_on"]),
    );
    need(
        object_keys(&bundle["build"]) == Some(exactly(BUILD_KEYS)),
        format!("build keys {:?}", object_keys(&bundle["build"])),
    );
    need(
        bundle["build"]["version"] == json!(env!("CARGO_PKG_VERSION")),
        format!("build.version {}", bundle["build"]["version"]),
    );
    need(
        bundle["build"]["channel"] == json!(env!("PCFO_BUILD_CHANNEL")),
        format!("build.channel {}", bundle["build"]["channel"]),
    );
    need(
        bundle["platform"] == json!(std::env::consts::OS),
        format!("platform {}", bundle["platform"]),
    );
    need(bundle["coverage"] == json!(COVERAGE), "coverage".into());
    need(
        bundle["records_retained"]
            .as_u64()
            .is_some_and(|n| n <= CAPACITY as u64),
        "records_retained".into(),
    );
    need(
        bundle["rejected_at_admission"].as_u64().is_some(),
        "rejected_at_admission".into(),
    );
    match bundle["dropped_by_capacity"].as_object() {
        Some(dropped) => {
            for (metric, count) in dropped {
                need(
                    METRICS.contains(&metric.as_str()) && count.as_u64().is_some_and(|n| n > 0),
                    format!("dropped_by_capacity.{metric} = {count}"),
                );
            }
        }
        None => need(false, "dropped_by_capacity is not an object".into()),
    }
    match bundle["records"].as_array() {
        Some(records) => {
            for (i, record) in records.iter().enumerate() {
                for problem in record_violations(record, approved.max_at_s) {
                    need(false, format!("records[{i}]: {problem}"));
                }
            }
        }
        None => need(false, "records is not an array".into()),
    }
    v
}

fn record_violations(record: &Json, max_at_s: u64) -> Vec<String> {
    let mut v = Vec::new();
    if object_keys(record) != Some(exactly(RECORD_KEYS)) {
        v.push(format!("keys {:?}", object_keys(record)));
        return v;
    }
    if record["at_s"].as_u64().is_none_or(|s| s > max_at_s) {
        v.push(format!("at_s {}", record["at_s"]));
    }
    let Some(metric) = record["metric"].as_str().filter(|m| METRICS.contains(m)) else {
        v.push(format!("metric {}", record["metric"]));
        return v;
    };
    let value = record["value"].as_object();
    let Some((shape, inner)) = value.filter(|o| o.len() == 1).and_then(|o| o.iter().next()) else {
        v.push(format!("value {}", record["value"]));
        return v;
    };
    let ok = match shape.as_str() {
        "duration" => {
            DURATION_METRICS.contains(&metric)
                && inner
                    .as_str()
                    .is_some_and(|b| DURATION_BUCKETS.contains(&b))
        }
        "count" => {
            COUNT_METRICS.contains(&metric)
                && inner
                    .as_u64()
                    .is_some_and(|n| n <= u64::from(COUNT_CEILING))
        }
        "status" => {
            STATUS_METRICS.contains(&metric)
                && inner
                    .as_str()
                    .is_some_and(|s| CONNECTOR_STATUSES.contains(&s))
        }
        "outcome" => {
            OUTCOME_METRICS.contains(&metric)
                && (inner == "success"
                    || (object_keys(inner) == Some(exactly(&["failure"]))
                        && inner["failure"]
                            .as_str()
                            .is_some_and(|c| FAILURE_CATEGORIES.contains(&c))))
        }
        _ => false,
    };
    if !ok {
        v.push(format!("{metric} = {}", record["value"]));
    }
    v
}

/// Byte-level checks that hold for any allowed bundle: printable ASCII only (no
/// raw Unicode, no control characters), no path separators or `@`, and no run
/// long enough to be an identifier, account number or hash.
fn identifier_violations(bytes: &[u8]) -> Vec<String> {
    let mut v = Vec::new();
    for (i, &b) in bytes.iter().enumerate() {
        if !(b == b'\n' || (0x20..0x7f).contains(&b)) {
            v.push(format!("non-printable-ASCII byte {b:#04x} at {i}"));
        }
        if matches!(b, b'/' | b'\\' | b'@') {
            v.push(format!("`{}` at {i}", b as char));
        }
    }
    let text = String::from_utf8_lossy(bytes);
    let mut digits = 0;
    let mut hex = 0;
    for c in text.chars() {
        digits = if c.is_ascii_digit() { digits + 1 } else { 0 };
        hex = if c.is_ascii_hexdigit() { hex + 1 } else { 0 };
        if digits == 8 {
            v.push("a run of 8+ digits (account number, id or amount)".into());
        }
        if hex == 12 {
            v.push("a run of 12+ hex characters (UUID, token or hash)".into());
        }
    }
    v
}

fn leaks_in(haystack: &str) -> Vec<&'static str> {
    FORBIDDEN
        .iter()
        .copied()
        .filter(|needle| haystack.contains(needle))
        .collect()
}

// ---- harness ---------------------------------------------------------------

struct Session {
    _root: TempDir,
    state: AppState,
    vault_dir: PathBuf,
    started: Instant,
}

impl Session {
    fn kernel_diagnostics(&self) -> Diagnostics {
        self.state
            .lock_controller()
            .unwrap()
            .kernel()
            .expect("unlocked")
            .diagnostics()
            .clone()
    }
}

fn unlocked() -> Session {
    let root = TempDir::new().expect("temp dir");
    let vault_dir = root.path().join(HOSTILE_FOLDER);
    std::fs::create_dir(&vault_dir).unwrap();
    let state = AppState::new(VaultController::open(vault_dir.join("vault.db")));
    create_vault_impl(&state, PASSWORD.to_owned()).expect("create vault");
    Session {
        _root: root,
        state,
        vault_dir,
        started: Instant::now(),
    }
}

/// A user's save folder, outside app data, whose own name is hostile.
fn save_folder() -> (TempDir, PathBuf) {
    let dir = TempDir::new().expect("save dir");
    let folder = dir.path().join(SAVE_FOLDER);
    std::fs::create_dir(&folder).unwrap();
    (dir, folder)
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

struct Succeeds;
impl JobHandler<Kernel> for Succeeds {
    fn kind(&self) -> &'static str {
        JOB_KIND_OK
    }
    fn execute(&self, _: &JobRecord, _: &CancellationToken, _: &Kernel) -> JobExecution {
        JobExecution::Succeeded
    }
}

struct FailsWithProviderBody;
impl JobHandler<Kernel> for FailsWithProviderBody {
    fn kind(&self) -> &'static str {
        JOB_KIND_FAIL
    }
    fn execute(&self, _: &JobRecord, _: &CancellationToken, _: &Kernel) -> JobExecution {
        JobExecution::Failed(JobFailure::permanent(format!(
            "{PROVIDER_BODY} {RAW_ERROR}"
        )))
    }
}

fn job(kind: &str) -> JobSpec {
    JobSpec {
        id: Uuid::now_v7(),
        kind: kind.to_owned(),
        schedule: Schedule::Once,
        next_due_at: chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
        max_attempts: 1,
        backoff: BackoffPolicy::default(),
        enabled: true,
        requires_explicit_opt_in: false,
        payload_json: Some(format!(r#"{{"note":"{JOB_PAYLOAD}"}}"#)),
    }
}

/// Push every seeded value through a real input of the running app: account
/// name and opening balance, an imported statement's descriptions and amounts,
/// a connector setup token, a successful and a failing backup (into a hostile
/// path), two durable jobs (one carrying a secret payload, one failing with a
/// provider body and raw error text), and secret-bearing log lines.
///
/// Returns the records the capture sources are expected to have admitted, in
/// order (job durations are checked by shape, not bucket).
fn seed_sensitive_session(session: &Session) -> Vec<(Metric, Option<Value>)> {
    let state = &session.state;
    let account_id = create_account_impl(
        state,
        CreateAccountInput {
            name: ACCOUNT_NAME.to_owned(),
            cashflow_role: CashflowRoleDto::LiquidCash,
            currency: "USD".to_owned(),
            flags: Some(AccountFlagsDto {
                retirement: false,
                tax_advantaged: false,
                joint: false,
                business: false,
            }),
            opening_balance: Some(MoneyDto {
                minor_units: OPENING_BALANCE_MINOR,
                currency: "USD".to_owned(),
            }),
            subtype: None,
            idempotency_key: String::new(),
        },
    )
    .expect("account")
    .account_id;

    let csv = format!(
        "Date,Description,Amount\n2026-06-20,\"{DESCRIPTION}\",{CSV_AMOUNT}\n2026-06-21,\"{PROVIDER_BODY_CSV}\",-42.00\n",
        PROVIDER_BODY_CSV = PROVIDER_BODY.replace('"', "\"\"")
    );
    let imported = import_batch_impl(
        state,
        ImportBatchInput {
            data: csv.into_bytes(),
            filename: Some(format!("{HOSTILE_FOLDER} statement.csv")),
            target_account_id: Some(account_id),
            account_map: None,
            plugin_id: None,
            preset_id: None,
            column_mapping: None,
            default_currency: Some("USD".to_owned()),
            date_format: None,
            idempotency_key: String::new(),
        },
    )
    .expect("import");
    assert_eq!(
        imported.committed, 2,
        "the seeded statement is really in the vault"
    );

    // A credential the provider rejects (its error text comes back too), then
    // a link that succeeds.
    let adapter = MockConnector::with_fixture().with_id("other");
    let link = |setup_token: &str| {
        connector_link_impl(
            state,
            &adapter,
            ConnectorLinkInput {
                adapter_id: "other".to_owned(),
                setup_token: setup_token.to_owned(),
            },
        )
    };
    link(SETUP_TOKEN).expect_err("the mock accepts only its own token");
    link("mock-setup-token").expect("link");

    export_backup_impl(
        state,
        path_string(&session.vault_dir.join("ok backup.pcfobk")),
    )
    .expect("backup");
    let hostile = session
        .vault_dir
        .join(RAW_ERROR.replace('/', "_"))
        .join("missing")
        .join("backup.pcfobk");
    export_backup_impl(state, path_string(&hostile)).expect_err("folder does not exist");

    {
        let guard = state.lock_controller().unwrap();
        let kernel = guard.kernel().unwrap();
        kernel.schedule_job(&job(JOB_KIND_OK)).unwrap();
        kernel.schedule_job(&job(JOB_KIND_FAIL)).unwrap();
    }
    state.register_job_handler(Arc::new(Succeeds)).unwrap();
    state
        .register_job_handler(Arc::new(FailsWithProviderBody))
        .unwrap();
    let report = run_due_jobs_on_unlock_impl(state, state.job_dispatcher().as_ref(), "ryjx")
        .expect("run jobs");
    assert_eq!((report.succeeded, report.failed), (1, 1), "{report:?}");

    // Logs and the capture are separate kinds of record (§5): a log line can
    // never become a bundle record.
    tracing::warn!("password={PASSWORD} token={SETUP_TOKEN} balance $98,765.43 {PROVIDER_BODY}");

    vec![
        (
            Metric::BackupOutcome,
            Some(Value::Outcome(Outcome::Success)),
        ),
        (
            Metric::BackupOutcome,
            Some(Value::Outcome(Outcome::Failure(
                FailureCategory::Validation,
            ))),
        ),
        (Metric::JobDuration, None),
        (Metric::JobDuration, None),
    ]
}

/// Every file under `dirs` that looks like a diagnostic artifact: anything
/// holding the bundle's format marker in plaintext.
fn diagnostic_artifacts(dirs: &[&Path]) -> Vec<PathBuf> {
    fn walk(dir: &Path, found: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.is_dir() {
                walk(&path, found);
            } else if meta.is_file()
                && std::fs::read(&path)
                    .is_ok_and(|b| b.windows(FORMAT.len()).any(|w| w == FORMAT.as_bytes()))
            {
                found.push(path);
            }
        }
    }
    let mut found = Vec::new();
    for dir in dirs {
        walk(dir, &mut found);
    }
    found
}

/// Captures everything the process logs, **unredacted**, so a test can prove a
/// value never reaches a sink at all (not merely that the redactor caught it).
#[derive(Clone, Default)]
struct RawLog(Arc<Mutex<Vec<u8>>>);

impl Write for RawLog {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl RawLog {
    fn capture<T>(&self, f: impl FnOnce() -> T) -> T {
        let writer = self.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE)
            .with_writer(move || writer.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, f)
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

fn preview(session: &Session) -> DiagnosticsPreviewDto {
    diagnostics_preview_impl(&session.state).expect("preview")
}

fn bundle_json(text: &str) -> Json {
    serde_json::from_str(text).expect("bundle is JSON")
}

/// Replace the values that legitimately vary by run, date, build or OS with
/// fixed markers — after asserting each is exactly the approved value — so the
/// snapshot pins everything else byte for byte.
fn normalized(text: &str, approved: &ApprovedHeader) -> String {
    let violations = allowlist_violations(&bundle_json(text), approved);
    assert!(violations.is_empty(), "{violations:?}");
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let indent = &line[..line.len() - trimmed.len()];
        let replace = |key: &str, marker: &str| {
            trimmed.starts_with(&format!("\"{key}\": ")).then(|| {
                let comma = if trimmed.trim_end().ends_with(',') {
                    ","
                } else {
                    ""
                };
                format!("{indent}\"{key}\": \"{marker}\"{comma}\n")
            })
        };
        let replaced = replace("at_s", "<session-seconds>")
            .or_else(|| replace("created_on", "<utc-date>"))
            .or_else(|| replace("platform", "<os>"))
            .or_else(|| replace("channel", "<build-channel>"))
            .or_else(|| {
                // `build.version` — the only `"version": "…"` (string) line.
                trimmed
                    .starts_with("\"version\": \"")
                    .then(|| replace("version", "<app-version>"))
                    .flatten()
            });
        out.push_str(replaced.as_deref().unwrap_or(line));
    }
    out
}

fn without_time(bundle: &observability::diagnostics::Bundle) -> Vec<(Metric, Value)> {
    bundle.records.iter().map(|r| (r.metric, r.value)).collect()
}

// ---- AC1: the seeded corpus never reaches anything the export produces -------

#[test]
fn seeded_sensitive_values_never_reach_the_preview_the_saved_file_or_any_artifact() {
    let session = unlocked();
    let expected = seed_sensitive_session(&session);
    let (save_root, folder) = save_folder();
    // The seeded statements really are in the vault: the corpus was live input.
    assert!(
        diagnostic_artifacts(&[&session.vault_dir, save_root.path()]).is_empty(),
        "capture writes nothing to disk on its own"
    );

    let log = RawLog::default();
    let (dto, result, target) = log.capture(|| {
        let dto = preview(&session);
        let target = folder.join("dohflow-diagnostics.json");
        let result =
            diagnostics_save_impl(&session.state, dto.snapshot_id, path_string(&target)).unwrap();
        (dto, result, target)
    });
    assert_eq!(result, DiagnosticsSaveResult::Saved);

    // Packaging: the saved bytes are exactly the previewed bytes.
    let saved = std::fs::read(&target).expect("saved");
    assert_eq!(saved, dto.text.as_bytes(), "saved bytes == previewed bytes");

    // Parse: the retained records are exactly what the sources admitted.
    let bundle = parse_bundle(&saved).expect("round-trips");
    assert_eq!(bundle.records.len(), expected.len(), "{}", dto.text);
    for (record, (metric, value)) in bundle.records.iter().zip(&expected) {
        assert_eq!(record.metric, *metric);
        match value {
            Some(value) => assert_eq!(record.value, *value),
            None => assert!(matches!(record.value, Value::Duration(_))),
        }
    }
    assert_eq!(dto.records as usize, expected.len());
    assert_eq!((dto.dropped, dto.rejected), (0, 0));

    // Nothing seeded survives — in the preview, the saved file, the DTO's other
    // fields, the save's own log output, or any file in the save folder.
    // The corpus was live input: the vault holds it, and the log capture saw
    // the export run.
    let accounts = account_list_impl(&session.state).unwrap();
    assert!(accounts.iter().any(|a| a.name == ACCOUNT_NAME));
    assert!(log.text().contains("diagnostics_save"), "{}", log.text());

    let saved_text = String::from_utf8(saved.clone()).unwrap();
    for (surface, text) in [
        ("preview", dto.text.clone()),
        ("saved file", saved_text),
        ("suggested file name", dto.suggested_file_name.clone()),
        ("preview DTO", format!("{dto:?}")),
        ("save result", serde_json::to_string(&result).unwrap()),
        ("preview/save logs", log.text()),
    ] {
        assert_eq!(leaks_in(&text), Vec::<&str>::new(), "{surface} leaked");
    }
    for path in [
        session.vault_dir.as_path(),
        save_root.path(),
        folder.as_path(),
    ] {
        let p = path_string(path);
        assert!(!dto.text.contains(&p), "preview holds a path");
        assert!(!log.text().contains(&p), "the save logged a path");
    }
    assert_eq!(
        diagnostic_artifacts(&[save_root.path(), &session.vault_dir]),
        vec![target],
        "the save created exactly one artifact"
    );

    // And the bundle is fully inside the allowlist.
    let approved = ApprovedHeader::since(session.started);
    assert_eq!(
        allowlist_violations(&bundle_json(&dto.text), &approved),
        Vec::<String>::new()
    );
    assert_eq!(identifier_violations(&saved), Vec::<String>::new());
}

// ---- AC2: positive controls — only policy-allowed fields and values ----------

#[test]
fn a_bundle_with_every_metric_holds_only_policy_allowed_fields_and_values() {
    let session = unlocked();
    seed_sensitive_session(&session);
    let diagnostics = session.kernel_diagnostics();
    // One record of every metric through the production admission, every
    // status and failure category, both ends of the count range, and every
    // duration bucket.
    for (metric, value) in [
        (Metric::ImportRows, Value::Count(0)),
        (Metric::DedupeCandidates, Value::Count(COUNT_CEILING)),
        (Metric::RetryCount, Value::Count(3)),
        (
            Metric::ForecastDuration,
            Value::Duration(DurationBucket::Under10ms),
        ),
        (
            Metric::QueryDuration,
            Value::Duration(DurationBucket::From10To100ms),
        ),
        (
            Metric::UiRenderTiming,
            Value::Duration(DurationBucket::From100msTo1s),
        ),
        (
            Metric::MigrationDuration,
            Value::Duration(DurationBucket::From1To10s),
        ),
        (
            Metric::JobDuration,
            Value::Duration(DurationBucket::Over10s),
        ),
        (Metric::RestoreOutcome, Value::Outcome(Outcome::Success)),
    ] {
        diagnostics.record(metric, value);
    }
    for status in [
        ConnectorStatus::Ok,
        ConnectorStatus::AuthFailed,
        ConnectorStatus::RateLimited,
        ConnectorStatus::ProviderError,
        ConnectorStatus::NetworkUnavailable,
    ] {
        diagnostics.record(Metric::ConnectorStatus, Value::Status(status));
    }
    for category in [
        FailureCategory::Validation,
        FailureCategory::Storage,
        FailureCategory::Vault,
        FailureCategory::PermissionDenied,
        FailureCategory::DiskFull,
        FailureCategory::Io,
        FailureCategory::Unavailable,
        FailureCategory::Internal,
    ] {
        diagnostics.record_outcome(Metric::BackupOutcome, Outcome::Failure(category));
    }
    // An identifier-sized number offered as a count, and wrong-shape values.
    diagnostics.record_count(Metric::ImportRows, 4_111_111_111_111_111);
    diagnostics.record(Metric::BackupOutcome, Value::Count(9_876_543));
    diagnostics.record(Metric::RetryCount, Value::Status(ConnectorStatus::Ok));

    let dto = preview(&session);
    let json = bundle_json(&dto.text);
    let approved = ApprovedHeader::since(session.started);
    assert_eq!(allowlist_violations(&json, &approved), Vec::<String>::new());
    assert_eq!(
        identifier_violations(dto.text.as_bytes()),
        Vec::<String>::new()
    );
    assert_eq!(leaks_in(&dto.text), Vec::<&str>::new());

    // Every metric, every status, every category appears: the positive control
    // is not passing vacuously.
    let metrics: BTreeSet<&str> = json["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["metric"].as_str().unwrap())
        .collect();
    assert_eq!(metrics, exactly(METRICS));
    for word in CONNECTOR_STATUSES
        .iter()
        .chain(FAILURE_CATEGORIES)
        .chain(DURATION_BUCKETS)
    {
        assert!(dto.text.contains(&format!("\"{word}\"")), "{word}");
    }
    assert_eq!(json["rejected_at_admission"], json!(2));
    assert!(dto.text.contains(&format!(": {COUNT_CEILING}")));
    assert!(!dto.text.contains("4111111"), "a large count saturates");
}

#[test]
fn the_allowlist_oracle_catches_every_kind_of_forbidden_field() {
    // The in-suite mutation check: take a real, allowed bundle and insert each
    // kind of forbidden content. If the oracle missed any of these, the other
    // tests would pass vacuously.
    let session = unlocked();
    seed_sensitive_session(&session);
    session
        .kernel_diagnostics()
        .record(Metric::ConnectorStatus, Value::Status(ConnectorStatus::Ok));
    let dto = preview(&session);
    let approved = ApprovedHeader::since(session.started);
    let clean = bundle_json(&dto.text);
    assert!(allowlist_violations(&clean, &approved).is_empty());

    let mutations: Vec<Mutation> = vec![
        (
            "top-level vault id",
            Box::new(|b| b["vault_id"] = json!(Uuid::now_v7().to_string())),
        ),
        (
            "top-level device id",
            Box::new(|b| b["device_id"] = json!("MacBook-Pro-of-Jane")),
        ),
        (
            "build commit hash",
            Box::new(|b| b["build"]["commit"] = json!("0604447a1b2c3d4e")),
        ),
        (
            "build version carrying text",
            Box::new(|b| b["build"]["version"] = json!("0.2.0 password=hunter2")),
        ),
        (
            "platform carrying a host name",
            Box::new(|b| b["platform"] = json!("jane-macbook.local")),
        ),
        (
            "created_on with a time",
            Box::new(|b| b["created_on"] = json!("2026-10-03T10:11:12Z")),
        ),
        (
            "rewritten coverage",
            Box::new(|b| b["coverage"] = json!("Complete history of acct 4111")),
        ),
        (
            "record message",
            Box::new(|b| b["records"][0]["message"] = json!("ENOENT /Users/jane")),
        ),
        (
            "record correlation id",
            Box::new(|b| b["records"][0]["correlation_id"] = json!(Uuid::now_v7().to_string())),
        ),
        (
            "value with a second key",
            Box::new(|b| b["records"][0]["value"]["path"] = json!("/Users/jane")),
        ),
        (
            "unknown metric",
            Box::new(|b| b["records"][0]["metric"] = json!("account_balance")),
        ),
        (
            "unknown status",
            Box::new(|b| {
                let last = b["records"].as_array().unwrap().len() - 1;
                b["records"][last]["value"]["status"] = json!("HTTP/1.1 401 Unauthorized");
            }),
        ),
        (
            "failure carrying text",
            Box::new(|b| b["records"][1]["value"]["outcome"]["failure"] = json!("ENOENT")),
        ),
        (
            "count above the ceiling",
            Box::new(|b| {
                b["records"][0]["value"] = json!({ "count": 4_111_111_111_111_111_u64 });
                b["records"][0]["metric"] = json!("import_rows");
            }),
        ),
        (
            "wrong shape for its metric",
            Box::new(|b| b["records"][0]["value"] = json!({ "count": 1 })),
        ),
        (
            "wall-clock timestamp",
            Box::new(|b| b["records"][0]["at_s"] = json!(1_790_000_000_u64)),
        ),
        (
            "dropped counter for an unknown metric",
            Box::new(|b| b["dropped_by_capacity"]["account_number"] = json!(1)),
        ),
    ];
    for (name, mutate) in mutations {
        let mut mutated = clean.clone();
        mutate(&mut mutated);
        assert!(
            !allowlist_violations(&mutated, &approved).is_empty(),
            "the oracle missed: {name}"
        );
    }

    // And the byte-level identifier scan catches what a string allowlist would
    // not see in a number.
    for bad in [
        r#"{"x": 41111111}"#,
        r#"{"x": "0f8e7d6c5b4a"}"#,
        r#"{"x": "a/b"}"#,
        r#"{"x": "a@b"}"#,
        "{\"x\": \"Zo\u{eb}\"}",
        "{\"x\": \"tab\there\"}",
    ] {
        assert!(!identifier_violations(bad.as_bytes()).is_empty(), "{bad}");
    }
}

// ---- AC3: parsed output equals the approved snapshot; eviction accounted -----

#[test]
fn the_retained_records_equal_the_approved_redacted_snapshot() {
    let session = unlocked();
    let state = &session.state;
    let (save_root, folder) = save_folder();

    // Real sources: a successful and a failing backup.
    export_backup_impl(state, path_string(&session.vault_dir.join("a.pcfobk"))).unwrap();
    export_backup_impl(
        state,
        path_string(&session.vault_dir.join("absent").join("b.pcfobk")),
    )
    .unwrap_err();
    // The production admission for the shapes no source wires yet, plus two
    // rejections (wrong shape for the metric).
    let diagnostics = session.kernel_diagnostics();
    diagnostics.record(
        Metric::ConnectorStatus,
        Value::Status(ConnectorStatus::RateLimited),
    );
    diagnostics.record_count(Metric::ImportRows, 42);
    diagnostics.record_duration(Metric::QueryDuration, Duration::from_millis(250));
    diagnostics.record(Metric::JobDuration, Value::Count(5));
    diagnostics.record(
        Metric::RestoreOutcome,
        Value::Duration(DurationBucket::Over10s),
    );

    let dto = preview(&session);
    let target = folder.join("snapshot.json");
    assert_eq!(
        diagnostics_save_impl(state, dto.snapshot_id, path_string(&target)).unwrap(),
        DiagnosticsSaveResult::Saved
    );
    let saved = std::fs::read_to_string(&target).unwrap();
    assert_eq!(saved, dto.text);
    drop(save_root);

    let approved = ApprovedHeader::since(session.started);
    insta::assert_snapshot!("populated_session_bundle", normalized(&saved, &approved));

    let bundle = parse_bundle(saved.as_bytes()).unwrap();
    assert_eq!(
        without_time(&bundle),
        vec![
            (Metric::BackupOutcome, Value::Outcome(Outcome::Success)),
            (
                Metric::BackupOutcome,
                Value::Outcome(Outcome::Failure(FailureCategory::Validation))
            ),
            (
                Metric::ConnectorStatus,
                Value::Status(ConnectorStatus::RateLimited)
            ),
            (Metric::ImportRows, Value::Count(42)),
            (
                Metric::QueryDuration,
                Value::Duration(DurationBucket::From100msTo1s)
            ),
        ]
    );
    assert_eq!(bundle.rejected_at_admission, 2);
    assert!(bundle.dropped_by_capacity.is_empty());
    assert_eq!((dto.records, dto.dropped, dto.rejected), (5, 0, 2));
}

#[test]
fn an_empty_session_round_trips_to_the_approved_empty_bundle() {
    let session = unlocked();
    let dto = preview(&session);
    assert_eq!((dto.records, dto.dropped, dto.rejected), (0, 0, 0));
    let approved = ApprovedHeader::since(session.started);
    insta::assert_snapshot!("empty_session_bundle", normalized(&dto.text, &approved));
    let bundle = parse_bundle(dto.text.as_bytes()).unwrap();
    assert!(bundle.records.is_empty());
    assert_eq!(bundle.records_retained, 0);

    // An empty bundle saves like any other: still exactly the preview.
    let (_save_root, folder) = save_folder();
    let target = folder.join("empty.json");
    assert_eq!(
        diagnostics_save_impl(&session.state, dto.snapshot_id, path_string(&target)).unwrap(),
        DiagnosticsSaveResult::Saved
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), dto.text);
}

#[test]
fn eviction_and_rejection_are_accounted_for_exactly() {
    let session = unlocked();
    let diagnostics = session.kernel_diagnostics();
    // A model of what the ring must hold. The two oldest records come from a
    // real source, so real records are among those evicted.
    let mut model: VecDeque<(Metric, Value)> = VecDeque::new();
    let mut admitted: BTreeMap<Metric, u64> = BTreeMap::new();
    let mut dropped: BTreeMap<Metric, u64> = BTreeMap::new();
    let mut admit = |model: &mut VecDeque<(Metric, Value)>, metric, value| {
        *admitted.entry(metric).or_default() += 1;
        if model.len() == CAPACITY {
            let (old, _) = model.pop_front().unwrap();
            *dropped.entry(old).or_default() += 1;
        }
        model.push_back((metric, value));
    };

    export_backup_impl(
        &session.state,
        path_string(&session.vault_dir.join("a.pcfobk")),
    )
    .unwrap();
    admit(
        &mut model,
        Metric::BackupOutcome,
        Value::Outcome(Outcome::Success),
    );
    export_backup_impl(
        &session.state,
        path_string(&session.vault_dir.join("b.pcfobk")),
    )
    .unwrap();
    admit(
        &mut model,
        Metric::BackupOutcome,
        Value::Outcome(Outcome::Success),
    );

    // Exactly at capacity: nothing dropped yet.
    let cycle = [
        (Metric::RetryCount, Value::Count(1)),
        (
            Metric::QueryDuration,
            Value::Duration(DurationBucket::Under10ms),
        ),
        (Metric::ConnectorStatus, Value::Status(ConnectorStatus::Ok)),
    ];
    for i in 0..CAPACITY - 2 {
        let (metric, value) = cycle[i % cycle.len()];
        diagnostics.record(metric, value);
        admit(&mut model, metric, value);
    }
    let at_capacity = preview(&session);
    assert_eq!(
        (at_capacity.records, at_capacity.dropped),
        (CAPACITY as u32, 0),
        "exactly CAPACITY records: no eviction"
    );

    // One past capacity drops exactly the oldest (a real backup record).
    diagnostics.record(Metric::ImportRows, Value::Count(7));
    admit(&mut model, Metric::ImportRows, Value::Count(7));
    let one_over = parse_bundle(preview(&session).text.as_bytes()).unwrap();
    assert_eq!(
        one_over.dropped_by_capacity,
        BTreeMap::from([(Metric::BackupOutcome, 1)])
    );

    // Well past capacity, with rejections interleaved (never stored, never
    // evict anything).
    let mut rejected = 0_u64;
    for i in 0..(CAPACITY / 2 + 17) {
        let (metric, value) = cycle[(i + 1) % cycle.len()];
        diagnostics.record(metric, value);
        admit(&mut model, metric, value);
        if i % 100 == 0 {
            diagnostics.record(Metric::BackupOutcome, Value::Count(1));
            rejected += 1;
        }
    }

    let dto = preview(&session);
    let bundle = parse_bundle(dto.text.as_bytes()).unwrap();
    assert_eq!(
        without_time(&bundle),
        model.iter().copied().collect::<Vec<_>>(),
        "the ring holds exactly the newest CAPACITY admitted records, in order"
    );
    assert_eq!(bundle.dropped_by_capacity, dropped);
    assert_eq!(bundle.rejected_at_admission, rejected);
    // Retained + dropped == admitted, per metric.
    for (metric, total) in &admitted {
        let retained = bundle
            .records
            .iter()
            .filter(|r| r.metric == *metric)
            .count() as u64;
        let gone = dropped.get(metric).copied().unwrap_or(0);
        assert_eq!(retained + gone, *total, "{metric:?}");
    }
    // The UI's overflow summary reports the same accounting.
    assert_eq!(dto.records as usize, CAPACITY);
    assert_eq!(u64::from(dto.dropped), dropped.values().sum::<u64>());
    assert_eq!(u64::from(dto.rejected), rejected);
    assert_eq!(
        allowlist_violations(
            &bundle_json(&dto.text),
            &ApprovedHeader::since(session.started)
        ),
        Vec::<String>::new()
    );
}

#[test]
fn records_and_evictions_after_the_preview_never_enter_the_saved_bundle() {
    let session = unlocked();
    let diagnostics = session.kernel_diagnostics();
    for _ in 0..CAPACITY {
        diagnostics.record_count(Metric::RetryCount, 1);
    }
    let dto = preview(&session);

    // After the preview the ring keeps moving: new records, and evictions of
    // records the preview holds.
    seed_sensitive_session(&session);
    for _ in 0..10 {
        diagnostics.record(
            Metric::ConnectorStatus,
            Value::Status(ConnectorStatus::AuthFailed),
        );
    }

    let (_save_root, folder) = save_folder();
    let target = folder.join("frozen.json");
    assert_eq!(
        diagnostics_save_impl(&session.state, dto.snapshot_id, path_string(&target)).unwrap(),
        DiagnosticsSaveResult::Saved
    );
    let saved = std::fs::read(&target).unwrap();
    assert_eq!(saved, dto.text.as_bytes());
    let bundle = parse_bundle(&saved).unwrap();
    assert!(bundle
        .records
        .iter()
        .all(|r| r.metric == Metric::RetryCount));
    assert!(bundle.dropped_by_capacity.is_empty());
    assert_eq!(bundle.rejected_at_admission, 0);
}

// ---- AC4: edge cases --------------------------------------------------------

#[test]
fn unknown_duplicate_and_escaped_fields_are_rejected_at_every_level() {
    let session = unlocked();
    seed_sensitive_session(&session);
    let text = preview(&session).text;
    let clean = bundle_json(&text);
    assert!(parse_bundle(text.as_bytes()).is_ok());

    let malformed = |b: &str| matches!(parse_bundle(b.as_bytes()), Err(ParseError::Malformed(_)));
    let mutations: Vec<Mutation> = vec![
        ("top level", Box::new(|b| b["note"] = json!("acct 4111"))),
        ("build", Box::new(|b| b["build"]["commit"] = json!("abc"))),
        (
            "record",
            Box::new(|b| b["records"][0]["message"] = json!("x")),
        ),
        (
            "value",
            Box::new(|b| b["records"][0]["value"]["path"] = json!("/x")),
        ),
    ];
    for (name, mutate) in mutations {
        let mut mutated = clean.clone();
        mutate(&mut mutated);
        assert!(
            malformed(&serde_json::to_string(&mutated).unwrap()),
            "unknown field at {name} must be rejected"
        );
    }

    // A second `records` (or `coverage`) key cannot smuggle a different payload
    // past a reader that keeps the first one.
    for key in ["records", "coverage", "format"] {
        let duplicated = text.replacen(
            &format!("\"{key}\""),
            &format!("\"{key}\": {},\n  \"{key}\"", clean[key]),
            1,
        );
        assert_ne!(duplicated, text);
        assert!(malformed(&duplicated), "duplicate {key}");
    }

    // JSON escapes decode before the allowlist applies: an escaped unknown key
    // is still unknown, and escaped text in a fixed field is still not the
    // fixed text.
    let escaped_key = text.replacen("\"platform\"", "\"\\u006eote\": \"x\",\n  \"platform\"", 1);
    assert!(malformed(&escaped_key));
    let escaped_coverage = text.replacen("Covers only", "Covers \\u00f6nly", 1);
    assert_eq!(
        parse_bundle(escaped_coverage.as_bytes()),
        Err(ParseError::UnsupportedFormat)
    );
    // An escaped spelling of an allowed value decodes to that value — and is
    // still allowed, because it carries nothing new.
    let escaped_allowed = text.replacen("\"backup_outcome\"", "\"\\u0062ackup_outcome\"", 1);
    assert_ne!(escaped_allowed, text);
    assert!(parse_bundle(escaped_allowed.as_bytes()).is_ok());
}

#[test]
fn unicode_and_escape_characters_cannot_enter_a_bundle() {
    // The header is the only place the app puts text of its own choosing; each
    // field is held to a narrow ASCII set at preview time.
    let base = BundleHeader {
        build_version: "0.2.0".into(),
        build_channel: "release".into(),
        platform: "macos".into(),
        created_on: "2026-10-03".into(),
    };
    let d = Diagnostics::new();
    d.record_count(Metric::RetryCount, 1);
    for bad in [
        BundleHeader {
            build_version: "0.2.0é".into(),
            ..base.clone()
        },
        BundleHeader {
            build_version: "0.2.0\u{202e}".into(),
            ..base.clone()
        },
        BundleHeader {
            platform: "mac\"os".into(),
            ..base.clone()
        },
        BundleHeader {
            platform: "mac\\os".into(),
            ..base.clone()
        },
        BundleHeader {
            build_channel: "release\n\"injected\": 1".into(),
            ..base.clone()
        },
        BundleHeader {
            build_channel: "release\u{0}".into(),
            ..base.clone()
        },
        BundleHeader {
            created_on: "２０２６-10-03".into(),
            ..base.clone()
        },
    ] {
        assert_eq!(d.preview(&bad), Err(PreviewError::InvalidHeader), "{bad:?}");
    }
    assert!(d.preview(&base).is_ok());

    // A destination full of Unicode, quotes and a newline is a legitimate place
    // to save; none of it reaches the bundle, and the bundle stays ASCII.
    let session = unlocked();
    seed_sensitive_session(&session);
    let dto = preview(&session);
    let save_root = TempDir::new().unwrap();
    let folder = save_root.path().join("Zoë \"quoted\" \\ ✓ 数据\nnext");
    if std::fs::create_dir(&folder).is_ok() {
        let target = folder.join("ünïcode.json");
        assert_eq!(
            diagnostics_save_impl(&session.state, dto.snapshot_id, path_string(&target)).unwrap(),
            DiagnosticsSaveResult::Saved
        );
        let saved = std::fs::read(&target).unwrap();
        assert_eq!(saved, dto.text.as_bytes());
        assert_eq!(identifier_violations(&saved), Vec::<String>::new());
        let text = String::from_utf8(saved).unwrap();
        for fragment in ["Zoë", "quoted", "数据", "ünïcode", "next"] {
            assert!(!text.contains(fragment), "{fragment}");
        }
    }
}

#[test]
fn truncation_and_ceiling_boundaries_hold() {
    let session = unlocked();
    let diagnostics = session.kernel_diagnostics();
    for n in [
        0,
        u64::from(COUNT_CEILING) - 1,
        u64::from(COUNT_CEILING),
        u64::from(COUNT_CEILING) + 1,
        u64::from(u32::MAX) + 1,
        u64::MAX,
    ] {
        diagnostics.record_count(Metric::ImportRows, n);
    }
    let dto = preview(&session);
    let bundle = parse_bundle(dto.text.as_bytes()).unwrap();
    assert_eq!(
        bundle.records.iter().map(|r| r.value).collect::<Vec<_>>(),
        vec![
            Value::Count(0),
            Value::Count(COUNT_CEILING - 1),
            Value::Count(COUNT_CEILING),
            Value::Count(COUNT_CEILING),
            Value::Count(COUNT_CEILING),
            Value::Count(COUNT_CEILING),
        ]
    );

    // A bundle cut short anywhere before its closing brace never parses, and
    // never panics: a partially written file is refused, not half-read.
    let bytes = dto.text.as_bytes();
    let end = dto.text.rfind('}').unwrap();
    for len in 0..=end {
        assert!(
            parse_bundle(&bytes[..len]).is_err(),
            "prefix of {len} bytes"
        );
    }
    // Only trailing whitespace is optional.
    assert!(parse_bundle(&bytes[..=end]).is_ok());

    // Record-count boundary on the parse side: CAPACITY records parse; one more
    // is inconsistent even when `records_retained` agrees.
    let mut json = bundle_json(&dto.text);
    let one = json["records"][0].clone();
    json["records"] = Json::Array(vec![one.clone(); CAPACITY]);
    json["records_retained"] = json!(CAPACITY);
    assert!(parse_bundle(serde_json::to_string(&json).unwrap().as_bytes()).is_ok());
    json["records"] = Json::Array(vec![one; CAPACITY + 1]);
    json["records_retained"] = json!(CAPACITY + 1);
    assert_eq!(
        parse_bundle(serde_json::to_string(&json).unwrap().as_bytes()),
        Err(ParseError::Inconsistent)
    );
}

#[test]
fn preview_and_cancel_create_no_diagnostic_artifact() {
    let session = unlocked();
    seed_sensitive_session(&session);
    let (save_root, folder) = save_folder();

    // Preview alone writes nothing anywhere.
    let dto = preview(&session);
    assert!(diagnostic_artifacts(&[&session.vault_dir, save_root.path()]).is_empty());

    // Cancelling (Close, Escape, or dismissing the Save dialog all discard)
    // leaves the preview unsavable and still writes nothing.
    diagnostics_discard_impl(&session.state, dto.snapshot_id).unwrap();
    let target = folder.join("cancelled.json");
    assert_eq!(
        diagnostics_save_impl(&session.state, dto.snapshot_id, path_string(&target)).unwrap(),
        DiagnosticsSaveResult::PreviewExpired
    );
    // A guessed snapshot id is no better.
    for guess in [0, dto.snapshot_id + 1, u32::MAX] {
        assert_eq!(
            diagnostics_save_impl(&session.state, guess, path_string(&target)).unwrap(),
            DiagnosticsSaveResult::PreviewExpired
        );
    }
    assert!(!target.exists());
    assert!(diagnostic_artifacts(&[&session.vault_dir, save_root.path()]).is_empty());
}

#[cfg(unix)]
#[test]
fn write_failures_are_fixed_results_that_reveal_nothing_and_leave_no_artifact() {
    use std::os::unix::fs::PermissionsExt;

    let session = unlocked();
    seed_sensitive_session(&session);
    let (save_root, folder) = save_folder();
    let dto = preview(&session);

    let read_only = folder.join("read-only ACCT-SENTINEL-771");
    std::fs::create_dir(&read_only).unwrap();
    let existing = folder.join("existing.json");
    std::fs::write(&existing, b"user's own file").unwrap();
    std::fs::set_permissions(&existing, std::fs::Permissions::from_mode(0o444)).unwrap();
    std::fs::set_permissions(&read_only, std::fs::Permissions::from_mode(0o555)).unwrap();
    let a_directory = folder.join("a-directory.json");
    std::fs::create_dir(&a_directory).unwrap();

    let log = RawLog::default();
    let results = log.capture(|| {
        [
            read_only.join("d.json"),
            existing.clone(),
            a_directory.clone(),
        ]
        .map(|target| {
            diagnostics_save_impl(&session.state, dto.snapshot_id, path_string(&target)).unwrap()
        })
    });
    std::fs::set_permissions(&read_only, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::set_permissions(&existing, std::fs::Permissions::from_mode(0o644)).unwrap();

    assert_eq!(
        results,
        [
            DiagnosticsSaveResult::PermissionDenied,
            DiagnosticsSaveResult::PermissionDenied,
            DiagnosticsSaveResult::InvalidDestination,
        ]
    );
    for result in &results {
        let wire = serde_json::to_string(result).unwrap();
        assert!(
            SAVE_RESULTS.contains(&wire.trim_matches('"')),
            "a save result is a fixed token, not text: {wire}"
        );
    }
    let logged = log.text();
    assert!(
        logged.contains("diagnostics_save"),
        "the capture saw the save"
    );
    assert_eq!(leaks_in(&logged), Vec::<&str>::new(), "{logged}");
    assert!(!logged.contains(&path_string(&folder)));
    assert!(!read_only.join("d.json").exists());
    assert_eq!(std::fs::read(&existing).unwrap(), b"user's own file");
    assert!(diagnostic_artifacts(&[&session.vault_dir, save_root.path()]).is_empty());

    // A failed save leaves the preview pending: the user can choose again, and
    // gets exactly the bytes they were shown.
    let retry = folder.join("retry.json");
    assert_eq!(
        diagnostics_save_impl(&session.state, dto.snapshot_id, path_string(&retry)).unwrap(),
        DiagnosticsSaveResult::Saved
    );
    assert_eq!(std::fs::read(&retry).unwrap(), dto.text.as_bytes());
}

#[test]
fn a_vault_lock_discards_the_preview_and_reveals_nothing() {
    let session = unlocked();
    seed_sensitive_session(&session);
    let (save_root, folder) = save_folder();
    let dto = preview(&session);
    assert!(dto.records > 0);

    lock_vault_impl(&session.state).unwrap();
    let target = folder.join("locked.json");
    let error = diagnostics_save_impl(&session.state, dto.snapshot_id, path_string(&target))
        .expect_err("locked");
    assert!(matches!(error, IpcError::VaultLocked));
    let wire = serde_json::to_string(&error).unwrap();
    assert_eq!(leaks_in(&wire), Vec::<&str>::new(), "{wire}");
    assert!(matches!(
        diagnostics_preview_impl(&session.state),
        Err(IpcError::VaultLocked)
    ));

    unlock_vault_impl(&session.state, PASSWORD.to_owned()).unwrap();
    assert_eq!(
        diagnostics_save_impl(&session.state, dto.snapshot_id, path_string(&target)).unwrap(),
        DiagnosticsSaveResult::PreviewExpired
    );
    let fresh = preview(&session);
    assert_eq!(fresh.records, 0, "the locked session's capture is gone");
    assert!(diagnostic_artifacts(&[&session.vault_dir, save_root.path()]).is_empty());
}

#[test]
fn a_vault_switch_discards_the_preview_and_nothing_follows_into_the_new_vault() {
    let root = TempDir::new().unwrap();
    let registry = VaultRegistry::load(root.path());
    let path = registry
        .active_path(root.path())
        .unwrap_or_else(|| root.path().join("vault.db"));
    let state = AppState::with_registry(
        VaultController::open(path),
        registry,
        root.path().to_path_buf(),
    );
    create_vault_named_impl(&state, "First".into(), PASSWORD.into()).unwrap();
    let vault_dir = {
        let guard = state.lock_controller().unwrap();
        guard.path().parent().unwrap().to_path_buf()
    };
    let first = Session {
        _root: root,
        state,
        vault_dir,
        started: Instant::now(),
    };
    seed_sensitive_session(&first);
    let dto = preview(&first);
    assert!(dto.records > 0);

    create_vault_named_impl(&first.state, "Second".into(), PASSWORD.into()).unwrap();
    let (save_root, folder) = save_folder();
    let target = folder.join("switched.json");
    assert_eq!(
        diagnostics_save_impl(&first.state, dto.snapshot_id, path_string(&target)).unwrap(),
        DiagnosticsSaveResult::PreviewExpired
    );
    let second = preview(&first);
    assert_eq!(second.records, 0, "the first vault's capture never follows");
    assert_eq!(leaks_in(&second.text), Vec::<&str>::new());
    assert!(diagnostic_artifacts(&[first._root.path(), save_root.path()]).is_empty());
}

// ---- AC5: no egress capability in the export path ----------------------------

#[test]
fn the_exporter_crate_has_no_network_capable_dependency() {
    // The bundle is built and parsed by `observability::diagnostics`; the save
    // is `std::fs::write` in the IPC layer. Pin the exporter crate's
    // dependencies so a network client cannot be added to it unnoticed. (The
    // desktop crate as a whole does have connector HTTP clients; this suite
    // never constructs one — it uses only temp folders and the in-memory mock.)
    let manifest = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../crates/observability/Cargo.toml"),
    )
    .expect("observability manifest");
    let manifest: toml::Table = manifest.parse().expect("manifest parses");
    let deps: BTreeSet<&str> = manifest["dependencies"]
        .as_table()
        .expect("dependencies table")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        deps,
        BTreeSet::from([
            "regex",
            "serde",
            "serde_json",
            "tracing",
            "tracing-subscriber"
        ]),
        "a new exporter dependency needs logging-policy §8 review"
    );
    assert!(
        !manifest.contains_key("build-dependencies"),
        "no build script may reach the network"
    );
}
