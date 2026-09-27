//! ACL coverage guard (personal-cfo-3fdd.6).
//!
//! With app permissions present, Tauri v2 is deny-by-default for the ENTIRE custom
//! command surface: any command registered in `collect_commands!` but missing from
//! `permissions/*.toml` fails at runtime with "not allowed by ACL". This test keeps
//! the three lists (registration, general grants, destructive grants) in lockstep so
//! that drift is a test failure, not a broken app.

use std::collections::BTreeSet;

#[path = "support/capability_audit.rs"]
mod capability_audit;

use capability_audit::{audit, Inputs, Violation};

const LIB_RS: &str = include_str!("../src/lib.rs");
const APP_COMMANDS: &str = include_str!("../permissions/app-commands.toml");
const DESTRUCTIVE_COMMANDS: &str = include_str!("../permissions/destructive-commands.toml");

/// Command idents out of the `collect_commands![...]` block — the audit's parser,
/// so there is one command inventory, not two.
fn registered_commands() -> BTreeSet<String> {
    capability_audit::registered_commands(LIB_RS)
}

/// Quoted command names out of a permissions TOML's allow list.
fn granted(toml: &str) -> BTreeSet<String> {
    toml.lines()
        .flat_map(|line| line.split('"').skip(1).step_by(2))
        .filter(|s| s.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
        .map(str::to_owned)
        .collect()
}

#[test]
fn every_registered_command_has_exactly_one_acl_grant() {
    let registered = registered_commands();
    assert!(
        registered.len() > 100,
        "sanity: parsed the real command list, got {}",
        registered.len()
    );
    let app = granted(APP_COMMANDS);
    let destructive = granted(DESTRUCTIVE_COMMANDS);
    assert!(app.contains("backup_history"));
    assert!(!destructive.contains("backup_history"));

    let both: Vec<_> = app.intersection(&destructive).collect();
    assert!(both.is_empty(), "commands granted twice: {both:?}");

    let all: BTreeSet<_> = app.union(&destructive).cloned().collect();
    let missing: Vec<_> = registered.difference(&all).collect();
    assert!(
        missing.is_empty(),
        "registered commands MISSING from permissions/*.toml (would be denied at runtime): {missing:?}"
    );
    let stale: Vec<_> = all.difference(&registered).collect();
    assert!(
        stale.is_empty(),
        "permissions grant commands that are not registered: {stale:?}"
    );
}

#[test]
fn destructive_grants_stay_a_deliberate_short_list() {
    // The reviewed inventory is `[permission_sets.allow-destructive-commands]` in
    // expected-capabilities.toml (personal-cfo-cjd): growing it is a conscious,
    // reviewed edit there, not a count here.
    let drift = violations_for(&["permission-commands", "permission-set"]);
    assert!(
        drift.is_empty(),
        "the destructive/exfiltration set drifted:\n{}",
        render(&drift)
    );
    assert!(granted(DESTRUCTIVE_COMMANDS).contains("delete_vault"));
}

// ---------------------------------------------------------------------------
// Capability drift guard (personal-cfo-n76x.18, ADR 0010 addendum 2026-09-06).
//
// `tauri-build` validates that every capability→permission reference resolves,
// but it will happily accept `opener:default` (mailto/tel/http/https to anywhere,
// plus reveal-in-Finder) or a widened URL scope. These tests pin the grant to
// exactly what the addendum allows, so widening it is a deliberate edit here too.
// ---------------------------------------------------------------------------

const DEFAULT_CAPABILITY: &str = include_str!("../capabilities/default.json");
const DESTRUCTIVE_CAPABILITY: &str = include_str!("../capabilities/destructive.json");
const DOCUMENT_PREVIEW_CAPABILITY: &str = include_str!("../capabilities/document-preview.json");
const AGENT_REPORT_CAPABILITY: &str = include_str!("../capabilities/agent-report.json");
const TAURI_CONF: &str = include_str!("../tauri.conf.json");

/// The only URL pattern the main window may hand to the system browser: pages the
/// project controls. Shipped binaries outlive URLs, so github.com / any payment
/// processor / anything else stays out of the app forever.
const OPENER_URL_SCOPE: &str = "https://dohflow.app/*";

/// The permission entries of a capability file, each as `(identifier, full entry)`.
/// Tauri accepts either a bare identifier string or an object carrying a scope.
fn permissions(capability: &str) -> Vec<(String, serde_json::Value)> {
    let cap: serde_json::Value =
        serde_json::from_str(capability).expect("capability file is valid JSON");
    cap["permissions"]
        .as_array()
        .expect("capability declares a permissions array")
        .iter()
        .map(|entry| {
            let id = match entry {
                serde_json::Value::String(id) => id.clone(),
                serde_json::Value::Object(obj) => obj["identifier"]
                    .as_str()
                    .expect("object-form permission names an identifier")
                    .to_owned(),
                other => panic!("unexpected permission entry: {other}"),
            };
            (id, entry.clone())
        })
        .collect()
}

/// Every capability file the app ships, by name. `default` and `destructive` both
/// target the `main` window, and Tauri MERGES scopes across every capability that targets a window
/// (the opener's `open_url` checks the command scope chained with the global
/// scope), so a grant in one file silently widens a grant in the other. Any test
/// that pins "exactly one" of something must therefore look at the union.
const CAPABILITIES: [(&str, &str); 4] = [
    ("default.json", DEFAULT_CAPABILITY),
    ("destructive.json", DESTRUCTIVE_CAPABILITY),
    ("document-preview.json", DOCUMENT_PREVIEW_CAPABILITY),
    ("agent-report.json", AGENT_REPORT_CAPABILITY),
];

#[test]
fn the_main_window_opener_grant_is_exactly_open_url_scoped_to_dohflow() {
    let opener: Vec<(&str, String, serde_json::Value)> = CAPABILITIES
        .into_iter()
        .flat_map(|(file, capability)| {
            permissions(capability)
                .into_iter()
                .filter(|(id, _)| id.starts_with("opener:"))
                .map(move |(id, entry)| (file, id, entry))
        })
        .collect();
    assert_eq!(
        opener.len(),
        1,
        "exactly one opener permission across ALL capability files (ADR 0010 addendum \
         2026-09-06) — a second entry anywhere merges into and widens the grant, got {opener:?}"
    );
    let (file, id, entry) = &opener[0];
    assert_eq!(
        *file, "default.json",
        "the opener grant lives in the general capability, never the destructive one"
    );
    assert_eq!(
        id, "opener:allow-open-url",
        "only the open_url command, never opener:default"
    );

    let allow = entry["allow"].as_array().expect(
        "the opener grant carries an explicit allow scope — a scope-less grant is the drift \
         this test exists to catch",
    );
    assert_eq!(
        allow,
        &[serde_json::json!({ "url": OPENER_URL_SCOPE })],
        "the opener URL scope is exactly one entry: {OPENER_URL_SCOPE}"
    );
    assert!(
        entry.get("deny").is_none(),
        "no deny list: the allow list is the whole policy"
    );
}

#[test]
fn no_capability_grants_opener_default_shell_fs_or_http() {
    // The opener plugin's other permissions: `default` bundles allow-default-urls
    // (mailto/tel/http/https to ANY host) + reveal-item-in-dir; the rest open local
    // paths or widen the URL scope. None of them are ever granted.
    let forbidden_opener = [
        "opener:default",
        "opener:allow-default-urls",
        "opener:allow-open-path",
        "opener:allow-reveal-item-in-dir",
    ];
    for (file, capability) in CAPABILITIES {
        for (id, _) in permissions(capability) {
            assert!(
                !forbidden_opener.contains(&id.as_str()),
                "{file} grants {id}: the About card needs open_url to dohflow.app only"
            );
            // The dangerous capability plugins stay opt-in and un-opted (ADR 0010).
            for prefix in ["shell:", "fs:", "http:"] {
                assert!(
                    !id.starts_with(prefix),
                    "{file} grants {id}: the {prefix} plugin is never granted to any window (ADR 0010)"
                );
            }
        }
    }
}

#[test]
fn the_opener_plugin_injects_no_click_interceptor() {
    // `tauri_plugin_opener::init()` is `Builder::default().build()` with
    // `open_js_links_on_click: true`, which injects an init script into EVERY window
    // that catches clicks on `target="_blank"` / modifier-clicked http(s)/mailto/tel
    // anchors and sends them straight to `open_url` — bypassing the frontend's
    // allow-listing helper (`src/lib/openExternal.ts`), and running in the future
    // isolated windows too. The app registers the builder with that switched off, so
    // the helper is the only path to the command and no script is injected anywhere.
    assert!(
        !LIB_RS.contains("tauri_plugin_opener::init()"),
        "lib.rs registers the opener through init(), which turns the anchor-click \
         interceptor on — use Builder::new().open_js_links_on_click(false).build()"
    );
    assert!(
        LIB_RS.contains("tauri_plugin_opener::Builder::new()")
            && LIB_RS.contains(".open_js_links_on_click(false)"),
        "lib.rs must register the opener via Builder::new().open_js_links_on_click(false)"
    );
}

#[test]
fn the_opener_grant_did_not_loosen_the_csp() {
    // Opening a page in the SYSTEM browser needs nothing from the WebView's CSP; the
    // temptation is to add the site origin to connect-src/frame-src "while we're here".
    // The CSP stays exactly as strict as ADR 0010 specifies: no remote origins at all.
    let conf: serde_json::Value =
        serde_json::from_str(TAURI_CONF).expect("tauri.conf.json is valid JSON");
    let csp = conf["app"]["security"]["csp"]
        .as_str()
        .expect("app.security.csp is set (ADR 0010 — CI also guards this)");
    assert!(
        !csp.contains("dohflow.app"),
        "the site origin must not enter the CSP — links open in the system browser: {csp}"
    );
    for directive in [
        "default-src 'self'",
        "connect-src 'self' ipc: http://ipc.localhost",
        "frame-src 'none'",
    ] {
        assert!(csp.contains(directive), "CSP lost `{directive}`: {csp}");
    }
}

// ---------------------------------------------------------------------------
// Updater capability drift guard (personal-cfo-867.1.2, ADR 0068).
//
// The updater plugin does its own network fetch (a `reqwest` client the plugin
// manages internally, confirmed by its transitive deps) against the single
// endpoint in tauri.conf.json's plugins.updater — never the generic `http:`
// capability plugin, which `no_capability_grants_opener_default_shell_fs_or_http`
// above already keeps ungranted. These tests pin the grant itself: exactly one
// `updater:*` permission, in `default.json` only, and exactly one `process:*`
// permission, in `destructive.json` only — the same "exactly this, nowhere
// else" shape as the opener tests above.
// ---------------------------------------------------------------------------

#[test]
fn the_updater_grant_is_exactly_updater_default_in_the_general_capability() {
    let updater: Vec<(&str, String)> = CAPABILITIES
        .into_iter()
        .flat_map(|(file, capability)| {
            permissions(capability)
                .into_iter()
                .filter(|(id, _)| id.starts_with("updater:"))
                .map(move |(id, _)| (file, id))
        })
        .collect();
    assert_eq!(
        updater,
        vec![("default.json", "updater:default".to_owned())],
        "exactly one updater:* permission, in default.json only — the update check itself \
         is not destructive, unlike the install/relaunch step below"
    );
}

#[test]
fn the_window_theme_grant_is_exactly_allow_set_theme_in_the_general_capability() {
    // personal-cfo-17u1: the dark-mode toggle syncs the native title bar to an explicit
    // Light/Dark override via `getCurrentWindow().setTheme(...)`. `core:default` already
    // grants the READ half (`core:window:allow-theme` is in its default set) but not the
    // write — this pins that exactly one write grant exists, and only in the general
    // (non-destructive) capability, since reading/writing the window's OWN appearance is
    // not an exfiltration/destructive concern the way delete_vault or apply_update are.
    let window_theme: Vec<(&str, String)> = CAPABILITIES
        .into_iter()
        .flat_map(|(file, capability)| {
            permissions(capability)
                .into_iter()
                .filter(|(id, _)| id.starts_with("core:window:"))
                .map(move |(id, _)| (file, id))
        })
        .collect();
    assert_eq!(
        window_theme,
        vec![("default.json", "core:window:allow-set-theme".to_owned())],
        "exactly one core:window:* permission, in default.json only — a widened set here \
         (e.g. core:window:allow-set-size, allow-close) would grant far more than the theme \
         sync this bead needs"
    );
}

#[test]
fn the_process_grant_is_exactly_allow_restart_in_the_destructive_capability() {
    let process: Vec<(&str, String)> = CAPABILITIES
        .into_iter()
        .flat_map(|(file, capability)| {
            permissions(capability)
                .into_iter()
                .filter(|(id, _)| id.starts_with("process:"))
                .map(move |(id, _)| (file, id))
        })
        .collect();
    assert_eq!(
        process,
        vec![("destructive.json", "process:allow-restart".to_owned())],
        "exactly one process:* permission, in destructive.json only — restarting the app \
         (after the updater has already swapped the binary) belongs beside apply_update/ \
         relaunch_app, never the default capability future isolated windows might get"
    );
    // process:default additionally grants exit/restart-with-args/plain restart variants this
    // app has no use for — the narrow allow-restart is deliberate, not an oversight.
    for (file, capability) in CAPABILITIES {
        for (id, _) in permissions(capability) {
            assert_ne!(
                id, "process:default",
                "{file} grants process:default — narrow this to process:allow-restart only"
            );
        }
    }
}

#[test]
fn the_updater_pubkey_and_endpoint_are_exactly_the_committed_values() {
    let conf: serde_json::Value =
        serde_json::from_str(TAURI_CONF).expect("tauri.conf.json is valid JSON");
    let updater = &conf["plugins"]["updater"];
    assert_eq!(
        updater["pubkey"].as_str(),
        Some(
            "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEUyMUIyRTUyODg4N0Y3Q0UK\
             UldUTzk0ZUlVaTRiNHE5NkpGc2RKMlZXY0NtWGhucjhrbFlodEU5Z2N6T2pGUi9oVzgwUXNaNVYK"
        ),
        "the committed updater public key changed — this must only ever happen as a \
         deliberate, dated key-rotation addendum to ADR 0068, never a routine edit"
    );
    assert_eq!(
        updater["endpoints"],
        serde_json::json!([
            "https://github.com/dohflow/dohflow/releases/latest/download/latest.json"
        ]),
        "exactly one endpoint, the real repo's latest.json (ADR 0068 point 2) — a second \
         endpoint or a different repo/path needs the same deliberate-edit treatment as the key"
    );
    assert_eq!(
        conf["bundle"]["createUpdaterArtifacts"],
        serde_json::json!(true),
        "createUpdaterArtifacts must stay true — release.sh depends on it to emit the \
         signed .app.tar.gz + .sig pair"
    );
}

// ---------------------------------------------------------------------------
// Strict CSP + webview posture (personal-cfo-2rf, ADR 0010 and its addenda).
//
// The CI step only asserts the CSP is non-null, and the opener test above pins
// three directives. These pin the WHOLE policy, plus the webview settings the ADR
// relies on but nothing previously asserted: no exposed Tauri global, no release
// devtools, no remote origin granted IPC, no remote window URL, the navigation
// guard, and the general/destructive capability split.
// ---------------------------------------------------------------------------

const CARGO_TOML: &str = include_str!("../Cargo.toml");

fn security() -> serde_json::Value {
    let conf: serde_json::Value =
        serde_json::from_str(TAURI_CONF).expect("tauri.conf.json is valid JSON");
    conf["app"]["security"].clone()
}

#[test]
fn the_production_csp_is_exactly_the_adr_0010_policy() {
    // Exact directives: [production.csp] in expected-capabilities.toml. The rules the
    // baseline itself may not break (script-src exactly 'self'; 'unsafe-inline' only
    // in style-src) are invariants in the audit, checked against both.
    let drift = violations_for(&["csp", "csp-invariant"]);
    assert!(
        drift.is_empty(),
        "production CSP drifted:\n{}",
        render(&drift)
    );
}

#[test]
fn the_dev_csp_relaxes_only_script_and_connect_for_the_local_dev_server() {
    let drift = violations_for(&["dev-csp", "dev-csp-invariant", "dev-leak"]);
    assert!(
        drift.is_empty(),
        "development CSP drifted:\n{}",
        render(&drift)
    );
}

#[test]
fn the_webview_security_block_holds_only_the_two_policies() {
    // Every other `app.security` key loosens something: `dangerousDisableAssetCspModification`
    // stops Tauri hashing the bundle's scripts (pushing toward 'unsafe-inline'),
    // `assetProtocol` exposes local files to the WebView, `capabilities` inlines grants
    // outside the reviewed files, and `headers` can override the CSP.
    let keys: Vec<String> = security()
        .as_object()
        .expect("app.security is an object")
        .keys()
        .cloned()
        .collect();
    assert_eq!(
        keys,
        vec!["csp".to_owned(), "devCsp".to_owned()],
        "app.security gained a key — review it against ADR 0010 and pin it here"
    );
}

#[test]
fn the_tauri_global_is_not_exposed_to_the_frontend() {
    let conf: serde_json::Value = serde_json::from_str(TAURI_CONF).expect("valid JSON");
    assert!(
        matches!(
            conf["app"].get("withGlobalTauri"),
            None | Some(serde_json::Value::Bool(false))
        ),
        "app.withGlobalTauri must stay off — the frontend reaches Rust only through the \
         generated bindings (ADR 0010)"
    );
}

#[test]
fn release_builds_cannot_enable_webview_devtools() {
    // Tauri enables devtools in debug builds only, unless the `devtools` cargo feature
    // is on — which turns them on in release too. Read through the TOML, so the table
    // form (`[dependencies.tauri]`) and a crate `[features]` entry naming
    // `tauri/devtools` are caught as well as the inline form (2rf review F2).
    let features = capability_audit::tauri_release_features(CARGO_TOML).expect("Cargo.toml parses");
    assert!(
        !features.contains("devtools"),
        "the tauri `devtools` feature enables the inspector in release builds: {features:?}"
    );
    let conf: serde_json::Value = serde_json::from_str(TAURI_CONF).expect("valid JSON");
    for window in conf["app"]["windows"].as_array().expect("app.windows") {
        assert_ne!(
            window.get("devtools"),
            Some(&serde_json::Value::Bool(true)),
            "a window opts into devtools: {window}"
        );
    }
}

#[test]
fn windows_load_only_the_bundled_app_and_no_capability_admits_a_remote_origin() {
    let conf: serde_json::Value = serde_json::from_str(TAURI_CONF).expect("valid JSON");
    for window in conf["app"]["windows"].as_array().expect("app.windows") {
        if let Some(url) = window.get("url") {
            let url = url.as_str().expect("window url is a string");
            assert!(
                !url.contains(':'),
                "a window loads a non-app URL ({url}) — windows render the bundled app only"
            );
        }
    }
    for (file, capability) in CAPABILITIES {
        let cap: serde_json::Value = serde_json::from_str(capability).expect("valid JSON");
        assert!(
            cap.get("remote").is_none(),
            "{file} grants IPC to remote origins — no remote page may reach the command surface"
        );
    }
}

#[test]
fn the_capability_directory_holds_exactly_the_reviewed_files() {
    // A new capability file is picked up by tauri-build automatically, so without this
    // a grant could land that none of the tests above ever look at.
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities");
    let mut files: Vec<String> = std::fs::read_dir(&dir)
        .expect("capabilities/ is readable")
        .map(|entry| {
            entry
                .expect("dir entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    files.sort();
    let mut reviewed: Vec<String> = CAPABILITIES
        .iter()
        .map(|(file, _)| (*file).to_owned())
        .collect();
    reviewed.sort();
    assert_eq!(
        files, reviewed,
        "capabilities/ changed — add the new file to CAPABILITIES so the drift tests cover it"
    );
}

#[test]
fn the_general_and_destructive_command_grants_stay_in_separate_capabilities() {
    // ADR 0010 addendum 2026-07-04: the split is the seam that lets a future isolated
    // window get the plain surface, or nothing, but never the destructive set.
    let ids = |capability| -> Vec<String> {
        permissions(capability)
            .into_iter()
            .map(|(id, _)| id)
            .collect()
    };
    let general = ids(DEFAULT_CAPABILITY);
    let destructive = ids(DESTRUCTIVE_CAPABILITY);
    assert!(general.contains(&"allow-app-commands".to_owned()));
    assert!(!general.contains(&"allow-destructive-commands".to_owned()));
    assert!(destructive.contains(&"allow-destructive-commands".to_owned()));
    assert!(!destructive.contains(&"allow-app-commands".to_owned()));
    // The main window keeps Tauri's curated baseline (ADR 0010) — in the general file.
    assert!(general.contains(&"core:default".to_owned()));
    assert!(!destructive.iter().any(|id| id.starts_with("core:")));
}

#[test]
fn the_navigation_guard_is_registered_for_every_webview() {
    // CSP cannot stop a top-level navigation; `src/navigation_guard.rs` does, and it
    // only works if the plugin is registered on the app builder.
    assert!(
        LIB_RS.contains(".plugin(navigation_guard::init())"),
        "lib.rs must register the navigation guard plugin (personal-cfo-2rf, ADR 0010)"
    );
}

// ---------------------------------------------------------------------------
// Window topology (personal-cfo-2no, ADR 0010 addendum 2026-09-27).
//
// Tauri serves one invoke handler to every webview, so an untrusted window is
// isolated by an EMPTY capability that targets only its own label. These pin the
// capability files to that shape; `tests/window_isolation.rs` proves the resulting
// ACL decisions on the mock runtime.
// ---------------------------------------------------------------------------

const WINDOWS_RS: &str = include_str!("../src/windows.rs");

fn targets(capability: &str) -> Vec<String> {
    let cap: serde_json::Value = serde_json::from_str(capability).expect("valid JSON");
    cap["windows"]
        .as_array()
        .expect("capability declares windows")
        .iter()
        .map(|w| w.as_str().expect("window label").to_owned())
        .collect()
}

#[test]
fn each_capability_targets_exactly_its_own_window() {
    for (file, capability, label) in [
        ("default.json", DEFAULT_CAPABILITY, "main"),
        ("destructive.json", DESTRUCTIVE_CAPABILITY, "main"),
        (
            "document-preview.json",
            DOCUMENT_PREVIEW_CAPABILITY,
            "document_preview",
        ),
        ("agent-report.json", AGENT_REPORT_CAPABILITY, "agent_report"),
    ] {
        assert_eq!(
            targets(capability),
            vec![label.to_owned()],
            "{file} must target only `{label}` — never a wildcard or a second window"
        );
        let cap: serde_json::Value = serde_json::from_str(capability).expect("valid JSON");
        assert!(
            cap.get("webviews").is_none() && cap.get("platforms").is_none(),
            "{file} scopes by window label only"
        );
    }
}

#[test]
fn the_untrusted_shells_are_granted_nothing() {
    for (file, capability) in [
        ("document-preview.json", DOCUMENT_PREVIEW_CAPABILITY),
        ("agent-report.json", AGENT_REPORT_CAPABILITY),
    ] {
        assert!(
            permissions(capability).is_empty(),
            "{file} must grant nothing — any grant to an untrusted window needs its own \
             dated ADR 0010 addendum first"
        );
    }
}

#[test]
fn main_is_the_only_configured_window_and_is_built_in_rust() {
    let conf: serde_json::Value = serde_json::from_str(TAURI_CONF).expect("valid JSON");
    let windows = conf["app"]["windows"].as_array().expect("app.windows");
    assert_eq!(
        windows.len(),
        1,
        "the shells are created on demand, never from config"
    );
    assert_eq!(windows[0]["label"], "main");
    assert_eq!(
        windows[0]["create"],
        serde_json::Value::Bool(false),
        "main is built by windows::build_main so it gets the new-window deny"
    );
    assert!(
        LIB_RS.contains("windows::build_main(app)?"),
        "setup() must build the main window"
    );
}

#[test]
fn every_window_builder_denies_new_windows() {
    // A builder without the hook lets `window.open` create a popup on Windows.
    let builders = WINDOWS_RS.matches("WebviewWindowBuilder::").count();
    assert_eq!(
        builders, 2,
        "windows.rs builds main (from_config) and the shells (new)"
    );
    assert_eq!(
        WINDOWS_RS
            .matches(".on_new_window(deny_new_window)")
            .count(),
        builders,
        "every WebviewWindowBuilder in windows.rs installs deny_new_window"
    );
    assert!(WINDOWS_RS.contains("NewWindowResponse::Deny"));
    // Nothing else in the crate builds windows or webviews behind the module's back.
    for (path, source) in rust_sources() {
        if path.ends_with("windows.rs") {
            continue;
        }
        for builder in ["WebviewWindowBuilder", "WebviewBuilder", "WindowBuilder"] {
            assert!(
                !source.contains(builder),
                "{path} uses {builder} — every window is built in src/windows.rs"
            );
        }
    }
}

#[test]
fn the_shell_smoke_fixture_is_compiled_out_of_release_builds() {
    // AC: debug facilities stay development-only. Both the definition and its one
    // call site sit behind `#[cfg(debug_assertions)]`, so a release binary has no
    // code path that opens a shell from an environment variable.
    assert!(
        WINDOWS_RS.contains("#[cfg(debug_assertions)]\npub fn open_smoke_shells_if_requested"),
        "the smoke fixture must be defined under #[cfg(debug_assertions)]"
    );
    assert!(
        LIB_RS.contains(
            "#[cfg(debug_assertions)]\n            windows::open_smoke_shells_if_requested(app)?;"
        ),
        "the smoke fixture must be called under #[cfg(debug_assertions)]"
    );
    assert_eq!(
        LIB_RS.matches("open_smoke_shells_if_requested").count(),
        1,
        "exactly one call site"
    );
}

#[test]
fn app_code_sends_nothing_through_an_ipc_channel() {
    // ADR 0010 addendum 2026-09-27 rule 7: Tauri's channel-fetch command skips the
    // ACL and parks large Channel payloads in an app-wide queue under sequential
    // ids, so an untrusted window could take one. Keep vault data off that path.
    //
    // A whole-word scan for the type name, not for the `ipc::Channel` path: an
    // alias (`use tauri::ipc::Channel as C`), a grouped import
    // (`use tauri::ipc::{Channel, …}`) or a glob plus a bare `Channel<…>` all still
    // name `Channel` (0hp6, hardening the 2no review's advisory). No app source
    // uses the word for anything else.
    for (path, source) in rust_sources() {
        let named = source
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .any(|word| word == "Channel");
        assert!(
            !named,
            "{path} names `Channel` — tauri::ipc::Channel is off-limits (ADR 0010 addendum \
             2026-09-27 rule 7); rename an unrelated type rather than weaken this scan"
        );
    }
}

#[test]
fn app_code_never_grants_capabilities_at_runtime() {
    // Tauri's default `dynamic-acl` feature enables `Manager::add_capability`, which
    // would grant permissions outside the reviewed capability files and the drift
    // audit (8ea audit §7.3, 0hp6). Every grant is static and reviewed.
    for (path, source) in rust_sources() {
        assert!(
            !source.contains("add_capability"),
            "{path} calls add_capability — grants live in capabilities/*.json, reviewed \
             against expected-capabilities.toml, never at run time"
        );
    }
}

#[test]
fn the_isolation_probe_is_compiled_out_of_release_builds() {
    // The runtime probe (src/isolation_probe.rs) evaluates script in every window;
    // it must not exist in a release binary. Both the module and its one call site
    // sit behind `#[cfg(debug_assertions)]`.
    assert!(
        LIB_RS.contains("#[cfg(debug_assertions)]\nmod isolation_probe;"),
        "the isolation probe module must be declared under #[cfg(debug_assertions)]"
    );
    assert!(
        LIB_RS.contains(
            "#[cfg(debug_assertions)]\n            isolation_probe::start_if_requested(app.handle())?;"
        ),
        "the isolation probe must be started under #[cfg(debug_assertions)]"
    );
    assert_eq!(
        LIB_RS.matches("isolation_probe::").count(),
        1,
        "exactly one call site"
    );
}

#[test]
fn ci_runs_the_runtime_probe_and_the_built_html_check_on_pull_requests() {
    // The static suite cannot prove runtime isolation; these two CI steps do
    // (0hp6). Pin that they exist, in the PR-time jobs, with the switches that make
    // them meaningful.
    let ci = std::fs::read_to_string(manifest_dir().join("../../../.github/workflows/ci.yml"))
        .expect("ci.yml is readable");
    // The lines of one top-level job: from `  <name>:` up to the next line that
    // starts another job (two-space indent, then a key — not a comment).
    let job = |name: &str| -> String {
        let header = format!("  {name}:");
        let mut lines = ci.lines().skip_while(|line| *line != header);
        let first = lines
            .next()
            .unwrap_or_else(|| panic!("ci.yml has no job `{name}`"));
        let body = lines.take_while(|line| {
            let starts_next_job = line.starts_with("  ")
                && !line.starts_with("   ")
                && !line.trim_start().starts_with('#')
                && line.trim_end().ends_with(':');
            !starts_next_job
        });
        std::iter::once(first)
            .chain(body)
            .collect::<Vec<_>>()
            .join("\n")
    };
    let codegen = job("ipc-codegen");
    for needle in [
        "- 'apps/desktop/src-tauri/**'",
        "- 'apps/desktop/public/isolated-shell.html'",
        "- name: Runtime isolation probe (real WebView, ADR 0010)",
        "cargo build --features tauri/custom-protocol",
        "PCFO_ISOLATION_PROBE=",
        "PCFO_DATA_DIR=\"$(mktemp -d)\"",
        "xvfb-run",
    ] {
        assert!(codegen.contains(needle), "ipc-codegen job lost `{needle}`");
    }
    let frontend = job("frontend");
    // The slicer stops at the job boundary: the codegen job's probe is not "found"
    // in the frontend job, and vice versa.
    assert!(!frontend.contains("Runtime isolation probe") && !codegen.contains("check-dist-csp"));
    for needle in [
        "- 'scripts/check-dist-csp.mjs'",
        "run: pnpm -r build",
        "run: node scripts/check-dist-csp.mjs apps/desktop/dist",
    ] {
        assert!(frontend.contains(needle), "frontend job lost `{needle}`");
    }
}

/// Every `.rs` file under `src/`, as `(path, contents)`.
fn rust_sources() -> Vec<(String, String)> {
    fn walk(dir: &std::path::Path, out: &mut Vec<(String, String)>) {
        for entry in std::fs::read_dir(dir).expect("readable dir") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let text = std::fs::read_to_string(&path).expect("readable source");
                out.push((path.display().to_string(), text));
            }
        }
    }
    let mut out = Vec::new();
    walk(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut out,
    );
    assert!(out.len() > 5, "sanity: walked the real source tree");
    out
}

// ---------------------------------------------------------------------------
// Capability drift audit (personal-cfo-cjd, ADR 0010 and all its addenda).
//
// `tests/support/capability_audit.rs` works out what every window is actually
// granted and compares it with the hand-reviewed `expected-capabilities.toml`.
// The capability and permission directories are read at run time, so a NEW file
// is audited without anyone remembering to list it. The negative tests below
// break one input at a time, in memory, and prove the audit names the failure.
// ---------------------------------------------------------------------------

fn manifest_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// `(file name, contents)` for every file in `dir` with extension `ext`, sorted.
fn read_dir_files(dir: &str, ext: &str) -> Vec<(String, String)> {
    let mut files: Vec<(String, String)> = std::fs::read_dir(manifest_dir().join(dir))
        .unwrap_or_else(|e| panic!("{dir}/ is readable: {e}"))
        .map(|entry| entry.expect("dir entry").path())
        .filter(|path| path.extension().is_some_and(|e| e == ext))
        .map(|path| {
            (
                path.file_name()
                    .expect("file name")
                    .to_string_lossy()
                    .into_owned(),
                std::fs::read_to_string(&path).expect("readable file"),
            )
        })
        .collect();
    files.sort();
    files
}

fn real_inputs() -> Inputs {
    let read = |file: &str| {
        std::fs::read_to_string(manifest_dir().join(file)).unwrap_or_else(|e| panic!("{file}: {e}"))
    };
    Inputs {
        tauri_conf: read("tauri.conf.json"),
        capabilities: read_dir_files("capabilities", "json"),
        permissions: read_dir_files("permissions", "toml"),
        lib_rs: read("src/lib.rs"),
        cargo_toml: read("Cargo.toml"),
        baseline: read("expected-capabilities.toml"),
    }
}

fn render(violations: &[Violation]) -> String {
    violations.iter().map(|v| format!("  {v}\n")).collect()
}

/// The real configuration's violations of the given rules.
fn violations_for(rules: &[&str]) -> Vec<Violation> {
    audit(&real_inputs())
        .into_iter()
        .filter(|v| rules.contains(&v.rule))
        .collect()
}

#[test]
fn the_configuration_matches_the_reviewed_capability_baseline() {
    let violations = audit(&real_inputs());
    assert!(
        violations.is_empty(),
        "capability drift — fix the configuration, or (for an INTENTIONAL change) edit \
         expected-capabilities.toml by hand in this PR, per its header:\n{}",
        render(&violations)
    );
}

/// Apply `edit` to one input, run the audit, and return what it reported.
fn audit_with(edit: impl FnOnce(&mut Inputs)) -> Vec<Violation> {
    let mut inputs = real_inputs();
    edit(&mut inputs);
    audit(&inputs)
}

/// Replace exactly one occurrence of `from` in the named capability file.
fn edit_capability(inputs: &mut Inputs, file: &str, from: &str, to: &str) {
    let (_, text) = inputs
        .capabilities
        .iter_mut()
        .find(|(name, _)| name == file)
        .unwrap_or_else(|| panic!("{file} exists"));
    assert_eq!(
        text.matches(from).count(),
        1,
        "`{from}` occurs once in {file}"
    );
    *text = text.replacen(from, to, 1);
}

fn replace_once(text: &mut String, from: &str, to: &str) {
    assert_eq!(
        text.matches(from).count(),
        1,
        "`{from}` occurs exactly once"
    );
    *text = text.replacen(from, to, 1);
}

/// The audit reported `rule` against `subject` (substring match on the subject).
fn assert_reports(violations: &[Violation], rule: &str, subject: &str) {
    assert!(
        violations
            .iter()
            .any(|v| v.rule == rule && v.subject.contains(subject)),
        "expected a `{rule}` violation naming `{subject}`, got:\n{}",
        render(violations)
    );
}

#[test]
fn audit_fails_on_a_forbidden_grant_to_an_untrusted_window() {
    let found = audit_with(|i| {
        edit_capability(
            i,
            "document-preview.json",
            r#""permissions": []"#,
            r#""permissions": ["fs:default"]"#,
        )
    });
    assert_reports(&found, "window-grants", "document_preview");
    assert_reports(&found, "untrusted-grant", "document_preview");
    assert_reports(&found, "forbidden-grant", "document_preview");
}

#[test]
fn audit_fails_when_a_capability_is_broadened_to_another_window() {
    let found = audit_with(|i| {
        edit_capability(
            i,
            "destructive.json",
            "\"main\"\n  ]",
            "\"main\", \"agent_report\"\n  ]",
        )
    });
    assert_reports(&found, "window-capabilities", "agent_report");
    assert_reports(&found, "window-grants", "agent_report");
    assert_reports(&found, "destructive-reach", "agent_report");
}

#[test]
fn audit_fails_on_a_wildcard_window() {
    let found = audit_with(|i| edit_capability(i, "default.json", "\"main\"\n  ]", "\"*\"\n  ]"));
    assert_reports(&found, "wildcard-window", "default.json");
    assert_reports(&found, "window-set", "topology");
}

#[test]
fn audit_fails_on_a_new_unreviewed_window() {
    let found = audit_with(|i| {
        i.capabilities.push((
            "debug.json".into(),
            r#"{"identifier":"debug","windows":["debug"],"permissions":["core:default"]}"#.into(),
        ));
    });
    assert_reports(&found, "window-set", "topology");
}

#[test]
fn audit_fails_on_a_broadened_origin() {
    let widened = audit_with(|i| {
        edit_capability(
            i,
            "default.json",
            r#""url": "https://dohflow.app/*""#,
            r#""url": "https://*""#,
        )
    });
    assert_reports(&widened, "scope", "scoped grants");

    let remote = audit_with(|i| {
        edit_capability(
            i,
            "default.json",
            "\"windows\": [",
            "\"remote\": {\"urls\": [\"https://*.example.com\"]},\n  \"windows\": [",
        )
    });
    assert_reports(&remote, "remote-origin", "default.json");
}

#[test]
fn audit_fails_on_a_production_csp_regression() {
    let found = audit_with(|i| {
        replace_once(
            &mut i.tauri_conf,
            "\"csp\": \"default-src 'self'; script-src 'self';",
            "\"csp\": \"default-src 'self'; script-src 'self' 'unsafe-eval';",
        )
    });
    assert_reports(&found, "csp", "script-src");
    assert_reports(&found, "csp-invariant", "script-src");
}

#[test]
fn audit_fails_when_development_policy_leaks_into_production() {
    let found = audit_with(|i| {
        replace_once(
            &mut i.tauri_conf,
            "\"csp\": \"default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self' ipc: http://ipc.localhost;",
            "\"csp\": \"default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self' ipc: http://ipc.localhost ws://localhost:1420;",
        )
    });
    assert_reports(&found, "dev-leak", "connect-src");
}

#[test]
fn audit_fails_when_the_dev_policy_relaxes_more_than_hmr_needs() {
    let found = audit_with(|i| {
        replace_once(
            &mut i.tauri_conf,
            "form-action 'none'\"\n    }",
            "form-action 'self'\"\n    }",
        )
    });
    assert_reports(&found, "dev-csp", "form-action");
    assert_reports(&found, "dev-csp-invariant", "form-action");
}

#[test]
fn audit_fails_on_missing_acl_coverage() {
    let found = audit_with(|i| {
        replace_once(
            &mut i.lib_rs,
            "collect_commands![",
            "collect_commands![\n        ipc::commands::brand_new_command,",
        )
    });
    assert_reports(&found, "acl-coverage", "brand_new_command");
}

#[test]
fn audit_fails_when_a_destructive_command_moves_to_the_general_set() {
    let found = audit_with(|i| {
        let (_, destructive) = i
            .permissions
            .iter_mut()
            .find(|(name, _)| name == "destructive-commands.toml")
            .expect("destructive set");
        replace_once(destructive, "  \"delete_vault\",\n", "");
        let (_, general) = i
            .permissions
            .iter_mut()
            .find(|(name, _)| name == "app-commands.toml")
            .expect("general set");
        replace_once(
            general,
            "commands.allow = [\n",
            "commands.allow = [\n  \"delete_vault\",\n",
        );
    });
    assert_reports(&found, "permission-commands", "allow-destructive-commands");
}

#[test]
fn audit_fails_on_the_devtools_feature_in_any_form() {
    // 2rf review F2: the table form and a crate [features] entry, not only the
    // inline `tauri = { ... }` line.
    let table_form = audit_with(|i| {
        replace_once(
            &mut i.cargo_toml,
            "tauri = { version = \"2.11.2\", features = [] }\n",
            "",
        );
        i.cargo_toml
            .push_str("\n[dependencies.tauri]\nversion = \"2.11.2\"\nfeatures = [\"devtools\"]\n");
    });
    let crate_feature = audit_with(|i| {
        replace_once(
            &mut i.cargo_toml,
            "export-bindings = []",
            "export-bindings = [\"tauri/devtools\"]",
        )
    });
    for (form, found) in [("table", table_form), ("crate feature", crate_feature)] {
        assert_reports(&found, "devtools", "tauri dependency");
        assert!(
            found.iter().any(|v| v.rule == "tauri-features"),
            "{form}: {}",
            render(&found)
        );
    }
}

#[test]
fn a_baseline_edit_alone_cannot_approve_a_forbidden_grant() {
    // Editing BOTH the configuration and the baseline to agree on a forbidden grant
    // still fails: the ADR 0010 invariants are code, not data.
    let found = audit_with(|i| {
        edit_capability(
            i,
            "agent-report.json",
            r#""permissions": []"#,
            r#""permissions": ["allow-app-commands"]"#,
        );
        replace_once(
            &mut i.baseline,
            "capabilities = [\"agent-report\"]\ngrants = []",
            "capabilities = [\"agent-report\"]\ngrants = [\"allow-app-commands\"]",
        );
    });
    assert!(
        !found.iter().any(|v| v.rule == "window-grants"),
        "drift agrees by construction:\n{}",
        render(&found)
    );
    assert_reports(&found, "untrusted-grant", "agent_report");

    let relabelled = audit_with(|i| {
        replace_once(
            &mut i.baseline,
            "[windows.agent_report]\ntrust = \"untrusted\"",
            "[windows.agent_report]\ntrust = \"trusted\"",
        )
    });
    assert_reports(&relabelled, "untrusted-grant", "agent_report");

    let unsafe_script = audit_with(|i| {
        replace_once(
            &mut i.baseline,
            "\"script-src\" = [\"'self'\"]",
            "\"script-src\" = [\"'self'\", \"'unsafe-inline'\"]",
        );
        replace_once(
            &mut i.tauri_conf,
            "\"csp\": \"default-src 'self'; script-src 'self';",
            "\"csp\": \"default-src 'self'; script-src 'self' 'unsafe-inline';",
        );
    });
    assert_reports(&unsafe_script, "csp-invariant", "script-src");
}

#[test]
fn a_baseline_edit_alone_cannot_approve_network_egress() {
    // 04a-review F1 (PR 40): a remote origin added to connect-src in BOTH the config
    // and the baseline used to pass. Egress is now an ADR 0010 invariant in code.
    let remote = "https://api.example.com";
    let prod = audit_with(|i| {
        replace_once(
            &mut i.tauri_conf,
            "connect-src 'self' ipc: http://ipc.localhost; img-src",
            &format!("connect-src 'self' ipc: http://ipc.localhost {remote}; img-src"),
        );
        replace_once(
            &mut i.baseline,
            "\"connect-src\" = [\"'self'\", \"ipc:\", \"http://ipc.localhost\"]",
            &format!(
                "\"connect-src\" = [\"'self'\", \"ipc:\", \"http://ipc.localhost\", \"{remote}\"]"
            ),
        );
    });
    assert!(
        !prod.iter().any(|v| v.rule == "csp"),
        "drift agrees by construction:\n{}",
        render(&prod)
    );
    assert_reports(
        &prod,
        "csp-egress",
        "tauri.conf.json production connect-src",
    );
    assert_reports(&prod, "csp-egress", "baseline production connect-src");

    // The same for the development policy, and for a directive other than connect-src.
    let dev = audit_with(|i| {
        replace_once(
            &mut i.tauri_conf,
            "http://localhost:1420; img-src 'self' data:;",
            "http://localhost:1420; img-src 'self' data: https:;",
        );
        replace_once(
            &mut i.baseline,
            "\"img-src\" = [\"'self'\", \"data:\"]\n\"object-src\" = [\"'none'\"]\n\"script-src\" = [\"'self'\", \"'unsafe-inline'\"",
            "\"img-src\" = [\"'self'\", \"data:\", \"https:\"]\n\"object-src\" = [\"'none'\"]\n\"script-src\" = [\"'self'\", \"'unsafe-inline'\"",
        );
    });
    assert_reports(&dev, "csp-egress", "tauri.conf.json development img-src");
    assert_reports(&dev, "csp-egress", "baseline development img-src");
}

#[test]
fn the_baseline_rejects_unknown_fields() {
    // A typo in the baseline must not silently turn a check off.
    let found = audit_with(|i| {
        replace_once(
            &mut i.baseline,
            "[production]\n",
            "[production]\nwith_global_tauri_typo = true\n",
        )
    });
    assert_reports(&found, "baseline-shape", "expected-capabilities.toml");
}

#[test]
fn ci_runs_the_audit_on_every_change_it_covers() {
    // Config, capabilities, permissions, command registration (src/lib.rs), the
    // baseline and the audit itself all live under apps/desktop/src-tauri/, which the
    // desktop job's path filter watches and whose gate runs the whole `cargo test`.
    let ci = std::fs::read_to_string(manifest_dir().join("../../../.github/workflows/ci.yml"))
        .expect("ci.yml is readable");
    assert!(
        ci.contains("- 'apps/desktop/src-tauri/**'"),
        "the desktop job's path filter must watch apps/desktop/src-tauri/**"
    );
    let gate = ci
        .split("- name: Rust gates (desktop crate)")
        .nth(1)
        .expect("the desktop Rust gate step exists");
    let gate = &gate[..gate.find("- name:").unwrap_or(gate.len())];
    assert!(
        gate.contains("working-directory: apps/desktop/src-tauri") && gate.contains("cargo test\n"),
        "the desktop gate runs the full `cargo test`, which includes this audit"
    );
    for file in [
        "expected-capabilities.toml",
        "tests/support/capability_audit.rs",
    ] {
        assert!(
            manifest_dir().join(file).is_file(),
            "{file} lives under src-tauri/"
        );
    }
}
