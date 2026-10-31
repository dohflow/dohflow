//! The unclassified-rejection ledger (personal-cfo-g3m.26 part 2, 03a-escalation's M1): the number
//! of `DbError::InvalidCommand(` sites per module, which may only shrink. See
//! `unclassified_sites.toml` for why an unclassified rejection is a stall risk and not a verdict.

use std::collections::BTreeMap;
use std::path::Path;

const LEDGER: &str = include_str!("../unclassified_sites.toml");

fn source_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            source_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn production(source: &str) -> String {
    let lines: Vec<&str> = source.lines().collect();
    let cut = lines
        .iter()
        .enumerate()
        .find(|(i, l)| {
            l.trim() == "#[cfg(test)]"
                && lines.get(i + 1).is_some_and(|next| {
                    (next.starts_with("mod ") || next.starts_with("pub mod "))
                        && next.trim_end().ends_with('{')
                })
        })
        .map_or(lines.len(), |(i, _)| i);
    lines[..cut]
        .iter()
        .filter(|l| !l.trim_start().starts_with("//"))
        .copied()
        .collect::<Vec<_>>()
        .join("\n")
}

fn counted() -> BTreeMap<String, usize> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    source_files(&root, &mut files);
    let mut found = BTreeMap::new();
    for path in files {
        let rel = path
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .to_string();
        if rel.starts_with("tests/") || rel.ends_with("tests.rs") || rel.ends_with("_tests.rs") {
            continue;
        }
        let count = production(&std::fs::read_to_string(&path).unwrap())
            .matches("DbError::InvalidCommand(")
            .count();
        if count > 0 {
            found.insert(rel, count);
        }
    }
    found
}

fn ledger() -> BTreeMap<String, usize> {
    let mut sites = BTreeMap::new();
    let mut module = None;
    for line in LEDGER.lines().map(str::trim) {
        if let Some(value) = line.strip_prefix("module = ") {
            module = Some(value.trim_matches('"').to_owned());
        } else if let Some(value) = line.strip_prefix("statements = ") {
            sites.insert(module.take().unwrap(), value.parse().unwrap());
        }
    }
    sites
}

#[test]
fn the_unclassified_rejection_sites_only_shrink() {
    let (found, listed) = (counted(), ledger());
    for (module, count) in &found {
        let allowed = listed.get(module).copied().unwrap_or(0);
        assert!(
            *count <= allowed,
            "{module}: {count} unclassified DbError::InvalidCommand sites, the ledger allows {allowed}. \
             Classify the new rejection (DbError::reject, audited, in deterministic_sites.toml) or \
             justify it in the PR; this ledger never grows"
        );
    }
    assert_eq!(
        found, listed,
        "a site was classified or removed: lower unclassified_sites.toml to match (it only shrinks)"
    );
}
