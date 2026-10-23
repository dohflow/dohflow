//! The membership-chain conformance vectors (`docs/architecture/vectors/membership-chain-v1.json`,
//! personal-cfo-g3m.21) and the tests that keep the verifier, the generator and the file in
//! agreement.
//!
//! The file is PUBLIC and carries only public data: signed records (hex), public keys, and the
//! expected outcome of each case. The test-only key seeds live here, in the generator, as
//! unmistakably synthetic repeated bytes; they never appear in the file. Signing is RFC 8032
//! Ed25519 and so deterministic: regenerating the file gives the same bytes, and a test fails if
//! the committed file drifts from the generator. A TypeScript verifier or a server pins the file's
//! `vectors_version` and must agree with every case, on the C.11 failure name and the position.
//!
//! Regenerate with `DOHFLOW_UPDATE_VECTORS=1 cargo test -p sync-core --test membership_vectors`
//! (a spec change under C.13 change control).

use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sync_core::cbe::Writer;
use sync_core::header::Fingerprint;
use sync_core::keys::{
    ed25519_point_acceptable, ed25519_verify_strict, x25519_public_key_acceptable,
};
use sync_core::membership::{
    device_fingerprint, extend, verify_chain, Action, ChainError, EpochBody, MembershipBody,
    Reason, Recipient, SignedRecord, VerifiedState, KEM_ID, SEALED_KEYS_CIPHERTEXT_LEN, SUITE_ID,
};

const VECTORS: &str = include_str!("../../../docs/architecture/vectors/membership-chain-v1.json");
const VECTORS_PATH: &str = "../../docs/architecture/vectors/membership-chain-v1.json";
const VAULT: [u8; 16] = [0, 0, 0, 0, 0, 0, 0x40, 0, 0x80, 0, 0, 0, 0, 0, 0, 0xf0];

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A synthetic device. The seed is a repeated byte; nothing here is a real key.
#[derive(Clone)]
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
        device_id[14] = 0xa0;
        device_id[15] = n;
        Self {
            ed25519_pk: signing.verifying_key().to_bytes(),
            signing,
            // A synthetic X25519 public value: an ordinary u-coordinate, not a real key.
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
}

fn sign(body: &[u8], by: &SigningKey) -> Vec<u8> {
    SignedRecord {
        body: body.to_vec(),
        signature: by.sign(body).to_bytes(),
    }
    .encode()
    .unwrap()
}

fn id_of(record: &[u8]) -> [u8; 32] {
    Sha256::digest(&record[4..record.len() - 64]).into()
}

fn membership_body(
    signer: &Dev,
    version: u64,
    prev: Option<[u8; 32]>,
    action: Action,
    target: &Dev,
) -> MembershipBody {
    MembershipBody {
        suite_id: SUITE_ID,
        kem_id: KEM_ID,
        vault_id: VAULT,
        version,
        prev_id: prev,
        action,
        signer_fingerprint: signer.fingerprint(),
        device_id: target.device_id,
        ed25519_pk: target.ed25519_pk,
        kem_public_key: target.kem_pk,
        issued_at: 1_790_000_000_000_000 + version as i64 * 1_000_000,
    }
}

fn membership(
    signer: &Dev,
    version: u64,
    prev: Option<[u8; 32]>,
    action: Action,
    target: &Dev,
) -> Vec<u8> {
    sign(
        &membership_body(signer, version, prev, action, target)
            .encode()
            .unwrap(),
        &signer.signing,
    )
}

fn commitment(label: &str, key: [u8; 32]) -> [u8; 32] {
    let mut w = Writer::with_label(label);
    w.fixed(&key);
    Sha256::digest(w.as_bytes()).into()
}

/// The commitments to a synthetic epoch key `K_n` and to the synthetic tag secret `S`.
fn ke(n: u8) -> [u8; 32] {
    commitment("dohflow/sync-epoch-key-commit/v1", [0x50 + n; 32])
}

fn s_commitment() -> [u8; 32] {
    commitment("dohflow/sync-tag-key-commit/v1", [0x60; 32])
}

fn recipient(dev: &Dev) -> Recipient {
    Recipient {
        device_id: dev.device_id,
        fingerprint: dev.fingerprint(),
        suite_id: SUITE_ID,
        kem_id: KEM_ID,
        // Placeholders of the exact suite-1 lengths; HPKE is out of scope of these vectors.
        enc: [0xee; 32],
        ciphertext: vec![0xcc; SEALED_KEYS_CIPHERTEXT_LEN],
    }
}

#[allow(clippy::too_many_arguments)]
fn epoch_body(
    sealer: &Dev,
    key_epoch: u64,
    reason: Reason,
    head: [u8; 32],
    prev: Option<[u8; 32]>,
    ke_commit: [u8; 32],
    s_commit: [u8; 32],
    recipients: &[&Dev],
) -> EpochBody {
    let mut recipients: Vec<Recipient> = recipients.iter().map(|d| recipient(d)).collect();
    recipients.sort_by(|a, b| a.fingerprint.as_bytes().cmp(b.fingerprint.as_bytes()));
    EpochBody {
        suite_id: SUITE_ID,
        kem_id: KEM_ID,
        vault_id: VAULT,
        key_epoch,
        reason,
        membership_head: head,
        prev_epoch_id: prev,
        key_commitment_ke: ke_commit,
        key_commitment_s: s_commit,
        sealer_fingerprint: sealer.fingerprint(),
        recipients,
    }
}

#[allow(clippy::too_many_arguments)]
fn epoch(
    sealer: &Dev,
    key_epoch: u64,
    reason: Reason,
    head: [u8; 32],
    prev: Option<[u8; 32]>,
    ke_commit: [u8; 32],
    s_commit: [u8; 32],
    recipients: &[&Dev],
) -> Vec<u8> {
    sign(
        &epoch_body(
            sealer, key_epoch, reason, head, prev, ke_commit, s_commit, recipients,
        )
        .encode()
        .unwrap(),
        &sealer.signing,
    )
}

/// A growing, valid chain the cases branch from.
struct Built {
    a: Dev,
    b: Dev,
    c: Dev,
    m1: Vec<u8>,
    e1: Vec<u8>,
    m2: Vec<u8>,
    e2: Vec<u8>,
    m3: Vec<u8>,
    e3: Vec<u8>,
    e4: Vec<u8>,
}

fn build() -> Built {
    let (a, b, c) = (Dev::new(0x11), Dev::new(0x22), Dev::new(0x33));
    // Genesis: A adds itself; epoch 1 seals K_1 to A.
    let m1 = membership(&a, 1, None, Action::Add, &a);
    let e1 = epoch(
        &a,
        1,
        Reason::Genesis,
        id_of(&m1),
        None,
        ke(1),
        s_commitment(),
        &[&a],
    );
    // A adds B; a grant (same key) seals to B only.
    let m2 = membership(&a, 2, Some(id_of(&m1)), Action::Add, &b);
    let e2 = epoch(
        &a,
        1,
        Reason::Grant,
        id_of(&m2),
        Some(id_of(&e1)),
        ke(1),
        s_commitment(),
        &[&b],
    );
    // B revokes... A adds C then B (a member) revokes A: a cutover sealed by B to B and C.
    let m3 = membership(&a, 3, Some(id_of(&m2)), Action::Add, &c);
    let e3 = epoch(
        &a,
        1,
        Reason::Grant,
        id_of(&m3),
        Some(id_of(&e2)),
        ke(1),
        s_commitment(),
        &[&c],
    );
    // A forced rotation at the same head: fresh key, same members.
    let e4 = epoch(
        &b,
        2,
        Reason::Rotation,
        id_of(&m3),
        Some(id_of(&e3)),
        ke(2),
        s_commitment(),
        &[&a, &b, &c],
    );
    Built {
        a,
        b,
        c,
        m1,
        e1,
        m2,
        e2,
        m3,
        e3,
        e4,
    }
}

fn refs(v: &[&Vec<u8>]) -> Vec<Vec<u8>> {
    v.iter().map(|x| (*x).clone()).collect()
}

/// One call to `extend`: the membership records and the epoch records it is given.
type Step = (Vec<Vec<u8>>, Vec<Vec<u8>>);

/// A case: one or more steps, each extending the state the previous successful step left.
struct Case {
    name: &'static str,
    description: &'static str,
    steps: Vec<Step>,
}

fn one(
    name: &'static str,
    description: &'static str,
    memberships: Vec<Vec<u8>>,
    epochs: Vec<Vec<u8>>,
) -> Case {
    Case {
        name,
        description,
        steps: vec![(memberships, epochs)],
    }
}

fn slices(v: &[Vec<u8>]) -> Vec<&[u8]> {
    v.iter().map(Vec::as_slice).collect()
}

fn state_summary(state: &VerifiedState) -> Value {
    json!({
        "result": "valid",
        "head_version": state.head_version(),
        "head_id": hex(&state.head_id().unwrap_or([0; 32])),
        "epoch_head_id": hex(&state.epoch_head_id().unwrap_or([0; 32])),
        "key_epoch": state.key_epoch(),
        "authorized": state.authorized().map(|m| hex(m.fingerprint.as_bytes())).collect::<Vec<_>>(),
    })
}

fn error_summary(error: &ChainError) -> Value {
    json!({
        "result": "invalid",
        "kind": error.kind().name(),
        "position": error.position(),
        "halts_sync": error.halts_sync(),
    })
}

/// Run a case with the verifier: returns the expected-outcome JSON for each step.
fn run(case: &Case) -> Vec<Value> {
    let mut state = VerifiedState::empty(VAULT);
    let mut outcomes = Vec::new();
    for (memberships, epochs) in &case.steps {
        match extend(&state, &slices(memberships), &slices(epochs)) {
            Ok(next) => {
                outcomes.push(state_summary(&next));
                state = next;
            }
            Err(error) => outcomes.push(error_summary(&error)),
        }
    }
    outcomes
}

fn all_cases() -> Vec<Case> {
    let t = build();
    let (a, b, c) = (&t.a, &t.b, &t.c);
    let mut cases = Vec::new();

    // ---- valid ----
    cases.push(one(
        "valid_genesis_only",
        "the first device and its genesis epoch record",
        refs(&[&t.m1]),
        refs(&[&t.e1]),
    ));
    cases.push(one(
        "valid_add_with_grant",
        "A adds B; the grant seals the unchanged key to B only",
        refs(&[&t.m1, &t.m2]),
        refs(&[&t.e1, &t.e2]),
    ));
    cases.push(one(
        "valid_three_members",
        "A adds B then C, one grant each",
        refs(&[&t.m1, &t.m2, &t.m3]),
        refs(&[&t.e1, &t.e2, &t.e3]),
    ));
    // Revoke with cutover: B revokes... A revokes C? Only an authorized member signs. A revokes B.
    let m4 = membership(a, 4, Some(id_of(&t.m3)), Action::Revoke, b);
    let e5 = epoch(
        a,
        2,
        Reason::Cutover,
        id_of(&m4),
        Some(id_of(&t.e3)),
        ke(2),
        s_commitment(),
        &[a, c],
    );
    cases.push(one(
        "valid_revoke_with_cutover",
        "A revokes B; the cutover publishes a fresh key sealed to the remaining members",
        refs(&[&t.m1, &t.m2, &t.m3, &m4]),
        refs(&[&t.e1, &t.e2, &t.e3, &e5]),
    ));
    cases.push(one(
        "valid_forced_rotation",
        "a reason-4 rotation names the same head, a fresh key, the same members",
        refs(&[&t.m1, &t.m2, &t.m3]),
        refs(&[&t.e1, &t.e2, &t.e3, &t.e4]),
    ));
    cases.push(Case {
        name: "valid_incremental_extension",
        description:
            "the chain is verified in two calls; the second extends the state the first returned",
        steps: vec![
            (refs(&[&t.m1, &t.m2]), refs(&[&t.e1, &t.e2])),
            (refs(&[&t.m3]), refs(&[&t.e3])),
            (vec![], vec![]),
        ],
    });
    cases.push(Case {
        name: "valid_exact_duplicate_is_idempotent",
        description: "re-presenting records already accepted changes nothing",
        steps: vec![
            (refs(&[&t.m1, &t.m2]), refs(&[&t.e1, &t.e2])),
            (refs(&[&t.m2]), refs(&[&t.e2])),
        ],
    });

    // B seals a rotation while authorized (head 3); A then revokes B. The rotation stays valid
    // because its sealer is checked at its OWN named head, not at the newest.
    let e5_after_rotation = epoch(
        a,
        3,
        Reason::Cutover,
        id_of(&m4),
        Some(id_of(&t.e4)),
        ke(3),
        s_commitment(),
        &[a, c],
    );
    cases.push(one("valid_epoch_sealed_by_a_member_revoked_later", "a rotation sealed by B stays valid after B is revoked, because B was authorized at its head",
        refs(&[&t.m1, &t.m2, &t.m3, &m4]), refs(&[&t.e1, &t.e2, &t.e3, &t.e4, &e5_after_rotation])));
    // ---- invalid: signatures and signers ----
    let mut unsigned = t.m2.clone();
    let len = unsigned.len();
    unsigned[len - 64..].fill(0);
    cases.push(one(
        "invalid_unsigned_add",
        "an add whose signature is all zero",
        refs(&[&t.m1]).into_iter().chain([unsigned]).collect(),
        refs(&[&t.e1, &t.e2]),
    ));
    let intruder = Dev::new(0x77);
    let stranger = membership(&intruder, 2, Some(id_of(&t.m1)), Action::Add, b);
    cases.push(one(
        "invalid_signer_not_a_member",
        "a record signed by a device that was never added",
        refs(&[&t.m1, &stranger]),
        refs(&[&t.e1]),
    ));
    let m4_revoked_signer = membership(b, 5, Some(id_of(&m4)), Action::Add, &Dev::new(0x44));
    cases.push(one(
        "invalid_signer_revoked",
        "B signs after A revoked it",
        refs(&[&t.m1, &t.m2, &t.m3, &m4, &m4_revoked_signer]),
        refs(&[&t.e1, &t.e2, &t.e3, &e5]),
    ));
    // The key in the record is not the key that verifies: sign with the wrong key but name A.
    let forged = sign(
        &membership_body(a, 2, Some(id_of(&t.m1)), Action::Add, b)
            .encode()
            .unwrap(),
        &intruder.signing,
    );
    cases.push(one(
        "invalid_signature_by_another_key",
        "named signer A, signed with a different key",
        refs(&[&t.m1]).into_iter().chain([forged]).collect(),
        refs(&[&t.e1]),
    ));

    // The attacker's own key signs the add of the attacker, while naming member A as the signer:
    // verifying with the key inside the record would accept it; the chain's key for A rejects it.
    let attacker = Dev::new(0x78);
    let self_signed = sign(
        &membership_body(a, 2, Some(id_of(&t.m1)), Action::Add, &attacker)
            .encode()
            .unwrap(),
        &attacker.signing,
    );
    cases.push(one(
        "invalid_add_signed_by_the_key_it_adds",
        "names A as the signer but is signed by the key being added",
        refs(&[&t.m1]).into_iter().chain([self_signed]).collect(),
        refs(&[&t.e1]),
    ));
    // ---- invalid: linkage ----
    cases.push(one(
        "invalid_wrong_prev_id",
        "prev_id is not the previous record's id",
        refs(&[&t.m1])
            .into_iter()
            .chain([membership(a, 2, Some([0x99; 32]), Action::Add, b)])
            .collect(),
        refs(&[&t.e1]),
    ));
    cases.push(one(
        "invalid_wrong_version",
        "version skips a number",
        refs(&[&t.m1])
            .into_iter()
            .chain([membership(a, 3, Some(id_of(&t.m1)), Action::Add, b)])
            .collect(),
        refs(&[&t.e1]),
    ));
    let other_m2 = membership(a, 2, Some(id_of(&t.m1)), Action::Add, c);
    cases.push(Case {
        name: "invalid_fork",
        description: "a different record at an already-accepted version",
        steps: vec![
            (refs(&[&t.m1, &t.m2]), refs(&[&t.e1, &t.e2])),
            (refs(&[&other_m2]), vec![]),
        ],
    });
    cases.push(one(
        "invalid_genesis_not_self_signed",
        "the first record is signed by someone else",
        vec![membership(&intruder, 1, None, Action::Add, a)],
        vec![],
    ));

    // ---- invalid: adds and revokes ----
    let dup_fp = membership(a, 3, Some(id_of(&t.m2)), Action::Add, b);
    cases.push(one(
        "invalid_fingerprint_reuse",
        "an add of a fingerprint already in the chain",
        refs(&[&t.m1, &t.m2, &dup_fp]),
        refs(&[&t.e1, &t.e2]),
    ));
    let mut reuse_id = Dev::new(0x55);
    reuse_id.device_id = b.device_id;
    cases.push(one(
        "invalid_device_id_reuse",
        "an add that reuses a device id under new keys",
        refs(&[&t.m1, &t.m2])
            .into_iter()
            .chain([membership(a, 3, Some(id_of(&t.m2)), Action::Add, &reuse_id)])
            .collect(),
        refs(&[&t.e1, &t.e2]),
    ));
    let mut reuse_key = Dev::new(0x66);
    reuse_key.ed25519_pk = b.ed25519_pk;
    reuse_key.signing = b.signing.clone();
    cases.push(one(
        "invalid_public_key_reuse",
        "a new device that reuses another member's Ed25519 key",
        refs(&[&t.m1, &t.m2])
            .into_iter()
            .chain([membership(
                a,
                3,
                Some(id_of(&t.m2)),
                Action::Add,
                &reuse_key,
            )])
            .collect(),
        refs(&[&t.e1, &t.e2]),
    ));
    let mut tampered_revoke = membership_body(a, 4, Some(id_of(&t.m3)), Action::Revoke, b);
    tampered_revoke.kem_public_key[0] ^= 1;
    cases.push(one(
        "invalid_revoke_not_repeating_the_add",
        "a revoke whose KEM key differs from the add's",
        refs(&[&t.m1, &t.m2, &t.m3])
            .into_iter()
            .chain([sign(&tampered_revoke.encode().unwrap(), &a.signing)])
            .collect(),
        refs(&[&t.e1, &t.e2, &t.e3]),
    ));
    let solo_revoke = membership(a, 2, Some(id_of(&t.m1)), Action::Revoke, a);
    cases.push(one(
        "invalid_revoke_of_the_last_member",
        "a revoke that would leave nobody to seal the cutover",
        refs(&[&t.m1]).into_iter().chain([solo_revoke]).collect(),
        refs(&[&t.e1]),
    ));
    // Unacceptable keys at add time.
    let mut weak = Dev::new(0x88);
    weak.ed25519_pk = {
        let mut p = [0_u8; 32];
        p[0] = 1; // the identity point
        p
    };
    cases.push(one(
        "invalid_small_order_ed25519_key",
        "an add whose Ed25519 key is the identity point",
        refs(&[&t.m1])
            .into_iter()
            .chain([membership(a, 2, Some(id_of(&t.m1)), Action::Add, &weak)])
            .collect(),
        refs(&[&t.e1]),
    ));
    let mut noncanonical = Dev::new(0x89);
    noncanonical.ed25519_pk = noncanonical_valid_point();
    cases.push(one(
        "invalid_non_canonical_ed25519_key",
        "an add whose Ed25519 key is a valid point in a non-canonical encoding",
        refs(&[&t.m1])
            .into_iter()
            .chain([membership(
                a,
                2,
                Some(id_of(&t.m1)),
                Action::Add,
                &noncanonical,
            )])
            .collect(),
        refs(&[&t.e1]),
    ));
    let mut low_kem = Dev::new(0x8a);
    low_kem.kem_pk = [0; 32];
    cases.push(one(
        "invalid_zero_kem_key",
        "an add whose X25519 key is all zero",
        refs(&[&t.m1])
            .into_iter()
            .chain([membership(a, 2, Some(id_of(&t.m1)), Action::Add, &low_kem)])
            .collect(),
        refs(&[&t.e1]),
    ));
    let mut twin_kem = Dev::new(0x8b);
    twin_kem.kem_pk = {
        let mut k = [0xff_u8; 32];
        k[0] = 0xed; // p with the unused top bit set: a non-canonical twin of u = 0
        k
    };
    cases.push(one(
        "invalid_non_canonical_low_order_kem_key",
        "an add whose X25519 key is a non-canonical twin of a low-order value",
        refs(&[&t.m1])
            .into_iter()
            .chain([membership(a, 2, Some(id_of(&t.m1)), Action::Add, &twin_kem)])
            .collect(),
        refs(&[&t.e1]),
    ));

    // ---- invalid: signatures at the byte level ----
    let sig_case = |name: &'static str, description: &'static str, edit: &dyn Fn(&mut [u8; 64])| {
        let mut record = t.m2.clone();
        let n = record.len();
        let mut sig: [u8; 64] = record[n - 64..].try_into().unwrap();
        edit(&mut sig);
        record[n - 64..].copy_from_slice(&sig);
        one(
            name,
            description,
            refs(&[&t.m1]).into_iter().chain([record]).collect(),
            refs(&[&t.e1, &t.e2]),
        )
    };
    cases.push(sig_case(
        "invalid_signature_s_at_or_above_group_order",
        "S + L: the same signature with a non-canonical scalar",
        &|sig| add_group_order(sig),
    ));
    cases.push(sig_case(
        "invalid_signature_small_order_r",
        "R is the identity point",
        &|sig| {
            sig[..32].fill(0);
            sig[0] = 1;
        },
    ));
    cases.push(sig_case(
        "invalid_signature_non_canonical_r",
        "R is a valid point in a non-canonical encoding",
        &|sig| sig[..32].copy_from_slice(&noncanonical_valid_point()),
    ));

    // Points of order 8 as the key and as R (the identity is covered above).
    let order_8: [u8; 32] =
        unhex("c7176a703d4dd84fba3c0b760d10670f2a2053fa2c39ccc64ec7fd7792ac037a")
            .try_into()
            .unwrap();
    let mut order_8_key = Dev::new(0x8d);
    order_8_key.ed25519_pk = order_8;
    cases.push(one(
        "invalid_order_8_ed25519_key",
        "an add whose Ed25519 key is a point of order 8",
        refs(&[&t.m1])
            .into_iter()
            .chain([membership(
                a,
                2,
                Some(id_of(&t.m1)),
                Action::Add,
                &order_8_key,
            )])
            .collect(),
        refs(&[&t.e1]),
    ));
    cases.push(sig_case(
        "invalid_signature_order_8_r",
        "R is a point of order 8",
        &|sig| sig[..32].copy_from_slice(&order_8),
    ));
    // A mixed-order key and a mixed-order R: valid points, canonical encodings, not torsion-free.
    let mut mixed_key = Dev::new(0x8c);
    mixed_key.ed25519_pk = add_torsion(&mixed_key.ed25519_pk);
    cases.push(one(
        "invalid_mixed_order_ed25519_key",
        "an add whose Ed25519 key is a full-order point plus a point of order 8",
        refs(&[&t.m1])
            .into_iter()
            .chain([membership(
                a,
                2,
                Some(id_of(&t.m1)),
                Action::Add,
                &mixed_key,
            )])
            .collect(),
        refs(&[&t.e1]),
    ));
    cases.push(sig_case(
        "invalid_signature_mixed_order_r",
        "R is the real R plus a point of order 8",
        &|sig| {
            let r: [u8; 32] = sig[..32].try_into().unwrap();
            sig[..32].copy_from_slice(&add_torsion(&r));
        },
    ));

    // ---- invalid: conflicts that are not authenticated are not forks ----
    let conflicting_add = |sign_with: Option<&Dev>| {
        let body = membership_body(a, 2, Some(id_of(&t.m1)), Action::Add, c)
            .encode()
            .unwrap();
        match sign_with {
            Some(dev) => sign(&body, &dev.signing),
            None => SignedRecord {
                body,
                signature: [0; 64],
            }
            .encode()
            .unwrap(),
        }
    };
    cases.push(Case {
        name: "invalid_unsigned_conflicting_record_is_not_a_fork",
        description:
            "a record at an accepted version with no valid signature is garbage, not a fork",
        steps: vec![
            (refs(&[&t.m1, &t.m2]), refs(&[&t.e1, &t.e2])),
            (vec![conflicting_add(None)], vec![]),
        ],
    });
    cases.push(Case {
        name: "invalid_conflicting_record_from_a_non_member_is_not_a_fork",
        description:
            "a record at an accepted version signed by a device that is not a member is not a fork",
        steps: vec![
            (refs(&[&t.m1, &t.m2]), refs(&[&t.e1, &t.e2])),
            (
                vec![sign(
                    &membership_body(&intruder, 2, Some(id_of(&t.m1)), Action::Add, c)
                        .encode()
                        .unwrap(),
                    &intruder.signing,
                )],
                vec![],
            ),
        ],
    });
    let foreign_genesis = membership(&intruder, 1, None, Action::Add, &intruder);
    cases.push(Case {
        name: "invalid_foreign_genesis_is_not_a_fork",
        description: "a validly self-signed genesis from a device that is not the founder, for the same vault, is not evidence of a fork",
        steps: vec![
            (refs(&[&t.m1]), refs(&[&t.e1])),
            (vec![foreign_genesis], vec![]),
        ],
    });
    let founder_regenesis = {
        let mut body = membership_body(a, 1, None, Action::Add, a);
        body.issued_at += 1;
        sign(&body.encode().unwrap(), &a.signing)
    };
    cases.push(Case {
        name: "invalid_second_genesis_signed_by_the_founder_is_a_fork",
        description: "the founding device signed two different genesis records: a real fork",
        steps: vec![
            (refs(&[&t.m1]), refs(&[&t.e1])),
            (vec![founder_regenesis], vec![]),
        ],
    });
    let conflicting_epoch = |sealer: &Dev, signer: Option<&Dev>| {
        let body = epoch_body(
            sealer,
            2,
            Reason::Rotation,
            id_of(&t.m3),
            Some(id_of(&t.e3)),
            ke(3),
            s_commitment(),
            &[a, b, c],
        )
        .encode()
        .unwrap();
        match signer {
            Some(dev) => sign(&body, &dev.signing),
            None => SignedRecord {
                body,
                signature: [0; 64],
            }
            .encode()
            .unwrap(),
        }
    };
    cases.push(Case {
        name: "invalid_unsigned_conflicting_epoch_is_not_equivocation",
        description: "an epoch record after an accepted prev_epoch_id with no valid signature is garbage, not equivocation",
        steps: vec![
            (refs(&[&t.m1, &t.m2, &t.m3]), refs(&[&t.e1, &t.e2, &t.e3, &t.e4])),
            (vec![], vec![conflicting_epoch(b, None)]),
        ],
    });
    cases.push(Case {
        name: "invalid_conflicting_epoch_from_a_non_member_is_not_equivocation",
        description: "an epoch record after an accepted prev_epoch_id sealed by a non-member is not equivocation",
        steps: vec![
            (refs(&[&t.m1, &t.m2, &t.m3]), refs(&[&t.e1, &t.e2, &t.e3, &t.e4])),
            (vec![], vec![conflicting_epoch(&intruder, Some(&intruder))]),
        ],
    });

    // ---- invalid: epoch side ----
    cases.push(one(
        "invalid_membership_without_an_epoch_record",
        "an add that no epoch record names",
        refs(&[&t.m1, &t.m2, &t.m3]),
        refs(&[&t.e1, &t.e2]),
    ));
    cases.push(one(
        "invalid_no_genesis_epoch",
        "a genesis membership with no epoch record",
        refs(&[&t.m1]),
        vec![],
    ));
    let cutover_naming_add = epoch(
        a,
        2,
        Reason::Cutover,
        id_of(&t.m2),
        Some(id_of(&t.e1)),
        ke(2),
        s_commitment(),
        &[a, b],
    );
    cases.push(one(
        "invalid_cutover_naming_an_add",
        "a cutover whose head is an add",
        refs(&[&t.m1, &t.m2]),
        refs(&[&t.e1, &cutover_naming_add]),
    ));
    let grant_naming_revoke = epoch(
        a,
        1,
        Reason::Grant,
        id_of(&m4),
        Some(id_of(&t.e3)),
        ke(1),
        s_commitment(),
        &[a],
    );
    cases.push(one(
        "invalid_grant_naming_a_revoke",
        "a grant whose head is a revoke",
        refs(&[&t.m1, &t.m2, &t.m3, &m4]),
        refs(&[&t.e1, &t.e2, &t.e3, &grant_naming_revoke]),
    ));
    let sealer_revoked = epoch(
        b,
        2,
        Reason::Cutover,
        id_of(&m4),
        Some(id_of(&t.e3)),
        ke(2),
        s_commitment(),
        &[a, c],
    );
    cases.push(one(
        "invalid_sealer_revoked_at_its_head",
        "the cutover is sealed by the device it revokes",
        refs(&[&t.m1, &t.m2, &t.m3, &m4]),
        refs(&[&t.e1, &t.e2, &t.e3, &sealer_revoked]),
    ));
    let reused_key_cutover = epoch(
        a,
        2,
        Reason::Cutover,
        id_of(&m4),
        Some(id_of(&t.e3)),
        ke(1),
        s_commitment(),
        &[a, c],
    );
    cases.push(one(
        "invalid_cutover_reusing_an_old_key",
        "a cutover that republishes the old key's commitment",
        refs(&[&t.m1, &t.m2, &t.m3, &m4]),
        refs(&[&t.e1, &t.e2, &t.e3, &reused_key_cutover]),
    ));
    let changed_s = epoch(
        a,
        2,
        Reason::Cutover,
        id_of(&m4),
        Some(id_of(&t.e3)),
        ke(2),
        commitment("dohflow/sync-tag-key-commit/v1", [0x61; 32]),
        &[a, c],
    );
    cases.push(one(
        "invalid_secret_commitment_changes",
        "a cutover that changes the S commitment",
        refs(&[&t.m1, &t.m2, &t.m3, &m4]),
        refs(&[&t.e1, &t.e2, &t.e3, &changed_s]),
    ));
    let extra_recipient = epoch(
        a,
        2,
        Reason::Cutover,
        id_of(&m4),
        Some(id_of(&t.e3)),
        ke(2),
        s_commitment(),
        &[a, b, c],
    );
    cases.push(one(
        "invalid_cutover_sealing_to_the_revoked_member",
        "an extra recipient: the revoked device",
        refs(&[&t.m1, &t.m2, &t.m3, &m4]),
        refs(&[&t.e1, &t.e2, &t.e3, &extra_recipient]),
    ));
    let missing_recipient = epoch(
        a,
        2,
        Reason::Cutover,
        id_of(&m4),
        Some(id_of(&t.e3)),
        ke(2),
        s_commitment(),
        &[a],
    );
    cases.push(one(
        "invalid_cutover_missing_a_recipient",
        "a remaining member is left out",
        refs(&[&t.m1, &t.m2, &t.m3, &m4]),
        refs(&[&t.e1, &t.e2, &t.e3, &missing_recipient]),
    ));
    let rotation_new_head = epoch(
        a,
        2,
        Reason::Rotation,
        id_of(&t.m3),
        Some([0x99; 32]),
        ke(2),
        s_commitment(),
        &[a, b, c],
    );
    cases.push(one(
        "invalid_epoch_skipping_its_predecessor",
        "prev_epoch_id names no record of the chain",
        refs(&[&t.m1, &t.m2, &t.m3]),
        refs(&[&t.e1, &t.e2, &t.e3, &rotation_new_head]),
    ));
    let equivocal = epoch(
        b,
        2,
        Reason::Rotation,
        id_of(&t.m3),
        Some(id_of(&t.e3)),
        ke(3),
        s_commitment(),
        &[a, b, c],
    );
    cases.push(Case {
        name: "invalid_epoch_equivocation",
        description: "two different epoch records follow one prev_epoch_id",
        steps: vec![
            (
                refs(&[&t.m1, &t.m2, &t.m3]),
                refs(&[&t.e1, &t.e2, &t.e3, &t.e4]),
            ),
            (vec![], refs(&[&equivocal])),
        ],
    });
    // Recipients out of canonical order: swap the two recipient blocks of the rotation record.
    let unsorted = {
        let body = epoch_body(
            b,
            2,
            Reason::Rotation,
            id_of(&t.m3),
            Some(id_of(&t.e3)),
            ke(2),
            s_commitment(),
            &[a, b, c],
        );
        let mut bytes = body.encode().unwrap();
        let block = 16 + 32 + 2 + 2 + (4 + 32) + (4 + SEALED_KEYS_CIPHERTEXT_LEN);
        let first = bytes.len() - 3 * block;
        let (x, y) = (
            bytes[first..first + block].to_vec(),
            bytes[first + block..first + 2 * block].to_vec(),
        );
        bytes[first..first + block].copy_from_slice(&y);
        bytes[first + block..first + 2 * block].copy_from_slice(&x);
        sign(&bytes, &b.signing)
    };
    cases.push(one(
        "invalid_recipients_out_of_order",
        "the same recipient set in a non-canonical order",
        refs(&[&t.m1, &t.m2, &t.m3]),
        refs(&[&t.e1, &t.e2, &t.e3, &unsorted]),
    ));

    let stranger_seal = epoch(
        &intruder,
        2,
        Reason::Rotation,
        id_of(&t.m3),
        Some(id_of(&t.e3)),
        ke(2),
        s_commitment(),
        &[a, b, c],
    );
    cases.push(one(
        "invalid_sealer_not_a_member",
        "an epoch record sealed by a device that is not in the chain",
        refs(&[&t.m1, &t.m2, &t.m3]),
        refs(&[&t.e1, &t.e2, &t.e3, &stranger_seal]),
    ));
    let wrong_key_seal = sign(
        &epoch_body(
            b,
            2,
            Reason::Rotation,
            id_of(&t.m3),
            Some(id_of(&t.e3)),
            ke(2),
            s_commitment(),
            &[a, b, c],
        )
        .encode()
        .unwrap(),
        &intruder.signing,
    );
    cases.push(one(
        "invalid_epoch_signed_with_another_key",
        "named sealer B, signed with a different key",
        refs(&[&t.m1, &t.m2, &t.m3]),
        refs(&[&t.e1, &t.e2, &t.e3, &wrong_key_seal]),
    ));
    let unknown_head = epoch(
        a,
        2,
        Reason::Rotation,
        [0x98; 32],
        Some(id_of(&t.e3)),
        ke(2),
        s_commitment(),
        &[a, b, c],
    );
    cases.push(one(
        "invalid_epoch_naming_an_unknown_head",
        "the epoch record names a membership record that is not in the chain",
        refs(&[&t.m1, &t.m2, &t.m3]),
        refs(&[&t.e1, &t.e2, &t.e3, &unknown_head]),
    ));
    let grant_to_old = epoch(
        a,
        1,
        Reason::Grant,
        id_of(&t.m2),
        Some(id_of(&t.e1)),
        ke(1),
        s_commitment(),
        &[a, b],
    );
    cases.push(one(
        "invalid_grant_sealed_to_an_existing_member",
        "a grant that lists the founding device again",
        refs(&[&t.m1, &t.m2]),
        refs(&[&t.e1, &grant_to_old]),
    ));
    let grant_new_key = epoch(
        a,
        1,
        Reason::Grant,
        id_of(&t.m2),
        Some(id_of(&t.e1)),
        ke(2),
        s_commitment(),
        &[&t.b],
    );
    cases.push(one(
        "invalid_grant_that_changes_the_key",
        "a grant whose K_e commitment differs from the previous record's",
        refs(&[&t.m1, &t.m2]),
        refs(&[&t.e1, &grant_new_key]),
    ));

    // ---- invalid: versions and suites ----
    let mut v2 = membership_body(a, 1, None, Action::Add, a)
        .encode()
        .unwrap();
    let version_at = 2 + "dohflow/sync-membership/v1".len();
    v2[version_at + 1] = 2; // protocol_version 2
    cases.push(one(
        "invalid_unsupported_protocol_version",
        "a membership body with protocol_version 2",
        vec![sign(&v2, &a.signing)],
        vec![],
    ));
    let mut suite2 = membership_body(a, 1, None, Action::Add, a);
    suite2.suite_id = 2;
    cases.push(one(
        "invalid_unsupported_suite",
        "a membership body with suite_id 2",
        vec![sign(&suite2.encode().unwrap(), &a.signing)],
        vec![],
    ));
    let mut padded = t.m1.clone();
    padded.push(0);
    cases.push(one(
        "invalid_trailing_byte_after_the_signature",
        "a signed record with one extra byte",
        vec![padded],
        refs(&[&t.e1]),
    ));
    cases
}

/// A valid Ed25519 point written with a y coordinate at or above the field prime, found by
/// scanning the small y values: deterministic, and not small order.
fn noncanonical_valid_point() -> [u8; 32] {
    for y in 2_u8..19 {
        // p + y, little-endian: 2^255 - 19 + y.
        let mut bytes = [0xff_u8; 32];
        bytes[31] = 0x7f;
        bytes[0] = 0xed + y;
        if y > 18 {
            continue;
        }
        if ed25519_dalek::VerifyingKey::from_bytes(&bytes).is_ok_and(|k| !k.is_weak()) {
            return bytes;
        }
    }
    panic!("no non-canonical encoding of a full-order point among y in 2..19");
}

/// A valid point plus a point of order 8: still on the curve and canonically encoded, but of mixed
/// order, so outside the prime-order subgroup.
fn add_torsion(point: &[u8; 32]) -> [u8; 32] {
    let order_8: [u8; 32] =
        unhex("c7176a703d4dd84fba3c0b760d10670f2a2053fa2c39ccc64ec7fd7792ac037a")
            .try_into()
            .unwrap();
    let torsion = VerifyingKey::from_bytes(&order_8).unwrap().to_edwards();
    let base = VerifyingKey::from_bytes(point).unwrap().to_edwards();
    (base + torsion).compress().to_bytes()
}

/// `sig` with `S` replaced by `S + L` (the same scalar mod L, but not below L).
fn add_group_order(sig: &mut [u8; 64]) {
    const L: [u8; 32] = [
        0xed, 0xd3, 0xf5, 0x5c, 0x1a, 0x63, 0x12, 0x58, 0xd6, 0x9c, 0xf7, 0xa2, 0xde, 0xf9, 0xde,
        0x14, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x10,
    ];
    let mut carry = 0_u16;
    for i in 0..32 {
        let sum = u16::from(sig[32 + i]) + u16::from(L[i]) + carry;
        sig[32 + i] = (sum & 0xff) as u8;
        carry = sum >> 8;
    }
    assert_eq!(carry, 0, "S + L fits in 256 bits");
}

fn document() -> Value {
    let cases: Vec<Value> = all_cases()
        .iter()
        .map(|case| {
            let expected = run(case);
            json!({
                "name": case.name,
                "description": case.description,
                "steps": case.steps.iter().zip(expected).map(|((memberships, epochs), expect)| json!({
                    "memberships": memberships.iter().map(|r| hex(r)).collect::<Vec<_>>(),
                    "epochs": epochs.iter().map(|r| hex(r)).collect::<Vec<_>>(),
                    "expect": expect,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let devices: Vec<Value> = [0x11_u8, 0x22, 0x33]
        .iter()
        .map(|n| {
            let d = Dev::new(*n);
            json!({
                "name": format!("device-{n:02x}"),
                "device_id": hex(&d.device_id),
                "ed25519_public_key": hex(&d.ed25519_pk),
                "kem_public_key": hex(&d.kem_pk),
                "fingerprint": hex(d.fingerprint().as_bytes()),
            })
        })
        .collect();
    json!({
        "vectors_version": 1,
        "spec": "docs/architecture/vault-format.md C.6 (spec-version 8)",
        "synthetic": "Every key, id and timestamp here is synthetic and test-only. Private seeds are not in this file.",
        "suite_id": SUITE_ID,
        "kem_id": KEM_ID,
        "vault_id": hex(&VAULT),
        "placeholders": "In epoch records, each recipient's enc is 32 bytes of 0xee and its ciphertext is 109 bytes of 0xcc: placeholders of the exact suite-1 lengths. HPKE is out of scope of these vectors.",
        "check_order": "Every invalid case has exactly one defect, so the order in which an implementation checks does not change its answer. Where a record has several defects, C.6 fixes the order: parse and protocol version, then suite and vault, then position and linkage, then signer and signature, then the rule checks.",
        "how_to_read": "Each case is a list of steps. Step n passes its memberships and epochs (hex of the signed record containers, in chain order) to extend(state, ...), where state is the result of the previous successful step (the empty state for the first). 'expect' is the outcome: 'valid' with the state summary, or 'invalid' with the C.11 failure name (kind), the zero-based position of the offending record in that step's input (null when the failure has none), and whether it halts sync. Implementations must agree on kind and position; internal variant names are not part of the contract.",
        "devices": devices,
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

/// Decode a hex field of the committed file.
fn unhex(text: &str) -> Vec<u8> {
    (0..text.len() / 2)
        .map(|i| u8::from_str_radix(&text[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}

#[test]
fn the_verifier_agrees_with_every_committed_case() {
    let doc: Value = serde_json::from_str(VECTORS).unwrap();
    assert_eq!(doc["vectors_version"], 1);
    let vault: [u8; 16] = unhex(doc["vault_id"].as_str().unwrap()).try_into().unwrap();
    let cases = doc["cases"].as_array().unwrap();
    assert!(cases.len() >= 40, "{} cases", cases.len());
    let mut valid = 0;
    let mut invalid = 0;
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let mut state = VerifiedState::empty(vault);
        for step in case["steps"].as_array().unwrap() {
            let memberships: Vec<Vec<u8>> = step["memberships"]
                .as_array()
                .unwrap()
                .iter()
                .map(|h| unhex(h.as_str().unwrap()))
                .collect();
            let epochs: Vec<Vec<u8>> = step["epochs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|h| unhex(h.as_str().unwrap()))
                .collect();
            let result = extend(&state, &slices(&memberships), &slices(&epochs));
            let expect = &step["expect"];
            match (expect["result"].as_str().unwrap(), result) {
                ("valid", Ok(next)) => {
                    assert_eq!(state_summary(&next), *expect, "{name}");
                    state = next;
                    valid += 1;
                }
                ("invalid", Err(error)) => {
                    assert_eq!(error_summary(&error), *expect, "{name}: {error}");
                    invalid += 1;
                }
                (want, got) => panic!("{name}: expected {want}, got {got:?}"),
            }
        }
    }
    assert!(
        valid >= 8 && invalid >= 30,
        "valid {valid}, invalid {invalid}"
    );
}

#[test]
fn verify_chain_is_extend_from_the_empty_state() {
    let t = build();
    let whole = verify_chain(
        &VAULT,
        &[&t.m1, &t.m2, &t.m3],
        &[&t.e1, &t.e2, &t.e3, &t.e4],
    )
    .unwrap();
    let first = extend(
        &VerifiedState::empty(VAULT),
        &[&t.m1, &t.m2],
        &[&t.e1, &t.e2],
    )
    .unwrap();
    let second = extend(&first, &[&t.m3], &[&t.e3, &t.e4]).unwrap();
    assert_eq!(whole, second);
    assert_eq!(whole.head_version(), 3);
    assert_eq!(whole.key_epoch(), 2);
    assert_eq!(whole.authorized().count(), 3);
}

#[test]
fn a_failed_extend_changes_nothing() {
    let t = build();
    let state = extend(&VerifiedState::empty(VAULT), &[&t.m1], &[&t.e1]).unwrap();
    let before = state.clone();
    let bad = membership(&t.a, 3, Some(id_of(&t.m1)), Action::Add, &t.b);
    assert!(extend(&state, &[&t.m2, &bad], &[&t.e2]).is_err());
    assert_eq!(state, before, "the input state is untouched");
    // And the good part still extends it afterwards.
    assert!(extend(&state, &[&t.m2], &[&t.e2]).is_ok());
}

#[test]
fn the_state_round_trips_canonically_and_a_damaged_one_is_refused() {
    let t = build();
    let state = verify_chain(
        &VAULT,
        &[&t.m1, &t.m2, &t.m3],
        &[&t.e1, &t.e2, &t.e3, &t.e4],
    )
    .unwrap();
    let bytes = state.encode().unwrap();
    let back = VerifiedState::decode(&bytes).unwrap();
    assert_eq!(back, state);
    assert_eq!(back.encode().unwrap(), bytes);
    for len in 0..bytes.len() {
        assert!(
            VerifiedState::decode(&bytes[..len]).is_err(),
            "{len}-byte prefix"
        );
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(VerifiedState::decode(&trailing).is_err());
    // A persisted state still extends: the decoded copy accepts a record the original does.
    let more = extend(&back, &[], &[]).unwrap();
    assert_eq!(more, state);
}

// ---- the acceptance rule itself (A1/A2) -----------------------------------------------------

#[test]
fn the_ed25519_acceptance_rule_rejects_every_weak_or_non_canonical_input() {
    let dev = Dev::new(0x11);
    let message = b"conformance message";
    let signature = dev.signing.sign(message).to_bytes();
    assert!(ed25519_verify_strict(&dev.ed25519_pk, message, &signature));

    // The eight small-order points.
    let small_order = [
        "0100000000000000000000000000000000000000000000000000000000000000",
        "ecffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
        "0000000000000000000000000000000000000000000000000000000000000000",
        "0000000000000000000000000000000000000000000000000000000000000080",
        "26e8958fc2b227b045c3f489f2ef98f0d5dfac05d3c63339b13802886d53fc05",
        "c7176a703d4dd84fba3c0b760d10670f2a2053fa2c39ccc64ec7fd7792ac037a",
        "26e8958fc2b227b045c3f489f2ef98f0d5dfac05d3c63339b13802886d53fc85",
        "c7176a703d4dd84fba3c0b760d10670f2a2053fa2c39ccc64ec7fd7792ac03fa",
    ];
    for encoded in small_order {
        let point: [u8; 32] = unhex(encoded).try_into().unwrap();
        assert!(
            !ed25519_point_acceptable(&point),
            "small-order A {encoded} accepted"
        );
        let mut sig = signature;
        sig[..32].copy_from_slice(&point);
        assert!(
            !ed25519_verify_strict(&dev.ed25519_pk, message, &sig),
            "small-order R {encoded} accepted"
        );
    }
    // A non-canonical (but valid, full-order) encoding of a point.
    let twin = noncanonical_valid_point();
    assert!(!ed25519_point_acceptable(&twin));
    let mut sig = signature;
    sig[..32].copy_from_slice(&twin);
    assert!(!ed25519_verify_strict(&dev.ed25519_pk, message, &sig));
    // S at or above L.
    let mut high_s = signature;
    add_group_order(&mut high_s);
    assert!(!ed25519_verify_strict(&dev.ed25519_pk, message, &high_s));
    // A mixed-order point (valid, canonical, but outside the prime-order subgroup), as A and as R.
    let mixed = add_torsion(&dev.ed25519_pk);
    assert!(!ed25519_point_acceptable(&mixed), "mixed-order A accepted");
    let mut mixed_r = signature;
    mixed_r[..32].copy_from_slice(&add_torsion(&signature[..32].try_into().unwrap()));
    assert!(
        !ed25519_verify_strict(&dev.ed25519_pk, message, &mixed_r),
        "mixed-order R accepted"
    );
    // An honest key is in the prime-order subgroup.
    assert!(ed25519_point_acceptable(&dev.ed25519_pk));
    // Not a point at all.
    assert!(!ed25519_point_acceptable(&[0xff; 32]));
    // A different message, and a flipped bit anywhere in the signature.
    assert!(!ed25519_verify_strict(
        &dev.ed25519_pk,
        b"another",
        &signature
    ));
    for byte in [0, 31, 32, 63] {
        let mut bad = signature;
        bad[byte] ^= 1;
        assert!(
            !ed25519_verify_strict(&dev.ed25519_pk, message, &bad),
            "byte {byte}"
        );
    }
}

#[test]
fn the_x25519_screen_rejects_the_low_order_values_and_their_twins() {
    for encoded in [
        "0000000000000000000000000000000000000000000000000000000000000000",
        "0100000000000000000000000000000000000000000000000000000000000000",
        "e0eb7a7c3b41b8ae1656e3faf19fc46ada098deb9c32b1fd866205165f49b800",
        "5f9c95bca3508c24b1d0b1559c83ef5b04445cc4581c8e86d8224eddd09f1157",
        "ecffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
        "edffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
        "eeffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
        // twins: the unused top bit set (X25519 ignores it), or a value at or above p
        "0000000000000000000000000000000000000000000000000000000000000080",
        "e0eb7a7c3b41b8ae1656e3faf19fc46ada098deb9c32b1fd866205165f49b880",
        "5f9c95bca3508c24b1d0b1559c83ef5b04445cc4581c8e86d8224eddd09f11d7",
        "ecffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        "eeffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
    ] {
        let key: [u8; 32] = unhex(encoded).try_into().unwrap();
        assert!(!x25519_public_key_acceptable(&key), "{encoded} accepted");
    }
    // An ordinary value and the base point are fine.
    let mut base = [0_u8; 32];
    base[0] = 9;
    assert!(x25519_public_key_acceptable(&base));
    assert!(x25519_public_key_acceptable(&Dev::new(0x11).kem_pk));
}

/// Each invalid case fails for the reason its name says, not for an incidental one.
#[test]
fn every_invalid_case_fails_for_its_intended_reason() {
    let intended: &[(&str, &str)] = &[
        ("invalid_unsigned_add", "BadSignature"),
        ("invalid_signer_not_a_member", "SignerUnknown"),
        ("invalid_signer_revoked", "SignerRevoked"),
        ("invalid_signature_by_another_key", "BadSignature"),
        ("invalid_add_signed_by_the_key_it_adds", "BadSignature"),
        ("invalid_mixed_order_ed25519_key", "KeyUnacceptable"),
        ("invalid_signature_mixed_order_r", "BadSignature"),
        (
            "invalid_unsigned_conflicting_record_is_not_a_fork",
            "BadSignature",
        ),
        (
            "invalid_conflicting_record_from_a_non_member_is_not_a_fork",
            "SignerUnknown",
        ),
        (
            "invalid_unsigned_conflicting_epoch_is_not_equivocation",
            "EpochBadSignature",
        ),
        (
            "invalid_conflicting_epoch_from_a_non_member_is_not_equivocation",
            "SealerNotAuthorized",
        ),
        ("invalid_foreign_genesis_is_not_a_fork", "BadGenesis"),
        (
            "invalid_second_genesis_signed_by_the_founder_is_a_fork",
            "Fork",
        ),
        ("invalid_order_8_ed25519_key", "KeyUnacceptable"),
        ("invalid_signature_order_8_r", "BadSignature"),
        ("invalid_wrong_prev_id", "WrongPrevId"),
        ("invalid_wrong_version", "WrongVersion"),
        ("invalid_fork", "Fork"),
        ("invalid_genesis_not_self_signed", "BadGenesis"),
        ("invalid_fingerprint_reuse", "FingerprintReused"),
        ("invalid_device_id_reuse", "DeviceIdReused"),
        ("invalid_public_key_reuse", "KeyReused"),
        ("invalid_revoke_not_repeating_the_add", "RevokeMismatch"),
        ("invalid_revoke_of_the_last_member", "WouldEmptyChain"),
        ("invalid_small_order_ed25519_key", "KeyUnacceptable"),
        ("invalid_non_canonical_ed25519_key", "KeyUnacceptable"),
        ("invalid_zero_kem_key", "KeyUnacceptable"),
        ("invalid_non_canonical_low_order_kem_key", "KeyUnacceptable"),
        (
            "invalid_signature_s_at_or_above_group_order",
            "BadSignature",
        ),
        ("invalid_signature_small_order_r", "BadSignature"),
        ("invalid_signature_non_canonical_r", "BadSignature"),
        (
            "invalid_membership_without_an_epoch_record",
            "EpochRecordMissing",
        ),
        ("invalid_no_genesis_epoch", "EpochRecordMissing"),
        ("invalid_cutover_naming_an_add", "CutoverHeadNotRevoke"),
        ("invalid_grant_naming_a_revoke", "GrantHeadNotAdd"),
        ("invalid_sealer_revoked_at_its_head", "SealerNotAuthorized"),
        ("invalid_cutover_reusing_an_old_key", "BadEpochSequence"),
        ("invalid_secret_commitment_changes", "BadEpochSequence"),
        (
            "invalid_cutover_sealing_to_the_revoked_member",
            "RecipientsMismatch",
        ),
        ("invalid_cutover_missing_a_recipient", "RecipientsMismatch"),
        ("invalid_epoch_skipping_its_predecessor", "BadEpochSequence"),
        ("invalid_epoch_equivocation", "EpochEquivocation"),
        ("invalid_recipients_out_of_order", "Malformed"),
        ("invalid_sealer_not_a_member", "SealerNotAuthorized"),
        ("invalid_epoch_signed_with_another_key", "EpochBadSignature"),
        ("invalid_epoch_naming_an_unknown_head", "UnknownHead"),
        (
            "invalid_grant_sealed_to_an_existing_member",
            "RecipientsMismatch",
        ),
        ("invalid_grant_that_changes_the_key", "BadEpochSequence"),
        (
            "invalid_unsupported_protocol_version",
            "UnsupportedProtocol",
        ),
        ("invalid_unsupported_suite", "UnsupportedSuite"),
        ("invalid_trailing_byte_after_the_signature", "Malformed"),
    ];
    let cases = all_cases();
    for case in &cases {
        if !case.name.starts_with("invalid_") {
            continue;
        }
        let want = intended
            .iter()
            .find(|(name, _)| *name == case.name)
            .unwrap_or_else(|| panic!("{} has no intended reason", case.name))
            .1;
        let mut state = VerifiedState::empty(VAULT);
        let mut failure = None;
        for (memberships, epochs) in &case.steps {
            match extend(&state, &slices(memberships), &slices(epochs)) {
                Ok(next) => state = next,
                Err(error) => failure = Some(error),
            }
        }
        let error = failure.unwrap_or_else(|| panic!("{} did not fail", case.name));
        assert_eq!(error.name(), want, "{}: {error}", case.name);
    }
    assert_eq!(
        intended.len(),
        cases
            .iter()
            .filter(|c| c.name.starts_with("invalid_"))
            .count(),
        "every invalid case has an intended reason"
    );
}

// ---- the codecs reproduce the spec's own C.14 vectors ---------------------------------------

const SPEC: &str = include_str!("../../../docs/architecture/vault-format.md");

fn spec_vector(name: &str) -> Vec<u8> {
    let marker = format!("<!-- sync-vector: {name}=");
    let start = SPEC
        .find(&marker)
        .unwrap_or_else(|| panic!("no vector {name}"))
        + marker.len();
    unhex(SPEC[start..].split(" -->").next().unwrap())
}

/// The spec's device A and B (C.14): their key bytes are fill values, not valid curve points, so
/// the verifier would refuse them, but the codecs and derivations must still reproduce the
/// vectors byte for byte.
fn spec_device(id: u8, ed: u8, kem: u8) -> ([u8; 16], [u8; 32], [u8; 32]) {
    ([id; 16], [ed; 32], [kem; 32])
}

#[test]
fn the_membership_codec_and_fingerprint_reproduce_the_spec_vectors() {
    let (a_id, a_ed, a_kem) = spec_device(0xa1, 0xe1, 0xc1);
    let (b_id, b_ed, b_kem) = spec_device(0xb1, 0xe2, 0xc2);
    let fingerprint_a = device_fingerprint(SUITE_ID, KEM_ID, &a_ed, &a_kem);
    let fingerprint_b = device_fingerprint(SUITE_ID, KEM_ID, &b_ed, &b_kem);
    assert_eq!(
        fingerprint_a.as_bytes().as_slice(),
        spec_vector("fingerprint_a")
    );
    assert_eq!(
        fingerprint_b.as_bytes().as_slice(),
        spec_vector("fingerprint_b")
    );

    let first = MembershipBody {
        suite_id: SUITE_ID,
        kem_id: KEM_ID,
        vault_id: [0x11; 16],
        version: 1,
        prev_id: None,
        action: Action::Add,
        signer_fingerprint: fingerprint_a,
        device_id: a_id,
        ed25519_pk: a_ed,
        kem_public_key: a_kem,
        issued_at: 1_790_000_001_000_000,
    };
    let first_bytes = first.encode().unwrap();
    assert_eq!(first_bytes, spec_vector("membership_body_1"));
    assert_eq!(MembershipBody::decode(&first_bytes).unwrap(), first);
    let first_id: [u8; 32] = Sha256::digest(&first_bytes).into();
    assert_eq!(first_id.as_slice(), spec_vector("membership_id_1"));

    let second = MembershipBody {
        version: 2,
        prev_id: Some(first_id),
        signer_fingerprint: fingerprint_a,
        device_id: b_id,
        ed25519_pk: b_ed,
        kem_public_key: b_kem,
        issued_at: 1_790_000_002_000_000,
        ..first
    };
    let second_bytes = second.encode().unwrap();
    assert_eq!(second_bytes, spec_vector("membership_body_2"));
    let second_id: [u8; 32] = Sha256::digest(&second_bytes).into();
    assert_eq!(second_id.as_slice(), spec_vector("membership_id_2"));
}

#[test]
fn the_epoch_codec_reproduces_the_spec_header_vector() {
    let (_, a_ed, a_kem) = spec_device(0xa1, 0xe1, 0xc1);
    let fingerprint_a = device_fingerprint(SUITE_ID, KEM_ID, &a_ed, &a_kem);
    let genesis = EpochBody {
        suite_id: SUITE_ID,
        kem_id: KEM_ID,
        vault_id: [0x11; 16],
        key_epoch: 1,
        reason: Reason::Genesis,
        membership_head: spec_vector("membership_id_1").try_into().unwrap(),
        prev_epoch_id: None,
        key_commitment_ke: spec_vector("key_commitment_ke").try_into().unwrap(),
        key_commitment_s: spec_vector("key_commitment_s").try_into().unwrap(),
        sealer_fingerprint: fingerprint_a,
        recipients: vec![],
    };
    // The spec's vector is the header: everything before the recipient list.
    let header = spec_vector("epoch_header_genesis");
    let encoded = genesis.encode().unwrap();
    assert_eq!(&encoded[..encoded.len() - 4], header.as_slice());
    assert_eq!(
        &encoded[encoded.len() - 4..],
        &[0, 0, 0, 0],
        "an empty recipient list"
    );
    assert_eq!(EpochBody::decode(&encoded).unwrap(), genesis);
}

/// Bytes that are not an authenticated record can never put a device into the halt class: only a
/// conflicting record that would itself have been valid at its position is a fork or an
/// equivocation.
#[test]
fn unauthenticated_conflicts_never_halt_sync() {
    let doc: Value = serde_json::from_str(VECTORS).unwrap();
    let mut halting = Vec::new();
    for case in doc["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        for step in case["steps"].as_array().unwrap() {
            if step["expect"]["halts_sync"] == true {
                halting.push(name.to_owned());
            }
        }
    }
    halting.sort();
    assert_eq!(
        halting,
        [
            "invalid_epoch_equivocation",
            "invalid_fork",
            "invalid_second_genesis_signed_by_the_founder_is_a_fork"
        ]
    );
}
