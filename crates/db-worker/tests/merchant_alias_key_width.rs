//! `merchant_aliases.normalized_key` is a replicated primary key, so its width is bounded by the
//! canonical row encoding (vault-format.md C.1.5, 256 bytes; personal-cfo-t2s4b part 5b). The
//! schema bounds nothing, so the writer does: `MintMerchantAnchors` is the one command that writes
//! an alias (personal-cfo-5ymg8.5), and it refuses a key that would be too wide with a typed
//! rejection and never truncates it.

mod common;

use common::{meta, worker};
use db_worker::{DbError, MerchantAnchor, RejectCode, WriteCommand, MAX_ALIAS_KEY_BYTES};
use sync_core::row_digest::{primary_key, Kind, RowDigestError, Value};

fn mint(worker: &db_worker::DbWorker, key: &str) -> Result<(), DbError> {
    worker
        .dispatch(
            meta(),
            WriteCommand::MintMerchantAnchors {
                anchors: vec![MerchantAnchor {
                    key: key.to_owned(),
                    display_name: "Coffee Shop".to_owned(),
                }],
            },
        )
        .map(|_| ())
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
    let fits = "K".repeat(MAX_ALIAS_KEY_BYTES);
    mint(&worker, &fits).unwrap();
    let id = worker
        .resolve_merchant_identity(&fits)
        .unwrap()
        .expect("the at-limit key is stored")
        .id;

    let over = "K".repeat(MAX_ALIAS_KEY_BYTES + 1);
    let identities = worker.merchant_identity_count().unwrap();
    let err = mint(&worker, &over).unwrap_err();
    assert!(
        matches!(
            err,
            DbError::Rejected {
                code: RejectCode::PayloadInvalid,
                ..
            }
        ),
        "{err:?}"
    );
    assert!(
        !err.to_string().contains(&over),
        "the error never carries the key"
    );
    // Nothing was stored, and no truncated prefix was stored either.
    assert_eq!(worker.merchant_identity_count().unwrap(), identities);
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
    // 126 two-byte characters is 252 bytes: over the limit though only 126 characters long.
    let wide = "\u{e9}".repeat(126);
    assert!(wide.chars().count() < MAX_ALIAS_KEY_BYTES && wide.len() > MAX_ALIAS_KEY_BYTES);
    assert!(matches!(
        mint(&worker, &wide).unwrap_err(),
        DbError::Rejected {
            code: RejectCode::PayloadInvalid,
            ..
        }
    ));
    let ok = "\u{e9}".repeat(125); // 250 bytes
    mint(&worker, &ok).unwrap();
}
