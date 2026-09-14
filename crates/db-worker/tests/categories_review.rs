//! Category taxonomy, recategorization, and review status
//! (moved verbatim from the old in-file `lib.rs` tests module).

mod common;

use chrono::{NaiveDate, Utc};
use common::*;
use core_ledger::{
    Account, AccountFlags, AccountId, CashflowRole, CategoryId, LedgerAccountId, TransactionId,
};
use core_money::{Currency, Money};
use db_worker::*;
use rusqlite::params;
use tempfile::TempDir;
use uuid::Uuid;

#[test]
fn reviewed_defaults_by_source_and_honors_overrides() {
    let (_dir, worker) = worker();
    let account_id = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(Account::new(
                    account_id,
                    LedgerAccountId::new(),
                    "Checking",
                    CashflowRole::LiquidCash,
                    Currency::Usd,
                    AccountFlags::default(),
                )),
                opening_balance: None,
            },
        )
        .unwrap();
    let dt = |day: u32| {
        NaiveDate::from_ymd_opt(2026, 6, day)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
    };
    let record = |amount: i64, day: u32| WriteCommand::RecordTransaction {
        transaction_id: TransactionId::new(),
        account_id,
        amount: Money::new(amount, Currency::Usd),
        occurred_at: dt(day),
    };
    let find = |id: TransactionId| {
        worker
            .recent_transactions(50)
            .unwrap()
            .into_iter()
            .find(|r| r.transaction_id == id)
            .unwrap()
    };

    // A manual transaction defaults REVIEWED.
    worker.dispatch(meta(), record(-4_000, 20)).unwrap();
    let manual_id = worker.recent_transactions(50).unwrap()[0].transaction_id;
    assert!(find(manual_id).reviewed, "manual defaults reviewed");

    // Explicit overrides, latest-wins.
    worker
        .dispatch(
            meta(),
            WriteCommand::MarkReviewed {
                transaction_id: manual_id,
                reviewed: false,
            },
        )
        .unwrap();
    assert!(!find(manual_id).reviewed);
    worker
        .dispatch(
            meta(),
            WriteCommand::MarkReviewed {
                transaction_id: manual_id,
                reviewed: true,
            },
        )
        .unwrap();
    assert!(find(manual_id).reviewed);

    // A transaction with import provenance defaults UNREVIEWED.
    worker.dispatch(meta(), record(-2_000, 21)).unwrap();
    let imported_id = worker
        .recent_transactions(50)
        .unwrap()
        .into_iter()
        .find(|r| r.amount.minor_units() == -2_000)
        .unwrap()
        .transaction_id;
    {
        // Plant an import provenance link directly. FKs are off here — the test only
        // needs the link to EXIST for the reviewed default; it doesn't need a real
        // source_records row.
        let conn = worker.read_connection().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=OFF;").unwrap();
        conn.execute(
            "INSERT INTO source_provenance_links
                    (id, entity_type, entity_id, source_record_id, relationship, created_at)
                 VALUES (?1, 'ledger_transaction', ?2, ?3, 'created_from', ?4)",
            params![
                Uuid::now_v7(),
                imported_id.as_uuid(),
                Uuid::now_v7(),
                Utc::now().to_rfc3339()
            ],
        )
        .unwrap();
    }
    assert!(!find(imported_id).reviewed, "import defaults unreviewed");

    // An explicit review overrides the import default.
    worker
        .dispatch(
            meta(),
            WriteCommand::MarkReviewed {
                transaction_id: imported_id,
                reviewed: true,
            },
        )
        .unwrap();
    assert!(find(imported_id).reviewed);
}

#[test]
fn default_taxonomy_is_seeded_on_open() {
    // personal-cfo-d3p: a fresh vault is seeded with the §9.6 taxonomy.
    let (_dir, worker) = worker();
    let conn = worker.read_connection().unwrap();

    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM categories", [], |r| r.get(0))
        .unwrap();
    assert_eq!(total, 64, "11 groups + 53 leaves");
    let roots: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM categories WHERE parent_id IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(roots, 11, "11 top-level groups");
    let all_system: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM categories WHERE is_system = 0",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(all_system, 0, "every seeded category is is_system");

    // A known leaf has the expected type/behavior and parent.
    let (kind, behavior, parent): (String, String, String) = conn
        .query_row(
            "SELECT c.type, c.forecast_behavior, p.name
                 FROM categories c JOIN categories p ON p.id = c.parent_id
                 WHERE c.name = 'Groceries'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(kind, "expense");
    assert_eq!(behavior, "variable_regular");
    assert_eq!(parent, "Food and Drink");
}

#[test]
fn taxonomy_seed_is_idempotent_across_reopen() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.vault");
    let count_after_open = || {
        let worker = DbWorker::open(&path, KEY).unwrap();
        worker
            .read_connection()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM categories", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap()
    };
    assert_eq!(count_after_open(), 64);
    assert_eq!(count_after_open(), 64, "re-open must not reseed");
}

#[test]
fn categories_reject_invalid_tokens_and_self_parent() {
    let (_dir, worker) = worker();
    let conn = worker.read_connection().unwrap();
    let insert = |type_token: &str, behavior: &str, parent: Option<Uuid>, id: Uuid| {
        conn.execute(
            "INSERT INTO categories (id, name, type, is_system, forecast_behavior,
                    created_at, updated_at, parent_id)
                 VALUES (?1, 'X', ?2, 0, ?3, 'now', 'now', ?4)",
            params![id, type_token, behavior, parent],
        )
    };
    // Bad type token.
    assert!(insert("nonsense", "income", None, Uuid::now_v7()).is_err());
    // Bad forecast_behavior token.
    assert!(insert("expense", "nope", None, Uuid::now_v7()).is_err());
    // Self-parent (1-cycle): parent_id == id.
    let id = Uuid::now_v7();
    assert!(insert("expense", "deterministic", Some(id), id).is_err());
    // A valid row inserts fine.
    assert!(insert("expense", "deterministic", None, Uuid::now_v7()).is_ok());
}

#[test]
fn categories_reject_reparenting_cycles() {
    let (_dir, worker) = worker();
    let conn = worker.read_connection().unwrap();
    let a = Uuid::now_v7();
    let b = Uuid::now_v7();
    let insert = |id: Uuid, parent: Option<Uuid>| {
        conn.execute(
            "INSERT INTO categories (id, name, type, is_system, forecast_behavior,
                    created_at, updated_at, parent_id)
                 VALUES (?1, 'n', 'expense', 0, 'deterministic', 'n', 'n', ?2)",
            params![id, parent],
        )
    };
    insert(a, None).unwrap();
    insert(b, Some(a)).unwrap(); // b is a child of a
                                 // Making a a child of b would close a cycle a->b->a.
    let result = conn.execute(
        "UPDATE categories SET parent_id = ?1 WHERE id = ?2",
        params![b, a],
    );
    assert!(result.is_err(), "re-parenting cycle must be rejected");
}

#[test]
fn category_alias_must_be_unique() {
    let (_dir, worker) = worker();
    let conn = worker.read_connection().unwrap();
    let category_id: Uuid = conn
        .query_row("SELECT id FROM categories LIMIT 1", [], |r| r.get(0))
        .unwrap();
    let insert_alias = || {
        conn.execute(
            "INSERT INTO category_aliases (id, category_id, alias, created_at)
                 VALUES (?1, ?2, 'whole foods', 'now')",
            params![Uuid::now_v7(), category_id],
        )
    };
    insert_alias().unwrap();
    assert!(insert_alias().is_err(), "duplicate alias must be rejected");
}

/// bac: the seeded §9.6 taxonomy reads back as a hierarchy of system categories.
#[test]
fn category_views_returns_the_seeded_taxonomy() {
    let (_dir, worker) = worker();
    let cats = worker.category_views().unwrap();
    assert!(
        !cats.is_empty(),
        "the §9.6 taxonomy is seeded at vault create"
    );
    assert!(
        cats.iter().all(|c| c.is_system),
        "a fresh vault has only the system defaults"
    );
    assert!(
        cats.iter().any(|c| c.parent_id.is_none()),
        "has top-level groups"
    );
    assert!(
        cats.iter().any(|c| c.parent_id.is_some()),
        "has leaf categories under groups"
    );
}

/// bac: a user category can be created, archived (hidden), and reinstated; the
/// forecast behavior is derived from its type (ADR 0030).
#[test]
fn create_archive_reinstate_a_user_category() {
    let (_dir, worker) = worker();
    let id = CategoryId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateCategory {
                id,
                parent_id: None,
                name: "Coffee Shops".to_owned(),
                category_type: "expense".to_owned(),
                color: Some("#DB8F6B".to_owned()),
                icon: None,
            },
        )
        .unwrap();

    let find = |w: &DbWorker| {
        w.category_views()
            .unwrap()
            .into_iter()
            .find(|c| c.id == id)
            .expect("category present")
    };
    let created = find(&worker);
    assert_eq!(created.name, "Coffee Shops");
    assert!(!created.is_system, "a created category is a user category");
    assert!(!created.archived);
    assert_eq!(created.forecast_behavior, "variable_regular");

    worker
        .dispatch(meta(), WriteCommand::ArchiveCategory(id))
        .unwrap();
    assert!(find(&worker).archived, "archived hides the category");

    worker
        .dispatch(meta(), WriteCommand::ReinstateCategory(id))
        .unwrap();
    assert!(!find(&worker).archived, "reinstate un-hides it");
}

/// bac: creating under a non-existent parent is rejected.
#[test]
fn create_category_rejects_an_unknown_parent() {
    let (_dir, worker) = worker();
    let result = worker.dispatch(
        meta(),
        WriteCommand::CreateCategory {
            id: CategoryId::new(),
            parent_id: Some(CategoryId::new()),
            name: "Orphan".to_owned(),
            category_type: "expense".to_owned(),
            color: None,
            icon: None,
        },
    );
    assert!(result.is_err(), "unknown parent must be rejected");
}

/// bac: a user category can be renamed/recolored and re-parented (ADR 0030).
#[test]
fn update_and_move_a_user_category() {
    let (_dir, worker) = worker();
    let group = CategoryId::new();
    let leaf = CategoryId::new();
    let create = |id, parent, name: &str| WriteCommand::CreateCategory {
        id,
        parent_id: parent,
        name: name.to_owned(),
        category_type: "expense".to_owned(),
        color: None,
        icon: None,
    };
    worker
        .dispatch(meta(), create(group, None, "Hobbies"))
        .unwrap();
    worker
        .dispatch(meta(), create(leaf, None, "Guitar"))
        .unwrap();

    let find = |w: &DbWorker, id| {
        w.category_views()
            .unwrap()
            .into_iter()
            .find(|c: &CategoryView| c.id == id)
            .expect("category present")
    };

    // Rename + recolor + set an emoji icon (personal-cfo-4d8.24.10).
    worker
        .dispatch(
            meta(),
            WriteCommand::UpdateCategory {
                id: leaf,
                name: "Guitar Lessons".to_owned(),
                color: Some("#006341".to_owned()),
                icon: Some("🎸".to_owned()),
            },
        )
        .unwrap();
    let edited = find(&worker, leaf);
    assert_eq!(edited.name, "Guitar Lessons");
    assert_eq!(edited.color.as_deref(), Some("#006341"));
    assert_eq!(
        edited.icon.as_deref(),
        Some("🎸"),
        "the emoji icon round-trips"
    );
    assert!(
        edited.parent_id.is_none(),
        "still top-level before the move"
    );

    // Re-parent under the group.
    worker
        .dispatch(
            meta(),
            WriteCommand::MoveCategory {
                id: leaf,
                new_parent_id: Some(group),
            },
        )
        .unwrap();
    assert_eq!(
        find(&worker, leaf).parent_id,
        Some(group),
        "moved under group"
    );

    // Move back to top-level.
    worker
        .dispatch(
            meta(),
            WriteCommand::MoveCategory {
                id: leaf,
                new_parent_id: None,
            },
        )
        .unwrap();
    assert!(find(&worker, leaf).parent_id.is_none(), "back to top-level");
}

/// kogu (ADR 0030 amendment): a system category's IDENTITY (name/parent) is immutable,
/// but its APPEARANCE (color + icon) is user-customizable; archive is still allowed and
/// re-parent still rejected.
#[test]
fn system_categories_accept_appearance_but_preserve_identity() {
    let (_dir, worker) = worker();
    let system = worker
        .category_views()
        .unwrap()
        .into_iter()
        .find(|c| c.is_system)
        .expect("seeded system categories");
    let system_id = system.id;
    let original_name = system.name.clone();
    let original_parent = system.parent_id;
    let reload = |w: &DbWorker| {
        w.category_views()
            .unwrap()
            .into_iter()
            .find(|c| c.id == system_id)
            .unwrap()
    };

    // Appearance update succeeds (sending the unchanged canonical name).
    worker
        .dispatch(
            meta(),
            WriteCommand::UpdateCategory {
                id: system_id,
                name: original_name.clone(),
                color: Some("#006341".to_owned()),
                icon: Some("🎸".to_owned()),
            },
        )
        .unwrap();
    let after = reload(&worker);
    assert_eq!(after.color.as_deref(), Some("#006341"), "color applied");
    assert_eq!(after.icon.as_deref(), Some("🎸"), "icon applied");

    // A submitted DIFFERENT name is IGNORED — identity stays fixed (the core invariant).
    worker
        .dispatch(
            meta(),
            WriteCommand::UpdateCategory {
                id: system_id,
                name: "Renamed".to_owned(),
                color: Some("#111111".to_owned()),
                icon: Some("🍎".to_owned()),
            },
        )
        .unwrap();
    let after = reload(&worker);
    assert_eq!(
        after.name, original_name,
        "a system category's name is preserved even when a different name is submitted"
    );
    assert_eq!(after.parent_id, original_parent, "parent unchanged");
    assert_eq!(
        after.color.as_deref(),
        Some("#111111"),
        "appearance still applied"
    );
    assert_eq!(after.icon.as_deref(), Some("🍎"));

    // Re-parent is still rejected (identity immutable).
    let r#move = worker.dispatch(
        meta(),
        WriteCommand::MoveCategory {
            id: system_id,
            new_parent_id: None,
        },
    );
    assert!(r#move.is_err(), "cannot re-parent a system category");

    // Archive is still permitted.
    worker
        .dispatch(meta(), WriteCommand::ArchiveCategory(system_id))
        .unwrap();
    assert!(
        reload(&worker).archived,
        "a system category can still be archived"
    );
}

/// kogu: a category can be created with an emoji icon (and color) in one command.
#[test]
fn create_persists_an_icon_and_color() {
    let (_dir, worker) = worker();
    let id = CategoryId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateCategory {
                id,
                parent_id: None,
                name: "Hobbies".to_owned(),
                category_type: "expense".to_owned(),
                color: Some("#DB8F6B".to_owned()),
                icon: Some("🎨".to_owned()),
            },
        )
        .unwrap();
    let created = worker
        .category_views()
        .unwrap()
        .into_iter()
        .find(|c| c.id == id)
        .expect("created category");
    assert_eq!(created.icon.as_deref(), Some("🎨"));
    assert_eq!(created.color.as_deref(), Some("#DB8F6B"));
}

/// bac: re-parenting that would close a cycle is rejected (ADR 0030).
#[test]
fn move_category_rejects_a_cycle() {
    let (_dir, worker) = worker();
    let a = CategoryId::new();
    let b = CategoryId::new();
    let create = |id, parent, name: &str| WriteCommand::CreateCategory {
        id,
        parent_id: parent,
        name: name.to_owned(),
        category_type: "expense".to_owned(),
        color: None,
        icon: None,
    };
    worker.dispatch(meta(), create(a, None, "A")).unwrap();
    worker.dispatch(meta(), create(b, Some(a), "B")).unwrap();

    // a -> b -> a would be a cycle.
    let result = worker.dispatch(
        meta(),
        WriteCommand::MoveCategory {
            id: a,
            new_parent_id: Some(b),
        },
    );
    assert!(result.is_err(), "a re-parenting cycle must be rejected");

    // Self-parent is likewise rejected.
    let self_parent = worker.dispatch(
        meta(),
        WriteCommand::MoveCategory {
            id: a,
            new_parent_id: Some(a),
        },
    );
    assert!(self_parent.is_err(), "a category cannot be its own parent");
}

/// bac: a transaction's category can be set, re-assigned (latest wins), and
/// cleared; the transactions list reflects it (ADR 0030).
#[test]
fn recategorize_a_transaction_sets_and_clears_the_category() {
    let (_dir, worker) = worker();
    let account = Account::new(
        AccountId::new(),
        LedgerAccountId::new(),
        "Checking",
        CashflowRole::LiquidCash,
        Currency::Usd,
        AccountFlags::default(),
    );
    let account_id = account.id();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(account),
                opening_balance: None,
            },
        )
        .unwrap();
    worker
        .dispatch(
            meta(),
            WriteCommand::RecordTransaction {
                transaction_id: TransactionId::new(),
                account_id,
                amount: Money::new(-4_000, Currency::Usd),
                occurred_at: Utc::now(),
            },
        )
        .unwrap();
    let txn_id = worker.recent_transactions(1).unwrap()[0].transaction_id;

    let groceries = CategoryId::new();
    let dining = CategoryId::new();
    for (id, name) in [(groceries, "Groceries"), (dining, "Dining")] {
        worker
            .dispatch(
                meta(),
                WriteCommand::CreateCategory {
                    id,
                    parent_id: None,
                    name: name.to_owned(),
                    category_type: "expense".to_owned(),
                    color: None,
                    icon: None,
                },
            )
            .unwrap();
    }
    let category_of = |w: &DbWorker| w.recent_transactions(1).unwrap()[0].category_id;

    // Uncategorized to start.
    assert_eq!(category_of(&worker), None);

    // Assign, then re-assign (latest wins).
    worker
        .dispatch(
            meta(),
            WriteCommand::RecategorizeTransaction {
                transaction_id: txn_id,
                category_id: Some(groceries),
            },
        )
        .unwrap();
    assert_eq!(category_of(&worker), Some(groceries));
    worker
        .dispatch(
            meta(),
            WriteCommand::RecategorizeTransaction {
                transaction_id: txn_id,
                category_id: Some(dining),
            },
        )
        .unwrap();
    assert_eq!(category_of(&worker), Some(dining), "latest assignment wins");

    // Clear it.
    worker
        .dispatch(
            meta(),
            WriteCommand::RecategorizeTransaction {
                transaction_id: txn_id,
                category_id: None,
            },
        )
        .unwrap();
    assert_eq!(category_of(&worker), None, "cleared");
}

/// bac: recategorizing rejects an unknown transaction or category.
#[test]
fn recategorize_rejects_unknown_transaction_or_category() {
    let (_dir, worker) = worker();
    // No such transaction.
    assert!(worker
        .dispatch(
            meta(),
            WriteCommand::RecategorizeTransaction {
                transaction_id: TransactionId::new(),
                category_id: None,
            },
        )
        .is_err());

    // A real transaction, but an unknown category.
    let account = Account::new(
        AccountId::new(),
        LedgerAccountId::new(),
        "Checking",
        CashflowRole::LiquidCash,
        Currency::Usd,
        AccountFlags::default(),
    );
    let account_id = account.id();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(account),
                opening_balance: None,
            },
        )
        .unwrap();
    worker
        .dispatch(
            meta(),
            WriteCommand::RecordTransaction {
                transaction_id: TransactionId::new(),
                account_id,
                amount: Money::new(-1_000, Currency::Usd),
                occurred_at: Utc::now(),
            },
        )
        .unwrap();
    let txn_id = worker.recent_transactions(1).unwrap()[0].transaction_id;
    assert!(worker
        .dispatch(
            meta(),
            WriteCommand::RecategorizeTransaction {
                transaction_id: txn_id,
                category_id: Some(CategoryId::new()),
            },
        )
        .is_err());
}
