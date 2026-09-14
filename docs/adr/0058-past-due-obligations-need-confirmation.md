# ADR 0058 — Where a past-due unconfirmed obligation is surfaced, and what proves it

- **Status:** Accepted
- **Date:** 2026-08-07
- **Bead:** `personal-cfo-4d8.27.7.6`
- **Builds on:** ADR 0026 §19 (`ConfirmObligationEarly`), ADR 0049 (IA), ADR 0018 (descriptive)
- **Reconciles:** `personal-cfo-92x7` (upcoming-obligation insight), `personal-cfo-vn6b`
  (recurring-instance status divergence)

## Context

A bill whose scheduled date has passed with no matching transaction leaves the forecast in
a state it cannot resolve alone: either the household paid it (and the cash is already
gone) or they did not (and it is still coming). The projection is wrong in one direction or
the other until someone says which.

The machinery to fix it already exists — `ConfirmObligationEarly` posts the transaction,
links the occurrence, and the forecast stops projecting it. What is missing is a surface
that says *which* occurrences are waiting.

Two beads already touch this space, and the bead's own acceptance criteria require
reconciling with both.

## Decision

### 1. Past-due lives on Cash Flow; upcoming lives in the Money Inbox

| Window | Surface | Bead |
| --- | --- | --- |
| `scheduled_date` **has passed** | a section on **Cash Flow**, above Projected Activity | this one |
| due within the next N days (default 14) | a **Money Inbox** insight | `92x7` |

They are **disjoint by construction** — `< today` versus `>= today` — so no occurrence is
ever nagged about in two places at once. That disjointness is the reason both can exist
without becoming two competing queues.

Past-due belongs on Cash Flow because the thing at stake *is* the cash-flow number: the
projection above it is unreliable until these are resolved, and the fix should sit next to
the figure it corrects rather than one destination away. Upcoming belongs in the Money
Inbox because nothing is wrong yet — it is a piece of triage, which is what that surface
is for.

### 2. `confirmed_obligations` is the proof, never `recurring_event_instances.status`

The queue lists occurrences with **no row in `confirmed_obligations`**. It does **not**
filter on `recurring_event_instances.status = 'scheduled'`.

This is not a style preference; the alternative is broken today. `vn6b` documents that a
confirm does **not** flip the instance's status when it lands more than ±7 days early or
carries a $0 amount — the row stays `'scheduled'` while the obligation is genuinely
confirmed. A status-driven queue would therefore list occurrences the user has already
confirmed, and confirming again posts a **second transaction**. A queue whose whole purpose
is resolving uncertainty would be manufacturing a double-payment.

`confirmed_obligations` is already what `collect_bill_events` consults to suppress a
confirmed occurrence from the projection (`confirmed_obligation_dates`). Reading the same
table is what keeps this section and the forecast agreeing about what is outstanding — the
same coupling rule ADR 0056 §2 applied to the liquid-account set.

**This does not close `vn6b`.** The read-model divergence is real and still affects any
other surface reading `recurring_event_instances.status`. This ADR routes *around* it for
the one surface where the consequence would be a duplicate ledger write.

### 3. The window is bounded, and the boundary is stated

The queue looks back a bounded number of days rather than to the beginning of the ledger. A
household that starts using the app with two years of recurring history would otherwise
open Cash Flow to hundreds of occurrences it cannot meaningfully answer for.

The bound is the same window the instance projection already materializes, so the queue
never claims to know about occurrences the instance table has not expanded.

### 4. Copy is descriptive (ADR 0018)

The section states what is unresolved and what the forecast currently assumes. It does not
tell the user to pay anything, does not characterize them as late, and does not rank the
items by urgency. "Due 12 days ago, not yet confirmed" is a fact; "overdue — pay this
now" is advice.

## Consequences

- The Cash Flow projection above the section is only as trustworthy as the section is
  empty. That is worth stating in the copy, and it is the honest reason the section sits
  there rather than elsewhere.
- Resolving an item uses the **existing** `MarkObligationPaid` control, so a confirm from
  this queue is the same audited write as a confirm from Projected Activity — one path,
  one behaviour, and the undo bar keeps working.
- `92x7` stays open and unchanged in scope; its window is now explicitly the complement of
  this one.
- `vn6b` stays open. If it is later fixed such that status always reflects
  `confirmed_obligations`, this query does not need to change — it is already reading the
  authoritative source.
- An occurrence the user genuinely never paid stays in the queue indefinitely. That is
  correct: it is unresolved, and hiding it after N days would quietly restore the
  uncertainty this section exists to remove.

## Addendum (2026-09-08): the window's start boundary was per-run, not per-bill (`personal-cfo-5ie.10`)

§3 says the queue's window is bounded and states the bound — but the bound it names (the
instance projection's `[window_start, window_end]`) is shared across every schedule, and
`pay_schedule` deliberately expands a schedule's whole lattice within that window regardless
of the anchor's position in it (ADR 0047 §3: any occurrence is an equivalent anchor). A bill
entered today with a year-old anchor therefore materialized roughly a year of unlinked
monthly occurrences, and every one of them passed §3's stated bound — the bug was that the
bound was never meant to answer "since when has *this bill* been tracked," only "how far
back does the instance table reach at all."

**Corrected rule:** an occurrence dated before its bill's `recurring_events.created_at`
surfaces only while it is that bill's single most recent past occurrence (resolved or not);
once it is confirmed, nothing older than `created_at` is ever offered for that bill again. A
plain `created_at` floor — excluding every pre-creation occurrence outright — was considered
and rejected: `created_at` is stamped essentially at the same moment as "the bill I'm
entering was already due," which is the normal flow for adding a bill, not an edge case: a
strict floor would silently drop the one occurrence a household most needs to confirm. A bill
tracked for a while, with several genuinely missed occurrences since creation, is unaffected
by either form of the rule — every one of those already satisfies `scheduled_date >=
created_at` directly, so this addendum narrows only the pre-creation portion of the lattice,
never the "hide nothing genuinely unresolved" consequence above.

The first implementation of this rule computed "the bill's newest occurrence" over only the
still-unresolved candidate rows, which meant confirming the one pre-creation row that
surfaced made the next-older pre-creation occurrence become the new maximum and surface in
its place — repeatable back through the entire lattice, each step a real posted transaction.
The reference must be fixed over the bill's full history, not recomputed against what is
still outstanding, or the corrected rule degrades back into §3's original bug one confirm at
a time.
