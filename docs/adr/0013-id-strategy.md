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

The HMAC message bytes are normative and have no implicit text encoding:

```text
message = command_id.bytes                 # 16 bytes, RFC UUID/network order
       || u8_be(tag_len)                   # exactly one byte
       || tag.bytes                        # tag_len bytes
       || u16_be(ordinal)                  # exactly two bytes
```

`tag` is a non-empty printable US-ASCII byte string (`0x21..=0x7e`), bounded
to 1..=255 bytes.  The one-octet length prefix is therefore sufficient; an
empty tag, a non-ASCII byte, or a length greater than 255 is a validation
error, and no Unicode normalization or NUL terminator is applied.  `ordinal`
is validated in 0..=4095 before it is encoded.  These rules make the HMAC
input exactly `16 + 1 + tag_len + 2` bytes on every device.

Let `digest = HMAC-SHA256(id_key, message)`.  `rand_b` is the *high* 62 bits
of the digest, interpreted as an unsigned big-endian integer:

```text
rand_b = u64_be(digest[0..8]) >> 2
```

The shift is normative right-alignment: the high 62 digest bits become a
62-bit integer with two leading zero bits before its eight-byte encoding; no
implementation may copy the raw first eight HMAC bytes into the builder.

The ten-byte buffer passed to uuid 1.23.2 is packed exactly as follows (the
top two bits of `rand_b` are consequently zero):

```text
counter_random_bytes[0] = (ordinal >> 8) & 0x0f
counter_random_bytes[1] = ordinal & 0xff
counter_random_bytes[2..10] = rand_b.to_be_bytes()
```

`uuid::Builder::from_unix_timestamp_millis(unix_ms,
&counter_random_bytes).into_uuid()` then sets the RFC 9562 version and variant
bits.  The builder masks the top two bits of byte 2 and writes the `10` variant
bits itself; it does not consume the raw first eight HMAC bytes.  Thus the
62 HMAC bits are preserved as UUID byte 8's low six bits followed by bytes
9–15, while `rand_a` occupies UUID bytes 6–7.  The UUID is consequently a
function of the command, not of a device, process, wall-clock read, or
replica base.

The 32-byte, non-secret `id_key` is fixed as:

```text
id_key = SHA-256(ASCII("dohflow/derived-id-key/v1") || vault_id.bytes)
```

The ASCII label has no trailing NUL or length prefix; `vault_id.bytes` is the
canonical 16-byte UUID/network-order representation.  Genesis carries this
value beside `vault_id`; a receiver recomputes it and rejects a mismatch
before applying the snapshot.  It is an identifier-domain constant, not a
credential or encryption key, so exposing it does not weaken vault secrecy.
Changing the label or encoding is a payload-schema change and is a revisit
trigger below.  The UUID is consequently a function of the command, not of a
device, process, wall-clock read, or replica base.

These two known-answer vectors are normative (all hexadecimal is lowercase),
including the message, full HMAC, packed builder input, and final UUID.  The
future SYNC-1 `core-ids` test must consume these same vectors and add a
different-tag/different-ordinal assertion rather than retyping an alternate
grammar.

```text
vector A
vault_id              = 00112233-4455-6677-8899-aabbccddeeff
command_id            = ffeeddcc-bbaa-9988-7766-554433221100
issued_at_unix_ms     = 1700000000123
tag                   = ledger_transaction
ordinal               = 0
id_key                = f2f6ac71bc63e3df2f66370676ead3f3b6b677a87c124f190941659224b42149
message_hex           = ffeeddccbbaa99887766554433221100126c65646765725f7472616e73616374696f6e0000
hmac_sha256           = bd3f13b4d1dc70d711049be5e9dc25476758dd74c249b2b2a09d7fe9f2ecab20
counter_random_bytes  = 00002f4fc4ed34771c35
uuid                  = 018bcfe5-687b-7000-af4f-c4ed34771c35

vector B
vault_id              = 11223344-5566-7788-99aa-bbccddeeff00
command_id            = 01234567-89ab-cdef-0123-456789abcdef
issued_at_unix_ms     = 1700000000999
tag                   = split_line
ordinal               = 7
id_key                = b33777244109db333d33cacff0f92a7d03e93497cc2d460e8f2b2db3548891b8
message_hex           = 0123456789abcdef0123456789abcdef0a73706c69745f6c696e650007
hmac_sha256           = fa00859fd235ec6e29d41c7e4ccea2df6260ed2141e15183b4c9ece7fc1182a6
counter_random_bytes  = 00073e802167f48d7b1b
uuid                  = 018bcfe5-6be7-7007-be80-2167f48d7b1b
```

#### 2. Metadata and time boundary

`issued_at` is added to `CommandMeta` at the command-construction boundary and
is persisted in the v2 envelope.  `apply` receives `(payload, meta, base)` and
never reads the wall clock.  The current inventory is the 21 `Utc::now()` sites
under `crates/db-worker/src/apply/` (accounts, categorization commands,
ingestion commands, recurring commands, scenario promotion, and transactions);
SYNC-1a replaces each with `meta.issued_at` and tests that no apply path calls
the clock.

*(Scope corrected 2026-10-19: the rule covers every clock read reachable from
`apply`, including helpers outside `apply/` and class-2 rebuilds, not only these 21
sites. See section 10(d).)*

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

*(Corrected 2026-10-19: `ensure_system_ledger_account`, and any other
create-if-missing singleton, takes a content-derived id outside the ordinal
sequence instead of an ordinal. See section 10(b).)*

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

*(Corrected 2026-10-19: the payload and the metadata alone determine the
sequence, never the base, and the order of mints must itself be deterministic.
See section 10(a) and 10(c).)*

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
   *(Superseded 2026-10-19: a singleton's id is content-derived and outside the
   ordinal sequence, so replay does not depend on whether it exists. See section
   10(b).)*
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

#### 9. Exception: content-derived provider-transaction identifiers (2026-10-18, `personal-cfo-g3m.12`)

The mint-site rule of section 3 covers ids minted *by apply*. One more class of id
is deliberately **not command-scoped**: the identity of a transaction a connector
imported from a bank-data provider. Two devices that each fetch the same provider
transaction with their own credentials must derive the **same** id, whichever
command carries it and whenever each device fetched it, or the ledger commits it
twice (ADR 0074 Addendum B). So this id is derived from the provider's own
identifiers, **never from `issued_at`, a `command_id` or an ordinal**, it is
computed when the commit command is *constructed*, and it is carried in the
`CommitStaged` payload like the three caller-minted ids of section 3. It consumes
no ordinal and is outside the 4,096-id cap.

```text
label    = ASCII "dohflow/provider-txn-id/v1"                (26 bytes)
message  = u8(26) || label
        || connection_id.bytes                               # 16 bytes, RFC UUID order
        || u16_be(len(provider_account_key)) || provider_account_key
        || u16_be(len(provider_txn_id))      || provider_txn_id
digest   = HMAC-SHA256(id_key, message)                      # the same per-vault id_key as section 1
id_bytes = digest[0..16], then
           byte 6 = (byte 6 & 0x0f) | 0x80                    # UUID version 8
           byte 8 = (byte 8 & 0x3f) | 0x80                    # RFC 9562 variant
```

The id is a UUID version 8 (RFC 9562 custom): it carries no time, so nothing may
rely on `ORDER BY id` for creation order of provider-derived rows.
`provider_account_key` and `provider_txn_id` are the provider's own strings,
used **byte for byte as the adapter received them** (no case folding, no Unicode
normalization), each 1 to 1,024 bytes of valid UTF-8 and **containing no C0 control
character or DEL (`0x00..=0x1f`, `0x7f`)**. A transaction whose provider
identifiers do not satisfy this has no provider identity: it is not given a
derived id, and no fallback fingerprint stands in for one (Addendum B, item 6).
The `connection_id` is the replicated connection identity (ADR 0074 Addendum B,
item 3).

*Domain separation.* The two derivations share `id_key`, so their message spaces
must not overlap, and they do not, for every input including a hostile
`command_id`. Suppose a provider message were also a valid section 1 message. Section
1 reads the byte at offset 16 as `tag_len` and the next `tag_len` bytes as a tag whose
every byte is printable (`0x21..=0x7e`). In a provider message the byte at offset 16
is `label[15]`, `r` (`0x72`), so `tag_len` would be 114 and the tag would span offsets
17 to 130. Offset 43 lies in that span, and in a provider message it is the high byte
of `u16_be(len(provider_account_key))`, which is `0x00..=0x04` because the length is
at most 1,024: never printable. So no provider message is a valid section 1 message,
for any `connection_id` and any provider strings. The outputs are disjoint as well:
provider-derived ids are UUID version 8 and section 1 ids are version 7, so the two id
spaces differ in the version nibble even before the messages are compared.

*Adapter stability.* Because identity is byte-exact, **each adapter's
`provider_account_key` function is part of the identity.** Each adapter defines its key
canonically, a test pins it, and changing that function is a payload-schema change.
If the provider starts or stops returning a component of the key for an account that
is already linked (for example a connection id that appears or disappears), that is an
identity change, handled as a disposition that queues for the user; it is never a
silent re-key, which would re-import the account's whole history under new ids.

*Residual.* A provider that reuses one transaction id for two different transactions
collapses them into one derived id on purpose: the provider is vouching that they
are the same. Addendum B's rule then decides (equal provider-financial fields: a
no-op; differing fields: one disposition item).

Known-answer vector (normative; hexadecimal is lowercase; `id_key` is vector A's
key from section 1):

```text
id_key               = f2f6ac71bc63e3df2f66370676ead3f3b6b677a87c124f190941659224b42149
connection_id        = 0198c2c1-5f0a-7c3d-8a4e-9b1d2e3f4a5b
provider_account_key = ACT-7f3a91
provider_txn_id      = TXN-2026-10-17-0042
message_hex          = 1a646f68666c6f772f70726f76696465722d74786e2d69642f76310198c2c15f0a7c3d8a4e9b1d2e3f4a5b000a4143542d376633613931001354584e2d323032362d31302d31372d30303432
hmac_sha256          = 946109b1f8035430f5e8d148d476d8296f95b704f265c666a5d89fa7c9b21b81
uuid                 = 946109b1-f803-8430-b5e8-d148d476d829

provider_txn_id      = TXN-2026-10-17-0043      (everything else as above)
uuid                 = cf5a7fd8-f6b4-8c2f-a9e1-ef4b6ca8e52e
```

Changing the label, the field encoding or the bit layout is a payload-schema
change, as for section 1. File imports, which have no provider identifiers,
derive their identity the same way from a file fingerprint and a row ordinal under
their own label, fixed by the import-reshape bead (`personal-cfo-0g528`) with its
own vector.

#### 10. Correction note (2026-10-19, `personal-cfo-g3m.14`): payload-only ordinals, singleton ids, mint order and the clock scope

Review of the first implementation steps (`personal-cfo-t2s4b`) found four places
where sections 2, 3, 4 and 6, as accepted, do not give the cross-device determinism
this addendum exists for. This note corrects them. Where an earlier sentence
conflicts with this note, this note governs, and those sentences carry a dated
pointer here; their accepted text is otherwise unchanged.

**(a) Ordinal consumption depends only on the payload and the metadata.** Section 4
said the payload, the metadata *and the base* determine the ordinal sequence. They
must not. If a mint happens only on some base states (create-if-missing: the row
exists on one replica's base and not on another's), then the number of ordinals a
command consumes depends on the base. A rollback-then-replay onto a base where
another device already created the row would shift every later id the command mints
by one, so the rows would get different ids than in the original local apply, and
later local commands that refer to those ids would fail validation. The rule: **the
number of ordinals a command consumes, and which mint takes which ordinal, is a
function of the command's payload and metadata alone.** Any branch that depends on
base state and mints must use a content-derived id outside the ordinal sequence, as
in (b).

**(b) Create-if-missing singletons get a content-derived id.**
`ensure_system_ledger_account`, and any other create-if-missing singleton that the
SYNC-1a inventory finds, takes an id that is a function of the singleton's identity
alone, outside the ordinal sequence and outside the 4,096-id cap. Every replica
that creates the row therefore creates the *same* row, and replaying a command
neither consumes an ordinal nor mints a second id.

```text
label    = ASCII "dohflow/system-account-id/v1"              (28 bytes)
message  = u8(28) || label
        || u8(len(role)) || role                             # a member of the closed role set, [a-z_]{1,64}
        || currency                                          # exactly 3 bytes, ASCII A-Z (ISO 4217)
digest   = HMAC-SHA256(id_key, message)                      # the per-vault id_key of section 1
rand_b   = u64_be(digest[0..8]) >> 2                         # the high 62 bits, as in section 1
packing  = uuid::Builder::from_unix_timestamp_millis(0, &counter_random_bytes)
           counter_random_bytes = 0x00 0x00 || rand_b.to_be_bytes()
```

The layout is section 1's, with a fixed zero time field and ordinal zero, so a
singleton id is a UUID version 7 that sorts before every command-minted id:
`ORDER BY id` lists the system singletons first, on every replica. The zero in its
timestamp is a constant, **not a creation time**: nothing may read a singleton's
creation instant from its id. The role and
currency are validated before use, and anything else is a validation error raised
before any write: the currency is exactly three ASCII uppercase letters, and the role
is a member of the closed set of system role names defined with the ledger schema
(today `opening_balance_equity`, `unmatched_income` and `unmatched_expense`); a new
role enters the set by a code change, never from data. The validation is what makes
the next argument hold. The label, its length byte and the fixed-width tail keep this
message space apart from the other two. From section 9: the first byte (28, `0x1c`)
differs from the provider derivation's (26, `0x1a`), and the ids differ by version
nibble (7 here, 8 there). From section 1: a command-scoped message ends with
`u16_be(ordinal)` for an ordinal below 4,096, so its second-to-last byte is at most
`0x0f`, whereas the message here always ends with the currency's final two uppercase
letters (the second-to-last byte is at least `0x41`), so no singleton message can be
read as a command-scoped one.

Known-answer vectors (normative; hexadecimal is lowercase; `id_key` is vector A's key
from section 1):

```text
id_key               = f2f6ac71bc63e3df2f66370676ead3f3b6b677a87c124f190941659224b42149
role                 = opening_balance_equity
currency             = USD
message_hex          = 1c646f68666c6f772f73797374656d2d6163636f756e742d69642f7631166f70656e696e675f62616c616e63655f657175697479555344
hmac_sha256          = c859852d50d9e3ea52e4a41f13a4be25d457ac6aa8f582194e9de638ecd64440
counter_random_bytes = 00003216614b543678fa
uuid                 = 00000000-0000-7000-b216-614b543678fa

role                 = unmatched_income
currency             = EUR
message_hex          = 1c646f68666c6f772f73797374656d2d6163636f756e742d69642f763110756e6d6174636865645f696e636f6d65455552
hmac_sha256          = 981fabd230c3785ecba2e2bed52af266b26be9dd64c4e596bcf88e4960535dec
counter_random_bytes = 00002607eaf48c30de17
uuid                 = 00000000-0000-7000-a607-eaf48c30de17
```

This is the same family as the provider-transaction exception of section 9
(`personal-cfo-g3m.12`): a domain-separated HMAC under the per-vault `id_key`, a
content-derived id that does not use `issued_at`, a command id or an ordinal. The
open-time seeders of section 3 may adopt the same derivation so that devices that
seed independently also converge; SYNC-1a decides that when it classifies each
site. Changing the label or the encoding is a payload-schema change, as in section 1.

**(c) Mint order is deterministic.** The ordinal is allocated in the order the mints
happen, so the order itself must not depend on anything unspecified. On any path
that mints: no iteration over a `HashMap`, a `HashSet` or any other unordered
collection; every `SELECT` that feeds a mint loop has a total `ORDER BY` (SQLite
does not guarantee row order without one, and the key must be unique so that ties
cannot occur); and no dependence on thread scheduling, addresses or time. Two
replicas that assign ordinals in different orders mint different ids and the
write-set hash flags a fork. SYNC-1a enforces this with a ratchet test.

**(d) The no-clock rule covers everything reachable from `apply`.** Section 2
scoped the rule to the 21 `Utc::now()` calls directly under
`crates/db-worker/src/apply/`. `apply` also calls helpers elsewhere that read the
clock themselves, and the rule covers them, for example: the `ingestion` helpers
(`create_source_batch`, `update_batch_status`, `link_provenance`,
`record_dedupe_decision`), `record_transaction_detail`,
`reverse_and_hide_transaction`, the inbox helpers `record_dismiss` and
`record_snooze`, the household-local "today" read, and the **class-2 rebuilds**
(`rebuild_in` of the commitments and money-inbox projections). The list is
illustrative; the ratchet test derives the actual set from the code. Class-2 rows must also be hash-equal across vaults (the exit
criterion of the Sync foundations milestone), so a clock read inside a class-2
rebuild is in scope. Every one of them reads `meta.issued_at`, passed down by the
caller. `personal-cfo-t2s4b` pins the inventory with a ratchet test that derives
the set of clock-reading helpers from the crate and may only shrink; SYNC-1a
empties it.

#### 11. Exception: merge-import derived identifiers (2026-10-19, `personal-cfo-g3m.15`)

"Bring items from another vault" replays items from a saved second vault into the
household vault as ordinary commands (ADR 0074 Addendum C). Every row such a merge
**creates** gets a content-derived id, so running the same merge again, or running it on
two devices, mints the same ids and converges instead of creating the rows twice. Like
section 9 it is **not command-scoped**: it is derived from the source row's identity,
never from `issued_at`, a `command_id` or an ordinal; it is computed when the command is
*constructed* and carried in the payload; it consumes no ordinal and is outside the
4,096-id cap.

```text
label    = ASCII "dohflow/merge-import-derived-id/v1"        (34 bytes)
message  = u8(34) || label
        || source_vault_id.bytes                             # 16 bytes, RFC UUID order
        || u8(len(entity_tag))  || entity_tag                # ASCII [a-z0-9_], 1..32 bytes
        || u16_be(len(source_row_key)) || source_row_key     # 1..256 bytes
digest   = c920d88d9c429df3285d8c929e64ae71f82c24c19132e7a423f0188169cd0d6c-SHA256(id_key, message)                      # the HOUSEHOLD vault's id_key (section 1)
id_bytes = digest[0..16], then
           byte 6 = (byte 6 & 0x0f) | 0x80                    # UUID version 8
           byte 8 = (byte 8 & 0x3f) | 0x80                    # RFC 9562 variant
```

`id_key` is the **household** vault's key (`SHA-256("dohflow/derived-id-key/v1" ||
vault_id)` of the vault being merged *into*), so two households that merge the same
source mint unrelated ids. `source_vault_id` is the saved vault's own `vault_id`.
`entity_tag` names the kind of row from a closed registry fixed by the merge-import
feature bead (`personal-cfo-i93n3.7`); a new tag needs an amendment to this section.
`source_row_key` is the source row's primary key in the canonical row encoding
(`vault-format.md` C.1.5: for a `uuid` id, the tag byte `03` and the 16 raw bytes), so
text-keyed and composite-keyed rows are covered by the same rule and the key is
byte-exact. The id is a UUID version 8: it carries no time, so nothing may rely on
`ORDER BY id` for creation order of merge-created rows.

*Domain separation.* It shares `id_key` with sections 1, 9 and 10, so the message spaces
must not overlap. Against section 9 and section 10 the first byte differs (34, `0x22`,
against 26 and 28), and each message is a fixed layout from that byte on, so no message of
one is a message of another. Against section 1: a section 1 message reads the byte at
offset 16 as `tag_len` and the next `tag_len` bytes as a printable tag
(`0x21..=0x7e`). In a merge message the byte at offset 16 is `label[15]`, `m` (`0x6d`), so
`tag_len` would be 109 and the tag would span offsets 17 to 125. The high byte of
`u16_be(len(source_row_key))` lies at offset `52 + len(entity_tag)`, which is between 53
and 84 for every legal tag, inside that span, and it is `0x00` or `0x01` because the key is
at most 256 bytes: never printable. So no merge message is a valid section 1 message. The
outputs are UUID version 8, like section 9's, so the two spaces are kept apart by the
message domain (the labels differ in length and content), not by the version nibble.

Known-answer vector (normative; hexadecimal is lowercase; `id_key` is vector A's key from
section 1):

```text
id_key           = f2f6ac71bc63e3df2f66370676ead3f3b6b677a87c124f190941659224b42149
source_vault_id  = 0198c2c1-5f0a-7c3d-8a4e-9b1d00000063
entity_tag       = account
source_row_key   = 030198c2c15f0a7c3d8a4e9b1d00000071      (a uuid key, C.1.5)
message_hex      = 22646f68666c6f772f6d657267652d696d706f72742d646572697665642d69642f76310198c2c15f0a7c3d8a4e9b1d00000063076163636f756e740011030198c2c15f0a7c3d8a4e9b1d00000071
hmac_sha256      = c920d88d9c429df3285d8c929e64ae71f82c24c19132e7a423f0188169cd0d6c
uuid             = c920d88d-9c42-8df3-a85d-8c929e64ae71

entity_tag       = category      (everything else as above)
uuid             = 8af23603-a0c7-88e6-8234-2e0864d732a8
```

Changing the label, the field encoding or the bit layout is a payload-schema change, as
for sections 1, 9 and 10.

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
