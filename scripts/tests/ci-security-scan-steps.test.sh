#!/usr/bin/env bash
#
# Guard for the required `Security scanning (secrets + dependencies)` job in
# .github/workflows/ci.yml (personal-cfo-md8h2.14). When the advisory scans were made
# conditional, one edit silently deleted six other steps. This test FAILS unless the job
# has, UNCONDITIONALLY, the steps that must run on every change, and the audit steps gated
# exactly on the advisory decision. It mutation-checks itself: for each required step it
# deletes that step from a scratch copy of ci.yml (and separately adds a skipping `if:`) and
# requires the check to fail on the mutant.
#
# "Unconditional" means: no `if:` at all, or exactly `steps.visibility.outputs.run == 'true'`,
# which is true for every pull_request, merge_group, push and dispatch (it is false only for a
# scheduled run on a private repository). Any other condition could skip the step on a normal
# pull request or merge group and fails the test.

# The `check && pass || fail` lines are if-then-else on purpose: pass() always
# returns 0, so fail() runs only when the check failed.
# shellcheck disable=SC2015
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
CI="$REPO_ROOT/.github/workflows/ci.yml"
[ -f "$CI" ] || { echo "missing: $CI" >&2; exit 1; }

TMP="$(cd "$(mktemp -d)" && pwd -P)"
trap 'cd /; find "$TMP" -depth -mindepth 1 \( -type f -o -type l \) -exec unlink {} \; 2>/dev/null; find "$TMP" -depth -type d -exec rmdir {} \; 2>/dev/null' EXIT

FAILURES=0
CASES=0
pass() { echo "    ok   — $*"; }
fail() { echo "    FAIL — $*" >&2; FAILURES=$((FAILURES + 1)); }
case_start() { CASES=$((CASES + 1)); echo; echo "[$CASES] $*"; }

VISIBLE="steps.visibility.outputs.run == 'true'"
GATED="steps.visibility.outputs.run == 'true' && steps.deps.outputs.advisory == 'true'"

# Steps that must run on every change.
ALWAYS=(
  "Install gitleaks"
  "Secret scan (gitleaks)"
  "Real-value scan (bank names, SSN/card shapes)"
  "ADR tier check (business phrases in docs/adr, docs/research)"
  "Dependency license scan — Rust (cargo deny)"
  "Install frontend dependencies"
  "Dependency license scan — frontend"
)
# Steps that must be gated on the advisory decision.
GATED_STEPS=(
  "Dependency scan — Rust (cargo audit)"
  "Dependency scan — frontend (pnpm audit)"
)

# job_text <file>: the security-scan job (its key to the next top-level job).
job_text() {
  awk '/^  security-scan:/ { on = 1; print; next } on && /^  [a-z0-9-]+:$/ { exit } on { print }' "$1"
}

# step_block <file> <step name>: one step (its "- name:" line to the next step line).
step_block() {
  job_text "$1" | awk -v n="      - name: $2" '
    $0 == n { on = 1; print; next }
    on && /^      - / { exit }
    on { print }'
}

# step_if <file> <step name>: the step's `if:` value, or empty.
step_if() {
  step_block "$1" "$2" | sed -n 's/^        if: //p' | head -n 1
}

# check <file>: prints one line per problem; no output means the file is fine.
check() {
  local f="$1" s cond job
  # Match with bash, not `job_text | grep -q`: under `pipefail`, grep -q exits at the first match,
  # the producer can get SIGPIPE, and the pipeline then reports failure on a file that is fine
  # (it did, on the hosted Linux runner: "the decision step is missing").
  job="$(job_text "$f")"
  [[ "$job" == *"tool: cargo-audit,cargo-deny"* ]] || echo "the cargo-audit,cargo-deny install is missing"
  grep -qF "name: Security scanning (secrets + dependencies)" "$f" || echo "the required check name changed or the job is gone"
  [[ "$job" == *"      - name: Decide whether the advisory scans are required"* ]] || echo "the decision step is missing"
  [[ "$job" == *"      - name: Advisory scans not required for this change"* ]] || echo "the explicit not-required step is missing"
  for s in "${ALWAYS[@]}"; do
    if [ -z "$(step_block "$f" "$s")" ]; then
      echo "step MISSING: $s"
    else
      cond="$(step_if "$f" "$s")"
      if [ -n "$cond" ] && [ "$cond" != "$VISIBLE" ]; then echo "step is conditional ($cond): $s"; fi
    fi
  done
  for s in "${GATED_STEPS[@]}"; do
    if [ -z "$(step_block "$f" "$s")" ]; then
      echo "step MISSING: $s"
    else
      cond="$(step_if "$f" "$s")"
      [ "$cond" = "$GATED" ] || echo "step is not gated exactly on the advisory decision (got: ${cond:-none}): $s"
    fi
  done
}

# remove_step <file> <step name> <out>: a copy of the file without that step.
remove_step() {
  awk -v n="      - name: $2" '
    /^  security-scan:/ { injob = 1 }
    injob && $0 == n { skipping = 1; next }
    skipping && /^      (- |#)/ { skipping = 0 }
    skipping && /^  [a-z0-9-]+:$/ { skipping = 0 }
    !skipping { print }' "$1" > "$3"
}

# add_skip <file> <step name> <out>: a copy where the step also skips on pull requests.
add_skip() {
  awk -v n="      - name: $2" '
    $0 == n { inside = 1 }
    inside && /^        if: / { sub(/$/, " \\&\\& github.event_name != '"'"'pull_request'"'"'"); inside = 0 }
    { print }' "$1" > "$3"
}

case_start "the real ci.yml passes the guard"
problems="$(check "$CI")"
[ -z "$problems" ] && pass "every always-run step is present and unconditional; the audit steps are gated; the names are right" || { fail "problems in the real ci.yml:"; printf '%s\n' "$problems" | sed 's/^/        /' >&2; }

case_start "mutation check: deleting any required step makes the guard fail"
for s in "${ALWAYS[@]}" "${GATED_STEPS[@]}"; do
  remove_step "$CI" "$s" "$TMP/mutant.yml"
  if cmp -s "$CI" "$TMP/mutant.yml"; then
    fail "the mutation tool did not remove: $s"
  elif [ -n "$(check "$TMP/mutant.yml")" ]; then
    pass "deleting '$s' is caught"
  else
    fail "deleting '$s' is NOT caught"
  fi
done

case_start "mutation check: deleting the (unnamed) cargo-audit/cargo-deny tool install makes the guard fail"
awk '
  /^  security-scan:/ { injob = 1 }
  injob && /^      - uses: taiki-e\/install-action@/ { skipping = 1; next }
  skipping && /^      (- |#)/ { skipping = 0 }
  !skipping { print }' "$CI" > "$TMP/mutant.yml"
if cmp -s "$CI" "$TMP/mutant.yml"; then fail "the mutation tool did not remove the tool install"
elif [ -n "$(check "$TMP/mutant.yml")" ]; then pass "deleting the tool install is caught"
else fail "deleting the tool install is NOT caught"; fi

case_start "mutation check: making a required step skip on pull requests makes the guard fail"
for s in "${ALWAYS[@]}" "${GATED_STEPS[@]}"; do
  add_skip "$CI" "$s" "$TMP/mutant.yml"
  if cmp -s "$CI" "$TMP/mutant.yml"; then
    # A step with no `if:` has nothing to append to; give it one.
    awk -v n="      - name: $s" '$0 == n { print; print "        if: github.event_name != '"'"'pull_request'"'"'"; next } { print }' "$CI" > "$TMP/mutant.yml"
  fi
  if [ -n "$(check "$TMP/mutant.yml")" ]; then pass "a skipping condition on '$s' is caught"; else fail "a skipping condition on '$s' is NOT caught"; fi
done

echo
if [ "$FAILURES" -eq 0 ]; then
  echo "PASS — $CASES cases, 0 failures"
  exit 0
fi
echo "FAIL — $CASES cases, $FAILURES failed assertions" >&2
exit 1
