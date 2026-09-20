#!/usr/bin/env bash
#
# Contract regression test for ADR 0074 (personal-cfo-j8ab7).
#
# ADR 0074 is the authority that future Sync persistence, service, wire, and
# client beads implement. These assertions deliberately guard the safety
# boundaries added during architecture review: a rebase must not suppress an
# unpushed command through an inherited idempotency memo; the service must not
# be a nonce-authority; and class-1b artifacts need an opaque, durable closure
# before history may be truncated. This is a document contract test, not a
# substitute for the runtime simulator those downstream beads must ship.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
ADR="$REPO_ROOT/docs/adr/0074-dohflow-sync-architecture.md"

if [ ! -f "$ADR" ]; then
  echo "FAIL — ADR 0074 is missing: $ADR" >&2
  exit 1
fi

CASES=0
require_text() {
  local label="$1" needle="$2"
  CASES=$((CASES + 1))
  if grep -Fq -- "$needle" "$ADR"; then
    echo "    ok   — $label"
  else
    echo "    FAIL — $label (missing: $needle)" >&2
    exit 1
  fi
}

require_text "rebase captures a durable plan" "rebase plan for every local outbox"
require_text "replay-owned memos are withheld" "replay-owned idempotency memo is deliberately absent"
require_text "replay rebuilds the memo target" "rebuilt memo points at the"
require_text "queued retry cannot silently apply" "QueuedForDisposition"
require_text "rebase validates an artifact closure before swap" "durable receipt/pin closure"
require_text "envelopes preserve replay identity" "complete original \`CommandMeta\`"
require_text "service is not the nonce authority" "it is **not** a confidentiality authority"
require_text "membership starts from a client-created genesis" "The first Sync device creates the genesis membership record locally"
require_text "AEAD keys are client-derived" "K_e,d,t = HKDF-SHA256-Expand"
require_text "counter rollback fails closed" "NonceStateLost"
require_text "service nonce tracking is only a backstop" "not the security source"
require_text "artifact addresses are opaque and epoch scoped" "sync_artifact_id_e"
require_text "artifact publication is upload first" "Publication is upload-before-reference"
require_text "snapshot truncation needs a durable closure" "closure pin are durable in one CAS transaction"
require_text "artifact cache loss has a typed recovery path" "ArtifactRebootstrapRequired"
require_text "artifact privacy covers epoch rotation" "A fresh epoch therefore re-addresses and re-seals every"
require_text "fact-sheet invariant stays intact" "The local app needs no DohFlow server and no DohFlow account."

echo "PASS — $CASES ADR 0074 contract assertions, 0 failures"
