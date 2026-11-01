#!/usr/bin/env bash
#
# The signed keychain smoke harness (personal-cfo-md8h2.18; ADR 0002 addendum 2026-10-24).
#
# A restricted entitlement (the data-protection keychain's keychain-access-groups) is honored only
# inside a SIGNED APP BUNDLE that carries the provisioning profile, so an ordinary `cargo test` can
# never read back a stored item's attributes. This script builds a throwaway bundle around
# platform-macos's keychain test binary, signs it exactly as release.sh signs the app (the real app id,
# the same Developer ID identity, the same entitlements template, the same embedded profile), runs the
# entitled `--ignored` tests inside it, and reports pass or fail. It also runs the same binary
# UNENTITLED and requires the typed failure, so a pass cannot come from a test that never needed the
# entitlement.
#
# It is NOT a smoke mode inside the shipped app (that would be an extra entry point in the release
# binary), and the bundle it builds is never shipped, uploaded, stapled or notarized for distribution:
# it lives in a temporary directory that the script removes on exit.
#
# Run it on the release machine, before a release (release-checklist.md step 5):
#
#   ./scripts/keychain-smoke.sh --check      # everything except signing and running
#   ./scripts/keychain-smoke.sh              # the full run
#
# Environment (read from the environment only, never the repo):
#   APPLE_SIGNING_IDENTITY       the Developer ID Application identity, ending in (TEAMID). When unset and
#                                exactly one valid "Developer ID Application" identity is installed, that
#                                one is used.
#   APPLE_PROVISIONING_PROFILE   the Developer ID provisioning profile. Defaults to
#                                ~/.config/personal-cfo/DohFlow_Developer_ID.provisionprofile.
# Neither needs release.env's notarization credentials, so do not source it for this.
#
# Not covered: the Touch ID paths (they need a person at the sensor; run them by hand from the signed
# bundle, see docs/operations/release-signing.md), and the launch behavior of the shipped app.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"

fail() { echo "keychain-smoke: $*" >&2; exit 1; }

mode="run"
case "${1:-}" in
  "") ;;
  --check) mode="check" ;;
  -h|--help) sed -n '2,36p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
  *) fail "unknown argument '$1' (use --check or no argument)" ;;
esac

[[ "$(uname -s)" == "Darwin" ]] || fail "macOS only."
command -v cargo >/dev/null 2>&1 || fail "'cargo' not on PATH."

# ── Identity and profile: the same checks release.sh makes, against the same inputs ──
identities="$(security find-identity -v -p codesigning || true)"
identity="${APPLE_SIGNING_IDENTITY:-}"
if [[ -z "$identity" ]]; then
  candidates="$(grep -F 'Developer ID Application' <<<"$identities" | sed -E 's/^[^"]*"([^"]*)".*$/\1/' || true)"
  count="$(grep -c . <<<"$candidates" || true)"
  [[ "$count" == "1" ]] \
    || fail "set APPLE_SIGNING_IDENTITY (found $count valid Developer ID Application identities)."
  identity="$candidates"
fi
grep -qF -- "$identity" <<<"$identities" \
  || fail "'$identity' is not a valid codesigning identity on this machine."
[[ "$identity" =~ \(([A-Z0-9]{10})\)$ ]] \
  || fail "cannot read the 10-character team id from the identity: it must end in (TEAMID)."
team_id="${BASH_REMATCH[1]}"
identity_sha1="$(awk -v id="$identity" 'index($0, id) {print $2; exit}' <<<"$identities")"

profile="${APPLE_PROVISIONING_PROFILE:-$HOME/.config/personal-cfo/DohFlow_Developer_ID.provisionprofile}"
[[ -f "$profile" ]] || fail "the provisioning profile is not a file: $profile (set APPLE_PROVISIONING_PROFILE)."

bundle_id="$(python3 -c "import json,sys;print(json.load(open(sys.argv[1]))['identifier'])" \
  "$repo_root/apps/desktop/src-tauri/tauri.conf.json")"
python3 -I "$repo_root/scripts/check-provisioning-profile.py" \
  --profile "$profile" --bundle-id "$bundle_id" --team-id "$team_id" --min-days 30 \
  --cert-sha1 "$identity_sha1" \
  || fail "the provisioning profile is not usable (see above)."
access_group="$team_id.$bundle_id"

work="$(mktemp -d "${TMPDIR:-/tmp}/dohflow-keychain-smoke.XXXXXX")"
trap 'rm -rf "$work"' EXIT

# ── Build the test binary (the code under test, exactly as `cargo test` builds it) ──
echo "==> building platform-macos's keychain tests"
messages="$work/cargo-messages.json"
cargo test -p platform-macos --test keychain --no-run --message-format=json >"$messages" \
  || fail "the test build failed."
test_binary="$(python3 -c "
import json, sys
exe = None
for line in open(sys.argv[1]):
    try:
        message = json.loads(line)
    except ValueError:
        continue
    if message.get('reason') == 'compiler-artifact' and message.get('executable') \
            and message.get('target', {}).get('name') == 'keychain' and message['target'].get('test', False):
        exe = message['executable']
print(exe or '')
" "$messages")"
[[ -x "$test_binary" ]] || fail "could not find the built test binary."

# ── The throwaway bundle: the real app id, the same entitlements, the same profile ──
app="$work/DohFlowKeychainSmoke.app"
mkdir -p "$app/Contents/MacOS"
cp "$test_binary" "$app/Contents/MacOS/DohFlowKeychainSmoke"
cp "$profile" "$app/Contents/embedded.provisionprofile"
cat >"$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>$bundle_id</string>
  <key>CFBundleExecutable</key><string>DohFlowKeychainSmoke</string>
  <key>CFBundleName</key><string>DohFlow keychain smoke (throwaway, never shipped)</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>0</string>
  <key>CFBundleShortVersionString</key><string>0</string>
</dict></plist>
PLIST
entitlements="$work/entitlements.plist"
python3 -c "
import sys
template, team, bundle, out = sys.argv[1:5]
open(out, 'w').write(open(template).read().replace('@TEAM_ID@', team).replace('@BUNDLE_ID@', bundle))
" "$repo_root/apps/desktop/src-tauri/entitlements/macos-release.plist.in" "$team_id" "$bundle_id" "$entitlements"
plutil -lint "$entitlements" >/dev/null || fail "the generated entitlements plist is not valid."

if [[ "$mode" == "check" ]]; then
  echo "ok: --check: identity, profile ($access_group), test build and bundle layout are fine; nothing was signed or run."
  exit 0
fi

echo "==> signing the throwaway bundle (hardened runtime, no timestamp, never notarized)"
codesign --force --options runtime --timestamp=none \
  --entitlements "$entitlements" --sign "$identity" "$app" \
  || fail "codesign failed."
codesign --verify --strict "$app" || fail "the throwaway bundle does not verify."
# The harness must carry exactly what the shipped app carries, so a pass speaks for the release.
signed_entitlements="$(codesign -d --entitlements :- "$app" 2>/dev/null)" \
  || fail "cannot read the signed bundle's entitlements."
python3 -I "$repo_root/scripts/check-signed-entitlements.py" --bundle-id "$bundle_id" --team-id "$team_id" \
  <<<"$signed_entitlements" || fail "the throwaway bundle's entitlements are not the release's."
cmp -s "$app/Contents/embedded.provisionprofile" "$profile" || fail "the embedded profile differs from the profile."

# ── The entitled run, inside the bundle ──
# The tests are named, and exactly that many must pass: a rename that leaves nothing to run must not read
# as a pass. The conformance suite adapts to the machine's RUNTIME biometric state (a Mac with nothing
# enrolled must refuse a biometric item with the typed reason and store nothing; a Touch ID Mac must
# store it), so it runs everywhere. What needs a person at the sensor is not run here.
entitled_tests=(
  entitled_a_device_only_item_round_trips
  entitled_a_stored_item_has_the_device_only_class_and_is_not_synchronizable
  entitled_a_public_tag_is_stored_and_read_back_without_authentication
  entitled_the_access_group_is_the_signed_entitlement
  entitled_the_store_passes_the_conformance_suite
)
echo "==> entitled run (inside the signed bundle)"
entitled_log="$work/entitled.log"
set +e
DOHFLOW_KEYCHAIN_ACCESS_GROUP="$access_group" \
  "$app/Contents/MacOS/DohFlowKeychainSmoke" --ignored --test-threads=1 --exact "${entitled_tests[@]}" \
  >"$entitled_log" 2>&1
entitled_status=$?
set -e
cat "$entitled_log"
[[ "$entitled_status" == "0" ]] || fail "FAIL: the entitled tests failed inside the signed bundle."
grep -q "test result: ok. ${#entitled_tests[@]} passed; 0 failed" "$entitled_log" \
  || fail "FAIL: expected exactly the ${#entitled_tests[@]} entitled tests to pass."

# ── The unentitled run: the same binary without the entitlement must get the typed failure ──
echo "==> unentitled run (the same binary, ad-hoc signed, no profile)"
bare="$work/unentitled-runner"
cp "$test_binary" "$bare"
codesign --force --sign - "$bare" >/dev/null 2>&1 || fail "could not ad-hoc sign the unentitled copy."
unentitled_log="$work/unentitled.log"
# The always-run tests assert the typed failure and that nothing reached the login keychain.
"$bare" --test-threads=1 >"$unentitled_log" 2>&1 \
  || { cat "$unentitled_log"; fail "FAIL: the unentitled build did not behave as the always-run tests require."; }
grep -q "test result: ok. [1-9][0-9]* passed; 0 failed" "$unentitled_log" \
  || fail "FAIL: the unentitled always-run tests did not run."
# And an entitled test, run without the entitlement, must fail with MissingEntitlement.
set +e
DOHFLOW_KEYCHAIN_ACCESS_GROUP="$access_group" \
  "$bare" --ignored --test-threads=1 --exact "${entitled_tests[1]}" >"$unentitled_log" 2>&1
unentitled_status=$?
set -e
[[ "$unentitled_status" != "0" ]] \
  || fail "FAIL: an entitled test passed WITHOUT the entitlement; the signed pass proves nothing."
grep -q "MissingEntitlement" "$unentitled_log" \
  || fail "FAIL: the unentitled run did not fail with MissingEntitlement."

head="$(git -C "$repo_root" rev-parse --short HEAD 2>/dev/null || echo unknown)"
git -C "$repo_root" diff --quiet HEAD 2>/dev/null || head="$head+uncommitted"
echo
echo "PASS keychain-smoke @ $head on macOS $(sw_vers -productVersion), $(date -u +%Y-%m-%dT%H:%MZ)"
echo "  - a data-protection item round-trips (store, load, delete, gone) inside the signed bundle"
echo "  - its attributes read back device-only (akpu), synchronizable false, in group $access_group"
echo "  - the same binary without the entitlement fails with MissingEntitlement"
echo "  - the app reads its own access group from its entitlement; a public tag reads back with no prompt"
echo "  - the conformance suite passed for this Mac's biometric state ($(sw_vers -productVersion); on a Mac with no"
echo "    enrolled biometrics it required the typed refusal for a biometric item)"
echo "NOT RUN: the cases that need a person at the Touch ID sensor (a fresh match releasing a biometric item,"
echo "         the pre-flight prompt). Run them by hand on a Touch ID Mac; record this line, do not tick them."
echo "  (the bundle was throwaway: removed on exit, never shipped or notarized)"
