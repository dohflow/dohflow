//! The id-mint inventory is exact (personal-cfo-32fmp.2; docs/architecture/id-mint-inventory.md):
//! every remaining random mint in `db-worker`'s non-test source is a row of the table with its
//! count; no mint under `apply/` or in a helper module `apply` reaches; and no SQL the engine runs
//! reads SQLite's clock or random source (03a-escalation's C3).

use std::collections::BTreeMap;
use std::path::Path;

const INVENTORY: &str = include_str!("../../../docs/architecture/id-mint-inventory.md");
const MIGRATIONS: &str = include_str!("../src/migrations.rs");

/// Modules `apply` reaches that must hold no raw mint at all.
const APPLY_REACHED_MODULES: &[&str] = &["debt.rs", "money_inbox.rs", "commitments.rs"];

const MINT_TOKENS: &[&str] = &[
    "Uuid::now_v7",
    "Uuid::new_v4",
    "uuid::Uuid::now_v7",
    "new_v7",
    "ContextV7",
    "next_id()",
];

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

fn production_lines(source: &str) -> Vec<String> {
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
    lines[..cut].iter().map(|l| (*l).to_owned()).collect()
}

/// The count of `<Entity>Id::new()` calls on a line (a generated id type minting a fresh id).
fn entity_id_news(line: &str) -> usize {
    let mut count = 0;
    let mut rest = line;
    while let Some(at) = rest.find("Id::new()") {
        let before = &rest[..at];
        if before
            .chars()
            .next_back()
            .is_some_and(|c| c.is_ascii_alphabetic())
        {
            count += 1;
        }
        rest = &rest[at + "Id::new()".len()..];
    }
    count
}

fn function_name(line: &str) -> Option<String> {
    let at = line.find("fn ")?;
    let name: String = line[at + 3..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    (!name.is_empty()).then_some(name)
}

/// `(file, function) -> mints` over the non-test source.
fn scanned() -> BTreeMap<(String, String), usize> {
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
        let mut function = String::new();
        for line in production_lines(&std::fs::read_to_string(&path).unwrap()) {
            if let Some(name) = function_name(&line) {
                function = name;
            }
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            let mut mints = entity_id_news(code);
            for token in MINT_TOKENS {
                mints += code.matches(token).count();
            }
            // `uuid::Uuid::now_v7` contains `Uuid::now_v7`: count once.
            mints -= code.matches("uuid::Uuid::now_v7").count();
            if mints > 0 {
                *found.entry((rel.clone(), function.clone())).or_insert(0) += mints;
            }
        }
    }
    found
}

fn listed() -> BTreeMap<(String, String), usize> {
    let mut rows = BTreeMap::new();
    let mut in_keep_table = false;
    for line in INVENTORY.lines() {
        if line.starts_with("## Remaining random mints") {
            in_keep_table = true;
        } else if line.starts_with("## ") {
            in_keep_table = false;
        }
        if !in_keep_table || !line.starts_with("| `") {
            continue;
        }
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        let file = cells[1].trim_matches('`').to_owned();
        let function = cells[2].trim_matches('`').to_owned();
        let count: usize = cells[3].parse().unwrap();
        rows.insert((file, function), count);
    }
    rows
}

#[test]
fn every_remaining_mint_is_a_row_of_the_inventory_with_its_count() {
    assert_eq!(
        scanned(),
        listed(),
        "a mint was added, removed or moved: update docs/architecture/id-mint-inventory.md in \
         the same change (inside apply, derive it instead)"
    );
}

#[test]
fn nothing_under_apply_or_in_the_modules_apply_reaches_mints_a_random_id() {
    for ((file, function), count) in scanned() {
        assert!(
            !file.starts_with("apply/"),
            "{file}::{function} mints {count} random id(s) under apply/: use core_ids::DerivedIds"
        );
        assert!(
            !APPLY_REACHED_MODULES.contains(&file.as_str()),
            "{file}::{function} mints {count} random id(s) in a module apply reaches"
        );
    }
}

#[test]
fn the_system_ledger_singleton_and_the_seeds_no_longer_mint_a_random_id() {
    // personal-cfo-32fmp.3: the create-if-missing system account derives its id from its content,
    // and the seeders derive theirs from the vault, so neither is a random mint any more.
    let still_random: Vec<_> = listed()
        .into_iter()
        .filter(|((file, function), _)| {
            file == "lib.rs"
                && matches!(
                    function.as_str(),
                    "ensure_system_ledger_account" | "ensure_default_categories"
                )
        })
        .collect();
    assert!(still_random.is_empty(), "{still_random:?}");
}

#[test]
fn no_sql_the_engine_runs_reads_the_sqlite_clock_or_random_source() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    source_files(&root, &mut files);
    let needles = [
        "randomblob(",
        "CURRENT_TIMESTAMP",
        "CURRENT_DATE",
        "datetime('now",
        "date('now",
        "'now')",
        "'now',",
        "julianday('now",
    ];
    let mut hits = BTreeMap::new();
    for path in files {
        let rel = path
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .to_string();
        if rel.starts_with("tests/") || rel.ends_with("tests.rs") || rel.ends_with("_tests.rs") {
            continue;
        }
        let production = production_lines(&std::fs::read_to_string(&path).unwrap()).join("\n");
        let found: usize = needles
            .iter()
            .map(|needle| {
                production
                    .to_lowercase()
                    .matches(&needle.to_lowercase())
                    .count()
            })
            .sum();
        if found > 0 {
            hits.insert(rel, found);
        }
    }
    // Migration 44's two strftime('now') stamps predate any genesis and are the only ones.
    assert_eq!(
        hits,
        BTreeMap::from([("migrations.rs".to_owned(), 2)]),
        "SQL reads SQLite's clock or random source"
    );
    // And they are in migration 44, not a later one.
    let at = MIGRATIONS
        .find("strftime('%Y-%m-%dT%H:%M:%SZ', 'now')")
        .unwrap();
    let before = &MIGRATIONS[..at];
    let version: i64 = before
        .rmatch_indices("version: ")
        .next()
        .map(|(i, _)| {
            before[i + 9..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse()
                .unwrap()
        })
        .unwrap();
    assert_eq!(version, 44);
}

/// Functions (by file) whose body contains a derived-id mint, with the body text.
fn minting_functions() -> Vec<(String, String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    source_files(&root, &mut files);
    let mut out = Vec::new();
    for path in files {
        let rel = path
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .to_string();
        if rel.starts_with("tests/") || rel.ends_with("tests.rs") || rel.ends_with("_tests.rs") {
            continue;
        }
        let mut current: Option<(String, String)> = None;
        for line in production_lines(&std::fs::read_to_string(&path).unwrap()) {
            if let Some(name) = function_name(&line) {
                if let Some((n, body)) = current.take() {
                    if body.contains("ids.next(") || body.contains("ids.keyed(") {
                        out.push((rel.clone(), n, body));
                    }
                }
                current = Some((name, String::new()));
            }
            if let Some((_, body)) = current.as_mut() {
                body.push_str(&line);
                body.push('\n');
            }
        }
        if let Some((n, body)) = current {
            if body.contains("ids.next(") || body.contains("ids.keyed(") {
                out.push((rel.clone(), n, body));
            }
        }
    }
    out
}

/// Mint order is deterministic (ADR 0013 Addendum A section 10(c)): a function that mints
/// iterates no hash-ordered collection, and a query whose rows feed it has a total order.
#[test]
fn a_function_that_mints_iterates_no_hash_collection_and_orders_every_row_set() {
    let functions = minting_functions();
    assert!(functions.len() >= 10, "the minting functions were found");
    for (file, function, body) in functions {
        assert!(
            !body.contains("HashMap") && !body.contains("HashSet"),
            "{file}::{function} mints ids and uses a hash-ordered collection"
        );
        if body.contains("query_map") || body.contains(".query(") {
            assert!(
                body.contains("ORDER BY"),
                "{file}::{function} mints ids from a row set without an ORDER BY"
            );
        }
    }
}
