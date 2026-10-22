#!/usr/bin/env bash
#
# Guard for the required `Security scanning (secrets + dependencies)` job in
# .github/workflows/ci.yml (personal-cfo-md8h2.14). When the advisory scans were made
# conditional, one edit silently deleted six other steps (the tool install, gitleaks, the
# real-value scan, the ADR tier check). This test fails if any step that must run on EVERY
# change goes missing from that job, or becomes conditional on the advisory decision.

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

# The security-scan job's text: from its key to the next top-level job.
JOB="$(awk '/^  security-scan:/ { on = 1; print; next } on && /^  [a-z0-9-]+:$/ { exit } on { print }' "$CI")"
[ -n "$JOB" ] || { echo "no security-scan job in $CI" >&2; exit 1; }

has() { printf '%s\n' "$JOB" | grep -qF -- "$1"; }

# step_block <step name>: the lines of one step (its "- name:" line to the next "- " step line).
step_block() {
  printf '%s\n' "$JOB" | awk -v n="      - name: $1" '
    $0 == n { on = 1; print; next }
    on && /^      - / { exit }
    on { print }'
}

case_start "the required check name is unchanged"
grep -qF "name: Security scanning (secrets + dependencies)" "$CI" && pass "job name is 'Security scanning (secrets + dependencies)'" || fail "job name changed"

case_start "every step that must run on EVERY change is still in the job"
for step in \
  "Install gitleaks" \
  "Secret scan (gitleaks)" \
  "Real-value scan (bank names, SSN/card shapes)" \
  "ADR tier check (business phrases in docs/adr, docs/research)" \
  "Dependency license scan — Rust (cargo deny)" \
  "Install frontend dependencies" \
  "Dependency license scan — frontend"; do
  has "      - name: $step" && pass "step present: $step" || fail "step MISSING from security-scan: $step"
done
has "tool: cargo-audit,cargo-deny" && pass "cargo-audit and cargo-deny are installed" || fail "the cargo-audit,cargo-deny install is missing"

case_start "those steps are not conditional on the advisory decision"
for step in \
  "Install gitleaks" \
  "Secret scan (gitleaks)" \
  "Real-value scan (bank names, SSN/card shapes)" \
  "ADR tier check (business phrases in docs/adr, docs/research)" \
  "Dependency license scan — Rust (cargo deny)" \
  "Install frontend dependencies" \
  "Dependency license scan — frontend"; do
  if step_block "$step" | grep -q "steps.deps"; then fail "'$step' depends on steps.deps (it must always run)"; else pass "'$step' always runs"; fi
done

case_start "the advisory scans are the only conditional ones, and the not-required path reports success"
step_block "Dependency scan — Rust (cargo audit)" | grep -q "steps.deps.outputs.advisory == 'true'" && pass "cargo audit is conditional on the decision" || fail "cargo audit is not gated"
step_block "Dependency scan — frontend (pnpm audit)" | grep -q "steps.deps.outputs.advisory == 'true'" && pass "pnpm audit is conditional on the decision" || fail "pnpm audit is not gated"
has "      - name: Advisory scans not required for this change" && pass "an explicit 'not required' step exists" || fail "no explicit not-required step"
has "      - name: Decide whether the advisory scans are required" && pass "the decision step exists" || fail "the decision step is missing"

echo
if [ "$FAILURES" -eq 0 ]; then
  echo "PASS — $CASES cases, 0 failures"
  exit 0
fi
echo "FAIL — $CASES cases, $FAILURES failed assertions" >&2
exit 1
