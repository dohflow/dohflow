# Stack — pinned versions & persistence decisions

Living record of load-bearing technology choices and their pinned versions.
Update this when a version is intentionally bumped.

## Toolchain

| Component | Version | Pinned in |
|---|---|---|
| Rust toolchain | `1.96.0` (+ rustfmt, clippy) | `rust-toolchain.toml` |
| Rust MSRV | `1.95` | `Cargo.toml` `[workspace.package].rust-version` and `apps/desktop/src-tauri/Cargo.toml` (must match; CI-enforced) |
| Node | `>= 20` (CI uses 22) | `package.json` `engines`, CI |
| pnpm | `11.5.2` | `package.json` `packageManager` |

## Persistence: SQLCipher-encrypted SQLite

**Decision (spike `personal-cfo-6cp`): GO with `rusqlite` + bundled SQLCipher.**

The vault is a SQLCipher-encrypted SQLite database accessed through `rusqlite`
with the `bundled-sqlcipher-vendored-openssl` feature. SQLCipher and OpenSSL are
compiled from vendored source, so:

- there is **no system library dependency** (only a C compiler is needed);
- the SQLCipher version is **pinned reproducibly** via the Rust dependency
  graph (no reliance on whatever `libsqlcipher` a given machine has installed);
- builds are identical on dev machines and CI.

### Pinned versions

`rusqlite` is pinned **exact** (`=0.40.2`) in the workspace `Cargo.toml`, which —
with the committed `Cargo.lock` — locks the bundled engine below.

| Component | Version | Source |
|---|---|---|
| `rusqlite` | `0.40.2` | `Cargo.toml` (`=0.40.2`) |
| `libsqlite3-sys` | `0.38.2` | transitive, locked by `Cargo.lock` (root and desktop) |
| SQLCipher (`PRAGMA cipher_version`) | `4.14.0 community` | bundled by `libsqlite3-sys` |
| SQLite (`rusqlite::version()`) | `3.51.3` | bundled by `libsqlite3-sys` |
| SQLite source ID (`sqlite_source_id()`) | `2026-03-13 10:38:09 737ae4a3…6alt1` | asserted in full by `crates/db-worker/tests/engine_contract.rs` |
| Bundled amalgamation (`sqlcipher/sqlite3.c`) SHA-256 | `ea0bf0b08f688ca5d9312b2e33e7f81b3f4ae54b5016fb062ae1f2632a30a1b9` | `libsqlite3-sys` 0.38.2 crate |
| `openssl-src` (vendored) | `300.6.0+3.6.2` (OpenSSL 3.6.2) | transitive |

### Version-pin policy & cross-version testing (`personal-cfo-7igv`)

The SQLCipher/SQLite versions are **load-bearing**: a vault written by one version
must stay openable by the next, or users lose data (risk `personal-cfo-aia0`).

- **Enforced in CI.** `crates/finance-kernel/tests/cross_version_vault.rs` asserts
  the *linked* `cipher_version` / `sqlite_version` equal the table above, and
  round-trips a golden vault through the production `unlock_vault`. It runs on
  every PR (`cargo test --workspace`), so a silent `cargo update` that bumps the
  bundled engine fails the build here rather than shipping.
- **Bumping the engine is deliberate.** A PR that bumps `rusqlite` /
  `libsqlite3-sys` must, in the same PR: (1) update the table above, the manifest
  pin, and the release notes (`CHANGELOG.md`); (2) update `EXPECTED_SQLCIPHER` /
  `EXPECTED_SQLITE` in the test; and (3) capture the **outgoing** version's vault
  and add it to the test's cross-version corpus, proving the new engine opens
  every prior on-disk format.

### Engine update to SQLCipher 4.14.0 / SQLite 3.51.3 (`personal-cfo-g3m.5`)

**Why.** The previous bundle (SQLCipher 4.5.7 / SQLite 3.45.3, `libsqlite3-sys`
0.30.1) predates SQLite's fix for the WAL-reset database-corruption bug
([sqlite.org/wal.html §11](https://sqlite.org/wal.html)), fixed upstream in
SQLite 3.51.3 and shipped in [SQLCipher 4.14.0](https://github.com/sqlcipher/sqlcipher/releases/tag/v4.14.0).
The bug needs concurrent connections writing or checkpointing. The app's
controller mutex serializes most work, but `DbWorker` has independent reader
and projection connections, so the app is not claimed to be unaffected.

**Fix provenance (checked in the packaged source, not inferred from the version
number).** In the bundled amalgamation above, `walCheckpoint()` now re-reads the
live WAL header salt after taking read-lock slot 0 and skips the backfill if the
WAL was reset since the checkpoint began (`memcmp(pLive->aSalt, pWal->hdr.aSalt, …)`).
That guard is absent from the 0.30.1 amalgamation.

**Change scope.** Only `rusqlite`, `libsqlite3-sys` and their hashing
dependencies moved in either lockfile; the vendored OpenSSL, Tauri and all other
packages are unchanged. No vault, envelope, backup, KDF or schema format change.

**Compatibility evidence.** The outgoing engine's synthetic corpus was frozen
*before* the bump: a checkpointed vault, a committed-but-uncheckpointed WAL
(no SHM), its envelope, an encrypted attachment, a v2 backup package and the full
70-table logical state (`crates/finance-kernel/tests/fixtures/sqlcipher-4.5.7.json`,
`crates/db-worker/tests/fixtures/sqlcipher-4.5.7-state.json`). The new engine
opens all of it with identical canonical state (`engine_compatibility.rs`,
`engine_contract.rs`).

**Synthetic smoke (never on real data).** With the pinned toolchain:
`cargo test -p finance-kernel --test engine_compatibility --test cross_version_vault`
and `cargo test -p db-worker --test engine_contract`. Together they prove the
linked versions and source ID, open the frozen outgoing corpus (settled vault,
committed WAL without SHM, attachment, v2 backup) with identical state, recover
a WAL-only commit after an abrupt process exit, and preserve commits across a
reader-bounded checkpoint. For the app itself: create a throwaway vault in a dev
build, add an account and a transaction, quit, reopen, and confirm Settings →
Vault health is green.

**Rollback is not promised.** An older DohFlow build has not been tested against
a vault that this engine has written, and is not supported for that. To return
to an older build, restore a backup made by that build. Never test rollback on
real vaults.

### Minimum supported Rust (MSRV)

The declared MSRV is **1.95**, the lowest version in the owner-approved range
1.95–1.96 (ADR 0001 addendum, 2026-09-28): `rusqlite` 0.40 uses `cfg_select!`,
stabilized in Rust 1.95. The floor was verified with the actual 1.95 compiler
against the complete locked root workspace and the standalone desktop graph
(`--all-targets`, plus the desktop's `export-bindings` and
`tauri/custom-protocol` features). CI rechecks both graphs with the declared
compiler on every Rust change (`Declared MSRV builds` steps in
`.github/workflows/ci.yml`). The pinned build toolchain stays **1.96.0**.

> The durable WAL/SHM/temp plaintext-leak suite is owned by `personal-cfo-zxvl`
> (`crates/finance-kernel/tests/side_file_leak.rs`).

### Configuration validated by the spike

- `PRAGMA key` is applied immediately after `open`, before any other access.
- `journal_mode = WAL`; `busy_timeout` set.
- WAL sidecar (`-wal`) is created on write and is itself encrypted.

### Opacity / encryption evidence (validated by the `6cp` spike)

The spike (since removed) demonstrated, and the durable suites (`personal-cfo-zxvl`,
`personal-cfo-7igv`) carry forward:

- The main DB file does **not** begin with the plaintext `SQLite format 3\0`
  magic header.
- A known plaintext sentinel never appears in the DB or `-wal` bytes.
- A **wrong key** fails (at keying or first read); an **unkeyed** connection
  (stand-in for stock SQLite / `sqlite3`) cannot read the data.
- The correct key round-trips.

### `rusqlite` vs `sqlx`

`rusqlite` chosen for v1: synchronous, direct SQLCipher support via
`libsqlite3-sys` bundled feature, mature `PRAGMA key` ergonomics, and a natural
fit for the single-writer db-worker model (`personal-cfo-klr.3`). `sqlx`'s async
model adds complexity without benefit for a local single-process vault. Revisit
only if a concrete need (e.g. compile-time-checked queries) outweighs this.

### Ownership

`crates/db-worker` (`personal-cfo-klr.3`) is the **sole** owner of `rusqlite`
in the workspace, CI-enforced (`.github/workflows/ci.yml`). The original
`sqlcipher-spike` crate that validated this stack has been removed now that
db-worker exists. The ongoing opacity / plaintext-leak regression suite is owned
by `personal-cfo-zxvl`, and cross-version vault-open tests by
`personal-cfo-7igv`.
