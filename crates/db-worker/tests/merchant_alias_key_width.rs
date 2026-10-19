//! `merchant_aliases.normalized_key` is a replicated primary key, so its width is bounded by the
//! canonical row encoding (vault-format.md C.1.5, 256 bytes; personal-cfo-t2s4b part 5b). The
//! schema bounds nothing, so the writer does: a key that would be too wide is rejected with a
//! typed error and never truncated.

mod common;

use common::worker;
use db_worker::{DbError, MAX_ALIAS_KEY_BYTES};
use sync_core::row_digest::{primary_key, Kind, RowDigestError, Value};

fn identity(worker: &db_worker::DbWorker) -> uuid::Uuid {
    worker
        .create_merchant_identity("Coffee Shop", None, false, "user")
        .unwrap()
}

#[test]
fn the_constant_is_exactly_the_widest_key_the_encoder_accepts() {
    let fits = "k".repeat(MAX_ALIAS_KEY_BYTES);
    let over = "k".repeat(MAX_ALIAS_KEY_BYTES + 1);
    assert!(primary_key(&[(Kind::Text, Value::Text(fits.as_bytes()))]).is_ok());
    assert_eq!(
        primary_key(&[(Kind::Text, Value::Text(over.as_bytes()))]),
        Err(RowDigestError::KeyTooLong)
    );
}

#[test]
fn a_key_at_the_limit_is_stored_and_one_byte_over_is_rejected_not_truncated() {
    let (_dir, worker) = worker();
    let id = identity(&worker);
    let fits = "K".repeat(MAX_ALIAS_KEY_BYTES);
    worker
        .link_merchant_alias(&fits, id, "user", 10_000)
        .unwrap();
    assert_eq!(
        worker.resolve_merchant_identity(&fits).unwrap().unwrap().id,
        id
    );

    let over = "K".repeat(MAX_ALIAS_KEY_BYTES + 1);
    let err = worker
        .link_merchant_alias(&over, id, "user", 10_000)
        .unwrap_err();
    assert!(
        matches!(
            err,
            DbError::ReplicatedKeyTooLong {
                table: "merchant_aliases"
            }
        ),
        "{err:?}"
    );
    assert!(
        !err.to_string().contains(&over),
        "the error never carries the key"
    );
    // Nothing was stored, and no truncated prefix was stored either.
    assert!(worker.resolve_merchant_identity(&over).unwrap().is_none());
    let prefix = &over[..MAX_ALIAS_KEY_BYTES];
    assert_eq!(
        worker
            .resolve_merchant_identity(prefix)
            .unwrap()
            .unwrap()
            .id,
        id,
        "the at-limit key from before"
    );
}

#[test]
fn width_is_counted_in_utf8_bytes_not_characters() {
    let (_dir, worker) = worker();
    let id = identity(&worker);
    // 126 two-byte characters is 252 bytes: over the limit though only 126 characters long.
    let wide = "\u{e9}".repeat(126);
    assert!(wide.chars().count() < MAX_ALIAS_KEY_BYTES && wide.len() > MAX_ALIAS_KEY_BYTES);
    assert!(matches!(
        worker
            .link_merchant_alias(&wide, id, "user", 1)
            .unwrap_err(),
        DbError::ReplicatedKeyTooLong { .. }
    ));
    let ok = "\u{e9}".repeat(125); // 250 bytes
    worker.link_merchant_alias(&ok, id, "user", 1).unwrap();
}
