#!/usr/bin/env bash
#
# The verdict of the iOS cross-compile check (personal-cfo-md8h2.7, md8h2.24).
#
# `iOS cross-compile (kernel crates)` is a small Linux job whose only work is to run this
# script, so that the check fails CLOSED. GitHub reports a skipped job as Success, which
# would let a failed `gate` job (a paths-filter or API error) skip the macOS build and
# still turn a required check green. The macOS build runs on two runner pools in parallel
# (arm64 and Intel), because a shortage of one pool leaves its job without a runner until
# GitHub cancels it, and a check that never ran used to drop queued pull requests. This
# script passes only when
#
#   * the gate job succeeded, and
#   * either the build was wanted (code that the build depends on changed), at least one
#     pool's build succeeded, and neither pool's build FAILED; or the build was not wanted
#     and both pools' builds were therefore skipped.
#
# So a cancelled build beside a success passes (the pool that built has proven the
# cross-compile; the cross-compile does not depend on the host), two builds that are not
# successes fail, and a failure on either pool fails. Any other combination (a failed or
# skipped gate, a gate that reported neither true nor false, a build that ran when it was
# not wanted) fails with a message.
#
# For a build that was cancelled the script also says whether GitHub never gave the job a
# runner or the job was cancelled mid-run, and which pool built. That reads the jobs API
# (the verdict job has `actions: read`) and is diagnostic only: it can never change the
# verdict, and a failed lookup only says so.
#
# Usage: ios-kernel-ci-verdict.sh <gate result> <build wanted: true|false|""> \
#                                 <arm64 build result> <intel build result>
# The results are the values of the `needs.<job>.result` expression: success, failure,
# cancelled or skipped.
#
# Diagnostic inputs (all optional): GITHUB_REPOSITORY, GITHUB_RUN_ID and GH_TOKEN for the
# jobs API; IOS_KERNEL_JOBS_JSON, a file holding the API's jobs response, which replaces
# the API call (the contract test uses it).

set -euo pipefail

gate_result="${1:?gate result}"
build_wanted="${2-}"
arm64_result="${3:?arm64 build result}"
intel_result="${4:?intel build result}"

ARM64_JOB="iOS cross-compile (macOS build, arm64)"
INTEL_JOB="iOS cross-compile (macOS build, intel)"

fail() {
  echo "::error::iOS cross-compile check failed closed: $1" >&2
  exit 1
}

# Says how a cancelled build ended. Never fails the script.
explain_cancelled() {
  local pool="$1" job_name="$2" jobs_json="" found=""
  if [ -n "${IOS_KERNEL_JOBS_JSON-}" ]; then
    jobs_json="$(cat "$IOS_KERNEL_JOBS_JSON" 2>/dev/null)" || jobs_json=""
  elif [ -n "${GITHUB_REPOSITORY-}" ] && [ -n "${GITHUB_RUN_ID-}" ] && command -v gh >/dev/null 2>&1; then
    jobs_json="$(gh api "repos/${GITHUB_REPOSITORY}/actions/runs/${GITHUB_RUN_ID}/jobs?per_page=100" 2>/dev/null)" || jobs_json=""
  fi
  if [ -z "$jobs_json" ] || ! command -v jq >/dev/null 2>&1; then
    echo "iOS build ($pool) was cancelled; the jobs API could not be read, so it is unknown whether a runner was ever assigned."
    return 0
  fi
  # A job GitHub never found a runner for has no runner id and no steps.
  found="$(jq -r --arg name "$job_name" '
      [.jobs[]? | select(.name == $name)][0] as $job
      | if $job == null then "missing"
        elif $job.runner_id == null and (($job.steps // []) | length) == 0 then "never-acquired"
        else "mid-run" end' <<<"$jobs_json" 2>/dev/null)" || found=""
  case "$found" in
    never-acquired)
      echo "iOS build ($pool) was never acquired: no runner was assigned to it before GitHub cancelled the job (a runner shortage)." ;;
    mid-run)
      echo "iOS build ($pool) was cancelled mid-run: a runner was assigned and the job started." ;;
    *)
      echo "iOS build ($pool) was cancelled; the jobs API has no record of the job, so it is unknown whether a runner was ever assigned." ;;
  esac
}

[ "$gate_result" = "success" ] \
  || fail "the gate job did not succeed (result: $gate_result), so it is unknown whether the build was needed"

case "$build_wanted" in
  true | false) ;;
  *) fail "the gate reported an unrecognized value for whether the build is needed ('$build_wanted')" ;;
esac

if [ "$build_wanted" = "false" ]; then
  { [ "$arm64_result" = "skipped" ] && [ "$intel_result" = "skipped" ]; } \
    || fail "the iOS build was not needed but its results are arm64='$arm64_result' intel='$intel_result' (expected both 'skipped')"
  echo "iOS build was not needed (no change it depends on) and was skipped."
  exit 0
fi

# The build was wanted. A failure on either pool fails the check, whatever the other did.
[ "$arm64_result" != "failure" ] || fail "the iOS build failed on the arm64 pool (the Intel pool: '$intel_result')"
[ "$intel_result" != "failure" ] || fail "the iOS build failed on the Intel pool (the arm64 pool: '$arm64_result')"

built=()
[ "$arm64_result" != "success" ] || built+=("arm64")
[ "$intel_result" != "success" ] || built+=("intel")

if [ "${#built[@]}" -eq 0 ]; then
  [ "$arm64_result" != "cancelled" ] || explain_cancelled arm64 "$ARM64_JOB"
  [ "$intel_result" != "cancelled" ] || explain_cancelled intel "$INTEL_JOB"
  fail "the iOS build was needed but neither pool built it (arm64: '$arm64_result', intel: '$intel_result')"
fi

echo "iOS build was needed and succeeded on: ${built[*]}."
[ "$arm64_result" != "cancelled" ] || explain_cancelled arm64 "$ARM64_JOB"
[ "$intel_result" != "cancelled" ] || explain_cancelled intel "$INTEL_JOB"
exit 0
