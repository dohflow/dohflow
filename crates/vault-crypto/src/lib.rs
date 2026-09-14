//! Vault cryptography — Argon2id key derivation (personal-cfo-j0o).
//!
//! This crate turns a vault password into a **Key Encryption Key (KEK)** using
//! Argon2id with calibrated, versioned parameters. Per ADR 0002's envelope
//! model the KEK wraps a random Data Encryption Key (DEK); the DEK becomes the
//! SQLCipher raw key. This crate owns *only* the password → KEK step.
//!
//! # Trust boundary
//!
//! `vault-crypto` is a **pure, database-free** crate. It never imports
//! `rusqlite`, an async runtime, or the Tauri shell (enforced in CI). The KDF
//! takes the salt and parameters as arguments — it does not read or persist
//! them. The surrounding lifecycle is owned by sibling beads:
//!
//! - **`personal-cfo-vhv`** — the DEK envelope (wrap/unwrap) and the
//!   create/unlock/lock lifecycle that call [`derive_kek`]; persisting the salt
//!   and params to `vault_metadata` (including the `kdf_salt` column this crate
//!   does not add).
//! - **`personal-cfo-0sqk`** — per-device calibration and the CI floor asserting
//!   `memory >= 64 MiB && time_cost >= calibrated_minimum`.
//! - **`personal-cfo-1t0`** — comprehensive `mlock`/guard-page protection across
//!   *all* key buffers. This crate does its own slice: [`Kek`] zeroizes on drop.
//! - **`personal-cfo-2y8`** — key rotation / rekey and the
//!   [`Profile::LegacyCompatibility`] migration path.
//!
//! # Logging
//!
//! No key, password, or salt material is ever placed in an error message or
//! `Debug` output (plan §6.6). [`Kek`]'s `Debug` is redacted.

pub mod attachment;
pub mod backup;
pub mod envelope;
pub mod secret;

pub use attachment::{
    decrypt_blob, encrypt_blob, generate_content_key, storage_id, unwrap_content_key,
    wrap_content_key, ContentKey, EncryptedBlob, StorageId, WrappedContentKey,
};
pub use envelope::{
    generate_dek, rewrap_envelope, unwrap_dek, wrap_dek, Dek, VaultEnvelope, WrappedDek,
};
pub use secret::SecretBytes;
use zeroize::Zeroize;

/// Version stamp for the [`Argon2Params`] schema. Stored alongside the
/// parameters so a future change to the parameter *shape* (not just values) can
/// be detected and migrated. Bump only when the meaning of the fields changes.
pub const PARAMS_VERSION: u32 = 1;

/// The KDF algorithm token. Mirrors `vault_metadata.kdf_algorithm`.
pub const ALGORITHM: &str = "argon2id";

/// The canonical no-password-reset warning shown during vault creation
/// (ADR 0002 §"Canonical onboarding warning string", personal-cfo-n7bo).
///
/// This is the **single source of truth**: onboarding MUST render it verbatim and
/// MUST NOT re-type the literal — it is surfaced to the frontend through the typed
/// IPC command `no_reset_warning`. A test (`tests/no_reset_warning.rs`) pins it to
/// ADR 0002 byte-for-byte; changing this copy is an ADR change.
pub const CANONICAL_NO_RESET_WARNING: &str = "DohFlow has no cloud password reset. If you forget your password, your data is unrecoverable. Save your password somewhere safe.";

/// Length of a derived KEK, in bytes (256-bit).
pub const KEK_LEN: usize = 32;

/// Length of a per-vault salt, in bytes. Argon2 recommends >= 16.
pub const SALT_LEN: usize = 16;

/// A calibrated Argon2id parameter profile.
///
/// `InteractiveDefault` is the everyday unlock profile and **must** stay in
/// sync with `db-worker`'s seeded `vault_metadata` defaults (see the drift
/// guard in this crate's tests).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// Everyday unlock: ~64 MiB, tuned for a sub-second unlock (default).
    InteractiveDefault,
    /// Slower unlock, higher memory — for users who opt into stronger KDF cost.
    HighSecurity,
    /// Weaker parameters retained **only** to open older vaults until they are
    /// rekeyed to a current profile (`personal-cfo-2y8`). Never select for a
    /// freshly created vault.
    LegacyCompatibility,
}

impl Profile {
    /// The canonical Argon2id parameters for this profile.
    #[must_use]
    pub const fn params(self) -> Argon2Params {
        match self {
            // Byte-identical to db-worker's DEFAULT_KDF_* (argon2id, 65_536 KiB,
            // t=3, p=1) so a fresh vault's seeded metadata and the KDF agree.
            Profile::InteractiveDefault => Argon2Params {
                algorithm: ALGORITHM,
                memory_kib: 65_536,
                time_cost: 3,
                parallelism: 1,
                version: PARAMS_VERSION,
            },
            Profile::HighSecurity => Argon2Params {
                algorithm: ALGORITHM,
                memory_kib: 262_144, // 256 MiB
                time_cost: 4,
                parallelism: 1,
                version: PARAMS_VERSION,
            },
            // OWASP Argon2id floor (19 MiB, t=2). Open-only; rekey forward.
            Profile::LegacyCompatibility => Argon2Params {
                algorithm: ALGORITHM,
                memory_kib: 19_456,
                time_cost: 2,
                parallelism: 1,
                version: PARAMS_VERSION,
            },
        }
    }
}

/// Versioned Argon2id parameters.
///
/// Field names mirror the `vault_metadata.kdf_*` columns so the vault module
/// (`personal-cfo-vhv`) can map them 1:1 at the app layer without this crate
/// ever importing `db-worker`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Argon2Params {
    /// KDF algorithm token — always [`ALGORITHM`] for now.
    pub algorithm: &'static str,
    /// Memory cost in KiB (`m_cost`). Mirrors `kdf_memory_kib`.
    pub memory_kib: u32,
    /// Time cost / iterations (`t_cost`). Mirrors `kdf_time_cost`.
    pub time_cost: u32,
    /// Parallelism / lanes (`p_cost`). Mirrors `kdf_parallelism`.
    pub parallelism: u32,
    /// [`PARAMS_VERSION`] stamp this set was produced under.
    pub version: u32,
}

/// A per-vault random salt for Argon2id. Public, not secret.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Salt([u8; SALT_LEN]);

impl Salt {
    /// Construct a salt from raw bytes (e.g. read back from `vault_metadata`).
    #[must_use]
    pub const fn from_bytes(bytes: [u8; SALT_LEN]) -> Self {
        Self(bytes)
    }

    /// The raw salt bytes, for persistence.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; SALT_LEN] {
        &self.0
    }
}

/// Generate a fresh per-vault salt from the operating-system CSPRNG.
///
/// # Errors
/// Returns [`VaultCryptoError::Rng`] if the OS random source is unavailable.
pub fn generate_salt() -> Result<Salt, VaultCryptoError> {
    let mut bytes = [0u8; SALT_LEN];
    getrandom::getrandom(&mut bytes).map_err(|_| VaultCryptoError::Rng)?;
    Ok(Salt(bytes))
}

/// A derived Key Encryption Key.
///
/// Backed by [`SecretBytes`] (personal-cfo-1t0): the 256-bit key lives in a
/// best-effort `mlock`ed, zeroize-on-drop allocation, and the type is
/// deliberately **not** `Clone`/`Copy` so the compiler rejects accidental key
/// duplication. Its `Debug` is redacted so it can never leak into logs (§6.6).
pub struct Kek(SecretBytes<KEK_LEN>);

impl Kek {
    /// Borrow the raw key bytes (e.g. to wrap a DEK in `personal-cfo-vhv`).
    /// Callers must not copy these into an un-zeroized buffer.
    #[must_use]
    pub fn expose_bytes(&self) -> &[u8; KEK_LEN] {
        self.0.expose()
    }
}

impl Zeroize for Kek {
    fn zeroize(&mut self) {
        self.0.zeroize();
    }
}

impl core::fmt::Debug for Kek {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Kek([REDACTED])")
    }
}

/// Errors from vault-crypto. Messages never embed key, password, or salt bytes.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum VaultCryptoError {
    /// Parameters were rejected by Argon2 (e.g. `memory_kib` below the
    /// algorithm minimum, or `parallelism` of zero).
    #[error("invalid Argon2id parameters")]
    InvalidParams,
    /// Argon2id derivation failed.
    #[error("key derivation failed")]
    Derivation,
    /// The OS random source was unavailable.
    #[error("secure random source unavailable")]
    Rng,
    /// Wrapping the DEK under the KEK (AEAD encryption) failed.
    #[error("key wrap failed")]
    KeyWrap,
    /// Unwrapping the DEK failed: the AEAD tag did not verify (wrong KEK ⇐ wrong
    /// password) or the recovered key was the wrong length.
    #[error("key unwrap failed")]
    KeyUnwrap,
    /// The vault envelope bytes were malformed (bad magic, unknown KDF id,
    /// truncation, or trailing garbage).
    #[error("malformed vault envelope")]
    MalformedEnvelope,
    /// The vault envelope used a format version this build does not understand.
    #[error("unsupported vault envelope version")]
    UnsupportedEnvelopeVersion,
}

/// Derive a Key Encryption Key from a password using Argon2id.
///
/// The password buffer is owned by the caller and is **not** copied here; the
/// caller is responsible for zeroizing it (typically via `personal-cfo-1t0`'s
/// protected buffers). The returned [`Kek`] zeroizes itself on drop.
///
/// Argon2id is data-independent, so derivation latency does not depend on
/// whether the password is "correct" — wrong-password latency stays within the
/// constant-time envelope (asserted by the timing-regression test).
///
/// # Errors
/// - [`VaultCryptoError::InvalidParams`] if `params` are rejected by Argon2.
/// - [`VaultCryptoError::Derivation`] if the hashing step itself fails.
pub fn derive_kek(
    password: &[u8],
    salt: &Salt,
    params: &Argon2Params,
) -> Result<Kek, VaultCryptoError> {
    use argon2::{Algorithm, Argon2, Params, Version};

    // `output_len = None` lets the output buffer length (KEK_LEN) drive the tag
    // size, avoiding a length-mismatch error in `hash_password_into`.
    let argon_params = Params::new(
        params.memory_kib,
        params.time_cost,
        params.parallelism,
        None,
    )
    .map_err(|_| VaultCryptoError::InvalidParams)?;

    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, argon_params);

    let mut kek = [0u8; KEK_LEN];
    let result = argon.hash_password_into(password, salt.as_bytes(), &mut kek);
    if result.is_err() {
        kek.zeroize();
        return Err(VaultCryptoError::Derivation);
    }

    // Move the bytes into a protected (mlock'd, zeroizing) allocation, then
    // scrub the transient stack copy.
    let out = Kek(SecretBytes::new(kek));
    kek.zeroize();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeroize::Zeroize;

    /// A fast profile so unit tests don't pay the full interactive KDF cost.
    /// (Argon2 floor: m_cost >= 8 * p_cost.)
    const fn test_params() -> Argon2Params {
        Argon2Params {
            algorithm: ALGORITHM,
            memory_kib: 64,
            time_cost: 1,
            parallelism: 1,
            version: PARAMS_VERSION,
        }
    }

    #[test]
    fn interactive_default_meets_floor_and_matches_db_worker_seed() {
        let p = Profile::InteractiveDefault.params();
        // ADR 0002 / Risk §24 floor.
        assert!(p.memory_kib >= 65_536, "interactive must use >= 64 MiB");
        assert!(p.time_cost >= 3);
        assert_eq!(p.algorithm, "argon2id");
        // Drift guard: these MUST equal db-worker's seeded DEFAULT_KDF_* values
        // (crates/db-worker/src/lib.rs). If you change one, change both.
        assert_eq!(p.memory_kib, 65_536);
        assert_eq!(p.time_cost, 3);
        assert_eq!(p.parallelism, 1);
    }

    #[test]
    fn high_security_is_costlier_than_interactive() {
        let hi = Profile::HighSecurity.params();
        let it = Profile::InteractiveDefault.params();
        assert!(hi.memory_kib >= it.memory_kib);
        assert!(hi.time_cost >= it.time_cost);
    }

    #[test]
    fn derivation_is_deterministic() {
        let salt = Salt::from_bytes([7u8; SALT_LEN]);
        let a = derive_kek(b"correct horse", &salt, &test_params()).unwrap();
        let b = derive_kek(b"correct horse", &salt, &test_params()).unwrap();
        assert_eq!(a.expose_bytes(), b.expose_bytes());
    }

    #[test]
    fn different_salt_yields_different_kek() {
        let p = test_params();
        let a = derive_kek(b"pw", &Salt::from_bytes([1u8; SALT_LEN]), &p).unwrap();
        let b = derive_kek(b"pw", &Salt::from_bytes([2u8; SALT_LEN]), &p).unwrap();
        assert_ne!(a.expose_bytes(), b.expose_bytes());
    }

    #[test]
    fn different_password_yields_different_kek() {
        let salt = Salt::from_bytes([9u8; SALT_LEN]);
        let a = derive_kek(b"password-a", &salt, &test_params()).unwrap();
        let b = derive_kek(b"password-b", &salt, &test_params()).unwrap();
        assert_ne!(a.expose_bytes(), b.expose_bytes());
    }

    #[test]
    fn generated_salts_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..256 {
            assert!(seen.insert(*generate_salt().unwrap().as_bytes()));
        }
    }

    #[test]
    fn invalid_params_are_rejected_without_panicking() {
        // parallelism = 0 is below Argon2's minimum.
        let bad = Argon2Params {
            algorithm: ALGORITHM,
            memory_kib: 64,
            time_cost: 1,
            parallelism: 0,
            version: PARAMS_VERSION,
        };
        let salt = Salt::from_bytes([0u8; SALT_LEN]);
        assert!(matches!(
            derive_kek(b"pw", &salt, &bad),
            Err(VaultCryptoError::InvalidParams)
        ));
    }

    #[test]
    fn kek_debug_is_redacted() {
        let salt = Salt::from_bytes([3u8; SALT_LEN]);
        let kek = derive_kek(b"pw", &salt, &test_params()).unwrap();
        assert_eq!(format!("{kek:?}"), "Kek([REDACTED])");
    }

    #[test]
    fn kek_buffer_is_scrubbed_by_zeroize() {
        // `ZeroizeOnDrop` runs this same `Zeroize` impl on drop; calling it
        // directly asserts the buffer is scrubbed without the UB of reading
        // freed memory after a drop.
        let salt = Salt::from_bytes([5u8; SALT_LEN]);
        let mut kek = derive_kek(b"pw", &salt, &test_params()).unwrap();
        assert_ne!(*kek.expose_bytes(), [0u8; KEK_LEN]);
        kek.zeroize();
        assert_eq!(*kek.expose_bytes(), [0u8; KEK_LEN]);
    }
}
