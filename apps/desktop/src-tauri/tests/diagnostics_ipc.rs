//! Local diagnostics through the real IPC implementations (personal-cfo-lyd,
//! `docs/security/logging-policy.md` §6–§8, ADR 0066-A §5).
//!
//! capture → preview → save, driven through the same `*_impl` functions the
//! Tauri commands call, against real synthetic vaults. Proves: the saved file is
//! byte-for-byte the preview; records after the preview never enter it; cancel,
//! lock and vault switch leave nothing savable; bad destinations and denied
//! writes produce fixed results and no file; nothing sensitive reaches the
//! bundle. Every operation is local — these tests make no network request.

use std::path::Path;
use std::sync::Arc;

use app_lib::ipc::commands::{
    create_vault_impl, create_vault_named_impl, diagnostics_discard_impl, diagnostics_preview_impl,
    diagnostics_save_impl, export_backup_impl, list_vaults_impl, lock_vault_impl,
    run_due_jobs_on_unlock_impl, unlock_vault_impl,
};
use app_lib::ipc::dto::DiagnosticsSaveResult;
use app_lib::ipc::IpcError;
use app_lib::vault_registry::VaultRegistry;
use app_lib::AppState;
use finance_kernel::{
    BackoffPolicy, CancellationToken, JobExecution, JobHandler, JobRecord, JobSpec, Kernel,
    Schedule, VaultController,
};
use observability::diagnostics::{parse_bundle, Metric, Outcome, Value};
use tempfile::TempDir;
use uuid::Uuid;

const PASSWORD: &str = "synthetic diagnostics password";

fn unlocked() -> (TempDir, AppState) {
    let dir = TempDir::new().expect("temp dir");
    let state = AppState::new(VaultController::open(dir.path().join("vault.db")));
    create_vault_impl(&state, PASSWORD.to_owned()).expect("create vault");
    (dir, state)
}

/// A successful manual backup: one `backup_outcome` record.
fn backup(state: &AppState, dir: &Path, name: &str) {
    export_backup_impl(state, dir.join(name).to_string_lossy().into_owned()).expect("backup");
}

fn out(dir: &Path, name: &str) -> String {
    dir.join(name).to_string_lossy().into_owned()
}

/// A folder OUTSIDE the vault and app-data directories — where a user's saved
/// bundle legitimately goes. (Saving into the vault's own folder is refused.)
fn outside() -> TempDir {
    TempDir::new().expect("output folder")
}

#[test]
fn the_saved_file_is_exactly_the_preview_and_later_records_never_enter_it() {
    let (dir, state) = unlocked();
    let saves = outside();
    backup(&state, dir.path(), "one.pcfobk");

    let preview = diagnostics_preview_impl(&state).expect("preview");
    assert_eq!(preview.records, 1);
    assert!(preview
        .suggested_file_name
        .starts_with("dohflow-diagnostics-"));
    assert!(preview.suggested_file_name.ends_with(".json"));

    // Captured AFTER the preview: must not appear in what gets saved.
    backup(&state, dir.path(), "two.pcfobk");

    let target = out(saves.path(), "diag.json");
    let result = diagnostics_save_impl(&state, preview.snapshot_id, target.clone()).expect("save");
    assert_eq!(result, DiagnosticsSaveResult::Saved);
    let written = std::fs::read(&target).expect("saved file");
    assert_eq!(
        written,
        preview.text.as_bytes(),
        "saved bytes == previewed bytes"
    );

    let bundle = parse_bundle(&written).expect("round-trips");
    assert_eq!(bundle.records.len(), 1);
    assert_eq!(bundle.records[0].metric, Metric::BackupOutcome);
    assert_eq!(bundle.records[0].value, Value::Outcome(Outcome::Success));

    // A save completes the preview: the same snapshot cannot be saved again.
    assert_eq!(
        diagnostics_save_impl(&state, preview.snapshot_id, out(saves.path(), "again.json"))
            .unwrap(),
        DiagnosticsSaveResult::PreviewExpired
    );
    // A fresh preview now includes the later record.
    assert_eq!(diagnostics_preview_impl(&state).unwrap().records, 2);
}

struct Complete;

impl JobHandler<Kernel> for Complete {
    fn kind(&self) -> &'static str {
        "diagnostics_test"
    }

    fn execute(&self, _: &JobRecord, _: &CancellationToken, _: &Kernel) -> JobExecution {
        JobExecution::Succeeded
    }
}

#[test]
fn durable_job_runs_are_captured_as_duration_buckets() {
    let (_dir, state) = unlocked();
    state
        .lock_controller()
        .unwrap()
        .kernel()
        .unwrap()
        .schedule_job(&JobSpec {
            id: Uuid::now_v7(),
            kind: "diagnostics_test".to_owned(),
            schedule: Schedule::Once,
            next_due_at: chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
            max_attempts: 1,
            backoff: BackoffPolicy::default(),
            enabled: true,
            requires_explicit_opt_in: false,
            payload_json: Some("opaque payload stays in the vault".to_owned()),
        })
        .unwrap();
    state.register_job_handler(Arc::new(Complete)).unwrap();
    let report =
        run_due_jobs_on_unlock_impl(&state, state.job_dispatcher().as_ref(), "diag-window")
            .expect("run jobs");
    assert_eq!(report.succeeded, 1);

    let preview = diagnostics_preview_impl(&state).unwrap();
    let bundle = parse_bundle(preview.text.as_bytes()).unwrap();
    assert!(
        bundle
            .records
            .iter()
            .any(|r| r.metric == Metric::JobDuration && matches!(r.value, Value::Duration(_))),
        "{}",
        preview.text
    );
    assert!(
        !preview.text.contains("opaque payload"),
        "job payloads never enter a bundle"
    );
    assert!(
        !preview.text.contains("diagnostics_test"),
        "job kinds are not captured as text"
    );
}

#[test]
fn cancelling_writes_nothing_and_the_preview_cannot_be_saved_afterwards() {
    let (_dir, state) = unlocked();
    let saves = outside();
    let preview = diagnostics_preview_impl(&state).unwrap();
    diagnostics_discard_impl(&state, preview.snapshot_id).unwrap();
    let target = out(saves.path(), "cancelled.json");
    assert_eq!(
        diagnostics_save_impl(&state, preview.snapshot_id, target.clone()).unwrap(),
        DiagnosticsSaveResult::PreviewExpired
    );
    assert!(
        !Path::new(&target).exists(),
        "a cancelled preview creates no file"
    );
}

#[test]
fn locking_the_vault_invalidates_the_pending_preview() {
    let (dir, state) = unlocked();
    let saves = outside();
    backup(&state, dir.path(), "b.pcfobk");
    let preview = diagnostics_preview_impl(&state).unwrap();

    lock_vault_impl(&state).unwrap();
    let target = out(saves.path(), "locked.json");
    assert!(matches!(
        diagnostics_save_impl(&state, preview.snapshot_id, target.clone()),
        Err(IpcError::VaultLocked)
    ));
    assert!(matches!(
        diagnostics_preview_impl(&state),
        Err(IpcError::VaultLocked)
    ));

    unlock_vault_impl(&state, PASSWORD.to_owned()).unwrap();
    assert_eq!(
        diagnostics_save_impl(&state, preview.snapshot_id, target.clone()).unwrap(),
        DiagnosticsSaveResult::PreviewExpired
    );
    assert!(!Path::new(&target).exists());
    assert_eq!(
        diagnostics_preview_impl(&state).unwrap().records,
        0,
        "a new session starts empty"
    );
}

#[test]
fn switching_vaults_invalidates_the_preview_and_starts_an_empty_session() {
    let root = TempDir::new().unwrap();
    let saves = outside();
    let registry = VaultRegistry::load(root.path());
    let path = registry
        .active_path(root.path())
        .unwrap_or_else(|| root.path().join("vault.db"));
    let state = AppState::with_registry(
        VaultController::open(path),
        registry,
        root.path().to_path_buf(),
    );
    create_vault_named_impl(&state, "First".into(), PASSWORD.into()).expect("first vault");
    backup(&state, root.path(), "first.pcfobk");
    let preview = diagnostics_preview_impl(&state).unwrap();
    assert_eq!(preview.records, 1);

    // Creating a second vault switches to it: the first vault is locked (its
    // kernel, capture and pending preview dropped) and the new one is unlocked.
    create_vault_named_impl(&state, "Second".into(), PASSWORD.into()).expect("switch to second");
    let target = out(saves.path(), "switched.json");
    assert_eq!(
        diagnostics_save_impl(&state, preview.snapshot_id, target.clone())
            .expect("second vault is unlocked"),
        DiagnosticsSaveResult::PreviewExpired
    );
    assert!(!Path::new(&target).exists());
    assert_eq!(
        diagnostics_preview_impl(&state)
            .expect("second vault preview")
            .records,
        0,
        "the first vault's capture never follows"
    );
}

#[test]
fn bad_destinations_write_nothing() {
    let (dir, state) = unlocked();
    let saves = outside();
    let preview = diagnostics_preview_impl(&state).unwrap();
    for bad in [
        "relative.json".to_owned(),
        out(dir.path(), "vault.db"),
        out(dir.path(), "backup.pcfobk"),
        out(dir.path(), "notes.txt"),
        dir.path()
            .join("missing-folder")
            .join("d.json")
            .to_string_lossy()
            .into_owned(),
    ] {
        assert_eq!(
            diagnostics_save_impl(&state, preview.snapshot_id, bad.clone()).unwrap(),
            DiagnosticsSaveResult::InvalidDestination,
            "{bad}"
        );
    }
    assert!(!dir.path().join("notes.txt").exists());
    // The preview is still pending after a rejected destination: the user can pick again.
    assert_eq!(
        diagnostics_save_impl(&state, preview.snapshot_id, out(saves.path(), "ok.json")).unwrap(),
        DiagnosticsSaveResult::Saved
    );
}

#[cfg(unix)]
#[test]
fn a_denied_write_is_a_fixed_result_with_no_file() {
    use std::os::unix::fs::PermissionsExt;
    let (_dir, state) = unlocked();
    let saves = outside();
    let read_only = saves.path().join("read-only");
    std::fs::create_dir(&read_only).unwrap();
    std::fs::set_permissions(&read_only, std::fs::Permissions::from_mode(0o555)).unwrap();
    let preview = diagnostics_preview_impl(&state).unwrap();
    let target = read_only.join("d.json").to_string_lossy().into_owned();
    let result = diagnostics_save_impl(&state, preview.snapshot_id, target.clone()).unwrap();
    std::fs::set_permissions(&read_only, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(result, DiagnosticsSaveResult::PermissionDenied);
    assert!(!Path::new(&target).exists());
}

#[test]
fn sensitive_inputs_never_reach_the_preview_or_the_saved_file() {
    let (dir, state) = unlocked();
    let saves = outside();
    // A failing backup whose path carries an email, an account-number sentinel
    // and a person's folder name.
    let hostile = dir
        .path()
        .join("jane.doe@example.com ACCT-SENTINEL-771")
        .join("absent")
        .join("x.pcfobk");
    let _ = export_backup_impl(&state, hostile.to_string_lossy().into_owned());
    // And a secret-bearing log line during the session: logs and the bundle are
    // separate, and a log line can never become a record.
    tracing::warn!("password=SENTINEL-PASS token=SENTINEL-TOKEN balance $98,765.43");

    let preview = diagnostics_preview_impl(&state).unwrap();
    let target = out(saves.path(), "diag.json");
    assert_eq!(
        diagnostics_save_impl(&state, preview.snapshot_id, target.clone()).unwrap(),
        DiagnosticsSaveResult::Saved
    );
    let saved = std::fs::read_to_string(&target).unwrap();
    for leak in [
        "jane.doe",
        "example.com",
        "ACCT-SENTINEL-771",
        "absent",
        "SENTINEL-PASS",
        "SENTINEL-TOKEN",
        "98,765",
        dir.path().to_string_lossy().as_ref(),
    ] {
        assert!(!preview.text.contains(leak), "preview leaked `{leak}`");
        assert!(!saved.contains(leak), "saved file leaked `{leak}`");
    }
    // The failure itself is there — as a fixed category.
    let bundle = parse_bundle(saved.as_bytes()).unwrap();
    assert!(bundle
        .records
        .iter()
        .any(|r| r.metric == Metric::BackupOutcome
            && matches!(r.value, Value::Outcome(Outcome::Failure(_)))));
}

#[cfg(unix)]
#[test]
fn a_save_can_never_land_inside_app_data_or_the_vault_folder() {
    // Review F1 (PR 52): `<data_dir>/vaults.json` is the vault registry. Writing a
    // bundle over it orphaned every named vault. No destination inside the
    // app-data root or the active vault's folder is allowed — directly, through
    // `..`, through a symlinked folder, or through a symlinked target file.
    use std::os::unix::fs::symlink;

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
    create_vault_named_impl(&state, "Household".into(), PASSWORD.into()).expect("named vault");
    let registry_file = root.path().join("vaults.json");
    let registry_before = std::fs::read(&registry_file).expect("registry exists");

    let saves = outside();
    std::fs::create_dir(root.path().join("sub")).unwrap();
    symlink(root.path(), saves.path().join("linked-root")).unwrap();
    symlink(&registry_file, saves.path().join("looks-harmless.json")).unwrap();
    let vault_folder = {
        let guard = state.lock_controller().unwrap();
        guard.path().parent().unwrap().to_path_buf()
    };

    let preview = diagnostics_preview_impl(&state).unwrap();
    for attempt in [
        out(root.path(), "vaults.json"),
        root.path()
            .join("sub")
            .join("..")
            .join("vaults.json")
            .to_string_lossy()
            .into_owned(),
        saves
            .path()
            .join("linked-root")
            .join("vaults.json")
            .to_string_lossy()
            .into_owned(),
        out(saves.path(), "looks-harmless.json"),
        out(&vault_folder, "diagnostics.json"),
        out(root.path(), "anything-else.json"),
    ] {
        assert_eq!(
            diagnostics_save_impl(&state, preview.snapshot_id, attempt.clone()).unwrap(),
            DiagnosticsSaveResult::InvalidDestination,
            "{attempt}"
        );
    }

    assert_eq!(
        std::fs::read(&registry_file).unwrap(),
        registry_before,
        "the registry is byte-identical"
    );
    let listed = list_vaults_impl(&state).expect("list vaults");
    assert!(
        listed.vaults.iter().any(|v| v.name == "Household"),
        "the vault is still registered: {listed:?}"
    );
    assert!(!vault_folder.join("diagnostics.json").exists());
    assert!(!root.path().join("anything-else.json").exists());

    // A normal destination outside app data still works with the same preview.
    assert_eq!(
        diagnostics_save_impl(&state, preview.snapshot_id, out(saves.path(), "ok.json")).unwrap(),
        DiagnosticsSaveResult::Saved
    );
}
