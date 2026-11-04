//! `WriteCommand::CommitStaged` and `WriteCommand::SkipStaged` are retired (personal-cfo-0g528.1):
//! they stay in the enum so an operation log written before the change decodes and replays as it
//! did, and nothing produces them any more. The commit is `CommitImportedTransaction`, decided by a
//! read-only planner on the issuing device, and the skip is a plain worker method.

use std::path::{Path, PathBuf};

fn sources_under(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources_under(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Production sources: the crates' `src` trees, without the unit tests of a crate (which exercise the
/// retired commands on purpose, since they still replay).
fn production_sources() -> Vec<(PathBuf, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for dir in [
        "crates/db-worker/src",
        "crates/finance-kernel/src",
        "apps/desktop/src-tauri/src",
    ] {
        sources_under(&root.join(dir), &mut files);
    }
    assert!(files.len() > 10, "the scan found the sources");
    files
        .into_iter()
        .filter(|file| !file.components().any(|part| part.as_os_str() == "tests"))
        .map(|file| {
            let text = std::fs::read_to_string(&file).unwrap();
            (file, text)
        })
        .collect()
}

/// Where `needle` appears followed by a field value (a producer builds the variant) rather than by
/// `..` or a pattern (the enum's own arms match it).
fn builders(needle: &str, first_field: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (file, text) in production_sources() {
        let mut rest = text.as_str();
        while let Some(at) = rest.find(needle) {
            let after = rest[at + needle.len()..].trim_start();
            if after.starts_with(first_field) {
                found.push(file.display().to_string());
            }
            rest = &rest[at + needle.len()..];
        }
    }
    found
}

#[test]
fn no_production_source_builds_the_retired_commit() {
    let found = builders("WriteCommand::CommitStaged {", "staged_transaction_id:");
    assert!(
        found.is_empty(),
        "the retired CommitStaged has a producer: {found:?}"
    );
}

#[test]
fn no_production_source_builds_the_retired_skip() {
    let found = builders("WriteCommand::SkipStaged {", "staged_transaction_id:");
    assert!(
        found.is_empty(),
        "the retired SkipStaged has a producer: {found:?}"
    );
}

#[test]
fn the_kernel_no_longer_has_a_commit_or_skip_command_type() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let kernel = std::fs::read_to_string(root.join("crates/finance-kernel/src/lib.rs")).unwrap();
    assert!(!kernel.contains("pub struct CommitStaged"));
    assert!(!kernel.contains("pub struct SkipStaged"));
    assert!(kernel.contains("pub fn import_staged_anyway"));
    assert!(kernel.contains("pub fn skip_staged"));
}
