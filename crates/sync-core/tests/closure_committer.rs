//! `ClosureCommitter` (personal-cfo-v98vd AC36): the C.3 closure commitment computed as the sidecar body
//! streams, holding no ids, so a service that never holds the 32 MiB closure checks the same commitment
//! the client sealed.
//!
//! Pinned: (1) the public closure-sidecar vectors, run through the committer in one chunk, byte by byte
//! (the small bodies), in fixed splits and in random splits: every accepted body gives the vector's
//! commitment and every refused body the vector's refusal; (2) a property test that the committer
//! equals `ClosureManifest::commitment` for random closures under random splits; (3) the cases a stream
//! adds: an id split across chunks at every offset, an order break across a chunk boundary, a count
//! mismatch either way, a partial id at the end, and a sticky failure.

use serde_json::Value;
use sync_core::artifact::{
    ClosureCommitter, ClosureManifest, ClosureSidecarError, MAX_CLOSURE_SIDECAR_LEN,
};

const VECTORS: &str = include_str!("../../../docs/architecture/vectors/closure-sidecar-v1.json");

fn counted_id(i: u64) -> [u8; 32] {
    let mut id = [0_u8; 32];
    id[..8].copy_from_slice(&i.to_be_bytes());
    id
}

fn body_of(description: &Value) -> Vec<u8> {
    if let Some(text) = description["hex"].as_str() {
        return (0..text.len() / 2)
            .map(|i| u8::from_str_radix(&text[2 * i..2 * i + 2], 16).unwrap())
            .collect();
    }
    let count: u64 = description["counted_ids"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let extra: usize = description["extra_zero_bytes"]
        .as_str()
        .map_or(0, |n| n.parse().unwrap());
    let mut bytes: Vec<u8> = (0..count).flat_map(counted_id).collect();
    bytes.resize(bytes.len() + extra, 0);
    bytes
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

fn name(error: ClosureSidecarError) -> &'static str {
    match error {
        ClosureSidecarError::TooManyIds => "TooManyIds",
        ClosureSidecarError::NotWholeIds => "NotWholeIds",
        ClosureSidecarError::NotAscending => "NotAscending",
        ClosureSidecarError::CountMismatch => "CountMismatch",
    }
}

/// The committer's answer for `body` fed in chunks of the sizes `sizes` (cycled), in the vector's terms.
fn run(body: &[u8], sizes: &[usize]) -> Value {
    let mut committer = match ClosureCommitter::for_body_len(body.len() as u64) {
        Ok(c) => c,
        Err(error) => return serde_json::json!({ "error": name(error) }),
    };
    let (mut at, mut i) = (0, 0);
    while at < body.len() {
        let size = sizes[i % sizes.len()].max(1).min(body.len() - at);
        if let Err(error) = committer.push(&body[at..at + size]) {
            return serde_json::json!({ "error": name(error) });
        }
        at += size;
        i += 1;
    }
    match committer.finish() {
        Ok(commitment) => {
            serde_json::json!({ "ok": { "count": (body.len() / 32).to_string(), "commitment": hex(commitment.as_bytes()) } })
        }
        Err(error) => serde_json::json!({ "error": name(error) }),
    }
}

/// A deterministic split sequence (xorshift), so a failure reproduces.
fn splits(seed: u64, n: usize, max: usize) -> Vec<usize> {
    let mut x = seed | 1;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x % max as u64) as usize + 1
        })
        .collect()
}

#[test]
fn every_committed_vector_is_judged_alike_whole_byte_by_byte_and_in_random_splits() {
    let file: Value = serde_json::from_str(VECTORS).unwrap();
    for case in file["cases"].as_array().unwrap() {
        let body = body_of(&case["body"]);
        let expect = &case["expect"];
        let big = body.len() > 1 << 20;
        let label = case["name"].as_str().unwrap();
        assert_eq!(&run(&body, &[usize::MAX / 2]), expect, "{label}: one chunk");
        assert_eq!(&run(&body, &[1 << 20]), expect, "{label}: 1 MiB chunks");
        assert_eq!(
            &run(&body, &[33]),
            expect,
            "{label}: 33-byte chunks (every id split)"
        );
        assert_eq!(
            &run(&body, &splits(0x9e37_79b9, 64, 100)),
            expect,
            "{label}: random splits of up to 100"
        );
        assert_eq!(
            &run(&body, &splits(0x1234_5678, 64, 5000)),
            expect,
            "{label}: random splits of up to 5000"
        );
        if !big {
            assert_eq!(&run(&body, &[1]), expect, "{label}: byte by byte");
        }
    }
}

#[test]
fn it_equals_the_commitment_of_the_closure_for_random_closures_under_random_splits() {
    let mut x = 0xdead_beef_u64;
    for round in 0..40_u64 {
        // A random strictly ascending list: gaps of random size on random bytes.
        let count = (round * 37 % 600) as usize;
        let mut ids: Vec<[u8; 32]> = Vec::new();
        let mut next = [0_u8; 32];
        for _ in 0..count {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let step = (x % 250 + 1) as u8;
            let mut carry = u16::from(step);
            for byte in next.iter_mut().rev() {
                let sum = u16::from(*byte) + carry;
                *byte = sum as u8;
                carry = sum >> 8;
                if carry == 0 {
                    break;
                }
            }
            ids.push(next);
        }
        let closure = ClosureManifest::new(ids).unwrap();
        let body = closure.to_sidecar();
        let expected = closure.commitment();
        for sizes in [
            splits(round + 1, 32, 7),
            splits(round + 99, 32, 4096),
            vec![32],
            vec![31],
            vec![33],
        ] {
            let mut committer = ClosureCommitter::for_body_len(body.len() as u64).unwrap();
            let (mut at, mut i) = (0, 0);
            while at < body.len() {
                let size = sizes[i % sizes.len()].min(body.len() - at);
                committer.push(&body[at..at + size]).unwrap();
                at += size;
                i += 1;
            }
            assert_eq!(
                committer.finish().unwrap(),
                expected,
                "round {round}, {} ids",
                closure.ids().len()
            );
        }
    }
}

#[test]
fn an_id_split_at_every_offset_is_read_whole() {
    let body: Vec<u8> = (1..=3_u64).flat_map(counted_id).collect();
    let expected = ClosureManifest::from_sidecar(&body).unwrap().commitment();
    for cut in 1..body.len() {
        let mut committer = ClosureCommitter::for_body_len(body.len() as u64).unwrap();
        committer.push(&body[..cut]).unwrap();
        committer.push(&body[cut..]).unwrap();
        assert_eq!(committer.finish().unwrap(), expected, "cut at {cut}");
    }
}

#[test]
fn an_order_break_is_found_across_a_chunk_boundary_and_inside_a_split_id() {
    let body: Vec<u8> = [counted_id(5), counted_id(5)].concat(); // a repeat
    for cut in 1..body.len() {
        let mut committer = ClosureCommitter::for_body_len(64).unwrap();
        let first = committer.push(&body[..cut]);
        let second = committer.push(&body[cut..]);
        assert_eq!(
            first.and(second),
            Err(ClosureSidecarError::NotAscending),
            "cut at {cut}"
        );
    }
    let descending: Vec<u8> = [counted_id(9), counted_id(2)].concat();
    let mut committer = ClosureCommitter::for_body_len(64).unwrap();
    committer.push(&descending[..32]).unwrap(); // the boundary falls exactly between the ids
    assert_eq!(
        committer.push(&descending[32..]),
        Err(ClosureSidecarError::NotAscending)
    );
}

#[test]
fn a_count_that_the_body_does_not_match_is_refused_either_way() {
    let ids: Vec<u8> = (1..=3_u64).flat_map(counted_id).collect();
    // Declared 3, two ids sent.
    let mut few = ClosureCommitter::new(3).unwrap();
    few.push(&ids[..64]).unwrap();
    assert_eq!(
        few.finish().map(|_| ()),
        Err(ClosureSidecarError::CountMismatch)
    );
    // Declared 2, three ids sent: refused at the third.
    let mut many = ClosureCommitter::new(2).unwrap();
    many.push(&ids[..64]).unwrap();
    assert_eq!(
        many.push(&ids[64..]),
        Err(ClosureSidecarError::CountMismatch)
    );
    // Declared 0, one id sent.
    let mut none = ClosureCommitter::new(0).unwrap();
    assert_eq!(
        none.push(&ids[..32]),
        Err(ClosureSidecarError::CountMismatch)
    );
    // The empty closure is committed to like any other.
    assert_eq!(
        ClosureCommitter::new(0).unwrap().finish().unwrap(),
        ClosureManifest::new(Vec::new()).unwrap().commitment()
    );
}

#[test]
fn a_partial_id_at_the_end_and_the_length_rules_come_first() {
    let mut committer = ClosureCommitter::new(2).unwrap();
    committer.push(&counted_id(1)).unwrap();
    committer.push(&counted_id(2)[..31]).unwrap();
    assert_eq!(
        committer.finish().map(|_| ()),
        Err(ClosureSidecarError::NotWholeIds)
    );
    assert_eq!(
        ClosureCommitter::for_body_len(31).map(|_| ()),
        Err(ClosureSidecarError::NotWholeIds)
    );
    assert_eq!(
        ClosureCommitter::for_body_len(MAX_CLOSURE_SIDECAR_LEN as u64 + 1).map(|_| ()),
        Err(ClosureSidecarError::TooManyIds)
    );
    assert_eq!(
        ClosureCommitter::for_body_len(MAX_CLOSURE_SIDECAR_LEN as u64 + 32).map(|_| ()),
        Err(ClosureSidecarError::TooManyIds)
    );
    assert!(ClosureCommitter::for_body_len(MAX_CLOSURE_SIDECAR_LEN as u64).is_ok());
    assert!(ClosureCommitter::new(1 << 20).is_ok());
    assert_eq!(
        ClosureCommitter::new((1 << 20) + 1).map(|_| ()),
        Err(ClosureSidecarError::TooManyIds)
    );
}

#[test]
fn a_failure_is_sticky() {
    let mut committer = ClosureCommitter::new(3).unwrap();
    committer.push(&counted_id(2)).unwrap();
    assert_eq!(
        committer.push(&counted_id(1)),
        Err(ClosureSidecarError::NotAscending)
    );
    // Later bytes, even good ones, do not undo it.
    assert_eq!(
        committer.push(&counted_id(9)),
        Err(ClosureSidecarError::NotAscending)
    );
    assert_eq!(
        committer.finish().map(|_| ()),
        Err(ClosureSidecarError::NotAscending)
    );
}

#[test]
fn it_holds_no_ids() {
    // A hash state, a previous id, a partial id and a few counters: nowhere near the 32 MiB closure.
    assert!(
        std::mem::size_of::<ClosureCommitter>() < 512,
        "{}",
        std::mem::size_of::<ClosureCommitter>()
    );
}
