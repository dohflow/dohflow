//! Receiving the artifacts an envelope references (`docs/architecture/vault-format.md` C.4 stage 9,
//! C.9; `personal-cfo-0g528.2`): the stages, in order, from the delta manifest the envelope's
//! plaintext carries to rows and blobs on this device.
//!
//! 1. **The live-hash rule, before any fetch.** An id the live closure at the preceding position holds
//!    with a hash other than the one the delta binds is `ArtifactClosureMismatch`, decided from the
//!    envelope and replicated state alone: no request is made, so a faulty or hostile service can
//!    neither delay nor choose the verdict.
//! 2. **Fetch, hash first.** Bytes that do not hash to the bound value are not the artifact the
//!    envelope names: a service or transport fault, **not determinable** (9(a)), retried, never a
//!    basis to honor a void. Bytes that do hash to it and then fail the outer AEAD or the descriptor
//!    are the producer's fault (9(b)). If any artifact of the delta is not determinable the whole
//!    delta is, whatever the others show, so a verdict never depends on which request failed first.
//! 3. **Judge the closure** ([`check_delta`]) once every artifact is held and verified: set
//!    equality with the payload's references, the batches' structure and canonical split, one
//!    binding per logical artifact against the live closure.
//! 4. **Materialize**, only after every verdict is in: nothing is written to the vault for an
//!    envelope that is invalid or not determinable. A failure here is a reason local to the device.
//!
//! A device that already holds an artifact still fetches it: the no-refetch optimization of C.9 needs
//! the plaintext a cache hit does not keep, and is a later change.

use sync_core::artifact::{ArtifactId, LogicalBinding};
use sync_core::artifact_closure::{check_delta, BoundPlaintext, LiveClosure, Mismatch, PayloadRef};
use sync_core::artifact_transport::ArtifactTransport;
use sync_core::envelope::BoundManifest;
use sync_core::source_batch::join;
use thiserror::Error;
use vault_crypto::sync_artifact::OpenedArtifact;
use vault_crypto::sync_artifact_flow::{fetch, FetchError};
use vault_crypto::sync_envelope::EpochKey;

use crate::sync_materialize::{BatchPartRef, MaterializeError};
use crate::DbWorker;

/// Why a delta was not received.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ReceiveError {
    /// An artifact of the delta could not be judged: the service did not answer, did not hold it, or
    /// served bytes that are not the ones the envelope bound (stage 9(a)). Retry; never a verdict.
    #[error("the delta is not determinable: {0}")]
    NotDeterminable(FetchError),
    /// The delta is wrong (stage 9(b), `ArtifactClosureMismatch`): deterministic from the envelope,
    /// the artifacts and the live closure, and voidable.
    #[error("the delta is invalid: {0}")]
    Mismatch(Mismatch),
    /// The delta is valid and this device could not write it (`ArtifactMaterializationFailed`).
    /// A reason local to the device; the vault is unchanged.
    #[error("the delta could not be materialized on this device")]
    Materialization(#[from] MaterializeError),
}

/// What a received delta wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Received {
    /// Attachments materialized (their crypto fields staged for the attach command).
    pub attachments: usize,
    /// Source batches materialized.
    pub batches: usize,
}

/// Receive the artifacts of `delta` (the envelope plaintext's manifest of ids and bound hashes) for
/// an envelope whose payload references `refs`, judged against `live`, the closure at the preceding
/// log position. Every stage runs in the order above; nothing is written before the last verdict.
///
/// # Errors
/// A [`ReceiveError`].
pub fn receive_delta(
    worker: &DbWorker,
    transport: &impl ArtifactTransport,
    key: &EpochKey,
    delta: &BoundManifest,
    refs: &[PayloadRef],
    live: &LiveClosure,
) -> Result<Received, ReceiveError> {
    // 1. The live-hash rule, before any request.
    for entry in delta.entries() {
        let id = ArtifactId::from_derived(entry.id);
        if live
            .hash_of(&id)
            .is_some_and(|live_hash| live_hash != entry.ciphertext_sha256)
        {
            return Err(ReceiveError::Mismatch(Mismatch::HashDiffersFromLive(id)));
        }
    }

    // 2. Fetch every artifact, hash first. A fault of the service makes the delta not determinable
    //    whatever the other artifacts show; only then is a producer's fault a verdict.
    let mut opened: Vec<(ArtifactId, [u8; 32], OpenedArtifact)> =
        Vec::with_capacity(delta.entries().len());
    let mut producer_fault: Option<Mismatch> = None;
    for entry in delta.entries() {
        let id = ArtifactId::from_derived(entry.id);
        match fetch(transport, key, &id, &entry.ciphertext_sha256) {
            Ok(artifact) => opened.push((id, entry.ciphertext_sha256, artifact)),
            Err(error) => match error.mismatch(id) {
                Some(mismatch) => {
                    producer_fault.get_or_insert(mismatch);
                }
                None => return Err(ReceiveError::NotDeterminable(error)),
            },
        }
    }
    if let Some(mismatch) = producer_fault {
        return Err(ReceiveError::Mismatch(mismatch));
    }

    // 3. The closure, once every artifact is held and verified.
    let plaintexts: Vec<BoundPlaintext<'_>> = opened
        .iter()
        .map(|(_, hash, artifact)| BoundPlaintext {
            plaintext: artifact.plaintext(),
            ciphertext_sha256: *hash,
        })
        .collect();
    check_delta(refs, &plaintexts, live).map_err(ReceiveError::Mismatch)?;

    // 4. Materialize. Attachments one by one; the parts of a batch together.
    let mut received = Received::default();
    let mut batches: std::collections::BTreeMap<[u8; 16], Vec<(BatchPartRef, usize)>> =
        std::collections::BTreeMap::new();
    for (position, (id, hash, artifact)) in opened.iter().enumerate() {
        let plaintext = artifact.plaintext();
        match plaintext.binding() {
            Ok(LogicalBinding::Attachment { .. }) => {
                worker.materialize_attachment(key.key_epoch(), hash, artifact)?;
                received.attachments += 1;
            }
            Ok(LogicalBinding::BatchPart {
                batch_id,
                part_index,
                part_count,
            }) => batches.entry(batch_id).or_default().push((
                BatchPartRef {
                    artifact_id: *id.as_bytes(),
                    ciphertext_sha256: *hash,
                    index: part_index,
                    count: part_count,
                },
                position,
            )),
            // `check_delta` read every binding already.
            Err(_) => return Err(ReceiveError::Mismatch(Mismatch::BadBinding(*id))),
        }
    }
    for (batch_id, parts) in &batches {
        let blobs: Vec<(u32, u32, &[u8])> = parts
            .iter()
            .map(|(part, position)| (part.index, part.count, opened[*position].2.plaintext().blob))
            .collect();
        // `check_delta` verified the canonical split of the same parts, so this cannot fail.
        let batch = join(batch_id, &blobs)
            .map_err(|_| ReceiveError::Mismatch(Mismatch::BatchNotCanonical(*batch_id)))?;
        let refs: Vec<BatchPartRef> = parts.iter().map(|(part, _)| *part).collect();
        worker.materialize_batch(key.key_epoch(), &refs, &batch)?;
        received.batches += 1;
    }
    Ok(received)
}
