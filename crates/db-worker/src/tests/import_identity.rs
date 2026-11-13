//! The commit rule of `CommitImportedTransaction` for a row that claims a cross-device identity
//! (`personal-cfo-0g528.3`, ADR 0074 Addendum J items 3 and 4; `dohflow-engine-5`): what apply
//! refuses, what it records, and what two commits of one identity make of each other.

use std::sync::Arc;

use chrono::{DateTime, TimeZone, Utc};
use core_ledger::{AccountId, SourceBatchId, SourceRecordId, TransactionId};
use core_money::{Currency, Money};
use sync_core::snapshot::verify;
use sync_core::status::EngineVersion;
use uuid::Uuid;

use super::{liquid_subtype_cmd, meta, seed_golden_vault, worker};
use crate::logical_export::{export_genesis_plaintext, ExportInput};
use crate::rebase::StepContext;
use crate::sync_bootstrap::{bootstrap_replica, ReplicaSource};
use crate::{
    AccountSubtype, DbError, DbWorker, ImportIdentityClaim, ImportIdentityComponents, RejectCode,
    WriteCommand,
};

const CONNECTION: u128 = 0x0198_c2c1_5f0a_7c3d_8a4e_9b1d_0000_0c01;
const FINGERPRINT: &str = "sha256:2c30d6d4f9c6954bfc4f2e31460da5660e8d4aa9d893b6228eddc467516a6830";

fn when() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 4, 12, 0, 0).unwrap()
}

fn key(worker: &DbWorker) -> core_ids::DerivedIdKey {
    core_ids::DerivedIdKey::for_vault(&worker.vault_id().unwrap())
}

pub(super) fn account(worker: &DbWorker, name: &str) -> AccountId {
    let id = AccountId::new();
    worker
        .dispatch(
            meta(),
            liquid_subtype_cmd(id, name, Some(AccountSubtype::Checking), 0),
        )
        .unwrap();
    id
}

/// A batch and one source record in it, dispatched as commands (so a replica can pull them).
fn stage(
    worker: &DbWorker,
    source_type: &str,
    fingerprint: Option<&str>,
    connection: Option<Uuid>,
    external_id: Option<&str>,
    normalized: &str,
) -> (SourceBatchId, SourceRecordId, Vec<WriteCommand>) {
    let batch = SourceBatchId::new();
    let record = SourceRecordId::new();
    let commands = vec![
        WriteCommand::CreateSourceBatch {
            id: batch,
            source_type: source_type.to_owned(),
            source_name: None,
            file_fingerprint: fingerprint.map(str::to_owned),
            parser_version: None,
            connector_connection_id: connection,
        },
        WriteCommand::AttachSourceRecord {
            id: record,
            batch_id: batch,
            external_id: external_id.map(str::to_owned),
            source_hash: format!("sha256:{}", record.as_uuid()),
            normalized_json: normalized.to_owned(),
            parse_confidence_bps: Some(10_000),
        },
    ];
    for command in &commands {
        worker.dispatch(meta(), command.clone()).unwrap();
    }
    (batch, record, commands)
}

fn simplefin_json(description: &str) -> String {
    format!(r#"{{"account_key":"ACT-9","posted":1772625600,"description":"{description}"}}"#)
}

/// A provider row: its batch, its record and the commit that claims the identity of
/// (connection, "ACT-9", `txn`). The commit's ledger id is the derived id.
pub(super) fn provider_row(
    worker: &DbWorker,
    account: AccountId,
    txn: &str,
    minor: i64,
    description: &str,
) -> (Vec<WriteCommand>, WriteCommand) {
    let connection = Uuid::from_u128(CONNECTION);
    let (_, record, staged) = stage(
        worker,
        "simplefin",
        None,
        Some(connection),
        Some(txn),
        &simplefin_json(description),
    );
    let id = key(worker)
        .provider_txn_id(&connection, "ACT-9", txn)
        .unwrap();
    let commit = WriteCommand::CommitImportedTransaction {
        transaction_id: TransactionId::from_uuid(id),
        account_id: account,
        source_record_id: record,
        occurred_at: when(),
        amount: Money::new(minor, Currency::Usd),
        counterparty: None,
        memo: Some(description.to_owned()),
        transaction_date: None,
        category_id: None,
        forced: false,
        identity: Some(ImportIdentityClaim {
            identity_id: id,
            components: ImportIdentityComponents::Provider {
                connection_id: connection,
                provider_account_key: "ACT-9".to_owned(),
                provider_txn_id: txn.to_owned(),
            },
        }),
    };
    (staged, commit)
}

/// A file row: the commit for ordinal `ordinal` of the file `FINGERPRINT`, in a batch of its own (an
/// upload), with the ledger id of that batch and ordinal.
fn file_row(
    worker: &DbWorker,
    account: AccountId,
    ordinal: u32,
    minor: i64,
) -> (SourceBatchId, WriteCommand) {
    let (batch, record, _) = stage(worker, "csv", Some(FINGERPRINT), None, None, "{}");
    let fingerprint = core_ids::FileFingerprint::parse(FINGERPRINT).unwrap();
    let key = key(worker);
    let ledger_id = key
        .content_id(
            "txn.import",
            &[
                batch.as_uuid().as_bytes(),
                i64::from(ordinal).to_be_bytes().as_slice(),
            ],
        )
        .unwrap();
    let commit = WriteCommand::CommitImportedTransaction {
        transaction_id: TransactionId::from_uuid(ledger_id),
        account_id: account,
        source_record_id: record,
        occurred_at: when(),
        amount: Money::new(minor, Currency::Usd),
        counterparty: None,
        memo: None,
        transaction_date: None,
        category_id: None,
        forced: false,
        identity: Some(ImportIdentityClaim {
            identity_id: key.file_row_id(&fingerprint, ordinal).unwrap(),
            components: ImportIdentityComponents::File {
                file_fingerprint: FINGERPRINT.to_owned(),
                row_ordinal: ordinal,
            },
        }),
    };
    (batch, commit)
}

fn count(worker: &DbWorker, sql: &str) -> i64 {
    worker
        .read_connection()
        .unwrap()
        .query_row(sql, [], |r| r.get(0))
        .unwrap()
}

fn imported_transactions(worker: &DbWorker) -> i64 {
    count(
        worker,
        "SELECT COUNT(*) FROM ledger_transactions lt WHERE EXISTS (
             SELECT 1 FROM source_provenance_links pl
              WHERE pl.entity_id = lt.id AND pl.relationship = 'created_from')",
    )
}

fn rejected(result: Result<crate::Outcome, DbError>) -> RejectCode {
    match result {
        Err(DbError::Rejected { code, .. }) => code,
        other => panic!("expected a deterministic rejection, got {other:?}"),
    }
}

#[test]
fn a_provider_commit_records_its_identity_and_the_decision_names_it() {
    let (_dir, worker) = worker();
    let account = account(&worker, "Checking");
    let (_, commit) = provider_row(&worker, account, "TXN-1", -1_234, "COFFEE");
    let WriteCommand::CommitImportedTransaction { transaction_id, .. } = &commit else {
        unreachable!()
    };
    let id = transaction_id.as_uuid();
    worker.dispatch(meta(), commit).unwrap();

    let conn = worker.read_connection().unwrap();
    let identity: (Uuid, String, Uuid, i64, String, String, Option<String>) = conn
        .query_row(
            "SELECT transaction_id, kind, account_id, amount_minor, currency, posted_date,
                    provider_description
               FROM import_identities WHERE identity_id = ?1",
            [id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        identity,
        (
            id,
            "provider".to_owned(),
            account.as_uuid(),
            -1_234,
            "USD".to_owned(),
            "2026-03-04".to_owned(),
            Some("COFFEE".to_owned())
        ),
        "a provider row's ledger id IS its derived id, and the snapshot is what was committed"
    );
    let decision: (Option<Uuid>, Option<Uuid>, String) = conn
        .query_row(
            "SELECT transaction_id, identity_id, decision FROM dedupe_decisions",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        decision,
        (Some(id), Some(id), "committed".to_owned()),
        "the committed decision carries the identity in the same apply"
    );
}

#[test]
fn two_commits_of_one_provider_identity_meet_as_a_no_op_and_one_merged_decision() {
    let (_dir, worker) = worker();
    let account = account(&worker, "Checking");
    let (_, first) = provider_row(&worker, account, "TXN-1", -1_234, "COFFEE");
    // Another device's refresh (or this device's next one): its own batch and record, one provider
    // transaction.
    let (_, second) = provider_row(&worker, account, "TXN-1", -1_234, "COFFEE");
    let (_, third) = provider_row(&worker, account, "TXN-1", -1_234, "COFFEE");
    worker.dispatch(meta(), first).unwrap();
    assert_eq!(imported_transactions(&worker), 1);
    worker.dispatch(meta(), second).unwrap();
    worker.dispatch(meta(), third).unwrap();

    assert_eq!(imported_transactions(&worker), 1, "no second transaction");
    assert_eq!(
        count(
            &worker,
            "SELECT COUNT(*) FROM dedupe_decisions WHERE decision = 'merged'"
        ),
        1,
        "one merged decision, whose id derives from the identity, so a repeat writes nothing"
    );
    assert_eq!(
        count(
            &worker,
            "SELECT COUNT(*) FROM audit_events WHERE event_type = 'import.identity_merged'"
        ),
        2,
        "one audit line per meeting"
    );
    assert_eq!(count(&worker, "SELECT COUNT(*) FROM import_identities"), 1);
    assert_eq!(
        count(&worker, "SELECT COUNT(*) FROM import_dispositions"),
        0,
        "equal fields raise nothing"
    );
}

#[test]
fn a_differing_provider_commit_records_one_revision_and_commits_nothing() {
    let (_dir, worker) = worker();
    let account = account(&worker, "Checking");
    let (_, first) = provider_row(&worker, account, "TXN-1", -1_234, "COFFEE");
    let (_, revised) = provider_row(&worker, account, "TXN-1", -1_300, "COFFEE");
    let (_, revised_again) = provider_row(&worker, account, "TXN-1", -1_300, "COFFEE SHOP");
    worker.dispatch(meta(), first).unwrap();
    worker.dispatch(meta(), revised).unwrap();

    assert_eq!(imported_transactions(&worker), 1, "nothing is committed");
    let disposition = |worker: &DbWorker| -> (String, String, String, i64, Option<String>) {
        worker
            .read_connection()
            .unwrap()
            .query_row(
                "SELECT kind, status, detail, generation, outcome FROM import_dispositions",
                [],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(2).map(|d: Option<String>| d.unwrap_or_default())?,
                        r.get(1)?,
                        r.get(3)?,
                        r.get(4)?,
                    ))
                },
            )
            .map(
                |(kind, detail, status, generation, outcome): (
                    String,
                    String,
                    String,
                    i64,
                    Option<String>,
                )| { (kind, status, detail, generation, outcome) },
            )
            .unwrap()
    };
    assert_eq!(
        disposition(&worker),
        (
            "provider_revision".to_owned(),
            "pending".to_owned(),
            "amount".to_owned(),
            1,
            None
        )
    );
    // A second differing revision updates the one pending disposition's detail, in command order.
    worker.dispatch(meta(), revised_again).unwrap();
    assert_eq!(
        count(&worker, "SELECT COUNT(*) FROM import_dispositions"),
        1
    );
    assert_eq!(disposition(&worker).2, "amount,description");
    // The identity still holds what was committed first: a revision is recorded, never applied.
    assert_eq!(
        count(
            &worker,
            "SELECT amount_minor FROM import_identities LIMIT 1"
        ),
        -1_234
    );
    assert_eq!(
        count(
            &worker,
            "SELECT COUNT(*) FROM dedupe_decisions WHERE decision = 'merged'"
        ),
        0
    );
}

#[test]
fn a_voided_identitys_transaction_is_never_resurrected_by_a_refresh() {
    let (_dir, worker) = worker();
    let account = account(&worker, "Checking");
    let (_, first) = provider_row(&worker, account, "TXN-1", -1_234, "COFFEE");
    let (_, again) = provider_row(&worker, account, "TXN-1", -1_234, "COFFEE");
    let WriteCommand::CommitImportedTransaction { transaction_id, .. } = &first else {
        unreachable!()
    };
    let id = *transaction_id;
    worker.dispatch(meta(), first).unwrap();
    worker
        .dispatch(meta(), WriteCommand::VoidTransaction { transaction_id: id })
        .unwrap();
    let before = count(&worker, "SELECT COUNT(*) FROM ledger_transactions");
    worker.dispatch(meta(), again).unwrap();
    assert_eq!(
        count(&worker, "SELECT COUNT(*) FROM ledger_transactions"),
        before,
        "nothing is committed again"
    );
    assert_eq!(
        count(
            &worker,
            "SELECT COUNT(*) FROM dedupe_decisions WHERE decision = 'merged'"
        ),
        0,
        "and it is not even a merge"
    );
    assert_eq!(
        count(&worker, "SELECT COUNT(*) FROM import_dispositions"),
        0
    );
}

#[test]
fn a_claim_that_is_not_its_rows_own_is_refused_and_writes_nothing() {
    let (_dir, worker) = worker();
    let account = account(&worker, "Checking");
    let connection = Uuid::from_u128(CONNECTION);
    let key = key(&worker);
    let (_, record, _) = stage(
        &worker,
        "simplefin",
        None,
        Some(connection),
        Some("TXN-1"),
        &simplefin_json("COFFEE"),
    );
    let derived = |conn: &Uuid, account_key: &str, txn: &str| {
        key.provider_txn_id(conn, account_key, txn).unwrap()
    };
    let claim = |id: Uuid, conn: Uuid, account_key: &str, txn: &str| ImportIdentityClaim {
        identity_id: id,
        components: ImportIdentityComponents::Provider {
            connection_id: conn,
            provider_account_key: account_key.to_owned(),
            provider_txn_id: txn.to_owned(),
        },
    };
    let commit =
        |ledger: Uuid, identity: ImportIdentityClaim| WriteCommand::CommitImportedTransaction {
            transaction_id: TransactionId::from_uuid(ledger),
            account_id: account,
            source_record_id: record,
            occurred_at: when(),
            amount: Money::new(-500, Currency::Usd),
            counterparty: None,
            memo: None,
            transaction_date: None,
            category_id: None,
            forced: false,
            identity: Some(identity),
        };
    let own = derived(&connection, "ACT-9", "TXN-1");
    let other_row = derived(&connection, "ACT-9", "TXN-2");
    let other_connection = Uuid::from_u128(CONNECTION + 1);
    for (label, command) in [
        (
            "an id that is not the components' derived id",
            commit(own, claim(other_row, connection, "ACT-9", "TXN-1")),
        ),
        (
            "another row's identity (consistent, but not this record's)",
            commit(other_row, claim(other_row, connection, "ACT-9", "TXN-2")),
        ),
        (
            "another connection's identity",
            commit(
                derived(&other_connection, "ACT-9", "TXN-1"),
                claim(
                    derived(&other_connection, "ACT-9", "TXN-1"),
                    other_connection,
                    "ACT-9",
                    "TXN-1",
                ),
            ),
        ),
        (
            "another account's key",
            commit(
                derived(&connection, "ACT-1", "TXN-1"),
                claim(
                    derived(&connection, "ACT-1", "TXN-1"),
                    connection,
                    "ACT-1",
                    "TXN-1",
                ),
            ),
        ),
        (
            "a ledger id that is not the derived id",
            commit(Uuid::now_v7(), claim(own, connection, "ACT-9", "TXN-1")),
        ),
        (
            "a component that fails the byte rules",
            commit(own, claim(own, connection, "ACT\u{1}", "TXN-1")),
        ),
    ] {
        assert_eq!(
            rejected(worker.dispatch(meta(), command)),
            RejectCode::PayloadInvalid,
            "{label}"
        );
    }
    assert_eq!(count(&worker, "SELECT COUNT(*) FROM import_identities"), 0);
    assert_eq!(imported_transactions(&worker), 0);

    // Only a SimpleFIN transaction has a provider identity: a LunchFlow row (or any other source)
    // that claims one is refused, however consistent its claim is.
    for source_type in ["lunchflow", "csv"] {
        let (_, record, _) = stage(
            &worker,
            source_type,
            None,
            Some(connection),
            Some("TXN-1"),
            &simplefin_json("COFFEE"),
        );
        let command = WriteCommand::CommitImportedTransaction {
            transaction_id: TransactionId::from_uuid(own),
            account_id: account,
            source_record_id: record,
            occurred_at: when(),
            amount: Money::new(-500, Currency::Usd),
            counterparty: None,
            memo: None,
            transaction_date: None,
            category_id: None,
            forced: false,
            identity: Some(claim(own, connection, "ACT-9", "TXN-1")),
        };
        assert_eq!(
            rejected(worker.dispatch(meta(), command)),
            RejectCode::PayloadInvalid,
            "a provider claim on a {source_type} row"
        );
    }
    assert_eq!(count(&worker, "SELECT COUNT(*) FROM import_identities"), 0);

    // A file row's claim is bound the same way: the batch's own fingerprint, and the (batch, ordinal) id.
    let (_, record, _) = stage(&worker, "csv", Some(FINGERPRINT), None, None, "{}");
    let fingerprint = core_ids::FileFingerprint::parse(FINGERPRINT).unwrap();
    let id = key.file_row_id(&fingerprint, 1).unwrap();
    let file_commit = |ledger: Uuid, fingerprint_text: &str, ordinal: u32, identity: Uuid| {
        WriteCommand::CommitImportedTransaction {
            transaction_id: TransactionId::from_uuid(ledger),
            account_id: account,
            source_record_id: record,
            occurred_at: when(),
            amount: Money::new(-500, Currency::Usd),
            counterparty: None,
            memo: None,
            transaction_date: None,
            category_id: None,
            forced: false,
            identity: Some(ImportIdentityClaim {
                identity_id: identity,
                components: ImportIdentityComponents::File {
                    file_fingerprint: fingerprint_text.to_owned(),
                    row_ordinal: ordinal,
                },
            }),
        }
    };
    let other = "sha256:0000000000000000000000000000000000000000000000000000000000000001";
    let other_id = key
        .file_row_id(&core_ids::FileFingerprint::parse(other).unwrap(), 1)
        .unwrap();
    for (label, command) in [
        (
            "another file's fingerprint",
            file_commit(Uuid::now_v7(), other, 1, other_id),
        ),
        (
            "a ledger id that is not the batch's and the ordinal's",
            file_commit(Uuid::now_v7(), FINGERPRINT, 1, id),
        ),
        (
            "a malformed fingerprint",
            file_commit(Uuid::now_v7(), "sha256:ABC", 1, id),
        ),
        (
            "ordinal zero",
            file_commit(Uuid::now_v7(), FINGERPRINT, 0, id),
        ),
    ] {
        assert_eq!(
            rejected(worker.dispatch(meta(), command)),
            RejectCode::PayloadInvalid,
            "{label}"
        );
    }
    assert_eq!(count(&worker, "SELECT COUNT(*) FROM import_identities"), 0);
}

#[test]
fn two_uploads_of_one_file_row_meet_and_a_different_row_is_a_conflict_not_a_silent_drop() {
    let (_dir, worker) = worker();
    let account = account(&worker, "Checking");
    let (_, first) = file_row(&worker, account, 1, -700);
    let (_, same) = file_row(&worker, account, 1, -700);
    worker.dispatch(meta(), first).unwrap();
    worker.dispatch(meta(), same).unwrap();
    assert_eq!(imported_transactions(&worker), 1, "the same row, once");
    assert_eq!(
        count(
            &worker,
            "SELECT COUNT(*) FROM dedupe_decisions WHERE decision = 'merged'"
        ),
        1
    );

    // A routed import: another device imported the same file under a different account map, so the
    // ordinal names a different row. Neither a silent drop nor a second transaction.
    let other_account = self::account(&worker, "Savings");
    let (_, routed) = file_row(&worker, other_account, 1, -700);
    worker.dispatch(meta(), routed).unwrap();
    assert_eq!(imported_transactions(&worker), 1);
    let (_, changed_amount) = file_row(&worker, account, 1, -999);
    worker.dispatch(meta(), changed_amount).unwrap();
    assert_eq!(imported_transactions(&worker), 1);
    assert_eq!(
        count(
            &worker,
            "SELECT COUNT(*) FROM import_dispositions WHERE kind = 'file_row_conflict'
                AND status = 'pending'"
        ),
        1,
        "one pending conflict per identity, never surfaced"
    );
    let detail: String = worker
        .read_connection()
        .unwrap()
        .query_row("SELECT detail FROM import_dispositions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(detail, "amount", "the latest differing commit's fields");
}

fn engine() -> EngineVersion {
    EngineVersion::new("dohflow-engine-1").unwrap()
}

/// N6: a second device whose class-1 state came from the first one's snapshot, with no staging of its
/// own, meets the same provider transaction through its own refresh: apply finds the identity the
/// snapshot carried and does nothing but record the meeting.
#[test]
fn a_replica_bootstrapped_from_a_snapshot_meets_the_same_provider_transaction_as_a_no_op() {
    let (_dir, origin) = worker();
    seed_golden_vault(&origin);
    let account = account(&origin, "Checking");
    let (batch_commands, commit) = provider_row(&origin, account, "TXN-1", -1_234, "COFFEE");
    let WriteCommand::CommitImportedTransaction { transaction_id, .. } = &commit else {
        unreachable!()
    };
    let id = transaction_id.as_uuid();
    // The snapshot a replica is born from, taken before the origin imports.
    let verified = verify(
        &export_genesis_plaintext(&origin.read_connection().unwrap(), &ExportInput::default())
            .unwrap(),
    )
    .unwrap();
    origin.dispatch(meta(), commit.clone()).unwrap();

    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("vault.db");
    let dek = Arc::new(vault_crypto::generate_dek().unwrap());
    let (version, own) = (engine(), engine());
    let replica = bootstrap_replica(
        &path,
        &dek,
        &ReplicaSource {
            vault_id: *origin.vault_id().unwrap().as_bytes(),
            root: 4,
            snapshot: &verified,
            engine_version: &version,
            own_engine_version: &own,
            held_voids: &[],
            closure: None,
        },
    )
    .unwrap()
    .worker;
    // The replica pulls what the origin did (its batch, its record and the commit), as envelopes.
    let mut base = 4;
    for command in batch_commands.iter().chain(std::iter::once(&commit)) {
        base += 1;
        replica
            .dispatch_rebase_step(
                meta(),
                command,
                StepContext {
                    applied_base_seq: Some(base),
                    rebased_from: None,
                    record_outbox: false,
                },
            )
            .unwrap();
    }
    assert_eq!(imported_transactions(&replica), 1);
    assert_eq!(
        count(&replica, "SELECT COUNT(*) FROM import_identities"),
        1,
        "the identity arrived with the commit"
    );

    // The replica's own refresh of the same provider transaction: its own batch and record.
    let (_, own_commit) = provider_row(&replica, account, "TXN-1", -1_234, "COFFEE");
    replica.dispatch(meta(), own_commit).unwrap();
    assert_eq!(imported_transactions(&replica), 1, "no second transaction");
    assert_eq!(
        count(
            &replica,
            "SELECT COUNT(*) FROM dedupe_decisions WHERE decision = 'merged'"
        ),
        1
    );
    let held: Uuid = replica
        .read_connection()
        .unwrap()
        .query_row("SELECT transaction_id FROM import_identities", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(held, id, "both devices name one ledger transaction");
    assert_eq!(
        count(&replica, "SELECT COUNT(*) FROM import_dispositions"),
        0,
        "and no revision is recorded for an unchanged provider transaction"
    );
}

/// A provider row's posted date and description are the record's own (the backfill's parser), not the
/// instant the commit carries, so a commit dated a day later is the same provider transaction.
#[test]
fn a_provider_snapshot_takes_its_date_from_the_record_not_from_the_commit() {
    let (_dir, worker) = worker();
    let account = account(&worker, "Checking");
    let (_, mut commit) = provider_row(&worker, account, "TXN-1", -1_234, "COFFEE");
    if let WriteCommand::CommitImportedTransaction { occurred_at, .. } = &mut commit {
        *occurred_at = Utc.with_ymd_and_hms(2026, 3, 9, 15, 0, 0).unwrap();
    }
    worker.dispatch(meta(), commit).unwrap();
    let date: String = worker
        .read_connection()
        .unwrap()
        .query_row("SELECT posted_date FROM import_identities", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(date, "2026-03-04", "the record's posted epoch, as UTC");
    // The same transaction again, dated as the record dates it: equal, not a revision.
    let (_, again) = provider_row(&worker, account, "TXN-1", -1_234, "COFFEE");
    worker.dispatch(meta(), again).unwrap();
    assert_eq!(
        count(&worker, "SELECT COUNT(*) FROM import_dispositions"),
        0
    );
}
