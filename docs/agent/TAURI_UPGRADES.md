# Tauri upgrades

**Decision:** ADR 0001 (pin Tauri v2 minor versions and treat each upgrade as its
own PR), carried out by `personal-cfo-io42`. **Why it matters:** ADR 0010's
addenda, the dated capability audit and the regression suite
([tauri-regression-suite.md](../security/tauri-regression-suite.md)) were
verified against the exact Tauri sources pinned today. A Tauri release, even a
patch, can change what those checks rely on, and some of those changes cannot be
seen in any file in this repository.

## What is pinned, and how drift is caught

| Where | Pin | Enforced by |
|---|---|---|
| `apps/desktop/src-tauri/Cargo.toml` | Every `tauri`, `tauri-build` and `tauri-plugin-*` spec is `~X.Y.Z` (the minor) or `=X.Y.Z` | `tests/tauri_pins.rs` rule `manifest-range` |
| `apps/desktop/package.json` | Every `@tauri-apps/*` spec is `~X.Y.Z` or exact | `manifest-range` |
| `Cargo.lock`, `pnpm-lock.yaml` | Exact resolved versions of the whole family (Rust `tauri*`/`wry`/`tao`, npm `@tauri-apps/*`) must equal `apps/desktop/src-tauri/tauri-pins.toml` | `drift`, even for a patch |
| Rust ⇄ npm | Each Rust plugin and its npm package share a major.minor; `tauri`, `@tauri-apps/cli` and `@tauri-apps/api` share one; every CLI platform binary equals the CLI | `lockstep` |
| CI | The desktop job runs on changes to `apps/desktop/src-tauri/**`, `apps/desktop/package.json` and `pnpm-lock.yaml` | `ci_runs_the_pin_check_on_every_change_that_can_move_tauri` |
| Dependabot | A `tauri` group listed first in both ecosystems, so a Tauri bump never rides along in a generic update PR | `dependabot_keeps_tauri_bumps_out_of_generic_update_prs` |

A `cargo update`, a `pnpm update`, or a Dependabot PR that moves any Tauri
package therefore fails CI until someone deliberately edits `tauri-pins.toml`,
and that edit belongs only in an upgrade PR.

## The upgrade PR

**One PR, nothing else in it.** An upgrade PR moves the Tauri family, both
halves together, and only makes the code changes the upgrade forces. It does not
also add a feature, grant or dependency. Open it from its own bead, on its own
branch (`agent/<bead>-tauri-<version>`), with the upgrade template:

```bash
gh pr create --body-file .github/PULL_REQUEST_TEMPLATE/tauri-upgrade.md ...
```

or, in the browser, add `?template=tauri-upgrade.md` to the compare URL.

### Steps

1. **Record the before state.** On `main`, build the desktop crate, then save the
   expanded grants:
   ```bash
   (cd apps/desktop/src-tauri && cargo build)
   node scripts/tauri-acl-expansion.mjs > acl-before.txt
   ```
2. **Move both halves together.** Update the Rust specs in `Cargo.toml` and the npm
   specs in `apps/desktop/package.json` to the new `~X.Y.Z`. Then
   `cargo update -p <crate> --precise <version>` for each directly-pinned crate,
   and `pnpm install`. Keep each Rust plugin and its npm package on the same minor.
   This is the one step that uses the network; name it in the PR.
3. **Update `tauri-pins.toml` by hand** to the new resolved versions. Every line
   that changes is part of the "what changed" table.
4. **Record the after state and diff it:**
   ```bash
   (cd apps/desktop/src-tauri && cargo build)
   node scripts/tauri-acl-expansion.mjs > acl-after.txt
   diff acl-before.txt acl-after.txt
   ```
   A grant that now allows more (for example `core:default` gaining a command) is
   a policy change. It needs an ADR 0010 addendum before merge, and may need the
   baseline or an invariant updated in the same PR.
5. **Re-verify the vendored-source claims.** Read each item in the checklist
   below in the new crate sources, and say in the PR whether it still holds.
6. **Run the full gates**, the regression suite and the probe (the template lists
   them). CI must be green, including "Runtime isolation probe (real WebView,
   ADR 0010)". Also run the probe locally on macOS: CI runs it on WebKitGTK, but
   the shipped engine is WKWebView.
7. **Build and smoke the release artifact:** `pnpm tauri build`, then the launch
   smoke, then an update check against the signed updater (ADR 0068).
8. **Review, then re-audit.** Independent review binds to the PR's SHA. The next
   pre-release review re-audits the release SHA against ADR 0010, as the dated
   audit requires.

### Vendored-source checklist

Each of these is a claim the security posture rests on, verified at Tauri 2.11.2
(file paths relative to each crate's source):

| # | Claim | Where to read it | Relied on by |
|---|---|---|---|
| 1 | With an app ACL manifest, `Webview::on_message` rejects any app/plugin/core command no capability grants to the calling window, before dispatch | `tauri/src/webview/mod.rs` `on_message` | ADR 0010 addenda 2026-07-04 and 2026-09-27 |
| 2 | One invoke handler serves every webview, and the IPC bridge is injected into every webview | `tauri/src/manager/webview.rs` `prepare_pending_webview` | addendum 2026-09-27 |
| 3 | The only ACL exception is `plugin:__TAURI_CHANNEL__\|fetch`, and its queue is app-wide under sequential ids | `tauri/src/ipc/channel.rs` | addendum 2026-09-27, rule 7 |
| 4 | Every registered plugin's `on_navigation` can veto a navigation, but the hooks do not see a webview's first load | `tauri/src/manager/webview.rs`, `tauri/src/plugin.rs` | `navigation_guard`, probe `page_origin` |
| 5 | The bundled origin is `tauri://localhost`, or `http(s)://tauri.localhost` on Windows/Android | `tauri/src/manager/mod.rs` `tauri_protocol_url` | `navigation_guard` |
| 6 | `is_dev()` (no `custom-protocol`) selects `devUrl` and `devCsp` | `tauri/src/lib.rs`, `tauri/src/manager/mod.rs` `csp()` | release/dev separation |
| 7 | The CSP is rewritten per asset: script hashes, and a style nonce only for `<style>` elements | `tauri/src/manager/mod.rs` `set_csp`, `tauri-codegen`, `tauri-utils/src/html.rs` | the accepted `style-src 'unsafe-inline'`, `scripts/check-dist-csp.mjs` |
| 8 | `core:default` contains no window/webview creation or navigation command; `image\|from_path`, `tray` and the devtools toggle are inert without their features | `scripts/tauri-acl-expansion.mjs` output; `tauri/src/image/plugin.rs`, `tauri/src/app.rs`, `tauri/src/webview/mod.rs` | audit §3 |
| 9 | `dynamic-acl` is still the only route to runtime grants (`Manager::add_capability`) | `tauri/src/lib.rs` | `app_code_never_grants_capabilities_at_runtime` |
| 10 | The opener matches its URL scope against the raw string, and refuses a `with` program unless scoped | `tauri-plugin-opener/src/scope.rs`, `src/commands.rs` | addendum 2026-09-06 |
| 11 | The updater verifies the minisign signature before install, and insecure transport stays opt-in | `tauri-plugin-updater/src/updater.rs`, `src/config.rs` | addendum 2026-09-08, ADR 0068 |
| 12 | Without a handler, wry creates no new window on macOS/Linux; `on_new_window` is still available on window builders | `wry/src/wkwebview/…`, `tauri/src/webview/webview_window.rs` | the new-window deny |
| 13 | `eval_with_callback`, the mock runtime (`tauri::test`), `generate_context!(test = true)` and `INVOKE_KEY` still exist | `tauri/src/webview/mod.rs`, `tauri/src/test/mod.rs` | the probe, `tests/window_isolation.rs` |

Anything that no longer holds is either fixed in the same PR, with the fix
reviewed, or blocks the upgrade. Never loosen a check to make an upgrade pass.
