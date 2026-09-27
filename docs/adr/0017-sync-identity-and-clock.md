# ADR 0017 — Sync identity and clock

- **Status:** Accepted
- **Tier:** Public (ADR 0082)
- **Date:** 2026-09-27
- **Bead:** `personal-cfo-6kn`
- **Decider:** Owner, via the accepted [ADR 0074](0074-dohflow-sync-architecture.md), Decisions 8–9
- **Builds on:** ADR 0002 (vault custody), [ADR 0013-A](0013-id-strategy.md)
  (independent derived row IDs), and ADR 0074 (Sync protocol and CAS order)
- **Consumers:** `personal-cfo-822b` (operation-log documentation),
  `personal-cfo-klr.4` (ADR 0077 wire formats), `personal-cfo-v98vd` (S2-1
  service), and `personal-cfo-elciw` (S3-1 client)

## Context

The shipped local vault is not yet a Sync replica. Its `vault_metadata.vault_id`
is seeded in `crates/db-worker/src/lib.rs`; `vault_meta.node_id` is created by
`ensure_node_id` in the same file and lives in the vault database. Local backup
restore copies the logical vault ID, as
`apps/desktop/src-tauri/tests/restore_as_new_vault.rs` asserts. A restored
local vault may therefore share vault contents and IDs with its source. That
is **current local restore behavior**, not permission to reuse a future Sync
device identity or nonce state.

The operation log already contains `node_id` and `hlc_timestamp`
(`crates/db-worker/src/lib.rs`, `BASELINE_UP`), but neither is a Sync enrollment
identity. `Inner::next_hlc` computes `max(last_hlc + 1, wall_ms)` for the local
process; on open, `current_max_hlc` initializes it from the largest stored
value. Despite the historical field name and comments, it does not receive
remote clock observations and is not a distributed hybrid logical clock.

ADR 0074 Decisions 8–9 fix the Sync key, membership, nonce, restore, and
compare-and-swap (CAS) boundaries. This ADR elaborates those accepted choices;
it does not introduce a new clock, storage format, or runtime behavior. The
earlier ADR 0017 scope proposed specifying a wall-clock-plus-logical HLC
algorithm. That clause was withdrawn: CAS makes the service sequence the
authoritative order, while the shipped field is only a local monotonic
counter. Describing it as a cross-device HLC would promise ordering it cannot
provide.

## Decision

### 1. Three distinct identities

`vault_id` is the logical vault identity in `vault_metadata.vault_id` and the
Sync service's storage key. A local restore preserves it. `node_id` remains an
identity of the vault database, created if missing and copied with a local
snapshot/restore; it does not authorize a Sync device. Sync's `device_id` is
minted **at enrollment**, separately from both IDs. It is bound to a
client-generated device encryption public key and an immutable
`device_key_fingerprint`. The corresponding private key, `device_id`, service
credential, and nonce anti-rollback witness are device-only state: none enters
the vault, logical Sync snapshot, or backup. This separation means a restored
copy cannot impersonate the original enrolled device merely by copying vault
bytes.

The initial Sync device creates the genesis membership record locally. Every
later recipient addition or replacement must be signed by an existing
authorized device and verifiable by clients against the authorized membership
chain. The service stores and enforces the membership and its own revocable
credential, but cannot create a recipient or substitute a public key or
fingerprint. A fingerprint cannot be reused by another enrollment. ADR 0074
Decision 8 governs epoch-root sealing, revocation, and fresh-key cutover;
S2-1 implements service enforcement. Enrollment never gives the service a
decryption key or authority over nonce allocation.

### 2. Restore, re-enrollment, and nonce state

Each device keeps a durable `next_counter` register keyed by
`(key_epoch, device_key_fingerprint, object_type)`. It advances and persists
**before** encryption. An independent, device-only anti-rollback witness must
agree with that state. An absent, restored, or inconsistent witness produces
typed `NonceStateLost`; the client must not encrypt again under that context.
It re-enrolls with a fresh `device_id`, key pair, and fingerprint, or uses a
fresh epoch under ADR 0074's authorized cutover. Neither a restored counter
nor a service-provided counter can repair the old context. The domain and
counter are client-derived/owned under ADR 0074 Decision 8; ADR 0077 specifies
their canonical wire encoding. A crash may burn a counter, and an exact-byte
retransmission uses the saved ciphertext; the service's duplicate check is
only a backstop, never the source of nonce uniqueness.

For R39, restoring a vault also creates a fresh Sync device identity and
nonce context before any new Sync encryption. If the service already knows
its preserved `vault_id`, Sync enablement enrolls this instance as a **replica**,
not a second genesis. It bootstraps from the latest accepted snapshot and
tail, retains local edits in the outbox, and runs the ordinary
rollback-then-replay rebase of ADR 0074 Decision 2. It never resumes the old
device's nonce context or silently drops local edits. This is a future Sync
enrollment contract; it does not change today's local restore flow or assign
future Sync enrollment to the active restore implementation.

### 3. Clock and order

The existing `hlc_timestamp` remains `max(last + 1, wall_ms)`, a local
per-process monotonic counter initialized from the stored maximum on open.
It may support display of approximate local chronology and a deterministic
**non-authoritative** tie-break where no safety or merge decision depends on
it. It cannot establish cross-device causality, decide which concurrent edit
wins, reorder accepted envelopes, or justify last-write-wins merging. Clock
skew, offline work, and restore twins make those uses unsound.

Under ADR 0074's CAS protocol, the service-assigned `seq` is the total order
of accepted envelopes for a `vault_id`. A stale `base_seq` triggers pull and
rebase, not a timestamp comparison. The single-device operation-log behavior
is unchanged. ADR 0013-A independently specifies deterministic IDs for rows
minted during command application; neither `hlc_timestamp` nor `device_id`
replaces that ID derivation.

## Required downstream verification

These are implementation tests for the owning Sync beads, not tests that can
run against the current local-only app:

| Scenario | Expected invariant and owner |
|---|---|
| Two local restores from one backup enroll against the same known `vault_id` | Each gets a distinct `device_id`, key, and fingerprint; neither publishes a second genesis or resumes the original nonce context; both bootstrap/rebase without dropping retained edits. S3-1 (`personal-cfo-elciw`), with the two-vault simulator. |
| Witness absent or inconsistent after restore/crash/rollback | Encryption fails closed as `NonceStateLost` before any old-context invocation; no service response can authorize reuse. S3-1. |
| Authorized re-enrollment after `NonceStateLost` | Existing member signs a verifiable replacement, fresh identity/key/fingerprint is used, and future objects use a fresh context. S3-1 and S2-1 (`personal-cfo-v98vd`). |
| Attempt to reuse a fingerprint or substitute an unsigned recipient | Client rejects the membership change; service rejects enforcement, with no epoch root sealed to the impostor. S3-1 and S2-1. |
| Skewed clocks and concurrent pushes | Accepted `seq`/CAS order and ADR 0073 disposition are unchanged by `hlc_timestamp`; stale-base work rebases or queues. S3-1, S2-1, and the two-vault simulator. |

`personal-cfo-822b` documents the resulting operation-log distinction;
`personal-cfo-klr.4` (ADR 0077) owns canonical membership/nonce wire fields
and compatibility rules. This ADR does not implement or serialize them.
ADR 0073 already defines disposition against the CAS base and **does not
depend on this ADR**.

## Consequences

- A copied vault identity is safe only because authorization and nonce state
  are outside the copied vault. A second local copy cannot be treated as the
  first device merely because its `node_id` or `vault_id` matches.
- Missing or rolled-back device state costs a fresh enrollment or epoch rather
  than risking AES-GCM key/nonce reuse. Offline recovery needs an authorized
  remaining device or ADR 0074's permitted fresh-epoch path; Sync v1 has no
  recovery escrow.
- S2-1 implements signed-membership/credential enforcement; S3-1 implements
  enrollment, nonce-state durability and fail-closed recovery. ADR 0077 fixes
  their interoperable encoding. These changes are downstream work, not
  changes to the current vault, schema, or restore behavior.

## Rejected alternatives

- **HLC as the ordering primitive:** the service already assigns a total
  sequence under CAS, while the local field does not incorporate remote
  observations.
- **Timestamp-only ordering:** skewed or offline device clocks cannot safely
  order financial intent.
- **Lamport clocks:** a logical clock alone loses useful wall-clock locality
  without solving conflict disposition or replacing CAS order.
- **Ledger-row CRDTs:** financial commands require validation and intent-aware
  queueing; merging row values can break ledger invariants (ADR 0073).
- **Service-controlled enrollment:** an equivocating service could add its own
  decryption recipient or replace an honest device's fingerprint.
- **Restoring nonce state:** copied or rolled-back counters can repeat a
  `(derived AEAD key, nonce)` pair; only a fresh context is safe.

## Revisit if

External cryptographic review of the Sync protocol finds the membership or
anti-rollback contract insufficient, or a future protocol replaces the CAS
sequencer. Such a change needs its own reviewed decision before runtime work.
