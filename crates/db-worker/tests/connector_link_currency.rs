//! Connector link currency (personal-cfo-049p6, migration 54): discovery
//! records the provider's currency, a later refresh backfills a link saved
//! with none, and an unknown report never overwrites a known currency.

mod common;

use common::*;
use core_ledger::{Account, AccountFlags, AccountId, CashflowRole, LedgerAccountId};
use core_money::Currency;
use db_worker::WriteCommand;
use uuid::Uuid;

fn usd_account(worker: &db_worker::DbWorker) -> Uuid {
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
    account_id.as_uuid()
}

fn connection(worker: &db_worker::DbWorker) -> Uuid {
    let id = Uuid::now_v7();
    worker
        .create_connector_connection(id, "other", "mock-access-url", None)
        .unwrap();
    id
}

fn currency(
    worker: &db_worker::DbWorker,
    connection_id: Uuid,
    external_id: &str,
) -> Option<String> {
    worker
        .connector_link_currency(connection_id, external_id)
        .unwrap()
        .expect("link exists")
}

#[test]
fn a_legacy_unknown_currency_is_backfilled_and_never_unset() {
    let (_dir, worker) = worker();
    let conn = connection(&worker);
    // A link saved with no currency (as every pre-migration link reads).
    worker
        .upsert_connector_link(conn, "ACT-1", Some("Checking"), None)
        .unwrap();
    assert_eq!(currency(&worker, conn, "ACT-1"), None);

    // The next refresh reports one: recorded.
    worker
        .upsert_connector_link(conn, "ACT-1", Some("Checking"), Some("EUR"))
        .unwrap();
    assert_eq!(currency(&worker, conn, "ACT-1").as_deref(), Some("EUR"));

    // A later refresh that states none never unsets it.
    worker
        .upsert_connector_link(conn, "ACT-1", Some("Checking renamed"), None)
        .unwrap();
    assert_eq!(currency(&worker, conn, "ACT-1").as_deref(), Some("EUR"));
    let links = worker.connector_links(conn).unwrap();
    assert_eq!(links[0].external_name.as_deref(), Some("Checking renamed"));
    assert_eq!(links[0].currency.as_deref(), Some("EUR"));

    // A provider that corrects it is believed.
    worker
        .upsert_connector_link(conn, "ACT-1", Some("Checking"), Some("USD"))
        .unwrap();
    assert_eq!(currency(&worker, conn, "ACT-1").as_deref(), Some("USD"));
}

#[test]
fn the_schema_admits_only_uppercase_three_letter_codes() {
    let (_dir, worker) = worker();
    let conn = connection(&worker);
    assert!(worker
        .upsert_connector_link(conn, "ACT-1", None, Some("usd"))
        .is_err());
    assert!(worker
        .upsert_connector_link(conn, "ACT-2", None, Some("DOLLARS"))
        .is_err());
    assert_eq!(
        worker.connector_link_currency(conn, "ACT-9").unwrap(),
        None,
        "no such link"
    );
}

#[test]
fn remapping_keeps_the_recorded_currency() {
    let (_dir, worker) = worker();
    let conn = connection(&worker);
    worker
        .upsert_connector_link(conn, "ACT-1", Some("Checking"), Some("USD"))
        .unwrap();
    let account = usd_account(&worker);
    worker
        .set_connector_link_account(conn, "ACT-1", Some(account))
        .unwrap();
    worker
        .set_connector_link_account(conn, "ACT-1", None)
        .unwrap();
    assert_eq!(currency(&worker, conn, "ACT-1").as_deref(), Some("USD"));
}
