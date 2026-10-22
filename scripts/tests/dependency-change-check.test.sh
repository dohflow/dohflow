#!/usr/bin/env bash
#
# Tests for scripts/dependency-change-check.sh (personal-cfo-md8h2.14): which
# changed paths make the advisory audits required. Pure, no network.

# The `check && pass || fail` lines are if-then-else on purpose: pass() always
# returns 0, so fail() runs only when the check failed.
# shellcheck disable=SC2015
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
SCRIPT="$REPO_ROOT/scripts/dependency-change-check.sh"
[ -x "$SCRIPT" ] || { echo "missing or not executable: $SCRIPT" >&2; exit 1; }

FAILURES=0
CASES=0
pass() { echo "    ok   — $*"; }
fail() { echo "    FAIL — $*" >&2; FAILURES=$((FAILURES + 1)); }
case_start() { CASES=$((CASES + 1)); echo; echo "[$CASES] $*"; }

# expect_required <path>...  /  expect_not_required <path>...
expect_required() {
  local out rc
  out="$(printf '%s\n' "$@" | "$SCRIPT")"; rc=$?
  if [ "$rc" = 0 ] && [ -n "$out" ]; then pass "required: $*"; else fail "should be required: $* (rc=$rc)"; fi
}
expect_not_required() {
  local out rc
  out="$(printf '%s\n' "$@" | "$SCRIPT")"; rc=$?
  if [ "$rc" = 1 ] && [ -z "$out" ]; then pass "not required: $*"; else fail "should NOT be required: $* (rc=$rc, out=$out)"; fi
}

case_start "dependency and audit-config paths make the audits required"
expect_required Cargo.toml
expect_required Cargo.lock
expect_required apps/desktop/src-tauri/Cargo.toml
expect_required apps/desktop/src-tauri/Cargo.lock
expect_required crates/db-worker/Cargo.toml
expect_required package.json
expect_required apps/desktop/package.json
expect_required pnpm-lock.yaml
expect_required pnpm-workspace.yaml
expect_required deny.toml
expect_required apps/desktop/src-tauri/deny.toml
expect_required audit.toml
expect_required .cargo/audit.toml
expect_required scripts/cargo-audit.py
expect_required scripts/tests/cargo-audit.test.sh
expect_required scripts/dependency-change-check.sh
expect_required scripts/advisory-scans-required.sh
expect_required scripts/tests/advisory-scans-required.test.sh
expect_required scripts/tests/dependency-change-check.test.sh
expect_required .npmrc
expect_required apps/desktop/.npmrc

case_start "other paths do not (docs, source, workflows, look-alikes)"
expect_not_required README.md docs/architecture/vault-format.md
expect_not_required crates/db-worker/src/lib.rs
expect_not_required apps/desktop/src/App.tsx
expect_not_required .github/workflows/ci.yml
expect_not_required scripts/value-scan.sh
expect_not_required docs/Cargo.toml.md
expect_not_required my-Cargo.toml
expect_not_required crates/x/not-package.json.txt

case_start "a mixed change is required if any one path matches, and prints the matches"
out="$(printf '%s\n' README.md crates/core-money/Cargo.toml docs/a.md | "$SCRIPT")"; rc=$?
[ "$rc" = 0 ] && pass "exits 0" || fail "rc=$rc"
[ "$out" = "crates/core-money/Cargo.toml" ] && pass "prints only the matching path" || fail "printed: $out"

case_start "an empty change list is not required"
out="$(printf '' | "$SCRIPT")"; rc=$?
{ [ "$rc" = 1 ] && [ -z "$out" ]; } && pass "empty input -> not required" || fail "rc=$rc out=$out"

echo
if [ "$FAILURES" -eq 0 ]; then
  echo "PASS — $CASES cases, 0 failures"
  exit 0
fi
echo "FAIL — $CASES cases, $FAILURES failed assertions" >&2
exit 1
