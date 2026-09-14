#!/usr/bin/env bash
#
# Regression test for scripts/value-scan.sh — bead personal-cfo-o1nxk.
#
# WHAT IT COVERS
#   value-scan.sh runs `git grep` over a repo's tracked tree looking for real
#   institution names, SSN-shaped strings, and card-number-shaped strings.
#   Real bugs shipped TWICE (both caught in review, PR #418): round 1 — the
#   script's own denylist literally NAMES every pattern it looks for, and the
#   review document that reports its results QUOTES both the denylist and the
#   CapitalOne exception's rationale, so once both files were committed
#   (making them visible to `git grep`, which never sees untracked files) the
#   script matched itself. Round 2 — the FIX for round 1 excluded the script
#   and the doc but not the new test file THIS COMMENT lives in, which plants
#   the same denylist words and exception strings as fixture content, so once
#   this file was committed it self-matched too. This test proves, in a
#   throwaway repo (never touching this real repo's tree):
#     (a) a clean repo containing copies of ALL THREE self-referential files
#         (the script, a review-doc excerpt, AND this test file itself —
#         exactly the complete shape that broke, twice, at two different
#         points) → exit 0 — the regression test for both actual bugs
#     (b) a real institution name planted in an unexcepted file → exit 1,
#         naming the pattern and the file
#     (c) an SSN-shaped string planted → exit 1
#     (d) a card-number-shaped string planted → exit 1
#     (e) "capitalone" inside one of the documented exception paths → exit 0
#         (the exception logic itself, not just default-deny)
#     (f) "capitalone" OUTSIDE any exception path → exit 1 (the exception is
#         scoped to specific files, not the whole tree)
#     (g) a hyphenated UUID fixture (this codebase's real ID scheme) does NOT
#         false-positive as a card number — a second real bug the hermetic
#         suite caught: the original boundary regex matched a UUID's own
#         "-NNNN-NNNN-NNNN-" internal run
#     (h) a Cargo.lock-shaped SHA256 checksum does NOT false-positive as a
#         card number — checksums are long enough to contain a 16-digit run
#         by chance; *.lock files are excluded entirely
#     (i) the redaction-fixture exception (mirroring .gitleaks.toml's own
#         allowlist) lets a known, deliberate fake-card-number test fixture
#         pass, while the SAME string in a non-excepted file still fails
#
# HOW IT IS TESTED
#   A real throwaway git repo (value-scan.sh requires `git grep`, which only
#   searches a real repository's tracked content) built fresh per case in a
#   temp directory, with the actual scripts/value-scan.sh AND this test file
#   itself both copied in at their real relative paths (so path-relative
#   exceptions/exclusions in the script under test resolve the same way they
#   do in the real repo — and so the fixture always contains the complete,
#   current self-referential set, not a stale snapshot of it) and committed,
#   then case-specific files added/committed on top.
#
# RUN
#   bash scripts/tests/value-scan.test.sh
#
#   KEEP_WORKDIR=1 leaves the temp tree in place for inspection.

set -uo pipefail   # deliberately not -e: every case must run and report.

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
SCRIPT_UNDER_TEST="$REPO_ROOT/scripts/value-scan.sh"
[ -f "$SCRIPT_UNDER_TEST" ] || { echo "no script at $SCRIPT_UNDER_TEST" >&2; exit 1; }
# This test file itself — copied into every fixture repo below (see
# new_case_repo) so case 1 reproduces the COMPLETE self-referential set
# (script + doc excerpt + this test), not just the first two. A prior
# version of this test only copied the script and a doc excerpt, which
# missed a real bug in review (personal-cfo-o1nxk PR #418, round 2): the
# script excluded itself and the review doc from its own scan, but not this
# test file, which plants the same denylist words and exception strings as
# fixture content — so once THIS file was committed, it self-matched too.
SELF="$REPO_ROOT/scripts/tests/value-scan.test.sh"
[ -f "$SELF" ] || { echo "no test file at $SELF" >&2; exit 1; }

WORKDIR="$(mktemp -d "${TMPDIR:-/tmp}/value-scan-test.XXXXXX")" || exit 1
cleanup() {
  if [ -n "${KEEP_WORKDIR:-}" ]; then
    echo "workdir kept: $WORKDIR"
    return
  fi
  # Guarded: only ever removes the directory this run created, under the
  # temp root, with the name this script chose (AGENTS.md §1).
  case "$WORKDIR" in
    */value-scan-test.??????) rm -rf "$WORKDIR" ;;
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
# test committed at scripts/value-scan.sh, a copy of the real review doc's
# relevant lines, AND a copy of this test file itself at
# scripts/tests/value-scan.test.sh (self-match shape fully preserved, all
# three self-referential files present) — so case (a) is a faithful,
# COMPLETE regression test of the actual bugs, not just "the script and one
# other file are fine."
new_case_repo() {
  CASE="$WORKDIR/case-$CASES"
  REPO="$CASE/repo"
  mkdir -p "$REPO/scripts/tests"
  cp "$SCRIPT_UNDER_TEST" "$REPO/scripts/value-scan.sh"
  chmod +x "$REPO/scripts/value-scan.sh"
  cp "$SELF" "$REPO/scripts/tests/value-scan.test.sh"
  chmod +x "$REPO/scripts/tests/value-scan.test.sh"
  mkdir -p "$REPO/docs/security"
  # A faithful excerpt, not the whole doc — enough to reproduce the actual
  # self-match (quotes the denylist AND the CapitalOne exception prose).
  cat > "$REPO/docs/security/release-review-v0.1.md" <<'DOC_EOF'
git grep -niE "chase\.com|wellsfargo|bankofamerica|citibank|schwab\.com|fidelity\.com|vanguard\.com" -- <same scope>
documented exception: `CapitalOne` appears legitimately in
"CapitalOne" pattern, only in these four files — confirmed a synthetic
DOC_EOF
  git -C "$REPO" init -q
  git -C "$REPO" config user.email "test@example.invalid"
  git -C "$REPO" config user.name "value-scan test"
  git -C "$REPO" add -A
  git -C "$REPO" commit -q -m "initial"
}

run_script() {  # commits any new files first, then runs the script
  git -C "$REPO" add -A
  git -C "$REPO" commit -q -m "case content" --allow-empty
  ( cd "$REPO" && bash scripts/value-scan.sh ) \
    >"$CASE/stdout.txt" 2>"$CASE/stderr.txt"
  RC=$?
  STDOUT="$(cat "$CASE/stdout.txt")"
  STDERR="$(cat "$CASE/stderr.txt")"
}

# ---------------------------------------------------------------------------
case_start "clean repo — script + review-doc excerpt + this test itself (both actual regressions)"
new_case_repo
run_script
assert_eq "exit 0" "$RC" "0"
assert_contains "reports clean" "$STDOUT" "value-scan: clean"

# ---------------------------------------------------------------------------
case_start "a real institution name in an unexcepted file fails, naming it"
new_case_repo
mkdir -p "$REPO/crates/some-other-crate/src"
echo '// customer uses wellsfargo for their checking account' \
  > "$REPO/crates/some-other-crate/src/lib.rs"
run_script
assert_eq "exit 1" "$RC" "1"
assert_contains "names the pattern" "$STDERR" "FOUND: real institution pattern 'wellsfargo'"
assert_contains "shows the offending file" "$STDOUT" "some-other-crate/src/lib.rs"

# ---------------------------------------------------------------------------
case_start "an SSN-shaped string fails"
new_case_repo
mkdir -p "$REPO/docs/notes"
echo 'test SSN: 123-45-6789' > "$REPO/docs/notes/scratch.md"
run_script
assert_eq "exit 1" "$RC" "1"
assert_contains "names SSN finding" "$STDERR" "FOUND: SSN-shaped string"

# ---------------------------------------------------------------------------
case_start "a card-number-shaped string fails"
new_case_repo
mkdir -p "$REPO/docs/notes"
echo 'card on file: 4111 1111 1111 1111' > "$REPO/docs/notes/scratch.md"
run_script
assert_eq "exit 1" "$RC" "1"
assert_contains "names card finding" "$STDERR" "FOUND: card-number-shaped string"

# ---------------------------------------------------------------------------
case_start "capitalone inside a documented exception path passes"
new_case_repo
mkdir -p "$REPO/crates/importers/csv-importer/src"
echo '// Real CapitalOne credit-card export shape (ADR 0045), synthetic data only' \
  > "$REPO/crates/importers/csv-importer/src/lib.rs"
run_script
assert_eq "exit 0" "$RC" "0"
assert_contains "reports clean" "$STDOUT" "value-scan: clean"

# ---------------------------------------------------------------------------
case_start "capitalone OUTSIDE any exception path still fails (exception is scoped, not blanket)"
new_case_repo
mkdir -p "$REPO/crates/unrelated-crate/src"
echo '// mentions capitalone here, not an exception path' \
  > "$REPO/crates/unrelated-crate/src/lib.rs"
run_script
assert_eq "exit 1" "$RC" "1"
assert_contains "names the pattern" "$STDERR" "FOUND: real institution pattern 'capitalone'"

# ---------------------------------------------------------------------------
case_start "a hyphenated UUID fixture does not false-positive as a card number"
new_case_repo
mkdir -p "$REPO/apps/desktop/src/accounts"
echo 'const id = "0190a000-0000-7000-8000-000000000001";' \
  > "$REPO/apps/desktop/src/accounts/fixture.test.tsx"
run_script
assert_eq "exit 0" "$RC" "0"
assert_contains "reports clean" "$STDOUT" "value-scan: clean"

# ---------------------------------------------------------------------------
case_start "a Cargo.lock-shaped checksum does not false-positive as a card number"
new_case_repo
cat > "$REPO/Cargo.lock" <<'LOCK_EOF'
[[package]]
name = "example"
version = "1.0.0"
checksum = "41f2619966050689382d2b44f664f4bc593e129785a36d6ee376ddf37259b924"
LOCK_EOF
run_script
assert_eq "exit 0" "$RC" "0"
assert_contains "reports clean" "$STDOUT" "value-scan: clean"

# ---------------------------------------------------------------------------
case_start "the redaction-fixture exception lets a known fake card number pass"
new_case_repo
mkdir -p "$REPO/crates/observability/src"
echo 'const NAME_EMBEDDED_NUMBER: &str = "4111111111111111";' \
  > "$REPO/crates/observability/src/lib.rs"
run_script
assert_eq "exit 0" "$RC" "0"
assert_contains "reports clean" "$STDOUT" "value-scan: clean"

case_start "...but the SAME fake card number OUTSIDE that exact file still fails"
new_case_repo
mkdir -p "$REPO/crates/some-other-crate/src"
echo 'const NAME_EMBEDDED_NUMBER: &str = "4111111111111111";' \
  > "$REPO/crates/some-other-crate/src/lib.rs"
run_script
assert_eq "exit 1" "$RC" "1"
assert_contains "names card finding" "$STDERR" "FOUND: card-number-shaped string"

# ---------------------------------------------------------------------------
echo
if [ "$FAILURES" -eq 0 ]; then
  echo "PASS — $CASES cases, 0 failures"
  exit 0
fi
echo "FAIL — $CASES cases, $FAILURES failed assertions" >&2
exit 1
