//! CLASS-0 table manifest checks (personal-cfo-w21q7).
//!
//! The manifest is intentionally kept as data rather than wired into a write
//! path. These tests validate the data and compare the declared SQLite tables
//! with a real, freshly migrated SQLCipher vault.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::worker;
use db_worker::{
    AUTO_CATEGORIZE_ON_IMPORT_KEY, COMFORT_BAND_UPPER_KEY, FUTURE_CASH_SERIES_KEY, LOCALE_KEY,
    MINIMUM_CASH_FLOOR_KEY, PERSISTED_SETTING_KEYS, REPORTING_CURRENCY_KEY,
};
use rusqlite::Connection;

const MANIFEST: &str = include_str!("../table_classes.toml");
const DB_WORKER_SOURCE: &str = include_str!("../src/lib.rs");

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ManifestRow {
    fields: BTreeMap<String, String>,
}

impl ManifestRow {
    fn get(&self, key: &str) -> Option<&str> {
        self.fields.get(key).map(String::as_str)
    }

    fn required(&self, key: &str) -> Result<&str, String> {
        self.get(key)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                format!(
                    "manifest row {:?} is missing required field {key}",
                    self.get("table")
                )
            })
    }

    fn kind(&self) -> &str {
        self.get("kind").unwrap_or("sqlite_table")
    }
}

fn parse_manifest(source: &str) -> Result<Vec<ManifestRow>, String> {
    let mut rows = Vec::new();
    let mut current: Option<ManifestRow> = None;

    for (line_number, raw_line) in source.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == "[[table]]" {
            if let Some(row) = current.take() {
                rows.push(row);
            }
            current = Some(ManifestRow::default());
            continue;
        }

        let Some((key, raw_value)) = line.split_once('=') else {
            return Err(format!(
                "manifest line {} is not a table header or key/value pair",
                line_number + 1
            ));
        };
        let row = current.as_mut().ok_or_else(|| {
            format!(
                "manifest line {} has fields before its [[table]] header",
                line_number + 1
            )
        })?;
        let key = key.trim();
        if key.is_empty() {
            return Err(format!(
                "manifest line {} has an empty key",
                line_number + 1
            ));
        }
        let value = raw_value.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .ok_or_else(|| {
                format!(
                    "manifest line {} must use a basic TOML string",
                    line_number + 1
                )
            })?;
        row.fields
            .insert(key.to_owned(), value.replace("\\\"", "\""));
    }

    if let Some(row) = current {
        rows.push(row);
    }
    Ok(rows)
}

fn validate_manifest(rows: &[ManifestRow]) -> Result<(), String> {
    if rows.is_empty() {
        return Err("table_classes.toml contains no rows".to_owned());
    }

    let allowed_classes = ["1", "1b", "2", "3", "4"];
    let mut sqlite_tables = BTreeSet::new();
    for row in rows {
        let table = row.required("table")?;
        let class = row.required("class")?;
        if !allowed_classes.contains(&class) {
            return Err(format!(
                "manifest row {table:?} has unsupported class {class:?}"
            ));
        }
        row.required("owner_bead")?;
        row.required("note")?;

        match row.kind() {
            "sqlite_table" => {
                if !sqlite_tables.insert(table.to_owned()) {
                    return Err(format!("manifest has duplicate sqlite table {table:?}"));
                }
            }
            "pseudo" | "planned" => {}
            other => {
                return Err(format!(
                    "manifest row {table:?} has unsupported kind {other:?}"
                ));
            }
        }

        if class == "3" {
            let phase = row.required("rebase_phase")?;
            if !matches!(phase, "stable" | "replay_owned" | "mixed") {
                return Err(format!(
                    "class-3 row {table:?} must declare rebase_phase = stable, replay_owned, or mixed"
                ));
            }
            match phase {
                "stable" => {
                    row.required("rebase_preservation")?;
                    row.required("reference_validation")?;
                }
                "replay_owned" => {
                    row.required("rebase_preservation")?;
                    row.required("reference_validation")?;
                    row.required("correlate_to_live_plan")?;
                    row.required("auto_replay_remap")?;
                    row.required("queued_disposition")?;
                }
                "mixed" => {
                    row.required("rebase_preservation")?;
                    row.required("reference_validation")?;
                    row.required("stable_selector")?;
                    row.required("stable_rebase_preservation")?;
                    row.required("stable_reference_validation")?;
                    row.required("replay_owned_selector")?;
                    row.required("replay_owned_rebase_preservation")?;
                    row.required("replay_owned_reference_validation")?;
                    row.required("replay_owned_correlate_to_live_plan")?;
                    row.required("replay_owned_auto_replay_remap")?;
                    row.required("replay_owned_queued_disposition")?;
                }
                _ => unreachable!(),
            }
        }

        if class == "1b" {
            row.required("local_identity")?;
            let opaque_reference = row.required("opaque_sync_reference")?;
            if opaque_reference.to_ascii_lowercase().contains("storageid")
                || opaque_reference
                    .to_ascii_lowercase()
                    .contains("plaintext digest")
            {
                return Err(format!(
                    "class-1b row {table:?} exposes a local StorageId or bare plaintext digest"
                ));
            }
            let closure_role = row.required("closure_role")?;
            if !matches!(closure_role, "snapshot-full" | "envelope-delta") {
                return Err(format!(
                    "class-1b row {table:?} has invalid closure_role {closure_role:?}"
                ));
            }
            row.required("retention")?;
            row.required("reference_validation")?;
        }

        if class == "4"
            && (!row.required("note")?.contains("ADR 0074")
                || !row.required("note")?.contains("SYNC-2"))
        {
            return Err(format!(
                "class-4 row {table:?} must record its ADR 0074 / SYNC-2 candidate disposition"
            ));
        }
    }

    let settings = rows
        .iter()
        .find(|row| row.get("table") == Some("settings"))
        .ok_or_else(|| "manifest has no settings row".to_owned())?;
    validate_settings_keys(settings)?;

    let attachments = rows
        .iter()
        .find(|row| row.get("table") == Some("attachments"))
        .ok_or_else(|| "manifest has no attachments row".to_owned())?;
    validate_attachment_boundary(attachments)?;

    let attachment_links = rows
        .iter()
        .find(|row| row.get("table") == Some("attachment_links"))
        .ok_or_else(|| "manifest has no attachment_links row".to_owned())?;
    validate_attachment_link_boundary(attachment_links)?;

    Ok(())
}

fn validate_settings_keys(row: &ManifestRow) -> Result<(), String> {
    let legacy = row.get("legacy_settings_keys").unwrap_or("");
    let legacy: BTreeSet<&str> = legacy
        .split(',')
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .collect();
    if legacy != BTreeSet::from(["locale", "reporting_currency"]) {
        return Err(format!(
            "settings legacy allowlist must be locale/reporting_currency, got {legacy:?}"
        ));
    }

    let persisted = row.required("legacy_persisted_keys")?;
    let persisted: BTreeSet<&str> = persisted
        .split(',')
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .collect();
    let expected = BTreeSet::from([
        "locale",
        "reporting_currency",
        "minimum_cash_floor_minor",
        "comfort_band_upper_minor",
        "auto_categorize_on_import",
        "future_cash_series_selection",
    ]);
    if persisted != expected {
        return Err(format!(
            "settings persisted inventory must enumerate the six current keys, got {persisted:?}"
        ));
    }

    let scopes = row.required("legacy_key_scope")?;
    let scopes: BTreeSet<_> = scopes
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .collect();
    let expected_scopes = BTreeSet::from([
        "locale=user",
        "reporting_currency=user",
        "minimum_cash_floor_minor=user",
        "comfort_band_upper_minor=user",
        "auto_categorize_on_import=user",
        "future_cash_series_selection=user",
    ]);
    if scopes != expected_scopes {
        return Err(format!(
            "settings legacy scope inventory must mark each current preference user-scoped, got {scopes:?}"
        ));
    }
    if row.required("prefix_migration_bead")? != "personal-cfo-5ymg8" {
        return Err("settings prefix migration must be owned by personal-cfo-5ymg8".to_owned());
    }

    for key in row
        .get("new_setting_keys")
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|key| !key.is_empty())
    {
        if !(key.starts_with("device.") || key.starts_with("user.")) {
            return Err(format!(
                "new settings key {key:?} must begin with device. or user."
            ));
        }
    }
    Ok(())
}

fn validate_attachment_boundary(row: &ManifestRow) -> Result<(), String> {
    let canonical = row.required("canonical_fields")?;
    for field in [
        "id",
        "plaintext_size",
        "mime_type",
        "original_filename",
        "created_at",
    ] {
        if !canonical
            .split(',')
            .any(|candidate| candidate.trim() == field)
        {
            return Err(format!(
                "attachments canonical_fields must include {field:?}"
            ));
        }
    }
    let derived = row.required("derived_fields")?;
    for field in ["content_alg", "ref_count"] {
        if !derived
            .split(',')
            .any(|candidate| candidate.trim() == field)
        {
            return Err(format!("attachments derived_fields must include {field:?}"));
        }
    }
    let receiver_local = row.required("receiver_local_fields")?;
    for field in [
        "storage_id",
        "wrapped_content_key",
        "content_key_nonce",
        "content_nonce",
    ] {
        if !receiver_local
            .split(',')
            .any(|candidate| candidate.trim() == field)
        {
            return Err(format!(
                "attachments receiver_local_fields must include {field:?}"
            ));
        }
        if canonical
            .split(',')
            .any(|candidate| candidate.trim() == field)
        {
            return Err(format!(
                "attachments service-facing canonical fields expose local crypto field {field:?}"
            ));
        }
    }
    if !row
        .required("receiver_transport")?
        .contains("SyncArtifactTransportV1")
        || !row
            .required("materialization_failure")?
            .contains("ArtifactMaterializationFailed")
    {
        return Err(
            "attachments must declare receiver transport and fail-closed materialization"
                .to_owned(),
        );
    }
    Ok(())
}

fn validate_attachment_link_boundary(row: &ManifestRow) -> Result<(), String> {
    let canonical = row.required("canonical_fields")?;
    for field in ["attachment_id", "entity_kind", "entity_id", "created_at"] {
        if !canonical
            .split(',')
            .any(|candidate| candidate.trim() == field)
        {
            return Err(format!(
                "attachment_links canonical_fields must include {field:?}"
            ));
        }
    }
    if !row.get("derived_fields").unwrap_or("").trim().is_empty()
        || !row
            .get("receiver_local_fields")
            .unwrap_or("")
            .trim()
            .is_empty()
    {
        return Err("attachment_links has no derived or receiver-local fields".to_owned());
    }
    Ok(())
}

fn manifest_field_set(row: &ManifestRow, key: &str) -> BTreeSet<String> {
    row.get(key)
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|field| !field.is_empty())
        .map(str::to_owned)
        .collect()
}

fn table_columns(conn: &Connection, table: &str) -> Result<BTreeSet<String>, rusqlite::Error> {
    let pragma = match table {
        "attachments" => "PRAGMA table_info(attachments)",
        "attachment_links" => "PRAGMA table_info(attachment_links)",
        _ => return Ok(BTreeSet::new()),
    };
    let mut stmt = conn.prepare(pragma)?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<BTreeSet<_>, _>>()?;
    Ok(columns)
}

fn validate_attachment_schema(
    conn: &Connection,
    attachments: &ManifestRow,
    attachment_links: &ManifestRow,
) -> Result<(), String> {
    let actual_attachments =
        table_columns(conn, "attachments").map_err(|error| error.to_string())?;
    let declared_attachments = manifest_field_set(attachments, "canonical_fields")
        .into_iter()
        .chain(manifest_field_set(attachments, "derived_fields"))
        .chain(manifest_field_set(attachments, "receiver_local_fields"))
        .collect::<BTreeSet<_>>();
    if actual_attachments != declared_attachments {
        return Err(format!(
            "attachments manifest columns {declared_attachments:?} do not match PRAGMA table_info {actual_attachments:?}"
        ));
    }

    let actual_links =
        table_columns(conn, "attachment_links").map_err(|error| error.to_string())?;
    let declared_links = manifest_field_set(attachment_links, "canonical_fields")
        .into_iter()
        .chain(manifest_field_set(attachment_links, "derived_fields"))
        .chain(manifest_field_set(
            attachment_links,
            "receiver_local_fields",
        ))
        .collect::<BTreeSet<_>>();
    if actual_links != declared_links {
        return Err(format!(
            "attachment_links manifest columns {declared_links:?} do not match PRAGMA table_info {actual_links:?}"
        ));
    }
    Ok(())
}

fn migrated_table_names(conn: &Connection) -> Result<BTreeSet<String>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master
         WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
         ORDER BY name",
    )?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(names
        .into_iter()
        .filter(|name| !name.ends_with("_new") && !name.ends_with("_old"))
        .collect())
}

fn manifest_sqlite_tables(rows: &[ManifestRow]) -> BTreeSet<String> {
    rows.iter()
        .filter(|row| row.kind() == "sqlite_table")
        .filter_map(|row| row.get("table"))
        .map(str::to_owned)
        .collect()
}

fn validate_sqlite_tables(actual: &BTreeSet<String>, rows: &[ManifestRow]) -> Result<(), String> {
    let declared = manifest_sqlite_tables(rows);
    let unknown: Vec<_> = actual.difference(&declared).cloned().collect();
    let missing: Vec<_> = declared.difference(actual).cloned().collect();
    if !unknown.is_empty() {
        return Err(format!(
            "unclassified table(s) {unknown:?}; classify it in table_classes.toml (plan §9.3)"
        ));
    }
    if !missing.is_empty() {
        return Err(format!(
            "manifest table(s) {missing:?} do not exist in the migrated vault"
        ));
    }
    Ok(())
}

fn manifest_rows() -> Vec<ManifestRow> {
    let rows = parse_manifest(MANIFEST).expect("table_classes.toml parses");
    validate_manifest(&rows).expect("table_classes.toml satisfies CLASS-0 validation");
    rows
}

fn source_defined_setting_keys(source: &str) -> BTreeSet<String> {
    source
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if !line.starts_with("pub const ") || !line.contains("_KEY: &str =") {
                return None;
            }
            let (_, value) = line.split_once("= ")?;
            Some(
                value
                    .trim()
                    .trim_end_matches(';')
                    .trim_matches('"')
                    .to_owned(),
            )
        })
        .collect()
}

fn source_unprefixed_setting_keys(source: &str) -> BTreeSet<String> {
    source_defined_setting_keys(source)
        .into_iter()
        .filter(|key| !key.contains('.'))
        .collect()
}

#[test]
fn fresh_vault_tables_have_exactly_one_manifest_row() {
    let rows = manifest_rows();
    let (_dir, worker) = worker();
    let conn = worker.read_connection().unwrap();
    let actual = migrated_table_names(&conn).unwrap();
    validate_sqlite_tables(&actual, &rows).unwrap();
    assert_eq!(actual, manifest_sqlite_tables(&rows));

    let attachments = rows
        .iter()
        .find(|row| row.get("table") == Some("attachments"))
        .unwrap();
    let attachment_links = rows
        .iter()
        .find(|row| row.get("table") == Some("attachment_links"))
        .unwrap();
    validate_attachment_schema(&conn, attachments, attachment_links).unwrap();
}

#[test]
fn unknown_table_reports_manifest_and_plan() {
    let rows = manifest_rows();
    let (_dir, worker) = worker();
    let conn = worker.read_connection().unwrap();
    conn.execute_batch("CREATE TABLE unclassified_table (id INTEGER NOT NULL);")
        .unwrap();
    let actual = migrated_table_names(&conn).unwrap();
    let error = validate_sqlite_tables(&actual, &rows).unwrap_err();
    assert!(error.contains("unclassified_table"), "{error}");
    assert!(error.contains("table_classes.toml"), "{error}");
    assert!(error.contains("plan §9.3"), "{error}");
}

#[test]
fn migration_temporaries_are_not_manifest_tables() {
    let actual = BTreeSet::from([
        "accounts".to_owned(),
        "forecast_assumption_events_new".to_owned(),
        "forecast_assumption_events_old".to_owned(),
        "risk_flags_new".to_owned(),
        "risk_flags_old".to_owned(),
    ]);
    let filtered: BTreeSet<_> = actual
        .into_iter()
        .filter(|name| !name.ends_with("_new") && !name.ends_with("_old"))
        .collect();
    assert_eq!(filtered, BTreeSet::from(["accounts".to_owned()]));
}

#[test]
fn class3_validation_rejects_missing_rebase_fields() {
    let mut rows = manifest_rows();
    let row = rows
        .iter_mut()
        .find(|row| row.get("table") == Some("vault_meta"))
        .unwrap();
    row.fields.remove("rebase_preservation");
    let error = validate_manifest(&rows).unwrap_err();
    assert!(error.contains("vault_meta"), "{error}");
    assert!(error.contains("rebase_preservation"), "{error}");
}

#[test]
fn replay_owned_class3_validation_requires_all_disposition_evidence() {
    let mut rows = manifest_rows();
    let row = rows
        .iter_mut()
        .find(|row| row.get("table") == Some("command_idempotency_keys"))
        .unwrap();
    row.fields.remove("queued_disposition");
    let error = validate_manifest(&rows).unwrap_err();
    assert!(error.contains("command_idempotency_keys"), "{error}");
    assert!(error.contains("queued_disposition"), "{error}");
}

#[test]
fn mixed_audit_rebase_validation_requires_every_replay_field() {
    let replay_fields = [
        "replay_owned_selector",
        "replay_owned_rebase_preservation",
        "replay_owned_reference_validation",
        "replay_owned_correlate_to_live_plan",
        "replay_owned_auto_replay_remap",
        "replay_owned_queued_disposition",
    ];
    for field in replay_fields {
        let mut rows = manifest_rows();
        let row = rows
            .iter_mut()
            .find(|row| row.get("table") == Some("audit_events"))
            .unwrap();
        row.fields.remove(field);
        let error = validate_manifest(&rows).unwrap_err();
        assert!(error.contains("audit_events"), "{field}: {error}");
        assert!(error.contains(field), "{field}: {error}");
    }
}

#[test]
fn mixed_audit_rebase_selects_the_declared_runtime_branch() {
    let row = manifest_rows()
        .into_iter()
        .find(|row| row.get("table") == Some("audit_events"))
        .unwrap();

    let stable_event = "capability_ack";
    assert!(!stable_event.contains("planned_command"));
    assert!(row
        .required("stable_selector")
        .unwrap()
        .contains("security"));
    assert!(row
        .required("stable_rebase_preservation")
        .unwrap()
        .contains("immutable"));

    let planned_event = "planned_command_applied";
    assert!(planned_event.contains("planned_command"));
    assert!(row
        .required("replay_owned_selector")
        .unwrap()
        .contains("planned-command"));
    assert!(row
        .required("replay_owned_auto_replay_remap")
        .unwrap()
        .contains("op_seq"));
}

#[test]
fn settings_key_convention_allows_only_declared_legacy_names() {
    let rows = manifest_rows();
    let settings = rows
        .iter()
        .find(|row| row.get("table") == Some("settings"))
        .unwrap();
    validate_settings_keys(settings).unwrap();

    let mut invalid = settings.clone();
    invalid.fields.insert(
        "new_setting_keys".to_owned(),
        "reporting_currency".to_owned(),
    );
    let error = validate_settings_keys(&invalid).unwrap_err();
    assert!(
        error.contains("device.") && error.contains("user."),
        "{error}"
    );

    let mut prefixed = settings.clone();
    prefixed.fields.insert(
        "new_setting_keys".to_owned(),
        "device.new_cache,user.new_preference".to_owned(),
    );
    validate_settings_keys(&prefixed).unwrap();
}

#[test]
fn settings_manifest_matches_the_actual_persisted_key_inventory() {
    let rows = manifest_rows();
    let settings = rows
        .iter()
        .find(|row| row.get("table") == Some("settings"))
        .unwrap();
    let expected: BTreeSet<String> = [
        LOCALE_KEY,
        REPORTING_CURRENCY_KEY,
        MINIMUM_CASH_FLOOR_KEY,
        COMFORT_BAND_UPPER_KEY,
        AUTO_CATEGORIZE_ON_IMPORT_KEY,
        FUTURE_CASH_SERIES_KEY,
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    assert_eq!(source_unprefixed_setting_keys(DB_WORKER_SOURCE), expected);
    let declared: BTreeSet<_> = settings
        .required("legacy_persisted_keys")
        .unwrap()
        .split(',')
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_owned)
        .collect();
    assert_eq!(declared, expected);

    let (_dir, worker) = worker();
    for key in PERSISTED_SETTING_KEYS {
        worker.set_setting(key, "inventory-test").unwrap();
    }
    let conn = worker.read_connection().unwrap();
    let persisted: BTreeSet<String> = conn
        .prepare("SELECT key FROM settings ORDER BY key")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(persisted, expected);
}

#[test]
fn settings_inventory_rejects_a_new_unprefixed_constant() {
    let source =
        format!("{DB_WORKER_SOURCE}\npub const NEW_UNPREFIXED_KEY: &str = \"new_unprefixed\";\n");
    let actual = source_unprefixed_setting_keys(&source);
    let expected: BTreeSet<String> = PERSISTED_SETTING_KEYS
        .iter()
        .map(|key| (*key).to_owned())
        .collect();
    assert!(actual.contains("new_unprefixed"));
    assert!(!actual.is_subset(&expected));

    let prefixed_source =
        format!("{DB_WORKER_SOURCE}\npub const NEW_DEVICE_KEY: &str = \"device.new_cache\";\n");
    assert_eq!(source_unprefixed_setting_keys(&prefixed_source), expected);
}

#[test]
fn class1b_and_attachment_boundaries_are_declared() {
    let rows = manifest_rows();
    let class1b = rows.iter().filter(|row| row.get("class") == Some("1b"));
    assert!(class1b.clone().count() >= 4);
    for row in class1b {
        let service_reference = row.required("opaque_sync_reference").unwrap();
        assert!(!service_reference.to_ascii_lowercase().contains("storageid"));
        assert!(!service_reference
            .to_ascii_lowercase()
            .contains("plaintext digest"));
    }
    let attachments = rows
        .iter()
        .find(|row| row.get("table") == Some("attachments"))
        .unwrap();
    validate_attachment_boundary(attachments).unwrap();
}

#[test]
fn attachment_schema_rejects_invented_and_misclassified_fields() {
    let rows = manifest_rows();
    let (_dir, worker) = worker();
    let conn = worker.read_connection().unwrap();
    let links = rows
        .iter()
        .find(|row| row.get("table") == Some("attachment_links"))
        .unwrap();

    let mut invented = rows
        .iter()
        .find(|row| row.get("table") == Some("attachments"))
        .unwrap()
        .clone();
    invented.fields.insert(
        "canonical_fields".to_owned(),
        "id,plaintext_size,mime_type,original_filename,created_at,size".to_owned(),
    );
    let error = validate_attachment_schema(&conn, &invented, links).unwrap_err();
    assert!(error.contains("size"), "{error}");

    let mut local_as_canonical = rows
        .iter()
        .find(|row| row.get("table") == Some("attachments"))
        .unwrap()
        .clone();
    local_as_canonical.fields.insert(
        "canonical_fields".to_owned(),
        "id,plaintext_size,mime_type,original_filename,created_at,storage_id".to_owned(),
    );
    let error = validate_attachment_boundary(&local_as_canonical).unwrap_err();
    assert!(error.contains("storage_id"), "{error}");
}

#[test]
fn attachment_schema_rejects_a_new_unclassified_column() {
    let rows = manifest_rows();
    let (_dir, worker) = worker();
    let conn = worker.read_connection().unwrap();
    conn.execute_batch("ALTER TABLE attachments ADD COLUMN future_materialization_state TEXT;")
        .unwrap();
    let attachments = rows
        .iter()
        .find(|row| row.get("table") == Some("attachments"))
        .unwrap();
    let links = rows
        .iter()
        .find(|row| row.get("table") == Some("attachment_links"))
        .unwrap();
    let error = validate_attachment_schema(&conn, attachments, links).unwrap_err();
    assert!(error.contains("future_materialization_state"), "{error}");
}

#[test]
fn every_class4_row_records_the_pending_disposition() {
    let rows = manifest_rows();
    let class4: Vec<_> = rows
        .iter()
        .filter(|row| row.get("class") == Some("4"))
        .collect();
    assert!(class4.len() >= 9);
    for row in class4 {
        let note = row.required("note").unwrap();
        assert!(
            note.contains("ADR 0074"),
            "{}",
            row.required("table").unwrap()
        );
        assert!(
            note.contains("SYNC-2"),
            "{}",
            row.required("table").unwrap()
        );
    }
}

#[test]
fn starting_assignment_rows_are_not_silently_dropped() {
    let rows = manifest_rows();
    let declared: BTreeSet<_> = rows.iter().filter_map(|row| row.get("table")).collect();
    for expected in [
        // Class 1 canonical ledger and user intent.
        "ledger_transactions",
        "ledger_postings",
        "accounts",
        "categories",
        "tags",
        "transaction_tags",
        "split_lines",
        "split_line_tags",
        "recurring_events",
        "recurring_event_instances",
        "recurring_event_tags",
        "recurring_transfers",
        "bill_contracts",
        "confirmed_obligations",
        "income_sources",
        "debt_terms",
        "credit_card_statements",
        "credit_card_cycles",
        "transaction_reviews",
        "change_journal_entries",
        "transaction_details",
        // Class 1b import and attachment artifacts.
        "source_batches",
        "source_records",
        "parser_runs",
        "source_provenance_links",
        "attachment_blobs",
        // Class 2 projections.
        "money_inbox_read_model",
        "commitments",
        "forecast_actuals",
        "forecast_diffs",
        "forecast_dirty_ranges",
        "forecast_quality_scores",
        "forecast_backtest_results",
        "transaction_display_rows_read_model",
        "recurring_suggestion_suppressions",
        "model_registry",
        "risk_flags",
        // Class 3 local state.
        "connector_connections",
        "staged_accounts",
        "staged_balances",
        "staged_transactions",
        "audit_events",
        "schema_migrations",
        "vault_metadata",
        "settings",
        "durable_jobs",
        "backup_history",
        // Class 4 candidates.
        "balance_observations",
        "scenarios",
        "forecast_assumption_events",
        "forecast_dependency_edges",
        "dedupe_decisions",
        "merchant_aliases",
        "merchant_identities",
        "manual_entry_links",
        "attachments",
        "attachment_links",
        "transaction_categorizations",
    ] {
        assert!(declared.contains(expected), "manifest omitted {expected}");
    }
}
