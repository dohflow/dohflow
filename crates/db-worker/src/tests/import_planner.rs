//! The planner and the file layer over the cross-device identity (`personal-cfo-0g528.3`, ADR 0074
//! Addendum J items 2, 3 and 6; S3b part 3b): what a staged row claims, when no command is sent, and what
//! the file check reads from class 1. Apply's own rule is tested in `import_identity`.

use chrono::{TimeZone, Utc};
use core_ledger::{AccountId, SourceBatchId, SourceRecordId, StagedTransactionId};
use uuid::Uuid;

use super::{liquid_subtype_cmd, meta};
use crate::staged_import::StagedDecision;
use crate::{AccountSubtype, DbWorker, WriteCommand};

const CONNECTION: u128 = 0x0198_c2c1_5f0a_7c3d_8a4e_9b1d_0000_0c02;
const FINGERPRINT: &str = "sha256:2c30d6d4f9c6954bfc4f2e31460da5660e8d4aa9d893b6228eddc467516a6830";

fn worker() -> (tempfile::TempDir, DbWorker) {
    super::worker()
}

fn account(worker: &DbWorker, name: &str) -> AccountId {
    let id = AccountId::new();
    worker
        .dispatch(
            meta(),
            liquid_subtype_cmd(id, name, Some(AccountSubtype::Checking), 0),
        )
        .unwrap();
    id
}

fn key(worker: &DbWorker) -> core_ids::DerivedIdKey {
    core_ids::DerivedIdKey::for_vault(&worker.vault_id().unwrap())
}

/// A staged row of a batch: the batch and the record go in as commands, the staged row directly.
#[allow(clippy::too_many_arguments)]
fn stage(
    worker: &DbWorker,
    account: AccountId,
    source_type: &str,
    fingerprint: Option<&str>,
    connection: Option<Uuid>,
    external_id: Option<&str>,
    normalized: &str,
    row_fingerprint: &str,
    ordinal: i64,
    minor: i64,
) -> (SourceBatchId, StagedTransactionId) {
    let batch = SourceBatchId::new();
    let record = SourceRecordId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateSourceBatch {
                id: batch,
                source_type: source_type.to_owned(),
                source_name: None,
                file_fingerprint: fingerprint.map(str::to_owned),
                parser_version: None,
                connector_connection_id: connection,
            },
        )
        .unwrap();
    worker
        .dispatch(
            meta(),
            WriteCommand::AttachSourceRecord {
                id: record,
                batch_id: batch,
                external_id: external_id.map(str::to_owned),
                source_hash: format!("sha256:{}", record.as_uuid()),
                normalized_json: normalized.to_owned(),
                parse_confidence_bps: None,
            },
        )
        .unwrap();
    let conn = worker.read_connection().unwrap();
    let staged = crate::ingestion::stage_transaction(
        &conn,
        &crate::ingestion::NewStagedTransaction {
            source_record_id: record.as_uuid(),
            proposed_account_id: Some(account.as_uuid()),
            posted_at: "2026-03-04",
            transaction_date: None,
            amount_minor: minor,
            currency: "USD",
            normalized_merchant: Some("merchant"),
            description: Some("COFFEE"),
            imported_category: None,
            txn_fingerprint: row_fingerprint,
            row_ordinal: ordinal,
        },
    )
    .unwrap();
    (batch, StagedTransactionId::from_uuid(staged))
}

fn json(description: &str) -> String {
    format!(r#"{{"account_key":"ACT-9","posted":1772625600,"description":"{description}"}}"#)
}

fn provider(
    worker: &DbWorker,
    account: AccountId,
    txn: &str,
    minor: i64,
    description: &str,
) -> StagedTransactionId {
    stage(
        worker,
        account,
        "simplefin",
        None,
        Some(Uuid::from_u128(CONNECTION)),
        Some(txn),
        &json(description),
        &format!("sfin:{txn}"),
        1,
        minor,
    )
    .1
}

fn file(
    worker: &DbWorker,
    account: AccountId,
    ordinal: i64,
    minor: i64,
) -> (SourceBatchId, StagedTransactionId) {
    // A fingerprint per upload keeps the local fingerprint layer out of the way: this tests identity.
    let unique = Uuid::now_v7();
    stage(
        worker,
        account,
        "csv",
        Some(FINGERPRINT),
        None,
        None,
        "{}",
        &format!("file|{ordinal}|{unique}"),
        ordinal,
        minor,
    )
}

fn count(worker: &DbWorker, sql: &str) -> i64 {
    worker
        .read_connection()
        .unwrap()
        .query_row(sql, [], |r| r.get(0))
        .unwrap()
}

fn ops(worker: &DbWorker) -> i64 {
    count(worker, "SELECT COUNT(*) FROM operation_log")
}

fn imported(worker: &DbWorker) -> i64 {
    count(
        worker,
        "SELECT COUNT(*) FROM ledger_transactions lt WHERE EXISTS (
             SELECT 1 FROM source_provenance_links pl
              WHERE pl.entity_id = lt.id AND pl.relationship = 'created_from')",
    )
}

fn committed_to(worker: &DbWorker, staged: StagedTransactionId) -> Option<Uuid> {
    worker
        .read_connection()
        .unwrap()
        .query_row(
            "SELECT committed_transaction_id FROM staged_transactions WHERE id = ?1",
            [staged.as_uuid()],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn a_simplefin_row_commits_under_its_derived_id_and_records_its_identity() {
    let (_dir, worker) = worker();
    let account = account(&worker, "Checking");
    let staged = provider(&worker, account, "TXN-1", -1_234, "COFFEE");
    assert!(matches!(
        worker.commit_staged(meta(), staged, false).unwrap(),
        StagedDecision::Committed(_)
    ));
    let derived = key(&worker)
        .provider_txn_id(&Uuid::from_u128(CONNECTION), "ACT-9", "TXN-1")
        .unwrap();
    assert_eq!(committed_to(&worker, staged), Some(derived));
    assert_eq!(
        count(
            &worker,
            "SELECT COUNT(*) FROM ledger_transactions WHERE voided_at IS NULL AND id IN
                 (SELECT transaction_id FROM import_identities WHERE kind = 'provider')"
        ),
        1,
        "the identity names the transaction, whose ledger id is the derived id"
    );
    assert_eq!(
        count(
            &worker,
            "SELECT COUNT(*) FROM dedupe_decisions WHERE decision = 'committed'
                AND identity_id IS NOT NULL AND transaction_id IS NOT NULL"
        ),
        1
    );
}

#[test]
fn an_equal_refresh_sends_no_command_and_is_bookkept_against_the_held_transaction() {
    let (_dir, worker) = worker();
    let account = account(&worker, "Checking");
    let first = provider(&worker, account, "TXN-1", -1_234, "COFFEE");
    worker.commit_staged(meta(), first, false).unwrap();
    // The next refresh (this device's, or another device's, once it has the rows): the same provider
    // transaction in a new batch.
    let again = provider(&worker, account, "TXN-1", -1_234, "COFFEE");
    let before = ops(&worker);
    assert_eq!(
        worker.commit_staged(meta(), again, false).unwrap(),
        StagedDecision::AlreadyCommitted
    );
    assert_eq!(ops(&worker), before, "no command was sent");
    assert_eq!(imported(&worker), 1);
    assert_eq!(
        committed_to(&worker, again),
        committed_to(&worker, first),
        "both staged rows name the one transaction"
    );
    assert_eq!(
        count(
            &worker,
            "SELECT COUNT(*) FROM dedupe_decisions WHERE decision = 'merged'"
        ),
        0,
        "apply reaches `merged` only when two commits meet"
    );
}

#[test]
fn a_voided_transaction_is_never_resurrected_by_a_refresh() {
    let (_dir, worker) = worker();
    let account = account(&worker, "Checking");
    let first = provider(&worker, account, "TXN-1", -1_234, "COFFEE");
    worker.commit_staged(meta(), first, false).unwrap();
    let held = committed_to(&worker, first).unwrap();
    worker
        .dispatch(
            meta(),
            WriteCommand::VoidTransaction {
                transaction_id: core_ledger::TransactionId::from_uuid(held),
            },
        )
        .unwrap();
    let before = count(&worker, "SELECT COUNT(*) FROM ledger_transactions");
    let again = provider(&worker, account, "TXN-1", -1_234, "COFFEE");
    assert_eq!(
        worker.commit_staged(meta(), again, false).unwrap(),
        StagedDecision::AlreadyCommitted
    );
    assert_eq!(
        count(&worker, "SELECT COUNT(*) FROM ledger_transactions"),
        before
    );
}

#[test]
fn a_revised_provider_row_on_the_device_that_staged_it_is_flagged_as_before_and_the_revision_is_recorded(
) {
    let (_dir, worker) = worker();
    let account = account(&worker, "Checking");
    let first = provider(&worker, account, "TXN-1", -1_234, "COFFEE");
    worker.commit_staged(meta(), first, false).unwrap();
    let revised = provider(&worker, account, "TXN-1", -1_300, "COFFEE");
    assert_eq!(
        worker.commit_staged(meta(), revised, false).unwrap(),
        StagedDecision::Flagged,
        "this device still flags it, as it always did"
    );
    let reason: Option<String> = worker
        .read_connection()
        .unwrap()
        .query_row(
            "SELECT flag_reason FROM staged_transactions WHERE id = ?1",
            [revised.as_uuid()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        reason.as_deref(),
        Some("duplicate of an already-committed transaction")
    );
    assert_eq!(imported(&worker), 1);
    assert_eq!(
        count(
            &worker,
            "SELECT COUNT(*) FROM import_dispositions
              WHERE kind = 'provider_revision' AND status = 'pending'"
        ),
        1,
        "and the revision is recorded for every device, not surfaced"
    );
}

#[test]
fn a_revised_row_on_a_device_with_no_staged_counterpart_records_the_revision_and_commits_nothing() {
    let (_dir, worker) = worker();
    let account = account(&worker, "Checking");
    let first = provider(&worker, account, "TXN-1", -1_234, "COFFEE");
    worker.commit_staged(meta(), first, false).unwrap();
    // A replica holds the committed row and its identity but never staged the first refresh.
    worker
        .read_connection()
        .unwrap()
        .execute(
            "DELETE FROM staged_transactions WHERE id = ?1",
            [first.as_uuid()],
        )
        .unwrap();
    let revised = provider(&worker, account, "TXN-1", -1_300, "COFFEE");
    assert!(matches!(
        worker.commit_staged(meta(), revised, false).unwrap(),
        StagedDecision::Committed(_)
    ));
    assert_eq!(imported(&worker), 1, "nothing was committed");
    assert_eq!(
        committed_to(&worker, revised),
        Some(
            key(&worker)
                .provider_txn_id(&Uuid::from_u128(CONNECTION), "ACT-9", "TXN-1")
                .unwrap()
        )
    );
    assert_eq!(
        count(
            &worker,
            "SELECT COUNT(*) FROM import_dispositions WHERE kind = 'provider_revision'"
        ),
        1
    );
}

#[test]
fn an_import_anyway_commits_the_row_without_claiming_an_identity() {
    let (_dir, worker) = worker();
    let account = account(&worker, "Checking");
    let first = provider(&worker, account, "TXN-1", -1_234, "COFFEE");
    worker.commit_staged(meta(), first, false).unwrap();
    let revised = provider(&worker, account, "TXN-1", -1_300, "COFFEE");
    worker.commit_staged(meta(), revised, false).unwrap();
    assert!(matches!(
        worker.commit_staged(meta(), revised, true).unwrap(),
        StagedDecision::Committed(_)
    ));
    assert_eq!(imported(&worker), 2, "the person's override commits it");
    assert_eq!(count(&worker, "SELECT COUNT(*) FROM import_identities"), 1);
}

#[test]
fn a_lunchflow_row_keeps_the_heuristic_path_and_gets_no_identity() {
    let (_dir, worker) = worker();
    let account = account(&worker, "Checking");
    let (_, staged) = stage(
        &worker,
        account,
        "lunchflow",
        None,
        Some(Uuid::from_u128(CONNECTION)),
        Some("LF-1"),
        &json("COFFEE"),
        "lflow:LF-1",
        1,
        -1_234,
    );
    assert!(matches!(
        worker.commit_staged(meta(), staged, false).unwrap(),
        StagedDecision::Committed(_)
    ));
    assert_eq!(count(&worker, "SELECT COUNT(*) FROM import_identities"), 0);
    assert_eq!(imported(&worker), 1);
}

#[test]
fn a_file_rows_claim_takes_its_ordinal_from_the_staged_row_the_ledger_id_derives_from() {
    let (_dir, worker) = worker();
    let account = account(&worker, "Checking");
    let (batch, staged) = file(&worker, account, 3, -700);
    worker.commit_staged(meta(), staged, false).unwrap();
    let (ordinal, transaction): (i64, Uuid) = worker
        .read_connection()
        .unwrap()
        .query_row(
            "SELECT row_ordinal, transaction_id FROM import_identities WHERE kind = 'file'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(ordinal, 3, "the staged row's own ordinal");
    let expected = key(&worker)
        .content_id(
            "txn.import",
            &[batch.as_uuid().as_bytes(), 3_i64.to_be_bytes().as_slice()],
        )
        .unwrap();
    assert_eq!(
        transaction, expected,
        "the ledger id derives from the same ordinal"
    );
    assert_eq!(committed_to(&worker, staged), Some(expected));
}

#[test]
fn a_second_upload_of_a_file_row_sends_no_command_and_a_routed_one_records_a_conflict() {
    let (_dir, worker) = worker();
    let checking = account(&worker, "Checking");
    let savings = account(&worker, "Savings");
    let (_, first) = file(&worker, checking, 1, -700);
    worker.commit_staged(meta(), first, false).unwrap();

    // The same row from another upload (another device, with the rows): nothing to send.
    let (_, same) = file(&worker, checking, 1, -700);
    let before = ops(&worker);
    assert_eq!(
        worker.commit_staged(meta(), same, false).unwrap(),
        StagedDecision::AlreadyCommitted
    );
    assert_eq!(ops(&worker), before);

    // Imported under another account map, the ordinal names a different row: no silent drop and no
    // second transaction, a conflict recorded.
    let (_, routed) = file(&worker, savings, 1, -700);
    worker.commit_staged(meta(), routed, false).unwrap();
    assert_eq!(imported(&worker), 1);
    assert_eq!(
        count(
            &worker,
            "SELECT COUNT(*) FROM import_dispositions WHERE kind = 'file_row_conflict'"
        ),
        1
    );
}

#[test]
fn re_uploading_a_voided_file_keeps_restoring_silently_until_the_restore_question_exists() {
    let (_dir, worker) = worker();
    let account = account(&worker, "Checking");
    let (_, first) = file(&worker, account, 1, -700);
    worker.commit_staged(meta(), first, false).unwrap();
    let held = committed_to(&worker, first).unwrap();
    worker
        .dispatch(
            meta(),
            WriteCommand::VoidTransaction {
                transaction_id: core_ledger::TransactionId::from_uuid(held),
            },
        )
        .unwrap();
    assert_eq!(
        worker.batch_with_fingerprint(FINGERPRINT).unwrap(),
        None,
        "an all-voided file is not 'already imported'"
    );
    let (_, again) = file(&worker, account, 1, -700);
    assert!(matches!(
        worker.commit_staged(meta(), again, false).unwrap(),
        StagedDecision::Committed(_)
    ));
    assert_eq!(
        imported(&worker),
        2,
        "a fresh copy; the original stays voided"
    );
}

#[test]
fn the_file_check_reads_class_one_so_a_device_with_no_staging_sees_an_active_file() {
    let (_dir, worker) = worker();
    let account = account(&worker, "Checking");
    assert_eq!(worker.batch_with_fingerprint(FINGERPRINT).unwrap(), None);
    let (batch, staged) = file(&worker, account, 1, -700);
    worker.commit_staged(meta(), staged, false).unwrap();
    assert_eq!(
        worker.batch_with_fingerprint(FINGERPRINT).unwrap(),
        Some(batch.as_uuid())
    );
    // A replica never staged the file: only the replicated rows remain.
    let conn = worker.read_connection().unwrap();
    conn.execute("DELETE FROM staged_transactions", []).unwrap();
    assert_eq!(
        worker.batch_with_fingerprint(FINGERPRINT).unwrap(),
        Some(batch.as_uuid()),
        "the identity and its transaction say it"
    );
}

#[test]
fn a_file_whose_rows_are_all_flagged_still_counts_as_imported_on_this_device() {
    let (_dir, worker) = worker();
    let account = account(&worker, "Checking");
    let (batch, staged) = file(&worker, account, 1, -700);
    // The local, staging-only check beside the class-1 one: no identity, no committed row.
    let conn = worker.read_connection().unwrap();
    crate::ingestion::flag_staged_for_review(&conn, staged.as_uuid(), "needs a look", None)
        .unwrap();
    conn.execute(
        "UPDATE source_batches SET status = 'committed' WHERE id = ?1",
        [batch.as_uuid()],
    )
    .unwrap();
    assert_eq!(
        worker.batch_with_fingerprint(FINGERPRINT).unwrap(),
        Some(batch.as_uuid())
    );
}

#[test]
fn a_fallback_fingerprint_feeds_no_identity() {
    let (_dir, worker) = worker();
    let account = account(&worker, "Checking");
    let (_, staged) = stage(
        &worker,
        account,
        "csv",
        Some("fallback:abc"),
        None,
        None,
        "{}",
        "file|fallback",
        1,
        -700,
    );
    assert!(matches!(
        worker.commit_staged(meta(), staged, false).unwrap(),
        StagedDecision::Committed(_)
    ));
    assert_eq!(count(&worker, "SELECT COUNT(*) FROM import_identities"), 0);
}

#[allow(dead_code)]
fn when() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 4, 12, 0, 0).unwrap()
}
