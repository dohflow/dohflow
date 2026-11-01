#!/usr/bin/env bash
#
# Contract tests for scripts/keychain-smoke.sh (personal-cfo-md8h2.18). The harness itself needs a
# Developer ID identity, the provisioning profile and a Mac, so it cannot run in CI; what CAN be
# guarded here is its shape: it never ships or notarizes the bundle it builds, it signs with the same
# inputs release.sh does, and the tests it names still exist (a rename would otherwise leave it
# running nothing). The argument handling is exercised wherever the script can run at all (macOS).

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
SCRIPT="$REPO_ROOT/scripts/keychain-smoke.sh"
TESTS="$REPO_ROOT/crates/platform-macos/tests/keychain.rs"

FAILURES=0
CASES=0
pass() { echo "    ok   — $*"; }
fail() { echo "    FAIL — $*" >&2; FAILURES=$((FAILURES + 1)); }
case_start() { CASES=$((CASES + 1)); echo; echo "[$CASES] $*"; }

case_start "the script parses"
bash -n "$SCRIPT" && pass "bash -n" || fail "syntax error"

case_start "the bundle is throwaway: a temp directory, removed on exit, never notarized or shipped"
grep -q 'mktemp -d' "$SCRIPT" && pass "built in a mktemp directory" || fail "no mktemp directory"
grep -q "trap 'rm -rf \"\$work\"' EXIT" "$SCRIPT" && pass "removed on exit" || fail "no cleanup trap"
code_only="$(grep -v '^[[:space:]]*#' "$SCRIPT")"
if grep -Eq 'notarytool|stapler|publish-release|gh release|upload' <<<"$code_only"; then
  fail "mentions notarizing, stapling or uploading"
else
  pass "no notarize, staple or upload step"
fi

case_start "it signs with release.sh's inputs: the entitlements template, the profile checker, the signed-entitlements check"
for needle in \
  'entitlements/macos-release.plist.in' \
  'scripts/check-provisioning-profile.py' \
  'scripts/check-signed-entitlements.py' \
  'embedded.provisionprofile' \
  '--options runtime'; do
  grep -qF -- "$needle" "$SCRIPT" && pass "$needle" || fail "missing: $needle"
done

case_start "every test it names exists, and the unentitled run is required"
names="$(sed -n '/^entitled_tests=(/,/^)/p; /entitled_tests+=(/p' "$SCRIPT" | grep -Eo 'entitled_[a-z_]+' | grep -vx 'entitled_tests')"
[[ -n "$names" ]] && pass "found the named tests" || fail "no named tests"
for name in $names; do
  grep -q "fn $name()" "$TESTS" && pass "$name" || fail "$name is named by the script but not defined in $TESTS"
done
grep -q 'MissingEntitlement' "$SCRIPT" && pass "requires MissingEntitlement unentitled" || fail "no unentitled check"

case_start "argument handling (macOS only: the script refuses elsewhere)"
if [[ "$(uname -s)" == "Darwin" ]]; then
  out="$("$SCRIPT" --bogus 2>&1)"; status=$?
  [[ $status -ne 0 && "$out" == *"unknown argument"* ]] && pass "an unknown argument is refused" || fail "unknown argument: $status $out"
  out="$("$SCRIPT" --help 2>&1)"; status=$?
  [[ $status -eq 0 && "$out" == *"signed keychain smoke harness"* ]] && pass "--help" || fail "--help: $status"
  out="$(APPLE_SIGNING_IDENTITY='Developer ID Application: Nobody (ZZZZZZZZZZ)' "$SCRIPT" --check 2>&1)"; status=$?
  [[ $status -ne 0 && "$out" == *"not a valid codesigning identity"* ]] && pass "an identity that is not installed is refused" || fail "bad identity: $status $out"
else
  pass "skipped on $(uname -s)"
fi

echo
if [[ $FAILURES -eq 0 ]]; then
  echo "keychain-smoke.test.sh: $CASES cases, all ok"
else
  echo "keychain-smoke.test.sh: $FAILURES failure(s)" >&2
  exit 1
fi
