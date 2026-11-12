//! Producing a source batch's artifacts (`db_worker::sync_produce`, personal-cfo-1cltt.7 slice C0):
//! the one pipeline from class-1 rows to receipts, the C.3 address, deterministic sealing (so a
//! retry is an exact retry), and the round trip: what is produced materializes on a replica
//! through the receiver to the same class-1 rows.

#[path = "support/artifacts.rs"]
mod artifacts;
mod common;

use artifacts::{key_of, FakeService};
use db_worker::sync_artifacts::{lookup, ArtifactKind as MappedKind};
use db_worker::sync_produce::{ProduceError, ProducedArtifact};
use db_worker::sync_receive::receive_delta;
use db_worker::DbWorker;
use sync_core::artifact::ArtifactId;
use sync_core::artifact_closure::{LiveClosure, PayloadRef};
use sync_core::artifact_transport::{ArtifactStatus, ArtifactTransport, TransportError};
use sync_core::envelope::{BoundArtifact, BoundManifest};
use sync_core::source_batch::{BatchRow, RecordRow, RunRow, SourceBatch};
use vault_crypto::sync_artifact::first_artifact_id;
use vault_crypto::{
    batch_part_storage_id_raw, derive_kek, generate_dek, generate_salt, unwrap_dek, wrap_dek, Dek,
    Profile,
};

const BATCH: [u8; 16] = [0xb1; 16];

/// A second handle on the same DEK bytes, via the production wrap/unwrap path (`Dek` is
/// deliberately non-`Clone`).
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
    _dir: tempfile::TempDir,
    worker: DbWorker,
}

impl Vault {
    /// A vault and a second handle on its DEK, for computing the expected addresses.
    fn new() -> (Self, Dek) {
        let dir = tempfile::tempdir().unwrap();
        let dek = generate_dek().unwrap();
        let again = duplicate(&dek);
        let worker = DbWorker::open_with_raw_key(dir.path().join("vault.db"), dek).unwrap();
        (Self { _dir: dir, worker }, again)
    }

    /// Write `batch`'s carried rows, as an import would have left them.
    fn put_batch(&self, batch: &SourceBatch) {
        let conn = self.worker.read_connection().unwrap();
        let b = &batch.batch;
        conn.execute(
            "INSERT INTO source_batches (id, source_type, source_name, file_fingerprint,
                 parser_version, summary_json, imported_at, created_at, updated_at,
                 connector_connection_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, ?9)",
            rusqlite::params![
                b.id.as_slice(),
                b.source_type,
                b.source_name,
                b.file_fingerprint,
                b.parser_version,
                b.summary_json,
                b.imported_at,
                b.created_at,
                b.connector_connection_id.as_ref().map(|c| c.as_slice()),
            ],
        )
        .unwrap();
        for r in &batch.runs {
            conn.execute(
                "INSERT INTO parser_runs (id, source_batch_id, parser_name, parser_version,
                     bytes_in, records_out, status, limit_hit, started_at, finished_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                rusqlite::params![
                    r.id.as_slice(),
                    r.source_batch_id.as_slice(),
                    r.parser_name,
                    r.parser_version,
                    r.bytes_in,
                    r.records_out,
                    r.status,
                    r.limit_hit,
                    r.started_at,
                    r.finished_at,
                ],
            )
            .unwrap();
        }
        for r in &batch.records {
            conn.execute(
                "INSERT INTO source_records (id, source_batch_id, external_id, source_hash,
                     normalized_json, parse_confidence_bps, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    r.id.as_slice(),
                    r.source_batch_id.as_slice(),
                    r.external_id,
                    r.source_hash,
                    r.normalized_json,
                    r.parse_confidence_bps,
                    r.created_at,
                ],
            )
            .unwrap();
        }
    }

    /// The carried columns of every batch row, as text, ascending: the rows a replica must equal.
    fn carried_rows(&self) -> Vec<String> {
        let conn = self.worker.read_connection().unwrap();
        let mut out = Vec::new();
        for sql in [
            "SELECT id, source_type, source_name, file_fingerprint, parser_version, summary_json,
                    imported_at, created_at, connector_connection_id
               FROM source_batches ORDER BY id",
            "SELECT id, source_batch_id, parser_name, parser_version, bytes_in, records_out,
                    status, limit_hit, started_at, finished_at
               FROM parser_runs ORDER BY id",
            "SELECT id, source_batch_id, external_id, source_hash, normalized_json,
                    parse_confidence_bps, created_at
               FROM source_records ORDER BY id",
        ] {
            let mut stmt = conn.prepare(sql).unwrap();
            let width = stmt.column_count();
            let rows = stmt
                .query_map([], |r| {
                    let values: Vec<rusqlite::types::Value> =
                        (0..width).map(|i| r.get(i)).collect::<Result<_, _>>()?;
                    Ok(format!("{values:?}"))
                })
                .unwrap();
            out.extend(rows.map(Result::unwrap));
        }
        out
    }
}

/// A batch of `records` records, each carrying `payload_bytes` of JSON, so that `payload_bytes`
/// decides how many parts it splits into (a part is at most 4 MiB).
fn batch_with(records: u8, payload_bytes: usize) -> SourceBatch {
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
            records_out: i64::from(records),
            status: "ok".to_owned(),
            limit_hit: None,
            started_at: "2026-11-01T00:00:00Z".to_owned(),
            finished_at: None,
        }],
        records: (1..=records)
            .map(|n| RecordRow {
                id: [n; 16],
                source_batch_id: BATCH,
                external_id: Some(format!("ext-{n}")),
                source_hash: format!("hash-{n}"),
                normalized_json: format!("{{\"n\":{n},\"pad\":\"{}\"}}", "x".repeat(payload_bytes)),
                parse_confidence_bps: Some(9000),
                created_at: "2026-11-01T00:00:00Z".to_owned(),
            })
            .collect(),
    }
}

fn manifest(produced: &[ProducedArtifact]) -> BoundManifest {
    BoundManifest::new(
        produced
            .iter()
            .map(|p| BoundArtifact {
                id: *p.id.as_bytes(),
                ciphertext_sha256: p.ciphertext_sha256,
            })
            .collect(),
    )
    .unwrap()
}

fn record_refs(batch: &SourceBatch) -> Vec<PayloadRef> {
    batch
        .records
        .iter()
        .map(|r| PayloadRef::Record(r.id))
        .collect()
}

/// The addresses of the parts, from the pipeline's spec: `first_artifact_id(K_e,
/// HMAC(addr_subkey_batch, blob))`, computed here from the split and a second handle on the DEK.
fn expected_ids(
    batch: &SourceBatch,
    dek: &Dek,
    key: &vault_crypto::sync_envelope::EpochKey,
) -> Vec<ArtifactId> {
    batch
        .split()
        .unwrap()
        .iter()
        .map(|part| first_artifact_id(key, &batch_part_storage_id_raw(dek, &part.blob)))
        .collect()
}

/// A service that refuses the `n`th put once (a crash or a lost response between two puts), then
/// behaves.
struct FailsOnce<'a> {
    inner: &'a FakeService,
    fail_at: std::cell::Cell<Option<usize>>,
    puts: std::cell::Cell<usize>,
}

impl ArtifactTransport for FailsOnce<'_> {
    fn put(
        &self,
        vault_id: &[u8; 16],
        key_epoch: u64,
        artifact_id: &ArtifactId,
        container: &[u8],
    ) -> Result<sync_core::artifact::ArtifactReceipt, TransportError> {
        self.puts.set(self.puts.get() + 1);
        if self.fail_at.get() == Some(self.puts.get()) {
            self.fail_at.set(None);
            return Err(TransportError::Unavailable);
        }
        self.inner.put(vault_id, key_epoch, artifact_id, container)
    }

    fn get(
        &self,
        vault_id: &[u8; 16],
        key_epoch: u64,
        artifact_id: &ArtifactId,
    ) -> Result<Vec<u8>, TransportError> {
        self.inner.get(vault_id, key_epoch, artifact_id)
    }

    fn status(
        &self,
        vault_id: &[u8; 16],
        key_epoch: u64,
        ids: &[ArtifactId],
    ) -> Result<Vec<ArtifactStatus>, TransportError> {
        self.inner.status(vault_id, key_epoch, ids)
    }
}

/// The ids are the C.3 addresses, in part order, and every part is mapped and has its receipt.
#[test]
fn every_part_is_published_at_its_c3_address_and_recorded() {
    let (origin, dek) = Vault::new();
    let (service, key) = (FakeService::default(), key_of(1, 0));
    let batch = batch_with(10, 900_000); // about 9 MB of records: three parts
    origin.put_batch(&batch);

    let produced = origin
        .worker
        .produce_batch_artifacts(&service, &key, &BATCH)
        .unwrap();
    assert_eq!(produced.len(), 3, "a batch of about 9 MB is three parts");
    assert_eq!(
        produced.iter().map(|p| p.id).collect::<Vec<_>>(),
        expected_ids(&batch, &dek, &key),
        "each id is first_artifact_id over HMAC(addr_subkey_batch, the part's blob)"
    );
    let conn = origin.worker.read_connection().unwrap();
    for (i, part) in produced.iter().enumerate() {
        assert_eq!(
            part.kind,
            MappedKind::BatchPart {
                index: i as u32,
                count: 3
            }
        );
        assert_eq!(part.logical_id, BATCH);
        assert_eq!(
            part.ciphertext_sha256,
            service.bound_hash(1, &part.id),
            "the hash is the one the service holds"
        );
        let stored = lookup(&conn, 1, part.id.as_bytes()).unwrap().unwrap();
        assert_eq!(stored.mapping.ciphertext_sha256, part.ciphertext_sha256);
        assert_eq!(stored.mapping.logical_id, BATCH);
        let receipt_at: Option<String> = conn
            .query_row(
                "SELECT receipt_at FROM sync_artifacts WHERE sync_artifact_id = ?1",
                [part.id.as_bytes().as_slice()],
                |r| r.get(0),
            )
            .unwrap();
        assert!(receipt_at.is_some(), "the service's receipt is recorded");
    }
}

/// The round trip: what a device produces, another device materializes through the receiver to
/// the same class-1 rows, across a part boundary.
#[test]
fn a_produced_batch_materializes_on_a_replica_to_the_same_class_1_rows() {
    let (origin, _dek) = Vault::new();
    let (replica, _) = Vault::new();
    let (service, key) = (FakeService::default(), key_of(1, 0));
    let batch = batch_with(10, 900_000);
    origin.put_batch(&batch);
    let produced = origin
        .worker
        .produce_batch_artifacts(&service, &key, &BATCH)
        .unwrap();
    assert!(produced.len() > 1);

    let received = receive_delta(
        &replica.worker,
        &service,
        &key,
        &manifest(&produced),
        &record_refs(&batch),
        &LiveClosure::new(),
    )
    .unwrap();
    assert_eq!(received.batches, 1);
    assert_eq!(
        replica.carried_rows(),
        origin.carried_rows(),
        "the replica holds exactly the class-1 rows the origin produced from"
    );
    assert!(!origin.carried_rows().is_empty());
}

/// Producing the same batch again is an exact retry: the same ids, hashes and receipts, and no
/// second mapping; the service never answers `ObjectConflict`.
#[test]
fn producing_a_batch_again_is_an_exact_retry() {
    let (origin, _dek) = Vault::new();
    let (service, key) = (FakeService::default(), key_of(1, 0));
    origin.put_batch(&batch_with(3, 1_000));
    let first = origin
        .worker
        .produce_batch_artifacts(&service, &key, &BATCH)
        .unwrap();
    let again = origin
        .worker
        .produce_batch_artifacts(&service, &key, &BATCH)
        .expect("an exact retry is not an ObjectConflict");
    assert_eq!(first, again);
    let mappings: i64 = origin
        .worker
        .read_connection()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM sync_artifacts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(mappings as usize, first.len());
}

/// A crash between two puts, then a resume: the batch is produced again from the rows, the parts
/// already put are put again with the same bytes, and the receipts equal those of a production that
/// never crashed. An `ObjectConflict` on the resume would fail this; it is never skipped.
#[test]
fn a_crash_between_puts_resumes_with_identical_receipts() {
    let (origin, _dek) = Vault::new();
    let key = key_of(1, 0);
    let batch = batch_with(10, 900_000);
    origin.put_batch(&batch);

    // The reference: the same batch of the same vault (addresses are keyed under this device's DEK)
    // produced without a crash, on its own service.
    let reference_service = FakeService::default();
    let reference = origin
        .worker
        .produce_batch_artifacts(&reference_service, &key, &BATCH)
        .unwrap();
    assert_eq!(reference.len(), 3);

    // The second put does not get through: part 0 is put, part 1 is not.
    let service = FakeService::default();
    let flaky = FailsOnce {
        inner: &service,
        fail_at: std::cell::Cell::new(Some(2)),
        puts: std::cell::Cell::new(0),
    };
    let crashed = origin.worker.produce_batch_artifacts(&flaky, &key, &BATCH);
    assert!(
        matches!(crashed, Err(ProduceError::Publish(_))),
        "{crashed:?}"
    );
    assert!(
        service.bound_hash(1, &reference[0].id) == reference[0].ciphertext_sha256,
        "part 0 was put before the crash"
    );

    // The resume.
    let resumed = origin
        .worker
        .produce_batch_artifacts(&flaky, &key, &BATCH)
        .expect("the resume re-puts the same bytes: never an ObjectConflict");
    assert_eq!(resumed, reference, "identical ids and receipts");
}

/// A batch the vault does not hold is refused by name, and nothing is put or recorded.
#[test]
fn a_batch_the_vault_does_not_hold_produces_nothing() {
    let (origin, _dek) = Vault::new();
    let (service, key) = (FakeService::default(), key_of(1, 0));
    assert!(matches!(
        origin
            .worker
            .produce_batch_artifacts(&service, &key, &[0xee; 16]),
        Err(ProduceError::NotFound)
    ));
    let mappings: i64 = origin
        .worker
        .read_connection()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM sync_artifacts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(mappings, 0);
}
