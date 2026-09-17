#!/usr/bin/env bash
#
# Regression test for scripts/adr-tier-check.sh — ADR 0082, bead
# personal-cfo-0uxwt.
#
# WHAT IT COVERS
#   adr-tier-check.sh runs `git grep` over docs/adr/ and docs/research/
#   looking for a fixed business-phrase list (pricing, commission, revenue,
#   etc.) that must never enter the public repo (ADR 0082, decision 4). This
#   test proves, in a throwaway repo (never touching this real repo's tree):
#     (a) a clean repo containing the script, this test file itself, and a
#         copy of ADR 0082/0066's own phrase-naming text (the same
#         self-referential shape scripts/value-scan.test.sh guards against —
#         see its own header for why this matters) → exit 0
#     (b) a planted business phrase in an unexcepted docs/adr/ file → exit 1,
#         naming the pattern and the file (this is AC4's "a deliberately
#         planted phrase fails the check")
#     (c) the same phrase in docs/research/ → exit 1 (both directories are
#         in scope)
#     (d) a documented exception file containing an excepted phrase → exit 0
#     (e) the SAME phrase OUTSIDE any exception path → exit 1 (the exception
#         is scoped to specific files, not the whole tree)
#     (f) the same phrase in a file OUTSIDE docs/adr/ and docs/research/
#         entirely → exit 0 (the check's scope is those two directories only)
#
# HOW IT IS TESTED
#   A real throwaway git repo built fresh per case, with the actual
#   scripts/adr-tier-check.sh AND this test file copied in at their real
#   relative paths, committed, then case-specific files added on top —
#   same shape as scripts/tests/value-scan.test.sh.
#
# RUN
#   bash scripts/tests/adr-tier-check.test.sh
#
#   KEEP_WORKDIR=1 leaves the temp tree in place for inspection.

set -uo pipefail   # deliberately not -e: every case must run and report.

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
SCRIPT_UNDER_TEST="$REPO_ROOT/scripts/adr-tier-check.sh"
[ -f "$SCRIPT_UNDER_TEST" ] || { echo "no script at $SCRIPT_UNDER_TEST" >&2; exit 1; }
SELF="$REPO_ROOT/scripts/tests/adr-tier-check.test.sh"
[ -f "$SELF" ] || { echo "no test file at $SELF" >&2; exit 1; }

WORKDIR="$(mktemp -d "${TMPDIR:-/tmp}/adr-tier-check-test.XXXXXX")" || exit 1
cleanup() {
  if [ -n "${KEEP_WORKDIR:-}" ]; then
    echo "workdir kept: $WORKDIR"
    return
  fi
  case "$WORKDIR" in
    */adr-tier-check-test.??????) rm -rf "$WORKDIR" ;;
    *) echo "refusing to remove unexpected workdir: $WORKDIR" >&2 ;;
  esac
}
trap cleanup EXIT

FAILURES=0
CASES=0
pass() { echo "    ok   — $*"; }
fail() { echo "    FAIL — $*" >&2; FAILURES=$((FAILURES + 1)); }
assert_eq() {  # <label> <actual> <expected>
  if [ "$2" = "$3" ]; then pass "$1"; else fail "$1 (expected '$3', got '$2')"; fi
}
assert_contains() {  # <label> <haystack> <needle>
  case "$2" in *"$3"*) pass "$1" ;; *) fail "$1 (no '$3' in: $2)" ;; esac
}
case_start() { CASES=$((CASES + 1)); echo; echo "[$CASES] $*"; }

# Builds a fresh throwaway git repo at $CASE/repo with the real script under
# test, this test file itself, and a stand-in for the two exception-listed
# ADRs (0082 and 0066) — real docs/adr/ and docs/research/ directories exist
# so the check's own pathspec has something to scope against.
new_case_repo() {
  CASE="$WORKDIR/case-$CASES"
  REPO="$CASE/repo"
  mkdir -p "$REPO/scripts/tests" "$REPO/docs/adr" "$REPO/docs/research"
  cp "$SCRIPT_UNDER_TEST" "$REPO/scripts/adr-tier-check.sh"
  chmod +x "$REPO/scripts/adr-tier-check.sh"
  cp "$SELF" "$REPO/scripts/tests/adr-tier-check.test.sh"
  chmod +x "$REPO/scripts/tests/adr-tier-check.test.sh"
  # Faithful excerpt of the two allowlisted ADRs' own vocabulary — enough to
  # reproduce a self-match if the exclusion list ever regresses.
  cat > "$REPO/docs/adr/0082-public-disclosure-boundary.md" <<'DOC_EOF'
phrase list: "$/month", "per month", "Stripe", "price", "pricing",
"commission", "revenue", "cost table", "subscriber", "invoice"
DOC_EOF
  cat > "$REPO/docs/adr/0066-business-model-free-app-paid-services.md" <<'DOC_EOF'
the business model: a paid subscriber pays a monthly price via Stripe,
generating revenue against a cost table, with an invoice on file.
DOC_EOF
  git -C "$REPO" init -q
  git -C "$REPO" config user.email "test@example.invalid"
  git -C "$REPO" config user.name "adr-tier-check test"
  git -C "$REPO" add -A
  git -C "$REPO" commit -q -m "initial"
}

run_script() {
  git -C "$REPO" add -A
  git -C "$REPO" commit -q -m "case content" --allow-empty
  ( cd "$REPO" && bash scripts/adr-tier-check.sh ) \
    >"$CASE/stdout.txt" 2>"$CASE/stderr.txt"
  RC=$?
  STDOUT="$(cat "$CASE/stdout.txt")"
  STDERR="$(cat "$CASE/stderr.txt")"
}

# ---------------------------------------------------------------------------
case_start "clean repo — script + this test + the two allowlisted ADRs' own vocabulary"
new_case_repo
run_script
assert_eq "exit 0" "$RC" "0"
assert_contains "reports clean" "$STDOUT" "adr-tier-check: clean"

# ---------------------------------------------------------------------------
case_start "a planted business phrase in an unexcepted docs/adr/ file fails, naming it"
new_case_repo
echo '- **Decision:** the Sync tier is $12/month, billed via Stripe.' \
  > "$REPO/docs/adr/0090-new-feature.md"
run_script
assert_eq "exit 1" "$RC" "1"
assert_contains "names a matched pattern" "$STDERR" "FOUND: business phrase"
assert_contains "shows the offending file" "$STDOUT" "0090-new-feature.md"

# ---------------------------------------------------------------------------
case_start "the same phrase in docs/research/ also fails (both dirs in scope)"
new_case_repo
echo 'Projected commission from the affiliate program: see the cost table below.' \
  > "$REPO/docs/research/new-research.md"
run_script
assert_eq "exit 1" "$RC" "1"
assert_contains "shows the offending file" "$STDOUT" "new-research.md"

# ---------------------------------------------------------------------------
case_start "a documented exception file containing an excepted phrase passes"
new_case_repo
mkdir -p "$REPO/docs/adr"
echo 'On a free tier of build-minutes per month, hosting costs nothing.' \
  > "$REPO/docs/adr/0061-website-stack-and-hosting.md"
run_script
assert_eq "exit 0" "$RC" "0"
assert_contains "reports clean" "$STDOUT" "adr-tier-check: clean"

# ---------------------------------------------------------------------------
case_start "the SAME phrase OUTSIDE any exception path still fails (scoped, not blanket)"
new_case_repo
echo 'Renews per month at the current rate.' \
  > "$REPO/docs/adr/0091-unrelated.md"
run_script
assert_eq "exit 1" "$RC" "1"
assert_contains "names the pattern" "$STDERR" "FOUND: business phrase 'per month'"

# ---------------------------------------------------------------------------
case_start "the same phrase in a file OUTSIDE docs/adr/ and docs/research/ is out of scope"
new_case_repo
mkdir -p "$REPO/crates/some-crate/src"
echo '// Stripe-shaped test fixture, commission rate, $9/month' \
  > "$REPO/crates/some-crate/src/lib.rs"
run_script
assert_eq "exit 0" "$RC" "0"
assert_contains "reports clean" "$STDOUT" "adr-tier-check: clean"

# ---------------------------------------------------------------------------
echo
if [ "$FAILURES" -eq 0 ]; then
  echo "PASS — $CASES cases, 0 failures"
  exit 0
fi
echo "FAIL — $CASES cases, $FAILURES failed assertions" >&2
exit 1
