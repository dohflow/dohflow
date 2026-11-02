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
[ "$(jq -r '.limits.max_snapshot_container_bytes' "$JSON")" = "1073742104" ] \
  && pass "snapshot container bound is 2^30 + 280 (246-byte snapshot header)" || fail "snapshot container bound changed"

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
need "genesis retry returns 200" 'unless it is the exact retry'
need "only a different genesis is VaultExists" 'only a *different* genesis for the vault is'
need "parsing precedes the reason-dependent checks" 'Parsing comes first because the remaining checks branch'
need "push failures name the offending envelope" 'name the first offending envelope'
need "ArtifactMissing lists the missing ids" 'every id of that envelope'"'"'s sidecar that lacks a durable receipt'
need "installEpoch ArtifactMissing lists the closure ids" 'the closure ids that lack a durable receipt'
need "pre-genesis uploads bind to one account" 'MUST come from the same account'
need "a snapshotless grant has no head check" 'has no head check'
need "forged grant may surface as either code" 'a conformance test accepts either'
need "compatible-with naming rule" '"Compatible with DohFlow"'
need "no official-service naming without agreement" 'MUST NOT present itself as the official DohFlow Sync'

case_start "the void additions (personal-cfo-g3m.33) stay stated"
need "appendVoid keeps every distinct void" 'Two voids with different bodies are two entries'
need "an exact retry is the only dedupe" 'The server dedupes only an exact retry'
need "a void takes the next seq" 'It takes the next `seq`'
need "the server never says a position is voided" 'it never says that a position *is* voided'
need "signer is checked by verify_void" 'This is the check `verify_void` of `sync-core` makes'
need "void claims are claims, not verdicts" 'A claim is a claim, not a verdict'
need "voids and envelopes share one sequence" 'Envelopes and voids share one sequence of `seq` values'
need "a client applies Addendum D and E to voids" 'it applies ADR 0074 Addendum D and Addendum E'
need "halt report is the device's own" 'a device reports only its own state'
need "the halt gate is outside sections 5 and 6" 'it is not a step of section 5 or 6'
need "appendVoid works for a halted device" '`appendVoid` and `reportHalt` must work for a halted device'
need "void evidence is retained" 'A server does not drop the envelope at a position a void'
need "additions are recorded with their date" '| 2026-11-02 | `personal-cfo-g3m.33` |'
GATED="installEpoch stageSnapshot publishSnapshot beginSnapshotUpload putSnapshotPart completeSnapshotUpload"
refused="$(jq -r --arg ops "$GATED" '($ops | split(" ")) as $g | [.operations[] | select(.op as $o | $g | index($o)) | select(.errors | index("ReplicaHalted"))] | length' "$JSON")"
[ "$refused" = "6" ] && pass "the three operations that stage a snapshot or install an epoch, and the three that stage in parts, can all return ReplicaHalted" || fail "ReplicaHalted is not on all six gated operations"
other="$(jq -r '[.operations[] | select(.errors | index("ReplicaHalted"))] | length' "$JSON")"
[ "$other" = "6" ] && pass "no other operation returns ReplicaHalted" || fail "ReplicaHalted appears on an operation that the halt gate does not cover"
abort_gated="$(jq -r '[.operations[] | select(.op == "abortSnapshotUpload") | select(.errors | index("ReplicaHalted"))] | length' "$JSON")"
[ "$abort_gated" = "0" ] && pass "abortSnapshotUpload is not gated, so a halted device can clean up" || fail "abortSnapshotUpload must not return ReplicaHalted"
SEC114="$(awk '/^### 11.4 /{on=1;next} /^### /{on=0} on' "$DOC")"
answers="$(grep -c '^[1-5]\. \*\*.*:\*\* not changed' <<<"$SEC114")"
[ "$answers" = "9" ] && pass "section 11.4 answers four limits of 11.1 for g3m.33 and all five for g3m.36 with 'not changed'" || fail "section 11.4 does not answer 4 + 5 limits of 11.1 with 'not changed' (found $answers)"
item2="$(grep -c '^2\. \*\*An existing member.s meaning:\*\*' <<<"$SEC114")"
[ "$item2" = "2" ] && pass "section 11.4 answers item 2 of 11.1 for both beads" || fail "section 11.4 lost its answer to item 2 of 11.1 for one of the beads (found $item2)"
need "item 2 names the pre-release amendment" 'that is the pre-release amendment of the first row, not an addition'
need "11.1 states the pre-release rule" 'Protocol version 1 is pre-release until the first deploy of a conforming Sync server to the operator'"'"'s production environment'
need "staging does not count as release" 'staging does not count'
need "a v1 client must process voids on pulls" '**A version-1 client MUST process `voids`, and `void_claims` on push results'
need "a v1 client must process void_claims" 'A version-1 client MUST process `void_claims` where it appears'
need "4.7 promises a log object per page" 'holds at least one log object (an envelope, an epoch entry or a void, section 4.11) whenever any exists past `after_seq`'
need "7.2 promises a log object per page" 'a page still holds at least one log object (an envelope, an epoch entry or a void)'
need "11.4 lists the page rule with the amendment" 'a page holds at least one log object (an envelope, an epoch entry or a void), not at least one entry'
if grep -Eq 'holds at least one entry' "$FLAT"; then
  fail "a page is still promised to hold 'at least one entry', which a void-only page contradicts"
else
  pass "no wording promises a page at least one entry"
fi
need "parts: a server accepts every snapshot through one of two paths" 'A server MUST accept every well-formed snapshot within section 7.1 through `stageSnapshot` or through these'
need "parts: complete checks the whole container before it counts as staged" '**Nothing counts as staged until the whole container has been checked.**'
need "parts: a digest mismatch discards the upload" 'and the server **discards the upload**'
need "parts: 4.13 says an incomplete upload expires" 'An upload that is not completed within `snapshot_upload_expiry_seconds` is aborted by the server (section 7.3)'
need "parts: 7.3 says an incomplete upload expires" 'An upload begun with `beginSnapshotUpload` that is not completed within `snapshot_upload_expiry_seconds` is aborted by the server'
need "parts: 2.5 names the discard as an exception to the failure rule" 'A failure mutates nothing, with two stated exceptions'
need "parts: 2.5 says which upload is discarded" 'whose assembled container does not match what was declared discards the upload it was completing'
need "parts: 11.4 lists the discard exception" 'the second exception to the failure rule of 2.5, the discard of an upload whose container does not match what was declared'
need "parts: 2.1 gives the part number its form" '`{part_number}` is an unsigned decimal in the syntax of section 2.2'
need "parts: the part size floor" 'at least **5,242,880** bytes'
need "parts: the part count ceiling" 'is at most **10,000**'
need "parts: the 7.1 clarification is a listed pre-release amendment" '| 2026-11-03 | `personal-cfo-g3m.36` | pre-release amendment |'
need "an incomplete page is a failed pull" 'treats it as a failed pull, never as a gap to skip'
need "the response-member rule excepts voids" 'except the members this document defines'
need "the pre-release changes are listed with a basis" '| pre-release amendment |'
VOID_VECTORS="$REPO_ROOT/docs/architecture/vectors/sync-service-void-v1.json"
if [ -f "$VOID_VECTORS" ] && [ "$(jq -r '.vectors_version' "$VOID_VECTORS")" = "1" ] && grep -Fq 'sync-service-void-v1.json' "$DOC"; then
  pass "the void conformance vectors exist and the document names them"
else
  fail "the void conformance vectors are missing, unversioned, or not named by section 11.2"
fi
PARTS_VECTORS="$REPO_ROOT/docs/architecture/vectors/sync-service-snapshot-parts-v1.json"
if [ -f "$PARTS_VECTORS" ] && [ "$(jq -r '.vectors_version' "$PARTS_VECTORS")" = "1" ] && grep -Fq 'sync-service-snapshot-parts-v1.json' "$DOC"; then
  pass "the snapshot-parts conformance vectors exist and the document names them"
else
  fail "the snapshot-parts conformance vectors are missing, unversioned, or not named by section 11.2"
fi

# The service dedupes only an identical void (spec-version 24): no passage may still say it dedupes a second void for the same position.
ADR="$REPO_ROOT/docs/adr/0074-dohflow-sync-architecture.md"
FLAT_FORMAT="$(tr '\n' ' ' < "$FORMAT" | tr -s ' ')"
if grep -Eq 'second void for (the )?(exact )?same' <<<"$FLAT_FORMAT"; then
  fail "vault-format.md still says the service dedupes a second void for the same position"
else
  pass "vault-format.md keeps every distinct void"
fi
FLAT_ADR="$(tr '\n' ' ' < "$ADR" | tr -s ' ')"
if grep -Eq 'second void for the exact same position\.? Otherwise|or a second void for the exact same position' <<<"$FLAT_ADR"; then
  fail "ADR 0074 Addendum D still says the service dedupes a second void for the same position"
else
  pass "ADR 0074 Addendum D keeps every distinct void"
fi

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
