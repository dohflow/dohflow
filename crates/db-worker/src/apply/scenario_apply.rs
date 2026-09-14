//! Applying a scenario onto the base forecast, and reverting it (ADR 0055,
//! personal-cfo-4d8.27.6.3).
//!
//! **Apply promotes events; it never rewrites entities.** For each `active` assumption
//! event carrying the scenario's id, a *new* base event (`scenario_id IS NULL`) is
//! inserted with the same kind / target / params and `promoted_from_scenario_id` set.
//! Bills, income sources and the ledger are untouched.
//!
//! The deciding reason (ADR 0055 §1) is coverage: only four of the twelve
//! [`AssumptionKind`]s have an entity whose column could be rewritten, so an
//! entity-rewriting apply would silently drop two-thirds of what a scenario can express
//! while reporting success. Promotion also needs no new forecast semantics — base is
//! already "events with no scenario" — and is reversible by construction, because it
//! only ever INSERTs.
//!
//! These live on the `WriteCommand` bus even though assumption events otherwise bypass it
//! (ADR 0055 §3): apply is the one assumption-layer operation that changes what the
//! household's real forecast says, and `meta.command_id` is stored as the reversal handle.

use rusqlite::{params, Transaction};
use uuid::Uuid;

use crate::DbError;

/// Promote a scenario's active events into base (ADR 0055 §1).
///
/// Returns the scenario id for provenance.
///
/// # Errors
/// [`DbError::InvalidCommand`] when the scenario does not exist, is already applied, or
/// carries no active events; [`DbError::Sqlite`] on a write failure.
pub(crate) fn apply_scenario(
    tx: &Transaction<'_>,
    scenario_id: Uuid,
    op_id: Uuid,
    now: &str,
) -> Result<Uuid, DbError> {
    let applied_at: Option<String> = tx
        .query_row(
            "SELECT applied_at FROM scenarios WHERE id = ?1",
            params![scenario_id],
            |row| row.get(0),
        )
        .map_err(|err| match err {
            rusqlite::Error::QueryReturnedNoRows => {
                DbError::InvalidCommand("no such scenario".to_owned())
            }
            other => DbError::from(other),
        })?;
    // Idempotent in the safe direction: applying twice would promote a second copy of
    // every event, each superseding the first, and the revert handle would only undo the
    // newer set (ADR 0055 consequences).
    if applied_at.is_some() {
        return Err(DbError::InvalidCommand(
            "this scenario is already applied".to_owned(),
        ));
    }

    // The events to promote, read before any write so the insert below cannot see its own
    // output (a promoted base event is not itself a scenario event, but reading first
    // keeps that independent of the WHERE clause).
    let mut stmt = tx.prepare(
        "SELECT id, kind, target_entity_type, target_entity_id, params_json, source
           FROM forecast_assumption_events
          WHERE scenario_id = ?1 AND status = 'active'
          ORDER BY created_at, id",
    )?;
    struct Promote {
        kind: String,
        target_entity_type: Option<String>,
        target_entity_id: Option<Uuid>,
        params_json: String,
        source: String,
    }
    let events: Vec<Promote> = stmt
        .query_map(params![scenario_id], |row| {
            Ok(Promote {
                kind: row.get(1)?,
                target_entity_type: row.get(2)?,
                target_entity_id: row.get(3)?,
                params_json: row.get(4)?,
                source: row.get(5)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    drop(stmt);

    // A scenario with nothing active does not become "applied": there is nothing to
    // promote, and marking it applied would offer a revert that undoes nothing — which is
    // how a user comes to believe something happened (ADR 0055 §5's reasoning).
    if events.is_empty() {
        return Err(DbError::InvalidCommand(
            "this scenario has no active changes to apply".to_owned(),
        ));
    }

    for event in &events {
        let new_id = Uuid::now_v7();
        // Supersede the base event this collides with, if any. Two `active` events of the
        // same kind against the same target would force the forecast to pick one by row
        // order (ADR 0055 §2). `IS` rather than `=` so a NULL target matches a NULL
        // target — kinds like `inflation_rate` and `minimum_cash_floor` are untargeted,
        // and `=` is never true for NULL, so those would silently never supersede.
        tx.execute(
            "UPDATE forecast_assumption_events
                SET status = 'superseded', superseded_by = ?1, updated_at = ?2
              WHERE scenario_id IS NULL
                AND status = 'active'
                AND kind = ?3
                AND target_entity_type IS ?4
                AND target_entity_id IS ?5",
            params![
                new_id,
                now,
                event.kind,
                event.target_entity_type,
                event.target_entity_id
            ],
        )?;
        tx.execute(
            "INSERT INTO forecast_assumption_events
                (id, kind, target_entity_type, target_entity_id, params_json,
                 source, scenario_id, status, superseded_by, origin_run_id,
                 created_at, updated_at, promoted_from_scenario_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, 'active', NULL, NULL, ?7, ?7, ?8)",
            params![
                new_id,
                event.kind,
                event.target_entity_type,
                event.target_entity_id,
                event.params_json,
                event.source,
                now,
                scenario_id,
            ],
        )?;
    }

    tx.execute(
        "UPDATE scenarios SET applied_at = ?2, applied_op_id = ?3, updated_at = ?2
          WHERE id = ?1",
        params![scenario_id, now, op_id],
    )?;
    Ok(scenario_id)
}

/// Undo an apply (ADR 0055 §5).
///
/// Clears the promoted base events and restores whatever they superseded. Because apply
/// only ever INSERTed, nothing of the prior base state was overwritten — so this restores
/// exactly rather than reconstructing from a remembered value.
///
/// # Errors
/// [`DbError::InvalidCommand`] when the scenario does not exist or is not applied;
/// [`DbError::Sqlite`] on a write failure.
pub(crate) fn revert_scenario_apply(
    tx: &Transaction<'_>,
    scenario_id: Uuid,
    now: &str,
) -> Result<Uuid, DbError> {
    let applied_at: Option<String> = tx
        .query_row(
            "SELECT applied_at FROM scenarios WHERE id = ?1",
            params![scenario_id],
            |row| row.get(0),
        )
        .map_err(|err| match err {
            rusqlite::Error::QueryReturnedNoRows => {
                DbError::InvalidCommand("no such scenario".to_owned())
            }
            other => DbError::from(other),
        })?;
    // An error rather than a silent no-op: a revert that reports success without
    // reverting anything is how a user comes to believe a change was undone (ADR 0055 §5).
    if applied_at.is_none() {
        return Err(DbError::InvalidCommand(
            "this scenario is not applied".to_owned(),
        ));
    }

    // Restore first, then clear — the restore selects on `superseded_by`, which points at
    // the promoted rows. Clearing them first would still work (the pointer survives), but
    // restoring first keeps the two statements independent of each other's ordering.
    tx.execute(
        "UPDATE forecast_assumption_events
            SET status = 'active', superseded_by = NULL, updated_at = ?2
          WHERE status = 'superseded'
            AND superseded_by IN (SELECT id FROM forecast_assumption_events
                                   WHERE promoted_from_scenario_id = ?1)",
        params![scenario_id, now],
    )?;
    // The promoted rows stay in the table as history, marked `cleared` — an existing and
    // already-reversible status. Nothing is deleted (AGENTS.md §1).
    tx.execute(
        "UPDATE forecast_assumption_events
            SET status = 'cleared', updated_at = ?2
          WHERE promoted_from_scenario_id = ?1 AND status = 'active'",
        params![scenario_id, now],
    )?;
    tx.execute(
        "UPDATE scenarios SET applied_at = NULL, applied_op_id = NULL, updated_at = ?2
          WHERE id = ?1",
        params![scenario_id, now],
    )?;
    Ok(scenario_id)
}
