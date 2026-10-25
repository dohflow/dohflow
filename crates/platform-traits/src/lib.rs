//! Platform service traits (plan §3.3.1; ADR 0002 addendum 2026-10-24, `personal-cfo-1zb`).
//!
//! The Finance Kernel stays portable by reaching the operating system only through the traits
//! here. This crate holds **no platform code and no `unsafe`**: the traits, the value types that
//! make secrets hard to leak, the typed errors, the capability matrix, and a conformance suite
//! every implementation (`platform-mock`, `platform-macos`, later Linux and iOS) must pass.
//!
//! The rules this crate encodes (see the ADR for the reasoning):
//!
//! * **Typed failure, never a silent fallback.** An implementation that cannot meet the requested
//!   protection returns [`SecretError::Unavailable`]; it never stores the secret somewhere weaker.
//! * **Custody is not anti-rollback.** A [`SecretStore`] keeps a secret on this device; it does not
//!   promise that the device cannot be rolled back to an older state. [`StoreCapabilities`]
//!   reports the two facts separately and no implementation may claim `rollback_resistant`.
//! * **Biometrics release secrets only through the store.** [`BiometricAuth`] is availability and
//!   user experience; a match from it must never be the condition for calling
//!   [`SecretStore::load_secret`]. A secret that needs a biometric is released by the platform's own
//!   access control, selected with [`AccessPolicy::DeviceOnlyBiometryCurrentSet`].
//! * **No secret in an error, a `Debug`, a log or a span.** [`SecretValue`] redacts itself, and
//!   [`SecretError`] carries only fixed text and a numeric platform status.

#![forbid(unsafe_code)]

mod biometric;
pub mod conformance;
mod error;
mod key;
pub mod matrix;
mod policy;
mod store;
mod value;

pub use biometric::{BiometricAuth, PreflightResult};
pub use error::{SecretError, UnavailableReason};
pub use key::{AccessGroup, SecretKey, SERVICE_PREFIX};
pub use policy::{AccessPolicy, AuthPrompt, MAX_PROMPT_CHARS};
pub use store::{Roaming, SecretStore, StoreCapabilities};
pub use value::{SecretValue, MAX_SECRET_BYTES};
