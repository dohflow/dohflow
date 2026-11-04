//! The logical genesis snapshot's plaintext (`docs/architecture/vault-format.md` C.8,
//! personal-cfo-1cltt.1): the framing `SnapshotPlaintextV1`, its manifest, and the per-table state
//! digest a replica recomputes from rows.
//!
//! This module is pure bytes. It reads no database and does no cryptography beyond SHA-256: the
//! walker that produces the table files is `db-worker`'s, and sealing the plaintext is
//! personal-cfo-1cltt.3's. What lives here is everything a producer and a receiver must agree on,
//! so the rule has one implementation: the file set and its order, the size caps, the manifest's
//! canonical form, and a verifier that checks a plaintext against its own manifest.
//!
//! Errors name a rule and never carry a value from the plaintext.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::canonical_json::{from_canonical_slice, to_canonical_vec, CanonicalJsonError};
use crate::canonical_json::{node_to_canonical_vec, Node};
use crate::cbe::{Reader, WireError, Writer};
use crate::row_digest::{Column, Kind, Value, MAX_NAME};

/// The label a snapshot plaintext starts with.
pub const SNAPSHOT_LABEL: &str = "dohflow/sync-snapshot-plaintext/v1";
/// The domain-separation label of a table's state digest.
pub const STATE_DIGEST_LABEL: &str = "dohflow/sync-state-digest/v1";
/// The path of the manifest, always the first file.
pub const MANIFEST_PATH: &str = "manifest.json";
/// The most files a snapshot holds, the manifest included.
pub const MAX_FILES: usize = 1024;
/// The longest path, in bytes.
pub const MAX_PATH: usize = 255;
/// The largest plaintext, in bytes (1 GiB): a larger vault fails with
/// [`SnapshotError::TooLarge`] until a chunked framing is specified.
pub const MAX_PLAINTEXT: usize = 1 << 30;
/// The constant `format` of the manifest.
pub const FORMAT: &str = "dohflow-sync-snapshot";
/// The manifest's `format_version` and `snapshot_schema_version` for this revision.
pub const VERSION: &str = "1";
/// The most voided ranges a manifest records (the ledger's own bound, C.10).
pub const MAX_VOIDS: usize = 4096;
/// The most artifacts a closure manifest lists.
pub const MAX_ARTIFACTS: usize = 1 << 20;

/// Why a snapshot could not be built or was refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum SnapshotError {
    /// The plaintext would be or is larger than 2^30 bytes (`SnapshotTooLarge`, C.11).
    #[error("the snapshot plaintext is larger than 2^30 bytes")]
    TooLarge,
    /// More than [`MAX_FILES`] files.
    #[error("the snapshot has more than 1024 files")]
    TooManyFiles,
    /// A CBE1 framing violation.
    #[error("the snapshot framing is malformed: {0}")]
    Framing(WireError),
    /// The first file is not `manifest.json`.
    #[error("the first file is not the manifest")]
    FirstFileNotManifest,
    /// The data files are not in strictly ascending path order.
    #[error("the data files are not in strictly ascending path order")]
    PathsNotAscending,
    /// A path is not `data/<table>.jsonl` for a valid table name.
    #[error("a file path is not a data file path")]
    BadPath,
    /// The manifest is not the canonical JSON of a manifest.
    #[error("the manifest is not valid canonical JSON: {0}")]
    ManifestNotCanonical(CanonicalJsonError),
    /// The manifest is well formed but breaks a rule of C.8.
    #[error("the manifest breaks a rule: {0}")]
    ManifestInvalid(&'static str),
    /// A stored value does not have its column's declared kind (a storage-class mismatch, a
    /// UUID column that is not 16 bytes, or text that is not UTF-8): never coerced.
    #[error("column {column}: the stored value does not match its declared kind")]
    KindMismatch {
        /// The column.
        column: String,
    },
    /// A text value cannot be written as a canonical JSON string (it contains U+0000).
    #[error("column {column}: the text cannot be written as canonical JSON")]
    Unrepresentable {
        /// The column.
        column: String,
    },
    /// A file the manifest names is absent, or a file it does not name is present.
    #[error("the files and the manifest name different sets")]
    FileSetMismatch,
    /// A file's SHA-256 differs from the manifest's.
    #[error("a table file does not match its manifest hash")]
    HashMismatch,
    /// A file's line count differs from the manifest's row count.
    #[error("a table file does not have its manifest row count")]
    RowCountMismatch,
}

impl From<WireError> for SnapshotError {
    fn from(error: WireError) -> Self {
        Self::Framing(error)
    }
}

/// One column of a table file, as the manifest declares it (the kind fixes its encoding, B.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColumnEntry {
    /// The column name.
    pub name: String,
    /// `int`, `text`, `uuid` or `bytes`.
    pub kind: String,
}

/// One table file of the manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TableEntry {
    /// The table's name; its file is `data/<name>.jsonl`.
    pub name: String,
    /// The number of rows (lines), a decimal string.
    pub row_count: String,
    /// SHA-256 of the file's bytes, lowercase hex.
    pub sha256: String,
    /// The table's state digest (lowercase hex), recomputed by a replica from its rows. Absent
    /// (`null`) for a folded child, whose rows are part of its parent's digest (C.1.5).
    pub state_digest: Option<String>,
    /// The table's columns in file order.
    pub columns: Vec<ColumnEntry>,
}

/// One honored void the snapshot records (C.10 "Void records", ADR 0074 Addendum D rule 8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoidEntry {
    /// The void's id, lowercase hex.
    pub void_id: String,
    /// The first skipped log position, a decimal string.
    pub first: String,
    /// The last skipped log position, a decimal string.
    pub last: String,
}

/// The manifest of a snapshot (`manifest.json`, canonical JSON, every integer a decimal string).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// The constant [`FORMAT`].
    pub format: String,
    /// [`VERSION`].
    pub format_version: String,
    /// [`VERSION`]: the layout of the plaintext and this manifest.
    pub snapshot_schema_version: String,
    /// The version of the B.2 inventory the table set follows.
    pub inventory_version: String,
    /// The producer's database schema version, a decimal string.
    pub schema_version: String,
    /// The origin vault's creation time (the `created_at` of its `vault_metadata` row). Carried
    /// here, never as a `vault_metadata` row: a bootstrapped replica installs it before any seed
    /// migration runs, so a migration that stamps it writes the origin's bytes (C.8).
    pub vault_created_at: String,
    /// Every table file, ascending by name.
    pub tables: Vec<TableEntry>,
    /// The voided ranges and void ids this snapshot records, ranges strictly ascending and
    /// disjoint, as the C.10 ledger encodes them.
    pub voids: Vec<VoidEntry>,
    /// The complete closure manifest of `sync_artifact_id_e` values (lowercase hex), strictly
    /// ascending. Empty until personal-cfo-1cltt.7.
    pub artifacts: Vec<String>,
}

/// The data file path of a table.
#[must_use]
pub fn data_path(table: &str) -> String {
    format!("data/{table}.jsonl")
}

fn table_of_path(path: &str) -> Result<&str, SnapshotError> {
    let name = path
        .strip_prefix("data/")
        .and_then(|rest| rest.strip_suffix(".jsonl"))
        .ok_or(SnapshotError::BadPath)?;
    if valid_table_name(name) {
        Ok(name)
    } else {
        Err(SnapshotError::BadPath)
    }
}

/// A table name is 1 to 64 bytes of `[a-z0-9_]`.
fn valid_table_name(name: &str) -> bool {
    (1..=MAX_NAME).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Lowercase hex of `bytes`.
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(DIGITS[usize::from(byte >> 4)]));
        out.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    out
}

/// Whether `text` is lowercase hex of exactly `bytes` bytes.
fn is_hex_of(text: &str, bytes: usize) -> bool {
    text.len() == bytes * 2
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A canonical decimal `u64`: digits only, no sign, no leading zero (except `0`).
fn decimal_u64(text: &str) -> Option<u64> {
    let canonical = !text.is_empty()
        && text.bytes().all(|b| b.is_ascii_digit())
        && (text == "0" || !text.starts_with('0'));
    if canonical {
        text.parse().ok()
    } else {
        None
    }
}

// ----------------------------------------------------------------------------------------------
// The state digest

/// The state digest of one table (C.8): `SHA-256(label "dohflow/sync-state-digest/v1" ‖ text(64)
/// table ‖ u64 row_count ‖ for each row in ascending unsigned primary-key byte order: digest
/// row_digest)`.
///
/// Rows are pushed in order and the order is checked, so two replicas that hold the same rows
/// produce the same digest however they enumerate them. A table with no rows has a digest too.
/// For a parent with folded children the row digest is the parent's (C.1.5), so the children are
/// inside it and a folded child has no digest of its own.
pub struct StateDigest {
    table: String,
    rows: Vec<[u8; 32]>,
    last_key: Option<Vec<u8>>,
}

impl StateDigest {
    /// Start the digest of `table`.
    ///
    /// # Errors
    /// [`SnapshotError::BadPath`] if `table` is not a valid table name.
    pub fn new(table: &str) -> Result<Self, SnapshotError> {
        if !valid_table_name(table) {
            return Err(SnapshotError::BadPath);
        }
        Ok(Self {
            table: table.to_owned(),
            rows: Vec::new(),
            last_key: None,
        })
    }

    /// Add the next row: its primary-key bytes (C.1.5) and its row digest.
    ///
    /// # Errors
    /// [`SnapshotError::ManifestInvalid`] if the key is not strictly greater than the previous
    /// row's (unsigned bytewise, a shorter key first when it is a prefix).
    pub fn push(&mut self, primary_key: &[u8], row_digest: [u8; 32]) -> Result<(), SnapshotError> {
        if let Some(last) = &self.last_key {
            if primary_key <= last.as_slice() {
                return Err(SnapshotError::ManifestInvalid(
                    "rows were not offered in strictly ascending primary-key order",
                ));
            }
        }
        self.last_key = Some(primary_key.to_vec());
        self.rows.push(row_digest);
        Ok(())
    }

    /// The digest.
    ///
    /// # Errors
    /// [`SnapshotError::Framing`] only for a table name over its bound, which `new` refuses.
    pub fn finish(self) -> Result<[u8; 32], SnapshotError> {
        let mut writer = Writer::with_label(STATE_DIGEST_LABEL);
        writer.text(MAX_NAME, &self.table)?;
        writer.u64(self.rows.len() as u64);
        for digest in &self.rows {
            writer.fixed(digest);
        }
        Ok(Sha256::digest(writer.as_bytes()).into())
    }
}

// ----------------------------------------------------------------------------------------------
// Building

/// One table's file as a producer hands it over.
#[derive(Debug, Clone)]
pub struct TableFile {
    /// The table's name.
    pub name: String,
    /// The columns in file order, with their kinds.
    pub columns: Vec<ColumnEntry>,
    /// The number of rows.
    pub row_count: u64,
    /// The state digest, or `None` for a folded child.
    pub state_digest: Option<[u8; 32]>,
    /// The file's bytes: one canonical JSON object per row, each ending in `\n`.
    pub content: Vec<u8>,
}

/// An honored void to record (C.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordedVoid {
    /// The void's id.
    pub void_id: [u8; 32],
    /// The first skipped position.
    pub first: u64,
    /// The last skipped position.
    pub last: u64,
}

/// The header fields a producer supplies; the table entries are derived from the files.
#[derive(Debug, Clone)]
pub struct Header<'a> {
    /// The producer's database schema version.
    pub schema_version: u64,
    /// The origin vault's creation time.
    pub vault_created_at: &'a str,
    /// The B.2 inventory version.
    pub inventory_version: &'a str,
    /// The voided ranges to record, ascending and disjoint.
    pub voids: &'a [RecordedVoid],
    /// The closure manifest: `sync_artifact_id_e` values, strictly ascending.
    pub artifacts: &'a [[u8; 32]],
}

/// Build a snapshot plaintext from `tables`.
///
/// The files are put in ascending path order, the manifest is derived from them (so it cannot
/// disagree with them) and the result is run through [`verify`] before it is returned.
///
/// # Errors
/// [`SnapshotError`]: `TooLarge` above 2^30 bytes, `TooManyFiles` above 1024, or any rule
/// [`verify`] enforces.
pub fn build(header: &Header<'_>, mut tables: Vec<TableFile>) -> Result<Vec<u8>, SnapshotError> {
    if tables.len() + 1 > MAX_FILES {
        return Err(SnapshotError::TooManyFiles);
    }
    tables.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
    let data_bytes = tables
        .iter()
        .try_fold(0usize, |sum, t| sum.checked_add(t.content.len()))
        .ok_or(SnapshotError::TooLarge)?;
    if data_bytes > MAX_PLAINTEXT {
        return Err(SnapshotError::TooLarge);
    }
    let manifest = Manifest {
        format: FORMAT.to_owned(),
        format_version: VERSION.to_owned(),
        snapshot_schema_version: VERSION.to_owned(),
        inventory_version: header.inventory_version.to_owned(),
        schema_version: header.schema_version.to_string(),
        vault_created_at: header.vault_created_at.to_owned(),
        tables: tables
            .iter()
            .map(|t| TableEntry {
                name: t.name.clone(),
                row_count: t.row_count.to_string(),
                sha256: hex(&Sha256::digest(&t.content)),
                state_digest: t.state_digest.as_ref().map(|d| hex(d)),
                columns: t.columns.clone(),
            })
            .collect(),
        voids: header
            .voids
            .iter()
            .map(|v| VoidEntry {
                void_id: hex(&v.void_id),
                first: v.first.to_string(),
                last: v.last.to_string(),
            })
            .collect(),
        artifacts: header.artifacts.iter().map(|a| hex(a)).collect(),
    };
    let manifest_bytes =
        to_canonical_vec(&manifest).map_err(SnapshotError::ManifestNotCanonical)?;

    let mut writer = Writer::with_label(SNAPSHOT_LABEL);
    writer.u32(u32::try_from(tables.len() + 1).map_err(|_| SnapshotError::TooManyFiles)?);
    writer.text(MAX_PATH, MANIFEST_PATH)?;
    writer.bytes(MAX_PLAINTEXT, &manifest_bytes)?;
    for table in &tables {
        writer.text(MAX_PATH, &data_path(&table.name))?;
        writer.bytes(MAX_PLAINTEXT, &table.content)?;
    }
    let plaintext = writer.finish();
    if plaintext.len() > MAX_PLAINTEXT {
        return Err(SnapshotError::TooLarge);
    }
    verify(&plaintext)?;
    Ok(plaintext)
}

// ----------------------------------------------------------------------------------------------
// Verifying

/// A snapshot plaintext that passed [`verify`]: its manifest and its table files.
#[derive(Debug, Clone)]
pub struct Verified {
    /// The parsed manifest.
    pub manifest: Manifest,
    /// `(table, file bytes)` in ascending table order, as the manifest lists them.
    pub files: Vec<(String, Vec<u8>)>,
}

/// Check a snapshot plaintext against C.8: the framing and its caps, the manifest as canonical
/// JSON with every rule below, the file set equal to the manifest's, each file's SHA-256 and line
/// count as the manifest states.
///
/// What the rows mean (kinds, keys, the state digest) is for the reader that has a database;
/// this checks everything that needs only the bytes.
///
/// # Errors
/// The first [`SnapshotError`] in the order: size, framing, file order and paths, manifest, file
/// set, hashes and counts.
pub fn verify(plaintext: &[u8]) -> Result<Verified, SnapshotError> {
    if plaintext.len() > MAX_PLAINTEXT {
        return Err(SnapshotError::TooLarge);
    }
    let mut reader = Reader::new(plaintext);
    reader.label(SNAPSHOT_LABEL)?;
    let count = usize::try_from(reader.u32()?).map_err(|_| SnapshotError::TooManyFiles)?;
    if count == 0 || count > MAX_FILES {
        return Err(SnapshotError::TooManyFiles);
    }
    let first_path = reader.text(MAX_PATH)?;
    if first_path != MANIFEST_PATH {
        return Err(SnapshotError::FirstFileNotManifest);
    }
    let manifest_bytes = reader.bytes(MAX_PLAINTEXT)?;
    let mut files: Vec<(String, &[u8])> = Vec::with_capacity(count - 1);
    let mut previous: Option<&str> = None;
    for _ in 1..count {
        let path = reader.text(MAX_PATH)?;
        let table = table_of_path(path)?;
        if previous.is_some_and(|p| p.as_bytes() >= path.as_bytes()) {
            return Err(SnapshotError::PathsNotAscending);
        }
        previous = Some(path);
        files.push((table.to_owned(), reader.bytes(MAX_PLAINTEXT)?));
    }
    reader.finish()?;

    let manifest: Manifest =
        from_canonical_slice(manifest_bytes).map_err(SnapshotError::ManifestNotCanonical)?;
    check_manifest(&manifest)?;

    if manifest.tables.len() != files.len()
        || manifest
            .tables
            .iter()
            .zip(&files)
            .any(|(entry, (table, _))| entry.name != *table)
    {
        return Err(SnapshotError::FileSetMismatch);
    }
    for (entry, (_, content)) in manifest.tables.iter().zip(&files) {
        if hex(&Sha256::digest(content)) != entry.sha256 {
            return Err(SnapshotError::HashMismatch);
        }
        // `row_count` was validated as a canonical decimal by `check_manifest`.
        let rows = decimal_u64(&entry.row_count).ok_or(SnapshotError::RowCountMismatch)?;
        let newlines = content.iter().filter(|b| **b == b'\n').count() as u64;
        let ends_cleanly = content.is_empty() || content.last() == Some(&b'\n');
        if newlines != rows || !ends_cleanly {
            return Err(SnapshotError::RowCountMismatch);
        }
    }
    Ok(Verified {
        files: files
            .into_iter()
            .map(|(table, content)| (table, content.to_vec()))
            .collect(),
        manifest,
    })
}

fn check_manifest(manifest: &Manifest) -> Result<(), SnapshotError> {
    let invalid = |why: &'static str| Err(SnapshotError::ManifestInvalid(why));
    if manifest.format != FORMAT {
        return invalid("the format is not the snapshot format");
    }
    if manifest.format_version != VERSION || manifest.snapshot_schema_version != VERSION {
        return invalid("an unsupported format or snapshot schema version");
    }
    if decimal_u64(&manifest.schema_version).is_none() {
        return invalid("the schema version is not a canonical decimal");
    }
    if manifest.inventory_version.is_empty() || manifest.vault_created_at.is_empty() {
        return invalid("a header field is empty");
    }
    if manifest.tables.len() + 1 > MAX_FILES {
        return Err(SnapshotError::TooManyFiles);
    }
    for pair in manifest.tables.windows(2) {
        if pair[0].name.as_bytes() >= pair[1].name.as_bytes() {
            return invalid("the tables are not in strictly ascending order");
        }
    }
    for table in &manifest.tables {
        if !valid_table_name(&table.name) {
            return invalid("a table name is not valid");
        }
        if decimal_u64(&table.row_count).is_none() {
            return invalid("a row count is not a canonical decimal");
        }
        if !is_hex_of(&table.sha256, 32) {
            return invalid("a file hash is not 32 bytes of lowercase hex");
        }
        if table
            .state_digest
            .as_deref()
            .is_some_and(|d| !is_hex_of(d, 32))
        {
            return invalid("a state digest is not 32 bytes of lowercase hex");
        }
        if table.columns.is_empty()
            || table.columns.iter().any(|c| {
                !valid_table_name(&c.name)
                    || !["int", "text", "uuid", "bytes"].contains(&c.kind.as_str())
            })
        {
            return invalid("a column is not valid");
        }
    }
    if manifest.voids.len() > MAX_VOIDS {
        return invalid("too many voided ranges");
    }
    let mut previous_last: Option<u64> = None;
    for void in &manifest.voids {
        let (Some(first), Some(last)) = (decimal_u64(&void.first), decimal_u64(&void.last)) else {
            return invalid("a voided position is not a canonical decimal");
        };
        if !is_hex_of(&void.void_id, 32) || first > last {
            return invalid("a void entry is not valid");
        }
        if previous_last.is_some_and(|p| first <= p) {
            return invalid("the voided ranges are not strictly ascending and disjoint");
        }
        previous_last = Some(last);
    }
    if manifest.artifacts.len() > MAX_ARTIFACTS {
        return invalid("too many artifacts");
    }
    if manifest.artifacts.iter().any(|a| !is_hex_of(a, 32)) {
        return invalid("an artifact id is not 32 bytes of lowercase hex");
    }
    if manifest
        .artifacts
        .windows(2)
        .any(|pair| pair[0].as_bytes() >= pair[1].as_bytes())
    {
        return invalid("the artifacts are not strictly ascending");
    }
    Ok(())
}

// ----------------------------------------------------------------------------------------------
// Rows

/// One row as a data-file line (B.3 "Row encoding"): a canonical JSON object keyed by column
/// name, ending in `\n`. By declared kind: `int` a decimal string, `text` a string, `uuid` the
/// lowercase hyphenated form of the 16 bytes, `bytes` standard padded base64, and NULL is `null`
/// in any kind. The storage class must match the kind; nothing is coerced.
///
/// # Errors
/// [`SnapshotError::KindMismatch`] or [`SnapshotError::Unrepresentable`] naming the column.
pub fn row_line(columns: &[Column<'_>]) -> Result<Vec<u8>, SnapshotError> {
    let mut object = std::collections::BTreeMap::new();
    for column in columns {
        let mismatch = || SnapshotError::KindMismatch {
            column: column.name.to_owned(),
        };
        let node = match (column.kind, column.value) {
            (_, Value::Null) => Node::Null,
            (Kind::Int, Value::Int(v)) => Node::String(v.to_string()),
            (Kind::Text, Value::Text(bytes)) => {
                let text = std::str::from_utf8(bytes).map_err(|_| mismatch())?;
                if text.contains('\0') {
                    return Err(SnapshotError::Unrepresentable {
                        column: column.name.to_owned(),
                    });
                }
                Node::String(text.to_owned())
            }
            (Kind::Uuid, Value::Blob(bytes)) => {
                Node::String(uuid_text(bytes.try_into().map_err(|_| mismatch())?))
            }
            (Kind::Bytes, Value::Blob(bytes)) => Node::String(base64(bytes)),
            _ => return Err(mismatch()),
        };
        if object.insert(column.name.to_owned(), node).is_some() {
            return Err(mismatch());
        }
    }
    let mut line = node_to_canonical_vec(&Node::Object(object))
        .map_err(SnapshotError::ManifestNotCanonical)?;
    line.push(b'\n');
    Ok(line)
}

/// The lowercase hyphenated text of 16 bytes.
#[must_use]
pub fn uuid_text(bytes: [u8; 16]) -> String {
    let h = hex(&bytes);
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// Standard base64 with padding (RFC 4648 section 4).
#[must_use]
pub fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(char::from(ALPHABET[(n >> 18) as usize & 63]));
        out.push(char::from(ALPHABET[(n >> 12) as usize & 63]));
        out.push(if chunk.len() > 1 {
            char::from(ALPHABET[(n >> 6) as usize & 63])
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            char::from(ALPHABET[n as usize & 63])
        } else {
            '='
        });
    }
    out
}
