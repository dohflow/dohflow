//! The carried-column lists, pinned against the column manifest (`table_columns.toml`) and against
//! what the part encoder actually writes (vault-format.md C.9, `personal-cfo-0g528.2`): **a column
//! is in the artifact if and only if production code sets it at INSERT and never UPDATEs it**, and
//! the rest is device-local state no replica reads. A new column on a batch table fails this test
//! until it has been classified in `sync_core::source_batch`, so it cannot silently stay out of an
//! artifact (a replica would then lack it) or silently go in (a replica would read device state).

use std::collections::BTreeSet;

use sync_core::source_batch::{
    BatchRow, RecordRow, RunRow, SourceBatch, BATCH_COLUMNS, BATCH_LOCAL_COLUMNS, RECORD_COLUMNS,
    RUN_COLUMNS,
};

const TABLE_COLUMNS: &str = include_str!("../table_columns.toml");

/// The column names the manifest lists for `table`, without their kinds.
fn manifest_columns(table: &str) -> BTreeSet<String> {
    let block = TABLE_COLUMNS
        .split("[[table]]")
        .find(|block| {
            block
                .lines()
                .any(|l| l.trim() == format!("name = \"{table}\""))
        })
        .unwrap_or_else(|| panic!("{table} is not in table_columns.toml"));
    let line = block
        .lines()
        .find_map(|l| l.strip_prefix("columns = ["))
        .unwrap_or_else(|| panic!("{table} has no columns"));
    line.trim_end_matches(']')
        .split(',')
        .map(|entry| {
            entry
                .trim()
                .trim_matches('"')
                .split(':')
                .next()
                .unwrap()
                .to_owned()
        })
        .collect()
}

fn set(columns: &[&str]) -> BTreeSet<String> {
    columns.iter().map(|c| (*c).to_owned()).collect()
}

#[test]
fn every_manifest_column_of_a_batch_table_is_carried_or_device_local() {
    let carried = set(BATCH_COLUMNS);
    let local = set(BATCH_LOCAL_COLUMNS);
    assert!(
        carried.is_disjoint(&local),
        "a column is carried or device-local, never both"
    );
    let classified: BTreeSet<String> = carried.union(&local).cloned().collect();
    assert_eq!(
        classified,
        manifest_columns("source_batches"),
        "source_batches: a new column must be classified in sync_core::source_batch"
    );
    // A source record and a parser run carry every column.
    assert_eq!(set(RECORD_COLUMNS), manifest_columns("source_records"));
    assert_eq!(set(RUN_COLUMNS), manifest_columns("parser_runs"));
}

fn sample() -> SourceBatch {
    let id = [0xb1; 16];
    SourceBatch {
        batch: BatchRow {
            id,
            source_type: "csv".to_owned(),
            source_name: Some("statement.csv".to_owned()),
            file_fingerprint: Some("fp".to_owned()),
            parser_version: Some("1".to_owned()),
            summary_json: Some("{}".to_owned()),
            imported_at: Some("2026-11-01T00:00:00Z".to_owned()),
            created_at: "2026-11-01T00:00:00Z".to_owned(),
            connector_connection_id: Some([0xcc; 16]),
        },
        runs: vec![RunRow {
            id: [0xc1; 16],
            source_batch_id: id,
            parser_name: "csv".to_owned(),
            parser_version: "1".to_owned(),
            bytes_in: 10,
            records_out: 1,
            status: "ok".to_owned(),
            limit_hit: Some("rows".to_owned()),
            started_at: "2026-11-01T00:00:00Z".to_owned(),
            finished_at: Some("2026-11-01T00:00:01Z".to_owned()),
        }],
        records: vec![RecordRow {
            id: [1; 16],
            source_batch_id: id,
            external_id: Some("ext".to_owned()),
            source_hash: "h".to_owned(),
            normalized_json: "{}".to_owned(),
            parse_confidence_bps: Some(9000),
            created_at: "2026-11-01T00:00:00Z".to_owned(),
        }],
    }
}

fn keys(value: &serde_json::Value) -> BTreeSet<String> {
    value.as_object().unwrap().keys().cloned().collect()
}

#[test]
fn the_part_encoder_writes_exactly_the_carried_columns() {
    let parts = sample().split().unwrap();
    let part0: serde_json::Value = serde_json::from_slice(&parts[0].blob).unwrap();
    assert_eq!(
        keys(&part0["batch"]),
        set(BATCH_COLUMNS),
        "the batch object carries exactly the carried columns, with every nullable one present"
    );
    assert!(
        BATCH_LOCAL_COLUMNS
            .iter()
            .all(|local| part0["batch"].get(local).is_none()),
        "no device-local column reaches an artifact"
    );
    assert_eq!(keys(&part0["parser_runs"][0]), set(RUN_COLUMNS));
    assert_eq!(keys(&part0["records"][0]), set(RECORD_COLUMNS));
}
