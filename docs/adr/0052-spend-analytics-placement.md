# ADR 0052 — Spend-by-category analytics live on Transactions

- **Status:** Accepted
- **Date:** 2026-07-31
- **Beads:** `personal-cfo-4d8.27.8.5` (this decision), `4d8.27.8.2` (the read model),
  `4d8.27.8.3` (the table), `4d8.27.8.4` (the drill-down viz)
- **Extends:** ADR 0037 §2 (where each analytic lives), ADR 0049 §1 (the nav groups)
- **Constrained by:** ADR 0018 (descriptive, never prescriptive)

## Context

The 2026-07-13 feedback asked for spend visualizations that are *"value-adding and
interrogatable"* — concretely: a category spend breakdown where **clicking a cell opens a
side panel of the transactions behind it**.

That interaction decides more than it first appears. A chart you can click *into* is not a
decoration on some other page; it is an index into a list of transactions. So the question
"where does the chart live?" is really "what does clicking a cell navigate to?".

The repo has answered a version of this before and the answers conflicted (top-level
sections, dashboard widgets, dashboard profiles, per-role sub-pages). ADR 0037 settled it
for cross-account analytics with a principle worth reusing rather than relitigating:

> analytics live **with the entity they describe**; the Dashboard keeps only summary
> widgets that **link into** them — "a glance + a jump, not the analytics home."

## Decision

### 1. Spend-by-category analytics live on the Transactions surface

Spend analytics describe **transactions**, so by ADR 0037's rule they belong on
Transactions — the same principle that put the debt burndown with debt and the net-worth
timeline at the Accounts level.

It is also the only placement where the owner's actual ask works without a detour: the
drill-down target *is* the transaction list. Charting on the Dashboard and listing on
Transactions would make every interrogation a cross-surface navigation, and the user would
have to trust that the rows they land on are the ones the cell counted.

Rejected:

- **Dashboard.** It is a glance surface, and it is already crowded — `4d8.27.2` just
  shrank Forecast Readiness to a compact row *because* it dominated a screen it only
  annotates. Hosting an interrogatable chart there repeats that mistake. The Dashboard may
  later carry a small linking widget, per ADR 0037 §2.
- **A new "Insights" destination.** A sixth answer to a question that already has five,
  and it separates the chart from the rows it indexes. If cross-entity analytics ever
  outgrow their entities, that is the moment to revisit — not before.

### 2. The chart and the list share one query

The chart reflects the **same filter state as the list beneath it**: same date range, same
account/category/tag filters, same search. One dataset, two views of it.

The alternative — a chart that always shows "everything" above a filtered list — is
incoherent the moment you click a cell: the totals would count transactions the list is
excluding, and the drill-down would disagree with the chart that produced it. Sharing the
query makes the chart answer a question the user can state exactly: *"of what I'm
currently looking at, where did it go?"*

Clicking a cell therefore **adds a category filter** rather than opening a disconnected
panel — the same mechanism the user already has, driven from the chart. That keeps the
back-out obvious (clear the filter) and means the side panel is just the list, already
filtered.

For that promise to hold, the category filter has to mean the same thing on both sides,
and today it does not:

- **The filter must match a category's SUBTREE, not just the exact id.** The list
  currently filters on `tc.category_id = ?`, so filtering to "Food" excludes everything
  categorized "Food / Dining". A parent cell's rollup counts its children, so without this
  the drill-down would show fewer rows than the cell it came from. (This also silently
  affects the filter bar as it ships today — picking a parent already hides its children.)
- **The filter must reach split lines.** The list joins `transaction_categorizations`
  only, so a $200 trip split into Groceries $150 / Household $50 is invisible when
  filtering to Groceries — while §3 has the chart counting it. Filtering must also match a
  transaction whose *split lines* carry the category.

Both are corrections to the list's existing behaviour, not new chart-only concessions.
Until they land, a chart that shares the filter would contradict its own drill-down, so
they are prerequisites of the viz (`4d8.27.8.4`), not follow-ups.

### 3. Totals follow split lines, not the parent categorization

A split transaction carries **per-line categories** (`split_lines.category_id` +
`amount_minor`, ADR 0034 — one unchanged ledger transaction decomposed in a side table).
Any spend aggregate must therefore read split lines when present and fall back to the
transaction's own categorization otherwise. Attributing a split $200 Costco trip entirely
to one category would be wrong on exactly the transactions users bother to split — the
ones they cared enough to itemize.

### 4. What counts as spend, and over what range

**Only EXPENSE-typed categories are spend.** The taxonomy's `type` is the discriminator,
not the posting shape. The two-user-posting test alone is not enough: in an import-first
app the *dominant* transfer is a one-sided row — a "Credit Card Payment" the importer
matched by name — which carries a system counter-posting and so looks exactly like an
expense. Counting it would add phantom spending on top of the card's own charges, i.e. the
same money twice. In-app transfers are additionally excluded by their second user posting,
belt and braces, since a user can categorize anything.

**Income is not negative spend.** Without the type filter a categorized paycheck would
land as a negative total and drag any percentage-of-total the chart draws. Income
categories are simply out of scope for this surface.

**Refunds net within a category.** Aggregation deliberately does *not* filter on sign, so a
positive amount in an expense category reduces that category rather than being dropped — a
returned purchase leaves no phantom spending behind. That is safe precisely because the
scope is expense categories.

**Mixed currencies are scoped, not summed.** There is no offline FX in this app, so the
aggregate is restricted to the reporting currency rather than adding unlike amounts. This
is a filter the *list* does not apply, so a foreign-currency account's rows can appear in
the list while contributing nothing to the chart; the surface must say which currency it
is reporting rather than let the two silently disagree.

**The chart needs a bounded default range.** The list's filters default to *no* date bound
— fine for a paged list, wrong for an aggregate, which would silently chart all history on
first paint. The chart therefore defaults to a bounded recent window and **shows the range
it is charting**, adopting the list's dates once the user sets them. A chart and a list
that disagree about "when" is the same failure as disagreeing about "what".

### 5. Descriptive only (ADR 0018)

The surface states **magnitudes and changes**, never judgments, targets, or
recommendations.

- **Allowed:** "Dining: $842 in July." · "Groceries: $612, up $118 from June." ·
  "Restaurants is 14% of spending in this range."
- **Forbidden:** "You're overspending on dining." · "Consider cutting restaurants." ·
  "You should set a $400 dining budget."

Budgets and targets are a separate feature with their own decision; a comparison to a
*user-set* target is not what this surface does today, and framing a period-over-period
delta as a verdict would violate ADR 0018 as surely as saying "you should".

Enforcement is mechanical, per ADR 0037 §4: `copy-review.test.ts` extends its scanned
globs to `src/transactions`, so the advice-phrase guard covers this copy where it actually
lands.

## Consequences

- The Transactions surface grows a chart above its list; both read one filter state
  (`4d8.27.8.3`, `4d8.27.8.4`).
- The `spend_by_category` read model (`4d8.27.8.2`) ships taking the **date range and
  drill level** only. The remaining filter inputs (account, tag, search, reviewed) land
  with the chart in `4d8.27.8.4` — the chart must not ship before them, because §2's
  one-shared-query promise is what makes the drill-down trustworthy. Staged deliberately:
  the read model has no consumer until the chart exists, so there is nothing to drift
  against yet.
- `copy-review.test.ts` must scan `src/transactions` before spend copy ships there.
- The list's category filter becomes subtree-aware and split-aware (`4d8.27.8.3`), which
  fixes a pre-existing gap in the filter bar as well.
- ADR 0037 §2's rule is reaffirmed and extended, not replaced; its Debt placement was
  already superseded by ADR 0049 §5, which does not affect the principle used here.
