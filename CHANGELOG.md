# Changelog

All notable changes to DohFlow are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/). This project is pre-1.0
and releases are not yet published; the **Unreleased** section is the running
record until the first tagged release.

## [Unreleased]

## [0.3.0] - 2026-10-19

### Added

- **Scheduled encrypted backups** (`personal-cfo-8qh`, `personal-cfo-ii3an`).
  In **Settings → Backups**, choose a destination folder and a schedule: off,
  daily, weekly (the default) or monthly. A backup that is due runs at the next
  unlock, never blocks the unlock and does not ask for your password again.
  **Back up now** makes and verifies one right away, and the card shows your
  most recent verified backup and any scheduled backup that failed. Backups are
  kept until you remove them yourself: DohFlow never deletes older backup files.
  If you pick a folder managed by iCloud Drive, Dropbox or a similar client,
  that client carries the encrypted file; DohFlow does not upload it. Restoring
  a backup still needs your vault password, and there is no DohFlow cloud
  backup and no password reset.
- **Restore a backup as a new vault** (`personal-cfo-g3m.3`). From the launch
  screen's locked vault, or from the recovery screen when a vault's files are
  incomplete, choose **Restore as a new vault…**, pick the backup file, name the
  restored copy and enter the password that protected the backup. DohFlow makes a
  separate vault, checks it and opens it; the original vault's files are left
  exactly as they were, so you do not have to delete anything to get back in.
- **Manual backup export lives in Settings** (`personal-cfo-lkgf`): **Export
  backup…** is now in the same Backups card as the schedule.
- **Rotate encryption key** in Settings (`personal-cfo-2y8`, ADR 0083). It
  gives the vault a new encryption key without changing your password, for
  when you think the key may have been exposed. Your data and attachments are
  unchanged, and an interrupted rotation is finished or undone safely the next
  time DohFlow opens. Backups made before rotating still open with the password
  you had then, and an older vault can move to the current key-derivation
  settings this way.
- **Local diagnostics** in Settings (`personal-cfo-lyd`). Preview exactly what a
  diagnostics file would contain, then save it to a file only if you choose to.
  It holds counts and timings from the current session, with no transaction,
  account or vault details. Nothing is uploaded, and what was collected is
  dropped when you lock the vault.
- **Refresh bank connections automatically** (`personal-cfo-lqk`). Each
  connection has a **Refresh automatically** setting: every time you open the
  vault, daily, weekly, or only when you press Refresh now.
- **Import from another app** (`personal-cfo-gvidg`, `personal-cfo-tulv`). The
  import dialog has an **Import from** step; **YNAB** register exports are
  supported, including tab-separated files, decimal commas, and a file that
  covers several accounts. A mapping step lets you send each source account to
  an existing account, a new one, or skip it.
- **Imports tell you what they skipped.** The import summary lists rows that
  could not be used, with the reason for each (`personal-cfo-pxi.10`). A row the
  ledger refuses is flagged in the Money Inbox instead of stopping the whole
  import (`personal-cfo-pxi.9`), and re-uploading the same file no longer fills
  the inbox with duplicates of rows you already reviewed (`personal-cfo-yl5`).
- **A newer DohFlow that needs a newer macOS is not offered** (`personal-cfo-md8h2.4`).
  If the latest release needs a macOS version above yours, Settings says which
  macOS it needs instead of claiming you are up to date, and this version keeps
  working.

### Changed

- Retry-safety keys (idempotency keys) are now always random version 4
  identifiers, and a key in any other shape is rejected with a clear error
  (`personal-cfo-g3m.6`). Nothing changes in how the app behaves: every action
  already sent such a key, and the places that made their own now use the same
  random kind. A key saved by an earlier version still protects a retried action
  until it expires after 30 days.
- **Bank connections check the account's currency** (`personal-cfo-049p6`,
  `personal-cfo-pxi.8`). A connector account maps to a DohFlow account only when
  its currency is known and matches your base currency; otherwise the mapping is
  refused with the reason, and creating a new account from the mapping step no
  longer leaves an empty account behind when it is refused.
- **Two connections to one account are caught** (`personal-cfo-6evt`). Mapping a
  connected account onto one that another connection already feeds asks you what
  to do (don't import this one, map it elsewhere, or import from both) and says
  plainly that a transaction reported on different dates can still import twice.
- **A failed duplicate check is shown as an error** (`personal-cfo-pxi.11`), with
  a Retry button, instead of reading as "safe to import".
- The app framework (Tauri) is updated from 2.11 to 2.12.4 (`personal-cfo-md8h2.8`).
  The same security checks were re-run; nothing changes in your data or
  settings.

### Security

- The app window is stopped from navigating anywhere outside the app itself, and
  the strict content-security policy is pinned by tests (`personal-cfo-2rf`).
- Two sets of dependency security advisories, in the TLS library `rustls` and an
  unused archive library, are cleared (`personal-cfo-md8h2.5`).

### Fixed

- The bundled database engine is updated to SQLCipher 4.14.0 / SQLite 3.51.3,
  which includes SQLite's fix for a rare write-ahead-log reset corruption bug
  (`personal-cfo-g3m.5`). Existing vaults open unchanged: no vault, backup or
  key format changes. Once a vault has been opened by this version, an older
  DohFlow build is not supported for it; to go back, restore a backup made by
  that build. Building from source now needs Rust 1.95 or newer.

- Existing vaults with a newer schema are refused before migrations or
  post-unlock writes (`personal-cfo-g3m.4`). The unlock screen asks for a
  compatible newer DohFlow version instead of opening or stamping the vault
  down. Unverified or incomplete existing layouts also fail closed.

## [0.2.0] - 2026-09-18

### Added

- **Intel Mac support via a universal binary** (`personal-cfo-sg8.1.1`,
  ADR 0072). DohFlow now ships one DMG containing both `arm64` and `x86_64`
  builds — Intel Macs are a fully supported tier, not just Apple silicon.
  Backed by a CI build-and-launch smoke test on real Intel hardware
  (`macos-15-intel`) that runs on every push, and a documented owner-run
  Gatekeeper drill (`docs/operations/intel-smoke.md`) for the real-hardware
  download/install/unlock path.

### Changed

- The connector action labeled **"Sync" is now "Refresh"** everywhere in the
  UI (`personal-cfo-p7qc6`); "Sync" is reserved for the upcoming DohFlow Sync
  feature so the two aren't confused.

### Fixed

- Development builds and release builds now use **separate vault data
  directories** (`personal-cfo-he3xo`), so running a dev build can no longer
  read or write a real release vault.

## [0.1.0] - 2026-09-14

### Added

- **Dark mode, following the macOS appearance — everywhere, including locked**
  (`personal-cfo-17u1`). The app follows the system appearance at launch and
  reacts live when macOS switches, with a shared theme state (`ThemeProvider`,
  mounted once at the app root) that both a floating System/Light/Dark toggle
  and a detailed Settings → Appearance card read — the toggle is reachable
  from every screen, the vault-picker and lock screens included, not just once
  unlocked. Applies immediately, persists across relaunch, and shows no flash
  of the wrong theme on start (a separate boot-time script applies the stored
  preference before React ever mounts). An explicit override best-effort syncs
  the native title bar too (`core:window:allow-set-theme`). Every `--chart-*`
  and semantic color is a live CSS variable already, so charts needed no
  per-component changes; token parity and WCAG contrast between the two
  themes are enforced by a new test (`styles/theme-contrast.test.ts`), with a
  small, named allowlist for tokens that deliberately don't flip.

- **About card and a quiet Support DohFlow row** (`personal-cfo-n76x.18`,
  ADR 0010 addendum 2026-09-06). Settings gains an About card — name, version,
  channel, commit — with links to the website, help, release notes, bug reports
  (stamped with version/channel/commit only), the security policy, and the
  license (AGPL-3.0-only), plus one muted "Support DohFlow" line; the sidebar
  gets a matching heart. Every link opens in the system browser and points only
  at `https://dohflow.app/`: the app is granted `opener:allow-open-url` scoped to
  that origin and nothing else, and the frontend refuses any other URL before
  the call.

### Changed

- **Renamed the app to DohFlow** (ADR 0067, `personal-cfo-fkt5.3`). The product
  name, window title, bundle name (`DohFlow.app`), and in-app strings now say
  DohFlow. The bundle identifier (`ai.personalcfo.desktop`) and the data
  directory are unchanged, so existing vaults open as before.
- **The brand mark and wordmark in the app** (`personal-cfo-4d8.28.3`). The
  sidebar header, the vault picker, and the About card show the approved
  DohFlow lockup — the mark and the typeset wordmark as SVG geometry carried
  verbatim from the brand assets, painted through the design tokens so the
  wordmark follows the theme's text color — in place of a placeholder icon and
  a plain-text name.

### Persistence / security

- **Pinned the vault storage engine** (`personal-cfo-7igv`, mitigates risk
  `personal-cfo-aia0` "SQLCipher version incompatibility"). The vault is a
  SQLCipher-encrypted SQLite database, and these versions are now pinned **exact**
  and verified in CI:

  | Engine | Version |
  |---|---|
  | SQLCipher (`PRAGMA cipher_version`) | `4.5.7 community` |
  | SQLite (`rusqlite::version()`) | `3.45.3` |
  | `rusqlite` | `=0.32.1` (`bundled-sqlcipher-vendored-openssl`) |
  | `libsqlite3-sys` | `0.30.1` |

  A vault written by one release must stay openable by the next. The pin is
  asserted on every PR by `crates/finance-kernel/tests/cross_version_vault.rs`
  — the *linked* engine versions must equal `docs/architecture/stack.md` — so an
  accidental engine bump fails the build instead of risking unopenable vaults.
  See the version-pin policy in `docs/architecture/stack.md` before changing these.
