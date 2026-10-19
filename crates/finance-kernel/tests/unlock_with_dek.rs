//! `unlock_vault_with_dek` (personal-cfo-7ie.10; ADR 0002 decision 6): open a vault from a DEK
//! released by the platform's biometric store.
//!
//! The released plaintext is checked against the vault's envelope before SQLCipher is keyed
//! for writing, every refusal is its own typed error, a wrong key is never a corrupt-vault
//! state, a password change or a key rotation retires every older item, and the DEK reaches
//! no log.

use std::io::{self, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

use finance_kernel::{
    Account, AccountFlags, AccountId, ActorType, CashflowRole, CommandEnvelope, CommandMeta,
    CreateAccount, Currency, Kernel, KernelError, LedgerAccountId, ReleasedUnlockError,
    VaultController,
};
use observability::RedactingMakeWriter;
use tempfile::TempDir;
use tracing_subscriber::fmt::MakeWriter;
use uuid::Uuid;
use vault_crypto::{
    derive_kek, unwrap_dek, ReleasedUnlockMaterial, SecretBytes, VaultEnvelope, RELEASED_LEN,
};

const PASSWORD: &[u8] = b"correct horse battery staple";

fn meta() -> CommandMeta {
    CommandMeta {
        command_id: Uuid::now_v7(),
        correlation_id: Uuid::now_v7(),
        causation_id: None,
        actor_type: ActorType::User,
        actor_id: "tester".to_owned(),
        idempotency_key: Uuid::new_v4().to_string(),
        issued_at: finance_kernel::IssuedAt::now(),
    }
}

fn account_named(name: &str) -> Account {
    Account::new(
        AccountId::new(),
        LedgerAccountId::new(),
        name,
        CashflowRole::LiquidCash,
        Currency::Usd,
        AccountFlags::default(),
    )
}

fn sidecar(db: &Path) -> std::path::PathBuf {
    let mut os = db.as_os_str().to_owned();
    os.push(".envelope");
    os.into()
}

/// What the platform layer does at enrollment, after a successful password unlock: read the
/// envelope, recover the DEK, and seal `version | vault_id | fingerprint | dek`.
fn enroll(db: &Path, password: &[u8]) -> SecretBytes<RELEASED_LEN> {
    let envelope = VaultEnvelope::from_bytes(&std::fs::read(sidecar(db)).unwrap()).unwrap();
    let kek = derive_kek(password, &envelope.salt, &envelope.kdf).unwrap();
    let dek = unwrap_dek(&kek, &envelope.wrapped).unwrap();
    ReleasedUnlockMaterial::encode(vault_id_of(db, password), &envelope.wrapped, &dek)
}

/// The vault's `vault_id`, read through a password unlock.
fn vault_id_of(db: &Path, password: &[u8]) -> [u8; 16] {
    let kernel = Kernel::unlock_vault(db, password).unwrap();
    let id = kernel.vault_id().unwrap();
    kernel.lock();
    *id.as_bytes()
}

/// A copy of the released bytes with `edit` applied (`SecretBytes` is not `Clone`).
fn tampered(
    base: &SecretBytes<RELEASED_LEN>,
    edit: impl FnOnce(&mut [u8; RELEASED_LEN]),
) -> SecretBytes<RELEASED_LEN> {
    let mut bytes = *base.expose();
    edit(&mut bytes);
    SecretBytes::new(bytes)
}

fn duplicate(base: &SecretBytes<RELEASED_LEN>) -> SecretBytes<RELEASED_LEN> {
    tampered(base, |_| {})
}

fn created_vault() -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("vault.db");
    let kernel = Kernel::create_vault(&path, PASSWORD).unwrap();
    kernel
        .dispatch(CommandEnvelope::new(
            meta(),
            CreateAccount::new(account_named("Checking")),
        ))
        .unwrap();
    kernel.lock();
    (dir, path)
}

fn files(path: &Path) -> (Vec<u8>, Vec<u8>) {
    (
        std::fs::read(path).unwrap(),
        std::fs::read(sidecar(path)).unwrap(),
    )
}

fn expect_released(result: Result<Kernel, KernelError>, expected: ReleasedUnlockError) {
    match result {
        Err(KernelError::ReleasedUnlock(found)) => {
            assert_eq!(found, expected);
            assert!(found.requires_reenrollment());
        }
        Err(other) => panic!("expected {expected:?}, got {other:?}"),
        Ok(_) => panic!("the vault must not open: expected {expected:?}"),
    }
}

#[test]
fn a_released_dek_opens_the_same_vault_with_its_data() {
    let (_dir, path) = created_vault();
    let item = enroll(&path, PASSWORD);
    let kernel = Kernel::unlock_vault_with_dek(&path, item).expect("unlock with the released DEK");
    assert_eq!(kernel.account_count().unwrap(), 1);
    kernel.lock();
    // The password path still works afterwards.
    Kernel::unlock_vault(&path, PASSWORD).expect("password unlock");
}

#[test]
fn an_unknown_version_is_refused_before_anything_is_written() {
    let (_dir, path) = created_vault();
    let item = enroll(&path, PASSWORD);
    let before = files(&path);
    for version in [0_u8, 2, 255] {
        expect_released(
            Kernel::unlock_vault_with_dek(&path, tampered(&item, |b| b[0] = version)),
            ReleasedUnlockError::UnknownVersion(version),
        );
    }
    assert_eq!(
        files(&path),
        before,
        "a refused item leaves the vault untouched"
    );
}

#[test]
fn a_stale_fingerprint_is_refused_before_the_database_is_opened() {
    let (_dir, path) = created_vault();
    let item = enroll(&path, PASSWORD);
    let before = files(&path);
    for byte in [17_usize, 30, 48] {
        expect_released(
            Kernel::unlock_vault_with_dek(&path, tampered(&item, |b| b[byte] ^= 1)),
            ReleasedUnlockError::StaleFingerprint,
        );
    }
    assert_eq!(files(&path), before);
}

#[test]
fn another_vaults_id_is_refused_before_anything_is_written() {
    let (_dir, path) = created_vault();
    let item = enroll(&path, PASSWORD);
    let before = files(&path);
    expect_released(
        Kernel::unlock_vault_with_dek(&path, tampered(&item, |b| b[1] ^= 0xFF)),
        ReleasedUnlockError::WrongVault,
    );
    assert_eq!(
        files(&path),
        before,
        "the identity check precedes bootstrap"
    );
    // Nothing was held open: the vault still unlocks.
    Kernel::unlock_vault(&path, PASSWORD)
        .expect("still unlocks")
        .lock();
}

#[test]
fn a_wrong_key_is_a_typed_error_and_never_a_corrupt_vault() {
    let (_dir, path) = created_vault();
    let item = enroll(&path, PASSWORD);
    let before = files(&path);
    // Right version, vault id and fingerprint; a DEK that does not open the database.
    expect_released(
        Kernel::unlock_vault_with_dek(&path, tampered(&item, |b| b[49..].fill(0x5A))),
        ReleasedUnlockError::KeyRejected,
    );
    assert_eq!(files(&path), before);
    // The vault is intact and unlocks normally, so nothing marked it for recovery.
    let kernel = Kernel::unlock_vault(&path, PASSWORD).expect("unlocks");
    assert_eq!(kernel.account_count().unwrap(), 1);
}

#[test]
fn the_wrapped_dek_of_another_vault_does_not_fit() {
    let (_dir_a, path_a) = created_vault();
    let (_dir_b, path_b) = created_vault();
    let item_a = enroll(&path_a, PASSWORD);
    // Vault A's item against vault B: same password, different envelope.
    expect_released(
        Kernel::unlock_vault_with_dek(&path_b, item_a),
        ReleasedUnlockError::StaleFingerprint,
    );
}

#[test]
fn a_missing_vault_is_reported_as_missing() {
    let dir = TempDir::new().unwrap();
    let item = SecretBytes::new([1_u8; RELEASED_LEN]);
    match Kernel::unlock_vault_with_dek(dir.path().join("none.db"), item) {
        Err(KernelError::VaultNotFound) => {}
        other => panic!("expected VaultNotFound, got {:?}", other.err()),
    }
}

#[test]
fn a_password_change_retires_the_old_item() {
    let (_dir, path) = created_vault();
    let old_item = enroll(&path, PASSWORD);
    let mut controller = VaultController::open(&path);
    controller.unlock(PASSWORD).unwrap();
    controller
        .change_password(PASSWORD, b"a brand new password")
        .unwrap();
    controller.lock().unwrap();

    expect_released(
        Kernel::unlock_vault_with_dek(&path, old_item),
        ReleasedUnlockError::StaleFingerprint,
    );
    // Re-enrolling under the new password gives a working item.
    let fresh = enroll(&path, b"a brand new password");
    Kernel::unlock_vault_with_dek(&path, fresh)
        .expect("re-enrolled item opens")
        .lock();
}

#[test]
fn a_key_rotation_retires_the_old_item() {
    let (_dir, path) = created_vault();
    let old_item = enroll(&path, PASSWORD);
    let mut controller = VaultController::open(&path);
    controller.unlock(PASSWORD).unwrap();
    controller.rotate_key(PASSWORD).unwrap();
    controller.lock().unwrap();

    // The old DEK is no longer the vault's key, and the envelope's wrapped DEK changed.
    expect_released(
        Kernel::unlock_vault_with_dek(&path, old_item),
        ReleasedUnlockError::StaleFingerprint,
    );
    let fresh = enroll(&path, PASSWORD);
    let kernel = Kernel::unlock_vault_with_dek(&path, fresh).expect("re-enrolled item opens");
    assert_eq!(kernel.account_count().unwrap(), 1);
}

#[test]
fn a_second_unlock_is_refused_while_the_vault_is_open() {
    let (_dir, path) = created_vault();
    // Enrollment itself unlocks the vault, so both items are made before the first open.
    let (item_one, item_two) = (enroll(&path, PASSWORD), enroll(&path, PASSWORD));
    let first = Kernel::unlock_vault_with_dek(&path, item_one).unwrap();
    match Kernel::unlock_vault_with_dek(&path, item_two) {
        Err(KernelError::VaultInUse) => {}
        other => panic!("expected VaultInUse, got {:?}", other.err()),
    }
    first.lock();
}

// ---------------------------------------------------------------------------
// Redaction: the DEK reaches no log, error text or Debug output
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Write for Capture {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Capture {
    type Writer = Capture;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn the_dek_appears_in_no_log_error_or_debug_output() {
    use tracing_subscriber::prelude::*;

    let (_dir, path) = created_vault();
    let item = enroll(&path, PASSWORD);
    let dek_hex = hex(&item.expose()[49..]);
    let fingerprint_hex = hex(&item.expose()[17..49]);
    let mut rendered = String::new();

    let buf = Capture::default();
    let layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(RedactingMakeWriter::new(buf.clone()));
    let subscriber = tracing_subscriber::registry().with(layer);
    observability::testing::install_global_test_subscriber();
    tracing::subscriber::with_default(subscriber, || {
        // The success path and every refusal.
        let refusals = [
            tampered(&item, |b| b[0] = 9),
            tampered(&item, |b| b[20] ^= 1),
            tampered(&item, |b| b[1] ^= 0xFF),
            tampered(&item, |b| b[49..].fill(0x5A)),
        ];
        for candidate in refusals {
            let error = Kernel::unlock_vault_with_dek(&path, candidate)
                .err()
                .unwrap();
            rendered.push_str(&format!("{error} {error:?}\n"));
        }
        let kernel = Kernel::unlock_vault_with_dek(&path, duplicate(&item)).unwrap();
        rendered.push_str(&format!("{:?}\n", kernel.account_count()));
        kernel.lock();
        let material = ReleasedUnlockMaterial::parse(&item).unwrap();
        rendered.push_str(&format!("{material:?} {:?}\n", material.dek));
    });

    let logs = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
    for (what, needle) in [("DEK", &dek_hex), ("fingerprint", &fingerprint_hex)] {
        assert!(!logs.contains(needle.as_str()), "the {what} reached a log");
        assert!(
            !rendered.contains(needle.as_str()),
            "the {what} reached an error or Debug text"
        );
    }
    // Not even a prefix long enough to be useful.
    assert!(!logs.contains(&dek_hex[..16]) && !rendered.contains(&dek_hex[..16]));
}
