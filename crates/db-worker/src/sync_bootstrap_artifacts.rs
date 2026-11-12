//! The closure of a snapshot, fetched and materialized while a replica is built
//! (`docs/architecture/vault-format.md` C.8 "Replica bootstrap", C.9; `personal-cfo-1cltt.7`,
//! slice D2).
//!
//! A snapshot's closure is every class-1b artifact its rows depend on. A manifest does not bind an id
//! to a logical row, so a replica learns what each id is only by opening it: it fetches **every**
//! artifact eagerly, before the vault is activated, through the same transport and checks a delta
//! uses (`personal-cfo-0g528.2`), in **one pass** in id order:
//!
//! * fetch, check the bytes hash to the value the snapshot's manifest binds, open (outer AEAD, then
//!   the descriptor), and keep only a small [`DeltaArtifact`] (id, binding, record ids, hash): a
//!   closure can hold 2^20 artifacts and nothing but this metadata outlives its artifact;
//! * an attachment is materialized at once (its bytes re-encrypted under this device's DEK, the
//!   crypto fields staged);
//! * a batch is materialized when its last part arrives. Ids are HMAC outputs, so the parts of
//!   different batches interleave in id order: a verified part that waits for its siblings is
//!   **spooled to disk as ciphertext** (never plaintext), and the spool is removed whatever the
//!   outcome;
//! * the stage-9(b) closure check then runs over the collected metadata (every batch complete and
//!   canonical, one binding per logical object, one batch per record), and the attachment rows are
//!   created from the snapshot's row plus the staged crypto, and the live closure is seeded.
//!
//! **Three kinds of failure**, and which one a caller sees decides what it does next:
//!
//! * [`InvalidSnapshot`], DETERMINISTIC: the snapshot is bad, at every replica (the producer's fault:
//!   an object that hashes to the bound value but does not authenticate or does not validate, a closure
//!   that is wrong, a row and an artifact that disagree, a row naming a row that is not there). The
//!   root is never picked again.
//! * [`TransientFault`]: the service's fault (`Missing`, unavailable, bytes that do not hash to the
//!   bound value). Retry the same root.
//! * [`LocalFault`]: this device's fault (a disk, a key, a write): `ArtifactMaterializationFailed`.
//!   It **never** marks the root invalid.
//!
//! A materialize error caused by an artifact's *content* is deterministic exactly as the closure check
//! would have ruled, so the verdict does not depend on the order the artifacts arrived in.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::PathBuf;

use sync_core::artifact::{ArtifactId, LogicalBinding};
use sync_core::artifact_closure::{
    check_batch_canonical, check_closure, BoundPlaintext, DeltaArtifact, LiveClosure, LogicalRef,
    Mismatch, PayloadRef,
};
use sync_core::artifact_transport::ArtifactTransport;
use sync_core::snapshot::ArtifactEntry;
use sync_core::source_batch::join;
use thiserror::Error;
use vault_crypto::sync_artifact_flow::{fetch_container, open_fetched, FetchError};
use vault_crypto::sync_envelope::EpochKey;

use crate::snapshot_install::SnapshotAttachment;
use crate::sync_artifacts::{
    create_attachment_from_snapshot_row, CreateAttachmentError, MappingError,
};
use crate::sync_live_closure::{seed, LiveClosureError, LiveEntry};
use crate::sync_materialize::{BatchPartRef, MaterializeError};
use crate::{DbError, DbWorker};

/// The snapshot is invalid, and so at every replica: its root is never picked again.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum InvalidSnapshot {
    /// An artifact of the closure is not what the snapshot says it is, or the closure is wrong
    /// (stage 9(b)): an object that hashes to the bound value but does not authenticate or validate,
    /// a part missing or duplicated, a batch that is not its canonical split, one logical object
    /// bound twice.
    #[error("the snapshot's closure is invalid: {0}")]
    Closure(Mismatch),
    /// An attachment row of the snapshot has no artifact in its closure.
    #[error("an attachment row of the snapshot has no artifact in its closure")]
    AttachmentWithoutArtifact,
    /// An attachment artifact of the closure has no row in the snapshot.
    #[error("an attachment artifact of the closure has no row in the snapshot")]
    ArtifactWithoutAttachment,
    /// An attachment row's plaintext size is not the size of its artifact's bytes.
    #[error("an attachment row's plaintext size is not its artifact's")]
    AttachmentSize,
    /// An artifact's content cannot be materialized (two attachments with identical bytes, a record
    /// held twice): the same defects the closure check rules on.
    #[error("an artifact's content cannot be materialized: {0}")]
    Content(ContentFault),
    /// A row of the snapshot names a row that is not there once the closure is materialized (a
    /// provenance link naming a source record that no batch holds).
    #[error("a row of the snapshot names a row that is not there")]
    DanglingReference,
    /// The snapshot's closure manifest has an entry that is not 32 bytes of hex.
    #[error("the snapshot's closure manifest has a malformed entry")]
    ManifestEntry,
}

/// Why an artifact's content cannot be materialized, by what is wrong with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ContentFault {
    /// The snapshot has an attachment id the vault already holds.
    #[error("the vault already holds an attachment with this id")]
    AttachmentExists,
    /// The artifact is not a valid attachment.
    #[error("the artifact is not a valid attachment")]
    NotAnAttachment,
    /// Rows for the batch are already in the vault and differ.
    #[error("rows already in the vault differ from the batch")]
    RowsDiffer,
    /// A replicated row names a record of the batch being materialized.
    #[error("a live row names a record of the batch being replaced")]
    LiveReference,
}

/// The artifact service did not deliver (`ArtifactMissing`, unavailable, bytes that are not the
/// bound ones): its fault, not the snapshot's. Retry the same root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum TransientFault {
    /// The service holds no object for the id (`ArtifactMissing`).
    #[error("the service holds no object for artifact {0:?}")]
    Missing(ArtifactId),
    /// The service could not be reached.
    #[error("the service could not be reached for artifact {0:?}")]
    Unavailable(ArtifactId),
    /// The fetched bytes are not the ones the snapshot bound to the id.
    #[error("the bytes fetched for artifact {0:?} are not the bound ones")]
    HashMismatch(ArtifactId),
}

/// This device could not materialize the closure (`ArtifactMaterializationFailed`): its disk, its key,
/// a write. It never marks the root invalid.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum LocalFault {
    /// An artifact could not be materialized here.
    #[error("{0}")]
    Materialize(MaterializeError),
    /// The spool could not be written or read.
    #[error("the spool could not be used: {0}")]
    Spool(io::Error),
    /// The database failed.
    #[error("{0}")]
    Db(DbError),
    /// An artifact's mapping could not be recorded.
    #[error("{0}")]
    Mapping(MappingError),
    /// The live closure could not be seeded.
    #[error("{0}")]
    LiveClosure(LiveClosureError),
}

/// Why the closure was not materialized.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ClosureError {
    /// Deterministic: the snapshot is bad.
    #[error(transparent)]
    Invalid(#[from] InvalidSnapshot),
    /// The service's fault: retry.
    #[error(transparent)]
    Transient(#[from] TransientFault),
    /// This device's fault.
    #[error(transparent)]
    Local(#[from] LocalFault),
}

impl From<DbError> for ClosureError {
    fn from(error: DbError) -> Self {
        Self::Local(LocalFault::Db(error))
    }
}

impl From<LiveClosureError> for ClosureError {
    fn from(error: LiveClosureError) -> Self {
        Self::Local(LocalFault::LiveClosure(error))
    }
}

/// A directory where verified ciphertext waits for its siblings. Created on first use, and removed
/// whole when the bootstrap ends, however it ends.
pub(crate) struct Spool {
    dir: PathBuf,
}

impl Spool {
    pub(crate) fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn name(&self, id: &ArtifactId) -> PathBuf {
        let hex: String = id.as_bytes().iter().map(|b| format!("{b:02x}")).collect();
        self.dir.join(hex)
    }

    fn put(&self, id: &ArtifactId, container: &[u8]) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        fs::write(self.name(id), container)
    }

    fn take(&self, id: &ArtifactId) -> io::Result<Vec<u8>> {
        let path = self.name(id);
        let bytes = fs::read(&path)?;
        fs::remove_file(&path)?;
        Ok(bytes)
    }

    /// Remove the spool and everything in it. A spool that was never made is not an error.
    pub(crate) fn remove(&self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// What the pass learned: the live closure's rows, and which artifact carries each attachment.
pub(crate) struct Materialized {
    pub(crate) live: Vec<LiveEntry>,
    pub(crate) attachments: BTreeMap<[u8; 16], [u8; 32]>,
}

struct BatchState {
    count: u32,
    /// `(index, artifact id, part count it claims, bound hash)` of the parts that arrived.
    parts: Vec<(u32, ArtifactId, u32, [u8; 32])>,
    done: bool,
}

fn hex32(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0_u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(hex.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

/// A fetch or open failure, as the closure check would class it: a fact about the artifact is the
/// snapshot's fault (deterministic); anything else is the service's.
fn fetch_error(error: FetchError, id: ArtifactId) -> ClosureError {
    if let Some(mismatch) = error.mismatch(id) {
        return InvalidSnapshot::Closure(mismatch).into();
    }
    match error {
        FetchError::Missing => TransientFault::Missing(id).into(),
        FetchError::HashMismatch => TransientFault::HashMismatch(id).into(),
        _ => TransientFault::Unavailable(id).into(),
    }
}

fn materialize_error(error: MaterializeError) -> ClosureError {
    match error {
        MaterializeError::NotAnAttachment => {
            InvalidSnapshot::Content(ContentFault::NotAnAttachment).into()
        }
        MaterializeError::RowsDiffer => InvalidSnapshot::Content(ContentFault::RowsDiffer).into(),
        MaterializeError::LiveReference => {
            InvalidSnapshot::Content(ContentFault::LiveReference).into()
        }
        other => LocalFault::Materialize(other).into(),
    }
}

/// Fetch, verify and materialize every artifact of `closure`, then run the closure check over what
/// was collected.
///
/// # Errors
/// A [`ClosureError`]. The vault holds whatever was materialized; the caller discards the build.
pub(crate) fn materialize_closure(
    worker: &DbWorker,
    transport: &dyn ArtifactTransport,
    key: &EpochKey,
    closure: &[ArtifactEntry],
    spool: &Spool,
) -> Result<Materialized, ClosureError> {
    let epoch = key.key_epoch();
    let mut deltas: Vec<DeltaArtifact> = Vec::with_capacity(closure.len());
    let mut live = Vec::with_capacity(closure.len());
    let mut attachments: BTreeMap<[u8; 16], [u8; 32]> = BTreeMap::new();
    let mut batches: BTreeMap<[u8; 16], BatchState> = BTreeMap::new();

    for entry in closure {
        let (Some(raw_id), Some(hash)) = (hex32(&entry.id), hex32(&entry.ciphertext_sha256)) else {
            return Err(InvalidSnapshot::ManifestEntry.into());
        };
        let id = ArtifactId::from_derived(raw_id);
        let container =
            fetch_container(transport, key, &id, &hash).map_err(|e| fetch_error(e, id))?;
        let opened = open_fetched(key, &id, &container).map_err(|e| fetch_error(e, id))?;
        let delta = DeltaArtifact::from_plaintext(&BoundPlaintext {
            plaintext: opened.plaintext(),
            ciphertext_sha256: hash,
        })
        .map_err(InvalidSnapshot::Closure)?;
        match delta.binding {
            LogicalBinding::Attachment { attachment_id } => {
                worker
                    .materialize_attachment(epoch, &hash, &opened)
                    .map_err(materialize_error)?;
                live.push(LiveEntry {
                    id,
                    ciphertext_sha256: hash,
                    logical: LogicalRef::Attachment(attachment_id),
                });
                attachments.insert(attachment_id, raw_id);
            }
            LogicalBinding::BatchPart {
                batch_id,
                part_index,
                part_count,
            } => {
                live.push(LiveEntry {
                    id,
                    ciphertext_sha256: hash,
                    logical: LogicalRef::Batch(batch_id),
                });
                let state = batches.entry(batch_id).or_insert_with(|| BatchState {
                    count: part_count,
                    parts: Vec::new(),
                    done: false,
                });
                state.parts.push((part_index, id, part_count, hash));
                if state.parts.len() == state.count as usize && !state.done {
                    // The last part: the others are read back from the spool (ciphertext), opened,
                    // and the batch is judged and materialized whole.
                    let mut blobs: Vec<(u32, u32, Vec<u8>)> = Vec::with_capacity(state.parts.len());
                    let mut refs: Vec<BatchPartRef> = Vec::with_capacity(state.parts.len());
                    for (index, part_id, claimed, part_hash) in &state.parts {
                        let blob = if *part_id == id {
                            opened.plaintext().blob.to_vec()
                        } else {
                            let stored = spool.take(part_id).map_err(LocalFault::Spool)?;
                            let reopened = open_fetched(key, part_id, &stored)
                                .map_err(|e| fetch_error(e, *part_id))?;
                            reopened.plaintext().blob.to_vec()
                        };
                        blobs.push((*index, *claimed, blob));
                        refs.push(BatchPartRef {
                            artifact_id: *part_id.as_bytes(),
                            ciphertext_sha256: *part_hash,
                            index: *index,
                            count: *claimed,
                        });
                    }
                    let parts: Vec<(u32, u32, &[u8])> = blobs
                        .iter()
                        .map(|(i, c, b)| (*i, *c, b.as_slice()))
                        .collect();
                    check_batch_canonical(&batch_id, &parts).map_err(InvalidSnapshot::Closure)?;
                    let batch = join(&batch_id, &parts).map_err(|_| {
                        InvalidSnapshot::Closure(Mismatch::BatchNotCanonical(batch_id))
                    })?;
                    worker
                        .materialize_batch(epoch, &refs, &batch, &LiveClosure::new())
                        .map_err(materialize_error)?;
                    state.done = true;
                } else {
                    // Waiting for its siblings: verified ciphertext, never plaintext, goes to disk.
                    spool.put(&id, &container).map_err(LocalFault::Spool)?;
                }
            }
        }
        deltas.push(delta);
    }

    // Every batch is complete, and the closure is one the check accepts: the same stage-9(b) rules
    // an envelope's delta meets, over what the pass collected, against an empty live closure (a
    // snapshot's closure is the live closure).
    if let Some((batch, _)) = batches.iter().find(|(_, state)| !state.done) {
        return Err(InvalidSnapshot::Closure(Mismatch::PartMissing(*batch)).into());
    }
    let mut refs = Vec::new();
    for delta in &deltas {
        match delta.binding {
            LogicalBinding::Attachment { attachment_id } => {
                refs.push(PayloadRef::Attachment(attachment_id));
            }
            LogicalBinding::BatchPart { .. } => {
                refs.extend(delta.records.iter().map(|r| PayloadRef::Record(*r)));
            }
        }
    }
    check_closure(&refs, &deltas, &LiveClosure::new()).map_err(InvalidSnapshot::Closure)?;
    Ok(Materialized { live, attachments })
}

/// Create the attachment rows of the snapshot and seed the live closure, in one transaction: each
/// row from the snapshot's columns and the staged crypto of its artifact (the staging cleared with
/// the row), `ref_count` rebuilt from the links, and the live closure from what was materialized.
///
/// # Errors
/// A [`ClosureError`]: a row with no artifact, or an artifact with no row, or a size that disagrees,
/// is deterministic.
pub(crate) fn finish(
    worker: &DbWorker,
    key_epoch: u64,
    rows: &[SnapshotAttachment],
    materialized: &Materialized,
) -> Result<(), ClosureError> {
    for row in rows {
        if !materialized.attachments.contains_key(&row.id) {
            return Err(InvalidSnapshot::AttachmentWithoutArtifact.into());
        }
    }
    if materialized
        .attachments
        .keys()
        .any(|id| !rows.iter().any(|row| row.id == *id))
    {
        return Err(InvalidSnapshot::ArtifactWithoutAttachment.into());
    }
    worker.write_sync_state(|tx| -> Result<(), ClosureError> {
        for row in rows {
            let artifact = materialized.attachments[&row.id];
            create_attachment_from_snapshot_row(tx, key_epoch, &artifact, row).map_err(
                |error| match error {
                    CreateAttachmentError::SizeDisagrees => {
                        ClosureError::from(InvalidSnapshot::AttachmentSize)
                    }
                    CreateAttachmentError::AlreadyExists => {
                        InvalidSnapshot::Content(ContentFault::AttachmentExists).into()
                    }
                    CreateAttachmentError::ArtifactMissing
                    | CreateAttachmentError::WrongAttachment => {
                        InvalidSnapshot::AttachmentWithoutArtifact.into()
                    }
                    CreateAttachmentError::Mapping(error) => {
                        ClosureError::Local(LocalFault::Mapping(error))
                    }
                    CreateAttachmentError::Store(error) => {
                        ClosureError::Local(LocalFault::Db(error))
                    }
                },
            )?;
        }
        // `ref_count` is rebuilt, never shipped: the count of the links that name the attachment.
        tx.execute(
            "UPDATE attachments
                SET ref_count = (SELECT COUNT(*) FROM attachment_links l
                                  WHERE l.attachment_id = attachments.id)",
            [],
        )
        .map_err(DbError::from)?;
        seed(tx, key_epoch, &materialized.live)?;
        Ok(())
    })
}
