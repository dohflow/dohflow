#!/usr/bin/env bash
# Document-contract checks for ADR 0066-A (personal-cfo-bc8iv).
# Future endpoint implementations own runtime enforcement. This script guards
# the approved written boundary, including the distinction between operational
# logs and long-lived ADR 0074 protocol records. CI runs scripts/tests/*.test.sh.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
ADR="$REPO_ROOT/docs/adr/0066-A-dohflow-endpoints-pinned-credential-not-unlock.md"
INDEX="$REPO_ROOT/docs/adr/README.md"
PARENT="$REPO_ROOT/docs/adr/0066-business-model-free-app-paid-services.md"
PROFILE="$REPO_ROOT/docs/agent/PROJECT_PROFILE.md"

CASES=0
require_text() {
  local label="$1" file="$2" needle="$3"
  CASES=$((CASES + 1))
  if grep -Fq -- "$needle" "$file"; then
    echo "    ok   — $label"
  else
    echo "    FAIL — $label (missing: $needle)" >&2
    exit 1
  fi
}

require_text "accepted public tier" "$ADR" '**Tier:** Public'
require_text "compiled pinned default" "$ADR" 'compiled-in default pinned to the'
require_text "override only in Settings" "$ADR" 'alternative egress in'
require_text "actual host shown in prompt" "$ADR" '> Host: **{host}**'
require_text "prompt discloses data" "$ADR" '> This host will receive: {protocol data and credential description}.'
require_text "device-local override" "$ADR" '`settings` `device.*`'
require_text "restore cannot set endpoint" "$ADR" 'excluded from vault backup'
require_text "untrusted input paths barred" "$ADR" 'deep link, URL scheme, file, and connector'
require_text "future TB3 sentence" "$ADR" 'The app has N pinned paths plus at most one user-configured egress per'
require_text "shipped TB3 remains accurate" "$ADR" 'remain unchanged in this docs-only change'
require_text "service cannot add a recipient" "$ADR" 'adding a recipient, choosing a nonce'
require_text "fresh-key revocation boundary" "$ADR" 'Fresh-key revocation protects future objects'
require_text "opaque artifact IDs" "$ADR" 'Opaque epoch-scoped artifact IDs/manifests'
require_text "service withholding risk" "$ADR" 'deny, delay, or withhold objects'
require_text "credential is not a local unlock" "$ADR" 'not a local code path or existing local data'
require_text "future entitlement check" "$ADR" 'scripts/check-client-service-entitlement-branches.sh'
require_text "future lapse test" "$ADR" 'lapse a service credential'
require_text "no automatic client diagnostics" "$ADR" 'no client diagnostic upload happens automatically'
require_text "operational logs only failures and lifecycle" "$ADR" 'Persist only failures and service lifecycle events'
require_text "no access history" "$ADR" 'no routine per-request'
require_text "minimal log allowlist" "$ADR" 'status/error category, and coarse duration bucket'
require_text "log exclusions include identifiers" "$ADR" 'Exclude vault/device/account IDs, sequence numbers, object sizes, ack'
require_text "log exclusions include raw values" "$ADR" 'exception messages, and stable hashes of any excluded identifier'
require_text "seven-day deletion" "$ADR" 'Delete automatically **within seven days**'
require_text "no archived log copies" "$ADR" 'no separate log archive'
require_text "designated operators only" "$ADR" 'designated reliability/security operators'
require_text "account deletion limitation" "$ADR" 'cannot be selectively'
require_text "hosting and CDN comply" "$ADR" 'Hosting and CDN access-log settings'
require_text "infrastructure noncompliance escalates" "$ADR" 'infrastructure that cannot meet the rule blocks deployment'
require_text "exceptions require approval" "$ADR" 'explicit approval **before** collection'
require_text "protocol retry index has no expiry" "$ADR" 'without automatic expiry'
require_text "seven-day rule excludes protocol roots" "$ADR" 'never expires protocol roots or accepted-envelope retry records'
require_text "BYO AI key" "$ADR" 'user-supplied API key (BYO key)'
require_text "extensions gate at acquisition" "$ADR" 'to **acquire** a'
require_text "ADR 0078 owns server publication" "$ADR" 'separate ADR 0078 decision'
require_text "test cleanup leaves protocol records" "$ADR" 'cleanup leaves ADR 0074 protocol roots and retry records intact'
require_text "index entry" "$INDEX" '0066-A-dohflow-endpoints-pinned-credential-not-unlock.md) | Public | Accepted'
require_text "ADR 0066 pointer" "$PARENT" 'See ADR 0066-A for pinned service endpoints'
require_text "profile pointer" "$PROFILE" '[ADR 0066-A](../adr/0066-A-dohflow-endpoints-pinned-credential-not-unlock.md)'

visible_row="$(grep -F '| Service-visible Sync protocol metadata |' "$ADR")"
retry_row="$(grep -F '| Service-visible accepted-envelope retry index |' "$ADR")"
test -n "$visible_row" && test -n "$retry_row"
require_text "envelope nonce header is visible" "$ADR" 'including `nonce_domain` and `invocation_counter`'
require_text "retry identities are opaque" "$ADR" '`canonical_envelope_digest`, immutable-metadata commitment'

check_visible_row() {
  local row="$1"
  case "$row" in
    *StorageId*|*plaintext*|*'content-key'*|*wrapped*|*'blob path'*|*materialization*) return 1 ;;
    *) return 0 ;;
  esac
}

CASES=$((CASES + 1))
if check_visible_row "$visible_row"; then
  echo '    ok   — no receiver-local crypto or plaintext artifact fields are service-visible'
else
  echo '    FAIL — service-visible row exposes a local or plaintext artifact field' >&2
  exit 1
fi

CASES=$((CASES + 1))
if check_visible_row "$retry_row"; then
  echo '    ok   — retry index contains no local crypto or plaintext artifact fields'
else
  echo '    FAIL — retry index exposes a local or plaintext artifact field' >&2
  exit 1
fi

for forbidden in 'StorageId' 'plaintext artifact bytes' 'plaintext digest' 'content-key nonce' 'wrapped key' 'blob path' 'materialization mapping'; do
  CASES=$((CASES + 1))
  if check_visible_row "| Service-visible Sync protocol metadata | $forbidden |"; then
    echo "    FAIL — negative fixture was accepted: $forbidden" >&2
    exit 1
  fi
  echo "    ok   — negative fixture rejects $forbidden"
done

echo "PASS — $CASES ADR 0066-A contract assertions, 0 failures"
