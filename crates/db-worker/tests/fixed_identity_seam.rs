//! The fixed-vault-identity seam (`with_fixed_vault_identity`, personal-cfo-32fmp.4) lets a
//! conformance test create the same vault in every run. It must stay a test seam: no code outside
//! a crate's `tests/` directory may call it, so application code cannot reach it unnoticed.

use std::path::{Path, PathBuf};

const SEAM: &str = "with_fixed_vault_identity";

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if name != "target" && name != "node_modules" && !name.starts_with('.') {
                rust_files(&path, out);
            }
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn only_tests_and_the_definition_name_the_seam() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    rust_files(&root.join("crates"), &mut files);
    rust_files(&root.join("apps"), &mut files);
    assert!(
        files.len() > 50,
        "the scan found only {} files",
        files.len()
    );

    let definition = root.join("crates/db-worker/src/lib.rs");
    let mut offenders = Vec::new();
    for file in files {
        let source = std::fs::read_to_string(&file).expect("a readable source file");
        if !source.contains(SEAM) {
            continue;
        }
        let relative = file
            .canonicalize()
            .ok()
            .and_then(|p| {
                p.strip_prefix(root.canonicalize().unwrap())
                    .map(Path::to_path_buf)
                    .ok()
            })
            .unwrap_or_else(|| file.clone());
        let components: Vec<_> = relative
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        let in_a_crate_tests_dir =
            components.len() >= 3 && components[0] == "crates" && components[2] == "tests";
        if in_a_crate_tests_dir {
            continue;
        }
        if file.canonicalize().ok() == definition.canonicalize().ok() {
            // The definition and its documentation only: a call would be a line that is neither
            // the `pub fn` nor a doc comment.
            for line in source.lines().filter(|l| l.contains(SEAM)) {
                let trimmed = line.trim_start();
                if !(trimmed.starts_with("pub fn ") || trimmed.starts_with("//")) {
                    offenders.push(format!("{}: {trimmed}", relative.display()));
                }
            }
            continue;
        }
        offenders.push(relative.display().to_string());
    }
    assert!(
        offenders.is_empty(),
        "the fixed-identity seam is a test seam; these name it outside a tests/ directory: {offenders:?}"
    );
}
