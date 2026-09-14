# ADR 0011: Hybrid ledger + operation-log persistence

- **Status:** Accepted
- **Date:** 2026-05-04
- **Deciders:** Project owner
- **Bead:** [`personal-cfo-7vq`](../../.beads/issues.jsonl)
- **Related plan sections:** §0 (decision 7), §2.6, §9.1.2, §9.4
- **Supersedes:** None

## Context

Earlier plan revisions considered full event sourcing — every state change as an immutable event in a single stream, with current state as a projection. Event sourcing has real strengths for auditability, sync, and time-travel debugging. It also has real costs: migration of event schemas, projection-rebuild discipline, and debugging an "event happened but state isn't right" production issue is expensive.

For a personal finance app at MVP scale, with one developer, single-device for the foreseeable future, the costs dominate. We still want auditability and the path to future sync — but we don't need to pay the full event-sourcing tax to get them.

## Decision

Use a **hybrid persistence model**:

- **Canonical state** is the normalized relational ledger and domain tables (ADR 0007 + §9). User-facing financial truth lives here.
- **Canonical history** is an immutable, append-only `operation_log` table that records every successful kernel command (ADR 0006).
- **Projections** are materialized read models derived from canonical tables (ADR 0009).

Operation log + canonical tables together give us:

- Auditability (replay the log to see every change).
- Deterministic rebuilds of projections.
- Provenance: every domain-table row links back to the operation_log entry that produced it.
- A clear path to future multi-device sync (replicate operation_log envelopes or purpose-built sync changesets).

We do **not** make the operation log the only source of truth. State queries read canonical tables.

### Operation log shape

```
operation_log(
  sequence_id          INTEGER PRIMARY KEY,           -- monotonic per vault
  command_id           BLOB UNIQUE NOT NULL,          -- UUIDv7
  idempotency_key      BLOB UNIQUE,
  node_id              BLOB NOT NULL,                 -- vault instance UUID
  hlc_timestamp        BLOB NOT NULL,                 -- hybrid logical clock
  actor_type           TEXT NOT NULL,                 -- user|importer|connector|agent|system|scheduled_job
  actor_id             TEXT NOT NULL,
  operation_type       TEXT NOT NULL,                 -- e.g. "create_account", "commit_staged_batch"
  affected_entities    BLOB NOT NULL,                 -- JSON: list of {table, id} pairs
  causation_id         BLOB,
  correlation_id       BLOB,
  metadata             BLOB,                          -- JSON: command-specific context
  created_at           TIMESTAMP NOT NULL
)
```

(Schema bead: `personal-cfo-0s0`.)

### Atomicity

Each successful command commits, in a single SQLCipher transaction:

1. Domain table writes.
2. Provenance link rows (`provenance_links`, `personal-cfo-pr8` family of beads).
3. Exactly one operation_log row.
4. Cursor advance for read models that subscribe to the affected tables.

If any of those fail, the whole transaction rolls back. There is **no** state where domain rows exist without a matching operation_log entry.

### Hybrid logical clock

`hlc_timestamp` combines wall-clock time with a logical counter to produce globally orderable, causally consistent timestamps without requiring clock synchronization. In single-device mode, HLC reduces to wall-clock time. The field is reserved now so future multi-device sync (ADR 0017) can make ordering decisions without a schema migration.

### Snapshots / checkpoints (optional optimization)

For very large vaults, periodic snapshots of canonical tables may be created to bound rebuild cost. Snapshots are an optimization, not a source of truth. We do not implement snapshots in MVP; the bead exists at `personal-cfo-vfo3` for later.

### Retention

- Canonical tables: retained indefinitely.
- Operation log: retained indefinitely (it's the audit trail).
- Idempotency keys: retained per a documented policy with TTL (`command_idempotency_keys.expires_at`); GC is safe because canonical state survives the GC.

## Consequences

### Positive

- Audit, debugging, and provenance work from day one.
- Projections are rebuildable; we don't fear changing them.
- Multi-device sync is a future addition with a known shape, not an architecture redo.
- Migrations are easier than in pure event sourcing — we migrate canonical schema and (separately) regenerate projections. The operation log's schema is small and stable.

### Negative

- We pay for two write paths (domain tables + operation log) on every command. Acceptable; SQLCipher transactions handle this cheaply for our scale.
- The operation log grows monotonically. We accept this for a single-user vault; if we ever need archival/compaction, that's a separate ADR.
- "What happened in this vault?" requires reading the operation log, not the domain tables alone. Headless CLI exposes a query path (`pfc operations …`).

## Rejected alternatives

### Pure event sourcing

- ✗ Migration cost: every schema-changing command needs an event-version bump.
- ✗ Projection rebuilds become unavoidable for any read shape change.
- ✗ Debugging "event happened but state isn't right" is expensive.
- ✗ Single-user single-device app doesn't need the distributed-systems benefits.

### Audit log as a side-effect with no atomicity guarantee

- ✗ Drift between domain state and audit log is inevitable under any failure mode.
- ✗ Rebuilds become impossible; sync becomes impossible.

### Rely on Dolt-style branching of the SQLCipher DB

- ✗ Beads itself uses Dolt; that's a different scale and use case.
- ✗ User-facing finance app shouldn't pay the cost of multi-version history at the storage layer.
- ✗ Doesn't give us provenance per command; gives us per-commit.

### Append-only postings without an operation log

- ✗ Postings are domain rows; using them as a substitute for operation log conflates "what happened in the world" with "what changes were applied to our model of the world" (e.g., a category re-label is a real event but isn't a posting).

## Revisit if

- The operation log starts dominating vault size on realistic 5-year fixtures (we'd consider compaction, not removal).
- Multi-device sync requirements force a richer schema for replication (we extend, not rebuild).
- A genuine need for backwards time-travel queries appears (we already have the substrate; we'd just add a query layer).

## Implementation notes

- Schema bead: `personal-cfo-0s0` (`operation_log` + `projection_cursors` + `read_model_checksums`).
- Atomic writer is the kernel command bus (`personal-cfo-02i`).
- Headless CLI: `pfc operations log [--correlation=<id>]` for diagnostics (future bead).
- Cross-cutting test suite: log correlation end-to-end (`personal-cfo-1de5`).

## Linked beads

- `personal-cfo-0s0` (Schema: operation_log + projection_cursors + read_model_checksums)
- `personal-cfo-02i` (Implement command bus with idempotency + atomic op-log writes)
- `personal-cfo-29l` (ADR 0012: command idempotency and retry semantics)
- `personal-cfo-sab` (ADR 0006: Finance Kernel and command boundary)
- `personal-cfo-a2a` (ADR 0009: materialized read-model strategy)
- `personal-cfo-1de5` (Cross-cutting test suite: log correlation end-to-end)
- `personal-cfo-6kn` (ADR 0017: sync-readiness fields)
