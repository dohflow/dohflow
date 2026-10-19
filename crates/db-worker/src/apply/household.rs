//! `apply_command` arm for the household configuration command
//! (personal-cfo-g3m.8; `docs/architecture/vault-format.md` B.2 rule 6).
//!
//! `household_timezone` sets the calendar boundary of the forecast horizon (ADR 0021
//! section 1), so it is a class-1 logical field that changes only through a logged
//! command. This is the only code that writes it after the vault's bootstrap row
//! (`ensure_vault_metadata` seeds the `UTC` default before any command exists).

use forecast_engine::Tz;
use rusqlite::{params, Connection};
use uuid::Uuid;

use crate::{CommandMeta, DbError};

/// The `audit_events.event_type` recorded for every household timezone change.
pub(crate) const HOUSEHOLD_TIMEZONE_CHANGED: &str = "household_timezone_changed";

/// Applies [`WriteCommand::SetHouseholdTimezone`](crate::WriteCommand): validate the IANA
/// name, store it on the singleton `vault_metadata` row, and append an `audit_events` row
/// in the same transaction as the `operation_log` entry the caller writes. Returns the
/// vault id as the affected entity.
///
/// A name that is not a real IANA zone is rejected here, the one gate that keeps a typo
/// or a removed zone name out of the vault, because `read_household_tz` and every caller
/// downstream of it (the whole Future Cash forecast surface, ADR 0021) fail loudly on a
/// bad value. Setting the value it already has is an accepted, logged no-op.
pub(crate) fn apply_set_household_timezone(
    tx: &Connection,
    meta: &CommandMeta,
    timezone: &str,
) -> Result<Uuid, DbError> {
    timezone.parse::<Tz>().map_err(|e| {
        DbError::InvalidCommand(format!("invalid household timezone {timezone:?}: {e}"))
    })?;
    let updated = tx.execute(
        "UPDATE vault_metadata SET household_timezone = ?1 WHERE singleton = 1",
        [timezone],
    )?;
    if updated != 1 {
        return Err(DbError::InvalidCommand(
            "the vault has no metadata row to hold the household timezone".to_owned(),
        ));
    }
    tx.execute(
        "INSERT INTO audit_events (id, event_type, command_id, correlation_id, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            Uuid::now_v7(),
            HOUSEHOLD_TIMEZONE_CHANGED,
            meta.command_id,
            meta.correlation_id,
            chrono::Utc::now().to_rfc3339(),
        ],
    )?;
    let vault_id: Uuid = tx.query_row(
        "SELECT vault_id FROM vault_metadata WHERE singleton = 1",
        [],
        |r| r.get(0),
    )?;
    Ok(vault_id)
}
