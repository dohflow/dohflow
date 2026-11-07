//! The device-local artifact mapping (`sync_artifacts`, vault-format.md B.2 and C.9;
//! `personal-cfo-0g528.2`): recorded and replaced, its staging consumed once, excluded from the
//! genesis export, and re-wrapped and re-addressed by a key rotation (ADR 0083) while staged.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{create_account_cmd, meta};
use db_worker::logical_export::{export_genesis_plaintext, ExportInput};
use db_worker::sync_artifacts::{
    create_attachment_from_staged, lookup, record, record_receipt, take_staged, ArtifactKind,
    CreateAttachmentError, Mapping, StagedAttachment,
};
use db_worker::{verify_rekeyed_copy, DbWorker};
use vault_crypto::{
    decrypt_blob, derive_kek, generate_dek, generate_salt, unwrap_content_key, unwrap_dek,
    wrap_dek, Dek, EncryptedBlob, Profile, WrappedContentKey,
};

const PDF: &[u8] = b"%PDF-1.4\nstaged attachment\n%%EOF\n";
const ID: [u8; 32] = [0x41; 32];
const OTHER: [u8; 32] = [0x42; 32];
const NOW: &str = "2026-11-08T00:00:00Z";

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

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut os = path.as_os_str().to_owned();
    os.push(suffix);
    PathBuf::from(os)
}

fn attachment_mapping(artifact_id: [u8; 32], hash: u8) -> Mapping {
    Mapping {
        key_epoch: 1,
        artifact_id,
        ciphertext_sha256: [hash; 32],
        kind: ArtifactKind::Attachment,
        logical_id: [0x07; 16],
    }
}

fn staged(storage_id: &str) -> StagedAttachment {
    StagedAttachment {
        storage_id: storage_id.to_owned(),
        wrapped_content_key: vec![0xaa; 48],
        content_key_nonce: vec![0xbb; 12],
        content_nonce: vec![0xcc; 12],
        content_alg: "AES-256-GCM".to_owned(),
        plaintext_size: 33,
        mime_type: Some("application/pdf".to_owned()),
    }
}

#[test]
fn a_mapping_is_recorded_replaced_by_a_newer_hash_and_keeps_its_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.db");
    {
        let worker = DbWorker::open_with_raw_key(&path, generate_dek().unwrap()).unwrap();
        let conn = worker.read_connection().unwrap();
        assert_eq!(lookup(&conn, 1, &ID).unwrap(), None);

        record(&conn, &attachment_mapping(ID, 1), None).unwrap();
        let stored = lookup(&conn, 1, &ID).unwrap().unwrap();
        assert_eq!(stored.mapping, attachment_mapping(ID, 1));
        assert_eq!(stored.receipt_at, None);
        assert_eq!(stored.staged, None);

        // A receipt is recorded once; a second one keeps the first time.
        assert!(record_receipt(&conn, 1, &ID, NOW).unwrap());
        assert!(record_receipt(&conn, 1, &ID, "2026-11-09T00:00:00Z").unwrap());
        assert_eq!(
            lookup(&conn, 1, &ID)
                .unwrap()
                .unwrap()
                .receipt_at
                .as_deref(),
            Some(NOW)
        );
        assert!(
            !record_receipt(&conn, 1, &OTHER, NOW).unwrap(),
            "no row, no receipt"
        );

        // The same address under another epoch is another row; a newer verified hash replaces the
        // old one and keeps the receipt.
        record(&conn, &attachment_mapping(ID, 2), None).unwrap();
        let again = lookup(&conn, 1, &ID).unwrap().unwrap();
        assert_eq!(again.mapping.ciphertext_sha256, [2; 32]);
        assert_eq!(again.receipt_at.as_deref(), Some(NOW));
        assert_eq!(lookup(&conn, 2, &ID).unwrap(), None);

        // A batch part keeps its place in the batch.
        let part = Mapping {
            kind: ArtifactKind::BatchPart { index: 2, count: 3 },
            logical_id: [0x09; 16],
            ..attachment_mapping(OTHER, 5)
        };
        record(&conn, &part, None).unwrap();
        assert_eq!(lookup(&conn, 1, &OTHER).unwrap().unwrap().mapping, part);
    }
}

#[test]
fn a_restart_keeps_the_mapping_and_its_staging() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.db");
    let dek = generate_dek().unwrap();
    {
        let worker = DbWorker::open_with_raw_key(&path, duplicate(&dek)).unwrap();
        record(
            &worker.read_connection().unwrap(),
            &attachment_mapping(ID, 1),
            Some(&staged("blob-a")),
        )
        .unwrap();
    }
    let worker = DbWorker::open_existing_with_raw_key(&path, duplicate(&dek)).unwrap();
    let stored = lookup(&worker.read_connection().unwrap(), 1, &ID)
        .unwrap()
        .unwrap();
    assert_eq!(stored.staged, Some(staged("blob-a")));
}

#[test]
fn the_staging_is_consumed_once_and_the_mapping_stays() {
    let dir = tempfile::tempdir().unwrap();
    let worker =
        DbWorker::open_with_raw_key(dir.path().join("vault.db"), generate_dek().unwrap()).unwrap();
    let conn = worker.read_connection().unwrap();
    record(&conn, &attachment_mapping(ID, 1), Some(&staged("blob-a"))).unwrap();

    // What the attach command's apply does, in one transaction with creating the attachments row.
    assert_eq!(take_staged(&conn, 1, &ID).unwrap(), Some(staged("blob-a")));
    let after = lookup(&conn, 1, &ID).unwrap().unwrap();
    assert_eq!(
        after.staged, None,
        "no second long-lived copy of the key material"
    );
    assert_eq!(after.mapping, attachment_mapping(ID, 1));
    let key_columns: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sync_artifacts WHERE staged_storage_id IS NOT NULL
                OR staged_wrapped_content_key IS NOT NULL OR staged_content_key_nonce IS NOT NULL
                OR staged_content_nonce IS NOT NULL OR staged_content_alg IS NOT NULL
                OR staged_plaintext_size IS NOT NULL OR staged_mime_type IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(key_columns, 0);

    // A second take finds nothing, a missing mapping finds nothing, and a batch part has no staging.
    assert_eq!(take_staged(&conn, 1, &ID).unwrap(), None);
    assert_eq!(take_staged(&conn, 1, &OTHER).unwrap(), None);
    // Re-recording the address (a refetch) does not bring a consumed staging back by itself.
    record(&conn, &attachment_mapping(ID, 3), None).unwrap();
    assert_eq!(lookup(&conn, 1, &ID).unwrap().unwrap().staged, None);
}

#[test]
fn key_material_is_not_in_a_debug_rendering() {
    let rendered = format!("{:?} {:#?}", staged("blob-a"), staged("blob-a"));
    for field in ["wrapped_content_key", "content_key_nonce", "content_nonce"] {
        assert!(!rendered.contains(field), "{field} in {rendered}");
    }
    assert!(rendered.contains("blob-a"));
}

#[test]
fn the_genesis_export_excludes_the_mapping() {
    let dir = tempfile::tempdir().unwrap();
    let worker =
        DbWorker::open_with_raw_key(dir.path().join("vault.db"), generate_dek().unwrap()).unwrap();
    worker.dispatch(meta(), create_account_cmd()).unwrap();
    let conn = worker.read_connection().unwrap();
    record(&conn, &attachment_mapping(ID, 1), Some(&staged("blob-a"))).unwrap();

    let plaintext = export_genesis_plaintext(&conn, &ExportInput::default()).unwrap();
    let verified = sync_core::snapshot::verify(&plaintext).unwrap();
    let tables: Vec<&str> = verified
        .manifest
        .tables
        .iter()
        .map(|t| t.name.as_str())
        .collect();
    assert!(tables.contains(&"accounts"), "the export is not empty");
    assert!(
        !tables.contains(&"sync_artifacts"),
        "a device-local table never enters a snapshot: {tables:?}"
    );
    // And no byte of the staged key material is in it.
    assert!(!plaintext.windows(48).any(|w| w == [0xaa; 48]));
}

// ---- rotation (ADR 0083) -------------------------------------------------------------------------

/// A raw-keyed vault with an attachment whose crypto fields have been moved into the mapping's
/// staging, as materialization leaves them before the attach command's `apply` has run: the blob is
/// on disk, and no `attachments` row names it.
fn vault_with_a_staged_attachment() -> (tempfile::TempDir, PathBuf, Dek, DbWorker, String) {
    let (dir, path, dek, worker, storage_id, _) = vault_with_a_staged_attachment_and_id();
    (dir, path, dek, worker, storage_id)
}

fn vault_with_a_staged_attachment_and_id() -> (
    tempfile::TempDir,
    PathBuf,
    Dek,
    DbWorker,
    String,
    core_ledger::AttachmentId,
) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.db");
    let dek = generate_dek().unwrap();
    drop(DbWorker::open_with_raw_key(&path, duplicate(&dek)).unwrap());
    let worker = DbWorker::open_existing_with_raw_key(&path, duplicate(&dek)).unwrap();
    worker.dispatch(meta(), create_account_cmd()).unwrap();
    let id = worker
        .import_attachment(PDF, Some("application/pdf"), Some("one.pdf"))
        .unwrap();
    let conn = worker.read_connection().unwrap();
    let (storage_id, wrapped, key_nonce, content_nonce): (String, Vec<u8>, Vec<u8>, Vec<u8>) = conn
        .query_row(
            "SELECT storage_id, wrapped_content_key, content_key_nonce, content_nonce
               FROM attachments WHERE id = ?1",
            [id.as_uuid()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    record(
        &conn,
        &Mapping {
            logical_id: *id.as_uuid().as_bytes(),
            ..attachment_mapping(ID, 1)
        },
        Some(&StagedAttachment {
            storage_id: storage_id.clone(),
            wrapped_content_key: wrapped,
            content_key_nonce: key_nonce,
            content_nonce,
            content_alg: "AES-256-GCM".to_owned(),
            plaintext_size: PDF.len() as u64,
            mime_type: Some("application/pdf".to_owned()),
        }),
    )
    .unwrap();
    // The attachments row is what apply has not created yet.
    conn.execute(
        "DELETE FROM attachment_links WHERE attachment_id = ?1",
        [id.as_uuid()],
    )
    .unwrap();
    conn.execute("DELETE FROM attachments WHERE id = ?1", [id.as_uuid()])
        .unwrap();
    (dir, path, dek, worker, storage_id, id)
}

#[test]
fn a_rotation_rewraps_and_readdresses_a_staged_attachment_and_nothing_stays_wrapped_under_the_old_key(
) {
    let (dir, path, old_dek, worker, old_storage_id) = vault_with_a_staged_attachment();
    let blobs = dir.path().join("blobs");
    let copy = with_suffix(&path, ".rekey-new");
    let new_dek = generate_dek().unwrap();
    let user_version = worker.user_version().unwrap();

    let renames = worker.prepare_rekeyed_copy(&new_dek, &copy).unwrap();
    assert_eq!(renames.len(), 1, "the staged blob is renamed");
    assert_eq!(renames[0].old, old_storage_id);
    assert_ne!(
        renames[0].new, old_storage_id,
        "re-addressed under the new key"
    );
    // Verification refuses until the new name is linked, exactly as for an attachments row.
    assert!(verify_rekeyed_copy(&copy, &new_dek, &blobs, user_version).is_err());
    fs::hard_link(blobs.join(&renames[0].old), blobs.join(&renames[0].new)).unwrap();
    verify_rekeyed_copy(&copy, &new_dek, &blobs, user_version).unwrap();
    assert!(
        verify_rekeyed_copy(&copy, &old_dek, &blobs, user_version).is_err(),
        "the old key no longer opens the copy"
    );

    // Swap in and reopen under the new key.
    drop(worker);
    for suffix in ["-wal", "-shm"] {
        let _ = fs::remove_file(with_suffix(&path, suffix));
    }
    fs::rename(&copy, &path).unwrap();
    fs::remove_file(blobs.join(&renames[0].old)).unwrap();
    let reopened = DbWorker::open_existing_with_raw_key(&path, duplicate(&new_dek)).unwrap();
    let stored = lookup(&reopened.read_connection().unwrap(), 1, &ID)
        .unwrap()
        .unwrap();
    let fields = stored.staged.expect("still staged: apply has not run");
    assert_eq!(fields.storage_id, renames[0].new);

    // The staged key unwraps under the NEW key only, and opens the re-addressed blob.
    let wrapped = WrappedContentKey {
        nonce: fields.content_key_nonce.clone().try_into().unwrap(),
        ciphertext: fields.wrapped_content_key.clone(),
    };
    assert!(
        unwrap_content_key(&old_dek, &wrapped).is_err(),
        "not wrapped under the old key"
    );
    let content_key = unwrap_content_key(&new_dek, &wrapped).unwrap();
    let blob = EncryptedBlob {
        nonce: fields.content_nonce.clone().try_into().unwrap(),
        ciphertext: fs::read(blobs.join(&fields.storage_id)).unwrap(),
    };
    assert_eq!(decrypt_blob(&content_key, &blob).unwrap(), PDF);
    // The rest of the row is untouched: the same address, hash and kind, and the same attachment.
    assert_eq!(
        stored.mapping,
        Mapping {
            logical_id: stored.mapping.logical_id,
            ..attachment_mapping(ID, 1)
        }
    );
    assert_eq!(fields.plaintext_size, PDF.len() as u64);
    assert_eq!(fields.mime_type.as_deref(), Some("application/pdf"));
}

#[test]
fn a_staged_blob_that_an_attachment_also_holds_is_renamed_once() {
    let (dir, path, _old_dek, worker, old_storage_id) = vault_with_a_staged_attachment();
    // Storage ids are content-addressed, and materialization reuses an existing blob and its key, so
    // an attachments row can hold exactly the fields the staged row holds.
    let conn = worker.read_connection().unwrap();
    conn.execute(
        "INSERT INTO attachments
             (id, storage_id, wrapped_content_key, content_key_nonce, content_nonce, content_alg,
              plaintext_size, mime_type, original_filename, ref_count, created_at)
         SELECT zeroblob(16), staged_storage_id, staged_wrapped_content_key,
                staged_content_key_nonce, staged_content_nonce, staged_content_alg,
                staged_plaintext_size, staged_mime_type, 'shared.pdf', 1, 't'
           FROM sync_artifacts WHERE staged_storage_id = ?1",
        [&old_storage_id],
    )
    .unwrap();

    let copy = with_suffix(&path, ".rekey-new");
    let renames = worker
        .prepare_rekeyed_copy(&generate_dek().unwrap(), &copy)
        .unwrap();
    assert_eq!(renames.len(), 1, "one file, one rename");
    assert_eq!(renames[0].old, old_storage_id);
    drop(dir);
}

// ---- the receiver's attach: the staging becomes the attachments row ---------------------------------

fn attachment_rows(conn: &rusqlite::Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM attachments", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn the_attachments_row_is_created_from_the_staging_and_it_becomes_the_only_copy_of_the_key() {
    let (_dir, _path, _dek, worker, storage_id, id) = vault_with_a_staged_attachment_and_id();
    let conn = worker.read_connection().unwrap();
    let before = lookup(&conn, 1, &ID).unwrap().unwrap().staged.unwrap();
    assert_eq!(attachment_rows(&conn), 0, "apply has not run");

    let tx = conn.unchecked_transaction().unwrap();
    create_attachment_from_staged(&tx, 1, &ID, id, Some("one.pdf"), "2026-11-08T00:00:00Z")
        .unwrap();
    tx.commit().unwrap();

    // The row carries exactly the staged fields.
    let column = |name: &str| -> rusqlite::types::Value {
        conn.query_row(
            &format!("SELECT {name} FROM attachments WHERE id = ?1"),
            [id.as_uuid()],
            |r| r.get(0),
        )
        .unwrap()
    };
    let text = |name: &str| match column(name) {
        rusqlite::types::Value::Text(t) => Some(t),
        rusqlite::types::Value::Null => None,
        other => panic!("{name}: {other:?}"),
    };
    let blob = |name: &str| match column(name) {
        rusqlite::types::Value::Blob(b) => b,
        other => panic!("{name}: {other:?}"),
    };
    let integer = |name: &str| match column(name) {
        rusqlite::types::Value::Integer(i) => i,
        other => panic!("{name}: {other:?}"),
    };
    let stored_storage = text("storage_id").unwrap();
    let wrapped = blob("wrapped_content_key");
    let key_nonce = blob("content_key_nonce");
    let content_nonce = blob("content_nonce");
    let size = integer("plaintext_size");
    let mime = text("mime_type");
    let filename = text("original_filename");
    let refs = integer("ref_count");
    let created = text("created_at").unwrap();
    assert_eq!(stored_storage, storage_id);
    assert_eq!(wrapped, before.wrapped_content_key);
    assert_eq!(key_nonce, before.content_key_nonce);
    assert_eq!(content_nonce, before.content_nonce);
    assert_eq!(size, before.plaintext_size as i64);
    assert_eq!(mime.as_deref(), Some("application/pdf"));
    assert_eq!(filename.as_deref(), Some("one.pdf"));
    assert_eq!(refs, 0);
    assert_eq!(created, "2026-11-08T00:00:00Z");
    // The bytes read back through the normal attachment path.
    assert_eq!(worker.read_attachment_bytes(id).unwrap(), PDF);

    // Single copy: no column of the mapping row holds the wrapped key or a storage id any more.
    let staged_left: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sync_artifacts WHERE staged_storage_id IS NOT NULL
                OR staged_wrapped_content_key IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(staged_left, 0);
    let copies: i64 = conn
        .query_row(
            "SELECT (SELECT COUNT(*) FROM attachments WHERE wrapped_content_key = ?1)
                  + (SELECT COUNT(*) FROM sync_artifacts WHERE staged_wrapped_content_key = ?1)",
            [&wrapped],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(copies, 1, "exactly one long-lived copy of the wrapped key");
    // The mapping itself stays (hash, receipt, kind): it is the no-refetch cache.
    assert!(lookup(&conn, 1, &ID).unwrap().is_some());
}

#[test]
fn a_rolled_back_attach_leaves_the_staging_and_no_row() {
    let (_dir, _path, _dek, worker, _storage, id) = vault_with_a_staged_attachment_and_id();
    let conn = worker.read_connection().unwrap();
    let staged_before = lookup(&conn, 1, &ID).unwrap().unwrap().staged;
    {
        let tx = conn.unchecked_transaction().unwrap();
        create_attachment_from_staged(&tx, 1, &ID, id, None, "t").unwrap();
        tx.rollback().unwrap();
    }
    assert_eq!(attachment_rows(&conn), 0);
    assert_eq!(
        lookup(&conn, 1, &ID).unwrap().unwrap().staged,
        staged_before
    );
    // And it can be done again.
    create_attachment_from_staged(&conn, 1, &ID, id, None, "t").unwrap();
    assert_eq!(attachment_rows(&conn), 1);
}

#[test]
fn every_refusal_writes_nothing_and_keeps_the_staging() {
    let (_dir, _path, _dek, worker, storage_id, id) = vault_with_a_staged_attachment_and_id();
    let conn = worker.read_connection().unwrap();
    let staged_before = lookup(&conn, 1, &ID).unwrap().unwrap().staged;
    let unchanged = |what: &str| {
        assert_eq!(attachment_rows(&conn), 0, "{what}: no row");
        assert_eq!(
            lookup(&conn, 1, &ID).unwrap().unwrap().staged,
            staged_before,
            "{what}: the staging"
        );
    };

    // No mapping for the address.
    assert!(matches!(
        create_attachment_from_staged(&conn, 1, &OTHER, id, None, "t"),
        Err(CreateAttachmentError::ArtifactMissing)
    ));
    // Another attachment than the one the artifact binds.
    let other_attachment = core_ledger::AttachmentId::from_uuid(uuid::Uuid::from_bytes([9; 16]));
    assert!(matches!(
        create_attachment_from_staged(&conn, 1, &ID, other_attachment, None, "t"),
        Err(CreateAttachmentError::WrongAttachment)
    ));
    unchanged("wrong attachment");

    // Another attachment already stores this blob name.
    conn.execute(
        "INSERT INTO attachments (id, storage_id, wrapped_content_key, content_key_nonce,
                content_nonce, content_alg, plaintext_size, ref_count, created_at)
         VALUES (zeroblob(16), ?1, x'00', x'00', x'00', 'AES-256-GCM', 1, 0, 't')",
        [&storage_id],
    )
    .unwrap();
    assert!(matches!(
        create_attachment_from_staged(&conn, 1, &ID, id, None, "t"),
        Err(CreateAttachmentError::StorageIdHeld)
    ));
    assert_eq!(
        lookup(&conn, 1, &ID).unwrap().unwrap().staged,
        staged_before,
        "storage id held: the staging"
    );
    conn.execute("DELETE FROM attachments", []).unwrap();

    // An attachments row with the id exists already.
    conn.execute(
        "INSERT INTO attachments (id, storage_id, wrapped_content_key, content_key_nonce,
                content_nonce, content_alg, plaintext_size, ref_count, created_at)
         VALUES (?1, 'another', x'00', x'00', x'00', 'AES-256-GCM', 1, 0, 't')",
        [id.as_uuid()],
    )
    .unwrap();
    assert!(matches!(
        create_attachment_from_staged(&conn, 1, &ID, id, None, "t"),
        Err(CreateAttachmentError::AlreadyExists)
    ));
    assert_eq!(
        attachment_rows(&conn),
        1,
        "only the row that was already there"
    );
    assert_eq!(
        lookup(&conn, 1, &ID).unwrap().unwrap().staged,
        staged_before
    );
    conn.execute("DELETE FROM attachments", []).unwrap();

    // After a success, a second call has nothing staged.
    create_attachment_from_staged(&conn, 1, &ID, id, None, "t").unwrap();
    assert!(matches!(
        create_attachment_from_staged(&conn, 1, &ID, id, None, "t"),
        Err(CreateAttachmentError::ArtifactMissing)
    ));
    assert_eq!(attachment_rows(&conn), 1);
}

#[test]
fn a_batch_part_has_no_attachment_to_create() {
    let dir = tempfile::tempdir().unwrap();
    let worker =
        DbWorker::open_with_raw_key(dir.path().join("vault.db"), generate_dek().unwrap()).unwrap();
    let conn = worker.read_connection().unwrap();
    record(
        &conn,
        &Mapping {
            kind: ArtifactKind::BatchPart { index: 0, count: 1 },
            ..attachment_mapping(ID, 1)
        },
        None,
    )
    .unwrap();
    let id = core_ledger::AttachmentId::from_uuid(uuid::Uuid::from_bytes([7; 16]));
    assert!(matches!(
        create_attachment_from_staged(&conn, 1, &ID, id, None, "t"),
        Err(CreateAttachmentError::ArtifactMissing)
    ));
}
