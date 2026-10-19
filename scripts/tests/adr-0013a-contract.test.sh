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
require_text "provider-id domain separation is proved for every input" "$ADR" "So no provider message is a valid section 1 message,"
require_text "provider-id and command-id spaces differ by version nibble" "$ADR" "provider-derived ids are UUID version 8 and section 1 ids are version 7"
require_text "adapter key function is part of the identity" "$ADR" "**each adapter's"
require_text "provider id reuse is a stated residual" "$ADR" "A provider that reuses one transaction id for two different transactions"
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
# The disjointness argument, re-checked on the byte arithmetic itself: section 1 would read
# offset 16 as tag_len; the tag it then claims must be entirely printable.
assert msg[16] == label[15] == ord("r") and msg[16] == 114
assert 17 <= 43 <= 17 + msg[16] - 1          # offset 43 would lie inside that tag
for n in range(1, 1025):                      # every allowed provider_account_key length
    high = (n >> 8) & 0xFF                    # offset 43 is the high byte of u16_be(len)
    assert high <= 0x04 and not (0x21 <= high <= 0x7E), n
assert msg[43] == (len(acct) >> 8)
PY
echo "    ok   — provider-id vector re-derived independently"

# Section 10 (personal-cfo-g3m.14): the 2026-10-19 correction note.
require_text "correction note is dated and named" "$ADR" "#### 10. Correction note (2026-10-19, \`personal-cfo-g3m.14\`): payload-only ordinals, singleton ids, mint order and the clock scope"
require_text "(a) ordinals depend only on payload and metadata" "$ADR" "**(a) Ordinal consumption depends only on the payload and the metadata.**"
require_text "(b) singletons get a content-derived id" "$ADR" "**(b) Create-if-missing singletons get a content-derived id.**"
require_text "(b) singleton domain label is fixed" "$ADR" 'label    = ASCII "dohflow/system-account-id/v1"'
require_text "(b) singleton packing uses a zero time field" "$ADR" "packing  = uuid::Builder::from_unix_timestamp_millis(0, &counter_random_bytes)"
require_text "(b) cross-references the provider-transaction exception" "$ADR" "the provider-transaction exception of section 9"
require_text "(b) validates before deriving" "$ADR" "the currency is exactly three ASCII uppercase letters, and the role"
require_text "(b) separates from section 9 by first byte and version nibble" "$ADR" "nibble (7 here, 8 there)"
require_text "(b) a singleton timestamp is not a creation time" "$ADR" "**not a creation time**"
require_text "(d) the helper list names the inbox helpers" "$ADR" "`record_dismiss` and"
require_text "(c) mint order is deterministic" "$ADR" "**(c) Mint order is deterministic.**"
require_text "(c) no unordered collection on a mint path" "$ADR" "no iteration over a \`HashMap\`, a \`HashSet\` or any other unordered"
require_text "(c) a total ORDER BY feeds every mint loop" "$ADR" "total \`ORDER BY\`"
require_text "(d) the clock rule covers everything reachable" "$ADR" "**(d) The no-clock rule covers everything reachable from \`apply\`.**"
require_text "(d) class-2 rebuilds are in scope" "$ADR" "**class-2 rebuilds**"
require_text "section 2 carries a dated pointer" "$ADR" "*(Scope corrected 2026-10-19: the rule covers every clock read reachable from"
require_text "section 3 carries a dated pointer" "$ADR" "*(Corrected 2026-10-19: \`ensure_system_ledger_account\`, and any other"
require_text "section 4 carries a dated pointer" "$ADR" "*(Corrected 2026-10-19: the payload and the metadata alone determine the"
require_text "section 6 carries a dated pointer" "$ADR" "*(Superseded 2026-10-19: a singleton's id is content-derived and outside the"
require_text "the accepted section 4 sentence is unchanged" "$ADR" "payload, metadata, and base determine the sequence; a rollback-then-replay"

# The singleton vectors are re-derived here, independently of the document's text, with a
# packer that is first checked against the document's own vector A.
CASES=$((CASES + 1))
python3 - "$ADR" <<'PY'
import hashlib, hmac, re, struct, sys, uuid
text = open(sys.argv[1]).read()
def pack_v7(ms, ordinal, high62):
    c = bytearray(10)
    c[0] = (ordinal >> 8) & 0x0F
    c[1] = ordinal & 0xFF
    c[2:10] = high62.to_bytes(8, "big")
    b = bytearray(16)
    b[0:6] = ms.to_bytes(6, "big")
    b[6] = 0x70 | (c[0] & 0x0F)
    b[7] = c[1]
    b[8] = (c[2] & 0x3F) | 0x80
    b[9:16] = c[3:10]
    return bytes(c), uuid.UUID(bytes=bytes(b))
# 1. the packer reproduces section 1's vector A exactly
a = text.split("vector A", 1)[1].split("vector B", 1)[0]
def f(name, chunk):
    m = re.search(r"^" + name + r"\s*=\s*(\S+)", chunk, re.M)
    assert m, name
    return m.group(1)
key = bytes.fromhex(f("id_key", a))
msg = bytes.fromhex(f("message_hex", a))
dig = hmac.new(key, msg, hashlib.sha256).digest()
counter, got = pack_v7(int(f("issued_at_unix_ms", a)), int(f("ordinal", a)), int.from_bytes(dig[:8], "big") >> 2)
assert counter.hex() == f("counter_random_bytes", a) and str(got) == f("uuid", a), "packer vs vector A"
# 2. the section 10 singleton vectors
sec = text.split("#### 10. Correction note", 1)[1]
block = sec.split("Known-answer vectors (normative; hexadecimal is lowercase; `id_key` is vector A's key", 1)[1]
block = block.split("```text", 1)[1].split("```", 1)[0]
label = b"dohflow/system-account-id/v1"
assert len(label) == 28
chunks = block.split("\n\n")
assert len(chunks) == 2, len(chunks)
for chunk in chunks:
    role, cur = f("role", chunk).encode(), f("currency", chunk).encode()
    assert role in (b"opening_balance_equity", b"unmatched_income", b"unmatched_expense")
    assert re.fullmatch(rb"[A-Z]{3}", cur)
    m = bytes([len(label)]) + label + bytes([len(role)]) + role + cur
    d = hmac.new(key, m, hashlib.sha256).digest()
    counter, u = pack_v7(0, 0, int.from_bytes(d[:8], "big") >> 2)
    assert m.hex() == f("message_hex", chunk), "message_hex"
    assert d.hex() == f("hmac_sha256", chunk), "hmac_sha256"
    assert counter.hex() == f("counter_random_bytes", chunk), "counter_random_bytes"
    assert str(u) == f("uuid", chunk), "uuid"
    assert u.version == 7 and str(u).startswith("00000000-0000-7000-")
    # not a valid section 1 message: its last two bytes cannot be an ordinal below 4096
    assert struct.unpack(">H", m[-2:])[0] >= 4096
    # and its first byte differs from the provider derivation's (26)
    assert m[0] == 0x1C != 0x1A
    # ... and the id is a version 7 layout, whereas the provider-derived ids are version 8
    assert u.version == 7
PY
echo "    ok   — singleton vectors re-derived independently (packer checked against vector A)"

# Section 11 (personal-cfo-g3m.15): merge-import derived identifiers.
require_text "section 11 is dated and named" "$ADR" "#### 11. Exception: merge-import derived identifiers (2026-10-19, \`personal-cfo-g3m.15\`)"
require_text "merge ids are not command-scoped" "$ADR" "Like"
require_text "merge label is fixed" "$ADR" 'label    = ASCII "dohflow/merge-import-derived-id/v1"'
require_text "merge message layout" "$ADR" "|| source_vault_id.bytes"
require_text "merge entity tag grammar" "$ADR" "|| u8(len(entity_tag))  || entity_tag"
require_text "merge row key is the C.1.5 key" "$ADR" "|| u16_be(len(source_row_key)) || source_row_key"
require_text "merge ids use the household id_key" "$ADR" "# the HOUSEHOLD vault's id_key (section 1)"
require_text "merge ids are UUID version 8" "$ADR" "The id is a UUID version 8: it carries no time"
require_text "merge domain separation names the offsets" "$ADR" "`tag_len` would be 109 and the tag would span offsets 17 to 125"
require_text "merge first byte differs from sections 9 and 10" "$ADR" "the first byte differs (34, \`0x22\`,"

CASES=$((CASES + 1))
python3 -I - "$ADR" <<'PY'
import hashlib, hmac, re, sys, uuid
text = open(sys.argv[1]).read()
sec = text.split("#### 11. Exception: merge-import derived identifiers", 1)[1].split("### Rejected alternatives", 1)[0]
block = sec.split("Known-answer vector", 1)[1].split("```text", 1)[1].split("```", 1)[0]
def f(name, chunk):
    m = re.search(r"^" + name + r"\s*=\s*(\S+)", chunk, re.M)
    assert m, name
    return m.group(1)
parts = block.split("\n\n")
assert len(parts) == 2, len(parts)
id_key = bytes.fromhex(f("id_key", parts[0]))
label = b"dohflow/merge-import-derived-id/v1"
assert len(label) == 34 and label[15:16] == b"m"
def derive(src_vault, tag, row_key):
    assert re.fullmatch(rb"[a-z0-9_]{1,32}", tag) and 1 <= len(row_key) <= 256
    m = bytes([len(label)]) + label + src_vault + bytes([len(tag)]) + tag + len(row_key).to_bytes(2, "big") + row_key
    d = hmac.new(id_key, m, hashlib.sha256).digest()
    b = bytearray(d[:16]); b[6] = (b[6] & 0x0F) | 0x80; b[8] = (b[8] & 0x3F) | 0x80
    return m, d, uuid.UUID(bytes=bytes(b))
src = uuid.UUID(f("source_vault_id", parts[0])).bytes
row = bytes.fromhex(f("source_row_key", parts[0]))
assert row[0] == 3 and len(row) == 17                     # a uuid key in the C.1.5 encoding
for chunk, tag in ((parts[0], b"account"), (parts[1], b"category")):
    if chunk is parts[1]:
        tag = f("entity_tag", chunk).encode()
    else:
        assert f("entity_tag", chunk).encode() == tag
    m, d, u = derive(src, tag, row)
    if chunk is parts[0]:
        assert m.hex() == f("message_hex", chunk), "message_hex"
        assert d.hex() == f("hmac_sha256", chunk), "hmac_sha256"
    assert str(u) == f("uuid", chunk), "uuid"
    assert u.version == 8
# Domain separation, re-checked on the bytes: against section 1 (offset 16 is label[15]) ...
m, _, _ = derive(src, b"account", row)
assert m[16] == label[15] == 0x6D and m[16] == 109
for n in range(1, 33):
    offset = 1 + 34 + 16 + 1 + n                       # the u16 high byte of the row-key length
    assert 17 <= offset <= 16 + 109 and offset < 128
    for key_len in (1, 17, 255, 256):
        probe = bytes([34]) + label + src + bytes([n]) + b"t" * n + key_len.to_bytes(2, "big") + b"k" * key_len
        high = probe[offset]
        assert high in (0, 1) and not (0x21 <= high <= 0x7E)
# ... and against sections 9 and 10 (first byte).
assert m[0] == 34 and 34 not in (26, 28)
PY
echo "    ok   — merge-import vectors re-derived independently; domain separation re-checked on the bytes"

echo "PASS — $CASES ADR 0013-A contract assertions, 0 failures"
