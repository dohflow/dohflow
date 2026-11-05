//! Stage 9 of C.4 and the receipt-gated delta (vault-format.md C.9; personal-cfo-0g528.2 S2c):
//! set equality between a payload's references and the delta's bound logical ids, the parts rules,
//! the one-binding rule against the live closure, and receipt before reference.

use std::collections::BTreeMap;

use sync_core::artifact::{ArtifactId, LogicalBinding};
use sync_core::artifact_closure::{
    build_delta, check_closure, ArtifactDirectory, BuildError, DeltaArtifact, LiveClosure,
    LogicalRef, Mismatch, PayloadRef, ReceiptState,
};

fn aid(n: u8) -> ArtifactId {
    ArtifactId::from_derived([n; 32])
}
fn rid(n: u8) -> [u8; 16] {
    [n; 16]
}

fn part(id: u8, batch: u8, index: u32, count: u32, records: &[u8]) -> DeltaArtifact {
    DeltaArtifact {
        id: aid(id),
        binding: LogicalBinding::BatchPart {
            batch_id: rid(batch),
            part_index: index,
            part_count: count,
        },
        records: records.iter().map(|r| rid(*r)).collect(),
    }
}
fn attachment(id: u8, attachment: u8) -> DeltaArtifact {
    DeltaArtifact {
        id: aid(id),
        binding: LogicalBinding::Attachment {
            attachment_id: rid(attachment),
        },
        records: Vec::new(),
    }
}
fn record(n: u8) -> PayloadRef {
    PayloadRef::Record(rid(n))
}
fn att(n: u8) -> PayloadRef {
    PayloadRef::Attachment(rid(n))
}

// ---- stage 9(b) ---------------------------------------------------------------------------------

#[test]
fn a_matching_closure_passes() {
    let refs = [record(1), record(2), att(7)];
    let delta = [
        part(10, 0xb1, 0, 2, &[1]),
        part(11, 0xb1, 1, 2, &[2]),
        attachment(12, 7),
    ];
    assert_eq!(check_closure(&refs, &delta, &LiveClosure::new()), Ok(()));
    // An empty payload with an empty delta is fine.
    assert_eq!(check_closure(&[], &[], &LiveClosure::new()), Ok(()));
}

#[test]
fn a_reference_with_no_artifact_is_a_mismatch() {
    let live = LiveClosure::new();
    assert_eq!(
        check_closure(&[record(1)], &[], &live),
        Err(Mismatch::RecordInNoBatch(rid(1)))
    );
    assert_eq!(
        check_closure(&[att(7)], &[], &live),
        Err(Mismatch::UnresolvedReference(att(7)))
    );
    // A record named but held by no part of the referenced batch.
    assert_eq!(
        check_closure(&[record(9)], &[part(10, 0xb1, 0, 1, &[1])], &live),
        Err(Mismatch::RecordInNoBatch(rid(9)))
    );
}

#[test]
fn an_artifact_no_reference_names_cannot_be_pinned() {
    let live = LiveClosure::new();
    // A whole extra batch, an extra attachment, and an extra attachment alongside valid ones.
    assert_eq!(
        check_closure(
            &[record(1)],
            &[part(10, 0xb1, 0, 1, &[1]), part(11, 0xb2, 0, 1, &[2])],
            &live
        ),
        Err(Mismatch::UnreferencedArtifact(aid(11)))
    );
    assert_eq!(
        check_closure(&[], &[attachment(12, 7)], &live),
        Err(Mismatch::UnreferencedArtifact(aid(12)))
    );
    assert_eq!(
        check_closure(&[att(7)], &[attachment(12, 7), attachment(13, 8)], &live),
        Err(Mismatch::UnreferencedArtifact(aid(13)))
    );
}

#[test]
fn the_parts_of_a_batch_are_exactly_zero_to_count_minus_one() {
    let live = LiveClosure::new();
    let refs = [record(1)];
    // A gap: index 1 of 3 is missing.
    assert_eq!(
        check_closure(
            &refs,
            &[part(10, 0xb1, 0, 3, &[1]), part(11, 0xb1, 2, 3, &[])],
            &live
        ),
        Err(Mismatch::PartMissing(rid(0xb1)))
    );
    // Only part 0 of 2.
    assert_eq!(
        check_closure(&refs, &[part(10, 0xb1, 0, 2, &[1])], &live),
        Err(Mismatch::PartMissing(rid(0xb1)))
    );
    // A duplicated index.
    assert_eq!(
        check_closure(
            &refs,
            &[part(10, 0xb1, 0, 2, &[1]), part(11, 0xb1, 0, 2, &[])],
            &live
        ),
        Err(Mismatch::PartDuplicated(rid(0xb1)))
    );
    // Parts disagreeing on part_count.
    assert_eq!(
        check_closure(
            &refs,
            &[part(10, 0xb1, 0, 2, &[1]), part(11, 0xb1, 1, 3, &[])],
            &live
        ),
        Err(Mismatch::PartsDisagree(rid(0xb1)))
    );
    // More parts than the count says is a duplicate or a gap, never a pass.
    assert!(check_closure(
        &refs,
        &[part(10, 0xb1, 0, 1, &[1]), part(11, 0xb1, 1, 1, &[])],
        &live
    )
    .is_err());
}

#[test]
fn a_record_is_in_exactly_one_referenced_batch() {
    let live = LiveClosure::new();
    assert_eq!(
        check_closure(
            &[record(1)],
            &[part(10, 0xb1, 0, 1, &[1]), part(11, 0xb2, 0, 1, &[1])],
            &live
        ),
        Err(Mismatch::RecordInSeveralBatches(rid(1)))
    );
    // Two records in two batches, each referenced, is fine.
    assert_eq!(
        check_closure(
            &[record(1), record(2)],
            &[part(10, 0xb1, 0, 1, &[1]), part(11, 0xb2, 0, 1, &[2])],
            &live
        ),
        Ok(())
    );
}

#[test]
fn an_attachment_bound_to_two_artifacts_in_one_delta_is_a_mismatch() {
    assert_eq!(
        check_closure(
            &[att(7)],
            &[attachment(12, 7), attachment(13, 7)],
            &LiveClosure::new()
        ),
        Err(Mismatch::AttachmentRebound(rid(7)))
    );
}

// ---- one binding per logical artifact, judged against the live closure ---------------------------

fn live_with(batch: u8, ids: &[u8]) -> LiveClosure {
    let mut live = LiveClosure::new();
    for id in ids {
        live.insert(LogicalRef::Batch(rid(batch)), aid(*id));
    }
    live
}

#[test]
fn a_rebinding_of_a_live_batch_or_attachment_is_a_mismatch() {
    // The batch is live as {10, 11}; the delta binds it to {10, 14}.
    let live = live_with(0xb1, &[10, 11]);
    assert_eq!(
        check_closure(
            &[record(1)],
            &[part(10, 0xb1, 0, 2, &[1]), part(14, 0xb1, 1, 2, &[])],
            &live
        ),
        Err(Mismatch::Rebinding(LogicalRef::Batch(rid(0xb1))))
    );
    // The same set passes (an earlier envelope's artifact referenced again).
    assert_eq!(
        check_closure(
            &[record(1)],
            &[part(10, 0xb1, 0, 2, &[1]), part(11, 0xb1, 1, 2, &[])],
            &live
        ),
        Ok(())
    );
    let mut live = LiveClosure::new();
    live.insert(LogicalRef::Attachment(rid(7)), aid(12));
    assert_eq!(
        check_closure(&[att(7)], &[attachment(13, 7)], &live),
        Err(Mismatch::Rebinding(LogicalRef::Attachment(rid(7))))
    );
    assert_eq!(
        check_closure(&[att(7)], &[attachment(12, 7)], &live),
        Ok(())
    );
}

#[test]
fn a_batch_that_is_not_live_may_be_bound_freely() {
    // Bootstrapped later in the epoch (the batch's last reference was removed): not live, so any
    // binding is the first one. The verdict is a function of the live closure alone.
    let live = LiveClosure::new();
    assert_eq!(
        check_closure(&[record(1)], &[part(14, 0xb1, 0, 1, &[1])], &live),
        Ok(())
    );
}

#[test]
fn the_verdict_does_not_depend_on_the_order_of_the_delta_or_the_refs() {
    let refs = [record(2), att(7), record(1)];
    let delta = [
        attachment(12, 7),
        part(11, 0xb1, 1, 2, &[2]),
        part(10, 0xb1, 0, 2, &[1]),
    ];
    let mut reversed_delta = delta.to_vec();
    reversed_delta.reverse();
    let mut reversed_refs = refs.to_vec();
    reversed_refs.reverse();
    let live = LiveClosure::new();
    assert_eq!(
        check_closure(&refs, &delta, &live),
        check_closure(&reversed_refs, &reversed_delta, &live)
    );
    // And a wrong closure gives the same mismatch either way.
    let bad_delta = [part(10, 0xb1, 0, 2, &[1])];
    assert_eq!(
        check_closure(&refs, &bad_delta, &live),
        check_closure(&reversed_refs, &bad_delta, &live)
    );
}

// ---- receipt before reference ----------------------------------------------------------------------------

#[derive(Default)]
struct Directory {
    batch_of: BTreeMap<[u8; 16], [u8; 16]>,
    artifacts: BTreeMap<(LogicalRef, u64), Vec<(ArtifactId, ReceiptState)>>,
}

impl ArtifactDirectory for Directory {
    fn batch_of(&self, record: &[u8; 16]) -> Option<[u8; 16]> {
        self.batch_of.get(record).copied()
    }
    fn artifacts_of(
        &self,
        logical: &LogicalRef,
        key_epoch: u64,
    ) -> Option<Vec<(ArtifactId, ReceiptState)>> {
        self.artifacts.get(&(*logical, key_epoch)).cloned()
    }
}

fn directory() -> Directory {
    let mut d = Directory::default();
    d.batch_of.insert(rid(1), rid(0xb1));
    d.batch_of.insert(rid(2), rid(0xb1));
    d.artifacts.insert(
        (LogicalRef::Batch(rid(0xb1)), 1),
        vec![
            (aid(11), ReceiptState::Durable),
            (aid(10), ReceiptState::Durable),
        ],
    );
    d.artifacts.insert(
        (LogicalRef::Attachment(rid(7)), 1),
        vec![(aid(12), ReceiptState::Durable)],
    );
    d
}

#[test]
fn a_delta_is_every_referenced_artifact_sorted_with_its_commitment() {
    let built = build_delta(&directory(), &[record(1), record(2), att(7)], 1).unwrap();
    // Both records name one batch: its two parts appear once, with the attachment, ascending.
    let ids: Vec<[u8; 32]> = built.manifest().ids().to_vec();
    assert_eq!(ids, vec![[10; 32], [11; 32], [12; 32]]);
    assert_eq!(
        built.header_commitment(),
        Some(built.manifest().commitment())
    );
}

#[test]
fn an_empty_delta_has_no_header_commitment() {
    let built = build_delta(&directory(), &[], 1).unwrap();
    assert!(built.manifest().is_empty());
    assert_eq!(built.header_commitment(), None);
}

#[test]
fn a_reference_cannot_be_built_without_a_durable_receipt() {
    let mut d = directory();
    d.artifacts.insert(
        (LogicalRef::Batch(rid(0xb1)), 1),
        vec![
            (aid(10), ReceiptState::Durable),
            (aid(11), ReceiptState::None),
        ],
    );
    assert_eq!(
        build_delta(&d, &[record(1)], 1),
        Err(BuildError::ReceiptMissing(record(1), aid(11)))
    );
    // An attachment with no receipt either.
    d.artifacts.insert(
        (LogicalRef::Attachment(rid(7)), 1),
        vec![(aid(12), ReceiptState::None)],
    );
    assert_eq!(
        build_delta(&d, &[att(7)], 1),
        Err(BuildError::ReceiptMissing(att(7), aid(12)))
    );
}

#[test]
fn a_reference_with_no_artifact_in_this_epoch_is_not_built() {
    // The artifact exists for epoch 1; a producer at epoch 2 has not re-addressed it, so there is
    // nothing it may reference (old-epoch outbox entries need new-epoch receipts first, C.7).
    assert_eq!(
        build_delta(&directory(), &[att(7)], 2),
        Err(BuildError::NoArtifact(att(7)))
    );
    // A record the vault does not know a batch for.
    assert_eq!(
        build_delta(&directory(), &[record(9)], 1),
        Err(BuildError::NoArtifact(record(9)))
    );
    // An artifact entry that is empty is no artifact.
    let mut d = directory();
    d.artifacts
        .insert((LogicalRef::Attachment(rid(8)), 1), Vec::new());
    assert_eq!(
        build_delta(&d, &[att(8)], 1),
        Err(BuildError::NoArtifact(att(8)))
    );
}

#[test]
fn a_delta_over_the_envelope_bound_is_refused() {
    let mut d = Directory::default();
    let mut refs = Vec::new();
    for n in 0..=4096_u32 {
        let attachment_id = {
            let mut id = [0_u8; 16];
            id[..4].copy_from_slice(&n.to_be_bytes());
            id
        };
        let mut artifact = [0_u8; 32];
        artifact[..4].copy_from_slice(&n.to_be_bytes());
        d.artifacts.insert(
            (LogicalRef::Attachment(attachment_id), 1),
            vec![(ArtifactId::from_derived(artifact), ReceiptState::Durable)],
        );
        refs.push(PayloadRef::Attachment(attachment_id));
    }
    assert_eq!(
        build_delta(&d, &refs[..4096], 1).map(|b| b.manifest().ids().len()),
        Ok(4096)
    );
    assert_eq!(build_delta(&d, &refs, 1), Err(BuildError::TooManyArtifacts));
}
