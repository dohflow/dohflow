# Stack — pinned versions & persistence decisions

Living record of load-bearing technology choices and their pinned versions.
Update this when a version is intentionally bumped.

## Toolchain

| Component | Version | Pinned in |
|---|---|---|
| Rust toolchain | `1.96.0` (+ rustfmt, clippy) | `rust-toolchain.toml` |
| Rust MSRV | `1.82` | `Cargo.toml` `[workspace.package].rust-version` |
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

`rusqlite` is pinned **exact** (`=0.32.1`) in the workspace `Cargo.toml`, which —
with the committed `Cargo.lock` — locks the bundled engine below.

| Component | Version | Source |
|---|---|---|
| `rusqlite` | `0.32.1` | `Cargo.toml` (`=0.32.1`) |
| `libsqlite3-sys` | `0.30.1` | transitive, locked by `Cargo.lock` |
| SQLCipher (`PRAGMA cipher_version`) | `4.5.7 community` | bundled by `libsqlite3-sys` |
| SQLite (`rusqlite::version()`) | `3.45.3` | bundled by `libsqlite3-sys` |
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
