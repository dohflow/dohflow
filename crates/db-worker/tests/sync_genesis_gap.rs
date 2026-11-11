//! Arming a founding device whose vault changed after its genesis was exported
//! (`db_worker::sync_genesis_gap`, personal-cfo-1cltt.5 slice C): the base is the fetched genesis,
//! the gap is replayed on a scratch copy and proved equal to the log (C2), every refusal happens
//! before anything is written to the live vault, and arming is one transaction (C4).

#![allow(dead_code)]

mod common;
#[path = "../../vault-crypto/tests/common/snapshot_service.rs"]
mod snapshot_service;

use std::path::PathBuf;

use common::{create_account_cmd, income_cmd, meta};
use core_money::{Currency, Money};
use db_worker::sync_enable::{enable_sync, EnableOutcome, EnablingDevice};
use db_worker::sync_genesis::{self, GenesisState};
use db_worker::sync_genesis_gap::{gap_dir, reconcile_genesis_gap, Armed, GapError, GenesisProof};
use db_worker::{DbWorker, WriteCommand};
use platform_mock::{MockSecretStore, MockWitness};
use snapshot_service::{Credential, FakeSnapshotService};
use sync_core::membership::verify_chain;
use sync_core::status::{EngineVersion, SyncStatus};
use vault_crypto::sync_custody::SyncKeyCustody;

const DEVICE: EnablingDevice = EnablingDevice {
    device_id: [0xa1; 16],
    issued_at: 1_700_000_000,
};

/// A founding device with a raw-keyed vault (the scratch vaults are keyed like the live one).
struct Device {
    dir: tempfile::TempDir,
    worker: std::sync::Arc<DbWorker>,
    secrets: MockSecretStore,
    witness: MockWitness,
    service: FakeSnapshotService,
}

impl Device {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let worker = std::sync::Arc::new(
            DbWorker::open_with_raw_key(
                dir.path().join("test.vault"),
                vault_crypto::generate_dek().unwrap(),
            )
            .unwrap(),
        );
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

    fn enable(&self) -> EnableOutcome {
        enable_sync(
            &self.worker,
            &self.secrets,
            &self.witness,
            &self.service,
            &SyncStatus::Healthy,
            &DEVICE,
        )
        .unwrap()
    }

    fn count(&self, table: &str) -> i64 {
        self.worker
            .read_connection()
            .unwrap()
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    /// Reconcile with the proof the engine would supply from its custody.
    fn reconcile(&self) -> Result<Armed, GapError> {
        let genesis = self
            .worker
            .write_sync_state(|tx| sync_genesis::load(tx))
            .unwrap()
            .unwrap();
        let chain = verify_chain(
            &self.vault_id(),
            &[genesis.membership_record.as_slice()],
            &[genesis.epoch_record.as_slice()],
        )
        .unwrap();
        let key = SyncKeyCustody::new(&self.secrets, self.vault_id())
            .load_epoch_key(&chain, 1)
            .unwrap();
        let own = EngineVersion::new(db_worker::ENGINE_VERSION).unwrap();
        self.service.set_credential(Credential::Device);
        reconcile_genesis_gap(
            &self.worker,
            &self.service,
            &GenesisProof {
                epoch_key: &key,
                chain: &chain,
                own_engine_version: &own,
            },
        )
    }

    /// The vault file's neighbours that a failed reconcile must not leave behind.
    fn leftovers(&self) -> bool {
        gap_dir(self.worker.db_path()).exists()
    }
}

/// A vault with data, a published genesis, and then three commands the genesis does not carry:
/// each reads the instant it was issued (an id mint, a stamp, a tombstone).
fn device_with_a_gap() -> (Device, Vec<WriteCommand>) {
    let device = Device::new();
    device
        .worker
        .dispatch(meta(), create_account_cmd())
        .unwrap();
    assert_eq!(device.enable(), EnableOutcome::Enabled);
    let account = common::sample_account();
    let id = account.id();
    let commands = vec![
        WriteCommand::CreateAccount {
            account: Box::new(account),
            opening_balance: Some(Money::new(50_000, Currency::Usd)),
        },
        income_cmd(Some(id)),
        WriteCommand::ArchiveAccount(id),
    ];
    for command in &commands {
        device.worker.dispatch(meta(), command.clone()).unwrap();
    }
    assert!(matches!(
        device.enable(),
        EnableOutcome::EnabledWithLocalChanges { .. }
    ));
    (device, commands)
}

#[test]
fn a_gap_is_replayed_proved_and_armed_in_log_order_on_the_fetched_genesis() {
    let (device, commands) = device_with_a_gap();
    let before: Vec<(i64, Vec<u8>)> = {
        let conn = device.worker.read_connection().unwrap();
        let mut stmt = conn
            .prepare("SELECT op_seq, command_id FROM operation_log ORDER BY op_seq")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    let armed = device.reconcile().unwrap();
    assert_eq!(
        armed,
        Armed {
            commands: commands.len(),
            base_seq: 0
        }
    );

    // The outbox holds the gap, in log order, Pending, with the issue times the log recorded.
    let conn = device.worker.read_connection().unwrap();
    let rows: Vec<(Vec<u8>, i64, String, i64)> = {
        let mut stmt = conn
            .prepare(
                "SELECT o.command_id, o.op_seq, o.state, o.issued_at
                   FROM sync_outbox o ORDER BY o.op_seq",
            )
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(rows.len(), commands.len());
    let gap_ids: Vec<&Vec<u8>> = before[before.len() - commands.len()..]
        .iter()
        .map(|(_, id)| id)
        .collect();
    assert_eq!(rows.iter().map(|r| &r.0).collect::<Vec<_>>(), gap_ids);
    assert!(rows.iter().all(|r| r.2 == "Pending"));
    for (command_id, _, _, issued) in &rows {
        let logged: i64 = conn
            .query_row(
                "SELECT issued_at FROM operation_log WHERE command_id = ?1",
                [command_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            *issued, logged,
            "the outbox row carries the log's issue time"
        );
    }

    // The device is armed on the genesis: base at 0, ready, the file in place, and the cursor of
    // an origin.
    let (seq, state): (i64, String) = conn
        .query_row(
            "SELECT base_seq, state FROM sync_base WHERE singleton = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((seq, state.as_str()), (0, "Ready"));
    assert!(db_worker::base_image::base_image_path(device.worker.db_path()).exists());
    assert_eq!(
        db_worker::sync_genesis_gap::armed_position(&conn).unwrap(),
        Some((None, 1))
    );
    assert!(matches!(
        device.worker.base_image_status().unwrap(),
        db_worker::base_image::BaseImageStatus::Ready { base_seq: 0 }
    ));
    assert!(!device.leftovers(), "the work directory is gone");
    let _: PathBuf = device.dir.path().to_path_buf();
}

#[test]
fn a_vault_with_no_gap_is_armed_on_its_genesis_with_an_empty_outbox() {
    let device = Device::new();
    device
        .worker
        .dispatch(meta(), create_account_cmd())
        .unwrap();
    assert_eq!(device.enable(), EnableOutcome::Enabled);
    assert_eq!(
        device.reconcile().unwrap(),
        Armed {
            commands: 0,
            base_seq: 0
        }
    );
    assert_eq!(device.count("sync_outbox"), 0);
    assert_eq!(device.count("sync_base"), 1);
}

/// What a refused reconcile must leave: nothing armed, nothing in the outbox, no work directory.
fn assert_untouched(device: &Device) {
    assert_eq!(device.count("sync_base"), 0, "not armed");
    assert_eq!(device.count("sync_outbox"), 0, "no outbox row");
    assert_eq!(device.count("sync_position"), 0);
    assert!(!device.leftovers());
    assert!(!db_worker::base_image::base_image_path(device.worker.db_path()).exists());
}

#[test]
fn a_gap_command_with_no_recorded_issue_time_refuses_by_name_before_anything_is_enqueued() {
    let (device, _) = device_with_a_gap();
    // A row from before the column: the trigger that keeps the log append-only is lifted for the
    // one row, as a vault that predates the column would simply have NULL there.
    let victim: Vec<u8> = {
        let conn = device.worker.read_connection().unwrap();
        conn.execute_batch("DROP TRIGGER operation_log_no_update;")
            .unwrap();
        let victim: Vec<u8> = conn
            .query_row(
                "SELECT command_id FROM operation_log ORDER BY op_seq DESC LIMIT 1 OFFSET 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        conn.execute(
            "UPDATE operation_log SET issued_at = NULL WHERE command_id = ?1",
            [&victim],
        )
        .unwrap();
        victim
    };
    let error = device.reconcile().unwrap_err();
    match error {
        GapError::MissingIssuedAt { command_id } => {
            assert_eq!(command_id.as_bytes().as_slice(), victim.as_slice());
        }
        other => panic!("{other:?}"),
    }
    assert_untouched(&device);
}

#[test]
fn a_tampered_issue_time_is_refused_because_the_replay_no_longer_reproduces_the_log() {
    let (device, _) = device_with_a_gap();
    // Move the recorded issue time of every gap row by a day: the stamps `apply` derives from it
    // (the archive stamp, the opening entry's time) differ, so the write-set hash does.
    {
        let conn = device.worker.read_connection().unwrap();
        conn.execute_batch("DROP TRIGGER operation_log_no_update;")
            .unwrap();
        conn.execute(
            "UPDATE operation_log SET issued_at = issued_at + 86400000000
              WHERE op_seq > (SELECT export_high_water FROM sync_genesis)",
            [],
        )
        .unwrap();
    }
    let error = device.reconcile().unwrap_err();
    assert!(
        matches!(error, GapError::ReplayDiverged { .. }),
        "{error:?}"
    );
    assert_untouched(&device);
}

#[test]
fn the_service_having_moved_past_the_genesis_refuses_before_a_byte_is_fetched() {
    let (device, _) = device_with_a_gap();
    device.service.advance_head_to(3);
    let calls_before = device.service.calls().len();
    let error = device.reconcile().unwrap_err();
    assert!(
        matches!(error, GapError::ServiceMovedPastGenesis { head: 3 }),
        "{error:?}"
    );
    assert!(
        !device.service.calls()[calls_before..].contains(&"get_snapshot"),
        "nothing was fetched"
    );
    assert_untouched(&device);
}

#[test]
fn a_device_that_is_already_armed_is_refused() {
    let (device, _) = device_with_a_gap();
    device.reconcile().unwrap();
    assert!(matches!(
        device.reconcile().unwrap_err(),
        GapError::AlreadyArmed
    ));
}

#[test]
fn a_genesis_that_did_not_end_published_is_refused_whatever_the_service_says() {
    let (device, _) = device_with_a_gap();
    device
        .worker
        .read_connection()
        .unwrap()
        .execute_batch("UPDATE sync_genesis SET state = 'VaultKnown';")
        .unwrap();
    assert!(matches!(
        device.reconcile().unwrap_err(),
        GapError::GenesisNotPublished(Some(GenesisState::VaultKnown))
    ));
    assert_untouched(&device);
}

#[test]
fn a_command_taken_while_the_reconciliation_ran_aborts_the_arming_and_writes_nothing() {
    let (device, _) = device_with_a_gap();
    // The device keeps working while the genesis is fetched: a command lands after the gap was
    // read. The arming re-reads the log's high-water mark in its own transaction and refuses.
    let worker = std::sync::Arc::clone(&device.worker);
    device.service.before("get_snapshot", move || {
        worker.dispatch(meta(), create_account_cmd()).unwrap();
    });
    let error = device.reconcile().unwrap_err();
    assert!(matches!(error, GapError::LogMoved { .. }), "{error:?}");
    assert_untouched(&device);
    // A second attempt reads the longer log and arms.
    let armed = device.reconcile().unwrap();
    assert_eq!(armed.commands, 4);
}

#[test]
fn the_arming_transaction_re_reads_the_log_and_a_stale_mark_writes_nothing() {
    let (device, _) = device_with_a_gap();
    let image = {
        // A real genesis image, as the reconciliation would have built it.
        let work = gap_dir(device.worker.db_path());
        std::fs::create_dir_all(&work).unwrap();
        let image = work.join("image.db");
        std::fs::copy(
            // Any image file will do for the stale-mark path: it is refused before it is placed.
            device.worker.db_path(),
            &image,
        )
        .unwrap();
        image
    };
    let mark = {
        let conn = device.worker.read_connection().unwrap();
        let high: i64 = conn
            .query_row("SELECT MAX(op_seq) FROM operation_log", [], |r| r.get(0))
            .unwrap();
        u64::try_from(high).unwrap() - 1
    };
    let error =
        db_worker::sync_genesis_gap::arm(&device.worker, &image, mark, &[], &[]).unwrap_err();
    assert!(matches!(error, GapError::LogMoved { .. }), "{error:?}");
    std::fs::remove_dir_all(gap_dir(device.worker.db_path())).unwrap();
    assert_untouched(&device);
}

#[test]
fn a_genesis_recorded_by_an_earlier_build_has_no_identity_and_is_not_trusted() {
    let (device, _) = device_with_a_gap();
    // An earlier build recorded the genesis as published, with no snapshot identity.
    device
        .worker
        .read_connection()
        .unwrap()
        .execute_batch(
            "UPDATE sync_genesis SET genesis_snapshot_id = NULL, genesis_container_sha256 = NULL;",
        )
        .unwrap();
    assert!(matches!(
        device.reconcile().unwrap_err(),
        GapError::GenesisIdentityNotRecorded
    ));
    assert_untouched(&device);
    let state = device
        .worker
        .write_sync_state(|tx| sync_genesis::load(tx))
        .unwrap()
        .unwrap()
        .state;
    assert_eq!(state, GenesisState::Published);
}

#[test]
fn a_recorded_hash_the_replay_does_not_produce_is_refused_even_when_the_state_matches() {
    let (device, _) = device_with_a_gap();
    // Only the recorded hash of one gap row is wrong: the replay reproduces the state exactly, but
    // not what the log says it produced, so the command cannot be sent as the log describes it.
    let victim: Vec<u8> = {
        let conn = device.worker.read_connection().unwrap();
        conn.execute_batch("DROP TRIGGER operation_log_no_update;")
            .unwrap();
        let victim: Vec<u8> = conn
            .query_row(
                "SELECT command_id FROM operation_log ORDER BY op_seq DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        conn.execute(
            "UPDATE operation_log SET write_set_hash = zeroblob(32) WHERE command_id = ?1",
            [&victim],
        )
        .unwrap();
        victim
    };
    match device.reconcile().unwrap_err() {
        GapError::ReplayDiverged { command_id } => {
            assert_eq!(command_id.as_bytes().as_slice(), victim.as_slice());
        }
        other => panic!("{other:?}"),
    }
    assert_untouched(&device);
}

#[test]
fn a_live_change_the_log_does_not_explain_is_a_state_divergence_naming_the_table() {
    let (device, _) = device_with_a_gap();
    // Every hash matches (the commands replay exactly), but the live vault holds a change that no
    // command made: its class-1 state is not the log's.
    device
        .worker
        .read_connection()
        .unwrap()
        .execute_batch("UPDATE accounts SET name = 'Edited outside a command';")
        .unwrap();
    match device.reconcile().unwrap_err() {
        GapError::StateDiverged { table } => assert_eq!(table, "accounts"),
        other => panic!("{other:?}"),
    }
    assert_untouched(&device);
}
