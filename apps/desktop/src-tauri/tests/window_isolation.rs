//! Window isolation through the real ACL (personal-cfo-2no, ADR 0010 addendum
//! 2026-09-27).
//!
//! Tauri serves one invoke handler to every webview, so an untrusted window is
//! isolated by its capability, not by a missing handler. These tests build the app
//! on Tauri's mock runtime with the ACL resolved from the COMMITTED capability and
//! permission files (`generate_context!`), then send invoke requests from each
//! window label through `Webview::on_message` — the same entry point a real
//! WebView's IPC reaches — and assert which ones the ACL lets through. The windows
//! themselves are built by the app's own `windows` module, not by the test.

use app_lib::navigation_guard;
use app_lib::windows::{self, IsolatedSurface, MAIN};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::plugin::Plugin;
use tauri::test::{get_ipc_response, mock_builder, MockRuntime, INVOKE_KEY};
use tauri::webview::InvokeRequest;
use tauri::{App, Manager};

const UNTRUSTED: [&str; 2] = ["document_preview", "agent_report"];

// Stand-ins registered under two REAL command names — one from each permission
// file — so an ACL-approved call has something to dispatch to. What is under test
// is the ACL decision, which happens before dispatch.
#[tauri::command]
fn vault_status() -> &'static str {
    "dispatched"
}

#[tauri::command]
fn delete_vault() -> &'static str {
    "dispatched"
}

fn app() -> App<MockRuntime> {
    let app = mock_builder()
        .invoke_handler(tauri::generate_handler![vault_status, delete_vault])
        .build(tauri::generate_context!(test = true))
        .expect("mock app builds with the real context");
    windows::build_main(&app).expect("main builds from its tauri.conf.json entry");
    for surface in IsolatedSurface::ALL {
        windows::open_isolated_shell(&app, surface).expect("shell opens");
    }
    app
}

fn invoke(app: &App<MockRuntime>, label: &str, cmd: &str) -> Result<String, String> {
    let webview = app
        .get_webview_window(label)
        .unwrap_or_else(|| panic!("window {label} exists"));
    get_ipc_response(
        &webview,
        InvokeRequest {
            cmd: cmd.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            // The bundled-app origin, so the request is judged as LOCAL content —
            // the strongest position an untrusted page could be in.
            url: "tauri://localhost".parse().expect("url"),
            body: InvokeBody::default(),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        },
    )
    .map(|body| format!("{:?}", body))
    .map_err(|err| err.to_string())
}

fn assert_rejected_by_acl(app: &App<MockRuntime>, label: &str, cmd: &str) {
    let err =
        invoke(app, label, cmd).expect_err(&format!("{label} must not be able to invoke {cmd}"));
    assert!(
        err.contains("not allowed"),
        "{label} → {cmd} must be rejected by the ACL, not by dispatch: {err}"
    );
}

#[test]
fn main_is_granted_the_general_and_destructive_command_surfaces() {
    let app = app();
    for cmd in ["vault_status", "delete_vault"] {
        let ok = invoke(&app, MAIN, cmd).unwrap_or_else(|e| panic!("main → {cmd}: {e}"));
        assert!(
            ok.contains("dispatched"),
            "main → {cmd} reached its handler: {ok}"
        );
    }
}

#[test]
fn untrusted_windows_cannot_invoke_app_or_destructive_commands() {
    let app = app();
    for label in UNTRUSTED {
        assert_rejected_by_acl(&app, label, "vault_status");
        assert_rejected_by_acl(&app, label, "delete_vault");
    }
}

#[test]
fn untrusted_windows_cannot_invoke_core_or_plugin_commands() {
    let app = app();
    // One representative per grant main holds (core:default, core:window, dialog,
    // opener, updater, process) — none of which an untrusted window may reach.
    let plugin_commands = [
        "plugin:event|listen",
        "plugin:window|set_theme",
        "plugin:webview|create_webview_window",
        "plugin:dialog|save",
        "plugin:opener|open_url",
        "plugin:updater|check",
        "plugin:process|restart",
    ];
    for label in UNTRUSTED {
        for cmd in plugin_commands {
            assert_rejected_by_acl(&app, label, cmd);
        }
    }
}

#[test]
fn the_shell_labels_are_exactly_the_ones_the_empty_capabilities_target() {
    let labels: Vec<&str> = IsolatedSurface::ALL.iter().map(|s| s.label()).collect();
    assert_eq!(labels, UNTRUSTED);
}

#[test]
fn untrusted_windows_cannot_navigate_to_a_remote_origin() {
    // The guard is a plugin, so Tauri runs it for every webview; drive its hook for
    // each untrusted window exactly as the navigation chain does.
    let app = app();
    let mut guard = navigation_guard::init::<MockRuntime>();
    for label in UNTRUSTED {
        let window = app.get_webview_window(label).expect("shell window");
        let webview: &tauri::Webview<MockRuntime> = window.as_ref();
        for remote in [
            "https://example.com/",
            "https://dohflow.app/",
            "file:///etc/passwd",
            "data:text/html,<h1>x</h1>",
        ] {
            assert!(
                !guard.on_navigation(webview, &remote.parse().expect("url")),
                "{label} must not navigate to {remote}"
            );
        }
        assert!(
            guard.on_navigation(
                webview,
                &"tauri://localhost/isolated-shell.html"
                    .parse()
                    .expect("url")
            ),
            "{label} may load its own bundled shell page"
        );
    }
}

#[test]
fn opening_a_shell_twice_reuses_the_one_window() {
    let app = app();
    let surface = IsolatedSurface::DocumentPreview;
    let again = windows::open_isolated_shell(&app, surface).expect("re-open focuses");
    assert_eq!(again.label(), surface.label());
    assert_eq!(
        app.webview_windows()
            .keys()
            .filter(|label| label.as_str() == surface.label())
            .count(),
        1,
        "one window per shell label"
    );
    // Close → destroy → re-open is Tauri's default window lifecycle; the mock
    // runtime only processes a destroy inside its event loop, so that half is
    // covered by the built-app smoke instead (see the PR).
}

#[test]
fn only_the_three_known_windows_exist() {
    let app = app();
    let mut labels: Vec<String> = app.webview_windows().into_keys().collect();
    labels.sort();
    assert_eq!(labels, ["agent_report", "document_preview", "main"]);
}
