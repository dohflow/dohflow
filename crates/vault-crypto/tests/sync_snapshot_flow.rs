//! Publishing a snapshot (personal-cfo-1cltt.4, part B): a genesis by `installEpoch`, a re-snapshot
//! by `publishSnapshot`, the local refusals that come before any counter or request, the head
//! compare-and-swap, the closure receipts and pins, the idempotent re-query after a lost response,
//! and the vault the service already knows (R39). The service is the fake that runs the protocol's
//! decision procedure.

mod common;

use std::path::Path;

use common::register::MemoryRegisterStore;
use common::snapshot_service::{Credential, FakeSnapshotService, Fault};
use common::{chain, epoch_key, Chain, VAULT};
use platform_mock::MockWitness;
use sync_core::artifact::ArtifactId;
use sync_core::membership::{verify_chain, VerifiedState};
use sync_core::snapshot_service::{ServiceError, SnapshotService, SnapshotSource};
use sync_core::status::{ApplyFailureCode, HaltEvidence, InvalidEnvelope, StopReason, SyncStatus};
use vault_crypto::sync_counter::{CounterRegister, NonceError, RegisterStore};
use vault_crypto::sync_envelope::{EpochKey, SealerIdentity};
use vault_crypto::sync_keys::DeviceKeys;
use vault_crypto::sync_records::create_genesis;
use vault_crypto::sync_snapshot::{
    SavedSnapshotFiles, SnapshotError, SnapshotFields, SnapshotSealer,
};
use vault_crypto::sync_snapshot_flow::{
    find_published, publish, PublishError, PublishKind, PublishOutcome, PublishRequest,
};

struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "dohflow-snapshot-flow-{tag}-{}-{:?}",
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

/// Everything a publish needs on one device: the register and its witness, the saved-container
/// directory, the epoch key, the sealer's identity and the chain the receiver path judges against.
struct Rig {
    _dir: TempDir,
    store: MemoryRegisterStore,
    witness: MockWitness,
    files: SavedSnapshotFiles,
    key: EpochKey,
    sealer: SealerIdentity,
    state: VerifiedState,
    chain: Chain,
}

impl Rig {
    /// Epoch 1, device A, the genesis chain only.
    fn genesis(tag: &str) -> Self {
        let c = chain();
        let state = verify_chain(
            &VAULT,
            &[c.memberships[0].as_slice()],
            &[c.epochs[0].as_slice()],
        )
        .expect("the genesis chain");
        Self::with(tag, c, epoch_key(1), state)
    }

    /// Epoch 2, after the cutover that revoked C.
    fn cutover(tag: &str) -> Self {
        let c = chain();
        let m: Vec<&[u8]> = c.memberships[..4].iter().map(Vec::as_slice).collect();
        let e: Vec<&[u8]> = c.epochs[..4].iter().map(Vec::as_slice).collect();
        let state = verify_chain(&VAULT, &m, &e).expect("epoch 2");
        Self::with(tag, c, epoch_key(2), state)
    }

    fn with(tag: &str, c: Chain, key: EpochKey, state: VerifiedState) -> Self {
        let dir = TempDir::new(tag);
        let files = SavedSnapshotFiles::new(&dir.0);
        let sealer = SealerIdentity {
            vault_id: VAULT,
            device_id: c.a.id,
            fingerprint: c.a.fingerprint(),
        };
        Self {
            _dir: dir,
            store: MemoryRegisterStore::new(),
            witness: MockWitness::new(),
            files,
            key,
            sealer,
            state,
            chain: c,
        }
    }

    fn publish(
        &self,
        service: &FakeSnapshotService,
        status: &SyncStatus,
        request: &PublishRequest<'_>,
    ) -> Result<PublishOutcome, PublishError> {
        let register = CounterRegister::new(&self.store, &self.witness);
        let sealer = SnapshotSealer {
            register: &register,
            files: &self.files,
            key: &self.key,
            sealer: &self.sealer,
            chain: &self.state,
        };
        publish(service, &sealer, status, request)
    }

    fn saved_file_exists(&self, object_id: u8) -> bool {
        let name = format!("{}.snapshot", hex(&[object_id; 16]));
        Path::new(&self._dir.0).join(name).exists()
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn fields(object_id: u8, base_seq: u64) -> SnapshotFields {
    SnapshotFields {
        object_id: [object_id; 16],
        base_seq,
        payload_schema_version: 1,
        engine_version: "dohflow-test".to_owned(),
        schema_version: 67,
    }
}

fn genesis_request<'a>(rig: &'a Rig, plaintext: &'a [u8]) -> PublishRequest<'a> {
    PublishRequest {
        kind: PublishKind::Genesis {
            membership_record: &rig.chain.memberships[0],
            epoch_record: &rig.chain.epochs[0],
        },
        source: SnapshotSource::LiveVault,
        fields: fields(0x01, 0),
        plaintext,
    }
}

fn re_snapshot<'a>(object_id: u8, base_seq: u64, plaintext: &'a [u8]) -> PublishRequest<'a> {
    PublishRequest {
        kind: PublishKind::ReSnapshot,
        source: SnapshotSource::StateAtHead { head: base_seq },
        fields: fields(object_id, base_seq),
        plaintext,
    }
}

fn pt(marker: &str) -> Vec<u8> {
    common::snapshot_plaintext(&[], marker)
}

const HEALTHY: SyncStatus = SyncStatus::Healthy;

/// A service that already holds the genesis of `rig`, published through the normal path.
fn service_with_genesis(rig: &Rig) -> FakeSnapshotService {
    let service = FakeSnapshotService::new(Credential::Account);
    let plaintext = pt("genesis");
    rig.publish(&service, &HEALTHY, &genesis_request(rig, &plaintext))
        .expect("the genesis publishes");
    service.set_credential(Credential::Device);
    service
}

// ---- the genesis -------------------------------------------------------------------------------

#[test]
fn a_genesis_is_installed_by_install_epoch_and_the_saved_container_is_dropped() {
    let rig = Rig::genesis("genesis");
    let service = FakeSnapshotService::new(Credential::Account);
    let plaintext = pt("genesis");
    let outcome = rig
        .publish(&service, &HEALTHY, &genesis_request(&rig, &plaintext))
        .unwrap();

    let PublishOutcome::GenesisInstalled(installed) = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(
        (installed.key_epoch, installed.ordinal, installed.position),
        (1, 1, 0)
    );
    assert_eq!(service.calls(), ["stage_snapshot", "install_epoch"]);
    let published = service.published();
    assert_eq!(published.len(), 1);
    assert_eq!((published[0].base_seq, published[0].key_epoch), (0, 1));
    assert_eq!(published[0].snapshot_id, [0x01; 16]);
    assert!(!rig.saved_file_exists(0x01), "the publish is durable");
}

#[test]
fn a_lost_install_response_resumes_to_the_same_genesis_without_a_second_counter() {
    let rig = Rig::genesis("lost");
    let service = FakeSnapshotService::new(Credential::Account);
    let plaintext = pt("genesis");
    // The stage works; the install takes effect and its answer is lost.
    service.script(Fault::Pass);
    service.script(Fault::LoseResponse);

    assert_eq!(
        rig.publish(&service, &HEALTHY, &genesis_request(&rig, &plaintext)),
        Err(PublishError::Service(ServiceError::Unavailable))
    );
    assert!(service.knows_vault(), "the install took effect");
    assert!(
        rig.saved_file_exists(0x01),
        "and the container is still saved"
    );

    // The resume: the account credential is refused at the stage (the vault is known now), which
    // is not read as R39 or as a failure; installEpoch with the same bytes answers the original.
    let before = service.calls().len();
    let outcome = rig
        .publish(&service, &HEALTHY, &genesis_request(&rig, &plaintext))
        .unwrap();
    assert!(
        matches!(outcome, PublishOutcome::GenesisInstalled(_)),
        "{outcome:?}"
    );
    assert_eq!(
        service.calls()[before..],
        ["stage_snapshot", "install_epoch"],
        "a refused stage goes on to installEpoch"
    );
    assert_eq!(service.published().len(), 1, "one genesis, once");
    assert_eq!(
        rig.store.commits(),
        1,
        "the same sealed bytes were sent each time: one counter"
    );
    assert!(!rig.saved_file_exists(0x01));
}

#[test]
fn a_fresh_container_over_a_staged_one_stops_and_a_saved_one_goes_on_to_install() {
    // An earlier attempt staged a container; its saved file is gone, so the next seal is fresh.
    let rig = Rig::genesis("conflict");
    let service = FakeSnapshotService::new(Credential::Account);
    let plaintext = pt("genesis");
    service.script(Fault::Pass); // the stage takes effect
    service.script(Fault::Refuse(ServiceError::Unavailable)); // the install does not arrive
    assert!(rig
        .publish(&service, &HEALTHY, &genesis_request(&rig, &plaintext))
        .is_err());
    std::fs::remove_file(rig._dir.0.join(format!("{}.snapshot", hex(&[0x01; 16])))).unwrap();

    // Fresh: the stage answers ObjectConflict, which is not the saved container.
    assert_eq!(
        rig.publish(&service, &HEALTHY, &genesis_request(&rig, &plaintext)),
        Ok(PublishOutcome::GenesisResumeLost)
    );
    assert!(service.published().is_empty());
    assert!(
        !rig.saved_file_exists(0x01),
        "the container it sealed is dropped"
    );

    // Saved: the same refusal at the stage goes on to the install, which decides.
    let rig = Rig::genesis("conflict-saved");
    let service = FakeSnapshotService::new(Credential::Account);
    service.script(Fault::Pass);
    service.script(Fault::Refuse(ServiceError::Unavailable));
    assert!(rig
        .publish(&service, &HEALTHY, &genesis_request(&rig, &plaintext))
        .is_err());
    service.script(Fault::Refuse(ServiceError::ObjectConflict)); // the stage of the retry
    let outcome = rig
        .publish(&service, &HEALTHY, &genesis_request(&rig, &plaintext))
        .unwrap();
    assert!(
        matches!(outcome, PublishOutcome::GenesisInstalled(_)),
        "{outcome:?}"
    );
}

#[test]
fn a_fresh_container_after_the_genesis_was_installed_stops_and_a_saved_one_finishes() {
    for lose_the_file in [true, false] {
        let rig = Rig::genesis("installed");
        let service = FakeSnapshotService::new(Credential::Account);
        let plaintext = pt("genesis");
        service.script(Fault::Pass);
        service.script(Fault::LoseResponse); // the install takes effect; the answer is lost
        assert!(rig
            .publish(&service, &HEALTHY, &genesis_request(&rig, &plaintext))
            .is_err());
        if lose_the_file {
            std::fs::remove_file(rig._dir.0.join(format!("{}.snapshot", hex(&[0x01; 16]))))
                .unwrap();
        }
        let outcome = rig
            .publish(&service, &HEALTHY, &genesis_request(&rig, &plaintext))
            .unwrap();
        if lose_the_file {
            assert_eq!(outcome, PublishOutcome::GenesisResumeLost, "a fresh seal");
        } else {
            let PublishOutcome::GenesisInstalled(installed) = outcome else {
                panic!("{outcome:?}");
            };
            assert!(installed.already_installed, "the 200 exact retry decided");
        }
        assert_eq!(service.published().len(), 1, "the service's own snapshot");
    }
}

#[test]
fn an_install_that_finds_no_staged_snapshot_stages_again_and_retries() {
    let rig = Rig::genesis("restage");
    let service = FakeSnapshotService::new(Credential::Account);
    service.script(Fault::Refuse(ServiceError::Forbidden)); // the stage
    service.script(Fault::Refuse(ServiceError::SnapshotNotFound)); // the install
    let plaintext = pt("genesis");
    let outcome = rig
        .publish(&service, &HEALTHY, &genesis_request(&rig, &plaintext))
        .unwrap();
    assert!(matches!(outcome, PublishOutcome::GenesisInstalled(_)));
    assert_eq!(
        service.calls(),
        [
            "stage_snapshot",
            "install_epoch",
            "stage_snapshot",
            "install_epoch"
        ]
    );
}

#[test]
fn a_stage_that_cannot_be_reached_is_not_followed_by_an_install() {
    let rig = Rig::genesis("unreachable");
    let service = FakeSnapshotService::new(Credential::Account);
    service.script(Fault::Refuse(ServiceError::Unavailable));
    let plaintext = pt("genesis");
    assert_eq!(
        rig.publish(&service, &HEALTHY, &genesis_request(&rig, &plaintext)),
        Err(PublishError::Service(ServiceError::Unavailable))
    );
    assert_eq!(service.calls(), ["stage_snapshot"]);
}

#[test]
fn a_vault_the_service_knows_is_reported_and_nothing_is_published_or_kept() {
    let rig = Rig::genesis("known");
    let service = FakeSnapshotService::new(Credential::Account);
    // Another device's genesis, made with its own keys.
    let other = create_genesis(
        VAULT,
        [0xee; 16],
        &DeviceKeys::generate().unwrap(),
        1_700_000_000,
    )
    .unwrap();
    service.seed_vault(other.membership, other.epoch);

    let plaintext = pt("genesis");
    let outcome = rig
        .publish(&service, &HEALTHY, &genesis_request(&rig, &plaintext))
        .unwrap();
    assert_eq!(outcome, PublishOutcome::VaultKnown);
    assert!(service.published().is_empty(), "nothing was published");
    assert!(
        !rig.saved_file_exists(0x01),
        "the refused genesis is not kept for resending"
    );
    assert_eq!(rig.store.saved(&counter_context(&rig)), Ok(None));
}

fn counter_context(rig: &Rig) -> platform_traits::CounterContext {
    platform_traits::CounterContext {
        vault_id: VAULT,
        key_epoch: rig.key.key_epoch(),
        fingerprint: *rig.sealer.fingerprint.as_bytes(),
        object_type: 2,
    }
}

// ---- the re-snapshot ---------------------------------------------------------------------------

#[test]
fn a_re_snapshot_is_published_at_the_head_and_its_closure_is_pinned() {
    let rig = Rig::genesis("resnap");
    let service = service_with_genesis(&rig);
    service.advance_head_to(5);
    let id = ArtifactId::from_derived([0x42; 32]);
    service.receipt(id);
    let plaintext = common::snapshot_plaintext(&[[0x42; 32]], "at five");

    let outcome = rig
        .publish(&service, &HEALTHY, &re_snapshot(0x02, 5, &plaintext))
        .unwrap();
    let PublishOutcome::Published(published) = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!((published.base_seq, published.key_epoch), (5, 1));
    assert_eq!(published.snapshot_id, [0x02; 16]);
    assert!(
        service.is_pinned(&id),
        "the closure is pinned with the publication"
    );
    assert_eq!(service.published().last().unwrap().snapshot_id, [0x02; 16]);
    assert!(!rig.saved_file_exists(0x02));
}

#[test]
fn a_head_that_moved_refuses_the_publish_and_changes_nothing() {
    let rig = Rig::genesis("head");
    let service = service_with_genesis(&rig);
    service.advance_head_to(6);
    let plaintext = pt("built at five");

    assert_eq!(
        rig.publish(&service, &HEALTHY, &re_snapshot(0x02, 5, &plaintext)),
        Err(PublishError::Service(ServiceError::HeadMismatch {
            head: 6
        }))
    );
    assert_eq!(
        service.published().len(),
        1,
        "only the genesis is published"
    );

    // The retry is a new plaintext at the new head: a new publish, a new counter.
    let commits = rig.store.commits();
    let rebuilt = pt("built at six");
    rig.publish(&service, &HEALTHY, &re_snapshot(0x03, 6, &rebuilt))
        .unwrap();
    assert_eq!(rig.store.commits(), commits + 1);
    assert_eq!(service.published().last().unwrap().base_seq, 6);
}

#[test]
fn a_closure_id_without_a_receipt_pins_nothing_and_publishes_nothing() {
    let rig = Rig::genesis("closure");
    let service = service_with_genesis(&rig);
    service.advance_head_to(5);
    let id = ArtifactId::from_derived([0x42; 32]);
    let plaintext = common::snapshot_plaintext(&[[0x42; 32]], "with an artifact");

    assert_eq!(
        rig.publish(&service, &HEALTHY, &re_snapshot(0x02, 5, &plaintext)),
        Err(PublishError::Service(ServiceError::ArtifactMissing {
            missing: vec![id]
        }))
    );
    assert!(!service.is_pinned(&id));
    assert_eq!(service.published().len(), 1);

    // Receipt before reference: once the artifact has its receipt the same publish goes through,
    // with the saved bytes and no new counter.
    service.receipt(id);
    let commits = rig.store.commits();
    rig.publish(&service, &HEALTHY, &re_snapshot(0x02, 5, &plaintext))
        .unwrap();
    assert!(service.is_pinned(&id));
    assert_eq!(
        rig.store.commits(),
        commits,
        "the saved container was resent"
    );
}

#[test]
fn a_snapshot_that_starts_an_epoch_is_not_published_by_publish_snapshot() {
    let rig = Rig::cutover("epoch");
    let service = service_with_genesis(&Rig::genesis("epoch-g"));
    let plaintext = pt("epoch two");

    assert_eq!(
        rig.publish(&service, &HEALTHY, &re_snapshot(0x02, 0, &plaintext)),
        Err(PublishError::Service(ServiceError::EpochRejected))
    );
    assert_eq!(service.published().len(), 1);
}

#[test]
fn a_lost_publish_response_is_found_by_the_re_query_and_the_retry_sends_the_same_bytes() {
    let rig = Rig::genesis("requery");
    let service = service_with_genesis(&rig);
    service.advance_head_to(5);
    let plaintext = pt("at five");
    // The stage works; the publish takes effect and its answer is lost.
    service.script(Fault::Pass);
    service.script(Fault::LoseResponse);

    assert_eq!(
        rig.publish(&service, &HEALTHY, &re_snapshot(0x02, 5, &plaintext)),
        Err(PublishError::Service(ServiceError::Unavailable))
    );
    assert!(
        rig.saved_file_exists(0x02),
        "the publish is not known to be durable"
    );

    // The idempotent re-query finds the published state.
    let found = find_published(&service, &VAULT, &[0x02; 16])
        .unwrap()
        .unwrap();
    assert_eq!((found.base_seq, found.key_epoch), (5, 1));

    // The retry resends the saved container: no new counter, the original result.
    let commits = rig.store.commits();
    let outcome = rig
        .publish(&service, &HEALTHY, &re_snapshot(0x02, 5, &plaintext))
        .unwrap();
    assert_eq!(outcome, PublishOutcome::Published(found));
    assert_eq!(rig.store.commits(), commits);
    assert_eq!(service.published().len(), 2, "published once");
    assert!(!rig.saved_file_exists(0x02));
}

// ---- the refusals that come first --------------------------------------------------------------

fn halted() -> SyncStatus {
    SyncStatus::Halted(HaltEvidence::Fork {
        version: 2,
        accepted: [1; 32],
        conflicting: [2; 32],
    })
}

fn stopped(rig: &Rig) -> SyncStatus {
    SyncStatus::EnvelopeInvalid(InvalidEnvelope {
        seq: 9,
        envelope_digest: [3; 32],
        command_id: [4; 16],
        device_id: rig.chain.b.id,
        device_key_fingerprint: rig.chain.b.fingerprint(),
        reason: StopReason::Apply(ApplyFailureCode::new(1).unwrap()),
    })
}

#[test]
fn nothing_is_published_while_halted_or_stopped_and_nothing_is_spent_or_sent() {
    let rig = Rig::genesis("rule5");
    let service = service_with_genesis(&rig);
    service.advance_head_to(5);
    let calls = service.calls().len();
    let commits = rig.store.commits();
    let plaintext = pt("at five");

    assert_eq!(
        rig.publish(&service, &halted(), &re_snapshot(0x02, 5, &plaintext)),
        Err(PublishError::Halted)
    );
    assert_eq!(
        rig.publish(&service, &stopped(&rig), &re_snapshot(0x02, 5, &plaintext)),
        Err(PublishError::Stopped)
    );
    let genesis = pt("genesis two");
    assert_eq!(
        rig.publish(&service, &halted(), &genesis_request(&rig, &genesis)),
        Err(PublishError::Halted)
    );
    assert_eq!(
        service.calls().len(),
        calls,
        "the service recorded zero calls"
    );
    assert_eq!(rig.store.commits(), commits, "and no counter was reserved");
    assert!(!rig.saved_file_exists(0x02));
}

#[test]
fn a_device_the_service_reports_halted_gets_its_own_typed_error() {
    let rig = Rig::genesis("service-halt");
    let service = service_with_genesis(&rig);
    service.advance_head_to(5);
    service.halt_device(1003);
    let plaintext = pt("at five");
    assert_eq!(
        rig.publish(&service, &HEALTHY, &re_snapshot(0x02, 5, &plaintext)),
        Err(PublishError::ServiceHalted { stopped_at: 1003 })
    );
}

#[test]
fn the_simulator_judges_the_container_before_the_vault_side_forbidden_and_the_halt_gate() {
    // Protocol 4.9, the order of decision: the checks that need no vault state come first, then the
    // authorization that needs the vault, then the halt gate.
    let rig = Rig::genesis("order");
    let service = service_with_genesis(&rig);
    service.halt_device(1003);
    // A device that reported a halt, sending bytes that are no container: MalformedObject, not the halt.
    assert_eq!(
        service.stage_snapshot(&VAULT, &[7; 16], &[0; 20]),
        Err(ServiceError::MalformedObject)
    );
    // An account credential after the genesis, sending the same: still the stateless refusal, not Forbidden.
    service.set_credential(Credential::Account);
    assert_eq!(
        service.stage_snapshot(&VAULT, &[7; 16], &[0; 20]),
        Err(ServiceError::MalformedObject)
    );
}

#[test]
fn a_snapshot_after_the_genesis_is_never_read_from_the_live_vault() {
    let rig = Rig::genesis("source");
    let service = service_with_genesis(&rig);
    service.advance_head_to(5);
    let calls = service.calls().len();
    let commits = rig.store.commits();
    let plaintext = pt("at five");

    let mut request = re_snapshot(0x02, 5, &plaintext);
    request.source = SnapshotSource::LiveVault;
    assert_eq!(
        rig.publish(&service, &HEALTHY, &request),
        Err(PublishError::LiveVaultSource)
    );
    assert_eq!(service.calls().len(), calls);
    assert_eq!(rig.store.commits(), commits);

    // And a genesis is read from the live vault at position 0, nowhere else.
    let genesis = pt("genesis two");
    let mut bad = genesis_request(&rig, &genesis);
    bad.source = SnapshotSource::StateAtHead { head: 0 };
    assert_eq!(
        rig.publish(&service, &HEALTHY, &bad),
        Err(PublishError::GenesisSource)
    );
    let mut bad = genesis_request(&rig, &genesis);
    bad.fields.base_seq = 4;
    assert_eq!(
        rig.publish(&service, &HEALTHY, &bad),
        Err(PublishError::GenesisSource)
    );
    assert_eq!(service.calls().len(), calls);
}

#[test]
fn a_re_snapshot_built_from_another_position_than_it_declares_is_refused_locally() {
    let rig = Rig::genesis("source-head");
    let service = service_with_genesis(&rig);
    service.advance_head_to(5);
    let calls = service.calls().len();
    let commits = rig.store.commits();
    let plaintext = pt("state at three");

    // The state at 3 declared as built on 5, and the state at 7 declared as built on 5.
    for head in [3, 7] {
        let mut request = re_snapshot(0x02, 5, &plaintext);
        request.source = SnapshotSource::StateAtHead { head };
        assert_eq!(
            rig.publish(&service, &HEALTHY, &request),
            Err(PublishError::SourceHeadMismatch {
                source_head: head,
                base_seq: 5
            }),
            "source head {head}"
        );
    }
    assert_eq!(
        service.calls().len(),
        calls,
        "the service recorded zero calls"
    );
    assert_eq!(rig.store.commits(), commits, "no counter was reserved");
    assert!(!rig.saved_file_exists(0x02), "no container was saved");

    // The matching head publishes.
    let matched = re_snapshot(0x02, 5, &plaintext);
    assert_eq!(matched.source, SnapshotSource::StateAtHead { head: 5 });
    assert!(rig.publish(&service, &HEALTHY, &matched).is_ok());
}

#[test]
fn a_restored_register_stops_the_publish_before_anything_is_sent() {
    let rig = Rig::genesis("restore");
    let service = FakeSnapshotService::new(Credential::Account);
    let plaintext = pt("genesis");
    // A witness that has seen a counter the register has not: the vault was restored.
    let context = counter_context(&rig);
    platform_traits::RollbackWitness::advance_counter(&rig.witness, &context, 4).unwrap();

    let result = rig.publish(&service, &HEALTHY, &genesis_request(&rig, &plaintext));
    assert!(
        matches!(
            result,
            Err(PublishError::Snapshot(SnapshotError::Nonce(
                NonceError::NonceStateLost(_)
            )))
        ),
        "{result:?}"
    );
    assert!(service.calls().is_empty());
}

// ---- no production implementor ----------------------------------------------------------------

fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

#[test]
fn snapshot_service_has_no_implementor_outside_tests() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for entry in std::fs::read_dir(workspace.join("crates"))
        .unwrap()
        .flatten()
    {
        walk(&entry.path().join("src"), &mut files);
    }
    walk(&workspace.join("apps/desktop/src-tauri/src"), &mut files);
    assert!(files.len() > 100, "found {} source files", files.len());
    let mut implementors = Vec::new();
    for path in files {
        let source = std::fs::read_to_string(&path).unwrap();
        let code: String = source
            .lines()
            .map(|line| line.split("//").next().unwrap_or(""))
            .collect::<Vec<_>>()
            .join(" ");
        let code = code.split_whitespace().collect::<Vec<_>>().join(" ");
        if code.contains("SnapshotService for") {
            implementors.push(path.display().to_string());
        }
    }
    assert!(
        implementors.is_empty(),
        "SnapshotService implemented outside tests (the client engine, personal-cfo-elciw, owns \
         the first production implementor): {implementors:?}"
    );
}

#[test]
fn find_published_answers_only_for_the_newest_snapshot_of_that_id() {
    let rig = Rig::genesis("find");
    let service = service_with_genesis(&rig);
    assert_eq!(
        find_published(&service, &VAULT, &[0x01; 16])
            .unwrap()
            .map(|p| p.base_seq),
        Some(0)
    );
    assert_eq!(find_published(&service, &VAULT, &[0x09; 16]), Ok(None));
}
