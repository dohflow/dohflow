//! Repository guards for the two rules that code review alone is a weak defense for
//! (ADR 0002 addendum 2026-10-24, decisions 4 and 7).
//!
//! 1. The platform crates never log: no `tracing`, `log`, `println!`, `eprintln!` or `dbg!`, so no
//!    secret, key name or platform message can reach a span, a log line or a terminal from them.
//! 2. No source file calls both `preflight_prompt(` and `load_secret(`: a biometric pre-flight match
//!    must never be what lets a secret be loaded. (A heuristic by design: a legitimate caller shows
//!    the prompt in one place and loads in another, and must not branch on the first for the
//!    second; if this ever needs an exception it must be argued in review, not slipped in.)

use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if name == "target" || name == "node_modules" || name == ".git" {
            continue;
        }
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn platform_crate_sources() -> Vec<PathBuf> {
    let mut files = Vec::new();
    let crates = root().join("crates");
    for entry in fs::read_dir(&crates).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("platform-") {
            rust_files(&entry.path().join("src"), &mut files);
        }
    }
    assert!(
        !files.is_empty(),
        "found no platform crate sources: the scan would pass vacuously"
    );
    files
}

#[test]
fn the_platform_crates_never_log_or_print() {
    let forbidden = [
        "tracing::",
        "log::",
        "println!",
        "eprintln!",
        "dbg!",
        "print!(",
        "eprint!(",
    ];
    let mut offenders = Vec::new();
    for file in platform_crate_sources() {
        let text = fs::read_to_string(&file).unwrap();
        for (number, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            if forbidden.iter().any(|f| code.contains(f)) {
                offenders.push(format!(
                    "{}:{}: {}",
                    file.display(),
                    number + 1,
                    line.trim()
                ));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "platform crates must not log or print:\n{}",
        offenders.join("\n")
    );
}

/// Paths relative to the repository root with forward slashes, so the exclusion below cannot match
/// the checkout's own directory names.
fn relative(file: &Path) -> String {
    let root = root().canonicalize().unwrap();
    let file = file.canonicalize().unwrap();
    file.strip_prefix(&root)
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/")
}

/// Source files that are consumers (everything except the platform crates themselves) and call
/// both names. Takes `(path relative to the repository root, text)` pairs, so the self-test below can
/// feed synthetic ones: nothing is ever written into the repository, which would race the other scan
/// that reads the tree in parallel.
fn offenders_in(files: &[(String, String)]) -> (usize, Vec<String>) {
    let mut consumers = 0;
    let mut offenders = Vec::new();
    for (path, text) in files {
        // The trait definitions and their implementations/tests name both; they are the definition,
        // not a caller. Everything else is a consumer.
        if path.starts_with("crates/platform-") {
            continue;
        }
        consumers += 1;
        let calls = |name: &str| {
            text.lines()
                .any(|l| l.split("//").next().unwrap_or("").contains(name))
        };
        if calls("preflight_prompt(") && calls("load_secret(") {
            offenders.push(path.clone());
        }
    }
    (consumers, offenders)
}

#[test]
fn no_source_file_calls_both_preflight_and_load_secret() {
    let mut files = Vec::new();
    for dir in ["crates", "apps/desktop/src-tauri/src"] {
        rust_files(&root().join(dir), &mut files);
    }
    let pairs: Vec<(String, String)> = files
        .iter()
        .map(|file| (relative(file), fs::read_to_string(file).unwrap()))
        .collect();
    let (consumers, offenders) = offenders_in(&pairs);
    // Not vacuous: the exclusion must leave the app and kernel sources in the scan.
    assert!(
        consumers > 50,
        "the scan covered only {consumers} consumer files"
    );
    assert!(
        offenders.is_empty(),
        "a biometric pre-flight must not gate a secret load; offending files:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn the_scan_catches_an_offender_in_a_consumer_and_ignores_a_platform_crate() {
    // Self-check on synthetic files (no writes into the tree): the same text is flagged under a
    // consumer path and ignored under a platform crate, so a refactor of the scan cannot make it
    // vacuous. Comments are not calls, and a file that calls only one of the two names is fine.
    let both = "fn unlock() { auth.preflight_prompt(&p); store.load_secret(&k, &p); }";
    let pairs = vec![
        (
            "apps/desktop/src-tauri/src/zz_scan_probe.rs".to_owned(),
            both.to_owned(),
        ),
        (
            "crates/platform-traits/src/zz_scan_probe.rs".to_owned(),
            both.to_owned(),
        ),
        (
            "crates/finance-kernel/src/only_loads.rs".to_owned(),
            "fn a() { store.load_secret(&k, &p); }".to_owned(),
        ),
        (
            "crates/finance-kernel/src/only_comment.rs".to_owned(),
            "// preflight_prompt( is mentioned\nfn a() { store.load_secret(&k, &p); }".to_owned(),
        ),
    ];
    let (consumers, flagged) = offenders_in(&pairs);
    assert_eq!(consumers, 3, "the platform-crate copy is not a consumer");
    assert_eq!(
        flagged,
        vec!["apps/desktop/src-tauri/src/zz_scan_probe.rs".to_owned()]
    );
}
