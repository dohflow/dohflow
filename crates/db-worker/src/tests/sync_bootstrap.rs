//! The birth of a replica from a snapshot (`crate::sync_bootstrap`, personal-cfo-1cltt.5 slice B):
//! the replica becomes the vault only when its install, ledger, cursor and base are all on disk, a
//! crash at any step leaves no vault and a clean second attempt, and a snapshot that is not a valid
//! root, or a newer schema, writes nothing.

use std::path::Path;
use std::sync::Arc;

use sync_core::snapshot::{build, verify, Header, RecordedVoid, TableFile, Verified};
use sync_core::status::EngineVersion;
use vault_crypto::Dek;

use super::{liquid_subtype_cmd, meta, seed_golden_vault, worker};
use crate::base_image::{base_image_path, BaseImageStatus};
use crate::logical_export::{export_genesis_plaintext, ExportInput};
use crate::migrations::CURRENT_VERSION;
use crate::sync_bootstrap::{
    bootstrap_replica, bootstrapping_path, cut_after, BootstrapError, ReplicaSource, Step,
    SyncPosition,
};
use crate::{AccountId, AccountSubtype, DbWorker};

/// An origin with data, and the snapshot it would publish: its verified plaintext.
fn origin() -> (tempfile::TempDir, DbWorker, Verified) {
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
    let verified = verify(
        &export_genesis_plaintext(&worker.read_connection().unwrap(), &ExportInput::default())
            .unwrap(),
    )
    .unwrap();
    (dir, worker, verified)
}

fn unhex(text: &str) -> [u8; 32] {
    let mut out = [0_u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).unwrap();
    }
    out
}

/// The same tables framed again with `voids` recorded (and the schema version `schema`).
fn with_voids(verified: &Verified, voids: &[RecordedVoid], schema: u64) -> Verified {
    let m = &verified.manifest;
    let files = m
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
        .collect();
    verify(
        &build(
            &Header {
                schema_version: schema,
                vault_created_at: &m.vault_created_at,
                inventory_version: &m.inventory_version,
                voids,
                artifacts: &[],
            },
            files,
        )
        .unwrap(),
    )
    .unwrap()
}

fn void(id: u8, first: u64, last: u64) -> RecordedVoid {
    RecordedVoid {
        void_id: [id; 32],
        first,
        last,
    }
}

fn engine() -> EngineVersion {
    EngineVersion::new("dohflow-engine-1").unwrap()
}

fn source<'a>(
    origin: &DbWorker,
    snapshot: &'a Verified,
    root: u64,
    version: &'a EngineVersion,
    own: &'a EngineVersion,
    held: &'a [(u64, u64)],
) -> ReplicaSource<'a> {
    ReplicaSource {
        vault_id: *origin.vault_id().unwrap().as_bytes(),
        root,
        snapshot,
        engine_version: version,
        own_engine_version: own,
        held_voids: held,
    }
}

fn dek() -> Arc<Dek> {
    Arc::new(vault_crypto::generate_dek().unwrap())
}

fn states(worker: &DbWorker) -> (Vec<sync_core::snapshot::TableEntry>, Vec<(String, Vec<u8>)>) {
    let exported = verify(
        &export_genesis_plaintext(&worker.read_connection().unwrap(), &ExportInput::default())
            .unwrap(),
    )
    .unwrap();
    (exported.manifest.tables, exported.files)
}

/// What a clean bootstrap leaves, asserted in one place: the replica is the origin (hash-equal,
/// identity included), its base is `Ready` at the root, and the ledger and the cursor are the
/// snapshot's.
fn assert_born(replica: &DbWorker, origin: &DbWorker, snapshot: &Verified, root: u64, path: &Path) {
    assert_eq!(replica.vault_id().unwrap(), origin.vault_id().unwrap());
    let (tables, files) = states(replica);
    assert_eq!(
        tables, snapshot.manifest.tables,
        "hash-equal to the snapshot"
    );
    assert_eq!(files, snapshot.files);
    match replica.base_image_status().unwrap() {
        BaseImageStatus::Ready { base_seq, .. } => {
            assert_eq!(base_seq, i64::try_from(root).unwrap());
        }
        other => panic!("the base is not Ready: {other:?}"),
    }
    assert!(base_image_path(path).exists());
    assert_eq!(
        replica.sync_position().unwrap(),
        Some(SyncPosition {
            snapshot_root: Some(root),
            next_to_apply: root + 1
        })
    );
    let recorded = snapshot.manifest.recorded_voids().unwrap();
    let ledger = replica.void_ledger().unwrap();
    assert!(
        ledger.honored().is_empty(),
        "nothing is honored by a new replica"
    );
    assert_eq!(ledger.seeded().len(), recorded.len());
    for (seeded, from) in ledger.seeded().iter().zip(&recorded) {
        assert_eq!(
            (seeded.void_id, seeded.first, seeded.last),
            (from.void_id, from.first, from.last)
        );
    }
}

fn no_stray_files(path: &Path) {
    assert!(!bootstrapping_path(path).exists());
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut name = bootstrapping_path(path).into_os_string();
        name.push(suffix);
        assert!(!Path::new(&name).exists(), "{suffix}");
    }
}

#[test]
fn a_replica_becomes_the_vault_with_the_origins_identity_a_ready_base_and_a_seeded_ledger() {
    let (_origin_dir, origin, verified) = origin();
    let snapshot = with_voids(
        &verified,
        &[void(1, 2, 3), void(2, 5, 6)],
        CURRENT_VERSION.unsigned_abs(),
    );
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("vault.db");
    let (version, own) = (engine(), engine());
    let born = bootstrap_replica(
        &path,
        &dek(),
        &source(&origin, &snapshot, 7, &version, &own, &[]),
    )
    .unwrap();
    assert_eq!(born.voids_seeded, 2);
    assert!(born.installed.rows > 0);
    assert_born(&born.worker, &origin, &snapshot, 7, &path);
    no_stray_files(&path);

    // Every table holds only the origin's rows: nothing was seeded on top.
    let conn = born.worker.read_connection().unwrap();
    let (replica_categories, origin_categories): (i64, i64) = (
        conn.query_row("SELECT COUNT(*) FROM categories", [], |r| r.get(0))
            .unwrap(),
        origin
            .read_connection()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM categories", [], |r| r.get(0))
            .unwrap(),
    );
    assert_eq!(replica_categories, origin_categories);
    let ledger_transactions: i64 = conn
        .query_row("SELECT COUNT(*) FROM ledger_transactions", [], |r| r.get(0))
        .unwrap();
    assert!(ledger_transactions > 0);
    // The schema stamp is this build's.
    let stamp: i64 = conn
        .query_row("SELECT schema_version FROM vault_metadata", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(stamp, CURRENT_VERSION);
}

#[test]
fn the_replica_opens_again_as_an_ordinary_vault_and_is_never_replaced() {
    let (_origin_dir, origin, verified) = origin();
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("vault.db");
    let (version, own) = (engine(), engine());
    let dek = dek();
    let source = source(&origin, &verified, 4, &version, &own, &[]);
    drop(bootstrap_replica(&path, &dek, &source).unwrap());

    // A second attempt over the vault that now exists changes nothing.
    let before = std::fs::read(&path).unwrap();
    assert!(matches!(
        bootstrap_replica(&path, &dek, &source),
        Err(BootstrapError::VaultExists)
    ));
    assert_eq!(std::fs::read(&path).unwrap(), before);
    no_stray_files(&path);

    // An ordinary open: the base is still Ready and no seed was added.
    let dek = Arc::try_unwrap(dek).ok().unwrap();
    let reopened = DbWorker::open_existing_with_raw_key(&path, dek).unwrap();
    assert_born(&reopened, &origin, &verified, 4, &path);
}

#[test]
fn a_snapshot_that_is_not_a_valid_root_writes_nothing() {
    let (_origin_dir, origin, verified) = origin();
    let (version, own) = (engine(), engine());
    let other = EngineVersion::new("dohflow-engine-2").unwrap();
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("vault.db");

    let attempt = |snapshot: &Verified, root: u64, version: &EngineVersion, held: &[(u64, u64)]| {
        let result = bootstrap_replica(
            &path,
            &dek(),
            &source(&origin, snapshot, root, version, &own, held),
        );
        assert!(
            matches!(result, Err(BootstrapError::NotAValidRoot)),
            "{result:?}"
        );
        assert!(!path.exists(), "no vault");
        no_stray_files(&path);
    };
    // Another engine version.
    attempt(&verified, 7, &other, &[]);
    // A root inside a void range the replica holds, edges included.
    attempt(&verified, 12, &version, &[(10, 19)]);
    attempt(&verified, 10, &version, &[(10, 19)]);
    attempt(&verified, 19, &version, &[(10, 19)]);
    // A recorded void at the root, past it, overlapping another, or running backwards.
    let schema = CURRENT_VERSION.unsigned_abs();
    attempt(
        &with_voids(&verified, &[void(1, 5, 7)], schema),
        7,
        &version,
        &[],
    );
    attempt(
        &with_voids(&verified, &[void(1, 5, 9)], schema),
        7,
        &version,
        &[],
    );
}

#[test]
fn a_snapshot_of_a_newer_schema_is_update_required_and_writes_nothing() {
    let (_origin_dir, origin, verified) = origin();
    let newer = with_voids(&verified, &[], CURRENT_VERSION.unsigned_abs() + 1);
    let (version, own) = (engine(), engine());
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("vault.db");
    let result = bootstrap_replica(
        &path,
        &dek(),
        &source(&origin, &newer, 7, &version, &own, &[]),
    );
    assert!(
        matches!(result, Err(BootstrapError::UpdateRequired { snapshot, build })
            if snapshot == CURRENT_VERSION.unsigned_abs() + 1 && build == CURRENT_VERSION),
        "{result:?}"
    );
    assert!(!path.exists());
    no_stray_files(&path);
}

#[test]
fn a_snapshot_of_an_older_schema_installs_through_the_migrations() {
    // A snapshot exported at the previous schema: the replica is migrated to that version first,
    // the snapshot is installed, and the migrations after it run over the installed rows.
    let (_origin_dir, origin, verified) = origin();
    let older_schema = CURRENT_VERSION - 1;
    let older = with_voids(&verified, &[], older_schema.unsigned_abs());
    assert_eq!(older.manifest.schema_version, older_schema.to_string());
    let (version, own) = (engine(), engine());
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("vault.db");
    let born = bootstrap_replica(
        &path,
        &dek(),
        &source(&origin, &older, 3, &version, &own, &[]),
    )
    .unwrap();
    let conn = born.worker.read_connection().unwrap();
    let version_now: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version_now, CURRENT_VERSION);
    let migrated: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM schema_migrations WHERE version = ?1",
            [CURRENT_VERSION],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(migrated, 1, "the migrations after the snapshot's ran");
    // The class-1 rows are the snapshot's, byte for byte, at the older version's digests.
    assert_eq!(born.worker.vault_id().unwrap(), origin.vault_id().unwrap());
    assert_eq!(
        born.worker.sync_position().unwrap(),
        Some(SyncPosition {
            snapshot_root: Some(3),
            next_to_apply: 4
        })
    );
}

#[test]
fn a_crash_at_any_step_leaves_no_vault_and_a_clean_second_attempt_ends_ready() {
    let (_origin_dir, origin, verified) = origin();
    let snapshot = with_voids(&verified, &[void(1, 2, 3)], CURRENT_VERSION.unsigned_abs());
    let (version, own) = (engine(), engine());
    let src = source(&origin, &snapshot, 9, &version, &own, &[]);
    for step in [
        Step::Created,
        Step::Installed,
        Step::Ledger,
        Step::Opened,
        Step::Derived,
        Step::BaseImage,
        Step::Synced,
    ] {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("vault.db");
        let dek = dek();
        let cut = cut_after(step, || bootstrap_replica(&path, &dek, &src));
        assert!(
            matches!(cut, Err(BootstrapError::Cut(at)) if at == step),
            "{step:?}: {cut:?}"
        );
        // B1: no vault exists to take a command, so none can be taken unrecorded between the
        // install and the base. Only the temporary file is left.
        assert!(!path.exists(), "{step:?}");
        assert!(bootstrapping_path(&path).exists(), "{step:?}");
        assert!(
            crate::DbWorker::open_existing_with_raw_key(
                &path,
                vault_crypto::generate_dek().unwrap()
            )
            .is_err(),
            "{step:?}: nothing to open"
        );

        // The next start discards the temporary replica and ends in the same state as a clean run:
        // installed once (hash-equal, no row twice), with the ledger and the cursor written once.
        let born = bootstrap_replica(&path, &dek, &src).unwrap();
        assert_born(&born.worker, &origin, &snapshot, 9, &path);
        no_stray_files(&path);
    }
}

#[test]
fn a_crash_after_the_rename_leaves_a_complete_vault() {
    let (_origin_dir, origin, verified) = origin();
    let snapshot = with_voids(&verified, &[void(1, 2, 3)], CURRENT_VERSION.unsigned_abs());
    let (version, own) = (engine(), engine());
    let src = source(&origin, &snapshot, 9, &version, &own, &[]);
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("vault.db");
    let dek = dek();
    let cut = cut_after(Step::Renamed, || bootstrap_replica(&path, &dek, &src));
    assert!(
        matches!(cut, Err(BootstrapError::Cut(Step::Renamed))),
        "{cut:?}"
    );
    no_stray_files(&path);
    let reopened =
        DbWorker::open_existing_with_raw_key(&path, Arc::try_unwrap(dek).ok().unwrap()).unwrap();
    assert_born(&reopened, &origin, &snapshot, 9, &path);
}

#[test]
fn only_the_exact_temporary_name_is_discarded() {
    let (_origin_dir, origin, verified) = origin();
    let (version, own) = (engine(), engine());
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("vault.db");
    // A stale temporary replica, its journal files, and neighbors that are not its.
    std::fs::write(bootstrapping_path(&path), b"half a vault").unwrap();
    let mut wal = bootstrapping_path(&path).into_os_string();
    wal.push("-wal");
    std::fs::write(&wal, b"journal").unwrap();
    let neighbors = [
        dir.path().join("vault.db.bak"),
        dir.path().join("vault.db.bootstrapping.keep"),
        dir.path().join("other.db.bootstrapping"),
        dir.path().join("notes.txt"),
    ];
    for neighbor in &neighbors {
        std::fs::write(neighbor, b"mine").unwrap();
    }
    let born = bootstrap_replica(
        &path,
        &dek(),
        &source(&origin, &verified, 2, &version, &own, &[]),
    )
    .unwrap();
    assert_born(&born.worker, &origin, &verified, 2, &path);
    no_stray_files(&path);
    for neighbor in &neighbors {
        assert_eq!(std::fs::read(neighbor).unwrap(), b"mine", "{neighbor:?}");
    }
}

#[test]
fn a_replica_with_no_identity_is_never_given_one_by_the_open() {
    // The replica open does not seed: a database that holds no identity is refused, not created.
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("vault.db");
    let dek = dek();
    let conn = crate::open_keyed_raw(&path, &dek).unwrap();
    let lease = crate::VaultLease::acquire(&path).unwrap();
    let result = DbWorker::bootstrap_replica_vault(
        path.clone(),
        conn,
        crate::KeyMaterial::Raw(Arc::clone(&dek)),
        lease.file,
    );
    assert!(result.is_err());
}

#[test]
fn one_void_id_can_hold_more_than_one_entry() {
    // Q1: a snapshot that records one id at two ranges (the verifier does not refuse it) installs,
    // and each range is its own seeded entry.
    let (_origin_dir, origin, verified) = origin();
    let schema = CURRENT_VERSION.unsigned_abs();
    let snapshot = with_voids(&verified, &[void(1, 2, 3), void(1, 5, 6)], schema);
    let (version, own) = (engine(), engine());
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("vault.db");
    let born = bootstrap_replica(
        &path,
        &dek(),
        &source(&origin, &snapshot, 9, &version, &own, &[]),
    )
    .unwrap();
    assert_born(&born.worker, &origin, &snapshot, 9, &path);
    assert_eq!(born.worker.void_ledger().unwrap().seeded().len(), 2);

    // Q2: the same id honored here, past the root (the seeded entry is history, never a fork
    // check's input): a second entry for the id, and the next snapshot records all three.
    born.worker
        .lock()
        .conn
        .execute(
            "INSERT INTO sync_voids (ordinal, void_id, first, last, origin)
             VALUES (3, ?1, 12, 13, 'honored')",
            [vec![1_u8; 32]],
        )
        .unwrap();
    let ledger = born.worker.void_ledger().unwrap();
    assert_eq!((ledger.seeded().len(), ledger.honored().len()), (2, 1));
    let recorded = ledger.recorded();
    assert_eq!(recorded.len(), 3);
    assert!(recorded.iter().all(|v| v.void_id == [1; 32]));
}

#[test]
fn an_abandoned_bootstrap_leaves_no_base_image_behind() {
    // Q3: cut after the base capture, then cut again right after the next attempt discards what
    // the first left: no base.db belongs to a vault that does not exist.
    let (_origin_dir, origin, verified) = origin();
    let (version, own) = (engine(), engine());
    let src = source(&origin, &verified, 9, &version, &own, &[]);
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("vault.db");
    let dek = dek();
    let cut = cut_after(Step::BaseImage, || bootstrap_replica(&path, &dek, &src));
    assert!(matches!(cut, Err(BootstrapError::Cut(Step::BaseImage))));
    assert!(base_image_path(&path).exists(), "the capture reached disk");
    let cut = cut_after(Step::Created, || bootstrap_replica(&path, &dek, &src));
    assert!(matches!(cut, Err(BootstrapError::Cut(Step::Created))));
    assert!(
        !base_image_path(&path).exists(),
        "discarded with the temporary replica"
    );
}

#[test]
fn a_base_image_that_is_not_the_abandoned_bootstraps_own_is_never_removed() {
    let (_origin_dir, origin, verified) = origin();
    let (version, own) = (engine(), engine());
    let src = source(&origin, &verified, 9, &version, &own, &[]);

    // An existing vault: a stale temporary file beside it changes nothing, and its base survives.
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("vault.db");
    let dek = dek();
    drop(bootstrap_replica(&path, &dek, &src).unwrap());
    let base_before = std::fs::read(base_image_path(&path)).unwrap();
    std::fs::write(bootstrapping_path(&path), b"stale").unwrap();
    assert!(matches!(
        bootstrap_replica(&path, &dek, &src),
        Err(BootstrapError::VaultExists)
    ));
    assert_eq!(std::fs::read(base_image_path(&path)).unwrap(), base_before);
    assert_eq!(std::fs::read(bootstrapping_path(&path)).unwrap(), b"stale");

    // No vault and no stale temporary file at all: a base image beside the path is left alone.
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("vault.db");
    std::fs::create_dir_all(base_image_path(&path).parent().unwrap()).unwrap();
    std::fs::write(base_image_path(&path), b"no temporary file").unwrap();
    let cut = cut_after(Step::Created, || bootstrap_replica(&path, &dek, &src));
    assert!(matches!(cut, Err(BootstrapError::Cut(Step::Created))));
    assert_eq!(
        std::fs::read(base_image_path(&path)).unwrap(),
        b"no temporary file"
    );

    // No vault, a stale temporary file that records no base of this image: the base is left alone
    // (the next capture replaces it), while the temporary file goes.
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("vault.db");
    std::fs::create_dir_all(base_image_path(&path).parent().unwrap()).unwrap();
    std::fs::write(base_image_path(&path), b"somebody else's base").unwrap();
    std::fs::write(bootstrapping_path(&path), b"not even a database").unwrap();
    let cut = cut_after(Step::Created, || bootstrap_replica(&path, &dek, &src));
    assert!(matches!(cut, Err(BootstrapError::Cut(Step::Created))));
    assert_eq!(
        std::fs::read(base_image_path(&path)).unwrap(),
        b"somebody else's base"
    );

    // No vault, a stale temporary replica that did capture a base, but the file on disk is no
    // longer that base (changed since): also left alone.
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("vault.db");
    let cut = cut_after(Step::BaseImage, || bootstrap_replica(&path, &dek, &src));
    assert!(matches!(cut, Err(BootstrapError::Cut(Step::BaseImage))));
    std::fs::write(base_image_path(&path), b"changed since").unwrap();
    let cut = cut_after(Step::Created, || bootstrap_replica(&path, &dek, &src));
    assert!(matches!(cut, Err(BootstrapError::Cut(Step::Created))));
    assert_eq!(
        std::fs::read(base_image_path(&path)).unwrap(),
        b"changed since"
    );
}
