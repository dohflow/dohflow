# ADR 0010: Tauri window + capability isolation model

- **Status:** Accepted — amended 2026-07-04 (see [Addendum](#addendum-2026-07-04-app-commands-are-capability-gated-after-all)); amended 2026-09-06 (see [Addendum: scoped opener grant for the About card](#addendum-2026-09-06-scoped-opener-grant-for-the-about-card)); amended 2026-09-08 (see [Addendum: updater + process grants for the signed release updater](#addendum-2026-09-08-updater--process-grants-for-the-signed-release-updater))
- **Date:** 2026-06-20
- **Deciders:** Project owner
- **Bead:** [`personal-cfo-tif`](../../.beads/issues.jsonl)
- **Related plan sections:** §2.3, §2.6, §6.2, §6.6
- **Supersedes:** None

## Context

ADR 0001 (Tauri + Rust + React) and ADR 0003 (trust boundary) both place the
security boundary *inside* the desktop app and both name **strict CSP** and
**Tauri capability isolation** as the controls that keep the untrusted WebView
from navigating to hostile origins, exposing the Tauri global, or reaching
commands it has no business calling. ADR 0003 also names a separate, IPC-write-less
WebView for untrusted document/agent rendering.

Those controls are currently **asserted but not configured.** As of this ADR:

- `apps/desktop/src-tauri/tauri.conf.json` → `app.security.csp` is **`null`** (no CSP).
- The only capability file is `capabilities/default.json`, granting **`core:default`**
  to the `main` window — a blanket grant, not a least-privilege command allowlist.
- There is a single `main` window; the isolated `document_preview` / `agent_report`
  surfaces (beads `personal-cfo-2no`, `-vrng`, `-z6pn`) do not exist yet.

This ADR defines the **target** window + capability + CSP model so the controls
the other ADRs depend on are specified, enforceable, and tracked. It is a
**policy** ADR; the configuration work is tracked by the implementation beads in
"Linked beads" and is part of the Week-8 real-data safety gate (`personal-cfo-rtez`).

## Decision

### Window model (three trust classes)

| Window | Trust | IPC capability | Renders |
|---|---|---|---|
| `main` | Trusted app UI | The enumerated Finance-Kernel command set | Owned React UI only |
| `document_preview` | **Untrusted** (imported docs) | **None** (no financial commands) | Sanitized document text in an isolated surface (`personal-cfo-vrng`) |
| `agent_report` | **Untrusted** (LLM output) | **None** | Typed structured blocks only — never arbitrary HTML/Markdown (`personal-cfo-z6pn`) |

Each window has its **own capability file**. Untrusted windows are display-only
sandboxes for hostile input (ADR 0003 "hostile input handling") with no access to
the app's command surface.

### Capabilities: how isolation actually works in Tauri v2

This corrects an earlier framing. **`core:default` is not a blanket "allow
everything" grant** — it is Tauri's *curated baseline* permission set (the default
permissions of the core modules: app, event, path, webview, window, …). It
deliberately **excludes** the dangerous capability plugins (filesystem, shell,
http), which are opt-in. And our **custom Finance-Kernel commands are not gated by
the capability system at all** in Tauri v2 — they are reachable by any window whose
WebView runs the `invoke` handler. *(This claim is wrong as an absolute — it holds
only while the app defines no app-level permissions. Superseded by the
[Addendum](#addendum-2026-07-04-app-commands-are-capability-gated-after-all).)*
Therefore:

- The **trusted `main` window keeps `core:default`.** It is the appropriate,
  least-surprising baseline for the one window that legitimately drives the app;
  hand-enumerating `core:*` permissions would be churn with no security gain, since
  `core:default` already excludes the dangerous plugins.
- **Per-window least privilege is achieved by window separation, not per-command
  capability lists.** Untrusted windows (`document_preview`, `agent_report`) get a
  **minimal/empty** capability file and **do not load the app's `invoke` handler**,
  so they structurally cannot reach Finance-Kernel commands.
- Adding a dangerous capability plugin (fs/shell/http) to **any** window requires
  explicit justification in review; the default is "don't".

### Content Security Policy (strict in production; relaxed only for dev HMR)

Tauri injects two policies (`tauri.conf.json` → `app.security`): **`csp`** on the
built app, and **`devCsp`** during `tauri dev`. We set both.

**Production `csp`** (the security-relevant one):

- `default-src 'self'` — no remote origins.
- `script-src 'self'` — **no `unsafe-inline` / `unsafe-eval`.** The critical
  lockdown: no injected or `eval`'d JavaScript. Tauri rewrites the bundle's own
  scripts with per-asset hashes under `'self'`, so the app still loads.
- `style-src 'self' 'unsafe-inline'` — a pragmatic allowance: Radix/Tailwind set
  inline `style` attributes at runtime (positioning, etc.). Style injection is not
  a code-execution vector; tightening to a nonce/hash is a follow-up.
- `connect-src 'self' ipc: http://ipc.localhost` — IPC only, no network egress.
- `img-src 'self' data:`; `font-src 'self'` — fonts are self-hosted
  (`@fontsource-variable`, no CDN), honoring the no-network rule.
- `object-src 'none'`; `frame-src 'none'`; `base-uri 'self'`; `form-action 'none'`.

**Development `devCsp`** relaxes `script-src` (`'unsafe-inline' 'unsafe-eval'`) and
`connect-src` (the Vite dev server + HMR websocket) so hot-reload works. It applies
only to `tauri dev` against the local trusted dev server, never to a built artifact.

The Tauri global is **not** exposed to the frontend (`withGlobalTauri` stays off);
the frontend reaches Rust only through the generated typed bindings (ADR 0003).

### External / OAuth flows

OAuth and aggregator "Link" flows open in the **system browser**, never in any
in-app WebView (ADR 0003 §2.3). No in-app window is granted remote-navigation.

### Enforcement

1. **CI fails if `app.security.csp` is null** — the policy cannot silently regress
   to "no CSP" (the gap this ADR closes).
2. When the untrusted windows land, a test asserts an **untrusted window cannot
   invoke a Finance-Kernel command** (it loads no `invoke` handler), complementing
   the IPC/CSP/redaction tests in `personal-cfo-1z4o` and `personal-cfo-zobt`.

## Consequences

### Positive

- A compromised or buggy WebView cannot navigate out, pull remote code, expose the
  Tauri global, or `eval` injected script.
- Untrusted document/agent content renders in a separate window that loads no
  command handler — no write path to financial state — so the ADR 0003 boundary
  becomes structurally true, not aspirational.
- The `csp` non-null CI guard means the WebView can never silently regress to "no CSP".

### Negative

- A strict `script-src 'self'` can break libraries that assume inline scripts/`eval`
  or CDNs; mitigated by self-hosting assets, the dev `devCsp`, and Tauri's per-asset
  hashing. Any future inline bootstrap must use a hash/nonce, not `unsafe-inline`.
- The `style-src 'unsafe-inline'` allowance is a deliberate, documented relaxation
  (Radix/Tailwind runtime styles); tightening it is a follow-up.
- The isolated untrusted windows add build/config complexity (deferred work).

## Rejected alternatives

- **Single window for everything (incl. untrusted content).** ✗ No isolation for
  hostile document/agent rendering — the exact risk ADR 0001/0003 call out.
- **Hand-enumerate `core:*` permissions on the `main` window.** ✗ Churn with no
  security gain — `core:default` is a curated baseline that already excludes the
  dangerous plugins, and custom commands are not capability-gated anyway.
- **Leave `csp` null.** ✗ A null CSP is the single largest WebView risk — no
  script-origin control at all. (A *relaxed `devCsp`* for HMR is fine: it applies
  only to the trusted local dev server, never a built artifact.)

## Revisit if

- A new feature needs an additional window trust class (extend the table, don't
  weaken existing grants).
- A required library genuinely cannot run under the CSP (prefer self-hosting /
  patching over relaxing `script-src`).

## Implementation notes

- **`csp: null` is the open gap** this ADR closes. Sequence: set a strict `csp` +
  a dev `devCsp` (`personal-cfo-2rf`) with a CI guard that `csp` is non-null; the
  `main` window keeps `core:default`; then add the isolated windows
  (`-2no`/`-vrng`/`-z6pn`) as those features land. The CSP is the first, cheapest
  step and lands within the Week-8 safety-gate window.
- Human-readable boundary notes live alongside `docs/architecture/trust-boundaries.md`
  (ADR 0003 implementation note).

## Addendum (2026-07-04): app commands ARE capability-gated after all

**Bead:** `personal-cfo-3fdd.6`. **Verified against the pinned versions in
`apps/desktop/src-tauri/Cargo.lock`:** `tauri 2.11.2`, `tauri-build 2.6.2`,
`tauri-utils 2.9.2` — by reading the vendored crate sources, not the docs.

### What this ADR got wrong

The Decision section claims custom Finance-Kernel commands "are not gated by the
capability system at all in Tauri v2". That is only the **default** behavior. The
accurate statement is:

- Application (non-plugin) commands bypass the ACL **only while the app defines no
  app-level permissions** (`RuntimeAuthority::has_app_manifest()` is false).
- The moment the app ACL manifest exists, **every** custom command invocation must
  resolve to a capability grant for the calling window/webview or it is rejected
  with `Command <name> not allowed by ACL` — deny-by-default for the *entire*
  custom command surface, local origin included.

Source of truth (vendored sources): `tauri-2.11.2/src/webview/mod.rs`, the
`(plugin_command.is_some() || has_app_acl_manifest || !is_local) && invoke.acl.is_none()`
rejection in `on_message`; `tauri-build-2.6.2/src/acl.rs` (`app_manifest_permissions`
+ the `has_app_manifest` check that registers the `__app-acl__` manifest);
`tauri-utils-2.9.2/src/acl/resolved.rs` (app command ACL keys are the raw
snake_case command names).

### How the gate is wired (config-only — no `build.rs` change)

The default `tauri_build::build()` already globs **`src-tauri/permissions/**/*`**
for app-defined permission files, so the opt-in is pure config:

1. `permissions/app-commands.toml` — permission `allow-app-commands`: the
   non-destructive command surface (kept in `src/lib.rs` `collect_commands!` order).
2. `permissions/destructive-commands.toml` — permission
   `allow-destructive-commands`: `delete_vault`, `export_backup`, `restore_backup`,
   `apply_update`, `relaunch_app` (data destruction, exfiltration, binary swap).
3. `capabilities/default.json` — main window: `core:default`, scoped dialog perms,
   `allow-app-commands`. App permissions are referenced **unprefixed**; identifiers
   are kebab-case (ACL identifier charset is alphanumeric + hyphen).
4. `capabilities/destructive.json` — main window: `allow-destructive-commands`,
   as a **separate named capability** so the future isolated windows
   (`document_preview`/`agent_report`, beads `-2no`/`-vrng`/`-z6pn`) can be granted
   the plain surface — or nothing — but never the destructive set.

Today both capabilities target `main` (the only window), so runtime behavior is
unchanged; the value is (a) the deny-by-default posture — a renderer compromise
can only call enumerated commands — and (b) the pre-cut seam for window isolation.

### What stays true from the original decision

Window separation (untrusted windows load no `invoke` handler) remains the primary
isolation mechanism, and `core:default` remains the right baseline for `main`. The
ACL gate is a **second, declarative layer** on top — defense in depth, not a
replacement.

### Maintenance rule (the one real cost)

Every command added to `collect_commands![...]` in `src/lib.rs` **must** be added
to exactly one of the two permission files, or it compiles fine and then fails at
runtime with `not allowed by ACL`. Build-time validation only covers
capability→permission references, not command-name spelling. Follow-up candidate:
a CI guard diffing the `collect_commands!` list against the union of
`permissions/*.toml` (would have to parse both; cheap script).

## Addendum (2026-09-06): scoped opener grant for the About card

**Bead:** `personal-cfo-n76x.18`. **Verified against the pinned versions in
`apps/desktop/src-tauri/Cargo.lock`:** `tauri 2.11.2`, `tauri-plugin-opener 2.5.5`
(npm `@tauri-apps/plugin-opener 2.5.5`) — by reading the crate's shipped
`permissions/` files and `src/commands.rs`, not the docs.

### What changes

Until now the app shipped **no** opener or shell capability: nothing in the
WebView could hand a URL to the operating system. The Settings **About card**
(website, help, release notes, report-a-bug, security policy, license) and the
**Support DohFlow** row + sidebar heart need exactly one thing — open a page on
the project's own site in the **system browser** — and this addendum grants
exactly that, nothing wider:

1. **`tauri-plugin-opener` is added** (pinned exactly, `=2.5.5`, and registered
   in `src/lib.rs` as
   `tauri_plugin_opener::Builder::new().open_js_links_on_click(false).build()` —
   **not** the plugin's `init()` default). That default switches on an
   anchor-click interceptor: an init script injected into *every* window that
   catches clicks on `target="_blank"` (or modifier-clicked) `http:`/`https:`/
   `mailto:`/`tel:` anchors and sends them straight to `open_url`, bypassing the
   frontend helper below. It is disabled, so the plugin injects no script into
   any window — the future isolated ones included — and the helper is the only
   WebView-side path to the command.
2. **The trusted `main` window is granted `opener:allow-open-url` with a scope
   allowing only URLs matching `https://dohflow.app/*`** — in
   `capabilities/default.json`, as the object form
   `{"identifier": "opener:allow-open-url", "allow": [{"url": "https://dohflow.app/*"}]}`.
   The scope entry shape (`url` + optional `app`, glob-matched) is the plugin's
   own `OpenerScopeEntry`; the `open_url` command checks the merged
   command+global scope and returns `ForbiddenUrl` for anything else, so the
   grant is enforced in Rust regardless of what the WebView asks for.
3. **NOT granted, and never to be:** `opener:default` (which bundles
   `allow-default-urls` — `mailto:`/`tel:`/`http:`/`https:` to *any* host — and
   `allow-reveal-item-in-dir`), `opener:allow-open-path`,
   `opener:allow-reveal-item-in-dir`, and the `shell`, `fs`, and `http` plugins.
   The isolated windows (`document_preview`/`agent_report`) get no opener grant
   at all when they land.
4. **The CSP is unchanged.** Opening a page in the system browser needs nothing
   from the WebView's policy; the site origin does not enter `connect-src`,
   `frame-src`, or anything else.

### Why only `https://dohflow.app/`

Shipped binaries outlive URLs. A repo host, a sponsor platform, or a payment
processor baked into a released app cannot be changed after the fact; a page on
the project's own domain can redirect wherever those things live this year. So
the allow-list is the `https://dohflow.app/` prefix and **nothing else** — the
About card links, the Support row (`/sponsor`), and the bug-report shortcut
(`/contribute?version=…&channel=…&commit=…`, carrying the build identity only —
never vault data) all resolve there. Placement is persistent and never
interruptive: no nags, no prompts, no check of whether anyone gave anything.

### Defense in depth on the WebView side

Every outbound link goes through one helper, `apps/desktop/src/lib/openExternal.ts`,
which refuses any URL not starting with `https://dohflow.app/` **before** the IPC
call (unit-tested against look-alike hosts, plain `http:`, and non-web schemes).
With the plugin's anchor-click interceptor off (item 1 above), nothing else in
the WebView reaches `open_url`. The links render as `<button>`s, not `<a href>`s,
so the WebView is never handed a navigation to perform, and a failed launch is
reported in the WebView console rather than dropped. Neither layer is the
security control on its own — the
capability scope is — but a wrong constant fails loudly in tests instead of
silently reaching Rust.

### The drift test that pins it

`apps/desktop/src-tauri/tests/acl_coverage.rs`:

- `the_main_window_opener_grant_is_exactly_open_url_scoped_to_dohflow` — across
  **both** capability files (`default.json` and `destructive.json` target the
  same `main` window, and Tauri merges scopes per window, so a second `opener:*`
  entry in either file would silently widen the grant) there is exactly one
  `opener:*` permission, it lives in `default.json`, it is
  `opener:allow-open-url`, its `allow` is the single entry
  `{"url": "https://dohflow.app/*"}`, and there is no `deny` list.
- `the_opener_plugin_injects_no_click_interceptor` — `src/lib.rs` registers the
  plugin through `Builder::new().open_js_links_on_click(false)`, never through
  `init()`, so no anchor-click script is injected into any window.
- `no_capability_grants_opener_default_shell_fs_or_http` — neither capability
  file grants `opener:default`, `opener:allow-default-urls`,
  `opener:allow-open-path`, `opener:allow-reveal-item-in-dir`, or any
  `shell:` / `fs:` / `http:` permission.
- `the_opener_grant_did_not_loosen_the_csp` — `app.security.csp` still carries
  `default-src 'self'`, the IPC-only `connect-src`, and `frame-src 'none'`, and
  does not mention the site origin.

`tauri-build` validates the capability → permission references at compile time,
so a typo in the identifier or scope shape fails the desktop Rust build; the
tests above catch the *semantic* drift (a widened scope, an extra grant) that a
successful build would accept.

### Maintenance rule

A new in-app link must be a page on `https://dohflow.app/` added to
`DOHFLOW_LINKS` in `openExternal.ts`. Anything that needs a different origin, a
local path, or reveal-in-Finder is a new decision: extend this ADR first, then
widen the grant and the drift test together.

## Addendum (2026-09-08): updater + process grants for the signed release updater

**Bead:** `personal-cfo-867.1.2` (ADR 0068 decides the distribution/update channel;
this addendum decides only the capability grants that channel needs).
**Verified against the pinned versions in `apps/desktop/src-tauri/Cargo.lock`:**
`tauri 2.11.2`, `tauri-plugin-updater 2.11.0` (npm `@tauri-apps/plugin-updater 2.11.0`,
pinned in lockstep), `tauri-plugin-process 2.3.1` (npm `@tauri-apps/plugin-process`).

### What changes

Release builds stop updating themselves by rebuilding from source and start
installing signed artifacts fetched from GitHub Releases (ADR 0068). That needs
exactly two new grants, and nothing wider:

1. **`updater:default` in `capabilities/default.json`** (the general capability,
   not the destructive one). It covers the plugin's `check` and
   `download_and_install` commands. Checking for an update is not a destructive
   act — it reads a manifest — so it belongs beside the other everyday grants.
   The *endpoint* it may contact is not part of the capability at all: it is
   pinned in `tauri.conf.json`'s `plugins.updater.endpoints` to exactly one URL,
   so a grant of `updater:default` cannot be pointed anywhere else from the
   renderer.
2. **`process:allow-restart` in `capabilities/destructive.json`** — narrowly, not
   `process:default`. Restarting the app is the step that swaps the running
   binary for the one just installed, which puts it in the same class as
   `apply_update`/`relaunch_app` (the from-source updater's own commands) that
   this capability already gates. `process:default` additionally grants `exit`
   and the other restart variants the app has no use for; granting the single
   command keeps the isolated windows' future story simple — they get neither.
3. **NOT granted, and never to be:** `process:default`, and (unchanged from the
   2026-09-06 addendum) the `shell`, `fs`, and `http` plugins. The updater
   performs its own HTTPS fetch from **inside the plugin, in Rust** — it carries
   its own client rather than borrowing the `http` capability — so no
   renderer-reachable network capability is added by this change.
4. **The CSP is unchanged.** The fetch and the minisign signature verification
   both happen Rust-side; the WebView's `connect-src 'self' ipc:
   http://ipc.localhost` does not gain the GitHub origin. See ADR 0003's
   2026-09-08 addendum for the trust-boundary reasoning.

### What pins it

`apps/desktop/src-tauri/tests/acl_coverage.rs`, in the same style as the opener
tests from the 2026-09-06 addendum:

- `the_updater_grant_is_exactly_updater_default_in_the_general_capability` —
  exactly one `updater:*` permission across both capability files, and it is
  `updater:default` in `default.json`.
- `the_process_grant_is_exactly_allow_restart_in_the_destructive_capability` —
  exactly one `process:*` permission, `process:allow-restart`, in
  `destructive.json`; `process:default` is asserted absent everywhere.
- `the_updater_pubkey_and_endpoint_are_exactly_the_committed_values` — the
  committed public key, the single endpoint, and `createUpdaterArtifacts: true`.

The pre-existing `no_capability_grants_opener_default_shell_fs_or_http` and
`the_opener_grant_did_not_loosen_the_csp` continue to hold unmodified, which is
the evidence that this change adds no network or filesystem surface.

## Linked beads

- `personal-cfo-n76x.18` (About card, Support row, scoped opener grant — 2026-09-06 addendum)
- `personal-cfo-867.1.2` (updater + process capability grants — 2026-09-08 addendum)
- `personal-cfo-tif` (this ADR's implementation: window/capability isolation)
- `personal-cfo-2rf` (strict CSP + capability isolation)
- `personal-cfo-2no` (window/webview model: main / document_preview / agent_report)
- `personal-cfo-vrng` (isolated `document_preview` WebView)
- `personal-cfo-z6pn` (isolated `agent_report` WebView)
- `personal-cfo-1z4o` (security tests: IPC permissions, CSP, logging redaction)
- `personal-cfo-zobt` (logging redaction CI test)
- `personal-cfo-40t` (typed IPC command pattern)
- `personal-cfo-1al` (ADR 0003: trust boundary)
- `personal-cfo-v4l` (ADR 0022: parser/document isolation boundary)
