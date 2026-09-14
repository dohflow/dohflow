# ADR 0012: Command idempotency and retry semantics

- **Status:** Accepted
- **Date:** 2026-05-04
- **Deciders:** Project owner
- **Bead:** [`personal-cfo-29l`](../../.beads/issues.jsonl)
- **Related plan sections:** §9.1.2, §2.6
- **Supersedes:** None

## Context

A finance app must never silently double-charge a transfer, double-record a paycheck, or double-categorize a transaction because of a retry, a crash, an interrupted sync, or a UI re-submit. Even on a single-user, single-device vault, retries happen: import jobs restart, agent runs are re-invoked, the user clicks a button twice, the OS suspends the app mid-transaction. Some of these surface as user-visible bugs; the silent ones are worse.

The Finance Kernel (ADR 0006) is the single chokepoint for state mutations, so idempotency is enforced there.

## Decision

Every kernel command carries an **idempotency key**, scoped per command type. Replaying a command with a previously-seen idempotency key returns the original result and writes nothing new.

### Required per-command metadata

(Recap from §9.1.2 and ADR 0006 — this ADR is the authoritative source.)

| Field | Type | Notes |
| --- | --- | --- |
| `command_id` | UUIDv7 | Server-side generated. Globally unique. |
| `idempotency_key` | bytes | Caller-supplied. Unique per command type. |
| `actor_type` | enum | `user`, `importer`, `connector`, `agent`, `system`, `scheduled_job` |
| `actor_id` | string | Identifies the actor instance (e.g., user profile id, importer plugin name + version). |
| `vault_schema_version` | int | Rejected if mismatched against the current vault. |
| `node_id` | UUID | Vault instance (device) identifier. |
| `hlc_timestamp` | bytes | Hybrid logical clock value. |
| `causation_id` | UUID? | command_id of the parent command, if any. |
| `correlation_id` | UUID? | Trace correlation across a multi-step operation (e.g., importer batch). |

### Idempotency key generation

The caller is responsible for constructing a deterministic key for the operation it intends to perform:

| Caller | Idempotency key strategy |
| --- | --- |
| User UI | `hash(user_id, route, form_submit_token, current_state_version)` — a fresh token is generated per form render, so resubmitting the same form is idempotent but a new form is a new operation. |
| Importer | `hash(importer_name, source_batch_id, source_record_id, intended_command_type)` — re-running the same import on the same source is a no-op. |
| Connector | `hash(connector_name, provider_account_id, provider_transaction_id, posting_kind)` — provider-side idempotency is inherited where available. |
| Agent | `hash(agent_run_id, intended_command_type, target_entity_id)` — agent re-runs are idempotent; agents can't write financial records anyway, so this applies to staged-candidate / report commands. |
| Scheduled job | `hash(job_name, run_at_bucket, target_entity_id)` — daily jobs bucket by date, hourly by hour, etc. |

The kernel does **not** invent idempotency keys. Callers that don't supply one are rejected with a typed error.

### Replay semantics

```
on dispatch(cmd, meta):
    existing = command_idempotency_keys.lookup(meta.idempotency_key, cmd.type())
    if existing is not None:
        if existing.command_type != cmd.type():
            return Err(IdempotencyKeyTypeMismatch)
        if existing.expired():
            return Err(IdempotencyKeyExpired)   -- treat like a new operation requires fresh key
        return Ok(existing.result_ref)          -- replay → no new ledger writes, no new op-log row
    txn = open transaction
    cmd.validate(ctx)                            -- fail before any write
    outcome = cmd.apply(txn)
    insert operation_log row
    insert command_idempotency_keys row { key, command_id, command_type, result_ref, expires_at }
    commit txn
    return Ok(outcome)
```

The `command_idempotency_keys` insert is part of the same transaction as the domain writes and the operation_log row. There is no path where a command appears to succeed but the idempotency record is missing.

### Retention / GC

`command_idempotency_keys.expires_at` is set per command type:

- High-frequency commands (UI form submits, agent runs): 7 days.
- Importer batch commands: 90 days (long enough that re-importing a forgotten CSV is still safe).
- Connector sync commands: provider-determined or 30 days.
- Schema migration commands: forever.

Expired rows can be reaped by a periodic vacuum. Reaping is safe because canonical state survives — the only thing lost is the ability to detect a replay older than the TTL. The TTL is calibrated so the user-visible replay window covers realistic re-run behavior.

### Causation and correlation

- `causation_id` chains commands: a `commit_staged_batch` command that produces N follow-up `categorize` commands sets `causation_id = commit_staged_batch.command_id` on each.
- `correlation_id` groups all commands from one user action / batch / agent run for log correlation. The cross-cutting test suite `personal-cfo-1de5` exercises this end-to-end.

## Consequences

### Positive

- Deterministic, idempotent retry across user, importer, connector, agent, and scheduled-job paths.
- Operation log + idempotency keys together support a future replay-and-reconcile sync model.
- Clear contract for callers: supply a deterministic key, retry as needed, never double-write.
- Tracing spans + correlation_id give debugging across multi-command operations without ad-hoc tagging.

### Negative

- A `command_idempotency_keys` table with unbounded growth without GC. Acceptable with a TTL policy.
- Callers must produce good idempotency keys; bad keys (random per call) defeat the system. Mitigated by per-actor strategy guidance in this ADR and by code review.
- Replay returns the *original* result, which may surprise a caller expecting current state. Callers that need fresh state run a query, not a replay.

## Rejected alternatives

### Timestamp-only deduplication

- ✗ Two commands at the same timestamp (UI fast-clicks) are indistinguishable.
- ✗ Clock skew across devices breaks the model (relevant once we add sync).

### Client-generated request IDs only, no server-side tracking

- ✗ Clients can lose state and re-issue with a new ID.
- ✗ No defense against double-submit from form re-render.

### Database UNIQUE constraints on natural keys (e.g., `(provider_account_id, provider_transaction_id)`)

- ✗ Catches one specific case (connector dupes) but not user re-submits, agent re-runs, importer re-imports.
- ✗ Schema-coupled, brittle.
- We use these as a *secondary* defense at the importer/connector layer, not as the primary idempotency mechanism.

### Nonce-based protocol with periodic key rotation

- ✗ Solves a problem we don't have (cryptographic replay across an authenticated channel).
- ✗ Doesn't address logical idempotency (same operation, replayed).

## Revisit if

- A specific command type can't form a deterministic idempotency key from caller-side state (we'd document the caller-side discipline change, not weaken the kernel rule).
- The TTL policy lets a real-world replay slip through (we'd extend the TTL or refine the strategy).
- Multi-device sync demands a different replay mechanism (ADR 0017 will handle that — likely an extension here, not a replacement).

## Implementation notes

- Schema bead: `personal-cfo-2l2` (`command_idempotency_keys`).
- Kernel command bus bead: `personal-cfo-02i`.
- Property test: replaying a command returns identical `result_ref` with no duplicate ledger postings (covered in `personal-cfo-02i` AC).
- Tracing: every `dispatch` emits a span with `command_id`, `idempotency_key` (hash, not raw — see logging policy `personal-cfo-2vs`), `actor_type`, outcome.

## Linked beads

- `personal-cfo-02i` (Implement command bus with idempotency + atomic op-log writes)
- `personal-cfo-2l2` (Schema: command_idempotency_keys)
- `personal-cfo-sab` (ADR 0006: Finance Kernel and command boundary)
- `personal-cfo-7vq` (ADR 0011: hybrid ledger + operation log)
- `personal-cfo-6kn` (ADR 0017: sync-readiness fields)
- `personal-cfo-1de5` (Cross-cutting test suite: log correlation end-to-end)
