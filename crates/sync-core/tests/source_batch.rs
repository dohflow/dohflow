//! A source batch as artifact parts (vault-format.md C.9, personal-cfo-0g528.2 S2c): the deterministic
//! split, the staging bounds, the part bound at its edge and one past it, and the strict reverse.

use sync_core::artifact::{MAX_BATCH_HEADER, MAX_PART_BLOB, MAX_SOURCE_RECORD};
use sync_core::source_batch::{
    decode_part, header_len, join, record_ids, record_json, BatchRow, RecordRow, RunRow,
    SourceBatch, SourceBatchError, BATCH_COLUMNS, BATCH_LOCAL_COLUMNS, RECORD_COLUMNS, RUN_COLUMNS,
};

const BATCH: [u8; 16] = [0xb1; 16];

fn id(n: u32) -> [u8; 16] {
    let mut out = [0_u8; 16];
    out[12..].copy_from_slice(&n.to_be_bytes());
    out
}

fn batch() -> BatchRow {
    BatchRow {
        id: BATCH,
        source_type: "csv".into(),
        source_name: Some("june.csv".into()),
        file_fingerprint: Some("fp-file".into()),
        parser_version: Some("1.0.0".into()),
        summary_json: None,
        imported_at: None,
        created_at: "2026-06-01T00:00:00Z".into(),
        connector_connection_id: None,
    }
}

fn run(n: u32) -> RunRow {
    RunRow {
        id: id(1_000 + n),
        source_batch_id: BATCH,
        parser_name: "csv".into(),
        parser_version: "1.0.0".into(),
        bytes_in: 4_096,
        records_out: 10,
        status: "ok".into(),
        limit_hit: None,
        started_at: "2026-06-01T00:00:01Z".into(),
        finished_at: Some("2026-06-01T00:00:02Z".into()),
    }
}

fn record(n: u32, padding: usize) -> RecordRow {
    RecordRow {
        id: id(n),
        source_batch_id: BATCH,
        external_id: Some(format!("ext-{n}")),
        source_hash: format!("sha256:row-{n}"),
        normalized_json: "x".repeat(padding),
        parse_confidence_bps: Some(10_000),
        created_at: "2026-06-01T00:00:03Z".into(),
    }
}

fn small(records: u32) -> SourceBatch {
    SourceBatch {
        batch: batch(),
        runs: vec![run(1)],
        records: (1..=records).map(|n| record(n, 20)).collect(),
    }
}

fn parts_as_input(parts: &[sync_core::source_batch::Part]) -> Vec<(u32, u32, &[u8])> {
    parts
        .iter()
        .map(|p| (p.index, p.count, p.blob.as_slice()))
        .collect()
}

// ---- the shape ---------------------------------------------------------------------------------

#[test]
fn the_column_lists_are_the_insert_only_columns_and_the_device_local_ones() {
    let mut all: Vec<&str> = BATCH_COLUMNS
        .iter()
        .chain(BATCH_LOCAL_COLUMNS)
        .copied()
        .collect();
    all.sort_unstable();
    let mut expected = vec![
        "id",
        "source_type",
        "source_name",
        "file_fingerprint",
        "parser_version",
        "status",
        "staged_count",
        "committed_count",
        "skipped_count",
        "summary_json",
        "imported_at",
        "created_at",
        "updated_at",
        "connector_connection_id",
    ];
    expected.sort_unstable();
    assert_eq!(
        all, expected,
        "every source_batches column is carried or device-local"
    );
    assert_eq!(RECORD_COLUMNS.len(), 7);
    assert_eq!(RUN_COLUMNS.len(), 10);
}

#[test]
fn a_small_batch_is_one_part_that_joins_back_to_the_same_rows() {
    let b = small(3);
    let parts = b.split().unwrap();
    assert_eq!(parts.len(), 1);
    assert_eq!((parts[0].index, parts[0].count), (0, 1));
    assert!(parts[0].blob.starts_with(br#"{"batch":{"#));
    assert!(parts[0].blob.ends_with(b"]}"));
    let joined = join(&BATCH, &parts_as_input(&parts)).unwrap();
    assert_eq!(joined, b);
    // The part's metadata names the batch, the count and the index.
    assert_eq!(
        std::str::from_utf8(&parts[0].logical_metadata).unwrap(),
        r#"{"batch_id":"b1b1b1b1-b1b1-b1b1-b1b1-b1b1b1b1b1b1","part_count":"1","part_index":"0"}"#
    );
}

#[test]
fn the_split_depends_on_the_rows_alone_not_on_their_order() {
    let ordered = small(8);
    let mut shuffled = ordered.clone();
    shuffled.records.reverse();
    shuffled.runs.reverse();
    assert_eq!(ordered.split().unwrap(), shuffled.split().unwrap());
    // A different row is a different part.
    let mut changed = ordered.clone();
    changed.records[3].source_hash = "sha256:other".into();
    assert_ne!(ordered.split().unwrap(), changed.split().unwrap());
}

#[test]
fn nulls_and_integers_survive_the_round_trip() {
    let mut b = small(2);
    b.records[0].external_id = None;
    b.records[0].parse_confidence_bps = None;
    b.records[1].parse_confidence_bps = Some(-7);
    b.batch.connector_connection_id = Some(id(99));
    b.batch.source_name = None;
    b.runs[0].limit_hit = Some("rows".into());
    b.runs[0].finished_at = None;
    b.runs[0].bytes_in = i64::MAX;
    let parts = b.split().unwrap();
    assert_eq!(join(&BATCH, &parts_as_input(&parts)).unwrap(), b);
}

// ---- the staging bounds ---------------------------------------------------------------------------

fn record_with_json_len(n: u32, target: usize) -> RecordRow {
    let base = record_json(&record(n, 0)).unwrap().len();
    record(n, target - base)
}

#[test]
fn a_record_at_the_record_bound_is_carried_and_one_past_it_is_refused() {
    let at = record_with_json_len(1, MAX_SOURCE_RECORD);
    assert_eq!(record_json(&at).unwrap().len(), MAX_SOURCE_RECORD);
    let mut b = small(0);
    b.records = vec![at];
    let parts = b.split().unwrap();
    assert_eq!(
        parts.len(),
        1,
        "the biggest permitted record fits part 0 with a full header"
    );
    assert_eq!(join(&BATCH, &parts_as_input(&parts)).unwrap(), b);
    let mut over = record_with_json_len(1, MAX_SOURCE_RECORD);
    over.normalized_json.push('x');
    assert_eq!(
        record_json(&over),
        Err(SourceBatchError::SourceRecordTooLarge)
    );
    b.records = vec![over];
    assert_eq!(b.split(), Err(SourceBatchError::SourceRecordTooLarge));
}

#[test]
fn a_header_at_the_header_bound_is_carried_and_one_past_it_is_refused() {
    let mut b = small(1);
    // Pad the one run until the header is exactly at the bound.
    let base = header_len(&b.batch, &b.runs).unwrap();
    b.runs[0].parser_name = "p".repeat(MAX_BATCH_HEADER - base + 3);
    assert_eq!(header_len(&b.batch, &b.runs), Ok(MAX_BATCH_HEADER));
    let parts = b.split().unwrap();
    assert!(parts.iter().all(|p| p.blob.len() <= MAX_PART_BLOB));
    b.runs[0].parser_name.push('p');
    assert_eq!(
        header_len(&b.batch, &b.runs),
        Err(SourceBatchError::SourceBatchHeaderTooLarge)
    );
    assert_eq!(b.split(), Err(SourceBatchError::SourceBatchHeaderTooLarge));
}

#[test]
fn the_largest_header_and_the_largest_record_still_fit_part_zero() {
    let mut b = small(0);
    let base = header_len(&b.batch, &b.runs).unwrap();
    b.runs[0].parser_name = "p".repeat(MAX_BATCH_HEADER - base + 3);
    b.records = vec![record_with_json_len(1, MAX_SOURCE_RECORD)];
    let parts = b.split().unwrap();
    assert_eq!(parts.len(), 1);
    assert!(
        parts[0].blob.len() <= MAX_PART_BLOB,
        "{}",
        parts[0].blob.len()
    );
    assert_eq!(join(&BATCH, &parts_as_input(&parts)).unwrap(), b);
}

// ---- the part bound ----------------------------------------------------------------------------------

/// A batch whose single part is exactly `target` bytes, built by padding the last record.
fn batch_with_one_part_of(target: usize) -> SourceBatch {
    let mut b = SourceBatch {
        batch: batch(),
        runs: vec![run(1)],
        records: (1..=5).map(|n| record(n, 800_000)).collect(),
    };
    let current = b.split().unwrap();
    assert_eq!(current.len(), 1);
    let pad = target - current[0].blob.len();
    b.records[4].normalized_json.push_str(&"x".repeat(pad));
    b
}

#[test]
fn a_batch_whose_part_is_exactly_at_the_bound_is_one_part_and_one_byte_more_is_two() {
    let at = batch_with_one_part_of(MAX_PART_BLOB);
    let parts = at.split().unwrap();
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].blob.len(), MAX_PART_BLOB);
    assert_eq!(join(&BATCH, &parts_as_input(&parts)).unwrap(), at);

    let mut past = at.clone();
    past.records[4].normalized_json.push('x');
    let parts = past.split().unwrap();
    assert_eq!(
        parts.len(),
        2,
        "one byte past the bound opens a second part"
    );
    assert!(parts.iter().all(|p| p.blob.len() <= MAX_PART_BLOB));
    assert_eq!((parts[0].index, parts[0].count), (0, 2));
    assert_eq!((parts[1].index, parts[1].count), (1, 2));
    // Nothing was dropped or truncated.
    assert_eq!(join(&BATCH, &parts_as_input(&parts)).unwrap(), past);
    assert!(parts[1].blob.starts_with(br#"{"records":["#));
}

#[test]
fn a_big_batch_is_carried_whole_in_ordered_parts_each_within_the_bound() {
    let b = SourceBatch {
        batch: batch(),
        runs: vec![run(1), run(2)],
        records: (1..=2_000).map(|n| record(n, 5_000)).collect(),
    };
    let parts = b.split().unwrap();
    assert!(parts.len() >= 3, "{} parts", parts.len());
    for (i, part) in parts.iter().enumerate() {
        assert_eq!(part.index as usize, i);
        assert_eq!(part.count as usize, parts.len());
        assert!(part.blob.len() <= MAX_PART_BLOB);
    }
    // Records are in ascending id order across the parts, each in exactly one part.
    let mut seen = Vec::new();
    for part in &parts {
        seen.extend(record_ids(&BATCH, part.index, &part.blob).unwrap());
    }
    assert_eq!(seen.len(), 2_000);
    assert!(seen.windows(2).all(|w| w[0] < w[1]));
    assert_eq!(join(&BATCH, &parts_as_input(&parts)).unwrap(), b);
    // The input order of the parts does not matter to the join.
    let mut reversed = parts_as_input(&parts);
    reversed.reverse();
    assert_eq!(join(&BATCH, &reversed).unwrap(), b);
}

// ---- refusals on the way in ------------------------------------------------------------------------------

#[test]
fn rows_of_another_batch_and_duplicate_ids_are_refused_before_anything_is_split() {
    let mut other_record = small(2);
    other_record.records[1].source_batch_id = [0xee; 16];
    assert_eq!(
        other_record.split(),
        Err(SourceBatchError::RowOfAnotherBatch)
    );
    let mut other_run = small(1);
    other_run.runs[0].source_batch_id = [0xee; 16];
    assert_eq!(other_run.split(), Err(SourceBatchError::RowOfAnotherBatch));
    let mut dup_record = small(2);
    dup_record.records[1].id = dup_record.records[0].id;
    assert_eq!(dup_record.split(), Err(SourceBatchError::DuplicateRow));
    let mut dup_run = small(1);
    dup_run.runs.push(run(1));
    assert_eq!(dup_run.split(), Err(SourceBatchError::DuplicateRow));
    // A text that cannot be written canonically (U+0000) is malformed, not truncated.
    let mut nul = small(1);
    nul.records[0].normalized_json = "a\0b".into();
    assert_eq!(nul.split(), Err(SourceBatchError::Malformed));
}

// ---- the strict reverse ----------------------------------------------------------------------------------

#[test]
fn parts_that_are_missing_duplicated_or_disagreeing_do_not_join() {
    let b = batch_with_one_part_of(MAX_PART_BLOB);
    let mut past = b.clone();
    past.records[4].normalized_json.push('x');
    let parts = past.split().unwrap();
    let input = parts_as_input(&parts);
    // A gap.
    assert_eq!(join(&BATCH, &input[1..]), Err(SourceBatchError::Incomplete));
    assert_eq!(join(&BATCH, &input[..1]), Err(SourceBatchError::Incomplete));
    // A duplicate index.
    let dup = vec![input[0], input[0]];
    assert_eq!(join(&BATCH, &dup), Err(SourceBatchError::Incomplete));
    // Disagreeing counts.
    let disagree = vec![(0, 2, input[0].2), (1, 3, input[1].2)];
    assert_eq!(join(&BATCH, &disagree), Err(SourceBatchError::Incomplete));
    // No parts, and a zero count.
    assert_eq!(join(&BATCH, &[]), Err(SourceBatchError::Incomplete));
    assert_eq!(
        join(&BATCH, &[(0, 0, input[0].2)]),
        Err(SourceBatchError::Incomplete)
    );
    // Another batch's id.
    assert_eq!(
        join(&[0xee; 16], &input),
        Err(SourceBatchError::RowOfAnotherBatch)
    );
}

#[test]
fn a_part_with_the_wrong_shape_or_extra_members_is_malformed() {
    let parts = small(2).split().unwrap();
    // Part 0 must hold the batch; a later part must not.
    assert_eq!(
        decode_part(&BATCH, 1, &parts[0].blob),
        Err(SourceBatchError::Malformed)
    );
    assert_eq!(
        decode_part(&BATCH, 0, br#"{"records":[]}"#),
        Err(SourceBatchError::Malformed)
    );
    // Not canonical, not an object, an unknown member.
    assert_eq!(
        decode_part(&BATCH, 1, b"[]"),
        Err(SourceBatchError::Malformed)
    );
    assert_eq!(
        decode_part(&BATCH, 1, br#"{"records": []}"#),
        Err(SourceBatchError::Malformed)
    );
    assert_eq!(
        decode_part(&BATCH, 1, br#"{"extra":[],"records":[]}"#),
        Err(SourceBatchError::Malformed)
    );
    // A record with a column missing, or a value of the wrong type.
    let good = record_json(&record(1, 0)).unwrap();
    let text = String::from_utf8(good).unwrap();
    let missing = text.replace(r#","source_hash":"sha256:row-1""#, "");
    assert_eq!(
        decode_part(
            &BATCH,
            1,
            format!(r#"{{"records":[{missing}]}}"#).as_bytes()
        ),
        Err(SourceBatchError::Malformed)
    );
    let wrong_type = text.replace(
        r#""parse_confidence_bps":"10000""#,
        r#""parse_confidence_bps":"1e4""#,
    );
    assert_eq!(
        decode_part(
            &BATCH,
            1,
            format!(r#"{{"records":[{wrong_type}]}}"#).as_bytes()
        ),
        Err(SourceBatchError::Malformed)
    );
    // A leading zero is not a canonical integer.
    let zero = text.replace(
        r#""parse_confidence_bps":"10000""#,
        r#""parse_confidence_bps":"010000""#,
    );
    assert_eq!(
        decode_part(&BATCH, 1, format!(r#"{{"records":[{zero}]}}"#).as_bytes()),
        Err(SourceBatchError::Malformed)
    );
}

#[test]
fn records_out_of_order_or_repeated_in_a_part_are_refused() {
    let a = String::from_utf8(record_json(&record(1, 0)).unwrap()).unwrap();
    let b = String::from_utf8(record_json(&record(2, 0)).unwrap()).unwrap();
    assert!(decode_part(&BATCH, 1, format!(r#"{{"records":[{a},{b}]}}"#).as_bytes()).is_ok());
    assert_eq!(
        decode_part(&BATCH, 1, format!(r#"{{"records":[{b},{a}]}}"#).as_bytes()),
        Err(SourceBatchError::DuplicateRow)
    );
    assert_eq!(
        decode_part(&BATCH, 1, format!(r#"{{"records":[{a},{a}]}}"#).as_bytes()),
        Err(SourceBatchError::DuplicateRow)
    );
    // The same record in two parts is a duplicate on join.
    let part0 = small(1).split().unwrap().remove(0);
    let part1 = format!(
        r#"{{"records":[{}]}}"#,
        String::from_utf8(record_json(&record(1, 20)).unwrap()).unwrap()
    );
    assert_eq!(
        join(
            &BATCH,
            &[(0, 2, part0.blob.as_slice()), (1, 2, part1.as_bytes())]
        ),
        Err(SourceBatchError::DuplicateRow)
    );
}

#[test]
fn part_zero_names_its_own_batch_even_with_no_rows_to_betray_it() {
    let mut empty = small(0);
    empty.runs.clear();
    let parts = empty.split().unwrap();
    assert!(decode_part(&BATCH, 0, &parts[0].blob).is_ok());
    assert_eq!(
        decode_part(&[0xee; 16], 0, &parts[0].blob),
        Err(SourceBatchError::RowOfAnotherBatch)
    );
}
