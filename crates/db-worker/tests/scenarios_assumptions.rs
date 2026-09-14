//! Scenarios, assumption events, manual entries, and forecast state persistence
//! (moved verbatim from the old in-file `lib.rs` tests module).

mod common;

use chrono::NaiveDate;
use common::*;
use core_money::{Currency, Money};
use db_worker::*;
use rusqlite::params;
use uuid::Uuid;

#[test]
fn assumption_events_round_trip_and_survive_rerun() {
    let (_dir, worker) = worker();

    // No events in a fresh vault.
    assert!(worker.active_assumption_events(None).unwrap().is_empty());

    // A base user override reads back as an active event.
    let base = Uuid::now_v7();
    worker
        .record_assumption_event(&new_event(base, AssumptionKind::IncomeAmount, None))
        .unwrap();
    let active = worker.active_assumption_events(None).unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].id, base);
    assert_eq!(active[0].kind, AssumptionKind::IncomeAmount);
    assert_eq!(active[0].status, AssumptionStatus::Active);
    assert_eq!(active[0].source, AssumptionSource::UserOverride);

    // A scenario-scoped event is excluded from the base set and returned for
    // its own scenario.
    let scenario = Uuid::now_v7();
    let scoped = Uuid::now_v7();
    worker
        .record_assumption_event(&new_event(
            scoped,
            AssumptionKind::OneTimeEvent,
            Some(scenario),
        ))
        .unwrap();
    assert_eq!(worker.active_assumption_events(None).unwrap().len(), 1);
    let in_scenario = worker.active_assumption_events(Some(scenario)).unwrap();
    assert_eq!(in_scenario.len(), 1);
    assert_eq!(in_scenario[0].id, scoped);

    // Superseding swaps which event is active but retains the old row.
    let replacement = Uuid::now_v7();
    worker
        .record_assumption_event(&new_event(replacement, AssumptionKind::IncomeAmount, None))
        .unwrap();
    worker
        .supersede_assumption_event(base, replacement)
        .unwrap();
    let after = worker.active_assumption_events(None).unwrap();
    assert_eq!(after.len(), 1, "superseded event must leave the active set");
    assert_eq!(after[0].id, replacement);

    // Clearing deactivates the survivor without deleting it.
    worker.clear_assumption_event(replacement).unwrap();
    assert!(worker.active_assumption_events(None).unwrap().is_empty());
}

#[test]
fn dependency_edges_attribute_rows_to_events() {
    let (_dir, worker) = worker();
    let run = Uuid::now_v7();
    let event = Uuid::now_v7();
    let row = Uuid::now_v7();
    let other_row = Uuid::now_v7();

    worker
        .record_dependency_edges(&[
            DependencyEdge {
                id: Uuid::now_v7(),
                forecast_run_id: run,
                from_event_id: event,
                to_row_id: row,
                edge_type: EdgeType::Affects,
            },
            DependencyEdge {
                id: Uuid::now_v7(),
                forecast_run_id: run,
                from_event_id: Uuid::now_v7(),
                to_row_id: other_row,
                edge_type: EdgeType::DerivesFrom,
            },
        ])
        .unwrap();

    // Only the edge pointing at `row` comes back — the explanation join.
    let edges = worker.dependency_edges_for_row(row).unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].from_event_id, event);
    assert_eq!(edges[0].edge_type, EdgeType::Affects);
}

#[test]
fn dirty_ranges_enqueue_and_resolve() {
    let (_dir, worker) = worker();
    assert!(worker.open_dirty_ranges().unwrap().is_empty());

    let id = Uuid::now_v7();
    worker
        .enqueue_dirty_range(&DirtyRange {
            id,
            from_date: NaiveDate::from_ymd_opt(2026, 6, 1).unwrap(),
            to_date: NaiveDate::from_ymd_opt(2026, 6, 30).unwrap(),
            reason: DirtyReason::AssumptionChanged,
            triggering_event_id: Some(Uuid::now_v7()),
        })
        .unwrap();

    let open = worker.open_dirty_ranges().unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].id, id);
    assert_eq!(open[0].reason, DirtyReason::AssumptionChanged);
    assert_eq!(
        open[0].from_date,
        NaiveDate::from_ymd_opt(2026, 6, 1).unwrap()
    );
    assert_eq!(
        open[0].to_date,
        NaiveDate::from_ymd_opt(2026, 6, 30).unwrap()
    );

    worker.resolve_dirty_range(id).unwrap();
    assert!(worker.open_dirty_ranges().unwrap().is_empty());
}

#[test]
fn input_snapshot_capture_is_content_addressed() {
    let (_dir, worker) = worker();
    let seed = worker.read_connection().unwrap();
    seed.execute(
        "INSERT INTO recurring_events
                (id, name, amount_expected_minor, currency, frequency,
                 next_expected_date, created_at, updated_at)
             VALUES (?1, 'Rent', 180000, 'USD', 'monthly', '2026-07-01', 'now', 'now')",
        params![Uuid::now_v7()],
    )
    .unwrap();
    seed.execute(
        "INSERT INTO income_sources
                (id, name, net_minor_units, currency, frequency, anchor_date, created_at)
             VALUES (?1, 'Acme', 500000, 'USD', 'monthly', '2026-06-15', 'now')",
        params![Uuid::now_v7()],
    )
    .unwrap();

    // Two captures of identical inputs dedup to the same snapshot id.
    let first = worker.capture_input_snapshot().unwrap();
    let again = worker.capture_input_snapshot().unwrap();
    assert_eq!(first, again, "identical inputs must map to one snapshot id");

    // Mutating an input yields a new, distinct snapshot.
    seed.execute(
        "UPDATE recurring_events SET amount_expected_minor = 200000 WHERE name = 'Rent'",
        [],
    )
    .unwrap();
    let changed = worker.capture_input_snapshot().unwrap();
    assert_ne!(
        changed, first,
        "a changed input must produce a new snapshot"
    );

    // The snapshot reads back with its content-addressed fields populated.
    let snap = worker
        .input_snapshot(changed)
        .unwrap()
        .expect("snapshot exists");
    assert!(snap.content_hash.is_some());
    assert!(snap.ledger_cutoff_op_seq.is_some());
    assert!(snap
        .recurring_events_snapshot_json
        .unwrap()
        .contains("Rent"));
}

#[test]
fn model_registry_upserts_and_reads() {
    let (_dir, worker) = worker();
    assert!(worker.model("layer1_deterministic").unwrap().is_none());

    worker
        .register_model("layer1_deterministic", "1", None)
        .unwrap();
    let model = worker.model("layer1_deterministic").unwrap().unwrap();
    assert_eq!(model.version, "1");
    assert_eq!(model.parameters_json, None);

    // Re-registering the same id upserts in place (no duplicate row).
    worker
        .register_model("layer1_deterministic", "2", Some(r#"{"smoothing":3}"#))
        .unwrap();
    let updated = worker.model("layer1_deterministic").unwrap().unwrap();
    assert_eq!(updated.version, "2");
    assert_eq!(
        updated.parameters_json.as_deref(),
        Some(r#"{"smoothing":3}"#)
    );
}

#[test]
fn forecast_diff_round_trip() {
    let (_dir, worker) = worker();
    let run = Uuid::now_v7();
    let prior = Uuid::now_v7();
    worker
        .record_forecast_diff(&NewForecastDiff {
            id: Uuid::now_v7(),
            forecast_run_id: run,
            prior_run_id: Some(prior),
            summary_json: r#"{"min_cash_delta_minor":-5000}"#.to_owned(),
        })
        .unwrap();

    let diffs = worker.forecast_diffs_for_run(run).unwrap();
    assert_eq!(diffs.len(), 1);
    assert_eq!(diffs[0].prior_run_id, Some(prior));
    assert_eq!(diffs[0].summary_json, r#"{"min_cash_delta_minor":-5000}"#);
    // A diff for an unrelated run is not returned.
    assert!(worker.forecast_diffs_for_run(prior).unwrap().is_empty());
}

#[test]
fn scenarios_round_trip_and_lifecycle() {
    let (_dir, worker) = worker();
    assert!(worker.list_scenarios().unwrap().is_empty());

    let id = Uuid::now_v7();
    worker
        .create_scenario(&NewScenario {
            id,
            name: "New job".to_owned(),
            description: Some("+$1k/mo take-home".to_owned()),
            base_run_id: None,
        })
        .unwrap();

    let scenario = worker.scenario(id).unwrap().unwrap();
    assert_eq!(scenario.name, "New job");
    assert_eq!(scenario.status, ScenarioStatus::Draft);
    assert_eq!(worker.list_scenarios().unwrap().len(), 1);

    // Rename (vru6): a new name sticks; a blank name is rejected.
    worker.rename_scenario(id, "New job + raise").unwrap();
    assert_eq!(
        worker.scenario(id).unwrap().unwrap().name,
        "New job + raise"
    );
    assert!(worker.rename_scenario(id, "   ").is_err());
    assert_eq!(
        worker.scenario(id).unwrap().unwrap().name,
        "New job + raise",
        "a rejected rename leaves the name unchanged"
    );

    // Lifecycle: draft -> active -> archived.
    worker
        .set_scenario_status(id, ScenarioStatus::Active)
        .unwrap();
    assert_eq!(
        worker.scenario(id).unwrap().unwrap().status,
        ScenarioStatus::Active
    );
    worker
        .set_scenario_status(id, ScenarioStatus::Archived)
        .unwrap();
    assert_eq!(
        worker.scenario(id).unwrap().unwrap().status,
        ScenarioStatus::Archived
    );
}

#[test]
fn scenario_events_are_scenario_scoped_assumption_events() {
    // The unified model (ADR 0026 §5): a scenario's overlay events are just
    // scenario-scoped assumption events — no separate store.
    let (_dir, worker) = worker();
    let scenario = Uuid::now_v7();
    worker
        .create_scenario(&NewScenario {
            id: scenario,
            name: "Bonus".to_owned(),
            description: None,
            base_run_id: None,
        })
        .unwrap();

    let event = Uuid::now_v7();
    worker
        .record_assumption_event(&NewAssumptionEvent {
            id: event,
            kind: AssumptionKind::OneTimeEvent,
            target_entity_type: None,
            target_entity_id: None,
            params_json: r#"{"amount_minor":500000,"date":"2026-08-01"}"#.to_owned(),
            source: AssumptionSource::UserOverride,
            scenario_id: Some(scenario),
            origin_run_id: None,
        })
        .unwrap();

    // The event belongs to the scenario, not the base set.
    let in_scenario = worker.active_assumption_events(Some(scenario)).unwrap();
    assert_eq!(in_scenario.len(), 1);
    assert_eq!(in_scenario[0].id, event);
    assert!(worker.active_assumption_events(None).unwrap().is_empty());
}

#[test]
fn forecast_state_tables_accept_valid_rows_and_reject_bad_tokens() {
    let (_dir, worker) = worker();
    let conn = worker.read_connection().unwrap();
    let run = Uuid::now_v7();

    conn.execute(
        "INSERT INTO forecast_actuals
                (id, forecast_run_id, realized_date, realized_amount_minor, currency,
                 match_status, created_at)
             VALUES (?1, ?2, '2026-06-20', 100000, 'USD', 'matched', 'now')",
        params![Uuid::now_v7(), run],
    )
    .unwrap();
    assert!(
        conn.execute(
            "INSERT INTO forecast_actuals
                    (id, forecast_run_id, realized_date, realized_amount_minor, currency,
                     match_status, created_at)
                 VALUES (?1, ?2, '2026-06-20', 100000, 'USD', 'bogus', 'now')",
            params![Uuid::now_v7(), run],
        )
        .is_err(),
        "an invalid match_status is rejected by the CHECK"
    );

    conn.execute(
        "INSERT INTO forecast_quality_scores
                (id, forecast_run_id, metric_type, score_bps, sample_size, computed_at)
             VALUES (?1, ?2, 'mape', 850, 30, 'now')",
        params![Uuid::now_v7(), run],
    )
    .unwrap();
    assert!(
        conn.execute(
            "INSERT INTO forecast_quality_scores
                    (id, forecast_run_id, metric_type, score_bps, sample_size, computed_at)
                 VALUES (?1, ?2, 'bogus', 850, 30, 'now')",
            params![Uuid::now_v7(), run],
        )
        .is_err(),
        "an invalid metric_type is rejected"
    );

    conn.execute(
            "INSERT INTO forecast_backtest_results
                (id, model_id, as_of_date, horizon_days, metric_type, score_bps, sample_size, created_at)
             VALUES (?1, 'layer1_deterministic', '2026-01-01', 30, 'mae', 500, 12, 'now')",
            params![Uuid::now_v7()],
        )
        .unwrap();

    conn.execute(
        "INSERT INTO risk_flags
                (id, forecast_run_id, flag_type, severity, readiness_required_bps, created_at)
             VALUES (?1, ?2, 'low_balance', 'warning', 0, 'now')",
        params![Uuid::now_v7(), run],
    )
    .unwrap();
    assert!(
        conn.execute(
            "INSERT INTO risk_flags
                    (id, forecast_run_id, flag_type, severity, readiness_required_bps, created_at)
                 VALUES (?1, ?2, 'low_balance', 'apocalyptic', 0, 'now')",
            params![Uuid::now_v7(), run],
        )
        .is_err(),
        "an invalid severity is rejected"
    );

    let actuals: i64 = conn
        .query_row("SELECT COUNT(*) FROM forecast_actuals", [], |r| r.get(0))
        .unwrap();
    assert_eq!(actuals, 1, "only the valid actual persisted");
}

#[test]
fn manual_entries_list_round_trips() {
    let (_dir, worker) = worker();
    assert!(worker.manual_entries().unwrap().is_empty());

    let id = Uuid::now_v7();
    let date = NaiveDate::from_ymd_opt(2026, 9, 15).unwrap();
    worker
        .record_manual_entry(
            id,
            Money::new(-25_000, Currency::Usd),
            date,
            "Vet bill",
            None,
        )
        .unwrap();

    let entries = worker.manual_entries().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, id);
    assert_eq!(entries[0].amount.minor_units(), -25_000);
    assert_eq!(entries[0].occurs_on, date);
    assert_eq!(entries[0].label, "Vet bill");
}

/// ADR 0051 §1: archive KEEPS every event (it is a filing action), while delete
/// removes the scenario and cascades to its overlay. Before ADR 0051 these were the
/// same call and archiving silently cleared the user's work.
#[test]
fn archive_keeps_events_and_delete_cascades() {
    let (_dir, worker) = worker();
    let scenario = Uuid::now_v7();
    worker
        .create_scenario(&NewScenario {
            id: scenario,
            name: "Maternity leave".to_owned(),
            description: None,
            base_run_id: None,
        })
        .unwrap();
    let base_event = Uuid::now_v7();
    worker
        .record_assumption_event(&new_event(base_event, AssumptionKind::IncomeAmount, None))
        .unwrap();
    let scoped = Uuid::now_v7();
    worker
        .record_assumption_event(&new_event(
            scoped,
            AssumptionKind::OneTimeEvent,
            Some(scenario),
        ))
        .unwrap();
    assert_eq!(worker.scenario(scenario).unwrap().unwrap().event_count, 1);

    // Archive: status flips, the overlay survives intact, and restoring returns it.
    worker.archive_scenario(scenario).unwrap();
    let archived = worker.scenario(scenario).unwrap().unwrap();
    assert_eq!(archived.status, ScenarioStatus::Archived);
    assert_eq!(
        archived.event_count, 1,
        "archiving must NOT clear the scenario's events"
    );
    assert_eq!(
        worker
            .active_assumption_events(Some(scenario))
            .unwrap()
            .len(),
        1
    );
    worker
        .set_scenario_status(scenario, ScenarioStatus::Draft)
        .unwrap();
    assert_eq!(
        worker
            .active_assumption_events(Some(scenario))
            .unwrap()
            .len(),
        1,
        "a restored scenario is whole"
    );

    // Delete: the scenario and its overlay are gone; the BASE event is untouched.
    worker.delete_scenario(scenario).unwrap();
    assert!(worker.scenario(scenario).unwrap().is_none());
    assert!(worker
        .active_assumption_events(Some(scenario))
        .unwrap()
        .is_empty());
    let base = worker.active_assumption_events(None).unwrap();
    assert_eq!(base.len(), 1, "a scenario delete never touches base events");
    assert_eq!(base[0].id, base_event);

    // Deleting a scenario that is not there is an error, not a silent no-op.
    assert!(worker.delete_scenario(scenario).is_err());
}

/// ADR 0051 §2: a clone is a fresh DRAFT carrying its own copy of every active event.
#[test]
fn clone_copies_the_overlay_into_a_new_draft() {
    let (_dir, worker) = worker();
    let source = Uuid::now_v7();
    worker
        .create_scenario(&NewScenario {
            id: source,
            name: "New job".to_owned(),
            description: Some("+$1k/mo".to_owned()),
            base_run_id: None,
        })
        .unwrap();
    let kept = Uuid::now_v7();
    worker
        .record_assumption_event(&new_event(kept, AssumptionKind::OneTimeEvent, Some(source)))
        .unwrap();
    worker
        .set_scenario_status(source, ScenarioStatus::Active)
        .unwrap();

    let copy = Uuid::now_v7();
    worker
        .clone_scenario(source, copy, "New job (copy)")
        .unwrap();

    let cloned = worker.scenario(copy).unwrap().unwrap();
    assert_eq!(cloned.name, "New job (copy)");
    assert_eq!(cloned.description.as_deref(), Some("+$1k/mo"));
    assert_eq!(
        cloned.status,
        ScenarioStatus::Draft,
        "a copy of an active plan is not itself active"
    );
    assert_eq!(cloned.event_count, 1);

    // The copied event is a DISTINCT row carrying the same content — editing the copy
    // must not reach back into the source.
    let copied = worker.active_assumption_events(Some(copy)).unwrap();
    assert_eq!(copied.len(), 1);
    assert_ne!(copied[0].id, kept, "the clone's event needs its own id");
    let original = worker.active_assumption_events(Some(source)).unwrap();
    assert_eq!(original.len(), 1);
    assert_eq!(copied[0].kind, original[0].kind);
    assert_eq!(copied[0].params_json, original[0].params_json);
    assert_eq!(copied[0].source, original[0].source);

    assert!(worker
        .clone_scenario(Uuid::now_v7(), Uuid::now_v7(), "x")
        .is_err());
    assert!(worker.clone_scenario(source, Uuid::now_v7(), "  ").is_err());
}

/// ADR 0051 §3: expiry is stored as a plain date and read back; nothing flips status.
#[test]
fn expiry_round_trips_without_touching_status() {
    let (_dir, worker) = worker();
    let id = Uuid::now_v7();
    worker
        .create_scenario(&NewScenario {
            id,
            name: "Summer trip".to_owned(),
            description: None,
            base_run_id: None,
        })
        .unwrap();
    assert!(worker.scenario(id).unwrap().unwrap().expires_on.is_none());

    worker.set_scenario_expiry(id, Some("2026-09-01")).unwrap();
    let with_expiry = worker.scenario(id).unwrap().unwrap();
    assert_eq!(with_expiry.expires_on.as_deref(), Some("2026-09-01"));
    assert_eq!(
        with_expiry.status,
        ScenarioStatus::Draft,
        "expiry is a read-time filter, never a stored status flip"
    );

    worker.set_scenario_expiry(id, None).unwrap();
    assert!(worker.scenario(id).unwrap().unwrap().expires_on.is_none());
}
