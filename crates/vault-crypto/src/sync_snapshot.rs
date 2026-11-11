//! Sealing and opening a Sync snapshot (`docs/architecture/vault-format.md` C.3, C.4 and C.8;
//! `personal-cfo-1cltt.3`).
//!
//! A snapshot is an AEAD object of type `2`: the same header, container and key derivation as an
//! envelope, with object type `2` in the key derivation and the nonce domain (so its key is never an
//! envelope's key), header fields 7 and 8 absent, and field 17 **always** present. This module is
//! the snapshot's counterpart of `sync_envelope`'s `seal_envelope` and `ReceiverOpener`, and it
//! shares their AEAD and nonce construction.
//!
//! * [`seal_snapshot`] takes a [`CounterReservation`] (the register has durably advanced past it),
//!   refuses any identity that is not the sealer's own, **before** it encrypts, and refuses a
//!   plaintext above `2^30` bytes with `SnapshotTooLarge`.
//! * [`SnapshotOpener`] is the receiver path: the key comes from the **expected** vault and epoch
//!   and the fingerprint the membership chain holds for the header's device, never from the
//!   header's claims. It refuses an envelope, as the envelope opener refuses a snapshot. Like
//!   `ReceiverOpener` it is the only way to open a snapshot outside tests, and a source-scan test
//!   guards that.
//! * [`SnapshotSealer::seal`] is the publish path: it keeps the saved ciphertext (W4) so an
//!   exact-byte retransmission of the same publish reuses the same bytes and burns nothing, and it
//!   runs seal-then-verify through the receiver path before it hands anything back.
//!
//! # Exact-byte retransmission and the changed retry
//!
//! A transport retry of one publish (the same snapshot id, `base_seq` and plaintext) re-sends the
//! saved container: a second encryption with a new counter would be a different object for the
//! same publish. Anything else, including a retry after a failed compare-and-swap, is a new
//! plaintext at a new head and **burns a counter**. The saved container is a file in the vault's
//! sync directory (it can be up to 1 GiB, so never a database row), and the register store keeps
//! its path-free record ([`SavedSealed`]: ids, the plaintext's SHA-256, the container's SHA-256 and
//! length) beside the context's counter. A file that is missing or whose hash no longer matches is
//! simply not reusable: the publish reseals with a fresh counter.
//!
//! # Memory
//!
//! The plaintext, the ciphertext and the verified plaintext are held in memory together (up to
//! three copies of 1 GiB at the cap). A chunked framing, which C.8 defers, is what removes that.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use platform_traits::{CounterContext, RollbackWitness};
use sha2::{Digest, Sha256};
use sync_core::artifact::{ArtifactId, MAX_SNAPSHOT_CLOSURE};
use sync_core::envelope::{binding_commitment, ArtifactManifest, BoundArtifact};
use sync_core::header::{
    encode_container, parse_container, BindingCommitment, ClearHeader, Fingerprint,
    ManifestCommitment, NonceDomain, ObjectKind, ParsedContainer, OBJECT_SNAPSHOT,
};
use sync_core::membership::VerifiedState;
use sync_core::snapshot::verify as verify_plaintext;
use thiserror::Error;

use crate::sync_counter::{
    CounterRegister, CounterReservation, LostReason, NonceError, RegisterStore, RegisterStoreError,
    SavedSealed,
};
use crate::sync_envelope::{
    aead_key, gcm_decrypt, gcm_encrypt, EpochKey, ObjectType, SealerIdentity,
};

/// The most a snapshot's plaintext can be (C.8): 1 GiB. A larger vault fails with
/// [`SnapshotSealError::SnapshotTooLarge`] until a chunked framing is specified.
pub const MAX_SNAPSHOT_PLAINTEXT: usize = 1 << 30;

/// The header fields of a snapshot that the caller chooses. Everything else in the header is the
/// sealer's identity, the reserved counter and the closure commitment (field 17), which is
/// **derived from the plaintext's own manifest** and never supplied, so the two cannot disagree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotFields {
    /// The snapshot id (header `object_id`).
    pub object_id: [u8; 16],
    /// The server sequence number the snapshot is built on.
    pub base_seq: u64,
    /// The payload schema version.
    pub payload_schema_version: u32,
    /// The apply-semantics generation (C.13).
    pub engine_version: String,
    /// The producer's vault schema version.
    pub schema_version: u32,
}

/// Why a snapshot was not sealed. Nothing was encrypted in any of these cases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum SnapshotSealError {
    /// The key belongs to another vault than the sealer.
    #[error("the key and the sealer are for different vaults")]
    WrongVault,
    /// The reservation is for another vault, epoch, fingerprint or object type than this seal.
    #[error("the counter reservation is for another context")]
    ReservationMismatch,
    /// The plaintext is above 2^30 bytes (C.11 `SnapshotTooLarge`).
    #[error("the snapshot plaintext is too large")]
    SnapshotTooLarge,
    /// The plaintext is not a well-formed snapshot (C.8), or its manifest names a closure that
    /// cannot be committed to. Nothing is sealed: a snapshot no replica could verify is not
    /// published.
    #[error("the plaintext is not a valid snapshot")]
    PlaintextInvalid,
    /// The header cannot be encoded within its bounds.
    #[error("the snapshot cannot be encoded")]
    Unencodable,
    /// The AEAD refused to encrypt.
    #[error("encryption failed")]
    Encryption,
}

fn context_of(key: &EpochKey, fingerprint: &Fingerprint) -> CounterContext {
    CounterContext {
        vault_id: *key.vault_id(),
        key_epoch: key.key_epoch(),
        fingerprint: *fingerprint.as_bytes(),
        object_type: OBJECT_SNAPSHOT,
    }
}

/// The commitment to the closure a snapshot plaintext names (C.3 "Artifact manifest commitment"),
/// the value header field 17 must carry: see [`closure_commitments_of`] for field 18 too.
///
/// # Errors
/// As [`closure_commitments_of`].
pub fn closure_commitment_of(plaintext: &[u8]) -> Result<ManifestCommitment, SnapshotSealError> {
    Ok(closure_commitments_of(plaintext)?.manifest)
}

/// The two commitments a snapshot's closure fixes in the header (C.3, C.4): field 17, over the ids,
/// and field 18, over the ciphertext hashes bound to them in the ids' order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClosureCommitments {
    /// Header field 17.
    pub manifest: ManifestCommitment,
    /// Header field 18.
    pub binding: BindingCommitment,
}

fn hex32(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0_u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(hex.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

fn closure_bound_of(plaintext: &[u8]) -> Result<Vec<BoundArtifact>, SnapshotSealError> {
    let verified = verify_plaintext(plaintext).map_err(|_| SnapshotSealError::PlaintextInvalid)?;
    verified
        .manifest
        .artifacts
        .iter()
        .map(|entry| {
            Some(BoundArtifact {
                id: hex32(&entry.id)?,
                ciphertext_sha256: hex32(&entry.ciphertext_sha256)?,
            })
        })
        .collect::<Option<Vec<_>>>()
        .ok_or(SnapshotSealError::PlaintextInvalid)
}

/// Both commitments of the closure a snapshot plaintext names: the plaintext is verified as a whole
/// (C.8 framing, manifest and file hashes), the manifest's entries are decoded, and each commitment
/// is taken over its list in the ids' strictly ascending order. They are the values header fields 17
/// and 18 must carry, the commitments of the empty closure included.
///
/// # Errors
/// [`SnapshotSealError::PlaintextInvalid`] if the plaintext does not verify or its closure is not a
/// strictly ascending list of at most 4096 ids.
pub fn closure_commitments_of(plaintext: &[u8]) -> Result<ClosureCommitments, SnapshotSealError> {
    let bound = closure_bound_of(plaintext)?;
    // `verify` already requires strictly ascending ids; the manifest type orders and refuses a
    // duplicate or an oversized closure again, so the commitments are over lists that are.
    let ids: Vec<[u8; 32]> = bound.iter().map(|entry| entry.id).collect();
    let hashes: Vec<[u8; 32]> = bound.iter().map(|entry| entry.ciphertext_sha256).collect();
    Ok(ClosureCommitments {
        manifest: ArtifactManifest::new(ids)
            .map_err(|_| SnapshotSealError::PlaintextInvalid)?
            .commitment(),
        binding: binding_commitment(&hashes, MAX_SNAPSHOT_CLOSURE)
            .map_err(|_| SnapshotSealError::PlaintextInvalid)?,
    })
}

/// The artifact ids a snapshot plaintext's manifest names, ascending: the closure sidecar that
/// travels with `publishSnapshot` and `installEpoch`. Empty until `personal-cfo-1cltt.7` puts class-1b
/// artifacts into a genesis.
///
/// # Errors
/// [`SnapshotSealError::PlaintextInvalid`], as [`closure_commitments_of`].
pub fn closure_ids_of(plaintext: &[u8]) -> Result<Vec<ArtifactId>, SnapshotSealError> {
    Ok(closure_bound_of(plaintext)?
        .into_iter()
        .map(|entry| ArtifactId::from_derived(entry.id))
        .collect())
}

/// Seal `plaintext` as the snapshot `fields` describe, with `key` as the epoch's key and `sealer`
/// as this device. The counter is the reservation's, consumed here. Header field 17 is the
/// commitment of the closure the plaintext's manifest names.
///
/// # Errors
/// A [`SnapshotSealError`]; nothing is encrypted unless every check passes.
pub fn seal_snapshot(
    key: &EpochKey,
    sealer: &SealerIdentity,
    fields: &SnapshotFields,
    reservation: CounterReservation,
    plaintext: &[u8],
) -> Result<Vec<u8>, SnapshotSealError> {
    if plaintext.len() > MAX_SNAPSHOT_PLAINTEXT {
        return Err(SnapshotSealError::SnapshotTooLarge);
    }
    let commitments = closure_commitments_of(plaintext)?;
    seal_with_commitment(key, sealer, fields, commitments, reservation, plaintext)
}

fn seal_with_commitment(
    key: &EpochKey,
    sealer: &SealerIdentity,
    fields: &SnapshotFields,
    commitments: ClosureCommitments,
    reservation: CounterReservation,
    plaintext: &[u8],
) -> Result<Vec<u8>, SnapshotSealError> {
    if key.vault_id() != &sealer.vault_id {
        return Err(SnapshotSealError::WrongVault);
    }
    if reservation.context() != &context_of(key, &sealer.fingerprint) {
        return Err(SnapshotSealError::ReservationMismatch);
    }
    if plaintext.len() > MAX_SNAPSHOT_PLAINTEXT {
        return Err(SnapshotSealError::SnapshotTooLarge);
    }
    let domain = NonceDomain::derive(
        key.vault_id(),
        key.key_epoch(),
        &sealer.fingerprint,
        OBJECT_SNAPSHOT,
    );
    let header = ClearHeader {
        vault_id: sealer.vault_id,
        key_epoch: key.key_epoch(),
        object: ObjectKind::Snapshot {
            artifact_manifest_commitment: commitments.manifest,
            artifact_binding_commitment: commitments.binding,
        },
        object_id: fields.object_id,
        device_id: sealer.device_id,
        device_key_fingerprint: sealer.fingerprint,
        nonce_domain: domain,
        invocation_counter: reservation.counter(),
        base_seq: fields.base_seq,
        payload_schema_version: fields.payload_schema_version,
        engine_version: fields.engine_version.clone(),
        schema_version: fields.schema_version,
    };
    let aad = header
        .encode()
        .map_err(|_| SnapshotSealError::Unencodable)?;
    let aead = aead_key(key, &sealer.fingerprint, ObjectType::Snapshot);
    let ciphertext = gcm_encrypt(&aead, &domain, reservation.counter(), &aad, plaintext)
        .map_err(|()| SnapshotSealError::Encryption)?;
    encode_container(&header, &ciphertext).map_err(|_| SnapshotSealError::Unencodable)
}

/// Why the receiver path refused to open a snapshot. Each is deterministic from the bytes, the
/// verified key and the verified chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum SnapshotOpenFailure {
    /// The header is not a snapshot header (an envelope fails closed here).
    #[error("not a snapshot header")]
    NotASnapshot,
    /// The header names another vault than the key's.
    #[error("another vault")]
    WrongVault,
    /// The header names another key epoch than the key's.
    #[error("another key epoch")]
    WrongEpoch,
    /// The header's device is not a member of the epoch.
    #[error("the device is not a member of the epoch")]
    DeviceOutsideEpoch,
    /// The header's fingerprint is not the one the chain holds for its device.
    #[error("the fingerprint is not the chain's")]
    IdentityMismatch,
    /// The header's nonce domain is not the derived one.
    #[error("the nonce domain is not the derived one")]
    NonceDomain,
    /// The ciphertext does not open under the derived key and the header as AAD.
    #[error("the ciphertext does not open")]
    NotOpenable,
    /// The opened plaintext is not a well-formed snapshot (C.8).
    #[error("the opened plaintext is not a valid snapshot")]
    PlaintextInvalid,
    /// Header field 17 is not the commitment of the closure the plaintext's manifest names.
    #[error("the header's closure commitment is not the manifest's")]
    ClosureCommitmentMismatch,
    /// Header field 18 is not the binding commitment of the hashes the plaintext's manifest binds.
    /// A snapshot like this is deterministically invalid at every opener, never "not
    /// determinable" (C.8).
    #[error("the header's binding commitment is not the manifest's")]
    BindingCommitmentMismatch,
}

/// The receiver path for snapshots: the key is derived from the expected vault and epoch and the
/// fingerprint the chain holds for the header's device as a member of that epoch, never from the
/// header's claims; the header must restate exactly those; the AAD is the received header bytes.
pub struct SnapshotOpener<'a> {
    key: &'a EpochKey,
    chain: &'a VerifiedState,
}

impl<'a> SnapshotOpener<'a> {
    /// An opener for snapshots of `key`'s epoch, judged against `chain`.
    #[must_use]
    pub const fn new(key: &'a EpochKey, chain: &'a VerifiedState) -> Self {
        Self { key, chain }
    }

    /// Open `container`: authenticated decryption, then the plaintext verified as a snapshot, then
    /// header field 17 compared with the closure the manifest names.
    ///
    /// # Errors
    /// A [`SnapshotOpenFailure`].
    pub fn open(&self, container: &ParsedContainer<'_>) -> Result<Vec<u8>, SnapshotOpenFailure> {
        let header = &container.header;
        if header.object_type() != OBJECT_SNAPSHOT {
            return Err(SnapshotOpenFailure::NotASnapshot);
        }
        if header.vault_id != *self.key.vault_id() {
            return Err(SnapshotOpenFailure::WrongVault);
        }
        if header.key_epoch != self.key.key_epoch() {
            return Err(SnapshotOpenFailure::WrongEpoch);
        }
        let member = self
            .chain
            .authorized_member_in_epoch(&header.device_id, self.key.key_epoch())
            .ok_or(SnapshotOpenFailure::DeviceOutsideEpoch)?;
        if member.fingerprint != header.device_key_fingerprint {
            return Err(SnapshotOpenFailure::IdentityMismatch);
        }
        let domain = NonceDomain::derive(
            self.key.vault_id(),
            self.key.key_epoch(),
            &member.fingerprint,
            OBJECT_SNAPSHOT,
        );
        if header.nonce_domain != domain {
            return Err(SnapshotOpenFailure::NonceDomain);
        }
        let aead = aead_key(self.key, &member.fingerprint, ObjectType::Snapshot);
        let plaintext = gcm_decrypt(
            &aead,
            &domain,
            header.invocation_counter,
            container.header_bytes,
            container.ciphertext,
        )
        .map_err(|()| SnapshotOpenFailure::NotOpenable)?;
        // After authenticated decryption, the receiver checks the commitment of header field 17
        // against the plaintext's own manifest (C.4 receiver check 4, for a snapshot).
        let commitments = closure_commitments_of(&plaintext)
            .map_err(|_| SnapshotOpenFailure::PlaintextInvalid)?;
        match header.object {
            ObjectKind::Snapshot {
                artifact_manifest_commitment,
                artifact_binding_commitment,
            } => {
                if artifact_manifest_commitment != commitments.manifest {
                    Err(SnapshotOpenFailure::ClosureCommitmentMismatch)
                } else if artifact_binding_commitment != commitments.binding {
                    Err(SnapshotOpenFailure::BindingCommitmentMismatch)
                } else {
                    Ok(plaintext)
                }
            }
            ObjectKind::Envelope { .. } => Err(SnapshotOpenFailure::ClosureCommitmentMismatch),
        }
    }
}

impl core::fmt::Debug for SnapshotOpener<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SnapshotOpener")
            .field("key_epoch", &self.key.key_epoch())
            .finish_non_exhaustive()
    }
}

/// A file operation on the saved snapshot failed. Carries no path and no message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("the saved snapshot file could not be read or written")]
pub struct FilesError;

/// The saved sealed snapshots, one file per snapshot id in the vault's sync directory. The
/// directory belongs to the caller (it sits beside the vault, never in a database).
#[derive(Debug, Clone)]
pub struct SavedSnapshotFiles {
    dir: PathBuf,
}

fn file_name(object_id: &[u8; 16]) -> String {
    use core::fmt::Write as _;
    let mut name = String::with_capacity(41);
    for byte in object_id {
        write!(name, "{byte:02x}").expect("writing to a String cannot fail");
    }
    name.push_str(".snapshot");
    name
}

impl SavedSnapshotFiles {
    /// Files under `dir`, which must exist.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn path(&self, object_id: &[u8; 16]) -> PathBuf {
        self.dir.join(file_name(object_id))
    }

    /// Write the container durably: to a temporary name, flushed to disk, then renamed over the
    /// final one and the directory flushed, so a crash leaves the old file or the new one whole.
    ///
    /// # Errors
    /// [`FilesError`].
    pub fn write(&self, object_id: &[u8; 16], container: &[u8]) -> Result<(), FilesError> {
        let target = self.path(object_id);
        let temp = self.dir.join(format!(".{}.tmp", file_name(object_id)));
        let mut file = fs::File::create(&temp).map_err(|_| FilesError)?;
        file.write_all(container).map_err(|_| FilesError)?;
        file.sync_all().map_err(|_| FilesError)?;
        drop(file);
        fs::rename(&temp, &target).map_err(|_| FilesError)?;
        sync_dir(&self.dir)
    }

    /// The container, or `None` if there is no file.
    ///
    /// # Errors
    /// [`FilesError`] for anything but a missing file.
    pub fn read(&self, object_id: &[u8; 16]) -> Result<Option<Vec<u8>>, FilesError> {
        match fs::read(self.path(object_id)) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(FilesError),
        }
    }

    /// Remove the file. A file that is already gone is not an error.
    ///
    /// # Errors
    /// [`FilesError`] for anything but a missing file.
    pub fn remove(&self, object_id: &[u8; 16]) -> Result<(), FilesError> {
        match fs::remove_file(self.path(object_id)) {
            Ok(()) => sync_dir(&self.dir),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(FilesError),
        }
    }
}

fn sync_dir(dir: &Path) -> Result<(), FilesError> {
    fs::File::open(dir)
        .and_then(|handle| handle.sync_all())
        .map_err(|_| FilesError)
}

/// Why a snapshot could not be prepared for publication.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum SnapshotError {
    /// No counter was reserved (including `NonceStateLost` and `RotationRequired`).
    #[error("{0}")]
    Nonce(NonceError),
    /// The snapshot was not sealed.
    #[error("{0}")]
    Seal(SnapshotSealError),
    /// The sealed bytes do not open through the receiver path: a producer bug. Nothing is
    /// published, and the counter is burned.
    #[error("the sealed snapshot does not open through the receiver path: {0}")]
    NotOpenable(SnapshotOpenFailure),
    /// The sealed bytes open, but not to the plaintext that was sealed: a producer bug.
    #[error("the sealed snapshot does not open to what was sealed")]
    Inconsistent,
    /// The register store failed.
    #[error("{0}")]
    Register(RegisterStoreError),
    /// The saved container file failed.
    #[error("{0}")]
    Files(FilesError),
}

impl From<NonceError> for SnapshotError {
    fn from(error: NonceError) -> Self {
        Self::Nonce(error)
    }
}
impl From<SnapshotSealError> for SnapshotError {
    fn from(error: SnapshotSealError) -> Self {
        Self::Seal(error)
    }
}
impl From<RegisterStoreError> for SnapshotError {
    fn from(error: RegisterStoreError) -> Self {
        Self::Register(error)
    }
}
impl From<FilesError> for SnapshotError {
    fn from(error: FilesError) -> Self {
        Self::Files(error)
    }
}

/// A sealed, verified snapshot container ready for publication.
#[derive(Clone, PartialEq, Eq)]
pub struct SealedSnapshot {
    /// The container bytes: the only bytes to publish.
    pub container: Vec<u8>,
    /// The counter the container was sealed with.
    pub counter: u64,
    /// `true` when this is the saved container of the same publish, sent again; no counter was
    /// reserved for it.
    pub reused: bool,
}

impl core::fmt::Debug for SealedSnapshot {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // Up to 1 GiB of ciphertext: the size, not the bytes.
        f.debug_struct("SealedSnapshot")
            .field("len", &self.container.len())
            .field("counter", &self.counter)
            .field("reused", &self.reused)
            .finish()
    }
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// The publish path of a snapshot: counter reservation, sealing, seal-then-verify, and the saved
/// container. See the module comment.
pub struct SnapshotSealer<'a, S: RegisterStore, W: RollbackWitness> {
    /// The device's counter register.
    pub register: &'a CounterRegister<'a, S, W>,
    /// Where sealed containers are kept until the publish is durable.
    pub files: &'a SavedSnapshotFiles,
    /// The verified key of the epoch the snapshot is sealed under.
    pub key: &'a EpochKey,
    /// This device.
    pub sealer: &'a SealerIdentity,
    /// The verified chain the receiver path judges the snapshot against.
    pub chain: &'a VerifiedState,
}

impl<S: RegisterStore, W: RollbackWitness> SnapshotSealer<'_, S, W> {
    fn context(&self) -> CounterContext {
        context_of(self.key, &self.sealer.fingerprint)
    }

    /// The saved container of this exact publish, if there is one that can still be sent.
    fn reusable(
        &self,
        fields: &SnapshotFields,
        plaintext_sha256: &[u8; 32],
    ) -> Result<Option<SealedSnapshot>, SnapshotError> {
        let context = self.context();
        // The register and its witness are consulted first, on every call: a device whose vault
        // was restored stops at `NonceStateLost` here, so nothing is sent from a rolled-back
        // timeline, and the saved record (which sits in the vault, beside the register) can only
        // name a counter the register has already passed.
        let next = self.register.check(&context)?;
        let Some(saved) = self.register.store().saved(&context)? else {
            return Ok(None);
        };
        if saved.object_id != fields.object_id
            || saved.base_seq != fields.base_seq
            || &saved.plaintext_sha256 != plaintext_sha256
        {
            return Ok(None);
        }
        self.saved_container(&saved, next)
    }

    /// The container a saved record names, if the file is still whole and still this key's and
    /// device's. `next` is the register's value for the context.
    fn saved_container(
        &self,
        saved: &SavedSealed,
        next: Option<u64>,
    ) -> Result<Option<SealedSnapshot>, SnapshotError> {
        if !next.is_some_and(|next| saved.counter < next) {
            return Err(NonceError::NonceStateLost(LostReason::SavedAheadOfRegister).into());
        }
        let Some(container) = self.files.read(&saved.object_id)? else {
            return Ok(None);
        };
        if container.len() as u64 != saved.container_len
            || sha256(&container) != saved.container_sha256
        {
            return Ok(None);
        }
        // The bytes must still belong to this key and device: a rotation or a re-enrollment since
        // the seal makes them another context's.
        let Ok(parsed) = parse_container(&container) else {
            return Ok(None);
        };
        if parsed.header.key_epoch != self.key.key_epoch()
            || parsed.header.device_key_fingerprint != self.sealer.fingerprint
            || parsed.header.invocation_counter != saved.counter
        {
            return Ok(None);
        }
        Ok(Some(SealedSnapshot {
            container,
            counter: saved.counter,
            reused: true,
        }))
    }

    /// The saved container of the publish `object_id`, if one is saved and can still be sent: what
    /// a resume sends instead of exporting and sealing again. The register and its witness are
    /// consulted first, as for a seal, so a restored vault stops at `NonceStateLost`.
    ///
    /// # Errors
    /// A [`SnapshotError`]; `Ok(None)` when nothing usable is saved for that id.
    pub fn saved(&self, object_id: &[u8; 16]) -> Result<Option<SealedSnapshot>, SnapshotError> {
        let context = self.context();
        let next = self.register.check(&context)?;
        let Some(saved) = self.register.store().saved(&context)? else {
            return Ok(None);
        };
        if &saved.object_id != object_id {
            return Ok(None);
        }
        self.saved_container(&saved, next)
    }

    /// Seal `plaintext` for publication, or hand back the saved container of the same publish.
    ///
    /// The order for a new seal: reserve the counter (register, then witness), seal, open the
    /// result through the receiver path and compare it with the plaintext, write the container
    /// file, and only then record it beside the register entry. A crash anywhere burns the
    /// counter and leaves nothing that could be sent twice with different bytes.
    ///
    /// # Errors
    /// A [`SnapshotError`]; on any error nothing may be published.
    pub fn seal(
        &self,
        fields: &SnapshotFields,
        plaintext: &[u8],
    ) -> Result<SealedSnapshot, SnapshotError> {
        if plaintext.len() > MAX_SNAPSHOT_PLAINTEXT {
            return Err(SnapshotSealError::SnapshotTooLarge.into());
        }
        // A plaintext no replica could verify is refused before a counter is spent.
        let commitments = closure_commitments_of(plaintext)?;
        let plaintext_sha256 = sha256(plaintext);
        if let Some(saved) = self.reusable(fields, &plaintext_sha256)? {
            return Ok(saved);
        }
        let context = self.context();
        let previous = self.register.store().saved(&context)?;
        let reservation = self.register.reserve(&context)?;
        let counter = reservation.counter();
        let container = seal_with_commitment(
            self.key,
            self.sealer,
            fields,
            commitments,
            reservation,
            plaintext,
        )?;
        let parsed = parse_container(&container).map_err(|_| SnapshotSealError::Unencodable)?;
        let opened = SnapshotOpener::new(self.key, self.chain)
            .open(&parsed)
            .map_err(SnapshotError::NotOpenable)?;
        if opened != plaintext {
            return Err(SnapshotError::Inconsistent);
        }
        self.files.write(&fields.object_id, &container)?;
        self.register.store().set_saved(
            &context,
            Some(&SavedSealed {
                object_id: fields.object_id,
                base_seq: fields.base_seq,
                plaintext_sha256,
                container_sha256: sha256(&container),
                container_len: container.len() as u64,
                counter,
            }),
        )?;
        // A different publish's file is dead once the record points at this one.
        if let Some(old) = previous.filter(|old| old.object_id != fields.object_id) {
            self.files.remove(&old.object_id)?;
        }
        Ok(SealedSnapshot {
            container,
            counter,
            reused: false,
        })
    }

    /// The publish is durable (the receipt has arrived): forget the saved container, record and
    /// file. Nothing is left that could send the snapshot again.
    ///
    /// # Errors
    /// A [`SnapshotError`].
    pub fn discard(&self, object_id: &[u8; 16]) -> Result<(), SnapshotError> {
        let context = self.context();
        if self
            .register
            .store()
            .saved(&context)?
            .is_some_and(|saved| &saved.object_id == object_id)
        {
            self.register.store().set_saved(&context, None)?;
        }
        self.files.remove(object_id)?;
        Ok(())
    }
}
