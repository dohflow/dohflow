//! Class-1b artifact addresses, sealing and opening (vault-format.md C.3 and C.9;
//! personal-cfo-0g528.2 S2b): the spec's known-answer vectors reproduced by production code, the
//! open path's refusals, the fixed-plaintext guard, the chained re-address and its in-memory reseal.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use hkdf::Hkdf;
use sha2::Sha256;
use sync_core::artifact::{
    artifact_aad, encode_container, ArtifactError, ArtifactId, ArtifactKind, ArtifactReceipt,
    LogicalBinding, TransportPlaintext, MAX_ARTIFACT_CONTAINER,
};
use vault_crypto::sync_artifact::{
    first_artifact_id, first_attachment_artifact_id, open_artifact, readdress_container,
    readdress_id, readdress_plaintext, seal_artifact, Fixity, OpenArtifactError, ReaddressError,
    SealArtifactError,
};
use vault_crypto::sync_envelope::EpochKey;
use vault_crypto::sync_keys::epoch_key_commitment;
use vault_crypto::SecretBytes;

const SPEC: &str = include_str!("../../../docs/architecture/vault-format.md");
const VAULT: [u8; 16] = [0x11; 16];
const STORAGE: [u8; 32] = [0x5a; 32];
const METADATA: &[u8] = br#"{"mime_type":"text/plain"}"#;
const DOCUMENT: &[u8] = b"synthetic document";

fn vector(name: &str) -> Vec<u8> {
    let marker = format!("<!-- sync-vector: {name}=");
    let start = SPEC
        .find(&marker)
        .unwrap_or_else(|| panic!("no vector {name}"))
        + marker.len();
    let hex = SPEC[start..].split(" -->").next().unwrap();
    (0..hex.len() / 2)
        .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}

fn key_of(vault: [u8; 16], epoch: u64, first_byte: u8) -> EpochKey {
    let raw: [u8; 32] = std::array::from_fn(|i| first_byte.wrapping_add(i as u8));
    let secret = SecretBytes::new(raw);
    let commitment = epoch_key_commitment(&secret);
    EpochKey::from_verified(vault, epoch, secret, &commitment).unwrap()
}

/// The spec's epoch-1 key `00 01 .. 1f` and epoch-2 key `40 41 .. 5f`.
fn k1() -> EpochKey {
    key_of(VAULT, 1, 0x00)
}
fn k2() -> EpochKey {
    key_of(VAULT, 2, 0x40)
}

fn sample(id: ArtifactId) -> TransportPlaintext<'static> {
    TransportPlaintext {
        artifact_id: id,
        kind: ArtifactKind::Attachment,
        logical_metadata: METADATA,
        blob: DOCUMENT,
    }
}

fn id_1() -> ArtifactId {
    first_artifact_id(&k1(), &STORAGE)
}

// ---- the spec's vectors, reproduced by production code ---------------------------------------

#[test]
fn the_addresses_match_the_spec_vectors() {
    assert_eq!(id_1().as_bytes().as_slice(), vector("artifact_id_epoch1"));
    assert_eq!(
        first_artifact_id(&k2(), &STORAGE).as_bytes().as_slice(),
        vector("artifact_id_epoch2")
    );
    assert_eq!(
        readdress_id(&k2(), &id_1()).as_bytes().as_slice(),
        vector("artifact_id_readdress_epoch2")
    );
    assert_eq!(artifact_aad(&VAULT, 1, &id_1()), vector("artifact_aad"));
}

#[test]
fn the_sealed_object_its_container_and_its_receipt_match_the_spec_vectors() {
    let sealed = seal_artifact(&k1(), &sample(id_1()), Fixity::First).unwrap();
    assert_eq!(sealed.container, vector("artifact_container"));
    let ciphertext = vector("artifact_object_ciphertext");
    assert_eq!(sealed.container, encode_container(&ciphertext));
    assert_eq!(sealed.receipt.encode(), vector("artifact_receipt"));
    assert_eq!(
        sealed.receipt,
        ArtifactReceipt::for_ciphertext(VAULT, 1, id_1(), &ciphertext)
    );
    assert_eq!(
        sealed.plaintext_digest,
        sample(id_1()).digest().unwrap(),
        "the digest to store is the digest of the complete transport plaintext"
    );
}

#[test]
fn the_vector_object_opens_and_is_the_sample_artifact() {
    let opened = open_artifact(
        &k1(),
        &id_1(),
        &vector("artifact_container"),
        MAX_ARTIFACT_CONTAINER,
    )
    .unwrap();
    let plaintext = opened.plaintext();
    assert_eq!(plaintext, sample(id_1()));
    assert_eq!(plaintext.blob, DOCUMENT);
    // The sample is a format vector: it names no attachment id, so it has no logical binding.
    assert_eq!(plaintext.binding(), Err(ArtifactError::BadLogicalMetadata));
}

/// An independent derivation of the object key from the spec text (C.3), so the seal path is not
/// only checked against itself.
fn spec_object_key(key_bytes: &[u8; 32], epoch: u64, id: &ArtifactId) -> [u8; 32] {
    let mut info = Vec::new();
    let label = b"dohflow/sync-artifact-object/v1";
    info.extend_from_slice(&(label.len() as u16).to_be_bytes());
    info.extend_from_slice(label);
    info.extend_from_slice(&1_u16.to_be_bytes());
    info.extend_from_slice(&VAULT);
    info.extend_from_slice(&epoch.to_be_bytes());
    info.extend_from_slice(id.as_bytes());
    let mut out = [0_u8; 32];
    Hkdf::<Sha256>::from_prk(key_bytes)
        .unwrap()
        .expand(&info, &mut out)
        .unwrap();
    out
}

fn raw_seal(object_key: &[u8; 32], aad: &[u8], plaintext: &[u8]) -> Vec<u8> {
    Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(object_key))
        .encrypt(
            Nonce::from_slice(&[0_u8; 12]),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .unwrap()
}

#[test]
fn the_object_key_derivation_is_the_spec_vector_and_production_seals_with_it() {
    let k_bytes: [u8; 32] = std::array::from_fn(|i| i as u8);
    let key = spec_object_key(&k_bytes, 1, &id_1());
    assert_eq!(key.as_slice(), vector("artifact_object_key"));
    let ct = raw_seal(
        &key,
        &artifact_aad(&VAULT, 1, &id_1()),
        &sample(id_1()).encode().unwrap(),
    );
    assert_eq!(ct, vector("artifact_object_ciphertext"));
}

// ---- the open path ---------------------------------------------------------------------------------

fn sealed_container() -> Vec<u8> {
    seal_artifact(&k1(), &sample(id_1()), Fixity::First)
        .unwrap()
        .container
}

fn open(key: &EpochKey, id: &ArtifactId, container: &[u8]) -> Result<(), OpenArtifactError> {
    open_artifact(key, id, container, MAX_ARTIFACT_CONTAINER).map(|_| ())
}

#[test]
fn a_wrong_key_a_wrong_address_a_wrong_vault_and_tampering_fail_closed() {
    let container = sealed_container();
    assert_eq!(open(&k1(), &id_1(), &container), Ok(()));
    // Another K_e for the same epoch number and vault.
    assert_eq!(
        open(&key_of(VAULT, 1, 0x80), &id_1(), &container),
        Err(OpenArtifactError::NotOpenable)
    );
    // The epoch-2 key, and the same key under another epoch number.
    assert_eq!(
        open(&k2(), &id_1(), &container),
        Err(OpenArtifactError::NotOpenable)
    );
    assert_eq!(
        open(&key_of(VAULT, 2, 0x00), &id_1(), &container),
        Err(OpenArtifactError::NotOpenable)
    );
    // Another vault with the same key bytes.
    assert_eq!(
        open(&key_of([0x12; 16], 1, 0x00), &id_1(), &container),
        Err(OpenArtifactError::NotOpenable)
    );
    // The same bytes presented under another address do not authenticate.
    assert_eq!(
        open(&k1(), &ArtifactId::from_derived([9; 32]), &container),
        Err(OpenArtifactError::NotOpenable)
    );
    // Every flipped byte of the ciphertext (first, middle, last: the tag) fails.
    let ct_start = 4 + 2 + 8;
    for at in [
        ct_start,
        ct_start + container.len() / 2 - 7,
        container.len() - 1,
    ] {
        let mut flipped = container.clone();
        flipped[at] ^= 1;
        assert_eq!(
            open(&k1(), &id_1(), &flipped),
            Err(OpenArtifactError::NotOpenable),
            "byte {at}"
        );
    }
}

#[test]
fn container_defects_are_refused_before_any_decryption() {
    let container = sealed_container();
    let mut bad_magic = container.clone();
    bad_magic[0] = b'X';
    assert!(matches!(
        open(&k1(), &id_1(), &bad_magic),
        Err(OpenArtifactError::Container(_))
    ));
    assert!(matches!(
        open(&k1(), &id_1(), &container[..container.len() - 1]),
        Err(OpenArtifactError::Container(_))
    ));
    assert!(matches!(
        open(&k1(), &id_1(), &[]),
        Err(OpenArtifactError::Container(_))
    ));
    // The size cap is the caller's, checked first.
    assert_eq!(
        open_artifact(&k1(), &id_1(), &container, container.len() - 1).map(|_| ()),
        Err(OpenArtifactError::Container(
            ArtifactError::ContainerTooLarge
        ))
    );
}

#[test]
fn an_authentic_object_that_names_another_address_is_refused() {
    // Sealed under address X, but the plaintext inside names Y: it authenticates under X and is
    // still refused.
    let x = id_1();
    let y = ArtifactId::from_derived([0x77; 32]);
    let k_bytes: [u8; 32] = std::array::from_fn(|i| i as u8);
    let key = spec_object_key(&k_bytes, 1, &x);
    let ct = raw_seal(
        &key,
        &artifact_aad(&VAULT, 1, &x),
        &sample(y).encode().unwrap(),
    );
    assert_eq!(
        open(&k1(), &x, &encode_container(&ct)),
        Err(OpenArtifactError::WrongAddress)
    );
}

#[test]
fn an_authentic_object_whose_descriptor_or_kind_is_wrong_is_refused() {
    let k_bytes: [u8; 32] = std::array::from_fn(|i| i as u8);
    let key = spec_object_key(&k_bytes, 1, &id_1());
    let aad = artifact_aad(&VAULT, 1, &id_1());
    let good = sample(id_1()).encode().unwrap();
    // A blob byte changed after the descriptor was written.
    let mut bad_digest = good.clone();
    let last = bad_digest.len() - 1;
    bad_digest[last] ^= 1;
    assert_eq!(
        open(
            &k1(),
            &id_1(),
            &encode_container(&raw_seal(&key, &aad, &bad_digest))
        ),
        Err(OpenArtifactError::Plaintext(
            ArtifactError::DescriptorDigest
        ))
    );
    // Reserved kinds 3 and 4.
    let kind_at = 2 + "dohflow/sync-artifact-transport/v1".len() + 32;
    for reserved in [3_u8, 4] {
        let mut kinded = good.clone();
        kinded[kind_at] = reserved;
        assert_eq!(
            open(
                &k1(),
                &id_1(),
                &encode_container(&raw_seal(&key, &aad, &kinded))
            ),
            Err(OpenArtifactError::Plaintext(ArtifactError::ReservedKind))
        );
    }
    // Not a transport plaintext at all.
    assert!(matches!(
        open(
            &k1(),
            &id_1(),
            &encode_container(&raw_seal(&key, &aad, b"not a transport plaintext"))
        ),
        Err(OpenArtifactError::Plaintext(_))
    ));
}

// ---- the fixed-plaintext guard -------------------------------------------------------------------

#[test]
fn sealing_is_deterministic_and_the_digest_guards_every_later_seal() {
    let first = seal_artifact(&k1(), &sample(id_1()), Fixity::First).unwrap();
    let again = seal_artifact(
        &k1(),
        &sample(id_1()),
        Fixity::Expected(first.plaintext_digest),
    )
    .unwrap();
    assert_eq!(
        first.container, again.container,
        "a retry rebuilds identical bytes"
    );
    assert_eq!(first.receipt, again.receipt);
    // A changed descriptor, a changed blob and a changed id are each refused before encryption.
    let digest = first.plaintext_digest;
    let changed_metadata = TransportPlaintext {
        logical_metadata: br#"{"mime_type":"application/pdf"}"#,
        ..sample(id_1())
    };
    let changed_blob = TransportPlaintext {
        blob: b"synthetic document!",
        ..sample(id_1())
    };
    let changed_id = sample(ArtifactId::from_derived([0x99; 32]));
    let changed_kind = TransportPlaintext {
        kind: ArtifactKind::SourceBatchPart,
        ..sample(id_1())
    };
    for changed in [changed_metadata, changed_blob, changed_id, changed_kind] {
        assert_eq!(
            seal_artifact(&k1(), &changed, Fixity::Expected(digest)).map(|_| ()),
            Err(SealArtifactError::ArtifactDescriptorChanged),
            "{changed:?}"
        );
    }
    // The guard compares the plaintext, not the key: the same plaintext still seals.
    assert!(seal_artifact(&k1(), &sample(id_1()), Fixity::Expected(digest)).is_ok());
}

#[test]
fn an_unencodable_plaintext_and_an_oversize_container_are_refused() {
    let bad_metadata = TransportPlaintext {
        logical_metadata: br#"{"b":"1","a":"2"}"#,
        ..sample(id_1())
    };
    assert_eq!(
        seal_artifact(&k1(), &bad_metadata, Fixity::First).map(|_| ()),
        Err(SealArtifactError::Format(
            ArtifactError::MetadataNotCanonical
        ))
    );
    // A blob that leaves no room for the framing, the metadata and the tag.
    let big = vec![0_u8; MAX_ARTIFACT_CONTAINER];
    let too_big = TransportPlaintext {
        blob: &big,
        ..sample(id_1())
    };
    assert_eq!(
        seal_artifact(&k1(), &too_big, Fixity::First).map(|_| ()),
        Err(SealArtifactError::ContainerTooLarge)
    );
}

// ---- re-addressing at a key-changing epoch ----------------------------------------------------------

#[test]
fn a_re_addressed_artifact_is_the_same_bytes_under_the_chained_address() {
    let old = sealed_container();
    let moved = readdress_container(&k1(), &k2(), &id_1(), &old, Fixity::First).unwrap();
    assert_eq!(
        moved.artifact_id.as_bytes().as_slice(),
        vector("artifact_id_readdress_epoch2")
    );
    // It opens under the new key at the new address and not under the old key.
    let opened = open_artifact(
        &k2(),
        &moved.artifact_id,
        &moved.sealed.container,
        MAX_ARTIFACT_CONTAINER,
    )
    .unwrap();
    assert_eq!(opened.plaintext().blob, DOCUMENT);
    assert_eq!(opened.plaintext().logical_metadata, METADATA);
    assert_eq!(opened.plaintext().artifact_id, moved.artifact_id);
    assert_eq!(
        open(&k1(), &moved.artifact_id, &moved.sealed.container),
        Err(OpenArtifactError::NotOpenable)
    );
    // The old object does not open at the new address, nor the new one at the old address.
    assert_eq!(
        open(&k2(), &id_1(), &moved.sealed.container),
        Err(OpenArtifactError::NotOpenable)
    );
    assert_eq!(
        open(&k2(), &moved.artifact_id, &old),
        Err(OpenArtifactError::NotOpenable)
    );
    // The ciphertext and the receipt are different objects; the plaintext digest differs only
    // because the id inside it does.
    assert_ne!(moved.sealed.container, old);
    assert_eq!(moved.sealed.receipt.artifact_id, moved.artifact_id);
    assert_eq!(moved.sealed.receipt.key_epoch, 2);
}

/// The digest stored at first publication in epoch 1: the SHA-256 of the complete transport plaintext.
fn stored_digest() -> [u8; 32] {
    sample(id_1()).digest().unwrap()
}

#[test]
fn the_local_copy_and_the_fetched_object_re_address_to_identical_bytes() {
    let old = sealed_container();
    let fetched = readdress_container(&k1(), &k2(), &id_1(), &old, Fixity::First).unwrap();
    // The local path: a materialized copy that matches the digest stored at first publication.
    let local = readdress_plaintext(
        &k1(),
        &k2(),
        &id_1(),
        &sample(id_1()),
        &stored_digest(),
        Fixity::First,
    )
    .unwrap();
    assert_eq!(local.artifact_id, fetched.artifact_id);
    assert_eq!(local.sealed.container, fetched.sealed.container);
    assert_eq!(
        local.sealed.plaintext_digest,
        fetched.sealed.plaintext_digest
    );
    // A retry, which now also holds the epoch-2 digest, rebuilds the same bytes.
    let retry = readdress_plaintext(
        &k1(),
        &k2(),
        &id_1(),
        &sample(id_1()),
        &stored_digest(),
        Fixity::Expected(fetched.sealed.plaintext_digest),
    )
    .unwrap();
    assert_eq!(retry.sealed.container, fetched.sealed.container);
}

#[test]
fn a_local_copy_that_is_not_what_was_published_is_refused_on_the_first_re_address() {
    // No epoch-2 digest exists yet (Fixity::First): the stored epoch-1 digest is the only guard.
    let first = |plaintext: &TransportPlaintext<'_>, id: &ArtifactId| {
        readdress_plaintext(&k1(), &k2(), id, plaintext, &stored_digest(), Fixity::First)
            .map(|_| ())
    };
    let differs = Err(ReaddressError::LocalCopyDiffers);
    // A drifted blob, a drifted descriptor, and a drifted kind.
    assert_eq!(
        first(
            &TransportPlaintext {
                blob: b"a different document",
                ..sample(id_1())
            },
            &id_1()
        ),
        differs
    );
    assert_eq!(
        first(
            &TransportPlaintext {
                logical_metadata: br#"{"mime_type":"application/pdf"}"#,
                ..sample(id_1())
            },
            &id_1()
        ),
        differs
    );
    assert_eq!(
        first(
            &TransportPlaintext {
                kind: ArtifactKind::SourceBatchPart,
                ..sample(id_1())
            },
            &id_1()
        ),
        differs
    );
    // A copy that names another address is refused even when everything else matches the digest of
    // *its own* plaintext: it is not the artifact at `old_id`.
    let other_id = ArtifactId::from_derived([0x77; 32]);
    assert_eq!(first(&sample(other_id), &id_1()), differs);
    // The address check stands on its own: a copy whose digest *is* the stored one but which names
    // another address (the stored digest here is that of the copy itself) is still refused.
    let other = sample(other_id);
    assert_eq!(
        readdress_plaintext(
            &k1(),
            &k2(),
            &id_1(),
            &other,
            &other.digest().unwrap(),
            Fixity::First
        )
        .map(|_| ()),
        differs
    );
    // A copy held at the right address but checked against the wrong stored digest is refused.
    assert_eq!(
        readdress_plaintext(
            &k1(),
            &k2(),
            &id_1(),
            &sample(id_1()),
            &[0u8; 32],
            Fixity::First
        )
        .map(|_| ()),
        differs
    );
    // And nothing was sealed on any refusal: the good copy still re-addresses.
    assert!(first(&sample(id_1()), &id_1()).is_ok());
}

#[test]
fn a_local_copy_that_drifted_after_the_epoch_two_seal_is_refused_by_the_fixity_too() {
    let sealed = readdress_plaintext(
        &k1(),
        &k2(),
        &id_1(),
        &sample(id_1()),
        &stored_digest(),
        Fixity::First,
    )
    .unwrap();
    // The stored epoch-1 digest still matches the copy, but the epoch-2 fixity guards the retry.
    assert_eq!(
        readdress_plaintext(
            &k1(),
            &k2(),
            &id_1(),
            &sample(id_1()),
            &stored_digest(),
            Fixity::Expected([9u8; 32])
        )
        .map(|_| ()),
        Err(ReaddressError::Seal(
            SealArtifactError::ArtifactDescriptorChanged
        ))
    );
    let _ = sealed;
}

#[test]
fn an_object_that_cannot_be_opened_is_not_re_addressed() {
    let mut flipped = sealed_container();
    let last = flipped.len() - 1;
    flipped[last] ^= 1;
    assert_eq!(
        readdress_container(&k1(), &k2(), &id_1(), &flipped, Fixity::First).map(|_| ()),
        Err(ReaddressError::Open(OpenArtifactError::NotOpenable))
    );
    // A container opened under the wrong old key is not re-addressed either.
    assert!(readdress_container(
        &key_of(VAULT, 1, 0x80),
        &k2(),
        &id_1(),
        &sealed_container(),
        Fixity::First
    )
    .is_err());
}

#[test]
fn only_the_next_epoch_of_the_same_vault_is_a_re_address() {
    let old = sealed_container();
    let skip = key_of(VAULT, 3, 0x40);
    let same = key_of(VAULT, 1, 0x40);
    let other_vault = key_of([0x12; 16], 2, 0x40);
    for bad in [&skip, &same, &other_vault] {
        assert_eq!(
            readdress_container(&k1(), bad, &id_1(), &old, Fixity::First).map(|_| ()),
            Err(ReaddressError::NotTheNextEpoch)
        );
        assert_eq!(
            readdress_plaintext(
                &k1(),
                bad,
                &id_1(),
                &sample(id_1()),
                &stored_digest(),
                Fixity::First
            )
            .map(|_| ()),
            Err(ReaddressError::NotTheNextEpoch)
        );
    }
    // Going backwards is not a re-address.
    assert_eq!(
        readdress_plaintext(
            &k2(),
            &k1(),
            &id_1(),
            &sample(id_1()),
            &stored_digest(),
            Fixity::First
        )
        .map(|_| ()),
        Err(ReaddressError::NotTheNextEpoch)
    );
}

#[test]
fn the_chain_is_unlinkable_without_the_new_key_and_never_revisits_an_id() {
    let first = id_1();
    let second = readdress_id(&k2(), &first);
    let third = readdress_id(&key_of(VAULT, 3, 0x80), &second);
    assert!(first != second && second != third && first != third);
    // A holder of the old key and the old id, not the new key, computes a different value.
    assert_ne!(readdress_id(&k1(), &first), second);
    // A different vault, epoch or previous id gives a different address.
    assert_ne!(readdress_id(&key_of([0x12; 16], 2, 0x40), &first), second);
    assert_ne!(readdress_id(&key_of(VAULT, 3, 0x40), &first), second);
    assert_ne!(
        readdress_id(&k2(), &ArtifactId::from_derived([0x55; 32])),
        second
    );
    // The first-publication id and the re-address id of the same epoch are not the same function.
    assert_ne!(first_artifact_id(&k2(), &STORAGE), second);
    // Other storage addresses give other first ids (no cross-artifact collision), and the same
    // plaintext in another vault is unlinkable.
    assert_ne!(first_artifact_id(&k1(), &[0x6b; 32]), first);
    assert_ne!(
        first_artifact_id(&key_of([0x12; 16], 1, 0x00), &STORAGE),
        first
    );
}

// ---- no secret in a Debug --------------------------------------------------------------------------------

#[test]
fn debug_forms_show_sizes_and_addresses_never_bytes() {
    let sealed = seal_artifact(&k1(), &sample(id_1()), Fixity::First).unwrap();
    let shown = format!("{sealed:?}");
    assert!(shown.contains("len"));
    let opened = open_artifact(&k1(), &id_1(), &sealed.container, MAX_ARTIFACT_CONTAINER).unwrap();
    let shown_open = format!("{opened:?} {:?}", opened.plaintext());
    assert!(!shown_open.contains("synthetic"));
    assert!(!shown_open.contains("text/plain"));
    assert_eq!(
        opened.plaintext().binding(),
        Err(ArtifactError::BadLogicalMetadata)
    );
    let _ = LogicalBinding::of(ArtifactKind::Attachment, METADATA);
}

#[test]
fn the_container_size_bound_is_exact() {
    // The framing is 14 bytes and the tag 16; find the blob length that makes the container exactly
    // the service's maximum.
    let overhead = sample(id_1()).encode().unwrap().len() - DOCUMENT.len();
    let at_bound = MAX_ARTIFACT_CONTAINER - 14 - 16 - overhead;
    let blob = vec![0x5a_u8; at_bound];
    let at = TransportPlaintext {
        blob: &blob,
        ..sample(id_1())
    };
    let sealed = seal_artifact(&k1(), &at, Fixity::First).unwrap();
    assert_eq!(sealed.container.len(), MAX_ARTIFACT_CONTAINER);
    drop(sealed);
    let blob = vec![0x5a_u8; at_bound + 1];
    let past = TransportPlaintext {
        blob: &blob,
        ..sample(id_1())
    };
    assert_eq!(
        seal_artifact(&k1(), &past, Fixity::First).map(|_| ()),
        Err(SealArtifactError::ContainerTooLarge)
    );
}

/// An attachment's first id names its logical id as well as its bytes (C.3, the attachment-address
/// amendment): C.9 seals `attachment_id` into the plaintext, so an address that named the bytes
/// alone would carry two plaintexts after a shred and a re-import.
#[test]
fn an_attachments_address_matches_the_spec_vectors_and_names_its_logical_id() {
    let (one, two) = ([0x07_u8; 16], [0x08_u8; 16]);
    assert_eq!(
        first_attachment_artifact_id(&k1(), &STORAGE, &one)
            .as_bytes()
            .as_slice(),
        vector("attachment_artifact_id_epoch1")
    );
    assert_eq!(
        first_attachment_artifact_id(&k1(), &STORAGE, &two)
            .as_bytes()
            .as_slice(),
        vector("attachment_artifact_id_epoch1_other_id")
    );
    // The same bytes under two attachment ids are two addresses, and neither is the bytes-only one
    // a batch part uses.
    let (a, b) = (
        first_attachment_artifact_id(&k1(), &STORAGE, &one),
        first_attachment_artifact_id(&k1(), &STORAGE, &two),
    );
    assert_ne!(a, b);
    assert_ne!(a, first_artifact_id(&k1(), &STORAGE));
    // Other bytes, another vault and another epoch's key are other addresses too.
    assert_ne!(first_attachment_artifact_id(&k1(), &[0x6b; 32], &one), a);
    assert_ne!(
        first_attachment_artifact_id(&key_of([0x12; 16], 1, 0x00), &STORAGE, &one),
        a
    );
    assert_ne!(first_attachment_artifact_id(&k2(), &STORAGE, &one), a);
}
