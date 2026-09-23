//! Encrypted backup container (ADR 0024, personal-cfo-ef3).
//!
//! A backup is a single file: a small **plaintext header** followed by an
//! **AES-256-GCM-sealed payload** (a JSON manifest + the vault envelope +
//! `vault.db` + every attachment blob, length-framed). Format v2 carries the
//! wrapped vault envelope in the header and wraps a random per-backup DEK under
//! an HKDF-derived key from the in-memory vault DEK (ADR 0024-A). Format v1
//! remains readable through its historical password-derived key chain.
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
use subtle::ConstantTimeEq;
use uuid::Uuid;

use vault_crypto::{
    backup::derive_backup_kek,
    backup::{open, seal},
    derive_kek, generate_dek, generate_salt, unwrap_dek, wrap_dek, Argon2Params, Profile, Salt,
    VaultCryptoError, VaultEnvelope, ALGORITHM, SALT_LEN,
};

/// Container magic identifying a DohFlow backup.
const MAGIC: &[u8; 6] = b"PCFOBK";

/// Legacy password-derived backup format. Read-only: v1 is never emitted again.
const FORMAT_VERSION_V1: u16 = 1;

/// Current unattended-export format (ADR 0024-A).
const FORMAT_VERSION: u16 = 2;

/// Current manifest field-set schema for format v2.
const MANIFEST_SCHEMA_VERSION: u16 = 1;

/// Current serialized vault envelopes are 106 bytes. Keep a small allowance for
/// envelope-format evolution while bounding attacker-controlled header copies.
const MAX_ENVELOPE_LEN: usize = 128;

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
    /// Field-set schema. Version-1 manifests predate this field and return None.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest_schema_version: Option<u16>,
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

/// Historical v1 manifest grammar: deliberately keeps serde's legacy behavior
/// of ignoring fields added by newer writers.
#[derive(Debug, Deserialize)]
struct LegacyManifest {
    format_version: u16,
    created_at: String,
    backup_id: String,
    app_version: String,
    schema_version: i64,
    vault_envelope_version: u16,
    redaction_policy_version: u32,
    vault_db_sha256: String,
    envelope_sha256: String,
    blobs: Vec<BlobEntry>,
}

impl From<LegacyManifest> for Manifest {
    fn from(manifest: LegacyManifest) -> Self {
        Self {
            manifest_schema_version: None,
            format_version: manifest.format_version,
            created_at: manifest.created_at,
            backup_id: manifest.backup_id,
            app_version: manifest.app_version,
            schema_version: manifest.schema_version,
            vault_envelope_version: manifest.vault_envelope_version,
            redaction_policy_version: manifest.redaction_policy_version,
            vault_db_sha256: manifest.vault_db_sha256,
            envelope_sha256: manifest.envelope_sha256,
            blobs: manifest.blobs,
        }
    }
}

/// Strict v2 manifest schema. Unknown fields are a hard error, not silently
/// ignored, because they may signal a semantic change this build cannot honor.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestBlobV2 {
    storage_id: String,
    size: u64,
    ciphertext_sha256: String,
}

impl From<ManifestBlobV2> for BlobEntry {
    fn from(blob: ManifestBlobV2) -> Self {
        Self {
            storage_id: blob.storage_id,
            size: blob.size,
            ciphertext_sha256: blob.ciphertext_sha256,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestV2 {
    format_version: u16,
    manifest_schema_version: u16,
    created_at: String,
    backup_id: String,
    app_version: String,
    schema_version: i64,
    vault_envelope_version: u16,
    redaction_policy_version: u32,
    vault_db_sha256: String,
    envelope_sha256: String,
    blobs: Vec<ManifestBlobV2>,
}

impl From<ManifestV2> for Manifest {
    fn from(manifest: ManifestV2) -> Self {
        Self {
            manifest_schema_version: Some(manifest.manifest_schema_version),
            format_version: manifest.format_version,
            created_at: manifest.created_at,
            backup_id: manifest.backup_id,
            app_version: manifest.app_version,
            schema_version: manifest.schema_version,
            vault_envelope_version: manifest.vault_envelope_version,
            redaction_policy_version: manifest.redaction_policy_version,
            vault_db_sha256: manifest.vault_db_sha256,
            envelope_sha256: manifest.envelope_sha256,
            blobs: manifest.blobs.into_iter().map(BlobEntry::from).collect(),
        }
    }
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
    /// surfaces here as the vault-envelope unwrap failing for v2 or backup-key
    /// unwrap failing for v1.
    #[error("backup cryptography failed")]
    Crypto(#[from] VaultCryptoError),
    /// Manifest (JSON) serialization/deserialization failed. Parse diagnostics
    /// include an unknown field's name, as required by the v2 manifest contract.
    #[error("backup manifest is malformed: {0}")]
    Manifest(String),
    /// The package requests an Argon2 profile outside the versioned allowlist.
    #[error("backup Argon2id parameters are unsupported")]
    UnsupportedKdfParameters,
    /// The bootstrap envelope in the v2 header differs from the payload copy.
    #[error("backup header and payload vault envelopes do not match")]
    HeaderEnvelopeMismatch,
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

/// Check that a backup requests one of the explicitly supported, bounded KDF
/// profiles from ADR 0024-A before Argon2 can allocate or run.
fn validate_kdf(params: &Argon2Params) -> Result<(), BackupError> {
    let supported = [
        Profile::LegacyCompatibility.params(),
        Profile::InteractiveDefault.params(),
        Profile::HighSecurity.params(),
    ];
    if supported.contains(params) {
        Ok(())
    } else {
        Err(BackupError::UnsupportedKdfParameters)
    }
}

/// Assemble a format-v2 encrypted backup from the unlocked vault DEK.
///
/// The vault envelope is copied into both the plaintext bootstrap header and
/// the sealed payload. The payload copy remains authoritative for restore.
///
/// # Errors
/// [`BackupError`] if the envelope/KDF profile is unsupported, the manifest
/// cannot be serialized, or a crypto step fails.
pub fn assemble(dek: &vault_crypto::Dek, inputs: &BackupInputs) -> Result<Vec<u8>, BackupError> {
    if inputs.envelope_bytes.len() > MAX_ENVELOPE_LEN {
        return Err(BackupError::Malformed);
    }
    let envelope = VaultEnvelope::from_bytes(&inputs.envelope_bytes)?;
    validate_kdf(&envelope.kdf)?;
    if envelope.version != inputs.vault_envelope_version {
        return Err(BackupError::Malformed);
    }

    // 1. Manifest with ciphertext content-hashes (ADR 0024 §3).
    let manifest = ManifestV2 {
        format_version: FORMAT_VERSION,
        manifest_schema_version: MANIFEST_SCHEMA_VERSION,
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
            .map(|(storage_id, ciphertext)| ManifestBlobV2 {
                storage_id: storage_id.clone(),
                size: ciphertext.len() as u64,
                ciphertext_sha256: sha256_hex(ciphertext),
            })
            .collect(),
    };
    let manifest_json =
        serde_json::to_vec(&manifest).map_err(|error| BackupError::Manifest(error.to_string()))?;

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

    // 3. A fresh salt and backup DEK for every export. The KEK is derived from
    //    the unlocked vault DEK, never from a retained or re-entered password.
    let hkdf_salt = generate_salt()?;
    let backup_kek = derive_backup_kek(dek, hkdf_salt.as_bytes())?;
    let backup_dek = generate_dek()?;
    let wrapped = wrap_dek(&backup_kek, &backup_dek)?;
    let sealed = seal(&backup_dek, &payload)?;

    // 4. Format-v2 header: magic, version, bounded vault envelope, HKDF salt,
    //    wrapped backup DEK, then the sealed payload.
    let mut out = Vec::with_capacity(MAX_ENVELOPE_LEN + 64 + sealed.ciphertext.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&FORMAT_VERSION.to_be_bytes());
    put_bytes_u32(&mut out, &inputs.envelope_bytes);
    out.extend_from_slice(hkdf_salt.as_bytes());
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
/// [`BackupError::Crypto`] on a wrong password (the v1 backup-key unwrap or
/// v2 vault-envelope unwrap fails), or
/// [`BackupError::IntegrityMismatch`] if a component hash disagrees with the
/// manifest.
pub fn disassemble(password: &[u8], package: &[u8]) -> Result<RestoredBackup, BackupError> {
    let mut r = Reader::new(package);
    if r.take(MAGIC.len())? != MAGIC {
        return Err(BackupError::Malformed);
    }
    match r.u16()? {
        FORMAT_VERSION_V1 => disassemble_v1(password, &mut r),
        FORMAT_VERSION => disassemble_v2(password, &mut r),
        _ => Err(BackupError::Malformed),
    }
}

/// Open and verify a format-v2 backup using the already-unlocked vault DEK.
///
/// Scheduled export uses this after writing a package: it proves the output can
/// be opened by this vault and that every ciphertext component matches the
/// manifest, without prompting for or deriving from the password again.
pub(crate) fn disassemble_with_vault_dek(
    vault_dek: &vault_crypto::Dek,
    package: &[u8],
) -> Result<RestoredBackup, BackupError> {
    let mut r = Reader::new(package);
    if r.take(MAGIC.len())? != MAGIC || r.u16()? != FORMAT_VERSION {
        return Err(BackupError::Malformed);
    }

    let header_envelope_bytes = r.bytes_u32_bounded(MAX_ENVELOPE_LEN)?;
    let header_envelope =
        VaultEnvelope::from_bytes(&header_envelope_bytes).map_err(|_| BackupError::Malformed)?;
    validate_kdf(&header_envelope.kdf)?;

    let hkdf_salt = r.take_arr::<SALT_LEN>()?;
    let wrapped = vault_crypto::WrappedDek {
        nonce: r.take_arr::<NONCE_LEN>()?,
        ciphertext: r.bytes_u32()?,
    };
    let sealed = vault_crypto::backup::SealedPayload {
        nonce: r.take_arr::<NONCE_LEN>()?,
        ciphertext: r.bytes_u64()?,
    };

    let backup_kek = derive_backup_kek(vault_dek, &hkdf_salt)?;
    let backup_dek = unwrap_dek(&backup_kek, &wrapped)?;
    let payload = open(&backup_dek, &sealed)?;
    restore_payload(&payload, FORMAT_VERSION, Some(&header_envelope_bytes))
}

fn disassemble_v1(password: &[u8], r: &mut Reader<'_>) -> Result<RestoredBackup, BackupError> {
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
    validate_kdf(&params)?;
    let salt = Salt::from_bytes(r.take_arr::<SALT_LEN>()?);
    let wrapped = vault_crypto::WrappedDek {
        nonce: r.take_arr::<NONCE_LEN>()?,
        ciphertext: r.bytes_u32()?,
    };
    let sealed = vault_crypto::backup::SealedPayload {
        nonce: r.take_arr::<NONCE_LEN>()?,
        ciphertext: r.bytes_u64()?,
    };

    // Historical v1 used a password-derived backup KEK. Keep that key chain
    // byte-compatible while applying the bounded KDF allowlist first.
    let kek = derive_kek(password, &salt, &params)?;
    let backup_dek = unwrap_dek(&kek, &wrapped)?;
    let payload = open(&backup_dek, &sealed)?;
    restore_payload(&payload, FORMAT_VERSION_V1, None)
}

fn disassemble_v2(password: &[u8], r: &mut Reader<'_>) -> Result<RestoredBackup, BackupError> {
    let header_envelope_bytes = r.bytes_u32_bounded(MAX_ENVELOPE_LEN)?;
    let header_envelope =
        VaultEnvelope::from_bytes(&header_envelope_bytes).map_err(|_| BackupError::Malformed)?;
    validate_kdf(&header_envelope.kdf)?;

    let hkdf_salt = r.take_arr::<SALT_LEN>()?;
    let wrapped = vault_crypto::WrappedDek {
        nonce: r.take_arr::<NONCE_LEN>()?,
        ciphertext: r.bytes_u32()?,
    };
    let sealed = vault_crypto::backup::SealedPayload {
        nonce: r.take_arr::<NONCE_LEN>()?,
        ciphertext: r.bytes_u64()?,
    };

    // The password unlocks the envelope carried by the header; the recovered
    // vault DEK derives a distinct per-backup KEK for the wrapped backup DEK.
    let vault_kek = derive_kek(password, &header_envelope.salt, &header_envelope.kdf)?;
    let vault_dek = unwrap_dek(&vault_kek, &header_envelope.wrapped)?;
    let backup_kek = derive_backup_kek(&vault_dek, &hkdf_salt)?;
    let backup_dek = unwrap_dek(&backup_kek, &wrapped)?;
    let payload = open(&backup_dek, &sealed)?;

    // The payload envelope is authoritative. Compare it to the bootstrap copy
    // before parsing/installing any restored files.
    restore_payload(&payload, FORMAT_VERSION, Some(&header_envelope_bytes))
}

fn restore_payload(
    payload: &[u8],
    format_version: u16,
    header_envelope: Option<&[u8]>,
) -> Result<RestoredBackup, BackupError> {
    let mut p = Reader::new(payload);
    // Borrow the manifest and envelope slices first; do not deserialize/copy
    // payload contents until the v2 bootstrap envelope has been authenticated
    // against the payload copy.
    let manifest_json = p.bytes_u64_slice()?;
    let envelope_slice = p.bytes_u64_bounded_slice(MAX_ENVELOPE_LEN)?;
    if let Some(header_envelope) = header_envelope {
        if header_envelope.len() != envelope_slice.len()
            || !bool::from(header_envelope.ct_eq(envelope_slice))
        {
            return Err(BackupError::HeaderEnvelopeMismatch);
        }
    }

    let envelope_bytes = envelope_slice.to_vec();
    let manifest = parse_manifest(format_version, manifest_json)?;
    // V1's password-derived header KDF is distinct from the vault-envelope KDF.
    // Validate the embedded envelope too, before the restore installer opens it.
    let envelope =
        VaultEnvelope::from_bytes(&envelope_bytes).map_err(|_| BackupError::Malformed)?;
    validate_kdf(&envelope.kdf)?;

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

fn parse_manifest(format_version: u16, manifest_json: &[u8]) -> Result<Manifest, BackupError> {
    let manifest = if format_version == FORMAT_VERSION {
        let manifest: ManifestV2 = serde_json::from_slice(manifest_json)
            .map_err(|error| BackupError::Manifest(error.to_string()))?;
        if manifest.manifest_schema_version != MANIFEST_SCHEMA_VERSION {
            return Err(BackupError::Manifest(format!(
                "unsupported manifest_schema_version {}",
                manifest.manifest_schema_version
            )));
        }
        Manifest::from(manifest)
    } else {
        let manifest: LegacyManifest = serde_json::from_slice(manifest_json)
            .map_err(|error| BackupError::Manifest(error.to_string()))?;
        Manifest::from(manifest)
    };
    if manifest.format_version != format_version {
        return Err(BackupError::Manifest(
            "manifest format_version does not match the container".into(),
        ));
    }
    Ok(manifest)
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
    fn u16(&mut self) -> Result<u16, BackupError> {
        Ok(u16::from_be_bytes(self.take_arr::<2>()?))
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
    fn bytes_u32_bounded(&mut self, max: usize) -> Result<Vec<u8>, BackupError> {
        let n = self.u32()? as usize;
        if n > max {
            return Err(BackupError::Malformed);
        }
        Ok(self.take(n)?.to_vec())
    }
    fn bytes_u64(&mut self) -> Result<Vec<u8>, BackupError> {
        let n = usize::try_from(self.u64()?).map_err(|_| BackupError::Malformed)?;
        Ok(self.take(n)?.to_vec())
    }
    fn bytes_u64_slice(&mut self) -> Result<&'a [u8], BackupError> {
        let n = usize::try_from(self.u64()?).map_err(|_| BackupError::Malformed)?;
        self.take(n)
    }
    fn bytes_u64_bounded_slice(&mut self, max: usize) -> Result<&'a [u8], BackupError> {
        let n = usize::try_from(self.u64()?).map_err(|_| BackupError::Malformed)?;
        if n > max {
            return Err(BackupError::Malformed);
        }
        self.take(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vault_crypto::{rewrap_envelope, wrap_dek, Profile};

    const PASSWORD: &[u8] = b"correct horse";

    fn test_vault() -> (BackupInputs, vault_crypto::Dek) {
        let params = Profile::LegacyCompatibility.params();
        let vault_salt = generate_salt().unwrap();
        let vault_kek = derive_kek(PASSWORD, &vault_salt, &params).unwrap();
        let vault_dek = generate_dek().unwrap();
        let wrapped_vault_dek = wrap_dek(&vault_kek, &vault_dek).unwrap();
        let envelope_bytes = VaultEnvelope::new(params, vault_salt, wrapped_vault_dek).to_bytes();
        let inputs = BackupInputs {
            db_bytes: b"SQLCipher ciphertext bytes".to_vec(),
            envelope_bytes,
            blobs: vec![
                ("a3f9".into(), b"blob-one-ciphertext".to_vec()),
                ("7b20".into(), b"blob-two-ciphertext".to_vec()),
            ],
            app_version: "0.1.0".into(),
            schema_version: 5,
            vault_envelope_version: 1,
            created_at: "2026-06-20T00:00:00Z".into(),
            backup_id: Uuid::from_bytes([7u8; 16]),
        };
        (inputs, vault_dek)
    }

    #[test]
    fn assemble_then_disassemble_round_trips() {
        let (inputs, dek) = test_vault();
        let pkg = assemble(&dek, &inputs).unwrap();
        let restored = disassemble(PASSWORD, &pkg).unwrap();
        assert_eq!(restored.db_bytes, inputs.db_bytes);
        assert_eq!(restored.envelope_bytes, inputs.envelope_bytes);
        assert_eq!(restored.blobs, inputs.blobs);
        assert_eq!(restored.manifest.manifest_schema_version, Some(1));
        assert_eq!(restored.manifest.schema_version, 5);
        assert_eq!(restored.manifest.backup_id, inputs.backup_id.to_string());
        assert_eq!(restored.manifest.blobs.len(), 2);

        let verified = disassemble_with_vault_dek(&dek, &pkg).unwrap();
        assert_eq!(verified.manifest.backup_id, inputs.backup_id.to_string());
        assert_eq!(verified.db_bytes, inputs.db_bytes);

        let wrong_dek = generate_dek().unwrap();
        assert!(matches!(
            disassemble_with_vault_dek(&wrong_dek, &pkg),
            Err(BackupError::Crypto(_))
        ));

        assert_eq!(&pkg[..MAGIC.len()], MAGIC);
        assert_eq!(u16::from_be_bytes([pkg[6], pkg[7]]), FORMAT_VERSION);
        let envelope_len = u32::from_be_bytes(pkg[8..12].try_into().unwrap()) as usize;
        assert_eq!(&pkg[12..12 + envelope_len], inputs.envelope_bytes);
        let hkdf_salt_offset = 12 + envelope_len;
        let wrapped_nonce_offset = hkdf_salt_offset + SALT_LEN;
        let wrapped_len_offset = wrapped_nonce_offset + NONCE_LEN;
        let wrapped_len = u32::from_be_bytes(
            pkg[wrapped_len_offset..wrapped_len_offset + 4]
                .try_into()
                .unwrap(),
        ) as usize;
        assert_eq!(wrapped_len, 32 + 16, "wrapped backup DEK is AES-GCM sealed");
        let sealed_nonce_offset = wrapped_len_offset + 4 + wrapped_len;
        let sealed_len_offset = sealed_nonce_offset + NONCE_LEN;
        let sealed_len = u64::from_be_bytes(
            pkg[sealed_len_offset..sealed_len_offset + 8]
                .try_into()
                .unwrap(),
        ) as usize;
        assert_eq!(sealed_len_offset + 8 + sealed_len, pkg.len());
    }

    #[test]
    fn wrong_password_fails_to_open() {
        let (inputs, dek) = test_vault();
        let pkg = assemble(&dek, &inputs).unwrap();
        assert!(matches!(
            disassemble(b"wrong password", &pkg),
            Err(BackupError::Crypto(_))
        ));
    }

    #[test]
    fn no_plaintext_vault_bytes_in_the_package() {
        // The wrapped vault envelope is intentionally visible in the v2 header;
        // financial data and bundled ciphertext remain sealed.
        let (inputs, dek) = test_vault();
        let pkg = assemble(&dek, &inputs).unwrap();
        for needle in [
            b"SQLCipher ciphertext bytes".as_slice(),
            b"blob-one-ciphertext".as_slice(),
        ] {
            assert!(
                !pkg.windows(needle.len()).any(|w| w == needle),
                "bundled bytes leaked into the package in the clear"
            );
        }
    }

    #[test]
    fn tampered_payload_is_rejected() {
        let (inputs, dek) = test_vault();
        let mut pkg = assemble(&dek, &inputs).unwrap();
        let last = pkg.len() - 1;
        pkg[last] ^= 0xff;
        assert!(disassemble(PASSWORD, &pkg).is_err());
    }

    #[test]
    fn truncated_package_is_malformed() {
        let (inputs, dek) = test_vault();
        let pkg = assemble(&dek, &inputs).unwrap();
        assert!(matches!(
            disassemble(PASSWORD, &pkg[..pkg.len() / 2]),
            Err(BackupError::Malformed | BackupError::Crypto(_))
        ));
    }

    #[test]
    fn tampered_v2_header_fields_fail_closed() {
        let (inputs, dek) = test_vault();
        let package = assemble(&dek, &inputs).unwrap();
        let envelope_len = u32::from_be_bytes(package[8..12].try_into().unwrap()) as usize;
        let envelope_start = 12;
        let hkdf_salt_start = envelope_start + envelope_len;
        let wrapped_ciphertext_start = hkdf_salt_start + SALT_LEN + NONCE_LEN + 4;

        // Mutating the wrapped vault DEK leaves a structurally valid envelope,
        // but the user's password can no longer authenticate it.
        let mut envelope_tampered = package.clone();
        envelope_tampered[envelope_start + 26] ^= 1; // vault-envelope salt
        assert!(matches!(
            disassemble(PASSWORD, &envelope_tampered),
            Err(BackupError::Crypto(_))
        ));

        let mut salt_tampered = package.clone();
        salt_tampered[hkdf_salt_start] ^= 1;
        assert!(matches!(
            disassemble(PASSWORD, &salt_tampered),
            Err(BackupError::Crypto(_))
        ));

        let mut wrapped_key_tampered = package;
        wrapped_key_tampered[wrapped_ciphertext_start] ^= 1;
        assert!(matches!(
            disassemble(PASSWORD, &wrapped_key_tampered),
            Err(BackupError::Crypto(_))
        ));
    }

    #[test]
    fn valid_but_different_header_envelope_is_rejected_after_authentication() {
        let (inputs, dek) = test_vault();
        let mut package = assemble(&dek, &inputs).unwrap();
        let envelope_len = u32::from_be_bytes(package[8..12].try_into().unwrap()) as usize;
        let original = VaultEnvelope::from_bytes(&inputs.envelope_bytes).unwrap();
        let rewrapped = rewrap_envelope(
            &original,
            PASSWORD,
            PASSWORD,
            Profile::LegacyCompatibility.params(),
        )
        .unwrap()
        .to_bytes();
        assert_eq!(rewrapped.len(), envelope_len);
        package[12..12 + envelope_len].copy_from_slice(&rewrapped);

        assert!(matches!(
            disassemble(PASSWORD, &package),
            Err(BackupError::HeaderEnvelopeMismatch)
        ));
    }

    #[test]
    fn header_kdf_outside_the_allowlist_is_rejected_before_derivation() {
        let (inputs, dek) = test_vault();
        let mut package = assemble(&dek, &inputs).unwrap();
        let envelope_len = u32::from_be_bytes(package[8..12].try_into().unwrap()) as usize;
        let memory_offset = 12 + 10;
        package[memory_offset..memory_offset + 4].copy_from_slice(&19_455u32.to_be_bytes());
        assert!(matches!(
            disassemble(PASSWORD, &package),
            Err(BackupError::UnsupportedKdfParameters)
        ));
        assert!(envelope_len <= MAX_ENVELOPE_LEN);
    }

    #[test]
    fn header_envelope_length_is_bounded_before_copy() {
        let (inputs, dek) = test_vault();
        let mut package = assemble(&dek, &inputs).unwrap();
        package[8..12].copy_from_slice(&((MAX_ENVELOPE_LEN as u32) + 1).to_be_bytes());
        assert!(matches!(
            disassemble(PASSWORD, &package),
            Err(BackupError::Malformed)
        ));
    }

    #[test]
    fn v2_manifest_rejects_unknown_fields_and_names_them() {
        let json = br#"{
            "format_version":2,
            "manifest_schema_version":1,
            "created_at":"2026-06-20T00:00:00Z",
            "backup_id":"id",
            "app_version":"test",
            "schema_version":1,
            "vault_envelope_version":1,
            "redaction_policy_version":1,
            "vault_db_sha256":"hash",
            "envelope_sha256":"hash",
            "blobs":[],
            "unexpected_field":true
        }"#;
        assert!(matches!(
            parse_manifest(FORMAT_VERSION, json),
            Err(BackupError::Manifest(message)) if message.contains("unexpected_field")
        ));
    }

    #[test]
    fn v2_manifest_rejects_unknown_blob_fields() {
        let json = br#"{
            "format_version":2,
            "manifest_schema_version":1,
            "created_at":"2026-06-20T00:00:00Z",
            "backup_id":"id",
            "app_version":"test",
            "schema_version":1,
            "vault_envelope_version":1,
            "redaction_policy_version":1,
            "vault_db_sha256":"hash",
            "envelope_sha256":"hash",
            "blobs":[{"storage_id":"sid","size":1,"ciphertext_sha256":"hash","extra_blob_field":true}]
        }"#;
        assert!(matches!(
            parse_manifest(FORMAT_VERSION, json),
            Err(BackupError::Manifest(message)) if message.contains("extra_blob_field")
        ));
    }

    #[test]
    fn v2_manifest_requires_manifest_schema_version() {
        let json = br#"{
            "format_version":2,
            "created_at":"2026-06-20T00:00:00Z",
            "backup_id":"id",
            "app_version":"test",
            "schema_version":1,
            "vault_envelope_version":1,
            "redaction_policy_version":1,
            "vault_db_sha256":"hash",
            "envelope_sha256":"hash",
            "blobs":[]
        }"#;
        assert!(matches!(
            parse_manifest(FORMAT_VERSION, json),
            Err(BackupError::Manifest(message)) if message.contains("manifest_schema_version")
        ));
    }
}
