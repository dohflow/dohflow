//! The deterministic rejections are a function of payload, metadata and class-1 state only
//! (personal-cfo-g3m.26). Every site listed in `deterministic_sites.toml` has a scenario here, and
//! the whole scenario list is run in four environments that must not matter:
//!
//! * a plain vault;
//! * the same vault with class-3 (device-local) state added: a device setting, audit events, an
//!   idempotency record, a backup record;
//! * the process under two different device time zones (each in a child process).
//!
//! The outcome of every scenario (the code of a rejection, or the row count of a setup success) must
//! be identical in all of them. That is the behavioral proof of the audit: a validator that read a
//! clock, a time zone or class-3 state would differ somewhere.

mod common;

use common::KEY;
use core_ledger::{
    Account, AccountFlags, AccountId, AccountSubtype, CashflowRole, CategoryId, LedgerAccountId,
    TransactionId,
};
use core_money::{Currency, Money};
use db_worker::{
    ActorType, AssumptionKind, AssumptionSource, CommandMeta, DbError, DbWorker, IssuedAt,
    NewAssumptionEvent, RejectCode, WriteCommand,
};
use std::path::{Path, PathBuf};
use uuid::Uuid;

fn u(n: u128) -> Uuid {
    Uuid::from_u128(0x0198_c2c1_5f0a_7c3d_8a4e_9b1d_0000_0000 + n)
}
fn account(n: u128) -> AccountId {
    AccountId::from_uuid(u(n))
}
fn category(n: u128) -> CategoryId {
    CategoryId::from_uuid(u(n))
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

fn create_account(n: u128, role: CashflowRole, currency: Currency) -> WriteCommand {
    WriteCommand::CreateAccount {
        account: Box::new(Account::new(
            account(n),
            LedgerAccountId::from_uuid(u(500 + n)),
            format!("Account {n}"),
            role,
            currency,
            AccountFlags::default(),
        )),
        opening_balance: None,
    }
}

fn event(n: u128, scenario: Option<u128>, params_json: &str) -> NewAssumptionEvent {
    NewAssumptionEvent {
        id: u(n),
        kind: AssumptionKind::OneTimeEvent,
        target_entity_type: None,
        target_entity_id: None,
        params_json: params_json.to_owned(),
        source: AssumptionSource::UserOverride,
        scenario_id: scenario.map(u),
    }
}

fn record_event(n: u128, scenario: Option<u128>, params_json: &str) -> WriteCommand {
    WriteCommand::RecordAssumptionEvent {
        event: event(n, scenario, params_json),
    }
}

fn supersede_event(old: u128, new: u128) -> WriteCommand {
    WriteCommand::SupersedeAssumptionEvent {
        id: u(old),
        replacement: event(new, None, "{}"),
    }
}

fn clear_event(n: u128) -> WriteCommand {
    WriteCommand::ClearAssumptionEvent { id: u(n) }
}

fn create_category(n: u128, parent: Option<u128>, name: &str) -> WriteCommand {
    WriteCommand::CreateCategory {
        id: category(n),
        parent_id: parent.map(category),
        name: name.to_owned(),
        category_type: "expense".to_owned(),
        color: None,
        icon: None,
    }
}

/// What a scenario expects: a rejection with a code, or success.
#[derive(Clone, Copy)]
enum Expect {
    Reject(RejectCode),
    Ok,
}

struct Step {
    label: &'static str,
    command: WriteCommand,
    expect: Expect,
}

fn step(label: &'static str, command: WriteCommand, expect: Expect) -> Step {
    Step {
        label,
        command,
        expect,
    }
}

use Expect::{Ok as Fine, Reject};
use RejectCode::{PayloadInvalid, ReferenceMissing, StateConflict};

/// The scenarios, in order on one vault: setup steps (expected to succeed) and the rejections.
fn scenarios() -> Vec<Step> {
    vec![
        // ---- setup ----
        step(
            "liquid",
            create_account(1, CashflowRole::LiquidCash, Currency::Usd),
            Fine,
        ),
        step(
            "asset",
            create_account(2, CashflowRole::RealAsset, Currency::Usd),
            Fine,
        ),
        step(
            "loan",
            create_account(3, CashflowRole::LoanLiability, Currency::Usd),
            Fine,
        ),
        step(
            "second asset",
            create_account(4, CashflowRole::RealAsset, Currency::Usd),
            Fine,
        ),
        step(
            "link asset to loan",
            WriteCommand::SetAccountLink {
                asset_id: account(2),
                liability_id: Some(account(3)),
            },
            Fine,
        ),
        step(
            "a recorded transaction",
            WriteCommand::RecordTransaction {
                transaction_id: TransactionId::from_uuid(u(30)),
                account_id: account(1),
                amount: Money::new(-1_000, Currency::Usd),
                occurred_at: chrono::DateTime::from_timestamp_micros(ISSUED_MICROS).unwrap(),
            },
            Fine,
        ),
        step("group", create_category(10, None, "Home"), Fine),
        step("child", create_category(11, Some(10), "Utilities"), Fine),
        // ---- accounts.rs ----
        step(
            "an opening balance in another currency than the account",
            WriteCommand::CreateAccount {
                account: Box::new(Account::new(
                    account(20),
                    LedgerAccountId::from_uuid(u(520)),
                    "Mismatch",
                    CashflowRole::LiquidCash,
                    Currency::Usd,
                    AccountFlags::default(),
                )),
                opening_balance: Some(Money::new(100, Currency::Eur)),
            },
            Reject(PayloadInvalid),
        ),
        step(
            "a subtype of an unknown account",
            WriteCommand::SetAccountSubtype {
                id: account(99),
                subtype: Some(AccountSubtype::Checking),
            },
            Reject(ReferenceMissing),
        ),
        step(
            "a subtype that does not belong to the account's role",
            WriteCommand::SetAccountSubtype {
                id: account(1),
                subtype: Some(AccountSubtype::CreditCard),
            },
            Reject(StateConflict),
        ),
        step(
            "a link from an unknown asset",
            WriteCommand::SetAccountLink {
                asset_id: account(99),
                liability_id: None,
            },
            Reject(ReferenceMissing),
        ),
        step(
            "a link from an account that is not a real asset",
            WriteCommand::SetAccountLink {
                asset_id: account(1),
                liability_id: Some(account(3)),
            },
            Reject(StateConflict),
        ),
        step(
            "a link of an asset to itself",
            WriteCommand::SetAccountLink {
                asset_id: account(4),
                liability_id: Some(account(4)),
            },
            Reject(PayloadInvalid),
        ),
        step(
            "a link to an account that is not a loan",
            WriteCommand::SetAccountLink {
                asset_id: account(4),
                liability_id: Some(account(1)),
            },
            Reject(StateConflict),
        ),
        step(
            "a link to an unknown liability",
            WriteCommand::SetAccountLink {
                asset_id: account(4),
                liability_id: Some(account(98)),
            },
            Reject(ReferenceMissing),
        ),
        step(
            "a loan already linked to another asset",
            WriteCommand::SetAccountLink {
                asset_id: account(4),
                liability_id: Some(account(3)),
            },
            Reject(StateConflict),
        ),
        // ---- categorization_cmds.rs ----
        step(
            "a category with an empty name",
            create_category(12, None, "   "),
            Reject(PayloadInvalid),
        ),
        step(
            "a category under an unknown parent",
            create_category(13, Some(98), "Orphan"),
            Reject(ReferenceMissing),
        ),
        step(
            "an update to an empty name",
            WriteCommand::UpdateCategory {
                id: category(11),
                name: String::new(),
                color: None,
                icon: None,
            },
            Reject(PayloadInvalid),
        ),
        step(
            "an update of an unknown category",
            WriteCommand::UpdateCategory {
                id: category(97),
                name: "Ghost".to_owned(),
                color: None,
                icon: None,
            },
            Reject(ReferenceMissing),
        ),
        step(
            "a move under an unknown parent",
            WriteCommand::MoveCategory {
                id: category(11),
                new_parent_id: Some(category(96)),
            },
            Reject(ReferenceMissing),
        ),
        step(
            "a move that would create a cycle",
            WriteCommand::MoveCategory {
                id: category(10),
                new_parent_id: Some(category(11)),
            },
            Reject(StateConflict),
        ),
        step(
            "archiving an unknown category",
            WriteCommand::ArchiveCategory(category(95)),
            Reject(ReferenceMissing),
        ),
        step(
            "reinstating an unknown category",
            WriteCommand::ReinstateCategory(category(94)),
            Reject(ReferenceMissing),
        ),
        step(
            "recategorizing a real transaction to an unknown category",
            WriteCommand::RecategorizeTransaction {
                transaction_id: TransactionId::from_uuid(u(30)),
                category_id: Some(category(92)),
            },
            Reject(ReferenceMissing),
        ),
        step(
            "recategorizing an unknown transaction",
            WriteCommand::RecategorizeTransaction {
                transaction_id: TransactionId::from_uuid(u(93)),
                category_id: None,
            },
            Reject(ReferenceMissing),
        ),
        step(
            "recording an assumption event",
            record_event(70, None, "{}"),
            Fine,
        ),
        step(
            "an assumption event whose parameters are not a JSON object",
            record_event(71, None, "[1]"),
            Reject(PayloadInvalid),
        ),
        step(
            "an assumption event in an unknown scenario",
            record_event(72, Some(97), "{}"),
            Reject(ReferenceMissing),
        ),
        step(
            "an assumption event id that is already in use",
            record_event(70, None, "{}"),
            Reject(StateConflict),
        ),
        step(
            "an assumption event superseding itself",
            supersede_event(70, 70),
            Reject(PayloadInvalid),
        ),
        step(
            "superseding an unknown assumption event",
            supersede_event(98, 73),
            Reject(ReferenceMissing),
        ),
        step("superseding an active event", supersede_event(70, 74), Fine),
        step(
            "superseding an event that is no longer active",
            supersede_event(70, 75),
            Reject(StateConflict),
        ),
        step("clearing an active event", clear_event(74), Fine),
        step(
            "clearing an event that is no longer active",
            clear_event(74),
            Reject(StateConflict),
        ),
    ]
}

fn open(path: &Path) -> DbWorker {
    DbWorker::open(path, KEY).unwrap()
}

/// One line per scenario: the rejection code, or the write-set hash of a success. Asserts the
/// expectation on the way.
fn run(worker: &DbWorker) -> Vec<String> {
    scenarios()
        .into_iter()
        .enumerate()
        .map(|(n, s)| {
            let outcome = worker.dispatch(meta(n as u128), s.command);
            let line = match (&outcome, s.expect) {
                (Ok(_), Fine) => {
                    let set = worker.last_write_set().unwrap();
                    // Setup steps only: their write sets can include a system account whose id is
                    // random until personal-cfo-32fmp.3, so only the row count is compared.
                    format!("{}: ok rows={}", s.label, set.rows.len())
                }
                (Err(DbError::Rejected { code, .. }), Reject(expected)) => {
                    assert_eq!(*code, expected, "{}", s.label);
                    // A rejected command wrote nothing.
                    format!("{}: rejected {}", s.label, code.as_u16())
                }
                (other, _) => panic!("{}: unexpected outcome {other:?}", s.label),
            };
            line
        })
        .collect()
}

/// Class-3 state a device may hold, none of which a deterministic validator may read.
fn add_class_3_noise(worker: &DbWorker) {
    let conn = worker.read_connection().unwrap();
    conn.execute_batch(
        "INSERT INTO settings (key, value, updated_at) VALUES ('device.noise', 'x', '2030-01-01T00:00:00+00:00');
         INSERT INTO audit_events (id, event_type, command_id, correlation_id, created_at)
             VALUES (x'00000000000000000000000000000f01', 'noise', x'00000000000000000000000000000f02',
                     x'00000000000000000000000000000f03', '2030-01-01T00:00:00+00:00');
         INSERT INTO command_idempotency_keys (idempotency_key, command_id, command_type, result_ref_json, op_seq, expires_at)
             VALUES ('noise-key', x'00000000000000000000000000000f04', 'noise', '{}', 0,
                     '2031-01-01T00:00:00+00:00');",
    )
    .unwrap_or_else(|e| panic!("class-3 noise: {e}"));
}

fn lines_in_child(zone: &str) -> Vec<String> {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "child_run_under_the_inherited_time_zone",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("TZ", zone)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "child under {zone} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|l| {
            l.find("OUTCOME ")
                .map(|at| l[at + "OUTCOME ".len()..].to_owned())
        })
        .collect()
}

#[test]
#[ignore = "run by a_rejection_is_the_same_under_every_environment in a child process"]
fn child_run_under_the_inherited_time_zone() {
    let dir = tempfile::tempdir().unwrap();
    let worker = open(&dir.path().join("v.vault"));
    for line in run(&worker) {
        println!("OUTCOME {line}");
    }
}

#[test]
fn a_rejection_is_the_same_under_every_environment() {
    let dir = tempfile::tempdir().unwrap();
    let plain = run(&open(&dir.path().join("plain.vault")));
    assert_eq!(plain.len(), scenarios().len());

    // Class-3 state a device may hold.
    let noisy_worker = open(&dir.path().join("noisy.vault"));
    add_class_3_noise(&noisy_worker);
    assert_eq!(
        plain,
        run(&noisy_worker),
        "class-3 state changed a verdict or a write set"
    );

    // Two device time zones.
    let chicago = lines_in_child("America/Chicago");
    let auckland = lines_in_child("Pacific/Auckland");
    assert_eq!(
        chicago, auckland,
        "the device time zone changed a verdict or a write set"
    );
    assert_eq!(
        plain, chicago,
        "the parent process and the children agree on every verdict"
    );
}

/// Every `DbError::reject(` call in the production source of a listed module, as (file, line).
fn source_sites() -> std::collections::BTreeSet<(String, u32)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut sites = std::collections::BTreeSet::new();
    for module in [
        "apply/accounts.rs",
        "apply/assumption_cmds.rs",
        "apply/categorization_cmds.rs",
        "apply/mod.rs",
    ] {
        let path = root.join("src").join(module);
        let source = std::fs::read_to_string(&path).unwrap();
        for (index, line) in source.lines().enumerate() {
            if line.contains("DbError::reject(") && !line.trim_start().starts_with("//") {
                sites.insert((
                    format!("crates/db-worker/src/{module}"),
                    u32::try_from(index + 1).unwrap(),
                ));
            }
        }
    }
    sites
}

#[test]
fn every_listed_site_is_reached_by_a_scenario() {
    // Per SITE, not per code: the location each rejection reports (the call of `reject`) is
    // collected over every scenario and compared with the `reject(` calls in the source.
    let dir = tempfile::tempdir().unwrap();
    let worker = open(&dir.path().join("v.vault"));
    let mut reached = std::collections::BTreeSet::new();
    for (n, s) in scenarios().into_iter().enumerate() {
        if let Err(DbError::Rejected { site, .. }) = worker.dispatch(meta(n as u128), s.command) {
            reached.insert((site.file().replace('\\', "/"), site.line()));
        }
    }
    // The issued_at refusal is a site too.
    let mut late = meta(900);
    late.issued_at = IssuedAt::from_unix_micros(i64::MAX);
    if let Err(DbError::Rejected { site, .. }) = worker.dispatch(
        late,
        WriteCommand::SetHouseholdTimezone {
            timezone: "UTC".to_owned(),
        },
    ) {
        reached.insert((site.file().replace('\\', "/"), site.line()));
    }
    let reached: std::collections::BTreeSet<(String, u32)> = reached
        .into_iter()
        .map(|(file, line)| {
            let at = file.find("crates/db-worker/").unwrap_or(0);
            (file[at..].to_owned(), line)
        })
        .collect();
    assert_eq!(
        reached,
        source_sites(),
        "a listed rejection site has no scenario that reaches it (or a scenario hit an unlisted one)"
    );
}

#[test]
fn the_issued_at_bound_is_the_years_1970_to_9999_and_a_refusal_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let worker = open(&dir.path().join("v.vault"));
    let command = |tz: &str| WriteCommand::SetHouseholdTimezone {
        timezone: tz.to_owned(),
    };
    let at = |micros: i64, n: u128| {
        let mut m = meta(n);
        m.issued_at = IssuedAt::from_unix_micros(micros);
        m
    };
    // The earliest and latest accepted instants.
    worker
        .dispatch(at(0, 1), command("America/Chicago"))
        .unwrap();
    worker
        .dispatch(at(253_402_300_799_999_999, 2), command("Asia/Tokyo"))
        .unwrap();
    // One microsecond outside each end, and the extremes.
    let logged = || -> i64 {
        worker
            .read_connection()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM operation_log", [], |r| r.get(0))
            .unwrap()
    };
    let before = logged();
    for (n, micros) in [
        -1_i64,
        -62_167_219_200_000_000,
        253_402_300_800_000_000,
        i64::MAX,
        i64::MIN,
    ]
    .into_iter()
    .enumerate()
    {
        let error = worker
            .dispatch(at(micros, 10 + n as u128), command("UTC"))
            .unwrap_err();
        assert!(
            matches!(
                error,
                DbError::Rejected {
                    code: RejectCode::IssuedAtOutOfRange,
                    ..
                }
            ),
            "{micros}: {error:?}"
        );
        assert_eq!(error.apply_failure_code().map(|c| c.get()), Some(1));
    }
    assert_eq!(logged(), before, "a refused command writes nothing");
}

/// The receiver's bound, the apply check and the id mint are one bound: every instant the receiver
/// accepts mints ids (a command that mints, at both ends), and the mint's own range error is
/// unreachable from apply (personal-cfo-g3m.26 part 2).
#[test]
fn every_issued_at_the_receiver_accepts_mints_ids_and_one_outside_is_refused_before_the_mint() {
    let dir = tempfile::tempdir().unwrap();
    let worker = open(&dir.path().join("v.vault"));
    let minting = |n: u128| WriteCommand::CreateAccount {
        account: Box::new(Account::new(
            account(n),
            LedgerAccountId::from_uuid(u(500 + n)),
            format!("Account {n}"),
            CashflowRole::LiquidCash,
            Currency::Usd,
            AccountFlags::default(),
        )),
        opening_balance: Some(Money::new(500_000, Currency::Usd)),
    };
    let at = |micros: i64, n: u128| {
        let mut m = meta(n);
        m.issued_at = IssuedAt::from_unix_micros(micros);
        m
    };
    let key = core_ids::DerivedIdKey::for_vault(&Uuid::from_u128(1));
    for (n, micros) in [
        (1_u128, sync_core::envelope::ISSUED_AT_MIN_MICROS),
        (2, 1_000),
        (3, sync_core::envelope::ISSUED_AT_MAX_MICROS),
    ] {
        assert!(sync_core::envelope::issued_at_in_range(micros));
        // The id mint accepts it, and so does a command that mints.
        core_ids::DerivedIds::new(key, IssuedAt::from_unix_micros(micros), Uuid::from_u128(n))
            .next("x")
            .unwrap();
        worker.dispatch(at(micros, n), minting(n)).unwrap();
    }
    for (n, micros) in [
        (10_u128, sync_core::envelope::ISSUED_AT_MIN_MICROS - 1),
        (11, sync_core::envelope::ISSUED_AT_MAX_MICROS + 1),
    ] {
        assert!(!sync_core::envelope::issued_at_in_range(micros));
        // Refused as a deterministic rejection, never as the mint's unclassified range error.
        let error = worker.dispatch(at(micros, n), minting(n)).unwrap_err();
        assert!(
            matches!(
                error,
                DbError::Rejected {
                    code: RejectCode::IssuedAtOutOfRange,
                    ..
                }
            ),
            "{micros}: {error:?}"
        );
    }
}

// Unused imports kept honest for the next scenario that needs them.
#[allow(dead_code)]
fn _paths(_: PathBuf) {}
