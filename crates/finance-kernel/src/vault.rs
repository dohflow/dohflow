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
//! (`-ef3` / `-au3`), envelope v1→v2, and KDF calibration (`-0sqk`). Key
//! rotation (`-2y8`, ADR 0083) is driven from here; its crash-safe file
//! protocol lives in `crate::rekey`.

use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use db_worker::{DbWorker, IntegrityOutcome, WorkerState};
use subtle::ConstantTimeEq;
use vault_crypto::{
    dek_fingerprint, derive_kek, generate_dek, generate_salt, rewrap_envelope, unwrap_dek,
    wrap_dek, Profile, ReleasedParseError, ReleasedUnlockMaterial, SecretBytes, VaultEnvelope,
    RELEASED_LEN,
};

use crate::rekey::{self, CrashPoint, RekeyPaths, RekeyRecovery};
use crate::{Kernel, KernelError, ReleasedUnlockError};

/// Host-chosen options for the unlock calls ([`Kernel::unlock_vault_with_options`],
/// [`Kernel::unlock_vault_with_dek_options`]). Generic: the kernel does not know why a host sets
/// one (personal-cfo-7ie.14; ADR 0085).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct UnlockOptions {
    /// Run `PRAGMA integrity_check` before anything opens, migrates, stamps or writes the
    /// vault; problems are [`KernelError::IntegrityCheckFailed`]. A host sets this when it has
    /// reason to doubt the file, for example after it detects a restored install. It reads the
    /// whole database, so it is opt-in.
    pub integrity_check: bool,
}

impl UnlockOptions {
    /// Options with the pre-unlock integrity check on.
    #[must_use]
    pub const fn with_integrity_check() -> Self {
        Self {
            integrity_check: true,
        }
    }
}

/// A single-use proof that a password was just checked against this vault's key envelope
/// (personal-cfo-xuu; 03b's M2 on the Touch ID design). [`Kernel::released_unlock_material`] exports the
/// live DEK as bytes for the platform store, so it cannot be called without one: the type has no public
/// constructor, is not `Clone` or `Copy`, and the export consumes it.
///
/// The proof is bound to a vault only through that fingerprint: it names no path and no vault id, so it is
/// meaningful only to the kernel that issued it, and the export re-checks the envelope it reads.
///
/// It carries only a fingerprint of the wrapped DEK that was verified, so the export can refuse when the
/// envelope has changed since (a password change or a key rotation between the check and the export).
#[derive(Debug)]
#[must_use = "a password proof is only useful when handed to released_unlock_material"]
pub struct PasswordVerified {
    fingerprint: [u8; 32],
}

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
    /// - [`KernelError::NewerVaultSchema`] or [`KernelError::UnsupportedVaultSchema`]
    ///   before migrations/seed writes when existing schema inspection refuses.
    pub fn unlock_vault(path: impl AsRef<Path>, password: &[u8]) -> Result<Self, KernelError> {
        Self::unlock_vault_with_options(path, password, UnlockOptions::default())
    }

    /// [`Kernel::unlock_vault`] with host-chosen [`UnlockOptions`]. With
    /// `integrity_check`, `PRAGMA integrity_check` runs first, before the worker is opened,
    /// migrated, stamped or written, and a failure is [`KernelError::IntegrityCheckFailed`].
    ///
    /// # Errors
    /// As [`Kernel::unlock_vault`], plus [`KernelError::IntegrityCheckFailed`].
    pub fn unlock_vault_with_options(
        path: impl AsRef<Path>,
        password: &[u8],
        options: UnlockOptions,
    ) -> Result<Self, KernelError> {
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
        if options.integrity_check {
            // A wrong key is not corruption: only reported problems stop the unlock, and a
            // key that does not fit falls through to the open below, which reports it as it
            // always has.
            if let IntegrityOutcome::Damaged { problems } =
                DbWorker::integrity_check_with_raw_key(db_path, &dek)?
            {
                return Err(KernelError::IntegrityCheckFailed { problems });
            }
        }
        let worker = DbWorker::open_existing_with_raw_key(db_path, dek)?;
        Ok(Self::with_worker(worker))
    }

    /// Unlock the existing vault at `path` from a DEK released by the platform's biometric
    /// store (ADR 0002 decision 6; personal-cfo-7ie.10).
    ///
    /// The platform layer (Rust, never the WebView) decrypts its wrapped item into a
    /// zeroizing [`SecretBytes`] and calls this. The DEK crosses no IPC boundary: no Tauri
    /// command accepts or returns it. The released plaintext (format in
    /// `vault_crypto::released`) carries a version, the vault's `vault_id` and a
    /// fingerprint of the wrapped DEK. Before the worker is opened, migrated or written:
    ///
    /// 1. the version is checked;
    /// 2. the fingerprint is compared, in constant time, with the fingerprint of the
    ///    wrapped DEK in the vault's envelope sidecar, so a password change or a key
    ///    rotation (both change the wrapped DEK) retires every older item;
    /// 3. a read-only handle is keyed with the DEK and the `vault_id` read from the
    ///    database is compared with the item's.
    ///
    /// Only then is the worker opened. A wrong key is a typed
    /// [`ReleasedUnlockError::KeyRejected`], never a corrupt-vault state, and it does not
    /// ask the caller to delete the item (the item matched the envelope).
    ///
    /// Call this only after the vault controller has opened the vault and settled any
    /// interrupted key rotation (it recovers in `VaultController::open`); a rotation journal
    /// on disk is refused with [`ReleasedUnlockError::RotationInProgress`].
    ///
    /// # Errors
    /// - [`KernelError::VaultNotFound`] if there is no envelope at `path`.
    /// - [`KernelError::ReleasedUnlock`] with the cause: unknown version, stale
    ///   fingerprint, wrong vault, or a key that does not open the database. In each case the
    ///   caller deletes the platform item and asks for the password.
    /// - [`KernelError::Vault`], [`KernelError::Persistence`], [`KernelError::VaultInUse`],
    ///   [`KernelError::NewerVaultSchema`], [`KernelError::UnsupportedVaultSchema`] as for
    ///   [`Kernel::unlock_vault`].
    pub fn unlock_vault_with_dek(
        path: impl AsRef<Path>,
        released: SecretBytes<RELEASED_LEN>,
    ) -> Result<Self, KernelError> {
        Self::unlock_vault_with_dek_options(path, released, UnlockOptions::default())
    }

    /// [`Kernel::unlock_vault_with_dek`] with host-chosen [`UnlockOptions`]. With
    /// `integrity_check`, the check runs after the released material has been accepted (the key
    /// opens the file and the vault is the right one) and before the worker is opened,
    /// migrated, stamped or written.
    ///
    /// # Errors
    /// As [`Kernel::unlock_vault_with_dek`], plus [`KernelError::IntegrityCheckFailed`].
    pub fn unlock_vault_with_dek_options(
        path: impl AsRef<Path>,
        released: SecretBytes<RELEASED_LEN>,
        options: UnlockOptions,
    ) -> Result<Self, KernelError> {
        let db_path = path.as_ref();
        // A rotation journal means the database and envelope may be a half-rotated pair; the
        // vault controller recovers it when it opens. Never probe that pair: the item would
        // look stale or wrong and be deleted for a fault that is not its own.
        if rekey::journal_present(db_path) {
            return Err(KernelError::ReleasedUnlock(
                ReleasedUnlockError::RotationInProgress,
            ));
        }
        let sidecar = sidecar_path(db_path);
        let bytes = match std::fs::read(&sidecar) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Err(KernelError::VaultNotFound)
            }
            Err(error) => return Err(KernelError::Vault(error.to_string())),
        };
        let envelope = VaultEnvelope::from_bytes(&bytes)?;

        let material = ReleasedUnlockMaterial::parse(&released).map_err(|error| match error {
            ReleasedParseError::UnknownVersion(found) => {
                KernelError::ReleasedUnlock(ReleasedUnlockError::UnknownVersion(found))
            }
            _ => KernelError::ReleasedUnlock(ReleasedUnlockError::Unreadable),
        })?;
        drop(released);

        let current = dek_fingerprint(&envelope.wrapped);
        if !bool::from(material.fingerprint.ct_eq(&current)) {
            return Err(KernelError::ReleasedUnlock(
                ReleasedUnlockError::StaleFingerprint,
            ));
        }

        match DbWorker::probe_raw_key(db_path, &material.dek)? {
            None => Err(KernelError::ReleasedUnlock(
                ReleasedUnlockError::KeyRejected,
            )),
            Some(stored) if stored.as_bytes() != &material.vault_id => {
                Err(KernelError::ReleasedUnlock(ReleasedUnlockError::WrongVault))
            }
            Some(_) => {
                if options.integrity_check {
                    if let IntegrityOutcome::Damaged { problems } =
                        DbWorker::integrity_check_with_raw_key(db_path, &material.dek)?
                    {
                        return Err(KernelError::IntegrityCheckFailed { problems });
                    }
                }
                Self::open_with_released_dek(db_path, material)
            }
        }
    }

    /// The step after a successful probe: take the runner lock (by opening the worker) and
    /// make sure the item still matches the envelope. The probe and this open each take the
    /// runner lock for themselves, so another process can rotate the key between them. Then
    /// either the open fails (the old key no longer fits; the worker's open reports a failed
    /// keyed read as an unsupported schema) or it succeeds against a vault the envelope no
    /// longer matches. Both are the same cause, an item that went stale in that window, so the
    /// envelope is re-read and a changed fingerprint is `StaleFingerprint`, never a corrupt or
    /// unsupported vault (ADR 0080: a wrong-key open never yields `CorruptNeedsRecovery`).
    fn open_with_released_dek(
        db_path: &Path,
        material: ReleasedUnlockMaterial,
    ) -> Result<Self, KernelError> {
        let sidecar = sidecar_path(db_path);
        let fingerprint = material.fingerprint;
        let still_matches = || -> Result<bool, KernelError> {
            let bytes =
                std::fs::read(&sidecar).map_err(|error| KernelError::Vault(error.to_string()))?;
            let envelope = VaultEnvelope::from_bytes(&bytes)?;
            Ok(bool::from(
                fingerprint.ct_eq(&dek_fingerprint(&envelope.wrapped)),
            ))
        };
        let stale = || KernelError::ReleasedUnlock(ReleasedUnlockError::StaleFingerprint);
        match DbWorker::open_existing_with_raw_key(db_path, material.dek) {
            Ok(worker) => {
                if still_matches()? {
                    Ok(Self::with_worker(worker))
                } else {
                    // Dropping the worker releases the lock and scrubs the key.
                    drop(worker);
                    Err(stale())
                }
            }
            // A failed open with a changed envelope is the race; with an unchanged envelope
            // it is a genuine persistence or schema error and is reported as such.
            Err(error) => match still_matches() {
                Ok(false) => Err(stale()),
                _ => Err(error.into()),
            },
        }
    }

    /// Check `password` against this vault's envelope, changing nothing (personal-cfo-xuu, AC C).
    ///
    /// Unwraps the DEK with a KEK derived from `password` using **the same envelope bytes the
    /// unlock fingerprint is computed from**, then compares the unwrapped key with the live key in
    /// constant time. A wrong password is [`KernelError::VaultUnlockFailed`]; an envelope that no
    /// longer wraps the key this vault is open with (another process changed it) is a typed error
    /// too. Either way nothing is enabled, written or unlocked a second time.
    ///
    /// The host calls this under its vault-controller lock, so no state change interleaves with the
    /// check and the export that follows it.
    ///
    /// # Errors
    /// [`KernelError::VaultUnlockFailed`], [`KernelError::Vault`] (a malformed or changed
    /// envelope), or [`KernelError::VaultNotFound`].
    pub fn verify_password(&self, password: &[u8]) -> Result<PasswordVerified, KernelError> {
        let envelope = self.read_envelope()?;
        let kek = derive_kek(password, &envelope.salt, &envelope.kdf)?;
        let unwrapped = unwrap_dek(&kek, &envelope.wrapped)?;
        let matches = self
            .worker
            .with_dek(|live| bool::from(live.expose_bytes().ct_eq(unwrapped.expose_bytes())))?;
        if !matches {
            // The password opened the envelope on disk, but that envelope wraps a different key than
            // the one this vault is open with: it changed behind our back. Enable nothing.
            return Err(KernelError::Vault(
                "the vault's key envelope no longer matches the open vault".to_owned(),
            ));
        }
        Ok(PasswordVerified {
            fingerprint: dek_fingerprint(&envelope.wrapped),
        })
    }

    /// The unlock material for the platform's biometric store: the released plaintext of
    /// `vault_crypto::released` built from this vault's live DEK, its `vault_id` and the fingerprint
    /// of the wrapped DEK on disk. Consumes the [`PasswordVerified`] proof.
    ///
    /// The envelope is read again and its fingerprint must equal the one the proof recorded: a
    /// password change or a rotation between the check and this call refuses the export.
    ///
    /// The result is a zeroizing buffer for the platform layer (Rust, never the WebView); it must
    /// never reach IPC (`dek_boundary.rs`).
    ///
    /// # Errors
    /// [`KernelError::Vault`] when the envelope changed since the proof, or on an I/O failure.
    pub fn released_unlock_material(
        &self,
        proof: PasswordVerified,
    ) -> Result<SecretBytes<RELEASED_LEN>, KernelError> {
        let envelope = self.read_envelope()?;
        if !bool::from(proof.fingerprint.ct_eq(&dek_fingerprint(&envelope.wrapped))) {
            return Err(KernelError::Vault(
                "the vault's key envelope changed since the password was checked".to_owned(),
            ));
        }
        let vault_id = *self.vault_metadata()?.vault_id.as_bytes();
        Ok(self
            .worker
            .with_dek(|dek| ReleasedUnlockMaterial::encode(vault_id, &envelope.wrapped, dek))?)
    }

    fn read_envelope(&self) -> Result<VaultEnvelope, KernelError> {
        let bytes = match std::fs::read(sidecar_path(self.worker.db_path())) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Err(KernelError::VaultNotFound)
            }
            Err(error) => return Err(KernelError::Vault(error.to_string())),
        };
        Ok(VaultEnvelope::from_bytes(&bytes)?)
    }

    /// Lock the vault by consuming the kernel: checkpoint the write-ahead log into the main
    /// file (`wal_checkpoint(TRUNCATE)`), then drop the db-worker, which closes its connection
    /// and zeroizes the raw key (the DEK) (ADR 0002, `personal-cfo-1t0`; personal-cfo-7ie.14).
    /// A system backup taken afterwards sees one consistent `vault.db` with no `-wal` content.
    ///
    /// The vault is locked whether or not the checkpoint succeeded; a host that wants to know
    /// uses [`Kernel::lock_checked`].
    pub fn lock(self) {
        let _ = self.lock_checked();
    }

    /// [`Kernel::lock`], reporting whether the checkpoint completed. The kernel is consumed,
    /// the connection closed and the key zeroized in every case; an `Err` only means the
    /// on-disk vault may still carry `-wal` content (a reader outside the kernel held the
    /// database open), so a host may retry its backup-sensitive step later.
    ///
    /// # Errors
    /// [`KernelError::Persistence`] with a generic message if the checkpoint did not complete.
    pub fn lock_checked(self) -> Result<(), KernelError> {
        let checkpointed = self.worker.checkpoint_truncate();
        // Drop the db-worker (its connection closes, its DEK is zeroized) before reporting.
        drop(self);
        checkpointed.map_err(KernelError::from)
    }

    /// Export an encrypted backup of this unlocked vault to `out_path` (ADR
    /// 0024-A). Bundles a consistent `vault.db` snapshot, the envelope sidecar,
    /// and every attachment blob into format v2 using the worker-owned DEK; no
    /// password is retained or requested. `created_at`, `backup_id`, and
    /// `app_version` are supplied by the caller (the kernel reads no clock).
    ///
    /// # Errors
    /// [`KernelError`] on a snapshot, file-read, crypto, or write failure.
    pub fn export_unattended(
        &self,
        out_path: &Path,
        app_version: &str,
        created_at: String,
        backup_id: uuid::Uuid,
    ) -> Result<(), KernelError> {
        let package = self.assemble_unattended_backup(app_version, created_at, backup_id)?;
        std::fs::write(out_path, &package)
            .map_err(|e| KernelError::Vault(format!("writing backup: {e}")))?;
        Ok(())
    }

    /// Export a newly scheduled backup without replacing any existing file.
    /// The exclusive create prevents a timestamp collision from overwriting a
    /// previous recovery point.
    pub fn export_unattended_new(
        &self,
        out_path: &Path,
        app_version: &str,
        created_at: String,
        backup_id: uuid::Uuid,
    ) -> Result<(), KernelError> {
        let package = self.assemble_unattended_backup(app_version, created_at, backup_id)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(out_path)
            .map_err(|_| KernelError::Vault("creating backup file failed".to_owned()))?;
        file.write_all(&package)
            .map_err(|_| KernelError::Vault("writing backup file failed".to_owned()))?;
        Ok(())
    }

    pub(crate) fn assemble_unattended_backup(
        &self,
        app_version: &str,
        created_at: String,
        backup_id: uuid::Uuid,
    ) -> Result<Vec<u8>, KernelError> {
        self.assemble_unattended_backup_cancellable(app_version, created_at, backup_id, &|| false)?
            .ok_or_else(|| KernelError::Vault("backup cancelled".to_owned()))
    }

    /// [`Kernel::assemble_unattended_backup`] that looks at `cancelled` between its steps (each
    /// blob read, the snapshot, each manifest and frame step, the seal) and returns `None`,
    /// having produced nothing, when it reads true (personal-cfo-g3m.29).
    pub(crate) fn assemble_unattended_backup_cancellable(
        &self,
        app_version: &str,
        created_at: String,
        backup_id: uuid::Uuid,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Option<Vec<u8>>, KernelError> {
        if cancelled() {
            return Ok(None);
        }
        let db_path = self.worker.db_path();
        let envelope_bytes = std::fs::read(sidecar_path(db_path))
            .map_err(|e| KernelError::Vault(format!("reading envelope: {e}")))?;
        let Some(blobs) = read_blobs_cancellable(&self.worker.blobs_dir(), cancelled)? else {
            return Ok(None);
        };
        if cancelled() {
            return Ok(None);
        }
        let db_bytes = self.worker.snapshot_bytes()?;
        if cancelled() {
            return Ok(None);
        }

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
        let package = self.worker.with_dek(|dek| {
            match crate::backup::assemble_cancellable(dek, &inputs, cancelled) {
                Ok(package) => Ok(Some(package)),
                Err(crate::backup::BackupError::Cancelled) => Ok(None),
                Err(error) => Err(KernelError::Vault(format!("assembling backup: {error}"))),
            }
        })??;
        Ok(package)
    }

    /// Re-open a format-v2 backup with this unlocked vault's in-memory DEK and
    /// verify every manifest component hash before the caller records success.
    pub fn verify_unattended_backup(
        &self,
        package_path: &Path,
        expected_backup_id: uuid::Uuid,
    ) -> Result<crate::backup::Manifest, KernelError> {
        let package = std::fs::read(package_path)
            .map_err(|_| KernelError::Vault("reading backup file failed".to_owned()))?;
        self.verify_unattended_backup_bytes(&package, expected_backup_id)
    }

    /// [`Kernel::verify_unattended_backup`] over the container's bytes, for a
    /// destination that is not a file (a [`crate::BackupSink`] returns bytes).
    pub fn verify_unattended_backup_bytes(
        &self,
        package: &[u8],
        expected_backup_id: uuid::Uuid,
    ) -> Result<crate::backup::Manifest, KernelError> {
        let restored = self.worker.with_dek(|dek| {
            crate::backup::disassemble_with_vault_dek(dek, package)
                .map_err(|_| KernelError::Vault("backup verification failed".to_owned()))
        })??;
        if restored.manifest.backup_id != expected_backup_id.to_string() {
            return Err(KernelError::Vault(
                "backup identity verification failed".to_owned(),
            ));
        }
        Ok(restored.manifest)
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
        //    wrong password fails while unwrapping the v1 backup key or v2 vault
        //    envelope — before any byte is written to disk.
        let package = std::fs::read(package_path)
            .map_err(|e| KernelError::Vault(format!("reading backup: {e}")))?;
        Self::restore_backup_bytes(&package, password, dest_db_path)
    }

    /// [`Kernel::restore_backup`] over the container's bytes, so a backup read back
    /// from a [`crate::BackupSink`] restores without a file path. The same
    /// verify-then-install rules apply.
    ///
    /// # Errors
    /// The same as [`Kernel::restore_backup`], except the read error.
    pub fn restore_backup_bytes(
        package: &[u8],
        password: &[u8],
        dest_db_path: &Path,
    ) -> Result<Self, KernelError> {
        let restored = crate::backup::disassemble(password, package).map_err(map_restore_err)?;

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
        let kernel = Self::unlock_vault(dest_db_path, password)?;
        // The restore's success is the first record of the restored vault's
        // session (personal-cfo-lyd). A failed restore has no session to record in.
        kernel.record_restore_success();
        Ok(kernel)
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

/// The vault's attachment blobs as `(storage id, ciphertext)`, ordered by storage id.
type BlobList = Vec<(String, Vec<u8>)>;

/// Read every attachment blob in `dir` as `(storage_id = filename, ciphertext)`
/// in a deterministic order. A missing directory (no attachments yet) is an
/// empty list; `.tmp` files (a crash-interrupted import) are skipped. Looks at
/// `cancelled` before each blob and returns `None` when it reads true.
fn read_blobs_cancellable(
    dir: &Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<Option<BlobList>, KernelError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Some(Vec::new())),
        Err(error) => return Err(KernelError::Vault(format!("reading blobs dir: {error}"))),
    };
    let mut blobs = Vec::new();
    for entry in entries {
        if cancelled() {
            return Ok(None);
        }
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
    Ok(Some(blobs))
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

/// Map a backup-parse failure to a kernel error. A wrong password fails an
/// envelope/key unwrap; surface it as [`KernelError::VaultUnlockFailed`],
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
                // Key rotation (ADR 0083 §4): after the commit point the kernel
                // is closed; if the rotated vault cannot be safely reopened in
                // the same call, the vault is left sealed for a normal unlock.
                | (Rekeying, Locked)
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
///
/// A key-rotation journal still on disk (ADR 0083 §4) also means
/// `CorruptNeedsRecovery`: [`VaultController::open`] runs rotation recovery
/// first, and a journal survives it only when the files contradict it.
/// Classification itself never modifies the disk.
#[must_use]
pub fn classify_vault(path: impl AsRef<Path>) -> VaultState {
    let db_path = path.as_ref();
    if rekey::journal_present(db_path) {
        return VaultState::CorruptNeedsRecovery;
    }
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
    /// What startup recovery did with an interrupted key rotation.
    rekey_recovery: RekeyRecovery,
}

impl VaultController {
    /// Inspect the vault on disk at `path` and enter the matching initial state
    /// (`NoVault` / `Locked` / `CorruptNeedsRecovery`).
    ///
    /// Before classifying, finishes or undoes a key rotation that a crash
    /// interrupted (ADR 0083 §4): an uncommitted one is rolled back, a committed
    /// one rolled forward; if the journal and the files disagree nothing is
    /// deleted and the vault classifies as `CorruptNeedsRecovery`.
    #[must_use]
    pub fn open(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref().to_path_buf();
        let rekey_recovery = rekey::recover_interrupted_rekey(&path);
        let state = classify_vault(&path);
        Self {
            path,
            state,
            kernel: None,
            rekey_recovery,
        }
    }

    /// What startup recovery did with an interrupted key rotation when this
    /// controller was opened.
    #[must_use]
    pub fn rekey_recovery(&self) -> RekeyRecovery {
        self.rekey_recovery
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

    /// [`Self::reclassify`] preceded by rotation recovery (ADR 0083 §4): used
    /// wherever the controller starts treating a vault path as current without
    /// going through [`Self::open`] — a switch, or a failed restore.
    fn recover_and_reclassify(&mut self) {
        self.rekey_recovery = rekey::recover_interrupted_rekey(&self.path);
        self.reclassify();
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
        self.unlock_with_options(password, UnlockOptions::default())
    }

    /// [`VaultController::unlock`] with the host's [`UnlockOptions`] (personal-cfo-i93n3.13):
    /// with `integrity_check`, `PRAGMA integrity_check` runs before anything opens, migrates
    /// or writes, and a failure is [`KernelError::IntegrityCheckFailed`] with the controller
    /// back in `Locked`, the file untouched.
    ///
    /// # Errors
    /// As [`VaultController::unlock`], plus [`KernelError::IntegrityCheckFailed`].
    pub fn unlock_with_options(
        &mut self,
        password: &[u8],
        options: UnlockOptions,
    ) -> Result<(), KernelError> {
        self.transition_to(VaultState::Unlocking)?;
        match Kernel::unlock_vault_with_options(&self.path, password, options) {
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

    /// Unlock with the material a platform biometric store released (personal-cfo-xuu): the
    /// controller edge of [`Kernel::unlock_vault_with_dek_options`]. Requires `Locked`. The host
    /// passes the same [`UnlockOptions`] it passes the password path (its per-vault integrity-check
    /// decision), and every failure returns the controller to its classified state with the file
    /// untouched, exactly as a wrong password does.
    ///
    /// # Errors
    /// [`KernelError::IllegalVaultTransition`] if not in `Locked`, or any
    /// [`Kernel::unlock_vault_with_dek_options`] error (including
    /// [`KernelError::ReleasedUnlock`] with the cause the caller's item policy branches on).
    pub fn unlock_with_released(
        &mut self,
        released: SecretBytes<RELEASED_LEN>,
        options: UnlockOptions,
    ) -> Result<(), KernelError> {
        self.transition_to(VaultState::Unlocking)?;
        match Kernel::unlock_vault_with_dek_options(&self.path, released, options) {
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

    /// The enrollment step for a platform biometric store: check `password` and, only if it is right,
    /// build the unlock material, both inside one `&self` call so the caller's controller lock spans
    /// the check and the export (personal-cfo-xuu, AC C and H). Requires `Unlocked`. A wrong
    /// password enables nothing and changes nothing.
    ///
    /// # Errors
    /// [`KernelError::IllegalVaultTransition`] if there is no unlocked kernel,
    /// [`KernelError::VaultUnlockFailed`] for a wrong password, or as
    /// [`Kernel::released_unlock_material`].
    pub fn enrollment_unlock_material(
        &self,
        password: &[u8],
    ) -> Result<SecretBytes<RELEASED_LEN>, KernelError> {
        let kernel = self
            .kernel
            .as_ref()
            .ok_or(KernelError::IllegalVaultTransition {
                from: self.state,
                to: VaultState::Unlocked,
            })?;
        let proof = kernel.verify_password(password)?;
        kernel.released_unlock_material(proof)
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
                self.recover_and_reclassify();
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
        // Checkpoint the WAL and close the connection; drop → DEK zeroized (personal-cfo-1t0,
        // personal-cfo-7ie.14). The vault ends `Locked` even if the checkpoint reports busy.
        let checkpointed = match self.kernel.take() {
            Some(kernel) => kernel.lock_checked(),
            None => Ok(()),
        };
        self.transition_to(VaultState::Locked)?;
        checkpointed
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

    /// Rotate the vault's DEK (ADR 0083): a new random DEK, re-wrapped under a
    /// KEK derived from `password` at the current `InteractiveDefault` profile
    /// with a fresh salt; the database re-encrypted and every attachment
    /// re-wrapped and re-addressed. The password does not change. Requires
    /// `Unlocked` and ends `Unlocked` on the rotated vault.
    ///
    /// Everything is prepared beside the live vault; one journal rename is the
    /// commit point. A failure before it leaves the vault unchanged and
    /// `Unlocked` on the old DEK. A failure after it (the kernel is already
    /// closed) finishes or reports the rotation through recovery and leaves the
    /// vault `Locked` (or `CorruptNeedsRecovery` if the files contradict the
    /// journal).
    ///
    /// # Errors
    /// - [`KernelError::IllegalVaultTransition`] if the vault is not `Unlocked`.
    /// - [`KernelError::VaultUnlockFailed`] if `password` is wrong (nothing written).
    /// - [`KernelError::InsufficientDiskSpace`] if the disk precheck fails.
    /// - [`KernelError::Vault`] / [`KernelError::Persistence`] on an I/O, crypto
    ///   or database failure.
    pub fn rotate_key(&mut self, password: &[u8]) -> Result<(), KernelError> {
        self.rotate_key_with(password, None)
    }

    /// [`Self::rotate_key`] with a test-only crash point: at `stop` the rotation
    /// returns immediately, leaving the files as a process death would.
    pub(crate) fn rotate_key_with(
        &mut self,
        password: &[u8],
        stop: Option<CrashPoint>,
    ) -> Result<(), KernelError> {
        self.transition_to(VaultState::Rekeying)?;
        let Some(kernel) = self.kernel.as_ref() else {
            self.state = VaultState::Unlocked;
            return Err(KernelError::Vault(
                "key rotation requires an unlocked vault".to_owned(),
            ));
        };
        if let Err(error) = rekey::prepare_and_commit(kernel, password, stop) {
            // Not committed: the live vault and its kernel are untouched.
            self.transition_to(VaultState::Unlocked)?;
            return Err(error);
        }
        // Committed. Close the kernel so no connection or runner lock remains
        // on the old database, then apply.
        self.kernel = None;
        if let Err(error) = rekey::finish_rotation(&self.path, stop) {
            if !rekey::is_simulated_crash(&error) {
                self.rekey_recovery = rekey::recover_interrupted_rekey(&self.path);
                self.state = classify_vault(&self.path);
            }
            return Err(error);
        }
        match Kernel::unlock_vault(&self.path, password) {
            Ok(kernel) => {
                self.transition_to(VaultState::Unlocked)?;
                self.kernel = Some(kernel);
                Ok(())
            }
            Err(error) => {
                self.transition_to(VaultState::Locked)?;
                Err(error)
            }
        }
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
        // Key-rotation artifacts (ADR 0083): best-effort like the side files.
        for artifact in RekeyPaths::new(&db).all_artifacts() {
            let _ = std::fs::remove_file(artifact);
        }

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
        self.recover_and_reclassify();
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

#[cfg(test)]
mod released_dek_race_tests {
    //! The probe-to-lock race of `unlock_vault_with_dek` (personal-cfo-7ie.13): another process
    //! can rotate the key after the read-only probe passed and before this process owns the
    //! runner lock. The post-probe step is tested directly, because the race itself cannot be
    //! scheduled from outside.

    use super::*;
    use tempfile::TempDir;
    use vault_crypto::{derive_kek, unwrap_dek};

    const PASSWORD: &[u8] = b"correct horse battery staple";

    fn material_for(db: &Path) -> ReleasedUnlockMaterial {
        let envelope =
            VaultEnvelope::from_bytes(&std::fs::read(sidecar_path(db)).unwrap()).unwrap();
        let kek = derive_kek(PASSWORD, &envelope.salt, &envelope.kdf).unwrap();
        let dek = unwrap_dek(&kek, &envelope.wrapped).unwrap();
        let sealed = ReleasedUnlockMaterial::encode([7; 16], &envelope.wrapped, &dek);
        ReleasedUnlockMaterial::parse(&sealed).unwrap()
    }

    fn created() -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("vault.db");
        Kernel::create_vault(&path, PASSWORD).unwrap().lock();
        (dir, path)
    }

    #[test]
    fn a_key_rotated_between_the_probe_and_the_lock_is_stale_not_corrupt() {
        let (_dir, path) = created();
        let old = material_for(&path);
        let mut controller = VaultController::open(&path);
        controller.unlock(PASSWORD).unwrap();
        controller.rotate_key(PASSWORD).unwrap();
        controller.lock().unwrap();

        // The probe had passed against the old key; the open now meets the rotated vault.
        match Kernel::open_with_released_dek(&path, old) {
            Err(KernelError::ReleasedUnlock(ReleasedUnlockError::StaleFingerprint)) => {}
            Err(other) => panic!("expected StaleFingerprint, got {other:?}"),
            Ok(_) => panic!("an old key must not open the rotated vault"),
        }
        // The vault is intact and unlocks with the password, so nothing marked it for recovery.
        assert_eq!(classify_vault(&path), VaultState::Locked);
        Kernel::unlock_vault(&path, PASSWORD)
            .expect("unlocks")
            .lock();
    }

    #[test]
    fn a_password_change_between_the_probe_and_the_lock_is_stale_too() {
        let (_dir, path) = created();
        let old = material_for(&path);
        let mut controller = VaultController::open(&path);
        controller.unlock(PASSWORD).unwrap();
        controller
            .change_password(PASSWORD, b"another password")
            .unwrap();
        controller.lock().unwrap();
        // The DEK is unchanged, so the open succeeds, and the re-check after it catches the
        // changed envelope.
        match Kernel::open_with_released_dek(&path, old) {
            Err(KernelError::ReleasedUnlock(ReleasedUnlockError::StaleFingerprint)) => {}
            Err(other) => panic!("expected StaleFingerprint, got {other:?}"),
            Ok(_) => panic!("a stale item must not open the vault"),
        }
    }

    #[test]
    fn an_unchanged_envelope_reports_a_genuine_open_failure_as_it_is() {
        let (_dir, path) = created();
        // A wrong key against an unchanged envelope is not the race: it is reported as the
        // open's own error, never relabeled as a stale item.
        let mut material = material_for(&path);
        let envelope =
            VaultEnvelope::from_bytes(&std::fs::read(sidecar_path(&path)).unwrap()).unwrap();
        material.fingerprint = dek_fingerprint(&envelope.wrapped);
        let wrong = vault_crypto::generate_dek().unwrap();
        let broken = ReleasedUnlockMaterial {
            vault_id: material.vault_id,
            fingerprint: material.fingerprint,
            dek: wrong,
        };
        match Kernel::open_with_released_dek(&path, broken) {
            Err(KernelError::ReleasedUnlock(ReleasedUnlockError::StaleFingerprint)) => {
                panic!("an unchanged envelope is not a stale item")
            }
            Err(_) => {}
            Ok(_) => panic!("a wrong key must not open the vault"),
        }
    }
}
