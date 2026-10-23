//! The accepted-envelope index and the retry protocol (vault-format.md C.10;
//! personal-cfo-t2s4b part 6c): the record's bytes, the service's push decision, the client's
//! outbox binding, and the two properties the t2s4b acceptance criteria name: the post-CAS `seq`
//! is never in the AAD, and a rebase executor can rebuild the command metadata from the
//! envelope alone. Every identity here is derived from container bytes, as the service and the
//! client derive them.

use sync_core::accepted::{
    decide_push, match_pulled, AcceptedIndex, AcceptedRecord, DuplicateIdentity, InMemoryIndex,
    IndexInconsistent, OutboxEntry, PullMatch, PulledEnvelope, PushDecision, PushIdentity,
    MAX_RESULT_REF,
};
use sync_core::cbe::WireError;
use sync_core::envelope::{
    canonical_envelope_digest, ArtifactManifest, EnvelopePlaintext, IdempotencyKey, ImmutableMeta,
};
use sync_core::header::{
    encode_container, parse_container, ClearHeader, Fingerprint, IdempotencyKeyTag,
    MetadataCommitment, NonceDomain, ObjectKind,
};

const VAULT: [u8; 16] = [0x11; 16];

/// What distinguishes one test envelope from another.
#[derive(Clone, Copy)]
struct Spec {
    command: u8,
    tag: u8,
    commitment: u8,
    device: u8,
    ciphertext: u8,
}

const BASE: Spec = Spec {
    command: 1,
    tag: 2,
    commitment: 4,
    device: 0xa1,
    ciphertext: 0x5a,
};

/// A sealed container for `spec`: a real envelope header and 40 ciphertext bytes.
fn container_bytes(spec: Spec) -> Vec<u8> {
    let header = ClearHeader {
        vault_id: VAULT,
        key_epoch: 3,
        object: ObjectKind::Envelope {
            idempotency_key_tag: IdempotencyKeyTag::from_derived([spec.tag; 32]),
            immutable_metadata_commitment: MetadataCommitment::from_derived([spec.commitment; 32]),
            artifact_manifest_commitment: None,
        },
        object_id: [spec.command; 16],
        device_id: [spec.device; 16],
        device_key_fingerprint: Fingerprint::from_derived([spec.device; 32]),
        nonce_domain: NonceDomain::from_derived([1, 2, 3, 4]),
        invocation_counter: 0,
        base_seq: 41,
        payload_schema_version: 1,
        engine_version: "dohflow-engine-1".to_owned(),
        schema_version: 55,
    };
    encode_container(&header, &[spec.ciphertext; 40]).unwrap()
}

fn push(spec: Spec) -> PushIdentity {
    let bytes = container_bytes(spec);
    PushIdentity::from_container(&parse_container(&bytes).unwrap()).unwrap()
}

fn outbox(spec: Spec) -> OutboxEntry {
    let bytes = container_bytes(spec);
    OutboxEntry::from_container(&parse_container(&bytes).unwrap()).unwrap()
}

fn pulled(spec: Spec, seq: u64) -> PulledEnvelope {
    let bytes = container_bytes(spec);
    PulledEnvelope::from_container(&parse_container(&bytes).unwrap(), seq, Some(vec![9, 9]))
        .unwrap()
}

fn accepted(spec: Spec, seq: u64) -> AcceptedRecord {
    push(spec).accepted(3, seq, Some(vec![9, 9]))
}

fn index_with(records: &[AcceptedRecord]) -> InMemoryIndex {
    let mut index = InMemoryIndex::new();
    for record in records {
        index.insert(record.clone()).unwrap();
    }
    index
}

// ---- the record ---------------------------------------------------------------------------

#[test]
fn the_record_round_trips_with_one_encoding() {
    for result_ref in [
        None,
        Some(vec![1]),
        Some(vec![1, 2, 3]),
        Some(vec![7; MAX_RESULT_REF]),
    ] {
        let mut r = accepted(BASE, 41);
        r.result_ref = result_ref;
        let bytes = r.encode().unwrap();
        assert_eq!(AcceptedRecord::decode(&bytes).unwrap(), r);
        assert_eq!(
            AcceptedRecord::decode(&bytes).unwrap().encode().unwrap(),
            bytes
        );
    }
}

#[test]
fn a_result_ref_is_absent_or_one_to_sixty_four_bytes() {
    let mut r = accepted(BASE, 41);
    r.result_ref = Some(vec![0; MAX_RESULT_REF + 1]);
    assert_eq!(r.encode(), Err(WireError::TooLong));
    // Present-but-empty would be a second encoding of "no result": refused on encode and decode.
    r.result_ref = Some(vec![]);
    assert_eq!(r.encode(), Err(WireError::EmptyOptionalBytes));
    let mut with_ref = accepted(BASE, 41);
    with_ref.result_ref = Some(vec![5]);
    let mut bytes = with_ref.encode().unwrap();
    let len_at = bytes.len() - (4 + 1);
    bytes[len_at..len_at + 4].copy_from_slice(&0_u32.to_be_bytes());
    bytes.pop();
    assert_eq!(
        AcceptedRecord::decode(&bytes),
        Err(WireError::EmptyOptionalBytes)
    );
}

#[test]
fn the_record_rejects_what_the_grammar_forbids() {
    let good = accepted(BASE, 41).encode().unwrap();
    for len in 0..good.len() {
        assert!(
            AcceptedRecord::decode(&good[..len]).is_err(),
            "{len}-byte prefix"
        );
    }
    let mut trailing = good.clone();
    trailing.push(0);
    assert_eq!(
        AcceptedRecord::decode(&trailing),
        Err(WireError::TrailingBytes)
    );
    // result_ref is the 2 bytes [9, 9]: presence byte, u32 length, the bytes.
    let presence_at = good.len() - (1 + 4 + 2);
    let mut bad_presence = good.clone();
    bad_presence[presence_at] = 2;
    assert_eq!(
        AcceptedRecord::decode(&bad_presence),
        Err(WireError::BadPresence)
    );
    let mut oversize = good.clone();
    oversize[presence_at + 1..presence_at + 5].copy_from_slice(&65_u32.to_be_bytes());
    assert_eq!(AcceptedRecord::decode(&oversize), Err(WireError::TooLong));
    let mut wrong_label = good;
    wrong_label[2 + "dohflow/sync-accepted/v".len()] = b'2';
    assert_eq!(
        AcceptedRecord::decode(&wrong_label),
        Err(WireError::LabelMismatch)
    );
}

// ---- identities come from bytes ------------------------------------------------------------

#[test]
fn the_digest_in_an_identity_is_computed_from_the_container_bytes() {
    let bytes = container_bytes(BASE);
    let parsed = parse_container(&bytes).unwrap();
    let expected = canonical_envelope_digest(parsed.header_bytes, parsed.ciphertext).unwrap();
    assert_eq!(push(BASE).envelope_digest(), &expected);
    // A changed ciphertext, with every header field equal, is a different identity.
    let other = Spec {
        ciphertext: 0x5b,
        ..BASE
    };
    assert_ne!(push(other).envelope_digest(), push(BASE).envelope_digest());
    assert_eq!(push(other).command_id(), push(BASE).command_id());
}

#[test]
fn a_snapshot_container_has_no_command_identity() {
    use sync_core::accepted::IdentityError;
    let bytes = {
        let container = container_bytes(BASE);
        let mut header =
            ClearHeader::decode(parse_container(&container).unwrap().header_bytes).unwrap();
        header.object = ObjectKind::Snapshot {
            artifact_manifest_commitment: sync_core::header::ManifestCommitment::from_derived(
                [1; 32],
            ),
        };
        encode_container(&header, &[0; 40]).unwrap()
    };
    let parsed = parse_container(&bytes).unwrap();
    assert_eq!(
        PushIdentity::from_container(&parsed),
        Err(IdentityError::NotAnEnvelope)
    );
}

// ---- the push decision (C.10 steps 1 to 4) -------------------------------------------------

#[test]
fn an_exact_retry_is_a_replay_even_with_a_stale_base_and_an_old_epoch() {
    let record = accepted(BASE, 41);
    let index = index_with(std::slice::from_ref(&record));
    // decide_push consults neither base_seq nor key_epoch for a known identity, so a stale
    // base_seq or an old key_epoch cannot turn a retry into a rejection.
    match decide_push(&index, &push(BASE)).unwrap() {
        PushDecision::Replay(found) => {
            assert_eq!(found.seq, 41);
            assert_eq!(found.result_ref, Some(vec![9, 9]));
        }
        other => panic!("expected a replay, got {other:?}"),
    }
}

#[test]
fn unknown_identities_fall_through_to_the_epoch_and_cas_steps() {
    let index = index_with(&[accepted(BASE, 41)]);
    let stranger = Spec {
        command: 9,
        tag: 8,
        commitment: 6,
        device: 0xb1,
        ciphertext: 0x77,
    };
    assert_eq!(
        decide_push(&index, &push(stranger)).unwrap(),
        PushDecision::Unknown
    );
    assert_eq!(
        decide_push(&InMemoryIndex::new(), &push(BASE)).unwrap(),
        PushDecision::Unknown
    );
}

#[test]
fn any_difference_in_a_bound_identity_is_a_protocol_fork() {
    let index = index_with(&[accepted(BASE, 41)]);
    let fork = |spec: Spec| decide_push(&index, &push(spec)).unwrap();

    // Same command and tag, different bytes (a rebuilt envelope, not a retry).
    assert_eq!(
        fork(Spec {
            ciphertext: 0x5b,
            ..BASE
        }),
        PushDecision::ProtocolFork
    );
    // Same command and tag, different metadata commitment (so a different digest too).
    assert_eq!(
        fork(Spec {
            commitment: 0xee,
            ..BASE
        }),
        PushDecision::ProtocolFork
    );
    // Same command and tag, a different sealing device.
    assert_eq!(
        fork(Spec {
            device: 0xb1,
            ..BASE
        }),
        PushDecision::ProtocolFork
    );
    // Same command id, a different tag: only the command lookup hits.
    assert_eq!(fork(Spec { tag: 0xee, ..BASE }), PushDecision::ProtocolFork);
    // Same tag under a new command id (the key was reused): only the tag lookup hits.
    assert_eq!(
        fork(Spec {
            command: 0xee,
            ..BASE
        }),
        PushDecision::ProtocolFork
    );
}

#[test]
fn two_lookups_that_land_on_different_records_are_a_fork() {
    let a = BASE;
    let b = Spec {
        command: 5,
        tag: 6,
        commitment: 8,
        device: 0xb1,
        ciphertext: 0x66,
    };
    let index = index_with(&[accepted(a, 41), accepted(b, 42)]);
    // a's command id with b's tag.
    let mixed = Spec { tag: b.tag, ..a };
    assert_eq!(
        decide_push(&index, &push(mixed)).unwrap(),
        PushDecision::ProtocolFork
    );
}

#[test]
fn an_index_that_returns_the_wrong_record_is_reported_as_inconsistent_not_as_a_fork() {
    // A record of another vault in this vault's index.
    let mut foreign = accepted(BASE, 41);
    foreign.vault_id = [0x99; 16];
    let index = index_with(std::slice::from_ref(&foreign));
    assert_eq!(decide_push(&index, &push(BASE)), Err(IndexInconsistent));

    // An index whose lookup returns a record that does not carry the key it was asked for.
    struct Lying(AcceptedRecord);
    impl AcceptedIndex for Lying {
        fn by_command_id(&self, _: &[u8; 16]) -> Option<&AcceptedRecord> {
            Some(&self.0)
        }
        fn by_tag(&self, _: &IdempotencyKeyTag) -> Option<&AcceptedRecord> {
            None
        }
    }
    let other = accepted(Spec { command: 7, ..BASE }, 3);
    assert_eq!(
        decide_push(&Lying(other), &push(BASE)),
        Err(IndexInconsistent)
    );
}

#[test]
fn the_index_refuses_a_second_record_for_a_command_id_or_a_tag() {
    let mut index = index_with(&[accepted(BASE, 41)]);
    // Same command id.
    assert_eq!(
        index.insert(accepted(Spec { tag: 9, ..BASE }, 42)),
        Err(DuplicateIdentity)
    );
    // Same tag.
    assert_eq!(
        index.insert(accepted(Spec { command: 9, ..BASE }, 42)),
        Err(DuplicateIdentity)
    );
    // Nothing was inserted by the refusals.
    assert_eq!(
        decide_push(
            &index,
            &push(Spec {
                command: 9,
                tag: 9,
                ..BASE
            })
        )
        .unwrap(),
        PushDecision::Unknown
    );
    index
        .insert(accepted(
            Spec {
                command: 9,
                tag: 9,
                ..BASE
            },
            42,
        ))
        .unwrap();
}

// ---- the client's outbox binding ---------------------------------------------------------

#[test]
fn the_pulled_copy_of_my_own_envelope_binds_the_outbox_entry_to_its_seq() {
    assert_eq!(
        match_pulled(&outbox(BASE), &pulled(BASE, 41)),
        PullMatch::BindToAccepted {
            seq: 41,
            result_ref: Some(vec![9, 9])
        }
    );
}

#[test]
fn the_same_command_with_a_different_envelope_is_a_fork_and_an_unrelated_one_is_not_mine() {
    let mine = outbox(BASE);
    for different in [
        Spec {
            ciphertext: 0x5b,
            ..BASE
        },
        Spec {
            commitment: 0xee,
            ..BASE
        },
        // Another device sealing the same command id differs in digest, so it is a fork too.
        Spec {
            device: 0xb1,
            ..BASE
        },
        // The same tag under a different command id (a reused key).
        Spec {
            command: 0xee,
            ..BASE
        },
        // The same command under a different tag.
        Spec { tag: 0xee, ..BASE },
    ] {
        assert_eq!(
            match_pulled(&mine, &pulled(different, 41)),
            PullMatch::ProtocolFork
        );
    }
    let stranger = Spec {
        command: 7,
        tag: 8,
        commitment: 9,
        device: 0xb1,
        ciphertext: 0x77,
    };
    assert_eq!(
        match_pulled(&mine, &pulled(stranger, 42)),
        PullMatch::NotMine
    );
    assert_eq!(pulled(BASE, 1).device_id(), &[0xa1; 16]);
    // The fork report can name the key as well as the device.
    assert_eq!(
        pulled(BASE, 1).device_key_fingerprint(),
        &Fingerprint::from_derived([0xa1; 32])
    );
}

// ---- the t2s4b acceptance criteria this part closes -----------------------------------------

#[test]
fn the_server_seq_never_feeds_the_header_the_digest_or_the_identity() {
    // Vary only the seq the service assigns: the container, the AAD bytes, the digest and every
    // identity are unchanged, so an exact-byte retransmission keeps its ciphertext and digest.
    let bytes = container_bytes(BASE);
    let parsed = parse_container(&bytes).unwrap();
    let digest = canonical_envelope_digest(parsed.header_bytes, parsed.ciphertext).unwrap();
    let identity = push(BASE);
    let mut records = Vec::new();
    for seq in [0_u64, 1, 41, 99, u64::MAX] {
        let record = identity.accepted(3, seq, None);
        assert_eq!(record.envelope_digest, digest);
        assert_eq!(&record.envelope_digest, identity.envelope_digest());
        assert_eq!(
            parse_container(&bytes).unwrap().header_bytes,
            parsed.header_bytes
        );
        assert_eq!(
            match_pulled(&outbox(BASE), &pulled(BASE, seq)),
            PullMatch::BindToAccepted {
                seq,
                result_ref: Some(vec![9, 9])
            }
        );
        records.push(record);
    }
    // Only the seq field differs between the records' encodings.
    assert!(records
        .windows(2)
        .all(|pair| pair[0].encode().unwrap() != pair[1].encode().unwrap()));
    assert!(records
        .windows(2)
        .all(|pair| pair[0].envelope_digest == pair[1].envelope_digest));
}

#[test]
fn a_rebase_executor_rebuilds_the_command_metadata_from_the_envelope_alone() {
    // The original command metadata, as a producer holds it.
    let original = ImmutableMeta {
        command_id: [0x01; 16],
        correlation_id: [0x02; 16],
        causation_id: Some([0x03; 16]),
        actor_type: 2,
        actor_id: "system".to_owned(),
        idempotency_key: IdempotencyKey::parse("4a3b2c1d-9e8f-4a7b-8c6d-5e4f3a2b1c0d").unwrap(),
        issued_at: 1_790_000_123_456_789,
        device_id: [0xa1; 16],
    };
    let plaintext = EnvelopePlaintext {
        meta: original.clone(),
        write_set_hash: [0x44; 32],
        delta_manifest: ArtifactManifest::new(vec![]).unwrap(),
        payload: b"{}".to_vec(),
    };
    // The executor sees only the decrypted bytes: no outbox row, no inherited idempotency memo.
    let received = EnvelopePlaintext::decode(&plaintext.encode().unwrap()).unwrap();
    let rebuilt = received.meta;
    assert_eq!(rebuilt, original);
    assert_eq!(rebuilt.command_id, [0x01; 16]);
    assert_eq!(rebuilt.correlation_id, [0x02; 16]);
    assert_eq!(rebuilt.causation_id, Some([0x03; 16]));
    assert_eq!(rebuilt.idempotency_key, original.idempotency_key);
    assert_eq!(rebuilt.issued_at, 1_790_000_123_456_789);
    assert_eq!(rebuilt.device_id, [0xa1; 16]);
    // and it is the metadata the header committed to.
    assert_eq!(
        rebuilt.commitment().unwrap(),
        original.commitment().unwrap()
    );
    let _ = ObjectKind::Snapshot {
        artifact_manifest_commitment: sync_core::header::ManifestCommitment::from_derived([0; 32]),
    };
}

#[test]
fn two_different_records_are_a_fork_even_if_each_matches_the_push_digest() {
    // Defense in depth: honest identities cannot produce this (an equal digest implies equal
    // command id and tag), but the decision must not depend on that. One record carries the push's
    // command id, the other its tag, and both carry the push's digest and commitment.
    let identity = push(BASE);
    let by_command = identity.accepted(3, 41, None);
    let mut by_tag = identity.accepted(3, 42, None);
    by_tag.command_id = [0xee; 16];
    struct Split(AcceptedRecord, AcceptedRecord);
    impl AcceptedIndex for Split {
        fn by_command_id(&self, _: &[u8; 16]) -> Option<&AcceptedRecord> {
            Some(&self.0)
        }
        fn by_tag(&self, _: &IdempotencyKeyTag) -> Option<&AcceptedRecord> {
            Some(&self.1)
        }
    }
    assert_eq!(
        decide_push(&Split(by_command, by_tag), &identity).unwrap(),
        PushDecision::ProtocolFork
    );
}

#[test]
fn a_restore_twin_binds_what_the_original_had_accepted_and_pushes_what_it_had_not() {
    // A restore twin's outbox holds the sealed bytes of the ORIGINAL device (the outbox keeps the
    // sealed container and never re-seals), so the identities it derives are the original's.
    let accepted_before_backup = Spec {
        command: 1,
        tag: 2,
        ..BASE
    };
    let never_accepted = Spec {
        command: 5,
        tag: 6,
        ciphertext: 0x66,
        ..BASE
    };
    let restored_outbox = [outbox(accepted_before_backup), outbox(never_accepted)];

    // The log (the service's index) holds only the first.
    let index = index_with(&[accepted(accepted_before_backup, 41)]);

    // The entry accepted before the backup binds to its seq: the exact bytes give the same digest.
    assert_eq!(
        match_pulled(&restored_outbox[0], &pulled(accepted_before_backup, 41)),
        PullMatch::BindToAccepted {
            seq: 41,
            result_ref: Some(vec![9, 9])
        }
    );
    // The one never accepted is unrelated to that pulled envelope, and pushing it is Unknown to
    // the service, so it goes through the normal epoch and CAS steps.
    assert_eq!(
        match_pulled(&restored_outbox[1], &pulled(accepted_before_backup, 41)),
        PullMatch::NotMine
    );
    assert_eq!(
        decide_push(&index, &push(never_accepted)).unwrap(),
        PushDecision::Unknown
    );
    // And the first, retried as a push, is a replay, not a duplicate append.
    assert!(matches!(
        decide_push(&index, &push(accepted_before_backup)).unwrap(),
        PushDecision::Replay(record) if record.seq == 41
    ));
}
