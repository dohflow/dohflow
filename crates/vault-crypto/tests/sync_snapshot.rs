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

const PLAINTEXT: &[u8] = b"snapshot plaintext: manifest.json then the data files";

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

fn fields(object_id: u8, base_seq: u64) -> SnapshotFields {
    SnapshotFields {
        object_id: [object_id; 16],
        base_seq,
        payload_schema_version: 1,
        engine_version: "dohflow-test".to_owned(),
        schema_version: 65,
        closure_commitment: empty_closure(),
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
    let (bytes, key) = sealed(&c, &c.a, PLAINTEXT);
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
            artifact_manifest_commitment: empty_closure()
        }
    );
    assert_eq!(open(&key, &chain_state, &bytes).unwrap(), PLAINTEXT);
}

#[test]
fn a_wrong_key_an_aad_mismatch_and_tampering_fail_closed() {
    let c = chain();
    let chain_state = state(&c);
    let (bytes, key) = sealed(&c, &c.a, PLAINTEXT);
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
    let (snapshot, key) = sealed(&c, &c.a, PLAINTEXT);
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
        seal_snapshot(&key, &identity(d), &fields(0x33, 41), r, PLAINTEXT).unwrap()
    };
    let (sa, sb) = (seal(&c.a), seal(&c.b));
    assert_eq!(open(&key, &chain_state, &sa).unwrap(), PLAINTEXT);
    assert_eq!(open(&key, &chain_state, &sb).unwrap(), PLAINTEXT);
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
    let (bytes, key) = sealed(&c, &c.a, PLAINTEXT);
    // A chain that holds only the first two records: device C is not a member of it, A is. Seal as
    // C against the full chain, then open against the short one.
    let full = state(&c);
    let (store, witness) = (MemoryRegisterStore::new(), MockWitness::new());
    let register = CounterRegister::new(&store, &witness);
    let r = register.reserve(&context(&key, &c.c, 2)).unwrap();
    let by_c = seal_snapshot(&key, &identity(&c.c), &fields(0x34, 1), r, PLAINTEXT).unwrap();
    let m: Vec<&[u8]> = c.memberships[..2].iter().map(Vec::as_slice).collect();
    let e: Vec<&[u8]> = c.epochs[..2].iter().map(Vec::as_slice).collect();
    let short = verify_chain(&VAULT, &m, &e).unwrap();
    assert_eq!(open(&key, &full, &by_c).unwrap(), PLAINTEXT);
    assert_eq!(
        open(&key, &short, &by_c),
        Err(SnapshotOpenFailure::DeviceOutsideEpoch)
    );
    assert!(open(&key, &short, &bytes).is_ok());
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
        seal_snapshot(&key, &identity(&c.a), &fields(0x33, 41), r, PLAINTEXT)
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
        seal_snapshot(&key, &other_vault, &fields(0x33, 41), r, PLAINTEXT).unwrap_err(),
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
    let first = rig.seal(&fields(0x33, 41), PLAINTEXT).unwrap();
    assert!(!first.reused);
    assert_eq!(first.counter, 0);
    let commits = rig.store.commits();
    let witnessed = rig.witness.counter_advances();
    // A transport retry of the same publish: the same bytes, no counter.
    let again = rig.seal(&fields(0x33, 41), PLAINTEXT).unwrap();
    assert!(again.reused);
    assert_eq!(again.container, first.container);
    assert_eq!(again.counter, 0);
    assert_eq!(rig.store.commits(), commits);
    assert_eq!(rig.witness.counter_advances(), witnessed);
    assert_eq!(rig.store.next_counter(&rig.ctx()), Ok(Some(1)));
    // The saved container opens through the receiver path.
    assert_eq!(
        open(&rig.key, &rig.chain_state, &again.container).unwrap(),
        PLAINTEXT
    );
}

#[test]
fn a_changed_retry_burns_a_counter_and_replaces_the_saved_container() {
    let rig = Rig::new("changed");
    let first = rig.seal(&fields(0x33, 41), PLAINTEXT).unwrap();
    // A retry after a failed compare-and-swap is a new head and a new snapshot.
    let new_head = rig.seal(&fields(0x34, 57), b"a newer plaintext").unwrap();
    assert!(!new_head.reused);
    assert_eq!(new_head.counter, 1);
    assert_ne!(new_head.container, first.container);
    assert!(!rig.file(0x33).exists(), "the superseded file is gone");
    assert!(rig.file(0x34).exists());
    // The same snapshot id with a different base, or different content, is not the same publish.
    let rebased = rig.seal(&fields(0x34, 58), b"a newer plaintext").unwrap();
    assert!(!rebased.reused);
    assert_eq!(rebased.counter, 2);
    let edited = rig.seal(&fields(0x34, 58), b"an edited plaintext").unwrap();
    assert!(!edited.reused);
    assert_eq!(edited.counter, 3);
    // The latest publish is the one that is saved.
    assert!(
        rig.seal(&fields(0x34, 58), b"an edited plaintext")
            .unwrap()
            .reused
    );
    // Every counter used was distinct.
    assert_eq!(rig.store.next_counter(&rig.ctx()), Ok(Some(4)));
}

#[test]
fn a_missing_or_damaged_saved_file_is_not_reused() {
    let rig = Rig::new("damaged");
    let first = rig.seal(&fields(0x33, 41), PLAINTEXT).unwrap();
    // Damaged: one byte flipped on disk.
    let mut bytes = std::fs::read(rig.file(0x33)).unwrap();
    let mid = bytes.len() / 2;
    bytes[mid] ^= 1;
    std::fs::write(rig.file(0x33), &bytes).unwrap();
    let second = rig.seal(&fields(0x33, 41), PLAINTEXT).unwrap();
    assert!(!second.reused);
    assert_eq!(second.counter, 1);
    assert_ne!(second.container, first.container);
    // Missing.
    std::fs::remove_file(rig.file(0x33)).unwrap();
    let third = rig.seal(&fields(0x33, 41), PLAINTEXT).unwrap();
    assert!(!third.reused);
    assert_eq!(third.counter, 2);
}

#[test]
fn the_saved_record_and_file_survive_a_reopen_and_are_dropped_on_the_receipt() {
    let rig = Rig::new("receipt");
    rig.seal(&fields(0x33, 41), PLAINTEXT).unwrap();
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
    let again = publish.seal(&fields(0x33, 41), PLAINTEXT).unwrap();
    assert!(again.reused);
    // The publish is durable: forget it.
    publish.discard(&[0x33; 16]).unwrap();
    assert_eq!(rig.store.saved(&rig.ctx()), Ok(None));
    assert!(!rig.file(0x33).exists());
    // Discarding twice is not an error, and sealing again is a new seal.
    publish.discard(&[0x33; 16]).unwrap();
    let after = publish.seal(&fields(0x33, 41), PLAINTEXT).unwrap();
    assert!(!after.reused);
    assert_eq!(after.counter, 1);
}

#[test]
fn a_crash_between_the_file_and_its_record_burns_the_counter_and_reseals() {
    let rig = Rig::new("crash-record");
    rig.store.fail_next_set_saved(1);
    assert!(matches!(
        rig.seal(&fields(0x33, 41), PLAINTEXT).unwrap_err(),
        SnapshotError::Register(_)
    ));
    assert_eq!(
        rig.store.next_counter(&rig.ctx()),
        Ok(Some(1)),
        "counter 0 is burned"
    );
    assert_eq!(rig.store.saved(&rig.ctx()), Ok(None), "nothing is recorded");
    let retry = rig.seal(&fields(0x33, 41), PLAINTEXT).unwrap();
    assert!(!retry.reused);
    assert_eq!(retry.counter, 1, "counter 0 is never reused");
}

#[test]
fn a_witness_failure_after_the_register_commit_burns_the_counter_and_publishes_nothing() {
    let rig = Rig::new("crash-reserve");
    let first = rig.seal(&fields(0x33, 41), PLAINTEXT).unwrap();
    assert_eq!(first.counter, 0);
    // The register commits counter 1 and the witness then fails: counter 1 is spent, never sealed.
    rig.witness.fail_next_counter_advances(1);
    assert_eq!(
        rig.seal(&fields(0x34, 57), b"second").unwrap_err(),
        SnapshotError::Nonce(NonceError::Witness(WitnessError::Unavailable))
    );
    assert!(!rig.file(0x34).exists(), "nothing was sealed or saved");
    assert_eq!(rig.store.next_counter(&rig.ctx()), Ok(Some(2)));
    // The first publish is still saved and can still be sent again, at no cost.
    assert!(rig.file(0x33).exists());
    assert!(rig.seal(&fields(0x33, 41), PLAINTEXT).unwrap().reused);
    // The next new publish uses counter 2: counter 1 is never handed out.
    let next = rig.seal(&fields(0x34, 57), b"second").unwrap();
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
    .seal(&fields(0x33, 41), PLAINTEXT);
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
    rig.seal(&fields(0x33, 41), PLAINTEXT).unwrap();
    rig.seal(&fields(0x34, 57), b"second").unwrap();
    // A restore of the vault: the register forgets both counters; the witness remembers.
    rig.store.force_next(&rig.ctx(), Some(0));
    let result = rig.seal(&fields(0x35, 60), b"third");
    assert_eq!(
        result.unwrap_err(),
        SnapshotError::Nonce(NonceError::NonceStateLost(
            LostReason::RegisterBehindWitness
        ))
    );
    assert!(!rig.file(0x35).exists());
}

#[test]
fn the_cap_seals_the_last_counter_and_the_next_publish_requires_a_rotation() {
    let rig = Rig::new("cap");
    let last = sync_core::header::MAX_INVOCATION_COUNTER;
    rig.store.force_next(&rig.ctx(), Some(last));
    rig.witness.force_counter(&rig.ctx(), last);
    let sealed = rig.seal(&fields(0x33, 41), PLAINTEXT).unwrap();
    assert_eq!(sealed.counter, last);
    assert_eq!(
        parse_container(&sealed.container)
            .unwrap()
            .header
            .invocation_counter,
        last
    );
    assert_eq!(
        rig.seal(&fields(0x34, 57), b"one more").unwrap_err(),
        SnapshotError::Nonce(NonceError::RotationRequired)
    );
    // The same publish can still be sent again: that spends nothing.
    assert!(rig.seal(&fields(0x33, 41), PLAINTEXT).unwrap().reused);
}

#[test]
fn another_epoch_is_another_context_and_never_reuses_the_saved_container() {
    let rig = Rig::new("epoch");
    let first = rig.seal(&fields(0x33, 41), PLAINTEXT).unwrap();
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
    .seal(&fields(0x33, 41), PLAINTEXT)
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
    let sealed = rig.seal(&fields(0x33, 41), PLAINTEXT).unwrap();
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
