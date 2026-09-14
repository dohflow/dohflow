# ADR 0067 — DohFlow rename policy: what renames and what stays

- **Status:** Accepted (2026-09-05). The policy was already decided in the
  notes of beads `fkt5.2` and `fkt5.3`; this ADR records it as its own
  decision so the rename can proceed without waiting on ADR 0062.
- **Decision:** **Humans see DohFlow; machines see `personal-cfo`.** Every
  string a user reads renames. Every identifier a filesystem, operating
  system, package registry, build tool, or tracker addresses stays.
- **Bead:** `personal-cfo-4d8.28.1` (this decision); executed by
  `personal-cfo-fkt5.3` (the rename); parent `personal-cfo-4d8.28`
  (launch polish round 1).
- **Related:** ADR 0062 (public-repo fork mechanics, pending — the rename
  policy was split out of it), ADR 0066 (business model, reserved by bead
  `915.4`; the number gap is intentional), ADR 0002 (local encrypted vault —
  amended by addendum, see rule 3), ADR 0042 (multi-vault registry, which
  lives in the app-data directory this ADR protects), ADR 0063 (typeface
  pairing), `docs/product/brand-direction.md`, `TRADEMARK.md`.

## Context

The public name is **DohFlow** (`docs/product/brand-direction.md`, owner
decision 2026-09-01). The code, bundle, documentation, and scripts still
say **Personal CFO**: `productName` and the window title in
`apps/desktop/src-tauri/tauri.conf.json`, the `<title>` in
`apps/desktop/index.html`, five product source files and three tests under
`apps/desktop/src`, the no-reset warning constant in `crates/vault-crypto`,
`README.md`, `CHANGELOG.md`, `docs/agent/PROJECT_PROFILE.md`, the three
build scripts that address `Personal CFO.app` by path, and the in-app
software-update relaunch, which hardcodes the same path.

The rename was scoped inside ADR 0062 together with the fork mechanics.
Those two decisions have different blockers. Fork mechanics genuinely
depend on the owner (author identity, exposure decisions) and on the
history rewrite (`fkt5.11`, ADR 0064). The rename depends on nothing but
a written policy — and three launch items wait on it: the screenshot set
(`n76x.13`), the v0.1.0 release checklist (`867.1.3`), and the updater
(`867.1.2`). Owner priority #1 on 2026-09-05 is a screenshot-ready build,
so the policy is recorded here and the rename proceeds.

## Decision

### Renames (user-facing)

| Surface | Today | After | Notes |
|---|---|---|---|
| `productName` (`tauri.conf.json`) | `Personal CFO` | `DohFlow` | Drives the bundle name (`DohFlow.app`), the macOS menu-bar app menu, Finder, and the release artifact name (`DohFlow_0.1.0_aarch64.dmg`; the updater archive `DohFlow.app.tar.gz` appears once `867.1.2` ships). No other change is needed for artifacts. |
| `description` in `apps/desktop/src-tauri/Cargo.toml` | `Personal CFO …` | `DohFlow …` | Tauri uses it as the bundle's short description (Finder "Get Info"). Crate doc-comments that name the product follow the same rule. |
| Window title (`tauri.conf.json` `app.windows[0].title`) and `<title>` in `apps/desktop/index.html` | `Personal CFO` | `DohFlow` | |
| In-app strings (`apps/desktop/src`) | `Personal CFO` | `DohFlow` | Product sites: `vault/screens/Sidebar.tsx`, `vault/screens/VaultPickerScreen.tsx`, `settings/VaultHealthCard.tsx`, `backup/RestoreFromBackup.tsx`, `backup/BackupView.tsx`. Tests and comments: `Sidebar.test.tsx`, `NoResetWarning.test.tsx`, `copy-review.test.ts`, `styles/globals.css`. The About card follows the rule when `n76x.18` lands. Snapshot and copy-review tests are updated, never weakened. |
| No-reset warning (`crates/vault-crypto/src/lib.rs`, served through `ipc/commands.rs`) | `Personal CFO` | `DohFlow` | Its test asserts the text verbatim against ADR 0002; see rule 3 for the carve-out. |
| In-app relaunch after a software update (`apps/desktop/src-tauri/src/update.rs`, `relaunch()`) | `open '/Applications/Personal CFO.app'` | `DohFlow.app` | Without this, Settings › Software update relaunches the stale bundle after the rename. |
| Build scripts | `Personal CFO.app` path and messages in `scripts/build-app.sh`, `scripts/release.sh`, `scripts/update-app.sh` | `DohFlow.app` | `update-app.sh` keeps replacing its own exactly-named bundle as it does today (`rm -rf` + `ditto`, scoped to `/Applications/DohFlow.app`); it never touches `Personal CFO.app`, and if that bundle still exists it prints an instruction to move it to the Trash. Both bundles share the same identifier and therefore the same data directory, so the stale one is harmless but confusing. |
| Prose in `README.md`, `CONTRIBUTING.md`, `SECURITY.md`, `docs/agent/PROJECT_PROFILE.md` | `Personal CFO` | `DohFlow` | The README title, tagline, and the non-advice section. |
| `CHANGELOG.md` | header prose | `DohFlow` | Existing dated entries are history and keep their wording. |
| Repository URL fields (`package.json`, Cargo manifests) | none / placeholder | `github.com/dohflow/...` | Only once the org slug exists (`fkt5.1`). Not part of the first rename PR. |

### Stays (machine-facing)

| Surface | Value | Why it must not change |
|---|---|---|
| Bundle identifier | `ai.personalcfo.desktop` | `app.path().app_data_dir()` derives the data directory from it (`apps/desktop/src-tauri/src/lib.rs`), so every vault, the multi-vault registry (ADR 0042), and backups live under `~/Library/Application Support/ai.personalcfo.desktop`. Changing it orphans every existing vault and resets macOS privacy (TCC) grants. Today nothing else is keyed on it: signing and notarization use the Team ID (`APPLE_SIGNING_IDENTITY` in `scripts/release.sh`) and there is no keychain use; once the updater ships (`867.1.2`) its identity is keyed on it too, which is one more reason to freeze it now. Precedent: Obsidian ships as `md.obsidian`, Slack as `com.tinyspeck.slackmacgap`; users never read identifiers. |
| App-data directory and vault paths | derived from the identifier | Same reason. The rename is verified by installing the renamed build over the existing data directory and opening the dogfood vault unchanged. |
| On-disk magic bytes and extensions | `PCFOVLT` (`crates/vault-crypto/src/envelope.rs`), `PCFOBK` and `.pcfobk` (`crates/finance-kernel/src/backup.rs`) | Format identifiers written into every vault and backup; changing them is a format migration, not a rename. They are documented as-is in the vault-format spec (`klr.4`). |
| Rust crate names (`personal-cfo-desktop`, `app_lib`, `core-money`, `core-ledger`, `vault-crypto`, `db-worker`, …) | unchanged | Internal. Renaming churns `Cargo.lock`, CI caches, docs, and beads for zero user value. |
| npm package names (`@personal-cfo/desktop`, root `personal-cfo`) | unchanged | Internal, same reason. |
| `PCFO_*` environment variables (`PCFO_BUILD_CHANNEL`, `PCFO_BUILD_ID`, `PCFO_BUILD_TIME`, `PCFO_GIT_COMMIT`, `PCFO_GIT_DIRTY`, `PCFO_REPO_ROOT`, `PCFO_SEED_ROOT`) and `release.env` | unchanged | Build tooling and the owner's local environment. |
| Bead prefix `personal-cfo-` | unchanged | About 1,300 beads cross-reference each other by id. |
| Historical text: earlier ADRs, dated changelog entries, closed beads, git history | unchanged | History is not rewritten for a name (consistent with ADR 0062's concern about stale SHA references). |
| Repository name and owner | decided in ADR 0062 and `fkt5.1` | Out of scope here. |

Only identifiers that already exist and carry data or history are frozen.
Machine identifiers created after this ADR use the new name — the launchd
label `app.dohflow.mirror-backup` (`scripts/launchd/`) already does — so
the split is "old path-bearing ids stay, new ids are DohFlow", not "machine
names are Personal CFO forever".

### Rules that travel with the name

1. **The wordmark is never rendered from a font.** In logo contexts the word
   is the SVG-path wordmark from `docs/product/brand/wordmark.svg`
   (`n76x.3` tool decision). Plain-text "DohFlow" in titles, prose, and
   dialogs is fine and expected.
2. **Pronunciation is "doe-flow"** wherever documentation explains the
   name (`n76x.6` copy rule).
3. **Where "Personal CFO" may still appear:** earlier ADRs, changelog
   entries dated before the rename, dated planning and research documents
   (for example the original project plan that code and beads cite by
   section), closed beads, and git history. One carve-out: `crates/vault-crypto/tests/no_reset_warning.rs` asserts the
   no-reset warning text verbatim against ADR 0002, so ADR 0002 gains a
   dated addendum carrying the renamed text (the original wording stays
   above it as history). The acceptance grep for `fkt5.3` —
   `grep -rn 'Personal CFO' apps crates scripts docs README.md
   CONTRIBUTING.md SECURITY.md CHANGELOG.md Cargo.toml` — must return only
   those historical mentions.
4. **A guard test pins both halves of the rule:** `productName` equals
   `DohFlow` and `identifier` equals `ai.personalcfo.desktop`, read from
   `tauri.conf.json`. It extends `apps/desktop/src-tauri/tests/build_identity.rs`,
   which already reads that file. Lands with `fkt5.3`.
5. **Tests are updated, not weakened.** Copy-review, palette-guard, and
   snapshot tests that carry the old name get the new name; no assertion
   is removed to make the rename pass.

## Consequences

- **Positive.** The rename can ship now, on its own PR, and unblocks the
  screenshot set, the release checklist, and the updater without touching
  ADR 0062's owner-gated questions.
- **Positive.** Every existing vault, on the dogfooding Mac and on any
  future user's machine, keeps working across the rename because nothing
  path-bearing changes.
- **Negative, accepted.** The identifier says `personalcfo` forever: in
  `~/Library` paths, in `codesign -dv` output, and in the updater
  configuration. Users do not read identifiers, and the precedents above
  show product names and identifiers routinely diverge.
- **Negative, accepted.** Until the owner trashes the old bundle, the
  dogfooding Mac carries both `Personal CFO.app` and `DohFlow.app`, which
  open the same data directory. `update-app.sh` says so explicitly.

## Rejected alternatives

- **Change the identifier to `app.dohflow.desktop` with a data-directory
  migration.** Real risk to real vaults, resets TCC grants, and
  complicates the signing and updater identity, all for a string nobody
  reads. Rejected.
- **Rename crates and packages to `dohflow-*`.** Repo-wide churn with no
  user-visible effect. Rejected for now; see "Revisit if".
- **Wait for ADR 0062.** The fork mechanics depend on the owner and on the
  history rewrite; the rename does not. Coupling them delayed the
  screenshot-ready build by weeks for no reason. Rejected.

## Revisit if

- A core crate or package is ever published to crates.io or npm. At that
  point its name becomes user-facing and should be `dohflow-*` (`n76x.8`
  already reserves the namespaces).
- ADR 0062 chooses a fresh-repository option and the owner wants the new
  history to carry DohFlow-named packages from its first commit.

## Implementation notes

- `fkt5.3` executes the "Renames" table in one PR (plan mode, per its
  bead), then verifies the dogfood vault opens unchanged and that a
  release build produces `DohFlow`-named artifacts.
- The legacy app icon (`4d8.28.2`) and the in-app brand placement
  (`4d8.28.3`) follow in their own PRs; the icon does not depend on the
  rename, the brand placement does.
