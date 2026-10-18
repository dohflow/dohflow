//! A source-level guard for personal-cfo-g3m.6: no production code mints an
//! idempotency key from anything but the one CSPRNG helper.
//!
//! The format check at the command boundary rejects a bad key at run time; this
//! test keeps a new mint site from being written that way in the first place. It
//! scans every non-test Rust source under `crates/` and the desktop shell, so a
//! site added later is covered without being listed here.

use std::fs;
use std::path::{Path, PathBuf};

const HELPER: &str = "new_idempotency_key()";

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root")
}

fn production_sources() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if path.is_dir() {
                if matches!(name.as_ref(), "target" | "node_modules" | ".git" | "tests") {
                    continue;
                }
                walk(&path, out);
            } else if name.ends_with(".rs") {
                out.push(path);
            }
        }
    }
    let root = repo_root();
    let mut files = Vec::new();
    walk(&root.join("crates"), &mut files);
    walk(&root.join("apps/desktop/src-tauri/src"), &mut files);
    // The db-worker's own unit tests live under src/tests/ and are skipped by name.
    files
}

/// A statement `idempotency_key: <value>,` with its value joined across lines.
/// Struct field declarations (`idempotency_key: String,`) and shorthand
/// (`idempotency_key,`) are not mints and are skipped.
fn assigned_values(source: &str) -> Vec<(usize, String)> {
    let lines: Vec<&str> = source.lines().collect();
    let mut found = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") {
            continue;
        }
        let Some(rest) = trimmed
            .strip_prefix("idempotency_key:")
            .or_else(|| trimmed.strip_prefix("pub idempotency_key:"))
        else {
            continue;
        };
        let mut value = rest.trim().to_owned();
        let mut j = i;
        while !value.ends_with(',') && !value.ends_with(';') && j + 1 < lines.len() && j < i + 6 {
            j += 1;
            value.push(' ');
            value.push_str(lines[j].trim());
        }
        let bare = value.trim_end_matches([',', ';']).trim();
        // A type, not a value.
        if matches!(bare, "String" | "&str" | "Option<String>" | "&'a str") {
            continue;
        }
        found.push((i + 1, bare.to_owned()));
    }
    found
}

#[test]
fn no_production_site_mints_a_prefixed_or_time_ordered_key() {
    let root = repo_root();
    let mut violations = Vec::new();
    for path in production_sources() {
        let source = fs::read_to_string(&path).unwrap();
        for (line, value) in assigned_values(&source) {
            let rel = path.strip_prefix(&root).unwrap().display().to_string();
            // Deterministic keys exist only under cfg(test); the rekey test helper
            // builds one from fixed bytes.
            if rel.ends_with("crates/finance-kernel/src/rekey.rs")
                && value.contains("from_random_bytes")
            {
                continue;
            }
            let bad = value.contains("now_v7")
                || value.contains("new_v7")
                || value.contains("format!")
                || value.starts_with('"')
                || value.contains("from_bytes")
                || value.contains("from_random_bytes")
                || value.contains("new_v5");
            if bad {
                violations.push(format!("{rel}:{line}: idempotency_key = {value}"));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "idempotency keys must come from {HELPER} (a CSPRNG UUIDv4, no prefix):\n{}",
        violations.join("\n")
    );
}

/// The inventory of the four named mint sites, kept honest: each still mints
/// through the single helper (personal-cfo-g3m.6 AC1).
#[test]
fn the_known_mint_sites_use_the_single_helper() {
    let root = repo_root();
    let count = |rel: &str| {
        fs::read_to_string(root.join(rel))
            .unwrap()
            .matches(HELPER)
            .count()
    };
    assert!(count("crates/finance-kernel/src/lib.rs") >= 1, "next_meta");
    assert!(
        count("apps/desktop/src-tauri/src/ipc/commands.rs") >= 2,
        "resolve_idempotency_key and the connector auto-sync key"
    );
    assert!(
        count("apps/desktop/src-tauri/src/connector_refresh.rs") >= 1,
        "the scheduled connector refresh key"
    );
}

#[test]
fn the_scanner_flags_the_shapes_it_exists_to_catch() {
    let bad = [
        "x { idempotency_key: Uuid::now_v7().to_string(), }",
        "x { idempotency_key: format!(\"connector-refresh-{}\", id), }",
        "x { idempotency_key: \"fixed\".to_owned(), }",
    ];
    for source in bad {
        let values = assigned_values(&format!(
            "{}\n",
            source.replace("x { ", "").replace(" }", "")
        ));
        assert_eq!(values.len(), 1, "{source}");
        let value = &values[0].1;
        assert!(
            value.contains("now_v7") || value.contains("format!") || value.starts_with('"'),
            "{source}"
        );
    }
    // A type declaration and a helper call are fine.
    assert!(assigned_values("    pub idempotency_key: String,\n").is_empty());
    let ok = assigned_values("        idempotency_key: new_idempotency_key(),\n");
    assert_eq!(ok.len(), 1);
    assert!(!ok[0].1.contains("now_v7"));
}
