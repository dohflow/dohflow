#!/usr/bin/env bash
#
# Contract test for scripts/cargo-audit.py (personal-cfo-md8h2.13): the one scoped, dated
# exception for RUSTSEC-2026-0381 on webpki-roots 0.26.11 is matched by advisory + crate +
# version (never by id alone), expires, and cannot go stale. No network; fixtures only.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
SCRIPT="$REPO_ROOT/scripts/cargo-audit.py"
WORK="$(cd "$(mktemp -d)" && pwd -P)"
# Remove only the files this test made (no recursive delete), then the directory.
trap 'cd /; find "$WORK" -depth -mindepth 1 \( -type f -o -type l \) -exec unlink {} \; 2>/dev/null; find "$WORK" -depth -type d -exec rmdir {} \; 2>/dev/null' EXIT
CASES=0

report() {  # report FILE then (id crate version)...
  local file="$1"; shift
  python3 -I - "$file" "$@" <<'PY'
import json, sys
out, items = sys.argv[1], sys.argv[2:]
lst = []
for i in range(0, len(items), 3):
    lst.append({"advisory": {"id": items[i]}, "package": {"name": items[i + 1], "version": items[i + 2]}})
json.dump({"vulnerabilities": {"found": bool(lst), "count": len(lst), "list": lst}, "warnings": {}}, open(out, "w"))
PY
}

expect() {  # expect LABEL WANT_EXIT -- args...
  local label="$1" want="$2"; shift 3
  CASES=$((CASES + 1))
  set +e
  python3 -I "$SCRIPT" "$@" >"$WORK/out.txt" 2>&1
  local got=$?
  set -e
  if [ "$got" -ne "$want" ]; then
    echo "    FAIL — $label (exit $got, wanted $want)" >&2
    cat "$WORK/out.txt" >&2
    exit 1
  fi
  echo "    ok   — $label"
}

NEW=RUSTSEC-2026-0381
report "$WORK/clean.json"
report "$WORK/shim.json" $NEW webpki-roots 0.26.11
report "$WORK/shim-and-other.json" $NEW webpki-roots 0.26.11 RUSTSEC-2099-0001 foo 1.0.0
report "$WORK/real-crate.json" $NEW webpki-roots 1.0.9
report "$WORK/other-crate.json" $NEW some-other-crate 0.26.11
echo "not json" > "$WORK/garbage.json"

expect "the shim finding in both lockfiles is allowed" 0 -- --json-file "$WORK/shim.json" --json-file "$WORK/shim.json" --today 2026-11-01
expect "the shim finding in one lockfile is allowed" 0 -- --json-file "$WORK/shim.json" --json-file "$WORK/clean.json" --today 2026-11-01
expect "any other vulnerability still fails" 1 -- --json-file "$WORK/shim-and-other.json" --today 2026-11-01
expect "the same advisory on the 1.x crate is NOT excepted" 1 -- --json-file "$WORK/real-crate.json" --today 2026-11-01
expect "the same advisory on another crate is NOT excepted" 1 -- --json-file "$WORK/other-crate.json" --today 2026-11-01
expect "the exception expires" 1 -- --json-file "$WORK/shim.json" --today 2027-01-01
expect "an exception that matches nothing fails, so it gets removed" 1 -- --json-file "$WORK/clean.json" --json-file "$WORK/clean.json" --today 2026-11-01
expect "unreadable audit output is an error, not a pass" 2 -- --json-file "$WORK/garbage.json" --today 2026-11-01

CASES=$((CASES + 1))
if grep -q "RUSTSEC-2026-0381" "$REPO_ROOT/.github/workflows/ci.yml"; then
  echo "    FAIL — ci.yml must call the wrapper, not name the advisory itself" >&2; exit 1
fi
grep -q "scripts/cargo-audit.py" "$REPO_ROOT/.github/workflows/ci.yml" || { echo "    FAIL — ci.yml does not run scripts/cargo-audit.py" >&2; exit 1; }
echo "    ok   — ci.yml runs the wrapper"
echo "cargo-audit.test.sh: $CASES cases passed"
