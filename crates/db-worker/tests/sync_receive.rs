//! Receiving the artifacts of an envelope (`db_worker::sync_receive`, vault-format.md C.4 stage 9
//! and C.9; `personal-cfo-0g528.2`): the stages in order, with real sealed artifacts and a
//! protocol-following fake service. A verdict is decided from the envelope, the artifacts and the
//! live closure; a service fault is never a verdict; nothing is written until every verdict is in.

#[path = "support/artifacts.rs"]
mod artifacts;
mod common;

use artifacts::{
    key_of, part_metadata, publish_attachment, publish_batch, publish_raw, FakeService, Published,
};
use db_worker::sync_artifacts::{create_attachment_from_staged, lookup};
use db_worker::sync_receive::{receive_delta, ReceiveError, Received};
use db_worker::DbWorker;
use sync_core::artifact::{ArtifactId, ArtifactKind};
use sync_core::artifact_closure::{LiveClosure, LogicalRef, Mismatch, PayloadRef};
use sync_core::envelope::{BoundArtifact, BoundManifest};
use sync_core::source_batch::{BatchRow, RecordRow, RunRow, SourceBatch};
use vault_crypto::generate_dek;
use vault_crypto::sync_artifact_flow::FetchError;

const PDF: &[u8] = b"%PDF-1.4\nreceived attachment\n%%EOF\n";
const ATT: [u8; 16] = [0x07; 16];
const BATCH: [u8; 16] = [0xb1; 16];
const RECORD: [u8; 16] = [0x01; 16];

struct Vault {
    dir: tempfile::TempDir,
    worker: DbWorker,
}

impl Vault {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let worker =
            DbWorker::open_with_raw_key(dir.path().join("vault.db"), generate_dek().unwrap())
                .unwrap();
        Self { dir, worker }
    }

    fn count(&self, table: &str) -> i64 {
        self.worker
            .read_connection()
            .unwrap()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    /// Everything a receive writes: rows, mappings and blob files.
    fn footprint(&self) -> (i64, i64, i64, i64, usize) {
        let blobs = std::fs::read_dir(self.dir.path().join("blobs"))
            .map(|entries| entries.count())
            .unwrap_or(0);
        (
            self.count("source_batches"),
            self.count("source_records"),
            self.count("parser_runs"),
            self.count("sync_artifacts"),
            blobs,
        )
    }
}

fn batch() -> SourceBatch {
    SourceBatch {
        batch: BatchRow {
            id: BATCH,
            source_type: "csv".to_owned(),
            source_name: Some("statement.csv".to_owned()),
            file_fingerprint: Some("fp".to_owned()),
            parser_version: Some("1".to_owned()),
            summary_json: Some("{}".to_owned()),
            imported_at: Some("2026-11-01T00:00:00Z".to_owned()),
            created_at: "2026-11-01T00:00:00Z".to_owned(),
            connector_connection_id: None,
        },
        runs: vec![RunRow {
            id: [0xc1; 16],
            source_batch_id: BATCH,
            parser_name: "csv".to_owned(),
            parser_version: "1".to_owned(),
            bytes_in: 10,
            records_out: 1,
            status: "ok".to_owned(),
            limit_hit: None,
            started_at: "2026-11-01T00:00:00Z".to_owned(),
            finished_at: None,
        }],
        records: vec![RecordRow {
            id: RECORD,
            source_batch_id: BATCH,
            external_id: Some("ext-1".to_owned()),
            source_hash: "hash-1".to_owned(),
            normalized_json: "{\"n\":1}".to_owned(),
            parse_confidence_bps: Some(9000),
            created_at: "2026-11-01T00:00:00Z".to_owned(),
        }],
    }
}

fn manifest(published: &[Published]) -> BoundManifest {
    BoundManifest::new(
        published
            .iter()
            .map(|p| BoundArtifact {
                id: *p.id.as_bytes(),
                ciphertext_sha256: p.hash,
            })
            .collect(),
    )
    .unwrap()
}

fn refs() -> [PayloadRef; 2] {
    [PayloadRef::Attachment(ATT), PayloadRef::Record(RECORD)]
}

#[test]
fn an_attachment_and_a_batch_are_received_and_the_attachment_reads_back_after_the_attach() {
    let vault = Vault::new();
    let (service, key) = (FakeService::default(), key_of(1, 0));
    let attachment = publish_attachment(&service, &key, 0x51, ATT, PDF, "application/pdf");
    let parts = publish_batch(&service, &key, 0x60, &batch());
    let delta = manifest(&[vec![attachment], parts.clone()].concat());

    let received = receive_delta(
        &vault.worker,
        &service,
        &key,
        &delta,
        &refs(),
        &LiveClosure::new(),
    )
    .unwrap();
    assert_eq!(
        received,
        Received {
            attachments: 1,
            batches: 1,
            discarded_imports: 0,
        }
    );
    assert_eq!(vault.count("source_batches"), 1);
    assert_eq!(vault.count("source_records"), 1);
    assert_eq!(
        vault.count("attachments"),
        0,
        "the attach command's apply has not run"
    );

    // Every artifact is mapped with the hash it was received under.
    let conn = vault.worker.read_connection().unwrap();
    for published in std::iter::once(&attachment).chain(&parts) {
        let stored = lookup(&conn, 1, published.id.as_bytes()).unwrap().unwrap();
        assert_eq!(stored.mapping.ciphertext_sha256, published.hash);
    }
    // And the attachment becomes a row and reads back.
    let id = core_ledger::AttachmentId::from_uuid(uuid::Uuid::from_bytes(ATT));
    create_attachment_from_staged(&conn, 1, attachment.id.as_bytes(), id, None, "t").unwrap();
    assert_eq!(vault.worker.read_attachment_bytes(id).unwrap(), PDF);

    // A replay of the same envelope changes nothing.
    let before = vault.footprint();
    receive_delta(
        &vault.worker,
        &service,
        &key,
        &delta,
        &refs(),
        &LiveClosure::new(),
    )
    .unwrap();
    assert_eq!(vault.footprint(), before);
}

#[test]
fn the_live_hash_rule_is_decided_before_any_request_is_made() {
    let vault = Vault::new();
    let (service, key) = (FakeService::default(), key_of(1, 0));
    let attachment = publish_attachment(&service, &key, 0x51, ATT, PDF, "application/pdf");
    // The live closure at the preceding position holds this id with another hash.
    let mut live = LiveClosure::new();
    live.insert(LogicalRef::Attachment(ATT), attachment.id, [0x99; 32]);
    let gets_before = service.gets.get();

    let result = receive_delta(
        &vault.worker,
        &service,
        &key,
        &manifest(&[attachment]),
        &[PayloadRef::Attachment(ATT)],
        &live,
    );
    assert!(matches!(
        result,
        Err(ReceiveError::Mismatch(Mismatch::HashDiffersFromLive(id))) if id == attachment.id
    ));
    assert_eq!(service.gets.get(), gets_before, "no request was made");
    assert_eq!(vault.footprint(), (0, 0, 0, 0, 0));

    // Even when the service has lost the object, the verdict is the same: the service cannot delay
    // or choose it.
    service.lose(1, &attachment.id);
    assert!(matches!(
        receive_delta(
            &vault.worker,
            &service,
            &key,
            &manifest(&[attachment]),
            &[PayloadRef::Attachment(ATT)],
            &live
        ),
        Err(ReceiveError::Mismatch(Mismatch::HashDiffersFromLive(_)))
    ));
}

#[test]
fn a_service_fault_makes_the_delta_not_determinable_and_nothing_is_written() {
    let vault = Vault::new();
    let (service, key) = (FakeService::default(), key_of(1, 0));
    let attachment = publish_attachment(&service, &key, 0x51, ATT, PDF, "application/pdf");
    let parts = publish_batch(&service, &key, 0x60, &batch());
    let delta = manifest(&[vec![attachment], parts.clone()].concat());
    let receive = |service: &FakeService| {
        receive_delta(
            &vault.worker,
            service,
            &key,
            &delta,
            &refs(),
            &LiveClosure::new(),
        )
    };

    // The service serves other bytes than the envelope bound for one artifact: the others are fine
    // and are not materialized either.
    service.corrupt(1, &parts[0].id);
    assert!(matches!(
        receive(&service),
        Err(ReceiveError::NotDeterminable(FetchError::HashMismatch))
    ));
    assert_eq!(vault.footprint(), (0, 0, 0, 0, 0));

    // It has lost one.
    let service = FakeService::default();
    let attachment = publish_attachment(&service, &key, 0x51, ATT, PDF, "application/pdf");
    let parts = publish_batch(&service, &key, 0x60, &batch());
    service.lose(1, &attachment.id);
    let delta = manifest(&[vec![attachment], parts].concat());
    assert!(matches!(
        receive_delta(
            &vault.worker,
            &service,
            &key,
            &delta,
            &refs(),
            &LiveClosure::new()
        ),
        Err(ReceiveError::NotDeterminable(FetchError::Missing))
    ));
    assert_eq!(vault.footprint(), (0, 0, 0, 0, 0));
}

#[test]
fn bytes_that_hash_to_the_bound_value_and_do_not_open_are_the_producers_fault() {
    let vault = Vault::new();
    let (service, key) = (FakeService::default(), key_of(1, 0));
    // An artifact sealed under another epoch key is what the receiver's key cannot open; its hash is
    // what a member that bound it would write in the envelope.
    let foreign = publish_attachment(
        &service,
        &key_of(1, 0x80),
        0x51,
        ATT,
        PDF,
        "application/pdf",
    );
    let result = receive_delta(
        &vault.worker,
        &service,
        &key,
        &manifest(&[foreign]),
        &[PayloadRef::Attachment(ATT)],
        &LiveClosure::new(),
    );
    assert!(matches!(
        result,
        Err(ReceiveError::Mismatch(Mismatch::ArtifactInvalid(id))) if id == foreign.id
    ));
    assert_eq!(vault.footprint(), (0, 0, 0, 0, 0));

    // But a service fault on another artifact of the same delta wins: the verdict never depends on
    // which request happened to fail first.
    let other = publish_attachment(&service, &key, 0x52, [0x08; 16], PDF, "application/pdf");
    service.lose(1, &other.id);
    let result = receive_delta(
        &vault.worker,
        &service,
        &key,
        &manifest(&[foreign, other]),
        &[
            PayloadRef::Attachment(ATT),
            PayloadRef::Attachment([0x08; 16]),
        ],
        &LiveClosure::new(),
    );
    assert!(matches!(
        result,
        Err(ReceiveError::NotDeterminable(FetchError::Missing))
    ));
}

#[test]
fn a_delta_that_does_not_match_the_payload_is_invalid_and_nothing_is_written() {
    let vault = Vault::new();
    let (service, key) = (FakeService::default(), key_of(1, 0));
    let attachment = publish_attachment(&service, &key, 0x51, ATT, PDF, "application/pdf");

    // An artifact that no reference binds.
    let result = receive_delta(
        &vault.worker,
        &service,
        &key,
        &manifest(&[attachment]),
        &[],
        &LiveClosure::new(),
    );
    assert!(matches!(
        result,
        Err(ReceiveError::Mismatch(Mismatch::UnreferencedArtifact(id))) if id == attachment.id
    ));
    // A reference with no artifact.
    let result = receive_delta(
        &vault.worker,
        &service,
        &key,
        &manifest(&[]),
        &[PayloadRef::Attachment(ATT)],
        &LiveClosure::new(),
    );
    assert!(matches!(
        result,
        Err(ReceiveError::Mismatch(Mismatch::UnresolvedReference(_)))
    ));
    // A logical object of the live closure bound to another id.
    let mut live = LiveClosure::new();
    live.insert(
        LogicalRef::Attachment(ATT),
        ArtifactId::from_derived([0x77; 32]),
        [0x78; 32],
    );
    let result = receive_delta(
        &vault.worker,
        &service,
        &key,
        &manifest(&[attachment]),
        &[PayloadRef::Attachment(ATT)],
        &live,
    );
    assert!(matches!(
        result,
        Err(ReceiveError::Mismatch(Mismatch::Rebinding(LogicalRef::Attachment(a)))) if a == ATT
    ));
    assert_eq!(vault.footprint(), (0, 0, 0, 0, 0));
}

#[test]
fn a_local_failure_after_the_verdicts_is_materialization_not_a_verdict() {
    let vault = Vault::new();
    let (service, key) = (FakeService::default(), key_of(1, 0));
    let attachment = publish_attachment(&service, &key, 0x51, ATT, PDF, "application/pdf");
    vault
        .worker
        .read_connection()
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER refuse_mapping BEFORE INSERT ON sync_artifacts
             BEGIN SELECT RAISE(ABORT, 'refused'); END;",
        )
        .unwrap();
    let result = receive_delta(
        &vault.worker,
        &service,
        &key,
        &manifest(&[attachment]),
        &[PayloadRef::Attachment(ATT)],
        &LiveClosure::new(),
    );
    assert!(
        matches!(result, Err(ReceiveError::Materialization(_))),
        "a disk or database fault is the device's, never an invalid envelope"
    );
    assert_eq!(
        vault.footprint(),
        (0, 0, 0, 0, 0),
        "and it leaves nothing behind"
    );
}

// ---- stage 9(b): every refusal class decides from the artifacts and writes nothing -----------------

/// A batch with the records `[n; 16]` for each `n` in `records`, split canonically into one part.
fn batch_of(id: [u8; 16], records: &[u8]) -> SourceBatch {
    let mut next = batch();
    next.batch.id = id;
    next.runs[0].source_batch_id = id;
    next.records = records
        .iter()
        .map(|n| RecordRow {
            id: [*n; 16],
            source_batch_id: id,
            external_id: Some(format!("ext-{n}")),
            source_hash: format!("hash-{n}"),
            normalized_json: format!("{{\"n\":{n}}}"),
            ..batch().records[0].clone()
        })
        .collect();
    next
}

fn to_json(value: &serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(value).unwrap()
}

/// Part 0 of `batch` as JSON, with only the records at the given positions.
fn part0_keeping(batch: &SourceBatch, keep: &[usize]) -> Vec<u8> {
    let mut part: serde_json::Value =
        serde_json::from_slice(&batch.split().unwrap()[0].blob).unwrap();
    let all = part["records"].as_array().unwrap().clone();
    part["records"] = serde_json::Value::Array(keep.iter().map(|i| all[*i].clone()).collect());
    to_json(&part)
}

/// A later part holding the records of `batch` at the given positions.
fn later_part(batch: &SourceBatch, keep: &[usize]) -> Vec<u8> {
    let part: serde_json::Value = serde_json::from_slice(&batch.split().unwrap()[0].blob).unwrap();
    let all = part["records"].as_array().unwrap().clone();
    to_json(
        &serde_json::json!({ "records": keep.iter().map(|i| all[*i].clone()).collect::<Vec<_>>() }),
    )
}

#[test]
fn every_stage_9b_refusal_class_is_decided_from_the_artifacts_and_writes_nothing() {
    let key = key_of(1, 0);
    let part = |service: &FakeService,
                address: u8,
                batch: [u8; 16],
                index: u32,
                count: u32,
                blob: &[u8]| {
        publish_raw(
            service,
            &key,
            address,
            ArtifactKind::SourceBatchPart,
            &part_metadata(batch, index, count),
            blob,
        )
    };
    let run = |name: &str,
               service: &FakeService,
               delta: &[Published],
               refs: &[PayloadRef],
               live: &LiveClosure,
               expected: &dyn Fn(&Mismatch) -> bool| {
        let vault = Vault::new();
        match receive_delta(&vault.worker, service, &key, &manifest(delta), refs, live) {
            Err(ReceiveError::Mismatch(found)) => assert!(expected(&found), "{name}: {found:?}"),
            other => panic!("{name}: expected a stage-9(b) verdict, got {other:?}"),
        }
        assert_eq!(
            vault.footprint(),
            (0, 0, 0, 0, 0),
            "{name}: nothing written"
        );
    };
    let two = batch_of(BATCH, &[1, 2]);
    let none = LiveClosure::new();
    let records = [PayloadRef::Record([1; 16]), PayloadRef::Record([2; 16])];

    // A batch that is not its canonical split: two records that belong in one part are in two.
    let s = FakeService::default();
    let delta = [
        part(&s, 0x10, BATCH, 0, 2, &part0_keeping(&two, &[0])),
        part(&s, 0x11, BATCH, 1, 2, &later_part(&two, &[1])),
    ];
    run(
        "a non-canonical split",
        &s,
        &delta,
        &records,
        &none,
        &|m| matches!(m, Mismatch::BatchNotCanonical(b) if *b == BATCH),
    );

    // A record carried by two parts of one batch.
    let s = FakeService::default();
    let delta = [
        part(&s, 0x20, BATCH, 0, 2, &part0_keeping(&two, &[0, 1])),
        part(&s, 0x21, BATCH, 1, 2, &later_part(&two, &[1])),
    ];
    run(
        "a record in two parts",
        &s,
        &delta,
        &records,
        &none,
        &|m| matches!(m, Mismatch::RecordDuplicated(r) if *r == [2; 16]),
    );

    // An authenticated part that does not decode as batch rows.
    let s = FakeService::default();
    let bad = part(&s, 0x31, BATCH, 1, 2, b"this is not a batch part");
    let delta = [
        part(&s, 0x30, BATCH, 0, 2, &part0_keeping(&two, &[0, 1])),
        bad,
    ];
    run(
        "an undecodable part",
        &s,
        &delta,
        &records,
        &none,
        &|m| matches!(m, Mismatch::PartUndecodable(id) if *id == bad.id),
    );

    // One source record held by two batches of the delta.
    let s = FakeService::default();
    let first = batch_of([0xb1; 16], &[1]);
    let second = batch_of([0xb2; 16], &[1]);
    let delta = [
        part(&s, 0x40, [0xb1; 16], 0, 1, &part0_keeping(&first, &[0])),
        part(&s, 0x41, [0xb2; 16], 0, 1, &part0_keeping(&second, &[0])),
    ];
    run(
        "a record in two batches",
        &s,
        &delta,
        &[PayloadRef::Record([1; 16])],
        &none,
        &|m| matches!(m, Mismatch::RecordInSeveralBatches(r) if *r == [1; 16]),
    );

    // The same record held by a batch of the delta and by a different batch of the live closure.
    let s = FakeService::default();
    let one = batch_of(BATCH, &[1]);
    let delta = [part(&s, 0x50, BATCH, 0, 1, &part0_keeping(&one, &[0]))];
    let mut live = LiveClosure::new();
    live.insert_record([0xee; 16], [1; 16]);
    run(
        "a record in a different live batch",
        &s,
        &delta,
        &[PayloadRef::Record([1; 16])],
        &live,
        &|m| matches!(m, Mismatch::RecordInLiveBatch(r) if *r == [1; 16]),
    );

    // The parts of a batch: a gap, a repeated index, and parts that disagree on the count.
    let s = FakeService::default();
    let delta = [part(&s, 0x60, BATCH, 0, 2, &part0_keeping(&two, &[0, 1]))];
    run(
        "a missing part",
        &s,
        &delta,
        &records,
        &none,
        &|m| matches!(m, Mismatch::PartMissing(b) if *b == BATCH),
    );
    let s = FakeService::default();
    let delta = [
        part(&s, 0x70, BATCH, 0, 2, &part0_keeping(&two, &[0])),
        part(&s, 0x71, BATCH, 0, 2, &part0_keeping(&two, &[1])),
    ];
    run(
        "a repeated part index",
        &s,
        &delta,
        &records,
        &none,
        &|m| matches!(m, Mismatch::PartDuplicated(b) if *b == BATCH),
    );
    let s = FakeService::default();
    let delta = [
        part(&s, 0x80, BATCH, 0, 2, &part0_keeping(&two, &[0, 1])),
        part(&s, 0x81, BATCH, 1, 3, &later_part(&two, &[1])),
    ];
    run(
        "parts that disagree on the count",
        &s,
        &delta,
        &records,
        &none,
        &|m| matches!(m, Mismatch::PartsDisagree(b) if *b == BATCH),
    );

    // A payload that names a record no batch of the delta holds.
    let s = FakeService::default();
    let delta = [part(&s, 0x90, BATCH, 0, 1, &part0_keeping(&one, &[0]))];
    run(
        "a named record in no batch",
        &s,
        &delta,
        &[PayloadRef::Record([1; 16]), PayloadRef::Record([0x55; 16])],
        &none,
        &|m| matches!(m, Mismatch::RecordInNoBatch(r) if *r == [0x55; 16]),
    );

    // An attachment whose metadata names no attachment id, and one attachment bound twice.
    let s = FakeService::default();
    let unnamed = publish_raw(
        &s,
        &key,
        0xa0,
        ArtifactKind::Attachment,
        br#"{"mime_type":"text/plain"}"#,
        PDF,
    );
    run(
        "an attachment with no id",
        &s,
        &[unnamed],
        &[],
        &none,
        &|m| matches!(m, Mismatch::BadBinding(id) if *id == unnamed.id),
    );
    let s = FakeService::default();
    let delta = [
        publish_attachment(&s, &key, 0xb0, ATT, PDF, "application/pdf"),
        publish_attachment(&s, &key, 0xb1, ATT, b"other bytes", "application/pdf"),
    ];
    run(
        "an attachment bound twice",
        &s,
        &delta,
        &[PayloadRef::Attachment(ATT)],
        &none,
        &|m| matches!(m, Mismatch::AttachmentRebound(a) if *a == ATT),
    );
}
