#!/usr/bin/env bash
#
# Waits for the two macOS pools of a required check, for at most a hard deadline, and then
# hands what it saw to scripts/macos-pools-verdict.sh, which writes the only verdict
# (personal-cfo-md8h2.26; the pools themselves are personal-cfo-md8h2.24 and .25).
#
# Why this exists. The verdict job used to `need` both pool jobs, so the slower pool set the
# check's latency, and `timeout-minutes` did not bound a job stuck in setup: run 41920261907's
# Intel pool sat in "Set up job" for 55:00 against a 45-minute timeout (and a job GitHub never
# finds a runner for is cancelled by the platform after about 15 minutes whatever its timeout
# says). The merge queue drops a pull request whose required check has not reported in 60
# minutes. This script makes the check report by a deadline of its own, from the jobs API, so no
# claim about what GitHub does to a stalled job is needed.
#
# What it does:
#
#   * Polls this run's jobs (this run_attempt only, matched by exact job name) every
#     VERDICT_POLL_SECONDS (30) until BOTH pool jobs have a conclusion, or a pool has FAILED
#     (a failure cannot be undone within an attempt, so waiting for the other pool would only
#     delay the red), or VERDICT_WAIT_SECONDS (1200, twenty minutes) have passed. It never stops
#     early on one pool's success: the other pool may still fail, and that failure fails the check.
#   * Maps each pool to one of:
#       success, cancelled, skipped;
#       failure   - a completed job with ANY other conclusion (failure, timed_out, startup_failure,
#                   action_required, neutral, stale, or none), which fails the check;
#       unfinished - queued, waiting or in progress: the ONLY neutral state;
#       unknown   - the job is missing or appears twice, or the response is unreadable or
#                   unparseable (the API or token failed). A missing job is waited for while the
#                   wait runs (it may not have been created yet), but it is unknown at the end,
#                   and an unknown pool fails the check whatever the other pool did: a renamed
#                   job must not let the check pass on one pool alone.
#   * Runs the verdict script with those values. The verdict script passes when the gate
#     succeeded and a pool succeeded and no pool failed or is unknown, and prints a notice for a
#     pool left unfinished; with no success it fails and says which case it was. A build the
#     gate did not want has both pools skipped by the gate's output, which counts as finished
#     here, so that case does not wait.
#   * The final poll decides. If the last poll could not be read, the states are unknown and the
#     check fails: an earlier readable snapshot is not reused, because the pool it showed
#     succeeding may be the one whose job was since renamed or duplicated.
#
# The gate result and the "build wanted" value are checked by the verdict script, not here: a
# failed gate, or a value that is neither true nor false, is handed straight to it with both
# pools unknown, without waiting.
#
# Usage: macos-pools-wait.sh <gate result> <build wanted: true|false|"">
#
# Naming (as for the verdict script): VERDICT_CHECK, VERDICT_BUILD, VERDICT_ARM64_JOB,
# VERDICT_INTEL_JOB. Live inputs: GITHUB_REPOSITORY, GITHUB_RUN_ID, GITHUB_RUN_ATTEMPT, GH_TOKEN
# (the verdict job has `actions: read`).
#
# Test inputs (no live API call): VERDICT_SNAPSHOTS_DIR, a directory holding 1.json, 2.json, ...
# one jobs response per poll (poll n reads n.json, or the highest-numbered file below n; a file
# whose content is exactly UNREADABLE stands for a failed API call), and VERDICT_VIRTUAL_CLOCK=1,
# which advances a virtual clock by the poll interval instead of sleeping.

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"

gate_result="${1:?gate result}"
build_wanted="${2-}"

check_label="${VERDICT_CHECK:-iOS cross-compile check}"
ARM64_JOB="${VERDICT_ARM64_JOB:-iOS cross-compile (macOS build, arm64)}"
INTEL_JOB="${VERDICT_INTEL_JOB:-iOS cross-compile (macOS build, intel)}"
deadline="${VERDICT_WAIT_SECONDS:-1200}"
interval="${VERDICT_POLL_SECONDS:-30}"
attempt="${GITHUB_RUN_ATTEMPT:-1}"

case "$deadline$interval$attempt" in
  *[!0-9]*) echo "::error::${check_label} failed closed: the wait settings are not whole numbers" >&2; exit 1 ;;
esac
[ "$interval" -gt 0 ] || { echo "::error::${check_label} failed closed: the poll interval must be positive" >&2; exit 1; }

# A failed gate or an unrecognized "wanted" value needs no waiting: the verdict script refuses it.
if [ "$gate_result" != "success" ] || { [ "$build_wanted" != "true" ] && [ "$build_wanted" != "false" ]; }; then
  exec "$here/macos-pools-verdict.sh" "$gate_result" "$build_wanted" unknown unknown
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
last_snapshot=""
virtual_now=0

now() {
  if [ -n "${VERDICT_VIRTUAL_CLOCK-}" ]; then echo "$virtual_now"; else echo "$SECONDS"; fi
}
pause() {
  if [ -n "${VERDICT_VIRTUAL_CLOCK-}" ]; then virtual_now=$((virtual_now + interval)); else sleep "$interval"; fi
}

# fetch <poll number>: prints the jobs response, or fails.
fetch() {
  local n="$1" file="" k
  if [ -n "${VERDICT_SNAPSHOTS_DIR-}" ]; then
    k="$n"
    while [ "$k" -ge 1 ]; do
      if [ -f "$VERDICT_SNAPSHOTS_DIR/$k.json" ]; then file="$VERDICT_SNAPSHOTS_DIR/$k.json"; break; fi
      k=$((k - 1))
    done
    [ -n "$file" ] || return 1
    [ "$(cat "$file")" != "UNREADABLE" ] || return 1
    cat "$file"
    return 0
  fi
  [ -n "${GITHUB_REPOSITORY-}" ] && [ -n "${GITHUB_RUN_ID-}" ] && command -v gh >/dev/null 2>&1 || return 1
  gh api "repos/${GITHUB_REPOSITORY}/actions/runs/${GITHUB_RUN_ID}/jobs?filter=latest&per_page=100"
}

# pool_state <jobs json file> <job name>: success, failure, cancelled, skipped or unfinished,
# or unknown, with a reason after a colon for failure and unknown. Exact name, this attempt only.
pool_state() {
  jq -r --arg name "$2" --argjson attempt "$attempt" '
    [.jobs[]? | select(.name == $name and .run_attempt == $attempt)] as $m
    | if ($m | length) == 0 then "unknown:the job is missing from the response"
      elif ($m | length) > 1 then "unknown:the job appears more than once"
      else $m[0] as $j
        | if $j.status != "completed" then "unfinished"
          else ($j.conclusion // "") as $c
            | if $c == "success" then "success"
              elif $c == "cancelled" then "cancelled"
              elif $c == "skipped" then "skipped"
              else "failure:the job concluded \"" + (if $c == "" then "none" else $c end) + "\"" end
          end
      end' "$1" 2>/dev/null || echo "unknown:the response could not be parsed"
}

concluded() {
  case "$1" in success | failure | cancelled | skipped) return 0 ;; *) return 1 ;; esac
}

poll=0
while :; do
  poll=$((poll + 1))
  arm64_state="unknown:the jobs API could not be read"
  intel_state="$arm64_state"
  snapshot="$work/snapshot-$poll.json"
  if fetch "$poll" >"$snapshot" 2>/dev/null && jq -e . "$snapshot" >/dev/null 2>&1; then
    last_snapshot="$snapshot"
    arm64_state="$(pool_state "$snapshot" "$ARM64_JOB")"
    intel_state="$(pool_state "$snapshot" "$INTEL_JOB")"
  fi
  echo "poll $poll at ${SECONDS}s (virtual ${virtual_now}s): arm64=${arm64_state} intel=${intel_state}" >&2

  # A failure cannot be undone within an attempt.
  if [ "${arm64_state%%:*}" = "failure" ] || [ "${intel_state%%:*}" = "failure" ]; then break; fi
  # Both pools have a conclusion (a skipped pool is finished): nothing left to wait for.
  if concluded "${arm64_state%%:*}" && concluded "${intel_state%%:*}"; then break; fi
  if [ "$(now)" -ge "$deadline" ]; then
    echo "::notice::${check_label}: the wait reached its ${deadline}-second deadline with arm64=${arm64_state} intel=${intel_state}." >&2
    break
  fi
  pause
done

# The verdict script's diagnostics read the last good response, never a fresh API call.
if [ -n "$last_snapshot" ]; then export VERDICT_JOBS_JSON="$last_snapshot"; fi

# failure:<reason> and unknown:<reason> go to the verdict as `failure` and `unknown`, with the
# reason said here.
for pool in arm64 intel; do
  state_var="${pool}_state"
  state="${!state_var}"
  case "$state" in
    unknown:*)
      echo "::warning::${check_label}: the ${pool} pool's state is unknown (${state#unknown:}); an unknown pool fails the check." >&2
      printf -v "$state_var" '%s' unknown ;;
    failure:*)
      echo "::warning::${check_label}: the ${pool} pool's job is a failure (${state#failure:})." >&2
      printf -v "$state_var" '%s' failure ;;
  esac
done

"$here/macos-pools-verdict.sh" "$gate_result" "$build_wanted" "$arm64_state" "$intel_state"
