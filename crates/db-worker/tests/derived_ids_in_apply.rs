//! Every id `apply` mints is a pure function of the command (personal-cfo-32fmp.2; ADR 0013
//! Addendum A): the same commands applied to two copies of one base mint the same ids and write the
//! same rows, the ordinals of a multi-line command keep creation order, and a command that would
//! mint past the cap fails before it writes anything.

mod common;

use chrono::NaiveDate;
use common::KEY;
use core_ledger::{
    Account, AccountFlags, AccountId, AccountSubtype, CashflowRole, LedgerAccountId, TransactionId,
};
use core_money::{Currency, Money};
use db_worker::{
    ActorType, AssumptionKind, AssumptionSource, CommandMeta, DbError, DbWorker, IssuedAt,
    NewAssumptionEvent, NewScenario, SplitLineInput, WriteCommand,
};
use std::path::{Path, PathBuf};
use uuid::Uuid;

fn u(n: u128) -> Uuid {
    Uuid::from_u128(0x0198_c2c1_5f0a_7c3d_8a4e_9b1d_0000_0000 + n)
}
fn account(n: u128) -> AccountId {
    AccountId::from_uuid(u(n))
}
fn usd(minor: i64) -> Money {
    Money::new(minor, Currency::Usd)
}

const ISSUED_MICROS: i64 = 1_000_000_000_000_000;

fn meta(n: u128) -> CommandMeta {
    CommandMeta {
        command_id: u(1_000 + n),
        correlation_id: u(2_000 + n),
        causation_id: None,
        actor_type: ActorType::User,
        actor_id: "tester".to_owned(),
        idempotency_key: format!("00000000-0000-4000-8000-{:012x}", 3_000 + n),
        issued_at: IssuedAt::from_unix_micros(ISSUED_MICROS),
    }
}

fn open(path: &Path) -> DbWorker {
    DbWorker::open(path, KEY).unwrap()
}

fn create_account(n: u128, role: CashflowRole, opening: Option<Money>) -> WriteCommand {
    WriteCommand::CreateAccount {
        account: Box::new(Account::new(
            account(n),
            LedgerAccountId::from_uuid(u(500 + n)),
            format!("Account {n}"),
            role,
            Currency::Usd,
            AccountFlags::default(),
        )),
        opening_balance: opening,
    }
}

/// A base vault, built once and copied, so both workers start from byte-identical state.
fn base(dir: &Path) -> PathBuf {
    let path = dir.join("base.vault");
    let worker = open(&path);
    let mut n = 0_u128;
    let mut dispatch = |command: WriteCommand| {
        n += 1;
        let mut m = meta(900 + n);
        m.issued_at = IssuedAt::from_unix_micros(ISSUED_MICROS - 1_000_000);
        worker.dispatch(m, command).unwrap();
    };
    dispatch(create_account(
        1,
        CashflowRole::LiquidCash,
        Some(usd(500_000)),
    ));
    dispatch(create_account(2, CashflowRole::LiquidCash, None));
    dispatch(create_account(28, CashflowRole::CreditFacility, None));
    dispatch(WriteCommand::SetAccountSubtype {
        id: account(28),
        subtype: Some(AccountSubtype::CreditCard),
    });
    dispatch(WriteCommand::RecordTransaction {
        transaction_id: TransactionId::from_uuid(u(10)),
        account_id: account(1),
        amount: usd(-4_250),
        occurred_at: chrono::DateTime::from_timestamp_micros(ISSUED_MICROS).unwrap(),
    });
    worker
        .create_scenario(&NewScenario {
            id: u(25),
            name: "What if".to_owned(),
            description: None,
            base_run_id: None,
        })
        .unwrap();
    for (n, kind) in [
        (1_u128, AssumptionKind::InflationRate),
        (2, AssumptionKind::MinimumCashFloor),
    ] {
        worker
            .record_assumption_event(&NewAssumptionEvent {
                id: u(60 + n),
                kind,
                target_entity_type: None,
                target_entity_id: None,
                params_json: r#"{"annual_bps":400}"#.to_owned(),
                source: AssumptionSource::UserOverride,
                scenario_id: Some(u(25)),
                origin_run_id: None,
            })
            .unwrap();
    }
    worker.checkpoint_truncate().unwrap();
    path
}

fn line(minor: i64) -> SplitLineInput {
    SplitLineInput {
        amount: usd(minor),
        category_id: None,
        note: None,
        tag_ids: vec![],
    }
}

/// The commands under test: every one mints ids inside apply.
fn sequence() -> Vec<(&'static str, WriteCommand)> {
    vec![
        (
            "create an account with an opening balance",
            create_account(3, CashflowRole::LiquidCash, Some(usd(12_000))),
        ),
        (
            "transfer",
            WriteCommand::Transfer {
                source_account_id: account(1),
                dest_account_id: account(2),
                amount: usd(25_000),
                occurred_at: chrono::DateTime::from_timestamp_micros(ISSUED_MICROS).unwrap(),
            },
        ),
        (
            "split a transaction into three lines",
            WriteCommand::SetSplits {
                transaction_id: TransactionId::from_uuid(u(10)),
                lines: vec![line(-1_000), line(-1_250), line(-2_000)],
            },
        ),
        (
            "snooze an inbox item",
            WriteCommand::SnoozeInboxItem {
                item_id: u(36),
                until: NaiveDate::from_ymd_opt(2026, 12, 25).unwrap(),
            },
        ),
        (
            "dismiss an inbox item",
            WriteCommand::DismissInboxItem {
                item_id: u(37),
                reason: "not mine".into(),
            },
        ),
        (
            "record a statement balance",
            WriteCommand::SetCardStatementBalance {
                account_id: account(28),
                cycle_close: NaiveDate::from_ymd_opt(2001, 8, 15).unwrap(),
                statement_balance_minor: Some(184_500),
            },
        ),
        (
            "set the household timezone",
            WriteCommand::SetHouseholdTimezone {
                timezone: "America/Chicago".into(),
            },
        ),
        (
            "dismiss a recurring suggestion",
            WriteCommand::DismissRecurringSuggestion {
                merchant_key: "CAFE".into(),
                currency: "USD".into(),
                amount_minor: 450,
                frequency: "weekly".into(),
                reason: None,
            },
        ),
        (
            "apply a scenario",
            WriteCommand::ApplyScenario { scenario_id: u(25) },
        ),
        (
            "void a transaction",
            WriteCommand::VoidTransaction {
                transaction_id: TransactionId::from_uuid(u(10)),
            },
        ),
    ]
}

/// Every id in the tables apply mints into, as sorted text.
fn minted_ids(worker: &DbWorker) -> Vec<String> {
    let conn = worker.read_connection().unwrap();
    let mut out = Vec::new();
    for (table, column) in [
        ("ledger_transactions", "id"),
        ("split_lines", "id"),
        ("change_journal_entries", "id"),
        ("credit_card_statements", "id"),
        ("forecast_assumption_events", "id"),
        ("audit_events", "id"),
    ] {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT hex({column}) FROM {table} ORDER BY {column}"
            ))
            .unwrap();
        let ids: Vec<String> = stmt
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        out.extend(ids.into_iter().map(|id| format!("{table}:{id}")));
    }
    out
}

#[test]
fn the_same_commands_on_two_copies_of_a_base_mint_the_same_ids_and_write_the_same_rows() {
    let dir = tempfile::tempdir().unwrap();
    let base = base(dir.path());
    let (a_path, b_path) = (dir.path().join("a.vault"), dir.path().join("b.vault"));
    std::fs::copy(&base, &a_path).unwrap();
    std::fs::copy(&base, &b_path).unwrap();
    let (a, b) = (open(&a_path), open(&b_path));
    let mut previous = minted_ids(&a);
    assert_eq!(previous, minted_ids(&b));
    for (n, ((label, command_a), (_, command_b))) in
        sequence().into_iter().zip(sequence()).enumerate()
    {
        a.dispatch(meta(n as u128), command_a)
            .unwrap_or_else(|e| panic!("{label}: {e}"));
        b.dispatch(meta(n as u128), command_b)
            .unwrap_or_else(|e| panic!("{label}: {e}"));
        let (wa, wb) = (a.last_write_set().unwrap(), b.last_write_set().unwrap());
        assert_eq!(wa.hash, wb.hash, "{label}: write-set hash");
        assert_eq!(wa.rows, wb.rows, "{label}: the rows written");
        let (ids_a, ids_b) = (minted_ids(&a), minted_ids(&b));
        assert_eq!(ids_a, ids_b, "{label}: minted ids");
        previous = ids_a;
    }
    // The commands did mint into every table apply mints into.
    let count = |table: &str| {
        previous
            .iter()
            .filter(|id| id.starts_with(&format!("{table}:")))
            .count()
    };
    assert_eq!(count("split_lines"), 3);
    assert_eq!(count("change_journal_entries"), 2);
    assert_eq!(count("credit_card_statements"), 1);
    assert_eq!(
        count("forecast_assumption_events"),
        4,
        "two in the scenario, two promoted"
    );
    assert_eq!(count("audit_events"), 1);
    // Two in the base, then the opening balance, the transfer and the reversal.
    assert!(count("ledger_transactions") >= 5);
}

#[test]
fn split_lines_keep_creation_order_under_order_by_id() {
    let dir = tempfile::tempdir().unwrap();
    let worker = open(&base(dir.path()));
    let lines: Vec<SplitLineInput> = [-500, -700, -300, -400, -600, -450, -300, -1_000]
        .into_iter()
        .map(line)
        .collect();
    // The transaction is -4_250: the lines sum to it.
    assert_eq!(
        lines.iter().map(|l| l.amount.minor_units()).sum::<i64>(),
        -4_250
    );
    worker
        .dispatch(
            meta(1),
            WriteCommand::SetSplits {
                transaction_id: TransactionId::from_uuid(u(10)),
                lines,
            },
        )
        .unwrap();
    let conn = worker.read_connection().unwrap();
    let mut stmt = conn
        .prepare("SELECT sort_order FROM split_lines WHERE transaction_id = ?1 ORDER BY id")
        .unwrap();
    let order: Vec<i64> = stmt
        .query_map([u(10)], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        order,
        (0..8).collect::<Vec<_>>(),
        "ORDER BY id is creation order"
    );
}

#[test]
fn a_command_that_would_mint_past_the_cap_fails_before_anything_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let worker = open(&base(dir.path()));
    // 4,097 lines: 4,096 empty slices and the whole amount, so the payload is valid and only the
    // 4,097th id is refused.
    let mut lines = vec![line(0); 4_096];
    lines.push(line(-4_250));
    let logged = |w: &DbWorker| -> i64 {
        w.read_connection()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM operation_log", [], |r| r.get(0))
            .unwrap()
    };
    let before = logged(&worker);
    let error = worker
        .dispatch(
            meta(1),
            WriteCommand::SetSplits {
                transaction_id: TransactionId::from_uuid(u(10)),
                lines,
            },
        )
        .unwrap_err();
    assert!(matches!(error, DbError::InvalidCommand(_)), "{error:?}");
    assert!(error.to_string().contains("4096"), "{error}");
    assert_eq!(logged(&worker), before, "nothing was logged");
    let split_rows: i64 = worker
        .read_connection()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM split_lines", [], |r| r.get(0))
        .unwrap();
    assert_eq!(split_rows, 0, "no line survived the rolled-back command");
}

#[test]
fn the_household_timezone_audit_row_is_written_once_per_command_with_a_derived_id() {
    let dir = tempfile::tempdir().unwrap();
    let base = base(dir.path());
    let ids_on_fresh_copy = |name: &str| -> Vec<String> {
        let path = dir.path().join(name);
        std::fs::copy(&base, &path).unwrap();
        let worker = open(&path);
        worker
            .dispatch(
                meta(1),
                WriteCommand::SetHouseholdTimezone {
                    timezone: "Asia/Tokyo".into(),
                },
            )
            .unwrap();
        let conn = worker.read_connection().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT hex(id) FROM audit_events WHERE event_type = 'household_timezone_changed'",
            )
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    let (first, second) = (
        ids_on_fresh_copy("one.vault"),
        ids_on_fresh_copy("two.vault"),
    );
    assert_eq!(first.len(), 1);
    assert_eq!(first, second, "an identical id on two fresh vaults");
}
