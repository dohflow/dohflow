# ADR 0046 — Recurring-suggestion dismissal and suppression

- Status: Accepted
- Date: 2026-07-07
- Bead: personal-cfo-4d8.24.6 (dismiss + suppress recurring suggestions) — 2026-07-07 Transactions wave (epic 4d8.24)
- Builds on: ADR 0014 §7 (Money Inbox / review — dismissal keeps an item hidden across rebuilds), ADR 0018 (non-advice: the app suggests, never nags), personal-cfo-98ql (recurring detection), personal-cfo-5n4.8 (durable merchant-key dedupe)

## Context

Recurring detection (98ql) surfaces candidate bills from realized outflows; the user can promote one to
a real bill (which then excludes it, name- and merchant-key-based, 5n4.8). But the only action was
**promote** — there was no way to say *"this is not a recurring bill, stop suggesting it."* A merchant the
user has deliberately decided is not a bill (an annual membership they cancel and re-buy, a coincidentally
regular spend) keeps re-appearing every time detection runs. That is exactly the "nagging" ADR 0018 forbids.

Dismissal must be **durable but not permanent-by-fiat**: a dismissed suggestion should stay gone, yet a
*materially different* pattern later (the amount jumps, or the cadence changes) is genuinely new information
and should be allowed to re-surface — otherwise a one-time dismissal would blind the user to a real change
(e.g. a subscription that doubled in price).

## Decision

**1. Dismissal records a suppression keyed on `(merchant_key, currency)`.** A `DismissRecurringSuggestion`
sealed kernel command upserts a row into a new `recurring_suggestion_suppressions` table capturing the
**dismissed amount**, **frequency token**, a **timestamp**, and an optional **reason**. The key is
`(merchant_key, currency)` — the same identity detection groups on — so it is stable across the merchant's
future occurrences and independent of any bill. Upsert is **latest-dismiss-wins** (idempotent): re-dismissing
updates the recorded amount/frequency/timestamp.

**2. A suppressed candidate is excluded from suggestions — unless the pattern materially changed.**
`recurring_candidates` drops a candidate whose `(merchant_key, currency)` is suppressed, **except** when:

- the candidate's **inferred cadence differs** from the dismissed frequency token (e.g. dismissed monthly,
  now weekly), **or**
- the candidate's **amount moves outside the detector's amount band** around the dismissed amount.

The band is the detector's **own** `amount_band_minor` (`AMOUNT_BAND_BPS` = 10%, floor $5) — reused, not
re-invented, so "materially different amount" means the same thing here as it does inside detection. A
within-band amount drift or an unchanged cadence keeps the suppression in force.

**3. This composes with the two existing exclusion sources.** The detection `retain()` now filters on three
keyed sets: active bill **names** (98ql), active bill **`source_merchant_key`** (5n4.8), and **suppressions**
(this ADR, with the material-change carve-out). All three key on the normalized merchant key.

## Consequences

- Dismissal is a first-class review action alongside promote, satisfying ADR 0018 (no nagging) while ADR
  0014 §7's "hidden across rebuilds" durability now extends to recurring suggestions.
- The re-surface carve-out means a dismissal is not a permanent mute: a price change beyond ±10% (or the
  $5 floor) or a cadence change re-offers the suggestion once — the user can dismiss again (latest-wins).
- Suppression rows are keyed on `(merchant_key, currency)`, not a bill, so they persist independently and
  never block a legitimately-different future merchant of the same name (currency disambiguates; the amount/
  cadence carve-out handles the rest).
- The table is small (one row per dismissed merchant) and additive; no read-model rebuild.

## Alternatives considered

- **Permanent mute (no re-surface).** Rejected — blinds the user to a materially changed pattern (a doubled
  subscription price), which is real new information, not noise.
- **Time-boxed suppression (re-surface after N months).** Rejected for v1 — a fixed timer re-nags on an
  unchanged pattern (the ADR 0018 problem) and adds a clock dependency; the material-change rule is both
  quieter and more informative. A timer remains a possible future refinement.
- **Key on the bill / a created entity.** Rejected — dismissal explicitly means "*not* a bill," so there is
  no entity to hang it on; the merchant identity is the durable key.
