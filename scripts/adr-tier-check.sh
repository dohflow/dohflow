#!/usr/bin/env bash
#
# ADR 0082's tier tripwire (personal-cfo-0uxwt). Business-tier content —
# pricing, charging, commission economics, revenue figures — must never enter
# docs/adr/ or docs/research/ in the PUBLIC repo; it belongs in the private
# dohflow/internal repository instead (ADR 0082, decisions 2-3). This is a
# fixed phrase list, not a redaction layer: it cannot see intent, only
# phrases, so it complements ADR 0082's two-tier repository split rather than
# replacing it. A future public STUB ADR (ADR 0082, decision 3) that
# legitimately uses this vocabulary is added to `exceptions` below when it's
# written, the same way every exception here was added: verified by hand
# first, never guessed.
#
# Scope: the git-TRACKED tree, matching what a contributor's clone actually
# publishes — same rationale as scripts/value-scan.sh, whose check()/
# exceptions shape this script reuses directly.
#
# Usage: ./scripts/adr-tier-check.sh [path-to-repo-checkout]
#   Defaults to the current directory. Exits 1 and prints every match if
#   anything is found; exits 0 (silent) if clean.
set -euo pipefail

repo="${1:-.}"
cd "$repo"

fail=0

# Same self-reference problem scripts/value-scan.sh documents at length: this
# script, its own test, and the two ADRs that discuss the disclosure boundary
# and the business model in the abstract all necessarily NAME the phrase
# list. Excepted from matching their own vocabulary, not from the check
# existing.
excluded_paths=(
  ':!scripts/adr-tier-check.sh'
  ':!scripts/tests/adr-tier-check.test.sh'
  ':!docs/adr/0082-public-disclosure-boundary.md'
  ':!docs/adr/0066-business-model-free-app-paid-services.md'
)

# Runs one denylist pattern via `git grep`, scoped to docs/adr/ and
# docs/research/ only (ADR 0082's stated scope), drops any match whose FILE
# is a documented exception, and reports+fails on whatever's left. Identical
# shape to scripts/value-scan.sh's check() — same match-then-except-then-
# report logic, so there is exactly one place either can have a bug.
#   check <label> <pattern> <exceptions-ere-or-"">
check() {
  local label="$1" pattern="$2" exceptions="$3" matches unexcepted
  matches="$(git grep -nIE "$pattern" -- 'docs/adr/*' 'docs/research/*' "${excluded_paths[@]}" 2>/dev/null || true)"
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

# Verified by hand, 2026-09-16, against every match these patterns produce on
# the tree at that date (see personal-cfo-0uxwt's PR for the full dry-run
# output). Each is a false positive in a different sense:
#   - 0061 "per month": Cloudflare Workers' own free-tier build-minute quota,
#     not a DohFlow price.
#   - 0030 "Stripe": a third-party payment processor NAME recognized while
#     normalizing a transaction description, not DohFlow's own business
#     relationship with Stripe.
#   - 0028/0060 "price": investment/holding price-refresh scope and a
#     third-party connector's own repricing risk — neither is DohFlow pricing.
#   - 0046 "price" (×3): a user's OWN recurring bill/subscription changing
#     price is the feature this ADR describes detecting, not DohFlow's price.
#   - 0054 "price": colloquial ("the price of separating two colors"), not
#     money at all.
#   - 0007 "revenue": double-entry bookkeeping terminology (a ledger revenue
#     account), not DohFlow's own revenue.
#   - trademark-clearance-dossier.md "pricing": cites the site's own already-
#     public `/pricing` page (ADR 0066, allowlisted) by name, discloses
#     nothing new.
adr_exceptions='docs/adr/0061-website-stack-and-hosting\.md|docs/adr/0030-categorization\.md|docs/adr/0028-account-subtype-and-cash-tiers\.md|docs/adr/0060-connector-strategy-simplefin-first\.md|docs/adr/0046-recurring-suggestion-dismissal\.md|docs/adr/0054-categorical-chart-palette\.md|docs/adr/0007-ledger-transaction-posting-model\.md|docs/research/trademark-clearance-dossier\.md'

# ADR 0082's own phrase list, verbatim, checked one at a time against the
# same exceptions list — a file legitimately excepted for one phrase (e.g.
# "price") stays excepted for all of them, since these false-positive
# categories recur across the same handful of pre-existing files rather than
# needing a different exception set per phrase.
business_phrases=(
  '\$/month' 'per month' 'Stripe' '[Pp]rice' '[Pp]ricing' 'commission'
  'revenue' 'cost table' 'subscriber' 'invoice'
)

for pat in "${business_phrases[@]}"; do
  check "business phrase '$pat'" "$pat" "$adr_exceptions"
done

if [ "$fail" -eq 0 ]; then
  echo "adr-tier-check: clean — no business-tier phrases in docs/adr/ or docs/research/ outside documented exceptions."
fi
exit "$fail"
