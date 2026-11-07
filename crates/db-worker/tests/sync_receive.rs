//! Receiving the artifacts of an envelope (`db_worker::sync_receive`, vault-format.md C.4 stage 9
//! and C.9; `personal-cfo-0g528.2`): the stages in order, with real sealed artifacts and a
//! protocol-following fake service. A verdict is decided from the envelope, the artifacts and the
//! live closure; a service fault is never a verdict; nothing is written until every verdict is in.

#[path = "support/artifacts.rs"]
mod artifacts;
mod common;

use artifacts::{key_of, publish_attachment, publish_batch, FakeService, Published};
use db_worker::sync_artifacts::{create_attachment_from_staged, lookup};
use db_worker::sync_receive::{receive_delta, ReceiveError, Received};
use db_worker::DbWorker;
use sync_core::artifact::ArtifactId;
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
