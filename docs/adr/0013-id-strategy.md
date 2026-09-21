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

## Addendum A (2026-09-20, personal-cfo-1df8d): deterministic derived identifiers for apply-minted rows

### Context

The multi-device trigger in ADR 0013's revisit clause has fired: SYNC-1 and
SYNC-1a replay the same command on more than one device.  A process-local
`ContextV7` counter cannot be part of a replicated result; two devices can see
the same millisecond and still mint different IDs.  This addendum narrows the
change to IDs minted while applying a versioned command payload.  It does not
rewrite existing rows, turn the operation log into an event stream, or change
the caller-owned IDs already carried by a payload.

### Decision

#### 1. Layout and derivation

An ID minted by `apply` is a UUIDv7 whose fields are a pure function of the
command metadata, the entity tag, the mint ordinal, and the vault's public
identifier key:

```text
unix_ms = CommandMeta.issued_at as Unix milliseconds
rand_a  = ordinal (12 bits, 0..=4095)
rand_b  = first 62 bits of
          HMAC-SHA256(id_key, command_id || tag || ordinal)
```

`command_id` is its canonical 16-byte UUID representation, `tag` is a stable
ASCII entity-domain label (length-prefixed in the canonical input), and
`ordinal` is a big-endian `u16`.  The HMAC input is therefore unambiguous and
the same on every device.  The first 62 output bits are interpreted
big-endian; the UUID variant bits remain the RFC 9562 variant bits.  The
implementation constructs the value with uuid 1.23.2's
`Builder::from_unix_timestamp_millis`, supplying the 10-byte counter/random
block whose `rand_a` and `rand_b` fields are fixed above.

The 32-byte, non-secret `id_key` is fixed as:

```text
id_key = SHA-256(ASCII("dohflow/derived-id-key/v1") || vault_id.bytes)
```

where `vault_id.bytes` is the canonical 16-byte UUID.  Genesis carries this
value beside `vault_id`; a receiver recomputes it and rejects a mismatch
before applying the snapshot.  It is an identifier-domain constant, not a
credential or encryption key, so exposing it does not weaken vault secrecy.
Changing the label or encoding is a payload-schema change and is a revisit
trigger below.  The UUID is consequently a function of the command, not of a
device, process, wall-clock read, or replica base.

#### 2. Metadata and time boundary

`issued_at` is added to `CommandMeta` at the command-construction boundary and
is persisted in the v2 envelope.  `apply` receives `(payload, meta, base)` and
never reads the wall clock.  The current inventory is the 21 `Utc::now()` sites
under `crates/db-worker/src/apply/` (accounts, categorization commands,
ingestion commands, recurring commands, scenario promotion, and transactions);
SYNC-1a replaces each with `meta.issued_at` and tests that no apply path calls
the clock.

The six kernel-side metadata/dispatch sites to update are the current
`next_meta` constructor and its five production dispatch families:

1. `crates/finance-kernel/src/lib.rs:2501` (`next_meta`),
2. `crates/finance-kernel/src/lib.rs:2698` (`ingest_batch` creates a source batch),
3. `crates/finance-kernel/src/lib.rs:2713` (`ingest_batch` failure state),
4. `crates/finance-kernel/src/lib.rs:2735` (`ingest_batch` staged-commit loop),
5. `crates/finance-kernel/src/lib.rs:2747` (`ingest_batch` final state), and
6. `crates/finance-kernel/src/lib.rs:2814–2865` (`ingest_sync_batch` dispatches).

The IPC ingress that supplies the initial metadata is
`apps/desktop/src-tauri/src/ipc/commands.rs:82` (`user_meta_with_key`).  Test
helpers are not production mint sites.  SYNC-1 records the field and these
construction boundaries; SYNC-1a performs the site-by-site apply rewrite.

#### 3. Mint-site rule and the ContextV7 boundary

Reviewers can classify every mint site with one rule: if a persistent entity ID
is allocated on the call path below `crates/db-worker/src/apply/`, it must use
the derived helper above and consume exactly one command ordinal.  This
includes typed `Id::new()` calls, direct `Uuid::now_v7()` calls, and helpers
reached by apply, including `ensure_system_ledger_account` and scenario
promotion.  The keyless recurring-suppression result token is also derived
when returned from apply; it is not a persistent entity and must never be
used as a replicated row identity.

The exception is an ID already present in the command payload: the three
caller-minted IDs remain caller-minted and are copied, not regenerated:
`RecordTransaction.transaction_id`, `CreateSourceBatch.id`, and
`AttachSourceRecord.id` (`crates/finance-kernel/src/lib.rs:199–216`,
`:1453–1474`, and `:1514–1539`; the historical plan references these as
`lib.rs:288`, `484`, and `499`).  Command/operation provenance IDs made
outside apply (`apps/desktop/.../commands.rs:82` and
`crates/finance-kernel/src/lib.rs:2501`), open-time seeders, and device-local
rows remain on the existing monotonic `ContextV7` path.  `ContextV7` is
therefore retired only for apply-minted IDs, not globally.

#### 4. Creation order and replay

The ordinal is allocated in the order an apply path would otherwise mint IDs,
so `ORDER BY id` retains creation order within one command.  The command's
payload, metadata, and base determine the sequence; a rollback-then-replay
reaches the same sequence on every replica.  No receipt or device mapping is
needed to make allocation deterministic.

#### 5. Ordinal cap and validation

`rand_a` permits 4,096 ordinals (`0..=4095`) per command.  Allocation of a
4,097th ID fails before the first database write with the typed validation
error `DerivedIdOrdinalOverflow { command_id, attempted: 4096 }` (serialized
as a validation failure in the command envelope).  SYNC-1a must assert both
the 4,096-success and 4,097-failure boundaries.  The largest known minting
*command family* is an import-batch commit: the staged-row commit loop can
mint one or more entity IDs per staged row and has no schema-fixed maximum;
the current fixtures top out at two records, and production telemetry has not
recorded a batch near the cap.  The implementation must record the observed
maximum when the SYNC-1a inventory test is added rather than silently raising
the cap.

#### 6. Create-if-missing, upserts, and seeding

Rollback-then-replay first finds a singleton on the new base.  It does not
mint a second ID when the row already exists.  The nine idempotent paths in
the current apply inventory are the helper plus the eight SQL upsert/ignore
statements (the plan's “nine upsert/ignore statements” count includes the
helper's create-if-missing branch):

1. `ensure_system_ledger_account` (`crates/db-worker/src/lib.rs:4515`), called
   from `apply/accounts.rs:67`, `apply/ingestion_cmds.rs:236`, and
   `apply/transactions.rs:60`, `:202`, `:360`: select the `(role,currency)`
   singleton first; derive an ID only on the insert path.
2. `apply/recurring.rs:443`, `recurring_event_tags` `INSERT OR IGNORE`:
   join row only; no ID mint.
3. `apply/recurring.rs:502–509`, recurring-suggestion suppression
   `ON CONFLICT(merchant_key,currency) DO UPDATE`: keyless upsert; returned
   token is derived and not stored as an entity ID.
4. `apply/ingestion_cmds.rs:281`, imported `transaction_categorizations`
   `INSERT OR IGNORE`: replay keeps the existing assignment.
5. `apply/categorization_cmds.rs:222`, manual categorization `INSERT OR
   REPLACE`: replay applies the same command payload.
6. `apply/transactions.rs:542`, `transaction_reviews` `INSERT OR REPLACE`:
   replay updates the same transaction key.
7. `apply/transactions.rs:605`, `transaction_tags` `INSERT OR IGNORE`:
   join row only; no ID mint.
8. `apply/transactions.rs:635–639`, `transaction_details` `ON CONFLICT`:
   replay updates the same transaction key.
9. `apply/transactions.rs:742`, `split_line_tags` `INSERT OR IGNORE`:
   join row only; split-line IDs themselves use the derived helper.

The three open-time seeders are outside a command and run only while creating
the origin vault: `ensure_system_accounts` (`crates/db-worker/src/lib.rs:4226`),
`ensure_default_categories` (`:4459`), and
`merchant_identity::ensure_seed_merchants`
(`crates/db-worker/src/merchant_identity.rs:112`; its IDs are already
name-derived UUIDv5).  **A replica never seeds — the genesis carries seeded
rows.**  Simulator case (ii), the seed-twin bootstrap, must prove that a
replica opening from genesis does not create duplicate “Groceries” or system
rows.

#### 7. Compatibility and replay invariant

Pre-v2 entity IDs remain byte-for-byte untouched.  Existing operation-log rows
without a versioned payload are audit history, not a sync stream; no migration
rewrites their IDs or pretends they are replayable.  A dated note on
`personal-cfo-89oi` re-scopes its property to a hash of canonically ordered
class-1 rows (manifest table order, primary-key order, schema-column order,
fixed value encoding), not byte-identical databases: HashMap iteration and
SQLite rowids are not cross-device invariants.  With the derived IDs above,
the same v2 payload on two vaults sharing one genesis has identical entity IDs
and a hash-equal canonical state.

#### 8. Downstream obligations

SYNC-1 (`personal-cfo-t2s4b`) owns the payload-schema version, `issued_at`,
the `core-ids` derived-ID helper, and the ordinal-cap error.  SYNC-1a
(`personal-cfo-32fmp`) owns the ~137-site determinism rewrite, the
create-if-missing inventory, and the 21-site `issued_at` replacement.  The
two-vault simulator in SYNC-1b (`personal-cfo-tp2ln`) is the exit test for
same-genesis replay and seed-twin case (ii).  The canonical reference for both
downstream beads is this section, **“Addendum A … deterministic derived
identifiers for apply-minted rows.”**

### Rejected alternatives

- **Apply receipts containing mint counts and IDs.** Rejected: a refactor that
  changes mint order or count would invalidate stored envelopes, and a receipt
  is only reproducible when the base is identical.
- **Random IDs plus a per-device mapping table.** Rejected: the mapping table
  becomes a second state bus that must itself be synchronized and repaired.
- **Central server allocation.** Rejected: offline writes are a core Sync
  requirement and the free path has no allocation server.
- **ULID.** Rejected for the reasons already recorded in ADR 0013; UUIDv7 is
  the RFC-standard storage and wire type.
- **Sequential per-device counters in the UUID.** Rejected: they leak device
  identity and make `ORDER BY id` incomparable across devices.

### Revisit if

- A command legitimately needs more than 4,096 derived IDs.
- The HMAC key derivation or its canonical encoding must change (which requires
  a new payload-schema version and migration plan).
- ADR 0017 changes what identifies a device or how vault identity is carried.
- ADR 0074 changes the rollback-then-replay primitive.
