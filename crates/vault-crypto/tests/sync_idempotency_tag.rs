//! The idempotency-key tag (vault-format.md C.3, C.5; personal-cfo-t2s4b part 6b): the C.14
//! vectors, read from the spec, and the properties the protocol relies on.

use vault_crypto::sync::{idempotency_key_tag, idempotency_tag_key};
use vault_crypto::SecretBytes;

const SPEC: &str = include_str!("../../../docs/architecture/vault-format.md");

fn vector(name: &str) -> Vec<u8> {
    let marker = format!("<!-- sync-vector: {name}=");
    let start = SPEC
        .find(&marker)
        .unwrap_or_else(|| panic!("no vector {name}"))
        + marker.len();
    let hex = SPEC[start..].split(" -->").next().unwrap();
    (0..hex.len() / 2)
        .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}

/// `S` is the 32 bytes `20 21 22 ... 3f` (C.14).
fn secret_s() -> SecretBytes<32> {
    SecretBytes::from_slice(&(0x20..0x40).collect::<Vec<u8>>()).unwrap()
}

const VAULT: [u8; 16] = [0x11; 16];
/// `4a3b2c1d-9e8f-4a7b-8c6d-5e4f3a2b1c0d` as 16 raw bytes.
const KEY: [u8; 16] = [
    0x4a, 0x3b, 0x2c, 0x1d, 0x9e, 0x8f, 0x4a, 0x7b, 0x8c, 0x6d, 0x5e, 0x4f, 0x3a, 0x2b, 0x1c, 0x0d,
];

#[test]
fn the_tag_key_and_tag_match_the_spec_vectors() {
    let tag_key = idempotency_tag_key(&secret_s(), &VAULT);
    assert_eq!(tag_key.expose().as_slice(), vector("idempotency_tag_key"));
    assert_eq!(
        idempotency_key_tag(&tag_key, &KEY).as_slice(),
        vector("idempotency_tag")
    );
}

#[test]
fn a_tag_is_stable_and_binds_the_key_the_vault_and_the_secret() {
    let tag_key = idempotency_tag_key(&secret_s(), &VAULT);
    let tag = idempotency_key_tag(&tag_key, &KEY);
    // Stable: the same inputs always give the same tag (a retry is recognized).
    assert_eq!(
        tag,
        idempotency_key_tag(&idempotency_tag_key(&secret_s(), &VAULT), &KEY)
    );
    // A different key, vault or secret gives a different tag.
    let mut other_key = KEY;
    other_key[15] ^= 1;
    assert_ne!(tag, idempotency_key_tag(&tag_key, &other_key));
    assert_ne!(
        tag,
        idempotency_key_tag(&idempotency_tag_key(&secret_s(), &[0x12; 16]), &KEY)
    );
    let other_secret = SecretBytes::<32>::from_slice(&[7_u8; 32]).unwrap();
    assert_ne!(
        tag,
        idempotency_key_tag(&idempotency_tag_key(&other_secret, &VAULT), &KEY)
    );
}

#[test]
fn the_tag_does_not_reveal_the_key() {
    let tag_key = idempotency_tag_key(&secret_s(), &VAULT);
    let tag = idempotency_key_tag(&tag_key, &KEY);
    assert!(
        !tag.windows(KEY.len()).any(|w| w == KEY),
        "the key's bytes appear in its tag"
    );
}
