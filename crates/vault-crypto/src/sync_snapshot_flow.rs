//! Publishing a snapshot (`docs/architecture/sync-service-protocol.md` 4.4 and 4.9;
//! personal-cfo-1cltt.4, part B).
//!
//! * A **genesis** (the first snapshot of a vault, sealed under epoch 1) is published by
//!   `installEpoch` with `reason 1`, `base_seq` `0`, the persisted genesis records and an empty
//!   closure (until `personal-cfo-1cltt.7`). It is read from the live vault, at sync-enable, before
//!   any outbox row exists.
//! * A **re-snapshot** (the only way history is truncated) is published by `publishSnapshot` with
//!   `base_seq` equal to the head and to the snapshot's own. It is never read from the live vault:
//!   the live vault holds the effects of local commands no envelope has sequenced, so the source
//!   must be a state equal to the log at its head.
//!
//! The order of one publish is: refuse locally (a stopped or halted replica, the wrong source),
//! seal (the counter is reserved, the container saved), stage, publish, and only when the service
//! has answered drop the saved container. A crash or a lost response anywhere repeats the same
//! calls with the same bytes: every operation is idempotent by its immutable id, and the sealer
//! hands back the saved container of the same publish instead of sealing again.
//!
//! **Nothing here persists the outcome.** The caller records it in the vault's class-3 Sync state
//! (the genesis state, the base).

use sha2::{Digest, Sha256};
use sync_core::artifact::ArtifactId;
use sync_core::header::parse_container;
use sync_core::snapshot_service::{
    EpochInstall, InstalledEpoch, PublishedSnapshot, ServiceError, SnapshotService, SnapshotSource,
};
use sync_core::status::SyncStatus;
use thiserror::Error;

use platform_traits::RollbackWitness;

use crate::sync_counter::RegisterStore;
use crate::sync_snapshot::{
    closure_ids_of, SnapshotError, SnapshotFields, SnapshotOpener, SnapshotSealError,
    SnapshotSealer,
};

/// What is being published.
#[derive(Clone, Copy)]
pub enum PublishKind<'a> {
    /// The first snapshot of the vault: the persisted genesis records, as they were persisted.
    Genesis {
        /// The signed membership record (version 1, `add`).
        membership_record: &'a [u8],
        /// The signed reason-1 epoch record.
        epoch_record: &'a [u8],
    },
    /// A later snapshot at the head the plaintext was built on.
    ReSnapshot,
}

/// One publish.
pub struct PublishRequest<'a> {
    /// Genesis or re-snapshot.
    pub kind: PublishKind<'a>,
    /// Where `plaintext` was read from. A genesis is `LiveVault`; anything else is not.
    pub source: SnapshotSource,
    /// The snapshot's header fields (`base_seq` is `0` for a genesis).
    pub fields: SnapshotFields,
    /// The snapshot plaintext (C.8).
    pub plaintext: &'a [u8],
}

impl core::fmt::Debug for PublishRequest<'_> {
    /// Up to 1 GiB of vault data and the genesis records: sizes and kinds only.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PublishRequest")
            .field(
                "kind",
                &match self.kind {
                    PublishKind::Genesis { .. } => "Genesis",
                    PublishKind::ReSnapshot => "ReSnapshot",
                },
            )
            .field("source", &self.source)
            .field("base_seq", &self.fields.base_seq)
            .field("plaintext_len", &self.plaintext.len())
            .finish()
    }
}

/// How a publish ended, when it ended in a state the caller records.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PublishOutcome {
    /// A re-snapshot is published.
    Published(PublishedSnapshot),
    /// A genesis is installed with its snapshot.
    GenesisInstalled(InstalledEpoch),
    /// The service already knows this vault and does not hold this genesis (R39). Nothing was
    /// published, the saved container is dropped, and nothing in the vault changed: the caller
    /// records that the genesis was refused and bootstraps as a replica.
    VaultKnown,
}

/// Why a snapshot was not published. Where the error comes from the service, the request changed
/// nothing there, except that an `Unavailable` answer may have taken effect (ask again).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum PublishError {
    /// The replica is halted at a poisoned position (Addendum D rule 5): no snapshot is published.
    /// Refused locally, before any counter is spent and any request is made.
    #[error("sync is halted: no snapshot is published")]
    Halted,
    /// The replica is stopped at an invalid envelope or an engine version it does not run
    /// (Addendum D rule 5): no snapshot is published. Refused locally.
    #[error("sync is stopped: no snapshot is published")]
    Stopped,
    /// A snapshot other than a genesis was read from the live vault, which holds unsequenced local
    /// effects. Refused locally.
    #[error("a snapshot after the genesis is not exported from the live vault")]
    LiveVaultSource,
    /// A re-snapshot was read from the state at one position but declares another as its
    /// `base_seq`. Refused locally.
    #[error(
        "the snapshot is built from the state at position {source_head} but declares {base_seq}"
    )]
    SourceHeadMismatch {
        /// The position the plaintext was read at.
        source_head: u64,
        /// The `base_seq` the snapshot declares.
        base_seq: u64,
    },
    /// A genesis was not read from the live vault, or does not start at position `0`.
    #[error("a genesis is exported from the live vault at position 0")]
    GenesisSource,
    /// The snapshot could not be sealed (including `NonceStateLost`).
    #[error("{0}")]
    Snapshot(SnapshotError),
    /// The service reports this device as halted (protocol 4.12).
    #[error("the service reports this device as halted at position {stopped_at}")]
    ServiceHalted {
        /// The position the device reported.
        stopped_at: u64,
    },
    /// The service refused or failed the request.
    #[error("{0}")]
    Service(ServiceError),
}

impl From<ServiceError> for PublishError {
    fn from(error: ServiceError) -> Self {
        match error {
            ServiceError::ReplicaHalted { stopped_at } => Self::ServiceHalted { stopped_at },
            other => Self::Service(other),
        }
    }
}

impl From<SnapshotError> for PublishError {
    fn from(error: SnapshotError) -> Self {
        Self::Snapshot(error)
    }
}

/// The refusals that come before anything is sealed or sent (Addendum D rule 5).
fn refuse_locally(status: &SyncStatus) -> Result<(), PublishError> {
    match status {
        SyncStatus::Healthy => Ok(()),
        SyncStatus::Halted(_) => Err(PublishError::Halted),
        _ => Err(PublishError::Stopped),
    }
}

/// Publish a snapshot. See the module comment for the order and for what is refused before
/// anything is sealed or sent.
///
/// # Errors
/// A [`PublishError`]; on any error nothing may be recorded as published.
pub fn publish<Store, Witness, Service>(
    service: &Service,
    sealer: &SnapshotSealer<'_, Store, Witness>,
    status: &SyncStatus,
    request: &PublishRequest<'_>,
) -> Result<PublishOutcome, PublishError>
where
    Store: RegisterStore,
    Witness: RollbackWitness,
    Service: SnapshotService,
{
    // Addendum D rule 5, before anything else: no counter, no file, no request.
    refuse_locally(status)?;
    match (&request.kind, request.source) {
        (PublishKind::ReSnapshot, SnapshotSource::LiveVault) => {
            return Err(PublishError::LiveVaultSource)
        }
        // The state the plaintext was read from must be the position the snapshot claims: a
        // snapshot that says it is built on position 5 but holds the state at 3 would make every
        // replica that bootstraps from it lose the envelopes in between.
        (PublishKind::ReSnapshot, SnapshotSource::StateAtHead { head })
            if head != request.fields.base_seq =>
        {
            return Err(PublishError::SourceHeadMismatch {
                source_head: head,
                base_seq: request.fields.base_seq,
            })
        }
        (PublishKind::Genesis { .. }, source)
            if source != SnapshotSource::LiveVault || request.fields.base_seq != 0 =>
        {
            return Err(PublishError::GenesisSource)
        }
        _ => {}
    }
    let manifest = closure_ids_of(request.plaintext)
        .map_err(|e| PublishError::Snapshot(SnapshotError::Seal(e)))?;
    let sealed = sealer.seal(&request.fields, request.plaintext)?;
    deliver(
        service,
        sealer,
        &request.kind,
        &request.fields.object_id,
        request.fields.base_seq,
        &sealed.container,
        &manifest,
    )
}

/// Send the snapshot `snapshot_id` again from its **saved container**, if one is saved and can still
/// be sent, without exporting or sealing anything: a resume after a crash or a lost response sends
/// exactly the bytes it sent before, whatever the vault holds now. `Ok(None)` means nothing usable
/// is saved, and the caller exports and calls [`publish`].
///
/// # Errors
/// A [`PublishError`], as [`publish`]. [`PublishError::GenesisSource`] if a genesis resume finds a
/// saved container that does not start at position `0`.
pub fn resume_saved<Store, Witness, Service>(
    service: &Service,
    sealer: &SnapshotSealer<'_, Store, Witness>,
    status: &SyncStatus,
    kind: &PublishKind<'_>,
    snapshot_id: &[u8; 16],
) -> Result<Option<PublishOutcome>, PublishError>
where
    Store: RegisterStore,
    Witness: RollbackWitness,
    Service: SnapshotService,
{
    refuse_locally(status)?;
    let Some(sealed) = sealer.saved(snapshot_id)? else {
        return Ok(None);
    };
    // The saved bytes are this device's own, so they open through the receiver path; the closure
    // sidecar the service wants is read from what they hold, never from anything remembered.
    let parsed = parse_container(&sealed.container)
        .map_err(|_| PublishError::Snapshot(SnapshotError::Seal(SnapshotSealError::Unencodable)))?;
    let plaintext = SnapshotOpener::new(sealer.key, sealer.chain)
        .open(&parsed)
        .map_err(|e| PublishError::Snapshot(SnapshotError::NotOpenable(e)))?;
    let manifest =
        closure_ids_of(&plaintext).map_err(|e| PublishError::Snapshot(SnapshotError::Seal(e)))?;
    let base_seq = parsed.header.base_seq;
    if matches!(kind, PublishKind::Genesis { .. }) && base_seq != 0 {
        return Err(PublishError::GenesisSource);
    }
    deliver(
        service,
        sealer,
        kind,
        snapshot_id,
        base_seq,
        &sealed.container,
        &manifest,
    )
    .map(Some)
}

/// Stage and send one sealed container, then drop the saved copy once the answer says it is
/// accepted (or a genesis is refused as `VaultKnown`). Any other answer keeps the saved container
/// for an exact-byte retry.
fn deliver<Store, Witness, Service>(
    service: &Service,
    sealer: &SnapshotSealer<'_, Store, Witness>,
    kind: &PublishKind<'_>,
    snapshot_id: &[u8; 16],
    base_seq: u64,
    container: &[u8],
    manifest: &[ArtifactId],
) -> Result<PublishOutcome, PublishError>
where
    Store: RegisterStore,
    Witness: RollbackWitness,
    Service: SnapshotService,
{
    let vault_id = sealer.key.vault_id();
    let outcome = match *kind {
        PublishKind::ReSnapshot => {
            service.stage_snapshot(vault_id, snapshot_id, container)?;
            PublishOutcome::Published(service.publish_snapshot(
                vault_id,
                snapshot_id,
                base_seq,
                manifest,
            )?)
        }
        PublishKind::Genesis {
            membership_record,
            epoch_record,
        } => {
            let install = EpochInstall {
                base_seq: 0,
                membership_record: Some(membership_record),
                epoch_record,
                snapshot_id: Some(*snapshot_id),
                artifact_manifest: manifest,
            };
            match install_genesis(service, vault_id, snapshot_id, container, &install) {
                Ok(installed) => PublishOutcome::GenesisInstalled(installed),
                Err(ServiceError::VaultExists) => PublishOutcome::VaultKnown,
                Err(other) => return Err(other.into()),
            }
        }
    };
    // The service has answered: the saved container could only send this snapshot again.
    sealer.discard(snapshot_id)?;
    Ok(outcome)
}

/// The id of a vault's genesis snapshot, derived from its genesis epoch record's id so that a
/// resume after a crash names the same snapshot the first attempt did, with no extra state to
/// persist: the first 16 bytes of `SHA-256("dohflow/genesis-snapshot-id/v1" ‖ epoch_record_id)`,
/// with the UUID version (4) and variant bits set.
#[must_use]
pub fn genesis_snapshot_id(epoch_record_id: &[u8; 32]) -> [u8; 16] {
    let digest = Sha256::new()
        .chain_update(b"dohflow/genesis-snapshot-id/v1")
        .chain_update(epoch_record_id)
        .finalize();
    let mut id = [0_u8; 16];
    id.copy_from_slice(&digest[..16]);
    id[6] = (id[6] & 0x0f) | 0x40;
    id[8] = (id[8] & 0x3f) | 0x80;
    id
}

/// Stage and install a genesis. A refused stage is **never** read as the vault being known or as a
/// failure: once a genesis is installed the service refuses the account credential at
/// `stageSnapshot`, which is exactly the state after a lost `installEpoch` response, and the vault
/// being known is `installEpoch`'s answer alone. So a refusal at the stage goes on to
/// `installEpoch` with the same bytes; if that says the snapshot was never staged, the stage is
/// made to succeed and the install is tried once more.
fn install_genesis<Service: SnapshotService>(
    service: &Service,
    vault_id: &[u8; 16],
    snapshot_id: &[u8; 16],
    container: &[u8],
    install: &EpochInstall<'_>,
) -> Result<InstalledEpoch, ServiceError> {
    match service.stage_snapshot(vault_id, snapshot_id, container) {
        Ok(_) | Err(ServiceError::Forbidden | ServiceError::VaultExists) => {}
        Err(other) => return Err(other),
    }
    match service.install_epoch(vault_id, install) {
        Err(ServiceError::SnapshotNotFound) => {
            service.stage_snapshot(vault_id, snapshot_id, container)?;
            service.install_epoch(vault_id, install)
        }
        other => other,
    }
}

/// Whether the service holds `snapshot_id` as its newest published snapshot: the idempotent
/// re-query after a crash or a lost response. `None` means it does not (yet).
///
/// # Errors
/// [`PublishError::Service`] / [`PublishError::ServiceHalted`].
pub fn find_published<Service: SnapshotService>(
    service: &Service,
    vault_id: &[u8; 16],
    snapshot_id: &[u8; 16],
) -> Result<Option<PublishedSnapshot>, PublishError> {
    Ok(service
        .latest_snapshot(vault_id)?
        .filter(|latest| &latest.snapshot_id == snapshot_id))
}
