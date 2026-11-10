//! The seam between the client and the Sync service's snapshot and epoch endpoints
//! (`docs/architecture/sync-service-protocol.md` 4.4 `installEpoch`, 4.9 `stageSnapshot`,
//! `publishSnapshot` and `getLatestSnapshot`; personal-cfo-1cltt.4, part B).
//!
//! [`SnapshotService`] is those four operations as the client calls them. **There is no production
//! implementor in this tree**: the service is `personal-cfo-v98vd` and the client engine that owns
//! the network is `personal-cfo-elciw`. The only implementor is the in-memory fake in the
//! vault-crypto tests, which runs the server's decision procedure (so a test exercises the real
//! refusals), and a source-scan test fails any other until the engine supplies one, so no code can
//! start talking to a service by accident.
//!
//! The trait does not carry the credential: a genesis is sent with the account credential (there is
//! no device to check before the genesis exists), every later call with the device's. Which one a
//! call uses is the transport's concern, and the service's refusal of the wrong one is
//! [`ServiceError::Forbidden`].

use thiserror::Error;

use crate::artifact::ArtifactId;

/// What `stageSnapshot` answers: the snapshot is held, staged, not yet published.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StagedSnapshot {
    /// The snapshot id (the header's `object_id`).
    pub snapshot_id: [u8; 16],
    /// The stored container's length.
    pub size: u64,
}

/// A published snapshot: what `publishSnapshot` and `getLatestSnapshot` answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublishedSnapshot {
    /// The snapshot id.
    pub snapshot_id: [u8; 16],
    /// The log position it is built on.
    pub base_seq: u64,
    /// The key epoch it is sealed under.
    pub key_epoch: u64,
}

/// What `installEpoch` answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstalledEpoch {
    /// The epoch now in force.
    pub key_epoch: u64,
    /// The record's install order (1 for the genesis record).
    pub ordinal: u64,
    /// The log head at which it was installed.
    pub position: u64,
    /// The id of the newest membership record.
    pub membership_head: [u8; 32],
    /// The id of the epoch record just installed.
    pub epoch_record_id: [u8; 32],
    /// `true` when the service answered an **exact retry** of an epoch record it had already
    /// installed (HTTP `200` rather than `201`, protocol 4.4 and 6.1 check 3): the genesis was
    /// installed by an earlier request, with the snapshot that request staged.
    pub already_installed: bool,
}

/// One `installEpoch` request (protocol 4.4). The epoch record carries its own `reason`; the
/// genesis (`reason 1`) has `base_seq` `0`, a membership record, a snapshot and, until
/// `personal-cfo-1cltt.7`, an empty closure.
#[derive(Debug, Clone, Copy)]
pub struct EpochInstall<'a> {
    /// The log head the client believes is current (`0` for a genesis).
    pub base_seq: u64,
    /// The signed membership record, when the epoch record names one.
    pub membership_record: Option<&'a [u8]>,
    /// The signed epoch record.
    pub epoch_record: &'a [u8],
    /// The staged snapshot published with it.
    pub snapshot_id: Option<[u8; 16]>,
    /// The snapshot's full closure sidecar, ascending.
    pub artifact_manifest: &'a [ArtifactId],
}

/// Why the service did not do what was asked. A refusal changes nothing on the service (protocol
/// 2.5), and [`Self::Unavailable`] says nothing about whether the request took effect: the client
/// asks again, and every operation here is idempotent by its immutable id.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ServiceError {
    /// The vault is unknown (`VaultNotFound`).
    #[error("the service does not know this vault")]
    VaultNotFound,
    /// A genesis for a vault the service already knows (`VaultExists`): the client bootstraps as a
    /// replica instead (R39).
    #[error("the service already knows this vault")]
    VaultExists,
    /// This device has reported a halt (`ReplicaHalted`, protocol 4.12).
    #[error("this device is halted at position {stopped_at}")]
    ReplicaHalted {
        /// The position the device reported.
        stopped_at: u64,
    },
    /// The snapshot was never staged (`SnapshotNotFound`).
    #[error("the snapshot is not staged")]
    SnapshotNotFound,
    /// `base_seq` is not the head (`HeadMismatch`).
    #[error("the log head is {head}")]
    HeadMismatch {
        /// The head the service holds.
        head: u64,
    },
    /// The snapshot starts a key epoch but was sent to `publishSnapshot`, or its epoch is not the
    /// one in force (`EpochRejected`).
    #[error("the key epoch is not accepted here")]
    EpochRejected,
    /// A closure id has no durable receipt (`ArtifactMissing`).
    #[error("{} artifact(s) have no receipt", .missing.len())]
    ArtifactMissing {
        /// The ids that lack one.
        missing: Vec<ArtifactId>,
    },
    /// A different byte string already exists for this snapshot id (`ObjectConflict`).
    #[error("a different object already exists at this address")]
    ObjectConflict,
    /// The object is not well formed or disagrees with its own header (`MalformedObject`).
    #[error("the object is malformed")]
    MalformedObject,
    /// The snapshot plaintext is over the bound (`SnapshotTooLarge`).
    #[error("the snapshot is too large")]
    SnapshotTooLarge,
    /// The container is larger than one request may carry (`PayloadTooLarge`).
    #[error("the object is larger than one request may carry")]
    PayloadTooLarge,
    /// The account's quota is spent (`QuotaExceeded`).
    #[error("the quota is exceeded")]
    QuotaExceeded,
    /// The membership chain does not verify (`MembershipChainInvalid`).
    #[error("the membership chain is invalid")]
    MembershipChainInvalid,
    /// The epoch record does not verify (`EpochRecordInvalid`).
    #[error("the epoch record is invalid")]
    EpochRecordInvalid,
    /// The credential is not allowed to call this operation now: an account credential once the
    /// vault's genesis is installed, for the operations that need a device.
    #[error("the credential is not allowed here")]
    Forbidden,
    /// The service could not be reached or answered with a server error.
    #[error("the service is unavailable")]
    Unavailable,
}

/// The snapshot and epoch endpoints, from the client's side.
pub trait SnapshotService {
    /// `stageSnapshot` (protocol 4.9): hold `container` as the staged snapshot `snapshot_id`. An
    /// exact retry answers the same; different bytes for the id are
    /// [`ServiceError::ObjectConflict`].
    ///
    /// # Errors
    /// A [`ServiceError`].
    fn stage_snapshot(
        &self,
        vault_id: &[u8; 16],
        snapshot_id: &[u8; 16],
        container: &[u8],
    ) -> Result<StagedSnapshot, ServiceError>;

    /// `publishSnapshot` (protocol 4.9): publish the staged `snapshot_id` if and only if
    /// `base_seq` is the head and the snapshot's own, its epoch is the one in force, its sidecar
    /// matches the header's commitment and every closure id has a receipt, all in one
    /// transaction. Publishing a published snapshot answers the original result. A snapshot that
    /// starts a key epoch is [`ServiceError::EpochRejected`] here: `install_epoch` publishes it.
    ///
    /// # Errors
    /// A [`ServiceError`].
    fn publish_snapshot(
        &self,
        vault_id: &[u8; 16],
        snapshot_id: &[u8; 16],
        base_seq: u64,
        artifact_manifest: &[ArtifactId],
    ) -> Result<PublishedSnapshot, ServiceError>;

    /// `installEpoch` (protocol 4.4): install one epoch record, and the snapshot published with it,
    /// in one transaction. An exact retry answers the original values; a genesis for a vault the
    /// service knows is [`ServiceError::VaultExists`] unless it is this very genesis again.
    ///
    /// # Errors
    /// A [`ServiceError`].
    fn install_epoch(
        &self,
        vault_id: &[u8; 16],
        install: &EpochInstall<'_>,
    ) -> Result<InstalledEpoch, ServiceError>;

    /// `getLatestSnapshot` (protocol 4.9): the newest published snapshot, if any. It is how a
    /// client that lost a response finds out whether its publish took effect.
    ///
    /// # Errors
    /// A [`ServiceError`].
    fn latest_snapshot(
        &self,
        vault_id: &[u8; 16],
    ) -> Result<Option<PublishedSnapshot>, ServiceError>;
}

/// Where a snapshot's plaintext was read from (personal-cfo-1cltt.4 AC4).
///
/// A snapshot that truncates a tail must equal the log at its head `h`. The live vault holds
/// effects of local commands that no envelope has sequenced yet (the outbox), so it is the source of
/// a **genesis** only, taken at sync-enable before any `sync_base` or outbox row exists. Every other
/// snapshot (a re-snapshot, a cutover, a rotation) is exported from the state at its head.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotSource {
    /// The live vault, as it is now.
    LiveVault,
    /// A state equal to the log applied through `head`, never the live vault.
    StateAtHead {
        /// The log position the state was rebuilt to.
        head: u64,
    },
}
