//! A snapshot after the genesis is never read from the live vault (personal-cfo-1cltt.4 part B).
//!
//! With Sync enabled, a local command is an unpushed outbox entry: the live vault holds its effect,
//! and no envelope has sequenced it. A re-snapshot that truncates a tail must equal the log at its
//! head, so a publish whose source is the live vault is refused, here with a real vault that has a
//! `Pending` outbox row, the SQLite register, and a service that records every call.

mod common;

use std::sync::Mutex;

use common::{create_account_cmd, meta, worker};
use db_worker::base_image::saved_snapshots_dir;
use db_worker::logical_export::{export_genesis_plaintext, ExportInput};
use db_worker::sync_register::SqliteRegisterStore;
use platform_mock::MockWitness;
use sync_core::artifact::ArtifactId;
use sync_core::membership::verify_chain;
use sync_core::snapshot_service::{
    EpochInstall, InstalledEpoch, PublishedSnapshot, ServiceError, SnapshotService, SnapshotSource,
    StagedSnapshot,
};
use sync_core::status::SyncStatus;
use vault_crypto::sync_counter::{CounterRegister, RegisterStore};
use vault_crypto::sync_envelope::SealerIdentity;
use vault_crypto::sync_keys::DeviceKeys;
use vault_crypto::sync_records::{create_genesis, fresh_epoch_key};
use vault_crypto::sync_snapshot::{SavedSnapshotFiles, SnapshotFields, SnapshotSealer};
use vault_crypto::sync_snapshot_flow::{publish, PublishError, PublishKind, PublishRequest};

const VAULT: [u8; 16] = [0x11; 16];

/// A service that counts its calls and never accepts anything.
#[derive(Default)]
struct RecordingService(Mutex<Vec<&'static str>>);

impl RecordingService {
    fn calls(&self) -> Vec<&'static str> {
        self.0.lock().unwrap().clone()
    }

    fn record(&self, name: &'static str) -> ServiceError {
        self.0.lock().unwrap().push(name);
        ServiceError::Unavailable
    }
}

impl SnapshotService for RecordingService {
    fn stage_snapshot(
        &self,
        _: &[u8; 16],
        _: &[u8; 16],
        _: &[u8],
    ) -> Result<StagedSnapshot, ServiceError> {
        Err(self.record("stage_snapshot"))
    }

    fn stage_snapshot_closure(
        &self,
        _: &[u8; 16],
        _: &[u8; 16],
        _: &[u8],
    ) -> Result<sync_core::snapshot_service::StagedClosure, ServiceError> {
        Err(self.record("stage_snapshot_closure"))
    }

    fn get_snapshot_closure(&self, _: &[u8; 16], _: &[u8; 16]) -> Result<Vec<u8>, ServiceError> {
        Err(self.record("get_snapshot_closure"))
    }

    fn publish_snapshot(
        &self,
        _: &[u8; 16],
        _: &[u8; 16],
        _: u64,
        _: &[ArtifactId],
    ) -> Result<PublishedSnapshot, ServiceError> {
        Err(self.record("publish_snapshot"))
    }

    fn install_epoch(
        &self,
        _: &[u8; 16],
        _: &EpochInstall<'_>,
    ) -> Result<InstalledEpoch, ServiceError> {
        Err(self.record("install_epoch"))
    }

    fn latest_snapshot(&self, _: &[u8; 16]) -> Result<Option<PublishedSnapshot>, ServiceError> {
        Err(self.record("latest_snapshot"))
    }

    fn log_head(&self, _: &[u8; 16]) -> Result<u64, ServiceError> {
        Err(self.record("log_head"))
    }

    fn get_snapshot<'a>(
        &'a self,
        _: &[u8; 16],
        _: &[u8; 16],
    ) -> Result<Box<dyn std::io::Read + 'a>, ServiceError> {
        Err(self.record("get_snapshot"))
    }
}

#[test]
fn a_re_snapshot_from_the_live_vault_is_refused_while_an_outbox_row_is_pending() {
    let (dir, worker) = worker();
    let conn = worker.read_connection().unwrap();
    conn.execute(
        "INSERT INTO sync_base (singleton, base_seq, base_digest, state, updated_at)
         VALUES (1, 7, zeroblob(32), 'Ready', '2026-11-06T00:00:00Z')",
        [],
    )
    .unwrap();
    worker.dispatch(meta(), create_account_cmd()).unwrap();
    let pending: i64 = conn
        .query_row(
            "SELECT count(*) FROM sync_outbox WHERE state = 'Pending'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(pending, 1, "a local command is waiting for its envelope");
    let live = export_genesis_plaintext(&conn, &ExportInput::default()).unwrap();

    // The device's identity, key and chain.
    let keys = DeviceKeys::generate().unwrap();
    let genesis = create_genesis(VAULT, [0xa1; 16], &keys, 1_700_000_000).unwrap();
    let chain = verify_chain(
        &VAULT,
        &[genesis.membership.as_slice()],
        &[genesis.epoch.as_slice()],
    )
    .unwrap();
    let key = fresh_epoch_key(VAULT, 1).unwrap();
    let identity = SealerIdentity {
        vault_id: VAULT,
        device_id: [0xa1; 16],
        fingerprint: keys.public().fingerprint,
    };
    let snapshots = saved_snapshots_dir(&dir.path().join("test.vault"));
    std::fs::create_dir_all(&snapshots).unwrap();
    let files = SavedSnapshotFiles::new(&snapshots);
    let store = SqliteRegisterStore::new(&worker);
    let witness = MockWitness::new();
    let register = CounterRegister::new(&store, &witness);
    let sealer = SnapshotSealer {
        register: &register,
        files: &files,
        key: &key,
        sealer: &identity,
        chain: &chain,
    };
    let service = RecordingService::default();
    let request = |source| PublishRequest {
        kind: PublishKind::ReSnapshot,
        source,
        fields: SnapshotFields {
            object_id: [0x02; 16],
            base_seq: 7,
            payload_schema_version: 1,
            engine_version: "dohflow-test".to_owned(),
            schema_version: 67,
        },
        plaintext: &live,
    };

    // The live vault as the source: refused before a counter, a file or a request.
    let result = publish(
        &service,
        &sealer,
        &SyncStatus::Healthy,
        &request(SnapshotSource::LiveVault),
    );
    assert_eq!(result, Err(PublishError::LiveVaultSource));
    assert!(service.calls().is_empty(), "no request was made");
    assert!(
        std::fs::read_dir(&snapshots).unwrap().next().is_none(),
        "no container was saved"
    );
    let context = platform_traits::CounterContext {
        vault_id: VAULT,
        key_epoch: 1,
        fingerprint: *identity.fingerprint.as_bytes(),
        object_type: 2,
    };
    assert_eq!(
        store.next_counter(&context),
        Ok(None),
        "no counter was spent"
    );

    // The refusal keys on the source, not on the bytes: the same bytes tagged as a state at the
    // head go on to be sealed and sent (the fake service here refuses, to stop the run).
    let result = publish(
        &service,
        &sealer,
        &SyncStatus::Healthy,
        &request(SnapshotSource::StateAtHead { head: 7 }),
    );
    assert_eq!(
        result,
        Err(PublishError::Service(ServiceError::Unavailable))
    );
    assert_eq!(service.calls(), ["stage_snapshot"]);
    assert_eq!(store.next_counter(&context), Ok(Some(1)));
}
