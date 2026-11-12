//! Enabling Sync on the founding device (personal-cfo-1cltt.4 part C; `vault-format.md` C.4, C.6
//! rule 5 and C.8 "Publishing a snapshot").
//!
//! [`enable_sync`] is one library function, not a UI command. Its order is:
//!
//! 1. judge the vault's verified state against the witness (a restored or stripped vault stops here,
//!    before any service call);
//! 2. **mint and persist**: record the birth mark in the witness, build the genesis
//!    (`prepare_genesis`), persist its records in the vault's class-3 state, then record the mark of
//!    what was persisted. A vault that holds a persisted genesis never mints another;
//! 3. **install** the keys from the persisted records (`install_genesis`, idempotent), then move the
//!    genesis state to `Installed`;
//! 4. **publish**: send the saved container of an earlier attempt if there is one, otherwise export
//!    the live vault and publish it as the genesis (`installEpoch`, reason 1);
//! 5. record how it ended: `Published`, or `VaultKnown` when the service already knows the vault
//!    (R39). A vault that is `VaultKnown` is never published, retried or re-minted, nothing in it is
//!    discarded, and the device bootstraps as a replica (`personal-cfo-1cltt.5`).
//!
//! A crash or a lost response at any step is finished by calling [`enable_sync`] again: each step
//! reads what the previous one left (the genesis state, the saved container) and repeats only what
//! has not happened. The state is read before anything is minted or sent, so a call on a vault that
//! is `Published` or `VaultKnown` makes no service call.

use chrono::Utc;
use platform_traits::{RollbackWitness, SecretStore};
use sync_core::membership::verify_chain;
use sync_core::snapshot_service::{SnapshotService, SnapshotSource};
use sync_core::status::SyncStatus;
use thiserror::Error;
use vault_crypto::sync_counter::{CounterRegister, VerifiedStateError, VerifiedStateGuard};
use vault_crypto::sync_custody::{CustodyError, EnableError as CustodyEnableError, SyncKeyCustody};
use vault_crypto::sync_envelope::SealerIdentity;
use vault_crypto::sync_snapshot::{SavedSnapshotFiles, SnapshotFields, SnapshotSealer};
use vault_crypto::sync_snapshot_flow::{
    genesis_snapshot_id, publish_keeping_saved, resume_saved_keeping, PublishError, PublishKind,
    PublishOutcome, PublishRequest,
};

use crate::base_image::saved_snapshots_dir;
use crate::logical_export::{export_genesis_with_mark, ExportError, ExportInput};
use crate::sync_genesis::{self, GenesisState, GenesisStoreError};
use crate::sync_register::SqliteRegisterStore;
use crate::{DbError, DbWorker, ENGINE_VERSION};

/// This device, as the genesis names it.
#[derive(Debug, Clone, Copy)]
pub struct EnablingDevice {
    /// The device id the genesis `add` record carries.
    pub device_id: [u8; 16],
    /// The time the genesis records are issued at (Unix seconds).
    pub issued_at: i64,
}

/// How enabling ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EnableOutcome {
    /// The genesis is published. Sync is enabled on this device.
    Enabled,
    /// The genesis is published, but the vault changed after the export: its `operation_log` holds
    /// commands past the mark the genesis was exported at, and no envelope carries them. The device
    /// is **not armed**: whoever arms it must not treat the vault as ready, because its state is
    /// not what the genesis carries. The reconciliation (a base built from the genesis plaintext
    /// and the later commands enqueued as unpushed work) is the replica bootstrap's
    /// (`personal-cfo-1cltt.5`).
    EnabledWithLocalChanges {
        /// The `operation_log` high-water mark of the export.
        since_mark: u64,
        /// The high-water mark now.
        high_water: u64,
    },
    /// The service already knows this vault and does not hold this genesis (R39). Nothing was
    /// published, nothing in the vault changed, and the genesis is never retried: the device
    /// bootstraps from the service's snapshot as a replica.
    VaultKnown,
    /// An earlier attempt of this genesis reached the service and the container it sent is gone, so
    /// a fresh export would not be what the service holds. Nothing was published, minted or
    /// discarded, and a restart does not retry. Shown in Sync status with the support path: the
    /// vault holds changes the service's snapshot lacks, so the device does not bootstrap on its
    /// own.
    GenesisResumeLost,
}

/// Why enabling did not finish. Calling [`enable_sync`] again resumes it, except where noted.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum EnableError {
    /// The vault's verified Sync state does not match the witness: a restore, or state that was
    /// stripped. Nothing is minted or sent; the device re-enrolls.
    #[error("{0}")]
    Verified(VerifiedStateError),
    /// The persisted genesis could not be read or changed.
    #[error("{0}")]
    Genesis(GenesisStoreError),
    /// The device's keys or the genesis could not be made or installed.
    #[error("{0}")]
    Enable(CustodyEnableError),
    /// A key could not be loaded from the secret store.
    #[error("{0}")]
    Custody(CustodyError),
    /// The vault could not be exported as a genesis.
    #[error("{0}")]
    Export(ExportError),
    /// The publish was refused or failed (including a halted or stopped replica).
    #[error("{0}")]
    Publish(PublishError),
    /// The database failed.
    #[error("{0}")]
    Db(DbError),
    /// The vault already holds Sync state of an earlier life (a `sync_base` row or an outbox row)
    /// and no persisted genesis: it has synced before, or it is a restored copy of a vault that
    /// did. A genesis exported from it would carry the effects of unsequenced local commands that an
    /// envelope will also carry, so it is never minted or exported; the device joins the vault as a
    /// device instead (`personal-cfo-1cltt.5`). Nothing was minted or sent.
    #[error("this vault has synced before: join it as a device instead of enabling Sync")]
    LiveVaultHasSyncState,
    /// The directory for the saved snapshot could not be created.
    #[error("the saved snapshot directory could not be created")]
    Io,
}

macro_rules! from_error {
    ($($ty:ty => $variant:ident),* $(,)?) => {$(
        impl From<$ty> for EnableError {
            fn from(error: $ty) -> Self {
                Self::$variant(error)
            }
        }
    )*};
}

from_error! {
    VerifiedStateError => Verified,
    GenesisStoreError => Genesis,
    CustodyEnableError => Enable,
    CustodyError => Custody,
    ExportError => Export,
    PublishError => Publish,
    DbError => Db,
}

/// The largest `op_seq` of the operation log (`0` for an empty log): the high-water mark.
fn op_log_high_water(worker: &DbWorker) -> Result<u64, EnableError> {
    let mark: i64 = worker.write_sync_state(|tx| -> Result<i64, DbError> {
        Ok(tx.query_row(
            "SELECT COALESCE(MAX(op_seq), 0) FROM operation_log",
            [],
            |r| r.get(0),
        )?)
    })?;
    Ok(u64::try_from(mark).unwrap_or(0))
}

/// Whether the device has been armed for Sync: a `sync_base` row exists. Until then the commands
/// the vault takes are not recorded in the outbox, so a log that has grown past the genesis's mark
/// holds commands no envelope will ever carry.
fn is_armed(worker: &DbWorker) -> Result<bool, EnableError> {
    Ok(worker.write_sync_state(|tx| -> Result<bool, DbError> {
        let rows: i64 = tx.query_row("SELECT count(*) FROM sync_base", [], |r| r.get(0))?;
        Ok(rows > 0)
    })?)
}

/// `Enabled` for a published genesis, or `EnabledWithLocalChanges` when the log has grown past the
/// mark of the export and the device is not armed yet.
fn published_outcome(
    worker: &DbWorker,
    export_high_water: Option<u64>,
) -> Result<EnableOutcome, EnableError> {
    let since_mark = export_high_water.ok_or(GenesisStoreError::Corrupt)?;
    if is_armed(worker)? {
        return Ok(EnableOutcome::Enabled);
    }
    let high_water = op_log_high_water(worker)?;
    Ok(if high_water > since_mark {
        EnableOutcome::EnabledWithLocalChanges {
            since_mark,
            high_water,
        }
    } else {
        EnableOutcome::Enabled
    })
}

/// Whether the live vault holds Sync state of an earlier life: a `sync_base` row, or any outbox row.
fn holds_sync_state(worker: &DbWorker) -> Result<bool, EnableError> {
    Ok(worker.write_sync_state(|tx| -> Result<bool, DbError> {
        let found: i64 = tx.query_row(
            "SELECT (SELECT count(*) FROM sync_base) + (SELECT count(*) FROM sync_outbox)",
            [],
            |r| r.get(0),
        )?;
        Ok(found > 0)
    })?)
}

/// Enable Sync on this device for the vault `worker` holds, or finish an enabling that was
/// interrupted. See the module comment for the order and for what a repeated call does.
///
/// # Errors
/// An [`EnableError`]; the state each step reached is persisted, so calling again resumes.
pub fn enable_sync<W: RollbackWitness, Service: SnapshotService>(
    worker: &DbWorker,
    secrets: &dyn SecretStore,
    witness: &W,
    service: &Service,
    status: &SyncStatus,
    device: &EnablingDevice,
) -> Result<EnableOutcome, EnableError> {
    let vault_id = *worker.vault_id()?.as_bytes();
    let guard = VerifiedStateGuard::new(witness);

    // 1. What the vault holds, judged against the witness before anything else happens. A state
    //    the witness has not seen yet (the crash window after a persist) is recorded here.
    let held = worker.write_sync_state(|tx| sync_genesis::load(tx))?;
    let mark = worker.write_sync_state(|tx| sync_genesis::loaded_verified_mark(tx))?;
    guard.reconcile(&vault_id, mark.as_ref())?;
    match held.as_ref().map(|genesis| genesis.state) {
        Some(GenesisState::VaultKnown) => return Ok(EnableOutcome::VaultKnown),
        Some(GenesisState::Published) => {
            return published_outcome(
                worker,
                held.as_ref().and_then(|genesis| genesis.export_high_water),
            )
        }
        Some(GenesisState::GenesisResumeLost) => return Ok(EnableOutcome::GenesisResumeLost),
        _ => {}
    }
    // A vault with Sync state of an earlier life and no genesis of its own never mints one.
    if held.is_none() && holds_sync_state(worker)? {
        return Err(EnableError::LiveVaultHasSyncState);
    }
    // A halted or stopped replica publishes nothing (Addendum D rule 5); it does not start
    // minting a genesis either.
    match status {
        SyncStatus::Healthy => {}
        SyncStatus::Halted(_) => return Err(PublishError::Halted.into()),
        _ => return Err(PublishError::Stopped.into()),
    }

    // 2. Mint and persist, once. The birth mark goes into the witness BEFORE the first persist and
    //    the mark of what was persisted AFTER it, so a crash between the steps is never mistaken
    //    for a restore.
    let custody = SyncKeyCustody::new(secrets, vault_id);
    let genesis = if let Some(held) = held {
        held
    } else {
        guard.begin(&vault_id)?;
        let records = custody.prepare_genesis(device.device_id, device.issued_at)?;
        let now = Utc::now().to_rfc3339();
        worker.write_sync_state(|tx| {
            sync_genesis::persist(tx, &vault_id, &records.membership, &records.epoch, &now)
        })?;
        let mark = worker.write_sync_state(|tx| sync_genesis::loaded_verified_mark(tx))?;
        if let Some(mark) = &mark {
            guard.record(&vault_id, mark)?;
        }
        worker
            .write_sync_state(|tx| sync_genesis::load(tx))?
            .ok_or(GenesisStoreError::NotPersisted)?
    };

    // 3. Install the keys from the persisted records, never from the values that sealed them.
    if genesis.state == GenesisState::Persisted {
        custody.install_genesis(&vault_crypto::sync_records::GenesisRecords {
            membership: genesis.membership_record.clone(),
            epoch: genesis.epoch_record.clone(),
        })?;
        let now = Utc::now().to_rfc3339();
        worker.write_sync_state(|tx| sync_genesis::advance(tx, GenesisState::Installed, &now))?;
    }

    // 4. Publish.
    let chain = verify_chain(
        &vault_id,
        &[genesis.membership_record.as_slice()],
        &[genesis.epoch_record.as_slice()],
    )
    .map_err(|_| GenesisStoreError::Corrupt)?;
    let key = custody.load_epoch_key(&chain, 1)?;
    let keys = custody.load_device_keys()?;
    let identity = SealerIdentity {
        vault_id,
        device_id: device.device_id,
        fingerprint: keys.public().fingerprint,
    };
    let snapshots = saved_snapshots_dir(worker.db_path());
    std::fs::create_dir_all(&snapshots).map_err(|_| EnableError::Io)?;
    let files = SavedSnapshotFiles::new(&snapshots);
    let store = SqliteRegisterStore::new(worker);
    let register = CounterRegister::new(&store, witness);
    let sealer = SnapshotSealer {
        register: &register,
        files: &files,
        key: &key,
        sealer: &identity,
        chain: &chain,
    };
    let snapshot_id =
        genesis_snapshot_id(&chain.epoch_head_id().ok_or(GenesisStoreError::Corrupt)?);
    let kind = PublishKind::Genesis {
        membership_record: &genesis.membership_record,
        epoch_record: &genesis.epoch_record,
    };
    let outcome = match resume_saved_keeping(service, &sealer, status, &kind, &snapshot_id)? {
        Some(outcome) => outcome,
        None => {
            // Nothing saved: the genesis is read from the live vault, now, before any outbox row
            // exists. A vault that has gained Sync state since is not exported (and a genesis that
            // is persisted is not sent from it).
            if holds_sync_state(worker)? {
                return Err(EnableError::LiveVaultHasSyncState);
            }
            let (plaintext, high_water) =
                export_genesis_with_mark(&worker.read_connection()?, &ExportInput::default())?;
            // The mark is persisted before anything is sealed; a later export replaces it.
            let now = Utc::now().to_rfc3339();
            worker.write_sync_state(|tx| sync_genesis::record_export_mark(tx, high_water, &now))?;
            publish_keeping_saved(
                service,
                &sealer,
                status,
                &PublishRequest {
                    kind,
                    source: SnapshotSource::LiveVault,
                    fields: SnapshotFields {
                        object_id: snapshot_id,
                        base_seq: 0,
                        payload_schema_version: crate::payload::PAYLOAD_SCHEMA_VERSION,
                        engine_version: ENGINE_VERSION.to_owned(),
                        schema_version: u32::try_from(worker.schema_version())
                            .map_err(|_| GenesisStoreError::Corrupt)?,
                    },
                    plaintext: &plaintext,
                },
            )?
        }
    };

    // 5. Record how it ended, and only then drop the saved container: a crash between the two
    //    leaves the outcome recorded and a leftover file, never a dropped container with the
    //    outcome lost (which would reseal and reach a service that already holds the genesis).
    let now = Utc::now().to_rfc3339();
    let (terminal, result) = match outcome {
        PublishOutcome::GenesisInstalled(_) => (GenesisState::Published, EnableOutcome::Enabled),
        PublishOutcome::VaultKnown => (GenesisState::VaultKnown, EnableOutcome::VaultKnown),
        PublishOutcome::GenesisResumeLost => (
            GenesisState::GenesisResumeLost,
            EnableOutcome::GenesisResumeLost,
        ),
        // A genesis is never published as a re-snapshot.
        _ => return Err(PublishError::GenesisSource.into()),
    };
    worker.write_sync_state(|tx| sync_genesis::advance(tx, terminal, &now))?;
    sealer
        .discard(&snapshot_id)
        .map_err(PublishError::Snapshot)?;
    if terminal == GenesisState::Published {
        let mark = worker
            .write_sync_state(|tx| sync_genesis::load(tx))?
            .and_then(|genesis| genesis.export_high_water);
        return published_outcome(worker, mark);
    }
    Ok(result)
}
