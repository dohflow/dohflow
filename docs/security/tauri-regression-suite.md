# Tauri capability and CSP regression suite

**Bead:** `personal-cfo-0hp6` · **Policy:** ADR 0010 and all its addenda ·
**Latest audit:** [capability-audit-2026-09-27.md](capability-audit-2026-09-27.md)
(`personal-cfo-8ea`, audited SHA `04ec14d`)

This page is the map of how ADR 0010's desktop trust boundary is kept true after
it ships. For each rule it says which layer enforces it, where that runs in CI,
and what to do when the policy has to change on purpose. The audit is the dated
evidence for one commit; this suite is what keeps later commits honest.

## The four layers

| Layer | Where | What it proves | Runs |
|---|---|---|---|
| **1. Static pins** | `apps/desktop/src-tauri/tests/acl_coverage.rs` | Individual ADR rules on the committed files: every registered command has exactly one ACL grant; the opener, updater, process and theme grants are exactly what the addenda allow; no `shell`/`fs`/`http`; the untrusted shells are granted nothing; every window builder denies new windows; nothing uses `ipc::Channel` or `add_capability`; the debug-only fixtures are compiled out of release builds | `cargo test` (the desktop Rust gate) |
| **2. Drift audit** | `tests/support/capability_audit.rs` + the hand-edited baseline `expected-capabilities.toml` | What each window is *effectively* granted (the union of its capability files), scoped grants, the destructive command inventory, production and dev CSP and the `tauri` features, compared with the reviewed baseline. It also applies code invariants to both the configuration and the baseline (strict `script-src`, no egress, untrusted windows get nothing, destructive commands reach only `main`, no wildcard window, no devtools). Negative tests break one input at a time. | `cargo test` |
| **3. Mock-runtime ACL** | `tests/window_isolation.rs` | The real resolved ACL on Tauri's mock runtime, with windows built by the app's own `windows` module: `main` reaches the general and destructive surfaces; `document_preview` and `agent_report` are rejected for app, destructive, core and plugin commands; the navigation guard refuses remote, `file:` and `data:` navigation from each shell | `cargo test` |
| **4. Real-WebView probe** | `src/isolation_probe.rs` (debug builds only) | What static tests cannot: in each **real** WebView (`main` and both shells) the first-load origin, no Tauri global, `invoke` accepted or rejected as above, and the **effective** CSP, read from the browser's own `securitypolicyviolation` reports. An injected inline script and `eval` are refused, a remote `fetch` is refused by `connect-src`, and an inline `style` attribute still applies. `window.open` creates nothing, and a remote navigation is cancelled by the guard itself (its per-window count must rise) and leaves the page on the app origin. | CI step "Runtime isolation probe (real WebView, ADR 0010)" (Ubuntu, WebKitGTK under Xvfb); on demand locally (macOS WKWebView) |

Two build-output checks complete the picture:

- **`scripts/check-dist-csp.mjs`** runs on the frontend job's built `dist/`. It
  fails on a `<style>` element (Tauri would add a `style-src` nonce, and a nonce
  makes browsers *ignore* the accepted `'unsafe-inline'`), an inline script, or a
  remote script or stylesheet.
- **"Content Security Policy is configured (ADR 0010)"** is the original CI guard
  that the CSP is never null.

## ADR 0010 rules and the tests that hold them

| Rule | Layer(s) |
|---|---|
| Deny-by-default app ACL; every command in exactly one permission set | 1 `every_registered_command_has_exactly_one_acl_grant`, 2 `acl-coverage` |
| No untrusted Finance-Kernel or destructive IPC | 1 `the_untrusted_shells_are_granted_nothing`, 2 `untrusted-grant` / `destructive-reach`, 3 `untrusted_windows_cannot_invoke_*`, 4 `invoke_*_rejected` |
| No arbitrary remote navigation | 1 `the_navigation_guard_is_registered_for_every_webview`, 3 `untrusted_windows_cannot_navigate_to_a_remote_origin`, 4 `remote_navigation_blocked`, `page_origin` |
| No new windows or popups | 1 `every_window_builder_denies_new_windows`, 4 `window_open_denied` plus the post-probe window count |
| Tauri global disabled | 1 `the_tauri_global_is_not_exposed_to_the_frontend`, 2 `global-tauri`, 4 `no_tauri_global` |
| No inline or eval scripts | 2 `csp` / `csp-invariant`, 4 `inline_script_blocked` / `eval_blocked`, build check |
| No network egress from the WebView | 2 `csp-egress`, 4 `remote_fetch_blocked_by_csp` |
| Release devtools disabled | 1 `release_builds_cannot_enable_webview_devtools`, 2 `devtools` / `tauri-features` |
| **Exception:** `style-src 'unsafe-inline'` | 2 baseline + invariant (allowed only there), 4 `inline_style_applies`, build check (no `<style>`) |
| **Exception:** development HMR policy | 2 `dev-csp` / `dev-csp-invariant` / `dev-leak` |
| **Exception:** scoped opener, updater, process grants | 1 the opener/updater/process tests, 2 baseline `scopes` / `grants` |
| OAuth only in the system browser | No in-app auth window exists; any new window fails 2 `window-set` and 1 `every_window_builder_denies_new_windows` |

## Running it

```bash
# Layers 1–3 (also the CI desktop gate)
cd apps/desktop/src-tauri && cargo test

# Layer 4 on this machine: a debug build that serves the bundled origin, then the probe.
# Scratch data only; windows flash open and close; the report is JSON; exit 0 = pass.
cd apps/desktop && pnpm build && cd src-tauri
cargo build --features tauri/custom-protocol
PCFO_DATA_DIR="$(mktemp -d)" PCFO_ISOLATION_PROBE=/tmp/isolation-probe.json \
  ./target/debug/personal-cfo-desktop

# Build-output check
node scripts/check-dist-csp.mjs apps/desktop/dist
```

The probe refuses to run without `PCFO_DATA_DIR`, and refuses a dev-server build
(which would test `devCsp` and the dev origin instead of the shipped ones). No
remote host is contacted: every target is in the reserved `.invalid` domain, and
every check requires positive evidence of the refusal, so none can pass just
because a remote page failed to load:
- a `securitypolicyviolation` report, for the CSP checks;
- an ACL rejection message, for `invoke`;
- the navigation guard's own per-window count of cancelled navigations, for
  navigation.

No real financial fixtures are involved.

**What triggers CI.** The desktop job's path filter watches
`apps/desktop/src-tauri/**` (configuration, capabilities, permissions, command
registration in `src/lib.rs`, window construction in `src/windows.rs`, the
baseline, the probe and all the tests), `apps/desktop/public/isolated-shell.html`
and `.github/workflows/ci.yml`. The frontend job watches `apps/desktop/**` and
this suite's build check. A test (`ci_runs_the_*`) fails if either wiring is
removed.

## Changing the policy on purpose

A grant, scope, origin, window or CSP source is never widened by editing a test
until it passes. The order is:

1. **Decide.** Write a dated ADR 0010 addendum (or a new ADR) and get it
   Accepted. Anything that loosens a code invariant (script-src, egress, untrusted
   grants, destructive reach, wildcard windows, devtools) is an ADR decision
   first.
2. **Change the configuration** (`tauri.conf.json`, `capabilities/`,
   `permissions/`, `src/windows.rs`).
3. **Update the reviewed baseline by hand**
   (`apps/desktop/src-tauri/expected-capabilities.toml`), per its header. There
   is no regenerate command.
4. **Update the affected invariant or pin in the same PR**, and add a negative
   case for the new boundary.
5. **Review.** The independent review reruns the suite at the PR's SHA.
6. **Re-audit before release.** The dated audit approves one commit; a
   pre-release review audits the release SHA again.

A new *ordinary* command needs only `collect_commands!` plus
`permissions/app-commands.toml`. A new *destructive* command also goes in
`permissions/destructive-commands.toml` and in the baseline's destructive list.

## Known limits

- **CI runs the probe on WebKitGTK (Linux), not WKWebView (macOS).** The
  shipped engine is macOS, and the probe has passed there locally (see the 0hp6
  PR). A per-PR macOS run is not wired; post-merge macOS coverage would be a
  separate change to the `intel-smoke` job.
- **On macOS, `window.open` returns `null` even without the deny hook** (wry
  creates no window without a handler). So the probe's `window_open_denied`
  cannot, on its own, tell whether the hook is there. The static pin
  `every_window_builder_denies_new_windows` covers the hook, and on
  Windows/WebView2 (where it matters) the probe would.
- **Some pins are source scans:** the new-window hook, `Channel`,
  `add_capability`, and the fixtures' `cfg` gates. They fail loudly, but a
  determined rewrite could evade them. Review remains the backstop.
- **Plugin navigation hooks never see a webview's first load** (Tauri 2.11.2
  `manager/webview.rs`). The probe checks each window's first-load origin
  directly (`page_origin`).
