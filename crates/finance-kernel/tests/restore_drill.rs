//! Restore-drill regression (personal-cfo-7pfu, §19.5). Seeds a realistic vault
//! (accounts with opening balances + dated transactions + a recurring income
//! source + an attachment), exports it, restores on a **fresh** instance, and
//! asserts the canonical state is reproduced: read-model checksums match, every
//! read model is byte-identical, and the attachment decrypts to the same bytes.
//! A divergence is surfaced by the failing assertion (which signal — and which
//! rows — differ). This guards the whole backup/restore + vault-crypto path and
//! runs in CI on every PR (`cargo test --workspace`).

use chrono::{DateTime, NaiveDate, Utc};
use finance_kernel::{
    Account, AccountFlags, AccountId, AccountView, ActorType, CashflowRole, CommandEnvelope,
    CommandMeta, CommitmentView, CreateAccount, CreateIncomeSource, Currency, Frequency,
    IncomeSourceView, Kernel, LedgerAccountId, Money, RecordTransaction, RecurringBillView,
    TransactionId, TransactionRow,
};
use uuid::Uuid;

const PW: &[u8] = b"drill password correct horse";

fn meta() -> CommandMeta {
    CommandMeta {
        command_id: Uuid::now_v7(),
        correlation_id: Uuid::now_v7(),
        causation_id: None,
        actor_type: ActorType::User,
        actor_id: "drill".to_owned(),
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

/// A capture of the canonical read-model state, for equality comparison.
struct Canonical {
    account_count: u64,
    operation_count: u64,
    commitments_checksum: u64,
    transaction_display_checksum: u64,
    accounts: Vec<AccountView>,
    transactions: Vec<TransactionRow>,
    income: Vec<IncomeSourceView>,
    bills: Vec<RecurringBillView>,
    commitments: Vec<CommitmentView>,
}

fn capture(k: &Kernel) -> Canonical {
    Canonical {
        account_count: k.account_count().unwrap(),
        operation_count: k.operation_count().unwrap(),
        commitments_checksum: k.commitments_checksum().unwrap(),
        transaction_display_checksum: k.transaction_display_checksum().unwrap(),
        accounts: k.account_views().unwrap(),
        transactions: k.transactions(1000).unwrap(),
        income: k.income_source_views().unwrap(),
        bills: k.recurring_bill_views().unwrap(),
        commitments: k.commitment_views().unwrap(),
    }
}

#[test]
fn restore_drill_reproduces_the_canonical_state() {
    let src = tempfile::tempdir().unwrap();
    let kernel = Kernel::create_vault(src.path().join("vault.db"), PW).unwrap();

    // Seed a realistic vault: two accounts with opening balances, two
    // transactions, and a recurring income source.
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
    kernel
        .dispatch(CommandEnvelope::new(
            meta(),
            RecordTransaction::new(
                TransactionId::new(),
                checking_id,
                Money::new(150_000, Currency::Usd),
                at("2026-06-10T00:00:00Z"),
            ),
        ))
        .unwrap();
    kernel
        .dispatch(CommandEnvelope::new(
            meta(),
            CreateIncomeSource::new(
                "Acme paycheck",
                Money::new(300_000, Currency::Usd),
                Frequency::Biweekly,
                "2026-06-01".parse::<NaiveDate>().unwrap(),
                Some(checking_id),
            ),
        ))
        .unwrap();
    kernel.rebuild_transaction_display().unwrap();

    // Attach a document to the first transaction (exercises the blob store).
    let txn_id = kernel
        .transactions(10)
        .unwrap()
        .first()
        .unwrap()
        .transaction_id;
    let pdf = [
        b"%PDF-1.4\n".as_slice(),
        b"drill-attachment-canary",
        b"\n%%EOF\n",
    ]
    .concat();
    let attachment = kernel
        .attach_document(txn_id, &pdf, Some("application/pdf"), Some("receipt.pdf"))
        .unwrap();

    // A connector connection + account link (personal-cfo-gglk): the stored
    // credential must round-trip through backup/restore — vault-secret
    // storage exists precisely so a restore carries the connection.
    let connection_id = uuid::Uuid::now_v7();
    kernel
        .create_connector_connection(
            connection_id,
            "simplefin",
            "https://user:pw@bridge.example/simplefin",
            Some("Drill connection"),
        )
        .unwrap();
    kernel
        .upsert_connector_link(connection_id, "ACT-1", Some("Drill Checking"))
        .unwrap();

    let before = capture(&kernel);

    // Export, then restore into a fresh location (a "new machine").
    let pkg = src.path().join("backup.pcfobk");
    kernel
        .export_backup(
            PW,
            &pkg,
            "0.1.0-test",
            "2026-06-20T00:00:00Z".into(),
            Uuid::from_bytes([5u8; 16]),
        )
        .unwrap();

    let dest = tempfile::tempdir().unwrap();
    let restored = Kernel::restore_backup(&pkg, PW, &dest.path().join("vault.db")).unwrap();
    let after = capture(&restored);

    // The connector connection survived, credential included.
    assert_eq!(
        restored.connector_credential(connection_id).unwrap(),
        Some("https://user:pw@bridge.example/simplefin".to_owned()),
        "connector credential must round-trip through backup/restore"
    );
    assert_eq!(restored.connector_links(connection_id).unwrap().len(), 1);

    // Read-model checksums match (ADR 0024 §4 content-checksum equality).
    assert_eq!(
        before.transaction_display_checksum, after.transaction_display_checksum,
        "transaction-display checksum diverged after restore"
    );
    assert_eq!(
        before.commitments_checksum, after.commitments_checksum,
        "commitments checksum diverged after restore"
    );
    // Every read model is byte-identical — the assert diff names the diverging rows.
    assert_eq!(before.account_count, after.account_count);
    assert_eq!(before.operation_count, after.operation_count);
    assert_eq!(
        before.accounts, after.accounts,
        "accounts diverged after restore"
    );
    assert_eq!(
        before.transactions, after.transactions,
        "transactions diverged after restore"
    );
    assert_eq!(
        before.income, after.income,
        "income sources diverged after restore"
    );
    assert_eq!(
        before.bills, after.bills,
        "recurring bills diverged after restore"
    );
    assert_eq!(
        before.commitments, after.commitments,
        "commitments diverged after restore"
    );
    // The attachment round-trips: the restored blob decrypts to the same bytes
    // under the restored DEK (proves blobs/ were bundled + restored).
    assert_eq!(
        restored.read_attachment_bytes(attachment.id).unwrap(),
        pdf,
        "attachment bytes diverged after restore"
    );
}
