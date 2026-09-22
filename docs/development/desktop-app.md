# Building & running the desktop app

Two ways to run DohFlow locally: a **dev server** (for development, with hot
reload) and a **built `.app`** (for dogfooding — double-click to launch, no
terminal). This page covers both. The signed/notarized **release** pipeline
(DMG, auto-update) is separate — bead `personal-cfo-867.1`.

## Prerequisites

`tauri` needs the **Rust toolchain on your `PATH`**. A bare interactive shell
often doesn't have it, which shows up as:

```
failed to run command `cargo metadata ...`: No such file or directory
```

Fix it for the session (or add to your `~/.zshrc`):

```bash
export PATH="$HOME/.cargo/bin:/opt/homebrew/opt/rustup/bin:/opt/homebrew/bin:$PATH"
```

The `scripts/build-app.sh` wrapper below sets this for you.

## Dogfooding: a double-clickable `.app`

Build an unsigned, local `DohFlow.app`:

```bash
./scripts/build-app.sh           # optimized release build (default)
./scripts/build-app.sh --debug   # faster build, larger / slower app
```

or, with `cargo` already on your `PATH`:

```bash
pnpm -C apps/desktop app          # = tauri build --bundles app
```

The app lands at:

```
apps/desktop/src-tauri/target/release/bundle/macos/DohFlow.app
```

**Run it:** double-click it, run `open "…/DohFlow.app"`, or drag it into
**/Applications** so it's in Spotlight/Launchpad.

**First launch is unsigned** — macOS Gatekeeper will say "unidentified
developer." Right-click the app and choose **Open** once to allow it; after that
it launches normally. (Proper signing + notarization is bead `personal-cfo-867.1`.)

**Rebuild after pulling changes:** re-run `./scripts/build-app.sh`. The built app
is a snapshot; it does not update itself.

### Update in one command

After an update is shipped (merged to `main`), refresh your installed app in a
single step:

```bash
./scripts/update-app.sh        # or: pnpm update-app
```

It fast-forward-pulls the latest code (only when your tree is clean — it never
touches local changes), rebuilds the `.app`, and installs it into **/Applications**,
replacing the old copy. Flags: `--no-pull` (build the current checkout without
pulling), `--debug` (faster build). It stays **local + offline** — this is not a
network auto-updater (that needs signing + a release feed and would break the
app's no-network posture; the signed release pipeline is bead `personal-cfo-867.1`).

Your vault data lives outside the app bundle (at
`~/Library/Application Support/ai.personalcfo.desktop/`), so rebuilding or
replacing the `.app` never touches your vaults.

## Development: the dev server

For active development (hot reload of the frontend, watch-rebuild of the Rust
side):

```bash
pnpm -C apps/desktop tauri dev     # PATH must include cargo (see above)
```

This opens a native window backed by the Vite dev server on `localhost:1420`.
Use this while changing code; use the built `.app` for everyday dogfooding.

### Dev and release vaults are separate directories (ADR 0070)

`pnpm tauri dev` (channel `dev`) and `/Applications/DohFlow.app` (channel
`release`) never open the same vault directory. A dev build resolves its data
directory to a sibling of the release one, suffixed `-dev`:

```
~/Library/Application Support/ai.personalcfo.desktop        # release
~/Library/Application Support/ai.personalcfo.desktop-dev    # dev
```

Point a dev build at a specific fixture directory instead of the default
`-dev` sibling with `PCFO_DATA_DIR`:

```bash
PCFO_DATA_DIR=/tmp/my-fixture-vault pnpm -C apps/desktop tauri dev
```

`PCFO_DATA_DIR` is honored **only** in a dev-channel build — a release build
(`cargo build --release`, and therefore every signed release artifact) never
reads it, so there is no environment variable that can redirect the shipped
app. A fresh dev checkout starts from an empty vault the first time; it does
not currently seed the Polish Demo fixture automatically (tracked separately,
`personal-cfo-wmsw2`).

**One unlocked instance of a given vault at a time.** `DbWorker` holds an OS
advisory lock on `<vault>.runner.lock` for the whole unlocked session. A second
`tauri dev` process (or `cargo test --test seed_polish_vault -- --ignored`
racing a running `tauri dev`) cannot unlock the same vault; it receives a
plain "vault is already open" error before it can migrate or run durable-job
recovery. The lock is released when the first instance locks, exits, or
crashes, so a later unlock can recover safely. This protects a shared vault,
but does not coordinate two processes that are only classifying an empty or
locked directory, and it does not replace the broader one-build-at-a-time rule
in `docs/agent/WORKFLOW_ROLES.md`.

**Time Machine:** the dev directory holds disposable fixture/test data, not
your real vault, so it's excluded from Time Machine rather than backed up
alongside it. Run once per machine:

```bash
tmutil addexclusion "$HOME/Library/Application Support/ai.personalcfo.desktop-dev"
tmutil isexcluded "$HOME/Library/Application Support/ai.personalcfo.desktop-dev"   # verify
```

The app's own encrypted backup/restore (Settings → Vault) already can't cross
the boundary: it always operates on whichever vault is active for the running
process, so a dev build's backups land in the dev directory and a release
build's in the release directory — never mixed.

## CI: the Intel build + launch smoke (`personal-cfo-rr0lm`)

Every push to `main` (and manual `workflow_dispatch`) runs a job on a real,
native `macos-15-intel` GitHub runner: `intel-smoke` in `.github/workflows/ci.yml`.
It builds the universal binary (`--target universal-apple-darwin`, ADR 0072),
confirms `lipo -archs` reports both `arm64` and `x86_64`, then launches the
binary headlessly with `PCFO_SMOKE_TEST_EXIT=1` — an opt-in env var that makes
`setup()` (vault-registry bootstrap, ADR 0070's data-dir resolution) run to
completion and then exit `0` on its own, with no display, no vault, and no
network required.

**What this proves:** the x86_64 slice actually compiles and its Rust startup
logic runs without panicking, on real Intel hardware, without needing the
owner to own an Intel Mac.

**What this does NOT prove** — deliberately out of scope, no secrets are
available to this job (D9a; this is not release signing, D9b, which stays on
the local build Mac per `scripts/release.sh`):

- **No Gatekeeper check.** The binary is unsigned and unnotarized; a real user
  downloading an unsigned build would see the "unidentified developer" wall
  this job never triggers or clears.
- **No notarization.** `xcrun notarytool`/`stapler` never run here.
- **No signed, distributable artifact.** `--bundles app` only, no DMG, no
  updater archive/signature (`createUpdaterArtifacts: false` for this job).
- **No real UI/webview interaction.** The smoke exit fires before the window
  ever paints; this is not a UI test.

The real signing/notarization/Gatekeeper proof is the owner's manual smoke
test against a real release draft — see
[`docs/operations/release-checklist.md`](../operations/release-checklist.md)
step 5, which now also gates on this CI job passing for the release commit.
