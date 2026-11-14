//! Sealing and opening a Sync snapshot (vault-format.md C.3, C.4 and C.8; personal-cfo-1cltt.3):
//! the receiver path, fail-closed on a wrong key or AAD or object type, distinct keys for devices
//! with equal counters, the counter reservation, the saved ciphertext and the changed retry, the
//! crash points, the cap, and the guard on the opener.

mod common;

use common::register::MemoryRegisterStore;
use common::{chain, epoch_key, Chain, VAULT};
use platform_mock::MockWitness;
use platform_traits::{CounterContext, WitnessError};
use sync_core::envelope::ArtifactManifest;
use sync_core::header::{
    encode_container, parse_container, ClearHeader, IdempotencyKeyTag, ManifestCommitment,
    MetadataCommitment, NonceDomain, ObjectKind, OBJECT_ENVELOPE, OBJECT_SNAPSHOT,
};
use sync_core::membership::{verify_chain, VerifiedState};
use vault_crypto::sync_counter::{CounterRegister, LostReason, NonceError, RegisterStore};
use vault_crypto::sync_envelope::{
    aead_key, seal_envelope, EpochKey, ObjectType, ReceiverOpener, SealerIdentity,
};
use vault_crypto::sync_snapshot::{
    seal_snapshot, SavedSnapshotFiles, SnapshotError, SnapshotFields, SnapshotOpenFailure,
    SnapshotOpener, SnapshotSealError, SnapshotSealer, MAX_SNAPSHOT_PLAINTEXT,
};

/// The sample snapshot plaintext: valid, with an empty closure.
static SAMPLE: std::sync::LazyLock<Vec<u8>> =
    std::sync::LazyLock::new(|| common::snapshot_plaintext(&[], "the sample snapshot"));
fn sample() -> &'static [u8] {
    SAMPLE.as_slice()
}

/// A valid snapshot plaintext with its own marker.
fn pt(marker: &str) -> Vec<u8> {
    common::snapshot_plaintext(&[], marker)
}

/// A temporary directory that is removed when dropped.
struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "dohflow-sync-snapshot-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn state(c: &Chain) -> VerifiedState {
    let m: Vec<&[u8]> = c.memberships[..3].iter().map(Vec::as_slice).collect();
    let e: Vec<&[u8]> = c.epochs[..3].iter().map(Vec::as_slice).collect();
    verify_chain(&VAULT, &m, &e).expect("epoch 1 with devices A, B and C")
}

fn identity(d: &common::Dev) -> SealerIdentity {
    SealerIdentity {
        vault_id: VAULT,
        device_id: d.id,
        fingerprint: d.fingerprint(),
    }
}

fn empty_closure() -> ManifestCommitment {
    ArtifactManifest::new(Vec::new()).unwrap().commitment()
}

fn empty_binding() -> sync_core::header::BindingCommitment {
    sync_core::envelope::binding_commitment(&[], 1 << 20).unwrap()
}

fn fields(object_id: u8, base_seq: u64) -> SnapshotFields {
    SnapshotFields {
        object_id: [object_id; 16],
        base_seq,
        payload_schema_version: 1,
        engine_version: "dohflow-test".to_owned(),
        schema_version: 65,
    }
}

fn context(key: &EpochKey, d: &common::Dev, object_type: u8) -> CounterContext {
    CounterContext {
        vault_id: VAULT,
        key_epoch: key.key_epoch(),
        fingerprint: *d.fingerprint().as_bytes(),
        object_type,
    }
}

fn open(
    key: &EpochKey,
    chain: &VerifiedState,
    bytes: &[u8],
) -> Result<Vec<u8>, SnapshotOpenFailure> {
    let container = parse_container(bytes).expect("parses");
    SnapshotOpener::new(key, chain).open(&container)
}

fn sealed(c: &Chain, counter_ctx_dev: &common::Dev, plaintext: &[u8]) -> (Vec<u8>, EpochKey) {
    let key = epoch_key(1);
    let (store, witness) = (MemoryRegisterStore::new(), MockWitness::new());
    let register = CounterRegister::new(&store, &witness);
    let reservation = register
        .reserve(&context(&key, counter_ctx_dev, 2))
        .unwrap();
    let bytes = seal_snapshot(
        &key,
        &identity(counter_ctx_dev),
        &fields(0x33, 41),
        reservation,
        plaintext,
    )
    .unwrap();
    let _ = c;
    (bytes, key)
}

// ---- seal and open -----------------------------------------------------------------------------

#[test]
fn a_snapshot_seals_and_opens_through_the_receiver_path() {
    let c = chain();
    let chain_state = state(&c);
    let (bytes, key) = sealed(&c, &c.a, sample());
    let container = parse_container(&bytes).unwrap();
    let header = &container.header;
    assert_eq!(header.object_type(), OBJECT_SNAPSHOT);
    assert_eq!(header.object_id, [0x33; 16]);
    assert_eq!(header.base_seq, 41);
    assert_eq!(header.invocation_counter, 0);
    assert_eq!(header.device_id, c.a.id);
    assert_eq!(header.device_key_fingerprint, c.a.fingerprint());
    assert_eq!(
        header.nonce_domain,
        NonceDomain::derive(&VAULT, 1, &c.a.fingerprint(), OBJECT_SNAPSHOT)
    );
    // Field 17 is present, even for the empty closure.
    assert_eq!(
        header.object,
        ObjectKind::Snapshot {
            artifact_manifest_commitment: empty_closure(),
            artifact_binding_commitment: empty_binding(),
        }
    );
    assert_eq!(open(&key, &chain_state, &bytes).unwrap(), sample());
}

#[test]
fn a_wrong_key_an_aad_mismatch_and_tampering_fail_closed() {
    let c = chain();
    let chain_state = state(&c);
    let (bytes, key) = sealed(&c, &c.a, sample());
    // A wrong key for the same epoch.
    let wrong = common::other_epoch_key(1);
    assert_eq!(
        open(&wrong, &chain_state, &bytes),
        Err(SnapshotOpenFailure::NotOpenable)
    );
    // Another epoch's key, and another vault's.
    assert_eq!(
        open(&epoch_key(2), &chain_state, &bytes),
        Err(SnapshotOpenFailure::WrongEpoch)
    );
    // An AAD mismatch: a header field the receiver does not itself recompute (base_seq) changes,
    // and the ciphertext no longer authenticates.
    let container = parse_container(&bytes).unwrap();
    let mut header = container.header.clone();
    header.base_seq += 1;
    let forged = encode_container(&header, container.ciphertext).unwrap();
    assert_eq!(
        open(&key, &chain_state, &forged),
        Err(SnapshotOpenFailure::NotOpenable)
    );
    // And so does every flipped ciphertext or tag byte.
    for at in [
        0,
        container.ciphertext.len() / 2,
        container.ciphertext.len() - 1,
    ] {
        let mut ct = container.ciphertext.to_vec();
        ct[at] ^= 1;
        let tampered = encode_container(&container.header, &ct).unwrap();
        assert_eq!(
            open(&key, &chain_state, &tampered),
            Err(SnapshotOpenFailure::NotOpenable),
            "byte {at}"
        );
    }
}

#[test]
fn an_envelope_is_not_a_snapshot_and_a_snapshot_is_not_an_envelope() {
    let c = chain();
    let chain_state = state(&c);
    let (snapshot, key) = sealed(&c, &c.a, sample());
    // The envelope opener refuses a snapshot.
    let parsed = parse_container(&snapshot).unwrap();
    assert!(sync_core::producer::EnvelopeOpener::open(
        &ReceiverOpener::for_producer(&key, &chain_state),
        &parsed
    )
    .is_none());
    // An envelope sealed by the same device under the same epoch key.
    let a = identity(&c.a);
    let header = ClearHeader {
        vault_id: VAULT,
        key_epoch: 1,
        object: ObjectKind::Envelope {
            idempotency_key_tag: IdempotencyKeyTag::from_derived([1; 32]),
            immutable_metadata_commitment: MetadataCommitment::from_derived([2; 32]),
            artifact_manifest_commitment: None,
            artifact_binding_commitment: None,
        },
        object_id: [0x44; 16],
        device_id: a.device_id,
        device_key_fingerprint: a.fingerprint,
        nonce_domain: NonceDomain::derive(&VAULT, 1, &a.fingerprint, OBJECT_ENVELOPE),
        invocation_counter: 0,
        base_seq: 0,
        payload_schema_version: 1,
        engine_version: "dohflow-test".to_owned(),
        schema_version: 65,
    };
    let envelope = seal_envelope(&key, &a, &header, b"an envelope plaintext").unwrap();
    assert_eq!(
        open(&key, &chain_state, &envelope),
        Err(SnapshotOpenFailure::NotASnapshot)
    );
    // The two object types never share a key.
    assert_ne!(
        aead_key(&key, &a.fingerprint, ObjectType::Envelope).expose(),
        aead_key(&key, &a.fingerprint, ObjectType::Snapshot).expose()
    );
}

#[test]
fn devices_with_equal_counters_get_distinct_keys_and_domains_and_a_claimed_domain_is_refused() {
    let c = chain();
    let chain_state = state(&c);
    let key = epoch_key(1);
    // Two devices, both at counter 0 in a hostile world where the service offers each the other's
    // nonce domain: the derived keys and domains still differ.
    let (a, b) = (identity(&c.a), identity(&c.b));
    assert_ne!(
        aead_key(&key, &a.fingerprint, ObjectType::Snapshot).expose(),
        aead_key(&key, &b.fingerprint, ObjectType::Snapshot).expose()
    );
    let domain =
        |d: &SealerIdentity| NonceDomain::derive(&VAULT, 1, &d.fingerprint, OBJECT_SNAPSHOT);
    assert_ne!(domain(&a), domain(&b));
    // Both seal their own counter 0 fine, and neither opens as the other.
    let seal = |d: &common::Dev| {
        let (store, witness) = (MemoryRegisterStore::new(), MockWitness::new());
        let register = CounterRegister::new(&store, &witness);
        let r = register.reserve(&context(&key, d, 2)).unwrap();
        seal_snapshot(&key, &identity(d), &fields(0x33, 41), r, sample()).unwrap()
    };
    let (sa, sb) = (seal(&c.a), seal(&c.b));
    assert_eq!(open(&key, &chain_state, &sa).unwrap(), sample());
    assert_eq!(open(&key, &chain_state, &sb).unwrap(), sample());
    assert_ne!(
        parse_container(&sa).unwrap().ciphertext,
        parse_container(&sb).unwrap().ciphertext
    );
    // A header that claims the other device's nonce domain is refused by the receiver before the
    // AEAD is tried (stage 5).
    let container = parse_container(&sa).unwrap();
    let mut claimed = container.header.clone();
    claimed.nonce_domain = domain(&b);
    let forged = encode_container(&claimed, container.ciphertext).unwrap();
    assert_eq!(
        open(&key, &chain_state, &forged),
        Err(SnapshotOpenFailure::NonceDomain)
    );
    // A header that claims the other device's id while keeping A's fingerprint is an identity
    // mismatch.
    let mut renamed = container.header.clone();
    renamed.device_id = c.b.id;
    let forged = encode_container(&renamed, container.ciphertext).unwrap();
    assert_eq!(
        open(&key, &chain_state, &forged),
        Err(SnapshotOpenFailure::IdentityMismatch)
    );
}

#[test]
fn a_device_outside_the_epoch_does_not_open() {
    let c = chain();
    let (bytes, key) = sealed(&c, &c.a, sample());
    // A chain that holds only the first two records: device C is not a member of it, A is. Seal as
    // C against the full chain, then open against the short one.
    let full = state(&c);
    let (store, witness) = (MemoryRegisterStore::new(), MockWitness::new());
    let register = CounterRegister::new(&store, &witness);
    let r = register.reserve(&context(&key, &c.c, 2)).unwrap();
    let by_c = seal_snapshot(&key, &identity(&c.c), &fields(0x34, 1), r, sample()).unwrap();
    let m: Vec<&[u8]> = c.memberships[..2].iter().map(Vec::as_slice).collect();
    let e: Vec<&[u8]> = c.epochs[..2].iter().map(Vec::as_slice).collect();
    let short = verify_chain(&VAULT, &m, &e).unwrap();
    assert_eq!(open(&key, &full, &by_c).unwrap(), sample());
    assert_eq!(
        open(&key, &short, &by_c),
        Err(SnapshotOpenFailure::DeviceOutsideEpoch)
    );
    assert!(open(&key, &short, &bytes).is_ok());
}

// ---- header field 17 and the manifest's closure -------------------------------------------------

/// Seal `plaintext` under a header whose field 17 is `commitment`, bypassing the checks that
/// `seal_snapshot` makes: a producer that is buggy or hostile, for the receiver to refuse.
fn hand_sealed(
    key: &EpochKey,
    d: &common::Dev,
    commitment: ManifestCommitment,
    plaintext: &[u8],
) -> Vec<u8> {
    // The binding is the plaintext's own, so a test about the manifest commitment has one defect.
    let binding = vault_crypto::sync_snapshot::closure_commitments_of(plaintext)
        .map_or_else(|_| empty_binding(), |c| c.binding);
    hand_sealed_with(key, d, commitment, binding, plaintext)
}

/// As [`hand_sealed`], with both header commitments chosen by the caller.
fn hand_sealed_with(
    key: &EpochKey,
    d: &common::Dev,
    commitment: ManifestCommitment,
    binding: sync_core::header::BindingCommitment,
    plaintext: &[u8],
) -> Vec<u8> {
    use aes_gcm::aead::{Aead, KeyInit, Payload};
    use aes_gcm::{Aes256Gcm, Key, Nonce};
    let id = identity(d);
    let domain = NonceDomain::derive(&VAULT, 1, &id.fingerprint, OBJECT_SNAPSHOT);
    let header = ClearHeader {
        vault_id: VAULT,
        key_epoch: 1,
        object: ObjectKind::Snapshot {
            artifact_manifest_commitment: commitment,
            artifact_binding_commitment: binding,
        },
        object_id: [0x55; 16],
        device_id: id.device_id,
        device_key_fingerprint: id.fingerprint,
        nonce_domain: domain,
        invocation_counter: 7,
        base_seq: 0,
        payload_schema_version: 1,
        engine_version: "dohflow-test".to_owned(),
        schema_version: 65,
    };
    let aad = header.encode().unwrap();
    let aead = aead_key(key, &id.fingerprint, ObjectType::Snapshot);
    let mut nonce = [0_u8; 12];
    nonce[..4].copy_from_slice(domain.as_bytes());
    nonce[4..].copy_from_slice(&7_u64.to_be_bytes());
    let ciphertext = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(aead.expose()))
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad: &aad,
            },
        )
        .unwrap();
    encode_container(&header, &ciphertext).unwrap()
}

#[test]
fn header_field_17_is_the_commitment_of_the_closure_the_manifest_names() {
    let c = chain();
    let chain_state = state(&c);
    let key = epoch_key(1);
    let (a, b) = ([0x0a; 32], [0xb0; 32]);
    let with_closure = common::snapshot_plaintext(&[a, b], "two artifacts");
    let (store, witness) = (MemoryRegisterStore::new(), MockWitness::new());
    let register = CounterRegister::new(&store, &witness);
    let seal = |plaintext: &[u8]| {
        let r = register.reserve(&context(&key, &c.a, 2)).unwrap();
        seal_snapshot(&key, &identity(&c.a), &fields(0x33, 41), r, plaintext).unwrap()
    };
    let sealed = seal(&with_closure);
    let header = parse_container(&sealed).unwrap().header;
    let expected = ArtifactManifest::new(vec![a, b]).unwrap().commitment();
    assert_eq!(
        header.object,
        ObjectKind::Snapshot {
            artifact_manifest_commitment: expected,
            artifact_binding_commitment: sync_core::envelope::binding_commitment(
                &[
                    common::bound(a).ciphertext_sha256,
                    common::bound(b).ciphertext_sha256
                ],
                1 << 20
            )
            .unwrap(),
        }
    );
    assert_ne!(
        expected,
        empty_closure(),
        "a non-empty closure commits to something else"
    );
    assert_eq!(
        vault_crypto::sync_snapshot::closure_commitment_of(&with_closure),
        Ok(expected)
    );
    assert_eq!(open(&key, &chain_state, &sealed).unwrap(), with_closure);
    // The empty closure is committed to as well: the field is never absent.
    let empty = seal(&common::snapshot_plaintext(&[], "no artifacts"));
    assert_eq!(
        parse_container(&empty).unwrap().header.object,
        ObjectKind::Snapshot {
            artifact_manifest_commitment: empty_closure(),
            artifact_binding_commitment: empty_binding(),
        }
    );
}

#[test]
fn the_receiver_refuses_a_header_whose_binding_commitment_is_not_the_manifests() {
    use sync_core::envelope::binding_commitment;
    use sync_core::header::BindingCommitment;
    let c = chain();
    let chain_state = state(&c);
    let key = epoch_key(1);
    let plaintext = common::snapshot_plaintext(&[[0x0a; 32], [0xb0; 32]], "two artifacts");
    let right_manifest = ArtifactManifest::new(vec![[0x0a; 32], [0xb0; 32]])
        .unwrap()
        .commitment();
    let hash_a = common::bound([0x0a; 32]).ciphertext_sha256;
    let hash_b = common::bound([0xb0; 32]).ciphertext_sha256;
    let right_binding = binding_commitment(&[hash_a, hash_b], 1 << 20).unwrap();

    // Field 17 is right and field 18 is not: another hash, the hashes in the wrong order, one hash
    // missing, the empty list's, and an arbitrary value. Each is deterministically invalid.
    for wrong in [
        binding_commitment(&[hash_a, [0x99; 32]], 1 << 20).unwrap(),
        binding_commitment(&[hash_b, hash_a], 1 << 20).unwrap(),
        binding_commitment(&[hash_a], 1 << 20).unwrap(),
        empty_binding(),
        BindingCommitment::from_derived([0; 32]),
    ] {
        assert_ne!(wrong, right_binding);
        let forged = hand_sealed_with(&key, &c.a, right_manifest, wrong, &plaintext);
        assert_eq!(
            open(&key, &chain_state, &forged),
            Err(SnapshotOpenFailure::BindingCommitmentMismatch)
        );
    }

    // Both fields wrong: field 17 is judged first.
    let forged = hand_sealed_with(
        &key,
        &c.a,
        ManifestCommitment::from_derived([0; 32]),
        BindingCommitment::from_derived([0; 32]),
        &plaintext,
    );
    assert_eq!(
        open(&key, &chain_state, &forged),
        Err(SnapshotOpenFailure::ClosureCommitmentMismatch)
    );

    // Both right: the helper is a faithful sealer.
    let honest = hand_sealed_with(&key, &c.a, right_manifest, right_binding, &plaintext);
    assert_eq!(open(&key, &chain_state, &honest).unwrap(), plaintext);

    // The empty closure binds to the empty list, and a binding for artifacts it does not name is
    // refused.
    let empty_plaintext = common::snapshot_plaintext(&[], "no artifacts");
    let forged = hand_sealed_with(
        &key,
        &c.a,
        empty_closure(),
        binding_commitment(&[hash_a], 1 << 20).unwrap(),
        &empty_plaintext,
    );
    assert_eq!(
        open(&key, &chain_state, &forged),
        Err(SnapshotOpenFailure::BindingCommitmentMismatch)
    );
}

#[test]
fn the_receiver_refuses_a_header_whose_commitment_is_not_the_manifests() {
    let c = chain();
    let chain_state = state(&c);
    let key = epoch_key(1);
    let plaintext = common::snapshot_plaintext(&[[0x0a; 32]], "one artifact");
    // The commitment of a different closure, and of the empty one.
    for wrong in [
        ArtifactManifest::new(vec![[0x0b; 32]])
            .unwrap()
            .commitment(),
        empty_closure(),
        ManifestCommitment::from_derived([0; 32]),
    ] {
        let forged = hand_sealed(&key, &c.a, wrong, &plaintext);
        assert_eq!(
            open(&key, &chain_state, &forged),
            Err(SnapshotOpenFailure::ClosureCommitmentMismatch)
        );
    }
    // The right commitment opens (the helper is a faithful sealer).
    let right = ArtifactManifest::new(vec![[0x0a; 32]])
        .unwrap()
        .commitment();
    let honest = hand_sealed(&key, &c.a, right, &plaintext);
    assert_eq!(open(&key, &chain_state, &honest).unwrap(), plaintext);
}

#[test]
fn the_receiver_refuses_an_authentic_plaintext_that_is_not_a_snapshot() {
    let c = chain();
    let chain_state = state(&c);
    let key = epoch_key(1);
    let not_a_snapshot = hand_sealed(&key, &c.a, empty_closure(), b"authentic but not a snapshot");
    assert_eq!(
        open(&key, &chain_state, &not_a_snapshot),
        Err(SnapshotOpenFailure::PlaintextInvalid)
    );
    // A snapshot whose file no longer matches its manifest (a flipped byte) is not valid either.
    let mut broken = common::snapshot_plaintext(&[], "a file");
    let at = broken.len() - 3;
    broken[at] ^= 1;
    let forged = hand_sealed(&key, &c.a, empty_closure(), &broken);
    assert_eq!(
        open(&key, &chain_state, &forged),
        Err(SnapshotOpenFailure::PlaintextInvalid)
    );
}

#[test]
fn a_plaintext_that_is_not_a_valid_snapshot_is_refused_before_a_counter_is_spent() {
    let rig = Rig::new("invalid-plaintext");
    assert_eq!(
        rig.seal(&fields(0x33, 41), b"not a snapshot plaintext")
            .unwrap_err(),
        SnapshotError::Seal(SnapshotSealError::PlaintextInvalid)
    );
    assert_eq!(rig.store.commits(), 0, "no counter was reserved");
    assert!(!rig.file(0x33).exists());
    assert_eq!(rig.store.saved(&rig.ctx()), Ok(None));
    // The low-level seal refuses it as well, after the reservation it was handed.
    let r = CounterRegister::new(&rig.store, &rig.witness)
        .reserve(&rig.ctx())
        .unwrap();
    assert_eq!(
        seal_snapshot(&rig.key, &identity(&rig.c.a), &fields(0x33, 41), r, b"nope").unwrap_err(),
        SnapshotSealError::PlaintextInvalid
    );
}

// ---- the reservation ---------------------------------------------------------------------------

#[test]
fn a_reservation_for_another_context_is_refused_before_anything_is_encrypted() {
    let c = chain();
    let key = epoch_key(1);
    let (store, witness) = (MemoryRegisterStore::new(), MockWitness::new());
    let register = CounterRegister::new(&store, &witness);
    let seal_with = |ctx: CounterContext| {
        let r = register.reserve(&ctx).unwrap();
        seal_snapshot(&key, &identity(&c.a), &fields(0x33, 41), r, sample())
    };
    assert_eq!(
        seal_with(context(&key, &c.b, 2)).unwrap_err(),
        SnapshotSealError::ReservationMismatch,
        "another device"
    );
    assert_eq!(
        seal_with(context(&epoch_key(2), &c.a, 2)).unwrap_err(),
        SnapshotSealError::ReservationMismatch,
        "another epoch"
    );
    assert_eq!(
        seal_with(context(&key, &c.a, 1)).unwrap_err(),
        SnapshotSealError::ReservationMismatch,
        "the envelope context"
    );
    // The key and the sealer must be one vault's.
    let mut other_vault = identity(&c.a);
    other_vault.vault_id = [9; 16];
    let r = register.reserve(&context(&key, &c.a, 2)).unwrap();
    assert_eq!(
        seal_snapshot(&key, &other_vault, &fields(0x33, 41), r, sample()).unwrap_err(),
        SnapshotSealError::WrongVault
    );
}

#[test]
fn a_plaintext_over_a_gibibyte_is_snapshot_too_large_and_spends_no_counter() {
    let c = chain();
    let chain_state = state(&c);
    let key = epoch_key(1);
    // Untouched zero pages: the size check must come before any read of the bytes.
    let huge = vec![0_u8; MAX_SNAPSHOT_PLAINTEXT + 1];
    let (store, witness) = (MemoryRegisterStore::new(), MockWitness::new());
    let register = CounterRegister::new(&store, &witness);
    let dir = TempDir::new("toolarge");
    let files = SavedSnapshotFiles::new(&dir.0);
    let sealer = identity(&c.a);
    let publish = SnapshotSealer {
        register: &register,
        files: &files,
        key: &key,
        sealer: &sealer,
        chain: &chain_state,
    };
    assert_eq!(
        publish.seal(&fields(0x33, 41), &huge).unwrap_err(),
        SnapshotError::Seal(SnapshotSealError::SnapshotTooLarge)
    );
    assert_eq!(store.commits(), 0, "no counter was reserved");
    let r = register.reserve(&context(&key, &c.a, 2)).unwrap();
    assert_eq!(
        seal_snapshot(&key, &sealer, &fields(0x33, 41), r, &huge).unwrap_err(),
        SnapshotSealError::SnapshotTooLarge
    );
}

// ---- the publish path --------------------------------------------------------------------------

struct Rig {
    c: Chain,
    chain_state: VerifiedState,
    key: EpochKey,
    store: MemoryRegisterStore,
    witness: MockWitness,
    dir: TempDir,
}

impl Rig {
    fn new(tag: &str) -> Self {
        let c = chain();
        let chain_state = state(&c);
        Self {
            c,
            chain_state,
            key: epoch_key(1),
            store: MemoryRegisterStore::new(),
            witness: MockWitness::new(),
            dir: TempDir::new(tag),
        }
    }

    fn seal(
        &self,
        fields: &SnapshotFields,
        plaintext: &[u8],
    ) -> Result<vault_crypto::sync_snapshot::SealedSnapshot, SnapshotError> {
        let register = CounterRegister::new(&self.store, &self.witness);
        let files = SavedSnapshotFiles::new(&self.dir.0);
        let sealer = identity(&self.c.a);
        SnapshotSealer {
            register: &register,
            files: &files,
            key: &self.key,
            sealer: &sealer,
            chain: &self.chain_state,
        }
        .seal(fields, plaintext)
    }

    fn ctx(&self) -> CounterContext {
        context(&self.key, &self.c.a, 2)
    }

    fn file(&self, object_id: u8) -> std::path::PathBuf {
        let name: String = [object_id; 16].iter().map(|b| format!("{b:02x}")).collect();
        self.dir.0.join(format!("{name}.snapshot"))
    }
}

#[test]
fn an_exact_retransmission_reuses_the_saved_ciphertext_and_burns_no_counter() {
    let rig = Rig::new("reuse");
    let first = rig.seal(&fields(0x33, 41), sample()).unwrap();
    assert!(!first.reused);
    assert_eq!(first.counter, 0);
    let commits = rig.store.commits();
    let witnessed = rig.witness.counter_advances();
    // A transport retry of the same publish: the same bytes, no counter.
    let again = rig.seal(&fields(0x33, 41), sample()).unwrap();
    assert!(again.reused);
    assert_eq!(again.container, first.container);
    assert_eq!(again.counter, 0);
    assert_eq!(rig.store.commits(), commits);
    assert_eq!(rig.witness.counter_advances(), witnessed);
    assert_eq!(rig.store.next_counter(&rig.ctx()), Ok(Some(1)));
    // The saved container opens through the receiver path.
    assert_eq!(
        open(&rig.key, &rig.chain_state, &again.container).unwrap(),
        sample()
    );
}

#[test]
fn a_changed_retry_burns_a_counter_and_replaces_the_saved_container() {
    let rig = Rig::new("changed");
    let first = rig.seal(&fields(0x33, 41), sample()).unwrap();
    // A retry after a failed compare-and-swap is a new head and a new snapshot.
    let new_head = rig
        .seal(&fields(0x34, 57), &pt("a newer plaintext"))
        .unwrap();
    assert!(!new_head.reused);
    assert_eq!(new_head.counter, 1);
    assert_ne!(new_head.container, first.container);
    assert!(!rig.file(0x33).exists(), "the superseded file is gone");
    assert!(rig.file(0x34).exists());
    // The same snapshot id with a different base, or different content, is not the same publish.
    let rebased = rig
        .seal(&fields(0x34, 58), &pt("a newer plaintext"))
        .unwrap();
    assert!(!rebased.reused);
    assert_eq!(rebased.counter, 2);
    let edited = rig
        .seal(&fields(0x34, 58), &pt("an edited plaintext"))
        .unwrap();
    assert!(!edited.reused);
    assert_eq!(edited.counter, 3);
    // The latest publish is the one that is saved.
    assert!(
        rig.seal(&fields(0x34, 58), &pt("an edited plaintext"))
            .unwrap()
            .reused
    );
    // Every counter used was distinct.
    assert_eq!(rig.store.next_counter(&rig.ctx()), Ok(Some(4)));
}

#[test]
fn a_missing_or_damaged_saved_file_is_not_reused() {
    let rig = Rig::new("damaged");
    let first = rig.seal(&fields(0x33, 41), sample()).unwrap();
    // Damaged: one byte flipped on disk.
    let mut bytes = std::fs::read(rig.file(0x33)).unwrap();
    let mid = bytes.len() / 2;
    bytes[mid] ^= 1;
    std::fs::write(rig.file(0x33), &bytes).unwrap();
    let second = rig.seal(&fields(0x33, 41), sample()).unwrap();
    assert!(!second.reused);
    assert_eq!(second.counter, 1);
    assert_ne!(second.container, first.container);
    // Missing.
    std::fs::remove_file(rig.file(0x33)).unwrap();
    let third = rig.seal(&fields(0x33, 41), sample()).unwrap();
    assert!(!third.reused);
    assert_eq!(third.counter, 2);
}

#[test]
fn the_saved_record_and_file_survive_a_reopen_and_are_dropped_on_the_receipt() {
    let rig = Rig::new("receipt");
    rig.seal(&fields(0x33, 41), sample()).unwrap();
    // "Reopen": new handles on the same store, witness and directory, the same chain and keys.
    let (store, witness) = (rig.store.clone(), rig.witness.clone());
    let register = CounterRegister::new(&store, &witness);
    let files = SavedSnapshotFiles::new(&rig.dir.0);
    let sealer = identity(&rig.c.a);
    let publish = SnapshotSealer {
        register: &register,
        files: &files,
        key: &rig.key,
        sealer: &sealer,
        chain: &rig.chain_state,
    };
    let again = publish.seal(&fields(0x33, 41), sample()).unwrap();
    assert!(again.reused);
    // The publish is durable: forget it.
    publish.discard(&[0x33; 16]).unwrap();
    assert_eq!(rig.store.saved(&rig.ctx()), Ok(None));
    assert!(!rig.file(0x33).exists());
    // Discarding twice is not an error, and sealing again is a new seal.
    publish.discard(&[0x33; 16]).unwrap();
    let after = publish.seal(&fields(0x33, 41), sample()).unwrap();
    assert!(!after.reused);
    assert_eq!(after.counter, 1);
}

#[test]
fn a_crash_between_the_file_and_its_record_burns_the_counter_and_reseals() {
    let rig = Rig::new("crash-record");
    rig.store.fail_next_set_saved(1);
    assert!(matches!(
        rig.seal(&fields(0x33, 41), sample()).unwrap_err(),
        SnapshotError::Register(_)
    ));
    assert_eq!(
        rig.store.next_counter(&rig.ctx()),
        Ok(Some(1)),
        "counter 0 is burned"
    );
    assert_eq!(rig.store.saved(&rig.ctx()), Ok(None), "nothing is recorded");
    let retry = rig.seal(&fields(0x33, 41), sample()).unwrap();
    assert!(!retry.reused);
    assert_eq!(retry.counter, 1, "counter 0 is never reused");
}

#[test]
fn a_witness_failure_after_the_register_commit_burns_the_counter_and_publishes_nothing() {
    let rig = Rig::new("crash-reserve");
    let first = rig.seal(&fields(0x33, 41), sample()).unwrap();
    assert_eq!(first.counter, 0);
    // The register commits counter 1 and the witness then fails: counter 1 is spent, never sealed.
    rig.witness.fail_next_counter_advances(1);
    assert_eq!(
        rig.seal(&fields(0x34, 57), &pt("second")).unwrap_err(),
        SnapshotError::Nonce(NonceError::Witness(WitnessError::Unavailable))
    );
    assert!(!rig.file(0x34).exists(), "nothing was sealed or saved");
    assert_eq!(rig.store.next_counter(&rig.ctx()), Ok(Some(2)));
    // The first publish is still saved and can still be sent again, at no cost.
    assert!(rig.file(0x33).exists());
    assert!(rig.seal(&fields(0x33, 41), sample()).unwrap().reused);
    // The next new publish uses counter 2: counter 1 is never handed out.
    let next = rig.seal(&fields(0x34, 57), &pt("second")).unwrap();
    assert_eq!(next.counter, 2);
}

#[test]
fn seal_then_verify_refuses_a_snapshot_the_receiver_path_would_refuse() {
    let rig = Rig::new("verify");
    // A chain in which the sealing device is not a member of the epoch: what this device sealed is
    // not something any replica would open, so nothing is saved or published.
    let m: Vec<&[u8]> = rig.c.memberships[..1].iter().map(Vec::as_slice).collect();
    let e: Vec<&[u8]> = rig.c.epochs[..1].iter().map(Vec::as_slice).collect();
    let only_a = verify_chain(&VAULT, &m, &e).unwrap();
    let register = CounterRegister::new(&rig.store, &rig.witness);
    let files = SavedSnapshotFiles::new(&rig.dir.0);
    let sealer = identity(&rig.c.b);
    let result = SnapshotSealer {
        register: &register,
        files: &files,
        key: &rig.key,
        sealer: &sealer,
        chain: &only_a,
    }
    .seal(&fields(0x33, 41), sample());
    assert_eq!(
        result.unwrap_err(),
        SnapshotError::NotOpenable(SnapshotOpenFailure::DeviceOutsideEpoch)
    );
    assert!(!rig.file(0x33).exists());
    assert_eq!(rig.store.saved(&context(&rig.key, &rig.c.b, 2)), Ok(None));
}

#[test]
fn a_rolled_back_register_is_nonce_state_lost_and_seals_nothing() {
    let rig = Rig::new("lost");
    rig.seal(&fields(0x33, 41), sample()).unwrap();
    rig.seal(&fields(0x34, 57), &pt("second")).unwrap();
    // A restore of the vault: the register forgets both counters; the witness remembers.
    rig.store.force_next(&rig.ctx(), Some(0));
    let result = rig.seal(&fields(0x35, 60), &pt("third"));
    assert_eq!(
        result.unwrap_err(),
        SnapshotError::Nonce(NonceError::NonceStateLost(
            LostReason::RegisterBehindWitness
        ))
    );
    assert!(!rig.file(0x35).exists());
}

#[test]
fn a_retransmission_checks_the_register_first_and_stops_at_nonce_state_lost() {
    let rig = Rig::new("t1-lost");
    let first = rig.seal(&fields(0x33, 41), sample()).unwrap();
    assert_eq!(first.counter, 0);
    // A restore rolled the vault's register back behind the witness; the saved record (which sits
    // in the vault) is still there with its container file.
    rig.store.force_next(&rig.ctx(), Some(0));
    rig.witness.force_counter(&rig.ctx(), 1);
    assert!(rig.file(0x33).exists());
    assert_eq!(
        rig.seal(&fields(0x33, 41), sample()).unwrap_err(),
        SnapshotError::Nonce(NonceError::NonceStateLost(
            LostReason::RegisterBehindWitness
        )),
        "the saved container of a rolled-back timeline is never sent"
    );
    // The same holds for a publish that has nothing saved: the device is stopped in the context.
    assert_eq!(
        rig.seal(&fields(0x34, 57), &pt("another")).unwrap_err(),
        SnapshotError::Nonce(NonceError::NonceStateLost(
            LostReason::RegisterBehindWitness
        ))
    );
}

#[test]
fn a_saved_record_ahead_of_the_register_is_never_sent() {
    let rig = Rig::new("t1-ahead");
    rig.seal(&fields(0x33, 41), sample()).unwrap(); // sealed with counter 0, register now 1
                                                    // Register and witness agree on 0 (both rolled back), so the pair looks consistent, but the
                                                    // saved record names counter 0, which the register has not passed.
    rig.store.force_next(&rig.ctx(), Some(0));
    rig.witness.force_counter(&rig.ctx(), 0);
    assert_eq!(
        rig.seal(&fields(0x33, 41), sample()).unwrap_err(),
        SnapshotError::Nonce(NonceError::NonceStateLost(LostReason::SavedAheadOfRegister))
    );
    // And a saved record that the register has passed is sent as before.
    rig.store.force_next(&rig.ctx(), Some(1));
    rig.witness.force_counter(&rig.ctx(), 1);
    assert!(rig.seal(&fields(0x33, 41), sample()).unwrap().reused);
}

#[test]
fn the_cap_seals_the_last_counter_and_the_next_publish_requires_a_rotation() {
    let rig = Rig::new("cap");
    let last = sync_core::header::MAX_INVOCATION_COUNTER;
    rig.store.force_next(&rig.ctx(), Some(last));
    rig.witness.force_counter(&rig.ctx(), last);
    let sealed = rig.seal(&fields(0x33, 41), sample()).unwrap();
    assert_eq!(sealed.counter, last);
    assert_eq!(
        parse_container(&sealed.container)
            .unwrap()
            .header
            .invocation_counter,
        last
    );
    assert_eq!(
        rig.seal(&fields(0x34, 57), &pt("one more")).unwrap_err(),
        SnapshotError::Nonce(NonceError::RotationRequired)
    );
    // The same publish can still be sent again: that spends nothing.
    assert!(rig.seal(&fields(0x33, 41), sample()).unwrap().reused);
}

#[test]
fn another_epoch_is_another_context_and_never_reuses_the_saved_container() {
    let rig = Rig::new("epoch");
    let first = rig.seal(&fields(0x33, 41), sample()).unwrap();
    // After a rotation, the same publish under the next epoch's key is sealed from counter 0.
    let register = CounterRegister::new(&rig.store, &rig.witness);
    let files = SavedSnapshotFiles::new(&rig.dir.0);
    let sealer = identity(&rig.c.a);
    let key2 = epoch_key(2);
    let m: Vec<&[u8]> = rig.c.memberships[..4].iter().map(Vec::as_slice).collect();
    let e: Vec<&[u8]> = rig.c.epochs[..4].iter().map(Vec::as_slice).collect();
    let state2 = verify_chain(&VAULT, &m, &e).unwrap();
    let second = SnapshotSealer {
        register: &register,
        files: &files,
        key: &key2,
        sealer: &sealer,
        chain: &state2,
    }
    .seal(&fields(0x33, 41), sample())
    .unwrap();
    assert!(!second.reused);
    assert_eq!(second.counter, 0);
    assert_ne!(second.container, first.container);
}

// ---- the guard on the opener -------------------------------------------------------------------

/// A snapshot is opened only through `SnapshotOpener::new`, and a replica code path must not
/// hand-roll another opener: construction appears only in `vault-crypto` and in tests.
#[test]
fn the_snapshot_opener_is_constructed_only_in_vault_crypto_and_tests() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut stack = vec![root.join("crates"), root.join("apps")];
    let mut offenders = Vec::new();
    let mut seen = 0;
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
                let shown = path.to_string_lossy().replace('\\', "/");
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                if text.contains("SnapshotOpener::new(") {
                    seen += 1;
                    if !(shown.contains("crates/vault-crypto/") || shown.contains("/tests/")) {
                        offenders.push(shown.clone());
                    }
                }
                // No one else implements an opener by hand under another name.
                if text.contains("fn open(&self, container: &ParsedContainer")
                    && text.contains("OBJECT_SNAPSHOT")
                    && !shown.contains("crates/vault-crypto/")
                    && !shown.contains("/tests/")
                {
                    offenders.push(shown);
                }
            }
        }
    }
    assert!(seen >= 1, "the guard found no use at all");
    assert!(offenders.is_empty(), "{offenders:?}");
}

#[test]
fn nothing_secret_is_in_a_debug_or_an_error() {
    let rig = Rig::new("debug");
    let sealed = rig.seal(&fields(0x33, 41), sample()).unwrap();
    let shown = format!("{sealed:?}");
    assert!(shown.contains("len"));
    assert!(!shown.contains("manifest.json"));
    for error in [
        SnapshotError::Seal(SnapshotSealError::SnapshotTooLarge),
        SnapshotError::Inconsistent,
        SnapshotError::NotOpenable(SnapshotOpenFailure::NotOpenable),
    ] {
        let text = format!("{error} {error:?}");
        assert!(!text.contains("manifest.json"));
    }
    let _ = common::VAULT;
    let _ = ObjectType::Snapshot;
}

// ---- the closure bound: 2^20 for a snapshot, never an envelope delta's 4096 ----------------------

/// `count` strictly ascending ids.
fn ascending_ids(count: usize) -> Vec<[u8; 32]> {
    (0..count)
        .map(|i| {
            let mut id = [0_u8; 32];
            id[..8].copy_from_slice(&(i as u64).to_be_bytes());
            id
        })
        .collect()
}

/// Seal a snapshot whose closure is `ids`, check header field 17 is the closure commitment (2^20
/// bound), and open it again.
fn seal_and_open_a_closure(ids: &[[u8; 32]]) {
    use sync_core::artifact::ClosureManifest;
    let c = chain();
    let chain_state = state(&c);
    let key = epoch_key(1);
    let plaintext = common::snapshot_plaintext(ids, "a large closure");
    let expected = ClosureManifest::new(ids.to_vec()).unwrap().commitment();
    assert_eq!(
        vault_crypto::sync_snapshot::closure_commitment_of(&plaintext),
        Ok(expected)
    );
    let (store, witness) = (MemoryRegisterStore::new(), MockWitness::new());
    let register = CounterRegister::new(&store, &witness);
    let reservation = register.reserve(&context(&key, &c.a, 2)).unwrap();
    let sealed = seal_snapshot(
        &key,
        &identity(&c.a),
        &fields(0x33, 41),
        reservation,
        &plaintext,
    )
    .expect("a closure within the bound seals");
    match parse_container(&sealed).unwrap().header.object {
        ObjectKind::Snapshot {
            artifact_manifest_commitment,
            ..
        } => assert_eq!(artifact_manifest_commitment, expected),
        ObjectKind::Envelope { .. } => panic!("a snapshot header"),
    }
    assert_eq!(open(&key, &chain_state, &sealed).unwrap(), plaintext);
}

/// An envelope's delta holds 4096 ids; a snapshot's closure holds 2^20. At and above the delta's
/// bound the closure commits, seals and opens, and at 4096 the commitment is the delta's own
/// function of the list (same label and bytes).
#[test]
fn a_closure_above_a_deltas_bound_commits_seals_and_opens() {
    let ids = ascending_ids(4096);
    assert_eq!(
        sync_core::artifact::ClosureManifest::new(ids.clone())
            .unwrap()
            .commitment(),
        ArtifactManifest::new(ids.clone()).unwrap().commitment(),
        "at the delta's bound the two commitments are one function"
    );
    seal_and_open_a_closure(&ids);
    seal_and_open_a_closure(&ascending_ids(4097));
}

/// A closure at the snapshot's bound seals and opens.
#[test]
fn a_closure_at_the_snapshot_bound_seals_and_opens() {
    seal_and_open_a_closure(&ascending_ids(sync_core::snapshot::MAX_ARTIFACTS));
}

/// The sync-core and the commitment bounds are the same number, so a closure the snapshot accepts
/// is one the commitment can be taken over, and the one above is refused at export.
#[test]
fn a_closure_above_the_bound_is_refused_at_export_with_the_typed_error() {
    use sync_core::snapshot::{
        build, ColumnEntry, Header, SnapshotError, TableFile, MAX_ARTIFACTS,
    };
    assert_eq!(MAX_ARTIFACTS, sync_core::artifact::MAX_SNAPSHOT_CLOSURE);
    let artifacts: Vec<_> = ascending_ids(MAX_ARTIFACTS + 1)
        .into_iter()
        .map(common::bound)
        .collect();
    let file = TableFile {
        name: "notes".to_owned(),
        columns: vec![ColumnEntry {
            name: "text".to_owned(),
            kind: "text".to_owned(),
        }],
        row_count: 1,
        state_digest: Some([7; 32]),
        content: b"{\"text\":\"x\"}\n".to_vec(),
    };
    let result = build(
        &Header {
            schema_version: 65,
            vault_created_at: "2026-01-01T00:00:00Z",
            inventory_version: "1",
            voids: &[],
            artifacts: &artifacts,
        },
        vec![file],
    );
    assert_eq!(result, Err(SnapshotError::TooLarge));
}
