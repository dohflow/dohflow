#!/usr/bin/env bash
#
# Contract test for the public Sync service protocol (personal-cfo-g3m.13).
#
# docs/architecture/sync-service-protocol.md is the human-readable contract a
# compatible Sync server and the DohFlow client both build against.
# docs/architecture/sync-service-protocol.json is the machine-readable list they
# share. This test pins the protocol version and fails when the document's
# operation, error and limit tables drift from that list, the same way
# crates/vault-crypto/tests/sync_wire_vectors.rs keeps vault-format.md honest.
# It is a document contract test, not a behavioral suite: that suite is owned by
# the server bead (personal-cfo-v98vd) and runs against the simulator as well.
#
# CI-safe: no network, no bead database, no credentials. Needs bash, awk, jq.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
DOC="$REPO_ROOT/docs/architecture/sync-service-protocol.md"
JSON="$REPO_ROOT/docs/architecture/sync-service-protocol.json"
FORMAT="$REPO_ROOT/docs/architecture/vault-format.md"

for f in "$DOC" "$JSON" "$FORMAT"; do
  [ -f "$f" ] || { echo "missing: $f" >&2; exit 1; }
done
command -v jq >/dev/null || { echo "jq is required" >&2; exit 1; }

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

FAILURES=0
CASES=0
pass() { echo "    ok   — $*"; }
fail() { echo "    FAIL — $*" >&2; FAILURES=$((FAILURES + 1)); }
case_start() { CASES=$((CASES + 1)); echo; echo "[$CASES] $*"; }

# table_rows <file> <name>: the data rows of the <!-- table:name --> block, one
# row per line, cells separated by TAB, backticks and padding removed.
table_rows() {
  awk -v name="$2" '
    $0 == "<!-- table:" name " -->" { on = 1; next }
    $0 == "<!-- /table:" name " -->" { on = 0 }
    on && /^\|/ {
      n++
      if (n <= 2) next               # header row and the --- separator
      line = $0
      sub(/^\|[ \t]*/, "", line); sub(/[ \t]*\|[ \t]*$/, "", line)
      gsub(/`/, "", line)
      m = split(line, c, /[ \t]*\|[ \t]*/)
      out = c[1]
      for (i = 2; i <= m; i++) out = out "\t" c[i]
      print out
    }' "$1"
}

# Normalized lines from the document and from a JSON file, for comparison.
doc_ops()    { table_rows "$1" operations | awk -F'\t' '{ e = $6; gsub(/, /, ",", e); if (e == "-") e = ""; print $1 "|" $2 "|" $3 "|" $4 "|" $5 "|" e }' | sort; }
json_ops()   { jq -r '.operations[] | [.op, .method, .path, .auth, .success, (.errors | join(","))] | join("|")' "$1" | sort; }
doc_errors() { table_rows "$1" errors | awk -F'\t' '{ print $1 "|" $2 }' | sort; }
json_errors(){ jq -r '.errors[] | "\(.code)|\(.status)"' "$1" | sort; }
doc_limits() { table_rows "$1" limits | awk -F'\t' '{ print $1 "|" $2 }' | sort; }
json_limits(){ jq -r '.limits | to_entries[] | "\(.key)|\(.value)"' "$1" | sort; }

# tables_agree <doc> <json>: 0 when all three tables match.
tables_agree() {
  [ "$(doc_ops "$1")" = "$(json_ops "$2")" ] \
    && [ "$(doc_errors "$1")" = "$(json_errors "$2")" ] \
    && [ "$(doc_limits "$1")" = "$(json_limits "$2")" ]
}

case_start "the protocol version is pinned"
if grep -Fq '<!-- sync-service-protocol: protocol-version=1 -->' "$DOC"; then
  pass "the document carries protocol-version=1"
else
  fail "the document lost its protocol-version=1 marker"
fi
if [ "$(jq -r '.protocol_version' "$JSON")" = "1" ] && [ "$(jq -r '.protocol' "$JSON")" = "dohflow-sync-service" ]; then
  pass "the machine-readable list is protocol 1 of dohflow-sync-service"
else
  fail "the machine-readable list does not say protocol 1"
fi
if grep -Fq '| `protocol_version` | `1` |' "$FORMAT"; then
  pass "vault-format.md C.1.3 registers protocol_version 1"
else
  fail "vault-format.md no longer registers protocol_version 1 (this protocol is bound to it)"
fi
if grep -Fq 'DohFlow-Protocol: 1' "$DOC"; then
  pass "the request header names the version"
else
  fail "the DohFlow-Protocol request header example is missing"
fi

case_start "the operation table equals the machine-readable list"
if [ "$(doc_ops "$DOC")" = "$(json_ops "$JSON")" ]; then
  pass "$(json_ops "$JSON" | wc -l | tr -d ' ') operations match (id, method, path, auth, success, errors)"
else
  fail "operation tables differ"
  diff <(doc_ops "$DOC") <(json_ops "$JSON") >&2 || true
fi
dups="$(jq -r '[.operations[] | .method + " " + .path] | group_by(.) | map(select(length > 1)) | length' "$JSON")"
[ "$dups" = "0" ] && pass "no two operations share a method and path" || fail "duplicate method and path"
dupids="$(jq -r '[.operations[].op] | group_by(.) | map(select(length > 1)) | length' "$JSON")"
[ "$dupids" = "0" ] && pass "operation ids are unique" || fail "duplicate operation id"

case_start "the error table equals the machine-readable list"
if [ "$(doc_errors "$DOC")" = "$(json_errors "$JSON")" ]; then
  pass "$(json_errors "$JSON" | wc -l | tr -d ' ') error codes match (code, status)"
else
  fail "error tables differ"
  diff <(doc_errors "$DOC") <(json_errors "$JSON") >&2 || true
fi
dupcodes="$(jq -r '[.errors[].code] | group_by(.) | map(select(length > 1)) | length' "$JSON")"
[ "$dupcodes" = "0" ] && pass "error codes are unique" || fail "duplicate error code"
unknown="$(jq -r '(.errors | map(.code)) as $reg | [(.operations[].errors[]), .common_errors[]] | map(select(. as $c | $reg | index($c) | not)) | unique | join(",")' "$JSON")"
[ -z "$unknown" ] && pass "every operation and common error is in the registry" || fail "unregistered error codes: $unknown"
unused="$(jq -r '([(.operations[].errors[]), .common_errors[]] | unique) as $used | .errors | map(.code) | map(select(. as $c | $used | index($c) | not)) | join(",")' "$JSON")"
[ -z "$unused" ] && pass "every registered error is returned by some operation" || fail "registered but never returned: $unused"
badstatus="$(jq -r '[.errors[] | select(.status < 400 or .status > 599)] | length' "$JSON")"
[ "$badstatus" = "0" ] && pass "every error status is 4xx or 5xx" || fail "an error status is outside 400-599"

case_start "the limits table equals the machine-readable list"
if [ "$(doc_limits "$DOC")" = "$(json_limits "$JSON")" ]; then
  pass "$(json_limits "$JSON" | wc -l | tr -d ' ') limits match"
else
  fail "limit tables differ"
  diff <(doc_limits "$DOC") <(json_limits "$JSON") >&2 || true
fi
# These bounds are derived from the field maxima of vault-format.md C.1.1-C.9;
# pin the two the rest of the protocol leans on.
[ "$(jq -r '.limits.max_envelope_container_bytes' "$JSON")" = "16909058" ] \
  && pass "envelope container bound is 16909058" || fail "envelope container bound changed"
[ "$(jq -r '.limits.max_snapshot_container_bytes' "$JSON")" = "1073742168" ] \
  && pass "snapshot container bound is 2^30 + 344" || fail "snapshot container bound changed"

case_start "the comparison detects drift (negative checks)"
cp "$JSON" "$TMP/drop-error.json"
jq 'del(.errors[] | select(.code == "HeadMismatch"))' "$JSON" > "$TMP/drop-error.json"
tables_agree "$DOC" "$TMP/drop-error.json" && fail "a dropped error code was not detected" || pass "a dropped error code is detected"
jq '.operations[0].path = "/information"' "$JSON" > "$TMP/path.json"
tables_agree "$DOC" "$TMP/path.json" && fail "a changed path was not detected" || pass "a changed path is detected"
jq '.limits.claim_skew_seconds = 301' "$JSON" > "$TMP/limit.json"
tables_agree "$DOC" "$TMP/limit.json" && fail "a changed limit was not detected" || pass "a changed limit is detected"
jq '(.operations[] | select(.op == "pushEnvelopes") | .errors) += ["Unlisted"]' "$JSON" > "$TMP/extra.json"
tables_agree "$DOC" "$TMP/extra.json" && fail "an extra operation error was not detected" || pass "an extra operation error is detected"
tables_agree "$DOC" "$JSON" && pass "the unmodified pair still agrees" || fail "the unmodified pair disagrees"

case_start "the required rules are stated"
# The document hard-wraps its prose, so search a copy with line breaks folded
# into single spaces (needles below are written as one unwrapped line).
FLAT="$TMP/doc.flat"
tr '\n' ' ' < "$DOC" | tr -s ' ' > "$FLAT"
need() {
  if grep -Fq -- "$2" "$FLAT"; then pass "$1"; else fail "$1 (missing: $2)"; fi
}
need "seq starts at 1" '`seq` starts at **1**'
need "seq is never 0" 'A server MUST NOT assign `seq = 0`'
need "genesis epoch has position 0" 'The genesis epoch has position `0`'
need "add racing a cutover rule" 'An add racing a cutover is rejected and retried'
need "stale membership prev_id is compare-and-swapped" 'only if its `prev_id` is still the newest installed membership'
need "index is looked up before old-epoch rejection" 'the lookup runs **before**'
need "index is recovery-durable" 'it MUST survive tail truncation, snapshot bootstrap and restart'
need "exact retry returns the original seq" 'its `seq` and `result_ref` are those of the original acceptance'
need "the server never chooses a nonce" 'never chooses, allocates or assigns a nonce domain or a counter range'
need "ProtocolFork rejects the whole request" 'reject the **whole request** with'
need "genesis for a known vault is VaultExists" 'that already has one is `VaultExists`'
need "lapsed credentials never disable local features" 'A server response never instructs a client to disable'
need "the never-sees section exists" '## 9. What a server never sees'
for item in 'the vault password' 'the Sync epoch root `K_e`' 'the tag secret `S`' 'the origin `StorageId`' 'the plaintext idempotency key' 'attachment bytes'; do
  need "never-sees lists: $item" "$item"
done
need "compatible-with naming rule" '"Compatible with DohFlow"'
need "no official-service naming without agreement" 'MUST NOT present itself as the official DohFlow Sync'

case_start "references resolve"
refs="$(grep -oE 'C\.[0-9]+' "$DOC" | sort -u)"
missing=""
for r in $refs; do
  if ! grep -Eq "^## ${r//./\\.} " "$FORMAT"; then missing="$missing $r"; fi
done
[ -z "$missing" ] && pass "every vault-format.md C.n reference names an existing section ($(echo "$refs" | tr '\n' ' '))" || fail "dangling vault-format.md references:$missing"
badlinks=""
while IFS= read -r link; do
  target="${link%%#*}"
  case "$target" in http*|"") continue ;; esac
  [ -e "$REPO_ROOT/docs/architecture/$target" ] || badlinks="$badlinks $target"
done < <(grep -oE '\]\([^)]+\)' "$DOC" | sed -E 's/^\]\(//; s/\)$//')
[ -z "$badlinks" ] && pass "every relative link in the document resolves" || fail "broken relative links:$badlinks"
grep -Fq 'sync-service-protocol.json' "$DOC" && pass "the document points at the machine-readable list" || fail "the document does not point at the list"

case_start "the document stays public-tier"
if grep -nE '\$[0-9]|USD|per month|/month|subscriber|price|pricing' "$DOC" >&2; then
  fail "price or business wording found in the public protocol document"
else
  pass "no price, cost or plan wording"
fi

echo
if [ "$FAILURES" -eq 0 ]; then
  echo "PASS — $CASES cases, 0 failures"
  exit 0
fi
echo "FAIL — $CASES cases, $FAILURES failed assertions" >&2
exit 1
