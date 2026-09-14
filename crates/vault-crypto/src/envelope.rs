//! The DEK envelope (personal-cfo-vhv, ADR 0002).
//!
//! A random 256-bit **Data Encryption Key (DEK)** is generated once at vault
//! creation and used directly as the SQLCipher raw key. The DEK itself is
//! wrapped (AEAD-encrypted) by the **KEK** that [`crate::derive_kek`] produces
//! from the master password, so the only persisted form of the DEK is
//! ciphertext that is useless without the password.
//!
//! The wrapped DEK, the per-vault salt, and the KDF parameters travel together
//! in a [`VaultEnvelope`], which is serialized to a **plaintext sidecar file**
//! next to the encrypted database: these values are needed *before* the DB can
//! be decrypted, so they cannot live inside it. The wrapped DEK's
//! confidentiality rests entirely on the password, not on the sidecar's secrecy.
//!
//! AEAD is **AES-256-GCM** with a fresh random 96-bit nonce per wrap (ADR 0002).

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use zeroize::Zeroize;

use crate::{Argon2Params, Kek, Salt, VaultCryptoError, ALGORITHM, SALT_LEN};

/// Length of a Data Encryption Key, in bytes (256-bit, the SQLCipher raw key).
pub const DEK_LEN: usize = 32;

/// AES-GCM nonce length, in bytes (96-bit, the standard for GCM).
const NONCE_LEN: usize = 12;

/// Sidecar magic, identifying a DohFlow vault envelope.
const MAGIC: &[u8; 7] = b"PCFOVLT";

/// The only KDF algorithm id currently serialized (mirrors [`ALGORITHM`]).
const ALGO_ARGON2ID: u8 = 1;

/// On-disk envelope format version. Bumped only when the byte layout changes
/// (ADR 0002's `vault_envelope_version`).
pub const ENVELOPE_VERSION: u16 = 1;

/// A Data Encryption Key: the random 256-bit key fed to SQLCipher.
///
/// Like [`Kek`], it is backed by [`crate::SecretBytes`] (mlock'd, zeroize on
/// drop, non-`Clone`) with a redacted `Debug` (§6.6).
pub struct Dek(crate::SecretBytes<DEK_LEN>);

impl Dek {
    /// Borrow the raw key bytes (e.g. to key SQLCipher in `db-worker`). Callers
    /// must not copy these into an un-zeroized buffer.
    #[must_use]
    pub fn expose_bytes(&self) -> &[u8; DEK_LEN] {
        self.0.expose()
    }

    /// Move `bytes` into a protected allocation, scrubbing the caller's copy.
    fn from_array(mut bytes: [u8; DEK_LEN]) -> Self {
        let dek = Dek(crate::SecretBytes::new(bytes));
        bytes.zeroize();
        dek
    }
}

impl Zeroize for Dek {
    fn zeroize(&mut self) {
        self.0.zeroize();
    }
}

impl core::fmt::Debug for Dek {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Dek([REDACTED])")
    }
}

/// Generate a fresh random DEK from the OS CSPRNG.
///
/// # Errors
/// Returns [`VaultCryptoError::Rng`] if the OS random source is unavailable.
pub fn generate_dek() -> Result<Dek, VaultCryptoError> {
    let mut bytes = [0u8; DEK_LEN];
    getrandom::getrandom(&mut bytes).map_err(|_| VaultCryptoError::Rng)?;
    Ok(Dek::from_array(bytes))
}

/// A DEK wrapped (AEAD-encrypted) by a KEK. Ciphertext + nonce — not secret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WrappedDek {
    /// The per-wrap random AES-GCM nonce.
    pub nonce: [u8; NONCE_LEN],
    /// AES-256-GCM ciphertext of the DEK (includes the authentication tag).
    pub ciphertext: Vec<u8>,
}

/// Wrap `dek` under `kek` with AES-256-GCM and a fresh random nonce.
///
/// # Errors
/// [`VaultCryptoError::Rng`] if the nonce cannot be sampled, or
/// [`VaultCryptoError::KeyWrap`] if the AEAD encryption fails.
pub fn wrap_dek(kek: &Kek, dek: &Dek) -> Result<WrappedDek, VaultCryptoError> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(kek.expose_bytes()));
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::getrandom(&mut nonce).map_err(|_| VaultCryptoError::Rng)?;
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), dek.expose_bytes().as_ref())
        .map_err(|_| VaultCryptoError::KeyWrap)?;
    Ok(WrappedDek { nonce, ciphertext })
}

/// Unwrap a [`WrappedDek`] using `kek`.
///
/// A wrong password derives a wrong KEK, which makes the AEAD tag fail to
/// verify — this is the gate that prevents the vault from opening under the
/// wrong password.
///
/// # Errors
/// [`VaultCryptoError::KeyUnwrap`] if the AEAD tag does not verify (wrong KEK)
/// or the recovered plaintext is not [`DEK_LEN`] bytes.
pub fn unwrap_dek(kek: &Kek, wrapped: &WrappedDek) -> Result<Dek, VaultCryptoError> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(kek.expose_bytes()));
    let mut plaintext = cipher
        .decrypt(
            Nonce::from_slice(&wrapped.nonce),
            wrapped.ciphertext.as_ref(),
        )
        .map_err(|_| VaultCryptoError::KeyUnwrap)?;
    if plaintext.len() != DEK_LEN {
        plaintext.zeroize();
        return Err(VaultCryptoError::KeyUnwrap);
    }
    let mut arr = [0u8; DEK_LEN];
    arr.copy_from_slice(&plaintext);
    plaintext.zeroize();
    Ok(Dek::from_array(arr))
}

/// Re-wrap an envelope's DEK under a **new** password — the change-password
/// seam (personal-cfo-zxq).
///
/// Unlocks `envelope` with `old_password` (derive the old KEK → unwrap the
/// DEK), then derives a new KEK from `new_password` with a **fresh salt** and
/// the caller-supplied `new_params` — the *current* KDF profile, never the old
/// envelope's parameters, so a rewrap always moves the vault forward to
/// today's calibration — and wraps the **same DEK** into a new envelope. The
/// DEK (and therefore the encrypted database it keys) never changes. Both KEKs
/// and the transient DEK are `SecretBytes`-backed and zeroize on drop.
///
/// # Errors
/// [`VaultCryptoError::KeyUnwrap`] if `old_password` is wrong (the AEAD tag
/// does not verify), or any KDF / RNG / wrap error from the primitives.
pub fn rewrap_envelope(
    envelope: &VaultEnvelope,
    old_password: &[u8],
    new_password: &[u8],
    new_params: Argon2Params,
) -> Result<VaultEnvelope, VaultCryptoError> {
    let old_kek = crate::derive_kek(old_password, &envelope.salt, &envelope.kdf)?;
    let dek = unwrap_dek(&old_kek, &envelope.wrapped)?;
    drop(old_kek); // zeroized; no longer needed once the DEK is recovered

    let new_salt = crate::generate_salt()?;
    let new_kek = crate::derive_kek(new_password, &new_salt, &new_params)?;
    let wrapped = wrap_dek(&new_kek, &dek)?;
    Ok(VaultEnvelope::new(new_params, new_salt, wrapped))
}

/// The plaintext envelope persisted alongside the encrypted vault. Carries the
/// unlock material: KDF parameters, the per-vault salt, and the wrapped DEK.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VaultEnvelope {
    /// On-disk format version ([`ENVELOPE_VERSION`]).
    pub version: u16,
    /// Argon2id parameters used to derive the KEK at unlock.
    pub kdf: Argon2Params,
    /// Per-vault random salt for the KDF.
    pub salt: Salt,
    /// The DEK wrapped under the KEK.
    pub wrapped: WrappedDek,
}

impl VaultEnvelope {
    /// Build an envelope at the current [`ENVELOPE_VERSION`].
    #[must_use]
    pub fn new(kdf: Argon2Params, salt: Salt, wrapped: WrappedDek) -> Self {
        Self {
            version: ENVELOPE_VERSION,
            kdf,
            salt,
            wrapped,
        }
    }

    /// Serialize to the sidecar byte format (big-endian, length-prefixed).
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64 + self.wrapped.ciphertext.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&self.version.to_be_bytes());
        out.push(ALGO_ARGON2ID);
        out.extend_from_slice(&self.kdf.memory_kib.to_be_bytes());
        out.extend_from_slice(&self.kdf.time_cost.to_be_bytes());
        out.extend_from_slice(&self.kdf.parallelism.to_be_bytes());
        out.extend_from_slice(&self.kdf.version.to_be_bytes());
        out.extend_from_slice(self.salt.as_bytes());
        out.extend_from_slice(&self.wrapped.nonce);
        // ciphertext length fits in u32 for any realistic key wrap.
        let ct_len = u32::try_from(self.wrapped.ciphertext.len()).unwrap_or(u32::MAX);
        out.extend_from_slice(&ct_len.to_be_bytes());
        out.extend_from_slice(&self.wrapped.ciphertext);
        out
    }

    /// Parse a sidecar produced by [`to_bytes`](Self::to_bytes).
    ///
    /// # Errors
    /// [`VaultCryptoError::UnsupportedEnvelopeVersion`] if the version is not
    /// understood, or [`VaultCryptoError::MalformedEnvelope`] for a bad magic,
    /// unknown KDF id, truncation, or trailing garbage.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, VaultCryptoError> {
        let mut r = Reader::new(bytes);
        if r.take(MAGIC.len())? != MAGIC {
            return Err(VaultCryptoError::MalformedEnvelope);
        }
        let version = u16::from_be_bytes(r.take_arr::<2>()?);
        if version != ENVELOPE_VERSION {
            return Err(VaultCryptoError::UnsupportedEnvelopeVersion);
        }
        if r.take_arr::<1>()?[0] != ALGO_ARGON2ID {
            return Err(VaultCryptoError::MalformedEnvelope);
        }
        let memory_kib = u32::from_be_bytes(r.take_arr::<4>()?);
        let time_cost = u32::from_be_bytes(r.take_arr::<4>()?);
        let parallelism = u32::from_be_bytes(r.take_arr::<4>()?);
        let params_version = u32::from_be_bytes(r.take_arr::<4>()?);
        let salt = Salt::from_bytes(r.take_arr::<SALT_LEN>()?);
        let nonce = r.take_arr::<NONCE_LEN>()?;
        let ct_len = u32::from_be_bytes(r.take_arr::<4>()?) as usize;
        let ciphertext = r.take(ct_len)?.to_vec();
        if !r.is_empty() {
            return Err(VaultCryptoError::MalformedEnvelope);
        }
        Ok(Self {
            version,
            kdf: Argon2Params {
                algorithm: ALGORITHM,
                memory_kib,
                time_cost,
                parallelism,
                version: params_version,
            },
            salt,
            wrapped: WrappedDek { nonce, ciphertext },
        })
    }
}

/// A minimal big-endian byte reader with bounds checks.
struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], VaultCryptoError> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or(VaultCryptoError::MalformedEnvelope)?;
        let slice = self
            .buf
            .get(self.pos..end)
            .ok_or(VaultCryptoError::MalformedEnvelope)?;
        self.pos = end;
        Ok(slice)
    }

    fn take_arr<const N: usize>(&mut self) -> Result<[u8; N], VaultCryptoError> {
        let mut arr = [0u8; N];
        arr.copy_from_slice(self.take(N)?);
        Ok(arr)
    }

    fn is_empty(&self) -> bool {
        self.pos >= self.buf.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{derive_kek, generate_salt, Profile};

    /// Cheap profile so tests don't pay the full interactive KDF cost.
    const fn cheap_params(memory_kib: u32) -> Argon2Params {
        Argon2Params {
            algorithm: ALGORITHM,
            memory_kib,
            time_cost: 1,
            parallelism: 1,
            version: 1,
        }
    }

    fn test_kek(password: &[u8], salt: &Salt) -> Kek {
        derive_kek(password, salt, &cheap_params(64)).unwrap()
    }

    #[test]
    fn wrap_then_unwrap_round_trips() {
        let salt = generate_salt().unwrap();
        let kek = test_kek(b"password", &salt);
        let dek = generate_dek().unwrap();
        let original = *dek.expose_bytes();

        let wrapped = wrap_dek(&kek, &dek).unwrap();
        let recovered = unwrap_dek(&kek, &wrapped).unwrap();
        assert_eq!(recovered.expose_bytes(), &original);
    }

    #[test]
    fn unwrap_with_wrong_kek_fails() {
        let salt = generate_salt().unwrap();
        let kek_right = test_kek(b"correct", &salt);
        let kek_wrong = test_kek(b"incorrect", &salt);
        let dek = generate_dek().unwrap();

        let wrapped = wrap_dek(&kek_right, &dek).unwrap();
        assert!(matches!(
            unwrap_dek(&kek_wrong, &wrapped),
            Err(VaultCryptoError::KeyUnwrap)
        ));
    }

    #[test]
    fn each_wrap_uses_a_fresh_nonce() {
        let salt = generate_salt().unwrap();
        let kek = test_kek(b"password", &salt);
        let dek = generate_dek().unwrap();
        let a = wrap_dek(&kek, &dek).unwrap();
        let b = wrap_dek(&kek, &dek).unwrap();
        assert_ne!(a.nonce, b.nonce);
        assert_ne!(a.ciphertext, b.ciphertext);
    }

    #[test]
    fn envelope_byte_round_trips() {
        let salt = generate_salt().unwrap();
        let kek = test_kek(b"password", &salt);
        let dek = generate_dek().unwrap();
        let wrapped = wrap_dek(&kek, &dek).unwrap();
        let env = VaultEnvelope::new(Profile::InteractiveDefault.params(), salt, wrapped);

        let bytes = env.to_bytes();
        let parsed = VaultEnvelope::from_bytes(&bytes).unwrap();
        assert_eq!(parsed, env);

        // And the parsed envelope still unwraps to the same DEK.
        let recovered = unwrap_dek(&kek, &parsed.wrapped).unwrap();
        assert_eq!(recovered.expose_bytes(), dek.expose_bytes());
    }

    #[test]
    fn truncated_or_garbage_envelopes_are_rejected() {
        let salt = generate_salt().unwrap();
        let kek = test_kek(b"password", &salt);
        let dek = generate_dek().unwrap();
        let wrapped = wrap_dek(&kek, &dek).unwrap();
        let env = VaultEnvelope::new(Profile::InteractiveDefault.params(), salt, wrapped);
        let bytes = env.to_bytes();

        assert!(matches!(
            VaultEnvelope::from_bytes(&bytes[..bytes.len() - 5]),
            Err(VaultCryptoError::MalformedEnvelope)
        ));
        assert!(matches!(
            VaultEnvelope::from_bytes(b"not a vault envelope at all"),
            Err(VaultCryptoError::MalformedEnvelope)
        ));
        // Trailing garbage is rejected.
        let mut extra = bytes.clone();
        extra.push(0);
        assert!(matches!(
            VaultEnvelope::from_bytes(&extra),
            Err(VaultCryptoError::MalformedEnvelope)
        ));
    }

    /// Build an envelope for `password` with cheap params, returning the raw DEK
    /// bytes too so rewrap tests can prove the DEK is preserved.
    fn envelope_for(password: &[u8]) -> (VaultEnvelope, [u8; DEK_LEN]) {
        let params = cheap_params(64);
        let salt = generate_salt().unwrap();
        let kek = derive_kek(password, &salt, &params).unwrap();
        let dek = generate_dek().unwrap();
        let original = *dek.expose_bytes();
        let wrapped = wrap_dek(&kek, &dek).unwrap();
        (VaultEnvelope::new(params, salt, wrapped), original)
    }

    #[test]
    fn rewrap_round_trips_the_same_dek_under_the_new_password_only() {
        let (env, original_dek) = envelope_for(b"old password");
        // Distinct new params prove the rewrap stamps the *given* profile
        // rather than copying the old envelope's parameters.
        let new_params = cheap_params(128);
        let renewed = rewrap_envelope(&env, b"old password", b"new password", new_params).unwrap();
        assert_eq!(
            renewed.kdf, new_params,
            "current profile stamped, not copied"
        );
        assert_ne!(
            renewed.salt.as_bytes(),
            env.salt.as_bytes(),
            "a fresh salt, never the old one"
        );

        // The new password recovers the SAME DEK — the data still decrypts.
        let new_kek = derive_kek(b"new password", &renewed.salt, &renewed.kdf).unwrap();
        let recovered = unwrap_dek(&new_kek, &renewed.wrapped).unwrap();
        assert_eq!(recovered.expose_bytes(), &original_dek);

        // The old password no longer opens the renewed envelope.
        let old_kek = derive_kek(b"old password", &renewed.salt, &renewed.kdf).unwrap();
        assert!(matches!(
            unwrap_dek(&old_kek, &renewed.wrapped),
            Err(VaultCryptoError::KeyUnwrap)
        ));
    }

    #[test]
    fn rewrap_with_the_wrong_old_password_fails_closed() {
        let (env, _) = envelope_for(b"old password");
        assert!(matches!(
            rewrap_envelope(
                &env,
                b"not the old password",
                b"new password",
                cheap_params(64)
            ),
            Err(VaultCryptoError::KeyUnwrap)
        ));
    }

    #[test]
    fn dek_debug_is_redacted() {
        let dek = generate_dek().unwrap();
        assert_eq!(format!("{dek:?}"), "Dek([REDACTED])");
    }
}
