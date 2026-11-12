//! Shared builders for the Sync key-custody tests (personal-cfo-1cltt.2): real devices, real
//! membership chains and epoch records sealed with real HPKE, with keys the test knows.

#![allow(dead_code)]

pub mod register;
pub mod snapshot_service;

use sync_core::header::Fingerprint;
use sync_core::membership::{Action, MembershipBody, Reason, SignedRecord, KEM_ID, SUITE_ID};
use vault_crypto::sync::TagSecret;
use vault_crypto::sync_envelope::EpochKey;
use vault_crypto::sync_keys::{epoch_key_commitment, tag_key_commitment, DeviceKeys, DevicePublic};
use vault_crypto::sync_records::{
    seal_epoch_record, sign_epoch, sign_membership, RecipientKeys, SealSpec,
};
use vault_crypto::SecretBytes;

pub const VAULT: [u8; 16] = [0x11; 16];

/// A device with keys the test holds.
pub struct Dev {
    pub keys: DeviceKeys,
    pub id: [u8; 16],
    pub public: DevicePublic,
}

pub fn dev(n: u8) -> Dev {
    let keys = DeviceKeys::generate().expect("device keys");
    let public = keys.public();
    Dev {
        keys,
        id: [n; 16],
        public,
    }
}

impl Dev {
    pub fn as_recipient(&self) -> RecipientKeys {
        RecipientKeys {
            device_id: self.id,
            public: self.public,
        }
    }

    pub fn fingerprint(&self) -> Fingerprint {
        self.public.fingerprint
    }
}

/// A device whose private keys the test chose, so it can search a log for them.
pub fn dev_known(n: u8, signing: u8, agreement: u8) -> Dev {
    let keys = DeviceKeys::from_secrets(
        SecretBytes::new([signing; 32]),
        SecretBytes::new([agreement; 32]),
    )
    .expect("device keys");
    let public = keys.public();
    Dev {
        keys,
        id: [n; 16],
        public,
    }
}

/// `K_e` of `epoch`: 32 bytes derived from the epoch number, so a test can rebuild it.
pub fn epoch_key(epoch: u64) -> EpochKey {
    let raw = SecretBytes::new([epoch as u8 ^ 0x40; 32]);
    let commitment = epoch_key_commitment(&raw);
    EpochKey::from_verified(VAULT, epoch, raw, &commitment).unwrap()
}

/// A different `K_e` for the same epoch (what a malicious sealer might seal to one recipient).
pub fn other_epoch_key(epoch: u64) -> EpochKey {
    let raw = SecretBytes::new([epoch as u8 ^ 0xa0; 32]);
    let commitment = epoch_key_commitment(&raw);
    EpochKey::from_verified(VAULT, epoch, raw, &commitment).unwrap()
}

pub fn tag_secret(byte: u8) -> TagSecret {
    let raw = SecretBytes::new([byte; 32]);
    let commitment = tag_key_commitment(&raw);
    TagSecret::from_verified(raw, &commitment).unwrap()
}

pub fn id_of(record: &[u8]) -> [u8; 32] {
    SignedRecord::parse(record, 1 << 22).unwrap().id()
}

pub fn membership_body(
    version: u64,
    prev: Option<[u8; 32]>,
    action: Action,
    signer: &Dev,
    subject: &Dev,
) -> MembershipBody {
    MembershipBody {
        suite_id: SUITE_ID,
        kem_id: KEM_ID,
        vault_id: VAULT,
        version,
        prev_id: prev,
        action,
        signer_fingerprint: signer.fingerprint(),
        device_id: subject.id,
        ed25519_pk: subject.public.ed25519_pk,
        kem_public_key: subject.public.kem_public_key,
        issued_at: 1_700_000_000_000_000,
    }
}

pub fn membership(
    version: u64,
    prev: Option<[u8; 32]>,
    action: Action,
    signer: &Dev,
    subject: &Dev,
) -> Vec<u8> {
    sign_membership(
        &signer.keys,
        &membership_body(version, prev, action, signer, subject),
    )
    .unwrap()
}

pub fn epoch_record(
    reason: Reason,
    head: [u8; 32],
    prev: Option<[u8; 32]>,
    sealer: &Dev,
    key: &EpochKey,
    tag: &TagSecret,
    recipients: &[&Dev],
) -> Vec<u8> {
    let recipients: Vec<RecipientKeys> = recipients.iter().map(|d| d.as_recipient()).collect();
    seal_epoch_record(&SealSpec {
        reason,
        membership_head: head,
        prev_epoch_id: prev,
        sealer: &sealer.keys,
        epoch_key: key,
        tag_secret: tag,
        recipients: &recipients,
    })
    .unwrap()
}

/// Re-sign a (possibly tampered) epoch body as `signer`, the way a malicious but authorized sealer
/// could, so the failure under test is the one past the signature check.
pub fn resign(signer: &Dev, body: &sync_core::membership::EpochBody) -> Vec<u8> {
    sign_epoch(&signer.keys, body).unwrap()
}

/// The whole sample chain: A founds (epoch 1), adds B (grant), adds C (grant), then revokes C
/// (cutover into epoch 2, sealed to A and B), then a forced rotation (epoch 3, sealed to A and B).
pub struct Chain {
    pub a: Dev,
    pub b: Dev,
    pub c: Dev,
    pub memberships: Vec<Vec<u8>>,
    pub epochs: Vec<Vec<u8>>,
}

pub fn chain() -> Chain {
    let (a, b, c) = (dev(0xa1), dev(0xb1), dev(0xc1));
    let tag = tag_secret(0x77);
    let m1 = membership(1, None, Action::Add, &a, &a);
    let e1 = epoch_record(
        Reason::Genesis,
        id_of(&m1),
        None,
        &a,
        &epoch_key(1),
        &tag,
        &[&a],
    );
    let m2 = membership(2, Some(id_of(&m1)), Action::Add, &a, &b);
    let e2 = epoch_record(
        Reason::Grant,
        id_of(&m2),
        Some(id_of(&e1)),
        &a,
        &epoch_key(1),
        &tag,
        &[&b],
    );
    let m3 = membership(3, Some(id_of(&m2)), Action::Add, &a, &c);
    let e3 = epoch_record(
        Reason::Grant,
        id_of(&m3),
        Some(id_of(&e2)),
        &a,
        &epoch_key(1),
        &tag,
        &[&c],
    );
    let m4 = membership(4, Some(id_of(&m3)), Action::Revoke, &a, &c);
    let e4 = epoch_record(
        Reason::Cutover,
        id_of(&m4),
        Some(id_of(&e3)),
        &a,
        &epoch_key(2),
        &tag,
        &[&a, &b],
    );
    let e5 = epoch_record(
        Reason::Rotation,
        id_of(&m4),
        Some(id_of(&e4)),
        &a,
        &epoch_key(3),
        &tag,
        &[&a, &b],
    );
    Chain {
        a,
        b,
        c,
        memberships: vec![m1, m2, m3, m4],
        epochs: vec![e1, e2, e3, e4, e5],
    }
}

pub fn refs(records: &[Vec<u8>]) -> Vec<&[u8]> {
    records.iter().map(Vec::as_slice).collect()
}

/// The artifact `id` bound to a hash derived from it, as the snapshot tests bind their closures.
pub fn bound(id: [u8; 32]) -> sync_core::envelope::BoundArtifact {
    sync_core::envelope::BoundArtifact {
        id,
        ciphertext_sha256: [id[0] ^ 0xff; 32],
    }
}

/// A valid snapshot plaintext (C.8) with one small table whose file carries `marker`, and the
/// closure `artifacts` (strictly ascending). Distinct markers give distinct plaintexts.
pub fn snapshot_plaintext(artifacts: &[[u8; 32]], marker: &str) -> Vec<u8> {
    let artifacts: Vec<_> = artifacts.iter().map(|id| bound(*id)).collect();
    use sync_core::snapshot::{build, ColumnEntry, Header, TableFile};
    let file = TableFile {
        name: "notes".to_owned(),
        columns: vec![ColumnEntry {
            name: "text".to_owned(),
            kind: "text".to_owned(),
        }],
        row_count: 1,
        state_digest: Some([7; 32]),
        content: format!("{{\"text\":\"{marker}\"}}\n").into_bytes(),
    };
    build(
        &Header {
            schema_version: 65,
            vault_created_at: "2026-01-01T00:00:00Z",
            inventory_version: "1",
            voids: &[],
            artifacts: &artifacts,
        },
        vec![file],
    )
    .expect("a valid snapshot plaintext")
}
