# ADR 0013: Entity identifier strategy

- **Status:** Accepted
- **Date:** 2026-06-07
- **Deciders:** Project owner
- **Bead:** [`personal-cfo-i2t`](../../.beads/issues.jsonl)
- **Related plan sections:** §9.1.2
- **Supersedes:** None

## Context

Every entity in the vault (accounts, ledger accounts, transactions, operations,
and the schema types still to come — categories, forecast runs, income sources)
needs a primary identity. The kernel already minted UUIDv7 IDs via a `uuid_id!`
macro living in `core-ledger`, but the strategy was incomplete and inconsistent:

- IDs were stored as 36-char `TEXT` in SQLite — 36 bytes for 128 bits of data.
- There were no human-facing "reference code" display IDs.
- The macro was ledger-local, so future schema crates would have to depend on
  the whole ledger (or duplicate the macro) just to mint IDs.
- Generation used plain `Uuid::now_v7()`, which is not monotonic within a
  millisecond — `ORDER BY id` did not strictly reproduce creation order.

## Decision

A single shared crate, **`core-ids`**, owns identity for the whole workspace.

1. **UUIDv7, monotonic.** IDs are UUIDv7 (RFC 9562): time-ordered, globally
   unique, opaque. `core-ids` generates them through a process-global
   `ContextV7` (behind a `Mutex`, as it carries an interior counter), so IDs
   minted in sequence are strictly increasing even within one millisecond.
   ULID was considered and rejected (see below).
2. **The `uuid_id!` macro lives in `core-ids`.** Each domain/schema crate
   defines its own typed newtypes (`AccountId`, `TransactionId`, …) by invoking
   the shared macro, so there is one ID foundation and no dependency on the
   ledger. `core-ids` is a pure-types crate (no DB/async/Tauri), CI-enforced.
3. **Three representations, one per boundary:**
   - **Storage:** the compact 16-byte form (`as_bytes`), stored by db-worker as
     a SQLite `BLOB` via rusqlite's `uuid` feature. Entity primary keys and the
     foreign keys that join them are `BLOB`.
   - **Wire (IPC):** `#[serde(transparent)]` → the hyphenated UUID **string**,
     so the Tauri/TypeScript contract stays human-readable and stable.
   - **Display:** `display_id()` renders the 128 bits as 26-char Crockford
     base32 (no ambiguous `I/L/O/U`, case-insensitive, round-trippable) for
     user-facing reference codes (e.g. a transaction confirmation number).
4. **Command/operation provenance stays `TEXT`.** `operation_log.command_id` /
   `correlation_id` / `causation_id` and `ledger_transactions.operation_id` are
   command-bus provenance, not entity primary keys; they remain `TEXT` (and
   join consistently). `idempotency_key` and `node_id` are arbitrary strings and
   stay `TEXT`.

## Consequences

### Positive

- Entity-id storage halves (16 vs 36 bytes) and indexes are tighter.
- One ID foundation for every current and future schema crate.
- Strict creation-order sorting by primary key (monotonic generation).
- The IPC wire format is unchanged, so existing TypeScript bindings are
  untouched by the storage migration.

### Negative

- A mixed storage boundary (`BLOB` entity ids vs `TEXT` provenance UUIDs) must
  be respected when adding columns or joins; documented in the schema.
- A per-ID `Mutex` lock on generation. Uncontended and negligible in practice
  (a million IDs generate in well under a second).

## Rejected alternatives

### ULID instead of UUIDv7

- ✗ Non-RFC; needs a separate crate and ecosystem support.
- ✗ Its main advantage (compact base32 display) is handled here by `display_id`
  as a separate concern, so the storage type doesn't need to carry it.
- UUIDv7 is the RFC standard with first-class `uuid`-crate and rusqlite support.

### Keep IDs as `TEXT`

- ✗ 36 bytes per 128-bit id; larger indexes and storage.
- ✗ Leaves the "compact binary representation in DB" requirement (§9.1.2) unmet.

### Keep the macro in `core-ledger`

- ✗ Forces every non-ledger schema crate to depend on the ledger (or duplicate
  the macro) to mint IDs.

## Revisit if

- A future schema needs non-UUID identities (e.g. externally-assigned ids).
- Multi-device sync (ADR 0017) requires globally-coordinated id allocation.

## Implementation notes

- Crate: `crates/core-ids` — macro, monotonic generator, 16-byte codec,
  Crockford display codec, `IdError`. Pure-types; added to the CI boundary check.
- db-worker stores entity-id columns as `BLOB` (`SCHEMA_VERSION` bumped to 2;
  pre-data, so no migration framework needed — that is `personal-cfo-wkn`).
- Test: one million ids generate with no collisions and in non-decreasing order
  (`crates/core-ids/tests/ids.rs`); db-worker asserts the on-disk `BLOB`/16-byte
  storage (`entity_ids_are_stored_as_16_byte_blobs`).
