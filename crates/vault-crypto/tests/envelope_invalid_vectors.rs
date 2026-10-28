//! Public pipeline-level conformance vectors for "found envelope N invalid" (vault-format.md C.4
//! "When an envelope is found invalid", C.14; personal-cfo-g3m.25).
//!
//! The void rules (ADR 0074 Addendum D) honor a void only for an envelope the replica itself found
//! invalid. That verdict comes from the whole pipeline (parse, epoch position, key, device window,
//! AEAD open, plaintext decode, `check_envelope`), and it must be the same on every replica, or a
//! void that some honor and others refuse would split the vault. The verdict is **invalid** only
//! with the evidence, **not determinable** while the replica's key or chain is missing (never a
//! basis to honor a void), and otherwise valid.
//!
//! `docs/architecture/vectors/envelope-invalid-v1.json` pins it: sealed envelope containers built
//! from synthetic keys over a sample membership chain, each with the state the replica is in (the
//! chain, which epoch keys it holds, the epoch in force at the position) and the expected verdict.
//! Epoch keys are **not in the file**: they derive from a labeled formula described in the file, so
//! no key-shaped literal is committed. A deterministic generator fails the test if the committed
//! file drifts; the verifier test classifies every case with `classify_envelope`; the stage test
//! reads the verdict reasons from the source so a new stage without a vector fails.
//!
//! Regenerate with `DOHFLOW_UPDATE_VECTORS=1 cargo test -p vault-crypto --test
//! envelope_invalid_vectors the_committed` (a spec change under C.13).

use ed25519_dalek::{Signer, SigningKey};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sync_core::envelope::{ArtifactManifest, EnvelopePlaintext, IdempotencyKey, ImmutableMeta};
use sync_core::header::{
    encode_container, parse_container, ClearHeader, Fingerprint, IdempotencyKeyTag, NonceDomain,
    ObjectKind, OBJECT_ENVELOPE,
};
use sync_core::membership::{
    device_fingerprint, verify_chain, Action, EpochAtPosition, EpochBody, MembershipBody, Reason,
    Recipient, SignedRecord, VerifiedState, KEM_ID, SEALED_KEYS_CIPHERTEXT_LEN, SUITE_ID,
};
use vault_crypto::sync::{idempotency_key_tag, idempotency_tag_key};
use vault_crypto::sync_envelope::{
    classify_envelope, seal_envelope, EpochKey, InvalidReason, OpenFailure, SealerIdentity,
    Verdict, Wait,
};
use vault_crypto::SecretBytes;

const VECTORS: &str = include_str!("../../../docs/architecture/vectors/envelope-invalid-v1.json");
const VECTORS_PATH: &str = "../../docs/architecture/vectors/envelope-invalid-v1.json";
const SOURCE: &str = include_str!("../src/sync_envelope.rs");
const VAULT: [u8; 16] = [0x11; 16];

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len() / 2)
        .map(|i| u8::from_str_radix(&text[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}

/// The epoch key of the vectors: `SHA-256(label "dohflow/test-vector-epoch-key/v1" ‖ u64 epoch)`.
fn epoch_key_bytes(epoch: u64) -> [u8; 32] {
    let label = b"dohflow/test-vector-epoch-key/v1";
    let mut input = (label.len() as u16).to_be_bytes().to_vec();
    input.extend_from_slice(label);
    input.extend_from_slice(&epoch.to_be_bytes());
    Sha256::digest(&input).into()
}

fn commitment(key: &[u8; 32]) -> [u8; 32] {
    let label = b"dohflow/sync-epoch-key-commit/v1";
    let mut input = (label.len() as u16).to_be_bytes().to_vec();
    input.extend_from_slice(label);
    input.extend_from_slice(key);
    Sha256::digest(&input).into()
}

fn epoch_key(epoch: u64) -> EpochKey {
    let bytes = epoch_key_bytes(epoch);
    EpochKey::from_verified(VAULT, epoch, SecretBytes::new(bytes), &commitment(&bytes))
        .expect("the commitment matches")
}

// ---- a sample chain: A adds B in epoch 1, then revokes B in a cutover into epoch 2 ----------

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

#[allow(clippy::too_many_arguments)]
fn epoch(
    sealer: &Dev,
    key_epoch: u64,
    reason: Reason,
    head: [u8; 32],
    prev: Option<[u8; 32]>,
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
        key_commitment_ke: commitment(&epoch_key_bytes(key_epoch)),
        key_commitment_s: [2; 32],
        sealer_fingerprint: sealer.fingerprint(),
        recipients,
    }
    .encode()
    .unwrap();
    let id = Sha256::digest(&body).into();
    (sign(&body, &sealer.signing), id)
}

/// The records of the sample chain, in order: (memberships, epochs, epoch record ids).
/// The sample chain's records and the ids of its epoch records.
type ChainRecords = (Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<[u8; 32]>);

fn chain_records(a: &Dev, b: &Dev) -> ChainRecords {
    let (m1, id_m1) = membership(1, None, Action::Add, a, a);
    let (e1, id_e1) = epoch(a, 1, Reason::Genesis, id_m1, None, &[a]);
    let (m2, id_m2) = membership(2, Some(id_m1), Action::Add, a, b);
    let (e2, id_e2) = epoch(a, 1, Reason::Grant, id_m2, Some(id_e1), &[b]);
    let (m3, id_m3) = membership(3, Some(id_m2), Action::Revoke, a, b);
    let (e3, id_e3) = epoch(a, 2, Reason::Cutover, id_m3, Some(id_e2), &[a]);
    (
        vec![m1, m2, m3],
        vec![e1, e2, e3],
        vec![id_e1, id_e2, id_e3],
    )
}

fn verified(memberships: &[Vec<u8>], epochs: &[Vec<u8>]) -> VerifiedState {
    let m: Vec<&[u8]> = memberships.iter().map(Vec::as_slice).collect();
    let e: Vec<&[u8]> = epochs.iter().map(Vec::as_slice).collect();
    verify_chain(&VAULT, &m, &e).expect("the sample chain is valid")
}

// ---- the replica's tagger and the envelopes ---------------------------------------------

/// The tagger the vectors use: vault-crypto's own, under the C.14 sample tag secret `S` (the 32
/// bytes 0x20 .. 0x3f). The file carries its answer for each case's plaintext key.
fn tagger(key: &IdempotencyKey) -> [u8; 32] {
    let s: [u8; 32] = std::array::from_fn(|i| 0x20 + i as u8);
    idempotency_key_tag(
        &idempotency_tag_key(&SecretBytes::new(s), &VAULT),
        key.as_bytes(),
    )
}

fn sample_key(command: u8) -> IdempotencyKey {
    let mut key = [command; 16];
    key[6] = 0x40 | (key[6] & 0x0f);
    key[8] = 0x80 | (key[8] & 0x3f);
    IdempotencyKey::from_bytes(key).unwrap()
}

fn parts(dev: &Dev, command: u8, key_epoch: u64) -> (ClearHeader, EnvelopePlaintext) {
    let meta = ImmutableMeta {
        command_id: [command; 16],
        correlation_id: [0x22; 16],
        causation_id: None,
        actor_type: 1,
        actor_id: "tester".to_owned(),
        idempotency_key: sample_key(command),
        issued_at: 1_790_000_000_000_000,
        device_id: dev.device_id,
    };
    let fingerprint = dev.fingerprint();
    let header = ClearHeader {
        vault_id: VAULT,
        key_epoch,
        object: ObjectKind::Envelope {
            idempotency_key_tag: IdempotencyKeyTag::from_derived(tagger(&meta.idempotency_key)),
            immutable_metadata_commitment: meta.commitment().unwrap(),
            artifact_manifest_commitment: None,
        },
        object_id: meta.command_id,
        device_id: dev.device_id,
        device_key_fingerprint: fingerprint,
        nonce_domain: NonceDomain::derive(&VAULT, key_epoch, &fingerprint, OBJECT_ENVELOPE),
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

fn seal(dev: &Dev, header: &ClearHeader, plaintext: &[u8]) -> Vec<u8> {
    seal_envelope(
        &epoch_key(header.key_epoch),
        &dev.identity(),
        header,
        plaintext,
    )
    .unwrap()
}

fn sealed(dev: &Dev, command: u8, key_epoch: u64) -> Vec<u8> {
    let (header, plaintext) = parts(dev, command, key_epoch);
    seal(dev, &header, &plaintext.encode().unwrap())
}

// ---- the cases ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum ChainKind {
    /// The whole sample chain (epochs 1 and 2).
    Full,
    /// Only epoch 1 (the replica is behind).
    Epoch1Only,
}

struct Case {
    name: &'static str,
    description: &'static str,
    chain: ChainKind,
    held_keys: Vec<u64>,
    /// Index (0..3) of the newest epoch record positioned before the envelope, or `None` when the
    /// replica has not processed the log in order up to this position.
    newest_epoch_before: Option<usize>,
    container: Vec<u8>,
    /// The tagger's answer for the plaintext's key.
    tagger_output: [u8; 32],
    verdict: &'static str,
    reason: &'static str,
}

fn all_cases() -> Vec<Case> {
    let a = Dev::new(0xa1);
    let b = Dev::new(0xb4);
    let case = |name,
                description,
                container: Vec<u8>,
                held: &[u64],
                in_force: Option<usize>,
                verdict,
                reason| Case {
        name,
        description,
        chain: ChainKind::Full,
        held_keys: held.to_vec(),
        newest_epoch_before: in_force,
        container,
        tagger_output: tagger(&sample_key(1)),
        verdict,
        reason,
    };
    let valid_a = sealed(&a, 1, 1);
    // Epoch records: 0 = genesis (epoch 1, membership v1), 1 = grant (epoch 1, v2),
    // 2 = cutover (epoch 2, v3).
    let (e_genesis, e_grant, e_cutover) = (Some(0), Some(1), Some(2));
    let mut cases = Vec::new();

    cases.push(case("valid_member_envelope", "a member's coherent envelope in the epoch in force at its position, with that epoch's key held", valid_a.clone(), &[1], e_grant, "valid", ""));
    cases.push(case("valid_envelope_of_a_device_revoked_later", "B sealed in epoch 1 and is revoked at the cutover, which is positioned AFTER this envelope: still a member at its position", sealed(&b, 1, 1), &[1], e_grant, "valid", ""));
    let mut revoked_later_short = case("valid_envelope_of_a_device_revoked_later_on_a_shorter_chain", "the same, for a replica that has verified only up to the grant: the verdict does not depend on how far it has verified", sealed(&b, 1, 1), &[1], e_grant, "valid", "");
    revoked_later_short.chain = ChainKind::Epoch1Only;
    cases.push(revoked_later_short);
    cases.push(case(
        "valid_extra_keys_for_other_epochs_change_nothing",
        "the replica also holds epoch 2's key; the verdict is the same",
        valid_a.clone(),
        &[1, 2],
        e_grant,
        "valid",
        "",
    ));

    // ---- invalid: the bytes and the key prove it ----
    let flip = |bytes: &[u8], from_end: usize| {
        let mut out = bytes.to_vec();
        let i = out.len() - from_end;
        out[i] ^= 1;
        out
    };
    cases.push(case(
        "invalid_flipped_ciphertext_byte",
        "one ciphertext byte flipped",
        flip(&valid_a, 20),
        &[1],
        e_grant,
        "invalid",
        "Open:NotOpenable",
    ));
    cases.push(case(
        "invalid_flipped_tag_byte",
        "the last byte, part of the GCM tag, flipped",
        flip(&valid_a, 1),
        &[1],
        e_grant,
        "invalid",
        "Open:NotOpenable",
    ));
    let truncated_ciphertext = {
        let parsed = parse_container(&valid_a).unwrap();
        encode_container(
            &parsed.header,
            &parsed.ciphertext[..parsed.ciphertext.len() - 1],
        )
        .unwrap()
    };
    cases.push(case(
        "invalid_truncated_ciphertext",
        "the ciphertext is one byte short (the container still parses)",
        truncated_ciphertext,
        &[1],
        e_grant,
        "invalid",
        "Open:NotOpenable",
    ));
    let plaintext_garbage = {
        let (header, _) = parts(&a, 1, 1);
        seal(&a, &header, b"not an envelope plaintext")
    };
    cases.push(case(
        "invalid_plaintext_does_not_decode",
        "a good open of bytes that are not an envelope plaintext",
        plaintext_garbage,
        &[1],
        e_grant,
        "invalid",
        "PlaintextMalformed",
    ));
    let wrong_tag = {
        let (mut header, plaintext) = parts(&a, 1, 1);
        if let ObjectKind::Envelope {
            idempotency_key_tag,
            ..
        } = &mut header.object
        {
            *idempotency_key_tag = IdempotencyKeyTag::from_derived([0; 32]);
        }
        seal(&a, &header, &plaintext.encode().unwrap())
    };
    cases.push(case(
        "invalid_receiver_check_rejects",
        "a coherent seal of a header whose idempotency-key tag is not the plaintext key's tag",
        wrong_tag,
        &[1],
        e_grant,
        "invalid",
        "Check:TagMismatch",
    ));
    let snapshot_header = {
        let parsed = parse_container(&valid_a).unwrap();
        let mut header = parsed.header.clone();
        header.object = ObjectKind::Snapshot {
            artifact_manifest_commitment: sync_core::header::ManifestCommitment::from_derived(
                [1; 32],
            ),
        };
        encode_container(&header, parsed.ciphertext).unwrap()
    };
    cases.push(case(
        "invalid_snapshot_header_at_an_envelope_position",
        "a snapshot header where an envelope is required",
        snapshot_header,
        &[1],
        e_grant,
        "invalid",
        "Open:NotAnEnvelope",
    ));
    let wrong_domain = {
        let parsed = parse_container(&valid_a).unwrap();
        let mut header = parsed.header.clone();
        header.nonce_domain = NonceDomain::from_derived([9, 9, 9, 9]);
        encode_container(&header, parsed.ciphertext).unwrap()
    };
    cases.push(case(
        "invalid_nonce_domain_is_not_the_derived_one",
        "the header's nonce domain differs from the derived one",
        wrong_domain,
        &[1],
        e_grant,
        "invalid",
        "Open:NonceDomain",
    ));

    // ---- invalid: the position and the chain prove it ----
    cases.push(case(
        "invalid_device_revoked_before_the_position",
        "B sealing in epoch 2, positioned after the cutover that revoked it",
        sealed(&b, 2, 2),
        &[2],
        e_cutover,
        "invalid",
        "Open:DeviceOutsideEpoch",
    ));
    cases.push(case("invalid_device_granted_after_the_position", "B sealing in epoch 1 at a position before the grant that adds it (the genesis is the newest epoch record before it): not a member at the position", sealed(&b, 5, 1), &[1], e_genesis, "invalid", "Open:DeviceOutsideEpoch"));
    let mut granted_after_short = case("invalid_device_granted_after_the_position_on_a_shorter_chain", "the same for a replica that has verified only up to the grant; the verdict is identical on both chain lengths", sealed(&b, 5, 1), &[1], e_genesis, "invalid", "Open:DeviceOutsideEpoch");
    granted_after_short.chain = ChainKind::Epoch1Only;
    cases.push(granted_after_short);
    let mismatched_identity = {
        let original = sealed(&b, 3, 1);
        let parsed = parse_container(&original).unwrap();
        let mut header = parsed.header.clone();
        header.device_id = a.device_id; // A's id with B's fingerprint
        encode_container(&header, parsed.ciphertext).unwrap()
    };
    cases.push(case(
        "invalid_header_restates_another_identity",
        "A's device id with B's fingerprint",
        mismatched_identity,
        &[1],
        e_grant,
        "invalid",
        "Open:IdentityMismatch",
    ));
    cases.push(case(
        "invalid_past_epoch_at_the_position",
        "an epoch-1 envelope at a position where epoch 2 is in force; the replica holds both keys",
        valid_a.clone(),
        &[1, 2],
        e_cutover,
        "invalid",
        "EpochNotInForce",
    ));
    cases.push(case(
        "invalid_past_epoch_needs_no_key",
        "the same, with no key held: the position alone proves it",
        valid_a.clone(),
        &[],
        e_cutover,
        "invalid",
        "EpochNotInForce",
    ));
    cases.push(case("invalid_future_epoch_at_the_position", "an epoch-2 envelope at a position where epoch 1 is in force, although epoch 2 exists later in the chain", sealed(&a, 4, 2), &[2], e_grant, "invalid", "EpochNotInForce"));
    let mut future_unknown = case("invalid_future_epoch_unknown_to_the_chain", "the same for a replica whose chain stops in epoch 1: the position is certain, so the verdict is the same", sealed(&a, 4, 2), &[2], e_grant, "invalid", "EpochNotInForce");
    future_unknown.chain = ChainKind::Epoch1Only;
    cases.push(future_unknown);

    // ---- invalid: a header that names another vault ----
    let other_vault = {
        let parsed = parse_container(&valid_a).unwrap();
        let mut header = parsed.header.clone();
        header.vault_id[0] ^= 1;
        encode_container(&header, parsed.ciphertext).unwrap()
    };
    cases.push(case(
        "invalid_header_names_another_vault",
        "the header's vault id is not the replica's own, with the epoch-1 key held",
        other_vault.clone(),
        &[1],
        e_grant,
        "invalid",
        "WrongVault",
    ));
    cases.push(case("invalid_header_names_another_vault_with_nothing_else_known", "the same with no keys held and the position unknown: the vault is known from the chain, so it is still invalid", other_vault, &[], None, "invalid", "WrongVault"));

    // ---- invalid: the container does not parse, with nothing else known ----
    cases.push(case(
        "invalid_unparseable_container_with_nothing_known",
        "a truncated container, no keys held and the position unknown: still invalid",
        valid_a[..valid_a.len() / 2].to_vec(),
        &[],
        None,
        "invalid",
        "Unparseable",
    ));
    let mut bad_magic = valid_a.clone();
    bad_magic[0] ^= 1;
    cases.push(case(
        "invalid_unparseable_bad_magic",
        "the container magic is wrong",
        bad_magic,
        &[1],
        e_grant,
        "invalid",
        "Unparseable",
    ));

    // ---- not determinable: never a basis to honor a void ----
    cases.push(case(
        "not_determinable_no_verified_key",
        "a good envelope but the replica holds no verified key for epoch 1",
        valid_a.clone(),
        &[],
        e_grant,
        "not_determinable",
        "NoVerifiedKey",
    ));
    cases.push(case(
        "not_determinable_key_of_another_epoch_only",
        "the replica holds epoch 2's key only; the envelope is epoch 1 and epoch 1 is in force",
        valid_a.clone(),
        &[2],
        e_grant,
        "not_determinable",
        "NoVerifiedKey",
    ));
    cases.push(case(
        "not_determinable_flipped_byte_without_the_key",
        "a flipped ciphertext byte, but without the key the replica cannot tell: NOT invalid",
        flip(&valid_a, 20),
        &[],
        e_grant,
        "not_determinable",
        "NoVerifiedKey",
    ));
    cases.push(case(
        "not_determinable_position_unknown",
        "the replica has not processed the log in order up to this position",
        valid_a.clone(),
        &[1],
        None,
        "not_determinable",
        "ChainBehind",
    ));
    cases.push(case(
        "not_determinable_bad_envelope_while_the_position_is_unknown",
        "a flipped byte while the position is unknown: NOT invalid yet",
        flip(&valid_a, 20),
        &[1],
        None,
        "not_determinable",
        "ChainBehind",
    ));
    let mut behind = case("not_determinable_epoch_record_before_the_position_not_verified", "the newest epoch record before the position is the cutover, but the replica's verified chain stops at the grant: it cannot build the position", sealed(&a, 4, 2), &[2], e_cutover, "not_determinable", "ChainBehind");
    behind.chain = ChainKind::Epoch1Only;
    cases.push(behind);
    cases
}

fn chain_for(kind: ChainKind) -> VerifiedState {
    let (a, b) = (Dev::new(0xa1), Dev::new(0xb4));
    let (memberships, epochs, _) = chain_records(&a, &b);
    match kind {
        ChainKind::Full => verified(&memberships, &epochs),
        // Memberships 1 and 2 with epochs 1 and 2 (genesis, grant): epoch 1 only.
        ChainKind::Epoch1Only => verified(&memberships[..2], &epochs[..2]),
    }
}

fn describe(verdict: Verdict) -> (&'static str, String) {
    match verdict {
        Verdict::Valid => ("valid", String::new()),
        Verdict::Invalid(reason) => (
            "invalid",
            match reason {
                InvalidReason::Unparseable => "Unparseable".to_owned(),
                InvalidReason::EpochNotInForce => "EpochNotInForce".to_owned(),
                InvalidReason::Open(failure) => format!("Open:{failure:?}"),
                InvalidReason::PlaintextMalformed => "PlaintextMalformed".to_owned(),
                InvalidReason::Check(error) => format!("Check:{error:?}"),
                other => format!("{other:?}"),
            },
        ),
        Verdict::NotDeterminable(wait) => ("not_determinable", format!("{wait:?}")),
    }
}

fn epoch_ids() -> Vec<[u8; 32]> {
    let (a, b) = (Dev::new(0xa1), Dev::new(0xb4));
    chain_records(&a, &b).2
}

fn classify(
    case_chain: ChainKind,
    held: &[u64],
    newest: Option<usize>,
    container: &[u8],
    tag: [u8; 32],
) -> Verdict {
    let state = chain_for(case_chain);
    let keys: Vec<EpochKey> = held.iter().map(|e| epoch_key(*e)).collect();
    // The position exists only if the replica's verified chain contains that epoch record.
    let position =
        newest.and_then(|i| EpochAtPosition::after_epoch_record(&state, &epoch_ids()[i]));
    classify_envelope(container, &keys, &state, position.as_ref(), |_| {
        IdempotencyKeyTag::from_derived(tag)
    })
}

fn document() -> Value {
    let cases: Vec<Value> = all_cases()
        .iter()
        .map(|c| {
            // The intended verdict must be what the code says, or the vector is wrong.
            let (verdict, reason) = describe(classify(c.chain, &c.held_keys, c.newest_epoch_before, &c.container, c.tagger_output));
            assert_eq!((verdict, reason.as_str()), (c.verdict, c.reason), "{}: intended verdict", c.name);
            json!({
                "name": c.name,
                "description": c.description,
                "chain": match c.chain { ChainKind::Full => "full", ChainKind::Epoch1Only => "epoch1_only" },
                "held_epoch_keys": c.held_keys,
                "newest_epoch_record_before_position": c.newest_epoch_before.map(|i| hex(&epoch_ids()[i])),
                "container": hex(&c.container),
                "tagger_output": hex(&c.tagger_output),
                "expect": { "verdict": c.verdict, "reason": c.reason },
            })
        })
        .collect();
    let (a, b) = (Dev::new(0xa1), Dev::new(0xb4));
    let (memberships, epochs, _) = chain_records(&a, &b);
    let hex_all = |records: &[Vec<u8>]| records.iter().map(|r| hex(r)).collect::<Vec<_>>();
    json!({
        "vectors_version": 1,
        "spec": "docs/architecture/vault-format.md C.4 \"When an envelope is found invalid\" and C.14",
        "synthetic": "Every key, id and timestamp here is synthetic and test-only. Private signing seeds and epoch keys are not in this file.",
        "vault_id": hex(&VAULT),
        "epoch_keys": "The replica's epoch key for epoch e is SHA-256 of the CBE1 label 'dohflow/test-vector-epoch-key/v1' followed by the 8-byte big-endian epoch e. A replica 'holds a verified key' for e when that key matches the epoch record's key_commitment_ke (C.3); 'held_epoch_keys' lists the epochs it holds.",
        "tag_secret": "The tagger is the idempotency-key tagger of C.3 under the C.14 sample tag secret S (the 32 bytes 0x20 to 0x3f). 'tagger_output' is its answer for the plaintext's canonical key.",
        "chain": {
            "full": { "memberships": hex_all(&memberships), "epochs": hex_all(&epochs), "note": "Device A (founder) adds device B in epoch 1 (a grant), then revokes B in a cutover into epoch 2." },
            "epoch1_only": { "memberships": hex_all(&memberships[..2]), "epochs": hex_all(&epochs[..2]), "note": "The same chain truncated after the grant: the replica's verified chain stops in epoch 1." },
        },
        "devices": [
            { "name": "A", "device_id": hex(&a.device_id), "fingerprint": hex(a.fingerprint().as_bytes()) },
            { "name": "B", "device_id": hex(&b.device_id), "fingerprint": hex(b.fingerprint().as_bytes()) },
        ],
        "how_to_read": "Each case is the state of one replica at one log position and the envelope container found there. 'newest_epoch_record_before_position' is the id of the newest epoch record positioned before the envelope (C.7), as the replica sees it processing the log in order, or null when it has not processed the log up to this position; the epoch in force and the membership version in force at the position are that record's key_epoch and the version of the membership record it names, and exist only if the replica's verified chain contains the record. The verdict is 'valid', 'invalid' or 'not_determinable'. The verdict class is the contract; 'reason' is informative. 'invalid' needs the evidence (C.4); 'not_determinable' means the replica must wait and never honors a void on that basis.",
        "stage_order": ["Unparseable", "ChainBehind (position unknown)", "EpochNotInForce", "NoVerifiedKey", "Open (NotAnEnvelope, DeviceOutsideEpoch, IdentityMismatch, NonceDomain, NotOpenable)", "PlaintextMalformed", "Check"],
        "unreachable_from_classification": [
            { "variant": "WrongVault", "why": "Raised when the header names another vault than the key's; the key is selected by the header's vault and epoch, so a header of another vault finds no key and is NoVerifiedKey." },
            { "variant": "WrongEpoch", "why": "Raised when the header names another epoch than the key's; the key is selected by the header's epoch." }
        ],
        "cases": cases,
    })
}

fn pretty(value: &Value) -> String {
    let mut text = serde_json::to_string_pretty(value).unwrap();
    text.push('\n');
    text
}

#[test]
fn the_committed_vectors_are_what_the_generator_produces() {
    let generated = pretty(&document());
    if std::env::var_os("DOHFLOW_UPDATE_VECTORS").is_some() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(VECTORS_PATH);
        std::fs::write(&path, &generated).unwrap();
        return;
    }
    assert_eq!(
        VECTORS, generated,
        "the vectors file drifted from the generator; regenerate with DOHFLOW_UPDATE_VECTORS=1 \
         (a spec change under C.13)"
    );
}

#[test]
fn classify_envelope_agrees_with_every_committed_case() {
    let doc: Value = serde_json::from_str(VECTORS).unwrap();
    assert_eq!(doc["vectors_version"], 1);
    let cases = doc["cases"].as_array().unwrap();
    assert!(cases.len() >= 20, "{} cases", cases.len());
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let kind = match case["chain"].as_str().unwrap() {
            "full" => ChainKind::Full,
            _ => ChainKind::Epoch1Only,
        };
        let held: Vec<u64> = case["held_epoch_keys"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e.as_u64().unwrap())
            .collect();
        let newest = case["newest_epoch_record_before_position"]
            .as_str()
            .map(|id| {
                let id = unhex(id);
                epoch_ids()
                    .iter()
                    .position(|e| e.as_slice() == id.as_slice())
                    .unwrap()
            });
        let tag: [u8; 32] = unhex(case["tagger_output"].as_str().unwrap())
            .try_into()
            .unwrap();
        let container = unhex(case["container"].as_str().unwrap());
        let (verdict, reason) = describe(classify(kind, &held, newest, &container, tag));
        assert_eq!(
            verdict,
            case["expect"]["verdict"].as_str().unwrap(),
            "{name}"
        );
        assert_eq!(reason, case["expect"]["reason"].as_str().unwrap(), "{name}");
    }
}

#[test]
fn a_bad_envelope_is_invalid_only_when_the_replica_can_judge_it() {
    // The same flipped byte is invalid with the key and a known position, and merely not
    // determinable without them: the core of the "never honor a void on a missing key" rule.
    let doc: Value = serde_json::from_str(VECTORS).unwrap();
    let verdict_of = |name: &str| -> String {
        let case = doc["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .unwrap();
        case["expect"]["verdict"].as_str().unwrap().to_owned()
    };
    assert_eq!(verdict_of("invalid_flipped_ciphertext_byte"), "invalid");
    assert_eq!(
        verdict_of("not_determinable_flipped_byte_without_the_key"),
        "not_determinable"
    );
    assert_eq!(
        verdict_of("not_determinable_bad_envelope_while_the_position_is_unknown"),
        "not_determinable"
    );
    // Every not-determinable case is never "invalid".
    for case in doc["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        if name.starts_with("not_determinable_") {
            assert_eq!(case["expect"]["verdict"], "not_determinable", "{name}");
        }
    }
}

#[test]
fn every_verdict_reason_has_a_vector_or_a_stated_reason() {
    // Reasons are read from the source, so a new stage or reason cannot be added without this
    // failing until it has a vector (or an explicit, reasoned entry in the file).
    let doc: Value = serde_json::from_str(VECTORS).unwrap();
    let covered: Vec<String> = doc["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["expect"]["reason"].as_str().unwrap().to_owned())
        .collect();
    let excused: Vec<&str> = doc["unreachable_from_classification"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["variant"].as_str().unwrap())
        .collect();
    let variants_of = |enum_name: &str| -> Vec<&'static str> {
        let start = SOURCE
            .find(&format!("pub enum {enum_name} {{"))
            .unwrap_or_else(|| panic!("no enum {enum_name}"));
        let body = &SOURCE[start..];
        let body = &body[..body.find("\n}\n").unwrap()];
        body.lines()
            .map(str::trim)
            .filter(|l| l.chars().next().is_some_and(|c| c.is_ascii_uppercase()))
            .map(|l| l.split(['(', ',']).next().unwrap())
            .collect::<Vec<_>>()
            .into_iter()
            .map(|s| &*Box::leak(s.to_owned().into_boxed_str()))
            .collect()
    };
    for variant in variants_of("InvalidReason") {
        if variant == "Open" {
            continue; // covered through OpenFailure below
        }
        if variant == "Check" {
            assert!(
                covered.iter().any(|r| r.starts_with("Check:")),
                "no Check vector"
            );
            continue;
        }
        assert!(
            covered.iter().any(|r| r == variant),
            "InvalidReason::{variant} has no vector"
        );
    }
    for variant in variants_of("OpenFailure") {
        let wanted = format!("Open:{variant}");
        assert!(
            covered.contains(&wanted) || excused.contains(&variant),
            "OpenFailure::{variant} has no vector"
        );
    }
    for variant in variants_of("Wait") {
        assert!(
            covered.iter().any(|r| r == variant),
            "Wait::{variant} has no vector"
        );
    }
    // The verdict classes themselves.
    for class in ["valid", "invalid", "not_determinable"] {
        assert!(
            doc["cases"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["expect"]["verdict"] == class),
            "no {class} vector"
        );
    }
    // Type-check that the enums used above are the real ones.
    let _ = (Wait::ChainBehind, OpenFailure::NotOpenable);
}
