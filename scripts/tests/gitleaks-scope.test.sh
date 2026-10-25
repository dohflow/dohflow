#!/usr/bin/env bash
#
# The P0 scenario, with the REAL gitleaks (personal-cfo-md8h2.22): a flagged file on an unrelated
# branch must not fail the scan of a clean change, a flagged file in the change's own range must
# fail it, and the daily audit (--all-refs) must still see the unrelated branch.
#
# Needs the gitleaks binary. Without it the test SKIPS (exit 0) and says so: the shell-tests job
# has no gitleaks, and the `Security scanning` job, which installs it, runs this test explicitly.

# The `check && pass || fail` lines are if-then-else on purpose: pass() always
# returns 0, so fail() runs only when the check failed.
# shellcheck disable=SC2015
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
SCRIPT="$REPO_ROOT/scripts/gitleaks-scan.sh"
if ! command -v gitleaks >/dev/null 2>&1; then
  echo "SKIP — gitleaks is not installed (the Security scanning job runs this test with it)"
  exit 0
fi

TMP="$(cd "$(mktemp -d)" && pwd -P)"
trap 'cd /; chmod -R u+w "$TMP" 2>/dev/null; find "$TMP" -depth -mindepth 1 \( -type f -o -type l \) -exec unlink {} \; 2>/dev/null; find "$TMP" -depth -type d -exec rmdir {} \; 2>/dev/null' EXIT

FAILURES=0
CASES=0
pass() { echo "    ok   — $*"; }
fail() { echo "    FAIL — $*" >&2; FAILURES=$((FAILURES + 1)); }
case_start() { CASES=$((CASES + 1)); echo; echo "[$CASES] $*"; }

# A synthetic, non-working key that the repository's gitleaks config flags (generic-api-key).
# It is assembled at run time from two halves: written out whole, this very file would be a
# finding of the scan it tests.
half_a="k3J9xQ2mP8vL5"
half_b="nR7tY1wZ4bC6dF0gH2sA8uE"
FLAGGED="api_key = \"${half_a}${half_b}\""

cd "$TMP" || exit 1
git init -q -b main
git config user.email t@example.invalid
git config user.name tester
cp "$REPO_ROOT/.gitleaks.toml" .
printf 'clean\n' > a.txt
git add -A && git commit -q -m "base"
BASE="$(git rev-parse HEAD)"

# An unrelated branch carrying a flagged file (what #177's branch did).
git checkout -q -b unrelated
printf '%s\n' "$FLAGGED" > vectors.txt
git add -A && git commit -q -m "flagged file on an unrelated branch"
git checkout -q main

# A clean change on main.
printf 'more\n' >> a.txt
git commit -q -am "clean change"
CLEAN_HEAD="$(git rev-parse HEAD)"

scan() { "$SCRIPT" "$@" >"$TMP/out" 2>&1; }

case_start "the unrelated branch really is flagged (the control: gitleaks sees it)"
scan --all-refs && fail "--all-refs found nothing: the flagged file is not detected" || pass "--all-refs fails on the flagged branch"

case_start "a clean change passes although an unrelated branch is flagged (the P0)"
scan --event pull_request --base "$BASE" --head "$CLEAN_HEAD" && pass "pull_request: clean range passes" || fail "pull_request failed: $(tail -3 "$TMP/out")"
scan --event merge_group --base "$BASE" --head "$CLEAN_HEAD" && pass "merge_group: clean range passes" || fail "merge_group failed: $(tail -3 "$TMP/out")"
scan --event push --base "$BASE" --head "$CLEAN_HEAD" && pass "push: clean range passes" || fail "push failed: $(tail -3 "$TMP/out")"
scan --event schedule && pass "schedule/dispatch: the history of the checked-out main passes" || fail "schedule failed: $(tail -3 "$TMP/out")"

case_start "a flagged file IN the change's own range fails"
git checkout -q -b bad main
printf '%s\n' "$FLAGGED" > leaked.txt
git add -A && git commit -q -m "flagged file in this change"
BAD_HEAD="$(git rev-parse HEAD)"
scan --event pull_request --base "$BASE" --head "$BAD_HEAD" && fail "a flagged commit in the PR range passed" || pass "pull_request: the flagged range fails"
scan --event merge_group --base "$CLEAN_HEAD" --head "$BAD_HEAD" && fail "a flagged commit in the group range passed" || pass "merge_group: the flagged range fails"
scan --event push --base "$CLEAN_HEAD" --head "$BAD_HEAD" && fail "a flagged pushed range passed" || pass "push: the flagged range fails"
scan --event schedule && fail "the checked-out history includes the flagged commit but passed" || pass "schedule/dispatch on a branch with the flagged commit fails"

case_start "the range scan sees ONLY its range: a flagged commit before BASE is not read"
scan --event pull_request --base "$BAD_HEAD" --head "$BAD_HEAD" && pass "an empty range after the flagged commit passes" || fail "empty range failed: $(tail -3 "$TMP/out")"

echo
if [ "$FAILURES" -eq 0 ]; then
  echo "PASS — $CASES cases, 0 failures"
  exit 0
fi
echo "FAIL — $CASES cases, $FAILURES failed assertions" >&2
exit 1
