//! `artifact::peek_container` (personal-cfo-v98vd AC35): the frame of a `DFSA` artifact container read
//! from its first bytes and its total length, as a service does before it stores a byte of an upload.
//!
//! Pinned: (1) a public vector file, `docs/architecture/vectors/artifact-peek-v1.json`, that a second
//! implementation must pass: the exact frame, the total one short and one long, a ciphertext length under
//! a tag and near `u64::MAX`, the largest container and one byte over it, a wrong magic and version, and
//! the prefixes too short to decide; (2) **agreement with `parse_container`**: over a valid container and
//! every single-byte and length mutation of its frame, the peek on the frame's prefix gives the answer and
//! the error `parse_container` gives on the whole object, because there is one frame reader; (3) the
//! "prefix too short" result names exactly the bytes that decide it.
//!
//! Regenerate the vectors with `DOHFLOW_UPDATE_VECTORS=1 cargo test -p sync-core --test artifact_peek`.

use serde_json::{json, Value};
use sync_core::artifact::{
    encode_container, parse_container, peek_container, ArtifactError, ContainerPeek, PeekError,
    CONTAINER_FRAME_LEN, MAX_ARTIFACT_CONTAINER,
};

const VECTORS: &str = include_str!("../../../docs/architecture/vectors/artifact-peek-v1.json");
const VECTORS_PATH: &str = "../../docs/architecture/vectors/artifact-peek-v1.json";
const MAX: u64 = MAX_ARTIFACT_CONTAINER as u64;

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len() / 2)
        .map(|i| u8::from_str_radix(&text[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}

/// A frame with the stated version and ciphertext length.
fn frame(magic: &[u8; 4], version: u16, ciphertext_len: u64) -> Vec<u8> {
    let mut out = magic.to_vec();
    out.extend_from_slice(&version.to_be_bytes());
    out.extend_from_slice(&ciphertext_len.to_be_bytes());
    out
}

fn outcome(result: &Result<ContainerPeek, PeekError>) -> Value {
    match result {
        Ok(peek) => json!({ "ok": { "ciphertext_len": peek.ciphertext_len.to_string() } }),
        Err(PeekError::PrefixTooShort { needed }) => {
            json!({ "prefix_too_short": { "needed": needed.to_string() } })
        }
        Err(PeekError::Refused(ArtifactError::ContainerTooLarge)) => {
            json!({ "error": "ContainerTooLarge" })
        }
        Err(PeekError::Refused(ArtifactError::Wire(wire))) => {
            json!({ "error": format!("{wire:?}") })
        }
        Err(PeekError::Refused(other)) => panic!("an artifact frame cannot give {other:?}"),
    }
}

struct Case {
    name: &'static str,
    description: &'static str,
    prefix: Vec<u8>,
    total_len: u64,
}

fn case(name: &'static str, description: &'static str, prefix: Vec<u8>, total_len: u64) -> Case {
    Case {
        name,
        description,
        prefix,
        total_len,
    }
}

fn all_cases() -> Vec<Case> {
    let container = encode_container(&[0xab; 64]); // 14 + 64 = 78
    let good = frame(b"DFSA", 1, 64);
    vec![
        case("the_exact_frame", "The 14-byte frame and the container's length: the ciphertext length is proven.", good.clone(), 78),
        case("the_whole_container_as_the_prefix", "A prefix that is the whole container gives the same answer.", container.clone(), 78),
        case("a_prefix_longer_than_the_frame", "Bytes past the frame are never read: 20 bytes of a 78-byte container give the same answer.", container[..20].to_vec(), 78),
        case("total_len_one_short", "The container is one byte shorter than its ciphertext length says.", good.clone(), 77),
        case("total_len_one_long", "One byte follows the ciphertext.", good.clone(), 79),
        case("ciphertext_len_under_a_tag", "A ciphertext of 15 bytes cannot hold a 16-byte authentication tag.", frame(b"DFSA", 1, 15), 29),
        case("ciphertext_len_zero", "A frame of an empty ciphertext is under a tag.", frame(b"DFSA", 1, 0), 14),
        case("ciphertext_len_u64_max", "ciphertext_len is 2^64 - 1: an error, never an overflow.", frame(b"DFSA", 1, u64::MAX), 78),
        case("total_len_u64_max", "A container claiming to be 2^64 - 1 bytes is over the largest the service stores, whatever its frame says.", good.clone(), u64::MAX),
        case("the_largest_container", "A container of exactly 67,108,864 bytes (max_artifact_bytes) is within the limit.", frame(b"DFSA", 1, MAX - 14), MAX),
        case("one_byte_over_the_largest", "67,108,865 bytes is over the limit, refused before anything is parsed, even with an empty prefix.", Vec::new(), MAX + 1),
        case("bad_magic", "The first four bytes are not DFSA.", frame(b"XFSA", 1, 64), 78),
        case("bad_magic_in_a_two_byte_prefix", "A wrong magic is refused as soon as a wrong byte is in hand.", vec![0x44, 0x58], 78),
        case("unsupported_container_version", "Container version 2.", frame(b"DFSA", 2, 64), 78),
        case("unsupported_version_in_a_six_byte_prefix", "The version is judged as soon as its bytes are in hand.", frame(b"DFSA", 2, 64)[..6].to_vec(), 78),
        case("an_empty_prefix", "Nothing yet: fourteen bytes are needed to read the frame.", Vec::new(), 78),
        case("a_good_magic_prefix", "The magic so far matches; the frame is not complete.", vec![0x44, 0x46, 0x53], 78),
        case("a_good_magic_prefix_of_a_short_container", "A prefix never asks for more than the container holds: three bytes of a 10-byte container need ten.", vec![0x44, 0x46, 0x53], 10),
        case("a_thirteen_byte_prefix", "One byte short of the frame.", good[..13].to_vec(), 78),
        case("total_len_shorter_than_the_frame", "A container of 10 bytes cannot hold a 14-byte frame.", vec![0x44, 0x46, 0x53, 0x41, 0, 1, 0, 0, 0, 0], 10),
        case("a_short_container_seen_through_a_short_prefix", "The same 10-byte container seen through a six-byte prefix is already decided.", vec![0x44, 0x46, 0x53, 0x41, 0, 1], 10),
        case("an_empty_container", "A container of no bytes is truncated, not a request for more.", Vec::new(), 0),
        case("a_prefix_longer_than_the_total", "More bytes in hand than the container has.", good.clone(), 10),
    ]
}

fn run(prefix: &[u8], total_len: u64) -> Value {
    outcome(&peek_container(prefix, total_len, MAX_ARTIFACT_CONTAINER))
}

fn document() -> Value {
    let cases: Vec<Value> = all_cases()
        .iter()
        .map(|c| json!({ "name": c.name, "description": c.description, "prefix": hex(&c.prefix), "total_len": c.total_len.to_string(), "expect": run(&c.prefix, c.total_len) }))
        .collect();
    json!({
        "vectors_version": 1,
        "spec": "docs/architecture/vault-format.md C.9 (the DFSA container); personal-cfo-v98vd AC35",
        "synthetic": "Every container is built from synthetic public values. No key, secret or real value is in this file.",
        "how_to_read": "A case gives the first bytes of a DFSA artifact container ('prefix', hex) and the container's total length in bytes ('total_len', a decimal string), the way a service knows a request body's length before it reads the body; the largest container the service stores is 67,108,864 bytes (max_artifact_bytes). 'expect' is one of: {ok: {ciphertext_len}}, the ciphertext length, proven equal to total_len - 14; {error: <name>}, the error that parsing the whole container would give (BadMagic, UnsupportedContainer, Truncated, LengthExceedsInput, TrailingBytes, CiphertextTooShort, or ContainerTooLarge for a total_len over the maximum, which is judged before anything else); or {prefix_too_short: {needed}}, meaning the prefix ends before the answer can be decided and 'needed' bytes of prefix are enough to continue. The frame is magic 'DFSA', u16 container_version (1), u64 ciphertext_len, all big-endian, then the ciphertext; 14 + ciphertext_len must equal total_len, and the ciphertext is at least 16 bytes (a GCM tag). The checks come in this order: total_len over the maximum; a prefix longer than total_len; the magic and the version as soon as their bytes are in hand; a total_len under 14; the prefix being under 14 bytes; the ciphertext length against what remains (LengthExceedsInput, then TrailingBytes); the tag minimum. The error names are those of the reference implementation's wire errors; an implementation must agree that the container is malformed and that no ciphertext byte is read, but the specific name is informative except where the case says otherwise.",
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
        "the vectors file drifted from the generator; regenerate with DOHFLOW_UPDATE_VECTORS=1"
    );
}

#[test]
fn peek_container_gives_every_committed_vector_its_outcome() {
    let file: Value = serde_json::from_str(VECTORS).unwrap();
    let cases = file["cases"].as_array().unwrap();
    assert!(cases.len() >= 22);
    for case in cases {
        let prefix = unhex(case["prefix"].as_str().unwrap());
        let total: u64 = case["total_len"].as_str().unwrap().parse().unwrap();
        assert_eq!(run(&prefix, total), case["expect"], "{}", case["name"]);
    }
}

#[test]
fn peek_agrees_with_parse_container_on_a_container_and_every_mutation_of_its_frame() {
    let valid = encode_container(&[0x5a; 40]);
    let mut subjects = vec![valid.clone()];
    for at in 0..CONTAINER_FRAME_LEN {
        for value in [0x00, 0x01, 0x02, 0x7f, 0xff, valid[at] ^ 0x01] {
            let mut bytes = valid.clone();
            bytes[at] = value;
            subjects.push(bytes);
        }
    }
    // The container shortened and lengthened, and cut inside the frame.
    for len in [0, 1, 3, 4, 5, 6, 13, 14, 15, 29, 30, 53] {
        subjects.push(valid[..len.min(valid.len())].to_vec());
    }
    subjects.push([valid.clone(), vec![0]].concat());
    for bytes in subjects {
        let total = bytes.len() as u64;
        let prefix = &bytes[..bytes.len().min(CONTAINER_FRAME_LEN)];
        let parsed = parse_container(&bytes, MAX_ARTIFACT_CONTAINER);
        let peeked = peek_container(prefix, total, MAX_ARTIFACT_CONTAINER);
        match (parsed, peeked) {
            (Ok(ciphertext), Ok(peek)) => assert_eq!(ciphertext.len() as u64, peek.ciphertext_len),
            (Err(error), Err(PeekError::Refused(refused))) => {
                assert_eq!(error, refused, "{}", hex(&bytes))
            }
            // Whole object in hand: a shortage is Truncated in both.
            (parsed, peeked) => panic!(
                "disagree on {}: parse {parsed:?}, peek {peeked:?}",
                hex(&bytes)
            ),
        }
    }
}

#[test]
fn a_short_prefix_names_exactly_the_bytes_that_decide_it() {
    let container = encode_container(&[0x5a; 40]);
    let total = container.len() as u64;
    for len in 0..CONTAINER_FRAME_LEN {
        match peek_container(&container[..len], total, MAX_ARTIFACT_CONTAINER) {
            Err(PeekError::PrefixTooShort { needed }) => {
                assert_eq!(needed, CONTAINER_FRAME_LEN as u64, "{len} bytes");
                assert!(peek_container(
                    &container[..needed as usize],
                    total,
                    MAX_ARTIFACT_CONTAINER
                )
                .is_ok());
            }
            other => panic!("{len} bytes should need more, got {other:?}"),
        }
    }
    // Never more than the container holds.
    assert_eq!(
        peek_container(&[0x44, 0x46, 0x53, 0x41, 0, 1], 12, MAX_ARTIFACT_CONTAINER),
        Err(PeekError::Refused(ArtifactError::Wire(
            sync_core::cbe::WireError::Truncated
        )))
    );
}

#[test]
fn parse_container_names_a_wrong_magic_as_soon_as_a_wrong_byte_is_present() {
    // Changed from the earlier parser, which called every input under four bytes Truncated (C.11 maps
    // both to MalformedObject): the one frame reader judges the bytes it has.
    assert_eq!(
        parse_container(&[0x58], MAX_ARTIFACT_CONTAINER),
        Err(sync_core::cbe::WireError::BadMagic.into())
    );
    assert_eq!(
        parse_container(&[0x44, 0x46], MAX_ARTIFACT_CONTAINER),
        Err(sync_core::cbe::WireError::Truncated.into())
    );
    assert_eq!(
        parse_container(&[], MAX_ARTIFACT_CONTAINER),
        Err(sync_core::cbe::WireError::Truncated.into())
    );
}
