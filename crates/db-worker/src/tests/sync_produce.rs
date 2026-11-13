//! The producer refuses a worker that does not accept writes, as every Sync writer does
//! (`crate::sync_produce`, personal-cfo-1cltt.7 slice C0): it puts nothing from such a vault, and a
//! worker that stops accepting writes between a put and its record leaves no mapping.
//!
//! The put is a closure here: nothing in `src` implements `ArtifactTransport` (a source-scan test
//! forbids it until the engine does), and the closure counts what the pipeline asks it to put.

use sync_core::artifact::TransportPlaintext;
use vault_crypto::sync_artifact::{seal_artifact, Fixity};
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

/// What a faithful service answers for `plaintext`: the receipt of the bytes it seals to.
fn sealed(key: &EpochKey, plaintext: &TransportPlaintext<'_>) -> Result<Published, PublishError> {
    let sealed = seal_artifact(key, plaintext, Fixity::First).map_err(PublishError::Seal)?;
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
    let result = worker.produce_batch_with(&key, &batch, &mut |plaintext| {
        puts.set(puts.get() + 1);
        sealed(&key, plaintext)
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
    let result = worker.produce_batch_with(&key, &batch, &mut |plaintext| {
        let published = sealed(&key, plaintext);
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
        mappings(&worker),
        0,
        "no mapping and no receipt were recorded"
    );
}
