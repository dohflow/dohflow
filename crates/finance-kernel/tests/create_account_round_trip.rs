//! Account commands round-trip through the kernel against a real SQLCipher
//! vault: validate -> DB write -> op-log entry -> typed outcome (plan DoD §1.2),
//! plus the d3o lifecycle invariant (create -> update -> archive preserves
//! balance; the read-model rebuilds identically from canonical tables).

use finance_kernel::{
    Account, AccountFlags, AccountId, ActorType, ArchiveAccount, CashflowRole, CommandEnvelope,
    CommandMeta, CreateAccount, Currency, Kernel, LedgerAccountId, Money, Outcome, UpdateAccount,
};
use tempfile::TempDir;
use uuid::Uuid;

fn meta() -> CommandMeta {
    CommandMeta {
        command_id: Uuid::now_v7(),
        correlation_id: Uuid::now_v7(),
        causation_id: None,
        actor_type: ActorType::User,
        actor_id: "tester".to_owned(),
        idempotency_key: Uuid::now_v7().to_string(),
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

fn kernel() -> (TempDir, Kernel) {
    let dir = TempDir::new().unwrap();
    let kernel = Kernel::open(dir.path().join("vault.db"), "correct horse battery staple").unwrap();
    (dir, kernel)
}

#[test]
fn create_account_round_trips_through_kernel() {
    let (_dir, kernel) = kernel();
    let account = account_named("Checking");
    let id = account.id();

    let outcome = kernel
        .dispatch(CommandEnvelope::new(meta(), CreateAccount::new(account)))
        .expect("dispatch should succeed");

    assert!(matches!(outcome, Outcome::Applied { .. }));
    assert_eq!(kernel.account_count().unwrap(), 1);
    assert!(kernel.account_exists(id).unwrap());
    assert_eq!(kernel.operation_count().unwrap(), 1);
}

#[test]
fn replayed_command_writes_nothing_new() {
    let (_dir, kernel) = kernel();
    let m = meta();
    let account = account_named("Checking");

    let first = kernel
        .dispatch(CommandEnvelope::new(
            m.clone(),
            CreateAccount::new(account.clone()),
        ))
        .unwrap();
    let second = kernel
        .dispatch(CommandEnvelope::new(m, CreateAccount::new(account)))
        .unwrap();

    assert!(matches!(first, Outcome::Applied { .. }));
    assert!(matches!(second, Outcome::Replayed { .. }));
    assert_eq!(kernel.account_count().unwrap(), 1);
    assert_eq!(kernel.operation_count().unwrap(), 1);
}

#[test]
fn opening_balance_via_equity_posting_sets_balance() {
    let (_dir, kernel) = kernel();
    let account = account_named("Savings");
    let id = account.id();
    let opening = Money::new(100_000, Currency::Usd);

    kernel
        .dispatch(CommandEnvelope::new(
            meta(),
            CreateAccount::with_opening_balance(account, opening),
        ))
        .unwrap();

    assert_eq!(kernel.account_balance(id).unwrap(), Some(opening));
}

#[test]
fn mismatched_opening_balance_currency_is_rejected() {
    let (_dir, kernel) = kernel();
    let account = account_named("Savings"); // USD
    let bad = CreateAccount::with_opening_balance(account, Money::new(100, Currency::Eur));
    let result = kernel.dispatch(CommandEnvelope::new(meta(), bad));
    assert!(result.is_err(), "currency mismatch must fail validation");
    // Nothing was written.
    assert_eq!(kernel.account_count().unwrap(), 0);
}

#[test]
fn lifecycle_preserves_balance_and_rebuilds_identically() {
    // d3o: create -> update -> archive preserves balance; the read-model row
    // rebuilt from canonical tables is identical on repeated projection.
    let (_dir, kernel) = kernel();
    let account = account_named("Checking");
    let id = account.id();
    let opening = Money::new(25_000, Currency::Usd);

    kernel
        .dispatch(CommandEnvelope::new(
            meta(),
            CreateAccount::with_opening_balance(account, opening),
        ))
        .unwrap();
    let after_create = kernel.account_view(id).unwrap().expect("exists");

    kernel
        .dispatch(CommandEnvelope::new(
            meta(),
            UpdateAccount::new(id, "Primary Checking"),
        ))
        .unwrap();
    kernel
        .dispatch(CommandEnvelope::new(meta(), ArchiveAccount::new(id)))
        .unwrap();

    let after_archive = kernel.account_view(id).unwrap().expect("still present");

    // Balance is preserved across update + archive.
    assert_eq!(after_create.balance, opening);
    assert_eq!(after_archive.balance, opening);
    // Archive is non-destructive: the row still exists, just inactive + renamed.
    assert!(after_create.active);
    assert!(!after_archive.active);
    assert_eq!(after_archive.name, "Primary Checking");

    // Rebuild determinism: projecting again yields an identical read-model row.
    let rebuilt = kernel.account_view(id).unwrap().expect("still present");
    assert_eq!(after_archive, rebuilt);
}

#[test]
fn empty_update_name_is_rejected() {
    let (_dir, kernel) = kernel();
    let account = account_named("Checking");
    let id = account.id();
    kernel
        .dispatch(CommandEnvelope::new(meta(), CreateAccount::new(account)))
        .unwrap();

    let result = kernel.dispatch(CommandEnvelope::new(meta(), UpdateAccount::new(id, "   ")));
    assert!(result.is_err(), "empty name must fail validation");
}

/// The Future Cash chart's series-selection preference round-trips through the vault
/// settings KV (personal-cfo-4d8.25.26): unset reads None (the frontend then defaults
/// to the aggregate tiers); set persists the opaque JSON; re-set upserts.
#[test]
fn future_cash_series_selection_round_trips() {
    let (_dir, kernel) = kernel();
    assert_eq!(kernel.future_cash_series_selection().unwrap(), None);

    kernel
        .set_future_cash_series_selection(r#"["net","spendable"]"#)
        .unwrap();
    assert_eq!(
        kernel.future_cash_series_selection().unwrap().as_deref(),
        Some(r#"["net","spendable"]"#),
    );

    kernel
        .set_future_cash_series_selection(r#"["acct:abc"]"#)
        .unwrap();
    assert_eq!(
        kernel.future_cash_series_selection().unwrap().as_deref(),
        Some(r#"["acct:abc"]"#),
    );
}
