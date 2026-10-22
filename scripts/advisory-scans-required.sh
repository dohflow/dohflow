#!/usr/bin/env bash
#
# Decide whether the advisory scans (cargo audit, pnpm audit) are required for a
# change (personal-cfo-md8h2.14). Reads the changed file paths on stdin and writes
# exactly one line to stdout, for $GITHUB_OUTPUT:
#
#   advisory=true    a dependency or audit-config path changed (or its check said so)
#   advisory=false   none did
#
# It FAILS CLOSED. scripts/dependency-change-check.sh answers 0 (changed) or 1 (not
# changed); ANY other result (the script is missing, broken, or crashed) is an error
# here, exit 2 with nothing on stdout, so a change that deletes or breaks the checker
# can never turn the audits off. Messages go to stderr.
#
# DEPENDENCY_CHECK_SCRIPT overrides the checker's path (tests only).

set -uo pipefail

CHECK="${DEPENDENCY_CHECK_SCRIPT:-$(dirname "${BASH_SOURCE[0]}")/dependency-change-check.sh}"

changed="$(cat)"
rc=0
matches="$(printf '%s\n' "$changed" | bash "$CHECK")" || rc=$?

case "$rc" in
  0)
    echo "Dependency or audit-config files changed, so the advisory scans are required:" >&2
    printf '%s\n' "$matches" >&2
    echo "advisory=true"
    ;;
  1)
    echo "No dependency or audit-config change." >&2
    echo "advisory=false"
    ;;
  *)
    echo "::error::scripts/dependency-change-check.sh exited $rc (expected 0 or 1); failing closed so the advisory scans cannot be skipped by a broken checker." >&2
    exit 2
    ;;
esac
