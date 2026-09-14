# ADR 0033: Transaction tags and notes

- **Status:** Accepted
- **Date:** 2026-06-28
- **Deciders:** Project owner
- **Bead:** [`personal-cfo-4d8.14`](../../.beads/issues.jsonl)
- **Related:** ADR 0030 (categorization — the 1:1 category), ADR 0007 (ledger immutability), `personal-cfo-2ryf` (tags schema), `personal-cfo-hmt` (kernel tags/notes), `personal-cfo-byxe` (memo/counterparty), `personal-cfo-2db9` (manual memo), ADR 0034 (splits — pending)

## Context

Dogfooding R2 surfaced two transaction-metadata asks:

1. **Tags.** *"Custom tags (not just categories — so people can make a tag for a
   vacation that spans different categories, and see how much the vacation cost and how
   it breaks down by category)."* A tag is a cross-cutting label; one transaction can
   carry many.
2. **Notes.** *"A way to add custom notes to each transaction"* — a free-text
   annotation.

Both are per-transaction metadata that sit **beside** the pure double-entry ledger
(ADR 0007 stays clean — no ledger change), and both are distinct from fields that
already exist:

- A **category** (ADR 0030) is a single **1:1** classification of *what kind* of spend
  (Groceries, Rent).
- The **memo / counterparty** (`byxe`) is the transaction's imported description / payee
  — its *title* — and the **manual memo** (`2db9`) lets a manual entry set that title.
- A **note** is the user's own free-text annotation, separate from the title.

## Decision

### 1. Tags are many-to-many labels, orthogonal to categories

A transaction has exactly **one category** (ADR 0030) and **zero or more tags**. A tag
is a free user-defined label ("Vacation 2026", "Reimbursable", "Tax-deductible") that
applies *across* categories. That orthogonality is what makes the cross-category rollup
possible: "total Vacation" sums every transaction tagged Vacation regardless of its
category — and can then be broken down *by* category. Categories answer *what kind*;
tags answer *which initiative / context*. A transaction is both categorized and
(optionally) tagged; they don't compete.

### 2. Tag storage (`2ryf`)

- **`tags`** (id, name, color, archived) — the tag definitions. Names unique within the
  vault; **soft-delete** via `archived` so historical `transaction_tags` links survive.
- **`transaction_tags`** (transaction_id, tag_id, PK both) — the many-to-many link.

This is canonical state (like `transaction_categorizations`), **not** a rebuilt read
model.

### 3. Note storage

A note is free text, **≤ 4096 chars**, stored as a `note` column on
**`transaction_details`** (migration), 1:1 with the transaction, beside the existing
`memo` / `counterparty` (`byxe`). One detail row per transaction that has any detail;
the list LEFT JOINs it. Notes are **sensitive free text**, so they flow through the
redaction layer — a pasted account number must never leak into logs (`hmt`'s redaction
CI obligation).

### 4. Kernel commands (`hmt`)

`hmt`'s `update_transaction_metadata` is split into focused, op-logged commands so each
surface stays simple:

- **`SetTags`** — replace a transaction's tag set (add / remove); minting a new tag is
  its own **`CreateTag`** (name + color).
- **`SetNote`** — set / clear a transaction's note.

(Attachments — the third leg of `hmt` — already shipped via the encrypted attachment
store, ADR 0023; `hmt`'s remaining scope is tags + notes.)

### 5. Read-model exposure

The transaction list read (`read_recent_transactions`) and `TransactionRowDto` gain
**`tag_ids`** (an array) and **`note`**, mirroring how `category_id` and `reviewed` are
exposed — so the list shows tag chips and the drawer edits them. The `2ryf` AC's
`transaction_display_rows_read_model.tag_ids` projection column is the materialized
counterpart, filled when the projection is wired to the UI (like the deferred
categorization column, `uc95`).

### 6. Cross-category rollups are enabled; the rollup UI is deferred

This model makes "spending by tag" (and a tag's category breakdown) computable. The
report **UI** is a separate follow-on — tags + notes ship first (assign + display),
tag-filtered reporting later.

### 7. Split-line tags (forward note)

When transaction splits ship (ADR 0034, model C — `split_lines` carry per-line
category / tag / note), tags + notes extend from the transaction to the split line.
Until then they are transaction-keyed; the splits work re-keys the reads to the line
level when a transaction is split. This ADR's transaction-level model is the base case.

## Consequences

### Positive

- Tags give the cross-category view categories can't (the "vacation cost" use case)
  without overloading the category taxonomy.
- Reuses the canonical-state + read-model-exposure pattern already used for categories
  and reviewed-state — no new architecture.
- Notes live beside the existing memo in `transaction_details` — one detail table, no
  new per-transaction table.

### Negative

- Another many-to-many aggregate per transaction (`tag_ids`) on the list query; fine at
  single-household scale, and the materialized projection column (`2ryf`) is the
  optimization if it's ever needed.
- Notes are sensitive free text → the redaction obligation must be honored (`hmt` AC).

## Rejected alternatives

- **Tags as a second (hierarchical) category dimension.** ✗ Tags are intentionally
  flat + cross-cutting; a hierarchy just re-creates categories. Rejected.
- **Notes in a separate `notes` table.** ✗ A note is 1:1 with a transaction, like
  memo / counterparty, which `transaction_details` already holds; a parallel table
  fragments per-transaction detail. Rejected.
- **One `update_transaction_metadata` mega-command.** ✗ Tags and notes are edited from
  different controls at different times; focused `SetTags` / `SetNote` keep each call +
  its cache invalidation simple. Kept the `hmt` intent, split the command.

## Revisit if

- Tag-keyed reporting needs the materialized `tag_ids` projection column for
  performance → fill it (`2ryf`) then.
- Splits ship (ADR 0034) → extend tags + notes to split-lines.

## Linked beads

- `personal-cfo-4d8.14` (this ADR)
- `personal-cfo-2ryf` (Schema: tags + transaction_tags)
- `personal-cfo-hmt` (kernel: `SetTags` / `SetNote`, + the shipped attachment leg)
- `personal-cfo-4d8.15` (tags + notes UI)
- ADR 0030 (categorization — the 1:1 category)
- ADR 0034 (splits — split-line tags, pending)

## Addendum (2026-07-08, personal-cfo-4d8.24.5.1): tags on recurring bills

Owner dogfooding wanted to set tags (not just a category) when **promoting a suggestion to a
recurring bill**, so the bill's forecasted occurrences carry those tags. Tags were transaction-only.

**Decision.** Reuse the **shared `tags` vocabulary** (the same `tags` table) for bills, joined by a
new **`recurring_event_tags (recurring_event_id, tag_id)`** table mirroring `transaction_tags`
(migration v42). A tag is one concept the user applies across transactions and bills — a separate
bill-tag vocabulary would fragment it.

- **Set at create/promote.** `CreateRecurringBill` gains an optional `tag_ids` (like the category,
  personal-cfo-4d8.24.5); the apply validates each tag exists and inserts into `recurring_event_tags`.
  A dedicated `SetBillTags` edit command (mirroring `SetTags`) is a follow-up; v1 sets tags at create.
- **Round-trip on read.** `RecurringBillView` / `RecurringBillDto` expose `tag_ids` so the Bills
  list/edit reflect them, exactly as the category now does.
- **Invariant preserved.** ADR 0018 "user confirms, never auto-create" is untouched — this only
  widens what the user confirms when promoting.

This extends "Revisit if → splits ship" with a third join surface; the `tags` table and the
`SetTags` clear-then-add pattern are reused unchanged.
