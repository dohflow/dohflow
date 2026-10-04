//! Vault key rotation, database side (ADR 0083 §2–§3, personal-cfo-2y8).
//!
//! Rotation never converts the live vault in place. This module builds a
//! **re-encrypted copy** beside it and verifies that copy; the Finance Kernel
//! owns the journal, the envelope, the blob links and the swap (ADR 0083 §2).
//!
//! The copy is made with SQLCipher's `sqlcipher_export` into an attached
//! database keyed with the new DEK, in rollback-journal mode, so that every
//! byte of it lands in the copy's own file — nothing can be stranded in a
//! `-wal` that a later rename would leave behind (ADR 0083 §2, "finalize the
//! copy as one closed file"). Inside the copy, each attachment's content key is
//! re-wrapped under the new DEK and its storage id recomputed under the new
//! addressing subkey (§3). Blob ciphertext and content keys never change.

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OpenFlags};
use uuid::Uuid;
use vault_crypto::{Dek, WrappedContentKey};
use zeroize::{Zeroize, Zeroizing};

use crate::{attachments, migrations, DbError, DbWorker, WorkerState};

/// One attachment blob's on-disk name before and after re-addressing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobRename {
    /// The storage id (file name) under the current DEK.
    pub old: String,
    /// The storage id (file name) under the new DEK.
    pub new: String,
}

/// The side files SQLite may create beside a database file.
const SIDE_SUFFIXES: [&str; 3] = ["-wal", "-shm", "-journal"];

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut os = path.as_os_str().to_owned();
    os.push(suffix);
    PathBuf::from(os)
}

/// Fail unless `path` has no `-wal`, `-shm` or `-journal` sibling.
fn assert_no_side_files(path: &Path) -> Result<(), DbError> {
    for suffix in SIDE_SUFFIXES {
        if with_suffix(path, suffix).exists() {
            return Err(DbError::InvalidCommand(format!(
                "rekeyed copy left a {suffix} side file"
            )));
        }
    }
    Ok(())
}

/// Build the raw-key SQL fragment `"x'<64 hex>'"`. The caller must zeroize the
/// returned string after use, as `key_raw` does for `PRAGMA key`.
fn raw_key_literal(dek: &Dek) -> Zeroizing<String> {
    use std::fmt::Write as _;
    let mut literal = Zeroizing::new(String::with_capacity(70));
    literal.push_str("\"x'");
    for byte in dek.expose_bytes() {
        let _ = write!(literal, "{byte:02x}");
    }
    literal.push_str("'\"");
    literal
}

/// Open `path` keyed with `dek`, in rollback-journal mode, never creating it.
fn open_copy(path: &Path, dek: &Dek, flags: OpenFlags) -> Result<Connection, DbError> {
    let conn = Connection::open_with_flags(path, flags)?;
    crate::key_raw(&conn, dek)?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    Ok(conn)
}

impl DbWorker {
    /// Build a re-encrypted copy of this vault at `dest`, keyed with `new_dek`
    /// (ADR 0083 §2 "Prepare"), and return every attachment's old and new blob
    /// name. The live vault is only read.
    ///
    /// Holds the writer lock throughout, so no write can land between the WAL
    /// checkpoint and the export. On return the copy is one closed file with
    /// no side files, its `user_version` equals the live vault's, and every
    /// attachment row in it is re-wrapped and re-addressed under `new_dek`.
    /// Blob files are not touched: the caller links them under their new names
    /// once its journal lists them.
    ///
    /// # Errors
    /// [`DbError::KeyUnavailable`] in passphrase mode;
    /// [`DbError::WorkerUnavailable`] if the writer is not healthy;
    /// [`DbError::InvalidCommand`] if `dest` already exists or the copy leaves a
    /// side file; [`DbError::Sqlite`] / [`DbError::Crypto`] / [`DbError::Io`]
    /// on an export, crypto or blob-read failure. On error `dest` may exist and
    /// is the caller's to remove.
    pub fn prepare_rekeyed_copy(
        &self,
        new_dek: &Dek,
        dest: &Path,
    ) -> Result<Vec<BlobRename>, DbError> {
        let old_dek = self.dek()?;
        // Create the destination as an empty file first, atomically refusing an
        // existing one. The writer of an unlocked vault is opened without
        // SQLITE_OPEN_CREATE, and an ATTACH inherits its connection's flags, so
        // it cannot create the file itself; SQLCipher treats an empty file as a
        // new, empty database to key and export into.
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dest)
        {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(DbError::InvalidCommand(
                    "rekey copy destination already exists".to_owned(),
                ))
            }
            Err(error) => return Err(error.into()),
        }
        let blobs = attachments::blobs_dir(&self.path);
        let guard = self.lock();
        if guard.state != WorkerState::Healthy {
            return Err(DbError::WorkerUnavailable(guard.state));
        }
        let conn = &guard.conn;
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        let user_version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;

        // Export into an attached database keyed with the new raw DEK. An
        // attached database keeps SQLite's default rollback journal, so the
        // export's writes go straight into `dest`.
        let dest_str = dest
            .to_str()
            .ok_or_else(|| DbError::InvalidCommand("rekey copy path is not UTF-8".to_owned()))?;
        let mut attach = Zeroizing::new(String::from("ATTACH DATABASE ?1 AS rekeyed KEY "));
        attach.push_str(&raw_key_literal(new_dek));
        let attached = conn.execute(&attach, params![dest_str]);
        attach.zeroize();
        attached?;
        let exported = conn
            .query_row("SELECT sqlcipher_export('rekeyed')", [], |_| Ok(()))
            .map_err(DbError::from)
            .and_then(|()| {
                // `sqlcipher_export` copies schema and rows; the version marker
                // is set explicitly so the copy can never read as a different
                // schema than the vault it came from.
                conn.execute_batch(&format!("PRAGMA rekeyed.user_version = {user_version};"))
                    .map_err(DbError::from)
            });
        let detached = conn.execute_batch("DETACH DATABASE rekeyed;");
        exported?;
        detached?;
        drop(guard);

        let renames = rewrap_attachments(dest, old_dek, new_dek, &blobs)?;
        assert_no_side_files(dest)?;
        Ok(renames)
    }
}

/// Re-wrap and re-address every attachment row inside the copy at `dest`, in
/// one transaction, and close the copy (ADR 0083 §3).
fn rewrap_attachments(
    dest: &Path,
    old_dek: &Dek,
    new_dek: &Dek,
    blobs: &Path,
) -> Result<Vec<BlobRename>, DbError> {
    let mut conn = open_copy(dest, new_dek, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    conn.pragma_update(None, "journal_mode", "DELETE")?;
    let tx = conn.transaction()?;
    let rows: Vec<(Uuid, String)> = {
        let mut stmt = tx.prepare("SELECT id, storage_id FROM attachments ORDER BY storage_id")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    let mut renames = Vec::with_capacity(rows.len());
    for (id, old_storage_id) in rows {
        let fields = attachments::crypto_fields(&tx, core_ledger::AttachmentId::from_uuid(id))?
            .ok_or(DbError::AttachmentNotFound)?;
        let content_key = vault_crypto::unwrap_content_key(old_dek, &fields.wrapped)?;
        let blob = attachments::read_blob(blobs, &old_storage_id, fields.content_nonce)?;
        let plaintext = Zeroizing::new(vault_crypto::decrypt_blob(&content_key, &blob)?);
        let new_storage_id = vault_crypto::storage_id(new_dek, &plaintext);
        let rewrapped: WrappedContentKey = vault_crypto::wrap_content_key(new_dek, &content_key)?;
        tx.execute(
            "UPDATE attachments
                SET storage_id = ?1, wrapped_content_key = ?2, content_key_nonce = ?3
              WHERE id = ?4",
            params![
                new_storage_id.as_str(),
                rewrapped.ciphertext,
                rewrapped.nonce.to_vec(),
                id
            ],
        )?;
        renames.push(BlobRename {
            old: old_storage_id,
            new: new_storage_id.as_str().to_owned(),
        });
    }
    tx.commit()?;
    drop(conn);
    Ok(renames)
}

/// Verify a prepared rekey copy before it is committed (ADR 0083 §2): it opens
/// read-only under `new_dek`, passes `PRAGMA integrity_check`, passes the same
/// existing-schema inspection every unlock runs, carries `expected_user_version`,
/// and every attachment row unwraps under `new_dek` and names a present file in
/// `blobs`. The copy is opened without changing its journal mode and is left
/// with no side files.
///
/// # Errors
/// [`DbError::InvalidCommand`] naming the failed check, or the underlying
/// [`DbError`] from opening, keying, or reading the copy.
pub fn verify_rekeyed_copy(
    copy: &Path,
    new_dek: &Dek,
    blobs: &Path,
    expected_user_version: i64,
) -> Result<(), DbError> {
    let conn = open_copy(copy, new_dek, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let integrity: String = conn.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    if integrity != "ok" {
        return Err(DbError::InvalidCommand(
            "rekeyed copy failed integrity_check".to_owned(),
        ));
    }
    migrations::inspect_existing(&conn)?;
    let user_version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if user_version != expected_user_version {
        return Err(DbError::InvalidCommand(format!(
            "rekeyed copy user_version {user_version} != {expected_user_version}"
        )));
    }
    let rows: Vec<(Uuid, String)> = {
        let mut stmt = conn.prepare("SELECT id, storage_id FROM attachments")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    for (id, storage_id) in rows {
        let fields = attachments::crypto_fields(&conn, core_ledger::AttachmentId::from_uuid(id))?
            .ok_or(DbError::AttachmentNotFound)?;
        vault_crypto::unwrap_content_key(new_dek, &fields.wrapped)?;
        if !blobs.join(&storage_id).is_file() {
            return Err(DbError::InvalidCommand(
                "rekeyed copy names a missing blob".to_owned(),
            ));
        }
    }
    drop(conn);
    assert_no_side_files(copy)
}

/// Free space, in bytes, on the filesystem holding `dir` (ADR 0083 §5 precheck).
///
/// # Errors
/// [`DbError::Io`] if the filesystem cannot be queried.
pub fn available_space(dir: &Path) -> Result<u64, DbError> {
    Ok(fs2::available_space(dir)?)
}

/// The vault's current `user_version` marker, read through the writer.
impl DbWorker {
    /// The live vault's `PRAGMA user_version` (ADR 0083 §2: the rekeyed copy
    /// must carry it exactly).
    ///
    /// # Errors
    /// [`DbError::Sqlite`] if the pragma cannot be read.
    pub fn user_version(&self) -> Result<i64, DbError> {
        Ok(self
            .lock()
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))?)
    }

    /// Total bytes of the live database file (for the §5 disk precheck).
    ///
    /// # Errors
    /// [`DbError::Io`] if the file cannot be stat'ed.
    pub fn database_size(&self) -> Result<u64, DbError> {
        Ok(std::fs::metadata(&self.path)?.len())
    }

    /// Total bytes of the attachment blob files (for the §5 disk precheck when
    /// hard links are unavailable). A missing blob directory is zero.
    ///
    /// # Errors
    /// [`DbError::Io`] if the directory cannot be read.
    pub fn blobs_size(&self) -> Result<u64, DbError> {
        let dir = attachments::blobs_dir(&self.path);
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(error.into()),
        };
        let mut total = 0u64;
        for entry in entries {
            let metadata = entry?.metadata()?;
            if metadata.is_file() {
                total = total.saturating_add(metadata.len());
            }
        }
        Ok(total)
    }
}
