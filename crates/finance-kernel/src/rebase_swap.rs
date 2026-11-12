//! The rebase swap's crash-safe file protocol (personal-cfo-bxsbz.7; ADR 0074 decision 2, ADR 0083
//! Addenda B and C).
//!
//! A rebase builds the new vault beside the live one (`db-worker`'s `rebase` module); this module
//! swaps it in. It reuses the key rotation's journal discipline (`crate::rekey`): everything is
//! made and hashed first, one journal rename is the commit point, and an idempotent apply moves the
//! files. Startup recovery ([`recover_interrupted_rebase`]) reads the journal and either undoes the
//! swap (it never committed), finishes it (it did), or reports a contradiction and deletes nothing.
//!
//! File family (all on one filesystem, so every move is a rename):
//! - `vault.db.rebase`      — the journal (`preparing` → `prepared` → `committed`)
//! - `vault.db.rebase.tmp`  — the journal's write-then-rename temp file
//! - `sync/rebase-*`        — the rebuilt vault and the new base image, as the rebase named them
//!   (the scratch vault, `rebase-scratch.post-pull.db`, the trial copies, their lock sidecars)
//! - `sync/pre-rebase.db`   — the previous live vault, kept until the next accepted push
//!
//! The commit point is the journal's `committed` rename. Before it the live vault is untouched (its
//! `sync_base` row says `Rebasing`, which the next open turns back into `Ready`). After it the
//! apply runs: the live vault moves to `sync/pre-rebase.db`, the rebuilt vault becomes `vault.db`,
//! the new base image becomes `sync/base.db`, the rebase's working files are removed, and the
//! journal goes last. Every step accepts "already done", so recovery can re-run it from any
//! interruption.

use std::fs::{self, File};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use db_worker::base_image::{
    is_rebase_artifact, pre_rebase_path, rebase_journal_path, remove_rebase_artifacts, sync_dir,
    BASE_IMAGE_FILE,
};
use db_worker::rebase::SwapInputs;

use crate::KernelError;

/// Journal format version. Bump only if the journal's fields change meaning.
const JOURNAL_VERSION: u32 = 1;

const SIDE_SUFFIXES: [&str; 3] = ["-wal", "-shm", "-journal"];

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut os = path.as_os_str().to_owned();
    os.push(suffix);
    PathBuf::from(os)
}

/// Every path in the swap's file family, derived from the vault path and the journal's file names.
#[derive(Debug, Clone)]
pub(crate) struct SwapPaths {
    pub db: PathBuf,
    pub journal: PathBuf,
    pub journal_tmp: PathBuf,
    /// `<vault dir>/sync`.
    pub sync: PathBuf,
    pub pre_rebase: PathBuf,
    pub base: PathBuf,
}

impl SwapPaths {
    pub(crate) fn new(db: &Path) -> Self {
        let sync = sync_dir(db);
        let journal = rebase_journal_path(db);
        Self {
            db: db.to_path_buf(),
            journal_tmp: with_suffix(&journal, ".tmp"),
            journal,
            pre_rebase: pre_rebase_path(db),
            base: sync.join(BASE_IMAGE_FILE),
            sync,
        }
    }

    fn dir(&self) -> &Path {
        self.db.parent().unwrap_or_else(|| Path::new("."))
    }

    /// A file of the rebase under `sync/`, by the name the journal recorded.
    fn working(&self, name: &str) -> PathBuf {
        self.sync.join(name)
    }

    /// Every file of the swap beside the vault (for vault delete and restore-attempt cleanup).
    /// The files under `sync/` go with that directory.
    pub(crate) fn all_artifacts(&self) -> Vec<PathBuf> {
        vec![self.journal.clone(), self.journal_tmp.clone()]
    }
}

/// The journal's state. Only `Committed` makes the rebuilt vault authoritative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum JournalState {
    Preparing,
    Prepared,
    Committed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Journal {
    version: u32,
    state: JournalState,
    /// The rebuilt vault's file name under `sync/` (a `rebase*` name).
    scratch: String,
    /// The new base image's file name under `sync/` (a `rebase*` name).
    new_base: String,
    /// Hash of the finished rebuilt vault.
    scratch_sha256: String,
    /// Hash of the finished new base image, the digest the rebuilt vault records for `base.db`.
    new_base_sha256: String,
    /// Hash of the live vault being replaced, closed and checkpointed (set from `prepared` on).
    old_live_sha256: Option<String>,
    /// The server seq of the base the rebase started from.
    old_base_seq: i64,
    /// The server seq of the new base.
    new_base_seq: i64,
}

fn io(context: &str) -> impl FnOnce(std::io::Error) -> KernelError + '_ {
    move |error| KernelError::Vault(format!("{context}: {error}"))
}

fn remove_if_present(path: &Path) -> Result<(), KernelError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(KernelError::Vault(format!(
            "removing {}: {error}",
            path.file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
        ))),
    }
}

fn fsync_dir(dir: &Path) -> Result<(), KernelError> {
    File::open(dir)
        .and_then(|d| d.sync_all())
        .map_err(io("syncing vault directory"))
}

/// SHA-256 of a file as lowercase hex, streamed so a large database is never read into memory.
pub(crate) fn sha256_hex(path: &Path) -> Result<String, KernelError> {
    let mut file = File::open(path).map_err(io("hashing rebase file"))?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).map_err(io("hashing rebase file"))?;
    Ok(format!("{:x}", hasher.finalize()))
}

fn hex(digest: &[u8; 32]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Write the journal by temp file, `fsync`, rename, directory `fsync`: the rename is atomic, so a
/// crash leaves the previous state or the new one, never a torn file.
fn write_journal(paths: &SwapPaths, journal: &Journal) -> Result<(), KernelError> {
    let bytes = serde_json::to_vec(journal)
        .map_err(|error| KernelError::Vault(format!("encoding rebase journal: {error}")))?;
    let mut file = File::create(&paths.journal_tmp).map_err(io("writing rebase journal"))?;
    file.write_all(&bytes)
        .map_err(io("writing rebase journal"))?;
    file.sync_all().map_err(io("syncing rebase journal"))?;
    drop(file);
    fs::rename(&paths.journal_tmp, &paths.journal).map_err(io("installing rebase journal"))?;
    fsync_dir(paths.dir())
}

/// Read the journal: `Ok(None)` if absent, `Err` if present but unreadable.
fn read_journal(paths: &SwapPaths) -> Result<Option<Journal>, ()> {
    match fs::read(&paths.journal) {
        Ok(bytes) => match serde_json::from_slice::<Journal>(&bytes) {
            Ok(journal) if journal.version == JOURNAL_VERSION && names_are_safe(&journal) => {
                Ok(Some(journal))
            }
            _ => Err(()),
        },
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(_) => Err(()),
    }
}

/// The two file names a journal carries are joined onto `sync/`: each must be a plain file name
/// that the rebase's own prefix owns, so a damaged or hostile journal cannot point a rename or a
/// removal anywhere else.
fn names_are_safe(journal: &Journal) -> bool {
    [&journal.scratch, &journal.new_base]
        .iter()
        .all(|name| is_plain_rebase_name(name))
        && journal.scratch != journal.new_base
}

fn is_plain_rebase_name(name: &str) -> bool {
    is_rebase_artifact(name)
        && !name.contains(['/', '\\'])
        && Path::new(name).file_name().and_then(|n| n.to_str()) == Some(name)
}

/// The journal's `sync/` file name for one of the swap's inputs, or `None` if the file is not a
/// plain rebase file directly inside `sync/`.
fn working_name(sync: &Path, path: &Path) -> Option<String> {
    if path.parent() != Some(sync) {
        return None;
    }
    let name = path.file_name()?.to_str()?;
    is_plain_rebase_name(name).then(|| name.to_owned())
}

/// Undo a swap that never committed: the rebase's working files and the journal go; the live vault
/// is untouched. Missing files are fine.
fn roll_back(paths: &SwapPaths) -> Result<(), KernelError> {
    remove_rebase_artifacts(&paths.db).map_err(io("removing rebase files"))?;
    remove_if_present(&paths.journal_tmp)?;
    remove_if_present(&paths.journal)?;
    fsync_dir(paths.dir())
}

/// Whether the rebuilt vault and the new base image are both waiting under `sync/` and hash to the
/// values the journal carries. A missing file is a mismatch, not an error.
fn working_files_match(paths: &SwapPaths, journal: &Journal) -> Result<bool, KernelError> {
    let matches = |path: PathBuf, expected: &str| -> Result<bool, KernelError> {
        Ok(path.is_file() && sha256_hex(&path)? == expected)
    };
    Ok(
        matches(paths.working(&journal.scratch), &journal.scratch_sha256)?
            && matches(paths.working(&journal.new_base), &journal.new_base_sha256)?,
    )
}

/// Whether the files on disk agree with a `committed` journal. Every check is "already applied, or
/// ready to apply", so the same test holds at every apply step and a contradiction is reported
/// instead of guessed at.
fn committed_files_agree(paths: &SwapPaths, journal: &Journal) -> Result<bool, KernelError> {
    let Some(old_live) = &journal.old_live_sha256 else {
        return Ok(false);
    };
    let scratch = paths.working(&journal.scratch);
    let new_base = paths.working(&journal.new_base);
    let hashes = |path: &Path, expected: &str| -> Result<bool, KernelError> {
        Ok(path.exists() && sha256_hex(path)? == expected)
    };
    // The rebuilt vault: still waiting under `sync/`, or already `vault.db`.
    let vault_ok = if scratch.exists() {
        hashes(&scratch, &journal.scratch_sha256)?
    } else {
        hashes(&paths.db, &journal.scratch_sha256)?
    };
    // The old live vault matters only while the rebuilt one still waits: it is `vault.db` until it
    // is moved aside, `pre-rebase.db` after.
    let old_ok = if scratch.exists() {
        if paths.db.exists() {
            hashes(&paths.db, old_live)?
        } else {
            hashes(&paths.pre_rebase, old_live)?
        }
    } else {
        true
    };
    // The new base image: still waiting, or already `base.db`.
    let base_ok = if new_base.exists() {
        hashes(&new_base, &journal.new_base_sha256)?
    } else {
        hashes(&paths.base, &journal.new_base_sha256)?
    };
    Ok(vault_ok && old_ok && base_ok)
}

/// The apply steps after the commit point, each idempotent so recovery can re-run them from any
/// interruption. `stop` is the test-only crash hook.
fn apply_committed(
    paths: &SwapPaths,
    journal: &Journal,
    stop: Option<CrashPoint>,
) -> Result<(), KernelError> {
    let scratch = paths.working(&journal.scratch);
    let new_base = paths.working(&journal.new_base);
    // 1. The old vault's side files go before it moves. While the journal exists the vault has
    //    never been opened since the worker closed, so any side file belongs to the old one.
    if scratch.exists() {
        for suffix in SIDE_SUFFIXES {
            remove_if_present(&with_suffix(&paths.db, suffix))?;
        }
    }
    crash(stop, CrashPoint::OldSideFilesRemoved)?;
    // 2. The old live vault is kept as `sync/pre-rebase.db`, replacing the one a previous rebase
    //    left. An atomic rename, so there is never a moment with neither file.
    if scratch.exists() && paths.db.exists() {
        for suffix in SIDE_SUFFIXES {
            remove_if_present(&with_suffix(&paths.pre_rebase, suffix))?;
        }
        fs::rename(&paths.db, &paths.pre_rebase).map_err(io("keeping the previous vault"))?;
        fsync_dir(paths.dir())?;
        fsync_dir(&paths.sync)?;
    }
    crash(stop, CrashPoint::OldLiveKept)?;
    // 3. The rebuilt vault becomes `vault.db`.
    if scratch.exists() {
        fs::rename(&scratch, &paths.db).map_err(io("installing the rebuilt vault"))?;
        fsync_dir(paths.dir())?;
        fsync_dir(&paths.sync)?;
    }
    crash(stop, CrashPoint::RebuiltInstalled)?;
    // 4. The new base image becomes `sync/base.db`.
    if new_base.exists() {
        for suffix in SIDE_SUFFIXES {
            remove_if_present(&with_suffix(&paths.base, suffix))?;
        }
        fs::rename(&new_base, &paths.base).map_err(io("installing the new base image"))?;
        fsync_dir(&paths.sync)?;
    }
    crash(stop, CrashPoint::BaseInstalled)?;
    // 5. What is left of the rebase: the trial copies, side files and lock sidecars.
    remove_rebase_artifacts(&paths.db).map_err(io("removing rebase files"))?;
    crash(stop, CrashPoint::ArtifactsRemoved)?;
    // 6. The journal, last.
    remove_if_present(&paths.journal_tmp)?;
    remove_if_present(&paths.journal)?;
    fsync_dir(paths.dir())
}

/// What startup recovery did with an interrupted rebase swap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebaseRecovery {
    /// No swap was in progress.
    Clean,
    /// An uncommitted swap was undone; the live vault stands.
    RolledBack,
    /// A committed swap was finished; the rebuilt vault stands.
    RolledForward,
    /// The journal and the files disagree, or the journal is unreadable; nothing was deleted.
    Contradiction,
    /// A key-rotation journal and a rebase journal are both present. The two protocols are mutually
    /// exclusive, so neither was run and nothing was touched: the open fails closed
    /// ([`KernelError::ConflictingJournals`]).
    BothJournals,
    /// Another process owns the vault (it holds the vault's lease), so recovery was not attempted
    /// and no file was touched.
    Deferred,
}

/// Startup recovery for an interrupted rebase swap. Runs before the vault is classified and after
/// the check that no rotation journal is also present; never opens the database.
#[must_use]
pub fn recover_interrupted_rebase(db_path: &Path) -> RebaseRecovery {
    let paths = SwapPaths::new(db_path);
    match read_journal(&paths) {
        Err(()) => RebaseRecovery::Contradiction,
        Ok(None) => {
            // A swap that died before its first journal write leaves at most the journal's temp
            // file. The rebase's working files are removed by the next open (a leftover with no
            // journal never outlives it), and the `Rebasing` record is reset there too.
            if paths.journal_tmp.exists() {
                return match remove_if_present(&paths.journal_tmp) {
                    Ok(()) => RebaseRecovery::RolledBack,
                    Err(_) => RebaseRecovery::Contradiction,
                };
            }
            RebaseRecovery::Clean
        }
        Ok(Some(journal)) => match journal.state {
            JournalState::Preparing | JournalState::Prepared => match roll_back(&paths) {
                Ok(()) => RebaseRecovery::RolledBack,
                Err(_) => RebaseRecovery::Contradiction,
            },
            JournalState::Committed => match committed_files_agree(&paths, &journal) {
                Ok(true) => match apply_committed(&paths, &journal, None) {
                    Ok(()) => RebaseRecovery::RolledForward,
                    Err(_) => RebaseRecovery::Contradiction,
                },
                _ => RebaseRecovery::Contradiction,
            },
        },
    }
}

/// Whether a rebase journal is present.
#[must_use]
pub(crate) fn journal_present(db_path: &Path) -> bool {
    SwapPaths::new(db_path).journal.exists()
}

/// Test-only crash points: each names the last step completed. The swap stops right after it,
/// leaving files exactly as a process death would.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CrashPoint {
    /// The worker is closed and `sync_base` says `Rebasing`; no journal yet.
    Frozen,
    PreparingJournalWritten,
    PreparedJournalWritten,
    Committed,
    OldSideFilesRemoved,
    OldLiveKept,
    RebuiltInstalled,
    BaseInstalled,
    ArtifactsRemoved,
}

const SIMULATED_CRASH: &str = "simulated crash";

fn crash(stop: Option<CrashPoint>, here: CrashPoint) -> Result<(), KernelError> {
    #[cfg(test)]
    crate::rekey::observe::at(&format!("rebase:{here:?}"));
    if stop == Some(here) {
        return Err(KernelError::Vault(SIMULATED_CRASH.to_owned()));
    }
    Ok(())
}

pub(crate) fn is_simulated_crash(error: &KernelError) -> bool {
    matches!(error, KernelError::Vault(message) if message == SIMULATED_CRASH)
}

/// Prepare and commit the swap of a finished rebase into the vault at `db_path`. The caller has
/// frozen the live worker (`DbWorker::begin_swap`), closed it into the vault lease (so the file is
/// whole and nothing holds it) and holds that lease for the whole call and the apply that follows.
/// On success the journal is `committed` and the caller runs [`finish_swap`]. On failure the swap is
/// undone (the journal and the rebase's working files are removed) and the live vault is untouched;
/// a failed swap is redone by a fresh rebase.
pub(crate) fn prepare_and_commit(
    db_path: &Path,
    inputs: &SwapInputs,
    stop: Option<CrashPoint>,
) -> Result<(), KernelError> {
    let paths = SwapPaths::new(db_path);
    let (Some(scratch), Some(new_base)) = (
        working_name(&paths.sync, &inputs.scratch_path),
        working_name(&paths.sync, &inputs.new_base_path),
    ) else {
        return Err(KernelError::Vault(
            "the rebuilt vault is not where the swap expects it".to_owned(),
        ));
    };
    if scratch == new_base || paths.journal.exists() || paths.journal_tmp.exists() {
        return Err(KernelError::Vault(
            "a previous rebase swap was not cleaned up; reopen the vault first".to_owned(),
        ));
    }
    let result = prepare_inner(&paths, inputs, scratch, new_base, stop);
    if let Err(error) = &result {
        if !is_simulated_crash(error) {
            let _ = roll_back(&paths);
        }
    }
    result
}

fn prepare_inner(
    paths: &SwapPaths,
    inputs: &SwapInputs,
    scratch: String,
    new_base: String,
    stop: Option<CrashPoint>,
) -> Result<(), KernelError> {
    crash(stop, CrashPoint::Frozen)?;
    let mut journal = Journal {
        version: JOURNAL_VERSION,
        state: JournalState::Preparing,
        scratch,
        new_base,
        scratch_sha256: hex(&inputs.scratch_sha256),
        new_base_sha256: hex(&inputs.new_base_sha256),
        old_live_sha256: None,
        old_base_seq: inputs.old_base_seq,
        new_base_seq: inputs.head_seq,
    };
    write_journal(paths, &journal)?;
    crash(stop, CrashPoint::PreparingJournalWritten)?;

    // The rebuilt vault and the new base were hashed when the rebase finished them, and the journal
    // is about to attest those hashes. Verify the files still match before the commit point
    // (04b #354 F1): a damaged file, a scratch rebuilt at the same path after `abandon_rebase`, or
    // inputs left over from an earlier rebase would otherwise be installed under a hash they do not
    // have, a state recovery would call a contradiction. A mismatch is refused here, before any
    // rename, and `prepare_and_commit` undoes the journal and the working files.
    if !working_files_match(paths, &journal)? {
        return Err(KernelError::RebaseNotReady);
    }

    // The old live vault is whole (checkpointed, the worker closed): this is the file the journal
    // names.
    journal.old_live_sha256 = Some(sha256_hex(&paths.db)?);
    journal.state = JournalState::Prepared;
    write_journal(paths, &journal)?;
    crash(stop, CrashPoint::PreparedJournalWritten)?;

    // The commit point: one journal rename.
    journal.state = JournalState::Committed;
    if let Err(error) = write_journal(paths, &journal) {
        // Committed only if the rename landed; re-read to know which side we are on.
        return match read_journal(paths) {
            Ok(Some(j)) if j.state == JournalState::Committed => Ok(()),
            _ => Err(error),
        };
    }
    crash(stop, CrashPoint::Committed)?;
    Ok(())
}

/// Apply a committed swap under the caller's lease, after the files were agreed.
pub(crate) fn finish_swap(db_path: &Path, stop: Option<CrashPoint>) -> Result<(), KernelError> {
    let paths = SwapPaths::new(db_path);
    let journal = match read_journal(&paths) {
        Ok(Some(journal)) if journal.state == JournalState::Committed => journal,
        _ => {
            return Err(KernelError::Vault(
                "the rebase swap journal is not committed".to_owned(),
            ))
        }
    };
    apply_committed(&paths, &journal, stop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_working_file_name_is_a_plain_rebase_name_and_nothing_else() {
        assert!(is_plain_rebase_name("rebase-scratch.db"));
        assert!(is_plain_rebase_name("rebase-scratch.trial-1.db"));
        for bad in [
            "base.db",
            "pre-rebase.db",
            "../rebase-x.db",
            "rebase/../../vault.db",
            "snapshots",
            "",
            "rebase\\x",
        ] {
            assert!(!is_plain_rebase_name(bad), "{bad:?}");
        }
    }
}

#[cfg(test)]
mod protocol_tests {
    use super::*;
    use crate::vault::VaultController;
    use crate::{Kernel, VaultState};
    use db_worker::base_image::BaseImageStatus;
    use db_worker::rebase::Preservation;
    use uuid::Uuid;

    const PASSWORD: &[u8] = b"rebase swap test password";
    const BASE_SEQ: i64 = 5;
    const AFTER_BASE: &str = "device.after_base";

    /// A vault with a credential, a base image taken at `BASE_SEQ`, and a device-local change made
    /// after it (class 3, so a rebase carries it), left unlocked.
    fn seeded() -> (tempfile::TempDir, PathBuf, VaultController) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.db");
        let mut controller = VaultController::open(&path);
        controller.create(PASSWORD).unwrap();
        let kernel = controller.kernel().unwrap();
        kernel
            .worker
            .create_connector_connection(
                Uuid::from_bytes([9; 16]),
                "lunchflow",
                "tok-swap-fixture",
                None,
            )
            .unwrap();
        kernel.worker.initialize_base_image(BASE_SEQ).unwrap();
        kernel.worker.set_device_setting(AFTER_BASE, "x").unwrap();
        (dir, path, controller)
    }

    /// Run the rebase to the end on `controller`'s vault (no envelopes pulled, no local commands to
    /// replay): the finished files and what it carried over.
    fn rebuilt(controller: &VaultController) -> (SwapInputs, Preservation) {
        let kernel = controller.kernel().unwrap();
        let image = kernel.capture_rebase_preservation().unwrap();
        let mut scratch = kernel.rollback_to_base(BASE_SEQ).unwrap();
        scratch.apply_pulled(&[]).unwrap();
        scratch.rebase_local(&[]).unwrap();
        scratch.overlay_class3(&image).unwrap();
        scratch.apply_pulled_class3_effects().unwrap();
        (scratch.into_swap().unwrap(), image)
    }

    fn hash(path: &Path) -> String {
        sha256_hex(path).unwrap()
    }

    /// Every name under `sync/`, lock sidecars included.
    fn sync_names(path: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(SwapPaths::new(path).sync)
            .map(|dir| {
                dir.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    fn rebase_names(path: &Path) -> Vec<String> {
        sync_names(path)
            .into_iter()
            .filter(|name| is_rebase_artifact(name) && name != "pre-rebase.db")
            .collect()
    }

    fn assert_no_swap_files(path: &Path) {
        let paths = SwapPaths::new(path);
        assert!(!paths.journal.exists(), "journal left behind");
        assert!(!paths.journal_tmp.exists(), "journal temp left behind");
        assert_eq!(rebase_names(path), Vec::<String>::new());
    }

    fn base_status(controller: &VaultController) -> BaseImageStatus {
        controller
            .kernel()
            .unwrap()
            .worker
            .base_image_status()
            .unwrap()
    }

    #[test]
    fn a_swap_installs_the_rebuilt_vault_keeps_the_old_one_and_leaves_no_working_file() {
        let (_dir, path, mut controller) = seeded();
        let (inputs, image) = rebuilt(&controller);
        let paths = SwapPaths::new(&path);
        // A sealed snapshot saved for a retransmission lives under `sync/snapshots`: not a rebase
        // file, so no sweep or removal of the swap touches it.
        let saved = paths.sync.join("snapshots");
        fs::create_dir_all(&saved).unwrap();
        fs::write(saved.join("00.snapshot"), b"sealed").unwrap();

        controller.swap_rebased(&inputs, &image).unwrap();

        assert_eq!(controller.state(), VaultState::Unlocked);
        assert_no_swap_files(&path);
        // The rebuilt vault stands (the reopen only checkpoints it), the old one is kept, and the
        // new base image is the one the rebuilt vault vouches for.
        assert_eq!(
            hash(&paths.base),
            crate::rebase_swap::hex(&inputs.new_base_sha256)
        );
        assert!(paths.pre_rebase.exists());
        assert_eq!(
            base_status(&controller),
            BaseImageStatus::Ready {
                base_seq: inputs.head_seq
            }
        );
        let kernel = controller.kernel().unwrap();
        assert_eq!(
            kernel.get_setting(AFTER_BASE).unwrap().as_deref(),
            Some("x")
        );
        assert!(controller.health_check().unwrap().is_healthy());
        assert_eq!(fs::read(saved.join("00.snapshot")).unwrap(), b"sealed");
        // Nothing was left locked: the next open works.
        controller.lock().unwrap();
        controller.unlock(PASSWORD).unwrap();
        assert_eq!(
            base_status(&controller),
            BaseImageStatus::Ready {
                base_seq: inputs.head_seq
            }
        );
    }

    #[test]
    fn an_abandoned_rebase_removes_its_working_files_and_only_those() {
        // 04b's #308 O2: a rebase that stops before the swap (a block, an overlay error, an error
        // inside `into_swap`) leaves its scratch vault, new base and lock sidecars behind.
        let (_dir, path, controller) = seeded();
        let paths = SwapPaths::new(&path);
        let kernel = controller.kernel().unwrap();
        let mut scratch = kernel.rollback_to_base(BASE_SEQ).unwrap();
        scratch.apply_pulled(&[]).unwrap();
        scratch.rebase_local(&[]).unwrap();
        drop(scratch);
        let saved = paths.sync.join("snapshots");
        fs::create_dir_all(&saved).unwrap();
        fs::write(saved.join("00.snapshot"), b"sealed").unwrap();
        let base = hash(&paths.base);
        assert!(!rebase_names(&path).is_empty());

        assert!(kernel.abandon_rebase().unwrap() >= 3);

        assert_no_swap_files(&path);
        assert_eq!(hash(&paths.base), base);
        assert_eq!(fs::read(saved.join("00.snapshot")).unwrap(), b"sealed");
        assert_eq!(kernel.abandon_rebase().unwrap(), 0, "idempotent");
    }

    #[test]
    fn the_previous_vault_is_discarded_on_request_and_nothing_else_under_sync_is_touched() {
        let (_dir, path, mut controller) = seeded();
        let (inputs, image) = rebuilt(&controller);
        let paths = SwapPaths::new(&path);
        controller.swap_rebased(&inputs, &image).unwrap();
        let saved = paths.sync.join("snapshots");
        fs::create_dir_all(&saved).unwrap();
        fs::write(saved.join("00.snapshot"), b"sealed").unwrap();
        let base = hash(&paths.base);
        assert!(paths.pre_rebase.exists());

        let kernel = controller.kernel().unwrap();
        assert!(kernel.discard_pre_rebase_copy().unwrap());
        assert!(!paths.pre_rebase.exists());
        assert!(
            !kernel.discard_pre_rebase_copy().unwrap(),
            "nothing left to discard"
        );

        assert_eq!(hash(&paths.base), base);
        assert_eq!(fs::read(saved.join("00.snapshot")).unwrap(), b"sealed");
        assert!(controller.health_check().unwrap().is_healthy());
    }

    #[test]
    fn the_credential_the_rebuilt_vault_keeps_never_reaches_the_new_base_image() {
        let (_dir, path, mut controller) = seeded();
        let (inputs, image) = rebuilt(&controller);
        controller.swap_rebased(&inputs, &image).unwrap();
        let kernel = controller.kernel().unwrap();
        assert_eq!(
            kernel
                .connector_credential(Uuid::from_bytes([9; 16]))
                .unwrap()
                .as_deref(),
            Some("tok-swap-fixture")
        );
        let base = fs::read(SwapPaths::new(&path).base).unwrap();
        assert!(!String::from_utf8_lossy(&base).contains("tok-swap-fixture"));
    }

    #[test]
    fn a_live_vault_that_moved_on_is_refused_changes_nothing_and_its_rebase_files_are_removed() {
        let (_dir, path, mut controller) = seeded();
        let (inputs, image) = rebuilt(&controller);
        controller
            .kernel()
            .unwrap()
            .worker
            .set_device_setting("device.moved_on", "y")
            .unwrap();
        let before = rebase_names(&path);
        assert!(!before.is_empty(), "the rebase left its files");

        let error = controller.swap_rebased(&inputs, &image).unwrap_err();

        assert!(matches!(error, KernelError::RebaseLiveMovedOn), "{error:?}");
        assert_eq!(controller.state(), VaultState::Unlocked);
        assert_no_swap_files(&path);
        assert_eq!(
            base_status(&controller),
            BaseImageStatus::Ready { base_seq: BASE_SEQ }
        );
        // The vault still works and still has the change the rebase would have dropped.
        assert_eq!(
            controller
                .kernel()
                .unwrap()
                .get_setting("device.moved_on")
                .unwrap()
                .as_deref(),
            Some("y")
        );
    }

    /// What a refused swap must leave: the old vault and its data, the old base image, no journal
    /// and no working file, and a vault that unlocks and stays healthy (04b #354 F1).
    fn assert_refused_and_nothing_swapped(
        controller: &mut VaultController,
        path: &Path,
        inputs: &SwapInputs,
        base_before: &str,
    ) {
        let paths = SwapPaths::new(path);
        assert_eq!(controller.state(), VaultState::Unlocked);
        assert_no_swap_files(path);
        assert!(!paths.pre_rebase.exists(), "nothing was moved aside");
        assert_eq!(
            hash(&paths.base),
            base_before,
            "the base image is as it was"
        );
        assert_ne!(
            hash(&paths.db),
            crate::rebase_swap::hex(&inputs.scratch_sha256),
            "the rebuilt vault was not installed"
        );
        assert_eq!(
            base_status(controller),
            BaseImageStatus::Ready { base_seq: BASE_SEQ }
        );
        assert_eq!(
            controller
                .kernel()
                .unwrap()
                .get_setting(AFTER_BASE)
                .unwrap()
                .as_deref(),
            Some("x")
        );
        assert!(controller.health_check().unwrap().is_healthy());
        controller.lock().unwrap();
        controller.unlock(PASSWORD).unwrap();
        assert!(controller.health_check().unwrap().is_healthy());
    }

    #[test]
    fn a_rebuilt_vault_or_new_base_changed_after_into_swap_is_refused_before_anything_is_swapped() {
        // 04b #354 F1: the journal attests the hashes `into_swap` took, so the files must still
        // have them when the swap commits. Damage each in turn, and a missing file too.
        for target in ["scratch", "new base", "scratch missing", "new base missing"] {
            let (_dir, path, mut controller) = seeded();
            let (inputs, image) = rebuilt(&controller);
            let base_before = hash(&SwapPaths::new(&path).base);
            let file = if target.starts_with("scratch") {
                &inputs.scratch_path
            } else {
                &inputs.new_base_path
            };
            if target.ends_with("missing") {
                fs::remove_file(file).unwrap();
            } else {
                let mut bytes = fs::read(file).unwrap();
                bytes[100] ^= 0xff;
                fs::write(file, &bytes).unwrap();
            }

            let error = controller.swap_rebased(&inputs, &image).unwrap_err();

            assert!(
                matches!(error, KernelError::RebaseNotReady),
                "{target}: {error:?}"
            );
            assert_refused_and_nothing_swapped(&mut controller, &path, &inputs, &base_before);
        }
    }

    #[test]
    fn inputs_left_over_from_an_earlier_rebase_are_refused_the_same_way() {
        // A scratch vault rebuilt at the same path after `abandon_rebase` has different bytes than
        // the `SwapInputs` of the first rebase hashed.
        let (_dir, path, mut controller) = seeded();
        let (stale, image) = rebuilt(&controller);
        let base_before = hash(&SwapPaths::new(&path).base);
        assert!(controller.kernel().unwrap().abandon_rebase().unwrap() >= 2);
        let (fresh, _) = rebuilt(&controller);
        // The same file names, different content (every vault has its own salt and nonces).
        assert_eq!(stale.scratch_path, fresh.scratch_path);
        assert_ne!(stale.scratch_sha256, fresh.scratch_sha256);

        let error = controller.swap_rebased(&stale, &image).unwrap_err();

        assert!(matches!(error, KernelError::RebaseNotReady), "{error:?}");
        assert_refused_and_nothing_swapped(&mut controller, &path, &stale, &base_before);
    }

    /// The point a swap that stopped there leaves the files at, and what recovery must do with it.
    const POINTS: [(CrashPoint, bool); 9] = [
        (CrashPoint::Frozen, false),
        (CrashPoint::PreparingJournalWritten, false),
        (CrashPoint::PreparedJournalWritten, false),
        (CrashPoint::Committed, true),
        (CrashPoint::OldSideFilesRemoved, true),
        (CrashPoint::OldLiveKept, true),
        (CrashPoint::RebuiltInstalled, true),
        (CrashPoint::BaseInstalled, true),
        (CrashPoint::ArtifactsRemoved, true),
    ];

    /// Crash the swap at `point`, then reopen like a new process: the controller recovers, the
    /// vault classifies and unlocks, and everything is consistent.
    fn crash_and_reopen(point: CrashPoint, forward: bool) {
        let (dir, path, mut controller) = seeded();
        let (inputs, image) = rebuilt(&controller);
        let paths = SwapPaths::new(&path);
        let base_before = hash(&paths.base);

        let error = controller
            .swap_rebased_with(&inputs, &image, Some(point))
            .unwrap_err();
        assert!(is_simulated_crash(&error), "{point:?}: {error:?}");
        // A process death: no destructor of the controller's state is trusted, and the lease goes
        // with the process. Dropping the controller releases it.
        let old_live = read_journal(&paths)
            .ok()
            .flatten()
            .and_then(|journal| journal.old_live_sha256);
        drop(controller);

        let mut reopened = VaultController::open(&path);
        let expected = match (point, forward) {
            (CrashPoint::Frozen, _) => RebaseRecovery::Clean,
            (_, true) => RebaseRecovery::RolledForward,
            (_, false) => RebaseRecovery::RolledBack,
        };
        assert_eq!(reopened.rebase_recovery(), expected, "{point:?}");
        assert_eq!(reopened.state(), VaultState::Locked, "{point:?}");
        // Before anything opens the database: whose file is `vault.db`?
        let vault_is_rebuilt = hash(&paths.db) == crate::rebase_swap::hex(&inputs.scratch_sha256);
        assert_eq!(vault_is_rebuilt, forward, "{point:?}");
        if forward {
            let old = old_live
                .clone()
                .expect("a committed journal names the old vault");
            assert_eq!(hash(&paths.pre_rebase), old, "{point:?}");
            assert_eq!(
                hash(&paths.base),
                crate::rebase_swap::hex(&inputs.new_base_sha256),
                "{point:?}"
            );
        } else {
            assert_eq!(hash(&paths.base), base_before, "{point:?}");
        }
        assert!(!paths.journal.exists(), "{point:?}");

        reopened.unlock(PASSWORD).unwrap();
        assert_no_swap_files(&path);
        assert_eq!(
            base_status(&reopened),
            BaseImageStatus::Ready {
                base_seq: if forward { inputs.head_seq } else { BASE_SEQ }
            },
            "{point:?}"
        );
        assert_eq!(
            reopened
                .kernel()
                .unwrap()
                .get_setting(AFTER_BASE)
                .unwrap()
                .as_deref(),
            Some("x"),
            "{point:?}"
        );
        assert!(reopened.health_check().unwrap().is_healthy(), "{point:?}");
        // The same vault, locked and unlocked once more, with a fresh process's view of the files.
        reopened.lock().unwrap();
        let again = VaultController::open(&path);
        assert_eq!(again.rebase_recovery(), RebaseRecovery::Clean, "{point:?}");
        drop(dir);
    }

    #[test]
    fn a_kill_between_any_two_steps_leaves_a_consistent_vault_on_reopen() {
        for (point, forward) in POINTS {
            crash_and_reopen(point, forward);
        }
    }

    #[test]
    fn a_swap_interrupted_a_second_time_while_it_is_being_finished_still_finishes() {
        let (_dir, path, mut controller) = seeded();
        let (inputs, image) = rebuilt(&controller);
        let paths = SwapPaths::new(&path);
        controller
            .swap_rebased_with(&inputs, &image, Some(CrashPoint::OldLiveKept))
            .unwrap_err();
        drop(controller);
        // The process restarts and is killed again, later in the same apply.
        let lease = db_worker::VaultLease::acquire(&path).unwrap();
        let error = finish_swap(&path, Some(CrashPoint::BaseInstalled)).unwrap_err();
        assert!(is_simulated_crash(&error));
        drop(lease);

        let mut reopened = VaultController::open(&path);
        assert_eq!(reopened.rebase_recovery(), RebaseRecovery::RolledForward);
        reopened.unlock(PASSWORD).unwrap();
        assert_no_swap_files(&path);
        assert_eq!(
            hash(&paths.base),
            crate::rebase_swap::hex(&inputs.new_base_sha256)
        );
    }

    #[test]
    fn files_that_disagree_with_a_committed_journal_are_reported_and_nothing_is_deleted() {
        let (_dir, path, mut controller) = seeded();
        let (inputs, image) = rebuilt(&controller);
        let paths = SwapPaths::new(&path);
        controller
            .swap_rebased_with(&inputs, &image, Some(CrashPoint::Committed))
            .unwrap_err();
        drop(controller);
        // The rebuilt vault is damaged after the commit.
        let scratch = inputs.scratch_path.clone();
        let mut bytes = fs::read(&scratch).unwrap();
        bytes[100] ^= 0xff;
        fs::write(&scratch, &bytes).unwrap();

        let reopened = VaultController::open(&path);

        assert_eq!(reopened.rebase_recovery(), RebaseRecovery::Contradiction);
        assert_eq!(reopened.state(), VaultState::CorruptNeedsRecovery);
        assert!(paths.journal.exists(), "the journal is kept");
        assert!(scratch.exists() && paths.db.exists() && inputs.new_base_path.exists());
        assert!(!paths.pre_rebase.exists(), "nothing was moved");
    }

    #[test]
    fn a_damaged_old_vault_is_a_contradiction_wherever_the_interrupted_apply_left_it() {
        // 04b #354 F2: the old live vault is `vault.db` until it is moved aside and
        // `sync/pre-rebase.db` after. A committed journal names its hash, and a file that no longer
        // has it must not be rolled forward over (the rebuilt vault would replace a vault recovery
        // can no longer vouch for being the one the journal kept).
        for (point, damaged_is_pre_rebase) in [
            (CrashPoint::Committed, false),
            (CrashPoint::OldSideFilesRemoved, false),
            (CrashPoint::OldLiveKept, true),
        ] {
            let (_dir, path, mut controller) = seeded();
            let (inputs, image) = rebuilt(&controller);
            let paths = SwapPaths::new(&path);
            controller
                .swap_rebased_with(&inputs, &image, Some(point))
                .unwrap_err();
            drop(controller);
            let old = if damaged_is_pre_rebase {
                &paths.pre_rebase
            } else {
                &paths.db
            };
            let mut bytes = fs::read(old).unwrap();
            bytes[100] ^= 0xff;
            fs::write(old, &bytes).unwrap();

            let reopened = VaultController::open(&path);

            assert_eq!(
                reopened.rebase_recovery(),
                RebaseRecovery::Contradiction,
                "{point:?}"
            );
            assert_eq!(reopened.state(), VaultState::CorruptNeedsRecovery);
            assert!(paths.journal.exists(), "{point:?}: the journal is kept");
            assert!(
                inputs.scratch_path.exists() && inputs.new_base_path.exists(),
                "{point:?}: the rebuilt files were not touched"
            );
            assert_ne!(
                hash(&paths.base),
                crate::rebase_swap::hex(&inputs.new_base_sha256),
                "{point:?}: the new base was not installed"
            );
        }
    }

    #[test]
    fn a_journal_that_names_a_file_outside_the_rebase_is_a_contradiction_and_moves_nothing() {
        let (_dir, path, mut controller) = seeded();
        let (inputs, image) = rebuilt(&controller);
        let paths = SwapPaths::new(&path);
        controller
            .swap_rebased_with(&inputs, &image, Some(CrashPoint::Committed))
            .unwrap_err();
        drop(controller);
        let envelope = SwapPaths::new(&path).db.with_extension("db.envelope");
        let envelope_bytes = fs::read(&envelope).unwrap();
        let mut journal: serde_json::Value =
            serde_json::from_slice(&fs::read(&paths.journal).unwrap()).unwrap();
        for hostile in [
            "../vault.db.envelope",
            "base.db",
            "pre-rebase.db",
            "rebase/../x",
        ] {
            journal["scratch"] = hostile.into();
            // The files the name points at "agree" with the journal (it carries their hash), so
            // only the name check stands between a damaged journal and a rename of them.
            let target = paths.sync.join(hostile);
            if target.is_file() {
                journal["scratch_sha256"] = hash(&target).into();
            }
            fs::write(&paths.journal, serde_json::to_vec(&journal).unwrap()).unwrap();

            assert_eq!(
                recover_interrupted_rebase(&path),
                RebaseRecovery::Contradiction,
                "{hostile}"
            );
            assert_eq!(fs::read(&envelope).unwrap(), envelope_bytes, "{hostile}");
            assert!(paths.db.exists() && paths.base.exists(), "{hostile}");
        }
    }

    #[test]
    fn a_rotation_journal_and_a_rebase_journal_together_fail_the_open_closed_with_a_typed_error() {
        let (_dir, path, mut controller) = seeded();
        let (inputs, image) = rebuilt(&controller);
        let paths = SwapPaths::new(&path);
        controller
            .swap_rebased_with(&inputs, &image, Some(CrashPoint::Committed))
            .unwrap_err();
        drop(controller);
        let rekey_journal = crate::rekey::RekeyPaths::new(&path).journal;
        fs::write(&rekey_journal, b"{}").unwrap();
        let before: Vec<_> = [&paths.journal, &rekey_journal, &paths.db, &paths.base]
            .iter()
            .map(|file| fs::read(file).unwrap())
            .collect();
        let names = sync_names(&path);

        let mut reopened = VaultController::open(&path);

        assert_eq!(reopened.rebase_recovery(), RebaseRecovery::BothJournals);
        assert_eq!(reopened.state(), VaultState::CorruptNeedsRecovery);
        let refused = reopened.unlock(PASSWORD).unwrap_err();
        assert!(
            matches!(refused, KernelError::ConflictingJournals),
            "{refused:?}"
        );
        let direct = Kernel::unlock_vault(&path, PASSWORD).err().unwrap();
        assert!(
            matches!(direct, KernelError::ConflictingJournals),
            "{direct:?}"
        );
        // Nothing was run, moved or deleted by either recovery.
        let after: Vec<_> = [&paths.journal, &rekey_journal, &paths.db, &paths.base]
            .iter()
            .map(|file| fs::read(file).unwrap())
            .collect();
        assert_eq!(before, after);
        assert_eq!(names, sync_names(&path));
    }

    #[test]
    fn a_rebase_journal_alone_is_not_opened_by_a_direct_unlock() {
        let (_dir, path, mut controller) = seeded();
        let (inputs, image) = rebuilt(&controller);
        controller
            .swap_rebased_with(&inputs, &image, Some(CrashPoint::Committed))
            .unwrap_err();
        drop(controller);

        let direct = Kernel::unlock_vault(&path, PASSWORD).err().unwrap();

        assert!(
            matches!(direct, KernelError::RebaseInProgress),
            "{direct:?}"
        );
        assert!(SwapPaths::new(&path).journal.exists());
        assert!(
            !rebase_names(&path).is_empty(),
            "the files are left for recovery"
        );
    }

    #[test]
    fn a_rotation_refuses_to_prepare_while_a_rebase_journal_exists_and_changes_nothing() {
        let (_dir, path, mut controller) = seeded();
        let (inputs, image) = rebuilt(&controller);
        let paths = SwapPaths::new(&path);
        // The vault controller never runs both at once; a journal on disk is the file-level guard.
        fs::write(&paths.journal, b"present").unwrap();
        let envelope = fs::read(crate::rekey::RekeyPaths::new(&path).envelope).unwrap();
        let _ = (&inputs, &image);

        let error = controller.rotate_key(PASSWORD).unwrap_err();

        assert!(matches!(error, KernelError::RebaseInProgress), "{error:?}");
        assert_eq!(controller.state(), VaultState::Unlocked);
        assert_eq!(
            fs::read(crate::rekey::RekeyPaths::new(&path).envelope).unwrap(),
            envelope
        );
        assert_eq!(fs::read(&paths.journal).unwrap(), b"present");
    }

    #[test]
    fn a_key_rotation_after_a_swap_killed_before_its_journal_re_encrypts_a_base_that_still_verifies(
    ) {
        let (_dir, path, mut controller) = seeded();
        let (inputs, image) = rebuilt(&controller);
        // The kill lands with `sync_base` saying `Rebasing` and no journal.
        controller
            .swap_rebased_with(&inputs, &image, Some(CrashPoint::Frozen))
            .unwrap_err();
        drop(controller);
        let mut reopened = VaultController::open(&path);
        reopened.unlock(PASSWORD).unwrap();
        // The open settled the leftover: the base is `Ready`, the working files are gone.
        assert_eq!(
            base_status(&reopened),
            BaseImageStatus::Ready { base_seq: BASE_SEQ }
        );
        assert_no_swap_files(&path);

        reopened.rotate_key(PASSWORD).unwrap();

        assert_eq!(
            base_status(&reopened),
            BaseImageStatus::Ready { base_seq: BASE_SEQ }
        );
        assert_no_swap_files(&path);
    }

    #[test]
    fn a_rotation_removes_the_working_files_of_a_rebase_that_was_never_swapped() {
        let (_dir, path, mut controller) = seeded();
        let (_inputs, _image) = rebuilt(&controller);
        // The rebase finished and was abandoned: its files hold a copy of the vault under the key
        // being retired.
        assert!(!rebase_names(&path).is_empty());

        controller.rotate_key(PASSWORD).unwrap();

        assert_no_swap_files(&path);
        assert_eq!(
            base_status(&controller),
            BaseImageStatus::Ready { base_seq: BASE_SEQ }
        );
    }

    /// A blob file no attachment row names, as a rebase's scratch vault leaves one (a blob written
    /// for a row that only the scratch vault holds).
    fn plant_scratch_only_blob(path: &Path) -> PathBuf {
        let blobs = path.parent().unwrap().join("blobs");
        fs::create_dir_all(&blobs).unwrap();
        let blob = blobs.join("ab".repeat(32));
        fs::write(&blob, b"ciphertext only the scratch vault names").unwrap();
        blob
    }

    #[test]
    fn the_orphan_blob_sweep_stands_down_while_a_rebase_journal_is_on_disk() {
        let (_dir, path, controller) = seeded();
        let blob = plant_scratch_only_blob(&path);
        let kernel = controller.kernel().unwrap();
        // No rebase: the sweep removes the orphan.
        assert_eq!(kernel.worker.sweep_orphan_blobs().unwrap(), 1);
        assert!(!blob.exists());
        // A rebase journal (alone, or its temp file): the sweep removes nothing.
        let blob = plant_scratch_only_blob(&path);
        let paths = SwapPaths::new(&path);
        for guard in [&paths.journal, &paths.journal_tmp] {
            fs::write(guard, b"present").unwrap();
            assert_eq!(kernel.worker.sweep_orphan_blobs().unwrap(), 0);
            assert!(blob.exists(), "{}", guard.display());
            fs::remove_file(guard).unwrap();
        }
        assert_eq!(kernel.worker.sweep_orphan_blobs().unwrap(), 1);
    }

    #[test]
    fn a_crashed_rebase_with_scratch_only_blobs_then_an_unlock_deletes_nothing_and_recovery_completes(
    ) {
        for point in [CrashPoint::Committed, CrashPoint::RebuiltInstalled] {
            let (_dir, path, mut controller) = seeded();
            let (inputs, image) = rebuilt(&controller);
            let blob = plant_scratch_only_blob(&path);
            controller
                .swap_rebased_with(&inputs, &image, Some(point))
                .unwrap_err();
            drop(controller);

            let mut reopened = VaultController::open(&path);
            assert_eq!(
                reopened.rebase_recovery(),
                RebaseRecovery::RolledForward,
                "{point:?}"
            );
            // The unlock runs the sweep; `sync/pre-rebase.db` is the kept previous vault, and the
            // sweep stands down while it is there.
            reopened.unlock(PASSWORD).unwrap();

            assert!(blob.exists(), "{point:?}: the sweep deleted a blob");
            assert!(SwapPaths::new(&path).pre_rebase.exists());
            assert_no_swap_files(&path);
        }
    }

    /// The cost of the swap's own file protocol, in release, at the sizes of the base-copy
    /// measurement (personal-cfo-bxsbz.7, AC3). The three files are synthetic stand-ins of the
    /// size under test: the protocol hashes and renames files and never opens them, so their
    /// content does not change its cost. `into_swap`'s strip, vacuum and hashes (the rebase's cost)
    /// are measured with the rebase, and `begin_swap`'s hold of the writer is one checkpoint.
    ///
    /// `cargo test --release -p finance-kernel --lib measure_the_swap_cost -- --ignored --nocapture`
    #[test]
    #[ignore = "a measurement, not a check"]
    fn measure_the_swap_cost() {
        use std::time::Instant;
        for megabytes in [10_usize, 50, 200] {
            let dir = tempfile::tempdir().unwrap();
            let db = dir.path().join("vault.db");
            let sync = dir.path().join("sync");
            fs::create_dir_all(&sync).unwrap();
            let fill = |path: &Path, seed: u8| {
                let chunk = vec![seed; 1 << 20];
                let mut file = File::create(path).unwrap();
                for _ in 0..megabytes {
                    file.write_all(&chunk).unwrap();
                }
                file.sync_all().unwrap();
            };
            fill(&db, 1);
            fill(&sync.join("rebase-scratch.trial-1.db"), 2);
            fill(&sync.join("rebase-scratch.post-pull.db"), 3);
            let raw = |hex: String| -> [u8; 32] {
                let mut out = [0_u8; 32];
                for (i, byte) in out.iter_mut().enumerate() {
                    *byte = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap();
                }
                out
            };
            let inputs = SwapInputs {
                scratch_path: sync.join("rebase-scratch.trial-1.db"),
                scratch_sha256: raw(sha256_hex(&sync.join("rebase-scratch.trial-1.db")).unwrap()),
                new_base_path: sync.join("rebase-scratch.post-pull.db"),
                new_base_sha256: raw(sha256_hex(&sync.join("rebase-scratch.post-pull.db")).unwrap()),
                head_seq: 7,
                old_base_seq: 5,
            };
            let started = Instant::now();
            prepare_and_commit(&db, &inputs, None).unwrap();
            let prepared = started.elapsed();
            finish_swap(&db, None).unwrap();
            let total = started.elapsed();
            println!(
                "swap protocol at {megabytes} MB: journal + file hashes + commit {prepared:?}, \
                 apply (renames, syncs, cleanup) {:?}, total {total:?}",
                total - prepared
            );
        }
    }

    #[test]
    fn deleting_the_vault_removes_every_rebase_name() {
        let (_dir, path, mut controller) = seeded();
        let (inputs, image) = rebuilt(&controller);
        let paths = SwapPaths::new(&path);
        controller.swap_rebased(&inputs, &image).unwrap();
        fs::write(&paths.journal, b"stray").unwrap();
        fs::write(&paths.journal_tmp, b"stray").unwrap();

        controller.delete().unwrap();

        assert_eq!(controller.state(), VaultState::NoVault);
        assert!(!paths.journal.exists() && !paths.journal_tmp.exists());
        assert!(!paths.sync.exists(), "sync/ goes, with pre-rebase.db");
    }

    #[test]
    fn a_swap_names_only_files_directly_under_sync_that_a_rebase_owns() {
        let (_dir, path, mut controller) = seeded();
        let (mut inputs, image) = rebuilt(&controller);
        let outside = inputs
            .scratch_path
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("x.db");
        fs::copy(&inputs.scratch_path, &outside).unwrap();
        inputs.scratch_path = outside;

        let error = controller.swap_rebased(&inputs, &image).unwrap_err();

        assert!(matches!(error, KernelError::Vault(_)), "{error:?}");
        assert_eq!(controller.state(), VaultState::Unlocked);
        assert!(SwapPaths::new(&path).db.exists());
    }
}
