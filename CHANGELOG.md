# Changelog

All notable changes to DohFlow are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/). This project is pre-1.0
and releases are not yet published; the **Unreleased** section is the running
record until the first tagged release.

## [Unreleased]

## [0.4.0] - 2026-11-12

### Added

- **LunchFlow is now a second bank-connection provider** (`personal-cfo-r2pow`,
  `personal-cfo-dto2j`, `personal-cfo-p3f7r`). **Settings → Connections** opens a
  picker of the available providers, SimpleFIN and LunchFlow. For each one it shows
  what it fetches, what it costs and who you pay, its terms and the date we last
  checked them, the countries it covers, and a plain statement of what the provider
  is and what you give it, before you paste a key or token. LunchFlow is a separate
  company that charges its own fee, paid to them; DohFlow does not bill it. DohFlow
  has a referral arrangement with LunchFlow and may earn a commission if you sign
  up through the link in the app, which does not change what LunchFlow charges you;
  the app says this where the link is. You create an API key in your own LunchFlow
  dashboard and paste it into DohFlow; your bank sign-in stays with LunchFlow. The
  key is stored encrypted in your vault like a SimpleFIN access URL, and the app
  only contacts LunchFlow once you have linked it. Everything still works with
  manual entry and file imports.
- **Link a recurring bill to the loan it pays** (`personal-cfo-6wk.22`,
  `personal-cfo-6wk.20`, `personal-cfo-6wk.23`). A bill that pays a loan can be
  linked to it so the forecast counts the payment once instead of twice. A loan's
  own payment is no longer offered as a new bill, adding a bill or a recurring
  item checks for the overlap, and Cash Flow and Bills warn when a bill of any
  type looks like a loan payment you already track.
- **Undo after removing** (`personal-cfo-egn67`, `personal-cfo-4d8.32`). Removing
  an income source, a bill or a recurring transfer shows a notice with Undo, and so
  does marking a bill paid on Cash Flow. A removed item is hidden, not erased
  (`personal-cfo-4d8.50`), and the Undo notice is the only way back, which the
  confirmation now says.
- **A bank connection that needs you to sign in again says so** (`personal-cfo-pxi.22`).
  The notice stays until you deal with it and is kept on this Mac only.
- **A shared dialog** (`personal-cfo-4d8.6.1`, `personal-cfo-4d8.6.2`): the scenario
  Apply and Delete confirmations, New loan and Create category keep focus inside the
  dialog, close with Escape, give focus back, and scroll within the window.

### Changed

- **Settings sections open and close the same way everywhere** and lay out in a grid
  that follows the window width (`personal-cfo-4d8.39.1`). Errors in Household,
  Comfort band, Backups, Vault health, Diagnostics, Software update, Vaults,
  General and Connections now stay visible in their section instead of vanishing
  (`personal-cfo-4d8.39.2`).
- **Local reads and writes no longer wait on the network** (`personal-cfo-g3m.34`):
  the state of your Mac's connectivity does not delay opening or saving.
- **A bank refresh you cancel stops at the fetch and commits nothing**; a refresh that
  already committed is never shown as cancelled (`personal-cfo-pxi.25`).
- The Debt terms table shows each row's money in that debt's own currency
  (`personal-cfo-6wk.25`).
- There is still no Sync in this app: nothing syncs your data between devices.
  Groundwork for a future feature continues behind the scenes and changes nothing you
  can do.

### Fixed

- **Saves tell the truth when something goes wrong** instead of leaving you to
  guess, and a retry does not create a second copy. The Add transaction and Add
  transfer forms are locked while a save is in flight (`personal-cfo-g3m.38`,
  `personal-cfo-g3m.39`); a multi-step save continues rather than duplicating
  (`personal-cfo-g3m.35`); the batch balance save and the account editor's balance
  save report each outcome (`personal-cfo-g3m.37`, `personal-cfo-g3m.40`); Pay card
  recovers and cannot be closed mid-payment (`personal-cfo-6wk.24`); Cover it
  cannot record a transfer twice (`personal-cfo-5ie.17`); an error is shown when a
  step after adding a bill fails (`personal-cfo-6wk.22`).
- **Reads say when they failed.** Cash Flow and the Dashboard no longer present a
  failed or expired read as an answer (`personal-cfo-5ie.18`); Activity shows one
  message with Retry (`personal-cfo-4d8.58`); attachments offer Retry and say when
  the outcome of an Attach or Remove is unknown (`personal-cfo-4d8.59`); a failed
  Debt terms read shows its message and leaves the totals blank instead of zero
  (`personal-cfo-twpo.8.2`); "Explain with transactions" shows Reconciled only
  once it is verified (`personal-cfo-4d8.64`).
- **Money Inbox** labels a row the ledger could not accept apart from a possible
  duplicate, with its reason, and offers Skip instead of Import anyway
  (`personal-cfo-pxi.14`); its stale-balance actions are honest and share the
  Accounts archive flow (`personal-cfo-4d8.57`).
- **Scenarios:** applying one replaces a base event only where they collide, so
  your manual future entries survive (`personal-cfo-5ie.13`); undoing an applied
  scenario follows what you edited since, so an edited item is never counted twice
  (`personal-cfo-5ie.12`); the conflict notice states the overlap
  (`personal-cfo-5ie.15`); the intro and Delete dialog tell the truth once a
  scenario is applied (`personal-cfo-5ie.19`); a refused row action shows its
  reason (`personal-cfo-5ie.20`).
- **Bulk tag** stops on a failed read, cannot run twice and refuses to overwrite a
  concurrent edit (`personal-cfo-5n4.10`); accept-all of low-confidence matches
  reports a truthful partial outcome (`personal-cfo-5n4.12`).
- A CSV amount written with a Unicode minus sign reads as negative, wherever a
  plain hyphen does (`personal-cfo-pxi.20`).
- A scheduled backup you cancel stops at its checkpoints and leaves the destination
  as it was (`personal-cfo-g3m.29`).
- **Accessibility and legibility:** badge labels and form-control borders meet AA
  contrast (`personal-cfo-4d8.35`); warning text in the light theme uses a darker ink
  (`personal-cfo-4d8.43`); a disabled button keeps a readable label
  (`personal-cfo-4d8.52`); the provenance badge holds contrast on hovered rows
  (`personal-cfo-4d8.56`); the account select points at the line that explains its
  state (`personal-cfo-pxi.28`); the Debt payoff result is announced to screen
  readers (`personal-cfo-6wk.26`); labels meant only for screen readers can no
  longer stretch the page (`personal-cfo-twpo.1`).

### Security

- Vault keys are handed to the encryption engine through its binary-key interface and
  are built in place, so no copy of a key is left on the stack; the encryption
  library's key schedule is scrubbed after use (`personal-cfo-7ie.17`,
  `personal-cfo-xzpra`, `personal-cfo-7ie.19`).
- Locking a vault checkpoints its write-ahead log and closes the connection
  (`personal-cfo-7ie.14`).
- Dependency security advisories in the TLS stack (`rustls-webpki`, RUSTSEC-2026-0381
  to 0383) and in a validation library (GHSA-mrhx-c7xc-m3w5) are resolved
  (`personal-cfo-md8h2.13`, `personal-cfo-md8h2.16`).

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

### Fixed

- **Reviewing a transaction in the Money Inbox card view could act on the
  wrong one** (`personal-cfo-bv6w2`, `personal-cfo-q8l5t`). The arrow-key
  shortcuts (Reviewed, Later) could briefly stay tied to the card you had just
  finished, so a fast key press right after the card changed could mark that
  previous card instead of the one on screen, or file an edit against it. The
  shortcuts are now re-bound in the same step that swaps the card, and a test
  fails if that ever regresses.

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
