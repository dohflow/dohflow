#!/usr/bin/env python3
"""Check the entitlements a SIGNED app actually carries (personal-cfo-md8h2.17).

Reads the XML plist `codesign -d --entitlements :- <app>` prints, on stdin, and requires EXACTLY:

  com.apple.application-identifier  = <TEAM>.<bundle id>
  com.apple.developer.team-identifier = <TEAM>
  keychain-access-groups            = [<TEAM>.<bundle id>]

and nothing else ("nothing broader": no wildcard group, no other entitlement). release.sh runs it on
the built app, so a build that signed with the wrong, an extra, or no entitlements cannot ship.

Usage: codesign -d --entitlements :- APP | check-signed-entitlements.py --bundle-id ID --team-id TEAM
"""
import argparse
import plistlib
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--bundle-id", required=True)
    parser.add_argument("--team-id", required=True)
    args = parser.parse_args()
    raw = sys.stdin.buffer.read()
    start = raw.find(b"<?xml")
    if start < 0:
        print("check-signed-entitlements: FAILED - the app carries no entitlements plist", file=sys.stderr)
        sys.exit(1)
    try:
        found = plistlib.loads(raw[start:])
    except (plistlib.InvalidFileException, ValueError) as error:
        print(f"check-signed-entitlements: FAILED - unreadable entitlements: {error}", file=sys.stderr)
        sys.exit(1)
    app_id = f"{args.team_id}.{args.bundle_id}"
    expected = {
        "com.apple.application-identifier": app_id,
        "com.apple.developer.team-identifier": args.team_id,
        "keychain-access-groups": [app_id],
    }
    problems = []
    for key, value in expected.items():
        if found.get(key) != value:
            problems.append(f"{key} is {found.get(key)!r}, expected {value!r}")
    for key in sorted(set(found) - set(expected)):
        problems.append(f"unexpected entitlement {key} = {found[key]!r} (only the three keychain/identity keys are allowed)")
    if problems:
        print("check-signed-entitlements: FAILED:", file=sys.stderr)
        for problem in problems:
            print(f"  - {problem}", file=sys.stderr)
        sys.exit(1)
    print(f"check-signed-entitlements: ok - exactly the keychain group {app_id}")


if __name__ == "__main__":
    main()
