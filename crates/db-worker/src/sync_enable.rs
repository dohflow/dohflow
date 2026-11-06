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
    genesis_snapshot_id, publish, resume_saved, PublishError, PublishKind, PublishOutcome,
    PublishRequest,
};

use crate::base_image::saved_snapshots_dir;
use crate::logical_export::{export_genesis_plaintext, ExportError, ExportInput};
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
    /// The service already knows this vault and does not hold this genesis (R39). Nothing was
    /// published, nothing in the vault changed, and the genesis is never retried: the device
    /// bootstraps from the service's snapshot as a replica.
    VaultKnown,
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
        Some(GenesisState::Published) => return Ok(EnableOutcome::Enabled),
        _ => {}
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
    let outcome = match resume_saved(service, &sealer, status, &kind, &snapshot_id)? {
        Some(outcome) => outcome,
        None => {
            // Nothing saved: the genesis is read from the live vault, now, before any outbox row
            // exists.
            let plaintext =
                export_genesis_plaintext(&worker.read_connection()?, &ExportInput::default())?;
            publish(
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

    // 5. Record how it ended.
    let now = Utc::now().to_rfc3339();
    match outcome {
        PublishOutcome::GenesisInstalled(_) => {
            worker
                .write_sync_state(|tx| sync_genesis::advance(tx, GenesisState::Published, &now))?;
            Ok(EnableOutcome::Enabled)
        }
        PublishOutcome::VaultKnown => {
            worker
                .write_sync_state(|tx| sync_genesis::advance(tx, GenesisState::VaultKnown, &now))?;
            Ok(EnableOutcome::VaultKnown)
        }
        // A genesis is never published as a re-snapshot.
        _ => Err(PublishError::GenesisSource.into()),
    }
}
