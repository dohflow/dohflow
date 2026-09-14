//! Accounts, transactions, transfers, tiers, and balance assertions
//! (moved verbatim from the old in-file `lib.rs` tests module).

mod common;

use chrono::{DateTime, NaiveDate, Utc};
use common::*;
use core_ledger::{
    Account, AccountFlags, AccountId, AccountSubtype, CashflowRole, LedgerAccountId, TagId,
    TransactionId,
};
use core_money::{Currency, Money};
use db_worker::*;
use uuid::Uuid;

#[test]
fn voiding_a_transaction_hides_it_and_nets_the_balance() {
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

    // Record a −$40 expense.
    worker
        .dispatch(
            meta(),
            WriteCommand::RecordTransaction {
                transaction_id: TransactionId::new(),
                account_id,
                amount: Money::new(-4_000, Currency::Usd),
                occurred_at: NaiveDate::from_ymd_opt(2026, 6, 20)
                    .unwrap()
                    .and_hms_opt(0, 0, 0)
                    .unwrap()
                    .and_utc(),
            },
        )
        .unwrap();
    assert_eq!(worker.recent_transactions(50).unwrap().len(), 1);
    assert_eq!(
        worker.account_balance(account_id).unwrap(),
        Some(Money::new(-4_000, Currency::Usd)),
    );
    let txn_id = worker.recent_transactions(50).unwrap()[0].transaction_id;

    // Void it: gone from the list, postings net to zero.
    worker
        .dispatch(
            meta(),
            WriteCommand::VoidTransaction {
                transaction_id: txn_id,
            },
        )
        .unwrap();
    assert!(
        worker.recent_transactions(50).unwrap().is_empty(),
        "the voided transaction (and its reversal) are hidden from the list",
    );

    // The ledger kept both entries (append-only); the user-account postings net out.
    let conn = worker.read_connection().unwrap();
    let txn_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM ledger_transactions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(txn_count, 2, "original + reversal both persisted");
    let posting_sum: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(lp.minor_units), 0) FROM ledger_postings lp
                 JOIN ledger_accounts la ON la.id = lp.ledger_account_id
                 JOIN accounts a ON a.ledger_account_id = la.id
                 WHERE a.id = ?1",
            [account_id.as_uuid()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(posting_sum, 0, "the reversal nets the original to zero");

    // Voiding an already-voided transaction is rejected.
    assert!(
        worker
            .dispatch(
                meta(),
                WriteCommand::VoidTransaction {
                    transaction_id: txn_id,
                },
            )
            .is_err(),
        "double-void must fail",
    );
}

#[test]
fn tags_and_notes_round_trip() {
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
    worker
        .dispatch(
            meta(),
            WriteCommand::RecordTransaction {
                transaction_id: TransactionId::new(),
                account_id,
                amount: Money::new(-4_000, Currency::Usd),
                occurred_at: NaiveDate::from_ymd_opt(2026, 6, 20)
                    .unwrap()
                    .and_hms_opt(0, 0, 0)
                    .unwrap()
                    .and_utc(),
            },
        )
        .unwrap();
    let txn_id = worker.recent_transactions(50).unwrap()[0].transaction_id;

    // Create two tags; a duplicate active name is rejected.
    let vacation = TagId::new();
    let reimbursable = TagId::new();
    for (id, name, color) in [
        (vacation, "Vacation", None),
        (reimbursable, "Reimbursable", Some("#db8f6b".to_owned())),
    ] {
        worker
            .dispatch(
                meta(),
                WriteCommand::CreateTag {
                    id,
                    name: name.to_owned(),
                    color,
                },
            )
            .unwrap();
    }
    assert!(
        worker
            .dispatch(
                meta(),
                WriteCommand::CreateTag {
                    id: TagId::new(),
                    name: "Vacation".to_owned(),
                    color: None,
                },
            )
            .is_err(),
        "a duplicate active tag name is rejected"
    );
    assert_eq!(worker.tag_views().unwrap().len(), 2);

    // Tag + note the transaction; the list read exposes both.
    worker
        .dispatch(
            meta(),
            WriteCommand::SetTags {
                transaction_id: txn_id,
                tag_ids: vec![vacation, reimbursable],
            },
        )
        .unwrap();
    worker
        .dispatch(
            meta(),
            WriteCommand::SetNote {
                transaction_id: txn_id,
                note: Some("Hotel in Cancún".to_owned()),
            },
        )
        .unwrap();
    let row = worker.recent_transactions(50).unwrap().remove(0);
    assert_eq!(row.tag_ids.len(), 2);
    assert!(row.tag_ids.contains(&vacation) && row.tag_ids.contains(&reimbursable));
    assert_eq!(row.note.as_deref(), Some("Hotel in Cancún"));

    // Replacing the set removes a tag; clearing the note empties it.
    worker
        .dispatch(
            meta(),
            WriteCommand::SetTags {
                transaction_id: txn_id,
                tag_ids: vec![vacation],
            },
        )
        .unwrap();
    worker
        .dispatch(
            meta(),
            WriteCommand::SetNote {
                transaction_id: txn_id,
                note: None,
            },
        )
        .unwrap();
    let row = worker.recent_transactions(50).unwrap().remove(0);
    assert_eq!(row.tag_ids, vec![vacation]);
    assert_eq!(row.note, None);
}

#[test]
fn splits_round_trip_and_enforce_the_sum() {
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
    worker
        .dispatch(
            meta(),
            WriteCommand::RecordTransaction {
                transaction_id: TransactionId::new(),
                account_id,
                amount: Money::new(-20_000, Currency::Usd),
                occurred_at: NaiveDate::from_ymd_opt(2026, 6, 20)
                    .unwrap()
                    .and_hms_opt(0, 0, 0)
                    .unwrap()
                    .and_utc(),
            },
        )
        .unwrap();
    let txn_id = worker.recent_transactions(50).unwrap()[0].transaction_id;

    let tag = TagId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateTag {
                id: tag,
                name: "Costco run".to_owned(),
                color: None,
            },
        )
        .unwrap();

    // A split whose lines do not sum to the transaction amount is rejected.
    assert!(worker
        .dispatch(
            meta(),
            WriteCommand::SetSplits {
                transaction_id: txn_id,
                lines: vec![SplitLineInput {
                    amount: Money::new(-10_000, Currency::Usd),
                    category_id: None,
                    note: None,
                    tag_ids: vec![],
                }],
            },
        )
        .is_err());

    // Two lines (-120 + -80) sum to -200 — accepted; the read exposes them in order.
    worker
        .dispatch(
            meta(),
            WriteCommand::SetSplits {
                transaction_id: txn_id,
                lines: vec![
                    SplitLineInput {
                        amount: Money::new(-12_000, Currency::Usd),
                        category_id: None,
                        note: Some("groceries".to_owned()),
                        tag_ids: vec![tag],
                    },
                    SplitLineInput {
                        amount: Money::new(-8_000, Currency::Usd),
                        category_id: None,
                        note: None,
                        tag_ids: vec![],
                    },
                ],
            },
        )
        .unwrap();
    let lines = worker.transaction_splits(txn_id).unwrap();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].amount, Money::new(-12_000, Currency::Usd));
    assert_eq!(lines[0].note.as_deref(), Some("groceries"));
    assert_eq!(lines[0].tag_ids, vec![tag]);
    assert_eq!(lines[1].amount, Money::new(-8_000, Currency::Usd));
    assert_eq!(worker.recent_transactions(50).unwrap()[0].split_count, 2);

    // An empty split set un-splits the transaction.
    worker
        .dispatch(
            meta(),
            WriteCommand::SetSplits {
                transaction_id: txn_id,
                lines: vec![],
            },
        )
        .unwrap();
    assert!(worker.transaction_splits(txn_id).unwrap().is_empty());
    assert_eq!(worker.recent_transactions(50).unwrap()[0].split_count, 0);
}

#[test]
fn settings_round_trip_and_upsert() {
    let (_dir, worker) = worker();

    // An unset key reads as None.
    assert_eq!(worker.get_setting("reporting_currency").unwrap(), None);

    // Set, then read back.
    worker.set_setting("reporting_currency", "USD").unwrap();
    assert_eq!(
        worker.get_setting("reporting_currency").unwrap().as_deref(),
        Some("USD")
    );

    // Re-setting the same key upserts in place (no error, new value).
    worker.set_setting("reporting_currency", "EUR").unwrap();
    assert_eq!(
        worker.get_setting("reporting_currency").unwrap().as_deref(),
        Some("EUR")
    );

    // Independent keys coexist.
    worker.set_setting("locale", "en-US").unwrap();
    assert_eq!(
        worker.get_setting("locale").unwrap().as_deref(),
        Some("en-US")
    );
    assert_eq!(
        worker.get_setting("reporting_currency").unwrap().as_deref(),
        Some("EUR")
    );
}

#[test]
fn balance_assertion_anchors_the_balance_without_transactions() {
    let (_dir, worker) = worker();
    let account = AccountId::new();
    worker
        .dispatch(meta(), liquid_account_cmd(account, Currency::Usd, 0))
        .unwrap();
    // Set the balance directly — no transactions (ADR 0027).
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            account,
            Money::new(550_000, Currency::Usd),
            NaiveDate::from_ymd_opt(2026, 3, 20).unwrap(),
        )
        .unwrap();
    assert_eq!(
        worker.account_balance(account).unwrap(),
        Some(Money::new(550_000, Currency::Usd)),
    );
    // Nothing explains it yet → fully unexplained.
    assert_eq!(
        worker.account_unexplained(account).unwrap(),
        Some(Money::new(550_000, Currency::Usd)),
    );
}

#[test]
fn assertion_plug_shrinks_as_postings_explain_it() {
    use chrono::TimeZone;
    let (_dir, worker) = worker();
    let account = AccountId::new();
    worker
        .dispatch(meta(), liquid_account_cmd(account, Currency::Usd, 0))
        .unwrap();
    let txn = |amount: i64, y, m, d| WriteCommand::RecordTransaction {
        transaction_id: TransactionId::new(),
        account_id: account,
        amount: Money::new(amount, Currency::Usd),
        occurred_at: Utc.with_ymd_and_hms(y, m, d, 0, 0, 0).unwrap(),
    };
    // A $3,000 deposit on Mar 5 (a real posting).
    worker.dispatch(meta(), txn(300_000, 2026, 3, 5)).unwrap();
    // Assert $5,500 as of Mar 20: the balance jumps; $2,500 stays unexplained.
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            account,
            Money::new(550_000, Currency::Usd),
            NaiveDate::from_ymd_opt(2026, 3, 20).unwrap(),
        )
        .unwrap();
    assert_eq!(
        worker.account_balance(account).unwrap(),
        Some(Money::new(550_000, Currency::Usd)),
    );
    assert_eq!(
        worker.account_unexplained(account).unwrap(),
        Some(Money::new(250_000, Currency::Usd)),
    );
    // A $2,500 deposit on Mar 15 (inside the window) explains the rest.
    worker.dispatch(meta(), txn(250_000, 2026, 3, 15)).unwrap();
    // The balance is unchanged (anchored), but the plug is now zero.
    assert_eq!(
        worker.account_balance(account).unwrap(),
        Some(Money::new(550_000, Currency::Usd)),
    );
    assert_eq!(
        worker.account_unexplained(account).unwrap(),
        Some(Money::new(0, Currency::Usd)),
    );
}

#[test]
fn cash_tiers_bucket_liquid_subtypes_into_spendable_and_reserve() {
    let (_dir, worker) = worker();
    // Spendable = checking 1,000 + cash 200 = 1,200.
    worker
        .dispatch(
            meta(),
            liquid_subtype_cmd(
                AccountId::new(),
                "Checking",
                Some(AccountSubtype::Checking),
                100_000,
            ),
        )
        .unwrap();
    worker
        .dispatch(
            meta(),
            liquid_subtype_cmd(
                AccountId::new(),
                "Wallet",
                Some(AccountSubtype::Cash),
                20_000,
            ),
        )
        .unwrap();
    // Reserve = savings 5,000 + money market 800 = 5,800.
    worker
        .dispatch(
            meta(),
            liquid_subtype_cmd(
                AccountId::new(),
                "Savings",
                Some(AccountSubtype::Savings),
                500_000,
            ),
        )
        .unwrap();
    worker
        .dispatch(
            meta(),
            liquid_subtype_cmd(
                AccountId::new(),
                "MM",
                Some(AccountSubtype::MoneyMarket),
                80_000,
            ),
        )
        .unwrap();

    let tiers = worker.cash_tiers().unwrap();
    assert_eq!(tiers.spendable, Money::new(120_000, Currency::Usd));
    assert_eq!(tiers.reserve, Money::new(580_000, Currency::Usd));
    // Net = spendable + reserve = every liquid account (the AC invariant).
    assert_eq!(tiers.net, Money::new(700_000, Currency::Usd));
}

#[test]
fn unclassified_liquid_folds_into_spendable_so_net_equals_the_tiers() {
    let (_dir, worker) = worker();
    // A liquid account with no subtype is treated as spendable.
    worker
        .dispatch(
            meta(),
            liquid_subtype_cmd(AccountId::new(), "Unlabeled", None, 30_000),
        )
        .unwrap();
    worker
        .dispatch(
            meta(),
            liquid_subtype_cmd(
                AccountId::new(),
                "Savings",
                Some(AccountSubtype::Savings),
                70_000,
            ),
        )
        .unwrap();

    let tiers = worker.cash_tiers().unwrap();
    assert_eq!(tiers.spendable, Money::new(30_000, Currency::Usd));
    assert_eq!(tiers.reserve, Money::new(70_000, Currency::Usd));
    assert_eq!(tiers.net, Money::new(100_000, Currency::Usd));
    assert_eq!(
        tiers.net.minor_units(),
        tiers.spendable.minor_units() + tiers.reserve.minor_units(),
    );
}

#[test]
fn cash_tiers_exclude_non_liquid_accounts_and_use_asserted_balances() {
    let (_dir, worker) = worker();
    let checking = AccountId::new();
    worker
        .dispatch(
            meta(),
            liquid_subtype_cmd(checking, "Checking", Some(AccountSubtype::Checking), 0),
        )
        .unwrap();
    // A credit card (non-liquid) must never enter the cash tiers.
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(
                    Account::new(
                        AccountId::new(),
                        LedgerAccountId::new(),
                        "Card",
                        CashflowRole::CreditFacility,
                        Currency::Usd,
                        AccountFlags::default(),
                    )
                    .with_subtype(Some(AccountSubtype::CreditCard)),
                ),
                opening_balance: Some(Money::new(50_000, Currency::Usd)),
            },
        )
        .unwrap();
    // The tier reflects the assertion-anchored balance (ADR 0027), not the
    // opening posting — no transactions recorded.
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            checking,
            Money::new(425_000, Currency::Usd),
            NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
        )
        .unwrap();

    let tiers = worker.cash_tiers().unwrap();
    assert_eq!(tiers.spendable, Money::new(425_000, Currency::Usd));
    assert_eq!(tiers.reserve, Money::zero(Currency::Usd));
    assert_eq!(tiers.net, Money::new(425_000, Currency::Usd));
}

#[test]
fn set_account_subtype_sets_clears_and_rejects_role_mismatch() {
    let (_dir, worker) = worker();
    let id = AccountId::new();
    worker
        .dispatch(meta(), liquid_subtype_cmd(id, "Checking", None, 0))
        .unwrap();

    // Set a valid liquid subtype, then read it back.
    worker
        .dispatch(
            meta(),
            WriteCommand::SetAccountSubtype {
                id,
                subtype: Some(AccountSubtype::Savings),
            },
        )
        .unwrap();
    assert_eq!(
        worker.account_view(id).unwrap().unwrap().subtype.as_deref(),
        Some("savings"),
    );

    // Clearing it (None) returns to no subtype.
    worker
        .dispatch(
            meta(),
            WriteCommand::SetAccountSubtype { id, subtype: None },
        )
        .unwrap();
    assert_eq!(worker.account_view(id).unwrap().unwrap().subtype, None);

    // A subtype from another role is rejected (a liquid account is not a card).
    assert!(matches!(
        worker.dispatch(
            meta(),
            WriteCommand::SetAccountSubtype {
                id,
                subtype: Some(AccountSubtype::CreditCard),
            },
        ),
        Err(DbError::InvalidCommand(_)),
    ));
    // …and the rejected write left the account unchanged.
    assert_eq!(worker.account_view(id).unwrap().unwrap().subtype, None);
}

#[test]
fn recent_transactions_lists_newest_first_with_account_name() {
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

    let older = DateTime::parse_from_rfc3339("2026-06-01T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let newer = DateTime::parse_from_rfc3339("2026-06-05T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    // Income +$150, then expense -$40.
    worker
        .dispatch(
            meta(),
            WriteCommand::RecordTransaction {
                transaction_id: TransactionId::new(),
                account_id,
                amount: Money::new(15_000, Currency::Usd),
                occurred_at: older,
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
                occurred_at: newer,
            },
        )
        .unwrap();

    let txns = worker.recent_transactions(10).unwrap();
    assert_eq!(txns.len(), 2);
    // Newest first; one row per transaction (the user-account side only).
    assert_eq!(txns[0].occurred_at, newer);
    assert_eq!(txns[0].amount.minor_units(), -4_000);
    assert_eq!(txns[0].account_id, account_id);
    assert_eq!(txns[0].account_name, "Checking");
    assert_eq!(txns[1].occurred_at, older);
    assert_eq!(txns[1].amount.minor_units(), 15_000);
    // The limit caps the result.
    assert_eq!(worker.recent_transactions(1).unwrap().len(), 1);
}

/// npoe: a transfer moves money between two liquid accounts and leaves the
/// aggregate cash unchanged; it shows on both accounts in the list.
#[test]
fn transfer_moves_both_balances_and_leaves_net_unchanged() {
    let (_dir, worker) = worker();
    let liquid = |name: &str, opening: i64| {
        let account = Account::new(
            AccountId::new(),
            LedgerAccountId::new(),
            name,
            CashflowRole::LiquidCash,
            Currency::Usd,
            AccountFlags::default(),
        );
        let id = account.id();
        worker
            .dispatch(
                meta(),
                WriteCommand::CreateAccount {
                    account: Box::new(account),
                    opening_balance: Some(Money::new(opening, Currency::Usd)),
                },
            )
            .unwrap();
        id
    };
    let checking = liquid("Checking", 100_000);
    let savings = liquid("Savings", 20_000);

    worker
        .dispatch(
            meta(),
            WriteCommand::Transfer {
                source_account_id: checking,
                dest_account_id: savings,
                amount: Money::new(30_000, Currency::Usd),
                occurred_at: Utc::now(),
            },
        )
        .unwrap();

    let balance = |id| worker.account_balance(id).unwrap().unwrap().minor_units();
    assert_eq!(balance(checking), 70_000, "source debited");
    assert_eq!(balance(savings), 50_000, "destination credited");
    assert_eq!(
        balance(checking) + balance(savings),
        120_000,
        "aggregate cash unchanged"
    );

    // The transfer shows on both accounts (one signed row each).
    let rows = worker.recent_transactions(10).unwrap();
    let transfer: Vec<_> = rows
        .iter()
        .filter(|r| r.amount.minor_units().abs() == 30_000)
        .collect();
    assert_eq!(transfer.len(), 2, "one row per account");
    assert!(transfer.iter().any(|r| r.amount.minor_units() == -30_000));
    assert!(transfer.iter().any(|r| r.amount.minor_units() == 30_000));
}

/// npoe/r7sb: a transfer is rejected for the same account, a currency mismatch, a non-liquid
/// payer, or an investment destination; a liability destination (a debt payment) is allowed.
#[test]
fn transfer_validates_accounts_and_currency() {
    let (_dir, worker) = worker();
    let account = |name: &str, role: CashflowRole, currency: Currency| {
        let account = Account::new(
            AccountId::new(),
            LedgerAccountId::new(),
            name,
            role,
            currency,
            AccountFlags::default(),
        );
        let id = account.id();
        worker
            .dispatch(
                meta(),
                WriteCommand::CreateAccount {
                    account: Box::new(account),
                    opening_balance: None,
                },
            )
            .unwrap();
        id
    };
    let checking = account("Checking", CashflowRole::LiquidCash, Currency::Usd);
    let euro = account("Euro", CashflowRole::LiquidCash, Currency::Eur);
    let card = account("Card", CashflowRole::CreditFacility, Currency::Usd);
    let brokerage = account("Brokerage", CashflowRole::InvestmentAsset, Currency::Usd);
    let transfer = |src, dst, amount, currency| {
        worker.dispatch(
            meta(),
            WriteCommand::Transfer {
                source_account_id: src,
                dest_account_id: dst,
                amount: Money::new(amount, currency),
                occurred_at: Utc::now(),
            },
        )
    };
    assert!(
        transfer(checking, checking, 1_000, Currency::Usd).is_err(),
        "same account"
    );
    assert!(
        transfer(checking, euro, 1_000, Currency::Usd).is_err(),
        "currency mismatch"
    );
    assert!(
        transfer(card, checking, 1_000, Currency::Usd).is_err(),
        "a liability payer is rejected"
    );
    assert!(
        transfer(checking, brokerage, 1_000, Currency::Usd).is_ok(),
        "a liquid→investment contribution is allowed (9h0.1)"
    );
    assert!(
        transfer(checking, card, 1_000, Currency::Usd).is_ok(),
        "a liability dest is a debt payment (r7sb)"
    );
    assert!(
        transfer(brokerage, checking, 1_000, Currency::Usd).is_ok(),
        "an investment→liquid withdrawal is allowed (j0cg.2)"
    );
    assert!(
        transfer(brokerage, card, 1_000, Currency::Usd).is_err(),
        "an investment is liquidated to cash first, never paid straight to a liability"
    );
}

/// r7sb / ADR 0035 §3: a mixed-role transfer (liquid → liability) pays down the card — the
/// payer's liquid balance drops and the card's owed (negatively-stored) balance rises toward
/// zero by the same amount, and the two legs net to zero.
#[test]
fn mixed_role_transfer_pays_down_a_liability() {
    let (_dir, worker) = worker();
    let mk = |name: &str, role: CashflowRole, opening: i64| {
        let account = Account::new(
            AccountId::new(),
            LedgerAccountId::new(),
            name,
            role,
            Currency::Usd,
            AccountFlags::default(),
        );
        let id = account.id();
        worker
            .dispatch(
                meta(),
                WriteCommand::CreateAccount {
                    account: Box::new(account),
                    opening_balance: Some(Money::new(opening, Currency::Usd)),
                },
            )
            .unwrap();
        id
    };
    // Checking holds $2,000; the card owes $1,000 (a liability opens negative).
    let checking = mk("Checking", CashflowRole::LiquidCash, 200_000);
    let card = mk("Card", CashflowRole::CreditFacility, -100_000);

    worker
        .dispatch(
            meta(),
            WriteCommand::Transfer {
                source_account_id: checking,
                dest_account_id: card,
                amount: Money::new(30_000, Currency::Usd), // pay $300
                occurred_at: Utc::now(),
            },
        )
        .unwrap();

    assert_eq!(
        worker.account_balance(checking).unwrap().unwrap(),
        Money::new(170_000, Currency::Usd),
        "the payer's liquid balance drops by the payment"
    );
    assert_eq!(
        worker.account_balance(card).unwrap().unwrap(),
        Money::new(-70_000, Currency::Usd),
        "the card's owed balance shrinks toward zero"
    );

    // A loan_liability pays down identically (same credit-normal, negatively-stored sign).
    let loan = mk("Auto loan", CashflowRole::LoanLiability, -500_000);
    worker
        .dispatch(
            meta(),
            WriteCommand::Transfer {
                source_account_id: checking,
                dest_account_id: loan,
                amount: Money::new(50_000, Currency::Usd),
                occurred_at: Utc::now(),
            },
        )
        .unwrap();
    assert_eq!(
        worker.account_balance(loan).unwrap().unwrap(),
        Money::new(-450_000, Currency::Usd),
        "the loan principal shrinks toward zero"
    );
}

/// j0cg.2: a withdrawal (investment → liquid) moves cash from a brokerage into checking — the
/// brokerage's (positively-stored) asset balance drops and checking rises by the same amount.
#[test]
fn investment_withdrawal_moves_cash_from_a_brokerage() {
    let (_dir, worker) = worker();
    let mk = |name: &str, role: CashflowRole, opening: i64| {
        let account = Account::new(
            AccountId::new(),
            LedgerAccountId::new(),
            name,
            role,
            Currency::Usd,
            AccountFlags::default(),
        );
        let id = account.id();
        worker
            .dispatch(
                meta(),
                WriteCommand::CreateAccount {
                    account: Box::new(account),
                    opening_balance: Some(Money::new(opening, Currency::Usd)),
                },
            )
            .unwrap();
        id
    };
    let checking = mk("Checking", CashflowRole::LiquidCash, 100_000);
    let brokerage = mk("Brokerage", CashflowRole::InvestmentAsset, 500_000);

    worker
        .dispatch(
            meta(),
            WriteCommand::Transfer {
                source_account_id: brokerage,
                dest_account_id: checking,
                amount: Money::new(80_000, Currency::Usd), // withdraw $800
                occurred_at: Utc::now(),
            },
        )
        .unwrap();

    assert_eq!(
        worker.account_balance(brokerage).unwrap().unwrap(),
        Money::new(420_000, Currency::Usd),
        "the investment is drawn down"
    );
    assert_eq!(
        worker.account_balance(checking).unwrap().unwrap(),
        Money::new(180_000, Currency::Usd),
        "the cash arrives in checking"
    );

    // 9h0.1: the reverse leg — a contribution (liquid → investment) — raises the brokerage and
    // draws down checking.
    worker
        .dispatch(
            meta(),
            WriteCommand::Transfer {
                source_account_id: checking,
                dest_account_id: brokerage,
                amount: Money::new(50_000, Currency::Usd), // contribute $500
                occurred_at: Utc::now(),
            },
        )
        .unwrap();
    assert_eq!(
        worker.account_balance(brokerage).unwrap().unwrap(),
        Money::new(470_000, Currency::Usd),
        "the contribution raises the investment"
    );
    assert_eq!(
        worker.account_balance(checking).unwrap().unwrap(),
        Money::new(130_000, Currency::Usd),
        "the contribution draws down checking"
    );
}

/// dyy4: the unexplained adjustment plug auto-shrinks as real postings land in
/// the assertion window, and converting the residual records it as a real
/// transaction that zeroes the plug (ADR 0027 §8).
#[test]
fn unexplained_plug_auto_shrinks_then_converts_to_a_transaction() {
    let (_dir, worker) = worker();
    let account = Account::new(
        AccountId::new(),
        LedgerAccountId::new(),
        "Checking",
        CashflowRole::LiquidCash,
        Currency::Usd,
        AccountFlags::default(),
    );
    let id = account.id();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(account),
                opening_balance: None,
            },
        )
        .unwrap();

    let today = Utc::now().date_naive();
    let plug = |w: &DbWorker| w.account_unexplained(id).unwrap().unwrap().minor_units();

    // Assert $1000 as of today — the whole amount is unexplained (no postings).
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            id,
            Money::new(100_000, Currency::Usd),
            today,
        )
        .unwrap();
    assert_eq!(
        plug(&worker),
        100_000,
        "asserted balance starts fully unexplained"
    );

    // A real $300 inflow dated in the window shrinks the plug automatically.
    let at_today = today.and_hms_opt(0, 0, 0).unwrap().and_utc();
    worker
        .dispatch(
            meta(),
            WriteCommand::RecordTransaction {
                transaction_id: TransactionId::new(),
                account_id: id,
                amount: Money::new(30_000, Currency::Usd),
                occurred_at: at_today,
            },
        )
        .unwrap();
    assert_eq!(
        plug(&worker),
        70_000,
        "an in-window posting auto-shrinks the plug"
    );

    // Convert the residual → the plug is fully explained.
    worker
        .dispatch(
            meta(),
            WriteCommand::ConvertUnexplainedToTransaction { account_id: id },
        )
        .unwrap();
    assert_eq!(plug(&worker), 0, "converting the residual zeroes the plug");

    // The converted residual is a real transaction; the inflows now sum to the
    // asserted balance (300 + 700).
    let inflows: i64 = worker
        .recent_transactions(10)
        .unwrap()
        .iter()
        .map(|r| r.amount.minor_units())
        .filter(|amount| *amount > 0)
        .sum();
    assert_eq!(
        inflows, 100_000,
        "recorded inflows now equal the asserted balance"
    );

    // Nothing left to convert.
    assert!(worker
        .dispatch(
            meta(),
            WriteCommand::ConvertUnexplainedToTransaction { account_id: id },
        )
        .is_err());
}

#[test]
fn system_ledger_accounts_created_at_init() {
    let (_dir, worker) = worker();
    let conn = worker.read_connection().unwrap();
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM system_ledger_accounts", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        n, 3,
        "opening_balance_equity + unmatched_income + unmatched_expense"
    );
}

#[test]
fn account_views_lists_every_account_ordered_by_name() {
    let (_dir, worker) = worker();
    assert!(worker.account_views().unwrap().is_empty());

    // Insert out of alphabetical order; one carries an opening balance.
    for (name, opening) in [
        ("Zebra Savings", Some(Money::new(25_000, Currency::Usd))),
        ("apple checking", None),
    ] {
        let account = Account::new(
            AccountId::new(),
            LedgerAccountId::new(),
            name,
            CashflowRole::LiquidCash,
            Currency::Usd,
            AccountFlags::default(),
        );
        worker
            .dispatch(
                meta(),
                WriteCommand::CreateAccount {
                    account: Box::new(account),
                    opening_balance: opening,
                },
            )
            .unwrap();
    }

    let views = worker.account_views().unwrap();
    assert_eq!(views.len(), 2);
    // Ordered by name, case-insensitively (COLLATE NOCASE).
    assert_eq!(views[0].name, "apple checking");
    assert_eq!(views[1].name, "Zebra Savings");
    assert_eq!(views[1].balance, Money::new(25_000, Currency::Usd));
}

#[test]
fn record_transaction_moves_balance_and_routes_by_sign() {
    let (_dir, worker) = worker();
    let account = sample_account();
    let id = account.id();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(account),
                opening_balance: None,
            },
        )
        .unwrap();

    // Income then expense.
    worker
        .dispatch(
            meta(),
            WriteCommand::RecordTransaction {
                transaction_id: TransactionId::new(),
                account_id: id,
                amount: Money::new(15_000, Currency::Usd),
                occurred_at: Utc::now(),
            },
        )
        .unwrap();
    worker
        .dispatch(
            meta(),
            WriteCommand::RecordTransaction {
                transaction_id: TransactionId::new(),
                account_id: id,
                amount: Money::new(-4_000, Currency::Usd),
                occurred_at: Utc::now(),
            },
        )
        .unwrap();
    assert_eq!(
        worker.account_balance(id).unwrap(),
        Some(Money::new(11_000, Currency::Usd))
    );

    // The counter postings landed in the income and expense system accounts.
    let conn = worker.read_connection().unwrap();
    let book: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(minor_units),0) FROM ledger_postings",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(book, 0, "every posting is balanced by a counter-posting");
}

#[test]
fn record_transaction_rejects_zero_unknown_account_and_currency_mismatch() {
    let (_dir, worker) = worker();
    let account = sample_account(); // USD
    let id = account.id();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(account),
                opening_balance: None,
            },
        )
        .unwrap();

    // Zero amount.
    assert!(worker
        .dispatch(
            meta(),
            WriteCommand::RecordTransaction {
                transaction_id: TransactionId::new(),
                account_id: id,
                amount: Money::new(0, Currency::Usd),
                occurred_at: Utc::now(),
            },
        )
        .is_err());
    // Currency mismatch (account is USD).
    assert!(worker
        .dispatch(
            meta(),
            WriteCommand::RecordTransaction {
                transaction_id: TransactionId::new(),
                account_id: id,
                amount: Money::new(100, Currency::Eur),
                occurred_at: Utc::now(),
            },
        )
        .is_err());
    // Unknown account.
    assert!(worker
        .dispatch(
            meta(),
            WriteCommand::RecordTransaction {
                transaction_id: TransactionId::new(),
                account_id: AccountId::new(),
                amount: Money::new(100, Currency::Usd),
                occurred_at: Utc::now(),
            },
        )
        .is_err());
}

#[test]
fn opening_balance_is_recorded_as_an_equity_posting() {
    let (_dir, worker) = worker();
    let account = sample_account();
    let id = account.id();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(account),
                opening_balance: Some(Money::new(15_000, Currency::Usd)),
            },
        )
        .unwrap();

    // Balance comes from postings, not a column.
    assert_eq!(
        worker.account_balance(id).unwrap(),
        Some(Money::new(15_000, Currency::Usd))
    );

    let conn = worker.read_connection().unwrap();
    // Exactly two postings (account + opening-balance equity), summing to zero.
    let posting_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM ledger_postings", [], |r| r.get(0))
        .unwrap();
    assert_eq!(posting_count, 2);
    let sum: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(minor_units), 0) FROM ledger_postings",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(sum, 0, "double-entry: postings net to zero");
    // The accounts table has no balance column to hold a magic value.
    let has_balance_col: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('accounts') WHERE name = 'balance'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        has_balance_col, 0,
        "balance must not be a column on accounts"
    );
}

#[test]
fn account_without_opening_balance_has_zero_balance() {
    let (_dir, worker) = worker();
    let account = sample_account();
    let id = account.id();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateAccount {
                account: Box::new(account),
                opening_balance: None,
            },
        )
        .unwrap();
    assert_eq!(
        worker.account_balance(id).unwrap(),
        Some(Money::zero(Currency::Usd))
    );
}

/// HSA + crypto investment subtypes (ADR 0028 addendum, personal-cfo-4d8.25.23): the
/// widened migration CHECK accepts them, they persist + read back, and the kernel rejects
/// them on a non-investment role.
#[test]
fn hsa_and_crypto_investment_subtypes_persist_and_gate_by_role() {
    let (_dir, worker) = worker();
    for (name, subtype) in [
        ("Fidelity HSA", AccountSubtype::Hsa),
        ("Coinbase", AccountSubtype::Crypto),
    ] {
        let id = AccountId::new();
        worker
            .dispatch(
                meta(),
                WriteCommand::CreateAccount {
                    account: Box::new(
                        Account::new(
                            id,
                            LedgerAccountId::new(),
                            name,
                            CashflowRole::InvestmentAsset,
                            Currency::Usd,
                            AccountFlags::default(),
                        )
                        .with_subtype(Some(subtype)),
                    ),
                    opening_balance: Some(Money::new(100_000, Currency::Usd)),
                },
            )
            .unwrap();
        assert_eq!(
            worker.account_view(id).unwrap().unwrap().subtype.as_deref(),
            Some(subtype.as_str()),
        );
    }

    // An investment subtype on a liquid account is rejected by the role gate.
    let liquid = AccountId::new();
    worker
        .dispatch(meta(), liquid_subtype_cmd(liquid, "Checking", None, 0))
        .unwrap();
    assert!(matches!(
        worker.dispatch(
            meta(),
            WriteCommand::SetAccountSubtype {
                id: liquid,
                subtype: Some(AccountSubtype::Hsa),
            },
        ),
        Err(DbError::InvalidCommand(_)),
    ));
}

/// The seeded "Credit Card Payment" category is a TRANSFER under the "Transfers" group,
/// not an expense under "Debt" (ADR 0030 addendum, personal-cfo-4d8.25.21) — so it is
/// excluded from spend totals and recurring-bill detection.
#[test]
fn credit_card_payment_is_seeded_as_a_transfer_under_transfers() {
    let (_dir, worker) = worker();
    let cats = worker.category_views().unwrap();
    let ccp = cats
        .iter()
        .find(|c| c.name == "Credit Card Payment")
        .expect("Credit Card Payment is seeded");
    assert_eq!(ccp.category_type, "transfer");
    assert_eq!(ccp.forecast_behavior, "ignore_cashflow");
    let parent = cats
        .iter()
        .find(|c| Some(c.id) == ccp.parent_id)
        .expect("has a parent group");
    assert_eq!(parent.name, "Transfers");
    // It no longer lives under Debt.
    let debt = cats.iter().find(|c| c.name == "Debt").unwrap();
    assert!(!cats
        .iter()
        .any(|c| c.parent_id == Some(debt.id) && c.name == "Credit Card Payment"));
}

/// personal-cfo-4d8.27.8.1: a transfer moves money between two USER accounts, so each of
/// its two rows names the far side — that is what lets the list read "Checking → Savings".
/// An ordinary expense touches one user account and a SYSTEM counter-account, so it has
/// no counter account to name and must stay `None` rather than surfacing an internal one.
#[test]
fn a_transfer_row_names_the_other_account_and_an_expense_does_not() {
    let (_dir, worker) = worker();
    let liquid = |name: &str, opening: i64| {
        let account = Account::new(
            AccountId::new(),
            LedgerAccountId::new(),
            name,
            CashflowRole::LiquidCash,
            Currency::Usd,
            AccountFlags::default(),
        );
        let id = account.id();
        worker
            .dispatch(
                meta(),
                WriteCommand::CreateAccount {
                    account: Box::new(account),
                    opening_balance: Some(Money::new(opening, Currency::Usd)),
                },
            )
            .unwrap();
        id
    };
    let checking = liquid("Checking", 100_000);
    let savings = liquid("Savings", 20_000);

    worker
        .dispatch(
            meta(),
            WriteCommand::Transfer {
                source_account_id: checking,
                dest_account_id: savings,
                amount: Money::new(30_000, Currency::Usd),
                occurred_at: Utc::now(),
            },
        )
        .unwrap();
    worker
        .dispatch(
            meta(),
            WriteCommand::RecordTransaction {
                transaction_id: TransactionId::new(),
                account_id: checking,
                amount: Money::new(-4_500, Currency::Usd),
                occurred_at: Utc::now(),
            },
        )
        .unwrap();

    let rows = worker.recent_transactions(50).unwrap();

    // The transfer appears once per account; each names the OTHER one.
    let out = rows
        .iter()
        .find(|r| r.account_id == checking && r.amount.minor_units() == -30_000)
        .expect("the outgoing leg");
    assert_eq!(out.counter_account_name.as_deref(), Some("Savings"));
    assert_eq!(out.counter_account_id, Some(savings));
    let into = rows
        .iter()
        .find(|r| r.account_id == savings && r.amount.minor_units() == 30_000)
        .expect("the incoming leg");
    assert_eq!(into.counter_account_name.as_deref(), Some("Checking"));
    assert_eq!(into.counter_account_id, Some(checking));

    // The ordinary expense has no user counter-account — the system expense account
    // must never leak into the list.
    let spend = rows
        .iter()
        .find(|r| r.amount.minor_units() == -4_500)
        .expect("the expense");
    assert_eq!(spend.counter_account_id, None);
    assert_eq!(spend.counter_account_name, None);
}
