# Backup & recovery

How to back up your DohFlow vault and restore it on another machine. This
is the **manual recovery procedure** the Week-8 safety gate requires; it is
exercised automatically by the restore-drill regression test
(`personal-cfo-7pfu`, `crates/finance-kernel/tests/restore_drill.rs`), which runs
in CI on every change to the backup/restore or vault-crypto code.

## What a backup is

A backup is a **single encrypted file** (`*.pcfobk`) containing your entire vault
— accounts, transactions, income, bills, and document attachments. Format v2
uses a random per-backup key wrapped by an HKDF-SHA256 key derived from the
unlocked vault's in-memory DEK (ADR 0024 addendum A). Export does not ask you to
re-enter or retain your password. Restore still requires the vault password to
unwrap the vault DEK carried in the backup.

The plaintext header contains no financial data. It includes the same wrapped
vault envelope already stored beside `vault.db`; that envelope is not a usable
key without your password. Its repeated fingerprint does let someone who can
see the backup and sidecar correlate them, and lets them correlate backups made
before the next password rewrap/rekey.

> **There is no DohFlow cloud backup and no password reset.** If you choose a
> cloud-synced folder, your own provider's client carries the encrypted file;
> DohFlow does not upload it. The backup is only as recoverable as your
> password. If you lose the password, it cannot be opened. Store the password
> somewhere safe and separate from the backup file.

## Make a backup

1. Unlock your vault and open **Settings → Backups**.
2. Choose a destination folder and a cadence: off, daily, weekly (the default),
   or monthly. The schedule is saved for this vault.
3. Choose **Save backup settings**. A due backup runs at the next unlock; it
   never blocks the unlock or asks for your password again.
4. Choose **Back up now** to create and verify a backup immediately in the
   selected folder. The card shows the most recent verified backup and terminal
   scheduled-job errors.

Scheduled and one-off backups are kept until you remove them yourself. DohFlow
does not automatically prune or delete older backup files. To free space, use
Finder or your file manager to remove the copies you no longer need. You may also
use **Export backup…** in the same Settings card for a one-off backup in a
separately chosen file.

For an off-device copy, you can select a folder managed by iCloud Drive, Dropbox,
or another sync client. If that folder is synced, the provider's own client sends
the encrypted backup off your Mac as ciphertext. The v2 header's stable
vault-envelope fingerprint allows someone who can see the ciphertext to
correlate backups made before a password rewrap or rekey.

## Restore on a new machine

1. Install DohFlow and launch it. On a machine with no vault you'll see the
   **Create your vault** screen.
2. Choose **Restore from backup…** and select your `.pcfobk` file in the Open
   dialog.
3. Enter the vault password that was current when the backup was created.

The app verifies the backup's integrity (every component's content hash) **before
writing anything**, restores the vault into a fresh location, and opens it. A
wrong password is rejected before any file is written, and an existing vault is
never overwritten.

## Restore while a vault is already on this device

When the launch picker shows a locked vault, choose **Restore as a new vault…**.
If the selected vault's database or envelope is incomplete, the recovery screen
offers the same action. Select the `.pcfobk` file, give the restored copy a
nonblank name, and enter the password that protected that backup. DohFlow creates
a separate app-managed vault, verifies and opens it, then makes it the active
vault. Any existing registry entry for the original vault remains, and its
database, envelope, and attachments are retained unchanged. A failed restore
leaves the original selection in place. You do not need to delete the old vault
to recover access.

If a restore is interrupted, the launch or recovery screen may report an
unregistered vault folder. DohFlow does not open or silently remove that folder
on restart; keep it for diagnosis. The original vault and backup file remain
available. A successful new restore can be attempted separately.

## Guarantees

- **Byte-identical restore.** The restored vault reproduces the original's
  canonical state exactly — the same accounts, ledger, transactions, income,
  bills, read-model checksums, and decrypted attachment bytes. This is what the
  restore drill asserts.
- **Verify-then-install.** Restore decrypts and checks all content hashes in
  memory before writing vault data. Restoring alongside an existing vault first
  reserves a fresh empty slot and marks the attempt so an interruption can be
  diagnosed. A mismatch or wrong password never replaces the original vault.
- **No silent downgrade.** A backup from a newer app version is refused rather
  than corrupted; an older-schema backup is migrated forward on open.

## Existing-vault compatibility gate

Ordinary unlock and restored-vault reopening share the db-worker guard; the
optional health check is not the gate. With the ADR 0070 cooperative runner
lock held, an existing-only read-only SQLCipher connection inspects a single
read transaction before a writable connection is opened. It reads committed
WAL state, not just the main database page. It independently checks
`PRAGMA user_version`, `schema_migrations.version`, and
`vault_metadata.schema_version`; credible newer evidence in any marker is a
typed refusal even if another marker is missing, damaged, or lowered.

Caller inventory: `Kernel::open` uses the passphrase open-or-create API;
`Kernel::create_vault` uses raw-DEK creation (preflight if a DB already exists);
`Kernel::unlock_vault` uses the existing-only raw-DEK API. Controller/IPC unlock
and backup restore's verified-payload reopening reach that same unlock gate.
Worker read/maintenance connections retain its ownership boundary and omit
SQLite's CREATE flag, so a disappearing path is never a replacement vault.
No alternate credential-based vault unlock surface is added.

The guard also verifies known migration hashes, a contiguous applied prefix,
and readable existing metadata. The supported historical matrix is the known
versioned migration prefixes, including lower/zero stale stamps and completed
per-migration transactions interrupted before the next migration. No verified
pre-framework/no-tracker fixture exists; such layouts fail closed rather than
being guessed to be the baseline. Missing DB/metadata on existing unlock is
not creation. Genuinely new vault creation still bootstraps normally.

The runner lock remains held across inspection, pathname file-identity checks,
writer-handle revalidation, and bootstrap. On supported macOS/Unix targets,
identity checks compare filesystem device/inode to the held original file.
This is a cooperative single-owner boundary, not protection against malicious
non-cooperating writers or pathname swaps. Before `configure_conn` requests
WAL or `ensure_tracker` creates a table, the writer handle is rechecked;
the migration runner independently rechecks all version witnesses before any
tracker/migration/version-stamp write.

Refusal leaves no usable worker/kernel and does not start forecast persistence
or local jobs. It does not write the DB, envelope, or blobs or checkpoint,
truncate, delete, or lose existing committed WAL data. SQLite may create
previously absent WAL/SHM coordination files or rebuild its disposable SHM
index, and the advisory runner lock file may exist afterward; file-set equality
is deliberately not claimed. No immutable/no-WAL shortcut, engine/crypto
format change, automatic repair, or downgrade is performed.

Synthetic evidence lives in `db-worker`'s newer-marker/layout/WAL and historical
prefix tests, `raw_key_vault`, and the desktop `vault_commands` refusal tests.
Tests compare settled encrypted DB/envelope/blob bytes and committed WAL bytes,
retain independent SQLCipher observers, and verify locked state, repeated
refusal, and other-vault preservation. Never reproduce this by stamping a real
vault. Keep its files intact and obtain a separately approved recovery plan.

## Recommended practice: a periodic restore drill

Treat a backup as untested until you have restored it. Periodically restore your
latest backup into a throwaway location and confirm it opens — the app's automated
drill does exactly this on every release, and you should do the manual equivalent
before relying on the app for important data.
