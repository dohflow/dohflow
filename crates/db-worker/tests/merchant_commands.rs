//! `MintMerchantAnchors` and `ApplyMerchantMemory` (personal-cfo-5ymg8.5): what each writes, what it
//! skips and why, and that a skip writes nothing. Fake data only. The rejections are in
//! `apply_rejections.rs`; the end-to-end passes are pinned by `merchant_memory_parity.rs`.

mod common;

use chrono::NaiveDate;
use common::*;
use core_ledger::{AccountId, CashflowRole, CategoryId, TransactionId};
use db_worker::*;
use uuid::Uuid;

fn mint(worker: &DbWorker, anchors: &[(&str, &str)]) {
    worker
        .dispatch(
            meta(),
            WriteCommand::MintMerchantAnchors {
                anchors: anchors
                    .iter()
                    .map(|(key, name)| MerchantAnchor {
                        key: (*key).to_owned(),
                        display_name: (*name).to_owned(),
                    })
                    .collect(),
            },
        )
        .unwrap();
}

fn fill(worker: &DbWorker, fills: &[(TransactionId, CategoryId, i64)]) {
    let mut assignments: Vec<MerchantAssignment> = fills
        .iter()
        .map(
            |(transaction_id, category_id, confidence_bps)| MerchantAssignment {
                transaction_id: *transaction_id,
                category_id: *category_id,
                confidence_bps: *confidence_bps,
            },
        )
        .collect();
    assignments.sort_by_key(|a| a.transaction_id.as_uuid());
    worker
        .dispatch(meta(), WriteCommand::ApplyMerchantMemory { assignments })
        .unwrap();
}

fn write_set_rows(worker: &DbWorker) -> usize {
    worker.last_write_set().expect("a write set").rows.len()
}

// ---- MintMerchantAnchors ----------------------------------------------------------------------

/// The identity id of an earned brand is a frozen function of its key: vaults already hold
/// identities minted under it, so a change needs a new command variant. One literal key, one literal
/// UUID.
#[test]
fn the_identity_id_of_an_earned_brand_is_frozen() {
    let (_dir, worker) = worker();
    mint(
        &worker,
        &[
            ("RITUAL COFFEE", "Ritual Coffee"),
            ("SHELL OIL", "Shell Oil"),
        ],
    );
    let id_of = |key: &str| worker.resolve_merchant_identity(key).unwrap().unwrap().id;
    assert_eq!(
        id_of("RITUAL COFFEE").to_string(),
        "2bdccf44-c5f9-53b3-8732-d5fedc3d1b88"
    );
    assert_eq!(
        id_of("SHELL OIL").to_string(),
        "59bb4e50-e0d2-5b0d-ae6a-abde814179d5"
    );
}

#[test]
fn a_mint_writes_an_auto_identity_and_its_prefix_alias() {
    let (_dir, worker) = worker();
    mint(&worker, &[("RITUAL COFFEE", "Ritual Coffee")]);
    let identity = worker
        .resolve_merchant_identity("RITUAL COFFEE OAKLAND")
        .unwrap()
        .expect("every location of the brand resolves through the prefix alias");
    assert_eq!(identity.display_name, "Ritual Coffee");
    assert_eq!(identity.source, "auto");
    assert!(!identity.is_ambiguous);
    assert_eq!(identity.default_category_id, None);
    let conn = worker.read_connection().unwrap();
    let (source, bps, match_type): (String, i64, String) = conn
        .query_row(
            "SELECT source, confidence_bps, match_type FROM merchant_aliases
              WHERE normalized_key = 'RITUAL COFFEE'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        (source.as_str(), bps, match_type.as_str()),
        ("auto", 8000, "prefix")
    );
}

/// An alias key some other mapping already holds is skipped whole: no identity is left behind
/// without the alias that points at it, and the holder keeps the key.
#[test]
fn a_key_another_identity_holds_is_skipped_whole() {
    let (_dir, worker) = worker();
    let mine = worker
        .create_merchant_identity("Ritual (mine)", None, false, "user")
        .unwrap();
    worker
        .link_merchant_alias("RITUAL COFFEE", mine, "user", 10_000)
        .unwrap();
    let identities = worker.merchant_identity_count().unwrap();

    mint(
        &worker,
        &[
            ("RITUAL COFFEE", "Ritual Coffee"),
            ("SHELL OIL", "Shell Oil"),
        ],
    );

    // Only the free key minted; the held one wrote nothing at all.
    assert_eq!(worker.merchant_identity_count().unwrap(), identities + 1);
    assert_eq!(
        worker
            .resolve_merchant_identity("RITUAL COFFEE")
            .unwrap()
            .unwrap()
            .id,
        mine
    );
    let conn = worker.read_connection().unwrap();
    let orphans: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM merchant_identities mi
              WHERE mi.source = 'auto'
                AND NOT EXISTS (SELECT 1 FROM merchant_aliases ma WHERE ma.merchant_identity_id = mi.id)",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(orphans, 0, "no identity without an alias");
}

#[test]
fn minting_a_held_key_again_writes_nothing_and_the_first_name_stands() {
    let (_dir, worker) = worker();
    mint(&worker, &[("RITUAL COFFEE", "Ritual Coffee")]);
    mint(&worker, &[("RITUAL COFFEE", "A different name")]);
    assert_eq!(
        write_set_rows(&worker),
        0,
        "a skipped mint is an empty write set"
    );
    assert_eq!(
        worker
            .resolve_merchant_identity("RITUAL COFFEE")
            .unwrap()
            .unwrap()
            .display_name,
        "Ritual Coffee"
    );
}

// ---- ApplyMerchantMemory ----------------------------------------------------------------------

struct Fixture {
    worker: DbWorker,
    _dir: tempfile::TempDir,
    category: CategoryId,
    other: CategoryId,
    next: NaiveDate,
    account: AccountId,
}

impl Fixture {
    fn new() -> Self {
        let (dir, worker) = worker();
        let account = AccountId::new();
        create_role_account(&worker, account, "Checking", CashflowRole::LiquidCash);
        let conn = worker.read_connection().unwrap();
        let mut stmt = conn
            .prepare("SELECT id FROM categories ORDER BY name LIMIT 2")
            .unwrap();
        let ids: Vec<Uuid> = stmt
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        drop(stmt);
        drop(conn);
        Self {
            worker,
            _dir: dir,
            category: CategoryId::from_uuid(ids[0]),
            other: CategoryId::from_uuid(ids[1]),
            next: NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
            account,
        }
    }

    fn txn(&mut self) -> TransactionId {
        self.next += chrono::Duration::days(1);
        record_with_counterparty(&self.worker, self.account, -1_000, self.next, "CORNER CAFE")
    }

    fn row(&self, txn: TransactionId) -> Option<(Uuid, String, i64)> {
        self.worker
            .read_connection()
            .unwrap()
            .query_row(
                "SELECT category_id, source, confidence_bps FROM transaction_categorizations
                  WHERE transaction_id = ?1",
                [txn.as_uuid()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .ok()
    }
}

#[test]
fn a_fill_writes_a_rule_categorization_with_the_payload_confidence() {
    let mut f = Fixture::new();
    let t = f.txn();
    fill(&f.worker, &[(t, f.category, 7_500)]);
    assert_eq!(
        f.row(t),
        Some((f.category.as_uuid(), "rule".to_owned(), 7_500))
    );
}

/// "No silent re-tag" (ADR 0030): a transaction that already has a categorization of ANY source is
/// left alone, and a run that skips everything writes nothing.
#[test]
fn a_categorized_transaction_is_skipped_whatever_the_source_and_nothing_is_written() {
    let mut f = Fixture::new();
    let by_user = f.txn();
    f.worker
        .dispatch(
            meta(),
            WriteCommand::RecategorizeTransaction {
                transaction_id: by_user,
                category_id: Some(f.other),
            },
        )
        .unwrap();
    let by_rule = f.txn();
    fill(&f.worker, &[(by_rule, f.other, 9_000)]);
    let by_import = f.txn();
    f.worker
        .read_connection()
        .unwrap()
        .execute(
            "INSERT INTO transaction_categorizations
                (transaction_id, category_id, source, confidence_bps, assigned_at)
             VALUES (?1, ?2, 'import_alias', 5000, '2026-01-01T00:00:00+00:00')",
            rusqlite::params![by_import.as_uuid(), f.other.as_uuid()],
        )
        .unwrap();

    fill(
        &f.worker,
        &[
            (by_user, f.category, 10_000),
            (by_rule, f.category, 10_000),
            (by_import, f.category, 10_000),
        ],
    );

    assert_eq!(write_set_rows(&f.worker), 0, "every entry was skipped");
    for (txn, source, bps) in [
        (by_user, "user", 10_000),
        (by_rule, "rule", 9_000),
        (by_import, "import_alias", 5_000),
    ] {
        assert_eq!(
            f.row(txn),
            Some((f.other.as_uuid(), source.to_owned(), bps)),
            "{source} stays"
        );
    }
}

#[test]
fn a_voided_transaction_is_skipped() {
    let mut f = Fixture::new();
    let voided = f.txn();
    let live = f.txn();
    f.worker
        .dispatch(
            meta(),
            WriteCommand::VoidTransaction {
                transaction_id: voided,
            },
        )
        .unwrap();
    fill(
        &f.worker,
        &[(voided, f.category, 10_000), (live, f.category, 10_000)],
    );
    assert_eq!(f.row(voided), None, "a voided transaction gets no category");
    assert!(
        f.row(live).is_some(),
        "its neighbour in the same command is filled"
    );
}

/// An archived category is not a reason to skip: it mirrors `RecategorizeTransaction`, which accepts
/// one, and what the planner did before the move onto the bus.
#[test]
fn an_archived_category_does_not_stop_a_fill() {
    let mut f = Fixture::new();
    let t = f.txn();
    f.worker
        .dispatch(meta(), WriteCommand::ArchiveCategory(f.category))
        .unwrap();
    fill(&f.worker, &[(t, f.category, 10_000)]);
    assert_eq!(
        f.row(t),
        Some((f.category.as_uuid(), "rule".to_owned(), 10_000))
    );
}

// ---- the pass as a whole ----------------------------------------------------------------------

/// A run an import triggers carries the import's correlation and causation, so the commands queue
/// with it if it queues (ADR 0073 Addendum J).
#[test]
fn a_run_caused_by_another_command_carries_its_correlation_and_causation() {
    let mut f = Fixture::new();
    // A learned category and an uncategorized visit, and a two-location chain to mint.
    for _ in 0..2 {
        let t = f.txn();
        f.worker
            .dispatch(
                meta(),
                WriteCommand::RecategorizeTransaction {
                    transaction_id: t,
                    category_id: Some(f.category),
                },
            )
            .unwrap();
    }
    let target = f.txn();
    for (i, place) in ["SF", "OAKLAND"].into_iter().enumerate() {
        record_with_counterparty(
            &f.worker,
            f.account,
            -500,
            NaiveDate::from_ymd_opt(2026, 4, 1 + u32::try_from(i).unwrap()).unwrap(),
            &format!("RITUAL COFFEE {place}"),
        );
    }

    let cause = meta();
    let applied = f
        .worker
        .apply_merchant_memory_caused_by(Some(&cause))
        .unwrap();
    assert_eq!(applied, 1, "the one uncategorized CORNER CAFE visit");
    assert!(f.row(target).is_some());

    let conn = f.worker.read_connection().unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT operation_type, correlation_id, causation_id FROM operation_log
              WHERE operation_type IN ('mint_merchant_anchors', 'apply_merchant_memory')
              ORDER BY op_seq",
        )
        .unwrap();
    let rows: Vec<(String, Uuid, Option<Uuid>)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let kinds: Vec<&str> = rows.iter().map(|r| r.0.as_str()).collect();
    assert_eq!(kinds, ["mint_merchant_anchors", "apply_merchant_memory"]);
    for (kind, correlation, causation) in rows {
        assert_eq!(correlation, cause.correlation_id, "{kind}");
        assert_eq!(causation, Some(cause.command_id), "{kind}");
    }
}

/// Nothing to do dispatches nothing: a second pass appends no command to the log.
#[test]
fn a_pass_with_nothing_to_do_dispatches_no_command() {
    let f = Fixture::new();
    let before: i64 = f
        .worker
        .read_connection()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM operation_log", [], |r| r.get(0))
        .unwrap();
    assert_eq!(f.worker.apply_merchant_memory().unwrap(), 0);
    let after: i64 = f
        .worker
        .read_connection()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM operation_log", [], |r| r.get(0))
        .unwrap();
    assert_eq!(before, after);
}
