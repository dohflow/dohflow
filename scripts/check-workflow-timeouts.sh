#!/usr/bin/env bash
#
# personal-cfo-56w.6: every job in every workflow has a job-level `timeout-minutes`; no workflow
# calls `apt-get` directly (an apt step goes through scripts/ci-apt-install.sh, which bounds and
# retries it and falls back to the canonical mirrors); and a step that calls that script has its
# own step-level `timeout-minutes`.
#
# Why: a job with no timeout runs for GitHub's 6-hour default. In October and November 2026 a hung
# `apt-get` in IPC codegen stalled the merge queue for about an hour, three times, until someone
# cancelled the run by hand.
#
# Usage: scripts/check-workflow-timeouts.sh [workflow-dir ...]   (default: .github/workflows)
# Exit:  0 clean; 1 a violation (each one is printed); 2 usage or an unreadable directory.
#
# It reads the workflows' own layout (jobs at two spaces of indent under `jobs:`, a job's keys at
# four), which every file here uses; it does not parse YAML. A job that is a call to a reusable
# workflow (`uses:` at the job level) cannot carry `timeout-minutes` and is skipped.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
dirs=("$@")
[ "${#dirs[@]}" -gt 0 ] || dirs=("$REPO_ROOT/.github/workflows")

status=0
checked=0
for dir in "${dirs[@]}"; do
  if [ ! -d "$dir" ]; then
    echo "check-workflow-timeouts: not a directory: $dir" >&2
    exit 2
  fi
  shopt -s nullglob
  files=("$dir"/*.yml "$dir"/*.yaml)
  shopt -u nullglob
  if [ "${#files[@]}" -eq 0 ]; then
    echo "check-workflow-timeouts: no workflow files in $dir" >&2
    exit 2
  fi
  for file in "${files[@]}"; do
    # awk prints one line per violation; a job's timeout must appear at four spaces of indent.
    out="$(awk -v file="$file" '
      function finish_step() {
        if (step_apt && !step_timeout) printf "%s:%d: a step that runs ci-apt-install.sh has no timeout-minutes of its own\n", file, step_line
        step_apt = 0; step_timeout = 0
      }
      function finish() {
        finish_step()
        if (job != "") {
          jobs++
          if (!reusable && !has_timeout) printf "%s: job \"%s\" has no timeout-minutes\n", file, job
        }
        job = ""; has_timeout = 0; reusable = 0
      }
      /^jobs:[[:space:]]*(#.*)?$/ { in_jobs = 1; next }
      in_jobs && /^[^[:space:]#]/ { finish(); in_jobs = 0 }
      !in_jobs { next }
      /^  [A-Za-z0-9_-]+:[[:space:]]*(#.*)?$/ { finish(); job = $1; sub(/:$/, "", job); next }
      job != "" && /^    timeout-minutes:[[:space:]]*[^[:space:]#]/ { has_timeout = 1 }
      job != "" && /^    uses:/ { reusable = 1 }
      /^      - / { finish_step(); step_line = NR }
      /^        timeout-minutes:[[:space:]]*[^[:space:]#]/ { step_timeout = 1 }
      /ci-apt-install\.sh/ && $0 !~ /^[[:space:]]*#/ { step_apt = 1 }
      /apt-get/ && $0 !~ /^[[:space:]]*#/ { printf "%s:%d: calls apt-get directly (use scripts/ci-apt-install.sh)\n", file, NR }
      END {
        finish()
        # A file this reads as having no jobs is a file it did not understand, never a clean one.
        if (jobs == 0) printf "%s: no jobs found (the guard reads `jobs:` at column 0 and jobs at two spaces of indent)\n", file
        printf "JOBS %d\n", jobs > "/dev/stderr"
      }
    ' "$file" 2>"${TMPDIR:-/tmp}/.cwt.$$")" || true
    n="$(sed -n 's/^JOBS //p' "${TMPDIR:-/tmp}/.cwt.$$")"
    rm -f "${TMPDIR:-/tmp}/.cwt.$$"
    checked=$((checked + ${n:-0}))
    if [ -n "$out" ]; then
      printf '%s\n' "$out" >&2
      status=1
    fi
  done
done

if [ "$status" -eq 0 ]; then
  if [ "$checked" -eq 0 ]; then
    echo "check-workflow-timeouts: found no jobs to check (is the layout what this script reads?)" >&2
    exit 2
  fi
  echo "All $checked workflow jobs have timeout-minutes, and no workflow calls apt-get directly."
fi
exit "$status"
