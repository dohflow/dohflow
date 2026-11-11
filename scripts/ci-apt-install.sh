#!/usr/bin/env bash
#
# personal-cfo-56w.6: install apt packages on a hosted Ubuntu runner so that a hung or unreachable
# mirror ends as a red step inside a known time, never as a stalled job.
#
#   1. `apt-get update` and `install` with network retries and a per-connection timeout, each
#      attempt capped by `timeout`, against the runner's default mirror (azure.archive.ubuntu.com).
#   2. If that fails, say so, rewrite the apt sources to the canonical archive.ubuntu.com and
#      security.ubuntu.com, and try ONCE more. (2026-11-11: the default mirror did not resolve for
#      14 minutes, and retries against a mirror that is down cannot help.)
#   3. Log which mirror served the install (and a ::notice:: annotation when it was the fallback).
#
# The caller's step carries its own `timeout-minutes` (10): the two attempts below add up to less.
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

attempt_seconds="${CI_APT_ATTEMPT_SECONDS:-270}"
sudo_cmd="${CI_APT_SUDO-sudo}"
sources="${CI_APT_SOURCES:-/etc/apt/sources.list /etc/apt/sources.list.d/ubuntu.sources}"
apt_opts=(-o Acquire::Retries=3 -o Acquire::http::Timeout=20 -o Acquire::https::Timeout=20 -o DPkg::Lock::Timeout=60)

run_apt() {
  # shellcheck disable=SC2086
  $sudo_cmd timeout "$attempt_seconds" apt-get "${apt_opts[@]}" "$@"
}

attempt() {
  run_apt update && run_apt install -y "$@"
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
