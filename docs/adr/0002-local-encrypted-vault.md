# ADR 0002: Local encrypted vault model

- **Status:** Accepted
- **Date:** 2026-05-04
- **Deciders:** Project owner
- **Bead:** [`personal-cfo-6yf`](../../.beads/issues.jsonl)
- **Related plan sections:** §0 (decision 2), §6.2, §6.3, §6.4, §6.5
- **Supersedes:** None

## Context

Financial data must be encrypted at rest before any useful feature ships (§1.5 non-negotiable). The product must be useful in manual-only mode without any cloud dependency. macOS Keychain is appropriate for small wrapping secrets; LocalAuthentication can drive optional Touch ID. The chosen storage layer is SQLCipher (ADR-pending, but locked by §3.2 and the plan profile), which provides full-database AES-256 encryption transparently to SQLite.

We need a vault model that:

- Never stores the master password.
- Survives password change without re-encrypting the whole database every time.
- Allows future key rotation and KDF parameter migration.
- Treats Keychain and Touch ID as **convenience layers** on top of password unlock, never as the only path to plaintext data.
- Encrypts attachments separately from the main DB, since attachments don't fit in SQLCipher pages.

## Decision

The vault uses a layered key hierarchy:

```
master password
   │  Argon2id (per-vault salt + calibrated parameters)
   ▼
KEK (key-encrypting key, never persisted)
   │  AEAD wrap
   ▼
DEK (data-encrypting key, random-generated at vault creation)
   │  fed to SQLCipher
   ▼
SQLCipher-encrypted vault.db   +   per-attachment content keys (each wrapped by DEK)
```

Concretely:

1. **Master password** is the primary unlock path. It is never persisted in any form.
2. **Argon2id** derives the KEK from the password using a per-vault random salt and parameters that are calibrated per device class and stored in `vault_metadata`. KDF parameters are versioned; rekey is supported (ADR 0002 covers the *model*; the calibration bead is `personal-cfo-0sqk` and the rekey bead is `personal-cfo-2y8`).
3. **DEK** is a random 256-bit key generated at vault creation and wrapped by the KEK (AES-KW or AES-GCM with a fixed nonce role). The wrapped DEK lives in the encrypted envelope on disk alongside vault metadata.
4. **SQLCipher** is keyed from the DEK at unlock time. The DEK lives only in app-controlled memory while the vault is unlocked; lock zeroizes it (`personal-cfo-1t0`).
5. **Attachments** each have a randomly-generated content key wrapped by the DEK. Filenames are stored only as encrypted metadata; on-disk filenames are opaque storage IDs (`personal-cfo-bcj`).
6. **Optional Touch ID** is a Keychain-stored wrapped form of unlock material, available only after a successful password unlock has established intent. Touch ID never replaces the password as the system of record. macOS Keychain is used per [Apple's guidance](https://developer.apple.com/documentation/security/keychain_services) for small secrets, with appropriate access controls.
7. **Versioned envelope.** The on-disk envelope carries a `vault_envelope_version` so we can migrate from v1 → v2 (e.g., upgraded Argon2id parameters, different AEAD primitive) without breaking older vaults.

The vault state machine (`personal-cfo-tg5`, §6.2.1) governs transitions: NoVault → CreatingVault → Locked → Unlocking → Unlocked → Locking → Rekeying → Migrating → RestoringBackup → CorruptNeedsRecovery.

## Canonical onboarding warning string

Because there is no cloud password reset by design (see the Negative consequence
below and §6.5.1), vault creation MUST present an unmissable confirmation step
that displays **exactly** this string — verbatim, including punctuation:

```
Personal CFO has no cloud password reset. If you forget your password, your data is unrecoverable. Save your password somewhere safe.
```

Feature code MUST treat this as the single source of truth via the constant
`CANONICAL_NO_RESET_WARNING` (defined in the `vault-crypto` crate and surfaced to
the frontend through the typed bindings) and MUST NOT re-type the literal. A unit
test asserts the rendered onboarding text equals the constant byte-for-byte, and
the user's acknowledgement is recorded as an `audit_event` with `command_id`,
`correlation_id`, and timestamp (`personal-cfo-n7bo`). Changing this copy is an
ADR change, not a code change.

## Consequences

### Positive

- Password change re-wraps the DEK; the encrypted DB itself does not need re-encryption (cheap, fast).
- Key rotation rotates the DEK (and thus all attachment-key wraps) without changing the password.
- Touch ID is genuinely optional: removing the Keychain entry does not lock the user out of their data.
- Attachment leakage outside SQLCipher is bounded by per-attachment keys; one compromised attachment doesn't compromise the rest.
- The state machine + startup self-test (`personal-cfo-tg5`) bounds half-open states (interrupted unlock / rekey / migration / restore).

### Negative

- Two layers of key wrapping (password→KEK→DEK) is more complex than a single-layer KDF→SQLCipher key. Mitigated by a small, well-tested vault crate.
- KDF calibration is hardware-dependent; we must enforce a minimum cost (≥ 250ms on dev hardware) in CI to prevent regression on faster machines.
- The Touch-ID-as-convenience model means the user must remember their password — there is no recovery from a lost master password by design. We do not implement an emergency recovery key in MVP (`personal-cfo-1ahq` reserves the slot but defers implementation per §6.5.1).

## Rejected alternatives

### macOS FileVault as the only at-rest protection

- ✗ FileVault encrypts at the disk level; an unlocked Mac with a logged-in user has plaintext access. We need app-level encryption that survives "user is logged in but app is locked."
- ✗ Doesn't help with backup portability or future Linux/Windows.

### macOS Keychain as the only secret store

- ✗ Keychain is suitable for small secrets, not multi-megabyte vault-key material.
- ✗ Doesn't give us the cross-platform path.
- ✗ Compromised user session = Keychain exposure.

### Plaintext SQLite + per-row field encryption

- ✗ Side files (WAL/SHM/temp/journal) leak plaintext; SQLCipher handles this transparently and we have a CI test (`personal-cfo-zxvl`) that validates it.
- ✗ Indexes and query optimization break on encrypted-field columns.

### Single password → SQLCipher key (no DEK layer)

- ✗ Password change requires full DB re-encryption (slow, high-risk on a large vault).
- ✗ KDF parameter migration requires the same.
- ✗ No clean path for attachment keys.

### Argon2i / scrypt / PBKDF2 instead of Argon2id

- ✗ Argon2id is the [PHC winner](https://www.password-hashing.net/) and the modern recommendation; Argon2i is more side-channel-resistant in narrow contexts but Argon2id is the practical default. PBKDF2 / scrypt have known weaknesses for memory-hard adversaries.

## Revisit if

- SQLCipher is EOL, has an unfixed CVE in our pinned line, or upstream guidance changes the recommended cipher mode.
- Argon2id is superseded by a future PHC recommendation.
- We add multi-device sync (ADR 0017): the vault model may need a separate device key wrapped by the DEK for sync envelope encryption.
- A real recovery-key requirement (per §6.5.1) becomes urgent — at that point we re-open this ADR and design the reserve slot.

## Implementation notes

- Rust crate: `vault-crypto` (under `crates/`); owns key derivation, wrap/unwrap, envelope serialization, and zeroization of in-memory key buffers.
- Argon2id parameters per device class are recorded in `docs/security/encryption-design.md` (one of the early required docs in the project profile) and asserted in CI by `personal-cfo-0sqk`.
- Envelope format version is part of `vault_metadata` (`personal-cfo-2lm`).

## Linked beads

- `personal-cfo-vhv` (FEATURE: Secure Vault module)
- `personal-cfo-tg5` (Vault state machine + startup self-test)
- `personal-cfo-3ry` (Implement vault creation flow)
- `personal-cfo-8v2` (Implement vault unlock/lock)
- `personal-cfo-bcj` (Encrypted attachment store)
- `personal-cfo-0sqk` (Calibrate Argon2id profiles per device class)
- `personal-cfo-2y8` (Implement key rotation / rekey)

## Addendum (2026-09-06): wording after the DohFlow rename

ADR 0067 renamed the product to DohFlow (`personal-cfo-fkt5.3`). The warning
keeps its meaning and punctuation; only the product name changes. The original
wording above stays as history, and `CANONICAL_NO_RESET_WARNING` now quotes this
string — verbatim, including punctuation:

```
DohFlow has no cloud password reset. If you forget your password, your data is unrecoverable. Save your password somewhere safe.
```
