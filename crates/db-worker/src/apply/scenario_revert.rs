//! Undoing an applied scenario, following what the user did to the promoted items afterwards
//! (ADR 0055 section 5 as amended by personal-cfo-5ie.12).
//!
//! The first revert reactivated the base events an apply had superseded and cleared the promoted
//! rows that were still active. If the user had EDITED a promoted item after the apply (a
//! supersede P -> P', where P' is not marked promoted), the revert left P' active next to the
//! restored base event: the item counted twice. The owner chose to follow the chain: reverting
//! undoes the whole apply, so it also clears the active descendants of each promoted row.
//!
//! [`plan_revert`] reads the state and decides everything; the command applies the plan and the
//! preview shows it, so what the person confirmed is what the command checks.
//!
//! * A promoted row's chain is followed through `superseded_by`. A row of the same scenario is part
//!   of the apply (two events of one scenario can collide with each other) and is passed through;
//!   any other row with `promoted_from_scenario_id` set belongs to a LATER apply, so the revert is
//!   blocked: later applies are undone first.
//! * A base event the apply superseded is restored, unless its kind collides and another active
//!   base event now holds its key. If that one came from a later apply the revert is blocked; if
//!   it is a plain base row (the user recorded a newer value) the older value stays superseded.
//!
//! Everything here is a function of the payload and the class-1 event state, so a replica on the
//! same base reaches the same verdict.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use uuid::Uuid;

use crate::assumptions::AssumptionKind;
use crate::scenarios::{RevertChange, RevertLine};
use crate::{DbError, RejectCode};

/// The columns of one event the plan reads.
#[derive(Clone)]
struct EventRow {
    id: Uuid,
    kind: String,
    target_entity_type: Option<String>,
    target_entity_id: Option<Uuid>,
    params_json: String,
    status: String,
    superseded_by: Option<Uuid>,
    promoted_from: Option<Uuid>,
}

const EVENT_COLUMNS: &str = "id, kind, target_entity_type, target_entity_id, params_json, status,
     superseded_by, promoted_from_scenario_id";

fn event_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<EventRow> {
    Ok(EventRow {
        id: row.get(0)?,
        kind: row.get(1)?,
        target_entity_type: row.get(2)?,
        target_entity_id: row.get(3)?,
        params_json: row.get(4)?,
        status: row.get(5)?,
        superseded_by: row.get(6)?,
        promoted_from: row.get(7)?,
    })
}

fn load(conn: &Connection, id: Uuid) -> Result<Option<EventRow>, DbError> {
    Ok(conn
        .query_row(
            &format!("SELECT {EVENT_COLUMNS} FROM forecast_assumption_events WHERE id = ?1"),
            params![id],
            event_row,
        )
        .optional()?)
}

fn line_kind(kind: &str) -> Option<AssumptionKind> {
    AssumptionKind::from_token(kind)
}

/// The scenario of a LATER apply that `row` came from, if any: the row itself is promoted from
/// another scenario, or it is an edit (a supersede) somewhere down a chain that starts at a row
/// promoted from another scenario. A plain base row the person recorded has no such origin.
fn later_apply_behind(
    conn: &Connection,
    row: &EventRow,
    scenario_id: Uuid,
) -> Result<Option<Uuid>, DbError> {
    if let Some(owner) = row.promoted_from {
        return Ok((owner != scenario_id && still_applied(conn, owner)?).then_some(owner));
    }
    let mut seen: HashSet<Uuid> = HashSet::from([row.id]);
    let mut stack = vec![row.id];
    while let Some(id) = stack.pop() {
        let parents: Vec<EventRow> = {
            let mut stmt = conn.prepare(&format!(
                "SELECT {EVENT_COLUMNS} FROM forecast_assumption_events WHERE superseded_by = ?1"
            ))?;
            let rows = stmt.query_map(params![id], event_row)?;
            rows.collect::<Result<_, _>>()?
        };
        for parent in parents {
            if !seen.insert(parent.id) {
                continue;
            }
            match parent.promoted_from {
                Some(owner) if owner != scenario_id && still_applied(conn, owner)? => {
                    return Ok(Some(owner))
                }
                _ => stack.push(parent.id),
            }
        }
    }
    Ok(None)
}

/// Whether `scenario_id` is applied now. A scenario that was undone is no blocker: the retired
/// revert could leave a person's edit of its promoted row active, and that edit is then a plain
/// newer value, not a later apply that has to be undone first.
fn still_applied(conn: &Connection, scenario_id: Uuid) -> Result<bool, DbError> {
    Ok(conn
        .query_row(
            "SELECT applied_at IS NOT NULL FROM scenarios WHERE id = ?1",
            params![scenario_id],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(false))
}

/// What a revert of one applied scenario would do.
pub(crate) struct RevertPlan {
    /// The ACTIVE descendants of the promoted rows, which the revert clears: exactly the ids the
    /// person confirms.
    pub descendants: BTreeSet<Uuid>,
    /// The scenario whose apply sits on top of this one, when the revert has to wait for its undo.
    pub blocker: Option<Uuid>,
    /// What the confirmation lists, one line per changed item.
    pub lines: Vec<RevertLine>,
    /// The base events the revert reactivates.
    pub restore: Vec<Uuid>,
}

/// Plan the revert of `scenario_id`'s apply from the current state.
///
/// # Errors
/// [`DbError::Sqlite`] on a read failure.
pub(crate) fn plan_revert(conn: &Connection, scenario_id: Uuid) -> Result<RevertPlan, DbError> {
    let mut plan = RevertPlan {
        descendants: BTreeSet::new(),
        blocker: None,
        lines: Vec::new(),
        restore: Vec::new(),
    };

    let promoted: Vec<EventRow> = {
        let mut stmt = conn.prepare(&format!(
            "SELECT {EVENT_COLUMNS} FROM forecast_assumption_events
              WHERE promoted_from_scenario_id = ?1 ORDER BY created_at, id"
        ))?;
        let rows = stmt.query_map(params![scenario_id], event_row)?;
        rows.collect::<Result<_, _>>()?
    };
    let promoted_ids: HashSet<Uuid> = promoted.iter().map(|row| row.id).collect();

    // One line per chain tip, so two promoted rows that end in one tip list the item once.
    let mut lines: BTreeMap<Uuid, RevertLine> = BTreeMap::new();
    for head in &promoted {
        let mut tip = head.clone();
        let mut edited = false;
        let mut seen: HashSet<Uuid> = HashSet::from([head.id]);
        let mut next = head.superseded_by;
        // A chain is a path (a supersede always points at a row inserted by the same command), so
        // the visited set only bounds the walk against a vault that was edited by hand.
        while let Some(id) = next {
            if !seen.insert(id) {
                break;
            }
            let Some(row) = load(conn, id)? else { break };
            match row.promoted_from {
                Some(owner) if owner == scenario_id => {}
                Some(other) => {
                    plan.blocker = Some(other);
                    return Ok(plan);
                }
                None => {
                    edited = true;
                    if row.status == "active" {
                        plan.descendants.insert(row.id);
                    }
                }
            }
            next = row.superseded_by;
            tip = row;
        }
        let change = match tip.status.as_str() {
            "cleared" => Some((RevertChange::Cleared, None)),
            "active" if edited => Some((RevertChange::Edited, Some(tip.params_json.clone()))),
            _ => None,
        };
        if let Some((change, after_params_json)) = change {
            lines.entry(tip.id).or_insert_with(|| RevertLine {
                change,
                kind: head.kind.clone(),
                target_entity_type: head.target_entity_type.clone(),
                target_entity_id: head.target_entity_id,
                before_params_json: head.params_json.clone(),
                after_params_json,
            });
        }
    }
    plan.lines = lines.into_values().collect();

    // The base events the apply retired.
    let retired: Vec<EventRow> = {
        let mut stmt = conn.prepare(&format!(
            "SELECT {EVENT_COLUMNS} FROM forecast_assumption_events
              WHERE status = 'superseded' AND superseded_by IN
                    (SELECT id FROM forecast_assumption_events WHERE promoted_from_scenario_id = ?1)
              ORDER BY created_at, id"
        ))?;
        let rows = stmt.query_map(params![scenario_id], event_row)?;
        rows.collect::<Result<_, _>>()?
    };
    for old in retired {
        // A row of this scenario that a later row of the same scenario superseded is part of the
        // apply: restore it and let the closing clear mark it, with no holder check and no line.
        let collides = old.promoted_from != Some(scenario_id)
            && line_kind(&old.kind).is_none_or(AssumptionKind::supersedes_on_promotion);
        if collides {
            let mut stmt = conn.prepare(&format!(
                "SELECT {EVENT_COLUMNS} FROM forecast_assumption_events
                  WHERE scenario_id IS NULL AND status = 'active' AND kind = ?1
                    AND target_entity_type IS ?2 AND target_entity_id IS ?3
                  ORDER BY created_at, id"
            ))?;
            let holders: Vec<EventRow> = stmt
                .query_map(
                    params![old.kind, old.target_entity_type, old.target_entity_id],
                    event_row,
                )?
                .collect::<Result<_, _>>()?;
            let holders: Vec<&EventRow> = holders
                .iter()
                .filter(|row| {
                    !plan.descendants.contains(&row.id) && !promoted_ids.contains(&row.id)
                })
                .collect();
            for row in &holders {
                if let Some(later) = later_apply_behind(conn, row, scenario_id)? {
                    plan.blocker = Some(later);
                    return Ok(plan);
                }
            }
            if let Some(newer) = holders.first() {
                plan.lines.push(RevertLine {
                    change: RevertChange::KeptNewer,
                    kind: old.kind.clone(),
                    target_entity_type: old.target_entity_type.clone(),
                    target_entity_id: old.target_entity_id,
                    before_params_json: old.params_json.clone(),
                    after_params_json: Some(newer.params_json.clone()),
                });
                continue;
            }
        }
        plan.restore.push(old.id);
    }
    Ok(plan)
}

/// A refusal here writes nothing, and the interface relies on that to tell the person "nothing was
/// undone": `apply_command` (`lib.rs`) opens ONE transaction (`inner.conn.transaction()`), this
/// function and the operation-log and idempotency writes all run inside it, and every `?` after it
/// returns the error with the transaction dropped uncommitted, which rolls it back; only the final
/// `tx.commit()?` publishes. Nothing in `Kernel::dispatch` or the IPC command is fallible after the
/// worker returns. `tests/scenario_revert.rs::a_refused_undo_leaves_the_vault_exactly_as_it_was`
/// holds that to a full dump of the affected tables.
///
/// Applies [`WriteCommand::RevertScenarioApplyConfirmed`](crate::WriteCommand): clears the active
/// descendants the person confirmed, restores the base events the apply superseded and clears the
/// promoted rows. Returns the scenario id.
///
/// # Errors
/// [`DbError::Rejected`]: `PayloadInvalid` when the confirmed ids are not strictly ascending,
/// `ReferenceMissing` when the scenario does not exist, `StateConflict` when it is not applied,
/// when a later apply sits on top of it, or when the active descendants are no longer the ones
/// confirmed (another device changed a promoted item meanwhile). Nothing is written then.
pub(crate) fn revert_scenario_apply_confirmed(
    tx: &Transaction<'_>,
    scenario_id: Uuid,
    confirmed_descendants: &[Uuid],
    now: &str,
) -> Result<Uuid, DbError> {
    if !confirmed_descendants
        .windows(2)
        .all(|pair| pair[0] < pair[1])
    {
        return Err(DbError::reject(
            RejectCode::PayloadInvalid,
            "the confirmed changes must be listed once each, in order".to_owned(),
        ));
    }
    let applied_at: Option<Option<String>> = tx
        .query_row(
            "SELECT applied_at FROM scenarios WHERE id = ?1",
            params![scenario_id],
            |row| row.get(0),
        )
        .optional()?;
    match applied_at {
        None => {
            return Err(DbError::reject(
                RejectCode::ReferenceMissing,
                "no such scenario".to_owned(),
            ))
        }
        Some(None) => {
            return Err(DbError::reject(
                RejectCode::StateConflict,
                "this scenario is not applied".to_owned(),
            ))
        }
        Some(Some(_)) => {}
    }

    let plan = plan_revert(tx, scenario_id)?;
    if plan.blocker.is_some() {
        return Err(DbError::reject(
            RejectCode::StateConflict,
            "a scenario applied later has to be undone first".to_owned(),
        ));
    }
    if !plan
        .descendants
        .iter()
        .copied()
        .eq(confirmed_descendants.iter().copied())
    {
        return Err(DbError::reject(
            RejectCode::StateConflict,
            "the changes made since this scenario was applied are not the ones confirmed"
                .to_owned(),
        ));
    }

    for id in &plan.descendants {
        tx.execute(
            "UPDATE forecast_assumption_events SET status = 'cleared', updated_at = ?2
              WHERE id = ?1",
            params![id, now],
        )?;
    }
    for id in &plan.restore {
        tx.execute(
            "UPDATE forecast_assumption_events
                SET status = 'active', superseded_by = NULL, updated_at = ?2
              WHERE id = ?1",
            params![id, now],
        )?;
    }
    // The promoted rows stay as history, marked `cleared` (nothing is deleted, AGENTS.md section 1).
    tx.execute(
        "UPDATE forecast_assumption_events SET status = 'cleared', updated_at = ?2
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
