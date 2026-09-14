# ADR 0032: Transaction reviewed-state and the Money Inbox as the unified review queue

- **Status:** Accepted
- **Date:** 2026-06-27
- **Deciders:** Project owner
- **Bead:** [`personal-cfo-6y1m`](../../.beads/issues.jsonl)
- **Extends:** ADR 0014 §7 (Money Inbox persistence + generation)
- **Related:** ADR 0007 (ledger immutability), `personal-cfo-4d8.7` (implementation), `personal-cfo-tlhe` (Review queues), `personal-cfo-rab` (review_queue_read_model), `personal-cfo-ci71` / `personal-cfo-3d3` (change_journal soft actions)

## Context

Dogfooding R2: imported transactions arrive with no indication of whether the user has
looked at them, and the Money Inbox only surfaces *suspected duplicates*. The
maintainer asked that:

- imported transactions land **UNREVIEWED**, and manually-entered ones default
  **REVIEWED** (they were just typed in);
- the Money Inbox house **all unreviewed items** (not only duplicates), so transactions
  can be categorized / tagged / reviewed from one queue.

This is the unified "Review Queues" idea (`tlhe`, plan §18.5) realized on the
infrastructure that already shipped: the Money Inbox is a deterministic, checksummed
projection (ADR 0014 §7) whose soft user actions (snooze / dismiss) are recorded in
`change_journal_entries` and replayed on rebuild (`ci71` / `3d3`). reviewed-state is the
same *shape* of problem, so it reuses the same machinery rather than introducing a
parallel store.

## Decision

### 1. Transactions carry a `reviewed` flag, defaulted by source

The transaction read model exposes `reviewed: bool`.

- Transactions committed from import (`CommitStaged`) default **UNREVIEWED**.
- Transactions recorded manually (`RecordTransaction`) default **REVIEWED** — the user
  entered them deliberately.
- Transfers and other manual money-movement default **REVIEWED** on the same principle.

### 2. reviewed-state is a soft action in the change journal (not a column the user edits)

"Mark reviewed" (and un-review) is recorded as a `change_journal_entries` entry (new
`entry_kind` `review` / `unreview`), exactly like snooze / dismiss (`3d3`). On every
projection rebuild the journal is **replayed**, so reviewed-state survives a rebuild and
the read-model checksum stays deterministic and clock-independent. The default-by-source
(§1) is intrinsic to the committed transaction; the override is the journal entry. This
mirrors ADR 0027's "derived default + journal override" discipline and ADR 0014 §7's
"resolution is intrinsic to canonical state" rule.

> **Implementation amendment (2026-06-27, `personal-cfo-4d8.7`):** the override is stored
> in a dedicated **`transaction_reviews`** table (`transaction_id` → `reviewed`), not in
> `change_journal_entries`. Reviewed-state turned out to be a *transaction* property — it
> drives the transaction list's `reviewed` flag as well as the inbox generator — not
> purely a money-inbox soft action; and `change_journal` is keyed by money-inbox item id
> and `CHECK`-constrained to inbox actions. A dedicated table is the cleaner fit: it is
> **canonical state** (like `transaction_categorizations`), written by the op-logged
> `MarkReviewed` command and read by both surfaces, so the determinism + survives-rebuild
> properties still hold (it is *read*, never rebuilt). The **default** (no row) is derived
> at read time from import provenance (`source_provenance_links`) — imported = unreviewed,
> manual = reviewed — so existing imports are flagged without a backfill.

### 3. The Money Inbox gains an unreviewed-transaction generator

The `money_inbox_read_model` gains a generator: one review item per
committed-but-unreviewed transaction (`item_kind = unreviewed_transaction`). Because
reviewed-state is **event-driven** (set at commit, overridden by the journal), these
items are **materialized** in the deterministic projection (like
`imported_waiting_commit`), **not** computed-on-read (unlike the time-derived
stale-balance items, `r52x`). Marking reviewed removes the item on rebuild (like
dismiss).

> **Implementation amendment (2026-06-28, `personal-cfo-4d8.7`):** the unreviewed items
> are **computed on read** in `money_inbox_list` (like the stale-balance items, `r52x`),
> **not** materialized into `money_inbox_read_model`. Materializing them would require
> expanding the read model's `item_kind` `CHECK` (a table-recreate migration) **plus** new
> money-inbox-rebuild triggers on `MarkReviewed` / `VoidTransaction`. Computing them from
> canonical state (transactions + `transaction_reviews` + import provenance) is
> deterministic, needs no projection or migration change, and means reviewing a
> transaction simply drops it from the next read. The materialization optimization can be
> revisited if read latency ever needs it at scale.

### 4. Duplicates are a flagged subset, not a separate queue

A suspected-duplicate transaction is an unreviewed item that *additionally* carries the
dedupe reason + suspected counterpart (already in the payload, `dsq`). The Review panel
(`4d8.8`) surfaces incoming-vs-counterpart for those. So the inbox is **one** queue
(unreviewed transactions) with duplicates highlighted — not two parallel surfaces.

### 5. Resolution semantics

- **Mark reviewed** (single or bulk): clears the item.
- **Categorize / tag from the inbox:** allowed inline; does not by itself mark reviewed
  (reviewing is the explicit act), though a "categorize and mark reviewed" affordance is
  reasonable UX.
- **Duplicates:** import-anyway / skip as today (`asqy`), plus the side-by-side Review
  panel (`4d8.8`).
- The materialized money_inbox read (`read_rows`, used by the checksum) stays filtered
  to unresolved items only, deterministically; reviewed-state overrides come from the
  journal replay.

### 6. `tlhe` and `rab` reconcile onto this

`tlhe` (Review queues) and `rab` (review_queue_read_model) are **realized by** the
`money_inbox_read_model` + its generators (`imported_waiting_commit`,
`unreviewed_transaction`, `stale_balance`, snooze/dismiss journal), not a separate
`review_queue` table. They are reconciled to track this ADR rather than describe a
parallel surface (`3d3` already subsumed `review_queue_items` into the inbox; this
finishes the job).

## Consequences

### Positive

- One review surface: the inbox is where unreviewed work lives, matching the feedback
  and the `tlhe` vision.
- No new store — reuses the deterministic projection + change-journal replay already
  shipped (`ci71` / `3d3`), so the checksum / rebuild guarantees hold automatically.
- reviewed default-by-source is honest and zero-friction (manual = reviewed, import =
  needs a look).

### Negative

- A large import produces many inbox items (one per unreviewed transaction). Mitigated
  by bulk mark-reviewed (`5jzg`) + pagination (the new pagination bead) + duplicates
  being a highlighted subset. The queue is meant to be **worked down**; this is intended
  behavior, not noise.
- A new change-journal `entry_kind` (`review` / `unreview`) grows the journal
  vocabulary — acceptable; it is the established extension point.

## Rejected alternatives

- **A `reviewed` column the projection writes directly (no journal).** ✗ A rebuild would
  lose user review decisions unless re-derived; the journal-replay pattern (`3d3`)
  already solves exactly this and keeps determinism. Rejected.
- **A separate `review_queue` table parallel to the money_inbox read model.** ✗ Two
  surfaces for one concept; `3d3` already chose the single-projection path. Rejected.
- **Import defaults reviewed (only duplicates surface).** ✗ Contradicts the feedback;
  imported data is exactly what the user has not yet looked at. Rejected.

## Revisit if

- The unreviewed queue is too noisy for very large imports even with bulk-review +
  pagination → group items by import batch as one collapsible review item.
- Auto-categorization (`5n4`) lands → low-confidence categorization items (`uc95` /
  `j5ij`) join the same queue as another generator; revisit the item taxonomy then.

## Linked beads

- `personal-cfo-6y1m` (this ADR)
- `personal-cfo-4d8.7` (reviewed-state + unreviewed-items generator — implementation)
- `personal-cfo-4d8.8` (duplicate side-by-side Review panel)
- `personal-cfo-tlhe` (Review queues — reconciled onto this)
- `personal-cfo-rab` (review_queue_read_model — reconciled onto the money_inbox projection)
- ADR 0014 §7 (Money Inbox persistence + generation — extended here)
- ADR 0007 (ledger immutability — delete-via-reversal for committed rows)
