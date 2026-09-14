//! Import staging, dedupe, commit/skip, and the Money Inbox read model
//! (moved verbatim from the old in-file `lib.rs` tests module).

mod common;

use chrono::{NaiveDate, Utc};
use common::*;
use core_ledger::{
    Account, AccountFlags, AccountId, CashflowRole, CategoryId, LedgerAccountId, SourceBatchId,
    SourceRecordId, TransactionId,
};
use core_money::{Currency, Money};
use db_worker::*;
use rusqlite::params;
use uuid::Uuid;

#[test]
fn money_inbox_surfaces_unreviewed_transactions() {
    let (_dir, worker) = worker();
    let account_id = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(Account::new(
                    account_id,
                    LedgerAccountId::new(),
                    "Checking",
                    CashflowRole::LiquidCash,
                    Currency::Usd,
                    AccountFlags::default(),
                )),
                opening_balance: None,
            },
        )
        .unwrap();
    let dt = |day: u32| {
        NaiveDate::from_ymd_opt(2026, 6, day)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
    };
    let record = |amount: i64, day: u32| WriteCommand::RecordTransaction {
        transaction_id: TransactionId::new(),
        account_id,
        amount: Money::new(amount, Currency::Usd),
        occurred_at: dt(day),
    };
    let txn_with_amount = |amount: i64| {
        worker
            .recent_transactions(50)
            .unwrap()
            .into_iter()
            .find(|r| r.amount.minor_units() == amount)
            .unwrap()
            .transaction_id
    };
    let unreviewed_ids = || {
        worker
            .money_inbox_list()
            .unwrap()
            .into_iter()
            .filter(|i| i.item_kind == "unreviewed_transaction")
            .map(|i| i.target_id)
            .collect::<Vec<_>>()
    };

    // A manual transaction is reviewed → NOT surfaced; an imported one is unreviewed.
    worker.dispatch(meta(), record(-4_000, 20)).unwrap();
    worker.dispatch(meta(), record(-2_000, 21)).unwrap();
    let imported_id = txn_with_amount(-2_000);
    {
        let conn = worker.read_connection().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=OFF;").unwrap();
        conn.execute(
            "INSERT INTO source_provenance_links
                    (id, entity_type, entity_id, source_record_id, relationship, created_at)
                 VALUES (?1, 'ledger_transaction', ?2, ?3, 'created_from', ?4)",
            params![
                Uuid::now_v7(),
                imported_id.as_uuid(),
                Uuid::now_v7(),
                Utc::now().to_rfc3339()
            ],
        )
        .unwrap();
    }
    assert_eq!(
        unreviewed_ids(),
        vec![imported_id.as_uuid()],
        "only the imported (unreviewed) transaction surfaces",
    );

    // Marking it reviewed clears it.
    worker
        .dispatch(
            meta(),
            WriteCommand::MarkReviewed {
                transaction_id: imported_id,
                reviewed: true,
            },
        )
        .unwrap();
    assert!(unreviewed_ids().is_empty(), "reviewing clears the item");

    // Un-reviewing a manual transaction surfaces it.
    let manual_id = txn_with_amount(-4_000);
    worker
        .dispatch(
            meta(),
            WriteCommand::MarkReviewed {
                transaction_id: manual_id,
                reviewed: false,
            },
        )
        .unwrap();
    assert_eq!(unreviewed_ids(), vec![manual_id.as_uuid()]);
}

/// Low-confidence review queue (ADR 0030 addendum, personal-cfo-j5ij/-uc95): a
/// transaction auto-categorized below the 70% threshold AND unreviewed surfaces as a
/// `low_confidence_category` money-inbox item (not also a generic unreviewed one);
/// accepting it (mark reviewed) drops it; the bulk-accept id list matches.
#[test]
fn low_confidence_categories_surface_in_money_inbox_and_clear_on_review() {
    let (_dir, worker) = worker();
    let account_id = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(account_id)),
                opening_balance: Some(Money::new(5_000_000, Currency::Usd)),
            },
        )
        .unwrap();
    let cats: Vec<Uuid> = {
        let conn = worker.read_connection().unwrap();
        let mut stmt = conn
            .prepare("SELECT id FROM categories WHERE forecast_behavior='variable_regular' LIMIT 2")
            .unwrap();
        let rows = stmt.query_map([], |r| r.get::<_, Uuid>(0)).unwrap();
        rows.map(Result::unwrap).collect()
    };
    assert!(cats.len() >= 2, "need two seeded categories");

    // ShopMart: 2 user→catA, 1 user→catB ⇒ winner catA at 2/3 = 6666 bps (< 7000).
    let d = |day| NaiveDate::from_ymd_opt(2026, 3, day).unwrap();
    let s1 = record_with_counterparty(&worker, account_id, -2000, d(2), "SHOPMART #1");
    let s2 = record_with_counterparty(&worker, account_id, -2500, d(4), "SHOPMART #2");
    let s3 = record_with_counterparty(&worker, account_id, -1500, d(6), "SHOPMART #3");
    for (txn, cat) in [(s1, cats[0]), (s2, cats[0]), (s3, cats[1])] {
        worker
            .dispatch(
                meta(),
                WriteCommand::RecategorizeTransaction {
                    transaction_id: txn,
                    category_id: Some(CategoryId::from_uuid(cat)),
                },
            )
            .unwrap();
    }
    // A 4th ShopMart, made unreviewed (as an import would be) so the auto-apply fills it.
    let s4 = record_with_counterparty(&worker, account_id, -2200, d(10), "SHOPMART #4");
    worker
        .dispatch(
            meta(),
            WriteCommand::MarkReviewed {
                transaction_id: s4,
                reviewed: false,
            },
        )
        .unwrap();

    assert_eq!(worker.apply_merchant_memory().unwrap(), 1);
    assert_eq!(category_of(&worker, s4), Some((cats[0], "rule".to_owned())));

    // Exactly one low-confidence-category item, for s4, carrying its agreement confidence.
    let low: Vec<_> = worker
        .money_inbox_list()
        .unwrap()
        .into_iter()
        .filter(|item| item.item_kind == "low_confidence_category")
        .collect();
    assert_eq!(low.len(), 1);
    assert_eq!(low[0].target_id, s4.as_uuid());
    assert!(low[0].payload_json.contains("\"confidence_bps\":6666"));
    // Dedup: s4 is not also surfaced as a generic unreviewed item.
    assert!(worker.money_inbox_list().unwrap().iter().all(|item| {
        !(item.item_kind == "unreviewed_transaction" && item.target_id == s4.as_uuid())
    }));
    // The bulk-accept id list matches the queue.
    assert_eq!(
        worker.low_confidence_category_transaction_ids().unwrap(),
        vec![s4.as_uuid()]
    );

    // Accept = mark reviewed → drops from the queue.
    worker
        .dispatch(
            meta(),
            WriteCommand::MarkReviewed {
                transaction_id: s4,
                reviewed: true,
            },
        )
        .unwrap();
    assert!(worker
        .money_inbox_list()
        .unwrap()
        .iter()
        .all(|item| item.item_kind != "low_confidence_category"));
}

/// 3bb: a source batch driven through its lifecycle records exactly one
/// op-log entry per state transition (create + each update_batch_state).
#[test]
fn source_batch_lifecycle_records_one_oplog_per_transition() {
    let (_dir, worker) = worker();
    let batch_id = SourceBatchId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateSourceBatch {
                id: batch_id,
                source_type: "csv".to_owned(),
                source_name: Some("statement.csv".to_owned()),
                file_fingerprint: Some("sha256:file".to_owned()),
                parser_version: Some("csv-v1".to_owned()),
            },
        )
        .unwrap();
    worker
        .dispatch(
            meta(),
            WriteCommand::UpdateBatchState {
                batch_id,
                status: "staged".to_owned(),
                staged_count: 2,
                committed_count: 0,
                skipped_count: 0,
            },
        )
        .unwrap();
    worker
        .dispatch(
            meta(),
            WriteCommand::UpdateBatchState {
                batch_id,
                status: "committed".to_owned(),
                staged_count: 2,
                committed_count: 2,
                skipped_count: 0,
            },
        )
        .unwrap();

    let conn = worker.read_connection().unwrap();
    // create + 2 transitions = exactly 3 op-log entries.
    assert_eq!(conn.operation_count().unwrap(), 3);
    let (status, committed): (String, i64) = conn
        .query_row(
            "SELECT status, committed_count FROM source_batches WHERE id = ?1",
            rusqlite::params![batch_id.as_uuid()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(status, "committed");
    assert_eq!(committed, 2);
}

/// 3bb: re-attaching the same `source_hash` to a batch is content-deduped —
/// no duplicate source_record (re-importing a file produces no new records).
#[test]
fn attaching_the_same_source_hash_is_idempotent() {
    let (_dir, worker) = worker();
    let batch_id = SourceBatchId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateSourceBatch {
                id: batch_id,
                source_type: "csv".to_owned(),
                source_name: None,
                file_fingerprint: None,
                parser_version: None,
            },
        )
        .unwrap();
    let attach = |id: SourceRecordId| WriteCommand::AttachSourceRecord {
        id,
        batch_id,
        external_id: None,
        source_hash: "sha256:row1".to_owned(),
        normalized_json: "{\"amount\":-1299}".to_owned(),
        parse_confidence_bps: Some(10_000),
    };
    // Two distinct commands (different ids + idempotency keys), same content.
    worker
        .dispatch(meta(), attach(SourceRecordId::new()))
        .unwrap();
    worker
        .dispatch(meta(), attach(SourceRecordId::new()))
        .unwrap();

    let conn = worker.read_connection().unwrap();
    let records: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM source_records WHERE source_batch_id = ?1",
            rusqlite::params![batch_id.as_uuid()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        records, 1,
        "re-attaching the same source_hash adds no new record"
    );
}

/// r52x: an account whose latest manual balance is older than its role's
/// threshold surfaces a stale-balance Money Inbox item; a fresh or
/// never-asserted account does not, and archiving drops it (ADR 0014 §7).
#[test]
fn stale_balance_accounts_surface_in_the_money_inbox() {
    let (_dir, worker) = worker();
    let liquid = |name: &str| {
        let account = Account::new(
            AccountId::new(),
            LedgerAccountId::new(),
            name,
            CashflowRole::LiquidCash,
            Currency::Usd,
            AccountFlags::default(),
        );
        let id = account.id();
        worker
            .dispatch(
                meta(),
                WriteCommand::CreateAccount {
                    account: Box::new(account),
                    opening_balance: None,
                },
            )
            .unwrap();
        id
    };
    let stale = liquid("Stale");
    let fresh = liquid("Fresh");
    let _never = liquid("Never"); // no observation → not flagged

    // Seed manual balance observations directly (an old one + a fresh one).
    let today = Utc::now().date_naive();
    {
        let conn = worker.read_connection().unwrap();
        let insert = |account: AccountId, date: NaiveDate| {
            conn.execute(
                "INSERT INTO balance_observations
                        (id, account_id, observed_at, balance_amount_minor,
                         balance_currency, source, source_record_id,
                         reconciliation_session_id, created_at)
                     VALUES (?1, ?2, ?3, 10000, 'USD', 'manual', NULL, NULL, ?4)",
                params![
                    Uuid::now_v7(),
                    account.as_uuid(),
                    date.to_string(),
                    Utc::now().to_rfc3339()
                ],
            )
            .unwrap();
        };
        insert(stale, today - chrono::Duration::days(40));
        insert(fresh, today);
    }

    let stale_items: Vec<_> = worker
        .money_inbox_list()
        .unwrap()
        .into_iter()
        .filter(|i| i.item_kind == "stale_balance")
        .collect();
    assert_eq!(stale_items.len(), 1, "only the stale account is flagged");
    assert_eq!(stale_items[0].target_id, stale.as_uuid());

    // Archiving the stale account drops the item (intrinsic resolution).
    worker
        .dispatch(meta(), WriteCommand::ArchiveAccount(stale))
        .unwrap();
    assert!(
        worker
            .money_inbox_list()
            .unwrap()
            .iter()
            .all(|i| i.item_kind != "stale_balance"),
        "archived account no longer flagged"
    );
}

/// ci71: the Money Inbox list hides dismissed items and snoozed items whose
/// snooze date is still in the future; an expired snooze re-appears (ADR 0014 §7).
#[test]
fn money_inbox_list_hides_snoozed_and_dismissed_items() {
    let (_dir, worker) = worker();
    let today = Utc::now().date_naive();
    let insert = |snoozed: Option<String>, dismissed: Option<String>| -> Uuid {
        let id = Uuid::now_v7();
        let conn = worker.read_connection().unwrap();
        conn.execute(
            "INSERT INTO money_inbox_read_model
                    (item_id, item_kind, target_table, target_id, priority, surfaced_at,
                     snoozed_until, dismissed_at, resolved_at, payload_json)
                 VALUES (?1, 'imported_waiting_commit', 'staged_transactions', ?1, 20, ?2,
                         ?3, ?4, NULL, '{}')",
            params![id, today.to_string(), snoozed, dismissed],
        )
        .unwrap();
        id
    };
    let visible = insert(None, None);
    let snoozed_future = insert(Some((today + chrono::Duration::days(5)).to_string()), None);
    let snoozed_past = insert(Some((today - chrono::Duration::days(1)).to_string()), None);
    let dismissed = insert(None, Some("2026-01-01T00:00:00Z".to_owned()));

    let ids: Vec<Uuid> = worker
        .money_inbox_list()
        .unwrap()
        .iter()
        .map(|i| i.item_id)
        .collect();
    assert!(ids.contains(&visible), "an active item is shown");
    assert!(ids.contains(&snoozed_past), "an expired snooze re-appears");
    assert!(!ids.contains(&snoozed_future), "a future snooze is hidden");
    assert!(!ids.contains(&dismissed), "a dismissed item is hidden");
}

/// personal-cfo-zfyo: a connection whose last sync recorded an error surfaces
/// a derived `connector_error` inbox item; a later successful sync clears
/// `last_error` and the item resolves intrinsically on the next read.
#[test]
fn connector_error_item_surfaces_and_resolves_intrinsically() {
    let (_dir, worker) = worker();
    let connection_id = Uuid::now_v7();
    worker
        .create_connector_connection(
            connection_id,
            "simplefin",
            "https://user:pw@bridge.example/simplefin",
            Some("Test connection"),
        )
        .unwrap();

    // Healthy connection: no item.
    assert!(worker
        .money_inbox_list()
        .unwrap()
        .iter()
        .all(|item| item.item_kind != "connector_error"));

    // A failed sync records an error -> the item appears with the connection
    // as its deterministic id and NO credential in the payload.
    worker
        .record_connector_error(connection_id, "access revoked — re-link required")
        .unwrap();
    let items = worker.money_inbox_list().unwrap();
    let item = items
        .iter()
        .find(|item| item.item_kind == "connector_error")
        .expect("connector_error item surfaces");
    assert_eq!(item.item_id, connection_id);
    assert_eq!(item.target_table, "connector_connections");
    assert!(item.payload_json.contains("access revoked"));
    assert!(
        !item.payload_json.contains("bridge.example"),
        "credential must never enter the payload: {}",
        item.payload_json
    );

    // A successful sync clears the error -> intrinsic resolution.
    worker.record_connector_sync(connection_id, None).unwrap();
    assert!(worker
        .money_inbox_list()
        .unwrap()
        .iter()
        .all(|item| item.item_kind != "connector_error"));
}
