//! Income sources, recurring bills/transfers, commitments, instances, and actualization
//! (moved verbatim from the old in-file `lib.rs` tests module).

mod common;

use chrono::{NaiveDate, Utc};
use common::*;
use core_ledger::{
    Account, AccountFlags, AccountId, BillContractId, CashflowRole, CategoryId, IncomeSourceId,
    LedgerAccountId, RecurringEventId, RecurringTransferId, TagId, TransactionId,
};
use core_money::{Currency, Money};
use db_worker::*;
use pay_schedule::Frequency;
use rusqlite::params;
use uuid::Uuid;

#[test]
fn income_source_round_trips_with_next_pay_date() {
    let (_dir, worker) = worker();
    worker.dispatch(meta(), income_cmd(None)).unwrap();
    let views = worker.income_source_views().unwrap();
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].name, "Acme Corp");
    assert_eq!(views[0].net_amount, Money::new(300_000, Currency::Usd));
    assert_eq!(views[0].frequency, Frequency::Biweekly);
    assert!(views[0].next_pay_date.is_some());
    assert_eq!(views[0].deposit_account_name, None);
}

#[test]
fn income_source_edit_archive_restore_and_delete() {
    let (_dir, worker) = worker();
    worker.dispatch(meta(), income_cmd(None)).unwrap();
    let id = worker.income_source_views().unwrap()[0].id;

    // Edit: rename + change the amount/cadence.
    worker
        .dispatch(
            meta(),
            WriteCommand::UpdateIncomeSource {
                id,
                name: "Globex".to_owned(),
                net_amount: Money::new(400_000, Currency::Usd),
                frequency: Frequency::Monthly,
                anchor: NaiveDate::from_ymd_opt(2026, 6, 1).unwrap(),
                deposit_account_id: None,
            },
        )
        .unwrap();
    let v = worker.income_source_views().unwrap().remove(0);
    assert_eq!(v.name, "Globex");
    assert_eq!(v.net_amount, Money::new(400_000, Currency::Usd));
    assert_eq!(v.frequency, Frequency::Monthly);
    assert!(v.active && v.archived_at.is_none());

    // Archive: still listed but inactive + dated, and dropped from the forecast.
    worker
        .dispatch(meta(), WriteCommand::ArchiveIncomeSource(id))
        .unwrap();
    let v = worker.income_source_views().unwrap().remove(0);
    assert!(!v.active && v.archived_at.is_some());
    let forecast = worker.future_cash_forecast(120, &[]).unwrap();
    assert!(
        !forecast
            .days
            .iter()
            .any(|d| d.events.iter().any(|e| e.kind == "income")),
        "archived income must not appear in the forecast",
    );

    // Restore returns it to active (and the forecast).
    worker
        .dispatch(meta(), WriteCommand::RestoreIncomeSource(id))
        .unwrap();
    let v = worker.income_source_views().unwrap().remove(0);
    assert!(v.active && v.archived_at.is_none());

    // Delete removes it entirely.
    worker
        .dispatch(meta(), WriteCommand::DeleteIncomeSource(id))
        .unwrap();
    assert!(worker.income_source_views().unwrap().is_empty());
}

#[test]
fn income_source_validates_name_amount_and_deposit_account() {
    let (_dir, worker) = worker();
    // Unknown deposit account.
    let err = worker
        .dispatch(meta(), income_cmd(Some(AccountId::new())))
        .unwrap_err();
    assert!(matches!(err, DbError::InvalidCommand(_)));
    // Zero amount.
    let zero = WriteCommand::CreateIncomeSource {
        id: IncomeSourceId::new(),
        name: "X".to_owned(),
        net_amount: Money::zero(Currency::Usd),
        frequency: Frequency::Monthly,
        anchor: NaiveDate::from_ymd_opt(2026, 6, 1).unwrap(),
        deposit_account_id: None,
    };
    assert!(matches!(
        worker.dispatch(meta(), zero).unwrap_err(),
        DbError::InvalidCommand(_)
    ));
}

#[test]
fn migration_creates_recurring_and_commitment_tables() {
    let (_dir, worker) = worker();
    let conn = worker.read_connection().unwrap();
    for table in [
        "recurring_events",
        "recurring_event_instances",
        "bill_contracts",
        "commitments",
    ] {
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "{table} must exist after migration 4");
    }
}

#[test]
fn commitments_rebuild_derives_from_recurring_events_and_is_idempotent() {
    let (_dir, worker) = worker();
    let conn = worker.read_connection().unwrap();
    let event_id = Uuid::now_v7();
    let autopay = Uuid::now_v7();
    let now = Utc::now().to_rfc3339();
    // Seed canonical rows directly — no command bus writes these yet (PR 1).
    conn.execute(
        "INSERT INTO recurring_events (
                id, name, source, amount_expected_minor, currency, frequency,
                autopay_account_id, autopay_enabled, include_in_forecast, is_active,
                created_at, updated_at
            ) VALUES (?1, 'Netflix', 'manual', 1599, 'USD', 'monthly', ?2, 1, 1, 1, ?3, ?3)",
        params![event_id, autopay, now],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO bill_contracts (
                id, name, type, currency, cadence, recurring_event_id, status,
                include_in_forecast, created_at, updated_at
            ) VALUES (?1, 'Netflix', 'subscription', 'USD', 'monthly', ?2, 'active', 1, ?3, ?3)",
        params![Uuid::now_v7(), event_id, now],
    )
    .unwrap();

    let n = worker.rebuild_commitments().unwrap();
    assert_eq!(n, 1);
    let commitments = worker.commitment_views().unwrap();
    assert_eq!(commitments.len(), 1);
    let c = &commitments[0];
    assert_eq!(c.name, "Netflix");
    assert_eq!(c.commitment_type, "subscription"); // from the bill contract
    assert_eq!(c.amount_expected_minor, Some(1599));
    assert_eq!(
        c.payment_source_account_id,
        Some(AccountId::from_uuid(autopay))
    );
    assert_eq!(c.autopay_status, "enabled");
    assert_eq!(c.source_entity_type.as_deref(), Some("recurring_event"));
    assert_eq!(c.source_entity_id, Some(event_id));

    // Rebuild is idempotent: same canonical state → byte-identical projection.
    let first = worker.commitments_checksum().unwrap();
    worker.rebuild_commitments().unwrap();
    assert_eq!(worker.commitments_checksum().unwrap(), first);
    assert_eq!(worker.commitment_views().unwrap(), commitments);
}

#[test]
fn commitments_exclude_inactive_or_non_forecast_events() {
    let (_dir, worker) = worker();
    let conn = worker.read_connection().unwrap();
    let now = Utc::now().to_rfc3339();
    // is_active = 0 → excluded.
    conn.execute(
        "INSERT INTO recurring_events (
                id, name, source, amount_expected_minor, currency, frequency,
                include_in_forecast, is_active, created_at, updated_at
            ) VALUES (?1, 'Old Gym', 'manual', 5000, 'USD', 'monthly', 1, 0, ?2, ?2)",
        params![Uuid::now_v7(), now],
    )
    .unwrap();
    // include_in_forecast = 0 → excluded.
    conn.execute(
        "INSERT INTO recurring_events (
                id, name, source, amount_expected_minor, currency, frequency,
                include_in_forecast, is_active, created_at, updated_at
            ) VALUES (?1, 'Hidden', 'manual', 5000, 'USD', 'monthly', 0, 1, ?2, ?2)",
        params![Uuid::now_v7(), now],
    )
    .unwrap();

    assert_eq!(worker.rebuild_commitments().unwrap(), 0);
    assert!(worker.commitment_views().unwrap().is_empty());
}

#[test]
fn recurring_bill_round_trips_with_next_due_and_commitment() {
    let (_dir, worker) = worker();
    worker.dispatch(meta(), bill_cmd(None)).unwrap();

    let bills = worker.recurring_bill_views().unwrap();
    assert_eq!(bills.len(), 1);
    assert_eq!(bills[0].name, "Rent");
    assert_eq!(bills[0].bill_type, "rent_mortgage");
    assert_eq!(bills[0].amount, Money::new(180_000, Currency::Usd));
    assert_eq!(bills[0].frequency, Frequency::Monthly);
    assert!(bills[0].next_due_date.is_some());
    assert_eq!(bills[0].autopay_account_name, None);
    assert_eq!(bills[0].description, None);

    // The commitments projection was refreshed atomically by the bill write.
    let commitments = worker.commitment_views().unwrap();
    assert_eq!(commitments.len(), 1);
    assert_eq!(commitments[0].name, "Rent");
    assert_eq!(commitments[0].commitment_type, "mortgage"); // rent_mortgage -> mortgage
}

/// 4d8.24.5: a bill promoted with a category writes recurring_events.category_id, and it
/// round-trips on the bill read (so the Bills list/edit reflect it). No category -> None.
#[test]
fn recurring_bill_persists_and_round_trips_its_category() {
    let (_dir, worker) = worker();
    let cat_id: Uuid = {
        let conn = worker.read_connection().unwrap();
        conn.query_row(
            "SELECT id FROM categories WHERE archived_at IS NULL ORDER BY id LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateRecurringBill {
                event_id: RecurringEventId::new(),
                contract_id: BillContractId::new(),
                name: "Netflix".to_owned(),
                amount: Money::new(1_099, Currency::Usd),
                bill_type: "subscription".to_owned(),
                frequency: Frequency::Monthly,
                anchor: NaiveDate::from_ymd_opt(2026, 7, 5).unwrap(),
                autopay_account_id: None,
                description: None,
                source_merchant_key: None,
                category_id: Some(CategoryId::from_uuid(cat_id)),
                tag_ids: Vec::new(),
            },
        )
        .unwrap();

    let bills = worker.recurring_bill_views().unwrap();
    assert_eq!(bills.len(), 1);
    assert_eq!(bills[0].category_id, Some(CategoryId::from_uuid(cat_id)));

    // A bill created without a category reads back None.
    worker.dispatch(meta(), bill_cmd(None)).unwrap();
    let uncategorized = worker
        .recurring_bill_views()
        .unwrap()
        .into_iter()
        .find(|b| b.name == "Rent")
        .unwrap();
    assert_eq!(uncategorized.category_id, None);
}

/// 4d8.24.5.1 (ADR 0033 addendum): a bill promoted with tags persists them to
/// recurring_event_tags and round-trips on the read; an unknown tag id is rejected.
#[test]
fn recurring_bill_persists_and_round_trips_its_tags() {
    let (_dir, worker) = worker();
    let tag_a = TagId::new();
    let tag_b = TagId::new();
    for (id, name) in [(tag_a, "vacation"), (tag_b, "reimbursable")] {
        worker
            .dispatch(
                meta(),
                WriteCommand::CreateTag {
                    id,
                    name: name.to_owned(),
                    color: None,
                },
            )
            .unwrap();
    }

    let bill_with_tags = |name: &str, tag_ids: Vec<TagId>| WriteCommand::CreateRecurringBill {
        event_id: RecurringEventId::new(),
        contract_id: BillContractId::new(),
        name: name.to_owned(),
        amount: Money::new(4_000, Currency::Usd),
        bill_type: "membership".to_owned(),
        frequency: Frequency::Monthly,
        anchor: NaiveDate::from_ymd_opt(2026, 7, 5).unwrap(),
        autopay_account_id: None,
        description: None,
        source_merchant_key: None,
        category_id: None,
        tag_ids,
    };

    worker
        .dispatch(meta(), bill_with_tags("Gym", vec![tag_a, tag_b]))
        .unwrap();
    let bills = worker.recurring_bill_views().unwrap();
    assert_eq!(bills.len(), 1);
    assert_eq!(bills[0].tag_ids.len(), 2);
    assert!(bills[0].tag_ids.contains(&tag_a));
    assert!(bills[0].tag_ids.contains(&tag_b));

    // An unknown tag id is rejected at create (and nothing partial is committed).
    let err = worker.dispatch(meta(), bill_with_tags("Bad", vec![TagId::new()]));
    assert!(err.is_err(), "an unknown tag id is rejected");
    assert_eq!(worker.recurring_bill_views().unwrap().len(), 1);
}

/// mc7f/ADR 0041: SetBillAutopay toggles the explicit autopay flag on the bill and the
/// commitments projection reflects the intent (enabled/disabled), not account presence.
#[test]
fn set_bill_autopay_toggles_the_flag_and_commitment_status() {
    let (_dir, worker) = worker();
    worker.dispatch(meta(), bill_cmd(None)).unwrap();
    let id = worker.recurring_bill_views().unwrap()[0].id;
    // Default (never set): manual / unknown.
    assert!(!worker.recurring_bill_views().unwrap()[0].autopay_enabled);
    assert_eq!(
        worker.commitment_views().unwrap()[0].autopay_status,
        "unknown"
    );

    worker
        .dispatch(
            meta(),
            WriteCommand::SetBillAutopay {
                event_id: id,
                autopay: true,
            },
        )
        .unwrap();
    assert!(worker.recurring_bill_views().unwrap()[0].autopay_enabled);
    assert_eq!(
        worker.commitment_views().unwrap()[0].autopay_status,
        "enabled"
    );

    // Editing the bill's other fields must NOT reset the autopay intent (ADR 0041 drift fix).
    let anchor = worker.recurring_bill_views().unwrap()[0].anchor;
    worker
        .dispatch(
            meta(),
            WriteCommand::UpdateRecurringBill {
                event_id: id,
                name: "Rent (renamed)".to_owned(),
                amount: Money::new(180_000, Currency::Usd),
                bill_type: "rent_mortgage".to_owned(),
                frequency: Frequency::Monthly,
                anchor,
                autopay_account_id: None,
                description: None,
            },
        )
        .unwrap();
    assert!(
        worker.recurring_bill_views().unwrap()[0].autopay_enabled,
        "editing a bill preserves its autopay intent",
    );

    worker
        .dispatch(
            meta(),
            WriteCommand::SetBillAutopay {
                event_id: id,
                autopay: false,
            },
        )
        .unwrap();
    assert!(!worker.recurring_bill_views().unwrap()[0].autopay_enabled);
    assert_eq!(
        worker.commitment_views().unwrap()[0].autopay_status,
        "disabled"
    );

    // An unknown bill is rejected.
    assert!(worker
        .dispatch(
            meta(),
            WriteCommand::SetBillAutopay {
                event_id: RecurringEventId::new(),
                autopay: true,
            },
        )
        .is_err());
}

#[test]
fn recurring_bill_validates_name_amount_and_autopay_account() {
    let (_dir, worker) = worker();
    // Unknown autopay account.
    let err = worker
        .dispatch(meta(), bill_cmd(Some(AccountId::new())))
        .unwrap_err();
    assert!(matches!(err, DbError::InvalidCommand(_)));
    // Zero amount.
    let zero = WriteCommand::CreateRecurringBill {
        event_id: RecurringEventId::new(),
        contract_id: BillContractId::new(),
        name: "X".to_owned(),
        amount: Money::zero(Currency::Usd),
        bill_type: "other".to_owned(),
        frequency: Frequency::Monthly,
        anchor: NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
        autopay_account_id: None,
        description: None,
        source_merchant_key: None,
        category_id: None,
        tag_ids: Vec::new(),
    };
    assert!(matches!(
        worker.dispatch(meta(), zero).unwrap_err(),
        DbError::InvalidCommand(_)
    ));
}

#[test]
fn update_recurring_bill_rewrites_fields_and_refreshes_commitments() {
    let (_dir, worker) = worker();
    let create = bill_cmd(None);
    let WriteCommand::CreateRecurringBill { event_id, .. } = create else {
        unreachable!()
    };
    worker.dispatch(meta(), create).unwrap();
    let before = worker.commitments_checksum().unwrap();

    // Edit the name, amount, schedule, and add a description.
    worker
        .dispatch(
            meta(),
            WriteCommand::UpdateRecurringBill {
                event_id,
                name: "Mortgage".to_owned(),
                amount: Money::new(250_000, Currency::Usd),
                bill_type: "rent_mortgage".to_owned(),
                frequency: Frequency::Monthly,
                anchor: NaiveDate::from_ymd_opt(2026, 8, 1).unwrap(),
                autopay_account_id: None,
                description: Some("Primary residence".to_owned()),
            },
        )
        .unwrap();

    let bills = worker.recurring_bill_views().unwrap();
    assert_eq!(bills.len(), 1);
    assert_eq!(bills[0].name, "Mortgage");
    assert_eq!(bills[0].amount, Money::new(250_000, Currency::Usd));
    assert_eq!(
        bills[0].anchor,
        NaiveDate::from_ymd_opt(2026, 8, 1).unwrap()
    );
    assert_eq!(bills[0].description.as_deref(), Some("Primary residence"));

    // The commitments projection reflects the edit (amount + name changed).
    assert_ne!(before, worker.commitments_checksum().unwrap());
    let commitments = worker.commitment_views().unwrap();
    assert_eq!(commitments.len(), 1);
    assert_eq!(commitments[0].name, "Mortgage");
    assert_eq!(commitments[0].amount_expected_minor, Some(250_000));
}

#[test]
fn delete_recurring_bill_removes_it_and_empties_commitments() {
    let (_dir, worker) = worker();
    let create = bill_cmd(None);
    let WriteCommand::CreateRecurringBill { event_id, .. } = create else {
        unreachable!()
    };
    worker.dispatch(meta(), create).unwrap();
    assert_eq!(worker.recurring_bill_views().unwrap().len(), 1);

    worker
        .dispatch(meta(), WriteCommand::DeleteRecurringBill { event_id })
        .unwrap();

    assert!(worker.recurring_bill_views().unwrap().is_empty());
    assert!(worker.commitment_views().unwrap().is_empty());
}

#[test]
fn update_or_delete_unknown_recurring_bill_is_an_error() {
    let (_dir, worker) = worker();
    let missing = RecurringEventId::new();
    let update = WriteCommand::UpdateRecurringBill {
        event_id: missing,
        name: "Ghost".to_owned(),
        amount: Money::new(1_000, Currency::Usd),
        bill_type: "other".to_owned(),
        frequency: Frequency::Monthly,
        anchor: NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
        autopay_account_id: None,
        description: None,
    };
    assert!(matches!(
        worker.dispatch(meta(), update).unwrap_err(),
        DbError::InvalidCommand(_)
    ));
    assert!(matches!(
        worker
            .dispatch(
                meta(),
                WriteCommand::DeleteRecurringBill { event_id: missing }
            )
            .unwrap_err(),
        DbError::InvalidCommand(_)
    ));
}

#[test]
fn archive_and_restore_recurring_bill_toggles_forecast_inclusion() {
    let (_dir, worker) = worker();
    let create = bill_cmd(None);
    let WriteCommand::CreateRecurringBill { event_id, .. } = create else {
        unreachable!()
    };
    worker.dispatch(meta(), create).unwrap();
    assert_eq!(worker.commitment_views().unwrap().len(), 1);

    // Archive: the bill is retained (still listed) but leaves the forecast.
    worker
        .dispatch(meta(), WriteCommand::ArchiveRecurringBill(event_id))
        .unwrap();
    let bills = worker.recurring_bill_views().unwrap();
    assert_eq!(bills.len(), 1, "archived bill is still listed");
    assert!(!bills[0].active);
    assert!(bills[0].archived_at.is_some());
    assert!(!bills[0].created_at.is_empty());
    assert!(
        worker.commitment_views().unwrap().is_empty(),
        "archived bill is excluded from commitments / forecast"
    );

    // Restore: it returns to active + the forecast.
    worker
        .dispatch(meta(), WriteCommand::RestoreRecurringBill(event_id))
        .unwrap();
    let bills = worker.recurring_bill_views().unwrap();
    assert!(bills[0].active);
    assert!(bills[0].archived_at.is_none());
    assert_eq!(worker.commitment_views().unwrap().len(), 1);
}

#[test]
fn archive_or_restore_unknown_recurring_bill_is_an_error() {
    let (_dir, worker) = worker();
    let missing = RecurringEventId::new();
    assert!(matches!(
        worker
            .dispatch(meta(), WriteCommand::ArchiveRecurringBill(missing))
            .unwrap_err(),
        DbError::InvalidCommand(_)
    ));
    assert!(matches!(
        worker
            .dispatch(meta(), WriteCommand::RestoreRecurringBill(missing))
            .unwrap_err(),
        DbError::InvalidCommand(_)
    ));
}

/// 5ie.9 (review): the confirm command refuses the cases outside its v1 liquid-paid-bill scope
/// — a non-liquid payer, a currency mismatch, and a card-charged bill (double-counts vs the
/// card payment) — rather than silently mismodeling.
#[test]
fn confirming_rejects_out_of_scope_inputs() {
    let (_dir, worker) = worker();
    let checking = AccountId::new();
    let card = AccountId::new();
    create_role_account(&worker, checking, "Checking", CashflowRole::LiquidCash);
    create_role_account(&worker, card, "Visa", CashflowRole::CreditFacility);
    worker
        .dispatch(
            meta(),
            WriteCommand::SetDebtTerms {
                account_id: card,
                terms: sample_debt_terms(Some(checking)),
            },
        )
        .unwrap();
    // A plain bill paid from checking, and a card-charged bill (autopay = the card).
    let bill = RecurringEventId::new();
    let card_bill = RecurringEventId::new();
    for (id, autopay) in [(bill, checking), (card_bill, card)] {
        worker
            .dispatch(
                meta(),
                WriteCommand::CreateRecurringBill {
                    event_id: id,
                    contract_id: BillContractId::new(),
                    name: "Bill".to_owned(),
                    amount: Money::new(10_000, Currency::Usd),
                    bill_type: "subscription".to_owned(),
                    frequency: Frequency::Monthly,
                    anchor: NaiveDate::from_ymd_opt(2026, 7, 15).unwrap(),
                    autopay_account_id: Some(autopay),
                    description: None,
                    source_merchant_key: None,
                    category_id: None,
                    tag_ids: Vec::new(),
                },
            )
            .unwrap();
    }
    let confirm = |event: RecurringEventId, amount: Money, paying: AccountId| {
        worker.dispatch(
            meta(),
            WriteCommand::ConfirmObligationEarly {
                recurring_event_id: event,
                scheduled_date: NaiveDate::from_ymd_opt(2026, 7, 15).unwrap(),
                actual_amount: amount,
                actual_date: NaiveDate::from_ymd_opt(2026, 7, 2)
                    .unwrap()
                    .and_hms_opt(12, 0, 0)
                    .unwrap()
                    .and_utc(),
                paying_account_id: paying,
            },
        )
    };
    // Paying from a non-liquid (credit) account: rejected.
    assert!(confirm(bill, Money::new(10_000, Currency::Usd), card).is_err());
    // Currency mismatch with the paying account: rejected.
    assert!(confirm(bill, Money::new(10_000, Currency::Eur), checking).is_err());
    // A card-charged bill (its impact is the card payment): rejected.
    assert!(confirm(card_bill, Money::new(10_000, Currency::Usd), checking).is_err());
    // The in-scope case still works.
    assert!(confirm(bill, Money::new(10_000, Currency::Usd), checking).is_ok());
}

/// personal-cfo-4d8.24.7.1: the paged transaction read can filter to the payments
/// confirmed against one recurring bill (its `confirmed_obligations`), the history a
/// bill's detail panel shows — and only those, never unrelated transactions.
#[test]
fn transaction_page_filters_to_a_bills_confirmed_payments() {
    let (_dir, worker) = worker();
    let checking = AccountId::new();
    create_role_account(&worker, checking, "Checking", CashflowRole::LiquidCash);

    // An unrelated manual transaction that must NOT appear under the bill filter.
    worker
        .dispatch(
            meta(),
            WriteCommand::RecordTransaction {
                transaction_id: TransactionId::new(),
                account_id: checking,
                amount: Money::new(-5_000, Currency::Usd),
                occurred_at: NaiveDate::from_ymd_opt(2026, 7, 1)
                    .unwrap()
                    .and_hms_opt(9, 0, 0)
                    .unwrap()
                    .and_utc(),
            },
        )
        .unwrap();

    let bill = RecurringEventId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateRecurringBill {
                event_id: bill,
                contract_id: BillContractId::new(),
                name: "Rent".to_owned(),
                amount: Money::new(10_000, Currency::Usd),
                bill_type: "rent_mortgage".to_owned(),
                frequency: Frequency::Monthly,
                anchor: NaiveDate::from_ymd_opt(2026, 7, 15).unwrap(),
                autopay_account_id: Some(checking),
                description: None,
                source_merchant_key: None,
                category_id: None,
                tag_ids: Vec::new(),
            },
        )
        .unwrap();
    // Confirming the bill posts the paying transaction AND records the
    // confirmed_obligations link (recurring_event_id -> transaction_id).
    worker
        .dispatch(
            meta(),
            WriteCommand::ConfirmObligationEarly {
                recurring_event_id: bill,
                scheduled_date: NaiveDate::from_ymd_opt(2026, 7, 15).unwrap(),
                actual_amount: Money::new(10_000, Currency::Usd),
                actual_date: NaiveDate::from_ymd_opt(2026, 7, 3)
                    .unwrap()
                    .and_hms_opt(12, 0, 0)
                    .unwrap()
                    .and_utc(),
                paying_account_id: checking,
            },
        )
        .unwrap();

    let page = |recurring_event_id: Option<RecurringEventId>| {
        worker
            .transaction_page(&TransactionPageQuery {
                recurring_event_id,
                limit: 50,
                ..TransactionPageQuery::default()
            })
            .unwrap()
    };

    // Unfiltered: both the unrelated transaction and the confirmed payment.
    assert_eq!(page(None).total, 2);
    // Filtered to the bill: exactly its confirmed payment (the -$100 outflow), never
    // the unrelated -$50 one.
    let only = page(Some(bill));
    assert_eq!(only.total, 1);
    assert_eq!(only.rows.len(), 1);
    assert_eq!(only.rows[0].amount.minor_units(), -10_000);
    // A different bill with no confirmations returns nothing.
    assert_eq!(page(Some(RecurringEventId::new())).total, 0);
}

/// npoe: a recurring transfer projects two legs in the per-account forecast
/// (source −, destination +) and is omitted from the aggregate (ADR 0026 §14).
#[test]
fn recurring_transfer_projects_two_legs_and_nets_to_zero() {
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

    // A monthly transfer anchored a few days out so it lands inside a 30-day
    // horizon.
    let anchor = Utc::now().date_naive() + chrono::Duration::days(3);
    let id = RecurringTransferId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateRecurringTransfer {
                id,
                source_account_id: checking,
                dest_account_id: savings,
                amount: Money::new(30_000, Currency::Usd),
                frequency: Frequency::Monthly,
                anchor,
            },
        )
        .unwrap();

    // The view lists it with a next occurrence.
    let views = worker.recurring_transfer_views().unwrap();
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].source_account_name, "Checking");
    assert_eq!(views[0].dest_account_name, "Savings");
    assert!(views[0].next_date.is_some());

    // Per-account: −30000 on the source series, +30000 on the destination.
    let multi = worker.future_cash_by_account(30, &[]).unwrap();
    let leg_amounts = |account: AccountId| -> Vec<i64> {
        multi
            .accounts
            .iter()
            .find(|s| s.account_id == Some(account.as_uuid()))
            .unwrap()
            .days
            .iter()
            .flat_map(|d| d.events.iter())
            .filter(|e| e.kind == "transfer")
            .map(|e| e.amount.minor_units())
            .collect()
    };
    assert_eq!(leg_amounts(checking), vec![-30_000], "source debited");
    assert_eq!(leg_amounts(savings), vec![30_000], "destination credited");

    // The aggregate forecast omits transfers entirely (net zero) — with no
    // income or bills, every day's closing equals the starting balance.
    let agg = worker.future_cash_forecast(30, &[]).unwrap();
    let starting = agg.starting_balance.minor_units();
    assert!(
        agg.days
            .iter()
            .all(|d| d.events.is_empty() && d.closing.p50.minor_units() == starting),
        "a transfer must not move the aggregate"
    );

    // Delete stops the projection.
    worker
        .dispatch(meta(), WriteCommand::DeleteRecurringTransfer(id))
        .unwrap();
    assert!(worker.recurring_transfer_views().unwrap().is_empty());
    let after = worker.future_cash_by_account(30, &[]).unwrap();
    assert!(
        after
            .accounts
            .iter()
            .all(|s| s.days.iter().all(|d| d.events.is_empty())),
        "deleted transfer no longer projects"
    );
}

/// 9h0.1: a recurring transfer to an INVESTMENT account (a DCA contribution) is a REAL outflow
/// in the aggregate liquid forecast — unlike a net-zero liquid↔liquid transfer — and the
/// per-account series still reconcile to the aggregate (the investment leg is dropped, ADR 0035 §3).
#[test]
fn recurring_investment_contribution_reduces_the_aggregate_and_reconciles() {
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
    let brokerage = mk("Brokerage", CashflowRole::InvestmentAsset, 0);

    let anchor = Utc::now().date_naive() + chrono::Duration::days(3);
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateRecurringTransfer {
                id: RecurringTransferId::new(),
                source_account_id: checking,
                dest_account_id: brokerage,
                amount: Money::new(30_000, Currency::Usd),
                frequency: Frequency::Monthly,
                anchor,
            },
        )
        .unwrap();

    // Aggregate: the source (liquid) leg is a real outflow — exactly one event, and the closing
    // ends $300 below the start (the cash left the liquid pool for the investment).
    let agg = worker.future_cash_forecast(30, &[]).unwrap();
    let starting = agg.starting_balance.minor_units();
    assert_eq!(
        starting, 100_000,
        "only the liquid account seeds the aggregate"
    );
    let moved: Vec<i64> = agg
        .days
        .iter()
        .filter(|d| !d.events.is_empty())
        .map(|d| d.closing.p50.minor_units())
        .collect();
    assert_eq!(
        moved,
        vec![70_000],
        "the contribution leaves the aggregate liquid forecast once"
    );
    assert_eq!(
        agg.days.last().unwrap().closing.p50.minor_units(),
        70_000,
        "and stays gone (not netted back like a liquid↔liquid transfer)"
    );

    // Reconciliation: Σ per-account liquid closings == the aggregate closing, day by day. The
    // brokerage is non-liquid, so it never enters the per-account series (its +leg is dropped).
    let multi = worker.future_cash_by_account(30, &[]).unwrap();
    assert!(
        multi
            .accounts
            .iter()
            .all(|s| s.account_id != Some(brokerage.as_uuid())),
        "the investment account is not a liquid series"
    );
    for (i, agg_day) in agg.days.iter().enumerate() {
        let per_account_sum: i64 = multi
            .accounts
            .iter()
            .map(|s| s.days[i].closing.p50.minor_units())
            .sum();
        assert_eq!(
            per_account_sum,
            agg_day.closing.p50.minor_units(),
            "per-account liquid reconciles to the aggregate on day {i}"
        );
    }
}

/// Interval frequencies end to end (ADR 0048, personal-cfo-4d8.25.14): an every-6-weeks
/// bill projects liquid outflows exactly 42 days apart, anchored on the bill's anchor.
#[test]
fn every_n_weeks_bill_projects_on_the_interval_lattice() {
    use chrono::{Days, Utc};
    let (_dir, worker) = worker();
    let checking = AccountId::new();
    create_role_account(&worker, checking, "Checking", CashflowRole::LiquidCash);
    let today = Utc::now().date_naive();
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            checking,
            Money::new(1_000_000, Currency::Usd),
            today - Days::new(1),
        )
        .unwrap();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateRecurringBill {
                event_id: RecurringEventId::new(),
                contract_id: BillContractId::new(),
                name: "Lawn service".to_owned(),
                amount: Money::new(9_000, Currency::Usd),
                bill_type: "utility".to_owned(),
                frequency: Frequency::EveryNWeeks(6),
                anchor: today + Days::new(3),
                autopay_account_id: Some(checking),
                description: None,
                source_merchant_key: None,
                category_id: None,
                tag_ids: Vec::new(),
            },
        )
        .unwrap();

    let view = worker.future_cash_forecast(180, &[]).unwrap();
    let dates: Vec<chrono::NaiveDate> = view
        .days
        .iter()
        .filter(|d| d.events.iter().any(|e| e.kind == "recurring_bill"))
        .map(|d| d.date)
        .collect();
    assert!(
        dates.len() >= 4,
        "a 180-day horizon holds at least 4 six-week occurrences: {dates:?}"
    );
    assert_eq!(dates[0], today + Days::new(3), "anchored on the given date");
    for pair in dates.windows(2) {
        assert_eq!(
            (pair[1] - pair[0]).num_days(),
            42,
            "every 6 weeks means 42-day spacing"
        );
    }
}
