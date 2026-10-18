#!/usr/bin/env bash
#
# Regression test for scripts/write-latest-json.py — bead personal-cfo-md8h2.4.
#
# WHAT IT COVERS
#   The updater manifest that scripts/release.sh writes must carry the bundle's
#   macOS floor, so an installed app can decline an update it couldn't open
#   (ADR 0065 addendum 2026-10-17). This proves:
#     (a) minimumSystemVersion equals tauri.conf.json's
#         bundle.macOS.minimumSystemVersion, whatever that value is
#     (b) the existing shape is unchanged: version, notes, and both darwin
#         platform keys carrying the same signature and the url placeholder
#     (c) a tauri.conf.json without the floor makes the script fail and write
#         nothing, rather than publish a manifest the guard can't read
#
# HOW IT IS TESTED
#   Fixture files in a temporary directory; no build, signing or network.
#
# RUN
#   bash scripts/tests/write-latest-json.test.sh
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WRITER="$ROOT/scripts/write-latest-json.py"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/write-latest-json-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

PASS=0; FAIL=0
ok()   { echo "ok   - $1"; PASS=$((PASS + 1)); }
fail() { echo "FAIL - $1"; FAIL=$((FAIL + 1)); }
field() { python3 -c "import json,sys; d=json.load(open(sys.argv[1])); print(eval(sys.argv[2]))" "$1" "$2"; }

printf 'fixture-signature' > "$WORK/app.sig"

# (a) + (b), for two different floors so a hard-coded value can't pass.
for floor in 14.0 15.0; do
  printf '{"version":"0.3.0","bundle":{"macOS":{"minimumSystemVersion":"%s"}}}' "$floor" > "$WORK/conf.json"
  out="$WORK/latest-$floor.json"
  if python3 "$WRITER" 0.3.0 "fixture notes" "$WORK/app.sig" "$out" "$WORK/conf.json"; then
    ok "writes latest.json for floor $floor"
  else
    fail "writes latest.json for floor $floor"; continue
  fi
  [ "$(field "$out" "d['minimumSystemVersion']")" = "$floor" ] && ok "minimumSystemVersion is $floor" || fail "minimumSystemVersion is $floor"
done
out="$WORK/latest-15.0.json"
[ "$(field "$out" "d['version']")" = "0.3.0" ] && ok "version unchanged" || fail "version unchanged"
[ "$(field "$out" "d['notes']")" = "fixture notes" ] && ok "notes unchanged" || fail "notes unchanged"
[ "$(field "$out" "d['platforms']['darwin-aarch64']['signature'] == d['platforms']['darwin-x86_64']['signature'] == 'fixture-signature'")" = "True" ] \
  && ok "both platform keys carry the signature" || fail "both platform keys carry the signature"
[ "$(field "$out" "d['platforms']['darwin-aarch64']['url']")" = "REPLACE_WITH_THE_UPLOADED_APP_TAR_GZ_ASSET_URL" ] \
  && ok "url placeholder unchanged" || fail "url placeholder unchanged"

# (c)
printf '{"version":"0.3.0"}' > "$WORK/no-floor.json"
if python3 "$WRITER" 0.3.0 notes "$WORK/app.sig" "$WORK/latest-none.json" "$WORK/no-floor.json" 2>"$WORK/err.txt"; then
  fail "refuses a tauri.conf.json without a floor"
else
  ok "refuses a tauri.conf.json without a floor"
fi
[ ! -e "$WORK/latest-none.json" ] && ok "writes nothing when it refuses" || fail "writes nothing when it refuses"
grep -q "minimumSystemVersion" "$WORK/err.txt" && ok "the refusal names the missing field" || fail "the refusal names the missing field"

echo "$PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
