# Threat model v1

DohFlow is a **local-first, offline, single-user** desktop finance vault (a user
may keep several such vaults side by side — ADR 0025 — but each one remains a
single-user unit; see "Accepted risks and out of scope" below). This model is
asset-centric with a STRIDE-lite lens, and covers the shipped R1 surface
(`personal-cfo-agh`, Risk Register §24). It is a living document — extend it as
new surfaces land (import, documents, agents).

**Last substantive revision: 2026-09-11** (`personal-cfo-7ie.8`) — see
[Revision history](#revision-history) at the bottom of this document.

## What we are protecting (assets)

| | Asset | Property |
|---|-------|----------|
| **A1** | Financial data at rest — accounts, transactions, income, bills, balances, attachments | confidentiality + integrity |
| **A2** | The data-encryption key (DEK) — unwrapped into memory only while the vault is unlocked | confidentiality |
| **A3** | The master password — derives the key-encryption key (KEK) via Argon2id | confidentiality |
| **A4** | Encrypted backups — single-file portable containers | confidentiality + integrity |
| **A5** | Logs / diagnostics — must never carry A1 | confidentiality |

## Trust boundaries

- **TB1 — Rust core ↔ WebView frontend** (ADR 0003). The Rust core is trusted; the
  WebView is treated as semi-trusted. All traffic crosses a typed IPC command surface
  (ADR 0006); no database types and **no key material** ever reach the frontend.
- **TB2 — unlocked memory ↔ disk at rest.** While unlocked the DEK lives in memory;
  at rest everything is SQLCipher-encrypted. Parameters:
  [`docs/security/encryption-design.md`](encryption-design.md).
- **TB3 — the app ↔ the outside world.** Three sanctioned egress paths exist
  (added since R1; see [Revision history](#revision-history)). Two of the
  three fire **automatically on every vault unlock**, not only when asked;
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
  A user-initiated file export (backup) or import remains the only other data
  movement and is local-disk, not network. See the threats table below for each
  path's specific mitigations and evidence.
- **TB4 — WebView ↔ OS** (ADR 0010). A strict Content Security Policy and a minimal
  Tauri capability set bound what the WebView can reach.

## Threats and mitigations (shipped)

| Threat | Vector | Mitigation | Ref |
|--------|--------|------------|-----|
| Disk theft / lost laptop reads A1 | the `.db` file on disk | full-database SQLCipher AES-256; the envelope wraps the DEK under the password-derived KEK | ADR 0002 |
| Plaintext leaks to side files | WAL / SHM / journal / OS temp | SQLCipher encrypts the side files; `PRAGMA temp_store=MEMORY`; a sentinel leak test scans every artifact incl. after an unclean drop | `zxvl` (`1bkb`) |
| DEK scraped from memory | process memory while unlocked | `SecretBytes` with `mlock` + zeroize-on-drop; the DEK is dropped (scrubbed) on lock | `1t0`, ADR 0002 |
| Brute-force the password / weak KDF | offline guessing against the envelope | Argon2id at **64 MiB / t=3** by default (exceeds the OWASP floor); a 256 MiB profile exists | ADR 0002 · [`encryption-design.md`](encryption-design.md) · *hardening: `0sqk`* |
| Financial data leaks into logs | tracing spans / diagnostics | a global redacting tracing layer scrubs §6.6 attributes app-wide; a release-blocking CI test asserts an exhaustive corpus is scrubbed | `2vs` / `zobt` |
| WebView XSS / malicious rendered content | a content-injection bug in the UI | strict CSP (CI-guarded against regression), no remote content loaded, no secrets in the frontend, scoped capabilities | ADR 0010 (`h7h8`) |
| Untrusted IPC input reaches the core | crafted command payloads | a sealed, typed command bus; Rust is authoritative for validation; raw `invoke` is lint-forbidden | ADR 0003 / 0006 |
| Backup theft | a copied backup file | encrypted single-file container (AES-256-GCM payload under a freshly-wrapped backup DEK); wrong password fails closed before any write | ADR 0024 (`ef3`/`au3`) |
| Attachment plaintext at rest | blobs on disk | per-blob AES-256-GCM, content-addressed, keys wrapped under the DEK; a canary test proves no plaintext artifact | ADR 0023 (`bcj`) |
| Vulnerable third-party dependency | supply chain | `cargo audit` (both workspaces) + `pnpm audit` in CI | `7t7` |
| Secret committed by mistake | git history | gitleaks in CI with a tight allowlist | `6r6` |
| Silent data corruption | disk faults, interrupted writes | `PRAGMA integrity_check` + read-model rebuild repair + a verified backup/restore drill | `n9w` / `5ivp` / `7pfu` |
| Schema downgrade / tampered vault | an older/forked binary opening a vault | schema-version coherence check; up/down migration safety tests; pinned SQLCipher engine | `n9w` / `c545` / `7igv` |
| Connector credential or synced data intercepted/misused | the SimpleFIN Bridge egress path (`connector_link`/`connector_sync`/`connector_forget`, on vault-open + manual refresh) | user's own token, claimed client-side, no project secret, no relay; access URL stored as an encrypted `connector_connections.credential` column (SQLCipher, round-trips via backup/restore, never logged or on the listing row type); fetch is Rust-side (`ureq`), not WebView-reachable. **Likelihood:** low — needs a compromised SimpleFIN Bridge account or a fully-compromised host (already accepted below). **Impact:** medium — scoped to synced account/transaction data plus provider access, not the vault password or DEK. | ADR 0060 · `gglk` · `crates/connectors/simplefin-adapter/src/transport.rs` · `simplefin_drill.rs` |
| WebView navigates to an attacker-controlled or unexpected origin | the opener grant behind the About card / Support row "open in browser" buttons | scope is exactly `https://dohflow.app/*` (enforced in Rust regardless of WebView request); `opener:default`/`allow-open-path`/`allow-reveal-item-in-dir` and the `shell`/`fs`/`http` plugins are never granted; anchor-click interceptor explicitly disabled, so `openExternal.ts` is the only path to the command and it pre-filters the URL; 4 `acl_coverage.rs` tests pin the grant shape, the disabled interceptor, the absence of wider grants, and an unchanged CSP. **Likelihood:** low — a widened grant must survive review and 4 drift tests. **Impact:** low — worst case opens the project's own domain, not an arbitrary URL. | ADR 0010 addendum 2026-09-06 · `n76x.18` · `apps/desktop/src-tauri/tests/acl_coverage.rs` |
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
- **No cloud sync, no telemetry, no analytics, no arbitrary-host egress — by
  design.** The only egress is the three narrow, pinned-endpoint paths in TB3
  (SimpleFIN sync, the `dohflow.app`-scoped opener, and the pinned updater
  endpoint), each with its own mitigations in the threats table above. This is
  **not** "no background network activity": two of the three — SimpleFIN sync
  and the updater's version check — fire automatically on every vault unlock,
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
