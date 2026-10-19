//! A ratchet for "`apply` never reads the wall clock" (personal-cfo-t2s4b; ADR 0013
//! Addendum A, section 2).
//!
//! A command carries `issued_at` in its metadata, set once at the command boundary, and
//! `apply` must read that instead of the clock, so replaying the command on another
//! device reads the same instant. Today 21 `Utc::now()` calls under
//! `crates/db-worker/src/apply/` still read the clock; replacing them is SYNC-1a
//! (`personal-cfo-32fmp`). Until then this test pins the inventory exactly:
//!
//! * a **new** clock read in `apply/` fails the test, so the number can only go down;
//! * a **removed** read fails it too, until the table below is lowered, so a stale
//!   allowance cannot linger;
//! * SYNC-1a ends with an empty table, and the test then asserts zero.
//!
//! Besides direct reads (`Utc::now()`, `SystemTime::now()`, `Instant::now()`,
//! `Local::now()`), a call to `household_today(`, which reads the clock itself, counts
//! as an indirect read.

use std::fs;
use std::path::{Path, PathBuf};

/// `(file under crates/db-worker/src/apply/, direct clock reads, indirect reads)`.
/// SYNC-1a empties this table.
const KNOWN_CLOCK_READS: &[(&str, usize, usize)] = &[
    ("accounts.rs", 1, 0),
    ("categorization_cmds.rs", 6, 0),
    ("ingestion_cmds.rs", 1, 0),
    ("recurring.rs", 9, 0),
    ("transactions.rs", 4, 1),
];

const DIRECT: &[&str] = &[
    "Utc::now(",
    "SystemTime::now(",
    "Instant::now(",
    "Local::now(",
];
const INDIRECT: &[&str] = &["household_today("];

fn apply_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/apply")
}

/// Count `needles` on code lines of `source`: comment-only lines (`//`, `///`, `//!`) are
/// ignored, so prose that mentions a call is not a call.
fn count(source: &str, needles: &[&str]) -> usize {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .map(|line| {
            needles
                .iter()
                .map(|n| line.matches(n).count())
                .sum::<usize>()
        })
        .sum()
}

fn observed() -> Vec<(String, usize, usize)> {
    let mut rows = Vec::new();
    for entry in fs::read_dir(apply_dir()).expect("src/apply") {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let source = fs::read_to_string(&path).unwrap();
        rows.push((name, count(&source, DIRECT), count(&source, INDIRECT)));
    }
    rows.sort();
    rows
}

#[test]
fn apply_clock_reads_match_the_pinned_inventory() {
    let mut expected: Vec<(String, usize, usize)> = KNOWN_CLOCK_READS
        .iter()
        .map(|(f, d, i)| ((*f).to_owned(), *d, *i))
        .collect();
    expected.sort();

    let actual: Vec<(String, usize, usize)> = observed()
        .into_iter()
        .filter(|(_, direct, indirect)| *direct + *indirect > 0)
        .collect();

    assert_eq!(
        actual, expected,
        "`apply` must not read the wall clock: take the instant from `meta.issued_at`. \
         This table may only shrink; when SYNC-1a removes the last read, delete the table \
         and assert zero."
    );
}

#[test]
fn the_pinned_total_is_the_adr_inventory() {
    // ADR 0013 Addendum A counts 21 `Utc::now()` sites; the table must not drift from it
    // without the ADR's number being revisited.
    let direct: usize = KNOWN_CLOCK_READS.iter().map(|(_, d, _)| d).sum();
    assert_eq!(direct, 21);
}

#[test]
fn the_household_timezone_apply_function_uses_issued_at() {
    // The one apply function written after the rule, which must already comply.
    let source = fs::read_to_string(apply_dir().join("household.rs")).unwrap();
    assert_eq!(count(&source, DIRECT), 0);
    assert!(source.contains("meta.issued_at"));
}

#[test]
fn the_scanner_recognizes_what_it_exists_to_catch() {
    assert_eq!(count("let t = Utc::now();", DIRECT), 1);
    assert_eq!(count("chrono::Utc::now().to_rfc3339()", DIRECT), 1);
    assert_eq!(count("let a = Utc::now(); let b = Utc::now();", DIRECT), 2);
    assert_eq!(count("std::time::SystemTime::now()", DIRECT), 1);
    assert_eq!(count("Instant::now()", DIRECT), 1);
    assert_eq!(count("chrono::Local::now()", DIRECT), 1);
    assert_eq!(count("crate::forecast::household_today(tx)?", INDIRECT), 1);
    // Prose that merely mentions a call is not a call.
    assert_eq!(
        count("// replaces Utc::now() with meta.issued_at", DIRECT),
        0
    );
    assert_eq!(count("/// never calls `Utc::now()`", DIRECT), 0);
    assert_eq!(count("//! SystemTime::now() is forbidden", DIRECT), 0);
    // And a field or a type named like the call is not a call.
    assert_eq!(count("let issued = meta.issued_at;", DIRECT), 0);
    assert_eq!(count("use chrono::Utc;", DIRECT), 0);
}
