//! Cross-version vault-open gate (personal-cfo-7igv, risk personal-cfo-aia0).
//!
//! Pins the persistence engine. Two guarantees, both release-blocking via
//! `cargo test --workspace`:
//!
//! 1. **Version pin.** The *linked* SQLCipher (`PRAGMA cipher_version`) and SQLite
//!    (`rusqlite::version()`) versions must equal the values recorded in
//!    `docs/architecture/stack.md`. A silent `cargo update` that bumps the bundled
//!    engine — which could make existing vaults unopenable — fails here instead of
//!    shipping.
//! 2. **Golden-vault round-trip.** A vault built with known data by the production
//!    Finance Kernel, then closed and re-opened through the real `unlock_vault`
//!    password path, reproduces its canonical state on the pinned engine.
//!
//! ## How the cross-version corpus grows
//! There is one pinned engine today, so the golden vault is generated with the
//! current one. When the pin is **intentionally** bumped, the bumping PR must:
//!   1. update `EXPECTED_SQLCIPHER` / `EXPECTED_SQLITE` here, plus
//!      `docs/architecture/stack.md` and the release notes; and
//!   2. capture the *outgoing* version's vault (`vault.db` + `.envelope`) and add
//!      it to a corpus this test opens, proving the new engine still opens every
//!      prior on-disk format. Per `tests/fixtures/README.md`, binaries are not
//!      committed — capture as base64 text or regenerate from the tagged release.

use chrono::{DateTime, Utc};
use finance_kernel::{
    sqlite_version, Account, AccountFlags, AccountId, ActorType, CashflowRole, CommandEnvelope,
    CommandMeta, CreateAccount, Currency, Kernel, LedgerAccountId, Money, RecordTransaction,
    TransactionId,
};
use uuid::Uuid;

/// The pinned engine versions — **must** match `docs/architecture/stack.md`.
/// Changing these is a deliberate act; see the module-level corpus note.
const EXPECTED_SQLCIPHER: &str = "4.5.7 community";
const EXPECTED_SQLITE: &str = "3.45.3";

const PW: &[u8] = b"golden vault cross-version password";

fn meta() -> CommandMeta {
    CommandMeta {
        command_id: Uuid::now_v7(),
        correlation_id: Uuid::now_v7(),
        causation_id: None,
        actor_type: ActorType::User,
        actor_id: "7igv".to_owned(),
        idempotency_key: Uuid::now_v7().to_string(),
    }
}

fn account(name: &str) -> Account {
    Account::new(
        AccountId::new(),
        LedgerAccountId::new(),
        name,
        CashflowRole::LiquidCash,
        Currency::Usd,
        AccountFlags::default(),
    )
}

fn at(rfc3339: &str) -> DateTime<Utc> {
    rfc3339.parse().unwrap()
}

#[test]
fn linked_engine_versions_match_the_pin() {
    let dir = tempfile::tempdir().unwrap();
    let kernel = Kernel::create_vault(dir.path().join("vault.db"), PW).unwrap();

    assert_eq!(
        kernel.cipher_version().unwrap(),
        EXPECTED_SQLCIPHER,
        "linked SQLCipher version drifted from the pin in docs/architecture/stack.md \
         (risk personal-cfo-aia0): bump the pin, the stack doc, and the release notes, \
         and add the outgoing vault to the cross-version corpus before changing this"
    );
    assert_eq!(
        sqlite_version(),
        EXPECTED_SQLITE,
        "linked SQLite version drifted from the pin in docs/architecture/stack.md \
         (risk personal-cfo-aia0)"
    );
}

#[test]
fn golden_vault_round_trips_on_the_pinned_engine() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("vault.db");

    // Build a golden vault with known data, then drop it (closing the worker) so
    // the next open is a cold launch, not a warm handle.
    let (accounts, checksum) = {
        let kernel = Kernel::create_vault(&db_path, PW).unwrap();
        let checking = account("Checking");
        let checking_id = checking.id();
        kernel
            .dispatch(CommandEnvelope::new(
                meta(),
                CreateAccount::with_opening_balance(checking, Money::new(250_000, Currency::Usd)),
            ))
            .unwrap();
        kernel
            .dispatch(CommandEnvelope::new(
                meta(),
                CreateAccount::with_opening_balance(
                    account("Savings"),
                    Money::new(1_000_000, Currency::Usd),
                ),
            ))
            .unwrap();
        kernel
            .dispatch(CommandEnvelope::new(
                meta(),
                RecordTransaction::new(
                    TransactionId::new(),
                    checking_id,
                    Money::new(-4_500, Currency::Usd),
                    at("2026-06-05T00:00:00Z"),
                ),
            ))
            .unwrap();
        kernel.rebuild_transaction_display().unwrap();
        // The engine that *created* the golden vault is the pinned one.
        assert_eq!(kernel.cipher_version().unwrap(), EXPECTED_SQLCIPHER);
        (
            kernel.account_count().unwrap(),
            kernel.transaction_display_checksum().unwrap(),
        )
    };

    // Re-open through the production password path (a fresh app launch would do
    // exactly this) and assert the canonical state survived close/reopen on the
    // pinned engine.
    let reopened = Kernel::unlock_vault(&db_path, PW).unwrap();
    assert_eq!(
        reopened.account_count().unwrap(),
        accounts,
        "golden vault lost accounts across close/reopen"
    );
    assert_eq!(
        reopened.transaction_display_checksum().unwrap(),
        checksum,
        "golden vault read-model checksum changed across close/reopen on the pinned engine"
    );
    assert_eq!(reopened.cipher_version().unwrap(), EXPECTED_SQLCIPHER);
    assert_eq!(sqlite_version(), EXPECTED_SQLITE);
}
