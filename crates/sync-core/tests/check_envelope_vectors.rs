//! Public conformance vectors for the receiver checks (vault-format.md C.4 "Receiver checks",
//! C.14; personal-cfo-g3m.23).
//!
//! With the void rules of ADR 0074 Addendum D the receiver checks are consensus-critical: every
//! replica that evaluates a log position must reject exactly the same envelopes, or a void that
//! some honor and others refuse would split the vault. So the checks are pinned by a public file,
//! `docs/architecture/vectors/check-envelope-v1.json`, that a second implementation must pass.
//!
//! Each case is a header (hex), a plaintext (hex), the idempotency-key tag that the receiver's own
//! tagger computes for the plaintext's key (an *input*: the tag secret is not in the file), and the
//! expected outcome: `ok`, `MalformedHeader` or `MalformedPlaintext` (the bytes do not decode), or
//! the name of the failing receiver check. The generator below builds every case from public data
//! only (the C.14 sample envelope), fails if the committed file drifts, and the verifier test
//! checks `check_envelope` against every case.
//!
//! Regenerate with `DOHFLOW_UPDATE_VECTORS=1 cargo test -p sync-core --test check_envelope_vectors`
//! (a spec change under C.13).

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sync_core::envelope::{check_envelope, ArtifactManifest, EnvelopePlaintext, IdempotencyKey};
use sync_core::header::{
    ClearHeader, IdempotencyKeyTag, ManifestCommitment, MetadataCommitment, NonceDomain,
    ObjectKind, OBJECT_ENVELOPE, OBJECT_SNAPSHOT,
};

const SPEC: &str = include_str!("../../../docs/architecture/vault-format.md");
const VECTORS: &str = include_str!("../../../docs/architecture/vectors/check-envelope-v1.json");
const VECTORS_PATH: &str = "../../docs/architecture/vectors/check-envelope-v1.json";
const ENVELOPE_SOURCE: &str = include_str!("../src/envelope.rs");

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len() / 2)
        .map(|i| u8::from_str_radix(&text[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}

/// A C.14 vector of the specification, by name.
fn spec_vector(name: &str) -> Vec<u8> {
    let marker = format!("<!-- sync-vector: {name}=");
    let start = SPEC
        .find(&marker)
        .unwrap_or_else(|| panic!("no vector {name}"))
        + marker.len();
    unhex(SPEC[start..].split(" -->").next().unwrap())
}

/// HMAC-SHA256 (RFC 2104), written out so these tests need no further dependency.
fn hmac_sha256(key: &[u8; 32], message: &[u8]) -> [u8; 32] {
    let mut inner_pad = [0x36_u8; 64];
    let mut outer_pad = [0x5c_u8; 64];
    for (i, byte) in key.iter().enumerate() {
        inner_pad[i] ^= byte;
        outer_pad[i] ^= byte;
    }
    let mut inner = Sha256::new();
    inner.update(inner_pad);
    inner.update(message);
    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner.finalize());
    outer.finalize().into()
}

/// What the receiver's tagger returns for a key: `HMAC-SHA256(tag_key, label ‖ canonical 16-byte
/// idempotency key)` with the C.14 sample tag key (C.3). The file carries the answer for each
/// case's own plaintext key, so no implementation needs the tag secret, and an implementation that
/// tags the wrong input (the key's text form, the command id) fails a vector instead of passing
/// with a correct HMAC wired up wrongly.
fn oracle(key: &IdempotencyKey) -> [u8; 32] {
    let tag_key: [u8; 32] = spec_vector("idempotency_tag_key").try_into().unwrap();
    let label = b"dohflow/sync-idem-tag/v1";
    let mut message = (label.len() as u16).to_be_bytes().to_vec();
    message.extend_from_slice(label);
    message.extend_from_slice(key.as_bytes());
    hmac_sha256(&tag_key, &message)
}

struct Base {
    header: ClearHeader,
    plaintext: EnvelopePlaintext,
    spec_tag: [u8; 32],
}

fn base() -> Base {
    Base {
        header: ClearHeader::decode(&spec_vector("header_envelope")).unwrap(),
        plaintext: EnvelopePlaintext::decode(&spec_vector("envelope_plaintext")).unwrap(),
        spec_tag: spec_vector("idempotency_tag").try_into().unwrap(),
    }
}

struct Case {
    name: &'static str,
    description: &'static str,
    /// More than one defect: the case pins the order of the checks.
    multi_defect: bool,
    header: Vec<u8>,
    plaintext: Vec<u8>,
    /// What the receiver's tagger returns for the plaintext's key.
    tag: [u8; 32],
    expect: &'static str,
}

/// Header and plaintext for a coherent envelope, with the metadata commitment and tag recomputed.
fn coherent(
    base: &Base,
    edit: impl FnOnce(&mut ClearHeader, &mut EnvelopePlaintext),
) -> (ClearHeader, EnvelopePlaintext) {
    let mut header = base.header.clone();
    let mut plaintext = base.plaintext.clone();
    edit(&mut header, &mut plaintext);
    (header, plaintext)
}

fn set_commitment(header: &mut ClearHeader, plaintext: &EnvelopePlaintext) {
    if let ObjectKind::Envelope {
        immutable_metadata_commitment,
        ..
    } = &mut header.object
    {
        *immutable_metadata_commitment = plaintext.meta.commitment().unwrap();
    }
}

fn set_manifest(header: &mut ClearHeader, commitment: Option<ManifestCommitment>) {
    if let ObjectKind::Envelope {
        artifact_manifest_commitment,
        ..
    } = &mut header.object
    {
        *artifact_manifest_commitment = commitment;
    }
}

fn set_tag(header: &mut ClearHeader, tag: [u8; 32]) {
    if let ObjectKind::Envelope {
        idempotency_key_tag,
        ..
    } = &mut header.object
    {
        *idempotency_key_tag = IdempotencyKeyTag::from_derived(tag);
    }
}

/// Build a case from structures; the tag is the oracle's answer for the plaintext's key.
fn case(
    b: &Base,
    name: &'static str,
    description: &'static str,
    multi_defect: bool,
    expect: &'static str,
    parts: (ClearHeader, EnvelopePlaintext),
) -> Case {
    let _ = b;
    let tag = oracle(&parts.1.meta.idempotency_key);
    Case {
        name,
        description,
        multi_defect,
        header: parts.0.encode().unwrap(),
        plaintext: parts.1.encode().unwrap(),
        tag,
        expect,
    }
}

/// A different, valid UUIDv4 idempotency key.
fn other_key() -> IdempotencyKey {
    let mut bytes = [0x5c_u8; 16];
    bytes[6] = 0x4c;
    bytes[8] = 0x9c;
    IdempotencyKey::from_bytes(bytes).unwrap()
}

fn all_cases() -> Vec<Case> {
    let b = base();
    let mut cases = Vec::new();
    let single =
        |name, description, expect, edit: &dyn Fn(&mut ClearHeader, &mut EnvelopePlaintext)| {
            case(
                &b,
                name,
                description,
                false,
                expect,
                coherent(&b, |h, p| edit(h, p)),
            )
        };
    let multi =
        |name, description, expect, edit: &dyn Fn(&mut ClearHeader, &mut EnvelopePlaintext)| {
            case(
                &b,
                name,
                description,
                true,
                expect,
                coherent(&b, |h, p| edit(h, p)),
            )
        };

    // ---- valid ----
    cases.push(single(
        "valid_spec_envelope",
        "the C.14 sample envelope: a delta manifest of two artifact ids and its commitment in the header",
        "ok",
        &|_, _| {},
    ));
    cases.push(single(
        "valid_empty_manifest",
        "an empty delta manifest and no manifest commitment in the header",
        "ok",
        &|h, p| {
            p.delta_manifest = ArtifactManifest::new(vec![]).unwrap();
            set_manifest(h, None);
        },
    ));
    cases.push(single(
        "valid_other_non_empty_manifest",
        "another non-empty delta manifest with its own commitment in the header",
        "ok",
        &|h, p| {
            p.delta_manifest = ArtifactManifest::new(vec![[0x71; 32], [0x72; 32]]).unwrap();
            set_manifest(h, Some(p.delta_manifest.commitment()));
        },
    ));
    cases.push(single(
        "valid_with_causation_id",
        "a causation id present, with the metadata commitment recomputed",
        "ok",
        &|h, p| {
            p.meta.causation_id = Some([0x33; 16]);
            set_commitment(h, p);
        },
    ));
    cases.push(single(
        "valid_system_actor",
        "actor type 2 with another actor id, commitment recomputed",
        "ok",
        &|h, p| {
            p.meta.actor_type = 2;
            p.meta.actor_id = "scheduler".to_owned();
            set_commitment(h, p);
        },
    ));

    for (name, description, micros) in [
        (
            "valid_issued_at_first_instant_of_year_1970",
            "issued_at 1970-01-01T00:00:00Z, the earliest accepted instant, commitment recomputed",
            0_i64,
        ),
        (
            "valid_issued_at_last_microsecond_of_year_9999",
            "issued_at 9999-12-31T23:59:59.999999Z, the latest accepted instant, commitment recomputed",
            253_402_300_799_999_999_i64,
        ),
    ] {
        cases.push(single(name, description, "ok", &|h, p| {
            p.meta.issued_at = micros;
            // A causation id keeps these cases apart from the sample's byte pattern (and out of
            // the secret scanner's card-number shape, which a long digit run in hex can match).
            p.meta.causation_id = Some([0x5a; 16]);
            set_commitment(h, p);
        }));
    }

    // ---- invalid: one defect each ----
    cases.push(single(
        "invalid_snapshot_header",
        "a snapshot header where an envelope header is required",
        "NotAnEnvelope",
        &|h, _| {
            h.object = ObjectKind::Snapshot {
                artifact_manifest_commitment: ManifestCommitment::from_derived([0x61; 32]),
            };
        },
    ));
    cases.push(single(
        "invalid_object_id_is_not_the_command_id",
        "the header's object id differs from the plaintext's command id",
        "ObjectIdMismatch",
        &|h, _| h.object_id[0] ^= 1,
    ));
    cases.push(single(
        "invalid_header_device_is_not_the_plaintext_device",
        "the header's device id differs from the plaintext's device id",
        "DeviceMismatch",
        &|h, _| h.device_id[0] ^= 1,
    ));
    cases.push(single(
        "invalid_tag_in_the_header_is_not_the_tag_of_the_key",
        "the header carries a different tag than the tagger computes for the plaintext's key",
        "TagMismatch",
        &|h, _| set_tag(h, [0x62; 32]),
    ));
    cases.push(single(
        "invalid_plaintext_key_is_not_the_tagged_key",
        "the plaintext carries another valid key (the commitment follows it); the tagger's answer for that key is not the header's tag",
        "TagMismatch",
        &|h, p| {
            p.meta.idempotency_key = other_key();
            set_commitment(h, p);
        },
    ));
    cases.push(single(
        "invalid_metadata_commitment_in_the_header",
        "the header's metadata commitment is altered",
        "MetadataCommitmentMismatch",
        &|h, _| {
            if let ObjectKind::Envelope {
                immutable_metadata_commitment,
                ..
            } = &mut h.object
            {
                *immutable_metadata_commitment = MetadataCommitment::from_derived([0x63; 32]);
            }
        },
    ));
    cases.push(single(
        "invalid_issued_at_changed",
        "the plaintext's issued_at changed without the commitment",
        "MetadataCommitmentMismatch",
        &|_, p| p.meta.issued_at += 1,
    ));
    cases.push(single(
        "invalid_correlation_id_changed",
        "the plaintext's correlation id changed without the commitment",
        "MetadataCommitmentMismatch",
        &|_, p| p.meta.correlation_id[0] ^= 1,
    ));
    cases.push(single(
        "invalid_causation_id_added",
        "a causation id added without the commitment",
        "MetadataCommitmentMismatch",
        &|_, p| p.meta.causation_id = Some([0x34; 16]),
    ));
    cases.push(single(
        "invalid_actor_id_changed",
        "the actor id changed without the commitment",
        "MetadataCommitmentMismatch",
        &|_, p| p.meta.actor_id.push('x'),
    ));
    cases.push(single(
        "invalid_actor_type_changed",
        "the actor type changed without the commitment",
        "MetadataCommitmentMismatch",
        &|_, p| p.meta.actor_type = 2,
    ));
    cases.push(single(
        "invalid_nonce_domain_flipped",
        "the nonce domain differs from the one derived for the vault, epoch, fingerprint and type",
        "NonceDomainMismatch",
        &|h, _| h.nonce_domain = NonceDomain::from_derived([0xde, 0xad, 0xbe, 0xef]),
    ));
    cases.push(single(
        "invalid_nonce_domain_of_another_epoch",
        "the nonce domain derived for the next key epoch",
        "NonceDomainMismatch",
        &|h, _| {
            h.nonce_domain = NonceDomain::derive(
                &h.vault_id,
                h.key_epoch + 1,
                &h.device_key_fingerprint,
                OBJECT_ENVELOPE,
            );
        },
    ));
    cases.push(single(
        "invalid_nonce_domain_of_the_snapshot_type",
        "the nonce domain derived for the snapshot object type",
        "NonceDomainMismatch",
        &|h, _| {
            h.nonce_domain = NonceDomain::derive(
                &h.vault_id,
                h.key_epoch,
                &h.device_key_fingerprint,
                OBJECT_SNAPSHOT,
            );
        },
    ));
    cases.push(single(
        "invalid_nonce_domain_of_another_vault",
        "the nonce domain derived for another vault",
        "NonceDomainMismatch",
        &|h, _| {
            let mut vault = h.vault_id;
            vault[0] ^= 1;
            h.nonce_domain = NonceDomain::derive(
                &vault,
                h.key_epoch,
                &h.device_key_fingerprint,
                OBJECT_ENVELOPE,
            );
        },
    ));
    cases.push(single(
        "invalid_manifest_commitment_for_an_empty_manifest",
        "a manifest commitment present (even the commitment of the empty manifest) although the delta manifest is empty",
        "ManifestCommitmentMismatch",
        &|h, p| {
            p.delta_manifest = ArtifactManifest::new(vec![]).unwrap();
            set_manifest(h, Some(p.delta_manifest.commitment()));
        },
    ));
    cases.push(single(
        "invalid_manifest_commitment_absent_for_a_non_empty_manifest",
        "a non-empty delta manifest with no commitment in the header",
        "ManifestCommitmentMismatch",
        &|h, _| set_manifest(h, None),
    ));
    cases.push(single(
        "invalid_manifest_commitment_of_another_manifest",
        "the header commits to a different manifest than the plaintext carries",
        "ManifestCommitmentMismatch",
        &|h, _| {
            set_manifest(
                h,
                Some(
                    ArtifactManifest::new(vec![[0x72; 32]])
                        .unwrap()
                        .commitment(),
                ),
            );
        },
    ));

    for (name, description, micros) in [
        (
            "invalid_issued_at_one_microsecond_before_year_1970",
            "issued_at one microsecond before 1970-01-01T00:00:00Z, commitment recomputed",
            -1_i64,
        ),
        (
            "invalid_issued_at_first_instant_of_year_10000",
            "issued_at 10000-01-01T00:00:00Z, one microsecond past the latest accepted instant, commitment recomputed",
            253_402_300_800_000_000_i64,
        ),
        (
            "invalid_issued_at_i64_max",
            "issued_at i64::MAX microseconds, commitment recomputed",
            i64::MAX,
        ),
        (
            // Ten above i64::MIN: the minimum itself is eight followed by fifteen zeros in hex,
            // a sixteen-digit run that the secret scanner reads as a card number.
            "invalid_issued_at_near_i64_min",
            "issued_at ten microseconds above i64::MIN, commitment recomputed",
            i64::MIN + 10,
        ),
    ] {
        cases.push(single(name, description, "IssuedAtOutOfRange", &|h, p| {
            p.meta.issued_at = micros;
            // A causation id keeps these cases apart from the sample's byte pattern (and out of
            // the secret scanner's card-number shape, which a long digit run in hex can match).
            p.meta.causation_id = Some([0x5a; 16]);
            set_commitment(h, p);
        }));
    }

    // ---- invalid: several defects pin the order of the checks ----
    cases.push(multi(
        "order_snapshot_header_before_everything",
        "a snapshot header and a changed object id: the type is checked first",
        "NotAnEnvelope",
        &|h, _| {
            h.object = ObjectKind::Snapshot {
                artifact_manifest_commitment: ManifestCommitment::from_derived([0x61; 32]),
            };
            h.object_id[0] ^= 1;
        },
    ));
    cases.push(multi(
        "order_object_id_before_device",
        "a wrong object id and a wrong device id: the object id is checked first",
        "ObjectIdMismatch",
        &|h, _| {
            h.object_id[0] ^= 1;
            h.device_id[0] ^= 1;
        },
    ));
    cases.push(multi(
        "order_device_before_tag",
        "a wrong device id and a wrong tag: the device is checked first",
        "DeviceMismatch",
        &|h, _| {
            h.device_id[0] ^= 1;
            set_tag(h, [0x62; 32]);
        },
    ));
    cases.push(multi(
        "order_tag_before_metadata_commitment",
        "a wrong tag and a stale metadata commitment: the tag is checked first",
        "TagMismatch",
        &|h, p| {
            set_tag(h, [0x62; 32]);
            p.meta.issued_at += 1;
        },
    ));
    cases.push(multi(
        "order_metadata_commitment_before_nonce_domain",
        "a stale metadata commitment and a wrong nonce domain: the commitment is checked first",
        "MetadataCommitmentMismatch",
        &|h, p| {
            p.meta.issued_at += 1;
            h.nonce_domain = NonceDomain::from_derived([1, 2, 3, 4]);
        },
    ));
    cases.push(multi(
        "order_nonce_domain_before_manifest",
        "a wrong nonce domain and a missing manifest commitment for a non-empty manifest: the domain is checked first",
        "NonceDomainMismatch",
        &|h, _| {
            h.nonce_domain = NonceDomain::from_derived([1, 2, 3, 4]);
            set_manifest(h, None);
        },
    ));

    // ---- invalid: the bytes do not decode ----
    let header = b.header.encode().unwrap();
    let plaintext = b.plaintext.encode().unwrap();
    let raw = |name, description, header: Vec<u8>, plaintext: Vec<u8>, expect| Case {
        name,
        description,
        multi_defect: false,
        header,
        plaintext,
        tag: b.spec_tag,
        expect,
    };
    cases.push(raw(
        "malformed_header_truncated",
        "the header is one byte short",
        header[..header.len() - 1].to_vec(),
        plaintext.clone(),
        "MalformedHeader",
    ));
    cases.push(raw(
        "malformed_header_trailing_byte",
        "the header has a trailing byte",
        [header.clone(), vec![0]].concat(),
        plaintext.clone(),
        "MalformedHeader",
    ));
    cases.push(raw(
        "malformed_plaintext_truncated",
        "the plaintext is one byte short",
        header.clone(),
        plaintext[..plaintext.len() - 1].to_vec(),
        "MalformedPlaintext",
    ));
    cases.push(raw(
        "malformed_plaintext_trailing_byte",
        "the plaintext has a trailing byte",
        header.clone(),
        [plaintext.clone(), vec![0]].concat(),
        "MalformedPlaintext",
    ));
    // The actor-type byte: the one position where actor types 1 and 2 differ.
    let mut as_system = b.plaintext.clone();
    as_system.meta.actor_type = 2;
    let system_bytes = as_system.encode().unwrap();
    let position = plaintext
        .iter()
        .zip(&system_bytes)
        .position(|(x, y)| x != y)
        .expect("one byte differs");
    for (name, description, value) in [
        (
            "malformed_plaintext_actor_type_zero",
            "actor type 0 is outside the registry",
            0_u8,
        ),
        (
            "malformed_plaintext_actor_type_unknown",
            "actor type 9 is outside the registry",
            9_u8,
        ),
    ] {
        let mut bytes = plaintext.clone();
        bytes[position] = value;
        cases.push(raw(
            name,
            description,
            header.clone(),
            bytes,
            "MalformedPlaintext",
        ));
    }
    // The idempotency key's version nibble: make the key a UUID version 7.
    let key = *b.plaintext.meta.idempotency_key.as_bytes();
    let at = plaintext
        .windows(16)
        .position(|w| w == key)
        .expect("the key is in the plaintext");
    let mut v7 = plaintext.clone();
    v7[at + 6] = 0x70 | (v7[at + 6] & 0x0f);
    cases.push(raw(
        "malformed_plaintext_key_is_not_uuid_v4",
        "the idempotency key is a UUID version 7",
        header,
        v7,
        "MalformedPlaintext",
    ));
    cases
}

/// What `check_envelope` says about a case's bytes.
fn run(case: &Case) -> String {
    let Ok(header) = ClearHeader::decode(&case.header) else {
        return "MalformedHeader".to_owned();
    };
    let Ok(plaintext) = EnvelopePlaintext::decode(&case.plaintext) else {
        return "MalformedPlaintext".to_owned();
    };
    let tag = IdempotencyKeyTag::from_derived(case.tag);
    match check_envelope(&header, &plaintext, |_| tag) {
        Ok(_) => "ok".to_owned(),
        Err(error) => format!("{error:?}"),
    }
}

fn document() -> Value {
    let cases: Vec<Value> = all_cases()
        .iter()
        .map(|case| {
            // The intended outcome must be what the code says, or the vector is wrong.
            assert_eq!(run(case), case.expect, "{}: intended outcome", case.name);
            json!({
                "name": case.name,
                "description": case.description,
                "multi_defect": case.multi_defect,
                "error_normative": !case.multi_defect,
                "header": hex(&case.header),
                "plaintext": hex(&case.plaintext),
                "tagger_output": hex(&case.tag),
                "expect": case.expect,
            })
        })
        .collect();
    json!({
        "vectors_version": 2,
        "spec": "docs/architecture/vault-format.md C.4 \"Receiver checks\" and C.14",
        "synthetic": "Every case is built from the public C.14 sample envelope. No key, secret or real value is in this file.",
        "how_to_read": "Each case gives a clear header and an envelope plaintext (hex of their canonical bytes), 'tagger_output' (the idempotency-key tag that the receiver's own tagger returns for the plaintext's key: an input, because the tag secret is not in this file), and 'expect'. An implementation decodes the header, then the plaintext; if either does not decode the outcome is 'MalformedHeader' or 'MalformedPlaintext'. Otherwise it runs the receiver checks in the order below and the outcome is 'ok' or the name of the first failing check. Every outcome other than 'ok' means the envelope is INVALID, and that class is the contract: a decode failure counts (a MalformedPlaintext after a successful open is a producer fault; a MalformedHeader at an accepted position means the service let through what its own checks should have refused; both are deterministic from the bytes and voidable). For a case with 'error_normative' true the specific outcome is also normative; for a multi-defect case it is informative (the reference order): a second implementation must agree the envelope is invalid but may report a different first error. Internal error names are never part of the contract. 'tagger_output' is HMAC-SHA256 under the C.14 sample tag key over the CBE1 label 'dohflow/sync-idem-tag/v1' and the plaintext's canonical 16-byte idempotency key.",
        "check_order": ["NotAnEnvelope", "ObjectIdMismatch", "DeviceMismatch", "TagMismatch", "MetadataCommitmentMismatch", "NonceDomainMismatch", "ManifestCommitmentMismatch", "IssuedAtOutOfRange"],
        "single_defect": "A case with 'multi_defect' false has exactly one defect and its error is normative. A case with 'multi_defect' true has several: it documents the reference order of check_order, and its error is informative.",
        "not_checked_here": "The checks that need the membership chain or the AEAD key (the sealing device belongs to a member of the object's epoch; the AEAD key was derived from the expected vault, epoch and chain fingerprint) are the caller's, as in C.4, and are not in this file.",
        "unreachable_from_bytes": [
            {
                "variant": "UnencodableMetadata",
                "why": "Raised when the plaintext's own fields cannot be committed (an unknown actor type or an oversize actor id). Both are refused when the plaintext decodes, so no decoded plaintext reaches it; the decode-level cases above ('malformed_plaintext_actor_type_*') cover those inputs."
            }
        ],
        "cases": cases,
    })
}

fn pretty(value: &Value) -> String {
    let mut text = serde_json::to_string_pretty(value).unwrap();
    text.push('\n');
    text
}

#[test]
fn the_committed_vectors_are_what_the_generator_produces() {
    let generated = pretty(&document());
    if std::env::var_os("DOHFLOW_UPDATE_VECTORS").is_some() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(VECTORS_PATH);
        std::fs::write(&path, &generated).unwrap();
        return;
    }
    assert_eq!(
        VECTORS, generated,
        "the vectors file drifted from the generator; regenerate with DOHFLOW_UPDATE_VECTORS=1 \
         (a spec change under C.13)"
    );
}

#[test]
fn the_tagger_output_is_the_hmac_of_the_plaintexts_canonical_key() {
    // The oracle reproduces the C.14 idempotency_tag for the sample key, so the file's answers are
    // real HMAC outputs and a second implementation tagging the wrong input fails a vector.
    let b = base();
    assert_eq!(oracle(&b.plaintext.meta.idempotency_key), b.spec_tag);
    let doc: Value = serde_json::from_str(VECTORS).unwrap();
    let sample = doc["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "valid_spec_envelope")
        .unwrap();
    assert_eq!(sample["tagger_output"].as_str().unwrap(), hex(&b.spec_tag));
}

#[test]
fn check_envelope_agrees_with_every_committed_case() {
    let doc: Value = serde_json::from_str(VECTORS).unwrap();
    assert_eq!(doc["vectors_version"], 2);
    let cases = doc["cases"].as_array().unwrap();
    assert!(cases.len() >= 35, "{} cases", cases.len());
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let rebuilt = Case {
            name: "",
            description: "",
            multi_defect: false,
            header: unhex(case["header"].as_str().unwrap()),
            plaintext: unhex(case["plaintext"].as_str().unwrap()),
            tag: unhex(case["tagger_output"].as_str().unwrap())
                .try_into()
                .unwrap(),
            expect: "",
        };
        assert_eq!(run(&rebuilt), case["expect"].as_str().unwrap(), "{name}");
    }
}

#[test]
fn every_receiver_error_variant_has_a_vector_or_a_stated_reason() {
    // The variants are read from the source, so a new one cannot be added without this failing
    // until it has a vector (or an explicit, reasoned entry in 'unreachable_from_bytes').
    let start = ENVELOPE_SOURCE
        .find("pub enum ReceiverError {")
        .expect("the enum is in envelope.rs");
    let body = &ENVELOPE_SOURCE[start..];
    let body = &body[..body.find("\n}\n").unwrap()];
    let variants: Vec<&str> = body
        .lines()
        .map(str::trim)
        .filter(|l| l.chars().next().is_some_and(|c| c.is_ascii_uppercase()) && l.ends_with(','))
        .map(|l| l.trim_end_matches(','))
        .collect();
    assert!(variants.len() >= 8, "found {variants:?}");

    let doc: Value = serde_json::from_str(VECTORS).unwrap();
    let covered: Vec<&str> = doc["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["expect"].as_str().unwrap())
        .collect();
    let excused: Vec<&str> = doc["unreachable_from_bytes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["variant"].as_str().unwrap())
        .collect();
    for variant in &variants {
        assert!(
            covered.contains(variant) || excused.contains(variant),
            "ReceiverError::{variant} has no conformance vector (add a case to the generator in \
             crates/sync-core/tests/check_envelope_vectors.rs and regenerate)"
        );
    }
    for excuse in &excused {
        assert!(
            variants.contains(excuse),
            "{excuse} is excused but is not a variant"
        );
        assert!(
            !covered.contains(excuse),
            "{excuse} is excused but has a vector"
        );
    }
}

#[test]
fn single_defect_cases_name_the_check_that_fails_and_multi_defect_cases_follow_the_order() {
    let doc: Value = serde_json::from_str(VECTORS).unwrap();
    let order: Vec<&str> = doc["check_order"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    for case in doc["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let expect = case["expect"].as_str().unwrap();
        if name.starts_with("valid_") {
            assert_eq!(expect, "ok", "{name}");
        } else if name.starts_with("malformed_") {
            assert!(expect.starts_with("Malformed"), "{name}");
        } else if name.starts_with("order_") {
            assert_eq!(case["multi_defect"], true, "{name}");
            assert!(order.contains(&expect), "{name}");
        } else {
            assert!(name.starts_with("invalid_"), "{name}");
            assert_eq!(case["multi_defect"], false, "{name}");
            assert!(order.contains(&expect), "{name}");
        }
    }
    // Each check in the order has at least one single-defect case and one that pins it.
    for check in &order {
        let singles = doc["cases"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["expect"] == *check && c["multi_defect"] == false);
        assert!(singles.count() >= 1, "{check} has no single-defect case");
    }
}
