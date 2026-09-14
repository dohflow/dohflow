#!/usr/bin/env bash
#
# Regression test for scripts/capture-shot.sh — bead personal-cfo-n76x.13.
#
# WHAT IT COVERS
#   capture-shot.sh drives macOS window mechanics from a single slug
#   argument: ONE osascript/System Events call brings the frontmost DohFlow
#   window forward, resizes/positions it, and reads back whatever rectangle
#   ACTUALLY resulted (not necessarily the requested one — see the script's
#   own "WHY READ BACK" header comment), then `screencapture -o -R <rect>`
#   captures exactly that region. This test proves, without ever touching a
#   real screen, a real DohFlow window, or requiring the Accessibility grant
#   osascript needs on a real run:
#     (a) --help / no-args print usage and exit 0
#     (b) --list prints exactly the fourteen valid slugs
#     (c) an unknown slug is rejected (exit 2) with the valid list on stderr
#     (d) too many arguments is rejected (exit 2)
#     (e) a missing `screencapture` binary is reported distinctly (exit 1),
#         checked BEFORE osascript ever runs
#     (f) the happy path: the single osascript call is invoked with the
#         resize/frontmost script, its returned rect is parsed, and
#         screencapture is invoked with exactly `-o -R <rect> <path>` — no
#         -C (cursor), no warning printed, since the returned rect matches
#         what was requested
#     (g) the actual-size-differs path: when the returned rect is SMALLER
#         than requested (the real, verified behavior on a machine with a
#         visible Dock), the script still succeeds, still captures at the
#         ACTUAL rect, and prints the Dock/menu-bar warning on stderr
#     (h) an osascript failure (simulating a missing Accessibility grant)
#         exits 1 with the guidance message, and screencapture is never
#         invoked
#     (i) osascript succeeding but returning an empty rectangle is reported
#         distinctly, and screencapture is never invoked
#     (j) a screencapture failure (after a successful resize) exits 1
#
# HOW IT IS TESTED
#   Stub `osascript` and `screencapture` on PATH. The osascript stub reads
#   its AppleScript from stdin (heredoc, matching how the script feeds it)
#   and returns a canned rect string or fails, per $OSASCRIPT_STUB_MODE —
#   there is only ONE osascript call in the real script now (see WHAT IT
#   COVERS), so the stub does not need to dispatch on script content the way
#   an earlier, two-call version of this script needed to. PATH deliberately
#   excludes /usr/sbin (where the real screencapture lives) for case (e).
#   `CAPTURE_SHOT_OUT_DIR` redirects the output file into the test's own
#   scratch directory (see the script's own header — testing only, never
#   production) so this test never writes into this repo's real
#   apps/desktop/screenshots/raw/.
#
# RUN
#   bash scripts/tests/capture-shot.test.sh
#
#   KEEP_WORKDIR=1 leaves the temp tree in place for inspection.

set -uo pipefail   # deliberately not -e: every case must run and report.

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
SCRIPT_UNDER_TEST="$REPO_ROOT/scripts/capture-shot.sh"
[ -f "$SCRIPT_UNDER_TEST" ] || { echo "no script at $SCRIPT_UNDER_TEST" >&2; exit 1; }

WORKDIR="$(mktemp -d "${TMPDIR:-/tmp}/capture-shot-test.XXXXXX")" || exit 1
cleanup() {
  if [ -n "${KEEP_WORKDIR:-}" ]; then
    echo "workdir kept: $WORKDIR"
    return
  fi
  # Guarded: only ever removes the directory this run created, under the
  # temp root, with the name this script chose (AGENTS.md §1).
  case "$WORKDIR" in
    */capture-shot-test.??????) rm -rf "$WORKDIR" ;;
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
assert_not_contains() {  # <label> <haystack> <needle>
  case "$2" in *"$3"*) fail "$1 (unexpectedly found '$3' in: $2)" ;; *) pass "$1" ;; esac
}
case_start() { CASES=$((CASES + 1)); echo; echo "[$CASES] $*"; }

STUB_BIN="$WORKDIR/bin"
mkdir -p "$STUB_BIN"

# Deterministic `osascript` stub — the script's single call, which resizes,
# brings frontmost, and returns "x,y,w,h" on success. Behavior selected by
# $OSASCRIPT_STUB_MODE:
#   success       — prints $OSASCRIPT_STUB_RECT (default "20,38,1440,900")
#   empty         — exits 0 but prints nothing (the "no rectangle" edge case)
#   fails         — exits 1, prints nothing (simulates a missing
#                   Accessibility grant, the real failure shape)
write_osascript_stub() {  # <mode>
  cat > "$STUB_BIN/osascript" <<OSASCRIPT_EOF
#!/usr/bin/env bash
MODE="$1"
INPUT="\$(cat)"
echo "\$INPUT" >> "\$OSASCRIPT_STUB_LOG"
case "\$MODE" in
  fails) exit 1 ;;
  empty) exit 0 ;;
  success) echo "\${OSASCRIPT_STUB_RECT:-20,38,1440,900}"; exit 0 ;;
  *) echo "unknown OSASCRIPT_STUB_MODE: \$MODE" >&2; exit 9 ;;
esac
OSASCRIPT_EOF
  chmod +x "$STUB_BIN/osascript"
}

# Deterministic `screencapture` stub. Records its full argv and, on success,
# writes a dummy file at the last argument (the output path).
# Behavior selected by $SCREENCAPTURE_STUB_MODE: success | fails.
write_screencapture_stub() {  # <mode>
  cat > "$STUB_BIN/screencapture" <<CAPTURE_EOF
#!/usr/bin/env bash
MODE="$1"
echo "\$*" >> "\$SCREENCAPTURE_STUB_LOG"
if [ "\$MODE" = "fails" ]; then
  echo "simulated screencapture failure" >&2
  exit 1
fi
out="\${@: -1}"
echo "fake-png-bytes" > "\$out"
exit 0
CAPTURE_EOF
  chmod +x "$STUB_BIN/screencapture"
}

# Full PATH (real screencapture reachable) vs. one that omits /usr/sbin,
# where the real binary lives, for the "not installed" case.
FULL_PATH="$STUB_BIN:/usr/bin:/bin:/usr/sbin:/sbin"
NO_SBIN_PATH="$STUB_BIN:/usr/bin:/bin"

run_script() {  # runs with $FULL_PATH and the given args; captures stdout/stderr/rc
  local case_dir="$WORKDIR/case-$CASES"
  mkdir -p "$case_dir"
  ( PATH="$FULL_PATH" bash "$SCRIPT_UNDER_TEST" "$@" ) \
    >"$case_dir/stdout.txt" 2>"$case_dir/stderr.txt"
  echo $? > "$case_dir/rc.txt"
  STDOUT="$(cat "$case_dir/stdout.txt")"
  STDERR="$(cat "$case_dir/stderr.txt")"
  RC="$(cat "$case_dir/rc.txt")"
}

# ---------------------------------------------------------------------------
case_start "--help prints usage and exits 0"
run_script --help
assert_eq "exit 0" "$RC" "0"
assert_contains "mentions the slug argument" "$STDOUT" "<slug>"
assert_contains "mentions --list" "$STDOUT" "--list"

# ---------------------------------------------------------------------------
case_start "no arguments prints usage and exits 0 (same as --help)"
run_script
assert_eq "exit 0" "$RC" "0"
assert_contains "mentions the slug argument" "$STDOUT" "<slug>"

# ---------------------------------------------------------------------------
case_start "--list prints exactly the fourteen valid slugs"
run_script --list
assert_eq "exit 0" "$RC" "0"
LIST_COUNT="$(echo "$STDOUT" | grep -c . || true)"
assert_eq "fourteen slugs" "$LIST_COUNT" "14"
assert_contains "includes dashboard-light" "$STDOUT" "dashboard-light"
assert_contains "includes settings-about-dark" "$STDOUT" "settings-about-dark"

# ---------------------------------------------------------------------------
case_start "an unknown slug is rejected"
run_script nonexistent-slug
assert_eq "exit 2" "$RC" "2"
assert_contains "names the bad slug" "$STDERR" "nonexistent-slug"
assert_contains "lists valid slugs on stderr" "$STDERR" "dashboard-light"

# ---------------------------------------------------------------------------
case_start "too many arguments is rejected"
run_script dashboard-light extra-arg
assert_eq "exit 2" "$RC" "2"

# ---------------------------------------------------------------------------
case_start "a missing screencapture binary is reported distinctly, before osascript runs"
case_dir="$WORKDIR/case-$CASES"
mkdir -p "$case_dir"
( PATH="$NO_SBIN_PATH" bash "$SCRIPT_UNDER_TEST" dashboard-light ) \
  >"$case_dir/stdout.txt" 2>"$case_dir/stderr.txt"
RC=$?
STDERR="$(cat "$case_dir/stderr.txt")"
assert_eq "exit 1" "$RC" "1"
assert_contains "names screencapture" "$STDERR" "screencapture"

# ---------------------------------------------------------------------------
case_start "happy path: osascript returns the requested rect, screencapture invoked correctly, no warning"
write_osascript_stub "success"
write_screencapture_stub "success"
CASE="$WORKDIR/happy"
mkdir -p "$CASE"
export OSASCRIPT_STUB_LOG="$CASE/osascript-invocations.log"; : > "$OSASCRIPT_STUB_LOG"
export OSASCRIPT_STUB_RECT="20,38,1440,900"
export SCREENCAPTURE_STUB_LOG="$CASE/screencapture-invocations.log"; : > "$SCREENCAPTURE_STUB_LOG"
export CAPTURE_SHOT_OUT_DIR="$CASE/out"
STDOUT_LOG="$CASE/stdout.txt"
STDERR_LOG="$CASE/stderr.txt"
( PATH="$FULL_PATH" bash "$SCRIPT_UNDER_TEST" dashboard-light ) \
  >"$STDOUT_LOG" 2>"$STDERR_LOG"
RC=$?
assert_eq "exit 0" "$RC" "0"
assert_contains "osascript invoked with the resize/frontmost script" "$(cat "$OSASCRIPT_STUB_LOG")" "set frontmost to true"
# The log holds the FULL multi-line AppleScript per call, so count
# invocations by a marker that appears exactly once per call, not by line
# count (which counts every line of the script, not every call).
assert_eq "osascript invoked exactly once" \
  "$(grep -c 'tell application "System Events"' "$OSASCRIPT_STUB_LOG")" "1"
CAPTURE_ARGS="$(cat "$SCREENCAPTURE_STUB_LOG")"
assert_eq "screencapture invoked with exactly -o -R 20,38,1440,900 <path>" \
  "$CAPTURE_ARGS" "-o -R 20,38,1440,900 $CASE/out/dashboard-light.png"
assert_not_contains "no cursor flag (-C) ever passed" "$CAPTURE_ARGS" "-C"
[ -f "$CASE/out/dashboard-light.png" ] && pass "output file exists" || fail "output file missing"
assert_contains "confirms the captured path on stdout" "$(cat "$STDOUT_LOG")" "dashboard-light.png"
assert_not_contains "no size-mismatch warning when the rect matches" "$(cat "$STDERR_LOG")" "requested"
unset OSASCRIPT_STUB_LOG OSASCRIPT_STUB_RECT SCREENCAPTURE_STUB_LOG CAPTURE_SHOT_OUT_DIR

# ---------------------------------------------------------------------------
case_start "actual-size-differs: a smaller returned rect still succeeds, captures at the ACTUAL size, and warns"
write_osascript_stub "success"
write_screencapture_stub "success"
CASE="$WORKDIR/size-differs"
mkdir -p "$CASE"
export OSASCRIPT_STUB_LOG="$CASE/osascript-invocations.log"; : > "$OSASCRIPT_STUB_LOG"
# The real, verified shape on a machine with a visible Dock: requested
# 1440x900, landed at 1440x879.
export OSASCRIPT_STUB_RECT="20,38,1440,879"
export SCREENCAPTURE_STUB_LOG="$CASE/screencapture-invocations.log"; : > "$SCREENCAPTURE_STUB_LOG"
export CAPTURE_SHOT_OUT_DIR="$CASE/out"
STDOUT_LOG="$CASE/stdout.txt"
STDERR_LOG="$CASE/stderr.txt"
( PATH="$FULL_PATH" bash "$SCRIPT_UNDER_TEST" dashboard-light ) \
  >"$STDOUT_LOG" 2>"$STDERR_LOG"
RC=$?
assert_eq "exit 0 (still succeeds)" "$RC" "0"
assert_eq "screencapture invoked with the ACTUAL (879-tall) rect, not the requested 900" \
  "$(cat "$SCREENCAPTURE_STUB_LOG")" "-o -R 20,38,1440,879 $CASE/out/dashboard-light.png"
assert_contains "warns about the size mismatch on stderr" "$(cat "$STDERR_LOG")" "requested 1440x900, got 1440x879"
assert_contains "names the Dock/menu bar as the likely cause" "$(cat "$STDERR_LOG")" "Dock"
[ -f "$CASE/out/dashboard-light.png" ] && pass "output file exists" || fail "output file missing"
unset OSASCRIPT_STUB_LOG OSASCRIPT_STUB_RECT SCREENCAPTURE_STUB_LOG CAPTURE_SHOT_OUT_DIR

# ---------------------------------------------------------------------------
case_start "an osascript failure surfaces Accessibility guidance and never calls screencapture"
write_osascript_stub "fails"
write_screencapture_stub "success"
CASE="$WORKDIR/osascript-fails"
mkdir -p "$CASE"
export OSASCRIPT_STUB_LOG="$CASE/osascript-invocations.log"; : > "$OSASCRIPT_STUB_LOG"
export SCREENCAPTURE_STUB_LOG="$CASE/screencapture-invocations.log"; : > "$SCREENCAPTURE_STUB_LOG"
export CAPTURE_SHOT_OUT_DIR="$CASE/out"
STDERR_LOG="$CASE/stderr.txt"
( PATH="$FULL_PATH" bash "$SCRIPT_UNDER_TEST" dashboard-light ) \
  >"$CASE/stdout.txt" 2>"$STDERR_LOG"
RC=$?
assert_eq "exit 1" "$RC" "1"
assert_contains "mentions Accessibility" "$(cat "$STDERR_LOG")" "Accessibility"
assert_eq "screencapture never invoked" "$(cat "$SCREENCAPTURE_STUB_LOG")" ""
unset OSASCRIPT_STUB_LOG SCREENCAPTURE_STUB_LOG CAPTURE_SHOT_OUT_DIR

# ---------------------------------------------------------------------------
case_start "osascript succeeding with an empty rectangle is reported distinctly"
write_osascript_stub "empty"
write_screencapture_stub "success"
CASE="$WORKDIR/empty-rect"
mkdir -p "$CASE"
export OSASCRIPT_STUB_LOG="$CASE/osascript-invocations.log"; : > "$OSASCRIPT_STUB_LOG"
export SCREENCAPTURE_STUB_LOG="$CASE/screencapture-invocations.log"; : > "$SCREENCAPTURE_STUB_LOG"
export CAPTURE_SHOT_OUT_DIR="$CASE/out"
STDERR_LOG="$CASE/stderr.txt"
( PATH="$FULL_PATH" bash "$SCRIPT_UNDER_TEST" dashboard-light ) \
  >"$CASE/stdout.txt" 2>"$STDERR_LOG"
RC=$?
assert_eq "exit 1" "$RC" "1"
assert_contains "mentions no rectangle" "$(cat "$STDERR_LOG")" "no window rectangle"
assert_eq "screencapture never invoked" "$(cat "$SCREENCAPTURE_STUB_LOG")" ""
unset OSASCRIPT_STUB_LOG SCREENCAPTURE_STUB_LOG CAPTURE_SHOT_OUT_DIR

# ---------------------------------------------------------------------------
case_start "a screencapture failure (after a successful resize) exits 1"
write_osascript_stub "success"
write_screencapture_stub "fails"
CASE="$WORKDIR/capture-fails"
mkdir -p "$CASE"
export OSASCRIPT_STUB_LOG="$CASE/osascript-invocations.log"; : > "$OSASCRIPT_STUB_LOG"
export OSASCRIPT_STUB_RECT="20,38,1440,900"
export SCREENCAPTURE_STUB_LOG="$CASE/screencapture-invocations.log"; : > "$SCREENCAPTURE_STUB_LOG"
export CAPTURE_SHOT_OUT_DIR="$CASE/out"
STDERR_LOG="$CASE/stderr.txt"
( PATH="$FULL_PATH" bash "$SCRIPT_UNDER_TEST" dashboard-light ) \
  >"$CASE/stdout.txt" 2>"$STDERR_LOG"
RC=$?
assert_eq "exit 1" "$RC" "1"
assert_contains "names screencapture in the error" "$(cat "$STDERR_LOG")" "screencapture"
unset OSASCRIPT_STUB_LOG OSASCRIPT_STUB_RECT SCREENCAPTURE_STUB_LOG CAPTURE_SHOT_OUT_DIR

# ---------------------------------------------------------------------------
echo
if [ "$FAILURES" -eq 0 ]; then
  echo "PASS — $CASES cases, 0 failures"
  exit 0
fi
echo "FAIL — $CASES cases, $FAILURES failed assertions" >&2
exit 1
