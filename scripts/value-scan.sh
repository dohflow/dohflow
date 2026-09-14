#!/usr/bin/env bash
#
# The "o1nxk value scan" referenced by docs/operations/public-launch-snapshot.md
# step 4 ("the o1nxk value scan (real financial figures, bank/broker patterns)
# — see that bead for the exact scan command and pattern list"). This IS that
# command (personal-cfo-o1nxk item 10/11).
#
# gitleaks (run separately, see docs/security/scanning.md) catches
# credential-shaped strings. This catches a different, complementary risk:
# REAL financial content accidentally committed — a real bank/brokerage name,
# a real-looking SSN, or a real-looking full card number — none of which are
# secrets in gitleaks' sense, but all of which would be a privacy incident if
# they ever reached a public repo (this project's own demo/seed data is
# entirely invented on purpose — docs/agent/demo-vault.md: "Every name is
# invented... No real brand, bank...").
#
# Scope: the git-TRACKED tree (matches what an export-ignore-respecting
# `git archive` — the actual go-live mechanism — would publish), not the
# working directory (which can hold local, gitignored junk this scan does
# not need to care about).
#
# KNOWN LIMITATION: this is a text/grep scan. It cannot inspect the content of
# binary files (PNG screenshots, PDFs). The n76x.13 screenshot set's privacy
# review was a separate, manual pixel-by-pixel inspection (see that bead and
# dohflow-site/docs/screenshots.md's "Privacy check" section) — this script
# does not replace that, and does not re-verify it.
#
# Usage: ./scripts/value-scan.sh [path-to-repo-checkout]
#   Defaults to the current directory. Exits 1 and prints every match if
#   anything is found; exits 0 (silent) if clean.
set -euo pipefail

repo="${1:-.}"
cd "$repo"

fail=0

# This script's own denylist necessarily NAMES every pattern it looks for,
# the review document that reports its results necessarily QUOTES the
# denylist and the exception rationale, and its own hermetic test
# necessarily PLANTS every one of those patterns as fixture content. All
# three would otherwise match themselves the moment they're committed — this
# happened TWICE in review (personal-cfo-o1nxk PR #418): first for the
# script+doc pair (round 1), then again for the test file alone (round 2,
# after the first fix excluded the first two but not the third) — both times
# because `git grep` cannot see untracked files, so a "clean" result recorded
# before the newest self-referential file was committed stopped being true
# the moment it was. None of the three is exempted from shipping (all are
# meant to be in the public tree) — only from this scan matching its own
# vocabulary. `*.lock` (Cargo.lock) is excluded for a different reason:
# SHA256 checksums are 64 hex characters, long enough to contain a 16-digit
# run by pure chance — machine-generated hash content, never narrative text,
# so it carries none of the privacy risk this scan exists to catch. Every
# `git grep` below uses this same pathspec.
excluded_paths=(
  ':!*.png' ':!*.jpg' ':!*.jpeg' ':!*.pdf' ':!*.lock'
  ':!scripts/value-scan.sh'
  ':!scripts/tests/value-scan.test.sh'
  ':!docs/security/release-review-v0.1.md'
)

# Runs one denylist pattern via `git grep`, drops any match whose FILE is a
# documented exception (an ERE alternation of exact paths, or "" for none),
# and reports+fails on whatever's left. Centralizes the same
# match-then-except-then-report shape all three checks below need, so
# there's exactly one place that logic can have a bug, not three.
#   check <label> <pattern> <exceptions-ere-or-"">
check() {
  local label="$1" pattern="$2" exceptions="$3" matches unexcepted
  matches="$(git grep -nIE "$pattern" -- . "${excluded_paths[@]}" 2>/dev/null || true)"
  [ -z "$matches" ] && return 0
  if [ -n "$exceptions" ]; then
    unexcepted="$(echo "$matches" | grep -vE "^($exceptions):" || true)"
  else
    unexcepted="$matches"
  fi
  if [ -n "$unexcepted" ]; then
    echo "$unexcepted"
    echo "FOUND: $label — see matches above" >&2
    fail=1
  fi
}

# `(^|[^0-9-])`/`([^0-9-]|$)` stand in for `\b` word boundaries, tightened to
# also reject a hyphen neighbor (not just a bare digit neighbor):
#   - git's grep engine does not support `\b` in -E mode at all (verified
#     directly — it compiles without error but silently never matches,
#     rather than failing loudly); `-P` (PCRE, which does support `\b`) is
#     not guaranteed available on every git build this runs against
#     (notably CI's), so this uses plain POSIX ERE character classes instead.
#   - Excluding a hyphen neighbor specifically (not just excluding another
#     digit) is what keeps this from matching a substring of a longer
#     hyphenated UUID (this codebase's real fixture ID scheme, e.g.
#     "0190a000-0000-7000-8000-000000000001") — a UUID's own internal
#     "-NNNN-NNNN-NNNN-" run is exactly 4-4-4 digit-grouped and would
#     otherwise false-positive as a truncated card-number shape. A real
#     card number is never itself embedded inside a longer hyphenated ID.
SSN_RE='(^|[^0-9-])[0-9]{3}-[0-9]{2}-[0-9]{4}([^0-9-]|$)'
CARD_RE='(^|[^0-9-])[0-9]{4}[- ]?[0-9]{4}[- ]?[0-9]{4}[- ]?[0-9]{4}([^0-9-]|$)'

# --- Real institution names -------------------------------------------------
# The demo vault's own fictional set (Saltmarsh CU, Kestrel, Copperleaf,
# Tidepool, Quillfeather Finance, Foxglove Home Lending, Ledgerline Systems,
# Northhollow Benefits, Larkspur Brokerage, and similar invented names) is
# NOT on this list by design — this list is real, well-known US banks and
# brokerages that should never appear in this codebase's tracked content.
real_institutions=(
  "chase\.com" "wellsfargo" "bankofamerica" "citibank" "citi\.com"
  "schwab\.com" "fidelity\.com" "vanguard\.com" "usaa\.com"
  "navyfederal" "capitalone" "discover\.com" "americanexpress" "amex\.com"
  "ally\.com" "pnc\.com" "usbank\.com" "truist\.com" "regions\.com"
  "synchronybank" "marcus\.com" "sofi\.com"
)
# Documented exception: a real institution NAME referenced purely to
# describe a CSV export FORMAT for import compatibility (never accompanied
# by real account numbers, balances, or personal data) is not a privacy
# leak — it's the importer documenting what it parses. Verified by hand
# before excepting; do not add here without checking the actual context.
#   - "capitalone": crates/importers/csv-importer/src/lib.rs,
#     crates/db-worker/src/migrations.rs, crates/importers/importer-core/src/lib.rs,
#     docs/adr/0045-ingestion-field-capture-and-dual-date.md — all describe
#     CapitalOne's real CSV column shape (personal-cfo-4d8.24.1, ADR 0045)
#     using entirely synthetic fixture data (placeholder "Card No. 1234",
#     "Coffee Shop"/"Paycheck" labels, fictional dates).
institution_exceptions='crates/importers/csv-importer/src/lib.rs|crates/db-worker/src/migrations.rs|crates/importers/importer-core/src/lib.rs|docs/adr/0045-ingestion-field-capture-and-dual-date.md'

for pat in "${real_institutions[@]}"; do
  check "real institution pattern '$pat'" "$pat" "$institution_exceptions"
done

# Documented exception, SSN/card checks only: the redaction layer's own test
# corpus deliberately embeds fake card-number-shaped strings (the canonical
# "4111111111111111" test Visa number and similar) to prove they get
# scrubbed from logs — the exact same files `.gitleaks.toml` already
# allowlists for this exact reason (personal-cfo-zobt), plus one more this
# scan's broader digit-shape heuristic catches that gitleaks' own
# credential-pattern rules do not:
#   - crates/observability/src/lib.rs, crates/finance-kernel/tests/log_redaction.rs
#     — .gitleaks.toml's own allowlist, same rationale.
#   - crates/finance-kernel/tests/side_file_leak.rs — a plaintext-leak-canary
#     test with its own obviously-fake sentinel constants (e.g.
#     "ACME-SENTINEL-INC-5999000099990000"), not gitleaks-allowlisted
#     because gitleaks' credential-shaped rules don't happen to trigger on
#     it, but this scan's card-shape heuristic does.
value_fixture_exceptions='crates/observability/src/lib.rs|crates/finance-kernel/tests/log_redaction.rs|crates/finance-kernel/tests/side_file_leak.rs'

check "SSN-shaped string" "$SSN_RE" "$value_fixture_exceptions"
check "card-number-shaped string" "$CARD_RE" "$value_fixture_exceptions"

if [ "$fail" -eq 0 ]; then
  echo "value-scan: clean — no real institution names, SSN-shaped, or card-number-shaped strings in the tracked tree."
fi
exit "$fail"
