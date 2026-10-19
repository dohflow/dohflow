#!/usr/bin/env bash
#
# The verdict of the iOS cross-compile check (personal-cfo-md8h2.7).
#
# `iOS cross-compile (kernel crates)` is a small Linux job whose only work is to run this
# script, so that the check fails CLOSED. GitHub reports a skipped job as Success, which
# would let a failed `gate` job (a paths-filter or API error) skip the macOS build and
# still turn a required check green. This script passes only when
#
#   * the gate job succeeded, and
#   * either the build was wanted (code that the build depends on changed) and it
#     succeeded, or it was not wanted and was therefore skipped.
#
# Any other combination (a failed, cancelled or unexpectedly run build, a failed or
# skipped gate, a gate that reported neither true nor false) fails with a message.
#
# Usage: ios-kernel-ci-verdict.sh <gate result> <build wanted: true|false|""> <build result>
# The results are the values of the `needs.<job>.result` expression: success, failure,
# cancelled or skipped.

set -euo pipefail

gate_result="${1:?gate result}"
build_wanted="${2-}"
build_result="${3:?build result}"

fail() {
  echo "::error::iOS cross-compile check failed closed: $1" >&2
  exit 1
}

[ "$gate_result" = "success" ] \
  || fail "the gate job did not succeed (result: $gate_result), so it is unknown whether the build was needed"

case "$build_wanted" in
  true | false) ;;
  *) fail "the gate reported an unrecognized value for whether the build is needed ('$build_wanted')" ;;
esac

if [ "$build_wanted" = "true" ]; then
  [ "$build_result" = "success" ] \
    || fail "the iOS build was needed but its result is '$build_result'"
  echo "iOS build was needed and succeeded."
else
  [ "$build_result" = "skipped" ] \
    || fail "the iOS build was not needed but its result is '$build_result' (expected 'skipped')"
  echo "iOS build was not needed (no change it depends on) and was skipped."
fi
