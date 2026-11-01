//! Which assumption kinds supersede on promotion (ADR 0055 section 2 as amended by
//! personal-cfo-5ie.13): the classification function agrees with the ADR table row by row, and the
//! read-only count finds exactly the base events an earlier apply wrongly retired.

mod common;

use common::KEY;
use db_worker::{AssumptionKind, DbWorker};
use uuid::Uuid;

fn u(n: u128) -> Uuid {
    Uuid::from_u128(0x0198_c2c1_5f0a_7c3d_8a4e_9b1d_0000_0000 + n)
}

fn worker() -> (tempfile::TempDir, DbWorker) {
    let dir = tempfile::tempdir().unwrap();
    let worker = DbWorker::open(dir.path().join("v.vault"), KEY).unwrap();
    (dir, worker)
}

/// The `(token, COLLIDES|ADDS)` rows of the amendment's table in ADR 0055.
fn adr_table() -> Vec<(String, String)> {
    let adr = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/adr/0055-applying-a-scenario.md"),
    )
    .unwrap();
    adr.lines()
        .filter_map(|line| {
            let cells: Vec<&str> = line.split('|').map(str::trim).collect();
            let [_, kind, class, ..] = cells.as_slice() else {
                return None;
            };
            let token = kind.strip_prefix('`')?.strip_suffix('`')?;
            matches!(*class, "COLLIDES" | "ADDS").then(|| (token.to_owned(), (*class).to_owned()))
        })
        .collect()
}

#[test]
fn the_function_and_the_adr_table_classify_every_kind_identically() {
    let table = adr_table();
    assert_eq!(
        table.len(),
        AssumptionKind::ALL.len(),
        "the ADR table has one row per kind"
    );
    for kind in AssumptionKind::ALL {
        let class = if kind.supersedes_on_promotion() {
            "COLLIDES"
        } else {
            "ADDS"
        };
        assert!(
            table.contains(&(kind.as_token().to_owned(), class.to_owned())),
            "{} is {class} in code but not in the ADR table",
            kind.as_token()
        );
    }
}

#[test]
fn the_free_standing_kinds_add_and_the_singleton_and_targeted_kinds_collide() {
    for kind in [
        AssumptionKind::OneTimeEvent,
        AssumptionKind::RecurringDebtPayment,
        AssumptionKind::VariableSpendOverride,
        AssumptionKind::BillAmount,
        AssumptionKind::IncomeAmount,
    ] {
        assert!(!kind.supersedes_on_promotion(), "{kind:?} adds");
    }
    for kind in [
        AssumptionKind::BillDate,
        AssumptionKind::IncomeDate,
        AssumptionKind::Exclusion,
        AssumptionKind::InflationRate,
        AssumptionKind::MinimumCashFloor,
    ] {
        assert!(kind.supersedes_on_promotion(), "{kind:?} collides");
    }
}

fn insert_event(worker: &DbWorker, id: u128, kind: AssumptionKind, promoted_from: Option<u128>) {
    worker
        .read_connection()
        .unwrap()
        .execute(
            "INSERT INTO forecast_assumption_events
                (id, kind, target_entity_type, target_entity_id, params_json, source,
                 scenario_id, status, superseded_by, origin_run_id, created_at, updated_at,
                 promoted_from_scenario_id)
             VALUES (?1, ?2, NULL, NULL, '{}', 'user_override', NULL, 'active', NULL, NULL,
                     't', 't', ?3)",
            rusqlite::params![u(id), kind.as_token(), promoted_from.map(u)],
        )
        .unwrap();
}

fn supersede(worker: &DbWorker, old: u128, by: u128) {
    worker
        .read_connection()
        .unwrap()
        .execute(
            "UPDATE forecast_assumption_events
                SET status = 'superseded', superseded_by = ?2 WHERE id = ?1",
            rusqlite::params![u(old), u(by)],
        )
        .unwrap();
}

#[test]
fn the_count_finds_exactly_the_adding_kind_events_an_applied_scenario_retired() {
    let (_dir, worker) = worker();
    for id in [1, 2] {
        worker
            .read_connection()
            .unwrap()
            .execute(
                "INSERT INTO scenarios (id, name, status, created_at, updated_at)
                 VALUES (?1, 'Scenario', 'draft', 't', 't')",
                [u(id)],
            )
            .unwrap();
    }
    // Scenario 1 is applied; scenario 2 was reverted (applied_at is NULL again).
    worker
        .read_connection()
        .unwrap()
        .execute(
            "UPDATE scenarios SET applied_at = 't', applied_op_id = x'01' WHERE id = ?1",
            [u(1)],
        )
        .unwrap();
    let total = |worker: &DbWorker| -> u64 {
        worker
            .count_events_wrongly_superseded_by_apply()
            .unwrap()
            .iter()
            .map(|(_, n)| n)
            .sum()
    };
    assert_eq!(total(&worker), 0);

    use AssumptionKind::{BillAmount, BillDate, OneTimeEvent, RecurringDebtPayment};
    // Counted: a base one-off and a base extra debt payment retired by rows promoted from the
    // applied scenario, and a promoted one-off retired by the next promoted one-off of the same
    // scenario (the second symptom of the bug).
    insert_event(&worker, 10, OneTimeEvent, None);
    insert_event(&worker, 11, RecurringDebtPayment, None);
    insert_event(&worker, 12, OneTimeEvent, Some(1));
    insert_event(&worker, 13, OneTimeEvent, Some(1));
    insert_event(&worker, 14, RecurringDebtPayment, Some(1));
    supersede(&worker, 10, 12);
    supersede(&worker, 11, 14);
    supersede(&worker, 12, 13);

    // Not counted: a collision kind retired by a promotion (the intended supersession), a
    // one-off replaced by the user's own edit, a one-off retired by a scenario that is no
    // longer applied, and one that is still active.
    insert_event(&worker, 20, BillDate, None);
    insert_event(&worker, 21, BillDate, Some(1));
    supersede(&worker, 20, 21);
    insert_event(&worker, 22, OneTimeEvent, None);
    insert_event(&worker, 23, OneTimeEvent, None);
    supersede(&worker, 22, 23);
    insert_event(&worker, 24, OneTimeEvent, None);
    insert_event(&worker, 25, OneTimeEvent, Some(2));
    supersede(&worker, 24, 25);
    insert_event(&worker, 26, OneTimeEvent, None);
    // Counted: a base amount override retired by a promoted amount (an adding kind now).
    insert_event(&worker, 27, BillAmount, None);
    insert_event(&worker, 28, BillAmount, Some(1));
    supersede(&worker, 27, 28);

    assert_eq!(total(&worker), 4);
    // One entry per adding kind, none for a collision kind, and the counts split by kind.
    let by_kind = worker.count_events_wrongly_superseded_by_apply().unwrap();
    let expected: Vec<AssumptionKind> = AssumptionKind::ALL
        .iter()
        .copied()
        .filter(|kind| !kind.supersedes_on_promotion())
        .collect();
    assert_eq!(
        by_kind.iter().map(|(k, _)| *k).collect::<Vec<_>>(),
        expected
    );
    let count_of = |kind| by_kind.iter().find(|(k, _)| *k == kind).unwrap().1;
    assert_eq!(count_of(OneTimeEvent), 2);
    assert_eq!(count_of(RecurringDebtPayment), 1);
    assert_eq!(count_of(BillAmount), 1);
}
