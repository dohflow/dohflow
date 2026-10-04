//! Frozen outgoing-engine corpus. Capture BEFORE changing the engine; consumers
//! always open new temporary copies through the production password path.
use base64::{engine::general_purpose::STANDARD, Engine as _};
use finance_kernel::{
    Account, AccountFlags, AccountId, ActorType, AttachmentId, CashflowRole, CommandEnvelope,
    CommandMeta, CreateAccount, Currency, Kernel, LedgerAccountId, Money, RecordTransaction,
    TransactionId, VaultController, CURRENT_SCHEMA_VERSION, NO_RESET_WARNING_ACKNOWLEDGED,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, io::Write, path::Path};
use uuid::Uuid;

const PASSWORD: &[u8] = b"synthetic outgoing engine compatibility password";
const ATTACHMENT: &[u8] = b"%PDF-1.4\nsynthetic-engine-attachment\n%%EOF\n";
const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/sqlcipher-4.5.7.json"
);

#[derive(Serialize, Deserialize)]
struct Artifact {
    sha256: String,
    base64: String,
}

impl Artifact {
    fn from_bytes(bytes: &[u8]) -> Self {
        Self {
            sha256: format!("{:x}", Sha256::digest(bytes)),
            base64: STANDARD.encode(bytes),
        }
    }

    fn bytes(&self) -> Vec<u8> {
        let bytes = STANDARD.decode(&self.base64).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(&bytes)), self.sha256);
        bytes
    }
}

#[derive(Serialize, Deserialize)]
struct Corpus {
    provenance: Value,
    files: BTreeMap<String, Artifact>,
    settled: Value,
    committed_wal: Value,
    attachment_id: AttachmentId,
}

fn meta(index: u8) -> CommandMeta {
    CommandMeta {
        command_id: Uuid::from_bytes([index; 16]),
        correlation_id: Uuid::from_bytes([index; 16]),
        causation_id: None,
        actor_type: ActorType::User,
        actor_id: "synthetic-engine-fixture".into(),
        idempotency_key: format!("engine-fixture-{index}"),
    }
}

fn canonical(kernel: &Kernel) -> Value {
    json!({
        // Views intentionally have no serde contract. Freeze their complete
        // test-only Debug representation without changing production types.
        "accounts": format!("{:?}", kernel.account_views().unwrap()),
        "transactions": format!("{:?}", kernel.transactions(100).unwrap()),
        "operation_count": kernel.operation_count().unwrap(),
        "audit_count": kernel.audit_event_count(NO_RESET_WARNING_ACKNOWLEDGED).unwrap(),
        "metadata": format!("{:?}", kernel.vault_metadata().unwrap()),
        "projection_checksum": kernel.transaction_display_checksum().unwrap(),
    })
}

/// The schema version the outgoing corpus was frozen at. Opening it on a newer
/// build runs the supported forward migrations, so the expected state is the
/// frozen one with only `schema_version` advanced to the current version.
const CORPUS_SCHEMA_VERSION: i64 = 53;

/// The frozen canonical state carried forward to the current schema: every
/// field must still match byte-for-byte, and `schema_version` must equal
/// [`CURRENT_SCHEMA_VERSION`] (proving the forward migrations ran on the
/// outgoing engine's vault rather than being skipped).
fn at_current_schema(frozen: &Value) -> Value {
    assert!(CURRENT_SCHEMA_VERSION >= CORPUS_SCHEMA_VERSION);
    let mut expected = frozen.clone();
    let metadata = frozen["metadata"].as_str().unwrap();
    let captured = format!("schema_version: {CORPUS_SCHEMA_VERSION},");
    assert_eq!(metadata.matches(&captured).count(), 1, "{metadata}");
    expected["metadata"] = Value::String(metadata.replace(
        &captured,
        &format!("schema_version: {CURRENT_SCHEMA_VERSION},"),
    ));
    expected
}

fn corpus() -> Corpus {
    serde_json::from_slice(&fs::read(FIXTURE).expect("frozen outgoing corpus must exist")).unwrap()
}

fn install(corpus: &Corpus, root: &Path, with_wal: bool) {
    for (name, artifact) in &corpus.files {
        if name == "backup-v2.pcfobk" || (name == "vault.db-wal" && !with_wal) {
            continue;
        }
        assert!(
            name == "vault.db"
                || name == "vault.db.envelope"
                || name == "vault.db-wal"
                || (name.starts_with("blobs/") && !name.contains(".."))
        );
        let path = root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, artifact.bytes()).unwrap();
    }
}

#[test]
#[ignore = "manual outgoing-engine capture; requires G3M5_CAPTURE_PATH, never replaces a corpus"]
fn capture_outgoing_engine_corpus() {
    let output = std::env::var_os("G3M5_CAPTURE_PATH").expect("explicit output path required");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.db");
    let kernel = Kernel::create_vault(&path, PASSWORD).unwrap();
    assert_eq!(kernel.cipher_version().unwrap(), "4.5.7 community");
    assert_eq!(finance_kernel::sqlite_version(), "3.45.3");
    let id = AccountId::from_uuid(Uuid::from_bytes([1; 16]));
    kernel
        .dispatch(CommandEnvelope::new(
            meta(2),
            CreateAccount::with_opening_balance(
                Account::new(
                    id,
                    LedgerAccountId::from_uuid(Uuid::from_bytes([3; 16])),
                    "Synthetic Checking",
                    CashflowRole::LiquidCash,
                    Currency::Usd,
                    AccountFlags::default(),
                ),
                Money::new(250_000, Currency::Usd),
            ),
        ))
        .unwrap();
    let txn = TransactionId::from_uuid(Uuid::from_bytes([4; 16]));
    kernel
        .dispatch(CommandEnvelope::new(
            meta(5),
            RecordTransaction::new(
                txn,
                id,
                Money::new(-4_500, Currency::Usd),
                "2026-06-05T00:00:00Z".parse().unwrap(),
            ),
        ))
        .unwrap();
    let attachment = kernel
        .attach_document(
            txn,
            ATTACHMENT,
            Some("application/pdf"),
            Some("synthetic.pdf"),
        )
        .unwrap();
    kernel.acknowledge_no_reset_warning(&meta(6)).unwrap();
    kernel.rebuild_transaction_display().unwrap();
    kernel
        .export_unattended(
            &dir.path().join("backup-v2.pcfobk"),
            "synthetic-outgoing",
            "2026-09-28T00:00:00Z".into(),
            Uuid::from_bytes([7; 16]),
        )
        .unwrap();
    let settled = canonical(&kernel);
    let mut files = BTreeMap::new();
    for name in ["vault.db", "vault.db.envelope", "backup-v2.pcfobk"] {
        files.insert(
            name.into(),
            Artifact::from_bytes(&fs::read(dir.path().join(name)).unwrap()),
        );
    }
    for entry in fs::read_dir(dir.path().join("blobs")).unwrap() {
        let entry = entry.unwrap();
        files.insert(
            format!("blobs/{}", entry.file_name().to_str().unwrap()),
            Artifact::from_bytes(&fs::read(entry.path()).unwrap()),
        );
    }
    // Commit new canonical state AFTER the checkpoint. Capture while the
    // writer is alive, before Drop can checkpoint: a crash-style DB+WAL image.
    kernel
        .dispatch(CommandEnvelope::new(
            meta(8),
            RecordTransaction::new(
                TransactionId::from_uuid(Uuid::from_bytes([9; 16])),
                id,
                Money::new(-1_250, Currency::Usd),
                "2026-06-06T00:00:00Z".parse().unwrap(),
            ),
        ))
        .unwrap();
    kernel.rebuild_transaction_display().unwrap();
    let committed_wal = canonical(&kernel);
    assert_ne!(settled, committed_wal);
    assert_eq!(
        fs::read(&path).unwrap(),
        files["vault.db"].bytes(),
        "new commit must live only in WAL"
    );
    let wal = fs::read(dir.path().join("vault.db-wal")).unwrap();
    assert!(wal.len() > 32);
    files.insert("vault.db-wal".into(), Artifact::from_bytes(&wal));
    let corpus = Corpus {
        provenance: json!({
            "sqlcipher": "4.5.7 community", "sqlite": "3.45.3",
            "rusqlite": "0.32.1", "libsqlite3_sys": "0.30.1",
            "sqlite_source_id": "2024-04-15 13:34:05 8653b758870e6ef0c98d46b3ace27849054af85da891eb121e9aaa537f1ealt1",
            "amalgamation_sha256": "943812115199728aee17b9c0a740b692be9337b5e59693333158bf4755df18fe",
            "decision_commit": "e1bd6790ac8a3444d0a5beeecfec991f022101b6",
            "features": "bundled-sqlcipher-vendored-openssl,uuid",
            "wal_capture": "committed after backup checkpoint; before writer close; no SHM retained"
        }),
        files,
        settled,
        committed_wal,
        attachment_id: attachment.id,
    };
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .unwrap();
    file.write_all(&serde_json::to_vec_pretty(&corpus).unwrap())
        .unwrap();
    file.sync_all().unwrap();
}

#[test]
fn outgoing_settled_vault_and_attachment_open_repeatedly() {
    let corpus = corpus();
    assert_eq!(corpus.provenance["sqlite"], "3.45.3");
    let dir = tempfile::tempdir().unwrap();
    install(&corpus, dir.path(), false);
    for _ in 0..2 {
        let mut controller = VaultController::open(dir.path().join("vault.db"));
        controller.unlock(PASSWORD).unwrap();
        assert_eq!(
            canonical(controller.kernel().unwrap()),
            at_current_schema(&corpus.settled)
        );
        assert_eq!(
            controller
                .kernel()
                .unwrap()
                .read_attachment_bytes(corpus.attachment_id)
                .unwrap(),
            ATTACHMENT
        );
        assert!(controller.health_check().unwrap().is_healthy());
        controller.lock().unwrap();
    }
}

#[test]
fn outgoing_committed_wal_recovers_without_a_shared_memory_index() {
    let corpus = corpus();
    let dir = tempfile::tempdir().unwrap();
    install(&corpus, dir.path(), true);
    assert!(!dir.path().join("vault.db-shm").exists());
    let path = dir.path().join("vault.db");
    let kernel = Kernel::unlock_vault(&path, PASSWORD).unwrap();
    assert_eq!(canonical(&kernel), at_current_schema(&corpus.committed_wal));
    assert_eq!(
        kernel.read_attachment_bytes(corpus.attachment_id).unwrap(),
        ATTACHMENT
    );
    // The normal backup checkpoint must retain every recovered commit.
    let package = dir.path().join("checkpoint.pcfobk");
    kernel
        .export_unattended(
            &package,
            "synthetic-incoming",
            "2026-09-28T00:00:00Z".into(),
            Uuid::from_bytes([10; 16]),
        )
        .unwrap();
    drop(kernel);
    let reopened = Kernel::unlock_vault(&path, PASSWORD).unwrap();
    assert_eq!(
        canonical(&reopened),
        at_current_schema(&corpus.committed_wal)
    );
    let target = tempfile::tempdir().unwrap();
    let restored =
        Kernel::restore_backup(&package, PASSWORD, &target.path().join("vault.db")).unwrap();
    assert_eq!(
        canonical(&restored),
        at_current_schema(&corpus.committed_wal)
    );
}

#[test]
fn outgoing_v2_package_restores_with_original_key_and_state() {
    let corpus = corpus();
    let dir = tempfile::tempdir().unwrap();
    let package = dir.path().join("outgoing.pcfobk");
    fs::write(&package, corpus.files["backup-v2.pcfobk"].bytes()).unwrap();
    let destination = dir.path().join("restored").join("vault.db");
    assert!(matches!(
        Kernel::restore_backup(&package, b"wrong", &destination),
        Err(finance_kernel::KernelError::VaultUnlockFailed)
    ));
    assert!(!destination.exists());
    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    let restored = Kernel::restore_backup(&package, PASSWORD, &destination).unwrap();
    assert_eq!(canonical(&restored), at_current_schema(&corpus.settled));
    assert_eq!(
        restored
            .read_attachment_bytes(corpus.attachment_id)
            .unwrap(),
        ATTACHMENT
    );
    assert_eq!(
        fs::read(destination.with_extension("db.envelope")).unwrap(),
        corpus.files["vault.db.envelope"].bytes()
    );
}
