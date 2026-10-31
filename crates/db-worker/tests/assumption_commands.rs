//! Assumption events through the bus (personal-cfo-5ymg8.4; ADR 0026 section 4; ADR 0073
//! Addendum G): an edit is ONE command, supersede and clear refuse an event that is no longer
//! active, and a command-written event carries the command's `issued_at` and no forecast run.

mod common;

use chrono::NaiveDate;
use common::KEY;
use core_money::{Currency, Money};
use db_worker::{
    ActorType, AssumptionKind, AssumptionSource, CommandMeta, DbError, DbWorker, IssuedAt,
    NewAssumptionEvent, RejectCode, WriteCommand,
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

fn event(n: u128) -> NewAssumptionEvent {
    NewAssumptionEvent {
        id: u(n),
        kind: AssumptionKind::OneTimeEvent,
        target_entity_type: None,
        target_entity_id: None,
        params_json: format!(r#"{{"n":"{n}"}}"#),
        source: AssumptionSource::UserOverride,
        scenario_id: None,
    }
}

fn worker() -> (tempfile::TempDir, DbWorker) {
    let dir = tempfile::tempdir().unwrap();
    let worker = DbWorker::open(dir.path().join("v.vault"), KEY).unwrap();
    (dir, worker)
}

fn record(worker: &DbWorker, n: u128, offset: i64) {
    worker
        .dispatch(
            meta(n, at(offset)),
            WriteCommand::RecordAssumptionEvent { event: event(n) },
        )
        .unwrap();
}

type Row = (String, Option<Uuid>, Option<Uuid>, String, String);

/// (status, superseded_by, origin_run_id, created_at, updated_at) of one event.
fn row(worker: &DbWorker, n: u128) -> Row {
    worker
        .read_connection()
        .unwrap()
        .query_row(
            "SELECT status, superseded_by, origin_run_id, created_at, updated_at
               FROM forecast_assumption_events WHERE id = ?1",
            [u(n)],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap()
}

fn active_ids(worker: &DbWorker) -> Vec<Uuid> {
    worker
        .active_assumption_events(None)
        .unwrap()
        .into_iter()
        .map(|e| e.id)
        .collect()
}

fn logged(worker: &DbWorker) -> Vec<String> {
    let conn = worker.read_connection().unwrap();
    let mut stmt = conn
        .prepare("SELECT operation_type FROM operation_log ORDER BY rowid")
        .unwrap();
    let rows = stmt.query_map([], |r| r.get(0)).unwrap();
    rows.map(Result::unwrap).collect()
}

#[test]
fn a_recorded_event_is_active_stamped_with_the_commands_instant_and_names_no_run() {
    let (_dir, worker) = worker();
    record(&worker, 1, 0);
    let (status, superseded_by, origin_run, created, updated) = row(&worker, 1);
    assert_eq!(status, "active");
    assert_eq!((superseded_by, origin_run), (None, None));
    assert!(created.starts_with("2026-10-20T00:00:00"), "{created}");
    assert_eq!(created, updated);
    assert_eq!(active_ids(&worker), [u(1)]);
}

#[test]
fn an_edit_is_one_command_and_leaves_exactly_one_active_event() {
    let (_dir, worker) = worker();
    let amount = Money::new(500_000, Currency::Usd);
    let day = NaiveDate::from_ymd_opt(2026, 12, 1).unwrap();
    worker
        .record_manual_entry(u(1), amount, day, "Bonus", None)
        .unwrap();
    let before = logged(&worker).len();
    // The producer's edit path: one call, one command, no separate record.
    worker
        .supersede_manual_entry(
            u(1),
            u(2),
            Money::new(700_000, Currency::Usd),
            day,
            "Bigger",
            None,
        )
        .unwrap();
    let commands = logged(&worker);
    assert_eq!(
        &commands[before..],
        ["supersede_assumption_event"],
        "an edit logs one command and never a record followed by a supersede"
    );
    assert_eq!(active_ids(&worker), [u(2)]);
    let (status, superseded_by, ..) = row(&worker, 1);
    assert_eq!((status.as_str(), superseded_by), ("superseded", Some(u(2))));
}

#[test]
fn a_second_supersede_of_one_event_is_refused_and_leaves_one_active_event() {
    let (_dir, worker) = worker();
    record(&worker, 1, 0);
    worker
        .dispatch(
            meta(2, at(1)),
            WriteCommand::SupersedeAssumptionEvent {
                id: u(1),
                replacement: event(2),
            },
        )
        .unwrap();
    // Another device's concurrent edit of the same event: the old event is no longer active.
    let second = worker.dispatch(
        meta(3, at(2)),
        WriteCommand::SupersedeAssumptionEvent {
            id: u(1),
            replacement: event(3),
        },
    );
    assert!(
        matches!(
            second,
            Err(DbError::Rejected {
                code: RejectCode::StateConflict,
                ..
            })
        ),
        "{second:?}"
    );
    assert_eq!(active_ids(&worker), [u(2)], "the refusal wrote nothing");
    assert!(worker
        .read_connection()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) = 0 FROM forecast_assumption_events WHERE id = ?1",
            [u(3)],
            |r| r.get::<_, bool>(0)
        )
        .unwrap());
}

#[test]
fn clear_deactivates_keeps_the_row_and_refuses_an_unknown_or_inactive_event() {
    let (_dir, worker) = worker();
    record(&worker, 1, 0);
    worker
        .dispatch(
            meta(2, at(5)),
            WriteCommand::ClearAssumptionEvent { id: u(1) },
        )
        .unwrap();
    let (status, _, _, created, updated) = row(&worker, 1);
    assert_eq!(status, "cleared");
    assert_ne!(
        created, updated,
        "updated_at moved to the clearing command's instant"
    );
    assert!(active_ids(&worker).is_empty());
    for (n, id, code) in [
        (3, u(1), RejectCode::StateConflict),
        (4, u(99), RejectCode::ReferenceMissing),
    ] {
        let again = worker.dispatch(meta(n, at(6)), WriteCommand::ClearAssumptionEvent { id });
        assert!(
            matches!(&again, Err(DbError::Rejected { code: c, .. }) if *c == code),
            "{again:?}"
        );
    }
}
