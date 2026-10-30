//! `apply_command` arms for the connector identity and mapping (personal-cfo-5ymg8.2; ADR 0074
//! Addendum B; ADR 0073 Addendum D). The class-1 half of a connection: the identity
//! (`connector_connections`) and the external-account mapping (`connector_account_links`). The
//! credential, the refresh watermark and the notice are class 3 and never pass through here
//! (they are written by [`crate::DbWorker`]'s device-local methods).
//!
//! **A race with a delete is an accepted no-op, never a rejection.** A link discovered or a
//! mapping changed on one device while another device deleted the connection reaches a replica
//! after the tombstone; refusing it would be a deterministic apply failure that stops every
//! replica for a user's ordinary race. So every command on a tombstoned connection applies as a
//! logged no-op with an empty write set. An UNKNOWN connection is a producer fault (the create
//! precedes it in the log) and is rejected.

use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::connectors::{scrub_local_rows, MAX_LINK_EXTERNAL_ID_BYTES};
use crate::{CommandMeta, DbError};

/// The longest adapter id and display hint accepted (text from a registry token and from a
/// provider's institution name).
const MAX_ADAPTER_ID_BYTES: usize = 64;
const MAX_DISPLAY_HINT_BYTES: usize = 256;

/// Whether the connection exists, and if so whether it is tombstoned.
fn state_of(tx: &Connection, id: Uuid) -> Result<Option<bool>, DbError> {
    Ok(tx
        .query_row(
            "SELECT deleted_at IS NOT NULL FROM connector_connections WHERE id = ?1",
            params![id],
            |r| r.get::<_, bool>(0),
        )
        .optional()?)
}

/// Applies [`WriteCommand::CreateConnectorConnection`]: the identity only. Idempotent: the same id
/// and adapter again (a replay, or a replica that already holds it) changes nothing; the same id
/// for another adapter is a producer fault.
pub(crate) fn apply_create_connector_connection(
    tx: &Connection,
    meta: &CommandMeta,
    id: &Uuid,
    adapter_id: &str,
    display_hint: Option<&str>,
) -> Result<Uuid, DbError> {
    if adapter_id.is_empty() || adapter_id.len() > MAX_ADAPTER_ID_BYTES {
        return Err(DbError::InvalidCommand(
            "the connector adapter id must be 1 to 64 bytes".to_owned(),
        ));
    }
    if display_hint.is_some_and(|hint| hint.len() > MAX_DISPLAY_HINT_BYTES) {
        return Err(DbError::InvalidCommand(
            "the connection display hint is too long".to_owned(),
        ));
    }
    let existing: Option<String> = tx
        .query_row(
            "SELECT adapter_id FROM connector_connections WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .optional()?;
    match existing {
        Some(existing) if existing == adapter_id => return Ok(*id),
        Some(_) => {
            return Err(DbError::InvalidCommand(
                "that connection id belongs to another adapter".to_owned(),
            ))
        }
        None => {}
    }
    let at = crate::apply::issued_at_text(meta);
    tx.execute(
        "INSERT INTO connector_connections (id, adapter_id, display_hint, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?4)",
        params![id, adapter_id, display_hint, at],
    )?;
    Ok(*id)
}

/// Applies [`WriteCommand::UpsertConnectorLink`]: a discovered external account. A re-discovery
/// refreshes the display name and records the provider's currency when it states one, but never
/// clobbers the user's mapping or a known currency with an unknown one.
pub(crate) fn apply_upsert_connector_link(
    tx: &Connection,
    meta: &CommandMeta,
    connection_id: &Uuid,
    external_id: &str,
    external_name: Option<&str>,
    currency: Option<&str>,
) -> Result<Uuid, DbError> {
    if external_id.len() > MAX_LINK_EXTERNAL_ID_BYTES {
        return Err(DbError::ReplicatedKeyTooLong {
            table: "connector_account_links",
        });
    }
    if currency.is_some_and(|c| c.len() != 3 || !c.bytes().all(|b| b.is_ascii_uppercase())) {
        return Err(DbError::InvalidCommand(
            "a connector account currency is an uppercase three-letter code".to_owned(),
        ));
    }
    match state_of(tx, *connection_id)? {
        None => {
            return Err(DbError::InvalidCommand(
                "unknown connector connection".to_owned(),
            ))
        }
        Some(true) => return Ok(*connection_id), // tombstoned: an accepted no-op
        Some(false) => {}
    }
    let at = crate::apply::issued_at_text(meta);
    tx.execute(
        "INSERT INTO connector_account_links
            (connection_id, external_id, external_name, currency, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5)
         ON CONFLICT(connection_id, external_id) DO UPDATE SET
             external_name = excluded.external_name,
             currency = COALESCE(excluded.currency, connector_account_links.currency),
             updated_at = excluded.updated_at",
        params![connection_id, external_id, external_name, currency, at],
    )?;
    Ok(*connection_id)
}

/// Applies [`WriteCommand::SetConnectorLinkAccount`]: map (or unmap, with `None`) an external
/// account onto a real account. Any (re)mapping also clears THIS device's refresh watermark (class
/// 3, not in the write set): a newly targeted account must receive the full history on its next
/// sync, and every device that applies the mapping clears its own.
pub(crate) fn apply_set_connector_link_account(
    tx: &Connection,
    meta: &CommandMeta,
    connection_id: &Uuid,
    external_id: &str,
    account_id: Option<Uuid>,
) -> Result<Uuid, DbError> {
    match state_of(tx, *connection_id)? {
        None => {
            return Err(DbError::InvalidCommand(
                "unknown connector connection".to_owned(),
            ))
        }
        Some(true) => return Ok(*connection_id), // tombstoned: an accepted no-op
        Some(false) => {}
    }
    let changed = tx.execute(
        "UPDATE connector_account_links
            SET account_id = ?3, updated_at = ?4
          WHERE connection_id = ?1 AND external_id = ?2",
        params![
            connection_id,
            external_id,
            account_id,
            crate::apply::issued_at_text(meta)
        ],
    )?;
    if changed == 0 {
        return Err(DbError::InvalidCommand(
            "unknown connector account link".to_owned(),
        ));
    }
    tx.execute(
        "DELETE FROM connector_link_cursors WHERE connection_id = ?1 AND external_id = ?2",
        params![connection_id, external_id],
    )?;
    Ok(*connection_id)
}

/// Applies [`WriteCommand::DeleteConnectorConnection`]: TOMBSTONE the identity (`deleted_at` =
/// the command's `issued_at`), never a hard delete, and remove THIS device's secret, cursors and
/// notice for it (class 3, not in the write set; every device does the same when it applies the
/// command). The mapping rows stay and are ignored by every reader. A connection that is unknown
/// or already tombstoned is an accepted no-op.
pub(crate) fn apply_delete_connector_connection(
    tx: &rusqlite::Transaction<'_>,
    meta: &CommandMeta,
    id: &Uuid,
) -> Result<Uuid, DbError> {
    let at = crate::apply::issued_at_text(meta);
    match state_of(tx, *id)? {
        None | Some(true) => return Ok(*id),
        Some(false) => {}
    }
    tx.execute(
        "UPDATE connector_connections SET deleted_at = ?2, updated_at = ?2 WHERE id = ?1",
        params![id, at],
    )?;
    let had_secret = scrub_local_rows(tx, *id)?;
    if had_secret {
        // The audit row ("token removed: connection deleted") is keyed by the command, so a
        // replay of the same command cannot write it twice.
        tx.execute(
            "INSERT OR IGNORE INTO audit_events (id, event_type, command_id, correlation_id, created_at)
             VALUES (?1, ?2, ?1, ?3, ?4)",
            params![
                meta.command_id,
                crate::connectors::CONNECTOR_SECRET_REMOVED_EVENT,
                meta.correlation_id,
                at
            ],
        )?;
    }
    Ok(*id)
}
