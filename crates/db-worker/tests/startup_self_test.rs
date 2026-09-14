//! Startup self-test: the writer connection is configured per policy (plan §3.2).

use db_worker::DbWorker;
use tempfile::TempDir;

#[test]
fn self_test_reports_wal_and_configured_limits() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("vault.db");
    let worker = DbWorker::open(&path, "correct horse battery staple").unwrap();

    let report = worker.self_test().unwrap();
    assert!(
        report.journal_mode.eq_ignore_ascii_case("wal"),
        "journal_mode was {}",
        report.journal_mode
    );
    assert!(report.busy_timeout_ms >= 1000, "busy_timeout too low");
    assert_eq!(report.journal_size_limit, 64 * 1024 * 1024);
}
