# ADR 0070 — Dev vs. release vault data-directory separation

- **Status:** Accepted (2026-09-17)
- **Tier:** Public — pure engineering/architecture decision, no business content.
- **Bead:** `personal-cfo-he3xo`
- **Decider:** Owner, 2026-09-17, in chat with the implementation session
- **Related:** ADR 0067 (rename policy — freezes the bundle identifier
  `ai.personalcfo.desktop` that `app_data_dir()` derives from), ADR 0002
  (local encrypted vault model), ADR 0042 (multi-vault registry, which lives
  inside the app-data directory this ADR splits), ADR 0024 / 0024-A
  (encrypted backup/restore format), `personal-cfo-h93wf` (the incident that
  motivates this)

## Context

`apps/desktop/src-tauri/src/lib.rs` resolves the vault location with
`app.path().app_data_dir()?` (line 253), which derives from the bundle
identifier `ai.personalcfo.desktop` — frozen by ADR 0067 because every vault,
the multi-vault registry, and backups live under it. Nothing about that path
is channel-aware. `PCFO_BUILD_CHANNEL` already exists as a compile-time
constant (`apps/desktop/src-tauri/build.rs` sets it from the Cargo profile —
`release` profile ⇒ `"release"`, anything else, including `cargo run` and
`tauri dev` ⇒ `"dev"`, unless explicitly overridden) and already gates the
updater's behavior (`update.rs`), but it has no effect on storage today. So
`pnpm -C apps/desktop tauri dev`, a local debug build, and the installed
`/Applications/DohFlow.app` all open the same `vaults.json` registry and the
same vault files.

This has already caused a real incident: during the 2026-09-09 screenshot
session an unnoticed second instance mutated the demo vault mid-capture,
producing findings that looked like seed drift and cost a round of
investigation (`personal-cfo-h93wf`, closed by observation). The failure that
has not happened yet is the expensive one: a dev build running an
in-progress migration against the owner's real financial vault, which is now
in continuous use post-launch (AGENTS.md §15).

## Decision

**1. Mechanism: a new `PCFO_DATA_DIR` environment variable, read at runtime,
honored only when the compile-time `PCFO_BUILD_CHANNEL` constant is `"dev"`.**

This extends the existing `PCFO_*` env-var family (`PCFO_BUILD_CHANNEL`,
`PCFO_SEED_ROOT`, …) rather than introducing a new naming convention. A pure
function, independent of any running Tauri instance so it stays unit-testable,
resolves the directory:

```text
resolve_data_dir(channel: &str, app_data_dir: PathBuf, override_dir: Option<PathBuf>) -> PathBuf
  if channel != "dev":
      return app_data_dir                      // override is never even consulted
  if let Some(dir) = override_dir:
      return dir                                // explicit dev override wins
  return sibling of app_data_dir named "<last-component>-dev"
      // e.g. .../Application Support/ai.personalcfo.desktop-dev
```

The caller passes `update::BUILD_CHANNEL`, `app.path().app_data_dir()?`, and
`std::env::var("PCFO_DATA_DIR").ok().map(PathBuf::from)`.

Two options considered and rejected:

- **A channel-derived suffix as the *only* mechanism (no override).** Rejected
  as the sole answer because a fixed `-dev` sibling gives up the ability to
  point a dev build at an arbitrary fixture directory (useful for tests and
  one-off reproductions) — but adopted as the **fallback default** so a fresh
  dev checkout is safe with zero configuration, which is the more important
  property day to day.
- **A separate bundle identifier for dev builds.** Cleanest in isolation, but
  interacts with ADR 0067's freeze (a second identifier means a second
  Keychain access group, a second TCC prompt, and separate code-signing
  considerations) for a problem that a same-identifier, path-level split
  already solves. Not adopted.

**2. The release build ignores any override, unconditionally.** Because
`PCFO_BUILD_CHANNEL` is a compile-time `env!()` constant baked into the
binary (not read at runtime), a release binary's `channel` argument is always
the literal `"release"` — the `if channel != "dev"` branch above returns
`app_data_dir` unchanged without ever calling `std::env::var("PCFO_DATA_DIR")`.
There is no runtime code path in a release binary that consults the
environment for this purpose. This is asserted directly by a test that calls
`resolve_data_dir("release", base, Some(some_other_dir))` and checks the
result is `base`, not the override.

**3. Seeding the dev directory with fixture data is out of scope for this
ADR.** The Polish Demo vault fixture (`docs/agent/demo-vault.md`) could
plausibly auto-seed on first dev run, but doing so requires extracting the
seeding logic out of `apps/desktop/src-tauri/tests/seed_polish_vault.rs` (a
test-only target, not linked into the app binary) into something callable at
app startup — a larger, separable change. Tracked as a follow-up bead rather
than folded in here.

**4. Detecting concurrent instances (a lock file or similar) is out of scope
for this ADR.** It would have caught `h93wf` outright and is worth building,
but it is a distinct mechanism from directory separation and ships
independently. Tracked as a follow-up bead.

**5. Backup/restore interaction.** The app's own backup/restore commands
operate on whichever vault is active in `AppState`, which is already
correctly scoped to the resolved directory by decision 1 — a dev build's
backup/export commands can only ever touch files under the dev directory,
never the release one, satisfied by construction with no extra code. The
remaining gap is OS-level: the dev directory should not consume Time Machine
space or be mistaken for the real vault's backup. There is no existing
scripted `tmutil` usage in this repo to extend (it was previously only run
ad hoc, e.g. `personal-cfo-r36ck` step 8's `tmutil isexcluded` verification),
so this ADR decides a **documented, one-time local step** rather than new
automation: `tmutil addexclusion "<dev data directory>"`, run once per
machine and recorded in `docs/development/desktop-app.md`, verified with
`tmutil isexcluded`. No scheduled-backup-job interaction exists yet
(`personal-cfo-8qh` is unbuilt), so there is nothing else to exclude it from
today.

## Consequences

- **Positive.** A dev build can never again write to the owner's real vault,
  closing the gap that caused `h93wf` and that has not yet caused a more
  expensive incident.
- **Positive.** No change to the frozen bundle identifier, no new signing or
  TCC surface, no change to the release build's behavior or code path.
- **Positive.** The mechanism is a pure, injectable function — testable
  without a running Tauri instance or a second compiled binary.
- **Negative / accepted.** A fresh dev checkout's first run starts from an
  empty vault, not a pre-seeded one, until the seeding follow-up bead lands.
  This matches today's actual first-run experience for a brand-new install,
  so it is not a regression.
- **Negative / accepted.** Nothing stops two `tauri dev` processes from
  colliding with each other inside the dev directory; only the dev-vs-release
  boundary is closed here. Tracked as a follow-up (decision 4).
- **Follow-up beads to file once this ADR is Accepted:** auto-seed the dev
  directory with the Polish Demo vault fixture; concurrent-instance
  detection/locking for the dev profile.
