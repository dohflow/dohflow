# ADR 0034: Transaction splits

- **Status:** Accepted
- **Date:** 2026-06-28
- **Deciders:** Project owner
- **Bead:** [`personal-cfo-4d8.17`](../../.beads/issues.jsonl)
- **Related:** ADR 0007 (ledger immutability), ADR 0030 (categories are a side classification, not ledger accounts), ADR 0032 (reviewed-state side table), ADR 0033 (tags + notes side tables), `personal-cfo-kr9` (schema), `personal-cfo-e7i` (kernel)

## Context

A single purchase often spans categories — a $200 Target run is $120 groceries +
$80 household. Users want to **split** one transaction into multiple
categorized / tagged / noted lines while it stays **one** transaction: one
bank-statement line, one row, and the lines' amounts add up to the whole.

The high-level shape ("Option C") was ratified earlier: one ledger transaction,
per-line category / tag / note, an expandable parent row, totals reconcile. This ADR
settles the **storage sub-question** — how the slices are stored — which the earlier
beads (`kr9` / `e7i`) had left as a *multi-posting* model inherited from their original
acceptance criteria.

**Decision (2026-06-28): the slices live in a side `split_lines` table, not as ledger
postings.** This app deliberately keeps the ledger a pure double-entry substrate and
puts *all* classification beside it: categories aren't ledger accounts (ADR 0030), and
tags / notes (ADR 0033) and reviewed-state (ADR 0032) are side tables. A split is
classification — *which slice is which category* — so it belongs in that same
side-table layer. Nothing in the app needs the slices to be postings: account balances
derive from the (unchanged) asset posting, and reporting reads the lines. The
multi-posting alternative is recorded and rejected below.

## Decision

### 1. A split is a side-table decomposition of one transaction

The **ledger transaction is unchanged** — the original asset posting + its single
counter-posting stand (ADR 0007 immutability preserved; balances unaffected). A
`split_lines` table holds the per-line decomposition. An **unsplit** transaction has
zero split lines (its own category / tags / note apply). A **split** transaction's
per-line classification **supersedes** the transaction-level one for reads.

### 2. Schema (`kr9`, revised off multi-posting)

- **`split_lines`** (id, transaction_id FK, amount_minor, category_id FK nullable,
  note TEXT nullable, sort_order INTEGER). Canonical state, keyed by transaction_id
  (like `transaction_categorizations`).
- **`split_line_tags`** (split_line_id FK, tag_id FK, PK both) — per-line tags (the
  ADR 0033 §7 extension: tags reach from the transaction down to the line).
- The old AC's **`split_groups`** (intent enum `{even, percentages, custom}`,
  `ui_state_json`) is **dropped from the first cut.** Even / percentage splitting is a
  UI computation that emits explicit per-line `amount_minor`; persisting the *intent*
  for re-edit is a later `split_lines.share_basis` (or a small `split_groups`
  companion) if it's ever wanted. Amounts are stored explicitly.

### 3. The sum invariant is command-enforced

The split lines' `amount_minor` **must sum to the transaction's signed amount** (a split
partitions the whole), same sign as the transaction. This is enforced by the
`SetSplits` command — reject if the sum ≠ the transaction amount. The **caller supplies
explicit minor-unit amounts** (the UI computes even / percentage splits and assigns any
rounding remainder to a chosen line before calling), so the kernel only *checks* the
sum — no kernel-side rounding policy.

### 4. Kernel command (`e7i`, revised off multi-posting)

**`SetSplits { transaction_id, lines: Vec<SplitLine> }`**, where
`SplitLine = { amount, category_id?, note?, tag_ids }`. **Replace semantics:** it
replaces the transaction's entire split set; an empty `lines` **un-splits** the
transaction. Op-logged. Validates: the transaction exists and is not voided; the sum
equals the transaction amount; referenced categories / tags exist. It writes
`split_lines` + `split_line_tags` and **leaves the ledger postings untouched**.

### 5. Read exposure + rollup resolution

- The transaction read exposes the lines — a `splits` array on the row (or a
  `split_count` + a `transaction_splits(transaction_id)` read for the expand).
- Category / tag / spending rollups read the **lines** when a transaction is split, the
  **transaction-level** category / tags otherwise. "Spending on Groceries" sums
  split-line amounts categorized Groceries **plus** whole unsplit transactions
  categorized Groceries.
- The transaction list shows an **expandable parent row**: collapsed = the transaction
  (total amount + a split indicator); expanded = the lines with their category / tags /
  note.

### 6. Interactions

- **Reviewed-state (ADR 0032):** a transaction is reviewed as a whole; splitting does
  not change reviewed-state.
- **Void (ADR 0007 §9):** voiding voids the whole transaction (the reversal nets the
  asset posting). `split_lines` are read-layer and excluded with the transaction (reads
  filter by the transaction's `voided_at`); no cascade is required.
- **Tags / notes (ADR 0033):** transaction-level tags / notes remain for unsplit
  transactions; split-line tags / notes are the per-line refinement used once a
  transaction is split.
- **Balances / forecast:** unaffected — the asset posting is the source of truth and is
  never touched by a split.

## Consequences

### Positive

- **One mental model:** the ledger stays a pure double-entry substrate and *every*
  classification dimension — category, tags, notes, reviewed, now splits — lives in a
  side table.
- **Simple schema + kernel:** `split_lines` parallels `transaction_categorizations`;
  `SetSplits` is a replace-the-set write plus a sum check. No new postings, no ledger
  structure change.
- **Balances / forecast untouched** — the asset posting is authoritative.

### Negative

- The sum invariant is a **command-level** check, not a ledger-enforced property.
  Acceptable: the command is the only writer, as for every other canonical-state table.
- Per-line rollups add a read-time branch (split → read lines, else read the
  transaction). Fine at single-household scale.

## Rejected alternatives

- **Multi-posting (the original `kr9` / `e7i` framing).** ✗ The counter-side becomes N
  real postings within the transaction, with `split_lines.posting_id` mapping each to a
  category. But nothing needs slices to be postings (balances come from the asset
  posting; reporting reads the lines), and it injects categorization-driven structure
  into the ledger — N counter-postings to the *same* system account differing only by
  their `split_line` category, which strains "categories aren't accounts" (ADR 0030).
  The side table is simpler and consistent. **Rejected (maintainer, 2026-06-28).**
- **Multiple ledger transactions (one per slice).** ✗ Fractures the bank-statement 1:1,
  breaks dedup / reconciliation, and double-counts in naive reads. Rejected.
- **Postings to per-category accounts.** ✗ Requires categories to be ledger accounts;
  ADR 0030 deliberately keeps categories a side classification. Rejected.

## Revisit if

- A need for the slice amounts to be first-class ledger entries emerges (e.g. per-category
  sub-ledgers) → reconsider multi-posting; the side table can be migrated into postings.
- Split UI intent (even / percentage re-edit) needs persistence → add
  `split_lines.share_basis` or a small `split_groups` companion.

## Linked beads

- `personal-cfo-4d8.17` (this ADR)
- `personal-cfo-kr9` (schema: `split_lines` + `split_line_tags` — revised off multi-posting)
- `personal-cfo-e7i` (kernel: `SetSplits` — revised off multi-posting)
- `personal-cfo-4d8.18` (splits UI — the expandable parent row + split editor)
- ADR 0030 / 0032 / 0033 (the side-table classification pattern)
- ADR 0007 (ledger immutability — splits never touch postings)
