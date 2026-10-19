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

require_text "section exists" "### C.1.5 Class-1 row digest and primary key"
require_text "storage class, not affinity" "**storage class** of each stored value (not the column's declared affinity)"
require_text "NULL tag" '| `0` | NULL | nothing |'
require_text "INTEGER tag" '| `1` | INTEGER | `i64` |'
require_text "TEXT tag" '| `2` | TEXT | `text(2^24)`'
require_text "BLOB tag" '| `3` | BLOB | `bytes(2^24)` |'
require_text "REAL is an error" "A REAL value has no tag and is an error"
require_text "primary key is capped" "It is at most 256 bytes; a"
require_text "primary key is never truncated" "longer key is an error and is never truncated"
require_text "row digest label" 'label "dohflow/sync-row/v1"'
require_text "columns sort by name bytes" "**raw bytes of their names**"
require_text "every column is included" "Every column of the row is included."
require_text "absent digest label" 'label "dohflow/sync-row-absent/v1"'
require_text "deletion changes the hash" "so a deletion changes the write-set hash"
require_text "insert then delete is not in the set" "inserted and deleted inside the same command is not in the set"
require_text "final state once" "touched several times appears once, with its final state"
require_text "the command's own log row is excluded" "The \`operation_log\` row of the command itself is not in the set"
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
def val(v):
    if v is None: return b"\x00"
    if isinstance(v, int): assert -2**63 <= v < 2**63; return b"\x01" + struct.pack(">q", v)
    if isinstance(v, str): return b"\x02" + text_(v, 2**24)
    if isinstance(v, bytes): return b"\x03" + bytes_(v, 2**24)
    raise TypeError(type(v))

tag_id = bytes.fromhex("0198c2c15f0a7c3d8a4e9b1d00000011")
row = {"id": tag_id, "name": "trip é", "color": None, "sort_order": -7}

pk = b"".join(val(v) for v in [tag_id]); assert len(pk) <= 256
msg = label("dohflow/sync-row/v1") + text_("tags", 64) + struct.pack(">H", len(row))
for name in sorted(row, key=lambda n: n.encode()):
    msg += text_(name, 64) + val(row[name])
absent = label("dohflow/sync-row-absent/v1") + text_("tags", 64) + bytes_(pk, 256)

assert pk.hex() == vectors["primary_key"], "primary_key"
assert msg.hex() == vectors["row_message"], "row_message"
assert hashlib.sha256(msg).hexdigest() == vectors["row_digest"], "row_digest"
assert absent.hex() == vectors["absent_message"], "absent_message"
assert hashlib.sha256(absent).hexdigest() == vectors["absent_digest"], "absent_digest"

# Domain separation: the first field of each message is a length-prefixed label, and the
# two labels have different lengths, so neither message is a prefix-parse of the other.
assert msg[:2] != absent[:2]
# Sorting by name bytes, not by declared order: the same row in another physical column
# order has the same digest.
shuffled = dict(reversed(list(row.items())))
m2 = label("dohflow/sync-row/v1") + text_("tags", 64) + struct.pack(">H", len(shuffled))
for name in sorted(shuffled, key=lambda n: n.encode()):
    m2 += text_(name, 64) + val(shuffled[name])
assert m2 == msg
# A NULL, an empty text, an empty blob and the integer 0 all encode differently.
assert len({val(None), val(""), val(b""), val(0)}) == 4
# Primary keys of different shapes never collide when fixed per table.
assert val(1) != val("1") != val(b"1")
PY
echo "    ok   — vectors re-derived independently; sorting and domain separation checked"

echo "PASS — $CASES row-digest contract assertions, 0 failures"
