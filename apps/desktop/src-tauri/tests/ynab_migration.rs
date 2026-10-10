//! personal-cfo-tulv AC #3: "Move from YNAB" end to end through the same IPC
//! entry points the import dialog uses — `list_source_presets` (the "Import
//! from YNAB" entry), `import_preview_columns` / `import_preview_accounts`
//! (what the dialog reads before asking where each YNAB account goes), then
//! `import_batch` with that account map — into a fresh encrypted test vault.
//! The file is the YNAB preset's own synthesized fixture, never a real export.

use app_lib::ipc::commands::{
    account_balance_impl, create_account_impl, import_batch_impl, import_preview_accounts_impl,
    import_preview_columns_impl, imported_transaction_fields_impl, list_source_presets_impl,
    money_inbox_list_impl, transaction_page_impl,
};
use app_lib::ipc::dto::{
    AccountFlagsDto, CashflowRoleDto, CreateAccountInput, ImportAccountMapEntryDto,
    ImportBatchInput, TransactionPageInput, TransactionRowDto,
};
use app_lib::ipc::IpcError;
use app_lib::AppState;
use finance_kernel::VaultController;
use tempfile::TempDir;

fn open_state() -> (TempDir, AppState) {
    let dir = TempDir::new().expect("temp dir");
    let mut controller = VaultController::open(dir.path().join("vault.db"));
    controller.create(b"test-key").expect("create vault");
    (dir, AppState::new(controller))
}

fn create_account(state: &AppState, name: &str, role: CashflowRoleDto) -> String {
    create_account_impl(
        state,
        CreateAccountInput {
            name: name.to_owned(),
            cashflow_role: role,
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
    .unwrap()
    .account_id
}

fn fixture() -> Vec<u8> {
    finance_kernel::preset_by_id("ynab")
        .expect("the YNAB preset is linked into the app")
        .fixture_csv()
        .as_bytes()
        .to_vec()
}

const FILENAME: &str = "YNAB Export - Household as of 2026-06-30.csv";

fn rows(state: &AppState, account_id: &str) -> Vec<TransactionRowDto> {
    transaction_page_impl(
        state,
        TransactionPageInput {
            query: None,
            account_ids: vec![account_id.to_owned()],
            with_balances: false,
            category_id: None,
            tag_id: None,
            recurring_event_id: None,
            from_date: None,
            to_date: None,
            unreviewed_only: false,
            sort: "oldest".to_owned(),
            limit: 100,
            offset: 0,
        },
    )
    .unwrap()
    .rows
}

fn ynab_import(
    state: &AppState,
    account_map: Option<Vec<ImportAccountMapEntryDto>>,
    target_account_id: Option<String>,
) -> Result<app_lib::ipc::dto::BatchResultDto, IpcError> {
    import_batch_impl(
        state,
        ImportBatchInput {
            data: fixture(),
            filename: Some(FILENAME.to_owned()),
            target_account_id,
            account_map,
            plugin_id: None,
            preset_id: Some("ynab".to_owned()),
            column_mapping: None,
            default_currency: Some("USD".to_owned()),
            date_format: None,
            idempotency_key: String::new(),
        },
    )
}

fn entry(source: &str, account_id: Option<&String>) -> ImportAccountMapEntryDto {
    ImportAccountMapEntryDto {
        source_account: source.to_owned(),
        account_id: account_id.cloned(),
    }
}

#[test]
fn a_ynab_register_export_imports_into_the_accounts_the_user_mapped() {
    let (_dir, state) = open_state();
    let checking = create_account(&state, "Everyday Checking", CashflowRoleDto::LiquidCash);
    let card = create_account(&state, "Rewards Card", CashflowRoleDto::CreditFacility);

    // The "Import from YNAB" entry, and the importer it routes through.
    let ynab = list_source_presets_impl(&state)
        .into_iter()
        .find(|p| p.id == "ynab")
        .expect("Import from YNAB is offered");
    assert_eq!(ynab.importer_id.as_deref(), Some("ynab-register"));
    assert!(
        !ynab.help_published,
        "the guide stays draft (owner, 2026-10-04)"
    );

    // What the dialog reads first: the headers (all the preset's columns
    // present, so no mapping step) and the YNAB accounts in the file.
    let headers = import_preview_columns_impl(
        &state,
        fixture(),
        Some(FILENAME.to_owned()),
        ynab.importer_id.clone(),
    )
    .unwrap();
    for column in [
        "Account",
        "Date",
        "Payee",
        "Category Group",
        "Outflow",
        "Inflow",
    ] {
        assert!(
            headers.iter().any(|h| h == column),
            "{column} in {headers:?}"
        );
    }
    let accounts = import_preview_accounts_impl(
        &state,
        fixture(),
        Some(FILENAME.to_owned()),
        None,
        Some("ynab".to_owned()),
        None,
    )
    .unwrap();
    assert_eq!(accounts, vec!["Checking", "Credit Card", "Savings"]);

    // The user maps two of the three YNAB accounts and leaves Savings out.
    let result = ynab_import(
        &state,
        Some(vec![
            entry("Checking", Some(&checking)),
            entry("Credit Card", Some(&card)),
            entry("Savings", None),
        ]),
        None,
    )
    .unwrap();

    // 11 readable rows; Savings' one row is left out, not committed elsewhere.
    assert_eq!(result.skipped_unmapped, 1);
    assert_eq!(result.staged, 10);
    // The exact duplicate pair: one commits, the other surfaces for review.
    assert_eq!(result.committed, 9);
    assert_eq!(result.flagged, 1);
    assert_eq!(result.status, "partially_committed");
    // Starting balance (zero), the foreign-currency row, the bad date.
    assert_eq!(result.skipped_rows, 3);
    let notes: Vec<_> = result
        .warnings
        .iter()
        .filter(|w| !w.skipped)
        .map(|w| w.message.as_str())
        .collect();
    assert_eq!(notes.len(), 3, "{notes:?}");
    assert!(notes[0].starts_with("2 rows are each a transfer between YNAB accounts"));
    assert!(notes[1].starts_with("2 rows are each a line of a YNAB split"));
    assert!(notes[2].starts_with("1 row is uncleared in YNAB"));

    // Signs and routing: Checking holds rent, paycheck, the transfer out, the
    // fee and the hardware purchase; the card holds the split lines, one
    // coffee and the uncleared gas purchase.
    let checking_amounts: Vec<i64> = rows(&state, &checking)
        .iter()
        .map(|r| r.amount.minor_units)
        .collect();
    assert_eq!(
        checking_amounts,
        vec![-120_000, 250_000, -30_000, -1_200, -4_200]
    );
    assert_eq!(
        account_balance_impl(&state, checking.clone())
            .unwrap()
            .unwrap()
            .minor_units,
        94_600
    );
    let card_rows = rows(&state, &card);
    let mut card_amounts: Vec<i64> = card_rows
        .iter()
        .map(|r| r.amount.minor_units.abs())
        .collect();
    card_amounts.sort_unstable();
    assert_eq!(card_amounts, vec![450, 2_420, 3_875, 6_000]);
    // Every card row moves the same direction (purchases, all outflows).
    assert!(
        card_rows
            .iter()
            .all(|r| r.amount.minor_units.signum() == card_rows[0].amount.minor_units.signum()),
        "{card_rows:?}"
    );

    // Categories are suggestions only: no YNAB "Group: Category" names a
    // DohFlow category, so nothing is assigned; the source's own category
    // is still visible in the row's imported details.
    let checking_rows = rows(&state, &checking);
    assert!(checking_rows.iter().all(|r| r.category_id.is_none()));
    let rent = &checking_rows[0];
    let imported = imported_transaction_fields_impl(&state, rent.transaction_id.clone())
        .unwrap()
        .expect("imported details");
    let field = |key: &str| {
        imported
            .fields
            .iter()
            .find(|f| f.key == key)
            .map(|f| f.value.clone())
    };
    assert_eq!(field("Category Group").as_deref(), Some("Bills"));
    assert_eq!(field("Category").as_deref(), Some("Rent"));

    // The duplicate surfaces in review (the Money Inbox), against the
    // account it belongs to; every new row is also there as unreviewed.
    let inbox = money_inbox_list_impl(&state).unwrap();
    let held: Vec<_> = inbox
        .iter()
        .filter(|item| item.item_kind == "imported_waiting_commit")
        .collect();
    assert_eq!(held.len(), 1, "{inbox:?}");
    assert!(held[0]
        .payload_json
        .contains("\"description\":\"Coffee Bar\""));
    assert!(held[0]
        .payload_json
        .contains(&format!("\"account_id\":\"{card}\"")));
    assert_eq!(
        inbox
            .iter()
            .filter(|item| item.item_kind == "unreviewed_transaction")
            .count(),
        9
    );

    // Importing the same file again adds nothing (file-level dedupe).
    let again = ynab_import(&state, Some(vec![entry("Checking", Some(&checking))]), None).unwrap();
    assert_eq!(again.status, "already_imported");
    assert_eq!(rows(&state, &checking).len(), 5);
}

#[test]
fn a_ynab_import_never_lands_an_unmapped_account_s_rows_somewhere_else() {
    let (_dir, state) = open_state();
    let checking = create_account(&state, "Everyday Checking", CashflowRoleDto::LiquidCash);

    // Only Checking is mapped: the card's and Savings' rows are left out.
    let result = ynab_import(&state, Some(vec![entry("Checking", Some(&checking))]), None).unwrap();
    assert_eq!(result.staged, 5);
    assert_eq!(result.committed, 5);
    assert_eq!(result.skipped_unmapped, 6);
    assert_eq!(rows(&state, &checking).len(), 5);
}

#[test]
fn an_import_names_exactly_one_destination() {
    let (_dir, state) = open_state();
    let checking = create_account(&state, "Everyday Checking", CashflowRoleDto::LiquidCash);

    for (map, target) in [
        (
            Some(vec![entry("Checking", Some(&checking))]),
            Some(checking.clone()),
        ),
        (None, None),
    ] {
        assert!(matches!(
            ynab_import(&state, map, target),
            Err(IpcError::Validation(_))
        ));
    }
    // A malformed account id in the map is refused before anything parses.
    assert!(matches!(
        ynab_import(
            &state,
            Some(vec![entry("Checking", Some(&"not-a-uuid".to_owned()))]),
            None
        ),
        Err(IpcError::Validation(_))
    ));
    assert!(rows(&state, &checking).is_empty());
}
