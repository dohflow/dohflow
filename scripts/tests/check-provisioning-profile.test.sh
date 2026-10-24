#!/usr/bin/env bash
#
# Tests for scripts/check-provisioning-profile.py and scripts/check-signed-entitlements.py
# (personal-cfo-md8h2.17). Pure: synthetic decoded plists, a pinned clock, no network, no Apple tools,
# no real profile, no secrets.

# shellcheck disable=SC2015
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
PROFILE_CHECK="$REPO_ROOT/scripts/check-provisioning-profile.py"
ENT_CHECK="$REPO_ROOT/scripts/check-signed-entitlements.py"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

FAILURES=0
CASES=0
pass() { echo "    ok   — $*"; }
fail() { echo "    FAIL — $*" >&2; FAILURES=$((FAILURES + 1)); }
case_start() { CASES=$((CASES + 1)); echo; echo "[$CASES] $*"; }

TEAM=ABCDE12345
BUNDLE=ai.personalcfo.desktop
NOW=2026-10-24T12:00:00+00:00

# make_profile <name> [overrides as python dict literal updates]
make_profile() {
  python3 -I - "$TMP/$1.plist" "${2:-}" <<'PY'
import plistlib, sys, datetime, hashlib
path, overrides = sys.argv[1], sys.argv[2]
team, bundle = "ABCDE12345", "ai.personalcfo.desktop"
profile = {
    "Name": "DohFlow Developer ID",
    "Platform": ["OSX"],
    "TeamIdentifier": [team],
    "ExpirationDate": datetime.datetime(2028, 1, 1),
    "DeveloperCertificates": [b"fake-certificate-bytes"],
    "Entitlements": {
        "com.apple.application-identifier": f"{team}.{bundle}",
        "com.apple.developer.team-identifier": team,
        "keychain-access-groups": [f"{team}.*"],
    },
}
if overrides:
    exec(overrides, {"profile": profile, "datetime": datetime, "team": team, "bundle": bundle})
plistlib.dump(profile, open(path, "wb"))
PY
}

run_profile() { python3 -I "$PROFILE_CHECK" --decoded "$TMP/$1.plist" --bundle-id "$BUNDLE" --team-id "$TEAM" --now "$NOW" "${@:2}"; }
expect_ok()   { local out rc; out="$(run_profile "$@" 2>&1)"; rc=$?; if [ "$rc" = 0 ]; then pass "$1 accepted"; else fail "$1 should be accepted (rc=$rc): $out"; fi; }
expect_bad()  { local name="$1" needle="$2"; shift 2; local out rc; out="$(run_profile "$name" "$@" 2>&1)"; rc=$?
  if [ "$rc" = 1 ] && grep -qF -- "$needle" <<<"$out"; then pass "$name refused: $needle"; else fail "$name should be refused with '$needle' (rc=$rc): $out"; fi; }

case_start "a good Developer ID profile is accepted"
make_profile good ""
expect_ok good
make_profile exact_group 'profile["Entitlements"]["keychain-access-groups"] = [f"{team}.{bundle}"]'
expect_ok exact_group

case_start "expiry: expired, inside the 90-day margin, exactly at the margin"
make_profile expired 'profile["ExpirationDate"] = datetime.datetime(2026, 10, 1)'
expect_bad expired "EXPIRED on 2026-10-01"
make_profile soon 'profile["ExpirationDate"] = datetime.datetime(2027, 1, 1)'
expect_bad soon "renew it"
make_profile edge 'profile["ExpirationDate"] = datetime.datetime(2027, 1, 22, 12, 0)'   # 90 days from the pinned now
expect_ok edge
make_profile just_short 'profile["ExpirationDate"] = datetime.datetime(2027, 1, 22, 11, 0)'
expect_bad just_short "renew it"
make_profile no_expiry 'del profile["ExpirationDate"]'
expect_bad no_expiry "no ExpirationDate"
expect_ok soon --min-days 30

case_start "the profile must belong to this team and this app"
make_profile other_team 'profile["TeamIdentifier"] = ["ZZZZZZ9999"]'
expect_bad other_team "not the signing team"
make_profile other_app 'profile["Entitlements"]["com.apple.application-identifier"] = f"{team}.some.other.app"'
expect_bad other_app "application-identifier"
make_profile wrong_declared_team 'profile["Entitlements"]["com.apple.developer.team-identifier"] = "ZZZZZZ9999"'
expect_bad wrong_declared_team "declares team-identifier"

case_start "the keychain access group must be allowed"
make_profile no_groups 'del profile["Entitlements"]["keychain-access-groups"]'
expect_bad no_groups "do not allow"
make_profile other_group 'profile["Entitlements"]["keychain-access-groups"] = [f"{team}.other.app"]'
expect_bad other_group "do not allow"
make_profile foreign_wildcard 'profile["Entitlements"]["keychain-access-groups"] = ["ZZZZZZ9999.*"]'
expect_bad foreign_wildcard "do not allow"

case_start "development and device profiles are refused"
make_profile devices 'profile["ProvisionedDevices"] = ["00008030-001234567890002E"]'
expect_bad devices "provisioned devices"
make_profile devprofile 'profile["Entitlements"]["get-task-allow"] = True'
expect_bad devprofile "get-task-allow"
make_profile ios 'profile["Platform"] = ["iOS"]'
expect_bad ios "not a macOS profile"

case_start "every problem is reported in one run, not just the first"
make_profile many 'profile["ExpirationDate"] = datetime.datetime(2026, 10, 1); profile["TeamIdentifier"] = ["ZZZZZZ9999"]; profile["ProvisionedDevices"] = ["x"]'
out="$(run_profile many 2>&1)"; count="$(grep -c '^  - ' <<<"$out")"
[ "$count" -ge 3 ] && pass "$count problems listed" || fail "expected at least 3 listed problems, got $count: $out"

case_start "the signing certificate must be one the profile carries (optional check)"
sha="$(printf 'fake-certificate-bytes' | shasum -a 1 | awk '{print $1}')"
expect_ok good --cert-sha1 "$sha"
expect_ok good --cert-sha1 "$(printf '%s' "$sha" | tr 'a-f' 'A-F' | sed 's/../&:/g; s/:$//')"
expect_bad good "signing certificate" --cert-sha1 0000000000000000000000000000000000000000

case_start "bad inputs fail loudly (exit 2), never as a pass"
for args in "--decoded $TMP/does-not-exist.plist" ; do
  out="$(python3 -I "$PROFILE_CHECK" $args --bundle-id "$BUNDLE" --team-id "$TEAM" 2>&1)"; rc=$?
  [ "$rc" = 2 ] && pass "missing file: exit 2" || fail "missing file should exit 2 (rc=$rc): $out"
done
printf 'not a plist' > "$TMP/garbage.plist"
out="$(python3 -I "$PROFILE_CHECK" --decoded "$TMP/garbage.plist" --bundle-id "$BUNDLE" --team-id "$TEAM" 2>&1)"; rc=$?
[ "$rc" = 2 ] && pass "garbage file: exit 2" || fail "garbage should exit 2 (rc=$rc): $out"
out="$(python3 -I "$PROFILE_CHECK" --decoded "$TMP/good.plist" --bundle-id "$BUNDLE" --team-id lowercase1 2>&1)"; rc=$?
[ "$rc" = 2 ] && pass "malformed team id: exit 2" || fail "malformed team id should exit 2 (rc=$rc): $out"
out="$(python3 -I "$PROFILE_CHECK" --profile "$TMP/not-a-real-profile" --bundle-id "$BUNDLE" --team-id "$TEAM" 2>&1)"; rc=$?
[ "$rc" = 2 ] && pass "a path that is not a profile: exit 2" || fail "non-profile path should exit 2 (rc=$rc): $out"

# ---- the signed app's entitlements ----------------------------------------------------------------
ent_plist() { python3 -I - "$1" <<'PY'
import plistlib, sys
team, bundle = "ABCDE12345", "ai.personalcfo.desktop"
data = {
  "com.apple.application-identifier": f"{team}.{bundle}",
  "com.apple.developer.team-identifier": team,
  "keychain-access-groups": [f"{team}.{bundle}"],
}
extra = sys.argv[1]
if extra:
    exec(extra, {"data": data, "team": team, "bundle": bundle})
sys.stdout.buffer.write(b"Executable=/x/app\n" if False else b"")
plistlib.dump(data, sys.stdout.buffer)
PY
}
run_ent() { python3 -I "$ENT_CHECK" --bundle-id "$BUNDLE" --team-id "$TEAM"; }

case_start "the signed app must carry exactly the three keychain and identity entitlements"
out="$(ent_plist '' | run_ent 2>&1)"; rc=$?; [ "$rc" = 0 ] && pass "exact set accepted" || fail "exact set should pass: $out"
out="$(ent_plist 'data["keychain-access-groups"]=[f"{team}.*"]' | run_ent 2>&1)"; rc=$?
[ "$rc" = 1 ] && grep -qF "keychain-access-groups" <<<"$out" && pass "wildcard group refused" || fail "wildcard should fail (rc=$rc): $out"
out="$(ent_plist 'data["keychain-access-groups"]=[f"{team}.{bundle}", f"{team}.extra"]' | run_ent 2>&1)"; rc=$?
[ "$rc" = 1 ] && pass "a second group refused" || fail "extra group should fail (rc=$rc): $out"
out="$(ent_plist 'data["com.apple.security.cs.allow-jit"]=True' | run_ent 2>&1)"; rc=$?
[ "$rc" = 1 ] && grep -qF "unexpected entitlement" <<<"$out" && pass "an extra entitlement refused" || fail "extra entitlement should fail (rc=$rc): $out"
out="$(ent_plist 'del data["com.apple.application-identifier"]' | run_ent 2>&1)"; rc=$?
[ "$rc" = 1 ] && pass "a missing identity key refused" || fail "missing key should fail (rc=$rc): $out"
out="$(printf 'no entitlements here' | run_ent 2>&1)"; rc=$?
[ "$rc" = 1 ] && pass "no plist at all refused" || fail "no plist should fail (rc=$rc): $out"
out="$( { printf 'Executable=/x/app\n'; ent_plist ''; } | run_ent 2>&1)"; rc=$?
[ "$rc" = 0 ] && pass "a leading Executable= line (as codesign prints) is tolerated" || fail "leading line should be tolerated: $out"

echo
if [ "$FAILURES" -eq 0 ]; then echo "check-provisioning-profile.test.sh: $CASES cases passed"; else echo "check-provisioning-profile.test.sh: $FAILURES failure(s)" >&2; exit 1; fi
