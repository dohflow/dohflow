# ADR 0009: Materialized read-model strategy

- **Status:** Accepted
- **Date:** 2026-05-04
- **Deciders:** Project owner
- **Bead:** [`personal-cfo-a2a`](../../.beads/issues.jsonl)
- **Related plan sections:** §2.6, §9.4, §13.3, §18.2
- **Supersedes:** None

## Context

The canonical state (ADR 0007) is the ledger: balanced postings against ledger accounts, plus categories, recurring events, income sources, and forecasts. The **screens users actually look at** — Dashboard, Future Cash, Transactions, Accounts overview, Money Inbox, Agent Reports — need denormalized, indexed, fast-to-query data. Computing those from canonical tables on every render is too slow once the vault holds a few years of postings.

We need a strategy that:

- Keeps the canonical tables as the only source of truth.
- Lets read-heavy screens query a denormalized shape directly.
- Stays in sync with canonical state automatically, without per-feature ad-hoc sync code.
- Survives schema migration and rebuild on demand.
- Detects drift before it becomes user-visible.

## Decision

Read-heavy data is served by **materialized read-model tables** projected incrementally from the canonical ledger + domain tables.

### Rules

1. **Read models live in dedicated tables**, named with a `_read_model` suffix:
   - `transaction_display_rows_read_model`
   - `account_balance_read_model`
   - `dashboard_summary_read_model`
   - `cash_projection_read_model`
   - `money_inbox_read_model`
   - `commitments_read_model`
   - …and so on per §9.x.
2. **Only the projection runner writes to them.** Command code is forbidden from writing to read-model tables (clippy or grep enforced). The Finance Kernel does not even have a write path to them.
3. **Projection cursors track freshness.** A `projection_cursors` row per read model holds `last_applied_op_seq` (the operation_log sequence_id of the last command applied). Writes to the canonical state advance the cursor as part of the same logical run.
4. **Incremental projection by default; full rebuild on demand.** Each read model has a deterministic `rebuild_from(canonical_state) -> RowSet` function. Rebuild runs at: schema migration, vault restore, drift detection, manual user request via the headless CLI (`pfc projections rebuild`).
5. **Drift detection.** Each read model exposes a `content_checksum()`. CI tests run incremental projection vs full rebuild on the golden fixture vault; mismatch fails the build (`personal-cfo-xn7`).
6. **Read models are tied to schema versions.** A migration that changes a canonical table specifies which read models must be rebuilt; the migration runner triggers those rebuilds during `Migrating` state.
7. **No cross-projection joins inside read models.** Each read model is built from canonical tables only. This keeps projection rebuild deterministic and parallelizable.

### Performance budget

- Full rebuild of any read model on a 50k-transaction fixture: **< 2s** on dev hardware (per the DoD).
- Incremental projection on a normal command tick: **< 25ms** P99.
- Both budgets are baseline-recorded; ≥ 2× regressions fail CI.

## Consequences

### Positive

- Fast, indexable read paths for the UI without sacrificing canonical correctness.
- Schema migrations of read models are cheap (drop + rebuild) — we don't carry write-path code from old shapes.
- A single `rebuild_from` function per read model is testable in isolation.
- Drift is caught by an explicit checksum, not by user reports.

### Negative

- Two layers to maintain (canonical + projection) for every user-facing surface.
- A bug in a `rebuild_from` causes silent UI inconsistency until the drift CI catches it. We mitigate by running drift CI on every PR that touches canonical or projection code.
- Long-running projections during migration can block the vault state machine; we structure migrations to project in foreground only when the user expects a wait.

## Rejected alternatives

### Live SQL queries, no materialized tables

- ✓ Always consistent.
- ✗ Too slow once the vault has a few years of postings; dashboard P99 violates the budget on synthetic fixtures.
- ✗ Some shapes (cash_projection_read_model, money_inbox_read_model) require multi-step computation that doesn't compress to one SQL query.

### SQLite views

- ✗ Views can't be indexed in interesting shapes; complex joins re-execute on every read.
- ✗ Schema evolution of views is awkward.

### Pure event-sourced projections (rebuild from operation_log alone)

- Rejected at the persistence layer in ADR 0011. We project from canonical tables (which themselves are the result of operation_log), not from the operation log directly. This means projection rebuild is independent of operation_log retention.

### Hand-written sync triggers in command code

- ✗ Every new command would need to update every relevant read model — drift is inevitable.
- ✗ No checksum / drift detection.

## Revisit if

- The 2s rebuild budget becomes infeasible on a realistic vault (we'd add a snapshot/checkpoint layer, not abandon read models).
- A specific read model needs cross-vault data (e.g., multi-household consolidation) — that gets a separate ADR.

## Implementation notes

- Crate: `core-projections` (under `crates/`).
- Read model schemas owned by their domain teams (forecast, transactions, dashboard); the projection-runner crate is a thin orchestrator.
- The `pfc` headless CLI exposes `pfc projections rebuild [--read-model=<name>]` for diagnostics (bead `personal-cfo-7l1n`).

## Linked beads

- `personal-cfo-9x4` (Schema: transaction_display_rows_read_model)
- `personal-cfo-lxj` (Ledger-backed transaction projection)
- `personal-cfo-xn7` (Materialized read-model refresh pattern + checksum-based drift detection)
- `personal-cfo-7l1n` (pfc projections rebuild)
- `personal-cfo-7vq` (ADR 0011: hybrid ledger + operation log)
