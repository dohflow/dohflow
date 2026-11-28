//! The scenario list of `apply_rejections.rs` (moved here so the engine pin can pin the same
//! outcomes): setup steps that must succeed and the deterministic rejections, in order on one vault.

use chrono::NaiveDate;
use core_ledger::{
    Account, AccountFlags, AccountId, AccountSubtype, CashflowRole, CategoryId, LedgerAccountId,
    SourceBatchId, SourceRecordId, TransactionId,
};
use core_money::{Currency, Money};
use db_worker::{
    ActorType, AssumptionKind, AssumptionSource, CommandMeta, DbError, DbWorker, IssuedAt,
    MerchantAnchor, MerchantAssignment, NewAssumptionEvent, NewScenario, RejectCode,
    ScenarioStatus, WriteCommand,
};
use std::path::Path;
use uuid::Uuid;

use super::KEY;

pub fn u(n: u128) -> Uuid {
    Uuid::from_u128(0x0198_c2c1_5f0a_7c3d_8a4e_9b1d_0000_0000 + n)
}
pub fn account(n: u128) -> AccountId {
    AccountId::from_uuid(u(n))
}
pub fn category(n: u128) -> CategoryId {
    CategoryId::from_uuid(u(n))
}

pub const ISSUED_MICROS: i64 = 1_000_000_000_000_000;

pub fn meta(n: u128) -> CommandMeta {
    CommandMeta {
        command_id: u(1_000 + n),
        correlation_id: u(2_000 + n),
        causation_id: None,
        actor_type: ActorType::User,
        actor_id: "tester".to_owned(),
        idempotency_key: format!("00000000-0000-4000-8000-{:012x}", 3_000 + n),
        issued_at: IssuedAt::from_unix_micros(ISSUED_MICROS),
    }
}

pub fn create_account(n: u128, role: CashflowRole, currency: Currency) -> WriteCommand {
    WriteCommand::CreateAccount {
        account: Box::new(Account::new(
            account(n),
            LedgerAccountId::from_uuid(u(500 + n)),
            format!("Account {n}"),
            role,
            currency,
            AccountFlags::default(),
        )),
        opening_balance: None,
    }
}

pub fn event(n: u128, scenario: Option<u128>, params_json: &str) -> NewAssumptionEvent {
    NewAssumptionEvent {
        id: u(n),
        kind: AssumptionKind::OneTimeEvent,
        target_entity_type: None,
        target_entity_id: None,
        params_json: params_json.to_owned(),
        source: AssumptionSource::UserOverride,
        scenario_id: scenario.map(u),
    }
}

pub fn record_event(n: u128, scenario: Option<u128>, params_json: &str) -> WriteCommand {
    WriteCommand::RecordAssumptionEvent {
        event: event(n, scenario, params_json),
    }
}

pub fn supersede_event(old: u128, new: u128) -> WriteCommand {
    WriteCommand::SupersedeAssumptionEvent {
        id: u(old),
        replacement: event(new, None, "{}"),
    }
}

pub fn clear_event(n: u128) -> WriteCommand {
    WriteCommand::ClearAssumptionEvent { id: u(n) }
}

pub fn create_scenario(n: u128, name: &str) -> WriteCommand {
    WriteCommand::CreateScenario {
        scenario: NewScenario {
            id: u(n),
            name: name.to_owned(),
            description: None,
        },
    }
}

pub fn clone_scenario(source: u128, new: u128, events: &[u128]) -> WriteCommand {
    WriteCommand::CloneScenario {
        source_id: u(source),
        new_id: u(new),
        name: "Copy".to_owned(),
        event_ids: events.iter().copied().map(u).collect(),
    }
}

pub fn assertion(id: u128, account_id: AccountId, amount: Money) -> WriteCommand {
    WriteCommand::RecordBalanceAssertion {
        id: u(id),
        account_id,
        amount,
        as_of: NaiveDate::from_ymd_opt(2001, 9, 1).unwrap(),
    }
}

pub fn create_category(n: u128, parent: Option<u128>, name: &str) -> WriteCommand {
    WriteCommand::CreateCategory {
        id: category(n),
        parent_id: parent.map(category),
        name: name.to_owned(),
        category_type: "expense".to_owned(),
        color: None,
        icon: None,
    }
}

/// An import commit of `minor` units to `account_id`, citing source record 151, with everything else
/// valid; the scenarios below change one field at a time.
pub fn imported(n: u128, account_id: AccountId, amount: Money) -> WriteCommand {
    WriteCommand::CommitImportedTransaction {
        transaction_id: TransactionId::from_uuid(u(n)),
        account_id,
        source_record_id: SourceRecordId::from_uuid(u(151)),
        occurred_at: chrono::DateTime::from_timestamp_micros(ISSUED_MICROS).unwrap(),
        amount,
        counterparty: Some("corner cafe".to_owned()),
        memo: Some("CORNER CAFE #4".to_owned()),
        transaction_date: None,
        category_id: None,
        forced: false,
        identity: None,
    }
}

/// `command` (an import commit) with the cross-device identity `claim` attached.
fn claiming(command: WriteCommand, claim: db_worker::ImportIdentityClaim) -> WriteCommand {
    match command {
        WriteCommand::CommitImportedTransaction {
            transaction_id,
            account_id,
            source_record_id,
            occurred_at,
            amount,
            counterparty,
            memo,
            transaction_date,
            category_id,
            forced,
            ..
        } => WriteCommand::CommitImportedTransaction {
            transaction_id,
            account_id,
            source_record_id,
            occurred_at,
            amount,
            counterparty,
            memo,
            transaction_date,
            category_id,
            forced,
            identity: Some(claim),
        },
        _ => unreachable!("imported builds a CommitImportedTransaction"),
    }
}

/// What a scenario expects: a rejection with a code, or success.
#[derive(Clone, Copy)]
pub enum Expect {
    Reject(RejectCode),
    Ok,
}

pub struct Step {
    pub label: &'static str,
    pub command: WriteCommand,
    pub expect: Expect,
}

pub fn step(label: &'static str, command: WriteCommand, expect: Expect) -> Step {
    Step {
        label,
        command,
        expect,
    }
}

use Expect::{Ok as Fine, Reject};
use RejectCode::{PayloadInvalid, ReferenceMissing, StateConflict};

/// The scenarios, in order on one vault: setup steps (expected to succeed) and the rejections.
pub fn scenarios() -> Vec<Step> {
    vec![
        // ---- setup ----
        step(
            "liquid",
            create_account(1, CashflowRole::LiquidCash, Currency::Usd),
            Fine,
        ),
        step(
            "asset",
            create_account(2, CashflowRole::RealAsset, Currency::Usd),
            Fine,
        ),
        step(
            "loan",
            create_account(3, CashflowRole::LoanLiability, Currency::Usd),
            Fine,
        ),
        step(
            "second asset",
            create_account(4, CashflowRole::RealAsset, Currency::Usd),
            Fine,
        ),
        step(
            "link asset to loan",
            WriteCommand::SetAccountLink {
                asset_id: account(2),
                liability_id: Some(account(3)),
            },
            Fine,
        ),
        step(
            "a recorded transaction",
            WriteCommand::RecordTransaction {
                transaction_id: TransactionId::from_uuid(u(30)),
                account_id: account(1),
                amount: Money::new(-1_000, Currency::Usd),
                occurred_at: chrono::DateTime::from_timestamp_micros(ISSUED_MICROS).unwrap(),
            },
            Fine,
        ),
        step("group", create_category(10, None, "Home"), Fine),
        step("child", create_category(11, Some(10), "Utilities"), Fine),
        // ---- accounts.rs ----
        step(
            "an opening balance in another currency than the account",
            WriteCommand::CreateAccount {
                account: Box::new(Account::new(
                    account(20),
                    LedgerAccountId::from_uuid(u(520)),
                    "Mismatch",
                    CashflowRole::LiquidCash,
                    Currency::Usd,
                    AccountFlags::default(),
                )),
                opening_balance: Some(Money::new(100, Currency::Eur)),
            },
            Reject(PayloadInvalid),
        ),
        step(
            "a subtype of an unknown account",
            WriteCommand::SetAccountSubtype {
                id: account(99),
                subtype: Some(AccountSubtype::Checking),
            },
            Reject(ReferenceMissing),
        ),
        step(
            "a subtype that does not belong to the account's role",
            WriteCommand::SetAccountSubtype {
                id: account(1),
                subtype: Some(AccountSubtype::CreditCard),
            },
            Reject(StateConflict),
        ),
        step(
            "a link from an unknown asset",
            WriteCommand::SetAccountLink {
                asset_id: account(99),
                liability_id: None,
            },
            Reject(ReferenceMissing),
        ),
        step(
            "a link from an account that is not a real asset",
            WriteCommand::SetAccountLink {
                asset_id: account(1),
                liability_id: Some(account(3)),
            },
            Reject(StateConflict),
        ),
        step(
            "a link of an asset to itself",
            WriteCommand::SetAccountLink {
                asset_id: account(4),
                liability_id: Some(account(4)),
            },
            Reject(PayloadInvalid),
        ),
        step(
            "a link to an account that is not a loan",
            WriteCommand::SetAccountLink {
                asset_id: account(4),
                liability_id: Some(account(1)),
            },
            Reject(StateConflict),
        ),
        step(
            "a link to an unknown liability",
            WriteCommand::SetAccountLink {
                asset_id: account(4),
                liability_id: Some(account(98)),
            },
            Reject(ReferenceMissing),
        ),
        step(
            "a loan already linked to another asset",
            WriteCommand::SetAccountLink {
                asset_id: account(4),
                liability_id: Some(account(3)),
            },
            Reject(StateConflict),
        ),
        // ---- categorization_cmds.rs ----
        step(
            "a category with an empty name",
            create_category(12, None, "   "),
            Reject(PayloadInvalid),
        ),
        step(
            "a category under an unknown parent",
            create_category(13, Some(98), "Orphan"),
            Reject(ReferenceMissing),
        ),
        step(
            "an update to an empty name",
            WriteCommand::UpdateCategory {
                id: category(11),
                name: String::new(),
                color: None,
                icon: None,
            },
            Reject(PayloadInvalid),
        ),
        step(
            "an update of an unknown category",
            WriteCommand::UpdateCategory {
                id: category(97),
                name: "Ghost".to_owned(),
                color: None,
                icon: None,
            },
            Reject(ReferenceMissing),
        ),
        step(
            "a move under an unknown parent",
            WriteCommand::MoveCategory {
                id: category(11),
                new_parent_id: Some(category(96)),
            },
            Reject(ReferenceMissing),
        ),
        step(
            "a move that would create a cycle",
            WriteCommand::MoveCategory {
                id: category(10),
                new_parent_id: Some(category(11)),
            },
            Reject(StateConflict),
        ),
        step(
            "archiving an unknown category",
            WriteCommand::ArchiveCategory(category(95)),
            Reject(ReferenceMissing),
        ),
        step(
            "reinstating an unknown category",
            WriteCommand::ReinstateCategory(category(94)),
            Reject(ReferenceMissing),
        ),
        step(
            "recategorizing a real transaction to an unknown category",
            WriteCommand::RecategorizeTransaction {
                transaction_id: TransactionId::from_uuid(u(30)),
                category_id: Some(category(92)),
            },
            Reject(ReferenceMissing),
        ),
        step(
            "recategorizing an unknown transaction",
            WriteCommand::RecategorizeTransaction {
                transaction_id: TransactionId::from_uuid(u(93)),
                category_id: None,
            },
            Reject(ReferenceMissing),
        ),
        step(
            "recording an assumption event",
            record_event(70, None, "{}"),
            Fine,
        ),
        step(
            "an assumption event whose parameters are not a JSON object",
            record_event(71, None, "[1]"),
            Reject(PayloadInvalid),
        ),
        step(
            "an assumption event in an unknown scenario",
            record_event(72, Some(97), "{}"),
            Reject(ReferenceMissing),
        ),
        step(
            "an assumption event id that is already in use",
            record_event(70, None, "{}"),
            Reject(StateConflict),
        ),
        step(
            "an assumption event superseding itself",
            supersede_event(70, 70),
            Reject(PayloadInvalid),
        ),
        step(
            "superseding an unknown assumption event",
            supersede_event(98, 73),
            Reject(ReferenceMissing),
        ),
        step("superseding an active event", supersede_event(70, 74), Fine),
        step(
            "superseding an event that is no longer active",
            supersede_event(70, 75),
            Reject(StateConflict),
        ),
        step(
            "recording a base event to supersede",
            record_event(76, None, "{}"),
            Fine,
        ),
        step(
            "superseding a base event with a scenario overlay",
            WriteCommand::SupersedeAssumptionEvent {
                id: u(76),
                replacement: event(77, Some(97), "{}"),
            },
            Reject(StateConflict),
        ),
        step("clearing an active event", clear_event(74), Fine),
        step(
            "clearing an event that is already cleared",
            clear_event(74),
            Fine,
        ),
        step(
            "clearing an event that was superseded",
            clear_event(70),
            Reject(StateConflict),
        ),
        step(
            "an assertion on a real account",
            assertion(78, account(1), Money::new(100, Currency::Usd)),
            Fine,
        ),
        step(
            "an assertion on an unknown account",
            assertion(79, account(96), Money::new(100, Currency::Usd)),
            Reject(ReferenceMissing),
        ),
        step(
            "an assertion in a currency other than the account's",
            assertion(79, account(1), Money::new(100, Currency::Eur)),
            Reject(StateConflict),
        ),
        step(
            "an assertion whose id is already in use",
            assertion(78, account(1), Money::new(200, Currency::Usd)),
            Reject(StateConflict),
        ),
        step(
            "a connector reading dated after the command's household-local day",
            WriteCommand::RecordConnectorBalance {
                account_id: account(1),
                observed_at: NaiveDate::from_ymd_opt(2030, 1, 1).unwrap(),
                amount: Money::new(100, Currency::Usd),
            },
            Reject(StateConflict),
        ),
        step("creating a scenario", create_scenario(110, " Move "), Fine),
        step(
            "a scenario with a blank name",
            create_scenario(112, "  "),
            Reject(PayloadInvalid),
        ),
        step(
            "a scenario id that is already in use",
            create_scenario(110, "Again"),
            Reject(StateConflict),
        ),
        step(
            "renaming an unknown scenario",
            WriteCommand::RenameScenario {
                id: u(114),
                name: "Nope".to_owned(),
            },
            Reject(ReferenceMissing),
        ),
        step(
            "renaming a scenario",
            WriteCommand::RenameScenario {
                id: u(110),
                name: "Smaller place".to_owned(),
            },
            Fine,
        ),
        step(
            "an expiry before the supported years",
            WriteCommand::SetScenarioExpiry {
                id: u(110),
                expires_on: NaiveDate::from_ymd_opt(1899, 12, 31),
            },
            Reject(PayloadInvalid),
        ),
        step(
            "setting an expiry",
            WriteCommand::SetScenarioExpiry {
                id: u(110),
                expires_on: NaiveDate::from_ymd_opt(2030, 1, 1),
            },
            Fine,
        ),
        step(
            "recording the first event of a scenario",
            record_event(120, Some(110), "{}"),
            Fine,
        ),
        step(
            "recording the second event of a scenario",
            record_event(121, Some(110), "{}"),
            Fine,
        ),
        step(
            "cloning events listed out of order",
            clone_scenario(110, 111, &[121, 120]),
            Reject(PayloadInvalid),
        ),
        step(
            "cloning more events than one command can mint",
            WriteCommand::CloneScenario {
                source_id: u(110),
                new_id: u(111),
                name: "Copy".to_owned(),
                event_ids: (0..=4096).map(|n| u(10_000 + n)).collect(),
            },
            Reject(PayloadInvalid),
        ),
        step(
            "cloning an unknown scenario",
            clone_scenario(114, 111, &[]),
            Reject(ReferenceMissing),
        ),
        step(
            "cloning into an id that is already in use",
            clone_scenario(110, 110, &[120]),
            Reject(StateConflict),
        ),
        step(
            "cloning an event that is not the source's",
            clone_scenario(110, 111, &[70]),
            Reject(ReferenceMissing),
        ),
        step(
            "cloning a scenario",
            clone_scenario(110, 111, &[120, 121]),
            Fine,
        ),
        step(
            "deleting an unknown scenario",
            WriteCommand::DeleteScenario { id: u(114) },
            Reject(ReferenceMissing),
        ),
        step(
            "reinstating an unknown scenario",
            WriteCommand::ReinstateScenario { id: u(114) },
            Reject(ReferenceMissing),
        ),
        step(
            "applying an unknown scenario",
            WriteCommand::ApplyScenario {
                scenario_id: u(114),
            },
            Reject(ReferenceMissing),
        ),
        step(
            "applying a scenario",
            WriteCommand::ApplyScenario {
                scenario_id: u(110),
            },
            Fine,
        ),
        step(
            "applying a scenario that is already applied",
            WriteCommand::ApplyScenario {
                scenario_id: u(110),
            },
            Reject(StateConflict),
        ),
        step(
            "reverting an unknown scenario",
            WriteCommand::RevertScenarioApply {
                scenario_id: u(114),
            },
            Reject(ReferenceMissing),
        ),
        step(
            "reverting an applied scenario",
            WriteCommand::RevertScenarioApply {
                scenario_id: u(110),
            },
            Fine,
        ),
        step(
            "reverting a scenario that is not applied",
            WriteCommand::RevertScenarioApply {
                scenario_id: u(110),
            },
            Reject(StateConflict),
        ),
        step(
            "creating a scenario with nothing in it",
            create_scenario(115, "Empty"),
            Fine,
        ),
        step(
            "applying a scenario with no active changes",
            WriteCommand::ApplyScenario {
                scenario_id: u(115),
            },
            Reject(StateConflict),
        ),
        step(
            "deleting a scenario",
            WriteCommand::DeleteScenario { id: u(110) },
            Fine,
        ),
        step(
            "setting the status of a deleted scenario",
            WriteCommand::SetScenarioStatus {
                id: u(110),
                status: ScenarioStatus::Archived,
            },
            Reject(StateConflict),
        ),
        step(
            "recording an event into a deleted scenario",
            record_event(122, Some(110), "{}"),
            Reject(StateConflict),
        ),
        step(
            "applying a deleted scenario",
            WriteCommand::ApplyScenario {
                scenario_id: u(110),
            },
            Reject(StateConflict),
        ),
        step(
            "deleting a scenario that is already deleted",
            WriteCommand::DeleteScenario { id: u(110) },
            Fine,
        ),
        step(
            "reinstating a deleted scenario",
            WriteCommand::ReinstateScenario { id: u(110) },
            Fine,
        ),
        // ---- undoing an apply (personal-cfo-5ie.12) ----
        step(
            "undoing with the confirmed changes out of order",
            revert_confirmed(110, &[902, 901]),
            Reject(PayloadInvalid),
        ),
        step(
            "undoing an unknown scenario",
            revert_confirmed(114, &[]),
            Reject(ReferenceMissing),
        ),
        step(
            "undoing a scenario that is not applied",
            revert_confirmed(110, &[]),
            Reject(StateConflict),
        ),
        step(
            "creating the first of two stacked scenarios",
            create_scenario(130, "First"),
            Fine,
        ),
        step(
            "creating the second of two stacked scenarios",
            create_scenario(131, "Second"),
            Fine,
        ),
        step("recording a base rate", record_rate(140, None, 300), Fine),
        step(
            "recording the first scenario's rate",
            record_rate(141, Some(130), 400),
            Fine,
        ),
        step(
            "recording the second scenario's rate",
            record_rate(142, Some(131), 500),
            Fine,
        ),
        step(
            "applying the first scenario",
            WriteCommand::ApplyScenario {
                scenario_id: u(130),
            },
            Fine,
        ),
        step(
            "applying the second scenario over it",
            WriteCommand::ApplyScenario {
                scenario_id: u(131),
            },
            Fine,
        ),
        step(
            "undoing the first scenario while the second sits on top",
            revert_confirmed(130, &[]),
            Reject(StateConflict),
        ),
        step(
            "undoing with a change that was never made",
            revert_confirmed(131, &[901]),
            Reject(StateConflict),
        ),
        step(
            "undoing the second scenario",
            revert_confirmed(131, &[]),
            Fine,
        ),
        step(
            "undoing the first scenario",
            revert_confirmed(130, &[]),
            Fine,
        ),
        // ---- apply/ingestion_cmds.rs (personal-cfo-0g528.1) ----
        step(
            "a source batch",
            WriteCommand::CreateSourceBatch {
                id: SourceBatchId::from_uuid(u(150)),
                source_type: "csv".to_owned(),
                source_name: Some("statement.csv".to_owned()),
                file_fingerprint: None,
                parser_version: None,
                connector_connection_id: None,
            },
            Fine,
        ),
        step(
            "a source record",
            WriteCommand::AttachSourceRecord {
                id: SourceRecordId::from_uuid(u(151)),
                batch_id: SourceBatchId::from_uuid(u(150)),
                external_id: None,
                source_hash: "sha256:row".to_owned(),
                normalized_json: "{}".to_owned(),
                parse_confidence_bps: None,
            },
            Fine,
        ),
        step(
            "an import commit of a zero amount",
            imported(160, account(1), Money::new(0, Currency::Usd)),
            Reject(PayloadInvalid),
        ),
        step(
            "an import commit with an over-long memo",
            match imported(161, account(1), Money::new(-500, Currency::Usd)) {
                WriteCommand::CommitImportedTransaction {
                    transaction_id,
                    account_id,
                    source_record_id,
                    occurred_at,
                    amount,
                    counterparty,
                    transaction_date,
                    category_id,
                    forced,
                    ..
                } => WriteCommand::CommitImportedTransaction {
                    transaction_id,
                    account_id,
                    source_record_id,
                    occurred_at,
                    amount,
                    counterparty,
                    memo: Some("m".repeat(db_worker::MAX_IMPORT_TEXT_BYTES + 1)),
                    transaction_date,
                    category_id,
                    forced,
                    identity: None,
                },
                _ => unreachable!("imported builds a CommitImportedTransaction"),
            },
            Reject(PayloadInvalid),
        ),
        step(
            "an import commit dated outside the supported years",
            match imported(162, account(1), Money::new(-500, Currency::Usd)) {
                WriteCommand::CommitImportedTransaction {
                    transaction_id,
                    account_id,
                    source_record_id,
                    amount,
                    counterparty,
                    memo,
                    transaction_date,
                    category_id,
                    forced,
                    ..
                } => WriteCommand::CommitImportedTransaction {
                    transaction_id,
                    account_id,
                    source_record_id,
                    occurred_at: chrono::DateTime::parse_from_rfc3339("3001-01-01T12:00:00Z")
                        .unwrap()
                        .to_utc(),
                    amount,
                    counterparty,
                    memo,
                    transaction_date,
                    category_id,
                    forced,
                    identity: None,
                },
                _ => unreachable!("imported builds a CommitImportedTransaction"),
            },
            Reject(PayloadInvalid),
        ),
        step(
            "an import commit to an unknown account",
            imported(163, account(97), Money::new(-500, Currency::Usd)),
            Reject(ReferenceMissing),
        ),
        step(
            "an import commit in another currency than its account",
            imported(164, account(1), Money::new(-500, Currency::Eur)),
            Reject(PayloadInvalid),
        ),
        step(
            "an import commit naming an unknown category",
            match imported(165, account(1), Money::new(-500, Currency::Usd)) {
                WriteCommand::CommitImportedTransaction {
                    transaction_id,
                    account_id,
                    source_record_id,
                    occurred_at,
                    amount,
                    counterparty,
                    memo,
                    transaction_date,
                    forced,
                    ..
                } => WriteCommand::CommitImportedTransaction {
                    transaction_id,
                    account_id,
                    source_record_id,
                    occurred_at,
                    amount,
                    counterparty,
                    memo,
                    transaction_date,
                    category_id: Some(category(96)),
                    forced,
                    identity: None,
                },
                _ => unreachable!("imported builds a CommitImportedTransaction"),
            },
            Reject(ReferenceMissing),
        ),
        step(
            "an import commit citing an unknown source record",
            match imported(166, account(1), Money::new(-500, Currency::Usd)) {
                WriteCommand::CommitImportedTransaction {
                    transaction_id,
                    account_id,
                    occurred_at,
                    amount,
                    counterparty,
                    memo,
                    transaction_date,
                    category_id,
                    forced,
                    ..
                } => WriteCommand::CommitImportedTransaction {
                    transaction_id,
                    account_id,
                    source_record_id: SourceRecordId::from_uuid(u(152)),
                    occurred_at,
                    amount,
                    counterparty,
                    memo,
                    transaction_date,
                    category_id,
                    forced,
                    identity: None,
                },
                _ => unreachable!("imported builds a CommitImportedTransaction"),
            },
            Reject(ReferenceMissing),
        ),
        step(
            "a valid import commit",
            imported(167, account(1), Money::new(-500, Currency::Usd)),
            Fine,
        ),
        step(
            "the same import commit delivered again",
            imported(167, account(1), Money::new(-500, Currency::Usd)),
            Fine,
        ),
        // ---- apply/merchant_cmds.rs (personal-cfo-5ymg8.5) ----
        step("minting no anchors", mint(&[]), Reject(PayloadInvalid)),
        step(
            "minting past the per-command bound",
            WriteCommand::MintMerchantAnchors {
                anchors: (0..=db_worker::MAX_MERCHANT_COMMAND_ENTRIES)
                    .map(|n| anchor(&format!("K{n:05}")))
                    .collect(),
            },
            Reject(PayloadInvalid),
        ),
        step(
            "minting anchors out of key order",
            mint(&["ZED", "ABC"]),
            Reject(PayloadInvalid),
        ),
        step(
            "minting the same key twice in one command",
            mint(&["ABC", "ABC"]),
            Reject(PayloadInvalid),
        ),
        step("minting an empty key", mint(&[""]), Reject(PayloadInvalid)),
        step(
            "minting a key wider than the replicated key width",
            WriteCommand::MintMerchantAnchors {
                anchors: vec![anchor(&"K".repeat(db_worker::MAX_ALIAS_KEY_BYTES + 1))],
            },
            Reject(PayloadInvalid),
        ),
        step(
            "minting an anchor with a blank display name",
            WriteCommand::MintMerchantAnchors {
                anchors: vec![MerchantAnchor {
                    key: "ABC".to_owned(),
                    display_name: "  ".to_owned(),
                }],
            },
            Reject(PayloadInvalid),
        ),
        step("minting two anchors", mint(&["ABC", "ZED"]), Fine),
        step("minting a held key again skips it", mint(&["ABC"]), Fine),
        step(
            "applying memory to no transactions",
            WriteCommand::ApplyMerchantMemory {
                assignments: Vec::new(),
            },
            Reject(PayloadInvalid),
        ),
        step(
            "applying memory past the per-command bound",
            WriteCommand::ApplyMerchantMemory {
                assignments: (0..=db_worker::MAX_MERCHANT_COMMAND_ENTRIES as u128)
                    .map(|n| assignment(3_000 + n, 10, 10_000))
                    .collect(),
            },
            Reject(PayloadInvalid),
        ),
        step(
            "applying memory out of transaction order",
            WriteCommand::ApplyMerchantMemory {
                assignments: vec![assignment(31, 10, 10_000), assignment(30, 10, 10_000)],
            },
            Reject(PayloadInvalid),
        ),
        step(
            "applying memory with a confidence above 100%",
            WriteCommand::ApplyMerchantMemory {
                assignments: vec![assignment(30, 10, 10_001)],
            },
            Reject(PayloadInvalid),
        ),
        step(
            "applying memory to an unknown transaction",
            WriteCommand::ApplyMerchantMemory {
                assignments: vec![assignment(93, 10, 10_000)],
            },
            Reject(ReferenceMissing),
        ),
        step(
            "applying memory with an unknown category",
            WriteCommand::ApplyMerchantMemory {
                assignments: vec![assignment(30, 92, 10_000)],
            },
            Reject(ReferenceMissing),
        ),
        step(
            "applying memory fills the uncategorized transaction",
            WriteCommand::ApplyMerchantMemory {
                assignments: vec![assignment(30, 10, 7_500)],
            },
            Fine,
        ),
        step(
            "applying memory again skips the categorized transaction",
            WriteCommand::ApplyMerchantMemory {
                assignments: vec![assignment(30, 11, 9_000)],
            },
            Fine,
        ),
        // ---- apply/settings_cmds.rs (personal-cfo-5ymg8.7) ----
        step(
            "setting a key outside the household registry",
            set_user_setting("user.not_a_setting", "1"),
            Reject(PayloadInvalid),
        ),
        step(
            "setting a device key through the household command",
            set_user_setting("device.window", "1"),
            Reject(PayloadInvalid),
        ),
        step(
            "setting the minimum cash floor to a negative amount",
            set_user_setting("user.minimum_cash_floor_minor", "-5"),
            Reject(PayloadInvalid),
        ),
        step(
            "setting the reporting currency to an unsupported code",
            set_user_setting("user.reporting_currency", "XYZ"),
            Reject(PayloadInvalid),
        ),
        step(
            "setting the minimum cash floor",
            set_user_setting("user.minimum_cash_floor_minor", "250000"),
            Fine,
        ),
        step(
            "setting the same value again is a no-op write",
            set_user_setting("user.minimum_cash_floor_minor", "250000"),
            Fine,
        ),
        step(
            "clearing the comfort band's upper edge with the empty value",
            set_user_setting("user.comfort_band_upper_minor", ""),
            Fine,
        ),
        // ---- apply/import_identity.rs (personal-cfo-0g528.3): a claim is checked against its own
        // components and its own row, from the payload and the class-1 and class-1b base alone ----
        step(
            "an import commit whose identity is not its components' derived id",
            claiming(
                imported(168, account(1), Money::new(-500, Currency::Usd)),
                db_worker::ImportIdentityClaim {
                    identity_id: u(168),
                    components: db_worker::ImportIdentityComponents::Provider {
                        connection_id: u(169),
                        provider_account_key: "ACT-9".to_owned(),
                        provider_txn_id: "TXN-1".to_owned(),
                    },
                },
            ),
            Reject(PayloadInvalid),
        ),
        step(
            "an import commit whose identity names a malformed file fingerprint",
            claiming(
                imported(169, account(1), Money::new(-500, Currency::Usd)),
                db_worker::ImportIdentityClaim {
                    identity_id: u(169),
                    components: db_worker::ImportIdentityComponents::File {
                        file_fingerprint: "sha256:ABC".to_owned(),
                        row_ordinal: 1,
                    },
                },
            ),
            Reject(PayloadInvalid),
        ),
    ]
}

pub fn set_user_setting(key: &str, value: &str) -> WriteCommand {
    WriteCommand::SetUserSetting {
        key: key.to_owned(),
        value: value.to_owned(),
    }
}

pub fn anchor(key: &str) -> MerchantAnchor {
    MerchantAnchor {
        key: key.to_owned(),
        display_name: format!("Brand {key}"),
    }
}

pub fn mint(keys: &[&str]) -> WriteCommand {
    WriteCommand::MintMerchantAnchors {
        anchors: keys.iter().map(|k| anchor(k)).collect(),
    }
}

pub fn assignment(txn: u128, category_n: u128, confidence_bps: i64) -> MerchantAssignment {
    MerchantAssignment {
        transaction_id: TransactionId::from_uuid(u(txn)),
        category_id: category(category_n),
        confidence_bps,
    }
}

pub fn revert_confirmed(scenario: u128, confirmed: &[u128]) -> WriteCommand {
    WriteCommand::RevertScenarioApplyConfirmed {
        scenario_id: u(scenario),
        confirmed_descendants: confirmed.iter().copied().map(u).collect(),
    }
}

pub fn record_rate(n: u128, scenario: Option<u128>, bps: u32) -> WriteCommand {
    WriteCommand::RecordAssumptionEvent {
        event: NewAssumptionEvent {
            kind: AssumptionKind::InflationRate,
            ..event(n, scenario, &format!(r#"{{"annual_bps":{bps}}}"#))
        },
    }
}

pub fn open(path: &Path) -> DbWorker {
    DbWorker::open(path, KEY).unwrap()
}

/// One line per scenario: the rejection code, or the write-set hash of a success. Asserts the
/// expectation on the way.
pub fn run(worker: &DbWorker) -> Vec<String> {
    scenarios()
        .into_iter()
        .enumerate()
        .map(|(n, s)| {
            let outcome = worker.dispatch(meta(n as u128), s.command);
            let line = match (&outcome, s.expect) {
                (Ok(_), Fine) => {
                    let set = worker.last_write_set().unwrap();
                    // The whole write set is compared, hash included: a system ledger account a
                    // setup step creates has a content-derived id (personal-cfo-32fmp.3), the same
                    // on every device and in every environment.
                    let hash: String = set.hash.iter().map(|b| format!("{b:02x}")).collect();
                    format!("{}: ok rows={} hash={hash}", s.label, set.rows.len())
                }
                (Err(DbError::Rejected { code, .. }), Reject(expected)) => {
                    assert_eq!(*code, expected, "{}", s.label);
                    // A rejected command wrote nothing.
                    format!("{}: rejected {}", s.label, code.as_u16())
                }
                (other, _) => panic!("{}: unexpected outcome {other:?}", s.label),
            };
            line
        })
        .collect()
}
