//! The receiver's materialization of class-1b artifacts (`docs/architecture/vault-format.md` C.9;
//! `personal-cfo-0g528.2`): what a device writes, **outside any command**, after it has fetched,
//! hash-checked, opened and judged the artifacts an envelope references, and before that envelope is
//! applied.
//!
//! Materialization fails only for a reason local to the device (the disk, the DEK, a write): every
//! deterministic defect of an artifact or a batch was found at stage 9(b) first, so no replica
//! reaches a failure that another does not. A failure leaves the live vault unchanged and removes
//! the files it created.
//!
//! * [`DbWorker::materialize_attachment`] re-encrypts the bytes under this device's DEK with a fresh
//!   content key, names the blob by this device's `StorageId`, writes it, and records the mapping with
//!   the crypto fields **staged** for the attach command's `apply` (it never writes the class-4
//!   `attachments` row).
//! * [`DbWorker::materialize_batch`] writes the batch, record and parser-run rows of a decoded source
//!   batch, with the device-local columns the spec gives a received batch, and records the mapping
//!   of every part.

use rusqlite::{params, OptionalExtension};
use sync_core::source_batch::SourceBatch;
use thiserror::Error;

use crate::sync_artifacts::{record, ArtifactKind, Mapping, MappingError, StagedAttachment};
use crate::{attachments, DbError, DbWorker, WorkerState};

/// Why an artifact could not be materialized. Carries no value from an artifact.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum MaterializeError {
    /// A reason local to the device: the disk, the key, a write
    /// (`ArtifactMaterializationFailed`, C.9). The vault is unchanged.
    #[error("the artifact could not be materialized on this device")]
    Local(#[from] DbError),
    /// The mapping could not be written.
    #[error("the artifact mapping could not be written")]
    Mapping(#[from] MappingError),
    /// Another attachment, or another artifact's staging, already stores a blob under the name these
    /// bytes get (`attachments.storage_id` and the staged storage id are unique). Two attachment ids
    /// carrying identical bytes in one vault are not representable.
    #[error("another attachment already stores a blob with these bytes")]
    BlobHeld,
    /// Rows for this id are already in the vault and are not the rows being materialized.
    #[error("rows already in the vault differ from the batch being materialized")]
    RowsDiffer,
}

impl From<rusqlite::Error> for MaterializeError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Local(DbError::from(error))
    }
}

/// One part of a batch as it was fetched: its address, the hash this device verified for it, and its
/// place in the batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchPartRef {
    /// `sync_artifact_id_e`.
    pub artifact_id: [u8; 32],
    /// The SHA-256 of the part's AEAD output that the fetch verified.
    pub ciphertext_sha256: [u8; 32],
    /// The part's index, `0` to `count - 1`.
    pub index: u32,
    /// How many parts the batch has.
    pub count: u32,
}

impl DbWorker {
    /// Materialize the attachment artifact `(key_epoch, artifact_id)` whose verified plaintext is
    /// `bytes`: encrypt them under this device's DEK with a fresh content key, write the blob under
    /// this device's storage id, and record the mapping with the crypto fields staged. Idempotent: an
    /// artifact already materialized (its blob present and its mapping staged or consumed) is left as
    /// it is. An existing blob with the same storage id is reused, never overwritten.
    ///
    /// # Errors
    /// [`MaterializeError`]; the vault is unchanged and a blob this call created is removed.
    pub fn materialize_attachment(
        &self,
        key_epoch: u64,
        artifact_id: &[u8; 32],
        ciphertext_sha256: &[u8; 32],
        attachment_id: [u8; 16],
        bytes: &[u8],
        mime_type: Option<&str>,
    ) -> Result<(), MaterializeError> {
        let dek = self.dek()?;
        let storage_id = vault_crypto::storage_id(dek, bytes);
        let dir = attachments::blobs_dir(&self.path);
        let mut guard = self.lock();
        if guard.state != WorkerState::Healthy {
            return Err(DbError::WorkerUnavailable(guard.state).into());
        }
        let mapping = Mapping {
            key_epoch,
            artifact_id: *artifact_id,
            ciphertext_sha256: *ciphertext_sha256,
            kind: ArtifactKind::Attachment,
            logical_id: attachment_id,
        };

        // This artifact is already here: its blob is on disk and its mapping is staged, or the apply
        // consumed the staging and the attachments row exists. Only the verified hash is refreshed.
        let existing = crate::sync_artifacts::lookup(&guard.conn, key_epoch, artifact_id)?;
        let staged_here = existing.as_ref().and_then(|m| m.staged.as_ref());
        let attachment_row = attachments::find_id_by_storage_id(&guard.conn, storage_id.as_str())?;
        let row_is_this_attachment =
            attachment_row.is_some_and(|row| *row.as_uuid().as_bytes() == attachment_id);
        if (staged_here.is_some_and(|s| s.storage_id == storage_id.as_str())
            && dir.join(storage_id.as_str()).is_file())
            || row_is_this_attachment
        {
            record(&guard.conn, &mapping, None)?;
            return Ok(());
        }
        // Another attachment, or another artifact's staging, already holds these bytes' name.
        if attachment_row.is_some() {
            return Err(MaterializeError::BlobHeld);
        }
        let staged_elsewhere: bool = guard
            .conn
            .query_row(
                "SELECT 1 FROM sync_artifacts WHERE staged_storage_id = ?1
                    AND NOT (key_epoch = ?2 AND sync_artifact_id = ?3)",
                params![
                    storage_id.as_str(),
                    i64::try_from(key_epoch).map_err(|_| MappingError::Corrupt)?,
                    artifact_id.as_slice()
                ],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if staged_elsewhere {
            return Err(MaterializeError::BlobHeld);
        }

        let content_key = vault_crypto::generate_content_key().map_err(DbError::from)?;
        let blob = vault_crypto::encrypt_blob(&content_key, bytes).map_err(DbError::from)?;
        let wrapped = vault_crypto::wrap_content_key(dek, &content_key).map_err(DbError::from)?;
        // Nothing references a blob under this name (an attachments row and another artifact's
        // staging were refused above, and this artifact's own was handled), so a file here is an
        // orphan of an attempt that died before its mapping was recorded: it is replaced.
        attachments::write_blob(&dir, &storage_id, &blob.ciphertext)?;
        let staged = StagedAttachment {
            storage_id: storage_id.as_str().to_owned(),
            wrapped_content_key: wrapped.ciphertext.clone(),
            content_key_nonce: wrapped.nonce.to_vec(),
            content_nonce: blob.nonce.to_vec(),
            content_alg: attachments::CONTENT_ALG.to_owned(),
            plaintext_size: bytes.len() as u64,
            mime_type: mime_type.map(str::to_owned),
        };
        let recorded = (|| -> Result<(), MaterializeError> {
            let tx = guard.conn.transaction()?;
            record(&tx, &mapping, Some(&staged))?;
            tx.commit()?;
            Ok(())
        })();
        if let Err(error) = recorded {
            // The write failed, so the mapping does not exist and nothing references the file.
            let _ = attachments::remove_blob(&dir, storage_id.as_str());
            return Err(error);
        }
        Ok(())
    }

    /// Materialize a decoded source batch: its `source_batches`, `parser_runs` and `source_records`
    /// rows, with the device-local columns of a received batch (`status` `committed`, counts `0`,
    /// `updated_at` equal to `created_at`), and the mapping of every part, in one transaction.
    /// Idempotent: rows already in the vault that equal the batch's are left; a row that differs is
    /// [`MaterializeError::RowsDiffer`] and nothing is written.
    ///
    /// # Errors
    /// [`MaterializeError`].
    pub fn materialize_batch(
        &self,
        key_epoch: u64,
        parts: &[BatchPartRef],
        batch: &SourceBatch,
    ) -> Result<(), MaterializeError> {
        let mut guard = self.lock();
        if guard.state != WorkerState::Healthy {
            return Err(DbError::WorkerUnavailable(guard.state).into());
        }
        let tx = guard.conn.transaction()?;
        let b = &batch.batch;
        let batch_changed = tx.execute(
            "INSERT INTO source_batches
                 (id, source_type, source_name, file_fingerprint, parser_version, status,
                  staged_count, committed_count, skipped_count, summary_json, imported_at,
                  created_at, updated_at, connector_connection_id)
             VALUES (?1, ?2, ?3, ?4, ?5, 'committed', 0, 0, 0, ?6, ?7, ?8, ?8, ?9)
             ON CONFLICT (id) DO NOTHING",
            params![
                b.id.as_slice(),
                b.source_type,
                b.source_name,
                b.file_fingerprint,
                b.parser_version,
                b.summary_json,
                b.imported_at,
                b.created_at,
                b.connector_connection_id.as_ref().map(<[u8; 16]>::as_slice),
            ],
        )?;
        if batch_changed == 0 {
            let same: bool = tx.query_row(
                "SELECT source_type IS ?2 AND source_name IS ?3 AND file_fingerprint IS ?4
                            AND parser_version IS ?5 AND summary_json IS ?6 AND imported_at IS ?7
                            AND created_at IS ?8 AND connector_connection_id IS ?9
                       FROM source_batches WHERE id = ?1",
                params![
                    b.id.as_slice(),
                    b.source_type,
                    b.source_name,
                    b.file_fingerprint,
                    b.parser_version,
                    b.summary_json,
                    b.imported_at,
                    b.created_at,
                    b.connector_connection_id.as_ref().map(<[u8; 16]>::as_slice),
                ],
                |r| r.get(0),
            )?;
            if !same {
                return Err(MaterializeError::RowsDiffer);
            }
        }
        for run in &batch.runs {
            let changed = tx.execute(
                "INSERT INTO parser_runs
                     (id, source_batch_id, parser_name, parser_version, bytes_in, records_out,
                      status, limit_hit, started_at, finished_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT (id) DO NOTHING",
                params![
                    run.id.as_slice(),
                    run.source_batch_id.as_slice(),
                    run.parser_name,
                    run.parser_version,
                    run.bytes_in,
                    run.records_out,
                    run.status,
                    run.limit_hit,
                    run.started_at,
                    run.finished_at,
                ],
            )?;
            if changed == 0 {
                let same: bool = tx.query_row(
                    "SELECT source_batch_id IS ?2 AND parser_name IS ?3 AND parser_version IS ?4
                            AND bytes_in IS ?5 AND records_out IS ?6 AND status IS ?7
                            AND limit_hit IS ?8 AND started_at IS ?9 AND finished_at IS ?10
                       FROM parser_runs WHERE id = ?1",
                    params![
                        run.id.as_slice(),
                        run.source_batch_id.as_slice(),
                        run.parser_name,
                        run.parser_version,
                        run.bytes_in,
                        run.records_out,
                        run.status,
                        run.limit_hit,
                        run.started_at,
                        run.finished_at,
                    ],
                    |r| r.get(0),
                )?;
                if !same {
                    return Err(MaterializeError::RowsDiffer);
                }
            }
        }
        for rec in &batch.records {
            let changed = tx.execute(
                "INSERT INTO source_records
                     (id, source_batch_id, external_id, source_hash, normalized_json,
                      parse_confidence_bps, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT (id) DO NOTHING",
                params![
                    rec.id.as_slice(),
                    rec.source_batch_id.as_slice(),
                    rec.external_id,
                    rec.source_hash,
                    rec.normalized_json,
                    rec.parse_confidence_bps,
                    rec.created_at,
                ],
            )?;
            if changed == 0 {
                let same: bool = tx.query_row(
                    "SELECT source_batch_id IS ?2 AND external_id IS ?3 AND source_hash IS ?4
                            AND normalized_json IS ?5 AND parse_confidence_bps IS ?6
                            AND created_at IS ?7
                       FROM source_records WHERE id = ?1",
                    params![
                        rec.id.as_slice(),
                        rec.source_batch_id.as_slice(),
                        rec.external_id,
                        rec.source_hash,
                        rec.normalized_json,
                        rec.parse_confidence_bps,
                        rec.created_at,
                    ],
                    |r| r.get(0),
                )?;
                if !same {
                    return Err(MaterializeError::RowsDiffer);
                }
            }
        }
        for part in parts {
            record(
                &tx,
                &Mapping {
                    key_epoch,
                    artifact_id: part.artifact_id,
                    ciphertext_sha256: part.ciphertext_sha256,
                    kind: ArtifactKind::BatchPart {
                        index: part.index,
                        count: part.count,
                    },
                    logical_id: b.id,
                },
                None,
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}
