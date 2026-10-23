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
require_text "the macOS build is a separate job" "$WORKFLOW" "name: iOS cross-compile (macOS build)"
require_text "device target is checked" "$WORKFLOW" "cargo check --workspace --all-targets --locked --target aarch64-apple-ios"
require_text "simulator target is checked" "$WORKFLOW" "cargo check --workspace --all-targets --locked --target aarch64-apple-ios-sim"
require_text "targets are added on top of the pinned toolchain" "$WORKFLOW" "rustup target add aarch64-apple-ios aarch64-apple-ios-sim"
require_text "runs on an Apple Silicon macOS runner" "$WORKFLOW" "runs-on: macos-15"

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
require_text "the build needs the gate" "$WORKFLOW" "needs: gate"
require_text "check is skipped by a job-level if, never a workflow paths filter" "$WORKFLOW" "if: needs.gate.outputs.check == 'true'"
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
require_text "the verdict job waits for the gate and the build" "$WORKFLOW" "needs: [gate, ios-build]"
require_text "the verdict job runs whenever the gate or build did not succeed" "$WORKFLOW" "if: \${{ always() && github.event_name != 'schedule' }}"
require_text "the verdict job passes the gate result" "$WORKFLOW" 'GATE_RESULT: ${{ needs.gate.result }}'
require_text "the verdict job passes whether the build was wanted" "$WORKFLOW" 'BUILD_WANTED: ${{ needs.gate.outputs.check }}'
require_text "the verdict job passes the build result" "$WORKFLOW" 'BUILD_RESULT: ${{ needs.ios-build.result }}'
require_text "the verdict job runs the verdict script" "$WORKFLOW" 'run: scripts/ios-kernel-ci-verdict.sh "$GATE_RESULT" "$BUILD_WANTED" "$BUILD_RESULT"'
CASES=$((CASES + 1))
ios_check_runner="$(awk '/^  ios-check:/{f=1} f&&/runs-on:/{print; exit}' "$WORKFLOW")"
if grep -q "ubuntu-latest" <<<"$ios_check_runner"; then
  echo "    ok   — the verdict job runs on Linux (a failure costs no macOS minutes)"
else
  echo "    FAIL — the verdict job must run on ubuntu-latest" >&2
  exit 1
fi

# The verdict script's truth table, run for real. Arguments: gate result, whether the
# build was wanted, build result.
verdict_expect() {
  local want="$1" gate="$2" wanted="$3" build="$4"
  CASES=$((CASES + 1))
  set +e
  bash "$VERDICT" "$gate" "$wanted" "$build" >/dev/null 2>&1
  local got=$?
  set -e
  if { [ "$want" = pass ] && [ "$got" -eq 0 ]; } || { [ "$want" = fail ] && [ "$got" -ne 0 ]; }; then
    echo "    ok   — verdict $want: gate=$gate wanted=${wanted:-<empty>} build=$build"
  else
    echo "    FAIL — verdict should $want but exit was $got: gate=$gate wanted=${wanted:-<empty>} build=$build" >&2
    exit 1
  fi
}
# The gate succeeded and the build was wanted: only a successful build passes.
verdict_expect pass success true success
verdict_expect fail success true failure
verdict_expect fail success true cancelled
verdict_expect fail success true skipped
# The gate succeeded and the build was not wanted: only a skipped build passes.
verdict_expect pass success false skipped
verdict_expect fail success false success
verdict_expect fail success false failure
verdict_expect fail success false cancelled
# The gate did not succeed: nothing passes, whatever else is reported. This is the case
# where a skipped build would otherwise make the required check green.
for gate in failure cancelled skipped; do
  for wanted in true false ""; do
    for build in success failure cancelled skipped; do
      verdict_expect fail "$gate" "$wanted" "$build"
    done
  done
done
# A gate that succeeded but reported neither true nor false fails closed.
verdict_expect fail success "" skipped
verdict_expect fail success maybe skipped
verdict_expect fail success True skipped

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
