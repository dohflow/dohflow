#!/usr/bin/env bash
#
# Run the kernel's vault tests on an iOS simulator (personal-cfo-md8h2.7).
#
# Builds the selected test binaries for aarch64-apple-ios-sim, boots a throwaway
# simulator, runs each binary inside it with `xcrun simctl spawn`, and exits non-zero if
# any binary fails or none ran. CI runs this nightly and on demand
# (.github/workflows/ios-kernel.yml); it runs the same way locally on a Mac with Xcode
# and an iOS simulator runtime installed.
#
# The selection is the kernel property worth guarding on a real simulator:
#   - vault-crypto library tests: the key-derivation and AEAD primitives;
#   - finance-kernel vault_lifecycle / vault_state_machine / restore_drill /
#     cross_version_vault: create, unlock, wrong password, lock, backup restore, and a
#     frozen encrypted vault from earlier versions opening on iOS;
#   - db-worker forecast_debt_views: the Future Cash read surface over a real vault.
# Tests that re-run their own executable as a child process are not in the list.
#
# Environment:
#   IPHONEOS_DEPLOYMENT_TARGET  required; set it explicitly, see the workflow header
#   IOS_SIM_UDID                optional; reuse this simulator instead of creating one
#                               (it is left booted and is not deleted)
#   IOS_SIM_TARGET_DIR          optional cargo target dir for the simulator build

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
cd "$REPO_ROOT"

: "${IPHONEOS_DEPLOYMENT_TARGET:?IPHONEOS_DEPLOYMENT_TARGET must be set explicitly (unset, the C layer and Rust disagree about the minimum iOS)}"
TARGET="aarch64-apple-ios-sim"
BINS_FILE="$(mktemp)"
CREATED_SIM=""

cleanup() {
  rm -f "$BINS_FILE"
  if [ -n "$CREATED_SIM" ]; then
    xcrun simctl shutdown "$CREATED_SIM" >/dev/null 2>&1 || true
    xcrun simctl delete "$CREATED_SIM" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

target_dir_args=()
if [ -n "${IOS_SIM_TARGET_DIR:-}" ]; then
  target_dir_args=(--target-dir "$IOS_SIM_TARGET_DIR")
fi

# Build one cargo invocation's test binaries and append their executables to $BINS_FILE.
# Compiler diagnostics are rendered to stderr; only artifact messages reach the pipe.
build_tests() {
  echo "::group::build test binaries: cargo test $*"
  # (bash 3.2 on macOS errors on an empty array under `set -u`, hence the + form.)
  cargo test "$@" --no-run --locked --target "$TARGET" ${target_dir_args[@]+"${target_dir_args[@]}"} \
    --message-format=json-render-diagnostics \
    | jq -r 'select(.reason == "compiler-artifact" and .profile.test == true and .executable != null) | .executable' \
    >> "$BINS_FILE"
  echo "::endgroup::"
}

build_tests -p vault-crypto --lib
build_tests -p finance-kernel \
  --test vault_lifecycle --test vault_state_machine --test restore_drill --test cross_version_vault
build_tests -p db-worker --test forecast_debt_views

sort -u "$BINS_FILE" -o "$BINS_FILE"
count=$(wc -l < "$BINS_FILE" | tr -d ' ')
if [ "$count" -ne 6 ]; then
  echo "::error::expected 6 test binaries (1 + 4 + 1), built $count:" >&2
  cat "$BINS_FILE" >&2
  exit 1
fi

# A simulator to run in: the one given, or a throwaway of the newest iPhone type on the
# newest available iOS runtime.
if [ -n "${IOS_SIM_UDID:-}" ]; then
  UDID="$IOS_SIM_UDID"
else
  # The newest available iOS runtime, then the newest iPhone that runtime supports (the
  # iPhone product family also lists iPod touch models, which newer runtimes reject).
  runtime_json=$(xcrun simctl list runtimes -j \
    | jq -c '[.runtimes[] | select(.isAvailable == true and (.identifier | contains("SimRuntime.iOS-")))]
             | sort_by(.version | split(".") | map(tonumber)) | last // empty')
  runtime=$(printf '%s' "$runtime_json" | jq -r '.identifier // empty')
  devtype=$(printf '%s' "$runtime_json" \
    | jq -r '[.supportedDeviceTypes[]? | select(.productFamily == "iPhone" and (.name | startswith("iPhone")))]
             | last | .identifier // empty')
  if [ -z "$runtime" ] || [ -z "$devtype" ]; then
    echo "::error::no available iOS simulator runtime or iPhone device type" >&2
    xcrun simctl list runtimes >&2
    exit 1
  fi
  echo "simulator runtime: $runtime"
  echo "simulator device:  $devtype"
  UDID=$(xcrun simctl create "dohflow-kernel-ci" "$devtype" "$runtime")
  CREATED_SIM="$UDID"
fi
xcrun simctl boot "$UDID" 2>/dev/null || true
xcrun simctl bootstatus "$UDID" -b >/dev/null
echo "simulator $UDID booted"

failed=0
passed_total=0
while IFS= read -r bin; do
  name="$(basename "$bin")"
  echo "::group::simulator run: $name"
  log="$(mktemp)"
  if xcrun simctl spawn "$UDID" "$bin" 2>&1 | tee "$log"; then
    status=ok
  else
    status=FAILED
    failed=$((failed + 1))
  fi
  passed=$(grep -E '^test result:' "$log" | sed -E 's/.* ([0-9]+) passed.*/\1/' | head -1 || true)
  rm -f "$log"
  echo "::endgroup::"
  echo "$name: $status (${passed:-0} passed)"
  passed_total=$((passed_total + ${passed:-0}))
done < "$BINS_FILE"

echo "----"
echo "$count test binaries, $passed_total tests passed, $failed binaries failed"
if [ "$failed" -ne 0 ] || [ "$passed_total" -eq 0 ]; then
  exit 1
fi
