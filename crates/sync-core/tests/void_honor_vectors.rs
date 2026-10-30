//! Public conformance vectors for the void honor rule (personal-cfo-g3m.17 part C; ADR 0074
//! Addendum D; `docs/architecture/vault-format.md` C.10 and C.14).
//!
//! Each case is a sequence of offers of a void to one replica, with the replica's state at each
//! offer (status, frontier, snapshot root) and the outcome the rule gives. The file pins the rule
//! as **semantic inputs only**: no signature, key or chain is in it (those are the membership
//! vectors and `verify_void`); a second implementation checks its own honor rule against it.
//!
//! Three guards: the committed file equals the generator's output; an independent reference
//! implementation of the rule (written below from the spec text, over the JSON alone) agrees with
//! every expected outcome; and every `Inert` and `ForkSuspected` variant has a vector, read from
//! the source so a new variant fails until it has one.
//!
//! Regenerate with `DOHFLOW_UPDATE_VECTORS=1 cargo test -p sync-core --test void_honor_vectors`.

mod common;

use common::*;
use serde_json::{json, Value};
use sync_core::envelope::ReceiverError;
use sync_core::membership::EpochAtPosition;
use sync_core::status::{
    ApplyFailureCode, EngineVersion, HaltEvidence, InvalidEnvelope, StopReason, SyncStatus,
    UpdateRequired,
};
use sync_core::void::{
    verify_void, ForkSuspected, Honor, HonorContext, VoidBody, VoidLedger, VoidReason,
};

const VECTORS: &str = include_str!("../../../docs/architecture/vectors/void-honor-v1.json");
const VECTORS_PATH: &str = "../../docs/architecture/vectors/void-honor-v1.json";
const VOID_SOURCE: &str = include_str!("../src/void.rs");

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Clone, Copy)]
enum Status {
    Healthy,
    Halted,
    /// Stopped at an invalid envelope: (seq, digest byte, command byte).
    Stopped(u64, u8, u8),
    /// Stopped at an envelope that failed to apply deterministically: (seq, digest, command).
    ApplyStopped(u64, u8, u8),
    /// Update-required at an envelope of another engine version: (seq, digest).
    UpdateRequired(u64, u8),
}

/// A void: (seq, position, command byte, digest byte, issued_at offset, reason byte).
type VoidSpec = (u64, u64, u8, u8, i64, u8);

#[derive(Clone)]
struct Step {
    /// The voids offered in this step, in the order given (the rule requires ascending position).
    voids: Vec<VoidSpec>,
    status: Status,
    next_to_apply: u64,
    snapshot_root: Option<u64>,
    not_determinable: bool,
    /// One outcome per void offered, up to and including the first that changes the replica:
    /// `honored:first:last`, `already_honored`, `inert:Reason`, `fork:Kind`; or one `error:Kind`
    /// for a refused offer.
    expect: Vec<&'static str>,
}

struct Case {
    name: &'static str,
    description: &'static str,
    steps: Vec<Step>,
}

fn step(void: VoidSpec, status: Status, next_to_apply: u64, expect: &'static str) -> Step {
    Step {
        voids: vec![void],
        status,
        next_to_apply,
        snapshot_root: None,
        not_determinable: false,
        expect: vec![expect],
    }
}

/// A void for `seq` at `position`, naming command byte 0x11 and digest byte 0x22.
const fn v(seq: u64, position: u64) -> VoidSpec {
    (seq, position, 0x11, 0x22, 0, 1)
}

/// The replica is stopped at `seq` on the envelope the default void names.
const fn at(seq: u64) -> Status {
    Status::Stopped(seq, 0x22, 0x11)
}

fn all_cases() -> Vec<Case> {
    let root = |mut s: Step, r: u64| {
        s.snapshot_root = Some(r);
        s
    };
    let undeterminable = |mut s: Step| {
        s.not_determinable = true;
        s
    };
    let batch = |voids: Vec<VoidSpec>, status: Status, next: u64, expect: Vec<&'static str>| Step {
        voids,
        status,
        next_to_apply: next,
        snapshot_root: None,
        not_determinable: false,
        expect,
    };
    vec![
        Case {
            name: "honored_for_the_range_before_the_void",
            description: "Stopped at 5 in log order, the void for 5 at position 9 is honored: positions 5 to 8 are skipped; the void itself (9) applies as a log object.",
            steps: vec![step(v(5, 9), at(5), 5, "honored:5:8")],
        },
        Case {
            name: "honored_for_a_one_position_range",
            description: "A void immediately after the poisoned position skips exactly that position.",
            steps: vec![step(v(5, 6), at(5), 5, "honored:5:5")],
        },
        Case {
            name: "honored_at_the_edges_of_the_position_space",
            description: "Range arithmetic at the extremes: the first position, and the last two.",
            steps: vec![
                step(v(0, 1), at(0), 0, "honored:0:0"),
                step(v(u64::MAX - 1, u64::MAX), at(u64::MAX - 1), u64::MAX - 1, "honored:18446744073709551614:18446744073709551614"),
            ],
        },
        Case {
            name: "the_same_void_offered_again_at_the_same_position_is_already_honored",
            description: "After a restart or a re-pull the identical void at the identical position is reported as already honored, so rollback and re-issue (driven off the first honor only) run exactly once. The replica's cursor has moved past the range by then.",
            steps: vec![
                step(v(5, 9), at(5), 5, "honored:5:8"),
                step(v(5, 9), Status::Healthy, 9, "already_honored"),
            ],
        },
        Case {
            name: "inert_when_the_void_is_not_after_the_position_it_names",
            description: "A void at the same position as its target, or before it, is not a void for it.",
            steps: vec![
                step(v(5, 5), at(5), 5, "inert:VoidNotAfterTarget"),
                step(v(5, 4), at(5), 5, "inert:VoidNotAfterTarget"),
            ],
        },
        Case {
            name: "not_after_target_is_checked_before_everything_else",
            description: "Every later condition also fails here (below the root, already applied); the first failing condition in the pinned order decides.",
            steps: vec![root(step(v(5, 5), Status::Healthy, 9, "inert:VoidNotAfterTarget"), 8)],
        },
        Case {
            name: "inert_when_an_honored_void_already_covers_the_position",
            description: "Coverage comes only from honored voids. After 5..=19 is honored (the cursor is now 20), a void naming 8 is inert.",
            steps: vec![
                step(v(5, 20), at(5), 5, "honored:5:19"),
                step(v(8, 21), Status::Healthy, 20, "inert:AlreadyCovered"),
            ],
        },
        Case {
            name: "inert_when_the_position_is_at_or_below_the_snapshot_root",
            description: "A replica bootstrapped from a snapshot at root 8 starts its cursor at 9 and never evaluated position 5 or 8; the void is not honored and the engine must treat the root as suspect.",
            steps: vec![root(step(v(5, 12), Status::Healthy, 9, "inert:PrecedesRoot"), 8), root(step(v(8, 12), Status::Healthy, 9, "inert:PrecedesRoot"), 8)],
        },
        Case {
            name: "inert_when_the_replica_applied_the_position",
            description: "The replica found position 5 valid and applied it (the frontier is past it).",
            steps: vec![step(v(5, 9), Status::Healthy, 6, "inert:FoundValid")],
        },
        Case {
            name: "inert_while_the_frontier_is_behind_the_position",
            description: "Out of order: the replica verified a poisoned 30 before 10..=29 were processed. Honoring would clear a stop early and let it apply past 30 on unreleased state; it must wait for the frontier.",
            steps: vec![step(v(30, 40), at(30), 10, "inert:NotYetEvaluated")],
        },
        Case {
            name: "inert_when_the_replica_is_at_the_position_but_not_stopped_there",
            description: "At the frontier but healthy (not yet pulled or evaluated), halted, or stopped at a later position: nothing to honor yet.",
            steps: vec![
                step(v(5, 9), Status::Healthy, 5, "inert:NotYetEvaluated"),
                step(v(5, 9), Status::Halted, 5, "inert:NotYetEvaluated"),
                step(v(5, 9), at(9), 5, "inert:NotYetEvaluated"),
            ],
        },
        Case {
            name: "inert_when_the_replica_cannot_judge_the_position",
            description: "At the position with no verified key yet: NotDeterminable is never a basis to honor. It is reported whenever the replica is at the position, cannot judge it and is not stopped there, including with an out-of-order stop at a later position; a halted replica judges nothing.",
            steps: vec![
                undeterminable(step(v(5, 9), Status::Healthy, 5, "inert:NotDeterminable")),
                undeterminable(step(v(5, 9), at(9), 5, "inert:NotDeterminable")),
                undeterminable(step(v(5, 9), Status::Halted, 5, "inert:NotYetEvaluated")),
            ],
        },
        Case {
            name: "inert_when_the_command_differs_but_the_digest_matches",
            description: "The digest binds the command id, so equal digests with different command ids is a signer fault: inert.",
            steps: vec![step((5, 9, 0x99, 0x22, 0, 1), at(5), 5, "inert:CommandMismatch")],
        },
        Case {
            name: "a_different_digest_at_the_stop_is_a_suspected_fork",
            description: "An authorized member saw another envelope at the stop's position than this replica did. An honest service refuses such a void; the engine halts (a log fork).",
            steps: vec![step((5, 9, 0x11, 0x77, 0, 1), at(5), 5, "fork:DigestAtStop")],
        },
        Case {
            name: "the_same_void_at_another_position_is_a_suspected_fork",
            description: "A void already honored at position 9 offered again at 12: the log forked, not an idempotent repeat.",
            steps: vec![step(v(5, 9), at(5), 5, "honored:5:8"), step(v(5, 12), Status::Healthy, 9, "fork:VoidAtTwoPositions")],
        },
        Case {
            name: "a_transient_inert_void_is_honored_when_the_replica_catches_up",
            description: "Offered while the frontier is behind (inert, transient), then again at the frontier: honored. The same void gives the same ledger on a replica that stopped at 10 first and reached 30 later.",
            steps: vec![
                step(v(30, 40), at(30), 10, "inert:NotYetEvaluated"),
                step(v(30, 40), at(30), 30, "honored:30:39"),
            ],
        },
        Case {
            name: "an_inert_void_over_an_applied_range_never_blocks_a_later_genuine_void",
            description: "A void naming 3 arrives after 3 was applied: inert, it covers nothing. A genuinely poisoned 6 is reached later and voided at 12: still honored.",
            steps: vec![
                step(v(3, 10), Status::Healthy, 7, "inert:FoundValid"),
                step(v(6, 12), at(6), 6, "honored:6:11"),
            ],
        },
        Case {
            name: "two_voids_for_one_position_are_offered_in_position_order_and_the_earlier_wins",
            description: "A service that skips its same-seq dedupe serves two different voids for 10 at positions 12 and 15 (they differ in issued_at). Offered ascending, the earlier is honored for 10..=11 and the offer stops there (the context is stale); out of order, or at equal positions, the offer is refused. After the cursor moves past the range the later void is covered.",
            steps: vec![
                batch(vec![(10, 15, 0x11, 0x22, 1, 1), v(10, 12)], at(10), 10, vec!["error:VoidsNotAscending"]),
                batch(vec![v(10, 12), (10, 15, 0x11, 0x22, 1, 1)], at(10), 10, vec!["honored:10:11"]),
                step((10, 15, 0x11, 0x22, 1, 1), Status::Healthy, 12, "inert:AlreadyCovered"),
            ],
        },
        Case {
            name: "a_batch_of_inert_voids_is_reported_in_full",
            description: "Voids that change nothing are all reported, in order.",
            steps: vec![batch(vec![v(10, 12), (10, 15, 0x11, 0x22, 1, 1)], Status::Healthy, 3, vec!["inert:NotYetEvaluated", "inert:NotYetEvaluated"])],
        },
        Case {
            name: "honored_at_an_apply_failure_stop_by_a_reason_2_void",
            description: "A deterministic apply failure stops the replica at 5 like an invalid envelope; the reason-2 void for it is honored for the range before the void.",
            steps: vec![step((5, 9, 0x11, 0x22, 0, 2), Status::ApplyStopped(5, 0x22, 0x11), 5, "honored:5:8")],
        },
        Case {
            name: "the_reason_byte_plays_no_part_in_honoring",
            description: "A reason-1 void at an apply stop and a reason-2 void at a receiver-check stop are honored just the same.",
            steps: vec![
                step((5, 9, 0x11, 0x22, 0, 1), Status::ApplyStopped(5, 0x22, 0x11), 5, "honored:5:8"),
                step((20, 24, 0x11, 0x22, 0, 2), at(20), 20, "honored:20:23"),
            ],
        },
        Case {
            name: "a_different_digest_at_an_apply_stop_is_a_suspected_fork",
            description: "The same fork rule holds at an apply stop.",
            steps: vec![step((5, 9, 0x11, 0x77, 0, 2), Status::ApplyStopped(5, 0x22, 0x11), 5, "fork:DigestAtStop")],
        },
        Case {
            name: "a_replica_of_another_engine_version_cannot_judge_a_reason_2_void",
            description: "The envelope at 5 carries another engine_version than this build: the replica is update-required there (R40), has formed no verdict and is not stopped on the envelope, so the void is inert and transient. Its exits (a build of the envelope's version, or a snapshot past the void) are status transitions, not honoring.",
            steps: vec![step((5, 9, 0x11, 0x22, 0, 2), Status::UpdateRequired(5, 0x22), 5, "inert:NotYetEvaluated")],
        },
        Case {
            name: "an_inconsistent_replica_state_is_refused_never_a_permanent_found_valid",
            description: "A cursor that passed the stop (chain records verified past the stop must not advance it), a snapshot root at or above the cursor, and a cursor inside a range an honored void skipped are refused with an error, checked before any void: answering FoundValid would be permanent and split the vault.",
            steps: vec![
                step(v(5, 9), at(5), 7, "error:StopBehindFrontier"),
                root(step(v(5, 9), Status::Healthy, 5, "error:RootNotBelowFrontier"), 5),
                step(v(5, 9), at(5), 5, "honored:5:8"),
                step(v(5, 9), Status::Healthy, 8, "error:FrontierInsideHonoredRange"),
            ],
        },
    ]
}

fn status_of(step: &Step) -> SyncStatus {
    match step.status {
        Status::Healthy => SyncStatus::Healthy,
        Status::Halted => SyncStatus::Halted(HaltEvidence::Fork {
            version: 2,
            accepted: [1; 32],
            conflicting: [2; 32],
        }),
        Status::ApplyStopped(seq, digest, command) => {
            SyncStatus::EnvelopeInvalid(InvalidEnvelope {
                seq,
                envelope_digest: [digest; 32],
                command_id: [command; 16],
                device_id: [1; 16],
                device_key_fingerprint: Dev::new(1).fingerprint(),
                reason: StopReason::Apply(ApplyFailureCode::new(7).unwrap()),
            })
        }
        Status::UpdateRequired(seq, digest) => SyncStatus::UpdateRequired(UpdateRequired {
            seq,
            envelope_digest: [digest; 32],
            envelope_engine_version: EngineVersion::new("dohflow-engine-1").unwrap(),
            own_engine_version: EngineVersion::new("dohflow-engine-2").unwrap(),
        }),
        Status::Stopped(seq, digest, command) => SyncStatus::EnvelopeInvalid(InvalidEnvelope {
            seq,
            envelope_digest: [digest; 32],
            command_id: [command; 16],
            device_id: [1; 16],
            device_key_fingerprint: Dev::new(1).fingerprint(),
            reason: StopReason::ReceiverCheck(ReceiverError::TagMismatch),
        }),
    }
}

fn outcome_name(outcome: Honor) -> String {
    match outcome {
        Honor::Honored(h) => format!("honored:{}:{}", h.first, h.last),
        Honor::AlreadyHonored(_) => "already_honored".to_owned(),
        Honor::Inert(why) => format!("inert:{why:?}"),
        Honor::ForkSuspected(ForkSuspected::DigestAtStop { .. }) => "fork:DigestAtStop".to_owned(),
        Honor::ForkSuspected(ForkSuspected::VoidAtTwoPositions { .. }) => {
            "fork:VoidAtTwoPositions".to_owned()
        }
    }
}

/// Run a case against the real implementation; returns each step's outcomes and void ids.
fn run(case: &Case) -> Vec<(Vec<String>, Vec<String>)> {
    let chain = chain();
    let at_end = EpochAtPosition::after_epoch_record(&chain.state, &chain.e2).unwrap();
    let mut ledger = VoidLedger::new();
    case.steps
        .iter()
        .map(|s| {
            let voids: Vec<_> = s
                .voids
                .iter()
                .map(|&(seq, position, cmd, dig, issued, reason)| {
                    let body = VoidBody {
                        command_id: [cmd; 16],
                        envelope_digest: [dig; 32],
                        issued_at: 1_790_000_000_000_000 + issued,
                        reason: if reason == 2 {
                            VoidReason::DeterministicApply
                        } else {
                            VoidReason::ReceiverCheck
                        },
                        ..body(&chain.a, seq)
                    };
                    verify_void(&chain.state, &at_end, position, &signed(&body, &chain.a)).unwrap()
                })
                .collect();
            let ids = voids.iter().map(|v| hex(v.id())).collect();
            let status = status_of(s);
            let outcomes = match ledger.offer_in_order(
                &voids,
                &HonorContext {
                    status: &status,
                    next_to_apply: s.next_to_apply,
                    snapshot_root: s.snapshot_root,
                    target_not_determinable: s.not_determinable,
                },
            ) {
                Ok(outcomes) => outcomes.into_iter().map(outcome_name).collect(),
                Err(error) => vec![format!("error:{error:?}")],
            };
            (outcomes, ids)
        })
        .collect()
}

fn document() -> Value {
    let cases: Vec<Value> = all_cases()
        .iter()
        .map(|case| {
            let results = run(case);
            let steps: Vec<Value> = case
                .steps
                .iter()
                .zip(&results)
                .map(|(s, (outcomes, ids))| {
                    assert_eq!(outcomes, &s.expect, "{}: intended outcome", case.name);
                    let status = match s.status {
                        Status::Healthy => json!({"kind": "healthy"}),
                        Status::Halted => json!({"kind": "halted"}),
                        Status::Stopped(seq, dig, cmd) => json!({
                            "kind": "stopped_at_invalid_envelope",
                            "stop_reason": "receiver_check",
                            "seq": seq,
                            "envelope_digest": hex(&[dig; 32]),
                            "command_id": hex(&[cmd; 16]),
                        }),
                        Status::ApplyStopped(seq, dig, cmd) => json!({
                            "kind": "stopped_at_invalid_envelope",
                            "stop_reason": "apply",
                            "seq": seq,
                            "envelope_digest": hex(&[dig; 32]),
                            "command_id": hex(&[cmd; 16]),
                        }),
                        Status::UpdateRequired(seq, dig) => json!({
                            "kind": "update_required",
                            "seq": seq,
                            "envelope_digest": hex(&[dig; 32]),
                        }),
                    };
                    let voids: Vec<Value> = s
                        .voids
                        .iter()
                        .zip(ids)
                        .map(|(&(seq, position, cmd, dig, _, reason), id)| {
                            json!({
                                "id": id,
                                "seq": seq,
                                "position": position,
                                "command_id": hex(&[cmd; 16]),
                                "envelope_digest": hex(&[dig; 32]),
                                "reason": reason,
                            })
                        })
                        .collect();
                    json!({
                        "voids": voids,
                        "replica": {
                            "status": status,
                            "next_to_apply": s.next_to_apply,
                            "snapshot_root": s.snapshot_root,
                            "target_not_determinable": s.not_determinable,
                        },
                        "expect": s.expect,
                    })
                })
                .collect();
            json!({"name": case.name, "description": case.description, "steps": steps})
        })
        .collect();
    json!({
        "vectors_version": 1,
        "spec": "docs/architecture/vault-format.md C.10 \"Void records\" and C.14; ADR 0074 Addendum D and E",
        "synthetic": "Every value is a repeated byte or a small integer. No key, signature or real value is in this file: the void's signature and signer are checked before the rule runs (membership vectors), and this file pins the rule alone.",
        "how_to_read": "Each case is a sequence of steps against ONE replica, sharing one ledger of honored voids that starts empty. In each step the replica is offered a list of verified voids, in the order given ('voids': each with its id, the log 'seq' it names, the log 'position' V it occupies, and the 'command_id' and 'envelope_digest' it names) in the replica's state ('replica': its 'status' (healthy, halted, or stopped at an invalid envelope at a position with a digest and a command id), 'next_to_apply' (the lowest position not yet applied or skipped; every position below it is done; it never passes the stop), the 'snapshot_root' it bootstrapped from or null, and whether it is at the position but 'target_not_determinable'). 'expect' has one outcome per void offered, up to and including the first that changes the replica (the rest are not offered), or a single 'error:KIND' when the offer is refused. An outcome is 'honored:FIRST:LAST' (positions FIRST..=LAST are skipped and added to the ledger; rollback and re-issue are driven off this outcome only), 'already_honored' (the same void at the same position again: nothing changes and nothing is re-issued), 'inert:REASON' (nothing changes) or 'fork:KIND' (nothing changes and the replica halts as a log fork).",
        "refused_before_any_void": [
            "StopBehindFrontier: the status is a stop at a position below next_to_apply",
            "RootNotBelowFrontier: snapshot_root is set and snapshot_root >= next_to_apply",
            "FrontierInsideHonoredRange: an honored range ends at or after next_to_apply",
            "VoidsNotAscending: the voids' positions are not strictly ascending"
        ],
        "rule_order": [
            "same void id already honored: same position is already_honored, another position is fork:VoidAtTwoPositions",
            "VoidNotAfterTarget: position <= seq",
            "AlreadyCovered: an honored range contains seq",
            "PrecedesRoot: snapshot_root is set and seq <= snapshot_root",
            "FoundValid: seq < next_to_apply",
            "NotYetEvaluated: next_to_apply < seq",
            "here next_to_apply == seq. Stopped at an invalid envelope at seq: continue below. Halted: NotYetEvaluated. Otherwise (healthy, or stopped at another position): NotDeterminable when target_not_determinable, else NotYetEvaluated",
            "stopped here with a different envelope_digest: fork:DigestAtStop",
            "stopped here with a different command_id: CommandMismatch",
            "otherwise honored: FIRST = seq, LAST = position - 1"
        ],
        "transient_reasons": ["NotYetEvaluated", "NotDeterminable"],
        "transient_note": "A transient void is offered again, in position order, when the frontier or the key state changes. Every other inert reason is permanent for that void.",
        "reasons": "A void states reason 1 (the envelope fails the receiver checks) or 2 (it passes them and fails to apply deterministically). The reason is audit only: a replica stopped at the position honors the void whichever reason the void or the stop states. A replica of another engine_version is update-required at the position (R40), is not stopped there and cannot judge the void: the outcome is the transient NotYetEvaluated.",
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
        "the vectors file drifted from the generator; regenerate with DOHFLOW_UPDATE_VECTORS=1 \
         (a spec change under C.13)"
    );
}

/// An independent implementation of the rule, written from the spec text and reading ONLY the JSON.
fn reference(doc: &Value) -> Vec<(String, Vec<Vec<String>>)> {
    let u = |v: &Value| v.as_u64().expect("an integer");
    doc["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| {
            let mut ledger: Vec<(String, u64, u64)> = Vec::new();
            let steps = case["steps"]
                .as_array()
                .unwrap()
                .iter()
                .map(|step| {
                    let replica = &step["replica"];
                    let next = u(&replica["next_to_apply"]);
                    let status = &replica["status"];
                    let stop_seq = (status["kind"] == "stopped_at_invalid_envelope")
                        .then(|| u(&status["seq"]));
                    let voids = step["voids"].as_array().unwrap();
                    if stop_seq.is_some_and(|s| s < next) {
                        return vec!["error:StopBehindFrontier".to_owned()];
                    }
                    if replica["snapshot_root"].as_u64().is_some_and(|r| r >= next) {
                        return vec!["error:RootNotBelowFrontier".to_owned()];
                    }
                    if ledger.iter().any(|(_, _, last)| *last >= next) {
                        return vec!["error:FrontierInsideHonoredRange".to_owned()];
                    }
                    if voids
                        .windows(2)
                        .any(|w| u(&w[0]["position"]) >= u(&w[1]["position"]))
                    {
                        return vec!["error:VoidsNotAscending".to_owned()];
                    }
                    let mut outcomes = Vec::new();
                    for void in voids {
                        let id = void["id"].as_str().unwrap().to_owned();
                        let (seq, position) = (u(&void["seq"]), u(&void["position"]));
                        let (outcome, changes) = (|| {
                            if let Some((_, _, last)) = ledger.iter().find(|(i, _, _)| *i == id) {
                                return if *last + 1 == position {
                                    ("already_honored".to_owned(), false)
                                } else {
                                    ("fork:VoidAtTwoPositions".to_owned(), true)
                                };
                            }
                            let inert = |why: &str| (format!("inert:{why}"), false);
                            if position <= seq {
                                return inert("VoidNotAfterTarget");
                            }
                            if ledger.iter().any(|(_, f, l)| *f <= seq && seq <= *l) {
                                return inert("AlreadyCovered");
                            }
                            if replica["snapshot_root"].as_u64().is_some_and(|r| seq <= r) {
                                return inert("PrecedesRoot");
                            }
                            if seq < next {
                                return inert("FoundValid");
                            }
                            if next < seq {
                                return inert("NotYetEvaluated");
                            }
                            if stop_seq != Some(seq) {
                                let judged = status["kind"] != "halted"
                                    && replica["target_not_determinable"] == true;
                                return inert(if judged {
                                    "NotDeterminable"
                                } else {
                                    "NotYetEvaluated"
                                });
                            }
                            if status["envelope_digest"] != void["envelope_digest"] {
                                return ("fork:DigestAtStop".to_owned(), true);
                            }
                            if status["command_id"] != void["command_id"] {
                                return inert("CommandMismatch");
                            }
                            ledger.push((id.clone(), seq, position - 1));
                            (format!("honored:{seq}:{}", position - 1), true)
                        })();
                        outcomes.push(outcome);
                        if changes {
                            break;
                        }
                    }
                    outcomes
                })
                .collect();
            (case["name"].as_str().unwrap().to_owned(), steps)
        })
        .collect()
}

#[test]
fn an_independent_reference_implementation_agrees_with_every_expected_outcome() {
    let doc: Value = serde_json::from_str(VECTORS).unwrap();
    let produced = reference(&doc);
    for (case, (name, steps)) in doc["cases"].as_array().unwrap().iter().zip(&produced) {
        let expected: Vec<Vec<&str>> = case["steps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| {
                s["expect"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|e| e.as_str().unwrap())
                    .collect()
            })
            .collect();
        assert_eq!(&expected, steps, "{name}");
    }
}

fn variants_of(enum_name: &str) -> Vec<String> {
    let body = VOID_SOURCE
        .split_once(&format!("pub enum {enum_name} {{"))
        .unwrap()
        .1
        .split_once("\n}\n")
        .unwrap()
        .0;
    body.lines()
        .filter(|l| !l.trim_start().starts_with("//") && !l.trim_start().starts_with('#'))
        .filter_map(|l| {
            let t = l.trim();
            let name = t.split(|c: char| !c.is_ascii_alphanumeric()).next()?;
            (l.starts_with("    ")
                && !l.starts_with("     ")
                && name.chars().next()?.is_ascii_uppercase())
            .then(|| name.to_owned())
        })
        .collect()
}

#[test]
fn every_inert_and_fork_variant_has_a_vector() {
    let doc: Value = serde_json::from_str(VECTORS).unwrap();
    let expected: Vec<String> = doc["cases"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|c| c["steps"].as_array().unwrap().iter())
        .flat_map(|s| s["expect"].as_array().unwrap().iter())
        .map(|e| e.as_str().unwrap().to_owned())
        .collect();
    let inert = variants_of("Inert");
    assert!(inert.len() >= 7, "the scan read the variants: {inert:?}");
    for variant in inert {
        assert!(
            expected.contains(&format!("inert:{variant}")),
            "Inert::{variant} has no vector"
        );
    }
    for variant in variants_of("OfferError") {
        assert!(
            expected.contains(&format!("error:{variant}")),
            "OfferError::{variant} has no vector"
        );
    }
    let forks = variants_of("ForkSuspected");
    assert_eq!(forks.len(), 2, "{forks:?}");
    for variant in forks {
        assert!(
            expected.contains(&format!("fork:{variant}")),
            "ForkSuspected::{variant} has no vector"
        );
    }
}
