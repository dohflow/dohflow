# Recovering your vault

A plain-language guide to keeping your DohFlow data safe: what your
password actually protects, how to make a backup, and how to get your data back
on a new machine. (The technical version of this procedure lives in
[Backup & recovery](../operations/backup-and-recovery.md).)

## What your master password protects

Everything. Your accounts, transactions, income, bills, and attached documents
live in a single encrypted **vault** on your computer. The vault is sealed with
your master password: without the password, the file on disk is scrambled data
that nobody — including the app itself — can read.

There is no DohFlow cloud. Your data is never uploaded anywhere, which
also means there is no server that keeps a spare copy for you.

## There is no password reset

This is the single most important thing to understand about DohFlow. The
app shows this warning when you create a vault, and it means exactly what it
says:

> DohFlow has no cloud password reset. If you forget your password, your
> data is unrecoverable. Save your password somewhere safe.

There is no "Forgot password?" link, no recovery email, and no support team
that can unlock a vault. That is a deliberate design choice, not an oversight:
a back door that could recover *your* data could recover it for anyone who
steals your laptop, too. The password never leaves your machine, so nothing
outside your machine can undo it.

Practical advice:

- **Write the password down** or store it in a password manager you trust.
- Keep it **separate from the computer** the vault lives on — a password taped
  to the laptop protects against forgetting, not against losing the laptop.

## Make an encrypted backup

A backup is a single file (ending in `.pcfobk`) that contains your entire
vault, encrypted with the same password. Anyone who finds the file sees only
scrambled data; you, with the password, can turn it back into your vault.

1. Unlock your vault and open the **Backup** tab in the sidebar.
2. Type your vault password into the **Vault password** field (the backup is
   sealed with it, so the app asks you to confirm it).
3. Click **Export backup…** and choose where to save the file.

That's it — one file. Where to keep it:

- **Off this machine if you can**: an external drive, a USB stick, or a folder
  that syncs elsewhere. A backup on the same disk as the vault won't survive
  the disk dying.
- **Re-export after meaningful changes.** The backup is a snapshot; anything
  you enter after exporting is only in the vault until you export again.
- Once in a while, prove the backup works: restore it on another machine (or a
  scratch location) and check that it opens. An untested backup is a hope, not
  a backup.

## Restore on a new machine (or after your old one died)

You need two things: the `.pcfobk` file and the password it was exported with.

1. Install DohFlow on the new machine and launch it. With no vault yet,
   you'll land on the **Create your vault** screen.
2. Choose **Restore from backup…** and pick your `.pcfobk` file.
3. Enter the password the backup was created with.

The app checks the whole backup for damage *before* writing anything. If the
file is corrupted or the password is wrong, it tells you and leaves the machine
untouched — it will never overwrite an existing vault. If everything checks
out, your vault is restored exactly as it was: every account, transaction, and
attachment, to the byte.

## If you lost the password — or never made a backup

Honesty over comfort:

- **You have a backup but forgot the password:** the backup cannot be opened.
  It is encrypted with the same password as the vault, and there is no reset.
- **You have the password but no backup, and the machine is gone:** the data
  went with the machine. There is no cloud copy to pull down.
- **No backup and no password:** the data is gone. Nothing in the app, and
  nobody outside it, can bring it back.

If you're reading this section *before* disaster struck: export a backup now
and store the password somewhere safe. Those two steps make every scenario
above recoverable.

## Where your vault lives on disk

On macOS the vault sits in the app's data folder:

```
~/Library/Application Support/ai.personalcfo.desktop/
```

The main file is `vault.db` (additional vaults, if you've created them, sit
under a `vaults` folder next to it). The files are encrypted — copying them
somewhere is *not* a substitute for an exported backup, because a stray copy
misses the integrity checks and versioning a `.pcfobk` file carries. Use the
Backup tab; treat the data folder as the app's own business.
