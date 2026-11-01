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

use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Transaction};
use uuid::Uuid;

use crate::assumptions::AssumptionKind;
use crate::{DbError, RejectCode};

/// Promote a scenario's active events into base (ADR 0055 §1).
///
/// Returns the scenario id for provenance.
///
/// # Errors
/// [`DbError::Rejected`] when the scenario does not exist (`ReferenceMissing`), was deleted, is
/// already applied, or carries no active events (`StateConflict`); [`DbError::Sqlite`] on a write
/// failure.
pub(crate) fn apply_scenario(
    tx: &Transaction<'_>,
    ids: &core_ids::DerivedIds,
    scenario_id: Uuid,
    op_id: Uuid,
    issued: DateTime<Utc>,
) -> Result<Uuid, DbError> {
    let now = issued.to_rfc3339();
    let now = now.as_str();
    // A deleted scenario cannot be applied; undoing an apply of one can still be done
    // (`revert_scenario_apply` does not check).
    super::scenario_cmds::require_live(tx, &scenario_id)?;
    let applied_at: Option<String> = tx.query_row(
        "SELECT applied_at FROM scenarios WHERE id = ?1",
        params![scenario_id],
        |row| row.get(0),
    )?;
    // Idempotent in the safe direction: applying twice would promote a second copy of
    // every event, each superseding the first, and the revert handle would only undo the
    // newer set (ADR 0055 consequences).
    if applied_at.is_some() {
        return Err(DbError::reject(
            RejectCode::StateConflict,
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
        id: Uuid,
        kind: String,
        target_entity_type: Option<String>,
        target_entity_id: Option<Uuid>,
        params_json: String,
        source: String,
    }
    let events: Vec<Promote> = stmt
        .query_map(params![scenario_id], |row| {
            Ok(Promote {
                id: row.get(0)?,
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
        return Err(DbError::reject(
            RejectCode::StateConflict,
            "this scenario has no active changes to apply".to_owned(),
        ));
    }

    // The k-th promoted row is stamped `issued_at + k` microseconds, so the promoted rows keep the
    // scenario's own order: kinds that compose by window resolve an overlap by creation order, and
    // every row stamped the same instant would fall back to the id (ADR 0055 §2 amendment).
    for (k, event) in (0_i64..).zip(&events) {
        // The promoted event's id is KEYED by the scenario and the source event it copies (the
        // number of events is the scenario's, i.e. base state, so no ordinal is consumed).
        let new_id = ids.keyed(
            "scenario-event",
            &[scenario_id.as_bytes(), event.id.as_bytes()],
        )?;
        let created_at = (issued + Duration::microseconds(k)).to_rfc3339();
        // Supersede the base event this collides with, if any, but only for a COLLISION kind: the
        // forecast reads one event per (kind, target) for those, so two `active` ones would force
        // it to pick by row order (ADR 0055 §2). An ADDING kind is folded instance by instance,
        // so promoting one replaces nothing and the base entries stay (ADR 0055 §2 amendment).
        // `IS` rather than `=` so a NULL target matches a NULL target: `inflation_rate` and
        // `minimum_cash_floor` are untargeted, and `=` is never true for NULL.
        if AssumptionKind::from_token(&event.kind)
            .is_none_or(AssumptionKind::supersedes_on_promotion)
        {
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
        }
        tx.execute(
            "INSERT INTO forecast_assumption_events
                (id, kind, target_entity_type, target_entity_id, params_json,
                 source, scenario_id, status, superseded_by, origin_run_id,
                 created_at, updated_at, promoted_from_scenario_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, 'active', NULL, NULL, ?7, ?8, ?9)",
            params![
                new_id,
                event.kind,
                event.target_entity_type,
                event.target_entity_id,
                event.params_json,
                event.source,
                created_at,
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
/// [`DbError::Rejected`] when the scenario does not exist (`ReferenceMissing`) or is not applied
/// (`StateConflict`); [`DbError::Sqlite`] on a write failure.
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
            rusqlite::Error::QueryReturnedNoRows => DbError::reject(
                RejectCode::ReferenceMissing,
                "scenario does not exist".to_owned(),
            ),
            other => DbError::from(other),
        })?;
    // An error rather than a silent no-op: a revert that reports success without
    // reverting anything is how a user comes to believe a change was undone (ADR 0055 §5).
    if applied_at.is_none() {
        return Err(DbError::reject(
            RejectCode::StateConflict,
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
