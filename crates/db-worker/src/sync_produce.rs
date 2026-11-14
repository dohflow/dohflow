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
//! 4. the part is sealed (no counter, a zero nonce, deterministic encryption; [`Fixity::First`] the
//!    first time, [`Fixity::Expected`] with the recorded digest every time after), its mapping and
//!    descriptor are recorded in `sync_artifacts` (class 3), and only then is it put; the service's
//!    receipt must be the one the sealed bytes produce
//!    ([`vault_crypto::sync_artifact_flow::put_sealed`]);
//! 5. the receipt is recorded.
//!
//! Because the seal is deterministic **for a fixed plaintext**, a retry is an exact retry: producing
//! the same batch again after a crash, or after a lost response, puts the same bytes and gets the
//! same receipt, never an `ObjectConflict`. C.9's fixed-plaintext rule is kept by the pipeline, not
//! assumed: at an artifact's FIRST seal, before the put, the producer records the SHA-256 of the
//! complete transport plaintext and its `logical_metadata` in the `sync_artifacts` mapping; every
//! later seal of that logical object finds the row **by the logical id** (never by an address
//! recomputed from a storage id a rekey may have changed), rebuilds the plaintext from the stored
//! metadata and the blob, and seals with [`Fixity::Expected`], so a changed plaintext is
//! [`ProduceError::ArtifactDescriptorChanged`], refused before any encryption. Part ids are never
//! stored as an address input: only the ids are kept (C.3).
//!
//! An attachment is one artifact: its plaintext, addressed by its raw storage id
//! ([`vault_crypto::StorageId::raw`]) **and its attachment id** (C.3: the id is sealed into the
//! plaintext, so it is part of the address), with the attachment's id and media type as its logical
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
use vault_crypto::sync_artifact::{
    first_artifact_id, first_attachment_artifact_id, seal_artifact, Fixity, SealArtifactError,
    SealedArtifact,
};
use vault_crypto::sync_artifact_flow::{put_sealed, PublishError, Published};
use vault_crypto::sync_envelope::EpochKey;

use crate::attachments::crypto_fields;
use crate::sync_artifacts::{
    self, ArtifactKind as MappedKind, Descriptor, Mapping, MappingError, StoredMapping,
};
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
    /// A stored logical id is not 16 bytes.
    #[error("a stored logical id is not 16 bytes")]
    BadLogicalId,
    /// An attachment's logical metadata could not be written as canonical JSON.
    #[error("an attachment's logical metadata could not be written")]
    BadMetadata,
    /// An artifact the device is asked to put again has no mapping: nothing says what it is.
    #[error("no mapping is recorded for an artifact to put again")]
    NotMapped(ArtifactId),
    /// The plaintext an artifact would be sealed from is not the one first sealed at its address
    /// (C.9's fixed-plaintext rule): the blob, or the descriptor, changed. Refused before any
    /// encryption, because a second plaintext under one address's key and zero nonce is the one thing
    /// the zero nonce cannot survive.
    #[error("the plaintext at an artifact's address is not the one first sealed")]
    ArtifactDescriptorChanged(ArtifactId),
    /// An artifact put again does not have the hash recorded when it was first sealed: the rows it is
    /// made from changed, which an immutable artifact never does.
    #[error("an artifact put again is not the bytes first sealed")]
    Changed(ArtifactId),
    /// The service holds an object at this address whose hash is not the one this device recorded for
    /// it (C.9's residual class: something else was put there). Nothing is put over it, and the
    /// enable stops: it is never retried.
    #[error("the service holds another object at an artifact's address")]
    ServiceHoldsAnotherObject(ArtifactId),
    /// An artifact the service does not hold can no longer be produced: the attachment or the batch
    /// it is made from is gone from this vault.
    #[error("an artifact the service lacks can no longer be produced: its source is gone")]
    SourceGone(ArtifactId),
    /// The service could not answer which artifacts it holds.
    #[error("the service could not say which artifacts it holds")]
    Status,
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

/// Puts one sealed artifact and answers the verified receipt: [`put_sealed`] over a transport, or a
/// test's own.
pub(crate) type PutSealed<'a> =
    &'a mut dyn FnMut(&ArtifactId, &SealedArtifact) -> Result<Published, PublishError>;

/// One artifact to seal and put.
struct ToSeal<'a> {
    kind: ArtifactKind,
    mapped: MappedKind,
    logical_id: [u8; 16],
    /// The first id of these rows under this epoch's key. Used only at the artifact's FIRST seal:
    /// once a mapping with a descriptor exists, its stored id is the address.
    first_id: Option<ArtifactId>,
    /// The metadata of the current rows. Used only at the first seal, for the same reason.
    metadata: Option<&'a [u8]>,
    blob: &'a [u8],
    /// What this device already sealed for this logical object, if it did.
    stored: Option<&'a StoredMapping>,
}

/// The attachments and batches a genesis closure is made of, read on `conn`: inside the caller's
/// transaction when it holds one, so the answer is the state that transaction sees.
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
            out.push(<[u8; 16]>::try_from(bytes).map_err(|_| ProduceError::BadLogicalId)?);
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

/// The descriptor-bearing mappings of one logical object: what this device itself sealed (a
/// receiver's row has no descriptor and is never re-sealed from).
fn sealed_by_this_device(
    conn: &rusqlite::Connection,
    key: &EpochKey,
    attachment: bool,
    logical_id: &[u8; 16],
) -> Result<Vec<StoredMapping>, ProduceError> {
    Ok(
        sync_artifacts::lookup_logical(conn, key.key_epoch(), attachment, logical_id)?
            .into_iter()
            .filter(|stored| stored.descriptor.is_some())
            .collect(),
    )
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
        self.produce_batch_with(key, batch_id, &mut |id, sealed| {
            put_sealed(transport, key, id, sealed)
        })
    }

    /// The pipeline of [`Self::produce_batch_artifacts`] with the put left to `put`: put one sealed
    /// part and answer the verified receipt. It refuses before the first put when the worker does
    /// not accept writes (nothing is published from a vault that refuses them), reads the batch's
    /// rows in one read transaction (so they are one view, not three), and records each part's
    /// mapping and descriptor **before** its put and its receipt after, each in one transaction
    /// through [`DbWorker::write_sync_state`], which refuses an unhealthy worker as every other Sync
    /// writer does.
    pub(crate) fn produce_batch_with(
        &self,
        key: &EpochKey,
        batch_id: &[u8; 16],
        put: PutSealed<'_>,
    ) -> Result<Vec<ProducedArtifact>, ProduceError> {
        self.require_healthy()?;
        let dek = self.dek().map_err(ProduceError::Key)?;
        let (batch, stored) = {
            let conn = self
                .read_connection()
                .map_err(|e| ProduceError::Rows(e.into()))?;
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| ProduceError::Db(e.into()))?;
            let row = load_batch(&tx, batch_id)?.ok_or(ProduceError::NotFound)?;
            let batch = SourceBatch {
                batch: row,
                runs: load_runs(&tx, batch_id)?,
                records: load_records(&tx, batch_id)?,
            };
            (batch, sealed_by_this_device(&tx, key, false, batch_id)?)
        };
        let mut produced = Vec::new();
        for part in batch.split()? {
            let storage_id_raw = batch_part_storage_id_raw(dek, &part.blob);
            let kind = MappedKind::BatchPart {
                index: part.index,
                count: part.count,
            };
            let earlier = stored.iter().find(|held| {
                matches!(held.mapping.kind, MappedKind::BatchPart { index, .. } if index == part.index)
            });
            produced.push(self.seal_record_put(
                key,
                &ToSeal {
                    kind: ArtifactKind::SourceBatchPart,
                    mapped: kind,
                    logical_id: *batch_id,
                    first_id: Some(first_artifact_id(key, &storage_id_raw)),
                    metadata: Some(&part.logical_metadata),
                    blob: &part.blob,
                    stored: earlier,
                },
                put,
            )?);
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
    /// record its mapping, descriptor and receipt. At its first seal the address is
    /// `first_attachment_artifact_id` (the raw storage id and the attachment id; never the hex text),
    /// and the bytes are its decrypted plaintext; every later call finds the mapping by the
    /// attachment id and seals from the stored descriptor, so a retry is an exact retry and a
    /// changed plaintext is refused before encryption.
    ///
    /// # Errors
    /// A [`ProduceError`].
    pub fn produce_attachment_artifact<T: ArtifactTransport>(
        &self,
        transport: &T,
        key: &EpochKey,
        attachment_id: &[u8; 16],
    ) -> Result<ProducedArtifact, ProduceError> {
        self.produce_attachment_with(key, attachment_id, &mut |id, sealed| {
            put_sealed(transport, key, id, sealed)
        })
    }

    /// The pipeline of [`Self::produce_attachment_artifact`] with the put left to `put`; it refuses
    /// before the put when the worker does not accept writes, and records the mapping and descriptor
    /// before the put and the receipt after it, as [`Self::produce_batch_with`] does.
    pub(crate) fn produce_attachment_with(
        &self,
        key: &EpochKey,
        attachment_id: &[u8; 16],
        put: PutSealed<'_>,
    ) -> Result<ProducedArtifact, ProduceError> {
        self.require_healthy()?;
        let id = AttachmentId::from_uuid(uuid::Uuid::from_bytes(*attachment_id));
        let (storage_id, mime_type, stored) = {
            let conn = self.read_connection()?;
            let fields = crypto_fields(&conn, id)?.ok_or(ProduceError::AttachmentNotFound)?;
            let mime: Option<String> = conn
                .query_row(
                    "SELECT mime_type FROM attachments WHERE id = ?1",
                    [attachment_id.as_slice()],
                    |r| r.get(0),
                )
                .map_err(DbError::from)?;
            let stored = sealed_by_this_device(&conn, key, true, attachment_id)?;
            (fields.storage_id, mime, stored)
        };
        let bytes = self.read_attachment_bytes(id)?;
        let earlier = stored.first();
        // A first seal needs the address and the metadata of the current rows; a later one uses the
        // stored mapping's and never recomputes either (a rekey changes the storage id, and a MIME
        // edit changes the media type, but neither may move an artifact that was sealed).
        let (first_id, metadata) = if earlier.is_some() {
            (None, None)
        } else {
            let raw = vault_crypto::raw_of_hex(&storage_id).ok_or(ProduceError::BadStorageId)?;
            // Canonical JSON, keys ascending (C.9): the attachment's id, and its media type if it
            // has one.
            let metadata = match &mime_type {
                Some(mime) => format!(
                    r#"{{"attachment_id":"{}","mime_type":{}}}"#,
                    uuid::Uuid::from_bytes(*attachment_id).hyphenated(),
                    serde_json::to_string(mime).map_err(|_| ProduceError::BadMetadata)?
                ),
                None => format!(
                    r#"{{"attachment_id":"{}"}}"#,
                    uuid::Uuid::from_bytes(*attachment_id).hyphenated()
                ),
            };
            (
                Some(first_attachment_artifact_id(key, &raw, attachment_id)),
                Some(metadata),
            )
        };
        self.seal_record_put(
            key,
            &ToSeal {
                kind: ArtifactKind::Attachment,
                mapped: MappedKind::Attachment,
                logical_id: *attachment_id,
                first_id,
                metadata: metadata.as_deref().map(str::as_bytes),
                blob: &bytes,
                stored: earlier,
            },
            put,
        )
    }

    /// Seal one artifact, record what it was sealed from, put it, and record the receipt.
    ///
    /// The seal comes first, the record second and the put third, so a crash between any two leaves
    /// the digest (C.9): a retry finds the mapping by the logical id, rebuilds the plaintext from the
    /// stored metadata and seals with [`Fixity::Expected`].
    fn seal_record_put(
        &self,
        key: &EpochKey,
        item: &ToSeal<'_>,
        put: PutSealed<'_>,
    ) -> Result<ProducedArtifact, ProduceError> {
        let earlier = item.stored.and_then(|held| {
            held.descriptor
                .as_ref()
                .map(|descriptor| (held, descriptor))
        });
        let (id, metadata, fixity) = match earlier {
            Some((held, descriptor)) => (
                ArtifactId::from_derived(held.mapping.artifact_id),
                descriptor.logical_metadata.as_slice(),
                Fixity::Expected(descriptor.transport_plaintext_sha256),
            ),
            None => (
                item.first_id.ok_or(ProduceError::AttachmentNotFound)?,
                item.metadata.ok_or(ProduceError::AttachmentNotFound)?,
                Fixity::First,
            ),
        };
        let sealed = seal_artifact(
            key,
            &TransportPlaintext {
                artifact_id: id,
                kind: item.kind,
                logical_metadata: metadata,
                blob: item.blob,
            },
            fixity,
        )
        .map_err(|error| match error {
            SealArtifactError::ArtifactDescriptorChanged => {
                ProduceError::ArtifactDescriptorChanged(id)
            }
            other => ProduceError::Publish(PublishError::Seal(other)),
        })?;
        let ciphertext_sha256 = sealed.receipt.ciphertext_sha256;
        let mapping = Mapping {
            key_epoch: key.key_epoch(),
            artifact_id: *id.as_bytes(),
            ciphertext_sha256,
            kind: item.mapped,
            logical_id: item.logical_id,
        };
        match earlier {
            // Deterministic for a fixed plaintext: the hash is the recorded one. A difference is a
            // corrupted row, not a retry.
            Some((held, _)) if held.mapping.ciphertext_sha256 != ciphertext_sha256 => {
                return Err(ProduceError::Changed(id));
            }
            Some(_) => {}
            None => self.write_sync_state(|tx| -> Result<(), ProduceError> {
                sync_artifacts::record_produced(
                    tx,
                    &mapping,
                    &Descriptor {
                        transport_plaintext_sha256: sealed.plaintext_digest,
                        logical_metadata: metadata.to_vec(),
                    },
                )?;
                Ok(())
            })?,
        }
        let published = put(&id, &sealed)?;
        debug_assert_eq!(published.receipt.ciphertext_sha256, ciphertext_sha256);
        self.record_receipt_of(key, &mapping)?;
        Ok(ProducedArtifact {
            id,
            ciphertext_sha256,
            kind: item.mapped,
            logical_id: item.logical_id,
        })
    }

    /// Produce every artifact a genesis closure names (see [`Self::closure_sources`]) and return
    /// them with the sources they were produced for, ascending by artifact id: the closure's order.
    /// The caller compares the sources with the sources it reads inside its export's read
    /// transaction, to know no attachment or batch arrived in between.
    ///
    /// # Errors
    /// A [`ProduceError`]; what was put before it stays put and a call again re-puts it identically.
    pub fn produce_closure<T: ArtifactTransport>(
        &self,
        transport: &T,
        key: &EpochKey,
    ) -> Result<(ClosureSources, Vec<ProducedArtifact>), ProduceError> {
        self.produce_closure_with(key, &mut |id, sealed| {
            put_sealed(transport, key, id, sealed)
        })
    }

    /// [`Self::produce_closure`] with the put left to `put`.
    pub(crate) fn produce_closure_with(
        &self,
        key: &EpochKey,
        put: PutSealed<'_>,
    ) -> Result<(ClosureSources, Vec<ProducedArtifact>), ProduceError> {
        let sources = self.closure_sources()?;
        let mut produced = Vec::new();
        for attachment in &sources.attachments {
            produced.push(self.produce_attachment_with(key, attachment, put)?);
        }
        for batch in &sources.batches {
            produced.extend(self.produce_batch_with(key, batch, put)?);
        }
        produced.sort_by_key(|p| *p.id.as_bytes());
        Ok((sources, produced))
    }

    /// Make the service hold every artifact in `ids`, which an earlier production sealed: verify what
    /// it already holds, and put again only what it lacks. This is what a resume does before it
    /// sends a saved snapshot, so the service holds every receipt the saved closure names.
    ///
    /// Each id is looked up in the mapping that production recorded, and the service is asked which
    /// it holds (`getArtifactStatus`, in pages of 4096). An id the service holds with the hash this
    /// device recorded is satisfied; one it holds with another hash is
    /// [`ProduceError::ServiceHoldsAnotherObject`] (nothing is put over it, and there is no retry);
    /// one it lacks is produced again **by its logical id through the mapping** (the stored address,
    /// the stored metadata and the recorded digest; never an address recomputed from the current
    /// storage id). An id the service lacks whose attachment or batch is gone is
    /// [`ProduceError::SourceGone`]: the genesis that named it cannot have been installed, because the
    /// service pins the whole closure before the compare-and-swap.
    ///
    /// # Errors
    /// [`ProduceError::NotMapped`] for an id with no mapping, [`ProduceError::Status`] when the service
    /// cannot say, the typed errors above, otherwise as the production.
    pub fn reput_closure<T: ArtifactTransport>(
        &self,
        transport: &T,
        key: &EpochKey,
        ids: &[ArtifactId],
    ) -> Result<(), ProduceError> {
        let mut recorded: Vec<(ArtifactId, [u8; 32], [u8; 16], bool)> = Vec::new();
        {
            let conn = self.read_connection()?;
            for id in ids {
                let stored = sync_artifacts::lookup(&conn, key.key_epoch(), id.as_bytes())?
                    .ok_or(ProduceError::NotMapped(*id))?;
                recorded.push((
                    *id,
                    stored.mapping.ciphertext_sha256,
                    stored.mapping.logical_id,
                    matches!(stored.mapping.kind, MappedKind::Attachment),
                ));
            }
        }
        // What the service holds, asked in pages: a present object is verified against the hash this
        // device recorded; a missing one is put again.
        let mut missing: Vec<usize> = Vec::new();
        for (page, chunk) in recorded.chunks(STATUS_PAGE).enumerate() {
            let asked: Vec<ArtifactId> = chunk.iter().map(|(id, ..)| *id).collect();
            let answers = transport
                .status(key.vault_id(), key.key_epoch(), &asked)
                .map_err(|_| ProduceError::Status)?;
            if answers.len() != asked.len() {
                return Err(ProduceError::Status);
            }
            for (offset, answer) in answers.iter().enumerate() {
                let index = page * STATUS_PAGE + offset;
                let (id, hash, ..) = &recorded[index];
                if answer.artifact_id != *id {
                    return Err(ProduceError::Status);
                }
                if !answer.present {
                    missing.push(index);
                    continue;
                }
                match &answer.receipt {
                    Some(receipt) if receipt.ciphertext_sha256 == *hash => {}
                    Some(_) => return Err(ProduceError::ServiceHoldsAnotherObject(*id)),
                    None => return Err(ProduceError::Status),
                }
            }
        }
        // Put again what is missing, one logical object at a time, by its logical id.
        let mut logical: std::collections::BTreeMap<[u8; 16], (bool, ArtifactId)> =
            std::collections::BTreeMap::new();
        for index in &missing {
            let (id, _, logical_id, is_attachment) = &recorded[*index];
            logical.entry(*logical_id).or_insert((*is_attachment, *id));
        }
        let mut again: Vec<ProducedArtifact> = Vec::new();
        for (logical_id, (is_attachment, first_missing)) in logical {
            let produced = if is_attachment {
                self.produce_attachment_artifact(transport, key, &logical_id)
                    .map(|one| vec![one])
            } else {
                self.produce_batch_artifacts(transport, key, &logical_id)
            };
            match produced {
                Ok(parts) => again.extend(parts),
                Err(ProduceError::AttachmentNotFound | ProduceError::NotFound) => {
                    return Err(ProduceError::SourceGone(first_missing));
                }
                Err(other) => return Err(other),
            }
        }
        for index in missing {
            let (id, hash, ..) = &recorded[index];
            let now = again.iter().find(|p| p.id == *id);
            if now.map(|p| p.ciphertext_sha256) != Some(*hash) {
                return Err(ProduceError::Changed(*id));
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

    /// Record the service's receipt for a produced artifact, in one transaction.
    fn record_receipt_of(&self, key: &EpochKey, mapping: &Mapping) -> Result<(), ProduceError> {
        let now = chrono::Utc::now().to_rfc3339();
        self.write_sync_state(|tx| -> Result<(), ProduceError> {
            sync_artifacts::record_receipt(tx, key.key_epoch(), &mapping.artifact_id, &now)?;
            Ok(())
        })
    }
}

/// How many ids one `getArtifactStatus` call carries (protocol 4.10).
const STATUS_PAGE: usize = 4096;
