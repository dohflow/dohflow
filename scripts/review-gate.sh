#!/usr/bin/env bash
#
# The "Review gate" (personal-cfo-md8h2.23, owner decision personal-cfo-915.20): a change reaches
# `main` only while the reviewer's `merge-ready` label is on it, and the label is what the
# review-pr procedure applies ONLY at the exact head SHA it reviewed (and what
# merge-ready-label.yml removes on every new push). It makes the label load-bearing. It is a
# GUARDRAIL, not a security boundary: labels are not permission-restricted on GitHub and a pull
# request can edit this very workflow (ADR 0071's posture).
#
#   scripts/review-gate.sh --event pull_request --action ACTION --repo OWNER/REPO --pr N
#   scripts/review-gate.sh --event merge_group  --repo OWNER/REPO --ref REF --head-sha SHA
#
# pull_request: passes only when the pull request CURRENTLY carries `merge-ready`, and NEVER on a
# `synchronize` (a new commit): the label on a just-pushed head is stale by definition, and the
# workflow that removes it uses the Actions token, whose label removal does not trigger this gate
# again, so reading the label during the push would race the removal and could pass. A reviewer
# re-applies the label (a `labeled` event) to the exact head they reviewed, and the gate then passes.
#
# merge_group: each queue entry runs on its own and is named for its last pull request in the ref
# `refs/heads/gh-readonly-queue/<base>/pr-<N>-<sha>`; the <sha> there is the entry's BASE, not the
# PR's head. It gates ONLY pull request N: it passes only if N carries `merge-ready` AND N's
# current head (headRefOid) is an ANCESTOR of the tested commit (`--head-sha`, the temporary merge
# commit), so a head pushed after the review cannot be the one that merges.
#
# It FAILS CLOSED: an unparsable ref, a lookup error, a missing or malformed value, a missing label
# or a head that is not an ancestor all exit non-zero. Only the read-only token is needed
# (pull-requests: read, contents: read).
#
# DOCUMENTED LIMITATION: removing the label AFTER an entry's run has passed does not eject that
# entry from the queue; only a new commit does. Merge-queue entries also stack: a multi-PR group
# is several entries, each gated in its own run.

set -euo pipefail

EVENT="" ACTION="" REPO="${GITHUB_REPOSITORY:-}" PR="" REF="" HEAD_SHA=""
while [ $# -gt 0 ]; do
  case "$1" in
    --event) EVENT="${2:-}"; shift ;;
    --action) ACTION="${2:-}"; shift ;;
    --repo) REPO="${2:-}"; shift ;;
    --pr) PR="${2:-}"; shift ;;
    --ref) REF="${2:-}"; shift ;;
    --head-sha) HEAD_SHA="${2:-}"; shift ;;
    *) echo "review-gate: unknown option: $1" >&2; exit 2 ;;
  esac
  shift
done

deny() { echo "::error title=Review gate::$1" >&2; echo "Review gate: FAIL — $1" >&2; exit 1; }
ok() { echo "Review gate: PASS — $1"; exit 0; }

[ -n "$REPO" ] || { echo "review-gate: no repository" >&2; exit 2; }
sha_re='^[0-9a-f]{40}$'

# Sets LABELS (comma-joined) and HEAD for pull request $1, or fails (closed) on any lookup error.
lookup() {
  local info
  if ! info="$(gh pr view "$1" --repo "$REPO" --json labels,headRefOid 2>/dev/null)"; then
    deny "could not read pull request #$1 (failing closed)"
  fi
  LABELS="$(jq -r '[.labels[].name] | join(",")' <<<"$info" 2>/dev/null)" || deny "unreadable answer for pull request #$1 (failing closed)"
  HEAD="$(jq -r '.headRefOid // empty' <<<"$info" 2>/dev/null)" || deny "unreadable answer for pull request #$1 (failing closed)"
}

has_label() { case ",$1," in *,merge-ready,*) return 0 ;; *) return 1 ;; esac; }

case "$EVENT" in
  pull_request)
    [[ "$PR" =~ ^[0-9]+$ ]] || deny "no pull request number (failing closed)"
    if [ "$ACTION" = synchronize ]; then
      deny "pull request #$PR has a new commit: a reviewer must re-apply merge-ready to the exact head they reviewed"
    fi
    lookup "$PR"
    has_label "$LABELS" || deny "pull request #$PR does not carry merge-ready: a reviewer applies it to the exact head they reviewed"
    ok "pull request #$PR carries merge-ready"
    ;;
  merge_group)
    [[ "$HEAD_SHA" =~ $sha_re ]] || deny "the tested commit is not a full SHA (failing closed)"
    re='^refs/heads/gh-readonly-queue/[^/]+/pr-([0-9]+)-([0-9a-f]{40})$'
    [[ "$REF" =~ $re ]] || deny "cannot read a pull request number from the queue ref (failing closed)"
    n="${BASH_REMATCH[1]}"
    lookup "$n"
    head="$HEAD"
    has_label "$LABELS" || deny "queued pull request #$n does not carry merge-ready"
    [[ "$head" =~ $sha_re ]] || deny "pull request #$n has no readable head (failing closed)"
    if ! status="$(gh api "repos/$REPO/compare/${head}...${HEAD_SHA}" --jq .status 2>/dev/null)"; then
      deny "could not compare #$n's head with the tested commit (failing closed)"
    fi
    case "$status" in
      ahead|identical) ok "queued pull request #$n carries merge-ready and its head ${head:0:12} is part of the tested commit" ;;
      *) deny "pull request #$n's current head ${head:0:12} is not an ancestor of the tested commit (comparison: ${status:-none})" ;;
    esac
    ;;
  "") echo "review-gate: --event is required" >&2; exit 2 ;;
  *) deny "unknown event '$EVENT' (failing closed)" ;;
esac
