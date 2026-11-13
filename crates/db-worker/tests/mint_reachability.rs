//! No random or time-based id mint is REACHABLE from `apply` (personal-cfo-32fmp.4, parent AC3; ADR
//! 0013 Addendum A). `id_mint_inventory.rs` lists every mint site and bans them under `apply/` and in
//! three hand-named helper modules; this test replaces that hand-kept list with a call-graph walk.
//!
//! The roots are the `apply_command` dispatcher and every function in `apply/`. The walk follows a call
//! to any function defined in `crates/db-worker/src` (non-test source) by NAME, the way the clock
//! ratchet does (`common/callgraph.rs`): it can over-report when two functions share a name, never
//! under-report a call written in the usual form, and an over-report is resolved by a rename or an
//! exception row. A reachable function whose body holds a mint token fails, and so does one whose SQL
//! text holds a SQLite random or clock function. An exception is a row of the inventory's "Mints
//! reachable from apply" table with the function, the token, the call edge it excuses and a bead.
//!
//! The workspace crates `apply` links are scanned too, by token (their functions are not walked): a
//! mint in `core-money`, `core-ledger`, `categorization`, `pay-schedule`, `forecast-engine`,
//! `importer-core` or `sync-core` production source fails; `job-runtime` is a named exception (`personal-cfo-32fmp.4`), `core-ids`
//! is scanned file by file with ONE allowed file (`lib.rs`, which DEFINES `monotonic_v7` and the id
//! macro, and only for those tokens; `personal-cfo-0g528.3`), so `derived.rs` and any new derivation
//! are scanned like every other crate, and `vault-crypto` is out of scope because no reachable
//! function mentions it (also asserted).

mod common;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;

use common::callgraph::{blank_noise, count_calls, function_spans, rust_files};

const INVENTORY: &str = include_str!("../../../docs/architecture/id-mint-inventory.md");

const MINT_TOKENS: &[&str] = &[
    "Uuid::now_v7",
    "Uuid::new_v4",
    "new_v7",
    "ContextV7",
    "next_id(",
    "monotonic_v7",
    "rand::",
    "getrandom",
    "thread_rng",
    "OsRng",
];

/// SQL text the engine must not run from apply: SQLite's own random and clock sources.
const SQL_TOKENS: &[&str] = &[
    "random()",
    "randomblob(",
    "'now'",
    "current_timestamp",
    "current_date",
    "current_time",
];

/// A source file: its production text (cut before a `#[cfg(test)] mod`), raw and noise-blanked.
struct Source {
    file: String,
    raw: String,
    blanked: String,
}

fn production_text(source: &str) -> String {
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
    lines[..cut].join("\n")
}

fn load(dir: &Path) -> Vec<Source> {
    let mut files = Vec::new();
    rust_files(dir, &mut files);
    files.sort();
    files
        .into_iter()
        .filter_map(|path| {
            let rel = path
                .strip_prefix(dir)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            if rel.starts_with("tests/") || rel.ends_with("tests.rs") || rel.ends_with("_tests.rs")
            {
                return None;
            }
            let raw = production_text(&std::fs::read_to_string(&path).unwrap());
            let blanked = blank_noise(&raw);
            Some(Source {
                file: rel,
                raw,
                blanked,
            })
        })
        .collect()
}

/// `<Entity>Id::new()` calls in blanked text (a generated id type minting a fresh id).
fn entity_id_news(text: &str) -> usize {
    let mut count = 0;
    let mut rest = text;
    while let Some(at) = rest.find("Id::new()") {
        if rest[..at]
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

fn mints_in(blanked_body: &str, raw_body: &str) -> Vec<String> {
    let mut found = Vec::new();
    for token in MINT_TOKENS {
        if blanked_body.contains(token) {
            found.push((*token).to_owned());
        }
    }
    if entity_id_news(blanked_body) > 0 {
        found.push("<Entity>Id::new()".to_owned());
    }
    let sql = raw_body.to_ascii_lowercase();
    for token in SQL_TOKENS {
        if sql.contains(token) {
            found.push(format!("SQL {token}"));
        }
    }
    found
}

#[derive(Debug, PartialEq, Eq)]
struct Finding {
    function: String,
    token: String,
    /// The call edge that reaches it, `root -> ... -> function`.
    edge: String,
}

struct Walk {
    reachable: BTreeSet<String>,
    findings: Vec<Finding>,
}

fn walk(sources: &[Source], roots: &[&str]) -> Walk {
    // name -> (blanked body, raw body) for every definition of that name.
    let mut table: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for source in sources {
        for (name, start, end) in function_spans(&source.blanked) {
            table.entry(name).or_default().push((
                source.blanked[start..end].to_owned(),
                source.raw[start..end].to_owned(),
            ));
        }
    }
    let mut parent: BTreeMap<String, Option<String>> = BTreeMap::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    for root in roots {
        if table.contains_key(*root) && parent.insert((*root).to_owned(), None).is_none() {
            queue.push_back((*root).to_owned());
        }
    }
    while let Some(name) = queue.pop_front() {
        for (blanked, _) in &table[&name] {
            for callee in table.keys() {
                if !parent.contains_key(callee) && count_calls(blanked, callee) > 0 {
                    parent.insert(callee.clone(), Some(name.clone()));
                    queue.push_back(callee.clone());
                }
            }
        }
    }
    let edge_to = |function: &str| {
        let mut path = vec![function.to_owned()];
        let mut at = function.to_owned();
        while let Some(Some(up)) = parent.get(&at) {
            path.push(up.clone());
            at = up.clone();
        }
        path.reverse();
        path.join(" -> ")
    };
    let mut findings = Vec::new();
    for function in parent.keys() {
        for (blanked, raw) in &table[function] {
            for token in mints_in(blanked, raw) {
                findings.push(Finding {
                    function: function.clone(),
                    token,
                    edge: edge_to(function),
                });
            }
        }
    }
    Walk {
        reachable: parent.keys().cloned().collect(),
        findings,
    }
}

/// The roots: the dispatcher and every function defined under `apply/`.
fn roots(sources: &[Source]) -> Vec<String> {
    let mut roots = vec!["apply_command".to_owned()];
    for source in sources.iter().filter(|s| s.file.starts_with("apply/")) {
        roots.extend(
            function_spans(&source.blanked)
                .into_iter()
                .map(|(name, _, _)| name),
        );
    }
    roots
}

/// The exception rows of the inventory: `(function, token, edge, bead)`.
fn exceptions() -> Vec<(String, String, String, String)> {
    let mut rows = Vec::new();
    let mut in_table = false;
    for line in INVENTORY.lines() {
        if line.starts_with("## Mints reachable from apply") {
            in_table = true;
        } else if line.starts_with("## ") {
            in_table = false;
        }
        if !in_table || !line.starts_with("| `") {
            continue;
        }
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        rows.push((
            cells[1].trim_matches('`').to_owned(),
            cells[2].trim_matches('`').to_owned(),
            cells[3].trim_matches('`').to_owned(),
            cells[4].to_owned(),
        ));
    }
    rows
}

fn synthetic(files: &[(&str, &str)]) -> Vec<Source> {
    files
        .iter()
        .map(|(file, text)| Source {
            file: (*file).to_owned(),
            raw: (*text).to_owned(),
            blanked: blank_noise(text),
        })
        .collect()
}

#[test]
fn no_random_or_time_based_mint_is_reachable_from_apply() {
    let sources = load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"));
    let roots = roots(&sources);
    let root_refs: Vec<&str> = roots.iter().map(String::as_str).collect();
    let walked = walk(&sources, &root_refs);
    eprintln!(
        "mint walk: {} roots, {} functions reachable from apply",
        roots.len(),
        walked.reachable.len()
    );
    assert!(
        walked.reachable.len() > 150,
        "the walk reached the engine ({})",
        walked.reachable.len()
    );

    let allowed = exceptions();
    let unexcused: Vec<String> = walked
        .findings
        .iter()
        .filter(|f| {
            !allowed.iter().any(|(function, token, edge, _)| {
                *function == f.function && *token == f.token && *edge == f.edge
            })
        })
        .map(|f| {
            format!(
                "{} mints `{}` and is reachable from apply by {}",
                f.function, f.token, f.edge
            )
        })
        .collect();
    assert!(
        unexcused.is_empty(),
        "a random or time-based mint is reachable from apply; derive the id (core_ids::DerivedIds) \
         or, if it cannot be, add an exception row with a bead to id-mint-inventory.md:\n{}",
        unexcused.join("\n")
    );
    for (function, token, edge, bead) in &allowed {
        assert!(
            bead.contains("personal-cfo-"),
            "{function}: an exception names a bead"
        );
        assert!(
            walked
                .findings
                .iter()
                .any(|f| f.function == *function && f.token == *token && f.edge == *edge),
            "the exception for {function} ({token}, {edge}) is stale: remove it"
        );
    }
}

#[test]
fn no_reachable_function_mentions_the_crypto_crate() {
    let sources = load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"));
    let roots = roots(&sources);
    let root_refs: Vec<&str> = roots.iter().map(String::as_str).collect();
    let walked = walk(&sources, &root_refs);
    let mut table: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for source in &sources {
        for (name, start, end) in function_spans(&source.blanked) {
            table
                .entry(name)
                .or_default()
                .push(source.blanked[start..end].to_owned());
        }
    }
    let users: Vec<&String> = walked
        .reachable
        .iter()
        .filter(|f| table[*f].iter().any(|body| body.contains("vault_crypto::")))
        .collect();
    assert!(
        users.is_empty(),
        "vault-crypto generates keys with real randomness; apply must not reach it: {users:?}"
    );
}

/// The one place a linked crate may name a mint token: `(crate, file, the only tokens allowed there,
/// bead)`. Every token listed must really occur there, or the row is stale.
const FILE_EXCEPTIONS: &[(&str, &str, &[&str], &str)] = &[(
    "core-ids",
    "lib.rs",
    // `monotonic_v7`, which DEFINES the ordinary-id mint that apply never calls (it derives ids
    // through `DerivedIds`); the `uuid_id!` macro only calls it.
    &["new_v7", "ContextV7", "monotonic_v7"],
    "personal-cfo-0g528.3",
)];

/// The mint findings in one linked crate's production source, as `file: [tokens]` lines. SQL text is
/// not scanned here (a linked crate holds no SQL that apply runs). A file with an exception row may
/// hold exactly its listed tokens and no other, and each listed token must be present.
fn linked_crate_findings(name: &str, sources: &[Source]) -> Vec<String> {
    let mut failures = Vec::new();
    for source in sources {
        let hits: BTreeSet<String> = mints_in(&source.blanked, &source.raw)
            .into_iter()
            .filter(|h| !h.starts_with("SQL "))
            .collect();
        let allowed = FILE_EXCEPTIONS
            .iter()
            .find(|(c, f, _, _)| *c == name && *f == source.file);
        let Some((_, _, tokens, bead)) = allowed else {
            if !hits.is_empty() {
                failures.push(format!("{name}/src/{}: {hits:?}", source.file));
            }
            continue;
        };
        let unexpected: Vec<&String> = hits
            .iter()
            .filter(|h| !tokens.contains(&h.as_str()))
            .collect();
        if !unexpected.is_empty() {
            failures.push(format!(
                "{name}/src/{}: {unexpected:?} beyond the tokens {bead} allows there ({tokens:?})",
                source.file
            ));
        }
        for token in *tokens {
            if !hits.contains(*token) {
                failures.push(format!(
                    "{name}/src/{}: the exception for {token} ({bead}) is stale: remove it",
                    source.file
                ));
            }
        }
    }
    for (c, f, _, bead) in FILE_EXCEPTIONS {
        if *c == name && !sources.iter().any(|s| s.file == *f) {
            failures.push(format!(
                "{name}/src/{f}: the exception row ({bead}) names a file that is gone"
            ));
        }
    }
    failures
}

/// The workspace crates apply links, scanned by token over their production source.
#[test]
fn the_crates_apply_links_hold_no_mint() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    // (crate, why a mint ANYWHERE in it is acceptable); an empty reason means none is. `core-ids` is
    // not here: it has a per-file row in `FILE_EXCEPTIONS` instead of a wholesale exemption.
    let scanned: &[(&str, &str)] = &[
        ("core-money", ""),
        ("core-ledger", ""),
        ("categorization", ""),
        ("pay-schedule", ""),
        ("forecast-engine", ""),
        ("importer-core", ""),
        ("importers/importer-core", ""),
        ("sync-core", ""),
        ("core-ids", ""),
        (
            "job-runtime",
            "personal-cfo-32fmp.4: durable jobs run outside apply; apply reaches none of it",
        ),
    ];
    // The real scan must route every crate with a per-file exception through `linked_crate_findings`
    // with no wholesale excuse, or a restored excuse (or a dropped entry) would pass unnoticed.
    for (crate_name, _, _, _) in FILE_EXCEPTIONS {
        assert!(
            scanned
                .iter()
                .any(|(name, excuse)| name == crate_name && excuse.is_empty()),
            "{crate_name} has a file exception, so it must be scanned with no wholesale excuse"
        );
    }
    // Every wholesale exemption that remains names its bead.
    for (name, excuse) in scanned {
        assert!(
            excuse.is_empty() || excuse.starts_with("personal-cfo-"),
            "the exemption for {name} must name its bead: {excuse:?}"
        );
    }
    let mut failures = Vec::new();
    let mut scanned_any = 0;
    for (name, excuse) in scanned {
        let src = crates.join(name).join("src");
        if !src.is_dir() {
            continue;
        }
        scanned_any += 1;
        if !excuse.is_empty() {
            continue;
        }
        failures.extend(linked_crate_findings(name, &load(&src)));
    }
    assert!(
        scanned_any >= 7,
        "the scan found the crates ({scanned_any})"
    );
    assert!(
        crates.join("core-ids").join("src").is_dir(),
        "core-ids is scanned, not skipped"
    );
    assert!(
        failures.is_empty(),
        "a linked crate holds a mint:\n{}",
        failures.join("\n")
    );
}

#[test]
fn a_mint_in_a_core_ids_derivation_is_caught_and_only_lib_rs_is_excused() {
    // A new derivation that reaches for a random or time-based id is a finding, in derived.rs or in
    // any file added later.
    let sources = synthetic(&[
        ("lib.rs", "pub fn monotonic_v7() { let _ = Uuid::new_v7(Timestamp::now(&ContextV7::new())); }\nmacro_rules! m { () => { Self($crate::monotonic_v7()) } }"),
        ("derived.rs", "pub fn provider_txn_id() { let _ = Uuid::new_v4(); }"),
        ("provider.rs", "pub fn p() { let _ = Uuid::now_v7(); let _ = OsRng; }"),
    ]);
    let findings = linked_crate_findings("core-ids", &sources);
    assert_eq!(findings.len(), 2, "{findings:?}");
    assert!(findings[0].contains("derived.rs") && findings[0].contains("Uuid::new_v4"));
    assert!(findings[1].contains("provider.rs"));

    // lib.rs may hold its listed tokens and nothing else: a random source there is a finding too.
    let sources = synthetic(&[(
        "lib.rs",
        "pub fn monotonic_v7() { let _ = Uuid::new_v7(Timestamp::now(&ContextV7::new())); }\nfn bad() { let _ = Uuid::new_v4(); }",
    )]);
    let findings = linked_crate_findings("core-ids", &sources);
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(findings[0].contains("Uuid::new_v4") && findings[0].contains("beyond"));

    // A row whose token is gone is stale, and another crate's lib.rs gets no excuse.
    let sources = synthetic(&[("lib.rs", "pub fn monotonic_v7() {}")]);
    assert!(linked_crate_findings("core-ids", &sources)
        .iter()
        .any(|f| f.contains("stale")));
    let sources = synthetic(&[("lib.rs", "pub fn f() { let _ = Uuid::new_v4(); }")]);
    assert_eq!(linked_crate_findings("sync-core", &sources).len(), 1);
}

#[test]
fn a_mint_two_calls_deep_from_an_apply_arm_is_caught() {
    let sources = synthetic(&[
        (
            "lib.rs",
            "fn apply_command() { match x { _ => apply::arm() } }",
        ),
        ("apply/arm.rs", "pub fn arm() { helper_a(); }"),
        (
            "helpers.rs",
            "fn helper_a() { helper_b(); }\nfn helper_b() { let _ = Uuid::now_v7(); }",
        ),
    ]);
    let roots = roots(&sources);
    let refs: Vec<&str> = roots.iter().map(String::as_str).collect();
    let walked = walk(&sources, &refs);
    assert_eq!(
        walked.findings,
        vec![Finding {
            function: "helper_b".to_owned(),
            token: "Uuid::now_v7".to_owned(),
            edge: "arm -> helper_a -> helper_b".to_owned(),
        }]
    );
}

#[test]
fn a_mint_in_an_unreachable_function_a_closure_a_method_call_and_sql_are_handled() {
    // Unreachable: not found.
    let sources = synthetic(&[
        ("lib.rs", "fn apply_command() { arm(); }"),
        ("apply/arm.rs", "pub fn arm() {}"),
        ("seed.rs", "fn seeder() { let _ = Uuid::now_v7(); }"),
    ]);
    let roots_ = roots(&sources);
    let refs: Vec<&str> = roots_.iter().map(String::as_str).collect();
    assert!(walk(&sources, &refs).findings.is_empty());

    // A mint inside a closure of a reachable function, one behind a method call, and one in SQL.
    let sources = synthetic(&[
        ("lib.rs", "fn apply_command() { arm(); }"),
        (
            "apply/arm.rs",
            "pub fn arm() { let f = || Uuid::new_v4(); self.fresh(); conn.execute(\"INSERT INTO t VALUES (randomblob(16))\", []); }",
        ),
        ("x.rs", "fn fresh(&self) { let _ = OsRng; }"),
    ]);
    let roots_ = roots(&sources);
    let refs: Vec<&str> = roots_.iter().map(String::as_str).collect();
    let tokens: BTreeSet<String> = walk(&sources, &refs)
        .findings
        .into_iter()
        .map(|f| f.token)
        .collect();
    assert!(tokens.contains("Uuid::new_v4"), "{tokens:?}");
    assert!(tokens.contains("OsRng"), "{tokens:?}");
    assert!(tokens.contains("SQL randomblob("), "{tokens:?}");
}

#[test]
fn two_functions_sharing_a_name_are_both_followed() {
    // The documented over-report: the walk follows the name, so a same-named function elsewhere is
    // reached too. The remedy is a rename or an exception row.
    let sources = synthetic(&[
        ("lib.rs", "fn apply_command() { arm(); }"),
        ("apply/arm.rs", "pub fn arm() { make(); }"),
        ("a.rs", "fn make() {}"),
        ("b.rs", "fn make() { let _ = Uuid::now_v7(); }"),
    ]);
    let roots_ = roots(&sources);
    let refs: Vec<&str> = roots_.iter().map(String::as_str).collect();
    assert_eq!(walk(&sources, &refs).findings.len(), 1);
}
