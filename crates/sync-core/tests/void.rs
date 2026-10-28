//! The void record and its honor rule (personal-cfo-g3m.17 part C; ADR 0074 Addendum D; C.10).

mod common;

use common::*;
use sync_core::cbe::WireError;
use sync_core::envelope::ReceiverError;
use sync_core::membership::EpochAtPosition;
use sync_core::status::{InvalidEnvelope, SyncEvent, SyncStatus};
use sync_core::void::{
    verify_void, ForkSuspected, Honor, HonorContext, HonoredVoid, Inert, VerifiedVoid, VoidBody,
    VoidError, VoidLedger, VoidReason,
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
        reason: VoidReason::DeterministicApply,
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
    for unknown in [0_u8, 3, 255] {
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
    // Offering it again at the same position is idempotent.
    assert_eq!(ledger.offer(&v, &ctx(&status, 5)), Honor::Honored(expected));
    assert_eq!(ledger.honored().len(), 1);
}

#[test]
fn the_reason_byte_plays_no_part_in_honoring() {
    let c = chain();
    let apply = VoidBody {
        reason: VoidReason::DeterministicApply,
        ..body(&c.a, 5)
    };
    let v = verify_void(&c.state, &at_end(&c), 6, &signed(&apply, &c.a)).unwrap();
    let status = stopped_at(5);
    assert!(matches!(
        VoidLedger::new().offer(&v, &ctx(&status, 5)),
        Honor::Honored(_)
    ));
}

#[test]
fn each_failing_condition_leaves_the_void_inert_in_the_pinned_order() {
    let c = chain();
    let healthy = SyncStatus::Healthy;
    let stop5 = stopped_at(5);
    let stop9 = stopped_at(9);
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
        snapshot_root: Some(5),
        ..ctx(status, 5)
    };
    let v = |position| void_at(&c, 5, position);

    let cases: Vec<(&str, VerifiedVoid, HonorContext<'_>, Inert)> = vec![
        ("V == seq", v(5), ctx(&stop5, 5), Inert::VoidNotAfterTarget),
        ("V < seq", v(4), ctx(&stop5, 5), Inert::VoidNotAfterTarget),
        (
            "below the root",
            v(9),
            with_root(&stop5),
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
    // A second void naming a position inside the honored range is inert, wherever the replica is.
    let stop8 = stopped_at(8);
    assert_eq!(
        ledger.offer(&void_at(&c, 8, 21), &ctx(&stop8, 8)),
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
    assert_eq!(
        ledger.offer(&moved, &ctx(&stop, 5)),
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
