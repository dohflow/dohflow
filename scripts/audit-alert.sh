#!/usr/bin/env bash
#
# File, update or annotate the ONE GitHub issue for a dependency advisory on main
# (personal-cfo-md8h2.14). Called by the daily advisory-audit workflow.
#
#   scripts/audit-alert.sh --status failed|clean --report FILE [--repo OWNER/REPO] [--run-url URL]
#
#   failed  the audit found something (an advisory, or an allowance that matches
#           nothing). Opens an issue labeled `security-advisory` naming the advisory
#           IDs found in the report, or adds a comment to the one already open.
#   clean   the audit passed. If an issue is open, adds a comment saying so. It never
#           closes an issue: a person closes it once the fix is confirmed.
#
# The report text is untrusted tool output; it is only ever placed in an issue body
# inside a code fence, never in a shell. The agent on duty turns the issue into a P0
# bead (ADR 0069: P0 bypasses queues) - see docs/security/scanning.md.

set -euo pipefail

STATUS="" REPORT="" REPO="${GITHUB_REPOSITORY:-}" RUN_URL=""
while [ $# -gt 0 ]; do
  case "$1" in
    --status) STATUS="${2:-}"; shift ;;
    --report) REPORT="${2:-}"; shift ;;
    --repo) REPO="${2:-}"; shift ;;
    --run-url) RUN_URL="${2:-}"; shift ;;
    *) echo "audit-alert: unknown option: $1" >&2; exit 2 ;;
  esac
  shift
done

case "$STATUS" in failed|clean) ;; *) echo "audit-alert: --status must be failed or clean" >&2; exit 2 ;; esac
[ -n "$REPO" ] || { echo "audit-alert: no repository (--repo or GITHUB_REPOSITORY)" >&2; exit 2; }
[ -f "$REPORT" ] || { echo "audit-alert: no report file: $REPORT" >&2; exit 2; }

LABEL=security-advisory
gh label create "$LABEL" --repo "$REPO" --color b60205 \
  --description "A dependency advisory or stale audit allowance on main" 2>/dev/null || true

existing="$(gh issue list --repo "$REPO" --label "$LABEL" --state open --json number --jq '.[0].number // empty')"
run_line=""
[ -n "$RUN_URL" ] && run_line="Run: $RUN_URL"

if [ "$STATUS" = clean ]; then
  if [ -n "$existing" ]; then
    gh issue comment "$existing" --repo "$REPO" --body "The daily audit of main is clean again. ${run_line}
This issue is not closed automatically: close it once the fix is confirmed and the P0 bead is done."
    echo "Commented on #$existing: audit clean again."
  else
    echo "Audit clean and no open $LABEL issue: nothing to do."
  fi
  exit 0
fi

# failed: name the advisories (RUSTSEC / GHSA / CVE ids) found in the report.
# (grep exits 1 when a report names no advisory, e.g. a stale allowance; that is not an error.)
ids="$(grep -oE 'RUSTSEC-[0-9]{4}-[0-9]{4}|GHSA(-[a-z0-9]{4}){3}|CVE-[0-9]{4}-[0-9]{4,}' "$REPORT" | sort -u | paste -sd, - | sed 's/,/, /g' || true)"
stale=""
grep -qi 'matches nothing' "$REPORT" && stale="A stale allowance (an exception that matches nothing) was reported: remove it. "

if [ -n "$ids" ]; then
  title="Security advisory on main: $ids"
else
  title="Dependency audit failing on main (no advisory ID in the report)"
fi

# Cap the report so the issue stays readable; the full output is in the run.
report_text="$(head -c 40000 "$REPORT")"
body="The daily dependency audit of \`main\` failed. ${stale}${run_line}

The agent on duty files a P0 bead from this issue (ADR 0069: P0 bypasses queues); see docs/security/scanning.md.

Report (tool output):

~~~~
${report_text}
~~~~"

if [ -n "$existing" ]; then
  gh issue comment "$existing" --repo "$REPO" --body "$body"
  echo "Updated #$existing."
else
  gh issue create --repo "$REPO" --label "$LABEL" --title "$title" --body "$body"
fi
