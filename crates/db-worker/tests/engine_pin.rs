//! The engine generation pin (personal-cfo-32fmp.4, parent AC12; vault-format.md C.13).
//!
//! `ENGINE_VERSION` names the APPLY SEMANTICS: two builds with the same value write the same rows
//! and the same write-set hash from the same payload, metadata and base. This test makes that
//! checkable. It applies the deterministic sample of EVERY `WriteCommand` variant (the retired ones
//! included: they still apply) on a vault whose identity is fixed, in the dependency order the
//! write-set oracle uses, with fixed command metadata, and digests per variant:
//!
//! * the INPUT: the payload bytes and the comparable state (the write-set rows and the class-1b
//!   artifact rows) the command was applied on;
//! * the OUTPUT: the outcome, the write-set hash, and every comparable row the command changed.
//!
//! The digests are compared with `tests/engine_pins.txt`, which also pins the outcome of every
//! deterministic-rejection scenario. See `common/engine_pins.rs` for the verdicts: a variant whose
//! output changed on an unchanged input, or an added or removed variant, needs a new engine
//! generation (bump `ENGINE_VERSION`, append a section); an input-only change re-pins the current
//! section. The operation log is not compared (`operation_log.created_at`, the HLC and the node id
//! are the recording device's own: AC2). The pin runs under two time zones in child processes.

mod common;

use std::collections::BTreeMap;
use std::path::Path;

use chrono::{TimeZone, Utc};
use common::engine_pins::{generation, judge, parse, render, Section, Verdict};
use common::rejection_scenarios;
use common::replicated_state::comparable_state;
use common::samples::samples;
use common::variant_runner::{after_setup, before, command_for, Runtime, ORDER};
use common::KEY;
use db_worker::payload::encode_payload;
use db_worker::{
    with_fixed_vault_identity, ActorType, CommandMeta, DbError, DbWorker, IssuedAt, WriteCommand,
    ENGINE_VERSION,
};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const PINS_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/engine_pins.txt");
const PINS: &str = include_str!("engine_pins.txt");

const VAULT_ID: u128 = 0x0198_c2c1_5f0a_7c3d_8a4e_9b1d_f000_0001;
const CREATED_AT: &str = "2026-11-01T00:00:00+00:00";

fn u(n: u128) -> Uuid {
    Uuid::from_u128(0x0198_c2c1_5f0a_7c3d_8a4e_9b1d_f100_0000 + n)
}

/// Fixed metadata for the n-th command: no clock, no randomness.
fn pin_meta(n: u128) -> CommandMeta {
    let base = Utc
        .with_ymd_and_hms(2026, 11, 4, 12, 0, 0)
        .unwrap()
        .timestamp_micros();
    CommandMeta {
        command_id: u(n),
        correlation_id: u(10_000 + n),
        causation_id: None,
        actor_type: ActorType::User,
        actor_id: "pin".to_owned(),
        idempotency_key: format!("00000000-0000-4000-8000-{:012x}", 50_000 + n),
        issued_at: IssuedAt::from_unix_micros(base + i64::try_from(n).unwrap() * 1_000_000),
    }
}

/// A vault with a fixed identity, created in `dir`.
fn fixed_vault(dir: &Path, name: &str) -> DbWorker {
    with_fixed_vault_identity(Uuid::from_u128(VAULT_ID), CREATED_AT, || {
        DbWorker::open(dir.join(name), KEY).unwrap()
    })
}

fn sha(parts: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(u64::try_from(part.len()).unwrap().to_be_bytes());
        hasher.update(part);
    }
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

type State = BTreeMap<(String, Vec<u8>), [u8; 32]>;

fn state_bytes(state: &State) -> Vec<u8> {
    let mut out = Vec::new();
    for ((table, key), digest) in state {
        for part in [table.as_bytes(), key.as_slice(), digest.as_slice()] {
            out.extend_from_slice(&u32::try_from(part.len()).unwrap().to_be_bytes());
            out.extend_from_slice(part);
        }
    }
    out
}

/// The rows `after` has that `before` lacks or holds differently, and the rows it lost (as an
/// all-zero digest).
fn changed_rows(before: &State, after: &State) -> State {
    let mut changed = State::new();
    for (key, digest) in after {
        if before.get(key) != Some(digest) {
            changed.insert(key.clone(), *digest);
        }
    }
    for key in before.keys().filter(|key| !after.contains_key(*key)) {
        changed.insert(key.clone(), [0; 32]);
    }
    changed
}

/// Every variant's (input, output) digests, applied in order on one fixed vault.
fn variant_pins() -> BTreeMap<String, (String, String)> {
    let dir = tempfile::tempdir().unwrap();
    let worker = fixed_vault(dir.path(), "pin.vault");
    let mut runtime = Runtime {
        primary: Some(db_worker::households::primary_member_id(&Uuid::from_u128(
            VAULT_ID,
        ))),
        ..Runtime::default()
    };
    let mut by_name: BTreeMap<&str, WriteCommand> = samples().into_iter().collect();
    let mut pins = BTreeMap::new();
    let mut n = 0_u128;
    let state = |worker: &DbWorker| comparable_state(&worker.read_connection().unwrap());
    for name in ORDER {
        let sample = by_name.remove(name).expect("a sample per variant");
        for (label, setup) in before(name, &worker, &mut runtime) {
            n += 1;
            worker
                .dispatch(pin_meta(n), setup)
                .unwrap_or_else(|e| panic!("{name}: {label}: {e}"));
        }
        after_setup(&worker, &mut runtime);
        let command = command_for(name, sample, &runtime);
        let payload = encode_payload(&command).expect("a sample encodes");
        let base = state(&worker);
        n += 1;
        let outcome = worker.dispatch(pin_meta(n), command);
        let after = state(&worker);
        let (code, hash): (String, Vec<u8>) = match &outcome {
            Ok(_) => (
                "applied".to_owned(),
                worker.last_write_set().expect("a write set").hash.to_vec(),
            ),
            Err(DbError::Rejected { code, .. }) => (format!("rejected {}", code.as_u16()), vec![]),
            Err(other) => panic!("{name}: the sample did not apply: {other}"),
        };
        let input = sha(&[&payload, &state_bytes(&base)]);
        let output = sha(&[
            code.as_bytes(),
            &hash,
            &state_bytes(&changed_rows(&base, &after)),
        ]);
        pins.insert((*name).to_owned(), (input, output));
    }
    assert!(by_name.is_empty(), "ORDER covers every sample");
    pins
}

/// The outcome of every deterministic-rejection scenario, on a fixed vault.
fn rejection_pins() -> BTreeMap<String, String> {
    let dir = tempfile::tempdir().unwrap();
    let worker = fixed_vault(dir.path(), "rejections.vault");
    rejection_scenarios::run(&worker)
        .into_iter()
        .map(|line| {
            let (label, outcome) = line.split_once(": ").expect("label: outcome");
            (label.to_owned(), outcome.to_owned())
        })
        .collect()
}

fn current_section() -> Section {
    Section {
        engine: ENGINE_VERSION.to_owned(),
        variants: variant_pins(),
        rejections: rejection_pins(),
    }
}

fn explain(verdict: &Verdict) -> String {
    match verdict {
        Verdict::Same => "unchanged".to_owned(),
        Verdict::Repin(why) => format!(
            "INPUT-ONLY change: re-pin the current section at the same generation with \
             ENGINE_PIN_UPDATE=1 (no bump):\n  {}",
            why.join("\n  ")
        ),
        Verdict::Bump(why) => format!(
            "APPLY OUTPUT changed: bump ENGINE_VERSION to dohflow-engine-{} in crates/db-worker/src/lib.rs \
             and append a section with ENGINE_PIN_UPDATE=1 (C.13):\n  {}",
            generation(ENGINE_VERSION) + 1,
            why.join("\n  ")
        ),
    }
}

#[test]
fn the_pin_history_is_consecutive_and_ends_at_the_engine_version() {
    let sections = parse(PINS);
    assert!(
        !sections.is_empty(),
        "engine_pins.txt holds at least one section"
    );
    for (index, section) in sections.iter().enumerate() {
        assert_eq!(
            generation(&section.engine),
            u32::try_from(index).unwrap() + 1,
            "the sections are dohflow-engine-1, -2, ... with none missing or reordered"
        );
    }
    assert_eq!(
        sections.last().unwrap().engine,
        ENGINE_VERSION,
        "ENGINE_VERSION is the last pinned generation; a bump appends its section"
    );
}

#[test]
fn apply_writes_what_the_pin_says_for_this_engine_version() {
    let now = current_section();
    let mut sections = parse(PINS);
    let verdict = sections.iter().find(|s| s.engine == ENGINE_VERSION).map_or(
        Verdict::Bump(vec!["no section for this engine".to_owned()]),
        |pinned| judge(pinned, &now),
    );

    if std::env::var_os("ENGINE_PIN_UPDATE").is_some() {
        match (
            &verdict,
            sections.iter().position(|s| s.engine == ENGINE_VERSION),
        ) {
            (Verdict::Same, _) => return,
            (Verdict::Repin(_), Some(at)) => sections[at] = now,
            (_, None) => {
                // A new generation: ENGINE_VERSION was bumped, so append its section.
                assert_eq!(
                    generation(ENGINE_VERSION),
                    u32::try_from(sections.len()).unwrap() + 1,
                    "the new generation follows the last pinned one"
                );
                sections.push(now);
            }
            (Verdict::Bump(why), Some(_)) => panic!(
                "refusing to rewrite the pin for {ENGINE_VERSION}: apply's output changed, so \
                 bump ENGINE_VERSION first:\n  {}",
                why.join("\n  ")
            ),
        }
        std::fs::write(PINS_PATH, render(&sections)).unwrap();
        return;
    }
    assert_eq!(verdict, Verdict::Same, "{}", explain(&verdict));
}

#[test]
fn every_variant_and_every_rejection_scenario_is_pinned() {
    let now = current_section();
    assert_eq!(now.variants.len(), samples().len());
    assert!(
        now.rejections.len() > 50,
        "the rejection scenarios are pinned ({})",
        now.rejections.len()
    );
    // Pinning does not make the engine accept what the sample set leaves out: every variant of the
    // enum has a sample (payload_v1 guards that), so every variant has a pin entry.
    assert!(now
        .variants
        .keys()
        .all(|name| samples().iter().any(|(n, _)| n == name)));
}

#[test]
fn the_pin_is_a_function_of_the_inputs_alone_not_of_the_run() {
    assert_eq!(
        render(&[current_section()]),
        render(&[current_section()]),
        "two runs in this process agree"
    );
}

#[test]
#[ignore = "run by the pin is the same under other time zones, in a child process"]
fn child_print_the_current_section_under_the_inherited_time_zone() {
    for line in render(&[current_section()]).lines() {
        println!("PIN {line}");
    }
}

fn section_text_in_child(zone: &str) -> String {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "child_print_the_current_section_under_the_inherited_time_zone",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("TZ", zone)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "child under {zone} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.find("PIN ").map(|at| line[at + 4..].to_owned()))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_pin_is_the_same_under_other_time_zones() {
    let here = render(&[current_section()]);
    let here = here.trim_end().to_owned();
    for zone in ["America/Chicago", "Pacific/Auckland"] {
        assert_eq!(
            section_text_in_child(zone),
            here,
            "the device time zone ({zone}) changed what apply writes"
        );
    }
}

fn section(variants: &[(&str, &str, &str)], rejections: &[(&str, &str)]) -> Section {
    Section {
        engine: "dohflow-engine-1".to_owned(),
        variants: variants
            .iter()
            .map(|(n, i, o)| ((*n).to_owned(), ((*i).to_owned(), (*o).to_owned())))
            .collect(),
        rejections: rejections
            .iter()
            .map(|(l, o)| ((*l).to_owned(), (*o).to_owned()))
            .collect(),
    }
}

#[test]
fn the_verdicts_tell_a_bump_from_a_re_pin() {
    let pinned = section(
        &[("a", "in1", "out1"), ("b", "in2", "out2")],
        &[("r", "rejected 1")],
    );
    assert_eq!(judge(&pinned, &pinned), Verdict::Same);
    // An output that moved on an unchanged input is an engine change.
    let moved = section(
        &[("a", "in1", "OUT"), ("b", "in2", "out2")],
        &[("r", "rejected 1")],
    );
    assert!(matches!(judge(&pinned, &moved), Verdict::Bump(_)));
    // An input that moved explains an output that moved with it: re-pin, no bump.
    let rebased = section(
        &[("a", "IN", "OUT"), ("b", "in2", "out2")],
        &[("r", "rejected 1")],
    );
    assert!(matches!(judge(&pinned, &rebased), Verdict::Repin(_)));
    // A variant added or removed is a bump (what a receiver accepts is part of the engine).
    let added = section(
        &[("a", "in1", "out1"), ("b", "in2", "out2"), ("c", "i", "o")],
        &[("r", "rejected 1")],
    );
    assert!(matches!(judge(&pinned, &added), Verdict::Bump(_)));
    let removed = section(&[("a", "in1", "out1")], &[("r", "rejected 1")]);
    assert!(matches!(judge(&pinned, &removed), Verdict::Bump(_)));
    // A rejection that changes code is an engine change; a scenario list that changes re-pins.
    let recoded = section(
        &[("a", "in1", "out1"), ("b", "in2", "out2")],
        &[("r", "rejected 2")],
    );
    assert!(matches!(judge(&pinned, &recoded), Verdict::Bump(_)));
    let relisted = section(
        &[("a", "in1", "out1"), ("b", "in2", "out2")],
        &[("r", "rejected 1"), ("s", "rejected 1")],
    );
    assert!(matches!(judge(&pinned, &relisted), Verdict::Repin(_)));
}

#[test]
fn the_file_format_round_trips() {
    let sections = vec![section(
        &[("a", "in1", "out1")],
        &[("a label: with colon", "ok rows=1 hash=ff")],
    )];
    assert_eq!(parse(&render(&sections)), sections);
}
