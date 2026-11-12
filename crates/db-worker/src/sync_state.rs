//! The device-local Sync state (personal-cfo-bxsbz.1; ADR 0074 decision 2, ADR 0073 section 4).
//!
//! Three class-3 tables (`sync_base`, `sync_outbox`, `outbox_queue`, migration 66) hold what a
//! rebase needs and a replica must never send: which commands are unpushed, which keys each one
//! wrote, and which were queued for the user. Only the recording lives here; the base image, the
//! rebase and the swap are later children of the bead.
//!
//! **Recording.** While Sync is enabled on this device (a `sync_base` row exists), every local
//! apply inserts a `Pending` `sync_outbox` row in the SAME transaction as its operation-log row,
//! carrying the command's write-set row keys. With Sync off nothing is read or written beyond the
//! one existence check, so a vault that never enabled Sync grows no rows. The operation log stays
//! append-only; outbox state lives only in `sync_outbox`.

use rusqlite::{params, OptionalExtension, Transaction};
use uuid::Uuid;

use crate::{DbError, IssuedAt};

/// The `operation_log` columns that are DEVICE-LOCAL: written by this device, outside the write set
/// (no hash depends on them), and EXCLUDED from every cross-replica and snapshot comparison (the
/// replay property, the two-vault test, genesis and snapshot digests). `created_at` is the
/// recording device's wall clock (personal-cfo-32fmp.4 AC2); `applied_base_seq` and `rebased_from`
/// are this module's (bxsbz.1); `issued_at` is the instant the command was issued, in Unix
/// microseconds, kept so a command taken before Sync was armed can be replayed
/// (personal-cfo-1cltt.5). The CLASS-0 manifest states the same list for `operation_log`, and a
/// test keeps the two equal.
pub const DEVICE_LOCAL_OPERATION_LOG_COLUMNS: &[&str] = &[
    "created_at",
    "applied_base_seq",
    "rebased_from",
    "issued_at",
];

/// The server seq of the base a local command is applied on, or `None` while Sync is off. Sync is
/// enabled on this device exactly when a `sync_base` row exists, so `Some` is also the switch that
/// turns outbox recording on.
pub(crate) fn applied_base_seq(tx: &Transaction<'_>) -> Result<Option<i64>, DbError> {
    Ok(tx
        .query_row(
            "SELECT base_seq FROM sync_base WHERE singleton = 1",
            [],
            |r| r.get(0),
        )
        .optional()?)
}

/// Record a local command as `Pending` in the outbox. Call it in the transaction that inserted the
/// command's operation-log row, and only while Sync is enabled ([`applied_base_seq`] is `Some`).
pub(crate) fn record_pending(
    tx: &Transaction<'_>,
    command_id: &[u8],
    op_seq: i64,
    write_set_rows: &[(String, Vec<u8>)],
    created_at: &str,
    issued_at: IssuedAt,
) -> Result<(), DbError> {
    tx.execute(
        "INSERT INTO sync_outbox (command_id, op_seq, state, write_set_keys, created_at, issued_at)
         VALUES (?1, ?2, 'Pending', ?3, ?4, ?5)",
        params![
            command_id,
            op_seq,
            encode_row_keys(write_set_rows),
            created_at,
            issued_at.as_unix_micros()
        ],
    )?;
    Ok(())
}

/// Why an unpushed command's issue time cannot be read back.
#[derive(Debug, thiserror::Error)]
pub enum OutboxIssuedAtError {
    /// The outbox has no row for the command.
    #[error("the command {command_id} is not in the outbox")]
    NotInOutbox {
        /// The command that was asked for.
        command_id: Uuid,
    },
    /// The row predates the column (or was written without it): its original `issued_at` is lost,
    /// and a replay would mint other ids than the command did. A rebase refuses by name instead
    /// of guessing.
    #[error("the unpushed command {command_id} has no recorded issue time")]
    Missing {
        /// The command whose row has no `issued_at`.
        command_id: Uuid,
    },
    /// The database failed.
    #[error(transparent)]
    Db(#[from] DbError),
}

/// The original `issued_at` of an unpushed local command, as [`record_pending`] stored it.
///
/// # Errors
/// [`OutboxIssuedAtError::NotInOutbox`], [`OutboxIssuedAtError::Missing`] for a row with no
/// recorded time, or [`OutboxIssuedAtError::Db`].
pub fn outbox_issued_at(
    conn: &rusqlite::Connection,
    command_id: &Uuid,
) -> Result<IssuedAt, OutboxIssuedAtError> {
    let row: Option<Option<i64>> = conn
        .query_row(
            "SELECT issued_at FROM sync_outbox WHERE command_id = ?1",
            params![command_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(DbError::from)?;
    match row {
        None => Err(OutboxIssuedAtError::NotInOutbox {
            command_id: *command_id,
        }),
        Some(None) => Err(OutboxIssuedAtError::Missing {
            command_id: *command_id,
        }),
        Some(Some(micros)) => Ok(IssuedAt::from_unix_micros(micros)),
    }
}

/// The canonical encoding of a command's write-set row keys: for each `(table, primary key)`, a
/// big-endian `u16` table-name length, the name, a big-endian `u32` key length, and the key bytes,
/// in the order given (the write set's canonical order). Self-delimiting, so it decodes without a
/// schema.
pub fn encode_row_keys(rows: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (table, key) in rows {
        let name = table.as_bytes();
        out.extend_from_slice(
            &u16::try_from(name.len())
                .expect("a table name fits u16")
                .to_be_bytes(),
        );
        out.extend_from_slice(name);
        out.extend_from_slice(
            &u32::try_from(key.len())
                .expect("a key fits u32")
                .to_be_bytes(),
        );
        out.extend_from_slice(key);
    }
    out
}

/// The inverse of [`encode_row_keys`]. `None` when the bytes are truncated or a table name is not
/// UTF-8: an outbox row that cannot be read is never guessed at.
pub fn decode_row_keys(bytes: &[u8]) -> Option<Vec<(String, Vec<u8>)>> {
    let mut rows = Vec::new();
    let mut rest = bytes;
    while !rest.is_empty() {
        let (len, tail) = rest.split_first_chunk::<2>()?;
        let (name, tail) = tail.split_at_checked(usize::from(u16::from_be_bytes(*len)))?;
        let (len, tail) = tail.split_first_chunk::<4>()?;
        let (key, tail) = tail.split_at_checked(usize::try_from(u32::from_be_bytes(*len)).ok()?)?;
        rows.push((String::from_utf8(name.to_vec()).ok()?, key.to_vec()));
        rest = tail;
    }
    Some(rows)
}
