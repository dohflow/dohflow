//! personal-cfo-x6dr AC: adapter id + version ride on every sync's output so
//! the ingestion pipeline can record them on the `source_batch` / `parser_run`
//! rows it creates (plan §8.1.3) — exactly as importer plugin provenance is
//! recorded today. This crate cannot touch the DB (compile barrier), so the
//! contract tested here is the provenance-stamped envelope the pipeline
//! persists; the row-level half is owned by personal-cfo-w3gh's AC.

use chrono::NaiveDate;
use connector_core::mock::MockConnector;
use connector_core::{Connection, ConnectorAdapter, Credential, SyncBatch};

#[test]
fn sync_output_is_stamped_with_adapter_id_and_version() {
    let adapter = MockConnector::with_fixture();
    let conn = Connection {
        credential: Credential::new("mock-access-url"),
    };
    let since = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
    let synced = adapter.sync(&conn, Some(since)).unwrap();

    assert_eq!(synced.adapter_id, "mock");
    assert_eq!(synced.adapter_version, "0.1.0");
    assert_eq!(
        synced.adapter_version,
        adapter.version().to_string(),
        "the stamp must be the adapter's own version, not a copy that can drift"
    );
    // source_format must be a bare `source_batches.source_type` schema token
    // (the CHECK constraint reserves 'simplefin'/'teller'/'plaid'/…), so the
    // default sync emits the adapter id verbatim — never a prefixed variant.
    assert_eq!(synced.batch.source_format, "mock");
}

#[test]
fn provenance_survives_the_serde_round_trip_the_pipeline_persists() {
    let adapter = MockConnector::with_fixture();
    let conn = Connection {
        credential: Credential::new("mock-access-url"),
    };
    let synced = adapter.sync(&conn, None).unwrap();
    assert_eq!(
        synced.since, None,
        "first sync: full history, no lower bound"
    );

    let json = serde_json::to_string(&synced).unwrap();
    let back: SyncBatch = serde_json::from_str(&json).unwrap();
    assert_eq!(synced, back);
    assert_eq!(back.adapter_id, "mock");
    assert_eq!(back.adapter_version, "0.1.0");
}

#[test]
fn sync_output_never_contains_the_credential() {
    // §6.6 / DoD §2.2 at this layer: the staged output the pipeline persists
    // must not embed the user's secret. `Credential` has no serde support, so
    // leaking requires deliberately copying the string — assert none did, in
    // both the serialized form and the Debug rendering.
    let adapter = MockConnector::with_fixture();
    let secret = "CFO-CANARY-9f2d7c1e";
    let conn = Connection {
        credential: Credential::new(secret),
    };
    let synced = adapter.sync(&conn, None).unwrap();

    let json = serde_json::to_string(&synced).unwrap();
    assert!(
        !json.contains(secret),
        "credential leaked into persisted sync output"
    );
    let debug = format!("{synced:?}");
    assert!(
        !debug.contains(secret),
        "credential leaked into Debug rendering"
    );
}
