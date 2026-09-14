# macOS release signing + notarization runbook

- Bead: `personal-cfo-867.1` (Release packaging: code-signing + notarization + DMG + auto-update)
- Status: **runbook only** — the owner-side steps below can be done today; the repo
  wiring in Part B is the 867.1 implementation plan and is **deliberately not applied
  yet** (`tauri.conf.json` is untouched).
- Audience: the project owner, who has a brand-new Apple Developer account and has
  never signed or notarized anything. Every step is spelled out; nothing is assumed.

Sources: Tauri v2 docs — `https://v2.tauri.app/distribute/sign/macos/`,
`https://v2.tauri.app/distribute/dmg/`, `https://v2.tauri.app/plugin/updater/`,
`https://v2.tauri.app/reference/config/` (the `bundle.macOS` section). These are
cited from training knowledge, not fetched; anything tagged **VERIFY-ON-BUILD** must
be re-checked against the live docs / CLI when 867.1 implementation starts.

Why both halves matter: **signing** proves the app comes from you and wasn't
tampered with; **notarization** is Apple's automated malware scan. Without both,
macOS Gatekeeper shows users the "can't be opened / unidentified developer" wall.

---

## Part A — Owner manual steps (one-time setup, ~1 hour + Apple wait times)

### A1. Confirm the Apple Developer Program enrollment is complete

1. Go to `https://developer.apple.com` → **Account** and sign in.
2. You need a **paid, approved** Apple Developer Program membership ($99/yr).
   A free account can build and run locally but **cannot create Developer ID
   certificates**. If the page still says "enrollment pending", wait — approval can
   take from hours to a couple of days.
3. Once approved, open **Certificates, IDs & Profiles** (left sidebar). If you can
   see it, enrollment is done.
4. Note your **Team ID** now: Account → **Membership details** → "Team ID" (a
   10-character string like `ABCDE12345`). You'll see it again inside the signing
   identity string.

### A2. Create the *Developer ID Application* certificate

Apple has several certificate types. The one you need is **Developer ID
Application** — it signs apps distributed **outside** the Mac App Store (direct
download, which is our model). Do NOT pick "Apple Development" (local dev only),
"Apple Distribution" (App Store), or "Developer ID **Installer**" (signs `.pkg`
installers, not the app itself).

Two ways to create it — pick ONE. The Xcode route is easier; the portal route works
without opening Xcode.

Important either way: **only the Account Holder role can create Developer ID
certificates**, and Apple caps you at 5 of them — treat it as precious.

**Route 1 — Xcode (recommended):**

1. Install/open Xcode → **Settings** (⌘,) → **Accounts** tab.
2. `+` → sign in with your Apple ID if it isn't listed; select your team.
3. Click **Manage Certificates…** → `+` (bottom-left) → **Developer ID Application**.
4. Done — Xcode generated the private key and certificate directly into your
   **login keychain**.

**Route 2 — Developer portal + Keychain CSR:**

1. On your Mac: **Keychain Access** app → menu **Keychain Access → Certificate
   Assistant → Request a Certificate From a Certificate Authority…**
2. Fill in your email + name, leave CA Email empty, choose **"Saved to disk"**.
   This writes a `CertificateSigningRequest.certSigningRequest` file AND creates the
   private key in your login keychain.
3. `https://developer.apple.com` → **Certificates, IDs & Profiles → Certificates** →
   `+` → under "Software" pick **Developer ID Application** → Continue.
   - If it asks for a "profile type" choice (G2 Sub-CA vs Previous Sub-CA), take the
     default (G2). **VERIFY-ON-BUILD** — portal UI changes frequently.
4. Upload the `.certSigningRequest` file → Continue → **Download** the `.cer`.
5. Double-click the downloaded `.cer` — Keychain Access installs it and pairs it
   with the private key from step 2.
6. **Install the intermediate CA (Route 2 only — Xcode does this for you, the portal
   does not).** Without it the cert shows a red *"not trusted"* in Keychain Access and
   `security find-identity -v` reports **0 valid identities** even though the cert +
   key are correctly paired. Developer ID leaf certs chain through the **Developer ID
   – G2** intermediate up to the Apple Root; only the leaf comes in the `.cer`. Install
   the intermediate and verify it matches your leaf's issuer:

   ```sh
   # Confirm which intermediate your leaf needs (issuer line, e.g. "…OU=G2…"):
   security find-certificate -c "Developer ID Application" -p login.keychain \
     | openssl x509 -noout -issuer
   # Fetch that intermediate from Apple and confirm its SUBJECT == the issuer above:
   curl -fsSL -o /tmp/DeveloperIDG2CA.cer https://www.apple.com/certificateauthority/DeveloperIDG2CA.cer
   openssl x509 -inform DER -in /tmp/DeveloperIDG2CA.cer -noout -subject
   # Install it into the login keychain (trust flows down from the already-trusted Apple Root):
   security add-certificates -k "$HOME/Library/Keychains/login.keychain-db" /tmp/DeveloperIDG2CA.cer
   ```

   (The genuine G2 intermediate is SHA-256 `F16CD3C5…43D2DF3A`; verify with
   `openssl x509 -inform DER -in /tmp/DeveloperIDG2CA.cer -noout -fingerprint -sha256`.)

**Back up the identity now:** in Keychain Access, find the certificate (category
"My Certificates"), expand it to confirm a private key hangs under it, right-click →
**Export** → save a `.p12` with a strong password to your password manager /
offline backup. The private key exists ONLY on this Mac — if you lose it, you
revoke and start over (against the 5-cert cap).

### A3. Find your signing identity string

In Terminal:

```sh
security find-identity -v -p codesigning
```

Expected output line:

```
1) ABC123… "Developer ID Application: Chris Bustos (ABCDE12345)"
```

The quoted string — `Developer ID Application: NAME (TEAMID)` — is the **signing
identity**. Copy it exactly (with the parentheses).

If it reports **0 valid identities**, diagnose with the same command *without* `-v`
(which lists identities regardless of trust):

```sh
security find-identity -p codesigning     # all identities, valid or not
```

- Your cert **appears** here but `-v` shows 0 → the chain is untrusted: **install the
  Developer ID – G2 intermediate** (A2 Route 2, step 6). This is the usual portal-route
  miss.
- Your cert **doesn't appear at all** → the `.cer` is installed without its private key
  (wrong Mac, or the CSR was made on another machine — import your `.p12` backup).

### A4. Create notarization credentials

Notarization uploads the signed app to Apple and polls for a verdict. The tooling
authenticates one of two ways. **Recommended: the App Store Connect API key** — it
isn't coupled to your personal Apple ID password/2FA, can be scoped and revoked
independently, and is what CI will eventually use.

**Option 1 — App Store Connect API key (recommended):**

1. `https://appstoreconnect.apple.com` → **Users and Access** → **Integrations**
   tab → **App Store Connect API** → **Team Keys**.
2. `+` (Generate API Key) → name it (e.g. `personal-cfo-notarize`), role
   **Developer** is sufficient (Admin also works). **VERIFY-ON-BUILD**: role
   requirements for `notarytool`.
3. Record THREE things:
   - **Issuer ID** (top of the page, a UUID) — shared by the whole team.
   - **Key ID** (per-key, e.g. `2X9R4HXF34`).
   - The **`.p8` private key file** — downloadable **exactly once**. Click
     Download and store it at `~/.appstoreconnect/private_keys/AuthKey_<KEYID>.p8`
     (the conventional path `notarytool` searches) AND in your password manager.
     Never in the repo.

**Option 2 — app-specific password (fallback):**

1. `https://account.apple.com` → **Sign-In and Security** → **App-Specific
   Passwords** → generate one named e.g. `personal-cfo-notarize`.
2. You'd then use `APPLE_ID` (your Apple ID email), `APPLE_PASSWORD` (this
   app-specific password), and `APPLE_TEAM_ID` (from A1) instead of the API-key
   variables. Works, but rotates out from under you whenever the Apple ID password
   changes, and puts your personal account in the loop. Use only if the API key
   route is blocked.

### A5. Where secrets live for LOCAL manual releases

For manual releases from your Mac, everything is passed as **environment variables
in the shell that runs the release script** — never committed, never in
`tauri.conf.json`:

| Variable | Value | From step |
|---|---|---|
| `APPLE_SIGNING_IDENTITY` | `Developer ID Application: NAME (TEAMID)` | A3 |
| `APPLE_API_ISSUER` | Issuer ID (UUID) | A4 |
| `APPLE_API_KEY` | Key ID (e.g. `2X9R4HXF34`) | A4 |
| `APPLE_API_KEY_PATH` | absolute path to the `AuthKey_<KEYID>.p8` | A4 |

Practical pattern: keep them in `~/.config/personal-cfo/release.env` (outside the
repo, `chmod 600`), and `source` it in the shell before running the release script.
Do NOT put them in `.zshrc` (every process would inherit them) and do NOT use a
committed `.env`. The repo's `.gitignore` should never need to know these exist,
because they never enter the worktree.

(The env-var names above are the ones the Tauri v2 bundler reads for macOS
signing/notarization. **VERIFY-ON-BUILD** against
`https://v2.tauri.app/distribute/sign/macos/` — earlier Tauri versions used
`APPLE_API_KEY` for the file path; v2 splits Key ID and path.)

### A5b. Credential custody + expiry calendar

None of the values below live in this repo. This section records **where they
live and when they die** — no secrets, only locations and dates.

| Item | Expires / renews | Custody |
|---|---|---|
| Developer ID Application cert | **2031-07-06** | Login keychain + password manager (`.p12`, base64) |
| Apple Developer Program membership | Annual, USD 99, **auto-renew ON** | Apple ID account |
| App Store Connect API key (`.p8`) | No expiry; **one-time download only** | `~/.appstoreconnect/private_keys/` + password manager (base64) |
| `release.env` values | n/a | `~/.config/personal-cfo/release.env` + password manager |

Identity: `Developer ID Application: Christopher Bustos (SKU2HZ2U6L)`,
SHA-1 `9B56817B41438C25E21322DED79953A2F1B93411`.

Re-check the certificate expiry at any time with:

```sh
security find-certificate -c 'Developer ID Application' -p | openssl x509 -noout -enddate
```

**Why both dates matter.** The certificate's private key exists *only* in this
Mac's keychain — Apple cannot re-issue it, only revoke and replace. The `.p8`
downloads exactly once and is then gone forever. And if the membership lapses,
the certificate stops working: every notarized build breaks, including the
auto-updater for users already running the app. The membership is the shorter
fuse of the two by five years.

**Custody format is base64 text in password-manager note bodies, not file
attachments** — deliberately. The manager's attachments are excluded from its
own vault exports, which would silently drop the one file Apple will never
re-issue during a future migration. Text survives an export. Verified by
decoding back out and matching sha256 against the source (`personal-cfo-867.1.1`).

**Restore note:** `APPLE_API_KEY_PATH` is an absolute path on *this* Mac. On a
rebuild, decode the `.p8` back to that path and `chmod 600`, or update the
variable to the new location.

Both dates belong in the launch-ops calendar (`personal-cfo-n76x.22`), and the
locations feed the secrets-custody inventory (`personal-cfo-7ie.7`).

### A6. Generate the Tauri updater signing keypair

Separate from Apple entirely: the Tauri **updater** verifies update artifacts
against a Tauri-specific keypair (minisign format), so a compromised download
server can't feed users a hostile "update".

```sh
pnpm tauri signer generate -w ~/.tauri/dohflow.key
```

- Choose a password when prompted (or empty for none — prefer a password; it's
  then required via `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` at build time).
- **Private key** (`~/.tauri/dohflow.key`): stays on your machine + password
  manager. Never in the repo. Exposed to builds via `TAURI_SIGNING_PRIVATE_KEY`
  (path or content) + `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.
- **Public key** (`~/.tauri/dohflow.key.pub`): NOT secret — it goes into
  `tauri.conf.json` under `plugins.updater.pubkey` (Part B4, applied).
- Note: the app also ships a from-source dev updater (`check_for_update` /
  `apply_update` in `src-tauri/src/update.rs`), which stays the update path
  when `PCFO_BUILD_CHANNEL == dev`; the signed-artifact updater (Part B4) is
  what release builds use.
- **Done 2026-09-07** (`personal-cfo-867.1.2`): key generated at
  `~/.tauri/dohflow.key` (filename aligned to the DohFlow rename, ADR 0067 —
  earlier drafts of this doc said `personal-cfo.key`, corrected here). Add
  the two `TAURI_SIGNING_PRIVATE_KEY` / `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
  lines to `~/.config/personal-cfo/release.env` (A5's pattern) before running
  `scripts/release.sh` for real — as of this writing that file carries only
  the `APPLE_*` lines from A5's table; `release.sh`'s preflight refuses to
  build without both `TAURI_SIGNING_*` vars present, exactly like the
  `APPLE_SIGNING_IDENTITY` check above.

---

## Part B — Repo wiring (867.1 — APPLIED 2026-07-05, PR #264)

Shipped. The VERIFY-ON-BUILD markers below were resolved against the vendored
tauri-bundler 2.11.2 source and the `generate_context!` config validation (the
desktop crate compiles the config, so a bad key is a build error).

### B1. `tauri.conf.json` → `bundle.macOS` — APPLIED

```jsonc
"bundle": {
  "macOS": {
    "hardenedRuntime": true   // REQUIRED for notarization; explicit so a regression shows in review
  }
}
```

Resolved decisions:
- **`signingIdentity` is NOT in the committed config.** `release.sh` injects it at
  build time via `tauri build --config` from `$APPLE_SIGNING_IDENTITY`, so the repo
  stays machine-independent and contributors build unsigned. The injected `--config`
  also re-asserts `hardenedRuntime: true` (defense-in-depth: `--config` deep-merges, but
  re-asserting means a signed build can never silently lose hardened runtime even if
  merge semantics changed).
- **No entitlements file** — verified correct for a Tauri app: it is not App-Sandboxed
  (Developer ID), and — unlike Electron, which bundles Chromium and JITs *in* the app
  process (needing `allow-jit` + `allow-unsigned-executable-memory`) — Tauri uses the
  **system WKWebView**, whose JS/JIT runs in Apple's separately-signed `WebContent`
  helper. So the app process needs no `allow-jit`; the offline CSP means no network
  entitlement. Zero entitlements is the most-secure posture. *Fallback (B6 must
  confirm launch):* if the notarized app fails to launch with a JIT/codesign error, add
  `apps/desktop/src-tauri/entitlements.plist` containing `com.apple.security.cs.allow-jit`
  and set `bundle.macOS.entitlements` to it, then re-release.
- **No `minimumSystemVersion` set** — we inherit Tauri/wry's framework default (no
  app-specific macOS API forces a higher floor). If a support policy later dictates a
  floor, set it here with that rationale.

### B2. Notarization plumbing — APPLIED (automatic)

Verified: the bundler notarizes automatically during `tauri build` when the
`APPLE_API_KEY` + `APPLE_API_ISSUER` (+ `APPLE_API_KEY_PATH`, or a `.p8` discovered
under `~/.appstoreconnect/private_keys/`) credentials are in the environment **and**
a Developer ID signing identity was used — it uploads with `notarytool`, waits for the
verdict, and **staples automatically** (the default; `skip_stapling` is off). No config
keys. `release.sh` verifies the staple with `xcrun stapler validate` on both the `.app`
and the `.dmg`.

### B3. DMG artifact — APPLIED

`release.sh` builds `--bundles app,dmg` (verified flag form for CLI 2.11.2). The
committed `bundle.targets: "all"` is unchanged for dev flexibility; the release forces
the artifact set explicitly. `icons/icon.icns` is present, so the DMG has an icon.

### B4. Updater (graduates the from-source dev updater) — APPLIED (`personal-cfo-867.1.2`, ADR 0068)

- `tauri-plugin-updater` (crate + JS guest bindings `@tauri-apps/plugin-updater`,
  pinned exactly to match, `2.11.0`) and `tauri-plugin-process`
  (`@tauri-apps/plugin-process`, `2.3.1`) added.
- `capabilities/default.json` grants `updater:default`; `capabilities/destructive.json`
  grants `process:allow-restart` alongside `apply_update`/`relaunch_app` — a new
  binary swap belongs in the same capability those already gate. Pinned by
  `tests/acl_coverage.rs`'s `the_updater_grant_is_exactly_updater_default_in_the_general_capability`
  / `the_process_grant_is_exactly_allow_restart_in_the_destructive_capability`.
- `plugins.updater`: `pubkey` (A6's public key) and `endpoints` —
  `["https://github.com/dohflow/dohflow/releases/latest/download/latest.json"]`
  (ADR 0068 point 2) — both pinned by
  `the_updater_pubkey_and_endpoint_are_exactly_the_committed_values`.
- `bundle.createUpdaterArtifacts: true` so `tauri build` emits the signed updater
  archive + `.sig` alongside the DMG. (v2 uses this flag, not v1's `updater.active`.)
- `apply_update`/`relaunch_app` (the from-source dev updater's own commands) stay in
  the `destructive` capability unchanged — they're still the update path when
  `PCFO_BUILD_CHANNEL == dev`; release builds call the real plugin's JS API
  (`useSoftwareUpdate.ts`) directly instead — recorded in ADR 0003's 2026-09-08
  addendum, which draws the line between the app's OWN commands (generated-commands
  IPC) and first-party Tauri plugin bindings, and in ADR 0010's 2026-09-08 addendum
  for the two capability grants that permit it.
- `miei` (update-signature verification, the two negative tests) closes once the
  owner runs the live smoke-repo round trip — see
  `docs/operations/updater-smoke-test.md`.

`release.sh` now REQUIRES `TAURI_SIGNING_PRIVATE_KEY` +
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (A6) for every real build, alongside the
Apple identity/notary vars — there is no "skip" flag for the updater signature
the way `RELEASE_SKIP_NOTARIZE` skips Apple notarization, since a release with no
valid updater signature would ship an app that can never verify its own future
updates.

### B5. `scripts/release.sh` — SHIPPED (extended for the updater, `personal-cfo-867.1.2`)

The real script is `scripts/release.sh` (this replaced the earlier sketch). Usage:

```sh
source ~/.config/personal-cfo/release.env
./scripts/release.sh --check                       # preflight: valid identity + creds present, no build
./scripts/release.sh --notes "…"                    # full signed + notarized build + latest.json
./scripts/release.sh --notes "…" --smoke-endpoint "https://github.com/dohflow/updater-smoke/releases/latest/download/latest.json"
                                                     # same, but latest.json's endpoint override points the
                                                     # BUILT app at the smoke repo (see updater-smoke-test.md) —
                                                     # never set for a real release
```

It preflights (valid codesigning identity + notary creds + `TAURI_SIGNING_PRIVATE_KEY`
+ `_PASSWORD`, B4), refuses a dirty tree (`RELEASE_ALLOW_DIRTY=1` overrides), injects
the signing identity (and, only with `--smoke-endpoint`, the updater endpoint override)
via `--config`, builds `app,dmg` (the updater archive + `.sig` come along automatically
via `createUpdaterArtifacts`), then **verifies** (`codesign --verify --deep --strict`,
`stapler validate`, `spctl` Gatekeeper assessment, and that the updater archive + `.sig`
both exist) so a build that didn't actually sign — either half — fails loudly instead of
looking shippable. Writes `latest.json` next to the DMG, with a placeholder
`platforms.darwin-aarch64.url` (the real GitHub release asset URL only exists once this
build's artifacts are uploaded — filling it in is `./scripts/publish-release.sh package`'s
job, see [`release-checklist.md`](release-checklist.md) (`867.1.3`)).
`RELEASE_SKIP_NOTARIZE=1` signs only (fast iteration; the artifact won't pass Gatekeeper)
by hiding the notary creds from the bundler — this still requires the updater key; there
is no analogous skip for it.

### B6. First-release smoke test (manual, once) — and the entitlements safety-net

Copy the DMG to a second Mac (or a fresh macOS VM / new user account), download it
through a browser so it carries the quarantine attribute, and confirm two things:

1. **It opens with no Gatekeeper wall.** `xattr -p com.apple.quarantine` on the
   downloaded DMG should exist — quarantined + notarized = silent open.
2. **The app actually LAUNCHES and the UI renders.** This is the one check that
   validates the zero-entitlements decision (B1): if the app quits immediately or the
   window is blank with a codesign/JIT error in Console.app, apply the B1 fallback
   (add `entitlements.plist` with `com.apple.security.cs.allow-jit`) and re-release.
   Expected outcome for a Tauri/WKWebView app: it launches fine with no entitlements.

## Troubleshooting quick hits

- `security find-identity` shows nothing → the `.cer` is installed but its private
  key isn't in this keychain (A2 backup note; import the `.p12`).
- `errSecInternalComponent` while signing → keychain is locked (common over
  SSH): `security unlock-keychain login.keychain-db`.
- Cert shows red *"not trusted"* in Keychain Access, or "unable to build chain to
  self-signed root", or `find-identity -v` shows 0 valid while `find-identity` (no `-v`)
  lists the cert → **missing Developer ID – G2 intermediate**. Install it (A2 Route 2,
  step 6). Confirmed live on the maintainer's Mac 2026-07-05: this was the only thing
  standing between a correctly-installed cert and a valid signing identity.
- Notarization rejected → fetch the log:
  `xcrun notarytool log <submission-id> --issuer "$APPLE_API_ISSUER" --key-id "$APPLE_API_KEY" --key "$APPLE_API_KEY_PATH"`.
  Most common first-time causes: hardened runtime off, or an unsigned nested binary.
- First-ever notarization for a new team can take noticeably longer (Apple warms up
  trust for the account); subsequent runs are typically minutes.
