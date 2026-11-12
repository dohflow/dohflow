//! The class-1b artifacts a command's payload references (`docs/architecture/vault-format.md` C.9;
//! `personal-cfo-0g528.2`, D4): the logical ids the producer maps to artifact ids at seal time, and
//! the receiver checks the delta against (stage 9(b) set equality).
//!
//! The match is **exhaustive with no wildcard arm**, so a new `WriteCommand` variant is a compile
//! error here and its author decides what it references; `tests/payload_reference_lint.rs` keeps
//! naming the fields that point at class-1b rows, and a test below pins the list of variants that
//! carry such an id.

use sync_core::artifact_closure::PayloadRef;

use crate::WriteCommand;

/// The class-1b objects `command` references: a source record the receiver must hold (so the batch
/// that holds it is in the delta) or an attachment. Empty for a command that references only class-1
/// rows.
#[must_use]
pub fn closure_references(command: &WriteCommand) -> Vec<PayloadRef> {
    match command {
        // Names a source record whose rows must exist on the receiver before `apply` (it writes the
        // provenance link): the delta carries the batch that holds it.
        WriteCommand::CommitImportedTransaction {
            source_record_id, ..
        } => {
            vec![PayloadRef::Record(*source_record_id.as_uuid().as_bytes())]
        }
        // The local import path. It creates and updates class-1b rows of this device (`write_set`
        // `none` in `table_columns.toml`): a batch travels as an artifact when a commit first
        // references it, never as the payload of these commands, so none names an existing artifact.
        WriteCommand::CreateSourceBatch { .. }
        | WriteCommand::AttachSourceRecord { .. }
        | WriteCommand::UpdateBatchState { .. } => Vec::new(),
        // Retired history: decodable, produced by nothing, and naming staging rows (class 3).
        WriteCommand::CommitStaged { .. } | WriteCommand::SkipStaged { .. } => Vec::new(),
        // Every other command references class-1 rows only (accounts, categories, scenarios, ...),
        // or nothing: the payload-reference lint names any field that points elsewhere.
        WriteCommand::CreateAccount { .. }
        | WriteCommand::UpdateAccount { .. }
        | WriteCommand::ArchiveAccount(_)
        | WriteCommand::ReinstateAccount(_)
        | WriteCommand::SetAccountSubtype { .. }
        | WriteCommand::SetAccountNote { .. }
        | WriteCommand::SetAccountLink { .. }
        | WriteCommand::SetDebtTerms { .. }
        | WriteCommand::SetCardStatementBalance { .. }
        | WriteCommand::RecordTransaction { .. }
        | WriteCommand::ConfirmObligationEarly { .. }
        | WriteCommand::UnconfirmObligation { .. }
        | WriteCommand::ConvertUnexplainedToTransaction { .. }
        | WriteCommand::Transfer { .. }
        | WriteCommand::CreateRecurringTransfer { .. }
        | WriteCommand::DeleteRecurringTransfer(_)
        | WriteCommand::ReinstateRecurringTransfer(_)
        | WriteCommand::CreateIncomeSource { .. }
        | WriteCommand::UpdateIncomeSource { .. }
        | WriteCommand::DeleteIncomeSource(_)
        | WriteCommand::ReinstateIncomeSource(_)
        | WriteCommand::ArchiveIncomeSource(_)
        | WriteCommand::RestoreIncomeSource(_)
        | WriteCommand::CreateRecurringBill { .. }
        | WriteCommand::SetBillAutopay { .. }
        | WriteCommand::UpdateRecurringBill { .. }
        | WriteCommand::DeleteRecurringBill { .. }
        | WriteCommand::ReinstateRecurringBill { .. }
        | WriteCommand::ArchiveRecurringBill(_)
        | WriteCommand::LinkBillToLoan { .. }
        | WriteCommand::UnlinkBillFromLoan { .. }
        | WriteCommand::SetBillNotLoanPayment { .. }
        | WriteCommand::CreateConnectorConnection { .. }
        | WriteCommand::UpsertConnectorLink { .. }
        | WriteCommand::SetConnectorLinkAccount { .. }
        | WriteCommand::DeleteConnectorConnection { .. }
        | WriteCommand::RestoreRecurringBill(_)
        | WriteCommand::SnoozeInboxItem { .. }
        | WriteCommand::DismissInboxItem { .. }
        | WriteCommand::DismissRecurringSuggestion { .. }
        | WriteCommand::CreateCategory { .. }
        | WriteCommand::UpdateCategory { .. }
        | WriteCommand::MoveCategory { .. }
        | WriteCommand::SetHouseholdTimezone { .. }
        | WriteCommand::RecordBalanceAssertion { .. }
        | WriteCommand::RecordConnectorBalance { .. }
        | WriteCommand::SetHouseholdName { .. }
        | WriteCommand::AddHouseholdMember { .. }
        | WriteCommand::RenameHouseholdMember { .. }
        | WriteCommand::TransferPrimary { .. }
        | WriteCommand::RemoveHouseholdMember { .. }
        | WriteCommand::RestoreHouseholdMember { .. }
        | WriteCommand::SetMemberProfile { .. }
        | WriteCommand::RecordAssumptionEvent { .. }
        | WriteCommand::SupersedeAssumptionEvent { .. }
        | WriteCommand::ClearAssumptionEvent { .. }
        | WriteCommand::CreateScenario { .. }
        | WriteCommand::SetScenarioStatus { .. }
        | WriteCommand::RenameScenario { .. }
        | WriteCommand::SetScenarioExpiry { .. }
        | WriteCommand::CloneScenario { .. }
        | WriteCommand::DeleteScenario { .. }
        | WriteCommand::ReinstateScenario { .. }
        | WriteCommand::ArchiveCategory(_)
        | WriteCommand::ReinstateCategory(_)
        | WriteCommand::RecategorizeTransaction { .. }
        | WriteCommand::VoidTransaction { .. }
        | WriteCommand::MarkReviewed { .. }
        | WriteCommand::ApplyScenario { .. }
        | WriteCommand::RevertScenarioApply { .. }
        | WriteCommand::RevertScenarioApplyConfirmed { .. }
        | WriteCommand::CreateTag { .. }
        | WriteCommand::SetTags { .. }
        | WriteCommand::SetNote { .. }
        | WriteCommand::SetSplits { .. }
        | WriteCommand::MintMerchantAnchors { .. }
        | WriteCommand::ApplyMerchantMemory { .. }
        | WriteCommand::SetUserSetting { .. } => Vec::new(),
    }
}
