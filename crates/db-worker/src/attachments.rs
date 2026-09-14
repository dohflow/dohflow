//! Encrypted attachment store (personal-cfo-bcj, ADR 0023).
//!
//! Metadata lives in `attachments` / `attachment_links` (SQLCipher-encrypted at
//! rest); the encrypted bytes live in `<vault>/blobs/`, each file named by its
//! keyed [`vault_crypto::StorageId`]. Every attachment has a per-blob content key
//! wrapped under the DEK (ADR 0002 §5).
//!
//! **No plaintext attachment bytes ever leave the vault.** Import encrypts in
//! memory and writes ciphertext via a `.tmp` sibling *inside* `blobs/` followed
//! by an atomic rename — the OS temp dir is never used. Deletion is a
//! crypto-shred: dropping the metadata row removes the only wrapped copy of the
//! content key, so the blob ciphertext is permanently undecryptable whether or
//! not the file is reclaimed.
//!
//! This module holds the SQL + filesystem helpers; the `DbWorker` methods that
//! orchestrate them (supplying the DEK, the writer lock, and the vault path) live
//! in [`crate`].

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::Utc;
use core_ledger::AttachmentId;
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;
use vault_crypto::{EncryptedBlob, StorageId, WrappedContentKey};

use crate::DbError;

/// The AEAD token recorded per blob (informational; the format is fixed).
pub(crate) const CONTENT_ALG: &str = "AES-256-GCM";

/// AES-GCM nonce length (96-bit), as stored in `content_*_nonce` BLOB columns.
const NONCE_LEN: usize = 12;

/// Display metadata for an attachment — never key material or bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentMeta {
    /// Stable identity (UUIDv7).
    pub id: AttachmentId,
    /// IANA media type, if known at import.
    pub mime_type: Option<String>,
    /// The original filename, stored only inside the encrypted DB.
    pub original_filename: Option<String>,
    /// Plaintext size in bytes.
    pub plaintext_size: u64,
    /// Number of live links from domain entities.
    pub ref_count: u64,
    /// RFC 3339 creation instant.
    pub created_at: String,
}

/// The per-blob crypto fields needed to decrypt an attachment.
pub(crate) struct CryptoFields {
    pub storage_id: String,
    pub wrapped: WrappedContentKey,
    pub content_nonce: [u8; NONCE_LEN],
}

/// The `blobs/` directory beside the vault DB at `db_path`.
pub(crate) fn blobs_dir(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("blobs")
}

/// Write `ciphertext` to `blobs/<storage_id>` atomically and **never via the OS
/// temp dir**: a `.tmp` sibling inside `blobs/` is written, fsynced, then
/// renamed (an atomic overwrite). Dedup is handled at the metadata layer
/// ([`find_id_by_storage_id`]); this is only called on the fresh path, so it
/// always writes — overwriting any crash-orphaned file (which would otherwise
/// hold ciphertext under a now-shredded key) with the current ciphertext.
pub(crate) fn write_blob(
    dir: &Path,
    storage_id: &StorageId,
    ciphertext: &[u8],
) -> Result<(), DbError> {
    fs::create_dir_all(dir)?;
    let final_path = dir.join(storage_id.as_str());
    let tmp_path = dir.join(format!("{}.tmp", storage_id.as_str()));
    {
        let mut file = fs::File::create(&tmp_path)?;
        file.write_all(ciphertext)?;
        file.sync_all()?;
    }
    fs::rename(&tmp_path, &final_path)?;
    Ok(())
}

/// Read the ciphertext for `storage_id` and pair it with its `content_nonce`.
pub(crate) fn read_blob(
    dir: &Path,
    storage_id: &str,
    content_nonce: [u8; NONCE_LEN],
) -> Result<EncryptedBlob, DbError> {
    let ciphertext = fs::read(dir.join(storage_id))?;
    Ok(EncryptedBlob {
        nonce: content_nonce,
        ciphertext,
    })
}

/// Unlink a blob file, treating an already-absent file as success.
pub(crate) fn remove_blob(dir: &Path, storage_id: &str) -> Result<(), DbError> {
    match fs::remove_file(dir.join(storage_id)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(DbError::Io(e)),
    }
}

/// The attachment id already storing `storage_id`, if any (dedup lookup).
pub(crate) fn find_id_by_storage_id(
    conn: &Connection,
    storage_id: &str,
) -> Result<Option<AttachmentId>, DbError> {
    let id: Option<Uuid> = conn
        .query_row(
            "SELECT id FROM attachments WHERE storage_id = ?1",
            params![storage_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(id.map(AttachmentId::from_uuid))
}

/// Insert a new attachment metadata row (ref_count starts at 0; links bump it).
#[allow(clippy::too_many_arguments)]
pub(crate) fn insert_attachment(
    conn: &Connection,
    id: AttachmentId,
    storage_id: &StorageId,
    wrapped: &WrappedContentKey,
    content_nonce: &[u8; NONCE_LEN],
    plaintext_size: u64,
    mime_type: Option<&str>,
    original_filename: Option<&str>,
) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO attachments (
            id, storage_id, wrapped_content_key, content_key_nonce, content_nonce,
            content_alg, plaintext_size, mime_type, original_filename, ref_count,
            created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, ?10)",
        params![
            id.as_uuid(),
            storage_id.as_str(),
            wrapped.ciphertext,
            wrapped.nonce.as_slice(),
            content_nonce.as_slice(),
            CONTENT_ALG,
            i64::try_from(plaintext_size).unwrap_or(i64::MAX),
            mime_type,
            original_filename,
            Utc::now().to_rfc3339(),
        ],
    )?;
    Ok(())
}

/// Insert a link from a domain entity to an attachment.
pub(crate) fn insert_link(
    conn: &Connection,
    id: AttachmentId,
    entity_kind: &str,
    entity_id: Uuid,
) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO attachment_links (attachment_id, entity_kind, entity_id, created_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![
            id.as_uuid(),
            entity_kind,
            entity_id,
            Utc::now().to_rfc3339()
        ],
    )?;
    Ok(())
}

/// Adjust an attachment's `ref_count` by `delta`, returning the new value.
pub(crate) fn adjust_ref_count(
    conn: &Connection,
    id: AttachmentId,
    delta: i64,
) -> Result<i64, DbError> {
    conn.execute(
        "UPDATE attachments SET ref_count = MAX(0, ref_count + ?2) WHERE id = ?1",
        params![id.as_uuid(), delta],
    )?;
    let n: i64 = conn
        .query_row(
            "SELECT ref_count FROM attachments WHERE id = ?1",
            params![id.as_uuid()],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(0);
    Ok(n)
}

/// Delete one link, returning the number of rows removed (0 if no such link).
pub(crate) fn delete_link(
    conn: &Connection,
    id: AttachmentId,
    entity_kind: &str,
    entity_id: Uuid,
) -> Result<usize, DbError> {
    let removed = conn.execute(
        "DELETE FROM attachment_links
         WHERE attachment_id = ?1 AND entity_kind = ?2 AND entity_id = ?3",
        params![id.as_uuid(), entity_kind, entity_id],
    )?;
    Ok(removed)
}

/// Crypto-shred: delete the metadata row (dropping the only wrapped content key)
/// and return its `storage_id` so the caller can unlink the now-undecryptable
/// blob. Returns `None` if the row was already gone.
pub(crate) fn take_storage_id_and_delete(
    conn: &Connection,
    id: AttachmentId,
) -> Result<Option<String>, DbError> {
    let storage_id: Option<String> = conn
        .query_row(
            "SELECT storage_id FROM attachments WHERE id = ?1",
            params![id.as_uuid()],
            |r| r.get(0),
        )
        .optional()?;
    if storage_id.is_some() {
        conn.execute(
            "DELETE FROM attachments WHERE id = ?1",
            params![id.as_uuid()],
        )?;
    }
    Ok(storage_id)
}

/// The crypto fields needed to decrypt attachment `id`.
pub(crate) fn crypto_fields(
    conn: &Connection,
    id: AttachmentId,
) -> Result<Option<CryptoFields>, DbError> {
    conn.query_row(
        "SELECT storage_id, wrapped_content_key, content_key_nonce, content_nonce
         FROM attachments WHERE id = ?1",
        params![id.as_uuid()],
        |r| {
            let storage_id: String = r.get(0)?;
            let ciphertext: Vec<u8> = r.get(1)?;
            let key_nonce: Vec<u8> = r.get(2)?;
            let content_nonce: Vec<u8> = r.get(3)?;
            Ok((storage_id, ciphertext, key_nonce, content_nonce))
        },
    )
    .optional()?
    .map(|(storage_id, ciphertext, key_nonce, content_nonce)| {
        Ok(CryptoFields {
            storage_id,
            wrapped: WrappedContentKey {
                nonce: to_nonce(&key_nonce)?,
                ciphertext,
            },
            content_nonce: to_nonce(&content_nonce)?,
        })
    })
    .transpose()
}

/// The attachments linked to a given domain entity, newest first.
pub(crate) fn list_for(
    conn: &Connection,
    entity_kind: &str,
    entity_id: Uuid,
) -> Result<Vec<AttachmentMeta>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT a.id, a.mime_type, a.original_filename, a.plaintext_size,
                a.ref_count, a.created_at
         FROM attachments a
         JOIN attachment_links l ON l.attachment_id = a.id
         WHERE l.entity_kind = ?1 AND l.entity_id = ?2
         ORDER BY a.created_at DESC",
    )?;
    let rows = stmt.query_map(params![entity_kind, entity_id], |r| {
        let id: Uuid = r.get(0)?;
        Ok(AttachmentMeta {
            id: AttachmentId::from_uuid(id),
            mime_type: r.get(1)?,
            original_filename: r.get(2)?,
            plaintext_size: u64::try_from(r.get::<_, i64>(3)?).unwrap_or(0),
            ref_count: u64::try_from(r.get::<_, i64>(4)?).unwrap_or(0),
            created_at: r.get(5)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Convert a stored nonce BLOB to a fixed-size array, rejecting a wrong length
/// (corruption) rather than panicking.
fn to_nonce(bytes: &[u8]) -> Result<[u8; NONCE_LEN], DbError> {
    bytes
        .try_into()
        .map_err(|_| DbError::SelfTestFailed("attachment nonce has wrong length".into()))
}
