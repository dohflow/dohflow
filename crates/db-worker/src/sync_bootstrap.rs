//! Birth of a replica from a snapshot (`docs/architecture/vault-format.md` C.8 "Replica
//! bootstrap"; `personal-cfo-1cltt.5`, slice B).
//!
//! A device with no vault becomes a replica by installing the snapshot of a vault that already
//! exists, never by seeding a new one. [`bootstrap_replica`] builds the replica **beside** the vault
//! path, at `<vault>.bootstrapping`, and only renames it into place when all of it is on disk:
//!
//! 1. a new vault is migrated to the snapshot's schema version, and the snapshot is installed
//!    ([`crate::snapshot_install`]): the origin's identity, then every row. A snapshot that carries
//!    a class-1b closure is installed with foreign keys off and without its `attachments` rows
//!    (their local crypto fields come from the artifacts);
//! 2. the vault is carried to this build's schema over the installed rows, and the void ledger
//!    (`sync_voids`, the snapshot's recorded voids as `seeded` entries) and the applied cursor
//!    (`sync_position`) are written in one transaction;
//! 3. **the closure** (`personal-cfo-1cltt.7`): every artifact is fetched, verified and
//!    materialized ([`crate::sync_bootstrap_artifacts`]), the attachment rows are created, the live
//!    closure is seeded, foreign keys come back on and `foreign_key_check` must find nothing, and the
//!    installed state is proved equal to the snapshot, table for table (a snapshot with no closure
//!    was proved inside the install, as before);
//! 4. the first base image is captured (`sync_base` `Ready` at the root, credentials stripped);
//! 5. the file is synced and renamed to the vault path, and the directory is synced.
//!
//! **The commit point is the rename.** Any failure before it discards the build, the spool of
//! verified ciphertext included, and the next start begins again.
//!
//! **A vault that exists always has its base** (B1): before the rename nothing is at the vault
//! path, so no command can be taken, and left out of the outbox because the base was not yet
//! recorded, between the install and the base. A crash at any step leaves at most the temporary
//! file, which the next start discards (that exact name and its journal files, nothing else) before
//! it begins again; a vault file that exists is never replaced.
//!
//! Which snapshot is a valid root is decided before anything is written, by the one rule set the
//! re-bootstrap out of a stop uses ([`sync_core::status::snapshot_is_valid_root`]).

use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rusqlite::{params, Connection, OptionalExtension};
use sync_core::artifact_transport::ArtifactTransport;
use sync_core::snapshot::{SnapshotError, Verified};
use sync_core::status::{snapshot_is_valid_root, EngineVersion, SnapshotFacts};
use sync_core::void::{HonoredVoid, LedgerError, VoidLedger};
use thiserror::Error;
use vault_crypto::sync_envelope::EpochKey;
use vault_crypto::Dek;

use crate::base_image::{base_image_path, sha256_file, BaseCapture, BaseImageError};
use crate::snapshot_install::{
    install_snapshot, install_snapshot_with_closure, prove_installed, InstallError,
    InstalledSnapshot,
};
use crate::sync_bootstrap_artifacts::{
    finish, materialize_closure, ClosureError, InvalidSnapshot, LocalFault, Spool, TransientFault,
};
use crate::{
    migrations, open_existing_keyed_raw, open_keyed_raw, DbError, DbWorker, KeyMaterial, VaultLease,
};

/// The suffix of the file a replica is built in, beside the vault path.
pub const BOOTSTRAPPING_SUFFIX: &str = ".bootstrapping";

/// Where the replica of the vault at `vault_path` is built before it becomes the vault.
#[must_use]
pub fn bootstrapping_path(vault_path: &Path) -> PathBuf {
    with_suffix(vault_path, BOOTSTRAPPING_SUFFIX)
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Where a snapshot's class-1b closure comes from: the artifact service, and the epoch key its
/// artifacts are sealed under. A snapshot with an empty closure and no attachment row needs neither.
#[derive(Clone, Copy)]
pub struct ClosureSource<'a> {
    /// The artifact transport.
    pub transport: &'a dyn ArtifactTransport,
    /// The key of the snapshot's epoch.
    pub key: &'a EpochKey,
}

impl std::fmt::Debug for ClosureSource<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClosureSource").finish_non_exhaustive()
    }
}

/// What the caller verified about the snapshot it wants to build a replica from. Its provenance
/// (membership, epoch, signature, the container digest) is the engine's existing snapshot
/// verification, not this module's: it passes the plaintext it opened and the facts it learned.
#[derive(Debug, Clone, Copy)]
pub struct ReplicaSource<'a> {
    /// The vault the snapshot belongs to: the replica's `vault_id`.
    pub vault_id: [u8; 16],
    /// The snapshot's root: the last log position it covers.
    pub root: u64,
    /// The verified snapshot plaintext.
    pub snapshot: &'a Verified,
    /// The snapshot's `engine_version`.
    pub engine_version: &'a EngineVersion,
    /// This build's `engine_version`.
    pub own_engine_version: &'a EngineVersion,
    /// The ranges `(seq, V - 1)` of the verified void records positioned after the root that the
    /// caller already holds (the tail it must pull anyway). A snapshot rooted inside any of them
    /// was built on poisoned state and is refused before anything is written.
    pub held_voids: &'a [(u64, u64)],
    /// Where the closure comes from, if the snapshot has one (or an attachment row). `None` for a
    /// snapshot that needs no artifact: a snapshot that needs them is refused, writing nothing.
    pub closure: Option<ClosureSource<'a>>,
}

/// A replica that is now the vault.
pub struct Bootstrapped {
    /// The vault, open: its base image is `Ready` at the root.
    pub worker: DbWorker,
    /// What the install wrote.
    pub installed: InstalledSnapshot,
    /// How many recorded voids were seeded into the ledger.
    pub voids_seeded: usize,
    /// The first base image.
    pub base: BaseCapture,
}

impl std::fmt::Debug for Bootstrapped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bootstrapped")
            .field("installed", &self.installed)
            .field("voids_seeded", &self.voids_seeded)
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

/// Why a replica was not created. A vault file never appears unless every step succeeded; a
/// failure after the temporary file exists leaves it for the next start to discard.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum BootstrapError {
    /// A vault already exists at the path: a replica is never built over one.
    #[error("a vault already exists at the path")]
    VaultExists,
    /// The snapshot is not a valid root for a replica: another `engine_version`, rooted inside a
    /// void range, or recording a void at or above its root.
    #[error("the snapshot is not a valid root")]
    NotAValidRoot,
    /// The snapshot is of a newer schema than this build: update-required, never installed.
    #[error("the snapshot is of schema version {snapshot}; this build is at {build}")]
    UpdateRequired {
        /// The snapshot's schema version.
        snapshot: u64,
        /// This build's.
        build: i64,
    },
    /// The snapshot's recorded voids are not a ledger.
    #[error("the snapshot's recorded voids are not valid: {0}")]
    Ledger(#[from] LedgerError),
    /// The snapshot is not valid.
    #[error("the snapshot is not valid: {0}")]
    Snapshot(#[from] SnapshotError),
    /// The install refused or failed; the temporary vault is discarded on the next start.
    #[error(transparent)]
    Install(#[from] InstallError),
    /// DETERMINISTIC: the snapshot's closure or rows are bad, at every replica. Its root is never
    /// picked again; the next root is the engine's to choose.
    #[error(transparent)]
    Invalid(#[from] InvalidSnapshot),
    /// TRANSIENT: the artifact service did not deliver. Retry the same root.
    #[error(transparent)]
    Transient(#[from] TransientFault),
    /// LOCAL: this device could not materialize the closure (`ArtifactMaterializationFailed`). It
    /// never marks the root invalid.
    #[error(transparent)]
    Local(#[from] LocalFault),
    /// The first base image could not be captured.
    #[error(transparent)]
    BaseImage(#[from] BaseImageError),
    /// The database failed.
    #[error(transparent)]
    Db(#[from] DbError),
    /// The filesystem failed.
    #[error("the replica could not be put in place: {0}")]
    Io(#[from] io::Error),
    /// A step of the bootstrap was cut short on purpose (tests only).
    #[cfg(test)]
    #[error("cut short after {0:?}")]
    Cut(Step),
}

impl From<ClosureError> for BootstrapError {
    fn from(error: ClosureError) -> Self {
        match error {
            ClosureError::Invalid(error) => Self::Invalid(error),
            ClosureError::Transient(error) => Self::Transient(error),
            ClosureError::Local(error) => Self::Local(error),
        }
    }
}

/// What a failed bootstrap says about what to do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// The snapshot is bad at every replica: never pick its root again.
    Deterministic,
    /// The service's fault: retry the same root.
    Transient,
    /// This device's fault (a disk, a key, a write): never marks the root invalid.
    Local,
}

impl BootstrapError {
    /// Which of the three it is. Recording a deterministic verdict, and choosing the next root, are
    /// the engine's (`personal-cfo-elciw`, ADR 0074 Addendum D rule 4 and E).
    #[must_use]
    pub fn kind(&self) -> FailureKind {
        match self {
            Self::Invalid(_) | Self::Snapshot(_) | Self::NotAValidRoot | Self::Ledger(_) => {
                FailureKind::Deterministic
            }
            Self::Install(
                InstallError::Snapshot(_)
                | InstallError::ColumnsMismatch { .. }
                | InstallError::UnknownTable { .. }
                | InstallError::MissingTable { .. }
                | InstallError::DigestMismatch { .. }
                | InstallError::HeaderMismatch,
            ) => FailureKind::Deterministic,
            Self::Transient(_) => FailureKind::Transient,
            _ => FailureKind::Local,
        }
    }
}

impl From<rusqlite::Error> for BootstrapError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Db(DbError::from(error))
    }
}

/// The steps of a bootstrap, each a point a crash can fall after (the tests cut it there).
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// The temporary vault exists, migrated to the snapshot's schema, with nothing installed.
    Created,
    /// The snapshot's rows are installed.
    Installed,
    /// The vault is at this build's schema and the ledger and cursor are written.
    Ledger,
    /// The vault is open as a worker.
    Opened,
    /// The base image is captured.
    BaseImage,
    /// The temporary file is closed and synced, and not yet renamed.
    Synced,
    /// The file is renamed into place and the directory synced.
    Renamed,
}

#[cfg(test)]
thread_local! {
    /// What `PRAGMA foreign_keys` held on the build connection when the base image was captured.
    static FOREIGN_KEYS_AT_CAPTURE: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
    static CUT_AFTER: std::cell::Cell<Option<Step>> = const { std::cell::Cell::new(None) };
}

/// Run `f` with the bootstrap cut short right after `step`, as a crash there would: the temporary
/// file is left as it is.
#[cfg(test)]
pub(crate) fn cut_after<T>(step: Step, f: impl FnOnce() -> T) -> T {
    CUT_AFTER.with(|cut| cut.set(Some(step)));
    let result = f();
    CUT_AFTER.with(|cut| cut.set(None));
    result
}

#[cfg(test)]
fn reached(step: Step) -> Result<(), BootstrapError> {
    if CUT_AFTER.with(std::cell::Cell::get) == Some(step) {
        return Err(BootstrapError::Cut(step));
    }
    Ok(())
}

fn fsync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

/// Whether `base.db` beside the vault at `path` is the base image the stale temporary replica
/// captured: the temporary replica opens under `dek`, its `sync_base` row is `Ready`, and the
/// recorded digest is the digest of the file on disk. Anything else (it cannot be opened, it has no
/// base, the digests differ, there is no file) is "not its own", and the file is left alone.
fn base_is_the_stale_replicas(temp: &Path, path: &Path, dek: &Dek) -> bool {
    let image = base_image_path(path);
    let Ok(on_disk) = sha256_file(&image) else {
        return false;
    };
    let Ok(conn) = open_existing_keyed_raw(temp, dek) else {
        return false;
    };
    let recorded: rusqlite::Result<Option<Vec<u8>>> = conn.query_row(
        "SELECT base_digest FROM sync_base WHERE singleton = 1 AND state = 'Ready'",
        [],
        |r| r.get(0),
    );
    matches!(recorded, Ok(Some(digest)) if digest.as_slice() == on_disk.as_slice())
}

/// Remove a temporary replica a crashed bootstrap left: that exact name and its journal files. A
/// bootstrap cut after its base capture also left `sync/base.db`, which belongs to no vault. It is
/// removed only when **no vault exists at `path`** (the caller checked) **and the file is the
/// abandoned bootstrap's own** ([`base_is_the_stale_replicas`]); any other base image is never
/// touched. The capture's in-flight copy is left to the next capture, which removes it first.
fn discard_stale(temp: &Path, path: &Path, dek: &Dek) -> io::Result<()> {
    // The spool of the abandoned build (that exact name, a directory this module made).
    match fs::remove_dir_all(with_suffix(path, SPOOL_SUFFIX)) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        // Something that is not a directory sits at the spool's name: leave it; making the spool
        // there will fail as a local fault.
        Err(_) => {}
    }
    let own_base = temp.exists() && base_is_the_stale_replicas(temp, path, dek);
    for suffix in ["", "-wal", "-shm", "-journal"] {
        match fs::remove_file(with_suffix(temp, suffix)) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    if own_base {
        match fs::remove_file(base_image_path(path)) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn to_i64(value: u64) -> Result<i64, BootstrapError> {
    i64::try_from(value).map_err(|_| BootstrapError::NotAValidRoot)
}

/// Write the void ledger and the applied cursor of a replica rooted at `root`, in ONE transaction:
/// the recorded voids as `seeded` entries in ascending order with dense ordinals, and
/// `next_to_apply = root + 1`.
fn write_ledger(
    conn: &mut Connection,
    ledger: &VoidLedger,
    root: u64,
) -> Result<(), BootstrapError> {
    let root = to_i64(root)?;
    let tx = conn.transaction()?;
    for (index, void) in ledger.seeded().iter().enumerate() {
        tx.execute(
            "INSERT INTO sync_voids (ordinal, void_id, first, last, origin)
             VALUES (?4, ?1, ?2, ?3, 'seeded')",
            params![
                void.void_id.as_slice(),
                to_i64(void.first)?,
                to_i64(void.last)?,
                i64::try_from(index + 1).map_err(|_| BootstrapError::NotAValidRoot)?,
            ],
        )?;
    }
    tx.execute(
        "INSERT INTO sync_position (singleton, snapshot_root, next_to_apply)
         VALUES (1, ?1, ?2)",
        params![
            root,
            root.checked_add(1).ok_or(BootstrapError::NotAValidRoot)?
        ],
    )?;
    tx.commit()?;
    Ok(())
}

/// Build a replica of the snapshot in `source` and make it the vault at `path`.
///
/// Nothing is written unless the snapshot is a valid root for a replica and of a schema this build
/// can carry; from then on a failure leaves at most `<path>.bootstrapping`, and the vault path is
/// untouched until every step has succeeded. The vault's lock is held throughout and handed to the
/// worker that is returned.
///
/// # Errors
/// A [`BootstrapError`].
pub fn bootstrap_replica(
    path: &Path,
    dek: &Arc<Dek>,
    source: &ReplicaSource<'_>,
) -> Result<Bootstrapped, BootstrapError> {
    let lease = VaultLease::acquire(path)?;
    if path.try_exists()? {
        return Err(BootstrapError::VaultExists);
    }

    // Everything that can be decided from the snapshot alone, before a byte is written.
    let recorded: Vec<HonoredVoid> = source
        .snapshot
        .manifest
        .recorded_voids()?
        .into_iter()
        .map(|void| HonoredVoid {
            void_id: void.void_id,
            first: void.first,
            last: void.last,
        })
        .collect();
    let facts = SnapshotFacts {
        root: source.root,
        own_engine_version: source.own_engine_version,
        engine_version: source.engine_version,
        recorded_voids: &recorded,
        held_voids: source.held_voids,
    };
    // The engine version first: a snapshot of another version is update-required itself, and says
    // nothing about its schema.
    if !snapshot_is_valid_root(&facts) {
        return Err(BootstrapError::NotAValidRoot);
    }
    // A snapshot that needs its closure is refused, writing nothing, when the caller has no artifact
    // service to give: no replica is ever built from half a snapshot.
    let needs_artifacts = !source.snapshot.manifest.artifacts.is_empty()
        || source
            .snapshot
            .manifest
            .tables
            .iter()
            .any(|t| t.name == "attachments" && t.row_count != "0");
    if needs_artifacts && source.closure.is_none() {
        return Err(BootstrapError::Install(InstallError::NeedsArtifacts));
    }
    let ledger = VoidLedger::from_snapshot(&recorded, source.root)?;
    let snapshot_schema: u64 = source
        .snapshot
        .manifest
        .schema_version
        .parse()
        .map_err(|_| SnapshotError::ManifestInvalid("the schema version"))?;
    let snapshot_schema = i64::try_from(snapshot_schema)
        .map_err(|_| SnapshotError::ManifestInvalid("the schema version"))?;
    if snapshot_schema > migrations::CURRENT_VERSION {
        return Err(BootstrapError::UpdateRequired {
            snapshot: u64::try_from(snapshot_schema).unwrap_or(u64::MAX),
            build: migrations::CURRENT_VERSION,
        });
    }
    let root = to_i64(source.root)?;

    // A crash left a temporary replica: discard it, and the base image it captured before the
    // crash, and begin again.
    let temp = bootstrapping_path(path);
    discard_stale(&temp, path, dek)?;

    // 1. Create and install. A snapshot that carries a closure is installed with foreign keys off
    // (a provenance link names a source record only a batch artifact makes) and without its
    // attachment rows; both are made good in step 3.
    let mut conn = open_keyed_raw(&temp, dek)?;
    migrations::run_migrations_to(&mut conn, snapshot_schema)?;
    #[cfg(test)]
    reached(Step::Created)?;
    // Only a snapshot with a closure is installed with foreign keys off. Any other keeps them on, so
    // the install's deferred check (C.8's commit-time check) still binds.
    conn.pragma_update(None, "foreign_keys", !needs_artifacts)?;
    // A snapshot with no closure and no attachment row is installed and proved whole, at its own
    // schema, as before.
    let (installed, snapshot_attachments) = if needs_artifacts {
        install_snapshot_with_closure(&mut conn, source.vault_id, source.snapshot)?
    } else {
        (
            install_snapshot(&mut conn, source.vault_id, source.snapshot)?,
            Vec::new(),
        )
    };
    #[cfg(test)]
    reached(Step::Installed)?;

    // 2. This build's schema over the installed rows; the ledger and the cursor, together.
    migrations::run_migrations(&mut conn)?;
    write_ledger(&mut conn, &ledger, source.root)?;
    conn.pragma_update(None, "foreign_keys", !needs_artifacts)?;
    #[cfg(test)]
    reached(Step::Ledger)?;

    // 3. Open it as a worker (nothing is seeded), then the closure.
    let worker = DbWorker::bootstrap_replica_vault(
        temp.clone(),
        conn,
        KeyMaterial::Raw(Arc::clone(dek)),
        lease.file,
    )?;
    #[cfg(test)]
    reached(Step::Opened)?;
    let spool = Spool::new(with_suffix(path, SPOOL_SUFFIX));
    let closure_result = complete_the_build(
        &worker,
        source,
        needs_artifacts,
        &snapshot_attachments,
        &spool,
    );
    // The spool is verified ciphertext waiting for siblings: gone whatever the outcome.
    spool.remove();
    closure_result?;
    #[cfg(test)]
    FOREIGN_KEYS_AT_CAPTURE.with(|seen| {
        let on: i64 = worker
            .lock()
            .conn
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .unwrap_or(-1);
        seen.set(Some(on == 1));
    });
    let base = worker.initialize_base_image(root)?;
    #[cfg(test)]
    reached(Step::BaseImage)?;

    // 4. Close it, sync it, and make it the vault.
    {
        let guard = worker.lock();
        let busy: i64 = guard
            .conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))?;
        if busy != 0 {
            return Err(DbError::CheckpointBusy.into());
        }
    }
    let closed = worker.close_into_lease();
    File::open(&temp)?.sync_all()?;
    #[cfg(test)]
    reached(Step::Synced)?;
    if path.try_exists()? {
        return Err(BootstrapError::VaultExists);
    }
    fs::rename(&temp, path)?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fsync_dir(parent)?;
    // The journal files of the temporary name are gone with its last connection (checkpoint and
    // close); none is left to carry over.
    #[cfg(test)]
    reached(Step::Renamed)?;

    // The vault, now at its own path: opened as any existing vault is. Its base is `Ready`.
    let conn = open_existing_keyed_raw(path, dek)?;
    let worker = DbWorker::bootstrap(
        path.to_path_buf(),
        conn,
        KeyMaterial::Raw(Arc::clone(dek)),
        closed.file,
    )?;
    Ok(Bootstrapped {
        worker,
        installed,
        voids_seeded: ledger.seeded().len(),
        base,
    })
}

/// Make the installed rows good: materialize the snapshot's closure and create its attachment rows
/// (when it has any), seed the live closure, bring foreign keys back and check the whole database,
/// and prove the installed state equal to the snapshot, table for table.
fn complete_the_build(
    worker: &DbWorker,
    source: &ReplicaSource<'_>,
    needs_artifacts: bool,
    attachments: &[crate::snapshot_install::SnapshotAttachment],
    spool: &Spool,
) -> Result<(), BootstrapError> {
    // Gated on what the snapshot needs, not on whether the caller passed a service: the engine may
    // pass one whenever Sync is on, and a snapshot with no artifact was proved whole in its install.
    if let (true, Some(closure)) = (needs_artifacts, source.closure) {
        let materialized = materialize_closure(
            worker,
            closure.transport,
            closure.key,
            &source.snapshot.manifest.artifacts,
            spool,
        )?;
        finish(worker, closure.key.key_epoch(), attachments, &materialized)?;
    }
    let guard = worker.lock();
    // Foreign keys are on for everything from here, on both paths: the proof, the class-2 rebuild,
    // the base capture and every later open. A row that names a row that is not there, once the
    // closure is materialized, is a defect of the snapshot at every replica.
    guard.conn.pragma_update(None, "foreign_keys", true)?;
    let dangling = {
        let mut stmt = guard.conn.prepare("PRAGMA foreign_key_check")?;
        let mut rows = stmt.query([])?;
        rows.next()?.is_some()
    };
    if dangling {
        return Err(InvalidSnapshot::DanglingReference.into());
    }
    if needs_artifacts {
        // A snapshot with no closure was proved whole inside its install.
        prove_installed(&guard.conn, source.snapshot)?;
    }
    Ok(())
}

/// Whether foreign keys were on when the last bootstrap on this thread captured its base image.
#[cfg(test)]
pub(crate) fn foreign_keys_at_capture() -> Option<bool> {
    FOREIGN_KEYS_AT_CAPTURE.with(std::cell::Cell::get)
}

/// The suffix of the directory verified ciphertext waits in while a closure is materialized.
pub const SPOOL_SUFFIX: &str = ".bootstrapping.spool";

/// A replica's applied cursor (`sync_position`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncPosition {
    /// The last log position of the snapshot the replica bootstrapped from; `None` for an origin.
    pub snapshot_root: Option<u64>,
    /// The lowest log position the replica has neither applied nor skipped.
    pub next_to_apply: u64,
}

impl DbWorker {
    /// The replica's applied cursor, or `None` when this vault has none (it was never bootstrapped
    /// and Sync has not armed a cursor on it).
    ///
    /// # Errors
    /// [`DbError`] if the row cannot be read.
    pub fn sync_position(&self) -> Result<Option<SyncPosition>, DbError> {
        let guard = self.lock();
        let row: Option<(Option<i64>, i64)> = guard
            .conn
            .query_row(
                "SELECT snapshot_root, next_to_apply FROM sync_position WHERE singleton = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        row.map(|(root, next)| {
            let to_u64 = |value: i64| {
                u64::try_from(value)
                    .map_err(|_| DbError::SelfTestFailed("a negative sync position".to_owned()))
            };
            Ok(SyncPosition {
                snapshot_root: root.map(to_u64).transpose()?,
                next_to_apply: to_u64(next)?,
            })
        })
        .transpose()
    }

    /// The void ledger: every `sync_voids` row in ordinal order, `seeded` ones and `honored` ones
    /// kept apart.
    ///
    /// # Errors
    /// [`DbError`] if a row cannot be read or the rows are not a ledger.
    pub fn void_ledger(&self) -> Result<VoidLedger, DbError> {
        let guard = self.lock();
        let mut statement = guard
            .conn
            .prepare("SELECT void_id, first, last, origin FROM sync_voids ORDER BY ordinal")?;
        let rows = statement
            .query_map([], |r| {
                Ok((
                    r.get::<_, Vec<u8>>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let bad = |what: &str| DbError::SelfTestFailed(format!("sync_voids: {what}"));
        let mut seeded = Vec::new();
        let mut honored = Vec::new();
        for (void_id, first, last, origin) in rows {
            let void = HonoredVoid {
                void_id: void_id.try_into().map_err(|_| bad("a void id length"))?,
                first: u64::try_from(first).map_err(|_| bad("a position"))?,
                last: u64::try_from(last).map_err(|_| bad("a position"))?,
            };
            match origin.as_str() {
                "seeded" => seeded.push(void),
                "honored" => honored.push(void),
                _ => return Err(bad("an origin")),
            }
        }
        VoidLedger::from_parts(seeded, honored).map_err(|_| bad("the ranges"))
    }
}
