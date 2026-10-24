#!/usr/bin/env python3
"""Check the macOS provisioning profile release.sh embeds (personal-cfo-md8h2.17; ADR 0002 addendum
2026-10-24).

The data-protection keychain needs the restricted entitlements `com.apple.application-identifier` and
`keychain-access-groups`; a Developer ID app can only carry them when an embedded provisioning profile
authorizes them. Without a valid profile the keychain does not work and the app may not launch at all
[unverified; see the addendum], so release.sh REFUSES to build when this check fails.

Usage:
  check-provisioning-profile.py --profile PATH --bundle-id ID --team-id TEAM [--min-days 90]
                                [--cert-sha1 HEX]
  check-provisioning-profile.py --decoded PLIST ...        (an already-decoded plist; used by the tests)

`--profile` is the CMS-signed file Apple issues; it is decoded with `/usr/bin/security cms -D`.
`--now ISO8601` pins the clock (tests). Exit 0 and a one-line summary when the profile is usable;
otherwise exit 1 listing every problem (never just the first), so a fix is one round trip.

What is checked (the profile's own key names; the first real profile may show a difference, in which
case fix this script and its tests together, never loosen a check to make a build pass):
  * it is a macOS profile (`Platform` is present and contains OSX or macOS), not a development or device profile
    (no `ProvisionedDevices`, no `get-task-allow`);
  * it belongs to the signing team (`TeamIdentifier`, `com.apple.developer.team-identifier`);
  * it authorizes this app: `com.apple.application-identifier` is `<TEAM>.<bundle id>` (or `<TEAM>.*`);
  * it allows the keychain access group `<TEAM>.<bundle id>` (exactly, or the `<TEAM>.*` wildcard);
  * it is not expired and does not expire within `--min-days` (default 90);
  * when `--cert-sha1` is given, the signing certificate is one of the profile's certificates.
"""
import argparse
import datetime
import hashlib
import plistlib
import re
import subprocess
import sys


def fail(message: str, code: int = 2):
    print(f"check-provisioning-profile: {message}", file=sys.stderr)
    sys.exit(code)


def load(args) -> dict:
    if args.decoded:
        try:
            with open(args.decoded, "rb") as handle:
                return plistlib.load(handle)
        except (OSError, plistlib.InvalidFileException, ValueError) as error:
            fail(f"cannot read the decoded profile {args.decoded}: {error}")
    try:
        decoded = subprocess.run(
            ["/usr/bin/security", "cms", "-D", "-i", args.profile],
            capture_output=True,
            check=True,
        ).stdout
    except FileNotFoundError:
        fail(f"profile not found, or /usr/bin/security missing: {args.profile}")
    except subprocess.CalledProcessError as error:
        fail(f"{args.profile} is not a readable provisioning profile ({error.stderr.decode(errors='replace').strip()[:200]})")
    try:
        return plistlib.loads(decoded)
    except (plistlib.InvalidFileException, ValueError) as error:
        fail(f"the decoded profile is not a property list: {error}")


def aware(moment: datetime.datetime) -> datetime.datetime:
    return moment if moment.tzinfo else moment.replace(tzinfo=datetime.timezone.utc)


def check(profile: dict, bundle_id: str, team: str, min_days: int, now: datetime.datetime, cert_sha1):
    problems = []
    entitlements = profile.get("Entitlements") or {}
    app_id = f"{team}.{bundle_id}"
    wildcard = f"{team}.*"

    platforms = [str(p) for p in (profile.get("Platform") or [])]
    if not platforms:
        problems.append("has no Platform key: cannot tell it is a macOS profile")
    elif not {"OSX", "macOS"} & set(platforms):
        problems.append(f"not a macOS profile (Platform: {platforms})")
    if profile.get("ProvisionedDevices"):
        problems.append("lists provisioned devices: a development or ad-hoc profile, not Developer ID")
    if entitlements.get("get-task-allow"):
        problems.append("has get-task-allow: a development profile, not Developer ID")

    teams = [str(t) for t in (profile.get("TeamIdentifier") or [])]
    if team not in teams:
        problems.append(f"belongs to team {teams or 'none'}, not the signing team {team}")
    declared_team = entitlements.get("com.apple.developer.team-identifier")
    if declared_team is not None and declared_team != team:
        problems.append(f"declares team-identifier {declared_team}, not {team}")

    application_identifier = entitlements.get("com.apple.application-identifier")
    if application_identifier not in (app_id, wildcard):
        problems.append(f"application-identifier is {application_identifier!r}, expected {app_id!r}")

    groups = [str(g) for g in (entitlements.get("keychain-access-groups") or [])]
    if app_id not in groups and wildcard not in groups:
        problems.append(f"keychain-access-groups {groups} do not allow {app_id!r}")

    expiry = profile.get("ExpirationDate")
    expiry_text = "unknown"
    if not isinstance(expiry, datetime.datetime):
        problems.append("has no ExpirationDate")
    else:
        expiry = aware(expiry)
        days_left = (expiry - now).days
        expiry_text = f"{expiry.date().isoformat()} ({days_left} days)"
        if expiry <= now:
            problems.append(f"EXPIRED on {expiry.date().isoformat()}")
        elif days_left < min_days:
            problems.append(f"expires {expiry.date().isoformat()}, in {days_left} days: renew it (the minimum is {min_days} days)")

    if cert_sha1:
        wanted = cert_sha1.replace(":", "").lower()
        have = [hashlib.sha1(bytes(c)).hexdigest() for c in (profile.get("DeveloperCertificates") or [])]
        if wanted not in have:
            problems.append("the signing certificate is not one of the profile's certificates")

    return problems, expiry_text


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--profile", help="the CMS-signed provisioning profile")
    source.add_argument("--decoded", help="an already decoded plist (tests)")
    parser.add_argument("--bundle-id", required=True)
    parser.add_argument("--team-id", required=True)
    parser.add_argument("--min-days", type=int, default=90)
    parser.add_argument("--cert-sha1")
    parser.add_argument("--now")
    args = parser.parse_args()

    if not re.fullmatch(r"[A-Z0-9]{10}", args.team_id):
        fail(f"--team-id must be a 10-character Apple team id, got {args.team_id!r}")
    now = aware(datetime.datetime.fromisoformat(args.now)) if args.now else datetime.datetime.now(datetime.timezone.utc)

    profile = load(args)
    problems, expiry_text = check(profile, args.bundle_id, args.team_id, args.min_days, now, args.cert_sha1)
    name = profile.get("Name", "unnamed")
    if problems:
        print(f"check-provisioning-profile: FAILED for profile {name!r}:", file=sys.stderr)
        for problem in problems:
            print(f"  - {problem}", file=sys.stderr)
        sys.exit(1)
    print(f"check-provisioning-profile: ok - {name!r}, team {args.team_id}, app {args.team_id}.{args.bundle_id}, expires {expiry_text}")


if __name__ == "__main__":
    main()
