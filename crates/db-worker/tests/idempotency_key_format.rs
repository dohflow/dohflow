//! Every idempotency key is a canonical hyphenated UUID version 4 from the OS
//! CSPRNG, enforced at the command boundary (ADR 0077 C.5, personal-cfo-g3m.6).
//!
//! The rule applies to NEW commands. A key stored before the rule keeps deduping
//! until its 30-day expiry, so the replay lookup runs before the format check.

mod common;

use common::*;
use db_worker::{new_idempotency_key, validate_idempotency_key, DbError, Outcome};
use rusqlite::params;
use uuid::Uuid;

fn is_rejected(key: &str) -> bool {
    matches!(
        validate_idempotency_key(key),
        Err(DbError::InvalidIdempotencyKey)
    )
}

#[test]
fn minted_keys_are_canonical_lowercase_hyphenated_uuid_v4() {
    let mut seen = std::collections::HashSet::new();
    for _ in 0..200 {
        let key = new_idempotency_key();
        assert!(validate_idempotency_key(&key).is_ok(), "{key}");
        assert_eq!(key, key.to_lowercase());
        assert_eq!(key.len(), 36);
        assert_eq!(key.as_bytes()[14], b'4', "version nibble");
        assert!(matches!(key.as_bytes()[19], b'8' | b'9' | b'a' | b'b'));
        // No prefix, and parseable as a version-4 UUID.
        let parsed = Uuid::parse_str(&key).unwrap();
        assert_eq!(parsed.get_version_num(), 4);
        assert!(seen.insert(key), "a CSPRNG key must not repeat");
    }
}

#[test]
fn a_canonical_uuid_v4_is_accepted() {
    for key in [
        "4a3b2c1d-9e8f-4a7b-8c6d-5e4f3a2b1c0d",
        "00000000-0000-4000-8000-000000000000",
        "ffffffff-ffff-4fff-bfff-ffffffffffff",
        "123e4567-e89b-42d3-a456-426614174000",
    ] {
        assert!(validate_idempotency_key(key).is_ok(), "{key}");
    }
}

#[test]
fn a_uuid_v7_is_rejected() {
    // The backend's old minting scheme.
    assert!(is_rejected(&Uuid::now_v7().to_string()));
}

#[test]
fn a_prefixed_string_is_rejected() {
    let uuid = Uuid::new_v4();
    for key in [
        format!("connector-refresh-{uuid}"),
        format!("connector-auto-{uuid}"),
        format!("key-{uuid}"),
        format!("{uuid}-suffix"),
    ] {
        assert!(is_rejected(&key), "{key}");
    }
}

#[test]
fn a_non_uuid_is_rejected() {
    for key in [
        "not-a-uuid",
        "one-user-action",
        "k1",
        "bulk-key-1",
        "12345678",
        "zzzzzzzz-zzzz-4zzz-8zzz-zzzzzzzzzzzz",
    ] {
        assert!(is_rejected(key), "{key}");
    }
}

#[test]
fn a_blank_key_is_rejected() {
    assert!(is_rejected(""));
    assert!(is_rejected("   "));
    assert!(is_rejected("\n"));
}

#[test]
fn only_the_canonical_form_is_accepted() {
    let v4 = Uuid::new_v4();
    for key in [
        v4.to_string().to_uppercase(), // uppercase
        format!("{{{v4}}}"),           // braced
        format!("urn:uuid:{v4}"),      // URN
        v4.simple().to_string(),       // no hyphens
        format!(" {v4}"),              // leading space
        format!("{v4} "),              // trailing space
        format!("{v4}\n"),             // trailing newline
        // Wrong variant nibble.
        "4a3b2c1d-9e8f-4a7b-0c6d-5e4f3a2b1c0d".to_owned(),
        "4a3b2c1d-9e8f-4a7b-cc6d-5e4f3a2b1c0d".to_owned(),
        // Other versions.
        "4a3b2c1d-9e8f-1a7b-8c6d-5e4f3a2b1c0d".to_owned(),
        "4a3b2c1d-9e8f-3a7b-8c6d-5e4f3a2b1c0d".to_owned(),
        "4a3b2c1d-9e8f-5a7b-8c6d-5e4f3a2b1c0d".to_owned(),
        "4a3b2c1d-9e8f-6a7b-8c6d-5e4f3a2b1c0d".to_owned(),
        "4a3b2c1d-9e8f-7a7b-8c6d-5e4f3a2b1c0d".to_owned(),
        // Misplaced hyphens and the wrong length.
        "4a3b2c1d9-e8f-4a7b-8c6d-5e4f3a2b1c0d".to_owned(),
        "4a3b2c1d-9e8f-4a7b-8c6d-5e4f3a2b1c0".to_owned(),
        "4a3b2c1d-9e8f-4a7b-8c6d-5e4f3a2b1c0d0".to_owned(),
    ] {
        assert!(is_rejected(&key), "{key:?}");
    }
    // A UUIDv5 and the nil UUID are not v4 either.
    assert!(is_rejected(
        &Uuid::new_v5(&Uuid::NAMESPACE_DNS, b"dohflow").to_string()
    ));
    assert!(is_rejected(&Uuid::nil().to_string()));
}

#[test]
fn dispatch_rejects_an_invalid_key_with_a_typed_error_and_writes_nothing() {
    let (_dir, worker) = worker();
    let before = worker.operation_count().unwrap();

    for key in [
        Uuid::now_v7().to_string(),
        format!("connector-refresh-{}", Uuid::new_v4()),
        "not-a-uuid".to_owned(),
    ] {
        let result = worker.dispatch(meta_with_key(&key), create_account_cmd());
        assert!(
            matches!(result, Err(DbError::InvalidIdempotencyKey)),
            "{key}: {result:?}"
        );
    }
    assert_eq!(worker.operation_count().unwrap(), before, "nothing written");
    assert_eq!(worker.idempotency_key_count().unwrap(), 0, "no key stored");
    assert_eq!(worker.account_count().unwrap(), 0);
}

#[test]
fn dispatch_still_reports_a_blank_key_as_missing_metadata() {
    let (_dir, worker) = worker();
    let result = worker.dispatch(meta_with_key(""), create_account_cmd());
    assert!(
        matches!(result, Err(DbError::MissingMetadata("idempotency_key"))),
        "{result:?}"
    );
    assert_eq!(worker.operation_count().unwrap(), 0);
}

#[test]
fn a_valid_key_applies_and_replays_as_before() {
    let (_dir, worker) = worker();
    let key = new_idempotency_key();
    let first = worker
        .dispatch(meta_with_key(&key), create_account_cmd())
        .unwrap();
    let second = worker
        .dispatch(meta_with_key(&key), create_account_cmd())
        .unwrap();
    assert!(matches!(first, Outcome::Applied { .. }));
    assert!(matches!(second, Outcome::Replayed { .. }));
    assert_eq!(worker.operation_count().unwrap(), 1);
}

/// AC4: a key stored before this rule (a UUIDv7, a prefixed string) is untouched
/// and still dedupes until its expiry. Only NEW commands are validated.
#[test]
fn a_key_stored_before_the_rule_still_dedupes_until_expiry() {
    let (_dir, worker) = worker();
    let key = new_idempotency_key();
    let applied = worker
        .dispatch(meta_with_key(&key), create_account_cmd())
        .unwrap();
    let Outcome::Applied { op_seq, .. } = applied else {
        panic!("the first dispatch applies: {applied:?}");
    };

    // Rewrite the stored key to the shapes the backend minted before the rule.
    let conn = worker.read_connection().unwrap();
    let legacy_v7 = Uuid::now_v7().to_string();
    let legacy_prefixed = format!("connector-refresh-{}", Uuid::now_v7());
    let changed = conn
        .execute(
            "UPDATE command_idempotency_keys SET idempotency_key = ?1 \
             WHERE idempotency_key = ?2",
            params![legacy_v7, key],
        )
        .unwrap();
    assert_eq!(changed, 1);
    // A second stored row with a prefixed legacy key, pointing at the same op.
    conn.execute(
        "INSERT INTO command_idempotency_keys \
         (idempotency_key, command_id, command_type, result_ref_json, op_seq, expires_at) \
         SELECT ?1, command_id, command_type, result_ref_json, op_seq, expires_at \
         FROM command_idempotency_keys WHERE idempotency_key = ?2",
        params![legacy_prefixed, legacy_v7],
    )
    .unwrap();

    let ops_before = worker.operation_count().unwrap();
    for legacy in [&legacy_v7, &legacy_prefixed] {
        let replay = worker
            .dispatch(meta_with_key(legacy), create_account_cmd())
            .unwrap();
        match replay {
            Outcome::Replayed { op_seq: replayed } => assert_eq!(replayed, op_seq),
            other => panic!("a stored legacy key must replay, got {other:?}"),
        }
    }
    assert_eq!(worker.operation_count().unwrap(), ops_before, "nothing new");

    // A legacy-shaped key that was never stored is a NEW command: rejected.
    let unknown = format!("connector-auto-{}", Uuid::now_v7());
    let result = worker.dispatch(meta_with_key(&unknown), create_account_cmd());
    assert!(matches!(result, Err(DbError::InvalidIdempotencyKey)));
}
