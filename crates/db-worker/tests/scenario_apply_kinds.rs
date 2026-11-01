//! Applying a scenario promotes events, and only a collision kind replaces a base event
//! (ADR 0055 section 2 as amended by personal-cfo-5ie.13). An adding kind (a one-off, an amount
//! window) leaves the base entries alone, so a user's manual future entries survive, two one-offs
//! in one scenario both land, and Future Cash after an apply equals the preview that was shown.

mod common;

use chrono::{Duration, Utc};
use common::*;
use core_ledger::AccountId;
use core_money::{Currency, Money};
use db_worker::*;
use uuid::Uuid;

fn event(
    id: Uuid,
    kind: AssumptionKind,
    target: Option<Uuid>,
    params: &str,
    scenario: Option<Uuid>,
) -> NewAssumptionEvent {
    NewAssumptionEvent {
        id,
        kind,
        target_entity_type: target.map(|_| "recurring_event".to_owned()),
        target_entity_id: target,
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

fn new_scenario(worker: &DbWorker) -> Uuid {
    let id = Uuid::now_v7();
    worker
        .create_scenario(&NewScenario {
            id,
            name: "Plan".to_owned(),
            description: None,
        })
        .unwrap();
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

fn revert(worker: &DbWorker, scenario: Uuid) {
    worker
        .dispatch(
            meta(),
            WriteCommand::RevertScenarioApply {
                scenario_id: scenario,
            },
        )
        .unwrap();
}

/// The ids of the active base events, sorted.
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

/// A manual future entry's stored params, reused as a scenario one-off's params.
fn one_off_params(worker: &DbWorker, day: chrono::NaiveDate, minor: i64, label: &str) -> String {
    let probe = Uuid::now_v7();
    worker
        .record_manual_entry(probe, Money::new(minor, Currency::Usd), day, label, None)
        .unwrap();
    let params = worker
        .active_assumption_events(None)
        .unwrap()
        .into_iter()
        .find(|e| e.id == probe)
        .unwrap()
        .params_json;
    worker.clear_assumption_event(probe).unwrap();
    params
}

#[test]
fn manual_future_entries_survive_a_scenario_with_one_one_off() {
    let (_dir, worker) = worker();
    let day = Utc::now().date_naive() + Duration::days(10);
    let originals: Vec<Uuid> = (0..3)
        .map(|n| {
            let id = Uuid::now_v7();
            worker
                .record_manual_entry(
                    id,
                    Money::new(-1_000 * (n + 1), Currency::Usd),
                    day,
                    "Manual",
                    None,
                )
                .unwrap();
            id
        })
        .collect();
    let before = base_ids(&worker);
    assert_eq!(before.len(), 3);

    let scenario = new_scenario(&worker);
    let params = one_off_params(&worker, day, -9_000, "Scenario one-off");
    record(
        &worker,
        event(
            Uuid::now_v7(),
            AssumptionKind::OneTimeEvent,
            None,
            &params,
            Some(scenario),
        ),
    );
    apply(&worker, scenario);

    let applied = base_ids(&worker);
    assert_eq!(applied.len(), 4, "the three entries and the promoted one");
    for id in &originals {
        assert!(applied.contains(id), "a manual entry was retired");
    }

    revert(&worker, scenario);
    assert_eq!(
        base_ids(&worker),
        before,
        "revert leaves exactly the entries that were there"
    );
}

#[test]
fn two_one_offs_in_one_scenario_both_land_and_both_clear() {
    let (_dir, worker) = worker();
    let day = Utc::now().date_naive() + Duration::days(5);
    let scenario = new_scenario(&worker);
    for minor in [-2_000, -3_000] {
        let params = one_off_params(&worker, day, minor, "Scenario one-off");
        record(
            &worker,
            event(
                Uuid::now_v7(),
                AssumptionKind::OneTimeEvent,
                None,
                &params,
                Some(scenario),
            ),
        );
    }
    apply(&worker, scenario);
    assert_eq!(base_ids(&worker).len(), 2, "neither superseded the other");
    revert(&worker, scenario);
    assert!(base_ids(&worker).is_empty());
}

#[test]
fn collision_kinds_still_replace_their_base_event_and_revert_restores_it() {
    let (_dir, worker) = worker();
    let bill = Uuid::now_v7();
    let day = Utc::now().date_naive() + Duration::days(20);
    let other_day = day + Duration::days(7);
    let base = [
        (
            AssumptionKind::InflationRate,
            None,
            r#"{"annual_bps":300}"#.to_owned(),
        ),
        (
            AssumptionKind::BillDate,
            Some(bill),
            format!(r#"{{"new_anchor_date":"{day}"}}"#),
        ),
        (
            AssumptionKind::Exclusion,
            Some(bill),
            format!(r#"{{"effective_date":"{day}"}}"#),
        ),
    ];
    let mut originals = Vec::new();
    for (kind, target, params) in &base {
        let id = Uuid::now_v7();
        originals.push(id);
        record(&worker, event(id, *kind, *target, params, None));
    }
    let scenario = new_scenario(&worker);
    let replacements = [
        (
            AssumptionKind::InflationRate,
            None,
            r#"{"annual_bps":400}"#.to_owned(),
        ),
        (
            AssumptionKind::BillDate,
            Some(bill),
            format!(r#"{{"new_anchor_date":"{other_day}"}}"#),
        ),
        (
            AssumptionKind::Exclusion,
            Some(bill),
            format!(r#"{{"effective_date":"{other_day}"}}"#),
        ),
    ];
    for (kind, target, params) in &replacements {
        record(
            &worker,
            event(Uuid::now_v7(), *kind, *target, params, Some(scenario)),
        );
    }
    let before = base_ids(&worker);
    apply(&worker, scenario);

    let applied = base_ids(&worker);
    assert_eq!(applied.len(), 3);
    for id in &originals {
        assert!(!applied.contains(id), "a colliding base event stayed");
    }
    revert(&worker, scenario);
    assert_eq!(base_ids(&worker), before);
}

#[test]
fn a_collision_kind_and_a_one_off_in_one_scenario_each_follow_their_own_rule() {
    let (_dir, worker) = worker();
    let day = Utc::now().date_naive() + Duration::days(3);
    let rate = Uuid::now_v7();
    record(
        &worker,
        event(
            rate,
            AssumptionKind::InflationRate,
            None,
            r#"{"annual_bps":300}"#,
            None,
        ),
    );
    let manual = Uuid::now_v7();
    worker
        .record_manual_entry(manual, Money::new(-500, Currency::Usd), day, "Manual", None)
        .unwrap();
    let before = base_ids(&worker);

    let scenario = new_scenario(&worker);
    record(
        &worker,
        event(
            Uuid::now_v7(),
            AssumptionKind::InflationRate,
            None,
            r#"{"annual_bps":400}"#,
            Some(scenario),
        ),
    );
    let params = one_off_params(&worker, day, -700, "Scenario one-off");
    record(
        &worker,
        event(
            Uuid::now_v7(),
            AssumptionKind::OneTimeEvent,
            None,
            &params,
            Some(scenario),
        ),
    );
    apply(&worker, scenario);

    let applied = base_ids(&worker);
    assert_eq!(applied.len(), 3);
    assert!(applied.contains(&manual), "the manual entry stayed");
    assert!(!applied.contains(&rate), "the old rate was replaced");

    revert(&worker, scenario);
    assert_eq!(base_ids(&worker), before);
}

#[test]
fn amount_windows_apply_as_the_preview_showed_and_revert_restores_the_base() {
    let (_dir, worker) = worker();
    let account = AccountId::new();
    worker
        .dispatch(
            meta(),
            liquid_account_cmd(account, Currency::Usd, 5_000_000),
        )
        .unwrap();
    let WriteCommand::CreateRecurringBill { event_id, .. } = bill_cmd(None) else {
        unreachable!()
    };
    let mut create = bill_cmd(Some(account));
    if let WriteCommand::CreateRecurringBill {
        event_id: id,
        anchor,
        ..
    } = &mut create
    {
        *id = event_id;
        *anchor = Utc::now().date_naive() + Duration::days(2);
    }
    worker.dispatch(meta(), create).unwrap();
    let bill = event_id.as_uuid();

    let today = Utc::now().date_naive();
    let window = |from: i64, to: i64, minor: i64| {
        format!(
            r#"{{"new_amount_minor":{minor},"effective_date":"{}","end_date":"{}"}}"#,
            today + Duration::days(from),
            today + Duration::days(to)
        )
    };
    // A base window the scenario partly overlaps, and the scenario's two windows: one overlapping
    // the base window (the scenario wins there) and one disjoint from it. The second is created
    // after the first in the scenario, so the scenario's own order must survive the promotion.
    let base_window = Uuid::now_v7();
    record(
        &worker,
        event(
            base_window,
            AssumptionKind::BillAmount,
            Some(bill),
            &window(0, 70, 150_000),
            None,
        ),
    );
    let scenario = new_scenario(&worker);
    for (from, to, minor) in [(40, 100, 90_000), (80, 110, 70_000)] {
        record(
            &worker,
            event(
                Uuid::now_v7(),
                AssumptionKind::BillAmount,
                Some(bill),
                &window(from, to, minor),
                Some(scenario),
            ),
        );
    }

    let base_forecast = worker.future_cash_forecast(120, &[]).unwrap();
    let preview = worker.future_cash_forecast(120, &[scenario]).unwrap();
    assert_ne!(preview, base_forecast, "the scenario must change something");
    let before = base_ids(&worker);

    apply(&worker, scenario);
    assert!(
        base_ids(&worker).contains(&base_window),
        "an amount window is an adding kind: the base window stays"
    );
    assert_eq!(
        worker.future_cash_forecast(120, &[]).unwrap(),
        preview,
        "Future Cash after the apply is the preview the user saw"
    );

    revert(&worker, scenario);
    assert_eq!(base_ids(&worker), before);
    assert_eq!(
        worker.future_cash_forecast(120, &[]).unwrap(),
        base_forecast
    );
}
