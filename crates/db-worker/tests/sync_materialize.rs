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
use sync_core::artifact_closure::{LiveClosure, LogicalRef};
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
            source_name: Some("private-statement-march.csv".to_owned()),
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
        .materialize_batch(1, &parts(), &batch(), &LiveClosure::new())
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
        .materialize_batch(1, &parts(), &batch(), &LiveClosure::new())
        .unwrap();
    vault
        .worker
        .materialize_batch(1, &parts(), &batch(), &LiveClosure::new())
        .unwrap();
    assert_eq!(vault.count("source_batches"), 1);
    assert_eq!(vault.count("parser_runs"), 1);
    assert_eq!(vault.count("source_records"), 2);
    assert_eq!(vault.count("sync_artifacts"), 2);
}

// ---- T4: the batch rows are judged against the live closure, not against which rows are here --------

const TABLES_OF_CLASS_1: &str = include_str!("../table_classes.toml");

/// Every row of every class-1 and class-1b table, as sorted strings: what "equal class-1 state"
/// means between two replicas.
fn class_one_state(conn: &rusqlite::Connection) -> Vec<(String, Vec<String>)> {
    let mut tables: Vec<String> = Vec::new();
    for block in TABLES_OF_CLASS_1.split("[[table]]").skip(1) {
        let field = |name: &str| {
            block
                .lines()
                .find_map(|l| l.strip_prefix(&format!("{name} = \"")))
                .and_then(|rest| rest.strip_suffix('"'))
                .map(str::to_owned)
        };
        if matches!(field("class").as_deref(), Some("1" | "1b")) {
            let table = field("table").unwrap();
            // The manifest also lists things that are not SQLite tables (the blob directory).
            let is_table: bool = conn
                .query_row(
                    "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    [&table],
                    |r| r.get(0),
                )
                .unwrap();
            if is_table {
                tables.push(table);
            }
        }
    }
    tables.sort();
    tables
        .into_iter()
        .map(|table| {
            let mut stmt = conn.prepare(&format!("SELECT * FROM {table}")).unwrap();
            let columns = stmt.column_count();
            let mut rows: Vec<String> = stmt
                .query_map([], |r| {
                    Ok((0..columns)
                        .map(|i| format!("{:?}", r.get_ref(i).unwrap()))
                        .collect::<Vec<_>>()
                        .join("|"))
                })
                .unwrap()
                .map(Result::unwrap)
                .collect();
            rows.sort();
            (table, rows)
        })
        .collect()
}

/// What materializing a batch may change: the three batch tables.
const BATCH_TABLES: [&str; 3] = ["source_batches", "source_records", "parser_runs"];

type State = Vec<(String, Vec<String>)>;

/// Two replicas end with equal class-1 state: the batch tables are equal between them, and nothing
/// else in any class-1 or class-1b table changed in either (the seeded rows of two fresh vaults
/// differ in timestamps, which is why the rest is compared with each replica's own past).
fn assert_equal_class_one_state(tailing: (&State, &State), bootstrapped: (&State, &State)) {
    for (index, (table, tailing_rows)) in tailing.1.iter().enumerate() {
        let (_, bootstrapped_rows) = &bootstrapped.1[index];
        if BATCH_TABLES.contains(&table.as_str()) {
            assert_eq!(tailing_rows, bootstrapped_rows, "{table}");
        } else {
            assert_eq!(
                tailing_rows, &tailing.0[index].1,
                "{table} changed on the tailing replica"
            );
            assert_eq!(
                bootstrapped_rows, &bootstrapped.0[index].1,
                "{table} changed on the bootstrapped replica"
            );
        }
    }
}

/// The mappings of the parts of the logical batch `id`: address and verified hash.
fn part_mappings(vault: &Vault, id: [u8; 16]) -> Vec<(Vec<u8>, Vec<u8>)> {
    let conn = vault.worker.read_connection().unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT sync_artifact_id, ciphertext_sha256 FROM sync_artifacts
              WHERE kind = 2 AND logical_id = ?1 ORDER BY sync_artifact_id",
        )
        .unwrap();
    stmt.query_map([id.as_slice()], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

const ART3: [u8; 32] = [0x53; 32];
const ART4: [u8; 32] = [0x54; 32];

fn parts_v2() -> [BatchPartRef; 2] {
    [
        BatchPartRef {
            artifact_id: ART3,
            ciphertext_sha256: [0x63; 32],
            index: 0,
            count: 2,
        },
        BatchPartRef {
            artifact_id: ART4,
            ciphertext_sha256: [0x64; 32],
            index: 1,
            count: 2,
        },
    ]
}

/// The same logical batch published again with other rows: record 1 changed, record 2 gone, record 3
/// new, another summary.
fn batch_v2() -> SourceBatch {
    let mut next = batch();
    next.batch.summary_json = Some("{\"republished\":true}".to_owned());
    next.records[0].normalized_json = "{\"n\":100}".to_owned();
    next.records.remove(1);
    next.records.push(RecordRow {
        id: [3; 16],
        normalized_json: "{\"n\":3}".to_owned(),
        external_id: Some("ext-3".to_owned()),
        source_hash: "hash-3".to_owned(),
        ..next.records[0].clone()
    });
    next
}

#[test]
fn a_republished_batch_that_is_not_live_ends_equal_on_a_tailing_and_a_bootstrapped_replica() {
    let none = LiveClosure::new();
    // The tailing replica materialized the first publication when it was live, and kept its rows.
    let tailing = Vault::new();
    tailing
        .worker
        .materialize_batch(1, &parts(), &batch(), &none)
        .unwrap();
    // The bootstrapped replica never had them.
    let bootstrapped = Vault::new();
    let state = |vault: &Vault| class_one_state(&vault.worker.read_connection().unwrap());
    let (tailing_before, bootstrapped_before) = (state(&tailing), state(&bootstrapped));

    for vault in [&tailing, &bootstrapped] {
        vault
            .worker
            .materialize_batch(1, &parts_v2(), &batch_v2(), &none)
            .unwrap();
    }
    assert_equal_class_one_state(
        (&tailing_before, &state(&tailing)),
        (&bootstrapped_before, &state(&bootstrapped)),
    );
    // The batch is the second publication: the old record is gone, the new one is there.
    assert_eq!(tailing.count("source_records"), 2);
    // And the old parts' addresses no longer name this batch.
    assert_eq!(
        part_mappings(&tailing, [0xb1; 16]),
        part_mappings(&bootstrapped, [0xb1; 16])
    );
    assert_eq!(part_mappings(&tailing, [0xb1; 16]).len(), 2);
    assert!(part_mappings(&tailing, [0xb1; 16])
        .iter()
        .all(|(id, _)| id.as_slice() == ART3 || id.as_slice() == ART4));
}

#[test]
fn a_record_id_reused_by_a_batch_that_is_not_live_ends_the_same_on_both_replicas() {
    let none = LiveClosure::new();
    let tailing = Vault::new();
    tailing
        .worker
        .materialize_batch(1, &parts(), &batch(), &none)
        .unwrap();
    let bootstrapped = Vault::new();

    // Another batch carries record 1 again (a different batch, different content).
    let mut other = batch();
    other.batch.id = [0xb2; 16];
    other.runs.clear();
    other.records = vec![RecordRow {
        source_batch_id: [0xb2; 16],
        normalized_json: "{\"n\":\"again\"}".to_owned(),
        ..batch().records[0].clone()
    }];
    let other_parts = [BatchPartRef {
        artifact_id: ART3,
        ciphertext_sha256: [0x63; 32],
        index: 0,
        count: 1,
    }];
    let state = |vault: &Vault| class_one_state(&vault.worker.read_connection().unwrap());
    let (tailing_before, bootstrapped_before) = (state(&tailing), state(&bootstrapped));
    for vault in [&tailing, &bootstrapped] {
        vault
            .worker
            .materialize_batch(1, &other_parts, &other, &none)
            .unwrap();
    }
    // No other class-1 or class-1b table changed on either replica.
    for (before, after) in [
        (&tailing_before, state(&tailing)),
        (&bootstrapped_before, state(&bootstrapped)),
    ] {
        for (index, (table, rows)) in after.iter().enumerate() {
            if !BATCH_TABLES.contains(&table.as_str()) {
                assert_eq!(rows, &before[index].1, "{table}");
            }
        }
    }
    // Record 1 belongs to the new batch on both; the old batch's other rows are a cache the
    // tailing replica keeps.
    let owner = |vault: &Vault| -> Vec<u8> {
        vault
            .worker
            .read_connection()
            .unwrap()
            .query_row(
                "SELECT source_batch_id FROM source_records WHERE id = ?1",
                [[1_u8; 16].as_slice()],
                |r| r.get(0),
            )
            .unwrap()
    };
    assert_eq!(owner(&tailing), [0xb2; 16]);
    assert_eq!(owner(&bootstrapped), [0xb2; 16]);
    assert_eq!(
        tailing.count("source_records"),
        2,
        "record 2 stays as a cache"
    );
    assert_eq!(bootstrapped.count("source_records"), 1);
    let rows_of = |vault: &Vault, batch: [u8; 16]| -> Vec<String> {
        let conn = vault.worker.read_connection().unwrap();
        let mut stmt = conn
            .prepare("SELECT id, external_id, source_hash, normalized_json FROM source_records WHERE source_batch_id = ?1")
            .unwrap();
        stmt.query_map([batch.as_slice()], |r| {
            Ok(format!(
                "{:?}|{:?}|{:?}|{:?}",
                r.get::<_, Vec<u8>>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
    };
    assert_eq!(
        rows_of(&tailing, [0xb2; 16]),
        rows_of(&bootstrapped, [0xb2; 16])
    );
}

fn live_with_batch() -> LiveClosure {
    let mut live = LiveClosure::new();
    live.insert(
        LogicalRef::Batch([0xb1; 16]),
        sync_core::artifact::ArtifactId::from_derived(ART),
        HASH,
    );
    live
}

#[test]
fn the_rows_of_a_live_batch_are_never_touched() {
    let live = live_with_batch();
    let vault = Vault::new();
    // Absent rows are written (a replica that lost them).
    vault
        .worker
        .materialize_batch(1, &parts(), &batch(), &live)
        .unwrap();
    let before = class_one_state(&vault.worker.read_connection().unwrap());
    // Equal rows are left as they are.
    vault
        .worker
        .materialize_batch(1, &parts(), &batch(), &live)
        .unwrap();
    assert_eq!(
        class_one_state(&vault.worker.read_connection().unwrap()),
        before
    );
    // A row that differs is an invariant violation: nothing is written, not even the new record.
    assert!(matches!(
        vault
            .worker
            .materialize_batch(1, &parts_v2(), &batch_v2(), &live),
        Err(MaterializeError::RowsDiffer)
    ));
    assert_eq!(
        class_one_state(&vault.worker.read_connection().unwrap()),
        before
    );
    assert_eq!(part_mappings(&vault, [0xb1; 16]).len(), 2);
    assert!(part_mappings(&vault, [0xb1; 16])
        .iter()
        .all(|(id, _)| id.as_slice() == ART || id.as_slice() == ART2));
}

#[test]
fn a_record_held_by_a_live_batch_is_never_taken_over() {
    // The T2 case that stage 9(b) rejects first: reaching the materializer is an invariant violation.
    let vault = Vault::new();
    vault
        .worker
        .materialize_batch(1, &parts(), &batch(), &LiveClosure::new())
        .unwrap();
    let before = class_one_state(&vault.worker.read_connection().unwrap());
    let mut other = batch();
    other.batch.id = [0xb2; 16];
    other.runs.clear();
    other.records = vec![RecordRow {
        source_batch_id: [0xb2; 16],
        ..batch().records[0].clone()
    }];
    assert!(matches!(
        vault
            .worker
            .materialize_batch(1, &[], &other, &live_with_batch()),
        Err(MaterializeError::RowsDiffer)
    ));
    assert_eq!(
        class_one_state(&vault.worker.read_connection().unwrap()),
        before
    );
}

#[test]
fn a_record_that_a_replicated_row_names_is_never_replaced() {
    let vault = Vault::new();
    vault
        .worker
        .materialize_batch(1, &parts(), &batch(), &LiveClosure::new())
        .unwrap();
    // A provenance link (a replicated, class-1b row) names record 1: the batch is live by definition.
    vault
        .worker
        .read_connection()
        .unwrap()
        .execute(
            "INSERT INTO source_provenance_links
                 (id, entity_type, entity_id, source_record_id, relationship, created_at)
             VALUES (zeroblob(16), 'ledger_transaction', zeroblob(16), ?1, 'created_from', 't')",
            [[1_u8; 16].as_slice()],
        )
        .unwrap();
    let before = class_one_state(&vault.worker.read_connection().unwrap());

    // Replacing the batch itself would orphan the link.
    assert!(matches!(
        vault
            .worker
            .materialize_batch(1, &parts_v2(), &batch_v2(), &LiveClosure::new()),
        Err(MaterializeError::LiveReference)
    ));
    // So would taking over the record from another batch.
    let mut other = batch();
    other.batch.id = [0xb2; 16];
    other.runs.clear();
    other.records = vec![RecordRow {
        source_batch_id: [0xb2; 16],
        ..batch().records[0].clone()
    }];
    assert!(matches!(
        vault
            .worker
            .materialize_batch(1, &[], &other, &LiveClosure::new()),
        Err(MaterializeError::LiveReference)
    ));
    assert_eq!(
        class_one_state(&vault.worker.read_connection().unwrap()),
        before
    );
    assert_eq!(
        part_mappings(&vault, [0xb1; 16]).len(),
        2,
        "and the mappings are untouched"
    );
}

// ---- a staged row whose blob is gone ---------------------------------------------------------------

#[test]
fn a_staged_attachment_whose_blob_is_gone_is_materialized_again() {
    let vault = Vault::new();
    let (service, key) = (FakeService::default(), key_of(1, 0));
    let (artifact, hash, id) = artifact(&service, &key, 0x51, ATT, PDF);
    vault
        .worker
        .materialize_attachment(1, &hash, &artifact)
        .unwrap();
    let first = lookup(&vault.worker.read_connection().unwrap(), 1, &id)
        .unwrap()
        .unwrap()
        .staged
        .unwrap();
    // A restore returned the row without its blob (or the file was lost).
    fs::remove_file(vault.blob_path(PDF)).unwrap();

    vault
        .worker
        .materialize_attachment(1, &hash, &artifact)
        .unwrap();
    assert!(vault.blob_path(PDF).is_file(), "the blob is written again");
    let again = lookup(&vault.worker.read_connection().unwrap(), 1, &id)
        .unwrap()
        .unwrap()
        .staged
        .unwrap();
    assert_ne!(
        again.wrapped_content_key, first.wrapped_content_key,
        "under a fresh content key, because the old one opens nothing"
    );
    // And the staged key opens the new blob.
    let conn = vault.worker.read_connection().unwrap();
    let attachment = core_ledger::AttachmentId::from_uuid(uuid::Uuid::from_bytes(ATT));
    create_attachment_from_staged(&conn, 1, &id, attachment, None, "t").unwrap();
    assert_eq!(vault.worker.read_attachment_bytes(attachment).unwrap(), PDF);
}

// ---- the startup sweep of orphaned blobs (D7) --------------------------------------------------------

#[test]
fn the_sweep_removes_what_nothing_names_and_keeps_what_a_row_or_a_staging_names() {
    let vault = Vault::new();
    let (service, key) = (FakeService::default(), key_of(1, 0));
    // A blob an attachments row names, and one a staged mapping names.
    let imported = vault
        .worker
        .import_attachment(b"an imported attachment", None, None)
        .unwrap();
    let staged_bytes = b"%PDF-1.4\nstaged for the attach command\n%%EOF\n";
    let (artifact, hash, _) = self::artifact(&service, &key, 0x51, ATT, staged_bytes);
    vault
        .worker
        .materialize_attachment(1, &hash, &artifact)
        .unwrap();
    // Two orphans: a blob whose row was never recorded, and a stale temporary file.
    fs::write(
        vault.blobs().join("0".repeat(64)),
        b"ciphertext whose key was never committed",
    )
    .unwrap();
    fs::write(
        vault.blobs().join(format!("{}.tmp", "1".repeat(64))),
        b"half written",
    )
    .unwrap();

    assert_eq!(vault.worker.sweep_orphan_blobs().unwrap(), 2);
    assert!(!vault.blobs().join("0".repeat(64)).exists());
    assert!(!vault
        .blobs()
        .join(format!("{}.tmp", "1".repeat(64)))
        .exists());
    assert_eq!(
        vault.worker.read_attachment_bytes(imported).unwrap(),
        b"an imported attachment"
    );
    assert!(
        vault.blob_path(staged_bytes).is_file(),
        "the staged blob is kept"
    );
    // A second sweep finds nothing.
    assert_eq!(vault.worker.sweep_orphan_blobs().unwrap(), 0);
}

#[test]
fn the_sweep_does_nothing_while_a_rotation_is_interrupted() {
    let vault = Vault::new();
    fs::create_dir_all(vault.blobs()).unwrap();
    // A crashed rotation leaves new-name blobs that the database does not name yet, and its journal.
    let new_name = vault.blobs().join("2".repeat(64));
    fs::write(&new_name, b"a re-addressed blob linked before the swap").unwrap();
    let journal = vault.dir.path().join("vault.db.rekey");
    fs::write(&journal, b"{}").unwrap();

    assert_eq!(vault.worker.sweep_orphan_blobs().unwrap(), 0);
    assert!(new_name.is_file(), "the new-name blob survives the sweep");

    // Once recovery has cleaned the rotation up, the same blob is an orphan like any other.
    fs::remove_file(&journal).unwrap();
    assert_eq!(vault.worker.sweep_orphan_blobs().unwrap(), 1);
    assert!(!new_name.exists());
    // Each artifact of an interrupted rotation counts, not only the journal.
    for artifact in [
        "vault.db.rekey.tmp",
        "vault.db.rekey-new",
        "vault.db.envelope.rekey-new",
        "vault.db.rekey-old",
    ] {
        let path = vault.dir.path().join(artifact);
        fs::write(&path, b"x").unwrap();
        fs::write(&new_name, b"orphan").unwrap();
        assert_eq!(vault.worker.sweep_orphan_blobs().unwrap(), 0, "{artifact}");
        fs::remove_file(&path).unwrap();
        fs::remove_file(&new_name).unwrap();
    }
}

#[test]
fn the_sweep_also_stands_down_while_a_rebase_is_in_progress() {
    let vault = Vault::new();
    fs::create_dir_all(vault.blobs()).unwrap();
    let sync = vault.dir.path().join("sync");
    fs::create_dir_all(&sync).unwrap();
    let orphan = vault.blobs().join("3".repeat(64));

    // Each of these means a swap or a rebase is under way or just done: the pre-rebase copy, the base
    // image's capture temp file, and anything the rebase swap names with its reserved prefix (its
    // journal, a scratch vault as a file or a directory).
    let artifacts: [(&str, bool); 5] = [
        ("pre-rebase.db", false),
        ("base.db.tmp", false),
        ("rebase.journal", false),
        ("rebase-scratch.db", false),
        ("rebase-scratch", true),
    ];
    for (name, is_dir) in artifacts {
        let path = sync.join(name);
        if is_dir {
            fs::create_dir(&path).unwrap();
        } else {
            fs::write(&path, b"x").unwrap();
        }
        fs::write(&orphan, b"a blob only the scratch vault names").unwrap();
        assert_eq!(vault.worker.sweep_orphan_blobs().unwrap(), 0, "{name}");
        assert!(orphan.is_file(), "{name}: the blob survives");
        if is_dir {
            fs::remove_dir(&path).unwrap();
        } else {
            fs::remove_file(&path).unwrap();
        }
    }
    // The base image itself, which exists whenever Sync is armed, is not a rebase in progress.
    fs::write(sync.join("base.db"), b"x").unwrap();
    assert_eq!(vault.worker.sweep_orphan_blobs().unwrap(), 1);
    assert!(!orphan.exists());
    // And the predicate is the one place that says so.
    assert!(!db_worker::base_image::swap_or_rebase_in_progress(
        &vault.dir.path().join("vault.db")
    ));
    fs::write(sync.join("pre-rebase.db"), b"x").unwrap();
    assert!(db_worker::base_image::swap_or_rebase_in_progress(
        &vault.dir.path().join("vault.db")
    ));
}

// ---- staging under a batch that is replaced (foreign keys are on) -----------------------------------

/// Put an import in progress under `batch`: a staged transaction and balance per record, a staged
/// account, and a dedupe decision that references both the batch and a staged transaction. The
/// merchant, description and amount are sentinels the log must never show.
fn stage_import(vault: &Vault, batch: [u8; 16], records: &[[u8; 16]]) {
    let conn = vault.worker.read_connection().unwrap();
    for (n, record) in records.iter().enumerate() {
        let tag = batch[0].wrapping_add(n as u8);
        conn.execute(
            "INSERT INTO staged_transactions
                 (id, source_record_id, posted_at, amount_minor, currency, normalized_merchant,
                  description, txn_fingerprint, created_at)
             VALUES (?1, ?2, '2026-11-01', 987654, 'USD', 'SENTINEL MERCHANT',
                     'SENTINEL DESCRIPTION', ?3, 't')",
            rusqlite::params![
                [tag, 1_u8].repeat(8),
                record.as_slice(),
                format!("fp-{tag}")
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO staged_balances
                 (id, source_record_id, observed_at, balance_minor, currency, created_at)
             VALUES (?1, ?2, '2026-11-01', 555444, 'USD', 't')",
            rusqlite::params![[tag, 2_u8].repeat(8), record.as_slice()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO dedupe_decisions
                 (id, source_batch_id, layer, staged_transaction_id, decision, reason, decided_at)
             VALUES (?1, ?2, 'transaction', ?3, 'flagged', 'r', 't')",
            rusqlite::params![
                [tag, 3_u8].repeat(8),
                batch.as_slice(),
                [tag, 1_u8].repeat(8)
            ],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO staged_accounts (id, source_batch_id, external_name, created_at)
         VALUES (?1, ?2, 'SENTINEL ACCOUNT', 't')",
        rusqlite::params![[batch[0], 4_u8].repeat(8), batch.as_slice()],
    )
    .unwrap();
}

/// The staging rows (class 3 and the class-4 decisions) that belong to `batch`.
fn staging_of(vault: &Vault, batch: [u8; 16]) -> [i64; 4] {
    let conn = vault.worker.read_connection().unwrap();
    let count = |sql: &str| -> i64 {
        conn.query_row(sql, [batch.as_slice()], |r| r.get(0))
            .unwrap()
    };
    [
        count(
            "SELECT COUNT(*) FROM staged_transactions WHERE source_record_id IN
                 (SELECT id FROM source_records WHERE source_batch_id = ?1)",
        ),
        count(
            "SELECT COUNT(*) FROM staged_balances WHERE source_record_id IN
                 (SELECT id FROM source_records WHERE source_batch_id = ?1)",
        ),
        count("SELECT COUNT(*) FROM staged_accounts WHERE source_batch_id = ?1"),
        count("SELECT COUNT(*) FROM dedupe_decisions WHERE source_batch_id = ?1"),
    ]
}

fn foreign_key_violations(vault: &Vault) -> usize {
    let conn = vault.worker.read_connection().unwrap();
    let mut stmt = conn.prepare("PRAGMA foreign_key_check").unwrap();
    stmt.query_map([], |_| Ok(())).unwrap().count()
}

fn everything(vault: &Vault) -> Vec<(String, Vec<String>)> {
    let conn = vault.worker.read_connection().unwrap();
    let mut tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    tables.sort();
    tables
        .into_iter()
        .map(|table| {
            let mut stmt = conn.prepare(&format!("SELECT * FROM {table}")).unwrap();
            let columns = stmt.column_count();
            let mut rows: Vec<String> = stmt
                .query_map([], |r| {
                    Ok((0..columns)
                        .map(|i| format!("{:?}", r.get_ref(i).unwrap()))
                        .collect::<Vec<_>>()
                        .join("|"))
                })
                .unwrap()
                .map(Result::unwrap)
                .collect();
            rows.sort();
            (table, rows)
        })
        .collect()
}

#[test]
fn an_identical_re_reference_with_staging_present_changes_nothing() {
    let vault = Vault::new();
    let none = LiveClosure::new();
    vault
        .worker
        .materialize_batch(1, &parts(), &batch(), &none)
        .unwrap();
    stage_import(&vault, [0xb1; 16], &[[1; 16], [2; 16]]);
    let before = everything(&vault);
    assert_eq!(staging_of(&vault, [0xb1; 16]), [2, 2, 1, 2]);

    // The same envelope delivered again, or an honest producer re-referencing the batch.
    let discarded = vault
        .worker
        .materialize_batch(1, &parts(), &batch(), &none)
        .unwrap();
    assert!(discarded.is_empty());
    assert_eq!(
        everything(&vault),
        before,
        "not one row of any table changed"
    );
    assert!(vault.worker.sync_notices().unwrap().is_empty());
    assert_eq!(foreign_key_violations(&vault), 0);
}

#[test]
fn a_changed_or_dropped_row_discards_that_batchs_staging_and_only_that_batchs() {
    for variant in ["changed", "dropped"] {
        let vault = Vault::new();
        let none = LiveClosure::new();
        vault
            .worker
            .materialize_batch(1, &parts(), &batch(), &none)
            .unwrap();
        // A second import in progress, for another batch, that must survive.
        let mut other = batch();
        other.batch.id = [0xb9; 16];
        other.runs.clear();
        other.records = vec![RecordRow {
            id: [9; 16],
            source_batch_id: [0xb9; 16],
            ..batch().records[0].clone()
        }];
        vault
            .worker
            .materialize_batch(1, &[], &other, &none)
            .unwrap();
        stage_import(&vault, [0xb1; 16], &[[1; 16], [2; 16]]);
        stage_import(&vault, [0xb9; 16], &[[9; 16]]);
        let other_before = staging_of(&vault, [0xb9; 16]);

        // The forged re-publication: record 1 has other content ("changed"), or record 2 is gone
        // ("dropped"). Both are inconsistent with what the user was reviewing.
        let mut forged = batch();
        match variant {
            "changed" => forged.records[0].normalized_json = "{\"n\":100}".to_owned(),
            _ => {
                forged.records.remove(1);
            }
        }
        let discarded = vault
            .worker
            .materialize_batch(1, &parts_v2(), &forged, &none)
            .unwrap();

        assert_eq!(discarded.len(), 1, "{variant}");
        assert_eq!(discarded[0].batch_id, [0xb1; 16]);
        assert_eq!(
            [
                discarded[0].staged_transactions,
                discarded[0].staged_balances,
                discarded[0].staged_accounts,
                discarded[0].dedupe_decisions
            ],
            [2, 2, 1, 2],
            "{variant}: ALL of the batch's staging, so a half-import cannot be committed"
        );
        assert_eq!(staging_of(&vault, [0xb1; 16]), [0, 0, 0, 0], "{variant}");
        assert_eq!(
            staging_of(&vault, [0xb9; 16]),
            other_before,
            "{variant}: another batch is untouched"
        );
        assert_eq!(foreign_key_violations(&vault), 0, "{variant}");

        // The rows are the forged batch's, in place.
        let conn = vault.worker.read_connection().unwrap();
        let json: Option<String> = conn
            .query_row(
                "SELECT normalized_json FROM source_records WHERE id = ?1",
                [[1_u8; 16].as_slice()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            json.as_deref(),
            Some(if variant == "changed" {
                "{\"n\":100}"
            } else {
                "{\"n\":1}"
            })
        );
        let record_two: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM source_records WHERE id = ?1",
                [[2_u8; 16].as_slice()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(record_two, if variant == "changed" { 1 } else { 0 });

        // The user is told, naming their file.
        let notices = vault.worker.sync_notices().unwrap();
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].batch_id, [0xb1; 16]);
        assert_eq!(
            notices[0].source_name.as_deref(),
            Some("private-statement-march.csv")
        );
        assert_eq!(notices[0].discarded_count, 7);
    }
}

#[test]
fn a_batch_that_loses_a_record_to_another_loses_its_staging_too() {
    let vault = Vault::new();
    let none = LiveClosure::new();
    vault
        .worker
        .materialize_batch(1, &parts(), &batch(), &none)
        .unwrap();
    stage_import(&vault, [0xb1; 16], &[[1; 16], [2; 16]]);

    // Another batch carries record 1 (a takeover). The batch that lost it is a cache and cannot be
    // committed consistently any more.
    let mut other = batch();
    other.batch.id = [0xb2; 16];
    other.batch.source_name = Some("the-other-file.csv".to_owned());
    other.runs.clear();
    other.records = vec![RecordRow {
        source_batch_id: [0xb2; 16],
        ..batch().records[0].clone()
    }];
    let discarded = vault
        .worker
        .materialize_batch(1, &[], &other, &none)
        .unwrap();
    assert_eq!(discarded.len(), 1);
    assert_eq!(
        discarded[0].batch_id, [0xb1; 16],
        "the batch that lost the record"
    );
    assert_eq!(staging_of(&vault, [0xb1; 16]), [0, 0, 0, 0]);
    assert_eq!(foreign_key_violations(&vault), 0);
    let notices = vault.worker.sync_notices().unwrap();
    assert_eq!(notices.len(), 1);
    assert_eq!(
        notices[0].source_name.as_deref(),
        Some("private-statement-march.csv")
    );
}

#[test]
fn the_notice_is_durable_device_local_and_can_be_dismissed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.db");
    let dek = generate_dek().unwrap();
    {
        let vault = Vault {
            dir: tempfile::tempdir().unwrap(),
            dek: duplicate(&dek),
            worker: DbWorker::open_with_raw_key(&path, duplicate(&dek)).unwrap(),
        };
        let none = LiveClosure::new();
        vault
            .worker
            .materialize_batch(1, &parts(), &batch(), &none)
            .unwrap();
        stage_import(&vault, [0xb1; 16], &[[1; 16]]);
        let mut forged = batch();
        forged.records[0].normalized_json = "{\"n\":100}".to_owned();
        vault
            .worker
            .materialize_batch(1, &parts_v2(), &forged, &none)
            .unwrap();
        assert_eq!(vault.worker.sync_notices().unwrap().len(), 1);
    }
    // After a restart it is still there.
    let worker = DbWorker::open_existing_with_raw_key(&path, duplicate(&dek)).unwrap();
    let notices = worker.sync_notices().unwrap();
    assert_eq!(notices.len(), 1);
    // Its Debug shows no file name.
    assert!(!format!("{:?}", notices[0]).contains("private-statement"));
    assert!(worker
        .dismiss_sync_notice(notices[0].id, "2026-11-08T00:00:00Z")
        .unwrap());
    assert!(
        !worker
            .dismiss_sync_notice(notices[0].id, "2026-11-08T00:00:01Z")
            .unwrap(),
        "once"
    );
    assert!(worker.sync_notices().unwrap().is_empty());
}

#[test]
fn the_log_line_carries_counts_and_nothing_the_user_typed_or_imported() {
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Buffer(Arc<Mutex<Vec<u8>>>);
    impl Write for Buffer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Buffer {
        type Writer = Buffer;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    // DoD §2.4 rule 1: the global test subscriber first, so events on other threads are not lost.
    observability::testing::install_global_test_subscriber();
    let vault = Vault::new();
    let none = LiveClosure::new();
    vault
        .worker
        .materialize_batch(1, &parts(), &batch(), &none)
        .unwrap();
    stage_import(&vault, [0xb1; 16], &[[1; 16], [2; 16]]);
    let mut forged = batch();
    forged.records[0].normalized_json = "{\"n\":100}".to_owned();

    let buffer = Buffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(buffer.clone())
        .with_ansi(false)
        .finish();
    tracing::subscriber::with_default(subscriber, || {
        vault
            .worker
            .materialize_batch(1, &parts_v2(), &forged, &none)
            .unwrap();
    });
    let logged = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
    assert_eq!(logged.lines().count(), 1, "one line: {logged}");
    for field in [
        "staged_transactions=2",
        "staged_balances=2",
        "staged_accounts=1",
        "dedupe_decisions=2",
    ] {
        assert!(logged.contains(field), "{field} in {logged}");
    }
    // Not the file name, merchant, description, amounts, or any id.
    for forbidden in [
        "private-statement",
        "csv",
        "SENTINEL",
        "987654",
        "555444",
        "b1b1b1",
        "177",
    ] {
        assert!(!logged.contains(forbidden), "{forbidden} in {logged}");
    }
}

#[test]
fn replicas_converge_when_a_forged_delta_hits_a_device_with_pending_staging() {
    let none = LiveClosure::new();
    let tailing = Vault::new();
    tailing
        .worker
        .materialize_batch(1, &parts(), &batch(), &none)
        .unwrap();
    stage_import(&tailing, [0xb1; 16], &[[1; 16], [2; 16]]);
    let bootstrapped = Vault::new();
    let state = |vault: &Vault| class_one_state(&vault.worker.read_connection().unwrap());
    let (tailing_before, bootstrapped_before) = (state(&tailing), state(&bootstrapped));

    for vault in [&tailing, &bootstrapped] {
        vault
            .worker
            .materialize_batch(1, &parts_v2(), &batch_v2(), &none)
            .unwrap();
    }
    assert_equal_class_one_state(
        (&tailing_before, &state(&tailing)),
        (&bootstrapped_before, &state(&bootstrapped)),
    );
    // Only the device that had an import in progress was told, and nothing is left dangling.
    assert_eq!(tailing.worker.sync_notices().unwrap().len(), 1);
    assert!(bootstrapped.worker.sync_notices().unwrap().is_empty());
    assert_eq!(foreign_key_violations(&tailing), 0);
}

#[test]
fn the_sweep_keeps_a_blob_that_only_the_base_image_names() {
    let vault = Vault::new();
    let bytes = b"an attachment the base image remembers";
    let id = vault.worker.import_attachment(bytes, None, None).unwrap();
    // The base image is taken while the attachment exists; it is the rollback target of a rebase.
    vault.worker.initialize_base_image(0).unwrap();
    assert!(vault.dir.path().join("sync").join("base.db").is_file());
    // The attachment is then deleted here, so only the base image names its blob.
    let conn = vault.worker.read_connection().unwrap();
    conn.execute(
        "DELETE FROM attachment_links WHERE attachment_id = ?1",
        [id.as_uuid()],
    )
    .unwrap();
    conn.execute("DELETE FROM attachments WHERE id = ?1", [id.as_uuid()])
        .unwrap();
    let orphan = vault.blobs().join("4".repeat(64));
    fs::write(&orphan, b"ciphertext whose key was never committed").unwrap();

    assert_eq!(
        vault.worker.sweep_orphan_blobs().unwrap(),
        1,
        "only the unnamed blob"
    );
    assert!(
        vault.blob_path(bytes).is_file(),
        "the base image's blob survives"
    );
    assert!(!orphan.exists());
}

#[test]
fn the_sweep_removes_nothing_when_the_base_image_exists_and_cannot_be_read() {
    let vault = Vault::new();
    let sync = vault.dir.path().join("sync");
    fs::create_dir_all(&sync).unwrap();
    fs::write(sync.join("base.db"), b"this is not a database").unwrap();
    fs::create_dir_all(vault.blobs()).unwrap();
    let orphan = vault.blobs().join("5".repeat(64));
    fs::write(&orphan, b"an orphan").unwrap();

    assert!(
        vault.worker.sweep_orphan_blobs().is_err(),
        "it fails closed and says so"
    );
    assert!(orphan.is_file(), "and removes nothing");
}

/// A committed dedupe decision, as `CommitImportedTransaction`'s apply writes it: no staged
/// transaction, replicated, present on every replica.
fn committed_decision(vault: &Vault, batch: [u8; 16]) {
    vault
        .worker
        .read_connection()
        .unwrap()
        .execute(
            "INSERT INTO dedupe_decisions
                 (id, source_batch_id, layer, staged_transaction_id, decision, reason, decided_at)
             VALUES (?1, ?2, 'transaction', NULL, 'committed', 'imported', 't')",
            rusqlite::params![[0xc0_u8, batch[0]].repeat(8), batch.as_slice()],
        )
        .unwrap();
}

fn committed_decisions(vault: &Vault, batch: [u8; 16]) -> i64 {
    vault
        .worker
        .read_connection()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM dedupe_decisions
              WHERE source_batch_id = ?1 AND staged_transaction_id IS NULL",
            [batch.as_slice()],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn a_committed_dedupe_decision_is_replicated_data_and_survives_the_discard() {
    let none = LiveClosure::new();
    let mut forged = batch();
    forged.records[0].normalized_json = "{\"n\":100}".to_owned();

    // A replica with no staging at all, holding only the committed decision: nothing to discard, so
    // no notice, and the decision stays.
    let replica = Vault::new();
    replica
        .worker
        .materialize_batch(1, &parts(), &batch(), &none)
        .unwrap();
    committed_decision(&replica, [0xb1; 16]);
    let discarded = replica
        .worker
        .materialize_batch(1, &parts_v2(), &forged, &none)
        .unwrap();
    assert!(discarded.is_empty(), "no staging, so nothing was discarded");
    assert!(
        replica.worker.sync_notices().unwrap().is_empty(),
        "and nobody is told"
    );
    assert_eq!(
        committed_decisions(&replica, [0xb1; 16]),
        1,
        "the replicated decision stays"
    );

    // A device that has both: only the decisions about its staged transactions go.
    let importing = Vault::new();
    importing
        .worker
        .materialize_batch(1, &parts(), &batch(), &none)
        .unwrap();
    stage_import(&importing, [0xb1; 16], &[[1; 16], [2; 16]]);
    committed_decision(&importing, [0xb1; 16]);
    let discarded = importing
        .worker
        .materialize_batch(1, &parts_v2(), &forged, &none)
        .unwrap();
    assert_eq!(discarded.len(), 1);
    assert_eq!(
        discarded[0].dedupe_decisions, 2,
        "counts only the staging's decisions"
    );
    assert_eq!(committed_decisions(&importing, [0xb1; 16]), 1);
    assert_eq!(foreign_key_violations(&importing), 0);
    // And the two replicas hold the same replicated decisions.
    assert_eq!(
        committed_decisions(&replica, [0xb1; 16]),
        committed_decisions(&importing, [0xb1; 16])
    );
}
