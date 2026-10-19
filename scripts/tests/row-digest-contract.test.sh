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
require_text "every manifest column is included" "Every column the manifest lists for the table, or for"
require_text "absent digest label" 'label "dohflow/sync-row-absent/v1"'
require_text "deletion changes the hash" "so a deletion changes the write-set hash"
require_text "insert then delete is not in the set" "inserted and deleted inside the same"
require_text "final state once" "touched several times appears once, with its final"
require_text "the command's own log row is excluded" "The \`operation_log\` row of the command itself is not in the write set"
require_text "class-1 tables hold only class-1 columns" "A class-1 table contains only class-1 columns"
require_text "the write set is a state difference" "**The write set is a state difference, not a record of writes.**"
require_text "a no-op write is not in the set" "A no-op \`UPDATE\`, an \`INSERT OR"
require_text "a key change is delete plus presence" "An update that changes a primary key is a deletion of the old key plus the"
require_text "cascades are included and tested" "a test covers a cascade"
require_text "keys are never assigned by SQLite" "**Keys are never SQLite's to assign.**"
require_text "no autoincrement keys" "\`AUTOINCREMENT\` key is left for SQLite to allocate"
require_text "the key limit is a schema-time guarantee" "the limit is a"
require_text "C.4 sorts by the tagged encoding" "The primary key's bytes are the C.1.5 tagged"
require_text "C.4 names the state difference" "The set of rows is the C.1.5 state difference"
require_text "the set covers what Sync replicates" "**What the write set covers.** Exactly what Sync replicates"
require_text "class-4 groups routed to class 1 are covered" "each class-4 group that ADR 0074 Decision 5 routes to"
require_text "field-split tables are projections" "a **projection**: only the replicated"
require_text "attachments projection excludes device-local columns" "never \`storage_id\`, wrapped keys or nonces"
require_text "settings projection excludes device keys" "never \`device.*\`"
require_text "household record digest" "digest uses the table name \`household\`"
require_text "household write set is never empty" "never empty for"
require_text "values outside every projection are named" "A value that lives in a class-3 table and is not part of a B.2 projection"
require_text "apply writes the payload value" "never something derived from local state"
require_text "declared-kind validation is mandatory" "**Declared-kind validation is mandatory before Sync.**"
require_text "mismatch blocks sync, repaired by migration" "repaired by a reviewed migration that rewrites the value to its kind"
require_text "genesis bead owns the check" "(\`personal-cfo-1cltt\`)"
require_text "5b delivers the schema tests" "are delivered by part 5b"
require_text "other schema version is update-required" "update-required state; the hash is never skipped"
require_text "keyless tables are folded into a parent" "**Folded children.** A replicated table that declares no primary key"
require_text "ledger_postings folds into ledger_transactions" '| `ledger_postings` | `ledger_transactions` | `transaction_id` (the parent'"'"'s `id`) |'
require_text "attachment_links folds into attachments" '| `attachment_links` | `attachments` | `attachment_id` (the parent'"'"'s `id`) |'
require_text "the child section layout" "A parent that has folded children appends a **child section**"
require_text "child rows sort bytewise and keep duplicates" "bytewise by the \`child_row\` bytes themselves"
require_text "the child digest is a multiset" "postings count twice and the digest is a multiset"
require_text "a child change touches the parent" "An insert, update or delete of"
require_text "an orphan child is an error" "a child row whose parent does not exist is an error"
require_text "provenance_links is outside the write set" "\`provenance_links\` is **not** in the write set"
require_text "the child sort key excludes the length prefix" "excluding the \`bytes(2^16)\` length prefix, lexicographically"
require_text "folded children are bounded and unordered" "**Folded children are bounded and unordered.**"
require_text "no reader depends on child row order" "no reader of a folded child table may depend on row order"
require_text "deleting and moving children" "**Deleting and moving.**"
require_text "an orphaned child fails the command" "fails the"
require_text "a moved child touches both parents" "both parents are write-set entries"
require_text "every replicated table has a key or is folded" "declares a supplied primary key or is listed as a folded child with its"
require_text "digests are bound to the schema version" "The digests are bound to the schema version"
require_text "C.4 points at C.1.5" "is defined in C.1.5, as is the \`primary_key\` encoding"
require_text "spec-version 4" "<!-- vault-format: spec-version=4 -->"
require_text "change-control note" '`spec-version = 4` adds C.1.5'

CASES=$((CASES + 1))
python3 -I - "$SPEC" <<'PY'
import hashlib, re, struct, sys

text = open(sys.argv[1]).read()
vectors = dict(re.findall(r"<!-- row-vector: (\w+)=([0-9a-f]+) -->", text))
assert set(vectors) == {"primary_key", "row_message", "row_digest", "absent_message", "absent_digest", "child_row_a", "child_row_b", "parent_message", "parent_digest", "order_message", "order_digest", "order_wrong_digest"}, vectors

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

# Folded children: a parent's digest carries its child rows as a multiset.
def cols_enc(cols):
    out = struct.pack(">H", len(cols))
    for n in sorted(cols, key=lambda x: x.encode()):
        out += text_(n, 64) + val(*cols[n])
    return out

txn = bytes.fromhex("0198c2c15f0a7c3d8a4e9b1d00000021")
acct1 = bytes.fromhex("0198c2c15f0a7c3d8a4e9b1d00000031")
acct2 = bytes.fromhex("0198c2c15f0a7c3d8a4e9b1d00000032")
def posting(acct, minor):
    return {"transaction_id": ("uuid", txn), "ledger_account_id": ("uuid", acct),
            "minor_units": ("int", minor), "currency": ("text", "USD"),
            "currency_exponent": ("int", 2), "posting_date": ("text", "2026-10-17")}
rows = [cols_enc(posting(acct1, -500)), cols_enc(posting(acct1, -500)), cols_enc(posting(acct2, 1000))]
assert rows[0] == rows[1] and rows[0] != rows[2], "two identical postings and one other"
ordered = sorted(rows)                      # bytewise, duplicates kept
distinct = sorted(set(rows))
assert distinct[0].hex() == vectors["child_row_a"] and distinct[1].hex() == vectors["child_row_b"]
parent = {"id": ("uuid", txn), "memo": ("text", "split")}
pm = label("dohflow/sync-row/v1") + text_("ledger_transactions", 64) + struct.pack(">H", len(parent))
for n in sorted(parent, key=lambda x: x.encode()):
    pm += text_(n, 64) + val(*parent[n])
pm += struct.pack(">H", 1) + text_("ledger_postings", 64) + struct.pack(">I", len(ordered))
for r in ordered:
    pm += bytes_(r, 2**16)
assert pm.hex() == vectors["parent_message"], "parent_message"
assert hashlib.sha256(pm).hexdigest() == vectors["parent_digest"], "parent_digest"
# The multiset matters: dropping one copy of the duplicate changes the digest.
fewer = label("dohflow/sync-row/v1") + text_("ledger_transactions", 64) + struct.pack(">H", len(parent))
for n in sorted(parent, key=lambda x: x.encode()):
    fewer += text_(n, 64) + val(*parent[n])
fewer += struct.pack(">H", 1) + text_("ledger_postings", 64) + struct.pack(">I", 2)
for r in sorted([rows[0], rows[2]]):
    fewer += bytes_(r, 2**16)
assert hashlib.sha256(fewer).hexdigest() != vectors["parent_digest"]

# Child sort order: by the child_row bytes without the length prefix.
att = bytes.fromhex("0198c2c15f0a7c3d8a4e9b1d00000041")
e1 = bytes.fromhex("0198c2c15f0a7c3d8a4e9b1d0000004a")
e2 = bytes.fromhex("0198c2c15f0a7c3d8a4e9b1d0000004b")
def link(entity, kind):
    return {"attachment_id": ("uuid", att), "entity_kind": ("text", kind),
            "entity_id": ("uuid", entity), "created_at": ("text", "2026-10-17")}
long_row = cols_enc(link(e1, "transaction_document"))
short_row = cols_enc(link(e2, "tx"))
assert len(long_row) > len(short_row)
def order_msg(rows):
    m = label("dohflow/sync-row/v1") + text_("attachments", 64) + struct.pack(">H", 2)
    for n, v in sorted({"id": ("uuid", att), "mime_type": ("text", "image/png")}.items(), key=lambda kv: kv[0].encode()):
        m += text_(n, 64) + val(*v)
    m += struct.pack(">H", 1) + text_("attachment_links", 64) + struct.pack(">I", len(rows))
    for r in rows:
        m += bytes_(r, 2**16)
    return m
right = sorted([long_row, short_row])                              # no length prefix: long row first
wrong = sorted([long_row, short_row], key=lambda b: bytes_(b, 2**16))  # length-prefixed: short first
assert right[0] == long_row and wrong[0] == short_row, "the two readings must disagree"
assert order_msg(right).hex() == vectors["order_message"], "order_message"
assert hashlib.sha256(order_msg(right)).hexdigest() == vectors["order_digest"], "order_digest"
assert hashlib.sha256(order_msg(wrong)).hexdigest() == vectors["order_wrong_digest"], "order_wrong_digest"
assert vectors["order_digest"] != vectors["order_wrong_digest"]

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
