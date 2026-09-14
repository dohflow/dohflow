# ADR 0008: Staged ingestion and provenance model

- **Status:** Accepted
- **Date:** 2026-06-24
- **Deciders:** Project owner
- **Bead:** [`personal-cfo-z2a`](../../.beads/issues.jsonl)
- **Related plan sections:** §8.1.1 (ingestion pipeline), §9.12 (provenance and source records)
- **Builds on:** ADR 0014 (Money Inbox + ingestion *philosophy*), ADR 0022 (parser isolation), ADR 0012 (command idempotency), ADR 0011 (hybrid ledger + operation log), ADR 0009 (materialized read models), ADR 0007 (ledger/posting model), ADR 0027 (additive balance — the reconciliation "plug")
- **Supersedes:** None

## Context

R2 turns the manual tracker into an **importable** one. Every inbound path — CSV/OFX
importers now, statement extractors and connectors later — produces records from
**outside** the trust boundary. We resolved the *product* questions in **ADR 0014**:
auto-commit the clean rows, triage only the exceptions in the Money Inbox, dedupe in two
layers without ever silently dropping, and shred the source file after parsing. ADR 0022
settled *how* untrusted bytes are parsed (sandboxed, bounded, hardened).

What is **not yet recorded** is the **data model** those decisions rest on: the staging
tables an importer writes, the lifecycle a batch moves through, how provenance pins a
committed transaction to the record it came from, and how dedupe decisions are stored so
they are auditable and replayable. The schema bead (`personal-cfo-ihe`) implements
exactly this, so the model is an architecturally-significant decision in its own right —
this ADR records it. ADR 0014 owns the *philosophy*; **ADR 0008 owns the *schema*.**

The plan's original §9.12 sketch predates ADR 0014 and ADR 0027; this ADR **reconciles**
it (drops raw-payload retention, drops `household_id`, and recasts the batch state
machine around auto-commit-clean).

## Decision

### 1. One staging substrate; importers never write the ledger directly

Every source parses into **staged** rows and hands them to a shared commit pipeline. No
importer, extractor, or connector writes a committed `ledger_transaction` /
`ledger_posting` (ADR 0007) directly. The substrate is eight tables:

| Table | Role |
|-------|------|
| `source_batches` | one ingestion event (a file import, a sync) + its lifecycle status |
| `source_records` | one parsed row / provider object: its `source_hash` + normalized fields |
| `parser_runs` | one parser execution against a batch: tool, version, bounds, outcome (ADR 0022) |
| `staged_transactions` | a proposed transaction awaiting commit/dedupe |
| `staged_accounts` | an external account observed in the source, to be matched to a real account |
| `staged_balances` | an observed ending balance → becomes a `balance_observations` assertion (ADR 0027) |
| `dedupe_decisions` | every committed/skipped/merged/flagged decision + reason (replayable) |
| `source_provenance_links` | the permanent link from a committed entity back to its `source_record` |

> **Naming note.** The plan's §9.12 calls this table `provenance_links`, but that name
> is already taken: the baseline schema (ADR 0011) has a `provenance_links` table that
> indexes *operation → entity* (the op-log's per-entity index). Import provenance —
> *entity → source_record* — is a distinct relationship, so it lives in
> **`source_provenance_links`** to avoid clobbering the existing table. Both forms of
> provenance coexist.

The `staged_*` tables are **TRUNCATE-safe**: discarding a batch's staged rows touches no
canonical state (accounts, ledger, balances). Staging is scratch space; the ledger is truth.

### 2. The `source_batch` lifecycle (reconciled with auto-commit-clean)

A batch moves through:

```
parsing → staged → committed
                 ↘ partially_committed     (some rows clean-committed, some flagged to the inbox)
                 ↘ discarded               (user threw the batch away before/at commit)
       ↘ failed                            (parse/limit/error — ADR 0022 bound hit)
committed/partially_committed → superseded (a later import replaces it)
```

There is deliberately **no `reviewing` batch state**. ADR 0014 ratified that *clean rows
commit automatically*; review is **per-record**, surfaced as Money-Inbox items for the
flagged minority — not a gate the whole batch waits behind. The batch status only records
**aggregate progress** (`committed` vs `partially_committed`), so a clean 300-row import
goes `parsing → staged → committed` with zero inbox friction.

### 3. Shred after parse: keep the fingerprint, not the file

Per ADR 0014 §4 the raw uploaded bytes are **never persisted**. We therefore **drop** the
plan's `raw_payload_ref` / `raw_payload_encrypted` columns. What `source_records` keeps is
the **`source_hash`** (content fingerprint, for file-level dedupe and "already imported on
<date>") and **`normalized_json`** (the extracted, typed fields) — never the original
document. Retaining a statement as a document is a separate, explicit opt-in through the
encrypted attachment store (ADR 0023), never the import default.

### 4. Provenance is first-class and FK-strict

Every committed entity that originated from an import carries a `source_provenance_links`
row pointing back at its `source_record`, **forever**. The link is **FK-strict**:
`source_provenance_links.source_record_id` is a real foreign key to `source_records(id)`, and the
commit pipeline upholds the invariant *every import-sourced committed `ledger_transaction`
has a `created_from` provenance row* (enforced in the pipeline, asserted in tests). The
`relationship` is a typed enum — `created_from`, `amended_by`, `inferred_from`,
`confirmed_by`, `contradicted_by`, `superseded_by` — so later edits, rules, and
reconciliations append to an entity's provenance rather than overwrite it. (`entity_type`
starts at `ledger_transaction` and is extensible; the per-batch and per-record FKs inside
the staging substrate are likewise declared, regardless of the connection's
`PRAGMA foreign_keys` setting, so the schema documents the intended integrity.)

### 5. Dedupe decisions are recorded and replayable

The two layers from ADR 0014 §3 each write a `dedupe_decisions` row:

- **File layer** — the whole-file `source_hash` matched an earlier batch ⇒ an exact
  re-upload is auto-skipped with a notice.
- **Transaction layer** — a per-transaction fingerprint (date + amount + normalized
  merchant + account) matched a committed *or* staged transaction ⇒ the row is **flagged**
  in the Money Inbox ("possible duplicate — skip, or import anyway"), never silently
  dropped.

Each decision records its `layer`, the matched entity, the `decision`
(`committed` / `skipped` / `merged` / `flagged`), and a human `reason`, so the dedupe of
any batch is auditable and re-derivable. The transaction-level matcher is the same one
that powers progressive reconciliation (ADR 0027): imported transactions explain — and
shrink — the additive-balance "plug" (`dyy4`).

### 6. Commit goes through the kernel, idempotently

Promoting staged rows to the ledger is a **kernel command** (ADR 0006) and is
**idempotent** (ADR 0012): re-running a commit (crash, retry, double-click) does not
double-post. The commit writes the domain ledger rows, their `source_provenance_links`, and the
operation-log entry **atomically** (ADR 0011), then refreshes the affected read models
(ADR 0009). *(This ADR fixes the model; the commit pipeline itself is built by the
unified-ingestion bead `cmx` with the first importer — `ihe` lays only the substrate.)*

### 7. Conventions

Single-household (no `household_id`); BLOB UUIDv7 primary keys; integer **minor units**
for money; ISO-8601 **TEXT** dates; TEXT-token `CHECK` enums — matching the existing
db-worker schema (ADR 0007, migrations framework).

## Consequences

- A single, auditable, deduplicated path from **any** source to the ledger; statement
  parsing and connectors reuse it unchanged.
- **No sensitive source files at rest** — the exposure window is one in-memory parse.
- Every committed import is traceable to its origin forever; nothing in the ledger is
  un-sourced.
- The user only ever touches the ambiguous minority; clean imports are silent.
- Cost: importers must go through staging + the kernel commit; they cannot shortcut
  straight into the ledger. That is the point.

## Alternatives considered

- **Importers write the ledger directly.** Rejected: bypasses audit, idempotency, shared
  dedupe, and provenance — every importer would re-implement (and eventually mis-implement)
  safety.
- **Event-sourced ingestion with no staging tables.** Rejected: no place to preview,
  partially commit, or triage; the Money-Inbox review UX (ADR 0014) needs durable staged
  candidates, not a replay-only stream. (Consistent with ADR 0011's rejection of full
  event sourcing as the canonical store.)
- **Trust-the-source dedupe (rely on provider/import IDs only).** Rejected: provider data
  drifts — IDs change across exports, overlapping date ranges re-export the same
  transactions with different identifiers. We fingerprint content and record the decision.
- **Keep the raw payload for re-parsing.** Rejected: unnecessary sensitive data at rest,
  against ADR 0014's shred-after-parse posture; retention is an explicit ADR-0023 opt-in.

## Revisit if…

- **Real-time connector streams** arrive (push/webhook relay) and a batch-oriented
  `source_batches` shape no longer fits a continuous feed — a streaming staging variant
  may be needed alongside the batch model.
- A future tier needs to **re-parse** originals (e.g. improved statement extraction),
  which would reopen the shred-after-parse decision (still an explicit attachment opt-in,
  not default retention).
