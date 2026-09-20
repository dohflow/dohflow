# ADR 0074 — DohFlow Sync architecture

- **Status:** Accepted (owner decision, 2026-09-20)
- **Tier:** Public
- **Date:** 2026-09-20
- **Bead:** `personal-cfo-j8ab7`
- **Decider:** Owner, 2026-09-20
- **Related:** ADR 0002 (local encrypted vault), ADR 0011 (hybrid
  persistence), ADR 0013-A (derived identifiers, planned), ADR 0066 (free
  local app and separate services), ADR 0073 (sync disposition rules,
  planned), ADR 0077 (wire formats, planned), and ADR 0082 (public disclosure
  boundary)

## Context

DohFlow's current single-vault persistence model deliberately does not yet
implement replication. The facts that bound a safe Sync design are visible in
the shipped source:

- The operation log schema has a nullable `metadata` column
  (`crates/db-worker/src/lib.rs:3831-3845`), but the writer records
  `Option::<Vec<u8>>::None` while using `cmd.kind()` as the operation type
  (`crates/db-worker/src/lib.rs:6463-6481`). Historical rows therefore have
  audit metadata, not a command payload capable of reconstructing canonical
  state.
- Identity is still minted while applying commands: the source audit that
  informed this decision found approximately 137 minting sites across 22 files,
  including `src/apply/`; representative apply-time allocations are in
  `crates/db-worker/src/apply/accounts.rs:74`,
  `crates/db-worker/src/apply/ingestion_cmds.rs:237`, and
  `crates/db-worker/src/apply/transactions.rs:715`. Replaying a payload that
  depends on those locally minted values would not be deterministic.
- `DbWorker::snapshot_bytes()` reads the checkpointed raw `vault.db`
  (`crates/db-worker/src/lib.rs:1236-1252`). That database contains
  `connector_connections.credential`, intentionally stored for local
  backup/restore (`crates/db-worker/src/migrations.rs:1480-1505`). A raw
  database image therefore cannot be a Sync genesis snapshot.
- `CommandMeta` already carries `correlation_id` and `causation_id`
  (`crates/db-worker/src/lib.rs:174-188`), and the current
  `WriteCommand` enum has 49 variants
  (`crates/db-worker/src/lib.rs:223-698`). Its payload and time semantics
  must become explicit before a command tail can replicate safely.
- The nine user-intent groups that are not yet uniformly on the command bus
  are the subject of Decision 5. The direct settings path explicitly bypasses
  `WriteCommand` (`crates/db-worker/src/lib.rs:2593-2608`), and
  `transaction_categorizations` has both command-bus writers
  (`crates/db-worker/src/apply/categorization_cmds.rs:222-235`) and the
  direct merchant-memory writer
  (`crates/db-worker/src/merchant_memory.rs:173-181`).
- A stable `vault_id` already exists in `VaultMetadata`
  (`crates/db-worker/src/lib.rs:1013-1020`) and is seeded once with vault
  metadata (`crates/db-worker/src/lib.rs:4167-4185`). The current
  `hlc_timestamp` is explicitly a single-device counter
  (`crates/db-worker/src/lib.rs:3824-3828`), so it is not a distributed
  ordering authority.
- `CommitStaged` currently carries a device-local
  `StagedTransactionId` (`crates/finance-kernel/src/lib.rs:1659-1708`).
  The three hard-delete paths are `recurring_transfers`
  (`crates/db-worker/src/apply/recurring.rs:93-108`), `income_sources`
  (`crates/db-worker/src/apply/recurring.rs:264-278`), and recurring bills
  including their contracts
  (`crates/db-worker/src/apply/recurring.rs:633-654`).
- Device-local staging can refer to replicated artifacts: for example,
  `staged_transactions.source_record_id` references `source_records`
  (`crates/db-worker/src/migrations.rs:636-654`). A rollback base therefore
  cannot silently substitute its older class-3 state for the currently live
  state after class-1 replay.

These constraints mean Sync starts from a logical, encrypted representation of
canonical user intent and adds a versioned command tail. It is not retroactive
replay of existing operation-log history, raw-database replication, or a
ledger-row CRDT.

## Decision

### 1. Shape: logical genesis snapshot plus a command-envelope tail

At Sync enablement, a client publishes an encrypted logical genesis snapshot
of class-1 tables. It is the Export Everything bundle specified by
`personal-cfo-klr.4` with one additional consumer: class-1b artifacts are
referenced by content hash, and no class-3 row is present. Afterwards clients
publish versioned command envelopes. A replica bootstraps from the latest
snapshot plus its tail.

Re-snapshotting is the only way the service truncates history, and it may
truncate only behind a client-published snapshot. A snapshot publication
carries `base_seq`; the service accepts it only when `base_seq` equals the
current head. This preserves a complete recovery path for every accepted
envelope without making the raw vault database a wire format.

`personal-cfo-1cltt` (SYNC-3) implements the logical snapshot and bootstrap;
`personal-cfo-w21q7` (CLASS-0) supplies the table classification; and ADR
0077, implemented by `personal-cfo-klr.4`, owns the durable wire-format
specification.

### 2. No lease: compare-and-swap, then rollback-then-replay

The service head sequence is the sole write serialization point. A push carries
`base_seq`; the service accepts it if and only if that value equals the
current head. Otherwise it returns the envelopes since `base_seq`. There is
no single-writer lease, expiry policy, or takeover window.

A client with unpushed work rebases by **rollback-then-replay** on a base image,
not by applying pulled envelopes over unpushed local state:

1. Enter `Rebasing` under the single-instance lock, freeze local writers, and
   capture a consistent **class-3 preservation image** from the current live
   vault. Keep `vault.db` as of `base_seq` beside the live vault and advance it
   on every accepted push.
2. Restore that base image only as a scratch vault and apply the service
   envelopes. The base's class-3 rows are never authoritative after this step.
3. Remove the stale class-3 rows inherited from the base and transactionally
   overlay the captured current class-3 state in the scratch vault using the
   per-table preservation and reference-validation rules in CLASS-0. Current
   device-local state wins over the base; it is never silently merged with, or
   replaced by, stale base state.
4. Reapply each local outbox envelope under its original `command_id` only
   when its write set is untouched and it still validates. Otherwise queue its
   whole correlation group for disposition.
5. Rebuild class 2 and validate every class-3 reference to changed class-1 or
   class-1b state before the swap. An invalid reference enters a typed
   `RebaseBlockedLocalState` recovery state, preserves both the live vault and
   scratch evidence, and requires an explicit repair or re-bootstrap. It never
   drops the row or falls back to the old base.
6. Atomically swap the fully validated rebuilt vault and retry the push.

Nothing is discarded, including device-local work, and no envelope is applied
twice. A failed preservation or validation step leaves the live vault
byte-identical. Garbage collection and rebase are therefore one integrity
problem. The replica's operation-log row records `applied_base_seq` and
`rebased_from` for R46. Pulled envelopes are never applied over unpushed local
state.

`personal-cfo-bxsbz` (SYNC-1c) implements the base image and atomic rebase;
`personal-cfo-vlfd` (ADR 0073) defines the disposition rules; and
`personal-cfo-mpep3` (S3-2) presents queued groups.

### 3. Envelopes: versioned payloads and verified class-1 writes

Every `WriteCommand` gains a versioned payload schema. Its envelope contains
the payload-schema version; the command metadata including `issued_at`,
`correlation_id`, `causation_id`, and `device_id`; the engine and schema
versions; `seq` and `base_seq`; and a write-set hash over canonically
ordered class-1 rows touched by the command. Class-2 rebuilds are excluded from
that write-set hash.

`issued_at` replaces every `Utc::now()` reached inside `apply/`, so apply
is a function of payload, metadata, and base state. An engine- or schema-version
mismatch tells the client to update before continuing (R40). A write-set hash
mismatch on a replica is **a fork, not a conflict**: it is counted and shown to
the user, preserves the outbox, and recovers through re-bootstrap rather than
attempting a silent merge.

`personal-cfo-t2s4b` (SYNC-1) implements the operation-log-v2 payloads,
metadata, version checks, and hash; `personal-cfo-32fmp` (SYNC-1a) makes
apply deterministic; and `personal-cfo-elciw` (S3-1) owns the client-facing
update and fork-recovery behavior.

### 4. Derived identifiers

ADR 0013-A (`personal-cfo-1df8d`) defines derived IDs before SYNC-1:
replicas never seed IDs locally. A genesis snapshot carries rows already seeded
by the origin. This makes the simulator's seed-twin case deterministic without
turning allocation control flow into a wire format.

### 5. Five write classes and the nine intent groups

The Sync model has five classes:

- **1 — replicated:** canonical user intent travels in the snapshot and tail.
- **1b — artifact:** user-supplied bytes travel by content hash, are fetched
  lazily, and are tombstoned rather than rewritten.
- **2 — derived:** rebuilt locally after each batch; never shipped.
- **3 — device-local:** never in a snapshot or tail.
- **4 — unclassified user intent:** must be routed to class 1 or explicitly
  declared class 3 with its UX cost before Sync implementation.

The following nine class-4 groups are decided now. Each row includes the
accepted user-facing consequence rather than silently treating a direct write
as local.

| Intent group | Class | Decision and accepted UX consequence |
|---|---|---|
| `balance_observations` | 1 | Assertions replicate. They are never last-write-wins: two assertions for one account and date queue for an explicit choice under ADR 0073. |
| `scenarios`, `forecast_assumption_events`, and `forecast_dependency_edges` | 1 | Scenario work follows the user to another device; conflicting changes queue rather than silently losing an assumption. |
| `dedupe_decisions` | 1 | An import decision follows the related history, so another device does not rediscover or reverse it. |
| `merchant_aliases` | 1 | A user correction to a merchant name follows the user instead of producing different local interpretations. |
| `merchant_identities` | 1 | Identity knowledge follows the user; conflicts remain visible through the disposition queue. |
| `manual_entry_links` | 1 | A manually linked assumption remains explainable on every device rather than looking like a missing relationship. |
| `attachments` and `attachment_links` | 1 for links; 1b for blobs | Links replicate with canonical data; blobs replicate by content hash and may be fetched lazily. |
| `transaction_categorizations` | 1 | `merchant_memory.rs` is routed through the command bus by SYNC-2, eliminating the present split writer and preserving the user's category decision. |
| `settings` keys under `user.*` | 1 | Household-level settings follow the user through the command bus. This explicitly reverses the direct `set_setting` convention at `lib.rs:2595`; `device.*` settings remain class 3. |

Class 1 covers user-facing canonical intent; class 1b covers artifact bytes;
class 2 is rebuilt; class 3 includes connector credentials, refresh
watermarks, staged rows, idempotency keys, audit events, node-local values,
KDF parameters, `device.*` settings, durable jobs, backup history, and local
Sync key-epoch, nonce-range cursor, and service-credential state.
CLASS-0 records the exact per-table manifest. `personal-cfo-5ymg8` (SYNC-2)
implements the routing and module-level lint; `personal-cfo-07u` (SYNC-2a)
splits connector identity from device-local secrets.

### 6. Payload-reference rule

An envelope payload may reference only a class-1 row or a class-1b artifact by
content hash. `CommitStaged` is reshaped to carry its materialized
transaction and cite the source record's hash instead of a device-local staging
ID. Source-batch, source-record, parser-run, and state kinds become class-1b
artifact envelopes. `SkipStaged` stays device-local and never ships.

`personal-cfo-0g528` (SYNC-2c) implements that materialization and a test
that checks every payload type against the CLASS-0 manifest.

### 7. Tombstones before the S1 freeze

The three hard deletes become tombstones before the S1 freeze. Their
`deleted_at` values use `issued_at`; readers, forecasts, and projections
filter tombstoned rows; reinstatement is the corresponding Toggle operation.
Nothing is purged. This makes deletion compatible with rebase and the ADR 0073
disposition model.

`personal-cfo-egn67` (SYNC-2b) implements this rule and covers all three
existing delete paths.

### 8. Client-side encryption, epochs, and key custody

Snapshots and envelopes use AES-256-GCM under a fresh, uniformly random
32-byte **Sync key** that is distinct from the vault DEK. Every encrypted object
belongs to a monotonically increasing `key_epoch`; the key is sealed only to
the enrolled devices in that epoch. The same enrollment and `device_id`
mechanism is reused by the phone-container work in ADR 0024-A. The service
stores sealed-key blobs and protocol metadata, never a decryption key.

Device revocation is a fresh-key, atomic epoch cutover — not a re-seal of a
key the removed device already knows:

1. An authorized remaining device generates a fresh Sync key for epoch
   `e + 1`, seals it to every remaining enrolled device, and encrypts a new
   logical snapshot at the current service head.
2. The service atomically verifies the device-management authorization and
   head, installs the `e + 1` sealed-key set and snapshot, records the active
   epoch, and revokes the removed device's enrollment and service credential.
3. The service rejects every old-epoch write or nonce allocation after the
   cutover. A remaining device can obtain its new sealed key and bootstrap from
   the new snapshot; a removed, offline device cannot decrypt or submit any
   post-cutover object.

Ciphertext already obtained before the cutover cannot be made secret
retroactively. That limitation is shown in the revocation UX; it does not
justify retaining the old key for new data. `personal-cfo-v98vd` owns the
service authorization, credential revocation, epoch transition, and atomic
cutover contract; `personal-cfo-elciw` owns the client Settings flow.

The GCM nonce is a deterministic 96-bit value
`nonce_domain(32) ‖ invocation_counter(64)`. The service assigns a unique
`nonce_domain` for each `(vault_id, key_epoch, device_id, object_type)` context
and allocates durable counter ranges for that context before encryption. A
client persists its next counter ahead of use in device-only local Sync state;
a crash or changed retry burns the value, while an exact-byte retransmission
reuses the existing ciphertext. Neither a domain nor a counter range is reused
within an epoch, including after a crash, retry, or re-enrollment. Snapshots
have their own `object_type` domain. The service charges every allocated value
against a per-epoch budget and refuses further allocation at
`2^32` invocations, forcing a fresh-key epoch before any additional object is
encrypted. It accepts a `(key_epoch, nonce)` only once for a new object; an
exact-byte retransmission receives the prior idempotent result, while any
non-identical reuse fails closed. This is the fixed-field/invocation-field
construction in
[NIST SP 800-38D §8.2.1](https://doi.org/10.6028/NIST.SP.800-38D), with a
deliberate operational cap rather than random IVs.

The authenticated data is the canonical encoding of every pre-encryption clear
header field: `protocol_version ‖ vault_id ‖ key_epoch ‖ object_type ‖
object_id ‖ device_id ‖ base_seq ‖ payload_schema_version ‖ engine_version ‖
schema_version`. `object_id` is the immutable `command_id` for an envelope or
the fresh snapshot ID for a snapshot. The service-assigned `seq` is deliberately
not in AAD: it does not exist until after the CAS accepts the already sealed
object. The wire format binds the returned sequence to that immutable object ID
in the append-only service log. Any AAD, epoch, object-type, domain, counter,
or tag mismatch fails closed.

**D1:** ADR 0066 leaves this encryption mechanism open; this ADR supplies the
client-side ciphertext-only mechanism while leaving ADR 0066's free-local-app
and separate-service boundary otherwise unchanged.

**D12, owner decision 2026-09-20:** there is no recovery escrow in Sync v1.
This is a key-custody constraint for the encrypted protocol and is also stated
in Decision 10; it does not change the local password or backup-recovery
model.

`personal-cfo-1cltt` (SYNC-3) implements the sealed Sync key and snapshot
encryption; `personal-cfo-klr.4` (ADR 0077) owns the canonical header, nonce,
and compatibility format; `personal-cfo-v98vd` owns nonce allocation and
device-service authorization; and `personal-cfo-bc8iv` (ADR 0066-A) owns the
endpoint and egress posture.

### 9. Identity, restore, and ordering

ADR 0017, implemented by `personal-cfo-6kn`, defines identity and clock
details: `device_id` is minted at enrollment and stored outside the vault;
`node_id` is per vault and copied by snapshot or restore; `vault_id` is the
service storage key. The existing HLC remains advisory. The service sequence is
the total order.

For R39, a restore mints a new `device_id`. Sync enablement never publishes a
new genesis for a `vault_id` the service already knows: it bootstraps a
replica, retains local edits in the outbox, and sends them through the ordinary
rebase path.

### 10. Escrow

The owner decided on 2026-09-20 that Sync v1 provides **no recovery escrow**
(D12). `personal-cfo-efv` may open a later recovery design only with the
planned external review. This is an intentional v1 limitation, not a hidden
server-held key or a new password-reset route.

### 11. The two-vault simulator is the acceptance instrument

An in-process two-vault harness uses two `DbWorker` instances, an in-memory
CAS sequencer, injected partitions, and injected clock skew. It is the S1/S3
exit instrument, not an optional test utility. It covers:

1. A device with unpushed envelopes pulls first.
2. Two vaults each seed at open, then one bootstraps as a replica.
3. An unresolved queue is visible from the other enrolled device; its age is
   shown on every device and it nags after 24 hours (R45).
4. A rebased replica operation-log row contains `applied_base_seq` and
   `rebased_from` (R46).

`personal-cfo-tp2ln` (SYNC-1b) implements the harness and convergence
property tests; `personal-cfo-mpep3` exposes the queue and audit evidence.

The simulator also has three mandatory safety suites:

- **Revocation cutover:** enroll A and B, take B offline, revoke B while it
  races an old-epoch push, and prove that A continues, B cannot unwrap or
  decrypt any post-cutover object, B has no accepted request, and old-epoch
  writes fail closed. The test documents the non-retroactive historical-data
  limitation.
- **Class-3 preservation:** after the base image, mutate representative local
  credentials and watermarks, staged rows, `device.*` settings, durable jobs,
  audit/idempotency state, and backup history. Successful and queued rebases
  preserve their canonical row values across swap and crash recovery while
  class 1 converges and class 2 rebuilds. Invalid local references and injected
  preservation failures leave the live vault untouched in
  `RebaseBlockedLocalState`.
- **Nonce lifecycle:** concurrent devices, rejected-CAS retries, crash/restart,
  re-snapshot, and epoch rotation never repeat a `(key_epoch, nonce)` pair.
  Injected reuse fails closed, and a boundary test rotates before the allocated
  invocation budget is exceeded.

### 12. The local-app promise

The fact-sheet invariant, to ship in S2a and be protected by
`check-claims`, is:

> The local app needs no DohFlow server and no DohFlow account. Optional paid
> services use an account; where a DohFlow server holds your data, it holds
> ciphertext only.

No Sync implementation may weaken the manual, offline local-app path in order
to make the optional service useful.

ADR 0075 and ADR 0078 remain separate downstream records; this ADR neither
changes them nor introduces an account requirement to the local app.

## Rejected alternatives

- **Replicate raw `vault.db`.** It carries `connector_connections.credential`
  and every vault rekey would invalidate every replica base.
- **Record an apply receipt with mint counts.** This would make apply control
  flow a wire format and would still be base-dependent under rebase.
- **Use a single-writer lease.** Expiry policy, takeover UX, and the
  two-writer window are more failure-prone than a compare-and-swap head.
- **Apply pulled envelopes over unpushed local state.** The first
  create-if-missing operation can fork canonical state.
- **Use HLC as the ordering primitive.** It is advisory; the service sequence
  is the authoritative total order under compare-and-swap.
- **Use ledger-row CRDTs.** Financial state needs intent-aware disposition,
  especially for assertions, toggles, and correlation groups.
- **Merge on the service.** The service does not hold a decryption key and
  must not receive plaintext canonical state.
- **Re-seal an unchanged Sync key on revocation.** The removed device already
  possesses that key, so rewrapping it cannot protect later ciphertext.
- **Use random GCM IVs for a shared epoch.** A per-device random draw does not
  make global uniqueness, crash recovery, or the epoch-wide budget enforceable.
- **Restore class-3 state from the rollback base.** It can erase later local
  credentials, staging work, settings, jobs, audit/idempotency state, or backup
  history.
- **Add recovery escrow in v1.** It widens the key-custody boundary before the
  dedicated recovery design and external review are complete.

## Consequences

- SYNC-1 (`personal-cfo-t2s4b`) adds operation-log-v2 envelopes;
  SYNC-1a (`personal-cfo-32fmp`) adds deterministic apply; SYNC-1b
  (`personal-cfo-tp2ln`) adds the simulator; SYNC-1c
  (`personal-cfo-bxsbz`) adds base-image rebase and class-3 preservation;
  SYNC-2
  (`personal-cfo-5ymg8`), SYNC-2a (`personal-cfo-07u`), SYNC-2b
  (`personal-cfo-egn67`), and SYNC-2c (`personal-cfo-0g528`) implement
  table routing, secret separation, tombstones, and payload materialization;
  SYNC-3 (`personal-cfo-1cltt`) adds genesis and the sealed Sync key. CLASS-0
  (`personal-cfo-w21q7`) additionally records every class-3 table's rebase
  preservation and reference-validation rule. S2-1
  (`personal-cfo-v98vd`) provides epoch authorization, credential revocation,
  nonce-range allocation, and the atomic cutover.
- ADR 0013-A, ADR 0073, ADR 0017, ADR 0077, ADR 0066-A, ADR 0024 Addendum A,
  ADR 0075, and ADR 0078 remain their own records. This ADR does not duplicate
  their detailed decisions.
- ADR 0066 receives one pointer to this record and is otherwise unchanged.
  ADR 0002's multi-device-sync revisit trigger is marked fired by its dated
  note. ADR 0066-A owns the Sync egress addition to the threat model.
- CLASS-0 is the machine-readable expression of Decision 5. The client and
  protocol are public when they ship; separate service self-hostability belongs
  to ADR 0078.
- This is documentation-only. It sets the S1 work budget at approximately
  25–30% more `WriteCommand` variants, or roughly 1.5–2 times an XL item,
  because the nine intent groups become explicit commands.

## Revisit if

- The planned external cryptographic review (`personal-cfo-5kua`) finds a
  fault in envelope encryption, key custody, or snapshot framing.
- Engine-version skew (R40) proves unmanageable in dogfooding.
- A future provider requires the service to process plaintext, which would
  violate the ciphertext-only rule and requires a new ADR rather than an
  amendment.
- Evidence establishes a recovery-escrow need; reopen D12 through
  `personal-cfo-efv` and external review.

## Implementation notes

- The simulator's required interleavings are mandatory S1/S3 exit tests:
  pull-first, seed-twin bootstrap, cross-device queue visibility, and rebase
  provenance.
- The first implementation change is not Sync transport. It is deterministic,
  payload-carrying command persistence under SYNC-1 and SYNC-1a.
- No existing historical operation-log row is recast as a replayable envelope;
  pre-v2 rows remain audit history.

## Linked beads

- `personal-cfo-j8ab7` (this ADR)
- `personal-cfo-t2s4b` (SYNC-1: operation-log v2)
- `personal-cfo-32fmp` (SYNC-1a: determinism)
- `personal-cfo-tp2ln` (SYNC-1b: two-vault simulator)
- `personal-cfo-bxsbz` (SYNC-1c: base image and rebase)
- `personal-cfo-5ymg8` (SYNC-2: write classes)
- `personal-cfo-07u` (SYNC-2a: connection identity and secret split)
- `personal-cfo-egn67` (SYNC-2b: tombstones)
- `personal-cfo-0g528` (SYNC-2c: payload materialization)
- `personal-cfo-1cltt` (SYNC-3: logical genesis and Sync key)
- `personal-cfo-v98vd` (S2-1: service CAS, device authorization, and epochs)
- `personal-cfo-elciw` (S3-1: client engine)
- `personal-cfo-mpep3` (S3-2: rebase queue UI)
- `personal-cfo-1df8d` (ADR 0013-A: derived IDs)
- `personal-cfo-vlfd` (ADR 0073: disposition rules)
- `personal-cfo-bc8iv` (ADR 0066-A: endpoint and egress posture)
- `personal-cfo-w21q7` (CLASS-0 table manifest)
- `personal-cfo-klr.4` (ADR 0077 and wire-format authority)
- `personal-cfo-5kua` (external cryptographic review)
- `personal-cfo-efv` (future recovery design)
