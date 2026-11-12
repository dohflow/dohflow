//! `BindingCommitter` (personal-cfo-v98vd AC31): the C.3 binding commitment (header field 18) computed as
//! the ciphertext hashes stream, holding none of them, so a service that checks a closure of up to 2^20 ids
//! (32 MiB of hashes) never holds the list; and `binding_commitment`, which is this committer over a slice,
//! so the commitment has one definition.
//!
//! Pinned: (1) the public vectors of `vault-format.md` C.3 (the empty list and two hashes), through the
//! committer and the wrapper; (2) a reference of the preimage written out here, byte by byte, with none of
//! the crate's writer in it, against the committer at 0, 1, 4095, 4096, 4097 and 2^20 hashes under splits
//! of every kind; (3) what a stream adds: a hash split at every offset, a count mismatch either way, an
//! end inside a hash, a bound, and a sticky failure.

use sha2::{Digest, Sha256};
use sync_core::cbe::WireError;
use sync_core::envelope::{
    binding_commitment, BindingCommitter, BindingError, MAX_MANIFEST_ENTRIES,
};

const MAX_CLOSURE: usize = 1 << 20;
const LABEL: &str = "dohflow/sync-artifact-binding/v1";

/// The published value for the empty list (`artifact_binding_commitment_empty`).
const EMPTY: &str = "e30750db1b856820b86032ab3235ce315e98dec24c6e34ef76243ad2a1be7bef";
/// The published value for two hashes, `0x11` and `0x22` repeated 32 times, in that order (`artifact_binding_commitment_2`).
const TWO: &str = "92bae898ceb1bdb96c06faec67b4812accb9f066056b01debd998d00da274990";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The preimage as C.3 states it, with no use of the crate's writer: a 2-byte big-endian label length, the
/// label, a 4-byte big-endian count, then the hashes.
fn reference(hashes: &[[u8; 32]]) -> String {
    let mut bytes = (LABEL.len() as u16).to_be_bytes().to_vec();
    bytes.extend_from_slice(LABEL.as_bytes());
    bytes.extend_from_slice(&(hashes.len() as u32).to_be_bytes());
    for hash in hashes {
        bytes.extend_from_slice(hash);
    }
    hex(&Sha256::digest(&bytes))
}

/// `n` hashes of arbitrary content (a hash has no order rule), from a small xorshift so a failure repeats.
fn hashes(n: usize, seed: u64) -> Vec<[u8; 32]> {
    let mut x = seed | 1;
    let mut next = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    (0..n)
        .map(|_| {
            let mut hash = [0_u8; 32];
            for word in hash.chunks_mut(8) {
                word.copy_from_slice(&next().to_le_bytes());
            }
            hash
        })
        .collect()
}

/// Streams `hashes` through a committer in chunks of the sizes `cuts` gives, cycling.
fn stream(hashes: &[[u8; 32]], max: usize, cuts: &[usize]) -> Result<String, BindingError> {
    let mut committer = BindingCommitter::new(hashes.len() as u64, max)?;
    let bytes: Vec<u8> = hashes.iter().flatten().copied().collect();
    let (mut at, mut i) = (0, 0);
    while at < bytes.len() {
        let end = (at + cuts[i % cuts.len()]).min(bytes.len());
        committer.push(&bytes[at..end])?;
        at = end;
        i += 1;
    }
    Ok(hex(committer.finish()?.as_bytes()))
}

#[test]
fn the_public_vectors_hold_through_the_committer_and_the_wrapper() {
    assert_eq!(stream(&[], MAX_MANIFEST_ENTRIES, &[1]).unwrap(), EMPTY);
    let two = [[0x11_u8; 32], [0x22_u8; 32]];
    assert_eq!(stream(&two, MAX_MANIFEST_ENTRIES, &[1]).unwrap(), TWO);
    assert_eq!(stream(&two, MAX_MANIFEST_ENTRIES, &[64]).unwrap(), TWO);
    assert_eq!(
        hex(binding_commitment(&[], 4096).unwrap().as_bytes()),
        EMPTY
    );
    assert_eq!(hex(binding_commitment(&two, 4096).unwrap().as_bytes()), TWO);
    // The order is the caller's: the commitment is not of a set.
    let swapped = [[0x22_u8; 32], [0x11_u8; 32]];
    assert_ne!(stream(&swapped, MAX_MANIFEST_ENTRIES, &[64]).unwrap(), TWO);
}

#[test]
fn the_committer_and_the_wrapper_equal_the_written_out_preimage_at_the_bounds_under_every_split() {
    let all_splits: &[&[usize]] = &[
        &[1],
        &[7],
        &[31],
        &[32],
        &[33],
        &[1000],
        &[5, 59, 3],
        &[usize::MAX],
    ];
    for n in [0_usize, 1, 2, 4095, 4096] {
        let list = hashes(n, 0x1234_5678 + n as u64);
        let want = reference(&list);
        for cuts in all_splits {
            assert_eq!(
                stream(&list, MAX_CLOSURE, cuts).unwrap(),
                want,
                "{n} hashes, cuts {cuts:?}"
            );
        }
        assert_eq!(
            hex(binding_commitment(&list, MAX_CLOSURE).unwrap().as_bytes()),
            want,
            "{n} hashes, the wrapper"
        );
    }
    // One above an envelope's bound is a closure's: fine at 2^20, too many at 4096.
    let list = hashes(4097, 9);
    let want = reference(&list);
    assert_eq!(stream(&list, MAX_CLOSURE, &[1000]).unwrap(), want);
    assert_eq!(
        stream(&list, MAX_MANIFEST_ENTRIES, &[1000]),
        Err(BindingError::TooManyHashes)
    );
    assert!(matches!(
        binding_commitment(&list, MAX_MANIFEST_ENTRIES),
        Err(WireError::TooLong)
    ));
}

#[test]
fn a_closure_of_two_to_the_twenty_hashes_is_the_same_commitment_streamed_or_whole() {
    let list = hashes(MAX_CLOSURE, 0xfeed);
    let want = reference(&list);
    assert_eq!(stream(&list, MAX_CLOSURE, &[1 << 20]).unwrap(), want);
    assert_eq!(stream(&list, MAX_CLOSURE, &[65_537]).unwrap(), want);
    assert_eq!(
        hex(binding_commitment(&list, MAX_CLOSURE).unwrap().as_bytes()),
        want
    );
    // One more than 2^20 is refused before a hash is fed.
    assert!(matches!(
        BindingCommitter::new(MAX_CLOSURE as u64 + 1, MAX_CLOSURE),
        Err(BindingError::TooManyHashes)
    ));
}

#[test]
fn a_hash_split_at_every_offset_gives_the_same_commitment() {
    let list = hashes(3, 77);
    let bytes: Vec<u8> = list.iter().flatten().copied().collect();
    let want = reference(&list);
    for cut in 0..=bytes.len() {
        let mut committer = BindingCommitter::new(3, MAX_MANIFEST_ENTRIES).unwrap();
        committer.push(&bytes[..cut]).unwrap();
        committer.push(&bytes[cut..]).unwrap();
        assert_eq!(
            hex(committer.finish().unwrap().as_bytes()),
            want,
            "cut at {cut}"
        );
    }
}

#[test]
fn a_count_that_is_not_the_stream_is_refused_either_way() {
    let list = hashes(3, 5);
    let bytes: Vec<u8> = list.iter().flatten().copied().collect();
    // Declared 2, sent 3: the third hash is over the count, and the committer says so as it arrives.
    let mut over = BindingCommitter::new(2, 4096).unwrap();
    assert_eq!(over.push(&bytes), Err(BindingError::CountMismatch));
    // Declared 4, sent 3: the end says so.
    let mut under = BindingCommitter::new(4, 4096).unwrap();
    under.push(&bytes).unwrap();
    assert!(matches!(under.finish(), Err(BindingError::CountMismatch)));
    // Nothing sent for a declared count.
    assert!(matches!(
        BindingCommitter::new(1, 4096).unwrap().finish(),
        Err(BindingError::CountMismatch)
    ));
}

#[test]
fn a_stream_that_ends_inside_a_hash_is_not_whole() {
    let mut committer = BindingCommitter::new(1, 4096).unwrap();
    committer.push(&[7; 31]).unwrap();
    assert!(matches!(
        committer.finish(),
        Err(BindingError::NotWholeHashes)
    ));
    // An empty push changes nothing.
    let mut committer = BindingCommitter::new(0, 4096).unwrap();
    committer.push(&[]).unwrap();
    assert_eq!(hex(committer.finish().unwrap().as_bytes()), EMPTY);
}

#[test]
fn an_error_is_sticky_and_finish_answers_it() {
    let mut committer = BindingCommitter::new(1, 4096).unwrap();
    // 33 bytes is one hash and a byte of a second: the second hash is over the declared count only once it is whole.
    committer.push(&[1; 33]).unwrap();
    assert_eq!(committer.push(&[1; 31]), Err(BindingError::CountMismatch));
    // After the failure, anything pushed is refused with the same error, and so is the end.
    assert_eq!(committer.push(&[]), Err(BindingError::CountMismatch));
    assert_eq!(committer.push(&[0; 32]), Err(BindingError::CountMismatch));
    assert!(matches!(
        committer.finish(),
        Err(BindingError::CountMismatch)
    ));
}
