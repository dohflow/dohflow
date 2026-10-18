//! The portable-export and Sync-genesis inventory in the public format
//! specification must cover every CLASS-0 table, follow its class, and name
//! every column of the tables whose fields are split (personal-cfo-klr.4).

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::worker;

const SPEC: &str = include_str!("../../../docs/architecture/vault-format.md");
const MANIFEST: &str = include_str!("../table_classes.toml");

const PORTABLE_DISPOSITIONS: &[&str] = &[
    "include", "fields", "history", "artifact", "rebuild", "exclude",
];
// Pre-Sync history is portable-only; a genesis snapshot carries state.
const SYNC_DISPOSITIONS: &[&str] = &["include", "fields", "artifact", "rebuild", "exclude"];

/// A row of the specification's inventory table.
struct Entry {
    class: String,
    portable: String,
    sync: String,
    rule: String,
}

/// `(table, class)` for every manifest row, including pseudo and planned rows.
fn manifest_classes() -> BTreeMap<String, String> {
    let mut rows = BTreeMap::new();
    let mut table: Option<String> = None;
    for line in MANIFEST.lines().map(str::trim) {
        if line == "[[table]]" {
            table = None;
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"');
        match key.trim() {
            "table" => table = Some(value.to_owned()),
            "class" => {
                let name = table.take().expect("class must follow table");
                assert!(
                    rows.insert(name.clone(), value.to_owned()).is_none(),
                    "duplicate manifest row {name}"
                );
            }
            _ => {}
        }
    }
    rows
}

fn inventory() -> BTreeMap<String, Entry> {
    let start = SPEC
        .find("<!-- inventory:start -->")
        .expect("inventory start marker");
    let end = SPEC
        .find("<!-- inventory:end -->")
        .expect("inventory end marker");
    let mut rows = BTreeMap::new();
    for line in SPEC[start..end].lines().filter(|l| l.starts_with("| `")) {
        let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
        assert_eq!(cells.len(), 5, "malformed inventory row: {line}");
        let table = cells[0].trim_matches('`').to_owned();
        let entry = Entry {
            class: cells[1].to_owned(),
            portable: cells[2].to_owned(),
            sync: cells[3].to_owned(),
            rule: cells[4].to_owned(),
        };
        assert!(
            rows.insert(table.clone(), entry).is_none(),
            "duplicate inventory row {table}"
        );
    }
    rows
}

#[test]
fn inventory_covers_every_manifest_table_with_its_class() {
    let manifest = manifest_classes();
    let spec = inventory();
    let manifest_names: BTreeSet<_> = manifest.keys().collect();
    let spec_names: BTreeSet<_> = spec.keys().collect();
    assert_eq!(
        manifest_names, spec_names,
        "every CLASS-0 row needs exactly one inventory row in vault-format.md"
    );
    for (table, class) in &manifest {
        assert_eq!(
            &spec[table].class, class,
            "{table}: inventory class disagrees with the manifest"
        );
    }
}

#[test]
fn dispositions_follow_the_class_unless_explicitly_excepted() {
    // The only tables whose treatment differs from their class default. Adding
    // an exception here is a deliberate, reviewed change to the contract.
    let exceptions: BTreeMap<&str, (&str, &str)> = BTreeMap::from([
        ("vault_metadata", ("fields", "fields")),
        ("settings", ("fields", "fields")),
        ("settings_user_keys", ("fields", "fields")),
        ("attachments", ("fields", "fields")),
        ("attachment_blobs", ("artifact", "artifact")),
        ("operation_log", ("history", "exclude")),
        ("provenance_links", ("history", "exclude")),
    ]);
    for (table, entry) in inventory() {
        assert!(
            PORTABLE_DISPOSITIONS.contains(&entry.portable.as_str()),
            "{table}: unknown portable disposition {}",
            entry.portable
        );
        assert!(
            SYNC_DISPOSITIONS.contains(&entry.sync.as_str()),
            "{table}: unknown Sync disposition {}",
            entry.sync
        );
        let expected =
            exceptions
                .get(table.as_str())
                .copied()
                .unwrap_or(match entry.class.as_str() {
                    "1" | "4" => ("include", "include"),
                    "1b" => ("include", "artifact"),
                    "2" => ("rebuild", "rebuild"),
                    "3" => ("exclude", "exclude"),
                    other => panic!("{table}: unknown class {other}"),
                });
        assert_eq!(
            (entry.portable.as_str(), entry.sync.as_str()),
            expected,
            "{table}: disposition differs from its class default; add a reviewed exception"
        );
        assert!(!entry.rule.is_empty(), "{table}: a rule must explain it");
    }
}

#[test]
fn credential_and_key_bearing_tables_never_travel() {
    let spec = inventory();
    for table in [
        "connector_connections",
        "connector_account_links",
        "command_idempotency_keys",
        "durable_jobs",
        "backup_history",
        "audit_events",
    ] {
        let entry = &spec[table];
        assert_eq!(
            (entry.portable.as_str(), entry.sync.as_str()),
            ("exclude", "exclude"),
            "{table} must not leave the vault"
        );
    }
}

/// Every column of a split-field table must be named in its inventory rule, as
/// either travelling or excluded, so a new column cannot be added unspecified.
#[test]
fn split_field_tables_name_every_real_column() {
    let spec = inventory();
    let (_dir, worker) = worker();
    let conn = worker.read_connection().unwrap();
    for table in ["vault_metadata", "attachments"] {
        let rule = &spec[table].rule;
        let mut stmt = conn
            .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))
            .unwrap();
        let columns: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(!columns.is_empty(), "{table} has no columns?");
        for column in columns {
            // `singleton` is the one-row guard, not data.
            if column == "singleton" {
                continue;
            }
            assert!(
                rule.contains(&format!("`{column}`")),
                "{table}.{column} is not named in the inventory rule"
            );
        }
    }
}

#[test]
fn legacy_setting_keys_match_the_manifest_count() {
    // The inventory says "the six legacy unprefixed user keys".
    let legacy = MANIFEST
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("legacy_persisted_keys = "))
        .expect("manifest lists the legacy keys")
        .trim_matches('"');
    assert_eq!(legacy.split(',').count(), 6);
    assert!(inventory()["settings"].rule.contains("six legacy"));
}
