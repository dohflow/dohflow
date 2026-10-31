#!/usr/bin/env bash
#
# Guards on the two alert jobs in ci.yml (personal-cfo-md8h2.21): the job that opens the
# ci-main-red alert and the job that closes it must watch the SAME gates, or one run can open
# an alert and close it again (a gate only one of them waits for), and the alert job needs the
# `actions: read` permission its `gh run list` call uses.

# The `check && pass || fail` lines are if-then-else on purpose: pass() always
# returns 0, so fail() runs only when the check failed.
# shellcheck disable=SC2015
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
CI="$REPO_ROOT/.github/workflows/ci.yml"
[ -f "$CI" ] || { echo "missing: $CI" >&2; exit 1; }

FAILURES=0
CASES=0
pass() { echo "    ok   — $*"; }
fail() { echo "    FAIL — $*" >&2; FAILURES=$((FAILURES + 1)); }
case_start() { CASES=$((CASES + 1)); echo; echo "[$CASES] $*"; }

# job_text <job id>: from its key to the next top-level job.
job_text() {
  awk -v key="  $1:" '$0 == key { on = 1; print; next } on && /^  [a-z0-9-]+:$/ { exit } on { print }' "$CI"
}
needs_of() { job_text "$1" | sed -n 's/^    needs: \[\(.*\)\]$/\1/p' | head -n 1; }

alert_needs="$(needs_of main-red-alert)"
resolve_needs="$(needs_of main-red-resolve)"

case_start "the alert and resolve jobs wait for the same gates"
[ -n "$alert_needs" ] && pass "main-red-alert needs: $alert_needs" || fail "main-red-alert has no needs list"
[ "$alert_needs" = "$resolve_needs" ] && pass "main-red-resolve needs the identical list" || fail "main-red-resolve needs differ: '$resolve_needs' vs '$alert_needs'"

case_start "every job id the alerts wait for exists"
IFS=',' read -r -a ids <<<"${alert_needs// /}"
for id in "${ids[@]}"; do
  [ -n "$(job_text "$id")" ] && pass "job '$id' exists" || fail "job '$id' does not exist in ci.yml"
done

case_start "the alert job can read Actions runs (gh run list) and write issues, nothing more"
alert="$(job_text main-red-alert)"
[[ "$alert" == *"actions: read"* ]] && pass "actions: read" || fail "main-red-alert lacks actions: read (its gh run list would get a 403)"
[[ "$alert" == *"issues: write"* ]] && pass "issues: write" || fail "main-red-alert lacks issues: write"
[[ "$alert" != *"contents: write"* && "$alert" != *"actions: write"* ]] && pass "no content or actions write" || fail "main-red-alert has a write permission it should not"

echo
if [ "$FAILURES" -eq 0 ]; then
  echo "PASS — $CASES cases, 0 failures"
  exit 0
fi
echo "FAIL — $CASES cases, $FAILURES failed assertions" >&2
exit 1
