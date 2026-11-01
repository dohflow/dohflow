//! Enrolling Touch ID (personal-cfo-xuu, acceptance criteria C and H; 03b's M2): the released unlock
//! material leaves the kernel only after a password check that changes nothing, through a single-use
//! proof; and the controller unlocks from that material.

use std::marker::PhantomData;
use std::path::Path;

use finance_kernel::{
    Account, AccountFlags, AccountId, ActorType, CashflowRole, CommandEnvelope, CommandMeta,
    CreateAccount, Currency, Kernel, KernelError, LedgerAccountId, PasswordVerified,
    ReleasedUnlockError, UnlockOptions, VaultController, VaultState,
};
use tempfile::TempDir;
use uuid::Uuid;
use vault_crypto::{
    derive_kek, unwrap_dek, ReleasedUnlockMaterial, SecretBytes, VaultEnvelope, RELEASED_LEN,
};

const PASSWORD: &[u8] = b"correct horse battery staple";
const OTHER: &[u8] = b"a completely different passphrase";

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

fn sidecar(db: &Path) -> std::path::PathBuf {
    let mut os = db.as_os_str().to_owned();
    os.push(".envelope");
    os.into()
}

/// An unlocked controller on a fresh vault that holds one account.
fn unlocked() -> (TempDir, VaultController) {
    let dir = TempDir::new().unwrap();
    let mut controller = VaultController::open(dir.path().join("vault.db"));
    controller.create(PASSWORD).unwrap();
    controller
        .kernel()
        .unwrap()
        .dispatch(CommandEnvelope::new(
            meta(),
            CreateAccount::new(Account::new(
                AccountId::new(),
                LedgerAccountId::new(),
                "Checking",
                CashflowRole::LiquidCash,
                Currency::Usd,
                AccountFlags::default(),
            )),
        ))
        .unwrap();
    (dir, controller)
}

/// What an independent reader of the envelope derives: the released plaintext for `password`.
fn independent_material(
    db: &Path,
    password: &[u8],
    vault_id: [u8; 16],
) -> SecretBytes<RELEASED_LEN> {
    let envelope = VaultEnvelope::from_bytes(&std::fs::read(sidecar(db)).unwrap()).unwrap();
    let kek = derive_kek(password, &envelope.salt, &envelope.kdf).unwrap();
    let dek = unwrap_dek(&kek, &envelope.wrapped).unwrap();
    ReleasedUnlockMaterial::encode(vault_id, &envelope.wrapped, &dek)
}

#[test]
fn the_right_password_yields_exactly_the_independently_derived_material() {
    let (dir, controller) = unlocked();
    let db = dir.path().join("vault.db");
    let vault_id = *controller.kernel().unwrap().vault_id().unwrap().as_bytes();

    let material = controller.enrollment_unlock_material(PASSWORD).unwrap();
    let expected = independent_material(&db, PASSWORD, vault_id);
    assert_eq!(material.expose(), expected.expose());
}

#[test]
fn a_wrong_password_enables_nothing_and_changes_nothing() {
    let (dir, controller) = unlocked();
    let db = dir.path().join("vault.db");
    let envelope_before = std::fs::read(sidecar(&db)).unwrap();

    let error = controller.enrollment_unlock_material(OTHER).unwrap_err();
    assert!(matches!(error, KernelError::VaultUnlockFailed), "{error:?}");
    assert!(matches!(
        controller.kernel().unwrap().verify_password(OTHER),
        Err(KernelError::VaultUnlockFailed)
    ));
    // No state change: still unlocked, the envelope is byte for byte what it was, and the vault works.
    assert_eq!(controller.state(), VaultState::Unlocked);
    assert_eq!(std::fs::read(sidecar(&db)).unwrap(), envelope_before);
    assert_eq!(controller.kernel().unwrap().account_count().unwrap(), 1);
}

#[test]
fn enrollment_needs_an_unlocked_vault() {
    let (dir, mut controller) = unlocked();
    controller.lock().unwrap();
    assert!(matches!(
        controller.enrollment_unlock_material(PASSWORD),
        Err(KernelError::IllegalVaultTransition { .. })
    ));
    drop(dir);
}

#[test]
fn the_export_requires_a_proof_and_a_proof_cannot_be_copied_or_reused() {
    // The only way to call the export is with a `PasswordVerified`: this does not compile otherwise.
    let _export: fn(&Kernel, PasswordVerified) -> Result<SecretBytes<RELEASED_LEN>, KernelError> =
        Kernel::released_unlock_material;

    // The proof is neither `Clone` nor `Copy`, and the export takes it by value, so one proof is one
    // export. (Autoref probe: the inherent method exists only when the bound holds.)
    trait NotClone {
        fn is_clone(&self) -> bool {
            false
        }
    }
    impl<T> NotClone for T {}
    struct Probe<T>(PhantomData<T>);
    #[allow(dead_code)]
    impl<T: Clone> Probe<T> {
        fn is_clone(&self) -> bool {
            true
        }
    }
    assert!(!Probe::<PasswordVerified>(PhantomData).is_clone());
}

#[test]
fn a_proof_is_refused_when_the_envelope_changed_after_the_check() {
    let (_dir, mut controller) = unlocked();
    let proof = controller
        .kernel()
        .unwrap()
        .verify_password(PASSWORD)
        .unwrap();

    // The password changes between the check and the export (the DEK does not, the envelope does).
    controller.change_password(PASSWORD, OTHER).unwrap();

    let error = controller
        .kernel()
        .unwrap()
        .released_unlock_material(proof)
        .unwrap_err();
    assert!(matches!(error, KernelError::Vault(_)), "{error:?}");
}

#[test]
fn after_a_password_change_only_the_new_password_enrolls() {
    let (_dir, mut controller) = unlocked();
    controller.change_password(PASSWORD, OTHER).unwrap();

    assert!(matches!(
        controller.enrollment_unlock_material(PASSWORD),
        Err(KernelError::VaultUnlockFailed)
    ));
    assert!(controller.enrollment_unlock_material(OTHER).is_ok());
}

#[test]
fn the_controller_unlocks_from_the_material_and_the_vault_is_intact() {
    let (_dir, mut controller) = unlocked();
    let material = controller.enrollment_unlock_material(PASSWORD).unwrap();
    controller.lock().unwrap();
    assert_eq!(controller.state(), VaultState::Locked);

    controller
        .unlock_with_released(material, UnlockOptions::default())
        .unwrap();

    assert_eq!(controller.state(), VaultState::Unlocked);
    assert_eq!(controller.kernel().unwrap().account_count().unwrap(), 1);
}

#[test]
fn the_integrity_option_reaches_the_released_unlock_path() {
    let (dir, mut controller) = unlocked();
    let path = dir.path().join("vault.db");
    // Enough rows that the file spans many pages, so damage to the later pages is real.
    for n in 0..400 {
        controller
            .kernel()
            .unwrap()
            .dispatch(CommandEnvelope::new(
                meta(),
                CreateAccount::new(Account::new(
                    AccountId::new(),
                    LedgerAccountId::new(),
                    format!("Account number {n} with a reasonably long name to fill pages"),
                    CashflowRole::LiquidCash,
                    Currency::Usd,
                    AccountFlags::default(),
                )),
            ))
            .unwrap();
    }
    let material = controller.enrollment_unlock_material(PASSWORD).unwrap();
    controller.lock().unwrap();

    // A healthy vault passes the check on the released path.
    let healthy = controller.enrollment_unlock_material(PASSWORD);
    assert!(healthy.is_err(), "locked: no enrollment");
    let again = {
        let mut c = VaultController::open(path.clone());
        c.unlock(PASSWORD).unwrap();
        let m = c.enrollment_unlock_material(PASSWORD).unwrap();
        c.lock().unwrap();
        m
    };
    controller
        .unlock_with_released(again, UnlockOptions::with_integrity_check())
        .expect("a sound vault passes the check");
    controller.lock().unwrap();

    // Damage only the LAST pages: unlike the password path, the released path reads the header and the
    // vault id (the key probe) before the integrity check, so the first pages must stay readable for the
    // check to be the thing that refuses.
    let mut bytes = std::fs::read(&path).unwrap();
    assert!(
        bytes.len() > 8 * 4096,
        "the vault spans pages: {}",
        bytes.len()
    );
    let pages = bytes.len() / 4096;
    for page in pages.saturating_sub(3)..pages {
        for byte in &mut bytes[page * 4096 + 64..page * 4096 + 192] {
            *byte ^= 0xA5;
        }
    }
    std::fs::write(&path, &bytes).unwrap();
    let damaged = std::fs::read(&path).unwrap();

    let refused = controller.unlock_with_released(material, UnlockOptions::with_integrity_check());
    assert!(
        matches!(refused, Err(KernelError::IntegrityCheckFailed { .. })),
        "{refused:?}"
    );
    assert_eq!(controller.state(), VaultState::Locked);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        damaged,
        "the file was not written"
    );
}

#[test]
fn an_envelope_that_wraps_a_different_key_than_the_open_vault_enables_nothing() {
    // Vault A is open. Another vault B with the SAME password has its envelope copied over A's sidecar,
    // as an outside process swapping the file would. The password opens that envelope, but it wraps a
    // different key than the one A is open with: the check must refuse, and the export must not run.
    let (dir_a, controller) = unlocked();
    let dir_b = TempDir::new().unwrap();
    let mut other = VaultController::open(dir_b.path().join("vault.db"));
    other.create(PASSWORD).unwrap();
    other.lock().unwrap();

    let a_sidecar = sidecar(&dir_a.path().join("vault.db"));
    let b_sidecar = sidecar(&dir_b.path().join("vault.db"));
    std::fs::copy(&b_sidecar, &a_sidecar).unwrap();
    let swapped = std::fs::read(&a_sidecar).unwrap();

    let kernel = controller.kernel().unwrap();
    assert!(matches!(
        kernel.verify_password(PASSWORD),
        Err(KernelError::Vault(_))
    ));
    assert!(matches!(
        controller.enrollment_unlock_material(PASSWORD),
        Err(KernelError::Vault(_))
    ));
    // Nothing was written by the refusal.
    assert_eq!(std::fs::read(&a_sidecar).unwrap(), swapped);
}

#[test]
fn a_stale_item_fails_typed_and_leaves_the_controller_locked_and_the_password_working() {
    let (_dir, mut controller) = unlocked();
    let material = controller.enrollment_unlock_material(PASSWORD).unwrap();
    controller.change_password(PASSWORD, OTHER).unwrap();
    controller.lock().unwrap();

    let error = controller
        .unlock_with_released(material, UnlockOptions::default())
        .unwrap_err();
    assert!(
        matches!(
            error,
            KernelError::ReleasedUnlock(ReleasedUnlockError::StaleFingerprint)
        ),
        "{error:?}"
    );
    assert_eq!(controller.state(), VaultState::Locked);
    controller.unlock(OTHER).unwrap();
    assert_eq!(controller.state(), VaultState::Unlocked);
}

#[test]
fn the_controller_refuses_a_released_unlock_unless_it_is_locked() {
    let (_dir, mut controller) = unlocked();
    let material = controller.enrollment_unlock_material(PASSWORD).unwrap();
    assert!(matches!(
        controller.unlock_with_released(material, UnlockOptions::default()),
        Err(KernelError::IllegalVaultTransition { .. })
    ));
    assert_eq!(controller.state(), VaultState::Unlocked);
}
