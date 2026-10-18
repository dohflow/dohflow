# ADR 0077 — Data ownership and wire formats

- **Status:** Accepted (owner decision, 2026-10-18)
- **Tier:** Public
- **Date:** 2026-10-18
- **Bead:** `personal-cfo-klr.4`
- **Related:** ADR 0002 (local encrypted vault), ADR 0017 (sync identity and
  clock), ADR 0024 and 0024-A (backup format), ADR 0073 (sync disposition
  rules), ADR 0074 (DohFlow Sync architecture), ADR 0083 (vault key rotation),
  ADR 0082 (public disclosure boundary)

## Context

DohFlow has three data formats that look alike and must not be confused: the
`.pcfobk` backup that restores the same vault, a portable export that lets the
owner take their data elsewhere, and the logical snapshot and command tail that
bootstrap and feed a Sync replica. ADR 0074 decided how Sync behaves and
delegated the exact canonical encodings, derivations and records to this ADR;
ADR 0017 delegated the membership and nonce wire fields to it. Several of those
choices are cryptographic, and no earlier accepted ADR fixed them.

## Decision

### 1. Three separate formats

The formats stay separate. The `.pcfobk` backup and its vault-password
behavior are unchanged. The portable export is a versioned logical JSON
inventory of user-owned records, attached and source documents, a manifest, the
existing CSV export and recovery instructions, encrypted with the vault password
and no other secret. The Sync genesis snapshot reuses only the approved logical
class-1 model and the class-1b artifact closure, never the password wrapper and
never all portable history. Giving someone a portable bundle and its password
exposes the vault password; the bundle is for the owner's own migration.

### 2. One format authority

[`docs/architecture/vault-format.md`](../architecture/vault-format.md) is the
format authority. Part A documents the shipped local vault and backup, Part B the
inclusion and exclusion inventory and logical model of the portable export and
the genesis snapshot, and Part C the cryptographic framing and Sync wire
formats. Its version markers and a reference implementation of its test vectors
are checked in CI, so a format change without a specification change fails. A
reader supports an explicit range of versions and refuses the rest with a typed
error; no version promises that every future version opens.

### 3. Cryptographic decisions (owner, 2026-10-18)

These were reviewed adversarially before the owner decided; the review's
conditions are part of the decision.

- **Idempotency tag.** `idempotency_key_tag` is an HMAC under a key derived from
  a **non-rotating, vault-scoped secret `S`**, sealed to each device together
  with the epoch root `K_e`. `S` is an equality-tag key, not a confidentiality
  key. A device revoked after learning `S` can recognize only the tags of
  idempotency keys it already saw, because every key entering a Sync envelope is
  a UUID version 4 from the OS CSPRNG, enforced by the envelope builder (the
  backend moves from version 7 to version 4; `command_id` may stay version 7).
  A per-epoch tag key was rejected because it cannot be both stable across epochs
  and uncomputable by a revoked device, and it would break ADR 0074 Decision 2.
- **Device keys.** Each device has two **independent** key pairs, Ed25519 for
  signing and X25519 for key agreement, generated in platform-neutral Rust and
  wrapped at rest by the OS keystore. Hardware P-256 keys are not protocol keys.
  The fingerprint commits to both public keys, the suite and the KEM id, and a
  membership record carries the suite-typed agreement key at its own length. Private keys and
  nonce state never roam, sync or restore across devices.
- **Sealing the epoch key.** `K_e` and `S` are sealed to each enrolled device
  with HPKE base mode (DHKEM(X25519, HKDF-SHA256), HKDF-SHA256, AES-256-GCM).
  Base mode is unauthenticated, so the sealed set travels only inside an
  **epoch record signed by an authorized member**, verified against the signed,
  chained membership record before use. The record carries key commitments for
  `K_e` and `S`, and each HPKE `info` binds the protocol version, suite, vault,
  epoch, recipient and sealer identities and the membership head and epoch-record
  header hashes. Each epoch record states its `reason` (genesis, revocation
  cutover, grant, forced rotation) and every membership change is installed with
  its coupled epoch record, so adding a device is a grant with no key rotation. A
  recipient accepts an anchor record only against the newest membership head it has
  verified, catches up backward from it, and detects equivocation at every
  position (Part C.6).
- **Encodings.** One canonical binary encoding covers every AAD, signed body,
  HKDF input and hash input. Manifests and payloads are RFC 8785 canonical JSON
  with every 64-bit integer as a decimal string and no floating-point values.
- **Post-quantum posture.** Classical now, with agility: every sealed-key blob,
  membership record and epoch record carries `suite_id` and `kem_id`, so a hybrid
  KEM can be adopted at a later key epoch without a format break.
- **Retry order.** The service performs the dual accepted-index lookup before it
  rejects an old-epoch write, and a client re-encrypts an outbox entry under a new
  epoch only after the index confirms both identities are unknown. This fills a
  gap in ADR 0074.

### 4. Portable export decisions (owner, 2026-10-18)

- `household_timezone` is **class-1 user data**, changed only through a logged
  `WriteCommand`, and named in the specification as a logical field so its storage
  can move to the households row without a format change. It is not a header-only
  value.
- The portable export carries `operation_log` and `provenance_links` as read-only
  history and excludes `audit_events`, idempotency keys, backup history, durable
  jobs, staged rows, connector credentials and watermarks, and `device.*`
  settings.
- The bundle uses the vault password with an exact KDF-profile allowlist, a fresh
  per-export salt, distinct HKDF labels, header-bound authenticated data, a
  manifest that commits the exact object set, one key per plaintext, and chunking
  for large documents.
- The Sync genesis carries every class-1 ledger row of every age, so any device
  can view all history. Only the pre-v2 operation and provenance history stays out.

### 5. Relationship to ADR 0074 and ADR 0017

Nothing here changes a decision of ADR 0074 or ADR 0017. ADR 0074 receives an
addendum recording the equality-tag role of `S`, the retry order and a clarifying
sentence that Sync reuses the **logical** export model, not the password-wrapped
bundle. ADR 0017 receives a pointer to the signed-record specification.

## Consequences

- Implementation beads can start against a merged contract: the portable
  export and import (`personal-cfo-vn91`), the genesis snapshot and key epochs
  (`personal-cfo-1cltt`), the service (`personal-cfo-v98vd`), and the device-key
  store (`personal-cfo-1zb`). The device-key custody rules go into the acceptance
  criteria of `personal-cfo-1zb`, and the retry-order rule into those of
  `personal-cfo-v98vd`.
- Code changes this ADR implies are separate beads, not part of this change: the
  UUIDv4 idempotency-key switch and its enforcement (`personal-cfo-g3m.6`), the
  new `household_timezone` command, and a deferred upgrade to a hybrid KEM.
- The test-vector set pins the bytes that feed Ed25519 and HPKE but cannot run
  those primitives yet; their known-answer tests are owed by the implementing
  beads.
- If every enrolled device is lost, Sync cannot be recovered in v1 (ADR 0074
  Decision 10); the specification names the typed outcome.

## Revisit if

- The planned external cryptographic review (`personal-cfo-5kua`) finds a fault in
  the framing, the signed epoch record or the key custody rules.
- A quantum-capable adversary becomes a credible threat to sealed key blobs: adopt
  a hybrid suite at a new key epoch.
- Canonical JSON proves inadequate for a payload that needs a number type.

## Linked beads

- `personal-cfo-klr.4` (this ADR and the format authority)
- `personal-cfo-vn91` (portable export and import)
- `personal-cfo-1cltt` (Sync genesis snapshot and key epochs)
- `personal-cfo-v98vd` (Sync service)
- `personal-cfo-1zb` (device-key store)
- `personal-cfo-g3m.6` (UUIDv4 idempotency keys)
- `personal-cfo-j8ab7` (ADR 0074)
