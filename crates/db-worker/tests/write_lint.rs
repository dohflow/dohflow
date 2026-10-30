//! The module write lint (personal-cfo-5ymg8.1; 5ymg8 AC1).
//!
//! A replicated table (class 1, 1b or 4 in `table_classes.toml`) is written through the command
//! bus, in `apply/`. This lint scans the non-test source of `db-worker` for raw INSERT / UPDATE /
//! DELETE / REPLACE statements outside `apply/` and `migrations.rs`, partitions them over write
//! SITES (a module and a table, with the number of statements), and compares them with
//! `write_sites.toml`:
//!
//! * a site that is not listed fails (a new raw write to a replicated table must be routed through
//!   the bus or justified in the manifest, in the same change);
//! * a listed site that no longer exists, or whose statement count changed, fails (the checklist
//!   cannot go stale);
//! * every `pending` site names the child bead that will route it.
//!
//! **Warn mode.** The lint tolerates `pending` sites and prints them (run with `--nocapture`).
//! `personal-cfo-5ymg8.8` flips `FAIL_ON_PENDING` to `true` in its last commit, with no site left.
//!
//! A statement built by string concatenation, or a table named through a variable, is invisible to
//! a text scan; the lint is a ratchet against the common case, not a proof. Writes to class 2
//! (derived) and class 3 (device-local) tables are never violations.

use std::collections::BTreeMap;
use std::path::Path;

/// Flipped to `true` by `personal-cfo-5ymg8.8`, when no `pending` site remains.
const FAIL_ON_PENDING: bool = false;

const CLASSES: &str = include_str!("../table_classes.toml");
const SITES: &str = include_str!("../write_sites.toml");

/// The beads a pending site may name: the children of 5ymg8 and SYNC-2c.
const KNOWN_BEADS: &[&str] = &[
    "personal-cfo-5ymg8.2",
    "personal-cfo-5ymg8.3",
    "personal-cfo-5ymg8.4",
    "personal-cfo-5ymg8.5",
    "personal-cfo-5ymg8.6",
    "personal-cfo-5ymg8.7",
    "personal-cfo-5ymg8.9",
    "personal-cfo-0g528",
    "personal-cfo-339",
];

/// `key = "value"` / `key = 123` rows of `[[header]]` blocks.
fn parse_blocks(source: &str, header: &str) -> Vec<BTreeMap<String, String>> {
    let mut blocks = Vec::new();
    let mut current: Option<BTreeMap<String, String>> = None;
    for line in source.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == header {
            blocks.extend(current.take());
            current = Some(BTreeMap::new());
            continue;
        }
        if let (Some(block), Some((key, value))) = (current.as_mut(), line.split_once(" = ")) {
            block.insert(
                key.trim().to_owned(),
                value.trim().trim_matches('"').to_owned(),
            );
        }
    }
    blocks.extend(current);
    blocks
}

/// Table name -> class, from the CLASS-0 manifest.
fn classes() -> BTreeMap<String, String> {
    parse_blocks(CLASSES, "[[table]]")
        .into_iter()
        .map(|row| (row["table"].clone(), row["class"].clone()))
        .collect()
}

/// The `(table, statement kind)` of every write statement in `source`, outside comments and test
/// modules. Tables that are not in `tables` are ignored (an English "update the" in a string).
fn write_statements(source: &str, tables: &BTreeMap<String, String>) -> Vec<String> {
    let mut code = source.to_owned();
    // Drop an inline test module: `#[cfg(test)] mod name {` to the end of the file.
    if let Some(at) = find_test_module(&code) {
        code.truncate(at);
    }
    let code: String = code
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    let words: Vec<&str> = code
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.'))
        .filter(|w| !w.is_empty())
        .collect();
    let mut found = Vec::new();
    let upper = |w: &str| w.to_ascii_uppercase();
    for (i, word) in words.iter().enumerate() {
        let next = |n: usize| words.get(i + n).map(|w| upper(w));
        let table = match upper(word).as_str() {
            "INSERT" => {
                // INSERT [OR <verb>] INTO <table>
                let skip = if next(1).as_deref() == Some("OR") {
                    3
                } else {
                    1
                };
                (next(skip).as_deref() == Some("INTO"))
                    .then(|| words.get(i + skip + 1).copied())
                    .flatten()
            }
            // `INSERT OR REPLACE INTO` is the INSERT above, not a second statement.
            "REPLACE" => (next(1).as_deref() == Some("INTO")
                && (i == 0 || upper(words[i - 1]) != "OR"))
                .then(|| words.get(i + 2).copied())
                .flatten(),
            "UPDATE" => {
                // UPDATE [OR <verb>] <table>
                let skip = if next(1).as_deref() == Some("OR") {
                    3
                } else {
                    1
                };
                words.get(i + skip).copied()
            }
            "DELETE" => (next(1).as_deref() == Some("FROM"))
                .then(|| words.get(i + 2).copied())
                .flatten(),
            _ => None,
        };
        let Some(table) = table else { continue };
        let table = table
            .strip_prefix("main.")
            .unwrap_or(table)
            .to_ascii_lowercase();
        if tables.contains_key(&table) {
            found.push(table);
        }
    }
    found
}

fn find_test_module(source: &str) -> Option<usize> {
    let mut search = 0;
    while let Some(rel) = source[search..].find("#[cfg(test)]") {
        let at = search + rel;
        let rest = source[at + "#[cfg(test)]".len()..].trim_start();
        if let Some(after_mod) = rest.strip_prefix("mod ") {
            let name_end = after_mod
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(after_mod.len());
            if after_mod[name_end..].trim_start().starts_with('{') {
                return Some(at);
            }
        }
        search = at + 1;
    }
    None
}

fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// `(module path under src/, table)` -> statement count, for class 1 / 1b / 4 tables.
fn current_sites() -> BTreeMap<(String, String), usize> {
    let classes = classes();
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    walk(&src, &mut files);
    let mut sites = BTreeMap::new();
    for path in files {
        let module = path
            .strip_prefix(&src)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if module.starts_with("apply/")
            || module.starts_with("tests/")
            || module.ends_with("_tests.rs")
            || module == "migrations.rs"
        {
            continue;
        }
        let source = std::fs::read_to_string(&path).unwrap();
        for table in write_statements(&source, &classes) {
            if matches!(classes[&table].as_str(), "1" | "1b" | "4") {
                *sites.entry((module.clone(), table)).or_insert(0) += 1;
            }
        }
    }
    sites
}

#[test]
fn the_write_sites_manifest_is_exactly_the_raw_writes_to_replicated_tables() {
    let listed: BTreeMap<(String, String), usize> = parse_blocks(SITES, "[[site]]")
        .into_iter()
        .map(|site| {
            (
                (site["module"].clone(), site["table"].clone()),
                site["statements"].parse().expect("a statement count"),
            )
        })
        .collect();
    let current = current_sites();
    let unlisted: Vec<_> = current
        .iter()
        .filter(|(k, n)| listed.get(*k) != Some(*n))
        .collect();
    let stale: Vec<_> = listed
        .iter()
        .filter(|(k, _)| !current.contains_key(*k))
        .collect();
    assert!(
        unlisted.is_empty() && stale.is_empty(),
        "raw writes to replicated tables changed. Route a new write through the command bus \
         (apply/) or list it in write_sites.toml with its bead; remove a fixed one and fix a \
         count. Not listed or wrong count (module, table) -> statements: {unlisted:?}; listed \
         but gone: {stale:?}"
    );
}

#[test]
fn every_site_is_classified_justified_and_pending_ones_name_their_child() {
    let classes = classes();
    for site in parse_blocks(SITES, "[[site]]") {
        let (module, table) = (&site["module"], &site["table"]);
        assert_eq!(
            classes.get(table).map(String::as_str),
            site.get("class").map(String::as_str),
            "{module} {table}: the class must match table_classes.toml"
        );
        assert!(
            matches!(site["class"].as_str(), "1" | "1b" | "4"),
            "{module} {table}: only replicated classes are listed"
        );
        let note = site.get("note").map_or("", String::as_str);
        assert!(note.len() >= 20, "{module} {table}: a note saying why");
        let bead = site["bead"].as_str();
        assert!(bead.starts_with("personal-cfo-"), "{module} {table}");
        match site["status"].as_str() {
            "pending" => assert!(
                KNOWN_BEADS.contains(&bead),
                "{module} {table}: a pending site names its child bead, not {bead}"
            ),
            "bus_internal" | "allowed" => {}
            other => panic!("{module} {table}: unknown status {other}"),
        }
    }
}

/// Warn mode: print the checklist; fail only once the flip commit sets `FAIL_ON_PENDING`.
#[test]
fn pending_violations_are_reported_and_fail_once_the_lint_is_flipped() {
    let pending: Vec<String> = parse_blocks(SITES, "[[site]]")
        .into_iter()
        .filter(|s| s["status"] == "pending")
        .map(|s| {
            format!(
                "{} {} ({} statement(s)) -> {}",
                s["module"], s["table"], s["statements"], s["bead"]
            )
        })
        .collect();
    eprintln!(
        "write lint (warn mode): {} pending raw-write site(s):\n  {}",
        pending.len(),
        pending.join("\n  ")
    );
    if FAIL_ON_PENDING {
        assert!(
            pending.is_empty(),
            "raw writes to replicated tables remain: {pending:?}"
        );
    }
}

// -- the scanner itself ---------------------------------------------------------------------------

fn sample_tables() -> BTreeMap<String, String> {
    ["accounts", "tags", "scenarios"]
        .map(|t| (t.to_owned(), "1".to_owned()))
        .into()
}

#[test]
fn the_scanner_finds_each_statement_shape_and_ignores_prose_comments_and_test_modules() {
    let tables = sample_tables();
    let source = r#"
        // INSERT INTO accounts in a comment is ignored
        fn a() {
            conn.execute("INSERT INTO accounts (id) VALUES (?1)", []);
            conn.execute("insert or replace into main.tags (id) VALUES (?1)", []);
            conn.execute("INSERT OR IGNORE INTO tags (id) VALUES (?1)", []);
            conn.execute("REPLACE INTO scenarios (id) VALUES (?1)", []);
            conn.execute("UPDATE accounts SET name = ?1", []);
            conn.execute("UPDATE OR IGNORE tags SET name = ?1", []);
            conn.execute("UPDATE main.scenarios SET name = ?1", []);
            conn.execute("DELETE FROM scenarios WHERE id = ?1", []);
            conn.execute("INSERT INTO not_a_replicated_table VALUES (1)", []);
            let note = "we update the accounts page";
        }
        #[cfg(test)]
        mod tests {
            fn seed() { conn.execute("INSERT INTO accounts VALUES (1)", []); }
        }
    "#;
    let mut found = write_statements(source, &tables);
    found.sort();
    assert_eq!(
        found,
        [
            "accounts",
            "accounts",
            "scenarios",
            "scenarios",
            "scenarios",
            "tags",
            "tags",
            "tags"
        ]
    );
}

#[test]
fn a_cfg_test_module_declaration_without_a_body_does_not_hide_the_rest_of_the_file() {
    let tables = sample_tables();
    let source =
        "#[cfg(test)]\nmod tests;\nfn a() { conn.execute(\"DELETE FROM accounts\", []); }\n";
    assert_eq!(write_statements(source, &tables), ["accounts"]);
}
