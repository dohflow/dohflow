#!/usr/bin/env bash
#
# Contract test for vault-format.md C.1.5, the class-1 row digest and primary-key encoding
# that the Sync write-set hash (C.4) commits to (personal-cfo-t2s4b part 5).
#
# It pins the normative text and re-derives every known-answer vector with an
# independent Python implementation of the grammar, so a wrong vector or a drifting
# rule fails here before any Rust code consumes it.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
SPEC="$REPO_ROOT/docs/architecture/vault-format.md"

if [ ! -f "$SPEC" ]; then
  echo "FAIL — missing document: $SPEC" >&2
  exit 1
fi

CASES=0
require_text() {
  local label="$1" needle="$2"
  CASES=$((CASES + 1))
  if grep -Fq -- "$needle" "$SPEC"; then
    echo "    ok   — $label"
  else
    echo "    FAIL — $label (missing: $needle)" >&2
    exit 1
  fi
}

require_text "section exists" "### C.1.5 Canonical row encoding and primary key"
require_text "one encoding for every use" "One encoding of a database row is used everywhere a row is hashed"
require_text "covers every table class" "It covers **every table of every class**"
require_text "tags follow the declared kind" "tagged by its column's **declared kind**"
require_text "affinity cannot change the tag" "affinity can never make two devices encode one value"
require_text "NULL tag" '| `0` | any kind | NULL | nothing |'
require_text "int tag" '| `1` | `int` | INTEGER | `i64` |'
require_text "text tag" '| `2` | `text` | TEXT | `text(2^24)`'
require_text "uuid tag" '| `3` | `uuid` | BLOB of exactly 16 bytes | the 16 raw bytes |'
require_text "bytes tag" '| `4` | `bytes` | BLOB | `bytes(2^24)` |'
require_text "a kind mismatch is an error" "is an error, never"
require_text "REAL is an error" "A REAL value has no tag and is an error"
require_text "manifest columns must match the table" "A column absent from the manifest, or a manifest column absent"
require_text "primary key is capped" "It is at most 256 bytes; a"
require_text "primary key is never truncated" "longer key is an error and is never truncated"
require_text "row digest label" 'label "dohflow/sync-row/v1"'
require_text "columns sort by name bytes" "**raw bytes of their names**"
require_text "every column is included" "Every column of the row is included."
require_text "absent digest label" 'label "dohflow/sync-row-absent/v1"'
require_text "deletion changes the hash" "so a deletion changes the write-set hash"
require_text "insert then delete is not in the set" "inserted and deleted inside the same command is"
require_text "final state once" "touched several times appears once, with its final state"
require_text "the command's own log row is excluded" "The \`operation_log\` row of the command itself is not in the write set"
require_text "digests are bound to the schema version" "The digests are bound to the schema version"
require_text "C.4 points at C.1.5" "is defined in C.1.5, as is the \`primary_key\` encoding"
require_text "spec-version 4" "<!-- vault-format: spec-version=4 -->"
require_text "change-control note" '`spec-version = 4` adds C.1.5'

CASES=$((CASES + 1))
python3 -I - "$SPEC" <<'PY'
import hashlib, re, struct, sys

text = open(sys.argv[1]).read()
vectors = dict(re.findall(r"<!-- row-vector: (\w+)=([0-9a-f]+) -->", text))
assert set(vectors) == {"primary_key", "row_message", "row_digest", "absent_message", "absent_digest"}, vectors

def label(s):
    b = s.encode(); assert 1 <= len(b) <= 64 and re.fullmatch(rb"[a-z0-9/_.-]+", b)
    return struct.pack(">H", len(b)) + b
def bytes_(b, mx):
    assert len(b) <= mx; return struct.pack(">I", len(b)) + b
def text_(s, mx): return bytes_(s.encode("utf-8"), mx)

def val(kind, stored):
    """Tag a stored value by its DECLARED kind; refuse any storage class that does not match."""
    if stored is None: return b"\x00"
    if kind == "int":
        assert type(stored) is int and -2**63 <= stored < 2**63, "int column holds a non-integer"
        return b"\x01" + struct.pack(">q", stored)
    if kind == "text":
        assert type(stored) is str, "text column holds a non-text"
        return b"\x02" + text_(stored, 2**24)
    if kind == "uuid":
        assert type(stored) is bytes and len(stored) == 16, "uuid column holds a non-16-byte blob"
        return b"\x03" + stored
    if kind == "bytes":
        assert type(stored) is bytes, "bytes column holds a non-blob"
        return b"\x04" + bytes_(stored, 2**24)
    raise AssertionError(kind)

def refuses(kind, stored):
    try: val(kind, stored)
    except AssertionError: return True
    return False

tag_id = bytes.fromhex("0198c2c15f0a7c3d8a4e9b1d00000011")
row = {"id": ("uuid", tag_id), "name": ("text", "trip \u00e9"), "color": ("text", None), "sort_order": ("int", -7)}

pk = val("uuid", tag_id); assert len(pk) <= 256
msg = label("dohflow/sync-row/v1") + text_("tags", 64) + struct.pack(">H", len(row))
for name in sorted(row, key=lambda n: n.encode()):
    msg += text_(name, 64) + val(*row[name])
absent = label("dohflow/sync-row-absent/v1") + text_("tags", 64) + bytes_(pk, 256)

assert pk.hex() == vectors["primary_key"], "primary_key"
assert msg.hex() == vectors["row_message"], "row_message"
assert hashlib.sha256(msg).hexdigest() == vectors["row_digest"], "row_digest"
assert absent.hex() == vectors["absent_message"], "absent_message"
assert hashlib.sha256(absent).hexdigest() == vectors["absent_digest"], "absent_digest"

# Domain separation: the labels have different lengths, so neither message parses as the other.
assert msg[:2] != absent[:2]
# Sorting by name bytes: the same row in another physical column order has the same digest.
shuffled = dict(reversed(list(row.items())))
m2 = label("dohflow/sync-row/v1") + text_("tags", 64) + struct.pack(">H", len(shuffled))
for name in sorted(shuffled, key=lambda n: n.encode()):
    m2 += text_(name, 64) + val(*shuffled[name])
assert m2 == msg
# Declared kind decides the tag: the same bytes under two kinds encode differently, and a
# stored value of the wrong storage class is refused rather than coerced.
assert val("uuid", tag_id) != val("bytes", tag_id)
assert refuses("int", "7") and refuses("int", b"\x07") and refuses("text", 7) and refuses("text", b"x")
assert refuses("uuid", tag_id[:15]) and refuses("uuid", tag_id + b"\x00") and refuses("uuid", "x" * 16)
assert refuses("bytes", "x") and refuses("int", 2**63) and refuses("int", 1.5) and refuses("text", 1.5)
# NULL is one byte for every kind, and an empty text, an empty blob and 0 differ from it and each other.
assert len({val("int", None), val("text", None), val("uuid", None), val("bytes", None)}) == 1
assert len({val("int", None), val("text", ""), val("bytes", b""), val("int", 0)}) == 4
PY
echo "    ok   — vectors re-derived independently; sorting, domain separation and kind mismatches checked"

echo "PASS — $CASES row-digest contract assertions, 0 failures"
