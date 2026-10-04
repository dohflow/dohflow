//! The multi-vault registry (personal-cfo-j0cg.6, ADR 0042).
//!
//! A plaintext `vaults.json` under the app-data root listing the known vaults — id, display name,
//! and DB path *relative to the root* — plus the active vault. It carries no secrets, so it is
//! readable before any vault is unlocked (what a launch picker needs). Only the Rust command layer
//! writes it (ADR 0003). Slice 1 establishes the registry + bootstraps the legacy single vault; it
//! makes no behavioural change (create-new / switch / picker land in later slices).

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The legacy single-vault DB filename at the app-data root (registered in place, never moved).
const LEGACY_VAULT_DB: &str = "vault.db";
/// Marker left inside an app-managed restore slot until its registry entry is committed.
/// It contains only the attempt UUID, never a backup path, vault name, or key material.
pub(crate) const RESTORE_ATTEMPT_MARKER: &str = ".restore-in-progress";

/// One known vault.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultEntry {
    pub id: Uuid,
    pub name: String,
    /// The vault's DB path, relative to the app-data root (`vault.db` for the legacy vault,
    /// `vaults/<id>/vault.db` for app-managed ones).
    pub path: PathBuf,
    pub created_at: String,
}

/// The known vaults + which one is active.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VaultRegistry {
    #[serde(default)]
    pub vaults: Vec<VaultEntry>,
    #[serde(default)]
    pub active: Option<Uuid>,
}

impl VaultRegistry {
    /// The registry file under `root`.
    fn file(root: &Path) -> PathBuf {
        root.join("vaults.json")
    }

    /// Load the registry from `root`, or an empty one if it's absent, unreadable, or corrupt (the
    /// launch path re-bootstraps from disk, so a lost registry is recoverable, not fatal).
    #[must_use]
    pub fn load(root: &Path) -> Self {
        std::fs::read(Self::file(root))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// Persist the registry as pretty JSON under `root`.
    ///
    /// # Errors
    /// Propagates an I/O failure. The existing registry is replaced only after the complete new
    /// contents have been written and synced to a unique sibling file.
    pub fn save(&self, root: &Path) -> std::io::Result<()> {
        let bytes = serde_json::to_vec_pretty(self).expect("registry always serializes");
        write_atomically(&Self::file(root), &bytes, Uuid::now_v7())
    }

    /// The absolute DB path of the active vault, if one is set and present in the registry.
    #[must_use]
    pub fn active_path(&self, root: &Path) -> Option<PathBuf> {
        let active = self.active?;
        self.vaults
            .iter()
            .find(|v| v.id == active)
            .map(|v| root.join(&v.path))
    }

    /// Whether an unregistered app-managed vault directory is present. Restore slots are
    /// registered only after verification; an unregistered UUID directory therefore identifies
    /// an interrupted or failed attempt. It is deliberately left in place for diagnosis rather
    /// than being auto-opened or silently deleted at startup.
    pub fn has_unregistered_restore_attempt(&self, root: &Path) -> io::Result<bool> {
        let vaults_root = root.join("vaults");
        let entries = match fs::read_dir(&vaults_root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error),
        };

        for entry in entries {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }

            let slot = entry.path();
            let is_registered = self
                .vaults
                .iter()
                .any(|vault| root.join(&vault.path).parent() == Some(slot.as_path()));
            if is_registered {
                continue;
            }

            let is_uuid_slot = entry
                .file_name()
                .to_str()
                .is_some_and(|name| Uuid::parse_str(name).is_ok());
            if is_uuid_slot || slot.join(RESTORE_ATTEMPT_MARKER).exists() {
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Reserve a fresh app-managed slot without following an existing `vaults` symlink or
    /// accepting an occupied UUID directory. Callers must treat `AlreadyExists` as a collision,
    /// never as permission to reuse the directory.
    pub(crate) fn reserve_restore_slot(root: &Path, id: Uuid) -> io::Result<PathBuf> {
        let vaults_root = root.join("vaults");
        match fs::symlink_metadata(&vaults_root) {
            Ok(metadata) if metadata.file_type().is_dir() => {}
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "app-managed vault location is not a directory",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                fs::create_dir(&vaults_root)?;
            }
            Err(error) => return Err(error),
        }

        let slot = vaults_root.join(id.to_string());
        fs::create_dir(&slot)?;
        Ok(slot)
    }

    /// Persist an attempt marker before restore writes any vault files.
    pub(crate) fn mark_restore_attempt(slot: &Path, id: Uuid) -> io::Result<()> {
        let path = slot.join(RESTORE_ATTEMPT_MARKER);
        let mut marker = OpenOptions::new().write(true).create_new(true).open(path)?;
        marker.write_all(id.to_string().as_bytes())?;
        marker.sync_all()
    }

    /// Remove only the known restore outputs from a directory proven to belong to `id`. If the
    /// marker is missing or differs, leave the directory untouched for diagnosis.
    pub(crate) fn cleanup_restore_attempt(slot: &Path, id: Uuid) -> io::Result<()> {
        let marker = slot.join(RESTORE_ATTEMPT_MARKER);
        let marker_id = fs::read_to_string(&marker)?;
        if marker_id != id.to_string() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "restore attempt marker did not match the reserved slot",
            ));
        }

        for filename in [
            "vault.db",
            "vault.db.envelope",
            "vault.db-wal",
            "vault.db-shm",
            "vault.db-journal",
            "vault.db.runner.lock",
            // Key-rotation artifacts (ADR 0083), in case a rotation ever ran here.
            "vault.db.rekey",
            "vault.db.rekey.tmp",
            "vault.db.rekey-new",
            "vault.db.rekey-new-wal",
            "vault.db.rekey-new-shm",
            "vault.db.rekey-new-journal",
            "vault.db.envelope.rekey-new",
            "vault.db.rekey-old",
        ] {
            remove_if_present(&slot.join(filename))?;
        }
        remove_dir_if_present(&slot.join("blobs"))?;
        remove_if_present(&marker)?;
        fs::remove_dir(slot)
    }

    /// Remove a successfully registered attempt marker. A failure is harmless: the scanner
    /// ignores markers inside registered slots, and the verified vault remains active.
    pub(crate) fn clear_restore_marker(slot: &Path) -> io::Result<()> {
        remove_if_present(&slot.join(RESTORE_ATTEMPT_MARKER))
    }

    /// Ensure the registry reflects at least the pre-multi-vault single vault, and that an active
    /// vault is selected. If the registry is empty and a legacy `vault.db` exists at `root`, it's
    /// registered *in place* as the active vault (ADR 0042 §3 — real vault files are never moved).
    /// Returns the active vault's absolute DB path, or `None` on a truly fresh install (no vault
    /// yet), where the caller falls back to the legacy path so create-vault still works.
    pub fn bootstrap(&mut self, root: &Path) -> Option<PathBuf> {
        if self.vaults.is_empty() && root.join(LEGACY_VAULT_DB).exists() {
            let id = Uuid::now_v7();
            self.vaults.push(VaultEntry {
                id,
                name: "My vault".to_owned(),
                path: PathBuf::from(LEGACY_VAULT_DB),
                created_at: now_rfc3339(),
            });
            self.active = Some(id);
        }
        // A missing/stale active id falls back to the first known vault.
        if self.active.is_none() || self.active_path(root).is_none() {
            self.active = self.vaults.first().map(|v| v.id);
        }
        self.active_path(root)
    }
}

fn write_atomically(path: &Path, bytes: &[u8], temp_id: Uuid) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let temp = parent.join(format!(".vaults.json.{temp_id}.tmp"));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    let write_result = (|| {
        // Replacing the file must not broaden permissions a user or earlier install tightened.
        // A first registry write keeps the platform's normal create-file permissions.
        match fs::metadata(path) {
            Ok(metadata) => file.set_permissions(metadata.permissions())?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        file.write_all(bytes)?;
        file.sync_all()
    })();
    if let Err(error) = write_result {
        drop(file);
        let _ = fs::remove_file(&temp);
        return Err(error);
    }
    drop(file);
    if let Err(error) = fs::rename(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(error);
    }
    Ok(())
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn remove_dir_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// The current time as an RFC-3339 string (the registry lives in the app shell, which may read the
/// clock — unlike the kernel).
fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn load_is_empty_when_absent_and_round_trips_when_saved() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        assert!(VaultRegistry::load(root).vaults.is_empty());

        let id = Uuid::now_v7();
        let registry = VaultRegistry {
            vaults: vec![VaultEntry {
                id,
                name: "Test".to_owned(),
                path: PathBuf::from("vaults/x/vault.db"),
                created_at: "2026-07-03T00:00:00Z".to_owned(),
            }],
            active: Some(id),
        };
        registry.save(root).unwrap();

        let loaded = VaultRegistry::load(root);
        assert_eq!(loaded.vaults, registry.vaults);
        assert_eq!(loaded.active, Some(id));
        assert_eq!(
            loaded.active_path(root),
            Some(root.join("vaults/x/vault.db"))
        );
    }

    #[test]
    fn bootstrap_registers_a_legacy_vault_in_place_and_activates_it() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join(LEGACY_VAULT_DB), b"db bytes").unwrap();

        let mut registry = VaultRegistry::default();
        let active = registry.bootstrap(root).unwrap();

        assert_eq!(
            active,
            root.join(LEGACY_VAULT_DB),
            "legacy vault stays in place"
        );
        assert_eq!(registry.vaults.len(), 1);
        assert_eq!(registry.vaults[0].path, PathBuf::from(LEGACY_VAULT_DB));
        assert_eq!(registry.active, Some(registry.vaults[0].id));
    }

    #[test]
    fn bootstrap_on_a_fresh_install_registers_nothing() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let mut registry = VaultRegistry::default();
        assert_eq!(registry.bootstrap(root), None);
        assert!(registry.vaults.is_empty());
        assert_eq!(registry.active, None);
    }

    #[test]
    fn bootstrap_repairs_a_stale_active_id() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let real = Uuid::now_v7();
        let mut registry = VaultRegistry {
            vaults: vec![VaultEntry {
                id: real,
                name: "A".to_owned(),
                path: PathBuf::from("vaults/a/vault.db"),
                created_at: "2026-07-03T00:00:00Z".to_owned(),
            }],
            active: Some(Uuid::now_v7()), // points at a vault that isn't in the list
        };
        registry.bootstrap(root);
        assert_eq!(
            registry.active,
            Some(real),
            "falls back to the first known vault"
        );
    }

    #[test]
    fn an_unregistered_uuid_slot_is_reported_but_never_removed() {
        let dir = TempDir::new().unwrap();
        let id = Uuid::now_v7();
        let slot = VaultRegistry::reserve_restore_slot(dir.path(), id).unwrap();
        VaultRegistry::mark_restore_attempt(&slot, id).unwrap();

        let loaded = VaultRegistry::load(dir.path());
        assert!(loaded.has_unregistered_restore_attempt(dir.path()).unwrap());
        assert!(slot.join(RESTORE_ATTEMPT_MARKER).exists());
        assert!(
            loaded.vaults.is_empty(),
            "an interrupted slot is not registered"
        );
    }

    #[test]
    fn a_registered_slot_with_a_stale_marker_is_not_reported_as_interrupted() {
        let dir = TempDir::new().unwrap();
        let id = Uuid::now_v7();
        let slot = VaultRegistry::reserve_restore_slot(dir.path(), id).unwrap();
        VaultRegistry::mark_restore_attempt(&slot, id).unwrap();
        let registry = VaultRegistry {
            vaults: vec![VaultEntry {
                id,
                name: "Restored".to_owned(),
                path: PathBuf::from("vaults")
                    .join(id.to_string())
                    .join("vault.db"),
                created_at: "2026-09-27T00:00:00Z".to_owned(),
            }],
            active: Some(id),
        };

        assert!(!registry
            .has_unregistered_restore_attempt(dir.path())
            .unwrap());
        assert!(slot.join(RESTORE_ATTEMPT_MARKER).exists());
    }

    #[test]
    fn restore_slot_reservation_refuses_to_reuse_an_existing_directory() {
        let dir = TempDir::new().unwrap();
        let id = Uuid::now_v7();
        let slot = VaultRegistry::reserve_restore_slot(dir.path(), id).unwrap();
        fs::write(slot.join("sentinel"), b"leave me alone").unwrap();

        let error = VaultRegistry::reserve_restore_slot(dir.path(), id).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(slot.join("sentinel")).unwrap(), b"leave me alone");
    }

    #[test]
    fn atomic_registry_write_failure_does_not_replace_existing_bytes() {
        let dir = TempDir::new().unwrap();
        let path = VaultRegistry::file(dir.path());
        fs::write(&path, b"previous registry bytes").unwrap();
        let temp_id = Uuid::now_v7();
        let occupied_temp = dir.path().join(format!(".vaults.json.{temp_id}.tmp"));
        fs::create_dir(&occupied_temp).unwrap();

        assert!(write_atomically(&path, b"new registry bytes", temp_id).is_err());
        assert_eq!(fs::read(path).unwrap(), b"previous registry bytes");
        assert!(occupied_temp.is_dir(), "a temp collision is not removed");
    }

    #[cfg(unix)]
    #[test]
    fn atomic_registry_write_preserves_existing_file_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().unwrap();
        let path = VaultRegistry::file(dir.path());
        fs::write(&path, b"{}").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

        VaultRegistry::default().save(dir.path()).unwrap();
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
