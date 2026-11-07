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

use crate::artifact::{ArtifactId, LogicalBinding, TransportPlaintext};
use crate::envelope::{BoundArtifact, BoundManifest};
use crate::header::{BindingCommitment, ManifestCommitment};
use crate::source_batch::{join, record_ids};

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
    /// The SHA-256 of the artifact's AEAD output that the envelope bound to this id, which the
    /// receiver checked the fetched bytes against before it opened them (C.4 stage 9, C.9).
    pub ciphertext_sha256: [u8; 32],
}

/// An authenticated artifact's plaintext with the hash the envelope bound to its id.
#[derive(Debug, Clone, Copy)]
pub struct BoundPlaintext<'a> {
    /// The opened, authenticated transport plaintext.
    pub plaintext: TransportPlaintext<'a>,
    /// The `ciphertext_sha256` the envelope bound to the artifact's id.
    pub ciphertext_sha256: [u8; 32],
}

impl DeltaArtifact {
    /// The stage-9 input for one **authenticated** artifact: its logical binding and, for a batch
    /// part, the ids of the records its blob holds. The only way to build one from a plaintext, so a
    /// part that does not decode, or whose metadata names no valid binding, has a verdict
    /// ([`Mismatch::PartUndecodable`], [`Mismatch::BadBinding`]) instead of reaching apply. The
    /// producer's own stage-9 self-check builds its inputs the same way.
    ///
    /// # Errors
    /// A [`Mismatch`] naming the artifact.
    pub fn from_plaintext(bound: &BoundPlaintext<'_>) -> Result<Self, Mismatch> {
        let plaintext = &bound.plaintext;
        let id = plaintext.artifact_id;
        let binding = plaintext.binding().map_err(|_| Mismatch::BadBinding(id))?;
        let records = match binding {
            LogicalBinding::Attachment { .. } => Vec::new(),
            LogicalBinding::BatchPart {
                batch_id,
                part_index,
                ..
            } => record_ids(&batch_id, part_index, plaintext.blob)
                .map_err(|_| Mismatch::PartUndecodable(id))?,
        };
        Ok(Self {
            id,
            binding,
            records,
            ciphertext_sha256: bound.ciphertext_sha256,
        })
    }
}

/// What a snapshot at the preceding log position would carry, in this epoch: each logical object's
/// ids. A function of replicated state, never a replica's local cache.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveClosure {
    bound: BTreeMap<LogicalRef, BTreeMap<ArtifactId, [u8; 32]>>,
    /// Each source record of a live batch, with the batch that holds it.
    records: BTreeMap<[u8; 16], [u8; 16]>,
}

impl LiveClosure {
    /// An empty closure.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add `id` to `logical`'s set, with the `ciphertext_sha256` the snapshot at the preceding
    /// position binds to it. The live closure is a set of `(id, hash)` pairs (C.9).
    pub fn insert(&mut self, logical: LogicalRef, id: ArtifactId, ciphertext_sha256: [u8; 32]) {
        self.bound
            .entry(logical)
            .or_default()
            .insert(id, ciphertext_sha256);
    }

    /// The hash the live closure binds to `id`, if the closure holds the id at all.
    #[must_use]
    pub fn hash_of(&self, id: &ArtifactId) -> Option<[u8; 32]> {
        self.bound.values().find_map(|ids| ids.get(id).copied())
    }

    /// Record that live batch `batch` holds source record `record`.
    pub fn insert_record(&mut self, batch: [u8; 16], record: [u8; 16]) {
        self.records.insert(record, batch);
    }

    /// The live batch that holds `record`, if any.
    #[must_use]
    pub fn batch_of_record(&self, record: &[u8; 16]) -> Option<[u8; 16]> {
        self.records.get(record).copied()
    }

    /// The ids bound to `logical`, if any.
    #[must_use]
    pub fn get(&self, logical: &LogicalRef) -> Option<BTreeSet<ArtifactId>> {
        self.bound
            .get(logical)
            .map(|ids| ids.keys().copied().collect())
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
    /// An authenticated artifact's metadata names no valid logical binding.
    #[error("an artifact's logical metadata names no valid binding")]
    BadBinding(ArtifactId),
    /// An authenticated artifact fails its own descriptor, is of a reserved kind, or was refused when
    /// it was opened (a fact about the artifact, unlike a fetch fault).
    #[error("an authenticated artifact is invalid")]
    ArtifactInvalid(ArtifactId),
    /// An authenticated batch part does not decode as a source batch part.
    #[error("a batch part does not decode")]
    PartUndecodable(ArtifactId),
    /// One source record is held more than once across the parts of a batch.
    #[error("a source record is held by more than one part of a batch")]
    RecordDuplicated([u8; 16]),
    /// A source record is held by a batch of the delta and by a **different** batch of the live
    /// closure at the preceding position (T2): it would collide with a live row.
    #[error("a source record is also held by a different live batch")]
    RecordInLiveBatch([u8; 16]),
    /// The parts of a batch decode, but are not what C.9's deterministic split gives for the batch
    /// they join to (an empty part, records out of order across parts, a part over the bound, a
    /// non-greedy split): the same rows would not always give the same parts.
    #[error("the parts of a batch are not its canonical split")]
    BatchNotCanonical([u8; 16]),
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
    /// The live closure at the preceding position holds this id with a ciphertext hash other than
    /// the one the delta binds (C.9): the producer bound a hash that is not the immutable
    /// artifact's. A local mapping entry outside the live closure never decides this.
    #[error("the live closure binds this id to a different ciphertext hash")]
    HashDiffersFromLive(ArtifactId),
}

/// Stage 9(b). The cases the signed spec text does not yet list (a record held by more than one part
/// of a batch, an authenticated part that does not decode as C.9 rows, a batch that is not its
/// canonical split, and a record id held by two batches of the delta or by a delta batch and a
/// different live batch) are spec text pending in S2c part 2 (spec 33), planning ruling on
/// personal-cfo-0g528.2; the same applies to [`check_batch_canonical`] and [`check_delta`]. With every
/// artifact of the delta held and verified, the logical ids they bind must
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
        // A record is held by exactly one part of its batch, whether or not the payload names it:
        // the parts would not join.
        let mut held = BTreeSet::new();
        for (artifact, _, _) in parts {
            for record in &artifact.records {
                if !held.insert(*record) {
                    return Err(Mismatch::RecordDuplicated(*record));
                }
            }
        }
    }
    for (attachment_id, artifacts) in &attachments {
        if artifacts.len() > 1 {
            return Err(Mismatch::AttachmentRebound(*attachment_id));
        }
    }

    // T2: no record id is held by two batches of the delta, named or not, nor by a delta batch and a
    // different batch of the live closure: it would collide on the primary key at materialization.
    let mut holder_of: BTreeMap<[u8; 16], [u8; 16]> = BTreeMap::new();
    for (batch_id, parts) in &batches {
        for (artifact, _, _) in parts {
            for record in &artifact.records {
                // A record twice in one batch was refused above (`RecordDuplicated`), so a previous
                // holder here is always another batch.
                if holder_of.insert(*record, *batch_id).is_some() {
                    return Err(Mismatch::RecordInSeveralBatches(*record));
                }
                if live
                    .batch_of_record(record)
                    .is_some_and(|live_batch| &live_batch != batch_id)
                {
                    return Err(Mismatch::RecordInLiveBatch(*record));
                }
            }
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
        if live.get(&logical).is_some_and(|bound| bound != ids) {
            return Err(Mismatch::Rebinding(logical));
        }
    }
    for (attachment_id, artifacts) in &attachments {
        let logical = LogicalRef::Attachment(*attachment_id);
        let ids: BTreeSet<ArtifactId> = artifacts.iter().map(|a| a.id).collect();
        if live.get(&logical).is_some_and(|bound| bound != ids) {
            return Err(Mismatch::Rebinding(logical));
        }
    }
    // The hash bound to an id the live closure holds is the hash the live closure binds to it.
    for artifact in delta {
        if live
            .hash_of(&artifact.id)
            .is_some_and(|live_hash| live_hash != artifact.ciphertext_sha256)
        {
            return Err(Mismatch::HashDiffersFromLive(artifact.id));
        }
    }
    Ok(())
}

/// One part of a batch as `(part_index, part_count, blob)`.
pub type PartRef<'a> = (u32, u32, &'a [u8]);

/// T1: a batch is valid only if its parts decode **and** re-splitting the batch they join to by
/// C.9's deterministic split gives exactly these blobs at these indices. This closes the whole class
/// of parts that join but are not what the rows would always produce (an empty part, records out of
/// order across parts, a part over the bound, a non-greedy split).
///
/// Spec text pending in S2c part 2 (spec 33), planning ruling on personal-cfo-0g528.2. The parts may
/// be given in any order.
///
/// # Errors
/// [`Mismatch::BatchNotCanonical`].
pub fn check_batch_canonical(
    batch_id: &[u8; 16],
    parts: &[(u32, u32, &[u8])],
) -> Result<(), Mismatch> {
    let not_canonical = Mismatch::BatchNotCanonical(*batch_id);
    let joined = join(batch_id, parts).map_err(|_| not_canonical)?;
    let expected = joined.split().map_err(|_| not_canonical)?;
    // `join` has checked that the indices are exactly `0..count` and that `count` is the number of
    // parts, so what remains is the number of parts and each blob, in index order.
    let mut ordered = parts.to_vec();
    ordered.sort_by_key(|p| p.0);
    let same = expected.len() == ordered.len()
        && expected
            .iter()
            .zip(&ordered)
            .all(|(e, p)| e.blob.as_slice() == p.2);
    if same {
        Ok(())
    } else {
        Err(not_canonical)
    }
}

/// Stage 9(b) end to end, from the **plaintext of every artifact in the delta**: build each stage-9
/// input ([`DeltaArtifact::from_plaintext`]), run [`check_closure`], then check every batch is its
/// canonical split ([`check_batch_canonical`]). A caller that skips the network for an id it already
/// holds must still pass that artifact's plaintext here: a cache saves a fetch, never a decode, so an
/// id fetched for an envelope that was invalid for another reason cannot stand in for a decoded part.
///
/// Spec text pending in S2c part 2 (spec 33), planning ruling on personal-cfo-0g528.2.
///
/// # Errors
/// The first [`Mismatch`] found, checking the artifacts in the order given (the reason is a
/// diagnostic; the verdict class is `ArtifactClosureMismatch` whichever is reported first).
pub fn check_delta(
    refs: &[PayloadRef],
    plaintexts: &[BoundPlaintext<'_>],
    live: &LiveClosure,
) -> Result<(), Mismatch> {
    let delta = plaintexts
        .iter()
        .map(DeltaArtifact::from_plaintext)
        .collect::<Result<Vec<_>, _>>()?;
    check_closure(refs, &delta, live)?;
    let mut by_batch: BTreeMap<[u8; 16], Vec<PartRef<'_>>> = BTreeMap::new();
    for (bound, artifact) in plaintexts.iter().zip(&delta) {
        let plaintext = &bound.plaintext;
        if let LogicalBinding::BatchPart {
            batch_id,
            part_index,
            part_count,
        } = artifact.binding
        {
            by_batch
                .entry(batch_id)
                .or_default()
                .push((part_index, part_count, plaintext.blob));
        }
    }
    for (batch_id, parts) in &by_batch {
        check_batch_canonical(batch_id, parts)?;
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
    /// A receipt that matched what the client computed has been recorded. It carries the
    /// `ciphertext_sha256` the client verified when it published the artifact or fetched it: the
    /// only hash a producer may bind (C.9), never a transport or blob plaintext digest.
    Durable {
        /// The SHA-256 of the artifact's AEAD output.
        ciphertext_sha256: [u8; 32],
    },
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
    manifest: BoundManifest,
}

impl BuiltDelta {
    /// The delta manifest for the envelope plaintext: each id with the hash bound to it.
    #[must_use]
    pub const fn manifest(&self) -> &BoundManifest {
        &self.manifest
    }

    /// Header field 17: the commitment, present **only when the delta is non-empty** (an empty delta
    /// has none, because a commitment for an empty delta would be a second encoding of the same
    /// envelope).
    #[must_use]
    pub fn header_commitment(&self) -> Option<ManifestCommitment> {
        (!self.manifest.is_empty()).then(|| self.manifest.commitment())
    }

    /// Header field 18: the binding commitment, present exactly when field 17 is.
    #[must_use]
    pub fn header_binding(&self) -> Option<BindingCommitment> {
        (!self.manifest.is_empty()).then(|| self.manifest.binding())
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
    let mut ids: BTreeMap<[u8; 32], [u8; 32]> = BTreeMap::new();
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
            let ReceiptState::Durable { ciphertext_sha256 } = receipt else {
                return Err(BuildError::ReceiptMissing(*reference, id));
            };
            ids.insert(*id.as_bytes(), ciphertext_sha256);
        }
    }
    let manifest = BoundManifest::new(
        ids.into_iter()
            .map(|(id, ciphertext_sha256)| BoundArtifact {
                id,
                ciphertext_sha256,
            })
            .collect(),
    )
    .map_err(|_| BuildError::TooManyArtifacts)?;
    Ok(BuiltDelta { manifest })
}
