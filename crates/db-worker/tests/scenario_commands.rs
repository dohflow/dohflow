//! Scenarios through the bus (personal-cfo-5ymg8.4; ADR 0051; ADR 0073 Addendum H): creation and
//! the field commands write the command's `issued_at` and no forecast run, a delete is a one-row
//! tombstone that a reinstate undoes whole, a clone copies exactly the events its payload names
//! with ids derived from the command, and two devices that disagree are refused deterministically.

mod common;

use chrono::NaiveDate;
use common::KEY;
use db_worker::{
    ActorType, AssumptionKind, AssumptionSource, CommandMeta, DbError, DbWorker, IssuedAt,
    NewAssumptionEvent, NewScenario, RejectCode, ScenarioStatus, WriteCommand,
};
use uuid::Uuid;

fn u(n: u128) -> Uuid {
    Uuid::from_u128(0x0198_c2c1_5f0a_7c3d_8a4e_9b1d_0000_0000 + n)
}

/// 2026-10-20T00:00:00Z in microseconds, plus `offset` seconds.
fn at(offset: i64) -> IssuedAt {
    IssuedAt::from_unix_micros(1_792_454_400_000_000 + offset * 1_000_000)
}

fn meta(n: u128, issued_at: IssuedAt) -> CommandMeta {
    CommandMeta {
        command_id: u(1_000 + n),
        correlation_id: u(2_000 + n),
        causation_id: None,
        actor_type: ActorType::User,
        actor_id: "tester".to_owned(),
        idempotency_key: format!("00000000-0000-4000-8000-{:012x}", 3_000 + n),
        issued_at,
    }
}

fn worker() -> (tempfile::TempDir, DbWorker) {
    let dir = tempfile::tempdir().unwrap();
    let worker = DbWorker::open(dir.path().join("v.vault"), KEY).unwrap();
    (dir, worker)
}

fn send(worker: &DbWorker, n: u128, offset: i64, command: WriteCommand) {
    worker.dispatch(meta(n, at(offset)), command).unwrap();
}

fn refused(result: Result<db_worker::Outcome, DbError>, code: RejectCode) {
    assert!(
        matches!(&result, Err(DbError::Rejected { code: c, .. }) if *c == code),
        "{result:?}"
    );
}

fn create(worker: &DbWorker, scenario: u128, n: u128, offset: i64) {
    send(
        worker,
        n,
        offset,
        WriteCommand::CreateScenario {
            scenario: NewScenario {
                id: u(scenario),
                name: "Move".to_owned(),
                description: Some("Smaller place".to_owned()),
            },
        },
    );
}

fn event_in(scenario: u128, n: u128) -> WriteCommand {
    WriteCommand::RecordAssumptionEvent {
        event: NewAssumptionEvent {
            id: u(n),
            kind: AssumptionKind::OneTimeEvent,
            target_entity_type: None,
            target_entity_id: None,
            params_json: format!(r#"{{"n":"{n}"}}"#),
            source: AssumptionSource::UserOverride,
            scenario_id: Some(u(scenario)),
        },
    }
}

/// (name, status, expires_on, base_run_id, created_at, updated_at, deleted_at) of one scenario.
type Row = (
    String,
    String,
    Option<String>,
    Option<Uuid>,
    String,
    String,
    Option<String>,
);

fn row(worker: &DbWorker, scenario: u128) -> Row {
    worker
        .read_connection()
        .unwrap()
        .query_row(
            "SELECT name, status, expires_on, base_run_id, created_at, updated_at, deleted_at
               FROM scenarios WHERE id = ?1",
            [u(scenario)],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            },
        )
        .unwrap()
}

/// The ids of the events of a scenario, whatever their status, ascending.
fn events_of(worker: &DbWorker, scenario: u128) -> Vec<Uuid> {
    let conn = worker.read_connection().unwrap();
    let mut stmt = conn
        .prepare("SELECT id FROM forecast_assumption_events WHERE scenario_id = ?1 ORDER BY id")
        .unwrap();
    let rows = stmt.query_map([u(scenario)], |r| r.get(0)).unwrap();
    rows.map(Result::unwrap).collect()
}

#[test]
fn a_created_scenario_is_a_draft_stamped_with_the_commands_instant_and_names_no_run() {
    let (_dir, worker) = worker();
    create(&worker, 1, 1, 0);
    let (name, status, expires_on, run, created, updated, deleted) = row(&worker, 1);
    assert_eq!((name.as_str(), status.as_str()), ("Move", "draft"));
    assert_eq!((expires_on, run, deleted), (None, None, None));
    assert!(created.starts_with("2026-10-20T00:00:00"), "{created}");
    assert_eq!(created, updated);
    let view = worker.scenario(u(1)).unwrap().unwrap();
    assert_eq!(view.description.as_deref(), Some("Smaller place"));
}

#[test]
fn a_name_is_trimmed_and_a_blank_or_over_long_one_is_refused() {
    let (_dir, worker) = worker();
    send(
        &worker,
        1,
        0,
        WriteCommand::CreateScenario {
            scenario: NewScenario {
                id: u(1),
                name: "  Padded  ".to_owned(),
                description: None,
            },
        },
    );
    assert_eq!(row(&worker, 1).0, "Padded");
    for bad in ["   ", &"x".repeat(101), "tab\there"] {
        refused(
            worker.dispatch(
                meta(2, at(1)),
                WriteCommand::RenameScenario {
                    id: u(1),
                    name: bad.to_owned(),
                },
            ),
            RejectCode::PayloadInvalid,
        );
    }
    assert_eq!(
        row(&worker, 1).0,
        "Padded",
        "a refused rename wrote nothing"
    );
}

#[test]
fn the_field_commands_set_the_field_and_setting_what_is_already_set_writes_nothing() {
    let (_dir, worker) = worker();
    create(&worker, 1, 1, 0);
    let date = NaiveDate::from_ymd_opt(2027, 6, 30);
    let commands = [
        WriteCommand::SetScenarioStatus {
            id: u(1),
            status: ScenarioStatus::Active,
        },
        WriteCommand::RenameScenario {
            id: u(1),
            name: "Smaller place".to_owned(),
        },
        WriteCommand::SetScenarioExpiry {
            id: u(1),
            expires_on: date,
        },
    ];
    for (n, command) in commands.iter().enumerate() {
        send(&worker, 10 + n as u128, 10, command.clone());
    }
    let (name, status, expires_on, _, _, updated, _) = row(&worker, 1);
    assert_eq!(
        (name.as_str(), status.as_str(), expires_on.as_deref()),
        ("Smaller place", "active", Some("2027-06-30"))
    );
    assert!(updated.starts_with("2026-10-20T00:00:10"), "{updated}");

    // The same three again, later: no row changes, not even `updated_at`.
    let before = row(&worker, 1);
    for (n, command) in commands.iter().enumerate() {
        send(&worker, 20 + n as u128, 60, command.clone());
        assert!(
            worker.last_write_set().unwrap().rows.is_empty(),
            "{command:?} wrote a row"
        );
    }
    assert_eq!(row(&worker, 1), before);

    send(
        &worker,
        30,
        90,
        WriteCommand::SetScenarioExpiry {
            id: u(1),
            expires_on: None,
        },
    );
    assert_eq!(row(&worker, 1).2, None, "an expiry can be cleared");
}

#[test]
fn an_expiry_outside_the_supported_years_is_refused() {
    let (_dir, worker) = worker();
    create(&worker, 1, 1, 0);
    for date in [
        NaiveDate::from_ymd_opt(1899, 12, 31),
        NaiveDate::from_ymd_opt(10_000, 1, 1),
    ] {
        refused(
            worker.dispatch(
                meta(2, at(1)),
                WriteCommand::SetScenarioExpiry {
                    id: u(1),
                    expires_on: date,
                },
            ),
            RejectCode::PayloadInvalid,
        );
    }
    assert_eq!(row(&worker, 1).2, None);
}

#[test]
fn a_delete_writes_one_row_and_a_reinstate_restores_the_scenario_with_its_events_and_status() {
    let (_dir, worker) = worker();
    create(&worker, 1, 1, 0);
    send(
        &worker,
        2,
        1,
        WriteCommand::SetScenarioStatus {
            id: u(1),
            status: ScenarioStatus::Archived,
        },
    );
    send(&worker, 3, 2, event_in(1, 11));
    send(&worker, 4, 3, event_in(1, 12));

    send(&worker, 5, 10, WriteCommand::DeleteScenario { id: u(1) });
    assert_eq!(
        worker.last_write_set().unwrap().rows.len(),
        1,
        "a delete touches the scenario row and nothing else"
    );
    let (.., updated, deleted) = row(&worker, 1);
    assert_eq!(deleted.as_deref(), Some(updated.as_str()));
    assert!(updated.starts_with("2026-10-20T00:00:10"), "{updated}");
    assert_eq!(events_of(&worker, 1), [u(11), u(12)], "its events stay");

    assert!(worker.scenario(u(1)).unwrap().is_none());
    assert!(worker.list_scenarios().unwrap().is_empty());
    assert!(worker
        .active_assumption_events(Some(u(1)))
        .unwrap()
        .is_empty());

    send(&worker, 6, 20, WriteCommand::ReinstateScenario { id: u(1) });
    assert_eq!(worker.last_write_set().unwrap().rows.len(), 1);
    let back = worker.scenario(u(1)).unwrap().unwrap();
    assert_eq!(back.status, ScenarioStatus::Archived, "the prior status");
    assert_eq!(back.event_count, 2, "the events came back with it");
    assert_eq!(row(&worker, 1).6, None);
}

#[test]
fn deleting_a_deleted_scenario_and_reinstating_a_live_one_write_nothing() {
    let (_dir, worker) = worker();
    create(&worker, 1, 1, 0);
    send(&worker, 2, 5, WriteCommand::ReinstateScenario { id: u(1) });
    assert!(worker.last_write_set().unwrap().rows.is_empty());
    send(&worker, 3, 10, WriteCommand::DeleteScenario { id: u(1) });
    let deleted = row(&worker, 1);
    send(&worker, 4, 99, WriteCommand::DeleteScenario { id: u(1) });
    assert!(worker.last_write_set().unwrap().rows.is_empty());
    assert_eq!(
        row(&worker, 1),
        deleted,
        "the first delete's instant stands"
    );
}

#[test]
fn a_deleted_scenario_refuses_every_command_that_needs_it_live_but_not_a_revert() {
    let (_dir, worker) = worker();
    create(&worker, 1, 1, 0);
    send(&worker, 2, 1, event_in(1, 11));
    send(
        &worker,
        3,
        2,
        WriteCommand::ApplyScenario { scenario_id: u(1) },
    );
    send(&worker, 4, 3, WriteCommand::DeleteScenario { id: u(1) });

    let live_only = [
        WriteCommand::SetScenarioStatus {
            id: u(1),
            status: ScenarioStatus::Active,
        },
        WriteCommand::RenameScenario {
            id: u(1),
            name: "Again".to_owned(),
        },
        WriteCommand::SetScenarioExpiry {
            id: u(1),
            expires_on: NaiveDate::from_ymd_opt(2030, 1, 1),
        },
        WriteCommand::CloneScenario {
            source_id: u(1),
            new_id: u(2),
            name: "Copy".to_owned(),
            event_ids: vec![u(11)],
        },
        WriteCommand::ApplyScenario { scenario_id: u(1) },
        event_in(1, 12),
    ];
    for (n, command) in live_only.into_iter().enumerate() {
        refused(
            worker.dispatch(meta(10 + n as u128, at(10)), command),
            RejectCode::StateConflict,
        );
    }

    // Undoing an apply stays possible for a scenario that was deleted afterward.
    send(
        &worker,
        20,
        20,
        WriteCommand::RevertScenarioApply { scenario_id: u(1) },
    );
    assert!(worker.active_assumption_events(None).unwrap().is_empty());
}

#[test]
fn a_clone_copies_exactly_the_named_events_even_one_cleared_since() {
    let (_dir, worker) = worker();
    create(&worker, 1, 1, 0);
    send(&worker, 2, 1, event_in(1, 11));
    send(&worker, 3, 2, event_in(1, 12));
    send(&worker, 4, 3, event_in(1, 13));
    // The user saw 11 and 12, then 11 was cleared before the clone landed.
    send(
        &worker,
        5,
        4,
        WriteCommand::ClearAssumptionEvent { id: u(11) },
    );
    send(
        &worker,
        6,
        10,
        WriteCommand::CloneScenario {
            source_id: u(1),
            new_id: u(2),
            name: "Copy".to_owned(),
            event_ids: vec![u(11), u(12)],
        },
    );

    let copied = events_of(&worker, 2);
    assert_eq!(copied.len(), 2, "event 13 was not named");
    assert_eq!(row(&worker, 2).1, "draft");
    assert_eq!(row(&worker, 2).2, None, "no expiry is carried");
    let conn = worker.read_connection().unwrap();
    for id in &copied {
        let (status, created, updated, run, superseded): (
            String,
            String,
            String,
            Option<Uuid>,
            Option<Uuid>,
        ) = conn
            .query_row(
                "SELECT status, created_at, updated_at, origin_run_id, superseded_by
                   FROM forecast_assumption_events WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .unwrap();
        assert_eq!((status.as_str(), run, superseded), ("active", None, None));
        assert!(
            created.starts_with("2026-10-20T00:00:01")
                || created.starts_with("2026-10-20T00:00:02"),
            "created_at is the source's, not the clone's: {created}"
        );
        assert!(updated.starts_with("2026-10-20T00:00:10"), "{updated}");
    }
}

#[test]
fn a_clone_naming_a_foreign_event_or_a_repeat_is_refused_and_writes_nothing() {
    let (_dir, worker) = worker();
    create(&worker, 1, 1, 0);
    create(&worker, 2, 2, 0);
    send(&worker, 3, 1, event_in(1, 11));
    send(&worker, 4, 1, event_in(2, 21));
    let clone = |events: Vec<Uuid>| WriteCommand::CloneScenario {
        source_id: u(1),
        new_id: u(3),
        name: "Copy".to_owned(),
        event_ids: events,
    };
    refused(
        worker.dispatch(meta(10, at(5)), clone(vec![u(11), u(21)])),
        RejectCode::ReferenceMissing,
    );
    refused(
        worker.dispatch(meta(11, at(5)), clone(vec![u(11), u(11)])),
        RejectCode::PayloadInvalid,
    );
    let none: Option<Uuid> = worker
        .read_connection()
        .unwrap()
        .query_row("SELECT id FROM scenarios WHERE id = ?1", [u(3)], |r| {
            r.get(0)
        })
        .ok();
    assert_eq!(none, None, "a refused clone left no scenario behind");
    assert!(events_of(&worker, 3).is_empty());
}

/// Two copies of one vault holding the scenario 1 with the events 11 and 12, to play two devices.
fn two_devices() -> (tempfile::TempDir, DbWorker, DbWorker) {
    let (dir, worker) = worker();
    create(&worker, 1, 1, 0);
    send(&worker, 2, 1, event_in(1, 11));
    send(&worker, 3, 2, event_in(1, 12));
    worker.checkpoint_truncate().unwrap();
    let base = dir.path().join("v.vault");
    let copy = |name: &str| {
        let path = dir.path().join(name);
        std::fs::copy(&base, &path).unwrap();
        DbWorker::open(&path, KEY).unwrap()
    };
    let (a, b) = (copy("a.vault"), copy("b.vault"));
    (dir, a, b)
}

#[test]
fn one_clone_command_derives_the_same_ids_and_write_set_on_every_device() {
    let (_dir, a, b) = two_devices();
    let command = WriteCommand::CloneScenario {
        source_id: u(1),
        new_id: u(2),
        name: "Copy".to_owned(),
        event_ids: vec![u(11), u(12)],
    };
    a.dispatch(meta(10, at(10)), command.clone()).unwrap();
    let hash_a = a.last_write_set().unwrap().hash;
    b.dispatch(meta(10, at(10)), command).unwrap();
    assert_eq!(hash_a, b.last_write_set().unwrap().hash);
    assert_eq!(events_of(&a, 2), events_of(&b, 2));
    assert_eq!(events_of(&a, 2).len(), 2);
}

#[test]
fn a_delete_on_one_device_refuses_a_rename_from_the_other_and_not_the_reverse() {
    let (_dir, a, b) = two_devices();
    let delete = WriteCommand::DeleteScenario { id: u(1) };
    let rename = WriteCommand::RenameScenario {
        id: u(1),
        name: "Renamed".to_owned(),
    };
    a.dispatch(meta(10, at(10)), delete.clone()).unwrap();
    b.dispatch(meta(11, at(10)), rename.clone()).unwrap();
    // The rename meets a deleted scenario and is refused; the delete meets a renamed one and lands.
    refused(
        a.dispatch(meta(12, at(11)), rename),
        RejectCode::StateConflict,
    );
    b.dispatch(meta(13, at(11)), delete).unwrap();
    assert!(a.scenario(u(1)).unwrap().is_none());
    assert!(b.scenario(u(1)).unwrap().is_none());
    assert_eq!(row(&a, 1).0, "Move");
    assert_eq!(
        row(&b, 1).0,
        "Renamed",
        "the rename stands under the tombstone"
    );
}

#[test]
fn two_devices_deleting_one_scenario_agree_and_the_second_delete_writes_nothing() {
    let (_dir, a, b) = two_devices();
    let delete = WriteCommand::DeleteScenario { id: u(1) };
    a.dispatch(meta(10, at(10)), delete.clone()).unwrap();
    b.dispatch(meta(11, at(20)), delete.clone()).unwrap();
    let before = row(&a, 1);
    a.dispatch(meta(12, at(30)), delete).unwrap();
    assert_eq!(row(&a, 1), before);
    assert!(a.last_write_set().unwrap().rows.is_empty());
}

#[test]
fn two_devices_creating_the_same_scenario_id_collide_and_the_second_is_refused() {
    let (_dir, a, b) = two_devices();
    let new = |n: u128| {
        (
            meta(n, at(10)),
            WriteCommand::CreateScenario {
                scenario: NewScenario {
                    id: u(5),
                    name: format!("Plan {n}"),
                    description: None,
                },
            },
        )
    };
    let (ma, ca) = new(10);
    let (mb, cb) = new(11);
    a.dispatch(ma, ca).unwrap();
    b.dispatch(mb, cb.clone()).unwrap();
    refused(a.dispatch(meta(12, at(11)), cb), RejectCode::StateConflict);
}
