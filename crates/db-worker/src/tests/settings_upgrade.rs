//! A vault written by an earlier build, with the unprefixed household settings, is migrated once on
//! open (personal-cfo-5ymg8.7, owner decision D-ST1: an eager rename, no dual-read), and every
//! reader then answers from the `user.*` keys. A vault already renamed is refused by a build that
//! does not know its schema (`health_accessors.rs` and `raw_key_vault.rs` pin that refusal for any
//! future version; this pins the rename's own schema number).

use super::*;

const LEGACY: [(&str, &str); 5] = [
    ("reporting_currency", "EUR"),
    ("minimum_cash_floor_minor", "250000"),
    ("comfort_band_upper_minor", "900000"),
    ("auto_categorize_on_import", "false"),
    ("future_cash_series_selection", r#"["total","checking"]"#),
];

fn old_vault(path: &Path) {
    let worker = DbWorker::open(path, KEY).unwrap();
    {
        let mut guard = worker.lock();
        migrations::migrate_down(&mut guard.conn, 63).unwrap();
        for (key, value) in LEGACY {
            guard
                .conn
                .execute(
                    "INSERT INTO settings (key, value, value_schema_version, updated_at)
                     VALUES (?1, ?2, 1, 'earlier')",
                    [key, value],
                )
                .unwrap();
        }
    }
}

#[test]
fn a_vault_with_the_unprefixed_keys_is_migrated_on_open_and_every_reader_answers() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("old.vault");
    old_vault(&path);

    let worker = DbWorker::open(&path, KEY).unwrap();
    assert_eq!(
        worker
            .get_setting(REPORTING_CURRENCY_KEY)
            .unwrap()
            .as_deref(),
        Some("EUR")
    );
    assert_eq!(
        worker
            .get_setting(MINIMUM_CASH_FLOOR_KEY)
            .unwrap()
            .as_deref(),
        Some("250000")
    );
    assert_eq!(
        worker
            .get_setting(AUTO_CATEGORIZE_ON_IMPORT_KEY)
            .unwrap()
            .as_deref(),
        Some("false")
    );
    assert_eq!(
        worker
            .get_setting(FUTURE_CASH_SERIES_KEY)
            .unwrap()
            .as_deref(),
        Some(r#"["total","checking"]"#)
    );
    let band = worker.comfort_band().unwrap();
    assert_eq!(band.currency, Currency::Eur);
    assert_eq!(band.lower, Money::new(250_000, Currency::Eur));
    assert_eq!(band.upper, Some(Money::new(900_000, Currency::Eur)));

    // No unprefixed household row is left behind, and the rename is not part of any command's
    // write set (migrations run before the capture is installed).
    let conn = worker.read_connection().unwrap();
    let legacy_left: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM settings WHERE key NOT LIKE 'user.%' AND key NOT LIKE 'device.%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(legacy_left, 0);
    worker
        .set_user_setting(MINIMUM_CASH_FLOOR_KEY, "250000")
        .unwrap();
    let next = worker.last_write_set().unwrap();
    assert!(
        next.rows.is_empty(),
        "the migrated row is already the value, so the next command writes nothing"
    );
}

/// A write after the upgrade goes to the prefixed key and never recreates an unprefixed one.
#[test]
fn a_write_after_the_upgrade_uses_the_prefixed_key() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("old.vault");
    old_vault(&path);
    let worker = DbWorker::open(&path, KEY).unwrap();
    worker
        .set_user_setting(MINIMUM_CASH_FLOOR_KEY, "300000")
        .unwrap();
    let keys: Vec<String> = worker
        .read_connection()
        .unwrap()
        .prepare("SELECT key FROM settings ORDER BY key")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(keys.iter().all(|k| k.starts_with("user.")), "{keys:?}");
    assert_eq!(keys.len(), 5);
}

/// The upgraded vault carries a schema stamp of at least 64 and records the migration, so a build
/// that only supports 63 refuses it (`observed > supported` is `NewerSchema`; `health_accessors.rs`
/// and `raw_key_vault.rs` pin that refusal itself) instead of reading keys it no longer finds.
#[test]
fn an_upgraded_vault_is_stamped_so_an_older_build_refuses_it() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("old.vault");
    old_vault(&path);
    let worker = DbWorker::open(&path, KEY).unwrap();
    assert!(worker.schema_version() >= 64);
    let recorded: Option<String> = worker
        .read_connection()
        .unwrap()
        .query_row(
            "SELECT name FROM schema_migrations WHERE version = 64",
            [],
            |r| r.get(0),
        )
        .ok();
    assert_eq!(recorded.as_deref(), Some("household_settings_user_prefix"));
}
