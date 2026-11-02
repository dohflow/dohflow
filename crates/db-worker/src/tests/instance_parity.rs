//! The recurring-instance table is a derived read model (personal-cfo-5ymg8.9): reclassifying it
//! as class 2 changes no value anyone reads. This characterizes everything that reads it on one
//! fixed-date vault (fake data) and pins the result to a golden file written from the code as it
//! was before the reclassification. Regenerate with `INSTANCE_PARITY_UPDATE=1` only when a change
//! is MEANT to move these values.

use super::*;
use chrono::TimeZone;
use std::collections::HashMap;
use std::fmt::Write as _;

const GOLDEN_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/tests/instance_parity.golden.txt"
);
const GOLDEN: &str = include_str!("instance_parity.golden.txt");

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn record(worker: &DbWorker, account: AccountId, minor: i64, on: NaiveDate) {
    worker
        .dispatch(
            meta(),
            WriteCommand::RecordTransaction {
                transaction_id: TransactionId::new(),
                account_id: account,
                amount: Money::new(minor, Currency::Usd),
                occurred_at: on.and_hms_opt(12, 0, 0).unwrap().and_utc(),
            },
        )
        .unwrap();
}

fn bill(
    worker: &DbWorker,
    name: &str,
    minor: i64,
    anchor: NaiveDate,
    pay_from: AccountId,
) -> RecurringEventId {
    let event_id = RecurringEventId::new();
    worker
        .dispatch(
            meta(),
            WriteCommand::CreateRecurringBill {
                event_id,
                contract_id: BillContractId::new(),
                name: name.to_owned(),
                amount: Money::new(minor, Currency::Usd),
                bill_type: "subscription".to_owned(),
                frequency: Frequency::Monthly,
                anchor,
                autopay_account_id: Some(pay_from),
                description: None,
                source_merchant_key: None,
                category_id: None,
                tag_ids: Vec::new(),
            },
        )
        .unwrap();
    event_id
}

/// Everything a reader of `recurring_event_instances` can see, as stable text. Ids are random per
/// run, so rows are named by their schedule and a linked posting by its date and amount.
fn snapshot() -> (String, Vec<String>) {
    let (_dir, worker) = worker();
    let checking = AccountId::new();
    let card = AccountId::new();
    worker
        .dispatch(
            meta(),
            liquid_subtype_cmd(checking, "Checking", Some(AccountSubtype::Checking), 0),
        )
        .unwrap();
    worker
        .record_balance_assertion(
            Uuid::now_v7(),
            checking,
            Money::new(500_000, Currency::Usd),
            date(2026, 6, 1),
        )
        .unwrap();
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

    // Income that links (the 2026-06-19 deposit), bills that link (liquid and card-charged), a
    // past-due bill nobody paid, and one the household confirmed.
    worker.dispatch(meta(), income_cmd(Some(checking))).unwrap();
    record(&worker, checking, 300_000, date(2026, 6, 19));
    let rent = bill(&worker, "Rent", 180_000, date(2026, 7, 1), checking);
    record(&worker, checking, -180_000, date(2026, 7, 1));
    let spotify = bill(&worker, "Spotify", 1_000, date(2026, 7, 10), card);
    record(&worker, card, -1_000, date(2026, 7, 10));
    let gym = bill(&worker, "Gym", 4_500, date(2026, 7, 12), checking);
    let water = bill(&worker, "Water", 6_000, date(2026, 7, 15), checking);
    worker
        .dispatch(
            meta(),
            WriteCommand::ConfirmObligationEarly {
                recurring_event_id: water,
                scheduled_date: date(2026, 7, 15),
                actual_amount: Money::new(6_000, Currency::Usd),
                actual_date: date(2026, 7, 15).and_hms_opt(12, 0, 0).unwrap().and_utc(),
                paying_account_id: checking,
            },
        )
        .unwrap();

    let today = date(2026, 7, 20);
    let mut conn = worker.read_connection().unwrap();
    crate::recurring_instances::rebuild(&mut conn, today).unwrap();

    let names: HashMap<Uuid, String> = [
        (rent.as_uuid(), "Rent"),
        (spotify.as_uuid(), "Spotify"),
        (gym.as_uuid(), "Gym"),
        (water.as_uuid(), "Water"),
    ]
    .into_iter()
    .map(|(id, name)| (id, name.to_owned()))
    .chain(
        worker
            .income_source_views()
            .unwrap()
            .into_iter()
            .map(|s| (s.id.as_uuid(), s.name)),
    )
    .collect();
    let name_of = |id: Uuid| names.get(&id).cloned().unwrap_or_else(|| id.to_string());

    // What each linked instance is linked to, by the posting's date and amount.
    let posting_of = |txn: Uuid| -> String {
        conn.query_row(
            "SELECT lp.posting_date, lp.minor_units FROM ledger_postings lp
              WHERE lp.transaction_id = ?1 ORDER BY lp.posting_date, lp.minor_units LIMIT 1",
            [txn],
            |r| {
                Ok(format!(
                    "{} {}",
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?
                ))
            },
        )
        .unwrap()
    };

    let mut out = String::new();
    let mut linked = Vec::new();

    out.push_str("[instances]\n");
    let mut rows = crate::recurring_instances::read_rows(&conn).unwrap();
    rows.sort_by(|a, b| {
        (name_of(a.recurring_event_id.into()), &a.scheduled_date)
            .cmp(&(name_of(b.recurring_event_id.into()), &b.scheduled_date))
    });
    for row in &rows {
        let name = name_of(row.recurring_event_id);
        let link = row
            .linked_transaction_id
            .map_or("-".to_owned(), &posting_of);
        if row.linked_transaction_id.is_some() {
            linked.push(format!("{name} {}", row.scheduled_date));
        }
        writeln!(
            out,
            "{name} | {} | {} {} | {} | {link}",
            row.scheduled_date, row.expected_amount_minor, row.currency, row.status
        )
        .unwrap();
    }

    out.push_str("[bills]\n");
    for (label, id) in [
        ("Rent", rent),
        ("Spotify", spotify),
        ("Gym", gym),
        ("Water", water),
    ] {
        for row in crate::recurring_instances::read_rows_for_event(&conn, id.as_uuid()).unwrap() {
            let link = row
                .linked_transaction_id
                .map_or("-".to_owned(), &posting_of);
            writeln!(
                out,
                "{label} | {} | {} | {} | {link}",
                row.scheduled_date, row.expected_amount_minor, row.status
            )
            .unwrap();
        }
    }

    out.push_str("[unconfirmed past due]\n");
    for o in crate::recurring_instances::read_unconfirmed_past_due(&conn, today).unwrap() {
        writeln!(
            out,
            "{} | {} | {} {} | {} days",
            o.name, o.scheduled_date, o.expected_amount_minor, o.currency, o.days_overdue
        )
        .unwrap();
    }

    out.push_str("[future cash]\n");
    let as_of = Utc.with_ymd_and_hms(2026, 7, 20, 12, 0, 0).unwrap();
    let view = crate::forecast::compute(&conn, as_of, 75, &[]).unwrap();
    writeln!(
        out,
        "start {} on {}",
        view.starting_balance.minor_units(),
        view.start_date
    )
    .unwrap();
    for day in &view.days {
        for e in &day.events {
            writeln!(
                out,
                "{} | {} | {} | {} | {:?} | closing {}",
                day.date,
                e.name,
                e.kind,
                e.amount.minor_units(),
                e.assumption_basis,
                day.closing.p50.minor_units()
            )
            .unwrap();
        }
    }

    (out, linked)
}

#[test]
fn every_reader_of_the_instance_table_sees_what_it_saw_before_it_became_class_2() {
    let (text, linked) = snapshot();

    // The fixture must exercise what it claims to: a vacuous snapshot proves nothing.
    for expected in [
        "Acme Corp 2026-06-19",
        "Rent 2026-07-01",
        "Spotify 2026-07-10",
    ] {
        assert!(
            linked.iter().any(|l| l == expected),
            "{expected} is a linked instance; linked: {linked:?}"
        );
    }
    assert!(
        text.contains("[unconfirmed past due]\nGym | 2026-07-12 | 4500 USD | 8 days\n"),
        "the unpaid Gym is the one unconfirmed past-due bill:\n{text}"
    );
    assert!(
        !text.contains("Water | 2026-07-15 | 6000 USD | 5 days"),
        "the confirmed Water bill is not unconfirmed"
    );

    if std::env::var_os("INSTANCE_PARITY_UPDATE").is_some() {
        std::fs::write(GOLDEN_PATH, &text).unwrap();
        return;
    }
    assert_eq!(
        text, GOLDEN,
        "a reader of recurring_event_instances saw different values; if that is intended, \
         regenerate with INSTANCE_PARITY_UPDATE=1 and review the diff"
    );
}
