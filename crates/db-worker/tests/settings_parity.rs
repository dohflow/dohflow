//! What the five household settings do, pinned to a golden file written from the code as it was
//! before `user.*` settings moved onto the command bus and their keys were renamed
//! (personal-cfo-5ymg8.7). Rows are dumped by LABEL, never by stored key name: the point is that no
//! reader's answer changes, whatever the key is called. Regenerate with `SETTINGS_PARITY_UPDATE=1`
//! only when a change is MEANT to move these values.

mod common;

use common::{create_role_account, KEY};
use core_ledger::{AccountId, CashflowRole};
use db_worker::*;
use std::fmt::Write as _;

const GOLDEN_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/settings_parity.golden.txt"
);
const GOLDEN: &str = include_str!("settings_parity.golden.txt");

/// (label, the key's constant). The constants are the only way this test names a key.
const PREFERENCES: &[(&str, &str)] = &[
    ("reporting currency", REPORTING_CURRENCY_KEY),
    ("minimum cash floor", MINIMUM_CASH_FLOOR_KEY),
    ("comfort band upper", COMFORT_BAND_UPPER_KEY),
    ("auto-categorize on import", AUTO_CATEGORIZE_ON_IMPORT_KEY),
    ("future cash series", FUTURE_CASH_SERIES_KEY),
];

/// The two places this test writes a setting, so a change of entry point edits only these.
fn set_household_preference(worker: &DbWorker, key: &str, value: &str) {
    worker.set_user_setting(key, value).unwrap();
}
fn set_device_preference(worker: &DbWorker, name: &str, value: &str) {
    worker
        .set_device_setting(&format!("device.{name}"), value)
        .unwrap();
}
fn get_device_preference(worker: &DbWorker, name: &str) -> Option<String> {
    worker.get_setting(&format!("device.{name}")).unwrap()
}

fn dump(worker: &DbWorker, heading: &str) -> String {
    let mut out = String::new();
    writeln!(out, "[{heading}]").unwrap();
    for (label, key) in PREFERENCES {
        writeln!(out, "{label}: {:?}", worker.get_setting(key).unwrap()).unwrap();
    }
    let band = worker.comfort_band().unwrap();
    writeln!(
        out,
        "comfort band: {} lower {} upper {:?}",
        band.currency.code(),
        band.lower.minor_units(),
        band.upper.map(|m| m.minor_units())
    )
    .unwrap();
    let availability = worker.cash_availability().unwrap();
    writeln!(
        out,
        "cash availability: floor {} below_floor {}",
        availability.floor.minor_units(),
        availability.below_floor
    )
    .unwrap();
    out
}

#[test]
fn every_household_setting_answers_what_it_answered_before_it_moved_onto_the_bus() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.vault");
    let mut text = String::new();
    {
        let worker = DbWorker::open(&path, KEY).unwrap();
        create_role_account(
            &worker,
            AccountId::new(),
            "Checking",
            CashflowRole::LiquidCash,
        );

        text.push_str(&dump(&worker, "nothing set"));

        for (label, key, value) in [
            ("reporting currency", REPORTING_CURRENCY_KEY, "EUR"),
            ("minimum cash floor", MINIMUM_CASH_FLOOR_KEY, "250000"),
            ("comfort band upper", COMFORT_BAND_UPPER_KEY, "900000"),
            (
                "auto-categorize on import",
                AUTO_CATEGORIZE_ON_IMPORT_KEY,
                "false",
            ),
            (
                "future cash series",
                FUTURE_CASH_SERIES_KEY,
                r#"["total","checking"]"#,
            ),
        ] {
            assert!(PREFERENCES.iter().any(|(l, k)| *l == label && *k == key));
            set_household_preference(&worker, key, value);
        }
        set_device_preference(&worker, "window", "1");
        text.push_str(&dump(&worker, "all five set"));
        writeln!(
            text,
            "device window: {:?}",
            get_device_preference(&worker, "window")
        )
        .unwrap();

        // A later write replaces the value.
        set_household_preference(&worker, MINIMUM_CASH_FLOOR_KEY, "300000");
        text.push_str(&dump(&worker, "floor changed"));
    }

    // Persisted: a fresh open of the same file answers the same.
    let reopened = DbWorker::open(&path, KEY).unwrap();
    text.push_str(&dump(&reopened, "after reopening"));
    writeln!(
        text,
        "device window: {:?}",
        get_device_preference(&reopened, "window")
    )
    .unwrap();

    // The fixture must exercise what it claims to.
    assert!(text.contains("minimum cash floor: Some(\"300000\")"));
    assert!(text.contains("comfort band: EUR lower 300000 upper Some(900000)"));
    assert!(text.contains("minimum cash floor: None"));

    if std::env::var_os("SETTINGS_PARITY_UPDATE").is_some() {
        std::fs::write(GOLDEN_PATH, &text).unwrap();
        return;
    }
    assert_eq!(
        text, GOLDEN,
        "a setting answered differently; if that is intended, regenerate with \
         SETTINGS_PARITY_UPDATE=1 and review the diff"
    );
}
