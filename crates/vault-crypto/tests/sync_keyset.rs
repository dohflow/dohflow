//! Sealing a key set to a device (vault-format.md C.6, "Sealing the key set (F1)";
//! personal-cfo-1cltt.2): the RFC 9180 known-answer vectors for suite 1, both directions, and the
//! properties the sealing relies on.

use serde_json::Value;
use sync_core::header::Fingerprint;
use sync_core::membership::{
    device_fingerprint, EpochBody, Reason, KEM_ID, SEALED_KEYS_CIPHERTEXT_LEN, SUITE_ID,
};
use vault_crypto::sync_keys::{
    epoch_key_commitment, generate_secret, tag_key_commitment, DeviceKeys,
};
use vault_crypto::sync_keyset::{
    open_with_key, seal_key_set, seal_with_seed, unseal_key_set, KeySetError, SealedKeySet,
    SEALED_KEYS_PLAINTEXT_LEN,
};
use vault_crypto::SecretBytes;

const VECTORS: &str =
    include_str!("../../../docs/architecture/vectors/hpke-rfc9180-x25519-v1.json");
const SPEC: &str = include_str!("../../../docs/architecture/vault-format.md");

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len() / 2)
        .map(|i| u8::from_str_radix(&text[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}

fn field(v: &Value, name: &str) -> Vec<u8> {
    unhex(v[name].as_str().unwrap_or_else(|| panic!("no {name}")))
}

fn secret32(bytes: &[u8]) -> SecretBytes<32> {
    SecretBytes::from_slice(bytes).expect("32 bytes")
}

fn spec_vector(name: &str) -> Vec<u8> {
    let marker = format!("<!-- sync-vector: {name}=");
    let start = SPEC
        .find(&marker)
        .unwrap_or_else(|| panic!("no vector {name}"))
        + marker.len();
    unhex(SPEC[start..].split(" -->").next().unwrap())
}

// ---- RFC 9180 known answers ---------------------------------------------------------------

#[test]
fn the_vector_file_pins_both_aead_ids_of_the_x25519_suite() {
    let file: Value = serde_json::from_str(VECTORS).unwrap();
    let ids: Vec<u64> = file["vectors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["aead_id"].as_u64().unwrap())
        .collect();
    assert_eq!(ids, [1, 2], "A.1 (AES-128-GCM) and suite 1 (AES-256-GCM)");
    for v in file["vectors"].as_array().unwrap() {
        assert_eq!(
            (
                v["mode"].as_u64(),
                v["kem_id"].as_u64(),
                v["kdf_id"].as_u64()
            ),
            (Some(0), Some(32), Some(1))
        );
        assert_eq!(
            v["encryptions"].as_array().unwrap().len(),
            1,
            "sequence 0 only"
        );
    }
}

#[test]
fn the_receiver_opens_the_published_ciphertext() {
    let file: Value = serde_json::from_str(VECTORS).unwrap();
    for v in file["vectors"].as_array().unwrap() {
        // Single-shot Open is defined for the AEAD sync uses; the A.1 entry shares the KEM and the
        // key schedule but not the AEAD, so only the suite-1 entry can be opened here.
        if v["aead_id"].as_u64() != Some(2) {
            continue;
        }
        let e = &v["encryptions"][0];
        let plaintext = open_with_key(
            &secret32(&field(v, "skRm")),
            &field(v, "enc").try_into().unwrap(),
            &field(v, "info"),
            &field(e, "aad"),
            &field(e, "ct"),
        )
        .expect("the published ciphertext opens");
        assert_eq!(plaintext.as_slice(), field(e, "pt").as_slice());
    }
}

#[test]
fn the_sender_reproduces_enc_and_the_ciphertext_from_the_published_seed() {
    let file: Value = serde_json::from_str(VECTORS).unwrap();
    for v in file["vectors"].as_array().unwrap() {
        if v["aead_id"].as_u64() != Some(2) {
            continue;
        }
        let e = &v["encryptions"][0];
        let (enc, ct) = seal_with_seed(
            &field(v, "pkRm").try_into().unwrap(),
            &field(v, "info"),
            &field(e, "aad"),
            &field(e, "pt"),
            &secret32(&field(v, "ikmE")),
        )
        .expect("seals");
        assert_eq!(enc.as_slice(), field(v, "enc").as_slice(), "enc");
        assert_eq!(ct, field(e, "ct"), "ciphertext");
    }
}

// ---- the spec's own HPKE info and aad bytes -----------------------------------------------------

/// The C.14 grant sample: device A seals to device B in epoch 1.
fn grant_sample_header() -> (EpochBody, [u8; 16], Fingerprint, [u8; 32]) {
    let header = spec_vector("epoch_header");
    // label (2 + 28), u16 protocol, u16 suite, u16 kem, 16 vault, u64 epoch, u8 reason, 32 head,
    // presence byte + 32 prev, 32 + 32 commitments, 32 sealer.
    let mut at = 2 + 28 + 2 + 2 + 2;
    let vault: [u8; 16] = header[at..at + 16].try_into().unwrap();
    at += 16;
    let key_epoch = u64::from_be_bytes(header[at..at + 8].try_into().unwrap());
    at += 8;
    assert_eq!(header[at], 3, "reason 3, a grant");
    at += 1;
    let head: [u8; 32] = header[at..at + 32].try_into().unwrap();
    at += 32;
    assert_eq!(header[at], 1);
    at += 1;
    let prev: [u8; 32] = header[at..at + 32].try_into().unwrap();
    at += 32;
    let ke: [u8; 32] = header[at..at + 32].try_into().unwrap();
    at += 32;
    let s: [u8; 32] = header[at..at + 32].try_into().unwrap();
    at += 32;
    let sealer: [u8; 32] = header[at..at + 32].try_into().unwrap();
    at += 32;
    assert_eq!(at, header.len());
    let body = EpochBody {
        suite_id: SUITE_ID,
        kem_id: KEM_ID,
        vault_id: vault,
        key_epoch,
        reason: Reason::Grant,
        membership_head: head,
        prev_epoch_id: Some(prev),
        key_commitment_ke: ke,
        key_commitment_s: s,
        sealer_fingerprint: Fingerprint::from_derived(sealer),
        recipients: Vec::new(),
    };
    let device_b = [0xb1_u8; 16];
    let fingerprint_b = device_fingerprint(SUITE_ID, KEM_ID, &[0xe2; 32], &[0xc2; 32]);
    (body, device_b, fingerprint_b, ke)
}

#[test]
fn the_sample_epoch_header_and_hpke_bytes_are_the_specs() {
    let (body, device_b, fingerprint_b, _) = grant_sample_header();
    assert_eq!(body.header_bytes(), spec_vector("epoch_header"));
    assert_eq!(
        body.header_hash().to_vec(),
        spec_vector("epoch_header_hash")
    );
    let info = body.hpke_info(&device_b, &fingerprint_b);
    assert_eq!(info.len(), 199, "C.14 states 199 bytes");
    assert_eq!(info, spec_vector("hpke_info_recipient_b"));
    assert_eq!(
        body.hpke_aad(&fingerprint_b),
        spec_vector("hpke_aad_recipient_b")
    );
}

#[test]
fn the_sample_commitments_are_those_of_the_sample_keys() {
    let (body, _, _, ke) = grant_sample_header();
    let k_e = secret32(&(0x00..0x20).collect::<Vec<u8>>());
    let s = secret32(&(0x20..0x40).collect::<Vec<u8>>());
    assert_eq!(epoch_key_commitment(&k_e), ke);
    assert_eq!(tag_key_commitment(&s), body.key_commitment_s);
    assert_ne!(
        epoch_key_commitment(&k_e),
        tag_key_commitment(&k_e),
        "separate labels"
    );
}

#[test]
fn the_199_byte_info_of_c14_seals_and_opens() {
    // C.14: "the 199-byte HPKE info should be confirmed against the HPKE crate". hpke 0.13 hashes
    // the info into the key schedule and imposes no length cap; this proves it for the real bytes.
    let (body, device_b, fingerprint_b, _) = grant_sample_header();
    let info = body.hpke_info(&device_b, &fingerprint_b);
    let aad = body.hpke_aad(&fingerprint_b);
    let device = DeviceKeys::generate().unwrap();
    let k_e = secret32(&(0x00..0x20).collect::<Vec<u8>>());
    let s = secret32(&(0x20..0x40).collect::<Vec<u8>>());
    let sealed = seal_key_set(&device.public().kem_public_key, &info, &aad, &k_e, &s).unwrap();
    let opened = unseal_key_set(&device, &sealed, &info, &aad).unwrap();
    assert_eq!(opened.epoch_root.expose(), k_e.expose());
    assert_eq!(opened.tag_secret.expose(), s.expose());
}

// ---- sealing properties -----------------------------------------------------------------------

fn sealed_for(
    device: &DeviceKeys,
    info: &[u8],
    aad: &[u8],
) -> (SealedKeySet, SecretBytes<32>, SecretBytes<32>) {
    let k_e = generate_secret().unwrap();
    let s = generate_secret().unwrap();
    let sealed = seal_key_set(&device.public().kem_public_key, info, aad, &k_e, &s).unwrap();
    (sealed, k_e, s)
}

#[test]
fn a_sealed_key_set_has_the_lengths_of_the_spec() {
    let device = DeviceKeys::generate().unwrap();
    let (sealed, ..) = sealed_for(&device, b"info", b"aad");
    assert_eq!(sealed.enc.len(), 32);
    assert_eq!(sealed.ciphertext.len(), SEALED_KEYS_CIPHERTEXT_LEN);
    assert_eq!(SEALED_KEYS_CIPHERTEXT_LEN, SEALED_KEYS_PLAINTEXT_LEN + 16);
}

#[test]
fn the_recipient_recovers_exactly_what_was_sealed() {
    let device = DeviceKeys::generate().unwrap();
    let (sealed, k_e, s) = sealed_for(&device, b"info", b"aad");
    let opened = unseal_key_set(&device, &sealed, b"info", b"aad").unwrap();
    assert_eq!(opened.epoch_root.expose(), k_e.expose());
    assert_eq!(opened.tag_secret.expose(), s.expose());
}

#[test]
fn two_seals_of_the_same_keys_differ() {
    let device = DeviceKeys::generate().unwrap();
    let k_e = generate_secret().unwrap();
    let s = generate_secret().unwrap();
    let pk = device.public().kem_public_key;
    let a = seal_key_set(&pk, b"i", b"a", &k_e, &s).unwrap();
    let b = seal_key_set(&pk, b"i", b"a", &k_e, &s).unwrap();
    assert_ne!(a.enc, b.enc, "a fresh ephemeral key per seal");
    assert_ne!(a.ciphertext, b.ciphertext);
}

#[test]
fn another_device_cannot_open_it() {
    let device = DeviceKeys::generate().unwrap();
    let other = DeviceKeys::generate().unwrap();
    let (sealed, ..) = sealed_for(&device, b"info", b"aad");
    assert_eq!(
        unseal_key_set(&other, &sealed, b"info", b"aad").err(),
        Some(KeySetError::Open)
    );
}

#[test]
fn a_changed_info_or_aad_or_enc_or_ciphertext_fails_closed() {
    let device = DeviceKeys::generate().unwrap();
    let (sealed, ..) = sealed_for(&device, b"info", b"aad");
    assert_eq!(
        unseal_key_set(&device, &sealed, b"INFO", b"aad").err(),
        Some(KeySetError::Open)
    );
    assert_eq!(
        unseal_key_set(&device, &sealed, b"info", b"AAD").err(),
        Some(KeySetError::Open)
    );
    let mut bad = sealed.clone();
    bad.enc[0] ^= 1;
    assert_eq!(
        unseal_key_set(&device, &bad, b"info", b"aad").err(),
        Some(KeySetError::Open)
    );
    for i in [0, 40, SEALED_KEYS_CIPHERTEXT_LEN - 1] {
        let mut bad = sealed.clone();
        bad.ciphertext[i] ^= 1;
        assert_eq!(
            unseal_key_set(&device, &bad, b"info", b"aad").err(),
            Some(KeySetError::Open),
            "byte {i}"
        );
    }
}

#[test]
fn a_wrong_length_is_not_a_key_set() {
    let device = DeviceKeys::generate().unwrap();
    let (mut sealed, ..) = sealed_for(&device, b"info", b"aad");
    sealed.ciphertext.pop();
    assert_eq!(
        unseal_key_set(&device, &sealed, b"info", b"aad").err(),
        Some(KeySetError::NotAKeySet)
    );
}

#[test]
fn an_opened_blob_with_the_wrong_label_is_not_a_key_set() {
    // A well-formed HPKE blob whose 93 bytes do not start with the sealed-keys label.
    let device = DeviceKeys::generate().unwrap();
    let seed = generate_secret().unwrap();
    let (enc, ciphertext) = seal_with_seed(
        &device.public().kem_public_key,
        b"info",
        b"aad",
        &[0x5a; SEALED_KEYS_PLAINTEXT_LEN],
        &seed,
    )
    .unwrap();
    assert_eq!(ciphertext.len(), SEALED_KEYS_CIPHERTEXT_LEN);
    assert_eq!(
        unseal_key_set(&device, &SealedKeySet { enc, ciphertext }, b"info", b"aad").err(),
        Some(KeySetError::NotAKeySet)
    );
}

#[test]
fn a_low_order_recipient_key_is_refused_before_sealing() {
    let k_e = generate_secret().unwrap();
    let s = generate_secret().unwrap();
    for low in [[0_u8; 32], {
        let mut one = [0_u8; 32];
        one[0] = 1;
        one
    }] {
        assert_eq!(
            seal_key_set(&low, b"i", b"a", &k_e, &s).err(),
            Some(KeySetError::UnacceptableRecipientKey)
        );
    }
}

#[test]
fn errors_and_debug_output_carry_no_key_bytes() {
    let device = DeviceKeys::generate().unwrap();
    assert_eq!(format!("{device:?}"), "DeviceKeys([REDACTED])");
    let (sealed, ..) = sealed_for(&device, b"info", b"aad");
    let opened = unseal_key_set(&device, &sealed, b"info", b"aad").unwrap();
    assert_eq!(format!("{opened:?}"), "UnsealedKeySet([REDACTED])");
    let error = format!("{:?}", KeySetError::Open);
    assert_eq!(error, "Open");
}
