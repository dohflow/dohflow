# ADR 0047: Recurring detection→promotion semantics — retro-attach, series consumption, anchors, provenance

- **Status:** Accepted
- **Date:** 2026-07-10
- **Deciders:** Project owner (2026-07-09 dogfooding feedback)
- **Beads:** [`personal-cfo-4d8.25.7`](../../.beads/issues.jsonl) (this ADR); implemented by
  `4d8.25.8` (retro-attach), `4d8.25.9` (variant suppression), `4d8.25.10` (anchor
  unification); `4d8.25.11` implements §4 (provenance surface + autopay prefill).
- **Builds on:** ADR 0026 (§16–17 instance projection/actualization + the 2026-07-08
  drift-tolerant matching addendum), ADR 0046 (dismissal suppression), ADR 0041 (autopay is
  intent, never a projection change), ADR 0032 (explicit review resolution), ADR 0030
  (merchant identity/grouping, `5n4.4` anchored containment), ADR 0021 (date discipline).

## Context

Detection (`98ql`) shipped without an ADR; only dismissal (0046) and instance matching
(0026 §16–17) are decided. Owner dogfooding 2026-07-09: *"I keep approving a recurring bill
item but then other items of the same exact transaction are still being recommended… when a
recurring bill is added we should more accurately identify all the other transactions that
fit that schema and add them automatically (or allow users to review them as part of it)"* —
plus a direct question about what the anchor date means. Root causes found:

1. Approving a candidate just creates the bill — **no retroactive linking**, no inbox
   effect, and candidate consumption keyed on a single exact merchant key, so import
   spelling variants (`SQ *SEVEN SEAS ROASTING C` vs a truncated or located variant) keep
   resurfacing.
2. The instance-linking seam ([`recurring_instances.rs`]) reads **liquid postings only** — a
   bill charged to a credit card can never link its historical card postings.
3. The three creation paths set **three different anchors**: candidate-promote uses the
   *next expected* date, make-recurring-from-transaction uses the *transaction's* date,
   manual Add-bill defaults to *today* — and nothing explains what the anchor means.

## Decision

### 1. Approving a bill retro-attaches its history (the instance seam is the one matcher)

Creating a recurring bill — via candidate promotion, make-recurring-from-transaction, or
manual entry — retro-links matching historical transactions through the **existing**
drift-tolerant instance matcher (same sign, ±7-day shared window, amount-band OR
normalized-payee, payee-first ranking; 0026 addendum 2026-07-08). No second matcher is
introduced. Two extensions to the seam:

- **Card postings are linkable.** Realized-posting collection extends from `liquid_cash` to
  include `credit_facility`, each posting carrying its account. An **account gate** keeps
  precision: an obligation whose `autopay_account_id` is set links only postings from that
  account; an obligation without one links liquid postings only (today's behavior); income
  occurrences link liquid postings only. This is what lets a card-charged subscription
  attach its card history.
- **The approve flow refreshes the seam immediately** (the rebuild is idempotent and
  deterministic), so the UI can report *"matched N past transactions"* right after approval
  instead of waiting for the daily actualization run.

**Inbox effect (ADR 0032 boundary):** retro-attachment never silently changes review state.
The UI reports the matched count and offers an explicit one-click *"mark N reviewed"*; the
user stays in control (the owner's "or allow users to review them as part of it").

### 2. Series consumption — suppression extends beyond the exact key, precision-first

A candidate is consumed (not re-suggested) when it matches a tracked active bill by, in
order of trust:

1. **Exact key** — the bill's normalized name or persisted `source_merchant_key`
   (unchanged, `5n4.8`).
2. **Evidence keys** — the normalized payee keys of transactions actually **linked to the
   bill's instances** (§1). Whatever spelling the imports used, once a posting links, its
   key suppresses future candidates for the same spelling. This is the durable fix: it
   grows with the data and cannot false-merge (the link already passed the matcher).
3. **Anchored grouping** — `same_merchant`/`brand_anchor` (ADR 0030 / `5n4.4` anchored
   containment: location-variant keys of one distinctive brand), plus a conservative
   **truncation rule** for bank-truncated descriptors: two keys conflict when one is a
   word-boundary prefix of the other AND the shorter side is multi-token AND ≥ 10
   characters (`SEVEN SEAS ROASTING` ↔ `SEVEN SEAS ROASTING C`; never `NEW YORK LIFE` ↔
   `NEW YORK PIZZA`, never single tokens). False merges remain the cardinal sin; anything
   uncertain stays a visible suggestion.

Dismissal suppression (ADR 0046) is **unchanged** — it stays keyed to the exact
`(merchant_key, currency)` with the material-change re-surface carve-out. The extended
matching above applies only to *tracked-bill* consumption, where a wrong suppression is
recoverable (the bill's own surface shows the series).

### 3. One anchor meaning: "a real occurrence the cadence counts from"

The anchor is **a date the cadence is measured from — ideally a real observed occurrence**.
`PaySchedule` expands the lattice in both directions, and the displayed next-due is derived
by rolling forward from today, so any occurrence of the series is an equivalent anchor
(shift-invariance is property-tested; one caveat: a month-end **clamped** occurrence — a
day-29+ schedule observed on Feb 28 — carries the clamped day-of-month, so a month-stepped
series re-anchored on it shifts by the clamp. Rare, self-correcting within the ±7-day match
window, and v1 accepts it; once per-observation provenance ships (`4d8.25.11`) the promote
path should prefer the most recent *unclamped* observation). Unified defaults:

- **Candidate promote:** `last_seen` (the most recent *observed* charge) — was the
  synthetic next-expected date; a real occurrence is better provenance and identical
  arithmetic.
- **Make-recurring-from-transaction:** that transaction's date (already correct).
- **Manual Add bill:** today, explicitly labeled.

All three forms carry the same help text: *"The schedule counts from this date — past and
future occurrences derive from it. Any date the bill actually happened works."* The stored
`next_expected_date` column keeps holding the anchor (a rename is deliberately deferred —
display derives from it, nothing reads it as "next due").

### 4. Provenance + autopay prefill (recorded here, built in `4d8.25.11`)

The candidate surface shows its evidence: occurrence count, typical day-of-cycle, amount
band, and source account(s) — derived from the detector's observations, which must
propagate per-observation account ids and dates. On promote, when the observed series is
**single-account**, `autopay_account_id` (and the paying-source display) prefill from it;
a multi-account series prefills nothing. Per ADR 0041, autopay stays a label — prefill
never changes projection behavior.

## Consequences

- Approving a candidate now visibly consumes the series: history attaches, the count is
  reported, review is one explicit click, and re-suggestion stops even across spelling
  variants — the trust loop the owner asked for.
- The account gate makes instance links *more* precise for autopay bills (a liquid bill
  can no longer steal a same-window card posting and vice versa) — actualization (`46jq`)
  and readiness inherit that precision.
- Evidence-based suppression means the exclusion list grows from real links, not string
  heuristics; the heuristics (§2.3) only bridge the gap before links exist.
- A promoted bill's anchor becomes a past date; anything displaying the raw anchor must
  derive next-due (already the case).

## Alternatives considered

- **Auto-mark retro-attached inbox items reviewed.** Rejected: ADR 0032 makes review an
  explicit act; silent bulk state changes are how trust is lost. One explicit click keeps
  the speed without the surprise.
- **A separate retro-match pass at approve time.** Rejected: a second matcher would drift
  from the instance seam's tolerances (the 4d8.24.4 lesson — one shared window constant).
- **Fuzzy string similarity for variant suppression.** Rejected (again — `5n4.4` judge):
  anchored containment + evidence keys give recall without the false-merge hole.
- **Renaming `next_expected_date` → `anchor_date`.** Deferred: a schema rename touching
  five consumers for zero behavior change; tracked as cleanup, not done here.

## Relates to

ADR 0026 (§16–17, drift addendum), ADR 0046, ADR 0041, ADR 0032, ADR 0030 (+`5n4.4`
grouping), ADR 0021. Beads: `98ql`, `5n4.8`, `4d8.24.4`–`.6`, `vn6b`, `46jq`;
implementation `4d8.25.8`–`.11`.
