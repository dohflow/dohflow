//! `SetUserSetting` (personal-cfo-5ymg8.7; ADR 0074 Decision 5).
//!
//! A household preference is class 1: it follows the person to another device, so it is written
//! only through this command. `device.*` settings are class 3 and keep their raw setter
//! (`DbWorker::set_device_setting`).
//!
//! **The registry is closed.** A `user.*` key is one of the five below, each with a pure validator,
//! so apply is a function of the payload and the set of legal rows is total. The value is stored
//! VERBATIM: the validators accept exactly what the writers already produce.
//!
//! **Pin rule (engine version).** Once the engine version is pinned (personal-cfo-32fmp.4), any
//! change to the registry or a validator is a change of what a logged command writes, so it MUST
//! bump `engine_version`: a new key, a newly supported currency, a different size cap. Until then
//! sync is not wired into any build, so no replica holds a row this module would judge differently.

use rusqlite::{params, Connection};
use uuid::Uuid;

use crate::{CommandMeta, DbError, RejectCode};

/// The largest value of any user setting, in UTF-8 bytes.
pub const MAX_USER_SETTING_VALUE_BYTES: usize = 16 * 1024;

/// Namespace of the provenance handle of a setting (the op-log entity of the command).
const SETTING_HANDLE_NAMESPACE: Uuid = Uuid::from_bytes([
    0x75, 0x73, 0x65, 0x72, 0x73, 0x65, 0x74, 0x74, 0x69, 0x6e, 0x67, 0x4a, 0x5d, 0x2e, 0x91, 0x07,
]);

/// Whether a value is one a setting accepts.
type Validator = fn(&str) -> bool;

/// The household's choices that follow the person: `(stored key, value validator)`.
const USER_SETTINGS: &[(&str, Validator)] = &[
    (crate::MINIMUM_CASH_FLOOR_KEY, is_minor_units),
    // Clearing the upper edge stores the empty string, which `comfort_band` reads as "no edge".
    (crate::COMFORT_BAND_UPPER_KEY, is_blank_or_minor_units),
    (crate::REPORTING_CURRENCY_KEY, is_supported_currency),
    (crate::AUTO_CATEGORIZE_ON_IMPORT_KEY, is_boolean_text),
    (crate::FUTURE_CASH_SERIES_KEY, is_json_string_array),
];

/// A non-negative whole number of minor units, as the writers format it: ASCII digits only.
fn is_minor_units(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) && value.parse::<i64>().is_ok()
}

fn is_blank_or_minor_units(value: &str) -> bool {
    value.is_empty() || is_minor_units(value)
}

/// A supported currency in canonical (upper-case) form.
fn is_supported_currency(value: &str) -> bool {
    matches!(value, "USD" | "EUR")
}

fn is_boolean_text(value: &str) -> bool {
    matches!(value, "true" | "false")
}

/// The Future Cash chart's selection: a JSON array of series keys.
fn is_json_string_array(value: &str) -> bool {
    serde_json::from_str::<Vec<String>>(value).is_ok()
}

/// Applies [`WriteCommand::SetUserSetting`](crate::WriteCommand).
///
/// The row's `updated_at` is the command's `issued_at`, never the wall clock. Reads no other row.
pub(crate) fn apply_set_user_setting(
    tx: &Connection,
    meta: &CommandMeta,
    key: &str,
    value: &str,
) -> Result<Uuid, DbError> {
    let Some((_, valid)) = USER_SETTINGS.iter().find(|(known, _)| *known == key) else {
        return Err(DbError::reject(
            RejectCode::PayloadInvalid,
            "the key is not a household setting".to_owned(),
        ));
    };
    if value.len() > MAX_USER_SETTING_VALUE_BYTES || !valid(value) {
        return Err(DbError::reject(
            RejectCode::PayloadInvalid,
            "the value is not one this setting accepts".to_owned(),
        ));
    }
    tx.execute(
        "INSERT INTO settings (key, value, value_schema_version, updated_at)
         VALUES (?1, ?2, 1, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value,
             updated_at = excluded.updated_at",
        params![key, value, crate::apply::issued_at_text(meta)],
    )?;
    Ok(Uuid::new_v5(&SETTING_HANDLE_NAMESPACE, key.as_bytes()))
}
