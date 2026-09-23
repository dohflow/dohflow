# Privacy

DohFlow is a local-first household finance app. This page is the plain-language
summary of what that means in practice: what stays on your Mac, what can leave
it, what we never collect, and how your data is protected. Every claim here is
checked against the app's actual code — the Tauri capability grants that
control what the app is even able to reach, and the network calls it actually
makes — not just a policy we're promising to follow.

## What stays on your Mac

Everything. Your accounts, transactions, income, bills, balances, attached
documents, and every setting live in a single encrypted **vault** file on your
computer. There is no DohFlow server and no DohFlow account — nothing to sign
up for, nothing to log into, and no copy of your data anywhere but the vault
itself (and any backup you choose to make).

## What can leave your Mac

These are the only outbound network connections the app can make. The app's
own capability configuration enforces this at the code level — no plugin that
could reach the network beyond these paths is even present, and the content
security policy blocks the app's interface from making any network request on
its own.

- **A bank connection, only if you set one up.** DohFlow supports linking
  accounts through the [SimpleFIN Bridge](https://bridge.simplefin.org). Your
  actual bank login credentials go to the Bridge, at the Bridge's own site —
  never to DohFlow. You paste your own SimpleFIN setup token into the app;
  the app talks to the Bridge directly, using that token, and stores the
  resulting access credential inside your encrypted vault. SimpleFIN is an
  independent, unaffiliated service — you contract and pay them directly for
  it, not us — and DohFlow never sits in the middle of that connection or
  sees anything beyond what the Bridge returns for your own accounts.
  Nothing is ever synced unless you complete that setup; manual entry and
  CSV import work fully without it. Once you've linked a connection,
  DohFlow refreshes it from the Bridge automatically each time you unlock
  your vault, plus whenever you trigger a manual sync — always using your
  own token, never through us.
- **A version check.** DohFlow checks whether a newer release is available
  each time you launch or unlock it, and any time you check manually from
  Settings. That check is a single request for a public file on GitHub
  Releases — it carries no identifier for you or your device, no usage data,
  nothing beyond what any ordinary web request inherently reveals (the
  requesting IP address, visible to GitHub the way it would be to any site
  you visit). Downloading and installing an update is never automatic —
  it happens only when you click to do it.
- **Links you click.** The About and Support screens link to pages on
  `dohflow.app` — release notes, the bug-report form, and similar. Clicking
  one is a hand-off to your regular web browser, never a request the app
  itself makes — DohFlow just opens the page and steps out of the way. The
  app cannot open any other site this way; that link scope is enforced the
  same way the network restrictions above are.

Those are the network connections DohFlow itself makes. If you save an
encrypted backup in a cloud-synced folder, it leaves your Mac through your
chosen provider's sync client as ciphertext. DohFlow does not manage that
upload and receives no folder or usage report. A format-v2 backup header does
contain a stable vault-envelope fingerprint, so someone who can see the
ciphertext can correlate backups made before a password rewrap or rekey.

## What we never collect

**No telemetry. No analytics. No crash reporting. No "anonymous usage
statistics." No account, no sign-up, no login.** There is no code path in the
app that could send any of that anywhere — not a setting you'd need to turn
off, because it was never built. If that ever changes, it would be a new,
disclosed capability, not a quiet policy update.

## Encryption

Your vault is protected by a password only you know — DohFlow never stores it
in any form. That password derives a key (via Argon2id, a modern,
memory-hard key-derivation function chosen to resist offline guessing) which
in turn unlocks the key that actually encrypts your data. The vault database
itself is encrypted end-to-end with SQLCipher (AES-256); each attached
document gets its own separate encryption key, so one compromised attachment
never exposes the rest. The unlocking key exists only in memory while the
vault is open, and is wiped the moment you lock it.

## Retention and deletion

Within a vault, nothing is ever deleted silently or automatically — every
removal is a choice you make explicitly. Accounts can only be archived, never
deleted, so your account history always stays intact. Bills and income
sources give you a choice: Archive keeps the record and lets you restore it
later; Delete removes it permanently, and always sits behind its own
confirmation step rather than a single click. Deleting an entire vault is
different from any of that, and is permanent — it wipes that vault's files
from disk, and the only way back is restoring a backup you made beforehand. A
fuller retention-and-deletion reference is planned; this page will link it
once it exists.

## If you lose your password

There is no password reset — not a missing feature, a deliberate one. Nothing
outside your machine holds your password or your data, so nothing outside
your machine can recover either one. DohFlow tells you this, in these exact
words, before you ever create a vault:

> DohFlow has no cloud password reset. If you forget your password, your data
> is unrecoverable. Save your password somewhere safe.

See [Recovering your vault](../user-guide/recover-a-vault.md) for the full
guide, including how to make and restore an encrypted backup.

## The full threat model

This page is the plain-language summary. The complete technical threat
model — what's protected, the trust boundaries, and every mitigation in
place — is a living document at
[`docs/security/threat-model.md`](../security/threat-model.md).

## Reporting a problem

If you find a privacy or security issue, please report it privately rather
than in a public issue — see [`SECURITY.md`](../../SECURITY.md) for how.
