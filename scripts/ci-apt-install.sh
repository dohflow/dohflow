#!/usr/bin/env bash
#
# personal-cfo-56w.6: install apt packages on a hosted Ubuntu runner so that a hung or unreachable
# mirror ends as a red step inside a known time, never as a stalled job.
#
#   1. `apt-get update` then `install` with network retries and a per-connection timeout, the WHOLE
#      attempt (update plus install) capped by ONE `timeout`, against the runner's default mirror
#      (azure.archive.ubuntu.com).
#   2. If that fails, say so, rewrite the apt sources to the canonical archive.ubuntu.com and
#      security.ubuntu.com, and try ONCE more. (2026-11-11: the default mirror did not resolve for
#      14 minutes, and retries against a mirror that is down cannot help.)
#   3. Log which mirror served the install (and a ::notice:: annotation when it was the fallback).
#
# The caller's step carries its own `timeout-minutes` (10). Worst case here: two attempts of 240 s
# (8 minutes) plus a source rewrite that takes a moment, so a hang on the first mirror still reaches
# the fallback inside the step, with about two minutes of margin.
#
# Usage: scripts/ci-apt-install.sh <package> [<package> ...]
# Test hooks (never set in CI): CI_APT_SOURCES (space-separated apt source files to rewrite),
# CI_APT_ATTEMPT_SECONDS (per-attempt cap), CI_APT_SUDO (the privilege command; empty = none).
# Exit: 0 installed; 1 both attempts failed; 2 usage.

set -euo pipefail

if [ "$#" -eq 0 ]; then
  echo "usage: ci-apt-install.sh <package> [<package> ...]" >&2
  exit 2
fi

attempt_seconds="${CI_APT_ATTEMPT_SECONDS:-240}"
sudo_cmd="${CI_APT_SUDO-sudo}"
sources="${CI_APT_SOURCES:-/etc/apt/sources.list /etc/apt/sources.list.d/ubuntu.sources}"

# One attempt: update, then install, under a single cap. The cap is on the whole attempt, not on each apt-get call, so
# two attempts always fit the step's timeout however the first one fails. The inline script gets the packages as arguments.
attempt() {
  # shellcheck disable=SC2016
  $sudo_cmd timeout "$attempt_seconds" bash -c '
    set -e
    opts=(-o Acquire::Retries=3 -o Acquire::http::Timeout=20 -o Acquire::https::Timeout=20 -o DPkg::Lock::Timeout=60)
    apt-get "${opts[@]}" update
    apt-get "${opts[@]}" install -y "$@"
  ' _ "$@"
}

if attempt "$@"; then
  echo "apt mirror: default (the runner's own mirror) served the install"
  exit 0
fi

echo "::warning::apt failed against the runner's default mirror; switching to archive.ubuntu.com and security.ubuntu.com and trying once more" >&2
for file in $sources; do
  [ -f "$file" ] || continue
  # Only the host names change: azure.archive.ubuntu.com -> archive.ubuntu.com, and any security mirror -> security.ubuntu.com.
  # Written back through `tee` (not `sed -i`, whose flags differ between GNU and BSD sed) so the script also runs on a Mac.
  rewritten="$(sed -E 's#//([A-Za-z0-9.-]+\.)?archive\.ubuntu\.com#//archive.ubuntu.com#g; s#//([A-Za-z0-9.-]+\.)?security\.ubuntu\.com#//security.ubuntu.com#g' "$file")"
  printf '%s\n' "$rewritten" | $sudo_cmd tee "$file" >/dev/null
done

if attempt "$@"; then
  echo "apt mirror: archive.ubuntu.com and security.ubuntu.com (the FALLBACK) served the install"
  echo "::notice::apt install used the canonical mirrors: the runner's default mirror failed"
  exit 0
fi

echo "::error::apt install failed against the default mirror and against archive.ubuntu.com" >&2
exit 1
