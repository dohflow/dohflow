#!/usr/bin/env bash
#
# Guard: no `producer | grep -q ...` in a shell script or a workflow step (personal-cfo-md8h2.15).
#
# `grep -q` exits at its first match. Under `set -o pipefail` the producer can then take
# SIGPIPE, the pipeline returns 141, and a check that FOUND its match reports failure. It
# made the required `Shell scripts` check red on main intermittently (a guard test), and it
# was latent in the release scripts (`curl ... | grep -qF`) and the merge-ready workflow.
# Capture the output first (`out="$(cmd)"`), then `grep -q ... <<<"$out"`.
#
# This test scans every shell script and workflow for the pattern, and proves the hazard is
# real on this machine and that the scanner can see it (a mutation check).

# The `check && pass || fail` lines are if-then-else on purpose: pass() always
# returns 0, so fail() runs only when the check failed.
# shellcheck disable=SC2015,SC2016
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"

TMP="$(cd "$(mktemp -d)" && pwd -P)"
trap 'cd /; find "$TMP" -depth -mindepth 1 \( -type f -o -type l \) -exec unlink {} \; 2>/dev/null; find "$TMP" -depth -type d -exec rmdir {} \; 2>/dev/null' EXIT

FAILURES=0
CASES=0
pass() { echo "    ok   — $*"; }
fail() { echo "    FAIL — $*" >&2; FAILURES=$((FAILURES + 1)); }
case_start() { CASES=$((CASES + 1)); echo; echo "[$CASES] $*"; }

# A pipe into grep (or egrep/fgrep) with a quiet flag, on a line that is not a comment. The flag
# may be anywhere in the grep's own arguments, bundled (-qF, -Eq) or long (--quiet, --silent).
PATTERN='\|[[:space:]]*(grep|egrep|fgrep)([[:space:]]+[^|;&[:space:]]+)*[[:space:]]+(-[A-Za-z]*q[A-Za-z]*|--quiet|--silent)([[:space:]]|$)'

# scan <file>...: prints "file:line:text" for each offending non-comment line.
scan() {
  grep -nHE -- "$PATTERN" "$@" 2>/dev/null | grep -vE '^[^:]+:[0-9]+:[[:space:]]*#' || true
}

case_start "the hazard is real: a quiet grep at the end of a pipe fails the pipeline under pipefail"
( set -o pipefail; yes | grep -q y ) 2>/dev/null; rc=$?
# `yes` never ends, so it always takes SIGPIPE once grep -q has matched and exited.
[ "$rc" -ne 0 ] && pass "'yes | grep -q y' returns $rc under pipefail although grep matched" || fail "expected a non-zero pipeline status"

case_start "the scanner sees each shape of the pattern (mutation check)"
for shape in \
  'cmd | grep -q x' \
  'cmd | grep -qF "x"' \
  'cmd | grep -F -q x' \
  'cmd | grep -E -q "x|y"' \
  'cmd |grep -Eq x' \
  'cmd | grep -qx x' \
  'cmd | grep --quiet x' \
  'a | b | grep -q x' ; do
  printf '%s\n' "$shape" > "$TMP/shape.sh"
  [ -n "$(scan "$TMP/shape.sh")" ] && pass "caught: $shape" || fail "NOT caught: $shape"
done
for safe in \
  'grep -q x file' \
  'grep -q x <<<"$out"' \
  'out="$(cmd)"' \
  'cmd | grep x' \
  'cmd | grep -c x' \
  '# cmd | grep -q x (a comment)' ; do
  printf '%s\n' "$safe" > "$TMP/safe.sh"
  [ -z "$(scan "$TMP/safe.sh")" ] && pass "allowed: $safe" || fail "wrongly flagged: $safe"
done

case_start "no script or workflow in the repository pipes into a quiet grep"
files=()
while IFS= read -r f; do files+=("$f"); done < <(
  # This file is excluded: it names the pattern in strings and in its own mutation checks.
  cd "$REPO_ROOT" && find scripts .github -type f \( -name '*.sh' -o -name '*.yml' -o -name '*.yaml' \) \
    -not -name 'no-pipe-into-grep-q.test.sh' 2>/dev/null | sort
)
found="$(cd "$REPO_ROOT" && scan "${files[@]}")"
if [ -z "$found" ]; then
  pass "none in ${#files[@]} files"
else
  fail "pipe into a quiet grep (capture the output first, then grep -q ... <<<\"\$out\"):"
  printf '%s\n' "$found" | sed 's/^/        /' >&2
fi

echo
if [ "$FAILURES" -eq 0 ]; then
  echo "PASS — $CASES cases, 0 failures"
  exit 0
fi
echo "FAIL — $CASES cases, $FAILURES failed assertions" >&2
exit 1
