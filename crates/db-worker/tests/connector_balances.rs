//! Connector balance promotion + anchoring (personal-cfo-yl53, ADR 0027
//! addendum 2026-09-02): a synced provider balance becomes a
//! `balance_observations` row in the staging transaction, anchors the
//! displayed balance, and resolves the stale_balance inbox item.

mod common;

use chrono::{Days, Utc};
use common::*;
use core_ledger::{Account, AccountFlags, AccountId, CashflowRole, LedgerAccountId};
use core_money::{Currency, Money};
use db_worker::*;
use importer_core::{ParsedAccount, ParsedBalance, ParsedBatch, ParsedRecord};
use rusqlite::params;
use uuid::Uuid;

fn account_with(worker: &DbWorker, role: CashflowRole, currency: Currency) -> AccountId {
    let account_id = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(Account::new(
                    account_id,
                    LedgerAccountId::new(),
                    "Checking",
                    role,
                    currency,
                    AccountFlags::default(),
                )),
                opening_balance: None,
            },
        )
        .unwrap();
    account_id
}

fn liquid_account(worker: &DbWorker) -> AccountId {
    account_with(worker, CashflowRole::LiquidCash, Currency::Usd)
}

/// A one-record sync batch carrying only a balance for `external`, observed at
/// `observed` — the connector shape after the transaction walk is filtered out.
fn balance_batch(external: &str, observed: chrono::NaiveDate, minor: i64) -> ParsedBatch {
    ParsedBatch {
        source_format: "other".to_owned(),
        accounts: vec![ParsedAccount {
            external_id: Some(external.to_owned()),
            external_name: Some("Ext Checking".to_owned()),
            external_number_hash: None,
            proposed_subtype: None,
        }],
        records: vec![ParsedRecord {
            external_id: Some(format!("{external}-bal")),
            source_hash: format!("sha256:{external}-bal-{observed}"),
            normalized_json: "{}".to_owned(),
            parse_confidence_bps: Some(10_000),
            transaction: None,
            balance: Some(ParsedBalance {
                observed_at: observed,
                amount: Money::new(minor, Currency::Usd),
                external_account: Some(external.to_owned()),
            }),
        }],
        warnings: Vec::new(),
    }
}

fn stage_batch(worker: &DbWorker, account: AccountId, batch: &ParsedBatch) {
    let batch_id = Uuid::now_v7();
    let conn = worker.read_connection().unwrap();
    conn.execute(
        "INSERT INTO source_batches
            (id, source_type, source_name, status, created_at, updated_at)
         VALUES (?1, 'other', 'test sync', 'staged', ?2, ?2)",
        params![batch_id, Utc::now().to_rfc3339()],
    )
    .unwrap();
    let map =
        std::collections::BTreeMap::from([("ext-1".to_owned(), account.as_uuid().to_owned())]);
    worker.stage_sync_batch(batch_id, batch, &map).unwrap();
}

#[test]
fn synced_balance_promotes_anchors_and_resolves_stale() {
    let (_dir, worker) = worker();
    let account = liquid_account(&worker);
    let today = Utc::now().date_naive();

    // A manual assertion 60 days old: the account is stale (liquid threshold
    // is 14 days) and its anchored balance is the old figure.
    let old = today.checked_sub_days(Days::new(60)).unwrap();
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            account,
            Money::new(50_000, Currency::Usd),
            old,
        )
        .unwrap();
    assert!(
        worker
            .money_inbox_list()
            .unwrap()
            .iter()
            .any(|i| i.item_kind == "stale_balance"),
        "60-day-old manual assertion must surface as stale"
    );

    // Sync a provider balance observed today.
    stage_batch(&worker, account, &balance_batch("ext-1", today, 123_456));

    // Promoted with provenance, staged row consumed.
    let conn = worker.read_connection().unwrap();
    let (count, source, provenance): (i64, String, Option<Uuid>) = conn
        .query_row(
            "SELECT COUNT(*), MAX(source), MAX(source_record_id)
               FROM balance_observations WHERE source = 'connector_sync'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(source, "connector_sync");
    assert!(provenance.is_some(), "observation keeps its source record");
    let staged: i64 = conn
        .query_row("SELECT COUNT(*) FROM staged_balances", [], |r| r.get(0))
        .unwrap();
    assert_eq!(staged, 0, "promoted staged balance is consumed");

    // The provider balance anchors the account (no postings after today).
    let view = worker
        .account_views()
        .unwrap()
        .into_iter()
        .find(|v| v.id == account)
        .unwrap();
    assert_eq!(view.balance.minor_units(), 123_456);

    // Stale item resolved intrinsically by the fresh connector observation.
    assert!(
        worker
            .money_inbox_list()
            .unwrap()
            .iter()
            .all(|i| i.item_kind != "stale_balance"),
        "a same-day connector observation is fresh"
    );

    // Same-day re-sync replaces rather than duplicates (last-wins per day).
    stage_batch(&worker, account, &balance_batch("ext-1", today, 130_000));
    let (count2, latest): (i64, i64) = conn
        .query_row(
            "SELECT COUNT(*),
                    (SELECT balance_amount_minor FROM balance_observations
                      WHERE source = 'connector_sync'
                      ORDER BY created_at DESC LIMIT 1)
               FROM balance_observations WHERE source = 'connector_sync'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(count2, 1, "same (account, day) observation is replaced");
    assert_eq!(latest, 130_000);
}

#[test]
fn manual_assertion_after_sync_wins_the_same_day() {
    let (_dir, worker) = worker();
    let account = liquid_account(&worker);
    let today = Utc::now().date_naive();

    stage_batch(&worker, account, &balance_batch("ext-1", today, 123_456));
    // The user overrides right after the sync: most recently recorded wins
    // (ADR 0027 addendum, same-day tie).
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            account,
            Money::new(99_999, Currency::Usd),
            today,
        )
        .unwrap();
    let view = worker
        .account_views()
        .unwrap()
        .into_iter()
        .find(|v| v.id == account)
        .unwrap();
    assert_eq!(view.balance.minor_units(), 99_999);
}

/// The connector-baseline rule (ADR 0027 addendum): a FIRST-ever observation
/// from a connector yields no plug — it is starting truth, not a discrepancy.
#[test]
fn first_connector_observation_is_a_baseline_not_a_plug() {
    let (_dir, worker) = worker();
    let account = liquid_account(&worker);
    let today = Utc::now().date_naive();
    stage_batch(&worker, account, &balance_batch("ext-1", today, 123_456));
    assert_eq!(worker.account_unexplained(account).unwrap(), None);
}

/// The fifth anchor site (yl53 review): converting the plug must date the
/// correction at the latest observation of EITHER source, so it lands inside
/// the window and zeroes the plug — no phantom-transaction stacking.
#[test]
fn convert_zeroes_a_plug_anchored_between_connector_and_manual_days() {
    let (_dir, worker) = worker();
    let account = liquid_account(&worker);
    let today = Utc::now().date_naive();
    let last_week = today.checked_sub_days(Days::new(7)).unwrap();

    // Connector baseline last week, manual assertion today: plug = 26,544.
    stage_batch(
        &worker,
        account,
        &balance_batch("ext-1", last_week, 100_000),
    );
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            account,
            Money::new(126_544, Currency::Usd),
            today,
        )
        .unwrap();
    assert_eq!(
        worker
            .account_unexplained(account)
            .unwrap()
            .map(|m| m.minor_units()),
        Some(26_544)
    );

    worker
        .dispatch(
            meta(),
            WriteCommand::ConvertUnexplainedToTransaction {
                account_id: account,
            },
        )
        .unwrap();
    let plug_after = worker
        .account_unexplained(account)
        .unwrap()
        .map(|m| m.minor_units());
    assert!(
        plug_after.is_none() || plug_after == Some(0),
        "conversion must zero the plug, got {plug_after:?}"
    );
    let conn = worker.read_connection().unwrap();
    let txns: i64 = conn
        .query_row("SELECT COUNT(*) FROM ledger_transactions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(txns, 1, "exactly one correction transaction");
}

/// Currency guard: a provider balance in another currency never anchors the
/// account — it stays staged, un-promoted (parity with the commit paths).
#[test]
fn cross_currency_balance_stays_staged() {
    let (_dir, worker) = worker();
    let account = liquid_account(&worker); // USD
    let today = Utc::now().date_naive();
    let mut batch = balance_batch("ext-1", today, 55_000);
    if let Some(bal) = &mut batch.records[0].balance {
        bal.amount = Money::new(55_000, Currency::Eur);
    }
    stage_batch(&worker, account, &batch);

    let conn = worker.read_connection().unwrap();
    let observations: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM balance_observations WHERE source = 'connector_sync'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(observations, 0, "EUR balance must not anchor a USD account");
    let staged: i64 = conn
        .query_row("SELECT COUNT(*) FROM staged_balances", [], |r| r.get(0))
        .unwrap();
    assert_eq!(staged, 1, "the mismatched balance stays staged for review");
}

/// Local-day clamp: a UTC-derived "tomorrow" observation is clamped to the
/// household-local today, so a manual assertion recorded later the same local
/// evening still wins its tie on created_at.
#[test]
fn future_dated_observation_clamps_to_local_today() {
    let (_dir, worker) = worker();
    let account = liquid_account(&worker);
    let tomorrow = Utc::now()
        .date_naive()
        .checked_add_days(Days::new(1))
        .unwrap();
    stage_batch(&worker, account, &balance_batch("ext-1", tomorrow, 123_456));
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            account,
            Money::new(99_999, Currency::Usd),
            Utc::now().date_naive(),
        )
        .unwrap();
    let view = worker
        .account_views()
        .unwrap()
        .into_iter()
        .find(|v| v.id == account)
        .unwrap();
    assert_eq!(
        view.balance.minor_units(),
        99_999,
        "the later manual assertion wins the same local day"
    );
}

/// Liability sign frame: a negative provider card balance anchors negative,
/// matching the manual convention (positive-owed entry stored negative).
#[test]
fn negative_card_balance_anchors_in_the_liability_frame() {
    let (_dir, worker) = worker();
    let card = account_with(&worker, CashflowRole::CreditFacility, Currency::Usd);
    let today = Utc::now().date_naive();
    stage_batch(&worker, card, &balance_batch("ext-1", today, -45_000));
    let view = worker
        .account_views()
        .unwrap()
        .into_iter()
        .find(|v| v.id == card)
        .unwrap();
    assert_eq!(view.balance.minor_units(), -45_000);
}
