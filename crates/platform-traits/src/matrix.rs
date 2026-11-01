//! The platform capability matrix (plan §3.3.1; ADR 0002 addendum 2026-10-24). The human form is
//! `docs/architecture/platform-capability-matrix.md`; a test keeps the two in step.

/// A platform a capability can be asked of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    MacOs,
    Ios,
    Linux,
    Windows,
    Android,
}

/// How far a platform supports a capability today.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Support {
    /// Implemented and tested.
    Yes,
    /// Implemented, with the limitation in the note.
    Partial,
    /// A planned implementation (named in the note); not available now.
    Planned,
    /// The platform cannot provide it.
    No,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub support: Support,
    pub note: &'static str,
}

const fn cell(support: Support, note: &'static str) -> Cell {
    Cell { support, note }
}

#[derive(Debug, Clone, Copy)]
pub struct Row {
    pub capability: &'static str,
    pub macos: Cell,
    pub ios: Cell,
    pub linux: Cell,
    pub windows: Cell,
    pub android: Cell,
}

impl Row {
    #[must_use]
    pub const fn on(&self, platform: Platform) -> Cell {
        match platform {
            Platform::MacOs => self.macos,
            Platform::Ios => self.ios,
            Platform::Linux => self.linux,
            Platform::Windows => self.windows,
            Platform::Android => self.android,
        }
    }
}

use Support::{No, Partial, Planned};

/// One row per capability, one cell per platform. Changing a cell here without the document (or the
/// reverse) fails `tests/matrix_document.rs`.
pub const MATRIX: &[Row] = &[
    Row {
        capability: "Device-only secret storage (not synced, not restored to another device)",
        macos: cell(Partial, "Data-protection keychain only; needs the keychain entitlement and its provisioning profile (`personal-cfo-md8h2.17`). Typed Unavailable(MissingEntitlement) without it; never the login keychain."),
        ios: cell(Planned, "Data-protection keychain by default; the shell uses it today, the trait implementation is a later bead."),
        linux: cell(Planned, "Secret Service only (`personal-cfo-fkcij`); typed Unavailable(NoSecretService) when absent, never a plaintext fallback."),
        windows: cell(Planned, "CRED_PERSIST_LOCAL_MACHINE or non-roaming DPAPI; never a roaming credential (`personal-cfo-rn9o`)."),
        android: cell(Planned, "A restored wrapped blob must yield NonceStateLost."),
    },
    Row {
        capability: "Biometric-gated secret (current biometric set, no passcode fallback)",
        macos: cell(Partial, "SecAccessControl .biometryCurrentSet on the data-protection keychain; needs the same entitlement. Whether this Mac can create one right now is a RUNTIME fact (BiometricAuth::availability: available, not enrolled, not present, locked out), asked before storing and typed in the refusal; sensor paths verified only on a Touch ID Mac with a signed build."),
        ios: cell(Planned, "Same access control; Secure Enclave capable devices."),
        linux: cell(No, "BiometricAuth reports unavailable in v1."),
        windows: cell(Planned, "Windows Hello; not designed."),
        android: cell(Planned, "BiometricPrompt with a keystore-bound key; not designed."),
    },
    Row {
        capability: "Distinct signal when the enrolled biometrics change",
        macos: cell(No, "Not promised: the item becomes inaccessible and may read as not-found; callers re-enroll with the vault password."),
        ios: cell(No, "Same."),
        linux: cell(No, "Not applicable."),
        windows: cell(No, "Not designed."),
        android: cell(No, "Not designed."),
    },
    Row {
        capability: "Anti-rollback witness (detect a restore of this device or vault to an older state)",
        macos: cell(No, "A keychain is custody, never the witness (ADR 0017). The witness design is owned by the Sync client engine bead (`personal-cfo-elciw`)."),
        ios: cell(No, "Same."),
        linux: cell(No, "Same."),
        windows: cell(No, "Same."),
        android: cell(No, "Same."),
    },
    Row {
        capability: "Secret bytes wiped from platform-managed memory",
        macos: cell(Partial, "Our buffers are zeroized; the Security framework's own copies are not wiped (best effort)."),
        ios: cell(Partial, "Same."),
        linux: cell(Partial, "Same shape for the Secret Service client library."),
        windows: cell(Partial, "Not designed."),
        android: cell(Partial, "Not designed."),
    },
];
