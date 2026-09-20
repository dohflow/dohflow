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
require_text "service retries accepted pushes before CAS" 'Before checking `base_seq`, the'
require_text "accepted pushes reconcile without replay" 'An exact match is **already accepted**'
require_text "same command id mismatch fails closed" 'mismatch fails closed as `ProtocolFork`'
require_text "accepted index has no retry horizon" "finite retry horizon or automatic expiry"
require_text "accepted index survives truncation and recovery" "latest-snapshot bootstrap"
require_text "accepted index carries no plaintext" "plaintext key or payload"
require_text "rebase consults index after tail expiry" "accepted envelope has expired from the"
require_text "same-age pre-acceptance failure is preserved" "pre-acceptance failure at the same age has no index entry"
require_text "index stores an opaque idempotency tag" "idempotency_key_tag"
require_text "service checks both identities" 'looks up **both** `command_id`'
require_text "exact dual identity binding is required" "exact two-key binding"
require_text "idempotency tag collisions fork" "collision with a different command also fails closed"
require_text "rebase checks the idempotency tag" 'by both its `command_id` and'
require_text "new command id with reused tag forks" 'new `command_id` but the first command'
require_text "idempotency lookup has no plaintext" "retained lookup contains no plaintext key"
require_text "AAD binds the dual identity fields" 'object_id ‖ idempotency_key_tag ‖ immutable_metadata_commitment'
require_text "AAD has canonical null markers" "canonical null markers"
require_text "receiver recomputes the keyed tag" "receiver recomputes the keyed"
require_text "receiver recomputes the metadata commitment" 'immutable_metadata_commitment` from'
require_text "validation precedes mutation and reconciliation" "before any mutation"
require_text "clear tag and commitment tamper is covered" "tampered clear tag or commitment"
require_text "tamper retains ciphertext" "while retaining the ciphertext"
require_text "replay-owned memos are withheld" "replay-owned idempotency memo is deliberately absent"
require_text "replay rebuilds the memo target" "memo points at the rebuilt"
require_text "queued retry cannot silently apply" "QueuedForDisposition"
require_text "rebase validates an artifact closure before swap" "receipt/pin closure"
require_text "envelopes preserve replay identity" "complete original \`CommandMeta\`"
require_text "envelopes have an opaque canonical digest" "canonical_envelope_digest"
require_text "service is not the nonce authority" "it is **not** a confidentiality authority"
require_text "membership starts from a client-created genesis" "The first Sync device creates the genesis membership record locally"
require_text "AEAD keys are client-derived" "K_e,d,t = HKDF-SHA256-Expand"
require_text "counter rollback fails closed" "NonceStateLost"
require_text "service nonce tracking is only a backstop" "not the security source"
require_text "artifact addresses are opaque and epoch scoped" "sync_artifact_id_e"
require_text "artifact transport carries portable bytes" 'canonical `SyncArtifactTransportV1` object'
require_text "receivers derive their own storage identity" "StorageId_B = HMAC-SHA256"
require_text "receivers keep local materialization state" "device-local mapping from"
require_text "received refs preserve the canonical service address" "preserves the origin's"
require_text "attachment crypto fields stay local" "class-1/1b schema is therefore split"
require_text "materialization failure is typed" "ArtifactMaterializationFailed"
require_text "artifact publication is upload first" "Publication is upload-before-reference"
require_text "snapshot truncation needs a durable closure" "closure pin are durable in one CAS transaction"
require_text "artifact cache loss has a typed recovery path" "ArtifactRebootstrapRequired"
require_text "artifact privacy covers epoch rotation" "A fresh epoch therefore re-addresses and re-seals every"
require_text "fact-sheet invariant stays intact" "The local app needs no DohFlow server and no DohFlow account."

echo "PASS — $CASES ADR 0074 contract assertions, 0 failures"
