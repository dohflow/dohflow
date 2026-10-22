#!/usr/bin/env bash
#
# Tests for scripts/advisory-scans-required.sh (personal-cfo-md8h2.14): the decision is
# correct, and it FAILS CLOSED when the checker is missing, broken or crashes (review
# finding: a change that deleted the checker while touching Cargo.lock used to turn the
# audits off). Pure, no network.

# The `check && pass || fail` lines are if-then-else on purpose: pass() always
# returns 0, so fail() runs only when the check failed.
# shellcheck disable=SC2015
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
SCRIPT="$REPO_ROOT/scripts/advisory-scans-required.sh"
[ -x "$SCRIPT" ] || { echo "missing or not executable: $SCRIPT" >&2; exit 1; }

TMP="$(cd "$(mktemp -d)" && pwd -P)"
trap 'cd /; find "$TMP" -depth -mindepth 1 \( -type f -o -type l \) -exec unlink {} \; 2>/dev/null; find "$TMP" -depth -type d -exec rmdir {} \; 2>/dev/null' EXIT

FAILURES=0
CASES=0
pass() { echo "    ok   — $*"; }
fail() { echo "    FAIL — $*" >&2; FAILURES=$((FAILURES + 1)); }
case_start() { CASES=$((CASES + 1)); echo; echo "[$CASES] $*"; }

# run <checker> <paths...>: prints "<rc>|<stdout>"
run() {
  local checker="$1"; shift
  local out rc
  out="$(printf '%s\n' "$@" | DEPENDENCY_CHECK_SCRIPT="$checker" "$SCRIPT" 2>/dev/null)"; rc=$?
  printf '%s|%s' "$rc" "$out"
}
REAL="$REPO_ROOT/scripts/dependency-change-check.sh"

case_start "the real checker: the decision is right"
[ "$(run "$REAL" Cargo.lock README.md)" = "0|advisory=true" ] && pass "a lockfile change -> advisory=true" || fail "lockfile: $(run "$REAL" Cargo.lock README.md)"
[ "$(run "$REAL" README.md docs/a.md)" = "0|advisory=false" ] && pass "docs only -> advisory=false" || fail "docs: $(run "$REAL" README.md docs/a.md)"

case_start "it fails closed when the checker is broken"
printf '#!/usr/bin/env bash\nexit 3\n' > "$TMP/crash.sh"
printf '#!/usr/bin/env bash\nexit 0\n' > "$TMP/yes.sh"
printf '#!/usr/bin/env bash\nexit 1\n' > "$TMP/no.sh"
r="$(run "$TMP/crash.sh" Cargo.lock)"
[ "${r%%|*}" = 2 ] && [ -z "${r#*|}" ] && pass "a crashing checker (exit 3) with Cargo.lock -> exit 2, nothing on stdout" || fail "crash: $r"
r="$(run "$TMP/does-not-exist.sh" Cargo.lock)"
[ "${r%%|*}" = 2 ] && [ -z "${r#*|}" ] && pass "a MISSING checker with Cargo.lock -> exit 2, nothing on stdout (was: audits skipped)" || fail "missing: $r"
r="$(run "$TMP/does-not-exist.sh" README.md)"
[ "${r%%|*}" = 2 ] && pass "a missing checker is an error even when no dependency file changed" || fail "missing, docs: $r"
[ "$(run "$TMP/yes.sh" x)" = "0|advisory=true" ] && pass "checker exit 0 -> advisory=true" || fail "exit 0"
[ "$(run "$TMP/no.sh" x)" = "0|advisory=false" ] && pass "checker exit 1 -> advisory=false" || fail "exit 1"

echo
if [ "$FAILURES" -eq 0 ]; then
  echo "PASS — $CASES cases, 0 failures"
  exit 0
fi
echo "FAIL — $CASES cases, $FAILURES failed assertions" >&2
exit 1
