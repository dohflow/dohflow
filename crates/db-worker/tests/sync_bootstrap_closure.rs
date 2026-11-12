//! A replica built from a snapshot that carries a class-1b closure
//! (`db_worker::sync_bootstrap::bootstrap_replica`, `db_worker::sync_bootstrap_artifacts`;
//! personal-cfo-1cltt.7 slice D2): every artifact is fetched, verified and materialized before the
//! vault exists, the attachment rows come from the snapshot's row plus the staged crypto, the live
//! closure is seeded, and a failure is one of three kinds: DETERMINISTIC (the snapshot is bad),
//! TRANSIENT (the service's fault) or LOCAL (this device's).

#![allow(dead_code)]

#[path = "support/artifacts.rs"]
mod artifacts;
mod common;

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use artifacts::{key_of, FakeService};
use db_worker::logical_export::{export_genesis_with_mark, ExportInput};
use db_worker::sync_bootstrap::{
    bootstrap_replica, BootstrapError, Bootstrapped, ClosureSource, FailureKind, ReplicaSource,
    SPOOL_SUFFIX,
};
use db_worker::sync_bootstrap_artifacts::{InvalidSnapshot, TransientFault};
use db_worker::sync_produce::ProducedArtifact;
use db_worker::DbWorker;
use sync_core::artifact::{ArtifactId, ArtifactReceipt};
use sync_core::artifact_transport::{ArtifactStatus, ArtifactTransport, TransportError};
use sync_core::envelope::BoundArtifact;
use sync_core::snapshot::{verify, Verified};
use sync_core::status::EngineVersion;
use vault_crypto::generate_dek;
use vault_crypto::sync_envelope::EpochKey;

const PDF: &[u8] = b"%PDF-1.4\nthe first attachment\n%%EOF\n";
const TWIN: [u8; 16] = [0x55; 16];
const PNG: &[u8] = b"\x89PNG\r\n\x1a\nthe second attachment";
/// A string that appears only in batch plaintext: it must never be in a spool file.
const MARKER: &str = "PLAINTEXT-MARKER-0123456789";

fn engine() -> EngineVersion {
    EngineVersion::new(db_worker::ENGINE_VERSION).unwrap()
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// A batch of five records of about 900 KB each (two parts: a part is at most 4 MiB), each record
/// carrying [`MARKER`], and a provenance link naming its first record, as the commit that references
/// the batch does.
fn put_batch(worker: &DbWorker, id: [u8; 16], n: u8) {
    let conn = worker.read_connection().unwrap();
    conn.execute(
        "INSERT INTO source_batches (id, source_type, source_name, file_fingerprint, parser_version,
             summary_json, imported_at, created_at, updated_at)
         VALUES (?1, 'csv', ?2, ?3, '1', '{}', '2026-11-01T00:00:00Z', '2026-11-01T00:00:00Z',
                 '2026-11-01T00:00:00Z')",
        rusqlite::params![id.as_slice(), format!("statement-{n}.csv"), format!("fp-{n}")],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO parser_runs (id, source_batch_id, parser_name, parser_version, bytes_in,
             records_out, status, started_at)
         VALUES (?1, ?2, 'csv', '1', 10, 5, 'ok', '2026-11-01T00:00:00Z')",
        rusqlite::params![[0xc0 + n; 16].as_slice(), id.as_slice()],
    )
    .unwrap();
    for r in 1..=5_u8 {
        let record = [n.wrapping_mul(16).wrapping_add(r); 16];
        conn.execute(
            "INSERT INTO source_records (id, source_batch_id, external_id, source_hash,
                 normalized_json, parse_confidence_bps, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 9000, '2026-11-01T00:00:00Z')",
            rusqlite::params![
                record.as_slice(),
                id.as_slice(),
                format!("ext-{n}-{r}"),
                format!("hash-{n}-{r}"),
                format!(
                    "{{\"marker\":\"{MARKER}\",\"pad\":\"{}\"}}",
                    "x".repeat(900_000)
                ),
            ],
        )
        .unwrap();
        if r == 1 {
            conn.execute(
                "INSERT INTO source_provenance_links
                     (id, entity_type, entity_id, source_record_id, relationship, created_at)
                 VALUES (?1, 'ledger_transaction', ?2, ?3, 'created_from', 't')",
                rusqlite::params![
                    [0xd0 + n; 16].as_slice(),
                    [0x99_u8; 16].as_slice(),
                    record.as_slice()
                ],
            )
            .unwrap();
        }
    }
}

struct Origin {
    _dir: tempfile::TempDir,
    worker: DbWorker,
    service: FakeService,
    key: EpochKey,
    verified: Verified,
    produced: Vec<ProducedArtifact>,
    attachment: [u8; 16],
}

impl Origin {
    /// An origin with two attachments (the first's row media type edited after its artifact was
    /// made) and `batches` batches of two parts, every artifact put on the service, and the genesis
    /// snapshot it would publish.
    fn new(batches: u8) -> Self {
        Self::build(batches, false)
    }

    /// [`Self::new`], and with `twin` a third attachment: the same bytes as the first under another
    /// attachment id (two devices attached one file), its row sharing the first's blob, its own
    /// artifact on the service and in the snapshot.
    fn build(batches: u8, twin: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let worker =
            DbWorker::open_with_raw_key(dir.path().join("origin.db"), generate_dek().unwrap())
                .unwrap();
        let key = key_of(1, 0);
        let service = FakeService::default();
        let a1 = worker
            .import_attachment(PDF, Some("application/pdf"), Some("a.pdf"))
            .unwrap();
        worker
            .import_attachment(PNG, Some("image/png"), None)
            .unwrap();
        for n in 1..=batches {
            put_batch(&worker, [0xb0 + n; 16], n);
        }
        let attachment = *a1.as_uuid().as_bytes();
        worker
            .read_connection()
            .unwrap()
            .execute(
                "INSERT INTO attachment_links (attachment_id, entity_kind, entity_id, created_at)
                 VALUES (?1, 'transaction', ?2, 't')",
                rusqlite::params![attachment.as_slice(), [0x77_u8; 16].as_slice()],
            )
            .unwrap();
        let (_, produced) = worker.produce_closure(&service, &key).unwrap();
        let mut extra: Vec<BoundArtifact> = Vec::new();
        if twin {
            let published =
                artifacts::publish_attachment(&service, &key, 0x9a, TWIN, PDF, "application/pdf");
            extra.push(BoundArtifact {
                id: *published.id.as_bytes(),
                ciphertext_sha256: published.hash,
            });
            let conn = worker.read_connection().unwrap();
            conn.execute(
                "INSERT INTO attachments (id, storage_id, wrapped_content_key, content_key_nonce,
                        content_nonce, content_alg, plaintext_size, mime_type, original_filename,
                        ref_count, created_at)
                 SELECT ?2, storage_id, wrapped_content_key, content_key_nonce, content_nonce,
                        content_alg, plaintext_size, mime_type, 'twin.pdf', 0, created_at
                   FROM attachments WHERE id = ?1",
                rusqlite::params![attachment.as_slice(), TWIN.as_slice()],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO attachment_links (attachment_id, entity_kind, entity_id, created_at)
                 VALUES (?1, 'transaction', ?2, 't')",
                rusqlite::params![TWIN.as_slice(), [0x78_u8; 16].as_slice()],
            )
            .unwrap();
        }
        // A later class-1 edit of the row: its media type is no longer the artifact's.
        worker
            .read_connection()
            .unwrap()
            .execute(
                "UPDATE attachments SET mime_type = 'application/x-edited' WHERE id = ?1",
                [attachment.as_slice()],
            )
            .unwrap();
        let mut bound: Vec<BoundArtifact> = produced
            .iter()
            .map(|p| BoundArtifact {
                id: *p.id.as_bytes(),
                ciphertext_sha256: p.ciphertext_sha256,
            })
            .collect();
        bound.extend(extra);
        bound.sort_by_key(|b| b.id);
        let (plaintext, _) = export_genesis_with_mark(
            &worker.read_connection().unwrap(),
            &ExportInput {
                artifacts: &bound,
                ..ExportInput::default()
            },
        )
        .unwrap();
        Self {
            _dir: dir,
            worker,
            service,
            key,
            verified: verify(&plaintext).unwrap(),
            produced,
            attachment,
        }
    }

    fn vault_id(&self) -> [u8; 16] {
        *self.worker.vault_id().unwrap().as_bytes()
    }

    /// Build a replica from the origin's genesis, fetching its closure from `transport`.
    fn bootstrap(
        &self,
        verified: &Verified,
        dir: &Path,
        transport: Option<&dyn ArtifactTransport>,
    ) -> Result<Bootstrapped, BootstrapError> {
        let (version, own) = (engine(), engine());
        let dek = Arc::new(generate_dek().unwrap());
        bootstrap_replica(
            &dir.join("replica.vault"),
            &dek,
            &ReplicaSource {
                vault_id: self.vault_id(),
                root: 0,
                snapshot: verified,
                engine_version: &version,
                own_engine_version: &own,
                held_voids: &[],
                closure: transport.map(|transport| ClosureSource {
                    transport,
                    key: &self.key,
                }),
            },
        )
    }

    /// The first part of the first batch, and an attachment artifact: handles for tampering.
    fn attachment_artifact(&self) -> &ProducedArtifact {
        self.produced
            .iter()
            .find(|p| p.logical_id == self.attachment)
            .unwrap()
    }
}

/// A transport that watches the spool while parts are fetched, and counts what it sees.
struct Watching<'a> {
    inner: &'a FakeService,
    spool: PathBuf,
    fetched: RefCell<Vec<ArtifactId>>,
    most_files: Cell<usize>,
    plaintext_seen: Cell<bool>,
}

impl ArtifactTransport for Watching<'_> {
    fn put(
        &self,
        vault_id: &[u8; 16],
        key_epoch: u64,
        artifact_id: &ArtifactId,
        container: &[u8],
    ) -> Result<ArtifactReceipt, TransportError> {
        self.inner.put(vault_id, key_epoch, artifact_id, container)
    }

    fn get(
        &self,
        vault_id: &[u8; 16],
        key_epoch: u64,
        artifact_id: &ArtifactId,
    ) -> Result<Vec<u8>, TransportError> {
        // Whatever waits on disk right now: how many files, and whether any holds plaintext.
        if let Ok(entries) = std::fs::read_dir(&self.spool) {
            let files: Vec<_> = entries.flatten().collect();
            self.most_files.set(self.most_files.get().max(files.len()));
            for file in files {
                let bytes = std::fs::read(file.path()).unwrap();
                if bytes
                    .windows(MARKER.len())
                    .any(|window| window == MARKER.as_bytes())
                {
                    self.plaintext_seen.set(true);
                }
            }
        }
        self.fetched.borrow_mut().push(*artifact_id);
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

fn count(worker: &DbWorker, sql: &str) -> i64 {
    worker
        .read_connection()
        .unwrap()
        .query_row(sql, [], |r| r.get(0))
        .unwrap()
}

/// The carried rows of every batch table, as text: what a replica must equal.
fn batch_rows(worker: &DbWorker) -> Vec<String> {
    let conn = worker.read_connection().unwrap();
    let mut out = Vec::new();
    for sql in [
        "SELECT id, source_type, source_name, file_fingerprint, parser_version, summary_json,
                imported_at, created_at FROM source_batches ORDER BY id",
        "SELECT id, source_batch_id, parser_name, parser_version, bytes_in, records_out, status,
                started_at FROM parser_runs ORDER BY id",
        "SELECT id, source_batch_id, external_id, source_hash, length(normalized_json)
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

/// The replica is the origin: its attachments (rows and bytes, the row's own media type), its
/// batches, and every class-1 table; the live closure holds the closure; nothing is left staged or
/// spooled.
#[test]
fn a_replica_is_built_with_every_artifact_of_the_closure_materialized() {
    let origin = Origin::new(4);
    let dir = tempfile::tempdir().unwrap();
    let transport = Watching {
        inner: &origin.service,
        spool: with_suffix(&dir.path().join("replica.vault"), SPOOL_SUFFIX),
        fetched: RefCell::new(Vec::new()),
        most_files: Cell::new(0),
        plaintext_seen: Cell::new(false),
    };
    let born = origin
        .bootstrap(&origin.verified, dir.path(), Some(&transport))
        .unwrap();
    let replica = &born.worker;

    // Every attachment: its row has the SNAPSHOT's columns (the edited media type is not an error),
    // its bytes read back, and `ref_count` is rebuilt from the links.
    type AttachmentRow = (Vec<u8>, i64, Option<String>, Option<String>, i64);
    let attachments: Vec<AttachmentRow> = replica
        .read_connection()
        .unwrap()
        .prepare("SELECT id, plaintext_size, mime_type, original_filename, ref_count FROM attachments ORDER BY id")
        .unwrap()
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(attachments.len(), 2);
    let first = attachments
        .iter()
        .find(|a| a.0.as_slice() == origin.attachment.as_slice())
        .unwrap();
    assert_eq!(first.2.as_deref(), Some("application/x-edited"));
    assert_eq!(first.3.as_deref(), Some("a.pdf"));
    assert_eq!(first.4, 1, "ref_count is the count of the links");
    let id = core_ledger::AttachmentId::from_uuid(uuid::Uuid::from_bytes(origin.attachment));
    assert_eq!(replica.read_attachment_bytes(id).unwrap(), PDF);
    // Every batch, to the same rows.
    assert_eq!(batch_rows(replica), batch_rows(&origin.worker));
    assert_eq!(count(replica, "SELECT COUNT(*) FROM source_batches"), 4);
    // The live closure is the closure, and no staging is left.
    assert_eq!(
        count(replica, "SELECT COUNT(*) FROM sync_live_closure"),
        origin.produced.len() as i64
    );
    assert_eq!(
        count(
            replica,
            "SELECT COUNT(*) FROM sync_artifacts WHERE staged_storage_id IS NOT NULL"
        ),
        0
    );
    // Foreign keys are on and nothing dangles.
    assert_eq!(count(replica, "PRAGMA foreign_keys"), 1);
    assert_eq!(
        count(replica, "SELECT COUNT(*) FROM pragma_foreign_key_check"),
        0
    );
    // Ciphertext waited on disk for siblings (the ids interleave), never plaintext, and the spool is
    // gone.
    assert!(transport.most_files.get() > 0, "a part waited in the spool");
    assert!(!transport.plaintext_seen.get(), "the spool held plaintext");
    assert!(!transport.spool.exists(), "the spool is removed");
    assert_eq!(
        transport.fetched.borrow().len(),
        origin.produced.len(),
        "every artifact was fetched once"
    );
}

/// The ids of different batches interleave in the closure's order, so a part waits for siblings that
/// arrive later: with four batches the fetch order is not grouped by batch. (The addresses depend on
/// the origin's key, so a grouped draw is drawn again.)
#[test]
fn parts_of_different_batches_interleave_and_the_replica_is_still_the_origin() {
    for _ in 0..8 {
        let origin = Origin::new(4);
        let order: Vec<[u8; 16]> = origin
            .verified
            .manifest
            .artifacts
            .iter()
            .map(|entry| {
                let id = hex_to_32(&entry.id);
                origin
                    .produced
                    .iter()
                    .find(|p| p.id.as_bytes() == &id)
                    .unwrap()
                    .logical_id
            })
            .filter(|logical| *logical != origin.attachment)
            .collect();
        let grouped = order.windows(3).all(|w| !(w[0] != w[1] && w[0] == w[2]));
        let mut runs = 1;
        for w in order.windows(2) {
            if w[0] != w[1] {
                runs += 1;
            }
        }
        if runs <= 4 && grouped {
            continue; // every batch arrived in one run: draw again
        }
        let dir = tempfile::tempdir().unwrap();
        let born = origin
            .bootstrap(&origin.verified, dir.path(), Some(&origin.service))
            .unwrap();
        assert_eq!(batch_rows(&born.worker), batch_rows(&origin.worker));
        return;
    }
    panic!("no draw interleaved the batches");
}

fn hex_to_32(hex: &str) -> [u8; 32] {
    let mut out = [0_u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap();
    }
    out
}

fn assert_no_vault_and_no_spool(dir: &Path) {
    let path = dir.join("replica.vault");
    assert!(!path.exists(), "no vault was made");
    assert!(
        !with_suffix(&path, SPOOL_SUFFIX).exists(),
        "the spool is removed whatever the outcome"
    );
}

/// DETERMINISTIC: an object that hashes to the bound value but does not authenticate is the snapshot's
/// fault, at every replica.
#[test]
fn an_object_that_hashes_to_the_bound_value_but_does_not_open_is_a_deterministic_invalid_snapshot()
{
    let origin = Origin::new(1);
    let target = *origin.attachment_artifact();
    // The service holds a container whose ciphertext is damaged, and the snapshot binds THAT hash.
    origin.service.corrupt(1, &target.id);
    let container = origin.service.get(&[0x11; 16], 1, &target.id).unwrap();
    let ciphertext = sync_core::artifact::parse_container(
        &container,
        sync_core::artifact::MAX_ARTIFACT_CONTAINER,
    )
    .unwrap();
    use sha2::Digest;
    let damaged_hash: [u8; 32] = sha2::Sha256::digest(ciphertext).into();
    let mut verified = origin.verified.clone();
    let entry = verified
        .manifest
        .artifacts
        .iter_mut()
        .find(|e| hex_to_32(&e.id) == *target.id.as_bytes())
        .unwrap();
    entry.ciphertext_sha256 = damaged_hash.iter().map(|b| format!("{b:02x}")).collect();

    let dir = tempfile::tempdir().unwrap();
    let error = origin
        .bootstrap(&verified, dir.path(), Some(&origin.service))
        .unwrap_err();
    assert!(
        matches!(error, BootstrapError::Invalid(InvalidSnapshot::Closure(_))),
        "{error:?}"
    );
    assert_eq!(error.kind(), FailureKind::Deterministic);
    assert_no_vault_and_no_spool(dir.path());
}

/// TRANSIENT: an object the service does not hold, or bytes that are not the bound ones, is the
/// service's fault: the same root is retried, and succeeds once the service delivers.
#[test]
fn a_missing_or_wrong_object_is_a_transient_fault_and_the_same_root_succeeds_on_retry() {
    let origin = Origin::new(1);
    let target = *origin.attachment_artifact();
    let dir = tempfile::tempdir().unwrap();

    // Missing.
    let held = origin.service.get(&[0x11; 16], 1, &target.id).unwrap();
    origin.service.lose(1, &target.id);
    let error = origin
        .bootstrap(&origin.verified, dir.path(), Some(&origin.service))
        .unwrap_err();
    assert!(
        matches!(error, BootstrapError::Transient(TransientFault::Missing(id)) if id == target.id),
        "{error:?}"
    );
    assert_eq!(error.kind(), FailureKind::Transient);
    assert_no_vault_and_no_spool(dir.path());

    // Bytes that are not the bound ones: the held bytes, damaged.
    origin.service.serve(1, &target.id, held.clone());
    origin.service.corrupt(1, &target.id);
    let error = origin
        .bootstrap(&origin.verified, dir.path(), Some(&origin.service))
        .unwrap_err();
    assert!(
        matches!(error, BootstrapError::Transient(TransientFault::HashMismatch(id)) if id == target.id),
        "{error:?}"
    );
    assert_eq!(error.kind(), FailureKind::Transient);
    assert_no_vault_and_no_spool(dir.path());

    // The service delivers: the same root builds the replica.
    origin.service.serve(1, &target.id, held);
    let born = origin
        .bootstrap(&origin.verified, dir.path(), Some(&origin.service))
        .expect("the same root, retried, is built");
    assert_eq!(count(&born.worker, "SELECT COUNT(*) FROM attachments"), 2);
}

/// LOCAL: a failure of this device (here, the spool cannot be made) is never the snapshot's fault.
#[test]
fn a_local_failure_is_a_local_fault_and_never_marks_the_root_invalid() {
    let origin = Origin::new(1);
    let dir = tempfile::tempdir().unwrap();
    // Something that is not a directory sits where the spool goes.
    std::fs::write(
        with_suffix(&dir.path().join("replica.vault"), SPOOL_SUFFIX),
        b"in the way",
    )
    .unwrap();
    let error = origin
        .bootstrap(&origin.verified, dir.path(), Some(&origin.service))
        .unwrap_err();
    assert!(matches!(error, BootstrapError::Local(_)), "{error:?}");
    assert_eq!(error.kind(), FailureKind::Local);
    assert!(!dir.path().join("replica.vault").exists());
}

/// A snapshot row with no artifact, a row whose size disagrees with its artifact, and a link that
/// names a record no batch holds are each deterministic.
#[test]
fn a_row_and_its_artifact_that_disagree_are_deterministic_invalid() {
    // A row with no artifact.
    let origin = Origin::new(1);
    let target = *origin.attachment_artifact();
    let mut verified = origin.verified.clone();
    verified
        .manifest
        .artifacts
        .retain(|e| hex_to_32(&e.id) != *target.id.as_bytes());
    let dir = tempfile::tempdir().unwrap();
    let error = origin
        .bootstrap(&verified, dir.path(), Some(&origin.service))
        .unwrap_err();
    assert!(
        matches!(
            error,
            BootstrapError::Invalid(InvalidSnapshot::AttachmentWithoutArtifact)
        ),
        "{error:?}"
    );
    assert_eq!(error.kind(), FailureKind::Deterministic);
    assert_no_vault_and_no_spool(dir.path());

    // A size that disagrees: the origin's row says another size than its bytes.
    let origin = Origin::new(1);
    origin
        .worker
        .read_connection()
        .unwrap()
        .execute(
            "UPDATE attachments SET plaintext_size = plaintext_size + 1",
            [],
        )
        .unwrap();
    let (plaintext, _) = export_genesis_with_mark(
        &origin.worker.read_connection().unwrap(),
        &ExportInput {
            artifacts: &origin
                .produced
                .iter()
                .map(|p| BoundArtifact {
                    id: *p.id.as_bytes(),
                    ciphertext_sha256: p.ciphertext_sha256,
                })
                .collect::<Vec<_>>(),
            ..ExportInput::default()
        },
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let error = origin
        .bootstrap(
            &verify(&plaintext).unwrap(),
            dir.path(),
            Some(&origin.service),
        )
        .unwrap_err();
    assert!(
        matches!(
            error,
            BootstrapError::Invalid(InvalidSnapshot::AttachmentSize)
        ),
        "{error:?}"
    );
    assert_no_vault_and_no_spool(dir.path());
}

/// A provenance link that names a source record no batch of the closure holds discards the build.
#[test]
fn a_dangling_provenance_link_discards_the_build() {
    let origin = Origin::new(2);
    // Leave every part of one batch out of the closure: its link now names a record nothing holds.
    let batch = origin
        .produced
        .iter()
        .find(|p| {
            matches!(
                p.kind,
                db_worker::sync_artifacts::ArtifactKind::BatchPart { .. }
            )
        })
        .unwrap()
        .logical_id;
    let dropped: Vec<[u8; 32]> = origin
        .produced
        .iter()
        .filter(|p| p.logical_id == batch)
        .map(|p| *p.id.as_bytes())
        .collect();
    let mut verified = origin.verified.clone();
    verified
        .manifest
        .artifacts
        .retain(|e| !dropped.contains(&hex_to_32(&e.id)));
    let dir = tempfile::tempdir().unwrap();
    let error = origin
        .bootstrap(&verified, dir.path(), Some(&origin.service))
        .unwrap_err();
    assert!(
        matches!(
            error,
            BootstrapError::Invalid(InvalidSnapshot::DanglingReference)
        ),
        "{error:?}"
    );
    assert_eq!(error.kind(), FailureKind::Deterministic);
    assert_no_vault_and_no_spool(dir.path());
}

/// A snapshot that needs its closure is refused, writing nothing, when the caller has no artifact
/// service.
#[test]
fn a_snapshot_that_needs_its_closure_is_refused_without_a_service() {
    let origin = Origin::new(1);
    let dir = tempfile::tempdir().unwrap();
    let error = origin
        .bootstrap(&origin.verified, dir.path(), None)
        .unwrap_err();
    assert!(
        matches!(
            error,
            BootstrapError::Install(db_worker::snapshot_install::InstallError::NeedsArtifacts)
        ),
        "{error:?}"
    );
    assert!(!dir.path().join("replica.vault").exists());
    assert!(!with_suffix(&dir.path().join("replica.vault"), ".bootstrapping").exists());
}

/// A snapshot with no artifact is built the same whether or not the caller passes a service (the
/// engine passes one whenever Sync is on): the service is never asked, the install's own proof is
/// the only proof, and the live closure stays empty.
#[test]
fn a_snapshot_with_no_artifact_ignores_a_service_the_caller_passes() {
    let dir = tempfile::tempdir().unwrap();
    let worker =
        DbWorker::open_with_raw_key(dir.path().join("origin.db"), generate_dek().unwrap()).unwrap();
    let (plaintext, _) =
        export_genesis_with_mark(&worker.read_connection().unwrap(), &ExportInput::default())
            .unwrap();
    let verified = verify(&plaintext).unwrap();
    assert!(verified.manifest.artifacts.is_empty());
    let (version, own) = (engine(), engine());
    let dek = Arc::new(generate_dek().unwrap());
    let key = key_of(1, 0);
    let service = FakeService::default();
    let path = dir.path().join("replica.vault");
    let born = bootstrap_replica(
        &path,
        &dek,
        &ReplicaSource {
            vault_id: *worker.vault_id().unwrap().as_bytes(),
            root: 0,
            snapshot: &verified,
            engine_version: &version,
            own_engine_version: &own,
            held_voids: &[],
            closure: Some(ClosureSource {
                transport: &service,
                key: &key,
            }),
        },
    )
    .unwrap();
    assert_eq!(
        count(&born.worker, "SELECT COUNT(*) FROM sync_live_closure"),
        0
    );
    assert_eq!(
        count(&born.worker, "PRAGMA foreign_keys"),
        1,
        "foreign keys are on"
    );
}

/// Two attachment ids with the same bytes are honest (two devices attached the same file): the replica
/// is built with both rows sharing ONE blob, each reads the bytes, and it is not a verdict.
#[test]
fn two_attachment_ids_with_the_same_bytes_are_built_with_one_blob() {
    let origin = Origin::build(1, true);
    let dir = tempfile::tempdir().unwrap();
    let born = origin
        .bootstrap(&origin.verified, dir.path(), Some(&origin.service))
        .unwrap();
    let replica = &born.worker;
    let rows = count(
        replica,
        "SELECT COUNT(*) FROM attachments WHERE original_filename IN ('a.pdf', 'twin.pdf')",
    );
    assert_eq!(rows, 2);
    assert_eq!(
        count(
            replica,
            "SELECT COUNT(DISTINCT storage_id) FROM attachments
              WHERE original_filename IN ('a.pdf', 'twin.pdf')"
        ),
        1,
        "the two rows name one blob"
    );
    for id in [origin.attachment, TWIN] {
        let id = core_ledger::AttachmentId::from_uuid(uuid::Uuid::from_bytes(id));
        assert_eq!(replica.read_attachment_bytes(id).unwrap(), PDF);
    }
    let blobs = std::fs::read_dir(dir.path().join("blobs")).unwrap().count();
    assert_eq!(blobs, 2, "the shared PDF blob and the PNG blob");
    // No staging is left, and no spool.
    assert_eq!(
        count(
            replica,
            "SELECT COUNT(*) FROM sync_artifacts WHERE staged_storage_id IS NOT NULL"
        ),
        0
    );
}

/// A spool an earlier build left (a crash) is discarded at the next start, with the build it belonged
/// to, and the next build succeeds and leaves none.
#[test]
fn a_stale_spool_from_a_crashed_build_is_discarded_at_the_next_start() {
    let origin = Origin::new(1);
    let dir = tempfile::tempdir().unwrap();
    let spool = with_suffix(&dir.path().join("replica.vault"), SPOOL_SUFFIX);
    std::fs::create_dir_all(&spool).unwrap();
    std::fs::write(spool.join("00aa"), b"left by a crash").unwrap();
    let born = origin
        .bootstrap(&origin.verified, dir.path(), Some(&origin.service))
        .unwrap();
    assert_eq!(
        count(&born.worker, "SELECT COUNT(*) FROM source_batches"),
        1
    );
    assert!(
        !spool.exists(),
        "the stale spool was discarded and none remains"
    );
}
