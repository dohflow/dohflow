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
