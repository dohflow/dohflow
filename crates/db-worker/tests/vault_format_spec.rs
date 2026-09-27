//! The public format specification must track the bytes and schema we ship.

mod common;

use common::worker;
use db_worker::CURRENT_SCHEMA_VERSION;
use vault_crypto::envelope::ENVELOPE_VERSION;
use vault_crypto::{Profile, Salt, VaultEnvelope, WrappedDek};

const SPEC: &str = include_str!("../../../docs/architecture/vault-format.md");

fn marker(key: &str) -> &str {
    let prefix = format!("<!-- vault-format: {key}=");
    let mut values = SPEC
        .lines()
        .filter_map(|line| line.strip_prefix(&prefix)?.strip_suffix(" -->"));
    let value = values
        .next()
        .unwrap_or_else(|| panic!("missing {key} marker"));
    assert!(values.next().is_none(), "duplicate {key} marker");
    value
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn migrated_schema_version_matches_public_spec() {
    assert_eq!(marker("spec-version"), "1");
    let documented: i64 = marker("schema-version").parse().unwrap();
    let (_dir, worker) = worker();
    let actual: i64 = worker
        .read_connection()
        .unwrap()
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(actual, CURRENT_SCHEMA_VERSION);
    assert_eq!(
        documented, actual,
        "update vault-format.md for the new schema"
    );
}

#[test]
fn envelope_layout_and_version_match_public_spec() {
    let documented: u16 = marker("envelope-version").parse().unwrap();
    assert_eq!(documented, ENVELOPE_VERSION);

    // Synthetic serialization fixture: not a valid wrapped DEK or a secret.
    let envelope = VaultEnvelope::new(
        Profile::InteractiveDefault.params(),
        Salt::from_bytes([0; 16]),
        WrappedDek {
            nonce: [0x11; 12],
            ciphertext: vec![0xaa, 0xbb, 0xcc],
        },
    );
    let encoded = envelope.to_bytes();
    assert_eq!(
        hex(&encoded),
        marker("envelope-v1-vector"),
        "update the documented envelope layout and version for any byte change"
    );
    assert_eq!(VaultEnvelope::from_bytes(&encoded).unwrap(), envelope);
}
