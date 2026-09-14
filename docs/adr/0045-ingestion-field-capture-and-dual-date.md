# ADR 0045 — Ingestion field capture, dual-date semantics, and imported-category prefill

- Status: Accepted
- Date: 2026-07-07
- Bead: personal-cfo-4d8.24.1 (import loses fields) — foundation for the 2026-07-07 Transactions wave (epic 4d8.24)
- Builds on: ADR 0014 (ingestion pipeline, §3 dedupe, §4 shred-after-parse), ADR 0022 (importer isolation), ADR 0027 (additive balances), ADR 0030 (categorization + `import_alias` source vocabulary)

## Context

Owner dogfooding (2026-07-07) imported real **CapitalOne credit-card CSV** — columns: `Transaction Date`,
`Posted Date`, `Card No.`, `Description`, `Category`, `Debit`, `Credit`. Three problems surfaced:

1. **Wrong primary date.** The CSV importer's `resolve_columns` matches the *first* header containing "date"
   (`csv-importer/src/lib.rs`), so **`Transaction Date`** becomes the posting date and **`Posted Date`** is
   dropped from the structured record. But the *posted* date is when the money actually moved — it must be the
   primary date used everywhere (list, forecast anchoring, dedupe).
2. **No place for the second date.** `ParsedTransaction` (importer-core) and `staged_transactions` /
   `transaction_details` (db-worker) each carry a **single** date, so even a correctly-chosen posted date leaves
   nowhere to keep the transaction/authorization date.
3. **Captured but invisible.** Every source column *is* retained: `csv-importer::normalized_json` writes the full
   header→value map into `source_records.normalized_json`, linked to the committed transaction via
   `source_provenance_links` (relationship `created_from`). So no field is lost *at rest* — but none of it
   (posted date, card no., the source's own category) is **surfaced**, and the pre-defined **category** is
   ignored rather than used as a starting point.

## Decision

**1. Dual-date semantics — posted is primary.** A transaction carries two dates:

- **Posted date** — when the money actually moved. This is the **primary** date: it is the account's
  `occurred_at` / `staged_transactions.posted_at`, drives the list's main date column, forecast anchoring, and the
  dedupe fingerprint. Unchanged plumbing; only the *choice* of which source column feeds it is corrected.
- **Transaction date** (a.k.a. authorization date) — when the purchase was authorized. **Secondary, nullable,
  descriptive** — shown in the detail view, never used for math or dedupe.

Source mapping: CSV prefers a header containing **"posted"** for the posted date, and a *different* date column
(e.g. "transaction"/"authorization") for the transaction date; when only one date column exists it is the posted
date and the transaction date is null. OFX maps `DTPOSTED` → posted, and `DTUSER` (when present) → transaction.
`ParsedTransaction` gains `transaction_date: Option<NaiveDate>`; a nullable `transaction_date` column is added to
`staged_transactions` and to `transaction_details` (committed side) — migration v38, additive, existing rows null.

**2. Raw-field capture is retained AND surfaced (no silent loss).** The existing `source_records.normalized_json`
(the full header→value map) is the canonical "nothing dropped" store; it is already linked to the committed
transaction via `source_provenance_links`. This ADR makes it a **contract**: every importer MUST write every
source field into `normalized_json`, and the transaction detail view MUST surface those imported fields
(posted date, transaction date, card no., original category, and any other columns) as read-only "imported
details" resolved through the provenance link. A field being absent from the structured schema is fine **iff** it
is retrievable from `normalized_json`.

**3. Imported category is a prefill, not a user choice.** When a source row carries a category, the commit path
records it as a categorization with source token **`import_alias`** (the existing ADR 0030 import vocabulary — we
do NOT add a fourth source term) at **reduced confidence**, so it (a) pre-fills the transaction's category,
(b) surfaces in the Money-Inbox low-confidence / review flow as a suggestion the user confirms or corrects, and
(c) is never treated as an authoritative `user` categorization. A user edit supersedes it normally.

The source's category **string** is matched to a real category by **name** (case-insensitive, non-archived).
Category names are **not** globally unique — the seeded taxonomy repeats leaf names (e.g. "Maintenance" under both
Housing and Transportation), and users may add same-name categories. When a name matches **more than one**
category, the prefill is **skipped** (left uncategorized) rather than guessing the wrong one: a wrong low-confidence
suggestion is worse than none, and the raw imported category is always still visible in the transaction's Imported
details. A fuzzier/deterministic mapping (parent-qualified names, per-institution category maps) is future work.

## Consequences

- The posted-date correction changes the dedupe fingerprint's date for CapitalOne-shaped CSVs (posted vs
  transaction date) — correct going forward; already-imported rows are unaffected (dedupe is per-import).
- The dual-date columns are additive/nullable and rebuild-safe; net-worth and forecast math are unchanged (they
  already key on the posted/`occurred_at` date).
- Because the "no-loss" guarantee rests on `normalized_json` + the provenance link, the detail view's "imported
  details" section is the user-facing proof; importer tests assert every source column is round-trippable from
  bytes → committed transaction.

## Implementation slices (personal-cfo-4d8.24.1 + children)

This ADR is the whole contract; the code lands incrementally so each PR stays reviewable:

- **Slice 1 (this bead's first PR):** dual-date end-to-end — `ParsedTransaction.transaction_date`, CSV posted-
  primary + transaction-secondary, OFX `DTUSER`, migration v38, staging + commit carry-through, and the detail
  view showing both dates plus the raw imported fields from `normalized_json`.
- **Slice 2 (child bead):** imported-category prefill on commit (`import_alias` source, reduced confidence, review
  flow) — decision §3 above.
- **Slice 3 (child bead):** a column-mapping UI so a user can remap arbitrary CSV headers when auto-detection is
  imperfect (importer-core already has `ColumnMapping`/`ParserHints`; this exposes it).
