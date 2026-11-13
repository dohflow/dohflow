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
//! An attachment is one artifact: its plaintext, addressed by its raw storage id
//! ([`vault_crypto::StorageId::raw`]), with the attachment's id and media type as its logical
//! metadata. [`DbWorker::produce_closure`] produces every artifact a genesis snapshot's closure
//! names: all attachments, and every batch that holds a record a provenance link names (a batch is
//! published at the first commit that references one of its records, C.9, so a batch still being
//! parsed or never committed is not in a closure).
//!
//! Genesis (`personal-cfo-1cltt.7`) and the push path (`personal-cfo-elciw`) both call this.

use core_ledger::AttachmentId;
use sync_core::artifact::{ArtifactId, ArtifactKind, TransportPlaintext};
use sync_core::artifact_transport::ArtifactTransport;
use sync_core::source_batch::{SourceBatch, SourceBatchError};
use thiserror::Error;
use vault_crypto::batch_part_storage_id_raw;
use vault_crypto::sync_artifact::{first_artifact_id, Fixity};
use vault_crypto::sync_artifact_flow::{publish, PublishError, Published};
use vault_crypto::sync_envelope::EpochKey;

use crate::attachments::crypto_fields;
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
    /// The vault could not be read, an attachment's bytes could not be read and decrypted, or the
    /// worker does not accept writes ([`DbError::WorkerUnavailable`]): nothing was published from
    /// it, or nothing more was.
    #[error("the vault could not be used: {0}")]
    Db(#[from] DbError),
    /// This vault holds no attachment with that id.
    #[error("the vault holds no such attachment")]
    AttachmentNotFound,
    /// An attachment's stored address is not 64 lowercase hex digits.
    #[error("an attachment's storage id is not a 32-byte address")]
    BadStorageId,
    /// An artifact the device is asked to put again has no mapping: nothing says what it is.
    #[error("no mapping is recorded for an artifact to put again")]
    NotMapped(ArtifactId),
    /// An artifact put again does not have the hash recorded when it was first put: the rows it is
    /// made from changed, which an immutable artifact never does.
    #[error("an artifact put again is not the bytes first put")]
    Changed(ArtifactId),
}

/// The logical rows a genesis snapshot's closure is made of.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClosureSources {
    /// Every attachment, ascending by id: each is one artifact.
    pub attachments: Vec<[u8; 16]>,
    /// Every batch a provenance link reaches through one of its records, ascending by id: each is
    /// one artifact per part.
    pub batches: Vec<[u8; 16]>,
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

    /// The attachments and batches a genesis closure is made of, read now.
    ///
    /// # Errors
    /// A [`ProduceError`] if the vault cannot be read.
    pub fn closure_sources(&self) -> Result<ClosureSources, ProduceError> {
        closure_sources_in(&self.read_connection()?)
    }

    /// Publish the attachment `attachment_id` as one class-1b artifact under `key`'s epoch, and
    /// record its mapping and receipt. The address is the attachment's raw storage id (never the
    /// hex text); the bytes are its decrypted plaintext; the seal is deterministic, so a retry is an
    /// exact retry.
    ///
    /// # Errors
    /// A [`ProduceError`].
    pub fn produce_attachment_artifact<T: ArtifactTransport>(
        &self,
        transport: &T,
        key: &EpochKey,
        attachment_id: &[u8; 16],
    ) -> Result<ProducedArtifact, ProduceError> {
        self.produce_attachment_with(key, attachment_id, &mut |plaintext| {
            publish(transport, key, plaintext, Fixity::First)
        })
    }

    /// The pipeline of [`Self::produce_attachment_artifact`] with the put left to `put_part`; it
    /// refuses before the put when the worker does not accept writes, and records the mapping and
    /// receipt in one transaction, as [`Self::produce_batch_with`] does.
    pub(crate) fn produce_attachment_with(
        &self,
        key: &EpochKey,
        attachment_id: &[u8; 16],
        put_part: &mut dyn FnMut(&TransportPlaintext<'_>) -> Result<Published, PublishError>,
    ) -> Result<ProducedArtifact, ProduceError> {
        self.require_healthy()?;
        let id = AttachmentId::from_uuid(uuid::Uuid::from_bytes(*attachment_id));
        let (storage_id, mime_type) = {
            let conn = self.read_connection()?;
            let fields = crypto_fields(&conn, id)?.ok_or(ProduceError::AttachmentNotFound)?;
            let mime: Option<String> = conn
                .query_row(
                    "SELECT mime_type FROM attachments WHERE id = ?1",
                    [attachment_id.as_slice()],
                    |r| r.get(0),
                )
                .map_err(DbError::from)?;
            (fields.storage_id, mime)
        };
        let raw = vault_crypto::raw_of_hex(&storage_id).ok_or(ProduceError::BadStorageId)?;
        let bytes = self.read_attachment_bytes(id)?;
        let artifact_id = first_artifact_id(key, &raw);
        // Canonical JSON, keys ascending (C.9): the attachment's id, and its media type if it has one.
        let metadata = match &mime_type {
            Some(mime) => format!(
                r#"{{"attachment_id":"{}","mime_type":{}}}"#,
                uuid::Uuid::from_bytes(*attachment_id).hyphenated(),
                serde_json::to_string(mime).map_err(|_| ProduceError::BadStorageId)?
            ),
            None => format!(
                r#"{{"attachment_id":"{}"}}"#,
                uuid::Uuid::from_bytes(*attachment_id).hyphenated()
            ),
        };
        let published = put_part(&TransportPlaintext {
            artifact_id,
            kind: ArtifactKind::Attachment,
            logical_metadata: metadata.as_bytes(),
            blob: &bytes,
        })?;
        let ciphertext_sha256 = published.receipt.ciphertext_sha256;
        self.record_produced(
            key,
            &Mapping {
                key_epoch: key.key_epoch(),
                artifact_id: *artifact_id.as_bytes(),
                ciphertext_sha256,
                kind: MappedKind::Attachment,
                logical_id: *attachment_id,
            },
        )?;
        Ok(ProducedArtifact {
            id: artifact_id,
            ciphertext_sha256,
            kind: MappedKind::Attachment,
            logical_id: *attachment_id,
        })
    }

    /// Produce every artifact a genesis closure names (see [`Self::closure_sources`]) and return
    /// them with the sources they were produced for, ascending by artifact id: the closure's order.
    /// The caller compares the sources with [`Self::closure_sources`] again after it exports, to
    /// know no attachment or batch arrived in between.
    ///
    /// # Errors
    /// A [`ProduceError`]; what was put before it stays put and a call again re-puts it identically.
    pub fn produce_closure<T: ArtifactTransport>(
        &self,
        transport: &T,
        key: &EpochKey,
    ) -> Result<(ClosureSources, Vec<ProducedArtifact>), ProduceError> {
        let sources = self.closure_sources()?;
        let mut produced = Vec::new();
        for attachment in &sources.attachments {
            produced.push(self.produce_attachment_artifact(transport, key, attachment)?);
        }
        for batch in &sources.batches {
            produced.extend(self.produce_batch_artifacts(transport, key, batch)?);
        }
        produced.sort_by_key(|p| *p.id.as_bytes());
        Ok((sources, produced))
    }

    /// Put again the artifacts named by `ids`, which an earlier production put: each id is looked up
    /// in the mappings that production recorded, its attachment or batch is produced again from the
    /// rows (the seal is deterministic, so this is an exact retry), and the hash must be the one
    /// recorded. This is what a resume does before it sends a saved snapshot, so the service holds
    /// every receipt the saved closure names.
    ///
    /// # Errors
    /// [`ProduceError::NotMapped`] for an id with no mapping, [`ProduceError::Changed`] for an
    /// artifact whose bytes are not the ones first put, otherwise as the production.
    pub fn reput_closure<T: ArtifactTransport>(
        &self,
        transport: &T,
        key: &EpochKey,
        ids: &[ArtifactId],
    ) -> Result<(), ProduceError> {
        let mut logical: std::collections::BTreeMap<[u8; 16], bool> =
            std::collections::BTreeMap::new();
        let mut expected: Vec<(ArtifactId, [u8; 32])> = Vec::new();
        {
            let conn = self.read_connection()?;
            for id in ids {
                let stored = sync_artifacts::lookup(&conn, key.key_epoch(), id.as_bytes())?
                    .ok_or(ProduceError::NotMapped(*id))?;
                logical.insert(
                    stored.mapping.logical_id,
                    matches!(stored.mapping.kind, MappedKind::Attachment),
                );
                expected.push((*id, stored.mapping.ciphertext_sha256));
            }
        }
        let mut again = Vec::new();
        for (logical_id, is_attachment) in logical {
            if is_attachment {
                again.push(self.produce_attachment_artifact(transport, key, &logical_id)?);
            } else {
                again.extend(self.produce_batch_artifacts(transport, key, &logical_id)?);
            }
        }
        for (id, hash) in expected {
            let now = again.iter().find(|p| p.id == id);
            if now.map(|p| p.ciphertext_sha256) != Some(hash) {
                return Err(ProduceError::Changed(id));
            }
        }
        Ok(())
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

/// The attachments and batches a snapshot at the state `conn` reads would carry: every attachment
/// row, and every batch a provenance link reaches through one of its records (never the mere
/// presence of batch rows). **The one reachability rule**: a genesis closure is made of it
/// ([`DbWorker::closure_sources`]) and the live closure's prune removes what it no longer holds
/// ([`crate::sync_live_closure::apply_delta`]); neither has a copy.
///
/// # Errors
/// A [`ProduceError`] if the vault cannot be read.
pub(crate) fn closure_sources_in(
    conn: &rusqlite::Connection,
) -> Result<ClosureSources, ProduceError> {
    let ids = |sql: &str| -> Result<Vec<[u8; 16]>, ProduceError> {
        let mut stmt = conn.prepare(sql).map_err(DbError::from)?;
        let rows = stmt
            .query_map([], |r| r.get::<_, Vec<u8>>(0))
            .map_err(DbError::from)?;
        let mut out = Vec::new();
        for row in rows {
            let bytes = row.map_err(DbError::from)?;
            out.push(<[u8; 16]>::try_from(bytes).map_err(|_| ProduceError::BadStorageId)?);
        }
        Ok(out)
    };
    Ok(ClosureSources {
        attachments: ids("SELECT id FROM attachments ORDER BY id")?,
        batches: ids("SELECT DISTINCT r.source_batch_id
               FROM source_provenance_links l
               JOIN source_records r ON r.id = l.source_record_id
              ORDER BY r.source_batch_id")?,
    })
}
