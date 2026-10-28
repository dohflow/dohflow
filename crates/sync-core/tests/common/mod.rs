//! Shared fixtures for the void tests: a two-device chain and signed void records.
#![allow(dead_code)]

use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};
use sync_core::header::{
    ClearHeader, Fingerprint, IdempotencyKeyTag, MetadataCommitment, NonceDomain, ObjectKind,
};
use sync_core::membership::{
    device_fingerprint, verify_chain, Action, EpochAtPosition, EpochBody, MembershipBody, Reason,
    Recipient, SignedRecord, VerifiedState, KEM_ID, SEALED_KEYS_CIPHERTEXT_LEN, SUITE_ID,
};
use sync_core::void::{
    verify_void, Honor, HonorContext, OfferError, VerifiedVoid, VoidBody, VoidLedger, VoidReason,
};

pub const VAULT: [u8; 16] = [0, 0, 0, 0, 0, 0, 0x40, 0, 0x80, 0, 0, 0, 0, 0, 0, 0xf0];

pub struct Dev {
    pub signing: SigningKey,
    pub ed25519_pk: [u8; 32],
    pub kem_pk: [u8; 32],
    pub device_id: [u8; 16],
}

impl Dev {
    pub fn new(n: u8) -> Self {
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
    pub fn fingerprint(&self) -> Fingerprint {
        device_fingerprint(SUITE_ID, KEM_ID, &self.ed25519_pk, &self.kem_pk)
    }
}

pub fn sign(body: &[u8], by: &SigningKey) -> Vec<u8> {
    SignedRecord {
        body: body.to_vec(),
        signature: by.sign(body).to_bytes(),
    }
    .encode()
    .unwrap()
}

pub fn id_of(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

pub fn membership(
    by: &Dev,
    version: u64,
    prev: Option<[u8; 32]>,
    subject: &Dev,
) -> (Vec<u8>, Vec<u8>) {
    let body = MembershipBody {
        suite_id: SUITE_ID,
        kem_id: KEM_ID,
        vault_id: VAULT,
        version,
        prev_id: prev,
        action: Action::Add,
        signer_fingerprint: by.fingerprint(),
        device_id: subject.device_id,
        ed25519_pk: subject.ed25519_pk,
        kem_public_key: subject.kem_pk,
        issued_at: 1_790_000_000_000_000 + version as i64,
    }
    .encode()
    .unwrap();
    (body.clone(), sign(&body, &by.signing))
}

pub fn epoch(
    by: &Dev,
    reason: Reason,
    head: [u8; 32],
    prev: Option<[u8; 32]>,
    to: &Dev,
) -> (Vec<u8>, Vec<u8>) {
    let body = EpochBody {
        suite_id: SUITE_ID,
        kem_id: KEM_ID,
        vault_id: VAULT,
        key_epoch: 1,
        reason,
        membership_head: head,
        prev_epoch_id: prev,
        key_commitment_ke: [1; 32],
        key_commitment_s: [2; 32],
        sealer_fingerprint: by.fingerprint(),
        recipients: vec![Recipient {
            device_id: to.device_id,
            fingerprint: to.fingerprint(),
            suite_id: SUITE_ID,
            kem_id: KEM_ID,
            enc: [0xee; 32],
            ciphertext: vec![0xcc; SEALED_KEYS_CIPHERTEXT_LEN],
        }],
    }
    .encode()
    .unwrap();
    (body.clone(), sign(&body, &by.signing))
}

/// A founds the vault, then adds B at membership version 2 (a grant epoch record).
pub struct Chain {
    pub a: Dev,
    pub b: Dev,
    pub state: VerifiedState,
    /// The epoch record id before B was added (membership version 1).
    pub e1: [u8; 32],
    /// The epoch record id that installs B (membership version 2).
    pub e2: [u8; 32],
}

pub fn chain() -> Chain {
    let (a, b) = (Dev::new(1), Dev::new(2));
    let (m1_body, m1) = membership(&a, 1, None, &a);
    let (e1_body, e1) = epoch(&a, Reason::Genesis, id_of(&m1_body), None, &a);
    let (m2_body, m2) = membership(&a, 2, Some(id_of(&m1_body)), &b);
    let (e2_body, e2) = epoch(
        &a,
        Reason::Grant,
        id_of(&m2_body),
        Some(id_of(&e1_body)),
        &b,
    );
    let state = verify_chain(&VAULT, &[&m1, &m2], &[&e1, &e2]).expect("a valid chain");
    Chain {
        a,
        b,
        state,
        e1: id_of(&e1_body),
        e2: id_of(&e2_body),
    }
}

pub fn body(by: &Dev, seq: u64) -> VoidBody {
    VoidBody {
        suite_id: SUITE_ID,
        vault_id: VAULT,
        seq,
        command_id: [0x11; 16],
        envelope_digest: [0x22; 32],
        reason: VoidReason::ReceiverCheck,
        signer_fingerprint: by.fingerprint(),
        issued_at: 1_790_000_000_000_000,
    }
}

pub fn signed(b: &VoidBody, by: &Dev) -> Vec<u8> {
    sign(&b.encode(), &by.signing)
}

pub fn at_end(c: &Chain) -> EpochAtPosition {
    EpochAtPosition::after_epoch_record(&c.state, &c.e2).unwrap()
}

/// A void by A for `seq`, pulled at log position `position`.
pub fn void_at(c: &Chain, seq: u64, position: u64) -> VerifiedVoid {
    verify_void(
        &c.state,
        &at_end(c),
        position,
        &signed(&body(&c.a, seq), &c.a),
    )
    .unwrap()
}

/// Offer one void; the tests that are about the context or the order use `offer_in_order`.
pub trait OfferOne {
    fn offer(&mut self, void: &VerifiedVoid, ctx: &HonorContext<'_>) -> Honor;
    fn try_offer(
        &mut self,
        void: &VerifiedVoid,
        ctx: &HonorContext<'_>,
    ) -> Result<Honor, OfferError>;
}

impl OfferOne for VoidLedger {
    fn offer(&mut self, void: &VerifiedVoid, ctx: &HonorContext<'_>) -> Honor {
        self.try_offer(void, ctx).expect("a consistent context")
    }

    fn try_offer(
        &mut self,
        void: &VerifiedVoid,
        ctx: &HonorContext<'_>,
    ) -> Result<Honor, OfferError> {
        let outcomes = self.offer_in_order(std::slice::from_ref(void), ctx)?;
        Ok(outcomes[0])
    }
}

/// A coherent-looking envelope header; the status machine only reads its identifiers.
pub fn sample_header() -> ClearHeader {
    let dev = Dev::new(1);
    let fingerprint = dev.fingerprint();
    ClearHeader {
        vault_id: VAULT,
        key_epoch: 1,
        object: ObjectKind::Envelope {
            idempotency_key_tag: IdempotencyKeyTag::from_derived([1; 32]),
            immutable_metadata_commitment: MetadataCommitment::from_derived([2; 32]),
            artifact_manifest_commitment: None,
        },
        object_id: [0x11; 16],
        device_id: dev.device_id,
        device_key_fingerprint: fingerprint,
        nonce_domain: NonceDomain::derive(&VAULT, 1, &fingerprint, 1),
        invocation_counter: 0,
        base_seq: 1,
        payload_schema_version: 1,
        engine_version: "dohflow-engine-1".to_owned(),
        schema_version: 55,
    }
}
