#!/usr/bin/env bash
#
# scripts/ci-disk.sh (personal-cfo-56w.7): `free` deletes directories, so the tests prove it refuses
# off a hosted Linux runner, removes exactly the listed toolchains and nothing else, and that `report`
# writes the evidence line a job summary needs.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
SCRIPT="$REPO_ROOT/scripts/ci-disk.sh"
CASES=0
FAILURES=0

check() {
  local label="$1"
  shift
  CASES=$((CASES + 1))
  if "$@"; then
    echo "    ok   — $label"
  else
    echo "    FAIL — $label" >&2
    FAILURES=$((FAILURES + 1))
  fi
}

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# 1. `free` refuses everywhere but a hosted Linux runner, and deletes nothing.
mkdir -p "$TMP/refuse/usr/share/dotnet"
refuses_without_runner() {
  env -u GITHUB_ACTIONS -u RUNNER_OS CI_DISK_ROOT="$TMP/refuse" CI_DISK_SUDO="" bash "$SCRIPT" free >/dev/null 2>&1
  [ $? -eq 2 ] && [ -d "$TMP/refuse/usr/share/dotnet" ]
}
check "free refuses outside GitHub Actions and deletes nothing" refuses_without_runner

refuses_on_macos_runner() {
  GITHUB_ACTIONS=true RUNNER_OS=macOS CI_DISK_ROOT="$TMP/refuse" CI_DISK_SUDO="" bash "$SCRIPT" free >/dev/null 2>&1
  [ $? -eq 2 ] && [ -d "$TMP/refuse/usr/share/dotnet" ]
}
check "free refuses on a non-Linux runner" refuses_on_macos_runner

# 2. On a Linux runner it removes exactly the listed toolchains.
ROOT="$TMP/runner"
mkdir -p \
  "$ROOT/usr/share/dotnet/sdk" \
  "$ROOT/usr/local/lib/android/sdk" \
  "$ROOT/opt/ghc/9.6" \
  "$ROOT/usr/local/.ghcup/bin" \
  "$ROOT/opt/hostedtoolcache/CodeQL/2.0" \
  "$ROOT/opt/hostedtoolcache/node/22" \
  "$ROOT/opt/hostedtoolcache/Python/3.12" \
  "$ROOT/usr/share/swift" \
  "$ROOT/usr/lib/jvm" \
  "$ROOT/home/runner/.rustup/toolchains/stable"
SUMMARY="$TMP/summary.md"
: >"$SUMMARY"
GITHUB_ACTIONS=true RUNNER_OS=Linux CI_DISK_ROOT="$ROOT" CI_DISK_SUDO="" GITHUB_STEP_SUMMARY="$SUMMARY" \
  bash "$SCRIPT" free >/dev/null 2>&1
free_status=$?
check "free succeeds on a Linux runner" [ "$free_status" -eq 0 ]
for gone in usr/share/dotnet usr/local/lib/android opt/ghc usr/local/.ghcup opt/hostedtoolcache/CodeQL usr/share/swift; do
  check "free removed /$gone" [ ! -e "$ROOT/$gone" ]
done
for kept in opt/hostedtoolcache/node/22 opt/hostedtoolcache/Python/3.12 usr/lib/jvm home/runner/.rustup/toolchains/stable; do
  check "free kept /$kept" [ -d "$ROOT/$kept" ]
done

# 3. The summary has the heading once and one line per checkpoint (before and after freeing).
check "the summary names the evidence table" grep -q "Runner disk (personal-cfo-56w.7)" "$SUMMARY"
check "the summary has a before-freeing line" grep -q "| before freeing |" "$SUMMARY"
check "the summary has an after-freeing line" grep -q "| after freeing |" "$SUMMARY"
heading_count=$(grep -c "Runner disk (personal-cfo-56w.7)" "$SUMMARY")
check "the heading is written once" [ "$heading_count" -eq 1 ]

# 4. report adds a line and reads only.
GITHUB_STEP_SUMMARY="$SUMMARY" bash "$SCRIPT" report "after clippy" >/dev/null 2>&1
check "report adds its checkpoint" grep -q "| after clippy |" "$SUMMARY"
check "report removed nothing" [ -d "$ROOT/opt/hostedtoolcache/node/22" ]

# 5. A bad subcommand is a usage error.
bash "$SCRIPT" bogus >/dev/null 2>&1
check "an unknown subcommand is a usage error" [ $? -eq 64 ]

echo "ci-disk: $CASES cases, $FAILURES failed"
[ "$FAILURES" -eq 0 ]
