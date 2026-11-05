#!/usr/bin/env bash
#
# Behavior test for scripts/macos-pools-wait.sh (personal-cfo-md8h2.26), run once for each of
# the two required checks that use it: the iOS kernel check (ios-kernel.yml) and the desktop
# crate's non-desktop target check (ci.yml). The pool names and labels are read from each
# workflow's own verdict step, so the test cannot drift from what the job passes.
#
# The wait script polls the run's jobs for a bounded time and hands what it saw to
# scripts/macos-pools-verdict.sh. Here every poll reads a fixture snapshot (a numbered file in
# a directory) and time is a virtual clock, so there is no network, no sleeping and no macOS.
# Each scenario states what the check must do:
#
#   * one pool built and the other is unfinished at the deadline: pass, with a notice;
#   * one pool failed before the deadline: fail, without waiting for the deadline;
#   * a pool's success never ends the wait on its own (the other may still fail);
#   * nothing built by the deadline: fail, and say it was the deadline;
#   * the API unreadable, a job missing or duplicated, a job of another attempt: no success;
#   * the build not wanted and both pools skipped: pass at once, no wait;
#   * a failed gate: fail without polling.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
WAIT="$REPO_ROOT/scripts/macos-pools-wait.sh"
VERDICT="$REPO_ROOT/scripts/macos-pools-verdict.sh"
IOS="$REPO_ROOT/.github/workflows/ios-kernel.yml"
CI="$REPO_ROOT/.github/workflows/ci.yml"

for file in "$WAIT" "$VERDICT" "$IOS" "$CI"; do
  [ -f "$file" ] || { echo "FAIL — missing file: $file" >&2; exit 1; }
done
command -v jq >/dev/null 2>&1 || { echo "FAIL — jq is required" >&2; exit 1; }

CASES=0
ok() { CASES=$((CASES + 1)); echo "    ok   — $1"; }
bad() { echo "    FAIL — $1" >&2; exit 1; }
has() { grep -Fq -- "$2" <<<"$1"; }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# job_text <file> <job id>: from its key to the next top-level job.
job_text() {
  awk -v key="  $2:" '$0 == key { on = 1; print; next } on && /^  [a-z0-9-]+:$/ { exit } on { print }' "$1"
}
env_of() { sed -n "s/^          $2: //p" <<<"$1" | head -n 1; }

# job <name> <status> <conclusion|null> [attempt]: one entry of a jobs response.
job() {
  jq -nc --arg name "$1" --arg status "$2" --arg conclusion "$3" --argjson attempt "${4:-1}" \
    '{name: $name, status: $status, conclusion: (if $conclusion == "null" then null else $conclusion end),
      run_attempt: $attempt, runner_id: (if $status == "queued" then null else 7 end),
      steps: (if $status == "queued" then [] else [{name: "Set up job"}] end)}'
}
# snapshot <job>...: a jobs response.
snapshot() { printf '%s\n' "$@" | jq -sc '{total_count: length, jobs: .}'; }
# sequence <dir> <snapshot>...: poll n reads the n-th snapshot (the last one repeats).
sequence() {
  local dir="$1" n=0 snap
  shift
  rm -rf "$dir"
  mkdir -p "$dir"
  for snap in "$@"; do
    n=$((n + 1))
    printf '%s\n' "$snap" >"$dir/$n.json"
  done
}

run_checks() {
  local label="$1" workflow="$2" job_id="$3"
  local verdict_job arm intel check build
  verdict_job="$(job_text "$workflow" "$job_id")"
  [ -n "$verdict_job" ] || bad "$label: the verdict job $job_id is missing"
  arm="$(env_of "$verdict_job" VERDICT_ARM64_JOB)"
  intel="$(env_of "$verdict_job" VERDICT_INTEL_JOB)"
  check="$(env_of "$verdict_job" VERDICT_CHECK)"
  build="$(env_of "$verdict_job" VERDICT_BUILD)"
  [ -n "$arm" ] && [ -n "$intel" ] && [ -n "$check" ] && [ -n "$build" ] \
    || bad "$label: the verdict step does not name its pools and labels"
  ok "$label: the verdict step names its pools ('$arm', '$intel')"

  local dir="$WORK/$label"

  # run <gate> <wanted> [attempt]: output, then exit=N and polls=N on the last lines.
  run() {
    local gate="$1" wanted="$2" attempt="${3:-1}" out got polls
    set +e
    out="$(VERDICT_SNAPSHOTS_DIR="$dir" VERDICT_VIRTUAL_CLOCK=1 VERDICT_WAIT_SECONDS=120 \
      VERDICT_POLL_SECONDS=30 GITHUB_RUN_ATTEMPT="$attempt" VERDICT_CHECK="$check" \
      VERDICT_BUILD="$build" VERDICT_ARM64_JOB="$arm" VERDICT_INTEL_JOB="$intel" \
      bash "$WAIT" "$gate" "$wanted" 2>&1)"
    got=$?
    set -e
    polls="$(grep -c '^poll ' <<<"$out" || true)"
    printf '%s\nexit=%s\npolls=%s\n' "$out" "$got" "$polls"
  }
  says() {
    local name="$1" want_exit="$2" want_polls="$3" expected="$4" out
    shift 4
    out="$(run "$@")"
    has "$out" "exit=$want_exit" || bad "$label: $name (wanted exit $want_exit; got: $out)"
    [ "$want_polls" = "-" ] || has "$out" "polls=$want_polls" || bad "$label: $name (wanted $want_polls polls; got: $out)"
    [ -z "$expected" ] || has "$out" "$expected" || bad "$label: $name (wanted '$expected'; got: $out)"
    ok "$label: $name"
  }

  local arm_ok arm_run arm_queued arm_fail arm_cancel arm_skip intel_ok intel_run intel_queued intel_fail intel_cancel intel_skip
  arm_ok="$(job "$arm" completed success)"
  arm_run="$(job "$arm" in_progress null)"
  arm_queued="$(job "$arm" queued null)"
  arm_fail="$(job "$arm" completed failure)"
  arm_cancel="$(job "$arm" completed cancelled)"
  arm_skip="$(job "$arm" completed skipped)"
  intel_ok="$(job "$intel" completed success)"
  intel_run="$(job "$intel" in_progress null)"
  intel_queued="$(job "$intel" queued null)"
  intel_fail="$(job "$intel" completed failure)"
  intel_cancel="$(job "$intel" completed cancelled)"
  intel_skip="$(job "$intel" completed skipped)"

  # --- both built: no wait --------------------------------------------------------------
  sequence "$dir" "$(snapshot "$arm_ok" "$intel_ok")"
  says "both pools built: pass on the first poll" 0 1 "succeeded on: arm64 intel" success true

  # --- one built, the other unfinished at the deadline: pass with a notice ---------------
  sequence "$dir" "$(snapshot "$arm_ok" "$intel_run")"
  says "one pool built and the other is still running at the deadline: pass" 0 - "succeeded on: arm64" success true
  says "  ...and the unfinished pool is named in a notice" 0 - "::notice::${build} (intel) was still unfinished" success true
  says "  ...and the wait reached the deadline (five polls: 0, 30, 60, 90 and 120 seconds)" 0 5 "the wait reached its 120-second deadline" success true
  sequence "$dir" "$(snapshot "$arm_queued" "$intel_ok")"
  says "a pool still queued at the deadline is unfinished too" 0 - "::notice::${build} (arm64) was still unfinished" success true

  # --- a failure before the deadline fails the check, without waiting --------------------
  sequence "$dir" "$(snapshot "$arm_run" "$intel_fail")"
  says "a failure on the Intel pool fails the check at once" 1 1 "the ${build} failed on the Intel pool" success true
  sequence "$dir" "$(snapshot "$arm_fail" "$intel_run")"
  says "a failure on the arm64 pool fails the check at once" 1 1 "the ${build} failed on the arm64 pool" success true

  # --- a success never ends the wait by itself --------------------------------------------
  sequence "$dir" \
    "$(snapshot "$arm_ok" "$intel_run")" \
    "$(snapshot "$arm_ok" "$intel_run")" \
    "$(snapshot "$arm_ok" "$intel_fail")"
  says "a pool's success does not end the wait: the other pool's later failure still fails the check" 1 3 "the ${build} failed on the Intel pool" success true
  sequence "$dir" \
    "$(snapshot "$arm_queued" "$intel_queued")" \
    "$(snapshot "$arm_run" "$intel_queued")" \
    "$(snapshot "$arm_ok" "$intel_run")" \
    "$(snapshot "$arm_ok" "$intel_ok")"
  says "the wait ends when both pools have finished, not before" 0 4 "succeeded on: arm64 intel" success true

  # --- a failure is a failure, whatever else is true ---------------------------------------
  sequence "$dir" "$(snapshot "$arm_ok" "$(job "$intel" completed timed_out)")"
  says "a timed-out pool is a failure" 1 1 "failed on the Intel pool" success true

  # --- nothing built by the deadline ---------------------------------------------------------
  sequence "$dir" "$(snapshot "$arm_run" "$intel_queued")"
  says "nothing built by the deadline: fail, and say the wait ended" 1 5 "neither pool had built it when the wait ended" success true
  sequence "$dir" "$(snapshot "$arm_cancel" "$intel_cancel")"
  says "both pools cancelled: fail" 1 1 "neither pool built it" success true
  sequence "$dir" "$(snapshot "$arm_cancel" "$intel_ok")"
  says "one pool cancelled beside a success: pass" 0 1 "succeeded on: intel" success true

  # --- the jobs API cannot be trusted to say yes -----------------------------------------------
  sequence "$dir" "UNREADABLE"
  says "an unreadable jobs API is no success: fail at the deadline" 1 5 "neither pool is known to have built it" success true
  sequence "$dir" "not json at all"
  says "an unparseable response is no success: fail" 1 5 "neither pool is known to have built it" success true
  sequence "$dir" "$(snapshot "$arm_ok")"
  says "a missing Intel job is no success for it, and the arm64 success still passes with a notice" 0 5 "::notice::${build} (intel) has an unknown state" success true
  sequence "$dir" "$(snapshot "$arm_ok" "$arm_ok" "$intel_ok")"
  says "a job that appears twice is unknown, not a success" 0 - "::notice::${build} (arm64) has an unknown state" success true
  sequence "$dir" "$(snapshot "$arm_queued" "$(job "$arm (decoy)" completed success)" "$intel_ok")"
  says "a job whose name only starts like the pool's is not the pool (exact names)" 0 - "::notice::${build} (arm64) was still unfinished" success true
  sequence "$dir" "$(snapshot "$(job "$arm" completed success 1)" "$(job "$intel" completed success 1)")"
  says "jobs of another run attempt are not this attempt's: fail" 1 - "neither pool is known to have built it" success true 2
  sequence "$dir" "$(snapshot "$(job "$arm" completed neutral)" "$(job "$intel" completed action_required)")"
  says "an unrecognized conclusion is no success: fail" 1 - "neither pool is known to have built it" success true
  sequence "$dir" "UNREADABLE" "$(snapshot "$arm_ok" "$intel_ok")"
  says "an API failure that clears is recovered from on the next poll" 0 2 "succeeded on: arm64 intel" success true
  sequence "$dir" "$(snapshot "$arm_ok" "$intel_ok")" "UNREADABLE"
  says "the wait ends at the first poll when both pools finished, so a later API failure is never seen" 0 1 "" success true

  # --- not wanted: skipped pools are finished, so there is no wait ------------------------------
  sequence "$dir" "$(snapshot "$arm_skip" "$intel_skip")"
  says "not wanted and both pools skipped: pass on the first poll, no wait" 0 1 "was not needed" success false
  sequence "$dir" "$(snapshot "$arm_ok" "$intel_skip")"
  says "not wanted but a pool ran: fail" 1 - "was not needed but its results are" success false
  sequence "$dir" "UNREADABLE"
  says "not wanted with an unreadable API: fail closed, never pass on a guess" 1 5 "expected both 'skipped'" success false

  # --- the gate -----------------------------------------------------------------------------------
  sequence "$dir" "$(snapshot "$arm_ok" "$intel_ok")"
  says "a failed gate fails the check without polling" 1 0 "the gate job did not succeed" failure true
  says "a cancelled gate fails the check without polling" 1 0 "the gate job did not succeed" cancelled true
  says "a gate that reports neither true nor false fails without polling" 1 0 "unrecognized value" success ""
  sequence "$dir" "$(snapshot "$arm_ok" "$intel_ok")"
  says "the labels name this check" 1 0 "${check} failed closed" failure true

  # --- the wait settings are checked --------------------------------------------------------------
  local out
  set +e
  out="$(VERDICT_WAIT_SECONDS=soon bash "$WAIT" success true 2>&1)"
  set -e
  has "$out" "not whole numbers" || bad "$label: a non-numeric deadline must fail closed (got: $out)"
  ok "$label: a non-numeric wait setting fails closed"
}

run_checks "ios" "$IOS" ios-check
run_checks "desktop" "$CI" desktop-nondesktop-check

CASES=$((CASES + 1))
bash -n "$WAIT"
ok "the wait script passes bash -n"

echo "PASS — $CASES wait-script assertions, 0 failures"
