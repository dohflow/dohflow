# ADR 0051 — Scenario lifecycle: archive, delete, clone, expiry

- **Status:** Accepted
- **Date:** 2026-07-30
- **Beads:** `personal-cfo-4d8.27.6.1`, `personal-cfo-4d8.27.6.6`
- **Extends:** ADR 0026 §5 (scenarios = assumption-event overlays), ADR 0049 §6 (the
  reserved Planning slot)

## Context

Scenario planning today lives as a bar inside the Cash Flow screen: pick base or a
scenario, create one, rename it, set its status, "delete" it. The 2026-07-13 dogfooding
feedback asked for a **dedicated tab with a full lifecycle** — create, adjust, apply to
real data, run several at once, and **clone / archive / delete / expire**.

Two things block that, and both are decisions rather than code:

1. **Archive and delete are the same action, with different data outcomes.** The status
   dropdown sets `status = 'archived'` and leaves the scenario's assumption events
   `active`. `delete_scenario` (`crates/db-worker/src/lib.rs:3081`) *also* lands on
   `status = 'archived'`, but additionally flips every one of its active events to
   `cleared`. So the destructive path and the reversible path share an end state, and
   which one you took is only discoverable by inspecting the events. A user who
   "archived" a scenario to revisit later and one who "deleted" it are indistinguishable
   afterwards — but only one of them still has their work.

2. **Nothing expires.** `scenarios` (migration v12) has `id, name, description, status,
   base_run_id, created_at, updated_at` — no expiry. A scenario built for "the maternity
   leave window" stays in the picker forever.

The owner's words distinguish the two explicitly: *"true archive (keep events), delete,
auto-expire"*. That is three distinct outcomes, and we currently ship one and a half.

## Decision

### 1. Archive and delete are different operations with different guarantees

| Operation | Scenario row | Its assumption events | Reversible |
| --- | --- | --- | --- |
| **Archive** | `status = 'archived'` | **kept exactly as-is** | Yes — restore flips status back |
| **Delete** | row removed | rows removed | No (confirmed first) |

**Archive keeps everything.** That is the whole point of the word: it is a filing
action, not a destructive one. An archived scenario is hidden from the selector and can
never affect a forecast (a run only loads the events of the scenario you selected, and
you cannot select an archived one), but every event survives byte-for-byte, so restoring
returns the scenario intact. This *changes* today's `delete_scenario` behaviour, which
cleared events — that clearing was compensating for the absence of a real delete.

**Delete actually deletes**, matching the house style for user-owned entities:
`apply_delete_recurring_bill` and `apply_delete_income_source`
(`crates/db-worker/src/apply/recurring.rs:634,265`) both issue real `DELETE FROM`
statements, with archive as the separate soft path. A scenario is user-authored planning
data, not ledger history, so the same rule applies. Deleting **cascades to the
scenario's assumption events in the same transaction** — leaving them behind would strand
rows whose `scenario_id` points at nothing.

This is a destructive operation under AGENTS.md §1, so it is confirmed in the UI before
it runs, and it is scoped to the scenario's own overlay: **a delete never touches base
(`scenario_id IS NULL`) events**, which are the user's real forecast.

### 2. Clone copies the definition and the overlay, not the history

Cloning produces a new `draft` scenario with a fresh id, the source's name suffixed
`(copy)`, its description, and **a fresh copy of every active event**, each with its own
new id. Deliberately *not* copied: `status` (a clone starts as a draft — a copy of an
active plan is not itself an active plan), `base_run_id` (the clone forks from whatever
the current base is, not the source's historical run), and `cleared`/superseded events
(they are undo history, not part of the plan).

This makes "what if this, but with one thing changed" a two-click operation, which is the
main thing scenario planning is for.

### 3. Expiry is a user-set date, filtered at read time

`scenarios` gains a nullable `expires_on TEXT` (`YYYY-MM-DD`, household-local). A
scenario whose `expires_on` is before today is **treated as archived for selection**: it
drops out of the picker and cannot be chosen for a run.

Two constraints shape this:

- **It is computed on read, never stored.** This repo already requires that
  time-derived state stay out of materialized/checksummed projections so rebuilds are
  clock-independent (ADR 0014 §7 addendum; the money-inbox snooze expiry and stale-balance
  item are both computed in the read path for exactly this reason). A background job that
  flipped `status` at midnight would make the same mistake those rules exist to prevent —
  and would be indistinguishable, afterwards, from the user archiving it by hand.
- **Expiry never destroys.** An expired scenario keeps its row and its events. It is
  recoverable by clearing or extending the date. Expiry answers "stop offering me this",
  not "throw this away".

Rejected alternative: *deriving* expiry from the scenario's events all being in the past.
It reads well but is wrong in practice — a scenario whose events are historical is often
exactly the one you want to keep selecting to compare against what actually happened, and
computing it means parsing every event's `params_json` date on every list call.

### 4. A scenario never affects the base forecast unless explicitly applied

Restated here because the lifecycle depends on it: scenario events are only loaded when
that scenario is the selected overlay (ADR 0026 §5 — running scenario X is base events
plus X's events). Archiving, expiring, or cloning therefore cannot perturb the user's
real numbers; they only change what is *offerable*. Applying a scenario onto real data
(`4d8.27.6.3`) is the sole path by which scenario content becomes base content, and it
is out of scope here — it gets its own decision.

### 5. The Scenarios tab is a manager, and Cash Flow keeps its selector

Scenarios get the reserved **Planning** slot immediately after Cash Flow (ADR 0049 §1,
§6). The tab is the manager surface: list every scenario with its status, event count and
dates; create; and per-row open / clone / archive / restore / delete.

The Cash Flow screen **keeps a scenario selector** rather than surrendering it. Comparing
a scenario against base is an act of *reading the forecast*, and forcing a tab switch
mid-comparison would break the one workflow scenarios exist to serve. The tab owns
lifecycle; the chart owns comparison. Selecting a scenario in the manager routes to Cash
Flow with that scenario active, so the two stay coherent.

## Consequences

- `delete_scenario`'s meaning changes from "archive and clear events" to "remove". The
  archive path must therefore *stop* clearing events, or archive silently keeps losing
  work. Both halves ship together.
- A migration adds `expires_on`; every scenario read that feeds a selector filters on it.
- Existing scenarios archived under the old behaviour already have cleared events. They
  are not retro-fixed — the events are recoverable in principle (`status = 'cleared'` is
  reversible) but no automatic migration guesses which clearing was a user's intent. The
  count is small and local to the owner's vault.
- Applying, composing, and conflict-reconciling scenarios (`4d8.27.6.2` through `.6.5`)
  build on this lifecycle and are decided separately.
