//! The class-1b artifact formats (vault-format.md C.9; personal-cfo-0g528.2 S2b): the transport
//! plaintext and its descriptor, the logical binding, the `DFSA` container, the receipt, and the two
//! closure bounds.

use sha2::{Digest, Sha256};
use sync_core::artifact::{
    artifact_aad, encode_container, parse_container, ArtifactError, ArtifactId, ArtifactKind,
    ArtifactReceipt, ClosureManifest, LogicalBinding, TransportPlaintext, MAX_ARTIFACT_CONTAINER,
    MAX_LOGICAL_METADATA, MAX_SNAPSHOT_CLOSURE,
};
use sync_core::cbe::WireError;
use sync_core::envelope::ArtifactManifest;

const ID: ArtifactId = ArtifactId::from_derived([0x42; 32]);
const BATCH: &str = "01938f2e-7c1a-7b3d-9a4e-123456789abc";
const BATCH_BYTES: [u8; 16] = [
    0x01, 0x93, 0x8f, 0x2e, 0x7c, 0x1a, 0x7b, 0x3d, 0x9a, 0x4e, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc,
];

fn meta_attachment() -> Vec<u8> {
    format!(r#"{{"attachment_id":"{BATCH}","mime_type":"text/plain"}}"#).into_bytes()
}

fn meta_part(index: u32, count: u32) -> Vec<u8> {
    format!(r#"{{"batch_id":"{BATCH}","part_count":"{count}","part_index":"{index}"}}"#)
        .into_bytes()
}

fn plaintext<'a>(metadata: &'a [u8], blob: &'a [u8]) -> TransportPlaintext<'a> {
    TransportPlaintext {
        artifact_id: ID,
        kind: ArtifactKind::Attachment,
        logical_metadata: metadata,
        blob,
    }
}

// ---- the transport plaintext --------------------------------------------------------------------

#[test]
fn a_transport_plaintext_round_trips_and_is_deterministic() {
    let meta = meta_attachment();
    let p = plaintext(&meta, b"synthetic document");
    let bytes = p.encode().unwrap();
    assert_eq!(bytes, p.encode().unwrap());
    assert_eq!(TransportPlaintext::decode(&bytes).unwrap(), p);
    assert_eq!(
        p.digest().unwrap(),
        <[u8; 32]>::from(Sha256::digest(&bytes))
    );
    // The descriptor is derived from the blob, so a different blob is a different plaintext.
    let other = plaintext(&meta, b"another document");
    assert_ne!(other.digest().unwrap(), p.digest().unwrap());
}

#[test]
fn the_kinds_are_the_registry_and_three_and_four_are_reserved() {
    assert_eq!(ArtifactKind::from_code(1), Ok(ArtifactKind::Attachment));
    assert_eq!(
        ArtifactKind::from_code(2),
        Ok(ArtifactKind::SourceBatchPart)
    );
    for reserved in [3, 4] {
        assert_eq!(
            ArtifactKind::from_code(reserved),
            Err(ArtifactError::ReservedKind)
        );
    }
    for unknown in [0, 5, 255] {
        assert_eq!(
            ArtifactKind::from_code(unknown),
            Err(ArtifactError::UnknownKind)
        );
    }
    // A plaintext that carries kind 3 or 4 does not decode, even if everything else is valid.
    let meta = meta_attachment();
    let mut bytes = plaintext(&meta, b"x").encode().unwrap();
    let kind_at = 2 + "dohflow/sync-artifact-transport/v1".len() + 32;
    for reserved in [3_u8, 4] {
        bytes[kind_at] = reserved;
        assert_eq!(
            TransportPlaintext::decode(&bytes),
            Err(ArtifactError::ReservedKind)
        );
    }
}

#[test]
fn a_descriptor_that_disagrees_with_its_blob_is_refused() {
    let meta = meta_attachment();
    let good = plaintext(&meta, b"synthetic document").encode().unwrap();
    let length_at = 2 + "dohflow/sync-artifact-transport/v1".len() + 32 + 1;
    let digest_at = length_at + 8;
    // A wrong plaintext_length.
    let mut wrong_length = good.clone();
    wrong_length[length_at + 7] ^= 1;
    assert_eq!(
        TransportPlaintext::decode(&wrong_length),
        Err(ArtifactError::DescriptorLength)
    );
    // A wrong plaintext_digest.
    let mut wrong_digest = good.clone();
    wrong_digest[digest_at] ^= 1;
    assert_eq!(
        TransportPlaintext::decode(&wrong_digest),
        Err(ArtifactError::DescriptorDigest)
    );
    // A changed blob byte with the descriptor left alone.
    let mut wrong_blob = good.clone();
    let last = wrong_blob.len() - 1;
    wrong_blob[last] ^= 1;
    assert_eq!(
        TransportPlaintext::decode(&wrong_blob),
        Err(ArtifactError::DescriptorDigest)
    );
}

#[test]
fn a_malformed_plaintext_is_refused_with_its_wire_error() {
    let meta = meta_attachment();
    let good = plaintext(&meta, b"synthetic document").encode().unwrap();
    assert_eq!(
        TransportPlaintext::decode(&good[..good.len() - 1]),
        Err(WireError::LengthExceedsInput.into())
    );
    let mut trailing = good.clone();
    trailing.push(0);
    assert_eq!(
        TransportPlaintext::decode(&trailing),
        Err(WireError::TrailingBytes.into())
    );
    let mut label = good.clone();
    label[5] ^= 1;
    assert!(TransportPlaintext::decode(&label).is_err());
    assert!(TransportPlaintext::decode(&[]).is_err());
}

#[test]
fn logical_metadata_must_be_a_canonical_json_object_within_its_bound() {
    for bad in [
        &br#"{"b":"1","a":"2"}"#[..], // not sorted
        &br#"["a"]"#[..],             // not an object
        &br#"{"a": "1"}"#[..],        // whitespace
        &br#"{"a":1}"#[..],           // a number
        &b""[..],
    ] {
        let p = plaintext(bad, b"x");
        assert_eq!(
            p.encode(),
            Err(ArtifactError::MetadataNotCanonical),
            "{bad:?}"
        );
    }
    let too_long = vec![b' '; MAX_LOGICAL_METADATA + 1];
    assert_eq!(
        plaintext(&too_long, b"x").encode(),
        Err(WireError::TooLong.into())
    );
    // A decoder refuses what an encoder would not write.
    let meta = meta_attachment();
    let mut bytes = plaintext(&meta, b"x").encode().unwrap();
    let at = bytes
        .windows(meta.len())
        .position(|w| w == meta.as_slice())
        .unwrap();
    bytes.swap(at + 1, at + 2); // unsorted/garbled member name
    assert!(TransportPlaintext::decode(&bytes).is_err());
}

#[test]
fn debug_shows_sizes_and_never_content() {
    let meta = meta_attachment();
    let p = plaintext(&meta, b"SENTINEL-document-bytes");
    let shown = format!("{p:?}");
    assert!(shown.contains("blob_len"));
    assert!(!shown.contains("SENTINEL"));
    assert!(!shown.contains("text/plain"));
}

// ---- the logical binding ---------------------------------------------------------------------

#[test]
fn an_attachment_binds_its_attachment_id() {
    assert_eq!(
        LogicalBinding::of(ArtifactKind::Attachment, &meta_attachment()),
        Ok(LogicalBinding::Attachment {
            attachment_id: BATCH_BYTES
        })
    );
    let meta = meta_attachment();
    let p = plaintext(&meta, b"x");
    assert_eq!(
        p.binding(),
        Ok(LogicalBinding::Attachment {
            attachment_id: BATCH_BYTES
        })
    );
}

#[test]
fn a_batch_part_binds_its_batch_index_and_count() {
    assert_eq!(
        LogicalBinding::of(ArtifactKind::SourceBatchPart, &meta_part(2, 5)),
        Ok(LogicalBinding::BatchPart {
            batch_id: BATCH_BYTES,
            part_index: 2,
            part_count: 5
        })
    );
    assert_eq!(
        LogicalBinding::of(ArtifactKind::SourceBatchPart, &meta_part(0, 1)),
        Ok(LogicalBinding::BatchPart {
            batch_id: BATCH_BYTES,
            part_index: 0,
            part_count: 1
        })
    );
}

#[test]
fn a_binding_that_is_missing_or_malformed_is_refused() {
    let bad = ArtifactError::BadLogicalMetadata;
    // Missing members.
    assert_eq!(
        LogicalBinding::of(ArtifactKind::Attachment, br#"{"mime_type":"text/plain"}"#),
        Err(bad)
    );
    assert_eq!(
        LogicalBinding::of(
            ArtifactKind::SourceBatchPart,
            format!(r#"{{"batch_id":"{BATCH}","part_count":"3"}}"#).as_bytes()
        ),
        Err(bad)
    );
    // An index at or above the count, a zero count, a count over the closure bound.
    assert_eq!(
        LogicalBinding::of(ArtifactKind::SourceBatchPart, &meta_part(3, 3)),
        Err(bad)
    );
    assert_eq!(
        LogicalBinding::of(ArtifactKind::SourceBatchPart, &meta_part(0, 0)),
        Err(bad)
    );
    assert_eq!(
        LogicalBinding::of(ArtifactKind::SourceBatchPart, &meta_part(0, 1_048_577)),
        Err(bad)
    );
    // The edge of the bound is a count, not an error.
    assert!(LogicalBinding::of(ArtifactKind::SourceBatchPart, &meta_part(0, 1_048_576)).is_ok());
    // Integers are decimal strings without leading zeros, a sign or a space.
    for count in ["01", "+1", "-1", " 1", "1 ", "1.0", ""] {
        let meta = format!(r#"{{"batch_id":"{BATCH}","part_count":"{count}","part_index":"0"}}"#);
        assert_eq!(
            LogicalBinding::of(ArtifactKind::SourceBatchPart, meta.as_bytes()),
            Err(bad),
            "{count:?}"
        );
    }
    // A uuid is lowercase and hyphenated, 36 characters.
    for id in [
        "01938F2E-7C1A-7B3D-9A4E-123456789ABC",
        "01938f2e7c1a7b3d9a4e123456789abc",
        "01938f2e-7c1a-7b3d-9a4e-123456789ab",
        "01938f2e-7c1a-7b3d-9a4e-123456789abg",
        "",
    ] {
        let meta = format!(r#"{{"attachment_id":"{id}"}}"#);
        assert_eq!(
            LogicalBinding::of(ArtifactKind::Attachment, meta.as_bytes()),
            Err(bad),
            "{id:?}"
        );
    }
    // Not an object at all.
    assert_eq!(
        LogicalBinding::of(ArtifactKind::Attachment, b"[]"),
        Err(ArtifactError::MetadataNotCanonical)
    );
}

// ---- the container ---------------------------------------------------------------------------

#[test]
fn a_container_round_trips_and_every_framing_defect_is_refused() {
    let ciphertext = vec![7_u8; 64];
    let bytes = encode_container(&ciphertext);
    assert_eq!(&bytes[..4], b"DFSA");
    assert_eq!(
        parse_container(&bytes, MAX_ARTIFACT_CONTAINER),
        Ok(&ciphertext[..])
    );
    // Truncated, with trailing bytes, a bad magic, another version, a length that lies.
    assert!(parse_container(&bytes[..bytes.len() - 1], MAX_ARTIFACT_CONTAINER).is_err());
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert_eq!(
        parse_container(&trailing, MAX_ARTIFACT_CONTAINER),
        Err(WireError::TrailingBytes.into())
    );
    let mut magic = bytes.clone();
    magic[3] = b'Y';
    assert_eq!(
        parse_container(&magic, MAX_ARTIFACT_CONTAINER),
        Err(WireError::BadMagic.into())
    );
    let mut version = bytes.clone();
    version[5] = 2;
    assert_eq!(
        parse_container(&version, MAX_ARTIFACT_CONTAINER),
        Err(WireError::UnsupportedContainer.into())
    );
    let mut length = bytes.clone();
    length[13] = 200;
    assert!(parse_container(&length, MAX_ARTIFACT_CONTAINER).is_err());
    assert!(parse_container(&[], MAX_ARTIFACT_CONTAINER).is_err());
    assert!(parse_container(b"DFS", MAX_ARTIFACT_CONTAINER).is_err());
}

#[test]
fn a_ciphertext_shorter_than_a_tag_and_an_oversize_container_are_refused() {
    let short = encode_container(&[0_u8; 15]);
    assert_eq!(
        parse_container(&short, MAX_ARTIFACT_CONTAINER),
        Err(WireError::CiphertextTooShort.into())
    );
    assert!(parse_container(&encode_container(&[0_u8; 16]), MAX_ARTIFACT_CONTAINER).is_ok());
    // The size cap is checked before anything is parsed.
    let big = encode_container(&[0_u8; 64]);
    assert_eq!(
        parse_container(&big, big.len() - 1),
        Err(ArtifactError::ContainerTooLarge)
    );
    assert!(parse_container(&big, big.len()).is_ok());
}

// ---- the receipt and the AAD ---------------------------------------------------------------------

#[test]
fn a_receipt_binds_the_address_to_the_ciphertext_and_round_trips() {
    let ciphertext = vec![9_u8; 40];
    let receipt = ArtifactReceipt::for_ciphertext([0x11; 16], 3, ID, &ciphertext);
    assert_eq!(receipt.ciphertext_length, 40);
    assert_eq!(
        receipt.ciphertext_sha256,
        <[u8; 32]>::from(Sha256::digest(&ciphertext))
    );
    let bytes = receipt.encode();
    assert_eq!(ArtifactReceipt::decode(&bytes), Ok(receipt));
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(ArtifactReceipt::decode(&trailing).is_err());
    assert!(ArtifactReceipt::decode(&bytes[..bytes.len() - 1]).is_err());
    // Every field is part of it.
    for other in [
        ArtifactReceipt::for_ciphertext([0x12; 16], 3, ID, &ciphertext),
        ArtifactReceipt::for_ciphertext([0x11; 16], 4, ID, &ciphertext),
        ArtifactReceipt::for_ciphertext(
            [0x11; 16],
            3,
            ArtifactId::from_derived([0; 32]),
            &ciphertext,
        ),
        ArtifactReceipt::for_ciphertext([0x11; 16], 3, ID, &[9_u8; 41]),
        ArtifactReceipt::for_ciphertext([0x11; 16], 3, ID, &[8_u8; 40]),
    ] {
        assert_ne!(other, receipt);
        assert_ne!(other.encode(), bytes);
    }
}

#[test]
fn the_aad_binds_the_vault_the_epoch_and_the_address() {
    let base = artifact_aad(&[0x11; 16], 1, &ID);
    assert_ne!(base, artifact_aad(&[0x12; 16], 1, &ID));
    assert_ne!(base, artifact_aad(&[0x11; 16], 2, &ID));
    assert_ne!(
        base,
        artifact_aad(&[0x11; 16], 1, &ArtifactId::from_derived([0x43; 32]))
    );
}

// ---- the closure bounds --------------------------------------------------------------------------

fn ids(n: usize) -> Vec<[u8; 32]> {
    (0..n)
        .map(|i| {
            let mut id = [0_u8; 32];
            id[..8].copy_from_slice(&(i as u64).to_be_bytes());
            id
        })
        .collect()
}

#[test]
fn an_envelope_delta_holds_4096_ids_and_a_snapshot_closure_2_to_the_20() {
    // The delta edge: 4096 is fine, 4097 is not.
    assert!(ArtifactManifest::new(ids(4096)).is_ok());
    assert_eq!(ArtifactManifest::new(ids(4097)), Err(WireError::TooLong));
    // The closure edge: 2^20 is fine, 2^20 + 1 is not, and 4097 is fine for a closure.
    assert!(ClosureManifest::new(ids(4097)).is_ok());
    assert!(ClosureManifest::new(ids(MAX_SNAPSHOT_CLOSURE)).is_ok());
    assert_eq!(
        ClosureManifest::new(ids(MAX_SNAPSHOT_CLOSURE + 1)),
        Err(ArtifactError::BadClosure)
    );
    // A duplicate is refused by both.
    let mut dup = ids(3);
    dup.push(dup[0]);
    assert_eq!(
        ClosureManifest::new(dup.clone()),
        Err(ArtifactError::BadClosure)
    );
    assert!(ArtifactManifest::new(dup).is_err());
}

#[test]
fn the_closure_commitment_is_the_delta_commitment_for_the_same_list_and_orders_its_ids() {
    // Same label and bytes: only the bound differs (C.3).
    let list = ids(300);
    assert_eq!(
        ClosureManifest::new(list.clone()).unwrap().commitment(),
        ArtifactManifest::new(list.clone()).unwrap().commitment()
    );
    let mut reversed = list.clone();
    reversed.reverse();
    assert_eq!(
        ClosureManifest::new(reversed).unwrap().commitment(),
        ClosureManifest::new(list.clone()).unwrap().commitment(),
        "the commitment is over the ascending order"
    );
    assert_ne!(
        ClosureManifest::new(ids(299)).unwrap().commitment(),
        ClosureManifest::new(list).unwrap().commitment()
    );
    // The empty closure is committed to as well, and a closure past the delta bound commits.
    assert_ne!(
        ClosureManifest::new(Vec::new()).unwrap().commitment(),
        ClosureManifest::new(ids(1)).unwrap().commitment()
    );
    let big = ClosureManifest::new(ids(MAX_SNAPSHOT_CLOSURE)).unwrap();
    assert_eq!(big.ids().len(), MAX_SNAPSHOT_CLOSURE);
    let _ = big.commitment();
}
