#!/usr/bin/env bash
#
# The "Review gate" (personal-cfo-md8h2.23, owner decision personal-cfo-915.20): a change reaches
# `main` only while the reviewer's `merge-ready` label is on it, and the label is what the
# review-pr procedure applies ONLY at the exact head SHA it reviewed (and what
# merge-ready-label.yml removes on every new push). It makes the label load-bearing. It is a
# GUARDRAIL against mistakes, not malice (ADR 0071): labels are not permission-restricted, any
# commenter can write a marker, and a pull request can edit this very workflow.
#
#   scripts/review-gate.sh --event pull_request --action ACTION --repo OWNER/REPO --pr N
#   scripts/review-gate.sh --event merge_group  --repo OWNER/REPO --ref REF
#
# pull_request: passes only when the pull request CURRENTLY carries `merge-ready`, and NEVER on a
# `synchronize` (a new commit): the label on a just-pushed head is stale by definition, and the
# workflow that removes it uses the Actions token, whose label removal does not trigger this gate
# again, so reading the label during the push would race the removal and could pass. A reviewer
# re-applies the label (a `labeled` event) to the exact head they reviewed, and the gate then passes.
#
# merge_group: each queue entry runs on its own and is named for its last pull request in the ref
# `refs/heads/gh-readonly-queue/<base>/pr-<N>-<sha>`; the <sha> there is the entry's BASE, not the
# PR's head. It gates ONLY pull request N, and passes only if (a) N carries `merge-ready`, and (b) N's
# current head (headRefOid) EQUALS the full SHA in the NEWEST review-verdict marker on the pull
# request, that marker says PASS, and no later marker says anything else. The marker is a hidden
# line in the reviewer's verdict comment, written by the review-pr procedure:
#
#     <!-- review-verdict: PASS <full 40-hex SHA> -->
#
# (any other verdict word, CHANGES_REQUIRED or HUMAN_DECISION_REQUIRED, takes the same shape). The
# queue squash-merges, so the tested commit is never a descendant of the PR head and ancestry cannot
# bind the head; the marker can. A marker that starts `<!-- review-verdict:` but is not exactly this
# shape is an error, not an absence.
#
# It FAILS CLOSED: an unparsable ref, a lookup error, a missing or malformed value, a missing label,
# no marker, a malformed marker, a marker for another SHA, or a newer non-PASS marker all exit
# non-zero. Only the read-only token is needed (pull-requests: read).
#
# DOCUMENTED LIMITATIONS: the head binding relies on GitHub dequeuing an entry when the pull request
# gets a new commit, plus the label removal. Removing the label AFTER an entry's run has passed does
# not eject that entry from the queue; only a new commit does. A multi-PR group is several entries,
# each gated in its own run.

set -euo pipefail

EVENT="" ACTION="" REPO="${GITHUB_REPOSITORY:-}" PR="" REF=""
while [ $# -gt 0 ]; do
  case "$1" in
    --event) EVENT="${2:-}"; shift ;;
    --action) ACTION="${2:-}"; shift ;;
    --repo) REPO="${2:-}"; shift ;;
    --pr) PR="${2:-}"; shift ;;
    --ref) REF="${2:-}"; shift ;;
    *) echo "review-gate: unknown option: $1" >&2; exit 2 ;;
  esac
  shift
done

deny() { echo "::error title=Review gate::$1" >&2; echo "Review gate: FAIL — $1" >&2; exit 1; }
ok() { echo "Review gate: PASS — $1"; exit 0; }

[ -n "$REPO" ] || { echo "review-gate: no repository" >&2; exit 2; }

# Sets LABELS (comma-joined), HEAD and MARKERS (one "VERDICT SHA" per line, oldest first) for pull
# request $1, or fails (closed) on any lookup error or malformed marker.
lookup() {
  local info
  if ! info="$(gh pr view "$1" --repo "$REPO" --json labels,headRefOid,comments 2>/dev/null)"; then
    deny "could not read pull request #$1 (failing closed)"
  fi
  LABELS="$(jq -r '[.labels[].name] | join(",")' <<<"$info" 2>/dev/null)" || deny "unreadable answer for pull request #$1 (failing closed)"
  HEAD="$(jq -r '.headRefOid // empty' <<<"$info" 2>/dev/null)" || deny "unreadable answer for pull request #$1 (failing closed)"
  # Every "<!-- review-verdict:" in every comment, oldest comment first, in order within a comment.
  local raw
  raw="$(jq -r '.comments[].body | [match("<!-- review-verdict:(?:(?!-->).)*(?:-->)?"; "g").string] | .[]' <<<"$info" 2>/dev/null)" \
    || deny "unreadable comments on pull request #$1 (failing closed)"
  MARKERS=""
  local line
  while IFS= read -r line; do
    [ -n "$line" ] || continue
    if [[ "$line" =~ ^'<!-- review-verdict: '([A-Z_]+)' '([0-9a-f]{40})' -->'$ ]]; then
      MARKERS="${MARKERS}${BASH_REMATCH[1]} ${BASH_REMATCH[2]}"$'\n'
    else
      deny "pull request #$1 has a malformed review-verdict marker (failing closed)"
    fi
  done <<<"$raw"
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
    re='^refs/heads/gh-readonly-queue/[^/]+/pr-([0-9]+)-([0-9a-f]{40})$'
    [[ "$REF" =~ $re ]] || deny "cannot read a pull request number from the queue ref (failing closed)"
    n="${BASH_REMATCH[1]}"
    lookup "$n"
    has_label "$LABELS" || deny "queued pull request #$n does not carry merge-ready"
    [[ "$HEAD" =~ ^[0-9a-f]{40}$ ]] || deny "pull request #$n has no readable head (failing closed)"
    [ -n "$MARKERS" ] || deny "pull request #$n has no review-verdict marker: the review has to post one (failing closed)"
    newest="${MARKERS%$'\n'}"
    newest="${newest##*$'\n'}"
    verdict="${newest%% *}"
    reviewed="${newest##* }"
    [ "$verdict" = PASS ] || deny "the newest review verdict on #$n is $verdict, not PASS"
    [ "$reviewed" = "$HEAD" ] || deny "pull request #$n's current head ${HEAD:0:12} is not the head the reviewer passed (${reviewed:0:12})"
    ok "queued pull request #$n carries merge-ready and its head ${HEAD:0:12} is the newest PASS"
    ;;
  "") echo "review-gate: --event is required" >&2; exit 2 ;;
  *) deny "unknown event '$EVENT' (failing closed)" ;;
esac
