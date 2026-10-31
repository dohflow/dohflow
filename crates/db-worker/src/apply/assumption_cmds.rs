//! `apply_command` arms for forecast assumption events (personal-cfo-5ymg8.4; ADR 0026 section 4;
//! ADR 0055). An assumption event is class 1: a user override or scenario overlay that follows the
//! user to another device, so it is written only here, by a command.
//!
//! * [`apply_record_assumption_event`] appends an `active` event. The id is minted by the caller.
//! * [`apply_supersede_assumption_event`] replaces an active event with a new one in ONE command,
//!   so an edit can never land as two writes that a replica applies apart (which would leave two
//!   active events and double a cash entry).
//! * [`apply_clear_assumption_event`] deactivates an active event and keeps the row.
//!
//! An event is never deleted by these commands, and none of them reads the clock: `created_at` and
//! `updated_at` are the command's `issued_at`. An event carries no forecast run (a run is class 2
//! and never replicates), so `origin_run_id` is NULL on every command-written row.

use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::{CommandMeta, DbError, NewAssumptionEvent, RejectCode};

/// The event's status, or `None` when there is no such event.
fn status_of(tx: &Connection, id: &Uuid) -> Result<Option<String>, DbError> {
    Ok(tx
        .query_row(
            "SELECT status FROM forecast_assumption_events WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .optional()?)
}

/// The event must exist and still be `active`: a replica that finds it already superseded or
/// cleared by another device's command refuses this one deterministically instead of leaving two
/// active events behind.
fn require_active(tx: &Connection, id: &Uuid) -> Result<(), DbError> {
    match status_of(tx, id)?.as_deref() {
        None => Err(DbError::reject(
            RejectCode::ReferenceMissing,
            "assumption event does not exist".to_owned(),
        )),
        Some("active") => Ok(()),
        Some(_) => Err(DbError::reject(
            RejectCode::StateConflict,
            "assumption event is no longer active".to_owned(),
        )),
    }
}

/// Validate and insert one new `active` event.
fn insert_event(
    tx: &Connection,
    meta: &CommandMeta,
    event: &NewAssumptionEvent,
) -> Result<(), DbError> {
    if !serde_json::from_str::<serde_json::Value>(&event.params_json).is_ok_and(|v| v.is_object()) {
        return Err(DbError::reject(
            RejectCode::PayloadInvalid,
            "assumption parameters must be a JSON object".to_owned(),
        ));
    }
    if let Some(scenario) = event.scenario_id {
        let known: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM scenarios WHERE id = ?1)",
            params![scenario],
            |r| r.get(0),
        )?;
        if !known {
            return Err(DbError::reject(
                RejectCode::ReferenceMissing,
                "scenario does not exist".to_owned(),
            ));
        }
    }
    if status_of(tx, &event.id)?.is_some() {
        return Err(DbError::reject(
            RejectCode::StateConflict,
            "that assumption event id is already in use".to_owned(),
        ));
    }
    let at = super::issued_at_text(meta);
    tx.execute(
        "INSERT INTO forecast_assumption_events
            (id, kind, target_entity_type, target_entity_id, params_json,
             source, scenario_id, status, superseded_by, origin_run_id,
             created_at, updated_at, promoted_from_scenario_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'active', NULL, NULL, ?8, ?8, NULL)",
        params![
            event.id,
            event.kind.as_token(),
            event.target_entity_type,
            event.target_entity_id,
            event.params_json,
            event.source.as_token(),
            event.scenario_id,
            at,
        ],
    )?;
    Ok(())
}

/// Applies [`WriteCommand::RecordAssumptionEvent`](crate::WriteCommand). Returns the event id.
pub(crate) fn apply_record_assumption_event(
    tx: &Connection,
    meta: &CommandMeta,
    event: &NewAssumptionEvent,
) -> Result<Uuid, DbError> {
    insert_event(tx, meta, event)?;
    Ok(event.id)
}

/// Applies [`WriteCommand::SupersedeAssumptionEvent`](crate::WriteCommand): the old event must be
/// `active`; the replacement is inserted and the old event points at it, in one transaction.
/// Returns the replacement's id.
pub(crate) fn apply_supersede_assumption_event(
    tx: &Connection,
    meta: &CommandMeta,
    id: &Uuid,
    replacement: &NewAssumptionEvent,
) -> Result<Uuid, DbError> {
    if replacement.id == *id {
        return Err(DbError::reject(
            RejectCode::PayloadInvalid,
            "an assumption event cannot supersede itself".to_owned(),
        ));
    }
    require_active(tx, id)?;
    insert_event(tx, meta, replacement)?;
    tx.execute(
        "UPDATE forecast_assumption_events
            SET status = 'superseded', superseded_by = ?2, updated_at = ?3
          WHERE id = ?1",
        params![id, replacement.id, super::issued_at_text(meta)],
    )?;
    Ok(replacement.id)
}

/// Applies [`WriteCommand::ClearAssumptionEvent`](crate::WriteCommand): the event must be
/// `active`. Returns the event id.
pub(crate) fn apply_clear_assumption_event(
    tx: &Connection,
    meta: &CommandMeta,
    id: &Uuid,
) -> Result<Uuid, DbError> {
    require_active(tx, id)?;
    tx.execute(
        "UPDATE forecast_assumption_events SET status = 'cleared', updated_at = ?2 WHERE id = ?1",
        params![id, super::issued_at_text(meta)],
    )?;
    Ok(*id)
}
