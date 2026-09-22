//! Vault lifecycle — create / unlock / lock (personal-cfo-vhv).
//!
//! This is the integration layer that assembles the `vault-crypto` primitives
//! (Argon2id KEK + AES-256-GCM DEK envelope) with the `db-worker` raw-key open
//! into a usable vault, per ADR 0002. It owns the **plaintext envelope sidecar**:
//! the salt, KDF parameters, and wrapped DEK are needed before the database can
//! be decrypted, so they live in `<vault.db>.envelope` next to the encrypted DB.
//! The wrapped DEK is AEAD-encrypted and useless without the master password.
//!
//! Layering: `vault-crypto` does the math, `db-worker` owns the connection, and
//! this module orchestrates. The desktop command layer reaches a vault only
//! through [`Kernel::unlock_vault`] / [`Kernel::create_vault`] — it never opens
//! SQLCipher directly.
//!
//! Out of scope here (their own beads): the vault state machine + startup
//! self-test (`personal-cfo-tg5`), the Tauri create/unlock/lock commands + UI
//! (`personal-cfo-3ry` / `-8v2`), encrypted attachments (`-bcj`), backup/restore
//! (`-ef3` / `-au3`), rekey + envelope v1→v2 (`-2y8`), and KDF calibration
//! (`-0sqk`).

use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use db_worker::{DbWorker, WorkerState};
use vault_crypto::{
    derive_kek, generate_dek, generate_salt, rewrap_envelope, unwrap_dek, wrap_dek, Profile,
    VaultEnvelope,
};

use crate::{Kernel, KernelError};

/// The sidecar that holds a vault's unlock material, derived from the DB path by
/// appending `.envelope` (so `…/vault.db` → `…/vault.db.envelope`).
fn sidecar_path(db_path: &Path) -> PathBuf {
    let mut os = db_path.as_os_str().to_owned();
    os.push(".envelope");
    PathBuf::from(os)
}

/// Remove `path`, treating an already-absent file as success (personal-cfo-j0cg.5).
fn remove_if_present(path: &Path) -> Result<(), KernelError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(KernelError::Vault(format!(
            "deleting {}: {e}",
            path.display()
        ))),
    }
}

impl Kernel {
    /// Create a brand-new encrypted vault at `path`, protected by `password`.
    ///
    /// Generates a per-vault salt and a random 256-bit DEK, derives the KEK from
    /// the password (Argon2id, `InteractiveDefault`), wraps the DEK under the
    /// KEK, writes the envelope sidecar, then creates the SQLCipher database
    /// keyed by the DEK. Returns an unlocked kernel.
    ///
    /// # Errors
    /// - [`KernelError::VaultExists`] if an envelope already exists at `path`.
    /// - [`KernelError::Vault`] on a crypto or sidecar-I/O failure.
    /// - [`KernelError::Persistence`] if the database cannot be created.
    pub fn create_vault(path: impl AsRef<Path>, password: &[u8]) -> Result<Self, KernelError> {
        let db_path = path.as_ref();
        let sidecar = sidecar_path(db_path);

        let params = Profile::InteractiveDefault.params();
        let salt = generate_salt()?;
        let kek = derive_kek(password, &salt, &params)?;
        let dek = generate_dek()?;
        let wrapped = wrap_dek(&kek, &dek)?;
        let envelope = VaultEnvelope::new(params, salt, wrapped);

        // Write the sidecar first (create-new, so we never clobber an existing
        // vault). If keying the DB then fails, roll the sidecar back so a failed
        // create leaves no half-initialized vault behind.
        write_sidecar_new(&sidecar, &envelope.to_bytes())?;
        let worker = match DbWorker::open_with_raw_key(db_path, dek) {
            Ok(worker) => worker,
            Err(error) => {
                let _ = std::fs::remove_file(&sidecar);
                return Err(error.into());
            }
        };
        Ok(Self::with_worker(worker))
    }

    /// Unlock the existing vault at `path` with `password`.
    ///
    /// Reads the envelope sidecar, derives the KEK, and unwraps the DEK — a
    /// wrong password makes the AEAD tag fail and yields
    /// [`KernelError::VaultUnlockFailed`] without ever opening the database.
    ///
    /// # Errors
    /// - [`KernelError::VaultNotFound`] if there is no envelope at `path`.
    /// - [`KernelError::VaultUnlockFailed`] if the password is wrong.
    /// - [`KernelError::Vault`] if the envelope is malformed or unreadable.
    /// - [`KernelError::Persistence`] if the database cannot be opened.
    /// - [`KernelError::VaultInUse`] if another process already owns the
    ///   unlocked vault.
    pub fn unlock_vault(path: impl AsRef<Path>, password: &[u8]) -> Result<Self, KernelError> {
        let db_path = path.as_ref();
        let sidecar = sidecar_path(db_path);

        let bytes = match std::fs::read(&sidecar) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Err(KernelError::VaultNotFound)
            }
            Err(error) => return Err(KernelError::Vault(error.to_string())),
        };
        let envelope = VaultEnvelope::from_bytes(&bytes)?;
        let kek = derive_kek(password, &envelope.salt, &envelope.kdf)?;
        let dek = unwrap_dek(&kek, &envelope.wrapped)?;
        let worker = DbWorker::open_with_raw_key(db_path, dek)?;
        Ok(Self::with_worker(worker))
    }

    /// Lock the vault by consuming the kernel. Dropping it drops the db-worker,
    /// whose raw key (the DEK) is zeroized on drop (ADR 0002, `personal-cfo-1t0`).
    pub fn lock(self) {
        // The work is in `Drop`; this method exists to make intent explicit and
        // to statically guarantee the unlocked kernel is no longer usable.
    }

    /// Export an encrypted backup of this unlocked vault to `out_path` (ADR 0024,
    /// personal-cfo-ef3). Bundles a consistent `vault.db` snapshot, the envelope
    /// sidecar, and every attachment blob into one password-encrypted package.
    /// `created_at`, `backup_id`, and `app_version` are supplied by the caller
    /// (the kernel reads no clock).
    ///
    /// # Errors
    /// [`KernelError`] on a snapshot, file-read, crypto, or write failure.
    pub fn export_backup(
        &self,
        password: &[u8],
        out_path: &Path,
        app_version: &str,
        created_at: String,
        backup_id: uuid::Uuid,
    ) -> Result<(), KernelError> {
        let db_path = self.worker.db_path();
        let envelope_bytes = std::fs::read(sidecar_path(db_path))
            .map_err(|e| KernelError::Vault(format!("reading envelope: {e}")))?;
        let blobs = read_blobs(&self.worker.blobs_dir())?;
        let db_bytes = self.worker.snapshot_bytes()?;

        let inputs = crate::backup::BackupInputs {
            db_bytes,
            envelope_bytes,
            blobs,
            app_version: app_version.to_owned(),
            schema_version: self.worker.schema_version(),
            vault_envelope_version: vault_crypto::envelope::ENVELOPE_VERSION,
            created_at,
            backup_id,
        };
        let package = crate::backup::assemble(password, &inputs)
            .map_err(|e| KernelError::Vault(format!("assembling backup: {e}")))?;
        std::fs::write(out_path, &package)
            .map_err(|e| KernelError::Vault(format!("writing backup: {e}")))?;
        Ok(())
    }

    /// Restore an encrypted backup package into a **fresh** vault at
    /// `dest_db_path`, returning an unlocked kernel on the restored vault (ADR
    /// 0024 §5, personal-cfo-au3). Verify-then-install: the package is parsed,
    /// decrypted, and every ciphertext hash checked *before* any file is written;
    /// a wrong password fails before touching disk; an existing vault at the
    /// destination is never overwritten.
    ///
    /// # Errors
    /// - [`KernelError::VaultUnlockFailed`] if the password is wrong.
    /// - [`KernelError::VaultExists`] if a vault already exists at `dest_db_path`.
    /// - [`KernelError::Vault`] if the package is malformed, a hash mismatches,
    ///   the backup is from a newer app version, or a file write fails.
    pub fn restore_backup(
        package_path: &Path,
        password: &[u8],
        dest_db_path: &Path,
    ) -> Result<Self, KernelError> {
        // 1. Read + disassemble: parse, decrypt to memory, verify every hash. A
        //    wrong password fails here (the payload AEAD tag does not verify) —
        //    before any byte is written to disk.
        let package = std::fs::read(package_path)
            .map_err(|e| KernelError::Vault(format!("reading backup: {e}")))?;
        let restored = crate::backup::disassemble(password, &package).map_err(map_restore_err)?;

        // 2. Version compatibility: a backup newer than this build understands
        //    cannot be restored (no silent corruption). An older schema migrates
        //    forward when the restored vault is opened in step 4.
        if restored.manifest.schema_version > db_worker::CURRENT_SCHEMA_VERSION {
            return Err(KernelError::Vault(format!(
                "backup is from a newer app (schema {} > {})",
                restored.manifest.schema_version,
                db_worker::CURRENT_SCHEMA_VERSION
            )));
        }

        // 3. Never overwrite an existing vault. The create-new sidecar write is
        //    the atomic guard (as in create_vault); roll back on any failure so a
        //    partial restore never leaves a half-vault behind.
        let sidecar = sidecar_path(dest_db_path);
        if dest_db_path.exists() {
            return Err(KernelError::VaultExists);
        }
        write_sidecar_new(&sidecar, &restored.envelope_bytes)?;
        if let Err(error) = install_payload(dest_db_path, &restored) {
            let _ = std::fs::remove_file(&sidecar);
            let _ = std::fs::remove_file(dest_db_path);
            if let Some(parent) = dest_db_path.parent() {
                let _ = std::fs::remove_dir_all(parent.join("blobs"));
            }
            return Err(error);
        }

        // 4. Reopen the restored vault to verify it works on a fresh instance —
        //    this also runs any pending migrations for an older-schema backup.
        Self::unlock_vault(dest_db_path, password)
    }
}

/// Verify `old` against the envelope sidecar of the vault at `db_path`, rewrap
/// its DEK under `new` (fresh salt, current `InteractiveDefault` profile), and
/// **atomically** replace the sidecar: the new envelope is written to
/// `<sidecar>.tmp`, synced, then renamed over the sidecar — a crash at any
/// point leaves either the old or the new envelope fully intact, never a torn
/// file (personal-cfo-zxq).
fn rewrap_sidecar(db_path: &Path, old: &[u8], new: &[u8]) -> Result<(), KernelError> {
    let sidecar = sidecar_path(db_path);
    let bytes = std::fs::read(&sidecar)
        .map_err(|error| KernelError::Vault(format!("reading envelope: {error}")))?;
    let envelope = VaultEnvelope::from_bytes(&bytes)?;
    // A wrong `old` fails the AEAD unwrap here (→ VaultUnlockFailed via `From`)
    // before any file is touched.
    let renewed = rewrap_envelope(&envelope, old, new, Profile::InteractiveDefault.params())?;

    // Write-then-rename. A stale `.tmp` from an earlier interrupted change is
    // simply overwritten (create truncates); only the rename installs it.
    let tmp = {
        let mut os = sidecar.as_os_str().to_owned();
        os.push(".tmp");
        PathBuf::from(os)
    };
    let mut file = std::fs::File::create(&tmp)
        .map_err(|error| KernelError::Vault(format!("writing envelope tmp: {error}")))?;
    file.write_all(&renewed.to_bytes())
        .map_err(|error| KernelError::Vault(format!("writing envelope tmp: {error}")))?;
    // Flush to disk before the rename makes it live, so the swap survives a crash.
    file.sync_all()
        .map_err(|error| KernelError::Vault(format!("syncing envelope tmp: {error}")))?;
    std::fs::rename(&tmp, &sidecar)
        .map_err(|error| KernelError::Vault(format!("installing new envelope: {error}")))?;
    Ok(())
}

/// Write `bytes` to `sidecar`, failing with [`KernelError::VaultExists`] if it
/// is already present (the create-new flag makes this an atomic guard).
fn write_sidecar_new(sidecar: &Path, bytes: &[u8]) -> Result<(), KernelError> {
    let mut file = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(sidecar)
    {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            return Err(KernelError::VaultExists)
        }
        Err(error) => return Err(KernelError::Vault(error.to_string())),
    };
    file.write_all(bytes)
        .map_err(|error| KernelError::Vault(error.to_string()))
}

/// Read every attachment blob in `dir` as `(storage_id = filename, ciphertext)`
/// in a deterministic order. A missing directory (no attachments yet) is an
/// empty list; `.tmp` files (a crash-interrupted import) are skipped.
fn read_blobs(dir: &Path) -> Result<Vec<(String, Vec<u8>)>, KernelError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(KernelError::Vault(format!("reading blobs dir: {error}"))),
    };
    let mut blobs = Vec::new();
    for entry in entries {
        let entry =
            entry.map_err(|error| KernelError::Vault(format!("reading blobs dir: {error}")))?;
        let path = entry.path();
        // Blob files are named by their hex storage id (no extension); skip a
        // leftover `.tmp` and any subdirectory.
        if path.is_file() && path.extension().is_none() {
            let storage_id = entry.file_name().to_string_lossy().into_owned();
            let ciphertext = std::fs::read(&path)
                .map_err(|error| KernelError::Vault(format!("reading blob: {error}")))?;
            blobs.push((storage_id, ciphertext));
        }
    }
    blobs.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(blobs)
}

/// Write a restored backup's `vault.db` and attachment blobs into the vault
/// directory containing `dest_db_path`. The envelope sidecar is written
/// separately (create-new) by the caller, as the no-clobber guard.
fn install_payload(
    dest_db_path: &Path,
    restored: &crate::backup::RestoredBackup,
) -> Result<(), KernelError> {
    std::fs::write(dest_db_path, &restored.db_bytes)
        .map_err(|e| KernelError::Vault(format!("writing vault.db: {e}")))?;
    if !restored.blobs.is_empty() {
        let blobs_dir = dest_db_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("blobs");
        std::fs::create_dir_all(&blobs_dir)
            .map_err(|e| KernelError::Vault(format!("creating blobs dir: {e}")))?;
        for (storage_id, ciphertext) in &restored.blobs {
            std::fs::write(blobs_dir.join(storage_id), ciphertext)
                .map_err(|e| KernelError::Vault(format!("writing blob: {e}")))?;
        }
    }
    Ok(())
}

/// Map a backup-parse failure to a kernel error. A wrong password makes the
/// payload AEAD fail to open; surface it as [`KernelError::VaultUnlockFailed`],
/// matching `unlock_vault`.
fn map_restore_err(error: crate::backup::BackupError) -> KernelError {
    match error {
        crate::backup::BackupError::Crypto(_) => KernelError::VaultUnlockFailed,
        other => KernelError::Vault(other.to_string()),
    }
}

// ───────────────────────────── State machine (personal-cfo-tg5) ─────────────

/// The vault lifecycle states (plan §6.2.1, ADR 0002).
///
/// `Rekeying`, `Migrating`, and `RestoringBackup` are part of the graph but are
/// *entered/exited* by the beads that own those operations (`personal-cfo-2y8` /
/// `-wkn` / `-au3`); this bead defines the states and their legal edges and
/// drives only create / unlock / lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultState {
    /// No vault exists on disk yet.
    NoVault,
    /// A vault is being created (transient).
    CreatingVault,
    /// A vault exists but is sealed; a password is required to unlock.
    Locked,
    /// An unlock is in progress (transient).
    Unlocking,
    /// The vault is open; the DEK is in memory and writes are accepted.
    Unlocked,
    /// A lock is in progress (transient).
    Locking,
    /// A key rotation is in progress (driven by `personal-cfo-2y8`).
    Rekeying,
    /// A schema migration is in progress (driven by `personal-cfo-wkn`).
    Migrating,
    /// A backup restore is in progress (driven by `personal-cfo-au3`).
    RestoringBackup,
    /// The vault is half-open or inconsistent and needs explicit recovery.
    CorruptNeedsRecovery,
}

impl VaultState {
    /// Whether a transition from `self` to `next` is permitted by §6.2.1.
    ///
    /// A fault can be detected from any state, so every state may transition to
    /// [`CorruptNeedsRecovery`](VaultState::CorruptNeedsRecovery).
    #[must_use]
    pub fn can_transition_to(self, next: VaultState) -> bool {
        use VaultState::{
            CorruptNeedsRecovery, CreatingVault, Locked, Locking, Migrating, NoVault, Rekeying,
            RestoringBackup, Unlocked, Unlocking,
        };
        if next == CorruptNeedsRecovery {
            return true;
        }
        matches!(
            (self, next),
            (NoVault, CreatingVault)
                | (CreatingVault, Unlocked)
                | (CreatingVault, NoVault) // create aborted before anything persisted
                | (Locked, Unlocking)
                | (Unlocking, Unlocked)
                | (Unlocking, Locked) // wrong password / aborted unlock
                | (Unlocked, Locking)
                | (Locking, Locked)
                | (Unlocked, Rekeying)
                | (Rekeying, Unlocked)
                | (Unlocked, Migrating)
                | (Migrating, Unlocked)
                | (Locked, RestoringBackup)
                | (RestoringBackup, Locked)
                | (CorruptNeedsRecovery, RestoringBackup)
                | (CorruptNeedsRecovery, Locked)
                // Fresh-clone restore (au3): an empty machine restores a backup
                // into a fresh vault and ends unlocked. Restore-over-an-existing
                // vault (clobber with confirmation) stays out of scope (ADR 0024
                // §5; the engine refuses to clobber).
                | (NoVault, RestoringBackup)
                | (RestoringBackup, Unlocked)
        )
    }
}

/// Classify the vault on disk at `path` — the pre-unlock startup self-test.
///
/// Looks only at the presence of the encrypted DB and its envelope sidecar:
/// neither → [`NoVault`](VaultState::NoVault); both → [`Locked`](VaultState::Locked);
/// exactly one → [`CorruptNeedsRecovery`](VaultState::CorruptNeedsRecovery) (an
/// interrupted create, or a DB whose envelope was lost — unopenable as-is).
#[must_use]
pub fn classify_vault(path: impl AsRef<Path>) -> VaultState {
    let db_path = path.as_ref();
    let sidecar = sidecar_path(db_path);
    match (db_path.exists(), sidecar.exists()) {
        (false, false) => VaultState::NoVault,
        (true, true) => VaultState::Locked,
        _ => VaultState::CorruptNeedsRecovery,
    }
}

/// The result of a post-unlock vault health check (§6.2.1 startup self-test).
///
/// Each field is a single coherence check. `read_models_current` is
/// *informational*: a drifted read model is rebuildable, not corruption, so it
/// does not fail [`is_healthy`](VaultHealth::is_healthy).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VaultHealth {
    /// The writer is in its normal (`Healthy`) state.
    pub writer_healthy: bool,
    /// WAL/SHM/temp pragmas match policy (db-worker `self_test`).
    pub wal_configured: bool,
    /// The migrated schema version matches the vault metadata stamp.
    pub schema_coherent: bool,
    /// `PRAGMA integrity_check` reports `ok` — the database is not corrupt.
    pub integrity_ok: bool,
    /// Every stored attachment's encrypted blob is present on disk.
    pub attachments_consistent: bool,
    /// Materialized read models match a fresh compute (else: rebuildable).
    pub read_models_current: bool,
}

impl VaultHealth {
    /// Whether the vault is healthy enough to use. Read-model drift is excluded
    /// (it is recoverable by a deterministic rebuild, not a fault).
    #[must_use]
    pub fn is_healthy(&self) -> bool {
        self.writer_healthy
            && self.wal_configured
            && self.schema_coherent
            && self.integrity_ok
            && self.attachments_consistent
    }
}

/// Drives the vault through its lifecycle states, owning the unlocked
/// [`Kernel`] while open. This is what the Tauri layer (`personal-cfo-8v2` /
/// `-3ry`) will hold in place of a bare `Option<Kernel>`.
pub struct VaultController {
    path: PathBuf,
    state: VaultState,
    kernel: Option<Kernel>,
}

impl VaultController {
    /// Inspect the vault on disk at `path` and enter the matching initial state
    /// (`NoVault` / `Locked` / `CorruptNeedsRecovery`).
    #[must_use]
    pub fn open(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref().to_path_buf();
        let state = classify_vault(&path);
        Self {
            path,
            state,
            kernel: None,
        }
    }

    /// The vault file this controller is bound to. Stable across
    /// lock/unlock; changes only on [`Self::switch_to`] — connector work uses
    /// it as the vault-identity witness so an in-flight sync can never write
    /// into a different vault (personal-cfo-gglk review).
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The current lifecycle state.
    #[must_use]
    pub fn state(&self) -> VaultState {
        self.state
    }

    /// The unlocked kernel — `Some` only while [`Unlocked`](VaultState::Unlocked).
    #[must_use]
    pub fn kernel(&self) -> Option<&Kernel> {
        self.kernel.as_ref()
    }

    /// Apply a happy-path transition, rejecting edges the state machine forbids.
    fn transition_to(&mut self, next: VaultState) -> Result<(), KernelError> {
        if !self.state.can_transition_to(next) {
            return Err(KernelError::IllegalVaultTransition {
                from: self.state,
                to: next,
            });
        }
        self.state = next;
        Ok(())
    }

    /// Fault-recovery: reset to ground truth by re-classifying the disk. Always
    /// lands in `NoVault` (clean), `Locked`, or `CorruptNeedsRecovery` — never a
    /// silently-wrong state.
    fn reclassify(&mut self) {
        self.state = classify_vault(&self.path);
    }

    /// Create a brand-new vault and leave it unlocked. Requires `NoVault`.
    ///
    /// # Errors
    /// [`KernelError::IllegalVaultTransition`] if not in `NoVault`, or any
    /// [`Kernel::create_vault`] error (on which the controller re-classifies).
    pub fn create(&mut self, password: &[u8]) -> Result<(), KernelError> {
        self.transition_to(VaultState::CreatingVault)?;
        match Kernel::create_vault(&self.path, password) {
            Ok(kernel) => {
                self.transition_to(VaultState::Unlocked)?;
                self.kernel = Some(kernel);
                Ok(())
            }
            Err(error) => {
                self.reclassify();
                Err(error)
            }
        }
    }

    /// Unlock the vault with `password`. Requires `Locked`.
    ///
    /// On a wrong password the controller returns to `Locked` (never `Unlocked`);
    /// a corrupt/missing envelope lands in `CorruptNeedsRecovery`.
    ///
    /// # Errors
    /// [`KernelError::IllegalVaultTransition`] if not in `Locked`, or any
    /// [`Kernel::unlock_vault`] error.
    pub fn unlock(&mut self, password: &[u8]) -> Result<(), KernelError> {
        self.transition_to(VaultState::Unlocking)?;
        match Kernel::unlock_vault(&self.path, password) {
            Ok(kernel) => {
                self.transition_to(VaultState::Unlocked)?;
                self.kernel = Some(kernel);
                Ok(())
            }
            Err(error) => {
                self.reclassify();
                Err(error)
            }
        }
    }

    /// Restore an encrypted backup into the (empty) vault location and end
    /// unlocked (ADR 0024 §5, personal-cfo-au3). Requires `NoVault` — the
    /// fresh-clone / new-machine case; the engine refuses to clobber an existing
    /// vault, so restore-over-existing is out of scope here.
    ///
    /// # Errors
    /// [`KernelError::IllegalVaultTransition`] if a vault already exists (not
    /// `NoVault`), or any [`Kernel::restore_backup`] error (wrong password, or a
    /// malformed / incompatible package); on failure the controller re-classifies.
    pub fn restore(&mut self, package_path: &Path, password: &[u8]) -> Result<(), KernelError> {
        self.transition_to(VaultState::RestoringBackup)?;
        match Kernel::restore_backup(package_path, password, &self.path) {
            Ok(kernel) => {
                self.transition_to(VaultState::Unlocked)?;
                self.kernel = Some(kernel);
                Ok(())
            }
            Err(error) => {
                self.reclassify();
                Err(error)
            }
        }
    }

    /// Lock the vault, dropping (and zeroizing) the in-memory DEK. Requires
    /// `Unlocked`.
    ///
    /// # Errors
    /// [`KernelError::IllegalVaultTransition`] if not in `Unlocked`.
    pub fn lock(&mut self) -> Result<(), KernelError> {
        self.transition_to(VaultState::Locking)?;
        self.kernel = None; // drop → DEK zeroized (personal-cfo-1t0)
        self.transition_to(VaultState::Locked)
    }

    /// Change the vault's master password (personal-cfo-zxq): verify `old`
    /// against the current envelope, then rewrap the **same DEK** under a KEK
    /// derived from `new` with a fresh salt at the current `InteractiveDefault`
    /// profile. The DEK — and therefore the SQLCipher database it keys — never
    /// changes, so only the envelope sidecar is rewritten (atomically, via
    /// `.tmp` + rename). Requires `Unlocked`: the user proves ownership by
    /// having the vault open, and re-proves knowledge of the current password
    /// here. The vault stays `Unlocked` throughout — the in-memory DEK is
    /// untouched on success and failure alike.
    ///
    /// # Errors
    /// - [`KernelError::IllegalVaultTransition`] if the vault is not `Unlocked`.
    /// - [`KernelError::VaultUnlockFailed`] if `old` is wrong (nothing written).
    /// - [`KernelError::Vault`] on an envelope or sidecar-I/O failure (the old
    ///   envelope stays valid — only the final rename installs the new one).
    pub fn change_password(&mut self, old: &[u8], new: &[u8]) -> Result<(), KernelError> {
        // The rewrap borrows the Rekeying edge (Unlocked → Rekeying → Unlocked,
        // §6.2.1): entering it enforces the Unlocked precondition, and both the
        // success and failure paths legally return to Unlocked — a failed
        // rewrap never modified the sidecar, and the kernel was never dropped.
        self.transition_to(VaultState::Rekeying)?;
        let result = rewrap_sidecar(&self.path, old, new);
        self.transition_to(VaultState::Unlocked)?;
        result
    }

    /// Permanently delete the vault (personal-cfo-j0cg.5): drop the in-memory kernel (zeroizing the
    /// DEK) and remove the vault's files from disk — the DB, its `.envelope` sidecar (the unlock
    /// material), the SQLite WAL/SHM, and the blob store — leaving the controller in `NoVault`.
    /// Requires the vault to be `Unlocked`: the user proves ownership by having it open before
    /// wiping it. Irreversible except by restoring an encrypted backup (ADR 0024).
    ///
    /// # Errors
    /// [`KernelError::IllegalVaultTransition`] if the vault is not `Unlocked`; [`KernelError::Vault`]
    /// if removing the envelope or the DB fails (the encrypted DB is unrecoverable without the
    /// envelope, so both are required; the WAL/SHM and blob store are best-effort cleanup).
    pub fn delete(&mut self) -> Result<(), KernelError> {
        if self.state != VaultState::Unlocked {
            return Err(KernelError::IllegalVaultTransition {
                from: self.state,
                to: VaultState::NoVault,
            });
        }
        // Drop the kernel first so the DB connection is released before its file is removed.
        self.kernel = None;

        let db = self.path.clone();
        let with_suffix = |suffix: &str| -> PathBuf {
            let mut os = db.clone().into_os_string();
            os.push(suffix);
            PathBuf::from(os)
        };
        let blobs = db.parent().unwrap_or_else(|| Path::new(".")).join("blobs");

        // Ancillary files: best-effort (their absence is fine). `-wal`/`-shm` back the WAL journal
        // mode; `-journal` covers a rollback-journal mode the DB may briefly use.
        let _ = std::fs::remove_file(with_suffix("-wal"));
        let _ = std::fs::remove_file(with_suffix("-shm"));
        let _ = std::fs::remove_file(with_suffix("-journal"));
        let _ = std::fs::remove_dir_all(&blobs);

        // The envelope (the unlock material) and the DB are what make the vault readable/
        // recoverable, so removing either is a real failure to surface. Whatever happens, re-derive
        // the state from disk afterwards so the controller never lingers in `Unlocked` with no
        // kernel: a clean wipe lands in `NoVault`, a partial failure in `CorruptNeedsRecovery`.
        let removed = remove_if_present(&sidecar_path(&db)).and_then(|()| remove_if_present(&db));
        self.reclassify();
        removed
    }

    /// Point the controller at a different vault (personal-cfo-j0cg.6, ADR 0042 §4). Locks the
    /// current vault first — dropping and zeroizing its DEK, so **keys never cross vaults** — then
    /// re-points at `path` and re-classifies it (`NoVault` / `Locked` / `CorruptNeedsRecovery`). The
    /// caller unlocks (or creates) the target with its own password.
    ///
    /// # Errors
    /// [`KernelError::IllegalVaultTransition`] if the current vault can't be locked from its state.
    pub fn switch_to(&mut self, path: PathBuf) -> Result<(), KernelError> {
        // Lock the current vault first so its DEK is dropped before we point elsewhere.
        if self.state == VaultState::Unlocked {
            self.lock()?;
        }
        // No kernel from the previous vault may survive the switch.
        self.kernel = None;
        self.path = path;
        self.reclassify();
        Ok(())
    }

    /// Run the post-unlock health check. Requires the vault to be unlocked.
    ///
    /// # Errors
    /// [`KernelError::Vault`] if the vault is not currently unlocked.
    pub fn health_check(&self) -> Result<VaultHealth, KernelError> {
        let kernel = self.kernel.as_ref().ok_or_else(|| {
            KernelError::Vault("health check requires an unlocked vault".to_owned())
        })?;
        // `kernel.worker` is reachable from this child module; the worker's own
        // typed read methods do the work so no database types escape the kernel.
        let worker = &kernel.worker;
        let schema_coherent = worker
            .vault_metadata()
            .map(|metadata| metadata.schema_version == worker.schema_version())
            .unwrap_or(false);
        Ok(VaultHealth {
            writer_healthy: matches!(worker.state(), WorkerState::Healthy),
            wal_configured: worker.self_test().is_ok(),
            schema_coherent,
            integrity_ok: worker.integrity_ok(),
            attachments_consistent: worker.attachments_consistent().unwrap_or(false),
            read_models_current: worker.read_models_current().unwrap_or(false),
        })
    }
}
