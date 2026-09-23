# Vault table classes (CLASS-0)

`crates/db-worker/table_classes.toml` is the machine-readable CLASS-0
manifest. It is deliberately declarative: it records the current write shape
and the planned Sync contract, but it does not add a write lint, reclassify a
table, or run a migration. A fresh-vault CI test compares the rows marked
`kind = "sqlite_table"` with the migrated `sqlite_master` table set.

## The five classes

| Class | Meaning | Sync treatment |
| --- | --- | --- |
| **1 — replicated** | Canonical user intent: ledger records, user decisions, and their links. | Travels in the logical genesis snapshot and the Sync tail through the command bus. |
| **1b — artifact** | User-supplied bytes and import evidence that a class-1 row cites. | Travels as an opaque, epoch-scoped Sync artifact; it is fetched lazily and tombstoned only after the closure/retention rule permits collection. |
| **2 — derived** | Rebuildable projections, forecasts, checksums, and read models. | Never shipped; rebuilt after the class-1 batch. |
| **3 — device-local** | Credentials, staging, migration metadata, local cursors, KDF/envelope state, and local recovery evidence. | Never included in a snapshot or tail. During a scratch rebase, current local state is preserved and validated rather than replaced by the base. |
| **4 — unclassified user intent** | A current direct writer has not yet been routed through the command bus or explicitly accepted as device-local. | Recorded with its candidate disposition and UX cost. ADR 0074 and SYNC-2 decide the route; this bead does not enforce or reclassify it. |

The manifest includes every table in the current migrated schema. The
`attachment_blobs` row is a pseudo-row for the encrypted directory described by
ADR 0023. `durable_jobs` is a class-3 table owned by `personal-cfo-ati`; its
schedule, retry state, and unlock-window claims remain device-local. The
`backup_history` table is also class 3 and records manual/scheduled backup
receipts inside the vault. Its paths are local-only. Scheduled backup runs keep
all files and never use history to delete from a destination folder; any future
automatic retention requires the separate decision tracked by
`personal-cfo-g3m.2`.

## Class-3 rebase contract

Each class-3 row in the manifest has these fields:

- `rebase_phase` is `stable` for unrelated local state, `replay_owned` for a
  row owned by an active local outbox/queue plan, or `mixed` when one physical
  table contains both runtime branches.
- Stable rows use `rebase_preservation` and `reference_validation`. A mixed
  table instead declares `stable_selector`, `stable_rebase_preservation`, and
  `stable_reference_validation` alongside the corresponding
  `replay_owned_*` fields; its top-level preservation and validation fields
  describe the fail-closed branch dispatch.
- Replay-owned rows (including the replay branch of a mixed table) must declare
  `correlate_to_live_plan`, `auto_replay_remap`, and `queued_disposition`.
  These fields say how the live value is carried into the scratch vault and how
  the class-1/class-1b references and typed validation rule are checked before
  swap.

The phase is selected per row at runtime. Stable rows overlay the current live
value after the remote envelopes are applied. Replay-owned idempotency and
audit evidence are not overlaid as ordinary memos: SYNC-1c correlates them to
the immutable rebase plan, then either auto-replays and remaps them to the new
operation sequence or removes the dispatchable memo and queues the original
evidence for disposition. A declared reference-rule failure enters
`RebaseBlockedLocalState` and leaves the live vault untouched; there is no
fallback swap to the old base. The manifest only declares these rules.

## Class-1b and attachment boundaries

Every class-1b row records a local identity, an opaque
`sync_artifact_id_e` reference, a `closure_role` (`snapshot-full` or
`envelope-delta`), retention, and reference validation. A local `StorageId` or
bare plaintext digest is never a service-facing identity.

Attachment metadata and links are kept separate from the class-3 materialized
blob fields. The manifest uses the migrated schema's actual columns: attachment
identity and user metadata are `id`, `plaintext_size`, `mime_type`,
`original_filename`, and `created_at`; `content_alg` and `ref_count` are
derived; `storage_id`, wrapped/content-key fields, and content nonces are
receiver-local. Link identity is the canonical
`attachment_id`/`entity_kind`/`entity_id`/`created_at` tuple. A fresh-vault
`PRAGMA table_info` test rejects invented, omitted, newly added, or wrongly
classified columns before the manifest can be consumed. A receiver verifies
`SyncArtifactTransportV1`, re-encrypts with a receiver-local content key under
the receiver DEK, and fails closed with `ArtifactMaterializationFailed` when
the transport or DEK validation fails.

## Settings key convention

The existing unprefixed `locale` and `reporting_currency` keys are legacy and
remain unchanged. The current persisted inventory also contains the four
unprefixed keys `minimum_cash_floor_minor`, `comfort_band_upper_minor`,
`auto_categorize_on_import`, and `future_cash_series_selection`; these are
explicitly recorded as legacy user preferences rather than silently labeled
device-local. New settings keys must begin with `device.` or `user.`. The
manifest test enumerates the source-defined key inventory and a fresh vault
instead of validating only an empty allowlist. `personal-cfo-5ymg8` owns the
backwards-compatible prefix migration/dual-read rollout; this bead does not
rename or route any key. The intended migration scope is `user.*` for the six
existing user preferences, subject to that bead's mixed-version and conflict
tests. `device.*` remains class 3, while `user.*` is the class-4 candidate whose
routing is owned by ADR 0074 / SYNC-2.

## Class-4 candidate dispositions

The current direct-writer rows are intentionally visible rather than silently
treated as replicated. `balance_observations`, `scenarios`,
`forecast_assumption_events`, `forecast_dependency_edges`, `dedupe_decisions`,
`merchant_aliases`, `merchant_identities`, `manual_entry_links`,
`transaction_categorizations`, `attachments`, and `attachment_links` each carry
the candidate disposition in the manifest note and explicitly point at ADR
0074 / SYNC-2. `transaction_categorizations` is called out separately because
the apply path is already class-1-shaped while `merchant_memory.rs` still has
a class-4 writer.

## Consumers

The manifest is the shared input for:

- **MOB-0a / D24**, which builds the phone container from the class-1 + class-2
  allowlist;
- **SYNC-1c**, which captures class-3 state and performs the rollback-then-replay
  preservation and validation contract;
- **SYNC-2**, which adds module-level write linting and routes the current
  class-4 writers;
- **SYNC-3**, which emits the logical class-1 genesis snapshot;
- **`personal-cfo-klr.4`**, whose vault-format specification cites this
  inventory; and
- **ADR 0074**, which defines the Sync architecture and the accepted
  class-4 candidate dispositions.

The applicable test and logging obligations remain the project-wide Definition
of Done in `docs/architecture/definition-of-done.md`.
