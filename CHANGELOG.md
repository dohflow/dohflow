# Changelog

All notable changes to DohFlow are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/). This project is pre-1.0
and releases are not yet published; the **Unreleased** section is the running
record until the first tagged release.

## [Unreleased]

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
