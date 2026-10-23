//! Re-authentication notices (personal-cfo-pxi.22, migration 58): a device-local row per
//! connection that a refresh writes and the next healthy refresh clears.

mod common;

use common::*;
use uuid::Uuid;

fn connection(worker: &db_worker::DbWorker) -> Uuid {
    let id = Uuid::now_v7();
    worker
        .create_connector_connection(id, "other", "mock-access-url", None)
        .unwrap();
    id
}

fn attention_of(worker: &db_worker::DbWorker, id: Uuid) -> Option<String> {
    worker
        .connector_connections()
        .unwrap()
        .into_iter()
        .find(|row| row.id == id)
        .expect("the connection is listed")
        .attention
}

const ONE_BANK: &str = r#"{"kind":"reauth_institution","institutions":["First Bank"]}"#;

#[test]
fn a_new_connection_has_no_notice() {
    let (_dir, worker) = worker();
    let id = connection(&worker);
    assert_eq!(attention_of(&worker, id), None);
}

#[test]
fn a_notice_is_listed_replaced_and_cleared() {
    let (_dir, worker) = worker();
    let id = connection(&worker);

    worker.set_connector_attention(id, ONE_BANK).unwrap();
    assert_eq!(attention_of(&worker, id).as_deref(), Some(ONE_BANK));

    // A later refresh with a different message replaces it: one row per connection.
    let two = r#"{"kind":"reauth_institution","institutions":["First Bank","Credit Union"]}"#;
    worker.set_connector_attention(id, two).unwrap();
    assert_eq!(attention_of(&worker, id).as_deref(), Some(two));

    // The next healthy refresh clears it; clearing again is not an error.
    worker.clear_connector_attention(id).unwrap();
    assert_eq!(attention_of(&worker, id), None);
    worker.clear_connector_attention(id).unwrap();
}

#[test]
fn a_notice_belongs_to_its_own_connection_only() {
    let (_dir, worker) = worker();
    let a = connection(&worker);
    let b = connection(&worker);
    worker.set_connector_attention(a, ONE_BANK).unwrap();
    assert_eq!(attention_of(&worker, a).as_deref(), Some(ONE_BANK));
    assert_eq!(attention_of(&worker, b), None);
    worker.clear_connector_attention(b).unwrap();
    assert_eq!(
        attention_of(&worker, a).as_deref(),
        Some(ONE_BANK),
        "clearing b left a alone"
    );
}

#[test]
fn a_healthy_sync_record_does_not_clear_the_notice() {
    // `record_connector_sync(None)` clears last_error; the notice is a separate thing that only an
    // explicit clear removes, so a one-bank re-authentication survives a partly healthy refresh
    // until the desktop layer decides the refresh was fully healthy.
    let (_dir, worker) = worker();
    let id = connection(&worker);
    worker.set_connector_attention(id, ONE_BANK).unwrap();
    worker.record_connector_sync(id, None).unwrap();
    assert_eq!(attention_of(&worker, id).as_deref(), Some(ONE_BANK));
}

#[test]
fn a_notice_for_a_connection_that_is_gone_is_a_no_op_not_an_orphan() {
    let (_dir, worker) = worker();
    let id = connection(&worker);
    worker.delete_connector_connection(id).unwrap();
    // A refresh finishing after the user deleted the connection must not leave a row behind.
    worker.set_connector_attention(id, ONE_BANK).unwrap();
    assert!(worker.connector_connections().unwrap().is_empty());
}

#[test]
fn deleting_a_connection_removes_its_notice() {
    let (_dir, worker) = worker();
    let id = connection(&worker);
    let other = connection(&worker);
    worker.set_connector_attention(id, ONE_BANK).unwrap();
    worker.set_connector_attention(other, ONE_BANK).unwrap();
    worker.delete_connector_connection(id).unwrap();
    assert_eq!(attention_of(&worker, other).as_deref(), Some(ONE_BANK));
    // Re-creating a connection with the same id starts clean (no stale notice from the old one).
    worker
        .create_connector_connection(id, "other", "mock-access-url", None)
        .unwrap();
    assert_eq!(attention_of(&worker, id), None);
}

#[test]
fn only_valid_json_is_accepted() {
    let (_dir, worker) = worker();
    let id = connection(&worker);
    assert!(worker.set_connector_attention(id, "not json {").is_err());
    assert_eq!(
        attention_of(&worker, id),
        None,
        "a refused write leaves nothing behind"
    );
}
