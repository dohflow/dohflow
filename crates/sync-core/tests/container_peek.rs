//! `peek_container` (personal-cfo-v98vd AC29): the header of a container read from its first
//! bytes and its total length, as a service does before it stores a byte of a large upload.
//!
//! Three things are pinned. (1) A public vector file, `docs/architecture/vectors/container-peek-v1.json`,
//! a second implementation must pass: the exact frame, the total length one short and one long,
//! a header length past the prefix, a ciphertext length near `u64::MAX`, and the prefixes too
//! short to decide. (2) **Agreement with `parse_container`**: over every container the repository's
//! vectors hold, and over single-byte and length mutations of each, `peek_container` on a prefix
//! that holds the header gives the header and the errors `parse_container` gives on the whole
//! object, because there is one frame reader. (3) The "prefix too short" result is exact: it
//! names the bytes needed, and with that many bytes the question is decided.
//!
//! Regenerate the vectors with `DOHFLOW_UPDATE_VECTORS=1 cargo test -p sync-core --test container_peek`.

use serde_json::{json, Value};
use sync_core::cbe::WireError;
use sync_core::header::{
    encode_container, parse_container, peek_container, ClearHeader, ContainerPeek, Fingerprint,
    ManifestCommitment, NonceDomain, ObjectKind, PeekError, CONTAINER_MAGIC,
};

const VECTORS: &str = include_str!("../../../docs/architecture/vectors/container-peek-v1.json");
const VECTORS_PATH: &str = "../../docs/architecture/vectors/container-peek-v1.json";
const SPEC: &str = include_str!("../../../docs/architecture/vault-format.md");

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if text.len() & 1 == 1 {
        return None;
    }
    (0..text.len() / 2)
        .map(|i| u8::from_str_radix(&text[2 * i..2 * i + 2], 16).ok())
        .collect()
}

/// A snapshot container built from public, synthetic values.
fn snapshot_container(ciphertext_len: usize) -> Vec<u8> {
    let header = ClearHeader {
        vault_id: [0x11; 16],
        key_epoch: 3,
        object: ObjectKind::Snapshot {
            artifact_manifest_commitment: ManifestCommitment::from_derived([0x22; 32]),
        },
        object_id: [0x33; 16],
        device_id: [0xa1; 16],
        device_key_fingerprint: Fingerprint::from_derived([0x44; 32]),
        nonce_domain: NonceDomain::from_derived([0x55; 4]),
        invocation_counter: 0,
        base_seq: 1000,
        payload_schema_version: 1,
        engine_version: "dohflow-0.3.0".to_owned(),
        schema_version: 55,
    };
    encode_container(&header, &vec![0x66; ciphertext_len]).unwrap()
}

fn header_len(container: &[u8]) -> usize {
    u32::from_be_bytes(container[6..10].try_into().unwrap()) as usize
}

/// How a result is written in the vectors file.
fn outcome(result: &Result<ContainerPeek, PeekError>) -> Value {
    match result {
        Ok(peek) => json!({
            "ok": {
                "header": hex(&peek.header.encode().unwrap()),
                "header_len": peek.header_len.to_string(),
                "ciphertext_len": peek.ciphertext_len.to_string(),
            }
        }),
        Err(PeekError::PrefixTooShort { needed }) => {
            json!({ "prefix_too_short": { "needed": needed.to_string() } })
        }
        Err(PeekError::Wire(error)) => json!({ "error": format!("{error:?}") }),
    }
}

struct Case {
    name: &'static str,
    description: &'static str,
    prefix: Vec<u8>,
    total_len: u64,
    /// What the case is meant to produce, stated by hand and checked against the code.
    expect: Value,
}

fn all_cases() -> Vec<Case> {
    let container = snapshot_container(64);
    let hl = header_len(&container);
    let frame_end = 10 + hl + 8;
    let total = container.len() as u64;
    let ok = {
        let header = ClearHeader::decode(&container[10..10 + hl]).unwrap();
        json!({ "ok": { "header": hex(&header.encode().unwrap()), "header_len": hl.to_string(), "ciphertext_len": "64" } })
    };
    let with = |edit: &dyn Fn(&mut Vec<u8>)| {
        let mut bytes = container.clone();
        edit(&mut bytes);
        bytes
    };
    let set_ciphertext_len = |bytes: &mut Vec<u8>, value: u64| {
        bytes[10 + hl..frame_end].copy_from_slice(&value.to_be_bytes());
    };
    vec![
        Case {
            name: "the_exact_frame",
            description: "The prefix ends exactly after the ciphertext length and total_len is the container's length: the header and both lengths are returned and no ciphertext byte is read.",
            prefix: container[..frame_end].to_vec(),
            total_len: total,
            expect: ok.clone(),
        },
        Case {
            name: "the_whole_container_as_the_prefix",
            description: "A prefix that is the whole container gives the same answer as the exact frame.",
            prefix: container.clone(),
            total_len: total,
            expect: ok,
        },
        Case {
            name: "total_len_one_short",
            description: "The container is one byte shorter than its ciphertext length says.",
            prefix: container[..frame_end].to_vec(),
            total_len: total - 1,
            expect: json!({ "error": "LengthExceedsInput" }),
        },
        Case {
            name: "total_len_one_long",
            description: "One byte follows the ciphertext.",
            prefix: container[..frame_end].to_vec(),
            total_len: total + 1,
            expect: json!({ "error": "TrailingBytes" }),
        },
        Case {
            name: "header_len_past_the_total",
            description: "header_len is larger than the container leaves, with only the first ten bytes in hand: refused by the lengths alone.",
            prefix: with(&|b| b[6..10].copy_from_slice(&u32::MAX.to_be_bytes()))[..10].to_vec(),
            total_len: total,
            expect: json!({ "error": "LengthExceedsInput" }),
        },
        Case {
            name: "header_len_one_past_the_total",
            description: "header_len is one more than the container leaves after the frame head.",
            prefix: with(&|b| b[6..10].copy_from_slice(&u32::try_from(total - 9).unwrap().to_be_bytes()))[..10].to_vec(),
            total_len: total,
            expect: json!({ "error": "LengthExceedsInput" }),
        },
        Case {
            name: "header_len_leaves_no_room_for_the_ciphertext_length",
            description: "header_len fills the container exactly: more is needed, and never more than the container has.",
            prefix: with(&|b| b[6..10].copy_from_slice(&u32::try_from(total - 10).unwrap().to_be_bytes()))[..10].to_vec(),
            total_len: total,
            expect: json!({ "prefix_too_short": { "needed": total.to_string() } }),
        },
        Case {
            name: "a_prefix_longer_than_a_tiny_total",
            description: "More bytes in hand than a four-byte container has: refused, with no arithmetic on the missing bytes.",
            prefix: container[..20].to_vec(),
            total_len: 4,
            expect: json!({ "error": "LengthExceedsInput" }),
        },
        Case {
            name: "an_empty_container",
            description: "A container of no bytes is truncated, not a request for more.",
            prefix: Vec::new(),
            total_len: 0,
            expect: json!({ "error": "Truncated" }),
        },
        Case {
            name: "total_len_cannot_hold_the_ciphertext_length",
            description: "The header is in hand and decodes, the prefix is shorter than the total, and the total ends before the eight-byte ciphertext length could: the container is truncated, so this is an error and not a request for more bytes.",
            prefix: container[..10 + hl + 2].to_vec(),
            total_len: (10 + hl + 5) as u64,
            expect: json!({ "error": "Truncated" }),
        },
        Case {
            name: "header_len_past_the_prefix",
            description: "The first ten bytes are in hand, the header is not: the answer is that more is needed, and how much (the header, then the eight-byte ciphertext length).",
            prefix: container[..10].to_vec(),
            total_len: total,
            expect: json!({ "prefix_too_short": { "needed": frame_end.to_string() } }),
        },
        Case {
            name: "the_header_without_the_ciphertext_length",
            description: "The header is in hand and decodes, the eight-byte ciphertext length is not.",
            prefix: container[..frame_end - 1].to_vec(),
            total_len: total,
            expect: json!({ "prefix_too_short": { "needed": frame_end.to_string() } }),
        },
        Case {
            name: "ciphertext_len_u64_max",
            description: "ciphertext_len is 2^64 - 1: an error, never an overflow.",
            prefix: with(&|b| set_ciphertext_len(b, u64::MAX))[..frame_end].to_vec(),
            total_len: total,
            expect: json!({ "error": "LengthExceedsInput" }),
        },
        Case {
            name: "ciphertext_len_u64_max_minus_one",
            description: "ciphertext_len is 2^64 - 2.",
            prefix: with(&|b| set_ciphertext_len(b, u64::MAX - 1))[..frame_end].to_vec(),
            total_len: total,
            expect: json!({ "error": "LengthExceedsInput" }),
        },
        Case {
            name: "total_len_u64_max",
            description: "The container claims to be 2^64 - 1 bytes with a small ciphertext length: the surplus is trailing bytes, and no sum overflows.",
            prefix: container[..frame_end].to_vec(),
            total_len: u64::MAX,
            expect: json!({ "error": "TrailingBytes" }),
        },
        Case {
            name: "ciphertext_len_under_a_tag",
            description: "A ciphertext of 15 bytes cannot hold a 16-byte authentication tag.",
            prefix: snapshot_container(15)[..frame_end].to_vec(),
            total_len: snapshot_container(15).len() as u64,
            expect: json!({ "error": "CiphertextTooShort" }),
        },
        Case {
            name: "bad_magic",
            description: "The first four bytes are not DFSY.",
            prefix: with(&|b| b[0] = b'X')[..frame_end].to_vec(),
            total_len: total,
            expect: json!({ "error": "BadMagic" }),
        },
        Case {
            name: "bad_magic_in_a_two_byte_prefix",
            description: "A wrong magic is refused as soon as a wrong byte is in hand.",
            prefix: vec![b'D', b'X'],
            total_len: total,
            expect: json!({ "error": "BadMagic" }),
        },
        Case {
            name: "unsupported_container_version",
            description: "Container version 2.",
            prefix: with(&|b| b[4..6].copy_from_slice(&2_u16.to_be_bytes()))[..frame_end].to_vec(),
            total_len: total,
            expect: json!({ "error": "UnsupportedContainer" }),
        },
        Case {
            name: "an_empty_prefix",
            description: "Nothing yet: ten bytes are needed to read the frame head.",
            prefix: Vec::new(),
            total_len: total,
            expect: json!({ "prefix_too_short": { "needed": "10" } }),
        },
        Case {
            name: "a_good_magic_prefix",
            description: "The magic so far matches; the head is not complete.",
            prefix: CONTAINER_MAGIC[..3].to_vec(),
            total_len: total,
            expect: json!({ "prefix_too_short": { "needed": "10" } }),
        },
        Case {
            name: "a_malformed_header",
            description: "The header's label is wrong: the header error is the one parse_container gives.",
            prefix: with(&|b| b[10] ^= 0xff)[..frame_end].to_vec(),
            total_len: total,
            expect: json!({ "error": "BadLabel" }),
        },
        Case {
            name: "a_prefix_longer_than_the_total",
            description: "More bytes in hand than the container has: refused, since the lengths cannot both be true.",
            prefix: container.clone(),
            total_len: total - 1,
            expect: json!({ "error": "LengthExceedsInput" }),
        },
    ]
}

fn run(prefix: &[u8], total_len: u64) -> Value {
    outcome(&peek_container(prefix, total_len))
}

fn document() -> Value {
    let cases: Vec<Value> = all_cases()
        .iter()
        .map(|case| {
            assert_eq!(
                run(&case.prefix, case.total_len),
                case.expect,
                "{}: intended outcome",
                case.name
            );
            json!({
                "name": case.name,
                "description": case.description,
                "prefix": hex(&case.prefix),
                "total_len": case.total_len.to_string(),
                "expect": case.expect,
            })
        })
        .collect();
    json!({
        "vectors_version": 1,
        "spec": "docs/architecture/vault-format.md C.4 (the container frame); personal-cfo-v98vd AC29",
        "synthetic": "Every container is built from synthetic public values. No key, secret or real value is in this file.",
        "how_to_read": "A case gives the first bytes of a container ('prefix', hex) and the container's total length in bytes ('total_len', a decimal string), the way a service knows a request body's length before it reads the body. 'expect' is one of: {ok: {header, header_len, ciphertext_len}} where 'header' is the canonical header bytes (hex); {error: <name>}, the error that parsing the whole container would give; or {prefix_too_short: {needed}}, meaning the prefix ends before the answer can be decided and 'needed' bytes of prefix are enough to continue. The container frame is magic 'DFSY', u16 container_version, u32 header_len, the header, u64 ciphertext_len, the ciphertext, all lengths big-endian; 10 + header_len + 8 + ciphertext_len must equal total_len. Error names are those of the reference implementation's wire errors (Truncated, TrailingBytes, TooLong, LengthExceedsInput, BadMagic, UnsupportedContainer, CiphertextTooShort and the header's own); an implementation must agree that the container is malformed and that no ciphertext byte is read, but the specific name is informative except where the case says otherwise.",
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
    assert!(cases.len() >= 15);
    for case in cases {
        let prefix = unhex(case["prefix"].as_str().unwrap()).unwrap();
        let total: u64 = case["total_len"].as_str().unwrap().parse().unwrap();
        assert_eq!(run(&prefix, total), case["expect"], "{}", case["name"]);
    }
}

/// Every DFSY container in the repository's committed vectors: hex strings that start with the
/// magic, in the JSON vector files (any nesting) and in the spec's `sync-vector` markers.
fn vector_containers() -> Vec<Vec<u8>> {
    fn walk(value: &Value, out: &mut Vec<Vec<u8>>) {
        match value {
            Value::String(text) => {
                if text.starts_with(&hex(CONTAINER_MAGIC)) {
                    if let Some(bytes) = unhex(text) {
                        out.push(bytes);
                    }
                }
            }
            Value::Array(items) => items.iter().for_each(|v| walk(v, out)),
            Value::Object(map) => map.values().for_each(|v| walk(v, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    // Every JSON file in the vectors directory, so a future vector file joins the agreement test
    // without anyone remembering to list it. This file's own prefixes are not whole containers.
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/architecture/vectors");
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|e| e == "json"))
        .filter(|path| !path.ends_with("container-peek-v1.json"))
        .collect();
    files.sort();
    assert!(
        files.len() >= 6,
        "the vectors directory has {} files",
        files.len()
    );
    for path in files {
        let text = std::fs::read_to_string(&path).unwrap();
        walk(&serde_json::from_str::<Value>(&text).unwrap(), &mut out);
    }
    for part in SPEC.split("<!-- sync-vector: ").skip(1) {
        if let Some(value) = part.split("-->").next().and_then(|s| s.split_once('=')) {
            let text = value.1.trim();
            if text.starts_with(&hex(CONTAINER_MAGIC)) {
                out.extend(unhex(text));
            }
        }
    }
    // The generator's own containers, so the agreement does not depend on the files holding any.
    out.push(snapshot_container(16));
    out.push(snapshot_container(4096));
    out
}

/// What `parse_container` says about `bytes`, in the terms of `peek_container`.
fn parse_outcome(bytes: &[u8]) -> Result<(ClearHeader, u32, u64), WireError> {
    let parsed = parse_container(bytes)?;
    let header_len = u32::try_from(parsed.header_bytes.len()).unwrap();
    Ok((parsed.header, header_len, parsed.ciphertext.len() as u64))
}

fn peek_outcome(prefix: &[u8], total_len: u64) -> Result<(ClearHeader, u32, u64), PeekError> {
    peek_container(prefix, total_len).map(|p| (p.header, p.header_len, p.ciphertext_len))
}

#[test]
fn the_vectors_hold_containers_for_the_agreement_test_to_run_on() {
    let containers = vector_containers();
    assert!(containers.len() >= 3, "{} containers", containers.len());
    assert!(
        containers.iter().any(|c| parse_container(c).is_ok()),
        "at least one vector container parses"
    );
}

#[test]
fn peek_agrees_with_parse_on_every_vector_container_and_prefix_that_holds_the_header() {
    for container in vector_containers() {
        let whole = parse_outcome(&container);
        // Any prefix that holds the frame head, the header and the ciphertext length.
        let frame_end = 10
            + container
                .get(6..10)
                .map_or(0, |b| u32::from_be_bytes(b.try_into().unwrap()) as usize)
            + 8;
        let lengths: Vec<usize> = if whole.is_ok() {
            let top = container.len().min(frame_end + 40);
            (frame_end..=top).chain([container.len()]).collect()
        } else {
            vec![container.len()]
        };
        for length in lengths {
            let result = peek_outcome(&container[..length], container.len() as u64);
            match (&whole, result) {
                (Ok(a), Ok(b)) => assert_eq!(a, &b),
                (Err(a), Err(PeekError::Wire(b))) => assert_eq!(*a, b),
                (a, b) => panic!("parse {a:?} but peek {b:?}"),
            }
        }
    }
}

#[test]
fn peek_agrees_with_parse_on_mutated_containers() {
    for container in vector_containers() {
        let frame_end = 10 + header_len(&container) + 8;
        let mut variants: Vec<Vec<u8>> = Vec::new();
        // Every byte of the frame flipped, one at a time.
        for index in 0..frame_end.min(container.len()) {
            let mut bytes = container.clone();
            bytes[index] ^= 0x5a;
            variants.push(bytes);
        }
        // The container cut short by one, by two and to its frame, and with a byte added.
        for cut in [1, 2, 17] {
            variants.push(container[..container.len().saturating_sub(cut)].to_vec());
        }
        let mut longer = container.clone();
        longer.push(0);
        variants.push(longer);
        for bytes in variants {
            let whole = parse_outcome(&bytes);
            // The prefix is the frame (or all of it when the container is shorter): what a service has read.
            let prefix = &bytes[..bytes.len().min(frame_end)];
            match (whole, peek_outcome(prefix, bytes.len() as u64)) {
                (Ok(a), Ok(b)) => assert_eq!(a, b),
                (Err(a), Err(PeekError::Wire(b))) => assert_eq!(a, b, "same error"),
                // A prefix that ends before the answer is decidable is not a disagreement, but it
                // must be because the frame really is longer than the prefix.
                (Err(_), Err(PeekError::PrefixTooShort { needed })) => {
                    assert!(needed > prefix.len() as u64)
                }
                (a, b) => panic!("parse {a:?} but peek {b:?}"),
            }
        }
    }
}

#[test]
fn a_short_prefix_names_exactly_the_bytes_that_decide_it() {
    let container = snapshot_container(64);
    let frame_end = 10 + header_len(&container) + 8;
    for length in 0..frame_end {
        match peek_container(&container[..length], container.len() as u64) {
            Err(PeekError::PrefixTooShort { needed }) => {
                assert!(needed as usize > length, "needed {needed} for {length}");
                // The named length is enough to continue, and never more than the frame.
                assert!(needed as usize <= frame_end);
                if length >= 10 {
                    assert_eq!(needed as usize, frame_end);
                }
            }
            other => panic!("{length}: {other:?}"),
        }
    }
    assert!(peek_container(&container[..frame_end], container.len() as u64).is_ok());
}

#[test]
fn parse_container_names_a_wrong_magic_as_soon_as_a_wrong_byte_is_present() {
    // A whole input shorter than the magic: a byte that cannot start "DFSY" is BadMagic, and a
    // good start that simply ends is Truncated (the frame reader judges the bytes it has).
    for bytes in [&b"X"[..], b"DX", b"DFX", b"XYZ"] {
        assert_eq!(
            parse_container(bytes).err(),
            Some(WireError::BadMagic),
            "{bytes:?}"
        );
    }
    for bytes in [&b""[..], b"D", b"DF", b"DFS"] {
        assert_eq!(
            parse_container(bytes).err(),
            Some(WireError::Truncated),
            "{bytes:?}"
        );
    }
}
