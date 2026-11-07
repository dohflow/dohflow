//! Enabling Sync on the founding device (personal-cfo-1cltt.4 part C): prepare, persist, install,
//! publish, with a crash or a lost response at every step, a genesis that is never minted twice,
//! and the vault the service already knows (R39). The vault is real (SQLCipher, the SQLite
//! register and genesis tables); the service is the fake that runs the protocol's decision
//! procedure, shared with the vault-crypto tests.

#![allow(dead_code)]

mod common;
#[path = "../../vault-crypto/tests/common/snapshot_service.rs"]
mod snapshot_service;

use common::{create_account_cmd, meta, worker};
use db_worker::base_image::saved_snapshots_dir;
use db_worker::sync_enable::{enable_sync, EnableError, EnableOutcome, EnablingDevice};
use db_worker::sync_genesis::{self, GenesisState};
use db_worker::sync_register::SqliteRegisterStore;
use db_worker::DbWorker;
use platform_mock::{MockSecretStore, MockWitness};
use platform_traits::{CounterContext, RollbackWitness};
use snapshot_service::{Credential, FakeSnapshotService, Fault};
use sync_core::membership::verify_chain;
use sync_core::snapshot_service::ServiceError;
use sync_core::status::{HaltEvidence, SyncStatus};
use vault_crypto::sync_counter::{
    LostReason, NonceError, RegisterStore, VerifiedStateError, VerifiedStateGuard,
};
use vault_crypto::sync_custody::SyncKeyCustody;
use vault_crypto::sync_keys::DeviceKeys;
use vault_crypto::sync_records::create_genesis;
use vault_crypto::sync_snapshot::SnapshotError;
use vault_crypto::sync_snapshot_flow::{genesis_snapshot_id, PublishError};

const DEVICE: EnablingDevice = EnablingDevice {
    device_id: [0xa1; 16],
    issued_at: 1_700_000_000,
};

/// Everything one device has: its vault, secret store and witness, and the service it talks to.
struct Device {
    dir: tempfile::TempDir,
    worker: DbWorker,
    secrets: MockSecretStore,
    witness: MockWitness,
    service: FakeSnapshotService,
}

impl Device {
    fn new() -> Self {
        let (dir, worker) = worker();
        Self {
            dir,
            worker,
            secrets: MockSecretStore::new(),
            witness: MockWitness::new(),
            service: FakeSnapshotService::new(Credential::Account),
        }
    }

    fn vault_id(&self) -> [u8; 16] {
        *self.worker.vault_id().unwrap().as_bytes()
    }

    fn enable(&self) -> Result<EnableOutcome, EnableError> {
        enable_sync(
            &self.worker,
            &self.secrets,
            &self.witness,
            &self.service,
            &SyncStatus::Healthy,
            &DEVICE,
        )
    }

    fn genesis(&self) -> Option<sync_genesis::PersistedGenesis> {
        self.worker
            .write_sync_state(|tx| sync_genesis::load(tx))
            .unwrap()
    }

    fn state(&self) -> Option<GenesisState> {
        self.genesis().map(|g| g.state)
    }

    fn custody(&self) -> SyncKeyCustody<'_> {
        SyncKeyCustody::new(&self.secrets, self.vault_id())
    }

    fn counter(&self) -> Option<u64> {
        let keys = self.custody().load_device_keys().unwrap();
        SqliteRegisterStore::new(&self.worker)
            .next_counter(&CounterContext {
                vault_id: self.vault_id(),
                key_epoch: 1,
                fingerprint: *keys.public().fingerprint.as_bytes(),
                object_type: 2,
            })
            .unwrap()
    }

    fn saved_files(&self) -> usize {
        std::fs::read_dir(saved_snapshots_dir(self.worker.db_path()))
            .map(|entries| entries.count())
            .unwrap_or(0)
    }

    /// The device is now unreachable for the service: it is Account until the genesis is in, and a
    /// Device afterwards, as the transport would hold.
    fn count(&self, table: &str) -> i64 {
        self.worker
            .read_connection()
            .unwrap()
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }
}

// ---- the whole sequence ------------------------------------------------------------------------

#[test]
fn enabling_publishes_one_genesis_and_a_second_call_makes_no_request() {
    let device = Device::new();
    worker_makes_an_account(&device);
    assert_eq!(device.enable().unwrap(), EnableOutcome::Enabled);

    assert_eq!(device.state(), Some(GenesisState::Published));
    assert_eq!(device.service.calls(), ["stage_snapshot", "install_epoch"]);
    let published = device.service.published();
    assert_eq!(published.len(), 1);
    let genesis = device.genesis().unwrap();
    let chain = verify_chain(
        &device.vault_id(),
        &[genesis.membership_record.as_slice()],
        &[genesis.epoch_record.as_slice()],
    )
    .unwrap();
    assert_eq!(
        published[0].snapshot_id,
        genesis_snapshot_id(&chain.epoch_head_id().unwrap())
    );
    assert_eq!((published[0].base_seq, published[0].key_epoch), (0, 1));
    assert!(
        device.custody().has_epoch_key(1).unwrap(),
        "K_1 is installed"
    );
    assert_eq!(device.saved_files(), 0, "the saved container is dropped");
    assert_eq!(device.counter(), Some(1), "one snapshot, one counter");
    assert!(
        device
            .witness
            .verified_mark(&device.vault_id())
            .unwrap()
            .is_some(),
        "the witness holds the genesis's mark"
    );

    // A published genesis is never re-minted, re-sent or re-checked against the service.
    let before = genesis.clone();
    assert_eq!(device.enable().unwrap(), EnableOutcome::Enabled);
    assert_eq!(device.service.calls().len(), 2, "no request");
    assert_eq!(device.genesis().unwrap(), before);
}

fn worker_makes_an_account(device: &Device) {
    device
        .worker
        .dispatch(meta(), create_account_cmd())
        .unwrap();
}

// ---- a crash at each step ----------------------------------------------------------------------

/// What `enable_sync` does before a given step, done by hand, so the call under test starts from
/// exactly the state a crash at that point leaves.
fn mint(device: &Device) -> vault_crypto::sync_records::GenesisRecords {
    let guard = VerifiedStateGuard::new(&device.witness);
    guard.begin(&device.vault_id()).unwrap();
    device
        .custody()
        .prepare_genesis(DEVICE.device_id, DEVICE.issued_at)
        .unwrap()
}

fn persist(device: &Device, records: &vault_crypto::sync_records::GenesisRecords) {
    // Read the vault id first: `vault_id` takes the writer lock, which `write_sync_state` holds.
    let vault_id = device.vault_id();
    device
        .worker
        .write_sync_state(|tx| {
            sync_genesis::persist(
                tx,
                &vault_id,
                &records.membership,
                &records.epoch,
                "2026-11-06T00:00:00+00:00",
            )
        })
        .unwrap();
}

#[test]
fn a_crash_after_the_birth_mark_and_before_the_persist_mints_a_genesis_on_resume() {
    let device = Device::new();
    let _lost = mint(&device); // the records were built and lost
    assert!(device.genesis().is_none());
    assert_eq!(device.enable().unwrap(), EnableOutcome::Enabled);
    assert_eq!(device.service.published().len(), 1);
}

#[test]
fn a_crash_after_the_persist_and_before_the_mark_resumes_to_the_same_genesis() {
    let device = Device::new();
    let records = mint(&device);
    persist(&device, &records);
    // The witness holds only the birth mark: the crash window.
    assert_eq!(
        device.witness.verified_mark(&device.vault_id()),
        Ok(Some(platform_traits::VerifiedMark::BIRTH))
    );

    assert_eq!(device.enable().unwrap(), EnableOutcome::Enabled);
    let genesis = device.genesis().unwrap();
    assert_eq!(
        genesis.membership_record, records.membership,
        "the same genesis"
    );
    assert_eq!(genesis.epoch_record, records.epoch);
    assert_ne!(
        device.witness.verified_mark(&device.vault_id()),
        Ok(Some(platform_traits::VerifiedMark::BIRTH)),
        "and the mark is recorded now"
    );
}

#[test]
fn a_crash_between_the_install_and_its_record_finishes_the_install_and_publishes_the_same_genesis()
{
    let device = Device::new();
    let records = mint(&device);
    persist(&device, &records);
    device.custody().install_genesis(&records).unwrap(); // the keys are in; the state still says Persisted
    assert_eq!(device.state(), Some(GenesisState::Persisted));

    assert_eq!(device.enable().unwrap(), EnableOutcome::Enabled);
    assert_eq!(device.genesis().unwrap().epoch_record, records.epoch);
    assert_eq!(device.service.published().len(), 1);
}

#[test]
fn a_crash_after_the_install_resumes_from_installed() {
    let device = Device::new();
    device
        .service
        .script(Fault::Refuse(ServiceError::Unavailable));
    assert!(matches!(
        device.enable(),
        Err(EnableError::Publish(PublishError::Service(
            ServiceError::Unavailable
        )))
    ));
    assert_eq!(device.state(), Some(GenesisState::Installed));
    let installed = device.genesis().unwrap();

    assert_eq!(device.enable().unwrap(), EnableOutcome::Enabled);
    assert_eq!(
        device.genesis().unwrap().epoch_record,
        installed.epoch_record
    );
    assert_eq!(device.service.published().len(), 1);
}

// ---- a lost response ---------------------------------------------------------------------------

#[test]
fn a_lost_install_response_resumes_to_the_original_install_and_mints_nothing() {
    let device = Device::new();
    device.service.script(Fault::Pass); // the stage
    device.service.script(Fault::LoseResponse); // the install takes effect; the answer is lost
    assert!(matches!(
        device.enable(),
        Err(EnableError::Publish(PublishError::Service(
            ServiceError::Unavailable
        )))
    ));
    assert!(device.service.knows_vault());
    assert_eq!(device.state(), Some(GenesisState::Installed));
    assert_eq!(
        device.saved_files(),
        1,
        "the container is kept for the retry"
    );
    let before = device.genesis().unwrap();
    assert_eq!(device.counter(), Some(1));

    assert_eq!(device.enable().unwrap(), EnableOutcome::Enabled);
    assert_eq!(
        device.genesis().unwrap().epoch_record,
        before.epoch_record,
        "nothing was minted"
    );
    assert_eq!(device.service.published().len(), 1);
    assert_eq!(
        device.counter(),
        Some(1),
        "the same bytes were resent: no second counter"
    );
    assert_eq!(device.saved_files(), 0);
}

#[test]
fn the_resume_sends_the_saved_bytes_even_if_the_vault_changed_meanwhile() {
    let device = Device::new();
    device.service.script(Fault::Pass);
    device.service.script(Fault::LoseResponse);
    assert!(device.enable().is_err());
    let exported_before = device.saved_files();
    assert_eq!(exported_before, 1);

    // The user edits the vault between the crash and the resume. Re-exporting would build a
    // different snapshot under the id the service already holds.
    worker_makes_an_account(&device);

    assert_eq!(device.enable().unwrap(), EnableOutcome::Enabled);
    assert_eq!(device.service.published().len(), 1);
    assert_eq!(device.counter(), Some(1), "no second seal");
}

// ---- the vault the service already knows (R39) -------------------------------------------------

#[test]
fn a_vault_the_service_knows_ends_as_vault_known_with_nothing_published_or_discarded() {
    let device = Device::new();
    // A fresh vault with data (a command dispatched before any sync_base) whose vault_id the
    // service already knows, because another device enabled Sync for it with a genesis of its own.
    worker_makes_an_account(&device);
    let (log, accounts) = (device.count("operation_log"), device.count("accounts"));
    assert_eq!(device.count("sync_base") + device.count("sync_outbox"), 0);
    let other = create_genesis(
        device.vault_id(),
        [0xee; 16],
        &DeviceKeys::generate().unwrap(),
        1_700_000_000,
    )
    .unwrap();
    device.service.seed_vault(other.membership, other.epoch);
    let seeded_calls = device.service.calls().len();

    assert_eq!(device.enable().unwrap(), EnableOutcome::VaultKnown);
    assert_eq!(device.state(), Some(GenesisState::VaultKnown));
    assert!(
        device.service.published().is_empty(),
        "nothing was published"
    );
    assert_eq!(device.saved_files(), 0);
    assert_eq!(
        (device.count("operation_log"), device.count("accounts")),
        (log, accounts),
        "nothing in the vault was discarded"
    );

    // A restart after VaultKnown goes straight to the terminal state: no request, no new genesis.
    let calls_after = device.service.calls().len();
    assert!(calls_after > seeded_calls);
    let genesis = device.genesis().unwrap();
    assert_eq!(device.enable().unwrap(), EnableOutcome::VaultKnown);
    assert_eq!(device.service.calls().len(), calls_after, "no service call");
    assert_eq!(device.genesis().unwrap(), genesis, "never re-minted");
}

#[test]
fn a_vault_that_has_synced_before_is_refused_before_anything_is_minted_or_sent() {
    // A restored copy carries its sync_base row (a backup includes it) and an unpushed command.
    for rows in ["base", "outbox", "both"] {
        let device = Device::new();
        worker_makes_an_account(&device);
        let conn = device.worker.read_connection().unwrap();
        if rows != "outbox" {
            conn.execute(
                "INSERT INTO sync_base (singleton, base_seq, base_digest, state, updated_at)
                 VALUES (1, 7, zeroblob(32), 'Ready', '2026-11-06T00:00:00Z')",
                [],
            )
            .unwrap();
        }
        if rows == "outbox" {
            conn.execute(
                "INSERT INTO sync_outbox (command_id, op_seq, state, write_set_keys, created_at)
                 VALUES (zeroblob(16), 1, 'Pending', zeroblob(0), '2026-11-06T00:00:00Z')",
                [],
            )
            .unwrap();
        }
        if rows == "both" {
            worker_makes_an_account(&device);
        }
        let (log, outbox, accounts) = (
            device.count("operation_log"),
            device.count("sync_outbox"),
            device.count("accounts"),
        );

        let result = device.enable();
        assert!(
            matches!(result, Err(EnableError::LiveVaultHasSyncState)),
            "{rows}: {result:?}"
        );
        assert!(device.service.calls().is_empty(), "{rows}: no service call");
        assert!(device.genesis().is_none(), "{rows}: nothing minted");
        assert!(
            !device.custody().has_device_keys().unwrap(),
            "{rows}: no device keys made"
        );
        assert_eq!(
            device.witness.verified_mark(&device.vault_id()),
            Ok(None),
            "{rows}: the witness untouched"
        );
        assert_eq!(
            (
                device.count("operation_log"),
                device.count("sync_outbox"),
                device.count("accounts")
            ),
            (log, outbox, accounts),
            "{rows}: nothing discarded"
        );
    }
}

#[test]
fn a_persisted_genesis_is_not_exported_from_a_vault_that_has_gained_sync_state() {
    let device = Device::new();
    // Persisted and installed by hand, with nothing sealed: the retry would export the live vault.
    let records = mint(&device);
    persist(&device, &records);
    device.custody().install_genesis(&records).unwrap();
    device
        .worker
        .write_sync_state(|tx| {
            sync_genesis::advance(tx, GenesisState::Installed, "2026-11-06T00:00:00+00:00")
        })
        .unwrap();
    assert_eq!(device.saved_files(), 0);
    device
        .worker
        .read_connection()
        .unwrap()
        .execute(
            "INSERT INTO sync_base (singleton, base_seq, base_digest, state, updated_at)
             VALUES (1, 7, zeroblob(32), 'Ready', '2026-11-06T00:00:00Z')",
            [],
        )
        .unwrap();
    let calls = device.service.calls().len();

    assert!(matches!(
        device.enable(),
        Err(EnableError::LiveVaultHasSyncState)
    ));
    assert_eq!(device.service.calls().len(), calls, "no request");
    assert_eq!(device.state(), Some(GenesisState::Installed));
}

// ---- a resume that cannot send what the service holds ------------------------------------------

/// Delete the saved snapshot files, as a lost disk would.
fn lose_the_saved_container(device: &Device) {
    let dir = saved_snapshots_dir(device.worker.db_path());
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        std::fs::remove_file(entry.path()).unwrap();
    }
}

#[test]
fn a_fresh_seal_over_a_snapshot_the_service_staged_stops_in_the_terminal_state_once() {
    let device = Device::new();
    device.service.script(Fault::Pass); // the stage takes effect
    device
        .service
        .script(Fault::Refuse(ServiceError::Unavailable)); // the install does not arrive
    assert!(device.enable().is_err());
    assert_eq!(device.counter(), Some(1));
    lose_the_saved_container(&device);

    // The retry would seal a different container under the same id: the stage refuses it.
    assert_eq!(device.enable().unwrap(), EnableOutcome::GenesisResumeLost);
    assert_eq!(device.state(), Some(GenesisState::GenesisResumeLost));
    assert!(
        device.service.published().is_empty(),
        "nothing was published"
    );
    assert_eq!(
        device.counter(),
        Some(2),
        "one counter was spent finding out"
    );
    assert_eq!(
        device.saved_files(),
        0,
        "and the container it sealed is dropped"
    );

    // A restart makes no call and spends nothing.
    let calls = device.service.calls().len();
    assert_eq!(device.enable().unwrap(), EnableOutcome::GenesisResumeLost);
    assert_eq!(device.service.calls().len(), calls);
    assert_eq!(device.counter(), Some(2));
}

#[test]
fn a_fresh_seal_after_the_genesis_was_installed_stops_whatever_credential_it_holds() {
    for credential in [Credential::Account, Credential::Device] {
        let device = Device::new();
        device.service.script(Fault::Pass); // the stage
        device.service.script(Fault::LoseResponse); // the install takes effect; the answer is lost
        assert!(device.enable().is_err());
        assert!(device.service.knows_vault());
        lose_the_saved_container(&device);
        device.service.set_credential(credential);

        assert_eq!(
            device.enable().unwrap(),
            EnableOutcome::GenesisResumeLost,
            "{credential:?}"
        );
        assert_eq!(device.state(), Some(GenesisState::GenesisResumeLost));
        assert_eq!(
            device.service.published().len(),
            1,
            "the service's own snapshot"
        );
    }
}

#[test]
fn a_saved_container_reaches_published_whatever_the_stage_answers() {
    // The install took effect and its answer was lost; the saved bytes are what the service holds.
    for (credential, stage_fault) in [
        (Credential::Account, None),
        (Credential::Device, None),
        (
            Credential::Device,
            Some(Fault::Refuse(ServiceError::ObjectConflict)),
        ),
    ] {
        let device = Device::new();
        device.service.script(Fault::Pass);
        device.service.script(Fault::LoseResponse);
        assert!(device.enable().is_err());
        device.service.set_credential(credential);
        if let Some(fault) = stage_fault.clone() {
            device.service.script(fault);
        }

        assert_eq!(
            device.enable().unwrap(),
            EnableOutcome::Enabled,
            "{credential:?} {stage_fault:?}"
        );
        assert_eq!(device.state(), Some(GenesisState::Published));
        assert_eq!(device.service.published().len(), 1);
        assert_eq!(
            device.counter(),
            Some(1),
            "the same bytes: no second counter"
        );
    }
}

#[test]
fn a_published_vault_with_a_leftover_saved_file_makes_no_request_and_reseals_nothing() {
    // The outcome is recorded before the saved container is dropped, so a crash between the two
    // leaves a Published vault and a leftover file. The next call reads the state first.
    let device = Device::new();
    assert_eq!(device.enable().unwrap(), EnableOutcome::Enabled);
    assert_eq!(device.state(), Some(GenesisState::Published));
    let dir = saved_snapshots_dir(device.worker.db_path());
    std::fs::write(
        dir.join("00112233445566778899aabbccddeeff.snapshot"),
        b"leftover",
    )
    .unwrap();
    device.service.set_credential(Credential::Device);
    let calls = device.service.calls().len();

    assert_eq!(device.enable().unwrap(), EnableOutcome::Enabled);
    assert_eq!(device.service.calls().len(), calls, "no request");
    assert_eq!(device.counter(), Some(1), "nothing was sealed again");
}

// ---- the refusals that come first --------------------------------------------------------------

#[test]
fn a_halted_replica_does_not_start_enabling() {
    let device = Device::new();
    let halted = SyncStatus::Halted(HaltEvidence::Fork {
        version: 2,
        accepted: [1; 32],
        conflicting: [2; 32],
    });
    let result = enable_sync(
        &device.worker,
        &device.secrets,
        &device.witness,
        &device.service,
        &halted,
        &DEVICE,
    );
    assert!(matches!(
        result,
        Err(EnableError::Publish(PublishError::Halted))
    ));
    assert!(device.genesis().is_none(), "nothing was minted");
    assert!(device.service.calls().is_empty());
    assert_eq!(device.witness.verified_mark(&device.vault_id()), Ok(None));
}

#[test]
fn a_restored_vault_stops_before_any_request() {
    let device = Device::new();
    let backup = device.dir.path().join("before.db");
    std::fs::write(&backup, device.worker.snapshot_bytes().unwrap()).unwrap();
    assert_eq!(device.enable().unwrap(), EnableOutcome::Enabled);
    let calls = device.service.calls().len();

    // The vault is restored from before the genesis; the witness, which a restore does not touch,
    // has seen the genesis's mark.
    let restored = DbWorker::open(&backup, common::KEY).unwrap();
    assert!(restored
        .write_sync_state(|tx| sync_genesis::load(tx))
        .unwrap()
        .is_none());
    let result = enable_sync(
        &restored,
        &device.secrets,
        &device.witness,
        &device.service,
        &SyncStatus::Healthy,
        &DEVICE,
    );
    assert!(
        matches!(
            result,
            Err(EnableError::Verified(VerifiedStateError::StateRolledBack))
        ),
        "{result:?}"
    );
    assert_eq!(device.service.calls().len(), calls, "no request");
}

#[test]
fn a_restored_register_stops_the_publish_with_nonce_state_lost() {
    let device = Device::new();
    device
        .service
        .script(Fault::Refuse(ServiceError::Unavailable));
    assert!(device.enable().is_err());
    // The witness has seen a counter the register has not.
    let keys = device.custody().load_device_keys().unwrap();
    device
        .witness
        .advance_counter(
            &CounterContext {
                vault_id: device.vault_id(),
                key_epoch: 1,
                fingerprint: *keys.public().fingerprint.as_bytes(),
                object_type: 2,
            },
            5,
        )
        .unwrap();
    let calls = device.service.calls().len();
    let result = device.enable();
    assert!(
        matches!(
            result,
            Err(EnableError::Publish(PublishError::Snapshot(
                SnapshotError::Nonce(NonceError::NonceStateLost(
                    LostReason::RegisterBehindWitness
                ))
            )))
        ),
        "{result:?}"
    );
    assert_eq!(device.service.calls().len(), calls);
}
