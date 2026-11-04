//! Performance budgets for apply on a generated golden fixture (personal-cfo-32fmp.4, parent AC8):
//! the cost of a derived id (an HMAC per mint, ADR 0013 Addendum A), of an ordinary command with
//! write-set capture on, and of an import batch end to end (stage, plan, commit, bookkeeping, one
//! inbox rebuild: the personal-cfo-0g528.1 path).
//!
//! The budgets are ABSOLUTE and the tests `#[ignore]`d, like the write-set capture budget: they run in
//! the non-required `perf-budget` job (`ci.yml`), which prints the `ABSOLUTE` lines into the run
//! summary. A required, deterministic test proves the budget check can fail and that the fixture is
//! the same every time. The budgets are set at about three times what a developer machine measured
//! and are re-based from the hosted runner's first numbers.

mod common;

use std::time::{Duration, Instant};

use chrono::{NaiveDate, TimeZone, Utc};
use common::{worker, KEY};
use core_ids::{DerivedIdKey, DerivedIds};
use core_ledger::{
    Account, AccountFlags, AccountId, CashflowRole, LedgerAccountId, SourceBatchId, SourceRecordId,
    StagedTransactionId, TransactionId,
};
use core_money::{Currency, Money};
use db_worker::{ActorType, CommandMeta, DbWorker, IssuedAt, StagedDecision, WriteCommand};
use importer_core::{ParsedAccount, ParsedBatch, ParsedRecord, ParsedTransaction};
use uuid::Uuid;

/// Worst case of one command: the derived-id allocator's cap.
const MAX_ORDINALS: u32 = 4_096;

/// Budgets, in the unit named.
/// Measured 57 ms on a developer machine in the unoptimized test profile (14 us a mint: an HMAC-SHA256
/// and a UUID layout); about 3.5x.
const WORST_CASE_MINTS_MS: u128 = 200;
const ORDINARY_COMMAND_P95_US: u128 = 5_000;
const IMPORT_BATCH_ROWS: usize = 2_000;
const IMPORT_BATCH_SECONDS: u64 = 15;

fn u(n: u128) -> Uuid {
    Uuid::from_u128(0x0198_c2c1_5f0a_7c3d_8a4e_9b1d_f300_0000 + n)
}

fn meta(n: u128) -> CommandMeta {
    CommandMeta {
        command_id: u(n),
        correlation_id: u(n),
        causation_id: None,
        actor_type: ActorType::User,
        actor_id: "perf".to_owned(),
        idempotency_key: format!("00000000-0000-4000-8000-{:012x}", 200_000 + n),
        issued_at: IssuedAt::from_unix_micros(
            Utc.with_ymd_and_hms(2026, 11, 4, 12, 0, 0)
                .unwrap()
                .timestamp_micros()
                + i64::try_from(n).unwrap(),
        ),
    }
}

/// Whether `measured` is over `budget`. The one comparison every budget test uses.
fn over_budget(measured: u128, budget: u128) -> bool {
    measured > budget
}

fn percentile(samples: &mut [Duration], p: usize) -> Duration {
    samples.sort();
    samples[(samples.len() * p / 100).min(samples.len() - 1)]
}

/// The golden fixture: six accounts and `transactions` dated transactions, every one through a
/// command, from fixed ids and fixed metadata. Returns the worker, the next free command number and
/// the transaction ids.
fn golden(transactions: usize) -> (tempfile::TempDir, DbWorker, u128, Vec<TransactionId>) {
    let (dir, worker) = worker();
    let mut n = 0_u128;
    for (index, (name, role, currency)) in [
        ("Checking", CashflowRole::LiquidCash, Currency::Usd),
        ("Savings", CashflowRole::LiquidCash, Currency::Usd),
        ("Card", CashflowRole::CreditFacility, Currency::Usd),
        ("Loan", CashflowRole::LoanLiability, Currency::Usd),
        ("House", CashflowRole::RealAsset, Currency::Usd),
        ("Euro", CashflowRole::LiquidCash, Currency::Eur),
    ]
    .into_iter()
    .enumerate()
    {
        n += 1;
        worker
            .dispatch(
                meta(n),
                WriteCommand::CreateAccount {
                    account: Box::new(Account::new(
                        AccountId::from_uuid(u(1_000 + index as u128)),
                        LedgerAccountId::from_uuid(u(2_000 + index as u128)),
                        name,
                        role,
                        currency,
                        AccountFlags::default(),
                    )),
                    opening_balance: None,
                },
            )
            .unwrap();
    }
    let mut ids = Vec::new();
    for i in 0..transactions {
        n += 1;
        let id = TransactionId::from_uuid(u(10_000 + i as u128));
        let account = AccountId::from_uuid(u(1_000 + (i % 2) as u128));
        worker
            .dispatch(
                meta(n),
                WriteCommand::RecordTransaction {
                    transaction_id: id,
                    account_id: account,
                    amount: Money::new(-(100 + (i as i64 * 37) % 9_000), Currency::Usd),
                    occurred_at: Utc
                        .with_ymd_and_hms(2025, 1 + (i % 12) as u32, 1 + (i % 27) as u32, 9, 0, 0)
                        .unwrap(),
                },
            )
            .unwrap();
        ids.push(id);
    }
    (dir, worker, n, ids)
}

#[test]
fn the_budget_check_can_fail_and_the_fixture_is_the_same_every_time() {
    // The comparison rejects a number over the budget and accepts one at it.
    assert!(over_budget(WORST_CASE_MINTS_MS + 1, WORST_CASE_MINTS_MS));
    assert!(!over_budget(WORST_CASE_MINTS_MS, WORST_CASE_MINTS_MS));
    // The fixture is a pure function of its size: two builds hold the same ledger.
    let totals = |count: usize| {
        let (_dir, worker, _, ids) = golden(count);
        let conn = worker.read_connection().unwrap();
        let (n, sum): (i64, i64) = conn
            .query_row(
                "SELECT COUNT(*), SUM(minor_units) FROM ledger_postings",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        (ids.len(), n, sum)
    };
    assert_eq!(totals(40), totals(40));
    assert_eq!(totals(40).0, 40);
}

#[test]
#[ignore = "absolute latency budget; run by the non-required perf job (cargo test ... -- --ignored)"]
fn the_derived_id_mint_stays_inside_the_absolute_budget() {
    let key = DerivedIdKey::for_vault(&u(1));
    let started = Instant::now();
    let mut ids = DerivedIds::new(
        key.clone(),
        IssuedAt::from_unix_micros(1_000_000_000_000_000),
        u(2),
    );
    for _ in 0..MAX_ORDINALS {
        ids.next("perf.tag").unwrap();
    }
    let worst_case = started.elapsed();

    let rounds = 20_000_u32;
    let started = Instant::now();
    let ids = DerivedIds::new(key, IssuedAt::from_unix_micros(1_000_000_000_000_000), u(2));
    for round in 0..rounds {
        ids.content("perf.content", &[&round.to_be_bytes()])
            .unwrap();
    }
    let per_content = started.elapsed().as_nanos() / u128::from(rounds);
    println!(
        "ABSOLUTE derived ids: {} ordinal mints in {} us ({} ns each); a content id takes {} ns; budget for the worst case {} ms",
        MAX_ORDINALS,
        worst_case.as_micros(),
        worst_case.as_nanos() / u128::from(MAX_ORDINALS),
        per_content,
        WORST_CASE_MINTS_MS
    );
    assert!(
        !over_budget(worst_case.as_millis(), WORST_CASE_MINTS_MS),
        "{} ordinal mints took {} ms",
        MAX_ORDINALS,
        worst_case.as_millis()
    );
}

#[test]
#[ignore = "absolute latency budget; run by the non-required perf job (cargo test ... -- --ignored)"]
fn ordinary_commands_stay_inside_the_absolute_budget_on_the_golden_fixture() {
    let (_dir, worker, mut n, ids) = golden(3_000);
    let mut time = |label: &str, make: &dyn Fn(usize) -> WriteCommand| {
        let mut samples = Vec::new();
        for i in 0..200 {
            n += 1;
            let command = make(i);
            let started = Instant::now();
            worker.dispatch(meta(n), command).unwrap();
            samples.push(started.elapsed());
        }
        let median = percentile(&mut samples, 50);
        let p95 = percentile(&mut samples, 95);
        println!(
            "ABSOLUTE {label}: median {} us, p95 {} us (3,000 transactions in the vault); budget p95 {} us",
            median.as_micros(),
            p95.as_micros(),
            ORDINARY_COMMAND_P95_US
        );
        assert!(
            !over_budget(p95.as_micros(), ORDINARY_COMMAND_P95_US),
            "{label}: p95 {} us",
            p95.as_micros()
        );
    };
    time("record a transaction", &|i| {
        WriteCommand::RecordTransaction {
            transaction_id: TransactionId::from_uuid(u(500_000 + i as u128)),
            account_id: AccountId::from_uuid(u(1_000)),
            amount: Money::new(-250, Currency::Usd),
            occurred_at: Utc
                .with_ymd_and_hms(2026, 1, 1 + (i % 27) as u32, 9, 0, 0)
                .unwrap(),
        }
    });
    time("set a note", &|i| WriteCommand::SetNote {
        transaction_id: ids[i],
        note: Some(format!("note {i}")),
    });
    time("mark reviewed", &|i| WriteCommand::MarkReviewed {
        transaction_id: ids[200 + i],
        reviewed: true,
    });
    time("void a transaction", &|i| WriteCommand::VoidTransaction {
        transaction_id: ids[400 + i],
    });
}

fn import_batch(rows: usize) -> ParsedBatch {
    ParsedBatch {
        source_format: "csv".to_owned(),
        accounts: vec![ParsedAccount {
            external_id: None,
            external_name: None,
            external_number_hash: None,
            proposed_subtype: None,
            currency: None,
        }],
        records: (0..rows)
            .map(|i| ParsedRecord {
                external_id: None,
                source_hash: format!("sha256:perf-{i}"),
                normalized_json: "{}".to_owned(),
                parse_confidence_bps: Some(10_000),
                transaction: Some(ParsedTransaction {
                    posted_date: NaiveDate::from_ymd_opt(
                        2026,
                        1 + (i % 12) as u32,
                        1 + (i % 27) as u32,
                    )
                    .unwrap(),
                    transaction_date: None,
                    raw_date: "2026-01-01".to_owned(),
                    date_confidence_bps: 10_000,
                    amount: Money::new(-(100 + i as i64), Currency::Usd),
                    description: Some(format!("Store {i}")),
                    category: None,
                    normalized_merchant: Some(format!("store {i}")),
                    external_account: None,
                    txn_fingerprint: format!("perf-fp-{i}"),
                }),
                balance: None,
            })
            .collect(),
        warnings: Vec::new(),
        skipped: Vec::new(),
    }
}

#[test]
#[ignore = "absolute latency budget; run by the non-required perf job (cargo test ... -- --ignored)"]
fn an_import_batch_stays_inside_the_absolute_budget() {
    let (_dir, worker, mut n, _) = golden(1_000);
    n += 1;
    let batch = SourceBatchId::from_uuid(u(900_000));
    worker
        .dispatch(
            meta(n),
            WriteCommand::CreateSourceBatch {
                id: batch,
                source_type: "csv".to_owned(),
                source_name: Some("perf.csv".to_owned()),
                file_fingerprint: None,
                parser_version: None,
                connector_connection_id: None,
            },
        )
        .unwrap();
    let started = Instant::now();
    let staged = worker
        .stage_parsed_batch(batch.as_uuid(), &import_batch(IMPORT_BATCH_ROWS), u(1_000))
        .unwrap();
    let staged_in = started.elapsed();
    let mut committed = 0;
    for id in &staged {
        n += 1;
        if matches!(
            worker
                .commit_staged(meta(n), StagedTransactionId::from_uuid(*id), false)
                .unwrap(),
            StagedDecision::Committed(_)
        ) {
            committed += 1;
        }
    }
    let decided_in = started.elapsed();
    worker.rebuild_money_inbox().unwrap();
    let total = started.elapsed();
    println!(
        "ABSOLUTE import batch: {IMPORT_BATCH_ROWS} rows staged in {} ms, decided in {} ms ({committed} committed), inbox rebuilt, {} ms in all ({} us per row); budget {IMPORT_BATCH_SECONDS} s",
        staged_in.as_millis(),
        decided_in.as_millis(),
        total.as_millis(),
        total.as_micros() / IMPORT_BATCH_ROWS as u128
    );
    assert!(committed > IMPORT_BATCH_ROWS / 2);
    assert!(
        !over_budget(total.as_secs().into(), IMPORT_BATCH_SECONDS.into()),
        "{} rows took {} s",
        IMPORT_BATCH_ROWS,
        total.as_secs()
    );
    let _ = (KEY, SourceRecordId::from_uuid(u(0)));
}
