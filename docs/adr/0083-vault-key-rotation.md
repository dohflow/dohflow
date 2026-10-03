# ADR 0083 — Vault key rotation (DEK rekey)

- **Status:** Proposed
- **Tier:** Public
- **Date:** 2026-10-03
- **Bead:** `personal-cfo-2y8`
- **Related:** ADR 0002 (local encrypted vault — this ADR fills in its
  "key rotation" consequence), ADR 0023 (encrypted attachment store),
  ADR 0024 and 0024-A (backup format; a backup carries its own envelope),
  ADR 0074 (Sync uses a separate key, so rotation stays local),
  `personal-cfo-g3m.5` (bundled SQLCipher engine remediation)

## Context

ADR 0002 gives the vault two layers of keys: a password-derived KEK wraps a
random 256-bit DEK, and the DEK keys SQLCipher and wraps every attachment's
content key. Changing the password (`personal-cfo-zxq`) only re-wraps the
**same** DEK under a new KEK. That is cheap, but it does nothing if the DEK
itself is exposed — for example by a memory dump of the unlocked app or a
leaked diagnostic. ADR 0002 says rotation of the DEK is supported, but no
decision records **how** it works.

Rotating the DEK is not a single write. The DEK is used in four places, and all
four have to move together:

1. **The envelope sidecar** (`vault.db.envelope`) holds the wrapped DEK. It is
   the only place the wrapped DEK is stored.
2. **`vault.db`** is keyed with the raw DEK (`PRAGMA key = "x'…'"`), so the
   whole database has to be re-encrypted.
3. **Attachment rows** hold each content key wrapped directly by the DEK
   (`attachments.wrapped_content_key` / `content_key_nonce`).
4. **Attachment file names** (storage IDs) are
   `HMAC-SHA256(HKDF-SHA256(DEK, info = "personal-cfo/attachment-addressing/v1"), plaintext)`.
   (ADR 0023 §2 quotes the context as `"attachment-addr"`; the code's string
   above is the authoritative one.) A new DEK therefore gives every blob a
   different name.

Two of these are separate files that cannot be replaced together in one atomic
filesystem operation. If a crash left `vault.db` re-encrypted under the new DEK
while the sidecar still wrapped the old one, the vault would never open again —
total data loss. Startup today only checks whether `vault.db` and its sidecar
exist (`classify_vault`); there is no marker or recovery path. A rotation needs
a defined commit point and a recovery routine that cannot leave a mismatched
pair behind.

Outside these four, nothing else depends on the DEK in a way rotation must
rewrite:

- Connector credentials live inside SQLCipher, so re-encrypting the database
  covers them.
- Backups derive their key from the DEK at backup time and carry their own
  envelope (ADR 0024-A), so old backups stay valid.
- Sync has its own key (ADR 0074).
- Touch ID / Keychain unlock is not implemented.

## Decision

### 1. Rotation is an explicit, password-confirmed action

The user starts rotation from a **"Rotate encryption key"** card in the
Settings vault section. It asks for the current password. The KEK is never kept
in memory, so the password is needed to wrap the new DEK, and asking for it
also confirms the user's intent.

A rotation:

- generates a fresh random 256-bit DEK from the OS CSPRNG;
- re-wraps it under a KEK derived with the **current** Argon2id profile
  (`InteractiveDefault`) and a fresh salt. A vault still on
  `LegacyCompatibility` parameters moves forward as a side effect
  (`docs/security/encryption-design.md`);
- leaves the password unchanged.

The Tauri command is `rotate_vault_key`. It is added to the app command ACL
like `change_password`. No key material crosses the IPC boundary; the
password travels as a zeroizing string, as it does for `change_password`.

### 2. Prepare everything beside the live vault, then commit with one rename

The live vault is never converted in place. Rotation runs in the existing
`Rekeying` state and has three phases.

**Prepare** (the live files are only read):

- Checkpoint the WAL, then copy the database into a sibling file
  `vault.db.rekey-new`, keyed with the new DEK, using SQLCipher's export
  (`ATTACH … KEY` + `sqlcipher_export`). The copy must carry `PRAGMA
  user_version` and every other version marker exactly; the implementation
  sets any marker the export does not copy, and a test asserts they match.
- Inside that copy, in one transaction, for every attachment row:
  - re-wrap the content key under the new DEK;
  - recompute the storage ID under the new addressing subkey (§3).
- **Finalize the copy as one closed file.** All of the copy's writes must be in
  `vault.db.rekey-new` itself, never stranded in side files that a later rename
  would leave behind. Before anything hashes or renames it:
  - every write to the copy uses rollback-journal mode (`journal_mode = DELETE`),
    or is checkpointed with `PRAGMA wal_checkpoint(TRUNCATE)` before closing;
  - every connection to the copy is closed;
  - no `-wal`, `-shm` or `-journal` file exists beside it (asserted).

  The normal vault open re-enables WAL once the file is in place, as it does
  for every vault.
- Write a journal file `vault.db.rekey` in state `preparing`, **before any blob
  link exists**. It lists:
  - every new blob name that prepare is about to create;
  - every old blob name.

  Blob names are opaque HMACs that are already visible in a directory listing,
  so recording them reveals nothing new. Because the journal lists them first,
  recovery can always find every link prepare created.
- Create each re-addressed blob file under its new name. The blob ciphertext
  does not change, because the content keys do not change, so a hard link to
  the existing file is enough. Fall back to a copy where hard links are
  unavailable.
- Write the new envelope to `vault.db.envelope.rekey-new`.
- Verify before committing:
  - open the copy read-only with the new DEK, without changing its journal mode;
  - `PRAGMA integrity_check` returns `ok`;
  - the schema and version markers match the original;
  - every attachment row unwraps under the new DEK and names a present file.

  Close it again and re-assert that there are no side files. Then `fsync` the
  new files and the vault directory.
- Advance the journal to state `prepared`, now also recording the hashes of the
  finalized new database and the new envelope. Like every journal change, this
  is a write to a temp file, `fsync`, a rename, and an `fsync` of the directory.

**Commit**: atomically replace the journal with state `committed` (write a
temp file, `fsync`, rename, `fsync` the directory). This rename is the **only**
commit point. Before it, the old vault is authoritative; after it, the new one
is.

**Apply** (every step idempotent, so it can be re-run after a crash):

1. Close the kernel. This drops every connection and releases the runner lock.
2. Remove the old database's `-wal` and `-shm` files. This comes before the
   swap so that an old side file can never sit beside the new `vault.db`. Their
   contents are not needed: the export read a checkpointed database, and the
   controller lock (§5) let nothing write to it afterwards. Re-running this
   step during roll-forward is safe: while the journal exists the new database
   has never been opened (it was finalized with no side files and is only
   reopened after the journal is removed), so any side file beside `vault.db`
   belongs to the old database.
3. Move `vault.db` to `vault.db.rekey-old` and move `vault.db.rekey-new` into
   place.
4. Move `vault.db.envelope.rekey-new` over `vault.db.envelope`.
5. Remove the old blob names, skipping any name the journal also lists as new.
6. Remove `vault.db.rekey-old`.
7. Remove the journal last.

Then reopen the vault with the new DEK and return to `Unlocked`.

### 3. Attachments are re-addressed, not only re-wrapped

Re-wrapping alone would leave existing blobs named under the old DEK while new
attachments are named under the new one. Identical files would stop
deduplicating, and the old naming key would still let anyone holding the old
DEK check whether a known file is in the vault. Rotation therefore recomputes
every storage ID under the new DEK's addressing subkey. That means decrypting
each blob once, during prepare, to HMAC its plaintext. Content keys are
re-wrapped, not replaced. Re-encrypting blob contents under new content keys is
out of scope (see Alternatives).

### 4. Startup recovery finishes or undoes an interrupted rotation

`classify_vault` checks for the journal before anything else.

- **No journal:** classify as today, and remove stray `*.rekey-new` files (with
  any side files beside them). These are left from a prepare that crashed before
  writing its `preparing` journal. No blob link can exist yet at that point,
  because links are only created after the journal lists them. Recovery never
  sweeps the blob directory.
- **Journal `preparing` or `prepared`:** roll back. Delete
  `vault.db.rekey-new` (with any side files), the new envelope, and every new
  blob name the journal lists, except a name it also lists as old. Then delete
  the journal. The old vault is untouched and the result is `Locked`.
- **Journal `committed`:** roll forward. Re-run the idempotent apply steps,
  checking what already exists at each step. The result is `Locked`; the next
  unlock uses the new DEK with the unchanged password.
- **Journal present but the files contradict it** (a missing file, or a hash
  mismatch): `CorruptNeedsRecovery`. Nothing is deleted, so every candidate
  file stays available for diagnosis or restore from backup.

A failure **during** prepare (a wrong password, low disk space, a verification
failure) deletes the prepared files and returns the vault to `Unlocked` on the
old DEK with nothing changed. The state machine gains `Rekeying → Locked` for
the case where the kernel has already been closed when apply fails and the
reopen cannot be done safely.

### 5. Preconditions and serialization

- Rotation requires `Unlocked` and holds the vault controller mutex
  (`lock_controller`) from the start of prepare until it has reopened or
  reached `Locked`. **Every** kernel access goes through that mutex: IPC
  commands, scheduled jobs and backups, and every database phase of a
  connector sync. Connector network calls run outside it, but they hold no
  kernel handle. So nothing can read or write the vault while a rotation runs,
  which is what guarantees that no write lands between the export and the
  commit. A connector sync whose network fetch overlaps a rotation waits for the
  mutex and then writes into the rotated vault. The implementation keeps this
  invariant and pins it with a test, e.g.
  `rotation_blocks_kernel_access_and_an_overlapping_sync_lands_in_the_rotated_vault`.
- **What the user sees while other commands wait:** any other IPC command
  blocks until the rotation finishes, because it is waiting on the same mutex.
  That includes `vault_status`, so the frontend cannot observe `Rekeying`
  mid-run, and the existing state-driven busy screen would never appear. The
  rotate card therefore shows its own blocking, app-wide overlay ("Re-encrypting
  your vault…") for as long as its `rotate_vault_key` call is pending. That keeps
  the user from starting other actions, which would only queue. A background
  job, or a connector's database phase, simply runs after the rotation.
- Before prepare, rotation checks free disk space and refuses with a clear
  message if there is not enough. It needs room for one full copy of the
  database plus margin. It first probes whether the blob directory supports
  hard links; if it does not, the requirement also includes the total size of
  the blob files, since every blob would then be copied.
- The progress UI is deliberately coarse ("Re-encrypting your vault…"), and
  the action cannot be cancelled once it reaches commit.

### 6. What rotation protects, and what it does not

After a rotation, the live vault, its envelope and every attachment name and
wrap depend only on the new DEK. Rotation does **not** make data that was
already exposed secret again:

- **Backups** made before the rotation still open with the password that was
  current when they were made (ADR 0024-A). Each one is its own key epoch. A
  user who believes the old DEK leaked should delete the old backups they no
  longer need.
- **Copies** of the old vault taken before the rotation stay readable with the
  old DEK.
- **Blob contents** keep their existing content keys. Someone who already holds
  the old DEK **and** a copy of the old database can still decrypt blob
  contents captured at that time.
- **Removed files** are not overwritten. On SSDs and copy-on-write filesystems,
  data may remain recoverable forensically. Secure erase is not attempted.

The threat model records this as a residual risk.

### 7. Sequencing with the engine remediation

Rotation re-encrypts the whole database with the bundled SQLCipher engine.
Implementation starts after `personal-cfo-g3m.5` lands its engine change, so
the rotation is built and tested on the engine that ships. That bead's
cross-version tests are extended to include a vault that has been rotated.

## Consequences

### Positive

- A suspected DEK exposure has a real remedy that does not need a new password
  or a restore.
- A crash at any point leaves either the old vault or the new vault, never a
  mismatched pair.
- Rotation also moves older vaults to the current KDF profile.
- Attachment deduplication keeps working, and no old-key naming oracle remains.

### Negative

- Rotation needs free space for a full copy of the database, and its run time
  grows with database size and the total attachment bytes, because every blob
  is decrypted once to re-address it.
- A new on-disk file family (`.rekey`, `.rekey-new`, `.rekey-old`) and a
  startup recovery path need tests for every crash point.
- Vault delete and restore cleanup must also remove these files.
- `vault_metadata`'s `kdf_*` columns still do not reflect the real envelope
  parameters. This ADR does not change that.

## Alternatives considered

- **In-place `PRAGMA rekey`.** ✗ The database is re-encrypted in place, so the
  database and sidecar can no longer be committed as one pair. It also adds
  interaction with WAL mode on the engine that `g3m.5` is replacing.
- **Re-wrap content keys only, keep blob names.** ✗ Deduplication breaks across
  key epochs, and the old naming key stays useful to anyone holding the old DEK.
- **A separate, stable addressing key, wrapped by the DEK.** ✗ Cheaper, because
  names never change, but it keeps exactly the old-key oracle that a rotation
  is meant to end.
- **Re-encrypt every blob under new content keys.** ✗ The strongest break from
  the old key, but it rewrites every attachment file and adds no protection
  against someone who already holds the old database.
- **Command only, no UI.** ✗ A recovery action that users cannot reach does not
  help them.

## Revisit if

- Blob contents must be protected against a holder of the old DEK and the old
  database — for example by a compliance requirement or a real incident. That
  calls for re-encrypting the blobs.
- Touch ID / Keychain unlock lands. Its stored unlock material must be refreshed
  or invalidated by a rotation.
- The vault grows large enough that a full database copy no longer fits
  comfortably on typical free disk space.

## Linked beads

- `personal-cfo-2y8` (implementation)
- `personal-cfo-g3m.5` (engine remediation; implementation follows it)
- `personal-cfo-zxq` (password change — the KEK-only re-wrap this extends)
- `personal-cfo-bcj` (attachment store)
- `personal-cfo-tp2ln` (Sync simulator exit test runs a rekey on the origin)
