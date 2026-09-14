# ADR 0057 — The Debt page's view contract

- **Status:** Accepted
- **Date:** 2026-08-06
- **Bead:** `personal-cfo-4d8.27.9.1` — the contract the rest of `4d8.27.9.*` builds on
- **Builds on:** ADR 0049 §5 (Debt is top-level; Accounts keeps identity and balances,
  Debt owns analysis) and §6 (the nav entry ships with the page)
- **Follows:** ADR 0052 §4 (mixed currencies are scoped, not summed), ADR 0018
  (descriptive, never prescriptive)

## Context

ADR 0049 §5 already promoted Debt to its own destination and said what it is *for*:
per-account depth — single **and** multi-account selection, per-debt-type visualizations,
a scoped transaction log, payoff tools — that could not live inside Accounts without
dominating it. What it did not settle is how the page behaves once you are on it.

That is four questions, and every remaining bead in the arc depends on the answers:

1. What does selecting accounts *mean*?
2. Which visualization does each debt type get?
3. What bounds the scoped transaction log?
4. What happens when a selection spans currencies?

The app has exactly two debt roles: `credit_facility` (cards) and `loan_liability`
(loans). They are genuinely different shapes — a card revolves and has statements, a loan
amortizes — which is what makes "which viz?" a real question rather than a styling one.

## Decision

### 1. Selection is a filter over the page, not a mode

The selector picks **one or many** debt accounts and **defaults to all of them**. The page
opens on "your debt", not on an empty state asking you to choose.

Everything below the selector is scoped by it. Selection does not switch the page between
a "single" layout and a "multi" layout — that would make the page's shape depend on how
many boxes are ticked, and force every downstream bead to handle two layouts.

**Consequence for the payoff tools:** they already operate over the household's whole
carry-debt set (`DebtPayoffCompare`). Under this contract they operate over the
*selection*, which for the default selection is the same thing.

### 2. Visualizations are chosen by account role, and compose per role

| Role | Gets |
| --- | --- |
| `credit_facility` (cards) | balance-over-time (see 2026-09-01 addendum below) |
| `loan_liability` (loans) | paydown / amortization over time |

A selection spanning both roles renders **both sections**, each over its own subset. It
does not attempt one chart for two shapes: a card's revolving balance and a loan's
amortizing balance on one axis would invite a comparison that means nothing.

Every piece of this already exists and is reused rather than rebuilt:
`AccountBalanceChart` (history + projection cone), `RankedBars` with
`spend_by_category`'s account facet (ADR 0052, shipped in `4d8.27.8.4`), and
`DebtPerDebtChart` / `DebtBurndownChart` for paydown.

> The heatmap deferred in `personal-cfo-azeb` is **not** needed here. That dependency was
> assumed when the visualization form was still open; the dataviz pass settled on ranked
> bars, and the bead's own acceptance criteria — "balance-over-time" and
> "spending-by-category breakdown" — are served by primitives that now ship.

**Addendum (2026-09-01, owner dogfooding `personal-cfo-emgh`).** The
standalone card spend-by-category breakdown was removed from this page: it
duplicated the selection-scoped "Where the money went" breakdown inside the
Activity section directly below it, and the Activity copy is the actionable
one (it drills into the log). Cards now get balance-over-time here, and the
per-category view lives solely in the Activity section. `spend_by_category`
itself is unchanged (the Transactions surface still consumes it).

### 3. The transaction log is bounded by the selection, and says so

The Debt page embeds the Transactions table scoped to the selected accounts, with the
**account facet locked**: the page's selector owns it, and the embedded filter bar must not
offer a second, contradictory way to change it.

Everything else about the list — search, category, tags, date range, sort, pagination —
stays available. The scope is a floor on what the list shows, not a replacement for
filtering within it.

This mirrors what ADR 0052 §2 established for the spend chart and its list: one filter
state, one answer, no surface where two controls disagree about what is being shown.

### 4. A mixed-currency selection is scoped, not summed

There is no offline FX in this app. When a selection spans currencies, the page **reports
in the reporting currency and names the accounts it is leaving out** — it does not add
unlike amounts, and it does not silently drop them.

This is ADR 0052 §4's rule applied to a second surface, deliberately: a household that
learns "this app scopes to one currency and tells you" on Transactions should not have to
relearn it on Debt. `DebtPayoffCompare` already formats interest in the base currency
(`personal-cfo-6wk.13`); this makes that a stated contract rather than an implementation
detail.

Blocking the selection was rejected — a user with a foreign-currency card should still be
able to see their other debts — and per-currency panels were rejected as a large amount of
structure for a case no owner need has yet described. Revisit if that changes.

## Consequences

- `4d8.27.9.2` (page shell) relocates `CreditCardsView` and `DebtPayoffCompare` out of
  Accounts. Accounts keeps account identity and balances per ADR 0049 §5, so this is a
  move, not a copy — no surface should show debt analysis twice.
- `4d8.27.9.3` (selector) defaults to all debt accounts and is the single owner of the
  account scope for everything below it.
- `4d8.27.9.4` remains the real backend work in this arc: the transaction filter model,
  its IPC input, and the db-worker SQL all take a single `account_id` today and must accept
  a **set**. `spend_by_category` took the same single-account facet in `4d8.27.8.4` and
  needs the same widening, or the card spending breakdown cannot honour a multi-selection.
- `4d8.27.9.5` embeds the list with the account facet locked.
- `4d8.27.9.6` is unblocked and needs no new primitive.
- `4d8.27.9.7` (payoff tools) scopes to the selection rather than the whole household.
- Copy on this page is descriptive (ADR 0018). It states balances, rates, and projected
  paydown; it does not rank debts as good or bad, and "avalanche/snowball" remain neutral
  labels as ADR 0018 already requires.
