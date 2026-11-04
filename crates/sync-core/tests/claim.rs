//! The claim proof (`sync_core::claim`, protocol 4.5; personal-cfo-v98vd AC26): the body's bytes are
//! pinned with a known answer worked out independently of the code, and the check is exercised
//! against a real chain.

mod common;

use common::{chain, Dev, VAULT};
use ed25519_dalek::Signer;
use sync_core::claim::{claim_body, verify_claim, ClaimError, CLAIM_LABEL};
use sync_core::header::Fingerprint;
use sync_core::membership::{
    extend, Action, EpochBody, MembershipBody, Reason, Recipient, KEM_ID,
    SEALED_KEYS_CIPHERTEXT_LEN, SUITE_ID,
};

const ISSUED: i64 = 1_790_000_000_000_000;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn the_body_is_the_label_the_ids_the_fingerprint_and_the_time_in_that_order() {
    // Worked out by hand from protocol 4.5 and C.1.1, not from the code: a u16 length (32) and the
    // label, then 16 + 16 + 32 raw bytes, then the time as eight big-endian bytes.
    let vault = [0, 0, 0, 0, 0, 0, 0x40, 0, 0x80, 0, 0, 0, 0, 0, 0, 0xf0];
    let device = [0, 0, 0, 0, 0, 0, 0x40, 0, 0x80, 0, 0, 0, 0, 0, 0xa0, 0x11];
    let mut fp = [0_u8; 32];
    for (i, b) in fp.iter_mut().enumerate() {
        *b = i as u8;
    }
    let body = claim_body(&vault, &device, &Fingerprint::from_derived(fp), ISSUED);
    assert_eq!(body.len(), 106);
    assert_eq!(
        hex(&body),
        "0020646f68666c6f772f73796e632d63726564656e7469616c2d636c61696d2f7631\
         000000000000400080000000000000f0\
         0000000000004000800000000000a011\
         000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f\
         00065bfeda25e000"
    );
    assert_eq!(CLAIM_LABEL, "dohflow/sync-credential-claim/v1");
}

#[test]
fn each_field_changes_the_body() {
    let (v, d, f) = ([1_u8; 16], [2_u8; 16], Fingerprint::from_derived([3; 32]));
    let base = claim_body(&v, &d, &f, 5);
    assert_ne!(base, claim_body(&[9; 16], &d, &f, 5));
    assert_ne!(base, claim_body(&v, &[9; 16], &f, 5));
    assert_ne!(
        base,
        claim_body(&v, &d, &Fingerprint::from_derived([9; 32]), 5)
    );
    assert_ne!(base, claim_body(&v, &d, &f, 6));
    assert_ne!(claim_body(&v, &d, &f, -1), claim_body(&v, &d, &f, 1));
}

fn sign(dev: &Dev, vault: &[u8; 16], issued: i64) -> [u8; 64] {
    dev.signing
        .sign(&claim_body(
            vault,
            &dev.device_id,
            &dev.fingerprint(),
            issued,
        ))
        .to_bytes()
}

#[test]
fn a_current_members_own_proof_verifies() {
    let c = chain();
    assert_eq!(
        verify_claim(
            &c.state,
            &c.a.device_id,
            ISSUED,
            &sign(&c.a, &VAULT, ISSUED)
        ),
        Ok(())
    );
    assert_eq!(
        verify_claim(
            &c.state,
            &c.b.device_id,
            ISSUED,
            &sign(&c.b, &VAULT, ISSUED)
        ),
        Ok(())
    );
}

#[test]
fn another_devices_signature_another_time_another_vault_or_a_damaged_proof_does_not() {
    let c = chain();
    let wrong = Err(ClaimError::ProofInvalid);
    assert_eq!(
        verify_claim(
            &c.state,
            &c.a.device_id,
            ISSUED,
            &sign(&c.b, &VAULT, ISSUED)
        ),
        wrong
    );
    assert_eq!(
        verify_claim(
            &c.state,
            &c.a.device_id,
            ISSUED + 1,
            &sign(&c.a, &VAULT, ISSUED)
        ),
        wrong
    );
    assert_eq!(
        verify_claim(
            &c.state,
            &c.a.device_id,
            ISSUED,
            &sign(&c.a, &[9; 16], ISSUED)
        ),
        wrong
    );
    let mut damaged = sign(&c.a, &VAULT, ISSUED);
    damaged[10] ^= 1;
    assert_eq!(
        verify_claim(&c.state, &c.a.device_id, ISSUED, &damaged),
        wrong
    );
    assert_eq!(
        verify_claim(&c.state, &c.a.device_id, ISSUED, &[0; 64]),
        wrong
    );
}

#[test]
fn a_device_the_chain_never_held_is_not_found_even_with_a_valid_signature() {
    let c = chain();
    let stranger = Dev::new(9);
    assert_eq!(
        verify_claim(
            &c.state,
            &stranger.device_id,
            ISSUED,
            &sign(&stranger, &VAULT, ISSUED)
        ),
        Err(ClaimError::DeviceNotFound)
    );
}

#[test]
fn a_revoked_device_is_not_found_even_with_its_valid_proof() {
    // A revokes B in a cutover into key epoch 2. B's proof is still a valid signature, but B is no
    // longer a member, so the claim is refused as `DeviceNotFound`; A's still verifies.
    let c = chain();
    let m3_body = MembershipBody {
        suite_id: SUITE_ID,
        kem_id: KEM_ID,
        vault_id: VAULT,
        version: 3,
        prev_id: c.state.head_id(),
        action: Action::Revoke,
        signer_fingerprint: c.a.fingerprint(),
        device_id: c.b.device_id,
        ed25519_pk: c.b.ed25519_pk,
        kem_public_key: c.b.kem_pk,
        issued_at: 1_790_000_000_000_003,
    }
    .encode()
    .unwrap();
    let m3 = common::sign(&m3_body, &c.a.signing);
    let e3_body = EpochBody {
        suite_id: SUITE_ID,
        kem_id: KEM_ID,
        vault_id: VAULT,
        key_epoch: 2,
        reason: Reason::Cutover,
        membership_head: common::id_of(&m3_body),
        prev_epoch_id: Some(c.e2),
        key_commitment_ke: [3; 32],
        key_commitment_s: [2; 32],
        sealer_fingerprint: c.a.fingerprint(),
        recipients: vec![Recipient {
            device_id: c.a.device_id,
            fingerprint: c.a.fingerprint(),
            suite_id: SUITE_ID,
            kem_id: KEM_ID,
            enc: [0xee; 32],
            ciphertext: vec![0xcc; SEALED_KEYS_CIPHERTEXT_LEN],
        }],
    }
    .encode()
    .unwrap();
    let e3 = common::sign(&e3_body, &c.a.signing);
    let after = extend(&c.state, &[&m3], &[&e3]).expect("the cutover is a valid extension");

    assert_eq!(
        after
            .members()
            .iter()
            .filter(|m| m.revoked_at_version.is_some())
            .count(),
        1
    );
    assert_eq!(
        verify_claim(&after, &c.b.device_id, ISSUED, &sign(&c.b, &VAULT, ISSUED)),
        Err(ClaimError::DeviceNotFound)
    );
    assert_eq!(
        verify_claim(&after, &c.a.device_id, ISSUED, &sign(&c.a, &VAULT, ISSUED)),
        Ok(())
    );
    // Before the cutover B was a member, and the same proof verified.
    assert_eq!(
        verify_claim(
            &c.state,
            &c.b.device_id,
            ISSUED,
            &sign(&c.b, &VAULT, ISSUED)
        ),
        Ok(())
    );
}
