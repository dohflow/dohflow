//! Setup and substitution for the deterministic sample of every `WriteCommand` variant (moved out of
//! `column_kinds.rs` so the write-set oracle, the engine pin and the replay property share one
//! runner): the order the samples can apply in, the commands that satisfy a variant's references
//! (`before`), and the sample rewritten to name what the setup created (`command_for`).

use super::samples::{account, category, event, tag, txn, u, usd};
use chrono::NaiveDate;
use core_ledger::{Account, AccountFlags, BillContractId, CashflowRole, LedgerAccountId};
use core_money::{Currency, Money};
use db_worker::{
    AssumptionKind, AssumptionSource, DbWorker, NewAssumptionEvent, NewScenario, WriteCommand,
};
use pay_schedule::Frequency;
use uuid::Uuid;

/// The order in which the samples can apply: every variant after the entities it refers to
/// exist, and the variants that end a transaction's life (`void_transaction`) last. A new
/// variant must be placed here, or the permutation check below fails.
pub const ORDER: &[&str] = &[
    "create_account",
    "update_account",
    "archive_account",
    "reinstate_account",
    "set_account_subtype",
    "set_account_note",
    "set_account_link",
    "set_debt_terms",
    "set_card_statement_balance",
    "record_transaction",
    "create_category",
    "update_category",
    "move_category",
    "archive_category",
    "reinstate_category",
    "create_tag",
    "mint_merchant_anchors",
    "apply_merchant_memory",
    "set_user_setting",
    "recategorize_transaction",
    "mark_reviewed",
    "set_tags",
    "set_note",
    "set_splits",
    "confirm_obligation_early",
    "unconfirm_obligation",
    "convert_unexplained_to_transaction",
    "transfer",
    "create_recurring_transfer",
    "delete_recurring_transfer",
    "reinstate_recurring_transfer",
    "create_income_source",
    "update_income_source",
    "delete_income_source",
    "reinstate_income_source",
    "archive_income_source",
    "restore_income_source",
    "create_recurring_bill",
    "set_bill_autopay",
    "update_recurring_bill",
    "delete_recurring_bill",
    "reinstate_recurring_bill",
    "archive_recurring_bill",
    "restore_recurring_bill",
    "link_bill_to_loan",
    "unlink_bill_from_loan",
    "set_bill_not_loan_payment",
    "create_connector_connection",
    "upsert_connector_link",
    "set_connector_link_account",
    "delete_connector_connection",
    "create_source_batch",
    "attach_source_record",
    "update_batch_state",
    "commit_staged",
    "skip_staged",
    "commit_imported_transaction",
    "snooze_inbox_item",
    "dismiss_inbox_item",
    "dismiss_recurring_suggestion",
    "apply_scenario",
    "revert_scenario_apply_confirmed",
    "revert_scenario_apply",
    "void_transaction",
    "set_household_timezone",
    "record_balance_assertion",
    "record_connector_balance",
    "set_household_name",
    "add_household_member",
    "rename_household_member",
    "set_member_profile",
    "transfer_primary",
    "remove_household_member",
    "restore_household_member",
    "record_assumption_event",
    "supersede_assumption_event",
    "clear_assumption_event",
    "create_scenario",
    "set_scenario_status",
    "rename_scenario",
    "set_scenario_expiry",
    "clone_scenario",
    "delete_scenario",
    "reinstate_scenario",
];

/// Ids the setup learns at run time (staging mints its own).
#[derive(Default)]
pub struct Runtime {
    pub staged: Vec<Uuid>,
    /// The source record of the first staged row (an import commit names it).
    pub source_record: Option<Uuid>,
    /// The staged rows `before` named are not inserted until its setup commands have run.
    pub staged_pending: bool,
    /// The vault's first member, whose id derives from the vault id.
    pub primary: Option<Uuid>,
}

pub fn account_cmd(n: u128, name: &str, role: CashflowRole) -> WriteCommand {
    WriteCommand::CreateAccount {
        account: Box::new(Account::new(
            account(n),
            LedgerAccountId::from_uuid(u(100 + n)),
            name,
            role,
            Currency::Usd,
            AccountFlags::default(),
        )),
        opening_balance: None,
    }
}

pub fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// Insert the class-3 staged rows `before` asked for, once its setup commands (which create the
/// source records they point at) have been dispatched. Fixed ids, and the row ordinal a staging
/// pass would give them.
pub fn after_setup(worker: &DbWorker, rt: &mut Runtime) {
    if !std::mem::take(&mut rt.staged_pending) {
        return;
    }
    let conn = worker.read_connection().unwrap();
    for (index, staged) in rt.staged.iter().enumerate() {
        let n = i64::try_from(index).unwrap() + 1;
        conn.execute(
            "INSERT INTO staged_transactions
                (id, source_record_id, proposed_account_id, posted_at, amount_minor, currency,
                 normalized_merchant, description, txn_fingerprint, created_at, row_ordinal)
             VALUES (?1, ?2, ?3, '2026-06-01', ?4, 'USD', ?5, ?5, ?6, '2026-11-04T00:00:00+00:00', ?7)",
            rusqlite::params![
                staged,
                u(920 + u128::try_from(n).unwrap()),
                account(1).as_uuid(),
                -100 * n,
                format!("payee {n}"),
                format!("2026-06-01|{}|payee {n}", -100 * n),
                n,
            ],
        )
        .expect("setup: a staged transaction");
    }
}

/// Commands that satisfy a variant's references, dispatched (and checked) just before it.
pub fn before(
    name: &str,
    _worker: &DbWorker,
    rt: &mut Runtime,
) -> Vec<(&'static str, WriteCommand)> {
    match name {
        "create_account" => vec![(
            "setup: a euro account with an opening balance, so the system account for the new \
             currency is created (a map row)",
            WriteCommand::CreateAccount {
                account: Box::new(Account::new(
                    account(90),
                    LedgerAccountId::from_uuid(u(190)),
                    "Euro checking",
                    CashflowRole::LiquidCash,
                    Currency::Eur,
                    AccountFlags::default(),
                )),
                opening_balance: Some(Money::new(10_000, Currency::Eur)),
            },
        )],
        "set_account_link" => vec![
            (
                "setup: asset account",
                account_cmd(4, "House", CashflowRole::RealAsset),
            ),
            (
                "setup: liability account",
                account_cmd(5, "Mortgage", CashflowRole::LoanLiability),
            ),
        ],
        "set_card_statement_balance" => vec![
            (
                "setup: a credit card",
                account_cmd(28, "Visa", CashflowRole::CreditFacility),
            ),
            (
                "setup: mark it a credit card",
                WriteCommand::SetAccountSubtype {
                    id: account(28),
                    subtype: Some(core_ledger::AccountSubtype::CreditCard),
                },
            ),
        ],
        "record_connector_balance" => vec![(
            "setup: an earlier connector reading of the same day, so the sample updates it",
            WriteCommand::RecordConnectorBalance {
                account_id: account(1),
                observed_at: date(2026, 10, 1),
                amount: usd(100_000),
            },
        )],
        "link_bill_to_loan" => vec![
            (
                "setup: a loan paid monthly",
                account_cmd(27, "Auto loan", CashflowRole::LoanLiability),
            ),
            (
                "setup: its monthly payment terms",
                WriteCommand::SetDebtTerms {
                    account_id: account(27),
                    terms: db_worker::DebtTermsInput {
                        apr_bps: Some(500),
                        statement_close_day: None,
                        payment_due_day: Some(1),
                        grace_period_days: None,
                        credit_limit_minor: None,
                        repayment_philosophy: db_worker::RepaymentPhilosophy::PayFixedAmount,
                        fixed_amount_minor: Some(215_000),
                        min_payment_percent_bps: None,
                        min_payment_floor_minor: None,
                        paying_source_account_id: Some(account(3)),
                        original_principal_minor: Some(30_000_000),
                    },
                },
            ),
        ],
        "set_debt_terms" => vec![(
            "setup: paying source account",
            account_cmd(3, "Savings", CashflowRole::LiquidCash),
        )],
        "create_category" => vec![(
            "setup: parent category",
            WriteCommand::CreateCategory {
                id: category(24),
                parent_id: None,
                name: "Spending".into(),
                category_type: "expense".into(),
                color: None,
                icon: None,
            },
        )],
        "create_tag" => vec![(
            "setup: second tag",
            WriteCommand::CreateTag {
                id: tag(18),
                name: "work".into(),
                color: None,
            },
        )],
        "confirm_obligation_early" => vec![(
            "setup: the recurring bill the obligation belongs to",
            WriteCommand::CreateRecurringBill {
                event_id: event(11),
                contract_id: BillContractId::from_uuid(u(26)),
                name: "Phone".into(),
                amount: usd(8_000),
                bill_type: "utility".into(),
                frequency: Frequency::Monthly,
                anchor: date(2026, 1, 1),
                autopay_account_id: None,
                description: None,
                source_merchant_key: None,
                category_id: None,
                tag_ids: vec![],
            },
        )],
        "convert_unexplained_to_transaction" => vec![(
            // An asserted balance with nothing explaining it: the residual to convert.
            "setup: an asserted balance with nothing explaining it",
            WriteCommand::RecordBalanceAssertion {
                id: u(900),
                account_id: account(1),
                amount: Money::new(900_000, Currency::Usd),
                as_of: date(2026, 10, 1),
            },
        )],
        "commit_staged" | "commit_imported_transaction" => {
            // Two staged transactions: one to commit, one to skip; the import commit names the
            // source record of the first. Everything has a fixed id, so the same setup yields the
            // same state in every run (the engine pin hashes it). The batch and the records are
            // class 1b and go in through commands; the staged rows are class 3 and are inserted
            // directly, with the row ordinal a staging pass would give them.
            if !rt.staged.is_empty() {
                // Set up by the first of the two variants.
                return vec![];
            }
            rt.staged = vec![u(930), u(931)];
            rt.source_record = Some(u(921));
            let mut setup = vec![(
                "setup: the source batch",
                WriteCommand::CreateSourceBatch {
                    id: core_ledger::SourceBatchId::from_uuid(u(920)),
                    source_type: "csv".to_owned(),
                    source_name: Some("oracle".to_owned()),
                    file_fingerprint: Some(super::samples::ORACLE_FINGERPRINT.to_owned()),
                    parser_version: None,
                    connector_connection_id: None,
                },
            )];
            for (n, label) in [
                (1_i64, "setup: the first source record"),
                (2, "setup: the second source record"),
            ] {
                setup.push((
                    label,
                    WriteCommand::AttachSourceRecord {
                        id: core_ledger::SourceRecordId::from_uuid(u(920 + n as u128)),
                        batch_id: core_ledger::SourceBatchId::from_uuid(u(920)),
                        external_id: None,
                        source_hash: format!("sha256:oracle-{n}"),
                        normalized_json: "{}".to_owned(),
                        parse_confidence_bps: Some(10_000),
                    },
                ));
            }
            rt.staged_pending = true;
            setup
        }
        // The clone names two events of its source, so the source needs them first.
        "clone_scenario" => [
            (92, "setup: first event to clone"),
            (93, "setup: second event to clone"),
        ]
        .into_iter()
        .map(|(n, label)| {
            (
                label,
                WriteCommand::RecordAssumptionEvent {
                    event: NewAssumptionEvent {
                        id: u(n),
                        kind: AssumptionKind::InflationRate,
                        target_entity_type: None,
                        target_entity_id: None,
                        params_json: r#"{"annual_bps":400}"#.to_owned(),
                        source: AssumptionSource::UserOverride,
                        scenario_id: Some(u(90)),
                    },
                },
            )
        })
        .collect(),
        "apply_scenario" => vec![
            (
                "setup: a scenario",
                WriteCommand::CreateScenario {
                    scenario: NewScenario {
                        id: u(25),
                        name: "Oracle".to_owned(),
                        description: None,
                    },
                },
            ),
            (
                "setup: an assumption event in it",
                WriteCommand::RecordAssumptionEvent {
                    event: NewAssumptionEvent {
                        id: u(903),
                        kind: AssumptionKind::InflationRate,
                        target_entity_type: None,
                        target_entity_id: None,
                        params_json: r#"{"annual_bps":400}"#.to_owned(),
                        source: AssumptionSource::UserOverride,
                        scenario_id: Some(u(25)),
                    },
                },
            ),
        ],
        // The confirmed undo just unapplied the scenario; apply it again for the retired one.
        "revert_scenario_apply" => vec![(
            "setup: apply the scenario again",
            WriteCommand::ApplyScenario { scenario_id: u(25) },
        )],
        _ => vec![],
    }
}

/// The command to run for a variant. Most are the sample itself. A few samples carry values
/// chosen to stress the payload codec (an amount beyond a double, a zero transfer, a negative
/// bill) that no vault would accept, or ids minted at run time; those run as a command of the
/// SAME variant with applicable values, so the variant is still proven, not exempted.
pub fn command_for(name: &str, sample: WriteCommand, rt: &Runtime) -> WriteCommand {
    let mut command = command_for_inner(name, sample, rt);
    // The sample's bill type `rent` is not one of the vault's bill types.
    if let WriteCommand::CreateRecurringBill { bill_type, .. }
    | WriteCommand::UpdateRecurringBill { bill_type, .. } = &mut command
    {
        *bill_type = "rent_mortgage".to_owned();
    }
    command
}

pub fn command_for_inner(name: &str, sample: WriteCommand, rt: &Runtime) -> WriteCommand {
    let primary = rt.primary.expect("the primary member's id");
    match (name, sample) {
        ("transfer_primary", WriteCommand::TransferPrimary { to_member_id, .. }) => {
            WriteCommand::TransferPrimary {
                from_member_id: primary,
                to_member_id,
            }
        }
        // The sample names a second transaction that this vault does not hold.
        ("apply_merchant_memory", _) => WriteCommand::ApplyMerchantMemory {
            assignments: vec![db_worker::MerchantAssignment {
                transaction_id: txn(10),
                category_id: category(16),
                confidence_bps: 9_000,
            }],
        },
        ("revert_scenario_apply_confirmed", _) => WriteCommand::RevertScenarioApplyConfirmed {
            scenario_id: u(25),
            confirmed_descendants: Vec::new(),
        },
        ("remove_household_member", _) => {
            WriteCommand::RemoveHouseholdMember { member_id: primary }
        }
        ("restore_household_member", _) => {
            WriteCommand::RestoreHouseholdMember { member_id: primary }
        }
        (
            "confirm_obligation_early",
            WriteCommand::ConfirmObligationEarly {
                recurring_event_id,
                scheduled_date,
                actual_date,
                paying_account_id,
                ..
            },
        ) => WriteCommand::ConfirmObligationEarly {
            recurring_event_id,
            scheduled_date,
            actual_amount: usd(7_500),
            actual_date,
            paying_account_id,
        },
        (
            "transfer",
            WriteCommand::Transfer {
                source_account_id,
                occurred_at,
                ..
            },
        ) => WriteCommand::Transfer {
            // The sample's destination is a property, which cannot receive a transfer.
            source_account_id,
            dest_account_id: account(3),
            amount: usd(12_345),
            occurred_at,
        },
        (
            "create_recurring_transfer",
            WriteCommand::CreateRecurringTransfer {
                id,
                source_account_id,
                amount,
                frequency,
                anchor,
                ..
            },
        ) => WriteCommand::CreateRecurringTransfer {
            id,
            source_account_id,
            dest_account_id: account(3),
            amount,
            frequency,
            anchor,
        },
        (
            "create_recurring_bill",
            WriteCommand::CreateRecurringBill {
                event_id,
                contract_id,
                name,
                bill_type,
                frequency,
                anchor,
                autopay_account_id,
                description,
                source_merchant_key,
                category_id,
                tag_ids,
                ..
            },
        ) => WriteCommand::CreateRecurringBill {
            event_id,
            contract_id,
            name,
            amount: usd(215_000),
            bill_type,
            frequency,
            anchor,
            autopay_account_id,
            description,
            source_merchant_key,
            category_id,
            tag_ids,
        },
        (
            "update_recurring_bill",
            WriteCommand::UpdateRecurringBill {
                event_id,
                name,
                bill_type,
                frequency,
                anchor,
                autopay_account_id,
                description,
                ..
            },
        ) => WriteCommand::UpdateRecurringBill {
            event_id,
            name,
            amount: usd(220_000),
            bill_type,
            frequency,
            anchor,
            autopay_account_id,
            description,
        },
        ("link_bill_to_loan", WriteCommand::LinkBillToLoan { event_id, .. }) => {
            WriteCommand::LinkBillToLoan {
                event_id,
                loan_account_id: account(27),
            }
        }
        ("set_bill_not_loan_payment", WriteCommand::SetBillNotLoanPayment { event_id, .. }) => {
            WriteCommand::SetBillNotLoanPayment {
                event_id,
                loan_account_id: Some(account(27)),
            }
        }
        (
            "set_card_statement_balance",
            WriteCommand::SetCardStatementBalance { cycle_close, .. },
        ) => WriteCommand::SetCardStatementBalance {
            account_id: account(28),
            cycle_close,
            statement_balance_minor: Some(184_500),
        },
        (
            "record_transaction",
            WriteCommand::RecordTransaction {
                transaction_id,
                account_id,
                occurred_at,
                ..
            },
        ) => WriteCommand::RecordTransaction {
            // The split sample divides -1,000 into -700 and -300.
            transaction_id,
            account_id,
            amount: usd(-1_000),
            occurred_at,
        },
        ("commit_staged", WriteCommand::CommitStaged { force, .. }) => WriteCommand::CommitStaged {
            staged_transaction_id: core_ledger::StagedTransactionId::from_uuid(rt.staged[0]),
            force,
        },
        ("skip_staged", WriteCommand::SkipStaged { .. }) => WriteCommand::SkipStaged {
            staged_transaction_id: core_ledger::StagedTransactionId::from_uuid(rt.staged[1]),
        },
        (
            "commit_imported_transaction",
            WriteCommand::CommitImportedTransaction {
                transaction_id,
                occurred_at,
                amount,
                counterparty,
                memo,
                transaction_date,
                forced,
                source_record_id: identity_record,
                identity,
                ..
            },
        ) => WriteCommand::CommitImportedTransaction {
            transaction_id,
            account_id: account(1),
            // A sample that claims an identity names its own source record (the identity binds to
            // it); the others use the record the setup staged.
            source_record_id: if identity.is_some() {
                identity_record
            } else {
                core_ledger::SourceRecordId::from_uuid(
                    rt.source_record.expect("setup staged a batch"),
                )
            },
            occurred_at,
            amount,
            counterparty,
            memo,
            transaction_date,
            // The sample's category does not exist in the oracle's vault.
            category_id: None,
            forced,
            identity,
        },
        (_, sample) => sample,
    }
}
