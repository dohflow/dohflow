//! The base image store (personal-cfo-bxsbz.3; ADR 0074 decision 2, ADR 0083 Addendum C).
//!
//! A rebase restores the vault as of `sync_base.base_seq` and replays on top of it (bxsbz.4). That
//! restore point is `<vault dir>/sync/base.db`: a copy of `vault.db`, encrypted under the same DEK,
//! taken only when this device has nothing unpushed. This module owns the file and the one record
//! that vouches for it; the rebase, the swap and the pre-rebase copy that a swap leaves behind
//! belong to later children of the bead (the swap writes `sync/pre-rebase.db`, this module only
//! scrubs and re-keys it).
//!
//! **Taking and advancing the base.** [`DbWorker::initialize_base_image`] makes the first base (a
//! bootstrapped replica, or the first time Sync is enabled) and [`DbWorker::advance_base_image`]
//! moves it forward. Both run one critical section under the writer lock: check that the outbox is
//! empty, fold the WAL, stream the checkpointed file to a temp file, strip the secrets from the
//! temp file, hash it, rename it into place and record `base_seq`, `base_digest` and the `Ready`
//! state in `sync_base`. The vault is never read into memory.
//!
//! **No credential in the base.** Every table the CLASS-0 manifest marks `secret_bearing` is
//! emptied in the copy and the copy is vacuumed, so a credential removed from `vault.db` leaves no
//! stale copy in `base.db` (the base's class-3 rows are never authoritative; the rebase overlays
//! the live ones). [`SECRET_BEARING_TABLES`] is that set in code; `tests/table_classes.rs` keeps it
//! equal to the manifest.
//!
//! **A base the vault can vouch for.** `sync_base.base_digest` is the SHA-256 of the file. A
//! backup carries `vault.db` (and so the `sync_base` row) but not `sync/`, so a restored vault
//! finds a digest with no matching file. On every open [`DbWorker::base_image_status`] is checked
//! and a missing or mismatched base is recorded as `NoBase`: the vault never rebases on it and
//! goes to the pull-first rebootstrap with its unpushed work exported (elciw AC6, R39).
//!
//! **Rekey.** A key rotation re-encrypts `base.db` and `pre-rebase.db` inside the rekey journal
//! (see [`DbWorker::prepare_rekeyed_images`]), so no copy under the retired DEK survives.

use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::Utc;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use vault_crypto::Dek;

use crate::{DbError, DbWorker, KeyMaterial, WorkerState};

/// The tables whose rows are credentials: emptied from every base image at capture. This is the
/// set the CLASS-0 manifest marks `secret_bearing = "true"`; `tests/table_classes.rs` fails if
/// the two differ, so adding a credential holder is a reviewed change to both.
pub const SECRET_BEARING_TABLES: &[&str] = &["connector_connection_secrets"];

/// The directory beside `vault.db` that holds the Sync images.
pub const SYNC_DIR: &str = "sync";
/// The base image: the vault as of `sync_base.base_seq`, credentials stripped.
pub const BASE_IMAGE_FILE: &str = "base.db";
/// The copy of the live vault a rebase swap keeps until the next accepted push (bxsbz.7).
pub const PRE_REBASE_FILE: &str = "pre-rebase.db";
/// Suffix of the re-encrypted copy of an image while a key rotation is in flight.
pub const REKEY_NEW_SUFFIX: &str = ".rekey-new";

/// The Sync image directory for the vault at `vault_path`.
#[must_use]
pub fn sync_dir(vault_path: &Path) -> PathBuf {
    vault_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(SYNC_DIR)
}

/// Where the base image of the vault at `vault_path` lives.
#[must_use]
pub fn base_image_path(vault_path: &Path) -> PathBuf {
    sync_dir(vault_path).join(BASE_IMAGE_FILE)
}

/// Where the pre-rebase copy of the vault at `vault_path` lives.
#[must_use]
pub fn pre_rebase_path(vault_path: &Path) -> PathBuf {
    sync_dir(vault_path).join(PRE_REBASE_FILE)
}

/// The `sync_base.state` column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncBaseState {
    /// No usable image: none was ever captured, or the one on disk is missing or does not match
    /// the recorded digest.
    NoBase,
    /// `base.db` is the vault as of `base_seq`.
    Ready,
    /// A rebase swap is in flight (bxsbz.7).
    Rebasing,
}

impl SyncBaseState {
    fn parse(text: &str) -> Result<Self, DbError> {
        match text {
            "NoBase" => Ok(Self::NoBase),
            "Ready" => Ok(Self::Ready),
            "Rebasing" => Ok(Self::Rebasing),
            // The column's CHECK rules this out; an unknown word is a newer build's state.
            _ => Err(DbError::UnsupportedSchema),
        }
    }
}

/// What this device has for a rebase to restore.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaseImageStatus {
    /// Sync is not enabled on this device (no `sync_base` row).
    SyncOff,
    /// There is no usable base: rebootstrap pull-first.
    NoBase,
    /// `base.db` exists, matches the recorded digest, and is the vault as of `base_seq`.
    Ready {
        /// The server seq the image corresponds to.
        base_seq: i64,
    },
    /// A rebase swap is in flight; its recovery owns the files.
    Rebasing {
        /// The server seq of the base the swap started from.
        base_seq: i64,
    },
}

/// Why a base image was not taken or advanced. Nothing is written on any of these.
#[derive(Debug, thiserror::Error)]
pub enum BaseImageError {
    /// [`DbWorker::advance_base_image`] on a device that never enabled Sync.
    #[error("Sync is not enabled on this device")]
    SyncNotEnabled,
    /// [`DbWorker::initialize_base_image`] when a base image already exists.
    #[error("a base image already exists")]
    BaseAlreadyExists,
    /// [`DbWorker::advance_base_image`] when there is no usable base to advance (it was lost, or a
    /// swap is in flight): the vault rebootstraps instead.
    #[error("there is no usable base image to advance")]
    NoUsableBase(SyncBaseState),
    /// Unpushed commands remain: a base taken now would contain state the server has not seen.
    #[error("{pending} unpushed command(s) remain in the outbox")]
    OutboxNotEmpty {
        /// How many `sync_outbox` rows exist.
        pending: u64,
    },
    /// The base only moves forward.
    #[error("the base cannot move back from seq {current} to {requested}")]
    SeqWentBack {
        /// The recorded `base_seq`.
        current: i64,
        /// The seq the caller asked for.
        requested: i64,
    },
    /// A server seq is never negative.
    #[error("a base seq cannot be negative")]
    NegativeSeq,
    /// The database or the filesystem failed.
    #[error(transparent)]
    Db(#[from] DbError),
}

impl From<rusqlite::Error> for BaseImageError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Db(error.into())
    }
}

impl From<io::Error> for BaseImageError {
    fn from(error: io::Error) -> Self {
        Self::Db(error.into())
    }
}

/// A base image that was taken, with what it cost (the per-push copy cost bxsbz.7 budgets).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseCapture {
    /// The server seq the image corresponds to.
    pub base_seq: i64,
    /// SHA-256 of `base.db`, as recorded in `sync_base.base_digest`.
    pub digest: [u8; 32],
    /// Size of `base.db` after the strip.
    pub image_bytes: u64,
    /// Time to checkpoint and stream the vault to the temp file.
    pub copy: Duration,
    /// Time to empty the secret tables and vacuum the copy.
    pub strip: Duration,
    /// Time to hash, rename and record the image.
    pub seal: Duration,
}

#[derive(Debug, Clone, Copy)]
enum Capture {
    Initialize,
    Advance,
}

/// A rekeyed image: the file name under `sync/` and the SHA-256 of its re-encrypted copy
/// (`<name>.rekey-new`), or a removal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RekeyedImage {
    /// `base.db` or `pre-rebase.db`.
    pub file: &'static str,
    /// `Some` hash of the re-encrypted copy; `None` when the image is removed instead.
    pub new_sha256: Option<[u8; 32]>,
}

/// A test-only observation point inside the capture's critical section: a test installs a closure
/// on its own thread and learns where the code is, so it can look at the world at exactly that
/// step (for example, that a writer on another thread is blocked).
#[cfg(test)]
pub(crate) mod observe {
    use std::cell::RefCell;

    type Observer = Box<dyn Fn(&str)>;
    thread_local! {
        static OBSERVER: RefCell<Option<Observer>> = const { RefCell::new(None) };
    }

    pub(crate) fn install(observer: impl Fn(&str) + 'static) {
        OBSERVER.with(|o| *o.borrow_mut() = Some(Box::new(observer)));
    }

    pub(crate) fn clear() {
        OBSERVER.with(|o| *o.borrow_mut() = None);
    }

    pub(crate) fn at(label: &str) {
        OBSERVER.with(|o| {
            if let Some(observer) = o.borrow().as_ref() {
                observer(label);
            }
        });
    }
}

fn fsync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

/// SHA-256 of a file, streamed.
pub(crate) fn sha256_file(path: &Path) -> io::Result<[u8; 32]> {
    let mut reader = BufReader::with_capacity(1 << 20, File::open(path)?);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().into())
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// The side files SQLite may leave beside an image.
const SIDE_SUFFIXES: [&str; 3] = ["-wal", "-shm", "-journal"];

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut os = path.as_os_str().to_owned();
    os.push(suffix);
    PathBuf::from(os)
}

fn remove_image_files(path: &Path) -> io::Result<()> {
    remove_if_present(path)?;
    for suffix in SIDE_SUFFIXES {
        remove_if_present(&with_suffix(path, suffix))?;
    }
    Ok(())
}

impl KeyMaterial {
    /// Open the image at `path` read-write with this worker's key. The caller owns its journal
    /// mode: an image is a closed, single file.
    pub(crate) fn open_image(&self, path: &Path, flags: OpenFlags) -> Result<Connection, DbError> {
        let conn = crate::open_vault_connection(path, flags)?;
        match self {
            KeyMaterial::Raw(dek) => crate::key_raw(&conn, dek)?,
            KeyMaterial::Passphrase(key) => crate::key_passphrase(&conn, key)?,
        }
        conn.pragma_update(None, "temp_store", "MEMORY")?;
        Ok(conn)
    }
}

/// Empty every secret-bearing table of the image at `path` and vacuum it, leaving one closed file
/// with no side files. `secure_delete` zeroes deleted content and the vacuum rewrites the file, so
/// no free page keeps a credential the live vault once held.
fn strip_secrets(path: &Path, key: &KeyMaterial) -> Result<(), DbError> {
    let conn = key.open_image(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    conn.pragma_update(None, "journal_mode", "DELETE")?;
    conn.pragma_update(None, "secure_delete", "ON")?;
    for table in SECRET_BEARING_TABLES {
        conn.execute(&format!("DELETE FROM {table}"), [])?;
    }
    conn.execute_batch("VACUUM;")?;
    drop(conn);
    for suffix in SIDE_SUFFIXES {
        if with_suffix(path, suffix).exists() {
            return Err(DbError::InvalidCommand(format!(
                "a base image left a {suffix} side file"
            )));
        }
    }
    Ok(())
}

impl DbWorker {
    /// Make the first base image: `base.db` becomes this vault as it stands, with the secrets
    /// stripped, `sync_base.base_seq` is `base_seq`, its digest is recorded and the state is
    /// `Ready`. This is the entry point a bootstrapped replica calls (the bootstrap vault IS the
    /// state at `base_seq`) and the one that turns Sync on for an existing vault.
    ///
    /// It refuses, writing nothing, when a base already exists, or when the outbox is not empty.
    /// A vault whose base was lost (state `NoBase`, after a restore) may initialize again.
    ///
    /// # Errors
    /// [`BaseImageError::BaseAlreadyExists`], [`BaseImageError::OutboxNotEmpty`],
    /// [`BaseImageError::NegativeSeq`], or [`BaseImageError::Db`] for a database or file failure.
    pub fn initialize_base_image(&self, base_seq: i64) -> Result<BaseCapture, BaseImageError> {
        self.capture_base_image(Capture::Initialize, base_seq)
    }

    /// Advance the base to `head_seq`: the server head this vault's state corresponds to, known
    /// because the outbox is empty (after an accepted push, or a clean pull).
    ///
    /// The emptiness check and the copy are one critical section under the writer lock, so no
    /// command can land between them. Nothing is written when it refuses.
    ///
    /// # Errors
    /// [`BaseImageError::SyncNotEnabled`], [`BaseImageError::NoUsableBase`],
    /// [`BaseImageError::OutboxNotEmpty`], [`BaseImageError::SeqWentBack`],
    /// [`BaseImageError::NegativeSeq`], or [`BaseImageError::Db`].
    pub fn advance_base_image(&self, head_seq: i64) -> Result<BaseCapture, BaseImageError> {
        self.capture_base_image(Capture::Advance, head_seq)
    }

    fn capture_base_image(&self, mode: Capture, seq: i64) -> Result<BaseCapture, BaseImageError> {
        if seq < 0 {
            return Err(BaseImageError::NegativeSeq);
        }
        let guard = self.lock();
        if guard.state != WorkerState::Healthy {
            return Err(DbError::WorkerUnavailable(guard.state).into());
        }
        let conn = &guard.conn;
        let current = read_base_row(conn)?;
        // The state the recorded base is really in: a base whose file is gone or changed is no
        // base (the restore rule), so it may be initialized again and cannot be advanced.
        let really = current
            .as_ref()
            .map(|(_, digest, state)| verify_base(&self.path, *state, digest.as_deref()));
        match (mode, current, really) {
            (Capture::Initialize, _, None | Some(SyncBaseState::NoBase)) => {}
            (Capture::Initialize, _, Some(_)) => return Err(BaseImageError::BaseAlreadyExists),
            (Capture::Advance, None, _) | (Capture::Advance, _, None) => {
                return Err(BaseImageError::SyncNotEnabled)
            }
            (Capture::Advance, Some((base_seq, _, _)), Some(state)) => {
                if state != SyncBaseState::Ready {
                    return Err(BaseImageError::NoUsableBase(state));
                }
                if seq < base_seq {
                    return Err(BaseImageError::SeqWentBack {
                        current: base_seq,
                        requested: seq,
                    });
                }
            }
        }
        let pending: i64 = conn.query_row("SELECT COUNT(*) FROM sync_outbox", [], |r| r.get(0))?;
        if pending > 0 {
            return Err(BaseImageError::OutboxNotEmpty {
                pending: pending.unsigned_abs(),
            });
        }
        #[cfg(test)]
        observe::at("checked");

        // The checkpoint copies every committed frame into the main file only when no reader
        // holds an older snapshot; a busy result would leave frames behind, so the copy is refused.
        let busy: i64 = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))?;
        if busy != 0 {
            return Err(DbError::CheckpointBusy.into());
        }

        let dir = sync_dir(&self.path);
        fs::create_dir_all(&dir)?;
        let final_path = base_image_path(&self.path);
        let temp = with_suffix(&final_path, ".tmp");
        remove_image_files(&temp)?;

        let started = Instant::now();
        let copied = copy_file_synced(&self.path, &temp);
        let copy = started.elapsed();
        let stripped = copied
            .map_err(DbError::from)
            .and_then(|()| strip_secrets(&temp, &self.key));
        let strip = started.elapsed().saturating_sub(copy);
        #[cfg(test)]
        observe::at("copied");
        let sealed = Instant::now();
        let sealed_digest = stripped.map_err(BaseImageError::from).and_then(|()| {
            let file = File::open(&temp)?;
            file.sync_all()?;
            drop(file);
            Ok(sha256_file(&temp)?)
        });
        let digest = match sealed_digest {
            Ok(digest) => digest,
            Err(error) => {
                let _ = remove_image_files(&temp);
                return Err(error);
            }
        };
        let image_bytes = fs::metadata(&temp)?.len();

        // The record and the file change together: the row is written inside a transaction that
        // commits only after the rename, so a failed rename leaves the old record standing. A
        // crash between the two leaves a digest that matches no file, which the next open reads
        // as "no base" (the safe direction).
        let result = (|| -> Result<(), BaseImageError> {
            let tx = conn.unchecked_transaction()?;
            let now = Utc::now().to_rfc3339();
            tx.execute(
                "INSERT INTO sync_base (singleton, base_seq, base_digest, state, updated_at)
                 VALUES (1, ?1, ?2, 'Ready', ?3)
                 ON CONFLICT (singleton) DO UPDATE SET
                     base_seq = excluded.base_seq, base_digest = excluded.base_digest,
                     state = 'Ready', updated_at = excluded.updated_at",
                params![seq, digest.as_slice(), now],
            )?;
            fs::rename(&temp, &final_path)?;
            fsync_dir(&dir)?;
            tx.commit()?;
            Ok(())
        })();
        if let Err(error) = result {
            let _ = remove_image_files(&temp);
            return Err(error);
        }
        Ok(BaseCapture {
            base_seq: seq,
            digest,
            image_bytes,
            copy,
            strip,
            seal: sealed.elapsed(),
        })
    }

    /// What this device has for a rebase to restore: `base.db` is checked against the digest the
    /// vault recorded for it, so a restored backup, a half-written swap or a hand-edited file all
    /// read as [`BaseImageStatus::NoBase`].
    ///
    /// # Errors
    /// [`DbError::Sqlite`] if the `sync_base` row cannot be read.
    pub fn base_image_status(&self) -> Result<BaseImageStatus, DbError> {
        let guard = self.lock();
        status_of(&guard.conn, &self.path)
    }

    /// Record a lost base: a `Ready` row whose file is missing or whose digest differs becomes
    /// `NoBase` with no digest. Runs on every open, so the recorded state is the truth the
    /// rebase and the push path read. A base that cannot be vouched for is never rebased on.
    pub(crate) fn reconcile_base_image(&self) -> Result<(), DbError> {
        let guard = self.lock();
        let conn = &guard.conn;
        match read_base_row(conn)? {
            None => {
                // Sync is off: an image left beside the vault (an abandoned capture, or one that
                // outlived the row) is stale state that nothing will use.
                remove_image_files(&base_image_path(&self.path))?;
            }
            Some((_, digest, SyncBaseState::Ready)) => {
                if verify_base(&self.path, SyncBaseState::Ready, digest.as_deref())
                    == SyncBaseState::NoBase
                {
                    conn.execute(
                        "UPDATE sync_base
                            SET state = 'NoBase', base_digest = NULL, updated_at = ?1
                          WHERE singleton = 1",
                        params![Utc::now().to_rfc3339()],
                    )?;
                }
            }
            Some(_) => {}
        }
        Ok(())
    }

    /// Scrub the credentials of deleted connections from `sync/pre-rebase.db`, the copy of the
    /// live vault a rebase swap keeps (bxsbz.7). A removed token must not live on in it: each
    /// listed connection's secret row is overwritten and deleted and the file is vacuumed. A
    /// missing image is nothing to do. The base image never holds a credential (see
    /// [`strip_secrets`]), so it needs no scrub.
    ///
    /// # Errors
    /// [`DbError`] if the image cannot be opened with the vault's key or written.
    pub fn scrub_pre_rebase_credentials(&self, connections: &[Uuid]) -> Result<(), DbError> {
        let path = pre_rebase_path(&self.path);
        if connections.is_empty() || !path.exists() {
            return Ok(());
        }
        let conn = self
            .key
            .open_image(&path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        conn.pragma_update(None, "journal_mode", "DELETE")?;
        conn.pragma_update(None, "secure_delete", "ON")?;
        for id in connections {
            conn.execute(
                "UPDATE connector_connection_secrets SET credential = '' WHERE connection_id = ?1",
                params![id],
            )?;
            conn.execute(
                "DELETE FROM connector_connection_secrets WHERE connection_id = ?1",
                params![id],
            )?;
        }
        conn.execute_batch("VACUUM;")?;
        drop(conn);
        fsync_dir(&sync_dir(&self.path))?;
        Ok(())
    }

    /// Re-encrypt the Sync images under `new_dek` for a key rotation, beside the live ones (ADR
    /// 0083 Addendum C). Each existing image becomes `<image>.rekey-new`, written and verified
    /// under the new key; the live image is only read. An image that cannot be re-encrypted (it is
    /// damaged, or not under the current key) is reported for removal instead, so a rotation after
    /// a suspected compromise never waits on a bad image. When `base.db` is re-encrypted and the
    /// recorded base is the one on disk, the new digest is written into `new_db` (the rekeyed copy
    /// of the vault) in the same step; a base that was already lost stays lost.
    ///
    /// The caller journals the result and swaps the files at commit.
    ///
    /// # Errors
    /// [`DbError::KeyUnavailable`] in passphrase mode, [`DbError::InvalidCommand`] if a
    /// `.rekey-new` file already exists, or [`DbError`] for a database or file failure while
    /// writing `new_db`.
    pub fn prepare_rekeyed_images(
        &self,
        new_dek: &Dek,
        new_db: &Path,
    ) -> Result<Vec<RekeyedImage>, DbError> {
        let old_dek = self.dek()?;
        let recorded = {
            let guard = self.lock();
            read_base_row(&guard.conn)?
        };
        let mut images = Vec::new();
        for file in [BASE_IMAGE_FILE, PRE_REBASE_FILE] {
            let live = sync_dir(&self.path).join(file);
            if !live.exists() {
                continue;
            }
            let dest = with_suffix(&live, REKEY_NEW_SUFFIX);
            if dest.exists() {
                return Err(DbError::InvalidCommand(
                    "an image rekey copy already exists".to_owned(),
                ));
            }
            // Only an image that cannot be READ under the current key is dropped (it is damaged,
            // or not this vault's); a failure while writing the copy is the rotation's to report,
            // never a reason to lose an image.
            let rekeyed = if image_readable(&live, old_dek) {
                export_image(&live, old_dek, new_dek, &dest)?;
                Some(sha256_file(&dest)?)
            } else {
                None
            };
            images.push(RekeyedImage {
                file,
                new_sha256: rekeyed,
            });
        }
        if let Some(base) = images.iter().find(|i| i.file == BASE_IMAGE_FILE) {
            // Only a base the vault could vouch for before the rotation is vouched for after it.
            let was_ready = matches!(&recorded, Some((_, digest, SyncBaseState::Ready))
                if verify_base(&self.path, SyncBaseState::Ready, digest.as_deref())
                    == SyncBaseState::Ready);
            if was_ready {
                let conn = open_new_vault(new_db, new_dek)?;
                match base.new_sha256 {
                    Some(digest) => conn.execute(
                        "UPDATE sync_base SET base_digest = ?1 WHERE singleton = 1",
                        params![digest.as_slice()],
                    )?,
                    None => conn.execute(
                        "UPDATE sync_base SET state = 'NoBase', base_digest = NULL
                          WHERE singleton = 1",
                        [],
                    )?,
                };
                drop(conn);
                for suffix in SIDE_SUFFIXES {
                    if with_suffix(new_db, suffix).exists() {
                        return Err(DbError::InvalidCommand(format!(
                            "the rekeyed copy left a {suffix} side file"
                        )));
                    }
                }
            }
        }
        Ok(images)
    }
}

/// Open the rekeyed copy of the vault at `path` for the one `sync_base` update, in rollback-journal
/// mode so the write lands in the file.
fn open_new_vault(path: &Path, dek: &Dek) -> Result<Connection, DbError> {
    let conn = crate::open_vault_connection(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    crate::key_raw(&conn, dek)?;
    conn.pragma_update(None, "journal_mode", "DELETE")?;
    Ok(conn)
}

/// Whether the image at `live` opens under `dek` and passes a quick integrity check.
fn image_readable(live: &Path, dek: &Dek) -> bool {
    let check = || -> Result<bool, DbError> {
        let conn = crate::open_vault_connection(live, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        crate::key_raw(&conn, dek)?;
        let result: String = conn.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
        Ok(result == "ok")
    };
    check().unwrap_or(false)
}

/// Write a re-encrypted copy of the image `live` at `dest`: opened with `old_dek`, exported into a
/// database keyed with `new_dek`, then checked by reopening it under the new key.
fn export_image(live: &Path, old_dek: &Dek, new_dek: &Dek, dest: &Path) -> Result<(), DbError> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dest)?;
    let dest_text = dest
        .to_str()
        .ok_or_else(|| DbError::InvalidCommand("an image path is not UTF-8".to_owned()))?;
    // Read-write, because the export attaches the destination and an attached database inherits
    // the connection's flags; nothing is written to the image itself.
    let conn = crate::open_vault_connection(live, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    crate::key_raw(&conn, old_dek)?;
    let user_version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    // Attach without a key clause, then key the empty schema, as the vault copy does: the new
    // key never appears in SQL text (personal-cfo-7ie.17).
    conn.execute("ATTACH DATABASE ?1 AS rekeyed", params![dest_text])?;
    if let Err(error) = crate::key_raw_schema(&conn, "rekeyed", new_dek) {
        let _ = conn.execute_batch("DETACH DATABASE rekeyed;");
        return Err(error);
    }
    let exported = conn
        .query_row("SELECT sqlcipher_export('rekeyed')", [], |_| Ok(()))
        .map_err(DbError::from)
        .and_then(|()| {
            conn.execute_batch(&format!("PRAGMA rekeyed.user_version = {user_version};"))
                .map_err(DbError::from)
        });
    let detached = conn.execute_batch("DETACH DATABASE rekeyed;");
    exported?;
    detached?;
    drop(conn);

    let copy = crate::open_vault_connection(dest, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    crate::key_raw(&copy, new_dek)?;
    let integrity: String = copy.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    drop(copy);
    if integrity != "ok" {
        return Err(DbError::InvalidCommand(
            "a rekeyed image failed integrity_check".to_owned(),
        ));
    }
    for suffix in SIDE_SUFFIXES {
        if with_suffix(dest, suffix).exists() {
            return Err(DbError::InvalidCommand(format!(
                "a rekeyed image left a {suffix} side file"
            )));
        }
    }
    File::open(dest)?.sync_all()?;
    Ok(())
}

/// Stream `from` to a new file at `to` and sync it.
fn copy_file_synced(from: &Path, to: &Path) -> io::Result<()> {
    let mut reader = BufReader::with_capacity(1 << 20, File::open(from)?);
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(to)?;
    let mut writer = BufWriter::with_capacity(1 << 20, file);
    io::copy(&mut reader, &mut writer)?;
    writer.flush()?;
    writer.get_ref().sync_all()
}

/// The `sync_base` row: `(base_seq, base_digest, state)`.
type BaseRow = (i64, Option<Vec<u8>>, SyncBaseState);

fn read_base_row(conn: &Connection) -> Result<Option<BaseRow>, DbError> {
    let row: Option<(i64, Option<Vec<u8>>, String)> = conn
        .query_row(
            "SELECT base_seq, base_digest, state FROM sync_base WHERE singleton = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    row.map(|(seq, digest, state)| Ok((seq, digest, SyncBaseState::parse(&state)?)))
        .transpose()
}

/// The state a recorded base is really in: `Ready` only when the file on disk hashes to the
/// recorded digest. Any other state is returned as recorded.
fn verify_base(vault_path: &Path, recorded: SyncBaseState, digest: Option<&[u8]>) -> SyncBaseState {
    if recorded != SyncBaseState::Ready {
        return recorded;
    }
    let Some(digest) = digest else {
        return SyncBaseState::NoBase;
    };
    match sha256_file(&base_image_path(vault_path)) {
        Ok(actual) if actual.as_slice() == digest => SyncBaseState::Ready,
        _ => SyncBaseState::NoBase,
    }
}

fn status_of(conn: &Connection, vault_path: &Path) -> Result<BaseImageStatus, DbError> {
    Ok(match read_base_row(conn)? {
        None => BaseImageStatus::SyncOff,
        Some((seq, digest, state)) => match verify_base(vault_path, state, digest.as_deref()) {
            SyncBaseState::NoBase => BaseImageStatus::NoBase,
            SyncBaseState::Ready => BaseImageStatus::Ready { base_seq: seq },
            SyncBaseState::Rebasing => BaseImageStatus::Rebasing { base_seq: seq },
        },
    })
}
