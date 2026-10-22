#!/usr/bin/env bash
#
# Decide whether a change touches dependencies or the dependency-audit setup
# (personal-cfo-md8h2.14). Reads changed file paths, one per line, on stdin.
#
#   exit 0 and print the matching paths  - at least one dependency or audit-config path changed
#   exit 1, print nothing                - none did
#
# CI uses this to make the advisory audits (cargo audit, pnpm audit) required only
# when a pull request or merge group could actually change what they see. The
# advisory database changes under us; a change that touches no dependency must not be
# blocked by an advisory that appeared after it was written (owner decision
# personal-cfo-915.15, option B). Secrets scanning does not use this: it always runs.
#
# Matched paths: any Cargo.toml / Cargo.lock / package.json / pnpm-lock.yaml /
# pnpm-workspace.yaml / .npmrc / deny.toml / audit.toml at any depth, and the audit wrapper
# and its tests. Everything else, including .github/workflows/ci.yml, is a
# non-match: a workflow edit is exercised by the full run on the push to main and on
# the merge queue.

set -uo pipefail

PATTERN='(^|/)(Cargo\.toml|Cargo\.lock|package\.json|pnpm-lock\.yaml|pnpm-workspace\.yaml|\.npmrc|deny\.toml|audit\.toml)$|^scripts/cargo-audit\.py$|^scripts/tests/cargo-audit\.test\.sh$|^scripts/dependency-change-check\.sh$|^scripts/advisory-scans-required\.sh$|^scripts/tests/(dependency-change-check|advisory-scans-required)\.test\.sh$'

matches="$(grep -E -- "$PATTERN" || true)"
if [ -n "$matches" ]; then
  printf '%s\n' "$matches"
  exit 0
fi
exit 1
