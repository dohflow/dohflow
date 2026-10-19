//! The canonical row encoding of `docs/architecture/vault-format.md` C.1.5 (personal-cfo-t2s4b
//! part 5b): the primary-key bytes, the digest of a row that exists and the digest of a row a
//! command deleted. The write-set hash (C.4), the replay-property hash and the simulator's
//! equality checks all hash rows through these functions.
//!
//! The functions are pure and know nothing about SQLite: the caller reads a row's stored
//! values, pairs each with its **declared kind** from the column-kind manifest, and passes
//! them in. A stored value whose storage class does not match its kind is an error, never
//! coerced. Errors name the rule that was broken and never carry a value from the row.

use sha2::{Digest, Sha256};
use thiserror::Error;

/// The domain-separation label of an existing row's digest.
pub const ROW_LABEL: &str = "dohflow/sync-row/v1";
/// The domain-separation label of a deleted row's digest.
pub const ABSENT_LABEL: &str = "dohflow/sync-row-absent/v1";
/// The longest `primary_key`, in bytes.
pub const MAX_PRIMARY_KEY: usize = 256;
/// The longest text or bytes value, in bytes.
pub const MAX_VALUE: usize = 1 << 24;
/// The longest table or column name, in bytes.
pub const MAX_NAME: usize = 64;

/// A column's declared kind, aligned with the portable export's kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// A signed 64-bit integer, stored as INTEGER.
    Int,
    /// UTF-8 text, stored as TEXT.
    Text,
    /// A 16-byte identifier, stored as a BLOB of exactly 16 bytes.
    Uuid,
    /// Any other binary value, stored as a BLOB.
    Bytes,
}

/// A stored value as SQLite holds it, by storage class. There is no REAL: a real value
/// has no tag and no way to reach this type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Value<'a> {
    /// SQL NULL.
    Null,
    /// INTEGER.
    Int(i64),
    /// TEXT, as the raw stored bytes (SQLite does not validate UTF-8; this encoder does).
    Text(&'a [u8]),
    /// BLOB.
    Blob(&'a [u8]),
}

/// One column of a row: its name, declared kind and stored value.
#[derive(Debug, Clone, Copy)]
pub struct Column<'a> {
    /// The column name (at most 64 bytes).
    pub name: &'a str,
    /// The declared kind from the column-kind manifest.
    pub kind: Kind,
    /// The stored value.
    pub value: Value<'a>,
}

/// The folded child rows of one parent, for one child table (C.1.5 "Folded children"). The
/// rows of a child table that declares no primary key are part of their parent's digest.
#[derive(Debug, Clone)]
pub struct ChildTable<'a> {
    /// The child table's name.
    pub table: &'a str,
    /// One entry per child row, each the row's columns (all of the child's declared columns,
    /// never a rowid).
    pub rows: Vec<Vec<Column<'a>>>,
}

/// The longest encoded child row, in bytes.
pub const MAX_CHILD_ROW: usize = 1 << 16;

/// Why a row could not be encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum RowDigestError {
    /// A stored value's storage class does not match its declared kind.
    #[error("a stored value does not match its declared column kind")]
    KindMismatch,
    /// A `uuid` column holds a blob that is not exactly 16 bytes.
    #[error("a uuid column does not hold exactly 16 bytes")]
    BadUuidLength,
    /// A text value is not valid UTF-8.
    #[error("a text value is not valid UTF-8")]
    InvalidUtf8,
    /// A text or bytes value is longer than 2^24 bytes.
    #[error("a value exceeds the maximum length")]
    ValueTooLong,
    /// A table or column name is empty or longer than 64 bytes.
    #[error("a table or column name has an invalid length")]
    BadName,
    /// Two columns of one row have the same name.
    #[error("a row has two columns with the same name")]
    DuplicateColumn,
    /// A row has more than 65,535 columns.
    #[error("a row has too many columns")]
    TooManyColumns,
    /// A child row encodes to more than 65,536 bytes.
    #[error("a folded child row exceeds the maximum length")]
    ChildRowTooLong,
    /// A parent was given no child tables, two with the same name, or too many. C.1.5 defines no
    /// section for undeclared children: a parent that declares none uses [`row_digest`], and
    /// one that declares children always lists each declared table, with a count of 0 when empty.
    #[error("a parent has no, a duplicate or too many child tables")]
    BadChildTables,
    /// A primary key has no columns.
    #[error("a primary key has no columns")]
    EmptyKey,
    /// A primary key is longer than 256 bytes. It is never truncated.
    #[error("a primary key exceeds 256 bytes")]
    KeyTooLong,
}

fn label(out: &mut Vec<u8>, text: &str) {
    let bytes = text.as_bytes();
    let length = u16::try_from(bytes.len()).expect("labels are short constants");
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(bytes);
}

fn name(out: &mut Vec<u8>, text: &str) -> Result<(), RowDigestError> {
    let bytes = text.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_NAME {
        return Err(RowDigestError::BadName);
    }
    let length = u32::try_from(bytes.len()).map_err(|_| RowDigestError::BadName)?;
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(bytes);
    Ok(())
}

fn length_prefixed(out: &mut Vec<u8>, bytes: &[u8]) -> Result<(), RowDigestError> {
    if bytes.len() > MAX_VALUE {
        return Err(RowDigestError::ValueTooLong);
    }
    let length = u32::try_from(bytes.len()).map_err(|_| RowDigestError::ValueTooLong)?;
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(bytes);
    Ok(())
}

/// Append one tagged value. NULL is tag 0 for every kind; otherwise the storage class must
/// match the declared kind exactly.
fn tagged(out: &mut Vec<u8>, kind: Kind, value: Value<'_>) -> Result<(), RowDigestError> {
    match (kind, value) {
        (_, Value::Null) => out.push(0),
        (Kind::Int, Value::Int(v)) => {
            out.push(1);
            out.extend_from_slice(&v.to_be_bytes());
        }
        (Kind::Text, Value::Text(bytes)) => {
            std::str::from_utf8(bytes).map_err(|_| RowDigestError::InvalidUtf8)?;
            out.push(2);
            length_prefixed(out, bytes)?;
        }
        (Kind::Uuid, Value::Blob(bytes)) => {
            if bytes.len() != 16 {
                return Err(RowDigestError::BadUuidLength);
            }
            out.push(3);
            out.extend_from_slice(bytes);
        }
        (Kind::Bytes, Value::Blob(bytes)) => {
            out.push(4);
            length_prefixed(out, bytes)?;
        }
        _ => return Err(RowDigestError::KindMismatch),
    }
    Ok(())
}

/// The `primary_key` bytes: each key column's tagged value, concatenated in the table's
/// declared key-column order.
///
/// # Errors
/// [`RowDigestError`] for an empty key, a kind mismatch, or a key over 256 bytes.
pub fn primary_key(key: &[(Kind, Value<'_>)]) -> Result<Vec<u8>, RowDigestError> {
    if key.is_empty() {
        return Err(RowDigestError::EmptyKey);
    }
    let mut out = Vec::new();
    for (kind, value) in key {
        tagged(&mut out, *kind, *value)?;
        if out.len() > MAX_PRIMARY_KEY {
            return Err(RowDigestError::KeyTooLong);
        }
    }
    Ok(out)
}

fn finish(message: &[u8]) -> [u8; 32] {
    Sha256::digest(message).into()
}

/// The digest of a row that exists: SHA-256 over the label, the table, the column count and
/// every column as name plus tagged value, with the columns sorted by the raw bytes of their
/// names (so a table rebuild that reorders columns does not change it).
///
/// # Errors
/// [`RowDigestError`] naming the first rule broken; no value from the row is in it.
pub fn row_digest(table: &str, columns: &[Column<'_>]) -> Result<[u8; 32], RowDigestError> {
    let mut message = Vec::new();
    label(&mut message, ROW_LABEL);
    name(&mut message, table)?;
    columns_section(&mut message, columns)?;
    Ok(finish(&message))
}

/// `u16 column_count ‖ for each column sorted by name bytes: name ‖ tagged value`, shared by a
/// row and by a folded child row.
fn columns_section(out: &mut Vec<u8>, columns: &[Column<'_>]) -> Result<(), RowDigestError> {
    let count = u16::try_from(columns.len()).map_err(|_| RowDigestError::TooManyColumns)?;
    let mut sorted: Vec<&Column<'_>> = columns.iter().collect();
    sorted.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
    if sorted.windows(2).any(|pair| pair[0].name == pair[1].name) {
        return Err(RowDigestError::DuplicateColumn);
    }
    out.extend_from_slice(&count.to_be_bytes());
    for column in sorted {
        name(out, column.name)?;
        tagged(out, column.kind, column.value)?;
    }
    Ok(())
}

/// The digest of a parent row that has folded children: the row's message followed by the
/// child section `u16 child_table_count ‖ for each child table in raw-name order: text(64)
/// name ‖ u32 count ‖ for each child row: bytes(2^16) child_row`. The child rows of a table
/// are sorted bytewise by their encoding and **duplicates are kept**, so the section is a
/// multiset. The section is always present for a parent whose manifest declares children
/// (`children` lists every declared table, each possibly with 0 rows and a count of 0), and
/// never present for a parent that declares none (use [`row_digest`]).
///
/// # Errors
/// [`RowDigestError::BadChildTables`] if `children` is empty (the spec defines no digest for a
/// parent with undeclared children), names a table twice or is too long; otherwise
/// [`RowDigestError`] naming the first rule broken. No value from a row is in it.
pub fn parent_row_digest(
    table: &str,
    columns: &[Column<'_>],
    children: &[ChildTable<'_>],
) -> Result<[u8; 32], RowDigestError> {
    let mut message = Vec::new();
    label(&mut message, ROW_LABEL);
    name(&mut message, table)?;
    columns_section(&mut message, columns)?;

    if children.is_empty() {
        return Err(RowDigestError::BadChildTables);
    }
    let count = u16::try_from(children.len()).map_err(|_| RowDigestError::BadChildTables)?;
    let mut tables: Vec<&ChildTable<'_>> = children.iter().collect();
    tables.sort_by(|a, b| a.table.as_bytes().cmp(b.table.as_bytes()));
    if tables.windows(2).any(|pair| pair[0].table == pair[1].table) {
        return Err(RowDigestError::BadChildTables);
    }
    message.extend_from_slice(&count.to_be_bytes());
    for child in tables {
        name(&mut message, child.table)?;
        let mut encoded: Vec<Vec<u8>> = Vec::with_capacity(child.rows.len());
        for row in &child.rows {
            let mut bytes = Vec::new();
            columns_section(&mut bytes, row)?;
            if bytes.len() > MAX_CHILD_ROW {
                return Err(RowDigestError::ChildRowTooLong);
            }
            encoded.push(bytes);
        }
        encoded.sort();
        let rows = u32::try_from(encoded.len()).map_err(|_| RowDigestError::BadChildTables)?;
        message.extend_from_slice(&rows.to_be_bytes());
        for bytes in encoded {
            let length = u32::try_from(bytes.len()).map_err(|_| RowDigestError::ChildRowTooLong)?;
            message.extend_from_slice(&length.to_be_bytes());
            message.extend_from_slice(&bytes);
        }
    }
    Ok(finish(&message))
}

/// The digest of a row a command deleted, bound to its table and primary key.
///
/// # Errors
/// [`RowDigestError`] for a bad table name or a key over 256 bytes.
pub fn absent_digest(table: &str, primary_key: &[u8]) -> Result<[u8; 32], RowDigestError> {
    if primary_key.len() > MAX_PRIMARY_KEY {
        return Err(RowDigestError::KeyTooLong);
    }
    let mut message = Vec::new();
    label(&mut message, ABSENT_LABEL);
    name(&mut message, table)?;
    let length = u32::try_from(primary_key.len()).map_err(|_| RowDigestError::KeyTooLong)?;
    message.extend_from_slice(&length.to_be_bytes());
    message.extend_from_slice(primary_key);
    Ok(finish(&message))
}
