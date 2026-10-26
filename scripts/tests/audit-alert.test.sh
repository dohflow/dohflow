#!/usr/bin/env bash
#
# Tests for scripts/audit-alert.sh (personal-cfo-md8h2.14) with a stub `gh` that
# records its arguments. No network, no real issue is touched.

# The `check && pass || fail` lines are if-then-else on purpose: pass() always
# returns 0, so fail() runs only when the check failed.
# shellcheck disable=SC2015
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
SCRIPT="$REPO_ROOT/scripts/audit-alert.sh"
[ -x "$SCRIPT" ] || { echo "missing or not executable: $SCRIPT" >&2; exit 1; }

TMP="$(cd "$(mktemp -d)" && pwd -P)"
trap 'cd /; find "$TMP" -depth -mindepth 1 \( -type f -o -type l \) -exec unlink {} \; 2>/dev/null; find "$TMP" -depth -type d -exec rmdir {} \; 2>/dev/null' EXIT

FAILURES=0
CASES=0
pass() { echo "    ok   — $*"; }
fail() { echo "    FAIL — $*" >&2; FAILURES=$((FAILURES + 1)); }
case_start() { CASES=$((CASES + 1)); echo; echo "[$CASES] $*"; }

mkdir -p "$TMP/bin"
CALLS="$TMP/gh-calls"
# The stub: `issue list` answers with STUB_EXISTING (empty = none open); every call is recorded.
cat > "$TMP/bin/gh" <<STUB
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$CALLS"
if [ "\$1 \$2" = "issue list" ]; then printf '%s' "\${STUB_EXISTING:-}"; fi
exit 0
STUB
chmod +x "$TMP/bin/gh"
export PATH="$TMP/bin:$PATH"

run() { ( STUB_EXISTING="${STUB_EXISTING:-}" "$SCRIPT" --repo o/r --run-url https://example.test/run/1 "$@" ); }

printf 'Crate: rustls-webpki\nID: RUSTSEC-2026-0381\nID: RUSTSEC-2026-0382\nAlso RUSTSEC-2026-0381 again\n' > "$TMP/found.txt"
printf 'allowance X matches nothing; remove it\n' > "$TMP/stale.txt"
printf 'all good\n' > "$TMP/ok.txt"

case_start "a finding with no open issue opens one, labeled, naming the advisories"
: > "$CALLS"; STUB_EXISTING="" run --status failed --report "$TMP/found.txt" >/dev/null
grep -q "^label create security-advisory" "$CALLS" && pass "ensures the label exists" || fail "no label create"
grep -q "^issue create " "$CALLS" && pass "creates an issue" || fail "no issue create: $(cat "$CALLS")"
grep -q -- "--label security-advisory" "$CALLS" && pass "with the security-advisory label" || fail "label missing on create"
grep -q "Security advisory on main: RUSTSEC-2026-0381, RUSTSEC-2026-0382" "$CALLS" && pass "title names both advisories once each, sorted" || fail "title wrong: $(grep 'issue create' "$CALLS")"
grep -q "rustls-webpki" "$CALLS" && pass "the report (crate name) is in the body" || fail "report missing from body"
grep -q "P0 bead" "$CALLS" && pass "the body points the agent on duty at a P0 bead" || fail "no P0 pointer"

case_start "a finding with an issue already open comments on it instead of opening another"
: > "$CALLS"; STUB_EXISTING="42" run --status failed --report "$TMP/found.txt" >/dev/null
grep -q "^issue comment 42 " "$CALLS" && pass "comments on #42" || fail "no comment: $(cat "$CALLS")"
grep -q "^issue create" "$CALLS" && fail "opened a second issue" || pass "does not open a second issue"

case_start "a stale allowance also raises the issue, and says so"
: > "$CALLS"; STUB_EXISTING="" run --status failed --report "$TMP/stale.txt" >/dev/null
grep -q "^issue create " "$CALLS" && pass "opens an issue" || fail "no issue for a stale allowance"
grep -q "stale allowance" "$CALLS" && pass "the body names the stale allowance" || fail "stale allowance not named"
grep -q "Dependency audit failing on main (no advisory ID" "$CALLS" && pass "title says no advisory ID was found" || fail "title: $(grep 'issue create' "$CALLS")"

case_start "a clean audit with an open issue comments but never closes it"
: > "$CALLS"; STUB_EXISTING="42" run --status clean --report "$TMP/ok.txt" >/dev/null
grep -q "^issue comment 42 " "$CALLS" && pass "comments that the audit is clean" || fail "no comment"
grep -q "^issue close" "$CALLS" && fail "closed the issue" || pass "does not close the issue"

case_start "a clean audit with no open issue does nothing to issues"
: > "$CALLS"; STUB_EXISTING="" run --status clean --report "$TMP/ok.txt" >/dev/null
grep -q "^issue create\|^issue comment\|^issue close" "$CALLS" && fail "touched an issue: $(cat "$CALLS")" || pass "no issue created, commented or closed"

case_start "a report that contains a tilde fence cannot close the issue's code fence early"
printf 'before\n~~~~\ninjected ~~~~~~~~ text\n```\nafter\n' > "$TMP/fence.txt"
: > "$CALLS"; STUB_EXISTING="" run --status failed --report "$TMP/fence.txt" >/dev/null
fence="$(grep -E '^~{9,}$' "$CALLS" | head -1)"
{ [ -n "$fence" ] && [ "$(grep -cxF "$fence" "$CALLS")" = 2 ]; } && pass "the body is fenced by a longer run of tildes (${#fence}), used exactly twice" || fail "fence not longer than the longest run in the report: $(grep -E '^~+$' "$CALLS" | tr '\n' ' ')"

case_start "another audit can reuse it with its own label, title and intro (the secrets audit, md8h2.22)"
: > "$CALLS"; STUB_EXISTING="" run --status failed --report "$TMP/found.txt" --label secret-scan --title "Secrets scan finding in the repository" --intro "The daily secrets scan of all branches found something." >/dev/null
grep -q "^label create secret-scan" "$CALLS" && pass "creates the custom label" || fail "label: $(grep '^label' "$CALLS")"
grep -q -- "--label secret-scan --title Secrets scan finding in the repository" "$CALLS" && pass "uses the fixed title and label" || fail "create call: $(grep 'issue create' "$CALLS")"
grep -q "The daily secrets scan of all branches found something." "$CALLS" && pass "uses the intro" || fail "intro missing"
grep -q "The daily dependency audit" "$CALLS" && fail "the dependency wording leaked into another audit's issue" || pass "no dependency wording"

case_start "bad usage is an error"
run --status bogus --report "$TMP/ok.txt" >/dev/null 2>&1 && fail "accepted a bogus status" || pass "rejects a bogus status"
run --status failed --report "$TMP/nope.txt" >/dev/null 2>&1 && fail "accepted a missing report" || pass "rejects a missing report file"
# GitHub Actions sets GITHUB_REPOSITORY, which the script falls back to, so unset it for this case.
( env -u GITHUB_REPOSITORY "$SCRIPT" --status failed --report "$TMP/ok.txt" >/dev/null 2>&1 ) && fail "accepted a missing repository" || pass "rejects a missing repository"

echo
if [ "$FAILURES" -eq 0 ]; then
  echo "PASS — $CASES cases, 0 failures"
  exit 0
fi
echo "FAIL — $CASES cases, $FAILURES failed assertions" >&2
exit 1
