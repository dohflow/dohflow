//! Capability drift audit (personal-cfo-cjd, ADR 0010 and all its addenda).
//!
//! Pure functions over file CONTENTS — never paths — so the tests in
//! `acl_coverage.rs` can hand the audit a deliberately broken copy of any input
//! and prove it fails with the rule and window named.
//!
//! Two kinds of check:
//!
//! * **Drift** — what the configuration actually grants, per window, compared
//!   with the reviewed baseline in `expected-capabilities.toml`.
//! * **Invariants** — ADR 0010 rules that hold whatever the baseline says, checked
//!   against the configuration AND the baseline, so a baseline edit alone can never
//!   approve a forbidden grant.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

/// Windows ADR 0010 classifies as untrusted. Pinned in code, not only in the
/// baseline, so relabelling one "trusted" in the baseline is itself a violation.
pub const UNTRUSTED_WINDOWS: [&str; 2] = ["document_preview", "agent_report"];

/// The app permission that carries the destructive/exfiltration commands.
pub const DESTRUCTIVE_SET: &str = "allow-destructive-commands";

/// Every input the audit reads, as file contents.
#[derive(Clone)]
pub struct Inputs {
    pub tauri_conf: String,
    /// `(file name, contents)` for every `capabilities/*.json`.
    pub capabilities: Vec<(String, String)>,
    /// `(file name, contents)` for every `permissions/*.toml`.
    pub permissions: Vec<(String, String)>,
    pub lib_rs: String,
    pub cargo_toml: String,
    pub baseline: String,
}

/// One failed rule. `subject` names the window, capability, file or directive.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Violation {
    pub rule: &'static str,
    pub subject: String,
    pub detail: String,
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}: {}", self.rule, self.subject, self.detail)
    }
}

// ---------------------------------------------------------------------------
// The baseline file.
// ---------------------------------------------------------------------------

type Directives = BTreeMap<String, Vec<String>>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Baseline {
    production: Production,
    development: Development,
    windows: BTreeMap<String, WindowEntry>,
    #[serde(default)]
    scopes: Vec<ScopeEntry>,
    permission_sets: BTreeMap<String, PermissionSetEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Production {
    with_global_tauri: bool,
    security_keys: Vec<String>,
    tauri_features: Vec<String>,
    csp: Directives,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Development {
    csp: Directives,
}

#[derive(Deserialize, PartialEq, Eq, Clone, Copy, Debug)]
#[serde(rename_all = "lowercase")]
enum Trust {
    Trusted,
    Untrusted,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WindowEntry {
    trust: Trust,
    in_config: bool,
    capabilities: Vec<String>,
    grants: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScopeEntry {
    window: String,
    permission: String,
    #[serde(default)]
    allow: Vec<toml::Value>,
    #[serde(default)]
    deny: Vec<toml::Value>,
}

#[derive(Deserialize, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum Inventory {
    Lockstep,
    Exact,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PermissionSetEntry {
    file: String,
    inventory: Inventory,
    #[serde(default)]
    commands: Vec<String>,
}

// ---------------------------------------------------------------------------
// What the configuration actually grants.
// ---------------------------------------------------------------------------

struct Capability {
    file: String,
    identifier: String,
    windows: Vec<String>,
    /// `(identifier, scope)` — `scope` is `Some((allow, deny))` for object-form entries.
    permissions: Vec<(String, Option<(serde_json::Value, serde_json::Value)>)>,
    extra_keys: Vec<String>,
}

/// A scope as a canonical, comparable string: `window|permission|allow|deny`.
type ScopeKey = String;

fn scope_key(
    window: &str,
    permission: &str,
    allow: &serde_json::Value,
    deny: &serde_json::Value,
) -> ScopeKey {
    format!("{window}|{permission}|allow={allow}|deny={deny}")
}

fn v(rule: &'static str, subject: impl Into<String>, detail: impl Into<String>) -> Violation {
    Violation {
        rule,
        subject: subject.into(),
        detail: detail.into(),
    }
}

fn parse_capability(file: &str, text: &str) -> Result<Capability, Violation> {
    let json: serde_json::Value = serde_json::from_str(text)
        .map_err(|e| v("capability-shape", file, format!("invalid JSON: {e}")))?;
    let obj = json
        .as_object()
        .ok_or_else(|| v("capability-shape", file, "not a JSON object"))?;
    let allowed = [
        "$schema",
        "identifier",
        "description",
        "windows",
        "permissions",
    ];
    let extra_keys = obj
        .keys()
        .filter(|k| !allowed.contains(&k.as_str()))
        .cloned()
        .collect();
    let windows = obj
        .get("windows")
        .and_then(|w| w.as_array())
        .ok_or_else(|| {
            v(
                "capability-shape",
                file,
                "must scope by an explicit `windows` list",
            )
        })?
        .iter()
        .map(|w| w.as_str().unwrap_or("<non-string>").to_owned())
        .collect();
    let mut permissions = Vec::new();
    for entry in obj
        .get("permissions")
        .and_then(|p| p.as_array())
        .ok_or_else(|| v("capability-shape", file, "missing `permissions` array"))?
    {
        match entry {
            serde_json::Value::String(id) => permissions.push((id.clone(), None)),
            serde_json::Value::Object(o) => {
                let id = o
                    .get("identifier")
                    .and_then(|i| i.as_str())
                    .ok_or_else(|| {
                        v("capability-shape", file, "scoped entry without identifier")
                    })?;
                let empty = serde_json::Value::Array(vec![]);
                permissions.push((
                    id.to_owned(),
                    Some((
                        o.get("allow").cloned().unwrap_or_else(|| empty.clone()),
                        o.get("deny").cloned().unwrap_or(empty),
                    )),
                ));
            }
            other => return Err(v("capability-shape", file, format!("bad entry {other}"))),
        }
    }
    Ok(Capability {
        file: file.to_owned(),
        identifier: obj
            .get("identifier")
            .and_then(|i| i.as_str())
            .unwrap_or("")
            .to_owned(),
        windows,
        permissions,
        extra_keys,
    })
}

/// Command idents out of the `collect_commands![...]` block. Scans for every
/// `ipc::commands::` occurrence rather than line-by-line, so a rustfmt reflow can
/// never silently drop a command from the comparison.
pub fn registered_commands(lib_rs: &str) -> BTreeSet<String> {
    let Some(start) = lib_rs.find("collect_commands![") else {
        return BTreeSet::new();
    };
    let end = lib_rs[start..]
        .find(']')
        .map_or(lib_rs.len(), |e| e + start);
    lib_rs[start..end]
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

/// `(identifier, commands.allow)` for each `[[permission]]` in a permission file.
fn parse_permission_file(file: &str, text: &str) -> Result<Vec<(String, Vec<String>)>, Violation> {
    let table: toml::Table = text
        .parse()
        .map_err(|e| v("permission-shape", file, format!("invalid TOML: {e}")))?;
    if let Some(key) = table.keys().find(|k| k.as_str() != "permission") {
        return Err(v(
            "permission-shape",
            file,
            format!("unexpected top-level `{key}` — permission files hold [[permission]] only"),
        ));
    }
    let mut out = Vec::new();
    for perm in table
        .get("permission")
        .and_then(|p| p.as_array())
        .ok_or_else(|| v("permission-shape", file, "no [[permission]] entries"))?
    {
        let id = perm
            .get("identifier")
            .and_then(|i| i.as_str())
            .ok_or_else(|| v("permission-shape", file, "permission without identifier"))?;
        let commands = perm.get("commands");
        if commands.and_then(|c| c.get("deny")).is_some() || perm.get("scope").is_some() {
            return Err(v(
                "permission-shape",
                file,
                format!("`{id}` uses commands.deny or a scope — not part of the reviewed model"),
            ));
        }
        let allow = commands
            .and_then(|c| c.get("allow"))
            .and_then(|a| a.as_array())
            .ok_or_else(|| {
                v(
                    "permission-shape",
                    file,
                    format!("`{id}` has no commands.allow"),
                )
            })?
            .iter()
            .map(|c| c.as_str().unwrap_or("<non-string>").to_owned())
            .collect();
        out.push((id.to_owned(), allow));
    }
    Ok(out)
}

/// Features that can reach the `tauri` dependency in a release build: from
/// `[dependencies]` / `[target.*.dependencies]` in inline or table form, and from
/// any crate `[features]` entry naming `tauri/<feature>` or `tauri?/<feature>`.
/// (Dev-dependencies are excluded: they never reach a release build.)
pub fn tauri_release_features(cargo_toml: &str) -> Result<BTreeSet<String>, Violation> {
    let table: toml::Table = cargo_toml
        .parse()
        .map_err(|e| v("cargo-shape", "Cargo.toml", format!("invalid TOML: {e}")))?;
    let mut features = BTreeSet::new();
    let mut dep_tables: Vec<&toml::Value> = table.get("dependencies").into_iter().collect();
    if let Some(targets) = table.get("target").and_then(|t| t.as_table()) {
        dep_tables.extend(targets.values().filter_map(|t| t.get("dependencies")));
    }
    for deps in dep_tables {
        if let Some(features_list) = deps
            .get("tauri")
            .and_then(|d| d.get("features"))
            .and_then(|f| f.as_array())
        {
            features.extend(
                features_list
                    .iter()
                    .filter_map(|f| f.as_str())
                    .map(str::to_owned),
            );
        }
    }
    if let Some(crate_features) = table.get("features").and_then(|f| f.as_table()) {
        for enables in crate_features.values().filter_map(|e| e.as_array()) {
            for item in enables.iter().filter_map(|i| i.as_str()) {
                if let Some(feature) = item
                    .strip_prefix("tauri/")
                    .or_else(|| item.strip_prefix("tauri?/"))
                {
                    features.insert(feature.to_owned());
                }
            }
        }
    }
    Ok(features)
}

/// A CSP string as `directive -> sources`. A repeated directive is a violation:
/// the browser silently ignores the second copy.
fn parse_csp(which: &'static str, csp: &str, out: &mut Vec<Violation>) -> Directives {
    let mut map = Directives::new();
    for part in csp.split(';').map(str::trim).filter(|p| !p.is_empty()) {
        let mut tokens = part.split_whitespace().map(str::to_owned);
        let name = tokens.next().unwrap_or_default();
        if map.insert(name.clone(), tokens.collect()).is_some() {
            out.push(v(
                which,
                name,
                "directive repeated — the second copy is ignored",
            ));
        }
    }
    map
}

fn diff_directives(
    rule: &'static str,
    actual: &Directives,
    expected: &Directives,
    out: &mut Vec<Violation>,
) {
    let names: BTreeSet<&String> = actual.keys().chain(expected.keys()).collect();
    for name in names {
        match (actual.get(name), expected.get(name)) {
            (Some(a), Some(e)) if a == e => {}
            (Some(a), Some(e)) => out.push(v(rule, name, format!("is {a:?}, baseline {e:?}"))),
            (Some(a), None) => out.push(v(rule, name, format!("added {a:?} (not in baseline)"))),
            (None, Some(_)) => out.push(v(rule, name, "removed (baseline has it)")),
            (None, None) => {}
        }
    }
}

fn diff_sets(
    rule: &'static str,
    subject: &str,
    what: &str,
    actual: &BTreeSet<String>,
    expected: &BTreeSet<String>,
    out: &mut Vec<Violation>,
) {
    for gained in actual.difference(expected) {
        out.push(v(
            rule,
            subject,
            format!("gained {what} `{gained}` (not in baseline)"),
        ));
    }
    for lost in expected.difference(actual) {
        out.push(v(
            rule,
            subject,
            format!("lost {what} `{lost}` (baseline has it)"),
        ));
    }
}

// ---------------------------------------------------------------------------
// ADR 0010 invariants — applied to the configuration AND to the baseline.
// ---------------------------------------------------------------------------

/// Production CSP rules: `script-src` never carries an unsafe keyword, a wildcard
/// or a remote/scheme source; `'unsafe-inline'` appears only in `style-src`.
fn production_csp_invariants(source: &str, csp: &Directives, out: &mut Vec<Violation>) {
    for (name, sources) in csp {
        for s in sources {
            let unsafe_kw = s.starts_with("'unsafe-") || s == "'wasm-unsafe-eval'";
            if unsafe_kw && !(name == "style-src" && s == "'unsafe-inline'") {
                out.push(v(
                    "csp-invariant",
                    format!("{source} {name}"),
                    format!("{s} — only style-src 'unsafe-inline' is accepted (ADR 0010)"),
                ));
            }
        }
    }
    match csp.get("script-src") {
        Some(script) if script == &["'self'".to_owned()] => {}
        Some(script) => out.push(v(
            "csp-invariant",
            format!("{source} script-src"),
            format!("is {script:?} — must be exactly 'self' (ADR 0010)"),
        )),
        None => out.push(v(
            "csp-invariant",
            format!("{source} script-src"),
            "missing",
        )),
    }
}

/// Development may differ from production ONLY by HMR's script relaxations and the
/// dev server in connect-src — and nothing development-only may appear in production.
fn dev_csp_invariants(
    source: &str,
    prod: &Directives,
    dev: &Directives,
    dev_origin: &str,
    out: &mut Vec<Violation>,
) {
    let host = dev_origin
        .trim_end_matches('/')
        .trim_start_matches("http://");
    let mut allowed = prod.clone();
    allowed.insert(
        "script-src".into(),
        vec![
            "'self'".into(),
            "'unsafe-inline'".into(),
            "'unsafe-eval'".into(),
        ],
    );
    if let Some(connect) = allowed.get_mut("connect-src") {
        connect.extend([format!("ws://{host}"), format!("http://{host}")]);
    }
    if dev != &allowed {
        let mut sub = Vec::new();
        diff_directives("dev-csp-invariant", dev, &allowed, &mut sub);
        out.extend(sub.into_iter().map(|mut x| {
            x.subject = format!("{source} devCsp {}", x.subject);
            x
        }));
    }
    for (name, sources) in prod {
        for s in sources {
            if s.contains(host) || s.starts_with("ws:") || s == "'unsafe-eval'" {
                out.push(v(
                    "dev-leak",
                    format!("{source} production {name}"),
                    format!("{s} is development-only and must not reach production"),
                ));
            }
        }
    }
}

/// Grant rules per window, whatever the baseline says.
fn grant_invariants(
    source: &str,
    grants: &BTreeMap<String, BTreeSet<String>>,
    trust: &BTreeMap<String, Trust>,
    out: &mut Vec<Violation>,
) {
    for (window, set) in grants {
        let untrusted = UNTRUSTED_WINDOWS.contains(&window.as_str())
            || trust.get(window) == Some(&Trust::Untrusted);
        if untrusted && !set.is_empty() {
            out.push(v(
                "untrusted-grant",
                window.clone(),
                format!(
                    "{source} grants {set:?} — an untrusted window gets nothing without its \
                     own ADR 0010 addendum"
                ),
            ));
        }
        if set.contains(DESTRUCTIVE_SET) && (untrusted || window != "main") {
            out.push(v(
                "destructive-reach",
                window.clone(),
                format!("{source} grants {DESTRUCTIVE_SET} beyond the trusted main window"),
            ));
        }
        for id in set {
            let forbidden = ["shell:", "fs:", "http:"].iter().any(|p| id.starts_with(p))
                || [
                    "opener:default",
                    "opener:allow-default-urls",
                    "opener:allow-open-path",
                    "opener:allow-reveal-item-in-dir",
                    "process:default",
                ]
                .contains(&id.as_str());
            if forbidden {
                out.push(v(
                    "forbidden-grant",
                    window.clone(),
                    format!("{source} grants `{id}`, which ADR 0010 never allows"),
                ));
            }
        }
    }
    for label in UNTRUSTED_WINDOWS {
        if trust.get(label) == Some(&Trust::Trusted) {
            out.push(v(
                "untrusted-grant",
                label,
                format!("{source} labels it trusted — ADR 0010 classifies it untrusted"),
            ));
        }
    }
}

// ---------------------------------------------------------------------------
// The audit.
// ---------------------------------------------------------------------------

/// Every violation, sorted. Empty means the configuration matches the reviewed
/// baseline and every ADR 0010 invariant holds for both.
#[must_use]
pub fn audit(inputs: &Inputs) -> Vec<Violation> {
    let mut out = Vec::new();

    let baseline: Baseline = match toml::from_str(&inputs.baseline) {
        Ok(b) => b,
        Err(e) => {
            out.push(v(
                "baseline-shape",
                "expected-capabilities.toml",
                e.to_string(),
            ));
            return out;
        }
    };
    let conf: serde_json::Value = match serde_json::from_str(&inputs.tauri_conf) {
        Ok(c) => c,
        Err(e) => {
            out.push(v("config-shape", "tauri.conf.json", e.to_string()));
            return out;
        }
    };

    // --- production webview settings -----------------------------------------
    let security = conf["app"]["security"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    let keys: Vec<String> = security.keys().cloned().collect();
    if keys != baseline.production.security_keys {
        out.push(v(
            "security-keys",
            "app.security",
            format!(
                "keys {keys:?}, baseline {:?}",
                baseline.production.security_keys
            ),
        ));
    }
    let global = conf["app"]["withGlobalTauri"].as_bool().unwrap_or(false);
    if global != baseline.production.with_global_tauri || global {
        out.push(v(
            "global-tauri",
            "app.withGlobalTauri",
            format!("is {global}; must stay false"),
        ));
    }
    match tauri_release_features(&inputs.cargo_toml) {
        Ok(features) => {
            let expected: BTreeSet<String> =
                baseline.production.tauri_features.iter().cloned().collect();
            diff_sets(
                "tauri-features",
                "tauri dependency",
                "feature",
                &features,
                &expected,
                &mut out,
            );
            for set in [&features, &expected] {
                if set.contains("devtools") {
                    out.push(v(
                        "devtools",
                        "tauri dependency",
                        "the `devtools` feature enables the inspector in release builds",
                    ));
                }
            }
        }
        Err(e) => out.push(e),
    }

    // --- CSP ------------------------------------------------------------------
    let prod = parse_csp(
        "csp",
        security.get("csp").and_then(|c| c.as_str()).unwrap_or(""),
        &mut out,
    );
    let dev = parse_csp(
        "dev-csp",
        security
            .get("devCsp")
            .and_then(|c| c.as_str())
            .unwrap_or(""),
        &mut out,
    );
    diff_directives("csp", &prod, &baseline.production.csp, &mut out);
    diff_directives("dev-csp", &dev, &baseline.development.csp, &mut out);
    let dev_url = conf["build"]["devUrl"].as_str().unwrap_or("");
    if !dev_url.starts_with("http://localhost:") {
        out.push(v(
            "dev-csp-invariant",
            "build.devUrl",
            format!("{dev_url} is not the local dev server"),
        ));
    }
    production_csp_invariants("tauri.conf.json", &prod, &mut out);
    production_csp_invariants("baseline", &baseline.production.csp, &mut out);
    dev_csp_invariants("tauri.conf.json", &prod, &dev, dev_url, &mut out);
    dev_csp_invariants(
        "baseline",
        &baseline.production.csp,
        &baseline.development.csp,
        dev_url,
        &mut out,
    );

    // --- configured windows ---------------------------------------------------
    let mut in_config: BTreeSet<String> = BTreeSet::new();
    for window in conf["app"]["windows"]
        .as_array()
        .cloned()
        .unwrap_or_default()
    {
        let label = window["label"].as_str().unwrap_or("main").to_owned();
        if let Some(url) = window.get("url").and_then(|u| u.as_str()) {
            if url.contains(':') {
                out.push(v(
                    "remote-origin",
                    label.clone(),
                    format!("window loads non-app URL {url}"),
                ));
            }
        }
        if window.get("devtools") == Some(&serde_json::Value::Bool(true)) {
            out.push(v("devtools", label.clone(), "window opts into devtools"));
        }
        in_config.insert(label);
    }

    // --- capabilities → effective grants per window ----------------------------
    let mut grants: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut caps_by_window: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut scopes: BTreeSet<ScopeKey> = BTreeSet::new();
    for (file, text) in &inputs.capabilities {
        let cap = match parse_capability(file, text) {
            Ok(c) => c,
            Err(e) => {
                out.push(e);
                continue;
            }
        };
        for key in &cap.extra_keys {
            let rule = if key == "remote" {
                "remote-origin"
            } else {
                "capability-shape"
            };
            out.push(v(
                rule,
                cap.file.clone(),
                format!("declares `{key}` — capabilities scope by explicit window labels only"),
            ));
        }
        for window in &cap.windows {
            if window.contains('*') || window.contains('?') {
                out.push(v(
                    "wildcard-window",
                    cap.file.clone(),
                    format!("targets `{window}` — every capability names its windows exactly"),
                ));
            }
            caps_by_window
                .entry(window.clone())
                .or_default()
                .insert(cap.identifier.clone());
            let set = grants.entry(window.clone()).or_default();
            for (id, scope) in &cap.permissions {
                set.insert(id.clone());
                if let Some((allow, deny)) = scope {
                    scopes.insert(scope_key(window, id, allow, deny));
                }
            }
        }
    }

    // --- window topology + per-window grant drift ------------------------------
    let actual_windows: BTreeSet<String> = in_config.iter().chain(grants.keys()).cloned().collect();
    let expected_windows: BTreeSet<String> = baseline.windows.keys().cloned().collect();
    diff_sets(
        "window-set",
        "topology",
        "window",
        &actual_windows,
        &expected_windows,
        &mut out,
    );
    for (label, entry) in &baseline.windows {
        if actual_windows.contains(label) && in_config.contains(label) != entry.in_config {
            out.push(v(
                "window-config",
                label.clone(),
                format!(
                    "in tauri.conf.json: {}, baseline: {}",
                    in_config.contains(label),
                    entry.in_config
                ),
            ));
        }
        let empty = BTreeSet::new();
        let expected_caps: BTreeSet<String> = entry.capabilities.iter().cloned().collect();
        diff_sets(
            "window-capabilities",
            label,
            "capability",
            caps_by_window.get(label).unwrap_or(&empty),
            &expected_caps,
            &mut out,
        );
        let expected_grants: BTreeSet<String> = entry.grants.iter().cloned().collect();
        diff_sets(
            "window-grants",
            label,
            "grant",
            grants.get(label).unwrap_or(&empty),
            &expected_grants,
            &mut out,
        );
    }

    // --- scopes ---------------------------------------------------------------
    let expected_scopes: BTreeSet<ScopeKey> = baseline
        .scopes
        .iter()
        .map(|s| {
            let to_json = |vals: &Vec<toml::Value>| {
                serde_json::to_value(vals).unwrap_or(serde_json::Value::Null)
            };
            scope_key(
                &s.window,
                &s.permission,
                &to_json(&s.allow),
                &to_json(&s.deny),
            )
        })
        .collect();
    diff_sets(
        "scope",
        "scoped grants",
        "scope",
        &scopes,
        &expected_scopes,
        &mut out,
    );

    // --- invariants over configuration and baseline ------------------------------
    let trust: BTreeMap<String, Trust> = baseline
        .windows
        .iter()
        .map(|(k, w)| (k.clone(), w.trust))
        .collect();
    grant_invariants("configuration", &grants, &trust, &mut out);
    let baseline_grants: BTreeMap<String, BTreeSet<String>> = baseline
        .windows
        .iter()
        .map(|(k, w)| (k.clone(), w.grants.iter().cloned().collect()))
        .collect();
    grant_invariants("baseline", &baseline_grants, &trust, &mut out);

    // --- app permission sets + ACL coverage -------------------------------------
    let mut sets: BTreeMap<String, (String, Vec<String>)> = BTreeMap::new();
    for (file, text) in &inputs.permissions {
        match parse_permission_file(file, text) {
            Ok(perms) => {
                for (id, commands) in perms {
                    sets.insert(id, (file.clone(), commands));
                }
            }
            Err(e) => out.push(e),
        }
    }
    let actual_ids: BTreeSet<String> = sets.keys().cloned().collect();
    let expected_ids: BTreeSet<String> = baseline.permission_sets.keys().cloned().collect();
    diff_sets(
        "permission-set",
        "permissions/",
        "permission set",
        &actual_ids,
        &expected_ids,
        &mut out,
    );
    let mut owner: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (id, (file, commands)) in &sets {
        for c in commands {
            owner.entry(c.clone()).or_default().push(id.clone());
        }
        if let Some(entry) = baseline.permission_sets.get(id) {
            if &entry.file != file {
                out.push(v(
                    "permission-set",
                    id.clone(),
                    format!("lives in {file}, baseline {}", entry.file),
                ));
            }
            if entry.inventory == Inventory::Exact {
                let actual: BTreeSet<String> = commands.iter().cloned().collect();
                let expected: BTreeSet<String> = entry.commands.iter().cloned().collect();
                diff_sets(
                    "permission-commands",
                    id,
                    "command",
                    &actual,
                    &expected,
                    &mut out,
                );
            } else if !entry.commands.is_empty() {
                out.push(v(
                    "baseline-shape",
                    id.clone(),
                    "a lockstep set lists no commands — its inventory is permissions/*.toml",
                ));
            }
        }
    }
    let registered = registered_commands(&inputs.lib_rs);
    if registered.len() < 100 {
        out.push(v(
            "acl-coverage",
            "src/lib.rs",
            format!(
                "parsed only {} commands from collect_commands!",
                registered.len()
            ),
        ));
    }
    for cmd in &registered {
        match owner.get(cmd).map(Vec::len) {
            None => out.push(v(
                "acl-coverage",
                cmd.clone(),
                "registered but in no permission set — denied at runtime",
            )),
            Some(1) => {}
            Some(_) => out.push(v(
                "acl-coverage",
                cmd.clone(),
                format!("granted by several sets {:?}", owner[cmd]),
            )),
        }
    }
    for cmd in owner.keys().filter(|c| !registered.contains(*c)) {
        out.push(v(
            "acl-coverage",
            cmd.clone(),
            "granted but not registered in collect_commands!",
        ));
    }

    out.sort();
    out.dedup();
    out
}
