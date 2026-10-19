#!/usr/bin/env bash
#
# Contract regression test for ADR 0013 Addendum A (personal-cfo-1df8d).
# This guards the deterministic-ID decisions that SYNC-1 and SYNC-1a consume;
# it is intentionally a document test, not a substitute for their Rust tests.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
ADR="$REPO_ROOT/docs/adr/0013-id-strategy.md"
SYNC="$REPO_ROOT/docs/adr/0074-dohflow-sync-architecture.md"

for file in "$ADR" "$SYNC"; do
  if [ ! -f "$file" ]; then
    echo "FAIL — missing document: $file" >&2
    exit 1
  fi
done

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

require_text "dated addendum names the bead" "$ADR" "Addendum A (2026-09-20, personal-cfo-1df8d): deterministic derived identifiers for apply-minted rows"
require_text "issued_at supplies unix milliseconds" "$ADR" "unix_ms = CommandMeta.issued_at as Unix milliseconds"
require_text "ordinal occupies rand_a" "$ADR" "rand_a  = ordinal (12 bits, 0..=4095)"
require_text "HMAC occupies rand_b" "$ADR" "HMAC-SHA256(id_key, command_id || tag || ordinal)"
require_text "tag prefix width is fixed" "$ADR" "u8_be(tag_len)"
require_text "tag bounds are fixed" "$ADR" "0x21..=0x7e"
require_text "ordinal encoding is fixed" "$ADR" "u16_be(ordinal)"
require_text "HMAC slice is right-shifted" "$ADR" "u64_be(digest[0..8]) >> 2"
require_text "builder bytes are packed explicitly" "$ADR" "counter_random_bytes[2..10] = rand_b.to_be_bytes()"
require_text "builder variant masking is explicit" "$ADR" "masks the top two bits of byte 2"
require_text "constructor is fixed" "$ADR" "Builder::from_unix_timestamp_millis"
require_text "public key derivation is fixed" "$ADR" "dohflow/derived-id-key/v1"
require_text "genesis carries and validates the key" "$ADR" 'value beside `vault_id`; a receiver recomputes it'
require_text "vector A key is published" "$ADR" "f2f6ac71bc63e3df2f66370676ead3f3b6b677a87c124f190941659224b42149"
require_text "vector A packing is published" "$ADR" "counter_random_bytes  = 00002f4fc4ed34771c35"
require_text "vector A UUID is published" "$ADR" "018bcfe5-687b-7000-af4f-c4ed34771c35"
require_text "vector B key is published" "$ADR" "b33777244109db333d33cacff0f92a7d03e93497cc2d460e8f2b2db3548891b8"
require_text "vector B packing is published" "$ADR" "counter_random_bytes  = 00073e802167f48d7b1b"
require_text "vector B UUID is published" "$ADR" "018bcfe5-6be7-7007-be80-2167f48d7b1b"
require_text "future core-ids test consumes vectors" "$ADR" 'future SYNC-1 `core-ids` test must consume these same vectors'
require_text "apply is clock-free" "$ADR" "never reads the wall clock"
require_text "all 21 clock sites are in scope" "$ADR" 'the 21 `Utc::now()` sites'
require_text "metadata inventory has six production families" "$ADR" "The six kernel-side metadata/dispatch sites"
require_text "caller IDs stay in payload" "$ADR" "RecordTransaction.transaction_id"
require_text "ContextV7 boundary is explicit" "$ADR" "therefore retired only for apply-minted IDs"
require_text "overflow is typed" "$ADR" "DerivedIdOrdinalOverflow { command_id, attempted: 4096 }"
require_text "SYNC-1a owns cap tests" "$ADR" "SYNC-1a must assert both"
require_text "inventory explains nine paths" "$ADR" "nine idempotent paths"
require_text "helper callsites are enumerated" "$ADR" 'apply/transactions.rs:60`, `:202`, `:360'
require_text "replicas never seed" "$ADR" "A replica never seeds — the genesis carries seeded"
require_text "seed twin is required" "$ADR" "Simulator case (ii), the seed-twin bootstrap"
require_text "pre-v2 compatibility is explicit" "$ADR" "Existing operation-log rows"
require_text "replay uses canonical row hashes" "$ADR" "hash of canonically ordered"
require_text "receipt alternative is rejected" "$ADR" "Apply receipts containing mint counts and IDs."
require_text "mapping-table alternative is rejected" "$ADR" "Random IDs plus a per-device mapping table."
require_text "central allocation alternative is rejected" "$ADR" "Central server allocation."
require_text "ULID alternative is rejected" "$ADR" "**ULID.** Rejected"
require_text "per-device counter alternative is rejected" "$ADR" "Sequential per-device counters in the UUID"
require_text "revisit triggers include the cap" "$ADR" "A command legitimately needs more than 4,096 derived IDs."
require_text "SYNC-1 points at accepted heading" "$SYNC" "ADR 0013 Addendum A — deterministic derived identifiers for apply-minted rows"

# Section 9 (personal-cfo-g3m.12): the content-derived provider-transaction id.
require_text "provider-id exception is a dated section" "$ADR" "#### 9. Exception: content-derived provider-transaction identifiers (2026-10-18, \`personal-cfo-g3m.12\`)"
require_text "provider-id domain label is fixed" "$ADR" 'label    = ASCII "dohflow/provider-txn-id/v1"'
require_text "provider-id is never command-scoped" "$ADR" "**never from \`issued_at\`, a \`command_id\` or an ordinal**"
require_text "provider-id layout is UUID version 8" "$ADR" "byte 6 = (byte 6 & 0x0f) | 0x80"
require_text "provider strings may not hold control characters" "$ADR" "containing no C0 control"
require_text "provider-id domain separation is structural" "$ADR" "No provider-derived message can therefore be"
require_text "provider-id consumes no ordinal" "$ADR" "no ordinal and is outside the 4,096-id cap."

# The provider-id vector is re-derived here, independently of the document's own text:
# a stale or mistyped vector fails this script rather than a future implementation.
CASES=$((CASES + 1))
python3 - "$ADR" <<'PY'
import hashlib, hmac, re, struct, sys, uuid
text = open(sys.argv[1]).read()
block = text.split("Known-answer vector (normative; hexadecimal is lowercase; `id_key` is vector A's", 1)[1]
block = block.split("```text", 1)[1].split("```", 1)[0]
first, second = block.split("provider_txn_id      = TXN-2026-10-17-0043", 1)
def field(name, chunk):
    m = re.search(r"^" + name + r"\s*=\s*(\S+)", chunk, re.M)
    assert m, name
    return m.group(1)
key = bytes.fromhex(field("id_key", first))
conn = uuid.UUID(field("connection_id", first))
acct = field("provider_account_key", first).encode()
label = b"dohflow/provider-txn-id/v1"
def derive(txn):
    txn = txn.encode()
    msg = bytes([len(label)]) + label + conn.bytes + struct.pack(">H", len(acct)) + acct + struct.pack(">H", len(txn)) + txn
    digest = hmac.new(key, msg, hashlib.sha256).digest()
    b = bytearray(digest[:16])
    b[6] = (b[6] & 0x0F) | 0x80
    b[8] = (b[8] & 0x3F) | 0x80
    return msg, digest, uuid.UUID(bytes=bytes(b))
msg, digest, got = derive(field("provider_txn_id", first))
assert msg.hex() == field("message_hex", first), "message_hex"
assert digest.hex() == field("hmac_sha256", first), "hmac_sha256"
assert str(got) == field("uuid", first), "uuid"
assert got.version == 8
_, _, got2 = derive("TXN-2026-10-17-0043")
assert str(got2) == field("uuid", second), "second uuid"
# The disjointness argument: the last two bytes of the message are never a valid ordinal.
assert struct.unpack(">H", msg[-2:])[0] >= 4096
PY
echo "    ok   — provider-id vector re-derived independently"

echo "PASS — $CASES ADR 0013-A contract assertions, 0 failures"
