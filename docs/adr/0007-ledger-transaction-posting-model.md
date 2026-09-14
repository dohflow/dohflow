# ADR 0007: Ledger transaction / posting model

- **Status:** Accepted
- **Date:** 2026-05-04
- **Deciders:** Project owner
- **Bead:** [`personal-cfo-hgl`](../../.beads/issues.jsonl)
- **Related plan sections:** §9.3, §9.4, §9.5
- **Supersedes:** None

## Context

Financial domain modeling has a long-tested answer for "what is a transaction?": **double-entry bookkeeping**. Each financial movement is a transaction with two or more balanced postings against accounts. Every consumer-finance app eventually rediscovers some weakened version of this and pays for the simplification with bugs around transfers, splits, refunds, credit-card cycles, and reconciliation.

We commit up front to the proper model, sized for our domain.

## Decision

Canonical state for financial movements is two tables:

- `ledger_transactions` — one row per transaction (an event in the household's financial life).
- `ledger_postings` — two or more rows per transaction, each posting a signed amount against a `ledger_account`.

Postings reference accounts via two layers:

- `accounts` — user-visible accounts the household interacts with (a checking account, a credit card, a 401k).
- `ledger_accounts` (and `system_ledger_accounts`) — the underlying ledger-account substrate that gives every transaction somewhere to balance to.

Every transaction satisfies the **balance invariant**: for each transaction, the sum of `posting_amount × normal_balance_signed` across all postings equals zero. This is enforced by a DB-side CHECK constraint or a commit-time trigger; violations fail at insert time, not at read time.

### Money representation (per §9.1)

- Integer minor units (`i64`) + currency code (ISO 4217) + currency exponent.
- No `f64` anywhere on a money path. No `amount_cents` shortcut. No "USD assumed" defaults.
- Property tests cover associativity, conversion, and aggregation; mixed-currency aggregation without an explicit converter is rejected at the type level.

### Account taxonomy (per §9.3)

`accounts` carries:

- `cashflow_role` enum: `liquid_cash`, `credit_facility`, `loan_liability`, `investment_asset`, `real_asset`, `external_clearing`, `income_expense_virtual`. Drives forecast input and dashboard categorization.
- `normal_balance` enum: debit / credit. Drives the balance invariant and posting sign math.
- Tags: `retirement`, `tax_advantaged`, `joint`, `business`, `active`. Boolean flags that drive UI filtering and reporting.
- `last_synced_at`, `manual_balance_observed_at`. Records the last connector sync and the last manual balance observation independently.

### Multi-posting transactions are how we express:

- **Transfers** — two postings on real accounts (one credit, one debit) — no special "transfer" type flag.
- **Credit card payments** — postings against the cash account and the credit facility — both real, balanced.
- **Splits** — postings against multiple expense ledger accounts with one revenue/cash posting that sums them.
- **Reversals** — a new transaction with sign-flipped postings; the original is preserved.
- **Amendments / superseding records** — the original transaction is preserved; a new transaction supersedes it via `supersedes_transaction_id`. No invisible mutation of historical records.

User-facing transaction rows (the "single row per swipe" view a user expects) are a **read model** projected from postings — not the canonical table. See ADR 0009 (read-model strategy) and bead `personal-cfo-9x4`.

### Splits

`split_groups` + `split_lines` handle UI-facing splits without exploding into per-posting noise on the ledger side. Splits are implemented as multi-posting transactions; the split metadata captures the user intent for re-edit and display.

## Consequences

### Positive

- Refunds, partial refunds, chargebacks, and reconciliation adjustments fit the model without special cases.
- Forecasting can read "all postings against `cashflow_role = liquid_cash`" without knowing about transfers or splits.
- Audit history is honest: corrections are visible records, not silent edits.
- Multi-currency is a first-class concern, not a retrofit.

### Negative

- More tables and constraints to migrate and maintain than a single-row-per-transaction design.
- New developers need a brief lesson on double-entry; the kernel-test-harness crate's example commands cover this.
- The user-facing read model's projection logic is non-trivial — but it lives in one place (`personal-cfo-lxj`) and has a rebuild test on golden fixtures.

## Rejected alternatives

### Single-table transactions with a `transaction_type` flag

- ✗ Transfers, splits, refunds become flag soup.
- ✗ Multi-currency conversion math has no clean home.
- ✗ Reversal/amendment requires either silent mutation (bad) or a side table (worse: still drifts).

### Pure event sourcing (postings as event projections)

- Rejected at the persistence layer in ADR 0011. The hybrid model gives us the auditability without the projection-drift risk.

### Materialized "user transaction" as canonical, postings as a side ledger

- ✗ Postings end up out of sync with the user-facing rows under any non-trivial edit.
- ✗ We've inverted the source of truth — the side ledger is the actual financial record.

### Skip ledger accounts, only have user-visible accounts

- ✗ Income/expense/equity flows have no balancing accounts.
- ✗ "Where did this $50 come from?" has no formal answer.

## Revisit if

- The double-entry overhead becomes intolerable at scale (it doesn't, but if it did we'd consider a snapshot/checkpoint model on top, never replacing postings).
- Multi-currency rules require an external lot/cost-basis layer (likely if we ever do investment cost basis seriously — that's a separate ADR for §11 / §16, not a ledger redesign).

## Implementation notes

- Crate: `core-ledger` (under `crates/`).
- Schema bead: `personal-cfo-19s` (ledger_transactions + ledger_postings).
- Property tests: `personal-cfo-nn5` (ledger invariant tests), `personal-cfo-4lb5` (cashflow / split / date property suite).
- Money type bead: `personal-cfo-5vr`.

## 9. Transaction deletion via void (addendum 2026-06-27, `personal-cfo-4d8.11`)

Users need to delete a transaction (a wrong manual entry, a bad import), but the ledger
is **append-only** — postings are never mutated or removed (the balance trigger and the
op-log replay both depend on it). So "delete" is implemented as a **void**, not a
hard delete (ratified with the project owner; hard delete and a manual/imported hybrid
were both rejected for breaking the immutability the rest of the system relies on):

- **`VoidTransaction`** is an op-logged kernel command. The db-worker loads the
  target's postings, appends a **reversing transaction** (every leg negated, dated at
  the original's `occurred_at` so it lands in the same assertion window, ADR 0027), and
  marks both the original and the reversal with `ledger_transactions.voided_at`.
- **Balances and the forecast need no special-casing**: they sum postings, and the
  reversal nets the original to zero across every account (user leg + system counter).
  This is why a reversal — not a `voided` filter on the balance derivation — is the
  mechanism: it keeps every downstream sum correct for free.
- **The user-facing list** (`read_recent_transactions`) filters `voided_at IS NULL`, so
  neither the original nor the reversal appears.
- **The `transaction_display` projection is intentionally left unfiltered**: it contains
  both the original and the reversal (which net to zero), so the incremental update and a
  full rebuild stay byte-identical (ADR 0009 determinism). It is not the UI's list
  source today; when a consumer needs a void-aware projection, add the filter together
  with a rebuild-on-void.
- The transaction stays in the **encrypted local vault** for audit/recovery; true
  erasure, if ever needed, is a separate explicit "purge" capability, not this command.

## Linked beads

- `personal-cfo-4d8.11` (Delete a transaction — void via reversal)
- `personal-cfo-19s` (Schema: ledger_transactions + ledger_postings)
- `personal-cfo-459` (Schema: accounts + ledger_accounts + system_ledger_accounts)
- `personal-cfo-kr9` (Schema: split_groups + split_lines)
- `personal-cfo-nn5` (Ledger invariant tests)
- `personal-cfo-lxj` (Ledger-backed transaction projection)
- `personal-cfo-7vq` (ADR 0011: hybrid ledger + operation log)
- `personal-cfo-a2a` (ADR 0009: materialized read-model strategy)
