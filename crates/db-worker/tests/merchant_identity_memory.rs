//! Merchant identity, merchant memory auto-categorization, grouping, and recurring-candidate detection
//! (moved verbatim from the old in-file `lib.rs` tests module).

mod common;

use chrono::NaiveDate;
use common::*;
use core_ledger::{
    Account, AccountFlags, AccountId, BillContractId, CashflowRole, CategoryId, LedgerAccountId,
    RecurringEventId, TransactionId,
};
use core_money::{Currency, Money};
use db_worker::*;
use pay_schedule::Frequency;
use rusqlite::params;
use uuid::Uuid;

/// 98ql: a monthly merchant across the realized history is surfaced as a candidate; a
/// one-off is not; and once the merchant is tracked as an active bill it stops being
/// suggested.
#[test]
fn recurring_candidates_detects_monthly_and_excludes_tracked_merchants() {
    let (_dir, worker) = worker();
    let checking = AccountId::new();
    create_role_account(&worker, checking, "Checking", CashflowRole::LiquidCash);
    for month in 1..=6u32 {
        record_with_counterparty(
            &worker,
            checking,
            -1_099,
            NaiveDate::from_ymd_opt(2026, month, 5).unwrap(),
            "SPOTIFY",
        );
    }
    // A single unrelated purchase — not recurring.
    record_with_counterparty(
        &worker,
        checking,
        -8_000,
        NaiveDate::from_ymd_opt(2026, 3, 9).unwrap(),
        "BEST BUY",
    );

    let candidates = worker.recurring_candidates().unwrap();
    assert_eq!(candidates.len(), 1, "only the recurring merchant");
    assert_eq!(candidates[0].candidate.merchant_key, "SPOTIFY");
    assert_eq!(candidates[0].candidate.frequency, "monthly");
    assert_eq!(candidates[0].candidate.amount_minor, 1_099);
    assert!(
        candidates[0].candidate.confidence_bps >= 9_000,
        "clean 6-month history"
    );

    // Track Spotify as a bill → it drops out of the suggestions.
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateRecurringBill {
                event_id: RecurringEventId::new(),
                contract_id: BillContractId::new(),
                name: "Spotify".to_owned(),
                amount: Money::new(1_099, Currency::Usd),
                bill_type: "subscription".to_owned(),
                frequency: Frequency::Monthly,
                anchor: NaiveDate::from_ymd_opt(2026, 7, 5).unwrap(),
                autopay_account_id: Some(checking),
                description: None,
                source_merchant_key: None,
                category_id: None,
                tag_ids: Vec::new(),
            },
        )
        .unwrap();
    assert!(
        worker.recurring_candidates().unwrap().is_empty(),
        "a merchant already tracked as a bill is not re-suggested",
    );
}

/// 5n4.8: a bill promoted from a candidate persists the source merchant key, so the
/// suggestion stays suppressed even when the bill is renamed away from the merchant —
/// the exclusion keys on the durable merchant key, not the (mutable) name.
#[test]
fn recurring_candidate_stays_suppressed_after_rename_via_source_merchant_key() {
    let (_dir, worker) = worker();
    let checking = AccountId::new();
    create_role_account(&worker, checking, "Checking", CashflowRole::LiquidCash);
    for month in 1..=6u32 {
        record_with_counterparty(
            &worker,
            checking,
            -1_099,
            NaiveDate::from_ymd_opt(2026, month, 5).unwrap(),
            "SPOTIFY",
        );
    }
    assert_eq!(
        worker.recurring_candidates().unwrap()[0]
            .candidate
            .merchant_key,
        "SPOTIFY"
    );

    // Promote it, but give the bill an UNRELATED name (as if the user renamed it on
    // promotion) while persisting the candidate's merchant key.
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateRecurringBill {
                event_id: RecurringEventId::new(),
                contract_id: BillContractId::new(),
                name: "My Music Thing".to_owned(),
                amount: Money::new(1_099, Currency::Usd),
                bill_type: "subscription".to_owned(),
                frequency: Frequency::Monthly,
                anchor: NaiveDate::from_ymd_opt(2026, 7, 5).unwrap(),
                autopay_account_id: Some(checking),
                description: None,
                source_merchant_key: Some("SPOTIFY".to_owned()),
                category_id: None,
                tag_ids: Vec::new(),
            },
        )
        .unwrap();

    // The name "My Music Thing" does NOT match "SPOTIFY", so only the durable
    // source-merchant-key link keeps the suggestion suppressed.
    assert!(
        worker.recurring_candidates().unwrap().is_empty(),
        "a promoted-then-renamed bill still suppresses its merchant's suggestion",
    );
}

/// 4d8.24.6 (ADR 0046): dismissing a recurring suggestion suppresses it; it re-surfaces only
/// when the pattern materially changes — the amount moves outside the detector's band around
/// the dismissed amount, or the inferred cadence differs. Latest dismiss wins.
#[test]
fn dismissing_a_recurring_suggestion_suppresses_it_until_a_material_change() {
    let (_dir, worker) = worker();
    let checking = AccountId::new();
    create_role_account(&worker, checking, "Checking", CashflowRole::LiquidCash);
    for month in 1..=6u32 {
        record_with_counterparty(
            &worker,
            checking,
            -1_099,
            NaiveDate::from_ymd_opt(2026, month, 5).unwrap(),
            "SPOTIFY",
        );
    }
    let candidates = worker.recurring_candidates().unwrap();
    assert_eq!(candidates[0].candidate.merchant_key, "SPOTIFY");
    assert_eq!(candidates[0].candidate.amount_minor, 1_099);
    assert_eq!(candidates[0].candidate.frequency, "monthly");

    let dismiss = |amount_minor: i64, frequency: &str| {
        worker
            .dispatch(
                meta(),
                WriteCommand::DismissRecurringSuggestion {
                    merchant_key: "SPOTIFY".to_owned(),
                    currency: "USD".to_owned(),
                    amount_minor,
                    frequency: frequency.to_owned(),
                    reason: None,
                },
            )
            .unwrap();
    };

    // Dismissed at the detected pattern → suppressed.
    dismiss(1_099, "monthly");
    assert!(
        worker.recurring_candidates().unwrap().is_empty(),
        "a dismissed suggestion is suppressed"
    );

    // A within-band amount (±10%, floor $5) keeps it suppressed (latest dismiss wins).
    dismiss(1_050, "monthly");
    assert!(
        worker.recurring_candidates().unwrap().is_empty(),
        "a within-band dismissed amount stays suppressed"
    );

    // The current amount is far outside the band around the (now low) dismissed amount →
    // the pattern materially changed → re-surfaces.
    dismiss(100, "monthly");
    assert_eq!(
        worker.recurring_candidates().unwrap().len(),
        1,
        "an out-of-band amount change re-surfaces the suggestion"
    );

    // A dismissed cadence different from the detected one → re-surfaces.
    dismiss(1_099, "weekly");
    assert_eq!(
        worker.recurring_candidates().unwrap().len(),
        1,
        "a cadence change re-surfaces the suggestion"
    );
}

/// 98ql (review): outflows the user has categorized as a transfer (a savings sweep, card
/// autopay) are not surfaced as recurring-bill candidates even when they recur.
#[test]
fn recurring_candidates_excludes_transfer_categorized_outflows() {
    let (_dir, worker) = worker();
    let checking = AccountId::new();
    create_role_account(&worker, checking, "Checking", CashflowRole::LiquidCash);
    let mut txns = Vec::new();
    for month in 1..=4u32 {
        txns.push(record_with_counterparty(
            &worker,
            checking,
            -50_000,
            NaiveDate::from_ymd_opt(2026, month, 3).unwrap(),
            "ONLINE TRANSFER TO SAVINGS",
        ));
    }
    let transfer_cat: Uuid = worker
        .read_connection()
        .unwrap()
        .query_row(
            "SELECT id FROM categories WHERE name = 'Internal Transfer'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    for txn in &txns {
        worker
            .dispatch(
                meta(),
                WriteCommand::RecategorizeTransaction {
                    transaction_id: *txn,
                    category_id: Some(CategoryId::from_uuid(transfer_cat)),
                },
            )
            .unwrap();
    }

    assert!(
        worker.recurring_candidates().unwrap().is_empty(),
        "transfer-categorized outflows are not suggested as bills",
    );
}

/// personal-cfo-4d8.24.5: a candidate carries the dominant (most-common) category across its
/// observed charges + the amount range + last-seen; an uncategorized merchant reports no
/// dominant category.
#[test]
fn recurring_candidate_carries_metadata_and_dominant_category() {
    let (_dir, worker) = worker();
    let checking = AccountId::new();
    create_role_account(&worker, checking, "Checking", CashflowRole::LiquidCash);
    let category = |name: &str| -> Uuid {
        worker
            .read_connection()
            .unwrap()
            .query_row("SELECT id FROM categories WHERE name = ?1", [name], |r| {
                r.get(0)
            })
            .unwrap()
    };
    let coffee = category("Coffee");
    let gas = category("Gas");

    // Six monthly SPOTIFY charges; categorize 4 to Coffee, 2 to Gas → Coffee is dominant.
    let mut spotify = Vec::new();
    for month in 1..=6u32 {
        spotify.push(record_with_counterparty(
            &worker,
            checking,
            -1_099,
            NaiveDate::from_ymd_opt(2026, month, 5).unwrap(),
            "SPOTIFY",
        ));
    }
    for (i, txn) in spotify.iter().enumerate() {
        let cat = if i < 4 { coffee } else { gas };
        worker
            .dispatch(
                meta(),
                WriteCommand::RecategorizeTransaction {
                    transaction_id: *txn,
                    category_id: Some(CategoryId::from_uuid(cat)),
                },
            )
            .unwrap();
    }
    // A second recurring merchant with no categorization at all.
    for month in 1..=6u32 {
        record_with_counterparty(
            &worker,
            checking,
            -1_599,
            NaiveDate::from_ymd_opt(2026, month, 12).unwrap(),
            "NETFLIX",
        );
    }

    let candidates = worker.recurring_candidates().unwrap();
    let spotify_c = candidates
        .iter()
        .find(|c| c.candidate.merchant_key == "SPOTIFY")
        .expect("spotify candidate");
    assert_eq!(
        spotify_c.dominant_category_id,
        Some(coffee),
        "the most-common category wins (4 Coffee vs 2 Gas)",
    );
    assert_eq!(spotify_c.candidate.amount_min_minor, 1_099);
    assert_eq!(spotify_c.candidate.amount_max_minor, 1_099);
    assert_eq!(
        spotify_c.candidate.last_seen,
        NaiveDate::from_ymd_opt(2026, 6, 5).unwrap(),
    );

    let netflix_c = candidates
        .iter()
        .find(|c| c.candidate.merchant_key == "NETFLIX")
        .expect("netflix candidate");
    assert_eq!(
        netflix_c.dominant_category_id, None,
        "an uncategorized merchant has no dominant category",
    );
}

/// 7yh0 merchant memory: the user's manual category for a merchant fills that merchant's
/// other uncategorized transactions — and never overwrites a user assignment.
#[test]
fn merchant_memory_fills_same_merchant_uncategorized_transactions() {
    let (_dir, worker) = worker();
    let account_id = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(account_id)),
                opening_balance: Some(Money::new(5_000_000, Currency::Usd)),
            },
        )
        .unwrap();
    let category = variable_category(&worker);

    // Three visits to the same coffee shop; the raw strings differ only by auth code.
    let d = |day| NaiveDate::from_ymd_opt(2026, 1, day).unwrap();
    let t1 = record_with_counterparty(&worker, account_id, -1200, d(5), "SQ *BLUE BOTTLE *A1B2C3");
    let t2 = record_with_counterparty(&worker, account_id, -1500, d(12), "SQ *BLUE BOTTLE *D4E5F6");
    let t3 = record_with_counterparty(&worker, account_id, -1100, d(20), "SQ *BLUE BOTTLE *G7H8I9");

    // The user categorizes one of them.
    worker
        .dispatch(
            meta(),
            WriteCommand::RecategorizeTransaction {
                transaction_id: t1,
                category_id: Some(category),
            },
        )
        .unwrap();

    // Apply → the other two (same normalized merchant) are filled as source='rule'.
    assert_eq!(worker.apply_merchant_memory().unwrap(), 2);
    assert_eq!(
        category_of(&worker, t2),
        Some((category.as_uuid(), "rule".to_owned()))
    );
    assert_eq!(
        category_of(&worker, t3),
        Some((category.as_uuid(), "rule".to_owned()))
    );
    // The trained one keeps its user assignment (no silent re-tag).
    assert_eq!(
        category_of(&worker, t1),
        Some((category.as_uuid(), "user".to_owned()))
    );

    // Provenance surfaces on the transactions list (personal-cfo-5n4.1): the filled
    // rows carry source='rule' + the agreement ratio as confidence; the trained one
    // reads source='user'. This is what the row's provenance badge renders.
    let rows = worker.recent_transactions(50).unwrap();
    let row_of = |id: TransactionId| rows.iter().find(|r| r.transaction_id == id).unwrap();
    assert_eq!(row_of(t2).category_source.as_deref(), Some("rule"));
    assert_eq!(row_of(t2).category_confidence_bps, Some(10_000));
    assert_eq!(row_of(t3).category_source.as_deref(), Some("rule"));
    assert_eq!(row_of(t1).category_source.as_deref(), Some("user"));

    // Idempotent: re-running categorizes nothing new.
    assert_eq!(worker.apply_merchant_memory().unwrap(), 0);
}

/// A merchant the user has categorized inconsistently (50/50) is below the agreement
/// threshold, so merchant memory leaves its other transactions for review.
#[test]
fn merchant_memory_skips_conflicted_merchants() {
    let (_dir, worker) = worker();
    let account_id = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(account_id)),
                opening_balance: Some(Money::new(5_000_000, Currency::Usd)),
            },
        )
        .unwrap();
    let cats: Vec<Uuid> = {
        let conn = worker.read_connection().unwrap();
        let mut stmt = conn
            .prepare("SELECT id FROM categories WHERE forecast_behavior='variable_regular' LIMIT 2")
            .unwrap();
        let rows = stmt.query_map([], |r| r.get::<_, Uuid>(0)).unwrap();
        rows.map(Result::unwrap).collect()
    };
    assert!(cats.len() >= 2, "need two seeded categories");

    let d = |day| NaiveDate::from_ymd_opt(2026, 2, day).unwrap();
    let t1 = record_with_counterparty(&worker, account_id, -2000, d(3), "TARGET STORE #11");
    let t2 = record_with_counterparty(&worker, account_id, -3000, d(9), "TARGET STORE #12");
    let t3 = record_with_counterparty(&worker, account_id, -2500, d(15), "TARGET STORE #13");
    // Categorized two different ways → 50/50 conflict.
    worker
        .dispatch(
            meta(),
            WriteCommand::RecategorizeTransaction {
                transaction_id: t1,
                category_id: Some(CategoryId::from_uuid(cats[0])),
            },
        )
        .unwrap();
    worker
        .dispatch(
            meta(),
            WriteCommand::RecategorizeTransaction {
                transaction_id: t2,
                category_id: Some(CategoryId::from_uuid(cats[1])),
            },
        )
        .unwrap();

    assert_eq!(worker.apply_merchant_memory().unwrap(), 0);
    assert_eq!(
        category_of(&worker, t3),
        None,
        "conflicted merchant left for review"
    );
}

/// 7yh0-v2 (personal-cfo-5n4.3): a category learned under one alias fills uncategorized
/// transactions under a DIFFERENT alias of the same merchant identity — cross-alias
/// grouping that v1's per-string key could not do.
#[test]
fn merchant_memory_groups_by_identity_across_aliases() {
    use categorization::normalize_merchant;
    let (_dir, worker) = worker();
    let account_id = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(account_id)),
                opening_balance: Some(Money::new(5_000_000, Currency::Usd)),
            },
        )
        .unwrap();
    let category = variable_category(&worker);

    // One non-ambiguous Spotify identity with two distinct normalized-key aliases.
    let spotify = worker
        .create_merchant_identity("Spotify", None, false, "seed")
        .unwrap();
    worker
        .link_merchant_alias(&normalize_merchant("SPOTIFY"), spotify, "seed", 10_000)
        .unwrap();
    worker
        .link_merchant_alias(&normalize_merchant("SPOTIFY USA"), spotify, "seed", 10_000)
        .unwrap();

    let d = |day| NaiveDate::from_ymd_opt(2026, 2, day).unwrap();
    let learned = record_with_counterparty(&worker, account_id, -1099, d(3), "SPOTIFY");
    let other = record_with_counterparty(&worker, account_id, -1099, d(10), "SPOTIFY USA");

    worker
        .dispatch(
            meta(),
            WriteCommand::RecategorizeTransaction {
                transaction_id: learned,
                category_id: Some(category),
            },
        )
        .unwrap();

    // The two raw strings normalize differently, but resolve to the one identity → filled.
    assert_eq!(worker.apply_merchant_memory().unwrap(), 1);
    assert_eq!(
        category_of(&worker, other),
        Some((category.as_uuid(), "rule".to_owned())),
        "the other alias of the same identity is filled"
    );
}

/// 7yh0-v2 (personal-cfo-5n4.3): an AMBIGUOUS identity (Amazon spans categories) is never
/// auto-filled, even with a clear learned category — it defers to disambiguation.
#[test]
fn merchant_memory_skips_ambiguous_identities() {
    use categorization::normalize_merchant;
    let (_dir, worker) = worker();
    let account_id = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(account_id)),
                opening_balance: Some(Money::new(5_000_000, Currency::Usd)),
            },
        )
        .unwrap();
    let category = variable_category(&worker);

    let amazon = worker
        .create_merchant_identity("Amazon", None, true, "seed")
        .unwrap();
    worker
        .link_merchant_alias(&normalize_merchant("AMAZON.COM"), amazon, "seed", 10_000)
        .unwrap();
    worker
        .link_merchant_alias(&normalize_merchant("AMZN MKTP US"), amazon, "seed", 10_000)
        .unwrap();

    let d = |day| NaiveDate::from_ymd_opt(2026, 2, day).unwrap();
    let learned = record_with_counterparty(&worker, account_id, -2000, d(3), "AMAZON.COM");
    let other = record_with_counterparty(&worker, account_id, -4500, d(10), "AMZN MKTP US");
    worker
        .dispatch(
            meta(),
            WriteCommand::RecategorizeTransaction {
                transaction_id: learned,
                category_id: Some(category),
            },
        )
        .unwrap();

    // Ambiguous identity → not propagated; the other Amazon transaction stays open.
    assert_eq!(worker.apply_merchant_memory().unwrap(), 0);
    assert_eq!(
        category_of(&worker, other),
        None,
        "ambiguous merchant defers to disambiguation"
    );
}

#[test]
fn merchant_identity_resolve_groups_aliases_to_one_entity() {
    // The point of the entity layer (ADR 0030 addendum): two raw-string keys for the
    // same merchant resolve to one identity, so a category learned for one applies to both.
    // Uses a made-up merchant so it stays isolated from the seeded canonical merchants.
    let (_dir, worker) = worker();
    let before = worker.merchant_identity_count().unwrap();
    let contoso = worker
        .create_merchant_identity("Contoso", None, true, "user")
        .unwrap();
    worker
        .link_merchant_alias("CONTOSO HQ", contoso, "user", 10_000)
        .unwrap();
    worker
        .link_merchant_alias("CONTOSO STORE 42", contoso, "user", 10_000)
        .unwrap();

    let a = worker
        .resolve_merchant_identity("CONTOSO HQ")
        .unwrap()
        .unwrap();
    let b = worker
        .resolve_merchant_identity("CONTOSO STORE 42")
        .unwrap()
        .unwrap();
    assert_eq!(a.id, contoso);
    assert_eq!(b.id, contoso, "both aliases resolve to the one identity");
    assert_eq!(a.display_name, "Contoso");
    assert!(a.is_ambiguous);
    assert_eq!(
        worker.merchant_identity_count().unwrap(),
        before + 1,
        "exactly one new identity beyond the seeded merchants"
    );
}

#[test]
fn merchant_identity_resolve_unknown_key_is_none() {
    let (_dir, worker) = worker();
    assert!(worker
        .resolve_merchant_identity("NEVER SEEN")
        .unwrap()
        .is_none());
}

#[test]
fn merchant_identity_persists_default_category_and_ambiguity() {
    // An unambiguous merchant carries a default category (the auto-cat seam).
    let (_dir, worker) = worker();
    let category = variable_category(&worker);
    let starbucks = worker
        .create_merchant_identity("Starbucks", Some(category), false, "seed")
        .unwrap();
    worker
        .link_merchant_alias("STARBUCKS", starbucks, "seed", 10_000)
        .unwrap();

    let resolved = worker
        .resolve_merchant_identity("STARBUCKS")
        .unwrap()
        .unwrap();
    assert!(!resolved.is_ambiguous);
    assert_eq!(resolved.default_category_id, Some(category.as_uuid()));
    assert_eq!(resolved.source, "seed");
}

#[test]
fn merchant_alias_relink_moves_the_key() {
    // The normalized key is the PK — re-linking moves it to the new identity.
    let (_dir, worker) = worker();
    let before = worker.merchant_identity_count().unwrap();
    let a = worker
        .create_merchant_identity("Merchant A", None, false, "user")
        .unwrap();
    let b = worker
        .create_merchant_identity("Merchant B", None, false, "user")
        .unwrap();
    worker
        .link_merchant_alias("KEY", a, "user", 10_000)
        .unwrap();
    worker.link_merchant_alias("KEY", b, "auto", 8_000).unwrap();
    let resolved = worker.resolve_merchant_identity("KEY").unwrap().unwrap();
    assert_eq!(
        resolved.id, b,
        "re-linking a key moves it to the new identity"
    );
    assert_eq!(worker.merchant_identity_count().unwrap(), before + 2);
}

/// 6p6b: Amazon is seeded at vault init as an ambiguous identity, so a real Amazon
/// descriptor resolves to it (the normalized key matches the seeded alias).
#[test]
fn seed_flags_amazon_as_ambiguous() {
    use categorization::normalize_merchant;
    let (_dir, worker) = worker();
    let resolved = worker
        .resolve_merchant_identity(&normalize_merchant("AMZN MKTP US*A1B2C3"))
        .unwrap()
        .expect("a real Amazon descriptor resolves to the seeded identity");
    assert_eq!(resolved.display_name, "Amazon");
    assert!(resolved.is_ambiguous, "Amazon is seeded ambiguous");
    assert_eq!(resolved.source, "seed");
}

/// 6p6b acceptance fixture: an Amazon transaction stream spanning multiple categories is
/// NOT auto-categorized by merchant memory — the seeded ambiguity guard prevents one
/// learned category from mis-tagging the rest (it defers to disambiguation).
#[test]
fn seeded_amazon_stream_is_not_auto_categorized() {
    let (_dir, worker) = worker();
    let account_id = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(account_id)),
                opening_balance: Some(Money::new(5_000_000, Currency::Usd)),
            },
        )
        .unwrap();
    let category = variable_category(&worker);

    // A realistic Amazon stream: different raw strings, varied amounts (a subscription, a
    // big shopping order, a small one, a refund). All resolve to the seeded Amazon.
    let d = |day| NaiveDate::from_ymd_opt(2026, 3, day).unwrap();
    let learned = record_with_counterparty(&worker, account_id, -1499, d(2), "AMZN MKTP US*RT01");
    let big = record_with_counterparty(&worker, account_id, -34_800, d(7), "AMAZON.COM*RT02");
    let small = record_with_counterparty(&worker, account_id, -899, d(11), "AMZN MKTP US*RT03");

    // The user categorizes one Amazon charge.
    worker
        .dispatch(
            meta(),
            WriteCommand::RecategorizeTransaction {
                transaction_id: learned,
                category_id: Some(category),
            },
        )
        .unwrap();

    // Ambiguity guard: the learned category does NOT propagate to the other Amazon rows.
    assert_eq!(worker.apply_merchant_memory().unwrap(), 0);
    assert_eq!(category_of(&worker, big), None);
    assert_eq!(category_of(&worker, small), None);
}

/// 2a6r: a physical chain (Costco) resolves to its one ambiguous identity by brand PREFIX
/// even though the normalized key keeps the store city — across different locations.
#[test]
fn seed_resolves_costco_across_locations_by_prefix() {
    use categorization::normalize_merchant;
    let (_dir, worker) = worker();
    let seattle = worker
        .resolve_merchant_identity(&normalize_merchant("COSTCO WHSE #0455 SEATTLE WA"))
        .unwrap()
        .expect("Seattle Costco resolves");
    let portland = worker
        .resolve_merchant_identity(&normalize_merchant("COSTCO GAS #0612 PORTLAND OR"))
        .unwrap()
        .expect("Portland Costco resolves");
    assert_eq!(
        seattle.id, portland.id,
        "both locations are one Costco identity"
    );
    assert_eq!(seattle.display_name, "Costco");
    assert!(seattle.is_ambiguous);
}

/// 9rtv: Apple resolves both its online billing (exact alias) and its physical stores
/// (brand prefix) to one ambiguous identity.
#[test]
fn seed_resolves_apple_online_and_store() {
    use categorization::normalize_merchant;
    let (_dir, worker) = worker();
    let online = worker
        .resolve_merchant_identity(&normalize_merchant("APPLE.COM/BILL"))
        .unwrap()
        .expect("Apple online billing resolves");
    let store = worker
        .resolve_merchant_identity(&normalize_merchant("APPLE STORE R102 PALO ALTO CA"))
        .unwrap()
        .expect("Apple store resolves by prefix");
    assert_eq!(online.id, store.id);
    assert_eq!(online.display_name, "Apple");
    assert!(online.is_ambiguous);
}

/// 2a6r false-merge guard: the brand-prefix match respects a word boundary, so the
/// `APPLE` prefix never swallows `APPLEBEE'S` — which categorizes normally.
#[test]
fn seed_prefix_respects_word_boundary() {
    use categorization::normalize_merchant;
    let (_dir, worker) = worker();
    // Applebee's is NOT Apple — it must not resolve to the Apple identity.
    assert!(
        worker
            .resolve_merchant_identity(&normalize_merchant("APPLEBEE'S 0123 DENVER CO"))
            .unwrap()
            .is_none(),
        "APPLE prefix must not match APPLEBEE'S"
    );

    // ...and so merchant memory categorizes Applebee's normally (not suppressed).
    let account_id = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(account_id)),
                opening_balance: Some(Money::new(5_000_000, Currency::Usd)),
            },
        )
        .unwrap();
    let category = variable_category(&worker);
    let d = |day| NaiveDate::from_ymd_opt(2026, 4, day).unwrap();
    let a = record_with_counterparty(
        &worker,
        account_id,
        -3200,
        d(2),
        "APPLEBEE'S 0123 DENVER CO",
    );
    let b = record_with_counterparty(
        &worker,
        account_id,
        -2800,
        d(9),
        "APPLEBEE'S 0123 DENVER CO",
    );
    worker
        .dispatch(
            meta(),
            WriteCommand::RecategorizeTransaction {
                transaction_id: a,
                category_id: Some(category),
            },
        )
        .unwrap();
    assert_eq!(worker.apply_merchant_memory().unwrap(), 1);
    assert_eq!(
        category_of(&worker, b),
        Some((category.as_uuid(), "rule".to_owned())),
        "Applebee's categorizes normally — the prefix guard did not suppress it"
    );
}

/// x75h: a bare P2P descriptor with no counterparty (just "VENMO") is flagged ambiguous.
#[test]
fn seed_flags_bare_venmo_ambiguous() {
    use categorization::normalize_merchant;
    let (_dir, worker) = worker();
    let resolved = worker
        .resolve_merchant_identity(&normalize_merchant("VENMO"))
        .unwrap()
        .expect("a bare Venmo descriptor resolves to the seeded identity");
    assert_eq!(resolved.display_name, "Venmo");
    assert!(resolved.is_ambiguous);
}

#[test]
fn merchant_identity_rejects_out_of_domain_source() {
    // The CHECK constraint guards the provenance token.
    let (_dir, worker) = worker();
    assert!(
        worker
            .create_merchant_identity("Bogus", None, false, "imported")
            .is_err(),
        "source must be seed|user|auto"
    );
}

/// 5n4.4: an unseeded chain seen at two cities is minted as one auto identity, and a
/// category learned at one location then fills the other (cross-location grouping).
#[test]
fn merchant_grouping_mints_unseeded_chain_and_groups_learning() {
    use categorization::normalize_merchant;
    let (_dir, worker) = worker();
    let account_id = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(account_id)),
                opening_balance: Some(Money::new(5_000_000, Currency::Usd)),
            },
        )
        .unwrap();
    let category = variable_category(&worker);

    let d = |day| NaiveDate::from_ymd_opt(2026, 5, day).unwrap();
    let sf = record_with_counterparty(
        &worker,
        account_id,
        -650,
        d(3),
        "STARBUCKS STORE 123 SAN FRANCISCO CA",
    );
    let sea = record_with_counterparty(&worker, account_id, -700, d(10), "STARBUCKS SEATTLE WA");

    assert!(
        worker.apply_merchant_grouping().unwrap() >= 1,
        "Starbucks minted"
    );
    let a = worker
        .resolve_merchant_identity(&normalize_merchant("STARBUCKS STORE 123 SAN FRANCISCO CA"))
        .unwrap()
        .expect("SF Starbucks resolves to the minted anchor");
    let b = worker
        .resolve_merchant_identity(&normalize_merchant("STARBUCKS SEATTLE WA"))
        .unwrap()
        .expect("Seattle Starbucks resolves");
    assert_eq!(
        a.id, b.id,
        "both locations are one minted Starbucks identity"
    );
    assert_eq!(a.display_name, "Starbucks");
    assert_eq!(a.source, "auto");
    assert!(!a.is_ambiguous);

    // A category learned at SF fills the Seattle location across the minted anchor.
    worker
        .dispatch(
            meta(),
            WriteCommand::RecategorizeTransaction {
                transaction_id: sf,
                category_id: Some(category),
            },
        )
        .unwrap();
    assert_eq!(worker.apply_merchant_memory().unwrap(), 1);
    assert_eq!(
        category_of(&worker, sea),
        Some((category.as_uuid(), "rule".to_owned())),
    );
}

/// 5n4.4 precision: a generic industry noun (PIZZA) at two cities is two different shops —
/// it must NOT mint an anchor (the false-merge trap the design pass targeted).
#[test]
fn merchant_grouping_does_not_mint_generic_industry_brand() {
    use categorization::normalize_merchant;
    let (_dir, worker) = worker();
    let account_id = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(account_id)),
                opening_balance: Some(Money::new(5_000_000, Currency::Usd)),
            },
        )
        .unwrap();
    let d = |day| NaiveDate::from_ymd_opt(2026, 5, day).unwrap();
    record_with_counterparty(&worker, account_id, -1800, d(2), "PIZZA OAKLAND CA");
    record_with_counterparty(&worker, account_id, -2200, d(9), "PIZZA DENVER CO");

    let before = worker.merchant_identity_count().unwrap();
    assert_eq!(
        worker.apply_merchant_grouping().unwrap(),
        0,
        "PIZZA must not mint"
    );
    assert_eq!(worker.merchant_identity_count().unwrap(), before);
    assert!(worker
        .resolve_merchant_identity(&normalize_merchant("PIZZA OAKLAND CA"))
        .unwrap()
        .is_none());
}

/// 5n4.4: the minting pass is idempotent — a second run mints nothing new.
#[test]
fn merchant_grouping_is_idempotent() {
    let (_dir, worker) = worker();
    let account_id = AccountId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(checking(account_id)),
                opening_balance: Some(Money::new(5_000_000, Currency::Usd)),
            },
        )
        .unwrap();
    let d = |day| NaiveDate::from_ymd_opt(2026, 5, day).unwrap();
    record_with_counterparty(&worker, account_id, -650, d(3), "SHELL OIL HOUSTON TX");
    record_with_counterparty(&worker, account_id, -700, d(10), "SHELL OIL DALLAS TX");

    assert!(worker.apply_merchant_grouping().unwrap() >= 1);
    let count = worker.merchant_identity_count().unwrap();
    assert_eq!(
        worker.apply_merchant_grouping().unwrap(),
        0,
        "re-run mints nothing"
    );
    assert_eq!(worker.merchant_identity_count().unwrap(), count);
}

/// ADR 0047 §2.3 (personal-cfo-4d8.25.9): a tracked bill consumes candidate spellings
/// that are truncation variants of its name — the owner's "approving doesn't stop the
/// recommendations" symptom. The imported spelling normalizes to a LONGER key than the
/// bill's name; exact-key exclusion misses it, the variant rule must not.
#[test]
fn approved_series_suppresses_merchant_key_variants() {
    let (_dir, worker) = worker();
    let checking = AccountId::new();
    create_role_account(&worker, checking, "Checking", CashflowRole::LiquidCash);
    // Bank-truncated import spelling: "SQ *" strips, leaving "SEVEN SEAS ROASTING C".
    for month in 1..=6u32 {
        record_with_counterparty(
            &worker,
            checking,
            -1_770,
            NaiveDate::from_ymd_opt(2026, month, 16).unwrap(),
            "SQ *SEVEN SEAS ROASTING C",
        );
    }
    assert_eq!(worker.recurring_candidates().unwrap().len(), 1);

    // The user names the bill the un-truncated way — a DIFFERENT normalized key.
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateRecurringBill {
                event_id: RecurringEventId::new(),
                contract_id: BillContractId::new(),
                name: "Seven Seas Roasting".to_owned(),
                amount: Money::new(1_770, Currency::Usd),
                bill_type: "subscription".to_owned(),
                frequency: Frequency::Monthly,
                anchor: NaiveDate::from_ymd_opt(2026, 6, 16).unwrap(),
                autopay_account_id: Some(checking),
                description: None,
                source_merchant_key: None,
                category_id: None,
                tag_ids: Vec::new(),
            },
        )
        .unwrap();
    assert!(
        worker.recurring_candidates().unwrap().is_empty(),
        "a truncation variant of the tracked name must not resurface"
    );
}

/// ADR 0047 §1 (personal-cfo-4d8.25.8): approving a bill retro-attaches its historical
/// card postings (the account gate admits them) and the linked spellings consume the
/// candidate via EVIDENCE keys even when the bill name shares nothing with the imports.
#[test]
fn retro_attach_links_card_history_and_evidence_keys_consume_the_series() {
    let (_dir, worker) = worker();
    let card = AccountId::new();
    create_role_account(&worker, card, "Venture X", CashflowRole::CreditFacility);
    for month in 1..=6u32 {
        record_with_counterparty(
            &worker,
            card,
            -2_199,
            NaiveDate::from_ymd_opt(2026, month, 19).unwrap(),
            "ACME STREAMING",
        );
    }
    assert_eq!(worker.recurring_candidates().unwrap().len(), 1);

    // Promote with an unrelated display name, per the ADR 0047 §3 anchor (last observed
    // occurrence), charged to the card. No source_merchant_key on purpose: only the
    // evidence keys of the retro-attached postings can consume the candidate.
    let event_id = RecurringEventId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateRecurringBill {
                event_id,
                contract_id: BillContractId::new(),
                name: "Acme TV".to_owned(),
                amount: Money::new(2_199, Currency::Usd),
                bill_type: "subscription".to_owned(),
                frequency: Frequency::Monthly,
                anchor: NaiveDate::from_ymd_opt(2026, 6, 19).unwrap(),
                autopay_account_id: Some(card),
                description: None,
                source_merchant_key: None,
                category_id: None,
                tag_ids: Vec::new(),
            },
        )
        .unwrap();

    // The retro-attach surface: every historical occurrence linked to its card posting.
    let history = worker.recurring_bill_history(event_id).unwrap();
    let linked: Vec<_> = history
        .iter()
        .filter(|row| row.linked_transaction_id.is_some())
        .collect();
    assert_eq!(
        linked.len(),
        6,
        "all six historical card charges match the schedule: {history:?}"
    );
    assert!(linked.iter().all(|row| row.status == "paid"));

    // Evidence-key consumption: the imported spelling normalizes to ACME STREAMING while
    // the bill is named Acme TV — only the linked-transaction keys can suppress it.
    assert!(
        worker.recurring_candidates().unwrap().is_empty(),
        "linked-posting evidence keys consume the series"
    );
}

/// ADR 0047 §1: the account gate — an un-gated liquid bill must NOT retro-attach card
/// postings, and a card-gated bill must not link liquid postings from other accounts.
#[test]
fn retro_attach_respects_the_account_gate() {
    let (_dir, worker) = worker();
    let checking = AccountId::new();
    let card = AccountId::new();
    create_role_account(&worker, checking, "Checking", CashflowRole::LiquidCash);
    create_role_account(&worker, card, "Card", CashflowRole::CreditFacility);
    for month in 1..=4u32 {
        record_with_counterparty(
            &worker,
            card,
            -5_000,
            NaiveDate::from_ymd_opt(2026, month, 7).unwrap(),
            "GYMCO MEMBERSHIP",
        );
    }
    // Same cadence + amount, but the bill pays from CHECKING: the card history must not
    // attach (those postings belong to the card's own series, not this bill).
    let event_id = RecurringEventId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateRecurringBill {
                event_id,
                contract_id: BillContractId::new(),
                name: "Gymco".to_owned(),
                amount: Money::new(5_000, Currency::Usd),
                bill_type: "membership".to_owned(),
                frequency: Frequency::Monthly,
                anchor: NaiveDate::from_ymd_opt(2026, 4, 7).unwrap(),
                autopay_account_id: Some(checking),
                description: None,
                source_merchant_key: None,
                category_id: None,
                tag_ids: Vec::new(),
            },
        )
        .unwrap();
    let history = worker.recurring_bill_history(event_id).unwrap();
    assert!(
        history
            .iter()
            .all(|row| row.linked_transaction_id.is_none()),
        "a checking-paid bill never attaches card postings: {history:?}"
    );
}

/// Adversarial review of 4d8.25.9 (evidence-key trust): the matcher's $5 amount floor can
/// systematically mis-link an un-gated bill to an UNRELATED small subscription at a stable
/// monthly offset. Those links must never be harvested as merchant-identity evidence —
/// the unrelated merchant stays a visible candidate.
#[test]
fn amount_floor_mislinks_never_consume_an_unrelated_merchants_candidate() {
    let (_dir, worker) = worker();
    let checking = AccountId::new();
    create_role_account(&worker, checking, "Checking", CashflowRole::LiquidCash);
    // The genuine, distinct subscription: HULU $12.99 monthly on the 3rd.
    for month in 1..=6u32 {
        record_with_counterparty(
            &worker,
            checking,
            -1_299,
            NaiveDate::from_ymd_opt(2026, month, 3).unwrap(),
            "HULU",
        );
    }
    // An un-gated $9.99 bill due the 1st whose real charges are NOT in the ledger (paid
    // on an untracked card): its occurrences amount-link the HULU postings — a $3.00
    // difference is inside the matcher's $5.00 floor but far outside the 5% band.
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateRecurringBill {
                event_id: RecurringEventId::new(),
                contract_id: BillContractId::new(),
                name: "Netflix".to_owned(),
                amount: Money::new(999, Currency::Usd),
                bill_type: "subscription".to_owned(),
                frequency: Frequency::Monthly,
                anchor: NaiveDate::from_ymd_opt(2026, 6, 1).unwrap(),
                autopay_account_id: None,
                description: None,
                source_merchant_key: None,
                category_id: None,
                tag_ids: Vec::new(),
            },
        )
        .unwrap();
    // Materialize the instance links (the mis-links form here).
    worker.rebuild_recurring_instances().unwrap();

    let candidates = worker.recurring_candidates().unwrap();
    assert!(
        candidates
            .iter()
            .any(|c| c.candidate.merchant_key == "HULU"),
        "an amount-floor coincidence must not consume the HULU candidate: {candidates:?}"
    );
}

/// Adversarial review of 4d8.25.8 (actualization parity): a card-charged bill's realized
/// CARD posting must score exact/matched — not 'missed with realized 0' — now that the
/// linking seam attaches card postings (ADR 0047 s1).
#[test]
fn actualization_scores_a_card_linked_bill_occurrence_as_realized() {
    let (_dir, worker) = worker();
    let checking = AccountId::new();
    let card = AccountId::new();
    create_role_account(&worker, checking, "Checking", CashflowRole::LiquidCash);
    create_role_account(&worker, card, "Card", CashflowRole::CreditFacility);
    let today = chrono::Utc::now().date_naive();
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            checking,
            Money::new(500_000, Currency::Usd),
            today - chrono::Days::new(1),
        )
        .unwrap();
    // A bill charged to the card, due today (anchor = today, monthly).
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateRecurringBill {
                event_id: RecurringEventId::new(),
                contract_id: BillContractId::new(),
                name: "Acme Streaming".to_owned(),
                amount: Money::new(2_199, Currency::Usd),
                bill_type: "subscription".to_owned(),
                frequency: Frequency::Monthly,
                anchor: today,
                autopay_account_id: Some(card),
                description: None,
                source_merchant_key: None,
                category_id: None,
                tag_ids: Vec::new(),
            },
        )
        .unwrap();
    // Predict it (persist today's run), then realize it as a CARD charge on the due date.
    worker.persist_daily_forecast().unwrap();
    record_with_counterparty(&worker, card, -2_199, today, "ACME STREAMING");
    worker.actualize_forecasts().unwrap();

    let conn = worker.read_connection().unwrap();
    let statuses: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT match_status FROM forecast_actuals")
            .unwrap();
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        rows
    };
    assert!(
        statuses.iter().any(|s| s == "exact" || s == "matched"),
        "the card-linked occurrence must score as realized, got {statuses:?}"
    );
    assert!(
        !statuses.iter().any(|s| s == "missed"),
        "a linked card posting must not be scored missed: {statuses:?}"
    );
}

/// Candidate provenance (personal-cfo-4d8.25.11, ADR 0047 §4): the driver carries the
/// observed accounts (most-frequent first), the single-account prefill id, the typical
/// day-of-month for month-stepped cadences, and the capped observation proof rows.
#[test]
fn recurring_candidate_carries_account_provenance_and_proof() {
    let (_dir, worker) = worker();
    let card = AccountId::new();
    let checking = AccountId::new();
    create_role_account(&worker, card, "Venture X", CashflowRole::CreditFacility);
    create_role_account(&worker, checking, "Checking", CashflowRole::LiquidCash);

    // SPOTIFY: six monthly charges around the 19th, ALL on the card → single-account
    // series, prefillable; day drifts 18/19/20 with median 19.
    for (month, day) in [(1u32, 19u32), (2, 18), (3, 19), (4, 20), (5, 19), (6, 19)] {
        record_with_counterparty(
            &worker,
            card,
            -2_199,
            NaiveDate::from_ymd_opt(2026, month, day).unwrap(),
            "SPOTIFY",
        );
    }
    // EQUINOX: monthly, split 4 on checking / 2 on the card → multi-account, no prefill,
    // checking listed first (most frequent).
    for month in 1..=6u32 {
        let account = if month <= 4 { checking } else { card };
        record_with_counterparty(
            &worker,
            account,
            -4_500,
            NaiveDate::from_ymd_opt(2026, month, 3).unwrap(),
            "EQUINOX",
        );
    }

    let candidates = worker.recurring_candidates().unwrap();
    let spotify = candidates
        .iter()
        .find(|c| c.candidate.merchant_key == "SPOTIFY")
        .expect("spotify candidate");
    assert_eq!(spotify.source_account_names, vec!["Venture X".to_owned()]);
    assert_eq!(
        spotify.source_account_id,
        Some(card.as_uuid()),
        "a single-account series prefills the pay-from (ADR 0047 s4)"
    );
    assert_eq!(
        spotify.typical_day_of_month,
        Some(19),
        "median observed day"
    );
    assert_eq!(spotify.observations.len(), 6);
    // Most recent first; every row carries the account name.
    assert_eq!(
        spotify.observations[0].date,
        NaiveDate::from_ymd_opt(2026, 6, 19).unwrap()
    );
    assert!(spotify
        .observations
        .iter()
        .all(|o| o.account_name == "Venture X" && o.amount_minor == 2_199));

    let gym = candidates
        .iter()
        .find(|c| c.candidate.merchant_key == "EQUINOX")
        .expect("gym candidate");
    assert_eq!(
        gym.source_account_names,
        vec!["Checking".to_owned(), "Venture X".to_owned()],
        "most-frequent account first"
    );
    assert_eq!(gym.source_account_id, None, "multi-account: no prefill");
}

/// Provenance is keyed per (merchant, currency) exactly like candidate identity
/// (adversarial review of 4d8.25.11): one merchant recurring in two currencies yields
/// two candidates, each with its OWN observations, prefill account, and typical day —
/// never mixed, never emptied.
#[test]
fn multi_currency_merchant_keeps_provenance_per_candidate() {
    let (_dir, worker) = worker();
    let usd_card = AccountId::new();
    let eur_card = AccountId::new();
    create_role_account(&worker, usd_card, "US Card", CashflowRole::CreditFacility);
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(Account::new(
                    eur_card,
                    LedgerAccountId::new(),
                    "EU Card",
                    CashflowRole::CreditFacility,
                    Currency::Eur,
                    AccountFlags::default(),
                )),
                opening_balance: None,
            },
        )
        .unwrap();

    // Six monthly USD charges on the 19th; six monthly EUR charges on the 3rd — one
    // merchant, two currency series (distinct dates so the detail insert finds its row).
    for month in 1..=6u32 {
        record_with_counterparty(
            &worker,
            usd_card,
            -2_199,
            NaiveDate::from_ymd_opt(2026, month, 19).unwrap(),
            "SPOTIFY",
        );
        let date = NaiveDate::from_ymd_opt(2026, month, 3).unwrap();
        worker
            .dispatch(
                meta(),
                WriteCommand::RecordTransaction {
                    transaction_id: TransactionId::new(),
                    account_id: eur_card,
                    amount: Money::new(-1_899, Currency::Eur),
                    occurred_at: date.and_hms_opt(12, 0, 0).unwrap().and_utc(),
                },
            )
            .unwrap();
        let txn = worker
            .recent_transactions(500)
            .unwrap()
            .iter()
            .find(|t| t.occurred_at.date_naive() == date)
            .expect("just-planted eur txn")
            .transaction_id;
        worker
            .read_connection()
            .unwrap()
            .execute(
                "INSERT OR REPLACE INTO transaction_details
                        (transaction_id, memo, counterparty, created_at)
                     VALUES (?1, NULL, ?2, ?3)",
                params![txn.as_uuid(), "SPOTIFY", "2026-01-01T00:00:00Z"],
            )
            .unwrap();
    }

    let candidates = worker.recurring_candidates().unwrap();
    let spotify: Vec<_> = candidates
        .iter()
        .filter(|c| c.candidate.merchant_key == "SPOTIFY")
        .collect();
    assert_eq!(spotify.len(), 2, "one candidate per currency series");
    for view in spotify {
        match view.candidate.currency.as_str() {
            "USD" => {
                assert_eq!(view.source_account_names, vec!["US Card".to_owned()]);
                assert_eq!(view.source_account_id, Some(usd_card.as_uuid()));
                assert_eq!(view.typical_day_of_month, Some(19));
                assert_eq!(view.observations.len(), 6);
                assert!(view
                    .observations
                    .iter()
                    .all(|o| o.amount_minor == 2_199 && o.account_name == "US Card"));
            }
            "EUR" => {
                assert_eq!(view.source_account_names, vec!["EU Card".to_owned()]);
                assert_eq!(view.source_account_id, Some(eur_card.as_uuid()));
                assert_eq!(view.typical_day_of_month, Some(3));
                assert_eq!(view.observations.len(), 6);
                assert!(view
                    .observations
                    .iter()
                    .all(|o| o.amount_minor == 1_899 && o.account_name == "EU Card"));
            }
            other => panic!("unexpected candidate currency {other}"),
        }
    }
}
