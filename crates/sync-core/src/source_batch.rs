//! A source batch as a class-1b artifact (`docs/architecture/vault-format.md` C.9, "Source batch
//! parts"; personal-cfo-0g528.2, part S2c): the rows a part carries, the deterministic split of a
//! batch into parts, and the strict reverse.
//!
//! A batch is **one** artifact, carried as ordered immutable parts: part 0 holds the batch's own
//! columns and every parser run, then records, and later parts hold records only. The inputs are
//! bounded where they enter the vault ([`MAX_SOURCE_RECORD`], [`MAX_BATCH_HEADER`]), so every part
//! is at most [`MAX_PART_BLOB`] bytes with no exception and nothing that reached the vault is
//! refused at publish. The split is a function of the rows alone: the same rows always give the
//! same parts, so a device that rebuilds a batch from its rows reproduces the bytes it published
//! (and falls back to a fetch if it does not, which the fixed-plaintext digest of C.9 detects).
//!
//! A column is in an artifact if and only if production code sets it at INSERT and never UPDATEs
//! it (C.9): [`BATCH_COLUMNS`] omits `status`, the counts and `updated_at`. The lists are pinned
//! against `table_columns.toml` by a test in `db-worker`, so a new column must say which side it
//! is on.
//!
//! Row values use the B.3 encoding inside canonical JSON (C.1.2): a `uuid` is lowercase
//! hyphenated text, an integer is a decimal string, text is text, and NULL is `null`.

use std::collections::BTreeMap;

use thiserror::Error;

use crate::artifact::{MAX_BATCH_HEADER, MAX_PART_BLOB, MAX_SOURCE_RECORD};
use crate::canonical_json::{node_to_canonical_vec, parse_canonical_node, Node};

/// The `source_batches` columns an artifact carries (INSERT-only, C.9), in manifest order.
pub const BATCH_COLUMNS: &[&str] = &[
    "id",
    "source_type",
    "source_name",
    "file_fingerprint",
    "parser_version",
    "summary_json",
    "imported_at",
    "created_at",
    "connector_connection_id",
];
/// The `source_batches` columns that stay device-local (workflow state no replica reads).
pub const BATCH_LOCAL_COLUMNS: &[&str] = &[
    "status",
    "staged_count",
    "committed_count",
    "skipped_count",
    "updated_at",
];
/// The `source_records` columns an artifact carries: all of them.
pub const RECORD_COLUMNS: &[&str] = &[
    "id",
    "source_batch_id",
    "external_id",
    "source_hash",
    "normalized_json",
    "parse_confidence_bps",
    "created_at",
];
/// The `parser_runs` columns an artifact carries: all of them.
pub const RUN_COLUMNS: &[&str] = &[
    "id",
    "source_batch_id",
    "parser_name",
    "parser_version",
    "bytes_in",
    "records_out",
    "status",
    "limit_hit",
    "started_at",
    "finished_at",
];

/// Why a batch could not be split into parts, or parts could not be joined into a batch. No variant
/// carries a value from a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum SourceBatchError {
    /// A record's canonical JSON is over [`MAX_SOURCE_RECORD`]: refused at staging, before the row
    /// enters the vault.
    #[error("a source record is over the record bound")]
    SourceRecordTooLarge,
    /// The batch's columns plus all its parser runs are over [`MAX_BATCH_HEADER`]: the parser run
    /// that would pass it is refused before it enters the vault.
    #[error("a source batch's columns and parser runs are over the header bound")]
    SourceBatchHeaderTooLarge,
    /// A record or a run names another batch.
    #[error("a row belongs to another batch")]
    RowOfAnotherBatch,
    /// Two records, or two runs, share an id.
    #[error("two rows share an id")]
    DuplicateRow,
    /// A part is not a well-formed source batch part.
    #[error("the part is malformed")]
    Malformed,
    /// The parts are not exactly `0` to `part_count - 1` of one batch.
    #[error("the parts of the batch are incomplete or inconsistent")]
    Incomplete,
}

/// A `source_batches` row's carried columns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchRow {
    /// `id`.
    pub id: [u8; 16],
    /// `source_type`.
    pub source_type: String,
    /// `source_name`.
    pub source_name: Option<String>,
    /// `file_fingerprint`.
    pub file_fingerprint: Option<String>,
    /// `parser_version`.
    pub parser_version: Option<String>,
    /// `summary_json`.
    pub summary_json: Option<String>,
    /// `imported_at`.
    pub imported_at: Option<String>,
    /// `created_at`.
    pub created_at: String,
    /// `connector_connection_id`.
    pub connector_connection_id: Option<[u8; 16]>,
}

/// A `source_records` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordRow {
    /// `id`.
    pub id: [u8; 16],
    /// `source_batch_id`.
    pub source_batch_id: [u8; 16],
    /// `external_id`.
    pub external_id: Option<String>,
    /// `source_hash`.
    pub source_hash: String,
    /// `normalized_json`.
    pub normalized_json: String,
    /// `parse_confidence_bps`.
    pub parse_confidence_bps: Option<i64>,
    /// `created_at`.
    pub created_at: String,
}

/// A `parser_runs` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRow {
    /// `id`.
    pub id: [u8; 16],
    /// `source_batch_id`.
    pub source_batch_id: [u8; 16],
    /// `parser_name`.
    pub parser_name: String,
    /// `parser_version`.
    pub parser_version: String,
    /// `bytes_in`.
    pub bytes_in: i64,
    /// `records_out`.
    pub records_out: i64,
    /// `status`.
    pub status: String,
    /// `limit_hit`.
    pub limit_hit: Option<String>,
    /// `started_at`.
    pub started_at: String,
    /// `finished_at`.
    pub finished_at: Option<String>,
}

/// A whole source batch: its carried columns, its parser runs and its records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceBatch {
    /// The batch's carried columns.
    pub batch: BatchRow,
    /// Its parser runs.
    pub runs: Vec<RunRow>,
    /// Its records.
    pub records: Vec<RecordRow>,
}

/// One part of a batch, ready to become an artifact: its `logical_metadata` and its blob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    /// `0` to `count - 1`.
    pub index: u32,
    /// How many parts the batch has.
    pub count: u32,
    /// Canonical JSON: `{"batch_id", "part_count", "part_index"}`.
    pub logical_metadata: Vec<u8>,
    /// Canonical JSON: part 0 `{"batch", "parser_runs", "records"}`, later parts `{"records"}`.
    pub blob: Vec<u8>,
}

// ---- value encoding (B.3 inside canonical JSON) ------------------------------------------------------

fn hyphenated(id: &[u8; 16]) -> String {
    use core::fmt::Write as _;
    let mut out = String::with_capacity(36);
    for (i, byte) in id.iter().enumerate() {
        if matches!(i, 4 | 6 | 8 | 10) {
            out.push('-');
        }
        write!(out, "{byte:02x}").expect("writing to a String cannot fail");
    }
    out
}

fn uuid_of(text: &str) -> Option<[u8; 16]> {
    let bytes = text.as_bytes();
    if bytes.len() != 36 {
        return None;
    }
    let mut out = [0_u8; 16];
    let mut at = 0;
    let mut i = 0;
    while i < 36 {
        if matches!(i, 8 | 13 | 18 | 23) {
            if bytes[i] != b'-' {
                return None;
            }
            i += 1;
            continue;
        }
        let digit = |b: u8| match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            _ => None,
        };
        out[at] = digit(bytes[i])? * 16 + digit(bytes[i + 1])?;
        at += 1;
        i += 2;
    }
    Some(out)
}

fn text(value: &str) -> Node {
    Node::String(value.to_owned())
}
fn opt_text(value: Option<&String>) -> Node {
    value.map_or(Node::Null, |v| Node::String(v.clone()))
}
fn int(value: i64) -> Node {
    Node::String(value.to_string())
}
fn uuid(value: &[u8; 16]) -> Node {
    Node::String(hyphenated(value))
}

fn object(members: Vec<(&str, Node)>) -> Node {
    Node::Object(
        members
            .into_iter()
            .map(|(name, node)| (name.to_owned(), node))
            .collect::<BTreeMap<_, _>>(),
    )
}

// Each encoder below destructures its row WITHOUT `..`, so a column added to the struct is a compile
// error here until it is encoded (and classified in the column lists above), never a silent omission.
fn batch_node(row: &BatchRow) -> Node {
    let BatchRow {
        id,
        source_type,
        source_name,
        file_fingerprint,
        parser_version,
        summary_json,
        imported_at,
        created_at,
        connector_connection_id,
    } = row;
    object(vec![
        ("id", uuid(id)),
        ("source_type", text(source_type)),
        ("source_name", opt_text(source_name.as_ref())),
        ("file_fingerprint", opt_text(file_fingerprint.as_ref())),
        ("parser_version", opt_text(parser_version.as_ref())),
        ("summary_json", opt_text(summary_json.as_ref())),
        ("imported_at", opt_text(imported_at.as_ref())),
        ("created_at", text(created_at)),
        (
            "connector_connection_id",
            connector_connection_id.as_ref().map_or(Node::Null, uuid),
        ),
    ])
}

fn record_node(row: &RecordRow) -> Node {
    let RecordRow {
        id,
        source_batch_id,
        external_id,
        source_hash,
        normalized_json,
        parse_confidence_bps,
        created_at,
    } = row;
    object(vec![
        ("id", uuid(id)),
        ("source_batch_id", uuid(source_batch_id)),
        ("external_id", opt_text(external_id.as_ref())),
        ("source_hash", text(source_hash)),
        ("normalized_json", text(normalized_json)),
        (
            "parse_confidence_bps",
            parse_confidence_bps.map_or(Node::Null, int),
        ),
        ("created_at", text(created_at)),
    ])
}

fn run_node(row: &RunRow) -> Node {
    let RunRow {
        id,
        source_batch_id,
        parser_name,
        parser_version,
        bytes_in,
        records_out,
        status,
        limit_hit,
        started_at,
        finished_at,
    } = row;
    object(vec![
        ("id", uuid(id)),
        ("source_batch_id", uuid(source_batch_id)),
        ("parser_name", text(parser_name)),
        ("parser_version", text(parser_version)),
        ("bytes_in", int(*bytes_in)),
        ("records_out", int(*records_out)),
        ("status", text(status)),
        ("limit_hit", opt_text(limit_hit.as_ref())),
        ("started_at", text(started_at)),
        ("finished_at", opt_text(finished_at.as_ref())),
    ])
}

fn canonical(node: &Node) -> Result<Vec<u8>, SourceBatchError> {
    node_to_canonical_vec(node).map_err(|_| SourceBatchError::Malformed)
}

/// The canonical JSON of one record: what [`MAX_SOURCE_RECORD`] bounds, and the length a staging
/// check measures.
///
/// # Errors
/// [`SourceBatchError::Malformed`] if a value cannot be written canonically (for example a U+0000 in
/// a text); [`SourceBatchError::SourceRecordTooLarge`] over the bound.
pub fn record_json(record: &RecordRow) -> Result<Vec<u8>, SourceBatchError> {
    let bytes = canonical(&record_node(record))?;
    if bytes.len() > MAX_SOURCE_RECORD {
        return Err(SourceBatchError::SourceRecordTooLarge);
    }
    Ok(bytes)
}

/// The canonical JSON size of a batch's columns plus all its parser runs: what
/// [`MAX_BATCH_HEADER`] bounds. A parser run is refused before it enters the vault when adding it
/// would make this exceed the bound.
///
/// # Errors
/// [`SourceBatchError::Malformed`] or [`SourceBatchError::SourceBatchHeaderTooLarge`].
pub fn header_len(batch: &BatchRow, runs: &[RunRow]) -> Result<usize, SourceBatchError> {
    let mut total = canonical(&batch_node(batch))?.len();
    for run in runs {
        total += canonical(&run_node(run))?.len() + 1;
    }
    if total > MAX_BATCH_HEADER {
        return Err(SourceBatchError::SourceBatchHeaderTooLarge);
    }
    Ok(total)
}

fn metadata(batch_id: &[u8; 16], index: u32, count: u32) -> Vec<u8> {
    format!(
        r#"{{"batch_id":"{}","part_count":"{count}","part_index":"{index}"}}"#,
        hyphenated(batch_id)
    )
    .into_bytes()
}

impl SourceBatch {
    /// Split the batch into its ordered parts: part 0 holds the batch columns and the parser runs,
    /// then records are added in ascending `id` byte order, and a part is closed when adding the
    /// next record would pass [`MAX_PART_BLOB`]. Deterministic from the rows alone.
    ///
    /// # Errors
    /// [`SourceBatchError`]: a record over its bound, a header over its bound, a row of another
    /// batch, a duplicate id, or a value that cannot be written canonically.
    pub fn split(&self) -> Result<Vec<Part>, SourceBatchError> {
        let id = self.batch.id;
        let mut runs: Vec<&RunRow> = self.runs.iter().collect();
        runs.sort_by_key(|r| r.id);
        let mut records: Vec<&RecordRow> = self.records.iter().collect();
        records.sort_by_key(|r| r.id);
        if runs.iter().any(|r| r.source_batch_id != id)
            || records.iter().any(|r| r.source_batch_id != id)
        {
            return Err(SourceBatchError::RowOfAnotherBatch);
        }
        if runs.windows(2).any(|w| w[0].id == w[1].id)
            || records.windows(2).any(|w| w[0].id == w[1].id)
        {
            return Err(SourceBatchError::DuplicateRow);
        }
        header_len(&self.batch, &self.runs)?;
        let batch_json = canonical(&batch_node(&self.batch))?;
        let mut runs_json = Vec::new();
        for (i, run) in runs.iter().enumerate() {
            if i > 0 {
                runs_json.push(b',');
            }
            runs_json.extend_from_slice(&canonical(&run_node(run))?);
        }
        let mut record_jsons = Vec::with_capacity(records.len());
        for record in &records {
            record_jsons.push(record_json(record)?);
        }

        // Part 0's prefix carries the batch and the runs; later parts start at `{"records":[`.
        let mut first_prefix = br#"{"batch":"#.to_vec();
        first_prefix.extend_from_slice(&batch_json);
        first_prefix.extend_from_slice(br#","parser_runs":["#);
        first_prefix.extend_from_slice(&runs_json);
        first_prefix.extend_from_slice(br#"],"records":["#);
        let later_prefix: &[u8] = br#"{"records":["#;
        let suffix: &[u8] = b"]}";

        let mut blobs: Vec<Vec<u8>> = Vec::new();
        let mut current = first_prefix;
        let mut in_part = 0_usize;
        for json in &record_jsons {
            let comma = usize::from(in_part > 0);
            if in_part > 0 && current.len() + comma + json.len() + suffix.len() > MAX_PART_BLOB {
                current.extend_from_slice(suffix);
                blobs.push(core::mem::replace(&mut current, later_prefix.to_vec()));
                in_part = 0;
            }
            if in_part > 0 {
                current.push(b',');
            }
            current.extend_from_slice(json);
            in_part += 1;
        }
        current.extend_from_slice(suffix);
        blobs.push(current);

        let count = u32::try_from(blobs.len()).map_err(|_| SourceBatchError::Incomplete)?;
        Ok(blobs
            .into_iter()
            .enumerate()
            .map(|(i, blob)| {
                let index = u32::try_from(i).expect("bounded by the part count");
                Part {
                    index,
                    count,
                    logical_metadata: metadata(&id, index, count),
                    blob,
                }
            })
            .collect())
    }
}

// ---- the strict reverse ------------------------------------------------------------------------------

fn members(node: Node) -> Result<BTreeMap<String, Node>, SourceBatchError> {
    match node {
        Node::Object(members) => Ok(members),
        _ => Err(SourceBatchError::Malformed),
    }
}

fn exactly(members: &BTreeMap<String, Node>, names: &[&str]) -> Result<(), SourceBatchError> {
    let mut have: Vec<&str> = members.keys().map(String::as_str).collect();
    have.sort_unstable();
    let mut want: Vec<&str> = names.to_vec();
    want.sort_unstable();
    if have == want {
        Ok(())
    } else {
        Err(SourceBatchError::Malformed)
    }
}

fn get_text(m: &BTreeMap<String, Node>, name: &str) -> Result<String, SourceBatchError> {
    match m.get(name) {
        Some(Node::String(s)) => Ok(s.clone()),
        _ => Err(SourceBatchError::Malformed),
    }
}
fn get_opt_text(
    m: &BTreeMap<String, Node>,
    name: &str,
) -> Result<Option<String>, SourceBatchError> {
    match m.get(name) {
        Some(Node::Null) => Ok(None),
        Some(Node::String(s)) => Ok(Some(s.clone())),
        _ => Err(SourceBatchError::Malformed),
    }
}
fn parse_int(text: &str) -> Option<i64> {
    let digits = text.strip_prefix('-').unwrap_or(text);
    if digits.is_empty()
        || (digits.len() > 1 && digits.starts_with('0'))
        || !digits.bytes().all(|b| b.is_ascii_digit())
        || text == "-0"
    {
        return None;
    }
    text.parse().ok()
}
fn get_int(m: &BTreeMap<String, Node>, name: &str) -> Result<i64, SourceBatchError> {
    parse_int(&get_text(m, name)?).ok_or(SourceBatchError::Malformed)
}
fn get_opt_int(m: &BTreeMap<String, Node>, name: &str) -> Result<Option<i64>, SourceBatchError> {
    match m.get(name) {
        Some(Node::Null) => Ok(None),
        _ => Ok(Some(get_int(m, name)?)),
    }
}
fn get_uuid(m: &BTreeMap<String, Node>, name: &str) -> Result<[u8; 16], SourceBatchError> {
    uuid_of(&get_text(m, name)?).ok_or(SourceBatchError::Malformed)
}
fn get_opt_uuid(
    m: &BTreeMap<String, Node>,
    name: &str,
) -> Result<Option<[u8; 16]>, SourceBatchError> {
    match m.get(name) {
        Some(Node::Null) => Ok(None),
        _ => Ok(Some(get_uuid(m, name)?)),
    }
}

fn batch_of(node: Node) -> Result<BatchRow, SourceBatchError> {
    let m = members(node)?;
    exactly(&m, BATCH_COLUMNS)?;
    Ok(BatchRow {
        id: get_uuid(&m, "id")?,
        source_type: get_text(&m, "source_type")?,
        source_name: get_opt_text(&m, "source_name")?,
        file_fingerprint: get_opt_text(&m, "file_fingerprint")?,
        parser_version: get_opt_text(&m, "parser_version")?,
        summary_json: get_opt_text(&m, "summary_json")?,
        imported_at: get_opt_text(&m, "imported_at")?,
        created_at: get_text(&m, "created_at")?,
        connector_connection_id: get_opt_uuid(&m, "connector_connection_id")?,
    })
}

fn record_of(node: Node) -> Result<RecordRow, SourceBatchError> {
    let m = members(node)?;
    exactly(&m, RECORD_COLUMNS)?;
    Ok(RecordRow {
        id: get_uuid(&m, "id")?,
        source_batch_id: get_uuid(&m, "source_batch_id")?,
        external_id: get_opt_text(&m, "external_id")?,
        source_hash: get_text(&m, "source_hash")?,
        normalized_json: get_text(&m, "normalized_json")?,
        parse_confidence_bps: get_opt_int(&m, "parse_confidence_bps")?,
        created_at: get_text(&m, "created_at")?,
    })
}

fn run_of(node: Node) -> Result<RunRow, SourceBatchError> {
    let m = members(node)?;
    exactly(&m, RUN_COLUMNS)?;
    Ok(RunRow {
        id: get_uuid(&m, "id")?,
        source_batch_id: get_uuid(&m, "source_batch_id")?,
        parser_name: get_text(&m, "parser_name")?,
        parser_version: get_text(&m, "parser_version")?,
        bytes_in: get_int(&m, "bytes_in")?,
        records_out: get_int(&m, "records_out")?,
        status: get_text(&m, "status")?,
        limit_hit: get_opt_text(&m, "limit_hit")?,
        started_at: get_text(&m, "started_at")?,
        finished_at: get_opt_text(&m, "finished_at")?,
    })
}

fn array(m: &mut BTreeMap<String, Node>, name: &str) -> Result<Vec<Node>, SourceBatchError> {
    match m.remove(name) {
        Some(Node::Array(items)) => Ok(items),
        _ => Err(SourceBatchError::Malformed),
    }
}

/// What a part's blob holds, decoded and checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedPart {
    /// Part 0 only: the batch's carried columns.
    pub batch: Option<BatchRow>,
    /// Part 0 only: the parser runs.
    pub runs: Vec<RunRow>,
    /// The records in this part.
    pub records: Vec<RecordRow>,
}

/// Decode one part's blob (`index` selects part 0's shape): every row exactly the carried columns,
/// every row of the batch the part's metadata names (`batch_id`), records and runs in ascending id
/// order, canonical JSON throughout.
///
/// # Errors
/// [`SourceBatchError::Malformed`] or [`SourceBatchError::RowOfAnotherBatch`] or
/// [`SourceBatchError::DuplicateRow`].
pub fn decode_part(
    batch_id: &[u8; 16],
    index: u32,
    blob: &[u8],
) -> Result<DecodedPart, SourceBatchError> {
    let mut m = members(parse_canonical_node(blob).map_err(|_| SourceBatchError::Malformed)?)?;
    let decoded = if index == 0 {
        exactly(&m, &["batch", "parser_runs", "records"])?;
        let batch = batch_of(m.remove("batch").ok_or(SourceBatchError::Malformed)?)?;
        let runs = array(&mut m, "parser_runs")?
            .into_iter()
            .map(run_of)
            .collect::<Result<Vec<_>, _>>()?;
        let records = array(&mut m, "records")?
            .into_iter()
            .map(record_of)
            .collect::<Result<Vec<_>, _>>()?;
        DecodedPart {
            batch: Some(batch),
            runs,
            records,
        }
    } else {
        exactly(&m, &["records"])?;
        let records = array(&mut m, "records")?
            .into_iter()
            .map(record_of)
            .collect::<Result<Vec<_>, _>>()?;
        DecodedPart {
            batch: None,
            runs: Vec::new(),
            records,
        }
    };
    if decoded.batch.as_ref().is_some_and(|b| &b.id != batch_id)
        || decoded.runs.iter().any(|r| &r.source_batch_id != batch_id)
        || decoded
            .records
            .iter()
            .any(|r| &r.source_batch_id != batch_id)
    {
        return Err(SourceBatchError::RowOfAnotherBatch);
    }
    if decoded.runs.windows(2).any(|w| w[0].id >= w[1].id)
        || decoded.records.windows(2).any(|w| w[0].id >= w[1].id)
    {
        return Err(SourceBatchError::DuplicateRow);
    }
    Ok(decoded)
}

/// The ids of the records a part's blob holds, in order. Used by the closure check to bind a named
/// source record to the batch that holds it.
///
/// # Errors
/// As [`decode_part`].
pub fn record_ids(
    batch_id: &[u8; 16],
    index: u32,
    blob: &[u8],
) -> Result<Vec<[u8; 16]>, SourceBatchError> {
    Ok(decode_part(batch_id, index, blob)?
        .records
        .iter()
        .map(|r| r.id)
        .collect())
}

/// Join a batch's parts back into the batch: exactly parts `0` to `count - 1` (given as
/// `(index, count, blob)` in any order), one `count`, no gap and no duplicate, part 0 holding the
/// batch and the runs, and a record in no more than one part.
///
/// # Errors
/// [`SourceBatchError::Incomplete`] for a missing, duplicate or disagreeing part, otherwise as
/// [`decode_part`].
pub fn join(
    batch_id: &[u8; 16],
    parts: &[(u32, u32, &[u8])],
) -> Result<SourceBatch, SourceBatchError> {
    let count = parts
        .first()
        .map(|p| p.1)
        .ok_or(SourceBatchError::Incomplete)?;
    if parts.iter().any(|p| p.1 != count)
        || usize::try_from(count).ok() != Some(parts.len())
        || count == 0
    {
        return Err(SourceBatchError::Incomplete);
    }
    let mut ordered: Vec<&(u32, u32, &[u8])> = parts.iter().collect();
    ordered.sort_by_key(|p| p.0);
    if ordered
        .iter()
        .enumerate()
        .any(|(i, p)| usize::try_from(p.0).ok() != Some(i))
    {
        return Err(SourceBatchError::Incomplete);
    }
    let mut batch = None;
    let mut runs = Vec::new();
    let mut records: Vec<RecordRow> = Vec::new();
    for (index, _, blob) in ordered.iter().map(|p| **p) {
        let part = decode_part(batch_id, index, blob)?;
        if part.batch.is_some() {
            batch = part.batch;
            runs = part.runs;
        }
        records.extend(part.records);
    }
    records.sort_by_key(|r| r.id);
    if records.windows(2).any(|w| w[0].id == w[1].id) {
        return Err(SourceBatchError::DuplicateRow);
    }
    Ok(SourceBatch {
        batch: batch.ok_or(SourceBatchError::Incomplete)?,
        runs,
        records,
    })
}
