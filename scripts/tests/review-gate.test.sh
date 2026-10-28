#!/usr/bin/env bash
#
# Tests for scripts/review-gate.sh (personal-cfo-md8h2.23) with a stub `gh`. No network.
# The stub answers `gh pr view N` from per-PR files and `gh api .../compare/A...B` from a table.

# The `check && pass || fail` lines are if-then-else on purpose: pass() always
# returns 0, so fail() runs only when the check failed.
# shellcheck disable=SC2015,SC2016
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
SCRIPT="$REPO_ROOT/scripts/review-gate.sh"
[ -x "$SCRIPT" ] || { echo "missing or not executable: $SCRIPT" >&2; exit 1; }

TMP="$(cd "$(mktemp -d)" && pwd -P)"
trap 'cd /; find "$TMP" -depth -mindepth 1 \( -type f -o -type l \) -exec unlink {} \; 2>/dev/null; find "$TMP" -depth -type d -exec rmdir {} \; 2>/dev/null' EXIT

FAILURES=0
CASES=0
pass() { echo "    ok   — $*"; }
fail() { echo "    FAIL — $*" >&2; FAILURES=$((FAILURES + 1)); }
case_start() { CASES=$((CASES + 1)); echo; echo "[$CASES] $*"; }

mkdir -p "$TMP/bin" "$TMP/prs"
cat > "$TMP/bin/gh" <<STUB
#!/usr/bin/env bash
# gh pr view N ...  -> prs/N.json, or exit 1 when the file is absent (a lookup error)
# gh api repos/R/compare/A...B -> the status in compare/A...B, or exit 1
if [ "\$1 \$2" = "pr view" ]; then
  f="$TMP/prs/\$3.json"
  [ -f "\$f" ] || exit 1
  cat "\$f"
  exit 0
fi
if [ "\$1" = "api" ]; then
  key="\${2##*/compare/}"
  f="$TMP/compare/\$key"
  [ -f "\$f" ] || exit 1
  cat "\$f"
  exit 0
fi
exit 1
STUB
chmod +x "$TMP/bin/gh"
mkdir -p "$TMP/compare"
export PATH="$TMP/bin:$PATH"

H1=1111111111111111111111111111111111111111
H2=2222222222222222222222222222222222222222
TESTED=cccccccccccccccccccccccccccccccccccccccc
BASE=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
ref() { printf 'refs/heads/gh-readonly-queue/main/pr-%s-%s' "$1" "$BASE"; }

pr() { # pr N "label,label" HEAD   (labels contain no spaces)
  local labels="" first=1 l
  for l in $(printf '%s' "${2:-}" | tr ',' ' '); do
    [ "$first" = 1 ] || labels="$labels,"
    labels="$labels{\"name\":\"$l\"}"
    first=0
  done
  printf '{"labels":[%s],"headRefOid":"%s"}' "$labels" "$3" > "$TMP/prs/$1.json"
}
compare() { printf '%s' "$3" > "$TMP/compare/$1...$2"; }   # compare HEAD TESTED status

GRC=0
gate() { "$SCRIPT" --repo o/r "$@" >"$TMP/out" 2>&1; GRC=$?; }

case_start "pull_request: passes only with the merge-ready label"
pr 10 "merge-ready,area:ci" "$H1"; gate --event pull_request --pr 10
[ "$GRC" = 0 ] && pass "label present: pass" || fail "label present failed: $(cat "$TMP/out")"
pr 11 "area:ci" "$H1"; gate --event pull_request --pr 11
[ "$GRC" = 1 ] && pass "label absent: fail" || fail "label absent did not fail"
pr 12 "" "$H1"; gate --event pull_request --pr 12
[ "$GRC" = 1 ] && pass "no labels at all: fail" || fail "no labels did not fail"
pr 13 "not-merge-ready,merge-ready-ish" "$H1"; gate --event pull_request --pr 13
[ "$GRC" = 1 ] && pass "a similar label name does not count" || fail "a similar label counted"

case_start "pull_request: a new commit (synchronize) never passes, even while a stale label is still on"
pr 14 "merge-ready" "$H1"
gate --event pull_request --action synchronize --pr 14
[ "$GRC" = 1 ] && pass "synchronize with the label present: fail" || fail "synchronize passed: $(cat "$TMP/out")"
for action in opened reopened labeled unlabeled ready_for_review; do
  gate --event pull_request --action "$action" --pr 14
  [ "$GRC" = 0 ] && pass "$action with the label present: pass" || fail "$action failed: $(cat "$TMP/out")"
done
pr 15 "area:ci" "$H1"
gate --event pull_request --action unlabeled --pr 15
[ "$GRC" = 1 ] && pass "unlabeled with the label gone: fail" || fail "unlabeled with no label passed"

case_start "pull_request: fails closed on a lookup error or a bad number"
gate --event pull_request --pr 999
[ "$GRC" = 1 ] && pass "unreadable pull request: fail" || fail "lookup error did not fail"
gate --event pull_request
[ "$GRC" = 1 ] && pass "no number: fail" || fail "no number did not fail"
gate --event pull_request --pr "10; echo owned"
[ "$GRC" = 1 ] && pass "a non-numeric number: fail" || fail "a non-numeric number did not fail"

case_start "merge_group: a labeled PR whose head is part of the tested commit passes"
pr 20 "merge-ready" "$H1"; compare "$H1" "$TESTED" ahead
gate --event merge_group --ref "$(ref 20)" --head-sha "$TESTED"
[ "$GRC" = 0 ] && pass "ahead: pass" || fail "ahead failed: $(cat "$TMP/out")"
compare "$H1" "$TESTED" identical
gate --event merge_group --ref "$(ref 20)" --head-sha "$TESTED"
[ "$GRC" = 0 ] && pass "identical: pass" || fail "identical failed: $(cat "$TMP/out")"

case_start "merge_group: the label removed while queued fails"
pr 21 "area:ci" "$H1"; compare "$H1" "$TESTED" ahead
gate --event merge_group --ref "$(ref 21)" --head-sha "$TESTED"
[ "$GRC" = 1 ] && pass "unlabeled entry: fail" || fail "unlabeled entry passed"

case_start "merge_group: a head that is not an ancestor of the tested commit fails"
pr 22 "merge-ready" "$H2"
for status in behind diverged; do
  compare "$H2" "$TESTED" "$status"
  gate --event merge_group --ref "$(ref 22)" --head-sha "$TESTED"
  [ "$GRC" = 1 ] && pass "comparison '$status': fail" || fail "'$status' passed"
done
rm_cmp() { find "$TMP/compare" -type f -exec unlink {} \; ; }
rm_cmp
gate --event merge_group --ref "$(ref 22)" --head-sha "$TESTED"
[ "$GRC" = 1 ] && pass "comparison unavailable: fail" || fail "an unavailable comparison passed"

case_start "merge_group: fails closed on an unparsable ref or a bad tested commit"
for bad in "" "refs/heads/main" "refs/heads/gh-readonly-queue/main/pr-x-$BASE" "refs/heads/gh-readonly-queue/main/pr-20-short" "refs/heads/gh-readonly-queue/main/pr-20-$BASE/extra" "gh-readonly-queue/main/pr-20-$BASE"; do
  pr 20 "merge-ready" "$H1"; compare "$H1" "$TESTED" ahead
  gate --event merge_group --ref "$bad" --head-sha "$TESTED"
  [ "$GRC" = 1 ] && pass "ref '${bad:0:50}': fail" || fail "ref '$bad' passed"
done
gate --event merge_group --ref "$(ref 20)" --head-sha "short"
[ "$GRC" = 1 ] && pass "a short tested SHA: fail" || fail "a short tested SHA passed"
gate --event merge_group --ref "$(ref 404)" --head-sha "$TESTED"
[ "$GRC" = 1 ] && pass "the named PR cannot be read: fail" || fail "an unreadable PR passed"

case_start "merge_group: a two-PR group is TWO entries, each gated in its own run"
pr 30 "merge-ready" "$H1"; pr 31 "area:ci" "$H2"
compare "$H1" "$TESTED" ahead; compare "$H2" "$TESTED" ahead
gate --event merge_group --ref "$(ref 30)" --head-sha "$TESTED"; a=$GRC
gate --event merge_group --ref "$(ref 31)" --head-sha "$TESTED"; b=$GRC
[ "$a" = 0 ] && [ "$b" = 1 ] && pass "#30 (labeled) passes in its run, #31 (unlabeled) fails in its own" || fail "entries: #30=$a #31=$b"
grep -q "#31" "$TMP/out" && pass "the failure names the PR that lacks the label" || fail "failure does not name #31: $(cat "$TMP/out")"

case_start "an unknown event, or none, is an error"
gate --event push --pr 10
[ "$GRC" = 1 ] && pass "unknown event: fail" || fail "unknown event passed"
gate
[ "$GRC" = 2 ] && pass "no event: usage error" || fail "no event did not exit 2"

echo
if [ "$FAILURES" -eq 0 ]; then
  echo "PASS — $CASES cases, 0 failures"
  exit 0
fi
echo "FAIL — $CASES cases, $FAILURES failed assertions" >&2
exit 1
