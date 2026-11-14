//! What the swap needs from the worker (personal-cfo-bxsbz.7; ADR 0074 decision 2, ADR 0083 Addendum
//! B and C). The file renames, the journal and the recovery are the kernel's, under the vault lease;
//! this module gives them two things:
//!
//! * [`DbWorker::begin_swap`] freezes the live vault and checks, in one hold of the writer lock, that
//!   it is still what the rebase was built from, and records `Rebasing` in `sync_base` so a crash
//!   leaves the intent on disk. [`DbWorker::abort_swap`] undoes it.
//! * [`Scratch::into_swap`] finishes the rebuilt vault: the new base image (the vault at the new
//!   head, before the local commands) is stripped of credentials, vacuumed and hashed, the rebuilt
//!   vault's `sync_base` row is set to the new head and that digest, and both files are closed and
//!   hashed so the journal can name them.

use std::fs::File;
use std::path::PathBuf;

use chrono::Utc;

use super::overlay::{capture_in, Preservation};
use super::{RebaseError, Scratch};
use crate::base_image::{read_base_row, sha256_file, strip_secrets, SyncBaseState};
use crate::{DbError, DbWorker, WorkerState};

/// The two finished files of a rebase, hashed, ready for the kernel to swap in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwapInputs {
    /// The rebuilt vault, closed.
    pub scratch_path: PathBuf,
    /// Its SHA-256.
    pub scratch_sha256: [u8; 32],
    /// The new base image (credentials stripped), closed.
    pub new_base_path: PathBuf,
    /// Its SHA-256, the digest the rebuilt vault records for `sync/base.db`.
    pub new_base_sha256: [u8; 32],
    /// The server seq of the new base.
    pub head_seq: i64,
    /// The server seq of the base the rebase started from.
    pub old_base_seq: i64,
}

impl DbWorker {
    /// Freeze this vault for the swap. In one hold of the writer lock: the worker must be healthy
    /// with a `Ready` base, and the live vault must still be exactly what `captured` was read from
    /// (a command, a class-3 write or a staged import since then is [`RebaseError::LiveMovedOn`] and
    /// nothing changes); then `sync_base` is recorded `Rebasing`, the WAL is folded into the main
    /// file, and the worker is frozen (`WorkerState::Swapping`).
    ///
    /// # Errors
    /// [`RebaseError::NotReadyToSwap`], [`RebaseError::LiveMovedOn`], [`DbError::CheckpointBusy`] or
    /// [`RebaseError::Db`].
    pub fn begin_swap(&self, captured: &Preservation) -> Result<(), RebaseError> {
        let mut guard = self.lock();
        if guard.state != WorkerState::Healthy {
            return Err(DbError::WorkerUnavailable(guard.state).into());
        }
        match read_base_row(&guard.conn)? {
            Some((_, Some(_), SyncBaseState::Ready)) => {}
            _ => return Err(RebaseError::NotReadyToSwap),
        }
        if capture_in(&guard.conn)?.digest() != captured.digest() {
            return Err(RebaseError::LiveMovedOn);
        }
        guard.conn.execute(
            "UPDATE sync_base SET state = 'Rebasing', updated_at = ?1 WHERE singleton = 1",
            [Utc::now().to_rfc3339()],
        )?;
        // Any failure from here on puts the record back before it is reported, so a worker that
        // stays healthy never carries a `Rebasing` record (04b's #308 O1). If even the revert fails,
        // the next open resets a `Rebasing` record that has no journal.
        let revert = |conn: &rusqlite::Connection| {
            let _ = conn.execute(
                "UPDATE sync_base SET state = 'Ready' WHERE singleton = 1",
                [],
            );
        };
        let busy: i64 = match guard
            .conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))
        {
            Ok(busy) => busy,
            Err(error) => {
                revert(&guard.conn);
                return Err(error.into());
            }
        };
        if busy != 0 {
            revert(&guard.conn);
            return Err(DbError::CheckpointBusy.into());
        }
        guard.state = WorkerState::Swapping;
        Ok(())
    }

    /// Undo [`DbWorker::begin_swap`]: `sync_base` goes back to `Ready` and the worker is healthy
    /// again. A worker that is not swapping is left as it is.
    ///
    /// # Errors
    /// [`RebaseError::Db`].
    pub fn abort_swap(&self) -> Result<(), RebaseError> {
        let mut guard = self.lock();
        if guard.state != WorkerState::Swapping {
            return Ok(());
        }
        guard.conn.execute(
            "UPDATE sync_base SET state = 'Ready', updated_at = ?1 WHERE singleton = 1",
            [Utc::now().to_rfc3339()],
        )?;
        guard.state = WorkerState::Healthy;
        Ok(())
    }
}

impl Scratch<'_> {
    /// Finish the rebuilt vault and the new base image and hand both to the swap, consuming the
    /// scratch vault. Needs a rebase that ran to the end: replayed, overlaid and with the pulled
    /// class-3 effects run.
    ///
    /// # Errors
    /// [`RebaseError::NotReadyToSwap`] or [`RebaseError::Db`].
    pub fn into_swap(self) -> Result<SwapInputs, RebaseError> {
        if !(self.rebased && self.overlaid && self.effects_applied) {
            return Err(RebaseError::NotReadyToSwap);
        }
        let Some(new_base) = self.new_base.clone() else {
            return Err(RebaseError::NotReadyToSwap);
        };
        let Scratch {
            live,
            worker,
            path,
            base_seq,
            head_seq,
            ..
        } = self;
        // The new base: no credential, compacted, closed, hashed.
        strip_secrets(&new_base, &live.key)?;
        File::open(&new_base)?.sync_all()?;
        let new_base_sha256 = sha256_file(&new_base)?;
        // The rebuilt vault records the new base: at the new head, vouched by the digest above.
        {
            let guard = worker.lock();
            guard.conn.execute(
                "INSERT INTO sync_base (singleton, base_seq, base_digest, state, updated_at)
                 VALUES (1, ?1, ?2, 'Ready', ?3)
                 ON CONFLICT (singleton) DO UPDATE SET
                     base_seq = excluded.base_seq, base_digest = excluded.base_digest,
                     state = 'Ready', updated_at = excluded.updated_at",
                rusqlite::params![
                    head_seq,
                    new_base_sha256.as_slice(),
                    Utc::now().to_rfc3339()
                ],
            )?;
        }
        worker.checkpoint_truncate()?;
        drop(worker);
        File::open(&path)?.sync_all()?;
        let scratch_sha256 = sha256_file(&path)?;
        // A closed vault leaves no side file behind; one that did would mean the hash above is not
        // of the whole vault.
        for suffix in ["-wal", "-shm", "-journal"] {
            let mut side = path.clone().into_os_string();
            side.push(suffix);
            if PathBuf::from(&side).exists() {
                return Err(DbError::SelfTestFailed(format!(
                    "the rebuilt vault left a {suffix} side file"
                ))
                .into());
            }
        }
        Ok(SwapInputs {
            scratch_path: path,
            scratch_sha256,
            new_base_path: new_base,
            new_base_sha256,
            head_seq,
            old_base_seq: base_seq,
        })
    }
}

#[cfg(test)]
impl Scratch<'_> {
    /// Where the new base image is, once the local commands were rebased.
    pub(crate) fn new_base_path(&self) -> Option<&std::path::Path> {
        self.new_base.as_deref()
    }
}
