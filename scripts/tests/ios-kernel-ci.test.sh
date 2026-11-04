#!/usr/bin/env bash
#
# Contract regression test for the iOS kernel CI (personal-cfo-md8h2.7).
#
# .github/workflows/ios-kernel.yml cross-compiles the kernel crates for
# aarch64-apple-ios / aarch64-apple-ios-sim and runs the vault tests on a simulator
# (scripts/ios-kernel-sim-tests.sh). These assertions pin the properties that would
# otherwise erode silently: the explicit deployment-target pin, both targets, the locked
# workspace-wide check, SHA-pinned actions, the schedule-visibility handling, the
# job-level skip that keeps the check safe to require, and kernel-only scope (ADR 0085:
# nothing here names or builds an application). CI-safe: no network, no macOS needed.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
WORKFLOW="$REPO_ROOT/.github/workflows/ios-kernel.yml"
SCRIPT="$REPO_ROOT/scripts/ios-kernel-sim-tests.sh"
VERDICT="$REPO_ROOT/scripts/ios-kernel-ci-verdict.sh"

for file in "$WORKFLOW" "$SCRIPT" "$VERDICT"; do
  if [ ! -f "$file" ]; then
    echo "FAIL — missing file: $file" >&2
    exit 1
  fi
done

CASES=0
require_text() {
  local label="$1" file="$2" needle="$3"
  CASES=$((CASES + 1))
  if grep -Fq -- "$needle" "$file"; then
    echo "    ok   — $label"
  else
    echo "    FAIL — $label (missing: $needle)" >&2
    exit 1
  fi
}

reject_text() {
  local label="$1" file="$2" needle="$3"
  CASES=$((CASES + 1))
  if grep -Fqi -- "$needle" "$file"; then
    echo "    FAIL — $label (found: $needle)" >&2
    exit 1
  else
    echo "    ok   — $label"
  fi
}

# --- the deployment target is pinned explicitly, once, in the workflow ----------------
require_text "deployment target is pinned" "$WORKFLOW" "IPHONEOS_DEPLOYMENT_TARGET: '17.0'"
CASES=$((CASES + 1))
pins=$(grep -c "IPHONEOS_DEPLOYMENT_TARGET: " "$WORKFLOW")
if [ "$pins" -ne 1 ]; then
  echo "    FAIL — the pin must be set in exactly one place, found $pins" >&2
  exit 1
fi
echo "    ok   — the pin is set in exactly one place"
require_text "script refuses to run without the pin" "$SCRIPT" 'IPHONEOS_DEPLOYMENT_TARGET:?'

# --- the check job: both targets, the whole workspace, the locked graph --------------
require_text "the required-check name belongs to the verdict job" "$WORKFLOW" "name: iOS cross-compile (kernel crates)"
require_text "the arm64 macOS build is a separate job" "$WORKFLOW" "name: iOS cross-compile (macOS build, arm64)"
require_text "the Intel macOS build is a separate job" "$WORKFLOW" "name: iOS cross-compile (macOS build, intel)"
require_text "device target is checked" "$WORKFLOW" "cargo check --workspace --all-targets --locked --target aarch64-apple-ios"
require_text "simulator target is checked" "$WORKFLOW" "cargo check --workspace --all-targets --locked --target aarch64-apple-ios-sim"
require_text "targets are added on top of the pinned toolchain" "$WORKFLOW" "rustup target add aarch64-apple-ios aarch64-apple-ios-sim"
require_text "the arm64 pool is an Apple Silicon macOS runner" "$WORKFLOW" "runs-on: macos-15"
require_text "the Intel pool is an Intel macOS runner" "$WORKFLOW" "runs-on: macos-15-intel"
require_text "the Intel label's retirement position is recorded in the workflow" "$WORKFLOW" "publishes no"
CASES=$((CASES + 1))
for job in ios-build-arm64 ios-build-intel; do
  runs=$(awk -v job="  $job:" '$0==job{f=1;next} f&&/^  [a-z-]+:$/{f=0} f&&/cargo check --workspace --all-targets --locked --target aarch64-apple-ios( |$)/{n++} f&&/cargo check --workspace --all-targets --locked --target aarch64-apple-ios-sim/{m++} END{print n+0, m+0}' "$WORKFLOW")
  if [ "$runs" != "1 1" ]; then
    echo "    FAIL — $job must run both cargo check steps once (got: $runs)" >&2
    exit 1
  fi
done
echo "    ok   — both pools run both cargo check steps"

# --- the simulator job ----------------------------------------------------------------
require_text "simulator job has its name" "$WORKFLOW" "name: iOS simulator (vault tests)"
require_text "simulator job runs the script" "$WORKFLOW" "run: scripts/ios-kernel-sim-tests.sh"
require_text "nightly schedule" "$WORKFLOW" "cron: '30 7 * * *'"
require_text "manual dispatch" "$WORKFLOW" "workflow_dispatch:"
require_text "simulator job is gated on the gate output" "$WORKFLOW" "if: needs.gate.outputs.simulator == 'true'"
require_text "a change to the simulator machinery runs it on the pull request" "$WORKFLOW" 'if [ "$PRIVATE" = "false" ] && [ "$SIMULATOR_CHANGED" = "true" ]; then'
require_text "script targets the simulator triple" "$SCRIPT" 'TARGET="aarch64-apple-ios-sim"'
require_text "script uses the locked graph" "$SCRIPT" "--no-run --locked"
require_text "script runs vault create/unlock/wrong-password tests" "$SCRIPT" "--test vault_lifecycle"
require_text "script runs the frozen cross-version vault test" "$SCRIPT" "--test cross_version_vault"
require_text "script runs the Future Cash read tests" "$SCRIPT" "--test forecast_debt_views"
require_text "script runs the crypto library tests" "$SCRIPT" "-p vault-crypto --lib"
require_text "script runs binaries inside the simulator" "$SCRIPT" 'xcrun simctl spawn "$UDID"'
require_text "script fails when a binary fails or none ran" "$SCRIPT" 'if [ "$failed" -ne 0 ] || [ "$passed_total" -eq 0 ]; then'
require_text "script cleans up its throwaway simulator" "$SCRIPT" 'xcrun simctl delete "$CREATED_SIM"'
require_text "script survives bash 3.2 empty arrays" "$SCRIPT" 'target_dir_args[@]+'

# --- gating follows ci.yml's conventions ----------------------------------------------
require_text "the builds need the gate" "$WORKFLOW" "needs: gate"
CASES=$((CASES + 1))
if [ "$(grep -c "if: needs.gate.outputs.check == 'true'" "$WORKFLOW")" -eq 2 ]; then
  echo "    ok   — both pools are skipped by a job-level if, never a workflow paths filter"
else
  echo "    FAIL — each pool's job must be skipped by the gate's output" >&2
  exit 1
fi
CASES=$((CASES + 1))
if grep -n "matrix" "$WORKFLOW" >/dev/null; then
  echo "    FAIL — the pools must be separate jobs, not a matrix: a matrix job reports one aggregate result and the verdict needs each pool's" >&2
  exit 1
fi
echo "    ok   — the pools are separate jobs (the verdict sees each result)"
require_text "scheduled visibility is read from the API" "$WORKFLOW" 'gh api "repos/${{ github.repository }}" --jq .private'
require_text "path filter covers the lockfile" "$WORKFLOW" "- 'Cargo.lock'"
require_text "path filter covers the toolchain pin" "$WORKFLOW" "- 'rust-toolchain.toml'"
require_text "path filter covers the crates" "$WORKFLOW" "- 'crates/**'"
require_text "workflow has read-only contents permission" "$WORKFLOW" "contents: read"

CASES=$((CASES + 1))
if grep -nE '^\s*(- )?uses: ' "$WORKFLOW" | grep -vE 'uses: [^ ]+@[0-9a-f]{40} # v' >/dev/null; then
  echo "    FAIL — every action must be pinned to a full commit SHA with a version comment" >&2
  grep -nE '^\s*(- )?uses: ' "$WORKFLOW" | grep -vE 'uses: [^ ]+@[0-9a-f]{40} # v' >&2
  exit 1
fi
echo "    ok   — every action is pinned to a full commit SHA"

# --- the required check fails closed (a skipped job reports Success) ------------------
require_text "the verdict job waits for the gate and both builds" "$WORKFLOW" "needs: [gate, ios-build-arm64, ios-build-intel]"
require_text "the verdict job runs whenever the gate or a build did not succeed" "$WORKFLOW" "if: \${{ always() && github.event_name != 'schedule' }}"
require_text "the verdict job passes the gate result" "$WORKFLOW" 'GATE_RESULT: ${{ needs.gate.result }}'
require_text "the verdict job passes whether the build was wanted" "$WORKFLOW" 'BUILD_WANTED: ${{ needs.gate.outputs.check }}'
require_text "the verdict job passes the arm64 result" "$WORKFLOW" 'ARM64_RESULT: ${{ needs.ios-build-arm64.result }}'
require_text "the verdict job passes the Intel result" "$WORKFLOW" 'INTEL_RESULT: ${{ needs.ios-build-intel.result }}'
require_text "the verdict job runs the verdict script" "$WORKFLOW" 'run: scripts/ios-kernel-ci-verdict.sh "$GATE_RESULT" "$BUILD_WANTED" "$ARM64_RESULT" "$INTEL_RESULT"'
require_text "the verdict job may read the run's jobs" "$WORKFLOW" "actions: read"
require_text "the verdict job hands the token to the jobs lookup" "$WORKFLOW" 'GH_TOKEN: ${{ github.token }}'
CASES=$((CASES + 1))
ios_check_runner="$(awk '/^  ios-check:/{f=1} f&&/runs-on:/{print; exit}' "$WORKFLOW")"
if grep -qE "ubuntu-(latest|24\.04)" <<<"$ios_check_runner"; then
  echo "    ok   — the verdict job runs on Linux (a failure costs no macOS minutes)"
else
  echo "    FAIL — the verdict job must run on ubuntu-latest or a pinned ubuntu image" >&2
  exit 1
fi

# The verdict script's truth table, run for real. Arguments: gate result, whether the
# build was wanted, the arm64 build's result, the Intel build's result.
verdict_expect() {
  local want="$1" gate="$2" wanted="$3" arm="$4" intel="$5"
  CASES=$((CASES + 1))
  set +e
  bash "$VERDICT" "$gate" "$wanted" "$arm" "$intel" >/dev/null 2>&1
  local got=$?
  set -e
  if { [ "$want" = pass ] && [ "$got" -eq 0 ]; } || { [ "$want" = fail ] && [ "$got" -ne 0 ]; }; then
    echo "    ok   — verdict $want: gate=$gate wanted=${wanted:-<empty>} arm64=$arm intel=$intel"
  else
    echo "    FAIL — verdict should $want but exit was $got: gate=$gate wanted=${wanted:-<empty>} arm64=$arm intel=$intel" >&2
    exit 1
  fi
}
# The gate succeeded and the build was wanted: at least one success and no failure passes.
verdict_expect pass success true success success
verdict_expect pass success true success cancelled
verdict_expect pass success true cancelled success
verdict_expect pass success true success skipped
verdict_expect pass success true skipped success
# A failure on either pool fails, whatever the other pool did.
for other in success failure cancelled skipped; do
  verdict_expect fail success true failure "$other"
  verdict_expect fail success true "$other" failure
done
# Two builds that are not successes fail (a shortage in both pools must not pass).
for a in cancelled skipped; do
  for b in cancelled skipped; do
    verdict_expect fail success true "$a" "$b"
  done
done
# The gate succeeded and the build was not wanted: only two skipped builds pass.
verdict_expect pass success false skipped skipped
for a in success failure cancelled skipped; do
  for b in success failure cancelled skipped; do
    if [ "$a" != skipped ] || [ "$b" != skipped ]; then
      verdict_expect fail success false "$a" "$b"
    fi
  done
done
# The gate did not succeed: nothing passes, whatever else is reported. This is the case
# where a skipped build would otherwise make the required check green.
for gate in failure cancelled skipped; do
  for wanted in true false ""; do
    for a in success failure cancelled skipped; do
      for b in success failure cancelled skipped; do
        verdict_expect fail "$gate" "$wanted" "$a" "$b"
      done
    done
  done
done
# A gate that succeeded but reported neither true nor false fails closed.
verdict_expect fail success "" skipped skipped
verdict_expect fail success maybe skipped skipped
verdict_expect fail success True skipped skipped
# A missing argument fails rather than passing.
CASES=$((CASES + 1))
if bash "$VERDICT" success true success >/dev/null 2>&1; then
  echo "    FAIL — the verdict must refuse a missing Intel result" >&2
  exit 1
fi
echo "    ok   — the verdict refuses a missing result"

# The diagnostic for a cancelled build, read from a jobs API response. It must name the pool
# that built, say whether the cancelled job was never given a runner or was cancelled
# mid-run, say so when the API cannot be read, and never change the verdict.
JOBS_DIR="$(mktemp -d)"
trap 'rm -rf "$JOBS_DIR"' EXIT
cat >"$JOBS_DIR/never-acquired.json" <<'JSON'
{"total_count": 2, "jobs": [
  {"name": "iOS cross-compile (macOS build, arm64)", "status": "completed", "conclusion": "cancelled", "runner_id": null, "runner_name": null, "steps": []},
  {"name": "iOS cross-compile (macOS build, intel)", "status": "completed", "conclusion": "success", "runner_id": 12, "runner_name": "GitHub Actions 12", "steps": [{"name": "Set up job"}]}
]}
JSON
cat >"$JOBS_DIR/mid-run.json" <<'JSON'
{"total_count": 2, "jobs": [
  {"name": "iOS cross-compile (macOS build, arm64)", "status": "completed", "conclusion": "success", "runner_id": 9, "runner_name": "GitHub Actions 9", "steps": [{"name": "Set up job"}]},
  {"name": "iOS cross-compile (macOS build, intel)", "status": "completed", "conclusion": "cancelled", "runner_id": 12, "runner_name": "GitHub Actions 12", "steps": [{"name": "Set up job"}, {"name": "cargo check"}]}
]}
JSON
echo '{"total_count": 0, "jobs": []}' >"$JOBS_DIR/empty.json"

verdict_says() {
  local label="$1" jobs_file="$2" expected="$3"
  shift 3
  CASES=$((CASES + 1))
  local out
  set +e
  out="$(IOS_KERNEL_JOBS_JSON="$jobs_file" bash "$VERDICT" "$@" 2>&1)"
  local got=$?
  set -e
  if grep -Fq -- "$expected" <<<"$out"; then
    echo "    ok   — $label"
  else
    echo "    FAIL — $label (exit $got; wanted: $expected; got: $out)" >&2
    exit 1
  fi
}
verdict_says "a never-acquired arm64 build is named, and the Intel pool is said to have built" "$JOBS_DIR/never-acquired.json" "iOS build (arm64) was never acquired" success true cancelled success
verdict_says "the pool that built is named" "$JOBS_DIR/never-acquired.json" "succeeded on: intel" success true cancelled success
verdict_says "a build cancelled mid-run is said to have been mid-run" "$JOBS_DIR/mid-run.json" "iOS build (intel) was cancelled mid-run" success true success cancelled
verdict_says "a cancelled build the API does not know is reported as unknown" "$JOBS_DIR/empty.json" "the jobs API has no record" success true cancelled success
verdict_says "an unreadable jobs response is reported as unknown" "$JOBS_DIR/does-not-exist.json" "could not be read" success true cancelled success
verdict_says "both pools never acquired: both are named before the failure" "$JOBS_DIR/never-acquired.json" "iOS build (intel) was cancelled mid-run" success true cancelled cancelled
# The diagnostic never changes the verdict.
CASES=$((CASES + 1))
IOS_KERNEL_JOBS_JSON="$JOBS_DIR/never-acquired.json" bash "$VERDICT" success true cancelled success >/dev/null 2>&1
if IOS_KERNEL_JOBS_JSON="$JOBS_DIR/never-acquired.json" bash "$VERDICT" success true cancelled cancelled >/dev/null 2>&1; then
  echo "    FAIL — two cancelled builds must fail whatever the jobs API says" >&2
  exit 1
fi
echo "    ok   — the diagnostic does not change the verdict"

# --- kernel-only scope (ADR 0085) -----------------------------------------------------
for forbidden in "apps/desktop" "tauri" "swiftui" "uniffi" "xcodegen" "App Store" "TestFlight" "testflight"; do
  reject_text "workflow does not mention '$forbidden'" "$WORKFLOW" "$forbidden"
  reject_text "script does not mention '$forbidden'" "$SCRIPT" "$forbidden"
  reject_text "verdict script does not mention '$forbidden'" "$VERDICT" "$forbidden"
done

# --- syntax ---------------------------------------------------------------------------
CASES=$((CASES + 1))
bash -n "$SCRIPT"
bash -n "$VERDICT"
echo "    ok   — the simulator and verdict scripts pass bash -n"
CASES=$((CASES + 1))
if command -v ruby >/dev/null 2>&1; then
  ruby -ryaml -e 'YAML.load_file(ARGV[0])' "$WORKFLOW"
  echo "    ok   — the workflow parses as YAML"
else
  echo "    skip — ruby not available, YAML parse not checked"
fi

echo "PASS — $CASES iOS kernel CI contract assertions, 0 failures"
