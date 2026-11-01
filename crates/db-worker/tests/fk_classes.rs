//! A class-1 table's foreign keys target class-1 tables (personal-cfo-g3m.26; the generalization of
//! the 07u rule, vault-format.md C.10 "Apply failures").
//!
//! A foreign-key violation raised while a command applies is a deterministic failure only if the
//! parent table is replicated state: a class-3 (device-local) or class-2 (rebuilt) parent would let
//! state that differs between devices decide whether the same command applies. Class 4 is the
//! group ADR 0074 Decision 5 routes to class 1 (the manifest still lists it as 4 until
//! `personal-cfo-5ymg8` finishes), so it counts as class 1 here.

mod common;

use common::*;
use std::collections::BTreeMap;

const CLASSES: &str = include_str!("../table_classes.toml");

fn classes() -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for block in CLASSES.split("[[table]]").skip(1) {
        let field = |key: &str| -> Option<String> {
            block.lines().find_map(|l| {
                let (k, v) = l.split_once('=')?;
                (k.trim() == key).then(|| v.trim().trim_matches('"').to_owned())
            })
        };
        if matches!(field("kind").as_deref(), Some("pseudo") | Some("planned")) {
            continue;
        }
        out.insert(field("table").unwrap(), field("class").unwrap());
    }
    out
}

/// Known violations, each owned by the bead that removes it. The test fails on a new one AND on a
/// listed one that no longer exists, so the list can only shrink.
const KNOWN_VIOLATIONS: &[(&str, &str)] = &[(
    "dedupe_decisions.staged_transaction_id -> staged_transactions",
    // The import dedupe decision (class 4, routed to class 1 by ADR 0074 Decision 5) references the
    // device-local staging row it decided on. personal-cfo-5ymg8.6 reshapes the decision to
    // reference replicated state only (or drops the foreign key) when it routes the writer.
    "personal-cfo-5ymg8.6",
)];

fn replicated(class: &str) -> bool {
    matches!(class, "1" | "1b" | "4")
}

#[test]
fn every_foreign_key_of_a_replicated_table_targets_a_replicated_table() {
    let classes = classes();
    let (_dir, worker) = worker();
    let conn = worker.read_connection().unwrap();
    let mut checked = 0;
    let mut violations = Vec::new();
    for (table, class) in &classes {
        if !replicated(class) {
            continue;
        }
        let mut stmt = conn
            .prepare(&format!(
                "SELECT \"table\", \"from\" FROM pragma_foreign_key_list('{table}')"
            ))
            .unwrap();
        let keys: Vec<(String, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        for (parent, column) in keys {
            checked += 1;
            let parent_class = classes.get(&parent).map(String::as_str);
            if !parent_class.is_some_and(replicated) {
                violations.push(format!("{table}.{column} -> {parent}"));
            }
        }
    }
    assert!(
        checked >= 10,
        "the schema's foreign keys were found ({checked})"
    );
    let known: Vec<String> = KNOWN_VIOLATIONS
        .iter()
        .map(|(v, _)| (*v).to_owned())
        .collect();
    assert_eq!(
        violations, known,
        "a replicated table must not reference device-local or derived state (a new violation, or \
         a listed one that is fixed and must leave KNOWN_VIOLATIONS)"
    );
}

#[test]
fn the_scanner_would_catch_a_replicated_to_device_local_reference() {
    // The rule's own teeth: a class-1 child of a class-3 parent is a violation.
    let classes = BTreeMap::from([
        ("child".to_owned(), "1".to_owned()),
        ("parent".to_owned(), "3".to_owned()),
    ]);
    assert!(replicated(&classes["child"]) && !replicated(&classes["parent"]));
}

const COLUMNS: &str = include_str!("../table_columns.toml");

/// Columns of a replicated table that name a class-2 forecast run without a foreign key
/// (personal-cfo-5ymg8.4). A run is derived and rebuilt locally, so a replicated row must never
/// carry one: every command writes these columns NULL, and the column stays only so that older
/// rows keep their value. A new `*_run_id` column of a replicated table has to be listed here, in
/// the change that adds it, with the rule that keeps it NULL.
const RUN_REFERENCES: &[&str] = &[
    // Written NULL by RecordAssumptionEvent, SupersedeAssumptionEvent and scenario promotion.
    "forecast_assumption_events.origin_run_id",
    // Written NULL by the scenario commands of personal-cfo-5ymg8.4's second slice.
    "scenarios.base_run_id",
];

#[test]
fn a_replicated_table_names_a_forecast_run_only_through_a_listed_null_column() {
    let classes = classes();
    let mut found = Vec::new();
    for block in COLUMNS.split("[[table]]").skip(1) {
        let field = |key: &str| -> Option<String> {
            block.lines().find_map(|l| {
                let (k, v) = l.split_once('=')?;
                (k.trim() == key).then(|| v.trim().to_owned())
            })
        };
        let name = field("name").unwrap().trim_matches('"').to_owned();
        if !classes.get(&name).is_some_and(|c| replicated(c)) {
            continue;
        }
        for column in field("columns").unwrap().split('"') {
            if let Some((column, _kind)) = column.split_once(':') {
                if column.ends_with("_run_id") {
                    found.push(format!("{name}.{column}"));
                }
            }
        }
    }
    assert_eq!(
        found, RUN_REFERENCES,
        "a replicated table gained (or lost) a column that names a forecast run"
    );
    // None of them is a foreign key: a run never replicates, so no constraint may depend on one.
    let (_dir, worker) = worker();
    let conn = worker.read_connection().unwrap();
    for listed in RUN_REFERENCES {
        let (table, column) = listed.split_once('.').unwrap();
        let references: i64 = conn
            .query_row(
                &format!(
                    "SELECT COUNT(*) FROM pragma_foreign_key_list('{table}') WHERE \"from\" = ?1"
                ),
                [column],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(references, 0, "{listed} must not be a foreign key");
    }
}
