//! The filtered / sorted / paged transaction read (personal-cfo-3fdd.1):
//! server-side search over all history, replacing the client-side scan of the
//! recent 200-row window.

mod common;

use chrono::NaiveDate;
use common::*;
use core_ledger::{
    Account, AccountFlags, AccountId, CashflowRole, CategoryId, LedgerAccountId, TagId,
    TransactionId,
};
use core_money::{Currency, Money};
use db_worker::*;
use uuid::Uuid;

/// A vault with two accounts and 30 transactions: one per day across June 2026
/// (day `i+1`, amount −$(i+1)) alternating between the accounts, with a note on
/// two rows and a tag on three.
fn seeded_worker() -> (
    tempfile::TempDir,
    DbWorker,
    [AccountId; 2],
    TagId,
    Vec<TransactionId>,
) {
    let (dir, worker) = worker();
    let checking = AccountId::new();
    let savings = AccountId::new();
    for (id, name) in [(checking, "Checking"), (savings, "Savings")] {
        worker
            .dispatch(
                meta(),
                WriteCommand::CreateAccount {
                    account: Box::new(Account::new(
                        id,
                        LedgerAccountId::new(),
                        name,
                        CashflowRole::LiquidCash,
                        Currency::Usd,
                        AccountFlags::default(),
                    )),
                    opening_balance: None,
                },
            )
            .unwrap();
    }
    for i in 0..30u32 {
        let account_id = if i % 2 == 0 { checking } else { savings };
        worker
            .dispatch(
                meta(),
                WriteCommand::RecordTransaction {
                    transaction_id: TransactionId::new(),
                    account_id,
                    amount: Money::new(-(i64::from(i) + 1) * 100, Currency::Usd),
                    occurred_at: NaiveDate::from_ymd_opt(2026, 6, i + 1)
                        .unwrap()
                        .and_hms_opt(12, 0, 0)
                        .unwrap()
                        .and_utc(),
                },
            )
            .unwrap();
    }
    // Oldest-first ids (recent_transactions returns newest first).
    let ids: Vec<TransactionId> = worker
        .recent_transactions(50)
        .unwrap()
        .into_iter()
        .rev()
        .map(|t| t.transaction_id)
        .collect();
    assert_eq!(ids.len(), 30);

    // Notes on the June 3 + June 20 rows; a tag on the June 1/2/3 rows.
    for (idx, note) in [(2usize, "coffee with Sam"), (19, "COFFEE beans, 5kg")] {
        worker
            .dispatch(
                meta(),
                WriteCommand::SetNote {
                    transaction_id: ids[idx],
                    note: Some(note.to_owned()),
                },
            )
            .unwrap();
    }
    let tag = TagId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateTag {
                id: tag,
                name: "Vacation".to_owned(),
                color: None,
            },
        )
        .unwrap();
    for id in ids.iter().take(3) {
        worker
            .dispatch(
                meta(),
                WriteCommand::SetTags {
                    transaction_id: *id,
                    tag_ids: vec![tag],
                },
            )
            .unwrap();
    }
    (dir, worker, [checking, savings], tag, ids)
}

#[test]
fn unfiltered_page_matches_the_recent_list_and_windows_by_offset() {
    let (_dir, worker, _, _, _) = seeded_worker();

    // Offset 0 is exactly the head of the recent list.
    let page = worker
        .transaction_page(&TransactionPageQuery {
            limit: 10,
            ..TransactionPageQuery::default()
        })
        .unwrap();
    assert_eq!(page.total, 30);
    assert_eq!(page.rows.len(), 10);
    let recent = worker.recent_transactions(10).unwrap();
    assert_eq!(page.rows, recent, "first page == the recent list's head");

    // Successive windows tile the set without overlap or gaps.
    let mut seen = Vec::new();
    for offset in [0, 10, 20, 30] {
        let window = worker
            .transaction_page(&TransactionPageQuery {
                limit: 10,
                offset,
                ..TransactionPageQuery::default()
            })
            .unwrap();
        assert_eq!(window.total, 30, "total is offset-independent");
        assert_eq!(window.rows.len(), if offset == 30 { 0 } else { 10 });
        seen.extend(window.rows.into_iter().map(|r| r.transaction_id));
    }
    assert_eq!(seen.len(), 30);
    let unique: std::collections::HashSet<Uuid> = seen.iter().map(|id| id.as_uuid()).collect();
    assert_eq!(unique.len(), 30, "windows never overlap");
}

#[test]
fn query_spans_notes_and_account_names_case_insensitively() {
    let (_dir, worker, _, _, ids) = seeded_worker();

    // "coffee" hits the two notes only — regardless of case, on both sides.
    let page = worker
        .transaction_page(&TransactionPageQuery {
            query: Some("Coffee".to_owned()),
            limit: 10,
            ..TransactionPageQuery::default()
        })
        .unwrap();
    assert_eq!(page.total, 2, "the query spans notes");
    let hits: Vec<TransactionId> = page.rows.iter().map(|r| r.transaction_id).collect();
    assert_eq!(hits, vec![ids[19], ids[2]], "newest first");

    // The account name matches too (half the rows live on "Savings").
    let page = worker
        .transaction_page(&TransactionPageQuery {
            query: Some("savings".to_owned()),
            limit: 50,
            ..TransactionPageQuery::default()
        })
        .unwrap();
    assert_eq!(page.total, 15);

    // A LIKE wildcard in user input is literal, not a pattern.
    let page = worker
        .transaction_page(&TransactionPageQuery {
            query: Some("%".to_owned()),
            limit: 10,
            ..TransactionPageQuery::default()
        })
        .unwrap();
    assert_eq!(page.total, 0, "'%' matches nothing rather than everything");
}

#[test]
fn date_bounds_are_inclusive_on_both_edges() {
    let (_dir, worker, _, _, ids) = seeded_worker();

    let page = worker
        .transaction_page(&TransactionPageQuery {
            from: NaiveDate::from_ymd_opt(2026, 6, 10),
            to: NaiveDate::from_ymd_opt(2026, 6, 12),
            limit: 10,
            ..TransactionPageQuery::default()
        })
        .unwrap();
    assert_eq!(page.total, 3, "June 10, 11, and 12 all included");
    let hits: Vec<TransactionId> = page.rows.iter().map(|r| r.transaction_id).collect();
    // Newest first: the 12th, 11th, 10th (rows are timestamped mid-day, so both
    // boundary days prove the inclusive comparison).
    assert_eq!(hits, vec![ids[11], ids[10], ids[9]]);

    // Open-ended bounds work independently.
    let from_only = worker
        .transaction_page(&TransactionPageQuery {
            from: NaiveDate::from_ymd_opt(2026, 6, 28),
            limit: 10,
            ..TransactionPageQuery::default()
        })
        .unwrap();
    assert_eq!(from_only.total, 3);
    let to_only = worker
        .transaction_page(&TransactionPageQuery {
            to: NaiveDate::from_ymd_opt(2026, 6, 2),
            limit: 10,
            ..TransactionPageQuery::default()
        })
        .unwrap();
    assert_eq!(to_only.total, 2);
}

#[test]
fn balance_after_is_the_full_ledger_balance_regardless_of_filters() {
    // personal-cfo-ttuy. The number answers "what did this account hold at that moment",
    // which does not depend on what the reader is currently looking at. So a FILTERED page
    // must report the same balances as the unfiltered one for the same rows — the property
    // a running total computed over the returned rows would fail.
    let (_dir, worker, [checking, _], tag, _ids) = seeded_worker();

    let all = worker
        .transaction_page(&TransactionPageQuery {
            account_ids: vec![checking],
            with_balances: true,
            limit: 100,
            ..TransactionPageQuery::default()
        })
        .unwrap();
    assert!(
        all.rows.iter().all(|r| r.balance_after_minor.is_some()),
        "every row on an account-scoped page gets a balance",
    );

    let filtered = worker
        .transaction_page(&TransactionPageQuery {
            account_ids: vec![checking],
            tag_id: Some(tag),
            with_balances: true,
            limit: 100,
            ..TransactionPageQuery::default()
        })
        .unwrap();
    assert!(
        !filtered.rows.is_empty(),
        "the tag filter matches something"
    );

    for row in &filtered.rows {
        let unfiltered = all
            .rows
            .iter()
            .find(|r| r.transaction_id == row.transaction_id)
            .expect("the filtered row is also in the unfiltered page");
        assert_eq!(
            row.balance_after_minor, unfiltered.balance_after_minor,
            "filtering changed a balance — it must come from the full ledger, not the page",
        );
    }

    // Off by default, so no caller pays for it accidentally.
    let plain = worker
        .transaction_page(&TransactionPageQuery {
            account_ids: vec![checking],
            limit: 5,
            ..TransactionPageQuery::default()
        })
        .unwrap();
    assert!(plain.rows.iter().all(|r| r.balance_after_minor.is_none()));
}

#[test]
fn account_category_tag_and_review_facets_narrow_the_set() {
    let (_dir, worker, [checking, _], tag, ids) = seeded_worker();

    let by_account = worker
        .transaction_page(&TransactionPageQuery {
            account_ids: vec![checking],
            limit: 50,
            ..TransactionPageQuery::default()
        })
        .unwrap();
    assert_eq!(by_account.total, 15);
    assert!(by_account.rows.iter().all(|r| r.account_id == checking));

    // The SET case (personal-cfo-4d8.27.9.4). The Debt page scopes its embedded list to a
    // multi-account selection (ADR 0057 §3), so the facet had to widen from one id to a
    // set — and the single-account case above is now just the one-element instance of the
    // same field, which is why both live in one test.
    let (_d2, w2, [chk, sav], _t2, _i2) = seeded_worker();
    let both = w2
        .transaction_page(&TransactionPageQuery {
            account_ids: vec![chk, sav],
            limit: 100,
            ..TransactionPageQuery::default()
        })
        .unwrap();
    let just_one = w2
        .transaction_page(&TransactionPageQuery {
            account_ids: vec![chk],
            limit: 100,
            ..TransactionPageQuery::default()
        })
        .unwrap();
    let unscoped = w2
        .transaction_page(&TransactionPageQuery {
            limit: 100,
            ..TransactionPageQuery::default()
        })
        .unwrap();
    assert!(
        both.total > just_one.total,
        "two accounts must return strictly more than one — a set that silently used only \
         the first id would pass every single-account test",
    );
    assert!(
        both.rows
            .iter()
            .all(|r| r.account_id == chk || r.account_id == sav),
        "and nothing outside the set leaks in",
    );
    // Empty means NO constraint, not "no accounts" — getting that inverted would return
    // an empty list for every unfiltered read in the app.
    assert_eq!(unscoped.total, both.total.max(unscoped.total));
    assert!(unscoped.total >= both.total);

    let by_tag = worker
        .transaction_page(&TransactionPageQuery {
            tag_id: Some(tag),
            limit: 50,
            ..TransactionPageQuery::default()
        })
        .unwrap();
    assert_eq!(by_tag.total, 3);
    assert!(by_tag.rows.iter().all(|r| r.tag_ids.contains(&tag)));

    // Every seeded row is manual (uncategorized + reviewed-by-default), so the
    // uncategorized sentinel matches all and unreviewed-only matches none.
    let uncategorized = worker
        .transaction_page(&TransactionPageQuery {
            category: Some(CategoryFilter::Uncategorized),
            limit: 1,
            ..TransactionPageQuery::default()
        })
        .unwrap();
    assert_eq!(uncategorized.total, 30);
    let unreviewed = worker
        .transaction_page(&TransactionPageQuery {
            unreviewed_only: true,
            limit: 1,
            ..TransactionPageQuery::default()
        })
        .unwrap();
    assert_eq!(unreviewed.total, 0);

    // Marking one row unreviewed surfaces exactly it.
    worker
        .dispatch(
            meta(),
            WriteCommand::MarkReviewed {
                transaction_id: ids[0],
                reviewed: false,
            },
        )
        .unwrap();
    let unreviewed = worker
        .transaction_page(&TransactionPageQuery {
            unreviewed_only: true,
            limit: 10,
            ..TransactionPageQuery::default()
        })
        .unwrap();
    assert_eq!(unreviewed.total, 1);
    assert_eq!(unreviewed.rows[0].transaction_id, ids[0]);
}

#[test]
fn sort_orders_are_deterministic_and_total() {
    let (_dir, worker, _, _, ids) = seeded_worker();

    let oldest = worker
        .transaction_page(&TransactionPageQuery {
            sort: TransactionSortOrder::OldestFirst,
            limit: 3,
            ..TransactionPageQuery::default()
        })
        .unwrap();
    let hits: Vec<TransactionId> = oldest.rows.iter().map(|r| r.transaction_id).collect();
    assert_eq!(hits, vec![ids[0], ids[1], ids[2]]);

    // Amounts are −100·(day): amount_desc = smallest magnitude first (June 1),
    // amount_asc = most negative first (June 30).
    let desc = worker
        .transaction_page(&TransactionPageQuery {
            sort: TransactionSortOrder::AmountDesc,
            limit: 1,
            ..TransactionPageQuery::default()
        })
        .unwrap();
    assert_eq!(desc.rows[0].transaction_id, ids[0]);
    let asc = worker
        .transaction_page(&TransactionPageQuery {
            sort: TransactionSortOrder::AmountAsc,
            limit: 1,
            ..TransactionPageQuery::default()
        })
        .unwrap();
    assert_eq!(asc.rows[0].transaction_id, ids[29]);
}

/// ADR 0052 §2: filtering to a category means its whole SUBTREE, and reaches transactions
/// whose SPLIT LINES carry the category.
///
/// Both matter because the spend chart's drill-down is this filter: a parent cell counts
/// its children and counts split lines, so without these the list would show fewer rows
/// than the cell the user just clicked.
#[test]
fn the_category_filter_spans_the_subtree_and_split_lines() {
    let (_dir, worker, accounts, _tag, _txns) = seeded_worker();
    let checking = accounts[0];

    let conn = worker.read_connection().unwrap();
    let parent: Uuid = conn
        .query_row(
            "SELECT id FROM categories WHERE parent_id IS NULL AND type = 'expense' LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let child: Uuid = conn
        .query_row(
            "SELECT id FROM categories WHERE parent_id = ?1 LIMIT 1",
            [parent],
            |r| r.get(0),
        )
        .unwrap();
    drop(conn);

    let record = |cents: i64, day: u32| -> TransactionId {
        let id = TransactionId::new();
        worker
            .dispatch(
                meta(),
                WriteCommand::RecordTransaction {
                    transaction_id: id,
                    account_id: checking,
                    amount: Money::new(cents, Currency::Usd),
                    occurred_at: NaiveDate::from_ymd_opt(2026, 7, day)
                        .unwrap()
                        .and_hms_opt(12, 0, 0)
                        .unwrap()
                        .and_utc(),
                },
            )
            .unwrap();
        id
    };
    let categorize = |id: TransactionId, category: Uuid| {
        worker
            .dispatch(
                meta(),
                WriteCommand::RecategorizeTransaction {
                    transaction_id: id,
                    category_id: Some(CategoryId::from_uuid(category)),
                },
            )
            .unwrap();
    };

    // One filed on the CHILD, and one filed on the parent but SPLIT with a child line.
    let on_child = record(-4_000, 1);
    categorize(on_child, child);
    let split = record(-9_000, 2);
    categorize(split, parent);
    worker
        .dispatch(
            meta(),
            WriteCommand::SetSplits {
                transaction_id: split,
                lines: vec![
                    SplitLineInput {
                        amount: Money::new(-6_000, Currency::Usd),
                        category_id: Some(CategoryId::from_uuid(child)),
                        note: None,
                        tag_ids: vec![],
                    },
                    SplitLineInput {
                        amount: Money::new(-3_000, Currency::Usd),
                        category_id: None,
                        note: None,
                        tag_ids: vec![],
                    },
                ],
            },
        )
        .unwrap();

    // Filtering to the PARENT must return both: the child row (subtree) and the split
    // row (its line carries the child). Before ADR 0052 this returned only the split row,
    // and only because its own categorization happened to be the parent.
    let page = worker
        .transaction_page(&TransactionPageQuery {
            limit: 50,
            category: Some(CategoryFilter::Category(CategoryId::from_uuid(parent))),
            ..TransactionPageQuery::default()
        })
        .unwrap();
    let ids: Vec<TransactionId> = page.rows.iter().map(|r| r.transaction_id).collect();
    assert!(
        ids.contains(&on_child),
        "a child-category row is in the subtree"
    );
    assert!(ids.contains(&split), "the split row is reachable");

    // Filtering to the CHILD reaches the split row through its line, even though the
    // transaction's own categorization is the parent.
    let page = worker
        .transaction_page(&TransactionPageQuery {
            limit: 50,
            category: Some(CategoryFilter::Category(CategoryId::from_uuid(child))),
            ..TransactionPageQuery::default()
        })
        .unwrap();
    let ids: Vec<TransactionId> = page.rows.iter().map(|r| r.transaction_id).collect();
    assert!(ids.contains(&on_child));
    assert!(
        ids.contains(&split),
        "a split line's category must reach its transaction",
    );
    // …and the page total agrees with the rows (the count query shares the clause).
    assert_eq!(page.total as usize, page.rows.len(), "count matches rows");

    // EXCLUSION: a row in a sibling subtree must NOT come back — otherwise a clause that
    // matched everything would satisfy every assertion above.
    let sibling: Uuid = {
        let conn = worker.read_connection().unwrap();
        conn.query_row(
            "SELECT id FROM categories WHERE parent_id IS NULL AND type = 'expense'
              AND id <> ?1 LIMIT 1",
            [parent],
            |r| r.get(0),
        )
        .unwrap()
    };
    let elsewhere = record(-1_100, 3);
    categorize(elsewhere, sibling);
    let page = worker
        .transaction_page(&TransactionPageQuery {
            limit: 50,
            category: Some(CategoryFilter::Category(CategoryId::from_uuid(parent))),
            ..TransactionPageQuery::default()
        })
        .unwrap();
    let ids: Vec<TransactionId> = page.rows.iter().map(|r| r.transaction_id).collect();
    assert!(
        !ids.contains(&elsewhere),
        "a sibling subtree must not match",
    );

    // SPLITS WIN, in the list as in the chart (ADR 0052 §3): the split's own
    // categorization is the PARENT, but its lines are on the child — filtering to the
    // parent must NOT return it via that stale categorization once it has been split.
    // (It is still reachable under the parent above, because the child is in the
    // parent's subtree and the LINE carries the child.)
    let stale = record(-7_000, 4);
    categorize(stale, sibling);
    worker
        .dispatch(
            meta(),
            WriteCommand::SetSplits {
                transaction_id: stale,
                lines: vec![SplitLineInput {
                    amount: Money::new(-7_000, Currency::Usd),
                    category_id: Some(CategoryId::from_uuid(child)),
                    note: None,
                    tag_ids: vec![],
                }],
            },
        )
        .unwrap();
    let page = worker
        .transaction_page(&TransactionPageQuery {
            limit: 50,
            category: Some(CategoryFilter::Category(CategoryId::from_uuid(sibling))),
            ..TransactionPageQuery::default()
        })
        .unwrap();
    let ids: Vec<TransactionId> = page.rows.iter().map(|r| r.transaction_id).collect();
    assert!(
        !ids.contains(&stale),
        "once split, the parent categorization no longer decides the filter",
    );
}

/// Every facet bound at once — the category clause is the builder's only variable-arity
/// clause, so a reordering would desynchronize placeholders from params. Nothing else
/// exercises them together.
#[test]
fn every_filter_facet_binds_together() {
    let (_dir, worker, accounts, tag, _txns) = seeded_worker();
    let category: Uuid = {
        let conn = worker.read_connection().unwrap();
        conn.query_row(
            "SELECT id FROM categories WHERE parent_id IS NULL AND type = 'expense' LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };
    let page = worker
        .transaction_page(&TransactionPageQuery {
            limit: 10,
            offset: 0,
            query: Some("coffee".to_owned()),
            account_ids: vec![accounts[0]],
            tag_id: Some(tag),
            category: Some(CategoryFilter::Category(CategoryId::from_uuid(category))),
            from: Some(NaiveDate::from_ymd_opt(2026, 6, 1).unwrap()),
            to: Some(NaiveDate::from_ymd_opt(2026, 6, 30).unwrap()),
            unreviewed_only: true,
            ..TransactionPageQuery::default()
        })
        .expect("every facet binds without a placeholder/param mismatch");
    // The combination is deliberately over-constrained; what matters is that it RUNS.
    assert!(page.rows.len() as u32 <= page.total.max(page.rows.len() as u32));
}
