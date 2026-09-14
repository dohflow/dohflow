# ADR 0053 — List-surface convergence: DataTable, ListCard, and what stays

- **Status:** Accepted
- **Date:** 2026-08-01
- **Beads:** `personal-cfo-4d8.27.4.4` (this decision), `4d8.27.4.2` (the primitive),
  `4d8.27.4.3` (the audit it rests on)
- **Governs:** `docs/agent/FRONTEND.md`'s rule against hand-rolling a table per view

## Context

The app grew one collection surface at a time, and each brought its own header, its own
empty state, and its own loading treatment. The surface audit
(`docs/design-system/surface-audit.md`) counted the result: six real tables, ~14 list
surfaces, and **one** consumer of the shared `Skeleton` primitive. All four screens this
bead names — Categories, Bills, Income, Accounts — hand-roll a spinner and use neither
`EmptyState` nor `Skeleton`.

The question posed was "table or list-card?", per screen. Answering it screen-by-screen
turned out to matter, because the honest answer is not the same for all four.

## Decision

### 1. The inconsistency to fix is *states*, not *shape*

What reads as incoherence is that the same three moments — loading, empty, error — look
different on every screen. A user meets a centred spinner here, a bare sentence there, and
a blank region somewhere else, and concludes the app is several apps.

So the primitive's job is to make those states **structural**: `DataTable` takes a
`status` and an `empty` description and renders skeletons *in the real column layout*, so
a screen cannot accidentally invent a fourth treatment. Choosing table-vs-list is a
distant second.

### 2. Per screen

| Screen | Decision | Why |
| --- | --- | --- |
| **Bills** | **DataTable** | Every row answers the same set of questions — name, amount, frequency, next due, autopay, category. That is a table. |
| **Income** | **DataTable** | Same shape as Bills; they should not look like different kinds of thing. |
| **Recurring Transfers** | **DataTable** | Ditto — it is Bills with two accounts. |
| **Categories** | **stays a list** | It is a **tree**. Rows nest arbitrarily deep, and a table's columnar promise ("every row is comparable across these fields") is false for a hierarchy. |
| **Accounts** | **stays a list** | Two columns of grouped, collapsible sections (Assets \| Liabilities) — a deliberate layout decided in ADR 0037 / `4d8.23.6`. Flattening it into rows would discard the grouping that makes it readable. |

Categories and Accounts are not exceptions granted reluctantly; they are surfaces whose
structure genuinely is not tabular, and forcing them into a grid would cost more than the
consistency it bought.

### 3. There is no shared `ListCard` primitive

The bead anticipated one, consumed by all four screens. The audit says otherwise: after
Bills and Income move to `DataTable`, the remaining list surfaces are a **tree**
(Categories), a **two-column grouped layout** (Accounts), and a scatter of small bounded
lists that are prompts, alerts, or detail panes rather than collections. A primitive
abstracting over those would have almost no shared behaviour left to hold — it would be a
`<ul>` with a shrug.

What they *do* share is the state handling, and that is addressed directly: the
`EmptyState` and `Skeleton` primitives already exist, and the remaining list surfaces
adopt them rather than each keeping a bespoke spinner. That is the real convergence.

**This is a deliberate divergence from the bead's acceptance criteria**, recorded here
rather than left as a silent gap, and the bead has been updated to match.

### 4. Sorting is opt-in, never assumed

A column declares itself sortable. Two shipped tables show why the default must be off:
Transactions sorts in its filter bar because it sorts *server-side over the whole result
set* (sorting the page in view would be a different, wrong answer), and Projected Activity
**forbids** re-sorting entirely because its rows carry running balances that reordering
would falsify.

### 5. The primitive owns the expander span

Every table with an expander row has to keep its span in step with its column count by
hand — Transactions carried a `colSpan={8}` literal with a comment asking the next author
to remember, Projected Activity computes `3 + columns.length`. Both are maintenance the
author must not forget. `DataTable` derives the span from the column list, so it becomes
structural rather than remembered.

## Consequences

- `DataTable` ships with Transactions migrated as proof; the other five tables follow one
  change at a time (tracked on `4d8.27.4.2`). **Money Inbox goes last** — it renders five
  polymorphic per-kind row components, which a column-def API does not model cleanly, so
  it is a refactor rather than a swap.
- Projected Activity needs one more thing from the primitive before it migrates: a way to
  say "the filter matched nothing" while a pinned row is still present. Noted in the audit.
- Bills, Income and Recurring Transfers convert from list-card to `DataTable`.
- Categories and Accounts keep their shape and adopt `EmptyState`/`Skeleton`.
- No `ListCard` primitive is built. If a third genuinely list-shaped collection appears
  later, revisit — but do not build the abstraction ahead of a second real consumer.
- `4d8.27.4.7`'s token sweep remains separate; this decision is about structure.
