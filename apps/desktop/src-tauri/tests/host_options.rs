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
use app_lib::{app_commands_are_granted_to_main, register_plugins, HostOptions};
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
            process: true
        }
    );
}

#[test]
fn with_the_updater_omitted_its_commands_do_not_exist() {
    let without = app(HostOptions {
        updater: false,
        process: true,
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
        !is_unregistered(&result, "updater"),
        "download_and_install must reach the updater plugin: {result:?}"
    );
}

#[test]
fn with_the_process_plugin_omitted_restart_does_not_exist() {
    let without = app(HostOptions {
        updater: true,
        process: false,
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
fn a_context_that_grants_the_app_commands_to_main_is_accepted() {
    let mut context = tauri::generate_context!(test = true);
    assert!(app_commands_are_granted_to_main::<MockRuntime>(
        &mut context
    ));
}

#[test]
fn a_context_without_the_app_acl_is_refused() {
    // A context built by a crate that did not include this app's permission files
    // grants nothing to main, so `run_with` would refuse to start with it.
    let mut context = mock_context(noop_assets());
    assert!(!app_commands_are_granted_to_main::<MockRuntime>(
        &mut context
    ));
}
