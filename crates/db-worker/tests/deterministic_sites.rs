//! The deterministic-rejection ratchet (personal-cfo-g3m.26): `DbError::reject(` may be called only
//! from the modules listed in `deterministic_sites.toml`, with the listed number of calls. An
//! unlisted call, a stale entry and a changed count all fail, so the classification cannot grow
//! (or rot) without the audit's ledger changing in the same change.

use std::collections::BTreeMap;
use std::path::Path;

const SITES: &str = include_str!("../deterministic_sites.toml");

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

/// The production part of a source file: everything before an inline `#[cfg(test)]` module, with
/// comment lines removed. (A test module in its own file is excluded by the caller.)
fn production(source: &str) -> String {
    let cut = source
        .find("#[cfg(test)]\nmod ")
        .or_else(|| source.find("#[cfg(test)]\n#[allow"))
        .unwrap_or(source.len());
    source[..cut]
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn calls() -> BTreeMap<String, usize> {
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
        // Files that are only test code.
        if rel.starts_with("tests/") || rel.ends_with("_tests.rs") {
            continue;
        }
        let count = production(&std::fs::read_to_string(&path).unwrap())
            .matches("DbError::reject(")
            .count();
        if count > 0 {
            found.insert(rel, count);
        }
    }
    found
}

fn listed() -> BTreeMap<String, usize> {
    let mut sites = BTreeMap::new();
    let mut module = None;
    for line in SITES.lines().map(str::trim) {
        if let Some(value) = line.strip_prefix("module = ") {
            module = Some(value.trim_matches('"').to_owned());
        } else if let Some(value) = line.strip_prefix("statements = ") {
            sites.insert(
                module.take().expect("module before statements"),
                value.parse().unwrap(),
            );
        }
    }
    sites
}

#[test]
fn deterministic_rejections_are_raised_only_at_listed_sites() {
    assert_eq!(
        calls(),
        listed(),
        "DbError::reject( sites changed: audit the validator (payload, metadata and class-1 state \
         only), prove it in tests/apply_rejections.rs, and list it in deterministic_sites.toml"
    );
}

#[test]
fn every_site_names_its_bead_and_a_note() {
    let blocks = SITES.split("[[site]]").skip(1).count();
    assert_eq!(SITES.matches("\nbead = ").count(), blocks);
    assert_eq!(SITES.matches("\nnote = ").count(), blocks);
}

#[test]
fn the_error_variants_that_are_not_verdicts_never_produce_an_apply_failure_code() {
    use db_worker::{DbError, RejectCode};
    for code in RejectCode::ALL {
        let error = DbError::Rejected {
            code,
            message: "x".to_owned(),
        };
        assert_eq!(
            error.apply_failure_code().map(|c| c.get()),
            Some(code.as_u16())
        );
    }
    for error in [
        DbError::InvalidCommand("x".to_owned()),
        DbError::WriterPanicked,
        DbError::VaultInUse,
        DbError::KeyUnavailable,
        DbError::AttachmentNotFound,
        DbError::Sqlite(rusqlite::Error::QueryReturnedNoRows),
        DbError::Io(std::io::Error::other("disk full")),
    ] {
        assert!(error.apply_failure_code().is_none(), "{error:?}");
    }
}

#[test]
fn the_registry_codes_are_stable_unique_and_listed_in_c10() {
    use db_worker::RejectCode;
    let spec = include_str!("../../../docs/architecture/vault-format.md");
    let mut seen = std::collections::BTreeSet::new();
    for (index, code) in RejectCode::ALL.iter().enumerate() {
        assert_eq!(
            usize::from(code.as_u16()),
            index + 1,
            "codes are 1..=n in order"
        );
        assert!(seen.insert(code.as_u16()));
        let row = format!("| `{}` | `{}` |", code.as_u16(), code.name());
        assert!(spec.contains(&row), "C.10 lists {row}");
    }
}
