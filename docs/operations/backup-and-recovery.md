# Backup & recovery

How to back up your DohFlow vault and restore it on another machine. This
is the **manual recovery procedure** the Week-8 safety gate requires; it is
exercised automatically by the restore-drill regression test
(`personal-cfo-7pfu`, `crates/finance-kernel/tests/restore_drill.rs`), which runs
in CI on every change to the backup/restore or vault-crypto code.

## What a backup is

A backup is a **single encrypted file** (`*.pcfobk`) containing your entire vault
— accounts, transactions, income, bills, and document attachments. It is sealed
with **your vault password** using the same key model as the vault itself (ADR
0002 / ADR 0024): a random per-backup key wrapped by an Argon2id key derived from
your password. The only thing in the file that is *not* encrypted is a small
header (format version + KDF parameters); it contains no financial data.

> **There is no cloud, and no password reset.** The backup is only as recoverable
> as your password. If you lose the password, the backup cannot be opened. Store
> the password somewhere safe and separate from the backup file.

## Make a backup

1. Unlock your vault and open the **Backup** tab.
2. Enter your vault password and choose **Export backup…**.
3. Pick where to save the `.pcfobk` file in the system Save dialog.

Keep the backup somewhere durable and ideally off the machine (an external drive
or a synced folder). Re-export after meaningful changes.

## Restore on a new machine

1. Install DohFlow and launch it. On a machine with no vault you'll see the
   **Create your vault** screen.
2. Choose **Restore from backup…** and select your `.pcfobk` file in the Open
   dialog.
3. Enter the password the backup was created with.

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
