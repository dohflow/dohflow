//! Installing a snapshot into a new vault (`crate::snapshot_install`, personal-cfo-1cltt.5 slice A):
//! a golden origin round-trips hash-equal, keeps the origin's ids, seeds nothing, and a snapshot
//! with any defect installs nothing.

use rusqlite::Connection;
use sync_core::snapshot::{build, verify, Header, SnapshotError, TableFile, Verified};

use super::{liquid_subtype_cmd, meta, seed_golden_vault, worker};
use crate::logical_export::{export_genesis_plaintext, ExportInput};
use crate::migrations::{apply, MIGRATIONS};
use crate::snapshot_install::{install_snapshot, InstallError};
use crate::{AccountId, AccountSubtype, DbWorker};

/// An origin vault with data in every kind of replicated table the golden seed reaches, and an
/// account opened with a balance (so a ledger transaction and its postings exist).
fn origin() -> (tempfile::TempDir, DbWorker) {
    let (dir, worker) = worker();
    seed_golden_vault(&worker);
    worker
        .dispatch(
            meta(),
            liquid_subtype_cmd(
                AccountId::new(),
                "Opened with a balance",
                Some(AccountSubtype::Checking),
                12_345,
            ),
        )
        .unwrap();
    (dir, worker)
}

fn export(worker: &DbWorker) -> Vec<u8> {
    export_genesis_plaintext(&worker.read_connection().unwrap(), &ExportInput::default()).unwrap()
}

fn vault_id(worker: &DbWorker) -> [u8; 16] {
    *worker.vault_id().unwrap().as_bytes()
}

/// A new, migrated, unseeded vault: what the non-seeding create path hands the installer.
fn replica() -> Connection {
    let mut conn = Connection::open_in_memory().unwrap();
    conn.pragma_update(None, "foreign_keys", true).unwrap();
    apply(&mut conn, MIGRATIONS).unwrap();
    conn
}

fn count(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

fn unhex(text: &str) -> [u8; 32] {
    let mut out = [0_u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).unwrap();
    }
    out
}

/// The table files of `verified`, to be changed and framed again.
fn files_of(verified: &Verified) -> Vec<TableFile> {
    verified
        .manifest
        .tables
        .iter()
        .zip(&verified.files)
        .map(|(entry, (_, content))| TableFile {
            name: entry.name.clone(),
            columns: entry.columns.clone(),
            row_count: entry.row_count.parse().unwrap(),
            state_digest: entry.state_digest.as_deref().map(unhex),
            content: content.clone(),
        })
        .collect()
}

fn reframe(verified: &Verified, files: Vec<TableFile>) -> Verified {
    let m = &verified.manifest;
    let plaintext = build(
        &Header {
            schema_version: m.schema_version.parse().unwrap(),
            vault_created_at: &m.vault_created_at,
            inventory_version: &m.inventory_version,
            voids: &[],
            artifacts: &[],
        },
        files,
    )
    .unwrap();
    verify(&plaintext).unwrap()
}

/// Nothing of the snapshot is in the vault.
fn assert_untouched(conn: &Connection) {
    assert_eq!(count(conn, "vault_metadata"), 0, "no identity");
    for table in [
        "categories",
        "accounts",
        "ledger_transactions",
        "ledger_postings",
        "households",
    ] {
        assert_eq!(count(conn, table), 0, "{table}");
    }
}

#[test]
fn a_golden_origin_installs_hash_equal_with_its_own_identity_and_no_seed() {
    let (_dir, origin) = origin();
    let plaintext = export(&origin);
    let verified = verify(&plaintext).unwrap();
    let mut conn = replica();

    let installed = install_snapshot(&mut conn, vault_id(&origin), &verified).unwrap();
    assert!(installed.rows > 0 && installed.tables == verified.manifest.tables.len());

    // The origin's identity, from the snapshot.
    let (id, created_at): (Vec<u8>, String) = conn
        .query_row(
            "SELECT vault_id, created_at FROM vault_metadata WHERE singleton = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(id, vault_id(&origin));
    assert_eq!(created_at, verified.manifest.vault_created_at);

    // Hash-equal: the replica exports to the same table files, row counts and state digests.
    let again = export_genesis_plaintext(&conn, &ExportInput::default()).unwrap();
    let again = verify(&again).unwrap();
    assert_eq!(again.manifest.tables, verified.manifest.tables);
    assert_eq!(again.files, verified.files);

    // Nothing was seeded on top: every table has exactly the origin's rows.
    for table in [
        "categories",
        "households",
        "household_members",
        "accounts",
        "ledger_transactions",
        "ledger_postings",
        "merchant_identities",
    ] {
        assert_eq!(
            count(&conn, table),
            count(&origin.read_connection().unwrap(), table),
            "{table}"
        );
    }
    let violations: i64 = conn
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(violations, 0);
}

#[test]
fn a_legacy_origin_keeps_its_ids_and_the_replica_derives_none() {
    let (_dir, origin) = origin();
    // A legacy vault holds ids no seed would derive: change one seeded category's id.
    let legacy: Vec<u8> = vec![0x5a; 16];
    let (seeded, name): (Vec<u8>, String) = origin
        .read_connection()
        .unwrap()
        .query_row(
            "SELECT id, name FROM categories ORDER BY id LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    origin
        .read_connection()
        .unwrap()
        .execute(
            "UPDATE categories SET id = ?1 WHERE id = ?2",
            rusqlite::params![legacy, seeded],
        )
        .unwrap();
    let verified = verify(&export(&origin)).unwrap();
    let mut conn = replica();
    install_snapshot(&mut conn, vault_id(&origin), &verified).unwrap();

    let found: String = conn
        .query_row(
            "SELECT name FROM categories WHERE id = ?1",
            [&legacy],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(found, name, "the origin's own id, not a derived one");
    let derived: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM categories WHERE id = ?1",
            [&seeded],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(derived, 0, "nothing re-derives the seed's id");
    assert_eq!(
        count(&conn, "categories"),
        count(&origin.read_connection().unwrap(), "categories"),
        "zero seed rows on top of the snapshot's"
    );
}

#[test]
fn a_vault_that_is_not_new_or_not_at_the_snapshots_schema_installs_nothing() {
    let (_dir, origin) = origin();
    let verified = verify(&export(&origin)).unwrap();

    // It already has an identity.
    let mut conn = replica();
    crate::insert_vault_metadata(
        &conn,
        &uuid::Uuid::now_v7(),
        crate::migrations::CURRENT_VERSION,
        "t",
    )
    .unwrap();
    assert!(matches!(
        install_snapshot(&mut conn, vault_id(&origin), &verified),
        Err(InstallError::HasIdentity)
    ));
    assert_eq!(count(&conn, "categories"), 0);

    // A table the snapshot installs already holds a row.
    let mut conn = replica();
    conn.execute(
        "INSERT INTO tags (id, name, created_at) VALUES (zeroblob(16), 'x', 't')",
        [],
    )
    .unwrap();
    assert!(matches!(
        install_snapshot(&mut conn, vault_id(&origin), &verified),
        Err(InstallError::NotEmpty { ref table }) if table == "tags"
    ));
    assert_eq!(count(&conn, "vault_metadata"), 0, "no identity was written");
    assert_eq!(count(&conn, "categories"), 0);

    // A snapshot of another schema version, older or newer, is never installed here.
    for other in [
        crate::migrations::CURRENT_VERSION + 1,
        crate::migrations::CURRENT_VERSION - 1,
    ] {
        let mut changed = verify(&export(&origin)).unwrap();
        let files = files_of(&changed);
        let m = &changed.manifest;
        let plaintext = build(
            &Header {
                schema_version: u64::try_from(other).unwrap(),
                vault_created_at: &m.vault_created_at,
                inventory_version: &m.inventory_version,
                voids: &[],
                artifacts: &[],
            },
            files,
        )
        .unwrap();
        changed = verify(&plaintext).unwrap();
        let mut conn = replica();
        assert!(matches!(
            install_snapshot(&mut conn, vault_id(&origin), &changed),
            Err(InstallError::SchemaMismatch { .. })
        ));
        assert_untouched(&conn);
    }
}

#[test]
fn a_snapshot_with_a_defect_installs_nothing() {
    let (_dir, origin) = origin();
    let verified = verify(&export(&origin)).unwrap();
    let id = vault_id(&origin);

    // A row changed after its state digest was computed: the install re-exports and refuses.
    let mut files = files_of(&verified);
    let accounts = files.iter_mut().find(|f| f.name == "accounts").unwrap();
    let text = String::from_utf8(accounts.content.clone()).unwrap();
    accounts.content = text.replacen("Checking", "Chequing", 1).into_bytes();
    let mut conn = replica();
    assert!(matches!(
        install_snapshot(&mut conn, id, &reframe(&verified, files)),
        Err(InstallError::DigestMismatch { ref table }) if table == "accounts"
    ));
    assert_untouched(&conn);

    // A line that is not the canonical encoding of its columns.
    let mut files = files_of(&verified);
    let categories = files.iter_mut().find(|f| f.name == "categories").unwrap();
    categories.content = String::from_utf8(categories.content.clone())
        .unwrap()
        .replacen("\":\"", "\": \"", 1)
        .into_bytes();
    let mut conn = replica();
    assert!(matches!(
        install_snapshot(&mut conn, id, &reframe(&verified, files)),
        Err(InstallError::Snapshot(SnapshotError::MalformedRow))
    ));
    assert_untouched(&conn);

    // Columns that are not this build's: reordered.
    let mut files = files_of(&verified);
    files
        .iter_mut()
        .find(|f| f.name == "tags")
        .unwrap()
        .columns
        .reverse();
    let mut conn = replica();
    assert!(matches!(
        install_snapshot(&mut conn, id, &reframe(&verified, files)),
        Err(InstallError::ColumnsMismatch { ref table }) if table == "tags"
    ));
    assert_untouched(&conn);

    // A table this build does not know, and a table it needs that is missing.
    let mut files = files_of(&verified);
    files.push(TableFile {
        name: "zz_unknown".to_owned(),
        columns: vec![sync_core::snapshot::ColumnEntry {
            name: "id".to_owned(),
            kind: "uuid".to_owned(),
        }],
        row_count: 0,
        state_digest: Some([0; 32]),
        content: Vec::new(),
    });
    let mut conn = replica();
    let result = install_snapshot(&mut conn, id, &reframe(&verified, files));
    assert!(
        matches!(result, Err(InstallError::UnknownTable { .. })),
        "{result:?}"
    );
    let mut files = files_of(&verified);
    files.retain(|f| f.name != "tags");
    let mut conn = replica();
    assert!(matches!(
        install_snapshot(&mut conn, id, &reframe(&verified, files)),
        Err(InstallError::MissingTable { ref table }) if table == "tags"
    ));
    assert_untouched(&conn);
}

#[test]
fn a_snapshot_that_needs_artifacts_installs_nothing() {
    // An attachment row (its bytes are an artifact) needs a raw-key vault to import.
    let dir = tempfile::tempdir().unwrap();
    let origin = DbWorker::open_with_raw_key(
        dir.path().join("vault.db"),
        vault_crypto::generate_dek().unwrap(),
    )
    .unwrap();
    origin
        .import_attachment(b"an attachment", None, None)
        .unwrap();
    let verified = verify(&export(&origin)).unwrap();
    let mut conn = replica();
    assert!(matches!(
        install_snapshot(&mut conn, vault_id(&origin), &verified),
        Err(InstallError::NeedsArtifacts)
    ));
    assert_untouched(&conn);

    // A non-empty closure.
    let (_dir, origin) = super::worker();
    let closure = [[7_u8; 32]];
    let plaintext = export_genesis_plaintext(
        &origin.read_connection().unwrap(),
        &ExportInput {
            artifacts: &closure,
            ..ExportInput::default()
        },
    )
    .unwrap();
    let mut conn = replica();
    assert!(matches!(
        install_snapshot(&mut conn, vault_id(&origin), &verify(&plaintext).unwrap()),
        Err(InstallError::NeedsArtifacts)
    ));
    assert_untouched(&conn);
}

#[test]
fn an_installed_header_that_is_not_the_snapshots_installs_nothing() {
    // A vault whose identity row is changed as it is written (a trigger stands in for any defect
    // in the install's identity write) fails the proof on the header, not only on the tables.
    for (column, value) in [
        ("created_at", "'1999-01-01T00:00:00+00:00'"),
        ("schema_version", "1"),
    ] {
        let (_dir, origin) = origin();
        let verified = verify(&export(&origin)).unwrap();
        let mut conn = replica();
        conn.execute_batch(&format!(
            "CREATE TEMP TRIGGER skew AFTER INSERT ON vault_metadata
             BEGIN UPDATE vault_metadata SET {column} = {value}; END;"
        ))
        .unwrap();
        let result = install_snapshot(&mut conn, vault_id(&origin), &verified);
        assert!(
            matches!(result, Err(InstallError::HeaderMismatch)),
            "{column}: {result:?}"
        );
        assert_untouched(&conn);
    }
}
