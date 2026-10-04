//! Layered dedupe through the production import path (personal-cfo-yl5).
//!
//! ADR 0014 §3 (and its addenda) and ADR 0008 §5 define four layers: the exact
//! file hash, the account-scoped transaction fingerprint, the silent certain
//! provider refetch (connectors only), and the cross-source heuristic (same
//! account, exact posted date, same signed amount, different source type).
//! These tests drive real CSV and OFX bytes through `import_batch_impl` — the
//! registered importers, the bounded host, staging, `CommitStaged` and the
//! Money Inbox projection — and resolve flags through the same review commands
//! the UI uses.

use app_lib::ipc::commands::{
    create_account_impl, duplicate_candidates_impl, import_batch_impl, import_staged_anyway_impl,
    money_inbox_list_impl, skip_staged_transaction_impl, transaction_list_impl,
    void_transaction_impl,
};
use app_lib::ipc::dto::{
    AccountFlagsDto, BatchResultDto, CashflowRoleDto, CreateAccountInput, ImportBatchInput,
    MoneyInboxItemDto,
};
use app_lib::AppState;
use chrono::{Duration, NaiveDate};
use finance_kernel::VaultController;
use tempfile::TempDir;

const ROWS: usize = 50;

fn open_state() -> (TempDir, AppState) {
    let dir = TempDir::new().expect("temp dir");
    let mut controller = VaultController::open(dir.path().join("vault.db"));
    controller.create(b"test-key").expect("create vault");
    (dir, AppState::new(controller))
}

fn account(state: &AppState, name: &str) -> String {
    create_account_impl(
        state,
        CreateAccountInput {
            name: name.to_owned(),
            cashflow_role: CashflowRoleDto::LiquidCash,
            currency: "USD".to_owned(),
            flags: Some(AccountFlagsDto {
                retirement: false,
                tax_advantaged: false,
                joint: false,
                business: false,
            }),
            opening_balance: None,
            subtype: None,
            idempotency_key: String::new(),
        },
    )
    .expect("account")
    .account_id
}

/// One synthetic transaction: a distinct date and a distinct signed amount.
struct Txn {
    date: NaiveDate,
    minor: i64,
    merchant: String,
}

fn fifty() -> Vec<Txn> {
    let start = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
    (0..ROWS)
        .map(|i| Txn {
            date: start + Duration::days(i as i64),
            minor: -(1_000 + 37 * i as i64),
            merchant: format!("Store {i}"),
        })
        .collect()
}

fn amount(minor: i64) -> String {
    let sign = if minor < 0 { "-" } else { "" };
    format!("{sign}{}.{:02}", minor.abs() / 100, minor.abs() % 100)
}

fn csv(txns: &[Txn]) -> Vec<u8> {
    let mut out = String::from("Date,Description,Amount\n");
    for t in txns {
        out.push_str(&format!("{},{},{}\n", t.date, t.merchant, amount(t.minor)));
    }
    out.into_bytes()
}

/// An OFX 1.x statement of the same transactions, as a bank's QFX/OFX export
/// would carry them: its own FITIDs and upper-case payee names.
fn ofx(txns: &[Txn], fitid_prefix: &str) -> Vec<u8> {
    let mut out = String::from(
        "OFXHEADER:100\nDATA:OFXSGML\nVERSION:102\nSECURITY:NONE\nENCODING:USASCII\n\
         CHARSET:1252\nCOMPRESSION:NONE\nOLDFILEUID:NONE\nNEWFILEUID:NONE\n\n\
         <OFX>\n<BANKMSGSRSV1>\n<STMTTRNRS>\n<TRNUID>1\n<STMTRS>\n<CURDEF>USD\n\
         <BANKACCTFROM>\n<BANKID>999999999\n<ACCTID>000000001\n<ACCTTYPE>CHECKING\n\
         </BANKACCTFROM>\n<BANKTRANLIST>\n<DTSTART>20260401\n<DTEND>20260601\n",
    );
    for (i, t) in txns.iter().enumerate() {
        out.push_str(&format!(
            "<STMTTRN>\n<TRNTYPE>DEBIT\n<DTPOSTED>{}\n<TRNAMT>{}\n<FITID>{fitid_prefix}{i:04}\n<NAME>{}\n",
            t.date.format("%Y%m%d"),
            amount(t.minor),
            t.merchant.to_uppercase()
        ));
    }
    out.push_str(
        "</BANKTRANLIST>\n<LEDGERBAL>\n<BALAMT>0.00\n<DTASOF>20260601\n</LEDGERBAL>\n\
         </STMTRS>\n</STMTTRNRS>\n</BANKMSGSRSV1>\n</OFX>\n",
    );
    out.into_bytes()
}

fn import(state: &AppState, account_id: &str, data: Vec<u8>, filename: &str) -> BatchResultDto {
    import_batch_impl(
        state,
        ImportBatchInput {
            data,
            filename: Some(filename.to_owned()),
            target_account_id: account_id.to_owned(),
            plugin_id: None,
            preset_id: None,
            column_mapping: None,
            default_currency: Some("USD".to_owned()),
            date_format: None,
            idempotency_key: String::new(),
        },
    )
    .expect("import")
}

/// Committed, non-voided ledger transactions on `account_id`.
fn ledger_count(state: &AppState, account_id: &str) -> usize {
    transaction_list_impl(state)
        .expect("transactions")
        .iter()
        .filter(|t| t.account_id == account_id)
        .count()
}

/// Money Inbox items waiting on an imported row.
fn waiting(state: &AppState) -> Vec<MoneyInboxItemDto> {
    money_inbox_list_impl(state)
        .expect("inbox")
        .into_iter()
        .filter(|item| item.item_kind == "imported_waiting_commit")
        .collect()
}

fn key() -> String {
    uuid::Uuid::now_v7().to_string()
}

/// The inbox payload's suspected counterpart id.
fn suspected(item: &MoneyInboxItemDto) -> Option<String> {
    let payload: serde_json::Value = serde_json::from_str(&item.payload_json).unwrap();
    payload["suspected_committed_txn_id"]
        .as_str()
        .map(str::to_owned)
}

#[derive(Clone, Copy, Debug)]
enum Format {
    Csv,
    Ofx,
}

fn file(format: Format, txns: &[Txn]) -> (Vec<u8>, &'static str) {
    match format {
        Format::Csv => (csv(txns), "april.csv"),
        Format::Ofx => (ofx(txns, "BANK"), "april.ofx"),
    }
}

/// AC3: one statement, then an overlapping export of the same 50 transactions
/// in the other format. The cross-source layer flags every overlap for review;
/// nothing is committed twice and nothing is dropped. Then a true duplicate is
/// skipped, a deliberate false positive is imported anyway, and retries of
/// either file change nothing.
fn overlap_then_resolve(first: Format, second: Format) {
    let (_dir, state) = open_state();
    let checking = account(&state, "Checking");
    let txns = fifty();

    let (bytes, name) = file(first, &txns);
    let one = import(&state, &checking, bytes, name);
    assert_eq!(
        (one.status.as_str(), one.committed, one.flagged),
        ("committed", 50, 0)
    );
    let (bytes, name) = file(second, &txns);
    let two = import(&state, &checking, bytes, name);
    assert_eq!(
        (two.status.as_str(), two.committed, two.flagged),
        ("partially_committed", 0, 50),
        "{first:?} then {second:?}"
    );
    assert_eq!(ledger_count(&state, &checking), 50, "not 100");
    let items = waiting(&state);
    assert_eq!(items.len(), 50);

    // Every suspected overlap names exactly one counterpart, and the inbox
    // payload names the same one the review panel shows.
    for item in &items {
        let candidates = duplicate_candidates_impl(&state, item.target_id.clone()).unwrap();
        assert_eq!(candidates.len(), 1, "{}", item.payload_json);
        assert_eq!(
            suspected(item).as_deref(),
            Some(candidates[0].transaction_id.as_str())
        );
        let payload: serde_json::Value = serde_json::from_str(&item.payload_json).unwrap();
        assert_eq!(payload["amount_minor"], candidates[0].amount.minor_units);
        assert_eq!(
            payload["dedupe_reason"],
            "same date and amount already recorded from another source"
        );
    }

    // Retry before any review: both exact re-uploads are already imported, and
    // the inbox does not grow (no flood).
    for format in [first, second] {
        let (bytes, name) = file(format, &txns);
        assert_eq!(
            import(&state, &checking, bytes, name).status,
            "already_imported"
        );
    }
    assert_eq!(waiting(&state).len(), 50);

    // A true duplicate: Skip. No ledger write; the item leaves the inbox.
    skip_staged_transaction_impl(&state, items[0].target_id.clone(), key()).unwrap();
    assert_eq!(ledger_count(&state, &checking), 50);
    assert_eq!(waiting(&state).len(), 49);
    // A deliberate false positive: Import anyway. It posts once.
    import_staged_anyway_impl(&state, items[1].target_id.clone(), key()).unwrap();
    assert_eq!(ledger_count(&state, &checking), 51);
    assert_eq!(waiting(&state).len(), 48);
    // Each resolution is final: repeating it is refused, not re-applied.
    assert!(import_staged_anyway_impl(&state, items[1].target_id.clone(), key()).is_err());
    assert!(skip_staged_transaction_impl(&state, items[1].target_id.clone(), key()).is_err());

    // Retry after review: still already imported; nothing moves.
    for format in [first, second] {
        let (bytes, name) = file(format, &txns);
        assert_eq!(
            import(&state, &checking, bytes, name).status,
            "already_imported"
        );
    }
    assert_eq!(ledger_count(&state, &checking), 51);
    assert_eq!(waiting(&state).len(), 48);
}

#[test]
fn a_csv_then_an_overlapping_ofx_commits_fifty_and_flags_fifty() {
    overlap_then_resolve(Format::Csv, Format::Ofx);
}

#[test]
fn an_ofx_then_an_overlapping_csv_commits_fifty_and_flags_fifty() {
    overlap_then_resolve(Format::Ofx, Format::Csv);
}

/// The approved boundary of the file-hash skip (owner decision on yl5): a file
/// whose rows were ALL dismissed (skipped) no longer counts as imported, so
/// re-uploading it is a deliberate restore and its rows are reviewed again.
#[test]
fn a_fully_dismissed_import_can_be_uploaded_again() {
    let (_dir, state) = open_state();
    let checking = account(&state, "Checking");
    let txns = fifty();
    import(&state, &checking, csv(&txns), "april.csv");
    import(&state, &checking, ofx(&txns, "BANK"), "april.ofx");
    for item in waiting(&state) {
        skip_staged_transaction_impl(&state, item.target_id, key()).unwrap();
    }
    assert!(waiting(&state).is_empty());

    let again = import(&state, &checking, ofx(&txns, "BANK"), "april.ofx");
    assert_eq!((again.committed, again.flagged), (0, 50));
    assert_eq!(
        ledger_count(&state, &checking),
        50,
        "still never committed twice"
    );
}

/// AC4 negative controls: none of these is a duplicate, so all commit.
#[test]
fn other_accounts_signs_and_dates_are_not_duplicates() {
    let (_dir, state) = open_state();
    let checking = account(&state, "Checking");
    let savings = account(&state, "Savings");
    let txns = fifty();
    import(&state, &checking, csv(&txns), "april.csv");

    // The same export into a different account.
    let other_account = import(&state, &savings, ofx(&txns, "BANK"), "april.ofx");
    assert_eq!((other_account.committed, other_account.flagged), (50, 0));

    // The same dates with the opposite sign (refunds of the same amounts).
    let flipped: Vec<Txn> = fifty()
        .into_iter()
        .map(|t| Txn {
            minor: -t.minor,
            ..t
        })
        .collect();
    let refunds = import(&state, &checking, ofx(&flipped, "REFUND"), "refunds.ofx");
    assert_eq!((refunds.committed, refunds.flagged), (50, 0));

    // The same amounts one day later: there is no ±1-day fuzzy rule.
    let shifted: Vec<Txn> = fifty()
        .into_iter()
        .map(|t| Txn {
            date: t.date + Duration::days(1),
            ..t
        })
        .collect();
    let next_day = import(&state, &checking, ofx(&shifted, "NEXT"), "next-day.ofx");
    assert_eq!((next_day.committed, next_day.flagged), (50, 0));

    assert_eq!(ledger_count(&state, &checking), 150);
    assert_eq!(ledger_count(&state, &savings), 50);
    assert!(waiting(&state).is_empty());
}

/// AC4: within one source, distinct provider ids mean distinct transactions —
/// including two identical purchases on the same day. A CSV carries no ids, so
/// its identical rows are flagged for review, never silently dropped.
#[test]
fn same_source_rows_with_distinct_ids_and_identical_same_day_purchases_are_kept() {
    let (_dir, state) = open_state();
    let checking = account(&state, "Checking");
    let txns = fifty();
    import(&state, &checking, ofx(&txns, "FIRST"), "april.ofx");
    let reissued = import(
        &state,
        &checking,
        ofx(&txns, "SECOND"),
        "april-reissued.ofx",
    );
    assert_eq!((reissued.committed, reissued.flagged), (50, 0));

    let coffee = || Txn {
        date: NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
        minor: -450,
        merchant: "Coffee".to_owned(),
    };
    let two_coffees = import(
        &state,
        &checking,
        ofx(&[coffee(), coffee()], "CUP"),
        "cups.ofx",
    );
    assert_eq!((two_coffees.committed, two_coffees.flagged), (2, 0));

    // A different day and price, so only the two CSV rows can match each other.
    let tea = || Txn {
        date: NaiveDate::from_ymd_opt(2026, 7, 2).unwrap(),
        minor: -375,
        merchant: "Tea".to_owned(),
    };
    let csv_coffees = import(&state, &checking, csv(&[tea(), tea()]), "teas.csv");
    assert_eq!(csv_coffees.skipped_rows, 0, "neither row is dropped");
    assert_eq!(
        (csv_coffees.committed, csv_coffees.flagged),
        (1, 1),
        "the identical CSV row waits for review"
    );
    assert_eq!(ledger_count(&state, &checking), 103);
    assert_eq!(waiting(&state).len(), 1);
}

/// AC4: a voided counterpart is not a counterpart.
#[test]
fn a_voided_counterpart_does_not_flag_the_overlap() {
    let (_dir, state) = open_state();
    let checking = account(&state, "Checking");
    let txns = fifty();
    import(&state, &checking, csv(&txns), "april.csv");
    let first = transaction_list_impl(&state)
        .unwrap()
        .into_iter()
        .find(|t| t.amount.minor_units == txns[0].minor)
        .expect("row 0 committed");
    void_transaction_impl(&state, first.transaction_id, key()).unwrap();

    let overlap = import(&state, &checking, ofx(&txns, "BANK"), "april.ofx");
    assert_eq!((overlap.committed, overlap.flagged), (1, 49));
    assert_eq!(waiting(&state).len(), 49);
    for item in waiting(&state) {
        let candidates = duplicate_candidates_impl(&state, item.target_id.clone()).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_ne!(candidates[0].amount.minor_units, txns[0].minor);
    }
}

/// Several committed transactions share the flagged row's identity: the panel
/// gets them all, in the same account only.
#[test]
fn a_flag_with_several_committed_counterparts_lists_them_all() {
    let (_dir, state) = open_state();
    let checking = account(&state, "Checking");
    let txns = fifty();
    import(&state, &checking, csv(&txns[..1]), "one.csv");
    // A later export repeats row 0 (plus a new row): row 0 is flagged.
    let later = import(&state, &checking, csv(&txns[..2]), "two.csv");
    assert_eq!((later.committed, later.flagged), (1, 1));
    let flagged = waiting(&state).remove(0);
    let candidates = duplicate_candidates_impl(&state, flagged.target_id.clone()).unwrap();
    assert_eq!(candidates.len(), 1);
    // A same-source repeat is caught by the fingerprint layer, which now names
    // its counterpart just as the cross-source layer does.
    assert!(flagged
        .payload_json
        .contains("duplicate of an already-committed transaction"));
    assert_eq!(
        suspected(&flagged).as_deref(),
        Some(candidates[0].transaction_id.as_str())
    );
    // The user says it is a second real purchase.
    import_staged_anyway_impl(&state, flagged.target_id, key()).unwrap();

    // A third export repeats row 0 again: two committed counterparts now.
    let third = import(&state, &checking, csv(&txns[..3]), "three.csv");
    assert_eq!((third.committed, third.flagged), (1, 2));
    let row0 = waiting(&state)
        .into_iter()
        .find(|item| {
            let candidates = duplicate_candidates_impl(&state, item.target_id.clone()).unwrap();
            candidates.len() == 2
        })
        .expect("row 0's flag lists both committed copies");
    assert!(suspected(&row0).is_some());
}
