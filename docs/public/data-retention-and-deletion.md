# What DohFlow keeps, and how to delete it

> **DRAFT — not for merge.** This draft covers local data only. The uninstall
> list still needs a clean-Mac drill. Nothing here is a legal
> statement until the pre-release legal review (`personal-cfo-rv2s`).

This page says what data DohFlow keeps, for how long, and how you remove it. It
is the plain-language companion to the [privacy page](privacy.md) and the
[threat model](../security/threat-model.md).

## The short version

- Your data lives in an encrypted vault on your Mac. DohFlow needs no server and
  no account to keep it.
- DohFlow never deletes your financial history on its own. Every removal is a
  choice you make.
- Backups you save to a folder are kept until you delete them yourself.
- DohFlow collects no telemetry.

## Data on your Mac

### Where it lives

On macOS the vault is in the app's data folder:

```
~/Library/Application Support/ai.personalcfo.desktop/
```

with additional vaults you create under a `vaults` folder beside it. A vault is
an encrypted database, a small unencrypted sidecar holding only public key
settings and your wrapped key (no password and no financial data), and a
`blobs/` folder of separately encrypted attachments. DohFlow never writes your
password or your vault key to disk; the key exists in memory only while the vault
is unlocked.

### How long it is kept

Everything in a vault is kept until you remove it. DohFlow has no automatic
expiry or cleanup of your records. In particular:

- **Change history.** The vault keeps a record of the changes made to it. That
  record is append-only: the database refuses edits and deletes to it, and
  DohFlow does not compact or purge it.
- **Imported data.** Records of where imported transactions came from stay with
  the vault, so you can always trace an entry back to its import.
- **Attachments.** A document attached to a transaction is stored encrypted under
  its own key. Removing the last link to a document destroys its key, which makes
  the stored bytes unreadable (crypto-shredding, ADR 0023).
- **Logs.** DohFlow writes no log file. Its redacted log lines go to the
  process's standard output and are never stored by the app. The in-app
  diagnostics card keeps a short in-memory record that is dropped when you lock
  the vault.
- **Telemetry.** None. No analytics, no usage statistics, no crash reports.
- **Diagnostic bundle.** If you choose to share one, you open Settings →
  Diagnostics, preview exactly what it contains, and save it to a file. DohFlow
  never sends it anywhere; you decide whether to send it to anyone.

### What "delete" means

Removing something in DohFlow does one of a few different things:

| What you remove | What happens |
| --- | --- |
| A transaction | It is **voided**: DohFlow posts a reversing entry and hides both, so balances and the forecast update as if it never happened. The ledger stays append-only and auditable. |
| An account | It can only be **archived**, never deleted. Its history stays intact. |
| A bill or income source | You choose **Archive** (kept, restorable) or **Delete** (removed permanently, behind its own confirmation). |
| A recurring transfer | Deleting it stops future projections; transfers already posted are untouched. |
| A document attached to a transaction | Removing the last link crypto-shreds it (above). |
| An entire vault | **Permanent.** It wipes the vault's files from disk. The only way back is restoring a backup you made beforehand. |

## Backups on your Mac

Backups you save to a folder, scheduled or one-off, are kept until you remove
them. DohFlow never prunes or deletes older backup files for you; to free space,
delete the copies you no longer need in Finder. If the folder is managed by iCloud
Drive, Dropbox or another sync client, that provider's client copies the encrypted
file off your Mac, and deleting it there is up to you and that provider.

Each backup keeps the password that was current when it was made. A later
password change does not alter an older backup.

## Removing DohFlow and your data completely

1. **Delete each vault from inside DohFlow**, or skip this step and remove the
   data folder in step 3. Deleting a vault wipes its files from disk.
2. **Quit DohFlow and drag it from Applications to the Trash.**
3. **Delete the data folder** shown above
   (`~/Library/Application Support/ai.personalcfo.desktop/`). This removes every
   vault that is still there.
4. **Delete your backups** in the folders you chose, and in any cloud folder that
   synced them. These are yours: DohFlow does not track or remove them.

**TBD — to verify on a clean Mac before this page is final:** whether macOS leaves
any other per-app folders (for example under `~/Library/Caches` or
`~/Library/WebKit`) that belong in step 3.

## Questions and problems

For a privacy or security issue, see [`SECURITY.md`](../../SECURITY.md). For
recovering a vault, see [Recovering your vault](../user-guide/recover-a-vault.md).
