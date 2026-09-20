# ADR 0024: Encrypted backup/restore format

- **Status:** Accepted
- **Date:** 2026-06-20
- **Deciders:** Project owner
- **Bead:** [`personal-cfo-nf18`](../../.beads/issues.jsonl)
- **Related plan sections:** §6.2, §19.5
- **Supersedes:** None

## Context

The Week-8 safety gate requires an encrypted backup that exports to a single
file and restores into a **fresh app instance**, reproducing the original vault's
logical content with provenance + content-hash equality (`personal-cfo-ef3`,
`-au3`, `-7pfu`). The vault is two on-disk artifacts — `vault.db` (SQLCipher) and
`blobs/` (ADR 0023) — plus the key envelope (ADR 0002). A portable backup must
carry all three so a restore needs **only the password**, not the original
machine's Keychain or envelope file. This ADR fixes the package format and
restore semantics so `ef3`/`au3` are pure execution.

## Decision

### 1. One self-contained, password-encrypted container

A backup is a **single file**. Internally it is a small **plaintext header**
followed by an **AEAD-encrypted payload**:

```
header (plaintext, no financial data):
  magic "PCFOBK", format_version, kdf = {argon2id, salt, params},
  wrapped_backup_key, aead_alg, nonce
payload (AES-256-GCM, key = backup DEK):
  manifest.json
  vault_envelope            (wrapped vault DEK + KDF params, per ADR 0002)
  vault.db                  (SQLCipher ciphertext, as-is)
  blobs/<storage_id> …      (per-blob ciphertext, as-is, ADR 0023)
```

The payload is encrypted with a random **backup DEK** wrapped by a KEK derived
from the user's password (Argon2id, **fresh per-backup salt**) — mirroring ADR
0002's `password → KEK → DEK` pattern so there is one key model to reason about.
The header is the only plaintext and carries no financial data, only the
parameters needed to derive the backup key.

### 2. Inner artifacts ship as-is (no decrypt-then-re-encrypt on export)

`vault.db` and the `blobs/` files are copied into the payload **already
encrypted** (SQLCipher / per-blob AEAD). Export never decrypts financial data;
the outer container is defense-in-depth + metadata-hiding (sizes, blob count,
manifest) over already-ciphertext contents.

### 3. Manifest = the verifiable index

`manifest.json` (inside the encrypted payload) carries:

- `format_version`, `created_at`, `backup_id` (UUIDv7) — provenance.
- `app_version`, `schema_version` (DB schema), `vault_envelope_version`,
  `redaction_policy_version` — compatibility surface.
- `content_hashes`: SHA-256 of the **ciphertext** of each component — the
  `vault.db` file, the envelope sidecar, and a `blob_inventory`
  (`storage_id → size → ciphertext_sha256`). The keyed `storage_id` (an
  `HMAC-SHA256` of the blob plaintext, ADR 0023) is itself a plaintext-identity
  check, so blob plaintext need not be decrypted during backup.
- A manifest self-hash so tampering with the index itself is detectable.

**Why ciphertext, not plaintext, hashes (refined 2026-06-20).** §2 ships the
inner artifacts as-is and §5 restores them verbatim — there is **no
re-encryption**, so the restored `vault.db` is *byte-identical* to the original.
Ciphertext SHA-256 is therefore an exact, cheap integrity check, and it avoids
decrypting every blob (and computing a canonical table digest) on every backup.
Plaintext/logical digests only become necessary if a future **re-key-on-restore**
is added — not in MVP; that path would reintroduce them (see Revisit if).

### 4. "Byte-for-byte" is literal under as-is bundling

Because the vault files ship and restore verbatim (§2, §5), the gate's
"reproduces the original vault byte-for-byte" holds **literally**: after restore,
every installed file's ciphertext SHA-256 equals the manifest, so `vault.db`, the
envelope, and every blob are byte-identical to the originals. (If re-key-on-restore
is ever added, this reverts to *plaintext content-hash equality* across
`ledger_transactions`/`ledger_postings`/`accounts`/`recurring_events`/
`income_sources`/`operation_log` + blob plaintext, since re-encryption would
change ciphertext.)

### 5. Restore is atomic: verify, then install into a fresh location

`restore_backup(path, password)`:

1. Derive the backup key from the header + password; fail closed on wrong
   password (distinct, clear error — wrong-password failure is a gate test).
2. Decrypt the payload to a **staging directory** (never the live vault).
3. Check `schema_version`/`app_version`: a backup **newer** than the app supports
   is refused with a clear message (no silent corruption); an **older** backup is
   accepted and migrated post-install (migration framework, `personal-cfo-c545`).
4. Verify every `content_hash` against the staged content; any mismatch aborts.
5. Atomically install the staged `vault.db` + `blobs/` + envelope into a fresh
   vault location. Restore **never** overwrites an existing vault without explicit
   user confirmation. The vault state machine's `RestoringBackup` state (ADR 0002)
   bounds an interrupted restore.

### 6. No plaintext in the package or beside it

The only plaintext is the header (KDF params, versions) — no account numbers,
balances, or filenames. `ef3`'s CI test byte-scans the produced package (and any
sibling temp files) for known fixture values and fails the build on any hit.

### 7. Ownership

A `backup` module orchestrates export/restore, using `vault-crypto` for the outer
Argon2id/wrap/AEAD, `db-worker` for a consistent `vault.db` snapshot (checkpoint
WAL, read-only copy), and the ADR 0023 blob store for `blobs/`. Backup/restore are
kernel commands; the frontend never touches the package bytes directly.

## Consequences

### Positive

- One portable file restores on a fresh machine with only the password.
- Export is cheap and safe — no decrypt of financial data; inner ciphertext is
  copied as-is.
- Plaintext content hashes make "did the restore reproduce the vault?" a precise,
  testable question and underpin the restore-drill regression (`personal-cfo-7pfu`).
- Atomic verify-then-install means a failed/partial restore never harms existing
  data.

### Negative

- A consistent `vault.db` snapshot needs a WAL checkpoint + read-only copy under
  the worker, so export briefly coordinates with the writer.
- Computing a canonical plaintext table digest requires a defined, stable
  serialization (column order, encoding) — specified once, asserted in CI.
- Outer Argon2id on restore adds a deliberate cost; acceptable for a rare action.

## Rejected alternatives

- **Plain (unencrypted) tar/zip of already-encrypted inner files.** ✗ Leaks
  metadata (blob count, sizes, manifest, schema version) and offers no tamper
  envelope; the outer AEAD is cheap defense-in-depth.
- **Re-key everything into one fresh SQLCipher DB on export (fold blobs in).** ✗
  Re-couples large binaries to the DB (against ADR 0023) and forces a full decrypt
  /re-encrypt of all financial data on every backup.
- **Ciphertext-identical backup verification.** ✗ Impossible across re-encryption
  with fresh salts; logical (plaintext-hash) equality is the correct invariant.
- **Restore in place over the live vault.** ✗ A failed restore would destroy the
  user's current data; staging + atomic install is non-negotiable for a
  finance app.
- **Reuse the vault's KEK as the backup key.** ✗ Couples backup portability to the
  live envelope; a fresh per-backup salt/key keeps the backup self-contained and
  independently revocable.

## Addendum A (2026-09-15, personal-cfo-lhouc): unattended backup key model — format_version 2

### Context

Version 1 derives the outer backup KEK from a password supplied at export. A
scheduled job may run while a vault is already unlocked, when the in-memory
vault DEK is available but the master password is intentionally not retained.
Re-prompting for a password would make that job attended; retaining a password,
creating a second backup password, or introducing a Keychain-held backup secret
would weaken the vault model in ADR 0002.

This addendum fixes the version-2 container contract that `personal-cfo-ii3an`
implements. It does not change the v1 parser or the atomic verify-then-install
restore rule in this ADR.

### Decision

#### 1. Derive a distinct backup KEK from the unlocked vault DEK

While the vault is unlocked, the exporter derives a backup KEK with
`HKDF-SHA256`:

```text
backup KEK = HKDF-SHA256(
  ikm  = vault DEK,
  salt = fresh random 16-byte hkdf_salt for this backup,
  info = ASCII "dohflow-backup-kek-v2"
)
```

The fixed `info` string is a domain-separation boundary: it must never be
reused for attachment addressing or another DEK-derived key. The implementation
lives beside the existing backup AEAD helpers as
`vault_crypto::backup::derive_backup_kek`; it uses `Hkdf` over the in-memory
DEK and returns a KEK suitable for the existing `vault_crypto::wrap_dek` /
`vault_crypto::unwrap_dek` primitives.

Export generates a random backup DEK as today, wraps it under that distinct
backup KEK, and seals the framed payload with `vault_crypto::backup::seal`.
Restore performs the inverse chain:

```text
password
  → vault_crypto::derive_kek(header vault-envelope salt + Argon2id params)
  → vault KEK
  → vault_crypto::unwrap_dek(header vault envelope)
  → vault DEK
  → HKDF-SHA256 with the header hkdf_salt and fixed info
  → backup KEK
  → vault_crypto::unwrap_dek(wrapped backup DEK)
  → backup DEK
  → vault_crypto::backup::open(payload)
  → constant-time, byte-exact comparison of the header and payload vault envelopes
  → existing manifest verification and verify-then-install restore
```

The vault password remains the only user secret. No Keychain item, recovery
secret, or password material is added to the backup job, and the job runs only
while the vault is unlocked.

#### 2. Write format_version 2; retain format_version 1 forever

All version-2 integer lengths and versions use the existing big-endian framing.
The plaintext header fields are, in order:

1. `magic` = `PCFOBK`.
2. `format_version` = `u16(2)`.
3. `vault_envelope_len` = `u32`, followed by the serialized vault-envelope
   bytes (envelope version, Argon2id parameters and salt, then the wrapped
   vault DEK nonce and ciphertext).
4. `hkdf_salt` = a fresh 16-byte random salt.
5. `wrapped_backup_dek_nonce` = 12 bytes.
6. `wrapped_backup_dek_ciphertext_len` = `u32`, followed by its ciphertext.
7. `sealed_payload_nonce` = 12 bytes.
8. `sealed_payload_ciphertext_len` = `u64`, followed by its ciphertext.

The same serialized vault-envelope bytes remain inside the encrypted payload,
whose manifest carries their integrity hash. The encrypted payload copy is
authoritative: it is the only envelope installed on a successful restore. The
plaintext header copy is bootstrap material only. Immediately after
`backup::open` authenticates the payload, and before extracting to staging or
performing any filesystem write, `disassemble` checks the bounded raw header
and payload envelope lengths, then compares equal-length byte strings with
constant-time equality. A mismatch is the typed
`BackupError::HeaderEnvelopeMismatch`, never a wrong-password error, and
aborts the restore without creating restore state.

The header copy lets an unattendedly-created backup remain self-contained and
restorable with the vault password; it does not permit unattended restore. The
payload copy preserves the existing byte-for-byte restore property for the
installed envelope sidecar and keeps the existing component-verification model
intact.

`disassemble` accepts format versions 1 and 2 forever. The v1 parser and
its password-derived key chain remain byte-compatible; assembly writes only
v2. Both manual and scheduled export use the unlocked vault DEK and therefore
do not prompt for a password. Restore continues to ask for the vault password.

The v2 manifest gains `manifest_schema_version = 1`
(`personal-cfo-3fdd.15(c)) to describe the manifest field set. It is distinct
from both the outer `format_version` and the existing database
`schema_version`. The v2 parser must reject an unsupported manifest schema
value, a missing required field, or an unknown field with a clear error rather
than defaulting it. The v1 parser keeps its legacy manifest grammar so existing
v1 containers remain restorable.

Before invoking Argon2id, both parsers apply the versioned backup KDF admission
policy. For parameter-schema version 1, the build uses its fixed Argon2id
version `0x13`, permits only parallelism `1`, and accepts exactly one of the
three profile tuples already documented in `docs/security/encryption-design.md`:
LegacyCompatibility
(`19_456 KiB`, time cost `2`), InteractiveDefault (`65_536 KiB`, time cost
`3`), or HighSecurity (`262_144 KiB`, time cost `4`). This is a bounded
allowlist, not merely a lower floor: the largest accepted memory cost is
`262_144 KiB` and the largest accepted time cost is `4`.

The v1 parser validates its plaintext KDF fields, and the v2 parser validates
the parsed header vault envelope, before `derive_kek`, Argon2 allocation or
work, staging, or any vault write. An unsupported parameter-schema version or
resource tuple returns the typed `BackupError::UnsupportedKdfParameters`; this
is distinct from `BackupError::Crypto` for a wrong password. A malformed or
tampered envelope, unknown algorithm, HKDF salt, wrapped backup key, or sealed
payload likewise fails closed.

#### 3. Phone containers are a separate, non-restorable destination

Decision D24 is recorded here for the later mobile work: a phone container is
not a user-restorable backup. `personal-cfo-28js0` builds it, never strips an
existing SQLite file (a `DELETE` could leave ciphertext in freed pages), and
uses only the class-1 and class-2 allowlist from `personal-cfo-w21q7` (CLASS-0).
It is sealed with HPKE to a per-phone, Secure-Enclave-backed public key enrolled
by QR; the vault password never leaves the Mac. Revocation means ceasing to
seal future phone containers to that key. An unclassified table must visibly
fail to reach the phone rather than silently leak there.

### Rejected alternatives

- **Reuse the vault's KEK as the backup key.** The original rejection still
  applies. Version 2 does not reuse the vault KEK: it derives a distinct backup
  KEK from the vault DEK with HKDF, a per-backup salt, and a separate domain.
  The vault envelope travels in the header, so the container remains portable.
  A later live-vault rekey does not orphan an old backup: that backup carries
  the envelope that was current when it was made and opens with the password
  current at that time.
- **Keychain-held backup key.** ✗ This would make a portable backup depend on a
  macOS-only convenience layer that the backup path does not otherwise use.
- **A second backup password.** ✗ It creates another secret the user can lose
  without improving the vault's recovery model.
- **Plaintext backup DEK at rest.** ✗ It would let possession of a package
  defeat the vault's encryption boundary.

### Consequences

- `personal-cfo-ii3an` (BACK-0b) implements this container contract, including
  `export_unattended`, the no-password manual export surface, v1 compatibility,
  and the manifest grammar.
- The header exposes the same wrapped vault-envelope material already stored
  beside `vault.db`; it adds no plaintext financial data or usable key material.
  Its deterministic serialization is nevertheless a stable fingerprint: an
  observer of ciphertext objects can correlate version-2 backups made before a
  password rewrap/rekey and can match one to the existing vault-envelope
  sidecar; a rewrap/rekey ends and reveals such epochs. This accepted
  linkability consequence preserves the self-contained restore contract.
  BACK-0b updates the threat model's asset A4 and "Backup theft" row.
- `personal-cfo-8qh` consumes unattended export for the local job. A
  user-chosen cloud-synced folder sends ciphertext through that user's own
  sync client; it is not an app-managed upload. That bead owns the corresponding
  public privacy and site-privacy language for both this metadata linkability
  and the user-selected cloud-folder ciphertext egress.
- BACK-0b updates `docs/operations/backup-and-recovery.md` and
  `docs/user-guide/recover-a-vault.md` to remove the export password prompt;
  `personal-cfo-klr.4` records the resulting format in the future vault-format
  specification.
- CLASS-0 is a hard input to the phone-container builder, not an implicit data
  export rule.

### Revisit if

- The external cryptographic review (`personal-cfo-5kua`) finds a fault in the
  HKDF construction, header framing, or threat model.
- ADR 0002 changes `vault_envelope_version` or its serialization; the backup
  header must then dispatch explicitly on that version.
- A stronger or per-device-calibrated KDF needs values outside the current
  parameter-schema-1 allowlist; it must add a reviewed, explicitly versioned
  parser policy rather than silently widen these restore-time resource bounds.
- A cloud destination needs a different transport contract; it should reuse
  this self-contained container rather than create a second backup format.
- Sync-key rotation (ADR 0074) needs a separate rotation story; it must not
  silently redefine this backup key hierarchy.

## Revisit if

- Incremental/differential backups are needed (this ADR is full-snapshot only).
- A backup must be openable by a separate recovery key (the `§6.5.1` reserve slot,
  ADR 0002) in addition to the password.
- Cross-device sync (ADR 0017) introduces a streaming/chunked transport that
  should share the manifest/verification model.
- The format-version-2 unattended key model or its header needs to change (see
  Addendum A and its explicit review triggers).

## Test coverage

- `personal-cfo-ef3`: export produces a single package; CI byte-scan finds no
  plaintext fixture values.
- `personal-cfo-au3`: restore into a fresh instance; content-hash equality across
  the canonical tables + blobs; wrong-password fails closed; newer-schema refused.
- `personal-cfo-7pfu`: fresh-clone restore-drill regression (the gate's manual
  recovery instructions, executed).
- `personal-cfo-c545`: a backup one schema version old restores + migrates.
- `personal-cfo-ii3an` (BACK-0b): a checked-in v1 fixture restores; v2 completes
  `restore_drill_reproduces_the_canonical_state`; an ordinary v2 round trip is
  retained; wrong-password failure occurs before any write; and the backup KEK
  is domain-separated from the attachment key for the same vault DEK. A v2
  attack test uses `rewrap_envelope` to produce two valid envelopes for the
  same DEK, splices one into the other package's header, and asserts
  `BackupError::HeaderEnvelopeMismatch` before staging or any target write.
  Doctored v1-header and v2-envelope fixtures below the floor or above the
  version-1 allowlist each return `BackupError::UnsupportedKdfParameters`
  before Argon2 work or any write.

## Linked beads

- `personal-cfo-nf18` (this ADR)
- `personal-cfo-lhouc` (Addendum A: unattended backup key model)
- `personal-cfo-ef3` (encrypted backup export v1)
- `personal-cfo-au3` (encrypted backup restore + verification)
- `personal-cfo-7pfu` (restore-drill regression test)
- `personal-cfo-b7jk` (ADR 0023 attachment store — the blob store backup bundles)
- `personal-cfo-c545` (migration tests — older-backup restore path)
- `personal-cfo-2lm` (vault_metadata — schema/envelope versions in the manifest)
- `personal-cfo-ii3an` (BACK-0b format-version-2 implementation)
- `personal-cfo-3fdd.15` (Argon2id floor and manifest schema hardening)
- `personal-cfo-8qh` (scheduled backup job)
- `personal-cfo-w21q7` (CLASS-0 table-class manifest)
- `personal-cfo-28js0` (MOB-0a phone-container builder)
- `personal-cfo-5kua` (external cryptographic review)
