//! Tauri-managed application state (personal-cfo-40t, vault wiring in -8v2/-3ry).
//!
//! Holds the [`VaultController`] behind a mutex. The controller owns the vault
//! lifecycle state and the unlocked [`Kernel`](finance_kernel::Kernel) (present
//! only while `Unlocked`); the vault commands drive it and the account commands
//! read the kernel through it. When no vault is open, every account command
//! returns [`IpcError::VaultLocked`](crate::ipc::IpcError) — the correct steady
//! state of a locked app.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use finance_kernel::{JobDispatcher, JobHandler, JobSpecError, Kernel, VaultController};

use crate::ipc::IpcError;
use crate::vault_registry::VaultRegistry;

/// Application state managed by Tauri (`tauri::Builder::manage`).
pub struct AppState {
    /// The vault controller: lifecycle state + the unlocked kernel for the *active* vault.
    controller: Mutex<VaultController>,
    /// The multi-vault registry (personal-cfo-j0cg.6, ADR 0042): the known vaults + the active one.
    registry: Mutex<VaultRegistry>,
    /// The app-data root the registry + managed vaults live under — `Some` in the real app,
    /// `None` for single-vault tests that don't exercise the registry.
    vaults_root: Option<PathBuf>,
    /// Connector connections with a sync currently in flight
    /// (personal-cfo-gglk): claimed at sync start, released on every exit, so
    /// the unlock auto-sync, a manual sync, and rapid re-unlocks can never
    /// walk the network concurrently for one connection (the Bridge budget is
    /// ~24 requests/day, and the loser's whole batch would churn as skips).
    connector_syncs_in_flight: Mutex<std::collections::HashSet<uuid::Uuid>>,
    /// Process-local consumer registry. Consumers own their opt-in policy and
    /// domain work; the production state always installs the dispatcher so an
    /// unlock runs the durable scheduler even before a feature bead registers
    /// its first handler.
    job_dispatcher: Arc<JobDispatcher<Kernel>>,
}

impl AppState {
    /// Wrap a [`VaultController`] with an empty registry and no root. Used by tests that construct
    /// state around a single temp vault and don't exercise the registry.
    #[must_use]
    pub fn new(controller: VaultController) -> Self {
        Self {
            controller: Mutex::new(controller),
            registry: Mutex::new(VaultRegistry::default()),
            vaults_root: None,
            connector_syncs_in_flight: Mutex::new(std::collections::HashSet::new()),
            job_dispatcher: Arc::new(JobDispatcher::new()),
        }
    }

    /// Wrap a [`VaultController`] (already pointed at the active vault + classified) together with
    /// the loaded registry and the app-data root. This is what [`run`](crate::run) builds at launch.
    #[must_use]
    pub fn with_registry(
        controller: VaultController,
        registry: VaultRegistry,
        vaults_root: PathBuf,
    ) -> Self {
        Self {
            controller: Mutex::new(controller),
            registry: Mutex::new(registry),
            vaults_root: Some(vaults_root),
            connector_syncs_in_flight: Mutex::new(std::collections::HashSet::new()),
            job_dispatcher: Arc::new(JobDispatcher::new()),
        }
    }

    /// Lock the controller mutex, mapping a poisoned lock to a typed error
    /// rather than panicking inside a command.
    pub fn lock_controller(&self) -> Result<MutexGuard<'_, VaultController>, IpcError> {
        self.controller
            .lock()
            .map_err(|_| IpcError::Persistence("application state lock was poisoned".to_owned()))
    }

    /// Claim a connector sync slot for `connection_id`. `false` if a sync for
    /// that connection is already in flight; release with
    /// [`Self::release_connector_sync`] on every exit path.
    #[must_use]
    pub fn claim_connector_sync(&self, connection_id: uuid::Uuid) -> bool {
        self.connector_syncs_in_flight
            .lock()
            .map(|mut set| set.insert(connection_id))
            .unwrap_or(false)
    }

    /// Release a connector sync slot claimed by [`Self::claim_connector_sync`].
    pub fn release_connector_sync(&self, connection_id: uuid::Uuid) {
        if let Ok(mut set) = self.connector_syncs_in_flight.lock() {
            set.remove(&connection_id);
        }
    }

    /// Snapshot the production dispatcher for an unlock-spawn or a consumer
    /// registration. The `Arc` lets a post-unlock task outlive the command.
    #[must_use]
    pub fn job_dispatcher(&self) -> Arc<JobDispatcher<Kernel>> {
        Arc::clone(&self.job_dispatcher)
    }

    /// Register or replace one process-local durable-job consumer.
    pub fn register_job_handler(
        &self,
        handler: Arc<dyn JobHandler<Kernel>>,
    ) -> Result<(), JobSpecError> {
        self.job_dispatcher.register(handler)
    }

    /// Lock the registry mutex, mapping a poisoned lock to a typed error.
    pub fn lock_registry(&self) -> Result<MutexGuard<'_, VaultRegistry>, IpcError> {
        self.registry
            .lock()
            .map_err(|_| IpcError::Persistence("application state lock was poisoned".to_owned()))
    }

    /// The app-data root the registry + managed vaults live under, if configured (multi-vault mode).
    #[must_use]
    pub fn vaults_root(&self) -> Option<&std::path::Path> {
        self.vaults_root.as_deref()
    }
}
