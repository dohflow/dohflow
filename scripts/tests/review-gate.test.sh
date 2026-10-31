#!/usr/bin/env bash
#
# Tests for scripts/review-gate.sh (personal-cfo-md8h2.23) with a stub `gh`. No network.
# The stub answers `gh pr view N --json labels,headRefOid,comments` from per-PR files.

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
if [ "\$1 \$2" = "pr view" ]; then
  f="$TMP/prs/\$3.json"
  [ -f "\$f" ] || exit 1
  cat "\$f"
  exit 0
fi
exit 1
STUB
chmod +x "$TMP/bin/gh"
export PATH="$TMP/bin:$PATH"

H1=1111111111111111111111111111111111111111
H2=2222222222222222222222222222222222222222
BASE=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
ref() { printf 'refs/heads/gh-readonly-queue/main/pr-%s-%s' "$1" "$BASE"; }

# pr N "label,label" HEAD [comment-body ...]   (labels contain no spaces; each body is one comment, oldest first)
pr() {
  local n="$1" labels="" first=1 l head="$3" comments="" c
  local labelnames="$2"
  shift 3
  for l in $(printf '%s' "$labelnames" | tr ',' ' '); do
    [ "$first" = 1 ] || labels="$labels,"
    labels="$labels{\"name\":\"$l\"}"
    first=0
  done
  first=1
  for c in "$@"; do
    [ "$first" = 1 ] || comments="$comments,"
    comments="$comments$(jq -n --arg b "$c" '{body: $b, author: {login: "reviewer"}}')"
    first=0
  done
  printf '{"labels":[%s],"headRefOid":"%s","comments":[%s]}' "$labels" "$head" "$comments" > "$TMP/prs/$n.json"
}
marker() { printf 'Verdict.\n\n<!-- review-verdict: %s %s -->' "$1" "$2"; }

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

# A real recorded queue entry (dohflow/dohflow run 40614311641, #191): the queue ref and the tested
# commit exactly as GitHub sent them. Squash merges: the tested commit is NOT a descendant of the PR head.
REAL_REF="refs/heads/gh-readonly-queue/main/pr-191-2d5668a2bd8106aabcc8bfe59cf9eb05934742e6"
REAL_HEAD=8373e7f1218711e157437202ccd031684bfd5b04

case_start "merge_group: the label plus a PASS marker for the current head passes (a real recorded queue ref)"
pr 191 "merge-ready" "$REAL_HEAD" "Review.\n$(marker PASS "$REAL_HEAD")"
gate --event merge_group --ref "$REAL_REF"
[ "$GRC" = 0 ] && pass "recorded squash-queue ref: pass" || fail "recorded ref failed: $(cat "$TMP/out")"
pr 20 "merge-ready" "$H1" "$(marker PASS "$H1")"
gate --event merge_group --ref "$(ref 20)"
[ "$GRC" = 0 ] && pass "label + matching PASS marker: pass" || fail "failed: $(cat "$TMP/out")"
pr 20 "merge-ready" "$H1" "$(marker CHANGES_REQUIRED "$H1")" "$(marker PASS "$H1")"
gate --event merge_group --ref "$(ref 20)"
[ "$GRC" = 0 ] && pass "a PASS after an earlier CHANGES_REQUIRED: pass" || fail "failed: $(cat "$TMP/out")"
pr 20 "merge-ready" "$H1" "intro $(marker PASS "$H2") then $(marker PASS "$H1")"
gate --event merge_group --ref "$(ref 20)"
[ "$GRC" = 0 ] && pass "two markers in one comment: the last one counts" || fail "failed: $(cat "$TMP/out")"

case_start "merge_group: the label removed while queued fails"
pr 21 "area:ci" "$H1" "$(marker PASS "$H1")"
gate --event merge_group --ref "$(ref 21)"
[ "$GRC" = 1 ] && pass "unlabeled entry: fail" || fail "unlabeled entry passed"

case_start "merge_group: a head the reviewer did not pass fails"
pr 22 "merge-ready" "$H2" "$(marker PASS "$H1")"
gate --event merge_group --ref "$(ref 22)"
[ "$GRC" = 1 ] && pass "marker for an older SHA: fail" || fail "an older-SHA marker passed"
grep -q "not the head the reviewer passed" "$TMP/out" && pass "the failure says why" || fail "no reason: $(cat "$TMP/out")"
pr 23 "merge-ready" "$H1" "$(marker PASS "$H1")" "$(marker CHANGES_REQUIRED "$H1")"
gate --event merge_group --ref "$(ref 23)"
[ "$GRC" = 1 ] && pass "a later CHANGES_REQUIRED at the same SHA: fail" || fail "a later CHANGES_REQUIRED passed"
pr 24 "merge-ready" "$H1" "$(marker PASS "$H1")" "$(marker HUMAN_DECISION_REQUIRED "$H2")"
gate --event merge_group --ref "$(ref 24)"
[ "$GRC" = 1 ] && pass "a later HUMAN_DECISION_REQUIRED: fail" || fail "a later HUMAN_DECISION_REQUIRED passed"
pr 25 "merge-ready" "$H1" "$(marker PASS "$H2")" "$(marker PASS "$H1")" "$(marker PASS "$H2")"
gate --event merge_group --ref "$(ref 25)"
[ "$GRC" = 1 ] && pass "only the newest marker counts (PASS for the head, then a newer PASS for another): fail" || fail "an older matching marker passed"

case_start "merge_group: no marker, or a malformed one, fails closed"
pr 26 "merge-ready" "$H1" "Looks good to me."
gate --event merge_group --ref "$(ref 26)"
[ "$GRC" = 1 ] && pass "no marker: fail" || fail "no marker passed"
pr 26 "merge-ready" "$H1"
gate --event merge_group --ref "$(ref 26)"
[ "$GRC" = 1 ] && pass "no comments at all: fail" || fail "no comments passed"
for bad in "<!-- review-verdict: PASS ${H1:0:39} -->" "<!-- review-verdict: PASS $H1" "<!-- review-verdict: pass $H1 -->" "<!-- review-verdict: PASS $H1 extra -->" "<!-- review-verdict: PASS -->" "<!-- review-verdict:PASS $H1 -->"; do
  pr 26 "merge-ready" "$H1" "$(marker PASS "$H1")" "$bad"
  gate --event merge_group --ref "$(ref 26)"
  [ "$GRC" = 1 ] && pass "malformed marker '${bad:0:48}': fail" || fail "malformed marker '$bad' passed"
done
pr 26 "merge-ready" "$H1" "<!-- review-verdict: PASS $(printf 'G%.0s' $(seq 1 40)) -->"
gate --event merge_group --ref "$(ref 26)"
[ "$GRC" = 1 ] && pass "a non-hex SHA: fail" || fail "a non-hex SHA passed"

case_start "merge_group: fails closed on an unparsable ref or an unreadable pull request"
for bad in "" "refs/heads/main" "refs/heads/gh-readonly-queue/main/pr-x-$BASE" "refs/heads/gh-readonly-queue/main/pr-20-short" "refs/heads/gh-readonly-queue/main/pr-20-$BASE/extra" "gh-readonly-queue/main/pr-20-$BASE"; do
  pr 20 "merge-ready" "$H1" "$(marker PASS "$H1")"
  gate --event merge_group --ref "$bad"
  [ "$GRC" = 1 ] && pass "ref '${bad:0:50}': fail" || fail "ref '$bad' passed"
done
gate --event merge_group --ref "$(ref 404)"
[ "$GRC" = 1 ] && pass "the named PR cannot be read: fail" || fail "an unreadable PR passed"

case_start "merge_group: a two-PR group is TWO entries, each gated in its own run"
pr 30 "merge-ready" "$H1" "$(marker PASS "$H1")"
pr 31 "area:ci" "$H2" "$(marker PASS "$H2")"
gate --event merge_group --ref "$(ref 30)"; a=$GRC
gate --event merge_group --ref "$(ref 31)"; b=$GRC
[ "$a" = 0 ] && [ "$b" = 1 ] && pass "#30 (labeled, passed) passes in its run, #31 (unlabeled) fails in its own" || fail "entries: #30=$a #31=$b"
grep -q "#31" "$TMP/out" && pass "the failure names the PR that lacks the label" || fail "failure does not name #31: $(cat "$TMP/out")"
pr 32 "merge-ready" "$H2" "$(marker PASS "$H1")"
gate --event merge_group --ref "$(ref 30)"; a=$GRC
gate --event merge_group --ref "$(ref 32)"; b=$GRC
[ "$a" = 0 ] && [ "$b" = 1 ] && pass "a second entry whose head moved fails while the first passes" || fail "entries: #30=$a #32=$b"

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
