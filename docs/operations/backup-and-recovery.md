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

> **There is no cloud, and no password reset.** The backup is only as recoverable
> as your password. If you lose the password, the backup cannot be opened. Store
> the password somewhere safe and separate from the backup file.

## Make a backup

1. Unlock your vault and open the **Backup** tab.
2. Choose **Export backup…**.
3. Pick where to save the `.pcfobk` file in the system Save dialog.

Keep the backup somewhere durable and ideally off the machine (an external drive
or a synced folder). Re-export after meaningful changes.

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

## Guarantees

- **Byte-identical restore.** The restored vault reproduces the original's
  canonical state exactly — the same accounts, ledger, transactions, income,
  bills, read-model checksums, and decrypted attachment bytes. This is what the
  restore drill asserts.
- **Verify-then-install.** Restore decrypts and checks all content hashes in
  memory first; any mismatch or wrong password aborts with nothing written.
- **No silent downgrade.** A backup from a newer app version is refused rather
  than corrupted; an older-schema backup is migrated forward on open.

## Recommended practice: a periodic restore drill

Treat a backup as untested until you have restored it. Periodically restore your
latest backup into a throwaway location and confirm it opens — the app's automated
drill does exactly this on every release, and you should do the manual equivalent
before relying on the app for important data.
