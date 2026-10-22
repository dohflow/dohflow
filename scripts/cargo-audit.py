#!/usr/bin/env python3
"""Run `cargo audit` over both Cargo workspaces, with ONE scoped, dated exception.

CI used to run `cargo audit` and `cargo audit -f apps/desktop/src-tauri/Cargo.lock` directly.
`cargo audit`'s own ignore list is by advisory id only: ignoring RUSTSEC-2026-0381 there would
also hide it on the real, patched-or-not `webpki-roots` 1.x crate. This wrapper keeps the
default behavior (any vulnerability fails) and allows exactly the (advisory, crate, version)
triples listed in EXCEPTIONS, each with a reason, an owner decision and an expiry. It also
fails when an exception expires or matches nothing, so the exception cannot outlive its cause.

Usage:
  python3 -I scripts/cargo-audit.py                      # runs cargo audit on both lockfiles
  python3 -I scripts/cargo-audit.py --json-file F ...    # test mode: check these cargo-audit
                                                         # --json outputs instead of running it
  --today YYYY-MM-DD                                     # test mode: override the date
  --exceptions-file F                                    # test mode: use the exceptions in this
                                                         # JSON list instead of EXCEPTIONS
"""

import argparse
import datetime
import json
import subprocess
import sys

LOCKFILES = [None, "apps/desktop/src-tauri/Cargo.lock"]  # None = the root workspace

# (advisory id, crate, version) -> reason. Every entry carries an expiry and the bead that
# removes it. An exception is NEVER matched by advisory id alone. There are none today.
#
# History: RUSTSEC-2026-0381 on webpki-roots 0.26.11 (a re-export shim of the fixed
# webpki-roots 1.0.10; owner decision 2026-10-20, personal-cfo-md8h2.13) was allowed here until
# the advisory database stopped flagging the shim (2026-10-21); the script then failed as
# designed ("matches nothing") and the entry was removed. personal-cfo-pxi.23 (ureq 2 -> 3)
# still drops the shim itself. To add an exception, give it all of: id, crate, version,
# expires (YYYY-MM-DD), removed_by (a bead), decision, reason.
EXCEPTIONS = []


def run_audit(lockfile):
    command = ["cargo", "audit", "--json"]
    if lockfile:
        command += ["-f", lockfile]
    # cargo audit exits non-zero when it finds a vulnerability but still prints the JSON.
    done = subprocess.run(command, capture_output=True, text=True, check=False)
    return done.stdout, done.returncode, done.stderr


def findings(text):
    """The (id, crate, version) of every vulnerability in one `cargo audit --json` output."""
    report = json.loads(text)
    return [
        (item["advisory"]["id"], item["package"]["name"], item["package"]["version"])
        for item in report["vulnerabilities"]["list"]
    ]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--json-file", action="append", default=[])
    parser.add_argument("--today", default=None)
    parser.add_argument("--exceptions-file", default=None)
    args = parser.parse_args()
    exceptions = EXCEPTIONS
    if args.exceptions_file:
        with open(args.exceptions_file, encoding="utf-8") as handle:
            exceptions = json.load(handle)
    today = datetime.date.fromisoformat(args.today) if args.today else datetime.date.today()

    outputs = []
    if args.json_file:
        for path in args.json_file:
            with open(path, encoding="utf-8") as handle:
                outputs.append((path, handle.read()))
    else:
        for lockfile in LOCKFILES:
            label = lockfile or "Cargo.lock"
            text, code, err = run_audit(lockfile)
            if not text.strip():
                print(f"cargo-audit: no JSON from `cargo audit` for {label} (exit {code}): {err.strip()}")
                return 2
            outputs.append((label, text))

    allowed = {(e["id"], e["crate"], e["version"]): e for e in exceptions}
    used = set()
    problems = []
    for label, text in outputs:
        try:
            found = findings(text)
        except (ValueError, KeyError, TypeError) as error:
            print(f"cargo-audit: could not read cargo audit's output for {label}: {error}")
            return 2
        for triple in found:
            exception = allowed.get(triple)
            if exception is None:
                problems.append(f"{label}: {triple[0]} on {triple[1]} {triple[2]} (no exception)")
            else:
                used.add(triple)
                print(
                    f"cargo-audit: {label}: {triple[0]} on {triple[1]} {triple[2]} allowed by "
                    f"exception until {exception['expires']} (removed by {exception['removed_by']})"
                )

    for triple, exception in allowed.items():
        if datetime.date.fromisoformat(exception["expires"]) < today:
            problems.append(
                f"exception {triple[0]} on {triple[1]} {triple[2]} expired {exception['expires']} "
                f"(removal bead {exception['removed_by']})"
            )
        elif triple not in used:
            problems.append(
                f"exception {triple[0]} on {triple[1]} {triple[2]} matches nothing now; remove it "
                f"from scripts/cargo-audit.py ({exception['removed_by']})"
            )

    if problems:
        print("cargo-audit: FAILED")
        for problem in problems:
            print(f"  - {problem}")
        return 1
    print(f"cargo-audit: ok across {len(outputs)} lockfile(s)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
