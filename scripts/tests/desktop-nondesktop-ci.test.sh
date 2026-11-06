#!/usr/bin/env bash
#
# Contract regression test for the desktop crate's non-desktop target check in ci.yml
# (personal-cfo-md8h2.25). The check is required, and a required check that needs a macOS
# runner dropped queued pull requests during a runner shortage, so it has the same shape as
# the iOS check (ios-kernel.yml, personal-cfo-md8h2.24): a Linux gate decides whether the
# build is wanted, the build runs on two pools, and a Linux verdict job owns the required
# name and fails closed through scripts/macos-pools-verdict.sh. These assertions pin that shape,
# run the shared verdict with this check's names, and audit that no other required check
# runs on a macOS pool. CI-safe: no network, no macOS needed.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
CI="$REPO_ROOT/.github/workflows/ci.yml"
IOS="$REPO_ROOT/.github/workflows/ios-kernel.yml"
REVIEW="$REPO_ROOT/.github/workflows/review-gate.yml"
VERDICT="$REPO_ROOT/scripts/macos-pools-verdict.sh"
WAIT="$REPO_ROOT/scripts/macos-pools-wait.sh"

for file in "$CI" "$IOS" "$REVIEW" "$VERDICT" "$WAIT"; do
  [ -f "$file" ] || { echo "FAIL — missing file: $file" >&2; exit 1; }
done

CASES=0
ok() { CASES=$((CASES + 1)); echo "    ok   — $1"; }
bad() { echo "    FAIL — $1" >&2; exit 1; }

# job_text <file> <job id>: from its key to the next top-level job.
job_text() {
  awk -v key="  $2:" '$0 == key { on = 1; print; next } on && /^  [a-z0-9-]+:$/ { exit } on { print }' "$1"
}
has() { grep -Fq -- "$2" <<<"$1"; }
require_in() {
  local label="$1" text="$2" needle="$3"
  if has "$text" "$needle"; then ok "$label"; else bad "$label (missing: $needle)"; fi
}
reject_in() {
  local label="$1" text="$2" needle="$3"
  if has "$text" "$needle"; then bad "$label (found: $needle)"; else ok "$label"; fi
}

GATE="$(job_text "$CI" desktop-nondesktop-gate)"
ARM="$(job_text "$CI" desktop-nondesktop-build-arm64)"
INTEL="$(job_text "$CI" desktop-nondesktop-build-intel)"
CHECK="$(job_text "$CI" desktop-nondesktop-check)"
for pair in "gate:$GATE" "arm64 build:$ARM" "Intel build:$INTEL" "verdict:$CHECK"; do
  [ -n "${pair#*:}" ] || bad "the ${pair%%:*} job is missing from ci.yml"
done
ok "the gate, both pool builds and the verdict job exist"

# --- the required name belongs to the Linux verdict job, and only to it ---------------
REQUIRED='Desktop crate (non-desktop target check)'
CASES=$((CASES + 1))
if [ "$(grep -cF "name: $REQUIRED" "$CI")" -eq 1 ]; then
  ok "the required name appears once in ci.yml"
else
  bad "the required name must name exactly one job"
fi
require_in "the verdict job carries the required name" "$CHECK" "name: $REQUIRED"
require_in "the verdict job runs on Linux" "$CHECK" "runs-on: ubuntu-24.04"
require_in "the gate runs on Linux" "$GATE" "runs-on: ubuntu-24.04"
for pair in "gate:$GATE" "arm64 build:$ARM" "Intel build:$INTEL"; do
  reject_in "the ${pair%%:*} job does not carry the required name" "${pair#*:}" "$REQUIRED"
done

# --- two macOS pools, separate jobs, decided by the gate ------------------------------
require_in "the arm64 pool is an Apple Silicon runner" "$ARM" "runs-on: macos-15"
reject_in "the arm64 pool is not the Intel label" "$ARM" "macos-15-intel"
require_in "the Intel pool is an Intel runner" "$INTEL" "runs-on: macos-15-intel"
for pair in "arm64:$ARM" "Intel:$INTEL"; do
  pool="${pair%%:*}"
  text="${pair#*:}"
  require_in "the $pool build needs the gate" "$text" "needs: desktop-nondesktop-gate"
  require_in "the $pool build is skipped by the gate's output, at job level" "$text" "if: needs.desktop-nondesktop-gate.outputs.check == 'true'"
  reject_in "the $pool build has no path filter of its own (a filtered step still reports Success)" "$text" "paths-filter"
  reject_in "the $pool build has no step-level skip" "$text" "steps.filter"
  reject_in "the $pool build does not ignore its own failure" "$text" "continue-on-error"
  require_in "the $pool build pins the deployment target" "$text" "IPHONEOS_DEPLOYMENT_TARGET: '17.0'"
  require_in "the $pool build adds the non-desktop target" "$text" "rustup target add aarch64-apple-ios-sim"
  require_in "the $pool build checks the desktop crate against the locked graph" "$text" "cargo check --locked --target aarch64-apple-ios-sim --manifest-path apps/desktop/src-tauri/Cargo.toml"
done
require_in "the pools do not share a cache" "$ARM" "key: nondesktop-check-arm64"
require_in "the Intel pool has its own cache" "$INTEL" "key: nondesktop-check-intel"
reject_in "the pools are separate jobs, not a matrix" "$ARM$INTEL" "matrix"

# --- the gate decides, and the decision is the gate's alone ---------------------------
require_in "the gate lists the desktop crate's paths" "$GATE" "- 'apps/desktop/src-tauri/**'"
require_in "the gate lists the kernel crates" "$GATE" "- 'crates/**'"
require_in "the gate lists the toolchain pin" "$GATE" "- 'rust-toolchain.toml'"
require_in "the gate lists this workflow" "$GATE" "- '.github/workflows/ci.yml'"
require_in "the gate lists the verdict script" "$GATE" "- 'scripts/macos-pools-verdict.sh'"
require_in "the gate lists the wait script" "$GATE" "- 'scripts/macos-pools-wait.sh'"
require_in "a manual dispatch always builds" "$GATE" "workflow_dispatch)"
require_in "a private repository builds nothing on a pull request" "$GATE" 'if [ "$PRIVATE" = "false" ] && [ "$CODE_CHANGED" = "true" ]; then'
require_in "push and the merge queue build on a public repository" "$GATE" "push | merge_group)"
require_in "the gate skips scheduled runs (the weekly run is the security scan only)" "$GATE" "if: github.event_name != 'schedule'"

# --- the verdict fails closed, through the shared script ------------------------------
# personal-cfo-md8h2.26: the verdict job needs only the gate and polls the run's jobs for a
# bounded time (scripts/macos-pools-wait.sh); a verdict that waits for both pools is as slow as
# the slower one, and a pool stuck in setup is not bounded by its timeout-minutes.
require_in "the verdict job needs only the gate" "$CHECK" "needs: desktop-nondesktop-gate"
reject_in "the verdict job does not wait for a pool job" "$CHECK" "desktop-nondesktop-build"
require_in "the verdict job runs whether or not the gate succeeded" "$CHECK" "if: \${{ always() && github.event_name != 'schedule' }}"
require_in "the verdict job has a timeout above the wait" "$CHECK" "timeout-minutes: 25"
require_in "the verdict job passes the gate result" "$CHECK" 'GATE_RESULT: ${{ needs.desktop-nondesktop-gate.result }}'
require_in "the verdict job passes whether the build was wanted" "$CHECK" 'BUILD_WANTED: ${{ needs.desktop-nondesktop-gate.outputs.check }}'
reject_in "the verdict job no longer takes the pools' results" "$CHECK" "ARM64_RESULT"
require_in "the verdict job runs the wait script, which runs the shared verdict script" "$CHECK" 'run: scripts/macos-pools-wait.sh "$GATE_RESULT" "$BUILD_WANTED"'
require_in "the verdict job may read the run's jobs, nothing more" "$CHECK" "actions: read"
require_in "the verdict job may read the repository" "$CHECK" "contents: read"
reject_in "the verdict job writes nothing" "$CHECK" "write"
require_in "the verdict job hands the token to the jobs lookup" "$CHECK" 'GH_TOKEN: ${{ github.token }}'
reject_in "the verdict logic is not written a second time in ci.yml" "$CHECK" 'fail()'
# The verdict names the pools it will look up; they must be the pools' real names.
for pair in "VERDICT_ARM64_JOB:$ARM" "VERDICT_INTEL_JOB:$INTEL"; do
  var="${pair%%:*}"
  pool_name="$(sed -n 's/^    name: //p' <<<"${pair#*:}" | head -n 1)"
  require_in "the verdict's $var is the pool job's name" "$CHECK" "$var: $pool_name"
done

# --- main-red-alert and main-red-resolve (personal-cfo-md8h2.26 condition 4) --------------
# The verdict no longer waits for the pools, so a pool that fails after the verdict passed is
# seen only through its own result. main-red-alert therefore waits for the verdict AND both
# pool jobs; main-red-resolve waits for the verdict only, so a pool that never got a runner
# (cancelled) cannot keep a green main from closing the alert.
needs_of() { job_text "$CI" "$1" | sed -n 's/^    needs: \[\(.*\)\]$/\1/p' | head -n 1; }
ALERT_NEEDS="$(needs_of main-red-alert)"
RESOLVE_NEEDS="$(needs_of main-red-resolve)"
for id in desktop-nondesktop-check desktop-nondesktop-build-arm64 desktop-nondesktop-build-intel; do
  CASES=$((CASES + 1))
  if [[ ", $ALERT_NEEDS, " == *", $id, "* ]]; then
    ok "main-red-alert waits for $id"
  else
    bad "main-red-alert must wait for $id (needs: $ALERT_NEEDS)"
  fi
done
CASES=$((CASES + 1))
if [[ ", $RESOLVE_NEEDS, " == *", desktop-nondesktop-check, "* ]]; then
  ok "main-red-resolve waits for the verdict job"
else
  bad "main-red-resolve must wait for desktop-nondesktop-check (needs: $RESOLVE_NEEDS)"
fi
reject_in "main-red-resolve does not wait for a pool job (a cancelled pool beside a built one is not red)" "$RESOLVE_NEEDS" "desktop-nondesktop-build"
ALERT_JOB="$(job_text "$CI" main-red-alert)"
require_in "main-red-alert fires on any failed needed job" "$ALERT_JOB" "contains(needs.*.result, 'failure')"
# The alert's own failed-job list, run on a late pool failure: the verdict passed, then a pool
# failed. The jq filter is read out of ci.yml, so the test runs what the job runs.
FAILED_JQ="$(sed -n "s/^          failed=\$(printf '%s' \"\$NEEDS\" | jq -r '\(.*\)')$/\1/p" <<<"$ALERT_JOB" | head -n 1)"
[ -n "$FAILED_JQ" ] || bad "could not read the alert's failed-job filter out of ci.yml"
LATE_FAILURE='{"rust":{"result":"success"},"desktop-nondesktop-check":{"result":"success"},"desktop-nondesktop-build-arm64":{"result":"success"},"desktop-nondesktop-build-intel":{"result":"failure"}}'
CASES=$((CASES + 1))
listed="$(printf '%s' "$LATE_FAILURE" | jq -r "$FAILED_JQ")"
if [ "$listed" = "- desktop-nondesktop-build-intel" ]; then
  ok "a pool failing after the verdict passed is the one job the alert lists"
else
  bad "the alert must list the late pool failure (got: $listed)"
fi
CASES=$((CASES + 1))
if [ "$(printf '%s' "$LATE_FAILURE" | jq 'any(.[]; .result == "failure")')" = "true" ]; then
  ok "the alert's condition (any needed job failed) holds for that run"
else
  bad "the alert condition must hold for a late pool failure"
fi
CASES=$((CASES + 1))
NOT_RED='{"rust":{"result":"success"},"desktop-nondesktop-check":{"result":"success"},"desktop-nondesktop-build-arm64":{"result":"cancelled"},"desktop-nondesktop-build-intel":{"result":"success"}}'
if [ "$(printf '%s' "$NOT_RED" | jq 'any(.[]; .result == "failure")')" = "false" ]; then
  ok "a pool that never got a runner (cancelled) beside a built one does not raise the alert"
else
  bad "a cancelled pool must not raise the alert"
fi

# --- the stale comment is gone --------------------------------------------------------
CASES=$((CASES + 1))
if grep -Fq "Not a required check yet" "$CI"; then
  bad "ci.yml still says the desktop check is not required"
fi
ok "ci.yml no longer calls the desktop check not required"

# --- actions are pinned, as everywhere else -------------------------------------------
CASES=$((CASES + 1))
if grep -nE '^\s*(- )?uses: ' <<<"$GATE$ARM$INTEL$CHECK" | grep -vE 'uses: [^ ]+@[0-9a-f]{40} # v' >/dev/null; then
  bad "every action in these jobs must be pinned to a full commit SHA with a version comment"
fi
ok "every action in these jobs is pinned to a full commit SHA"

# --- the shared verdict, with this check's names --------------------------------------
JOBS_DIR="$(mktemp -d)"
trap 'rm -rf "$JOBS_DIR"' EXIT
cat >"$JOBS_DIR/arm64-never-acquired.json" <<'JSON'
{"total_count": 2, "jobs": [
  {"name": "Desktop crate non-desktop build (macOS arm64)", "status": "completed", "conclusion": "cancelled", "runner_id": null, "runner_name": null, "steps": []},
  {"name": "Desktop crate non-desktop build (macOS intel)", "status": "completed", "conclusion": "success", "runner_id": 12, "runner_name": "GitHub Actions 12", "steps": [{"name": "Set up job"}]}
]}
JSON
cat >"$JOBS_DIR/both-never-acquired.json" <<'JSON'
{"total_count": 2, "jobs": [
  {"name": "Desktop crate non-desktop build (macOS arm64)", "status": "completed", "conclusion": "cancelled", "runner_id": null, "runner_name": null, "steps": []},
  {"name": "Desktop crate non-desktop build (macOS intel)", "status": "completed", "conclusion": "cancelled", "runner_id": null, "runner_name": null, "steps": []}
]}
JSON

# run_verdict <jobs file> <args...>: stdout and stderr, then the exit status on a last line.
run_verdict() {
  local jobs_file="$1"
  shift
  local out got
  set +e
  out="$(VERDICT_CHECK="$(env_of VERDICT_CHECK)" VERDICT_BUILD="$(env_of VERDICT_BUILD)" \
    VERDICT_ARM64_JOB="$(env_of VERDICT_ARM64_JOB)" VERDICT_INTEL_JOB="$(env_of VERDICT_INTEL_JOB)" \
    VERDICT_JOBS_JSON="$jobs_file" bash "$VERDICT" "$@" 2>&1)"
  got=$?
  set -e
  printf '%s\nexit=%s\n' "$out" "$got"
}
# The names are read from ci.yml, so the test cannot drift from what the job passes.
env_of() { sed -n "s/^          $1: //p" <<<"$CHECK" | head -n 1; }

says() {
  local label="$1" jobs_file="$2" want_exit="$3" expected="$4"
  shift 4
  local out
  out="$(run_verdict "$jobs_file" "$@")"
  has "$out" "exit=$want_exit" || bad "$label (wanted exit $want_exit; got: $out)"
  has "$out" "$expected" || bad "$label (wanted: $expected; got: $out)"
  ok "$label"
}
refuses_to_mention() {
  local label="$1" jobs_file="$2" needle="$3"
  shift 3
  local out
  out="$(run_verdict "$jobs_file" "$@")"
  if has "$out" "$needle"; then bad "$label (found '$needle' in: $out)"; fi
  ok "$label"
}

says "built on both pools passes and says so" "$JOBS_DIR/none.json" 0 "Desktop crate build was needed and succeeded on: arm64 intel." success true success success
says "a pool that never got a runner is named, with the other pool built" "$JOBS_DIR/arm64-never-acquired.json" 0 "Desktop crate build (arm64) was never acquired" success true cancelled success
says "the pool that built is named" "$JOBS_DIR/arm64-never-acquired.json" 0 "succeeded on: intel" success true cancelled success
says "both pools never acquired: arm64 is named" "$JOBS_DIR/both-never-acquired.json" 1 "Desktop crate build (arm64) was never acquired" success true cancelled cancelled
says "both pools never acquired: Intel is named" "$JOBS_DIR/both-never-acquired.json" 1 "Desktop crate build (intel) was never acquired" success true cancelled cancelled
says "both pools never acquired: the failure names the check" "$JOBS_DIR/both-never-acquired.json" 1 "Desktop crate non-desktop target check failed closed" success true cancelled cancelled
says "a failure on one pool fails the check" "$JOBS_DIR/none.json" 1 "the Desktop crate build failed on the Intel pool" success true success failure
says "a failure on the arm64 pool fails the check" "$JOBS_DIR/none.json" 1 "the Desktop crate build failed on the arm64 pool" success true failure success
says "a failed gate fails the check" "$JOBS_DIR/none.json" 1 "the gate job did not succeed" failure true skipped skipped
says "a skipped build the gate did not want passes" "$JOBS_DIR/none.json" 0 "was not needed" success false skipped skipped
says "a build that ran when it was not wanted fails" "$JOBS_DIR/none.json" 1 "was not needed but its results are" success false success skipped
says "one pool built and the other unfinished passes, with a notice naming it" "$JOBS_DIR/none.json" 0 "::notice::Desktop crate build (intel) was still unfinished" success true success unfinished
says "a pool whose state is unknown fails the check even beside a success (a renamed job)" "$JOBS_DIR/none.json" 1 "arm64 pool's state of the Desktop crate build could not be determined" success true unknown success
says "an unknown Intel pool beside a success fails the same way" "$JOBS_DIR/none.json" 1 "Intel pool's state of the Desktop crate build could not be determined" success true success unknown
says "nothing built by the deadline fails and says so" "$JOBS_DIR/none.json" 1 "neither pool had built it when the wait ended" success true unfinished unfinished
says "both pools unknown fails" "$JOBS_DIR/none.json" 1 "could not be determined" success true unknown unknown
says "an unfinished pool never satisfies 'not wanted'" "$JOBS_DIR/none.json" 1 "expected both 'skipped'" success false unfinished skipped
refuses_to_mention "the iOS names do not leak into this check's messages" "$JOBS_DIR/both-never-acquired.json" "iOS" success true cancelled cancelled

# --- audit: every required check, and where it runs (personal-cfo-md8h2.25 AC6) --------
# The branch ruleset's required checks, as listed on 2026-11-04. A new required check is a
# repository-settings step, so a name added there is added here by the same person. Only the
# two verdict jobs may depend on macOS, and they do so only through pool jobs.
declare -a REQUIRED_CHECKS=(
  "IPC codegen (apps/desktop/src-tauri)|$CI"
  "Rust (workspace)|$CI"
  "Security scanning (secrets + dependencies)|$CI"
  "Shell scripts|$CI"
  "Frontend (apps/desktop)|$CI"
  "iOS cross-compile (kernel crates)|$IOS"
  "Desktop crate (non-desktop target check)|$CI"
  "Review gate|$REVIEW"
)
for entry in "${REQUIRED_CHECKS[@]}"; do
  name="${entry%%|*}"
  file="${entry#*|}"
  runner="$(awk -v n="    name: $name" '$0 == n { f = 1 } f && /runs-on:/ { print $2; exit }' "$file")"
  CASES=$((CASES + 1))
  case "$runner" in
    ubuntu-*) ok "required check '$name' runs on Linux ($runner), so a macOS shortage cannot cancel it" ;;
    "") bad "required check '$name' was not found in $(basename "$file")" ;;
    *) bad "required check '$name' runs on '$runner'; a required check on a single hosted macOS pool is dropped by a runner shortage" ;;
  esac
done
# Both macOS-dependent required checks have two pools behind them.
CASES=$((CASES + 1))
pools="$(grep -c "runs-on: macos-15$" <<<"$ARM$(job_text "$IOS" ios-build-arm64)")$(grep -c "runs-on: macos-15-intel$" <<<"$INTEL$(job_text "$IOS" ios-build-intel)")"
if [ "$pools" = "22" ]; then
  ok "each macOS-dependent required check has one arm64 and one Intel pool job"
else
  bad "expected an arm64 and an Intel pool job behind each of the two checks (got $pools)"
fi

CASES=$((CASES + 1))
bash -n "$VERDICT"
bash -n "$WAIT"
ok "the verdict and wait scripts pass bash -n"
CASES=$((CASES + 1))
if command -v ruby >/dev/null 2>&1; then
  ruby -ryaml -e 'YAML.load_file(ARGV[0])' "$CI"
  ok "ci.yml parses as YAML"
else
  echo "    skip — ruby not available, YAML parse not checked"
fi

echo "PASS — $CASES desktop non-desktop CI contract assertions, 0 failures"
