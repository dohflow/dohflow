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
7. **Versioned envelope.** The on-disk envelope carries a
   `vault_envelope_version` so we can migrate from v1 → v2 (e.g., upgraded
   Argon2id parameters, different AEAD primitive) without breaking older vaults.
   ADR 0024 Addendum A (`personal-cfo-lhouc`) carries these serialized envelope
   bytes in a format-version-2 backup header so an unattended backup remains
   recoverable with the vault password and introduces no new secret.
   Sync's separate, per-device-sealed epoch root is governed by
   [ADR 0074 Decision 8](0074-dohflow-sync-architecture.md), not by this
   local envelope or a backup-restored device key.

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
- ~~We add multi-device sync (ADR 0017): the vault model may need a separate device key wrapped by the DEK for sync envelope encryption.~~ **Fired 2026-09-20:** ADR 0074 defines a separate, per-device-sealed Sync key distinct from the vault DEK; it does not alter the password or backup-recovery hierarchy and introduces no recovery escrow in v1.
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

## Addendum (2026-10-24, `personal-cfo-1zb`): how secrets are kept on the device (Keychain, Touch ID, the platform traits)

Decision 6 above says Touch ID is "a Keychain-stored wrapped form of unlock material, with appropriate access
controls." This addendum makes "appropriate" concrete before any code depends on it. It governs the two platform
traits (`SecretStore`, `BiometricAuth`; plan §3.3.1), their macOS implementation, and every later consumer (Touch ID
unlock, `personal-cfo-xuu`; Sync's device-only state, ADR 0017 and ADR 0074 Decision 8).

### What was measured, not assumed

On macOS 27 (a scratch program with a throwaway service name; its items were deleted):

- The **file-based (login) keychain** accepts an item written with the accessibility class
  `WhenPasscodeSetThisDeviceOnly` and `kSecAttrSynchronizable = false`, **but silently stores neither**: reading the
  item back shows no accessibility class and no synchronizable flag. A test that asserts those attributes cannot pass
  there, and nothing there stops the item travelling with the login keychain (Time Machine, Migration Assistant).
- The **data-protection keychain** is the one that stores them, and the only one that offers biometric access control.
  For a binary with no matching entitlement (unsigned or ad-hoc signed, which is every `cargo test` and development
  run) it answers `errSecMissingEntitlement` (-34018), including for biometric access control.

### Decisions

1. **Data-protection keychain only, on macOS.** `platform-macos` writes every item with
   `kSecUseDataProtectionKeychain`. The login keychain is never used, for any secret, as a fallback.
2. **No silent fallback.** When the entitlement is missing, every operation returns the typed
   `SecretError::Unavailable(MissingEntitlement)`. The app stays password-only (ADR 0002 decision 6: the password is
   the system of record) and says so on the Settings vault card. A development build without the entitlement therefore
   cannot use Touch ID or store Sync device state, by design, and a test run in that state **asserts exactly that
   error** so an unentitled run can never read as a pass.
3. **Custody class.** Every item is device-only: `kSecAttrAccessibleWhenPasscodeSetThisDeviceOnly` and
   `kSecAttrSynchronizable = false`, with an explicit team-prefixed `kSecAttrAccessGroup` (item placement is a decision,
   not "the first group in the entitlement list"). A test asserts both attributes on a stored item where the
   entitlement is present.
4. **Biometrics release secrets only through the keychain's own access control.** A secret that needs Touch ID is
   stored with `SecAccessControl` `.biometryCurrentSet` and **no passcode fallback** (the vault password is the only
   fallback, and it is the unlock path itself, not a keychain path); the `LAContext` is handed to the keychain call
   (`kSecUseAuthenticationContext`). `BiometricAuth` is **availability and user experience only** (`is_available`, and
   an optional pre-flight prompt): a standalone biometric match is a screen gate, not cryptography, so **no code path
   may load a secret because an `authenticate` call returned a match.** The trait names its prompt method accordingly
   and a test or lint checks that no `SecretStore` call is conditioned on it.
5. **Custody, not anti-rollback.** `ThisDeviceOnly` stops an item moving to another device; it does **not** stop the
   same device being rolled back to an older state (a restore from its own backup). ADR 0017 requires an independent
   anti-rollback witness for nonce state for exactly that reason, and **this keychain is never that witness.** The
   trait's capability report therefore carries two separate facts, `roams_to_other_device` and
   `rollback_resistant` (always `false` for the keychain), and the capability matrix says so. Which component is the
   macOS witness is not decided here; the requirement is ADR 0017's (decision bead `personal-cfo-6kn`), the
   implementation belongs to the Sync client engine bead (`personal-cfo-elciw`), and nobody should read
   "ThisDeviceOnly" as satisfying ADR 0017.
6. **A `BiometryChanged` signal is not promised.** After the enrolled biometrics change, a `.biometryCurrentSet` item
   becomes inaccessible, and the platform may report that as "not found" or as an authentication failure rather than a
   distinct code. Callers treat "not found" for such an item as "enroll again with the vault password". A distinct
   `BiometryChanged` error exists only if it is observed on a Touch ID Mac.
7. **Residue we cannot wipe.** The Security framework copies bytes into memory it manages and frees without wiping.
   Secrets are copied into an exactly sized zeroizing buffer immediately and the surrounding calls are kept short; the
   platform copies are best effort, the same posture as `vault-crypto`. The redaction rules are unchanged: no secret
   value, and no error text that could echo one, enters a span, a log line or an error message (typed errors carry
   fixed strings and an `OSStatus` only).

### The dependency this creates (and its risk)

The data-protection keychain needs the restricted entitlements `com.apple.application-identifier` and
`keychain-access-groups`. A Developer ID app can carry them only if they are authorized by a **provisioning profile
embedded in the app bundle** (an Apple Developer account action by the Account Holder: an App ID with Keychain
Sharing, and a Developer ID profile). Today's release is Developer ID signed with the hardened runtime and has neither
an entitlements file nor a profile. That work is a separate release-engineering bead that blocks the Touch ID bead,
not the trait crates. Its risks, recorded here so they are not discovered late: an entitlement that a profile does not
authorize (including an expired profile) may make macOS **refuse to launch the app**, not merely disable the Keychain
[unverified; to be tested on a signed, notarized release candidate], so the release process needs (a) a gate that
launches the signed, notarized artifact and runs the entitled round trip and the attribute assertion before anything
is published, and (b) an expiry watch with lead time.

### Structure and scope

- `platform-traits` (traits, typed errors, the redacting value type, the capability matrix and a conformance suite
  every implementation runs), `platform-macos` (the only crate with `unsafe`, confined to one module),
  `platform-mock` (tests). The capability matrix lists macOS, Linux, Windows, iOS and Android per capability, with the
  rule for later implementations (Windows: non-roaming storage only; Android: a restored wrapped blob yields
  `NonceStateLost`).
- Nothing consumes these traits in the same change that introduces them.
- The Linux Secret Service implementation is split into its own bead; it needs a daemon on a Linux runner, a separate
  ADR for the crate choice, and nothing on the macOS path waits for it. The same rules apply there: a typed failure
  when no Secret Service is reachable, never a plaintext fallback.
- Touch ID itself cannot be exercised on a Mac without biometric hardware; its manual checks run on a Touch ID Mac and
  are an explicit checklist item on the consuming bead.

### Consequences and what stays open

- Touch ID unlock and Sync device state on macOS are blocked until the entitlement and profile bead lands. Password
  unlock is unaffected.
- The iOS shell already uses the data-protection keychain (an iOS app gets it by default) and is not blocked.
- Rotating the vault key (ADR 0083) must refresh or invalidate Touch ID material; that stays with the Touch ID bead.
- Open: the macOS anti-rollback witness design (decision 5), and the real behavior of a profile-bearing, notarized
  build (the dependency above).

## Addendum (2026-11-01, `personal-cfo-xuu`): runtime biometric availability, the public tag, and the access group

First of the four changes that build Touch ID unlock on the 2026-10-24 platform traits. It changes the traits
and the macOS implementation only; nothing consumes them yet.

1. **Biometric availability is a runtime state, asked each time.** Measured on a Mac with no enrolled biometrics:
   the OS refuses to store a biometric-gated item with `errSecAuthFailed`, which a caller cannot tell from a wrong
   finger. `BiometricAuth::availability()` returns `Available`, `NotEnrolled`, `NotPresent`, `LockedOut` or
   `Unknown` (`LAContext.canEvaluatePolicy`), is never cached, and an enrolled fingerprint is seen without a
   restart. `StoreCapabilities::biometry_current_set` now only says the store IMPLEMENTS the class. A biometric
   store is refused with the typed reason for the device's current state (`BiometryNotEnrolled`,
   `BiometryLockedOut`, `Unavailable(Unsupported)`) **before** anything is replaced, so a refusal never destroys
   an existing item. The conformance suite requires exactly that on a device that cannot, and the store on one
   that can.
2. **A public tag, readable without authentication.** An item may carry a small non-secret `PublicTag` (on macOS
   `kSecAttrGeneric`), replaced by every store and readable without any prompt, even for a biometric-gated item
   (`SecretStore::public_tag`). Its purpose is custody hygiene: the owner of an item can compare a fingerprint with
   the vault's current envelope and delete a stale item at launch, without asking the user to match a finger
   (03b's M1 on the Touch ID design: a crash between a rotation and the item's deletion must not leave a
   retired key behind). **It is not protected by the item's access control; never put a secret in it.**
3. **The access group comes from the app's own entitlements.** `platform-macos` reads the process's
   `keychain-access-groups` entitlement (`SecTaskCreateFromSelf`, in the one `unsafe` module). The first value must
   be exactly `<TeamID>.<bundle id>` (10-character uppercase team id and this app's bundle id); anything else (no
   entitlement, a wildcard, another app's group, a malformed team id) is treated as absent and every keychain call
   takes the typed `Unavailable(MissingEntitlement)` path. No team id is written in the source.
4. **Prompt problems have their own typed error** (`SecretError::InvalidPrompt`), and the pre-flight
   `LAContext` is invalidated however the call ends (a match, a cancel, a failure, a timeout), so no
   authenticated context lingers (the optional notes N1 and N2 from the first review of the traits).

The keychain is unavailable while the screen is locked (`Unavailable(Locked)`, `errSecInteractionNotAllowed`),
which the signed harness runs into if started on a locked Mac; that is the typed behavior, not a failure.
