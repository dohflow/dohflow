//! A ratchet for "`apply` never reads the wall clock" (personal-cfo-t2s4b; ADR 0013
//! Addendum A, section 2).
//!
//! A command carries `issued_at` in its metadata, set once at the command boundary, and
//! `apply` must read that instead of the clock, so replaying the command on another
//! device reads the same instant. Replacing today's reads is SYNC-1a
//! (`personal-cfo-32fmp`). Until then this test pins the inventory exactly, in two parts:
//!
//! * **Direct reads** under `crates/db-worker/src/apply/`: 21 `Utc::now()` calls (the ADR's
//!   number), plus any `SystemTime::now()`, `Instant::now()` or `Local::now()`.
//! * **Indirect reads**: calls from `apply/` to helpers defined elsewhere in the crate
//!   that read the clock themselves, directly or through another helper (for example
//!   `household_today`, the `ingestion` provenance and dedupe helpers, and the class-2
//!   `rebuild_in` projections). The set of such helpers is **derived here** from the
//!   crate's non-apply sources, not hand-listed, so a new clock-reading helper that
//!   `apply/` starts calling fails the test instead of slipping past it.
//!
//! The tables below may only shrink. A new read fails the test; a removed read fails it
//! too, until the table is lowered, so a stale allowance cannot linger. SYNC-1a ends with
//! empty tables and then asserts zero. Id minting (`now_v7()`, `Id::new()`) is a separate
//! determinism rule, tracked by the derived-id helper (part 2) and SYNC-1a; this test is
//! about the clock only.
//!
//! The derivation is name-based and conservative: a helper is a reader if its body calls
//! `Utc::now()` and friends, or calls another reader by name. It can over-report (two
//! functions sharing a name), never under-report a call written in the usual form; an
//! over-report is resolved by a human pinning or renaming, which is the point.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// `(file under src/apply/, direct clock reads)`. SYNC-1a empties this table.
const KNOWN_DIRECT: &[(&str, usize)] = &[
    ("accounts.rs", 1),
    ("categorization_cmds.rs", 6),
    ("ingestion_cmds.rs", 1),
    ("recurring.rs", 9),
    ("transactions.rs", 4),
];

/// `(file under src/apply/, clock-reading helper, calls)`. SYNC-1a empties this table.
/// `rebuild_in` is a class-2 projection rebuild (`money_inbox`, `commitments`); SYNC-1a
/// decides, helper by helper, whether to pass `meta.issued_at` or to keep the time a
/// local-only, non-replicated column.
const KNOWN_INDIRECT: &[(&str, &str, usize)] = &[
    ("inbox.rs", "rebuild_in", 2),
    ("inbox.rs", "record_dismiss", 1),
    ("inbox.rs", "record_snooze", 1),
    ("ingestion_cmds.rs", "create_source_batch", 1),
    ("ingestion_cmds.rs", "link_provenance", 1),
    ("ingestion_cmds.rs", "rebuild_in", 3),
    ("ingestion_cmds.rs", "record_dedupe_decision", 5),
    ("ingestion_cmds.rs", "record_transaction_detail", 1),
    ("ingestion_cmds.rs", "update_batch_status", 1),
    ("recurring.rs", "rebuild_in", 6),
    ("transactions.rs", "household_today", 1),
    ("transactions.rs", "reverse_and_hide_transaction", 2),
];

const DIRECT: &[&str] = &[
    "Utc::now(",
    "SystemTime::now(",
    "Instant::now(",
    "Local::now(",
];

fn src_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read_dir").flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

/// Replace comments and the contents of string and character literals with spaces, keeping
/// every other byte (and every newline), so prose and text that mention a call are not
/// mistaken for one and braces inside strings do not confuse function-body matching.
fn blank_noise(source: &str) -> String {
    let b = source.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    let blank = |c: u8| if c == b'\n' { b'\n' } else { b' ' };
    while i < b.len() {
        let c = b[i];
        if c == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                out.push(b' ');
                i += 1;
            }
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            let mut depth = 0;
            while i < b.len() {
                if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
                    depth += 1;
                    out.extend_from_slice(b"  ");
                    i += 2;
                } else if b[i] == b'*' && b.get(i + 1) == Some(&b'/') {
                    depth -= 1;
                    out.extend_from_slice(b"  ");
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    out.push(blank(b[i]));
                    i += 1;
                }
            }
        } else if c == b'r' && (i == 0 || !is_ident(b[i - 1])) && raw_string_len(&b[i..]).is_some()
        {
            let len = raw_string_len(&b[i..]).unwrap();
            out.push(b'r');
            out.extend(b[i + 1..i + len].iter().map(|&x| blank(x)));
            i += len;
        } else if c == b'"' {
            out.push(b'"');
            i += 1;
            while i < b.len() && b[i] != b'"' {
                if b[i] == b'\\' {
                    out.push(b' ');
                    i += 1;
                }
                if i < b.len() {
                    out.push(blank(b[i]));
                    i += 1;
                }
            }
            if i < b.len() {
                out.push(b'"');
                i += 1;
            }
        } else if c == b'\'' && is_char_literal(&b[i..]) {
            let len = char_literal_len(&b[i..]);
            out.extend(std::iter::repeat_n(b' ', len));
            i += len;
        } else {
            out.push(c);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The length of a raw string literal starting at `b[0] == b'r'`, if it is one.
fn raw_string_len(b: &[u8]) -> Option<usize> {
    let mut hashes = 0;
    let mut i = 1;
    while b.get(i) == Some(&b'#') {
        hashes += 1;
        i += 1;
    }
    if b.get(i) != Some(&b'"') {
        return None;
    }
    i += 1;
    while i < b.len() {
        if b[i] == b'"'
            && b.len() >= i + 1 + hashes
            && b[i + 1..i + 1 + hashes].iter().all(|&x| x == b'#')
        {
            return Some(i + 1 + hashes);
        }
        i += 1;
    }
    None
}

/// `'x'`, `'\n'`, `'\u{..}'` (a character literal) as opposed to a lifetime like `'a`.
fn is_char_literal(b: &[u8]) -> bool {
    match (b.get(1), b.get(2)) {
        (Some(b'\\'), _) => b[2..].iter().take(12).any(|&x| x == b'\''),
        (Some(_), Some(b'\'')) => true,
        _ => false,
    }
}

fn char_literal_len(b: &[u8]) -> usize {
    if b[1] == b'\\' {
        2 + b[2..].iter().position(|&x| x == b'\'').unwrap_or(0) + 1
    } else {
        3
    }
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// `(name, body)` of every function with a body in `source` (already noise-blanked).
fn functions(source: &str) -> Vec<(String, String)> {
    let b = source.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 3 < b.len() {
        let is_fn = b[i] == b'f'
            && b[i + 1] == b'n'
            && b[i + 2].is_ascii_whitespace()
            && (i == 0 || !is_ident(b[i - 1]));
        if !is_fn {
            i += 1;
            continue;
        }
        let mut j = i + 3;
        while b.get(j).is_some_and(u8::is_ascii_whitespace) {
            j += 1;
        }
        let start = j;
        while b.get(j).is_some_and(|&c| is_ident(c)) {
            j += 1;
        }
        if j == start {
            i += 3;
            continue;
        }
        let name = source[start..j].to_owned();
        // The body starts at the first `{` outside parentheses; a `;` first means no body.
        let mut paren = 0_i32;
        let mut body_start = None;
        while j < b.len() {
            match b[j] {
                b'(' => paren += 1,
                b')' => paren -= 1,
                b';' if paren == 0 => break,
                b'{' if paren == 0 => {
                    body_start = Some(j);
                    break;
                }
                _ => {}
            }
            j += 1;
        }
        let Some(open) = body_start else {
            i = j.max(i + 3);
            continue;
        };
        let mut depth = 0_i32;
        let mut k = open;
        while k < b.len() {
            match b[k] {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            k += 1;
        }
        out.push((name, source[open..=k.min(b.len() - 1)].to_owned()));
        i = open + 1;
    }
    out
}

fn count_calls(source: &str, name: &str) -> usize {
    let b = source.as_bytes();
    let n = name.as_bytes();
    let mut count = 0;
    let mut i = 0;
    while let Some(at) = source[i..].find(name) {
        let start = i + at;
        let end = start + n.len();
        let before_ok = start == 0 || !is_ident(b[start - 1]);
        let mut after = end;
        while b.get(after).is_some_and(u8::is_ascii_whitespace) {
            after += 1;
        }
        let is_call =
            before_ok && b.get(end).is_none_or(|&c| !is_ident(c)) && b.get(after) == Some(&b'(');
        // `fn name(` is a definition, not a call.
        let is_definition = source[..start].trim_end().ends_with("fn");
        if is_call && !is_definition {
            count += 1;
        }
        i = end;
    }
    count
}

fn count_direct(source: &str) -> usize {
    DIRECT.iter().map(|d| source.matches(d).count()).sum()
}

/// Names of functions that read the clock directly or by calling another such function:
/// a fixed point over function names.
fn reader_closure(bodies: &[(String, String)]) -> BTreeSet<String> {
    let mut readers: BTreeSet<String> = bodies
        .iter()
        .filter(|(_, body)| count_direct(body) > 0)
        .map(|(name, _)| name.clone())
        .collect();
    loop {
        let before = readers.len();
        for (name, body) in bodies {
            if !readers.contains(name) && readers.iter().any(|r| count_calls(body, r) > 0) {
                readers.insert(name.clone());
            }
        }
        if readers.len() == before {
            return readers;
        }
    }
}

/// Readers among the functions outside `apply/` and outside the unit-test tree.
fn clock_readers() -> BTreeSet<String> {
    let mut files = Vec::new();
    rust_files(&src_dir(), &mut files);
    let bodies: Vec<(String, String)> = files
        .iter()
        .filter(|p| {
            let rel = p
                .strip_prefix(src_dir())
                .unwrap()
                .to_string_lossy()
                .into_owned();
            !rel.starts_with("apply/") && !rel.starts_with("tests/")
        })
        .flat_map(|p| functions(&blank_noise(&fs::read_to_string(p).unwrap())))
        .collect();
    reader_closure(&bodies)
}

fn apply_sources() -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for entry in fs::read_dir(src_dir().join("apply")).unwrap().flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            out.insert(name, blank_noise(&fs::read_to_string(&path).unwrap()));
        }
    }
    out
}

#[test]
fn apply_direct_clock_reads_match_the_pinned_inventory() {
    let actual: BTreeMap<String, usize> = apply_sources()
        .into_iter()
        .map(|(file, source)| (file, count_direct(&source)))
        .filter(|(_, n)| *n > 0)
        .collect();
    let expected: BTreeMap<String, usize> = KNOWN_DIRECT
        .iter()
        .map(|(f, n)| ((*f).to_owned(), *n))
        .collect();
    assert_eq!(
        actual, expected,
        "`apply` must not read the wall clock: take the instant from `meta.issued_at`. This \
         table may only shrink; when SYNC-1a removes the last read, delete it and assert zero."
    );
}

#[test]
fn apply_indirect_clock_reads_match_the_pinned_inventory() {
    let readers = clock_readers();
    let mut actual: BTreeMap<(String, String), usize> = BTreeMap::new();
    for (file, source) in apply_sources() {
        for reader in &readers {
            let n = count_calls(&source, reader);
            if n > 0 {
                actual.insert((file.clone(), reader.clone()), n);
            }
        }
    }
    let expected: BTreeMap<(String, String), usize> = KNOWN_INDIRECT
        .iter()
        .map(|(f, r, n)| (((*f).to_owned(), (*r).to_owned()), *n))
        .collect();
    assert_eq!(
        actual, expected,
        "`apply` must not call a helper that reads the wall clock: pass `meta.issued_at` \
         down instead. This table may only shrink; SYNC-1a empties it."
    );
}

#[test]
fn the_pinned_direct_total_is_the_adr_inventory() {
    // ADR 0013 Addendum A counts 21 `Utc::now()` sites; the table must not drift from it
    // without the ADR's number being revisited.
    let direct: usize = KNOWN_DIRECT.iter().map(|(_, n)| n).sum();
    assert_eq!(direct, 21);
}

#[test]
fn every_pinned_helper_still_reads_the_clock() {
    // If a refactor renamed or cleaned a helper, the table would silently pin nothing for it.
    let readers = clock_readers();
    for (_, helper, _) in KNOWN_INDIRECT {
        assert!(
            readers.contains(*helper),
            "{helper} no longer reads the clock"
        );
    }
}

#[test]
fn the_household_timezone_apply_function_uses_issued_at() {
    // The one apply function written after the rule, which must already comply.
    let source = fs::read_to_string(src_dir().join("apply/household.rs")).unwrap();
    assert_eq!(count_direct(&blank_noise(&source)), 0);
    assert!(source.contains("meta.issued_at"));
}

// ---------------------------------------------------------------------------
// The scanner itself
// ---------------------------------------------------------------------------

#[test]
fn the_scanner_recognizes_what_it_exists_to_catch() {
    let direct = |s: &str| count_direct(&blank_noise(s));
    assert_eq!(direct("let t = Utc::now();"), 1);
    assert_eq!(direct("chrono::Utc::now().to_rfc3339()"), 1);
    assert_eq!(direct("let a = Utc::now(); let b = Utc::now();"), 2);
    assert_eq!(direct("std::time::SystemTime::now()"), 1);
    assert_eq!(direct("Instant::now()"), 1);
    assert_eq!(direct("chrono::Local::now()"), 1);
    // Prose and string text that merely mention a call are not a call.
    assert_eq!(direct("// replaces Utc::now() with meta.issued_at"), 0);
    assert_eq!(direct("/// never calls `Utc::now()`"), 0);
    assert_eq!(direct("/* Utc::now() */ let x = 1;"), 0);
    assert_eq!(direct(r#"let s = "Utc::now()";"#), 0);
    assert_eq!(direct(r##"let s = r#"Utc::now()"#;"##), 0);
    // And a field or a type named like the call is not a call.
    assert_eq!(direct("let issued = meta.issued_at;"), 0);
    assert_eq!(direct("use chrono::Utc;"), 0);
}

#[test]
fn functions_are_extracted_with_their_bodies() {
    let source = blank_noise(
        r#"
        pub(crate) fn a(x: i32) -> i32 { if x > 0 { 1 } else { 2 } }
        fn b() { let s = "}{"; let c = '}'; call_a(); }
        trait T { fn decl(&self); }
        fn generic<T: Fn(i32) -> i32>(f: T) where T: Copy { f(1); }
        "#,
    );
    let found = functions(&source);
    let names: Vec<&str> = found.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["a", "b", "generic"]);
    assert!(found[1].1.contains("call_a"));
    assert!(!found[1].1.contains("generic"));
}

#[test]
fn calls_are_recognized_but_definitions_and_lookalikes_are_not() {
    assert_eq!(count_calls("household_today(tx)", "household_today"), 1);
    assert_eq!(
        count_calls("crate::forecast::household_today (tx)?", "household_today"),
        1
    );
    assert_eq!(
        count_calls("fn household_today(conn: &C) {}", "household_today"),
        0
    );
    assert_eq!(count_calls("my_household_today(tx)", "household_today"), 0);
    assert_eq!(count_calls("household_today_at(tx)", "household_today"), 0);
    assert_eq!(
        count_calls("let f = household_today;", "household_today"),
        0
    );
}

#[test]
fn a_helper_that_reaches_the_clock_through_another_helper_is_a_reader() {
    // The closure is by name: `outer` calls `inner`, which reads the clock; `pure` does not.
    let bodies = vec![
        ("inner".to_owned(), "{ Utc::now() }".to_owned()),
        ("outer".to_owned(), "{ inner() }".to_owned()),
        ("pure".to_owned(), "{ 1 + 1 }".to_owned()),
    ];
    let readers: Vec<String> = reader_closure(&bodies).into_iter().collect();
    assert_eq!(readers, ["inner", "outer"]);
}
