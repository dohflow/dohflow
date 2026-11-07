//! The snapshot closure sidecar body (personal-cfo-v98vd AC30; sync-service-protocol 4.14): a
//! snapshot's full closure of up to 2^20 artifact ids, sent as their raw bytes in ascending order.
//!
//! Pinned here: (1) a public vector file, `docs/architecture/vectors/closure-sidecar-v1.json`, that a
//! second implementation must pass: the empty closure, one id, the envelope delta's bound (4096) and
//! one above it, the closure's bound (2^20 ids, 32 MiB) and one above it, a length that is not whole
//! ids, and a body that is out of order or repeats an id; (2) that the closure commitment of a body is
//! **exactly** `ClosureManifest::commitment` of the ids it carries, at the bound, so the check a
//! service makes against a snapshot's header (field 17) is the one the client made when it sealed it;
//! (3) that a body is never put in order for the sender.
//!
//! Regenerate the vectors with `DOHFLOW_UPDATE_VECTORS=1 cargo test -p sync-core --test closure_sidecar`.

use serde_json::{json, Value};
use sync_core::artifact::{
    ClosureManifest, ClosureSidecarError, CLOSURE_SIDECAR_ID_LEN, MAX_CLOSURE_SIDECAR_LEN,
    MAX_SNAPSHOT_CLOSURE,
};
use sync_core::envelope::ArtifactManifest;

const VECTORS: &str = include_str!("../../../docs/architecture/vectors/closure-sidecar-v1.json");
const VECTORS_PATH: &str = "../../docs/architecture/vectors/closure-sidecar-v1.json";

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

/// Id `i` (0-based): `i` as a big-endian `u64` in the first eight bytes, the rest zero. Ascending in `i`.
fn counted_id(i: u64) -> [u8; 32] {
    let mut id = [0_u8; 32];
    id[..8].copy_from_slice(&i.to_be_bytes());
    id
}

fn counted_body(count: usize) -> Vec<u8> {
    (0..count as u64).flat_map(counted_id).collect()
}

fn name(error: ClosureSidecarError) -> &'static str {
    match error {
        ClosureSidecarError::TooManyIds => "TooManyIds",
        ClosureSidecarError::NotWholeIds => "NotWholeIds",
        ClosureSidecarError::NotAscending => "NotAscending",
    }
}

fn outcome(body: &[u8]) -> Value {
    match ClosureManifest::from_sidecar(body) {
        Ok(closure) => {
            json!({ "ok": { "count": closure.ids().len().to_string(), "commitment": hex(closure.commitment().as_bytes()) } })
        }
        Err(error) => json!({ "error": name(error) }),
    }
}

struct Case {
    name: &'static str,
    description: &'static str,
    /// How the body is described in the file.
    body: Value,
    bytes: Vec<u8>,
}

fn counted(name: &'static str, description: &'static str, count: usize) -> Case {
    Case {
        name,
        description,
        body: json!({ "counted_ids": count.to_string() }),
        bytes: counted_body(count),
    }
}

fn literal(name: &'static str, description: &'static str, bytes: Vec<u8>) -> Case {
    Case {
        name,
        description,
        body: json!({ "hex": hex(&bytes) }),
        bytes,
    }
}

fn all_cases() -> Vec<Case> {
    let a = counted_id(7);
    let b = counted_id(9);
    let mut cases = vec![
        literal("empty", "An empty body is the empty closure, which has a commitment like any other.", Vec::new()),
        literal("one_id", "One id.", a.to_vec()),
        literal("two_ids_ascending", "Two ids in ascending order.", [a, b].concat()),
        counted("the_delta_bound", "4096 ids: the most an envelope's delta holds, and fine for a closure.", 4096),
        counted("one_above_the_delta_bound", "4097 ids: more than a delta can hold; a closure may.", 4097),
        counted("the_closure_bound", "2^20 ids (33,554,432 bytes): the most a closure holds.", MAX_SNAPSHOT_CLOSURE),
        Case {
            name: "one_above_the_closure_bound",
            description: "2^20 + 1 ids (33,554,464 bytes): refused by the length alone, before any id is read.",
            body: json!({ "counted_ids": (MAX_SNAPSHOT_CLOSURE + 1).to_string() }),
            bytes: counted_body(MAX_SNAPSHOT_CLOSURE + 1),
        },
        Case {
            name: "one_byte_above_the_closure_bound",
            description: "2^20 ids and one more byte: the length is over the bound, so it is TooManyIds and not NotWholeIds.",
            body: json!({ "counted_ids": MAX_SNAPSHOT_CLOSURE.to_string(), "extra_zero_bytes": "1" }),
            bytes: [counted_body(MAX_SNAPSHOT_CLOSURE), vec![0]].concat(),
        },
        literal("thirty_one_bytes", "One byte short of an id.", vec![1; CLOSURE_SIDECAR_ID_LEN - 1]),
        literal("thirty_three_bytes", "An id and one stray byte.", [a.to_vec(), vec![0]].concat()),
        literal("a_repeated_id", "The same id twice: not strictly ascending.", [a, a].concat()),
        literal("descending", "Two ids in descending order: refused, never put in order for the sender.", [b, a].concat()),
        literal("the_third_id_is_out_of_order", "Ascending, ascending, then a step down.", [counted_id(1), counted_id(5), counted_id(3)].concat()),
        literal("all_ones_then_zero", "The largest id, then the smallest.", [[0xff; 32], [0; 32]].concat()),
    ];
    // A body that is both not whole and out of order is judged by its length first.
    cases.push(literal(
        "length_is_judged_before_order",
        "Out of order and 33 bytes: NotWholeIds, because the length is checked first.",
        [b.to_vec(), vec![0]].concat(),
    ));
    cases
}

fn document() -> Value {
    let cases: Vec<Value> = all_cases()
        .iter()
        .map(|case| {
            json!({
                "name": case.name,
                "description": case.description,
                "body": case.body,
                "expect": outcome(&case.bytes),
            })
        })
        .collect();
    json!({
        "vectors_version": 1,
        "spec": "docs/architecture/sync-service-protocol.md 4.14 and 7.1; docs/architecture/vault-format.md C.3 and C.4 (field 17); personal-cfo-v98vd AC30",
        "synthetic": "Every id is a synthetic counter. No key, secret or real value is in this file.",
        "how_to_read": "A case gives a closure sidecar body and what a server must make of it. 'body' is {hex} (the bytes) or {counted_ids: N, extra_zero_bytes?: M}: the body of N ids followed by M zero bytes, where id i (0-based) is i as a big-endian 64-bit integer in its first eight bytes and zero in the other twenty-four, so the ids ascend. 'expect' is {ok: {count, commitment}}, where 'commitment' is the hex of the closure commitment of C.3 (SHA-256 of the label 'dohflow/sync-artifact-manifest/v1' and the ids as a CBE1 list) over the ids in the body, which a server compares with the snapshot header's field 17; or {error: name}: TooManyIds (the body is longer than 2^20 * 32 bytes; the protocol's SnapshotTooLarge), NotWholeIds (the length is not a multiple of 32) or NotAscending (an id is not greater than the one before it, which includes a repeat); the last two are the protocol's MalformedObject. The length is judged first, then order. The body is never sorted for the sender.",
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

/// Rebuilds a body from its description in the file, as a second implementation would.
fn body_of(description: &Value) -> Vec<u8> {
    if let Some(text) = description["hex"].as_str() {
        return (0..text.len() / 2)
            .map(|i| u8::from_str_radix(&text[2 * i..2 * i + 2], 16).unwrap())
            .collect();
    }
    let count: usize = description["counted_ids"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let extra: usize = description["extra_zero_bytes"]
        .as_str()
        .map_or(0, |n| n.parse().unwrap());
    let mut bytes = counted_body(count);
    bytes.resize(bytes.len() + extra, 0);
    bytes
}

#[test]
fn from_sidecar_gives_every_committed_vector_its_outcome() {
    let file: Value = serde_json::from_str(VECTORS).unwrap();
    let cases = file["cases"].as_array().unwrap();
    assert!(cases.len() >= 14);
    for case in cases {
        assert_eq!(
            outcome(&body_of(&case["body"])),
            case["expect"],
            "{}",
            case["name"]
        );
    }
}

#[test]
fn the_bound_is_2_to_the_20_ids_of_32_bytes_and_one_above_it_is_too_many() {
    assert_eq!(MAX_CLOSURE_SIDECAR_LEN, 33_554_432);
    let at_bound = counted_body(MAX_SNAPSHOT_CLOSURE);
    assert_eq!(at_bound.len(), MAX_CLOSURE_SIDECAR_LEN);
    assert_eq!(
        ClosureManifest::from_sidecar(&at_bound)
            .unwrap()
            .ids()
            .len(),
        MAX_SNAPSHOT_CLOSURE
    );
    let above = counted_body(MAX_SNAPSHOT_CLOSURE + 1);
    assert_eq!(
        ClosureManifest::from_sidecar(&above),
        Err(ClosureSidecarError::TooManyIds)
    );
    // The length alone decides: a body of the wrong length is refused without being read as ids.
    let mut ragged = at_bound;
    ragged.push(0);
    assert_eq!(
        ClosureManifest::from_sidecar(&ragged),
        Err(ClosureSidecarError::TooManyIds)
    );
}

#[test]
fn the_commitment_of_a_body_is_the_commitment_of_its_closure_at_every_size_that_matters() {
    // The check a service makes against the snapshot header is the commitment the client sealed: the same
    // function of the same ids, whether the closure is one id, the delta's bound, or the closure's.
    for count in [0, 1, 4095, 4096, 4097, 65_536, MAX_SNAPSHOT_CLOSURE] {
        let ids: Vec<[u8; 32]> = (0..count as u64).map(counted_id).collect();
        let closure = ClosureManifest::new(ids).unwrap();
        let received = ClosureManifest::from_sidecar(&closure.to_sidecar()).unwrap();
        assert_eq!(received, closure, "{count} ids round-trip");
        assert_eq!(
            received.commitment(),
            closure.commitment(),
            "{count} ids commit alike"
        );
    }
    // Up to the delta's bound the closure and the delta commit to the same list alike (C.3).
    let ids: Vec<[u8; 32]> = (0..4096_u64).map(counted_id).collect();
    assert_eq!(
        ClosureManifest::from_sidecar(&ids.concat())
            .unwrap()
            .commitment(),
        ArtifactManifest::new(ids).unwrap().commitment()
    );
}

#[test]
fn a_different_list_commits_differently_so_a_substituted_body_fails_the_header_check() {
    let honest = ClosureManifest::from_sidecar(&counted_body(1000)).unwrap();
    let mut swapped = counted_body(1000);
    swapped[31] = 1; // the first id becomes a different, still ascending, id
    let other = ClosureManifest::from_sidecar(&swapped).unwrap();
    assert_ne!(honest.commitment(), other.commitment());
    let dropped = ClosureManifest::from_sidecar(&counted_body(999)).unwrap();
    assert_ne!(honest.commitment(), dropped.commitment());
}

#[test]
fn a_sidecar_is_never_sorted_for_its_sender() {
    let a = counted_id(1);
    let b = counted_id(2);
    assert!(ClosureManifest::from_sidecar(&[a, b].concat()).is_ok());
    assert_eq!(
        ClosureManifest::from_sidecar(&[b, a].concat()),
        Err(ClosureSidecarError::NotAscending)
    );
    // Whereas the constructor for a producer orders its ids, and the two agree on the result.
    assert_eq!(
        ClosureManifest::new(vec![b, a]).unwrap().to_sidecar(),
        [a, b].concat()
    );
}
