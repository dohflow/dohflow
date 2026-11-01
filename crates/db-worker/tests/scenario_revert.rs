//! Undoing an applied scenario follows what the person did to its promoted items afterwards
//! (personal-cfo-5ie.12; ADR 0055 section 5 as amended). The undo clears the active descendants of
//! each promoted row, restores the base events the apply superseded, refuses while a later apply
//! sits on top, and keeps a newer base value the person recorded since.

mod common;

use chrono::{Duration, Utc};
use common::*;
use core_money::{Currency, Money};
use db_worker::*;
use uuid::Uuid;

const RATE: AssumptionKind = AssumptionKind::InflationRate;

fn rate(bps: u32) -> String {
    format!(r#"{{"annual_bps":{bps}}}"#)
}

fn event(
    id: Uuid,
    kind: AssumptionKind,
    params: &str,
    scenario: Option<Uuid>,
) -> NewAssumptionEvent {
    NewAssumptionEvent {
        id,
        kind,
        target_entity_type: None,
        target_entity_id: None,
        params_json: params.to_owned(),
        source: AssumptionSource::UserOverride,
        scenario_id: scenario,
    }
}

fn record(worker: &DbWorker, event: NewAssumptionEvent) {
    worker
        .dispatch(meta(), WriteCommand::RecordAssumptionEvent { event })
        .unwrap();
}

fn scenario_with(worker: &DbWorker, events: &[(AssumptionKind, String)]) -> Uuid {
    let id = Uuid::now_v7();
    worker
        .create_scenario(&NewScenario {
            id,
            name: "Plan".to_owned(),
            description: None,
        })
        .unwrap();
    for (kind, params) in events {
        record(worker, event(Uuid::now_v7(), *kind, params, Some(id)));
    }
    id
}

fn apply(worker: &DbWorker, scenario: Uuid) {
    worker
        .dispatch(
            meta(),
            WriteCommand::ApplyScenario {
                scenario_id: scenario,
            },
        )
        .unwrap();
}

fn revert_with(
    worker: &DbWorker,
    scenario: Uuid,
    confirmed: Vec<Uuid>,
) -> Result<Outcome, DbError> {
    worker.dispatch(
        meta(),
        WriteCommand::RevertScenarioApplyConfirmed {
            scenario_id: scenario,
            confirmed_descendants: confirmed,
        },
    )
}

/// Undo with exactly what the preview lists, the way the interface does.
fn revert(worker: &DbWorker, scenario: Uuid) {
    let preview = worker.revert_preview(scenario).unwrap();
    assert!(preview.blocker.is_none(), "{preview:?}");
    revert_with(worker, scenario, preview.confirmed_descendants).unwrap();
}

fn refused(result: Result<Outcome, DbError>, code: RejectCode) {
    assert!(
        matches!(&result, Err(DbError::Rejected { code: c, .. }) if *c == code),
        "{result:?}"
    );
}

fn base_ids(worker: &DbWorker) -> Vec<Uuid> {
    let mut ids: Vec<Uuid> = worker
        .active_assumption_events(None)
        .unwrap()
        .into_iter()
        .map(|e| e.id)
        .collect();
    ids.sort();
    ids
}

fn base_params(worker: &DbWorker) -> Vec<String> {
    let mut params: Vec<String> = worker
        .active_assumption_events(None)
        .unwrap()
        .into_iter()
        .map(|e| e.params_json)
        .collect();
    params.sort();
    params
}

/// The active base row promoted from `scenario` (there is one in these tests).
fn promoted(worker: &DbWorker, scenario: Uuid) -> Uuid {
    worker
        .active_assumption_events(None)
        .unwrap()
        .into_iter()
        .find(|e| e.promoted_from_scenario_id == Some(scenario))
        .unwrap()
        .id
}

fn edit(worker: &DbWorker, old: Uuid, kind: AssumptionKind, params: &str) -> Uuid {
    let new = Uuid::now_v7();
    worker
        .dispatch(
            meta(),
            WriteCommand::SupersedeAssumptionEvent {
                id: old,
                replacement: event(new, kind, params, None),
            },
        )
        .unwrap();
    new
}

fn clear(worker: &DbWorker, id: Uuid) {
    worker
        .dispatch(meta(), WriteCommand::ClearAssumptionEvent { id })
        .unwrap();
}

/// A base rate (the event the scenario's rate will supersede) and a scenario that sets another.
fn rate_vault() -> (tempfile::TempDir, DbWorker, Uuid, Uuid) {
    let (dir, worker) = worker();
    let original = Uuid::now_v7();
    record(&worker, event(original, RATE, &rate(300), None));
    let scenario = scenario_with(&worker, &[(RATE, rate(400))]);
    (dir, worker, original, scenario)
}

#[test]
fn the_retired_command_still_doubles_an_item_edited_after_the_apply() {
    // The behavior the new command replaces, kept decodable so an operation log written before the
    // fix replays as it did.
    let (_dir, worker, original, scenario) = rate_vault();
    apply(&worker, scenario);
    let edited = edit(&worker, promoted(&worker, scenario), RATE, &rate(450));
    worker
        .dispatch(
            meta(),
            WriteCommand::RevertScenarioApply {
                scenario_id: scenario,
            },
        )
        .unwrap();
    let mut doubled = vec![original, edited];
    doubled.sort();
    assert_eq!(
        base_ids(&worker),
        doubled,
        "two active entries for one rate"
    );
}

#[test]
fn an_edit_made_after_the_apply_is_undone_with_it() {
    let (_dir, worker, original, scenario) = rate_vault();
    apply(&worker, scenario);
    let edited = edit(&worker, promoted(&worker, scenario), RATE, &rate(450));

    let preview = worker.revert_preview(scenario).unwrap();
    assert_eq!(preview.confirmed_descendants, vec![edited]);
    assert_eq!(preview.lines.len(), 1);
    assert_eq!(preview.lines[0].change, RevertChange::Edited);
    assert_eq!(preview.lines[0].after_params_json, Some(rate(450)));

    revert(&worker, scenario);
    assert_eq!(base_ids(&worker), vec![original], "exactly one entry");
    assert!(worker
        .scenario(scenario)
        .unwrap()
        .unwrap()
        .applied_at
        .is_none());
}

#[test]
fn an_edit_chain_of_two_is_followed_to_its_end() {
    let (_dir, worker, original, scenario) = rate_vault();
    apply(&worker, scenario);
    let first = edit(&worker, promoted(&worker, scenario), RATE, &rate(450));
    let second = edit(&worker, first, RATE, &rate(500));
    let preview = worker.revert_preview(scenario).unwrap();
    assert_eq!(preview.confirmed_descendants, vec![second]);
    assert_eq!(preview.lines.len(), 1, "one line per item, not per edit");
    revert(&worker, scenario);
    assert_eq!(base_ids(&worker), vec![original]);
}

#[test]
fn an_item_cleared_after_the_apply_is_listed_and_the_earlier_value_returns() {
    let (_dir, worker, original, scenario) = rate_vault();
    apply(&worker, scenario);
    clear(&worker, promoted(&worker, scenario));
    let preview = worker.revert_preview(scenario).unwrap();
    assert!(preview.confirmed_descendants.is_empty());
    assert_eq!(preview.lines.len(), 1, "the dialog still shows");
    assert_eq!(preview.lines[0].change, RevertChange::Cleared);
    revert(&worker, scenario);
    assert_eq!(base_ids(&worker), vec![original]);
}

#[test]
fn an_untouched_apply_has_no_lines_and_reverts_to_the_prior_base() {
    let (_dir, worker, original, scenario) = rate_vault();
    apply(&worker, scenario);
    let preview = worker.revert_preview(scenario).unwrap();
    assert!(preview.lines.is_empty() && preview.blocker.is_none());
    assert!(preview.confirmed_descendants.is_empty());
    revert_with(&worker, scenario, Vec::new()).unwrap();
    assert_eq!(base_ids(&worker), vec![original]);
}

#[test]
fn an_edited_one_off_does_not_outlive_the_undo() {
    // An adding kind has no base event to restore, but the edit of its promoted row used to stay
    // active for good.
    let (_dir, worker) = worker();
    let day = Utc::now().date_naive() + Duration::days(4);
    worker
        .record_manual_entry(
            Uuid::now_v7(),
            Money::new(-500, Currency::Usd),
            day,
            "Manual",
            None,
        )
        .unwrap();
    let before = base_ids(&worker);
    let probe = Uuid::now_v7();
    worker
        .record_manual_entry(probe, Money::new(-700, Currency::Usd), day, "Plan", None)
        .unwrap();
    let params = worker
        .active_assumption_events(None)
        .unwrap()
        .into_iter()
        .find(|e| e.id == probe)
        .unwrap()
        .params_json;
    clear(&worker, probe);
    let scenario = scenario_with(&worker, &[(AssumptionKind::OneTimeEvent, params.clone())]);
    apply(&worker, scenario);
    let promoted_id = promoted(&worker, scenario);
    let edited = edit(&worker, promoted_id, AssumptionKind::OneTimeEvent, &params);
    assert_eq!(worker.manual_entries().unwrap().len(), 2);

    revert(&worker, scenario);
    assert_eq!(base_ids(&worker), before);
    assert_eq!(worker.manual_entries().unwrap().len(), 1);
    assert!(!base_ids(&worker).contains(&edited));
}

#[test]
fn a_stale_confirmation_is_refused_and_writes_nothing() {
    let (_dir, worker, _original, scenario) = rate_vault();
    apply(&worker, scenario);
    let first = edit(&worker, promoted(&worker, scenario), RATE, &rate(450));
    let confirmed = worker
        .revert_preview(scenario)
        .unwrap()
        .confirmed_descendants;
    assert_eq!(confirmed, vec![first]);
    // Another device edits the item again before this one confirms.
    edit(&worker, first, RATE, &rate(500));
    let before = base_ids(&worker);

    refused(
        revert_with(&worker, scenario, confirmed),
        RejectCode::StateConflict,
    );
    assert_eq!(base_ids(&worker), before, "nothing was written");
    assert!(worker
        .scenario(scenario)
        .unwrap()
        .unwrap()
        .applied_at
        .is_some());
    // Confirming what is there now works.
    revert(&worker, scenario);
}

#[test]
fn a_confirmation_that_names_too_much_or_nothing_is_refused() {
    let (_dir, worker, _original, scenario) = rate_vault();
    apply(&worker, scenario);
    edit(&worker, promoted(&worker, scenario), RATE, &rate(450));
    refused(
        revert_with(&worker, scenario, Vec::new()),
        RejectCode::StateConflict,
    );
    refused(
        revert_with(&worker, scenario, vec![Uuid::now_v7()]),
        RejectCode::StateConflict,
    );
}

#[test]
fn the_confirmed_ids_must_be_strictly_ascending() {
    let (_dir, worker, _original, scenario) = rate_vault();
    apply(&worker, scenario);
    let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
    refused(
        revert_with(&worker, scenario, vec![b, a]),
        RejectCode::PayloadInvalid,
    );
    refused(
        revert_with(&worker, scenario, vec![a, a]),
        RejectCode::PayloadInvalid,
    );
}

#[test]
fn a_missing_or_unapplied_scenario_is_refused() {
    let (_dir, worker, _original, scenario) = rate_vault();
    refused(
        revert_with(&worker, Uuid::now_v7(), Vec::new()),
        RejectCode::ReferenceMissing,
    );
    refused(
        revert_with(&worker, scenario, Vec::new()),
        RejectCode::StateConflict,
    );
    let preview = worker.revert_preview(scenario).unwrap();
    assert!(preview.lines.is_empty() && preview.confirmed_descendants.is_empty());
}

#[test]
fn applies_are_undone_last_first() {
    let (_dir, worker, original, first) = rate_vault();
    let second = scenario_with(&worker, &[(RATE, rate(500))]);
    apply(&worker, first);
    apply(&worker, second);

    let preview = worker.revert_preview(first).unwrap();
    assert_eq!(preview.blocker.as_ref().unwrap().scenario_id, second);
    let before = base_ids(&worker);
    refused(
        revert_with(&worker, first, preview.confirmed_descendants),
        RejectCode::StateConflict,
    );
    assert_eq!(base_ids(&worker), before);

    revert(&worker, second);
    revert(&worker, first);
    assert_eq!(base_ids(&worker), vec![original]);
}

#[test]
fn a_later_apply_over_a_cleared_item_still_blocks_the_earlier_undo() {
    let (_dir, worker, original, first) = rate_vault();
    apply(&worker, first);
    clear(&worker, promoted(&worker, first));
    let second = scenario_with(&worker, &[(RATE, rate(500))]);
    apply(&worker, second);

    let preview = worker.revert_preview(first).unwrap();
    assert_eq!(preview.blocker.as_ref().unwrap().scenario_id, second);
    refused(
        revert_with(&worker, first, Vec::new()),
        RejectCode::StateConflict,
    );

    revert(&worker, second);
    revert(&worker, first);
    assert_eq!(base_ids(&worker), vec![original]);
}

#[test]
fn an_edited_row_of_a_later_apply_still_blocks_the_earlier_undo() {
    let (_dir, worker, original, first) = rate_vault();
    apply(&worker, first);
    clear(&worker, promoted(&worker, first));
    let second = scenario_with(&worker, &[(RATE, rate(500))]);
    apply(&worker, second);
    // The person edits the later apply's row: the edit is a plain base row, but it stands on the
    // later apply.
    let edited = edit(&worker, promoted(&worker, second), RATE, &rate(550));

    let preview = worker.revert_preview(first).unwrap();
    assert_eq!(preview.blocker.as_ref().unwrap().scenario_id, second);
    refused(
        revert_with(&worker, first, Vec::new()),
        RejectCode::StateConflict,
    );

    // Undoing the later apply (which clears the edit) and then the earlier one leaves exactly the
    // original.
    let confirmed = worker.revert_preview(second).unwrap().confirmed_descendants;
    assert_eq!(confirmed, vec![edited]);
    revert_with(&worker, second, confirmed).unwrap();
    revert(&worker, first);
    assert_eq!(base_ids(&worker), vec![original]);
}

#[test]
fn an_edit_left_by_the_retired_revert_is_a_newer_value_not_a_blocker() {
    // A vault that used the retired command can hold the person's edit of a promoted row active
    // after that scenario was undone. The scenario is no longer applied, so it cannot be named as
    // something to undo first; the edit is a plain newer value.
    let (_dir, worker, original, first) = rate_vault();
    apply(&worker, first);
    clear(&worker, promoted(&worker, first));
    let second = scenario_with(&worker, &[(RATE, rate(500))]);
    apply(&worker, second);
    let edited = edit(&worker, promoted(&worker, second), RATE, &rate(550));
    worker
        .dispatch(
            meta(),
            WriteCommand::RevertScenarioApply {
                scenario_id: second,
            },
        )
        .unwrap();
    assert!(worker
        .scenario(second)
        .unwrap()
        .unwrap()
        .applied_at
        .is_none());
    assert_eq!(
        base_ids(&worker),
        vec![edited],
        "the old revert left the edit"
    );

    let preview = worker.revert_preview(first).unwrap();
    assert!(preview.blocker.is_none(), "{preview:?}");
    let kept: Vec<&RevertLine> = preview
        .lines
        .iter()
        .filter(|line| line.change == RevertChange::KeptNewer)
        .collect();
    assert_eq!(kept.len(), 1, "{preview:?}");
    assert_eq!(
        kept[0].after_params_json.as_deref(),
        Some(rate(550).as_str())
    );
    revert(&worker, first);
    assert!(worker
        .scenario(first)
        .unwrap()
        .unwrap()
        .applied_at
        .is_none());
    assert_eq!(base_ids(&worker), vec![edited], "the newer value stays");
    assert!(!base_ids(&worker).contains(&original));
}

#[test]
fn a_row_of_this_scenario_superseded_by_a_later_row_of_it_is_not_listed_as_kept() {
    // Two rates in one scenario collide with each other as they are promoted: the first promoted
    // row is superseded by the second. A plain newer base value then holds the key, and the undo
    // keeps it with one line, not one per row.
    let (_dir, worker) = worker();
    let original = Uuid::now_v7();
    record(&worker, event(original, RATE, &rate(300), None));
    let scenario = scenario_with(&worker, &[(RATE, rate(400)), (RATE, rate(420))]);
    apply(&worker, scenario);
    clear(&worker, promoted(&worker, scenario));
    let newer = Uuid::now_v7();
    record(&worker, event(newer, RATE, &rate(350), None));

    let preview = worker.revert_preview(scenario).unwrap();
    let kept = preview
        .lines
        .iter()
        .filter(|line| line.change == RevertChange::KeptNewer)
        .count();
    assert_eq!(kept, 1, "{preview:?}");
    let line = preview
        .lines
        .iter()
        .find(|line| line.change == RevertChange::KeptNewer)
        .unwrap();
    assert_eq!(
        line.before_params_json,
        rate(300),
        "the earlier value is the base rate the person had, never the scenario's own first rate"
    );
    revert(&worker, scenario);
    assert_eq!(base_ids(&worker), vec![newer]);
}

#[test]
fn a_newer_base_value_recorded_since_the_apply_stays() {
    let (_dir, worker, original, scenario) = rate_vault();
    apply(&worker, scenario);
    clear(&worker, promoted(&worker, scenario));
    let newer = Uuid::now_v7();
    record(&worker, event(newer, RATE, &rate(350), None));

    let preview = worker.revert_preview(scenario).unwrap();
    assert!(preview
        .lines
        .iter()
        .any(|line| line.change == RevertChange::KeptNewer
            && line.after_params_json.as_deref() == Some(rate(350).as_str())));
    revert(&worker, scenario);
    assert_eq!(base_ids(&worker), vec![newer]);
    assert!(!base_ids(&worker).contains(&original));
}

#[test]
fn a_deleted_applied_scenario_can_still_be_undone() {
    let (_dir, worker, original, scenario) = rate_vault();
    apply(&worker, scenario);
    worker
        .dispatch(meta(), WriteCommand::DeleteScenario { id: scenario })
        .unwrap();
    assert!(worker.scenario(scenario).unwrap().is_none());
    let listed = worker.deleted_applied_scenarios().unwrap();
    assert_eq!(
        listed.iter().map(|s| s.id).collect::<Vec<_>>(),
        vec![scenario]
    );
    assert!(worker
        .list_scenarios()
        .unwrap()
        .iter()
        .all(|s| s.id != scenario));

    revert(&worker, scenario);
    assert_eq!(base_ids(&worker), vec![original]);
    assert!(worker.deleted_applied_scenarios().unwrap().is_empty());
    assert_eq!(base_params(&worker), vec![rate(300)]);
}
