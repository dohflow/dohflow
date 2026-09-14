# Encryption design

Written 2026-09-11 (`personal-cfo-7ie.9`), promised by [ADR 0002](../adr/0002-local-encrypted-vault.md)
line 123 but never authored until now. The key-hierarchy *decision* is ADR
0002; this document is the concrete parameter reference ADR 0002 points at —
what the code actually ships, not a re-derivation of the design rationale.

**Source of truth:** [`crates/vault-crypto/src/lib.rs`](../../crates/vault-crypto/src/lib.rs)
(the `Profile` enum, lines 74–117) and
[`crates/finance-kernel/tests/cross_version_vault.rs`](../../crates/finance-kernel/tests/cross_version_vault.rs).
If this document and the code ever disagree, the code and its tests are
correct — file a bug against this doc rather than against the crate.

## Argon2id profiles

`vault-crypto::Profile` defines three calibrated Argon2id parameter sets.
Each mirrors the `vault_metadata.kdf_*` columns (`algorithm`, `memory_kib`,
`time_cost`, `parallelism`, `version`) 1:1 via `Argon2Params`, so persistence
never has to re-derive them.

| Profile | Memory | Time cost | Parallelism | Selected today | Purpose |
|---|---|---|---|---|---|
| `InteractiveDefault` | 65 536 KiB (64 MiB) | 3 | 1 | **Always**, at vault creation and rekey | Everyday unlock, tuned for a sub-second-to-~1s derivation on dev hardware |
| `HighSecurity` | 262 144 KiB (256 MiB) | 4 | 1 | Not wired to any user-facing flow yet | Reserved for a future stronger-KDF opt-in |
| `LegacyCompatibility` | 19 456 KiB (19 MiB) | 2 | 1 | Only to *open* an older vault | The OWASP Argon2id floor — never selected for a freshly created vault; existing vaults rekey forward to `InteractiveDefault`/`HighSecurity` |

All three use `algorithm = "argon2id"` (`ALGORITHM` constant) and
Argon2 version `0x13` (the latest Argon2 spec revision, via the `argon2`
crate's `Version::V0x13`). `PARAMS_VERSION = 1` stamps the parameter *shape*
(the struct's fields), separately from the parameter *values* above — it
only bumps if a future change adds/removes/reinterprets a field, not when
the KiB/time-cost numbers change.

`derive_kek()` (`lib.rs`) runs Argon2id over the caller's password and a
per-vault [`Salt`](#salt-and-derivation) to produce a 256-bit
[`Kek`](#key-hierarchy-recap), backed by `SecretBytes` (`mlock`'d,
zeroize-on-drop, non-`Clone`) so the derived key can't be accidentally
copied or logged — its `Debug` impl prints `Kek([REDACTED])`.

### What's actually wired up today

- **Vault creation** (`crates/finance-kernel/src/vault.rs`) and **backup
  export** (`crates/finance-kernel/src/backup.rs`) both call
  `Profile::InteractiveDefault.params()` directly — there is no
  device-class branch in either path today.
- **`HighSecurity`** is a real, tested profile (see
  `high_security_is_costlier_than_interactive` in `lib.rs`'s test module) but
  nothing in the app currently offers a user a way to select it. Wiring a
  user-facing opt-in is future work.
- **Per-device calibration** — choosing parameters based on the device's
  actual hashing throughput rather than a fixed profile, targeting a
  500–1000ms interactive unlock — is tracked separately and remains open:
  `personal-cfo-0sqk`. Until it ships, every device gets the fixed
  `InteractiveDefault` parameters above regardless of hardware class. This
  is accepted as secure-by-default (64 MiB / t=3 already clears the OWASP
  floor by more than 3×) rather than release-blocking.
- **`LegacyCompatibility`** exists so a vault created under an older,
  weaker parameter set can still be *opened* (unwrap succeeds against
  whatever `memory_kib`/`time_cost` the vault's own `vault_metadata` row
  records — `Profile` selection happens at write time; reading an envelope
  uses whatever parameters are stored alongside it, not a hardcoded
  profile). Full rekey-forward tooling is `personal-cfo-2y8`.

### Test floor

`lib.rs`'s `interactive_default_meets_floor_and_matches_db_worker_seed` unit
test pins two things simultaneously:

1. A **security floor**: `memory_kib >= 65_536` and `time_cost >= 3` (ADR
   0002 / Risk Register §24) — a future accidental weakening of
   `InteractiveDefault` fails `cargo test` immediately.
2. A **one-sided drift pin, not a mutual guarantee**: the test hardcodes
   literals (`assert_eq!(p.memory_kib, 65_536)`, etc.) under a "if you
   change one, change both" comment and asserts `vault-crypto`'s side
   against them. `vault-crypto` deliberately does not depend on
   `db-worker` (crate-boundary rule, this crate's module doc), and nothing
   on the `db-worker` side asserts the reverse — so editing
   `db-worker`'s seeded `DEFAULT_KDF_*` constants
   (`crates/db-worker/src/lib.rs`) alone would fail no test today. The
   values do match as of this writing (`argon2id` / 65 536 / 3 / 1 on both
   sides). Practical blast radius is limited because unlocking an
   *existing* vault always uses the parameters stored in that vault's own
   envelope, never a hardcoded profile (see
   [Envelope version scheme](#envelope-version-scheme)) — a one-sided
   drift would only affect a freshly created vault's initial
   `vault_metadata` seed. A real cross-crate assertion in both directions
   is open follow-up, not yet filed as a bead.

## Key hierarchy recap

The layered model itself — why a KEK/DEK split rather than a single
password→SQLCipher key — is ADR 0002's decision; the parameters above are
what fills in that model concretely:

```
master password
   │  Argon2id (InteractiveDefault today; HighSecurity reserved; LegacyCompatibility open-only)
   ▼
KEK  (crate: Kek — 256-bit, SecretBytes-backed, zeroizes on drop)
   │  AES-256-GCM wrap, fresh random 96-bit nonce
   ▼
DEK  (crate: Dek — 256-bit, SecretBytes-backed, random at vault creation)
   │  fed directly to SQLCipher as the raw key
   ▼
SQLCipher-encrypted vault.db   +   per-attachment content keys (each independently wrapped by the DEK)
```

- **KEK** never touches disk. It exists only in process memory between
  `derive_kek()` and the moment it wraps/unwraps a DEK.
- **DEK** is generated once per vault (`generate_dek()`, OS CSPRNG) and
  never changes on a password change — only its *wrapping* changes,
  which is why a password change is cheap (re-wrap, not re-encrypt the
  whole database).
- **Wrong password → wrong KEK → AEAD tag fails to verify** on
  `unwrap_dek()`. This AES-GCM authentication failure, not a separate
  password check, is the actual mechanism that rejects a wrong password
  (`VaultCryptoError::KeyUnwrap`).

## Salt and derivation

- `Salt` is 16 bytes (`SALT_LEN`), generated per-vault from the OS CSPRNG
  (`generate_salt()`). It is public — stored in plaintext alongside the
  envelope — and its role is to make precomputed dictionary/rainbow-table
  attacks per-vault-unique, not to add secrecy.
- Argon2id's `output_len` is left `None` in `derive_kek()`, so the 32-byte
  (`KEK_LEN`) output buffer itself drives the derived-key length.

## Envelope version scheme

The wrapped DEK, the per-vault salt, and the KDF parameters travel together
in a `VaultEnvelope`, serialized to a **plaintext sidecar file** next to the
encrypted database (`crates/vault-crypto/src/envelope.rs`) — these values
are needed *before* the database can be decrypted, so they cannot live
inside SQLCipher's own encrypted pages. The sidecar's confidentiality
requirement is nil: it carries only ciphertext (the wrapped DEK) plus public
parameters, all of which are useless without the master password.

- **Magic:** the sidecar begins with `PCFOVLT` (`MAGIC`, 7 bytes) so a
  corrupted or unrelated file is rejected before any cryptographic work
  (`VaultCryptoError::MalformedEnvelope`).
- **`ENVELOPE_VERSION: u16 = 1`** today. This is the on-disk *byte layout*
  version (ADR 0002's `vault_envelope_version`) — bumped only when the
  serialization format itself changes (new fields, different framing), not
  when Argon2id parameter values change (those are just data inside the
  envelope, already versioned separately via `PARAMS_VERSION`). An
  envelope whose version this build doesn't understand is rejected via
  `VaultCryptoError::UnsupportedEnvelopeVersion` rather than
  misinterpreted.
- **DEK wrap:** AES-256-GCM, a fresh random 96-bit nonce per wrap
  (`wrap_dek()`/`unwrap_dek()`), ciphertext includes the AEAD
  authentication tag.
- **KDF algorithm id:** the sidecar serializes a one-byte algorithm tag
  (`ALGO_ARGON2ID = 1`); Argon2id is the only value currently defined.

A version bump (v1 → v2) is how the project would migrate to, say, a
different AEAD primitive or an upgraded parameter *shape* without breaking
vaults created under v1 — the reader dispatches on the version byte before
parsing the rest of the sidecar.

## Golden-vault engine pin

Encryption parameters are only half of "can this vault still be opened" —
the other half is the underlying SQLCipher/SQLite build itself.
[`cross_version_vault.rs`](../../crates/finance-kernel/tests/cross_version_vault.rs)
is the release-blocking (`cargo test --workspace`) guard for that:

- `linked_engine_versions_match_the_pin` asserts the *linked* SQLCipher
  (`PRAGMA cipher_version`) and SQLite (`rusqlite::version()`) exactly equal
  the pins recorded in [`docs/architecture/stack.md`](../architecture/stack.md)
  — **SQLCipher `4.5.7 community`**, **SQLite `3.45.3`** — so a silent
  `cargo update` that bumps the bundled engine (and could make existing
  vaults unopenable) fails this test instead of shipping.
- `golden_vault_round_trips_on_the_pinned_engine` builds a real vault
  through the production Finance Kernel command bus, closes it, and
  reopens it through the actual `unlock_vault` password path — proving the
  full key-hierarchy round trip above (password → KEK → DEK → SQLCipher)
  survives a close/reopen cycle, not just in isolation.

When the pin is intentionally bumped, the module doc comment in that file
spells out the required steps: update `EXPECTED_SQLCIPHER`/`EXPECTED_SQLITE`
plus `docs/architecture/stack.md` and the release notes, and add the
*outgoing* engine's vault to a cross-version corpus so the new engine is
proven to still open vaults written by the old one.

## Verification

```sh
cargo test -p finance-kernel --test cross_version_vault
cargo test -p vault-crypto
```

Both pass as of this writing (2026-09-11): `cross_version_vault` — 2/2
(`linked_engine_versions_match_the_pin`,
`golden_vault_round_trips_on_the_pinned_engine`); `vault-crypto` — 39/39
across its unit tests, `derive_kek.rs`, and `no_reset_warning.rs` (0
doc-tests). See `personal-cfo-7ie.9`'s bead notes and the PR that introduced
this document for the exact run.

## References

- [ADR 0002 — Local encrypted vault model](../adr/0002-local-encrypted-vault.md) — the key-hierarchy decision this document fills in.
- [`docs/architecture/stack.md`](../architecture/stack.md) — SQLCipher/SQLite engine pins.
- [`docs/security/threat-model.md`](threat-model.md) — TB2 (unlocked memory ↔ disk) and the brute-force-password threat row cite this document.
- `personal-cfo-0sqk` — open follow-up: per-device Argon2id calibration + a CI parameter-floor regression guard.
- `personal-cfo-2y8` — key rotation / rekey implementation, including the `LegacyCompatibility` → current-profile migration path.
- `personal-cfo-1t0` — `SecretBytes` `mlock`/zeroize protection this crate's `Kek`/`Dek` types build on.
