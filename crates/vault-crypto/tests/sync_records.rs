//! Genesis and the recipient's acceptance of epoch records (vault-format.md C.6;
//! personal-cfo-1cltt.2), with real Ed25519 keys and real HPKE.
//!
//! The chain-shape rejections (a gap, a fork, a reused fingerprint, a revoked signer, and the small-
//! order and non-canonical Ed25519 cases) are already pinned, with public vectors, by
//! `crates/sync-core/tests/membership_vectors.rs`; this file does not repeat them. It covers what
//! needs keys: sealing, unsealing, the commitments, and the order of the checks.

mod common;

use common::{chain, dev, epoch_key, epoch_record, id_of, refs, resign, tag_secret, Chain, VAULT};
use sync_core::membership::{
    verify_chain, ChainError, EpochBody, Reason, SignedRecord, VerifiedState,
};
use vault_crypto::sync_envelope::{aead_key, ObjectType};
use vault_crypto::sync_keys::{epoch_key_commitment, tag_key_commitment, DeviceKeys};
use vault_crypto::sync_records::{accept_records, create_genesis, AcceptError};
use vault_crypto::SecretBytes;

fn decode(record: &[u8]) -> EpochBody {
    EpochBody::decode(&SignedRecord::parse(record, 1 << 22).unwrap().body).unwrap()
}

fn accept_all(
    c: &Chain,
    device: &DeviceKeys,
) -> Result<vault_crypto::sync_records::Accepted, AcceptError> {
    accept_records(
        &VerifiedState::empty(VAULT),
        &refs(&c.memberships),
        &refs(&c.epochs),
        device,
    )
}

// ---- RFC 8032 Ed25519, section 7.1 --------------------------------------------------------------

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len() / 2)
        .map(|i| u8::from_str_radix(&text[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}

#[test]
fn ed25519_signing_matches_the_rfc_8032_test_vectors() {
    // (secret key, public key, message, signature): TEST 1, 2 and 3 of RFC 8032 section 7.1.
    let cases = [
        (
            "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60",
            "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a",
            "",
            "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b",
        ),
        (
            "4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb",
            "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c",
            "72",
            "92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00",
        ),
        (
            "c5aa8df43f9f837bedb7442f31dcb7b166d38535076f094b85ce3a2e0b4458f7",
            "fc51cd8e6218a1a38da47ed00230f0580816ed13ba3303ac5deb911548908025",
            "af82",
            "6291d657deec24024827e69c3abe01a30ce548a284743a445e3680d7db5ac3ac18ff9b538d16f290ae67f760984dc6594a7c15e9716ed28dc027beceea1ec40a",
        ),
    ];
    for (secret, public, message, signature) in cases {
        let keys = DeviceKeys::from_secrets(
            SecretBytes::from_slice(&unhex(secret)).unwrap(),
            vault_crypto::sync_keys::generate_secret().unwrap(),
        )
        .unwrap();
        assert_eq!(keys.public().ed25519_pk.to_vec(), unhex(public));
        let message = unhex(message);
        let signature = unhex(signature);
        assert_eq!(keys.sign(&message).to_vec(), signature);
        // And the strict verifier the chain uses accepts it.
        assert!(sync_core::keys::ed25519_verify_strict(
            &keys.public().ed25519_pk,
            &message,
            &signature.as_slice().try_into().unwrap()
        ));
    }
}

// ---- the genesis ---------------------------------------------------------------------------------

#[test]
fn the_genesis_is_a_self_signed_add_and_a_reason_1_record_sealed_to_the_founder_alone() {
    let founder = DeviceKeys::generate().unwrap();
    let genesis = create_genesis(VAULT, [0xa1; 16], &founder, 1_700_000_000_000_000).unwrap();
    let state = verify_chain(&VAULT, &[&genesis.membership], &[&genesis.epoch]).unwrap();
    assert_eq!(state.head_version(), 1);
    assert_eq!(state.key_epoch(), 1);

    let body = decode(&genesis.epoch);
    assert_eq!(body.reason, Reason::Genesis);
    assert_eq!(body.key_epoch, 1);
    assert_eq!(body.prev_epoch_id, None);
    assert_eq!(body.recipients.len(), 1);
    assert_eq!(body.recipients[0].fingerprint, founder.public().fingerprint);
    assert_eq!(body.recipients[0].enc.len(), 32);
    assert_eq!(body.recipients[0].ciphertext.len(), 109);
    assert_eq!(body.sealer_fingerprint, founder.public().fingerprint);
}

#[test]
fn the_founder_installs_its_keys_only_by_accepting_its_own_record() {
    let founder = DeviceKeys::generate().unwrap();
    let genesis = create_genesis(VAULT, [0xa1; 16], &founder, 1_700_000_000_000_000).unwrap();
    let accepted = accept_records(
        &VerifiedState::empty(VAULT),
        &[&genesis.membership],
        &[&genesis.epoch],
        &founder,
    )
    .unwrap();
    assert_eq!(accepted.installs.len(), 1);
    assert_eq!(accepted.installs[0].epoch_key.key_epoch(), 1);
    assert_eq!(accepted.installs[0].record_id, id_of(&genesis.epoch));
    assert!(accepted.tag_secret.is_some());
}

#[test]
fn two_geneses_for_one_vault_do_not_share_keys() {
    let founder = DeviceKeys::generate().unwrap();
    let a = create_genesis(VAULT, [0xa1; 16], &founder, 1).unwrap();
    let b = create_genesis(VAULT, [0xa1; 16], &founder, 1).unwrap();
    assert_ne!(
        decode(&a.epoch).key_commitment_ke,
        decode(&b.epoch).key_commitment_ke
    );
    assert_ne!(
        decode(&a.epoch).key_commitment_s,
        decode(&b.epoch).key_commitment_s
    );
}

// ---- acceptance: every device gets the keys the record committed to ----------------------------------

#[test]
fn each_member_unseals_the_keys_the_chain_committed_to() {
    let c = chain();
    let a = accept_all(&c, &c.a.keys).unwrap();
    // A is a recipient of the genesis, the cutover and the rotation; not of the grants.
    let epochs: Vec<u64> = a.installs.iter().map(|i| i.epoch_key.key_epoch()).collect();
    assert_eq!(epochs, [1, 2, 3]);
    let b = accept_all(&c, &c.b.keys).unwrap();
    let epochs: Vec<u64> = b.installs.iter().map(|i| i.epoch_key.key_epoch()).collect();
    assert_eq!(
        epochs,
        [1, 2, 3],
        "B: the epoch 1 grant, then the cutover and the rotation"
    );
    // Both devices hold the same K_e for the same epoch: the derived AEAD keys agree.
    for (x, y) in a.installs.iter().zip(&b.installs) {
        assert_eq!(
            aead_key(&x.epoch_key, &c.a.fingerprint(), ObjectType::Envelope).expose(),
            aead_key(&y.epoch_key, &c.a.fingerprint(), ObjectType::Envelope).expose()
        );
    }
    assert_eq!(a.state, b.state);
    // And the keys are the ones the test sealed.
    assert_eq!(
        aead_key(
            &a.installs[0].epoch_key,
            &c.a.fingerprint(),
            ObjectType::Envelope
        )
        .expose(),
        aead_key(&epoch_key(1), &c.a.fingerprint(), ObjectType::Envelope).expose()
    );
}

#[test]
fn a_revoked_device_gets_nothing_from_the_cutover() {
    let c = chain();
    let accepted = accept_all(&c, &c.c.keys).unwrap();
    let epochs: Vec<u64> = accepted
        .installs
        .iter()
        .map(|i| i.epoch_key.key_epoch())
        .collect();
    assert_eq!(
        epochs,
        [1],
        "C held epoch 1 through its grant and is not a recipient after it"
    );
}

#[test]
fn a_stranger_gets_nothing_and_is_told_nothing() {
    let c = chain();
    let accepted = accept_all(&c, &dev(0xee).keys).unwrap();
    assert!(accepted.installs.is_empty());
    assert!(accepted.tag_secret.is_none());
}

#[test]
fn accepting_the_same_walk_twice_is_idempotent() {
    let c = chain();
    let first = accept_all(&c, &c.b.keys).unwrap();
    let again = accept_records(&first.state, &[], &[], &c.b.keys).unwrap();
    assert!(again.installs.is_empty(), "nothing new to install");
    assert_eq!(again.state, first.state);
}

// ---- AC4: rejections, each typed and before any key is used ------------------------------------------

#[test]
fn a_forged_record_is_refused_by_its_signature_before_any_key_is_opened() {
    let c = chain();
    // The record seals DIFFERENT keys than its header commits to, so it would fail with
    // KeyCommitmentMismatch if HPKE were reached; it is also signed by someone who is not a
    // member. The first failure in C.6's order wins: the signature.
    let forger = dev(0xf0);
    let mut body = decode(&c.epochs[0]);
    body.key_commitment_ke = epoch_key_commitment(&SecretBytes::new([0x01; 32]));
    let forged = resign(&forger, &body);
    let error = accept_records(
        &VerifiedState::empty(VAULT),
        &refs(&c.memberships[..1]),
        &[&forged],
        &c.a.keys,
    )
    .unwrap_err();
    assert!(matches!(error, AcceptError::Chain(_)), "{error:?}");
}

#[test]
fn a_flipped_signature_byte_is_refused_before_any_key_is_opened() {
    let c = chain();
    let mut forged = c.epochs[0].clone();
    let last = forged.len() - 1;
    forged[last] ^= 1;
    let error = accept_records(
        &VerifiedState::empty(VAULT),
        &refs(&c.memberships[..1]),
        &[&forged],
        &c.a.keys,
    )
    .unwrap_err();
    assert!(
        matches!(
            error,
            AcceptError::Chain(ChainError::EpochBadSignature { at: 0 })
        ),
        "{error:?}"
    );
}

#[test]
fn an_entry_replayed_from_an_old_epoch_does_not_open() {
    let c = chain();
    // B's entry in the epoch 2 cutover, copied into the epoch 3 rotation and re-signed by the
    // (malicious) authorized sealer: HPKE's info and aad bind the old header, so it cannot open.
    let old = decode(&c.epochs[3]);
    let mut replayed = decode(&c.epochs[4]);
    let fingerprint_b = c.b.fingerprint();
    let from_old = old
        .recipients
        .iter()
        .find(|r| r.fingerprint == fingerprint_b)
        .unwrap()
        .clone();
    for entry in &mut replayed.recipients {
        if entry.fingerprint == fingerprint_b {
            *entry = from_old.clone();
        }
    }
    let record = resign(&c.a, &replayed);
    let mut epochs = refs(&c.epochs[..4]);
    epochs.push(&record);
    let error = accept_records(
        &VerifiedState::empty(VAULT),
        &refs(&c.memberships),
        &epochs,
        &c.b.keys,
    )
    .unwrap_err();
    assert_eq!(error, AcceptError::EpochRecordInvalid { at: 4 });
    assert_eq!(error.name(), "EpochRecordInvalid");
}

#[test]
fn an_entry_moved_into_another_recipients_slot_does_not_open() {
    let c = chain();
    // A's entry placed under B's fingerprint in the same record: aad binds the recipient.
    let mut body = decode(&c.epochs[3]);
    let a_entry = body
        .recipients
        .iter()
        .find(|r| r.fingerprint == c.a.fingerprint())
        .unwrap()
        .clone();
    for entry in &mut body.recipients {
        if entry.fingerprint == c.b.fingerprint() {
            entry.enc = a_entry.enc;
            entry.ciphertext = a_entry.ciphertext.clone();
        }
    }
    let record = resign(&c.a, &body);
    let mut epochs = refs(&c.epochs[..3]);
    epochs.push(&record);
    let error = accept_records(
        &VerifiedState::empty(VAULT),
        &refs(&c.memberships),
        &epochs,
        &c.b.keys,
    )
    .unwrap_err();
    assert_eq!(error, AcceptError::EpochRecordInvalid { at: 3 });
}

#[test]
fn a_record_that_seals_different_keys_to_different_recipients_is_caught_by_the_one_who_got_the_wrong_key(
) {
    let c = chain();
    // The cutover seals the committed K_2 to A and a DIFFERENT key to B.
    let mut body = decode(&c.epochs[3]);
    let header_owner = &c.a;
    let honest = vault_crypto::sync_keyset::seal_key_set(
        &c.a.public.kem_public_key,
        &body.hpke_info(&c.a.id, &c.a.fingerprint()),
        &body.hpke_aad(&c.a.fingerprint()),
        &SecretBytes::new([2 ^ 0x40; 32]),
        &SecretBytes::new([0x77; 32]),
    )
    .unwrap();
    let wrong = vault_crypto::sync_keyset::seal_key_set(
        &c.b.public.kem_public_key,
        &body.hpke_info(&c.b.id, &c.b.fingerprint()),
        &body.hpke_aad(&c.b.fingerprint()),
        &SecretBytes::new([2 ^ 0xa0; 32]),
        &SecretBytes::new([0x77; 32]),
    )
    .unwrap();
    for entry in &mut body.recipients {
        let sealed = if entry.fingerprint == c.a.fingerprint() {
            &honest
        } else {
            &wrong
        };
        entry.enc = sealed.enc;
        entry.ciphertext = sealed.ciphertext.clone();
    }
    let record = resign(header_owner, &body);
    let mut epochs = refs(&c.epochs[..3]);
    epochs.push(&record);
    // A cannot see B's entry: A accepts.
    let for_a = accept_records(
        &VerifiedState::empty(VAULT),
        &refs(&c.memberships),
        &epochs,
        &c.a.keys,
    );
    assert!(for_a.is_ok(), "{for_a:?}");
    // B opens its entry, finds a key that does not match the record's commitment, and installs
    // nothing: the error is the only thing returned.
    let for_b = accept_records(
        &VerifiedState::empty(VAULT),
        &refs(&c.memberships),
        &epochs,
        &c.b.keys,
    )
    .unwrap_err();
    assert_eq!(for_b, AcceptError::KeyCommitmentMismatch { at: 3 });
    assert_eq!(for_b.name(), "KeyCommitmentMismatch");
}

#[test]
fn a_wrong_tag_secret_is_a_commitment_mismatch_too() {
    let c = chain();
    let mut body = decode(&c.epochs[0]);
    // Seal a different S than the header commits to, to the founder, and re-sign as the founder.
    let sealed = vault_crypto::sync_keyset::seal_key_set(
        &c.a.public.kem_public_key,
        &body.hpke_info(&c.a.id, &c.a.fingerprint()),
        &body.hpke_aad(&c.a.fingerprint()),
        &SecretBytes::new([1 ^ 0x40; 32]),
        &SecretBytes::new([0x78; 32]),
    )
    .unwrap();
    body.recipients[0].enc = sealed.enc;
    body.recipients[0].ciphertext = sealed.ciphertext;
    let record = resign(&c.a, &body);
    let error = accept_records(
        &VerifiedState::empty(VAULT),
        &refs(&c.memberships[..1]),
        &[&record],
        &c.a.keys,
    )
    .unwrap_err();
    assert_eq!(error, AcceptError::KeyCommitmentMismatch { at: 0 });
    // The tag commitment is a different function from the epoch commitment.
    let same = SecretBytes::new([9; 32]);
    assert_ne!(epoch_key_commitment(&same), tag_key_commitment(&same));
}

#[test]
fn a_failure_late_in_a_walk_returns_no_keys_from_the_earlier_records() {
    let c = chain();
    // Records 0 to 2 are good for B; the cutover's entry for B is damaged and re-signed.
    let mut body = decode(&c.epochs[3]);
    for entry in &mut body.recipients {
        if entry.fingerprint == c.b.fingerprint() {
            entry.ciphertext[0] ^= 1;
        }
    }
    let record = resign(&c.a, &body);
    let mut epochs = refs(&c.epochs[..3]);
    epochs.push(&record);
    let result = accept_records(
        &VerifiedState::empty(VAULT),
        &refs(&c.memberships),
        &epochs,
        &c.b.keys,
    );
    // `Result::Err` carries no `Accepted`: there is nothing to install.
    assert_eq!(
        result.err(),
        Some(AcceptError::EpochRecordInvalid { at: 3 })
    );
}

#[test]
fn a_member_missing_from_a_record_that_lists_everyone_is_refused_by_the_chain() {
    let c = chain();
    let mut body = decode(&c.epochs[3]);
    body.recipients
        .retain(|r| r.fingerprint != c.b.fingerprint());
    let record = resign(&c.a, &body);
    let mut epochs = refs(&c.epochs[..3]);
    epochs.push(&record);
    let error = accept_records(
        &VerifiedState::empty(VAULT),
        &refs(&c.memberships),
        &epochs,
        &c.b.keys,
    )
    .unwrap_err();
    assert!(
        matches!(
            error,
            AcceptError::Chain(ChainError::RecipientsMismatch { at: 3 })
        ),
        "{error:?}"
    );
}

#[test]
fn a_cutover_that_reseals_an_old_key_is_refused_by_the_chain() {
    let c = chain();
    // A cutover that re-seals an old K_e (K_1) is invalid: reasons 2 and 4 must publish a
    // commitment no earlier record carried.
    let reseal = epoch_record(
        Reason::Cutover,
        id_of(&c.memberships[3]),
        Some(id_of(&c.epochs[2])),
        &c.a,
        &epoch_key(1),
        &tag_secret(0x77),
        &[&c.a, &c.b],
    );
    let mut epochs = refs(&c.epochs[..3]);
    epochs.push(&reseal);
    let error = accept_records(
        &VerifiedState::empty(VAULT),
        &refs(&c.memberships),
        &epochs,
        &c.a.keys,
    )
    .unwrap_err();
    assert!(matches!(error, AcceptError::Chain(_)), "{error:?}");
}

#[test]
fn a_changed_s_is_refused_by_the_chain() {
    let c = chain();
    let changed = epoch_record(
        Reason::Cutover,
        id_of(&c.memberships[3]),
        Some(id_of(&c.epochs[2])),
        &c.a,
        &epoch_key(2),
        &tag_secret(0x99),
        &[&c.a, &c.b],
    );
    let mut epochs = refs(&c.epochs[..3]);
    epochs.push(&changed);
    let error = accept_records(
        &VerifiedState::empty(VAULT),
        &refs(&c.memberships),
        &epochs,
        &c.a.keys,
    )
    .unwrap_err();
    assert!(matches!(error, AcceptError::Chain(_)), "{error:?}");
}

#[test]
fn a_membership_chain_with_a_gap_is_refused_before_any_key_is_opened() {
    let c = chain();
    // m3 is missing from the walk, so m4 does not link: the chain fails, and no key is touched.
    let memberships = [
        c.memberships[0].as_slice(),
        c.memberships[1].as_slice(),
        c.memberships[3].as_slice(),
    ];
    let error = accept_records(
        &VerifiedState::empty(VAULT),
        &memberships,
        &refs(&c.epochs),
        &c.a.keys,
    )
    .unwrap_err();
    assert!(matches!(error, AcceptError::Chain(_)), "{error:?}");
}

#[test]
fn no_debug_output_of_an_accepted_walk_shows_key_bytes() {
    let c = chain();
    let accepted = accept_all(&c, &c.a.keys).unwrap();
    let shown = format!("{accepted:?}");
    assert!(shown.contains("EpochKey"), "{shown}");
    // The walk's real secrets: K_1, K_2 and K_3 (`epoch ^ 0x40` repeated) and the tag secret S
    // (`0x77` repeated). Neither their full 32-element array form nor their full hex form may
    // appear. A pattern this long cannot be hit by the random ids the same output prints.
    for byte in [0x41_u8, 0x42, 0x43, 0x77] {
        assert!(
            !shown.contains(&format!("{:?}", [byte; 32])),
            "key bytes {byte:#04x} in array form: {shown}"
        );
        assert!(
            !shown.contains(&format!("{byte:02x}").repeat(32)),
            "key bytes {byte:#04x} in hex form: {shown}"
        );
    }
}
