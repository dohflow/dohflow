//! Seed the **Polish Demo** vault — the fixture every DohFlow screenshot and design
//! walkthrough is taken against (personal-cfo-2pcx, refreshed in personal-cfo-4d8.28.4).
//!
//! Two entry points share one seeding body ([`seed_demo_vault`]):
//!
//! - `seed_polish_demo_vault` — **ignored**, double-gated on `PCFO_SEED_ROOT`. It registers a
//!   new "Polish Demo" vault in that root's `vaults.json` (never touching existing vaults'
//!   files) for the real app to open.
//! - `seeded_demo_vault_populates_every_screenshot_surface` — runs in CI against a temp
//!   root and asserts that each screenshot surface (`n76x.13`) has content: dashboard /
//!   forecast, scenarios, Money Inbox, accounts and debt, import, vault / backup.
//!
//! Every date is an offset from one **anchor** — the household's "today" — so the stored
//! data is a pure function of the anchor. The CI test pins it; the manual seed defaults to
//! the local date so today-relative surfaces (past-due, stale balances, upcoming bills)
//! look right in a screenshot, and `PCFO_SEED_ANCHOR=YYYY-MM-DD` pins it for reproducible
//! runs. Every institution, merchant, employer, and person is invented.
//!
//! Run (see `docs/agent/demo-vault.md`):
//! ```sh
//! PCFO_SEED_ROOT="$HOME/Library/Application Support/ai.personalcfo.desktop" \
//!   cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml \
//!   --test seed_polish_vault -- --ignored --nocapture
//! ```
//! Demo vault passphrase: `polish-demo` (a fixture, not a secret).

use std::path::{Path, PathBuf};

use app_lib::ipc::commands::{
    account_list_impl, assert_balance_impl, card_statement_forecast_impl, cash_flow_history_impl,
    cash_tiers_impl, category_list_impl, confirm_obligation_early_impl, connector_connections_impl,
    connector_link_impl, connector_set_account_link_impl, connector_sync_impl, create_account_impl,
    create_forecast_assumption_impl, create_income_source_impl, create_manual_future_entry_impl,
    create_recurring_bill_impl, create_recurring_transfer_impl, create_scenario_impl,
    create_tag_impl, create_vault_impl, debt_payoff_comparison_impl, debt_terms_list_impl,
    duplicate_candidates_impl, export_backup_impl, future_cash_forecast_impl, import_batch_impl,
    income_candidates_impl, income_source_list_impl, list_vaults_impl, lock_vault_impl,
    mark_inbox_reviewed_bulk_impl, money_inbox_list_impl, recategorize_transaction_impl,
    record_transaction_impl, record_transfer_impl, recurring_bill_history_impl,
    recurring_bill_list_impl, recurring_candidates_impl, restore_backup_impl, scenario_list_impl,
    set_account_link_impl, set_account_note_impl, set_card_statement_balance_impl,
    set_comfort_band_upper_impl, set_debt_terms_impl, set_household_timezone_impl,
    set_minimum_cash_floor_impl, set_note_impl, set_scenario_expiry_impl, set_tags_impl,
    transaction_page_impl, unconfirmed_past_due_impl, unlock_vault_impl, update_scenario_impl,
    vault_health_impl,
};
use app_lib::ipc::dto::{
    AccountFlagsDto, AssertBalanceInput, CashflowRoleDto, ConfirmObligationEarlyInput,
    ConnectorLinkInput, ConnectorSetAccountLinkInput, ConnectorSyncInput, CreateAccountInput,
    CreateForecastAssumptionInput, CreateIncomeSourceInput, CreateManualFutureEntryInput,
    CreateRecurringBillInput, CreateRecurringTransferInput, CreateScenarioInput, ImportBatchInput,
    MoneyDto, RecordTransactionInput, RecordTransferInput, RepaymentPhilosophyDto,
    SetCardStatementBalanceInput, SetDebtTermsInput, SetScenarioExpiryInput, TransactionPageInput,
    UpdateScenarioInput, VaultStateDto,
};
use app_lib::vault_registry::{VaultEntry, VaultRegistry};
use app_lib::AppState;
use chrono::{Datelike, Duration, Months, NaiveDate};
use connector_core::mock::MockConnector;
use connector_core::CapabilitySet;
use finance_kernel::VaultController;
use tempfile::TempDir;
use uuid::Uuid;

/// The registry name the vault picker shows.
const VAULT_NAME: &str = "Polish Demo";
/// The demo passphrase — a published fixture, deliberately not a secret.
const PASSWORD: &str = "polish-demo";
/// The anchor the CI test pins: on or before any day the test can run, so the
/// past-due and stale-balance surfaces are populated deterministically.
fn fixed_anchor() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 9, 1).expect("valid fixed anchor")
}
/// The name of the one manual bill left deliberately unpaid, so Cash Flow has a
/// past-due unconfirmed obligation (ADR 0058).
const PAST_DUE_BILL: &str = "Maple Street Piano Studio";
/// The mock connector's adapter id: a `source_batches.source_type` schema token with no
/// registered production adapter, so the real app's unlock auto-sync skips it and the
/// connection stays exactly as seeded (healthy, synced at seed time).
const DEMO_ADAPTER_ID: &str = "other";

/// What [`seed_demo_vault`] produced: the registered vault and the still-open state.
struct SeededVault {
    id: Uuid,
    path: PathBuf,
    state: AppState,
}

fn usd(minor_units: i64) -> MoneyDto {
    MoneyDto {
        minor_units,
        currency: "USD".to_owned(),
    }
}

// ── Date helpers — everything is relative to the anchor ──────────────────────

fn days(anchor: NaiveDate, offset: i64) -> NaiveDate {
    anchor + Duration::days(offset)
}

fn stamp(date: NaiveDate) -> String {
    format!("{date}T17:30:00Z")
}

/// The latest date with day-of-month `day` (≤ 28) on or before `anchor`.
fn on_or_before(anchor: NaiveDate, day: u32) -> NaiveDate {
    if anchor.day() >= day {
        anchor.with_day(day).expect("day <= 28")
    } else {
        anchor
            .with_day(1)
            .expect("first of month")
            .checked_sub_months(Months::new(1))
            .expect("previous month")
            .with_day(day)
            .expect("day <= 28")
    }
}

fn month_end(date: NaiveDate) -> NaiveDate {
    date.with_day(1)
        .expect("first of month")
        .checked_add_months(Months::new(1))
        .expect("next month")
        - Duration::days(1)
}

/// Every date with day-of-month `day` in `[from, to]`, ascending.
fn monthly_on(day: u32, from: NaiveDate, to: NaiveDate) -> Vec<NaiveDate> {
    let mut out = Vec::new();
    let mut cursor = from.with_day(1).expect("first of month");
    while cursor <= to {
        let date = cursor.with_day(day).expect("day <= 28");
        if date >= from && date <= to {
            out.push(date);
        }
        cursor = cursor
            .checked_add_months(Months::new(1))
            .expect("next month");
    }
    out
}

/// The 15th and the last day of every month in `[from, to]` (the pay-schedule crate's
/// semi-monthly rule), ascending.
fn semi_monthly_paydays(from: NaiveDate, to: NaiveDate) -> Vec<NaiveDate> {
    let mut out = Vec::new();
    for fifteenth in monthly_on(15, from, to) {
        out.push(fifteenth);
    }
    let mut cursor = from.with_day(1).expect("first of month");
    while cursor <= to {
        let last = month_end(cursor);
        if last >= from && last <= to {
            out.push(last);
        }
        cursor = cursor
            .checked_add_months(Months::new(1))
            .expect("next month");
    }
    out.sort_unstable();
    out
}

/// `latest`, then every `interval` days earlier, down to `from`; ascending.
fn stepping_back(latest: NaiveDate, interval: i64, from: NaiveDate) -> Vec<NaiveDate> {
    let mut out = Vec::new();
    let mut date = latest;
    while date >= from {
        out.push(date);
        date -= Duration::days(interval);
    }
    out.reverse();
    out
}

/// `latest` and the same day of the previous `count - 1` months, ascending.
fn monthly_back(latest: NaiveDate, count: u32) -> Vec<NaiveDate> {
    let mut out: Vec<NaiveDate> = (0..count)
        .map(|k| {
            latest
                .checked_sub_months(Months::new(k))
                .expect("previous month")
        })
        .collect();
    out.reverse();
    out
}

// ── Input builders ───────────────────────────────────────────────────────────

fn account(name: &str, role: CashflowRoleDto, subtype: &str) -> CreateAccountInput {
    CreateAccountInput {
        name: name.to_owned(),
        cashflow_role: role,
        currency: "USD".to_owned(),
        flags: Some(AccountFlagsDto {
            retirement: subtype == "retirement",
            tax_advantaged: matches!(subtype, "retirement" | "hsa"),
            joint: true,
            business: false,
        }),
        opening_balance: None,
        subtype: Some(subtype.to_owned()),
        idempotency_key: String::new(),
    }
}

/// One line of the generic-CSV bank export the checking account imports.
struct CsvRow {
    date: NaiveDate,
    description: &'static str,
    minor: i64,
}

fn csv_row(date: NaiveDate, description: &'static str, minor: i64) -> CsvRow {
    CsvRow {
        date,
        description,
        minor,
    }
}

fn csv_text(rows: &mut [CsvRow]) -> String {
    rows.sort_by(|a, b| a.date.cmp(&b.date).then(a.description.cmp(b.description)));
    let mut text = String::from("Date,Description,Amount\n");
    for row in rows {
        assert!(
            !row.description.contains(','),
            "unquoted CSV description must not contain a comma"
        );
        let sign = if row.minor < 0 { "-" } else { "" };
        let magnitude = row.minor.abs();
        text.push_str(&format!(
            "{},{},{sign}{}.{:02}\n",
            row.date,
            row.description,
            magnitude / 100,
            magnitude % 100
        ));
    }
    text
}

/// Record a manual expense/income with a merchant-style note and an optional category.
#[allow(clippy::too_many_arguments)]
fn spend(
    state: &AppState,
    account_id: &str,
    minor: i64,
    date: NaiveDate,
    merchant: &str,
    category: Option<&str>,
    categories: &[(String, String)],
    tag: Option<&str>,
    tags: &[(String, String)],
) {
    let txn_id = record_transaction_impl(
        state,
        RecordTransactionInput {
            account_id: account_id.to_owned(),
            amount: usd(minor),
            occurred_at: stamp(date),
            idempotency_key: String::new(),
        },
    )
    .expect("record transaction")
    .transaction_id;
    set_note_impl(
        state,
        txn_id.clone(),
        Some(merchant.to_owned()),
        String::new(),
    )
    .expect("set note");
    if let Some(wanted) = category {
        let (id, _) = categories
            .iter()
            .find(|(_, name)| name.eq_ignore_ascii_case(wanted))
            .unwrap_or_else(|| panic!("category {wanted:?} is in the default taxonomy"));
        recategorize_transaction_impl(state, txn_id.clone(), Some(id.clone()), String::new())
            .expect("categorize");
    }
    if let Some(wanted) = tag {
        let (id, _) = tags
            .iter()
            .find(|(_, name)| name == wanted)
            .unwrap_or_else(|| panic!("tag {wanted:?} was created above"));
        set_tags_impl(state, txn_id, vec![id.clone()], String::new()).expect("tag");
    }
}

// ── The seed ─────────────────────────────────────────────────────────────────

/// Register and fill the demo vault under `root` (an app-data root: the registry file plus
/// `vaults/<id>/vault.db`). `anchor` is the household's "today"; every date is relative to it.
/// Returns the still-unlocked state so the caller can inspect what was seeded.
fn seed_demo_vault(root: &Path, anchor: NaiveDate) -> SeededVault {
    assert!(root.is_dir(), "seed root must exist: {}", root.display());
    let at = |offset: i64| days(anchor, offset);
    // The imported bank history spans a little over a year: the recurring-instance
    // projection looks back 365 days from today, and every bill occurrence in that window
    // without a matching posting is a past-due obligation (ADR 0058). A shorter history
    // would leave months of spurious past-due rows on Cash Flow.
    let history_start = at(-370);

    // ── Register the vault (a fresh app-managed slot; existing entries untouched) ──
    let mut registry = VaultRegistry::load(root);
    registry.bootstrap(root);
    let id = Uuid::now_v7();
    let rel = Path::new("vaults").join(id.to_string()).join("vault.db");
    std::fs::create_dir_all(root.join("vaults").join(id.to_string())).expect("vault dir");
    registry.vaults.push(VaultEntry {
        id,
        name: VAULT_NAME.to_owned(),
        path: rel.clone(),
        created_at: chrono::Utc::now().to_rfc3339(),
    });
    registry.active = Some(id);
    registry.save(root).expect("save registry");

    let path = root.join(&rel);
    let state = AppState::with_registry(VaultController::open(&path), registry, root.to_path_buf());
    create_vault_impl(&state, PASSWORD.to_owned()).expect("create vault");
    // Non-UTC, deliberately (personal-cfo-q329): screenshots and the CI guard below should
    // exercise a real household timezone, not the untouched default every other test vault
    // sits at — the exact gap that let the launch-evening past-due symptom (5ie.11) ship
    // unnoticed against an all-UTC test fleet.
    set_household_timezone_impl(&state, "America/Los_Angeles".to_owned())
        .expect("set household timezone");

    // ── Accounts across every role (ADR 0028 subtypes) ─────────────────────────
    let mk = |input: CreateAccountInput| {
        create_account_impl(&state, input)
            .expect("account")
            .account_id
    };
    let checking = mk(account(
        "Saltmarsh CU Checking",
        CashflowRoleDto::LiquidCash,
        "checking",
    ));
    let savings = mk(account(
        "Kestrel Online Savings",
        CashflowRoleDto::LiquidCash,
        "savings",
    ));
    let copperleaf = mk(account(
        "Copperleaf Rewards Card",
        CashflowRoleDto::CreditFacility,
        "credit_card",
    ));
    let tidepool = mk(account(
        "Tidepool Cash-Back Card",
        CashflowRoleDto::CreditFacility,
        "credit_card",
    ));
    let auto_loan = mk(account(
        "Auto Loan — Quillfeather Finance",
        CashflowRoleDto::LoanLiability,
        "auto_loan",
    ));
    let mortgage = mk(account(
        "Mortgage — Foxglove Home Lending",
        CashflowRoleDto::LoanLiability,
        "mortgage",
    ));
    let brokerage = mk(account(
        "Larkspur Brokerage",
        CashflowRoleDto::InvestmentAsset,
        "brokerage",
    ));
    let retirement = mk(account(
        "401(k) — Ledgerline Systems",
        CashflowRoleDto::InvestmentAsset,
        "retirement",
    ));
    let hsa = mk(account(
        "HSA — Northhollow Benefits",
        CashflowRoleDto::InvestmentAsset,
        "hsa",
    ));
    let home = mk(account(
        "Home — 14 Alder Court",
        CashflowRoleDto::RealAsset,
        "property",
    ));
    let car = mk(account(
        "Car — 2022 hatchback",
        CashflowRoleDto::RealAsset,
        "vehicle",
    ));
    // Real assets point at the liabilities that finance them (ADR 0044 §5).
    set_account_link_impl(&state, home.clone(), Some(mortgage.clone()), String::new())
        .expect("home link");
    set_account_link_impl(&state, car.clone(), Some(auto_loan.clone()), String::new())
        .expect("car link");
    set_account_note_impl(
        &state,
        checking.clone(),
        Some("Joint household account — both paychecks land here.".to_owned()),
        String::new(),
    )
    .expect("checking note");

    // Balance assertions anchor every account (the additive model, ADR 0027). The savings
    // assertion is deliberately ~3 weeks old so the stale-balance Money Inbox item has
    // something to say (liquid threshold: 14 days).
    let anchor_balance = |account_id: &str, minor: i64, as_of: NaiveDate| {
        assert_balance_impl(
            &state,
            AssertBalanceInput {
                account_id: account_id.to_owned(),
                amount: usd(minor),
                as_of_date: as_of.to_string(),
            },
        )
        .expect("assert balance");
    };
    anchor_balance(&checking, 824_317, at(-1));
    anchor_balance(&savings, 2_489_000, at(-20));
    anchor_balance(&copperleaf, -231_455, at(-2));
    anchor_balance(&tidepool, -87_612, at(-2));
    anchor_balance(&auto_loan, -1_845_500, at(-3));
    anchor_balance(&mortgage, -41_238_000, at(-3));
    anchor_balance(&brokerage, 5_612_000, at(-4));
    anchor_balance(&retirement, 14_830_000, at(-4));
    anchor_balance(&hsa, 641_200, at(-4));
    anchor_balance(&home, 58_500_000, at(-30));
    anchor_balance(&car, 2_150_000, at(-30));

    // ── Debt terms (ADR 0035) + one real statement, so the debt views are full ──
    let terms = |account_id: &str,
                 apr_bps: i64,
                 close: Option<i64>,
                 due: i64,
                 limit: Option<i64>,
                 philosophy: RepaymentPhilosophyDto,
                 fixed_minor: Option<i64>,
                 principal: Option<i64>| SetDebtTermsInput {
        account_id: account_id.to_owned(),
        apr_bps: Some(apr_bps),
        original_principal_minor: principal,
        statement_close_day: close,
        payment_due_day: Some(due),
        grace_period_days: close.map(|_| 25),
        credit_limit_minor: limit,
        repayment_philosophy: philosophy,
        fixed_amount_minor: fixed_minor,
        min_payment_percent_bps: None,
        min_payment_floor_minor: None,
        paying_source_account_id: Some(checking.clone()),
        idempotency_key: String::new(),
    };
    set_debt_terms_impl(
        &state,
        terms(
            &copperleaf,
            2449,
            Some(12),
            8,
            Some(1_800_000),
            RepaymentPhilosophyDto::PayStatementBalance,
            None,
            None,
        ),
    )
    .expect("copperleaf terms");
    set_debt_terms_impl(
        &state,
        terms(
            &tidepool,
            2074,
            Some(20),
            15,
            Some(1_200_000),
            RepaymentPhilosophyDto::PayInFull,
            None,
            None,
        ),
    )
    .expect("tidepool terms");
    // The car note is a fixed monthly payment — never "pay the whole loan." The mortgage
    // is modeled as a recurring bill below instead of terms, so it is never double-counted.
    set_debt_terms_impl(
        &state,
        terms(
            &auto_loan,
            624,
            None,
            5,
            None,
            RepaymentPhilosophyDto::PayFixedAmount,
            Some(48_500),
            Some(2_890_000),
        ),
    )
    .expect("auto loan terms");
    set_card_statement_balance_impl(
        &state,
        SetCardStatementBalanceInput {
            account_id: copperleaf.clone(),
            cycle_close: on_or_before(anchor, 12).to_string(),
            statement_balance_minor: Some(231_455),
            idempotency_key: String::new(),
        },
    )
    .expect("copperleaf statement");

    // ── Income: both partners, two cadences ─────────────────────────────────────
    // Names normalize to the same merchant key as the imported deposits below, so the
    // income detector (gmnk) excludes them and suggests only what is NOT yet modeled.
    let ledgerline_paydays = semi_monthly_paydays(history_start, anchor);
    let northhollow_paydays = stepping_back(at(-4), 14, history_start);
    let income =
        |name: &str, minor: i64, frequency: &str, anchor_date: NaiveDate| CreateIncomeSourceInput {
            name: name.to_owned(),
            net_amount: usd(minor),
            frequency: frequency.to_owned(),
            anchor_date: anchor_date.to_string(),
            deposit_account_id: Some(checking.clone()),
            idempotency_key: String::new(),
        };
    create_income_source_impl(
        &state,
        income(
            "Ledgerline Systems Payroll",
            385_000,
            "semi_monthly",
            *ledgerline_paydays.last().expect("a payday in the window"),
        ),
    )
    .expect("income 1");
    create_income_source_impl(
        &state,
        income(
            "Northhollow Health Payroll",
            291_240,
            "biweekly",
            *northhollow_paydays.last().expect("a payday in the window"),
        ),
    )
    .expect("income 2");
    let northhollow_id = income_source_list_impl(&state)
        .expect("income list")
        .into_iter()
        .find(|source| source.name.starts_with("Northhollow"))
        .expect("Northhollow income source")
        .id;

    // ── Recurring bills: a realistic monthly stack, mixed autopay/manual ─────
    let categories: Vec<(String, String)> = category_list_impl(&state)
        .expect("categories")
        .into_iter()
        .map(|c| (c.id, c.name))
        .collect();
    let category_id = |wanted: &str| -> String {
        categories
            .iter()
            .find(|(_, name)| name.eq_ignore_ascii_case(wanted))
            .map(|(id, _)| id.clone())
            .unwrap_or_else(|| panic!("category {wanted:?} is in the default taxonomy"))
    };
    let bill = |name: &str,
                minor: i64,
                bill_type: &str,
                anchor_date: NaiveDate,
                autopay: bool,
                category: &str,
                description: Option<&str>| CreateRecurringBillInput {
        name: name.to_owned(),
        amount: usd(minor),
        bill_type: bill_type.to_owned(),
        frequency: "monthly".to_owned(),
        anchor_date: anchor_date.to_string(),
        autopay_account_id: Some(checking.clone()),
        autopay: Some(autopay),
        description: description.map(str::to_owned),
        source_merchant_key: None,
        category_id: Some(category_id(category)),
        tag_ids: Vec::new(),
        idempotency_key: String::new(),
    };
    // (name, minor, type, due day-of-month, autopay, category, description)
    type BillSpec<'a> = (&'a str, i64, &'a str, u32, bool, &'a str, Option<&'a str>);
    let bill_specs: &[BillSpec] = &[
        (
            "Foxglove Home Lending",
            285_000,
            "rent_mortgage",
            1,
            true,
            "Rent/Mortgage",
            None,
        ),
        (
            "Little Acorns Daycare",
            145_000,
            "childcare",
            1,
            false,
            "Childcare",
            Some("Tuition, both kids"),
        ),
        (
            "Glassmoor Utilities",
            18_432,
            "utility",
            8,
            false,
            "Utilities",
            Some("Electric + water + trash"),
        ),
        (
            "Wrenfield Auto Insurance",
            21_050,
            "insurance",
            25,
            false,
            "Auto Insurance",
            None,
        ),
        (
            "Fernwick Fiber Internet",
            8_999,
            "subscription",
            18,
            true,
            "Internet/Phone",
            None,
        ),
        (
            "Nimbuswire Mobile",
            9_500,
            "subscription",
            20,
            true,
            "Internet/Phone",
            None,
        ),
        (
            "Pollywog Streaming",
            1_549,
            "subscription",
            10,
            true,
            "Lifestyle",
            None,
        ),
        (
            "Oakhollow Community Pool",
            4_500,
            "membership",
            5,
            true,
            "Lifestyle",
            None,
        ),
    ];
    let mut bill_ids: Vec<(String, String)> = Vec::new();
    for (name, minor, bill_type, day, autopay, category, description) in bill_specs {
        let created = create_recurring_bill_impl(
            &state,
            bill(
                name,
                *minor,
                bill_type,
                on_or_before(anchor, *day),
                *autopay,
                category,
                *description,
            ),
        )
        .expect("bill");
        bill_ids.push(((*name).to_owned(), created.event_id));
    }
    // The past-due one: a manual bill whose imported payment history (below) only covers
    // occurrences strictly before `piano_anchor` — everything from `piano_anchor` up to
    // "today" (roughly seven monthly occurrences, 200 days back) has no matching
    // transaction, so Cash Flow shows exactly one unconfirmed obligation (ADR 0058), its
    // most recent occurrence. Before personal-cfo-5ie.10, those ~seven unmatched monthly
    // occurrences would ALL have surfaced as separate past-due rows; keeping the anchor
    // within a few days of "today" was the workaround, so only one could ever accumulate.
    // `read_unconfirmed_past_due` now caps a bill's pre-creation backlog to its single
    // newest occurrence, so the anchor no longer has to stay artificially recent — this
    // value is deliberately far enough back to keep proving that.
    let piano_anchor = at(-200);
    create_recurring_bill_impl(
        &state,
        bill(
            PAST_DUE_BILL,
            11_250,
            "other",
            piano_anchor,
            false,
            "Education",
            Some("Weekly lessons, billed monthly"),
        ),
    )
    .expect("piano bill");

    // DCA into the brokerage every mid-month (a real aggregate outflow, 9h0.1).
    create_recurring_transfer_impl(
        &state,
        CreateRecurringTransferInput {
            source_account_id: checking.clone(),
            dest_account_id: brokerage.clone(),
            amount: usd(50_000),
            frequency: "monthly".to_owned(),
            anchor_date: on_or_before(anchor, 15).to_string(),
            idempotency_key: String::new(),
        },
    )
    .expect("dca transfer");

    // ── A year of checking history, imported from a bank CSV ───────────────────
    // Descriptions carry the bank's spelling (processor prefixes, reference numbers) so
    // the merchant normalizer and the recurring matcher have real work to do. Paychecks
    // and bills cover the whole window (bills paid on their due days, auto-reconciled to
    // their occurrences), EXCEPT the current daycare occurrence (confirmed by hand below)
    // and the piano studio's latest one (left past due). Everyday activity stays recent.
    let mut rows: Vec<CsvRow> = Vec::new();
    for payday in &ledgerline_paydays {
        rows.push(csv_row(*payday, "LEDGERLINE SYSTEMS PAYROLL 0412", 385_000));
    }
    for payday in &northhollow_paydays {
        rows.push(csv_row(*payday, "NORTHHOLLOW HEALTH PAYROLL", 291_240));
    }
    // A side gig not modeled as income yet — the onboarding "detected income" suggestion.
    for date in monthly_back(at(-7), 3) {
        rows.push(csv_row(date, "BRAMBLEWICK TUTORING PAYOUT", 24_000));
    }
    let latest_daycare = on_or_before(anchor, 1);
    let utility_amounts = [-17_610, -19_145, -18_432, -17_988];
    for (name, minor, _, day, _, _, _) in bill_specs {
        for (index, due) in monthly_on(*day, history_start, anchor).iter().enumerate() {
            let (description, amount) = match *name {
                "Foxglove Home Lending" => ("FOXGLOVE HOME LENDING 8820417", *minor),
                "Little Acorns Daycare" if *due == latest_daycare => continue,
                "Little Acorns Daycare" => ("LITTLE ACORNS DAYCARE", *minor),
                "Glassmoor Utilities" => (
                    "GLASSMOOR UTILITIES 4471820",
                    utility_amounts[index % utility_amounts.len()],
                ),
                "Wrenfield Auto Insurance" => ("WRENFIELD AUTO INSURANCE", *minor),
                "Fernwick Fiber Internet" => ("FERNWICK FIBER INTERNET", *minor),
                "Nimbuswire Mobile" => ("NIMBUSWIRE MOBILE 555201", *minor),
                "Pollywog Streaming" => ("POLLYWOG STREAMING", *minor),
                "Oakhollow Community Pool" => ("OAKHOLLOW COMMUNITY POOL", *minor),
                other => panic!("no CSV spelling for bill {other:?}"),
            };
            rows.push(csv_row(*due, description, -amount.abs()));
        }
    }
    // The piano studio: every previous occurrence in the window paid, the latest one not.
    for date in monthly_back(piano_anchor, 13)
        .into_iter()
        .filter(|date| *date >= history_start && *date < piano_anchor)
    {
        rows.push(csv_row(date, "MAPLE STREET PIANO STUDIO", -11_250));
    }
    // A gym not modeled as a bill yet — the "suggested recurring" surface.
    for date in monthly_back(at(-4), 3) {
        rows.push(csv_row(date, "IRONFERN FITNESS", -5_200));
    }
    // Everyday debit activity, including one exact duplicate row (the bank export
    // repeated it), which the import flags into the Money Inbox for review.
    rows.push(csv_row(at(-3), "SQ *HARBOR FARMERS MARKET", -2_450));
    rows.push(csv_row(at(-2), "LANTERN GROCERY 00218804", -8_643));
    rows.push(csv_row(at(-2), "LANTERN GROCERY 00218804", -8_643));
    rows.push(csv_row(at(-12), "ATM WITHDRAWAL", -10_000));
    rows.push(csv_row(at(-15), "CHECK 1207", -6_000));
    rows.push(csv_row(at(-22), "CITY OF HARBORVIEW PARKING", -1_800));
    rows.push(csv_row(at(-40), "SUNNY MEADOW PEDIATRICS", -3_500));
    rows.push(csv_row(at(-55), "ANVIL AND OAK HARDWARE", -14_270));
    let csv = csv_text(&mut rows);
    let import = import_batch_impl(
        &state,
        ImportBatchInput {
            data: csv.into_bytes(),
            filename: Some("saltmarsh-checking.csv".to_owned()),
            target_account_id: checking.clone(),
            plugin_id: Some("generic-csv".to_owned()),
            preset_id: None,
            column_mapping: None,
            default_currency: Some("USD".to_owned()),
            date_format: Some("%Y-%m-%d".to_owned()),
            idempotency_key: String::new(),
        },
    )
    .expect("csv import");
    assert!(
        import.flagged >= 1,
        "the repeated grocery row must be flagged as a suspected duplicate"
    );
    // The household has reviewed most of that history; leave the newest few
    // unreviewed so the Money Inbox has a small, believable review queue.
    let unreviewed: Vec<String> = transaction_page_impl(
        &state,
        TransactionPageInput {
            query: None,
            account_ids: vec![],
            with_balances: false,
            category_id: None,
            tag_id: None,
            recurring_event_id: None,
            from_date: None,
            to_date: None,
            unreviewed_only: true,
            sort: "newest".to_owned(),
            limit: 500,
            offset: 0,
        },
    )
    .expect("unreviewed page")
    .rows
    .into_iter()
    .skip(3)
    .map(|row| row.transaction_id)
    .collect();
    mark_inbox_reviewed_bulk_impl(&state, String::new(), unreviewed).expect("bulk review");

    // The current daycare occurrence was paid by hand: confirm it early (ADR 0026 §19),
    // which posts the real outflow and stops the forecast from projecting it again.
    let daycare_id = bill_ids
        .iter()
        .find(|(name, _)| name == "Little Acorns Daycare")
        .map(|(_, id)| id.clone())
        .expect("daycare bill");
    confirm_obligation_early_impl(
        &state,
        ConfirmObligationEarlyInput {
            recurring_event_id: daycare_id,
            scheduled_date: latest_daycare.to_string(),
            actual_amount: usd(145_000),
            actual_date: latest_daycare.to_string(),
            paying_account_id: checking.clone(),
            idempotency_key: String::new(),
        },
    )
    .expect("confirm daycare");

    // ── Tags + six weeks of card spending (manual entries with merchant notes) ──
    let tags: Vec<(String, String)> = ["kids", "reimbursable", "vacation"]
        .into_iter()
        .map(|name| {
            let created =
                create_tag_impl(&state, name.to_owned(), None, String::new()).expect("tag");
            (created.tag_id, name.to_owned())
        })
        .collect();
    type Row<'a> = (i64, i64, &'a str, &'a str, Option<&'a str>, Option<&'a str>);
    // (days before the anchor, minor, merchant, card, category, tag)
    let card_rows: &[Row] = &[
        (
            40,
            -14_382,
            "Lantern Grocery",
            "copperleaf",
            Some("Groceries"),
            None,
        ),
        (
            38,
            -6_450,
            "Wagonwheel Fuel Stop",
            "tidepool",
            Some("Gas"),
            None,
        ),
        (
            36,
            -8_925,
            "Sunny Meadow Pediatrics copay",
            "copperleaf",
            Some("Medical"),
            Some("kids"),
        ),
        (
            34,
            -23_411,
            "Cartwheel Wholesale Club",
            "tidepool",
            Some("Groceries"),
            None,
        ),
        (
            32,
            -4_875,
            "Casa Verde Taqueria",
            "copperleaf",
            Some("Restaurants"),
            None,
        ),
        (
            30,
            -12_990,
            "Parcelmoon Online",
            "copperleaf",
            Some("Lifestyle"),
            None,
        ),
        (
            28,
            -3_250,
            "Kettle Drum Coffee",
            "copperleaf",
            Some("Coffee"),
            None,
        ),
        (
            26,
            -15_637,
            "Thornberry Market",
            "copperleaf",
            Some("Groceries"),
            None,
        ),
        (
            25,
            -5_899,
            "Summit Gas and Go",
            "tidepool",
            Some("Gas"),
            None,
        ),
        (
            23,
            -7_425,
            "Coop and Kettle",
            "copperleaf",
            Some("Restaurants"),
            Some("kids"),
        ),
        (
            21,
            -19_204,
            "Cartwheel Wholesale Club",
            "tidepool",
            Some("Groceries"),
            None,
        ),
        (20, -11_500, "Clip and Curl Salon", "copperleaf", None, None),
        (
            18,
            -8_770,
            "Hopalong Rides (airport)",
            "copperleaf",
            Some("Rideshare"),
            Some("reimbursable"),
        ),
        (
            16,
            -13_244,
            "Lantern Grocery",
            "copperleaf",
            Some("Groceries"),
            None,
        ),
        (
            14,
            -6_125,
            "Wagonwheel Fuel Stop",
            "tidepool",
            Some("Gas"),
            None,
        ),
        (
            13,
            -32_180,
            "Puffin Regional Air",
            "copperleaf",
            Some("Lifestyle"),
            Some("vacation"),
        ),
        (
            12,
            -9_640,
            "Trattoria Luna",
            "copperleaf",
            Some("Restaurants"),
            None,
        ),
        (
            10,
            -17_892,
            "Thornberry Market",
            "copperleaf",
            Some("Groceries"),
            None,
        ),
        (
            8,
            -4_212,
            "Marigold Pharmacy",
            "copperleaf",
            Some("Pharmacy"),
            None,
        ),
        (
            7,
            -21_305,
            "Cartwheel Wholesale Club",
            "tidepool",
            Some("Groceries"),
            None,
        ),
        (
            6,
            -7_150,
            "Anvil and Oak Hardware",
            "copperleaf",
            Some("Maintenance"),
            None,
        ),
        (
            4,
            -5_480,
            "Millstone Bakery Cafe",
            "copperleaf",
            Some("Restaurants"),
            None,
        ),
        (
            2,
            -14_820,
            "Lantern Grocery",
            "copperleaf",
            Some("Groceries"),
            None,
        ),
        (
            1,
            -6_890,
            "Summit Gas and Go",
            "tidepool",
            Some("Gas"),
            None,
        ),
    ];
    for (ago, minor, merchant, which, category, tag) in card_rows {
        let account_id = if *which == "copperleaf" {
            &copperleaf
        } else {
            &tidepool
        };
        spend(
            &state,
            account_id,
            *minor,
            at(-ago),
            merchant,
            *category,
            &categories,
            *tag,
            &tags,
        );
    }
    // A few checking-side one-offs recorded by hand.
    spend(
        &state,
        &checking,
        -30_000,
        at(-18),
        "Babysitter (Saturday night)",
        Some("Childcare"),
        &categories,
        Some("kids"),
        &tags,
    );
    spend(
        &state,
        &checking,
        12_500,
        at(-14),
        "Neighborhood marketplace sale — bike",
        None,
        &categories,
        None,
        &tags,
    );

    // One-off transfers: last month's card payments + a savings top-up.
    for (source, dest, minor, ago) in [
        (&checking, &copperleaf, 187_240, 24),
        (&checking, &tidepool, 64_410, 17),
        (&checking, &savings, 50_000, 16),
    ] {
        record_transfer_impl(
            &state,
            RecordTransferInput {
                source_account_id: (*source).clone(),
                dest_account_id: (*dest).clone(),
                amount: usd(minor),
                occurred_at: format!("{}T16:00:00Z", at(-ago)),
                idempotency_key: String::new(),
            },
        )
        .expect("transfer");
    }

    // ── A linked connection in a healthy, synced state (ADR 0060) ───────────────
    // The deterministic mock adapter, accounts-only, so no fixture rows land in the ledger
    // and no real token or network is ever involved. Both external accounts are mapped.
    let adapter = MockConnector::with_capabilities(CapabilitySet {
        accounts: true,
        transactions: false,
        balances: false,
        holdings: false,
        liabilities: false,
    })
    .with_id(DEMO_ADAPTER_ID);
    let link = connector_link_impl(
        &state,
        &adapter,
        ConnectorLinkInput {
            adapter_id: DEMO_ADAPTER_ID.to_owned(),
            setup_token: "mock-setup-token".to_owned(),
        },
    )
    .expect("link connection");
    for (external_id, account_id) in [
        ("mock-acct-checking", &checking),
        ("mock-acct-card", &copperleaf),
    ] {
        connector_set_account_link_impl(
            &state,
            ConnectorSetAccountLinkInput {
                connection_id: link.connection_id.clone(),
                external_id: external_id.to_owned(),
                account_id: Some(account_id.clone()),
            },
        )
        .expect("map external account");
    }
    let sync = connector_sync_impl(
        &state,
        &adapter,
        ConnectorSyncInput {
            connection_id: link.connection_id.clone(),
            idempotency_key: String::new(),
        },
    )
    .expect("sync connection");
    assert_eq!(
        sync.status,
        "synced",
        "{}",
        sync.message.unwrap_or_default()
    );

    // ── Forecast furniture: comfort band, one-time events, composable scenarios ──
    set_minimum_cash_floor_impl(&state, usd(500_000)).expect("floor");
    set_comfort_band_upper_impl(&state, Some(usd(1_200_000))).expect("band upper");
    create_manual_future_entry_impl(
        &state,
        CreateManualFutureEntryInput {
            amount: usd(-320_000),
            date: at(44).to_string(),
            label: "Property tax (2nd installment)".to_owned(),
            account_id: None,
        },
    )
    .expect("property tax");
    create_manual_future_entry_impl(
        &state,
        CreateManualFutureEntryInput {
            amount: usd(250_000),
            date: at(29).to_string(),
            label: "Quarterly bonus".to_owned(),
            account_id: None,
        },
    )
    .expect("bonus");

    let scenario = |name: &str, description: &str| {
        create_scenario_impl(
            &state,
            CreateScenarioInput {
                name: name.to_owned(),
                description: Some(description.to_owned()),
            },
        )
        .expect("scenario")
        .id
    };
    // 1. Parental leave: two pay regimes on Rowan's income, composed as windows.
    let leave = scenario(
        "Parental leave (fall)",
        "Rowan's leave: short-term disability pay, then the unpaid weeks",
    );
    let leave_regime = |new_amount_minor: i64, effective: NaiveDate, end: Option<NaiveDate>| {
        CreateForecastAssumptionInput {
            kind: "income_amount".to_owned(),
            scenario_id: Some(leave.clone()),
            target_entity_id: Some(northhollow_id.clone()),
            new_amount_minor: Some(new_amount_minor),
            effective_date: Some(effective.to_string()),
            end_date: end.map(|d| d.to_string()),
            ..Default::default()
        }
    };
    create_forecast_assumption_impl(&state, leave_regime(194_000, at(75), Some(at(120))))
        .expect("disability regime");
    create_forecast_assumption_impl(&state, leave_regime(149_800, at(120), None))
        .expect("unpaid regime");
    update_scenario_impl(
        &state,
        UpdateScenarioInput {
            id: leave.clone(),
            status: "active".to_owned(),
            name: None,
        },
    )
    .expect("activate leave scenario");
    // 2. Kitchen remodel: two one-time outflows, with an expiry (ADR 0051 §3).
    let remodel = scenario(
        "Kitchen remodel",
        "Contractor quote: deposit up front, balance at completion",
    );
    for (minor, ago, label) in [
        (-1_450_000, 60, "Contractor deposit"),
        (-980_000, 110, "Remodel — final payment"),
    ] {
        create_forecast_assumption_impl(
            &state,
            CreateForecastAssumptionInput {
                kind: "one_time_event".to_owned(),
                scenario_id: Some(remodel.clone()),
                amount: Some(usd(minor)),
                date: Some(at(ago).to_string()),
                label: Some(label.to_owned()),
                ..Default::default()
            },
        )
        .expect("remodel event");
    }
    set_scenario_expiry_impl(
        &state,
        SetScenarioExpiryInput {
            id: remodel,
            expires_on: Some(at(200).to_string()),
        },
    )
    .expect("remodel expiry");
    // 3. A daycare rate increase: a bill-amount override, stackable with the others
    // (ADR 0059 — selection order is precedence).
    let daycare_bill_id = bill_ids
        .iter()
        .find(|(name, _)| name == "Little Acorns Daycare")
        .map(|(_, id)| id.clone())
        .expect("daycare bill");
    let rate = scenario(
        "Daycare rate increase",
        "The center's announced 10% tuition increase next month",
    );
    create_forecast_assumption_impl(
        &state,
        CreateForecastAssumptionInput {
            kind: "bill_amount".to_owned(),
            scenario_id: Some(rate),
            target_entity_id: Some(daycare_bill_id),
            new_amount_minor: Some(159_500),
            effective_date: Some(at(30).to_string()),
            ..Default::default()
        },
    )
    .expect("rate increase");

    SeededVault { id, path, state }
}

// ── The manual seeding tool (real app-data root) ─────────────────────────────

#[test]
#[ignore = "manual seeding tool — set PCFO_SEED_ROOT and run with --ignored"]
fn seed_polish_demo_vault() {
    let Ok(root) = std::env::var("PCFO_SEED_ROOT") else {
        panic!("set PCFO_SEED_ROOT to the app-data root to seed a demo vault");
    };
    let root = PathBuf::from(root);
    assert!(
        root.is_dir(),
        "PCFO_SEED_ROOT must exist: {}",
        root.display()
    );
    // The household's "today": the local date, or a pinned one for a reproducible seed.
    let anchor = match std::env::var("PCFO_SEED_ANCHOR") {
        Ok(raw) => NaiveDate::parse_from_str(raw.trim(), "%Y-%m-%d")
            .expect("PCFO_SEED_ANCHOR must be YYYY-MM-DD"),
        Err(_) => chrono::Local::now().date_naive(),
    };

    let seeded = seed_demo_vault(&root, anchor);
    println!(
        "Seeded '{VAULT_NAME}' vault (id {}) at {}",
        seeded.id,
        seeded.path.display()
    );
    println!("Anchor date: {anchor}");
    println!("Passphrase: {PASSWORD}");
}

// ── The CI guard: every screenshot surface has content ───────────────────────

#[test]
fn seeded_demo_vault_populates_every_screenshot_surface() {
    let root = TempDir::new().expect("temp root");
    let seeded = seed_demo_vault(root.path(), fixed_anchor());
    let state = &seeded.state;

    // Accounts and debt: every cashflow role, subtypes, cash tiers, terms, payoff, cards.
    let accounts = account_list_impl(state).expect("accounts");
    for role in [
        "liquid_cash",
        "credit_facility",
        "loan_liability",
        "investment_asset",
        "real_asset",
    ] {
        assert!(
            accounts.iter().any(|a| a.cashflow_role == role),
            "an account with role {role} is seeded"
        );
    }
    assert!(accounts.iter().all(|a| a.subtype.is_some()));
    assert!(
        accounts
            .iter()
            .any(|a| a.linked_account_name.is_some() && a.cashflow_role == "real_asset"),
        "a real asset is linked to its financing liability"
    );
    let tiers = cash_tiers_impl(state).expect("cash tiers");
    assert!(tiers.spendable.minor_units > 0 && tiers.reserve.minor_units > 0);
    let terms = debt_terms_list_impl(state, &[]).expect("debt terms");
    assert!(terms.len() >= 3, "cards + the auto loan carry terms");
    assert!(terms.iter().any(|t| t.fixed_amount_minor.is_some()));
    let plans = debt_payoff_comparison_impl(state, 20_000, &[]).expect("payoff comparison");
    assert!(
        !plans.is_empty(),
        "the payoff comparison has debts to order"
    );
    let cards = card_statement_forecast_impl(state).expect("card forecast");
    assert!(
        cards.iter().any(|c| !c.cycles.is_empty()),
        "a card projects statement cycles"
    );

    // Transactions and cross-account analytics (ADR 0037).
    let page = transaction_page_impl(
        state,
        TransactionPageInput {
            query: None,
            account_ids: vec![],
            with_balances: false,
            category_id: None,
            tag_id: None,
            recurring_event_id: None,
            from_date: None,
            to_date: None,
            unreviewed_only: false,
            sort: "newest".to_owned(),
            limit: 500,
            offset: 0,
        },
    )
    .expect("transactions");
    assert!(page.total >= 150, "a year of history: {}", page.total);
    assert!(page.rows.iter().any(|r| r.category_id.is_some()));
    assert!(page.rows.iter().any(|r| !r.tag_ids.is_empty()));
    let history = cash_flow_history_impl(state, 90).expect("cash flow history");
    assert!(!history.accounts.is_empty());

    // Dashboard / forecast + bills: income, bills, auto-reconcile, past-due, upcoming.
    assert_eq!(income_source_list_impl(state).expect("income").len(), 2);
    let bills = recurring_bill_list_impl(state).expect("bills");
    assert!(bills.len() >= 9);
    assert!(
        bills
            .iter()
            .any(|b| !b.autopay_enabled && b.next_due_date.is_some()),
        "a manual bill has an upcoming occurrence to confirm"
    );
    let utilities = bills
        .iter()
        .find(|b| b.name == "Glassmoor Utilities")
        .expect("utilities bill");
    let occurrences = recurring_bill_history_impl(state, utilities.id.clone()).expect("history");
    assert!(
        occurrences
            .iter()
            .any(|o| o.status == "paid" && o.linked_transaction_id.is_some()),
        "imported payments auto-reconcile onto the bill's occurrences"
    );
    let past_due = unconfirmed_past_due_impl(state).expect("past due");
    let piano_rows: Vec<_> = past_due
        .iter()
        .filter(|o| o.name == PAST_DUE_BILL)
        .collect();
    assert_eq!(
        piano_rows.len(),
        1,
        "the piano studio's anchor (200 days back) is well before the imported history \
         starts — personal-cfo-5ie.10: exactly its newest occurrence surfaces, not a stack \
         of stale monthly rows: {past_due:?}"
    );
    // On or before the anchor, the piano studio is the ONLY unconfirmed obligation: the
    // imported year links every other occurrence and the daycare one was confirmed by
    // hand. (Occurrences between the pinned anchor and the real clock are out of scope.)
    let anchor_text = fixed_anchor().to_string();
    let stale: Vec<&str> = past_due
        .iter()
        .filter(|o| o.scheduled_date <= anchor_text && o.name != PAST_DUE_BILL)
        .map(|o| o.name.as_str())
        .collect();
    assert!(
        stale.is_empty(),
        "linked and confirmed occurrences are not past due: {stale:?}"
    );
    // Detected income and bills (the onboarding suggestions).
    let income_candidates = income_candidates_impl(state).expect("income candidates");
    assert!(
        income_candidates
            .iter()
            .any(|c| c.merchant_key.contains("BRAMBLEWICK")),
        "the unmodeled side gig is suggested as income: {income_candidates:?}"
    );
    assert!(
        !income_candidates
            .iter()
            .any(|c| c.merchant_key.contains("PAYROLL")),
        "modeled paychecks are not re-suggested"
    );
    let bill_candidates = recurring_candidates_impl(state).expect("bill candidates");
    assert!(
        bill_candidates
            .iter()
            .any(|c| c.merchant_key.contains("IRONFERN")),
        "the unmodeled gym is suggested as a recurring bill: {bill_candidates:?}"
    );
    let forecast = future_cash_forecast_impl(state, 120, vec![]).expect("base forecast");
    assert!(!forecast.days.is_empty());

    // Scenarios: at least two, composable in one run (ADR 0059).
    let scenarios = scenario_list_impl(state).expect("scenarios");
    assert!(scenarios.len() >= 2);
    assert!(scenarios.iter().all(|s| s.event_count >= 1));
    assert!(scenarios.iter().any(|s| s.expires_on.is_some()));
    let ids: Vec<String> = scenarios.iter().map(|s| s.id.clone()).collect();
    let composed = future_cash_forecast_impl(state, 120, ids).expect("composed forecast");
    assert_eq!(composed.days.len(), forecast.days.len());

    // Money Inbox: a flagged duplicate awaiting review, a stale balance, unreviewed rows.
    let inbox = money_inbox_list_impl(state).expect("inbox");
    let kinds = |kind: &str| inbox.iter().filter(|i| i.item_kind == kind).count();
    assert!(kinds("imported_waiting_commit") >= 1, "{inbox:?}");
    assert!(kinds("stale_balance") >= 1, "{inbox:?}");
    assert!(kinds("unreviewed_transaction") >= 1, "{inbox:?}");
    assert_eq!(kinds("connector_error"), 0, "the connection is healthy");
    // Import: the flagged row has a committed twin to compare against.
    let flagged = inbox
        .iter()
        .find(|i| i.item_kind == "imported_waiting_commit")
        .expect("flagged duplicate");
    let twins = duplicate_candidates_impl(state, flagged.target_id.clone()).expect("candidates");
    assert!(
        !twins.is_empty(),
        "the duplicate review panel has a counterpart"
    );

    // Connection health: linked, mapped, synced, no error — and no real token anywhere.
    let connections = connector_connections_impl(state).expect("connections");
    assert_eq!(connections.len(), 1);
    let connection = &connections[0];
    assert_eq!(connection.adapter_id, DEMO_ADAPTER_ID);
    assert!(connection.last_error.is_none());
    assert!(connection.last_synced_at.is_some());
    assert_eq!(connection.links.len(), 2);
    assert!(connection.links.iter().all(|l| l.account_id.is_some()));
    assert!(connection.links.iter().all(|l| l.last_synced_on.is_some()));

    // Vault / backup: registered + active, healthy, and a backup that restores.
    let vaults = list_vaults_impl(state).expect("vault list");
    assert!(vaults
        .vaults
        .iter()
        .any(|v| v.name == VAULT_NAME && v.is_active));
    assert!(vault_health_impl(state).expect("health").is_healthy);
    let account_count = accounts.len() as u32;
    let package = root.path().join("polish-demo.pcfobk");
    export_backup_impl(state, package.to_string_lossy().into_owned()).expect("export backup");
    let restore_root = TempDir::new().expect("restore root");
    let restored = AppState::new(VaultController::open(restore_root.path().join("vault.db")));
    let status = restore_backup_impl(
        &restored,
        package.to_string_lossy().into_owned(),
        PASSWORD.to_owned(),
    )
    .expect("restore backup");
    assert_eq!(status.state, VaultStateDto::Unlocked);
    assert_eq!(status.account_count, Some(account_count));

    // The seeded vault reopens with the demo passphrase, like the app would.
    lock_vault_impl(state).expect("lock");
    let path = seeded.path.clone();
    drop(seeded);
    let reopened = AppState::new(VaultController::open(path));
    let status = unlock_vault_impl(&reopened, PASSWORD.to_owned()).expect("unlock");
    assert_eq!(status.state, VaultStateDto::Unlocked);
    assert_eq!(status.account_count, Some(account_count));
}
