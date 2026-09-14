//! Integration tests for the shared ID strategy (personal-cfo-i2t).

use std::collections::HashSet;

use core_ids::uuid_id;

uuid_id! {
    /// Identifier used only to exercise the macro in tests.
    TestId
}

#[test]
fn one_million_ids_have_no_collisions_and_sort_in_creation_order() {
    const N: usize = 1_000_000;
    let ids: Vec<TestId> = (0..N).map(|_| TestId::new()).collect();

    // No collisions across a million ids.
    let unique: HashSet<[u8; 16]> = ids.iter().map(|id| id.as_bytes()).collect();
    assert_eq!(unique.len(), N, "expected {N} unique ids");

    // Non-decreasing in creation order (monotonic generator). Combined with the
    // uniqueness check above, this proves the sequence is strictly increasing —
    // i.e. ORDER BY id reproduces creation order.
    assert!(
        ids.windows(2).all(|w| w[0] <= w[1]),
        "ids must be monotonic by creation order"
    );
}

#[test]
fn display_id_round_trips() {
    let id = TestId::new();
    let code = id.display_id();
    assert_eq!(code.len(), 26);
    assert_eq!(TestId::from_display_id(&code).unwrap(), id);
}

#[test]
fn serde_stays_a_transparent_uuid_string() {
    let id = TestId::new();
    let json = serde_json::to_string(&id).unwrap();
    // #[serde(transparent)] → the hyphenated UUID string (quoted), so the IPC
    // wire format is unchanged by the BLOB storage migration.
    assert_eq!(json, format!("\"{id}\""));
    assert_eq!(serde_json::from_str::<TestId>(&json).unwrap(), id);
}

#[test]
fn bytes_round_trip_and_reject_bad_length() {
    let id = TestId::new();
    assert_eq!(TestId::from_bytes(&id.as_bytes()).unwrap(), id);
    assert_eq!(
        TestId::from_bytes(&[0u8; 4]).unwrap_err(),
        core_ids::IdError::BadLength
    );
}
