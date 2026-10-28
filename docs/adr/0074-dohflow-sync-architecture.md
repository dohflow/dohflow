# ADR 0074 — DohFlow Sync architecture

- **Status:** Accepted (owner decision, 2026-09-20)
- **Tier:** Public
- **Date:** 2026-09-20
- **Bead:** `personal-cfo-j8ab7`
- **Decider:** Owner, 2026-09-20
- **Related:** ADR 0002 (local encrypted vault), ADR 0011 (hybrid
  persistence), ADR 0013-A (derived identifiers, accepted in Addendum A), ADR 0066 (free
  local app and separate services), ADR 0073 (sync disposition rules, accepted
  at `docs/adr/0073-sync-disposition-rules.md`), ADR 0077 (wire formats,
  planned), and ADR 0082 (public disclosure boundary)

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
  `WriteCommand` enum has 50 variants
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
- The current attachment store is DEK-bound: `attachments` stores a local
  `storage_id`, wrapped content key, key/content nonces, and logical metadata
  (`crates/db-worker/src/migrations.rs:184-216`); `attachments.rs` writes blob
  ciphertext under that local ID and keeps the wrapped key in the row
  (`crates/db-worker/src/attachments.rs:1-16`, `:69-102`, `:129-160`). The
  crypto crate derives the ID from the vault DEK and wraps/unwraps each fresh
  content key under that DEK (`crates/vault-crypto/src/attachment.rs:100-121`,
  `:124-201`). A Sync replica with a different DEK cannot copy those local
  fields and expect the blob to decrypt.

These constraints mean Sync starts from a logical, encrypted representation of
canonical user intent and adds a versioned command tail. It is not retroactive
replay of existing operation-log history, raw-database replication, or a
ledger-row CRDT.

## Decision

### 1. Shape: logical genesis snapshot plus a command-envelope tail

At Sync enablement, a client publishes an encrypted logical genesis snapshot
of class-1 tables. It reuses the logical payload model of the Export Everything
bundle specified by `personal-cfo-klr.4` (ADR 0077), not that bundle's
user-facing password wrapper, with one additional consumer: class-1b artifacts use the
authenticated closure and opaque addressing rule below, and no class-3 row is
present. Afterwards clients publish versioned command envelopes. A replica
bootstraps from the latest snapshot plus its tail.

Re-snapshotting is the only way the service truncates history. A snapshot
publication carries `base_seq`; the service accepts it only when `base_seq`
equals the current head **and** the snapshot, its artifact closure, and every
closure pin are durable in one CAS transaction. This preserves a complete
recovery path for every accepted envelope without making the raw vault database
a wire format.

#### Class-1b closure, opaque addressing, and lifecycle

The local `StorageId` remains the vault-scoped keyed address from ADR 0023:
`HMAC-SHA256(addr_subkey, plaintext_bytes)`, where `addr_subkey` is derived
from the vault DEK. It is encrypted metadata and is **never** a service-facing
plaintext hash. For Sync epoch `e`, a client derives an opaque service address
from that local identity:

```text
sync_artifact_id_e = HMAC-SHA256(
  HKDF-SHA256-Expand(K_e, "dohflow/sync-artifact-address/v1" || vault_id),
  StorageId
)
```

`K_e` is the epoch root defined in Decision 8. The client derives the distinct
per-artifact outer-object key as
`HKDF-SHA256-Expand(K_e, "dohflow/sync-artifact-object/v1" ||
protocol_version || vault_id || key_epoch || sync_artifact_id_e, 32)`, then
seals the immutable artifact transport object under that key with the fixed
protocol nonce `0^96`. That nonce is safe only because the per-artifact key is
unique and invoked exactly once; the object has exactly one canonical ciphertext.
An exact retry sends the same bytes; a different byte string for the same
address fails closed. A fresh epoch therefore re-addresses and re-seals every
live artifact before its replacement snapshot is accepted.
The service can neither correlate the same plaintext across vaults nor use a
guessed plaintext to confirm an artifact; cross-vault deduplication is
forbidden. Local attachment dedup remains local.
Any plaintext integrity digest stays inside the encrypted artifact descriptor;
recipients verify the outer AEAD, then verify their receiver-local per-blob
AEAD and keyed local `StorageId` after materialization. A Sync-key epoch transition is thus an
intentional artifact re-upload boundary, not a reuse of a prior service object.

The portable encrypted payload is a canonical `SyncArtifactTransportV1` object.
It contains the opaque `sync_artifact_id_e`, artifact kind, plaintext length, a
plaintext integrity digest, allowed logical metadata, and the verified artifact
bytes. The bytes are encrypted by the outer Sync artifact object under the
epoch-derived key; the transport never carries an origin `StorageId`, blob
path, wrapped content key, content-key nonce, or origin blob ciphertext as a
replicated field. The digest and metadata remain inside the authenticated
encrypted descriptor, so the service still sees only the opaque ID and
ciphertext. Source-batch, source-record, parser-run, and attachment artifacts
all use this same portable object.

On receipt, the client verifies the outer AEAD and the descriptor digest before
materializing anything. It derives a receiver-local
`StorageId_B = HMAC-SHA256(addr_subkey_B, plaintext_bytes)` under the receiving
vault's DEK, generates a fresh local content key, encrypts the bytes with the
existing per-blob AEAD, wraps that key under `DEK_B`, and atomically writes the
receiver's blob plus metadata. It then records a device-local mapping from
`sync_artifact_id_e` to the local attachment ID, `StorageId_B`, wrapped key, and
nonces. No origin local crypto field is copied. A canonical attachment/link may
become usable only after that mapping commits; a missing or failed local
materialization returns typed `ArtifactMaterializationFailed`, discards
temporary plaintext/ciphertext, and leaves the live vault and cache unchanged.
The received canonical reference always preserves the origin's
`sync_artifact_id_e`; the receiver never recomputes a different service ID for
that object. Only a newly originated artifact derives a fresh address from its
own local `StorageId`, and same-vault cross-device local deduplication is not a
Sync requirement.

The class-1/1b schema is therefore split before Sync reads it: logical
attachment identity, links, size, and user metadata are replicated canonical
fields; `ref_count` is rebuilt as class 2 from those links; and
`storage_id`, wrapped/content-key fields, content nonces, blob paths, cache
state, and the materialization mapping are class-3 local fields.
The migration and manifest must make that split explicit; none of those local
fields may appear in a snapshot, tail, or service-visible metadata. A replica
may retain a pending encrypted descriptor while offline, but it must fetch and
materialize the object before resolving its local link.

Each retained snapshot carries a canonical, ordered **full closure manifest**
of its `sync_artifact_id_e` values; each envelope that creates, links, or cites
an artifact carries its canonical ordered **delta manifest**. The encrypted
payload repeats the applicable manifest. Its SHA-256 commitment is a
pre-encryption clear-header field and therefore authenticated by the enclosing
AEAD; the service stores the opaque manifest sidecar needed for pinning. A
client rejects a sidecar whose commitment differs from the decrypted manifest.
The service sees only vault routing metadata, opaque addresses, and ciphertext
objects—not a bare plaintext digest, filename, MIME type, or attachment bytes.

Publication is upload-before-reference: the client first uploads every opaque
artifact object and receives a durable receipt bound to its address and
ciphertext; only then may it submit an envelope or snapshot manifest. The
service rejects a manifest with a missing receipt. On accepted CAS it stores
the object, manifest, and pins atomically, so an active snapshot plus every
retained tail is a complete artifact root before any older tail is truncated.
If a client crashes before acceptance, the immutable `snapshot_id` makes the
publication query/idempotent; unpinned uploads remain retryable through the
published finite orphan-grace window and are eventually collected if no
publication succeeds.

Service GC is reachability-based: it may collect only an artifact with no pin
from an active or retained snapshot/tail and only after the accepted tombstone
is represented by a newer retained recovery root. The service publishes a
finite rebootstrap/retention window for superseded roots and orphan uploads;
it never shortens a previously announced window for an existing object. An
offline device outside that window bootstraps from the current complete
snapshot, retains its local outbox, and uploads any locally held artifact before
reissuing an artifact reference. It does not depend on an old tail remaining
available. A client may evict a local artifact cache only after it has both the
durable receipt and a confirmed pin, and never while an outbox, rebase, or
queue record needs that object. A missing required artifact enters typed
`ArtifactRebootstrapRequired`; it is not silently dropped or reconstructed from
a stale base.

`personal-cfo-1cltt` (SYNC-3) implements the logical snapshot, portable
artifact bootstrap, and receiver-local materialization;
`personal-cfo-0g528` (SYNC-2c) materializes opaque artifact references and the
portable transport descriptor;
`personal-cfo-v98vd` (S2-1) owns receipts, pins, retention, and GC;
`personal-cfo-elciw` (S3-1) owns upload, cache, and rebootstrap behavior; and
ADR 0077, implemented by `personal-cfo-klr.4`, owns the durable manifest and
wire-format specification. `personal-cfo-w21q7` (CLASS-0) supplies the table
classification.

### 2. No lease: compare-and-swap, then rollback-then-replay

The service head sequence is the sole write serialization point. A push carries
`base_seq`, an immutable envelope identity, and an opaque
`idempotency_key_tag`: a protocol-defined, keyed, vault-scoped fingerprint of
the canonical idempotency key, stable across key epochs and never the plaintext
key. Before checking `base_seq`, the service looks up **both** `command_id` and
that tag in its accepted-envelope index. An exact two-key binding with the
same canonical envelope digest and immutable-metadata commitment returns the
original accepted `seq` and result, even when the retry's `base_seq` is stale.
If either lookup finds a different bound command, tag, digest, or immutable
metadata commitment—or the two lookups disagree—the request fails closed as
`ProtocolFork`; it is never treated as a fresh command. This preserves the
same-`command_id` and same-`idempotency_key` fork rule after the tail is gone.
Only an envelope for which both identities are unknown is subject to the normal
CAS rule: the service accepts it if and only if `base_seq` equals the current
head, otherwise it returns the envelopes since `base_seq`. The accepted-envelope
index is retained with the operation log and is an authoritative,
recovery-durable record rather than a tail cache. It survives every tail
truncation, latest-snapshot bootstrap, and service restart/recovery, and is
retained for as long as any device can retry its outbox; Sync v1 defines no
finite retry horizon or automatic expiry. The index is privacy-minimal and
opaque: it contains only `command_id`, `idempotency_key_tag`, the canonical
envelope digest, the immutable-metadata commitment, the accepted `seq`/result
reference, and the recovery metadata needed to restore those bindings—never a
plaintext key or payload. A rebase or rebootstrap consults both identities
before ordinary replay even when the accepted envelope is no longer in the
pulled tail. There is no single-writer lease, expiry policy, or takeover window.

A client with unpushed work rebases by **rollback-then-replay** on a base image,
not by applying pulled envelopes over unpushed local state:

1. Enter `Rebasing` under the single-instance lock, freeze local writers, and
   capture a consistent **class-3 preservation image** plus an immutable
   rebase plan for every local outbox or queued correlation group. The plan
   records each original `command_id`, `idempotency_key`, old local `op_seq`,
   and audit evidence. It divides captured class-3 rows into stable rows and
   replay-owned rows: an idempotency memo or audit row that names a planned
   local command is replay-owned; unrelated rows are stable. Keep `vault.db`
   as of `base_seq` beside the live vault and advance it on every accepted
   push.
2. Restore that base image only as a scratch vault and apply the service
   envelopes. The base's class-3 rows are never authoritative after this step.
3. Remove the stale class-3 rows inherited from the base and transactionally
   overlay only the stable captured rows in the scratch vault, using the
   per-table preservation and reference-validation rules in CLASS-0. A
   replay-owned idempotency memo is deliberately absent at this point: it must
   never cause the ordinary dispatcher to return `Replayed` before the command
   mutation exists in scratch. Current device-local state wins over the base;
   it is never silently merged with, or replaced by, stale base state.
4. Before ordinary reapply, consult the authoritative accepted-envelope index
   for every planned outbox envelope by both its `command_id` and its
   `idempotency_key_tag`, even when the accepted envelope has expired from the
   pulled tail. Correlate the index record and any pulled envelope by immutable
   `command_id`, the authenticated tag, canonical envelope digest, immutable-
   metadata commitment, and the complete original `CommandMeta` plus
   payload/schema/version fields. An exact match is **already accepted** whether
   found in the index or tail: bind the local outbox record to the accepted
   server `seq` and result, reconcile/remap its scratch audit and idempotency
   evidence, and remove it from the replay plan without replaying or queueing
   it. A same-`command_id` mismatch fails closed as `ProtocolFork`; a tag
   collision with a different command also fails closed as `ProtocolFork` in
   either source, with both envelopes retained for diagnosis. A command whose
   response failed before service acceptance has no index or tail match and
   follows the ordinary path below.

   Reapply each remaining local outbox envelope under its original
   `command_id` and complete original `CommandMeta` only when its write set is
   untouched and it still validates. This uses a narrowly scoped rebase
   executor, not a blanket idempotency bypass: the executor accepts only a
   command in the immutable plan, requires its replay-owned memo to be absent,
   performs normal command validation and canonical writes, then writes a new
   operation-log row and idempotency memo in the same transaction. The rebuilt
   memo points at the rebuilt `op_seq`, never the pre-rebase numeric value.
   Otherwise queue the whole correlation group for disposition, preserving its
   original envelope and audit evidence in `outbox_queue` but installing no
   dispatchable memo. An ordinary retry of that key returns typed
   `QueuedForDisposition`, not a successful no-op or a second apply.
5. Reconcile the replay-owned audit evidence with the automatic, already
   accepted, or queued disposition, rebuild class 2, and validate every
   class-3 reference to changed class-1 or class-1b state—including the durable
   receipt/pin closure of each referenced class-1b artifact and any required
   receiver-local materialization mapping—before the swap. An invalid reference
   enters a typed `RebaseBlockedLocalState` recovery state, preserves both the
   live vault and scratch evidence, and requires an explicit repair or
   re-bootstrap. It never drops the row or falls back to the old base.
6. Atomically swap the fully validated rebuilt vault and retry the push.

Nothing is discarded, including device-local work, and no envelope is applied
twice. A failed preservation or validation step leaves the live vault
byte-identical. Garbage collection and rebase are therefore one integrity
problem. The replica's operation-log row records `applied_base_seq` and
`rebased_from` for R46. Pulled envelopes are never applied over unpushed local
state.

`personal-cfo-bxsbz` (SYNC-1c) implements the base image and atomic rebase;
`personal-cfo-v98vd` (S2-1) implements the authoritative recovery-durable,
privacy-minimal accepted-envelope index across tail truncation, snapshot
bootstrap, and service recovery, plus the exact retry response;
`personal-cfo-vlfd` (ADR 0073) defines the disposition rules;
and `personal-cfo-mpep3` (S3-2) presents queued groups.

### 3. Envelopes: versioned payloads and verified class-1 writes

Every `WriteCommand` gains a versioned payload schema. Its envelope contains
the payload-schema version; the complete original `CommandMeta`, including
immutable `command_id`, `idempotency_key`, `issued_at`, `correlation_id`,
`causation_id`, and `device_id`; the engine and schema versions; `seq` and
`base_seq`; and a write-set hash over canonically ordered class-1 rows touched
by the command. Class-2 rebuilds are excluded from that write-set hash. It also
has a `canonical_envelope_digest` over the canonical clear header and exact
ciphertext bytes. This is an opaque equality token for accepted-retry
reconciliation, not a plaintext content hash; the authenticated header and
AEAD tag must verify before a client accepts a digest match.

The service-visible clear header carries the authenticated
`idempotency_key_tag` rather than the plaintext idempotency key. Its keyed,
vault-scoped derivation is stable across key epochs and domain-separated from
payload content; ADR 0077 owns the exact derivation and canonical encoding.
The tag and the immutable-metadata commitment are included in the canonical
envelope digest and in the pre-encryption authenticated header. A service or
recovery root therefore retains only opaque equality and fork-detection data,
never the key or payload itself.

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

ADR 0013 Addendum A — deterministic derived identifiers for apply-minted rows
(`personal-cfo-1df8d`) defines derived IDs before SYNC-1:
replicas never seed IDs locally. A genesis snapshot carries rows already seeded
by the origin. This makes the simulator's seed-twin case deterministic without
turning allocation control flow into a wire format.

### 5. Five write classes and the nine intent groups

The Sync model has five classes:

- **1 — replicated:** canonical user intent travels in the snapshot and tail.
- **1b — artifact:** user-supplied bytes travel as opaque, epoch-scoped Sync
  artifact objects, are fetched lazily, and are tombstoned rather than
  rewritten.
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
| `attachments` and `attachment_links` | 1 for logical rows/links; 1b for bytes | Logical attachment rows and links replicate with canonical data; bytes replicate by opaque `sync_artifact_id` and are fetched/materialized lazily only after their closure pin is durable. Local crypto fields never replicate. |
| `transaction_categorizations` | 1 | `merchant_memory.rs` is routed through the command bus by SYNC-2, eliminating the present split writer and preserving the user's category decision. |
| `settings` keys under `user.*` | 1 | Household-level settings follow the user through the command bus. This explicitly reverses the direct `set_setting` convention at `lib.rs:2595`; `device.*` settings remain class 3. |

Class 1 covers user-facing canonical intent; class 1b covers artifact bytes;
class 2 is rebuilt; class 3 includes connector credentials, refresh
watermarks, staged rows, idempotency keys, audit events, node-local values,
KDF parameters, `device.*` settings, durable jobs, backup history, and local
Sync key-epoch, nonce-counter, service-credential, and class-1b
materialization state. That materialization state includes receiver-local
`storage_id`, wrapped/content-key fields, content nonces, blob paths, cache
state, and the `sync_artifact_id_e` mapping. CLASS-0 records
the per-table preservation and correlation rule; replay-owned is a per-row
phase selected only when a row belongs to an active outbox/queue command, and
every other captured row is stable.
CLASS-0 records the exact per-table manifest. `personal-cfo-5ymg8` (SYNC-2)
implements the routing and module-level lint; `personal-cfo-07u` (SYNC-2a)
splits connector identity from device-local secrets.

### 6. Payload-reference rule

An envelope payload may reference only a class-1 row or a class-1b artifact by
its opaque `sync_artifact_id`; a bare plaintext digest or local `StorageId`
never crosses the service boundary. A replica resolves that reference through
its device-local materialization mapping, fetching and verifying the portable
transport object before a local link is usable. `CommitStaged` is reshaped to
carry its materialized transaction and cite the source record's opaque artifact
ID instead of a device-local staging ID. Source-batch, source-record, parser-
run, and state kinds become class-1b artifact envelopes using the same
portable transport and receiver-local re-encryption rule. `SkipStaged` stays
device-local and never ships.

`personal-cfo-0g528` (SYNC-2c) implements that materialization and a test
that checks every payload type against the CLASS-0 manifest.

### 7. Tombstones before the S1 freeze

The three hard deletes become tombstones before the S1 freeze. Their
`deleted_at` values use `issued_at`; readers, forecasts, and projections
filter tombstoned rows; reinstatement is the corresponding Toggle operation.
Canonical rows are never silently purged. Physical class-1b ciphertext is
collected only by the closure-and-retention rule in Decision 1 after its
tombstone is represented by a newer retained recovery root. This makes deletion
compatible with rebase and the ADR 0073 disposition model.

`personal-cfo-egn67` (SYNC-2b) implements this rule and covers all three
existing delete paths.

### 8. Client-side encryption, epochs, and key custody

Each epoch has a fresh, uniformly random 32-byte **Sync epoch root** `K_e`,
distinct from the vault DEK. Snapshots, envelopes, and the outer class-1b
artifact objects use AES-256-GCM only with protocol-derived keys from that
root; `K_e` is never used directly as an AEAD key. Every encrypted object
belongs to a monotonically increasing `key_epoch`, and `K_e` is sealed only to
the enrolled devices in that epoch. The same enrollment and `device_id`
mechanism is reused by the phone-container work in ADR 0024-A *(superseded
2026-10-18: no phone container is planned; see ADR 0024-A section 3)*. The service
stores sealed-key blobs and protocol metadata, never a decryption key.

Device revocation is a fresh-key, atomic epoch cutover — not a re-seal of a
key the removed device already knows:

1. An authorized remaining device generates a fresh Sync key for epoch
   `e + 1`, seals it to every remaining enrolled device, and encrypts a new
   logical snapshot at the current service head.
2. The service atomically verifies the device-management authorization and
   head, installs the `e + 1` sealed-key set and snapshot, records the active
   epoch, and revokes the removed device's enrollment and service credential.
3. The service rejects every old-epoch write, artifact receipt, or pin after
   the cutover. A remaining device can obtain its new sealed key and bootstrap
   from the new snapshot; a removed, offline device cannot decrypt or submit
   any post-cutover object.

Ciphertext already obtained before the cutover cannot be made secret
retroactively. That limitation is shown in the revocation UX; it does not
justify retaining the old key for new data. `personal-cfo-v98vd` owns the
service authorization, credential revocation, epoch transition, and atomic
cutover contract; `personal-cfo-elciw` owns the client Settings flow.

The active service is a sequencer, availability provider, and service-credential
enforcer; it is **not** a confidentiality authority or the root of nonce
uniqueness. A client-generated device encryption public key and immutable
`device_key_fingerprint` are bound in a client-verifiable, signed enrollment
record. The first Sync device creates the genesis membership record locally;
every later recipient addition or replacement requires approval by an existing
authorized device. The service may store and enforce that record but cannot
invent a recipient, replace a fingerprint, choose a nonce domain, or allocate a
counter. A compromised service can deny, delay, or withhold service, but cannot
make two honest devices encrypt under the same AEAD key and nonce or obtain an
epoch root by inserting its own device.

For every envelope or snapshot context, the client derives an independent
AES-256-GCM key:

```text
K_e,d,t = HKDF-SHA256-Expand(
  K_e,
  "dohflow/sync-aead/v1" || protocol_version || vault_id || key_epoch ||
  device_key_fingerprint || object_type,
  32
)
```

`K_e` is uniformly random, so this is the Expand-only use described by
[RFC 5869](https://www.rfc-editor.org/rfc/rfc5869.html); the versioned,
canonical context provides domain separation. A snapshot has its own
`object_type`, and no two enrolled devices may reuse a fingerprint. Artifact
objects use the separate per-artifact derivation in Decision 1. The system-wide
invariant is therefore uniqueness of `(derived AEAD key, nonce)`, not a
service-issued `(key_epoch, nonce)` promise.

The GCM nonce is a deterministic 96-bit value
`nonce_domain(32) ‖ invocation_counter(64)`, where `nonce_domain` is the first
32 bits of `SHA-256("dohflow/sync-nonce/v1" || vault_id || key_epoch ||
device_key_fingerprint || object_type)`. It is entirely client-verifiable. The
device owns a durable `next_counter` register for each
`(key_epoch, device_key_fingerprint, object_type)` context and persists the
advance before encrypting. The register is device-only state with an independent
anti-rollback witness; a missing, restored, or inconsistent witness enters typed
`NonceStateLost` and the client must not encrypt with that context. It must
re-enroll with a fresh device key/fingerprint or bootstrap a fresh epoch.

A crash or changed retry burns the counter, while an exact-byte retransmission
reuses the saved ciphertext. Each derived AEAD key has a `2^32`-invocation cap;
before the next invocation the client must complete a fresh-key epoch rotation.
The service records the first
`(key_epoch, device_key_fingerprint, object_type, nonce)` it receives and
accepts only an exact-byte retry thereafter; a non-identical reuse fails closed.
That record is a detection and idempotency backstop, not the security source.
This uses NIST's fixed-field/invocation-field construction
([SP 800-38D §8.2.1](https://doi.org/10.6028/NIST.SP.800-38D)) without trusting
the service to allocate either field.

The authenticated data is the canonical encoding of every pre-encryption clear
header field: `protocol_version ‖ vault_id ‖ key_epoch ‖ object_type ‖
object_id ‖ idempotency_key_tag ‖ immutable_metadata_commitment ‖ device_id ‖
device_key_fingerprint ‖ nonce_domain ‖ invocation_counter ‖ base_seq ‖
payload_schema_version ‖ engine_version ‖ schema_version ‖
artifact_manifest_commitment`. Object-type-specific absent fields—including
`idempotency_key_tag` and `immutable_metadata_commitment` on non-envelope
objects—use explicit canonical null markers. `object_id` is the immutable
`command_id` for an envelope or the fresh snapshot ID for a snapshot. The
service-assigned `seq` is deliberately not in AAD: it does not exist until
after the CAS accepts the already sealed object. The wire format binds the
returned sequence to that immutable object ID in the append-only service log.
Any AAD, epoch, object-type, fingerprint, domain, counter, tag, or commitment
mismatch fails closed.

After authenticated decryption, the receiver recomputes the keyed
`idempotency_key_tag` from the canonical idempotency key and recomputes
`immutable_metadata_commitment` from the complete immutable `CommandMeta`.
Either recomputation must equal the authenticated clear-header field before
the client applies the command, enters rebase, or consults/remaps the accepted
index. A separately tampered clear tag or commitment therefore retains the
ciphertext and diagnostic evidence but fails closed before any mutation,
replay, queue, or index reconciliation.

**D1:** ADR 0066 leaves this encryption mechanism open; this ADR supplies the
client-side ciphertext-only mechanism while leaving ADR 0066's free-local-app
and separate-service boundary otherwise unchanged.

**D12, owner decision 2026-09-20:** there is no recovery escrow in Sync v1.
This is a key-custody constraint for the encrypted protocol and is also stated
in Decision 10; it does not change the local password or backup-recovery
model.

`personal-cfo-1cltt` (SYNC-3) implements the sealed Sync epoch root and
snapshot encryption; `personal-cfo-klr.4` (ADR 0077) owns the canonical
header, derivations, nonce, manifest commitment, and compatibility format;
`personal-cfo-6kn` (ADR 0017) owns signed device-key identity and nonce-state
loss/re-enrollment; `personal-cfo-v98vd` owns service enforcement, the
dual-identity accepted-index lookup, and the duplicate-detection backstop; and
`personal-cfo-bc8iv` (ADR 0066-A) owns the endpoint and egress posture.

### 9. Identity, restore, and ordering

ADR 0017, implemented by `personal-cfo-6kn`, defines identity and clock
details: `device_id` is minted at enrollment, bound to a client-generated
device-key fingerprint and signed membership record, and stored outside the
vault; `node_id` is per vault and copied by snapshot or restore; `vault_id` is
the service storage key. The existing HLC remains advisory. The service
sequence is the total order.

For R39, a restore—or any `NonceStateLost` recovery—mints a new `device_id`
and device key/fingerprint; it never resumes an old nonce context. Sync
enablement never publishes a new genesis for a `vault_id` the service already
knows: it bootstraps a replica, retains local edits in the outbox, and sends
them through the ordinary rebase path.

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

The simulator also has four mandatory safety suites:

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
  `RebaseBlockedLocalState`. A separate applied-but-unpushed command case starts
  with its idempotency memo, lets a remote operation claim the old numeric
  `op_seq`, and proves automatic replay writes the mutation exactly once and
  remaps the memo; queued resolution preserves audit evidence and blocks an
  ordinary retry; unrelated idempotency rows survive. A separate accepted-CAS
  then lost-response case matches the pulled envelope by command ID, canonical
  envelope digest, and complete metadata, binds the outbox to the remote
  `seq`, and never replays or queues it; a same-ID/different-envelope case
  fails closed as `ProtocolFork`. A failure before service acceptance still
  takes the ordinary replay path. The accepted case is repeated after tail
  expiry, latest-snapshot rebootstrap, and service restart/recovery from the
  latest retained root; the durable accepted-command index still binds exactly
  one mutation/op-log row and preserves the audit/outbox evidence. A genuinely
  pre-acceptance failure at the same age has no index entry, performs no
  automatic mutation or silent loss, and preserves its evidence for ordinary
  replay. The same long-offline sequence then submits a second envelope with a
  new `command_id` but the first command's `idempotency_key_tag`; dual lookup
  returns `ProtocolFork`, creates no second mutation/op-log row, and retains
  diagnostic evidence. An exact retry of the first envelope still returns its
  original `seq`/result, and the retained lookup contains no plaintext key or
  payload. Protocol cases separately tamper the clear `idempotency_key_tag` and
  the clear `immutable_metadata_commitment` while retaining the ciphertext;
  authenticated-AAD/recomputation validation fails closed before apply, rebase,
  or index reconciliation, while retaining ciphertext and diagnostic evidence.
- **Nonce lifecycle:** concurrent devices, rejected-CAS retries, crash/restart,
  re-snapshot, and epoch rotation never repeat a `(derived AEAD key, nonce)`
  pair. The test sequencer deliberately offers overlapping domain/range grants
  and rolls back/replays its own records; clients do not rely on those grants,
  A and B derive different AEAD keys even with equal counters, and a local
  counter rollback fails closed or re-enrolls. Injected same-context reuse fails
  closed, and a boundary test rotates before the per-derived-key budget is
  exceeded.
- **Artifact closure and lifecycle:** a snapshot with a missing artifact is
  rejected; crashes before and after publication preserve a complete recovery
  root; every object reachable from a retained snapshot/tail remains fetchable;
  concurrent re-snapshot, an offline-device tombstone/rebootstrap, and eventual
  orphan collection preserve that invariant. A two-replica fixture uses
  independently generated DEKs: the receiver verifies a portable transport
  object, derives a different local `StorageId`, re-encrypts bytes with a fresh
  local content key, wraps it under the receiver DEK, and records only a
  device-local materialization mapping. Origin crypto fields never enter the
  snapshot, tail, or service metadata; wrong-DEK unwrap, corrupt transport,
  failed materialization, and cache cleanup all fail closed. The same plaintext
  in two vaults or two Sync epochs has unlinkable service addresses and outer
  ciphertext; guessed plaintext cannot confirm presence, while altered bytes
  fail every required integrity check.

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
- **Let the service allocate the nonce domain or counter range.** A compromised
  sequencer could equivocate overlapping grants; client-derived AEAD keys and
  client-owned counters make that behavior unable to create a duplicate key/
  nonce pair.
- **Restore class-3 state from the rollback base.** It can erase later local
  credentials, staging work, settings, jobs, audit/idempotency state, or backup
  history.
- **Overlay every captured idempotency memo before replay.** The ordinary
  dispatcher would treat an absent scratch mutation as a successful replay; the
  rebase plan instead remaps or queues only the memo/audit rows it owns.
- **Treat an accepted push with a lost response as an ordinary replay.** An
  exact command/digest match must bind the outbox to the already accepted
  sequence; a same-ID mismatch is protocol corruption, and an unknown command
  remains eligible for normal replay.
- **Store plaintext idempotency keys in the durable index.** A stable,
  vault-scoped keyed tag preserves dual-key fork detection after tail expiry
  without exposing the key or payload to the service or recovery roots.
- **Accept or truncate a snapshot without an artifact closure.** A canonical
  row could then outlive the only recoverable ciphertext for its artifact.
- **Expose a bare content hash or reuse a service artifact address across an
  epoch.** Either permits equality/confirmation leakage that an opaque,
  epoch-scoped outer object avoids.
- **Copy the origin attachment `StorageId` or wrapped key into a replica.**
  Those fields are bound to the origin DEK; a receiver must verify portable
  bytes and re-encrypt them under its own DEK and local content key.
- **Add recovery escrow in v1.** It widens the key-custody boundary before the
  dedicated recovery design and external review are complete.

## Consequences

- SYNC-1 (`personal-cfo-t2s4b`) adds operation-log-v2 envelopes with complete
  replay metadata; SYNC-1a (`personal-cfo-32fmp`) adds deterministic apply;
  SYNC-1b (`personal-cfo-tp2ln`) adds the simulator; and SYNC-1c
  (`personal-cfo-bxsbz`) adds phase-specific class-3 rebase preservation,
  memo remapping, and crash-safe swap. SYNC-2 (`personal-cfo-5ymg8`), SYNC-2a
  (`personal-cfo-07u`), SYNC-2b (`personal-cfo-egn67`), and SYNC-2c
  (`personal-cfo-0g528`) implement table routing, secret separation,
  tombstones, portable opaque artifact transport/materialization, and closure
  deltas. SYNC-3 (`personal-cfo-1cltt`) adds genesis, the sealed epoch root,
  receiver-local artifact bootstrap, and DEK-separated materialization. CLASS-0
  (`personal-cfo-w21q7`) additionally records every
  class-3 table's rebase phase and reference-validation rule. S2-1
  (`personal-cfo-v98vd`) provides signed-membership enforcement, credential
  revocation, receipt/pin/retention/GC service behavior, duplicate detection,
  and the atomic cutover; it is not a nonce allocator.
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

## Addendum A — Wire-format clarifications (ADR 0077, 2026-10-18)

ADR 0077 and Part C of `docs/architecture/vault-format.md` fix the encodings and
records this ADR delegates. They change no decision above. They record the
following, which this ADR left open or implicit:

- **The idempotency-tag secret.** `idempotency_key_tag` is keyed by a
  non-rotating, vault-scoped secret `S`, sealed to each device together with
  `K_e` and re-sealed unchanged at every epoch. `S` is an equality-tag key, not a
  confidentiality key, so Decision 8's rule against retaining an old key for new
  data does not apply to it. A device revoked after learning `S` can recognize
  only the tags of idempotency keys it already saw: every idempotency key that
  enters a Sync envelope is a UUID version 4 from the OS CSPRNG, enforced by the
  envelope builder. That residual is accepted.
- **Retry order.** Decision 2's accepted-index lookup runs **before** the service
  rejects an old-epoch write, so an exact retry of an accepted envelope returns
  its original `seq` and result across a cutover. A client re-encrypts an outbox
  entry under a new epoch only after the index confirms that both its
  `command_id` and its tag are unknown.
- **Sealing the epoch key.** Decision 8's sealed keys travel only in an epoch
  record signed by an authorized member and checked against the membership chain,
  with key commitments for `K_e` and `S`, because the sealing primitive is
  unauthenticated. Adding a device installs a grant that seals the current `K_e`
  and `S` to it; only a revocation cutover or a forced rotation changes the key.
- **Epoch position.** A client rejects any envelope whose `key_epoch` is older than
  the epoch in force at its position in the log. The AAD of Decision 8 is unchanged.
- **Suite agility.** Sealed-key blobs and membership and epoch records carry
  `suite_id` and `kem_id`, so a later hybrid key-encapsulation suite can be adopted
  at a new key epoch.
- **Full history on a new device.** The genesis snapshot of Decision 1 carries
  every class-1 ledger row of every age; only pre-v2 operation and provenance
  history is not carried.

## Addendum B — Cross-device connector ingestion (2026-10-18, `personal-cfo-g3m.12`)

Two devices that each hold their own provider credential refresh the same bank
account independently. They must never commit the same provider transaction
twice, and neither may depend on the other being online. This addendum records
the owner's 2026-10-18 decision for that case (`personal-cfo-g3m.7`): derived
provider-transaction identity plus a rebase-time `AlreadyPresent` rule, **no
lease**. It rewrites none of the decisions above; Decision 2 (no lease,
compare-and-swap, rollback-then-replay) stands, and the rule is built on it.

**1. The problem.** Every duplicate check that exists today reads class-3 staged
rows (`crates/db-worker/src/ingestion.rs`: the cross-source duplicate reason and
the fingerprint tracked and committed checks). Staging does not replicate. After a
normal Sync, device B has A's committed transactions but not A's staged rows, so B
refreshes the same provider data, finds nothing to match against, and commits it
again. The gap is permanent, not a race, and no timing rule closes it: detection
must read class-1 state.

**2. Derived provider-transaction identity.** Each provider-imported transaction
gets a content-derived id that every device computes identically from the
provider's own identifiers: an HMAC of the per-vault `id_key` over a
domain-separated message of the replicated `connection_id`, the provider account
key and the provider transaction id, laid out as a UUID version 8. It is never
derived from `issued_at`. It is computed when the commit command is constructed and
carried in the `CommitStaged` payload, so it is not an apply-minted id and
consumes no derived-id ordinal. The exact byte grammar, the validation of the
provider strings, the argument that it cannot collide with command-scoped ids, and
a known-answer vector are in ADR 0013 Addendum A, section 9. Because the identity
is byte-exact, each adapter's account-key function is part of it: it is defined
canonically and pinned by a test, and changing it is a payload-schema change. If a
provider starts or stops returning a component of the key for an account that is
already linked, that is an identity change that queues for the user, never a silent
re-key.

**3. Class-1 identity records.** Two new class-1 tables carry the rule across
devices (their names and columns are fixed by the beads that implement the rule,
not here):

- a **provider-identity record** with `UNIQUE(connection_id,
  provider_account_key, provider_txn_id)`, mapping each committed provider
  transaction to its derived id; and
- a **skip / import-anyway disposition** keyed by the derived identity, so a
  user's choice to skip or to re-import a provider transaction replicates and is
  honoured on every device.

`connection_id` and the account mappings are the replicated connection identity of
`personal-cfo-07u`; every device that holds a token refreshes on its own.

**4. The apply rule and the `AlreadyPresent` exception.** Inside apply, a commit
of a provider transaction looks up its derived id in the provider-identity record
(class-1 rows only; apply never reads class-3 staging). The same check runs at
**local commit**, not only at rebase. The outcomes:

| Situation | Disposition |
| --- | --- |
| The identity is absent | Commit as usual. |
| Present, and the provider-financial fields are equal | A no-op, plus a dedupe decision and an audit line. No inbox item. |
| Present, and the provider-financial fields differ | One disposition item (queue-choose). |
| Present, and the transaction is voided or tombstoned | A no-op. Re-importing never resurrects it; an explicit restore is a separate Reinstate command. |

The provider-financial fields are the ones the provider supplies (amount,
currency, posted date, provider description). **User-editable fields (category,
note, tags, splits, review state) are never compared and never overwritten.**
This is a new disposition in the rebase executor, `AlreadyPresent`, an exception
to the strict auto-iff rule of ADR 0073 Decision 1 (an envelope whose write set
was touched queues): a replayed `CommitStaged` whose identity is already present
is resolved by this table instead of by the touched-write-set rule. ADR 0073
records it under its exceptions (section 3, item 6).

**5. Void and reinstate.** A void keeps the provider-identity record, so a later
refresh of the same provider transaction finds the identity and takes the voided
row of the table above. Reinstating a voided transaction is an explicit command; a
refresh never does it.

**6. What never feeds identity.** A **pending** provider transaction is never
committed. A **fallback fingerprint** (a heuristic hash of date, amount and
description) never feeds a derived id: a transaction without usable provider
identifiers has no provider identity and is handled by the existing heuristic
review path, not by this rule. **File imports** have no provider identifiers, so
they derive their identity from the file's content fingerprint and the row's
ordinal in it, under their own domain label (fixed by `personal-cfo-0g528`).

**7. Provider revisions.** If a provider later revises the amount or posted date
of a transaction under the same provider id, the identity is present and the
provider-financial fields differ, so it is one disposition item (queue-choose):
the user decides whether to take the provider's revision. It is not applied
silently.

**8. Non-goals.** There is no refresh lease, no designated committer and no
clock-based failover (ADR 0017 and Decision 2 forbid a clock deciding who may
write). Re-issued provider ids, where a provider reuses an id for a different
transaction, are out of scope.

**9. Downstream changes.** `personal-cfo-0g528` (import-family reshape) carries
the origin's heuristic dedupe decisions and evaluates the class-1 identity guard in
apply; the dedupe-decision record is reshaped off the class-3 staged id so it can
replicate; `personal-cfo-bxsbz` (base image and rebase) implements the
`AlreadyPresent` exception; and `personal-cfo-tp2ln` (the two-vault simulator) gains
the nine cases that exercise this rule.

**10. What the user sees.** An exact collapse is an audit line only. A mismatch is
one disposition item in the queue, with both values. A voided transaction that a
refresh sees again is silent.

**11. Verified scope.** The rule depends on one assumption: that a provider returns
the **same account keys and transaction ids to different credentials** for the same
accounts. That is **verified for SimpleFIN**: on 2026-10-18 two setup tokens on one
Bridge subscription, claimed to two different access URLs and each queried once
(accounts and recent posted transactions), returned identical `(connection id,
account id)` keys for every account and identical transaction-id sets on every
account (`personal-cfo-g3m.10`, recorded on `personal-cfo-g3m.7`; only identifiers
were compared, and the figures stay on the bead). It is **pending for LunchFlow** until
`personal-cfo-g3m.11` records the equivalent check for a second LunchFlow key
(planned before paid Sync). **Until then LunchFlow connections are not covered by
this rule and must not be described as covered.** If a provider's ids differ across
credentials, this decision returns to the owner.

## Addendum C — Bring items from another vault: merge-import (2026-10-19, `personal-cfo-g3m.15`)

A person who joins a device that already has its own vault does not lose that vault's
contents. v1 joins by replacing the device's vault after a mandatory backup; the later
"Bring items from another vault" feature (`personal-cfo-i93n3.7`) reads that saved vault
and brings its unique items into the household vault. This addendum records the engineering
decisions that feature rests on. It rewrites none of the decisions above: there is still one
synced vault per household, and Decisions 1 to 3 govern everything the merge writes.

**1. The merge model.** Items from the source vault are **replayed into the household vault
as ordinary `WriteCommand`s through the command bus**. There is no direct table copy and no
second synced vault. The saved vault is opened read-only as *data*, in its own temporary
container, by the same code that reads a backup; it is never a Sync participant. Each
replayed command is an ordinary command with its own CSPRNG UUIDv4 idempotency key
(ADR 0013-A section 3 and `vault-format.md` C.5), its own `command_id`, and `issued_at` the
time of the merge. Everything the Sync engine does to any command (envelope, write-set
hash, rebase, disposition) applies to these unchanged.

**2. Merge-import derived identity.** Every row the merge **creates** gets a
content-derived id, defined in ADR 0013-A section 11: a domain-separated HMAC under the
**household** vault's `id_key` over the source `vault_id`, a source entity tag and the
source row's key. Running the same merge twice, or on two devices, therefore mints the same
ids. The id is computed when the command is constructed and carried in its payload, like
the provider-derived ids of Addendum B and the caller-minted ids of ADR 0013-A section 3.
A second creation of an id that already exists with equal content is the `AlreadyPresent`
no-op of Addendum B item 4 and ADR 0073 exceptions item 6; with different content it is one
disposition item for the user, never a silent overwrite. Every row a replayed command creates
carries a derived id in its payload, child rows included (ADR 0013-A section 11), so a second
run compares like with like. A merge cancelled halfway leaves the rows it created in place; a
later run finds them `AlreadyPresent` and converges.

**A copy of the household's own vault is not a merge source.** Several things share the
household's actual row ids: its own backup or a restore twin (same `vault_id`), a portable
export imported into a fresh vault (new `vault_id`, every primary and foreign key preserved),
and another device's Sync replica. Merging any of them under derived ids would duplicate every
row. So (a) a source whose `vault_id` equals the household's is refused with a typed error
("this is a copy of this vault; use restore"), and (b) a source row whose **primary key
already exists in the same household table is an exact match** that maps to that row and is
not created (item 4): the same key means the same origin. Equal content is a no-op; differing
content is one disposition item. Two backups of the same *other* vault taken at different times
are unaffected: they derive the same ids from the same source `vault_id`, so the newer one
converges on the rows already merged, and changed rows become disposition items.

**3. The id remap.** A replayed command refers to other rows (an account, a category, tags).
The merge keeps a **remap from source ids to household ids** for one merge session:

- a source row the merge *creates* maps to its derived id (item 2);
- a source row *matched* to an existing household row (item 4) maps to that household row's
  id;
- a source row *skipped* maps to nothing, and a command that refers to it is not replayed.

The remap is **device-local working state (class 3)**, recomputable from the saved vault,
the household vault and the user's decisions, and discarded when the merge finishes or is
cancelled; it is never a source of truth. References are rewritten **when a command is
constructed**, so every payload that enters the log carries final household ids and none
refers to a source-vault id except through the derivation inputs. The user's decision on a
likely match travels in the commands themselves (they name the existing household id), so
other devices converge without sharing the remap.

**4. Matching.** *Exact* matches skip creation automatically and need no user input:

- a source row whose primary key already exists in the same household table (the shared
  origin of a copy, below) maps to that row;
- a connector transaction or account with the same provider identity (Addendum B items 2
  and 3) maps to the existing row, which the `AlreadyPresent` rule then confirms;
- a file-imported row with the same file fingerprint and row ordinal maps to the existing
  row (the file-import identity of `personal-cfo-0g528`);
- a system category or system tag with the same canonical name maps to the household's.

*Likely* matches (an account with the same name, type and currency; a transaction with the
same amount, date and counterparty) are **never automatic**: they are shown in the compare
report and created or matched only by the user's choice. Anything with no match is created
under its derived id.

**5. Matched accounts.** For an account the user matches, the household account stays the
identity; the user chooses, per account, which side's **opening balance and balance
assertions** apply: the household's (the default) or the source's. Choosing the source's
issues the ordinary balance-assertion commands with the source's values; the other values are
not merged in, because two balance assertions for one account on one date are a conflict, not
two facts.

**6. The honest costs.** History arrives as **new commands with new ids and the time of the
merge**, not as the source's original operation log, which is not replayed verbatim. A
merged transaction shows the time it was brought in as its operation time, with the source
date as its transaction date. Edits made to a matched row in the source vault, after the
backup was taken or before the user chose, are not carried unless the user picks the source's
values. A merge is undone the way any commands are: by correcting or voiding the rows it
created, not by rolling the merge back as a unit.

## Addendum D — Recovering from a poisoned log position: the member-signed void (2026-10-25, `personal-cfo-g3m.24`, owner decision `personal-cfo-915.19`)

Decisions 2, 3 and 8 make a replica fail closed on anything it cannot verify, and Decision 3
sends a write-set mismatch to re-bootstrap. They leave one case open. The service stores
ciphertext it cannot read, so it cannot tell a consistently sealed envelope from one a buggy or
partly failed producer sealed inconsistently, and it cannot tell one that passes every receiver
check but then fails to apply deterministically. Either kind is accepted at a log position `N`.
Every honest replica then reaches `N`, rejects it the same way, and stops (`AuthenticatedEnvelopeInvalid`,
`vault-format.md` C.4 and C.11). That state is not a fork (nobody equivocated) and not transient
(retrying changes nothing).

**Re-bootstrap alone cannot leave it.** A snapshot must equal the head (Decision 1), so a snapshot
needs a replica that has applied everything up to the head, and nobody can apply past `N`. The
newest snapshot is therefore at or before `N - 1`, and bootstrapping from it lands on `N` again.
Recovery needs a way to declare `N` void. The owner chose the mechanism below on 2026-10-25
(`personal-cfo-915.19`), after an adversarial read by `03a-escalation`.

**Rejected: a recovery-root skip.** A key that can authorize skipping a position is an escrow
key by another name; Decision 10 says Sync v1 has no escrow. **Rejected: producer self-repair.**
Re-sealing the same `command_id` with a different body is a fork (Decision 2). This addendum
rewrites none of the decisions above; it adds one record type and the rules for honoring it.

The numbered rules below are normative. The record's byte encoding is `vault-format.md` C.10
"Void records" (spec-version 10, with the vectors `void_body` and `void_id` in C.14).

**1. The receiver-check rule set is bound to the engine version (R40).** What
`check_envelope` accepts and rejects, and whether an envelope applies deterministically, are part
of what `engine_version` names. Changing what is accepted or rejected **is** an `engine_version`
change. Two replicas that both evaluate position `N` have the same `engine_version` (a replica
with another one is in the update-required state and applies nothing), so they run the same checks
and cannot disagree about whether `N` is valid within one version. A device registry that never
decrypts is not part of that agreement.

**2. The void record.** A void names a position `seq`, the `command_id` and
`envelope_digest` of the envelope seen there, a `reason` (receiver-check failure, or
deterministic apply failure), the signer's fingerprint and a time. It is signed by an authorized,
unrevoked member of the membership chain and verified under **the key the chain holds** for that
member (never a key carried in the record). The signature is **not authority over the outcome**:
trust comes from rule 3. It makes the decision attributable (nothing is skipped silently), gives
replicas a trigger to roll back, and keeps non-members from filling the log with voids.

**3. A replica honors a void only after rejecting `N` itself.** It must have evaluated position
`seq` in log order, found it invalid (as `vault-format.md` C.4 defines it, in stages: a
position it has reached by processing the log in order, a verified key for the epoch in force
there, and the device a member **at that position**, so a missing key or a position not yet
reached is never "invalid"; for any deterministic reason; the void's `reason` byte is for audit),
and see the same `command_id` and `envelope_digest` in the void. Anything else is
inert: kept for diagnosis, skipping nothing. The service cannot make this decision (it cannot read
the plaintext), so it never refuses a void because an earlier void's range would cover it; it
dedupes only an identical void or a second void for the exact same position. Otherwise a member
could append a void for a *valid* envelope and block every later genuine void inside its range.
The signer must be an authorized member at the membership head in force at the void's position. A
malicious member therefore cannot void a **valid** envelope (this is recovery, not censorship),
and spam is inert. A replica stopped at `N` always evaluates `N` before it can read a void
positioned after it.

**4. The range is `N` through `V - 1`.** If the void is appended at position `V`, every command
envelope at positions `N` through `V - 1` is skipped, because every object after `N` carries a
`base_seq` at or past `N` and was built on a state that included it. Their issuers rebase and
re-issue (rules 6 and 7). Membership and epoch records are never skipped: they are verified by
their own chain rules. A void whose `seq` an earlier honored void already covers is inert, so the
first honored void defines the range, and the objects after `V` are valid and apply. An earlier
void that was itself inert covers nothing, and a void for a position at or after `V` is an
independent poisoning honored on its own. A snapshot inside the range, including the snapshot
coupled to a key-changing epoch, was built on poisoned state and is never a valid bootstrap root;
a bootstrapping device uses one before `N`, or one at or after `V` that records the void. An epoch
record inside the range still takes effect for keys.

**5. No snapshot and no key-epoch cutover while halted.** A replica stopped at a poisoned
position publishes no snapshot, performs no cutover and pushes nothing. (A snapshot needs the
head, which such a replica cannot reach, and a cutover from a stopped replica would be built on a
state that skips `N`.) After the void is honored the replica resumes after `V` and is no longer
halted. **Operator order:** if the device that poisoned the log should be revoked, the void comes
first and the cutover after it; no cutover can happen while halted, so nobody should wait on a
revocation that cannot yet be made.

**6. Re-issue uses a new `command_id` and a new idempotency key.** Reusing the key hits the tag in
the accepted-envelope index and is a `ProtocolFork` (Decision 2). The index keeps the record of
each position a void names and records **"void claimed by `<void id>`"** against it. The service
cannot know whether a void was honored, so it never says a position *is* voided: an exact replay
returns the original `seq` plus any void claims, and the client decides from its own verdict,
rolling back and re-issuing only if it honored that void. A replica never re-applies a position
it skipped. A claimed position's `command_id` and tag are never reusable, either way.

**7. The producer rolls back.** A device whose outbox entry was bound to a position in `N` through
`V - 1` (it saw its own envelope accepted and bound it) must, on honoring the void, unbind and
roll that command back through rollback-then-replay (Decision 2), then re-issue it as a new
command through the producer rule (`vault-format.md` C.4). It never re-pushes the original
envelope.

**8. Evidence is retained.** The poisoned envelope's ciphertext and the void are kept, not
garbage-collected before a snapshot that supersedes the range. Snapshots taken past a void record
the **voided ranges and the void ids** in their metadata, so a device that bootstraps from one can
show the history and verify that the skipped positions were voided. The index's void claims are
permanent (Decision 2).

**9. The `check_envelope` conformance vectors are part of the protocol.** With rules 1 and 3 the
receiver checks become consensus-critical: two implementations that disagree on one envelope would
split the vault. So the vectors (`personal-cfo-g3m.23`: every receiver error, plus valid cases,
public and versioned, a new error without a vector failing a test) are normative, and any second
client implementation must reject exactly the same envelopes. The void rules have their own
honor-and-refuse vectors in the same family (`personal-cfo-g3m.17` part C).

**10. Deterministic apply failures are in scope.** An envelope that passes the checks but cannot
be applied deterministically (Decision 3: apply is a function of payload, metadata and base state)
poisons its position the same way. A void is honored only if the replica **itself** failed to
apply the position with a *deterministic rejection*. An error is one only if **all** of these
hold: it is a pure function of the payload, the metadata, the base state and `engine_version`; it
is raised before any write commits, so the transaction rolls back; and it depends on no class-3 or
device-local state. Excluded by name: I/O errors, `SQLITE_BUSY` and disk-full conditions,
`CipherUnavailable`, `NonceStateLost`, resource exhaustion, and a write-set mismatch. A write-set
mismatch stays a fork (Decision 3), not voidable, so a replica that diverges for a local reason
cannot be talked into skipping, and a replica that misclassifies a transient local failure cannot
honor a void the others reject. The closed list of members is fixed with the apply code
(`personal-cfo-g3m.17` part C, `personal-cfo-elciw`) against these criteria.

**11. The normative encoding lives in `vault-format.md`.** The void record's body, signature,
service checks and index marking are in C.10 "Void records" under C.13 change control
(spec-version 10), with vectors. This addendum states the rules; that section states the bytes.

**Consequences.** (a) A split is still possible if two replicas at one `engine_version` reach
different verdicts on `N` because of a bug; rules 1, 3 and 9 make that a defect to be found by
vectors rather than a protocol state, and the divergence is caught by the write-set hash (a fork,
Decision 3). (b) A voided range costs the commands in it, which their issuers re-issue; the
re-issued commands take new positions, so their order relative to unrelated commands can change.
(c) The service gains a second signed-record type to order and void claims in the index; it
still never reads a payload. (d) A void is a tool for a rare, serious state; the producer rule
(`vault-format.md` C.4) and the service checks exist to make it unnecessary.

**Implementation.** `personal-cfo-g3m.17` part C (the void record and the honor-or-refuse rule in
`sync-core`), `personal-cfo-v98vd` (the service checks, ordering and index marking),
`personal-cfo-elciw` (the client: honoring, rollback, re-issue, the status and its persistence),
`personal-cfo-g3m.22` (the receiver-path opener) and `personal-cfo-g3m.23` (the vectors).

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
- `personal-cfo-g3m.15` (Addendum C and ADR 0013-A section 11: merge-import identity)
- `personal-cfo-i93n3.7` (Bring items from another vault)
- `personal-cfo-g3m.7` (cross-device connector ingestion: the decision behind Addendum B)
- `personal-cfo-g3m.10` (SimpleFIN two-token id check)
- `personal-cfo-g3m.11` (LunchFlow two-key id check, pending)
- `personal-cfo-g3m.12` (Addendum B and the ADR 0013-A section 9 note)
- `personal-cfo-g3m.17` (Sync liveness: the producer rule, the named state, and the void's sync-core part)
- `personal-cfo-g3m.22` (the vault-crypto receiver-path opener)
- `personal-cfo-g3m.23` (the `check_envelope` conformance vectors)
- `personal-cfo-g3m.24` and `personal-cfo-915.19` (Addendum D and the owner decision behind it)
