//! Sealing and opening Sync envelopes (vault-format.md C.3 and C.4; personal-cfo-g3m.22): the C.14
//! vectors, the identity rules that stop nonce reuse across devices, the receiver path's
//! membership window, tamper resistance, and a guard that the receiver path is the only
//! production `EnvelopeOpener`.

use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};
use sync_core::envelope::{ArtifactManifest, EnvelopePlaintext, IdempotencyKey, ImmutableMeta};
use sync_core::header::{
    encode_container, parse_container, ClearHeader, Fingerprint, IdempotencyKeyTag,
    ManifestCommitment, NonceDomain, ObjectKind, OBJECT_ENVELOPE,
};
use sync_core::membership::{
    device_fingerprint, verify_chain, Action, EpochBody, MembershipBody, Reason, Recipient,
    SignedRecord, VerifiedState, KEM_ID, SEALED_KEYS_CIPHERTEXT_LEN, SUITE_ID,
};
use sync_core::producer::{verify_before_upload, ProducerError};
use vault_crypto::sync::{idempotency_key_tag, idempotency_tag_key};
use vault_crypto::sync_envelope::{
    aead_key, seal_envelope, EpochKey, KeyError, ObjectType, ReceiverOpener, SealError,
    SealerIdentity,
};
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

fn commitment(key: &[u8; 32]) -> [u8; 32] {
    let mut input = vec![0, 32];
    input.extend_from_slice(b"dohflow/sync-epoch-key-commit/v1");
    input.extend_from_slice(key);
    Sha256::digest(&input).into()
}

fn epoch_key(vault: [u8; 16], epoch: u64, bytes: [u8; 32]) -> EpochKey {
    EpochKey::from_verified(vault, epoch, SecretBytes::new(bytes), &commitment(&bytes))
        .expect("the commitment matches")
}

// ---- the C.14 vectors ---------------------------------------------------------------------

#[test]
fn the_aead_key_and_the_sealed_envelope_match_the_spec_vectors() {
    // K_e is 00 01 .. 1f, the vault is 0x11 repeated, epoch 1, device A (C.14).
    let k_e: [u8; 32] = std::array::from_fn(|i| i as u8);
    let container_bytes = vector("envelope_container");
    let container = parse_container(&container_bytes).unwrap();
    let header = &container.header;
    let key = EpochKey::from_verified(
        header.vault_id,
        header.key_epoch,
        SecretBytes::new(k_e),
        &vector("key_commitment_ke").try_into().unwrap(),
    )
    .expect("K_e matches the vector commitment");
    assert_eq!(
        aead_key(&key, &header.device_key_fingerprint, ObjectType::Envelope)
            .expose()
            .as_slice(),
        vector("aead_key_a_envelope")
    );
    // The nonce is the header's domain then the counter: the vector.
    let mut nonce = header.nonce_domain.as_bytes().to_vec();
    nonce.extend_from_slice(&header.invocation_counter.to_be_bytes());
    assert_eq!(nonce, vector("nonce_a_envelope_counter0"));
    // Sealing the vector plaintext under that header reproduces the vector ciphertext and the whole
    // container byte for byte.
    let sealer = SealerIdentity {
        vault_id: header.vault_id,
        device_id: header.device_id,
        fingerprint: header.device_key_fingerprint,
    };
    let sealed = seal_envelope(&key, &sealer, header, &vector("envelope_plaintext")).unwrap();
    assert_eq!(sealed, container_bytes);
    assert_eq!(
        parse_container(&sealed).unwrap().ciphertext,
        vector("envelope_ciphertext")
    );
}

#[test]
fn a_key_that_does_not_match_the_commitment_is_not_an_epoch_key() {
    let bytes = [7_u8; 32];
    assert!(matches!(
        EpochKey::from_verified([1; 16], 1, SecretBytes::new(bytes), &[0; 32]),
        Err(KeyError::CommitmentMismatch)
    ));
    // Another key's commitment is no better.
    assert!(matches!(
        EpochKey::from_verified([1; 16], 1, SecretBytes::new(bytes), &commitment(&[8; 32])),
        Err(KeyError::CommitmentMismatch)
    ));
}

#[test]
fn the_object_types_have_their_own_keys() {
    let key = epoch_key([1; 16], 1, [7; 32]);
    let fingerprint = Fingerprint::from_derived([2; 32]);
    assert_ne!(
        aead_key(&key, &fingerprint, ObjectType::Envelope).expose(),
        aead_key(&key, &fingerprint, ObjectType::Snapshot).expose()
    );
}

#[test]
fn a_debug_print_of_an_epoch_key_shows_no_key_bytes() {
    let key = epoch_key([1; 16], 3, [0xab; 32]);
    let shown = format!("{key:?}");
    assert!(
        !shown.contains("ab") || shown.matches("ab").count() < 2,
        "{shown}"
    );
    assert!(shown.contains("key_epoch"));
}

// ---- a real chain and real envelopes ------------------------------------------------------

const VAULT: [u8; 16] = [0x11; 16];

struct Dev {
    signing: SigningKey,
    ed25519_pk: [u8; 32],
    kem_pk: [u8; 32],
    device_id: [u8; 16],
}

impl Dev {
    fn new(n: u8) -> Self {
        let signing = SigningKey::from_bytes(&[n; 32]);
        let mut device_id = [0_u8; 16];
        device_id[6] = 0x40;
        device_id[8] = 0x80;
        device_id[15] = n;
        Self {
            ed25519_pk: signing.verifying_key().to_bytes(),
            signing,
            kem_pk: {
                let mut k = [0x30 + n; 32];
                k[31] &= 0x7f;
                k
            },
            device_id,
        }
    }

    fn fingerprint(&self) -> Fingerprint {
        device_fingerprint(SUITE_ID, KEM_ID, &self.ed25519_pk, &self.kem_pk)
    }

    fn identity(&self) -> SealerIdentity {
        SealerIdentity {
            vault_id: VAULT,
            device_id: self.device_id,
            fingerprint: self.fingerprint(),
        }
    }
}

fn sign(body: &[u8], by: &SigningKey) -> Vec<u8> {
    SignedRecord {
        body: body.to_vec(),
        signature: by.sign(body).to_bytes(),
    }
    .encode()
    .unwrap()
}

fn membership(
    version: u64,
    prev: Option<[u8; 32]>,
    action: Action,
    signer: &Dev,
    subject: &Dev,
) -> (Vec<u8>, [u8; 32]) {
    let body = MembershipBody {
        suite_id: SUITE_ID,
        kem_id: KEM_ID,
        vault_id: VAULT,
        version,
        prev_id: prev,
        action,
        signer_fingerprint: signer.fingerprint(),
        device_id: subject.device_id,
        ed25519_pk: subject.ed25519_pk,
        kem_public_key: subject.kem_pk,
        issued_at: 1_790_000_000_000_000 + version as i64,
    }
    .encode()
    .unwrap();
    let id = Sha256::digest(&body).into();
    (sign(&body, &signer.signing), id)
}

fn epoch(
    sealer: &Dev,
    key_epoch: u64,
    reason: Reason,
    head: [u8; 32],
    prev: Option<[u8; 32]>,
    ke: [u8; 32],
    recipients: &[&Dev],
) -> (Vec<u8>, [u8; 32]) {
    let mut recipients: Vec<Recipient> = recipients
        .iter()
        .map(|d| Recipient {
            device_id: d.device_id,
            fingerprint: d.fingerprint(),
            suite_id: SUITE_ID,
            kem_id: KEM_ID,
            enc: [0xee; 32],
            ciphertext: vec![0xcc; SEALED_KEYS_CIPHERTEXT_LEN],
        })
        .collect();
    recipients.sort_by_key(|r| *r.fingerprint.as_bytes());
    let body = EpochBody {
        suite_id: SUITE_ID,
        kem_id: KEM_ID,
        vault_id: VAULT,
        key_epoch,
        reason,
        membership_head: head,
        prev_epoch_id: prev,
        key_commitment_ke: commitment(&ke),
        key_commitment_s: [2; 32],
        sealer_fingerprint: sealer.fingerprint(),
        recipients,
    }
    .encode()
    .unwrap();
    let id = Sha256::digest(&body).into();
    (sign(&body, &sealer.signing), id)
}

const K1: [u8; 32] = [0x41; 32];
const K2: [u8; 32] = [0x42; 32];

/// A adds B in epoch 1, then revokes B in a cutover into epoch 2.
fn chain(a: &Dev, b: &Dev) -> VerifiedState {
    let (m1, id_m1) = membership(1, None, Action::Add, a, a);
    let (e1, id_e1) = epoch(a, 1, Reason::Genesis, id_m1, None, K1, &[a]);
    let (m2, id_m2) = membership(2, Some(id_m1), Action::Add, a, b);
    let (e2, id_e2) = epoch(a, 1, Reason::Grant, id_m2, Some(id_e1), K1, &[b]);
    let (m3, id_m3) = membership(3, Some(id_m2), Action::Revoke, a, b);
    let (e3, _) = epoch(a, 2, Reason::Cutover, id_m3, Some(id_e2), K2, &[a]);
    verify_chain(&VAULT, &[&m1, &m2, &m3], &[&e1, &e2, &e3]).expect("the chain is valid")
}

fn tag_for(key: &IdempotencyKey) -> IdempotencyKeyTag {
    let s = SecretBytes::new([0x33; 32]);
    IdempotencyKeyTag::from_derived(idempotency_key_tag(
        &idempotency_tag_key(&s, &VAULT),
        key.as_bytes(),
    ))
}

/// A coherent header and plaintext for a command sealed by `dev` in `key_epoch`.
fn parts(dev: &Dev, command: u8, key_epoch: u64, counter: u64) -> (ClearHeader, EnvelopePlaintext) {
    let mut key = [command; 16];
    key[6] = 0x40 | (key[6] & 0x0f);
    key[8] = 0x80 | (key[8] & 0x3f);
    let meta = ImmutableMeta {
        command_id: [command; 16],
        correlation_id: [0x22; 16],
        causation_id: None,
        actor_type: 1,
        actor_id: "tester".to_owned(),
        idempotency_key: IdempotencyKey::from_bytes(key).unwrap(),
        issued_at: 1_790_000_000_000_000,
        device_id: dev.device_id,
    };
    let fingerprint = dev.fingerprint();
    let header = ClearHeader {
        vault_id: VAULT,
        key_epoch,
        object: ObjectKind::Envelope {
            idempotency_key_tag: tag_for(&meta.idempotency_key),
            immutable_metadata_commitment: meta.commitment().unwrap(),
            artifact_manifest_commitment: None,
        },
        object_id: meta.command_id,
        device_id: dev.device_id,
        device_key_fingerprint: fingerprint,
        nonce_domain: NonceDomain::derive(&VAULT, key_epoch, &fingerprint, OBJECT_ENVELOPE),
        invocation_counter: counter,
        base_seq: 41,
        payload_schema_version: 1,
        engine_version: "dohflow-engine-1".to_owned(),
        schema_version: 55,
    };
    let plaintext = EnvelopePlaintext {
        meta,
        write_set_hash: [0x44; 32],
        delta_manifest: ArtifactManifest::new(vec![]).unwrap(),
        payload: b"{}".to_vec(),
    };
    (header, plaintext)
}

fn seal(key: &EpochKey, dev: &Dev, header: &ClearHeader, plaintext: &EnvelopePlaintext) -> Vec<u8> {
    seal_envelope(key, &dev.identity(), header, &plaintext.encode().unwrap()).unwrap()
}

// ---- the sealer's identity ----------------------------------------------------------------

#[test]
fn the_sealer_cannot_seal_under_another_devices_identity_and_nothing_is_encrypted() {
    let (a, b) = (Dev::new(0xa1), Dev::new(0xb1));
    let key = epoch_key(VAULT, 1, K1);
    let (good, plaintext) = parts(&a, 1, 1, 0);
    let bytes = plaintext.encode().unwrap();
    let attempt = |header: &ClearHeader| seal_envelope(&key, &a.identity(), header, &bytes);
    assert!(attempt(&good).is_ok());

    // B's fingerprint (and B's own nonce domain) in a header A is sealing: a nonce-reuse attempt
    // against B's (key, domain) with A's counter.
    let (b_header, _) = parts(&b, 1, 1, 0);
    assert_eq!(attempt(&b_header), Err(SealError::WrongDevice));
    let mut other_fingerprint = good.clone();
    other_fingerprint.device_key_fingerprint = b.fingerprint();
    assert_eq!(attempt(&other_fingerprint), Err(SealError::WrongDevice));
    let mut other_device_id = good.clone();
    other_device_id.device_id = b.device_id;
    assert_eq!(attempt(&other_device_id), Err(SealError::WrongDevice));

    let mut other_vault = good.clone();
    other_vault.vault_id[0] ^= 1;
    assert_eq!(attempt(&other_vault), Err(SealError::WrongVault));
    let mut other_epoch = good.clone();
    other_epoch.key_epoch = 2;
    assert_eq!(attempt(&other_epoch), Err(SealError::WrongEpoch));
    let mut other_domain = good.clone();
    other_domain.nonce_domain = NonceDomain::from_derived([0; 4]);
    assert_eq!(attempt(&other_domain), Err(SealError::WrongNonceDomain));
    let mut snapshot = good.clone();
    snapshot.object = ObjectKind::Snapshot {
        artifact_manifest_commitment: ManifestCommitment::from_derived([1; 32]),
    };
    assert_eq!(attempt(&snapshot), Err(SealError::NotAnEnvelope));

    // A key for another vault or epoch is refused too.
    assert_eq!(
        seal_envelope(&epoch_key([9; 16], 1, K1), &a.identity(), &good, &bytes),
        Err(SealError::WrongVault)
    );
    assert_eq!(
        seal_envelope(&epoch_key(VAULT, 2, K2), &a.identity(), &good, &bytes),
        Err(SealError::WrongEpoch)
    );
}

#[test]
fn the_counter_is_the_callers_and_a_reused_counter_gives_the_same_nonce() {
    // The seal function never chooses a counter. Documented by test: the same key, counter and
    // plaintext give identical output, and a new counter changes the ciphertext, so the caller
    // persisting the advance before sealing is what keeps nonces unique.
    let a = Dev::new(0xa1);
    let key = epoch_key(VAULT, 1, K1);
    let (h0, p) = parts(&a, 1, 1, 0);
    let (h1, _) = parts(&a, 1, 1, 1);
    assert_eq!(seal(&key, &a, &h0, &p), seal(&key, &a, &h0, &p));
    assert_ne!(
        parse_container(&seal(&key, &a, &h0, &p))
            .unwrap()
            .ciphertext,
        parse_container(&seal(&key, &a, &h1, &p))
            .unwrap()
            .ciphertext
    );
}

// ---- the receiver path --------------------------------------------------------------------

#[test]
fn a_sealed_envelope_opens_through_the_receiver_path_and_is_approved_for_upload() {
    let (a, b) = (Dev::new(0xa1), Dev::new(0xb1));
    let state = chain(&a, &b);
    let key = epoch_key(VAULT, 1, K1);
    let opener = ReceiverOpener::new(&key, &state);
    for dev in [&a, &b] {
        let (header, plaintext) = parts(dev, 1, 1, 7);
        let bytes = seal(&key, dev, &header, &plaintext);
        // The producer's seal-then-verify uses this SAME opener.
        let approved = verify_before_upload(&bytes, &opener, tag_for).expect("coherent and sealed");
        assert_eq!(approved.bytes(), bytes.as_slice());
    }
}

#[test]
fn a_mis_sealed_envelope_opens_but_is_refused_as_inconsistent() {
    let a = Dev::new(0xa1);
    let b = Dev::new(0xb1);
    let state = chain(&a, &b);
    let key = epoch_key(VAULT, 1, K1);
    let (mut header, plaintext) = parts(&a, 1, 1, 0);
    if let ObjectKind::Envelope {
        idempotency_key_tag,
        ..
    } = &mut header.object
    {
        *idempotency_key_tag = IdempotencyKeyTag::from_derived([0; 32]);
    }
    let bytes = seal(&key, &a, &header, &plaintext);
    let result = verify_before_upload(&bytes, &ReceiverOpener::new(&key, &state), tag_for);
    assert!(
        matches!(result, Err(ProducerError::Inconsistent(_))),
        "{result:?}"
    );
}

#[test]
fn an_envelope_sealed_by_a_device_revoked_later_still_opens_in_its_own_epoch() {
    // B was a member of epoch 1 and was revoked at the cutover into epoch 2. Its epoch-1
    // envelopes are still in the retained tail; a replica catching up must open them.
    let (a, b) = (Dev::new(0xa1), Dev::new(0xb1));
    let state = chain(&a, &b);
    let key1 = epoch_key(VAULT, 1, K1);
    let key2 = epoch_key(VAULT, 2, K2);
    let (header, plaintext) = parts(&b, 5, 1, 3);
    let bytes = seal(&key1, &b, &header, &plaintext);
    let container = parse_container(&bytes).unwrap();
    use sync_core::producer::EnvelopeOpener;
    assert!(ReceiverOpener::new(&key1, &state)
        .open(&container)
        .is_some());

    // The same device claiming epoch 2 (sealed, as it could by itself, with epoch 2's key) is not
    // a member of epoch 2.
    let (header2, plaintext2) = parts(&b, 6, 2, 0);
    let bytes2 = seal(&key2, &b, &header2, &plaintext2);
    let container2 = parse_container(&bytes2).unwrap();
    assert!(ReceiverOpener::new(&key2, &state)
        .open(&container2)
        .is_none());
    // A, who is, opens in both.
    let (ha, pa) = parts(&a, 7, 2, 0);
    let bytes_a = seal(&key2, &a, &ha, &pa);
    assert!(ReceiverOpener::new(&key2, &state)
        .open(&parse_container(&bytes_a).unwrap())
        .is_some());
}

#[test]
fn the_receiver_path_refuses_every_wrong_expectation() {
    use sync_core::producer::EnvelopeOpener;
    let (a, b) = (Dev::new(0xa1), Dev::new(0xb1));
    let stranger = Dev::new(0xc1);
    let state = chain(&a, &b);
    let key = epoch_key(VAULT, 1, K1);
    let (header, plaintext) = parts(&a, 1, 1, 0);
    let bytes = seal(&key, &a, &header, &plaintext);
    let container = parse_container(&bytes).unwrap();
    assert!(ReceiverOpener::new(&key, &state).open(&container).is_some());

    // A wrong K_e (with its own, matching, commitment) fails the AEAD.
    let wrong_key = epoch_key(VAULT, 1, [0x99; 32]);
    assert!(ReceiverOpener::new(&wrong_key, &state)
        .open(&container)
        .is_none());
    // A key of another epoch or vault is refused before any AEAD.
    assert!(ReceiverOpener::new(&epoch_key(VAULT, 2, K2), &state)
        .open(&container)
        .is_none());
    assert!(ReceiverOpener::new(&epoch_key([9; 16], 1, K1), &state)
        .open(&container)
        .is_none());
    // A device that is not in the chain: even sealed consistently, it does not open.
    let (sh, sp) = parts(&stranger, 2, 1, 0);
    let stranger_bytes = seal(&key, &stranger, &sh, &sp);
    assert!(ReceiverOpener::new(&key, &state)
        .open(&parse_container(&stranger_bytes).unwrap())
        .is_none());
    // A header that restates a consistent but WRONG identity: a's device id with b's fingerprint.
    let mut forged = header.clone();
    forged.device_key_fingerprint = b.fingerprint();
    forged.nonce_domain = NonceDomain::derive(&VAULT, 1, &b.fingerprint(), OBJECT_ENVELOPE);
    let forged_bytes = encode_container(&forged, container.ciphertext).unwrap();
    assert!(ReceiverOpener::new(&key, &state)
        .open(&parse_container(&forged_bytes).unwrap())
        .is_none());
}

#[test]
fn flipping_any_single_byte_of_a_sealed_envelope_stops_it_opening() {
    use sync_core::producer::EnvelopeOpener;
    let (a, b) = (Dev::new(0xa1), Dev::new(0xb1));
    let state = chain(&a, &b);
    let key = epoch_key(VAULT, 1, K1);
    let (header, plaintext) = parts(&a, 1, 1, 0);
    let bytes = seal(&key, &a, &header, &plaintext);
    let opener = ReceiverOpener::new(&key, &state);
    assert!(opener.open(&parse_container(&bytes).unwrap()).is_some());
    for index in 0..bytes.len() {
        let mut tampered = bytes.clone();
        tampered[index] ^= 0x01;
        // Either the container no longer parses, or it does and the object does not open.
        if let Ok(container) = parse_container(&tampered) {
            assert!(
                opener.open(&container).is_none(),
                "byte {index} was not authenticated"
            );
        }
    }
    // A truncated tag.
    let truncated = &bytes[..bytes.len() - 1];
    if let Ok(container) = parse_container(truncated) {
        assert!(opener.open(&container).is_none());
    }
}

// ---- the only-implementation guard --------------------------------------------------------

/// Every `impl ... EnvelopeOpener for ...` in the workspace must be the receiver path in
/// `vault-crypto`, or a marked test double under a `tests/` directory. Otherwise "verified by
/// opening" could mean "handed back the plaintext the producer still held".
#[test]
fn the_receiver_path_is_the_only_production_envelope_opener() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut found = Vec::new();
    let mut stack = vec![root.join("crates"), root.join("apps")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if path.is_dir() {
                if name != "target" && name != "node_modules" && !name.starts_with('.') {
                    stack.push(path);
                }
            } else if name.ends_with(".rs") {
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                let lines: Vec<&str> = text.lines().collect();
                for (i, line) in lines.iter().enumerate() {
                    let t = line.trim_start();
                    if t.starts_with("impl") && t.contains("EnvelopeOpener for") {
                        let preceding = lines[i.saturating_sub(4)..i].join("\n");
                        found.push((path.clone(), preceding.contains("TEST DOUBLE")));
                    }
                }
            }
        }
    }
    assert!(
        !found.is_empty(),
        "the guard found no implementations at all"
    );
    for (path, marked) in found {
        let shown = path.to_string_lossy().replace('\\', "/");
        let is_receiver_path = shown.ends_with("crates/vault-crypto/src/sync_envelope.rs");
        let is_marked_test_double = shown.contains("/tests/") && marked;
        assert!(
            is_receiver_path || is_marked_test_double,
            "{shown} implements EnvelopeOpener: only the receiver path in vault-crypto \
             (or a marked TEST DOUBLE under tests/) may"
        );
    }
}
