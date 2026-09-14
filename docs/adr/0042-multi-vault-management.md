# ADR 0042 — Multi-vault management: app-managed subfolders + a plaintext registry

- Status: Accepted
- Date: 2026-07-03
- Bead: personal-cfo-j0cg.6 (epic); slice 1 = registry foundation
- Related: ADR 0002 (vault crypto / envelope), ADR 0024 (backup/restore), ADR 0003 (Rust-authoritative, no data in the webview), personal-cfo-j0cg.5 (delete-vault)

## Context

The app is single-vault: `run()` opens one `VaultController` at a fixed path
(`<app_data_dir>/vault.db`), and `AppState` holds that one controller for the process lifetime.
The owner wants to keep and switch between **multiple** vaults — e.g. a real vault plus a throwaway
test vault, or per-household separation — without destroying one to use the other.

A vault is an encrypted SQLite DB (`vault.db`) + a plaintext `.envelope` sidecar (the unlock
material) + a `blobs/` attachment store. Exactly one vault is unlocked at a time today (one
controller, one in-memory DEK), and that invariant must hold under multi-vault: **keys never cross
vaults**.

## Decision

**1. App-managed subfolders (owner-chosen, 2026-07-03).** Each vault lives at
`<app_data_dir>/vaults/<uuid>/vault.db` (with its `.envelope`, WAL/SHM, and `blobs/` alongside).
The user *names* vaults ("Real", "Test"); they never pick a filesystem path. Opening a vault from an
arbitrary external file is explicitly out of scope for now (a possible later escape hatch).

**2. A plaintext registry** at `<app_data_dir>/vaults.json`, holding one entry per known vault —
`{ id, name, path (relative to app_data_dir), created_at }` — plus the `active` vault id. It carries
**no secrets** (only ids, display names, and relative paths), so it is readable *before* any vault is
unlocked, which is what the launch picker needs. It is authored only by the Rust command layer
(ADR 0003); the webview never touches it.

**3. The existing vault is registered in place, not moved.** On first multi-vault launch, if
`<app_data_dir>/vault.db` exists and the registry is absent/empty, it is registered with its actual
relative path (`vault.db`) and marked active. We do **not** move a real vault's files into
`vaults/<id>/` — a move is a needless risk to the user's real data. Registry entries therefore carry
an explicit path; new vaults get `vaults/<id>/vault.db`, the legacy vault keeps `vault.db`.

**4. One vault unlocked at a time; switching locks first.** Switching to another vault **locks the
current one** (dropping and zeroizing its DEK, ADR 0002) before the controller re-points at the
target and re-classifies it (`NoVault` / `Locked` / `CorruptNeedsRecovery`). The user then unlocks
the target with its own password. The single-controller design already guarantees at most one
in-memory DEK; switching preserves it.

**5. Backup, restore, and delete are per-vault and unchanged.** Export/restore (ADR 0024) and
delete-vault (j0cg.5) operate on the *active* vault. Restoring a backup still targets a `NoVault`
location (e.g. a freshly-created, still-empty vault slot). Deleting the active vault removes its
files and its registry entry, then falls back to another registered vault (or the create screen).

**6. Backward-compatible rollout.** Slice 1 introduces the registry + launch wiring + a `list_vaults`
read, with **no behavioral change**: the app still opens the one (now-registered) vault. Create-new,
switch, and the picker UI land in later slices behind the registry this establishes.

## Consequences

- The registry is a new, un-encrypted on-disk artifact. It leaks only that N vaults exist and their
  display names — acceptable (the threat model protects vault *contents*, not their existence).
- `AppState` gains the registry + the app-data root alongside the controller; the controller stays
  the sole owner of the unlocked kernel.
- A missing/corrupt `vaults.json` is recoverable: it can be rebuilt by scanning for `vault.db` +
  `vaults/*/vault.db`. Slice 1 rebuilds the minimal case (the legacy vault); a fuller scan is a
  follow-up.
- Switching mid-session is safe by construction (lock-before-repoint), but any in-flight read on the
  old vault must fail closed (`VaultLocked`) — the same steady state as an explicit lock.
