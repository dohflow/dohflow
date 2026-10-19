//! The bounds recorded for `settings.key` and `credit_card_cycles.cycle_close` in
//! `table_columns.toml` (vault-format.md C.1.5; personal-cfo-t2s4b part 5b). Both are
//! replicated text primary keys that SQLite does not bound, so each is bounded in code: the
//! settings writer rejects a long key with a typed error, and the cycle date is a formatted
//! `NaiveDate`, whose length is bounded by construction.

mod common;

use chrono::NaiveDate;
use common::worker;
use db_worker::{
    DbError, AUTO_CATEGORIZE_ON_IMPORT_KEY, COMFORT_BAND_UPPER_KEY, FUTURE_CASH_SERIES_KEY,
    LOCALE_KEY, MAX_SETTING_KEY_BYTES, MINIMUM_CASH_FLOOR_KEY, REPORTING_CURRENCY_KEY,
};
use sync_core::row_digest::{primary_key, Kind, Value};

const MANIFEST: &str = include_str!("../table_columns.toml");

fn manifest_bound(table: &str, column: &str) -> usize {
    let block = MANIFEST
        .split("[[key_bound]]")
        .skip(1)
        .find(|b| {
            b.contains(&format!("table = \"{table}\""))
                && b.contains(&format!("column = \"{column}\""))
        })
        .unwrap_or_else(|| panic!("no key_bound for {table}.{column}"));
    block
        .lines()
        .find_map(|l| l.strip_prefix("max_bytes = "))
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

#[test]
fn the_manifest_records_the_bounds_the_code_enforces() {
    assert_eq!(manifest_bound("settings", "key"), MAX_SETTING_KEY_BYTES);
    assert_eq!(
        manifest_bound("merchant_aliases", "normalized_key"),
        db_worker::MAX_ALIAS_KEY_BYTES
    );
    assert_eq!(manifest_bound("credit_card_cycles", "cycle_close"), 16);
}

#[test]
fn every_known_settings_key_fits_the_bound() {
    for key in [
        LOCALE_KEY,
        REPORTING_CURRENCY_KEY,
        MINIMUM_CASH_FLOOR_KEY,
        COMFORT_BAND_UPPER_KEY,
        AUTO_CATEGORIZE_ON_IMPORT_KEY,
        FUTURE_CASH_SERIES_KEY,
    ] {
        assert!(key.len() <= MAX_SETTING_KEY_BYTES, "{key}");
    }
}

#[test]
fn set_setting_rejects_a_key_over_the_bound_and_never_truncates() {
    let (_dir, worker) = worker();
    let fits = format!("user.{}", "k".repeat(MAX_SETTING_KEY_BYTES - 5));
    assert_eq!(fits.len(), MAX_SETTING_KEY_BYTES);
    worker.set_setting(&fits, "v").unwrap();
    let over = format!("user.{}", "k".repeat(MAX_SETTING_KEY_BYTES - 4));
    let err = worker.set_setting(&over, "v").unwrap_err();
    assert!(
        matches!(err, DbError::ReplicatedKeyTooLong { table: "settings" }),
        "{err:?}"
    );
    assert!(!err.to_string().contains(&over));
    // Nothing was stored for the long key, and the widest settings key still encodes as a key.
    let conn = worker.read_connection().unwrap();
    let stored: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM settings WHERE key LIKE 'user.kkkk%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stored, 1);
    assert!(primary_key(&[(Kind::Text, Value::Text(fits.as_bytes()))]).is_ok());
}

#[test]
fn a_cycle_close_date_is_short_for_every_possible_date() {
    // The apply path stores `NaiveDate::to_string()`; the widest forms are the extreme years.
    for date in [
        NaiveDate::MIN,
        NaiveDate::MAX,
        NaiveDate::from_ymd_opt(2026, 10, 18).unwrap(),
    ] {
        let text = date.to_string();
        assert!(text.len() <= 16, "{text}");
        assert!(primary_key(&[
            (Kind::Uuid, Value::Blob(&[0; 16])),
            (Kind::Text, Value::Text(text.as_bytes()))
        ])
        .is_ok());
    }
    assert_eq!(
        NaiveDate::from_ymd_opt(2026, 10, 18)
            .unwrap()
            .to_string()
            .len(),
        10
    );
}
