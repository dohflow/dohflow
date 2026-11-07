//! Publishing, fetching and materializing artifacts over an in-memory fake of the service's
//! artifact endpoints (sync-service-protocol 4.10; vault-format.md C.9; personal-cfo-0g528.2 S2c),
//! the two-replica fixture with independently generated DEKs, and the guard that keeps the transport
//! seam without a production implementor.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::Path;

use sha2::{Digest, Sha256};
use sync_core::artifact::{
    parse_container, ArtifactId, ArtifactKind, ArtifactReceipt, TransportPlaintext,
    MAX_ARTIFACT_CONTAINER,
};
use sync_core::artifact_transport::{ArtifactStatus, ArtifactTransport, TransportError};
use vault_crypto::attachment::{decrypt_blob, storage_id, unwrap_content_key};
use vault_crypto::sync_artifact::{
    first_artifact_id, readdress_container, Fixity, OpenArtifactError, SealArtifactError,
};
use vault_crypto::sync_artifact_flow::{
    fetch, materialize_attachment, publish, FetchError, MaterializeError, PublishError,
};
use vault_crypto::sync_envelope::EpochKey;
use vault_crypto::sync_keys::epoch_key_commitment;
use vault_crypto::{generate_dek, SecretBytes};

const VAULT: [u8; 16] = [0x11; 16];
const ATTACHMENT: &str = "01938f2e-7c1a-7b3d-9a4e-123456789abc";
const BYTES: &[u8] = b"synthetic statement bytes";

fn key_of(epoch: u64, first: u8) -> EpochKey {
    let raw: [u8; 32] = std::array::from_fn(|i| first.wrapping_add(i as u8));
    let secret = SecretBytes::new(raw);
    let commitment = epoch_key_commitment(&secret);
    EpochKey::from_verified(VAULT, epoch, secret, &commitment).unwrap()
}

/// Fetch with the hash the producer bound: that of the container first put at the address.
fn fetch_bound(
    service: &FakeService,
    key: &EpochKey,
    id: &ArtifactId,
) -> Result<vault_crypto::sync_artifact::OpenedArtifact, FetchError> {
    fetch(service, key, id, &service.bound_hash(key.key_epoch(), id))
}

// ---- the fake service ---------------------------------------------------------------------------

type ObjectKey = ([u8; 16], u64, [u8; 32]);

/// `putArtifact`, `getArtifact` and `getArtifactStatus` as protocol 4.10 describes them: it checks
/// only the framing and the size, stores the first bytes for an address, answers an exact retry with
/// the same receipt, and refuses different bytes with `ObjectConflict`. It cannot read an object.
#[derive(Default)]
struct FakeService {
    objects: RefCell<BTreeMap<ObjectKey, Vec<u8>>>,
    /// The ciphertext hash of the first container put at each address: the hash a producer that
    /// published it would bind in an envelope.
    bound: RefCell<BTreeMap<ObjectKey, [u8; 32]>>,
    puts: Cell<usize>,
    down: Cell<bool>,
}

impl FakeService {
    /// The hash bound for `(epoch, id)`: that of the container first put there.
    fn bound_hash(&self, epoch: u64, id: &ArtifactId) -> [u8; 32] {
        self.bound
            .borrow()
            .get(&(VAULT, epoch, *id.as_bytes()))
            .copied()
            .unwrap_or([0; 32])
    }

    fn lose(&self, epoch: u64, id: &ArtifactId) {
        self.objects
            .borrow_mut()
            .remove(&(VAULT, epoch, *id.as_bytes()));
    }
    /// Serve `bytes` for `(epoch, id)` from now on, whatever was put there.
    fn serve(&self, epoch: u64, id: &ArtifactId, bytes: Vec<u8>) {
        self.objects
            .borrow_mut()
            .insert((VAULT, epoch, *id.as_bytes()), bytes);
    }

    fn corrupt(&self, epoch: u64, id: &ArtifactId) {
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
        if self.down.get() {
            return Err(TransportError::Unavailable);
        }
        self.puts.set(self.puts.get() + 1);
        let ciphertext = parse_container(container, MAX_ARTIFACT_CONTAINER)
            .map_err(|_| TransportError::PayloadTooLarge)?;
        self.bound
            .borrow_mut()
            .entry((*vault_id, key_epoch, *artifact_id.as_bytes()))
            .or_insert_with(|| {
                ArtifactReceipt::for_ciphertext(*vault_id, key_epoch, *artifact_id, ciphertext)
                    .ciphertext_sha256
            });
        let mut objects = self.objects.borrow_mut();
        let stored = objects
            .entry((*vault_id, key_epoch, *artifact_id.as_bytes()))
            .or_insert_with(|| container.to_vec());
        if stored.as_slice() != container {
            return Err(TransportError::ObjectConflict);
        }
        Ok(ArtifactReceipt::for_ciphertext(
            *vault_id,
            key_epoch,
            *artifact_id,
            ciphertext,
        ))
    }

    fn get(
        &self,
        vault_id: &[u8; 16],
        key_epoch: u64,
        artifact_id: &ArtifactId,
    ) -> Result<Vec<u8>, TransportError> {
        if self.down.get() {
            return Err(TransportError::Unavailable);
        }
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

/// A service that answers every put with a receipt for something else.
struct LyingService(FakeService);
impl ArtifactTransport for LyingService {
    fn put(
        &self,
        vault_id: &[u8; 16],
        key_epoch: u64,
        artifact_id: &ArtifactId,
        container: &[u8],
    ) -> Result<ArtifactReceipt, TransportError> {
        let mut receipt = self.0.put(vault_id, key_epoch, artifact_id, container)?;
        receipt.ciphertext_sha256[0] ^= 1;
        Ok(receipt)
    }
    fn get(&self, v: &[u8; 16], e: u64, a: &ArtifactId) -> Result<Vec<u8>, TransportError> {
        self.0.get(v, e, a)
    }
    fn status(
        &self,
        v: &[u8; 16],
        e: u64,
        ids: &[ArtifactId],
    ) -> Result<Vec<ArtifactStatus>, TransportError> {
        self.0.status(v, e, ids)
    }
}

// ---- helpers -------------------------------------------------------------------------------------

fn metadata() -> Vec<u8> {
    format!(r#"{{"attachment_id":"{ATTACHMENT}","mime_type":"text/plain"}}"#).into_bytes()
}

fn id_for(key: &EpochKey, storage_hex: &str) -> ArtifactId {
    let raw: [u8; 32] =
        std::array::from_fn(|i| u8::from_str_radix(&storage_hex[2 * i..2 * i + 2], 16).unwrap());
    first_artifact_id(key, &raw)
}

// ---- publish ---------------------------------------------------------------------------------------

#[test]
fn a_published_artifact_returns_the_receipt_the_bytes_produce_and_a_retry_the_same_one() {
    let service = FakeService::default();
    let k = key_of(1, 0);
    let id = ArtifactId::from_derived([0x21; 32]);
    let meta = metadata();
    let plaintext = TransportPlaintext {
        artifact_id: id,
        kind: ArtifactKind::Attachment,
        logical_metadata: &meta,
        blob: BYTES,
    };
    let first = publish(&service, &k, &plaintext, Fixity::First).unwrap();
    assert_eq!(first.receipt.artifact_id, id);
    assert_eq!(first.receipt.key_epoch, 1);
    // A crash after the put, then a retry from the stored fixity: identical bytes, the same receipt.
    let again = publish(
        &service,
        &k,
        &plaintext,
        Fixity::Expected(first.plaintext_digest),
    )
    .unwrap();
    assert_eq!(again, first);
    assert_eq!(service.puts.get(), 2);
    assert_eq!(
        service.objects.borrow().len(),
        1,
        "one object for one address"
    );
}

#[test]
fn a_changed_plaintext_is_refused_before_the_service_is_asked() {
    let service = FakeService::default();
    let k = key_of(1, 0);
    let id = ArtifactId::from_derived([0x21; 32]);
    let meta = metadata();
    let plaintext = TransportPlaintext {
        artifact_id: id,
        kind: ArtifactKind::Attachment,
        logical_metadata: &meta,
        blob: BYTES,
    };
    let first = publish(&service, &k, &plaintext, Fixity::First).unwrap();
    let changed = TransportPlaintext {
        blob: b"a different statement",
        ..plaintext
    };
    assert_eq!(
        publish(
            &service,
            &k,
            &changed,
            Fixity::Expected(first.plaintext_digest)
        ),
        Err(PublishError::Seal(
            SealArtifactError::ArtifactDescriptorChanged
        ))
    );
    assert_eq!(
        service.puts.get(),
        1,
        "the service never saw the changed bytes"
    );
}

#[test]
fn different_bytes_at_an_existing_address_are_an_object_conflict() {
    let service = FakeService::default();
    let k = key_of(1, 0);
    let id = ArtifactId::from_derived([0x21; 32]);
    let meta = metadata();
    let first = TransportPlaintext {
        artifact_id: id,
        kind: ArtifactKind::Attachment,
        logical_metadata: &meta,
        blob: BYTES,
    };
    publish(&service, &k, &first, Fixity::First).unwrap();
    // A different plaintext sealed without the fixity guard (a producer bug) is stopped by the service.
    let other = TransportPlaintext {
        blob: b"a different statement",
        ..first
    };
    assert_eq!(
        publish(&service, &k, &other, Fixity::First),
        Err(PublishError::Transport(TransportError::ObjectConflict))
    );
}

#[test]
fn a_receipt_that_is_not_the_one_the_bytes_produce_is_never_recorded() {
    let lying = LyingService(FakeService::default());
    let id = ArtifactId::from_derived([0x21; 32]);
    let meta = metadata();
    let plaintext = TransportPlaintext {
        artifact_id: id,
        kind: ArtifactKind::Attachment,
        logical_metadata: &meta,
        blob: BYTES,
    };
    assert_eq!(
        publish(&lying, &key_of(1, 0), &plaintext, Fixity::First),
        Err(PublishError::ReceiptMismatch)
    );
}

#[test]
fn an_unreachable_service_is_a_transport_fault() {
    let service = FakeService::default();
    service.down.set(true);
    let id = ArtifactId::from_derived([0x21; 32]);
    let meta = metadata();
    let plaintext = TransportPlaintext {
        artifact_id: id,
        kind: ArtifactKind::Attachment,
        logical_metadata: &meta,
        blob: BYTES,
    };
    assert_eq!(
        publish(&service, &key_of(1, 0), &plaintext, Fixity::First),
        Err(PublishError::Transport(TransportError::Unavailable))
    );
}

// ---- fetch -----------------------------------------------------------------------------------------

#[test]
fn a_missing_a_corrupt_and_an_unreachable_object_are_service_faults_never_artifact_facts() {
    let service = FakeService::default();
    let k = key_of(1, 0);
    let id = ArtifactId::from_derived([0x21; 32]);
    let meta = metadata();
    let plaintext = TransportPlaintext {
        artifact_id: id,
        kind: ArtifactKind::Attachment,
        logical_metadata: &meta,
        blob: BYTES,
    };
    publish(&service, &k, &plaintext, Fixity::First).unwrap();
    assert!(fetch_bound(&service, &k, &id).is_ok());
    // The wrong epoch has no object.
    assert_eq!(
        fetch_bound(&service, &key_of(2, 0), &id).map(|_| ()),
        Err(FetchError::Missing)
    );
    // Bytes that do not hash to the bound value are not the artifact: a service or transport fault.
    service.corrupt(1, &id);
    assert_eq!(
        fetch_bound(&service, &k, &id).map(|_| ()),
        Err(FetchError::HashMismatch)
    );
    service.lose(1, &id);
    assert_eq!(
        fetch_bound(&service, &k, &id).map(|_| ()),
        Err(FetchError::Missing)
    );
    service.down.set(true);
    assert_eq!(
        fetch_bound(&service, &k, &id).map(|_| ()),
        Err(FetchError::Unavailable)
    );
}

#[test]
fn an_object_opened_under_the_wrong_key_is_an_outer_aead_fault() {
    let service = FakeService::default();
    let id = ArtifactId::from_derived([0x21; 32]);
    let meta = metadata();
    let plaintext = TransportPlaintext {
        artifact_id: id,
        kind: ArtifactKind::Attachment,
        logical_metadata: &meta,
        blob: BYTES,
    };
    publish(&service, &key_of(1, 0), &plaintext, Fixity::First).unwrap();
    assert_eq!(
        fetch_bound(&service, &key_of(1, 0x80), &id).map(|_| ()),
        Err(FetchError::OuterAead)
    );
    let _ = OpenArtifactError::NotOpenable;
}

// ---- the two-replica fixture (AC5) ------------------------------------------------------------------

#[test]
fn two_independently_keyed_replicas_round_trip_an_attachment_with_different_local_fields() {
    let service = FakeService::default();
    let k = key_of(1, 0);
    let dek_a = generate_dek().unwrap();
    let dek_b = generate_dek().unwrap();

    // Replica A has the attachment locally and publishes it.
    let storage_a = storage_id(&dek_a, BYTES);
    let id = id_for(&k, storage_a.as_str());
    let meta = metadata();
    let plaintext = TransportPlaintext {
        artifact_id: id,
        kind: ArtifactKind::Attachment,
        logical_metadata: &meta,
        blob: BYTES,
    };
    let published = publish(&service, &k, &plaintext, Fixity::First).unwrap();

    // The transport plaintext carries no origin crypto field: no storage id (text or raw), nothing
    // of A's DEK.
    let encoded = plaintext.encode().unwrap();
    let hex = storage_a.as_str().as_bytes();
    assert!(!encoded.windows(hex.len()).any(|w| w == hex));
    let raw: Vec<u8> = (0..32)
        .map(|i| u8::from_str_radix(&storage_a.as_str()[2 * i..2 * i + 2], 16).unwrap())
        .collect();
    assert!(!encoded.windows(32).any(|w| w == raw.as_slice()));

    // Replica B fetches it by the canonical id, verifies, and re-encrypts under its own DEK.
    let opened = fetch_bound(&service, &k, &id).unwrap();
    assert_eq!(
        opened.plaintext().artifact_id,
        id,
        "the origin's id is kept for references"
    );
    let local = materialize_attachment(&dek_b, &opened).unwrap();
    assert_eq!(local.attachment_id[0], 0x01);

    // B's local fields are B's own.
    assert_ne!(
        local.storage_id, storage_a,
        "B's address is keyed under B's DEK"
    );
    assert_eq!(local.storage_id, storage_id(&dek_b, BYTES));
    // B reads its bytes back under its own DEK.
    let key_b = unwrap_content_key(&dek_b, &local.wrapped_key).unwrap();
    assert_eq!(decrypt_blob(&key_b, &local.blob).unwrap(), BYTES);
    // A's DEK cannot unwrap B's key (wrong-DEK unwrap fails).
    assert!(unwrap_content_key(&dek_a, &local.wrapped_key).is_err());

    // A second materialization on B uses a fresh content key and nonces, but the same StorageId_B,
    // which is what dedupe keys on.
    let again = materialize_attachment(&dek_b, &opened).unwrap();
    assert_eq!(again.storage_id, local.storage_id);
    assert_ne!(again.blob, local.blob);
    assert_ne!(again.wrapped_key, local.wrapped_key);

    // The receipt A recorded is the one the service would give any later status query.
    let status = service.status(&VAULT, 1, &[id]).unwrap();
    assert_eq!(status[0].receipt, Some(published.receipt));
}

#[test]
fn a_corrupt_transport_fails_and_leaves_nothing_to_store() {
    let service = FakeService::default();
    let k = key_of(1, 0);
    let dek_b = generate_dek().unwrap();
    let id = ArtifactId::from_derived([0x21; 32]);
    let meta = metadata();
    let plaintext = TransportPlaintext {
        artifact_id: id,
        kind: ArtifactKind::Attachment,
        logical_metadata: &meta,
        blob: BYTES,
    };
    publish(&service, &k, &plaintext, Fixity::First).unwrap();
    service.corrupt(1, &id);
    // The fetch fails before there is anything to materialize, so no blob, key or row is produced.
    assert!(fetch_bound(&service, &k, &id).is_err());
    let _ = dek_b;
}

#[test]
fn an_artifact_that_is_not_an_attachment_is_not_materialized_as_one() {
    let service = FakeService::default();
    let k = key_of(1, 0);
    let dek_b = generate_dek().unwrap();
    let id = ArtifactId::from_derived([0x31; 32]);
    let part_meta =
        format!(r#"{{"batch_id":"{ATTACHMENT}","part_count":"1","part_index":"0"}}"#).into_bytes();
    let plaintext = TransportPlaintext {
        artifact_id: id,
        kind: ArtifactKind::SourceBatchPart,
        logical_metadata: &part_meta,
        blob: b"{}",
    };
    publish(&service, &k, &plaintext, Fixity::First).unwrap();
    let opened = fetch_bound(&service, &k, &id).unwrap();
    assert_eq!(
        materialize_attachment(&dek_b, &opened).map(|_| ()),
        Err(MaterializeError::NotAnAttachment)
    );
    // And an attachment whose metadata names no attachment id.
    let bare = br#"{"mime_type":"text/plain"}"#;
    let id2 = ArtifactId::from_derived([0x32; 32]);
    let no_binding = TransportPlaintext {
        artifact_id: id2,
        kind: ArtifactKind::Attachment,
        logical_metadata: bare,
        blob: BYTES,
    };
    publish(&service, &k, &no_binding, Fixity::First).unwrap();
    let opened = fetch_bound(&service, &k, &id2).unwrap();
    assert_eq!(
        materialize_attachment(&dek_b, &opened).map(|_| ()),
        Err(MaterializeError::NotAnAttachment)
    );
}

#[test]
fn a_device_that_did_not_originate_an_artifact_re_addresses_it_to_the_same_id_the_origin_would() {
    let service = FakeService::default();
    let (k1, k2) = (key_of(1, 0), key_of(2, 0x40));
    let dek_a = generate_dek().unwrap();
    let storage_a = storage_id(&dek_a, BYTES);
    let id1 = id_for(&k1, storage_a.as_str());
    let meta = metadata();
    let plaintext = TransportPlaintext {
        artifact_id: id1,
        kind: ArtifactKind::Attachment,
        logical_metadata: &meta,
        blob: BYTES,
    };
    publish(&service, &k1, &plaintext, Fixity::First).unwrap();

    // Device B never saw A's DEK or A's storage id. It holds only the epoch-1 id from the log and
    // both epoch keys, and runs the cutover from the object the service holds.
    let old = service.get(&VAULT, 1, &id1).unwrap();
    let by_b = readdress_container(&k1, &k2, &id1, &old, Fixity::First).unwrap();
    // A, which can compute the same thing from its local copy, gets the identical id and bytes.
    let by_a = vault_crypto::sync_artifact::readdress_plaintext(
        &k1,
        &k2,
        &id1,
        &plaintext,
        &plaintext.digest().unwrap(),
        Fixity::First,
    )
    .unwrap();
    assert_eq!(by_b.artifact_id, by_a.artifact_id);
    assert_eq!(by_b.sealed.container, by_a.sealed.container);

    // Published under epoch 2, it opens there and the epoch-1 object is untouched.
    let moved = TransportPlaintext {
        artifact_id: by_b.artifact_id,
        ..plaintext
    };
    publish(
        &service,
        &k2,
        &moved,
        Fixity::Expected(by_b.sealed.plaintext_digest),
    )
    .unwrap();
    assert!(fetch_bound(&service, &k2, &by_b.artifact_id).is_ok());
    assert!(fetch_bound(&service, &k1, &id1).is_ok());
    assert_ne!(by_b.artifact_id, id1);
    let digest: [u8; 32] = Sha256::digest(&old).into();
    assert_ne!(digest, Sha256::digest(&by_b.sealed.container).as_slice());
}

// ---- the transport seam has no production implementor -------------------------------------------------

fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

#[test]
fn artifact_transport_has_no_implementor_outside_tests() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for entry in std::fs::read_dir(workspace.join("crates"))
        .unwrap()
        .flatten()
    {
        walk(&entry.path().join("src"), &mut files);
    }
    walk(&workspace.join("apps/desktop/src-tauri/src"), &mut files);
    assert!(files.len() > 100, "found {} source files", files.len());
    let mut implementors = Vec::new();
    for path in files {
        let source = std::fs::read_to_string(&path).unwrap();
        // Code only (line comments dropped), whitespace collapsed so a split header still matches.
        let code: String = source
            .lines()
            .map(|line| line.split("//").next().unwrap_or(""))
            .collect::<Vec<_>>()
            .join(" ");
        let code = code.split_whitespace().collect::<Vec<_>>().join(" ");
        if code.contains("ArtifactTransport for") {
            implementors.push(path.display().to_string());
        }
    }
    assert!(
        implementors.is_empty(),
        "ArtifactTransport implemented outside tests (the client engine, personal-cfo-elciw, owns \
         the first production implementor): {implementors:?}"
    );
}

#[test]
fn only_a_fact_about_the_artifact_has_a_stage_9_verdict_and_service_faults_do_not() {
    use sync_core::artifact_closure::Mismatch;
    let id = ArtifactId::from_derived([0x21; 32]);
    assert_eq!(FetchError::Missing.mismatch(id), None);
    assert_eq!(FetchError::Unavailable.mismatch(id), None);
    assert_eq!(FetchError::HashMismatch.mismatch(id), None);
    // Bytes that hash to the bound value are one fixed object every replica judges alike: failing to
    // open or validate it is the producer's fault and voidable (stage 9(b)).
    assert_eq!(
        FetchError::OuterAead.mismatch(id),
        Some(Mismatch::ArtifactInvalid(id))
    );
    assert_eq!(
        FetchError::Invalid(OpenArtifactError::WrongAddress).mismatch(id),
        Some(Mismatch::ArtifactInvalid(id))
    );
}

#[test]
fn served_bytes_that_are_not_a_container_are_not_the_bound_artifact_and_have_no_verdict() {
    let service = FakeService::default();
    let k = key_of(1, 0);
    let id = ArtifactId::from_derived([0x21; 32]);
    let meta = metadata();
    let plaintext = TransportPlaintext {
        artifact_id: id,
        kind: ArtifactKind::Attachment,
        logical_metadata: &meta,
        blob: BYTES,
    };
    publish(&service, &k, &plaintext, Fixity::First).unwrap();
    let honest = service.bound_hash(1, &id);
    let good = service.objects.borrow()[&(VAULT, 1, *id.as_bytes())].clone();
    assert!(fetch(&service, &k, &id, &honest).is_ok());

    let mut bad_magic = good.clone();
    bad_magic[0] ^= 0xff;
    // The container frame ends with a length that must equal the body that follows it; make the
    // body one byte longer, then one byte shorter.
    let mut too_long = good.clone();
    too_long.push(0);
    let too_short = good[..good.len() - 1].to_vec();
    let nothing = Vec::new();
    let three_bytes = vec![b'D', b'F', b'S'];
    for (label, bytes) in [
        ("bad magic", bad_magic),
        ("a trailing byte after the declared length", too_long),
        ("a body shorter than the declared length", too_short),
        ("empty", nothing),
        ("three bytes", three_bytes),
    ] {
        service.serve(1, &id, bytes);
        let error = fetch(&service, &k, &id, &honest).err();
        assert_eq!(error, Some(FetchError::HashMismatch), "{label}");
        assert_eq!(
            error.and_then(|e| e.mismatch(id)),
            None,
            "{label}: a service fault leaves the envelope not determinable, never voidable"
        );
    }
}

#[test]
fn a_member_alone_cannot_hold_an_envelope_at_a_position_by_naming_an_id_it_never_stored_properly() {
    use sync_core::artifact_closure::Mismatch;
    let service = FakeService::default();
    let k = key_of(1, 0);
    let id = ArtifactId::from_derived([0x21; 32]);
    let meta = metadata();
    let plaintext = TransportPlaintext {
        artifact_id: id,
        kind: ArtifactKind::Attachment,
        logical_metadata: &meta,
        blob: BYTES,
    };
    publish(&service, &k, &plaintext, Fixity::First).unwrap();
    let honest = service.bound_hash(1, &id);

    // The service serves other bytes than the producer bound (a faulty or hostile service): the
    // hash does not match, so the envelope is not determinable and no verdict is formed.
    service.corrupt(1, &id);
    let error = fetch(&service, &k, &id, &honest).err();
    assert_eq!(error, Some(FetchError::HashMismatch));
    assert_eq!(error.and_then(|e| e.mismatch(id)), None);

    // A member that binds the hash of an object that does not open (here: sealed under another key)
    // has fixed those bytes: every replica sees the same failure, which is the producer's fault.
    let other = key_of(1, 0x80);
    let other_id = ArtifactId::from_derived([0x22; 32]);
    let other_plaintext = TransportPlaintext {
        artifact_id: other_id,
        kind: ArtifactKind::Attachment,
        logical_metadata: &meta,
        blob: BYTES,
    };
    publish(&service, &other, &other_plaintext, Fixity::First).unwrap();
    let bound_by_the_member = service.bound_hash(1, &other_id);
    let result = fetch(&service, &k, &other_id, &bound_by_the_member);
    assert_eq!(
        result.err().and_then(|e| e.mismatch(other_id)),
        Some(Mismatch::ArtifactInvalid(other_id)),
        "bytes that match the bound hash and fail the outer AEAD: the producer's fault"
    );
}
