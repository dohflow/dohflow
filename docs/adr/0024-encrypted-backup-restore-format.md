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

## Revisit if

- Incremental/differential backups are needed (this ADR is full-snapshot only).
- A backup must be openable by a separate recovery key (the `§6.5.1` reserve slot,
  ADR 0002) in addition to the password.
- Cross-device sync (ADR 0017) introduces a streaming/chunked transport that
  should share the manifest/verification model.

## Test coverage

- `personal-cfo-ef3`: export produces a single package; CI byte-scan finds no
  plaintext fixture values.
- `personal-cfo-au3`: restore into a fresh instance; content-hash equality across
  the canonical tables + blobs; wrong-password fails closed; newer-schema refused.
- `personal-cfo-7pfu`: fresh-clone restore-drill regression (the gate's manual
  recovery instructions, executed).
- `personal-cfo-c545`: a backup one schema version old restores + migrates.

## Linked beads

- `personal-cfo-nf18` (this ADR)
- `personal-cfo-ef3` (encrypted backup export v1)
- `personal-cfo-au3` (encrypted backup restore + verification)
- `personal-cfo-7pfu` (restore-drill regression test)
- `personal-cfo-b7jk` (ADR 0023 attachment store — the blob store backup bundles)
- `personal-cfo-c545` (migration tests — older-backup restore path)
- `personal-cfo-2lm` (vault_metadata — schema/envelope versions in the manifest)
