#!/usr/bin/env bash
#
# Runner disk, measured and freed, for the Linux CI jobs that build the Rust workspace or the desktop
# crate (personal-cfo-56w.7). The free hosted runner has a limited disk and the desktop crate builds
# several profiles (clippy, tests, the declared-MSRV check, the bindings binary, the custom-protocol
# binary), so the 'IPC codegen' job has failed with 'No space left on device'.
#
#   ci-disk.sh report <label>   print free disk and the largest directories, and add one line to the
#                               job summary: the evidence of how much room each job had at that point
#   ci-disk.sh free             remove the preinstalled toolchains no job of ours uses (.NET, the
#                               Android SDK, GHC, CodeQL, Swift, cached docker images), then report
#
# `free` deletes directories, so it refuses to run anywhere but a GitHub-hosted Linux runner (the
# GITHUB_ACTIONS and RUNNER_OS variables the runner sets). `report` only reads. Both honour
# CI_DISK_ROOT (a prefix for every path; the tests use it) and CI_DISK_SUDO (the command that elevates;
# `sudo` by default, empty in the tests).

set -uo pipefail

ROOT="${CI_DISK_ROOT:-}"
SUDO="${CI_DISK_SUDO-sudo}"

# Preinstalled on the ubuntu-24.04 image and used by none of our jobs. Node, Python and the Rust
# toolchain live elsewhere (the hosted tool cache, ~/.rustup) and are NOT touched: only CodeQL's
# bundle is removed from the tool cache.
REMOVABLE=(
  /usr/share/dotnet
  /usr/local/lib/android
  /opt/ghc
  /usr/local/.ghcup
  /opt/hostedtoolcache/CodeQL
  /usr/share/swift
)

# What is worth naming when a job is short of room.
INSPECT=(
  /usr/share/dotnet
  /usr/local/lib/android
  /opt/ghc
  /usr/local/.ghcup
  /opt/hostedtoolcache
  /usr/share/swift
  /var/lib/docker
  /var/cache/apt
  "$HOME/.cargo/registry"
  "$HOME/.rustup/toolchains"
  "$HOME/.cache"
  target
  apps/desktop/src-tauri/target
  apps/desktop/node_modules
)

free_kb() { df -Pk "${ROOT:-/}" | awk 'NR==2 {print $4}'; }
size_kb() { df -Pk "${ROOT:-/}" | awk 'NR==2 {print $2}'; }

gb() { awk -v kb="$1" 'BEGIN { printf "%.1f", kb / 1048576 }'; }

report() {
  local label="${1:-unlabeled}"
  echo "::group::disk: ${label}"
  df -h "${ROOT:-/}"
  echo "largest directories:"
  for dir in "${INSPECT[@]}"; do
    path="$dir"
    case "$dir" in /*) path="${ROOT}${dir}" ;; esac
    [ -e "$path" ] || continue
    du -xsh "$path" 2>/dev/null || true
  done | sort -rh
  echo "::endgroup::"
  if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
    if ! grep -q "Runner disk (personal-cfo-56w.7)" "$GITHUB_STEP_SUMMARY" 2>/dev/null; then
      printf '### Runner disk (personal-cfo-56w.7)\n\n| Checkpoint | Free (GB) | Of (GB) |\n|---|---|---|\n' >>"$GITHUB_STEP_SUMMARY"
    fi
    printf '| %s | %s | %s |\n' "$label" "$(gb "$(free_kb)")" "$(gb "$(size_kb)")" >>"$GITHUB_STEP_SUMMARY"
  fi
}

require_runner() {
  if [ "${GITHUB_ACTIONS:-}" != "true" ] || [ "${RUNNER_OS:-}" != "Linux" ]; then
    echo "ci-disk.sh $1 deletes directories and only runs on a GitHub-hosted Linux runner; refusing." >&2
    exit 2
  fi
}

free() {
  require_runner free
  report "before freeing"
  # Remove in parallel: the Android SDK alone is tens of thousands of files.
  for dir in "${REMOVABLE[@]}"; do
    if [ -e "${ROOT}${dir}" ]; then
      # shellcheck disable=SC2086 # SUDO is a command prefix, empty in the tests
      $SUDO rm -rf "${ROOT}${dir}" &
    fi
  done
  wait
  # Cached docker images are not used by any job here (no docker build in this workflow).
  if [ -z "${ROOT}" ] && command -v docker >/dev/null 2>&1; then
    # shellcheck disable=SC2086
    $SUDO docker image prune --all --force >/dev/null 2>&1 || true
  fi
  report "after freeing"
}

case "${1:-}" in
  report) report "${2:-unlabeled}" ;;
  free) free ;;
  *)
    echo "usage: ci-disk.sh report <label> | free" >&2
    exit 64
    ;;
esac
