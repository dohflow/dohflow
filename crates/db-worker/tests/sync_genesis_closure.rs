//! A genesis that carries class-1b artifacts (`db_worker::sync_enable`, `db_worker::sync_produce`;
//! personal-cfo-1cltt.7 slice C): every attachment's bytes and every batch a provenance link reaches
//! are produced and put BEFORE the snapshot that names them is sent, the export carries the closure
//! in order, a missing receipt is refused by the service and pins nothing, a resume puts the closure
//! again identically, and a vault that changes under the production is never exported with an
//! incomplete closure.

#![allow(dead_code)]

#[path = "support/artifacts.rs"]
mod artifacts;
mod common;
#[path = "../../vault-crypto/tests/common/snapshot_service.rs"]
mod snapshot_service;

use std::cell::{Cell, RefCell};
use std::sync::Arc;

use db_worker::sync_enable::{enable_sync, EnableError, EnableOutcome, EnablingDevice};
use db_worker::sync_genesis::{self, GenesisState};
use db_worker::sync_produce::ProduceError;
use db_worker::DbWorker;
use platform_mock::{MockSecretStore, MockWitness};
use snapshot_service::{Credential, FakeSnapshotService, Fault};
use sync_core::artifact::{ArtifactId, ArtifactReceipt};
use sync_core::artifact_transport::{ArtifactStatus, ArtifactTransport, TransportError};
use sync_core::snapshot_service::ServiceError;
use sync_core::source_batch::{BatchRow, RecordRow, RunRow, SourceBatch};
use sync_core::status::SyncStatus;
use vault_crypto::sync_snapshot_flow::PublishError;

const DEVICE: EnablingDevice = EnablingDevice {
    device_id: [0xa1; 16],
    issued_at: 1_700_000_000,
};

const PDF: &[u8] = b"%PDF-1.4\nthe first attachment\n%%EOF\n";
const PNG: &[u8] = b"\x89PNG\r\n\x1a\nthe second attachment";

/// What the artifact service tells the snapshot service: the receipts it holds. A real service is
/// one service; the two fakes are separate, so this is the bridge. `tell` false is a service whose
/// receipts the snapshot service does not see (a missing receipt at `installEpoch`).
struct Bridged<'a> {
    artifacts: &'a artifacts::FakeService,
    snapshots: &'a FakeSnapshotService,
    tell: bool,
    puts: Cell<usize>,
    /// Refuse the put with this number (1-based) once.
    fail_put: Cell<Option<usize>>,
    /// Run once, right after the first put of every production (what arrives under it).
    on_first_put: RefCell<Option<Box<dyn FnMut() + 'a>>>,
    /// Every container this device ever sent, by address, whatever the service answered.
    seen: RefCell<Vec<(ArtifactId, Vec<u8>)>>,
}

impl<'a> Bridged<'a> {
    fn new(artifacts: &'a artifacts::FakeService, snapshots: &'a FakeSnapshotService) -> Self {
        Self {
            artifacts,
            snapshots,
            tell: true,
            puts: Cell::new(0),
            fail_put: Cell::new(None),
            on_first_put: RefCell::new(None),
            seen: RefCell::new(Vec::new()),
        }
    }

    /// How many addresses were sent two different containers (C.9: never one).
    fn addresses_with_two_containers(&self) -> usize {
        let mut by_id: std::collections::BTreeMap<[u8; 32], Vec<&Vec<u8>>> = Default::default();
        let seen = self.seen.borrow();
        for (id, container) in seen.iter() {
            let held = by_id.entry(*id.as_bytes()).or_default();
            if !held.contains(&container) {
                held.push(container);
            }
        }
        by_id.values().filter(|held| held.len() > 1).count()
    }
}

impl ArtifactTransport for Bridged<'_> {
    fn put(
        &self,
        vault_id: &[u8; 16],
        key_epoch: u64,
        artifact_id: &ArtifactId,
        container: &[u8],
    ) -> Result<ArtifactReceipt, TransportError> {
        self.puts.set(self.puts.get() + 1);
        self.seen
            .borrow_mut()
            .push((*artifact_id, container.to_vec()));
        if self.fail_put.get() == Some(self.puts.get()) {
            self.fail_put.set(None);
            return Err(TransportError::Unavailable);
        }
        let receipt = self
            .artifacts
            .put(vault_id, key_epoch, artifact_id, container)?;
        if self.tell {
            self.snapshots
                .receipt_with_hash(*artifact_id, receipt.ciphertext_sha256);
        }
        if let Some(hook) = self.on_first_put.borrow_mut().as_mut() {
            hook();
        }
        Ok(receipt)
    }

    fn get(
        &self,
        vault_id: &[u8; 16],
        key_epoch: u64,
        artifact_id: &ArtifactId,
    ) -> Result<Vec<u8>, TransportError> {
        self.artifacts.get(vault_id, key_epoch, artifact_id)
    }

    fn status(
        &self,
        vault_id: &[u8; 16],
        key_epoch: u64,
        ids: &[ArtifactId],
    ) -> Result<Vec<ArtifactStatus>, TransportError> {
        self.artifacts.status(vault_id, key_epoch, ids)
    }
}

struct Device {
    _dir: tempfile::TempDir,
    worker: Arc<DbWorker>,
    secrets: MockSecretStore,
    witness: MockWitness,
    snapshots: FakeSnapshotService,
    artifacts: artifacts::FakeService,
}

impl Device {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let worker = DbWorker::open_with_raw_key(
            dir.path().join("test.vault"),
            vault_crypto::generate_dek().unwrap(),
        )
        .unwrap();
        Self {
            _dir: dir,
            worker: Arc::new(worker),
            secrets: MockSecretStore::new(),
            witness: MockWitness::new(),
            snapshots: FakeSnapshotService::new(Credential::Account),
            artifacts: artifacts::FakeService::default(),
        }
    }

    fn enable(&self, transport: &Bridged<'_>) -> Result<EnableOutcome, EnableError> {
        enable_sync(
            &self.worker,
            &self.secrets,
            &self.witness,
            &self.snapshots,
            transport,
            &SyncStatus::Healthy,
            &DEVICE,
        )
    }

    fn bridged(&self) -> Bridged<'_> {
        Bridged::new(&self.artifacts, &self.snapshots)
    }

    /// A batch with one record, written as an import leaves it, and (when `committed`) a provenance
    /// link naming its record, as the first commit that references it does.
    fn put_batch(&self, id: u8, committed: bool) -> SourceBatch {
        let batch_id = [id; 16];
        let record_id = [id.wrapping_add(0x40); 16];
        let batch = SourceBatch {
            batch: BatchRow {
                id: batch_id,
                source_type: "csv".to_owned(),
                source_name: Some("statement.csv".to_owned()),
                file_fingerprint: Some(format!("fp-{id}")),
                parser_version: Some("1".to_owned()),
                summary_json: Some("{}".to_owned()),
                imported_at: Some("2026-11-01T00:00:00Z".to_owned()),
                created_at: "2026-11-01T00:00:00Z".to_owned(),
                connector_connection_id: None,
            },
            runs: vec![RunRow {
                id: [id.wrapping_add(0x80); 16],
                source_batch_id: batch_id,
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
                id: record_id,
                source_batch_id: batch_id,
                external_id: Some(format!("ext-{id}")),
                source_hash: format!("hash-{id}"),
                normalized_json: "{\"n\":1}".to_owned(),
                parse_confidence_bps: Some(9000),
                created_at: "2026-11-01T00:00:00Z".to_owned(),
            }],
        };
        let conn = self.worker.read_connection().unwrap();
        let b = &batch.batch;
        conn.execute(
            "INSERT INTO source_batches (id, source_type, source_name, file_fingerprint,
                 parser_version, summary_json, imported_at, created_at, updated_at,
                 connector_connection_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, NULL)",
            rusqlite::params![
                b.id.as_slice(),
                b.source_type,
                b.source_name,
                b.file_fingerprint,
                b.parser_version,
                b.summary_json,
                b.imported_at,
                b.created_at,
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
                    r.finished_at
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
                    r.created_at
                ],
            )
            .unwrap();
            if committed {
                conn.execute(
                    "INSERT INTO source_provenance_links
                         (id, entity_type, entity_id, source_record_id, relationship, created_at)
                     VALUES (?1, 'ledger_transaction', ?2, ?3, 'created_from', ?4)",
                    rusqlite::params![
                        [id.wrapping_add(0xc0); 16].as_slice(),
                        [0x99_u8; 16].as_slice(),
                        r.id.as_slice(),
                        r.created_at
                    ],
                )
                .unwrap();
            }
        }
        batch
    }

    /// Remove a batch and everything of it, as discarding an import does.
    fn remove_batch(&self, id: u8) {
        let batch_id = [id; 16];
        let conn = self.worker.read_connection().unwrap();
        for sql in [
            "DELETE FROM source_provenance_links WHERE source_record_id IN
                 (SELECT id FROM source_records WHERE source_batch_id = ?1)",
            "DELETE FROM source_records WHERE source_batch_id = ?1",
            "DELETE FROM parser_runs WHERE source_batch_id = ?1",
            "DELETE FROM source_batches WHERE id = ?1",
        ] {
            conn.execute(sql, [batch_id.as_slice()]).unwrap();
        }
    }

    /// An attachment linked to an entity, as an import of a receipt leaves it.
    fn attach(&self, bytes: &[u8], entity: u8) -> core_ledger::AttachmentId {
        let id = self
            .worker
            .import_attachment(bytes, Some("application/pdf"), None)
            .unwrap();
        self.worker
            .link_attachment(id, "transaction", uuid::Uuid::from_bytes([entity; 16]))
            .unwrap();
        id
    }

    /// Unlink the attachment's last link: it is crypto-shredded.
    fn shred(&self, id: core_ledger::AttachmentId, entity: u8) {
        self.worker
            .unlink_attachment(id, "transaction", uuid::Uuid::from_bytes([entity; 16]))
            .unwrap();
    }

    /// The artifacts the closure holds, as production gives them.
    fn produced(&self) -> Vec<ArtifactId> {
        let service = artifacts::FakeService::default();
        let key = self.epoch_key();
        let (_, produced) = self.worker.produce_closure(&service, &key).unwrap();
        produced.iter().map(|p| p.id).collect()
    }

    fn epoch_key(&self) -> vault_crypto::sync_envelope::EpochKey {
        use sync_core::membership::verify_chain;
        use vault_crypto::sync_custody::SyncKeyCustody;
        let vault_id = *self.worker.vault_id().unwrap().as_bytes();
        let genesis = self
            .worker
            .write_sync_state(|tx| sync_genesis::load(tx))
            .unwrap()
            .unwrap();
        let chain = verify_chain(
            &vault_id,
            &[genesis.membership_record.as_slice()],
            &[genesis.epoch_record.as_slice()],
        )
        .unwrap();
        SyncKeyCustody::new(&self.secrets, vault_id)
            .load_epoch_key(&chain, 1)
            .unwrap()
    }
}

fn ids_of(bytes: &[u8]) -> Vec<ArtifactId> {
    bytes
        .as_chunks::<32>()
        .0
        .iter()
        .map(|id| ArtifactId::from_derived(*id))
        .collect()
}

/// The closure of a genesis is every attachment and every batch a provenance link reaches, in
/// ascending id order, in the install request; a batch no link reaches is not in it.
#[test]
fn a_genesis_carries_every_attachment_and_every_referenced_batch_in_order() {
    let device = Device::new();
    device
        .worker
        .import_attachment(PDF, Some("application/pdf"), Some("a.pdf"))
        .unwrap();
    device.worker.import_attachment(PNG, None, None).unwrap();
    device.put_batch(0x10, true);
    device.put_batch(0x20, false); // staged, never committed: not published (C.9)

    let transport = device.bridged();
    assert_eq!(device.enable(&transport).unwrap(), EnableOutcome::Enabled);
    let sent = device.snapshots.manifest_sent("install_epoch").unwrap();
    assert_eq!(sent.len(), 3, "two attachments and one batch of one part");
    let mut ascending = sent.clone();
    ascending.sort_by_key(|id| *id.as_bytes());
    assert_eq!(sent, ascending, "the closure is in ascending id order");
    assert!(sent.iter().all(|id| device.snapshots.is_pinned(id)));
    let snapshot_id = db_worker_genesis_snapshot_id(&device);
    assert_eq!(
        device
            .snapshots
            .closure_of(&snapshot_id)
            .map(|b| ids_of(&b)),
        Some(sent.clone())
    );
    // What production gives is what was sent, and the unreferenced batch is not in it.
    assert_eq!(device.produced(), sent);
    let sources = device.worker.closure_sources().unwrap();
    assert_eq!(sources.batches, vec![[0x10; 16]]);
    assert_eq!(sources.attachments.len(), 2);
}

fn db_worker_genesis_snapshot_id(device: &Device) -> [u8; 16] {
    use sync_core::membership::verify_chain;
    let vault_id = *device.worker.vault_id().unwrap().as_bytes();
    let genesis = device
        .worker
        .write_sync_state(|tx| sync_genesis::load(tx))
        .unwrap()
        .unwrap();
    let chain = verify_chain(
        &vault_id,
        &[genesis.membership_record.as_slice()],
        &[genesis.epoch_record.as_slice()],
    )
    .unwrap();
    vault_crypto::sync_snapshot_flow::genesis_snapshot_id(&chain.epoch_head_id().unwrap())
}

/// A put the service does not answer stops the enabling before anything is sealed or sent; calling
/// again puts the closure again and finishes, with the same artifacts.
#[test]
fn a_put_that_fails_stops_before_anything_is_sent_and_a_retry_finishes() {
    let device = Device::new();
    device
        .worker
        .import_attachment(PDF, Some("application/pdf"), None)
        .unwrap();
    device.worker.import_attachment(PNG, None, None).unwrap();

    let transport = device.bridged();
    transport.fail_put.set(Some(2));
    let first = device.enable(&transport);
    assert!(
        matches!(first, Err(EnableError::Closure(ProduceError::Publish(_)))),
        "{first:?}"
    );
    assert!(
        device.snapshots.calls().is_empty(),
        "no snapshot was staged or installed"
    );
    assert!(!device.snapshots.knows_vault());

    assert_eq!(device.enable(&transport).unwrap(), EnableOutcome::Enabled);
    let sent = device.snapshots.manifest_sent("install_epoch").unwrap();
    assert_eq!(sent.len(), 2);
    assert_eq!(device.produced(), sent, "the same artifacts, identically");
}

/// A closure whose receipts the service does not hold is refused at `installEpoch` with
/// `ArtifactMissing`, and nothing is pinned or published. When the service then lacks the objects,
/// the resume asks it (`getArtifactStatus`), finds them missing and puts them again, by their
/// logical ids through the mapping: the same ids and hashes, never an `ObjectConflict`, and the
/// saved container is then sent with the same bytes.
#[test]
fn a_missing_receipt_is_refused_and_pins_nothing_and_the_resume_puts_what_is_missing_again() {
    let mut device = Device::new();
    device
        .worker
        .import_attachment(PDF, Some("application/pdf"), None)
        .unwrap();
    device.put_batch(0x10, true);

    // The artifacts reach the artifact service, but the snapshot service has no receipt for them.
    let mut transport = device.bridged();
    transport.tell = false;
    let refused = device.enable(&transport);
    assert!(
        matches!(
            refused,
            Err(EnableError::Publish(PublishError::Service(
                ServiceError::ArtifactMissing { .. }
            )))
        ),
        "{refused:?}"
    );
    assert!(device.snapshots.published().is_empty(), "nothing published");
    let sources = device.worker.closure_sources().unwrap();
    assert!(!sources.attachments.is_empty());
    let ids = device.produced();
    assert!(ids.iter().all(|id| !device.snapshots.is_pinned(id)));
    let state = device
        .worker
        .write_sync_state(|tx| sync_genesis::load(tx))
        .unwrap()
        .unwrap()
        .state;
    assert_ne!(state, GenesisState::Published);

    // The service lost the objects (a fresh one): the resume finds them missing and puts them again
    // from the mapping, then sends the saved container.
    drop(transport);
    device.artifacts = artifacts::FakeService::default();
    let transport = device.bridged();
    assert_eq!(device.enable(&transport).unwrap(), EnableOutcome::Enabled);
    assert!(
        transport.puts.get() >= ids.len(),
        "the resume put what the service lacked"
    );
    assert!(ids.iter().all(|id| device.snapshots.is_pinned(id)));
    assert_eq!(
        device.snapshots.manifest_sent("install_epoch").unwrap(),
        ids
    );
}

/// A crash after the container was sealed and saved (the install answer is lost) is resumed with the
/// saved container: the service holds every artifact with its recorded hash, so nothing is put
/// again, `installEpoch` is an exact retry, and the genesis is installed once (04a's probe 2, with
/// the source still there).
#[test]
fn a_lost_install_response_resumes_by_resending_the_install_and_puts_nothing_again() {
    let device = Device::new();
    device
        .worker
        .import_attachment(PDF, Some("application/pdf"), None)
        .unwrap();
    device.put_batch(0x10, true);

    let transport = device.bridged();
    device.snapshots.script(Fault::Pass); // the stage
    device.snapshots.script(Fault::LoseResponse); // the install takes effect and the answer is lost
    let lost = device.enable(&transport);
    assert!(lost.is_err(), "{lost:?}");
    let puts_before = transport.puts.get();
    assert!(puts_before >= 2);

    let outcome = device.enable(&transport);
    assert!(
        matches!(
            outcome,
            Ok(EnableOutcome::Enabled | EnableOutcome::GenesisResumeLost)
        ),
        "{outcome:?}"
    );
    assert_eq!(
        transport.puts.get(),
        puts_before,
        "the service holds every artifact with its recorded hash: nothing is put again"
    );
    assert_eq!(device.snapshots.published().len(), 1, "installed once");
}

/// An attachment that arrives between producing the closure and exporting the vault is in the
/// closure the export carries: the closure is produced again, never exported without it.
#[test]
fn an_attachment_that_arrives_while_the_closure_is_produced_is_in_the_exported_closure() {
    let device = Device::new();
    device.worker.import_attachment(PDF, None, None).unwrap();
    let transport = device.bridged();
    let worker = Arc::clone(&device.worker);
    let arrived = Cell::new(false);
    *transport.on_first_put.borrow_mut() = Some(Box::new(move || {
        if !arrived.replace(true) {
            worker.import_attachment(PNG, None, None).unwrap();
        }
    }));

    assert_eq!(device.enable(&transport).unwrap(), EnableOutcome::Enabled);
    let sent = device.snapshots.manifest_sent("install_epoch").unwrap();
    assert_eq!(sent.len(), 2, "the late attachment is in the closure");
    assert_eq!(
        device.worker.closure_sources().unwrap().attachments.len(),
        2
    );
}

/// A vault that keeps changing under the production is never exported with an incomplete closure.
#[test]
fn a_vault_that_keeps_changing_is_never_exported_with_an_incomplete_closure() {
    let device = Device::new();
    device.worker.import_attachment(PDF, None, None).unwrap();
    let transport = device.bridged();
    let worker = Arc::clone(&device.worker);
    let n = Cell::new(0_u8);
    *transport.on_first_put.borrow_mut() = Some(Box::new(move || {
        // A different attachment after the first put of every production.
        n.set(n.get() + 1);
        let mut bytes = PNG.to_vec();
        bytes.push(n.get());
        worker.import_attachment(&bytes, None, None).unwrap();
    }));

    let result = device.enable(&transport);
    assert!(
        matches!(result, Err(EnableError::ClosureMoved)),
        "{result:?}"
    );
    assert!(device.snapshots.calls().is_empty(), "nothing was sent");
}

/// 04a's probe 1 (F1): an attachment is shredded and the same bytes are imported again between two
/// enable attempts. The new attachment has a new id, so it has its own address (C.3: the address
/// names the logical id), and no address ever receives two different containers.
#[test]
fn a_shred_and_a_reimport_between_enable_attempts_gives_the_new_attachment_its_own_address() {
    let device = Device::new();
    let first_id = device.attach(PDF, 0x77);
    device.put_batch(0x10, true);
    let transport = device.bridged();
    transport.fail_put.set(Some(2)); // the batch part's put fails; the attachment's landed
    assert!(device.enable(&transport).is_err());
    let first_sent: Vec<ArtifactId> = transport.seen.borrow().iter().map(|(id, _)| *id).collect();

    device.shred(first_id, 0x77);
    let second_id = device.attach(PDF, 0x78);
    assert_ne!(first_id, second_id, "a re-import is a new attachment");

    assert_eq!(device.enable(&transport).unwrap(), EnableOutcome::Enabled);
    assert_eq!(
        transport.addresses_with_two_containers(),
        0,
        "no address was sent two different plaintexts"
    );
    let sent = device.snapshots.manifest_sent("install_epoch").unwrap();
    assert_eq!(sent.len(), 2, "the new attachment and the batch part");
    assert!(
        !sent.contains(&first_sent[0]),
        "the shredded attachment's address is not in the genesis"
    );
}

/// 04a's probe 2 (F2): the service installed the genesis and its answer was lost, and then an
/// attachment of the closure was shredded. The service holds every artifact with its recorded hash,
/// so the resume puts nothing, re-sends `installEpoch` as an exact retry, and finishes.
#[test]
fn a_shred_after_a_lost_install_response_still_finishes_the_enable() {
    let device = Device::new();
    let attachment = device.attach(PDF, 0x78);
    let transport = device.bridged();
    device.snapshots.script(Fault::Pass);
    device.snapshots.script(Fault::LoseResponse);
    assert!(device.enable(&transport).is_err());
    assert_eq!(
        device.snapshots.published().len(),
        1,
        "the service installed it"
    );
    let puts = transport.puts.get();

    device.shred(attachment, 0x78);
    let outcome = device.enable(&transport);
    assert!(
        matches!(
            outcome,
            Ok(EnableOutcome::Enabled | EnableOutcome::GenesisResumeLost)
        ),
        "{outcome:?}"
    );
    assert_eq!(transport.puts.get(), puts, "nothing was put again");
    assert_eq!(device.snapshots.published().len(), 1, "installed once");
}

/// The same, for a batch that was discarded after the lost response.
#[test]
fn a_batch_discard_after_a_lost_install_response_still_finishes_the_enable() {
    let device = Device::new();
    device.put_batch(0x10, true);
    let transport = device.bridged();
    device.snapshots.script(Fault::Pass);
    device.snapshots.script(Fault::LoseResponse);
    assert!(device.enable(&transport).is_err());
    assert_eq!(device.snapshots.published().len(), 1);
    let puts = transport.puts.get();

    device.remove_batch(0x10);
    let outcome = device.enable(&transport);
    assert!(
        matches!(
            outcome,
            Ok(EnableOutcome::Enabled | EnableOutcome::GenesisResumeLost)
        ),
        "{outcome:?}"
    );
    assert_eq!(transport.puts.get(), puts);
    assert_eq!(device.snapshots.published().len(), 1);
}

/// A saved genesis whose artifacts the service lacks and whose attachment is gone cannot have been
/// installed (the service pins the whole closure before the compare-and-swap): the saved container
/// is discarded and one fresh genesis is produced from the vault as it is, which the service
/// accepts; no address is ever sent two containers.
#[test]
fn a_missing_artifact_whose_source_is_gone_discards_the_saved_genesis_and_produces_a_fresh_one() {
    let mut device = Device::new();
    let attachment = device.attach(PDF, 0x79);
    device.put_batch(0x10, true);
    let mut transport = device.bridged();
    transport.tell = false;
    let refused = device.enable(&transport);
    assert!(
        matches!(refused, Err(EnableError::Publish(_))),
        "{refused:?}"
    );
    assert!(device.snapshots.published().is_empty());

    // The service lost its objects and its staged snapshot (an unpinned object and a staged
    // snapshot are both collected after a while), and the attachment was shredded.
    drop(transport);
    device.artifacts = artifacts::FakeService::default();
    device.snapshots.script(Fault::DropStaged);
    device.shred(attachment, 0x79);
    let transport = device.bridged();
    assert_eq!(device.enable(&transport).unwrap(), EnableOutcome::Enabled);
    let sent = device.snapshots.manifest_sent("install_epoch").unwrap();
    assert_eq!(sent.len(), 1, "only the batch part is in the fresh genesis");
    assert!(sent.iter().all(|id| device.snapshots.is_pinned(id)));
    assert_eq!(transport.addresses_with_two_containers(), 0);
    assert_eq!(device.snapshots.published().len(), 1);
}

/// An object the service holds at an address with another hash than the one this device recorded is
/// a typed failure: nothing is put over it, nothing is installed, and the enable stops.
#[test]
fn an_object_with_another_hash_at_a_closure_address_stops_the_enable() {
    let mut device = Device::new();
    device.attach(PDF, 0x7a);
    let mut transport = device.bridged();
    transport.tell = false;
    assert!(device.enable(&transport).is_err());
    let ids = device.produced();
    drop(transport);

    // Something else sits at the address.
    device.artifacts = artifacts::FakeService::default();
    let squatter = sync_core::artifact::encode_container(&[0x5a; 48]);
    let vault_id = *device.worker.vault_id().unwrap().as_bytes();
    device
        .artifacts
        .put(&vault_id, 1, &ids[0], &squatter)
        .unwrap();
    let transport = device.bridged();
    let result = device.enable(&transport);
    assert!(
        matches!(
            result,
            Err(EnableError::Closure(
                ProduceError::ServiceHoldsAnotherObject(_)
            ))
        ),
        "{result:?}"
    );
    assert_eq!(transport.puts.get(), 0, "nothing was put over it");
    assert!(device.snapshots.published().is_empty());
}

/// A key rotation is refused while Sync is being enabled (the genesis is persisted and not yet
/// published), before anything is created; once the genesis is published it is allowed.
#[test]
fn a_key_rotation_is_refused_while_a_genesis_is_in_flight_and_allowed_after_it() {
    let device = Device::new();
    device.attach(PDF, 0x7b);
    let transport = device.bridged();
    transport.fail_put.set(Some(1));
    assert!(device.enable(&transport).is_err());
    assert!(device.worker.genesis_in_flight().unwrap());

    let new_dek = vault_crypto::generate_dek().unwrap();
    let dest = device._dir.path().join("rekeyed.copy");
    let refused = device.worker.prepare_rekeyed_copy(&new_dek, &dest);
    assert!(
        matches!(
            refused,
            Err(db_worker::DbError::RekeyRefusedGenesisInFlight)
        ),
        "{refused:?}"
    );
    assert!(!dest.exists(), "nothing was created");

    assert_eq!(device.enable(&transport).unwrap(), EnableOutcome::Enabled);
    assert!(!device.worker.genesis_in_flight().unwrap());
    device
        .worker
        .prepare_rekeyed_copy(&new_dek, &dest)
        .expect("a rotation is allowed once the genesis is published");
}
