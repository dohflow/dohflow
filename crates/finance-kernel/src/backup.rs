//! Encrypted backup container (ADR 0024, personal-cfo-ef3).
//!
//! A backup is a single file: a small **plaintext header** (magic, format
//! version, Argon2id salt + params, the wrapped backup DEK, and the payload
//! nonce) followed by an **AES-256-GCM-sealed payload** (a JSON manifest + the
//! vault envelope + `vault.db` + every attachment blob, length-framed). The
//! payload key is a random per-backup DEK wrapped under a password-derived KEK —
//! the same `password → KEK → DEK` model as the vault (ADR 0002).
//!
//! The inner artifacts are bundled **as-is** (already SQLCipher / AEAD
//! ciphertext); the only plaintext is the header, which carries no financial
//! data. Integrity is verified by **ciphertext** SHA-256 (ADR 0024 §3/§4): the
//! vault restores byte-identical, so an exact hash match is the check.
//!
//! [`assemble`] (export, `ef3`) and [`disassemble`] (restore, `au3`) are pure
//! bytes-in/bytes-out, so they round-trip in tests without touching the disk.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use vault_crypto::{
    backup::{open, seal},
    derive_kek, generate_dek, generate_salt, unwrap_dek, wrap_dek, Argon2Params, Profile, Salt,
    VaultCryptoError, ALGORITHM, SALT_LEN,
};

/// Container magic identifying a DohFlow backup.
const MAGIC: &[u8; 6] = b"PCFOBK";

/// On-disk container format version (ADR 0024). Bump on a byte-layout change.
const FORMAT_VERSION: u16 = 1;

/// The only KDF id serialized so far (mirrors [`ALGORITHM`]).
const ALGO_ARGON2ID: u8 = 1;

/// AES-GCM nonce length (96-bit).
const NONCE_LEN: usize = 12;

/// The redaction-policy version stamped in the manifest (plan §6.6 / `2vs`).
const REDACTION_POLICY_VERSION: u32 = 1;

/// One attachment blob's integrity record in the manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobEntry {
    /// Keyed content-address (HMAC over plaintext, ADR 0023) — a
    /// plaintext-identity check that needs no decryption.
    pub storage_id: String,
    /// Ciphertext size in bytes.
    pub size: u64,
    /// SHA-256 of the blob ciphertext (hex).
    pub ciphertext_sha256: String,
}

/// The verifiable index inside the sealed payload (ADR 0024 §3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// Container format version.
    pub format_version: u16,
    /// RFC 3339 creation instant (supplied by the caller — no clock here).
    pub created_at: String,
    /// Backup identity (UUIDv7 string).
    pub backup_id: String,
    /// Producing app version.
    pub app_version: String,
    /// DB schema version at export.
    pub schema_version: i64,
    /// Vault envelope format version.
    pub vault_envelope_version: u16,
    /// Redaction-policy version in force.
    pub redaction_policy_version: u32,
    /// SHA-256 of the `vault.db` ciphertext (hex).
    pub vault_db_sha256: String,
    /// SHA-256 of the envelope sidecar bytes (hex).
    pub envelope_sha256: String,
    /// Per-blob integrity records.
    pub blobs: Vec<BlobEntry>,
}

/// The vault bytes + provenance fed into [`assemble`].
pub struct BackupInputs {
    /// A consistent `vault.db` snapshot (SQLCipher ciphertext).
    pub db_bytes: Vec<u8>,
    /// The envelope sidecar bytes (`vault.db.envelope`).
    pub envelope_bytes: Vec<u8>,
    /// Attachment blobs as `(storage_id, ciphertext)`.
    pub blobs: Vec<(String, Vec<u8>)>,
    /// Producing app version (e.g. `CARGO_PKG_VERSION`).
    pub app_version: String,
    /// DB schema version.
    pub schema_version: i64,
    /// Vault envelope format version.
    pub vault_envelope_version: u16,
    /// RFC 3339 creation instant.
    pub created_at: String,
    /// Backup id (UUIDv7).
    pub backup_id: Uuid,
}

/// The bytes recovered from a package by [`disassemble`], with hashes verified.
pub struct RestoredBackup {
    /// The verified manifest.
    pub manifest: Manifest,
    /// The envelope sidecar bytes.
    pub envelope_bytes: Vec<u8>,
    /// The `vault.db` ciphertext bytes.
    pub db_bytes: Vec<u8>,
    /// Attachment blobs as `(storage_id, ciphertext)`.
    pub blobs: Vec<(String, Vec<u8>)>,
}

/// Backup assembly / parsing failures.
#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    /// A crypto step failed (KDF, wrap/unwrap, seal/open). A wrong password
    /// surfaces here as the payload AEAD failing to open.
    #[error("backup cryptography failed")]
    Crypto(#[from] VaultCryptoError),
    /// Manifest (JSON) serialization/deserialization failed.
    #[error("backup manifest is malformed")]
    Manifest(#[from] serde_json::Error),
    /// The container bytes were truncated, had a bad magic, or an unknown
    /// version.
    #[error("malformed backup package")]
    Malformed,
    /// A restored component's ciphertext hash did not match the manifest.
    #[error("backup integrity check failed: {0}")]
    IntegrityMismatch(String),
}

/// SHA-256 of `bytes`, lower-case hex.
fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn put_u32(out: &mut Vec<u8>, n: u32) {
    out.extend_from_slice(&n.to_be_bytes());
}
fn put_u64(out: &mut Vec<u8>, n: u64) {
    out.extend_from_slice(&n.to_be_bytes());
}
fn put_bytes_u32(out: &mut Vec<u8>, b: &[u8]) {
    put_u32(out, u32::try_from(b.len()).unwrap_or(u32::MAX));
    out.extend_from_slice(b);
}
fn put_bytes_u64(out: &mut Vec<u8>, b: &[u8]) {
    put_u64(out, b.len() as u64);
    out.extend_from_slice(b);
}

/// Assemble the encrypted backup package bytes from `inputs` under `password`.
///
/// # Errors
/// [`BackupError`] if the manifest cannot be serialized or any crypto step
/// fails.
pub fn assemble(password: &[u8], inputs: &BackupInputs) -> Result<Vec<u8>, BackupError> {
    // 1. Manifest with ciphertext content-hashes (ADR 0024 §3).
    let manifest = Manifest {
        format_version: FORMAT_VERSION,
        created_at: inputs.created_at.clone(),
        backup_id: inputs.backup_id.to_string(),
        app_version: inputs.app_version.clone(),
        schema_version: inputs.schema_version,
        vault_envelope_version: inputs.vault_envelope_version,
        redaction_policy_version: REDACTION_POLICY_VERSION,
        vault_db_sha256: sha256_hex(&inputs.db_bytes),
        envelope_sha256: sha256_hex(&inputs.envelope_bytes),
        blobs: inputs
            .blobs
            .iter()
            .map(|(storage_id, ciphertext)| BlobEntry {
                storage_id: storage_id.clone(),
                size: ciphertext.len() as u64,
                ciphertext_sha256: sha256_hex(ciphertext),
            })
            .collect(),
    };
    let manifest_json = serde_json::to_vec(&manifest)?;

    // 2. Frame the payload: manifest | envelope | vault.db | blobs.
    let mut payload = Vec::new();
    put_bytes_u64(&mut payload, &manifest_json);
    put_bytes_u64(&mut payload, &inputs.envelope_bytes);
    put_bytes_u64(&mut payload, &inputs.db_bytes);
    put_u32(
        &mut payload,
        u32::try_from(inputs.blobs.len()).unwrap_or(u32::MAX),
    );
    for (storage_id, ciphertext) in &inputs.blobs {
        put_bytes_u32(&mut payload, storage_id.as_bytes());
        put_bytes_u64(&mut payload, ciphertext);
    }

    // 3. Backup key: random per-backup DEK wrapped under a password KEK
    //    (fresh salt, interactive Argon2id profile) — the ADR 0002 key model.
    let salt = generate_salt()?;
    let params = Profile::InteractiveDefault.params();
    let kek = derive_kek(password, &salt, &params)?;
    let backup_dek = generate_dek()?;
    let wrapped = wrap_dek(&kek, &backup_dek)?;
    let sealed = seal(&backup_dek, &payload)?;

    // 4. Write the container: plaintext header + sealed payload.
    let mut out = Vec::with_capacity(64 + sealed.ciphertext.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&FORMAT_VERSION.to_be_bytes());
    out.push(ALGO_ARGON2ID);
    out.extend_from_slice(&params.memory_kib.to_be_bytes());
    out.extend_from_slice(&params.time_cost.to_be_bytes());
    out.extend_from_slice(&params.parallelism.to_be_bytes());
    out.extend_from_slice(&params.version.to_be_bytes());
    out.extend_from_slice(salt.as_bytes());
    out.extend_from_slice(&wrapped.nonce);
    put_bytes_u32(&mut out, &wrapped.ciphertext);
    out.extend_from_slice(&sealed.nonce);
    put_bytes_u64(&mut out, &sealed.ciphertext);
    Ok(out)
}

/// Parse, decrypt, and verify a backup package under `password`.
///
/// # Errors
/// [`BackupError::Malformed`] on a bad/truncated container,
/// [`BackupError::Crypto`] on a wrong password (the payload fails to open), or
/// [`BackupError::IntegrityMismatch`] if a component hash disagrees with the
/// manifest.
pub fn disassemble(password: &[u8], package: &[u8]) -> Result<RestoredBackup, BackupError> {
    let mut r = Reader::new(package);
    if r.take(MAGIC.len())? != MAGIC {
        return Err(BackupError::Malformed);
    }
    if u16::from_be_bytes(r.take_arr::<2>()?) != FORMAT_VERSION {
        return Err(BackupError::Malformed);
    }
    if r.take_arr::<1>()?[0] != ALGO_ARGON2ID {
        return Err(BackupError::Malformed);
    }
    let params = Argon2Params {
        algorithm: ALGORITHM,
        memory_kib: r.u32()?,
        time_cost: r.u32()?,
        parallelism: r.u32()?,
        version: r.u32()?,
    };
    let salt = Salt::from_bytes(r.take_arr::<SALT_LEN>()?);
    let wrapped = vault_crypto::WrappedDek {
        nonce: r.take_arr::<NONCE_LEN>()?,
        ciphertext: r.bytes_u32()?,
    };
    let sealed = vault_crypto::backup::SealedPayload {
        nonce: r.take_arr::<NONCE_LEN>()?,
        ciphertext: r.bytes_u64()?,
    };

    // Derive the KEK, unwrap the backup DEK, open the payload. A wrong password
    // makes `unwrap_dek` fail (its AEAD tag), surfaced as `Crypto`.
    let kek = derive_kek(password, &salt, &params)?;
    let backup_dek = unwrap_dek(&kek, &wrapped)?;
    let payload = open(&backup_dek, &sealed)?;

    // Unframe and verify against the manifest.
    let mut p = Reader::new(&payload);
    let manifest: Manifest = serde_json::from_slice(&p.bytes_u64()?)?;
    let envelope_bytes = p.bytes_u64()?;
    let db_bytes = p.bytes_u64()?;
    let blob_count = p.u32()? as usize;
    let mut blobs = Vec::with_capacity(blob_count);
    for _ in 0..blob_count {
        let storage_id = String::from_utf8(p.bytes_u32()?).map_err(|_| BackupError::Malformed)?;
        blobs.push((storage_id, p.bytes_u64()?));
    }

    verify(&manifest, &db_bytes, &envelope_bytes, &blobs)?;
    Ok(RestoredBackup {
        manifest,
        envelope_bytes,
        db_bytes,
        blobs,
    })
}

/// Check every component's ciphertext SHA-256 against the manifest (ADR 0024 §4).
fn verify(
    manifest: &Manifest,
    db_bytes: &[u8],
    envelope_bytes: &[u8],
    blobs: &[(String, Vec<u8>)],
) -> Result<(), BackupError> {
    if sha256_hex(db_bytes) != manifest.vault_db_sha256 {
        return Err(BackupError::IntegrityMismatch("vault.db".into()));
    }
    if sha256_hex(envelope_bytes) != manifest.envelope_sha256 {
        return Err(BackupError::IntegrityMismatch("envelope".into()));
    }
    if blobs.len() != manifest.blobs.len() {
        return Err(BackupError::IntegrityMismatch("blob count".into()));
    }
    for ((storage_id, ciphertext), entry) in blobs.iter().zip(&manifest.blobs) {
        if storage_id != &entry.storage_id || sha256_hex(ciphertext) != entry.ciphertext_sha256 {
            return Err(BackupError::IntegrityMismatch(format!("blob {storage_id}")));
        }
    }
    Ok(())
}

/// A minimal big-endian reader with bounds checks (mirrors the envelope reader).
struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], BackupError> {
        let end = self.pos.checked_add(n).ok_or(BackupError::Malformed)?;
        let slice = self.buf.get(self.pos..end).ok_or(BackupError::Malformed)?;
        self.pos = end;
        Ok(slice)
    }
    fn take_arr<const N: usize>(&mut self) -> Result<[u8; N], BackupError> {
        let mut arr = [0u8; N];
        arr.copy_from_slice(self.take(N)?);
        Ok(arr)
    }
    fn u32(&mut self) -> Result<u32, BackupError> {
        Ok(u32::from_be_bytes(self.take_arr::<4>()?))
    }
    fn u64(&mut self) -> Result<u64, BackupError> {
        Ok(u64::from_be_bytes(self.take_arr::<8>()?))
    }
    fn bytes_u32(&mut self) -> Result<Vec<u8>, BackupError> {
        let n = self.u32()? as usize;
        Ok(self.take(n)?.to_vec())
    }
    fn bytes_u64(&mut self) -> Result<Vec<u8>, BackupError> {
        let n = usize::try_from(self.u64()?).map_err(|_| BackupError::Malformed)?;
        Ok(self.take(n)?.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs() -> BackupInputs {
        BackupInputs {
            db_bytes: b"SQLCipher ciphertext bytes".to_vec(),
            envelope_bytes: b"PCFOVLT envelope bytes".to_vec(),
            blobs: vec![
                ("a3f9".into(), b"blob-one-ciphertext".to_vec()),
                ("7b20".into(), b"blob-two-ciphertext".to_vec()),
            ],
            app_version: "0.1.0".into(),
            schema_version: 5,
            vault_envelope_version: 1,
            created_at: "2026-06-20T00:00:00Z".into(),
            backup_id: Uuid::from_bytes([7u8; 16]),
        }
    }

    #[test]
    fn assemble_then_disassemble_round_trips() {
        let pkg = assemble(b"correct horse", &inputs()).unwrap();
        let restored = disassemble(b"correct horse", &pkg).unwrap();
        assert_eq!(restored.db_bytes, inputs().db_bytes);
        assert_eq!(restored.envelope_bytes, inputs().envelope_bytes);
        assert_eq!(restored.blobs, inputs().blobs);
        assert_eq!(restored.manifest.schema_version, 5);
        assert_eq!(restored.manifest.backup_id, inputs().backup_id.to_string());
        assert_eq!(restored.manifest.blobs.len(), 2);
    }

    #[test]
    fn wrong_password_fails_to_open() {
        let pkg = assemble(b"right", &inputs()).unwrap();
        assert!(matches!(
            disassemble(b"wrong", &pkg),
            Err(BackupError::Crypto(_))
        ));
    }

    #[test]
    fn no_plaintext_vault_bytes_in_the_package() {
        // The only plaintext is the header (magic + versions + salt + KDF params);
        // the bundled vault bytes must not appear in the clear.
        let pkg = assemble(b"pw", &inputs()).unwrap();
        for needle in [
            b"SQLCipher ciphertext bytes".as_slice(),
            b"blob-one-ciphertext".as_slice(),
            b"PCFOVLT envelope bytes".as_slice(),
        ] {
            assert!(
                !pkg.windows(needle.len()).any(|w| w == needle),
                "bundled bytes leaked into the package in the clear"
            );
        }
    }

    #[test]
    fn tampered_payload_is_rejected() {
        let mut pkg = assemble(b"pw", &inputs()).unwrap();
        let last = pkg.len() - 1;
        pkg[last] ^= 0xff;
        assert!(disassemble(b"pw", &pkg).is_err());
    }

    #[test]
    fn truncated_package_is_malformed() {
        let pkg = assemble(b"pw", &inputs()).unwrap();
        assert!(matches!(
            disassemble(b"pw", &pkg[..pkg.len() / 2]),
            Err(BackupError::Malformed | BackupError::Crypto(_))
        ));
    }
}
