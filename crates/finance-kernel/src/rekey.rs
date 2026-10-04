//! Vault key rotation — the crash-safe protocol (ADR 0083, personal-cfo-2y8).
//!
//! Rotation replaces the vault DEK without changing the password. The live
//! vault is never converted in place: everything is prepared beside it, a
//! single journal rename commits, and an idempotent apply swaps the files.
//! Startup recovery ([`recover_interrupted_rekey`]) reads the journal and either
//! rolls the rotation back (it never committed), rolls it forward (it did), or
//! reports a contradiction and deletes nothing.
//!
//! The database work (the re-encrypted copy, attachment re-wrap and
//! re-addressing, verification) is in `db-worker`'s `rekey` module; this module
//! owns the journal, the envelope, the blob links and the file swaps.
//!
//! File family beside `vault.db` (ADR 0083 §2):
//! - `vault.db.rekey`           — the journal (`preparing` → `prepared` → `committed`)
//! - `vault.db.rekey.tmp`       — the journal's write-then-rename temp file
//! - `vault.db.rekey-new`       — the re-encrypted copy
//! - `vault.db.envelope.rekey-new` — the new envelope
//! - `vault.db.rekey-old`       — the old database, between the swap and its removal

use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use vault_crypto::{
    derive_kek, generate_dek, generate_salt, unwrap_dek, wrap_dek, Profile, VaultEnvelope,
};

use crate::{Kernel, KernelError};

/// Journal format version. Bump only if the journal's fields change meaning.
const JOURNAL_VERSION: u32 = 1;

/// Headroom required beyond one database copy (ADR 0083 §5).
const MIN_MARGIN: u64 = 64 * 1024 * 1024;

/// Every path in the rotation's file family, derived from the database path.
#[derive(Debug, Clone)]
pub(crate) struct RekeyPaths {
    pub db: PathBuf,
    pub envelope: PathBuf,
    pub journal: PathBuf,
    pub journal_tmp: PathBuf,
    pub new_db: PathBuf,
    pub new_envelope: PathBuf,
    pub old_db: PathBuf,
    pub blobs: PathBuf,
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut os = path.as_os_str().to_owned();
    os.push(suffix);
    PathBuf::from(os)
}

impl RekeyPaths {
    pub(crate) fn new(db: &Path) -> Self {
        let envelope = with_suffix(db, ".envelope");
        Self {
            db: db.to_path_buf(),
            journal: with_suffix(db, ".rekey"),
            journal_tmp: with_suffix(db, ".rekey.tmp"),
            new_db: with_suffix(db, ".rekey-new"),
            new_envelope: with_suffix(&envelope, ".rekey-new"),
            old_db: with_suffix(db, ".rekey-old"),
            blobs: db.parent().unwrap_or_else(|| Path::new(".")).join("blobs"),
            envelope,
        }
    }

    fn dir(&self) -> &Path {
        self.db.parent().unwrap_or_else(|| Path::new("."))
    }

    /// Every rotation artifact (and its SQLite side files), for vault delete
    /// and restore-attempt cleanup (ADR 0083 consequences).
    pub(crate) fn all_artifacts(&self) -> Vec<PathBuf> {
        let mut paths = vec![
            self.journal.clone(),
            self.journal_tmp.clone(),
            self.new_envelope.clone(),
        ];
        for db in [&self.new_db, &self.old_db] {
            paths.push(db.clone());
            for suffix in SIDE_SUFFIXES {
                paths.push(with_suffix(db, suffix));
            }
        }
        paths
    }
}

const SIDE_SUFFIXES: [&str; 3] = ["-wal", "-shm", "-journal"];

/// The journal's state (ADR 0083 §2). Only `Committed` makes the new vault
/// authoritative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum JournalState {
    Preparing,
    Prepared,
    Committed,
}

/// One attachment blob's old and new name, as recorded in the journal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Rename {
    old: String,
    new: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Journal {
    version: u32,
    state: JournalState,
    /// Hash of the finalized `vault.db.rekey-new` (set from `prepared` on).
    new_db_sha256: Option<String>,
    /// Hash of `vault.db.envelope.rekey-new` (set from `prepared` on).
    new_envelope_sha256: Option<String>,
    renames: Vec<Rename>,
}

impl Journal {
    fn new_names(&self) -> impl Iterator<Item = &str> {
        self.renames.iter().map(|r| r.new.as_str())
    }
    fn is_old_name(&self, name: &str) -> bool {
        self.renames.iter().any(|r| r.old == name)
    }
    fn is_new_name(&self, name: &str) -> bool {
        self.renames.iter().any(|r| r.new == name)
    }
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

fn fsync_file(path: &Path) -> Result<(), KernelError> {
    File::open(path)
        .and_then(|f| f.sync_all())
        .map_err(io("syncing rekey file"))
}

fn sha256_file(path: &Path) -> Result<String, KernelError> {
    let bytes = fs::read(path).map_err(io("hashing rekey file"))?;
    Ok(format!("{:x}", Sha256::digest(&bytes)))
}

/// Write the journal by temp file, `fsync`, rename, directory `fsync` — the
/// rename is atomic, so a crash leaves the previous journal state or the new
/// one, never a torn file (ADR 0083 §2).
fn write_journal(paths: &RekeyPaths, journal: &Journal) -> Result<(), KernelError> {
    let bytes = serde_json::to_vec(journal)
        .map_err(|error| KernelError::Vault(format!("encoding rekey journal: {error}")))?;
    let mut file = File::create(&paths.journal_tmp).map_err(io("writing rekey journal"))?;
    file.write_all(&bytes)
        .map_err(io("writing rekey journal"))?;
    file.sync_all().map_err(io("syncing rekey journal"))?;
    drop(file);
    fs::rename(&paths.journal_tmp, &paths.journal).map_err(io("installing rekey journal"))?;
    fsync_dir(paths.dir())
}

/// Read the journal: `Ok(None)` if absent, `Err` if present but unreadable.
fn read_journal(paths: &RekeyPaths) -> Result<Option<Journal>, ()> {
    match fs::read(&paths.journal) {
        Ok(bytes) => match serde_json::from_slice::<Journal>(&bytes) {
            Ok(journal) if journal.version == JOURNAL_VERSION => Ok(Some(journal)),
            _ => Err(()),
        },
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(_) => Err(()),
    }
}

/// Remove the copy (with any side files), the new envelope, and every new
/// blob link the journal lists — never a name it also lists as old — then the
/// journal. The live vault is untouched. Missing files are fine: rollback must
/// tolerate links that were never created (ADR 0083 §4).
fn roll_back(paths: &RekeyPaths, journal: Option<&Journal>) -> Result<(), KernelError> {
    remove_if_present(&paths.new_db)?;
    for suffix in SIDE_SUFFIXES {
        remove_if_present(&with_suffix(&paths.new_db, suffix))?;
    }
    remove_if_present(&paths.new_envelope)?;
    if let Some(journal) = journal {
        for name in journal.new_names() {
            if !journal.is_old_name(name) {
                remove_if_present(&paths.blobs.join(name))?;
            }
        }
    }
    remove_if_present(&paths.journal_tmp)?;
    remove_if_present(&paths.journal)?;
    fsync_dir(paths.dir())
}

/// Whether the files on disk agree with a `committed` journal. Every check is
/// "already applied, or ready to apply", so the same test holds at every apply
/// step and a contradiction is reported instead of guessed at.
fn committed_files_agree(paths: &RekeyPaths, journal: &Journal) -> Result<bool, KernelError> {
    let (Some(db_hash), Some(envelope_hash)) =
        (&journal.new_db_sha256, &journal.new_envelope_sha256)
    else {
        return Ok(false);
    };
    let db_ok = if paths.new_db.exists() {
        &sha256_file(&paths.new_db)? == db_hash
    } else {
        paths.db.exists() && &sha256_file(&paths.db)? == db_hash
    };
    let envelope_ok = if paths.new_envelope.exists() {
        &sha256_file(&paths.new_envelope)? == envelope_hash
    } else {
        paths.envelope.exists() && &sha256_file(&paths.envelope)? == envelope_hash
    };
    let blobs_ok = journal
        .new_names()
        .all(|name| paths.blobs.join(name).is_file());
    Ok(db_ok && envelope_ok && blobs_ok)
}

/// The apply steps after the commit point (ADR 0083 §2), each idempotent so
/// recovery can re-run them from any interruption. `stop` is the test-only
/// crash hook.
fn apply_committed(
    paths: &RekeyPaths,
    journal: &Journal,
    stop: Option<CrashPoint>,
) -> Result<(), KernelError> {
    // 2. Old side files go before the swap. While the journal exists the new
    //    database has never been opened, so any side file belongs to the old one.
    remove_if_present(&with_suffix(&paths.db, "-wal"))?;
    remove_if_present(&with_suffix(&paths.db, "-shm"))?;
    crash(stop, CrashPoint::OldSideFilesRemoved)?;
    // 3. Swap the database.
    if paths.new_db.exists() {
        if paths.db.exists() {
            fs::rename(&paths.db, &paths.old_db).map_err(io("moving old database aside"))?;
            crash(stop, CrashPoint::OldDbMovedAside)?;
        }
        fs::rename(&paths.new_db, &paths.db).map_err(io("installing rekeyed database"))?;
        fsync_dir(paths.dir())?;
    }
    crash(stop, CrashPoint::DbSwapped)?;
    // 4. Swap the envelope.
    if paths.new_envelope.exists() {
        fs::rename(&paths.new_envelope, &paths.envelope).map_err(io("installing new envelope"))?;
        fsync_dir(paths.dir())?;
    }
    crash(stop, CrashPoint::EnvelopeSwapped)?;
    // 5. Old blob names, never one also listed as new.
    for rename in &journal.renames {
        if !journal.is_new_name(&rename.old) {
            remove_if_present(&paths.blobs.join(&rename.old))?;
        }
    }
    if paths.blobs.exists() {
        fsync_dir(&paths.blobs)?;
    }
    crash(stop, CrashPoint::OldBlobsRemoved)?;
    // 6. The old database.
    remove_if_present(&paths.old_db)?;
    crash(stop, CrashPoint::OldDbRemoved)?;
    // 7. The journal, last.
    remove_if_present(&paths.journal_tmp)?;
    remove_if_present(&paths.journal)?;
    fsync_dir(paths.dir())
}

/// What startup recovery did with an interrupted rotation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RekeyRecovery {
    /// No rotation was in progress.
    Clean,
    /// An uncommitted rotation was undone; the old vault stands.
    RolledBack,
    /// A committed rotation was finished; the new vault stands.
    RolledForward,
    /// The journal and the files disagree; nothing was deleted.
    Contradiction,
}

/// Startup recovery for an interrupted rotation (ADR 0083 §4). Runs before the
/// vault is classified; never opens the database.
#[must_use]
pub fn recover_interrupted_rekey(db_path: &Path) -> RekeyRecovery {
    let paths = RekeyPaths::new(db_path);
    match read_journal(&paths) {
        Err(()) => RekeyRecovery::Contradiction,
        Ok(None) => {
            // A prepare that crashed before its `preparing` journal: only the
            // copy and the journal temp file can exist; no blob link can.
            let stray = paths.new_db.exists()
                || paths.new_envelope.exists()
                || paths.journal_tmp.exists()
                || SIDE_SUFFIXES
                    .iter()
                    .any(|s| with_suffix(&paths.new_db, s).exists());
            if !stray {
                return RekeyRecovery::Clean;
            }
            match roll_back(&paths, None) {
                Ok(()) => RekeyRecovery::RolledBack,
                Err(_) => RekeyRecovery::Contradiction,
            }
        }
        Ok(Some(journal)) => match journal.state {
            JournalState::Preparing | JournalState::Prepared => {
                match roll_back(&paths, Some(&journal)) {
                    Ok(()) => RekeyRecovery::RolledBack,
                    Err(_) => RekeyRecovery::Contradiction,
                }
            }
            JournalState::Committed => match committed_files_agree(&paths, &journal) {
                Ok(true) => match apply_committed(&paths, &journal, None) {
                    Ok(()) => RekeyRecovery::RolledForward,
                    Err(_) => RekeyRecovery::Contradiction,
                },
                _ => RekeyRecovery::Contradiction,
            },
        },
    }
}

/// Whether a rotation journal is present.
#[must_use]
pub(crate) fn journal_present(db_path: &Path) -> bool {
    let paths = RekeyPaths::new(db_path);
    paths.journal.exists()
}

/// Test-only crash points: each names the last step completed. The rotation
/// stops right after it, leaving files exactly as a process death would.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CrashPoint {
    Copied,
    PreparingJournalWritten,
    Linked,
    NewEnvelopeWritten,
    PreparedJournalWritten,
    Committed,
    OldSideFilesRemoved,
    OldDbMovedAside,
    DbSwapped,
    EnvelopeSwapped,
    OldBlobsRemoved,
    OldDbRemoved,
}

/// Marker error for a simulated crash; never produced outside tests.
const SIMULATED_CRASH: &str = "simulated crash";

fn crash(stop: Option<CrashPoint>, here: CrashPoint) -> Result<(), KernelError> {
    if stop == Some(here) {
        return Err(KernelError::Vault(SIMULATED_CRASH.to_owned()));
    }
    Ok(())
}

pub(crate) fn is_simulated_crash(error: &KernelError) -> bool {
    matches!(error, KernelError::Vault(message) if message == SIMULATED_CRASH)
}

/// Hard-link `from` to `to`, falling back to a copy where links are not
/// supported. `to` must not exist.
fn link_or_copy(from: &Path, to: &Path) -> Result<(), KernelError> {
    match fs::hard_link(from, to) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => Err(KernelError::Vault(
            "rekey blob link target already exists".to_owned(),
        )),
        Err(_) => {
            fs::copy(from, to).map_err(io("copying blob for rekey"))?;
            fsync_file(to)
        }
    }
}

/// Whether the blob directory supports hard links (ADR 0083 §5 probe).
fn hard_links_supported(blobs: &Path) -> bool {
    let Ok(entries) = fs::read_dir(blobs) else {
        return true; // no blobs: nothing to copy either way
    };
    let Some(sample) = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| p.is_file() && p.extension().is_none())
    else {
        return true;
    };
    let probe = blobs.join("rekey-probe.tmp");
    let _ = fs::remove_file(&probe);
    let ok = fs::hard_link(&sample, &probe).is_ok();
    let _ = fs::remove_file(&probe);
    ok
}

/// Space needed for a rotation (ADR 0083 §5): one database copy, a margin, and
/// the blob bytes when hard links are unavailable.
fn required_space(db_size: u64, blobs_size: u64, links: bool) -> u64 {
    let margin = MIN_MARGIN.max(db_size / 10);
    let blobs = if links { 0 } else { blobs_size };
    db_size.saturating_add(margin).saturating_add(blobs)
}

/// Refuse a rotation that would not fit (ADR 0083 §5). Nothing has been
/// written when this fails.
fn check_space(needed: u64, available: u64) -> Result<(), KernelError> {
    if available < needed {
        return Err(KernelError::InsufficientDiskSpace { needed, available });
    }
    Ok(())
}

/// Prepare and commit a rotation of `kernel`'s vault (ADR 0083 §2). On success
/// the journal is `committed` and the caller must drop the kernel and run
/// [`finish_rotation`]. On failure everything prepared is removed and the live
/// vault is untouched; `KernelError::VaultUnlockFailed` means the password was
/// wrong and nothing was written.
pub(crate) fn prepare_and_commit(
    kernel: &Kernel,
    password: &[u8],
    stop: Option<CrashPoint>,
) -> Result<(), KernelError> {
    let paths = RekeyPaths::new(kernel.worker.db_path());

    // The password must open the current envelope before anything is written.
    let current =
        VaultEnvelope::from_bytes(&fs::read(&paths.envelope).map_err(io("reading envelope"))?)?;
    let old_kek = derive_kek(password, &current.salt, &current.kdf)?;
    drop(unwrap_dek(&old_kek, &current.wrapped)?);
    drop(old_kek);

    if paths.journal.exists() || paths.new_db.exists() || paths.new_envelope.exists() {
        return Err(KernelError::Vault(
            "a previous key rotation was not cleaned up; reopen the vault first".to_owned(),
        ));
    }

    let db_size = kernel.worker.database_size()?;
    let blobs_size = kernel.worker.blobs_size()?;
    let needed = required_space(db_size, blobs_size, hard_links_supported(&paths.blobs));
    check_space(needed, db_worker::available_space(paths.dir())?)?;

    let result = prepare_inner(kernel, password, &paths, stop);
    if let Err(error) = &result {
        if !is_simulated_crash(error) {
            let journal = read_journal(&paths).ok().flatten();
            let _ = roll_back(&paths, journal.as_ref());
        }
    }
    result
}

fn prepare_inner(
    kernel: &Kernel,
    password: &[u8],
    paths: &RekeyPaths,
    stop: Option<CrashPoint>,
) -> Result<(), KernelError> {
    let new_dek = generate_dek()?;
    let params = Profile::InteractiveDefault.params();
    let salt = generate_salt()?;
    let new_kek = derive_kek(password, &salt, &params)?;
    let envelope = VaultEnvelope::new(params, salt, wrap_dek(&new_kek, &new_dek)?);
    drop(new_kek);

    let user_version = kernel.worker.user_version()?;
    let renames = kernel
        .worker
        .prepare_rekeyed_copy(&new_dek, &paths.new_db)?
        .into_iter()
        .map(|r| Rename {
            old: r.old,
            new: r.new,
        })
        .collect::<Vec<_>>();
    crash(stop, CrashPoint::Copied)?;

    let mut journal = Journal {
        version: JOURNAL_VERSION,
        state: JournalState::Preparing,
        new_db_sha256: None,
        new_envelope_sha256: None,
        renames,
    };
    write_journal(paths, &journal)?;
    crash(stop, CrashPoint::PreparingJournalWritten)?;

    for rename in &journal.renames {
        link_or_copy(
            &paths.blobs.join(&rename.old),
            &paths.blobs.join(&rename.new),
        )?;
    }
    if paths.blobs.exists() {
        fsync_dir(&paths.blobs)?;
    }
    crash(stop, CrashPoint::Linked)?;

    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&paths.new_envelope)
        .map_err(io("writing new envelope"))?;
    file.write_all(&envelope.to_bytes())
        .map_err(io("writing new envelope"))?;
    file.sync_all().map_err(io("syncing new envelope"))?;
    drop(file);
    crash(stop, CrashPoint::NewEnvelopeWritten)?;

    db_worker::verify_rekeyed_copy(&paths.new_db, &new_dek, &paths.blobs, user_version)?;
    fsync_file(&paths.new_db)?;
    fsync_dir(paths.dir())?;

    journal.state = JournalState::Prepared;
    journal.new_db_sha256 = Some(sha256_file(&paths.new_db)?);
    journal.new_envelope_sha256 = Some(sha256_file(&paths.new_envelope)?);
    write_journal(paths, &journal)?;
    crash(stop, CrashPoint::PreparedJournalWritten)?;

    // The commit point (ADR 0083 §2): one journal rename.
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

/// Apply a committed rotation after the caller has dropped the kernel (so no
/// connection or runner lock remains on the old database).
pub(crate) fn finish_rotation(db_path: &Path, stop: Option<CrashPoint>) -> Result<(), KernelError> {
    let paths = RekeyPaths::new(db_path);
    let journal = match read_journal(&paths) {
        Ok(Some(journal)) if journal.state == JournalState::Committed => journal,
        _ => {
            return Err(KernelError::Vault(
                "key rotation journal is not committed".to_owned(),
            ))
        }
    };
    if !committed_files_agree(&paths, &journal)? {
        return Err(KernelError::Vault(
            "key rotation files do not match the journal".to_owned(),
        ));
    }
    apply_committed(&paths, &journal, stop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_space_adds_margin_and_blob_bytes_only_without_links() {
        assert_eq!(required_space(10, 1_000, true), 10 + MIN_MARGIN);
        assert_eq!(required_space(10, 1_000, false), 10 + MIN_MARGIN + 1_000);
        let big = 10 * MIN_MARGIN;
        assert_eq!(required_space(big, 0, true), big + big / 10);
    }

    #[test]
    fn check_space_refuses_when_free_space_is_short() {
        assert!(matches!(
            check_space(100, 99),
            Err(KernelError::InsufficientDiskSpace {
                needed: 100,
                available: 99
            })
        ));
        assert!(check_space(100, 100).is_ok());
    }

    #[test]
    fn rekey_paths_name_the_documented_file_family() {
        let paths = RekeyPaths::new(Path::new("/v/vault.db"));
        assert_eq!(paths.journal, Path::new("/v/vault.db.rekey"));
        assert_eq!(paths.journal_tmp, Path::new("/v/vault.db.rekey.tmp"));
        assert_eq!(paths.new_db, Path::new("/v/vault.db.rekey-new"));
        assert_eq!(
            paths.new_envelope,
            Path::new("/v/vault.db.envelope.rekey-new")
        );
        assert_eq!(paths.old_db, Path::new("/v/vault.db.rekey-old"));
        assert_eq!(paths.blobs, Path::new("/v/blobs"));
    }
}

/// End-to-end protocol tests through [`VaultController`], including one per
/// crash point (ADR 0083 §4).
#[cfg(test)]
mod protocol_tests {
    use super::*;
    use crate::vault::VaultController;
    use crate::{
        Account, AccountFlags, AccountId, ActorType, CashflowRole, CommandEnvelope, CommandMeta,
        CreateAccount, Currency, LedgerAccountId, Money, RecordTransaction, TransactionId,
        VaultState,
    };
    use uuid::Uuid;

    const PASSWORD: &[u8] = b"rotation test password";
    const PDF: &[u8] = b"%PDF-1.4\nrotation attachment\n%%EOF\n";

    fn meta(index: u8) -> CommandMeta {
        CommandMeta {
            command_id: Uuid::from_bytes([index; 16]),
            correlation_id: Uuid::from_bytes([index; 16]),
            causation_id: None,
            actor_type: ActorType::User,
            actor_id: "rekey-test".into(),
            idempotency_key: format!("rekey-test-{index}"),
        }
    }

    /// A vault with an account, a transaction and an attachment, left unlocked.
    fn seeded() -> (
        tempfile::TempDir,
        PathBuf,
        VaultController,
        crate::AttachmentId,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.db");
        let mut controller = VaultController::open(&path);
        controller.create(PASSWORD).unwrap();
        let kernel = controller.kernel().unwrap();
        let account = AccountId::from_uuid(Uuid::from_bytes([1; 16]));
        kernel
            .dispatch(CommandEnvelope::new(
                meta(2),
                CreateAccount::with_opening_balance(
                    Account::new(
                        account,
                        LedgerAccountId::from_uuid(Uuid::from_bytes([3; 16])),
                        "Rotation Checking",
                        CashflowRole::LiquidCash,
                        Currency::Usd,
                        AccountFlags::default(),
                    ),
                    Money::new(100_000, Currency::Usd),
                ),
            ))
            .unwrap();
        let txn = TransactionId::from_uuid(Uuid::from_bytes([4; 16]));
        kernel
            .dispatch(CommandEnvelope::new(
                meta(5),
                RecordTransaction::new(
                    txn,
                    account,
                    Money::new(-2_500, Currency::Usd),
                    "2026-10-01T00:00:00Z".parse().unwrap(),
                ),
            ))
            .unwrap();
        let attachment = kernel
            .attach_document(txn, PDF, Some("application/pdf"), Some("r.pdf"))
            .unwrap()
            .id;
        (dir, path, controller, attachment)
    }

    fn canonical(controller: &VaultController) -> String {
        let kernel = controller.kernel().unwrap();
        format!(
            "{:?}|{:?}|{}",
            kernel.account_views().unwrap(),
            kernel.transactions(100).unwrap(),
            kernel.operation_count().unwrap()
        )
    }

    fn blob_names(path: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(RekeyPaths::new(path).blobs)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn assert_no_artifacts(path: &Path) {
        for artifact in RekeyPaths::new(path).all_artifacts() {
            assert!(!artifact.exists(), "left behind: {}", artifact.display());
        }
        assert!(!RekeyPaths::new(path).blobs.join("rekey-probe.tmp").exists());
    }

    #[test]
    fn rotation_keeps_data_changes_the_key_and_ends_unlocked() {
        let (dir, path, mut controller, attachment) = seeded();
        let before = canonical(&controller);
        let old_envelope = fs::read(RekeyPaths::new(&path).envelope).unwrap();
        let old_blobs = blob_names(&path);

        controller.rotate_key(PASSWORD).unwrap();
        assert_eq!(controller.state(), VaultState::Unlocked);
        assert_eq!(canonical(&controller), before);
        assert_eq!(
            controller
                .kernel()
                .unwrap()
                .read_attachment_bytes(attachment)
                .unwrap(),
            PDF
        );
        assert!(controller.health_check().unwrap().is_healthy());
        assert_no_artifacts(&path);
        assert_ne!(
            fs::read(RekeyPaths::new(&path).envelope).unwrap(),
            old_envelope
        );
        let new_blobs = blob_names(&path);
        assert_eq!(new_blobs.len(), old_blobs.len());
        assert!(
            new_blobs.iter().all(|n| !old_blobs.contains(n)),
            "every blob renamed"
        );

        // Lock + unlock with the unchanged password reads the same plaintext.
        controller.lock().unwrap();
        controller.unlock(PASSWORD).unwrap();
        assert_eq!(canonical(&controller), before);
        assert_eq!(
            controller
                .kernel()
                .unwrap()
                .read_attachment_bytes(attachment)
                .unwrap(),
            PDF
        );
        controller.lock().unwrap();

        // The old DEK no longer opens the database: pair the rotated vault.db
        // with the old envelope and unlock with the (correct) password.
        let other = dir.path().join("old-key");
        fs::create_dir_all(&other).unwrap();
        fs::copy(&path, other.join("vault.db")).unwrap();
        fs::write(other.join("vault.db.envelope"), old_envelope).unwrap();
        assert!(Kernel::unlock_vault(other.join("vault.db"), PASSWORD).is_err());
    }

    #[test]
    fn a_wrong_password_writes_nothing_and_stays_unlocked() {
        let (dir, path, mut controller, _) = seeded();
        let before: Vec<_> = {
            let mut v: Vec<_> = fs::read_dir(dir.path())
                .unwrap()
                .map(|e| e.unwrap().file_name())
                .collect();
            v.sort();
            v
        };
        let envelope = fs::read(RekeyPaths::new(&path).envelope).unwrap();
        assert!(matches!(
            controller.rotate_key(b"not the password"),
            Err(KernelError::VaultUnlockFailed)
        ));
        assert_eq!(controller.state(), VaultState::Unlocked);
        assert!(controller.kernel().is_some());
        let mut after: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        after.sort();
        assert_eq!(after, before);
        assert_eq!(fs::read(RekeyPaths::new(&path).envelope).unwrap(), envelope);
    }

    #[test]
    fn rotation_requires_an_unlocked_vault() {
        let (_dir, _path, mut controller, _) = seeded();
        controller.lock().unwrap();
        assert!(matches!(
            controller.rotate_key(PASSWORD),
            Err(KernelError::IllegalVaultTransition { .. })
        ));
    }

    #[test]
    fn rotation_moves_a_legacy_kdf_vault_to_the_current_profile() {
        let (_dir, path, mut controller, _) = seeded();
        controller.lock().unwrap();
        let envelope_path = RekeyPaths::new(&path).envelope;
        let current = VaultEnvelope::from_bytes(&fs::read(&envelope_path).unwrap()).unwrap();
        let legacy = vault_crypto::rewrap_envelope(
            &current,
            PASSWORD,
            PASSWORD,
            Profile::LegacyCompatibility.params(),
        )
        .unwrap();
        fs::write(&envelope_path, legacy.to_bytes()).unwrap();
        controller.unlock(PASSWORD).unwrap();

        controller.rotate_key(PASSWORD).unwrap();
        let rotated = VaultEnvelope::from_bytes(&fs::read(&envelope_path).unwrap()).unwrap();
        assert_eq!(rotated.kdf, Profile::InteractiveDefault.params());
        assert_ne!(rotated.salt.as_bytes(), legacy.salt.as_bytes());
    }

    #[test]
    fn backups_before_and_after_rotation_both_restore() {
        let (dir, path, mut controller, attachment) = seeded();
        let before = canonical(&controller);
        let old_backup = dir.path().join("before.pcfobk");
        controller
            .kernel()
            .unwrap()
            .export_unattended(
                &old_backup,
                "t",
                "2026-10-04T00:00:00Z".into(),
                Uuid::from_bytes([7; 16]),
            )
            .unwrap();
        controller.rotate_key(PASSWORD).unwrap();
        let new_backup = dir.path().join("after.pcfobk");
        controller
            .kernel()
            .unwrap()
            .export_unattended(
                &new_backup,
                "t",
                "2026-10-04T00:00:01Z".into(),
                Uuid::from_bytes([8; 16]),
            )
            .unwrap();
        for (name, package) in [("a", &old_backup), ("b", &new_backup)] {
            let target = dir.path().join(name);
            fs::create_dir_all(&target).unwrap();
            let restored =
                Kernel::restore_backup(package, PASSWORD, &target.join("vault.db")).unwrap();
            assert_eq!(restored.read_attachment_bytes(attachment).unwrap(), PDF);
            assert_eq!(
                format!(
                    "{:?}|{:?}|{}",
                    restored.account_views().unwrap(),
                    restored.transactions(100).unwrap(),
                    restored.operation_count().unwrap()
                ),
                before
            );
        }
        let _ = path;
    }

    const BEFORE_COMMIT: [CrashPoint; 5] = [
        CrashPoint::Copied,
        CrashPoint::PreparingJournalWritten,
        CrashPoint::Linked,
        CrashPoint::NewEnvelopeWritten,
        CrashPoint::PreparedJournalWritten,
    ];
    const AFTER_COMMIT: [CrashPoint; 7] = [
        CrashPoint::Committed,
        CrashPoint::OldSideFilesRemoved,
        CrashPoint::OldDbMovedAside,
        CrashPoint::DbSwapped,
        CrashPoint::EnvelopeSwapped,
        CrashPoint::OldBlobsRemoved,
        CrashPoint::OldDbRemoved,
    ];

    /// Crash at `point`, "restart" (drop everything, reopen), and return the
    /// reopened controller plus what recovery did.
    fn crash_and_reopen(
        point: CrashPoint,
    ) -> (
        tempfile::TempDir,
        PathBuf,
        VaultController,
        crate::AttachmentId,
        String,
        Vec<u8>,
        Vec<String>,
    ) {
        let (dir, path, mut controller, attachment) = seeded();
        let before = canonical(&controller);
        let envelope = fs::read(RekeyPaths::new(&path).envelope).unwrap();
        let blobs = blob_names(&path);
        let error = controller
            .rotate_key_with(PASSWORD, Some(point))
            .unwrap_err();
        assert!(is_simulated_crash(&error), "{point:?}: {error}");
        drop(controller);
        let reopened = VaultController::open(&path);
        let _ = (envelope.len(), blobs.len());
        (dir, path, reopened, attachment, before, envelope, blobs)
    }

    #[test]
    fn a_crash_before_the_commit_point_rolls_back_to_the_old_vault() {
        for point in BEFORE_COMMIT {
            let (_dir, path, mut controller, attachment, before, envelope, blobs) =
                crash_and_reopen(point);
            assert_eq!(
                controller.rekey_recovery(),
                RekeyRecovery::RolledBack,
                "{point:?}"
            );
            assert_eq!(controller.state(), VaultState::Locked, "{point:?}");
            assert_no_artifacts(&path);
            assert_eq!(fs::read(RekeyPaths::new(&path).envelope).unwrap(), envelope);
            assert_eq!(blob_names(&path), blobs, "{point:?}: old names only");
            controller.unlock(PASSWORD).unwrap();
            assert_eq!(canonical(&controller), before, "{point:?}");
            assert_eq!(
                controller
                    .kernel()
                    .unwrap()
                    .read_attachment_bytes(attachment)
                    .unwrap(),
                PDF
            );
        }
    }

    #[test]
    fn a_crash_after_the_commit_point_rolls_forward_to_the_new_vault() {
        for point in AFTER_COMMIT {
            let (_dir, path, mut controller, attachment, before, envelope, blobs) =
                crash_and_reopen(point);
            assert_eq!(
                controller.rekey_recovery(),
                RekeyRecovery::RolledForward,
                "{point:?}"
            );
            assert_eq!(controller.state(), VaultState::Locked, "{point:?}");
            assert_no_artifacts(&path);
            assert_ne!(fs::read(RekeyPaths::new(&path).envelope).unwrap(), envelope);
            let now = blob_names(&path);
            assert_eq!(now.len(), blobs.len(), "{point:?}");
            assert!(now.iter().all(|n| !blobs.contains(n)), "{point:?}: renamed");
            controller.unlock(PASSWORD).unwrap();
            assert_eq!(canonical(&controller), before, "{point:?}");
            assert_eq!(
                controller
                    .kernel()
                    .unwrap()
                    .read_attachment_bytes(attachment)
                    .unwrap(),
                PDF
            );
            assert!(controller.health_check().unwrap().is_healthy(), "{point:?}");
        }
    }

    #[test]
    fn a_committed_journal_contradicted_by_the_files_is_corrupt_and_nothing_is_deleted() {
        let (_dir, path, mut controller, _) = seeded();
        let error = controller
            .rotate_key_with(PASSWORD, Some(CrashPoint::Committed))
            .unwrap_err();
        assert!(is_simulated_crash(&error));
        drop(controller);
        let paths = RekeyPaths::new(&path);
        fs::write(&paths.new_db, b"tampered").unwrap();
        let reopened = VaultController::open(&path);
        assert_eq!(reopened.rekey_recovery(), RekeyRecovery::Contradiction);
        assert_eq!(reopened.state(), VaultState::CorruptNeedsRecovery);
        for kept in [
            &paths.journal,
            &paths.new_db,
            &paths.new_envelope,
            &paths.db,
            &paths.envelope,
        ] {
            assert!(kept.exists(), "kept {}", kept.display());
        }
    }

    #[test]
    fn an_unreadable_journal_is_corrupt_and_nothing_is_deleted() {
        let (_dir, path, mut controller, _) = seeded();
        controller.lock().unwrap();
        let paths = RekeyPaths::new(&path);
        fs::write(&paths.journal, b"{not json").unwrap();
        fs::write(&paths.new_db, b"copy").unwrap();
        let reopened = VaultController::open(&path);
        assert_eq!(reopened.rekey_recovery(), RekeyRecovery::Contradiction);
        assert_eq!(reopened.state(), VaultState::CorruptNeedsRecovery);
        assert!(paths.journal.exists() && paths.new_db.exists());
    }

    #[test]
    fn no_journal_recovery_removes_stray_prepare_files_and_never_sweeps_blobs() {
        let (_dir, path, mut controller, _) = seeded();
        controller.lock().unwrap();
        let paths = RekeyPaths::new(&path);
        let blobs = blob_names(&path);
        fs::write(&paths.new_db, b"copy").unwrap();
        fs::write(with_suffix(&paths.new_db, "-journal"), b"j").unwrap();
        fs::write(&paths.journal_tmp, b"{").unwrap();
        // An unreferenced file in blobs/ is not prepare's to remove without a journal.
        fs::write(paths.blobs.join("ffff"), b"unrelated").unwrap();
        let reopened = VaultController::open(&path);
        assert_eq!(reopened.rekey_recovery(), RekeyRecovery::RolledBack);
        assert_eq!(reopened.state(), VaultState::Locked);
        assert_no_artifacts(&path);
        assert!(paths.blobs.join("ffff").exists(), "no blob-directory sweep");
        let mut expected = blobs;
        expected.push("ffff".into());
        expected.sort();
        assert_eq!(blob_names(&path), expected);
    }

    /// A real (not simulated) failure during prepare removes everything prepared
    /// and leaves the vault Unlocked on the old DEK.
    #[cfg(unix)]
    #[test]
    fn a_real_prepare_failure_rolls_back_and_stays_unlocked() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, path, mut controller, attachment) = seeded();
        let before = canonical(&controller);
        let paths = RekeyPaths::new(&path);
        let envelope = fs::read(&paths.envelope).unwrap();
        let blobs = blob_names(&path);
        // A read-only blob directory makes both the link and the copy fallback fail.
        fs::set_permissions(&paths.blobs, fs::Permissions::from_mode(0o555)).unwrap();
        let result = controller.rotate_key(PASSWORD);
        fs::set_permissions(&paths.blobs, fs::Permissions::from_mode(0o755)).unwrap();
        let error = result.unwrap_err();
        assert!(!is_simulated_crash(&error));
        assert_eq!(controller.state(), VaultState::Unlocked);
        assert_no_artifacts(&path);
        assert_eq!(fs::read(&paths.envelope).unwrap(), envelope);
        assert_eq!(blob_names(&path), blobs);
        assert_eq!(canonical(&controller), before);
        assert_eq!(
            controller
                .kernel()
                .unwrap()
                .read_attachment_bytes(attachment)
                .unwrap(),
            PDF
        );
        // And a retry after the cause is fixed succeeds.
        controller.rotate_key(PASSWORD).unwrap();
        assert_eq!(canonical(&controller), before);
    }

    #[test]
    fn delete_removes_rotation_artifacts() {
        let (_dir, path, mut controller, _) = seeded();
        let paths = RekeyPaths::new(&path);
        fs::write(&paths.old_db, b"x").unwrap();
        fs::write(&paths.journal_tmp, b"x").unwrap();
        controller.delete().unwrap();
        assert_eq!(controller.state(), VaultState::NoVault);
        assert_no_artifacts(&path);
    }
}
