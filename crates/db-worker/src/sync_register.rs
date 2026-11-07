//! The SQLite counter register (personal-cfo-1cltt.4; `vault-format.md` C.4 "Counter register and
//! nonce state").
//!
//! Two class-3 tables (`sync_counter_register`, `sync_saved_snapshot`, migration 68) are the durable
//! home of the `next_counter` of every `(vault_id, key_epoch, fingerprint, object_type)` context and
//! of the sealed snapshot saved for an exact-byte retransmission. [`SqliteRegisterStore`] is the one
//! register table: it passes the `RegisterStore` conformance suite of `vault-crypto`, and the
//! command engine commits its advance in the **same transaction** as the operation-log row through
//! the free functions below, never through a second table or API.
//!
//! Every write goes through the worker's writer lock, so a backup restore rolls the register back
//! with the rest of the vault. That rollback is what the independent witness exists to detect.

use platform_traits::CounterContext;
use rusqlite::{params, Connection, OptionalExtension};
use vault_crypto::sync_counter::{RegisterStore, RegisterStoreError, SavedSealed};

use crate::{DbError, DbWorker};

impl From<DbError> for RegisterStoreError {
    /// Whatever the database refused, the register is not available: it never says which row or
    /// value failed.
    fn from(_: DbError) -> Self {
        Self::Unavailable
    }
}

fn unavailable(_: rusqlite::Error) -> RegisterStoreError {
    RegisterStoreError::Unavailable
}

/// A context as the table stores it. The epoch is a `u64` on the wire and an `i64` in SQLite; no
/// vault reaches an epoch that does not fit, and one that did not would be refused, not wrapped.
fn key(context: &CounterContext) -> Result<(i64, i64), RegisterStoreError> {
    let epoch = i64::try_from(context.key_epoch).map_err(|_| RegisterStoreError::Unavailable)?;
    Ok((epoch, i64::from(context.object_type)))
}

/// The persisted `next_counter` of `context`, or `None` if it has no entry.
///
/// # Errors
/// [`RegisterStoreError::Unavailable`].
pub fn next_counter(
    conn: &Connection,
    context: &CounterContext,
) -> Result<Option<u64>, RegisterStoreError> {
    let (epoch, object_type) = key(context)?;
    let value: Option<i64> = conn
        .query_row(
            "SELECT next_counter FROM sync_counter_register
             WHERE vault_id = ?1 AND key_epoch = ?2 AND fingerprint = ?3 AND object_type = ?4",
            params![context.vault_id, epoch, context.fingerprint, object_type],
            |row| row.get(0),
        )
        .optional()
        .map_err(unavailable)?;
    value
        .map(|v| u64::try_from(v).map_err(|_| RegisterStoreError::Unavailable))
        .transpose()
}

/// Persist `next_counter = to` if it currently equals `from` (`None` meaning no entry). `to` must be
/// at least `1` and greater than `from`; anything else is [`RegisterStoreError::BadAdvance`],
/// checked before the comparison. It is one statement, so it is a compare-and-set whether or not
/// the caller holds a transaction: pass the transaction that inserts the operation-log row and the
/// advance commits or rolls back with it.
///
/// # Errors
/// [`RegisterStoreError`].
pub fn commit_advance(
    conn: &Connection,
    context: &CounterContext,
    from: Option<u64>,
    to: u64,
) -> Result<(), RegisterStoreError> {
    if to == 0 || from.is_some_and(|f| to <= f) {
        return Err(RegisterStoreError::BadAdvance);
    }
    let (epoch, object_type) = key(context)?;
    let to = i64::try_from(to).map_err(|_| RegisterStoreError::BadAdvance)?;
    let changed = match from {
        None => conn.execute(
            "INSERT INTO sync_counter_register
                 (vault_id, key_epoch, fingerprint, object_type, next_counter)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT DO NOTHING",
            params![
                context.vault_id,
                epoch,
                context.fingerprint,
                object_type,
                to
            ],
        ),
        Some(from) => {
            let from = i64::try_from(from).map_err(|_| RegisterStoreError::Conflict)?;
            conn.execute(
                "UPDATE sync_counter_register SET next_counter = ?5
                 WHERE vault_id = ?1 AND key_epoch = ?2 AND fingerprint = ?3 AND object_type = ?4
                   AND next_counter = ?6",
                params![
                    context.vault_id,
                    epoch,
                    context.fingerprint,
                    object_type,
                    to,
                    from
                ],
            )
        }
    }
    .map_err(unavailable)?;
    if changed == 1 {
        Ok(())
    } else {
        Err(RegisterStoreError::Conflict)
    }
}

/// `object_id`, `base_seq`, the two digests, `container_len` and `counter` as the table stores them.
type SavedRow = (Vec<u8>, i64, Vec<u8>, Vec<u8>, i64, i64);

/// The saved sealed snapshot of `context`, if any.
///
/// # Errors
/// [`RegisterStoreError::Unavailable`].
pub fn saved(
    conn: &Connection,
    context: &CounterContext,
) -> Result<Option<SavedSealed>, RegisterStoreError> {
    let (epoch, object_type) = key(context)?;
    let row: Option<SavedRow> = conn
        .query_row(
            "SELECT object_id, base_seq, plaintext_sha256, container_sha256, container_len, counter
             FROM sync_saved_snapshot
             WHERE vault_id = ?1 AND key_epoch = ?2 AND fingerprint = ?3 AND object_type = ?4",
            params![context.vault_id, epoch, context.fingerprint, object_type],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .optional()
        .map_err(unavailable)?;
    row.map(
        |(object_id, base_seq, plaintext, container, len, counter)| {
            // The table's CHECKs make each of these conversions total; a value that still fails is a
            // store that was edited outside this module, and it is unavailable rather than guessed at.
            let bad = || RegisterStoreError::Unavailable;
            Ok(SavedSealed {
                object_id: object_id.try_into().map_err(|_| bad())?,
                base_seq: u64::try_from(base_seq).map_err(|_| bad())?,
                plaintext_sha256: plaintext.try_into().map_err(|_| bad())?,
                container_sha256: container.try_into().map_err(|_| bad())?,
                container_len: u64::try_from(len).map_err(|_| bad())?,
                counter: u64::try_from(counter).map_err(|_| bad())?,
            })
        },
    )
    .transpose()
}

/// Replace (or, with `None`, clear) the saved sealed snapshot of `context`. It never touches the
/// counter.
///
/// # Errors
/// [`RegisterStoreError::Unavailable`].
pub fn set_saved(
    conn: &Connection,
    context: &CounterContext,
    record: Option<&SavedSealed>,
) -> Result<(), RegisterStoreError> {
    let (epoch, object_type) = key(context)?;
    match record {
        None => {
            conn.execute(
                "DELETE FROM sync_saved_snapshot
                 WHERE vault_id = ?1 AND key_epoch = ?2 AND fingerprint = ?3 AND object_type = ?4",
                params![context.vault_id, epoch, context.fingerprint, object_type],
            )
            .map_err(unavailable)?;
        }
        Some(record) => {
            let too_big = |_| RegisterStoreError::Unavailable;
            conn.execute(
                "INSERT OR REPLACE INTO sync_saved_snapshot
                     (vault_id, key_epoch, fingerprint, object_type, object_id, base_seq,
                      plaintext_sha256, container_sha256, container_len, counter)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    context.vault_id,
                    epoch,
                    context.fingerprint,
                    object_type,
                    record.object_id,
                    i64::try_from(record.base_seq).map_err(too_big)?,
                    record.plaintext_sha256,
                    record.container_sha256,
                    i64::try_from(record.container_len).map_err(too_big)?,
                    i64::try_from(record.counter).map_err(too_big)?,
                ],
            )
            .map_err(unavailable)?;
        }
    }
    Ok(())
}

/// The register of one vault, in the vault's `sync_counter_register` and `sync_saved_snapshot`
/// tables. Each call takes the worker's writer lock for its own transaction; to commit an advance
/// with other writes, call [`commit_advance`] on the transaction that holds them instead.
#[derive(Clone, Copy)]
pub struct SqliteRegisterStore<'a> {
    worker: &'a DbWorker,
}

impl<'a> SqliteRegisterStore<'a> {
    /// A register over `worker`'s vault.
    #[must_use]
    pub const fn new(worker: &'a DbWorker) -> Self {
        Self { worker }
    }
}

impl core::fmt::Debug for SqliteRegisterStore<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SqliteRegisterStore")
            .finish_non_exhaustive()
    }
}

impl RegisterStore for SqliteRegisterStore<'_> {
    fn next_counter(&self, context: &CounterContext) -> Result<Option<u64>, RegisterStoreError> {
        self.worker.write_sync_state(|tx| next_counter(tx, context))
    }

    fn commit_advance(
        &self,
        context: &CounterContext,
        from: Option<u64>,
        to: u64,
    ) -> Result<(), RegisterStoreError> {
        self.worker
            .write_sync_state(|tx| commit_advance(tx, context, from, to))
    }

    fn saved(&self, context: &CounterContext) -> Result<Option<SavedSealed>, RegisterStoreError> {
        self.worker.write_sync_state(|tx| saved(tx, context))
    }

    fn set_saved(
        &self,
        context: &CounterContext,
        record: Option<&SavedSealed>,
    ) -> Result<(), RegisterStoreError> {
        self.worker
            .write_sync_state(|tx| set_saved(tx, context, record))
    }
}
