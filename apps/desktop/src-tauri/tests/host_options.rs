//! The generic embedding point (personal-cfo-i93n3.8): `run_with` takes the
//! caller's own Tauri context and a [`HostOptions`] choosing the optional plugins.
//!
//! The registration code is generic over the runtime, so these tests drive the very
//! function `run_with` uses on Tauri's mock runtime, with the ACL resolved from the
//! COMMITTED capability files (`generate_context!`), and send invoke requests through
//! `Webview::on_message` like a real WebView's IPC would. An unregistered plugin's
//! command is answered with "Command <name> not found" AFTER the ACL has let it
//! through, so a grant in a capability file cannot make an omitted plugin's commands
//! exist.

use app_lib::windows::{self, MAIN};
use app_lib::{context_confines_app_commands_to_main, register_plugins, HostOptions};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{
    get_ipc_response, mock_builder, mock_context, noop_assets, MockRuntime, INVOKE_KEY,
};
use tauri::webview::InvokeRequest;
use tauri::{App, Manager};

// A stand-in under a real command name, so the ACL-approved path has a handler.
#[tauri::command]
fn vault_status() -> &'static str {
    "dispatched"
}

fn app(options: HostOptions) -> App<MockRuntime> {
    let app = register_plugins(mock_builder(), options)
        .invoke_handler(tauri::generate_handler![vault_status])
        .build(tauri::generate_context!(test = true))
        .expect("mock app builds with the real context");
    windows::build_main(&app).expect("main builds from its tauri.conf.json entry");
    app
}

fn invoke(app: &App<MockRuntime>, cmd: &str) -> Result<String, String> {
    let webview = app.get_webview_window(MAIN).expect("main exists");
    get_ipc_response(
        &webview,
        InvokeRequest {
            cmd: cmd.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "tauri://localhost".parse().expect("url"),
            body: InvokeBody::default(),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        },
    )
    .map(|body| format!("{body:?}"))
    .map_err(|err| err.to_string())
}

/// Tauri answers a command of a plugin that was never registered with
/// "plugin <name> not found", after the ACL has let the request through.
fn is_unregistered(result: &Result<String, String>, plugin: &str) -> bool {
    matches!(result, Err(e) if e.contains(&format!("plugin {plugin} not found")))
}

#[test]
fn the_default_options_register_every_optional_plugin() {
    // `run()` is `run_with(generate_context!(), HostOptions::default())`, so the
    // default must keep the desktop app's plugin set exactly.
    assert_eq!(
        HostOptions::default(),
        HostOptions {
            updater: true,
            process: true,
            system_backup_device: None,
        }
    );
}

#[test]
fn with_the_updater_omitted_its_commands_do_not_exist() {
    let without = app(HostOptions {
        updater: false,
        process: true,
        ..HostOptions::default()
    });
    // The real context grants `updater:default` to main, so the ACL lets these
    // through and the answer comes from dispatch: no such command.
    for cmd in [
        "plugin:updater|check",
        "plugin:updater|download",
        "plugin:updater|install",
        "plugin:updater|download_and_install",
    ] {
        let result = invoke(&without, cmd);
        assert!(
            is_unregistered(&result, "updater"),
            "{cmd} must not exist when the updater is omitted: {result:?}"
        );
    }
}

#[test]
fn with_the_updater_registered_its_commands_exist() {
    // The control for the test above: the same request reaches the plugin when it
    // is registered (and fails there, on a missing argument, before any network).
    let with = app(HostOptions::default());
    let result = invoke(&with, "plugin:updater|download_and_install");
    assert!(
        matches!(&result, Err(e) if e.contains("missing required key rid")),
        "download_and_install must reach the updater plugin and fail on its missing \
         argument: {result:?}"
    );
}

#[test]
fn with_the_process_plugin_omitted_restart_does_not_exist() {
    let without = app(HostOptions {
        updater: true,
        process: false,
        ..HostOptions::default()
    });
    // `restart` is granted to main in the destructive capability; it is only ever
    // invoked here on an app WITHOUT the plugin, where it must not dispatch.
    let result = invoke(&without, "plugin:process|restart");
    assert!(
        is_unregistered(&result, "process"),
        "restart must not exist when the process plugin is omitted: {result:?}"
    );
}

#[test]
fn omitting_the_optional_plugins_does_not_touch_the_always_on_ones() {
    let bare = app(HostOptions {
        updater: false,
        process: false,
        ..HostOptions::default()
    });
    // The app's own command still dispatches, so only the optional plugins left.
    let ok = invoke(&bare, "vault_status").expect("an app command still dispatches");
    assert!(ok.contains("dispatched"), "{ok}");
    // The dialog plugin's commands are still registered: the answer is not "not found".
    let dialog = invoke(&bare, "plugin:dialog|save");
    assert!(
        !is_unregistered(&dialog, "dialog"),
        "dialog stays registered: {dialog:?}"
    );
}

#[test]
fn a_context_that_confines_the_app_commands_to_main_is_accepted() {
    let mut context = tauri::generate_context!(test = true);
    assert!(context_confines_app_commands_to_main::<MockRuntime>(
        &mut context
    ));
}

#[test]
fn a_context_without_the_app_acl_is_refused() {
    // A context built by a crate that did not include this app's permission files
    // grants nothing to main, so `run_with` would refuse to start with it.
    let mut context = mock_context(noop_assets());
    assert!(!context_confines_app_commands_to_main::<MockRuntime>(
        &mut context
    ));
}

/// The real context with one extra capability added, as an embedding crate's own
/// capability file would. (`add_capability` is forbidden in `src/` by
/// `acl_coverage.rs`; a test building a hostile context is the one place it belongs.)
fn context_with_extra_capability(capability: &str) -> tauri::Context<MockRuntime> {
    let mut context = tauri::generate_context!(test = true);
    context
        .runtime_authority_mut()
        .add_capability(capability)
        .expect("the capability is valid");
    context
}

#[test]
fn a_context_granting_an_app_command_to_an_isolated_window_is_refused() {
    // The isolation rule of ADR 0010: an untrusted window gets no app command, in
    // any of the three permission files, and a wildcard window pattern counts too.
    for (windows, permission) in [
        ("\"document_preview\"", "allow-app-commands"),
        ("\"agent_report\"", "allow-app-commands"),
        ("\"document_preview\"", "allow-destructive-commands"),
        ("\"agent_report\"", "allow-destructive-commands"),
        ("\"*\"", "allow-app-commands"),
        ("\"*\"", "allow-destructive-commands"),
        ("\"document_preview\"", "allow-desktop-update-commands"),
        ("\"agent_report\"", "allow-desktop-update-commands"),
        ("\"*\"", "allow-desktop-update-commands"),
    ] {
        let capability = format!(
            r#"{{"identifier":"hostile","windows":[{windows}],"permissions":["{permission}"]}}"#
        );
        let mut context = context_with_extra_capability(&capability);
        assert!(
            !context_confines_app_commands_to_main::<MockRuntime>(&mut context),
            "a capability granting {permission} to {windows} must be refused"
        );
    }
}

#[test]
fn a_harmless_extra_capability_does_not_trip_the_guard() {
    // The control for the test above: the same helper with a grant that does not
    // touch the app commands (a core permission for an isolated window) is accepted,
    // so the refusals above come from the app-command grants and not from the helper.
    let mut context = context_with_extra_capability(
        r#"{"identifier":"harmless","windows":["document_preview"],"permissions":["core:event:default"]}"#,
    );
    assert!(context_confines_app_commands_to_main::<MockRuntime>(
        &mut context
    ));
}

// ---------------------------------------------------------------------------
// The host's self-update flag on build info (personal-cfo-i93n3.12).
// ---------------------------------------------------------------------------

fn build_info_app(options: HostOptions) -> App<MockRuntime> {
    let app = register_plugins(mock_builder(), options)
        .manage(options)
        .invoke_handler(tauri::generate_handler![app_lib::ipc::commands::build_info])
        .build(tauri::generate_context!(test = true))
        .expect("mock app builds with the real context");
    windows::build_main(&app).expect("main builds from its tauri.conf.json entry");
    app
}

fn self_update_flag(app: &App<MockRuntime>) -> bool {
    let webview = app.get_webview_window(MAIN).expect("main exists");
    let body = get_ipc_response(
        &webview,
        InvokeRequest {
            cmd: "build_info".into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "tauri://localhost".parse().expect("url"),
            body: InvokeBody::default(),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        },
    )
    .expect("main may call build_info");
    let info: serde_json::Value = body.deserialize().expect("build_info answers JSON");
    info["self_update_available"]
        .as_bool()
        .expect("build_info carries a boolean self_update_available")
}

#[test]
fn the_default_host_reports_self_update_where_the_updater_exists() {
    assert_eq!(
        HostOptions::default().self_update_available(),
        cfg!(desktop)
    );
    assert_eq!(
        self_update_flag(&build_info_app(HostOptions::default())),
        cfg!(desktop)
    );
}

#[test]
fn a_host_without_the_updater_reports_no_self_update() {
    let options = HostOptions {
        updater: false,
        process: true,
        ..HostOptions::default()
    };
    assert!(!options.self_update_available());
    assert!(!self_update_flag(&build_info_app(options)));
}

#[test]
fn the_process_plugin_alone_does_not_make_self_update_available() {
    let options = HostOptions {
        updater: false,
        process: false,
        ..HostOptions::default()
    };
    assert!(!options.self_update_available());
    assert!(!self_update_flag(&build_info_app(options)));
}

// ---------------------------------------------------------------------------
// The host-supplied system-backup device name (personal-cfo-i93n3.9).
// ---------------------------------------------------------------------------

fn backup_device(app: &App<MockRuntime>) -> serde_json::Value {
    let webview = app.get_webview_window(MAIN).expect("main exists");
    let body = get_ipc_response(
        &webview,
        InvokeRequest {
            cmd: "build_info".into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "tauri://localhost".parse().expect("url"),
            body: InvokeBody::default(),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        },
    )
    .expect("main may call build_info");
    let info: serde_json::Value = body.deserialize().expect("build_info answers JSON");
    info["system_backup_device"].clone()
}

#[test]
fn the_desktop_default_reports_no_system_backup_device() {
    assert_eq!(
        backup_device(&build_info_app(HostOptions::default())),
        serde_json::Value::Null
    );
}

#[test]
fn a_host_supplied_device_name_reaches_build_info() {
    let options = HostOptions {
        system_backup_device: Some("Test Device"),
        ..HostOptions::default()
    };
    assert_eq!(
        backup_device(&build_info_app(options)),
        serde_json::json!("Test Device")
    );
}

#[test]
fn a_blank_overlong_or_control_character_name_is_dropped_not_shown() {
    for name in [
        "",
        "   ",
        "a\nb",
        "tab\there",
        "this device name is much too long to show in a sentence",
    ] {
        let options = HostOptions {
            system_backup_device: Some(name),
            ..HostOptions::default()
        };
        assert_eq!(options.system_backup_device_label(), None, "{name:?}");
        assert_eq!(
            backup_device(&build_info_app(options)),
            serde_json::Value::Null,
            "{name:?}"
        );
    }
    // Surrounding space is trimmed; a name at the limit is kept.
    let padded = HostOptions {
        system_backup_device: Some("  Test Device  "),
        ..HostOptions::default()
    };
    assert_eq!(padded.system_backup_device_label(), Some("Test Device"));
    let at_limit = HostOptions {
        system_backup_device: Some("abcdefghijklmnopqrstuvwxyzabcdef"),
        ..HostOptions::default()
    };
    assert_eq!(
        at_limit.system_backup_device_label().map(str::len),
        Some(32)
    );
}
