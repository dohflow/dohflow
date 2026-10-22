//! `from_derived` is the one way to wrap bytes as a service-bound header field (the tag, the
//! commitments, the fingerprint, the nonce domain), so every call site is listed here: a new one
//! is a reviewed change, and a call from anywhere else (a local storage id wrapped as a manifest
//! commitment, say) fails this test. Tests may call it freely.

use std::path::{Path, PathBuf};

/// Non-test source files allowed to call `from_derived(`: the derivations. `sync-core` itself
/// only defines it.
const ALLOWED: &[&str] = &[
    "crates/sync-core/src/header.rs",
    // The metadata and manifest commitments (C.3), derived here from CBE1 bytes.
    "crates/sync-core/src/envelope.rs",
    // The accepted-record decoder wraps the tag and commitment it reads off the wire.
    "crates/sync-core/src/accepted.rs",
    // 6b adds the commitment derivations here, and vault-crypto adds the tag and domain
    // derivations; list each file when it lands.
];

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

#[test]
fn from_derived_is_called_only_by_the_derivations() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for entry in std::fs::read_dir(workspace.join("crates"))
        .unwrap()
        .flatten()
    {
        walk(&entry.path().join("src"), &mut files);
    }
    walk(&workspace.join("apps/desktop/src-tauri/src"), &mut files);
    assert!(files.len() > 100, "found {} source files", files.len());
    let mut offenders = Vec::new();
    for path in files {
        let relative = path.strip_prefix(&workspace).unwrap().display().to_string();
        let relative = relative.replace('\\', "/");
        if relative.contains("/tests/") || relative.ends_with("_tests.rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path).unwrap();
        // Code only: drop line comments and doc comments.
        let code: String = source
            .lines()
            .map(|line| line.split("//").next().unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n");
        if code.contains("from_derived(") && !ALLOWED.contains(&relative.as_str()) {
            offenders.push(relative);
        }
    }
    assert!(
        offenders.is_empty(),
        "from_derived( called outside the derivations: {offenders:?}"
    );
}
