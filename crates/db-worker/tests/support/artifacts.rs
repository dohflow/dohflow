//! Test support for the artifact receiver: an epoch key, a protocol-following fake artifact service
//! (4.10: it checks only the framing and the size, keeps the first bytes for an address, and cannot
//! read an object), and publishers that seal real attachment and source-batch-part artifacts.

#![allow(dead_code)]

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

use sync_core::artifact::{
    parse_container, ArtifactId, ArtifactKind, ArtifactReceipt, TransportPlaintext,
    MAX_ARTIFACT_CONTAINER,
};
use sync_core::artifact_transport::{ArtifactStatus, ArtifactTransport, TransportError};
use sync_core::source_batch::SourceBatch;
use vault_crypto::sync_artifact::{first_artifact_id, open_artifact, Fixity, OpenedArtifact};
use vault_crypto::sync_artifact_flow::publish;
use vault_crypto::sync_envelope::EpochKey;
use vault_crypto::sync_keys::epoch_key_commitment;
use vault_crypto::SecretBytes;

pub const VAULT: [u8; 16] = [0x11; 16];

pub fn key_of(epoch: u64, first: u8) -> EpochKey {
    let raw: [u8; 32] = std::array::from_fn(|i| first.wrapping_add(i as u8));
    let secret = SecretBytes::new(raw);
    let commitment = epoch_key_commitment(&secret);
    EpochKey::from_verified(VAULT, epoch, secret, &commitment).unwrap()
}

type ObjectKey = ([u8; 16], u64, [u8; 32]);

#[derive(Default)]
pub struct FakeService {
    objects: RefCell<BTreeMap<ObjectKey, Vec<u8>>>,
    bound: RefCell<BTreeMap<ObjectKey, [u8; 32]>>,
    pub gets: Cell<usize>,
}

impl FakeService {
    /// The ciphertext hash of the container first put at `(epoch, id)`.
    pub fn bound_hash(&self, epoch: u64, id: &ArtifactId) -> [u8; 32] {
        self.bound.borrow()[&(VAULT, epoch, *id.as_bytes())]
    }

    pub fn lose(&self, epoch: u64, id: &ArtifactId) {
        self.objects
            .borrow_mut()
            .remove(&(VAULT, epoch, *id.as_bytes()));
    }

    pub fn serve(&self, epoch: u64, id: &ArtifactId, bytes: Vec<u8>) {
        self.objects
            .borrow_mut()
            .insert((VAULT, epoch, *id.as_bytes()), bytes);
    }

    pub fn corrupt(&self, epoch: u64, id: &ArtifactId) {
        let mut objects = self.objects.borrow_mut();
        let bytes = objects.get_mut(&(VAULT, epoch, *id.as_bytes())).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
    }
}

impl ArtifactTransport for FakeService {
    fn put(
        &self,
        vault_id: &[u8; 16],
        key_epoch: u64,
        artifact_id: &ArtifactId,
        container: &[u8],
    ) -> Result<ArtifactReceipt, TransportError> {
        let ciphertext = parse_container(container, MAX_ARTIFACT_CONTAINER)
            .map_err(|_| TransportError::PayloadTooLarge)?;
        let at = (*vault_id, key_epoch, *artifact_id.as_bytes());
        let receipt =
            ArtifactReceipt::for_ciphertext(*vault_id, key_epoch, *artifact_id, ciphertext);
        self.bound
            .borrow_mut()
            .entry(at)
            .or_insert(receipt.ciphertext_sha256);
        let mut objects = self.objects.borrow_mut();
        let stored = objects.entry(at).or_insert_with(|| container.to_vec());
        if stored.as_slice() != container {
            return Err(TransportError::ObjectConflict);
        }
        Ok(receipt)
    }

    fn get(
        &self,
        vault_id: &[u8; 16],
        key_epoch: u64,
        artifact_id: &ArtifactId,
    ) -> Result<Vec<u8>, TransportError> {
        self.gets.set(self.gets.get() + 1);
        self.objects
            .borrow()
            .get(&(*vault_id, key_epoch, *artifact_id.as_bytes()))
            .cloned()
            .ok_or(TransportError::NotFound)
    }

    fn status(
        &self,
        vault_id: &[u8; 16],
        key_epoch: u64,
        ids: &[ArtifactId],
    ) -> Result<Vec<ArtifactStatus>, TransportError> {
        let objects = self.objects.borrow();
        Ok(ids
            .iter()
            .map(|id| {
                let stored = objects.get(&(*vault_id, key_epoch, *id.as_bytes()));
                ArtifactStatus {
                    artifact_id: *id,
                    present: stored.is_some(),
                    pinned: false,
                    receipt: stored.map(|container| {
                        let ciphertext =
                            parse_container(container, MAX_ARTIFACT_CONTAINER).unwrap();
                        ArtifactReceipt::for_ciphertext(*vault_id, key_epoch, *id, ciphertext)
                    }),
                }
            })
            .collect())
    }
}

pub fn hyphenated(id: &[u8; 16]) -> String {
    let hex: String = id.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// A published artifact: its address, the hash the service holds for it, and its logical id.
#[derive(Clone, Copy, Debug)]
pub struct Published {
    pub id: ArtifactId,
    pub hash: [u8; 32],
}

/// Publish an attachment artifact for `attachment_id` carrying `bytes`; the address is derived from
/// `address_byte` so tests choose distinct ones.
pub fn publish_attachment(
    service: &FakeService,
    key: &EpochKey,
    address_byte: u8,
    attachment_id: [u8; 16],
    bytes: &[u8],
    mime_type: &str,
) -> Published {
    let id = first_artifact_id(key, &[address_byte; 32]);
    let metadata = format!(
        r#"{{"attachment_id":"{}","mime_type":"{mime_type}"}}"#,
        hyphenated(&attachment_id)
    );
    publish(
        service,
        key,
        &TransportPlaintext {
            artifact_id: id,
            kind: ArtifactKind::Attachment,
            logical_metadata: metadata.as_bytes(),
            blob: bytes,
        },
        Fixity::First,
    )
    .unwrap();
    Published {
        id,
        hash: service.bound_hash(key.key_epoch(), &id),
    }
}

/// Publish every part of `batch` as a source-batch-part artifact, with addresses derived from
/// `address_byte` and the part index.
pub fn publish_batch(
    service: &FakeService,
    key: &EpochKey,
    address_byte: u8,
    batch: &SourceBatch,
) -> Vec<Published> {
    batch
        .split()
        .unwrap()
        .iter()
        .map(|part| {
            let id = first_artifact_id(key, &[address_byte.wrapping_add(part.index as u8); 32]);
            publish(
                service,
                key,
                &TransportPlaintext {
                    artifact_id: id,
                    kind: ArtifactKind::SourceBatchPart,
                    logical_metadata: &part.logical_metadata,
                    blob: &part.blob,
                },
                Fixity::First,
            )
            .unwrap();
            Published {
                id,
                hash: service.bound_hash(key.key_epoch(), &id),
            }
        })
        .collect()
}

/// Fetch and open an artifact as the receiver would, with the hash the service holds.
pub fn opened(service: &FakeService, key: &EpochKey, published: &Published) -> OpenedArtifact {
    let container = service.get(&VAULT, key.key_epoch(), &published.id).unwrap();
    open_artifact(key, &published.id, &container, MAX_ARTIFACT_CONTAINER).unwrap()
}
