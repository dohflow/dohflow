# ADR 0014: Money Inbox and the ingestion pipeline

- **Status:** Accepted
- **Date:** 2026-06-24
- **Deciders:** Project owner
- **Bead:** [`personal-cfo-0th`](../../.beads/issues.jsonl)
- **Related plan sections:** §8.1.1 (ingestion staging), §9.12 (provenance + dedupe), §17.1 / §18.1.1 (Money Inbox)
- **Builds on:** ADR 0009 (materialized read models), ADR 0027 (additive balance — the reconciliation "plug"), ADR 0022 (parser/document isolation — how untrusted files are parsed)
- **Companion:** ADR 0008 (the staged-ingestion + provenance *data model* this pipeline commits through — the `ihe` staging schema)
- **Contrasts with:** ADR 0023 (the encrypted attachment store keeps files; imports are ephemeral)

## Context

R2 turns the manual tracker into an **importable** one: the user brings transactions
from outside (CSV/OFX now; statements and connectors later). Every such path raises
the same four problems — *don't corrupt the ledger with duplicates*, *don't keep
sensitive source files lying around*, *keep the user in control of the ambiguous
cases*, and *stay auditable*. We solve them once, in a single pipeline, rather than
per importer. The **Money Inbox** is the surface where the human handles only what the
machine can't decide.

## Decision

### 1. One ingestion pipeline; importers never write the ledger directly

Every source — importers (R2), document extractors and connectors (later) — parses
into **staged** records and hands them to a shared commit pipeline (the `ihe` staging
schema). No source writes committed ledger rows directly. Every committed transaction
carries **first-class provenance**: a link back to its source batch and source record.

### 2. Commit model: auto-commit clean, triage the exceptions (ratified)

A staged transaction **commits automatically** when it (a) parsed cleanly, (b) maps to
a known account, and (c) is not a suspected duplicate. Anything else — a possible
duplicate, an unmatched/ambiguous account, a low-confidence row — becomes a **Money
Inbox item** the user resolves. The Money Inbox is the single **triage surface for
data-quality work**, not a per-row approval queue: a clean 300-row statement import
should produce zero inbox items and zero friction.

### 3. Two-layer deduplication; flag, never silently drop (ratified)

- **File level.** A content fingerprint (hash) of the uploaded bytes. An exact
  re-upload is **auto-skipped** with an "already imported on <date>" notice.
- **Transaction level.** A per-transaction fingerprint (date + amount + normalized
  merchant + account) matched against both committed and staged transactions — this is
  what catches *overlapping* exports whose files differ. A suspected duplicate is
  **flagged in the Money Inbox** ("possible duplicate — skip, or import anyway"),
  never silently dropped, so a false-positive match can't swallow a real transaction.

Dedupe decisions are stored and reviewable. The transaction-level matcher is also what
powers progressive reconciliation (§6).

**Addendum (2026-09-02, `personal-cfo-tevp`): a third, cross-source layer.**
Fingerprints are source-shaped — a CSV row hashes its description, a connector
row its provider id — so the exact-match layer is structurally blind across
sources, and a CSV backfill of an account that later syncs (or vice versa)
would double-book every overlapping transaction. Commit therefore adds a
heuristic layer: a row whose **account + posted date + signed amount** match a
committed, non-voided transaction from a **different source type** (or a
manual, provenance-free entry) is FLAGGED for Money Inbox review — never
silently dropped (two same-day identical charges are legitimately possible),
never auto-merged. Within one source type the layer is inert: distinct
fingerprints from the same source mean the provider or importer itself
vouches the rows are distinct (two identical coffees in one sync are both
real). `force` (import-anyway) bypasses it like the other layers. The
connector certain-duplicate pre-check (provider-id refetch overlap, gglk)
still runs first, stays silent, and now also treats a fingerprint sitting
**flagged or skipped** as tracked — a rewind re-fetch of an unresolved (or
deliberately skipped) collision must not mint a fresh inbox item on every
sync. Each flag records its matched ledger transaction on the dedupe
decision, so the review panel shows the counterpart even though fingerprints
never match across sources. Known accepted noise: provenance-free postings
that are not hand-typed entries (opening balances, early-confirmed
obligations, void reversals) can also match; the reason copy says "already
in this account's ledger" rather than claiming a manual entry.

### 4. Shred after parse: keep the fingerprint, not the file (ratified)

The raw uploaded file is **transient**: read into memory, parsed in an isolated worker
(ADR 0022), and **never written into the vault**. What persists is the extracted
transactions, their provenance, and the content fingerprint — *not the bytes*. This is
a deliberate departure from ADR 0023 (attachments are kept on purpose; imports are
ephemeral). If a user later wants to retain an original statement as a document, that
is a separate, **explicit opt-in** that uses the encrypted attachment store — never the
default for an import.

### 5. Deterministic insight cards are the primitive; narration is later

Money Inbox items and the insights built on them are **deterministic, rule-based, and
explainable** — no LLM required to function. Optional LLM *narration* of an already-
deterministic result is a later, clearly-bounded addition (ADR 0018 non-advice).

### 6. Imports feed progressive reconciliation

Committed imported transactions flow into the additive-balance model (ADR 0027): they
**explain the unexplained "plug"**, shrinking it as real activity is imported
(`dyy4`). Importing is how a hand-asserted balance becomes a transaction-backed one.

### 7. Money Inbox persistence & generation (ratified 2026-06-24)

The decisions above describe the *model*; this section settles how it is **stored and
generated**, which the schema beads (`dsq`, `3d3`) left ambiguous.

- **One materialized read model, `money_inbox_read_model`,** is the canonical item
  store. It follows the established projection pattern (ADR 0009): a deterministic
  `rebuild_in` that clears and re-derives every item from canonical state, recording a
  `projection_cursors` row + a `read_model_checksums` entry — exactly like
  `transaction_display` and `commitments`. It is *materialized* rather than a live query
  because it must hold per-item triage **state** (snoozed/dismissed/resolved). The
  earlier `review_queue_items` table (bead `3d3`) is **subsumed** by this one table — we
  do not keep two parallel item stores.

- **Deterministic typed generators populate it — one per item kind.** Each generator is
  a pure function of canonical state (no agent writes), so the projection is rebuildable
  and drift-checkable. The **imported-transactions-waiting-commit** generator is the
  first: it emits one item per `staged_transactions` row with `commit_status='flagged'`
  (a suspected duplicate or unmatched-account exception from §2/§3), carrying the
  `dedupe_decisions` reason + the suspected committed counterpart in `payload_json`. The
  item's id and `surfaced_at` derive from the staged row (its id and `created_at`), so a
  rebuild is byte-identical. The other eight kinds (low-confidence categories, possible
  transfers/recurring bills, stale balances, document extractions, forecast-assumption
  attention, connector errors, reconciliation discrepancies) are deferred beads that add
  a generator each — no schema change.

- **Resolution is intrinsic where canonical state already encodes it.** For the
  imported-waiting-commit kind, *import-anyway* flips the staged row to `committed` and
  *skip* flips it to `skipped`; either way it is no longer `flagged`, so the next rebuild
  simply omits the item. Soft actions that do **not** change canonical state —
  **snooze** (hide until a date) and **dismiss** (with a typed reason) — are kernel
  commands recorded in `change_journal_entries` (bead `3d3`) and re-applied to the
  projection on rebuild by `target_id`. `change_journal_entries` is also the per-action
  audit trail (`ci71`).

- **Time-derived kinds are computed on read, not materialized (addendum 2026-06-27,
  `r52x`).** A few item kinds — **stale balances** first — become true purely because the
  *wall clock advanced*, with no canonical write to trigger a rebuild. Materializing them
  would (a) require a rebuild on a timer and (b) make the read model's `read_model_checksums`
  entry clock-dependent, breaking the determinism the drift check relies on. So these kinds
  are **derived on read** — computed from canonical state + `as_of` inside `money_inbox_list`
  and merged with the materialized event-driven items — exactly like the
  derived-on-demand `cash_availability` and `forecast_readiness` snapshots. They carry **no
  per-item triage state** (snooze/dismiss applies to materialized items only); resolution is
  intrinsic (record a fresh balance, or archive the account). The stale-balance threshold is
  per account role (liquid 14d / credit + loan 30d / investment + asset 60d); user-configurable
  thresholds are a later refinement.

## Consequences

- A single, auditable, deduplicated path from any source to the ledger; the same
  pipeline serves future statement parsing and connectors.
- **No sensitive source files at rest** — the exposure window is one in-memory parse.
- The user only ever touches the ambiguous minority; large clean imports are silent.
- Requires the staging schema (`ihe`), a transaction fingerprint, and the Money Inbox
  read model + UI before any importer is user-visible.

## Alternatives considered

- **Review every imported row.** Rejected: unacceptable friction at statement scale;
  the inbox should hold *problems*, not everything.
- **Auto-skip duplicates silently.** Rejected: a wrong match would drop a real
  transaction with no trace (two identical $5.00 coffees on one day are not a dup).
- **Keep source files as attachments by default.** Rejected: unnecessary sensitive
  data at rest, against the privacy posture; retention is opt-in, not default.
- **Let importers write the ledger directly.** Rejected: no shared dedupe, provenance,
  or triage; every importer would re-implement safety.

## Decisions ratified (2026-06-24)

- Commit model: **auto-commit clean, triage exceptions**.
- Dedup: **flag suspected duplicates in the Money Inbox**; exact whole-file re-uploads
  auto-skip with a notice.
- Retention: **shred after parse** — persist the fingerprint + extracted records, never
  the uploaded file.
