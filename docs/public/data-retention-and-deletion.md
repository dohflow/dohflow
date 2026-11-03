# What DohFlow keeps, and how to delete it

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
- **Removed items.** A bill, income source, recurring transfer or scenario you
  remove or delete is hidden everywhere in DohFlow but not erased. It stays in the
  vault file and in backups made from it (see "What 'delete' means" below).
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
| A bill or income source | You choose **Archive** (kept and restorable from the Archived list) or **Remove** (behind its own confirmation). A removed item disappears everywhere in DohFlow at once and its forecast effect stops. DohFlow offers **Undo** for about ten seconds afterward; after that there is no screen for bringing it back. It is hidden, not erased: the record stays inside the vault file and in every backup made from it. |
| A recurring transfer | **Remove** works the same way: it disappears everywhere in DohFlow, future projections stop, and transfers already posted are untouched. Undo is offered for about ten seconds, and the record stays inside the vault file and your backups. |
| A scenario | **Delete** (behind its own confirmation) takes the scenario and its changes out of every view in DohFlow, with no screen for bringing it back, and your real forecast is not affected. They are hidden, not erased: they stay inside the vault file and in every backup made from it. **Archive** keeps a scenario you want to set aside, and you can restore it. |
| A document attached to a transaction | Removing the last link crypto-shreds it (above). |
| An entire vault | **Permanent.** It wipes the vault's files from disk, including everything that was hidden inside it. The only way back is restoring a backup you made beforehand. |

Taking something out of view is not the same as erasing it. Items you remove or
delete in the rows above stay inside the vault file, and inside every backup made
from it, until you delete the whole vault and those backups.

## Backups on your Mac

Backups you save to a folder, scheduled or one-off, are kept until you remove
them. DohFlow never prunes or deletes older backup files for you; to free space,
delete the copies you no longer need in Finder. If the folder is managed by iCloud
Drive, Dropbox or another sync client, that provider's client copies the encrypted
file off your Mac, and deleting it there is up to you and that provider.

Each backup keeps the password that was current when it was made. A later
password change does not alter an older backup.

## Removing DohFlow and your data completely

DohFlow keeps its data in four places on macOS, all under your own user folder.
It writes no log file and stores nothing in the macOS Keychain.

| Location | What is there |
| --- | --- |
| `~/Library/Application Support/ai.personalcfo.desktop/` | Your vaults (and the list of them), with attachments. This is your data. |
| `~/Library/Caches/ai.personalcfo.desktop/` | Web view cache for the app's own screens. No vault data. |
| `~/Library/WebKit/ai.personalcfo.desktop/` | Web view storage for the app's own screens. No vault data. |
| `~/Library/Preferences/ai.personalcfo.desktop.plist` | macOS's own record of app settings, including the last folder you picked in a file dialog (for example a backup folder). |

To remove all of it:

1. **Delete each vault from inside DohFlow**, or skip this step and remove the
   data folder in step 3. Deleting a vault wipes its files from disk.
2. **Quit DohFlow and drag it from Applications to the Trash.**
3. **Delete the four locations above.** In Finder, choose Go → Go to Folder and
   paste each path. Deleting the first removes every vault that is still there.
4. **Delete your backups** in the folders you chose, and in any cloud folder that
   synced them. These are yours: DohFlow does not track or remove them.

Deleting the data folder is permanent. Make a backup first if you might want the
data again.

## Questions and problems

For a privacy or security issue, see [`SECURITY.md`](../../SECURITY.md). For
recovering a vault, see [Recovering your vault](../user-guide/recover-a-vault.md).
