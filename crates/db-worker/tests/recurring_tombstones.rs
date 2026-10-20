//! The three hard deletes are tombstones (personal-cfo-egn67, SYNC-2b; ADR 0073 Addendum A).
//!
//! `DeleteRecurringTransfer`, `DeleteIncomeSource` and `DeleteRecurringBill` used to run a
//! literal `DELETE`; they now set `deleted_at` to the command's `issued_at`, every reader
//! skips a tombstoned row, and a `Reinstate*` command clears it. The contract these tests pin:
//!
//! * the row stays, with `deleted_at` equal to the command's `issued_at` (never the clock);
//! * everything a reader returns for a vault with a tombstoned row equals what it returns for a
//!   vault where the row never existed (the behavior a hard delete had);
//! * reinstating brings the row back to every reader, and tombstoning is independent of
//!   archiving;
//! * deleting a tombstoned row and reinstating a live one are accepted, logged no-ops, and
//!   anything that targets a missing row still errors;
//! * a tombstoned row cannot be edited;
//! * no code path deletes a row from the four tables.

mod common;

use chrono::{NaiveDate, Utc};
use common::*;
use core_ledger::{AccountId, IncomeSourceId, RecurringEventId, RecurringTransferId};
use core_money::{Currency, Money};
use db_worker::*;
use pay_schedule::Frequency;
use rusqlite::{params, Connection};
use uuid::Uuid;

/// The instant the tests issue their delete commands at: a fixed point that is not "now".
const ISSUED_MICROS: i64 = 1_792_411_200_000_000; // 2026-10-19T12:00:00Z

fn rfc3339(micros: i64) -> String {
    chrono::DateTime::from_timestamp_micros(micros)
        .unwrap()
        .to_rfc3339()
}

fn meta_at(micros: i64) -> CommandMeta {
    CommandMeta {
        issued_at: IssuedAt::from_unix_micros(micros),
        ..meta()
    }
}

fn liquid(worker: &DbWorker, opening: i64) -> AccountId {
    let id = AccountId::new();
    worker
        .dispatch(meta(), liquid_account_cmd(id, Currency::Usd, opening))
        .unwrap();
    id
}

fn deleted_at(conn: &Connection, table: &str, key: &str, id: Uuid) -> Option<String> {
    conn.query_row(
        &format!("SELECT deleted_at FROM {table} WHERE {key} = ?1"),
        [id],
        |r| r.get(0),
    )
    .unwrap()
}

fn row_count(conn: &Connection, table: &str, key: &str, id: Uuid) -> i64 {
    conn.query_row(
        &format!("SELECT COUNT(*) FROM {table} WHERE {key} = ?1"),
        [id],
        |r| r.get(0),
    )
    .unwrap()
}

/// A household with one of each kind, returning the ids of the three.
struct Household {
    transfer: RecurringTransferId,
    income: IncomeSourceId,
    bill: RecurringEventId,
}

fn income_named(name: &str) -> (IncomeSourceId, WriteCommand) {
    let id = IncomeSourceId::new();
    (
        id,
        WriteCommand::CreateIncomeSource {
            id,
            name: name.to_owned(),
            net_amount: Money::new(300_000, Currency::Usd),
            frequency: Frequency::Biweekly,
            anchor: Utc::now().date_naive() + chrono::Duration::days(2),
            deposit_account_id: None,
        },
    )
}

fn bill_named(name: &str) -> (RecurringEventId, WriteCommand) {
    let WriteCommand::CreateRecurringBill {
        event_id,
        contract_id,
        amount,
        bill_type,
        frequency,
        autopay_account_id,
        description,
        source_merchant_key,
        category_id,
        tag_ids,
        ..
    } = bill_cmd(None)
    else {
        unreachable!()
    };
    (
        event_id,
        WriteCommand::CreateRecurringBill {
            event_id,
            contract_id,
            name: name.to_owned(),
            amount,
            bill_type,
            frequency,
            anchor: Utc::now().date_naive() + chrono::Duration::days(4),
            autopay_account_id,
            description,
            source_merchant_key,
            category_id,
            tag_ids,
        },
    )
}

fn transfer_between(
    from: AccountId,
    to: AccountId,
    anchor: NaiveDate,
) -> (RecurringTransferId, WriteCommand) {
    let id = RecurringTransferId::new();
    (
        id,
        WriteCommand::CreateRecurringTransfer {
            id,
            source_account_id: from,
            dest_account_id: to,
            amount: Money::new(30_000, Currency::Usd),
            frequency: Frequency::Monthly,
            anchor,
        },
    )
}

/// Create the doomed trio, named so a comparison vault can leave them out.
fn doomed(worker: &DbWorker, from: AccountId, to: AccountId) -> Household {
    let (income, c) = income_named("Doomed Pay");
    worker.dispatch(meta(), c).unwrap();
    let (bill, c) = bill_named("Doomed Bill");
    worker.dispatch(meta(), c).unwrap();
    let (transfer, c) = transfer_between(
        from,
        to,
        Utc::now().date_naive() + chrono::Duration::days(3),
    );
    worker.dispatch(meta(), c).unwrap();
    Household {
        transfer,
        income,
        bill,
    }
}

fn delete_all(worker: &DbWorker, h: &Household) {
    worker
        .dispatch(
            meta_at(ISSUED_MICROS),
            WriteCommand::DeleteRecurringTransfer(h.transfer),
        )
        .unwrap();
    worker
        .dispatch(
            meta_at(ISSUED_MICROS),
            WriteCommand::DeleteIncomeSource(h.income),
        )
        .unwrap();
    worker
        .dispatch(
            meta_at(ISSUED_MICROS),
            WriteCommand::DeleteRecurringBill { event_id: h.bill },
        )
        .unwrap();
}

fn reinstate_all(worker: &DbWorker, h: &Household) {
    worker
        .dispatch(meta(), WriteCommand::ReinstateRecurringTransfer(h.transfer))
        .unwrap();
    worker
        .dispatch(meta(), WriteCommand::ReinstateIncomeSource(h.income))
        .unwrap();
    worker
        .dispatch(
            meta(),
            WriteCommand::ReinstateRecurringBill { event_id: h.bill },
        )
        .unwrap();
}

/// Everything the readers of the four tables return, in a form that does not depend on the
/// random ids two vaults mint.
#[derive(Debug, PartialEq, Eq)]
struct Observed {
    incomes: Vec<(String, i64)>,
    bills: Vec<(String, i64)>,
    transfers: Vec<(String, String, i64)>,
    commitments: Vec<(String, Option<i64>)>,
    forecast: Vec<(NaiveDate, String, String, i64)>,
    closing: Vec<i64>,
    /// The per-account projection: (series name, tier, per day (date, closing, events)).
    series: Vec<(String, String, Vec<SeriesDay>)>,
}

/// One day of a per-account series: (date, closing balance, events as (name, kind, amount)).
type SeriesDay = (NaiveDate, i64, Vec<(String, String, i64)>);

fn observe(worker: &DbWorker) -> Observed {
    let forecast = worker.future_cash_forecast(120, &[]).unwrap();
    let by_account = worker.future_cash_by_account(120, &[]).unwrap();
    Observed {
        incomes: worker
            .income_source_views()
            .unwrap()
            .into_iter()
            .map(|v| (v.name, v.net_amount.minor_units()))
            .collect(),
        bills: worker
            .recurring_bill_views()
            .unwrap()
            .into_iter()
            .map(|v| (v.name, v.amount.minor_units()))
            .collect(),
        transfers: worker
            .recurring_transfer_views()
            .unwrap()
            .into_iter()
            .map(|v| {
                (
                    v.source_account_name,
                    v.dest_account_name,
                    v.amount.minor_units(),
                )
            })
            .collect(),
        commitments: worker
            .commitment_views()
            .unwrap()
            .into_iter()
            .map(|v| (v.name, v.amount_expected_minor))
            .collect(),
        forecast: forecast
            .days
            .iter()
            .flat_map(|d| {
                d.events.iter().map(|e| {
                    (
                        d.date,
                        e.name.clone(),
                        e.kind.clone(),
                        e.amount.minor_units(),
                    )
                })
            })
            .collect(),
        closing: forecast
            .days
            .iter()
            .map(|d| d.closing.p50.minor_units())
            .collect(),
        series: by_account
            .accounts
            .iter()
            .map(|a| {
                (
                    a.name.clone(),
                    a.tier.clone(),
                    a.days
                        .iter()
                        .map(|d| {
                            (
                                d.date,
                                d.closing.p50.minor_units(),
                                d.events
                                    .iter()
                                    .map(|e| {
                                        (e.name.clone(), e.kind.clone(), e.amount.minor_units())
                                    })
                                    .collect(),
                            )
                        })
                        .collect(),
                )
            })
            .collect(),
    }
}

#[test]
fn deleting_each_kind_keeps_the_row_and_stamps_it_with_the_commands_issued_at() {
    let (_dir, worker) = worker();
    let (a, b) = (liquid(&worker, 100_000), liquid(&worker, 20_000));
    let h = doomed(&worker, a, b);
    let conn = worker.read_connection().unwrap();
    let expected = rfc3339(ISSUED_MICROS);

    for (table, key, id) in [
        ("recurring_transfers", "id", h.transfer.as_uuid()),
        ("income_sources", "id", h.income.as_uuid()),
        ("recurring_events", "id", h.bill.as_uuid()),
        ("bill_contracts", "recurring_event_id", h.bill.as_uuid()),
    ] {
        assert_eq!(
            deleted_at(&conn, table, key, id),
            None,
            "{table} starts live"
        );
    }

    delete_all(&worker, &h);

    for (table, key, id) in [
        ("recurring_transfers", "id", h.transfer.as_uuid()),
        ("income_sources", "id", h.income.as_uuid()),
        ("recurring_events", "id", h.bill.as_uuid()),
        ("bill_contracts", "recurring_event_id", h.bill.as_uuid()),
    ] {
        assert_eq!(
            row_count(&conn, table, key, id),
            1,
            "{table}: the row is kept"
        );
        assert_eq!(
            deleted_at(&conn, table, key, id).as_deref(),
            Some(expected.as_str()),
            "{table}: deleted_at is the command's issued_at, not the clock"
        );
    }
}

#[test]
fn a_vault_with_tombstoned_rows_reads_exactly_like_one_where_they_never_existed() {
    // The behavior a hard delete had, preserved: lists, commitments and the forecast.
    let (_dir_a, with_tombstones) = worker();
    let (_dir_b, never_existed) = worker();
    for w in [&with_tombstones, &never_existed] {
        // A contribution into an investment account reduces the aggregate forecast, and every
        // transfer is attributed per account, so both transfer readers are exercised.
        let (a, b) = (liquid(w, 100_000), AccountId::new());
        create_role_account(
            w,
            b,
            "Brokerage",
            core_ledger::CashflowRole::InvestmentAsset,
        );
        // A live row of each kind that both vaults keep.
        w.dispatch(meta(), income_named("Kept Pay").1).unwrap();
        w.dispatch(meta(), bill_named("Kept Bill").1).unwrap();
        w.dispatch(
            meta(),
            transfer_between(a, b, Utc::now().date_naive() + chrono::Duration::days(5)).1,
        )
        .unwrap();
        if std::ptr::eq(w, &with_tombstones) {
            let h = doomed(w, a, b);
            delete_all(w, &h);
        }
    }

    let tombstoned = observe(&with_tombstones);
    let baseline = observe(&never_existed);
    assert!(
        !baseline.forecast.is_empty() && !baseline.commitments.is_empty(),
        "the comparison must have something to compare: {baseline:?}"
    );
    assert_eq!(tombstoned, baseline);
    assert!(!tombstoned.incomes.iter().any(|(n, _)| n == "Doomed Pay"));
    assert!(!tombstoned.bills.iter().any(|(n, _)| n == "Doomed Bill"));
}

#[test]
fn reinstating_returns_each_row_to_every_reader() {
    let (_dir, worker) = worker();
    let (a, b) = (liquid(&worker, 100_000), liquid(&worker, 20_000));
    let before_create = observe(&worker);
    let h = doomed(&worker, a, b);
    let live = observe(&worker);
    assert_ne!(live, before_create, "the trio is visible while live");

    delete_all(&worker, &h);
    assert_eq!(
        observe(&worker),
        before_create,
        "deleted equals never created"
    );

    reinstate_all(&worker, &h);
    assert_eq!(observe(&worker), live, "reinstated equals live");
    let conn = worker.read_connection().unwrap();
    for (table, key, id) in [
        ("recurring_transfers", "id", h.transfer.as_uuid()),
        ("income_sources", "id", h.income.as_uuid()),
        ("recurring_events", "id", h.bill.as_uuid()),
        ("bill_contracts", "recurring_event_id", h.bill.as_uuid()),
    ] {
        assert_eq!(deleted_at(&conn, table, key, id), None, "{table}");
    }
}

#[test]
fn tombstoning_and_archiving_are_independent_states() {
    let (_dir, worker) = worker();
    let (income, c) = income_named("Side Gig");
    worker.dispatch(meta(), c).unwrap();
    let (bill, c) = bill_named("Gym");
    worker.dispatch(meta(), c).unwrap();

    // Archive both, then delete both: reinstating clears only the tombstone, so they come
    // back archived (hidden), not active.
    worker
        .dispatch(meta(), WriteCommand::ArchiveIncomeSource(income))
        .unwrap();
    worker
        .dispatch(meta(), WriteCommand::ArchiveRecurringBill(bill))
        .unwrap();
    worker
        .dispatch(meta(), WriteCommand::DeleteIncomeSource(income))
        .unwrap();
    worker
        .dispatch(meta(), WriteCommand::DeleteRecurringBill { event_id: bill })
        .unwrap();
    assert!(worker.income_source_views().unwrap().is_empty());
    assert!(worker.recurring_bill_views().unwrap().is_empty());

    worker
        .dispatch(meta(), WriteCommand::ReinstateIncomeSource(income))
        .unwrap();
    worker
        .dispatch(
            meta(),
            WriteCommand::ReinstateRecurringBill { event_id: bill },
        )
        .unwrap();
    let income_view = worker.income_source_views().unwrap().remove(0);
    let bill_view = worker.recurring_bill_views().unwrap().remove(0);
    assert!(!income_view.active && income_view.archived_at.is_some());
    assert!(!bill_view.active && bill_view.archived_at.is_some());
    assert!(
        worker.commitment_views().unwrap().is_empty(),
        "still archived"
    );

    // And restoring the archive still works after a tombstone round trip.
    worker
        .dispatch(meta(), WriteCommand::RestoreIncomeSource(income))
        .unwrap();
    worker
        .dispatch(meta(), WriteCommand::RestoreRecurringBill(bill))
        .unwrap();
    assert!(worker.income_source_views().unwrap()[0].active);
    assert_eq!(worker.commitment_views().unwrap().len(), 1);
}

#[test]
fn repeating_a_delete_or_a_reinstate_is_an_accepted_logged_no_op() {
    let (_dir, worker) = worker();
    let (a, b) = (liquid(&worker, 100_000), liquid(&worker, 20_000));
    let h = doomed(&worker, a, b);
    let conn = worker.read_connection().unwrap();

    // Reinstating a live row changes nothing but is accepted and logged.
    let ops = worker.operation_count().unwrap();
    reinstate_all(&worker, &h);
    assert_eq!(worker.operation_count().unwrap(), ops + 3);
    assert_eq!(
        deleted_at(&conn, "income_sources", "id", h.income.as_uuid()),
        None
    );

    // Deleting twice keeps the FIRST tombstone's instant: the second is a no-op.
    delete_all(&worker, &h);
    let first = deleted_at(&conn, "recurring_events", "id", h.bill.as_uuid());
    assert_eq!(first.as_deref(), Some(rfc3339(ISSUED_MICROS).as_str()));
    let ops = worker.operation_count().unwrap();
    worker
        .dispatch(
            meta_at(ISSUED_MICROS + 5_000_000),
            WriteCommand::DeleteRecurringBill { event_id: h.bill },
        )
        .unwrap();
    worker
        .dispatch(
            meta_at(ISSUED_MICROS + 5_000_000),
            WriteCommand::DeleteIncomeSource(h.income),
        )
        .unwrap();
    worker
        .dispatch(
            meta_at(ISSUED_MICROS + 5_000_000),
            WriteCommand::DeleteRecurringTransfer(h.transfer),
        )
        .unwrap();
    assert_eq!(worker.operation_count().unwrap(), ops + 3, "logged");
    assert_eq!(
        deleted_at(&conn, "recurring_events", "id", h.bill.as_uuid()),
        first,
        "a repeated delete does not move the tombstone"
    );
}

#[test]
fn a_command_that_targets_a_row_that_never_existed_still_errors() {
    let (_dir, worker) = worker();
    let transfer = RecurringTransferId::new();
    let income = IncomeSourceId::new();
    let bill = RecurringEventId::new();
    for command in [
        WriteCommand::DeleteRecurringTransfer(transfer),
        WriteCommand::ReinstateRecurringTransfer(transfer),
        WriteCommand::DeleteIncomeSource(income),
        WriteCommand::ReinstateIncomeSource(income),
        WriteCommand::DeleteRecurringBill { event_id: bill },
        WriteCommand::ReinstateRecurringBill { event_id: bill },
    ] {
        let label = format!("{command:?}");
        match worker.dispatch(meta(), command) {
            Err(DbError::InvalidCommand(message)) => {
                assert!(message.contains("does not exist"), "{label}: {message}");
            }
            other => panic!("{label}: expected an InvalidCommand, got {other:?}"),
        }
    }
}

#[test]
fn a_tombstoned_row_cannot_be_edited_archived_or_confirmed() {
    let (_dir, worker) = worker();
    let (a, b) = (liquid(&worker, 100_000), liquid(&worker, 20_000));
    let h = doomed(&worker, a, b);
    delete_all(&worker, &h);

    let does_not_exist = |result: Result<Outcome, DbError>, what: &str| match result {
        Err(DbError::InvalidCommand(message)) => {
            assert!(
                message.contains("does not exist") || message.contains("unknown recurring bill"),
                "{what}: {message}"
            );
        }
        other => panic!("{what}: expected a refusal, got {other:?}"),
    };
    does_not_exist(
        worker.dispatch(
            meta(),
            WriteCommand::UpdateIncomeSource {
                id: h.income,
                name: "Renamed".to_owned(),
                net_amount: Money::new(1, Currency::Usd),
                frequency: Frequency::Monthly,
                anchor: NaiveDate::from_ymd_opt(2026, 6, 1).unwrap(),
                deposit_account_id: None,
            },
        ),
        "update income",
    );
    does_not_exist(
        worker.dispatch(meta(), WriteCommand::ArchiveIncomeSource(h.income)),
        "archive income",
    );
    does_not_exist(
        worker.dispatch(meta(), WriteCommand::RestoreIncomeSource(h.income)),
        "restore income",
    );
    does_not_exist(
        worker.dispatch(meta(), WriteCommand::ArchiveRecurringBill(h.bill)),
        "archive bill",
    );
    does_not_exist(
        worker.dispatch(meta(), WriteCommand::RestoreRecurringBill(h.bill)),
        "restore bill",
    );
    does_not_exist(
        worker.dispatch(
            meta(),
            WriteCommand::SetBillAutopay {
                event_id: h.bill,
                autopay: true,
            },
        ),
        "set autopay",
    );
    // Nothing changed underneath.
    let conn = worker.read_connection().unwrap();
    let name: String = conn
        .query_row(
            "SELECT name FROM income_sources WHERE id = ?1",
            params![h.income.as_uuid()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(name, "Doomed Pay");
}

#[test]
fn a_bills_contract_row_is_tombstoned_with_its_schedule_and_comes_back_with_it() {
    let (_dir, worker) = worker();
    let (bill, c) = bill_named("Insurance");
    worker.dispatch(meta(), c).unwrap();
    let conn = worker.read_connection().unwrap();
    let both = |conn: &Connection| {
        (
            deleted_at(conn, "recurring_events", "id", bill.as_uuid()),
            deleted_at(conn, "bill_contracts", "recurring_event_id", bill.as_uuid()),
        )
    };
    assert_eq!(both(&conn), (None, None));
    worker
        .dispatch(
            meta_at(ISSUED_MICROS),
            WriteCommand::DeleteRecurringBill { event_id: bill },
        )
        .unwrap();
    let (event, contract) = both(&conn);
    assert!(
        event.is_some() && event == contract,
        "{event:?} {contract:?}"
    );
    assert!(worker.commitment_views().unwrap().is_empty());
    worker
        .dispatch(
            meta(),
            WriteCommand::ReinstateRecurringBill { event_id: bill },
        )
        .unwrap();
    assert_eq!(both(&conn), (None, None));
    assert_eq!(worker.commitment_views().unwrap().len(), 1);
}

#[test]
fn a_tombstoned_bill_is_not_a_confirmable_obligation() {
    let (_dir, worker) = worker();
    let paying = liquid(&worker, 500_000);
    let (bill, c) = bill_named("Phone");
    worker.dispatch(meta(), c).unwrap();
    worker
        .dispatch(meta(), WriteCommand::DeleteRecurringBill { event_id: bill })
        .unwrap();
    let result = worker.dispatch(
        meta(),
        WriteCommand::ConfirmObligationEarly {
            recurring_event_id: bill,
            scheduled_date: Utc::now().date_naive() + chrono::Duration::days(4),
            paying_account_id: paying,
            actual_amount: Money::new(5_000, Currency::Usd),
            actual_date: Utc::now(),
        },
    );
    match result {
        Err(DbError::InvalidCommand(message)) => {
            assert!(message.contains("unknown recurring bill"), "{message}");
        }
        other => panic!("expected the same refusal a deleted bill got, got {other:?}"),
    }
}

#[test]
fn no_code_path_deletes_a_row_from_the_four_tables() {
    // invariant 9, "nothing is deleted": the tombstones replaced the only deletes, and no purge
    // or compaction job may reintroduce one. A scan of the non-test source.
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n == "tests") {
                    continue;
                }
                walk(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    walk(&src, &mut files);
    assert!(files.len() > 20, "found {} source files", files.len());
    let mut hits = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap();
        // Collapse whitespace so a statement split over lines is still one statement.
        let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
        for table in [
            "recurring_transfers",
            "income_sources",
            "recurring_events",
            "bill_contracts",
        ] {
            let needle = format!("DELETE FROM {table}");
            if flat.contains(&needle) {
                hits.push(format!("{}: {needle}", file.display()));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "a row of these tables is deleted: {hits:?}"
    );
}

#[test]
fn the_forecast_input_snapshot_leaves_tombstoned_rows_out() {
    // A reproducible forecast run snapshots its inputs; a deleted bill or income source must not
    // be in them (it is not an input), and reinstating it brings it back into a new snapshot.
    let (_dir, worker) = worker();
    let (a, b) = (liquid(&worker, 100_000), liquid(&worker, 20_000));
    let (_, c) = income_named("Kept Pay");
    worker.dispatch(meta(), c).unwrap();
    let h = doomed(&worker, a, b);

    // (recurring events JSON, income sources JSON) of a fresh snapshot of the current state.
    let snapshot = || {
        let view = worker
            .input_snapshot(worker.capture_input_snapshot().unwrap())
            .unwrap()
            .unwrap();
        (
            view.recurring_events_snapshot_json.unwrap_or_default(),
            view.income_sources_snapshot_json.unwrap_or_default(),
        )
    };

    let (bills, incomes) = snapshot();
    assert!(bills.contains("Doomed Bill") && incomes.contains("Doomed Pay"));

    delete_all(&worker, &h);
    let (bills, incomes) = snapshot();
    assert!(!bills.contains("Doomed Bill"), "{bills}");
    assert!(!incomes.contains("Doomed Pay"), "{incomes}");
    assert!(incomes.contains("Kept Pay"));

    reinstate_all(&worker, &h);
    let (bills, incomes) = snapshot();
    assert!(bills.contains("Doomed Bill") && incomes.contains("Doomed Pay"));
}

#[test]
fn every_reader_of_the_four_tables_filters_tombstones() {
    // The reader inventory of personal-cfo-egn67 as a ratchet: any SQL string in non-test source
    // that reads one of the four tables (`FROM` or `JOIN`) must filter `deleted_at` at least once
    // per reference, so a new reader cannot quietly resurrect a deleted row. A reader that really
    // wants tombstones (an export, an admin view) must be added to ALLOWED with a reason.
    const ALLOWED: &[(&str, &str)] = &[];
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n == "tests") {
                    continue;
                }
                walk(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    walk(&src, &mut files);
    let tables = [
        "recurring_transfers",
        "income_sources",
        "recurring_events",
        "bill_contracts",
    ];
    let mut checked = 0;
    let mut problems = Vec::new();
    for file in files {
        let name = file.strip_prefix(&src).unwrap().display().to_string();
        let text = std::fs::read_to_string(&file).unwrap();
        // The SQL string literals of the file: text between double quotes that mention SQL.
        let mut literals = Vec::new();
        let mut rest = text.as_str();
        while let Some(open) = rest.find('"') {
            let after = &rest[open + 1..];
            let Some(close) = after.find('"') else { break };
            let literal = &after[..close];
            if literal.contains("SELECT") || literal.contains("FROM") {
                literals.push(literal.to_owned());
            }
            rest = &after[close + 1..];
        }
        for literal in literals {
            let flat: String = literal.split_whitespace().collect::<Vec<_>>().join(" ");
            let mut references = 0;
            // A bill's contract is tombstoned together with its schedule, so a read that joins
            // `bill_contracts` to `recurring_events` inherits the schedule's filter; a read of the
            // contract alone must filter it.
            let joins_the_schedule =
                flat.contains("FROM recurring_events") || flat.contains("JOIN recurring_events");
            for table in tables {
                if table == "bill_contracts" && joins_the_schedule {
                    continue;
                }
                for keyword in ["FROM", "JOIN"] {
                    let pattern = format!("{keyword} {table}");
                    let mut at = 0;
                    while let Some(found) = flat[at..].find(&pattern) {
                        let end = at + found + pattern.len();
                        let boundary = flat[end..]
                            .chars()
                            .next()
                            .is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
                        if boundary {
                            references += 1;
                        }
                        at = end;
                    }
                }
            }
            if references == 0 {
                continue;
            }
            checked += references;
            let filters = flat.matches("deleted_at").count();
            if filters < references && !ALLOWED.iter().any(|(f, _)| *f == name) {
                problems.push(format!(
                    "{name}: {references} reads, {filters} filters: {flat}"
                ));
            }
        }
    }
    assert!(
        checked >= 20,
        "the scan found only {checked} reads; it has stopped looking"
    );
    assert!(
        problems.is_empty(),
        "unfiltered tombstone readers:\n{}",
        problems.join("\n")
    );
}
