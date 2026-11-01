//! `WriteCommand::RevertScenarioApply` is retired (personal-cfo-5ie.12): it stays in the enum so an
//! operation log written before the fix decodes and replays as it did, and nothing produces it any
//! more. Every producer builds `RevertScenarioApplyConfirmed`, which carries the changes the
//! person confirmed.

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

#[test]
fn no_production_source_builds_the_retired_revert() {
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

    let mut found = Vec::new();
    for file in files {
        // Unit tests of the crate exercise the retired command on purpose (it still replays).
        if file.components().any(|part| part.as_os_str() == "tests") {
            continue;
        }
        let text = std::fs::read_to_string(&file).unwrap();
        let mut rest = text.as_str();
        let needle = "WriteCommand::RevertScenarioApply {";
        while let Some(at) = rest.find(needle) {
            let after = rest[at + needle.len()..].trim_start();
            // The enum's own arms match the variant: `{ .. }` or `{ scenario_id }`. A producer
            // builds it with a field value.
            if after.starts_with("scenario_id:") {
                found.push(file.display().to_string());
            }
            rest = &rest[at + needle.len()..];
        }
    }
    assert!(
        found.is_empty(),
        "the retired RevertScenarioApply has a producer: {found:?}"
    );
}

#[test]
fn the_kernel_producer_builds_the_confirmed_command() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let kernel = std::fs::read_to_string(root.join("crates/finance-kernel/src/lib.rs")).unwrap();
    assert!(kernel.contains("WriteCommand::RevertScenarioApplyConfirmed {"));
}
