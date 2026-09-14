# ADR 0006: Finance Kernel and command boundary

- **Status:** Accepted
- **Date:** 2026-05-04
- **Deciders:** Project owner
- **Bead:** [`personal-cfo-sab`](../../.beads/issues.jsonl)
- **Related plan sections:** §2.6, §9.1.2, §0 (decision 1)
- **Supersedes:** None

## Context

Personal CFO has multiple potential paths into financial state: manual UI entry, CSV/OFX imports, document extractors, connector adapters, AI agent reports, scheduled job runners, headless CLI commands. Without a single chokepoint, each path will independently re-implement validation, idempotency, audit logging, and provenance tracking — and will eventually drift.

We need an internal API that:

- Owns the canonical financial state mutations.
- Enforces idempotency, atomicity, and audit at one place.
- Refuses writes from importers, connectors, and AI agents (those produce **staged candidates**, not committed records).
- Stays small enough to be testable and reasoned about.
- Survives the addition of multi-device sync without rearchitecture.

## Decision

Define the **Finance Kernel** as the single boundary through which all financial state mutations pass. The kernel exposes a typed `KernelCommand` interface; every mutation is a command with stable identity and metadata.

### Kernel surface

```rust
trait KernelCommand {
    type Outcome;
    fn validate(&self, ctx: &KernelContext) -> Result<()>;
    fn apply(self, txn: &mut KernelTransaction) -> Result<Self::Outcome>;
}

struct KernelContext { /* vault state, schema version, household tz, ... */ }
struct KernelTransaction { /* SQLCipher transaction + op-log writer */ }
```

```rust
impl Kernel {
    fn dispatch<C: KernelCommand>(&self, cmd: C, meta: CommandMeta) -> Result<C::Outcome>;
}
```

### Required command metadata (per §9.1.2 and ADR 0012)

Every command carries:

- `command_id` — UUIDv7, server-side generated.
- `idempotency_key` — caller-supplied, scoped per command type. Replays return the original `result_ref`.
- `actor_id`, `actor_type` — `user`, `importer`, `connector`, `agent`, `system`, `scheduled_job`.
- `vault_schema_version` — rejected if mismatched.
- `node_id` — vault instance UUID (§2.6).
- `hlc_timestamp` — hybrid logical clock for future sync.
- `causation_id`, `correlation_id` — link to the user action / batch / agent run that triggered the command.

### Atomicity

Each successful command commits, in a single SQLCipher transaction:

1. Domain table mutations (ledger_transactions, ledger_postings, accounts, categories, etc.).
2. Provenance link rows (where the change came from).
3. One immutable operation_log row (ADR 0011).

Partial failure rolls all three back. There is no path where domain rows exist without a matching operation_log entry.

### Who writes what

| Actor | Direct domain writes | Allowed paths |
| --- | --- | --- |
| User / UI | Yes (via kernel commands) | Typed Tauri commands → kernel |
| Importer | **No** | Submit `staged_transactions`; kernel `commit_staged` command promotes them |
| Connector adapter | **No** | Submit `staged_transactions`; kernel `commit_staged` command promotes them |
| Document extractor | **No** | Submit `document_extractions`; user reviews; kernel commands commit |
| AI agent | **No** | Produce schema-validated reports; never commits financial records |
| Headless CLI | Yes | Via the same kernel commands the UI uses |
| Migration runner | Yes (limited) | Schema migrations only, gated by vault state machine |

### Validation philosophy

- Frontend validation (Zod) is for UX only — it shapes the form, surfaces obvious errors early, and prevents wasted IPC round-trips. It is **not authoritative**.
- Kernel validation is authoritative. It runs before any DB write and produces typed error variants that the UI renders without re-interpreting.

## Consequences

### Positive

- One audit point for every financial change. Operation_log tells the complete story (ADR 0011).
- Idempotency is uniform — the kernel doesn't trust callers to deduplicate.
- Adding a new ingestion source (a connector, a document extractor, a future LLM) can never bypass audit by accident; the type system requires going through `KernelCommand`.
- Multi-device sync can replicate operation_log envelopes (or purpose-built sync changesets) once we add it; no per-feature retrofit needed.

### Negative

- Every new feature with a write path requires a new `KernelCommand` impl + tests. This is the friction we want, but it costs implementation time.
- The kernel is a serialization point. The DB worker (ADR-pending; bead `personal-cfo-chz`) owns one writer connection; long-running write commands must be chunkable. Mitigated by keeping commands small and using read-only connections for projection/UI queries.

## Rejected alternatives

### Repository pattern with no command bus

- ✗ Each feature crate would re-implement idempotency / audit / provenance, and they'd drift.
- ✗ No single chokepoint for adding cross-cutting behavior (e.g., a future "audit every command to a report" feature).

### CQRS with full event sourcing

- Rejected at the persistence layer in ADR 0011 (hybrid model) — the command boundary stands on its own merits, but a pure event-sourced aggregate model is overkill for our scale and adds projection-rebuild risk.

### Direct SQL from each feature crate

- ✗ No invariant enforcement, no idempotency, no atomic op-log.
- ✗ Frontend or importer code can grow a write path that bypasses audit silently.

### Kernel as a thick service that owns everything (forecasting, categorization, agents)

- ✗ Bloats the trusted core; we want the kernel small. Forecasting, categorization, and agent-broker logic live in their own crates and call the kernel through commands.

## Revisit if

- The single-writer DB worker becomes a perf bottleneck on a realistic 5-year fixture vault.
- We genuinely need multi-process kernel instances (we don't, today).
- The metadata schema needs extension beyond §9.1.2 — we extend, not replace, and update ADR 0012.

## Implementation notes

- `finance-kernel` Rust crate under `crates/`. No public DB types leak out — DB access is through the db-worker crate.
- A thin "kernel-test-harness" crate constructs an in-memory SQLCipher fixture and exercises commands directly without the Tauri layer; used by integration tests.
- Tracing spans wrap each `dispatch()` call; spans pass redaction CI per the DoD.

## Linked beads

- `personal-cfo-jh6` (FEATURE: Finance Kernel skeleton)
- `personal-cfo-02i` (Implement command bus with idempotency + atomic op-log writes)
- `personal-cfo-chz` (DB worker / repository boundary)
- `personal-cfo-29l` (ADR 0012: command idempotency and retry semantics)
- `personal-cfo-7vq` (ADR 0011: hybrid ledger + operation log)
- `personal-cfo-1al` (ADR 0003: trust boundary)
