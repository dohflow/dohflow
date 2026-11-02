//! Public conformance vectors for the void additions to the Sync service protocol
//! (personal-cfo-g3m.33; `docs/architecture/sync-service-protocol.md` 4.11, 4.12 and 11.2).
//!
//! Each case is a sequence of requests to ONE server that starts empty (a vault with the sample
//! membership chain, head `0`), and the status, error code and result the server must give. The
//! file holds only public data: the chain's signed records and the void records are in it as
//! base64url; the seeds that signed them live only in this generator.
//!
//! Three guards: the committed file equals the generator's output; a reference server model,
//! written from the protocol text over the JSON alone and calling `verify_void` for the signer
//! (the one check a server must not reimplement), agrees with every expected outcome; and every
//! error the additions introduced, and every step of `appendVoid`'s order, has a vector.
//!
//! Regenerate with
//! `DOHFLOW_UPDATE_VECTORS=1 cargo test -p sync-core --test sync_service_void_vectors`.

mod common;

use common::{sign, Chain, Dev, VAULT};
use serde_json::{json, Map, Value};
use sync_core::membership::{verify_chain, EpochAtPosition, SignedRecord, SUITE_ID};
use sync_core::void::{verify_void, VoidBody, VoidError, VoidReason, MAX_VOID_BODY};

const VECTORS: &str = include_str!("../../../docs/architecture/vectors/sync-service-void-v1.json");
const VECTORS_PATH: &str = "../../docs/architecture/vectors/sync-service-void-v1.json";
const REGISTRY: &str = include_str!("../../../docs/architecture/sync-service-protocol.json");

// ---------- encodings ----------

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn b64_encode(bytes: &[u8]) -> String {
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n =
            chunk.iter().fold(0_u32, |a, b| (a << 8) | u32::from(*b)) << (8 * (3 - chunk.len()));
        for i in 0..=chunk.len() {
            out.push(B64[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    out
}

fn b64_decode(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0_u32, 0);
    for c in text.bytes() {
        let v = B64.iter().position(|b| *b == c).expect("base64url") as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    out
}

fn uuid_text(id: &[u8; 16]) -> String {
    let h = hex(id);
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

fn num(n: u64) -> Value {
    Value::String(n.to_string())
}

// ---------- the generator ----------

/// The command id and envelope digest of the sample envelope the index holds at `seq`.
fn command(seq: u64) -> [u8; 16] {
    [0xc0 + seq as u8; 16]
}
fn digest(seq: u64) -> [u8; 32] {
    [0xd0 + seq as u8; 32]
}

fn void_body(signer: &Dev, seq: u64, issued: i64) -> VoidBody {
    VoidBody {
        suite_id: SUITE_ID,
        vault_id: VAULT,
        seq,
        command_id: command(seq),
        envelope_digest: digest(seq),
        reason: VoidReason::ReceiverCheck,
        signer_fingerprint: signer.fingerprint(),
        issued_at: 1_790_000_000_000_000 + issued,
    }
}

fn record(signer: &Dev, body: &VoidBody) -> Vec<u8> {
    sign(&body.encode(), &signer.signing)
}

fn void_id(body: &VoidBody) -> String {
    hex(&body.id())
}

fn push(seq: u64) -> Value {
    json!({
        "op": "pushEnvelope",
        "command_id": hex(&command(seq)),
        "envelope_digest": hex(&digest(seq)),
        "expect": {"status": 200, "seq": num(seq)},
    })
}

fn append(credential: &str, record: &[u8], expect: Value) -> Value {
    json!({"op": "appendVoid", "credential": credential, "record": b64_encode(record), "expect": expect})
}

fn append_when(conditions: Value, mut step: Value) -> Value {
    step["conditions"] = conditions;
    step
}

fn created(body: &VoidBody, position: u64) -> Value {
    json!({"status": 201, "void_id": void_id(body), "position": num(position), "head_seq": num(position)})
}

fn retried(body: &VoidBody, position: u64, head: u64) -> Value {
    json!({"status": 200, "void_id": void_id(body), "position": num(position), "head_seq": num(head)})
}

fn refused(status: u16, error: &str) -> Value {
    json!({"status": status, "error": error})
}

fn pull(
    after: u64,
    limit: u64,
    entries: &[&str],
    voids: &[(u64, &VoidBody)],
    has_more: bool,
    head: u64,
) -> Value {
    let mut expect = json!({
        "status": 200,
        "head_seq": num(head),
        "has_more": has_more,
        "entries": entries,
    });
    if !voids.is_empty() {
        expect["voids"] = voids
            .iter()
            .map(|(p, b)| json!({"position": num(*p), "void_id": void_id(b)}))
            .collect();
    }
    json!({"op": "pullEnvelopes", "credential": "A", "after_seq": num(after), "limit": limit, "expect": expect})
}

fn replay(seq: u64, claims: &[&VoidBody]) -> Value {
    let mut expect = json!({"status": 200, "seq": num(seq), "duplicate": true});
    if !claims.is_empty() {
        expect["void_claims"] = claims.iter().map(|b| Value::String(void_id(b))).collect();
    }
    json!({"op": "replayPush", "seq": num(seq), "expect": expect})
}

fn report(credential: &str, device: &str, stopped_at: Option<u64>, expect: Value) -> Value {
    json!({
        "op": "reportHalt",
        "credential": credential,
        "device": device,
        "stopped_at": stopped_at.map(num),
        "expect": expect,
    })
}

/// A report that is accepted: the response names the device by its id.
fn reported(dev: &Dev, name: &str, stopped_at: Option<u64>) -> Value {
    report(
        name,
        name,
        stopped_at,
        json!({"status": 200, "device_id": uuid_text(&dev.device_id), "stopped_at": stopped_at.map(num)}),
    )
}

fn gated(request: &str, credential: &str, expect: Value) -> Value {
    json!({"op": request, "credential": credential, "expect": expect})
}

fn halted(stopped_at: u64) -> Value {
    json!({"status": 409, "error": "ReplicaHalted", "details": {"stopped_at": num(stopped_at)}})
}

fn passes() -> Value {
    json!({"gate": "pass"})
}

fn case(name: &str, description: &str, steps: Vec<Value>) -> Value {
    json!({"name": name, "description": description, "steps": steps})
}

fn all_cases(a: &Dev, b: &Dev, c: &Dev) -> Vec<Value> {
    let five: Vec<Value> = (1..=5).map(push).collect();
    let with_five = |more: Vec<Value>| {
        let mut steps = five.clone();
        steps.extend(more);
        steps
    };
    let by_a = |seq, issued| void_body(a, seq, issued);
    let by_b = |seq, issued| void_body(b, seq, issued);

    let v3 = by_a(3, 0);
    let v3_again = by_a(3, 1);
    let v3_b = by_b(3, 2);
    let v2 = by_a(2, 0);
    let v4 = by_a(4, 0);
    let v4_b = by_b(4, 0);
    let v1 = by_a(1, 0);
    let rec = |dev: &Dev, body: &VoidBody| record(dev, body);

    let mut wrong_command = by_a(3, 0);
    wrong_command.command_id = [0x99; 16];
    let mut wrong_digest = by_a(3, 0);
    wrong_digest.envelope_digest = [0x99; 32];
    let beyond = by_a(9, 0);
    let zero = by_a(0, 0);
    let mut other_suite = by_a(3, 0);
    other_suite.suite_id = 2;
    let mut other_vault = by_a(3, 0);
    other_vault.vault_id = [0x77; 16];
    let by_c = void_body(c, 3, 0);

    let good = rec(a, &v3);
    let mut bad_signature = good.clone();
    let last = bad_signature.len() - 1;
    bad_signature[last] ^= 1;
    let truncated = good[..good.len() - 1].to_vec();
    let mut other_version = v3.encode();
    // The u16 protocol_version follows the length-prefixed label ("dohflow/sync-void/v1").
    other_version[22 + 1] = 2;
    let other_version = sign(&other_version, &a.signing);

    vec![
        case(
            "a_member_signed_void_is_appended_at_the_next_seq",
            "Five envelopes are accepted. A signs a void for position 3: it is appended as position 6, which becomes the head. The same record again is an exact retry: 200 with the original position. A pull then shows the five envelopes and the void by position.",
            with_five(vec![
                append("A", &rec(a, &v3), created(&v3, 6)),
                append("A", &rec(a, &v3), retried(&v3, 6, 6)),
                pull(0, 500, &["envelope:1", "envelope:2", "envelope:3", "envelope:4", "envelope:5"], &[(6, &v3)], false, 6),
            ]),
        ),
        case(
            "any_member_may_send_and_any_member_may_sign",
            "B's credential carries a void that A signed, and A's credential carries one that B signed: the signer need not be the credential's device.",
            with_five(vec![
                append("B", &rec(a, &v3), created(&v3, 6)),
                append("A", &rec(b, &v4_b), created(&v4_b, 7)),
            ]),
        ),
        case(
            "two_distinct_claims_for_one_position_are_both_kept",
            "A and B each sign a void for position 3, and A signs a second, different one (another issued_at). All three are kept at positions 6, 7 and 8; none is deduped as a second void for the same seq. A replay of the envelope at 3 returns every claim in append order.",
            with_five(vec![
                append("A", &rec(a, &v3), created(&v3, 6)),
                append("B", &rec(b, &v3_b), created(&v3_b, 7)),
                append("A", &rec(a, &v3_again), created(&v3_again, 8)),
                replay(3, &[&v3, &v3_b, &v3_again]),
                replay(2, &[]),
                pull(0, 500, &["envelope:1", "envelope:2", "envelope:3", "envelope:4", "envelope:5"], &[(6, &v3), (7, &v3_b), (8, &v3_again)], false, 8),
            ]),
        ),
        case(
            "an_earlier_void_never_blocks_a_later_one",
            "A void for position 2 is appended; replicas decide whether it is honored, so the server must still accept a void for position 4, which that void's range would cover if it were honored, and one for position 1.",
            with_five(vec![
                append("A", &rec(a, &v2), created(&v2, 6)),
                append("A", &rec(a, &v4), created(&v4, 7)),
                append("A", &rec(a, &v1), created(&v1, 8)),
            ]),
        ),
        case(
            "a_signer_who_is_not_a_member_is_refused",
            "C is not in the chain. C's void is VoidSignerNotAuthorized (403) whoever carries it, the head does not move, and a pull has no voids member.",
            with_five(vec![
                append("A", &rec(c, &by_c), refused(403, "VoidSignerNotAuthorized")),
                pull(0, 500, &["envelope:1", "envelope:2", "envelope:3", "envelope:4", "envelope:5"], &[], false, 5),
                append("A", &rec(a, &v3), created(&v3, 6)),
            ]),
        ),
        case(
            "a_signature_that_does_not_verify_is_refused",
            "One flipped signature bit is VoidSignatureInvalid (400).",
            with_five(vec![append("A", &bad_signature, refused(400, "VoidSignatureInvalid"))]),
        ),
        case(
            "a_void_must_name_the_envelope_the_index_holds",
            "A void whose command id or envelope digest differs from the index's record, one for a position beyond the head, position 0, and one for a position a void occupies (not a command envelope) are VoidTargetMismatch (409).",
            with_five(vec![
                append("A", &rec(a, &wrong_command), refused(409, "VoidTargetMismatch")),
                append("A", &rec(a, &wrong_digest), refused(409, "VoidTargetMismatch")),
                append("A", &rec(a, &beyond), refused(409, "VoidTargetMismatch")),
                append("A", &rec(a, &zero), refused(409, "VoidTargetMismatch")),
                append("A", &rec(a, &v3), created(&v3, 6)),
                append("A", &rec(a, &void_body(a, 6, 0)), refused(409, "VoidTargetMismatch")),
            ]),
        ),
        case(
            "a_record_that_cannot_be_parsed_is_refused",
            "A truncated record, a protocol version other than 1 inside the body, and a void for another vault are MalformedObject (400); a suite other than 1 is UnsupportedSuite (400). Each is refused before its signer is looked at.",
            with_five(vec![
                append("A", &truncated, refused(400, "MalformedObject")),
                append("A", &other_version, refused(400, "MalformedObject")),
                append("A", &rec(a, &other_vault), refused(400, "MalformedObject")),
                append("A", &rec(a, &other_suite), refused(400, "UnsupportedSuite")),
                append("A", &rec(c, &{
                    let mut v = by_c.clone();
                    v.suite_id = 2;
                    v
                }), refused(400, "UnsupportedSuite")),
            ]),
        ),
        case(
            "the_decision_order_of_appendvoid",
            "The first failing step answers. A truncated record, a void for another vault and a suite other than 1 are answered even when writes are suspended (MalformedObject, UnsupportedSuite). An exact retry of an appended void is 200 even when writes are suspended; a new void then is WritesSuspended before its signer or its target is looked at; with writes allowed a non-member signer is answered before a mismatched target, a bad signature before a mismatched target, and a mismatched target before QuotaExceeded.",
            with_five(vec![
                append("A", &rec(a, &v3), created(&v3, 6)),
                append_when(json!({"writes_permitted": false}), append("A", &truncated, refused(400, "MalformedObject"))),
                append_when(json!({"writes_permitted": false}), append("A", &rec(a, &other_vault), refused(400, "MalformedObject"))),
                append_when(json!({"writes_permitted": false}), append("A", &rec(a, &other_suite), refused(400, "UnsupportedSuite"))),
                append_when(json!({"writes_permitted": false}), append("A", &rec(a, &v3), retried(&v3, 6, 6))),
                append_when(json!({"writes_permitted": false}), append("A", &rec(c, &by_c), refused(403, "WritesSuspended"))),
                append_when(json!({"writes_permitted": false}), append("A", &rec(a, &wrong_command), refused(403, "WritesSuspended"))),
                append("A", &rec(c, &{
                    let mut v = by_c.clone();
                    v.command_id = [0x99; 16];
                    v
                }), refused(403, "VoidSignerNotAuthorized")),
                append("A", &{
                    let mut r = rec(a, &wrong_command);
                    let n = r.len() - 1;
                    r[n] ^= 1;
                    r
                }, refused(400, "VoidSignatureInvalid")),
                append_when(json!({"quota_exceeded": true}), append("A", &rec(a, &wrong_command), refused(409, "VoidTargetMismatch"))),
                append_when(json!({"quota_exceeded": true}), append("A", &rec(a, &v4), refused(403, "QuotaExceeded"))),
                append_when(json!({"quota_exceeded": true}), append("A", &rec(a, &v3), retried(&v3, 6, 6))),
                append("A", &rec(a, &v4), created(&v4, 7)),
            ]),
        ),
        case(
            "voids_and_envelopes_share_one_sequence_in_pulls",
            "Envelopes 1 and 2, a void at 3 (naming 2), envelopes 4 and 5, and an epoch entry at the head, then a void at 7 (naming 5). A full pull lists every position once: the epoch entry follows the log object whose seq is its position. The voids member is present only on pages that hold a void.",
            vec![
                push(1),
                push(2),
                append("A", &rec(a, &v2), created(&v2, 3)),
                push(4),
                push(5),
                json!({"op": "recordEpochEntry", "expect": {"position": num(5)}}),
                append("A", &rec(a, &by_a(5, 0)), created(&by_a(5, 0), 6)),
                pull(
                    0,
                    500,
                    &["envelope:1", "envelope:2", "envelope:4", "envelope:5", "epoch:5"],
                    &[(3, &v2), (6, &by_a(5, 0))],
                    false,
                    6,
                ),
            ],
        ),
        case(
            "a_page_can_hold_only_a_void",
            "Envelopes 1 to 3 and a void at 4, in pages of three log objects. The second page has no envelope entry and carries the void with has_more false, so a client that stops on an empty entries list would lose it; the page before it says has_more true.",
            vec![
                push(1),
                push(2),
                push(3),
                append("A", &rec(a, &by_a(3, 0)), created(&by_a(3, 0), 4)),
                pull(0, 3, &["envelope:1", "envelope:2", "envelope:3"], &[], true, 4),
                pull(3, 3, &[], &[(4, &by_a(3, 0))], false, 4),
            ],
        ),
        case(
            "a_halt_is_reported_refused_and_cleared",
            "A reports a stop at 3. A's stageSnapshot, publishSnapshot and installEpoch are then ReplicaHalted (409) with the reported position; B's pass the gate, an account credential passes (it has no device), and A can still append a void, report again (the latest report wins) and pull. A reports null and A's requests pass the gate again.",
            with_five(vec![
                gated("stageSnapshot", "A", passes()),
                reported(a, "A", Some(3)),
                reported(a, "A", Some(3)),
                gated("stageSnapshot", "A", halted(3)),
                gated("publishSnapshot", "A", halted(3)),
                gated("installEpoch", "A", halted(3)),
                gated("stageSnapshot", "B", passes()),
                gated("installEpoch", "account", passes()),
                append("A", &rec(a, &v3), created(&v3, 6)),
                reported(a, "A", Some(4)),
                gated("publishSnapshot", "A", halted(4)),
                pull(5, 500, &[], &[(6, &v3)], false, 6),
                reported(a, "A", None),
                gated("stageSnapshot", "A", passes()),
                gated("publishSnapshot", "A", passes()),
                gated("installEpoch", "A", passes()),
            ]),
        ),
        case(
            "a_device_reports_only_its_own_halt",
            "B cannot report A's halt (Forbidden, 403) or clear it, and a refused report records nothing.",
            with_five(vec![
                reported(a, "A", Some(2)),
                report("B", "A", None, refused(403, "Forbidden")),
                report("B", "A", Some(4), refused(403, "Forbidden")),
                gated("stageSnapshot", "A", halted(2)),
                gated("stageSnapshot", "B", passes()),
            ]),
        ),
    ]
}

fn document() -> Value {
    let chain = common::chain();
    let (a, b, c) = (chain_dev(&chain, 'A'), chain_dev(&chain, 'B'), Dev::new(3));
    let cases = all_cases(&a, &b, &c);
    let devices: Map<String, Value> = [("A", &a), ("B", &b), ("C", &c)]
        .into_iter()
        .map(|(name, d)| {
            (
                name.to_owned(),
                json!({"device_id": uuid_text(&d.device_id)}),
            )
        })
        .collect();
    json!({
        "vectors_version": 1,
        "spec": "docs/architecture/sync-service-protocol.md 4.11, 4.12 and 11.2; ADR 0074 Addendum D and E; docs/architecture/vault-format.md C.10 \"Void records\"",
        "synthetic": "Every value is synthetic. The chain, its keys and the signatures are made from fixed seeds that live only in the generator (crates/sync-core/tests/sync_service_void_vectors.rs); the file holds the signed records. Command ids and envelope digests are repeated bytes.",
        "how_to_read": "Each case is a list of steps against ONE server that starts empty: a vault with the 'chain' below, head 0, no halts, writes permitted and no quota exceeded. A step may change those with 'conditions' (writes_permitted false, quota_exceeded true), for that step only. 'credential' names the device whose credential carries the request ('A', 'B', or 'account' for an account credential). 'pushEnvelope' stands for an accepted envelope: it appends an identity (command_id, envelope_digest) to the accepted-envelope index at head + 1, so cases need no sealed envelope. 'recordEpochEntry' stands for an installed epoch record at the current head (an epoch entry; it takes no seq). 'replayPush' is an exact replay of the envelope accepted at 'seq'. 'stageSnapshot', 'publishSnapshot' and 'installEpoch' stand for those requests arriving at the halt gate of section 4.12: the outcome is the gate's refusal, or {\"gate\": \"pass\"} when the request goes on to its own decision procedure. 'expect' is the outcome: a 'status' and the response members named, or 'error' for a non-2xx response, or 'details'. A pull's 'entries' lists the page's envelope and epoch entries as 'envelope:SEQ' and 'epoch:POSITION', and its 'voids' (absent when the page holds none) lists the voids by position and void id; a client merges them by seq. A 'void_id' is SHA-256 of the void body. In 'reportHalt', 'device' names the {device_id} in the path.",
        "decision_order_of_appendvoid": [
            "1 authenticate (not exercised: every credential here is valid)",
            "2 parse: MalformedObject (framing, body, a protocol version other than 1, another vault), UnsupportedSuite",
            "3 exact retry: a void with this void_id is in the log: 200 with its position",
            "4 writes permitted: WritesSuspended",
            "5 signer: VoidSignerNotAuthorized, then VoidSignatureInvalid (the verify_void of sync-core)",
            "6 target: VoidTargetMismatch",
            "7 quota: QuotaExceeded",
            "8 append at position head + 1"
        ],
        "chain": {
            "vault_id": uuid_text(&VAULT),
            "devices": devices,
            "membership_records": chain.membership_records.iter().map(|r| b64_encode(r)).collect::<Vec<_>>(),
            "epoch_records": chain.epoch_records.iter().map(|r| b64_encode(r)).collect::<Vec<_>>(),
        },
        "cases": cases,
    })
}

/// The `Dev` a chain was built from.
fn chain_dev(chain: &Chain, which: char) -> Dev {
    let d = if which == 'A' { &chain.a } else { &chain.b };
    Dev::new(d.device_id[15])
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

#[derive(Clone)]
enum Object {
    Envelope(u64),
    Void(u64),
    Epoch(u64),
}

struct VoidEntry {
    id: String,
    position: u64,
    target: u64,
}

struct Server {
    state: sync_core::membership::VerifiedState,
    at_end: EpochAtPosition,
    devices: Vec<(String, String)>,
    head: u64,
    log: Vec<Object>,
    index: Vec<(u64, String, String)>,
    voids: Vec<VoidEntry>,
    halts: Vec<(String, u64)>,
}

fn error(status: u16, code: &str) -> Value {
    json!({"status": status, "error": code})
}

impl Server {
    fn new(doc: &Value) -> Self {
        let chain = &doc["chain"];
        let records = |key: &str| -> Vec<Vec<u8>> {
            chain[key]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| b64_decode(r.as_str().unwrap()))
                .collect()
        };
        let (m, e) = (records("membership_records"), records("epoch_records"));
        let state = verify_chain(
            &VAULT,
            &m.iter().map(Vec::as_slice).collect::<Vec<_>>(),
            &e.iter().map(Vec::as_slice).collect::<Vec<_>>(),
        )
        .expect("the file's chain verifies");
        let newest = SignedRecord::parse(e.last().unwrap(), 1 << 22)
            .unwrap()
            .id();
        let at_end = EpochAtPosition::after_epoch_record(&state, &newest).unwrap();
        let devices = chain["devices"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v["device_id"].as_str().unwrap().to_owned()))
            .collect();
        Self {
            state,
            at_end,
            devices,
            head: 0,
            log: vec![],
            index: vec![],
            voids: vec![],
            halts: vec![],
        }
    }

    fn device_id(&self, name: &str) -> String {
        self.devices
            .iter()
            .find(|(n, _)| n == name)
            .unwrap()
            .1
            .clone()
    }

    fn append_void(
        &mut self,
        credential: &str,
        record: &[u8],
        writes: bool,
        quota_exceeded: bool,
    ) -> Value {
        let _ = credential; // the signer need not be the credential's device (section 4.11 step 1)
                            // 2. parse
        let Ok(signed) = SignedRecord::parse(record, MAX_VOID_BODY) else {
            return error(400, "MalformedObject");
        };
        let Ok(body) = VoidBody::decode(&signed.body) else {
            return error(400, "MalformedObject");
        };
        if body.suite_id != SUITE_ID {
            return error(400, "UnsupportedSuite");
        }
        if body.vault_id != VAULT {
            return error(400, "MalformedObject");
        }
        // 3. exact retry
        let id = hex(&body.id());
        if let Some(v) = self.voids.iter().find(|v| v.id == id) {
            return json!({"status": 200, "void_id": id, "position": num(v.position), "head_seq": num(self.head)});
        }
        // 4. writes
        if !writes {
            return error(403, "WritesSuspended");
        }
        // 5. signer
        match verify_void(&self.state, &self.at_end, self.head + 1, record) {
            Ok(_) => {}
            Err(VoidError::SignerNotAuthorized) => return error(403, "VoidSignerNotAuthorized"),
            Err(VoidError::BadSignature) => return error(400, "VoidSignatureInvalid"),
            Err(_) => return error(400, "MalformedObject"),
        }
        // 6. target
        let target = self.index.iter().find(|(s, _, _)| *s == body.seq);
        if !target.is_some_and(|(_, c, d)| {
            *c == hex(&body.command_id) && *d == hex(&body.envelope_digest)
        }) {
            return error(409, "VoidTargetMismatch");
        }
        // 7. quota
        if quota_exceeded {
            return error(403, "QuotaExceeded");
        }
        // 8. append
        self.head += 1;
        self.log.push(Object::Void(self.head));
        self.voids.push(VoidEntry {
            id: id.clone(),
            position: self.head,
            target: body.seq,
        });
        json!({"status": 201, "void_id": id, "position": num(self.head), "head_seq": num(self.head)})
    }

    fn pull(&self, after: u64, limit: usize) -> Value {
        let seq_of = |o: &Object| match o {
            Object::Envelope(s) | Object::Void(s) | Object::Epoch(s) => *s,
        };
        let past: Vec<&Object> = self.log.iter().filter(|o| seq_of(o) > after).collect();
        let page = &past[..past.len().min(limit)];
        let mut entries = Vec::new();
        let mut voids = Vec::new();
        for o in page {
            match o {
                Object::Envelope(s) => entries.push(format!("envelope:{s}")),
                Object::Epoch(p) => entries.push(format!("epoch:{p}")),
                Object::Void(p) => {
                    let v = self.voids.iter().find(|v| v.position == *p).unwrap();
                    voids.push(json!({"position": num(*p), "void_id": v.id}));
                }
            }
        }
        let mut out = json!({
            "status": 200,
            "head_seq": num(self.head),
            "has_more": past.len() > page.len(),
            "entries": entries,
        });
        if !voids.is_empty() {
            out["voids"] = Value::Array(voids);
        }
        out
    }

    fn step(&mut self, step: &Value) -> Value {
        let cond = &step["conditions"];
        let writes = cond["writes_permitted"] != json!(false);
        let quota = cond["quota_exceeded"] == json!(true);
        let credential = step["credential"].as_str().unwrap_or("");
        match step["op"].as_str().unwrap() {
            "pushEnvelope" => {
                self.head += 1;
                self.log.push(Object::Envelope(self.head));
                self.index.push((
                    self.head,
                    step["command_id"].as_str().unwrap().to_owned(),
                    step["envelope_digest"].as_str().unwrap().to_owned(),
                ));
                json!({"status": 200, "seq": num(self.head)})
            }
            "recordEpochEntry" => {
                self.log.push(Object::Epoch(self.head));
                json!({"position": num(self.head)})
            }
            "appendVoid" => {
                let record = b64_decode(step["record"].as_str().unwrap());
                self.append_void(credential, &record, writes, quota)
            }
            "replayPush" => {
                let seq: u64 = step["seq"].as_str().unwrap().parse().unwrap();
                let mut out = json!({"status": 200, "seq": num(seq), "duplicate": true});
                let claims: Vec<Value> = self
                    .voids
                    .iter()
                    .filter(|v| v.target == seq)
                    .map(|v| Value::String(v.id.clone()))
                    .collect();
                if !claims.is_empty() {
                    out["void_claims"] = Value::Array(claims);
                }
                out
            }
            "pullEnvelopes" => {
                let after: u64 = step["after_seq"].as_str().unwrap().parse().unwrap();
                self.pull(after, step["limit"].as_u64().unwrap() as usize)
            }
            "reportHalt" => {
                let target = step["device"].as_str().unwrap();
                if target != credential {
                    return error(403, "Forbidden");
                }
                let id = self.device_id(target);
                self.halts.retain(|(d, _)| *d != id);
                let stopped = step["stopped_at"]
                    .as_str()
                    .map(|s| s.parse::<u64>().unwrap());
                if let Some(s) = stopped {
                    self.halts.push((id.clone(), s));
                }
                json!({"status": 200, "device_id": self.device_id(target), "stopped_at": stopped.map(num)})
            }
            "stageSnapshot" | "publishSnapshot" | "installEpoch" => {
                let halt = (credential != "account")
                    .then(|| self.device_id(credential))
                    .and_then(|id| self.halts.iter().find(|(d, _)| *d == id).map(|(_, s)| *s));
                match halt {
                    Some(s) => {
                        json!({"status": 409, "error": "ReplicaHalted", "details": {"stopped_at": num(s)}})
                    }
                    None => json!({"gate": "pass"}),
                }
            }
            other => panic!("unknown op {other}"),
        }
    }
}

fn run(doc: &Value) -> Vec<(String, Vec<Value>)> {
    doc["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| {
            let mut server = Server::new(doc);
            let outcomes = case["steps"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| server.step(s))
                .collect();
            (case["name"].as_str().unwrap().to_owned(), outcomes)
        })
        .collect()
}

#[test]
fn a_reference_server_agrees_with_every_expected_outcome() {
    let doc: Value = serde_json::from_str(VECTORS).unwrap();
    let cases = doc["cases"].as_array().unwrap();
    assert!(cases.len() >= 12, "the file has its cases");
    for ((name, outcomes), case) in run(&doc).into_iter().zip(cases) {
        let expected: Vec<&Value> = case["steps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| &s["expect"])
            .collect();
        for (i, (got, want)) in outcomes.iter().zip(expected).enumerate() {
            assert_eq!(got, want, "{name}: step {i}");
        }
    }
}

#[test]
fn the_vectors_reach_every_error_and_every_step_the_additions_introduced() {
    let doc: Value = serde_json::from_str(VECTORS).unwrap();
    let mut seen: Vec<String> = Vec::new();
    let mut ops: Vec<String> = Vec::new();
    for case in doc["cases"].as_array().unwrap() {
        for step in case["steps"].as_array().unwrap() {
            ops.push(step["op"].as_str().unwrap().to_owned());
            if let Some(e) = step["expect"]["error"].as_str() {
                // The status in the file is the registry's.
                let registry: Value = serde_json::from_str(REGISTRY).unwrap();
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
        "VoidSignatureInvalid",
        "VoidSignerNotAuthorized",
        "VoidTargetMismatch",
        "ReplicaHalted",
        "WritesSuspended",
        "QuotaExceeded",
        "MalformedObject",
        "UnsupportedSuite",
        "Forbidden",
    ] {
        assert!(seen.iter().any(|s| s == code), "no vector for {code}");
    }
    for op in [
        "appendVoid",
        "pullEnvelopes",
        "replayPush",
        "reportHalt",
        "stageSnapshot",
        "publishSnapshot",
        "installEpoch",
    ] {
        assert!(ops.iter().any(|o| o == op), "no vector for {op}");
    }
    // Every operation and error the additions named is in the machine-readable list.
    let registry: Value = serde_json::from_str(REGISTRY).unwrap();
    for op in ["appendVoid", "reportHalt"] {
        assert!(
            registry["operations"]
                .as_array()
                .unwrap()
                .iter()
                .any(|o| o["op"] == op),
            "{op}"
        );
    }
}

#[test]
fn the_chain_in_the_file_is_the_sample_chain() {
    // The ids of the records in the file are what the generator's chain says, so a reader who
    // rebuilds the chain from seeds and one who reads the file hold the same one.
    let doc: Value = serde_json::from_str(VECTORS).unwrap();
    let chain = common::chain();
    let file: Vec<Vec<u8>> = doc["chain"]["epoch_records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| b64_decode(r.as_str().unwrap()))
        .collect();
    assert_eq!(file, chain.epoch_records);
}
