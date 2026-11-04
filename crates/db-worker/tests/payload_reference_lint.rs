//! The payload-reference lint (personal-cfo-0g528.1, SYNC-2c; program plan v0.8.1 section 9.3):
//! an envelope's payload may reference only class-1 rows or class-1b artifacts. This test reads the
//! declaration of `WriteCommand`, and every payload struct it names, and maps each id-typed field to
//! the table it points at; the table's class comes from the CLASS-0 manifest (`table_classes.toml`).
//!
//! * A field that points at a class-3 table (a staging row) FAILS, naming the field, unless the
//!   variant is on the explicit retired list: `CommitStaged` and `SkipStaged` are decodable history
//!   that nothing produces.
//! * A field that points at a class-2 or class-4 table FAILS unless it is a named known gap with the
//!   bead that closes it; a stale entry fails too.
//! * A field with an id type or a `Uuid` that no mapping covers FAILS, so a new reference has to be
//!   classified in the change that adds it. The test grows with the enum by itself.
//! * No field may be named for a device-local staging id, a storage id or a plaintext digest.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const TABLE_CLASSES: &str = include_str!("../table_classes.toml");
const LIB: &str = include_str!("../src/lib.rs");

/// The table each typed id points at.
const TYPE_TARGET: &[(&str, &str)] = &[
    ("AccountId", "accounts"),
    ("BillContractId", "bill_contracts"),
    ("CategoryId", "categories"),
    ("IncomeSourceId", "income_sources"),
    ("RecurringEventId", "recurring_events"),
    ("RecurringTransferId", "recurring_transfers"),
    ("SourceBatchId", "source_batches"),
    ("SourceRecordId", "source_records"),
    ("StagedTransactionId", "staged_transactions"),
    ("TagId", "tags"),
    ("TransactionId", "ledger_transactions"),
    ("SplitLineId", "split_lines"),
];

/// The table a bare `Uuid` field points at, by `Variant.field` (a `Uuid` carries no type).
const FIELD_TARGET: &[(&str, &str)] = &[
    ("CreateConnectorConnection.id", "connector_connections"),
    ("UpsertConnectorLink.connection_id", "connector_connections"),
    (
        "SetConnectorLinkAccount.connection_id",
        "connector_connections",
    ),
    ("DeleteConnectorConnection.id", "connector_connections"),
    (
        "CreateSourceBatch.connector_connection_id",
        "connector_connections",
    ),
    ("SnoozeInboxItem.item_id", "money_inbox_read_model"),
    ("DismissInboxItem.item_id", "money_inbox_read_model"),
    ("ApplyScenario.scenario_id", "scenarios"),
    ("RevertScenarioApply.scenario_id", "scenarios"),
    ("RevertScenarioApplyConfirmed.scenario_id", "scenarios"),
    (
        "RevertScenarioApplyConfirmed.confirmed_descendants",
        "forecast_assumption_events",
    ),
    ("RecordBalanceAssertion.id", "balance_observations"),
    ("AddHouseholdMember.member_id", "household_members"),
    ("RenameHouseholdMember.member_id", "household_members"),
    ("TransferPrimary.from_member_id", "household_members"),
    ("TransferPrimary.to_member_id", "household_members"),
    ("RemoveHouseholdMember.member_id", "household_members"),
    ("RestoreHouseholdMember.member_id", "household_members"),
    ("SetMemberProfile.member_id", "household_members"),
    ("SupersedeAssumptionEvent.id", "forecast_assumption_events"),
    ("ClearAssumptionEvent.id", "forecast_assumption_events"),
    ("SetScenarioStatus.id", "scenarios"),
    ("RenameScenario.id", "scenarios"),
    ("SetScenarioExpiry.id", "scenarios"),
    ("CloneScenario.source_id", "scenarios"),
    ("CloneScenario.new_id", "scenarios"),
    ("CloneScenario.event_ids", "forecast_assumption_events"),
    ("DeleteScenario.id", "scenarios"),
    ("ReinstateScenario.id", "scenarios"),
    // Fields of the payload structs the variants carry.
    (
        "RecordAssumptionEvent.event.id",
        "forecast_assumption_events",
    ),
    (
        "RecordAssumptionEvent.event.target_entity_id",
        "forecast_assumption_events",
    ),
    ("RecordAssumptionEvent.event.scenario_id", "scenarios"),
    (
        "SupersedeAssumptionEvent.replacement.id",
        "forecast_assumption_events",
    ),
    (
        "SupersedeAssumptionEvent.replacement.target_entity_id",
        "forecast_assumption_events",
    ),
    (
        "SupersedeAssumptionEvent.replacement.scenario_id",
        "scenarios",
    ),
    ("CreateScenario.scenario.id", "scenarios"),
];

/// Variants retired from production: decodable history that nothing produces, so a staging
/// reference in their payload never crosses the service boundary. By variant name, on purpose.
const RETIRED: &[&str] = &["CommitStaged", "SkipStaged"];

/// Reference fields that point at a class-2 or class-4 table today, each with the bead that closes
/// it. An entry whose field no longer points at such a table fails as stale.
const KNOWN_GAPS: &[(&str, &str)] = &[
    // Snooze and dismiss name a Money Inbox item, which is a class-2 projection row; for an
    // imported item the id is a staging row's id. The fix is to key them on the underlying entity.
    ("SnoozeInboxItem.item_id", "personal-cfo-g3m.32"),
    ("DismissInboxItem.item_id", "personal-cfo-g3m.32"),
];

/// A field name that says the payload carries a device-local staging id, a storage id or a
/// plaintext digest.
const FORBIDDEN_NAME_PARTS: &[&str] = &["staged_transaction", "storage_id", "plaintext", "digest"];

#[derive(Debug, Clone)]
struct Field {
    /// `Variant.field`, with `.sub` for a field of a payload struct.
    path: String,
    variant: String,
    ty: String,
}

fn class_of_table() -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut table = None;
    for line in TABLE_CLASSES.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("table = \"") {
            table = Some(rest.trim_end_matches('"').to_owned());
        } else if let Some(rest) = line.strip_prefix("class = \"") {
            if let Some(table) = table.take() {
                out.insert(table, rest.trim_end_matches('"').to_owned());
            }
        }
    }
    out
}

/// The text between the braces of `pub struct <name> {` / `pub enum <name> {` in `source`.
fn braced_body<'a>(source: &'a str, header: &str) -> Option<&'a str> {
    let start = source.find(header)? + header.len();
    let open = source[start..].find('{')? + start + 1;
    let mut depth = 1;
    for (i, c) in source[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&source[open..open + i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// The CamelCase identifiers in a type expression.
fn type_names(ty: &str) -> Vec<String> {
    ty.split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|word| word.chars().next().is_some_and(char::is_uppercase))
        .map(str::to_owned)
        .collect()
}

fn is_id_type(name: &str) -> bool {
    name == "Uuid" || TYPE_TARGET.iter().any(|(ty, _)| *ty == name)
}

/// Every field of `WriteCommand`, and of the payload structs its fields name, flattened.
fn payload_fields(enum_source: &str, struct_sources: &[String]) -> Vec<Field> {
    let body = braced_body(enum_source, "pub enum WriteCommand").expect("the WriteCommand enum");
    let mut fields = Vec::new();
    let mut variant = String::new();
    for line in body.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") || trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }
        let indent = line.len() - trimmed.len();
        if indent == 4 {
            let name: String = trimmed
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if name.chars().next().is_some_and(char::is_uppercase) {
                variant = name;
                // A tuple variant: `Name(Type),`.
                if let Some(open) = trimmed.find('(') {
                    let inner = trimmed[open + 1..].trim_end_matches([',', ')', ' ']);
                    fields.push(Field {
                        path: format!("{variant}.0"),
                        variant: variant.clone(),
                        ty: inner.to_owned(),
                    });
                }
            }
        } else if indent == 8 {
            if let Some((name, ty)) = trimmed.split_once(": ") {
                fields.push(Field {
                    path: format!("{variant}.{name}"),
                    variant: variant.clone(),
                    ty: ty.trim_end_matches(',').to_owned(),
                });
            }
        }
    }

    // One level of nested payload structs, found by name in the sources.
    let mut expanded = Vec::new();
    for field in &fields {
        for name in type_names(&field.ty) {
            if is_id_type(&name) || name == "Option" || name == "Vec" || name == "Box" {
                continue;
            }
            let header = format!("pub struct {name} ");
            for source in struct_sources {
                let Some(inner) = braced_body(source, &header) else {
                    continue;
                };
                for line in inner.lines() {
                    let trimmed = line.trim_start();
                    if trimmed.starts_with("//") || trimmed.starts_with('#') {
                        continue;
                    }
                    let Some(rest) = trimmed.strip_prefix("pub ") else {
                        continue;
                    };
                    if let Some((sub, ty)) = rest.split_once(": ") {
                        expanded.push(Field {
                            path: format!("{}.{sub}", field.path),
                            variant: field.variant.clone(),
                            ty: ty.trim_end_matches(',').to_owned(),
                        });
                    }
                }
                break;
            }
        }
    }
    fields.extend(expanded);
    fields
}

/// The lint: the failures, each naming its field. Empty means the payloads are clean.
fn lint(fields: &[Field], classes: &BTreeMap<String, String>) -> Vec<String> {
    let mut failures = Vec::new();
    let mut gaps_used = BTreeSet::new();
    for field in fields {
        let leaf = field.path.rsplit('.').next().unwrap_or("");
        if FORBIDDEN_NAME_PARTS.iter().any(|part| leaf.contains(part))
            && !RETIRED.contains(&field.variant.as_str())
        {
            failures.push(format!(
                "{}: a payload field is named for a staging id, a storage id or a plaintext digest",
                field.path
            ));
        }
        let names = type_names(&field.ty);
        let Some(id_type) = names.iter().find(|name| is_id_type(name)) else {
            continue;
        };
        let target = if id_type == "Uuid" {
            FIELD_TARGET
                .iter()
                .find(|(path, _)| *path == field.path)
                .map(|(_, table)| *table)
        } else {
            TYPE_TARGET
                .iter()
                .find(|(ty, _)| ty == id_type)
                .map(|(_, table)| *table)
        };
        let Some(table) = target else {
            failures.push(format!(
                "{}: a Uuid field with no mapped target table; classify it in FIELD_TARGET",
                field.path
            ));
            continue;
        };
        let Some(class) = classes.get(table) else {
            failures.push(format!(
                "{}: the target table {table} is not in the manifest",
                field.path
            ));
            continue;
        };
        match class.as_str() {
            "1" | "1b" => {}
            "3" if RETIRED.contains(&field.variant.as_str()) => {}
            "3" => failures.push(format!(
                "{}: references {table}, a class-3 (device-local) table; an envelope may reference only \
                 class-1 rows or class-1b artifacts",
                field.path
            )),
            other => {
                if let Some((_, bead)) = KNOWN_GAPS.iter().find(|(path, _)| *path == field.path) {
                    gaps_used.insert(field.path.clone());
                    let _ = bead;
                } else {
                    failures.push(format!(
                        "{}: references {table}, a class-{other} table, and is not a known gap",
                        field.path
                    ));
                }
            }
        }
    }
    for (path, bead) in KNOWN_GAPS {
        if !gaps_used.contains(*path) && !failures.iter().any(|f| f.starts_with(path)) {
            failures.push(format!(
                "{path}: the known gap ({bead}) no longer points at a class-2 or class-4 table; \
                 remove it from KNOWN_GAPS"
            ));
        }
    }
    failures
}

fn struct_sources() -> Vec<String> {
    fn walk(dir: &Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(std::fs::read_to_string(&path).unwrap());
            }
        }
    }
    let mut out = Vec::new();
    walk(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut out);
    out
}

#[test]
fn every_payload_reference_points_at_class_one_or_a_class_one_b_artifact() {
    let fields = payload_fields(LIB, &struct_sources());
    assert!(
        fields.len() > 100,
        "the lint read the enum's fields ({} found)",
        fields.len()
    );
    let failures = lint(&fields, &class_of_table());
    assert!(
        failures.is_empty(),
        "payload references that cross the boundary:\n{}",
        failures.join("\n")
    );
}

#[test]
fn the_import_commit_references_only_class_one_and_one_b() {
    let fields: Vec<Field> = payload_fields(LIB, &struct_sources())
        .into_iter()
        .filter(|f| f.variant == "CommitImportedTransaction")
        .collect();
    let paths: BTreeSet<&str> = fields.iter().map(|f| f.path.as_str()).collect();
    for expected in [
        "CommitImportedTransaction.account_id",
        "CommitImportedTransaction.source_record_id",
        "CommitImportedTransaction.category_id",
        "CommitImportedTransaction.transaction_id",
    ] {
        assert!(paths.contains(expected), "the lint sees {expected}");
    }
    assert!(
        !paths.iter().any(|path| path.contains("staged")),
        "the import commit names no staging row: {paths:?}"
    );
    assert!(lint(&fields, &class_of_table())
        .iter()
        .all(|failure| !failure.starts_with("CommitImportedTransaction")));
}

#[test]
fn a_payload_that_names_a_staging_row_is_caught_with_its_field() {
    let wrong = "pub enum WriteCommand {\n    CommitNow {\n        staged_transaction_id: StagedTransactionId,\n        force: bool,\n    },\n}\n";
    let failures = lint(&payload_fields(wrong, &[]), &class_of_table());
    assert!(
        failures
            .iter()
            .any(|f| f.starts_with("CommitNow.staged_transaction_id")
                && f.contains("class-3")
                && f.contains("staged_transactions")),
        "{failures:?}"
    );
    // The same field on a retired variant is allowed: it is decodable history.
    let retired = wrong.replace("CommitNow", "CommitStaged");
    assert!(lint(&payload_fields(&retired, &[]), &class_of_table())
        .iter()
        .all(|f| !f.starts_with("CommitStaged")));
}

#[test]
fn a_digest_or_storage_id_field_and_an_unclassified_uuid_are_caught() {
    let wrong = "pub enum WriteCommand {\n    Leak {\n        plaintext_digest: String,\n        storage_id: String,\n        thing: Uuid,\n    },\n}\n";
    let failures = lint(&payload_fields(wrong, &[]), &class_of_table());
    for expected in ["Leak.plaintext_digest", "Leak.storage_id", "Leak.thing"] {
        assert!(
            failures.iter().any(|f| f.starts_with(expected)),
            "{expected} is caught: {failures:?}"
        );
    }
}

#[test]
fn a_nested_payload_struct_is_read_too() {
    let enum_source =
        "pub enum WriteCommand {\n    Bundle {\n        lines: Vec<Line>,\n    },\n}\n";
    let line = "pub struct Line {\n    pub staged_ref: StagedTransactionId,\n}\n".to_owned();
    let failures = lint(&payload_fields(enum_source, &[line]), &class_of_table());
    assert!(
        failures
            .iter()
            .any(|f| f.starts_with("Bundle.lines.staged_ref") && f.contains("class-3")),
        "{failures:?}"
    );
}
