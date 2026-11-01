//! `apply_command` arms for scenario definitions (personal-cfo-5ymg8.4; ADR 0051; ADR 0073
//! Addendum H). A scenario is class 1: a named overlay that follows the user to another device, so
//! it is written only here (and by `ApplyScenario` / `RevertScenarioApply`, which stamp
//! `applied_at`), by a command.
//!
//! * [`apply_create_scenario`] adds a `draft` scenario with a caller-minted id.
//! * [`apply_set_scenario_status`], [`apply_rename_scenario`] and [`apply_set_scenario_expiry`]
//!   change one field of a live scenario; setting the value it already has writes nothing.
//! * [`apply_clone_scenario`] copies exactly the source events the payload names.
//! * [`apply_delete_scenario`] and [`apply_reinstate_scenario`] toggle the `deleted_at` tombstone
//!   and touch no other row: the scenario's events stay where they are and are ignored while their
//!   scenario is not live, so a reinstate restores the scenario whole.
//!
//! None of them reads the clock (`created_at`, `updated_at` and `deleted_at` are the command's
//! `issued_at`) and none writes a forecast run id: a run is class 2 and never replicates, so
//! `base_run_id` is NULL on every command-written row.

use chrono::{Datelike, NaiveDate};
use core_ids::DerivedIds;
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::{CommandMeta, DbError, NewScenario, RejectCode, ScenarioStatus};

/// The earliest and latest year an expiry may name. The upper bound keeps the stored text a
/// four-digit year; the lower bound rules out a partially typed year such as `0002`.
const EXPIRY_YEARS: std::ops::RangeInclusive<i32> = 1900..=9999;

/// A scenario name: 1 to 100 characters after trimming, no control characters. Returned trimmed.
fn clean_name(value: &str) -> Result<String, DbError> {
    let trimmed = value.trim();
    let length: usize = trimmed.chars().map(|_| 1_usize).sum();
    if length == 0 || length > 100 || trimmed.chars().any(char::is_control) {
        return Err(DbError::reject(
            RejectCode::PayloadInvalid,
            "a scenario name must be 1 to 100 characters without control characters".to_owned(),
        ));
    }
    Ok(trimmed.to_owned())
}

/// Whether the scenario is tombstoned, or `None` when there is no such scenario.
fn tombstoned(tx: &Connection, id: &Uuid) -> Result<Option<bool>, DbError> {
    Ok(tx
        .query_row(
            "SELECT deleted_at IS NOT NULL FROM scenarios WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .optional()?)
}

/// The scenario must exist (`ReferenceMissing`) and not be deleted (`StateConflict`): a replica
/// that finds it deleted by another device's command refuses this one deterministically.
pub(crate) fn require_live(tx: &Connection, id: &Uuid) -> Result<(), DbError> {
    match tombstoned(tx, id)? {
        None => Err(DbError::reject(
            RejectCode::ReferenceMissing,
            "scenario does not exist".to_owned(),
        )),
        Some(true) => Err(DbError::reject(
            RejectCode::StateConflict,
            "scenario was deleted".to_owned(),
        )),
        Some(false) => Ok(()),
    }
}

/// Applies [`WriteCommand::CreateScenario`](crate::WriteCommand). Returns the scenario id.
pub(crate) fn apply_create_scenario(
    tx: &Connection,
    meta: &CommandMeta,
    scenario: &NewScenario,
) -> Result<Uuid, DbError> {
    let name = clean_name(&scenario.name)?;
    if tombstoned(tx, &scenario.id)?.is_some() {
        return Err(DbError::reject(
            RejectCode::StateConflict,
            "that scenario id is already in use".to_owned(),
        ));
    }
    let at = super::issued_at_text(meta);
    tx.execute(
        "INSERT INTO scenarios
            (id, name, description, status, base_run_id, created_at, updated_at, deleted_at)
         VALUES (?1, ?2, ?3, 'draft', NULL, ?4, ?4, NULL)",
        params![scenario.id, name, scenario.description, at],
    )?;
    Ok(scenario.id)
}

/// Applies [`WriteCommand::SetScenarioStatus`](crate::WriteCommand) on a live scenario.
pub(crate) fn apply_set_scenario_status(
    tx: &Connection,
    meta: &CommandMeta,
    id: &Uuid,
    status: ScenarioStatus,
) -> Result<Uuid, DbError> {
    require_live(tx, id)?;
    tx.execute(
        "UPDATE scenarios SET status = ?2, updated_at = ?3 WHERE id = ?1 AND status <> ?2",
        params![id, status.as_token(), super::issued_at_text(meta)],
    )?;
    Ok(*id)
}

/// Applies [`WriteCommand::RenameScenario`](crate::WriteCommand) on a live scenario.
pub(crate) fn apply_rename_scenario(
    tx: &Connection,
    meta: &CommandMeta,
    id: &Uuid,
    name: &str,
) -> Result<Uuid, DbError> {
    let name = clean_name(name)?;
    require_live(tx, id)?;
    tx.execute(
        "UPDATE scenarios SET name = ?2, updated_at = ?3 WHERE id = ?1 AND name <> ?2",
        params![id, name, super::issued_at_text(meta)],
    )?;
    Ok(*id)
}

/// Applies [`WriteCommand::SetScenarioExpiry`](crate::WriteCommand) on a live scenario. The
/// expiry controls whether the scenario applies, so a date outside [`EXPIRY_YEARS`] would
/// quietly disable someone's plan and is refused.
pub(crate) fn apply_set_scenario_expiry(
    tx: &Connection,
    meta: &CommandMeta,
    id: &Uuid,
    expires_on: Option<NaiveDate>,
) -> Result<Uuid, DbError> {
    if expires_on.is_some_and(|date| !EXPIRY_YEARS.contains(&date.year())) {
        return Err(DbError::reject(
            RejectCode::PayloadInvalid,
            "a scenario expiry year must be from 1900 to 9999".to_owned(),
        ));
    }
    require_live(tx, id)?;
    let text = expires_on.map(|date| date.format("%Y-%m-%d").to_string());
    tx.execute(
        "UPDATE scenarios SET expires_on = ?2, updated_at = ?3 WHERE id = ?1 AND expires_on IS NOT ?2",
        params![id, text, super::issued_at_text(meta)],
    )?;
    Ok(*id)
}

/// Applies [`WriteCommand::CloneScenario`](crate::WriteCommand): a new `draft` scenario carrying a
/// fresh copy of exactly the source events the payload names, in the payload's order. Returns the
/// new scenario's id.
///
/// The copied set is a function of the payload, not of the base: each event id is minted by
/// ordinal (one per payload element), an event is copied whatever its status is by now (the user
/// cloned what they saw), and an event's content never changes once written (a supersede inserts
/// a new row), so the copy is the same on every device.
///
/// Copied: the source's description and each named event's kind, target, parameters and source,
/// with `created_at` COPIED, because `entity_overrides` orders base and scenario events together by
/// `(created_at, id)` and lets the later one win, so restamping would change how the copy resolves
/// against a base override created in between. Not copied: status, expiry and run (a copy of an
/// active plan is not itself active, and the clone forks from the current base).
pub(crate) fn apply_clone_scenario(
    tx: &Connection,
    meta: &CommandMeta,
    ids: &mut DerivedIds,
    source_id: &Uuid,
    new_id: &Uuid,
    name: &str,
    event_ids: &[Uuid],
) -> Result<Uuid, DbError> {
    let name = clean_name(name)?;
    // Strictly ascending forbids a repeated id and gives the payload one byte form.
    if event_ids.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(DbError::reject(
            RejectCode::PayloadInvalid,
            "the events to clone must be listed once each, in ascending order".to_owned(),
        ));
    }
    if u32::try_from(event_ids.len()).map_or(true, |count| count > ids.remaining()) {
        return Err(DbError::reject(
            RejectCode::PayloadInvalid,
            "too many events to clone in one command".to_owned(),
        ));
    }
    require_live(tx, source_id)?;
    if tombstoned(tx, new_id)?.is_some() {
        return Err(DbError::reject(
            RejectCode::StateConflict,
            "that scenario id is already in use".to_owned(),
        ));
    }
    let at = super::issued_at_text(meta);
    tx.execute(
        "INSERT INTO scenarios
            (id, name, description, status, base_run_id, created_at, updated_at, deleted_at)
         SELECT ?2, ?3, description, 'draft', NULL, ?4, ?4, NULL FROM scenarios WHERE id = ?1",
        params![source_id, new_id, name, at],
    )?;
    for event_id in event_ids {
        let copied = tx.execute(
            // Every NOT NULL column must be carried: `source` and `kind` are CHECK-constrained
            // and non-null. `superseded_by` and `origin_run_id` describe the SOURCE's undo and
            // run history, and the copy is a fresh plan.
            "INSERT INTO forecast_assumption_events
                (id, kind, target_entity_type, target_entity_id, params_json, source,
                 scenario_id, status, superseded_by, origin_run_id, created_at, updated_at,
                 promoted_from_scenario_id)
             SELECT ?3, kind, target_entity_type, target_entity_id, params_json, source,
                    ?4, 'active', NULL, NULL, created_at, ?5, NULL
               FROM forecast_assumption_events WHERE id = ?1 AND scenario_id = ?2",
            params![
                event_id,
                source_id,
                ids.next("scenario.clone-event")?,
                new_id,
                at
            ],
        )?;
        if copied == 0 {
            return Err(DbError::reject(
                RejectCode::ReferenceMissing,
                "that event is not part of the scenario being cloned".to_owned(),
            ));
        }
    }
    Ok(*new_id)
}

/// Applies [`WriteCommand::DeleteScenario`](crate::WriteCommand): a tombstone, never a purge.
/// Deleting a deleted scenario is an accepted no-op.
pub(crate) fn apply_delete_scenario(
    tx: &Connection,
    meta: &CommandMeta,
    id: &Uuid,
) -> Result<Uuid, DbError> {
    if tombstoned(tx, id)?.is_none() {
        return Err(DbError::reject(
            RejectCode::ReferenceMissing,
            "scenario does not exist".to_owned(),
        ));
    }
    let at = super::issued_at_text(meta);
    tx.execute(
        "UPDATE scenarios SET deleted_at = ?2, updated_at = ?2
          WHERE id = ?1 AND deleted_at IS NULL",
        params![id, at],
    )?;
    Ok(*id)
}

/// Applies [`WriteCommand::ReinstateScenario`](crate::WriteCommand). Reinstating a live scenario
/// is an accepted no-op.
pub(crate) fn apply_reinstate_scenario(
    tx: &Connection,
    meta: &CommandMeta,
    id: &Uuid,
) -> Result<Uuid, DbError> {
    if tombstoned(tx, id)?.is_none() {
        return Err(DbError::reject(
            RejectCode::ReferenceMissing,
            "scenario does not exist".to_owned(),
        ));
    }
    tx.execute(
        "UPDATE scenarios SET deleted_at = NULL, updated_at = ?2
          WHERE id = ?1 AND deleted_at IS NOT NULL",
        params![id, super::issued_at_text(meta)],
    )?;
    Ok(*id)
}
