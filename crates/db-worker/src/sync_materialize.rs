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
use sync_core::artifact_closure::{LiveClosure, LogicalRef};
use sync_core::source_batch::{BatchRow, RecordRow, RunRow, SourceBatch};
use thiserror::Error;
use vault_crypto::sync_artifact::OpenedArtifact;
use vault_crypto::sync_artifact_flow as flow;

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
    /// The artifact is not a valid attachment (its metadata names no attachment id). A deterministic
    /// defect that stage 9(b) rejects before materialization; reaching it is a caller error.
    #[error("the artifact is not a valid attachment")]
    NotAnAttachment,
    /// Local encryption failed.
    #[error("local encryption failed")]
    Crypto(flow::MaterializeError),
    /// A replicated row (a provenance link) names a record of the batch being replaced, so the batch
    /// is live by definition; replacing it would orphan that row. An invariant: the live closure
    /// passed in should hold the batch.
    #[error("a live row names a record of the batch being replaced")]
    LiveReference,
    /// Rows for this id are already in the vault and are not the rows being materialized.
    #[error("rows already in the vault differ from the batch being materialized")]
    RowsDiffer,
}

impl From<flow::MaterializeError> for MaterializeError {
    fn from(error: flow::MaterializeError) -> Self {
        match error {
            flow::MaterializeError::NotAnAttachment => Self::NotAnAttachment,
            other => Self::Crypto(other),
        }
    }
}

/// The MIME type an attachment artifact's logical metadata names, if it names one.
fn mime_type_of(logical_metadata: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(logical_metadata).ok()?;
    value.get("mime_type")?.as_str().map(str::to_owned)
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
    /// Materialize the verified attachment artifact `opened`: encrypt its bytes under this device's
    /// DEK with a fresh content key (the pure step is `vault_crypto`'s `materialize_attachment`),
    /// write the blob under this device's storage id, and record the mapping with the crypto fields
    /// staged. The attachment id comes from the artifact's own metadata, never from the caller.
    /// `ciphertext_sha256` is the hash the fetch verified for the artifact. Idempotent: an artifact
    /// already materialized (its blob present and its mapping staged, or the apply consumed the
    /// staging and the `attachments` row exists) is left as it is. A file with the name of the
    /// blob that nothing references is an orphan of an earlier attempt and is replaced.
    ///
    /// # Errors
    /// [`MaterializeError`]; the vault is unchanged and a blob this call wrote is removed.
    pub fn materialize_attachment(
        &self,
        key_epoch: u64,
        ciphertext_sha256: &[u8; 32],
        opened: &OpenedArtifact,
    ) -> Result<(), MaterializeError> {
        let plaintext = opened.plaintext();
        let artifact_id = *plaintext.artifact_id.as_bytes();
        let mime_type = mime_type_of(plaintext.logical_metadata);
        let dek = self.dek()?;
        let local = flow::materialize_attachment(dek, opened)?;
        let storage_id = local.storage_id;
        let attachment_id = local.attachment_id;
        let dir = attachments::blobs_dir(&self.path);
        let mut guard = self.lock();
        if guard.state != WorkerState::Healthy {
            return Err(DbError::WorkerUnavailable(guard.state).into());
        }
        let mapping = Mapping {
            key_epoch,
            artifact_id,
            ciphertext_sha256: *ciphertext_sha256,
            kind: ArtifactKind::Attachment,
            logical_id: attachment_id,
        };

        // This artifact is already here: its blob is on disk and its mapping is staged, or the apply
        // consumed the staging and the attachments row exists. Only the verified hash is refreshed.
        let existing = crate::sync_artifacts::lookup(&guard.conn, key_epoch, &artifact_id)?;
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

        // Nothing references a blob under this name (an attachments row and another artifact's
        // staging were refused above, and this artifact's own was handled), so a file here is an
        // orphan of an attempt that died before its mapping was recorded: it is replaced.
        attachments::write_blob(&dir, &storage_id, &local.blob.ciphertext)?;
        let staged = StagedAttachment {
            storage_id: storage_id.as_str().to_owned(),
            wrapped_content_key: local.wrapped_key.ciphertext.clone(),
            content_key_nonce: local.wrapped_key.nonce.to_vec(),
            content_nonce: local.blob.nonce.to_vec(),
            content_alg: attachments::CONTENT_ALG.to_owned(),
            plaintext_size: plaintext.blob.len() as u64,
            mime_type,
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
    ///
    /// The rows are judged against `live`, the closure at the preceding log position, **never against
    /// which rows are present** (C.9): the batch rows a replica holds are a cache of artifacts that
    /// were once live, which a tailing replica keeps and a bootstrapped one never had.
    /// * A batch **in** `live` is never touched: equal rows are left, absent rows are written, and a
    ///   row that differs is [`MaterializeError::RowsDiffer`] (an invariant: stage 9(b) already
    ///   checked the delta against the live closure).
    /// * A batch **not** in `live` is replaced, **by id and only where it differs**: a batch whose rows
    ///   already equal the artifact's is not written at all, rows that stay are updated in place
    ///   (their ids, which staging rows reference, never leave), and only a row the new batch no longer
    ///   has is removed. A record id that a different not-live batch holds moves to this one. Both kinds
    ///   of replica end with the same class-1 state.
    ///
    /// When the replacement changes, adds or drops any row of a batch for which this device holds
    /// **staging** (an import still being reviewed: `staged_transactions`, `staged_balances`,
    /// `staged_accounts`, `dedupe_decisions`), all of that batch's staging is removed in the same
    /// transaction, because a half-import must not be committed, and a durable notice is written so
    /// the user is told to run the import again. The returned [`DiscardedImport`]s say which, by
    /// counts. An identical batch discards nothing, and the staging of any other batch is untouched.
    ///
    /// A record that a replicated row (a provenance link) names is part of the live closure by
    /// definition; changing, dropping or moving it would orphan that row, so it is
    /// [`MaterializeError::LiveReference`] and nothing is written.
    ///
    /// # Errors
    /// [`MaterializeError`]; on any error nothing is written.
    pub fn materialize_batch(
        &self,
        key_epoch: u64,
        parts: &[BatchPartRef],
        batch: &SourceBatch,
        live: &LiveClosure,
    ) -> Result<Vec<DiscardedImport>, MaterializeError> {
        let mut guard = self.lock();
        if guard.state != WorkerState::Healthy {
            return Err(DbError::WorkerUnavailable(guard.state).into());
        }
        let tx = guard.conn.transaction()?;
        let b = &batch.batch;
        let mut discarded = Vec::new();
        if live.get(&LogicalRef::Batch(b.id)).is_none() {
            discarded = replace_not_live(&tx, batch, live)?;
            // The old parts' addresses no longer name this batch's artifacts; the ones that stay keep
            // their receipts.
            let keep: Vec<[u8; 32]> = parts.iter().map(|p| p.artifact_id).collect();
            let epoch = i64::try_from(key_epoch).map_err(|_| MappingError::Corrupt)?;
            let stale: Vec<Vec<u8>> = {
                let mut stmt = tx.prepare(
                    "SELECT sync_artifact_id FROM sync_artifacts
                      WHERE kind = 2 AND logical_id = ?1 AND key_epoch = ?2",
                )?;
                let rows = stmt.query_map(params![b.id.as_slice(), epoch], |r| r.get(0))?;
                rows.collect::<Result<_, _>>()?
            };
            for id in stale
                .into_iter()
                .filter(|id| !keep.iter().any(|k| k.as_slice() == id))
            {
                tx.execute(
                    "DELETE FROM sync_artifacts WHERE key_epoch = ?1 AND sync_artifact_id = ?2",
                    params![epoch, id],
                )?;
            }
        } else {
            write_rows(&tx, batch)?;
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
        let now = chrono::Utc::now().to_rfc3339();
        for import in &discarded {
            tx.execute(
                "INSERT INTO sync_notices (kind, batch_id, source_name, discarded_count, created_at)
                 VALUES ('import_discarded', ?1, ?2, ?3, ?4)",
                params![
                    import.batch_id.as_slice(),
                    import.source_name,
                    i64::try_from(import.rows()).unwrap_or(i64::MAX),
                    now
                ],
            )?;
        }
        tx.commit()?;
        // One line, after the commit, with counts only: the file name, merchants, amounts and ids
        // stay out of logs (docs/security/logging-policy.md).
        for import in &discarded {
            tracing::info!(
                staged_transactions = import.staged_transactions,
                staged_balances = import.staged_balances,
                staged_accounts = import.staged_accounts,
                dedupe_decisions = import.dedupe_decisions,
                "sync: an import in progress was discarded because a received batch replaced its rows"
            );
        }
        Ok(discarded)
    }

    /// The notices Sync left for the user that they have not dismissed, oldest first.
    ///
    /// # Errors
    /// [`DbError`].
    pub fn sync_notices(&self) -> Result<Vec<SyncNotice>, DbError> {
        let conn = self.read_connection()?;
        let mut stmt = conn.prepare(
            "SELECT id, batch_id, source_name, discarded_count, created_at
               FROM sync_notices WHERE dismissed_at IS NULL ORDER BY id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(SyncNotice {
                id: r.get(0)?,
                batch_id: r.get::<_, Vec<u8>>(1)?.try_into().unwrap_or([0; 16]),
                source_name: r.get(2)?,
                discarded_count: r.get(3)?,
                created_at: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Mark a notice as seen. Returns whether a pending notice with that id existed.
    ///
    /// # Errors
    /// [`DbError`].
    pub fn dismiss_sync_notice(&self, id: i64, now: &str) -> Result<bool, DbError> {
        let guard = self.lock();
        let changed = guard.conn.execute(
            "UPDATE sync_notices SET dismissed_at = ?2 WHERE id = ?1 AND dismissed_at IS NULL",
            params![id, now],
        )?;
        Ok(changed == 1)
    }
}

/// An import in progress on this device that a received batch replaced, by counts of what was
/// removed. The file name is the user's own text: it is shown in the notice and never logged.
#[derive(Clone, PartialEq, Eq)]
pub struct DiscardedImport {
    /// The batch whose staging was removed.
    pub batch_id: [u8; 16],
    /// The file name the user chose for the import, if the batch row had one.
    pub source_name: Option<String>,
    /// `staged_transactions` removed.
    pub staged_transactions: usize,
    /// `staged_balances` removed.
    pub staged_balances: usize,
    /// `staged_accounts` removed.
    pub staged_accounts: usize,
    /// `dedupe_decisions` removed.
    pub dedupe_decisions: usize,
}

impl DiscardedImport {
    /// How many staging rows were removed in all.
    #[must_use]
    pub const fn rows(&self) -> usize {
        self.staged_transactions
            + self.staged_balances
            + self.staged_accounts
            + self.dedupe_decisions
    }
}

impl core::fmt::Debug for DiscardedImport {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DiscardedImport")
            .field("rows", &self.rows())
            .finish_non_exhaustive()
    }
}

/// A notice for the user, as the app shows it.
#[derive(Clone, PartialEq, Eq)]
pub struct SyncNotice {
    /// The notice's id, for dismissing it.
    pub id: i64,
    /// The batch the notice is about.
    pub batch_id: [u8; 16],
    /// The file name of the discarded import, if it had one.
    pub source_name: Option<String>,
    /// How many staging rows were discarded.
    pub discarded_count: i64,
    /// When it was written.
    pub created_at: String,
}

impl core::fmt::Debug for SyncNotice {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SyncNotice")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

fn load_batch(
    tx: &rusqlite::Transaction<'_>,
    id: &[u8; 16],
) -> Result<Option<BatchRow>, MaterializeError> {
    Ok(tx
        .query_row(
            "SELECT source_type, source_name, file_fingerprint, parser_version, summary_json,
                    imported_at, created_at, connector_connection_id
               FROM source_batches WHERE id = ?1",
            params![id.as_slice()],
            |r| {
                Ok(BatchRow {
                    id: *id,
                    source_type: r.get(0)?,
                    source_name: r.get(1)?,
                    file_fingerprint: r.get(2)?,
                    parser_version: r.get(3)?,
                    summary_json: r.get(4)?,
                    imported_at: r.get(5)?,
                    created_at: r.get(6)?,
                    connector_connection_id: r
                        .get::<_, Option<Vec<u8>>>(7)?
                        .map(|v| <[u8; 16]>::try_from(v).unwrap_or([0; 16])),
                })
            },
        )
        .optional()?)
}

fn load_runs(
    tx: &rusqlite::Transaction<'_>,
    batch: &[u8; 16],
) -> Result<Vec<RunRow>, MaterializeError> {
    let mut stmt = tx.prepare(
        "SELECT id, parser_name, parser_version, bytes_in, records_out, status, limit_hit,
                started_at, finished_at
           FROM parser_runs WHERE source_batch_id = ?1 ORDER BY id",
    )?;
    let rows = stmt.query_map(params![batch.as_slice()], |r| {
        Ok(RunRow {
            id: r.get::<_, Vec<u8>>(0)?.try_into().unwrap_or([0; 16]),
            source_batch_id: *batch,
            parser_name: r.get(1)?,
            parser_version: r.get(2)?,
            bytes_in: r.get(3)?,
            records_out: r.get(4)?,
            status: r.get(5)?,
            limit_hit: r.get(6)?,
            started_at: r.get(7)?,
            finished_at: r.get(8)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn load_records(
    tx: &rusqlite::Transaction<'_>,
    batch: &[u8; 16],
) -> Result<Vec<RecordRow>, MaterializeError> {
    let mut stmt = tx.prepare(
        "SELECT id, external_id, source_hash, normalized_json, parse_confidence_bps, created_at
           FROM source_records WHERE source_batch_id = ?1 ORDER BY id",
    )?;
    let rows = stmt.query_map(params![batch.as_slice()], |r| {
        Ok(RecordRow {
            id: r.get::<_, Vec<u8>>(0)?.try_into().unwrap_or([0; 16]),
            source_batch_id: *batch,
            external_id: r.get(1)?,
            source_hash: r.get(2)?,
            normalized_json: r.get(3)?,
            parse_confidence_bps: r.get(4)?,
            created_at: r.get(5)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Remove all of a batch's staging (an import still being reviewed on this device), children first
/// (`dedupe_decisions` references `staged_transactions`), and say what went. A committed dedupe
/// decision (no staged transaction) is replicated data and stays. `None` if the device holds no
/// staging for the batch, so a replica that only holds committed rows gets no notice.
fn discard_staging(
    tx: &rusqlite::Transaction<'_>,
    batch: &[u8; 16],
    source_name: Option<String>,
) -> Result<Option<DiscardedImport>, MaterializeError> {
    let by_record = "SELECT id FROM source_records WHERE source_batch_id = ?1";
    // Only the decisions about this batch's *staged transactions*. A decision with no staged id is a
    // committed one that `CommitImportedTransaction`'s apply writes on every replica: replicated
    // data, which this device-local discard must never touch.
    let dedupe_decisions = tx.execute(
        &format!(
            "DELETE FROM dedupe_decisions WHERE staged_transaction_id IN
                 (SELECT id FROM staged_transactions WHERE source_record_id IN ({by_record}))"
        ),
        params![batch.as_slice()],
    )?;
    let staged_transactions = tx.execute(
        &format!("DELETE FROM staged_transactions WHERE source_record_id IN ({by_record})"),
        params![batch.as_slice()],
    )?;
    let staged_balances = tx.execute(
        &format!("DELETE FROM staged_balances WHERE source_record_id IN ({by_record})"),
        params![batch.as_slice()],
    )?;
    let staged_accounts = tx.execute(
        "DELETE FROM staged_accounts WHERE source_batch_id = ?1",
        params![batch.as_slice()],
    )?;
    let import = DiscardedImport {
        batch_id: *batch,
        source_name,
        staged_transactions,
        staged_balances,
        staged_accounts,
        dedupe_decisions,
    };
    Ok((import.rows() > 0).then_some(import))
}

/// Replace a batch that is not live, where it differs (see [`DbWorker::materialize_batch`]).
fn replace_not_live(
    tx: &rusqlite::Transaction<'_>,
    batch: &SourceBatch,
    live: &LiveClosure,
) -> Result<Vec<DiscardedImport>, MaterializeError> {
    let b = &batch.batch;
    let old_batch = load_batch(tx, &b.id)?;
    let mut old_runs = load_runs(tx, &b.id)?;
    let mut old_records = load_records(tx, &b.id)?;
    let (mut new_runs, mut new_records) = (batch.runs.clone(), batch.records.clone());
    old_runs.sort_by_key(|r| r.id);
    old_records.sort_by_key(|r| r.id);
    new_runs.sort_by_key(|r| r.id);
    new_records.sort_by_key(|r| r.id);

    // A record id that a *different* batch holds: that batch loses it. It must not be live.
    let mut holders: Vec<([u8; 16], [u8; 16])> = Vec::new();
    for record in &batch.records {
        let holder: Option<Vec<u8>> = tx
            .query_row(
                "SELECT source_batch_id FROM source_records WHERE id = ?1",
                params![record.id.as_slice()],
                |r| r.get(0),
            )
            .optional()?;
        let Some(holder) = holder else { continue };
        let holder = <[u8; 16]>::try_from(holder).map_err(|_| MappingError::Corrupt)?;
        if holder == b.id {
            continue;
        }
        // A different live batch holding the id is a T2 violation that stage 9(b) rejects first.
        if live.get(&LogicalRef::Batch(holder)).is_some() {
            return Err(MaterializeError::RowsDiffer);
        }
        holders.push((holder, record.id));
    }

    let identical = old_batch.as_ref() == Some(b)
        && old_runs == new_runs
        && old_records == new_records
        && holders.is_empty();
    if identical {
        return Ok(Vec::new());
    }

    // Records this replacement changes, drops or moves: none may be named by a replicated row.
    let mut touched: Vec<[u8; 16]> = old_records
        .iter()
        .filter(|old| {
            new_records
                .binary_search_by_key(&old.id, |n| n.id)
                .map_or(true, |i| &new_records[i] != *old)
        })
        .map(|old| old.id)
        .collect();
    touched.extend(holders.iter().map(|(_, record)| *record));
    for id in &touched {
        let named: Option<i64> = tx
            .query_row(
                "SELECT 1 FROM source_provenance_links WHERE source_record_id = ?1 LIMIT 1",
                params![id.as_slice()],
                |r| r.get(0),
            )
            .optional()?;
        if named.is_some() {
            return Err(MaterializeError::LiveReference);
        }
    }

    // Staging of every batch whose rows change, before any row moves.
    let mut discarded = Vec::new();
    let mut losing: Vec<[u8; 16]> = holders.iter().map(|(holder, _)| *holder).collect();
    losing.sort_unstable();
    losing.dedup();
    if let Some(old) = &old_batch {
        if let Some(import) = discard_staging(tx, &b.id, old.source_name.clone())? {
            discarded.push(import);
        }
    }
    for holder in losing {
        let name = load_batch(tx, &holder)?.and_then(|row| row.source_name);
        if let Some(import) = discard_staging(tx, &holder, name)? {
            discarded.push(import);
        }
    }

    // The batch row, then its runs and records, in place by id. Staging rows referenced these ids,
    // and none of them is deleted unless the new batch no longer has it.
    tx.execute(
        "INSERT INTO source_batches
             (id, source_type, source_name, file_fingerprint, parser_version, status,
              staged_count, committed_count, skipped_count, summary_json, imported_at,
              created_at, updated_at, connector_connection_id)
         VALUES (?1, ?2, ?3, ?4, ?5, 'committed', 0, 0, 0, ?6, ?7, ?8, ?8, ?9)
         ON CONFLICT (id) DO UPDATE SET
             source_type = excluded.source_type, source_name = excluded.source_name,
             file_fingerprint = excluded.file_fingerprint, parser_version = excluded.parser_version,
             status = 'committed', staged_count = 0, committed_count = 0, skipped_count = 0,
             summary_json = excluded.summary_json, imported_at = excluded.imported_at,
             created_at = excluded.created_at, updated_at = excluded.created_at,
             connector_connection_id = excluded.connector_connection_id",
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
    for old in old_runs
        .iter()
        .filter(|old| new_runs.binary_search_by_key(&old.id, |n| n.id).is_err())
    {
        tx.execute(
            "DELETE FROM parser_runs WHERE id = ?1",
            params![old.id.as_slice()],
        )?;
    }
    for run in &new_runs {
        tx.execute(
            "INSERT INTO parser_runs
                 (id, source_batch_id, parser_name, parser_version, bytes_in, records_out,
                  status, limit_hit, started_at, finished_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT (id) DO UPDATE SET
                 source_batch_id = excluded.source_batch_id, parser_name = excluded.parser_name,
                 parser_version = excluded.parser_version, bytes_in = excluded.bytes_in,
                 records_out = excluded.records_out, status = excluded.status,
                 limit_hit = excluded.limit_hit, started_at = excluded.started_at,
                 finished_at = excluded.finished_at",
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
    }
    for old in old_records
        .iter()
        .filter(|old| new_records.binary_search_by_key(&old.id, |n| n.id).is_err())
    {
        tx.execute(
            "DELETE FROM source_records WHERE id = ?1",
            params![old.id.as_slice()],
        )?;
    }
    for rec in &new_records {
        tx.execute(
            "INSERT INTO source_records
                 (id, source_batch_id, external_id, source_hash, normalized_json,
                  parse_confidence_bps, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (id) DO UPDATE SET
                 source_batch_id = excluded.source_batch_id, external_id = excluded.external_id,
                 source_hash = excluded.source_hash, normalized_json = excluded.normalized_json,
                 parse_confidence_bps = excluded.parse_confidence_bps,
                 created_at = excluded.created_at",
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
    }
    Ok(discarded)
}

/// Write the rows of `batch`; a row already present must equal the one being written.
fn write_rows(tx: &rusqlite::Transaction<'_>, batch: &SourceBatch) -> Result<(), MaterializeError> {
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
    Ok(())
}
