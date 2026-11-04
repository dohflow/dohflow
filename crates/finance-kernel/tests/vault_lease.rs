//! Single-instance ownership of a vault (personal-cfo-bxsbz.2; ADR 0070, ADR 0083): while one
//! process owns a vault (it holds the vault's lease), a second process gets the typed
//! `VaultInUse` on every way in, and the files of the owner are never touched. Each test runs the
//! owner as a real second process: the test binary re-runs itself on a child test that takes the
//! lease and waits to be released.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use finance_kernel::{Kernel, KernelError, RekeyRecovery, VaultController, VaultLease, VaultState};
use tempfile::TempDir;
use vault_crypto::{
    derive_kek, unwrap_dek, ReleasedUnlockMaterial, SecretBytes, VaultEnvelope, RELEASED_LEN,
};

const PASSWORD: &[u8] = b"correct horse battery staple";
const CHILD_VAULT: &str = "DOHFLOW_LEASE_CHILD_VAULT";
const CHILD_READY: &str = "DOHFLOW_LEASE_CHILD_READY";
const CHILD_RELEASE: &str = "DOHFLOW_LEASE_CHILD_RELEASE";
/// "lease": hold only the lease (an owner mid file-step). "open": hold the vault unlocked.
const CHILD_MODE: &str = "DOHFLOW_LEASE_CHILD_MODE";

/// The owner process. A normal run of the suite reaches this as a no-op; a parent re-runs the
/// binary on this test by exact name with the variables set.
#[test]
fn child_owns_the_vault_for_a_parent_test() {
    let Some(vault) = env::var_os(CHILD_VAULT) else {
        return;
    };
    let ready = PathBuf::from(env::var_os(CHILD_READY).expect("ready path"));
    let release = PathBuf::from(env::var_os(CHILD_RELEASE).expect("release path"));
    let mode = env::var(CHILD_MODE).expect("mode");
    // Both are held to the end of the function.
    let _lease;
    let _kernel;
    if mode == "open" {
        _kernel = Some(Kernel::unlock_vault(&vault, PASSWORD).expect("child unlocks"));
        _lease = None;
    } else {
        _lease = Some(VaultLease::acquire(&vault).expect("child takes the lease"));
        _kernel = None;
    }
    fs::write(&ready, b"ready").unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while !release.exists() {
        assert!(Instant::now() < deadline, "parent never released the child");
        std::thread::sleep(Duration::from_millis(5));
    }
}

struct Owner {
    child: Option<Child>,
    release: PathBuf,
}

impl Owner {
    fn spawn(dir: &Path, vault: &Path, mode: &str) -> Self {
        let ready = dir.join(format!("{mode}.ready"));
        let release = dir.join(format!("{mode}.release"));
        let child = Command::new(env::current_exe().unwrap())
            .args([
                "--exact",
                "child_owns_the_vault_for_a_parent_test",
                "--nocapture",
            ])
            .env(CHILD_VAULT, vault)
            .env(CHILD_READY, &ready)
            .env(CHILD_RELEASE, &release)
            .env(CHILD_MODE, mode)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn the owner");
        let owner = Self {
            child: Some(child),
            release,
        };
        let deadline = Instant::now() + Duration::from_secs(20);
        while !ready.exists() {
            assert!(Instant::now() < deadline, "the owner never took the vault");
            std::thread::sleep(Duration::from_millis(5));
        }
        owner
    }

    /// End the owner the way a crash does: no release, no cleanup.
    fn kill(mut self) {
        let mut child = self.child.take().unwrap();
        child.kill().unwrap();
        child.wait().unwrap();
    }

    fn release(mut self) {
        fs::write(&self.release, b"release").unwrap();
        assert!(self.child.take().unwrap().wait().unwrap().success());
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = fs::write(&self.release, b"release");
            let _ = child.wait();
        }
    }
}

fn created() -> (TempDir, PathBuf) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("vault.db");
    Kernel::create_vault(&path, PASSWORD).unwrap().lock();
    (dir, path)
}

/// Every file under `dir` except the lock file, with its bytes: what an owner's files look like.
fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        for entry in fs::read_dir(&next).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if !path.to_string_lossy().ends_with(".runner.lock")
                && path
                    .extension()
                    .is_none_or(|e| e != "ready" && e != "release")
            {
                found.push((path.clone(), fs::read(&path).unwrap()));
            }
        }
    }
    found.sort();
    found
}

fn sidecar(db: &Path) -> PathBuf {
    let mut os = db.as_os_str().to_owned();
    os.push(".envelope");
    os.into()
}

fn enroll(db: &Path) -> SecretBytes<RELEASED_LEN> {
    let envelope = VaultEnvelope::from_bytes(&fs::read(sidecar(db)).unwrap()).unwrap();
    let kek = derive_kek(PASSWORD, &envelope.salt, &envelope.kdf).unwrap();
    let dek = unwrap_dek(&kek, &envelope.wrapped).unwrap();
    let kernel = Kernel::unlock_vault(db, PASSWORD).unwrap();
    let id = *kernel.vault_id().unwrap().as_bytes();
    kernel.lock();
    ReleasedUnlockMaterial::encode(id, &envelope.wrapped, &dek)
}

fn assert_in_use<T>(result: Result<T, KernelError>) {
    match result {
        Err(KernelError::VaultInUse) => {}
        Err(other) => panic!("expected VaultInUse, got {other:?}"),
        Ok(_) => panic!("expected VaultInUse, got a success"),
    }
}

#[test]
fn create_is_refused_while_another_process_owns_the_path_and_leaves_nothing() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("vault.db");
    let owner = Owner::spawn(dir.path(), &path, "lease");
    assert_in_use(Kernel::create_vault(&path, PASSWORD));
    assert!(
        !sidecar(&path).exists(),
        "a refused create leaves no envelope"
    );
    assert!(!path.exists());
    owner.release();
}

#[test]
fn unlock_is_refused_while_another_process_owns_the_vault() {
    for mode in ["lease", "open"] {
        let (dir, path) = created();
        let before = snapshot(dir.path());
        let owner = Owner::spawn(dir.path(), &path, mode);
        assert_in_use(Kernel::unlock_vault(&path, PASSWORD));
        if mode == "lease" {
            assert_eq!(snapshot(dir.path()), before, "{mode}: no file was touched");
        }
        owner.release();
    }
}

#[test]
fn unlock_with_dek_is_refused_while_another_process_owns_the_vault() {
    let (dir, path) = created();
    let item = enroll(&path);
    let owner = Owner::spawn(dir.path(), &path, "lease");
    assert_in_use(Kernel::unlock_vault_with_dek(&path, item));
    owner.release();
}

#[test]
fn the_integrity_probe_is_refused_while_another_process_owns_the_vault() {
    let (dir, path) = created();
    let owner = Owner::spawn(dir.path(), &path, "lease");
    let options = finance_kernel::UnlockOptions::with_integrity_check();
    assert_in_use(Kernel::unlock_vault_with_options(&path, PASSWORD, options));
    owner.release();
}

#[test]
fn restore_is_refused_while_another_process_owns_the_destination_and_writes_nothing() {
    let (dir, path) = created();
    let backup = dir.path().join("vault.pcfobk");
    let kernel = Kernel::unlock_vault(&path, PASSWORD).unwrap();
    kernel
        .export_unattended(
            &backup,
            "0.0.0-test",
            "2026-11-04T00:00:00Z".to_owned(),
            uuid::Uuid::now_v7(),
        )
        .unwrap();
    kernel.lock();

    let target_dir = TempDir::new().unwrap();
    let target = target_dir.path().join("restored.db");
    let owner = Owner::spawn(target_dir.path(), &target, "lease");
    assert_in_use(Kernel::restore_backup(&backup, PASSWORD, &target));
    assert!(!target.exists());
    assert!(!sidecar(&target).exists());
    owner.release();
    // Once the owner is gone the same restore succeeds.
    Kernel::restore_backup(&backup, PASSWORD, &target)
        .unwrap()
        .lock();
}

#[test]
fn startup_recovery_touches_no_file_while_another_process_owns_the_vault() {
    let (dir, path) = created();
    // A stray rotation artifact: what recovery would delete if it ran.
    let stray = dir.path().join("vault.db.rekey-new");
    fs::write(&stray, b"half-written copy").unwrap();
    // A rotation journal too: the owner's live rotation, which a second process must not mistake
    // for a fault or act on.
    let journal = dir.path().join("vault.db.rekey");
    fs::write(&journal, b"the owner's journal").unwrap();
    let before = snapshot(dir.path());

    let owner = Owner::spawn(dir.path(), &path, "lease");
    let controller = VaultController::open(&path);
    assert_eq!(controller.rekey_recovery(), RekeyRecovery::Deferred);
    assert!(
        journal.exists(),
        "recovery must not touch the owner's journal"
    );
    assert!(stray.exists(), "recovery must not delete the owner's file");
    assert_eq!(snapshot(dir.path()), before);
    // The vault is not reported corrupt just because its owner is mid-step.
    assert_eq!(controller.state(), VaultState::Locked);
    drop(controller);
    owner.release();

    // With the owner gone, the same open looks at the journal: this one is not readable, so
    // nothing is deleted and the vault needs recovery.
    let controller = VaultController::open(&path);
    assert_eq!(controller.rekey_recovery(), RekeyRecovery::Contradiction);
    assert_eq!(controller.state(), VaultState::CorruptNeedsRecovery);
}

#[test]
fn switching_to_a_vault_another_process_owns_defers_recovery() {
    let (dir, path) = created();
    let stray = dir.path().join("vault.db.rekey-new");
    fs::write(&stray, b"half-written copy").unwrap();
    let owner = Owner::spawn(dir.path(), &path, "lease");
    let elsewhere = TempDir::new().unwrap();
    let mut controller = VaultController::open(elsewhere.path().join("vault.db"));
    controller.switch_to(path.clone()).unwrap();
    assert_eq!(controller.rekey_recovery(), RekeyRecovery::Deferred);
    assert!(stray.exists());
    owner.release();
}

#[test]
fn a_lock_file_left_by_a_crash_never_blocks_the_next_open() {
    for mode in ["lease", "open"] {
        let (dir, path) = created();
        let owner = Owner::spawn(dir.path(), &path, mode);
        owner.kill();
        let mut lock = path.as_os_str().to_owned();
        lock.push(".runner.lock");
        assert!(
            Path::new(&lock).exists(),
            "{mode}: the dead owner left its lock file"
        );
        Kernel::unlock_vault(&path, PASSWORD).unwrap().lock();
    }
}
