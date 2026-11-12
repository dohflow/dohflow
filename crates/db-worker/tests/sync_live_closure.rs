//! The live closure a replica holds (`db_worker::sync_live_closure`, personal-cfo-1cltt.7 slice D1):
//! seeded from a snapshot, maintained by each accepted envelope's apply in that transaction, rebuilt
//! at a key epoch, never written by a local command, and the same on a replica that tailed the log as
//! on one bootstrapped later (03a's R2 scenario). Class-1 state is written with raw SQL here, as the
//! envelope's apply would have written it.

#![allow(dead_code)]

#[path = "support/artifacts.rs"]
mod artifacts;
mod common;

use artifacts::{key_of, publish_attachment, publish_batch, FakeService, Published};
use core_ledger::AttachmentId;
use db_worker::sync_artifacts::create_attachment_from_staged;
use db_worker::sync_live_closure::{
    apply_delta, load, rebuild_for_epoch, seed, LiveClosureError, LiveEntry,
};
use db_worker::sync_receive::{receive_delta, ReceiveError};
use db_worker::DbWorker;
use sync_core::artifact_closure::{LiveClosure, LogicalRef, Mismatch, PayloadRef};
use sync_core::envelope::{BoundArtifact, BoundManifest};
use sync_core::source_batch::{BatchRow, RecordRow, RunRow, SourceBatch};
use vault_crypto::generate_dek;

const PDF: &[u8] = b"%PDF-1.4\nthe live attachment\n%%EOF\n";
const PNG: &[u8] = b"\x89PNG\r\n\x1a\nthe second attachment";
const ATT: [u8; 16] = [0x07; 16];
const ATT2: [u8; 16] = [0x08; 16];
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

    fn table(&self) -> Vec<String> {
        let conn = self.worker.read_connection().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT key_epoch, hex(sync_artifact_id), hex(ciphertext_sha256), logical_kind,
                        hex(logical_id)
                   FROM sync_live_closure ORDER BY key_epoch, sync_artifact_id",
            )
            .unwrap();
        stmt.query_map([], |r| {
            Ok(format!(
                "{}|{}|{}|{}|{}",
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
    }

    fn live(&self, epoch: u64) -> LiveClosure {
        self.worker.live_closure(epoch).unwrap()
    }

    /// Receive a delta the way the engine would: judged against this vault's own live closure.
    fn receive(
        &self,
        service: &FakeService,
        published: &[Published],
        refs: &[PayloadRef],
    ) -> Result<(), ReceiveError> {
        let key = key_of(1, 0);
        receive_delta(
            &self.worker,
            service,
            &key,
            &manifest(published),
            refs,
            &self.live(1),
        )
        .map(|_| ())
    }

    /// The effect of an accepted attach command on class 1 (what the apply would write).
    fn attach(&self, tx: &rusqlite::Connection, attachment: [u8; 16], artifact: &Published) {
        create_attachment_from_staged(
            tx,
            1,
            artifact.id.as_bytes(),
            AttachmentId::from_uuid(uuid::Uuid::from_bytes(attachment)),
            None,
            "t",
        )
        .unwrap();
    }

    /// The effect of an accepted commit that references the batch's record: a provenance link.
    fn link(&self, tx: &rusqlite::Connection) {
        tx.execute(
            "INSERT INTO source_provenance_links
                 (id, entity_type, entity_id, source_record_id, relationship, created_at)
             VALUES (?1, 'ledger_transaction', ?2, ?3, 'created_from', 't')",
            rusqlite::params![
                [0xc0_u8; 16].as_slice(),
                [0x99_u8; 16].as_slice(),
                RECORD.as_slice()
            ],
        )
        .unwrap();
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

fn entries(attachments: &[([u8; 16], Published)], batch: &[Published]) -> Vec<LiveEntry> {
    let mut out: Vec<LiveEntry> = attachments
        .iter()
        .map(|(id, p)| LiveEntry {
            id: p.id,
            ciphertext_sha256: p.hash,
            logical: LogicalRef::Attachment(*id),
        })
        .collect();
    out.extend(batch.iter().map(|p| LiveEntry {
        id: p.id,
        ciphertext_sha256: p.hash,
        logical: LogicalRef::Batch(BATCH),
    }));
    out
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

/// The artifacts of the delta at N - 1: an attachment and a batch.
struct Scenario {
    service: FakeService,
    attachment: Published,
    parts: Vec<Published>,
}

fn scenario() -> Scenario {
    let (service, key) = (FakeService::default(), key_of(1, 0));
    let attachment = publish_attachment(&service, &key, 0x51, ATT, PDF, "application/pdf");
    let parts = publish_batch(&service, &key, 0x60, &batch());
    Scenario {
        service,
        attachment,
        parts,
    }
}

fn refs_at_n_minus_1() -> [PayloadRef; 2] {
    [PayloadRef::Attachment(ATT), PayloadRef::Record(RECORD)]
}

/// A replica that tailed the log: the delta at N - 1 received, its command applied, and the table
/// maintained in that apply's transaction.
fn tailed(s: &Scenario) -> Vault {
    let vault = Vault::new();
    let delta: Vec<Published> = [vec![s.attachment], s.parts.clone()].concat();
    vault
        .receive(&s.service, &delta, &refs_at_n_minus_1())
        .unwrap();
    vault
        .worker
        .write_sync_state(|tx| -> Result<(), LiveClosureError> {
            vault.attach(tx, ATT, &s.attachment);
            vault.link(tx);
            apply_delta(tx, 1, &entries(&[(ATT, s.attachment)], &s.parts))
        })
        .unwrap();
    vault
}

/// A replica bootstrapped at N - 1: the same class-1 state, the table seeded from the closure.
fn bootstrapped(s: &Scenario) -> Vault {
    let vault = Vault::new();
    let delta: Vec<Published> = [vec![s.attachment], s.parts.clone()].concat();
    vault
        .receive(&s.service, &delta, &refs_at_n_minus_1())
        .unwrap();
    vault
        .worker
        .write_sync_state(|tx| -> Result<(), LiveClosureError> {
            vault.attach(tx, ATT, &s.attachment);
            vault.link(tx);
            seed(tx, 1, &entries(&[(ATT, s.attachment)], &s.parts))
        })
        .unwrap();
    vault
}

/// 03a's R2 scenario: a replica that tailed the log and one bootstrapped at N - 1 hold identical
/// rows and reach the same verdicts: both refuse the same rebinding delta with `Rebinding` and
/// accept the same honest one.
#[test]
fn a_tailed_replica_and_a_bootstrapped_one_hold_identical_rows_and_reach_the_same_verdicts() {
    let s = scenario();
    let (tail, boot) = (tailed(&s), bootstrapped(&s));
    assert_eq!(tail.table(), boot.table(), "identical rows");
    assert_eq!(tail.table().len(), 1 + s.parts.len());
    assert_eq!(tail.live(1), boot.live(1));
    // The T2 record map comes from the batch's own rows.
    assert_eq!(tail.live(1).batch_of_record(&RECORD), Some(BATCH));

    // A delta at N that binds the live attachment to a DIFFERENT artifact id.
    let key = key_of(1, 0);
    let rebound = publish_attachment(&s.service, &key, 0x52, ATT, PDF, "application/pdf");
    let verdict = |vault: &Vault| {
        vault
            .receive(&s.service, &[rebound], &[PayloadRef::Attachment(ATT)])
            .unwrap_err()
    };
    for vault in [&tail, &boot] {
        match verdict(vault) {
            ReceiveError::Mismatch(Mismatch::Rebinding(LogicalRef::Attachment(id))) => {
                assert_eq!(id, ATT);
            }
            other => panic!("{other:?}"),
        }
    }

    // An honest delta at N (a new attachment) is accepted by both.
    let honest = publish_attachment(&s.service, &key, 0x53, ATT2, PNG, "image/png");
    for vault in [&tail, &boot] {
        vault
            .receive(&s.service, &[honest], &[PayloadRef::Attachment(ATT2)])
            .unwrap();
    }

    // (b) After N, the tailed replica's table (maintained) equals a seed from the snapshot closure
    // at N.
    tail.worker
        .write_sync_state(|tx| -> Result<(), LiveClosureError> {
            tail.attach(tx, ATT2, &honest);
            apply_delta(tx, 1, &entries(&[(ATT2, honest)], &[]))
        })
        .unwrap();
    let at_n = Vault::new();
    at_n.worker
        .seed_live_closure(
            1,
            &entries(&[(ATT, s.attachment), (ATT2, honest)], &s.parts),
        )
        .unwrap();
    assert_eq!(
        tail.table(),
        at_n.table(),
        "a seed at N equals the table maintained to N"
    );
}

/// (c) The rebuild at a key epoch leaves exactly the re-addressed closure, in the new epoch, and
/// removes the rows of every other epoch.
#[test]
fn a_rebuild_for_an_epoch_leaves_exactly_that_closure() {
    let s = scenario();
    let vault = tailed(&s);
    let key2 = key_of(2, 0);
    let readdressed = publish_attachment(&s.service, &key2, 0x71, ATT, PDF, "application/pdf");
    vault
        .worker
        .rebuild_live_closure_for_epoch(
            2,
            &[LiveEntry {
                id: readdressed.id,
                ciphertext_sha256: readdressed.hash,
                logical: LogicalRef::Attachment(ATT),
            }],
        )
        .unwrap();
    let table = vault.table();
    assert_eq!(table.len(), 1);
    assert!(table[0].starts_with("2|"), "{table:?}");
    assert!(vault.live(1).get(&LogicalRef::Attachment(ATT)).is_none());
    assert!(vault.live(2).get(&LogicalRef::Attachment(ATT)).is_some());
}

/// The prune: an object that class-1 reachability no longer holds leaves the table in the apply's
/// transaction (adds first, then the prune), and one it still holds stays.
#[test]
fn an_apply_removes_what_reachability_no_longer_holds_and_keeps_the_rest() {
    let s = scenario();
    let vault = tailed(&s);
    assert!(!vault.table().is_empty());

    // The accepted envelope deletes the attachment's row and the provenance link that reached the
    // batch; the same transaction then maintains the table.
    vault
        .worker
        .write_sync_state(|tx| -> Result<(), LiveClosureError> {
            tx.execute("DELETE FROM attachments", [])?;
            apply_delta(tx, 1, &[])
        })
        .unwrap();
    let table = vault.table();
    assert_eq!(
        table.len(),
        s.parts.len(),
        "the attachment left, the batch (still linked) stayed: {table:?}"
    );
    vault
        .worker
        .write_sync_state(|tx| -> Result<(), LiveClosureError> {
            tx.execute("DELETE FROM source_provenance_links", [])?;
            apply_delta(tx, 1, &[])
        })
        .unwrap();
    assert!(vault.table().is_empty(), "no link reaches the batch now");

    // Adds come first, then the prune: an entry for an object nothing reaches is gone at once.
    vault
        .worker
        .write_sync_state(|tx| -> Result<(), LiveClosureError> {
            apply_delta(tx, 1, &entries(&[(ATT, s.attachment)], &[]))
        })
        .unwrap();
    assert!(vault.table().is_empty());
}

/// (e) An apply that fails after `apply_delta` rolls the table back with the command.
#[test]
fn an_apply_that_fails_after_the_delta_rolls_the_table_back_with_the_command() {
    let s = scenario();
    let vault = Vault::new();
    let delta: Vec<Published> = [vec![s.attachment], s.parts.clone()].concat();
    vault
        .receive(&s.service, &delta, &refs_at_n_minus_1())
        .unwrap();
    let failed = vault
        .worker
        .write_sync_state(|tx| -> Result<(), LiveClosureError> {
            vault.attach(tx, ATT, &s.attachment);
            vault.link(tx);
            apply_delta(tx, 1, &entries(&[(ATT, s.attachment)], &s.parts))?;
            Err(LiveClosureError::Malformed) // the apply fails after the delta
        });
    assert!(failed.is_err());
    assert!(vault.table().is_empty(), "the table rolled back");
    let attachments: i64 = vault
        .worker
        .read_connection()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM attachments", [], |r| r.get(0))
        .unwrap();
    assert_eq!(attachments, 0, "and so did the command's own write");
}

/// (d) A local command writes nothing to the table, and while the outbox is not empty every writer
/// refuses: a prune beside an unpushed delete would drop an object the accepted log still holds
/// live. A rebase scratch starts from the base image, which carries the table, so its verdicts are
/// the tailed replica's.
#[test]
fn a_pending_local_command_leaves_the_table_alone_and_the_scratch_reaches_the_tailed_verdict() {
    let s = scenario();
    let vault = tailed(&s);
    let before = vault.table();

    // Sync is enabled on this device: a base exists at the accepted position.
    vault.worker.initialize_base_image(0).unwrap();
    let base = db_worker::base_image::base_image_path(vault.worker.db_path());

    // A local command that deletes the live attachment, with its unpushed outbox row. Local
    // commands never call the table's writers.
    {
        let conn = vault.worker.read_connection().unwrap();
        conn.execute("DELETE FROM attachments", []).unwrap();
        conn.execute(
            "INSERT INTO operation_log (command_id, correlation_id, actor_type, actor_id,
                 idempotency_key, operation_type, affected_entities, node_id, vault_schema_version,
                 hlc_timestamp, created_at, payload, payload_schema_version, write_set_hash,
                 engine_version)
             VALUES (zeroblob(16), zeroblob(16), 'user', 'u', 'k', 'delete_attachment', '[]',
                     zeroblob(16), 74, 1, 't', x'00', 1, zeroblob(32), 'e')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sync_outbox (command_id, op_seq, state, write_set_keys, created_at, issued_at)
             VALUES (zeroblob(16), (SELECT MAX(op_seq) FROM operation_log), 'Pending', x'00', 't', 1)",
            [],
        )
        .unwrap();
    }
    assert_eq!(vault.table(), before, "a local command wrote nothing");

    // Every writer refuses while the outbox holds a command.
    for result in [
        vault.worker.write_sync_state(|tx| apply_delta(tx, 1, &[])),
        vault.worker.write_sync_state(|tx| seed(tx, 1, &[])),
        vault
            .worker
            .write_sync_state(|tx| rebuild_for_epoch(tx, 1, &[])),
    ] {
        assert!(
            matches!(result, Err(LiveClosureError::LocalChangesPending)),
            "{result:?}"
        );
    }
    assert_eq!(vault.table(), before, "a refusal changed nothing");

    // The rebase scratch starts from the base image: its table is the tailed replica's, and it
    // reaches the same verdict on a rebinding delta.
    let scratch = vault
        .worker
        .scratch_from_image(&base, 0, &vault.dir.path().join("scratch.db"))
        .unwrap();
    let on_scratch = scratch.worker().live_closure(1).unwrap();
    assert_eq!(
        on_scratch,
        tailed(&s).live(1),
        "the base image carries the table"
    );
    let key = key_of(1, 0);
    let rebound = publish_attachment(&s.service, &key, 0x52, ATT, PDF, "application/pdf");
    let verdict = receive_delta(
        scratch.worker(),
        &s.service,
        &key,
        &manifest(&[rebound]),
        &[PayloadRef::Attachment(ATT)],
        &on_scratch,
    )
    .unwrap_err();
    assert!(
        matches!(
            verdict,
            ReceiveError::Mismatch(Mismatch::Rebinding(LogicalRef::Attachment(_)))
        ),
        "{verdict:?}"
    );
}

/// A seed replaces the epoch's rows: what the snapshot's closure does not carry is not live.
#[test]
fn a_seed_replaces_the_epochs_rows() {
    let s = scenario();
    let vault = tailed(&s);
    assert!(vault.table().len() > 1);
    vault
        .worker
        .seed_live_closure(1, &entries(&[(ATT, s.attachment)], &[]))
        .unwrap();
    assert_eq!(vault.table().len(), 1, "only the seeded rows remain");
    assert!(vault.live(1).get(&LogicalRef::Batch(BATCH)).is_none());
}
