//! The deterministic sample of every `WriteCommand` variant (personal-cfo-t2s4b part 3b),
//! shared by the payload-schema tests and the write-set oracle (personal-cfo-vpza7).

use chrono::{NaiveDate, TimeZone, Utc};
use core_ledger::{
    Account, AccountFlags, AccountId, AccountSubtype, BillContractId, CashflowRole, CategoryId,
    IncomeSourceId, LedgerAccountId, RecurringEventId, RecurringTransferId, SourceBatchId,
    SourceRecordId, StagedTransactionId, TagId, TransactionId,
};
use core_money::{Currency, Money};
use db_worker::{
    AssumptionKind, AssumptionSource, DebtTermsInput, NewAssumptionEvent, NewScenario,
    RepaymentPhilosophy, ScenarioStatus, SplitLineInput, WriteCommand,
};
use pay_schedule::Frequency;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Deterministic sample values
// ---------------------------------------------------------------------------

pub fn u(n: u128) -> Uuid {
    Uuid::from_u128(0x0198_c2c1_5f0a_7c3d_8a4e_9b1d_0000_0000 + n)
}
pub fn account(n: u128) -> AccountId {
    AccountId::from_uuid(u(n))
}
pub fn category(n: u128) -> CategoryId {
    CategoryId::from_uuid(u(n))
}
pub fn tag(n: u128) -> TagId {
    TagId::from_uuid(u(n))
}
pub fn txn(n: u128) -> TransactionId {
    TransactionId::from_uuid(u(n))
}
pub fn event(n: u128) -> RecurringEventId {
    RecurringEventId::from_uuid(u(n))
}
pub fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}
pub fn instant() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 17, 12, 30, 45).unwrap()
}
pub fn usd(minor: i64) -> Money {
    Money::new(minor, Currency::Usd)
}

/// 2^53 + 1: the first integer a double cannot hold. A JSON number would corrupt it.
pub const BEYOND_DOUBLE: i64 = 9_007_199_254_740_993;

/// One sample per variant, named by the variant's wire key (its snake-case name).
pub fn samples() -> Vec<(&'static str, WriteCommand)> {
    let flags = AccountFlags {
        retirement: false,
        tax_advantaged: true,
        joint: false,
        business: true,
        active: true,
    };
    let checking = Account::new(
        account(1),
        LedgerAccountId::from_uuid(u(2)),
        "Checking \u{e9}",
        CashflowRole::LiquidCash,
        Currency::Usd,
        flags,
    );
    let terms = DebtTermsInput {
        apr_bps: Some(2_499),
        statement_close_day: Some(15),
        payment_due_day: Some(10),
        grace_period_days: Some(25),
        credit_limit_minor: Some(BEYOND_DOUBLE),
        repayment_philosophy: RepaymentPhilosophy::PayStatementBalance,
        fixed_amount_minor: None,
        min_payment_percent_bps: Some(200),
        min_payment_floor_minor: Some(2_500),
        paying_source_account_id: Some(account(3)),
        original_principal_minor: Some(i64::MAX),
    };
    vec![
        (
            "create_account",
            WriteCommand::CreateAccount {
                account: Box::new(checking.clone()),
                opening_balance: Some(usd(-150_000)),
            },
        ),
        (
            "update_account",
            WriteCommand::UpdateAccount {
                id: account(1),
                name: "Joint \"main\"".into(),
            },
        ),
        ("archive_account", WriteCommand::ArchiveAccount(account(1))),
        (
            "reinstate_account",
            WriteCommand::ReinstateAccount(account(1)),
        ),
        (
            "set_account_subtype",
            WriteCommand::SetAccountSubtype {
                id: account(1),
                subtype: Some(AccountSubtype::Savings),
            },
        ),
        (
            "set_account_note",
            WriteCommand::SetAccountNote {
                id: account(1),
                note: Some("line one\nline two \u{1f4b0}".into()),
            },
        ),
        (
            "set_account_link",
            WriteCommand::SetAccountLink {
                asset_id: account(4),
                liability_id: Some(account(5)),
            },
        ),
        (
            "set_debt_terms",
            WriteCommand::SetDebtTerms {
                account_id: account(5),
                terms,
            },
        ),
        (
            "set_card_statement_balance",
            WriteCommand::SetCardStatementBalance {
                account_id: account(5),
                cycle_close: date(2026, 9, 15),
                statement_balance_minor: Some(i64::MIN),
            },
        ),
        (
            "record_transaction",
            WriteCommand::RecordTransaction {
                transaction_id: txn(10),
                account_id: account(1),
                amount: usd(-1),
                occurred_at: instant(),
            },
        ),
        (
            "confirm_obligation_early",
            WriteCommand::ConfirmObligationEarly {
                recurring_event_id: event(11),
                scheduled_date: date(2026, 11, 1),
                actual_amount: Money::new(-BEYOND_DOUBLE, Currency::Eur),
                actual_date: instant(),
                paying_account_id: account(1),
            },
        ),
        (
            "unconfirm_obligation",
            WriteCommand::UnconfirmObligation {
                recurring_event_id: event(11),
                scheduled_date: date(2026, 11, 1),
            },
        ),
        (
            "convert_unexplained_to_transaction",
            WriteCommand::ConvertUnexplainedToTransaction {
                account_id: account(1),
            },
        ),
        (
            "transfer",
            WriteCommand::Transfer {
                source_account_id: account(1),
                dest_account_id: account(4),
                amount: usd(0),
                occurred_at: instant(),
            },
        ),
        (
            "create_recurring_transfer",
            WriteCommand::CreateRecurringTransfer {
                id: RecurringTransferId::from_uuid(u(12)),
                source_account_id: account(1),
                dest_account_id: account(4),
                amount: usd(50_000),
                frequency: Frequency::EveryNWeeks(6),
                anchor: date(2026, 1, 31),
            },
        ),
        (
            "delete_recurring_transfer",
            WriteCommand::DeleteRecurringTransfer(RecurringTransferId::from_uuid(u(12))),
        ),
        (
            "reinstate_recurring_transfer",
            WriteCommand::ReinstateRecurringTransfer(RecurringTransferId::from_uuid(u(12))),
        ),
        (
            "create_income_source",
            WriteCommand::CreateIncomeSource {
                id: IncomeSourceId::from_uuid(u(13)),
                name: "Paycheck".into(),
                net_amount: usd(312_345),
                frequency: Frequency::SemiMonthly,
                anchor: date(2026, 2, 15),
                deposit_account_id: Some(account(1)),
            },
        ),
        (
            "update_income_source",
            WriteCommand::UpdateIncomeSource {
                id: IncomeSourceId::from_uuid(u(13)),
                name: "Paycheck (raise)".into(),
                net_amount: usd(330_000),
                frequency: Frequency::Biweekly,
                anchor: date(2026, 2, 13),
                deposit_account_id: None,
            },
        ),
        (
            "delete_income_source",
            WriteCommand::DeleteIncomeSource(IncomeSourceId::from_uuid(u(13))),
        ),
        (
            "reinstate_income_source",
            WriteCommand::ReinstateIncomeSource(IncomeSourceId::from_uuid(u(13))),
        ),
        (
            "archive_income_source",
            WriteCommand::ArchiveIncomeSource(IncomeSourceId::from_uuid(u(13))),
        ),
        (
            "restore_income_source",
            WriteCommand::RestoreIncomeSource(IncomeSourceId::from_uuid(u(13))),
        ),
        (
            "create_recurring_bill",
            WriteCommand::CreateRecurringBill {
                event_id: event(14),
                contract_id: BillContractId::from_uuid(u(15)),
                name: "Rent".into(),
                amount: usd(-215_000),
                bill_type: "rent".into(),
                frequency: Frequency::Monthly,
                anchor: date(2026, 3, 1),
                autopay_account_id: Some(account(1)),
                description: Some("Lease #12".into()),
                source_merchant_key: Some("landlord llc".into()),
                category_id: Some(category(16)),
                tag_ids: vec![tag(17), tag(18)],
            },
        ),
        (
            "set_bill_autopay",
            WriteCommand::SetBillAutopay {
                event_id: event(14),
                autopay: true,
            },
        ),
        (
            "update_recurring_bill",
            WriteCommand::UpdateRecurringBill {
                event_id: event(14),
                name: "Rent".into(),
                amount: usd(-220_000),
                bill_type: "rent".into(),
                frequency: Frequency::EveryNMonths(3),
                anchor: date(2026, 3, 1),
                autopay_account_id: None,
                description: None,
            },
        ),
        (
            "delete_recurring_bill",
            WriteCommand::DeleteRecurringBill {
                event_id: event(14),
            },
        ),
        (
            "reinstate_recurring_bill",
            WriteCommand::ReinstateRecurringBill {
                event_id: event(14),
            },
        ),
        (
            "archive_recurring_bill",
            WriteCommand::ArchiveRecurringBill(event(14)),
        ),
        (
            "restore_recurring_bill",
            WriteCommand::RestoreRecurringBill(event(14)),
        ),
        (
            "link_bill_to_loan",
            WriteCommand::LinkBillToLoan {
                event_id: event(14),
                loan_account_id: account(1),
            },
        ),
        (
            "unlink_bill_from_loan",
            WriteCommand::UnlinkBillFromLoan {
                event_id: event(14),
            },
        ),
        (
            "set_bill_not_loan_payment",
            WriteCommand::SetBillNotLoanPayment {
                event_id: event(14),
                loan_account_id: Some(account(1)),
            },
        ),
        (
            "create_connector_connection",
            WriteCommand::CreateConnectorConnection {
                id: u(40),
                adapter_id: "simplefin".into(),
                display_hint: Some("Primary bank \u{e9}".into()),
            },
        ),
        (
            "upsert_connector_link",
            WriteCommand::UpsertConnectorLink {
                connection_id: u(40),
                external_id: "ACT-1".into(),
                external_name: Some("Checking".into()),
                currency: Some("USD".into()),
            },
        ),
        (
            "set_connector_link_account",
            WriteCommand::SetConnectorLinkAccount {
                connection_id: u(40),
                external_id: "ACT-1".into(),
                account_id: Some(account(1)),
            },
        ),
        (
            "delete_connector_connection",
            WriteCommand::DeleteConnectorConnection { id: u(40) },
        ),
        (
            "create_source_batch",
            WriteCommand::CreateSourceBatch {
                id: SourceBatchId::from_uuid(u(19)),
                source_type: "csv".into(),
                source_name: Some("export.csv".into()),
                file_fingerprint: Some("sha256:ab".into()),
                parser_version: Some("csv-v1".into()),
                connector_connection_id: Some(u(20)),
            },
        ),
        (
            "attach_source_record",
            WriteCommand::AttachSourceRecord {
                id: SourceRecordId::from_uuid(u(21)),
                batch_id: SourceBatchId::from_uuid(u(19)),
                external_id: Some("ext-1".into()),
                source_hash: "sha256:cd".into(),
                normalized_json: "{\"amount\":-1299,\"memo\":\"caf\u{e9}\"}".into(),
                parse_confidence_bps: Some(10_000),
            },
        ),
        (
            "update_batch_state",
            WriteCommand::UpdateBatchState {
                batch_id: SourceBatchId::from_uuid(u(19)),
                status: "committed".into(),
                staged_count: 12,
                committed_count: 10,
                skipped_count: 2,
            },
        ),
        (
            "commit_staged",
            WriteCommand::CommitStaged {
                staged_transaction_id: StagedTransactionId::from_uuid(u(22)),
                force: false,
            },
        ),
        (
            "skip_staged",
            WriteCommand::SkipStaged {
                staged_transaction_id: StagedTransactionId::from_uuid(u(22)),
            },
        ),
        (
            "snooze_inbox_item",
            WriteCommand::SnoozeInboxItem {
                item_id: u(23),
                until: date(2026, 12, 25),
            },
        ),
        (
            "dismiss_inbox_item",
            WriteCommand::DismissInboxItem {
                item_id: u(23),
                reason: "not mine".into(),
            },
        ),
        (
            "dismiss_recurring_suggestion",
            WriteCommand::DismissRecurringSuggestion {
                merchant_key: "coffee shop".into(),
                currency: "USD".into(),
                amount_minor: -450,
                frequency: "weekly".into(),
                reason: None,
            },
        ),
        (
            "create_category",
            WriteCommand::CreateCategory {
                id: category(16),
                parent_id: Some(category(24)),
                name: "Groceries".into(),
                category_type: "expense".into(),
                color: Some("#00aa55".into()),
                icon: None,
            },
        ),
        (
            "update_category",
            WriteCommand::UpdateCategory {
                id: category(16),
                name: "Food".into(),
                color: None,
                icon: Some("apple".into()),
            },
        ),
        (
            "move_category",
            WriteCommand::MoveCategory {
                id: category(16),
                new_parent_id: None,
            },
        ),
        (
            "archive_category",
            WriteCommand::ArchiveCategory(category(16)),
        ),
        (
            "reinstate_category",
            WriteCommand::ReinstateCategory(category(16)),
        ),
        (
            "recategorize_transaction",
            WriteCommand::RecategorizeTransaction {
                transaction_id: txn(10),
                category_id: Some(category(16)),
            },
        ),
        (
            "void_transaction",
            WriteCommand::VoidTransaction {
                transaction_id: txn(10),
            },
        ),
        (
            "mark_reviewed",
            WriteCommand::MarkReviewed {
                transaction_id: txn(10),
                reviewed: true,
            },
        ),
        (
            "apply_scenario",
            WriteCommand::ApplyScenario { scenario_id: u(25) },
        ),
        (
            "revert_scenario_apply",
            WriteCommand::RevertScenarioApply { scenario_id: u(25) },
        ),
        (
            "create_tag",
            WriteCommand::CreateTag {
                id: tag(17),
                name: "trip".into(),
                color: Some("#123456".into()),
            },
        ),
        (
            "set_tags",
            WriteCommand::SetTags {
                transaction_id: txn(10),
                tag_ids: vec![tag(17), tag(18)],
            },
        ),
        (
            "set_note",
            WriteCommand::SetNote {
                transaction_id: txn(10),
                note: None,
            },
        ),
        (
            "set_splits",
            WriteCommand::SetSplits {
                transaction_id: txn(10),
                lines: vec![
                    SplitLineInput {
                        amount: usd(-700),
                        category_id: Some(category(16)),
                        note: Some("part".into()),
                        tag_ids: vec![tag(17)],
                    },
                    SplitLineInput {
                        amount: usd(-300),
                        category_id: None,
                        note: None,
                        tag_ids: vec![],
                    },
                ],
            },
        ),
        (
            "set_household_timezone",
            WriteCommand::SetHouseholdTimezone {
                timezone: "America/Chicago".into(),
            },
        ),
        (
            "record_balance_assertion",
            WriteCommand::RecordBalanceAssertion {
                id: u(70),
                account_id: account(1),
                amount: Money::new(123_456, Currency::Usd),
                as_of: chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
            },
        ),
        (
            "record_connector_balance",
            WriteCommand::RecordConnectorBalance {
                account_id: account(1),
                observed_at: chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
                amount: Money::new(125_000, Currency::Usd),
            },
        ),
        (
            "set_household_name",
            WriteCommand::SetHouseholdName {
                name: "The Garcia household".into(),
            },
        ),
        (
            "add_household_member",
            WriteCommand::AddHouseholdMember {
                member_id: u(60),
                display_name: "Sam".into(),
            },
        ),
        (
            "rename_household_member",
            WriteCommand::RenameHouseholdMember {
                member_id: u(60),
                display_name: "Samuel".into(),
            },
        ),
        (
            "set_member_profile",
            WriteCommand::SetMemberProfile {
                member_id: u(60),
                initials: Some("SG".into()),
                color: Some("#3366CC".into()),
            },
        ),
        // The primary's id is derived from the vault; the oracle fills it in.
        (
            "transfer_primary",
            WriteCommand::TransferPrimary {
                from_member_id: u(0),
                to_member_id: u(60),
            },
        ),
        (
            "remove_household_member",
            WriteCommand::RemoveHouseholdMember { member_id: u(0) },
        ),
        (
            "restore_household_member",
            WriteCommand::RestoreHouseholdMember { member_id: u(0) },
        ),
        (
            "record_assumption_event",
            WriteCommand::RecordAssumptionEvent {
                event: assumption_event(80, AssumptionKind::OneTimeEvent),
            },
        ),
        (
            "supersede_assumption_event",
            WriteCommand::SupersedeAssumptionEvent {
                id: u(80),
                replacement: assumption_event(81, AssumptionKind::OneTimeEvent),
            },
        ),
        (
            "clear_assumption_event",
            WriteCommand::ClearAssumptionEvent { id: u(81) },
        ),
        (
            "create_scenario",
            WriteCommand::CreateScenario {
                scenario: NewScenario {
                    id: u(90),
                    name: "Move to a smaller place".to_owned(),
                    description: Some("Rent drops after the lease ends".to_owned()),
                },
            },
        ),
        (
            "set_scenario_status",
            WriteCommand::SetScenarioStatus {
                id: u(90),
                status: ScenarioStatus::Active,
            },
        ),
        (
            "rename_scenario",
            WriteCommand::RenameScenario {
                id: u(90),
                name: "Smaller place".to_owned(),
            },
        ),
        (
            "set_scenario_expiry",
            WriteCommand::SetScenarioExpiry {
                id: u(90),
                expires_on: Some(NaiveDate::from_ymd_opt(2027, 6, 30).unwrap()),
            },
        ),
        (
            "clone_scenario",
            WriteCommand::CloneScenario {
                source_id: u(90),
                new_id: u(91),
                name: "Smaller place, second try".to_owned(),
                event_ids: vec![u(92), u(93)],
            },
        ),
        (
            "delete_scenario",
            WriteCommand::DeleteScenario { id: u(90) },
        ),
        (
            "reinstate_scenario",
            WriteCommand::ReinstateScenario { id: u(90) },
        ),
    ]
}

/// A sample assumption event: a base `one_time_event` override.
fn assumption_event(n: u128, kind: AssumptionKind) -> NewAssumptionEvent {
    NewAssumptionEvent {
        id: u(n),
        kind,
        target_entity_type: None,
        target_entity_id: None,
        params_json:
            r#"{"amount_minor":"50000","currency":"USD","label":"Bonus","occurs_on":"2026-11-15"}"#
                .to_owned(),
        source: AssumptionSource::UserOverride,
        scenario_id: None,
    }
}
