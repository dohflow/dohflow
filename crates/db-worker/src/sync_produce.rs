//! Producing the class-1b artifacts of a source batch (`vault-format.md` C.3 "Artifact addresses"
//! and C.9; `personal-cfo-1cltt.7`, slice C0).
//!
//! One pipeline, from class-1 rows to a receipt, with nothing between that a caller could get
//! wrong:
//!
//! 1. the batch's carried rows are read by the **same readers the receiver uses**
//!    ([`crate::sync_materialize`]): a batch that is produced and one that is materialized are the
//!    same rows;
//! 2. [`SourceBatch::split`] cuts them into parts, deterministically from the rows alone;
//! 3. each part's address is `storage_id_raw = HMAC-SHA256(addr_subkey_batch, part blob)`
//!    ([`vault_crypto::batch_part_storage_id_raw`]) and its first id is `first_artifact_id(K_e,
//!    storage_id_raw)`;
//! 4. the part is sealed with [`Fixity::First`] (no counter, a zero nonce, deterministic encryption)
//!    and put, and the service's receipt must be the one the sealed bytes produce
//!    ([`vault_crypto::sync_artifact_flow::publish`]);
//! 5. the mapping and the receipt are recorded in `sync_artifacts` (class 3).
//!
//! Because the seal is deterministic, **a retry is an exact retry**: producing the same batch again
//! after a crash, or after a lost response, puts the same bytes and gets the same receipt, never an
//! `ObjectConflict`. Part ids are never stored as an address input: only the ids are kept (C.3).
//!
//! Genesis (`personal-cfo-1cltt.7`) and the push path (`personal-cfo-elciw`) both call this.

use sync_core::artifact::{ArtifactId, ArtifactKind, TransportPlaintext};
use sync_core::artifact_transport::ArtifactTransport;
use sync_core::source_batch::{SourceBatch, SourceBatchError};
use thiserror::Error;
use vault_crypto::batch_part_storage_id_raw;
use vault_crypto::sync_artifact::{first_artifact_id, Fixity};
use vault_crypto::sync_artifact_flow::{publish, PublishError, Published};
use vault_crypto::sync_envelope::EpochKey;

use crate::sync_artifacts::{self, ArtifactKind as MappedKind, Mapping, MappingError};
use crate::sync_materialize::{load_batch, load_records, load_runs, MaterializeError};
use crate::{DbError, DbWorker, WorkerState};

/// One produced artifact: what a snapshot's closure and an envelope's delta name, and what the
/// device keeps beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProducedArtifact {
    /// `sync_artifact_id_e`, the first id of the part under this epoch's key.
    pub id: ArtifactId,
    /// The SHA-256 of the AEAD output the service holds for it (the value a receipt carries and the
    /// binding commitment is taken over).
    pub ciphertext_sha256: [u8; 32],
    /// The batch part it is.
    pub kind: MappedKind,
    /// The batch it belongs to.
    pub logical_id: [u8; 16],
}

/// Why a batch's artifacts were not produced.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ProduceError {
    /// This vault holds no batch with that id.
    #[error("the vault holds no such source batch")]
    NotFound,
    /// A row of the batch could not be read.
    #[error("the batch could not be read: {0}")]
    Rows(#[from] MaterializeError),
    /// The batch cannot be cut into parts (a record over its bound, a value that cannot be written
    /// canonically).
    #[error("the batch cannot be split: {0}")]
    Split(#[from] SourceBatchError),
    /// The vault was opened in passphrase mode: there is no DEK to address under.
    #[error("the vault holds no raw key")]
    Key(#[source] DbError),
    /// A part could not be sealed or put, or the service's receipt is not the sealed bytes'.
    #[error("a part could not be published: {0}")]
    Publish(#[from] PublishError),
    /// The mapping could not be recorded.
    #[error("the mapping could not be recorded: {0}")]
    Mapping(#[from] MappingError),
    /// The vault could not be read, or the worker does not accept writes
    /// ([`DbError::WorkerUnavailable`]): nothing was published from it, or nothing more was.
    #[error("the vault could not be used: {0}")]
    Db(#[from] DbError),
}

impl DbWorker {
    /// Publish every part of the source batch `batch_id` as a class-1b artifact under `key`'s epoch,
    /// and record each mapping and receipt. Returns the parts in order (part 0 first). Idempotent
    /// and deterministic: the same rows give the same ids, bytes and receipts, so a call after a
    /// crash re-puts what was put and a service that holds it answers the same receipt.
    ///
    /// # Errors
    /// A [`ProduceError`]. Parts put before an error stay put (a retry re-puts them identically).
    pub fn produce_batch_artifacts<T: ArtifactTransport>(
        &self,
        transport: &T,
        key: &EpochKey,
        batch_id: &[u8; 16],
    ) -> Result<Vec<ProducedArtifact>, ProduceError> {
        self.produce_batch_with(key, batch_id, &mut |plaintext| {
            publish(transport, key, plaintext, Fixity::First)
        })
    }

    /// The pipeline of [`Self::produce_batch_artifacts`] with the put left to `put_part`: seal and
    /// put one part, answer the published receipt. It refuses before the first put when the worker
    /// does not accept writes (nothing is published from a vault that refuses them), reads the
    /// batch's rows in one read transaction (so they are one view, not three), and records each
    /// part's mapping and receipt in one transaction through [`DbWorker::write_sync_state`], which
    /// refuses an unhealthy worker as every other Sync writer does.
    pub(crate) fn produce_batch_with(
        &self,
        key: &EpochKey,
        batch_id: &[u8; 16],
        put_part: &mut dyn FnMut(&TransportPlaintext<'_>) -> Result<Published, PublishError>,
    ) -> Result<Vec<ProducedArtifact>, ProduceError> {
        self.require_healthy()?;
        let dek = self.dek().map_err(ProduceError::Key)?;
        let batch = {
            let conn = self
                .read_connection()
                .map_err(|e| ProduceError::Rows(e.into()))?;
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| ProduceError::Db(e.into()))?;
            let row = load_batch(&tx, batch_id)?.ok_or(ProduceError::NotFound)?;
            SourceBatch {
                batch: row,
                runs: load_runs(&tx, batch_id)?,
                records: load_records(&tx, batch_id)?,
            }
        };
        let mut produced = Vec::new();
        for part in batch.split()? {
            let storage_id_raw = batch_part_storage_id_raw(dek, &part.blob);
            let id = first_artifact_id(key, &storage_id_raw);
            let kind = MappedKind::BatchPart {
                index: part.index,
                count: part.count,
            };
            let published = put_part(&TransportPlaintext {
                artifact_id: id,
                kind: ArtifactKind::SourceBatchPart,
                logical_metadata: &part.logical_metadata,
                blob: &part.blob,
            })?;
            let ciphertext_sha256 = published.receipt.ciphertext_sha256;
            self.record_produced(
                key,
                &Mapping {
                    key_epoch: key.key_epoch(),
                    artifact_id: *id.as_bytes(),
                    ciphertext_sha256,
                    kind,
                    logical_id: *batch_id,
                },
            )?;
            produced.push(ProducedArtifact {
                id,
                ciphertext_sha256,
                kind,
                logical_id: *batch_id,
            });
        }
        Ok(produced)
    }

    /// Refuse when the worker does not accept writes (a restore in progress, or a panic that left
    /// it needing recovery), as every Sync writer does.
    fn require_healthy(&self) -> Result<(), ProduceError> {
        let state = self.lock().state;
        if state == WorkerState::Healthy {
            Ok(())
        } else {
            Err(ProduceError::Db(DbError::WorkerUnavailable(state)))
        }
    }

    /// Record a produced artifact's mapping and the service's receipt for it, in one transaction.
    fn record_produced(&self, key: &EpochKey, mapping: &Mapping) -> Result<(), ProduceError> {
        let now = chrono::Utc::now().to_rfc3339();
        self.write_sync_state(|tx| -> Result<(), ProduceError> {
            sync_artifacts::record(tx, mapping, None)?;
            sync_artifacts::record_receipt(tx, key.key_epoch(), &mapping.artifact_id, &now)?;
            Ok(())
        })
    }
}
