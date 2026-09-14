# ADR 0025: Multi-vault management (registry, naming, lifecycle)

- **Status:** Accepted
- **Date:** 2026-06-21
- **Deciders:** Project owner
- **Bead:** [`personal-cfo-7ie.2`](../../.beads/issues.jsonl)
- **Related plan sections:** §6.4 (vault file layout), §10.2 (onboarding)
- **Amends:** [ADR 0002](0002-local-encrypted-vault.md) (single fixed-path vault)
- **Supersedes:** None

## Context

Today the app supports exactly **one** vault. ADR 0002 fixes the crypto model
(master password → Argon2id KEK → AEAD-wrapped DEK → SQLCipher raw key); the
desktop app stores that single vault at a **fixed path** —
`<app_data_dir>/vault.db` plus its `vault.db.envelope` sidecar and `blobs/`
directory. On launch, `vault_status` checks whether the envelope exists and
routes to either the **Create** screen (no vault) or the **Unlock** screen (a
vault exists). There is no way to keep more than one vault, name them, switch
between them, or delete one from the UI (deletion is a manual `rm` today).

Dogfooding feedback (2026-06-21) asks for: multiple named vaults on one machine
(e.g. *personal* vs *business*, or a second household member), a launch-time
**"open existing vs create new"** choice, and **delete-from-the-UI** with an
unmissable "this can't be recovered — back it up first" warning.

This is an architecturally significant change: the vault stops being a singleton
at a known path and becomes one of N discoverable, named, manageable units. This
ADR fixes that model. The four decisions originally raised for review (marked
**Decided** below) were ratified as recommended on 2026-06-21.

## Decision

### 1. Storage layout — a managed vaults directory + a plaintext registry

Each vault is a **self-contained subdirectory**:

```
<app_data_dir>/
  vaults/
    <vault_uuid>/
      vault.db            # SQLCipher (ADR 0002)
      vault.db.envelope   # KDF salt/params + wrapped DEK (ADR 0002)
      blobs/              # encrypted attachments (ADR 0023)
  vaults.json             # the registry (plaintext discovery index)
```

The **registry** (`vaults.json`) is the source of truth for *discovery*: an
ordered list of `{ id, name, dir, created_at, last_opened_at }`. It exists
because the launch picker must show vault **names before unlock**, and a vault's
own name (inside `vault_metadata`) is encrypted and unreadable without its
password. Writes to the registry are atomic (temp-file + rename), like the
envelope sidecar.

Keeping each vault in its own directory makes it a complete, portable,
backup-able unit (the `.pcfobk` backup of ADR 0024 already bundles exactly these
three artifacts). Opening a vault from an **arbitrary** path (for portability /
restoring a loose vault directory) is a natural later addition and does not
change this core.

### 2. Vault naming — plaintext in the registry

The display name lives in the registry as plaintext, so the picker can render it
without unlocking. The name is also written into the vault's own (encrypted)
`vault_metadata` so a rename is auditable and the vault carries its own label,
but the **registry copy is authoritative** for the picker.

**Decided (plaintext names):** the name is plaintext on disk — like a filename.
It is *not* financial data, but a label such as "Chris — personal" is mildly
identifying. We accept this (consistent with filenames being plaintext; vault
*contents* stay fully encrypted) and document it as a known, minor metadata
exposure. Encrypting names would defeat the pre-unlock picker and is rejected.

### 3. Launch flow — a vault picker

On launch the app reads the registry:

- **More than one vault:** show a **Vault Picker** — the named vaults (most
  recently opened first) plus **"Create new vault,"** with per-row **open /
  rename / delete**. Selecting a vault routes to its **Unlock** screen.
- **Exactly one vault:** **Decided** — **skip the picker and go straight to
  Unlock** for that vault (no friction for single-vault users), with a way to
  reach the picker (a "switch vault" affordance on the Unlock screen).
- **No vaults:** the existing **Create your first vault** screen (with the n7bo
  no-reset warning).

Switching vaults locks the current one first (see §6). This becomes an explicit
state in the vault routing state machine (today: `NoVault` / `Locked` /
`Unlocked` / `CorruptNeedsRecovery`; add a `Picker`/selection state).

### 4. Delete-from-UI — crypto-shred, strong confirmation, backup nudge

Deleting a vault removes its **entire subdirectory** (`vault.db`, the envelope,
`blobs/`) and its registry entry. Because the data is encrypted and the key
envelope is destroyed with it, deletion is an effective **crypto-shred** — the
data is unrecoverable.

The confirmation modal MUST: (a) state plainly that deletion is **permanent and
unrecoverable** (no cloud, no password reset — reuse the ADR 0002 framing); (b)
**nudge the user to export a backup first** (deep-link to the `.pcfobk` export of
ADR 0024); and (c) require an explicit confirm.

**Decided (delete confirmation):** the user must **type the vault name** to
enable the Delete button (highest accident-resistance for an irreversible
action), rather than a simple checkbox.

The deletion is recorded as an audit event (`vault_deleted` with the vault id +
timestamp), written **before** the files are removed. Because that record cannot
live inside the deleted vault, it goes in a small **registry-level audit log**, a
sibling of `vaults.json` (distinct from the per-vault `audit_events` table) — to
be designed in the implementation bead.

### 5. Legacy migration — register the existing vault in place

Existing installs have a vault at the legacy `<app_data_dir>/vault.db`. On first
launch of the multi-vault build, detect it and **register it in place**: add a
registry entry pointing at the legacy path with a default name (e.g. "My Vault,"
or prompt for a name on first open). No files are moved.

**Decided (legacy migration):** **register-in-place** (backward-compatible, no
file-move on real user data) rather than relocating the legacy vault into
`vaults/<id>/`. A one-time, opt-in "tidy into the new layout" can be offered
later.

### 6. One active vault at a time

Only one vault is unlocked at a time (the *active* vault). Switching = lock the
current vault, return to the picker, unlock the next. Per ADR 0003 the
TanStack Query cache is cleared on lock, so no read-model data crosses between
vaults. Multiple-simultaneously-unlocked vaults are out of scope (added
complexity, no dogfooding need).

## Consequences

- **(+)** Multiple named vaults; a clean launch picker; safe delete-from-UI.
- **(+)** Each vault is a self-contained, portable, backup-able directory that
  the existing `.pcfobk` backup already matches.
- **(−)** A plaintext **registry** to maintain — a new (small) discovery surface
  holding names, directory names, and timestamps. **No financial data, no keys,
  no balances.** It must be covered by the §6.6 / `zobt` redaction expectations
  if ever logged, and excluded from plaintext-leak scans appropriately.
- **(−)** Vault **names are plaintext** on disk (Decision 2).
- **(−)** The `vault_deleted` audit record needs a home that outlives any single
  vault — a small **registry-level audit log** (sibling of `vaults.json`),
  distinct from the per-vault `audit_events` table. A new, minor artifact to
  design in the implementation bead.
- **(−)** The vault routing state machine and the desktop `VaultController` grow
  a selection/picker state and per-vault path handling (no longer a constant).
- Relates to **ADR 0002** (amended: vault path is no longer a fixed singleton),
  **ADR 0003** (cache-clear on lock now scopes per active vault), **ADR 0024**
  (backup is per-vault; "back up before delete" becomes prominent), and **ADR
  0023** (`blobs/` is per-vault, already under the vault dir).

## Alternatives considered

- **User-chosen arbitrary location per vault** (vault-as-document, like opening a
  `.sqlite` file anywhere). More flexible, but pushes path management onto the
  user and complicates discovery, backup, and the picker. Rejected as the *core*
  model; "open a vault from an arbitrary path" can be added later on top of the
  registry.
- **Single vault, multiple "profiles" inside one DB.** Keeps one file but mixes
  unrelated finances in one encryption boundary and one ledger — wrong blast
  radius for delete, backup, and privacy. Rejected.
- **Encrypted vault names.** Cannot render the picker pre-unlock. Rejected.

## Decisions ratified (2026-06-21)

1. Plaintext vault names in the registry — **accepted** (minor, documented
   exposure; contents stay encrypted).
2. With exactly one vault, **skip the picker** straight to Unlock.
3. Delete confirmation requires **typing the vault name**.
4. Legacy vault is **registered in place** (no file move).
