# DohFlow Sync service protocol

<!-- sync-service-protocol: protocol-version=1 -->

This is the public, versioned specification of the **request interface** between
a DohFlow Sync client and a Sync server. It is the second half of the published
protocol. The first half is the data format: the envelope, snapshot, membership
and epoch records and the canonical encodings of
[`vault-format.md`](vault-format.md) Part C and
[ADR 0077](../adr/0077-wire-formats.md). This document does not restate those
formats. It cites them by section and says how a server receives, orders,
stores and returns the objects they define.

Any server that implements this document can be used by the DohFlow desktop
client without a client fork, by setting the endpoint in Settings behind the
trust prompt of [ADR 0066-A](../adr/0066-A-dohflow-endpoints-pinned-credential-not-unlock.md)
decision 2. The DohFlow-operated service is one implementation of this protocol.
Nothing here describes or constrains a server's internal design, hosting, or
business terms.

- **Protocol version:** `1`. The machine-readable operation, error and limit
  lists are in [`sync-service-protocol.json`](sync-service-protocol.json). The
  tables below and that file must agree; CI checks it
  (`scripts/tests/sync-service-protocol-contract.test.sh`).
- **Status:** specified; no server or client implements it yet. The client
  engine (`personal-cfo-elciw`) and the server (`personal-cfo-v98vd`) are built
  against this document.
- **Sources of authority:** [ADR 0074](../adr/0074-dohflow-sync-architecture.md)
  decides what Sync protects and how it behaves; ADR 0077 and `vault-format.md`
  Part C fix the bytes; ADR 0066-A fixes the endpoint, credential and logging
  boundary. This document changes none of their decisions. Where this document
  and one of them disagree, that one wins and this document has an erratum.

The key words MUST, MUST NOT, SHOULD and MAY are used as in
[RFC 2119](https://www.rfc-editor.org/rfc/rfc2119).

## 1. Principles

1. **The server is a sequencer and a store, never a key holder.** It orders
   ciphertext, enforces the signed membership chain, enforces credentials, and
   returns what it stored. It cannot decrypt, add a recipient, choose a nonce or
   counter, or merge data (ADR 0074 decisions 2 and 8). Section 9 lists what it
   never receives.
2. **The credential is an endpoint credential, never a local unlock.** A server
   response never instructs a client to disable, lock or downgrade a local
   feature (ADR 0066-A decision 4). Every refusal is a statement about this
   endpoint only.
3. **Upload precedes reference.** Every artifact and every snapshot is durable
   before an envelope or epoch references it (ADR 0074 decision 1).
4. **Compare-and-swap on the log head, then retry safely.** The head sequence is
   the only write serialization point. Every write is safe to retry
   (section 5).
5. **Everything the client must trust is verified by the client.** A server
   response is evidence, not authority: the client re-verifies signatures, AEAD
   tags, commitments and chain rules (`vault-format.md` C.4, C.6, C.9).

## 2. Conventions

### 2.1 Transport

- All requests use HTTPS (TLS 1.2 or later). A client MUST NOT send a request,
  or a credential, over plaintext HTTP.
- Paths in this document are relative to the **base URL**: the endpoint the
  client is configured for. The base URL has no query string and no trailing
  slash.
- `{vault_id}`, `{device_id}` and `{snapshot_id}` are UUIDs in lowercase,
  hyphenated text. `{artifact_id}` is a `sync_artifact_id` as 64 lowercase hex
  characters. `{part_number}` is an unsigned decimal in the syntax of section 2.2
  (`0|[1-9][0-9]*`), from `1` to `10000`. Path forms are checked by routing before
  the credential is read; a malformed path is `MalformedRequest`.
- Request and response bodies are `application/json` (UTF-8) except where an
  operation says `application/octet-stream`, which carries one container exactly
  as defined in `vault-format.md` (`DFSY` for envelopes and snapshots, `DFSA`
  for artifacts), with no framing added.

### 2.2 Encodings in JSON

| Kind | JSON form |
| --- | --- |
| 64-bit integer (`seq`, `base_seq`, `key_epoch`, `position`, `version`, `issued_at`, byte counts above 32 bits) | decimal string, syntax `-?(0\|[1-9][0-9]*)`; never a JSON number (the rule of `vault-format.md` C.1.2) |
| smaller integer (`protocol_version`, `suite_id`, status counts) | JSON number |
| 32-byte digest, fingerprint, record id, artifact id | 64 lowercase hex characters |
| signed record, envelope container, opaque `result_ref`, signature | base64url without padding ([RFC 4648 section 5](https://www.rfc-editor.org/rfc/rfc4648#section-5)) |
| UUID | lowercase hyphenated text |
| timestamp chosen by the server | [RFC 3339](https://www.rfc-editor.org/rfc/rfc3339) UTC with a `Z` suffix |

A server MUST reject a request body with an unknown member, a duplicate member,
or a value outside these forms with `MalformedRequest`. A client MUST ignore
unknown members in a **response** (section 11 explains which additions may
appear within one protocol version), except the members this document defines
and requires a version-1 client to process: `voids` and `void_claims`
(sections 4.7 and 4.11).

### 2.3 Versioning

Every request carries the header `DohFlow-Protocol: 1`, an integer naming the
single protocol version the client is speaking. The server answers in that
version and echoes `DohFlow-Protocol` in every response. If it does not support
the version it returns `UnsupportedProtocol` (HTTP 400) with
`details.supported`, the list of versions it does support. The version is
negotiated per request, so an old and a new client can use one server. A request
without the header is `UnsupportedProtocol`. `getInfo` is the only operation
that is answered in every version.

The format versions inside objects (`protocol_version` in the clear header,
`container_version`, `suite_id`) are independent and are checked as
`vault-format.md` C.13 says; an object the server cannot parse is
`MalformedObject`, and a suite it does not implement is `UnsupportedSuite`.

### 2.4 Authentication

Requests carry `Authorization: Bearer <credential>`. There are two kinds of
credential, and a server MAY implement either with any opaque token format:

- An **account credential** proves that the caller may create and enroll
  devices for a vault on this server. How a user obtains one (sign-in, billing,
  entitlement) is outside this protocol; the DohFlow desktop client obtains it
  through a system-browser sign-in (ADR 0004 section 4). It is
  accepted only by `claimCredential`, and, **before the vault's genesis is
  installed**, by `stageSnapshot`, `stageSnapshotClosure` (4.14), the four
  operations that stage in parts (4.13: `beginSnapshotUpload`, `putSnapshotPart`,
  `completeSnapshotUpload` and `abortSnapshotUpload`), `putArtifact` and
  `installEpoch` (`reason 1`), because no device credential exists yet. (A vault
  whose first snapshot has more than 4096 artifacts is founded by staging its closure
  under the account credential, so that operation has to be on this list.)
- A **device credential** is scoped to one device of one vault and is returned
  by `claimCredential`. It is accepted by every other authenticated operation,
  and by all of them once the genesis is installed.
  The server binds it to the device's `device_id` and `device_key_fingerprint`
  in the membership chain. Revoking the device revokes the credential in the
  same transaction (section 6.3).

Before a vault's genesis is installed, the first request that names its
`vault_id` (a staged snapshot, an artifact upload or the genesis itself) binds
that `vault_id` to the calling account. Every later pre-genesis request, and the
genesis, MUST come from the same account; a different account gets `Forbidden`.
How the server records the binding is its own business.

A missing, malformed or unknown credential is `Unauthorized` (HTTP 401 with a
`WWW-Authenticate: Bearer` header). A credential for a revoked device is
`DeviceRevoked` (403). A credential not permitted to act on this vault is
`Forbidden` (403). A credential that no longer permits writes (for example a
lapsed entitlement) is `WritesSuspended` (403) on a write and has no effect on
reads during the server's grace period. Neither tells the client to change any
local behavior.

### 2.5 Errors

Every non-2xx response has the body

```json
{ "error": { "code": "HeadMismatch", "message": "human text", "details": {} } }
```

`code` is a name from the registry in section 8 and is the only part a client
interprets. `message` is for display and logs and MUST NOT be parsed.
`details` is an object defined by the operation, or empty. A client that meets an
unknown `code` treats it by HTTP status class (4xx: do not retry unchanged; 5xx:
retry with backoff).

A failure mutates nothing, with two stated exceptions: an upload that completed
before the failing step stays staged for the grace period of section 7.3, and a
`completeSnapshotUpload` whose assembled container does not match what was declared
discards the upload it was completing (section 4.13).

### 2.6 Retry, rate and availability

- Every operation is safe to retry byte for byte. `PUT` is idempotent by
  address. `pushEnvelopes` and `installEpoch` are idempotent by the rules of
  sections 5 and 6.
- A server MAY answer `RateLimited` (429) with `Retry-After` (seconds). A
  client waits at least that long, then retries unchanged.
- A server MAY answer `ServiceUnavailable` (503) with `Retry-After`. Clients
  back off exponentially with jitter when there is no `Retry-After`.
- A client MUST treat a lost response (timeout, connection reset) as unknown
  outcome and retry the same bytes, never a re-encrypted variant.

## 3. Objects the server stores

For each `vault_id` a server keeps, at minimum, the following. Everything is
opaque to the server except the clear fields named.

| Item | Contents |
| --- | --- |
| Log | Append-only. An **envelope entry** holds the sealed envelope container, its artifact-manifest sidecar and the assigned `seq`. An **epoch entry** marks the log head `h` at which an epoch record was installed. `seq` starts at **1** and increases by exactly 1; **it is never 0**. The genesis epoch has position `0` (`vault-format.md` C.7). |
| Head | The highest assigned `seq` (`0` for an empty log). |
| Accepted-envelope index | One record per accepted envelope: `vault-format.md` C.10. Authoritative and recovery-durable: it survives tail truncation, snapshot bootstrap and restart, and has no retry horizon or expiry. |
| Membership chain | Every signed membership record, by `version`. |
| Epoch chain | Every signed epoch record, in install order, with its position. |
| Snapshots | The latest and the previous published snapshot container, with their sidecars. |
| Artifacts | Immutable `DFSA` containers by `(key_epoch, sync_artifact_id)`, with receipts and pins. |
| Enrollments | Per device: `device_id`, fingerprint, credential state, last acked `seq`. |
| Nonce record | The first `(key_epoch, device_key_fingerprint, object_type, nonce)` tuple seen, for the duplicate-detection backstop (section 5.4). |
| Void entries | Member-signed void records (`vault-format.md` C.10 "Void records"), each in the same log as the envelopes and each holding the next `seq` (section 4.11). Every distinct void is kept. |
| Void claims | In the accepted-envelope index, against the position a void names: `void claimed by <void id>`. Permanent, like the index (section 5.3). |
| Halt record | Per enrolled device, at most one position: where the device last reported being stopped (section 4.12). |

A snapshot publication does **not** consume a `seq`: it is accepted at head `h`,
carries `base_seq = h`, and the next envelope is `h + 1`.

## 4. Operations

The table below is the complete interface of protocol version 1. `auth` is the
credential kind (`none`, `account`, `device`, or `account/device` where the
operation says which applies). `success` is the success status; two values
(`201/200`) mean created, or already present and identical. `errors` lists the
operation-specific error codes (HTTP statuses are in section 8); every
authenticated operation can also return the common errors of section 8.2.

<!-- table:operations -->

| op | method | path | auth | success | errors |
| --- | --- | --- | --- | --- | --- |
| `getInfo` | GET | `/info` | none | 200 | - |
| `getStatus` | GET | `/vaults/{vault_id}/status` | device | 200 | VaultNotFound |
| `getChain` | GET | `/vaults/{vault_id}/chain` | device | 200 | VaultNotFound |
| `installEpoch` | POST | `/vaults/{vault_id}/epochs` | account/device | 201/200 | VaultExists, VaultNotFound, ReplicaHalted, WritesSuspended, MembershipChainInvalid, EpochRecordInvalid, EpochRecordMissing, CutoverHeadNotRevoke, GrantHeadNotAdd, BadEpochSequence, KeyCommitmentMismatch, StaleMembershipHead, HeadMismatch, SnapshotNotFound, ArtifactMissing, QuotaExceeded, MalformedObject, UnsupportedSuite |
| `claimCredential` | POST | `/vaults/{vault_id}/devices/{device_id}/credential` | account | 200 | VaultNotFound, DeviceNotFound, ClaimProofInvalid |
| `pushEnvelopes` | POST | `/vaults/{vault_id}/envelopes` | device | 200 | VaultNotFound, WritesSuspended, MalformedObject, UnsupportedSuite, ProtocolFork, EpochRejected, ArtifactMissing, NonceTupleReused, HeadMismatch, QuotaExceeded, PayloadTooLarge |
| `pullEnvelopes` | GET | `/vaults/{vault_id}/envelopes` | device | 200 | VaultNotFound, TailTruncated |
| `ack` | POST | `/vaults/{vault_id}/acks` | device | 200 | VaultNotFound, AckBeyondHead |
| `stageSnapshot` | PUT | `/vaults/{vault_id}/snapshots/{snapshot_id}` | account/device | 201/200 | VaultNotFound, ReplicaHalted, WritesSuspended, PayloadTooLarge, MalformedObject, UnsupportedSuite, EpochRejected, NonceTupleReused, ObjectConflict, SnapshotTooLarge, QuotaExceeded |
| `publishSnapshot` | POST | `/vaults/{vault_id}/snapshots/{snapshot_id}/publish` | device | 200 | VaultNotFound, ReplicaHalted, WritesSuspended, SnapshotNotFound, MalformedObject, EpochRejected, HeadMismatch, ArtifactMissing, QuotaExceeded |
| `getLatestSnapshot` | GET | `/vaults/{vault_id}/snapshots/latest` | device | 200 | VaultNotFound, SnapshotNotFound |
| `getSnapshot` | GET | `/vaults/{vault_id}/snapshots/{snapshot_id}` | device | 200 | VaultNotFound, SnapshotNotFound |
| `stageSnapshotClosure` | PUT | `/vaults/{vault_id}/snapshots/{snapshot_id}/closure` | account/device | 201/200 | VaultNotFound, ReplicaHalted, WritesSuspended, SnapshotNotFound, SnapshotTooLarge, MalformedObject, QuotaExceeded |
| `getSnapshotClosure` | GET | `/vaults/{vault_id}/snapshots/{snapshot_id}/closure` | device | 200 | VaultNotFound, SnapshotNotFound |
| `putArtifact` | PUT | `/vaults/{vault_id}/artifacts/{artifact_id}` | account/device | 201/200 | VaultNotFound, WritesSuspended, MalformedObject, EpochRejected, ObjectConflict, PayloadTooLarge, QuotaExceeded |
| `getArtifact` | GET | `/vaults/{vault_id}/artifacts/{artifact_id}` | device | 200 | VaultNotFound, ArtifactNotFound |
| `getArtifactStatus` | POST | `/vaults/{vault_id}/artifacts/status` | device | 200 | VaultNotFound |
| `appendVoid` | POST | `/vaults/{vault_id}/voids` | device | 201/200 | VaultNotFound, WritesSuspended, MalformedObject, UnsupportedSuite, VoidSignerNotAuthorized, VoidSignatureInvalid, VoidTargetMismatch, QuotaExceeded |
| `reportHalt` | PUT | `/vaults/{vault_id}/devices/{device_id}/halt` | device | 200 | VaultNotFound |
| `beginSnapshotUpload` | POST | `/vaults/{vault_id}/snapshots/{snapshot_id}/uploads` | account/device | 201/200 | VaultNotFound, ReplicaHalted, WritesSuspended, SnapshotTooLarge, PayloadTooLarge, ObjectConflict, QuotaExceeded |
| `putSnapshotPart` | PUT | `/vaults/{vault_id}/snapshots/{snapshot_id}/uploads/parts/{part_number}` | account/device | 201/200 | VaultNotFound, ReplicaHalted, WritesSuspended, UploadNotFound, PartOutOfSequence, ObjectConflict, MalformedObject, UnsupportedSuite, EpochRejected, PayloadTooLarge |
| `completeSnapshotUpload` | POST | `/vaults/{vault_id}/snapshots/{snapshot_id}/uploads/complete` | account/device | 201/200 | VaultNotFound, ReplicaHalted, WritesSuspended, UploadNotFound, UploadIncomplete, MalformedObject, EpochRejected |
| `abortSnapshotUpload` | DELETE | `/vaults/{vault_id}/snapshots/{snapshot_id}/uploads` | account/device | 200 | VaultNotFound |

<!-- /table:operations -->

The sections below give each operation's request, response and rules. `vault_id`
in a path MUST equal the `vault_id` in the clear header of every object in the
body; a mismatch is `MalformedObject`.

### 4.1 `getInfo`

`GET /info`, no credential. Returns what this server supports so a client can
decide whether it can use it, before it sends any vault data:

```json
{
  "protocol_versions": [1],
  "suites": [1],
  "limits": {
    "max_artifact_bytes": "67108864",
    "push_batch_max_envelopes": 64,
    "pull_page_max_entries": 500,
    "pull_page_max_bytes": "33554432",
    "orphan_grace_seconds": 604800,
    "superseded_root_window_seconds": 2592000,
    "snapshot_single_request_max_bytes": "67108864",
    "snapshot_part_max_bytes": "67108864",
    "snapshot_upload_expiry_seconds": 604800
  }
}
```

The values shown are examples. The members of `limits` are the **operator
limits** of section 7.2: each server advertises its own, and a client reads them
rather than assuming them. `protocol_versions` and `suites` list the versions
the server implements. The last three members are optional (section 7.2): a server
that does not send `snapshot_single_request_max_bytes` accepts a snapshot of any
size within section 7.1 through `stageSnapshot`. `getInfo` needs no credential and
carries no vault data.

### 4.2 `getStatus`

`GET /vaults/{vault_id}/status`. Returns the server's view of the vault:

```json
{
  "head_seq": "1250",
  "floor_seq": "1000",
  "key_epoch": "3",
  "membership_head": "<64 hex>",
  "membership_version": "5",
  "epoch_head": "<64 hex>",
  "latest_snapshot": { "snapshot_id": "<uuid>", "base_seq": "1000", "key_epoch": "3" },
  "devices": [
    { "device_id": "<uuid>", "device_key_fingerprint": "<64 hex>", "last_ack_seq": "1240", "revoked": false }
  ],
  "fork_count": 0,
  "usage": { "ciphertext_bytes": "123456789", "ciphertext_bytes_limit": null }
}
```

`floor_seq` is the highest `seq` that has been truncated: envelopes with
`seq > floor_seq` are available. `fork_count` counts the `ProtocolFork`
rejections the server has issued, for diagnostics. `usage` reports quota use and
MAY carry a `null` limit. `ciphertext_bytes_limit` is whatever the server's
terms set; the protocol assigns it no value.

### 4.3 `getChain`

`GET /vaults/{vault_id}/chain?after_version=<n>&after_ordinal=<m>`. Returns the
membership and epoch records the caller has not seen, in install order, so a
client can verify them under `vault-format.md` C.6 and C.7 (the chain rules, the
anchor rule and the backward catch-up walk are the client's):

```json
{
  "membership_head": "<64 hex id of the newest membership record>",
  "membership": [ { "version": "5", "record": "<base64url signed record>" } ],
  "epochs": [
    { "ordinal": "4", "position": "1000", "key_epoch": "3", "record": "<base64url signed record>" }
  ]
}
```

`after_version` defaults to `0` (all membership records) and `after_ordinal` to
`0` (all epoch records). An epoch record's `ordinal` is its install order (1 for
the genesis record) and `position` is the log head at which it was installed
(C.7). Epoch records of `reason 3` (grants) share the previous record's
`key_epoch` and have their own `ordinal`. The server returns records exactly as
submitted; it cannot construct or alter one.

### 4.4 `installEpoch`

`POST /vaults/{vault_id}/epochs`. The only operation that changes the membership
chain or the key epoch. One call is one atomic transaction that installs one
epoch record and, when it carries one, the membership record that record names,
plus the snapshot published with it. It covers all four reasons of
`vault-format.md` C.6:

```json
{
  "base_seq": "1000",
  "membership_record": "<base64url signed record> | null",
  "epoch_record": "<base64url signed record>",
  "snapshot_id": "<uuid> | null",
  "artifact_manifest": ["<64 hex>"]
}
```

| `reason` of the epoch record | Meaning | `membership_record` | `snapshot_id` | Credential |
| --- | --- | --- | --- | --- |
| `1` genesis | the first device of a vault | required: the genesis `add` | required | account |
| `2` revocation cutover | revoke a device, new key epoch | required: the `revoke` | required | device (an authorized remaining member) |
| `3` grant | add a device, no rotation | required: the `add` | optional | device (an authorized member) |
| `4` forced rotation | new key epoch, no membership change | `null` | required | device (an authorized member) |

`base_seq` is the log head the client believes is current. For a genesis it is
`"0"`. When a snapshot is present, `base_seq` MUST equal the snapshot's header
`base_seq`. It is `null` only for a grant without a snapshot (section 6.1,
check 7). `artifact_manifest` is the snapshot's full closure sidecar: the
ordered list of `sync_artifact_id` values whose commitment is the snapshot
header's `artifact_manifest_commitment` (`vault-format.md` C.3); it is required
when `snapshot_id` is present and empty otherwise. The snapshot MUST have been
staged first with `stageSnapshot`. **A closure of more than 4096 ids is not sent
here** (`max_artifact_manifest_ids`): it is staged with `stageSnapshotClosure`
(section 4.14) before this call, and this request then carries an empty
`artifact_manifest` (a non-empty one beside a staged closure is `MalformedRequest`).
The closure the server checks is the staged one if the snapshot has one and the list
in the request otherwise, and the check is the same exact one in both cases (section 6.1
step 8).

On success the server has, in one transaction: installed the membership record
(when present) and the epoch record naming it; published the snapshot (when
present) at `base_seq`; durably recorded every closure pin; for `reason 2`,
revoked the revoked device's enrollment and credential; and, for `reason 2` and
`4`, begun rejecting writes under any older epoch. Response `201`:

```json
{
  "key_epoch": "4",
  "ordinal": "6",
  "position": "1000",
  "membership_head": "<64 hex>",
  "epoch_record_id": "<64 hex>"
}
```

An exact retry (the same epoch record id and, if present, the same membership
record id, already installed) returns `200` with the original values; this
makes a lost response safe. Any different record at an occupied position is a
rejection (section 6), never an overwrite.

`ArtifactMissing` from `installEpoch` carries `{ "key_epoch": "4", "missing":
["<64 hex>"] }`, the closure ids that lack a durable receipt: all of them when
there are at most 4096, and otherwise the first 4096 in ascending order, so a
client uploads those and calls again (a closure can hold 2^20 ids, section 4.14). The service
performs every check of section 6.1 that it can perform on public data. It cannot unseal the keys, so it relies on the client to verify them
(`vault-format.md` C.6, "Sealing the key set").

### 4.5 `claimCredential`

`POST /vaults/{vault_id}/devices/{device_id}/credential`, authenticated by the
**account credential**. After the membership chain authorizes a device (the
genesis, or a grant installed by an existing member), the new device obtains its
own device credential by proving possession of its signing key:

```json
{ "issued_at": "<i64 microseconds since the Unix epoch>", "proof": "<base64url Ed25519 signature>" }
```

`proof` is an Ed25519 signature, verified strictly under the device's
`ed25519_pk` from the membership chain, over the CBE1 body

```
label "dohflow/sync-credential-claim/v1" ‖ uuid vault_id ‖ uuid device_id ‖
fingerprint device_key_fingerprint ‖ i64 issued_at
```

(`vault-format.md` C.1.1). The body and the strict check are one implementation,
`sync_core::claim` (`claim_body` and `verify_claim`), which the client calls to build
the proof and the server calls to check it. `issued_at` is microseconds since the Unix epoch
(UTC); the server rejects a value more than `claim_skew_seconds` (section 7.1)
from its own clock with `ClaimProofInvalid`. A second successful claim for the
same device issues a new credential and invalidates the previous one, which lets
a device whose response was lost recover. The response is

```json
{ "device_id": "<uuid>", "credential": "<opaque token>", "expires_at": "2030-01-01T00:00:00Z" }
```

where `expires_at` is `null` when the credential does not expire. A device that is
not currently authorized in the chain is `DeviceNotFound` (a revoked device is
never re-enrolled; `vault-format.md` C.6, chain rule 4).

### 4.6 `pushEnvelopes`

`POST /vaults/{vault_id}/envelopes`:

```json
{
  "base_seq": "1000",
  "envelopes": [
    { "envelope": "<base64url DFSY container, object_type 1>", "artifact_manifest": ["<64 hex>"] }
  ]
}
```

`base_seq` is the log head of the base image the client rebased onto (ADR 0074
decision 2). Every envelope's clear-header `base_seq` MUST equal the request's
`base_seq`; a replayed outbox is built on one base image, so they agree. The
number of envelopes is at most the server's `push_batch_max_envelopes`. Each
`artifact_manifest` is that envelope's delta-manifest sidecar: the ordered ids
whose commitment is the header's `artifact_manifest_commitment`, and an empty
list when the header field is absent.

The decision procedure is section 5. On success, `200`:

```json
{
  "head_seq": "1003",
  "results": [
    { "command_id": "<uuid>", "seq": "1001", "result_ref": "<base64url> | null", "duplicate": false }
  ]
}
```

`results` has one entry per submitted envelope, in order. `duplicate` is `true`
for an envelope that the accepted-envelope index already held (an exact retry);
its `seq` and `result_ref` are those of the original acceptance.

**Which envelope failed.** A push is one unit, so these errors reject the whole
request. Each carries `details` that name the first offending envelope:

```json
{ "index": 2, "command_id": "<uuid> | null", "accepted_seq": "1002 | null" }
```

`index` is the zero-based position in the request's `envelopes`; `command_id` is
read from the clear header (`null` when the container is unparsable);
`accepted_seq` is, for `ProtocolFork`, the `seq` of the accepted envelope it
conflicts with (`null` otherwise). This applies to `MalformedObject`,
`ProtocolFork`, `EpochRejected` and `NonceTupleReused`. `ArtifactMissing` carries
`{ "index": 2, "key_epoch": "3", "missing": ["<64 hex>"] }`, listing every id of
that envelope's sidecar that lacks a durable receipt (at most 4096). After
`ProtocolFork` the client halts the named command and keeps both envelopes for
diagnosis; it may resubmit the others without it. After `ArtifactMissing` it
uploads the listed ids and resubmits unchanged.

On `HeadMismatch` (HTTP 409), `details` carries what the client needs to rebase
without another round trip:

```json
{
  "head_seq": "1005",
  "floor_seq": "1000",
  "accepted": [ { "command_id": "<uuid>", "seq": "1002", "result_ref": null } ],
  "entries": [ "<pull entries as in pullEnvelopes, seq > base_seq>" ],
  "has_more": false,
  "snapshot_required": false
}
```

`accepted` lists any submitted envelope the index already held. `entries` are the
log entries since the request's `base_seq`, bounded as a pull page. If
`base_seq` is below `floor_seq` the tail is gone: `entries` is empty,
`snapshot_required` is `true`, and the client bootstraps from the latest
snapshot and rebases (ADR 0074 decision 2).

### 4.7 `pullEnvelopes`

`GET /vaults/{vault_id}/envelopes?after_seq=<n>&limit=<k>`. `after_seq` is
required. `limit` is optional and is capped by the server's page limits. The
response:

```json
{
  "head_seq": "1250",
  "floor_seq": "1000",
  "has_more": true,
  "entries": [
    { "type": "envelope", "seq": "1001", "envelope": "<base64url>", "artifact_manifest": ["<64 hex>"] },
    { "type": "epoch", "position": "1001", "ordinal": "5", "epoch_record_id": "<64 hex>" }
  ]
}
```

Entries are in log order. An `epoch` entry sits immediately after the envelope
whose `seq` equals its `position` and before `position + 1`: that is how a client
learns an epoch's position `h` (`vault-format.md` C.7). A client treats an
envelope past the last authenticated epoch position as pending until it has
fetched that epoch's snapshot and checked `base_seq` against `position`. A page
holds at least one log object (an envelope, an epoch entry or a void, section
4.11) whenever any exists past `after_seq`, so a client always makes progress. If `after_seq` is below `floor_seq`, the server returns
`TailTruncated` (HTTP 410) with `details.latest_snapshot` as in `getStatus`; the
client bootstraps from that snapshot.

A page can also carry the optional member `voids`: the void records that sit in
the same log (section 4.11). A log without a void has the response shown above
and no `voids`. **A version-1 client MUST process `voids`, and `void_claims` on
push results (section 4.11).** A void takes a `seq`, so without `voids` a client
would see gaps in the envelope sequence, and a page can hold only voids: an empty
`entries` does not mean the log is exhausted, `has_more` and `head_seq` do. A
client that finds a `seq` between `floor_seq + 1` and `head_seq` that is neither
an envelope in `entries` nor a void in `voids` has an incomplete page and treats
it as a failed pull, never as a gap to skip.

### 4.8 `ack`

`POST /vaults/{vault_id}/acks` with `{ "seq": "1240" }`. The device states that it
has durably applied the log through `seq`. The server keeps the maximum per
device; a lower value is accepted and ignored. `seq` above the head is
`AckBeyondHead`. Response `200`: `{ "last_ack_seq": "1240" }`. Acks drive tail
truncation (section 7.3) and nothing else.

### 4.9 Snapshots

A snapshot is a type `2` container (`vault-format.md` C.4, C.8). Its publication
is two steps, to keep a large upload out of the compare-and-swap:

1. `stageSnapshot`: `PUT /vaults/{vault_id}/snapshots/{snapshot_id}` with the
   container as `application/octet-stream`. The server parses the clear header
   only. It checks that `vault_id`, `object_type` (`2`) and `object_id` (equal to
   `{snapshot_id}`) agree, that `device_id` and the fingerprint belong to the
   credential's device (before the genesis exists there is no device to check: the
   server instead requires `key_epoch` `1`, and `installEpoch` later requires the
   header's `device_id` to be the genesis device), that `key_epoch` is the epoch
   in force or the next one,
   that the container is well formed, and that the plaintext bound is respected
   (`SnapshotTooLarge`, HTTP 413). It stores the bytes as **staged**. Response
   `201` (or `200` for an exact retry): `{ "snapshot_id": "<uuid>", "size": "n" }`.
   A different byte string for the same `snapshot_id` is `ObjectConflict`. A
   container larger than the server's `snapshot_single_request_max_bytes` (section
   7.2) is `PayloadTooLarge` here, with `details.single_request_max_bytes`, and is
   staged through the upload operations of section 4.13 instead; the result, a
   staged snapshot, is the same.

   **The order of decision.** The first failing step answers, and none of steps 1
   to 3 changes anything. The operations of 4.13 and 4.14 that also stage a
   snapshot follow the same shape.
   1. **Authenticate from the credential alone:** `Unauthorized`, `DeviceRevoked`,
      and any `Forbidden` that the credential itself decides.
   2. **The checks that need no vault state,** made from the headers and the first
      bytes of the body: the media type and the declared length (`MalformedRequest`),
      the clear header (`MalformedObject`, `UnsupportedSuite`), the plaintext bound
      (`SnapshotTooLarge`) and the single-request limit (`PayloadTooLarge`). A
      request that fails here has not touched the vault, and does not bind a
      `vault_id` to an account (section 2.4).
   3. **The authorization that needs the vault:** `Forbidden` for an account
      credential once the genesis is installed, for an account that is not the
      household's primary member, and for the account of another household; then
      **the halt gate** of section 4.12 (`ReplicaHalted`).
   4. **The rest,** in the order the checks above describe them: that the header
      names the credential's device and an epoch in force or the next
      (`MalformedObject`, `EpochRejected`), writes (`WritesSuspended`), the nonce
      record and the snapshot id (`NonceTupleReused`, `ObjectConflict`), the quota
      (`QuotaExceeded`), and the bytes as stored.
2. `publishSnapshot`: `POST /vaults/{vault_id}/snapshots/{snapshot_id}/publish`:

   ```json
   { "base_seq": "1000", "artifact_manifest": ["<64 hex>"] }
   ```

   The server accepts the snapshot if and only if `base_seq` equals the current
   head **and** equals the snapshot's header `base_seq`, its `key_epoch` is the
   epoch in force, its sidecar matches the header's commitment, and every id in
   the closure has a durable receipt and a recorded pin, all in one transaction
   (ADR 0074 decision 1). Otherwise it changes nothing. Publication is idempotent
   by the immutable `snapshot_id`: publishing an already published snapshot
   returns the original result. Response `200`:
   `{ "snapshot_id": "<uuid>", "base_seq": "1000", "key_epoch": "3" }`.

   As for `installEpoch` (section 4.4), a closure of more than 4096 ids is staged
   first with `stageSnapshotClosure` (section 4.14) and this request then carries an
   empty `artifact_manifest`.

   A snapshot that starts a new key epoch (genesis, cutover, forced rotation) is
   published only by `installEpoch`, never by `publishSnapshot`
   (`EpochRejected`).

`getLatestSnapshot` (`GET /vaults/{vault_id}/snapshots/latest`) returns the
metadata of the newest published snapshot, `{ "snapshot_id", "base_seq",
"key_epoch", "size", "artifact_count", "artifact_manifest": [...] }`. `artifact_count`
is the number of ids in the closure, a decimal string. `artifact_manifest` lists them
(ascending) when there are at most 4096 and is `null` otherwise, and the client then
reads the closure with `getSnapshotClosure` (section 4.14). `getSnapshot` returns a
retained snapshot's container bytes (`application/octet-stream`); the client
verifies the AEAD and the sidecar's commitment itself. A snapshot that is no
longer retained is `SnapshotNotFound`. Staged snapshots are not returned.

### 4.10 Artifacts

An artifact is an immutable `DFSA` container (`vault-format.md` C.9).

- `putArtifact`: `PUT /vaults/{vault_id}/artifacts/{artifact_id}?key_epoch=<e>`
  with the container as `application/octet-stream`. `key_epoch` MUST be the
  epoch in force or the next one: a client re-addressing live artifacts for a
  cutover uploads them under epoch `e + 1` before the cutover is installed
  (`vault-format.md` C.7). The server cannot read the object, so it checks only
  the container framing and the size, stores the first bytes it receives for
  the address (before the genesis exists, only under `key_epoch` `1`), and answers `201` with the receipt below. An exact retry returns
  `200` with the same receipt. A different byte string for the same
  `(key_epoch, artifact_id)` is `ObjectConflict`.
- The **receipt** binds the address to the stored ciphertext
  (`vault-format.md` C.9, `ArtifactReceiptV1`); v1 receipts carry no service
  signature, so a client re-verifies the AEAD whenever it fetches:

  ```json
  { "artifact_id": "<64 hex>", "key_epoch": "3", "ciphertext_length": "4096", "ciphertext_sha256": "<64 hex>" }
  ```

- `getArtifact`: `GET /vaults/{vault_id}/artifacts/{artifact_id}?key_epoch=<e>`
  returns the container bytes, or `ArtifactNotFound`.
- `getArtifactStatus`: `POST /vaults/{vault_id}/artifacts/status` with
  `{ "key_epoch": "3", "ids": ["<64 hex>"] }` (at most 4096 ids) returns
  `{ "artifacts": [ { "artifact_id", "present", "ciphertext_length",
  "ciphertext_sha256", "pinned" } ] }` in request order. A client may evict a
  local artifact cache only after it holds both a durable receipt and a
  confirmed pin (ADR 0074 decision 1).

A **pin** is the server's record that a retained snapshot or an accepted
envelope references an artifact. Pins are created inside the transaction that
accepts the reference; the client never creates one directly.

### 4.11 Voids: `appendVoid` and the `voids` member

*Added to protocol version 1 on 2026-11-02 under section 11.1
(`personal-cfo-g3m.33`; section 11.4 answers each limit of 11.1).*

A **void record** (`vault-format.md` C.10 "Void records") is how a vault moves
past a log position that every honest replica rejects (ADR 0074 Addendum D). The
server only orders it. It checks the record's structure, that the signer is a
member, and that the record names the envelope the server holds at that position.
It never decides whether that envelope is invalid, and it never says that a
position *is* voided: that is each replica's own verdict (Addendum D rule 3 and
Addendum E). A void is an ordinary log object. It takes the next `seq`: appended
at `V`, it makes `V` the head, and a replica that honors it skips the command
envelopes at positions `seq` through `V - 1` (C.10 "Range").

`appendVoid`: `POST /vaults/{vault_id}/voids`, device credential:

```json
{ "record": "<base64url signed void record>" }
```

The record is exactly the bytes of C.10 (`u32 body_len ‖ body ‖ 64-byte
signature`). `vault_id` in the body MUST equal the path. Response `201`, or `200`
for an exact retry:

```json
{ "void_id": "<64 hex>", "position": "1007", "head_seq": "1007" }
```

`void_id` is `SHA-256(body)`. `position` is the `seq` the void took (`V`); a
retry returns the original. For each request the server decides in this order,
and the first failing step answers:

1. **Authenticate** the device credential (`Unauthorized`, `DeviceRevoked`,
   `Forbidden`). The signer of the record need not be the credential's device.
2. **Parse** the record: framing, body, a protocol version inside the body other
   than `1`, or another vault than the path's is `MalformedObject`; a suite other
   than `1` is `UnsupportedSuite`.
3. **Exact retry.** If a void with this `void_id` is already in the log, answer
   `200` with its `position`, whatever the signer's state is now.
4. **Writes permitted?** `WritesSuspended`.
5. **Signer.** The signer MUST be an authorized, unrevoked member at the
   membership head in force at the position the void would take (`V = head + 1`),
   verified under the key the chain holds for `signer_fingerprint`, never a key
   carried in the record (`VoidSignerNotAuthorized`), and the Ed25519 signature
   MUST verify strictly (`VoidSignatureInvalid`). This is the check
   `verify_void` of `sync-core` makes, and a server SHOULD call it rather than
   reimplement it, or it refuses voids that replicas would honor. `issued_at` is
   audit only and is never compared with a clock.
6. **Target.** The body's `seq` MUST be a command-envelope position in the
   accepted-envelope index, and the body's `command_id` and `envelope_digest`
   MUST equal that record's (`VoidTargetMismatch`). The index is never truncated
   (section 5.3), so a position below `floor_seq` still answers.
7. **Quota** (`QuotaExceeded`).
8. **Append**, in one transaction: assign `V = head + 1`, make `V` the head, store
   the record, and record `void claimed by <void_id>` against the named position.

**Every distinct void is kept.** Two voids with different bodies are two entries
even when they name the same `seq`. The server dedupes only an exact retry
(step 3), and it never refuses a void because an earlier void's range would cover
its position. A member can append a void for a *valid* envelope; replicas ignore
it, and it must not block a later genuine void. A server MAY answer `RateLimited`,
because a member can append voids without limit.

**Void claims on a replay.** An exact replay of an accepted envelope returns the
original `seq` and `result_ref` and, when voids name its position, the optional
member `void_claims`: the `void_id` of each, in the order they were appended. It
appears on an entry of `results` in `pushEnvelopes` (`duplicate` is `true`) and on
an entry of `accepted` in the `HeadMismatch` details, and is omitted when there is
none. A claim is a claim, not a verdict; the client decides from its own verdict
whether to roll back and re-issue (Addendum D rule 6), and a position's
`command_id` and idempotency-key tag are never reusable. A version-1 client MUST
process `void_claims` where it appears (section 4.7).

**Voids in a pull.** `pullEnvelopes`, and the `HeadMismatch` details, carry the
optional member `voids`, in log order, present only when the page holds a void:

```json
"voids": [ { "position": "1007", "void_id": "<64 hex>", "record": "<base64url signed void record>" } ]
```

- A void's `position` is the `seq` it occupies. Envelopes and voids share one
  sequence of `seq` values: across the pages of a log, every `seq` from
  `floor_seq + 1` to `head_seq` is an envelope in `entries` or a void in `voids`,
  once. (A void takes a `seq`; an epoch entry does not, section 3.)
- `entries` and `voids` together are one page, taken in `seq` order. The page
  bounds of section 7.2 count a void as one entry. A page holds at least one log
  object (an envelope, an epoch entry or a void) whenever any exists past
  `after_seq`, and `has_more` is `true` exactly when log objects remain past the
  last one in the page. The next `after_seq` is the highest envelope `seq` or void
  `position` in the page.
- An `epoch` entry whose `position` is the `seq` of a void sits after that void,
  as it does after an envelope.
- A client that receives `voids` MUST verify each record under
  `vault-format.md` C.10 and offer it to its honor rule, in position order; it
  applies ADR 0074 Addendum D and Addendum E. The server's order and presence are
  evidence, not a verdict (section 1, principle 5).

A void record and the envelope it names are retained as section 7.3 says.

### 4.12 Halt report: `reportHalt` and `ReplicaHalted`

*Added to protocol version 1 on 2026-11-02 under section 11.1
(`personal-cfo-g3m.33`).*

A replica stopped at a poisoned position publishes no snapshot, performs no
key-epoch cutover and pushes nothing (ADR 0074 Addendum D rule 5). The replica
enforces that itself. `reportHalt` lets it tell the server, so the server can
refuse the same three requests from that device as a backstop, in the way the
nonce backstop of section 5.4 backs the client's counter register.

`reportHalt`: `PUT /vaults/{vault_id}/devices/{device_id}/halt`, device credential:

```json
{ "stopped_at": "1003" }
```

`stopped_at` is the log position (`seq` of the envelope, form of section 2.2,
at least `1`) at which the replica is stopped, or `null` to report that it is no
longer halted. Response `200`: `{ "device_id": "<uuid>", "stopped_at": "1003" }`
(`null` after a clear). For each request the server decides in this order:

1. **Authenticate** (`Unauthorized`, `DeviceRevoked`). `{device_id}` MUST be the
   credential's device, or the request is `Forbidden`: a device reports only its
   own state.
2. **Parse** the body (`MalformedRequest`).
3. **Store.** Replace the device's halt record with `stopped_at`, or remove it for
   `null`. The same report again changes nothing.

The server stores one position per device, and nothing about why. It cannot read
the envelope, so the report is not evidence and the server does not check it
against the head. Reporting needs no write entitlement. The record ends when the
device reports `null` (it honored a void and resumes, or it re-bootstrapped), or
when the device is revoked.

**The halt gate.** A device credential whose device has a recorded halt is refused
by `stageSnapshot`, `stageSnapshotClosure`, `publishSnapshot` and `installEpoch` with `ReplicaHalted`
(HTTP 409), `details: { "stopped_at": "1003" }`. The refusal changes nothing
(section 2.5), applies to every `reason` of `installEpoch`, and is a statement
about that device's own report, never an instruction to change any local feature.
The gate runs after authentication, after the checks that need no vault state and
after the vault-side `Forbidden` of the operation (section 4.9, step 3), and before
the decisions that read the vault's state (the snapshot's presence, an exact retry,
writes, the quota); it is not a step of section 5 or 6, and a vault with no
recorded halt passes it, so those procedures run exactly as written. An account credential
(before the genesis) has no device and passes. `pushEnvelopes` has no gate, since
the replica holds back its own pushes (Addendum D rule 5), and `appendVoid` and
`reportHalt` must work for a halted device, because they are how it leaves. The
gate also applies to `beginSnapshotUpload`, `putSnapshotPart` and
`completeSnapshotUpload` (section 4.13), which stage a snapshot as `stageSnapshot`
does. The gate stops only the reporting device: other members are not affected.

**Operator order.** If the device that poisoned the log should be revoked, the void
comes first and the cutover after it; no cutover can happen while its author is
halted (Addendum D rule 5).

### 4.13 Snapshot upload in parts

*Added to protocol version 1 on 2026-11-03 under section 11.1
(`personal-cfo-g3m.36`; section 11.4 lists it).*

A snapshot container can be as large as `max_snapshot_container_bytes` (section
7.1), and a server's front end may accept less than that in one request. The four
operations below stage the same container in parts. A server MUST accept every
well-formed snapshot within section 7.1 through `stageSnapshot` or through these
operations; a client uses these when the container is larger than the server's
`snapshot_single_request_max_bytes` and MAY use them for any size. Nothing else
changes: the staged snapshot, `publishSnapshot`, `installEpoch` and every check
they make are as in sections 4.4, 4.9 and 6.1, and how a server stores the parts is
its own business (section 1).

Like `stageSnapshot`, all four accept an account credential before the vault's
genesis is installed and a device credential after it, and the first request that
names a `vault_id` binds it to the calling account (section 2.4). **The sizes.**
Every part has the size `part_size` the client chose at the beginning, except the
last, which holds the rest. `part_size` is at least **5,242,880** bytes (5 MiB), at
most the server's `snapshot_part_max_bytes`, and the number of parts,
`ceil(size / part_size)`, is at most **10,000**. (These two numbers are those of the
object stores servers commonly sit on; a protocol that promises them can be
implemented on any of them.)

**`beginSnapshotUpload`:** `POST /vaults/{vault_id}/snapshots/{snapshot_id}/uploads`

```json
{ "size": "1073742137", "sha256": "<64 hex>", "part_size": "67108864" }
```

`size` is the exact length of the whole container in bytes and `sha256` is the
SHA-256 of all of it. Response `201`, or `200` for an exact retry:

```json
{ "snapshot_id": "<uuid>", "state": "uploading", "part_size": "67108864", "part_count": "17", "next_part": "1", "expires_at": "2026-11-10T12:00:00Z" }
```

`state` is `uploading`, or `staged` when a snapshot with this id and this `size` and
`sha256` is already staged (then `next_part` and `expires_at` are `null` and nothing
more is sent). The server decides in this order, and the first failing step answers:

1. **Authenticate from the credential alone** (`Unauthorized`, `DeviceRevoked`, and
   any `Forbidden` the credential itself decides).
2. **Parse** the body (`MalformedRequest`): `size` a positive 64-bit decimal,
   `sha256` 64 lowercase hex characters, `part_size` a 64-bit decimal.
3. **Size.** `size` above `max_snapshot_container_bytes` is `SnapshotTooLarge`.
   A `part_size` above `snapshot_part_max_bytes` is `PayloadTooLarge`; one below
   5,242,880, or giving more than 10,000 parts, is `MalformedRequest`.
4. **The authorization that needs the vault,** as in section 4.9, step 3:
   `Forbidden` for an account credential once the genesis is installed, for an
   account that is not the household's primary member and for the account of
   another household; then **the halt gate** of section 4.12 (`ReplicaHalted`):
   this operation stages a snapshot like `stageSnapshot`, so a device with a
   recorded halt is refused.
5. **Exact retry.** An upload in progress for this `snapshot_id` with the same
   `size`, `sha256` and `part_size` answers `200` with its state; a staged or
   published snapshot with the same `size` and `sha256` answers `200` with
   `state: staged`. Any other snapshot or upload under this id is `ObjectConflict`.
6. **Writes permitted?** `WritesSuspended`.
7. **Quota.** `size` is reserved against the vault's storage at once, so uploads in
   progress cannot together exceed it: `QuotaExceeded`.
8. **Begin.** At most one upload is in progress per `snapshot_id`. The server
   records it with an expiry (section 7.3) and answers `201`.

**`putSnapshotPart`:** `PUT /vaults/{vault_id}/snapshots/{snapshot_id}/uploads/parts/{part_number}`,
the part as `application/octet-stream`. `{part_number}` is a decimal from `1`.
Parts are sent in order, one at a time. Response `201`, or `200` for a retry:
`{ "part_number": "3", "next_part": "4" }` (`null` after the last). The order:

1. **Authenticate from the credential alone** (`Unauthorized`, `DeviceRevoked`, and
   any `Forbidden` the credential itself decides).
2. **Forms.** A body that is not `application/octet-stream`, or that declares no
   length, is `MalformedRequest`. (A `{part_number}` that is not a decimal in `1`
   to `10000` never gets here: routing refuses it first, section 2.1.)
3. **The authorization that needs the vault,** as in section 4.9, step 3:
   `Forbidden`, then **the halt gate** (`ReplicaHalted`).
4. **The upload exists** for this `snapshot_id`: `UploadNotFound` (never begun,
   aborted, expired, discarded after a failed completion, or already completed).
5. **Sequence.** A part already received (`part_number` below `next_part`) with the
   same length and bytes answers `200`; with other bytes it is `ObjectConflict`. A
   `part_number` above `next_part`, or above `part_count`, is `PartOutOfSequence`
   with `details.next_part`.
6. **Writes permitted?** `WritesSuspended`.
7. **Size.** A body longer than `snapshot_part_max_bytes` is `PayloadTooLarge`; one
   whose length is not exactly the size this part must have (`part_size`, or the
   remainder for the last part) is `MalformedObject`.
8. **The header, on part 1 only.** Part 1 holds the whole clear header. The server
   checks it as `stageSnapshot` does (`MalformedObject`, `UnsupportedSuite`,
   `EpochRejected`), and refuses the upload before any more is sent. A refused part
   1 leaves the upload at `next_part` `1`.
9. **Store** the part and advance `next_part`.

**`completeSnapshotUpload`:** `POST /vaults/{vault_id}/snapshots/{snapshot_id}/uploads/complete`
with an empty JSON object. Response `201`, or `200` for a retry, the same as
`stageSnapshot`'s: `{ "snapshot_id": "<uuid>", "size": "n" }`. **Nothing counts as
staged until the whole container has been checked.** The order:

1. **Authenticate from the credential alone** (`Unauthorized`, `DeviceRevoked`, and
   any `Forbidden` the credential itself decides).
2. **The authorization that needs the vault,** as in section 4.9, step 3:
   `Forbidden`, then **the halt gate** (`ReplicaHalted`).
3. **The upload exists, or this is a retry.** A snapshot already staged from this
   upload answers `200` with its original result; no upload and no such snapshot is
   `UploadNotFound`.
4. **Writes permitted?** `WritesSuspended`.
5. **All parts received:** otherwise `UploadIncomplete` with `details.next_part`.
6. **The container.** The assembled length must equal `size` and its SHA-256 must
   equal `sha256`; otherwise `MalformedObject`, and the server **discards the
   upload** (the client begins again): a mismatch means the bytes sent are not the
   container that was declared, and no part of it is kept.
7. **The epoch.** The header's `key_epoch` is checked again against the epoch in
   force now, since it can have changed during a long upload: `EpochRejected`.
8. **Stage** the snapshot, turn the reservation of `size` into stored bytes, and
   answer `201`.

**`abortSnapshotUpload`:** `DELETE /vaults/{vault_id}/snapshots/{snapshot_id}/uploads`.
It removes an upload in progress and releases its reservation. Response `200`:
`{ "snapshot_id": "<uuid>", "aborted": true }`, or `false` when there was no upload
in progress (it is idempotent). It never removes a staged or published snapshot, and
the halt gate does not apply to it, so a halted device can clean up.

An upload that is not completed within `snapshot_upload_expiry_seconds` is aborted
by the server (section 7.3). Until completion the parts count toward `usage` only
through the reservation.

### 4.14 The snapshot closure sidecar

*Added to protocol version 1 on 2026-11-06 (`personal-cfo-v98vd` AC30; section 11.4
lists it).*

A snapshot's **closure** is every artifact id it references (`vault-format.md` C.3,
C.4 field 17, C.8). It holds at most **2^20** ids (`max_snapshot_closure_ids`,
section 7.1); an envelope's delta and a status request stay at 4096
(`max_artifact_manifest_ids`). Written as the JSON lists of sections 4.4 and 4.9, a
closure of 2^20 ids is about 70 MB (each id is 64 hex characters, two quotes and a
comma), nearly three times the 24 MiB every server must accept as JSON (7.1), and a
server would have to parse all of it into memory before it could check anything. So
a closure of more than 4096 ids travels as **its own octet-stream object**, staged
with the snapshot and before it is published; a closure of 4096 or fewer MAY use
either form.

**The body.** The ids' 32 raw bytes each, in strictly ascending byte order, with no
count, no separator and no framing: its length is 32 times the number of ids, at most
**33,554,432 bytes** (`max_snapshot_closure_bytes`, section 7.1), which every server
MUST accept in one request. Raw bytes are half the size of hex, and the same strict
rules as the JSON lists apply: a body that is not in strictly ascending order, or
repeats an id, is refused and never put in order for the sender. The public vectors
`docs/architecture/vectors/closure-sidecar-v1.json` pin this, with the empty closure,
4096 and 4097 ids, 2^20 ids and one more, a length that is not whole ids, and a body
out of order.

**`stageSnapshotClosure`:** `PUT /vaults/{vault_id}/snapshots/{snapshot_id}/closure`,
the body as `application/octet-stream` with a `Content-Length`. Like `stageSnapshot`
it accepts an account credential before the vault's genesis is installed and a device
credential after it. The server decides in this order, and the first failing step
answers. **Steps 1 to 4 are decided from the headers and the server's state alone, before
any byte of the body is read,** so a refused request costs the server no more than its
headers did.

1. **Authenticate from the credential alone** (`Unauthorized`, `DeviceRevoked`, and any
   `Forbidden` the credential itself decides).
2. **The length**, from `Content-Length` before any byte is read. A request with no
   `Content-Length`, or one sent with a chunked `Transfer-Encoding`, is
   `MalformedRequest`: the length is what lets a server refuse an over-long body unread.
   More than `max_snapshot_closure_bytes` is `SnapshotTooLarge` (HTTP 413, the closure
   holds more than 2^20 ids); a length that is not a multiple of 32 is `MalformedObject`.
3. **The vault, then the snapshot.** First the authorization that needs the vault, as in
   section 4.9, step 3: `Forbidden` for an account credential once the genesis is installed,
   for an account that is not the household's primary member and for the account of another
   household; then **the halt gate** of section 4.12 (`ReplicaHalted`): this operation
   stages a snapshot. Then the snapshot: it MUST have been staged (or already be published or
   retained), else `SnapshotNotFound`. A snapshot that is staged keeps its closure; one that
   was dropped is not there.
4. **Writes permitted?** `WritesSuspended`.
5. **The content.** The ids are strictly ascending (`MalformedObject`), and the
   closure commitment of `vault-format.md` C.3 over them, computed by the server, equals
   the `artifact_manifest_commitment` in the staged snapshot's clear header
   (`MalformedObject`). Nothing is stored for a body that fails, and a request that fails changes
   nothing for a snapshot that is already published.
6. **Quota.** `QuotaExceeded`.
7. **Store** the body with the snapshot and answer `201`, or `200` for a snapshot that
   already holds this closure (the commitment fixes the list, so a retry is always the
   same bytes). Response: `{ "snapshot_id": "<uuid>", "artifact_count": "1048576" }`.

The commitment check is the exact one of `installEpoch` step 8 and of
`publishSnapshot`, made once here and again when the snapshot is published or
installed, from the stored bytes. A snapshot with no staged closure has the list in
the `publishSnapshot` or `installEpoch` request, as before.

**`getSnapshotClosure`:** `GET /vaults/{vault_id}/snapshots/{snapshot_id}/closure`
returns a retained snapshot's closure in the same body, whatever its size (a closure
that was sent as a JSON list is returned in this form too), with `SnapshotNotFound`
for a snapshot that is not retained. The client checks the commitment against the
header itself. A staged snapshot's closure is not returned.

## 5. Push: the compare-and-swap and retry procedure

This section is normative for `pushEnvelopes` and applies `vault-format.md`
C.10. For each request the server decides in this order, and the first failing
step answers:

1. **Authenticate** the device credential (`Unauthorized`, `DeviceRevoked`,
   `Forbidden`). A revoked device can do nothing.
2. **Parse every container header** (`MalformedObject`, `UnsupportedSuite`).
   Check that each header's `vault_id` equals the path, `object_type` is `1`,
   `device_id` and `device_key_fingerprint` belong to the credential's device,
   `base_seq` equals the request's, and the number of envelopes is within the
   batch limit.
3. **Look up both identities of each envelope** in the accepted-envelope index:
   `command_id` and `idempotency_key_tag`.
   - Both found, and they are the same record with the same
     `canonical_envelope_digest` and `immutable_metadata_commitment`: the
     envelope is **already accepted**. Its result is that record's `seq` and
     `result_ref`. This holds even if the request's `base_seq` is stale and even
     if the envelope's `key_epoch` is an old one: the lookup runs **before**
     old-epoch rejection (the retry order of ADR 0077).
   - Either lookup finds a different bound command, tag, digest or commitment,
     or the two lookups disagree: reject the **whole request** with
     `ProtocolFork`. Nothing is accepted, and the server keeps both envelopes'
     identifiers for diagnosis.
   - Neither found: the envelope is **unknown**.
4. If every envelope is already accepted, answer `200` with the original results
   now. Steps 5 to 9 apply only to unknown envelopes, and they apply to the
   unknown envelopes as one unit.
5. **Writes permitted?** A credential that no longer permits writes is
   `WritesSuspended`.
6. **Epoch in force.** Each unknown envelope's `key_epoch` MUST be the epoch in
   force at the head; otherwise `EpochRejected`. This is the same
   epoch-position rule a client applies (`vault-format.md` C.7): an envelope with
   `seq <= h` belongs to the earlier epoch and one with `seq > h` to the new one.
7. **Artifacts.** For each unknown envelope the sidecar MUST be strictly
   ascending and duplicate-free, its commitment (`vault-format.md` C.3) MUST equal
   the header's `artifact_manifest_commitment` (absent if and only if the sidecar
   is empty), and every id MUST have a durable receipt at the envelope's
   `key_epoch`. Otherwise `MalformedObject` (commitment or order) or
   `ArtifactMissing` (receipt).
8. **Nonce backstop** (section 5.4): `NonceTupleReused`.
9. **Compare and swap.** If the request's `base_seq` equals the current head,
   append all unknown envelopes in order and assign `seq = head + 1`,
   `head + 2`, and so on, atomically with their pins and index records. If not,
   reject with `HeadMismatch` carrying the tail since `base_seq`. Quota is
   checked before the append (`QuotaExceeded`).

### 5.1 What "stale" means

A push with a stale `base_seq` is not an error when every envelope in it is
already accepted: the client sent it again after a lost response, and the index
answers. A stale push of an envelope the index does not hold is `HeadMismatch`:
the client pulls, rebases and sends again (ADR 0074 decision 2).

### 5.2 The first sequence number is 1

The log head is `0` for a vault with no envelopes, and the first envelope a
server accepts is assigned `seq = 1`. A server MUST NOT assign `seq = 0`. The
genesis epoch has position `0`, so the epoch-position rule of C.7 relies on this.

### 5.3 Index durability

The accepted-envelope index is part of the server's recovery state: it MUST be
retained for as long as any device can retry its outbox, with no automatic
expiry, and it MUST survive tail truncation, snapshot bootstrap and restart. It
holds only the opaque fields of C.10.

### 5.4 Duplicate-detection backstop

The server records the first `(key_epoch, device_key_fingerprint, object_type,
nonce)` tuple it sees in a container's clear header (the nonce is
`nonce_domain` and `invocation_counter`). A later object with the same tuple is
accepted only if it is **byte-identical** to the first; otherwise the server
answers `NonceTupleReused` and stores nothing. This is a detection and
idempotency aid, not the security source: nonce uniqueness comes from the
client's durable counter register (`vault-format.md` C.4). A server never
chooses, allocates or assigns a nonce domain or a counter range, and the
protocol has no operation that does.

## 6. Membership and epoch transactions

### 6.1 Checks `installEpoch` performs

The service enforces every rule it can check on public data. In order, the first
failure answers:

1. **Parse** both records (`MalformedObject`, `UnsupportedSuite`); the
   `kem_public_key` must be 32 bytes in suite 1. Parsing comes first because the
   remaining checks branch on the epoch record's `reason`.
2. **Authenticate and locate.** A genesis (`reason 1`) needs an account
   credential and a vault with no installed genesis; a genesis against a vault
   that already has one is `VaultExists`, the code the client turns into
   "bootstrap as a replica" (ADR 0074 decision 9), **unless it is the exact retry
   of step 3**. Every other reason needs a device credential of an authorized
   member of an existing vault (`VaultNotFound`). For every reason but the
   genesis, the signer of the membership record and the sealer of the epoch
   record MUST be the credential's device; the genesis signer and sealer are the
   founding device itself, and the staged snapshot's header `device_id` MUST be
   that device.
3. **Exact retry.** If this epoch record (and membership record) is already
   installed byte for byte, answer `200` with the original result, whatever the
   reason. A lost response to a genesis is therefore safe to retry: the same
   bytes return `200`, and only a *different* genesis for the vault is
   `VaultExists`.
4. **Epoch shape.** The record's `reason`, `key_epoch`, `prev_epoch_id` and
   `membership_head` MUST follow the reason table of `vault-format.md` C.6 and
   the sequencing rules of C.6, relative to the newest installed epoch record:
   `BadEpochSequence`. A cutover MUST name a `revoke` record
   (`CutoverHeadNotRevoke`), a grant an `add` record (`GrantHeadNotAdd`). A
   membership record that no epoch record names, or an epoch record that names a
   membership record not in the request, is `EpochRecordMissing`. Recipient
   lists MUST match the reason table exactly, computed from the membership
   chain (`EpochRecordInvalid`).
5. **Key commitments.** For `reason 3` both commitments MUST equal the previous
   record's, and for every record after the first `key_commitment_S` MUST equal
   the first record's: `KeyCommitmentMismatch`.
6. **Membership chain.** The membership record MUST satisfy chain rules 1 to 7
   of C.6: version, `prev_id`, authorized unrevoked signer, a strict Ed25519
   signature, fingerprint recomputation, no reused fingerprint, and a `revoke`
   that repeats its `add` exactly: `MembershipChainInvalid`. A record whose
   `prev_id` is no longer the newest installed membership record is
   `StaleMembershipHead`. Checks 4 and 5 come first, so an `add` and grant
   prepared before a cutover are reported as `BadEpochSequence` (or
   `KeyCommitmentMismatch` if forged), as C.7 states. A forged grant may surface
   as either code, depending on which of the two checks it trips first; a
   conformance test accepts either.
7. **Head.** When a snapshot is present, `base_seq` MUST equal the current head
   and the snapshot's header `base_seq`: `HeadMismatch`. A grant (`reason 3`)
   without a snapshot carries `base_seq` `null`, has no head check, and is
   recorded at whatever the head is when it is installed: a grant changes no key,
   so it has no epoch position to protect and need not lose races to ordinary
   pushes.
8. **Snapshot and closure.** The staged snapshot MUST exist
   (`SnapshotNotFound`), its header MUST agree with the record (`vault_id`, the
   epoch's `key_epoch`, `object_type 2`, the sidecar's commitment:
   `MalformedObject`), and every closure id MUST have a durable receipt at the
   new `key_epoch` (`ArtifactMissing`). The sidecar is the closure staged with
   `stageSnapshotClosure` if the snapshot has one (section 4.14), and the request's
   `artifact_manifest` otherwise; either way the commitment is recomputed from the
   ids, and `ArtifactMissing` lists at most the first 4096 missing ids (section 4.4).
9. **Writes and quota.** `WritesSuspended`, `QuotaExceeded`.

Then the transaction commits atomically (section 4.4).

### 6.2 Adding a device (a grant)

A new device B generates its two key pairs, shows its public keys to an existing
device A (the pairing flow is the client's, `personal-cfo-elciw`), and A submits
`installEpoch` with `reason 3`: the `add` record for B and a grant that seals the
current `K_e` and `S` to B. B then calls `claimCredential` with the account
credential and a proof of possession, receives its own device credential, calls
`getChain`, unseals its grant, and bootstraps from `getLatestSnapshot`. No key
rotates and no new snapshot is required (`vault-format.md` C.6).

**An add racing a cutover is rejected and retried.** The service installs a
membership record only if its `prev_id` is still the newest installed membership
record (compare and swap). An `add`, and the grant that seals the pre-cutover
`K_e`, prepared before a revocation cutover therefore fail: the grant is
`BadEpochSequence` (its `key_epoch` is not the epoch in force) or, if forged to
claim the new epoch, `KeyCommitmentMismatch`, and the `add`'s `prev_id` is stale.
The adder fetches the chain and retries from the new head with a fresh `add` and a
grant that seals the new `K_{e+1}` under the new epoch number (`vault-format.md`
C.6 "A grant never outlives its epoch" and C.7). A server MUST pass the contract
test of this rule before it ships (the test belongs to `personal-cfo-v98vd`).

### 6.3 Revoking a device (a cutover)

An authorized remaining device A, in order: uploads every live artifact under
epoch `e + 1` (`putArtifact`), stages a snapshot under `K_{e+1}` at the current
head (`stageSnapshot`), and calls `installEpoch` with `reason 2`. In one
transaction the server installs the `revoke` record and the epoch record,
publishes the snapshot, revokes the target device's enrollment **and its
credential**, and rejects every later write, artifact receipt or pin under epoch
`e` (ADR 0074 decision 8). A request from the revoked device arriving after the
transaction is `DeviceRevoked`. Ciphertext the revoked device obtained before the
cutover cannot be made secret again; that limitation is the client's to disclose.

If a cutover loses the race to another write (`HeadMismatch`), the client pulls,
rebases, builds and stages a **new** snapshot at the new head (the old one's
`base_seq` is stale), and retries; the artifacts it already uploaded under epoch
`e + 1` remain valid until their grace period ends (section 7.3). A replacement after `NonceStateLost` is a revoke of the old
device followed by an add of a new one with fresh keys (C.6 chain rule 7).

### 6.4 Epoch position

The position of a key-changing epoch record is the log head at which it was
installed, and its snapshot's header `base_seq` equals that value; the genesis
epoch has position `0`. The server reports it in `getChain` and as an `epoch`
entry in the log, and applies the epoch-position rule to new pushes (section 5,
step 6). The guarantee is relative to the log order a client has observed; a
malicious server can still order its own log (`vault-format.md` C.6, "Residual").

## 7. Limits, retention and truncation

### 7.1 Protocol constants and format maxima

These numbers are fixed by protocol version 1. A server MUST accept every
well-formed object within the format maxima, and a client MUST NOT exceed them. (A
snapshot is accepted through `stageSnapshot` up to the server's
`snapshot_single_request_max_bytes`, and above that through the upload operations of
section 4.13.)
They are derived from the field maxima of `vault-format.md` C.1.1 to C.9.

<!-- table:limits -->

| name | value | meaning |
| --- | --- | --- |
| `max_envelope_container_bytes` | 17040163 | largest well-formed `DFSY` envelope container: 2^24 payload, a delta manifest of 4096 items of 64 bytes (an id and its ciphertext hash), and a header that carries both manifest commitments (fields 17 and 18) |
| `max_snapshot_container_bytes` | 1073742137 | largest well-formed `DFSY` snapshot container: plaintext at most 2^30, and a 279-byte header because header fields 7 and 8 are absent for a snapshot and fields 17 and 18 are both present (2^30 + 313) |
| `max_membership_record_bytes` | 2300 | largest signed membership record |
| `max_epoch_record_bytes` | 2421030 | largest signed epoch record (1024 recipients) |
| `max_artifact_manifest_ids` | 4096 | ids in one JSON manifest sidecar (an envelope's delta, or a snapshot's closure sent in a request) or one status request |
| `max_snapshot_closure_ids` | 1048576 | ids in one snapshot closure (2^20), sent as the sidecar body of section 4.14 when it holds more than 4096 |
| `max_snapshot_closure_bytes` | 33554432 | largest closure sidecar body: 2^20 ids of 32 bytes; every server accepts a `stageSnapshotClosure` request body this large |
| `max_epoch_recipients` | 1024 | recipients in one epoch record |
| `min_json_body_bytes` | 25165824 | every server accepts a JSON request body at least this large (one maximum envelope, base64url, and its sidecar) |
| `claim_skew_seconds` | 300 | allowed clock difference for a `claimCredential` proof |

<!-- /table:limits -->

### 7.2 Operator limits

Each server advertises these in `getInfo` and MAY change them over time. A
client reads them and does not assume a default.

| name | meaning |
| --- | --- |
| `max_artifact_bytes` | largest artifact container the server stores; a larger artifact is `PayloadTooLarge` |
| `push_batch_max_envelopes` | most envelopes in one `pushEnvelopes`; at least 1 |
| `pull_page_max_entries`, `pull_page_max_bytes` | page bounds for pulls and `HeadMismatch` tails; a page still holds at least one log object (an envelope, an epoch entry or a void) |
| `orphan_grace_seconds` | how long an upload not referenced by an accepted publication is kept |
| `superseded_root_window_seconds` | how long a superseded snapshot and its tail stay available to an offline device |
| `snapshot_single_request_max_bytes` | optional: the largest snapshot `stageSnapshot` accepts in one request (a larger one is `PayloadTooLarge`); absent means any size within section 7.1 |
| `snapshot_part_max_bytes` | optional: the largest part `putSnapshotPart` accepts, and the largest `part_size` `beginSnapshotUpload` accepts; at least 5,242,880 when sent; absent means the server offers no upload in parts |
| `snapshot_upload_expiry_seconds` | optional: how long an upload in parts may stay incomplete before the server aborts it (section 7.3); absent means seven days |

Quotas (storage, devices, seats) are not protocol values: a server reports use
through `getStatus` and refuses with `QuotaExceeded`. Request-rate limits are
signalled with `RateLimited` and `Retry-After`; the protocol fixes the signal, not
the numbers.

### 7.3 Retention, truncation and collection

A server MUST publish finite windows for orphan uploads and superseded roots and
MUST NOT shorten a window already announced for an existing object (ADR 0074
decision 1). Beyond that, how a server is built is its own business, with these
protocol-visible consequences:

- **Truncation.** Re-snapshotting is the only way history is truncated. A server
  drops envelopes at or below `s` only if every enrolled device has acked at
  least `s` or the superseded-root window has passed, and never past the latest
  published snapshot's `base_seq`. A device that returns below `floor_seq`
  bootstraps from the latest snapshot and rebases (`TailTruncated`).
- **The accepted-envelope index is never truncated** (section 5.3).
- **Evidence of a void.** A server does not drop the envelope at a position a void
  names, or the void, until a snapshot whose `base_seq` is at or after the void's
  `position` has been published (Addendum D rule 8). The void claims in the index
  are permanent.
- **Incomplete uploads in parts.** An upload begun with `beginSnapshotUpload` that
  is not completed within `snapshot_upload_expiry_seconds` is aborted by the
  server, its parts are discarded and its reservation is released. The expiry is
  fixed when the upload begins (the rule above for announced windows). Seven days is
  the usual default of the object stores servers sit on.
- **Closure sidecars.** The bytes of a staged closure (section 4.14) count toward
  `usage` like any stored object, and are deleted with their snapshot.
- **Collection.** An artifact is collectible only if no active or retained
  snapshot or tail pins it and a newer retained recovery root excludes it. An
  uploaded object that no accepted publication references is collected after
  `orphan_grace_seconds`. A client whose upload is collected uploads it again
  before it reissues a reference (`ArtifactRebootstrapRequired` is the client's
  state, ADR 0074).
- **Logs.** A server's operational logs carry only the allowlist of
  ADR 0066-A decision 5 and are deleted within the period it sets.

## 8. Errors

### 8.1 Registry

Error codes reuse the names of `vault-format.md` C.11 where both a client and a
server can produce the failure. C.11 failures that only a client can detect
(`NonceStateLost`, `InvalidIdempotencyKey`, `NoAuthorizedDevice`,
`EpochEquivocation`, `MembershipChainFork`, `SyncHalted`, `StateRolledBack`,
`ArtifactDescriptorChanged`, `ArtifactMaterializationFailed`,
`ArtifactRebootstrapRequired`) are not service errors.

<!-- table:errors -->

| code | status | meaning and the client's response |
| --- | --- | --- |
| `MalformedRequest` | 400 | A header, path, query or JSON member violates section 2. Fix and resend. |
| `MalformedObject` | 400 | A container or record violates `vault-format.md` (CBE1, framing, header agreement, commitment). Never resent unchanged. |
| `UnsupportedProtocol` | 400 | The version is not supported; `details.supported` lists versions. |
| `UnsupportedSuite` | 400 | The suite or `kem_id` is not implemented by this server. |
| `ClaimProofInvalid` | 400 | The `claimCredential` proof or `issued_at` is invalid. |
| `MembershipChainInvalid` | 400 | The membership record breaks a chain rule of C.6. |
| `EpochRecordInvalid` | 400 | The epoch record's recipients or shape break C.6. |
| `EpochRecordMissing` | 400 | A membership record is not named by an epoch record in the request, or the reverse. |
| `CutoverHeadNotRevoke` | 400 | A `reason 2` record does not name a `revoke` record. |
| `GrantHeadNotAdd` | 400 | A `reason 3` record does not name an `add` record. |
| `KeyCommitmentMismatch` | 400 | A key commitment is not the one C.6 requires. |
| `VoidSignatureInvalid` | 400 | The void record's signature does not verify strictly under the chain's key for its signer (C.10). |
| `Unauthorized` | 401 | No valid credential. |
| `Forbidden` | 403 | The credential may not act on this vault or operation. |
| `DeviceRevoked` | 403 | The device behind the credential was revoked. The client tells the user; local data is untouched. |
| `VoidSignerNotAuthorized` | 403 | The void record's signer is not an authorized member at the position the void would take. Never resent unchanged. |
| `WritesSuspended` | 403 | The credential no longer permits writes at this endpoint. Reads continue through the grace period. A statement about this endpoint only. |
| `QuotaExceeded` | 403 | A server quota would be exceeded. |
| `VaultNotFound` | 404 | The server has no such vault. |
| `DeviceNotFound` | 404 | The device is not currently authorized in the chain. |
| `UploadNotFound` | 404 | No snapshot upload in progress for this snapshot id (never begun, aborted, expired, discarded after a failed completion, or already completed). Begin again. |
| `SnapshotNotFound` | 404 | No such published or staged snapshot. |
| `ArtifactNotFound` | 404 | No such artifact. |
| `VaultExists` | 409 | Genesis for a known `vault_id`: bootstrap as a replica instead. |
| `HeadMismatch` | 409 | The head moved. Pull, rebase, retry; `details` carries the tail. |
| `StaleMembershipHead` | 409 | A membership record's `prev_id` is not the newest. Fetch the chain, rebuild. |
| `BadEpochSequence` | 409 | The record's epoch does not follow the epoch in force. Fetch the chain, rebuild. |
| `EpochRejected` | 409 | An object's `key_epoch` is not the epoch in force at its position. |
| `ProtocolFork` | 409 | An identity matches an accepted envelope but not the whole of it. `details` names the command (section 4.6). Halt it; keep both envelopes. |
| `NonceTupleReused` | 409 | The same nonce tuple arrived with different bytes. Never retried unchanged. |
| `ObjectConflict` | 409 | Different bytes for an existing artifact address or snapshot id. |
| `ArtifactMissing` | 409 | A manifest names an object with no durable receipt. `details.missing` lists the ids: all of them when there are at most 4096, otherwise the first 4096 in ascending order (a closure can hold 2^20 ids, section 4.14). Upload those, then retry; if more are missing the answer lists the next 4096. |
| `AckBeyondHead` | 409 | The acked `seq` is above the head. |
| `PartOutOfSequence` | 409 | A snapshot part other than the next one expected. `details.next_part` is the one to send. |
| `UploadIncomplete` | 409 | A snapshot upload was completed before every part arrived. `details.next_part` is the first missing one. |
| `VoidTargetMismatch` | 409 | The void names a position that is not a command envelope in the index, or a `command_id` or `envelope_digest` other than the index's. |
| `ReplicaHalted` | 409 | This device reported a halt (section 4.12), so this request is refused until it reports that it is no longer halted. `details.stopped_at` is the reported position. A statement about this device's own report only. |
| `TailTruncated` | 410 | `after_seq` is below `floor_seq`. Bootstrap from the latest snapshot. |
| `PayloadTooLarge` | 413 | An object exceeds a size limit. |
| `SnapshotTooLarge` | 413 | A snapshot exceeds the plaintext bound of C.8. |
| `RateLimited` | 429 | Wait `Retry-After`, retry unchanged. |
| `InternalError` | 500 | Server fault. Retry with backoff. |
| `ServiceUnavailable` | 503 | Retry with backoff or `Retry-After`. |

<!-- /table:errors -->

### 8.2 Common errors

Any authenticated operation can also return `Unauthorized`, `Forbidden`,
`DeviceRevoked`, `MalformedRequest`, `UnsupportedProtocol`, `RateLimited`,
`InternalError` and `ServiceUnavailable`. They are not repeated in the
operations table.

## 9. What a server never sees

A conforming server is built so that none of the following reaches it, and a
conforming client never sends any of them:

- vault plaintext: ledger rows, balances, account names, payees, categories,
  notes, attachment bytes, filenames or MIME types;
- any key: the vault password, the vault DEK, the Sync epoch root `K_e`, the tag
  secret `S`, a derived AEAD key, an artifact object key, or a device private
  key (signing or agreement);
- unsealed anything: the only key material on the wire is the sealed blobs inside
  a signed epoch record, which the server cannot open;
- the plaintext idempotency key (the server sees only the keyed
  `idempotency_key_tag`);
- the origin `StorageId`, wrapped or content-key fields, content nonces, blob
  paths, or any bare plaintext digest of an artifact (`vault-format.md` C.9,
  "Never service-visible");
- bank, provider or connector credentials.

A server sees only: vault routing metadata, opaque ids and commitments, sizes,
timestamps, `device_id` and fingerprints, key epochs, the clear header fields of
`vault-format.md` C.4, and ciphertext. The server has no decrypt path and needs
no cryptographic primitive other than hashing and strict Ed25519 verification
(for the membership and epoch chain and for `claimCredential` proofs).

## 10. Compatibility and naming

- A server that implements this document can be used with the DohFlow desktop
  client through the Settings endpoint override (ADR 0066-A decision 2), which
  is a device-local setting behind a trust prompt that names the host.
- "Compatible with DohFlow" is a true statement for a server that passes the
  conformance checks of section 11.2, and the nominative use it makes of the name
  is permitted by `TRADEMARK.md`.
- This document and the protocol it publishes grant no right to the DohFlow
  name or marks. A server MUST NOT present itself as the official DohFlow Sync
  service, or use the name "DohFlow Sync" or confusingly similar branding,
  without a specific written license or agreement from the DohFlow maintainer
  (`TRADEMARK.md`: a hosted service named after DohFlow is a case to ask first).

## 11. Change control and conformance

### 11.1 Versions

- `protocol_version` here is the same registry value as the clear header's
  `protocol_version` in `vault-format.md` C.1.3; version `1` is the only one.
- Within one protocol version a server MAY add: new optional members in a
  response (clients ignore them), new operations, new operator limits, and new
  error codes (clients handle an unknown code by status class). It MUST NOT
  change an existing operation's request, an existing member's meaning, a status
  code, the decision order of sections 5 and 6, or a number in section 7.1.
  Anything else is a new `protocol_version`.
- **Pre-release** (ruled 2026-11-02, `personal-cfo-g3m.33`). Protocol version 1
  is pre-release until the first deploy of a conforming Sync server to the
  operator's production environment; staging does not count. Until then version
  1 may change in ways the rules above forbid, and every change is listed with
  its date and bead in the pre-release list of section 11.4. From that deploy the
  rules above govern strictly, and anything else is a new `protocol_version`.
- A change to this document that alters the operation, error or limit lists
  changes `sync-service-protocol.json` in the same commit and bumps the version
  marker only when the rule above requires a new protocol version. A new
  protocol version is a new document revision with its own reviewed vectors.
- `engine_version` and `schema_version` skew is handled by ADR 0074 (R40) in the
  client; the server stores and returns the values in the clear header and
  gives them no other meaning.

### 11.2 Conformance

`scripts/tests/sync-service-protocol-contract.test.sh` pins the protocol version
and checks that the operation, error and limit tables here equal
`sync-service-protocol.json`, which the client and the server both consume. The
`docs/architecture/vectors/sync-service-void-v1.json` (`vectors_version` 1) pins
the void additions of sections 4.11 and 4.12 as public conformance vectors, each
case a sequence of requests to a server in a stated state with the status, error
code and result it must give (`appendVoid` accepted, kept twice for one position,
retried, and refused at each step of its order; the `voids` member of a pull; the
halt gate), generated and checked by
`crates/sync-core/tests/sync_service_void_vectors.rs`.
`docs/architecture/vectors/sync-service-snapshot-parts-v1.json` (`vectors_version` 1)
does the same for the upload in parts of section 4.13 (a container staged in
parts, the maximum and one byte over, the part-size bounds, a part of the wrong
size or out of sequence, a retried part, a completion with a missing part or a digest
that does not match, abort, the decision order of begin, part and complete, the halt
gate, and a snapshot too large for one request), generated and checked by
`crates/sync-core/tests/sync_service_snapshot_parts_vectors.rs`. `docs/architecture/vectors/closure-sidecar-v1.json` (`vectors_version` 1) pins the
closure sidecar body of section 4.14 (the empty closure, 4096 and 4097 ids, 2^20 ids and
one more, a length that is not whole ids, a body out of order or repeating an id, and the
closure commitment of each accepted body), generated and checked by
`crates/sync-core/tests/closure_sidecar.rs`. The
behavioral contract suite is owned by `personal-cfo-v98vd`: the simulator's
in-memory sequencer (`personal-cfo-tp2ln`) is the reference implementation, and
one suite runs against it and against the real server. A disagreement between an
implementation and this document is an implementation bug, or an erratum to this
document through the change control above.

### 11.3 Not specified here

Account sign-in and the issuance of account credentials, billing and
entitlement, hosting, region and sub-processors, storage layout, quota and rate
numbers, and the service's internal design are outside this protocol.

### 11.4 Pre-release changes and additions within protocol version 1

Each row is a change made to version 1, with its date and bead. The version stays
`1`; `sync-service-protocol.json` changed in the same commit. The **basis** says
which rule of section 11.1 allows it: an *addition* (the second bullet) or a
*pre-release amendment* (the pre-release bullet, valid only before the first
production deploy).

| Date | Bead | Basis | Change |
| --- | --- | --- | --- |
| 2026-11-02 | `personal-cfo-g3m.33` | pre-release amendment | The optional response members `voids` (on `pullEnvelopes` and the `HeadMismatch` details) and `void_claims` (on `pushEnvelopes` results and `HeadMismatch` `accepted` entries), which a version-1 client MUST process (4.7, 4.11), and the shared sequence of `seq` values they imply: a void takes a `seq`; and the page rule that follows from it: a page holds at least one log object (an envelope, an epoch entry or a void), not at least one entry, because a page can hold only voids. |
| 2026-11-02 | `personal-cfo-g3m.33` | addition | Operations `appendVoid` (4.11) and `reportHalt` (4.12); errors `VoidSignatureInvalid`, `VoidSignerNotAuthorized`, `VoidTargetMismatch` and `ReplicaHalted`, the last of which the halt gate (4.12) returns to `stageSnapshot`, `publishSnapshot` and `installEpoch`; the void evidence rule of 7.3. |
| 2026-11-03 | `personal-cfo-g3m.36` | addition | Operations `beginSnapshotUpload`, `putSnapshotPart`, `completeSnapshotUpload` and `abortSnapshotUpload` (4.13), which stage a snapshot in parts; errors `UploadNotFound`, `PartOutOfSequence` and `UploadIncomplete`; the optional `getInfo` limits `snapshot_single_request_max_bytes`, `snapshot_part_max_bytes` and `snapshot_upload_expiry_seconds` (7.2); the incomplete-upload expiry of 7.3; the halt gate (4.12) also applies to the three operations that stage; and the second exception to the failure rule of 2.5, the discard of an upload whose container does not match what was declared (4.13). |
| 2026-11-03 | `personal-cfo-g3m.36` | pre-release amendment | What 7.1's acceptance rule means for a snapshot: a server accepts it through `stageSnapshot` up to its `snapshot_single_request_max_bytes` and above that through the operations of 4.13, so `stageSnapshot` can answer `PayloadTooLarge` (a code it did not list before) for a container over that limit. |
| 2026-11-03 | `personal-cfo-v98vd` (AC27) | pre-release amendment | The failure name for an epoch record that names a membership record that is neither installed nor in the request is `EpochRecordMissing` (400, a malformed pairing), as 6.1 step 4 already said, and no longer `BadEpochSequence` (409, "fetch the chain and rebuild"), which sent a client with a malformed request into a rebuild loop. The membership-chain verifier named it `BadEpochSequence`, and the public vector `invalid_epoch_naming_an_unknown_head` pinned that name; both are corrected (vault-format.md spec-version 27), and `invalid_grant_without_the_add_it_names` is added (`vectors_version` stays 1: 60 cases). Section 6.1 and the error registry are unchanged. |
| 2026-11-06 | `personal-cfo-v98vd` (AC30) | pre-release amendment | Section 2.4's list of operations that accept an account credential before the genesis is installed named `stageSnapshot`, `putArtifact` and `installEpoch` only, though 4.13 already gave the four operations that stage in parts the account credential. The list now names those four and `stageSnapshotClosure`. This corrects 2.4 to agree with 4.13 and with the operation table (`auth` is `account/device` for each); it changes no request, response or code. |
| 2026-11-06 | `personal-cfo-v98vd` (AC30) | addition | Operations `stageSnapshotClosure` and `getSnapshotClosure` (4.14), which stage and return a snapshot's closure as a binary sidecar of up to 2^20 ids; the halt gate (4.12) also applies to `stageSnapshotClosure`; the response member `artifact_count` of `getLatestSnapshot`. |
| 2026-11-06 | `personal-cfo-v98vd` (AC30) | pre-release amendment | A snapshot's closure may hold up to 2^20 ids, where 4.4 and 4.9 allowed the 4096 of `max_artifact_manifest_ids`: a closure of more than 4096 ids is staged first and the `publishSnapshot` or `installEpoch` request then carries an empty `artifact_manifest` (a request member's meaning); `getLatestSnapshot` returns `artifact_manifest` as `null` for such a closure; `ArtifactMissing` lists at most the first 4096 missing ids; and section 7.1 gains the rows `max_snapshot_closure_ids` and `max_snapshot_closure_bytes`. |
| 2026-11-11 | `personal-cfo-v98vd` | pre-release amendment | The decision order of the operations that stage a snapshot (`stageSnapshot` 4.9, `beginSnapshotUpload`, `putSnapshotPart` and `completeSnapshotUpload` 4.13, `stageSnapshotClosure` 4.14), and where the halt gate of 4.12 runs: authenticate from the credential alone, then the checks that need no vault state, then the `Forbidden` that needs the vault and the halt gate, then the rest. The common rule of 2.1, that routing checks the path forms before the credential is read, is now stated once and not per operation. 4.13 and 4.14 had put the vault-side `Forbidden` and the halt gate before the checks that need no vault state; 4.9 had no explicit order. No request, response, status or code changed. |

**Why `voids` is a pre-release amendment and not an addition.** A response member
that a client may ignore is an addition. `voids` cannot be ignored: a void takes a
`seq` and a page can hold only voids, so a client that ignored it would see gaps
in the envelope sequence and pages with an empty `entries`, and would loop on
`HeadMismatch` or skip a void (ADR 0074 Addendum D needs every replica to see
every void). No client or server implements version 1 yet, so requiring clients to
process it breaks nothing, and the alternative, a second protocol version before
the first has shipped, would add cost and no safety. The rule is fixed by the
pre-release bullet of 11.1 and ends at the first production deploy.

That change against the five things section 11.1 forbids a change to within a
version, for the **additions** of the second row and for the pre-release row where
it says so:

1. **An existing operation's request:** not changed. No existing request gains a
   member, a query parameter or a header; the two new operations are new paths.
2. **An existing member's meaning:** `entries`, `has_more`, `head_seq`,
   `floor_seq` and `results` mean what they did, and a log with no void gives
   byte-for-byte the responses it gave before, with no `voids` and no
   `void_claims`. For a log **with** a void, a page can hold only voids and the
   envelope sequence has gaps that `voids` fills; that is the pre-release
   amendment of the first row, not an addition.
3. **A status code:** not changed. The four new codes have their own statuses; no
   existing code moved.
4. **The decision order of sections 5 and 6:** not changed. No step is added,
   removed or reordered in either list. The halt gate is defined outside them
   (4.12) and passes for every device with no recorded halt.
5. **A number in section 7.1:** not changed. No row of the limits table changed,
   and none was added.

For `personal-cfo-g3m.36` (the third row), against the same five things:

1. **An existing operation's request:** not changed. `stageSnapshot` takes the
   request it took; the four new operations are new paths.
2. **An existing member's meaning:** not changed. `getInfo` gains three optional
   members and a client that does not read them still works against a server that
   offers `stageSnapshot` up to the 7.1 maximum.
3. **A status code:** not changed. The three new codes have their own statuses, and
   `PayloadTooLarge` keeps its `413`.
4. **The decision order of sections 5 and 6:** not changed. The new operations have
   their own orders (4.13), and the halt gate they join is defined outside both lists.
5. **A number in section 7.1:** not changed. No row of the limits table changed, and
   none was added: the 5 MiB and 10,000 of 4.13 are stated in that section, not in the
   table. The one rule of 7.1 whose reading is narrowed, that a server MUST accept every
   well-formed object, is the pre-release amendment of the fourth row.

For `personal-cfo-v98vd` AC30 (the last three rows), against the same five things. The
addition row is the two new operations and one optional response member. The first
pre-release row (the 2.4 list) is a text correction that changes no request, response,
code or number, so the five checks pass trivially for it. The second pre-release row is
what the checks below find:

1. **An existing operation's request:** changed, so this is a pre-release amendment and
   not an addition. `artifact_manifest` in `publishSnapshot` and `installEpoch` is empty
   when the closure was staged with `stageSnapshotClosure`; before, it was the whole
   closure. No member or path was added to either request. A client that never stages a
   closure sends what it sent before, for a closure of at most 4096 ids.
2. **An existing member's meaning:** `getLatestSnapshot`'s `artifact_manifest` is `null`
   for a closure of more than 4096 ids, which the 4096 bound made impossible before, and
   `ArtifactMissing`'s `missing` is capped at the first 4096 ids. Both are the same
   member for every closure the old text allowed. This is the second reason the row is a
   pre-release amendment.
3. **A status code:** not changed. `SnapshotTooLarge` keeps its `413` and now also
   answers a closure body of more than 2^20 ids.
4. **The decision order of sections 5 and 6:** not changed. `installEpoch` step 8 and the
   `publishSnapshot` check read the closure from the staged bytes instead of the request
   when there are staged bytes, and make the same checks in the same order.
5. **A number in section 7.1:** none changed. Two rows were added, and
   `max_artifact_manifest_ids` keeps 4096; its meaning row is reworded to say it bounds
   a JSON list, which is what it always bounded.

For `personal-cfo-v98vd` (the 2026-11-11 row), against the same five things. The row is a
pre-release amendment, valid only before the first production deploy. It changes no request,
no member of any response, no status code and no number of 7.1. A request that two checks
would refuse is now refused by the one that needs no vault state; both are `4xx` that a
client does not retry unchanged, so no client behaves differently. Neither decision order of
sections 5 and 6 gains, loses or reorders a step: the orders amended are those of 4.9, 4.13 and
4.14, and the halt gate keeps its rule (it runs before the decisions that read the vault's
state) and now says where it stands among the stateless checks.

The reason is that a request that can be refused without the vault should not reach it: a
stateless refusal costs no vault object and cannot bind a `vault_id` to an account (section
2.4), and the vault-side `Forbidden` is the first thing that needs the vault.

**Why not a larger JSON limit, and why not chunks.** A JSON closure of 2^20 ids is about
70 MB and a server would parse it whole before checking a byte of it. Raw ids are half the
size of their hex. They are still 32 MiB, because a `sync_artifact_id` is a 32-byte
HMAC-SHA256 output (`vault-format.md` C.3), so 2^20 of them do not fit under the 24 MiB
(`min_json_body_bytes`) that bounds a JSON request; the sidecar is therefore a separate
request with its own limit in 7.1. The upload in parts of 4.13
was considered and left out: a closure that large is already extreme (a million artifacts),
33,554,432 bytes is half the 64 MiB `snapshot_single_request_max_bytes` that 4.1 shows for
a snapshot, and chunking would add three operations and a state machine for a body that fits
in one request on every host the protocol targets.
