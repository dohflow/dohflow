//! The receiver's materialization (`db_worker::sync_materialize`, vault-format.md C.9;
//! `personal-cfo-0g528.2`): an attachment's bytes re-encrypted under this device's key with the
//! crypto fields staged for the attach command, and a source batch's rows written outside any
//! command; idempotent, atomic, and failing only for device-local reasons.

#[path = "support/artifacts.rs"]
mod artifacts;
mod common;

use artifacts::{key_of, opened, publish_attachment, FakeService};

use std::fs;
use std::path::PathBuf;

use db_worker::sync_artifacts::{create_attachment_from_staged, lookup, ArtifactKind};
use db_worker::sync_materialize::{BatchPartRef, MaterializeError};
use db_worker::DbWorker;
use sync_core::source_batch::{BatchRow, RecordRow, RunRow, SourceBatch};
use vault_crypto::{derive_kek, generate_dek, generate_salt, unwrap_dek, wrap_dek, Dek, Profile};

const PDF: &[u8] = b"%PDF-1.4\nmaterialized attachment\n%%EOF\n";
const ART: [u8; 32] = [0x51; 32];
const ART2: [u8; 32] = [0x52; 32];
const HASH: [u8; 32] = [0x61; 32];
const ATT: [u8; 16] = [0x07; 16];
const ATT2: [u8; 16] = [0x08; 16];

fn duplicate(dek: &Dek) -> Dek {
    let salt = generate_salt().unwrap();
    let kek = derive_kek(
        b"test-duplicate",
        &salt,
        &Profile::LegacyCompatibility.params(),
    )
    .unwrap();
    unwrap_dek(&kek, &wrap_dek(&kek, dek).unwrap()).unwrap()
}

struct Vault {
    dir: tempfile::TempDir,
    dek: Dek,
    worker: DbWorker,
}

impl Vault {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let dek = generate_dek().unwrap();
        let worker =
            DbWorker::open_with_raw_key(dir.path().join("vault.db"), duplicate(&dek)).unwrap();
        Self { dir, dek, worker }
    }

    fn blobs(&self) -> PathBuf {
        self.dir.path().join("blobs")
    }

    fn blob_path(&self, bytes: &[u8]) -> PathBuf {
        self.blobs()
            .join(vault_crypto::storage_id(&self.dek, bytes).as_str())
    }

    fn count(&self, table: &str) -> i64 {
        self.worker
            .read_connection()
            .unwrap()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }
}

// ---- attachments ---------------------------------------------------------------------------------

/// A published attachment artifact opened as the receiver would, and the hash the service holds.
fn artifact(
    service: &FakeService,
    key: &vault_crypto::sync_envelope::EpochKey,
    address: u8,
    attachment: [u8; 16],
    bytes: &[u8],
) -> (
    vault_crypto::sync_artifact::OpenedArtifact,
    [u8; 32],
    [u8; 32],
) {
    let published = publish_attachment(service, key, address, attachment, bytes, "application/pdf");
    (
        opened(service, key, &published),
        published.hash,
        *published.id.as_bytes(),
    )
}

#[test]
fn an_attachment_is_reencrypted_under_this_device_and_staged_for_the_attach_command() {
    let vault = Vault::new();
    let (service, key) = (FakeService::default(), key_of(1, 0));
    let (artifact, hash, id) = artifact(&service, &key, 0x51, ATT, PDF);
    vault
        .worker
        .materialize_attachment(1, &hash, &artifact)
        .unwrap();

    // The blob is on disk under this device's name and is not the plaintext.
    let on_disk = fs::read(vault.blob_path(PDF)).unwrap();
    assert!(!on_disk.windows(PDF.len()).any(|w| w == PDF));

    // The mapping holds the verified hash and the staged fields; no attachments row exists yet.
    let conn = vault.worker.read_connection().unwrap();
    let stored = lookup(&conn, 1, &id).unwrap().unwrap();
    assert_eq!(stored.mapping.ciphertext_sha256, hash);
    assert_eq!(stored.mapping.kind, ArtifactKind::Attachment);
    assert_eq!(
        stored.mapping.logical_id, ATT,
        "the id comes from the artifact's own metadata"
    );
    let staged = stored.staged.expect("staged for the attach command");
    assert_eq!(staged.plaintext_size, PDF.len() as u64);
    assert_eq!(staged.mime_type.as_deref(), Some("application/pdf"));
    assert_eq!(
        vault.count("attachments"),
        0,
        "materialization never writes the row"
    );

    // The attach command's apply turns the staging into the row, and the bytes read back.
    let attachment = core_ledger::AttachmentId::from_uuid(uuid::Uuid::from_bytes(ATT));
    create_attachment_from_staged(
        &conn,
        1,
        &id,
        attachment,
        Some("one.pdf"),
        "2026-11-08T00:00:00Z",
    )
    .unwrap();
    assert_eq!(vault.worker.read_attachment_bytes(attachment).unwrap(), PDF);
}

#[test]
fn materializing_an_attachment_twice_changes_nothing() {
    let vault = Vault::new();
    let (service, key) = (FakeService::default(), key_of(1, 0));
    let (artifact, hash, id) = artifact(&service, &key, 0x51, ATT, PDF);
    vault
        .worker
        .materialize_attachment(1, &hash, &artifact)
        .unwrap();
    let conn = vault.worker.read_connection().unwrap();
    let blob = fs::read(vault.blob_path(PDF)).unwrap();
    let staged = lookup(&conn, 1, &id).unwrap().unwrap().staged.unwrap();

    // A replay, with a newer verified hash: the blob and the staged key are the same.
    vault
        .worker
        .materialize_attachment(1, &[0x62; 32], &artifact)
        .unwrap();
    assert_eq!(fs::read(vault.blob_path(PDF)).unwrap(), blob);
    let again = lookup(&conn, 1, &id).unwrap().unwrap();
    assert_eq!(again.staged, Some(staged));
    assert_eq!(again.mapping.ciphertext_sha256, [0x62; 32]);

    // After the apply consumed the staging, a replay does not bring it back.
    let attachment = core_ledger::AttachmentId::from_uuid(uuid::Uuid::from_bytes(ATT));
    create_attachment_from_staged(&conn, 1, &id, attachment, None, "t").unwrap();
    vault
        .worker
        .materialize_attachment(1, &hash, &artifact)
        .unwrap();
    assert_eq!(lookup(&conn, 1, &id).unwrap().unwrap().staged, None);
    assert_eq!(vault.worker.read_attachment_bytes(attachment).unwrap(), PDF);
}

#[test]
fn identical_bytes_under_another_attachment_are_refused_and_nothing_is_written() {
    let (service, key) = (FakeService::default(), key_of(1, 0));
    let vault = Vault::new();
    // An attachment this device already stores.
    vault.worker.import_attachment(PDF, None, None).unwrap();
    let before = fs::read(vault.blob_path(PDF)).unwrap();
    let (artifact, hash, id) = artifact(&service, &key, 0x51, ATT, PDF);
    assert!(matches!(
        vault.worker.materialize_attachment(1, &hash, &artifact),
        Err(MaterializeError::BlobHeld)
    ));
    assert_eq!(
        fs::read(vault.blob_path(PDF)).unwrap(),
        before,
        "the blob is not overwritten"
    );
    assert_eq!(
        lookup(&vault.worker.read_connection().unwrap(), 1, &id).unwrap(),
        None
    );

    // The same when another artifact has the bytes staged.
    let other = Vault::new();
    let (first, first_hash, _) = artifact_pair(&service, &key, 0x71, ATT);
    other
        .worker
        .materialize_attachment(1, &first_hash, &first)
        .unwrap();
    let (second, second_hash, second_id) = artifact_pair(&service, &key, 0x72, ATT2);
    assert!(matches!(
        other
            .worker
            .materialize_attachment(1, &second_hash, &second),
        Err(MaterializeError::BlobHeld)
    ));
    assert_eq!(
        lookup(&other.worker.read_connection().unwrap(), 1, &second_id).unwrap(),
        None
    );
}

/// Two attachments with the same bytes published under two addresses.
fn artifact_pair(
    service: &FakeService,
    key: &vault_crypto::sync_envelope::EpochKey,
    address: u8,
    attachment: [u8; 16],
) -> (
    vault_crypto::sync_artifact::OpenedArtifact,
    [u8; 32],
    [u8; 32],
) {
    artifact(service, key, address, attachment, PDF)
}

#[test]
fn a_failed_record_removes_the_blob_it_wrote_and_leaves_no_mapping() {
    let vault = Vault::new();
    let (service, key) = (FakeService::default(), key_of(1, 0));
    let (artifact, hash, _) = artifact(&service, &key, 0x51, ATT, PDF);
    vault
        .worker
        .read_connection()
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER refuse_mapping BEFORE INSERT ON sync_artifacts
             BEGIN SELECT RAISE(ABORT, 'refused'); END;",
        )
        .unwrap();
    let result = vault.worker.materialize_attachment(1, &hash, &artifact);
    assert!(matches!(
        result,
        Err(MaterializeError::Mapping(_) | MaterializeError::Local(_))
    ));
    assert!(
        !vault.blob_path(PDF).exists(),
        "the blob this call wrote is removed"
    );
    assert_eq!(vault.count("sync_artifacts"), 0);

    // With the fault gone it succeeds.
    vault
        .worker
        .read_connection()
        .unwrap()
        .execute_batch("DROP TRIGGER refuse_mapping;")
        .unwrap();
    vault
        .worker
        .materialize_attachment(1, &hash, &artifact)
        .unwrap();
    assert!(vault.blob_path(PDF).exists());
}

#[test]
fn an_orphan_blob_of_an_earlier_attempt_is_replaced() {
    let vault = Vault::new();
    let (service, key) = (FakeService::default(), key_of(1, 0));
    let (artifact, hash, id) = artifact(&service, &key, 0x51, ATT, PDF);
    fs::create_dir_all(vault.blobs()).unwrap();
    fs::write(vault.blob_path(PDF), b"left behind by an attempt that died").unwrap();
    vault
        .worker
        .materialize_attachment(1, &hash, &artifact)
        .unwrap();
    let conn = vault.worker.read_connection().unwrap();
    let attachment = core_ledger::AttachmentId::from_uuid(uuid::Uuid::from_bytes(ATT));
    create_attachment_from_staged(&conn, 1, &id, attachment, None, "t").unwrap();
    assert_eq!(
        vault.worker.read_attachment_bytes(attachment).unwrap(),
        PDF,
        "the staged key opens the replaced blob"
    );
}

// ---- source batches ------------------------------------------------------------------------------

fn batch() -> SourceBatch {
    let id = [0xb1; 16];
    SourceBatch {
        batch: BatchRow {
            id,
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
            source_batch_id: id,
            parser_name: "csv".to_owned(),
            parser_version: "1".to_owned(),
            bytes_in: 10,
            records_out: 2,
            status: "ok".to_owned(),
            limit_hit: None,
            started_at: "2026-11-01T00:00:00Z".to_owned(),
            finished_at: Some("2026-11-01T00:00:01Z".to_owned()),
        }],
        records: (1..=2_u8)
            .map(|n| RecordRow {
                id: [n; 16],
                source_batch_id: id,
                external_id: Some(format!("ext-{n}")),
                source_hash: format!("hash-{n}"),
                normalized_json: format!("{{\"n\":{n}}}"),
                parse_confidence_bps: Some(9000),
                created_at: "2026-11-01T00:00:00Z".to_owned(),
            })
            .collect(),
    }
}

fn parts() -> [BatchPartRef; 2] {
    [
        BatchPartRef {
            artifact_id: ART,
            ciphertext_sha256: HASH,
            index: 0,
            count: 2,
        },
        BatchPartRef {
            artifact_id: ART2,
            ciphertext_sha256: [0x62; 32],
            index: 1,
            count: 2,
        },
    ]
}

#[test]
fn a_batch_is_written_with_the_local_columns_of_a_received_batch_and_its_parts_are_mapped() {
    let vault = Vault::new();
    vault
        .worker
        .materialize_batch(1, &parts(), &batch())
        .unwrap();

    let conn = vault.worker.read_connection().unwrap();
    assert_eq!(vault.count("source_batches"), 1);
    assert_eq!(vault.count("parser_runs"), 1);
    assert_eq!(vault.count("source_records"), 2);
    let (status, staged, committed, skipped, created, updated): (
        String,
        i64,
        i64,
        i64,
        String,
        String,
    ) = conn
        .query_row(
            "SELECT status, staged_count, committed_count, skipped_count, created_at, updated_at
               FROM source_batches WHERE id = ?1",
            [[0xb1_u8; 16].as_slice()],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(status, "committed");
    assert_eq!((staged, committed, skipped), (0, 0, 0));
    assert_eq!(updated, created, "updated_at equals created_at");

    for (part, hash) in parts().iter().zip([HASH, [0x62; 32]]) {
        let stored = lookup(&conn, 1, &part.artifact_id).unwrap().unwrap();
        assert_eq!(
            stored.mapping.kind,
            ArtifactKind::BatchPart {
                index: part.index,
                count: 2
            }
        );
        assert_eq!(stored.mapping.logical_id, [0xb1; 16]);
        assert_eq!(stored.mapping.ciphertext_sha256, hash);
        assert_eq!(stored.staged, None);
    }
}

#[test]
fn materializing_a_batch_twice_changes_nothing() {
    let vault = Vault::new();
    vault
        .worker
        .materialize_batch(1, &parts(), &batch())
        .unwrap();
    vault
        .worker
        .materialize_batch(1, &parts(), &batch())
        .unwrap();
    assert_eq!(vault.count("source_batches"), 1);
    assert_eq!(vault.count("parser_runs"), 1);
    assert_eq!(vault.count("source_records"), 2);
    assert_eq!(vault.count("sync_artifacts"), 2);
}

#[test]
fn a_row_that_differs_from_the_one_in_the_vault_is_refused_and_the_whole_batch_is_rolled_back() {
    let vault = Vault::new();
    // A record with the same id but other content is already in the vault.
    let mut existing = batch();
    existing.records[1].normalized_json = "{\"n\":99}".to_owned();
    existing.records.truncate(2);
    let mut one = batch();
    one.batch.id = [0xb2; 16];
    one.runs.clear();
    one.records = vec![RecordRow {
        id: [2; 16],
        source_batch_id: [0xb2; 16],
        ..existing.records[1].clone()
    }];
    vault.worker.materialize_batch(1, &[], &one).unwrap();

    // Materializing the batch that carries a different row under that id writes nothing at all,
    // not even the rows before the conflict.
    let rows_before = (
        vault.count("source_batches"),
        vault.count("parser_runs"),
        vault.count("source_records"),
        vault.count("sync_artifacts"),
    );
    assert!(matches!(
        vault.worker.materialize_batch(1, &parts(), &batch()),
        Err(MaterializeError::RowsDiffer)
    ));
    assert_eq!(
        rows_before,
        (
            vault.count("source_batches"),
            vault.count("parser_runs"),
            vault.count("source_records"),
            vault.count("sync_artifacts"),
        )
    );

    // A batch id already present with other carried columns is refused the same way.
    let mut changed = batch();
    changed.batch.id = [0xb2; 16];
    changed.batch.source_name = Some("another.csv".to_owned());
    changed.runs.clear();
    changed.records.clear();
    assert!(matches!(
        vault.worker.materialize_batch(1, &[], &changed),
        Err(MaterializeError::RowsDiffer)
    ));
}
