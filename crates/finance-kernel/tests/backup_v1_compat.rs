//! Frozen format-v1 compatibility artifact captured from the pre-v2 kernel.
//! The password is deliberately synthetic; the fixture contains an empty test
//! vault and no personal financial data.

use finance_kernel::backup::{disassemble, BackupError};
use finance_kernel::Kernel;

const PASSWORD: &[u8] = b"checked-in v1 compatibility fixture password";
const V1_FIXTURE: &[u8] = include_bytes!("fixtures/backup-v1-minimal.pcfobk");

#[test]
fn checked_in_v1_fixture_restores_into_a_fresh_vault() {
    let parsed = disassemble(PASSWORD, V1_FIXTURE).expect("parse historical v1 fixture");
    assert_eq!(parsed.manifest.format_version, 1);
    assert_eq!(parsed.manifest.manifest_schema_version, None);
    assert!(
        parsed.manifest.schema_version < finance_kernel::CURRENT_SCHEMA_VERSION,
        "the frozen fixture must exercise an actual older-schema upgrade"
    );

    let directory = tempfile::tempdir().unwrap();
    let package = directory.path().join("historical-v1.pcfobk");
    let destination = directory.path().join("restored-vault.db");
    std::fs::write(&package, V1_FIXTURE).unwrap();
    let restored = Kernel::restore_backup(&package, PASSWORD, &destination)
        .expect("restore historical format-v1 package");
    assert_eq!(restored.account_count().unwrap(), 0);
    assert_eq!(
        restored.vault_metadata().unwrap().schema_version,
        finance_kernel::CURRENT_SCHEMA_VERSION
    );
    assert_eq!(
        restored.vault_metadata().unwrap().household_timezone,
        "UTC",
        "the SQLCipher database was decrypted and opened from the v1 fixture"
    );
    assert!(destination.exists());
    assert!(directory.path().join("restored-vault.db.envelope").exists());
}

#[test]
fn newer_backup_schema_is_refused_before_installing_any_vault_data() {
    use finance_kernel::backup::{assemble, BackupInputs};
    use vault_crypto::{derive_kek, unwrap_dek, VaultEnvelope};
    let parsed = disassemble(PASSWORD, V1_FIXTURE).unwrap();
    let envelope = VaultEnvelope::from_bytes(&parsed.envelope_bytes).unwrap();
    let kek = derive_kek(PASSWORD, &envelope.salt, &envelope.kdf).unwrap();
    let dek = unwrap_dek(&kek, &envelope.wrapped).unwrap();
    let package = assemble(
        &dek,
        &BackupInputs {
            db_bytes: parsed.db_bytes,
            envelope_bytes: parsed.envelope_bytes,
            blobs: parsed.blobs,
            app_version: "synthetic-future".into(),
            schema_version: finance_kernel::CURRENT_SCHEMA_VERSION + 1,
            vault_envelope_version: parsed.manifest.vault_envelope_version,
            created_at: "2026-09-27T00:00:00Z".into(),
            backup_id: uuid::Uuid::now_v7(),
        },
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("newer.pcfobk");
    let destination = dir.path().join("refused").join("vault.db");
    std::fs::write(&source, package).unwrap();
    assert!(
        matches!(Kernel::restore_backup(&source, PASSWORD, &destination), Err(finance_kernel::KernelError::Vault(message)) if message.contains("backup is from a newer app"))
    );
    assert!(!destination.parent().unwrap().exists());
}

#[test]
fn v1_kdf_admission_rejects_unsupported_parameters_before_argon2() {
    let mut package = V1_FIXTURE.to_vec();
    // v1 header: magic (6), version (2), algorithm (1), then Argon2 memory.
    package[9..13].copy_from_slice(&19_455u32.to_be_bytes());
    assert!(matches!(
        disassemble(PASSWORD, &package),
        Err(BackupError::UnsupportedKdfParameters)
    ));
}
