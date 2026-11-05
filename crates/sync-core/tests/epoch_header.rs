//! The epoch header, its hash and the HPKE `info` and `aad` they feed (vault-format.md C.6, "Sealing
//! the key set (F1)"; personal-cfo-1cltt.2). The byte-level vectors are pinned in C.14 and checked by
//! `crates/vault-crypto/tests/sync_keyset.rs`; this file pins the structure.

use sha2::{Digest, Sha256};
use sync_core::header::Fingerprint;
use sync_core::membership::{EpochBody, Reason, Recipient, KEM_ID, SUITE_ID};

fn body(recipients: Vec<Recipient>) -> EpochBody {
    EpochBody {
        suite_id: SUITE_ID,
        kem_id: KEM_ID,
        vault_id: [0x11; 16],
        key_epoch: 2,
        reason: Reason::Cutover,
        membership_head: [0x22; 32],
        prev_epoch_id: Some([0x33; 32]),
        key_commitment_ke: [0x44; 32],
        key_commitment_s: [0x55; 32],
        sealer_fingerprint: Fingerprint::from_derived([0x66; 32]),
        recipients,
    }
}

fn recipient(byte: u8) -> Recipient {
    Recipient {
        device_id: [byte; 16],
        fingerprint: Fingerprint::from_derived([byte; 32]),
        suite_id: SUITE_ID,
        kem_id: KEM_ID,
        enc: [byte; 32],
        ciphertext: vec![byte; 109],
    }
}

#[test]
fn the_header_is_the_prefix_of_the_encoded_body() {
    let body = body(vec![recipient(1), recipient(2)]);
    let encoded = body.encode().unwrap();
    assert!(encoded.starts_with(&body.header_bytes()));
    assert_eq!(
        body.header_hash(),
        <[u8; 32]>::from(Sha256::digest(body.header_bytes()))
    );
}

#[test]
fn the_header_hash_does_not_depend_on_the_recipient_list() {
    let none = body(Vec::new());
    let some = body(vec![recipient(1), recipient(2), recipient(3)]);
    assert_eq!(none.header_hash(), some.header_hash());
    assert_eq!(none.header_bytes(), some.header_bytes());
}

#[test]
fn every_header_field_changes_the_hash() {
    let base = body(Vec::new());
    let mut variants = Vec::new();
    let mut v = base.clone();
    v.key_epoch = 3;
    variants.push(v);
    let mut v = base.clone();
    v.reason = Reason::Rotation;
    variants.push(v);
    let mut v = base.clone();
    v.membership_head = [0x23; 32];
    variants.push(v);
    let mut v = base.clone();
    v.prev_epoch_id = None;
    variants.push(v);
    let mut v = base.clone();
    v.key_commitment_ke = [0x45; 32];
    variants.push(v);
    let mut v = base.clone();
    v.key_commitment_s = [0x56; 32];
    variants.push(v);
    let mut v = base.clone();
    v.sealer_fingerprint = Fingerprint::from_derived([0x67; 32]);
    variants.push(v);
    let mut v = base.clone();
    v.vault_id = [0x12; 16];
    variants.push(v);
    for variant in variants {
        assert_ne!(variant.header_hash(), base.header_hash());
    }
}

#[test]
fn the_hpke_info_and_aad_bind_the_header_and_the_recipient() {
    let base = body(Vec::new());
    let device = [0xb1; 16];
    let fingerprint = Fingerprint::from_derived([0xb2; 32]);
    let info = base.hpke_info(&device, &fingerprint);
    let aad = base.hpke_aad(&fingerprint);
    // A different recipient device or fingerprint changes both.
    assert_ne!(info, base.hpke_info(&[0xb3; 16], &fingerprint));
    assert_ne!(
        info,
        base.hpke_info(&device, &Fingerprint::from_derived([0xb4; 32]))
    );
    assert_ne!(aad, base.hpke_aad(&Fingerprint::from_derived([0xb4; 32])));
    // A different header changes both (the epoch, the head, the sealer, a commitment).
    let mut other = base.clone();
    other.key_epoch = 3;
    assert_ne!(info, other.hpke_info(&device, &fingerprint));
    assert_ne!(aad, other.hpke_aad(&fingerprint));
    // The recipient list is not an input: the same bytes whoever builds or checks the record.
    let with_list = body(vec![recipient(1)]);
    assert_eq!(info, with_list.hpke_info(&device, &fingerprint));
    assert_eq!(aad, with_list.hpke_aad(&fingerprint));
    // And they start with their labels.
    assert!(info.starts_with(&[0x00, 0x19]));
    assert!(aad.starts_with(&[0x00, 0x18]));
}
