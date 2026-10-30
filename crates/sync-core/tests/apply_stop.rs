//! The apply-failure stop, the update-required state (R40), the write-set mismatch halt and the
//! snapshot exits (personal-cfo-g3m.26; vault-format.md C.10 and C.13).

mod common;

use common::*;
use sync_core::envelope::ReceiverError;
use sync_core::membership::EpochAtPosition;
use sync_core::status::{
    snapshot_passes_update_required, ApplyFailureCode, EngineVersion, HaltEvidence, StopReason,
    SyncEvent, SyncStatus, UpdateRequired,
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
    status.observe_apply_failure(seq, [digest; 32], &sample_header(), Err(code(7)))
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
    status.observe_envelope(20, [1; 32], &header, Err(ReceiverError::TagMismatch));
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
        status.observe_envelope(30, [3; 32], &header, Err(ReceiverError::TagMismatch)),
        None
    );
    // An Ok outcome records nothing.
    let mut fresh = SyncStatus::Healthy;
    assert_eq!(
        fresh.observe_apply_failure(5, [1; 32], &header, Ok(())),
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
        status.observe_apply_failure(5, [0x22; 32], &sample_header(), Err(code(3)));
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

#[test]
fn an_update_required_snapshot_exit_needs_the_position_covered_and_this_builds_version() {
    let mut status = SyncStatus::Healthy;
    status.observe_engine_version(5, [1; 32], v("dohflow-engine-2"), v("dohflow-engine-1"));
    let SyncStatus::UpdateRequired(stop) = status else {
        panic!()
    };
    assert!(snapshot_passes_update_required(
        &stop,
        5,
        &v("dohflow-engine-1")
    ));
    assert!(snapshot_passes_update_required(
        &stop,
        40,
        &v("dohflow-engine-1")
    ));
    assert!(
        !snapshot_passes_update_required(&stop, 4, &v("dohflow-engine-1")),
        "does not cover 5"
    );
    assert!(
        !snapshot_passes_update_required(&stop, 40, &v("dohflow-engine-2")),
        "a snapshot of the envelope's version is update-required itself"
    );
}

#[test]
fn installing_a_snapshot_ends_a_stop_at_or_below_its_root_and_never_a_halt() {
    let mut status = SyncStatus::Healthy;
    healthy_apply_stop(&mut status, 12, 1);
    assert_eq!(
        status.observe_rebootstrapped(11),
        None,
        "the stop is above the root"
    );
    assert!(matches!(
        status.observe_rebootstrapped(20),
        Some(SyncEvent::Rebootstrapped { root: 20 })
    ));
    assert!(status.is_healthy());
    let mut update = SyncStatus::Healthy;
    update.observe_engine_version(5, [1; 32], v("dohflow-engine-2"), v("dohflow-engine-1"));
    assert!(update.observe_rebootstrapped(5).is_some() && update.is_healthy());
    let mut halted = SyncStatus::Healthy;
    halted.observe_write_set_mismatch(3, [1; 32], [2; 32]);
    assert_eq!(halted.observe_rebootstrapped(100), None);
    assert!(matches!(halted, SyncStatus::Halted(_)));
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
