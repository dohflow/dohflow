//! macOS implementations of the platform traits (`personal-cfo-1zb`; ADR 0002 addendum 2026-10-24).
//!
//! * [`MacSecretStore`]: the **data-protection keychain only**. Every item is device-only
//!   (`kSecAttrAccessibleWhenPasscodeSetThisDeviceOnly`, `kSecAttrSynchronizable = false`) with an
//!   explicit team-prefixed access group. When the app lacks the keychain entitlement (every
//!   unsigned or ad-hoc development build) every call returns
//!   `Unavailable(MissingEntitlement)`: there is **no fallback to the login keychain**, because that
//!   keychain silently drops the device-only class and the synchronizable flag.
//! * [`MacBiometricAuth`]: `LocalAuthentication`, availability and a pre-flight prompt only. It never
//!   releases a secret; a biometric-gated secret is released by the keychain's own access control.
//!
//! The whole crate is empty off macOS. `unsafe` is confined to the two modules that call Apple
//! frameworks (`keychain`, `auth`); `status` stays safe. Nothing here logs: the repository scan in
//! `platform-traits` enforces it.

#![deny(unsafe_code)]

#[cfg(target_os = "macos")]
mod auth;
#[cfg(target_os = "macos")]
mod keychain;
mod status;

#[cfg(target_os = "macos")]
pub use auth::MacBiometricAuth;
#[cfg(target_os = "macos")]
pub use keychain::{MacSecretStore, StoredAttributes};
pub use status::map_os_status;
