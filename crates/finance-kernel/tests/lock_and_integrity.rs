//! Lock checkpoints the WAL and closes the connection; unlock can run an integrity check first
//! (personal-cfo-7ie.14). Both are generic, host-invoked engine behavior: no platform appears
//! here (ADR 0085).

use std::fs;
use std::path::{Path, PathBuf};

use finance_kernel::{
    Account, AccountFlags, AccountId, ActorType, CashflowRole, CommandEnvelope, CommandMeta,
    CreateAccount, Currency, Kernel, KernelError, LedgerAccountId, UnlockOptions,
};
use tempfile::TempDir;
use uuid::Uuid;

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

fn add_account(kernel: &Kernel, name: &str) -> AccountId {
    let account = Account::new(
        AccountId::new(),
        LedgerAccountId::new(),
        name,
        CashflowRole::LiquidCash,
        Currency::Usd,
        AccountFlags::default(),
    );
    let id = account.id();
    kernel
        .dispatch(CommandEnvelope::new(meta(), CreateAccount::new(account)))
        .unwrap();
    id
}

fn side(path: &Path, suffix: &str) -> PathBuf {
    let mut os = path.as_os_str().to_owned();
    os.push(suffix);
    PathBuf::from(os)
}

fn wal_len(path: &Path) -> u64 {
    fs::metadata(side(path, "-wal")).map_or(0, |m| m.len())
}

/// A vault with a few accounts written, still unlocked.
fn written_vault(dir: &TempDir) -> (PathBuf, Kernel) {
    let path = dir.path().join("vault.db");
    let kernel = Kernel::create_vault(&path, PASSWORD).unwrap();
    for n in 0..5 {
        add_account(&kernel, &format!("Account {n}"));
    }
    (path, kernel)
}

#[test]
fn lock_folds_the_wal_into_the_main_file_and_leaves_no_wal_content() {
    let dir = TempDir::new().unwrap();
    let (path, kernel) = written_vault(&dir);
    // While unlocked, the write-ahead log holds committed frames not yet in the main file.
    assert!(wal_len(&path) > 0, "the test must start with a live -wal");

    kernel.lock_checked().expect("a clean lock checkpoints");
    assert_eq!(wal_len(&path), 0, "no -wal content after lock");

    // The vault opens from the main file alone: copy ONLY vault.db and the envelope sidecar to a
    // fresh directory (what a system backup that ignores or misses side files would capture).
    let copy_dir = TempDir::new().unwrap();
    let copy = copy_dir.path().join("vault.db");
    fs::copy(&path, &copy).unwrap();
    for entry in fs::read_dir(dir.path()).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("vault.db")
            && name != "vault.db"
            && !name.ends_with("-wal")
            && !name.ends_with("-shm")
        {
            fs::copy(entry.path(), copy_dir.path().join(&name)).unwrap();
        }
    }
    let reopened = Kernel::unlock_vault(&copy, PASSWORD).expect("opens from the main file alone");
    assert_eq!(
        reopened.account_count().unwrap(),
        5,
        "every committed write is in the main file"
    );
}

#[test]
fn the_plain_lock_checkpoints_too() {
    let dir = TempDir::new().unwrap();
    let (path, kernel) = written_vault(&dir);
    assert!(wal_len(&path) > 0);
    kernel.lock();
    assert_eq!(wal_len(&path), 0);
}

#[test]
fn an_integrity_check_on_a_sound_vault_unlocks_it() {
    let dir = TempDir::new().unwrap();
    let (path, kernel) = written_vault(&dir);
    kernel.lock();
    let reopened =
        Kernel::unlock_vault_with_options(&path, PASSWORD, UnlockOptions::with_integrity_check())
            .expect("a sound vault passes the check");
    assert_eq!(reopened.account_count().unwrap(), 5);
}

#[test]
fn a_damaged_vault_fails_the_integrity_check_and_nothing_is_written() {
    let dir = TempDir::new().unwrap();
    let (path, kernel) = written_vault(&dir);
    kernel.lock();

    // Damage a page in the middle of the file (not the header, which would read as a wrong key).
    let mut bytes = fs::read(&path).unwrap();
    assert!(
        bytes.len() > 4096 * 4,
        "the vault has several pages ({})",
        bytes.len()
    );
    for byte in &mut bytes[4096 * 3 + 64..4096 * 3 + 192] {
        *byte ^= 0xA5;
    }
    fs::write(&path, &bytes).unwrap();
    let damaged = fs::read(&path).unwrap();

    match Kernel::unlock_vault_with_options(&path, PASSWORD, UnlockOptions::with_integrity_check())
    {
        Err(KernelError::IntegrityCheckFailed { problems }) => assert!(problems >= 1),
        Err(other) => panic!("expected IntegrityCheckFailed, got {other:?}"),
        Ok(_) => panic!("a damaged vault must not unlock when the check is asked for"),
    }
    // Nothing was opened for writing, migrated or stamped: the file is byte-identical and no
    // side file was left behind.
    assert_eq!(
        fs::read(&path).unwrap(),
        damaged,
        "the vault file was not written"
    );
    assert!(!side(&path, "-wal").exists() && !side(&path, "-journal").exists());
}

#[test]
fn a_wrong_password_is_still_a_wrong_password_with_the_check_on() {
    let dir = TempDir::new().unwrap();
    let (path, kernel) = written_vault(&dir);
    kernel.lock();
    match Kernel::unlock_vault_with_options(
        &path,
        b"not the password",
        UnlockOptions::with_integrity_check(),
    ) {
        Err(KernelError::VaultUnlockFailed) => {}
        other => panic!("expected VaultUnlockFailed, got {:?}", other.err()),
    }
}
