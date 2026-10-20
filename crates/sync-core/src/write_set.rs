//! The write-set hash of `docs/architecture/vault-format.md` C.4 (personal-cfo-t2s4b part 5c):
//! `SHA-256(label "dohflow/sync-writeset/v1" ‖ u32 count ‖ for each changed row: text(64) table
//! ‖ bytes(256) primary_key ‖ digest row_digest)`.
//!
//! Entries sort by the **raw bytes** of the table name, then by the raw bytes of the primary key
//! (the C.1.5 tagged encoding), bytewise lexicographically with a shorter value first when it is
//! a prefix of the other; they do not sort by their length-prefixed encodings. The count is at
//! most 2^20 and each `(table, primary_key)` pair appears at most once. Which rows are entries,
//! and what their digests are, is the caller's business (C.1.5); this module only assembles and
//! hashes them, so the rule has one implementation and one test.

use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::row_digest::{MAX_NAME, MAX_PRIMARY_KEY};

/// The domain-separation label of the write-set hash.
pub const WRITE_SET_LABEL: &str = "dohflow/sync-writeset/v1";
/// The most entries a write set may hold.
pub const MAX_ENTRIES: usize = 1 << 20;

/// One changed class-1 row: its table, primary key bytes, and the digest of its final state
/// (a row digest, or the deleted-row digest).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The table name (logical name for a projection).
    pub table: String,
    /// The primary key bytes (C.1.5).
    pub primary_key: Vec<u8>,
    /// The digest of the row's final state.
    pub digest: [u8; 32],
}

/// Why a write set could not be hashed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum WriteSetError {
    /// More than 2^20 entries.
    #[error("a write set has too many entries")]
    TooManyEntries,
    /// The same `(table, primary_key)` appears twice.
    #[error("a write set names the same row twice")]
    DuplicateEntry,
    /// A table name is empty or over 64 bytes, or a primary key is empty or over 256 bytes.
    #[error("a write-set entry has an invalid table name or key length")]
    BadEntry,
}

/// Sort `entries` into the canonical order and compute the write-set hash.
///
/// # Errors
/// [`WriteSetError`] for too many entries, a duplicate row, or an invalid entry.
pub fn write_set_hash(entries: &mut [Entry]) -> Result<[u8; 32], WriteSetError> {
    if entries.len() > MAX_ENTRIES {
        return Err(WriteSetError::TooManyEntries);
    }
    for entry in entries.iter() {
        if entry.table.is_empty()
            || entry.table.len() > MAX_NAME
            || entry.primary_key.is_empty()
            || entry.primary_key.len() > MAX_PRIMARY_KEY
        {
            return Err(WriteSetError::BadEntry);
        }
    }
    entries.sort_by(|a, b| {
        a.table
            .as_bytes()
            .cmp(b.table.as_bytes())
            .then_with(|| a.primary_key.cmp(&b.primary_key))
    });
    if entries
        .windows(2)
        .any(|pair| pair[0].table == pair[1].table && pair[0].primary_key == pair[1].primary_key)
    {
        return Err(WriteSetError::DuplicateEntry);
    }
    let mut message = Vec::new();
    let label = WRITE_SET_LABEL.as_bytes();
    message.extend_from_slice(
        &u16::try_from(label.len())
            .expect("short label")
            .to_be_bytes(),
    );
    message.extend_from_slice(label);
    let count = u32::try_from(entries.len()).map_err(|_| WriteSetError::TooManyEntries)?;
    message.extend_from_slice(&count.to_be_bytes());
    for entry in entries.iter() {
        let table = entry.table.as_bytes();
        message.extend_from_slice(&u32::try_from(table.len()).expect("checked").to_be_bytes());
        message.extend_from_slice(table);
        message.extend_from_slice(
            &u32::try_from(entry.primary_key.len())
                .expect("checked")
                .to_be_bytes(),
        );
        message.extend_from_slice(&entry.primary_key);
        message.extend_from_slice(&entry.digest);
    }
    Ok(Sha256::digest(&message).into())
}
