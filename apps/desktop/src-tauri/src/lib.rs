//! DohFlow desktop application (Tauri).
//!
//! The IPC command surface and its serde→TypeScript binding generation live in
//! the [`ipc`] module (personal-cfo-40t). [`ipc_builder`] is the single source
//! of truth for the command list, shared by the running app ([`run`]) and the
//! binding exporter ([`export_bindings`] / the `export_bindings` binary).

mod abandonable;
pub mod backup_job;
pub mod connector_attention;
pub mod connector_refresh;
pub mod data_dir;
pub mod ipc;
// Runtime isolation probe (personal-cfo-0hp6) — debug builds only.
#[cfg(debug_assertions)]
mod isolation_probe;
pub mod navigation_guard;
pub mod state;
pub mod touch_id;
pub mod update;
pub mod vault_registry;
pub mod windows;

pub use ipc::commands::lock_active_vault;
pub use state::AppState;

// Link anchor (personal-cfo-cu8): importer plugins contribute only `inventory`
// submissions, which the linker would dead-strip from a dependency nothing
// references. Naming the crate here forces it to be linked, so `GenericCsv`
// registers in the compile-time registry the import IPC resolves from. Each new
// importer crate gets a line here.
use csv_importer as _;
use ofx_importer as _;
// Source-app presets register the same way (personal-cfo-gvidg).
use source_presets as _;
// Connector adapters register the same way (personal-cfo-gglk, ADR 0060).
use lunchflow_adapter as _;
use simplefin_adapter as _;

use tauri_specta::{collect_commands, Builder};

/// The IPC command list, written once. `$desktop_only` is spliced in after the commands
/// every build has, so a target that lacks a command does not carry a second copy of
/// the list (`tests/acl_coverage.rs` reads this macro and `ipc_builder` as one list).
macro_rules! app_commands {
    ($($desktop_only:tt)*) => {
        collect_commands![
            ipc::commands::vault_status,
            ipc::commands::no_reset_warning,
            ipc::commands::acknowledge_no_reset_warning,
            ipc::commands::create_vault,
            ipc::commands::unlock_vault,
            ipc::commands::lock_vault,
            ipc::commands::change_password,
            ipc::commands::rotate_vault_key,
            ipc::commands::touch_id_status,
            ipc::commands::touch_id_enable,
            ipc::commands::touch_id_disable,
            ipc::commands::touch_id_unlock,
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
            ipc::commands::import_preview_accounts,
            ipc::commands::list_source_presets,
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
            ipc::commands::reinstate_income_source,
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
            ipc::commands::reinstate_recurring_transfer,
            ipc::commands::category_list,
            ipc::commands::create_category,
            ipc::commands::update_category,
            ipc::commands::move_category,
            ipc::commands::archive_category,
            ipc::commands::reinstate_category,
            ipc::commands::update_recurring_bill,
            ipc::commands::delete_recurring_bill,
            ipc::commands::reinstate_recurring_bill,
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
            ipc::commands::link_bill_to_loan,
            ipc::commands::unlink_bill_from_loan,
            ipc::commands::set_bill_not_loan_payment,
            ipc::commands::check_loan_overlap,
            ipc::commands::income_candidates,
            ipc::commands::recurring_bill_history,
            ipc::commands::build_info,
            ipc::commands::record_release_update_failure,
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
            ipc::commands::backup_schedule_settings,
            ipc::commands::backup_history,
            ipc::commands::export_backup,
            ipc::commands::configure_backup,
            ipc::commands::run_backup_now,
            ipc::commands::export_transactions_csv,
            // Local diagnostics (personal-cfo-lyd).
            ipc::commands::diagnostics_preview,
            ipc::commands::diagnostics_save,
            ipc::commands::diagnostics_discard,
            ipc::commands::restore_backup,
            ipc::commands::restore_backup_as_new_vault,
            // Connectors (personal-cfo-gglk, ADR 0060).
            ipc::commands::connector_link,
            ipc::commands::connector_enter_token,
            ipc::commands::connector_adapters,
            ipc::commands::connector_connections,
            ipc::commands::connector_set_account_link,
            ipc::commands::connector_set_refresh_cadence,
            ipc::commands::connector_create_mapped_account,
            ipc::commands::connector_sync,
            ipc::commands::connector_auto_sync,
            ipc::commands::open_provider_site,
            ipc::commands::connector_forget,
            $($desktop_only)*
        ]
    };
}

/// Build the tauri-specta [`Builder`] holding the full IPC command surface.
///
/// Used both to mount the live `invoke` handler in [`run`] and to generate the
/// TypeScript bindings, so the runtime commands and the generated types can
/// never drift apart.
#[must_use]
pub fn ipc_builder() -> Builder<tauri::Wry> {
    // The from-source self-update commands exist only in a desktop build
    // (personal-cfo-i93n3.11): a build for another platform has no command that
    // rebuilds, replaces or restarts the app, and no permission or capability that
    // would grant one (`capabilities/desktop-update.json`).
    #[cfg(desktop)]
    let commands = app_commands![
        ipc::commands::check_for_update,
        ipc::commands::apply_update,
        ipc::commands::relaunch_app,
    ];
    #[cfg(not(desktop))]
    let commands = app_commands![];
    Builder::<tauri::Wry>::new().commands(commands)
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
    // Specta emits spaces after union separators. Keep this committed artifact
    // deterministic and compatible with git diff --check, including on a fresh
    // regeneration in CI; this does not alter the generated TypeScript types.
    let generated = std::fs::read_to_string(path)?;
    let normalized = generated
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    std::fs::write(path, normalized)?;
    Ok(())
}

/// Which optional plugins [`register_plugins`] and [`run_with`] add to the app.
///
/// This is part of a generic embedding point: a crate that depends on this one
/// can build its own Tauri context and decide which of the app's optional
/// plugins its build carries. [`HostOptions::default`] is exactly what [`run`]
/// uses, so the default desktop build is unchanged. A plugin left out here is
/// not registered at all, so its commands do not exist in that build
/// (`tests/host_options.rs`), whatever a capability file says.
// No `PartialEq`: the hooks are function pointers, whose equality is not meaningful.
#[derive(Clone, Copy, Debug)]
pub struct HostOptions {
    /// Register the signed-artifact updater plugin (ADR 0068).
    pub updater: bool,
    /// Register the process plugin (restart after an update).
    pub process: bool,
    /// The device name to show in Settings when the host's system backups include this
    /// app's encrypted vault (personal-cfo-i93n3.9), or `None` when they do not, which is
    /// the desktop default and shows nothing. The host supplies the name; this crate
    /// hard-codes none. Kept short and plain: see [`HostOptions::system_backup_device_label`].
    pub system_backup_device: Option<&'static str>,
    /// Called once, at the end of setup, with the app's handle (personal-cfo-i93n3.13), so a
    /// host can keep it for its own lifecycle events. `None` (the default) does nothing.
    pub on_ready: Option<fn(&tauri::AppHandle)>,
    /// Called after EVERY transition to Unlocked, with the app's handle and the id of the vault
    /// that became unlocked (personal-cfo-i93n3.13, i93n3.14, i93n3.18): a vault created, a backup
    /// restored (as a fresh vault or as a new named one), a named vault created, and a password
    /// unlock. It is never called for a failed unlock, create or restore, and never for a
    /// transition to any other state. A transition that ends Unlocked calls it exactly once.
    /// The id is the vault's registry identifier, the same one the vault picker uses (a UUID in
    /// text form): never a path, a name or contents. It is `None` for a vault that is not
    /// registered yet (a legacy layout, or one just created before the registry knows it); a
    /// host must treat `None` as "unknown vault". `None` (the default) does nothing.
    pub on_unlocked: Option<fn(&tauri::AppHandle, Option<&str>)>,
    /// Consulted at every unlock with the id of the vault being unlocked (personal-cfo-i93n3.13,
    /// i93n3.18): `true` runs the engine's `PRAGMA integrity_check` before anything opens,
    /// migrates or writes, and a failure is the typed `VaultIntegrityCheckFailed` error. A host
    /// with a reason to distrust a vault's files (a restored install) answers `true` for each such
    /// vault until it has passed once. The id is as for `on_unlocked`; **a host MUST treat `None`
    /// as "run the check"**. `None` for the option (the default) never checks.
    pub unlock_integrity_check: Option<fn(Option<&str>) -> bool>,
}

/// The longest device name `build_info` passes to the UI.
const MAX_DEVICE_LABEL_CHARS: usize = 32;

impl HostOptions {
    /// Whether this build can update itself: a desktop target whose host kept the
    /// updater plugin (personal-cfo-i93n3.12). The shared UI reads it through
    /// `build_info` and shows no update surface, and calls no update command, when it
    /// is false. A non-desktop build has no updater whatever the options say.
    #[must_use]
    pub const fn self_update_available(self) -> bool {
        cfg!(desktop) && self.updater
    }
}

/// Call a host hook with its argument, or do nothing when the host set none
/// (personal-cfo-i93n3.13). Generic so a test can prove "fires when set, inert when `None`" for the
/// very function the app uses, without a native `AppHandle`.
#[doc(hidden)]
pub fn call_host_hook<A>(hook: Option<fn(&A)>, argument: &A) {
    if let Some(hook) = hook {
        hook(argument);
    }
}

impl HostOptions {
    /// The options for the next unlock of the vault with registry id `vault_id`: the integrity
    /// check when the host asks for it.
    #[must_use]
    pub fn unlock_options(self, vault_id: Option<&str>) -> finance_kernel::UnlockOptions {
        match self.unlock_integrity_check {
            Some(ask) if ask(vault_id) => finance_kernel::UnlockOptions::with_integrity_check(),
            _ => finance_kernel::UnlockOptions::default(),
        }
    }
}

impl HostOptions {
    /// The device name for the system-backup disclosure, or `None` if the host gave none
    /// or one the UI should not show: blank, over [`MAX_DEVICE_LABEL_CHARS`], or holding
    /// a control character. A bad value hides the disclosure rather than showing odd text.
    #[must_use]
    pub fn system_backup_device_label(self) -> Option<&'static str> {
        self.system_backup_device.map(str::trim).filter(|name| {
            !name.is_empty()
                && name.chars().count() <= MAX_DEVICE_LABEL_CHARS
                && !name.chars().any(char::is_control)
        })
    }
}

impl Default for HostOptions {
    fn default() -> Self {
        Self {
            updater: true,
            process: true,
            system_backup_device: None,
            on_ready: None,
            on_unlocked: None,
            unlock_integrity_check: None,
        }
    }
}

/// Register the app's plugins on `builder`, in the order the desktop app has always
/// used. The navigation guard, the dialog plugin and the scoped opener are always
/// added; the updater and process plugins follow `options`.
///
/// Generic over the runtime so a test can drive it on Tauri's mock runtime.
#[must_use]
#[cfg_attr(not(desktop), allow(unused_mut, unused_variables))]
pub fn register_plugins<R: tauri::Runtime>(
    builder: tauri::Builder<R>,
    options: HostOptions,
) -> tauri::Builder<R> {
    let mut builder = builder
        // Cancels any webview navigation away from the app's own origin (personal-cfo-2rf,
        // ADR 0010): the CSP limits what a page loads, not where it goes, so without this a
        // renderer could replace the trusted UI with a remote page. External pages open in
        // the system browser via the scoped opener grant below, never in a WebView.
        .plugin(navigation_guard::init())
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
        );
    // Self-update plugins exist only in a desktop build (personal-cfo-i93n3.11): on any
    // other target they are not compiled (Cargo.toml) or registered, so nothing here
    // downloads or swaps the app's own binary and `options` has no effect.
    #[cfg(desktop)]
    {
        if options.updater {
            // The real signed-artifact updater (personal-cfo-867.1.2, ADR 0068). Config-driven —
            // reads `plugins.updater.pubkey`/`endpoints` from tauri.conf.json, no builder options
            // needed. Release builds call its JS API (`@tauri-apps/plugin-updater`) directly from
            // `useSoftwareUpdate.ts`; the from-source dev updater (`update.rs`) is unaffected and
            // stays the path when `PCFO_BUILD_CHANNEL == dev`. Granted `updater:default` only
            // (capabilities/desktop-update.json).
            builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
        }
        if options.process {
            // Restarts the app after the updater installs a new binary (`Update::download_and_install`
            // relaunches nothing on its own — the frontend calls this plugin's `relaunch()` after a
            // successful install). Granted `process:allow-restart` only, beside `apply_update`/
            // `relaunch_app` in capabilities/desktop-update.json — swapping the running binary is
            // exactly the class of action that capability exists to gate.
            builder = builder.plugin(tauri_plugin_process::init());
        }
    }
    builder
}

/// Where a host's options wait for [`run`] (personal-cfo-i93n3.15). One host registration is
/// allowed, and only before `run` has resolved its options: after that the options are in use
/// and a registration would silently do nothing, so it is refused instead. A `Mutex`, not two
/// atomics, so a registration racing with the resolve is either fully before it or refused.
#[doc(hidden)]
pub struct HostRegistry(std::sync::Mutex<RegistryState>);

enum RegistryState {
    /// Nothing registered and nothing resolved yet.
    Open,
    /// A host registered its options; `run` has not resolved them yet.
    Registered(HostOptions),
    /// `run` has resolved its options; they are final.
    Resolved,
}

impl HostRegistry {
    /// An empty registry, open for one registration.
    #[must_use]
    pub const fn new() -> Self {
        Self(std::sync::Mutex::new(RegistryState::Open))
    }

    /// Register `options`. The first call before [`HostRegistry::resolve`] succeeds; any later
    /// call, whether a second registration or one after resolution, returns the options back and
    /// changes nothing.
    ///
    /// # Errors
    /// The unchanged `options`, when the registry is no longer open.
    pub fn register(&self, options: HostOptions) -> Result<(), HostOptions> {
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match *state {
            RegistryState::Open => {
                *state = RegistryState::Registered(options);
                Ok(())
            }
            RegistryState::Registered(_) | RegistryState::Resolved => Err(options),
        }
    }

    /// The registered options, or the defaults when none were registered. Closes the registry:
    /// from here on [`HostRegistry::register`] is refused.
    #[must_use]
    pub fn resolve(&self) -> HostOptions {
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let options = match *state {
            RegistryState::Registered(options) => options,
            RegistryState::Open | RegistryState::Resolved => HostOptions::default(),
        };
        *state = RegistryState::Resolved;
        options
    }
}

impl Default for HostRegistry {
    fn default() -> Self {
        Self::new()
    }
}

static HOST_REGISTRY: HostRegistry = HostRegistry::new();

/// Register the options [`run`] will use, for a host that starts the app through the mobile entry
/// point and so cannot pass them itself (personal-cfo-i93n3.15). Call it **once, before** `run`
/// (a host that owns its own entry point calls [`run_with`] instead and never needs this). The
/// desktop build never calls it, so `run` uses [`HostOptions::default`] there.
///
/// # Errors
/// The unchanged `options`, if one registration already happened or `run` has already resolved
/// its options. The earlier options stay in effect.
pub fn register_host_options(options: HostOptions) -> Result<(), HostOptions> {
    HOST_REGISTRY.register(options)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    run_with(tauri::generate_context!(), HOST_REGISTRY.resolve());
}

/// Run the app with its own Tauri context and the chosen optional plugins.
///
/// `context` is the caller's `tauri::generate_context!()` output: its
/// `tauri.conf.json`, capability files, content-security policy and windows
/// replace the ones compiled into this crate. [`run`] is
/// `run_with(tauri::generate_context!(), HostOptions::default())`.
///
/// The context must keep the app's access-control layout (ADR 0010): it needs the
/// app command permissions (`permissions/`) built into its ACL manifest, a
/// `main` window declared with `"create": false` (the app builds it in setup), and
/// a capability that grants the app commands to `main` and to no isolated window.
/// A context that does not grant the app's own commands to `main`, or that grants
/// them to an isolated window, is refused at startup
/// ([`context_confines_app_commands_to_main`]).
///
/// # Panics
/// If the supplied context does not confine the app commands to `main`, or if
/// Tauri cannot start the app (the same failure `run` has always reported).
pub fn run_with(mut context: tauri::Context<tauri::Wry>, options: HostOptions) {
    assert!(
        context_confines_app_commands_to_main(&mut context),
        "the supplied Tauri context does not confine the app commands to the `{}` window: \
         build its ACL from this app's permissions/ and capabilities, and grant them to \
         no isolated window (ADR 0010)",
        windows::MAIN
    );
    // Install the redacting `tracing` subscriber first, so kernel/app log lines
    // are scrubbed before any sink (plan §6.6, personal-cfo-2vs). This owns the
    // global logger; we deliberately do **not** also install `tauri-plugin-log`,
    // which would (a) panic on a second `set_logger` and (b) emit log lines that
    // bypass the redactor.
    observability::init();

    let builder = ipc_builder();

    register_plugins(tauri::Builder::default(), options)
        // The host's own options, for the commands that report what this build can do
        // (`build_info`'s self-update flag).
        .manage(options)
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            use tauri::Manager;

            // Resolve the per-user vault location and classify it (NoVault /
            // Locked / CorruptNeedsRecovery) before the webview loads, so the UI
            // routes correctly on first paint. The DEK only enters memory once
            // the user unlocks (personal-cfo-8v2).
            //
            // A dev build (PCFO_BUILD_CHANNEL == "dev") never resolves to the
            // release identifier's own directory — ADR 0070, personal-cfo-he3xo.
            // Neither does a debug-profile build under ANY channel label
            // (personal-cfo-qrh3t, ADR 0070 addendum) — `resolve_app_data_dir`
            // is `resolve_data_dir` wired to the CURRENT build's own profile,
            // so an explicit `PCFO_BUILD_CHANNEL=beta pnpm tauri dev` can no
            // longer point a debug binary at the real vault. This is what
            // keeps `pnpm tauri dev` from ever touching the real vault that
            // /Applications/DohFlow.app holds. The profile check itself lives
            // in `data_dir::resolve_app_data_dir`, not inline here, precisely
            // so it has a test that can assert on it directly (personal-cfo-
            // qrh3t review round 2) — this call site has nothing of its own
            // left to get backwards.
            let data_dir = data_dir::resolve_app_data_dir(
                update::BUILD_CHANNEL,
                app.path().app_data_dir()?,
                std::env::var("PCFO_DATA_DIR")
                    .ok()
                    .map(std::path::PathBuf::from),
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
            app.manage(
                AppState::with_registry(controller, registry, data_dir)
                    .with_touch_id(touch_id::TouchId::production(&app.config().identifier)),
            );
            // Retire a Touch ID item whose markers are unreadable or no longer match the vault's
            // envelope (a change made while the app was not running). No prompt, no unlock.
            let sweep_handle = app.handle().clone();
            tauri::async_runtime::spawn_blocking(move || {
                touch_id::app::sweep(&sweep_handle.state::<AppState>());
            });

            // The trusted main window is `"create": false` in tauri.conf.json and built
            // here (personal-cfo-2no, ADR 0010 addendum 2026-09-27) so it gets the same
            // new-window deny as the isolated shells — and only once `AppState` is
            // managed, so its first IPC call can never outrun the state it reads.
            windows::build_main(app)?;
            // Debug builds only: PCFO_SMOKE_OPEN_SHELLS=1 opens the isolated shells for
            // a hands-on isolation check. Not compiled into release builds.
            #[cfg(debug_assertions)]
            windows::open_smoke_shells_if_requested(app)?;
            // Debug builds only: PCFO_ISOLATION_PROBE=<report> probes every window's real
            // WebView (IPC rejection, effective CSP, navigation, window.open) and exits.
            #[cfg(debug_assertions)]
            isolation_probe::start_if_requested(app.handle())?;

            // The host's ready hook (personal-cfo-i93n3.13): once, with the app's handle, after
            // the app's own setup has finished. None for the desktop.
            call_host_hook(options.on_ready, app.handle());

            // CI launch smoke (personal-cfo-rr0lm, DIST-5): a headless proof that
            // the binary starts and setup() completes without a display or a
            // vault, distinct from D9b (release signing) — this reads no
            // secrets and changes nothing else about a normal launch. Opt-in
            // only: an unset/mismatched env var is a no-op in every real launch.
            if std::env::var("PCFO_SMOKE_TEST_EXIT").as_deref() == Ok("1") {
                println!("DohFlow smoke test: setup() completed successfully, exiting 0.");
                std::process::exit(0);
            }

            Ok(())
        })
        .run(context)
        .expect("error while running tauri application");
}

/// Whether `context`'s ACL confines the app's own commands to the `main` window.
///
/// Tauri builds the ACL manifest from the crate that calls `generate_context!`.
/// Without the app's permission files in it, the app's commands would be
/// unrestricted for local content in every window, including the untrusted ones.
/// A capability can only grant an app permission that the manifest defines, so a
/// grant of `vault_status` to `main` proves the manifest carries them. The other
/// half is the isolation rule of ADR 0010: neither an app command nor a destructive
/// one may be granted to any isolated window (a capability for `"*"` counts, since
/// it matches those labels too). [`run_with`] refuses a context for which this is
/// false.
#[must_use]
pub fn context_confines_app_commands_to_main<R: tauri::Runtime>(
    context: &mut tauri::Context<R>,
) -> bool {
    /// One command from each permission file: the general set, the destructive set and
    /// the desktop self-update set (a capability that grants that set is inactive off
    /// desktop, so `apply_update` then resolves to nothing, which is what is required).
    const APP_COMMANDS: [&str; 3] = ["vault_status", "delete_vault", "apply_update"];
    let authority = context.runtime_authority_mut();
    let granted = |label: &str, command: &str| {
        authority
            .resolve_access(command, label, label, &tauri::ipc::Origin::Local)
            .is_some()
    };
    granted(windows::MAIN, "vault_status")
        && windows::IsolatedSurface::ALL.iter().all(|surface| {
            APP_COMMANDS
                .iter()
                .all(|command| !granted(surface.label(), command))
        })
}
