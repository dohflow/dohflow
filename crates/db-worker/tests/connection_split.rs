//! The connection identity / device-local secret split (personal-cfo-07u, SYNC-2a; ADR 0074
//! decision 5 and Addendum B): a connection that arrived from another device has an identity and
//! mapping here and no token; entering a token writes only the local secret row; deleting is a
//! tombstone that scrubs this device's rows; refresh cursors are this device's own.

mod common;

use common::*;
use core_ledger::{Account, AccountFlags, AccountId, CashflowRole, LedgerAccountId};
use core_money::Currency;
use db_worker::{DbError, DbWorker, WriteCommand, MAX_LINK_EXTERNAL_ID_BYTES};
use uuid::Uuid;

const SECRET: &str = "https://user:pass@bridge.invalid/simplefin";

fn count(worker: &DbWorker, table: &str) -> i64 {
    worker
        .read_connection()
        .unwrap()
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

/// A real account to map a link onto (the mapping has a foreign key to `accounts`).
fn account(worker: &DbWorker, name: &str) -> Uuid {
    let id = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(Account::new(
                    id,
                    LedgerAccountId::new(),
                    name,
                    CashflowRole::LiquidCash,
                    Currency::Usd,
                    AccountFlags::default(),
                )),
                opening_balance: None,
            },
        )
        .unwrap();
    id.as_uuid()
}

/// A connection created here: identity and secret.
fn local(worker: &DbWorker) -> Uuid {
    let id = Uuid::now_v7();
    worker
        .create_connector_connection(id, "simplefin", SECRET, Some("Primary"))
        .unwrap();
    id
}

/// A connection that arrived from another device: identity only.
fn remote(worker: &DbWorker) -> Uuid {
    let id = Uuid::now_v7();
    worker
        .create_connector_identity(id, "simplefin", Some("Primary"))
        .unwrap();
    id
}

#[test]
fn an_identity_without_a_secret_lists_as_needing_a_token_and_has_no_credential() {
    let (_dir, worker) = worker();
    let id = remote(&worker);
    let row = worker
        .connector_connections()
        .unwrap()
        .into_iter()
        .find(|r| r.id == id)
        .unwrap();
    assert!(!row.has_secret);
    assert_eq!((row.last_synced_at, row.last_error), (None, None));
    assert_eq!(worker.connector_credential(id).unwrap(), None);
    assert!(worker.connector_connection_exists(id).unwrap());
    // Recording a sync outcome for a connection with no secret is a no-op, not an error and not a row.
    worker.record_connector_sync(id, None).unwrap();
    worker.record_connector_error(id, "boom").unwrap();
    assert_eq!(count(&worker, "connector_connection_secrets"), 0);
}

#[test]
fn entering_a_token_inserts_only_the_secret_row() {
    let (_dir, worker) = worker();
    let id = remote(&worker);
    worker
        .upsert_connector_link(id, "acct-1", Some("Checking"), Some("USD"))
        .unwrap();
    let before = (
        count(&worker, "connector_connections"),
        count(&worker, "connector_account_links"),
    );

    worker.set_connector_credential(id, SECRET).unwrap();

    assert_eq!(
        worker.connector_credential(id).unwrap().as_deref(),
        Some(SECRET)
    );
    assert!(worker.connector_connections().unwrap()[0].has_secret);
    assert_eq!(count(&worker, "connector_connection_secrets"), 1);
    assert_eq!(
        (
            count(&worker, "connector_connections"),
            count(&worker, "connector_account_links")
        ),
        before,
        "no second identity and no second mapping"
    );
    // Entering another token replaces the credential, clears the error and keeps the refresh time.
    worker.record_connector_sync(id, Some("a failure")).unwrap();
    worker
        .set_connector_credential(id, "https://other.invalid/")
        .unwrap();
    let row = &worker.connector_connections().unwrap()[0];
    assert_eq!(row.last_error, None);
    assert!(row.last_synced_at.is_some());
    assert_eq!(
        worker.connector_credential(id).unwrap().as_deref(),
        Some("https://other.invalid/")
    );
}

#[test]
fn a_token_cannot_be_entered_for_an_unknown_or_deleted_connection() {
    let (_dir, worker) = worker();
    assert!(matches!(
        worker.set_connector_credential(Uuid::now_v7(), SECRET),
        Err(DbError::InvalidCommand(_))
    ));
    let id = local(&worker);
    worker.delete_connector_connection(id).unwrap();
    assert!(matches!(
        worker.set_connector_credential(id, SECRET),
        Err(DbError::InvalidCommand(_))
    ));
    assert_eq!(count(&worker, "connector_connection_secrets"), 0);
}

#[test]
fn a_replicated_identity_and_mapping_never_make_a_second_link_or_account() {
    // The Sync-path double-count hazard: another device's identity and mapping arrive, then this
    // device discovers the same provider accounts. The mapping is keyed by (connection, external id),
    // so a second discovery updates the one row; entering a token adds no link.
    let (_dir, worker) = worker();
    let id = remote(&worker);
    worker
        .upsert_connector_link(id, "acct-1", Some("Checking"), Some("USD"))
        .unwrap();
    worker
        .upsert_connector_link(id, "acct-1", Some("Checking (renamed)"), None)
        .unwrap();
    worker.set_connector_credential(id, SECRET).unwrap();
    worker
        .upsert_connector_link(id, "acct-1", Some("Checking"), Some("USD"))
        .unwrap();
    assert_eq!(count(&worker, "connector_account_links"), 1);
    assert_eq!(count(&worker, "connector_connections"), 1);
    let links = worker.connector_links(id).unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(
        links[0].currency.as_deref(),
        Some("USD"),
        "a known currency is never clobbered"
    );
}

#[test]
fn deleting_a_connection_tombstones_the_identity_and_scrubs_this_devices_rows() {
    let (_dir, worker) = worker();
    let id = local(&worker);
    worker
        .upsert_connector_link(id, "acct-1", Some("Checking"), Some("USD"))
        .unwrap();
    worker
        .set_connector_attention(id, r#"{"kind":"relink_connection"}"#)
        .unwrap();
    worker
        .advance_connector_watermarks(id, "2026-10-20", &[])
        .unwrap();

    worker.delete_connector_connection(id).unwrap();

    // The identity row stays as a tombstone; everything reads as gone.
    assert_eq!(count(&worker, "connector_connections"), 1);
    assert!(worker.connector_connections().unwrap().is_empty());
    assert!(!worker.connector_connection_exists(id).unwrap());
    assert_eq!(worker.connector_credential(id).unwrap(), None);
    assert!(worker.connector_links(id).unwrap().is_empty());
    // This device's rows are gone; the mapping row stays but is ignored.
    assert_eq!(count(&worker, "connector_connection_secrets"), 0);
    assert_eq!(count(&worker, "connector_link_cursors"), 0);
    assert_eq!(count(&worker, "connector_attention"), 0);
    assert_eq!(count(&worker, "connector_account_links"), 1);
    // The id is never reusable, and nothing resurrects the connection.
    assert!(worker
        .create_connector_connection(id, "simplefin", SECRET, None)
        .is_err());
    worker
        .upsert_connector_link(id, "acct-2", None, None)
        .unwrap();
    assert_eq!(
        count(&worker, "connector_account_links"),
        1,
        "no link under a tombstoned identity"
    );
    worker
        .set_connector_attention(id, r#"{"kind":"relink_connection"}"#)
        .unwrap();
    assert_eq!(count(&worker, "connector_attention"), 0);
    // Deleting again, or an unknown id, is a no-op.
    worker.delete_connector_connection(id).unwrap();
    worker.delete_connector_connection(Uuid::now_v7()).unwrap();
}

#[test]
fn the_cursor_is_this_devices_own_and_a_remap_clears_it() {
    let (_dir, worker) = worker();
    let id = local(&worker);
    worker
        .upsert_connector_link(id, "acct-1", Some("Checking"), Some("USD"))
        .unwrap();
    worker
        .upsert_connector_link(id, "acct-2", Some("Savings"), Some("USD"))
        .unwrap();
    // Only a MAPPED link advances.
    let checking = account(&worker, "Checking");
    worker
        .set_connector_link_account(id, "acct-1", Some(checking))
        .unwrap();
    worker
        .advance_connector_watermarks(
            id,
            "2026-10-20",
            &["acct-1".to_owned(), "acct-2".to_owned()],
        )
        .unwrap();
    let cursor = |external: &str| {
        worker
            .connector_links(id)
            .unwrap()
            .into_iter()
            .find(|l| l.external_id == external)
            .unwrap()
            .last_synced_on
    };
    assert_eq!(cursor("acct-1").as_deref(), Some("2026-10-20"));
    assert_eq!(cursor("acct-2"), None, "an unmapped link does not advance");
    assert_eq!(count(&worker, "connector_link_cursors"), 1);
    // Advancing again moves the same cursor; a remap clears it.
    worker
        .advance_connector_watermarks(id, "2026-10-21", &["acct-1".to_owned()])
        .unwrap();
    assert_eq!(cursor("acct-1").as_deref(), Some("2026-10-21"));
    worker
        .set_connector_link_account(id, "acct-1", Some(account(&worker, "Savings")))
        .unwrap();
    assert_eq!(cursor("acct-1"), None);
    assert_eq!(count(&worker, "connector_link_cursors"), 0);
    // The class-1 mapping row has no watermark column to carry.
    assert!(worker
        .read_connection()
        .unwrap()
        .prepare("SELECT last_synced_on FROM connector_account_links")
        .is_err());
}

#[test]
fn a_provider_account_id_over_the_key_width_is_rejected_not_truncated() {
    let (_dir, worker) = worker();
    let id = local(&worker);
    let fits = "k".repeat(MAX_LINK_EXTERNAL_ID_BYTES);
    worker.upsert_connector_link(id, &fits, None, None).unwrap();
    let over = "k".repeat(MAX_LINK_EXTERNAL_ID_BYTES + 1);
    let err = worker
        .upsert_connector_link(id, &over, None, None)
        .unwrap_err();
    assert!(matches!(
        err,
        DbError::ReplicatedKeyTooLong {
            table: "connector_account_links"
        }
    ));
    assert!(
        !err.to_string().contains(&over),
        "the error never carries the id"
    );
    assert_eq!(count(&worker, "connector_account_links"), 1);
    // The constant is exactly the widest key the canonical encoder accepts for (uuid, text).
    use sync_core::row_digest::{primary_key, Kind, RowDigestError, Value};
    let key = |external: &str| {
        primary_key(&[
            (Kind::Uuid, Value::Blob(id.as_bytes())),
            (Kind::Text, Value::Text(external.as_bytes())),
        ])
    };
    assert!(key(&fits).is_ok());
    assert_eq!(key(&over), Err(RowDigestError::KeyTooLong));
}

#[test]
fn the_secret_and_cursor_tables_reference_the_identity_and_the_identity_references_nothing_local() {
    // Foreign keys run class 3 -> class 1 only, so rebase reference validation passes whenever the
    // identity is present (a tombstoned one counts as present), and the mapping's cursor is keyed by
    // external id as data, not as a foreign key to the links.
    let (_dir, worker) = worker();
    let conn = worker.read_connection().unwrap();
    let references = |table: &str| -> Vec<(String, String)> {
        conn.prepare(&format!(
            "SELECT \"table\", \"from\" FROM pragma_foreign_key_list('{table}') ORDER BY 1, 2"
        ))
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
    };
    let to_identity = vec![(
        "connector_connections".to_owned(),
        "connection_id".to_owned(),
    )];
    assert_eq!(references("connector_connection_secrets"), to_identity);
    assert_eq!(references("connector_link_cursors"), to_identity);
    assert_eq!(references("connector_attention"), to_identity);
    assert!(
        references("connector_connections").is_empty(),
        "the identity references no local table"
    );
}

#[test]
fn a_refresh_that_finishes_after_the_tombstone_advances_no_cursor() {
    // A MAPPED link, a delete, then the refresh that started before it finishes and advances the
    // watermark: nothing is written under the tombstoned identity.
    let (_dir, worker) = worker();
    let id = local(&worker);
    worker
        .upsert_connector_link(id, "acct-1", Some("Checking"), Some("USD"))
        .unwrap();
    worker
        .set_connector_link_account(id, "acct-1", Some(account(&worker, "Checking")))
        .unwrap();
    worker.delete_connector_connection(id).unwrap();
    worker
        .advance_connector_watermarks(id, "2026-10-27", &["acct-1".to_owned()])
        .unwrap();
    assert_eq!(count(&worker, "connector_link_cursors"), 0);
    assert_eq!(
        worker.connector_link_currency(id, "acct-1").unwrap(),
        None,
        "the lookup ignores the mapping of a deleted connection"
    );
}
