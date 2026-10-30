//! The apply-failure stop, the update-required state (R40), the write-set mismatch halt and the
//! snapshot exits (personal-cfo-g3m.26; vault-format.md C.10 and C.13).

mod common;

use common::*;
use sync_core::envelope::ReceiverError;
use sync_core::membership::EpochAtPosition;
use sync_core::status::{
    ApplyFailureCode, EngineVersion, HaltEvidence, SnapshotFacts, StopReason, SyncEvent,
    SyncStatus, UpdateRequired,
};
use sync_core::void::{
    snapshot_passes_stop, verify_void, Honor, HonorContext, HonoredVoid, VoidBody, VoidLedger,
    VoidReason,
};

fn code(n: u16) -> ApplyFailureCode {
    ApplyFailureCode::new(n).unwrap()
}

fn v(text: &str) -> EngineVersion {
    EngineVersion::new(text).unwrap()
}

fn healthy_apply_stop(status: &mut SyncStatus, seq: u64, digest: u8) -> Option<SyncEvent> {
    status.observe_apply_failure(
        seq,
        [digest; 32],
        &sample_header(),
        &own_engine(),
        Err(code(7)),
    )
}

// -- values ----------------------------------------------------------------------------------------

#[test]
fn a_failure_code_is_nonzero_and_an_engine_version_is_printable_ascii_up_to_64_bytes() {
    assert!(ApplyFailureCode::new(0).is_none());
    assert_eq!(code(65_535).get(), 65_535);
    assert_eq!(v("dohflow-engine-1").as_str(), "dohflow-engine-1");
    assert!(EngineVersion::new("").is_err());
    assert!(EngineVersion::new(&"a".repeat(65)).is_err());
    assert!(EngineVersion::new(&"a".repeat(64)).is_ok());
    for bad in ["has space", "tab\t", "caf\u{e9}", "del\u{7f}"] {
        assert!(EngineVersion::new(bad).is_err(), "{bad:?}");
    }
    // Opaque and equality-only: "dohflow-engine-10" is not "newer" than "dohflow-engine-9".
    assert_ne!(v("dohflow-engine-9"), v("dohflow-engine-10"));
}

// -- the apply-failure stop ----------------------------------------------------------------------------

#[test]
fn a_deterministic_apply_failure_stops_the_replica_like_an_invalid_envelope() {
    let mut status = SyncStatus::Healthy;
    let event = healthy_apply_stop(&mut status, 12, 1);
    let Some(SyncEvent::EnvelopeInvalid(stop)) = event else {
        panic!("{event:?}")
    };
    assert_eq!(stop.reason, StopReason::Apply(code(7)));
    assert!(status.can_apply(11) && !status.can_apply(12) && !status.can_apply(13));
    assert!(!status.can_push());
    // The same position again is idempotent; another digest there is a fork of the log.
    assert_eq!(healthy_apply_stop(&mut status, 12, 1), None);
    assert!(matches!(
        healthy_apply_stop(&mut status, 12, 2),
        Some(SyncEvent::Halted(HaltEvidence::SeqConflict { seq: 12, .. }))
    ));
}

#[test]
fn the_earlier_stop_wins_whichever_kind_it_is() {
    let header = sample_header();
    let mut status = SyncStatus::Healthy;
    // A receiver-check stop at 20, then an apply stop at 15: the earlier one replaces it.
    status.observe_envelope(
        20,
        [1; 32],
        &header,
        &own_engine(),
        Err(ReceiverError::TagMismatch),
    );
    assert!(matches!(
        healthy_apply_stop(&mut status, 15, 2),
        Some(SyncEvent::EnvelopeInvalid(_))
    ));
    assert!(matches!(
        status,
        SyncStatus::EnvelopeInvalid(s) if s.seq == 15 && matches!(s.reason, StopReason::Apply(_))
    ));
    // A later receiver-check stop does not displace it.
    assert_eq!(
        status.observe_envelope(
            30,
            [3; 32],
            &header,
            &own_engine(),
            Err(ReceiverError::TagMismatch)
        ),
        None
    );
    // An Ok outcome records nothing.
    let mut fresh = SyncStatus::Healthy;
    assert_eq!(
        fresh.observe_apply_failure(5, [1; 32], &header, &own_engine(), Ok(())),
        None
    );
    assert!(fresh.is_healthy());
}

#[test]
fn a_reason_2_void_is_honored_at_an_apply_stop_and_a_reason_1_void_too() {
    let c = chain();
    let at = EpochAtPosition::after_epoch_record(&c.state, &c.e2).unwrap();
    for reason in [VoidReason::DeterministicApply, VoidReason::ReceiverCheck] {
        let mut status = SyncStatus::Healthy;
        // The stop carries the command the sample header names and the digest the void names.
        status.observe_apply_failure(5, [0x22; 32], &sample_header(), &own_engine(), Err(code(3)));
        let body = VoidBody {
            reason,
            ..body(&c.a, 5)
        };
        let void = verify_void(&c.state, &at, 9, &signed(&body, &c.a)).unwrap();
        let mut ledger = VoidLedger::new();
        let outcome = ledger
            .offer_in_order(
                std::slice::from_ref(&void),
                &HonorContext {
                    status: &status,
                    next_to_apply: 5,
                    snapshot_root: None,
                    target_not_determinable: false,
                },
            )
            .unwrap();
        assert!(
            matches!(
                outcome[0],
                Honor::Honored(HonoredVoid {
                    first: 5,
                    last: 8,
                    ..
                })
            ),
            "{reason:?}: {outcome:?}"
        );
    }
}

#[test]
fn a_replica_of_another_engine_version_cannot_judge_a_reason_2_void_it_is_update_required() {
    let c = chain();
    let at = EpochAtPosition::after_epoch_record(&c.state, &c.e2).unwrap();
    // The envelope at 5 carries engine version 1; this build is 2: R40, no verdict formed.
    let mut status = SyncStatus::Healthy;
    status.observe_engine_version(5, [0x22; 32], v("dohflow-engine-1"), v("dohflow-engine-2"));
    assert!(matches!(status, SyncStatus::UpdateRequired(_)));
    let body = VoidBody {
        reason: VoidReason::DeterministicApply,
        ..body(&c.a, 5)
    };
    let void = verify_void(&c.state, &at, 9, &signed(&body, &c.a)).unwrap();
    let mut ledger = VoidLedger::new();
    let outcome = ledger
        .offer_in_order(
            &[void],
            &HonorContext {
                status: &status,
                next_to_apply: 5,
                snapshot_root: None,
                target_not_determinable: false,
            },
        )
        .unwrap();
    // Transient, not permanent: the exits below (or a build of version 1) change the answer.
    assert!(
        matches!(outcome[0], Honor::Inert(why) if why.is_transient()),
        "{outcome:?}"
    );
    assert!(ledger.honored().is_empty());
}

// -- update-required (R40) -----------------------------------------------------------------------------

#[test]
fn an_envelope_of_another_engine_version_stops_the_replica_without_a_verdict() {
    let mut status = SyncStatus::Healthy;
    // An equal version changes nothing.
    assert_eq!(
        status.observe_engine_version(5, [1; 32], v("dohflow-engine-1"), v("dohflow-engine-1")),
        None
    );
    let event =
        status.observe_engine_version(5, [1; 32], v("dohflow-engine-2"), v("dohflow-engine-1"));
    let Some(SyncEvent::UpdateRequired(update)) = event else {
        panic!("{event:?}")
    };
    assert_eq!(update.seq, 5);
    assert_eq!(update.envelope_engine_version, v("dohflow-engine-2"));
    assert!(status.can_apply(4) && !status.can_apply(5) && !status.can_push());
    // Idempotent; another digest at the position halts; a later or equal position does not displace.
    assert_eq!(
        status.observe_engine_version(5, [1; 32], v("dohflow-engine-2"), v("dohflow-engine-1")),
        None
    );
    assert_eq!(
        status.observe_engine_version(9, [9; 32], v("dohflow-engine-3"), v("dohflow-engine-1")),
        None
    );
    // An earlier position takes over.
    let mut later = SyncStatus::Healthy;
    later.observe_engine_version(9, [9; 32], v("dohflow-engine-3"), v("dohflow-engine-1"));
    assert!(matches!(
        later.observe_engine_version(5, [1; 32], v("dohflow-engine-2"), v("dohflow-engine-1")),
        Some(SyncEvent::UpdateRequired(UpdateRequired { seq: 5, .. }))
    ));
    assert!(matches!(
        status.observe_engine_version(5, [2; 32], v("dohflow-engine-2"), v("dohflow-engine-1")),
        Some(SyncEvent::Halted(HaltEvidence::SeqConflict { seq: 5, .. }))
    ));
    // A halted replica stays halted.
    assert_eq!(
        status.observe_engine_version(2, [4; 32], v("dohflow-engine-2"), v("dohflow-engine-1")),
        None
    );
}

#[test]
fn an_update_required_stop_ends_only_for_a_build_of_the_envelopes_version() {
    let mut status = SyncStatus::Healthy;
    status.observe_engine_version(5, [1; 32], v("dohflow-engine-2"), v("dohflow-engine-1"));
    status.observe_own_engine_version(v("dohflow-engine-3"));
    assert!(
        matches!(status, SyncStatus::UpdateRequired(_)),
        "another version is no exit"
    );
    status.observe_own_engine_version(v("dohflow-engine-2"));
    assert!(
        status.is_healthy(),
        "a build equal to the envelope's version evaluates it normally"
    );
    // Other statuses are untouched.
    let mut stopped = SyncStatus::Healthy;
    healthy_apply_stop(&mut stopped, 5, 1);
    let before = stopped;
    stopped.observe_own_engine_version(v("dohflow-engine-2"));
    assert_eq!(stopped, before);
}

// -- the snapshot exits ------------------------------------------------------------------------------------

#[test]
fn a_snapshot_is_an_exit_only_when_it_is_past_a_recorded_void_that_covers_the_stop() {
    let voided = HonoredVoid {
        void_id: [7; 32],
        first: 10,
        last: 19,
    };
    // Rooted at the void (20, the position after its range) or later, covering the stop.
    assert!(snapshot_passes_stop(12, 20, &[voided]));
    assert!(snapshot_passes_stop(10, 500, &[voided]));
    // Rooted inside the range (built on poisoned state) or at its last position: never a root.
    assert!(!snapshot_passes_stop(12, 19, &[voided]));
    assert!(!snapshot_passes_stop(12, 15, &[voided]));
    // The recorded void does not cover the stop, or nothing is recorded.
    assert!(!snapshot_passes_stop(25, 500, &[voided]));
    assert!(!snapshot_passes_stop(12, 500, &[]));
}

fn facts<'a>(
    root: u64,
    version: &'a EngineVersion,
    recorded: &'a [HonoredVoid],
    held: &'a [(u64, u64)],
) -> SnapshotFacts<'a> {
    SnapshotFacts {
        root,
        engine_version: version,
        recorded_voids: recorded,
        held_voids: held,
    }
}

#[test]
fn an_update_required_snapshot_exit_needs_the_position_covered_this_builds_version_and_no_held_void(
) {
    let mut status = SyncStatus::Healthy;
    status.observe_engine_version(5, [1; 32], v("dohflow-engine-2"), v("dohflow-engine-1"));
    let own = v("dohflow-engine-1");
    let other = v("dohflow-engine-2");
    assert!(status
        .rebootstrap_permit(&facts(5, &own, &[], &[]))
        .is_some());
    assert!(status
        .rebootstrap_permit(&facts(40, &own, &[], &[]))
        .is_some());
    assert!(
        status
            .rebootstrap_permit(&facts(4, &own, &[], &[]))
            .is_none(),
        "does not cover 5"
    );
    assert!(
        status
            .rebootstrap_permit(&facts(40, &other, &[], &[]))
            .is_none(),
        "a snapshot of the envelope's version is update-required itself"
    );
    // A void this replica holds covers 10..=19: a snapshot rooted inside it is never a root, but
    // one rooted past it is.
    let held = [(10, 19)];
    assert!(status
        .rebootstrap_permit(&facts(12, &own, &[], &held))
        .is_none());
    assert!(status
        .rebootstrap_permit(&facts(10, &own, &[], &held))
        .is_none());
    assert!(status
        .rebootstrap_permit(&facts(19, &own, &[], &held))
        .is_none());
    assert!(status
        .rebootstrap_permit(&facts(20, &own, &[], &held))
        .is_some());
}

#[test]
fn a_stop_covered_by_a_recorded_void_needs_a_snapshot_rooted_past_the_void() {
    let mut status = SyncStatus::Healthy;
    healthy_apply_stop(&mut status, 12, 1);
    let own = v("dohflow-engine-1");
    let recorded = [HonoredVoid {
        void_id: [7; 32],
        first: 10,
        last: 19,
    }];
    assert!(status
        .rebootstrap_permit(&facts(20, &own, &recorded, &[]))
        .is_some());
    assert!(status
        .rebootstrap_permit(&facts(19, &own, &recorded, &[]))
        .is_none());
    assert!(
        status
            .rebootstrap_permit(&facts(500, &own, &[], &[]))
            .is_none(),
        "no recorded void"
    );
}

#[test]
fn only_a_permit_for_the_current_stop_ends_it() {
    let own = v("dohflow-engine-1");
    let recorded = [HonoredVoid {
        void_id: [7; 32],
        first: 10,
        last: 19,
    }];
    let mut status = SyncStatus::Healthy;
    healthy_apply_stop(&mut status, 12, 1);
    let permit = status
        .rebootstrap_permit(&facts(20, &own, &recorded, &[]))
        .unwrap();
    // The status moved on to an EARLIER stop before the snapshot was installed: the old permit
    // is for another stop and changes nothing.
    let mut earlier = SyncStatus::Healthy;
    healthy_apply_stop(&mut earlier, 3, 2);
    assert_eq!(earlier.observe_rebootstrapped(permit), None);
    assert!(!earlier.is_healthy());
    assert!(matches!(
        status.observe_rebootstrapped(permit),
        Some(SyncEvent::Rebootstrapped { root: 20 })
    ));
    assert!(status.is_healthy());
    assert_eq!(
        status.observe_rebootstrapped(permit),
        None,
        "nothing left to clear"
    );
}

#[test]
fn a_re_bootstrap_clears_the_apply_semantics_forks_by_evidence_and_never_the_others() {
    let own = v("dohflow-engine-1");
    for mut halted in [
        {
            let mut s = SyncStatus::Healthy;
            s.observe_write_set_mismatch(8, [1; 32], [2; 32]);
            s
        },
        SyncStatus::Halted(HaltEvidence::AppliedPositionInvalid {
            seq: 8,
            envelope_digest: [3; 32],
        }),
    ] {
        assert!(halted
            .rebootstrap_permit(&facts(7, &own, &[], &[]))
            .is_none());
        let permit = halted
            .rebootstrap_permit(&facts(8, &own, &[], &[]))
            .unwrap();
        assert!(matches!(
            halted.observe_rebootstrapped(permit),
            Some(SyncEvent::Rebootstrapped { root: 8 })
        ));
        assert!(halted.is_healthy());
    }
    // Key quarantine and re-enrollment are the recovery of these, never a snapshot.
    for evidence in [
        HaltEvidence::Fork {
            version: 2,
            accepted: [1; 32],
            conflicting: [2; 32],
        },
        HaltEvidence::Equivocation {
            accepted: [1; 32],
            conflicting: [2; 32],
        },
        HaltEvidence::SeqConflict {
            seq: 4,
            first: [1; 32],
            second: [2; 32],
        },
        HaltEvidence::VoidAtTwoPositions {
            void_id: [3; 32],
            honored_at: 9,
            offered_at: 12,
        },
    ] {
        let halted = SyncStatus::Halted(evidence);
        assert!(
            halted
                .rebootstrap_permit(&facts(1_000, &own, &[], &[]))
                .is_none(),
            "{evidence:?}"
        );
    }
}

// -- the version gate is a precondition of every verdict (C.4 stage 1c) -----------------------------------

fn foreign_header() -> sync_core::header::ClearHeader {
    let mut header = sample_header();
    header.engine_version = "dohflow-engine-2".to_owned();
    header
}

#[test]
fn a_verdict_formed_at_a_foreign_version_is_not_a_verdict_in_either_call_order() {
    let foreign = foreign_header();
    let own = own_engine();
    // A failed receiver check on an envelope of another version: update required, not invalid.
    let mut a = SyncStatus::Healthy;
    assert!(matches!(
        a.observe_envelope(5, [1; 32], &foreign, &own, Err(ReceiverError::TagMismatch)),
        Some(SyncEvent::UpdateRequired(_))
    ));
    assert!(matches!(a, SyncStatus::UpdateRequired(_)));
    // Likewise an apply failure.
    let mut b = SyncStatus::Healthy;
    assert!(matches!(
        b.observe_apply_failure(5, [1; 32], &foreign, &own, Err(code(3))),
        Some(SyncEvent::UpdateRequired(_))
    ));
    // Even an Ok outcome for a foreign envelope is a stop: nothing was evaluated.
    let mut c = SyncStatus::Healthy;
    assert!(matches!(
        c.observe_envelope(5, [1; 32], &foreign, &own, Ok(())),
        Some(SyncEvent::UpdateRequired(_))
    ));
    // Order 1: the engine saw a verdict first (an equal-version observation), then learned the
    // version: at the same position update-required replaces the verdict.
    let mut d = SyncStatus::Healthy;
    d.observe_envelope(
        5,
        [1; 32],
        &sample_header(),
        &own,
        Err(ReceiverError::TagMismatch),
    );
    assert!(matches!(d, SyncStatus::EnvelopeInvalid(_)));
    assert!(matches!(
        d.observe_engine_version(5, [1; 32], v("dohflow-engine-2"), own),
        Some(SyncEvent::UpdateRequired(_))
    ));
    // Order 2: update-required first; a later verdict for the same position changes nothing.
    let mut e = SyncStatus::Healthy;
    e.observe_engine_version(5, [1; 32], v("dohflow-engine-2"), own);
    assert_eq!(
        e.observe_envelope(
            5,
            [1; 32],
            &sample_header(),
            &own,
            Err(ReceiverError::TagMismatch)
        ),
        None
    );
    assert!(matches!(e, SyncStatus::UpdateRequired(_)));
    // An equal-version header takes the ordinary path.
    let mut f = SyncStatus::Healthy;
    assert!(matches!(
        f.observe_envelope(
            5,
            [1; 32],
            &sample_header(),
            &own,
            Err(ReceiverError::TagMismatch)
        ),
        Some(SyncEvent::EnvelopeInvalid(_))
    ));
}

#[test]
fn a_cursor_past_an_update_required_stop_is_refused_like_one_past_an_invalid_stop() {
    let c = chain();
    let at = EpochAtPosition::after_epoch_record(&c.state, &c.e2).unwrap();
    let mut status = SyncStatus::Healthy;
    status.observe_engine_version(5, [0x22; 32], v("dohflow-engine-2"), v("dohflow-engine-1"));
    let void = verify_void(&c.state, &at, 9, &signed(&body(&c.a, 5), &c.a)).unwrap();
    let mut ledger = VoidLedger::new();
    let error = ledger
        .offer_in_order(
            &[void],
            &HonorContext {
                status: &status,
                next_to_apply: 7,
                snapshot_root: None,
                target_not_determinable: false,
            },
        )
        .unwrap_err();
    assert_eq!(error, sync_core::void::OfferError::StopBehindFrontier);
}

// -- a write-set mismatch is a fork --------------------------------------------------------------------------

#[test]
fn a_successful_apply_with_another_write_set_hash_halts_and_a_void_never_answers_it() {
    let mut status = SyncStatus::Healthy;
    assert_eq!(
        status.observe_write_set_mismatch(8, [5; 32], [5; 32]),
        None,
        "equal hashes are fine"
    );
    let event = status.observe_write_set_mismatch(8, [5; 32], [6; 32]);
    assert_eq!(
        event,
        Some(SyncEvent::Halted(HaltEvidence::WriteSetMismatch {
            seq: 8,
            expected: [5; 32],
            computed: [6; 32],
        }))
    );
    assert!(!status.can_apply(0) && !status.can_push());
    // A halt absorbs a second report and an honored void does not clear it.
    assert_eq!(status.observe_write_set_mismatch(9, [1; 32], [2; 32]), None);
    assert_eq!(
        status.observe_honored(&HonoredVoid {
            void_id: [1; 32],
            first: 8,
            last: 8
        }),
        None
    );
    assert!(matches!(status, SyncStatus::Halted(_)));
    // It also takes precedence over a stop already recorded.
    let mut stopped = SyncStatus::Healthy;
    healthy_apply_stop(&mut stopped, 12, 1);
    assert!(stopped
        .observe_write_set_mismatch(4, [1; 32], [2; 32])
        .is_some());
    assert!(matches!(stopped, SyncStatus::Halted(_)));
}
