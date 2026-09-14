//! Health-check accessors used by the vault state machine (personal-cfo-tg5):
//! `schema_version` and `read_models_current`.

use db_worker::DbWorker;

const KEY: &str = "correct horse battery staple";

#[test]
fn schema_version_matches_the_seeded_vault_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let worker = DbWorker::open(dir.path().join("vault.db"), KEY).unwrap();
    // The migrated schema version and the version stamped into vault_metadata
    // agree — the coherence the vault health check asserts.
    assert!(worker.schema_version() > 0);
    assert_eq!(
        worker.schema_version(),
        worker.vault_metadata().unwrap().schema_version
    );
}

/// personal-cfo-4d8.27.1.3: a vault created under an older build has a stale
/// write-once `vault_metadata.schema_version` stamp — migrations advance
/// `PRAGMA user_version` + the `schema_migrations` tracker but never re-stamp this
/// row. Reopening must advance the stamp to the migrated version so the vault
/// health check reads as coherent (before the fix it reported "schema version is
/// off" on every open of a pre-upgrade vault).
#[test]
fn reopening_advances_a_stale_schema_stamp() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.db");

    let current = {
        let worker = DbWorker::open(path.clone(), KEY).unwrap();
        let current = worker.schema_version();
        // Simulate a pre-upgrade vault: the stamp lags the migrated schema.
        let conn = worker.read_connection().unwrap();
        conn.execute(
            "UPDATE vault_metadata SET schema_version = ?1 WHERE singleton = 1",
            [current - 1],
        )
        .unwrap();
        let stamped: i64 = conn
            .query_row(
                "SELECT schema_version FROM vault_metadata WHERE singleton = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stamped, current - 1, "precondition: the stamp is stale");
        current
    }; // drop the worker + connection so the file is released before reopening

    let reopened = DbWorker::open(path, KEY).unwrap();
    assert_eq!(reopened.schema_version(), current);
    assert_eq!(
        reopened.vault_metadata().unwrap().schema_version,
        current,
        "reopen must re-stamp vault_metadata to the migrated schema version"
    );
}

/// The re-stamp only moves the version FORWARD: a stamp that is already newer than
/// the build's schema (a vault created by a newer DohFlow, opened here by an
/// older one) must be left intact so the health check still surfaces that genuine
/// mismatch instead of masking it (personal-cfo-4d8.27.1.3).
#[test]
fn reopening_does_not_regress_a_newer_schema_stamp() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.db");

    let current = {
        let worker = DbWorker::open(path.clone(), KEY).unwrap();
        let current = worker.schema_version();
        worker
            .read_connection()
            .unwrap()
            .execute(
                "UPDATE vault_metadata SET schema_version = ?1 WHERE singleton = 1",
                [current + 1],
            )
            .unwrap();
        current
    };

    let reopened = DbWorker::open(path, KEY).unwrap();
    assert_eq!(
        reopened.vault_metadata().unwrap().schema_version,
        current + 1,
        "a newer stamp must not be regressed to the older build's schema version"
    );
}

#[test]
fn read_models_current_is_true_on_a_fresh_then_rebuilt_vault() {
    let dir = tempfile::tempdir().unwrap();
    let worker = DbWorker::open(dir.path().join("vault.db"), KEY).unwrap();

    // No checksum recorded yet → treated as current (rebuilds on demand).
    assert!(worker.read_models_current().unwrap());

    // After a rebuild, the stored checksum matches a fresh compute.
    worker.rebuild_transaction_display().unwrap();
    assert!(worker.read_models_current().unwrap());
}

#[test]
fn read_models_current_detects_injected_drift() {
    let dir = tempfile::tempdir().unwrap();
    let worker = DbWorker::open(dir.path().join("vault.db"), KEY).unwrap();
    worker.rebuild_transaction_display().unwrap();
    assert!(worker.read_models_current().unwrap());

    // Corrupt the stored authoritative checksum so it no longer matches a fresh
    // compute. (We mutate the checksum row, not the read-model table, which is
    // write-protected.) The health check must report drift, not silent health.
    let conn = worker.read_connection().unwrap();
    conn.execute(
        "UPDATE read_model_checksums SET current_checksum = current_checksum + 999 \
         WHERE read_model_name = 'transaction_display'",
        [],
    )
    .unwrap();

    assert!(!worker.read_models_current().unwrap());
}
