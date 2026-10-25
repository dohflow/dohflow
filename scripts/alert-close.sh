#!/usr/bin/env bash
#
# Close the open alert issues of one label when the problem they report is gone
# (personal-cfo-md8h2.21). Used by the daily advisory audit (security-advisory) and the
# main-red alert (ci-main-red).
#
#   scripts/alert-close.sh --label L --repo OWNER/REPO --run-url URL --what "the daily audit of main is clean"
#                          [--needs-json JSON] [--only-if-tip SHA]
#
# For every open issue labeled L it adds a comment linking the clean run and closes the issue as
# completed, EXCEPT:
#   - an issue that also carries the `keep-open` label, and
#   - an issue a human reopened within the last 24 hours (a deliberate reopen must stick).
# `--needs-json` is the job-results object of a GitHub Actions job (`toJSON(needs)`): unless every
# result is "success" nothing is closed, so a skipped or cancelled gate never counts as green.
#
# `--only-if-tip SHA` closes nothing unless SHA is the current tip of `main`: a green run of an
# OLDER commit must not close an alert opened by a red run of a newer one (the sequence A, B: B is
# red and alerts first, then A's older green run finishes). If the tip has no run of its own (a
# `[skip ci]` commit) the alert stays open; closing is the conservative-to-skip direction here.
#
# Closing is not silent: each close carries the comment with the run link. The script only reads
# issues and, for an eligible issue, comments and closes it; it never reopens, edits or deletes.

set -euo pipefail

LABEL="" REPO="${GITHUB_REPOSITORY:-}" RUN_URL="" WHAT="" NEEDS="" TIP_SHA=""
while [ $# -gt 0 ]; do
  case "$1" in
    --label) LABEL="${2:-}"; shift ;;
    --repo) REPO="${2:-}"; shift ;;
    --run-url) RUN_URL="${2:-}"; shift ;;
    --what) WHAT="${2:-}"; shift ;;
    --needs-json) NEEDS="${2:-}"; shift ;;
    --only-if-tip) TIP_SHA="${2:-}"; shift ;;
    *) echo "alert-close: unknown option: $1" >&2; exit 2 ;;
  esac
  shift
done
[ -n "$LABEL" ] || { echo "alert-close: --label is required" >&2; exit 2; }
[ -n "$REPO" ] || { echo "alert-close: no repository (--repo or GITHUB_REPOSITORY)" >&2; exit 2; }
[ -n "$WHAT" ] || WHAT="the problem is gone"

if [ -n "$NEEDS" ]; then
  not_green="$(jq -r '[to_entries[] | select(.value.result != "success") | .key] | join(", ")' <<<"$NEEDS")"
  if [ -n "$not_green" ]; then
    echo "Not green (${not_green}): closing nothing."
    exit 0
  fi
fi

if [ -n "$TIP_SHA" ]; then
  tip="$(gh api "repos/$REPO/commits/main" --jq .sha)"
  if [ "$tip" != "$TIP_SHA" ]; then
    echo "main has moved on from ${TIP_SHA:0:12} (tip is ${tip:0:12}): closing nothing; the tip's own run decides."
    exit 0
  fi
fi

lines="$(gh issue list --repo "$REPO" --label "$LABEL" --state open --limit 100 \
  --json number,labels --jq '.[] | "\(.number)\t\([.labels[].name] | join(","))"')"
if [ -z "$lines" ]; then
  echo "No open $LABEL issue: nothing to close."
  exit 0
fi

while IFS="$(printf '\t')" read -r number labels; do
  [ -n "$number" ] || continue
  case ",$labels," in
    *,keep-open,*) echo "#$number carries keep-open: left open."; continue ;;
  esac
  recent="$(gh api "repos/$REPO/issues/$number/events" --paginate \
    --jq '[.[] | select(.event == "reopened") | select(now - (.created_at | fromdateiso8601) < 86400)] | length' \
    | awk '{ total += $1 } END { print total + 0 }')"
  if [ "${recent:-0}" -gt 0 ]; then
    echo "#$number was reopened within the last 24 hours: left open."
    continue
  fi
  run_line=""
  [ -n "$RUN_URL" ] && run_line=" Run: $RUN_URL"
  gh issue comment "$number" --repo "$REPO" \
    --body "Closing automatically: ${WHAT}.${run_line}
Reopen this issue (or add the \`keep-open\` label) if that is wrong; a reopen holds for 24 hours."
  gh issue close "$number" --repo "$REPO" --reason completed
  echo "Closed #$number."
done <<<"$lines"
