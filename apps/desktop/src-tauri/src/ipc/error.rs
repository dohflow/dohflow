//! The typed error surface returned to the frontend (personal-cfo-40t).
//!
//! [`IpcError`] is the *only* error the frontend ever sees. It is deliberately
//! flat and owned (no borrowed or database types): a [`KernelError`] is mapped
//! into it, never re-exported, so no persistence/`rusqlite` detail leaks across
//! the IPC boundary. It derives [`specta::Type`] so the generated TypeScript
//! bindings carry the exact variant shape.

use finance_kernel::KernelError;
use serde::Serialize;
use specta::Type;

/// An error crossing the Tauri IPC boundary. Serialized to the frontend as an
/// externally-tagged union (e.g. `{ "Validation": "..." }` or `"VaultLocked"`).
#[derive(Debug, Clone, Serialize, Type, thiserror::Error)]
pub enum IpcError {
    /// The request failed domain or input validation (bad UUID, unknown
    /// currency, empty name, …). Safe to show the message to the user.
    #[error("validation failed: {0}")]
    Validation(String),

    /// No vault is open. The caller must open/unlock a vault first. This is the
    /// steady state of a locked app.
    #[error("no vault is open")]
    VaultLocked,

    /// Unlocking failed because the password was wrong (the wrapped DEK failed
    /// to authenticate). Carries no detail — there is no oracle distinguishing a
    /// wrong password from a tampered envelope. The unlock screen surfaces this
    /// as "incorrect password".
    #[error("the password is incorrect")]
    VaultUnlockFailed,

    /// The vault is open but not accepting writes (e.g. restoring a backup or
    /// awaiting recovery). Carries the worker state as a non-sensitive label.
    #[error("vault unavailable: {0}")]
    Unavailable(String),

    /// A writer panic rolled back the transaction; the vault needs recovery.
    #[error("a write failed and the vault needs recovery")]
    WriterPanicked,

    /// Any other persistence failure, flattened to a message so no database
    /// type is exposed.
    #[error("persistence error: {0}")]
    Persistence(String),
}

impl From<KernelError> for IpcError {
    fn from(error: KernelError) -> Self {
        match error {
            KernelError::Validation(message) => IpcError::Validation(message),
            KernelError::MissingMetadata(field) => {
                IpcError::Validation(format!("missing required field: {field}"))
            }
            KernelError::Unavailable(state) => IpcError::Unavailable(format!("{state:?}")),
            KernelError::WriterPanicked => IpcError::WriterPanicked,
            KernelError::Persistence(message) => IpcError::Persistence(message),
            KernelError::VaultUnlockFailed => IpcError::VaultUnlockFailed,
            KernelError::VaultExists => {
                IpcError::Validation("a vault already exists at this location".to_owned())
            }
            KernelError::VaultNotFound => {
                IpcError::Validation("no vault exists at this location".to_owned())
            }
            // `KernelError` is `#[non_exhaustive]`; any other variant (a vault
            // I/O error, an illegal transition — an internal invariant) collapses
            // to a non-leaky persistence error rather than failing to compile.
            other => IpcError::Persistence(other.to_string()),
        }
    }
}
