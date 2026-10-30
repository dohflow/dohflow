//! `apply_command` arm for the household configuration command
//! (personal-cfo-g3m.8; `docs/architecture/vault-format.md` B.2 rule 6).
//!
//! `household_timezone` sets the calendar boundary of the forecast horizon (ADR 0021
//! section 1), so it is a class-1 logical field that changes only through a logged
//! command. This is the only code that writes it after the vault's bootstrap row
//! (`ensure_vault_metadata` seeds the `UTC` default before any command exists).
//!
//! The audit event's timestamp is `meta.issued_at`, not the wall clock: `apply` never
//! reads the clock (ADR 0013 Addendum A), so replaying the command elsewhere reads the
//! same instant.

use forecast_engine::Tz;
use rusqlite::{params, Connection};
use uuid::Uuid;

use crate::{CommandMeta, DbError};

/// The `audit_events.event_type` recorded for every household timezone change.
pub(crate) const HOUSEHOLD_TIMEZONE_CHANGED: &str = "household_timezone_changed";

/// The command's `issued_at` as an RFC 3339 UTC timestamp, for a local audit column.
pub(crate) fn issued_at_rfc3339(meta: &CommandMeta) -> Result<String, DbError> {
    Ok(super::issued_at_text(meta))
}

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
    ids: &core_ids::DerivedIds,
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
    // The audit row is device-local (class 3) and written once per command: its id is KEYED by
    // the command and the event type, and the insert ignores a repeat, so a rebase replay of the
    // same command leaves exactly one row (ADR 0073 Addendum D, rule (i)).
    tx.execute(
        "INSERT OR IGNORE INTO audit_events (id, event_type, command_id, correlation_id, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            ids.keyed("audit.household-timezone", &[])?,
            HOUSEHOLD_TIMEZONE_CHANGED,
            meta.command_id,
            meta.correlation_id,
            issued_at_rfc3339(meta)?,
        ],
    )?;
    let vault_id: Uuid = tx.query_row(
        "SELECT vault_id FROM vault_metadata WHERE singleton = 1",
        [],
        |r| r.get(0),
    )?;
    Ok(vault_id)
}
