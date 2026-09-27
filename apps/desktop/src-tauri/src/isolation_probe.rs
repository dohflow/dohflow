//! Runtime isolation probe (personal-cfo-0hp6, ADR 0010 and its addenda).
//!
//! Debug builds only — this module is compiled out of release builds entirely.
//!
//! The static suite (`tests/acl_coverage.rs`, the drift audit) and the mock-runtime
//! ACL test (`tests/window_isolation.rs`) prove what the configuration SAYS. This
//! probe proves what a real WebView DOES: it opens every window of the shipped
//! topology, runs a script in each through `eval_with_callback` — a path that needs
//! no Tauri IPC, so the untrusted shells can be probed without granting them
//! anything — and records, per window:
//!
//! * the page's origin (the first load, which plugin navigation hooks never see);
//! * that no Tauri global is exposed;
//! * `invoke` outcomes: `main` reaches the general surface; each untrusted shell is
//!   rejected by the ACL for an app, a destructive, and two core commands;
//! * the EFFECTIVE Content Security Policy, from the browser's own
//!   `securitypolicyviolation` reports: an injected inline `<script>` and `eval` are
//!   refused, a remote `fetch` is refused by `connect-src`, and an inline `style`
//!   attribute still applies (the accepted `style-src 'unsafe-inline'` exception —
//!   which a nonce in `style-src` would silently switch off);
//! * that `window.open` creates nothing, and that a remote navigation is cancelled
//!   by the navigation guard itself (its per-window count must rise) and leaves the
//!   page on the app origin.
//!
//! Run it from a bundled-origin debug build (`tauri build --debug`, or
//! `cargo build --features tauri/custom-protocol`) with a scratch data directory:
//!
//! ```text
//! PCFO_DATA_DIR=$(mktemp -d) PCFO_ISOLATION_PROBE=report.json <binary>
//! ```
//!
//! It writes a JSON report and exits 0 only if every check passed.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tauri::{AppHandle, Manager, Runtime, WebviewWindow};

use crate::navigation_guard::probe_evidence;
use crate::windows::{self, IsolatedSurface, MAIN};

/// Environment variable naming the report path; the probe runs only when set.
pub const PROBE_ENV: &str = "PCFO_ISOLATION_PROBE";

/// A remote origin every check points at. It is in the reserved `.invalid` domain
/// (RFC 6761), which never resolves, so no host is contacted even if a control
/// failed — and so no check can pass merely because a real page loaded or not:
/// each one requires positive evidence of the refusal (a CSP violation report, an
/// ACL rejection, or the navigation guard's own count).
const REMOTE: &str = "https://probe.invalid";

/// The origin Tauri serves the bundle from on this platform.
fn app_origin() -> &'static str {
    if cfg!(windows) || cfg!(target_os = "android") {
        "http://tauri.localhost"
    } else {
        "tauri://localhost"
    }
}

/// Start the probe on a background thread if [`PROBE_ENV`] is set.
///
/// # Errors
/// Returns an error if the probe is requested without a scratch data directory
/// or from a dev-server build, where it would not be testing the shipped origin.
pub fn start_if_requested<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let Ok(report_path) = std::env::var(PROBE_ENV) else {
        return Ok(());
    };
    if std::env::var_os("PCFO_DATA_DIR").is_none() {
        return Err(format!(
            "{PROBE_ENV} requires PCFO_DATA_DIR to point at a scratch directory"
        ));
    }
    if tauri::is_dev() {
        return Err(format!(
            "{PROBE_ENV} needs a bundled-origin build (tauri build --debug, or \
             cargo build --features tauri/custom-protocol), not the dev server"
        ));
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let report = run(&app);
        let passed = report["passed"].as_bool().unwrap_or(false);
        let text = serde_json::to_string_pretty(&report).unwrap_or_default();
        if let Err(err) = std::fs::write(&report_path, &text) {
            eprintln!("isolation probe: cannot write {report_path}: {err}");
        }
        println!("{text}");
        println!(
            "DohFlow isolation probe: {}",
            if passed { "PASS" } else { "FAIL" }
        );
        std::process::exit(i32::from(!passed));
    });
    Ok(())
}

/// Evaluate `js` in `window` and return its value, decoded from the JSON string the
/// webview hands back.
fn eval<R: Runtime>(
    window: &WebviewWindow<R>,
    js: &str,
    timeout: Duration,
) -> Result<Value, String> {
    let (tx, rx) = mpsc::channel();
    window
        .eval_with_callback(js, move |result| {
            let _ = tx.send(result);
        })
        .map_err(|e| e.to_string())?;
    let raw = rx
        .recv_timeout(timeout)
        .map_err(|_| "eval timed out".to_owned())?;
    serde_json::from_str(&raw).map_err(|e| format!("unparseable eval result {raw:?}: {e}"))
}

/// Poll `js` until it returns a value `done` accepts.
fn poll<R: Runtime>(
    window: &WebviewWindow<R>,
    js: &str,
    timeout: Duration,
    done: impl Fn(&Value) -> bool,
) -> Result<Value, String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(value) = eval(window, js, Duration::from_secs(2)) {
            if done(&value) {
                return Ok(value);
            }
        }
        if Instant::now() > deadline {
            return Err(format!("timed out waiting on `{js}`"));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn window_or_err<R: Runtime>(app: &AppHandle<R>, label: &str) -> Result<WebviewWindow<R>, String> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(window) = app.get_webview_window(label) {
            return Ok(window);
        }
        if Instant::now() > deadline {
            return Err(format!("window `{label}` never appeared"));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The in-page script. Runs asynchronously and leaves its results on
/// `window.__isolationProbe`, which [`probe_window`] polls.
fn page_script(trusted: bool) -> String {
    format!(
        r#"(() => {{
  const P = {{ done: false, checks: {{}}, violations: [] }};
  window.__isolationProbe = P;
  document.addEventListener('securitypolicyviolation', (e) => {{
    P.violations.push({{ directive: e.effectiveDirective || e.violatedDirective, blocked: String(e.blockedURI) }});
  }});
  const rec = (name, pass, detail) => {{ P.checks[name] = {{ pass: !!pass, detail: String(detail) }}; }};
  const msg = (e) => String(e && e.message ? e.message : e).slice(0, 240);
  const withTimeout = (p, ms) => Promise.race([p, new Promise((_, rej) => setTimeout(() => rej(new Error('probe-timeout')), ms))]);
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const violated = (directive) => P.violations.some((v) => v.directive === directive || v.directive.startsWith(directive + '-'));
  const expectRejected = async (name, cmd, args) => {{
    const ipc = window.__TAURI_INTERNALS__;
    if (!ipc || typeof ipc.invoke !== 'function') {{ rec(name, false, 'no IPC bridge — nothing was actually tried'); return; }}
    try {{ await withTimeout(ipc.invoke(cmd, args || {{}}), 5000); rec(name, false, 'resolved — the ACL let it through'); }}
    catch (e) {{ const m = msg(e); rec(name, /not allowed/i.test(m), m); }}
  }};
  (async () => {{
    try {{
      rec('page_origin', true, location.origin);
      rec('no_tauri_global', typeof window.__TAURI__ === 'undefined', typeof window.__TAURI__);
      // Tauri injects the IPC bridge into every webview; the rejections below only
      // mean something if a call was really made through it.
      const bridge = window.__TAURI_INTERNALS__;
      rec('ipc_bridge_present', !!bridge && typeof bridge.invoke === 'function', typeof (bridge && bridge.invoke));
      if ({trusted}) {{
        try {{ await withTimeout(window.__TAURI_INTERNALS__.invoke('vault_status'), 5000); rec('invoke_general_allowed', true, 'resolved'); }}
        catch (e) {{ rec('invoke_general_allowed', false, msg(e)); }}
      }} else {{
        await expectRejected('invoke_general_rejected', 'vault_status');
        await expectRejected('invoke_destructive_rejected', 'delete_vault');
        await expectRejected('invoke_core_event_rejected', 'plugin:event|listen', {{ event: 'probe', target: {{ kind: 'Any' }}, handler: 0 }});
        await expectRejected('invoke_core_window_rejected', 'plugin:window|set_theme', {{ value: null }});
      }}

      window.__probeInline = false;
      const s = document.createElement('script');
      s.textContent = 'window.__probeInline = true;';
      document.head.appendChild(s);
      await sleep(100);
      rec('inline_script_blocked', window.__probeInline === false && violated('script-src'), 'ran=' + window.__probeInline);

      let evalOutcome;
      try {{ (0, eval)('1 + 1'); evalOutcome = 'ran'; }} catch (e) {{ evalOutcome = e && e.name; }}
      rec('eval_blocked', evalOutcome === 'EvalError', evalOutcome);

      const d = document.createElement('div');
      d.setAttribute('style', 'color: rgb(1, 2, 3)');
      document.body.appendChild(d);
      const color = getComputedStyle(d).color;
      d.remove();
      rec('inline_style_applies', color === 'rgb(1, 2, 3)', color);

      let fetchOutcome;
      try {{ await withTimeout(fetch('{REMOTE}/probe-fetch', {{ mode: 'no-cors', cache: 'no-store' }}), 8000); fetchOutcome = 'fetched'; }}
      catch (e) {{ fetchOutcome = msg(e); }}
      await sleep(100);
      rec('remote_fetch_blocked_by_csp', fetchOutcome !== 'fetched' && violated('connect-src'), fetchOutcome);

      let opened;
      try {{ opened = window.open('{REMOTE}/probe-window'); }} catch (e) {{ opened = null; }}
      rec('window_open_denied', opened === null || opened === undefined, String(opened));
    }} catch (e) {{
      rec('probe_error', false, msg(e));
    }}
    P.done = true;
  }})();
  return 'started';
}})()"#
    )
}

/// Probe one window; returns `(checks, violations)`.
fn probe_window<R: Runtime>(
    app: &AppHandle<R>,
    label: &str,
    trusted: bool,
) -> Result<Value, String> {
    let window = window_or_err(app, label)?;
    poll(
        &window,
        "document.readyState",
        Duration::from_secs(30),
        |v| v.as_str() == Some("complete"),
    )?;
    eval(&window, &page_script(trusted), Duration::from_secs(5))?;
    let result = poll(
        &window,
        "JSON.stringify(window.__isolationProbe || null)",
        Duration::from_secs(40),
        |v| {
            v.as_str()
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
                .is_some_and(|p| p["done"] == json!(true))
        },
    )?;
    let mut probe: Value =
        serde_json::from_str(result.as_str().unwrap_or("null")).map_err(|e| e.to_string())?;

    // The page's origin is the FIRST load, which plugin navigation hooks never see.
    let origin = probe["checks"]["page_origin"]["detail"]
        .as_str()
        .unwrap_or("")
        .to_owned();
    probe["checks"]["page_origin"] = json!({
        "pass": origin == app_origin(),
        "detail": format!("{origin} (expected {})", app_origin()),
    });

    // Last, because a failure would leave the page: try to navigate away. Passing
    // needs BOTH the guard's own record of cancelling it for this window (positive
    // evidence — "the page did not move" alone also holds when the remote simply
    // fails to load) AND the page still on the app origin.
    let blocked_before = probe_evidence::blocked_count(label);
    let _ = eval(
        &window,
        &format!("location.href = '{REMOTE}/probe-navigation'; 'navigating'"),
        Duration::from_secs(5),
    );
    std::thread::sleep(Duration::from_millis(1500));
    let after = eval(&window, "location.origin", Duration::from_secs(5))
        .map(|v| v.as_str().unwrap_or("").to_owned())
        .unwrap_or_else(|e| format!("<{e}>"));
    let blocked_by_guard = probe_evidence::blocked_count(label) - blocked_before;
    probe["checks"]["remote_navigation_blocked"] = json!({
        "pass": blocked_by_guard >= 1 && after == app_origin(),
        "detail": format!("guard cancelled {blocked_by_guard} navigation(s); origin after: {after}"),
    });
    Ok(probe)
}

fn run<R: Runtime>(app: &AppHandle<R>) -> Value {
    let mut windows_report = serde_json::Map::new();
    let mut passed = true;

    for surface in IsolatedSurface::ALL {
        if let Err(err) = windows::open_isolated_shell(app, surface) {
            passed = false;
            windows_report.insert(surface.label().into(), json!({ "error": err.to_string() }));
        }
    }
    let targets = std::iter::once((MAIN, true))
        .chain(IsolatedSurface::ALL.iter().map(|s| (s.label(), false)));
    for (label, trusted) in targets {
        let entry = match probe_window(app, label, trusted) {
            Ok(probe) => {
                let ok = probe["checks"]
                    .as_object()
                    .is_some_and(|checks| checks.values().all(|c| c["pass"] == json!(true)));
                passed &= ok;
                json!({ "trusted": trusted, "passed": ok, "checks": probe["checks"], "violations": probe["violations"] })
            }
            Err(err) => {
                passed = false;
                json!({ "trusted": trusted, "passed": false, "error": err })
            }
        };
        windows_report.insert(label.into(), entry);
    }

    // window.open must not have created anything, anywhere.
    let mut labels: Vec<String> = app.webview_windows().into_keys().collect();
    labels.sort();
    let topology_ok = labels == ["agent_report", "document_preview", "main"];
    passed &= topology_ok;

    json!({
        "probe": "personal-cfo-0hp6 runtime isolation probe",
        "platform": std::env::consts::OS,
        "app_origin": app_origin(),
        "passed": passed,
        "windows_after_probe": { "pass": topology_ok, "labels": labels },
        "windows": windows_report,
    })
}
