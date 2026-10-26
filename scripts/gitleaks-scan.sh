#!/usr/bin/env bash
#
# Run gitleaks over the commits a CI run is responsible for, and ONLY those
# (personal-cfo-md8h2.22).
#
#   scripts/gitleaks-scan.sh --event EVENT [--base SHA] [--head SHA] [--report FILE]
#   scripts/gitleaks-scan.sh --all-refs [--report FILE]
#
# Why: a bare `gitleaks detect` walks `git log --all`, every branch the checkout fetched. One
# pushed branch carrying a flagged file then failed the REQUIRED scan of every pull request,
# queue run and push to main (2026-10-25: 36 hits on one unrelated branch blocked the queue).
# The required scan must read the change under test and nothing else:
#
#   pull_request, merge_group   BASE..HEAD   (the pull request's, or the queue group's, commits)
#   push                        BEFORE..AFTER (the pushed range); a first push (no BEFORE)
#                               reads the commits reachable from AFTER only
#   schedule, workflow_dispatch the history reachable from the checked-out commit (`HEAD`)
#
# `--all-refs` is the opposite, for the daily non-required audit of every branch and tag.
#
# It fails CLOSED: an event it does not know, or a range with a missing or malformed SHA, is an
# error (exit 2), never a quiet full or empty scan. It always passes an explicit --log-opts.
# Anything else on the command line after `--` goes to gitleaks (for example --report-path).

set -euo pipefail

EVENT="" BASE="" HEAD_SHA="" ALL_REFS=0 REPORT=""
while [ $# -gt 0 ]; do
  case "$1" in
    --event) EVENT="${2:-}"; shift ;;
    --base) BASE="${2:-}"; shift ;;
    --head) HEAD_SHA="${2:-}"; shift ;;
    --all-refs) ALL_REFS=1 ;;
    --report) REPORT="${2:-}"; shift ;;
    --) shift; break ;;
    *) echo "gitleaks-scan: unknown option: $1" >&2; exit 2 ;;
  esac
  shift
done

sha_re='^[0-9a-f]{40}$'
is_sha() { [[ "$1" =~ $sha_re ]]; }
zeros="0000000000000000000000000000000000000000"

if [ "$ALL_REFS" = 1 ]; then
  [ -z "$EVENT" ] || { echo "gitleaks-scan: --all-refs takes no --event" >&2; exit 2; }
  log_opts="--all"
else
  case "$EVENT" in
    pull_request|merge_group)
      if ! is_sha "$BASE" || ! is_sha "$HEAD_SHA"; then
        echo "gitleaks-scan: $EVENT needs --base and --head as full 40-character SHAs (failing closed)" >&2
        exit 2
      fi
      log_opts="${BASE}..${HEAD_SHA}"
      ;;
    push)
      if [ -z "$BASE" ] || [ "$BASE" = "$zeros" ]; then
        # A first push has no "before": the commits reachable from the pushed head only.
        if ! is_sha "$HEAD_SHA"; then echo "gitleaks-scan: push needs --head as a full SHA (failing closed)" >&2; exit 2; fi
        log_opts="$HEAD_SHA"
      else
        if ! is_sha "$BASE" || ! is_sha "$HEAD_SHA"; then
          echo "gitleaks-scan: push needs --base and --head as full 40-character SHAs (failing closed)" >&2
          exit 2
        fi
        log_opts="${BASE}..${HEAD_SHA}"
      fi
      ;;
    schedule|workflow_dispatch)
      log_opts="HEAD"
      ;;
    "") echo "gitleaks-scan: --event (or --all-refs) is required" >&2; exit 2 ;;
    *) echo "gitleaks-scan: unknown event '$EVENT' (failing closed)" >&2; exit 2 ;;
  esac
fi

echo "gitleaks: scanning --log-opts=\"$log_opts\""
args=(detect --config .gitleaks.toml --no-banner --redact "--log-opts=$log_opts")
if [ -n "$REPORT" ]; then
  args+=(--report-format json --report-path "$REPORT")
fi
exec gitleaks "${args[@]}" "$@"
