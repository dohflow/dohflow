//! Stage 9 of C.4 and the receipt-gated delta (vault-format.md C.9; personal-cfo-0g528.2 S2c):
//! set equality between a payload's references and the delta's bound logical ids, the parts rules,
//! the one-binding rule against the live closure, and receipt before reference.

use std::collections::BTreeMap;

use sync_core::artifact::{ArtifactId, ArtifactKind, LogicalBinding, TransportPlaintext};
use sync_core::artifact_closure::{
    build_delta, check_batch_canonical, check_closure, check_delta, ArtifactDirectory, BuildError,
    DeltaArtifact, LiveClosure, LogicalRef, Mismatch, PayloadRef, ReceiptState,
};
use sync_core::source_batch::{record_json, BatchRow, RecordRow, SourceBatch};

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

// ---- the stage-9 input: DeltaArtifact::from_plaintext ----------------------------------------------

const BATCH: [u8; 16] = [0xb1; 16];

fn one_record_batch() -> SourceBatch {
    SourceBatch {
        batch: BatchRow {
            id: BATCH,
            source_type: "csv".into(),
            source_name: None,
            file_fingerprint: None,
            parser_version: None,
            summary_json: None,
            imported_at: None,
            created_at: "2026-06-01T00:00:00Z".into(),
            connector_connection_id: None,
        },
        runs: Vec::new(),
        records: vec![RecordRow {
            id: rid(1),
            source_batch_id: BATCH,
            external_id: None,
            source_hash: "h".into(),
            normalized_json: "{}".into(),
            parse_confidence_bps: None,
            created_at: "2026-06-01T00:00:01Z".into(),
        }],
    }
}

fn meta(index: u32, count: u32) -> Vec<u8> {
    format!(
        r#"{{"batch_id":"b1b1b1b1-b1b1-b1b1-b1b1-b1b1b1b1b1b1","part_count":"{count}","part_index":"{index}"}}"#
    )
    .into_bytes()
}

fn plaintext<'a>(id: u8, metadata: &'a [u8], blob: &'a [u8]) -> TransportPlaintext<'a> {
    TransportPlaintext {
        artifact_id: aid(id),
        kind: ArtifactKind::SourceBatchPart,
        logical_metadata: metadata,
        blob,
    }
}

#[test]
fn a_real_batch_part_builds_its_stage_9_input_with_its_record_ids() {
    let parts = one_record_batch().split().unwrap();
    let m = meta(0, 1);
    let built = DeltaArtifact::from_plaintext(&plaintext(10, &m, &parts[0].blob)).unwrap();
    assert_eq!(built, part(10, 0xb1, 0, 1, &[1]));
    assert_eq!(
        check_closure(&[record(1)], &[built], &LiveClosure::new()),
        Ok(())
    );
    // An attachment has no records.
    let attachment_meta = br#"{"attachment_id":"07070707-0707-0707-0707-070707070707"}"#;
    let attachment = TransportPlaintext {
        artifact_id: aid(12),
        kind: ArtifactKind::Attachment,
        logical_metadata: attachment_meta,
        blob: b"x",
    };
    assert_eq!(
        DeltaArtifact::from_plaintext(&attachment),
        Ok(self::attachment(12, 7))
    );
}

#[test]
fn a_record_in_two_parts_of_one_batch_is_a_mismatch_not_a_pass() {
    // 04a's probe: part 0 (from split) holds r1, and part 1 also holds r1, with part_count 2.
    let parts = one_record_batch().split().unwrap();
    let record_json = {
        let blob = std::str::from_utf8(&parts[0].blob).unwrap();
        let start = blob.rfind(r#""records":["#).unwrap() + r#""records":["#.len();
        blob[start..blob.len() - 2].to_owned()
    };
    let part1 = format!(r#"{{"records":[{record_json}]}}"#);
    let (m0, m1) = (meta(0, 2), meta(1, 2));
    let p0 = DeltaArtifact::from_plaintext(&plaintext(10, &m0, &parts[0].blob)).unwrap();
    let p1 = DeltaArtifact::from_plaintext(&plaintext(11, &m1, part1.as_bytes())).unwrap();
    assert_eq!(
        check_closure(&[record(1)], &[p0.clone(), p1], &LiveClosure::new()),
        Err(Mismatch::RecordDuplicated(rid(1)))
    );
    // The same defect on hand-built inputs, and with the record not named by the payload.
    let live = LiveClosure::new();
    assert_eq!(
        check_closure(
            &[record(1)],
            &[part(10, 0xb1, 0, 2, &[1]), part(11, 0xb1, 1, 2, &[1])],
            &live
        ),
        Err(Mismatch::RecordDuplicated(rid(1)))
    );
    assert_eq!(
        check_closure(
            &[record(2)],
            &[part(10, 0xb1, 0, 2, &[1, 2]), part(11, 0xb1, 1, 2, &[1])],
            &live
        ),
        Err(Mismatch::RecordDuplicated(rid(1)))
    );
    // Distinct records across parts are fine.
    assert_eq!(
        check_closure(
            &[record(1), record(2)],
            &[part(10, 0xb1, 0, 2, &[1]), part(11, 0xb1, 1, 2, &[2])],
            &live
        ),
        Ok(())
    );
}

#[test]
fn an_authenticated_part_that_does_not_decode_is_a_mismatch() {
    let parts = one_record_batch().split().unwrap();
    let (m0, m1) = (meta(0, 2), meta(1, 2));
    let undecodable = Err(Mismatch::PartUndecodable(aid(10)));
    // Garbage, an unknown member, an empty object, and a non-canonical blob.
    for blob in [
        &b"not json"[..],
        br#"{"records":[],"extra":[]}"#,
        b"{}",
        br#"{"records": []}"#,
    ] {
        assert_eq!(
            DeltaArtifact::from_plaintext(&plaintext(10, &m1, blob)),
            undecodable,
            "{blob:?}"
        );
    }
    // Part 0's shape under a later part's metadata, and a later part's shape as part 0.
    assert_eq!(
        DeltaArtifact::from_plaintext(&plaintext(10, &m1, &parts[0].blob)),
        undecodable
    );
    assert_eq!(
        DeltaArtifact::from_plaintext(&plaintext(10, &m0, br#"{"records":[]}"#)),
        undecodable
    );
    // Rows of another batch than the metadata names.
    let other = SourceBatch {
        batch: BatchRow {
            id: [0xee; 16],
            ..one_record_batch().batch
        },
        records: vec![RecordRow {
            source_batch_id: [0xee; 16],
            ..one_record_batch().records[0].clone()
        }],
        runs: Vec::new(),
    };
    let foreign = other.split().unwrap();
    assert_eq!(
        DeltaArtifact::from_plaintext(&plaintext(10, &m0, &foreign[0].blob)),
        undecodable
    );
}

#[test]
fn an_authenticated_artifact_with_metadata_naming_no_binding_is_a_mismatch() {
    let bad = Err(Mismatch::BadBinding(aid(10)));
    // A part index at the count, and a missing member.
    assert_eq!(
        DeltaArtifact::from_plaintext(&plaintext(10, &meta(2, 2), b"{}")),
        bad
    );
    assert_eq!(
        DeltaArtifact::from_plaintext(&plaintext(10, br#"{"batch_id":"x"}"#, b"{}")),
        bad
    );
    let attachment = TransportPlaintext {
        artifact_id: aid(10),
        kind: ArtifactKind::Attachment,
        logical_metadata: br#"{"mime_type":"text/plain"}"#,
        blob: b"x",
    };
    assert_eq!(DeltaArtifact::from_plaintext(&attachment), bad);
}

// ---- T1: a batch is its canonical split --------------------------------------------------------------

fn rec(n: u8, padding: usize) -> RecordRow {
    RecordRow {
        id: rid(n),
        normalized_json: "x".repeat(padding),
        ..one_record_batch().records[0].clone()
    }
}

fn batch_of(records: Vec<RecordRow>) -> SourceBatch {
    SourceBatch {
        records,
        ..one_record_batch()
    }
}

fn records_only(records: &[RecordRow]) -> Vec<u8> {
    let json: Vec<String> = records
        .iter()
        .map(|r| String::from_utf8(record_json(r).unwrap()).unwrap())
        .collect();
    format!(r#"{{"records":[{}]}}"#, json.join(",")).into_bytes()
}

/// Part plaintexts from `(blob)` in order, with ids 10, 11, ... and `count` taken from the list.
fn delta_of(blobs: &[Vec<u8>]) -> Vec<(u8, Vec<u8>, Vec<u8>)> {
    let count = u32::try_from(blobs.len()).unwrap();
    blobs
        .iter()
        .enumerate()
        .map(|(i, blob)| {
            (
                10 + u8::try_from(i).unwrap(),
                meta(u32::try_from(i).unwrap(), count),
                blob.clone(),
            )
        })
        .collect()
}

fn verdict(refs: &[PayloadRef], blobs: &[Vec<u8>]) -> Result<(), Mismatch> {
    let owned = delta_of(blobs);
    let plaintexts: Vec<TransportPlaintext<'_>> = owned
        .iter()
        .map(|(id, metadata, blob)| plaintext(*id, metadata, blob))
        .collect();
    check_delta(refs, &plaintexts, &LiveClosure::new())
}

fn canonical_blobs(b: &SourceBatch) -> Vec<Vec<u8>> {
    b.split().unwrap().into_iter().map(|p| p.blob).collect()
}

#[test]
fn a_canonical_batch_of_one_part_and_of_many_passes_stage_9() {
    let one = batch_of(vec![rec(1, 20)]);
    assert_eq!(verdict(&[record(1)], &canonical_blobs(&one)), Ok(()));
    let many = batch_of((1..=250).map(|n| rec(n, 20_000)).collect());
    let blobs = canonical_blobs(&many);
    assert!(blobs.len() >= 2, "{} parts", blobs.len());
    assert_eq!(verdict(&[record(1), record(250)], &blobs), Ok(()));
}

#[test]
fn a_later_part_with_no_records_is_not_canonical() {
    let mut blobs = canonical_blobs(&batch_of(vec![rec(1, 20)]));
    blobs.push(br#"{"records":[]}"#.to_vec());
    assert_eq!(
        verdict(&[record(1)], &blobs),
        Err(Mismatch::BatchNotCanonical(BATCH))
    );
}

#[test]
fn part_zero_with_no_records_is_not_canonical() {
    let mut blobs = canonical_blobs(&batch_of(Vec::new()));
    blobs.push(records_only(&[rec(1, 20)]));
    assert_eq!(
        verdict(&[record(1)], &blobs),
        Err(Mismatch::BatchNotCanonical(BATCH))
    );
}

#[test]
fn records_out_of_order_across_parts_are_not_canonical() {
    let mut blobs = canonical_blobs(&batch_of(vec![rec(2, 20)]));
    blobs.push(records_only(&[rec(1, 20)]));
    assert_eq!(
        verdict(&[record(1), record(2)], &blobs),
        Err(Mismatch::BatchNotCanonical(BATCH))
    );
}

#[test]
fn a_part_over_the_bound_is_not_canonical() {
    // Five records of about 1 MB split into two parts; merge them into one oversize part.
    let rows: Vec<RecordRow> = (1..=5).map(|n| rec(n, 1_000_000 - 400)).collect();
    let parts = canonical_blobs(&batch_of(rows));
    assert_eq!(parts.len(), 2);
    let second = std::str::from_utf8(&parts[1]).unwrap();
    let inner = &second[r#"{"records":["#.len()..second.len() - 2];
    let first = std::str::from_utf8(&parts[0]).unwrap();
    let merged = format!("{},{inner}]}}", &first[..first.len() - 2]).into_bytes();
    assert!(merged.len() > 1 << 22);
    assert_eq!(
        verdict(&[record(1), record(5)], &[merged]),
        Err(Mismatch::BatchNotCanonical(BATCH))
    );
}

#[test]
fn a_non_greedy_split_is_not_canonical() {
    // Both records fit one part; carrying them as two is not what the rows always give.
    let mut blobs = canonical_blobs(&batch_of(vec![rec(1, 20)]));
    blobs.push(records_only(&[rec(2, 20)]));
    assert_eq!(
        verdict(&[record(1), record(2)], &blobs),
        Err(Mismatch::BatchNotCanonical(BATCH))
    );
}

// ---- T2: a record id is held by one batch ----------------------------------------------------------------

#[test]
fn an_unnamed_record_held_by_two_batches_of_the_delta_is_a_mismatch() {
    // r2 is in both batches and named by neither reference: it would hit the primary key.
    assert_eq!(
        check_closure(
            &[record(1), record(3)],
            &[part(10, 0xb1, 0, 1, &[1, 2]), part(11, 0xb2, 0, 1, &[2, 3])],
            &LiveClosure::new()
        ),
        Err(Mismatch::RecordInSeveralBatches(rid(2)))
    );
}

#[test]
fn a_record_held_by_a_delta_batch_and_a_different_live_batch_is_a_mismatch() {
    let mut live = LiveClosure::new();
    live.insert_record(rid(0xb9), rid(1));
    assert_eq!(
        check_closure(&[record(1)], &[part(10, 0xb1, 0, 1, &[1])], &live),
        Err(Mismatch::RecordInLiveBatch(rid(1)))
    );
    // Also when the record is not named by the payload.
    let mut live = LiveClosure::new();
    live.insert_record(rid(0xb9), rid(2));
    assert_eq!(
        check_closure(&[record(1)], &[part(10, 0xb1, 0, 1, &[1, 2])], &live),
        Err(Mismatch::RecordInLiveBatch(rid(2)))
    );
    // The same batch re-referenced holds the same records: fine.
    let mut live = LiveClosure::new();
    live.insert_record(rid(0xb1), rid(1));
    live.insert(LogicalRef::Batch(rid(0xb1)), aid(10));
    assert_eq!(
        check_closure(&[record(1)], &[part(10, 0xb1, 0, 1, &[1])], &live),
        Ok(())
    );
    assert_eq!(live.batch_of_record(&rid(1)), Some(rid(0xb1)));
    assert_eq!(live.batch_of_record(&rid(9)), None);
}

#[test]
fn the_same_number_of_parts_with_the_wrong_boundary_is_not_canonical() {
    // Greedy puts four ~1 MB records in part 0 and the fifth in part 1; carry three and two.
    let rows: Vec<RecordRow> = (1..=5).map(|n| rec(n, 1_000_000 - 400)).collect();
    let canonical = canonical_blobs(&batch_of(rows.clone()));
    assert_eq!(canonical.len(), 2);
    let mut skewed = canonical_blobs(&batch_of(rows[..3].to_vec()));
    assert_eq!(skewed.len(), 1);
    skewed.push(records_only(&rows[3..]));
    assert_eq!(
        verdict(&[record(1), record(5)], &canonical),
        Ok(()),
        "the real split is fine"
    );
    assert_eq!(
        verdict(&[record(1), record(5)], &skewed),
        Err(Mismatch::BatchNotCanonical(BATCH))
    );
}

// ---- the parts of a batch may arrive in any order ----------------------------------------------------------

#[test]
fn a_canonical_batch_passes_whatever_order_its_parts_arrive_in() {
    let many = batch_of((1..=250).map(|n| rec(n, 40_000)).collect());
    let blobs = canonical_blobs(&many);
    assert!(blobs.len() >= 3, "{} parts", blobs.len());
    let refs = [record(1), record(250)];
    // Through check_delta: reversed, and rotated.
    let mut reversed = delta_of(&blobs);
    reversed.reverse();
    let mut rotated = delta_of(&blobs);
    rotated.rotate_left(1);
    for owned in [reversed, rotated] {
        let plaintexts: Vec<TransportPlaintext<'_>> = owned
            .iter()
            .map(|(id, metadata, blob)| plaintext(*id, metadata, blob))
            .collect();
        assert_eq!(check_delta(&refs, &plaintexts, &LiveClosure::new()), Ok(()));
    }
    // And through check_batch_canonical directly.
    let count = u32::try_from(blobs.len()).unwrap();
    let mut parts: Vec<(u32, u32, &[u8])> = blobs
        .iter()
        .enumerate()
        .map(|(i, b)| (u32::try_from(i).unwrap(), count, b.as_slice()))
        .collect();
    assert_eq!(check_batch_canonical(&BATCH, &parts), Ok(()));
    parts.reverse();
    assert_eq!(check_batch_canonical(&BATCH, &parts), Ok(()));
}

#[test]
fn parts_that_do_not_join_are_not_canonical_when_checked_directly() {
    let blob = canonical_blobs(&batch_of(vec![rec(1, 20)])).remove(0);
    // A gap, a count that disagrees with the number of parts, and a blob that is not a part.
    assert_eq!(
        check_batch_canonical(&BATCH, &[(1, 2, blob.as_slice())]),
        Err(Mismatch::BatchNotCanonical(BATCH))
    );
    assert_eq!(
        check_batch_canonical(&BATCH, &[(0, 2, blob.as_slice())]),
        Err(Mismatch::BatchNotCanonical(BATCH))
    );
    assert_eq!(
        check_batch_canonical(&BATCH, &[(0, 1, &b"not json"[..])]),
        Err(Mismatch::BatchNotCanonical(BATCH))
    );
    assert_eq!(
        check_batch_canonical(&BATCH, &[]),
        Err(Mismatch::BatchNotCanonical(BATCH))
    );
}
