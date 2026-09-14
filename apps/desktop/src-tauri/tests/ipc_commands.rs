//! Integration tests for the IPC command surface (personal-cfo-40t).
//!
//! These drive the real `*_impl` functions against a real temp-vault
//! [`Kernel`] — no UI, no webview, no mocked DB (per the project DoD). They
//! exercise the happy path, the locked-vault path, and input-validation
//! failures, proving the typed command → kernel → DTO round-trip end to end.

use app_lib::ipc::commands::{
    account_balance_impl, account_count_impl, account_list_impl, account_unexplained_impl,
    account_view_impl, acknowledge_no_reset_warning_impl, archive_account_impl,
    archive_category_impl, archive_income_source_impl, archive_recurring_bill_impl,
    archive_scenario_impl, assert_balance_impl, attach_source_record_impl, base_currency_impl,
    cash_availability_impl, cash_tiers_impl, category_list_impl, clone_scenario_impl,
    convert_unexplained_to_transaction_impl, create_account_impl, create_category_impl,
    create_forecast_assumption_impl, create_income_source_impl, create_manual_future_entry_impl,
    create_recurring_bill_impl, create_recurring_transfer_impl, create_scenario_impl,
    create_source_batch_impl, delete_forecast_assumption_impl, delete_income_source_impl,
    delete_manual_future_entry_impl, delete_recurring_bill_impl, delete_recurring_transfer_impl,
    delete_scenario_impl, dismiss_inbox_item_impl, forecast_assumption_list_impl,
    forecast_readiness_impl, future_cash_by_account_impl, future_cash_forecast_impl,
    household_timezone_impl, import_batch_impl, import_staged_anyway_impl, income_source_list_impl,
    manual_future_entry_list_impl, mark_inbox_reviewed_bulk_impl, money_inbox_list_impl,
    move_category_impl, rebuild_read_models_impl, recategorize_transaction_impl,
    record_transaction_impl, record_transfer_impl, recurring_bill_list_impl,
    recurring_transfer_list_impl, reinstate_account_impl, reinstate_category_impl,
    restore_income_source_impl, restore_recurring_bill_impl, scenario_list_impl,
    set_account_subtype_impl, set_base_currency_impl, set_household_timezone_impl,
    set_minimum_cash_floor_impl, set_note_impl, skip_staged_transaction_impl,
    snooze_inbox_item_impl, transaction_list_impl, transaction_page_impl,
    transaction_rows_by_ids_impl, update_account_impl, update_batch_state_impl,
    update_category_impl, update_income_source_impl, update_manual_future_entry_impl,
    update_recurring_bill_impl, update_scenario_impl, vault_health_impl,
};
use app_lib::ipc::dto::{
    AccountFlagsDto, AccountSeriesDto, AssertBalanceInput, AttachSourceRecordInput,
    CashflowRoleDto, CloneScenarioInput, CreateAccountInput, CreateCategoryInput,
    CreateForecastAssumptionInput, CreateIncomeSourceInput, CreateManualFutureEntryInput,
    CreateRecurringBillInput, CreateRecurringTransferInput, CreateScenarioInput,
    CreateSourceBatchInput, ImportBatchInput, MoneyDto, MoveCategoryInput, RecordTransactionInput,
    RecordTransferInput, TransactionPageInput, UpdateAccountInput, UpdateBatchStateInput,
    UpdateCategoryInput, UpdateIncomeSourceInput, UpdateManualFutureEntryInput,
    UpdateRecurringBillInput, UpdateScenarioInput,
};
use app_lib::ipc::IpcError;
use app_lib::AppState;
use finance_kernel::VaultController;
use tempfile::TempDir;

/// An `AppState` backed by a fresh, unlocked encrypted vault in a temp dir —
/// built through the real `VaultController` create flow (personal-cfo-8v2).
fn open_state() -> (TempDir, AppState) {
    let dir = TempDir::new().expect("temp dir");
    let mut controller = VaultController::open(dir.path().join("vault.db"));
    controller.create(b"test-key").expect("create vault");
    (dir, AppState::new(controller))
}

fn create_input(name: &str) -> CreateAccountInput {
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
    }
}

#[test]
fn create_then_read_account_round_trips() {
    let (_dir, state) = open_state();

    assert_eq!(account_count_impl(&state).unwrap(), 0);

    let created = create_account_impl(&state, create_input("Checking")).unwrap();
    assert!(!created.mutation.replayed);

    assert_eq!(account_count_impl(&state).unwrap(), 1);

    let view = account_view_impl(&state, created.account_id.clone())
        .unwrap()
        .expect("account exists");
    assert_eq!(view.id, created.account_id);
    assert_eq!(view.name, "Checking");
    assert_eq!(view.cashflow_role, "liquid_cash");
    assert!(view.active);
}

#[test]
fn opening_balance_is_reflected_in_balance() {
    let (_dir, state) = open_state();
    let mut input = create_input("Savings");
    input.opening_balance = Some(MoneyDto {
        minor_units: 15_000,
        currency: "USD".to_owned(),
    });

    let created = create_account_impl(&state, input).unwrap();
    let balance = account_balance_impl(&state, created.account_id)
        .unwrap()
        .expect("balance present");
    assert_eq!(balance.minor_units, 15_000);
    assert_eq!(balance.currency, "USD");
}

#[test]
fn update_archive_reinstate_flow() {
    let (_dir, state) = open_state();
    let created = create_account_impl(&state, create_input("Old Name")).unwrap();
    let id = created.account_id;

    update_account_impl(
        &state,
        UpdateAccountInput {
            account_id: id.clone(),
            name: "New Name".to_owned(),
            idempotency_key: String::new(),
        },
    )
    .unwrap();
    let view = account_view_impl(&state, id.clone()).unwrap().unwrap();
    assert_eq!(view.name, "New Name");

    archive_account_impl(&state, id.clone(), String::new()).unwrap();
    assert!(
        !account_view_impl(&state, id.clone())
            .unwrap()
            .unwrap()
            .active
    );

    reinstate_account_impl(&state, id.clone(), String::new()).unwrap();
    assert!(account_view_impl(&state, id).unwrap().unwrap().active);
}

#[test]
fn subtype_and_cash_tiers_round_trip() {
    let (_dir, state) = open_state();

    // A checking account classified at creation.
    let mut checking = create_input("Checking");
    checking.subtype = Some("checking".to_owned());
    checking.opening_balance = Some(MoneyDto {
        minor_units: 120_000,
        currency: "USD".to_owned(),
    });
    let checking_id = create_account_impl(&state, checking).unwrap().account_id;

    // A savings account classified after the fact (the existing-vault path).
    let mut savings = create_input("Savings");
    savings.opening_balance = Some(MoneyDto {
        minor_units: 500_000,
        currency: "USD".to_owned(),
    });
    let savings_id = create_account_impl(&state, savings).unwrap().account_id;
    set_account_subtype_impl(
        &state,
        savings_id.clone(),
        Some("savings".to_owned()),
        String::new(),
    )
    .unwrap();

    // The read model carries the subtype tokens.
    let list = account_list_impl(&state).unwrap();
    let subtype_of = |id: &str| list.iter().find(|a| a.id == id).unwrap().subtype.clone();
    assert_eq!(subtype_of(&checking_id).as_deref(), Some("checking"));
    assert_eq!(subtype_of(&savings_id).as_deref(), Some("savings"));

    // The cash tiers reflect the classification: checking → spendable, savings →
    // reserve, net = both.
    let tiers = cash_tiers_impl(&state).unwrap();
    assert_eq!(tiers.spendable.minor_units, 120_000);
    assert_eq!(tiers.reserve.minor_units, 500_000);
    assert_eq!(tiers.net.minor_units, 620_000);

    // A subtype from the wrong role is rejected at the IPC boundary.
    assert!(set_account_subtype_impl(
        &state,
        checking_id,
        Some("credit_card".to_owned()),
        String::new(),
    )
    .is_err());
}

#[test]
fn cash_availability_reflects_balances_and_the_floor() {
    let (_dir, state) = open_state();
    let mut checking = create_input("Checking");
    checking.subtype = Some("checking".to_owned());
    checking.opening_balance = Some(MoneyDto {
        minor_units: 500_000,
        currency: "USD".to_owned(),
    });
    create_account_impl(&state, checking).unwrap();

    // Set the household floor.
    set_minimum_cash_floor_impl(
        &state,
        MoneyDto {
            minor_units: 100_000,
            currency: "USD".to_owned(),
        },
    )
    .unwrap();

    let avail = cash_availability_impl(&state).unwrap();
    assert_eq!(avail.accounts.len(), 1);
    let acct = &avail.accounts[0];
    assert_eq!(acct.ledger.minor_units, 500_000);
    assert_eq!(acct.pending.minor_units, 0); // manual mode
    assert_eq!(acct.available.minor_units, 500_000);
    // The invariant (ADR 0029): ledger >= available >= headroom.
    assert!(acct.ledger.minor_units >= acct.available.minor_units);
    assert!(acct.available.minor_units >= acct.headroom.minor_units);
    // The floor round-trips; net available is the balance.
    assert_eq!(avail.floor.minor_units, 100_000);
    assert_eq!(avail.net_available.minor_units, 500_000);

    // A negative floor is rejected.
    assert!(set_minimum_cash_floor_impl(
        &state,
        MoneyDto {
            minor_units: -1,
            currency: "USD".to_owned(),
        },
    )
    .is_err());
}

#[test]
fn forecast_readiness_reflects_coverage() {
    let (_dir, state) = open_state();

    // Empty vault: no liquid account is the hard-gate 0 (ADR 0026 §13).
    let empty = forecast_readiness_impl(&state).unwrap();
    assert_eq!(empty.score, 0);
    assert_eq!(empty.factors.len(), 7); // + categorization + spending_history + recurrence_actuals + backtest_mape (ADR 0026 §8/§18)

    // Account + income + bills → full coverage. With no recorded transactions the explained
    // + categorization factors stay neutral, and forecast-accuracy is neutral (nothing to
    // backtest yet), so the score clears the neutral floor regardless of the wall clock
    // (coverage 0.25 + explained 0.10 + categorization 0.15 + backtest 0.10 of the weighted
    // sum; freshness + spending-history + recurrence-actuals are 0 without a balance
    // assertion / spend history / realized actuals).
    create_account_impl(&state, create_input("Checking")).unwrap();
    create_income_source_impl(&state, income_input("biweekly")).unwrap();
    create_recurring_bill_impl(&state, bill_input("monthly", "rent_mortgage")).unwrap();

    let r = forecast_readiness_impl(&state).unwrap();
    let coverage = r.factors.iter().find(|f| f.key == "coverage").unwrap();
    let explained = r.factors.iter().find(|f| f.key == "explained").unwrap();
    assert_eq!(coverage.score, 100);
    assert_eq!(explained.score, 100); // no transactions recorded → neutral
    assert!(
        r.score >= 55,
        "full coverage should score at least the neutral floor, got {}",
        r.score
    );
}

#[test]
fn vault_health_reports_a_healthy_fresh_vault() {
    let (_dir, state) = open_state();
    create_account_impl(&state, create_input("Checking")).unwrap();

    // A freshly created, unlocked vault passes every coherence check (personal-cfo-n9w).
    let health = vault_health_impl(&state).unwrap();
    assert!(health.writer_healthy);
    assert!(health.wal_configured);
    assert!(health.schema_coherent);
    assert!(health.integrity_ok);
    assert!(health.attachments_consistent); // no attachments → trivially consistent
    assert!(health.is_healthy);
}

#[test]
fn rebuild_read_models_runs_and_leaves_the_vault_healthy() {
    let (_dir, state) = open_state();
    create_account_impl(&state, create_input("Checking")).unwrap();

    // The recovery repair runs end-to-end (personal-cfo-5ivp) and is non-destructive:
    // the vault stays healthy and read models stay current afterwards.
    rebuild_read_models_impl(&state).unwrap();
    let health = vault_health_impl(&state).unwrap();
    assert!(health.read_models_current);
    assert!(health.is_healthy);
}

#[test]
fn record_transaction_moves_the_balance_and_validates_input() {
    let (_dir, state) = open_state();
    let created = create_account_impl(&state, create_input("Checking")).unwrap();
    let id = created.account_id;

    let income = RecordTransactionInput {
        account_id: id.clone(),
        amount: MoneyDto {
            minor_units: 15_000,
            currency: "USD".to_owned(),
        },
        occurred_at: "2026-06-07T00:00:00Z".to_owned(),
        idempotency_key: String::new(),
    };
    record_transaction_impl(&state, income).unwrap();
    assert_eq!(
        account_balance_impl(&state, id.clone())
            .unwrap()
            .unwrap()
            .minor_units,
        15_000
    );

    let expense = RecordTransactionInput {
        account_id: id.clone(),
        amount: MoneyDto {
            minor_units: -4_000,
            currency: "USD".to_owned(),
        },
        occurred_at: "2026-06-07T12:00:00Z".to_owned(),
        idempotency_key: String::new(),
    };
    record_transaction_impl(&state, expense).unwrap();
    assert_eq!(
        account_balance_impl(&state, id.clone())
            .unwrap()
            .unwrap()
            .minor_units,
        11_000
    );

    // A malformed timestamp is a validation error.
    let bad = RecordTransactionInput {
        account_id: id,
        amount: MoneyDto {
            minor_units: 100,
            currency: "USD".to_owned(),
        },
        occurred_at: "not-a-date".to_owned(),
        idempotency_key: String::new(),
    };
    assert!(matches!(
        record_transaction_impl(&state, bad).unwrap_err(),
        IpcError::Validation(_)
    ));
}

/// personal-cfo-4d8.24.2.1: record_transaction returns the id the new transaction was
/// stored under, and it is the REAL persisted id (the inline add-transaction flow attaches
/// category/tags/notes to it), not a throwaway.
#[test]
fn record_transaction_returns_the_persisted_transaction_id() {
    let (_dir, state) = open_state();
    let id = create_account_impl(&state, create_input("Checking"))
        .unwrap()
        .account_id;

    let result = record_transaction_impl(
        &state,
        RecordTransactionInput {
            account_id: id,
            amount: MoneyDto {
                minor_units: 15_000,
                currency: "USD".to_owned(),
            },
            occurred_at: "2026-06-07T00:00:00Z".to_owned(),
            idempotency_key: String::new(),
        },
    )
    .unwrap();

    assert!(!result.result.replayed);
    // A well-formed UUID string.
    let returned = uuid::Uuid::parse_str(&result.transaction_id).expect("valid uuid");
    // And it is the id of the row that was actually persisted.
    let txns = transaction_list_impl(&state).unwrap();
    assert_eq!(txns.len(), 1);
    assert_eq!(txns[0].transaction_id, returned.to_string());
}

#[test]
fn transaction_list_returns_recent_transactions_newest_first() {
    let (_dir, state) = open_state();
    let id = create_account_impl(&state, create_input("Checking"))
        .unwrap()
        .account_id;

    record_transaction_impl(
        &state,
        RecordTransactionInput {
            account_id: id.clone(),
            amount: MoneyDto {
                minor_units: 15_000,
                currency: "USD".to_owned(),
            },
            occurred_at: "2026-06-01T00:00:00Z".to_owned(),
            idempotency_key: String::new(),
        },
    )
    .unwrap();
    record_transaction_impl(
        &state,
        RecordTransactionInput {
            account_id: id,
            amount: MoneyDto {
                minor_units: -4_000,
                currency: "USD".to_owned(),
            },
            occurred_at: "2026-06-05T00:00:00Z".to_owned(),
            idempotency_key: String::new(),
        },
    )
    .unwrap();

    let txns = transaction_list_impl(&state).unwrap();
    assert_eq!(txns.len(), 2);
    // Newest first, with the account name + signed amount for display.
    assert_eq!(txns[0].amount.minor_units, -4_000);
    assert_eq!(txns[0].account_name, "Checking");
    assert_eq!(txns[1].amount.minor_units, 15_000);
}

/// personal-cfo-3fdd.1: the paged read filters server-side and windows the full
/// set — three pages of 10 tile 25 rows without overlap, and a note-text query
/// narrows across ALL history (not just the current page).
#[test]
fn transaction_page_filters_and_paginates_through_all_history() {
    let (_dir, state) = open_state();
    let id = create_account_impl(&state, create_input("Checking"))
        .unwrap()
        .account_id;

    // 25 expenses, one per day of June 2026.
    for day in 1..=25u32 {
        record_transaction_impl(
            &state,
            RecordTransactionInput {
                account_id: id.clone(),
                amount: MoneyDto {
                    minor_units: -(i64::from(day)) * 100,
                    currency: "USD".to_owned(),
                },
                occurred_at: format!("2026-06-{day:02}T12:00:00Z"),
                idempotency_key: String::new(),
            },
        )
        .unwrap();
    }
    // A searchable note on the June 5 row (index 20 of the newest-first list).
    let noted_id = transaction_list_impl(&state).unwrap()[20]
        .transaction_id
        .clone();
    set_note_impl(
        &state,
        noted_id.clone(),
        Some("team coffee".to_owned()),
        String::new(),
    )
    .unwrap();

    let input = |offset: u32| TransactionPageInput {
        query: None,
        account_ids: Vec::new(),
        with_balances: false,
        category_id: None,
        tag_id: None,
        recurring_event_id: None,
        from_date: None,
        to_date: None,
        unreviewed_only: false,
        sort: "newest".to_owned(),
        limit: 10,
        offset,
    };

    // Three pages of 10 tile the 25 rows: 10 + 10 + 5, one shared total.
    let mut seen = Vec::new();
    for offset in [0, 10, 20] {
        let page = transaction_page_impl(&state, input(offset)).unwrap();
        assert_eq!(page.total, 25);
        assert_eq!(page.rows.len(), if offset == 20 { 5 } else { 10 });
        seen.extend(page.rows.into_iter().map(|r| r.transaction_id));
    }
    assert_eq!(seen.len(), 25, "pages tile the set");
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), 25, "pages never overlap");

    // The query filter reaches the note on page 3 of the unfiltered order.
    let hit = transaction_page_impl(
        &state,
        TransactionPageInput {
            query: Some("COFFEE".to_owned()),
            ..input(0)
        },
    )
    .unwrap();
    assert_eq!(hit.total, 1);
    assert_eq!(hit.rows[0].transaction_id, noted_id);

    // A bad sort token is a validation error (Rust stays authoritative).
    let bad = TransactionPageInput {
        sort: "spicy".to_owned(),
        ..input(0)
    };
    assert!(matches!(
        transaction_page_impl(&state, bad).unwrap_err(),
        IpcError::Validation(_)
    ));
}

/// npoe: a transfer between two accounts moves both balances through the IPC and
/// leaves aggregate cash unchanged.
#[test]
fn record_transfer_moves_both_balances_through_ipc() {
    let (_dir, state) = open_state();
    let open = |name: &str, opening: i64| {
        let mut input = create_input(name);
        input.opening_balance = Some(MoneyDto {
            minor_units: opening,
            currency: "USD".to_owned(),
        });
        create_account_impl(&state, input).unwrap().account_id
    };
    let checking = open("Checking", 100_000);
    let savings = open("Savings", 20_000);

    record_transfer_impl(
        &state,
        RecordTransferInput {
            source_account_id: checking.clone(),
            dest_account_id: savings.clone(),
            amount: MoneyDto {
                minor_units: 30_000,
                currency: "USD".to_owned(),
            },
            occurred_at: "2026-06-05T00:00:00Z".to_owned(),
            idempotency_key: String::new(),
        },
    )
    .unwrap();

    let balance = |id: &str| {
        account_balance_impl(&state, id.to_owned())
            .unwrap()
            .unwrap()
            .minor_units
    };
    assert_eq!(balance(&checking), 70_000, "source debited");
    assert_eq!(balance(&savings), 50_000, "destination credited");

    // Same-account transfer is rejected at the boundary.
    assert!(record_transfer_impl(
        &state,
        RecordTransferInput {
            source_account_id: checking.clone(),
            dest_account_id: checking,
            amount: MoneyDto {
                minor_units: 1_000,
                currency: "USD".to_owned(),
            },
            occurred_at: "2026-06-05T00:00:00Z".to_owned(),
            idempotency_key: String::new(),
        },
    )
    .is_err());
}

/// npoe: a recurring transfer round-trips through the IPC — create, list, delete.
#[test]
fn recurring_transfer_round_trips_through_ipc() {
    let (_dir, state) = open_state();
    let checking = create_account_impl(&state, create_input("Checking"))
        .unwrap()
        .account_id;
    let savings = create_account_impl(&state, create_input("Savings"))
        .unwrap()
        .account_id;

    let created = create_recurring_transfer_impl(
        &state,
        CreateRecurringTransferInput {
            source_account_id: checking,
            dest_account_id: savings,
            amount: MoneyDto {
                minor_units: 30_000,
                currency: "USD".to_owned(),
            },
            frequency: "monthly".to_owned(),
            anchor_date: "2026-07-01".to_owned(),
            idempotency_key: String::new(),
        },
    )
    .unwrap();

    let list = recurring_transfer_list_impl(&state).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].source_account_name, "Checking");
    assert_eq!(list[0].dest_account_name, "Savings");
    assert_eq!(list[0].amount.minor_units, 30_000);

    delete_recurring_transfer_impl(&state, created.recurring_transfer_id, String::new()).unwrap();
    assert!(recurring_transfer_list_impl(&state).unwrap().is_empty());
}

/// r52x: an account with an old balance assertion surfaces a stale-balance Money
/// Inbox item through the IPC (ADR 0014 §7).
#[test]
fn stale_balance_surfaces_in_the_money_inbox_through_ipc() {
    let (_dir, state) = open_state();
    let account = create_account_impl(&state, create_input("Checking"))
        .unwrap()
        .account_id;
    // Assert a balance dated far in the past → past the liquid 14-day threshold.
    assert_balance_impl(
        &state,
        AssertBalanceInput {
            account_id: account.clone(),
            amount: MoneyDto {
                minor_units: 100_000,
                currency: "USD".to_owned(),
            },
            as_of_date: "2020-01-01".to_owned(),
        },
    )
    .unwrap();

    let stale: Vec<_> = money_inbox_list_impl(&state)
        .unwrap()
        .into_iter()
        .filter(|i| i.item_kind == "stale_balance")
        .collect();
    assert_eq!(stale.len(), 1, "the stale account is flagged");
    assert_eq!(stale[0].target_id, account);
}

/// ci71: a Money Inbox item can be snoozed (hidden until a date) and dismissed
/// (hidden across rebuilds) through the IPC (ADR 0014 §7 / personal-cfo-3d3).
#[test]
fn snooze_and_dismiss_inbox_items_through_ipc() {
    let (_dir, state) = open_state();
    import_with_one_flagged(&state);
    let flagged = || {
        money_inbox_list_impl(&state)
            .unwrap()
            .into_iter()
            .find(|i| i.item_kind == "imported_waiting_commit")
            .map(|i| i.item_id)
    };
    let item_id = flagged().expect("an imported-waiting-commit item");

    // Snooze far in the future → hidden from the default list.
    snooze_inbox_item_impl(
        &state,
        item_id.clone(),
        "2999-01-01".to_owned(),
        String::new(),
    )
    .unwrap();
    assert!(flagged().is_none(), "a snoozed item is hidden");

    // Dismiss → stays hidden even after a full read-model rebuild (the 3d3 property).
    dismiss_inbox_item_impl(
        &state,
        item_id.clone(),
        "not_relevant".to_owned(),
        String::new(),
    )
    .unwrap();
    rebuild_read_models_impl(&state).unwrap();
    assert!(flagged().is_none(), "a dismissed item survives a rebuild");

    // An unknown dismiss reason is rejected at the boundary.
    assert!(
        dismiss_inbox_item_impl(&state, item_id, "nonsense".to_owned(), String::new()).is_err()
    );
}

/// dyy4: converting the unexplained adjustment records a transaction that zeroes
/// the plug through the IPC (ADR 0027 §8).
#[test]
fn convert_unexplained_to_transaction_through_ipc() {
    let (_dir, state) = open_state();
    let account = create_account_impl(&state, create_input("Checking"))
        .unwrap()
        .account_id;
    assert_balance_impl(
        &state,
        AssertBalanceInput {
            account_id: account.clone(),
            amount: MoneyDto {
                minor_units: 50_000,
                currency: "USD".to_owned(),
            },
            as_of_date: "2026-06-01".to_owned(),
        },
    )
    .unwrap();
    let plug = |state: &AppState| {
        account_unexplained_impl(state, account.clone())
            .unwrap()
            .map_or(0, |m| m.minor_units)
    };
    assert_eq!(plug(&state), 50_000, "the asserted balance is unexplained");

    convert_unexplained_to_transaction_impl(&state, account.clone(), String::new()).unwrap();
    assert_eq!(plug(&state), 0, "converting fully explains the balance");

    // Nothing left to convert.
    assert!(convert_unexplained_to_transaction_impl(&state, account, String::new()).is_err());
}

/// bac: a transaction's category can be set and cleared through the IPC, and the
/// transactions list reflects it (ADR 0030).
#[test]
fn recategorize_a_transaction_through_ipc() {
    let (_dir, state) = open_state();
    let account_id = create_account_impl(&state, create_input("Checking"))
        .unwrap()
        .account_id;
    record_transaction_impl(
        &state,
        RecordTransactionInput {
            account_id,
            amount: MoneyDto {
                minor_units: -4_000,
                currency: "USD".to_owned(),
            },
            occurred_at: "2026-06-05T00:00:00Z".to_owned(),
            idempotency_key: String::new(),
        },
    )
    .unwrap();
    let txn_id = transaction_list_impl(&state).unwrap()[0]
        .transaction_id
        .clone();
    let category_id = create_category_impl(
        &state,
        CreateCategoryInput {
            parent_id: None,
            name: "Groceries".to_owned(),
            category_type: "expense".to_owned(),
            color: None,
            icon: None,
            idempotency_key: String::new(),
        },
    )
    .unwrap()
    .category_id;

    let category_of =
        |state: &AppState| transaction_list_impl(state).unwrap()[0].category_id.clone();
    assert_eq!(category_of(&state), None, "uncategorized to start");

    recategorize_transaction_impl(
        &state,
        txn_id.clone(),
        Some(category_id.clone()),
        String::new(),
    )
    .unwrap();
    assert_eq!(category_of(&state), Some(category_id));

    recategorize_transaction_impl(&state, txn_id, None, String::new()).unwrap();
    assert_eq!(category_of(&state), None, "cleared");
}

#[test]
fn account_list_returns_all_accounts_ordered_with_balances() {
    let (_dir, state) = open_state();
    assert!(account_list_impl(&state).unwrap().is_empty());

    let mut zebra = create_input("Zebra Savings");
    zebra.opening_balance = Some(MoneyDto {
        minor_units: 25_000,
        currency: "USD".to_owned(),
    });
    create_account_impl(&state, zebra).unwrap();
    create_account_impl(&state, create_input("Apple Checking")).unwrap();

    let list = account_list_impl(&state).unwrap();
    assert_eq!(list.len(), 2);
    // Ordered by name.
    assert_eq!(list[0].name, "Apple Checking");
    assert_eq!(list[1].name, "Zebra Savings");
    assert_eq!(list[1].balance.minor_units, 25_000);
    assert_eq!(list[0].balance.minor_units, 0);
}

#[test]
fn commands_on_locked_vault_return_vault_locked() {
    // A controller pointed at a path with no vault classifies as NoVault, so the
    // kernel is absent and account commands report the locked state.
    let dir = TempDir::new().expect("temp dir");
    let state = AppState::new(VaultController::open(dir.path().join("absent.db")));

    let err = create_account_impl(&state, create_input("Nope")).unwrap_err();
    assert!(matches!(err, IpcError::VaultLocked));

    let err = account_count_impl(&state).unwrap_err();
    assert!(matches!(err, IpcError::VaultLocked));
}

#[test]
fn bad_input_is_a_validation_error() {
    let (_dir, state) = open_state();

    // Empty name.
    let err = create_account_impl(&state, create_input("   ")).unwrap_err();
    assert!(matches!(err, IpcError::Validation(_)));

    // Unsupported currency.
    let mut input = create_input("Foreign");
    input.currency = "XYZ".to_owned();
    let err = create_account_impl(&state, input).unwrap_err();
    assert!(matches!(err, IpcError::Validation(_)));

    // Malformed account id.
    let err = account_view_impl(&state, "not-a-uuid".to_owned()).unwrap_err();
    assert!(matches!(err, IpcError::Validation(_)));
}

fn income_input(frequency: &str) -> CreateIncomeSourceInput {
    CreateIncomeSourceInput {
        name: "Acme Corp".to_owned(),
        net_amount: MoneyDto {
            minor_units: 250_000,
            currency: "USD".to_owned(),
        },
        frequency: frequency.to_owned(),
        anchor_date: "2026-06-05".to_owned(),
        deposit_account_id: None,
        idempotency_key: String::new(),
    }
}

#[test]
fn income_source_create_then_list_round_trips() {
    let (_dir, state) = open_state();
    create_income_source_impl(&state, income_input("biweekly")).unwrap();

    let list = income_source_list_impl(&state).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, "Acme Corp");
    assert_eq!(list[0].frequency, "biweekly");
    assert_eq!(list[0].net_amount.minor_units, 250_000);
    assert!(list[0].next_pay_date.is_some());
}

#[test]
fn income_source_bad_frequency_or_date_is_validation_error() {
    let (_dir, state) = open_state();
    let err = create_income_source_impl(&state, income_input("fortnightly")).unwrap_err();
    assert!(matches!(err, IpcError::Validation(_)));

    let mut bad_date = income_input("monthly");
    bad_date.anchor_date = "06/05/2026".to_owned();
    let err = create_income_source_impl(&state, bad_date).unwrap_err();
    assert!(matches!(err, IpcError::Validation(_)));
}

#[test]
fn income_source_edit_archive_restore_delete_round_trips() {
    let (_dir, state) = open_state();
    create_income_source_impl(&state, income_input("biweekly")).unwrap();
    let id = income_source_list_impl(&state).unwrap()[0].id.clone();

    // Edit the source via the typed input.
    update_income_source_impl(
        &state,
        UpdateIncomeSourceInput {
            income_source_id: id.clone(),
            name: "Globex".to_owned(),
            net_amount: MoneyDto {
                minor_units: 400_000,
                currency: "USD".to_owned(),
            },
            frequency: "monthly".to_owned(),
            anchor_date: "2026-06-01".to_owned(),
            deposit_account_id: None,
            idempotency_key: String::new(),
        },
    )
    .unwrap();
    let after_edit = &income_source_list_impl(&state).unwrap()[0];
    assert_eq!(after_edit.name, "Globex");
    assert_eq!(after_edit.net_amount.minor_units, 400_000);
    assert_eq!(after_edit.frequency, "monthly");
    assert!(after_edit.active && after_edit.archived_at.is_none());

    // Archive -> still listed, inactive + dated.
    archive_income_source_impl(&state, id.clone(), String::new()).unwrap();
    let archived = &income_source_list_impl(&state).unwrap()[0];
    assert!(!archived.active && archived.archived_at.is_some());

    // Restore -> active again.
    restore_income_source_impl(&state, id.clone(), String::new()).unwrap();
    assert!(income_source_list_impl(&state).unwrap()[0].active);

    // Delete -> gone.
    delete_income_source_impl(&state, id, String::new()).unwrap();
    assert!(income_source_list_impl(&state).unwrap().is_empty());
}

fn bill_input(frequency: &str, bill_type: &str) -> CreateRecurringBillInput {
    CreateRecurringBillInput {
        name: "Rent".to_owned(),
        amount: MoneyDto {
            minor_units: 180_000,
            currency: "USD".to_owned(),
        },
        bill_type: bill_type.to_owned(),
        frequency: frequency.to_owned(),
        anchor_date: "2026-07-01".to_owned(),
        autopay_account_id: None,
        autopay: None,
        description: None,
        source_merchant_key: None,
        category_id: None,
        tag_ids: Vec::new(),
        idempotency_key: String::new(),
    }
}

#[test]
fn future_cash_by_account_attributes_flows_and_rolls_up_tiers() {
    let (_dir, state) = open_state();

    // Two liquid accounts with subtypes + opening balances.
    let mut checking_in = create_input("Checking");
    checking_in.subtype = Some("checking".to_owned());
    checking_in.opening_balance = Some(MoneyDto {
        minor_units: 100_000,
        currency: "USD".to_owned(),
    });
    let checking = create_account_impl(&state, checking_in).unwrap().account_id;

    let mut savings_in = create_input("Savings");
    savings_in.subtype = Some("savings".to_owned());
    savings_in.opening_balance = Some(MoneyDto {
        minor_units: 500_000,
        currency: "USD".to_owned(),
    });
    let savings = create_account_impl(&state, savings_in).unwrap().account_id;

    // Income deposits to checking; the bill autopays from checking.
    let mut income = income_input("biweekly");
    income.deposit_account_id = Some(checking.clone());
    create_income_source_impl(&state, income).unwrap();
    let mut bill = bill_input("monthly", "rent_mortgage");
    bill.autopay_account_id = Some(checking.clone());
    create_recurring_bill_impl(&state, bill).unwrap();
    // An account-agnostic manual entry → the Unallocated series. Dated relative to
    // the wall clock: this impl has no clock seam, and a fixed date went stale once
    // real time passed it — the entry left the window and Unallocated vanished
    // (caught 2026-07-20; masked earlier by a gate pipeline swallowing the exit code).
    let gift_date = (chrono::Utc::now().date_naive() + chrono::Days::new(5)).to_string();
    create_manual_future_entry_impl(
        &state,
        CreateManualFutureEntryInput {
            amount: MoneyDto {
                minor_units: 50_000,
                currency: "USD".to_owned(),
            },
            date: gift_date,
            label: "Gift".to_owned(),
            account_id: None,
        },
    )
    .unwrap();

    let multi = future_cash_by_account_impl(&state, 90, Vec::new()).unwrap();

    let series = |id: Option<&str>| {
        multi
            .accounts
            .iter()
            .find(|s| s.account_id.as_deref() == id)
            .unwrap()
    };
    // Tiers follow the subtypes; the manual entry lands in Unallocated.
    assert_eq!(series(Some(&checking)).tier, "spendable");
    assert_eq!(series(Some(&savings)).tier, "reserve");
    assert_eq!(series(None).tier, "unallocated");
    // Attribution: checking carries the income + bill; savings carries nothing.
    let events = |s: &AccountSeriesDto| s.days.iter().map(|d| d.events.len()).sum::<usize>();
    assert!(events(series(Some(&checking))) > 0);
    assert_eq!(events(series(Some(&savings))), 0);

    // Group rollups: net = spendable + reserve + unallocated on the final day, and
    // net equals the sum of the per-account series (internal reconciliation).
    let group = |tier: &str| multi.groups.iter().find(|g| g.tier == tier).unwrap();
    let last = group("net").closings.len() - 1;
    let g_final = |tier: &str| group(tier).closings[last].closing.p50.minor_units;
    let net_final = g_final("net");
    assert_eq!(
        g_final("spendable") + g_final("reserve") + g_final("unallocated"),
        net_final,
    );
    let sum_final: i64 = multi
        .accounts
        .iter()
        .map(|s| s.days[last].closing.p50.minor_units)
        .sum();
    assert_eq!(sum_final, net_final);
}

#[test]
fn recurring_bill_create_then_list_round_trips() {
    let (_dir, state) = open_state();
    create_recurring_bill_impl(&state, bill_input("monthly", "rent_mortgage")).unwrap();

    let list = recurring_bill_list_impl(&state).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, "Rent");
    assert_eq!(list[0].bill_type, "rent_mortgage");
    assert_eq!(list[0].frequency, "monthly");
    assert_eq!(list[0].amount.minor_units, 180_000);
    assert!(list[0].next_due_date.is_some());
}

#[test]
fn recurring_bill_bad_frequency_date_or_type_is_validation_error() {
    let (_dir, state) = open_state();
    let err =
        create_recurring_bill_impl(&state, bill_input("fortnightly", "rent_mortgage")).unwrap_err();
    assert!(matches!(err, IpcError::Validation(_)));

    let mut bad_date = bill_input("monthly", "rent_mortgage");
    bad_date.anchor_date = "07/01/2026".to_owned();
    let err = create_recurring_bill_impl(&state, bad_date).unwrap_err();
    assert!(matches!(err, IpcError::Validation(_)));

    let err = create_recurring_bill_impl(&state, bill_input("monthly", "not_a_type")).unwrap_err();
    assert!(matches!(err, IpcError::Validation(_)));
}

#[test]
fn recurring_bill_update_and_delete_round_trip() {
    let (_dir, state) = open_state();

    // Create, then read back the bill's id (no description yet).
    create_recurring_bill_impl(&state, bill_input("monthly", "subscription")).unwrap();
    let created = recurring_bill_list_impl(&state).unwrap();
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].description, None);
    let bill_id = created[0].id.clone();

    // Edit every field, including the new description (which is trimmed).
    update_recurring_bill_impl(
        &state,
        UpdateRecurringBillInput {
            bill_id: bill_id.clone(),
            name: "Streaming".to_owned(),
            amount: MoneyDto {
                minor_units: 2_299,
                currency: "USD".to_owned(),
            },
            bill_type: "subscription".to_owned(),
            frequency: "annual".to_owned(),
            anchor_date: "2026-09-15".to_owned(),
            autopay_account_id: None,
            autopay: None,
            description: Some("  4K family plan  ".to_owned()),
            idempotency_key: String::new(),
        },
    )
    .unwrap();

    let edited = recurring_bill_list_impl(&state).unwrap();
    assert_eq!(edited.len(), 1);
    assert_eq!(edited[0].name, "Streaming");
    assert_eq!(edited[0].frequency, "annual");
    assert_eq!(edited[0].amount.minor_units, 2_299);
    assert_eq!(edited[0].anchor_date, "2026-09-15");
    assert_eq!(edited[0].description.as_deref(), Some("4K family plan"));

    // Delete it; the list goes empty.
    delete_recurring_bill_impl(&state, bill_id, String::new()).unwrap();
    assert!(recurring_bill_list_impl(&state).unwrap().is_empty());
}

#[test]
fn update_or_delete_missing_recurring_bill_is_an_error() {
    let (_dir, state) = open_state();
    let missing = "0190d000-0000-7000-8000-0000000000ff".to_owned();

    let update = UpdateRecurringBillInput {
        bill_id: missing.clone(),
        name: "Ghost".to_owned(),
        amount: MoneyDto {
            minor_units: 1_000,
            currency: "USD".to_owned(),
        },
        bill_type: "other".to_owned(),
        frequency: "monthly".to_owned(),
        anchor_date: "2026-07-01".to_owned(),
        autopay_account_id: None,
        autopay: None,
        description: None,
        idempotency_key: String::new(),
    };
    // Unknown bill → the write fails (no row to rewrite).
    assert!(update_recurring_bill_impl(&state, update.clone()).is_err());

    // A malformed id fails input parsing at the boundary.
    let mut bad = update;
    bad.bill_id = "not-a-uuid".to_owned();
    assert!(matches!(
        update_recurring_bill_impl(&state, bad).unwrap_err(),
        IpcError::Validation(_)
    ));

    // Deleting an unknown bill is likewise an error.
    assert!(delete_recurring_bill_impl(&state, missing, String::new()).is_err());
}

#[test]
fn base_currency_defaults_then_persists() {
    let (_dir, state) = open_state();

    // An unset base currency defaults to USD.
    assert_eq!(base_currency_impl(&state).unwrap(), "USD");

    // Set, then read back — the code is stored canonically (lowercase upcased).
    set_base_currency_impl(&state, "eur".to_owned()).unwrap();
    assert_eq!(base_currency_impl(&state).unwrap(), "EUR");

    // An unsupported code is a validation error and leaves the value unchanged.
    let err = set_base_currency_impl(&state, "GBP".to_owned()).unwrap_err();
    assert!(matches!(err, IpcError::Validation(_)));
    assert_eq!(base_currency_impl(&state).unwrap(), "EUR");
}

/// personal-cfo-q329: the `household_timezone` / `set_household_timezone` IPC round trip,
/// mirroring `base_currency_defaults_then_persists` above. The DST-correctness and
/// household-today-shift behavior of the write itself is covered at the db-worker layer
/// (`set_household_timezone_persists_and_shifts_household_today`); this proves the IPC
/// pass-through carries both the value and the validation error through correctly.
#[test]
fn household_timezone_defaults_then_persists() {
    let (_dir, state) = open_state();

    // A fresh vault starts at the documented UTC default (ADR 0021).
    assert_eq!(household_timezone_impl(&state).unwrap(), "UTC");

    set_household_timezone_impl(&state, "America/Los_Angeles".to_owned()).unwrap();
    assert_eq!(
        household_timezone_impl(&state).unwrap(),
        "America/Los_Angeles"
    );

    // An unrecognized IANA name is a validation error and leaves the value unchanged.
    let err = set_household_timezone_impl(&state, "Not/A_Real_Zone".to_owned()).unwrap_err();
    assert!(matches!(err, IpcError::Validation(_)));
    assert_eq!(
        household_timezone_impl(&state).unwrap(),
        "America/Los_Angeles"
    );
}

#[test]
fn archive_and_restore_recurring_bill_round_trip() {
    let (_dir, state) = open_state();
    create_recurring_bill_impl(&state, bill_input("monthly", "subscription")).unwrap();
    let created = recurring_bill_list_impl(&state).unwrap();
    assert_eq!(created.len(), 1);
    assert!(created[0].active);
    assert!(created[0].archived_at.is_none());
    assert!(!created[0].created_at.is_empty());
    let bill_id = created[0].id.clone();

    // Archive: retained + still listed, but inactive with an archived_at timestamp.
    archive_recurring_bill_impl(&state, bill_id.clone(), String::new()).unwrap();
    let archived = recurring_bill_list_impl(&state).unwrap();
    assert_eq!(archived.len(), 1);
    assert!(!archived[0].active);
    assert!(archived[0].archived_at.is_some());

    // Restore: back to active, archived_at cleared.
    restore_recurring_bill_impl(&state, bill_id, String::new()).unwrap();
    let restored = recurring_bill_list_impl(&state).unwrap();
    assert!(restored[0].active);
    assert!(restored[0].archived_at.is_none());
}

/// End-to-end smoke test of the first-run user journey through the **real** IPC
/// command surface (the `rtez` first-playable gate): acknowledge the no-reset
/// warning, then seed an account, a transaction, a paycheck and a bill, and read
/// the Future Cash forecast the dashboard renders. No UI/webview, but every step
/// is the real `*_impl` against a real encrypted vault — the whole backend the
/// app drives, exercised the way a first run does.
#[test]
fn first_run_journey_produces_a_forecast() {
    let (_dir, state) = open_state();

    // The onboarding no-reset-warning acknowledgement is recorded (personal-cfo-n7bo).
    acknowledge_no_reset_warning_impl(&state).unwrap();

    // A liquid checking account with a $2,500 opening balance.
    let mut checking = create_input("Checking");
    checking.opening_balance = Some(MoneyDto {
        minor_units: 250_000,
        currency: "USD".to_owned(),
    });
    let checking_id = create_account_impl(&state, checking).unwrap().account_id;
    assert_eq!(account_count_impl(&state).unwrap(), 1);

    // A $45 manual expense.
    record_transaction_impl(
        &state,
        RecordTransactionInput {
            account_id: checking_id.clone(),
            amount: MoneyDto {
                minor_units: -4_500,
                currency: "USD".to_owned(),
            },
            occurred_at: "2026-06-07T00:00:00Z".to_owned(),
            idempotency_key: String::new(),
        },
    )
    .unwrap();

    // A biweekly paycheck deposited to checking.
    create_income_source_impl(
        &state,
        CreateIncomeSourceInput {
            name: "Acme paycheck".to_owned(),
            net_amount: MoneyDto {
                minor_units: 300_000,
                currency: "USD".to_owned(),
            },
            frequency: "biweekly".to_owned(),
            anchor_date: "2026-06-05".to_owned(),
            deposit_account_id: Some(checking_id.clone()),
            idempotency_key: String::new(),
        },
    )
    .unwrap();
    assert_eq!(income_source_list_impl(&state).unwrap().len(), 1);

    // A monthly rent bill.
    create_recurring_bill_impl(
        &state,
        CreateRecurringBillInput {
            name: "Rent".to_owned(),
            amount: MoneyDto {
                minor_units: 180_000,
                currency: "USD".to_owned(),
            },
            bill_type: "rent_mortgage".to_owned(),
            frequency: "monthly".to_owned(),
            anchor_date: "2026-06-01".to_owned(),
            autopay_account_id: None,
            autopay: None,
            description: None,
            source_merchant_key: None,
            category_id: None,
            tag_ids: Vec::new(),
            idempotency_key: String::new(),
        },
    )
    .unwrap();
    assert_eq!(recurring_bill_list_impl(&state).unwrap().len(), 1);

    // The dashboard's Future Cash forecast over 30 days.
    let forecast = future_cash_forecast_impl(&state, 30, Vec::new()).unwrap();
    assert_eq!(forecast.horizon_days, 30);
    assert_eq!(
        forecast.days.len(),
        30,
        "one row per day across the horizon"
    );

    // It starts from liquid cash on hand (opening balance net of the expense).
    let on_hand = account_balance_impl(&state, checking_id)
        .unwrap()
        .unwrap()
        .minor_units;
    assert_eq!(on_hand, 250_000 - 4_500);
    assert_eq!(forecast.starting_balance.minor_units, on_hand);

    // Scheduled income/bills move the projection: the recurring paycheck and rent
    // both produce occurrences inside the horizon, so the end differs from today.
    let last = forecast.days.last().unwrap();
    assert_ne!(
        last.closing.p50.minor_units, forecast.starting_balance.minor_units,
        "scheduled income/bills should move the 30-day projection"
    );
    // Deterministic Layer-1 → collapsed band (ADR 0026 §1).
    assert_eq!(last.closing.p10, last.closing.p50);
    assert_eq!(last.closing.p50, last.closing.p90);
}

#[test]
fn manual_future_entry_create_list_update_delete_journey() {
    use chrono::{Duration, Utc};

    let (_dir, state) = open_state();
    // A liquid account fixes the forecast currency.
    create_account_impl(&state, create_input("Checking")).unwrap();

    // A future date inside the horizon, regardless of when the test runs.
    let date = (Utc::now() + Duration::days(30))
        .format("%Y-%m-%d")
        .to_string();

    // Create a +$5,000 manual entry.
    let created = create_manual_future_entry_impl(
        &state,
        CreateManualFutureEntryInput {
            amount: MoneyDto {
                minor_units: 500_000,
                currency: "USD".to_owned(),
            },
            date: date.clone(),
            label: "Bonus".to_owned(),
            account_id: None,
        },
    )
    .unwrap();
    assert_eq!(created.label, "Bonus");

    // It lists, and the forecast folds it in (manual_entry source).
    assert_eq!(manual_future_entry_list_impl(&state).unwrap().len(), 1);
    let forecast = future_cash_forecast_impl(&state, 90, Vec::new()).unwrap();
    assert!(
        forecast
            .days
            .iter()
            .flat_map(|d| &d.events)
            .any(|e| e.kind == "manual_entry" && e.amount.minor_units == 500_000),
        "the manual entry is folded into the forecast",
    );

    // Editing supersedes (no silent mutation): still exactly one entry, the new one.
    let updated = update_manual_future_entry_impl(
        &state,
        UpdateManualFutureEntryInput {
            id: created.id.clone(),
            amount: MoneyDto {
                minor_units: 700_000,
                currency: "USD".to_owned(),
            },
            date,
            label: "Bigger bonus".to_owned(),
            account_id: None,
        },
    )
    .unwrap();
    assert_ne!(updated.id, created.id);
    let list = manual_future_entry_list_impl(&state).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].label, "Bigger bonus");

    // Deleting clears it.
    delete_manual_future_entry_impl(&state, updated.id).unwrap();
    assert!(manual_future_entry_list_impl(&state).unwrap().is_empty());
}

#[test]
fn scenario_overlay_create_apply_compare_journey() {
    use chrono::{Duration, Utc};

    let (_dir, state) = open_state();
    acknowledge_no_reset_warning_impl(&state).unwrap();

    // A liquid checking account with $5,000 on hand.
    let mut checking = create_input("Checking");
    checking.opening_balance = Some(MoneyDto {
        minor_units: 500_000,
        currency: "USD".to_owned(),
    });
    create_account_impl(&state, checking).unwrap();

    // Two monthly bills: Rent (the amount-override target) and Streaming (the
    // exclusion target).
    create_recurring_bill_impl(
        &state,
        CreateRecurringBillInput {
            name: "Rent".to_owned(),
            amount: MoneyDto {
                minor_units: 180_000,
                currency: "USD".to_owned(),
            },
            bill_type: "rent_mortgage".to_owned(),
            frequency: "monthly".to_owned(),
            anchor_date: "2026-06-01".to_owned(),
            autopay_account_id: None,
            autopay: None,
            description: None,
            source_merchant_key: None,
            category_id: None,
            tag_ids: Vec::new(),
            idempotency_key: String::new(),
        },
    )
    .unwrap();
    create_recurring_bill_impl(
        &state,
        CreateRecurringBillInput {
            name: "Streaming".to_owned(),
            amount: MoneyDto {
                minor_units: 5_000,
                currency: "USD".to_owned(),
            },
            bill_type: "subscription".to_owned(),
            frequency: "monthly".to_owned(),
            anchor_date: "2026-06-10".to_owned(),
            autopay_account_id: None,
            autopay: None,
            description: None,
            source_merchant_key: None,
            category_id: None,
            tag_ids: Vec::new(),
            idempotency_key: String::new(),
        },
    )
    .unwrap();
    let bills = recurring_bill_list_impl(&state).unwrap();
    let rent_id = bills.iter().find(|b| b.name == "Rent").unwrap().id.clone();
    let streaming_id = bills
        .iter()
        .find(|b| b.name == "Streaming")
        .unwrap()
        .id
        .clone();

    // Create a scenario (starts as a draft).
    let scenario = create_scenario_impl(
        &state,
        CreateScenarioInput {
            name: "Raise + rent hike".to_owned(),
            description: Some("rent up, drop streaming, plus a bonus".to_owned()),
        },
    )
    .unwrap();
    assert_eq!(scenario.status, "draft");
    assert_eq!(scenario_list_impl(&state).unwrap().len(), 1);

    // A date inside the 90-day horizon regardless of when the test runs.
    let bonus_date = (Utc::now() + Duration::days(20))
        .format("%Y-%m-%d")
        .to_string();

    // Addition: a +$5,000 bonus, scenario-scoped.
    create_forecast_assumption_impl(
        &state,
        CreateForecastAssumptionInput {
            kind: "one_time_event".to_owned(),
            scenario_id: Some(scenario.id.clone()),
            target_entity_id: None,
            amount: Some(MoneyDto {
                minor_units: 500_000,
                currency: "USD".to_owned(),
            }),
            date: Some(bonus_date),
            label: Some("Bonus".to_owned()),
            new_amount_minor: None,
            new_anchor_date: None,
            effective_date: None,
            end_date: None,
        },
    )
    .unwrap();

    // Modification: rent up to -$2,500.
    create_forecast_assumption_impl(
        &state,
        CreateForecastAssumptionInput {
            kind: "bill_amount".to_owned(),
            scenario_id: Some(scenario.id.clone()),
            target_entity_id: Some(rent_id.clone()),
            amount: None,
            date: None,
            label: None,
            new_amount_minor: Some(250_000),
            new_anchor_date: None,
            effective_date: None,
            end_date: None,
        },
    )
    .unwrap();

    // Removal: exclude the streaming subscription.
    create_forecast_assumption_impl(
        &state,
        CreateForecastAssumptionInput {
            kind: "exclusion".to_owned(),
            scenario_id: Some(scenario.id.clone()),
            target_entity_id: Some(streaming_id.clone()),
            amount: None,
            date: None,
            label: None,
            new_amount_minor: None,
            new_anchor_date: None,
            effective_date: None,
            end_date: None,
        },
    )
    .unwrap();

    // The scenario has three events; the base has none (events are scoped).
    assert_eq!(
        forecast_assumption_list_impl(&state, Some(scenario.id.clone()))
            .unwrap()
            .len(),
        3,
    );
    assert!(forecast_assumption_list_impl(&state, None)
        .unwrap()
        .is_empty());

    // The scenario forecast reflects all three overlays.
    let scen = future_cash_forecast_impl(&state, 90, vec![scenario.id.clone()]).unwrap();
    let scen_events: Vec<_> = scen.days.iter().flat_map(|d| &d.events).collect();
    assert!(
        scen_events
            .iter()
            .any(|e| e.kind == "manual_entry" && e.amount.minor_units == 500_000),
        "the addition is folded into the scenario forecast",
    );
    assert!(
        scen_events
            .iter()
            .any(|e| e.source_event_id == rent_id && e.amount.minor_units == -250_000),
        "the rent amount is overridden under the scenario",
    );
    assert!(
        scen_events
            .iter()
            .all(|e| e.source_event_id != streaming_id),
        "the streaming bill is excluded under the scenario",
    );

    // The base forecast is unchanged: original rent, streaming present, no bonus.
    let base = future_cash_forecast_impl(&state, 90, Vec::new()).unwrap();
    let base_events: Vec<_> = base.days.iter().flat_map(|d| &d.events).collect();
    assert!(
        base_events
            .iter()
            .any(|e| e.source_event_id == rent_id && e.amount.minor_units == -180_000),
        "base keeps the original rent amount",
    );
    assert!(
        base_events
            .iter()
            .any(|e| e.source_event_id == streaming_id),
        "base keeps the streaming bill",
    );
    assert!(
        base_events.iter().all(|e| e.kind != "manual_entry"),
        "base has no scenario addition",
    );

    // Deleting one assumption clears it (the scenario keeps the other two).
    let bonus = forecast_assumption_list_impl(&state, Some(scenario.id.clone()))
        .unwrap()
        .into_iter()
        .find(|e| e.kind == "one_time_event")
        .unwrap();
    delete_forecast_assumption_impl(&state, bonus.id).unwrap();
    assert_eq!(
        forecast_assumption_list_impl(&state, Some(scenario.id.clone()))
            .unwrap()
            .len(),
        2,
    );

    // Activate then delete the scenario: it is archived and its events stop
    // applying (non-destructive — the row is retained).
    let activated = update_scenario_impl(
        &state,
        UpdateScenarioInput {
            id: scenario.id.clone(),
            status: "active".to_owned(),
            name: None,
        },
    )
    .unwrap();
    assert_eq!(activated.status, "active");

    // vru6: the same command renames when a name is supplied.
    let renamed = update_scenario_impl(
        &state,
        UpdateScenarioInput {
            id: scenario.id.clone(),
            status: "active".to_owned(),
            name: Some("Renamed scenario".to_owned()),
        },
    )
    .unwrap();
    assert_eq!(renamed.name, "Renamed scenario");

    // ADR 0051 §1 — archive KEEPS the overlay and is reversible…
    let before = forecast_assumption_list_impl(&state, Some(scenario.id.clone()))
        .unwrap()
        .len();
    assert!(before > 0, "the journey attached overlay events");
    archive_scenario_impl(&state, scenario.id.clone()).unwrap();
    let archived = scenario_list_impl(&state)
        .unwrap()
        .into_iter()
        .find(|s| s.id == scenario.id)
        .expect("archiving retains the scenario");
    assert_eq!(archived.status, "archived");
    assert_eq!(
        forecast_assumption_list_impl(&state, Some(scenario.id.clone()))
            .unwrap()
            .len(),
        before,
        "archiving keeps the scenario's changes",
    );
    assert_eq!(archived.event_count as usize, before);

    // …and a clone carries a copy of them into a fresh draft (§2).
    let copy_id = clone_scenario_impl(
        &state,
        CloneScenarioInput {
            id: scenario.id.clone(),
            name: "Copy".to_owned(),
        },
    )
    .unwrap();
    let copy = scenario_list_impl(&state)
        .unwrap()
        .into_iter()
        .find(|s| s.id == copy_id)
        .expect("the clone exists");
    assert_eq!(copy.status, "draft");
    assert_eq!(
        copy.event_count as usize, before,
        "the clone copies the overlay"
    );

    // …while delete removes the scenario and its overlay outright.
    delete_scenario_impl(&state, scenario.id.clone()).unwrap();
    assert!(
        scenario_list_impl(&state)
            .unwrap()
            .into_iter()
            .all(|s| s.id != scenario.id),
        "delete removes the scenario",
    );
    assert!(
        forecast_assumption_list_impl(&state, Some(scenario.id.clone()))
            .unwrap()
            .is_empty(),
        "delete removes its events",
    );
    // The clone is independent — deleting the source leaves it untouched.
    assert_eq!(
        forecast_assumption_list_impl(&state, Some(copy_id))
            .unwrap()
            .len(),
        before,
        "a clone survives its source being deleted",
    );
}

#[test]
fn create_forecast_assumption_requires_kind_specific_fields() {
    let (_dir, state) = open_state();

    // A bill_amount override with no target is rejected.
    let err = create_forecast_assumption_impl(
        &state,
        CreateForecastAssumptionInput {
            kind: "bill_amount".to_owned(),
            scenario_id: None,
            target_entity_id: None,
            amount: None,
            date: None,
            label: None,
            new_amount_minor: Some(250_000),
            new_anchor_date: None,
            effective_date: None,
            end_date: None,
        },
    )
    .unwrap_err();
    assert!(matches!(err, IpcError::Validation(_)));

    // An unknown kind is rejected.
    let err = create_forecast_assumption_impl(
        &state,
        CreateForecastAssumptionInput {
            kind: "teleport".to_owned(),
            scenario_id: None,
            target_entity_id: None,
            amount: None,
            date: None,
            label: None,
            new_amount_minor: None,
            new_anchor_date: None,
            effective_date: None,
            end_date: None,
        },
    )
    .unwrap_err();
    assert!(matches!(err, IpcError::Validation(_)));
}

#[test]
fn assert_balance_journey_anchors_and_explains() {
    use chrono::{Duration, Utc};

    let (_dir, state) = open_state();
    // A liquid account with NO opening balance.
    let account_id = create_account_impl(&state, create_input("Checking"))
        .unwrap()
        .account_id;

    // Set the balance directly — no transactions (ADR 0027).
    let as_of = (Utc::now() - Duration::days(10))
        .format("%Y-%m-%d")
        .to_string();
    let result = assert_balance_impl(
        &state,
        AssertBalanceInput {
            account_id: account_id.clone(),
            amount: MoneyDto {
                minor_units: 550_000,
                currency: "USD".to_owned(),
            },
            as_of_date: as_of,
        },
    )
    .unwrap();
    assert_eq!(result.balance.minor_units, 550_000);
    // Nothing explains it yet → fully unexplained.
    assert_eq!(result.unexplained.unwrap().minor_units, 550_000);

    // account_balance + account_unexplained reflect the assertion.
    assert_eq!(
        account_balance_impl(&state, account_id.clone())
            .unwrap()
            .unwrap()
            .minor_units,
        550_000,
    );
    assert_eq!(
        account_unexplained_impl(&state, account_id.clone())
            .unwrap()
            .unwrap()
            .minor_units,
        550_000,
    );

    // A $3,000 deposit dated before the as-of explains part of the gap.
    record_transaction_impl(
        &state,
        RecordTransactionInput {
            account_id: account_id.clone(),
            amount: MoneyDto {
                minor_units: 300_000,
                currency: "USD".to_owned(),
            },
            occurred_at: format!(
                "{}T00:00:00Z",
                (Utc::now() - Duration::days(15)).format("%Y-%m-%d")
            ),
            idempotency_key: String::new(),
        },
    )
    .unwrap();
    assert_eq!(
        account_unexplained_impl(&state, account_id.clone())
            .unwrap()
            .unwrap()
            .minor_units,
        250_000,
    );

    // The forecast starts from the asserted balance — no transactions required.
    let forecast = future_cash_forecast_impl(&state, 30, Vec::new()).unwrap();
    assert_eq!(forecast.starting_balance.minor_units, 550_000);
}

/// personal-cfo-3bb: the ingestion staging surface end to end — open a batch,
/// attach a record (idempotent on re-attach), advance its lifecycle state, and
/// reject an invalid status at the kernel boundary.
#[test]
fn source_batch_ingestion_lifecycle_through_ipc() {
    let (_dir, state) = open_state();

    let batch = create_source_batch_impl(
        &state,
        CreateSourceBatchInput {
            source_type: "csv".to_owned(),
            source_name: Some("statement.csv".to_owned()),
            file_fingerprint: Some("sha256:file".to_owned()),
            parser_version: Some("csv-v1".to_owned()),
            idempotency_key: String::new(),
        },
    )
    .unwrap();
    assert!(!batch.source_batch_id.is_empty());

    let attach = |key: &str| AttachSourceRecordInput {
        source_batch_id: batch.source_batch_id.clone(),
        external_id: Some("row-1".to_owned()),
        source_hash: "sha256:row1".to_owned(),
        normalized_json: "{\"amount\":-1299}".to_owned(),
        parse_confidence_bps: Some(10_000),
        idempotency_key: key.to_owned(),
    };
    let attached = attach_source_record_impl(&state, attach("k1")).unwrap();
    assert!(!attached.source_record_id.is_empty());
    // Re-attaching the same content (distinct command) is content-deduped (no error).
    attach_source_record_impl(&state, attach("k2")).unwrap();

    // Advance the batch to a terminal state.
    update_batch_state_impl(
        &state,
        UpdateBatchStateInput {
            source_batch_id: batch.source_batch_id.clone(),
            status: "committed".to_owned(),
            staged_count: 1,
            committed_count: 1,
            skipped_count: 0,
            idempotency_key: String::new(),
        },
    )
    .unwrap();

    // An invalid lifecycle status is rejected before it touches persistence.
    let bad = update_batch_state_impl(
        &state,
        UpdateBatchStateInput {
            source_batch_id: batch.source_batch_id,
            status: "bogus".to_owned(),
            staged_count: 0,
            committed_count: 0,
            skipped_count: 0,
            idempotency_key: String::new(),
        },
    );
    assert!(bad.is_err());
}

/// personal-cfo-cu8: a real CSV imported end to end through the IPC — the
/// `GenericCsv` plugin (linked + auto-registered) is auto-detected, parsed in the
/// bounded host, then staged + deduped + committed. Row 3 duplicates row 1, so it
/// is flagged; the two unique transactions post to the ledger.
#[test]
fn import_batch_imports_a_csv_through_the_pipeline() {
    let (_dir, state) = open_state();
    let account_id = create_account_impl(&state, create_input("Checking"))
        .unwrap()
        .account_id;

    let csv = "Date,Description,Amount\n\
               2026-06-20,Coffee,-12.99\n\
               2026-06-21,Lunch,-42.00\n\
               2026-06-20,Coffee,-12.99\n";
    let result = import_batch_impl(
        &state,
        ImportBatchInput {
            data: csv.as_bytes().to_vec(),
            filename: Some("statement.csv".to_owned()),
            target_account_id: account_id.clone(),
            plugin_id: None, // auto-detect → GenericCsv
            column_mapping: None,
            default_currency: Some("USD".to_owned()),
            date_format: None,
            idempotency_key: String::new(),
        },
    )
    .unwrap();

    assert_eq!(result.status, "partially_committed");
    assert_eq!(result.committed, 2);
    assert_eq!(result.flagged, 1);

    // The two unique transactions posted (−12.99 + −42.00); the duplicate did not.
    let balance = account_balance_impl(&state, account_id)
        .unwrap()
        .expect("balance present");
    assert_eq!(balance.minor_units, -5499);
}

/// dsq: importing a CSV with an overlapping (duplicate) row flags it, and the
/// flagged row surfaces as a Money Inbox item through the IPC — the end-to-end
/// import → triage path the user sees.
#[test]
fn money_inbox_surfaces_a_flagged_import_through_ipc() {
    let (_dir, state) = open_state();
    let account_id = create_account_impl(&state, create_input("Checking"))
        .unwrap()
        .account_id;

    // A clean inbox before any import.
    assert!(
        money_inbox_list_impl(&state)
            .unwrap()
            .iter()
            .all(|i| i.item_kind != "imported_waiting_commit"),
        "the flagged item is resolved (clean imports remain as unreviewed items)"
    );

    let csv = "Date,Description,Amount\n\
               2026-06-20,Coffee,-12.99\n\
               2026-06-21,Lunch,-42.00\n\
               2026-06-20,Coffee,-12.99\n";
    let result = import_batch_impl(
        &state,
        ImportBatchInput {
            data: csv.as_bytes().to_vec(),
            filename: Some("statement.csv".to_owned()),
            target_account_id: account_id,
            plugin_id: None,
            column_mapping: None,
            default_currency: Some("USD".to_owned()),
            date_format: None,
            idempotency_key: String::new(),
        },
    )
    .unwrap();
    assert_eq!(result.flagged, 1);

    let items = money_inbox_list_impl(&state).unwrap();
    let flagged: Vec<_> = items
        .iter()
        .filter(|i| i.item_kind == "imported_waiting_commit")
        .collect();
    assert_eq!(flagged.len(), 1, "the flagged duplicate is one inbox item");
    let item = flagged[0];
    assert_eq!(item.item_kind, "imported_waiting_commit");
    assert_eq!(item.target_table, "staged_transactions");
    assert!(
        item.payload_json.contains("statement.csv"),
        "payload carries the source filename: {}",
        item.payload_json
    );
    assert!(
        item.payload_json
            .contains("duplicate of an already-committed transaction"),
        "payload carries the dedupe reason: {}",
        item.payload_json
    );
}

/// A CSV whose third row duplicates the first; importing it leaves one flagged
/// inbox item over a `target_account`. Returns the account id + the item id.
fn import_with_one_flagged(state: &AppState) -> (String, String) {
    let account_id = create_account_impl(state, create_input("Checking"))
        .unwrap()
        .account_id;
    let csv = "Date,Description,Amount\n\
               2026-06-20,Coffee,-12.99\n\
               2026-06-21,Lunch,-42.00\n\
               2026-06-20,Coffee,-12.99\n";
    import_batch_impl(
        state,
        ImportBatchInput {
            data: csv.as_bytes().to_vec(),
            filename: Some("statement.csv".to_owned()),
            target_account_id: account_id.clone(),
            plugin_id: None,
            column_mapping: None,
            default_currency: Some("USD".to_owned()),
            date_format: None,
            idempotency_key: String::new(),
        },
    )
    .unwrap();
    let items = money_inbox_list_impl(state).unwrap();
    let flagged: Vec<_> = items
        .iter()
        .filter(|i| i.item_kind == "imported_waiting_commit")
        .collect();
    assert_eq!(flagged.len(), 1, "one flagged duplicate");
    (account_id, flagged[0].item_id.clone())
}

/// asqy: "import anyway" through the IPC commits the flagged duplicate and clears
/// the inbox item.
#[test]
fn import_anyway_resolves_a_flagged_inbox_item_through_ipc() {
    let (_dir, state) = open_state();
    let (account_id, item_id) = import_with_one_flagged(&state);

    import_staged_anyway_impl(&state, item_id, String::new()).unwrap();

    assert!(
        money_inbox_list_impl(&state)
            .unwrap()
            .iter()
            .all(|i| i.item_kind != "imported_waiting_commit"),
        "the flagged item is resolved (clean imports remain as unreviewed items)"
    );
    let balance = account_balance_impl(&state, account_id)
        .unwrap()
        .expect("balance present");
    assert_eq!(
        balance.minor_units, -6798,
        "the duplicate -12.99 is now also committed (-54.99 + -12.99)"
    );
}

/// asqy: "skip" through the IPC clears the inbox item with no ledger change.
#[test]
fn skip_resolves_a_flagged_inbox_item_through_ipc() {
    let (_dir, state) = open_state();
    let (account_id, item_id) = import_with_one_flagged(&state);

    skip_staged_transaction_impl(&state, item_id, String::new()).unwrap();

    assert!(
        money_inbox_list_impl(&state)
            .unwrap()
            .iter()
            .all(|i| i.item_kind != "imported_waiting_commit"),
        "the flagged item is resolved (clean imports remain as unreviewed items)"
    );
    let balance = account_balance_impl(&state, account_id)
        .unwrap()
        .expect("balance present");
    assert_eq!(
        balance.minor_units, -5499,
        "skip leaves the ledger unchanged (only the two unique rows)"
    );
}

/// bac: a user category can be created, archived, and reinstated through the IPC.
#[test]
fn create_archive_reinstate_a_category_through_ipc() {
    let (_dir, state) = open_state();
    let created = create_category_impl(
        &state,
        CreateCategoryInput {
            parent_id: None,
            name: "Coffee Shops".to_owned(),
            category_type: "expense".to_owned(),
            color: None,
            icon: None,
            idempotency_key: String::new(),
        },
    )
    .unwrap();

    let id = created.category_id;
    let find = |state: &AppState| {
        category_list_impl(state)
            .unwrap()
            .into_iter()
            .find(|c| c.id == id)
            .expect("created category present")
    };
    let cat = find(&state);
    assert_eq!(cat.name, "Coffee Shops");
    assert!(!cat.is_system, "a created category is a user category");
    assert!(!cat.archived);

    archive_category_impl(&state, id.clone(), String::new()).unwrap();
    assert!(find(&state).archived);

    reinstate_category_impl(&state, id.clone(), String::new()).unwrap();
    assert!(!find(&state).archived);
}

/// bac/kogu: a user category can be renamed/recolored and re-parented through the IPC;
/// a system category's appearance (color + icon) is editable but its identity is fixed —
/// a submitted rename is ignored and re-parenting is rejected (ADR 0030 amendment).
#[test]
fn update_and_move_a_category_through_ipc() {
    let (_dir, state) = open_state();
    let create = |name: &str| {
        create_category_impl(
            &state,
            CreateCategoryInput {
                parent_id: None,
                name: name.to_owned(),
                category_type: "expense".to_owned(),
                color: None,
                icon: None,
                idempotency_key: String::new(),
            },
        )
        .unwrap()
        .category_id
    };
    let group = create("Hobbies");
    let leaf = create("Guitar");

    let find = |state: &AppState, id: &str| {
        category_list_impl(state)
            .unwrap()
            .into_iter()
            .find(|c| c.id == id)
            .expect("category present")
    };

    // Rename + recolor + set an emoji icon on the user category (personal-cfo-4d8.24.10).
    update_category_impl(
        &state,
        UpdateCategoryInput {
            id: leaf.clone(),
            name: "Guitar Lessons".to_owned(),
            color: Some("#006341".to_owned()),
            icon: Some("🎸".to_owned()),
            idempotency_key: String::new(),
        },
    )
    .unwrap();
    let edited = find(&state, &leaf);
    assert_eq!(edited.name, "Guitar Lessons");
    assert_eq!(edited.color.as_deref(), Some("#006341"));
    assert_eq!(edited.icon.as_deref(), Some("🎸"));

    // Re-parent it under the group.
    move_category_impl(
        &state,
        MoveCategoryInput {
            id: leaf.clone(),
            new_parent_id: Some(group.clone()),
            idempotency_key: String::new(),
        },
    )
    .unwrap();
    assert_eq!(
        find(&state, &leaf).parent_id.as_deref(),
        Some(group.as_str())
    );

    // A system default's APPEARANCE (color + icon) is editable, but a submitted name is
    // ignored — its identity is preserved (ADR 0030 amendment, personal-cfo-kogu).
    let system = category_list_impl(&state)
        .unwrap()
        .into_iter()
        .find(|c| c.is_system)
        .expect("seeded system categories");
    let system_id = system.id.clone();
    update_category_impl(
        &state,
        UpdateCategoryInput {
            id: system_id.clone(),
            name: "Renamed".to_owned(),
            color: Some("#111111".to_owned()),
            icon: Some("🍎".to_owned()),
            idempotency_key: String::new(),
        },
    )
    .unwrap();
    let after = find(&state, &system_id);
    assert_eq!(
        after.name, system.name,
        "a system category's name is preserved despite a submitted rename"
    );
    assert_eq!(
        after.color.as_deref(),
        Some("#111111"),
        "appearance applied"
    );
    assert_eq!(after.icon.as_deref(), Some("🍎"));

    // …but its identity is still fixed: re-parenting a system category is rejected.
    let rejected_move = move_category_impl(
        &state,
        MoveCategoryInput {
            id: system_id.clone(),
            new_parent_id: Some(group.clone()),
            idempotency_key: String::new(),
        },
    );
    assert!(
        rejected_move.is_err(),
        "a system category cannot be re-parented (ADR 0030)"
    );
}

/// bac: the seeded category taxonomy is exposed through the IPC for the UI.
#[test]
fn category_list_returns_the_seeded_taxonomy_through_ipc() {
    let (_dir, state) = open_state();
    let cats = category_list_impl(&state).unwrap();
    assert!(!cats.is_empty(), "the taxonomy is seeded at vault create");
    assert!(
        cats.iter().all(|c| c.is_system),
        "a fresh vault exposes only the system defaults"
    );
}

/// byxe: imported transactions surface their detail in the transactions list, so
/// an imported row shows what it is rather than a bare amount.
#[test]
fn imported_transactions_carry_their_detail_through_ipc() {
    let (_dir, state) = open_state();
    let _ = import_with_one_flagged(&state);

    let rows = transaction_list_impl(&state).unwrap();
    assert_eq!(rows.len(), 2, "the two unique rows committed");
    assert!(
        rows.iter().any(|r| r.memo.as_deref() == Some("Coffee")),
        "the Coffee row shows its description as the memo",
    );
    assert!(
        rows.iter().any(|r| r.memo.as_deref() == Some("Lunch")),
        "the Lunch row shows its description as the memo",
    );
}

/// Feedback 2026-07-03: when the user knows the REAL statement balance, recording it
/// replaces the estimate for that cycle (flagged as actual), drives the projected
/// payment, and clearing it falls back to the estimate.
#[test]
fn recording_a_real_statement_balance_overrides_the_estimate_and_clears_back() {
    use app_lib::ipc::commands::{
        card_statement_forecast_impl, set_card_statement_balance_impl, set_debt_terms_impl,
    };
    use app_lib::ipc::dto::{
        RepaymentPhilosophyDto, SetCardStatementBalanceInput, SetDebtTermsInput,
    };

    let (_dir, state) = open_state();
    let mut card_input = create_input("Sapphire");
    card_input.cashflow_role = CashflowRoleDto::CreditFacility;
    card_input.subtype = Some("credit_card".to_owned());
    let card = create_account_impl(&state, card_input).unwrap().account_id;

    // Owe $400 on the card, pays in full. A statement can only be recorded for a CLOSED
    // cycle (ADR 0039 addendum 2026-07-10 §1), and this impl-level test runs on the wall
    // clock — so derive the terms from today: the statement closed yesterday and is due
    // about a week out (the grace window), making cycles[0] a recordable, closed cycle
    // on every run date.
    use chrono::{Datelike, Days};
    let today = chrono::Utc::now().date_naive();
    let close_day = i64::from((today - Days::new(1)).day().min(28));
    let due_day = i64::from((today + Days::new(7)).day().min(28));
    assert_balance_impl(
        &state,
        AssertBalanceInput {
            account_id: card.clone(),
            amount: MoneyDto {
                minor_units: -40_000,
                currency: "USD".to_owned(),
            },
            as_of_date: "2026-06-20".to_owned(),
        },
    )
    .unwrap();
    set_debt_terms_impl(
        &state,
        SetDebtTermsInput {
            account_id: card.clone(),
            apr_bps: Some(2_400),
            original_principal_minor: None,
            statement_close_day: Some(close_day),
            payment_due_day: Some(due_day),
            grace_period_days: Some(25),
            credit_limit_minor: Some(1_000_000),
            repayment_philosophy: RepaymentPhilosophyDto::PayStatementBalance,
            fixed_amount_minor: None,
            min_payment_percent_bps: None,
            min_payment_floor_minor: None,
            paying_source_account_id: None,
            idempotency_key: String::new(),
        },
    )
    .unwrap();

    let before = card_statement_forecast_impl(&state).unwrap();
    let cycle = before[0].cycles[0].clone();
    assert!(!cycle.statement_is_actual, "estimates are not 'actual'");

    // The real statement arrives: $512.34.
    set_card_statement_balance_impl(
        &state,
        SetCardStatementBalanceInput {
            account_id: card.clone(),
            cycle_close: cycle.close_date.clone(),
            statement_balance_minor: Some(51_234),
            idempotency_key: String::new(),
        },
    )
    .unwrap();
    let after = card_statement_forecast_impl(&state).unwrap();
    let asserted = after[0].cycles[0].clone();
    assert!(
        asserted.statement_is_actual,
        "flagged as the real statement"
    );
    assert_eq!(asserted.statement_balance_minor, 51_234, "override wins");
    assert_eq!(
        asserted.forecast_payment_minor, 51_234,
        "pay-statement philosophy follows the real number"
    );
    // The management list shows the stored row (ADR 0039 addendum 2026-07-10 §1).
    assert_eq!(after[0].stored_statements.len(), 1);
    assert_eq!(after[0].stored_statements[0].close_date, cycle.close_date);
    assert_eq!(
        after[0].stored_statements[0].statement_balance_minor,
        51_234
    );

    // Clearing the assertion falls back to the estimate.
    set_card_statement_balance_impl(
        &state,
        SetCardStatementBalanceInput {
            account_id: card,
            cycle_close: cycle.close_date.clone(),
            statement_balance_minor: None,
            idempotency_key: String::new(),
        },
    )
    .unwrap();
    let cleared = card_statement_forecast_impl(&state).unwrap();
    assert!(!cleared[0].cycles[0].statement_is_actual);
    assert_eq!(
        cleared[0].cycles[0].statement_balance_minor, cycle.statement_balance_minor,
        "back to the estimate"
    );
    assert!(
        cleared[0].stored_statements.is_empty(),
        "clearing removes the stored row from the management list"
    );
}

/// Feedback 2026-07-03: a card with due dates set but NO chosen philosophy and NO minimum
/// terms must still show its payment in the forecast (previously a $0 minimum made the due
/// date silently vanish) — the fallback assumes the statement gets paid.
#[test]
fn unknown_philosophy_without_minimum_terms_still_projects_a_visible_payment() {
    use app_lib::ipc::commands::{card_statement_forecast_impl, set_debt_terms_impl};
    use app_lib::ipc::dto::{RepaymentPhilosophyDto, SetDebtTermsInput};

    let (_dir, state) = open_state();
    let mut card_input = create_input("Costco Visa");
    card_input.cashflow_role = CashflowRoleDto::CreditFacility;
    card_input.subtype = Some("credit_card".to_owned());
    let card = create_account_impl(&state, card_input).unwrap().account_id;
    assert_balance_impl(
        &state,
        AssertBalanceInput {
            account_id: card.clone(),
            amount: MoneyDto {
                minor_units: -25_000,
                currency: "USD".to_owned(),
            },
            as_of_date: "2026-06-20".to_owned(),
        },
    )
    .unwrap();
    set_debt_terms_impl(
        &state,
        SetDebtTermsInput {
            account_id: card.clone(),
            apr_bps: None,
            original_principal_minor: None,
            statement_close_day: Some(12),
            payment_due_day: Some(5),
            grace_period_days: None,
            credit_limit_minor: None,
            repayment_philosophy: RepaymentPhilosophyDto::Unknown,
            fixed_amount_minor: None,
            min_payment_percent_bps: None,
            min_payment_floor_minor: None,
            paying_source_account_id: None,
            idempotency_key: String::new(),
        },
    )
    .unwrap();

    let forecast = card_statement_forecast_impl(&state).unwrap();
    let payment = forecast[0].cycles[0].forecast_payment_minor;
    assert!(
        payment > 0,
        "an unknown philosophy with no minimum terms must not project $0 (got {payment})"
    );
}

/// fr79: an OFX file (Quicken/bank export) auto-detects to the `ofx` plugin (linked +
/// auto-registered beside GenericCsv), parses in the bounded host, and commits through the
/// same stage → dedupe → ledger pipeline. Row 3 repeats row 1's FITID, so it is flagged.
#[test]
fn import_batch_imports_an_ofx_file_through_the_pipeline() {
    let (_dir, state) = open_state();
    let account_id = create_account_impl(&state, create_input("Checking"))
        .unwrap()
        .account_id;

    let ofx = "OFXHEADER:100\r\nDATA:OFXSGML\r\nVERSION:102\r\n\r\n\
               <OFX><BANKMSGSRSV1><STMTTRNRS><STMTRS><CURDEF>USD\
               <BANKTRANLIST>\
               <STMTTRN><TRNTYPE>DEBIT<DTPOSTED>20260620<TRNAMT>-12.99<FITID>TXN-001<NAME>Blue Bottle Coffee</STMTTRN>\
               <STMTTRN><TRNTYPE>DEBIT<DTPOSTED>20260621<TRNAMT>-42.00<FITID>TXN-002<NAME>Corner Bistro<MEMO>Lunch</STMTTRN>\
               <STMTTRN><TRNTYPE>DEBIT<DTPOSTED>20260620<TRNAMT>-12.99<FITID>TXN-001<NAME>Blue Bottle Coffee</STMTTRN>\
               </BANKTRANLIST></STMTRS></STMTTRNRS></BANKMSGSRSV1></OFX>";
    let result = import_batch_impl(
        &state,
        ImportBatchInput {
            data: ofx.as_bytes().to_vec(),
            filename: Some("statement.ofx".to_owned()),
            target_account_id: account_id.clone(),
            plugin_id: None, // auto-detect → the OFX importer
            column_mapping: None,
            default_currency: None, // CURDEF carries the currency
            date_format: None,
            idempotency_key: String::new(),
        },
    )
    .unwrap();

    assert_eq!(result.status, "partially_committed");
    assert_eq!(result.committed, 2, "the two unique FITIDs post");
    assert_eq!(result.flagged, 1, "the repeated FITID flags as a duplicate");

    let balance = account_balance_impl(&state, account_id)
        .unwrap()
        .expect("balance present");
    assert_eq!(balance.minor_units, -5499);

    // The merchant names flow through to the transaction rows (NAME → counterparty path).
    let rows = transaction_list_impl(&state).unwrap();
    assert!(
        rows.iter().any(|r| {
            r.counterparty.as_deref() == Some("Blue Bottle Coffee")
                || r.memo.as_deref() == Some("Blue Bottle Coffee")
        }),
        "the OFX NAME reaches the transaction display fields"
    );
}

/// fr79 review fold: bank ids (OFX FITID) are only unique PER ACCOUNT, so the same
/// fingerprint committed to one account must not flag imports targeting ANOTHER account
/// — the dedupe key is date + amount + merchant + account.
#[test]
fn identical_fingerprints_on_different_accounts_are_not_duplicates() {
    let (_dir, state) = open_state();
    let checking = create_account_impl(&state, create_input("Checking"))
        .unwrap()
        .account_id;
    let savings = create_account_impl(&state, create_input("Savings"))
        .unwrap()
        .account_id;

    let ofx = |pad: &str| {
        format!(
            "OFXHEADER:100\r\n\r\n<OFX><STMTRS><CURDEF>USD<BANKTRANLIST>\
             <STMTTRN><DTPOSTED>20260620<TRNAMT>-25.00<FITID>SHARED-01<NAME>Metro Transit</STMTTRN>\
             </BANKTRANLIST></STMTRS></OFX>{pad}"
        )
    };
    let import = |account: &str, pad: &str| {
        import_batch_impl(
            &state,
            ImportBatchInput {
                data: ofx(pad).into_bytes(),
                filename: Some("statement.ofx".to_owned()),
                target_account_id: account.to_owned(),
                plugin_id: None,
                column_mapping: None,
                default_currency: None,
                date_format: None,
                idempotency_key: String::new(),
            },
        )
        .unwrap()
    };

    let first = import(&checking, "");
    assert_eq!(first.committed, 1);
    // Different bytes (padding) so the file-level dedupe doesn't short-circuit; the
    // row's fingerprint is identical, but it targets a DIFFERENT account → commits.
    let second = import(&savings, "\r\n");
    assert_eq!(
        second.committed, 1,
        "same FITID, different account — not a duplicate"
    );
    assert_eq!(second.flagged, 0);

    // Same account + same fingerprint still flags (a third, differently-padded file).
    let third = import(&checking, "\r\n\r\n");
    assert_eq!(third.committed, 0);
    assert_eq!(third.flagged, 1, "true same-account duplicate still caught");
}

/// An explicit idempotency key makes a retried money write replay, not double-post
/// (personal-cfo-3fdd.5). The frontend now mints one UUID per user action, so a
/// double-fired submit reaches the kernel twice with the SAME key: the second call
/// must report `replayed` and leave the balance reflecting ONE posting.
#[test]
fn record_transaction_with_same_key_replays_instead_of_double_posting() {
    let (_dir, state) = open_state();
    let id = create_account_impl(&state, create_input("Checking"))
        .unwrap()
        .account_id;

    let input = || RecordTransactionInput {
        account_id: id.clone(),
        amount: MoneyDto {
            minor_units: -4_000,
            currency: "USD".to_owned(),
        },
        occurred_at: "2026-06-07T00:00:00Z".to_owned(),
        idempotency_key: "one-user-action".to_owned(),
    };

    let first = record_transaction_impl(&state, input()).unwrap();
    assert!(!first.result.replayed);

    // The "retry": the identical submission, same explicit key.
    let second = record_transaction_impl(&state, input()).unwrap();
    assert!(second.result.replayed, "same key must replay, not re-apply");
    assert_eq!(
        second.result.op_seq, first.result.op_seq,
        "replay returns the original op"
    );
    // personal-cfo-4d8.24.2.1: the replay returns the SAME real id as the first call —
    // never a fresh, never-persisted phantom (it is derived from the idempotency key).
    assert_eq!(
        second.transaction_id, first.transaction_id,
        "replay returns the id the first call actually persisted"
    );

    // The balance reflects exactly ONE posting.
    assert_eq!(
        account_balance_impl(&state, id)
            .unwrap()
            .unwrap()
            .minor_units,
        -4_000
    );
}

/// hbd8: the plaintext CSV export writes every transaction chronologically with
/// resolved names and RFC-4180 escaping — the portability guarantee of the Launch gate.
#[test]
fn export_transactions_csv_writes_all_rows_with_escaping() {
    use app_lib::ipc::commands::{export_transactions_csv_impl, set_note_impl};

    let (dir, state) = open_state();
    let account = create_account_impl(&state, create_input("Checking"))
        .unwrap()
        .account_id;
    for (minor, day) in [(-1_299_i64, "2026-06-20"), (4_200, "2026-06-25")] {
        record_transaction_impl(
            &state,
            RecordTransactionInput {
                account_id: account.clone(),
                amount: MoneyDto {
                    minor_units: minor,
                    currency: "USD".to_owned(),
                },
                occurred_at: format!("{day}T12:00:00Z"),
                idempotency_key: String::new(),
            },
        )
        .unwrap();
    }
    // A note carrying a comma + a quote exercises the RFC-4180 escaping.
    let txn = transaction_list_impl(&state)
        .unwrap()
        .into_iter()
        .find(|r| r.amount.minor_units == -1_299)
        .unwrap();
    set_note_impl(
        &state,
        txn.transaction_id,
        Some("coffee, the \"good\" kind".to_owned()),
        String::new(),
    )
    .unwrap();

    let out = dir.path().join("export.csv");
    let count = export_transactions_csv_impl(&state, out.to_string_lossy().into_owned()).unwrap();
    assert_eq!(count, 2);

    let csv = std::fs::read_to_string(&out).unwrap();
    let lines: Vec<&str> = csv.lines().collect();
    assert_eq!(lines.len(), 3, "header + 2 rows");
    assert_eq!(
        lines[0],
        "date,account,amount,currency,category,merchant,memo,tags,note"
    );
    // Chronological: the June 20 expense first, formatted as signed decimal.
    assert!(lines[1].starts_with("2026-06-20,Checking,-12.99,USD,"));
    assert!(lines[2].starts_with("2026-06-25,Checking,42.00,USD,"));
    // The tricky note survives as one quoted field with the quote doubled.
    assert!(
        lines[1].ends_with("\"coffee, the \"\"good\"\" kind\""),
        "escaped note, got: {}",
        lines[1]
    );
}

/// ADR 0044 / 4d8.22.4: an account note round-trips through the read model and clears.
#[test]
fn account_note_round_trips_and_clears() {
    use app_lib::ipc::commands::set_account_note_impl;
    let (_dir, state) = open_state();
    let id = create_account_impl(&state, create_input("Checking"))
        .unwrap()
        .account_id;

    set_account_note_impl(
        &state,
        id.clone(),
        Some("rainy-day fund".to_owned()),
        String::new(),
    )
    .unwrap();
    assert_eq!(
        account_view_impl(&state, id.clone())
            .unwrap()
            .unwrap()
            .notes,
        Some("rainy-day fund".to_owned())
    );

    // Clearing (None) removes it.
    set_account_note_impl(&state, id.clone(), None, String::new()).unwrap();
    assert_eq!(account_view_impl(&state, id).unwrap().unwrap().notes, None);
}

/// ADR 0044 / 4d8.22.2: a Property real-asset account can be created with its subtype.
#[test]
fn a_property_real_asset_account_can_be_created() {
    let (_dir, state) = open_state();
    let mut input = create_input("Our house");
    input.cashflow_role = CashflowRoleDto::RealAsset;
    input.subtype = Some("property".to_owned());
    let id = create_account_impl(&state, input).unwrap().account_id;

    let view = account_view_impl(&state, id).unwrap().unwrap();
    assert_eq!(view.cashflow_role, "real_asset");
    assert_eq!(view.subtype, Some("property".to_owned()));
}

/// ADR 0044 / 4d8.22.5: original principal round-trips on a loan's debt terms.
#[test]
fn original_principal_round_trips_on_debt_terms() {
    use app_lib::ipc::commands::{debt_terms_impl, set_debt_terms_impl};
    use app_lib::ipc::dto::{RepaymentPhilosophyDto, SetDebtTermsInput};

    let (_dir, state) = open_state();
    let mut input = create_input("Auto loan");
    input.cashflow_role = CashflowRoleDto::LoanLiability;
    input.subtype = Some("auto_loan".to_owned());
    let id = create_account_impl(&state, input).unwrap().account_id;

    set_debt_terms_impl(
        &state,
        SetDebtTermsInput {
            account_id: id.clone(),
            apr_bps: Some(600),
            original_principal_minor: Some(3_500_000),
            statement_close_day: None,
            payment_due_day: Some(1),
            grace_period_days: None,
            credit_limit_minor: None,
            repayment_philosophy: RepaymentPhilosophyDto::PayFixedAmount,
            fixed_amount_minor: Some(60_000),
            min_payment_percent_bps: None,
            min_payment_floor_minor: None,
            paying_source_account_id: None,
            idempotency_key: String::new(),
        },
    )
    .unwrap();

    let terms = debt_terms_impl(&state, id).unwrap().unwrap();
    assert_eq!(terms.original_principal_minor, Some(3_500_000));
}

/// personal-cfo-3b8.2: income can only deposit into a liquid-cash account; a non-liquid
/// deposit target is rejected by the kernel.
#[test]
fn income_deposit_into_a_non_liquid_account_is_rejected() {
    let (_dir, state) = open_state();
    let mut brokerage = create_input("Brokerage");
    brokerage.cashflow_role = CashflowRoleDto::InvestmentAsset;
    brokerage.subtype = Some("brokerage".to_owned());
    let brokerage_id = create_account_impl(&state, brokerage).unwrap().account_id;

    let err = create_income_source_impl(
        &state,
        CreateIncomeSourceInput {
            name: "Paycheck".to_owned(),
            net_amount: MoneyDto {
                minor_units: 300_000,
                currency: "USD".to_owned(),
            },
            frequency: "biweekly".to_owned(),
            anchor_date: "2026-07-01".to_owned(),
            deposit_account_id: Some(brokerage_id),
            idempotency_key: String::new(),
        },
    )
    .unwrap_err();
    assert!(
        matches!(err, IpcError::Validation(_)),
        "a non-liquid deposit account must be rejected, got {err:?}"
    );
}

/// ADR 0044 §5 / 4d8.22.3: linking a property to its mortgage resolves the partner name
/// on BOTH sides and clears cleanly; net-worth-affecting fields are untouched.
#[test]
fn linking_a_property_to_a_mortgage_round_trips_both_sides() {
    use app_lib::ipc::commands::set_account_link_impl;
    let (_dir, state) = open_state();

    let mut house = create_input("Our house");
    house.cashflow_role = CashflowRoleDto::RealAsset;
    house.subtype = Some("property".to_owned());
    let house_id = create_account_impl(&state, house).unwrap().account_id;

    let mut mortgage = create_input("Home mortgage");
    mortgage.cashflow_role = CashflowRoleDto::LoanLiability;
    mortgage.subtype = Some("mortgage".to_owned());
    let mortgage_id = create_account_impl(&state, mortgage).unwrap().account_id;

    set_account_link_impl(
        &state,
        house_id.clone(),
        Some(mortgage_id.clone()),
        String::new(),
    )
    .unwrap();

    let house_view = account_view_impl(&state, house_id.clone())
        .unwrap()
        .unwrap();
    assert_eq!(house_view.linked_account_id, Some(mortgage_id.clone()));
    assert_eq!(
        house_view.linked_account_name,
        Some("Home mortgage".to_owned())
    );

    // The liability sees the link too, via the reverse lookup (its own id is unset).
    let mortgage_view = account_view_impl(&state, mortgage_id.clone())
        .unwrap()
        .unwrap();
    assert_eq!(mortgage_view.linked_account_id, None);
    assert_eq!(
        mortgage_view.linked_account_name,
        Some("Our house".to_owned())
    );

    // Clearing removes it on both sides.
    set_account_link_impl(&state, house_id.clone(), None, String::new()).unwrap();
    assert_eq!(
        account_view_impl(&state, house_id)
            .unwrap()
            .unwrap()
            .linked_account_name,
        None
    );
    assert_eq!(
        account_view_impl(&state, mortgage_id)
            .unwrap()
            .unwrap()
            .linked_account_name,
        None
    );
}

/// ADR 0044 §5: only a real asset may own a link; a liquid-cash source is rejected.
#[test]
fn linking_rejects_a_non_real_asset_source() {
    use app_lib::ipc::commands::set_account_link_impl;
    let (_dir, state) = open_state();
    let checking = create_account_impl(&state, create_input("Checking"))
        .unwrap()
        .account_id;
    let mut mortgage = create_input("Mortgage");
    mortgage.cashflow_role = CashflowRoleDto::LoanLiability;
    mortgage.subtype = Some("mortgage".to_owned());
    let mortgage_id = create_account_impl(&state, mortgage).unwrap().account_id;

    let err =
        set_account_link_impl(&state, checking, Some(mortgage_id), String::new()).unwrap_err();
    assert!(matches!(err, IpcError::Validation(_)), "got {err:?}");
}

/// ADR 0044 §5: the target must be a liability; a real asset -> investment link is rejected.
#[test]
fn linking_rejects_a_non_liability_target() {
    use app_lib::ipc::commands::set_account_link_impl;
    let (_dir, state) = open_state();
    let mut car = create_input("Car");
    car.cashflow_role = CashflowRoleDto::RealAsset;
    car.subtype = Some("vehicle".to_owned());
    let car_id = create_account_impl(&state, car).unwrap().account_id;
    let mut brokerage = create_input("Brokerage");
    brokerage.cashflow_role = CashflowRoleDto::InvestmentAsset;
    brokerage.subtype = Some("brokerage".to_owned());
    let brokerage_id = create_account_impl(&state, brokerage).unwrap().account_id;

    let err = set_account_link_impl(&state, car_id, Some(brokerage_id), String::new()).unwrap_err();
    assert!(matches!(err, IpcError::Validation(_)), "got {err:?}");
}

/// ADR 0044 §5 (personal-cfo-4d8.23.4): a real asset is financed by a LOAN, not a revolving
/// credit card — a real_asset -> credit_facility link is rejected.
#[test]
fn linking_rejects_a_credit_card_target() {
    use app_lib::ipc::commands::set_account_link_impl;
    let (_dir, state) = open_state();
    let mut house = create_input("House");
    house.cashflow_role = CashflowRoleDto::RealAsset;
    house.subtype = Some("property".to_owned());
    let house_id = create_account_impl(&state, house).unwrap().account_id;
    let mut card = create_input("Sapphire");
    card.cashflow_role = CashflowRoleDto::CreditFacility;
    card.subtype = Some("credit_card".to_owned());
    let card_id = create_account_impl(&state, card).unwrap().account_id;

    let err = set_account_link_impl(&state, house_id, Some(card_id), String::new()).unwrap_err();
    assert!(matches!(err, IpcError::Validation(_)), "got {err:?}");
}

/// ADR 0044 §5: one-to-one is enforced — a liability already linked by another asset is
/// rejected, so the reverse lookup stays deterministic.
#[test]
fn linking_a_second_asset_to_the_same_liability_is_rejected() {
    use app_lib::ipc::commands::set_account_link_impl;
    let (_dir, state) = open_state();
    let mut house = create_input("House");
    house.cashflow_role = CashflowRoleDto::RealAsset;
    house.subtype = Some("property".to_owned());
    let house_id = create_account_impl(&state, house).unwrap().account_id;
    let mut cottage = create_input("Cottage");
    cottage.cashflow_role = CashflowRoleDto::RealAsset;
    cottage.subtype = Some("property".to_owned());
    let cottage_id = create_account_impl(&state, cottage).unwrap().account_id;
    let mut mortgage = create_input("Mortgage");
    mortgage.cashflow_role = CashflowRoleDto::LoanLiability;
    mortgage.subtype = Some("mortgage".to_owned());
    let mortgage_id = create_account_impl(&state, mortgage).unwrap().account_id;

    set_account_link_impl(&state, house_id, Some(mortgage_id.clone()), String::new()).unwrap();
    let err =
        set_account_link_impl(&state, cottage_id, Some(mortgage_id), String::new()).unwrap_err();
    assert!(matches!(err, IpcError::Validation(_)), "got {err:?}");
}

/// ADR 0044 §5: a "Linked to …" chip surfaces only for an ACTIVE partner — archiving the
/// linked liability hides the chip on the still-active asset (no stale reference).
#[test]
fn archiving_a_linked_partner_hides_the_chip() {
    use app_lib::ipc::commands::{archive_account_impl, set_account_link_impl};
    let (_dir, state) = open_state();
    let mut house = create_input("House");
    house.cashflow_role = CashflowRoleDto::RealAsset;
    house.subtype = Some("property".to_owned());
    let house_id = create_account_impl(&state, house).unwrap().account_id;
    let mut mortgage = create_input("Mortgage");
    mortgage.cashflow_role = CashflowRoleDto::LoanLiability;
    mortgage.subtype = Some("mortgage".to_owned());
    let mortgage_id = create_account_impl(&state, mortgage).unwrap().account_id;

    set_account_link_impl(
        &state,
        house_id.clone(),
        Some(mortgage_id.clone()),
        String::new(),
    )
    .unwrap();
    assert_eq!(
        account_view_impl(&state, house_id.clone())
            .unwrap()
            .unwrap()
            .linked_account_name,
        Some("Mortgage".to_owned())
    );

    archive_account_impl(&state, mortgage_id, String::new()).unwrap();
    // The link row is untouched, but the chip no longer resolves an archived partner.
    assert_eq!(
        account_view_impl(&state, house_id)
            .unwrap()
            .unwrap()
            .linked_account_name,
        None
    );
}

/// personal-cfo-4d8.25.4: the statement-history surface — past windows with stored values
/// after recording, an empty list for an account with no payment boundary, and a typed
/// error for a malformed account id.
#[test]
fn card_statement_history_lists_windows_and_rejects_bad_input() {
    use app_lib::ipc::commands::{
        card_statement_history_impl, set_card_statement_balance_impl, set_debt_terms_impl,
    };
    use app_lib::ipc::dto::{
        RepaymentPhilosophyDto, SetCardStatementBalanceInput, SetDebtTermsInput,
    };

    let (_dir, state) = open_state();
    let mut card_input = create_input("Venture X");
    card_input.cashflow_role = CashflowRoleDto::CreditFacility;
    card_input.subtype = Some("credit_card".to_owned());
    let card = create_account_impl(&state, card_input).unwrap().account_id;
    set_debt_terms_impl(
        &state,
        SetDebtTermsInput {
            account_id: card.clone(),
            apr_bps: None,
            original_principal_minor: None,
            statement_close_day: Some(22),
            payment_due_day: Some(17),
            grace_period_days: None,
            credit_limit_minor: None,
            repayment_philosophy: RepaymentPhilosophyDto::PayStatementBalance,
            fixed_amount_minor: None,
            min_payment_percent_bps: None,
            min_payment_floor_minor: None,
            paying_source_account_id: None,
            idempotency_key: String::new(),
        },
    )
    .unwrap();

    // Happy: past windows are listed newest-first, keyed by the close day.
    let history = card_statement_history_impl(&state, card.clone()).unwrap();
    assert!(!history.is_empty());
    assert!(history
        .windows(2)
        .all(|w| w[0].close_date > w[1].close_date));
    assert!(history[0].close_date.ends_with("-22"));

    // Recording a past statement surfaces on its window.
    let target = history[1].close_date.clone();
    set_card_statement_balance_impl(
        &state,
        SetCardStatementBalanceInput {
            account_id: card.clone(),
            cycle_close: target.clone(),
            statement_balance_minor: Some(77_700),
            idempotency_key: String::new(),
        },
    )
    .unwrap();
    let after = card_statement_history_impl(&state, card).unwrap();
    let row = after
        .iter()
        .find(|w| w.close_date == target)
        .expect("recorded window present");
    assert_eq!(row.stored_statement_minor, Some(77_700));

    // Failure 1: a malformed account id is a typed error, not a panic.
    assert!(card_statement_history_impl(&state, "not-a-uuid".to_owned()).is_err());

    // Failure 2 (shape): an account with no debt terms yields an empty list.
    let plain = create_account_impl(&state, create_input("Checking"))
        .unwrap()
        .account_id;
    assert!(card_statement_history_impl(&state, plain)
        .unwrap()
        .is_empty());
}

/// personal-cfo-4d8.25.15: the by-ids row read resolves exactly the requested rows —
/// unknown ids are silently absent, and a malformed id is a typed InvalidInput error.
#[test]
fn transaction_rows_by_ids_resolves_known_ids_and_rejects_garbage() {
    let (_dir, state) = open_state();
    let id = create_account_impl(&state, create_input("Checking"))
        .unwrap()
        .account_id;
    let mut ids = Vec::new();
    for (minor, when) in [(-1_500i64, "2026-07-01"), (-2_500, "2026-07-02")] {
        let result = record_transaction_impl(
            &state,
            RecordTransactionInput {
                account_id: id.clone(),
                amount: MoneyDto {
                    minor_units: minor,
                    currency: "USD".to_owned(),
                },
                occurred_at: format!("{when}T00:00:00Z"),
                idempotency_key: String::new(),
            },
        )
        .unwrap();
        ids.push(result.transaction_id);
    }

    // Happy: both rows resolve; an unknown id is absent (not an error).
    let mut requested = ids.clone();
    requested.push(uuid::Uuid::now_v7().to_string());
    let rows = transaction_rows_by_ids_impl(&state, requested).unwrap();
    assert_eq!(rows.len(), 2);
    assert!(ids
        .iter()
        .all(|id| rows.iter().any(|r| &r.transaction_id == id)));

    // Failure: a malformed id is rejected with a typed error.
    let err = transaction_rows_by_ids_impl(&state, vec!["not-a-uuid".to_owned()]).unwrap_err();
    assert!(matches!(err, IpcError::Validation(_)));
}

/// personal-cfo-4d8.25.16: bulk mark-reviewed reviews every id in one call and reports
/// the count; a nonexistent id fails the batch with a typed error (earlier marks stay
/// applied — the audited per-command semantics).
#[test]
fn mark_inbox_reviewed_bulk_reviews_all_and_rejects_unknown_ids() {
    let (_dir, state) = open_state();
    let id = create_account_impl(&state, create_input("Checking"))
        .unwrap()
        .account_id;
    let mut ids = Vec::new();
    for when in ["2026-07-01", "2026-07-02", "2026-07-03"] {
        let result = record_transaction_impl(
            &state,
            RecordTransactionInput {
                account_id: id.clone(),
                amount: MoneyDto {
                    minor_units: -1_000,
                    currency: "USD".to_owned(),
                },
                occurred_at: format!("{when}T00:00:00Z"),
                idempotency_key: String::new(),
            },
        )
        .unwrap();
        ids.push(result.transaction_id);
    }

    let count =
        mark_inbox_reviewed_bulk_impl(&state, "bulk-key-1".to_owned(), ids.clone()).unwrap();
    assert_eq!(count, 3);
    let rows = transaction_rows_by_ids_impl(&state, ids).unwrap();
    assert!(rows.iter().all(|r| r.reviewed), "every row flips reviewed");

    // Failure: an unknown id in the set is a typed command error.
    let err = mark_inbox_reviewed_bulk_impl(
        &state,
        "bulk-key-2".to_owned(),
        vec![uuid::Uuid::now_v7().to_string()],
    )
    .unwrap_err();
    assert!(matches!(err, IpcError::Validation(_)));
}

/// hbd8 AC: the export is DETERMINISTIC given the canonical state — the same
/// state produces byte-identical CSV, run to run and vault to vault. The
/// inline expected string doubles as the format snapshot (the AC's insta
/// snapshot, realized without a new dependency — renegotiation on the bead).
#[test]
fn export_csv_is_deterministic_and_matches_the_format_snapshot() {
    use app_lib::ipc::commands::{export_transactions_csv_impl, set_note_impl};

    let build_vault = || {
        let (dir, state) = open_state();
        let account = create_account_impl(&state, create_input("Checking"))
            .unwrap()
            .account_id;
        for (minor, day) in [(-1_299_i64, "2026-06-20"), (4_200, "2026-06-25")] {
            record_transaction_impl(
                &state,
                RecordTransactionInput {
                    account_id: account.clone(),
                    amount: MoneyDto {
                        minor_units: minor,
                        currency: "USD".to_owned(),
                    },
                    occurred_at: format!("{day}T12:00:00Z"),
                    idempotency_key: String::new(),
                },
            )
            .unwrap();
        }
        let txn = transaction_list_impl(&state)
            .unwrap()
            .into_iter()
            .find(|r| r.amount.minor_units == -1_299)
            .unwrap();
        set_note_impl(
            &state,
            txn.transaction_id,
            Some("morning coffee".to_owned()),
            String::new(),
        )
        .unwrap();
        (dir, state)
    };

    let (dir_a, state_a) = build_vault();
    let out_a1 = dir_a.path().join("a1.csv");
    let out_a2 = dir_a.path().join("a2.csv");
    export_transactions_csv_impl(&state_a, out_a1.to_string_lossy().into_owned()).unwrap();
    export_transactions_csv_impl(&state_a, out_a2.to_string_lossy().into_owned()).unwrap();
    let csv_a1 = std::fs::read_to_string(&out_a1).unwrap();
    let csv_a2 = std::fs::read_to_string(&out_a2).unwrap();
    assert_eq!(csv_a1, csv_a2, "same vault, same bytes, run to run");

    let (dir_b, state_b) = build_vault();
    let out_b = dir_b.path().join("b.csv");
    export_transactions_csv_impl(&state_b, out_b.to_string_lossy().into_owned()).unwrap();
    assert_eq!(
        csv_a1,
        std::fs::read_to_string(&out_b).unwrap(),
        "identically-built vaults export byte-identical CSV"
    );

    // The format snapshot: any change to columns, ordering, formatting, or
    // escaping must be a conscious edit of this expected text.
    assert_eq!(
        csv_a1,
        "date,account,amount,currency,category,merchant,memo,tags,note\n\
         2026-06-20,Checking,-12.99,USD,,,,,morning coffee\n\
         2026-06-25,Checking,42.00,USD,,,,,\n"
    );
}

/// hbd8 AC: the exported CSV round-trips through the generic CSV importer
/// (personal-cfo-cu8) with no information loss for the fields both sides
/// support — dates, amounts, currency, and the note text (mapped onto the
/// importer's description field).
#[test]
fn export_csv_round_trips_through_the_csv_importer() {
    use app_lib::ipc::commands::{export_transactions_csv_impl, import_batch_impl, set_note_impl};
    use app_lib::ipc::dto::ColumnMappingDto;

    // Vault A: real data, exported.
    let (dir_a, state_a) = open_state();
    let account_a = create_account_impl(&state_a, create_input("Checking"))
        .unwrap()
        .account_id;
    for (minor, day) in [(-1_299_i64, "2026-06-20"), (4_200, "2026-06-25")] {
        record_transaction_impl(
            &state_a,
            RecordTransactionInput {
                account_id: account_a.clone(),
                amount: MoneyDto {
                    minor_units: minor,
                    currency: "USD".to_owned(),
                },
                occurred_at: format!("{day}T12:00:00Z"),
                idempotency_key: String::new(),
            },
        )
        .unwrap();
    }
    let txn = transaction_list_impl(&state_a)
        .unwrap()
        .into_iter()
        .find(|r| r.amount.minor_units == -1_299)
        .unwrap();
    set_note_impl(
        &state_a,
        txn.transaction_id,
        Some("coffee, the \"good\" kind".to_owned()),
        String::new(),
    )
    .unwrap();
    let out = dir_a.path().join("export.csv");
    export_transactions_csv_impl(&state_a, out.to_string_lossy().into_owned()).unwrap();
    let csv_bytes = std::fs::read(&out).unwrap();

    // Vault B ("the new machine"): import the export through the real CSV
    // importer, mapping the export's columns onto the importer's fields.
    let (_dir_b, state_b) = open_state();
    let account_b = create_account_impl(&state_b, create_input("Checking"))
        .unwrap()
        .account_id;
    let result = import_batch_impl(
        &state_b,
        ImportBatchInput {
            data: csv_bytes,
            filename: Some("export.csv".to_owned()),
            target_account_id: account_b,
            plugin_id: Some("generic-csv".to_owned()),
            column_mapping: Some(ColumnMappingDto {
                date: Some("date".to_owned()),
                description: Some("note".to_owned()),
                amount: Some("amount".to_owned()),
                debit: None,
                credit: None,
                account: Some("account".to_owned()),
                category: Some("category".to_owned()),
                currency: Some("currency".to_owned()),
                memo: Some("memo".to_owned()),
            }),
            date_format: None,
            default_currency: Some("USD".to_owned()),
            idempotency_key: String::new(),
        },
    )
    .unwrap();
    assert_eq!(result.staged, 2);
    assert_eq!(result.committed, 2, "clean rows auto-commit: {result:?}");

    // Supported fields survived intact.
    let rows = transaction_list_impl(&state_b).unwrap();
    assert_eq!(rows.len(), 2);
    let coffee = rows
        .iter()
        .find(|r| r.amount.minor_units == -1_299)
        .expect("the expense round-tripped");
    assert_eq!(coffee.occurred_at[..10].to_owned(), "2026-06-20");
    assert_eq!(coffee.amount.currency, "USD");
    let imported_text = coffee
        .memo
        .clone()
        .or_else(|| coffee.counterparty.clone())
        .unwrap_or_default();
    assert!(
        imported_text.contains("coffee, the \"good\" kind"),
        "the RFC-4180-escaped note survived the round trip: {coffee:?}"
    );
    let deposit = rows
        .iter()
        .find(|r| r.amount.minor_units == 4_200)
        .expect("the deposit round-tripped");
    assert_eq!(deposit.occurred_at[..10].to_owned(), "2026-06-25");
}

/// personal-cfo-gmnk: recurring inbound deposits surface as income candidates
/// through the IPC, with the deposit account prefilled — the onboarding
/// income step's approve/edit/deny source.
#[test]
fn recurring_deposits_surface_as_income_candidates() {
    use app_lib::ipc::commands::income_candidates_impl;

    let (_dir, state) = open_state();
    let account_id = create_account_impl(&state, create_input("Checking"))
        .unwrap()
        .account_id;
    // Four biweekly payroll credits + one debit through the real OFX importer.
    let ofx = "OFXHEADER:100\r\nDATA:OFXSGML\r\nVERSION:102\r\n\r\n\
               <OFX><BANKMSGSRSV1><STMTTRNRS><STMTRS><CURDEF>USD\
               <BANKTRANLIST>\
               <STMTTRN><TRNTYPE>CREDIT<DTPOSTED>20260710<TRNAMT>2500.00<FITID>PAY-1<NAME>ACME PAYROLL</STMTTRN>\
               <STMTTRN><TRNTYPE>CREDIT<DTPOSTED>20260724<TRNAMT>2500.00<FITID>PAY-2<NAME>ACME PAYROLL</STMTTRN>\
               <STMTTRN><TRNTYPE>CREDIT<DTPOSTED>20260807<TRNAMT>2500.00<FITID>PAY-3<NAME>ACME PAYROLL</STMTTRN>\
               <STMTTRN><TRNTYPE>CREDIT<DTPOSTED>20260821<TRNAMT>2500.00<FITID>PAY-4<NAME>ACME PAYROLL</STMTTRN>\
               <STMTTRN><TRNTYPE>DEBIT<DTPOSTED>20260815<TRNAMT>-60.00<FITID>WTR-1<NAME>CITY WATER</STMTTRN>\
               </BANKTRANLIST></STMTRS></STMTTRNRS></BANKMSGSRSV1></OFX>";
    let result = import_batch_impl(
        &state,
        ImportBatchInput {
            data: ofx.as_bytes().to_vec(),
            filename: Some("statement.ofx".to_owned()),
            target_account_id: account_id.clone(),
            plugin_id: None,
            column_mapping: None,
            default_currency: None,
            date_format: None,
            idempotency_key: String::new(),
        },
    )
    .unwrap();
    assert_eq!(result.committed, 5, "{result:?}");

    let candidates = income_candidates_impl(&state).unwrap();
    assert_eq!(candidates.len(), 1, "{candidates:?}");
    let payroll = &candidates[0];
    assert_eq!(payroll.display.to_lowercase(), "acme payroll");
    assert_eq!(payroll.amount_minor, 250_000);
    assert_eq!(payroll.frequency, "biweekly");
    assert_eq!(
        payroll.source_account_id.as_deref(),
        Some(account_id.as_str())
    );
}
