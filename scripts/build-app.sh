#!/usr/bin/env bash
#
# Build the local "DohFlow.app" for dogfooding — an unsigned, double-clickable
# macOS app you can launch without a terminal (personal-cfo-1ik.1).
#
# Why a wrapper: `tauri build` needs the Rust toolchain (cargo) on PATH, which a
# bare interactive shell often doesn't have. This prepends the usual locations so
# the build "just works." For the signed/notarized release pipeline see bead
# personal-cfo-867.1.
#
# Usage:
#   ./scripts/build-app.sh            # optimized release build (default)
#   ./scripts/build-app.sh --debug    # faster build, larger/slower app
set -euo pipefail

# Make the Rust toolchain + pnpm reachable even from a minimal shell.
export PATH="$HOME/.cargo/bin:/opt/homebrew/opt/rustup/bin:/opt/homebrew/bin:$PATH"

if ! command -v cargo >/dev/null 2>&1; then
  echo "error: 'cargo' not found on PATH. Install Rust (https://rustup.rs) or" >&2
  echo "       Homebrew's rustup, then re-run." >&2
  exit 1
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# Force build.rs to re-stamp the build provenance (personal-cfo-4d8.27.3.1). Cargo only
# re-runs it on its declared triggers, and editing a tracked file is not one of them —
# without this the installed app could report the PREVIOUS commit as clean.
export PCFO_BUILD_ID="$(date -u +%Y%m%dT%H%M%SZ)-$$"
cd "$repo_root/apps/desktop"

# --bundles app builds just the .app (skips the slower DMG packaging).
debug_flag=""
profile_dir="release"
if [[ "${1:-}" == "--debug" ]]; then
  debug_flag="--debug"
  profile_dir="debug"
fi

echo "Building DohFlow.app (${profile_dir})…"
pnpm tauri build ${debug_flag} --bundles app

app="$repo_root/apps/desktop/src-tauri/target/${profile_dir}/bundle/macos/DohFlow.app"
echo
if [[ -d "$app" ]]; then
  echo "Built: $app"
  echo "Run it:   open \"$app\""
  echo "Install:  drag it into /Applications"
  echo "First launch is unsigned — right-click the app and choose Open to clear Gatekeeper."
else
  echo "warning: build finished but the .app was not found at the expected path:" >&2
  echo "  $app" >&2
  exit 1
fi

# What did we actually build? (personal-cfo-4d8.27.3.4) — the bundle stamps this commit
# into the binary, so the app can show it back to you.
built_commit="$(git -C "$repo_root" rev-parse --short HEAD 2>/dev/null || echo unknown)"
built_dirty=""
if [[ -n "$(git -C "$repo_root" status --porcelain --untracked-files=no 2>/dev/null)" ]]; then
  built_dirty=" (with uncommitted changes)"
fi
echo "Built from: ${built_commit}${built_dirty}"
