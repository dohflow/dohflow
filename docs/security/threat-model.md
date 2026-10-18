# Threat model v1

DohFlow is a **local-first, offline, single-user** desktop finance vault (a user
may keep several such vaults side by side — ADR 0025 — but each one remains a
single-user unit; see "Accepted risks and out of scope" below). This model is
asset-centric with a STRIDE-lite lens, and covers the shipped R1 surface
(`personal-cfo-agh`, Risk Register §24). It is a living document — extend it as
new surfaces land (import, documents, agents).

**Last substantive revision: 2026-09-22** (`personal-cfo-8qh`) — see
[Revision history](#revision-history) at the bottom of this document.

## What we are protecting (assets)

| | Asset | Property |
|---|-------|----------|
| **A1** | Financial data at rest — accounts, transactions, income, bills, balances, attachments | confidentiality + integrity |
| **A2** | The data-encryption key (DEK) — unwrapped into memory only while the vault is unlocked | confidentiality |
| **A3** | The master password — derives the key-encryption key (KEK) via Argon2id | confidentiality |
| **A4** | Encrypted backups — portable ciphertext; the v2 header carries the same wrapped vault envelope as the sidecar, allowing backups from one password/rekey epoch to be correlated | confidentiality + integrity, with accepted envelope linkability |
| **A5** | Logs / diagnostics — must never carry A1 | confidentiality |

## Trust boundaries

- **TB1 — Rust core ↔ WebView frontend** (ADR 0003). The Rust core is trusted; the
  WebView is treated as semi-trusted. All traffic crosses a typed IPC command surface
  (ADR 0006); no database types and **no key material** ever reach the frontend.
- **TB2 — unlocked memory ↔ disk at rest.** While unlocked the DEK lives in memory;
  at rest everything is SQLCipher-encrypted. Parameters:
  [`docs/security/encryption-design.md`](encryption-design.md).
- **TB3 — the app ↔ the outside world.** Four sanctioned egress paths exist
  (added since R1; see [Revision history](#revision-history)). Three of the
  four fire **automatically on every vault unlock** once in use, not only when
  asked (the LunchFlow path is compiled in but cannot be used yet — item 4);
  none carries telemetry or any identifier beyond what the transport
  inherently exposes, and none is reachable from the WebView as a
  general-purpose network capability:
  1. **SimpleFIN Bridge sync** (ADR 0060) — fires automatically on vault
     unlock (fire-and-forget so it never blocks the unlock; 6-hour debounce,
     `CONNECTOR_SYNC_DEBOUNCE_HOURS`), plus a manual refresh button. The
     user's own setup token, claimed client-side; no project credential, no
     relay. The HTTPS fetch runs Rust-side (`ureq` in
     `crates/connectors/simplefin-adapter`), never through a
     WebView-reachable `http:` capability — no such capability is granted.
  2. **The `opener:allow-open-url` grant** (ADR 0010 addendum 2026-09-06) —
     opens a page on `https://dohflow.app/*` only, in the system browser, on an
     explicit user click.
  3. **The signed-artifact updater** (ADR 0068) — the version check
     (`latest.json` GET) runs **automatically** on every launch/unlock
     (`UpdateAvailableNotice`, mounted unconditionally on the unlocked home
     screen) plus on demand from Settings; the fetch and minisign
     verification happen Rust-side (`tauri-plugin-updater`, not a
     renderer-reachable `http:` grant). Only the **download-and-install**
     step requires an explicit user click, and is minisign-verified first.
  4. **LunchFlow refresh** (ADR 0076, `personal-cfo-r2pow`) — the pinned
     Personal API base `https://www.lunchflow.app/api/v1` (`GET /accounts`,
     `/accounts/{id}/transactions`, `/accounts/{id}/balance`). Same triggers
     as SimpleFIN once enabled: automatically on vault unlock (same debounce)
     plus the manual refresh button. The user's own API key, sent only in an
     `x-api-key` header; no project credential, no relay. The HTTPS fetch runs
     Rust-side (`ureq` in `crates/connectors/lunchflow-adapter`, no redirects,
     HTTPS only), never through a WebView-reachable capability. **Registered
     disabled:** until its release flips the registry flag (ADR 0076 decision
     3), `connector_link` refuses it, so no LunchFlow connection can be stored
     and this path makes no request.
  Backup exports and imports are file operations, not app-managed network
  paths. A scheduled backup stays local unless the user chooses a
  cloud-synced folder; then the provider's own client carries the encrypted
  file off-device as ciphertext. Folder classification is local display-only,
  and no destination or cloud-folder usage is reported to DohFlow. See the
  threats table below for the ciphertext's metadata exposure.
- **TB4 — WebView ↔ OS** (ADR 0010). A strict Content Security Policy and a minimal
  Tauri capability set bound what the WebView can reach.

## Threats and mitigations (shipped)

| Threat | Vector | Mitigation | Ref |
|--------|--------|------------|-----|
| Disk theft / lost laptop reads A1 | the `.db` file on disk | full-database SQLCipher AES-256; the envelope wraps the DEK under the password-derived KEK | ADR 0002 |
| Plaintext leaks to side files | WAL / SHM / journal / OS temp | SQLCipher encrypts the side files; `PRAGMA temp_store=MEMORY`; a sentinel leak test scans every artifact incl. after an unclean drop | `zxvl` (`1bkb`) |
| DEK scraped from memory | process memory while unlocked | `SecretBytes` with `mlock` + zeroize-on-drop; the DEK is dropped (scrubbed) on lock | `1t0`, ADR 0002 |
| Brute-force the password / weak KDF | offline guessing against the envelope | Argon2id at **64 MiB / t=3** by default (exceeds the OWASP floor); a 256 MiB profile exists | ADR 0002 · [`encryption-design.md`](encryption-design.md) · *hardening: `0sqk`* |
| Financial data leaks into logs | tracing spans / diagnostics | a global redacting tracing layer scrubs §6.6 attributes app-wide; a release-blocking CI test asserts an exhaustive corpus is scrubbed; free text (merchant names, descriptions) is kept out by source discipline only | `2vs` / `zobt` · [logging-policy.md](logging-policy.md) |
| WebView XSS / malicious rendered content | a content-injection bug in the UI | strict CSP (every directive pinned to ADR 0010 by `acl_coverage.rs`, so `script-src` can never gain `'unsafe-inline'`/`'unsafe-eval'`; CI also fails on a null CSP), no remote content loaded, no Tauri global (`withGlobalTauri` off), no devtools in release builds, no secrets in the frontend, scoped capabilities with no remote origin admitted to IPC | ADR 0010 (`h7h8`) · `2rf` |
| Untrusted IPC input reaches the core | crafted command payloads | a sealed, typed command bus; Rust is authoritative for validation; raw `invoke` is lint-forbidden | ADR 0003 / 0006 |
| Backup theft | a copied backup file | the AES-256-GCM payload uses a random backup DEK wrapped under an HKDF-SHA256 KEK derived from the vault DEK; the plaintext v2 header carries the same wrapped vault envelope as the on-disk sidecar, but no financial data or usable key. A stolen backup is no more decryptable than a stolen vault directory: the vault password is still required. The repeated envelope is a stable fingerprint that permits correlating backups within a password/rekey epoch and matching them to the sidecar. Wrong password, tampering, or component-hash mismatch fails before any restore write. | ADR 0024 / addendum A (`personal-cfo-ii3an`) |
| Vault DEK exposed | a memory dump of the unlocked app, a leaked diagnostic, or another way the in-memory key escapes | **Settings → Rotate encryption key** gives the vault a fresh random DEK without changing the password: the database is re-encrypted into a copy prepared beside the live vault, every attachment is re-wrapped and renamed under the new key, and one journal rename commits; a crash at any point leaves the old vault or the new one, never a pair that does not open. **Residual risk (accepted, ADR 0083 §6):** backups made before the rotation still open with the password current when they were made, and copies of the vault taken before it stay readable with the old DEK; attachment contents keep their content keys, so someone who already holds the old DEK *and* an old copy of the database can still decrypt the attachments captured then; removed files are not securely erased (SSD / copy-on-write remanence). The card states this, and tells the user to delete old backups they no longer need. **Likelihood:** low — needs the unlocked key, which the accepted compromised-host risk below already covers. **Impact:** high until rotated. | ADR 0083 · `personal-cfo-2y8` · `crates/finance-kernel/src/rekey.rs` |
| Cloud provider sees a scheduled backup | the user's chosen cloud-sync client copies a `.pcfobk` file from its selected folder | the client carries ciphertext, not plaintext financial data; the app does not perform an upload or report the chosen folder. The v2 header's repeated vault envelope remains linkable within one password/rekey epoch and reveals the backup's association with the vault sidecar. The user chooses whether to put backups under that provider's control. | ADR 0024 addendum A · `personal-cfo-8qh` |
| Attachment plaintext at rest | blobs on disk | per-blob AES-256-GCM, content-addressed, keys wrapped under the DEK; a canary test proves no plaintext artifact | ADR 0023 (`bcj`) |
| Vulnerable third-party dependency | supply chain | `cargo audit` (both workspaces) + `pnpm audit` in CI | `7t7` |
| Secret committed by mistake | git history | gitleaks in CI with a tight allowlist | `6r6` |
| Silent data corruption | disk faults, interrupted writes | `PRAGMA integrity_check` + read-model rebuild repair + a verified backup/restore drill | `n9w` / `5ivp` / `7pfu` |
| Schema downgrade / tampered vault | an older/forked binary opening a vault | pre-writer independent version-marker gate and migration-hash checks; cooperative runner lock and opened-handle revalidation; up/down migration safety tests; pinned SQLCipher engine | `g3m.4` / `n9w` / `c545` / `7igv` |
| Connector credential or synced data intercepted/misused | the SimpleFIN Bridge egress path (`connector_link`/`connector_sync`/`connector_forget`, on vault-open + manual refresh) | user's own token, claimed client-side, no project secret, no relay; access URL stored as an encrypted `connector_connections.credential` column (SQLCipher, round-trips via backup/restore, never logged or on the listing row type); fetch is Rust-side (`ureq`), not WebView-reachable. **Likelihood:** low — needs a compromised SimpleFIN Bridge account or a fully-compromised host (already accepted below). **Impact:** medium — scoped to synced account/transaction data plus provider access, not the vault password or DEK. | ADR 0060 · `gglk` · `crates/connectors/simplefin-adapter/src/transport.rs` · `simplefin_drill.rs` |
| LunchFlow API key or synced data intercepted/misused | the LunchFlow egress path (`connector_link`/`connector_sync`/`connector_forget` against `https://www.lunchflow.app/api/v1`, on vault-open + manual refresh once enabled; refused while its registry entry is disabled) | user's own Personal API key, no project secret, no relay, never the OAuth Platform API; the key rides only in an `x-api-key` header (never a URL or query), is stored as the encrypted `connector_connections.credential` column exactly like the SimpleFIN access URL, and is never logged, on the listing row, or on any DTO (`lunchflow_flow.rs` leak test; `connector_ipc.rs` span test); no redirects are followed, so the header is never replayed to another host; revocation = forget locally + delete the API destination in the user's LunchFlow dashboard (the API has no revoke endpoint). **Likelihood:** low — needs a compromised LunchFlow account or a fully-compromised host. **Impact:** medium — scoped to synced account/transaction data plus LunchFlow access, not the vault password or DEK. | ADR 0076 · `r2pow` · `crates/connectors/lunchflow-adapter/src/transport.rs` · `lunchflow_drill.rs` |
| WebView navigates to an attacker-controlled or unexpected origin | the opener grant behind the About card / Support row "open in browser" buttons | scope is exactly `https://dohflow.app/*` (enforced in Rust regardless of WebView request); `opener:default`/`allow-open-path`/`allow-reveal-item-in-dir` and the `shell`/`fs`/`http` plugins are never granted; anchor-click interceptor explicitly disabled, so `openExternal.ts` is the only path to the command and it pre-filters the URL; 4 `acl_coverage.rs` tests pin the grant shape, the disabled interceptor, the absence of wider grants, and an unchanged CSP. **Likelihood:** low — a widened grant must survive review and 4 drift tests. **Impact:** low — worst case opens the project's own domain, not an arbitrary URL. | ADR 0010 addendum 2026-09-06 · `n76x.18` · `apps/desktop/src-tauri/tests/acl_coverage.rs` |
| Renderer replaces the trusted UI with a remote page | a compromised or buggy renderer setting `location.href`, following an `<a href>`, or submitting a form | a navigation-guard plugin cancels every webview navigation outside the bundled app origin (`tauri://localhost`, or `http(s)://tauri.localhost` on Windows/Android only, never with a port; the dev server only in `tauri dev` builds) — CSP governs what a page loads, not where it goes; blocked attempts log scheme + host only. **Likelihood:** low — requires a renderer compromise first. **Impact:** low — the navigation is cancelled and a remote page would reach no IPC anyway (no capability admits a remote origin). | ADR 0010 · `2rf` · `apps/desktop/src-tauri/src/navigation_guard.rs` |
| Untrusted document or agent content reaches the command surface | a hostile document or LLM output rendered in the `document_preview` / `agent_report` window | those windows are built in Rust with their own capability files, which grant **nothing** — the app ACL rejects every app, destructive, core and plugin command from their labels before dispatch (Tauri serves one invoke handler to every webview, so the capability, not a missing handler, is the boundary); `default`/`destructive` target `main` only; navigation away and `window.open` are denied for every webview; a mock-runtime test proves the ACL decisions against the committed files. **Likelihood:** low — a grant to an untrusted window must survive an ADR addendum and the drift tests. **Impact:** high if it regressed, which is why it is pinned. | ADR 0010 addendum 2026-09-27 · `2no` · `apps/desktop/src-tauri/tests/window_isolation.rs` |
| A tampered update manifest or artifact is fetched or installed | `latest.json` GET + signed archive download, on launch/unlock + on demand | HTTPS-only, single pinned endpoint not renderer-reachable (`updater:default` grants only `check`/`download_and_install`, not the target URL); minisign verification Rust-side before install; install only on explicit click, never silent; restart scoped to `process:allow-restart` only (no `process:default`). A **one-time manual verification drill** (owner-directed, 2026-09-08, against the throwaway `dohflow/updater-smoke` repo, exact refusal text recorded in `867.1.2`'s notes) confirmed refusal of both a mismatched signature and a tampered `.tar.gz` — this is **not** an automated or release-blocking check: no test in the repo exercises the refusal path today, so a future `tauri-plugin-updater` bump or a config change that weakened verification would be caught by nothing in CI. **Likelihood:** low. **Impact:** high if bypassed (arbitrary code execution) — see the dedicated supply-chain row below. | ADR 0068 · ADR 0010 addendum 2026-09-08 · `867.1.2` (closes `miei`) |
| Supply-chain compromise of the update channel itself | (a) the owner's minisign private key is stolen; (b) the release feed (GitHub Releases/`latest.json`) is compromised or spoofed; (c) the build machine producing the signed artifact is compromised | (a) key lives only in `~/.tauri/dohflow.key`, `~/.config/personal-cfo/release.env`, and the password manager — never in a bead, commit, or agent session; quarterly restore-and-sign drill (`7ie.7`) so a lost/unusable key is caught before it's needed; rotation requires a dated ADR 0068 addendum, never silent. (b) GitHub's immutable-releases setting (`fkt5.9`) means a bad release is superseded, never deleted/unpublished; the 2026-09-08 manual drills above (see the row above — a one-time check, not a continuously enforced one) already demonstrated refusal of a mismatched-signature or tampered-artifact release regardless of feed integrity. (c) **accepted residual risk, stated explicitly** — ADR 0068 has no build-provenance/reproducible-build story today; a minisign signature proves the artifact matches what the key signed, not that the signing machine was uncompromised at signing time. **Likelihood:** low for (a)/(b) given the controls above; unquantified for (c). **Impact:** critical for all three — arbitrary code execution on every install that accepts the update. | ADR 0068 §3/§6 · `7ie.7` · `fkt5.9` · `miei` |

## Accepted risks and out of scope (v1)

- **A compromised host** — kernel malware, root access, a keylogger, or memory
  inspection of the *unlocked* process. The vault protects data at rest and in logs;
  it cannot defend a fully-compromised machine while the user is working in it.
- **Physical coercion** (rubber-hose).
- **No password recovery — by design** (ADR 0002). Losing the master password means
  losing the data; the canonical no-reset warning is shown and acknowledged at vault
  creation. This deliberately removes a recovery backdoor as an attack surface.
- **No DohFlow-managed cloud sync or backup upload, no telemetry, no analytics,
  no arbitrary-host egress — by design.** A user-selected cloud-synced backup
  folder is moved by the provider's own client as ciphertext, as described in
  TB3 and the threat row above; this is not an app-managed upload. The only
  network egress by the app is the four narrow, pinned-endpoint paths in TB3
  (SimpleFIN sync, the `dohflow.app`-scoped opener, the pinned updater
  endpoint, and LunchFlow refresh — the last registered but disabled until its
  release), each with its own mitigations in the threats table above. This is
  **not** "no background network activity": SimpleFIN sync and the updater's
  version check — and LunchFlow refresh, once enabled — fire automatically on
  every vault unlock,
  with no user action beyond entering the password; only the opener and the
  updater's download-and-install step require an explicit click. What this
  still eliminates is the broad class of background exfiltration to an
  arbitrary host, MITM-via-arbitrary-host, and server-breach threats a
  general-purpose network stack would carry — not the narrower claim that
  nothing happens on the network without a click.
- **Shared-machine / multi-user isolation.** Named, multiple vaults ship today
  (ADR 0025; `create_vault_named_impl`/`switch_vault_impl`/`list_vaults_impl`/
  `rename_vault_impl` in `apps/desktop/src-tauri/tests/vault_commands.rs`;
  UI in `apps/desktop/src/settings/VaultsCard.tsx`) — e.g. one household
  member keeps a separate vault from another, or "personal" separate from
  "business," each with its own password and full encryption boundary. What
  this does **not** provide, and remains future work: **true multi-user
  access to a single shared vault** — per-user roles/permissions, concurrent
  access, or an audit trail attributing changes to a specific person within
  one vault (ADR 0025 §6: only one vault is unlocked at a time, by design).
  It also does not add OS-level isolation between vaults on a shared machine
  — `vaults.json`'s plaintext registry (names, directory paths, timestamps)
  is readable by anyone with filesystem access to the app data directory,
  same as any other vault's own encrypted files being visible as files;
  only the *contents* of an unlocked-elsewhere vault stay protected, by its
  own password.
- **Cryptanalysis of AES-256 / Argon2id**, including quantum.

## Tracked residual hardening

- `0sqk` (P1) — per-device Argon2id calibration + a CI parameter-floor regression guard.
- `xuu` — OS Keychain + Touch ID, to keep the password out of the clipboard/keyboard path.
- `tif` — isolated WebViews for *future* untrusted content (parsed documents, agent
  output); not needed while the app renders only its own trusted UI (ADR 0010).

## Revision history

- **2026-06-24** — substantive content authored (`personal-cfo-agh`, PR #118).
- **2026-09-06** — find/replace rename only (DohFlow rebrand); no content change.
- **2026-09-11** (`personal-cfo-7ie.8`) — currency review found three shipped
  surfaces missing (SimpleFIN connector, the opener grant, the real signed
  updater) during
  [`docs/security/release-review-v0.1.md` §4](release-review-v0.1.md#4-threat-model-currency-closes-personal-cfo-zaq6-as-internal).
  Revised TB3's network-egress claim to name all three sanctioned paths and
  their controls; added threats-table rows for the SimpleFIN connector, the
  opener grant, the updater's manifest/artifact-tamper risk, and a dedicated
  supply-chain row for the updater (stolen signing key, compromised release
  feed, compromised build machine).
- **2026-09-11 (review follow-up, PR #444)** — corrected two overstatements
  the first pass introduced: the updater's mismatched-signature/tampered-`.tar.gz`
  checks are a one-time manual drill (2026-09-08, `867.1.2`'s notes), not an
  automated or release-blocking test, so the updater row and the supply-chain
  row no longer claim continuous enforcement; and the "no background network
  activity" accepted-risk bullet was contradicting TB3 and the threats table
  above it — SimpleFIN sync and the updater's version check both fire
  automatically on every vault unlock, not only on user action, which TB3
  item 1 and the accepted-risks bullet now state plainly.
- **2026-09-11** (`personal-cfo-a5jm6`) — the "Shared-machine / multi-user
  isolation" accepted-risk bullet was stale: it called named/multiple vaults
  "future work (ADR 0025)," but ADR 0025 shipped — the bullet now names what
  actually ships (independent, separately-encrypted named vaults, switchable
  one at a time) versus what remains unbuilt (true multi-user access —
  roles/permissions/concurrent access — *within* a single shared vault).
- **2026-09-22** (`personal-cfo-ii3an`, ADR 0024 addendum A) — updated asset A4
  and the backup-theft threat for format v2: the header carries the existing
  wrapped vault envelope, the payload key is derived from the unlocked vault
  DEK, and the resulting envelope fingerprint/linkability is an accepted
  metadata exposure. The vault password remains required to restore.
- **2026-09-22** (`personal-cfo-8qh`) — documented the user-selected cloud
  client as a ciphertext egress path outside DohFlow's network stack, and the
  v2 header's accepted cross-backup linkability; clarified that no destination
  or cloud-folder demand data is reported to DohFlow.
- **2026-09-27** (`personal-cfo-r2pow`, ADR 0076) — added the fourth TB3
  egress path, LunchFlow refresh against the pinned
  `https://www.lunchflow.app/api/v1`, stated as registered but disabled until its
  release; added its threats-table row beside SimpleFIN's; corrected the
  "three paths" wording in TB3 and the accepted-risks bullet.
