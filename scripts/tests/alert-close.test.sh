#!/usr/bin/env bash
#
# Tests for scripts/alert-close.sh (personal-cfo-md8h2.21) with a stub `gh`. No network, no real
# issue is touched.

# The `check && pass || fail` lines are if-then-else on purpose: pass() always
# returns 0, so fail() runs only when the check failed.
# shellcheck disable=SC2015,SC2016
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
SCRIPT="$REPO_ROOT/scripts/alert-close.sh"
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
# `issue list` answers STUB_OPEN_LINES ("N<TAB>labels" per line); `api` answers STUB_REOPEN
# (a count per issue number: "42=1 43=0"); every call is recorded.
cat > "$TMP/bin/gh" <<STUB
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$CALLS"
if [ "\$1 \$2" = "issue list" ]; then printf '%s' "\${STUB_OPEN_LINES:-}"; fi
if [ "\$1" = "api" ] && [ "\${2#*commits/main}" != "\$2" ]; then printf '%s\\n' "\${STUB_TIP:-}"; exit 0; fi
if [ "\$1" = "api" ]; then
  n=\$(printf '%s' "\$2" | sed -E 's#.*/issues/([0-9]+)/events#\1#')
  for pair in \${STUB_REOPEN:-}; do [ "\${pair%%=*}" = "\$n" ] && { echo "\${pair#*=}"; exit 0; }; done
  echo 0
fi
exit 0
STUB
chmod +x "$TMP/bin/gh"
export PATH="$TMP/bin:$PATH"

run() { ( "$SCRIPT" --label security-advisory --repo o/r --run-url https://example.test/run/9 --what "the audit is clean" "$@" ); }
tab() { printf '%s\t%s' "$1" "$2"; }

case_start "an open alert is commented on with the run link, then closed as completed"
: > "$CALLS"; STUB_OPEN_LINES="$(tab 42 security-advisory)" run >/dev/null
grep -q "^issue comment 42 .*https://example.test/run/9" "$CALLS" && pass "comment links the run" || fail "no comment with the link: $(cat "$CALLS")"
grep -q "^issue close 42 .*--reason completed" "$CALLS" && pass "closed as completed" || fail "not closed: $(cat "$CALLS")"
c="$(grep -n '^issue comment 42' "$CALLS" | head -1 | cut -d: -f1)"; k="$(grep -n '^issue close 42' "$CALLS" | head -1 | cut -d: -f1)"
[ -n "$c" ] && [ -n "$k" ] && [ "$c" -lt "$k" ] && pass "the comment comes before the close" || fail "order: comment line $c, close line $k"

case_start "several open alerts are each handled"
: > "$CALLS"; STUB_OPEN_LINES="$(printf '42\tsecurity-advisory\n43\tsecurity-advisory')" run >/dev/null
grep -q "^issue close 42 " "$CALLS" && grep -q "^issue close 43 " "$CALLS" && pass "both closed" || fail "not both closed: $(cat "$CALLS")"

case_start "keep-open is never closed"
: > "$CALLS"; STUB_OPEN_LINES="$(tab 42 security-advisory,keep-open)" run >/dev/null
grep -q "^issue close\|^issue comment" "$CALLS" && fail "touched a keep-open issue: $(cat "$CALLS")" || pass "left alone"
: > "$CALLS"; STUB_OPEN_LINES="$(printf '42\tsecurity-advisory,keep-open\n43\tsecurity-advisory')" run >/dev/null
grep -q "^issue close 43 " "$CALLS" && ! grep -q "^issue close 42 " "$CALLS" && pass "only the other issue is closed" || fail "mixed case wrong: $(cat "$CALLS")"

case_start "an issue a human reopened in the last 24 hours is left open"
: > "$CALLS"; STUB_OPEN_LINES="$(printf '42\tsecurity-advisory\n43\tsecurity-advisory')" STUB_REOPEN="42=1 43=0" run >/dev/null
! grep -q "^issue close 42 " "$CALLS" && grep -q "^issue close 43 " "$CALLS" && pass "#42 held, #43 closed" || fail "reopen handling wrong: $(cat "$CALLS")"

case_start "nothing is closed unless every gate was green (needs-json)"
: > "$CALLS"; STUB_OPEN_LINES="$(tab 42 ci-main-red)" run --needs-json '{"rust":{"result":"success"},"shell-scripts":{"result":"failure"}}' >/dev/null
grep -q "^issue close\|^issue comment\|^issue list" "$CALLS" && fail "acted although a gate failed: $(cat "$CALLS")" || pass "a failed gate: nothing happens"
: > "$CALLS"; STUB_OPEN_LINES="$(tab 42 ci-main-red)" run --needs-json '{"rust":{"result":"success"},"frontend":{"result":"skipped"}}' >/dev/null
grep -q "^issue close" "$CALLS" && fail "a skipped gate counted as green" || pass "a skipped gate is not green"
: > "$CALLS"; STUB_OPEN_LINES="$(tab 42 ci-main-red)" run --needs-json '{"rust":{"result":"cancelled"}}' >/dev/null
grep -q "^issue close" "$CALLS" && fail "a cancelled gate counted as green" || pass "a cancelled gate is not green"
: > "$CALLS"; STUB_OPEN_LINES="$(tab 42 ci-main-red)" run --needs-json '{"rust":{"result":"success"},"frontend":{"result":"success"}}' >/dev/null
grep -q "^issue close 42 " "$CALLS" && pass "all green: closed" || fail "all green but not closed: $(cat "$CALLS")"

case_start "only-if-tip: a run of an older commit closes nothing (the tip's own run decides)"
: > "$CALLS"; STUB_TIP=bbbbbbbbbbbb STUB_OPEN_LINES="$(tab 42 ci-main-red)" run --only-if-tip aaaaaaaaaaaa >/dev/null
grep -q "^issue close\|^issue comment" "$CALLS" && fail "an older commit's green run closed the alert: $(cat "$CALLS")" || pass "nothing closed when the run is not the tip"
grep -q "^issue list" "$CALLS" && fail "it even listed issues" || pass "it did not look at the issues"
: > "$CALLS"; STUB_TIP=aaaaaaaaaaaa STUB_OPEN_LINES="$(tab 42 ci-main-red)" run --only-if-tip aaaaaaaaaaaa >/dev/null
grep -q "^issue close 42 " "$CALLS" && pass "the tip's own green run closes it" || fail "tip run did not close: $(cat "$CALLS")"
: > "$CALLS"; STUB_TIP="" STUB_OPEN_LINES="$(tab 42 ci-main-red)" run --only-if-tip aaaaaaaaaaaa >/dev/null
grep -q "^issue close" "$CALLS" && fail "closed although the tip could not be read" || pass "an unreadable tip closes nothing"

case_start "no open alert: nothing to do"
: > "$CALLS"; STUB_OPEN_LINES="" run >/dev/null
grep -q "^issue close\|^issue comment" "$CALLS" && fail "acted with nothing open" || pass "no comment, no close"

case_start "it only ever comments and closes: never reopens, edits or deletes"
: > "$CALLS"; STUB_OPEN_LINES="$(tab 42 security-advisory)" run >/dev/null
grep -E "^issue (reopen|edit|delete|transfer|lock)|^label (delete|edit)" "$CALLS" && fail "a forbidden gh call" || pass "no reopen, edit, delete, transfer or lock"

case_start "bad usage is an error"
( "$SCRIPT" --repo o/r >/dev/null 2>&1 ) && fail "accepted a missing --label" || pass "rejects a missing --label"
( env -u GITHUB_REPOSITORY "$SCRIPT" --label x >/dev/null 2>&1 ) && fail "accepted a missing repository" || pass "rejects a missing repository"
( "$SCRIPT" --label x --repo o/r --bogus >/dev/null 2>&1 ) && fail "accepted an unknown option" || pass "rejects an unknown option"

echo
if [ "$FAILURES" -eq 0 ]; then
  echo "PASS — $CASES cases, 0 failures"
  exit 0
fi
echo "FAIL — $CASES cases, $FAILURES failed assertions" >&2
exit 1
