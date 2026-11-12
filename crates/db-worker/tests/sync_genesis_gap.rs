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
use db_worker::sync_genesis_gap::{
    gap_dir, reconcile_genesis_gap_with, Armed, GapError, GenesisProof,
};
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
        self.reconcile_with(&mut || {})
    }

    /// As [`Self::reconcile`], running `between` at the moment a command on the live vault would
    /// race the end-state check.
    fn reconcile_with(&self, between: &mut dyn FnMut()) -> Result<Armed, GapError> {
        self.with_proof(|proof| {
            reconcile_genesis_gap_with(&self.worker, &self.service, proof, between)
        })
    }

    /// Run `f` with the proof the engine would supply from its custody.
    fn with_proof<R>(&self, f: impl FnOnce(&GenesisProof<'_>) -> R) -> R {
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
        f(&GenesisProof {
            epoch_key: &key,
            chain: &chain,
            own_engine_version: &own,
        })
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

#[test]
fn a_command_taken_between_the_mark_and_the_export_is_the_log_moving_and_a_re_run_arms() {
    let (device, _) = device_with_a_gap();
    // The command lands after the live log's mark was read and before its state was exported: it
    // must never be reported as a divergence of the replay (which would not be retried).
    let worker = std::sync::Arc::clone(&device.worker);
    let error = device
        .reconcile_with(&mut || {
            worker.dispatch(meta(), create_account_cmd()).unwrap();
        })
        .unwrap_err();
    assert!(matches!(error, GapError::LogMoved { .. }), "{error:?}");
    assert!(db_worker::sync_genesis_gap::is_log_moved(&error));
    assert_untouched(&device);
    // The retry reads the longer log, replays it, and arms.
    assert_eq!(device.reconcile().unwrap().commands, 4);
}

#[test]
fn stray_files_where_the_base_goes_do_not_stop_an_arming_that_a_crash_left_them_for() {
    let (device, commands) = device_with_a_gap();
    // A crash after a placed base and before its record left a file at the base path, and an
    // earlier attempt a half-written temporary: neither is recorded (no sync_base row).
    let base = db_worker::base_image::base_image_path(device.worker.db_path());
    let temp = db_worker::base_image::base_image_temp_path(device.worker.db_path());
    std::fs::create_dir_all(base.parent().unwrap()).unwrap();
    std::fs::write(&base, b"a base nobody recorded").unwrap();
    std::fs::write(&temp, b"half of a copy").unwrap();
    assert_eq!(device.count("sync_base"), 0);

    let armed = device.reconcile().unwrap();
    assert_eq!(armed.commands, commands.len());
    // The record names the new image's digest, and the file in place is that image.
    let recorded: Vec<u8> = device
        .worker
        .read_connection()
        .unwrap()
        .query_row(
            "SELECT base_digest FROM sync_base WHERE singleton = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    {
        use sha2::{Digest, Sha256};
        assert_eq!(
            recorded,
            Sha256::digest(std::fs::read(&base).unwrap()).to_vec()
        );
    }
    assert!(matches!(
        device.worker.base_image_status().unwrap(),
        db_worker::base_image::BaseImageStatus::Ready { base_seq: 0 }
    ));
    assert!(!temp.exists(), "the temporary is gone");
}

// ---- the command must be sendable ---------------------------------------------------------------

#[test]
fn a_gap_command_that_cannot_become_an_envelope_is_refused_by_name_before_anything_is_written() {
    let (device, _) = device_with_a_gap();
    // `dispatch` does not bound the actor id; a seal does (256 bytes). Such a command would be
    // queued and never sendable, so arming refuses it by name.
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
            "UPDATE operation_log SET actor_id = ?2 WHERE command_id = ?1",
            rusqlite::params![&victim, "a".repeat(257)],
        )
        .unwrap();
        victim
    };
    match device.reconcile().unwrap_err() {
        GapError::UnsealableCommand { command_id } => {
            assert_eq!(command_id.as_bytes().as_slice(), victim.as_slice());
        }
        other => panic!("{other:?}"),
    }
    assert_untouched(&device);
}

// ---- the service's genesis must be the one this device sent --------------------------------------

#[test]
fn a_service_snapshot_that_is_not_the_recorded_genesis_is_refused_before_a_byte_is_fetched() {
    let (device, _) = device_with_a_gap();
    // The device recorded another snapshot id than the one the service holds.
    device
        .worker
        .read_connection()
        .unwrap()
        .execute_batch(
            "UPDATE sync_genesis SET genesis_snapshot_id = x'ffffffffffffffffffffffffffffffff';",
        )
        .unwrap();
    let calls_before = device.service.calls().len();
    let error = device.reconcile().unwrap_err();
    assert!(
        matches!(error, GapError::ServiceGenesisIsNotOurs),
        "{error:?}"
    );
    assert!(
        !device.service.calls()[calls_before..].contains(&"get_snapshot"),
        "nothing was fetched"
    );
    assert_untouched(&device);
}

#[test]
fn bytes_that_are_not_the_ones_this_device_sent_are_refused_and_the_vault_is_untouched() {
    let (device, _) = device_with_a_gap();
    // The service serves other bytes under the genesis id: the recorded digest is not theirs.
    device
        .worker
        .read_connection()
        .unwrap()
        .execute_batch("UPDATE sync_genesis SET genesis_container_sha256 = zeroblob(32);")
        .unwrap();
    let error = device.reconcile().unwrap_err();
    assert!(
        matches!(
            error,
            GapError::Fetch(sync_core::snapshot_service::CopyError::DigestMismatch)
        ),
        "{error:?}"
    );
    assert_untouched(&device);
}

// ---- what the armed device is, and what another replica makes of it ------------------------------

/// The class-1 table entries of a vault's logical export.
fn class_one(conn: &rusqlite::Connection) -> Vec<sync_core::snapshot::TableEntry> {
    let plaintext = db_worker::logical_export::export_genesis_plaintext(
        conn,
        &db_worker::logical_export::ExportInput::default(),
    )
    .unwrap();
    sync_core::snapshot::verify(&plaintext)
        .unwrap()
        .manifest
        .tables
}

/// The genesis as a replica would fetch it: its verified snapshot and engine version.
fn fetched_genesis(device: &Device) -> (sync_core::snapshot::Verified, EngineVersion) {
    let genesis = device
        .worker
        .write_sync_state(|tx| sync_genesis::load(tx))
        .unwrap()
        .unwrap();
    let dest = device.dir.path().join("fetched.container");
    device.with_proof(|proof| {
        db_worker::sync_genesis_gap::fetch_genesis(
            &device.service,
            &device.vault_id(),
            &genesis.genesis_snapshot_id.unwrap(),
            &genesis.genesis_container_sha256.unwrap(),
            proof,
            &dest,
        )
        .unwrap()
    })
}

#[test]
fn the_armed_base_is_the_genesis_state_and_a_second_replica_applying_the_outbox_reaches_the_origin()
{
    let (device, commands) = device_with_a_gap();
    device.reconcile().unwrap();
    let (verified, engine) = fetched_genesis(&device);

    // (2) The base the device armed on holds exactly the genesis snapshot's class-1 state (state,
    // not file bytes: the keys differ).
    let base = db_worker::base_image::base_image_path(device.worker.db_path());
    let scratch_path = device.dir.path().join("inspect.db");
    let inspected = device
        .worker
        .scratch_from_image(&base, 0, &scratch_path)
        .unwrap();
    assert_eq!(
        class_one(&inspected.worker().read_connection().unwrap()),
        verified.manifest.tables,
        "the base is the genesis, not the live vault"
    );
    drop(inspected);

    // (1) A second replica, keyed independently, is born from the same fetched genesis and applies
    // the origin's Pending outbox in op_seq order. It reproduces every recorded write-set hash and
    // ends in the origin's class-1 state.
    let replica_dek = std::sync::Arc::new(vault_crypto::generate_dek().unwrap());
    let replica = db_worker::sync_bootstrap::bootstrap_replica(
        &device.dir.path().join("replica.vault"),
        &replica_dek,
        &db_worker::sync_bootstrap::ReplicaSource {
            vault_id: device.vault_id(),
            root: 0,
            snapshot: &verified,
            engine_version: &engine,
            own_engine_version: &EngineVersion::new(db_worker::ENGINE_VERSION).unwrap(),
            held_voids: &[],
        },
    )
    .unwrap();
    let outbox = device.worker.read_local_commands().unwrap();
    assert_eq!(outbox.len(), commands.len());
    assert!(outbox.windows(2).all(|w| w[0].old_op_seq < w[1].old_op_seq));
    for command in &outbox {
        replica
            .worker
            .dispatch(command.meta.clone(), command.cmd.clone())
            .unwrap();
        let produced: Vec<u8> = replica
            .worker
            .read_connection()
            .unwrap()
            .query_row(
                "SELECT write_set_hash FROM operation_log WHERE command_id = ?1",
                [command.meta.command_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            produced.as_slice(),
            command.write_set_hash.as_slice(),
            "the replica reproduces the recorded hash"
        );
    }
    assert_eq!(
        class_one(&replica.worker.read_connection().unwrap()),
        class_one(&device.worker.read_connection().unwrap()),
        "the second replica reaches the origin's state"
    );
}

// ---- an import in the gap ------------------------------------------------------------------------

/// The three commands of an import (a batch, a record, the commit that references the record),
/// each with fixed metadata so the same import can be issued on another vault.
fn import_commands(account: core_ledger::AccountId) -> Vec<(db_worker::CommandMeta, WriteCommand)> {
    use core_ledger::{SourceBatchId, SourceRecordId, TransactionId};
    let batch = SourceBatchId::new();
    let record = SourceRecordId::new();
    vec![
        (
            meta(),
            WriteCommand::CreateSourceBatch {
                id: batch,
                source_type: "csv".to_owned(),
                source_name: Some("statement.csv".to_owned()),
                file_fingerprint: None,
                parser_version: None,
                connector_connection_id: None,
            },
        ),
        (
            meta(),
            WriteCommand::AttachSourceRecord {
                id: record,
                batch_id: batch,
                external_id: None,
                source_hash: "hash-1".to_owned(),
                normalized_json: "{\"amount\":-1250}".to_owned(),
                parse_confidence_bps: None,
            },
        ),
        (
            meta(),
            WriteCommand::CommitImportedTransaction {
                transaction_id: TransactionId::new(),
                account_id: account,
                source_record_id: record,
                occurred_at: chrono::Utc::now(),
                amount: Money::new(-1_250, Currency::Usd),
                counterparty: None,
                memo: None,
                transaction_date: None,
                category_id: None,
                forced: false,
            },
        ),
    ]
}

/// A device whose vault holds `account` and whose genesis is published.
fn device_with_account(account: &WriteCommand) -> Device {
    let device = Device::new();
    device.worker.dispatch(meta(), account.clone()).unwrap();
    assert_eq!(device.enable(), EnableOutcome::Enabled);
    device
}

/// Each outbox row, in order: command id, state, issue time, write-set keys.
fn outbox_rows(device: &Device) -> Vec<(Vec<u8>, String, i64, Vec<u8>)> {
    outbox_rows_of(&device.worker)
}

fn outbox_rows_of(worker: &DbWorker) -> Vec<(Vec<u8>, String, i64, Vec<u8>)> {
    let conn = worker.read_connection().unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT command_id, state, issued_at, write_set_keys FROM sync_outbox ORDER BY op_seq",
        )
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn an_import_taken_in_the_gap_arms_as_the_ordinary_pending_import_it_would_have_been() {
    let account = common::sample_account();
    let account_id = account.id();
    let account_cmd = WriteCommand::CreateAccount {
        account: Box::new(account),
        opening_balance: None,
    };
    let import = import_commands(account_id);

    // The device under test takes the import after its genesis, unarmed (no outbox row), and is
    // then armed by the reconciliation.
    let gapped = device_with_account(&account_cmd);
    for (m, c) in &import {
        gapped.worker.dispatch(m.clone(), c.clone()).unwrap();
    }
    let armed = gapped.reconcile().unwrap();
    assert_eq!(
        armed.commands,
        import.len(),
        "C2 (hash and class-1 state) passed"
    );

    // The control takes the SAME import, same metadata, on an armed vault of the SAME vault id (the
    // ids an import mints derive from the vault id): a replica born from the same genesis, keyed
    // independently, whose base is recorded (Ready) so its commands are recorded in its outbox.
    let (verified, engine) = fetched_genesis(&gapped);
    let control = db_worker::sync_bootstrap::bootstrap_replica(
        &gapped.dir.path().join("control.vault"),
        &std::sync::Arc::new(vault_crypto::generate_dek().unwrap()),
        &db_worker::sync_bootstrap::ReplicaSource {
            vault_id: gapped.vault_id(),
            root: 0,
            snapshot: &verified,
            engine_version: &engine,
            own_engine_version: &EngineVersion::new(db_worker::ENGINE_VERSION).unwrap(),
            held_voids: &[],
        },
    )
    .unwrap();
    for (m, c) in &import {
        control.worker.dispatch(m.clone(), c.clone()).unwrap();
    }

    // The outbox rows match: command id, Pending, the issue time, the write-set keys.
    let rows = outbox_rows(&gapped);
    assert_eq!(rows, outbox_rows_of(&control.worker));
    assert_eq!(rows.len(), import.len());
    assert!(rows.iter().all(|r| r.1 == "Pending"));
    for ((m, _), row) in import.iter().zip(&rows) {
        assert_eq!(row.0.as_slice(), m.command_id.as_bytes().as_slice());
        assert_eq!(row.2, m.issued_at.as_unix_micros());
    }
    // Each row names its own log row (the op_seq mapping).
    let conn = gapped.worker.read_connection().unwrap();
    for ((m, _), _) in import.iter().zip(&rows) {
        let (outbox_seq, log_seq): (i64, i64) = conn
            .query_row(
                "SELECT o.op_seq, l.op_seq FROM sync_outbox o
                   JOIN operation_log l ON l.command_id = o.command_id
                  WHERE o.command_id = ?1",
                [m.command_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(outbox_seq, log_seq);
    }
    // Arming fabricated no artifact receipt or reference: the device-local artifact mapping is
    // untouched, and publishing the import's artifacts is the push path's (receipt before
    // reference), not arming's.
    assert_eq!(gapped.count("sync_artifacts"), 0);
    assert_eq!(
        control
            .worker
            .read_connection()
            .unwrap()
            .query_row("SELECT count(*) FROM sync_artifacts", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

// ---- an edit before a resume, and a rekey after the snapshot -------------------------------------

/// The flow a founding device lives through when it is interrupted: enable reports local changes, a
/// command lands in the gap, the reconciliation is cut short, an edit lands, and the resume arms
/// it. The base is the genesis, the outbox holds every command after the mark in the order it was
/// logged, and the device is armed only once the resume has succeeded (personal-cfo-1cltt.5).
#[test]
fn an_edit_before_a_resume_arms_on_the_genesis_with_every_post_mark_command_in_order() {
    use snapshot_service::Fault;
    use sync_core::snapshot_service::ServiceError;

    let (device, gap) = device_with_a_gap();
    let account = match gap.last() {
        Some(WriteCommand::ArchiveAccount(id)) => *id,
        other => panic!("the gap ends in an archive: {other:?}"),
    };
    let mark: i64 = device
        .worker
        .read_connection()
        .unwrap()
        .query_row("SELECT MAX(op_seq) FROM operation_log", [], |r| r.get(0))
        .unwrap();

    // A command lands in the gap after enable reported the local changes.
    device
        .worker
        .dispatch(meta(), create_account_cmd())
        .unwrap();

    // The reconciliation is cut short: the service does not answer. Nothing was written.
    device.service.script(Fault::Pass);
    device
        .service
        .script(Fault::Refuse(ServiceError::Unavailable));
    assert!(device.reconcile().is_err());
    assert_untouched(&device);
    assert_eq!(
        db_worker::sync_genesis_gap::armed_position(&device.worker.read_connection().unwrap())
            .unwrap(),
        None,
        "not armed after an interrupted run"
    );

    // An edit lands before the resume (it changes data the gap created).
    device
        .worker
        .dispatch(meta(), WriteCommand::ReinstateAccount(account))
        .unwrap();

    // The resume arms the device.
    let armed = device.reconcile().unwrap();
    assert_eq!(armed.base_seq, 0);
    assert_eq!(
        armed.commands,
        gap.len() + 2,
        "every command after the mark"
    );

    // The outbox is every command logged after the genesis mark, in log order.
    let conn = device.worker.read_connection().unwrap();
    let logged: Vec<Vec<u8>> = {
        let mut stmt = conn
            .prepare("SELECT command_id FROM operation_log WHERE op_seq > ?1 ORDER BY op_seq")
            .unwrap();
        stmt.query_map([mark - i64::try_from(gap.len()).unwrap()], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    let queued: Vec<Vec<u8>> = outbox_rows(&device).into_iter().map(|r| r.0).collect();
    assert_eq!(logged.len(), gap.len() + 2);
    assert_eq!(queued, logged);
    assert!(outbox_rows(&device).iter().all(|r| r.1 == "Pending"));

    // The base is the genesis, not the live vault, and the device is armed on it.
    let (verified, _) = fetched_genesis(&device);
    let base = db_worker::base_image::base_image_path(device.worker.db_path());
    let inspected = device
        .worker
        .scratch_from_image(&base, 0, &device.dir.path().join("inspect.db"))
        .unwrap();
    assert_eq!(
        class_one(&inspected.worker().read_connection().unwrap()),
        verified.manifest.tables
    );
    drop(inspected);
    assert_eq!(
        db_worker::sync_genesis_gap::armed_position(&conn).unwrap(),
        Some((None, 1))
    );
    assert!(!device.leftovers());
}

/// A key rotation on the origin after it exported its genesis changes nothing a replica or the
/// engine depends on: the class-1 rows, the vault's identity, and the recorded genesis identity
/// are what they were, and a replica born from the snapshot is the origin (personal-cfo-1cltt.5,
/// AC6). The rotation is the database side of `finance-kernel`'s: a copy re-encrypted under a new
/// key, verified, and swapped in.
#[test]
fn a_rekey_on_the_origin_after_the_snapshot_changes_nothing_a_replica_depends_on() {
    use std::sync::Arc;

    let device = Device::new();
    device
        .worker
        .dispatch(meta(), create_account_cmd())
        .unwrap();
    device.worker.dispatch(meta(), income_cmd(None)).unwrap();
    assert_eq!(device.enable(), EnableOutcome::Enabled);
    let (verified, engine) = fetched_genesis(&device);

    let identity = |worker: &DbWorker| {
        let genesis = worker
            .write_sync_state(|tx| sync_genesis::load(tx))
            .unwrap()
            .unwrap();
        (
            genesis.genesis_snapshot_id.unwrap(),
            genesis.genesis_container_sha256.unwrap(),
        )
    };
    let created_at = |worker: &DbWorker| -> String {
        worker
            .read_connection()
            .unwrap()
            .query_row("SELECT created_at FROM vault_metadata", [], |r| r.get(0))
            .unwrap()
    };
    let class_one_before = class_one(&device.worker.read_connection().unwrap());
    assert_eq!(
        class_one_before, verified.manifest.tables,
        "no gap: the origin is its genesis"
    );
    let identity_before = identity(&device.worker);
    let vault_id = device.vault_id();
    let created_before = created_at(&device.worker);

    // Rotate: a re-encrypted copy under a new key, verified, swapped in, reopened under the new key.
    let Device { dir, worker, .. } = device;
    let path = worker.db_path().to_path_buf();
    let with_suffix = |suffix: &str| {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        PathBuf::from(name)
    };
    let new_dek = vault_crypto::generate_dek().unwrap();
    let copy = with_suffix(".rekey-new");
    let renames = worker.prepare_rekeyed_copy(&new_dek, &copy).unwrap();
    assert!(renames.is_empty(), "this vault holds no attachment");
    db_worker::verify_rekeyed_copy(
        &copy,
        &new_dek,
        &dir.path().join("blobs"),
        worker.user_version().unwrap(),
    )
    .unwrap();
    drop(worker);
    for suffix in ["-wal", "-shm"] {
        let _ = std::fs::remove_file(with_suffix(suffix));
    }
    std::fs::rename(&copy, &path).unwrap();
    let rotated = DbWorker::open_existing_with_raw_key(&path, new_dek).unwrap();

    // The origin after the rotation is the origin before it.
    assert_eq!(
        class_one(&rotated.read_connection().unwrap()),
        class_one_before,
        "the class-1 export is unchanged"
    );
    assert_eq!(
        identity(&rotated),
        identity_before,
        "the recorded genesis identity is unchanged"
    );
    assert_eq!(*rotated.vault_id().unwrap().as_bytes(), vault_id);
    assert_eq!(created_at(&rotated), created_before);

    // A replica born from the snapshot the origin published before the rotation is the origin.
    let replica = db_worker::sync_bootstrap::bootstrap_replica(
        &dir.path().join("replica.vault"),
        &Arc::new(vault_crypto::generate_dek().unwrap()),
        &db_worker::sync_bootstrap::ReplicaSource {
            vault_id,
            root: 0,
            snapshot: &verified,
            engine_version: &engine,
            own_engine_version: &EngineVersion::new(db_worker::ENGINE_VERSION).unwrap(),
            held_voids: &[],
        },
    )
    .unwrap();
    assert_eq!(
        class_one(&replica.worker.read_connection().unwrap()),
        class_one(&rotated.read_connection().unwrap()),
        "the replica is hash-equal to the rotated origin over class 1"
    );
    assert_eq!(*replica.worker.vault_id().unwrap().as_bytes(), vault_id);
    assert_eq!(created_at(&replica.worker), created_before);
}
