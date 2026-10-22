//! The envelope plaintext, its commitments and digest, and the receiver checks
//! (vault-format.md C.3, C.4, C.5; personal-cfo-t2s4b part 6b), against the C.14 vectors read
//! from the spec, plus a rejection for every rule.

use sync_core::cbe::WireError;
use sync_core::envelope::{
    canonical_envelope_digest, check_envelope, ArtifactManifest, EnvelopePlaintext, IdempotencyKey,
    ImmutableMeta, InvalidIdempotencyKey, ReceiverError,
};
use sync_core::header::{
    ClearHeader, Fingerprint, IdempotencyKeyTag, ManifestCommitment, MetadataCommitment,
    NonceDomain, ObjectKind,
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

const KEY_TEXT: &str = "4a3b2c1d-9e8f-4a7b-8c6d-5e4f3a2b1c0d";

fn sample_meta() -> ImmutableMeta {
    ImmutableMeta {
        command_id: array(&[
            0x01, 0x93, 0x8f, 0x2e, 0x7c, 0x1a, 0x7b, 0x3d, 0x9a, 0x4e, 0x12, 0x34, 0x56, 0x78,
            0x9a, 0xbc,
        ]),
        correlation_id: [0x22; 16],
        causation_id: None,
        actor_type: 1,
        actor_id: "device-a-user".to_owned(),
        idempotency_key: IdempotencyKey::parse(KEY_TEXT).unwrap(),
        issued_at: 1_790_000_000_000_000,
        device_id: [0xa1; 16],
    }
}

fn spec_plaintext() -> EnvelopePlaintext {
    EnvelopePlaintext::decode(&vector("envelope_plaintext")).expect("the spec plaintext parses")
}

fn spec_header() -> ClearHeader {
    ClearHeader::decode(&vector("header_envelope")).unwrap()
}

fn spec_tag() -> IdempotencyKeyTag {
    IdempotencyKeyTag::from_derived(array(&vector("idempotency_tag")))
}

// ---- idempotency keys (C.5, AC13) ---------------------------------------------------------

#[test]
fn the_idempotency_key_is_a_lowercase_hyphenated_uuid_version_four() {
    let key = IdempotencyKey::parse(KEY_TEXT).unwrap();
    assert_eq!(
        key.as_bytes(),
        &[
            0x4a, 0x3b, 0x2c, 0x1d, 0x9e, 0x8f, 0x4a, 0x7b, 0x8c, 0x6d, 0x5e, 0x4f, 0x3a, 0x2b,
            0x1c, 0x0d
        ]
    );
    assert_eq!(IdempotencyKey::from_bytes(*key.as_bytes()), Ok(key));
}

#[test]
fn every_other_idempotency_key_form_is_refused_with_the_typed_error() {
    let refused = [
        // a UUID version 7 (the command_id form)
        "01938f2e-7c1a-7b3d-9a4e-123456789abc",
        // a prefixed string
        "idem-4a3b2c1d-9e8f-4a7b-8c6d-5e4f3a2b1c0d",
        // not a UUID at all
        "not-a-uuid",
        "",
        // one text form per key: no uppercase, braces, or missing hyphens
        "4A3B2C1D-9E8F-4A7B-8C6D-5E4F3A2B1C0D",
        "{4a3b2c1d-9e8f-4a7b-8c6d-5e4f3a2b1c0d}",
        "4a3b2c1d9e8f4a7b8c6d5e4f3a2b1c0d",
        // a wrong RFC variant (the high bits of byte 8 are not 10)
        "4a3b2c1d-9e8f-4a7b-cc6d-5e4f3a2b1c0d",
        // version nibble 1, 3, 5
        "4a3b2c1d-9e8f-1a7b-8c6d-5e4f3a2b1c0d",
        "4a3b2c1d-9e8f-5a7b-8c6d-5e4f3a2b1c0d",
        // a good key with a trailing character
        "4a3b2c1d-9e8f-4a7b-8c6d-5e4f3a2b1c0dx",
        // non-hex
        "4a3b2c1d-9e8f-4a7b-8c6d-5e4f3a2b1c0g",
    ];
    for text in refused {
        assert_eq!(
            IdempotencyKey::parse(text),
            Err(InvalidIdempotencyKey),
            "{text:?}"
        );
    }
    // From raw bytes: version 7 and a wrong variant.
    let mut v7 = [0x11_u8; 16];
    v7[6] = 0x71;
    v7[8] = 0x80;
    assert_eq!(IdempotencyKey::from_bytes(v7), Err(InvalidIdempotencyKey));
    let mut bad_variant = [0x11_u8; 16];
    bad_variant[6] = 0x41;
    bad_variant[8] = 0xc0;
    assert_eq!(
        IdempotencyKey::from_bytes(bad_variant),
        Err(InvalidIdempotencyKey)
    );
    assert_eq!(
        format!("{:?}", IdempotencyKey::parse(KEY_TEXT).unwrap()),
        "IdempotencyKey(..)"
    );
}

// ---- derivations against the spec vectors -----------------------------------------------

#[test]
fn the_metadata_commitment_matches_the_spec_vector() {
    assert_eq!(
        sample_meta().commitment().unwrap().as_bytes().as_slice(),
        vector("immutable_metadata_commitment")
    );
}

#[test]
fn the_metadata_commitment_binds_every_field() {
    let base = sample_meta().commitment().unwrap();
    type Change = Box<dyn Fn(&mut ImmutableMeta)>;
    let mutate: Vec<Change> = vec![
        Box::new(|m| m.command_id[0] ^= 1),
        Box::new(|m| m.correlation_id[0] ^= 1),
        Box::new(|m| m.causation_id = Some([0; 16])),
        Box::new(|m| m.actor_type = 2),
        Box::new(|m| m.actor_id.push('x')),
        Box::new(|m| {
            m.idempotency_key =
                IdempotencyKey::parse("4a3b2c1d-9e8f-4a7b-8c6d-5e4f3a2b1c0e").unwrap()
        }),
        Box::new(|m| m.issued_at += 1),
        Box::new(|m| m.device_id[0] ^= 1),
    ];
    for (n, change) in mutate.iter().enumerate() {
        let mut meta = sample_meta();
        change(&mut meta);
        assert_ne!(meta.commitment().unwrap(), base, "field {n} is not bound");
    }
}

#[test]
fn the_envelope_digest_matches_the_spec_vector_and_binds_header_and_ciphertext() {
    let header = vector("header_envelope");
    let ciphertext = vector("envelope_ciphertext");
    let digest = canonical_envelope_digest(&header, &ciphertext).unwrap();
    assert_eq!(digest.as_slice(), vector("envelope_digest"));
    let mut other_header = header.clone();
    other_header[40] ^= 1;
    assert_ne!(
        canonical_envelope_digest(&other_header, &ciphertext).unwrap(),
        digest
    );
    let mut other_ciphertext = ciphertext.clone();
    other_ciphertext[0] ^= 1;
    assert_ne!(
        canonical_envelope_digest(&header, &other_ciphertext).unwrap(),
        digest
    );
    // The lengths are part of the digest: moving a byte across the boundary changes it.
    let mut moved_header = header.clone();
    moved_header.push(ciphertext[0]);
    assert_ne!(
        canonical_envelope_digest(&moved_header, &ciphertext[1..]).unwrap(),
        digest
    );
}

#[test]
fn the_manifest_commitment_is_exact_and_order_independent() {
    let (x, y) = ([1_u8; 32], [2_u8; 32]);
    let ab = ArtifactManifest::new(vec![x, y]).unwrap();
    let ba = ArtifactManifest::new(vec![y, x]).unwrap();
    assert_eq!(ab.commitment(), ba.commitment());
    assert_eq!(ab.ids(), &[x, y]);
    assert_ne!(
        ab.commitment(),
        ArtifactManifest::new(vec![x]).unwrap().commitment()
    );
    assert_ne!(
        ArtifactManifest::new(vec![x]).unwrap().commitment(),
        ArtifactManifest::new(vec![]).unwrap().commitment()
    );
    assert_eq!(
        ArtifactManifest::new(vec![x, x]),
        Err(WireError::ListNotAscending)
    );
    assert_eq!(
        ArtifactManifest::new(vec![[0; 32]; 4097]),
        Err(WireError::TooLong)
    );
}

// ---- the plaintext (C.4) ----------------------------------------------------------------

#[test]
fn the_envelope_plaintext_matches_the_spec_vector() {
    let plaintext = spec_plaintext();
    assert_eq!(plaintext.meta, sample_meta());
    assert_eq!(plaintext.write_set_hash, [0x44; 32]);
    assert_eq!(
        plaintext.payload,
        br#"{"amount_minor":"-1250","currency":"USD"}"#
    );
    assert_eq!(plaintext.delta_manifest.ids().len(), 2);
    assert!(plaintext.delta_manifest.ids()[0] < plaintext.delta_manifest.ids()[1]);
    assert_eq!(
        plaintext.delta_manifest.commitment().as_bytes().as_slice(),
        vector("artifact_manifest_commitment")
    );
    assert_eq!(plaintext.encode().unwrap(), vector("envelope_plaintext"));
}

#[test]
fn the_plaintext_rejects_what_the_grammar_forbids() {
    let good = vector("envelope_plaintext");
    for len in 0..good.len() {
        assert!(
            EnvelopePlaintext::decode(&good[..len]).is_err(),
            "a {len}-byte prefix parsed"
        );
    }
    let mut trailing = good.clone();
    trailing.push(0);
    assert_eq!(
        EnvelopePlaintext::decode(&trailing),
        Err(WireError::TrailingBytes)
    );

    // Offsets: label (2 + 34), command_id, correlation_id, causation presence, then actor_type.
    let actor_type_at = 2 + "dohflow/sync-envelope-plaintext/v1".len() + 16 + 16 + 1;
    for bad in [0_u8, 5, 255] {
        let mut bytes = good.clone();
        bytes[actor_type_at] = bad;
        assert_eq!(
            EnvelopePlaintext::decode(&bytes),
            Err(WireError::UnknownActorType)
        );
    }
    let mut v7_key = good.clone();
    let key_at = good
        .windows(16)
        .position(|w| w == IdempotencyKey::parse(KEY_TEXT).unwrap().as_bytes())
        .unwrap();
    v7_key[key_at + 6] = 0x71;
    assert_eq!(
        EnvelopePlaintext::decode(&v7_key),
        Err(WireError::InvalidIdempotencyKey)
    );
}

#[test]
fn a_delta_manifest_that_is_not_strictly_ascending_is_refused_on_decode() {
    let good = spec_plaintext();
    let (low, high) = (good.delta_manifest.ids()[0], good.delta_manifest.ids()[1]);
    let good_bytes = good.encode().unwrap();
    let find = |needle: &[u8; 32]| good_bytes.windows(32).position(|w| w == needle).unwrap();
    let (low_at, high_at) = (find(&low), find(&high));
    assert_eq!(high_at, low_at + 32, "adjacent list items");

    let mut descending = good_bytes.clone();
    descending[low_at..low_at + 32].copy_from_slice(&high);
    descending[high_at..high_at + 32].copy_from_slice(&low);
    assert_eq!(
        EnvelopePlaintext::decode(&descending),
        Err(WireError::ListNotAscending)
    );

    let mut duplicate = good_bytes;
    duplicate[high_at..high_at + 32].copy_from_slice(&low);
    assert_eq!(
        EnvelopePlaintext::decode(&duplicate),
        Err(WireError::ListNotAscending)
    );
}

#[test]
fn a_list_count_the_input_cannot_hold_is_refused_before_anything_is_allocated() {
    let good = vector("envelope_plaintext");
    let count_at = good
        .windows(32)
        .position(|w| w == [0x44; 32])
        .map(|at| at + 32)
        .unwrap();
    // 4096 is the maximum count; the input holds only two ids.
    let mut hostile = good.clone();
    hostile[count_at..count_at + 4].copy_from_slice(&4096_u32.to_be_bytes());
    assert_eq!(
        EnvelopePlaintext::decode(&hostile),
        Err(WireError::LengthExceedsInput)
    );
    // A count above the maximum.
    hostile[count_at..count_at + 4].copy_from_slice(&u32::MAX.to_be_bytes());
    assert_eq!(EnvelopePlaintext::decode(&hostile), Err(WireError::TooLong));
}

#[test]
fn the_plaintext_round_trips_for_an_empty_manifest_and_a_causation_id() {
    let plaintext = EnvelopePlaintext {
        meta: ImmutableMeta {
            causation_id: Some([0x33; 16]),
            ..sample_meta()
        },
        write_set_hash: [0x44; 32],
        delta_manifest: ArtifactManifest::new(vec![]).unwrap(),
        payload: b"{}".to_vec(),
    };
    assert_eq!(
        EnvelopePlaintext::decode(&plaintext.encode().unwrap()).unwrap(),
        plaintext
    );
    let unknown_actor = EnvelopePlaintext {
        meta: ImmutableMeta {
            actor_type: 9,
            ..sample_meta()
        },
        ..plaintext.clone()
    };
    assert_eq!(unknown_actor.encode(), Err(WireError::UnknownActorType));
    let long_actor = EnvelopePlaintext {
        meta: ImmutableMeta {
            actor_id: "a".repeat(257),
            ..sample_meta()
        },
        ..plaintext
    };
    assert_eq!(long_actor.encode(), Err(WireError::TooLong));
}

// ---- the receiver checks (C.4) ----------------------------------------------------------

#[test]
fn the_spec_envelope_passes_every_receiver_check() {
    check_envelope(&spec_header(), &spec_plaintext(), spec_tag())
        .expect("the spec envelope is valid");
}

#[test]
fn a_tampered_clear_tag_or_commitment_fails_before_anything_is_trusted() {
    let plaintext = spec_plaintext();

    let mut header = spec_header();
    if let ObjectKind::Envelope {
        idempotency_key_tag,
        ..
    } = &mut header.object
    {
        let mut bytes = *idempotency_key_tag.as_bytes();
        bytes[0] ^= 1;
        *idempotency_key_tag = IdempotencyKeyTag::from_derived(bytes);
    }
    assert_eq!(
        check_envelope(&header, &plaintext, spec_tag()),
        Err(ReceiverError::TagMismatch)
    );

    let mut header = spec_header();
    if let ObjectKind::Envelope {
        immutable_metadata_commitment,
        ..
    } = &mut header.object
    {
        let mut bytes = *immutable_metadata_commitment.as_bytes();
        bytes[0] ^= 1;
        *immutable_metadata_commitment = MetadataCommitment::from_derived(bytes);
    }
    assert_eq!(
        check_envelope(&header, &plaintext, spec_tag()),
        Err(ReceiverError::MetadataCommitmentMismatch)
    );

    // And a plaintext whose own metadata was changed no longer matches the header's commitment.
    let mut changed = spec_plaintext();
    changed.meta.issued_at += 1;
    assert_eq!(
        check_envelope(&spec_header(), &changed, spec_tag()),
        Err(ReceiverError::MetadataCommitmentMismatch)
    );
}

#[test]
fn the_header_and_plaintext_must_agree_on_the_command_and_the_device() {
    let mut header = spec_header();
    header.object_id[0] ^= 1;
    assert_eq!(
        check_envelope(&header, &spec_plaintext(), spec_tag()),
        Err(ReceiverError::ObjectIdMismatch)
    );
    let mut header = spec_header();
    header.device_id[0] ^= 1;
    assert_eq!(
        check_envelope(&header, &spec_plaintext(), spec_tag()),
        Err(ReceiverError::DeviceMismatch)
    );
}

#[test]
fn a_nonce_domain_that_is_not_the_derived_one_fails_closed() {
    let mut header = spec_header();
    header.nonce_domain = NonceDomain::from_derived([0, 0, 0, 0]);
    assert_eq!(
        check_envelope(&header, &spec_plaintext(), spec_tag()),
        Err(ReceiverError::NonceDomainMismatch)
    );
    // A domain derived for the other object type is also refused.
    let mut header = spec_header();
    header.nonce_domain = NonceDomain::derive(
        &header.vault_id,
        header.key_epoch,
        &header.device_key_fingerprint,
        2,
    );
    assert_eq!(
        check_envelope(&header, &spec_plaintext(), spec_tag()),
        Err(ReceiverError::NonceDomainMismatch)
    );
}

#[test]
fn the_header_manifest_commitment_is_present_exactly_when_the_delta_manifest_is_not_empty() {
    let empty = |plaintext: &EnvelopePlaintext| EnvelopePlaintext {
        delta_manifest: ArtifactManifest::new(vec![]).unwrap(),
        ..plaintext.clone()
    };
    let with_manifest = |header: &ClearHeader, commitment: Option<ManifestCommitment>| {
        let mut header = header.clone();
        if let ObjectKind::Envelope {
            artifact_manifest_commitment,
            ..
        } = &mut header.object
        {
            *artifact_manifest_commitment = commitment;
        }
        header
    };
    let plaintext = spec_plaintext();
    let header = spec_header();

    // M1: an empty decrypted manifest with a PRESENT header commitment (of the empty manifest)
    // would be a second valid encoding of the same envelope; it is refused.
    let empty_plaintext = empty(&plaintext);
    let empty_commitment = empty_plaintext.delta_manifest.commitment();
    assert_eq!(
        check_envelope(
            &with_manifest(&header, Some(empty_commitment)),
            &empty_plaintext,
            spec_tag()
        ),
        Err(ReceiverError::ManifestCommitmentMismatch)
    );
    // An empty manifest with the commitment absent is the one valid form.
    check_envelope(&with_manifest(&header, None), &empty_plaintext, spec_tag())
        .expect("empty manifest, absent commitment");

    // A non-empty manifest with the commitment absent, or with the wrong commitment, is refused.
    assert_eq!(
        check_envelope(&with_manifest(&header, None), &plaintext, spec_tag()),
        Err(ReceiverError::ManifestCommitmentMismatch)
    );
    assert_eq!(
        check_envelope(
            &with_manifest(&header, Some(empty_commitment)),
            &plaintext,
            spec_tag()
        ),
        Err(ReceiverError::ManifestCommitmentMismatch)
    );
}

#[test]
fn a_snapshot_header_is_not_an_envelope() {
    let snapshot = ClearHeader::decode(&vector("header_snapshot")).unwrap();
    assert_eq!(
        check_envelope(&snapshot, &spec_plaintext(), spec_tag()),
        Err(ReceiverError::NotAnEnvelope)
    );
}

#[test]
fn the_fingerprint_type_is_not_a_digest_stand_in() {
    // Compile-time separation is pinned by the header doctests; this documents the runtime
    // consequence: equal bytes in two different newtypes are not interchangeable in a header.
    let bytes = [7_u8; 32];
    assert_eq!(Fingerprint::from_derived(bytes).as_bytes(), &bytes);
    assert_eq!(IdempotencyKeyTag::from_derived(bytes).as_bytes(), &bytes);
}
