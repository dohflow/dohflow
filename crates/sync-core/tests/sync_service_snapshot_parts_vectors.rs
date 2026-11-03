//! Public conformance vectors for the Sync service's snapshot upload in parts
//! (personal-cfo-g3m.36; `docs/architecture/sync-service-protocol.md` 4.13 and 11.2).
//!
//! Each case is a sequence of requests to ONE server that starts empty (a vault with no halts,
//! writes permitted, nothing staged) and the status, error code and result it must give. Part and
//! container contents are described, not stored (`{length, fill}`), so a 12 MiB container is a few
//! lines of JSON; the SHA-256 the client declares is computed here from those bytes and written in.
//!
//! Three guards, as for the void vectors: the committed file equals the generator's output; a
//! reference server model written from the protocol text over the JSON alone agrees with every
//! expected outcome; and every error the addition introduced, and every step of every operation's
//! decision order, has a vector.
//!
//! The clear-header check of part 1 is the container check `stageSnapshot` shares; it needs sealed
//! containers and is not in this file.
//!
//! Regenerate with
//! `DOHFLOW_UPDATE_VECTORS=1 cargo test -p sync-core --test sync_service_snapshot_parts_vectors`.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const VECTORS: &str =
    include_str!("../../../docs/architecture/vectors/sync-service-snapshot-parts-v1.json");
const VECTORS_PATH: &str = "../../docs/architecture/vectors/sync-service-snapshot-parts-v1.json";
const REGISTRY: &str = include_str!("../../../docs/architecture/sync-service-protocol.json");

const MIB: u64 = 1024 * 1024;
const MAX_SNAPSHOT: u64 = 1_073_742_104;
const MIN_PART: u64 = 5 * MIB;
/// The operator limits the vectors use: a part and a single request both top out at 8 MiB.
const PART_MAX: u64 = 8 * MIB;
const SINGLE_MAX: u64 = 8 * MIB;
const EXPIRES_AT: &str = "2026-11-10T12:00:00Z";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn num(n: u64) -> Value {
    Value::String(n.to_string())
}
fn sha_of(parts: &[(u64, u8)]) -> String {
    let mut h = Sha256::new();
    for (len, fill) in parts {
        h.update(vec![*fill; *len as usize]);
    }
    hex(&h.finalize())
}

// ---------- the generator ----------

const SNAP: &str = "11111111-1111-4111-8111-111111111111";
const OTHER: &str = "22222222-2222-4222-8222-222222222222";

/// A 12 MiB container in 5 MiB parts: 5, 5 and 2 MiB of the bytes 0x61, 0x62 and 0x63.
fn three_parts() -> [(u64, u8); 3] {
    [(MIN_PART, 0x61), (MIN_PART, 0x62), (2 * MIB, 0x63)]
}
const TOTAL: u64 = 12 * MIB;

fn begin(cred: &str, snap: &str, size: u64, sha: &str, part_size: u64, expect: Value) -> Value {
    json!({"op": "begin", "credential": cred, "snapshot_id": snap, "body": {"size": num(size), "sha256": sha, "part_size": num(part_size)}, "expect": expect})
}
fn begin_raw(cred: &str, snap: &str, body: Value, expect: Value) -> Value {
    json!({"op": "begin", "credential": cred, "snapshot_id": snap, "body": body, "expect": expect})
}
fn part(cred: &str, snap: &str, number: &str, len: u64, fill: u8, expect: Value) -> Value {
    json!({"op": "part", "credential": cred, "snapshot_id": snap, "part_number": number, "bytes": {"length": num(len), "fill": fill}, "expect": expect})
}
fn complete(cred: &str, snap: &str, expect: Value) -> Value {
    json!({"op": "complete", "credential": cred, "snapshot_id": snap, "expect": expect})
}
fn abort(cred: &str, snap: &str, expect: Value) -> Value {
    json!({"op": "abort", "credential": cred, "snapshot_id": snap, "expect": expect})
}
fn stage_single(cred: &str, snap: &str, len: u64, fill: u8, expect: Value) -> Value {
    json!({"op": "stageSnapshot", "credential": cred, "snapshot_id": snap, "bytes": {"length": num(len), "fill": fill}, "expect": expect})
}
fn halt(cred: &str, stopped_at: u64) -> Value {
    json!({"op": "reportHalt", "credential": cred, "stopped_at": num(stopped_at), "expect": {"status": 200, "stopped_at": num(stopped_at)}})
}
fn when(conditions: Value, mut step: Value) -> Value {
    step["conditions"] = conditions;
    step
}

fn uploading(part_size: u64, count: u64, next: u64) -> Value {
    json!({"status": 201, "snapshot_id": SNAP, "state": "uploading", "part_size": num(part_size), "part_count": num(count), "next_part": num(next), "expires_at": EXPIRES_AT})
}
fn uploading_again(part_size: u64, count: u64, next: u64) -> Value {
    let mut v = uploading(part_size, count, next);
    v["status"] = json!(200);
    v
}
fn staged_answer(status: u16) -> Value {
    json!({"status": status, "snapshot_id": SNAP, "state": "staged", "part_size": null, "part_count": null, "next_part": null, "expires_at": null})
}
fn accepted_part(n: u64, next: Option<u64>, status: u16) -> Value {
    json!({"status": status, "part_number": num(n), "next_part": next.map(num)})
}
fn staged(size: u64, status: u16) -> Value {
    json!({"status": status, "snapshot_id": SNAP, "size": num(size)})
}
fn refused(status: u16, error: &str) -> Value {
    json!({"status": status, "error": error})
}
fn refused_next(status: u16, error: &str, next: u64) -> Value {
    json!({"status": status, "error": error, "details": {"next_part": num(next)}})
}
fn halted(at: u64) -> Value {
    json!({"status": 409, "error": "ReplicaHalted", "details": {"stopped_at": num(at)}})
}

fn case(name: &str, description: &str, steps: Vec<Value>) -> Value {
    json!({"name": name, "description": description, "steps": steps})
}

fn all_cases() -> Vec<Value> {
    let parts = three_parts();
    let sha = sha_of(&parts);
    let bad_sha = sha_of(&[(TOTAL, 0x7a)]);
    let run = |extra: Vec<Value>| {
        let mut steps = vec![
            begin("A", SNAP, TOTAL, &sha, MIN_PART, uploading(MIN_PART, 3, 1)),
            part(
                "A",
                SNAP,
                "1",
                MIN_PART,
                0x61,
                accepted_part(1, Some(2), 201),
            ),
            part(
                "A",
                SNAP,
                "2",
                MIN_PART,
                0x62,
                accepted_part(2, Some(3), 201),
            ),
        ];
        steps.extend(extra);
        steps
    };

    vec![
        case(
            "a_container_is_staged_in_parts",
            "A 12 MiB container in 5 MiB parts (5, 5 and 2 MiB). Each part is accepted in order; complete checks the length and the SHA-256 of the whole and stages it (201); completing again is 200 with the original result; beginning again with the same parameters is 200 and says it is staged, and with another digest or size it is ObjectConflict (409).",
            run(vec![
                part("A", SNAP, "3", 2 * MIB, 0x63, accepted_part(3, None, 201)),
                complete("A", SNAP, staged(TOTAL, 201)),
                complete("A", SNAP, staged(TOTAL, 200)),
                begin("A", SNAP, TOTAL, &sha, MIN_PART, staged_answer(200)),
                begin("A", SNAP, TOTAL, &bad_sha, MIN_PART, refused(409, "ObjectConflict")),
                begin("A", SNAP, TOTAL + 1, &sha, MIN_PART, refused(409, "ObjectConflict")),
            ]),
        ),
        case(
            "a_snapshot_at_the_maximum_is_accepted_and_one_byte_over_is_too_large",
            "The 7.1 maximum is accepted with 8 MiB parts (129 parts); one byte more is SnapshotTooLarge (413) whatever the digest.",
            vec![
                begin("A", SNAP, MAX_SNAPSHOT, &sha, PART_MAX, uploading(PART_MAX, MAX_SNAPSHOT.div_ceil(PART_MAX), 1)),
                begin("A", OTHER, MAX_SNAPSHOT + 1, &sha, PART_MAX, refused(413, "SnapshotTooLarge")),
            ],
        ),
        case(
            "the_part_size_is_bounded",
            "A part size below 5 MiB is MalformedRequest (400); above the operator's snapshot_part_max_bytes it is PayloadTooLarge (413); both are judged after the size and before any state is looked at.",
            vec![
                begin("A", SNAP, TOTAL, &sha, MIN_PART - 1, refused(400, "MalformedRequest")),
                begin("A", SNAP, TOTAL, &sha, PART_MAX + 1, refused(413, "PayloadTooLarge")),
                begin("A", SNAP, TOTAL, &sha, MIN_PART, uploading(MIN_PART, 3, 1)),
            ],
        ),
        case(
            "the_request_body_is_strict",
            "A size of zero, a non-decimal size, an upper-case or short digest, a part_size that is not a decimal, a missing member and an unknown member are each MalformedRequest (400).",
            vec![
                begin_raw("A", SNAP, json!({"size": "0", "sha256": sha, "part_size": num(MIN_PART)}), refused(400, "MalformedRequest")),
                begin_raw("A", SNAP, json!({"size": "12x", "sha256": sha, "part_size": num(MIN_PART)}), refused(400, "MalformedRequest")),
                begin_raw("A", SNAP, json!({"size": num(TOTAL), "sha256": sha.to_uppercase(), "part_size": num(MIN_PART)}), refused(400, "MalformedRequest")),
                begin_raw("A", SNAP, json!({"size": num(TOTAL), "sha256": &sha[..63], "part_size": num(MIN_PART)}), refused(400, "MalformedRequest")),
                begin_raw("A", SNAP, json!({"size": num(TOTAL), "sha256": sha, "part_size": 5242880}), refused(400, "MalformedRequest")),
                begin_raw("A", SNAP, json!({"size": num(TOTAL), "sha256": sha}), refused(400, "MalformedRequest")),
                begin_raw("A", SNAP, json!({"size": num(TOTAL), "sha256": sha, "part_size": num(MIN_PART), "extra": "x"}), refused(400, "MalformedRequest")),
            ],
        ),
        case(
            "beginning_again_is_a_retry_and_a_different_begin_is_a_conflict",
            "The same parameters while the upload is in progress answer 200 with its state (next_part moves with it); a different digest, size or part size under the same snapshot id is ObjectConflict (409).",
            vec![
                begin("A", SNAP, TOTAL, &sha, MIN_PART, uploading(MIN_PART, 3, 1)),
                part("A", SNAP, "1", MIN_PART, 0x61, accepted_part(1, Some(2), 201)),
                begin("A", SNAP, TOTAL, &sha, MIN_PART, uploading_again(MIN_PART, 3, 2)),
                begin("A", SNAP, TOTAL, &bad_sha, MIN_PART, refused(409, "ObjectConflict")),
                begin("A", SNAP, TOTAL + 1, &sha, MIN_PART, refused(409, "ObjectConflict")),
                begin("A", SNAP, TOTAL, &sha, MIN_PART + 1, refused(409, "ObjectConflict")),
            ],
        ),
        case(
            "a_part_of_the_wrong_size_is_refused",
            "Every part but the last must be exactly part_size, and the last exactly the remainder: one byte short, one byte over, and a last part that is not the remainder are MalformedObject (400); a part longer than the operator's snapshot_part_max_bytes is PayloadTooLarge (413). A refused part leaves next_part where it was.",
            vec![
                begin("A", SNAP, TOTAL, &sha, MIN_PART, uploading(MIN_PART, 3, 1)),
                part("A", SNAP, "1", MIN_PART - 1, 0x61, refused(400, "MalformedObject")),
                part("A", SNAP, "1", MIN_PART + 1, 0x61, refused(400, "MalformedObject")),
                part("A", SNAP, "1", PART_MAX + 1, 0x61, refused(413, "PayloadTooLarge")),
                part("A", SNAP, "1", MIN_PART, 0x61, accepted_part(1, Some(2), 201)),
                part("A", SNAP, "2", MIN_PART, 0x62, accepted_part(2, Some(3), 201)),
                part("A", SNAP, "3", 2 * MIB - 1, 0x63, refused(400, "MalformedObject")),
                part("A", SNAP, "3", 2 * MIB + 1, 0x63, refused(400, "MalformedObject")),
                part("A", SNAP, "3", 2 * MIB, 0x63, accepted_part(3, None, 201)),
            ],
        ),
        case(
            "a_part_number_must_be_the_next_one",
            "Part numbers are decimals from 1 to 10000 (0, 10001 and a non-decimal are MalformedRequest, 400); a part past the next one expected, or past the last, is PartOutOfSequence (409) carrying next_part; a part for a snapshot never begun is UploadNotFound (404).",
            vec![
                part("A", SNAP, "1", MIN_PART, 0x61, refused(404, "UploadNotFound")),
                begin("A", SNAP, TOTAL, &sha, MIN_PART, uploading(MIN_PART, 3, 1)),
                part("A", SNAP, "0", MIN_PART, 0x61, refused(400, "MalformedRequest")),
                part("A", SNAP, "10001", MIN_PART, 0x61, refused(400, "MalformedRequest")),
                part("A", SNAP, "one", MIN_PART, 0x61, refused(400, "MalformedRequest")),
                part("A", SNAP, "2", MIN_PART, 0x62, refused_next(409, "PartOutOfSequence", 1)),
                part("A", SNAP, "4", MIN_PART, 0x61, refused_next(409, "PartOutOfSequence", 1)),
                part("A", SNAP, "1", MIN_PART, 0x61, accepted_part(1, Some(2), 201)),
                part("A", SNAP, "3", 2 * MIB, 0x63, refused_next(409, "PartOutOfSequence", 2)),
            ],
        ),
        case(
            "a_part_sent_again_is_idempotent_and_a_different_one_is_a_conflict",
            "A part already received with the same length and bytes answers 200 with the original next_part; the same length with other bytes is ObjectConflict (409).",
            vec![
                begin("A", SNAP, TOTAL, &sha, MIN_PART, uploading(MIN_PART, 3, 1)),
                part("A", SNAP, "1", MIN_PART, 0x61, accepted_part(1, Some(2), 201)),
                part("A", SNAP, "2", MIN_PART, 0x62, accepted_part(2, Some(3), 201)),
                part("A", SNAP, "1", MIN_PART, 0x61, accepted_part(1, Some(3), 200)),
                part("A", SNAP, "1", MIN_PART, 0x7a, refused(409, "ObjectConflict")),
            ],
        ),
        case(
            "complete_before_every_part_has_arrived_is_incomplete",
            "Complete with parts missing is UploadIncomplete (409) naming the first missing part, and keeps the upload so the client can send it.",
            run(vec![
                complete("A", SNAP, refused_next(409, "UploadIncomplete", 3)),
                part("A", SNAP, "3", 2 * MIB, 0x63, accepted_part(3, None, 201)),
                complete("A", SNAP, staged(TOTAL, 201)),
            ]),
        ),
        case(
            "a_digest_that_does_not_match_stages_nothing_and_discards_the_upload",
            "The client declared a digest that is not that of the bytes it sent. Complete is MalformedObject (400) and the server discards the upload: the next part and the next complete are UploadNotFound (404), a begin starts afresh (201, not staged), and the snapshot was never staged.",
            vec![
                begin("A", SNAP, TOTAL, &bad_sha, MIN_PART, uploading(MIN_PART, 3, 1)),
                part("A", SNAP, "1", MIN_PART, 0x61, accepted_part(1, Some(2), 201)),
                part("A", SNAP, "2", MIN_PART, 0x62, accepted_part(2, Some(3), 201)),
                part("A", SNAP, "3", 2 * MIB, 0x63, accepted_part(3, None, 201)),
                complete("A", SNAP, refused(400, "MalformedObject")),
                complete("A", SNAP, refused(404, "UploadNotFound")),
                part("A", SNAP, "3", 2 * MIB, 0x63, refused(404, "UploadNotFound")),
                begin("A", SNAP, TOTAL, &sha, MIN_PART, uploading(MIN_PART, 3, 1)),
            ],
        ),
        case(
            "a_container_below_the_part_minimum_is_one_small_last_part",
            "A container smaller than 5 MiB is a single, last part; the 5 MiB minimum applies to every part but the last. It stages.",
            vec![
                begin("A", SNAP, 1000, &sha_of(&[(1000, 0x41)]), MIN_PART, json!({"status": 201, "snapshot_id": SNAP, "state": "uploading", "part_size": num(MIN_PART), "part_count": "1", "next_part": "1", "expires_at": EXPIRES_AT})),
                part("A", SNAP, "1", 1000, 0x41, accepted_part(1, None, 201)),
                complete("A", SNAP, staged(1000, 201)),
            ],
        ),
        case(
            "an_upload_can_be_aborted",
            "Abort removes an upload in progress (200, aborted true) and is idempotent (200, aborted false); a part after it is UploadNotFound. Abort never removes a staged snapshot: after a completed upload it answers aborted false and the snapshot is still staged.",
            vec![
                begin("A", SNAP, TOTAL, &sha, MIN_PART, uploading(MIN_PART, 3, 1)),
                part("A", SNAP, "1", MIN_PART, 0x61, accepted_part(1, Some(2), 201)),
                abort("A", SNAP, json!({"status": 200, "snapshot_id": SNAP, "aborted": true})),
                abort("A", SNAP, json!({"status": 200, "snapshot_id": SNAP, "aborted": false})),
                part("A", SNAP, "2", MIN_PART, 0x62, refused(404, "UploadNotFound")),
                begin("A", SNAP, TOTAL, &sha, MIN_PART, uploading(MIN_PART, 3, 1)),
                part("A", SNAP, "1", MIN_PART, 0x61, accepted_part(1, Some(2), 201)),
                part("A", SNAP, "2", MIN_PART, 0x62, accepted_part(2, Some(3), 201)),
                part("A", SNAP, "3", 2 * MIB, 0x63, accepted_part(3, None, 201)),
                complete("A", SNAP, staged(TOTAL, 201)),
                abort("A", SNAP, json!({"status": 200, "snapshot_id": SNAP, "aborted": false})),
                begin("A", SNAP, TOTAL, &sha, MIN_PART, staged_answer(200)),
            ],
        ),
        case(
            "the_decision_order_of_begin",
            "The first failing step answers. A malformed body is judged before writes; a size over the maximum before the writes check; an exact retry of an upload in progress is 200 even when writes are suspended; a new upload when writes are suspended is WritesSuspended (403), and when the quota is exhausted QuotaExceeded (403) only after the conflict and writes checks.",
            vec![
                begin("A", SNAP, TOTAL, &sha, MIN_PART, uploading(MIN_PART, 3, 1)),
                when(json!({"writes_permitted": false}), begin_raw("A", OTHER, json!({"size": "0", "sha256": sha, "part_size": num(MIN_PART)}), refused(400, "MalformedRequest"))),
                when(json!({"writes_permitted": false}), begin("A", OTHER, MAX_SNAPSHOT + 1, &sha, PART_MAX, refused(413, "SnapshotTooLarge"))),
                when(json!({"writes_permitted": false}), begin("A", SNAP, TOTAL, &sha, MIN_PART, uploading_again(MIN_PART, 3, 1))),
                when(json!({"writes_permitted": false}), begin("A", SNAP, TOTAL, &bad_sha, MIN_PART, refused(409, "ObjectConflict"))),
                when(json!({"writes_permitted": false}), begin("A", OTHER, TOTAL, &sha, MIN_PART, refused(403, "WritesSuspended"))),
                when(json!({"quota_exceeded": true}), begin("A", SNAP, TOTAL, &sha, MIN_PART, uploading_again(MIN_PART, 3, 1))),
                when(json!({"quota_exceeded": true}), begin("A", OTHER, TOTAL, &sha, MIN_PART, refused(403, "QuotaExceeded"))),
                begin("A", OTHER, TOTAL, &sha, MIN_PART, json!({"status": 201, "snapshot_id": OTHER, "state": "uploading", "part_size": num(MIN_PART), "part_count": "3", "next_part": "1", "expires_at": EXPIRES_AT})),
            ],
        ),
        case(
            "the_decision_order_of_part_and_complete",
            "For a part: the upload is looked for before the sequence, the sequence before writes, and writes before the size. For complete: a retry after success is 200 even when writes are suspended; a new complete when writes are suspended is WritesSuspended (403) before UploadIncomplete.",
            vec![
                begin("A", SNAP, TOTAL, &sha, MIN_PART, uploading(MIN_PART, 3, 1)),
                when(json!({"writes_permitted": false}), part("A", OTHER, "1", MIN_PART, 0x61, refused(404, "UploadNotFound"))),
                when(json!({"writes_permitted": false}), part("A", SNAP, "2", MIN_PART, 0x62, refused_next(409, "PartOutOfSequence", 1))),
                when(json!({"writes_permitted": false}), part("A", SNAP, "1", MIN_PART - 1, 0x61, refused(403, "WritesSuspended"))),
                part("A", SNAP, "1", MIN_PART, 0x61, accepted_part(1, Some(2), 201)),
                part("A", SNAP, "2", MIN_PART, 0x62, accepted_part(2, Some(3), 201)),
                when(json!({"writes_permitted": false}), part("A", SNAP, "1", MIN_PART, 0x61, accepted_part(1, Some(3), 200))),
                when(json!({"writes_permitted": false}), complete("A", SNAP, refused(403, "WritesSuspended"))),
                part("A", SNAP, "3", 2 * MIB, 0x63, accepted_part(3, None, 201)),
                complete("A", SNAP, staged(TOTAL, 201)),
                when(json!({"writes_permitted": false}), complete("A", SNAP, staged(TOTAL, 200))),
            ],
        ),
        case(
            "a_device_with_a_recorded_halt_cannot_stage_in_parts",
            "After A reports a halt, A's begin, part and complete are ReplicaHalted (409) with the reported position, so a halted device cannot stage a snapshot by the other path; A's abort still works (it only cleans up); B and an account credential are not affected.",
            vec![
                begin("A", SNAP, TOTAL, &sha, MIN_PART, uploading(MIN_PART, 3, 1)),
                halt("A", 5),
                begin("A", OTHER, TOTAL, &sha, MIN_PART, halted(5)),
                part("A", SNAP, "1", MIN_PART, 0x61, halted(5)),
                complete("A", SNAP, halted(5)),
                part("B", SNAP, "1", MIN_PART, 0x61, accepted_part(1, Some(2), 201)),
                begin("account", OTHER, TOTAL, &sha, MIN_PART, json!({"status": 201, "snapshot_id": OTHER, "state": "uploading", "part_size": num(MIN_PART), "part_count": "3", "next_part": "1", "expires_at": EXPIRES_AT})),
                abort("A", SNAP, json!({"status": 200, "snapshot_id": SNAP, "aborted": true})),
            ],
        ),
        case(
            "a_snapshot_over_the_single_request_limit_is_staged_in_parts_and_the_result_is_the_same",
            "stageSnapshot refuses a container over the operator's snapshot_single_request_max_bytes with PayloadTooLarge (413) carrying the limit; one at the limit stages. The same bytes staged through the parts path give the same staged snapshot, so beginning with their size and digest is 200 and says staged.",
            vec![
                stage_single("A", SNAP, SINGLE_MAX + 1, 0x41, json!({"status": 413, "error": "PayloadTooLarge", "details": {"single_request_max_bytes": num(SINGLE_MAX)}})),
                stage_single("A", SNAP, SINGLE_MAX, 0x41, staged(SINGLE_MAX, 201)),
                begin("A", SNAP, SINGLE_MAX, &sha_of(&[(SINGLE_MAX, 0x41)]), MIN_PART, staged_answer(200)),
                begin("A", OTHER, SINGLE_MAX, &sha_of(&[(SINGLE_MAX, 0x42)]), MIN_PART, json!({"status": 201, "snapshot_id": OTHER, "state": "uploading", "part_size": num(MIN_PART), "part_count": "2", "next_part": "1", "expires_at": EXPIRES_AT})),
            ],
        ),
    ]
}

fn document() -> Value {
    json!({
        "vectors_version": 1,
        "spec": "docs/architecture/sync-service-protocol.md 4.13, 7.2 and 11.2",
        "synthetic": "Every value is synthetic. Parts and containers are described by a length and a repeated byte; the SHA-256 a client declares is that of those bytes. No key, signature or sealed object is in this file.",
        "how_to_read": "Each case is a list of steps against ONE server that starts empty: nothing staged, no halts, writes permitted, quota available. The operator limits are fixed for the file: snapshot_part_max_bytes and snapshot_single_request_max_bytes are 8388608, snapshot_upload_expiry_seconds is 604800, and the server's clock is 2026-11-03T12:00:00Z, so a new upload expires at 2026-11-10T12:00:00Z. A step may change the conditions for that step only ('conditions': writes_permitted false, quota_exceeded true). 'credential' is 'A' or 'B' (devices) or 'account' (an account credential before the genesis). 'begin', 'part', 'complete' and 'abort' are beginSnapshotUpload, putSnapshotPart, completeSnapshotUpload and abortSnapshotUpload; 'stageSnapshot' is the single-request operation, which stands in for a container by its length and fill byte; 'reportHalt' records a halt for the credential's device. 'bytes' is {length, fill}: the part or container is 'length' copies of the byte 'fill'. 'expect' is the outcome: a 'status' and the response members named, or 'error' for a non-2xx response, or 'details'. The clear-header check of part 1 is the container check stageSnapshot shares and is not in this file.",
        "decision_order_of_begin": [
            "1 authenticate (not exercised: every credential here is valid)",
            "2 the halt gate: ReplicaHalted",
            "3 parse the body: MalformedRequest",
            "4 size: SnapshotTooLarge; part_size above the operator's maximum PayloadTooLarge, below 5 MiB MalformedRequest",
            "5 exact retry: 200 with the state; another snapshot or upload under the id: ObjectConflict",
            "6 writes permitted: WritesSuspended",
            "7 quota: QuotaExceeded",
            "8 begin: 201"
        ],
        "decision_order_of_part": [
            "1 the halt gate",
            "2 part_number form: MalformedRequest",
            "3 the upload exists: UploadNotFound",
            "4 sequence: a received part 200 or ObjectConflict; a part past the next PartOutOfSequence",
            "5 writes permitted: WritesSuspended",
            "6 size: PayloadTooLarge, then MalformedObject",
            "7 store"
        ],
        "decision_order_of_complete": [
            "1 the halt gate",
            "2 the upload exists, or this is a retry after success (200): UploadNotFound",
            "3 writes permitted: WritesSuspended",
            "4 all parts received: UploadIncomplete",
            "5 the whole container's length and SHA-256: MalformedObject, and the upload is discarded",
            "6 stage: 201"
        ],
        "cases": all_cases(),
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

// ---------- the reference server model ----------

struct Upload {
    size: u64,
    sha: String,
    part_size: u64,
    count: u64,
    next: u64,
    hasher: Sha256,
    received: Vec<(u64, String)>,
}

#[derive(Default)]
struct Server {
    uploads: Vec<(String, Upload)>,
    staged: Vec<(String, u64, String)>,
    halts: Vec<String>,
}

fn err(status: u16, code: &str) -> Value {
    json!({"status": status, "error": code})
}
fn err_next(status: u16, code: &str, next: u64) -> Value {
    json!({"status": status, "error": code, "details": {"next_part": num(next)}})
}
fn as_u64(v: &Value) -> Option<u64> {
    let s = v.as_str()?;
    (!s.is_empty()
        && s.len() <= 19
        && s.bytes().all(|b| b.is_ascii_digit())
        && !(s.len() > 1 && s.starts_with('0')))
    .then(|| s.parse().ok())
    .flatten()
}
fn bytes_of(v: &Value) -> (u64, u8) {
    (
        as_u64(&v["length"]).unwrap(),
        v["fill"].as_u64().unwrap() as u8,
    )
}
fn sha_bytes(len: u64, fill: u8) -> String {
    hex(&Sha256::digest(vec![fill; len as usize]))
}

impl Server {
    fn halted(&self, cred: &str) -> Option<Value> {
        self.halts.iter().any(|h| h == cred).then(
            || json!({"status": 409, "error": "ReplicaHalted", "details": {"stopped_at": "5"}}),
        )
    }

    fn upload(&mut self, snap: &str) -> Option<&mut Upload> {
        self.uploads
            .iter_mut()
            .find(|(s, _)| s == snap)
            .map(|(_, u)| u)
    }

    fn begin(
        &mut self,
        cred: &str,
        snap: &str,
        body: &Value,
        writes: bool,
        quota_exceeded: bool,
    ) -> Value {
        if let Some(h) = self.halted(cred) {
            return h;
        }
        let allowed = ["size", "sha256", "part_size"];
        let Some(obj) = body.as_object() else {
            return err(400, "MalformedRequest");
        };
        if obj.keys().any(|k| !allowed.contains(&k.as_str()))
            || allowed.iter().any(|k| !obj.contains_key(*k))
        {
            return err(400, "MalformedRequest");
        }
        let (Some(size), Some(part_size)) = (as_u64(&body["size"]), as_u64(&body["part_size"]))
        else {
            return err(400, "MalformedRequest");
        };
        let sha = body["sha256"].as_str().unwrap_or("");
        if size == 0
            || sha.len() != 64
            || !sha
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return err(400, "MalformedRequest");
        }
        if size > MAX_SNAPSHOT {
            return err(413, "SnapshotTooLarge");
        }
        if part_size > PART_MAX {
            return err(413, "PayloadTooLarge");
        }
        let count = size.div_ceil(part_size.max(1));
        if part_size < MIN_PART || count > 10_000 {
            return err(400, "MalformedRequest");
        }
        if let Some((_, s, d)) = self.staged.iter().find(|(id, _, _)| id == snap) {
            return if *s == size && d == sha {
                json!({"status": 200, "snapshot_id": snap, "state": "staged", "part_size": null, "part_count": null, "next_part": null, "expires_at": null})
            } else {
                err(409, "ObjectConflict")
            };
        }
        if let Some(u) = self.upload(snap) {
            return if u.size == size && u.sha == sha && u.part_size == part_size {
                json!({"status": 200, "snapshot_id": snap, "state": "uploading", "part_size": num(part_size), "part_count": num(u.count), "next_part": num(u.next), "expires_at": EXPIRES_AT})
            } else {
                err(409, "ObjectConflict")
            };
        }
        if !writes {
            return err(403, "WritesSuspended");
        }
        if quota_exceeded {
            return err(403, "QuotaExceeded");
        }
        self.uploads.push((
            snap.to_owned(),
            Upload {
                size,
                sha: sha.to_owned(),
                part_size,
                count,
                next: 1,
                hasher: Sha256::new(),
                received: vec![],
            },
        ));
        json!({"status": 201, "snapshot_id": snap, "state": "uploading", "part_size": num(part_size), "part_count": num(count), "next_part": "1", "expires_at": EXPIRES_AT})
    }

    fn part(
        &mut self,
        cred: &str,
        snap: &str,
        number: &str,
        bytes: (u64, u8),
        writes: bool,
    ) -> Value {
        if let Some(h) = self.halted(cred) {
            return h;
        }
        let Some(n) =
            as_u64(&Value::String(number.to_owned())).filter(|n| (1..=10_000).contains(n))
        else {
            return err(400, "MalformedRequest");
        };
        let Some(u) = self.upload(snap) else {
            return err(404, "UploadNotFound");
        };
        let (len, fill) = bytes;
        if n < u.next {
            let digest = sha_bytes(len, fill);
            return match u.received.get(n as usize - 1) {
                Some((l, d)) if *l == len && *d == digest => {
                    json!({"status": 200, "part_number": num(n), "next_part": if u.next > u.count { Value::Null } else { num(u.next) }})
                }
                _ => err(409, "ObjectConflict"),
            };
        }
        if n > u.next || n > u.count {
            return err_next(409, "PartOutOfSequence", u.next);
        }
        if !writes {
            return err(403, "WritesSuspended");
        }
        if len > PART_MAX {
            return err(413, "PayloadTooLarge");
        }
        let expected = if n == u.count {
            u.size - (u.count - 1) * u.part_size
        } else {
            u.part_size
        };
        if len != expected {
            return err(400, "MalformedObject");
        }
        u.hasher.update(vec![fill; len as usize]);
        u.received.push((len, sha_bytes(len, fill)));
        u.next += 1;
        json!({"status": 201, "part_number": num(n), "next_part": if u.next > u.count { Value::Null } else { num(u.next) }})
    }

    fn complete(&mut self, cred: &str, snap: &str, writes: bool) -> Value {
        if let Some(h) = self.halted(cred) {
            return h;
        }
        let at = self.uploads.iter().position(|(s, _)| s == snap);
        let Some(at) = at else {
            return match self.staged.iter().find(|(id, _, _)| id == snap) {
                Some((_, size, _)) => {
                    json!({"status": 200, "snapshot_id": snap, "size": num(*size)})
                }
                None => err(404, "UploadNotFound"),
            };
        };
        if !writes {
            return err(403, "WritesSuspended");
        }
        let u = &self.uploads[at].1;
        if u.next <= u.count {
            return err_next(409, "UploadIncomplete", u.next);
        }
        let digest = hex(&u.hasher.clone().finalize());
        let total: u64 = u.received.iter().map(|(l, _)| *l).sum();
        let (size, sha) = (u.size, u.sha.clone());
        let (_, upload) = self.uploads.remove(at);
        drop(upload);
        if total != size || digest != sha {
            return err(400, "MalformedObject");
        }
        self.staged.push((snap.to_owned(), size, sha));
        json!({"status": 201, "snapshot_id": snap, "size": num(size)})
    }

    fn abort(&mut self, snap: &str) -> Value {
        let before = self.uploads.len();
        self.uploads.retain(|(s, _)| s != snap);
        json!({"status": 200, "snapshot_id": snap, "aborted": self.uploads.len() < before})
    }

    fn stage_single(&mut self, snap: &str, bytes: (u64, u8)) -> Value {
        if bytes.0 > SINGLE_MAX {
            return json!({"status": 413, "error": "PayloadTooLarge", "details": {"single_request_max_bytes": num(SINGLE_MAX)}});
        }
        self.staged
            .push((snap.to_owned(), bytes.0, sha_bytes(bytes.0, bytes.1)));
        json!({"status": 201, "snapshot_id": snap, "size": num(bytes.0)})
    }

    fn step(&mut self, step: &Value) -> Value {
        let cond = &step["conditions"];
        let writes = cond["writes_permitted"] != json!(false);
        let quota = cond["quota_exceeded"] == json!(true);
        let cred = step["credential"].as_str().unwrap();
        let snap = step["snapshot_id"].as_str().unwrap_or("");
        match step["op"].as_str().unwrap() {
            "begin" => self.begin(cred, snap, &step["body"], writes, quota),
            "part" => self.part(
                cred,
                snap,
                step["part_number"].as_str().unwrap(),
                bytes_of(&step["bytes"]),
                writes,
            ),
            "complete" => self.complete(cred, snap, writes),
            "abort" => self.abort(snap),
            "stageSnapshot" => self.stage_single(snap, bytes_of(&step["bytes"])),
            "reportHalt" => {
                self.halts.push(cred.to_owned());
                json!({"status": 200, "stopped_at": step["stopped_at"]})
            }
            other => panic!("unknown op {other}"),
        }
    }
}

#[test]
fn a_reference_server_agrees_with_every_expected_outcome() {
    let doc: Value = serde_json::from_str(VECTORS).unwrap();
    let cases = doc["cases"].as_array().unwrap();
    assert!(cases.len() >= 16, "the file has its cases");
    for case in cases {
        let mut server = Server::default();
        for (i, step) in case["steps"].as_array().unwrap().iter().enumerate() {
            let got = server.step(step);
            assert_eq!(&got, &step["expect"], "{}: step {i}", case["name"]);
        }
    }
}

#[test]
fn the_vectors_reach_every_error_and_every_operation_the_addition_introduced() {
    let doc: Value = serde_json::from_str(VECTORS).unwrap();
    let registry: Value = serde_json::from_str(REGISTRY).unwrap();
    let (mut seen, mut ops) = (Vec::new(), Vec::new());
    for case in doc["cases"].as_array().unwrap() {
        for step in case["steps"].as_array().unwrap() {
            ops.push(step["op"].as_str().unwrap().to_owned());
            if let Some(e) = step["expect"]["error"].as_str() {
                let status = registry["errors"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|r| r["code"] == e)
                    .unwrap_or_else(|| panic!("{e} is in the registry"))["status"]
                    .as_u64();
                assert_eq!(step["expect"]["status"].as_u64(), status, "{e}");
                seen.push(e.to_owned());
            }
        }
    }
    for code in [
        "UploadNotFound",
        "PartOutOfSequence",
        "UploadIncomplete",
        "SnapshotTooLarge",
        "PayloadTooLarge",
        "ObjectConflict",
        "MalformedObject",
        "MalformedRequest",
        "WritesSuspended",
        "QuotaExceeded",
        "ReplicaHalted",
    ] {
        assert!(seen.iter().any(|s| s == code), "no vector for {code}");
    }
    for op in [
        "begin",
        "part",
        "complete",
        "abort",
        "stageSnapshot",
        "reportHalt",
    ] {
        assert!(ops.iter().any(|o| o == op), "no vector for {op}");
    }
    for op in [
        "beginSnapshotUpload",
        "putSnapshotPart",
        "completeSnapshotUpload",
        "abortSnapshotUpload",
    ] {
        assert!(
            registry["operations"]
                .as_array()
                .unwrap()
                .iter()
                .any(|o| o["op"] == op),
            "{op} is in the machine-readable list"
        );
    }
}
