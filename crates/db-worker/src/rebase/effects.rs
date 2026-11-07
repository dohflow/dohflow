//! The registry of pulled class-3 effects (personal-cfo-bxsbz.6; ADR 0073 Addendum D).
//!
//! A pulled command's apply writes device-local (class 3) rows as well as its class-1 write set: a
//! mapping change forgets this device's refresh watermark, a connection delete scrubs this
//! device's token. The scratch vault applied the command on the base image, whose class-3 rows are
//! stale, and the overlay then replaced them with the live vault's. So the class-3 half of every
//! pulled command is run again, on the overlaid scratch vault, before anything is swapped:
//!
//! ```text
//! apply_pulled -> rebase_local -> overlay_class3 -> apply_pulled_class3_effects -> (swap)
//! ```
//!
//! Each effect is idempotent and monotone (Addendum D): a command whose effect already ran leaves
//! the same rows, and an effect never reads class-3 state to decide a class-1 outcome. The effects
//! are the same functions the live apply arms call, so a device that rebuilds from a pulled command
//! does what a device that applied it live does.
//!
//! **Every command is classified.** [`effect_of`] matches every [`WriteCommand`] variant with no
//! wildcard, so a new command does not compile until someone decides what it does to class 3. A
//! test also scans the `apply/` sources for class-3 write statements and fails when a site is not
//! listed in [`CLASS3_APPLY_SITES`] against the effect that covers it (a text scan: a write made
//! through a helper in another module is outside it, as for the class-1 write lint).
//!
//! **The tombstone sweep.** After the effects, every tombstoned connection that still has a secret,
//! cursor or notice on the scratch vault is scrubbed in the same transaction, so a credential has no
//! crash window between the swap and a post-swap cleanup. The post-swap
//! `scrub_tombstoned_connector_secrets` stays as an idempotent backstop.

use rusqlite::params;
use uuid::Uuid;

use super::{RebaseError, Scratch};
use crate::apply::connector_cmds::{clear_link_cursor, remove_local_connection_state};
use crate::connectors::scrub_tombstoned_in;
use crate::{CommandMeta, WriteCommand};

/// What a pulled command does to class 3 beyond its class-1 write set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Class3Effect {
    /// Nothing: the command writes no class-3 row.
    None,
    /// The scratch vault's own apply wrote the class-3 rows and they are not in the base image:
    /// an audit row keyed by the command, or a `user.*` setting. The overlay never drops them.
    SurvivesOverlay,
    /// Forget this device's refresh watermark for the mapped account.
    ClearLinkCursor,
    /// Scrub this device's token, cursors and notice for the deleted connection, and record the
    /// token-removed audit row.
    ScrubDeletedConnection,
    /// The apply arm touches device-local import staging through a helper. The staged rows are
    /// device-local, so a pulled command from another device cannot name one of them; routing the
    /// staging effect is personal-cfo-5ymg8.6's.
    DeferredStaging,
}

/// The class-3 effect of `cmd`. No wildcard arm: a new [`WriteCommand`] must be classified here.
#[must_use]
#[allow(clippy::too_many_lines)]
pub(crate) const fn effect_of(cmd: &WriteCommand) -> Class3Effect {
    use WriteCommand as W;
    match cmd {
        W::SetConnectorLinkAccount { .. } => Class3Effect::ClearLinkCursor,
        W::DeleteConnectorConnection { .. } => Class3Effect::ScrubDeletedConnection,
        W::SetHouseholdTimezone { .. } | W::SetUserSetting { .. } => Class3Effect::SurvivesOverlay,
        W::CommitStaged { .. } | W::SkipStaged { .. } => Class3Effect::DeferredStaging,
        W::CreateAccount { .. }
        | W::UpdateAccount { .. }
        | W::SetAccountSubtype { .. }
        | W::SetAccountNote { .. }
        | W::SetAccountLink { .. }
        | W::SetDebtTerms { .. }
        | W::SetCardStatementBalance { .. }
        | W::RecordTransaction { .. }
        | W::ConfirmObligationEarly { .. }
        | W::UnconfirmObligation { .. }
        | W::ConvertUnexplainedToTransaction { .. }
        | W::Transfer { .. }
        | W::CreateRecurringTransfer { .. }
        | W::CreateIncomeSource { .. }
        | W::UpdateIncomeSource { .. }
        | W::CreateRecurringBill { .. }
        | W::SetBillAutopay { .. }
        | W::UpdateRecurringBill { .. }
        | W::DeleteRecurringBill { .. }
        | W::ReinstateRecurringBill { .. }
        | W::LinkBillToLoan { .. }
        | W::UnlinkBillFromLoan { .. }
        | W::SetBillNotLoanPayment { .. }
        | W::CreateConnectorConnection { .. }
        | W::UpsertConnectorLink { .. }
        | W::CreateSourceBatch { .. }
        | W::AttachSourceRecord { .. }
        | W::UpdateBatchState { .. }
        | W::CommitImportedTransaction { .. }
        | W::SnoozeInboxItem { .. }
        | W::DismissInboxItem { .. }
        | W::DismissRecurringSuggestion { .. }
        | W::CreateCategory { .. }
        | W::UpdateCategory { .. }
        | W::MoveCategory { .. }
        | W::RecategorizeTransaction { .. }
        | W::VoidTransaction { .. }
        | W::MarkReviewed { .. }
        | W::ApplyScenario { .. }
        | W::RevertScenarioApply { .. }
        | W::RevertScenarioApplyConfirmed { .. }
        | W::CreateTag { .. }
        | W::SetTags { .. }
        | W::SetNote { .. }
        | W::SetSplits { .. }
        | W::MintMerchantAnchors { .. }
        | W::ApplyMerchantMemory { .. }
        | W::RecordBalanceAssertion { .. }
        | W::RecordConnectorBalance { .. }
        | W::SetHouseholdName { .. }
        | W::AddHouseholdMember { .. }
        | W::RenameHouseholdMember { .. }
        | W::TransferPrimary { .. }
        | W::RemoveHouseholdMember { .. }
        | W::RestoreHouseholdMember { .. }
        | W::SetMemberProfile { .. }
        | W::RecordAssumptionEvent { .. }
        | W::SupersedeAssumptionEvent { .. }
        | W::ClearAssumptionEvent { .. }
        | W::CreateScenario { .. }
        | W::SetScenarioStatus { .. }
        | W::RenameScenario { .. }
        | W::SetScenarioExpiry { .. }
        | W::CloneScenario { .. }
        | W::DeleteScenario { .. }
        | W::ArchiveAccount(_)
        | W::ReinstateAccount(_)
        | W::DeleteRecurringTransfer(_)
        | W::ReinstateRecurringTransfer(_)
        | W::DeleteIncomeSource(_)
        | W::ReinstateIncomeSource(_)
        | W::ArchiveIncomeSource(_)
        | W::RestoreIncomeSource(_)
        | W::ArchiveRecurringBill(_)
        | W::RestoreRecurringBill(_)
        | W::ArchiveCategory(_)
        | W::ReinstateCategory(_)
        | W::ReinstateScenario { .. } => Class3Effect::None,
    }
}

/// The `apply/` modules that write a class-3 table, with the table and the effect that covers it.
/// The test `every_class_three_write_in_apply_has_an_effect` keeps this equal to what the sources
/// contain.
#[cfg(test)]
pub(crate) const CLASS3_APPLY_SITES: &[(&str, &str, Class3Effect)] = &[
    (
        "connector_cmds.rs",
        "connector_link_cursors",
        Class3Effect::ClearLinkCursor,
    ),
    (
        "connector_cmds.rs",
        "audit_events",
        Class3Effect::ScrubDeletedConnection,
    ),
    (
        "household.rs",
        "audit_events",
        Class3Effect::SurvivesOverlay,
    ),
    (
        "settings_cmds.rs",
        "settings",
        Class3Effect::SurvivesOverlay,
    ),
];

/// A command the scratch vault applied from the server's tail.
#[derive(Debug, Clone)]
pub(crate) struct PulledCommand {
    pub(crate) meta: CommandMeta,
    pub(crate) cmd: WriteCommand,
    /// The device's own accepted command: its class-3 effect ran live when it was issued, and the
    /// overlay already carries the result.
    pub(crate) own: bool,
}

/// What [`Scratch::apply_pulled_class3_effects`] did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EffectsReport {
    /// Watermarks cleared for a pulled mapping change.
    pub cursors_cleared: usize,
    /// Pulled connection deletes whose local state was scrubbed.
    pub connections_scrubbed: usize,
    /// Tombstoned connections the sweep cleaned afterwards (residue no pulled command named).
    pub swept: u32,
}

impl Scratch<'_> {
    /// Re-run the class-3 effect of every pulled command, in seq order, on the overlaid scratch
    /// vault, then sweep the tombstoned connections: one transaction, so a failure leaves the
    /// overlaid vault as it was. Once, after [`Scratch::overlay_class3`].
    ///
    /// # Errors
    /// [`RebaseError::EffectsBeforeOverlay`], [`RebaseError::EffectsAlreadyApplied`], or
    /// [`RebaseError::Db`].
    pub fn apply_pulled_class3_effects(&mut self) -> Result<EffectsReport, RebaseError> {
        if !self.overlaid {
            return Err(RebaseError::EffectsBeforeOverlay);
        }
        if self.effects_applied {
            return Err(RebaseError::EffectsAlreadyApplied);
        }
        let report = {
            let guard = self.worker.lock();
            let tx = guard.conn.unchecked_transaction()?;
            let mut report = EffectsReport::default();
            for pulled in self.pulled_commands.iter().filter(|p| !p.own) {
                match effect_of(&pulled.cmd) {
                    Class3Effect::ClearLinkCursor => {
                        let WriteCommand::SetConnectorLinkAccount {
                            connection_id,
                            external_id,
                            ..
                        } = &pulled.cmd
                        else {
                            continue;
                        };
                        let before = cursor_count(&tx)?;
                        clear_link_cursor(&tx, connection_id, external_id)?;
                        report.cursors_cleared += before.saturating_sub(cursor_count(&tx)?);
                    }
                    Class3Effect::ScrubDeletedConnection => {
                        let WriteCommand::DeleteConnectorConnection { id } = &pulled.cmd else {
                            continue;
                        };
                        // The apply arm tombstones and scrubs; a connection the scratch vault does
                        // not hold as tombstoned was a no-op there and is one here.
                        if is_tombstoned(&tx, id)? {
                            remove_local_connection_state(&tx, &pulled.meta, id)?;
                            report.connections_scrubbed += 1;
                        }
                    }
                    Class3Effect::None
                    | Class3Effect::SurvivesOverlay
                    | Class3Effect::DeferredStaging => {}
                }
            }
            report.swept = scrub_tombstoned_in(&tx)?;
            tx.commit()?;
            report
        };
        self.effects_applied = true;
        Ok(report)
    }
}

fn cursor_count(tx: &rusqlite::Connection) -> Result<usize, rusqlite::Error> {
    let n: i64 = tx.query_row("SELECT COUNT(*) FROM connector_link_cursors", [], |r| {
        r.get(0)
    })?;
    Ok(usize::try_from(n).unwrap_or(0))
}

fn is_tombstoned(tx: &rusqlite::Connection, id: &Uuid) -> Result<bool, rusqlite::Error> {
    tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM connector_connections WHERE id = ?1 AND deleted_at IS NOT NULL)",
        params![id],
        |r| r.get(0),
    )
}

#[cfg(test)]
impl Scratch<'_> {
    /// Treat the pulled command `command_id` as this device's own accepted command.
    pub(crate) fn mark_pulled_own(&mut self, command_id: Uuid) {
        for pulled in &mut self.pulled_commands {
            if pulled.meta.command_id == command_id {
                pulled.own = true;
            }
        }
    }
}
