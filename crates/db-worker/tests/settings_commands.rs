//! `SetUserSetting` and the device setter (personal-cfo-5ymg8.7): what each key accepts, that a
//! clear works, that the row is stamped from the command and not the clock, that an identical value
//! writes nothing, and that no other raw writer of the settings table exists. The rejections are
//! also in `apply_rejections.rs`; the readers' answers are pinned by `settings_parity.rs`.

mod common;

use common::*;
use db_worker::*;
use std::collections::BTreeMap;
use std::path::Path;

fn set(worker: &DbWorker, key: &str, value: &str) -> Result<Outcome, DbError> {
    worker.dispatch(
        meta(),
        WriteCommand::SetUserSetting {
            key: key.to_owned(),
            value: value.to_owned(),
        },
    )
}

fn rejected(result: Result<Outcome, DbError>) -> bool {
    matches!(
        result,
        Err(DbError::Rejected {
            code: RejectCode::PayloadInvalid,
            ..
        })
    )
}

#[test]
fn each_key_accepts_the_forms_its_writer_produces_and_nothing_else() {
    let (_dir, worker) = worker();
    let cases: &[(&str, &[&str], &[&str])] = &[
        (
            MINIMUM_CASH_FLOOR_KEY,
            &["0", "250000", "9223372036854775807"],
            &[
                "",
                "-1",
                "1.5",
                "1e3",
                " 5",
                "+5",
                "9223372036854775808",
                "abc",
            ],
        ),
        (
            COMFORT_BAND_UPPER_KEY,
            &["", "0", "1000000"],
            &["-1", " ", "1.5", "x"],
        ),
        (
            REPORTING_CURRENCY_KEY,
            &["USD", "EUR"],
            &["", "usd", "XYZ", "USD "],
        ),
        (
            AUTO_CATEGORIZE_ON_IMPORT_KEY,
            &["true", "false"],
            &["", "TRUE", "yes", "1"],
        ),
        (
            FUTURE_CASH_SERIES_KEY,
            &["[]", r#"["total","checking"]"#],
            &["", "null", r#"{"a":1}"#, "[1,2]", r#"["a""#],
        ),
    ];
    for (key, good, bad) in cases {
        for value in *good {
            set(&worker, key, value).unwrap_or_else(|e| panic!("{key} = {value:?}: {e:?}"));
        }
        for value in *bad {
            assert!(
                rejected(set(&worker, key, value)),
                "{key} = {value:?} must be refused"
            );
        }
    }
    // A value over the size bound is refused even when its form is right.
    let big = format!("[\"{}\"]", "a".repeat(MAX_USER_SETTING_VALUE_BYTES));
    assert!(rejected(set(&worker, FUTURE_CASH_SERIES_KEY, &big)));
}

#[test]
fn a_key_outside_the_registry_is_refused_whatever_its_scope() {
    let (_dir, worker) = worker();
    for key in [
        "user.theme",
        "device.window",
        "minimum_cash_floor_minor",
        "locale",
        "user.locale",
        "",
    ] {
        assert!(rejected(set(&worker, key, "1")), "{key:?}");
    }
}

/// Clearing the comfort band's upper edge stores the empty string, which `comfort_band` reads as no
/// edge (03a M1): set, clear, read none, all through the command.
#[test]
fn the_upper_edge_can_be_set_and_cleared_through_the_command() {
    let (_dir, worker) = worker();
    set(&worker, COMFORT_BAND_UPPER_KEY, "1000000").unwrap();
    assert_eq!(
        worker.comfort_band().unwrap().upper,
        Some(core_money::Money::new(
            1_000_000,
            worker.comfort_band().unwrap().currency
        ))
    );
    set(&worker, COMFORT_BAND_UPPER_KEY, "").unwrap();
    assert_eq!(worker.comfort_band().unwrap().upper, None);
    assert_eq!(
        worker
            .get_setting(COMFORT_BAND_UPPER_KEY)
            .unwrap()
            .as_deref(),
        Some(""),
        "the cleared form is stored verbatim"
    );
}

/// The row is stamped with the command's `issued_at`, never the wall clock, so two devices applying
/// the same command write the same row.
#[test]
fn the_row_is_stamped_from_the_command_not_the_clock() {
    let (_dir, worker) = worker();
    let mut m = meta();
    m.issued_at = IssuedAt::from_unix_micros(1_000_000_000_000_000);
    worker
        .dispatch(
            m,
            WriteCommand::SetUserSetting {
                key: MINIMUM_CASH_FLOOR_KEY.to_owned(),
                value: "5".to_owned(),
            },
        )
        .unwrap();
    let updated: String = worker
        .read_connection()
        .unwrap()
        .query_row(
            "SELECT updated_at FROM settings WHERE key = ?1",
            [MINIMUM_CASH_FLOOR_KEY],
            |r| r.get(0),
        )
        .unwrap();
    assert!(updated.starts_with("2001-09-09"), "{updated}");
}

#[test]
fn an_identical_value_writes_nothing_and_a_different_one_does() {
    let (_dir, worker) = worker();
    set(&worker, MINIMUM_CASH_FLOOR_KEY, "100").unwrap();
    assert_eq!(
        worker.last_write_set().unwrap().rows.len(),
        1,
        "a first value is a write"
    );
    set(&worker, MINIMUM_CASH_FLOOR_KEY, "100").unwrap();
    assert!(
        worker.last_write_set().unwrap().rows.is_empty(),
        "the same value is a no-op"
    );
    set(&worker, MINIMUM_CASH_FLOOR_KEY, "200").unwrap();
    assert_eq!(
        worker.last_write_set().unwrap().rows.len(),
        1,
        "a different value is a write"
    );
}

#[test]
fn the_two_setters_keep_to_their_own_scope() {
    let (_dir, worker) = worker();
    // The household setter refuses a device key (it is not in the registry) and the device setter
    // refuses a household key.
    assert!(worker.set_user_setting("device.window", "1").is_err());
    for key in [MINIMUM_CASH_FLOOR_KEY, "user.theme", "locale", ""] {
        assert!(
            matches!(
                worker.set_device_setting(key, "1"),
                Err(DbError::SettingScope)
            ),
            "{key:?}"
        );
    }
    worker.set_device_setting("device.window", "1").unwrap();
    assert_eq!(
        worker.get_setting("device.window").unwrap().as_deref(),
        Some("1")
    );
    // A device key is not part of any command's write set.
    assert!(worker.last_write_set().is_none_or(|s| s.rows.is_empty()));
}

/// The production part of a source file: everything before an inline test module WITH a body (a
/// `#[cfg(test)] mod tests;` declaration hides nothing).
fn production_part(source: &str) -> &str {
    let mut from = 0;
    while let Some(at) = source[from..].find("#[cfg(test)]\nmod ") {
        let start = from + at;
        let rest = &source[start..];
        let line_end = rest.find('\n').map_or(rest.len(), |i| i + 1);
        let decl_end = rest[line_end..].find(['{', ';']).map(|i| line_end + i);
        if decl_end.is_some_and(|i| rest.as_bytes()[i] == b'{') {
            return &source[..start];
        }
        from = start + line_end;
    }
    source
}

/// The only statements that write the `settings` table in production source: the command's apply
/// (household) and the device setter. A new raw writer would bypass the bus for a class-1 row.
#[test]
fn the_settings_table_has_exactly_two_production_writers() {
    fn files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n == "tests") {
                    continue;
                }
                files(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut all = Vec::new();
    files(&src, &mut all);
    let mut found: BTreeMap<String, usize> = BTreeMap::new();
    for path in all {
        let name = path
            .strip_prefix(&src)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        if name == "write_set_tests.rs" || name == "migrations.rs" {
            continue;
        }
        let source = std::fs::read_to_string(&path).unwrap();
        // Production code only: everything before an inline test module.
        let production = production_part(&source);
        let count = production
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .map(|l| {
                l.matches("INSERT INTO settings").count()
                    + l.matches("UPDATE settings").count()
                    + l.matches("DELETE FROM settings").count()
            })
            .sum::<usize>();
        if count > 0 {
            found.insert(name, count);
        }
    }
    let expected: BTreeMap<String, usize> = [
        ("apply/settings_cmds.rs".to_owned(), 1),
        ("lib.rs".to_owned(), 1),
    ]
    .into();
    assert_eq!(
        found, expected,
        "a settings writer was added or removed outside the command and the device setter"
    );
}
