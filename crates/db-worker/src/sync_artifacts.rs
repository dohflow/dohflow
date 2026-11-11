//! The device-local artifact mapping (`docs/architecture/vault-format.md` B.2 `sync_artifacts`, C.9;
//! `personal-cfo-0g528.2`): the mapping from a class-1b artifact's service address in an epoch to
//! the local identity that carries it, the ciphertext hash this device verified for it, and, for an
//! attachment received from a peer, the pre-apply staging of the receiver-local crypto fields.
//!
//! The hash is a **cache** for the no-refetch rule and the only hash a producer binds. It never
//! decides a verdict: the live closure at the preceding position does (C.9). The staged fields are
//! consumed by the attach command's `apply` ([`take_staged`]), which clears them in the same
//! transaction that creates the `attachments` row, so the key material has one long-lived copy.

use rusqlite::{params, Connection, OptionalExtension};
use thiserror::Error;

/// What an artifact carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactKind {
    /// An attachment's bytes.
    Attachment,
    /// One part of a source batch.
    BatchPart {
        /// The part's index, `0` to `part_count - 1`.
        index: u32,
        /// How many parts the batch has.
        count: u32,
    },
}

/// One mapping row without its staging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mapping {
    /// The key epoch of the address.
    pub key_epoch: u64,
    /// `sync_artifact_id_e`.
    pub artifact_id: [u8; 32],
    /// The SHA-256 of the artifact's AEAD output that this device verified.
    pub ciphertext_sha256: [u8; 32],
    /// What it carries.
    pub kind: ArtifactKind,
    /// The attachment id or the batch id it binds.
    pub logical_id: [u8; 16],
}

/// The receiver-local crypto fields of a received attachment, from materialization until the attach
/// command's `apply` creates the `attachments` row from them.
#[derive(Clone, PartialEq, Eq)]
pub struct StagedAttachment {
    /// The blob's name under this device's DEK.
    pub storage_id: String,
    /// The content key wrapped under this device's DEK.
    pub wrapped_content_key: Vec<u8>,
    /// The nonce of that wrap.
    pub content_key_nonce: Vec<u8>,
    /// The nonce the blob was encrypted with.
    pub content_nonce: Vec<u8>,
    /// The content algorithm.
    pub content_alg: String,
    /// The plaintext length.
    pub plaintext_size: u64,
    /// The MIME type from the artifact's logical metadata, if it names one.
    pub mime_type: Option<String>,
}

impl core::fmt::Debug for StagedAttachment {
    /// Key material never reaches a log.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("StagedAttachment")
            .field("storage_id", &self.storage_id)
            .finish_non_exhaustive()
    }
}

/// A mapping read back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredMapping {
    /// The row.
    pub mapping: Mapping,
    /// When a durable receipt matching what the client computed was recorded, if one was.
    pub receipt_at: Option<String>,
    /// The staged crypto fields, while the attach command's `apply` has not consumed them.
    pub staged: Option<StagedAttachment>,
}

/// Why a mapping operation failed. Carries no value from a row.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum MappingError {
    /// The database refused the statement (including a CHECK the row violates).
    #[error("the artifact mapping could not be read or written")]
    Store(#[from] rusqlite::Error),
    /// A stored value is out of range for its column (a corrupted row).
    #[error("a stored artifact mapping value is out of range")]
    Corrupt,
}

fn kind_columns(kind: ArtifactKind) -> (i64, Option<i64>, Option<i64>) {
    match kind {
        ArtifactKind::Attachment => (1, None, None),
        ArtifactKind::BatchPart { index, count } => {
            (2, Some(i64::from(index)), Some(i64::from(count)))
        }
    }
}

/// Record that this device verified `mapping`'s ciphertext hash (it published the artifact, or
/// fetched it and the bytes hashed to the bound value), with `staged` when it is a received
/// attachment awaiting its `apply`. An existing row for the address keeps its receipt, takes the new
/// hash, and keeps its staging unless `staged` replaces it.
///
/// # Errors
/// [`MappingError`].
pub fn record(
    conn: &Connection,
    mapping: &Mapping,
    staged: Option<&StagedAttachment>,
) -> Result<(), MappingError> {
    let (kind, part_index, part_count) = kind_columns(mapping.kind);
    let epoch = i64::try_from(mapping.key_epoch).map_err(|_| MappingError::Corrupt)?;
    let size = staged
        .map(|s| i64::try_from(s.plaintext_size).map_err(|_| MappingError::Corrupt))
        .transpose()?;
    conn.execute(
        "INSERT INTO sync_artifacts
             (key_epoch, sync_artifact_id, ciphertext_sha256, kind, logical_id, part_index,
              part_count, staged_storage_id, staged_wrapped_content_key, staged_content_key_nonce,
              staged_content_nonce, staged_content_alg, staged_plaintext_size, staged_mime_type)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
         ON CONFLICT (key_epoch, sync_artifact_id) DO UPDATE SET
             ciphertext_sha256 = excluded.ciphertext_sha256,
             staged_storage_id = COALESCE(excluded.staged_storage_id, staged_storage_id),
             staged_wrapped_content_key =
                 COALESCE(excluded.staged_wrapped_content_key, staged_wrapped_content_key),
             staged_content_key_nonce =
                 COALESCE(excluded.staged_content_key_nonce, staged_content_key_nonce),
             staged_content_nonce = COALESCE(excluded.staged_content_nonce, staged_content_nonce),
             staged_content_alg = COALESCE(excluded.staged_content_alg, staged_content_alg),
             staged_plaintext_size =
                 COALESCE(excluded.staged_plaintext_size, staged_plaintext_size),
             staged_mime_type = COALESCE(excluded.staged_mime_type, staged_mime_type)",
        params![
            epoch,
            mapping.artifact_id.as_slice(),
            mapping.ciphertext_sha256.as_slice(),
            kind,
            mapping.logical_id.as_slice(),
            part_index,
            part_count,
            staged.map(|s| s.storage_id.as_str()),
            staged.map(|s| s.wrapped_content_key.as_slice()),
            staged.map(|s| s.content_key_nonce.as_slice()),
            staged.map(|s| s.content_nonce.as_slice()),
            staged.map(|s| s.content_alg.as_str()),
            size,
            staged.and_then(|s| s.mime_type.as_deref()),
        ],
    )?;
    Ok(())
}

/// Record that the service acknowledged the artifact with a receipt that matched what the client
/// computed, at `now`. Returns whether a row existed.
///
/// # Errors
/// [`MappingError`].
pub fn record_receipt(
    conn: &Connection,
    key_epoch: u64,
    artifact_id: &[u8; 32],
    now: &str,
) -> Result<bool, MappingError> {
    let epoch = i64::try_from(key_epoch).map_err(|_| MappingError::Corrupt)?;
    let changed = conn.execute(
        "UPDATE sync_artifacts SET receipt_at = COALESCE(receipt_at, ?3)
          WHERE key_epoch = ?1 AND sync_artifact_id = ?2",
        params![epoch, artifact_id.as_slice(), now],
    )?;
    Ok(changed == 1)
}

type Row = (
    Vec<u8>,
    i64,
    Vec<u8>,
    Option<i64>,
    Option<i64>,
    Option<String>,
    Option<String>,
    Option<Vec<u8>>,
    Option<Vec<u8>>,
    Option<Vec<u8>>,
    Option<String>,
    Option<i64>,
    Option<String>,
);

fn fixed<const N: usize>(bytes: &[u8]) -> Result<[u8; N], MappingError> {
    bytes.try_into().map_err(|_| MappingError::Corrupt)
}

/// The mapping for `(key_epoch, artifact_id)`, if the device holds one.
///
/// # Errors
/// [`MappingError`].
pub fn lookup(
    conn: &Connection,
    key_epoch: u64,
    artifact_id: &[u8; 32],
) -> Result<Option<StoredMapping>, MappingError> {
    let epoch = i64::try_from(key_epoch).map_err(|_| MappingError::Corrupt)?;
    let row: Option<Row> = conn
        .query_row(
            "SELECT ciphertext_sha256, kind, logical_id, part_index, part_count, receipt_at,
                    staged_storage_id, staged_wrapped_content_key, staged_content_key_nonce,
                    staged_content_nonce, staged_content_alg, staged_plaintext_size,
                    staged_mime_type
               FROM sync_artifacts WHERE key_epoch = ?1 AND sync_artifact_id = ?2",
            params![epoch, artifact_id.as_slice()],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                    r.get(9)?,
                    r.get(10)?,
                    r.get(11)?,
                    r.get(12)?,
                ))
            },
        )
        .optional()?;
    row.map(|row| {
        let (hash, kind, logical, part_index, part_count, receipt_at) =
            (&row.0, row.1, &row.2, row.3, row.4, row.5);
        let kind = match (kind, part_index, part_count) {
            (1, None, None) => ArtifactKind::Attachment,
            (2, Some(index), Some(count)) => ArtifactKind::BatchPart {
                index: u32::try_from(index).map_err(|_| MappingError::Corrupt)?,
                count: u32::try_from(count).map_err(|_| MappingError::Corrupt)?,
            },
            _ => return Err(MappingError::Corrupt),
        };
        let staged = match (&row.6, &row.7, &row.8, &row.9, &row.10, row.11) {
            (Some(storage_id), Some(key), Some(key_nonce), Some(nonce), Some(alg), Some(size)) => {
                Some(StagedAttachment {
                    storage_id: storage_id.clone(),
                    wrapped_content_key: key.clone(),
                    content_key_nonce: key_nonce.clone(),
                    content_nonce: nonce.clone(),
                    content_alg: alg.clone(),
                    plaintext_size: u64::try_from(size).map_err(|_| MappingError::Corrupt)?,
                    mime_type: row.12.clone(),
                })
            }
            (None, None, None, None, None, None) => None,
            _ => return Err(MappingError::Corrupt),
        };
        Ok(StoredMapping {
            mapping: Mapping {
                key_epoch,
                artifact_id: *artifact_id,
                ciphertext_sha256: fixed(hash)?,
                kind,
                logical_id: fixed(logical)?,
            },
            receipt_at,
            staged,
        })
    })
    .transpose()
}

/// Read the staged crypto fields of the attachment artifact `(key_epoch, artifact_id)` and set them
/// to `NULL`, in the caller's transaction: the attach command's `apply` calls this in the same
/// transaction that creates the class-4 `attachments` row from them, so the row is the only
/// long-lived copy. `None` if nothing is staged (a missing mapping at `apply` is
/// `ArtifactMissing`, decided by the caller).
///
/// # Errors
/// [`MappingError`].
pub fn take_staged(
    conn: &Connection,
    key_epoch: u64,
    artifact_id: &[u8; 32],
) -> Result<Option<StagedAttachment>, MappingError> {
    let Some(stored) = lookup(conn, key_epoch, artifact_id)? else {
        return Ok(None);
    };
    let Some(staged) = stored.staged else {
        return Ok(None);
    };
    clear_staged(conn, key_epoch, artifact_id)?;
    Ok(Some(staged))
}

fn clear_staged(
    conn: &Connection,
    key_epoch: u64,
    artifact_id: &[u8; 32],
) -> Result<(), MappingError> {
    let epoch = i64::try_from(key_epoch).map_err(|_| MappingError::Corrupt)?;
    conn.execute(
        "UPDATE sync_artifacts
            SET staged_storage_id = NULL, staged_wrapped_content_key = NULL,
                staged_content_key_nonce = NULL, staged_content_nonce = NULL,
                staged_content_alg = NULL, staged_plaintext_size = NULL, staged_mime_type = NULL
          WHERE key_epoch = ?1 AND sync_artifact_id = ?2",
        params![epoch, artifact_id.as_slice()],
    )?;
    Ok(())
}

/// Why an `attachments` row could not be created from a staged artifact. In every case nothing was
/// written and the staging is untouched; the caller's transaction is rolled back with its command.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CreateAttachmentError {
    /// Nothing is staged for the artifact: no mapping, a batch part, or the staging was already
    /// consumed. The attach command's `apply` reports `ArtifactMissing` (C.9).
    #[error("no staged attachment for this artifact")]
    ArtifactMissing,
    /// The mapping binds a different logical attachment than the command names.
    #[error("the artifact binds another attachment than the one named")]
    WrongAttachment,
    /// An `attachments` row with this id already exists.
    #[error("the attachment already exists")]
    AlreadyExists,
    /// Another attachment already stores a blob under this name (`attachments.storage_id` is
    /// unique), so a second row cannot share it.
    #[error("another attachment already stores this blob")]
    StorageIdHeld,
    /// The mapping could not be read or written.
    #[error(transparent)]
    Mapping(#[from] MappingError),
    /// The database refused the insert.
    #[error("the attachments row could not be written")]
    Store(#[from] crate::DbError),
}

/// Create the class-4 `attachments` row for `attachment_id` from the crypto fields staged for
/// `(key_epoch, artifact_id)` and clear the staging, **in the caller's transaction** (the attach
/// command's `apply`): afterwards the `attachments` row is the only long-lived copy of the key
/// material (C.9, B.2 `sync_artifacts`). `created_at` is the caller's, so a replay is deterministic.
///
/// # Errors
/// [`CreateAttachmentError`]; the caller rolls the transaction back, which leaves the staging.
pub fn create_attachment_from_staged(
    conn: &Connection,
    key_epoch: u64,
    artifact_id: &[u8; 32],
    attachment_id: core_ledger::AttachmentId,
    original_filename: Option<&str>,
    created_at: &str,
) -> Result<(), CreateAttachmentError> {
    let stored =
        lookup(conn, key_epoch, artifact_id)?.ok_or(CreateAttachmentError::ArtifactMissing)?;
    let staged = stored
        .staged
        .ok_or(CreateAttachmentError::ArtifactMissing)?;
    if stored.mapping.kind != ArtifactKind::Attachment
        || stored.mapping.logical_id != *attachment_id.as_uuid().as_bytes()
    {
        return Err(CreateAttachmentError::WrongAttachment);
    }
    if crate::attachments::row_exists(conn, attachment_id)? {
        return Err(CreateAttachmentError::AlreadyExists);
    }
    if crate::attachments::find_id_by_storage_id(conn, &staged.storage_id)?.is_some() {
        return Err(CreateAttachmentError::StorageIdHeld);
    }
    crate::attachments::insert_attachment_fields(
        conn,
        attachment_id,
        &crate::attachments::AttachmentFields {
            storage_id: &staged.storage_id,
            wrapped_content_key: &staged.wrapped_content_key,
            content_key_nonce: &staged.content_key_nonce,
            content_nonce: &staged.content_nonce,
            content_alg: &staged.content_alg,
            plaintext_size: staged.plaintext_size,
            mime_type: staged.mime_type.as_deref(),
            original_filename,
            created_at,
        },
    )?;
    clear_staged(conn, key_epoch, artifact_id)?;
    Ok(())
}
