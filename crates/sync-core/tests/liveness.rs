//! Sync liveness (personal-cfo-g3m.17): the producer's seal-then-verify rule, the service's
//! structural push checks, and the replica's sync status. The AEAD itself is out of scope of
//! `sync-core`, so the producer tests use a **test double** opener that returns the plaintext the
//! test sealed; the production opener is the receiver path in `vault-crypto`.

use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};
use sync_core::accepted::PushIdentity;
use sync_core::accepted::{decide_push, InMemoryIndex, PushDecision};
use sync_core::cbe::WireError;
use sync_core::envelope::{
    canonical_envelope_digest, check_envelope, ArtifactManifest, EnvelopePlaintext, IdempotencyKey,
    ImmutableMeta, ReceiverError,
};
use sync_core::header::{
    encode_container, parse_container, ClearHeader, Fingerprint, IdempotencyKeyTag,
    ManifestCommitment, NonceDomain, ObjectKind, ParsedContainer, MAX_ENVELOPE_CONTAINER_LEN,
};
use sync_core::membership::{
    device_fingerprint, verify_chain, Action, EpochBody, MembershipBody, Reason, Recipient,
    SignedRecord, VerifiedState, KEM_ID, SEALED_KEYS_CIPHERTEXT_LEN, SUITE_ID,
};
use sync_core::producer::{verify_before_upload, EnvelopeOpener, ProducerError};
use sync_core::service::{check_new_push, parse_push, PushRefusal, PushView};
use sync_core::status::{HaltEvidence, SyncEvent, SyncStatus};

const VAULT: [u8; 16] = [0, 0, 0, 0, 0, 0, 0x40, 0, 0x80, 0, 0, 0, 0, 0, 0, 0xf0];

/// A synthetic device (a repeated-byte seed; nothing here is a real key).
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
            kem_pk: [0x30 + n; 32],
            device_id,
        }
    }

    fn fingerprint(&self) -> Fingerprint {
        device_fingerprint(SUITE_ID, KEM_ID, &self.ed25519_pk, &self.kem_pk)
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

/// A one-device chain (genesis membership and genesis epoch) for `dev`.
fn chain_of(dev: &Dev) -> VerifiedState {
    let membership = MembershipBody {
        suite_id: SUITE_ID,
        kem_id: KEM_ID,
        vault_id: VAULT,
        version: 1,
        prev_id: None,
        action: Action::Add,
        signer_fingerprint: dev.fingerprint(),
        device_id: dev.device_id,
        ed25519_pk: dev.ed25519_pk,
        kem_public_key: dev.kem_pk,
        issued_at: 1_790_000_000_000_000,
    };
    let m_bytes = membership.encode().unwrap();
    let m_id: [u8; 32] = Sha256::digest(&m_bytes).into();
    let epoch = EpochBody {
        suite_id: SUITE_ID,
        kem_id: KEM_ID,
        vault_id: VAULT,
        key_epoch: 1,
        reason: Reason::Genesis,
        membership_head: m_id,
        prev_epoch_id: None,
        key_commitment_ke: [1; 32],
        key_commitment_s: [2; 32],
        sealer_fingerprint: dev.fingerprint(),
        recipients: vec![Recipient {
            device_id: dev.device_id,
            fingerprint: dev.fingerprint(),
            suite_id: SUITE_ID,
            kem_id: KEM_ID,
            enc: [0xee; 32],
            ciphertext: vec![0xcc; SEALED_KEYS_CIPHERTEXT_LEN],
        }],
    };
    let m = sign(&m_bytes, &dev.signing);
    let e = sign(&epoch.encode().unwrap(), &dev.signing);
    verify_chain(&VAULT, &[&m], &[&e]).expect("the synthetic chain is valid")
}

fn test_tag(key: &IdempotencyKey) -> IdempotencyKeyTag {
    IdempotencyKeyTag::from_derived(Sha256::digest(key.as_bytes()).into())
}

/// A coherent header and plaintext for a command sealed by `dev` at `key_epoch`.
fn parts(dev: &Dev, command: u8, key_epoch: u64) -> (ClearHeader, EnvelopePlaintext) {
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
            idempotency_key_tag: test_tag(&meta.idempotency_key),
            immutable_metadata_commitment: meta.commitment().unwrap(),
            artifact_manifest_commitment: None,
        },
        object_id: meta.command_id,
        device_id: dev.device_id,
        device_key_fingerprint: fingerprint,
        nonce_domain: NonceDomain::derive(&VAULT, key_epoch, &fingerprint, 1),
        invocation_counter: 0,
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

/// The sealed container for a header and plaintext (the ciphertext is filler; the test opener
/// returns the plaintext it was given for that header).
struct Sealed {
    bytes: Vec<u8>,
    plaintext: Vec<u8>,
}

fn seal(header: &ClearHeader, plaintext: &EnvelopePlaintext) -> Sealed {
    Sealed {
        bytes: encode_container(header, &[0x5a; 40]).unwrap(),
        plaintext: plaintext.encode().unwrap(),
    }
}

/// TEST DOUBLE: returns the plaintext the test sealed, whatever the container. The production
/// opener derives the key from the expected vault, epoch and chain fingerprint and runs AES-GCM.
struct TestOpener(Option<Vec<u8>>);

impl EnvelopeOpener for TestOpener {
    fn open(&self, _container: &ParsedContainer<'_>) -> Option<Vec<u8>> {
        self.0.clone()
    }
}

// ---- the producer rule ------------------------------------------------------------------

#[test]
fn a_coherent_envelope_is_approved_and_the_approval_owns_the_verified_bytes() {
    let dev = Dev::new(0x11);
    let (header, plaintext) = parts(&dev, 1, 1);
    let sealed = seal(&header, &plaintext);
    let approved = verify_before_upload(
        &sealed.bytes,
        &TestOpener(Some(sealed.plaintext.clone())),
        test_tag,
    )
    .expect("a coherent envelope is approved");
    assert_eq!(approved.bytes(), sealed.bytes.as_slice(), "the exact bytes");
    let parsed = parse_container(&sealed.bytes).unwrap();
    assert_eq!(
        approved.push(),
        &PushIdentity::from_container(&parsed).unwrap()
    );
    // The outbox entry is for these bytes: the digest is computed from them.
    let digest = canonical_envelope_digest(parsed.header_bytes, parsed.ciphertext).unwrap();
    assert_eq!(approved.push().envelope_digest(), &digest);
}

#[test]
fn different_bytes_give_a_different_approval_so_one_cannot_stand_in_for_another() {
    let dev = Dev::new(0x11);
    let (header, plaintext) = parts(&dev, 1, 1);
    let sealed = seal(&header, &plaintext);
    let opener = TestOpener(Some(sealed.plaintext.clone()));
    let first = verify_before_upload(&sealed.bytes, &opener, test_tag).unwrap();
    let mut altered = sealed.bytes.clone();
    let last = altered.len() - 1;
    altered[last] ^= 1; // a ciphertext byte
    let second = verify_before_upload(&altered, &opener, test_tag).unwrap();
    assert_ne!(first.bytes(), second.bytes());
    assert_ne!(
        first.push().envelope_digest(),
        second.push().envelope_digest()
    );
    assert_ne!(first, second);
}

#[test]
fn every_inconsistent_sealing_is_refused_with_its_reason_and_nothing_is_approved() {
    let dev = Dev::new(0x11);
    let other = Dev::new(0x22);
    type Mutation = Box<dyn Fn(&mut ClearHeader, &mut EnvelopePlaintext)>;
    let cases: Vec<(&str, Mutation, ReceiverError)> = vec![
        (
            "wrong tag",
            Box::new(|h, _| {
                if let ObjectKind::Envelope {
                    idempotency_key_tag,
                    ..
                } = &mut h.object
                {
                    *idempotency_key_tag = IdempotencyKeyTag::from_derived([0; 32]);
                }
            }),
            ReceiverError::TagMismatch,
        ),
        (
            "stale metadata commitment",
            Box::new(|_, p| p.meta.issued_at += 1),
            ReceiverError::MetadataCommitmentMismatch,
        ),
        (
            "manifest commitment present for an empty manifest",
            Box::new(|h, p| {
                if let ObjectKind::Envelope {
                    artifact_manifest_commitment,
                    ..
                } = &mut h.object
                {
                    *artifact_manifest_commitment = Some(p.delta_manifest.commitment());
                }
            }),
            ReceiverError::ManifestCommitmentMismatch,
        ),
        (
            "manifest commitment of the wrong manifest",
            Box::new(|h, p| {
                p.delta_manifest = ArtifactManifest::new(vec![[7; 32]]).unwrap();
                if let ObjectKind::Envelope {
                    artifact_manifest_commitment,
                    ..
                } = &mut h.object
                {
                    *artifact_manifest_commitment = Some(ManifestCommitment::from_derived([9; 32]));
                }
            }),
            ReceiverError::ManifestCommitmentMismatch,
        ),
        (
            "nonce domain not derived",
            Box::new(|h, _| h.nonce_domain = NonceDomain::from_derived([0; 4])),
            ReceiverError::NonceDomainMismatch,
        ),
        (
            "object id is not the command id",
            Box::new(|h, _| h.object_id[0] ^= 1),
            ReceiverError::ObjectIdMismatch,
        ),
    ];
    for (name, mutate, expected) in cases {
        let (mut header, mut plaintext) = parts(&dev, 1, 1);
        mutate(&mut header, &mut plaintext);
        let sealed = seal(&header, &plaintext);
        let result = verify_before_upload(
            &sealed.bytes,
            &TestOpener(Some(sealed.plaintext.clone())),
            test_tag,
        );
        assert_eq!(
            result.unwrap_err(),
            ProducerError::Inconsistent(expected),
            "{name}"
        );
    }
    // A different device in the header than in the plaintext.
    let (mut header, plaintext) = parts(&dev, 1, 1);
    header.device_id = other.device_id;
    let sealed = seal(&header, &plaintext);
    assert_eq!(
        verify_before_upload(&sealed.bytes, &TestOpener(Some(sealed.plaintext)), test_tag)
            .unwrap_err(),
        ProducerError::Inconsistent(ReceiverError::DeviceMismatch)
    );
}

#[test]
fn an_unopenable_garbled_or_malformed_object_is_refused_before_any_upload() {
    let dev = Dev::new(0x11);
    let (header, plaintext) = parts(&dev, 1, 1);
    let sealed = seal(&header, &plaintext);
    // The ciphertext does not open under the receiver path.
    assert_eq!(
        verify_before_upload(&sealed.bytes, &TestOpener(None), test_tag).unwrap_err(),
        ProducerError::NotOpenable
    );
    // It opens to something that is not a plaintext.
    assert!(matches!(
        verify_before_upload(&sealed.bytes, &TestOpener(Some(vec![1, 2, 3])), test_tag)
            .unwrap_err(),
        ProducerError::PlaintextInvalid(_)
    ));
    // The bytes are not a container; the container is over the size cap.
    assert!(matches!(
        verify_before_upload(&sealed.bytes[..10], &TestOpener(None), test_tag).unwrap_err(),
        ProducerError::Unparseable(_)
    ));
    let huge = vec![0_u8; MAX_ENVELOPE_CONTAINER_LEN + 1];
    assert_eq!(
        verify_before_upload(&huge, &TestOpener(None), test_tag).unwrap_err(),
        ProducerError::Unparseable(WireError::ObjectTooLarge)
    );
}

#[test]
fn an_approved_envelope_passes_the_receiver_checks_a_replica_would_run() {
    // What the producer approved is exactly what a replica's check_envelope then accepts.
    let dev = Dev::new(0x11);
    let (header, plaintext) = parts(&dev, 1, 1);
    let sealed = seal(&header, &plaintext);
    verify_before_upload(&sealed.bytes, &TestOpener(Some(sealed.plaintext)), test_tag).unwrap();
    assert!(check_envelope(&header, &plaintext, test_tag).is_ok());
}

// ---- the service's checks, in C.10's order: parse, replay lookup, the rest -------------------

fn push_bytes(header: &ClearHeader) -> Vec<u8> {
    encode_container(header, &[0x5a; 40]).unwrap()
}

#[test]
fn the_service_parses_first_and_refuses_what_the_lookup_cannot_use() {
    let dev = Dev::new(0x11);
    let (header, _) = parts(&dev, 1, 1);
    let parsed = parse_push(&push_bytes(&header), &VAULT).expect("a well-formed push parses");
    assert_eq!(parsed.identity.command_id(), &[1; 16]);
    assert_eq!(parsed.header, header);

    let mut wrong_vault = header.clone();
    wrong_vault.vault_id[0] ^= 1;
    assert_eq!(
        parse_push(&push_bytes(&wrong_vault), &VAULT),
        Err(PushRefusal::WrongVault)
    );
    let mut snapshot = header.clone();
    snapshot.object = ObjectKind::Snapshot {
        artifact_manifest_commitment: ManifestCommitment::from_derived([1; 32]),
    };
    assert_eq!(
        parse_push(&push_bytes(&snapshot), &VAULT),
        Err(PushRefusal::NotAnEnvelope)
    );
    let bytes = push_bytes(&header);
    assert!(matches!(
        parse_push(&bytes[..bytes.len() - 1], &VAULT),
        Err(PushRefusal::Malformed(_))
    ));
    assert_eq!(
        parse_push(&vec![0; MAX_ENVELOPE_CONTAINER_LEN + 1], &VAULT),
        Err(PushRefusal::Malformed(WireError::ObjectTooLarge))
    );
}

#[test]
fn for_an_unknown_identity_the_service_then_checks_member_nonce_domain_and_epoch() {
    let dev = Dev::new(0x11);
    let chain = chain_of(&dev);
    let view = PushView {
        routed_vault: VAULT,
        chain: &chain,
        epoch_in_force: 1,
    };
    let (header, _) = parts(&dev, 1, 1);
    assert_eq!(check_new_push(&header, &view), Ok(()));

    // Right id, wrong fingerprint; wrong id, right fingerprint: not one authorized member.
    let mut wrong_fingerprint = header.clone();
    wrong_fingerprint.device_key_fingerprint = Fingerprint::from_derived([9; 32]);
    wrong_fingerprint.nonce_domain =
        NonceDomain::derive(&VAULT, 1, &wrong_fingerprint.device_key_fingerprint, 1);
    assert_eq!(
        check_new_push(&wrong_fingerprint, &view),
        Err(PushRefusal::UnknownDevice)
    );
    let mut wrong_id = header.clone();
    wrong_id.device_id[15] ^= 1;
    assert_eq!(
        check_new_push(&wrong_id, &view),
        Err(PushRefusal::UnknownDevice)
    );
    // A nonce domain that is not the derived one.
    let mut wrong_domain = header.clone();
    wrong_domain.nonce_domain = NonceDomain::from_derived([0; 4]);
    assert_eq!(
        check_new_push(&wrong_domain, &view),
        Err(PushRefusal::NonceDomainMismatch)
    );
    // An epoch that is not in force.
    let (old_epoch, _) = parts(&dev, 1, 7);
    assert_eq!(
        check_new_push(&old_epoch, &view),
        Err(PushRefusal::EpochRejected)
    );
}

#[test]
fn an_exact_retry_from_a_device_that_is_no_longer_a_member_is_still_a_replay() {
    // C.10: an accepted envelope returns its original seq even when its device has since been
    // revoked or replaced, and even with an old epoch. The chain here holds only device 0x11; the
    // envelope was sealed by device 0x22, which is not (or no longer) a member.
    let current = Dev::new(0x11);
    let former = Dev::new(0x22);
    let chain = chain_of(&current);
    let view = PushView {
        routed_vault: VAULT,
        chain: &chain,
        epoch_in_force: 5,
    };
    let (header, _) = parts(&former, 3, 1); // an old epoch too
    let bytes = push_bytes(&header);

    let parsed = parse_push(&bytes, &VAULT).unwrap();
    let mut index = InMemoryIndex::new();
    index
        .insert(parsed.identity.accepted(1, 41, Some(vec![9])))
        .unwrap();

    // The replay lookup happens right after the parse, so the retry is answered with its seq.
    match decide_push(&index, &parse_push(&bytes, &VAULT).unwrap().identity).unwrap() {
        PushDecision::Replay(record) => assert_eq!(record.seq, 41),
        other => panic!("expected a replay, got {other:?}"),
    }
    // The member, nonce-domain and epoch checks are for NEW identities only, and would refuse it.
    assert_eq!(
        check_new_push(&header, &view),
        Err(PushRefusal::UnknownDevice)
    );
}

// ---- the replica's sync status ----------------------------------------------------------

#[test]
fn a_replica_stops_at_an_invalid_envelope_applies_nothing_past_it_and_cannot_push() {
    let dev = Dev::new(0x11);
    let (header, _) = parts(&dev, 1, 1);
    let mut status = SyncStatus::Healthy;
    assert!(status.is_healthy() && status.can_apply(1_000_000) && status.can_push());

    let event = status.observe_envelope(7, [7; 32], &header, Err(ReceiverError::TagMismatch));
    let Some(SyncEvent::EnvelopeInvalid(invalid)) = event else {
        panic!("expected an EnvelopeInvalid event, got {event:?}");
    };
    assert_eq!(invalid.seq, 7);
    assert_eq!(invalid.command_id, header.object_id);
    assert_eq!(invalid.device_id, header.device_id);
    assert_eq!(
        invalid.device_key_fingerprint,
        header.device_key_fingerprint
    );
    assert_eq!(invalid.reason, ReceiverError::TagMismatch);
    assert!(
        status.can_apply(6),
        "positions before the invalid one still apply"
    );
    assert!(!status.can_apply(7) && !status.can_apply(8) && !status.can_apply(u64::MAX));
    assert!(!status.can_push());
    assert!(!status.is_healthy());
}

#[test]
fn seeing_the_same_invalid_envelope_again_is_idempotent_and_the_earlier_position_wins() {
    let dev = Dev::new(0x11);
    let (header, _) = parts(&dev, 1, 1);
    let mut status = SyncStatus::Healthy;
    assert!(status
        .observe_envelope(9, [9; 32], &header, Err(ReceiverError::TagMismatch))
        .is_some());
    // After a restart the device re-pulls position 9 and recomputes the same digest: no new event.
    assert_eq!(
        status.observe_envelope(9, [9; 32], &header, Err(ReceiverError::TagMismatch)),
        None
    );
    // A valid envelope never changes the status.
    assert_eq!(status.observe_envelope(3, [3; 32], &header, Ok(())), None);
    // An invalid one at an EARLIER position (verified out of order) moves the stop earlier.
    assert!(status
        .observe_envelope(5, [5; 32], &header, Err(ReceiverError::DeviceMismatch))
        .is_some());
    assert!(status.can_apply(4) && !status.can_apply(5) && !status.can_apply(9));
    // A later one does not.
    assert_eq!(
        status.observe_envelope(8, [8; 32], &header, Err(ReceiverError::TagMismatch)),
        None
    );
}

#[test]
fn a_different_digest_at_a_recorded_position_is_a_fork_of_the_log_and_halts() {
    let dev = Dev::new(0x11);
    let (header, _) = parts(&dev, 1, 1);
    let mut status = SyncStatus::Healthy;
    status.observe_envelope(9, [9; 32], &header, Err(ReceiverError::TagMismatch));
    let event = status.observe_envelope(9, [0xaa; 32], &header, Ok(()));
    assert_eq!(
        event,
        Some(SyncEvent::Halted(HaltEvidence::SeqConflict {
            seq: 9,
            first: [9; 32],
            second: [0xaa; 32]
        }))
    );
    assert!(!status.can_apply(0), "a halted replica applies nothing");
    // A halted replica stays halted and emits nothing more.
    assert_eq!(
        status.observe_envelope(2, [2; 32], &header, Err(ReceiverError::TagMismatch)),
        None
    );
}

#[test]
fn only_the_halt_class_of_chain_errors_changes_the_status() {
    use sync_core::membership::ChainError;
    let mut status = SyncStatus::Healthy;
    // Retryable and ordinary errors do not halt.
    assert_eq!(
        status.observe_chain_error(&ChainError::EpochRecordMissing { version: 3 }),
        None
    );
    assert_eq!(
        status.observe_chain_error(&ChainError::BadSignature { at: 1 }),
        None
    );
    assert!(status.is_healthy());
    // A fork does, with both ids kept.
    let event = status.observe_chain_error(&ChainError::Fork {
        version: 2,
        accepted: [1; 32],
        conflicting: [2; 32],
    });
    assert_eq!(
        event,
        Some(SyncEvent::Halted(HaltEvidence::Fork {
            version: 2,
            accepted: [1; 32],
            conflicting: [2; 32]
        }))
    );
    assert!(!status.can_apply(0) && !status.can_push());
}

#[test]
fn a_fork_found_while_stopped_at_an_invalid_envelope_halts() {
    let dev = Dev::new(0x11);
    let (header, _) = parts(&dev, 1, 1);
    let mut status = SyncStatus::Healthy;
    status.observe_envelope(7, [7; 32], &header, Err(ReceiverError::TagMismatch));
    assert!(matches!(status, SyncStatus::EnvelopeInvalid(_)));
    // The halt class takes precedence: keys must be quarantined and exposure flagged.
    let event = status.observe_chain_error(&sync_core::membership::ChainError::EpochEquivocation {
        accepted: [1; 32],
        conflicting: [2; 32],
    });
    assert_eq!(
        event,
        Some(SyncEvent::Halted(HaltEvidence::Equivocation {
            accepted: [1; 32],
            conflicting: [2; 32]
        }))
    );
    assert!(matches!(status, SyncStatus::Halted(_)));
    // Already halted: a second one is absorbed.
    assert_eq!(
        status.observe_chain_error(&sync_core::membership::ChainError::Fork {
            version: 2,
            accepted: [3; 32],
            conflicting: [4; 32]
        }),
        None
    );
}

#[test]
fn an_approved_envelope_prints_its_size_and_digest_not_its_bytes() {
    let dev = Dev::new(0x11);
    let (header, plaintext) = parts(&dev, 1, 1);
    let sealed = seal(&header, &plaintext);
    let approved =
        verify_before_upload(&sealed.bytes, &TestOpener(Some(sealed.plaintext)), test_tag).unwrap();
    let shown = format!("{approved:?}");
    assert!(shown.contains("len") && shown.contains("digest"));
    assert!(shown.len() < 400, "{} bytes of Debug output", shown.len());
}
