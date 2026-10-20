#!/usr/bin/env bash
#
# Tests for the exclusion list in scripts/generate-shipped-features.mjs
# (personal-cfo-md8h2.10).
#
# CI-safe: a stub `bd` on PATH prints canned feature beads; the generator runs in
# a throwaway directory, so the real docs/product/shipped-features.md and the
# real bead database are never touched. Needs node.

# The `check && pass || fail` lines are if-then-else on purpose: pass() always
# returns 0, so fail() runs only when the check failed.
# shellcheck disable=SC2015
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
GEN="$REPO_ROOT/scripts/generate-shipped-features.mjs"
LIST="$REPO_ROOT/scripts/shipped-features-exclusions.json"
[ -f "$GEN" ] || { echo "missing: $GEN" >&2; exit 1; }
[ -f "$LIST" ] || { echo "missing: $LIST" >&2; exit 1; }
command -v node >/dev/null || { echo "node is required" >&2; exit 1; }

TMP="$(cd "$(mktemp -d)" && pwd -P)"
trap 'cd /; find "$TMP" -depth -mindepth 1 \( -type f -o -type l \) -exec unlink {} \; 2>/dev/null; find "$TMP" -depth -type d -exec rmdir {} \; 2>/dev/null' EXIT

FAILURES=0
CASES=0
pass() { echo "    ok   — $*"; }
fail() { echo "    FAIL — $*" >&2; FAILURES=$((FAILURES + 1)); }
case_start() { CASES=$((CASES + 1)); echo; echo "[$CASES] $*"; }

# A stub bd: the generator only calls `bd list --type=feature --status=closed ...`.
mkdir -p "$TMP/bin" "$TMP/work/docs/product" "$TMP/home"
cat > "$TMP/bin/bd" <<'STUB'
#!/usr/bin/env bash
cat <<'JSON'
[
  {"id":"personal-cfo-28js0","title":"FEATURE: phone container builder","priority":1},
  {"id":"personal-cfo-7vbdq","title":"The M0 viewer","priority":1},
  {"id":"personal-cfo-r2pow","title":"LunchFlow adapter","priority":2},
  {"id":"personal-cfo-aaa1","title":"FEATURE: Real shipped thing","priority":2},
  {"id":"personal-cfo-bbb2","title":"Another shipped thing","priority":2}
]
JSON
STUB
chmod +x "$TMP/bin/bd"

# Run the generator the way a maintainer would: from the repo root of a checkout,
# with a stub bd first on PATH. HOME points at an empty dir so no real
# ~/.local/bin/bd is found.
gen() { ( cd "$TMP/work" && HOME="$TMP/home" PATH="$TMP/bin:$PATH" node "$GEN" "$@" ); }

OUT="$TMP/work/docs/product/shipped-features.md"

case_start "the shipped exclusion list is valid and names the three beads"
if node -e '
  const l = JSON.parse(require("fs").readFileSync(process.argv[1], "utf8")).exclusions
  const ids = l.map((e) => e.id).sort().join(",")
  if (ids !== "personal-cfo-28js0,personal-cfo-7vbdq,personal-cfo-r2pow") process.exit(1)
  if (!l.every((e) => typeof e.reason === "string" && e.reason.trim().length > 0)) process.exit(2)
' "$LIST"; then pass "28js0, 7vbdq and r2pow are listed, each with a reason"; else fail "exclusion list wrong or a reason is missing"; fi

case_start "excluded beads are omitted and the rest are kept"
gen >/dev/null 2>&1; rc=$?
[ "$rc" = 0 ] && pass "generator exits 0" || fail "generator exit $rc"
for id in 28js0 7vbdq r2pow; do
  if grep -q "\`$id\`" "$OUT"; then fail "excluded bead $id is in the output"; else pass "$id is omitted"; fi
done
for id in aaa1 bbb2; do
  if grep -q "\`$id\`" "$OUT"; then pass "$id is kept"; else fail "shipped bead $id is missing"; fi
done
grep -q "2 shipped features" "$OUT" && pass "the count excludes the omitted beads" || fail "wrong count: $(grep 'shipped features' "$OUT" | head -1)"

case_start "--check passes on a fresh generation and fails when an excluded row is hand-added"
gen --check >/dev/null 2>&1 && pass "--check passes right after generation" || fail "--check failed on a fresh file"
# shellcheck disable=SC2016  # the backticks are literal Markdown
printf '%s\n' '| `r2pow` | LunchFlow adapter |' >> "$OUT"
gen --check >/dev/null 2>&1 && fail "--check accepted a hand-added excluded row" || pass "--check rejects a hand-added excluded row"

case_start "a malformed exclusion list is an error, not a silent skip"
bad() {  # bad <label> <json>
  printf '%s' "$2" > "$TMP/bad.json"
  if ( cd "$TMP/work" && HOME="$TMP/home" PATH="$TMP/bin:$PATH" SHIPPED_FEATURES_EXCLUSIONS="$TMP/bad.json" node "$GEN" >/dev/null 2>&1 ); then
    fail "$1 was accepted"
  else
    pass "$1 is rejected"
  fi
}
bad "an entry with no reason" '{"exclusions":[{"id":"personal-cfo-aaa1"}]}'
bad "an entry with a blank reason" '{"exclusions":[{"id":"personal-cfo-aaa1","reason":"  "}]}'
bad "a bad bead id" '{"exclusions":[{"id":"nope","reason":"x"}]}'
bad "a duplicate id" '{"exclusions":[{"id":"personal-cfo-aaa1","reason":"x"},{"id":"personal-cfo-aaa1","reason":"y"}]}'
bad "exclusions that is not an array" '{"exclusions":{}}'

case_start "a stale entry only warns"
printf '%s' '{"exclusions":[{"id":"personal-cfo-gone","reason":"left over"}]}' > "$TMP/stale.json"
warn="$( cd "$TMP/work" && HOME="$TMP/home" PATH="$TMP/bin:$PATH" SHIPPED_FEATURES_EXCLUSIONS="$TMP/stale.json" node "$GEN" 2>&1 )"; rc=$?
[ "$rc" = 0 ] && pass "still exits 0" || fail "exit $rc"
case "$warn" in *"personal-cfo-gone matches no closed feature"*) pass "warns that the entry matches nothing" ;; *) fail "no stale-entry warning: $warn" ;; esac

echo
if [ "$FAILURES" -eq 0 ]; then
  echo "PASS — $CASES cases, 0 failures"
  exit 0
fi
echo "FAIL — $CASES cases, $FAILURES failed assertions" >&2
exit 1
