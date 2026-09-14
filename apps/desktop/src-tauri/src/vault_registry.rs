//! The multi-vault registry (personal-cfo-j0cg.6, ADR 0042).
//!
//! A plaintext `vaults.json` under the app-data root listing the known vaults — id, display name,
//! and DB path *relative to the root* — plus the active vault. It carries no secrets, so it is
//! readable before any vault is unlocked (what a launch picker needs). Only the Rust command layer
//! writes it (ADR 0003). Slice 1 establishes the registry + bootstraps the legacy single vault; it
//! makes no behavioural change (create-new / switch / picker land in later slices).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The legacy single-vault DB filename at the app-data root (registered in place, never moved).
const LEGACY_VAULT_DB: &str = "vault.db";

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
    /// Propagates an I/O failure writing the file.
    pub fn save(&self, root: &Path) -> std::io::Result<()> {
        let bytes = serde_json::to_vec_pretty(self).expect("registry always serializes");
        std::fs::write(Self::file(root), bytes)
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
}
