//! DohFlow desktop application (Tauri).
//!
//! The IPC command surface and its serde→TypeScript binding generation live in
//! the [`ipc`] module (personal-cfo-40t). [`ipc_builder`] is the single source
//! of truth for the command list, shared by the running app ([`run`]) and the
//! binding exporter ([`export_bindings`] / the `export_bindings` binary).

pub mod data_dir;
pub mod ipc;
pub mod state;
pub mod update;
pub mod vault_registry;

pub use state::AppState;

// Link anchor (personal-cfo-cu8): importer plugins contribute only `inventory`
// submissions, which the linker would dead-strip from a dependency nothing
// references. Naming the crate here forces it to be linked, so `GenericCsv`
// registers in the compile-time registry the import IPC resolves from. Each new
// importer crate gets a line here.
use csv_importer as _;
use ofx_importer as _;
// Connector adapters register the same way (personal-cfo-gglk, ADR 0060).
use simplefin_adapter as _;

use tauri_specta::{collect_commands, Builder};

/// Build the tauri-specta [`Builder`] holding the full IPC command surface.
///
/// Used both to mount the live `invoke` handler in [`run`] and to generate the
/// TypeScript bindings, so the runtime commands and the generated types can
/// never drift apart.
#[must_use]
pub fn ipc_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new().commands(collect_commands![
        ipc::commands::vault_status,
        ipc::commands::no_reset_warning,
        ipc::commands::acknowledge_no_reset_warning,
        ipc::commands::create_vault,
        ipc::commands::unlock_vault,
        ipc::commands::lock_vault,
        ipc::commands::change_password,
        ipc::commands::delete_vault,
        ipc::commands::list_vaults,
        ipc::commands::create_vault_named,
        ipc::commands::switch_vault,
        ipc::commands::rename_vault,
        ipc::commands::vault_health,
        ipc::commands::rebuild_read_models,
        ipc::commands::create_account,
        ipc::commands::record_transaction,
        ipc::commands::record_transfer,
        ipc::commands::create_source_batch,
        ipc::commands::attach_source_record,
        ipc::commands::update_batch_state,
        ipc::commands::import_batch,
        ipc::commands::import_preview_columns,
        ipc::commands::update_account,
        ipc::commands::set_account_subtype,
        ipc::commands::set_account_note,
        ipc::commands::set_account_link,
        ipc::commands::set_debt_terms,
        ipc::commands::set_card_statement_balance,
        ipc::commands::debt_terms,
        ipc::commands::debt_terms_list,
        ipc::commands::archive_account,
        ipc::commands::reinstate_account,
        ipc::commands::account_count,
        ipc::commands::cash_tiers,
        ipc::commands::cash_availability,
        ipc::commands::set_minimum_cash_floor,
        ipc::commands::comfort_band,
        ipc::commands::set_comfort_band_upper,
        ipc::commands::band_drift_signal,
        ipc::commands::household_timezone,
        ipc::commands::set_household_timezone,
        ipc::commands::forecast_readiness,
        ipc::commands::pending_capability_unlocks,
        ipc::commands::acknowledge_capability,
        ipc::commands::apply_merchant_memory,
        ipc::commands::auto_categorize_on_import,
        ipc::commands::set_auto_categorize_on_import,
        ipc::commands::future_cash_series_selection,
        ipc::commands::set_future_cash_series_selection,
        ipc::commands::account_balance,
        ipc::commands::account_view,
        ipc::commands::create_income_source,
        ipc::commands::income_source_list,
        ipc::commands::update_income_source,
        ipc::commands::delete_income_source,
        ipc::commands::archive_income_source,
        ipc::commands::restore_income_source,
        ipc::commands::create_recurring_bill,
        ipc::commands::confirm_obligation_early,
        ipc::commands::unconfirm_obligation,
        ipc::commands::set_bill_autopay,
        ipc::commands::recurring_bill_list,
        ipc::commands::create_recurring_transfer,
        ipc::commands::recurring_transfer_list,
        ipc::commands::delete_recurring_transfer,
        ipc::commands::category_list,
        ipc::commands::create_category,
        ipc::commands::update_category,
        ipc::commands::move_category,
        ipc::commands::archive_category,
        ipc::commands::reinstate_category,
        ipc::commands::update_recurring_bill,
        ipc::commands::delete_recurring_bill,
        ipc::commands::archive_recurring_bill,
        ipc::commands::restore_recurring_bill,
        ipc::commands::base_currency,
        ipc::commands::set_base_currency,
        ipc::commands::account_list,
        ipc::commands::transaction_list,
        ipc::commands::transaction_page,
        ipc::commands::recategorize_transaction,
        ipc::commands::money_inbox_list,
        ipc::commands::accept_low_confidence_categories,
        ipc::commands::mark_inbox_reviewed_bulk,
        ipc::commands::transaction_rows_by_ids,
        ipc::commands::import_staged_anyway,
        ipc::commands::skip_staged_transaction,
        ipc::commands::snooze_inbox_item,
        ipc::commands::dismiss_inbox_item,
        ipc::commands::dismiss_recurring_suggestion,
        ipc::commands::future_cash_forecast,
        ipc::commands::future_cash_by_account,
        ipc::commands::cash_flow_history,
        ipc::commands::card_statement_forecast,
        ipc::commands::card_statement_history,
        ipc::commands::debt_payoff_comparison,
        ipc::commands::unconfirmed_past_due,
        ipc::commands::loan_double_count_warnings,
        ipc::commands::recurring_candidates,
        ipc::commands::income_candidates,
        ipc::commands::recurring_bill_history,
        ipc::commands::check_for_update,
        ipc::commands::build_info,
        ipc::commands::apply_update,
        ipc::commands::relaunch_app,
        ipc::commands::create_manual_future_entry,
        ipc::commands::manual_future_entry_list,
        ipc::commands::update_manual_future_entry,
        ipc::commands::delete_manual_future_entry,
        ipc::commands::create_scenario,
        ipc::commands::scenario_list,
        ipc::commands::update_scenario,
        ipc::commands::delete_scenario,
        ipc::commands::archive_scenario,
        ipc::commands::apply_scenario,
        ipc::commands::revert_scenario_apply,
        ipc::commands::spend_by_category,
        ipc::commands::clone_scenario,
        ipc::commands::set_scenario_expiry,
        ipc::commands::create_forecast_assumption,
        ipc::commands::forecast_assumption_list,
        ipc::commands::delete_forecast_assumption,
        ipc::commands::assert_balance,
        ipc::commands::account_unexplained,
        ipc::commands::convert_unexplained_to_transaction,
        ipc::commands::attach_document,
        ipc::commands::transaction_attachments,
        ipc::commands::imported_transaction_fields,
        ipc::commands::remove_attachment,
        ipc::commands::void_transaction,
        ipc::commands::mark_reviewed,
        ipc::commands::tag_list,
        ipc::commands::create_tag,
        ipc::commands::set_tags,
        ipc::commands::set_note,
        ipc::commands::set_splits,
        ipc::commands::transaction_splits,
        ipc::commands::duplicate_candidates,
        ipc::commands::export_backup,
        ipc::commands::export_transactions_csv,
        ipc::commands::restore_backup,
        // Connectors (personal-cfo-gglk, ADR 0060).
        ipc::commands::connector_link,
        ipc::commands::connector_connections,
        ipc::commands::connector_set_account_link,
        ipc::commands::connector_sync,
        ipc::commands::connector_auto_sync,
        ipc::commands::connector_forget,
    ])
}

/// Export the TypeScript bindings for the IPC surface to `path`.
///
/// `i64`/`u64` types (money minor units, op-log sequence numbers) are exported
/// as TypeScript `number`. A personal vault never approaches 2^53 minor units,
/// so this is lossless and far more ergonomic than `bigint` on the frontend.
///
/// # Errors
/// Returns an error if the bindings cannot be generated or written.
pub fn export_bindings(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    use specta_typescript::Typescript;

    ipc_builder().export(Typescript::default(), path)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Install the redacting `tracing` subscriber first, so kernel/app log lines
    // are scrubbed before any sink (plan §6.6, personal-cfo-2vs). This owns the
    // global logger; we deliberately do **not** also install `tauri-plugin-log`,
    // which would (a) panic on a second `set_logger` and (b) emit log lines that
    // bypass the redactor.
    observability::init();

    let builder = ipc_builder();

    tauri::Builder::default()
        // Native file dialogs for backup export/restore (personal-cfo-dvxm),
        // scoped to open/save in capabilities/default.json (ADR 0010).
        .plugin(tauri_plugin_dialog::init())
        // System-browser opener for the About card links and the Support row
        // (personal-cfo-n76x.18). The capability grants only `opener:allow-open-url`
        // scoped to `https://dohflow.app/*` (ADR 0010 addendum 2026-09-06); the
        // plugin itself rejects anything outside that scope as ForbiddenUrl.
        //
        // Built explicitly rather than through the plugin's `init()` default, so the
        // anchor-click interceptor is OFF. That default injects a script into EVERY
        // window which sends `target="_blank"` / modifier-clicked http(s)/mailto/tel
        // anchors straight to `open_url`, bypassing the frontend's allow-listing
        // helper. With it off, `openExternal.ts` is the only path to the command and
        // no script is injected into any window (`tests/acl_coverage.rs` pins this).
        .plugin(
            tauri_plugin_opener::Builder::new()
                .open_js_links_on_click(false)
                .build(),
        )
        // The real signed-artifact updater (personal-cfo-867.1.2, ADR 0068). Config-driven —
        // reads `plugins.updater.pubkey`/`endpoints` from tauri.conf.json, no builder options
        // needed. Release builds call its JS API (`@tauri-apps/plugin-updater`) directly from
        // `useSoftwareUpdate.ts`; the from-source dev updater (`update.rs`) is unaffected and
        // stays the path when `PCFO_BUILD_CHANNEL == dev`. Granted `updater:default` only
        // (capabilities/default.json).
        .plugin(tauri_plugin_updater::Builder::new().build())
        // Restarts the app after the updater installs a new binary (`Update::download_and_install`
        // relaunches nothing on its own — the frontend calls this plugin's `relaunch()` after a
        // successful install). Granted `process:allow-restart` only, in the DESTRUCTIVE capability
        // (capabilities/destructive.json) alongside `apply_update`/`relaunch_app` — swapping the
        // running binary is exactly the class of action that capability exists to gate.
        .plugin(tauri_plugin_process::init())
        .invoke_handler(builder.invoke_handler())
        .setup(|app| {
            use tauri::Manager;

            // Resolve the per-user vault location and classify it (NoVault /
            // Locked / CorruptNeedsRecovery) before the webview loads, so the UI
            // routes correctly on first paint. The DEK only enters memory once
            // the user unlocks (personal-cfo-8v2).
            //
            // A dev build (PCFO_BUILD_CHANNEL == "dev") never resolves to the
            // release identifier's own directory — ADR 0070, personal-cfo-he3xo.
            // This is what keeps `pnpm tauri dev` from ever touching the real
            // vault that /Applications/DohFlow.app holds.
            let data_dir = data_dir::resolve_data_dir(
                update::BUILD_CHANNEL,
                app.path().app_data_dir()?,
                std::env::var("PCFO_DATA_DIR").ok().map(std::path::PathBuf::from),
            );
            std::fs::create_dir_all(&data_dir)?;
            // Multi-vault registry (ADR 0042): register the pre-existing single vault in place and
            // open the active one. Backward-compatible — the active path is the legacy `vault.db`
            // until create-new / switch land, and a truly fresh install falls back to it too.
            let mut registry = vault_registry::VaultRegistry::load(&data_dir);
            let active_path = registry
                .bootstrap(&data_dir)
                .unwrap_or_else(|| data_dir.join("vault.db"));
            let _ = registry.save(&data_dir);
            let controller = finance_kernel::VaultController::open(active_path);
            app.manage(AppState::with_registry(controller, registry, data_dir));

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
