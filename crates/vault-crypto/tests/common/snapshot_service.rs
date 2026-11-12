//! An in-memory [`SnapshotService`] for tests. It runs the server's decision procedure as
//! `sync-service-protocol.md` 4.4 and 4.9 write it (and nothing the spec does not say), so a test
//! exercises the real refusals: the head compare-and-swap, the epoch rule, the closure receipts,
//! the idempotent re-publish, `VaultExists`, and the credential the stage needs once a genesis is
//! installed. One fake, not a second sequencer.
//!
//! Scripted faults and hooks let a test lose a response (the request takes effect, the client sees
//! `Unavailable`), advance the head under a publisher, halt a device and record every call.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::Mutex;

use sync_core::artifact::{ArtifactId, ClosureManifest, MAX_CLOSURE_SIDECAR_LEN};
use sync_core::envelope::binding_commitment;
use sync_core::header::{parse_container, ObjectKind, OBJECT_SNAPSHOT};
use sync_core::membership::verify_chain;
use sync_core::snapshot_service::{
    EpochInstall, InstalledEpoch, PublishedSnapshot, ServiceError, SnapshotService, StagedClosure,
    StagedSnapshot, MAX_INLINE_CLOSURE,
};

/// The credential the calls are made with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Credential {
    /// The account, which is all a device has before the vault's genesis is installed.
    Account,
    /// An enrolled device.
    Device,
}

/// A scripted fault for the next call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fault {
    /// The call runs as it would with no fault: a placeholder that lets a later fault land on a
    /// later call.
    Pass,
    /// The call is refused before it does anything.
    Refuse(ServiceError),
    /// The call takes full effect and the client is told `Unavailable` (a lost response).
    LoseResponse,
}

struct Vault {
    genesis_membership: Vec<u8>,
    genesis_epoch: Vec<u8>,
    installed: InstalledEpoch,
    key_epoch: u64,
    head: u64,
    halted_at: Option<u64>,
}

#[derive(Default)]
struct State {
    vault: Option<Vault>,
    staged: HashMap<[u8; 16], Vec<u8>>,
    published: Vec<PublishedSnapshot>,
    /// The durable receipts: each artifact id with the SHA-256 of the ciphertext the service holds.
    receipts: BTreeMap<ArtifactId, [u8; 32]>,
    pins: BTreeSet<ArtifactId>,
    /// The closure sidecar held with each snapshot, as its ids' raw bytes: staged with
    /// `stageSnapshotClosure`, or kept from the request's list when the snapshot was published.
    closures: HashMap<[u8; 16], Vec<u8>>,
    /// The closure list each `publishSnapshot` and `installEpoch` request carried, by operation.
    requests: Vec<(&'static str, Vec<ArtifactId>)>,
    faults: VecDeque<Fault>,
    calls: Vec<&'static str>,
}

/// The operation a hook waits for, and what it runs.
type Hook = (&'static str, Box<dyn FnOnce() + Send>);

pub struct FakeSnapshotService {
    credential: Mutex<Credential>,
    state: Mutex<State>,
    /// Runs once, just before the named operation does anything: what happens on the client's
    /// device while a request is in flight.
    hook: Mutex<Option<Hook>>,
}

impl FakeSnapshotService {
    pub fn new(credential: Credential) -> Self {
        Self {
            credential: Mutex::new(credential),
            state: Mutex::new(State::default()),
            hook: Mutex::new(None),
        }
    }

    /// Run `action` once, just before the next call of `operation` does anything.
    pub fn before(&self, operation: &'static str, action: impl FnOnce() + Send + 'static) {
        *self.hook.lock().unwrap() = Some((operation, Box::new(action)));
    }

    pub fn set_credential(&self, credential: Credential) {
        *self.credential.lock().unwrap() = credential;
    }

    /// Queue a fault for a future call, in order.
    pub fn script(&self, fault: Fault) {
        self.state.lock().unwrap().faults.push_back(fault);
    }

    /// The names of the operations called, in order.
    pub fn calls(&self) -> Vec<&'static str> {
        self.state.lock().unwrap().calls.clone()
    }

    /// Another device advanced the log (what a push does).
    pub fn advance_head_to(&self, head: u64) {
        self.state.lock().unwrap().vault.as_mut().unwrap().head = head;
    }

    /// This device reported a halt (protocol 4.12).
    pub fn halt_device(&self, stopped_at: u64) {
        self.state.lock().unwrap().vault.as_mut().unwrap().halted_at = Some(stopped_at);
    }

    /// The service already knows this vault, with a genesis of its own (what another device's
    /// sync-enable leaves behind).
    pub fn seed_vault(&self, membership: Vec<u8>, epoch: Vec<u8>) {
        self.state.lock().unwrap().vault = Some(Vault {
            genesis_membership: membership,
            genesis_epoch: epoch,
            installed: InstalledEpoch {
                key_epoch: 1,
                ordinal: 1,
                position: 0,
                membership_head: [0; 32],
                epoch_record_id: [0; 32],
                already_installed: false,
            },
            key_epoch: 1,
            head: 0,
            halted_at: None,
        });
    }

    /// A client put this artifact (it has a durable receipt).
    pub fn receipt(&self, id: ArtifactId) {
        // The hash a test binds by default is `common::bound`'s: derived from the id.
        self.receipt_with_hash(id, [id.as_bytes()[0] ^ 0xff; 32]);
    }

    /// As [`Self::receipt`], with the ciphertext hash the service holds for the artifact.
    pub fn receipt_with_hash(&self, id: ArtifactId, ciphertext_sha256: [u8; 32]) {
        self.state
            .lock()
            .unwrap()
            .receipts
            .insert(id, ciphertext_sha256);
    }

    pub fn is_pinned(&self, id: &ArtifactId) -> bool {
        self.state.lock().unwrap().pins.contains(id)
    }

    pub fn head(&self) -> Option<u64> {
        self.state.lock().unwrap().vault.as_ref().map(|v| v.head)
    }

    pub fn knows_vault(&self) -> bool {
        self.state.lock().unwrap().vault.is_some()
    }

    pub fn published(&self) -> Vec<PublishedSnapshot> {
        self.state.lock().unwrap().published.clone()
    }

    /// The closure sidecar the service holds for `snapshot_id`, as the ids' raw bytes.
    pub fn closure_of(&self, snapshot_id: &[u8; 16]) -> Option<Vec<u8>> {
        self.state
            .lock()
            .unwrap()
            .closures
            .get(snapshot_id)
            .cloned()
    }

    /// The closure list the last request of `operation` (`publish_snapshot` or `install_epoch`)
    /// carried, if there was one.
    pub fn manifest_sent(&self, operation: &str) -> Option<Vec<ArtifactId>> {
        self.state
            .lock()
            .unwrap()
            .requests
            .iter()
            .rev()
            .find(|(name, _)| *name == operation)
            .map(|(_, ids)| ids.clone())
    }

    pub fn is_staged(&self, snapshot_id: &[u8; 16]) -> bool {
        self.state.lock().unwrap().staged.contains_key(snapshot_id)
    }

    /// Run `op` against the state, applying a scripted fault around it.
    fn call<R>(
        &self,
        name: &'static str,
        op: impl FnOnce(&mut State, Credential) -> Result<R, ServiceError>,
    ) -> Result<R, ServiceError> {
        let credential = *self.credential.lock().unwrap();
        let hook = {
            let mut slot = self.hook.lock().unwrap();
            if slot
                .as_ref()
                .is_some_and(|(operation, _)| *operation == name)
            {
                slot.take()
            } else {
                None
            }
        };
        if let Some((_, action)) = hook {
            action();
        }
        let mut state = self.state.lock().unwrap();
        state.calls.push(name);
        match state.faults.pop_front() {
            Some(Fault::Pass) | None => op(&mut state, credential),
            Some(Fault::Refuse(error)) => Err(error),
            Some(Fault::LoseResponse) => {
                let _ = op(&mut state, credential);
                Err(ServiceError::Unavailable)
            }
        }
    }
}

fn halt_gate(state: &State, credential: Credential) -> Result<(), ServiceError> {
    // Protocol 4.12: a device credential whose device has a recorded halt is refused; an account
    // credential (before the genesis) has no device and passes.
    match (&state.vault, credential) {
        (Some(vault), Credential::Device) if vault.halted_at.is_some() => {
            Err(ServiceError::ReplicaHalted {
                stopped_at: vault.halted_at.unwrap(),
            })
        }
        _ => Ok(()),
    }
}

impl SnapshotService for FakeSnapshotService {
    fn stage_snapshot(
        &self,
        vault_id: &[u8; 16],
        snapshot_id: &[u8; 16],
        container: &[u8],
    ) -> Result<StagedSnapshot, ServiceError> {
        self.call("stage_snapshot", |state, credential| {
            // The order of protocol 4.9: the checks that need no vault state (the container and the ids its
            // header names), then the authorization that needs the vault (`Forbidden`), then the halt gate.
            let parsed = parse_container(container).map_err(|_| ServiceError::MalformedObject)?;
            let header = &parsed.header;
            if &header.vault_id != vault_id
                || header.object_type() != OBJECT_SNAPSHOT
                || &header.object_id != snapshot_id
            {
                return Err(ServiceError::MalformedObject);
            }
            // Once a genesis is installed the account credential is not a device (protocol 2.4).
            if state.vault.is_some() && credential == Credential::Account {
                return Err(ServiceError::Forbidden);
            }
            halt_gate(state, credential)?;
            // Before the genesis the epoch must be 1; afterwards the one in force or the next.
            let in_force = state.vault.as_ref().map(|v| v.key_epoch);
            let allowed = match in_force {
                None => header.key_epoch == 1,
                Some(epoch) => header.key_epoch == epoch || header.key_epoch == epoch + 1,
            };
            if !allowed {
                return Err(ServiceError::EpochRejected);
            }
            match state.staged.get(snapshot_id) {
                Some(held) if held != container => return Err(ServiceError::ObjectConflict),
                Some(_) => {}
                None => {
                    state.staged.insert(*snapshot_id, container.to_vec());
                }
            }
            Ok(StagedSnapshot {
                snapshot_id: *snapshot_id,
                size: container.len() as u64,
            })
        })
    }

    fn stage_snapshot_closure(
        &self,
        vault_id: &[u8; 16],
        snapshot_id: &[u8; 16],
        sidecar: &[u8],
    ) -> Result<StagedClosure, ServiceError> {
        let _ = vault_id;
        self.call("stage_snapshot_closure", |state, credential| {
            // Protocol 4.14, in its order: the length, from the headers alone; then the vault's
            // authorization, the halt gate and the snapshot; then the content.
            if sidecar.len() > MAX_CLOSURE_SIDECAR_LEN {
                return Err(ServiceError::SnapshotTooLarge);
            }
            if sidecar.len() % 32 != 0 {
                return Err(ServiceError::MalformedObject);
            }
            if state.vault.is_some() && credential == Credential::Account {
                return Err(ServiceError::Forbidden);
            }
            halt_gate(state, credential)?;
            let container = match state.staged.get(snapshot_id) {
                Some(container) => container,
                None => return Err(ServiceError::SnapshotNotFound),
            };
            let parsed = parse_container(container).map_err(|_| ServiceError::MalformedObject)?;
            let ObjectKind::Snapshot {
                artifact_manifest_commitment,
                ..
            } = &parsed.header.object
            else {
                return Err(ServiceError::MalformedObject);
            };
            // The ids strictly ascending, and the commitment the server computes over them the
            // staged header's.
            let closure = ClosureManifest::from_sidecar(sidecar)
                .map_err(|_| ServiceError::MalformedObject)?;
            if closure.commitment() != *artifact_manifest_commitment {
                return Err(ServiceError::MalformedObject);
            }
            state.closures.insert(*snapshot_id, sidecar.to_vec());
            Ok(StagedClosure {
                snapshot_id: *snapshot_id,
                artifact_count: (sidecar.len() / 32) as u64,
            })
        })
    }

    fn get_snapshot_closure(
        &self,
        vault_id: &[u8; 16],
        snapshot_id: &[u8; 16],
    ) -> Result<Vec<u8>, ServiceError> {
        let _ = vault_id;
        self.call("get_snapshot_closure", |state, _| {
            if state.vault.is_none() {
                return Err(ServiceError::VaultNotFound);
            }
            // A staged snapshot's closure is not returned: only a published one's.
            if !state
                .published
                .iter()
                .any(|p| &p.snapshot_id == snapshot_id)
            {
                return Err(ServiceError::SnapshotNotFound);
            }
            state
                .closures
                .get(snapshot_id)
                .cloned()
                .ok_or(ServiceError::SnapshotNotFound)
        })
    }

    fn publish_snapshot(
        &self,
        vault_id: &[u8; 16],
        snapshot_id: &[u8; 16],
        base_seq: u64,
        artifact_manifest: &[ArtifactId],
    ) -> Result<PublishedSnapshot, ServiceError> {
        let _ = vault_id;
        self.call("publish_snapshot", |state, credential| {
            state
                .requests
                .push(("publish_snapshot", artifact_manifest.to_vec()));
            let Some(vault) = state.vault.as_ref() else {
                return Err(ServiceError::VaultNotFound);
            };
            halt_gate(state, credential)?;
            // Publishing a published snapshot answers the original result.
            if let Some(done) = state
                .published
                .iter()
                .find(|p| &p.snapshot_id == snapshot_id)
            {
                return Ok(*done);
            }
            let Some(container) = state.staged.get(snapshot_id) else {
                return Err(ServiceError::SnapshotNotFound);
            };
            let parsed = parse_container(container).map_err(|_| ServiceError::MalformedObject)?;
            let ObjectKind::Snapshot {
                artifact_manifest_commitment,
                artifact_binding_commitment,
            } = &parsed.header.object
            else {
                return Err(ServiceError::MalformedObject);
            };
            // A snapshot that starts an epoch is published by installEpoch only.
            if parsed.header.key_epoch != vault.key_epoch {
                return Err(ServiceError::EpochRejected);
            }
            if base_seq != vault.head {
                return Err(ServiceError::HeadMismatch { head: vault.head });
            }
            if parsed.header.base_seq != base_seq {
                return Err(ServiceError::MalformedObject);
            }
            let closure = closure_of_request(state, snapshot_id, artifact_manifest)?;
            let artifact_manifest = closure.as_slice();
            let sidecar =
                ClosureManifest::new(artifact_manifest.iter().map(|id| *id.as_bytes()).collect())
                    .map_err(|_| ServiceError::MalformedObject)?;
            if sidecar.commitment() != *artifact_manifest_commitment {
                return Err(ServiceError::MalformedObject);
            }
            let missing: Vec<ArtifactId> = artifact_manifest
                .iter()
                .filter(|id| !state.receipts.contains_key(id))
                .copied()
                .take(MAX_INLINE_CLOSURE)
                .collect();
            if !missing.is_empty() {
                return Err(ServiceError::ArtifactMissing { missing });
            }
            // Every receipt is present, so the binding can be checked: field 18 commits to the
            // stored ciphertext hashes in the sidecar's order.
            let hashes: Vec<[u8; 32]> = artifact_manifest
                .iter()
                .map(|id| state.receipts[id])
                .collect();
            if binding_commitment(&hashes, 1 << 20).map_err(|_| ServiceError::MalformedObject)?
                != *artifact_binding_commitment
            {
                return Err(ServiceError::ArtifactBindingMismatch);
            }
            // One transaction: the pins and the publication together, or nothing.
            let published = PublishedSnapshot {
                snapshot_id: *snapshot_id,
                base_seq,
                key_epoch: parsed.header.key_epoch,
            };
            state.pins.extend(artifact_manifest.iter().copied());
            state.closures.entry(*snapshot_id).or_insert_with(|| {
                artifact_manifest
                    .iter()
                    .flat_map(|id| *id.as_bytes())
                    .collect()
            });
            state.published.push(published);
            Ok(published)
        })
    }

    fn install_epoch(
        &self,
        vault_id: &[u8; 16],
        install: &EpochInstall<'_>,
    ) -> Result<InstalledEpoch, ServiceError> {
        self.call("install_epoch", |state, _credential| {
            state
                .requests
                .push(("install_epoch", install.artifact_manifest.to_vec()));
            if let Some(vault) = &state.vault {
                // The very same genesis again is an exact retry; anything else is VaultExists.
                let same = install.membership_record == Some(vault.genesis_membership.as_slice())
                    && install.epoch_record == vault.genesis_epoch.as_slice();
                return if same {
                    Ok(InstalledEpoch {
                        already_installed: true,
                        ..vault.installed
                    })
                } else {
                    Err(ServiceError::VaultExists)
                };
            }
            let membership = install
                .membership_record
                .ok_or(ServiceError::MembershipChainInvalid)?;
            let chain = verify_chain(vault_id, &[membership], &[install.epoch_record])
                .map_err(|_| ServiceError::EpochRecordInvalid)?;
            if install.base_seq != 0 {
                return Err(ServiceError::HeadMismatch { head: 0 });
            }
            let snapshot_id = install.snapshot_id.ok_or(ServiceError::MalformedObject)?;
            let Some(container) = state.staged.get(&snapshot_id) else {
                return Err(ServiceError::SnapshotNotFound);
            };
            let parsed = parse_container(container).map_err(|_| ServiceError::MalformedObject)?;
            let ObjectKind::Snapshot {
                artifact_manifest_commitment,
                artifact_binding_commitment,
            } = &parsed.header.object
            else {
                return Err(ServiceError::MalformedObject);
            };
            if parsed.header.key_epoch != chain.key_epoch() || parsed.header.base_seq != 0 {
                return Err(ServiceError::MalformedObject);
            }
            let closure = closure_of_request(state, &snapshot_id, install.artifact_manifest)?;
            let sidecar = ClosureManifest::new(closure.iter().map(|id| *id.as_bytes()).collect())
                .map_err(|_| ServiceError::MalformedObject)?;
            if sidecar.commitment() != *artifact_manifest_commitment {
                return Err(ServiceError::MalformedObject);
            }
            let missing: Vec<ArtifactId> = closure
                .iter()
                .filter(|id| !state.receipts.contains_key(id))
                .copied()
                .take(MAX_INLINE_CLOSURE)
                .collect();
            if !missing.is_empty() {
                return Err(ServiceError::ArtifactMissing { missing });
            }
            let hashes: Vec<[u8; 32]> = closure.iter().map(|id| state.receipts[id]).collect();
            if binding_commitment(&hashes, 1 << 20).map_err(|_| ServiceError::MalformedObject)?
                != *artifact_binding_commitment
            {
                return Err(ServiceError::ArtifactBindingMismatch);
            }
            let installed = InstalledEpoch {
                key_epoch: chain.key_epoch(),
                ordinal: 1,
                position: 0,
                membership_head: chain
                    .head_id()
                    .ok_or(ServiceError::MembershipChainInvalid)?,
                epoch_record_id: chain
                    .epoch_head_id()
                    .ok_or(ServiceError::EpochRecordInvalid)?,
                already_installed: false,
            };
            state.pins.extend(closure.iter().copied());
            state
                .closures
                .entry(snapshot_id)
                .or_insert_with(|| closure.iter().flat_map(|id| *id.as_bytes()).collect());
            state.published.push(PublishedSnapshot {
                snapshot_id,
                base_seq: 0,
                key_epoch: installed.key_epoch,
            });
            state.vault = Some(Vault {
                genesis_membership: membership.to_vec(),
                genesis_epoch: install.epoch_record.to_vec(),
                installed,
                key_epoch: installed.key_epoch,
                head: 0,
                halted_at: None,
            });
            Ok(installed)
        })
    }

    fn latest_snapshot(
        &self,
        vault_id: &[u8; 16],
    ) -> Result<Option<PublishedSnapshot>, ServiceError> {
        let _ = vault_id;
        self.call("latest_snapshot", |state, _| {
            if state.vault.is_none() {
                return Err(ServiceError::VaultNotFound);
            }
            Ok(state.published.last().copied())
        })
    }

    fn log_head(&self, vault_id: &[u8; 16]) -> Result<u64, ServiceError> {
        let _ = vault_id;
        self.call("log_head", |state, _| {
            state
                .vault
                .as_ref()
                .map(|vault| vault.head)
                .ok_or(ServiceError::VaultNotFound)
        })
    }

    fn get_snapshot<'a>(
        &'a self,
        vault_id: &[u8; 16],
        snapshot_id: &[u8; 16],
    ) -> Result<Box<dyn std::io::Read + 'a>, ServiceError> {
        let _ = vault_id;
        let bytes = self.call("get_snapshot", |state, _| {
            if state.vault.is_none() {
                return Err(ServiceError::VaultNotFound);
            }
            // Protocol 4.9: only a published snapshot is returned, never a staged one.
            if !state
                .published
                .iter()
                .any(|p| &p.snapshot_id == snapshot_id)
            {
                return Err(ServiceError::SnapshotNotFound);
            }
            state
                .staged
                .get(snapshot_id)
                .cloned()
                .ok_or(ServiceError::SnapshotNotFound)
        })?;
        Ok(Box::new(std::io::Cursor::new(bytes)))
    }
}

/// The closure a `publishSnapshot` or `installEpoch` request is judged against (protocol 4.4): the
/// one staged with the snapshot if it has one (and the request's list is then empty), the request's
/// list otherwise (at most 4096 ids).
fn closure_of_request(
    state: &State,
    snapshot_id: &[u8; 16],
    requested: &[ArtifactId],
) -> Result<Vec<ArtifactId>, ServiceError> {
    let published = state
        .published
        .iter()
        .any(|p| &p.snapshot_id == snapshot_id);
    match state.closures.get(snapshot_id) {
        // A closure was staged with the snapshot: a list in the request beside it is refused.
        Some(_) if !requested.is_empty() && !published => Err(ServiceError::MalformedRequest),
        Some(staged) if requested.is_empty() => Ok(staged
            .chunks_exact(32)
            .map(|chunk| ArtifactId::from_derived(chunk.try_into().expect("32 bytes")))
            .collect()),
        _ => {
            if requested.len() > MAX_INLINE_CLOSURE {
                return Err(ServiceError::MalformedRequest);
            }
            Ok(requested.to_vec())
        }
    }
}
