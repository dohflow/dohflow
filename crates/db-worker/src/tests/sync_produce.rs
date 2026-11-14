//! The producer refuses a worker that does not accept writes, as every Sync writer does
//! (`crate::sync_produce`, personal-cfo-1cltt.7 slice C0): it puts nothing from such a vault, and a
//! worker that stops accepting writes between a put and its record leaves no mapping.
//!
//! The put is a closure here: nothing in `src` implements `ArtifactTransport` (a source-scan test
//! forbids it until the engine does), and the closure counts what the pipeline asks it to put.
//!
//! The pipeline records an artifact's mapping and descriptor **before** its put (C.9), so a worker
//! that stops between the put and the receipt leaves the mapping and the digest, and no receipt.

use sync_core::artifact::ArtifactId;
use vault_crypto::sync_artifact::SealedArtifact;
use vault_crypto::sync_artifact_flow::{PublishError, Published};
use vault_crypto::sync_envelope::EpochKey;
use vault_crypto::sync_keys::epoch_key_commitment;
use vault_crypto::SecretBytes;

use super::*;
use crate::sync_produce::ProduceError;

/// A vault opened with a raw key, which is what addressing under the DEK needs.
fn raw_worker() -> (TempDir, DbWorker) {
    let dir = TempDir::new().unwrap();
    let worker = DbWorker::open_with_raw_key(
        dir.path().join("test.vault"),
        vault_crypto::generate_dek().unwrap(),
    )
    .unwrap();
    (dir, worker)
}

fn epoch_key() -> EpochKey {
    let secret = SecretBytes::new(std::array::from_fn(|i| 7_u8.wrapping_add(i as u8)));
    let commitment = epoch_key_commitment(&secret);
    EpochKey::from_verified([0x11; 16], 1, secret, &commitment).unwrap()
}

/// What a faithful service answers for `sealed`: the receipt of the bytes it was sent.
fn answered(sealed: &SealedArtifact) -> Result<Published, PublishError> {
    Ok(Published {
        receipt: sealed.receipt,
        plaintext_digest: sealed.plaintext_digest,
    })
}

/// A batch of one record, as an import leaves it.
fn put_batch(worker: &DbWorker, id: [u8; 16]) {
    let conn = worker.read_connection().unwrap();
    conn.execute(
        "INSERT INTO source_batches (id, source_type, created_at, updated_at)
         VALUES (?1, 'csv', '2026-11-01T00:00:00Z', '2026-11-01T00:00:00Z')",
        [id.as_slice()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO parser_runs (id, source_batch_id, parser_name, parser_version, status, started_at)
         VALUES (?1, ?2, 'csv', '1', 'ok', '2026-11-01T00:00:00Z')",
        rusqlite::params![[0xc1_u8; 16].as_slice(), id.as_slice()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO source_records (id, source_batch_id, source_hash, normalized_json, created_at)
         VALUES (?1, ?2, 'h', '{}', '2026-11-01T00:00:00Z')",
        rusqlite::params![[0x01_u8; 16].as_slice(), id.as_slice()],
    )
    .unwrap();
}

fn mappings(worker: &DbWorker) -> i64 {
    worker
        .read_connection()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM sync_artifacts", [], |r| r.get(0))
        .unwrap()
}

fn receipts(worker: &DbWorker) -> i64 {
    worker
        .read_connection()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM sync_artifacts WHERE receipt_at IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap()
}

/// A panic that leaves the worker needing recovery.
fn poison(worker: &DbWorker) {
    let outcome = worker.dispatch_inner(
        meta(),
        create_account_cmd(),
        Fault::PanicAt(FaultPoint::AfterMutation),
    );
    assert!(matches!(outcome, Err(DbError::WriterPanicked)));
    assert_eq!(worker.state(), WorkerState::CorruptNeedsRecovery);
}

/// A vault that refuses writes publishes nothing: the check comes before the first put.
#[test]
fn a_worker_that_refuses_writes_puts_nothing_and_records_nothing() {
    let (_dir, worker) = raw_worker();
    let batch = [0xb1; 16];
    put_batch(&worker, batch);
    poison(&worker);

    let key = epoch_key();
    let puts = std::cell::Cell::new(0);
    let result = worker.produce_batch_with(&key, &batch, &mut |_: &ArtifactId, sealed| {
        puts.set(puts.get() + 1);
        answered(sealed)
    });
    assert!(
        matches!(
            result,
            Err(ProduceError::Db(DbError::WorkerUnavailable(
                WorkerState::CorruptNeedsRecovery
            )))
        ),
        "{result:?}"
    );
    assert_eq!(
        puts.get(),
        0,
        "nothing was put from a vault that refuses writes"
    );
    assert_eq!(mappings(&worker), 0);
}

/// A worker that stops accepting writes between a put and its record records nothing: the mapping
/// and the receipt go through `write_sync_state`, which refuses it.
#[test]
fn a_worker_that_stops_between_a_put_and_its_record_leaves_no_mapping() {
    let (_dir, worker) = raw_worker();
    let batch = [0xb1; 16];
    put_batch(&worker, batch);

    let key = epoch_key();
    let result = worker.produce_batch_with(&key, &batch, &mut |_: &ArtifactId, sealed| {
        let published = answered(sealed);
        poison(&worker);
        published
    });
    assert!(
        matches!(
            result,
            Err(ProduceError::Db(DbError::WorkerUnavailable(
                WorkerState::CorruptNeedsRecovery
            )))
        ),
        "{result:?}"
    );
    assert_eq!(
        (mappings(&worker), receipts(&worker)),
        (1, 0),
        "the mapping and its descriptor were recorded before the put; no receipt was"
    );
}

/// An attachment, written as an import leaves it.
fn put_attachment(worker: &DbWorker) -> [u8; 16] {
    let id = worker
        .import_attachment(
            b"%PDF-1.4\nan attachment\n%%EOF\n",
            Some("application/pdf"),
            None,
        )
        .unwrap();
    *id.as_uuid().as_bytes()
}

/// An attachment is refused the same way: nothing is put from a vault that refuses writes, and a
/// worker that stops between the put and its record leaves no mapping.
#[test]
fn an_attachment_is_refused_by_a_worker_that_refuses_writes_and_by_one_that_stops_mid_way() {
    let key = epoch_key();

    let (_dir, worker) = raw_worker();
    let id = put_attachment(&worker);
    poison(&worker);
    let puts = std::cell::Cell::new(0);
    let result = worker.produce_attachment_with(&key, &id, &mut |_: &ArtifactId, sealed| {
        puts.set(puts.get() + 1);
        answered(sealed)
    });
    assert!(
        matches!(
            result,
            Err(ProduceError::Db(DbError::WorkerUnavailable(
                WorkerState::CorruptNeedsRecovery
            )))
        ),
        "{result:?}"
    );
    assert_eq!(puts.get(), 0);
    assert_eq!(mappings(&worker), 0);

    let (_dir, worker) = raw_worker();
    let id = put_attachment(&worker);
    let result = worker.produce_attachment_with(&key, &id, &mut |_: &ArtifactId, sealed| {
        let published = answered(sealed);
        poison(&worker);
        published
    });
    assert!(
        matches!(
            result,
            Err(ProduceError::Db(DbError::WorkerUnavailable(
                WorkerState::CorruptNeedsRecovery
            )))
        ),
        "{result:?}"
    );
    assert_eq!((mappings(&worker), receipts(&worker)), (1, 0));
}

/// What a first seal and a later one sent: every `(id, container)` the put closure saw.
type Sent = std::cell::RefCell<Vec<(ArtifactId, Vec<u8>)>>;

/// Produce the attachment `id`, recording what was put.
fn produce_attachment_into(
    worker: &DbWorker,
    key: &EpochKey,
    id: &[u8; 16],
    sent: &Sent,
) -> Result<crate::sync_produce::ProducedArtifact, ProduceError> {
    worker.produce_attachment_with(key, id, &mut |artifact_id: &ArtifactId, sealed| {
        sent.borrow_mut()
            .push((*artifact_id, sealed.container.clone()));
        answered(sealed)
    })
}

/// Open a produced container and answer its logical metadata (N2: the MIME type and the attachment
/// id travel inside the AEAD, never beside it).
fn metadata_of(key: &EpochKey, id: &ArtifactId, container: &[u8]) -> String {
    let opened = vault_crypto::sync_artifact::open_artifact(
        key,
        id,
        container,
        sync_core::artifact::MAX_ARTIFACT_CONTAINER,
    )
    .unwrap();
    String::from_utf8(opened.plaintext().logical_metadata.to_vec()).unwrap()
}

/// N2 (mutant P6): the produced attachment opens, and its metadata carries the attachment's id and
/// its media type in canonical JSON, keys ascending; an attachment without a media type carries the
/// id alone.
#[test]
fn a_produced_attachment_opens_and_carries_its_id_and_media_type() {
    let (_dir, worker) = raw_worker();
    let key = epoch_key();
    let with_mime = put_attachment(&worker);
    let without = *worker
        .import_attachment(b"no media type", None, None)
        .unwrap()
        .as_uuid()
        .as_bytes();
    let sent = Sent::default();
    let first = produce_attachment_into(&worker, &key, &with_mime, &sent).unwrap();
    let second = produce_attachment_into(&worker, &key, &without, &sent).unwrap();
    let sent = sent.borrow();
    assert_eq!(
        metadata_of(&key, &first.id, &sent[0].1),
        format!(
            r#"{{"attachment_id":"{}","mime_type":"application/pdf"}}"#,
            uuid::Uuid::from_bytes(with_mime).hyphenated()
        )
    );
    assert_eq!(
        metadata_of(&key, &second.id, &sent[1].1),
        format!(
            r#"{{"attachment_id":"{}"}}"#,
            uuid::Uuid::from_bytes(without).hyphenated()
        )
    );
}

/// The descriptor is on the row before the put (C.9): the digest of the complete transport
/// plaintext and the metadata bytes it carried.
#[test]
fn a_first_seal_records_the_digest_and_the_metadata_before_the_put() {
    let (_dir, worker) = raw_worker();
    let key = epoch_key();
    let id = put_attachment(&worker);
    let on_put = std::cell::RefCell::new(None);
    produce_attachment_with_probe(&worker, &key, &id, &on_put);
    let (digest, metadata): (Vec<u8>, Vec<u8>) = on_put.borrow().clone().unwrap();
    assert_eq!(digest.len(), 32);
    assert!(String::from_utf8(metadata)
        .unwrap()
        .contains(&uuid::Uuid::from_bytes(id).hyphenated().to_string()));
}

/// Produce, and read the row's descriptor from inside the put (the put is after the record).
fn produce_attachment_with_probe(
    worker: &DbWorker,
    key: &EpochKey,
    id: &[u8; 16],
    seen: &std::cell::RefCell<Option<(Vec<u8>, Vec<u8>)>>,
) {
    worker
        .produce_attachment_with(key, id, &mut |_: &ArtifactId, sealed| {
            let row = worker
                .read_connection()
                .unwrap()
                .query_row(
                    "SELECT transport_plaintext_sha256, logical_metadata FROM sync_artifacts",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            *seen.borrow_mut() = Some(row);
            answered(sealed)
        })
        .unwrap();
}

/// A changed plaintext at an address already sealed is refused BEFORE any encryption (C.9): here
/// the recorded digest no longer matches what the rows rebuild, nothing is put, and the error names
/// the address. (Mutant: `Fixity::First` on a later seal puts a second plaintext under the zero
/// nonce.)
#[test]
fn a_changed_plaintext_at_a_sealed_address_is_refused_before_encryption() {
    let (_dir, worker) = raw_worker();
    let key = epoch_key();
    let id = put_attachment(&worker);
    let sent = Sent::default();
    let first = produce_attachment_into(&worker, &key, &id, &sent).unwrap();

    worker
        .read_connection()
        .unwrap()
        .execute(
            "UPDATE sync_artifacts SET transport_plaintext_sha256 = zeroblob(32)",
            [],
        )
        .unwrap();
    let puts = std::cell::Cell::new(0);
    let result = worker.produce_attachment_with(&key, &id, &mut |_: &ArtifactId, sealed| {
        puts.set(puts.get() + 1);
        answered(sealed)
    });
    assert!(
        matches!(&result, Err(ProduceError::ArtifactDescriptorChanged(changed)) if *changed == first.id),
        "{result:?}"
    );
    assert_eq!(puts.get(), 0, "refused before anything was put");
}

/// The same for a batch part: a record edited after the first seal changes the part's blob, the
/// stored id and digest still stand for the logical batch, and the part is refused.
#[test]
fn a_batch_whose_rows_changed_after_its_first_seal_is_refused_before_encryption() {
    let (_dir, worker) = raw_worker();
    let key = epoch_key();
    let batch = [0xb1; 16];
    put_batch(&worker, batch);
    worker
        .produce_batch_with(&key, &batch, &mut |_: &ArtifactId, sealed| answered(sealed))
        .unwrap();
    worker
        .read_connection()
        .unwrap()
        .execute(
            "UPDATE source_records SET normalized_json = '{\"n\":2}'",
            [],
        )
        .unwrap();
    let puts = std::cell::Cell::new(0);
    let result = worker.produce_batch_with(&key, &batch, &mut |_: &ArtifactId, sealed| {
        puts.set(puts.get() + 1);
        answered(sealed)
    });
    assert!(
        matches!(result, Err(ProduceError::ArtifactDescriptorChanged(_))),
        "{result:?}"
    );
    assert_eq!(puts.get(), 0);
}

/// A MIME or filename edit after the first seal never re-seals (C.9): the stored metadata is used, so
/// the same address, container and hash come out and a re-put succeeds.
#[test]
fn a_media_type_edit_after_the_first_seal_does_not_change_the_artifact() {
    let (_dir, worker) = raw_worker();
    let key = epoch_key();
    let id = put_attachment(&worker);
    let sent = Sent::default();
    let first = produce_attachment_into(&worker, &key, &id, &sent).unwrap();
    worker
        .read_connection()
        .unwrap()
        .execute("UPDATE attachments SET mime_type = 'image/png'", [])
        .unwrap();
    let again = produce_attachment_into(&worker, &key, &id, &sent).unwrap();
    assert_eq!(again.id, first.id);
    assert_eq!(again.ciphertext_sha256, first.ciphertext_sha256);
    let sent = sent.borrow();
    assert_eq!(sent[0], sent[1], "the same container, byte for byte");
}

/// A rotation changes an attachment's storage id and renames its blob; an artifact that was sealed
/// keeps its address, because a later production finds it by the logical id and never recomputes the
/// address from the current storage id.
#[test]
fn a_rotation_after_the_first_seal_does_not_move_the_artifact() {
    let (_dir, worker) = raw_worker();
    let key = epoch_key();
    let id = put_attachment(&worker);
    let sent = Sent::default();
    let first = produce_attachment_into(&worker, &key, &id, &sent).unwrap();

    // What a rotation does to the row and the blob: a new storage id, the file renamed.
    let old: String = worker
        .read_connection()
        .unwrap()
        .query_row("SELECT storage_id FROM attachments", [], |r| r.get(0))
        .unwrap();
    let new = "ab".repeat(32);
    let dir = crate::attachments::blobs_dir(&worker.path);
    std::fs::rename(dir.join(&old), dir.join(&new)).unwrap();
    worker
        .read_connection()
        .unwrap()
        .execute("UPDATE attachments SET storage_id = ?1", [&new])
        .unwrap();

    let again = produce_attachment_into(&worker, &key, &id, &sent).unwrap();
    assert_eq!(again.id, first.id, "the address did not move");
    assert_eq!(again.ciphertext_sha256, first.ciphertext_sha256);
}

/// Re-importing the same bytes after a shred gives a new attachment id and so a new address (C.3):
/// the two never share an address, though their storage id is the same.
#[test]
fn the_same_bytes_under_two_attachment_ids_are_two_artifacts() {
    let (_dir, worker) = raw_worker();
    let key = epoch_key();
    let first = put_attachment(&worker);
    let sent = Sent::default();
    let one = produce_attachment_into(&worker, &key, &first, &sent).unwrap();
    worker
        .read_connection()
        .unwrap()
        .execute("DELETE FROM attachments", [])
        .unwrap();
    let second = put_attachment(&worker);
    assert_ne!(first, second);
    let two = produce_attachment_into(&worker, &key, &second, &sent).unwrap();
    assert_ne!(one.id, two.id);
}

/// F3 (mutant E1): the closure's sources are compared with what the export's own read transaction
/// saw. An attachment that arrives after the closure was produced and before the export, and is
/// removed again after the export, is in the export: the comparison sees it in the export's snapshot
/// and the closure is produced again. (A second read after the export would see neither and pass an
/// export that names an attachment the closure lacks.)
#[test]
fn an_attachment_added_before_the_export_and_removed_after_it_is_still_caught() {
    let (_dir, worker) = raw_worker();
    let key = epoch_key();
    let produced = std::cell::Cell::new(0);
    let late = std::cell::Cell::new(None);
    let mut produce = || {
        produced.set(produced.get() + 1);
        let result =
            worker.produce_closure_with(&key, &mut |_: &ArtifactId, sealed| answered(sealed));
        if produced.get() == 1 {
            // After the first production and before the export: an attachment arrives.
            late.set(Some(put_attachment(&worker)));
        }
        result
    };
    let mut after_export = || {
        if let Some(id) = late.take() {
            // After the first export: it is removed again.
            worker
                .read_connection()
                .unwrap()
                .execute("DELETE FROM attachments WHERE id = ?1", [id.as_slice()])
                .unwrap();
        }
    };
    let exported =
        crate::sync_enable::export_with_closure_using(&worker, &mut produce, &mut after_export);
    assert!(exported.is_ok(), "{:?}", exported.err());
    assert_eq!(
        produced.get(),
        2,
        "the first export named an attachment the closure lacked: it was produced again"
    );
}
