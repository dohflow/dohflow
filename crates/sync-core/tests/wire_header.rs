//! The clear header and container (vault-format.md C.4; personal-cfo-t2s4b part 6a), checked
//! against the C.14 byte vectors read from the spec at test time, and against the rejection
//! rules of C.1.1 and C.4: every parser rule has a test that mutates a known-good vector.

use sync_core::cbe::WireError;
use sync_core::header::{
    encode_container, parse_container, ClearHeader, Fingerprint, IdempotencyKeyTag,
    ManifestCommitment, MetadataCommitment, NonceDomain, ObjectKind, MAX_INVOCATION_COUNTER,
};

const SPEC: &str = include_str!("../../../docs/architecture/vault-format.md");

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

fn array<const N: usize>(bytes: &[u8]) -> [u8; N] {
    bytes.try_into().unwrap()
}

/// The sample envelope header of C.14, built from the input table and the derived vectors.
fn sample_envelope() -> ClearHeader {
    ClearHeader {
        vault_id: [0x11; 16],
        key_epoch: 1,
        object: ObjectKind::Envelope {
            idempotency_key_tag: IdempotencyKeyTag::from_derived(array(&vector("idempotency_tag"))),
            immutable_metadata_commitment: MetadataCommitment::from_derived(array(&vector(
                "immutable_metadata_commitment",
            ))),
            artifact_manifest_commitment: Some(ManifestCommitment::from_derived(array(&vector(
                "artifact_manifest_commitment",
            )))),
        },
        object_id: [
            0x01, 0x93, 0x8f, 0x2e, 0x7c, 0x1a, 0x7b, 0x3d, 0x9a, 0x4e, 0x12, 0x34, 0x56, 0x78,
            0x9a, 0xbc,
        ],
        device_id: [0xa1; 16],
        device_key_fingerprint: Fingerprint::from_derived(array(&vector("fingerprint_a"))),
        nonce_domain: NonceDomain::from_derived(array(&vector("nonce_domain_a_envelope"))),
        invocation_counter: 0,
        base_seq: 41,
        payload_schema_version: 1,
        engine_version: "dohflow-0.3.0".to_owned(),
        schema_version: 55,
    }
}

#[test]
fn the_envelope_header_matches_the_spec_vector_and_round_trips() {
    let expected = vector("header_envelope");
    let header = sample_envelope();
    assert_eq!(header.encode().unwrap(), expected, "encode");
    assert_eq!(ClearHeader::decode(&expected).unwrap(), header, "decode");
    assert_eq!(header.object_type(), 1);
}

#[test]
fn the_snapshot_header_matches_the_spec_vector_and_round_trips() {
    let bytes = vector("header_snapshot");
    let header = ClearHeader::decode(&bytes).expect("the spec's snapshot header parses");
    assert_eq!(header.object_type(), 2);
    assert_eq!(header.object_id, [0x33; 16]);
    assert_eq!(header.vault_id, [0x11; 16]);
    assert_eq!(header.device_id, [0xa1; 16]);
    assert_eq!(header.invocation_counter, 0);
    assert_eq!(header.base_seq, 41);
    assert_eq!(header.engine_version, "dohflow-0.3.0");
    assert_eq!(header.schema_version, 55);
    // A snapshot carries neither dual-identity field and always carries its manifest commitment.
    assert!(matches!(header.object, ObjectKind::Snapshot { .. }));
    assert_eq!(
        header.encode().unwrap(),
        bytes,
        "re-encoding is byte-identical"
    );
}

#[test]
fn the_container_matches_the_spec_vector() {
    let bytes = vector("envelope_container");
    let parsed = parse_container(&bytes).expect("the spec's container parses");
    assert_eq!(parsed.header, sample_envelope());
    assert_eq!(parsed.header_bytes, vector("header_envelope"));
    assert_eq!(parsed.ciphertext, vector("envelope_ciphertext"));
    assert_eq!(
        encode_container(&parsed.header, parsed.ciphertext).unwrap(),
        bytes,
        "re-encoding is byte-identical"
    );
}

#[test]
fn the_aad_is_the_header_bytes_and_excludes_the_server_seq() {
    // There is no `seq` anywhere in the header: it does not exist until after the service has
    // accepted the sealed object. Two headers equal in every field are equal in bytes, and the
    // type has no field a later `seq` could be written into.
    let header = sample_envelope();
    let again = sample_envelope();
    assert_eq!(header.encode().unwrap(), again.encode().unwrap());
    let container = vector("envelope_container");
    let parsed = parse_container(&container).unwrap();
    assert_eq!(parsed.header_bytes, header.encode().unwrap());
}

// ---------------------------------------------------------------------------
// Rejections: every one fails closed with a typed error and returns nothing.
// ---------------------------------------------------------------------------

fn header_bytes() -> Vec<u8> {
    vector("header_envelope")
}

#[test]
fn every_proper_prefix_of_a_header_is_refused() {
    let bytes = header_bytes();
    for len in 0..bytes.len() {
        let error = ClearHeader::decode(&bytes[..len]).expect_err("a prefix must not parse");
        assert!(error.is_malformed(), "len {len}: {error:?}");
    }
}

#[test]
fn trailing_bytes_after_the_header_are_refused() {
    let mut bytes = header_bytes();
    bytes.push(0);
    assert_eq!(ClearHeader::decode(&bytes), Err(WireError::TrailingBytes));
}

#[test]
fn a_wrong_or_malformed_label_is_refused() {
    let mut bytes = header_bytes();
    // "dohflow/sync-header/v1": flip the final '1' to '2' (still a valid label, not ours).
    let at = 2 + "dohflow/sync-header/v1".len() - 1;
    bytes[at] = b'2';
    assert_eq!(ClearHeader::decode(&bytes), Err(WireError::LabelMismatch));
    // An uppercase letter is outside [a-z0-9/_.-].
    let mut bytes = header_bytes();
    bytes[2] = b'D';
    assert_eq!(ClearHeader::decode(&bytes), Err(WireError::BadLabel));
    // A zero-length label.
    let mut bytes = header_bytes();
    bytes[0] = 0;
    bytes[1] = 0;
    assert_eq!(ClearHeader::decode(&bytes), Err(WireError::BadLabel));
}

#[test]
fn an_unsupported_protocol_version_is_refused_before_anything_else_is_trusted() {
    let mut bytes = header_bytes();
    let at = 2 + "dohflow/sync-header/v1".len(); // the u16 right after the label
    bytes[at] = 0;
    bytes[at + 1] = 2;
    let error = ClearHeader::decode(&bytes).unwrap_err();
    assert_eq!(error, WireError::UnsupportedProtocol);
    assert!(!error.is_malformed());
}

fn offsets() -> (usize, usize) {
    // After label (2 + 22), protocol (2), vault (16), epoch (8): the object_type byte.
    let object_type = 2 + "dohflow/sync-header/v1".len() + 2 + 16 + 8;
    (object_type, object_type + 1 + 16) // and the tag's presence byte
}

#[test]
fn an_unknown_object_type_is_refused() {
    let (object_type, _) = offsets();
    for value in [0_u8, 3, 255] {
        let mut bytes = header_bytes();
        bytes[object_type] = value;
        assert_eq!(
            ClearHeader::decode(&bytes),
            Err(WireError::UnknownObjectType),
            "type {value}"
        );
    }
}

#[test]
fn a_presence_byte_other_than_zero_or_one_is_refused() {
    let (_, tag_presence) = offsets();
    for value in [2_u8, 0x80, 255] {
        let mut bytes = header_bytes();
        bytes[tag_presence] = value;
        assert_eq!(ClearHeader::decode(&bytes), Err(WireError::BadPresence));
    }
}

#[test]
fn the_dual_identity_fields_must_match_the_object_type() {
    // An envelope whose type byte says snapshot, with its tag and commitment present.
    let (object_type, _) = offsets();
    let mut bytes = header_bytes();
    bytes[object_type] = 2;
    assert_eq!(
        ClearHeader::decode(&bytes),
        Err(WireError::ObjectFieldsMismatch)
    );
    // A snapshot whose type byte says envelope, with both fields absent.
    let mut bytes = vector("header_snapshot");
    bytes[object_type] = 1;
    assert_eq!(
        ClearHeader::decode(&bytes),
        Err(WireError::ObjectFieldsMismatch)
    );
}

#[test]
fn a_snapshot_without_a_manifest_commitment_is_refused() {
    let mut header = ClearHeader::decode(&vector("header_snapshot")).unwrap();
    let ObjectKind::Snapshot { .. } = header.object else {
        panic!()
    };
    // Re-encode by hand with the final opt() absent: presence byte 0 and no 32 bytes.
    let mut bytes = header.encode().unwrap();
    let new_len = bytes.len() - 33;
    bytes.truncate(new_len);
    bytes.push(0);
    assert_eq!(
        ClearHeader::decode(&bytes),
        Err(WireError::SnapshotWithoutManifest)
    );
    // An envelope may omit it (an empty delta manifest), and round-trips.
    header = sample_envelope();
    if let ObjectKind::Envelope {
        artifact_manifest_commitment,
        ..
    } = &mut header.object
    {
        *artifact_manifest_commitment = None;
    }
    let bytes = header.encode().unwrap();
    assert_eq!(ClearHeader::decode(&bytes).unwrap(), header);
}

#[test]
fn the_invocation_counter_cap_is_enforced_on_encode_and_decode() {
    let mut header = sample_envelope();
    header.invocation_counter = MAX_INVOCATION_COUNTER;
    let at_cap = header.encode().unwrap();
    assert_eq!(ClearHeader::decode(&at_cap).unwrap(), header);

    header.invocation_counter = MAX_INVOCATION_COUNTER + 1;
    assert_eq!(header.encode(), Err(WireError::CounterAboveCap));

    // Decode: patch the on-wire counter of a good header to 2^32.
    let mut bytes = header_bytes();
    // Locate the counter by re-encoding with a marker value instead of hand-counting offsets.
    let mut probe = sample_envelope();
    probe.invocation_counter = 0x0102_0304;
    let probe_bytes = probe.encode().unwrap();
    let marker = 0x0102_0304_u64.to_be_bytes();
    let position = probe_bytes
        .windows(8)
        .position(|w| w == marker)
        .expect("counter bytes");
    bytes[position..position + 8].copy_from_slice(&(1_u64 << 32).to_be_bytes());
    assert_eq!(ClearHeader::decode(&bytes), Err(WireError::CounterAboveCap));
}

#[test]
fn engine_version_is_bounded_at_sixty_four_bytes_and_must_be_utf8() {
    let mut header = sample_envelope();
    header.engine_version = "e".repeat(64);
    assert_eq!(
        ClearHeader::decode(&header.encode().unwrap()).unwrap(),
        header
    );
    header.engine_version = "e".repeat(65);
    assert_eq!(header.encode(), Err(WireError::TooLong));
    // Multi-byte characters count bytes, not characters.
    header.engine_version = "\u{e9}".repeat(33); // 66 bytes
    assert_eq!(header.encode(), Err(WireError::TooLong));

    // On the wire: an oversize length, then invalid UTF-8.
    let marker = b"dohflow-0.3.0";
    let bytes = header_bytes();
    let at = bytes
        .windows(marker.len())
        .position(|w| w == marker)
        .unwrap();
    let mut oversize = bytes.clone();
    oversize[at - 4..at].copy_from_slice(&65_u32.to_be_bytes());
    assert_eq!(ClearHeader::decode(&oversize), Err(WireError::TooLong));
    let mut bad_utf8 = bytes;
    bad_utf8[at] = 0xff;
    assert_eq!(ClearHeader::decode(&bad_utf8), Err(WireError::InvalidUtf8));
}

#[test]
fn the_empty_engine_version_follows_the_spec_text_64_rule() {
    // C.4 types the field text(64), with no minimum: the codec follows the spec exactly. (The
    // operation log refuses an empty engine_version; whether the wire should is a spec change.)
    let mut header = sample_envelope();
    header.engine_version = String::new();
    assert_eq!(
        ClearHeader::decode(&header.encode().unwrap()).unwrap(),
        header
    );
}

// ---------------------------------------------------------------------------
// Container rules
// ---------------------------------------------------------------------------

#[test]
fn every_proper_prefix_of_a_container_is_refused() {
    let bytes = vector("envelope_container");
    for len in 0..bytes.len() {
        assert!(
            parse_container(&bytes[..len]).is_err(),
            "a {len}-byte prefix must not parse"
        );
    }
}

#[test]
fn a_container_rejects_bad_magic_version_and_trailing_bytes() {
    let good = vector("envelope_container");

    let mut bad_magic = good.clone();
    bad_magic[0] = b'X';
    assert_eq!(parse_container(&bad_magic), Err(WireError::BadMagic));

    let mut bad_version = good.clone();
    bad_version[4] = 0;
    bad_version[5] = 2;
    let error = parse_container(&bad_version).unwrap_err();
    assert_eq!(error, WireError::UnsupportedContainer);
    assert!(!error.is_malformed());

    let mut trailing = good;
    trailing.push(0);
    assert_eq!(parse_container(&trailing), Err(WireError::TrailingBytes));
}

#[test]
fn lengths_that_claim_more_than_the_input_holds_are_refused_before_any_allocation() {
    let good = vector("envelope_container");
    // header_len = u32::MAX
    let mut huge_header = good.clone();
    huge_header[6..10].copy_from_slice(&u32::MAX.to_be_bytes());
    assert_eq!(
        parse_container(&huge_header),
        Err(WireError::LengthExceedsInput)
    );

    // ciphertext_len = u64::MAX
    let header_len = u32::from_be_bytes(good[6..10].try_into().unwrap()) as usize;
    let ct_len_at = 10 + header_len;
    let mut huge_ciphertext = good.clone();
    huge_ciphertext[ct_len_at..ct_len_at + 8].copy_from_slice(&u64::MAX.to_be_bytes());
    assert_eq!(
        parse_container(&huge_ciphertext),
        Err(WireError::LengthExceedsInput)
    );

    // A header_len shorter than the header: the header parser must consume exactly its bytes.
    let mut short = good;
    short[6..10].copy_from_slice(&((header_len - 1) as u32).to_be_bytes());
    assert!(parse_container(&short).is_err());
}

#[test]
fn a_header_decoded_from_a_container_equals_its_own_canonical_encoding() {
    // One encoding per value: for any header that parses, its bytes are exactly its encoding.
    for name in ["header_envelope", "header_snapshot"] {
        let bytes = vector(name);
        let header = ClearHeader::decode(&bytes).unwrap();
        assert_eq!(header.encode().unwrap(), bytes, "{name}");
    }
}

#[test]
fn a_ciphertext_shorter_than_a_gcm_tag_is_refused() {
    let good = vector("envelope_container");
    let header_len = u32::from_be_bytes(good[6..10].try_into().unwrap()) as usize;
    let ct_len_at = 10 + header_len;
    for length in [0_usize, 1, 15] {
        let mut short = good[..ct_len_at].to_vec();
        short.extend_from_slice(&(length as u64).to_be_bytes());
        short.extend(std::iter::repeat_n(0xAA, length));
        assert_eq!(
            parse_container(&short),
            Err(WireError::CiphertextTooShort),
            "{length}-byte ciphertext"
        );
    }
    // Exactly one tag's worth is the smallest accepted.
    let mut least = good[..ct_len_at].to_vec();
    least.extend_from_slice(&16_u64.to_be_bytes());
    least.extend_from_slice(&[0xAA; 16]);
    assert_eq!(parse_container(&least).unwrap().ciphertext.len(), 16);
}
