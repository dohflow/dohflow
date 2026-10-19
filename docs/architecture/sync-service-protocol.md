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
  characters.
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
appear within one protocol version).

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
  installed**, by `stageSnapshot`, `putArtifact` and `installEpoch` (`reason 1`),
  because no device credential exists yet.
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

A failure mutates nothing, with one stated exception: an upload that completed
before the failing step stays staged for the grace period of section 7.3.

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
| `installEpoch` | POST | `/vaults/{vault_id}/epochs` | account/device | 201/200 | VaultExists, VaultNotFound, WritesSuspended, MembershipChainInvalid, EpochRecordInvalid, EpochRecordMissing, CutoverHeadNotRevoke, GrantHeadNotAdd, BadEpochSequence, KeyCommitmentMismatch, StaleMembershipHead, HeadMismatch, SnapshotNotFound, ArtifactMissing, QuotaExceeded, MalformedObject, UnsupportedSuite |
| `claimCredential` | POST | `/vaults/{vault_id}/devices/{device_id}/credential` | account | 200 | VaultNotFound, DeviceNotFound, ClaimProofInvalid |
| `pushEnvelopes` | POST | `/vaults/{vault_id}/envelopes` | device | 200 | VaultNotFound, WritesSuspended, MalformedObject, UnsupportedSuite, ProtocolFork, EpochRejected, ArtifactMissing, NonceTupleReused, HeadMismatch, QuotaExceeded, PayloadTooLarge |
| `pullEnvelopes` | GET | `/vaults/{vault_id}/envelopes` | device | 200 | VaultNotFound, TailTruncated |
| `ack` | POST | `/vaults/{vault_id}/acks` | device | 200 | VaultNotFound, AckBeyondHead |
| `stageSnapshot` | PUT | `/vaults/{vault_id}/snapshots/{snapshot_id}` | account/device | 201/200 | VaultNotFound, WritesSuspended, MalformedObject, UnsupportedSuite, EpochRejected, NonceTupleReused, ObjectConflict, SnapshotTooLarge, QuotaExceeded |
| `publishSnapshot` | POST | `/vaults/{vault_id}/snapshots/{snapshot_id}/publish` | device | 200 | VaultNotFound, WritesSuspended, SnapshotNotFound, MalformedObject, EpochRejected, HeadMismatch, ArtifactMissing, QuotaExceeded |
| `getLatestSnapshot` | GET | `/vaults/{vault_id}/snapshots/latest` | device | 200 | VaultNotFound, SnapshotNotFound |
| `getSnapshot` | GET | `/vaults/{vault_id}/snapshots/{snapshot_id}` | device | 200 | VaultNotFound, SnapshotNotFound |
| `putArtifact` | PUT | `/vaults/{vault_id}/artifacts/{artifact_id}` | account/device | 201/200 | VaultNotFound, WritesSuspended, MalformedObject, EpochRejected, ObjectConflict, PayloadTooLarge, QuotaExceeded |
| `getArtifact` | GET | `/vaults/{vault_id}/artifacts/{artifact_id}` | device | 200 | VaultNotFound, ArtifactNotFound |
| `getArtifactStatus` | POST | `/vaults/{vault_id}/artifacts/status` | device | 200 | VaultNotFound |

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
    "superseded_root_window_seconds": 2592000
  }
}
```

The values shown are examples. The members of `limits` are the **operator
limits** of section 7.2: each server advertises its own, and a client reads them
rather than assuming them. `protocol_versions` and `suites` list the versions
the server implements. `getInfo` needs no credential and carries no vault data.

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
staged first with `stageSnapshot`.

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
["<64 hex>"] }`, the closure ids that lack a durable receipt. The service
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

(`vault-format.md` C.1.1). `issued_at` is microseconds since the Unix epoch
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
holds at least one entry whenever any exists past `after_seq`, so a client always
makes progress. If `after_seq` is below `floor_seq`, the server returns
`TailTruncated` (HTTP 410) with `details.latest_snapshot` as in `getStatus`; the
client bootstraps from that snapshot.

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
   A different byte string for the same `snapshot_id` is `ObjectConflict`.
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

   A snapshot that starts a new key epoch (genesis, cutover, forced rotation) is
   published only by `installEpoch`, never by `publishSnapshot`
   (`EpochRejected`).

`getLatestSnapshot` (`GET /vaults/{vault_id}/snapshots/latest`) returns the
metadata of the newest published snapshot, `{ "snapshot_id", "base_seq",
"key_epoch", "size", "artifact_manifest": [...] }`. `getSnapshot` returns a
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
   new `key_epoch` (`ArtifactMissing`).
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
well-formed object within the format maxima, and a client MUST NOT exceed them.
They are derived from the field maxima of `vault-format.md` C.1.1 to C.9.

<!-- table:limits -->

| name | value | meaning |
| --- | --- | --- |
| `max_envelope_container_bytes` | 16909058 | largest well-formed `DFSY` envelope container |
| `max_snapshot_container_bytes` | 1073742104 | largest well-formed `DFSY` snapshot container: plaintext at most 2^30, and a 246-byte header because header fields 7 and 8 are absent for a snapshot (2^30 + 280) |
| `max_membership_record_bytes` | 2300 | largest signed membership record |
| `max_epoch_record_bytes` | 2421030 | largest signed epoch record (1024 recipients) |
| `max_artifact_manifest_ids` | 4096 | ids in one manifest sidecar or one status request |
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
| `pull_page_max_entries`, `pull_page_max_bytes` | page bounds for pulls and `HeadMismatch` tails; a page still holds at least one entry |
| `orphan_grace_seconds` | how long an upload not referenced by an accepted publication is kept |
| `superseded_root_window_seconds` | how long a superseded snapshot and its tail stay available to an offline device |

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
| `Unauthorized` | 401 | No valid credential. |
| `Forbidden` | 403 | The credential may not act on this vault or operation. |
| `DeviceRevoked` | 403 | The device behind the credential was revoked. The client tells the user; local data is untouched. |
| `WritesSuspended` | 403 | The credential no longer permits writes at this endpoint. Reads continue through the grace period. A statement about this endpoint only. |
| `QuotaExceeded` | 403 | A server quota would be exceeded. |
| `VaultNotFound` | 404 | The server has no such vault. |
| `DeviceNotFound` | 404 | The device is not currently authorized in the chain. |
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
| `ArtifactMissing` | 409 | A manifest names an object with no durable receipt. `details.missing` lists the ids. Upload, then retry. |
| `AckBeyondHead` | 409 | The acked `seq` is above the head. |
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
behavioral contract suite is owned by `personal-cfo-v98vd`: the simulator's
in-memory sequencer (`personal-cfo-tp2ln`) is the reference implementation, and
one suite runs against it and against the real server. A disagreement between an
implementation and this document is an implementation bug, or an erratum to this
document through the change control above.

### 11.3 Not specified here

Account sign-in and the issuance of account credentials, billing and
entitlement, hosting, region and sub-processors, storage layout, quota and rate
numbers, and the service's internal design are outside this protocol.
