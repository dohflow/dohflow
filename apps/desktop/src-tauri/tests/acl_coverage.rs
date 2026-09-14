//! ACL coverage guard (personal-cfo-3fdd.6).
//!
//! With app permissions present, Tauri v2 is deny-by-default for the ENTIRE custom
//! command surface: any command registered in `collect_commands!` but missing from
//! `permissions/*.toml` fails at runtime with "not allowed by ACL". This test keeps
//! the three lists (registration, general grants, destructive grants) in lockstep so
//! that drift is a test failure, not a broken app.

use std::collections::BTreeSet;

const LIB_RS: &str = include_str!("../src/lib.rs");
const APP_COMMANDS: &str = include_str!("../permissions/app-commands.toml");
const DESTRUCTIVE_COMMANDS: &str = include_str!("../permissions/destructive-commands.toml");

/// Command idents out of the `collect_commands![...]` block. Parses by scanning for
/// every `ipc::commands::` occurrence rather than line-by-line, so a rustfmt reflow
/// (several commands on one line, wrapped items, comments) can never silently drop a
/// command from the comparison (review fold, 2026-07-05).
fn registered_commands() -> BTreeSet<String> {
    let start = LIB_RS
        .find("collect_commands![")
        .expect("collect_commands! block");
    let end = LIB_RS[start..].find(']').expect("block end") + start;
    LIB_RS[start..end]
        .split("ipc::commands::")
        .skip(1)
        .map(|rest| {
            rest.chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect::<String>()
        })
        .filter(|ident| !ident.is_empty())
        .collect()
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
    let destructive = granted(DESTRUCTIVE_COMMANDS);
    // Growing this list is fine — but it must be a conscious decision, so the test
    // names the expected set instead of just counting.
    let expected: BTreeSet<String> = [
        "delete_vault",
        "export_backup",
        "restore_backup",
        "apply_update",
        "relaunch_app",
        "export_transactions_csv",
        // Hard, un-op-logged DELETE of a stored connector credential
        // (personal-cfo-gglk) — deliberate addition, 2026-08-22.
        "connector_forget",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    assert_eq!(
        destructive, expected,
        "the destructive/exfiltration capability changed — update this test deliberately"
    );
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

/// Every capability file the app ships, by name. Both target the `main` window
/// today, and Tauri MERGES scopes across every capability that targets a window
/// (the opener's `open_url` checks the command scope chained with the global
/// scope), so a grant in one file silently widens a grant in the other. Any test
/// that pins "exactly one" of something must therefore look at the union.
const CAPABILITIES: [(&str, &str); 2] = [
    ("default.json", DEFAULT_CAPABILITY),
    ("destructive.json", DESTRUCTIVE_CAPABILITY),
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
