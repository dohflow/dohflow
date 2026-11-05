//! An envelope's artifact closure (`docs/architecture/vault-format.md` C.4 stage 9 and C.9;
//! personal-cfo-0g528.2, part S2c): what a payload references, the **producer's** receipt-gated
//! delta ([`ClosureBuilder`]), and the **receiver's** set-equality check ([`check_closure`]).
//!
//! The delta of an envelope is every class-1b artifact its payload references, whether or not an
//! earlier envelope already published it, and nothing else. A payload names *logical* ids (a source
//! record, an attachment); the sync artifact id appears only in the manifest. The producer maps the
//! logical references to ids through its device-local directory ([`ArtifactDirectory`]) and refuses
//! to build a delta unless every id has a durable receipt for the epoch (receipt before reference).
//! The receiver opens every artifact in the delta, learns the logical id each one binds, and checks
//! that the two sets are equal (stage 9(b)).
//!
//! "Already bound" is judged against the **live closure at the preceding position** ([`LiveClosure`]),
//! never against a replica's mapping: a replica that tailed the log and one bootstrapped later in the
//! same epoch must reach the same verdict.

use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;

use crate::artifact::{ArtifactId, LogicalBinding};
use crate::envelope::ArtifactManifest;
use crate::header::ManifestCommitment;

/// A logical class-1b object an artifact binds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LogicalRef {
    /// A source batch (carried as parts).
    Batch([u8; 16]),
    /// An attachment.
    Attachment([u8; 16]),
}

/// What a payload names. The producer's total function over the command set yields these; a payload
/// never carries a sync artifact id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PayloadRef {
    /// A source record, held by exactly one batch.
    Record([u8; 16]),
    /// An attachment.
    Attachment([u8; 16]),
}

// ---- the receiver: stage 9(b) -------------------------------------------------------------------

/// One artifact of a delta, after it has been fetched and verified: its address, the logical id it
/// binds, and (for a batch part) the ids of the records it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaArtifact {
    /// The artifact's address in the envelope's epoch.
    pub id: ArtifactId,
    /// The logical binding its encrypted metadata names.
    pub binding: LogicalBinding,
    /// For a batch part, the ids of the records in its blob; empty for an attachment.
    pub records: Vec<[u8; 16]>,
}

/// What a snapshot at the preceding log position would carry, in this epoch: each logical object's
/// ids. A function of replicated state, never a replica's local cache.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveClosure(BTreeMap<LogicalRef, BTreeSet<ArtifactId>>);

impl LiveClosure {
    /// An empty closure.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add `id` to `logical`'s set.
    pub fn insert(&mut self, logical: LogicalRef, id: ArtifactId) {
        self.0.entry(logical).or_default().insert(id);
    }

    /// The ids bound to `logical`, if any.
    #[must_use]
    pub fn get(&self, logical: &LogicalRef) -> Option<&BTreeSet<ArtifactId>> {
        self.0.get(logical)
    }
}

/// Why a closure is wrong (stage 9(b), `ArtifactClosureMismatch`). Decided only once every artifact of
/// the delta is held and verified. No variant carries a value from an artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum Mismatch {
    /// The delta holds an artifact no reference of the payload binds (junk cannot be pinned).
    #[error("an artifact in the delta is bound by no reference")]
    UnreferencedArtifact(ArtifactId),
    /// A reference of the payload has no artifact in the delta.
    #[error("a reference of the payload has no artifact in the delta")]
    UnresolvedReference(PayloadRef),
    /// A batch's parts do not cover `0` to `part_count - 1` exactly (a gap).
    #[error("a part index of a batch is missing")]
    PartMissing([u8; 16]),
    /// Two artifacts claim one part index of a batch.
    #[error("a part index of a batch is duplicated")]
    PartDuplicated([u8; 16]),
    /// The parts of a batch disagree on `part_count`.
    #[error("the parts of a batch disagree on part_count")]
    PartsDisagree([u8; 16]),
    /// A source record the payload names is in no referenced batch.
    #[error("a named source record is in no referenced batch")]
    RecordInNoBatch([u8; 16]),
    /// A source record the payload names is in more than one referenced batch.
    #[error("a named source record is in several referenced batches")]
    RecordInSeveralBatches([u8; 16]),
    /// One attachment is bound to several artifacts in the delta.
    #[error("an attachment is bound to several artifacts")]
    AttachmentRebound([u8; 16]),
    /// The delta binds a logical object of the live closure to a different id (set).
    #[error("a logical object is bound to a different id than in the live closure")]
    Rebinding(LogicalRef),
}

/// Stage 9(b): with every artifact of the delta held and verified, the logical ids they bind must
/// equal the logical ids the payload references. A batch's parts are exactly `0` to `part_count - 1`;
/// every named record is in exactly one referenced batch; every referenced batch holds a named
/// record and every delta attachment is named; and no logical object of the live closure is bound to
/// a different id (set).
///
/// # Errors
/// The first [`Mismatch`], in a fixed order, so every replica reports the same one.
pub fn check_closure(
    refs: &[PayloadRef],
    delta: &[DeltaArtifact],
    live: &LiveClosure,
) -> Result<(), Mismatch> {
    // Group the delta by logical object.
    let mut batches: BTreeMap<[u8; 16], Vec<(&DeltaArtifact, u32, u32)>> = BTreeMap::new();
    let mut attachments: BTreeMap<[u8; 16], Vec<&DeltaArtifact>> = BTreeMap::new();
    for artifact in delta {
        match artifact.binding {
            LogicalBinding::BatchPart {
                batch_id,
                part_index,
                part_count,
            } => batches
                .entry(batch_id)
                .or_default()
                .push((artifact, part_index, part_count)),
            LogicalBinding::Attachment { attachment_id } => {
                attachments.entry(attachment_id).or_default().push(artifact);
            }
        }
    }

    // A batch's parts: one part_count, every index once.
    for (batch_id, parts) in &batches {
        let count = parts[0].2;
        if parts.iter().any(|p| p.2 != count) {
            return Err(Mismatch::PartsDisagree(*batch_id));
        }
        let mut seen = BTreeSet::new();
        for (_, index, _) in parts {
            if !seen.insert(*index) {
                return Err(Mismatch::PartDuplicated(*batch_id));
            }
        }
        if usize::try_from(count).ok() != Some(seen.len()) {
            return Err(Mismatch::PartMissing(*batch_id));
        }
    }
    for (attachment_id, artifacts) in &attachments {
        if artifacts.len() > 1 {
            return Err(Mismatch::AttachmentRebound(*attachment_id));
        }
    }

    // Every named record is in exactly one referenced batch; every named attachment has an artifact.
    let mut named_in: BTreeSet<[u8; 16]> = BTreeSet::new();
    for reference in refs {
        match *reference {
            PayloadRef::Record(record) => {
                let holders: Vec<[u8; 16]> = batches
                    .iter()
                    .filter(|(_, parts)| parts.iter().any(|(a, _, _)| a.records.contains(&record)))
                    .map(|(batch, _)| *batch)
                    .collect();
                match holders.as_slice() {
                    [] => return Err(Mismatch::RecordInNoBatch(record)),
                    [one] => {
                        named_in.insert(*one);
                    }
                    _ => return Err(Mismatch::RecordInSeveralBatches(record)),
                }
            }
            PayloadRef::Attachment(attachment) => {
                if !attachments.contains_key(&attachment) {
                    return Err(Mismatch::UnresolvedReference(*reference));
                }
            }
        }
    }

    // Nothing else is in the delta.
    for (batch_id, parts) in &batches {
        if !named_in.contains(batch_id) {
            return Err(Mismatch::UnreferencedArtifact(parts[0].0.id));
        }
    }
    for (attachment_id, artifacts) in &attachments {
        if !refs.contains(&PayloadRef::Attachment(*attachment_id)) {
            return Err(Mismatch::UnreferencedArtifact(artifacts[0].id));
        }
    }

    // One binding per logical object: judged against the live closure, never the local mapping.
    for (batch_id, parts) in &batches {
        let logical = LogicalRef::Batch(*batch_id);
        let ids: BTreeSet<ArtifactId> = parts.iter().map(|(a, _, _)| a.id).collect();
        if live.get(&logical).is_some_and(|bound| bound != &ids) {
            return Err(Mismatch::Rebinding(logical));
        }
    }
    for (attachment_id, artifacts) in &attachments {
        let logical = LogicalRef::Attachment(*attachment_id);
        let ids: BTreeSet<ArtifactId> = artifacts.iter().map(|a| a.id).collect();
        if live.get(&logical).is_some_and(|bound| bound != &ids) {
            return Err(Mismatch::Rebinding(logical));
        }
    }
    Ok(())
}

// ---- the producer: receipt before reference ------------------------------------------------------------

/// Whether the service has acknowledged an artifact: the client holds a durable receipt for its
/// address in the epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptState {
    /// No receipt yet: the row is written, the object may not have been put.
    None,
    /// A receipt that matched what the client computed has been recorded.
    Durable,
}

/// The producer's device-local view of its artifacts for one epoch: the mapping from a logical
/// object to the ids that carry it in that epoch, and each id's receipt.
pub trait ArtifactDirectory {
    /// The batch that holds source record `record`, if the vault knows one.
    fn batch_of(&self, record: &[u8; 16]) -> Option<[u8; 16]>;

    /// The ids that carry `logical` in `key_epoch`, each with its receipt state; `None` if the
    /// object has no artifact in that epoch.
    fn artifacts_of(
        &self,
        logical: &LogicalRef,
        key_epoch: u64,
    ) -> Option<Vec<(ArtifactId, ReceiptState)>>;
}

/// Why a delta was not built. Nothing was emitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum BuildError {
    /// A named reference has no artifact in this epoch (never published, or not re-addressed).
    #[error("a referenced object has no artifact in this epoch")]
    NoArtifact(PayloadRef),
    /// An artifact has no durable receipt in this epoch: it must be put and acknowledged before a
    /// reference is emitted.
    #[error("a referenced artifact has no durable receipt in this epoch")]
    ReceiptMissing(PayloadRef, ArtifactId),
    /// More artifacts than an envelope's delta may hold.
    #[error("the delta would hold more artifacts than an envelope allows")]
    TooManyArtifacts,
}

/// An envelope's delta, built from the payload's references with every receipt checked. The
/// envelope's header takes its `artifact_manifest_commitment` from here and the plaintext its
/// `delta_manifest`, so a reference cannot be emitted without its receipts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltDelta {
    manifest: ArtifactManifest,
}

impl BuiltDelta {
    /// The delta manifest for the envelope plaintext.
    #[must_use]
    pub const fn manifest(&self) -> &ArtifactManifest {
        &self.manifest
    }

    /// Header field 17: the commitment, present **only when the delta is non-empty** (an empty delta
    /// has none, because a commitment for an empty delta would be a second encoding of the same
    /// envelope).
    #[must_use]
    pub fn header_commitment(&self) -> Option<ManifestCommitment> {
        (!self.manifest.is_empty()).then(|| self.manifest.commitment())
    }
}

/// Build the delta of `refs` for `key_epoch` from the producer's directory: every referenced object's
/// ids, each with a durable receipt, whether or not an earlier envelope already published them.
///
/// # Errors
/// [`BuildError`]; no delta is returned on any error.
pub fn build_delta(
    directory: &impl ArtifactDirectory,
    refs: &[PayloadRef],
    key_epoch: u64,
) -> Result<BuiltDelta, BuildError> {
    let mut ids: BTreeSet<[u8; 32]> = BTreeSet::new();
    for reference in refs {
        let logical = match *reference {
            PayloadRef::Record(record) => LogicalRef::Batch(
                directory
                    .batch_of(&record)
                    .ok_or(BuildError::NoArtifact(*reference))?,
            ),
            PayloadRef::Attachment(attachment) => LogicalRef::Attachment(attachment),
        };
        let carried = directory
            .artifacts_of(&logical, key_epoch)
            .filter(|c| !c.is_empty())
            .ok_or(BuildError::NoArtifact(*reference))?;
        for (id, receipt) in carried {
            if receipt != ReceiptState::Durable {
                return Err(BuildError::ReceiptMissing(*reference, id));
            }
            ids.insert(*id.as_bytes());
        }
    }
    let manifest = ArtifactManifest::new(ids.into_iter().collect())
        .map_err(|_| BuildError::TooManyArtifacts)?;
    Ok(BuiltDelta { manifest })
}
