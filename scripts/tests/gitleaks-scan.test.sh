#!/usr/bin/env bash
#
# Tests for scripts/gitleaks-scan.sh (personal-cfo-md8h2.22): which commits each CI event makes
# gitleaks read, and that it fails closed. A stub `gitleaks` records its arguments; no network.

# The `check && pass || fail` lines are if-then-else on purpose: pass() always
# returns 0, so fail() runs only when the check failed.
# shellcheck disable=SC2015
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
SCRIPT="$REPO_ROOT/scripts/gitleaks-scan.sh"
[ -x "$SCRIPT" ] || { echo "missing or not executable: $SCRIPT" >&2; exit 1; }

TMP="$(cd "$(mktemp -d)" && pwd -P)"
trap 'cd /; find "$TMP" -depth -mindepth 1 \( -type f -o -type l \) -exec unlink {} \; 2>/dev/null; find "$TMP" -depth -type d -exec rmdir {} \; 2>/dev/null' EXIT

FAILURES=0
CASES=0
pass() { echo "    ok   — $*"; }
fail() { echo "    FAIL — $*" >&2; FAILURES=$((FAILURES + 1)); }
case_start() { CASES=$((CASES + 1)); echo; echo "[$CASES] $*"; }

mkdir -p "$TMP/bin"
CALLS="$TMP/gitleaks-calls"
cat > "$TMP/bin/gitleaks" <<STUB
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$CALLS"
exit \${STUB_EXIT:-0}
STUB
chmod +x "$TMP/bin/gitleaks"
export PATH="$TMP/bin:$PATH"

A=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
B=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
ZERO=0000000000000000000000000000000000000000

run() { ( cd "$TMP" && "$SCRIPT" "$@" ) >"$TMP/out" 2>&1; }
last_call() { tail -n 1 "$CALLS" 2>/dev/null; }
# Capture the last call first, then match (no pipe into a quiet grep under pipefail).
call_has() { local line; line="$(last_call)"; grep -qF -- "$1" <<<"$line"; }
call_has_re() { local line; line="$(last_call)"; grep -qE -- "$1" <<<"$line"; }

case_start "pull requests and merge groups read base..head only"
: > "$CALLS"; run --event pull_request --base "$A" --head "$B"; rc=$?
[ "$rc" = 0 ] && pass "pull_request runs" || fail "pull_request rc=$rc: $(cat "$TMP/out")"
call_has "--log-opts=$A..$B" && pass "range is base..head" || fail "wrong call: $(last_call)"
: > "$CALLS"; run --event merge_group --base "$B" --head "$A"
call_has "--log-opts=$B..$A" && pass "merge_group range is the group's base..head" || fail "merge_group call: $(last_call)"

case_start "a push reads before..after; a first push reads what is reachable from the pushed head"
: > "$CALLS"; run --event push --base "$A" --head "$B"
call_has "--log-opts=$A..$B" && pass "before..after" || fail "push call: $(last_call)"
: > "$CALLS"; run --event push --base "$ZERO" --head "$B"
call_has_re "--log-opts=$B\$" && pass "all-zero before: the pushed head only" || fail "first push call: $(last_call)"
: > "$CALLS"; run --event push --head "$B"
call_has_re "--log-opts=$B\$" && pass "empty before: the pushed head only" || fail "empty-before call: $(last_call)"

case_start "scheduled and manual runs read the checked-out history (HEAD), not every branch"
for ev in schedule workflow_dispatch; do
  : > "$CALLS"; run --event "$ev"
  call_has "--log-opts=HEAD" && pass "$ev: HEAD" || fail "$ev call: $(last_call)"
done

case_start "--all-refs is the audit's opt-in to every branch and tag"
: > "$CALLS"; run --all-refs
call_has "--log-opts=--all" && pass "--all" || fail "all-refs call: $(last_call)"

case_start "it always passes an explicit --log-opts, the config, and --redact"
: > "$CALLS"
run --event pull_request --base "$A" --head "$B"; run --event push --base "$A" --head "$B"; run --event schedule; run --all-refs
bad=0; while IFS= read -r line; do
  case "$line" in *"--log-opts="*"--config .gitleaks.toml"*"--redact"*) ;; *"--config .gitleaks.toml"*"--redact"*"--log-opts="*) ;; *) bad=$((bad + 1)) ;; esac
done < "$CALLS"
total="$(wc -l < "$CALLS")"; total=$((total + 0))
[ "$bad" = 0 ] && [ "$total" = 4 ] && pass "4 calls, each with --log-opts, --config .gitleaks.toml and --redact" || fail "$bad of $total calls lack a flag: $(cat "$CALLS")"

case_start "a report path is forwarded"
: > "$CALLS"; run --event schedule --report "$TMP/r.json"
call_has "--report-format json --report-path $TMP/r.json" && pass "report flags added" || fail "report call: $(last_call)"

case_start "it fails closed: no scan, exit 2, on anything it cannot place"
check_closed() {
  : > "$CALLS"; run "$@"; rc=$?
  [ "$rc" = 2 ] && [ ! -s "$CALLS" ] && pass "exit 2, gitleaks not called: $*" || fail "rc=$rc, calls: $(cat "$CALLS") ($*)"
}
check_closed --event pull_request --head "$B"
check_closed --event pull_request --base "$A"
check_closed --event pull_request --base abc --head "$B"
check_closed --event merge_group --base "$A" --head "$B; echo owned"
check_closed --event pull_request --base "${A:0:39}Z" --head "$B"
check_closed --event push --base "not-a-sha" --head "$B"
check_closed --event push
check_closed --event release
check_closed
check_closed --event pull_request --all-refs --base "$A" --head "$B"
check_closed --bogus

case_start "gitleaks' own exit status is the script's (a finding fails the step)"
: > "$CALLS"; STUB_EXIT=1 run --event schedule; rc=$?
[ "$rc" = 1 ] && pass "exit 1 passes through" || fail "rc=$rc"

echo
if [ "$FAILURES" -eq 0 ]; then
  echo "PASS — $CASES cases, 0 failures"
  exit 0
fi
echo "FAIL — $CASES cases, $FAILURES failed assertions" >&2
exit 1
