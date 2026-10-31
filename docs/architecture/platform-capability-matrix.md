# Platform capability matrix

Status: living document. Source of truth for the **data** is `crates/platform-traits/src/matrix.rs`
(`MATRIX`); a test (`crates/platform-traits/tests/types.rs`, `the_matrix_document_matches_the_code`) fails when
this document and that data disagree, so change both in one commit. Decisions: ADR 0002 addendum 2026-10-24
(`personal-cfo-1zb`); plan section 3.3.1.

How to read it: **Yes** = implemented and tested; **Partial** = implemented, with the limitation in the note;
**Planned** = a named later bead, not available now; **No** = the platform cannot provide it or we deliberately do
not promise it.

Two rules that apply to every row, on every platform, now and in later implementations:

1. **Custody is not anti-rollback.** A secret store keeps a secret on this device. It does not detect that the
   device was restored to an older state, and **no store reports `rollback_resistant`**. The witness that ADR 0017
   requires for nonce state is a separate design (the Sync client engine bead, `personal-cfo-elciw`).
2. **Typed failure, never a silent fallback.** When a platform cannot provide the requested protection, the
   operation returns `Unavailable(<reason>)` and the app stays password-only. A secret is never stored somewhere
   weaker (not the macOS login keychain, not a plaintext file on Linux).

## Device-only secret storage (not synced, not restored to another device)

| Platform | Support | Note |
|---|---|---|
| macOS | Partial | Data-protection keychain only; needs the keychain entitlement and its provisioning profile (`personal-cfo-md8h2.17`). Typed Unavailable(MissingEntitlement) without it; never the login keychain. |
| iOS | Planned | Data-protection keychain by default; the shell uses it today, the trait implementation is a later bead. |
| Linux | Planned | Secret Service only (`personal-cfo-fkcij`); typed Unavailable(NoSecretService) when absent, never a plaintext fallback. |
| Windows | Planned | CRED_PERSIST_LOCAL_MACHINE or non-roaming DPAPI; never a roaming credential (`personal-cfo-rn9o`). |
| Android | Planned | A restored wrapped blob must yield NonceStateLost. |

## Biometric-gated secret (current biometric set, no passcode fallback)

| Platform | Support | Note |
|---|---|---|
| macOS | Partial | SecAccessControl .biometryCurrentSet on the data-protection keychain; needs the same entitlement; verified only on a Touch ID Mac with a signed build. |
| iOS | Planned | Same access control; Secure Enclave capable devices. |
| Linux | No | BiometricAuth reports unavailable in v1. |
| Windows | Planned | Windows Hello; not designed. |
| Android | Planned | BiometricPrompt with a keystore-bound key; not designed. |

## Distinct signal when the enrolled biometrics change

| Platform | Support | Note |
|---|---|---|
| macOS | No | Not promised: the item becomes inaccessible and may read as not-found; callers re-enroll with the vault password. |
| iOS | No | Same. |
| Linux | No | Not applicable. |
| Windows | No | Not designed. |
| Android | No | Not designed. |

## Anti-rollback witness (detect a restore of this device or vault to an older state)

| Platform | Support | Note |
|---|---|---|
| macOS | No | A keychain is custody, never the witness (ADR 0017). The witness design is owned by the Sync client engine bead (`personal-cfo-elciw`). |
| iOS | No | Same. |
| Linux | No | Same. |
| Windows | No | Same. |
| Android | No | Same. |

## Secret bytes wiped from platform-managed memory

| Platform | Support | Note |
|---|---|---|
| macOS | Partial | Our buffers are zeroized; the Security framework's own copies are not wiped (best effort). |
| iOS | Partial | Same. |
| Linux | Partial | Same shape for the Secret Service client library. |
| Windows | Partial | Not designed. |
| Android | Partial | Not designed. |

## Verification status

- Measured on macOS 27 (scratch program, throwaway service name): the login keychain silently drops the device-only
  class and the synchronizable flag; the data-protection keychain needs the keychain entitlement and answers
  `errSecMissingEntitlement` (-34018) without it.
- Measured in the signed harness (`scripts/keychain-smoke.sh`, `personal-cfo-md8h2.18`, first passing run
  2026-10-31, macOS 27.0.1 on a Mac with no enrolled biometrics, Developer ID identity, the real app id and
  profile): a device-only item round-trips in the data-protection keychain; its attributes read back
  `kSecAttrAccessibleWhenPasscodeSetThisDeviceOnly` (`akpu`), synchronizable `false` and the team-prefixed
  access group; the same binary without the entitlement fails with `MissingEntitlement`.
- Measured in the same run: with no enrolled biometrics the OS refuses to STORE a biometric-class item
  (`AuthFailed`), although `MacSecretStore::capabilities()` advertises `biometry_current_set`. The Touch ID
  bead (`personal-cfo-xuu`) must treat "biometrics unavailable" as a state before it stores such an item.
- Not yet verified: a notarized Developer ID build of the app itself (including whether a missing or expired
  profile stops it launching), and the Touch ID paths (a fresh match, a cancelled prompt, the conformance
  suite's biometric store). They need a Mac with Touch ID: run the harness with `KEYCHAIN_SMOKE_TOUCH_ID=1`
  and the manual `--ignored` tests from the signed bundle, then the Touch ID bead.
