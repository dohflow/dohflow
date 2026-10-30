//! The void record and its honor rule (personal-cfo-g3m.17 part C; ADR 0074 Addendum D; C.10).

mod common;

use common::*;
use sync_core::cbe::WireError;
use sync_core::envelope::ReceiverError;
use sync_core::membership::EpochAtPosition;
use sync_core::status::HaltEvidence;
use sync_core::status::{InvalidEnvelope, SyncEvent, SyncStatus};
use sync_core::void::{
    verify_void, ForkSuspected, Honor, HonorContext, HonoredVoid, Inert, OfferError, VerifiedVoid,
    VoidBody, VoidError, VoidLedger,
};

// -- format ------------------------------------------------------------------------------------

#[test]
fn the_body_round_trips_and_its_id_is_the_digest_of_its_bytes() {
    let a = Dev::new(1);
    let b = body(&a, 7);
    let bytes = b.encode();
    assert_eq!(VoidBody::decode(&bytes).unwrap(), b);
    assert_eq!(b.id(), id_of(&bytes));
    let other = VoidBody {
        issued_at: b.issued_at + 1,
        ..b.clone()
    };
    assert_ne!(other.id(), b.id(), "every field is bound into the id");
}

#[test]
fn the_decoder_refuses_each_malformation() {
    let a = Dev::new(1);
    let bytes = body(&a, 7).encode();
    // Trailing bytes, truncation, an unknown reason byte, a wrong protocol version.
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(VoidBody::decode(&trailing).is_err());
    assert!(VoidBody::decode(&bytes[..bytes.len() - 1]).is_err());
    // The reason byte sits after label + 2 + 2 + 16 + 8 + 16 + 32 bytes.
    let reason_at = bytes.len() - 8 - 32 - 1;
    assert_eq!(bytes[reason_at], 1);
    // Reason 2 is refused until personal-cfo-g3m.26 adds its engine_version field.
    for unknown in [0_u8, 2, 3, 255] {
        let mut b = bytes.clone();
        b[reason_at] = unknown;
        assert_eq!(
            VoidBody::decode(&b),
            Err(WireError::UnknownObjectType),
            "reason {unknown}"
        );
    }
    let protocol_at = bytes.len() - (2 + 2 + 16 + 8 + 16 + 32 + 1 + 32 + 8);
    let mut b = bytes.clone();
    b[protocol_at + 1] = 9;
    assert_eq!(VoidBody::decode(&b), Err(WireError::UnsupportedProtocol));
}

// -- verification ------------------------------------------------------------------------------

#[test]
fn a_member_signed_void_verifies_and_is_bound_to_the_position_it_was_pulled_at() {
    let c = chain();
    let v = void_at(&c, 5, 9);
    assert_eq!(v.body(), &body(&c.a, 5));
    assert_eq!(v.id(), &body(&c.a, 5).id());
    assert_eq!(v.position(), 9);
}

#[test]
fn the_signer_is_judged_at_the_membership_version_of_the_voids_own_position() {
    let c = chain();
    let record = signed(&body(&c.b, 5), &c.b);
    // B joined at version 2: a void positioned in the epoch before B was added is refused...
    let before = EpochAtPosition::after_epoch_record(&c.state, &c.e1).unwrap();
    assert_eq!(
        verify_void(&c.state, &before, 9, &record),
        Err(VoidError::SignerNotAuthorized)
    );
    // ...and accepted once the position is in the epoch that installs B.
    assert!(verify_void(&c.state, &at_end(&c), 9, &record).is_ok());
}

#[test]
fn a_bad_void_is_refused_with_the_right_error() {
    let c = chain();
    let at = at_end(&c);
    let good = body(&c.a, 5);

    let wrong_key = sign(&good.encode(), &c.b.signing);
    assert_eq!(
        verify_void(&c.state, &at, 9, &wrong_key),
        Err(VoidError::BadSignature)
    );
    let mut tampered = signed(&good, &c.a);
    tampered[20] ^= 1;
    assert!(verify_void(&c.state, &at, 9, &tampered).is_err());

    let stranger = Dev::new(9);
    assert_eq!(
        verify_void(&c.state, &at, 9, &signed(&body(&stranger, 5), &stranger)),
        Err(VoidError::SignerNotAuthorized)
    );
    // The key is the chain's, never the record's: a stranger claiming A's fingerprint fails.
    let forged = sign(&good.encode(), &stranger.signing);
    assert_eq!(
        verify_void(&c.state, &at, 9, &forged),
        Err(VoidError::BadSignature)
    );

    let other_vault = VoidBody {
        vault_id: [9; 16],
        ..good.clone()
    };
    assert_eq!(
        verify_void(&c.state, &at, 9, &signed(&other_vault, &c.a)),
        Err(VoidError::WrongVault)
    );
    let other_suite = VoidBody {
        suite_id: 2,
        ..good.clone()
    };
    assert_eq!(
        verify_void(&c.state, &at, 9, &signed(&other_suite, &c.a)),
        Err(VoidError::UnsupportedSuite)
    );
    assert!(matches!(
        verify_void(&c.state, &at, 9, b"junk"),
        Err(VoidError::Malformed(_))
    ));
}

#[test]
fn a_state_advanced_past_the_stop_still_verifies_a_void_pulled_after_it() {
    // A halted or stopped replica keeps verifying chain records past its stop; the void is then
    // verified against the advanced state at the epoch in force at its own position.
    let c = chain();
    let advanced = c.state.clone();
    let v = verify_void(
        &advanced,
        &EpochAtPosition::after_epoch_record(&advanced, &c.e2).unwrap(),
        40,
        &signed(&body(&c.a, 5), &c.a),
    )
    .unwrap();
    assert_eq!(v.position(), 40);
}

// -- the honor rule ----------------------------------------------------------------------------

fn stopped_at(seq: u64) -> SyncStatus {
    SyncStatus::EnvelopeInvalid(InvalidEnvelope {
        seq,
        envelope_digest: [0x22; 32],
        command_id: [0x11; 16],
        device_id: [1; 16],
        device_key_fingerprint: Dev::new(1).fingerprint(),
        reason: ReceiverError::TagMismatch,
    })
}

fn ctx(status: &SyncStatus, next_to_apply: u64) -> HonorContext<'_> {
    HonorContext {
        status,
        next_to_apply,
        snapshot_root: None,
        target_not_determinable: false,
    }
}

#[test]
fn a_void_over_the_stop_the_replica_reached_in_order_is_honored_for_the_range_before_it() {
    let c = chain();
    let v = void_at(&c, 5, 9);
    let status = stopped_at(5);
    let mut ledger = VoidLedger::new();
    let expected = HonoredVoid {
        void_id: *v.id(),
        first: 5,
        last: 8,
    };
    assert_eq!(ledger.offer(&v, &ctx(&status, 5)), Honor::Honored(expected));
    for seq in 5..=8 {
        assert!(ledger.skips(seq));
    }
    assert!(
        !ledger.skips(4) && !ledger.skips(9),
        "the void itself applies as a log object"
    );
    // Offering it again at the same position (after a restart or a re-pull, with the cursor
    // past the range) is AlreadyHonored: it must not drive rollback and re-issue a second time.
    let healthy = SyncStatus::Healthy;
    assert_eq!(
        ledger.offer(&v, &ctx(&healthy, 9)),
        Honor::AlreadyHonored(expected)
    );
    assert_eq!(ledger.honored().len(), 1);
}

#[test]
fn each_failing_condition_leaves_the_void_inert_in_the_pinned_order() {
    let c = chain();
    let healthy = SyncStatus::Healthy;
    let stop5 = stopped_at(5);
    let stop9 = stopped_at(9);
    let halted = SyncStatus::Halted(sync_core::status::HaltEvidence::Fork {
        version: 2,
        accepted: [1; 32],
        conflicting: [2; 32],
    });
    let wrong_command = SyncStatus::EnvelopeInvalid(InvalidEnvelope {
        command_id: [0x99; 16],
        ..match stopped_at(5) {
            SyncStatus::EnvelopeInvalid(i) => i,
            _ => unreachable!(),
        }
    });
    let not_determinable = |status| HonorContext {
        target_not_determinable: true,
        ..ctx(status, 5)
    };
    let with_root = |status| HonorContext {
        snapshot_root: Some(8),
        ..ctx(status, 9)
    };
    let v = |position| void_at(&c, 5, position);

    let cases: Vec<(&str, VerifiedVoid, HonorContext<'_>, Inert)> = vec![
        ("V == seq", v(5), ctx(&stop5, 5), Inert::VoidNotAfterTarget),
        ("V < seq", v(4), ctx(&stop5, 5), Inert::VoidNotAfterTarget),
        (
            "below the root",
            v(9),
            with_root(&healthy),
            Inert::PrecedesRoot,
        ),
        ("already applied", v(9), ctx(&healthy, 6), Inert::FoundValid),
        (
            "frontier behind",
            v(9),
            ctx(&stop5, 3),
            Inert::NotYetEvaluated,
        ),
        (
            "healthy at seq",
            v(9),
            ctx(&healthy, 5),
            Inert::NotYetEvaluated,
        ),
        (
            "cannot judge",
            v(9),
            not_determinable(&healthy),
            Inert::NotDeterminable,
        ),
        (
            "cannot judge, stopped later",
            v(9),
            not_determinable(&stop9),
            Inert::NotDeterminable,
        ),
        (
            "halted does not judge",
            v(9),
            not_determinable(&halted),
            Inert::NotYetEvaluated,
        ),
        ("a later stop", v(9), ctx(&stop9, 5), Inert::NotYetEvaluated),
        (
            "another command",
            v(9),
            ctx(&wrong_command, 5),
            Inert::CommandMismatch,
        ),
    ];
    for (label, void, context, why) in cases {
        let mut ledger = VoidLedger::new();
        assert_eq!(ledger.offer(&void, &context), Honor::Inert(why), "{label}");
        assert!(ledger.honored().is_empty() && !ledger.skips(5), "{label}");
    }
    // Transient versus permanent is in the type.
    for t in [Inert::NotYetEvaluated, Inert::NotDeterminable] {
        assert!(t.is_transient());
    }
    for p in [
        Inert::VoidNotAfterTarget,
        Inert::AlreadyCovered,
        Inert::PrecedesRoot,
        Inert::FoundValid,
        Inert::CommandMismatch,
    ] {
        assert!(!p.is_transient());
    }
}

#[test]
fn a_digest_the_replica_does_not_hold_is_a_suspected_fork_never_a_silent_inert() {
    let c = chain();
    let other = VoidBody {
        envelope_digest: [0x77; 32],
        ..body(&c.a, 5)
    };
    let v = verify_void(&c.state, &at_end(&c), 9, &signed(&other, &c.a)).unwrap();
    let status = stopped_at(5);
    let mut ledger = VoidLedger::new();
    assert_eq!(
        ledger.offer(&v, &ctx(&status, 5)),
        Honor::ForkSuspected(ForkSuspected::DigestAtStop {
            seq: 5,
            ours: [0x22; 32],
            theirs: [0x77; 32],
        })
    );
    assert!(ledger.honored().is_empty());
}

#[test]
fn a_transient_void_is_honored_when_the_replica_catches_up_and_the_result_is_order_independent() {
    // The out-of-order scenario: this replica verified a poisoned 30 before 10..29 were
    // processed. The void for 30 (at 40) is NOT honored then (frontier behind), so the replica
    // never leaves its stop early; when it reaches 30 in order it honors the same void.
    let c = chain();
    let v = void_at(&c, 30, 40);
    let stop = stopped_at(30);
    let mut ledger = VoidLedger::new();
    let behind = ledger.offer(&v, &ctx(&stop, 10));
    assert_eq!(behind, Honor::Inert(Inert::NotYetEvaluated));
    assert!(matches!(behind, Honor::Inert(i) if i.is_transient()));
    assert!(ledger.honored().is_empty());
    // Frontier reaches 30: now honored, with the range 30..=39.
    assert!(matches!(
        ledger.offer(&v, &ctx(&stop, 30)),
        Honor::Honored(HonoredVoid {
            first: 30,
            last: 39,
            ..
        })
    ));
    // A replica that stopped at 10 first and only later reaches 30 builds the same ledger.
    let mut other = VoidLedger::new();
    assert_eq!(
        other.offer(&v, &ctx(&stopped_at(10), 10)),
        Honor::Inert(Inert::NotYetEvaluated)
    );
    assert!(matches!(
        other.offer(&v, &ctx(&stop, 30)),
        Honor::Honored(_)
    ));
    assert_eq!(other, ledger);
}

#[test]
fn an_inert_void_over_an_applied_range_never_blocks_a_later_genuine_void_inside_it() {
    // The V1 scenario. A void naming 3 at V=10 arrives after the replica applied 3: inert, it
    // covers nothing. A genuinely poisoned 6 is later reached and voided at 12: still honored.
    let c = chain();
    let healthy = SyncStatus::Healthy;
    let mut ledger = VoidLedger::new();
    assert_eq!(
        ledger.offer(&void_at(&c, 3, 10), &ctx(&healthy, 7)),
        Honor::Inert(Inert::FoundValid)
    );
    assert!(!ledger.skips(6), "an inert void covers nothing");
    let stop = stopped_at(6);
    assert!(matches!(
        ledger.offer(&void_at(&c, 6, 12), &ctx(&stop, 6)),
        Honor::Honored(HonoredVoid {
            first: 6,
            last: 11,
            ..
        })
    ));
}

#[test]
fn a_position_an_honored_void_covers_cannot_be_voided_again() {
    let c = chain();
    let mut ledger = VoidLedger::new();
    let stop = stopped_at(5);
    assert!(matches!(
        ledger.offer(&void_at(&c, 5, 20), &ctx(&stop, 5)),
        Honor::Honored(_)
    ));
    // A second void naming a position inside the honored range is inert: the cursor moved past
    // the range, and coverage is checked before the frontier.
    let healthy = SyncStatus::Healthy;
    assert_eq!(
        ledger.offer(&void_at(&c, 8, 21), &ctx(&healthy, 20)),
        Honor::Inert(Inert::AlreadyCovered)
    );
    assert_eq!(ledger.honored().len(), 1);
}

#[test]
fn the_same_void_at_two_positions_is_a_suspected_fork_not_idempotent() {
    let c = chain();
    let mut ledger = VoidLedger::new();
    let stop = stopped_at(5);
    assert!(matches!(
        ledger.offer(&void_at(&c, 5, 9), &ctx(&stop, 5)),
        Honor::Honored(_)
    ));
    let moved = void_at(&c, 5, 12);
    let healthy = SyncStatus::Healthy;
    assert_eq!(
        ledger.offer(&moved, &ctx(&healthy, 9)),
        Honor::ForkSuspected(ForkSuspected::VoidAtTwoPositions {
            void_id: *moved.id(),
            honored_at: 9,
            offered_at: 12,
        })
    );
    assert_eq!(ledger.honored().len(), 1);
}

#[test]
fn range_arithmetic_holds_at_the_edges_of_u64() {
    let c = chain();
    let stop = stopped_at(u64::MAX - 1);
    let mut ledger = VoidLedger::new();
    assert!(matches!(
        ledger.offer(&void_at(&c, u64::MAX - 1, u64::MAX), &ctx(&stop, u64::MAX - 1)),
        Honor::Honored(HonoredVoid { first, last, .. }) if first == u64::MAX - 1 && last == u64::MAX - 1
    ));
    let stop0 = stopped_at(0);
    assert!(matches!(
        VoidLedger::new().offer(&void_at(&c, 0, 1), &ctx(&stop0, 0)),
        Honor::Honored(HonoredVoid {
            first: 0,
            last: 0,
            ..
        })
    ));
}

// -- the ledger --------------------------------------------------------------------------------

#[test]
fn the_ledger_round_trips_and_a_damaged_one_is_refused() {
    let c = chain();
    let mut ledger = VoidLedger::new();
    for (seq, position) in [(2_u64, 4_u64), (10, 14)] {
        let stop = stopped_at(seq);
        assert!(matches!(
            ledger.offer(&void_at(&c, seq, position), &ctx(&stop, seq)),
            Honor::Honored(_)
        ));
    }
    let bytes = ledger.encode().unwrap();
    assert_eq!(VoidLedger::decode(&bytes).unwrap(), ledger);
    assert_eq!(
        VoidLedger::decode(&VoidLedger::new().encode().unwrap()).unwrap(),
        VoidLedger::new()
    );
    assert!(VoidLedger::decode(&bytes[..bytes.len() - 1]).is_err());
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(VoidLedger::decode(&trailing).is_err());

    let entry = |first: u64, last: u64| -> Vec<u8> {
        let mut e = vec![7_u8; 32];
        e.extend(first.to_be_bytes());
        e.extend(last.to_be_bytes());
        e
    };
    let build = |entries: &[Vec<u8>]| -> Vec<u8> {
        let empty = VoidLedger::new().encode().unwrap();
        let mut out = empty[..empty.len() - 4].to_vec();
        out.extend((entries.len() as u32).to_be_bytes());
        for e in entries {
            out.extend(e);
        }
        out
    };
    assert!(VoidLedger::decode(&build(&[entry(2, 3), entry(10, 14)])).is_ok());
    assert!(
        VoidLedger::decode(&build(&[entry(5, 4)])).is_err(),
        "backwards"
    );
    assert!(
        VoidLedger::decode(&build(&[entry(2, 6), entry(6, 9)])).is_err(),
        "overlap"
    );
}

// -- the status machine ------------------------------------------------------------------------

#[test]
fn an_honored_void_over_the_stop_resumes_after_the_range() {
    let honored = HonoredVoid {
        void_id: [3; 32],
        first: 5,
        last: 8,
    };
    let mut status = stopped_at(5);
    assert_eq!(
        status.observe_honored(&honored),
        Some(SyncEvent::Resumed { after: 8 })
    );
    assert!(status.is_healthy() && status.can_apply(9) && status.can_push());
}

#[test]
fn a_void_for_another_position_or_a_halted_replica_leaves_the_status_alone() {
    let honored = HonoredVoid {
        void_id: [3; 32],
        first: 5,
        last: 8,
    };
    for stop in [2, 4, 6, 9] {
        let mut status = stopped_at(stop);
        let before = status;
        assert_eq!(status.observe_honored(&honored), None);
        assert_eq!(status, before, "the stop at {stop} stays");
    }
    let mut healthy = SyncStatus::Healthy;
    assert_eq!(healthy.observe_honored(&honored), None);
    let mut halted = SyncStatus::Halted(sync_core::status::HaltEvidence::Fork {
        version: 2,
        accepted: [1; 32],
        conflicting: [2; 32],
    });
    let before = halted;
    assert_eq!(halted.observe_honored(&honored), None);
    assert_eq!(halted, before, "a void is never an answer to a fork");
}

// -- the context and the order are enforced ----------------------------------------------------

#[test]
fn an_inconsistent_context_is_refused_never_a_permanent_found_valid() {
    let c = chain();
    let v = void_at(&c, 5, 9);
    let mut ledger = VoidLedger::new();
    // The cursor advanced over chain records past the stop: refused, not Inert(FoundValid).
    let stop = stopped_at(5);
    assert_eq!(
        ledger.try_offer(&v, &ctx(&stop, 7)),
        Err(OfferError::StopBehindFrontier)
    );
    // A bootstrapped replica's cursor starts above its root.
    let healthy = SyncStatus::Healthy;
    let at_root = HonorContext {
        snapshot_root: Some(5),
        ..ctx(&healthy, 5)
    };
    assert_eq!(
        ledger.try_offer(&v, &at_root),
        Err(OfferError::RootNotBelowFrontier)
    );
    // The cursor sits inside a range an honored void already skipped.
    let stop5 = stopped_at(5);
    assert!(matches!(
        ledger.offer(&v, &ctx(&stop5, 5)),
        Honor::Honored(_)
    ));
    assert_eq!(
        ledger.try_offer(&v, &ctx(&healthy, 8)),
        Err(OfferError::FrontierInsideHonoredRange)
    );
    assert_eq!(ledger.honored().len(), 1);
}

#[test]
fn two_voids_for_one_position_are_offered_in_position_order_and_the_earlier_wins() {
    let c = chain();
    // A service that skips its same-seq dedupe serves two different voids for seq 10, at 12
    // and 15 (here they differ in `issued_at`, so their ids differ).
    let at = at_end(&c);
    let again = VoidBody {
        issued_at: 1_790_000_000_000_001,
        ..body(&c.a, 10)
    };
    let early = void_at(&c, 10, 12);
    let late = verify_void(&c.state, &at, 15, &signed(&again, &c.a)).unwrap();
    let stop = stopped_at(10);
    // Not ascending (or equal) is refused.
    let mut ledger = VoidLedger::new();
    assert_eq!(
        ledger.offer_in_order(&[late.clone(), early.clone()], &ctx(&stop, 10)),
        Err(OfferError::VoidsNotAscending)
    );
    assert_eq!(
        ledger.offer_in_order(&[early.clone(), early.clone()], &ctx(&stop, 10)),
        Err(OfferError::VoidsNotAscending)
    );
    // In order, the earlier is honored and the later is not even offered (the context is stale).
    let outcomes = ledger
        .offer_in_order(&[early.clone(), late.clone()], &ctx(&stop, 10))
        .unwrap();
    assert_eq!(outcomes.len(), 1);
    assert!(matches!(
        outcomes[0],
        Honor::Honored(HonoredVoid {
            first: 10,
            last: 11,
            ..
        })
    ));
    // After the engine moves its cursor past the range, the later one is covered: inert.
    let healthy = SyncStatus::Healthy;
    assert_eq!(
        ledger.offer(&late, &ctx(&healthy, 12)),
        Honor::Inert(Inert::AlreadyCovered)
    );
    // The very same void body served at two positions is a suspected fork instead.
    let same_again = void_at(&c, 10, 15);
    assert!(matches!(
        ledger.offer(&same_again, &ctx(&healthy, 12)),
        Honor::ForkSuspected(ForkSuspected::VoidAtTwoPositions { .. })
    ));
    // A batch of inert voids is reported in full.
    let behind = SyncStatus::Healthy;
    let all_inert = VoidLedger::new()
        .offer_in_order(&[early, late], &ctx(&behind, 3))
        .unwrap();
    assert_eq!(all_inert.len(), 2);
}

#[test]
fn the_ledger_encoding_is_unique_strictly_ascending_ranges() {
    let entry = |first: u64, last: u64| -> Vec<u8> {
        let mut e = vec![7_u8; 32];
        e.extend(first.to_be_bytes());
        e.extend(last.to_be_bytes());
        e
    };
    let build = |entries: &[Vec<u8>]| -> Vec<u8> {
        let empty = VoidLedger::new().encode().unwrap();
        let mut out = empty[..empty.len() - 4].to_vec();
        out.extend((entries.len() as u32).to_be_bytes());
        for e in entries {
            out.extend(e);
        }
        out
    };
    assert!(VoidLedger::decode(&build(&[entry(2, 3), entry(10, 14)])).is_ok());
    assert!(
        VoidLedger::decode(&build(&[entry(10, 14), entry(2, 3)])).is_err(),
        "the same set in another order is another encoding"
    );
    assert!(VoidLedger::decode(&build(&[entry(2, 3), entry(3, 5)])).is_err());
}

// -- the status machine: halting and re-verification ---------------------------------------------

#[test]
fn a_suspected_fork_halts_from_healthy_and_from_a_stop_and_a_halt_absorbs_it() {
    let digest = ForkSuspected::DigestAtStop {
        seq: 5,
        ours: [1; 32],
        theirs: [2; 32],
    };
    let two = ForkSuspected::VoidAtTwoPositions {
        void_id: [3; 32],
        honored_at: 9,
        offered_at: 12,
    };
    for start in [SyncStatus::Healthy, stopped_at(5)] {
        let mut status = start;
        assert!(matches!(
            status.observe_fork_suspected(&digest),
            Some(SyncEvent::Halted(HaltEvidence::SeqConflict {
                seq: 5,
                first,
                second,
            })) if first == [1; 32] && second == [2; 32]
        ));
        assert!(matches!(status, SyncStatus::Halted(_)) && !status.can_apply(0));
        let before = status;
        assert_eq!(
            status.observe_fork_suspected(&two),
            None,
            "a halt absorbs it"
        );
        assert_eq!(status, before);
    }
    let mut status = SyncStatus::Healthy;
    assert!(matches!(
        status.observe_fork_suspected(&two),
        Some(SyncEvent::Halted(HaltEvidence::VoidAtTwoPositions {
            honored_at: 9,
            offered_at: 12,
            ..
        }))
    ));
}

#[test]
fn re_verifying_a_skipped_or_applied_position_does_not_reopen_a_stop() {
    let c = chain();
    let mut ledger = VoidLedger::new();
    let stop = stopped_at(10);
    assert!(matches!(
        ledger.offer(&void_at(&c, 10, 20), &ctx(&stop, 10)),
        Honor::Honored(_)
    ));
    let header = sample_header();
    let mut status = SyncStatus::Healthy;
    // Position 15 is inside the honored range: re-verified as invalid, it is ignored.
    assert_eq!(
        status.observe_envelope_in_context(
            &ledger,
            20,
            15,
            [9; 32],
            &header,
            Err(ReceiverError::TagMismatch)
        ),
        None
    );
    assert!(status.is_healthy());
    // A valid result for an applied position is ignored; a FAILED one that no void skips
    // contradicts the replica's own state and halts.
    assert_eq!(
        status.observe_envelope_in_context(&ledger, 20, 7, [9; 32], &header, Ok(())),
        None
    );
    assert!(status.is_healthy());
    let mut contradicted = SyncStatus::Healthy;
    assert!(matches!(
        contradicted.observe_envelope_in_context(
            &ledger,
            20,
            7,
            [9; 32],
            &header,
            Err(ReceiverError::TagMismatch)
        ),
        Some(SyncEvent::Halted(HaltEvidence::AppliedPositionInvalid {
            seq: 7,
            ..
        }))
    ));
    assert!(matches!(contradicted, SyncStatus::Halted(_)));
    // A position at or past the frontier that no void skips still stops the replica.
    assert!(matches!(
        status.observe_envelope_in_context(
            &ledger,
            20,
            25,
            [9; 32],
            &header,
            Err(ReceiverError::TagMismatch)
        ),
        Some(SyncEvent::EnvelopeInvalid(_))
    ));
}
