#!/usr/bin/env bash
#
# One-command local update for the dogfooding "DohFlow.app" (personal-cfo-1ik.2).
#
# Collapses the manual update dance — git pull, ./scripts/build-app.sh, drag the
# .app into /Applications — into a single command, so shipping an update is one step.
#
# Local + offline ONLY: this rebuilds from your checkout and installs it yourself.
# It is deliberately NOT a network in-app auto-updater — that needs code signing and
# a release feed and would cut against the app's offline-first, no-network posture
# (ADR 0003). The signed/notarized release pipeline is bead personal-cfo-867.1.
#
# Usage:
#   ./scripts/update-app.sh             # pull (when clean) + release build + install
#   ./scripts/update-app.sh --no-pull   # skip the git pull; build the current tree
#   ./scripts/update-app.sh --debug     # faster debug build
set -euo pipefail

# Make the Rust toolchain + pnpm reachable even from a minimal shell (same as build-app.sh).
export PATH="$HOME/.cargo/bin:/opt/homebrew/opt/rustup/bin:/opt/homebrew/bin:$PATH"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# Force build.rs to re-stamp the build provenance (personal-cfo-4d8.27.3.1). Cargo only
# re-runs it on its declared triggers, and editing a tracked file is not one of them —
# without this the installed app could report the PREVIOUS commit as clean.
export PCFO_BUILD_ID="$(date -u +%Y%m%dT%H%M%SZ)-$$"
cd "$repo_root"

pull=1
debug=0
profile_dir="release"
for arg in "$@"; do
  case "$arg" in
    --no-pull) pull=0 ;;
    --debug) debug=1; profile_dir="debug" ;;
    -h | --help)
      # Print the leading comment block (after the shebang), stripping the "# " prefix.
      awk 'NR>1 && /^#/ { sub(/^# ?/, ""); print; next } NR>1 { exit }' "${BASH_SOURCE[0]}"
      exit 0
      ;;
    *)
      echo "unknown option: $arg (try --help)" >&2
      exit 2
      ;;
  esac
done

# Pull latest — but never touch a dirty tree (non-destructive). Fast-forward only, so a
# diverged branch fails cleanly instead of creating a merge. Only TRACKED changes are
# considered dirty; untracked files (e.g. a stray SESSION_HANDOFF.md) don't block a ff-only pull.
if [[ "$pull" == 1 ]]; then
  if [[ -n "$(git status --porcelain --untracked-files=no)" ]]; then
    echo "Working tree has local changes — skipping the pull and building what you have."
    echo "(Commit or stash first for the latest shipped code, or pass --no-pull to silence this.)"
  else
    branch="$(git rev-parse --abbrev-ref HEAD)"
    echo "Pulling latest ${branch}…"
    git pull --ff-only
  fi
fi

# Build the .app (reuse the existing builder, which sets PATH + verifies cargo). Called
# without an array so it stays safe under `set -u` on macOS's stock bash 3.2.
if [[ "$debug" == 1 ]]; then
  "$repo_root/scripts/build-app.sh" --debug
else
  "$repo_root/scripts/build-app.sh"
fi

app="$repo_root/apps/desktop/src-tauri/target/${profile_dir}/bundle/macos/DohFlow.app"
dest="/Applications/DohFlow.app"

if [[ ! -d "$app" ]]; then
  echo "error: built app not found at: $app" >&2
  exit 1
fi

echo
echo "Installing to ${dest}…"
# Replace only our own, exactly-named bundle at the known Applications path.
if [[ "$dest" == "/Applications/DohFlow.app" && -d "$dest" ]]; then
  rm -rf "$dest"
fi
ditto "$app" "$dest"

# The pre-rename bundle (ADR 0067) is never touched here. It shares this bundle's
# identifier, so both open the same data directory — harmless, but confusing.
legacy="/Applications/Personal CFO.app"
if [[ -d "$legacy" ]]; then
  echo "note: the pre-rename bundle \"$legacy\" is still installed — move it to the Trash (DohFlow.app opens the same vaults)."
fi

echo "Updated. Launch it from Applications (or: open \"$dest\")."
echo "If macOS blocks the first launch of a new build (unsigned), right-click the app → Open."

# What did we actually install? (personal-cfo-4d8.27.3.4) — the whole point of the
# rebuild loop is knowing the running app matches this commit.
built_commit="$(git -C "$repo_root" rev-parse --short HEAD 2>/dev/null || echo unknown)"
built_dirty=""
if [[ -n "$(git -C "$repo_root" status --porcelain --untracked-files=no 2>/dev/null)" ]]; then
  built_dirty=" (with uncommitted changes)"
fi
echo "Installed build: ${built_commit}${built_dirty}"
echo "The app shows this in the sidebar footer and in Settings › Software update."
