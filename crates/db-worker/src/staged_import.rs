//! Deciding one staged import row, and committing it as a replicable command
//! (personal-cfo-0g528.1, SYNC-2c).
//!
//! The dedupe and row checks (a fingerprint already committed, the cross-source match, a row the
//! ledger cannot accept) read the device-local staging rows (class 3), so they run HERE, on the
//! device that staged the file, in a read-only planner. What the planner decides travels in the
//! command: a commit becomes [`WriteCommand::CommitImportedTransaction`] carrying the materialized
//! transaction, and a flag or a skip is device-local bookkeeping that never leaves this device. Apply
//! runs none of these heuristics, so a replica that never staged the file applies the commit to the
//! same rows, and a changed threshold cannot change what a logged command writes.
//!
//! **Re-planning is safe.** The command is the only step that touches class-1 rows; marking the staged
//! row committed comes after it. A crash between the two leaves a staged row that re-plans to the same
//! transaction id (a content id of its batch and row ordinal), finds the transaction, and finishes the
//! bookkeeping without dispatching again (planner step 0).
//!
//! **Planning runs on the writer connection.** The planner is read-only SQL, but a fresh read
//! connection costs about 75 ms to key with a passphrase, and an import decides thousands of rows. The
//! lock is held for the plan alone and released before the command is dispatched.

use chrono::{NaiveDate, Utc};
use core_ledger::StagedTransactionId;
use rusqlite::{params, Connection};
use uuid::Uuid;

use crate::apply::ingestion_cmds::{
    commit_target, cross_source_reason, fit_import_text, import_date_is_valid,
    import_transaction_id, staged_already_committed, staged_cannot_be_skipped, staged_missing,
    staged_ordinal_unusable, RowProblem,
};
use crate::{ingestion, CommandMeta, DbError, DbWorker, Outcome, WriteCommand};

/// What deciding one staged row did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StagedDecision {
    /// The row was committed to the ledger by this command.
    Committed(Outcome),
    /// The row's transaction already existed (a crash left the staged row behind); the staged row
    /// is now marked committed and no command was dispatched.
    AlreadyCommitted,
    /// The row was flagged for the Money Inbox, not committed (a duplicate, or a row the ledger
    /// cannot accept). Device-local; no command.
    Flagged,
}

/// A row to commit: the fields of [`WriteCommand::CommitImportedTransaction`] plus the staged row it
/// came from.
struct PlannedCommit {
    staged_id: Uuid,
    transaction_id: Uuid,
    command: WriteCommand,
}

/// A row to flag for review.
struct PlannedFlag {
    staged_id: Uuid,
    batch_id: Uuid,
    reason: &'static str,
    matched: Option<Uuid>,
    /// Whether the row is a suspected duplicate (as opposed to a row the ledger cannot accept).
    suspected_duplicate: bool,
    /// Whether the row is already flagged, so a repeat writes nothing.
    already_flagged: bool,
}

enum StagedPlan {
    /// The derived transaction exists already: only the bookkeeping is left.
    AlreadyCommitted {
        staged_id: Uuid,
        transaction_id: Uuid,
    },
    Commit(Box<PlannedCommit>),
    Flag(PlannedFlag),
}

/// Plan the commit of staged row `staged_id`. Read-only. `force` is the Money Inbox "import anyway":
/// it skips the two duplicate layers and refuses, rather than flags, a row the ledger cannot accept.
fn plan_staged(
    conn: &Connection,
    vault_id: &Uuid,
    staged_id: Uuid,
    force: bool,
) -> Result<StagedPlan, DbError> {
    let staged = ingestion::read_staged_transaction(conn, staged_id)?.ok_or_else(staged_missing)?;
    if staged.commit_status == "committed" {
        return Err(staged_already_committed());
    }
    let batch_id: Uuid = conn.query_row(
        "SELECT source_batch_id FROM source_records WHERE id = ?1",
        [staged.source_record_id],
        |r| r.get(0),
    )?;
    let ordinal = staged.row_ordinal.ok_or_else(staged_ordinal_unusable)?;
    let transaction_id = import_transaction_id(vault_id, batch_id, ordinal)?;

    // Step 0: the transaction this row derives already exists, so there is nothing to decide.
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM ledger_transactions WHERE id = ?1)",
        [transaction_id],
        |r| r.get(0),
    )?;
    if exists {
        // The derived transaction must have been made from THIS row's source record. A repeated
        // ordinal (a staging pass that restarted its numbering) would otherwise silently treat a
        // different row as already committed.
        let made_from_this_record: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM source_provenance_links
                            WHERE entity_type = 'ledger_transaction' AND entity_id = ?1
                              AND source_record_id = ?2)",
            params![transaction_id, staged.source_record_id],
            |r| r.get(0),
        )?;
        if !made_from_this_record {
            return Err(staged_ordinal_unusable());
        }
        return Ok(StagedPlan::AlreadyCommitted {
            staged_id,
            transaction_id,
        });
    }

    let already_flagged = staged.commit_status == "flagged";
    let flag = |reason: &'static str, matched: Option<Uuid>, suspected_duplicate: bool| {
        StagedPlan::Flag(PlannedFlag {
            staged_id,
            batch_id,
            reason,
            matched,
            suspected_duplicate,
            already_flagged,
        })
    };

    // Transaction-level dedupe (ADR 0014 §3): a fingerprint that already belongs to a committed
    // import is FLAGGED for the Money Inbox, never silently dropped and no ledger write. The flag
    // names the committed counterpart so review reads what was decided (personal-cfo-yl5).
    if !force {
        if let Some(matched) = ingestion::committed_fingerprint_match(
            conn,
            &staged.txn_fingerprint,
            staged.proposed_account_id,
            staged.id,
        )? {
            return Ok(flag(
                "duplicate of an already-committed transaction",
                Some(matched),
                true,
            ));
        }
        // Cross-source heuristic (ADR 0014 §3 addendum, personal-cfo-tevp): fingerprints are
        // source-shaped and never match across sources, so a CSV backfill and a connector sync of
        // the same transaction are caught on account, date and amount.
        if let Some((reason, matched)) = cross_source_reason(conn, &staged, &force)? {
            return Ok(flag(reason, Some(matched), true));
        }
    }

    let target = match commit_target(conn, &staged)? {
        // A date outside the years the ledger accepts reads as unreadable, the way a date that does
        // not parse does.
        Ok(target) if !import_date_is_valid(target.occurred_at) => Err(RowProblem::UnreadableDate),
        other => other,
    };
    let target = match target {
        Ok(target) => target,
        // A row the ledger cannot accept never aborts its batch: on the pipeline path it is FLAGGED
        // with a fixed reason (ADR 0014 §2, personal-cfo-pxi.9). An explicit commit (import anyway)
        // refuses it.
        Err(problem) if !force => return Ok(flag(problem.reason(), None, false)),
        Err(problem) => return Err(problem.error()),
    };

    // The category the source's own column named, when it names a real one (ADR 0045 §3); resolved
    // here so the command carries an id and apply looks nothing up by name.
    let category_id = match staged.imported_category.as_deref() {
        Some(name) => crate::categories::find_id_by_name(conn, name)?,
        None => None,
    };
    let account_id = staged
        .proposed_account_id
        .expect("commit_target found the account, so the row has one");
    let command = WriteCommand::CommitImportedTransaction {
        transaction_id: core_ledger::TransactionId::from_uuid(transaction_id),
        account_id: core_ledger::AccountId::from_uuid(account_id),
        source_record_id: core_ledger::SourceRecordId::from_uuid(staged.source_record_id),
        occurred_at: target.occurred_at,
        amount: target.amount,
        counterparty: staged.normalized_merchant.as_deref().map(fit_import_text),
        memo: staged.description.as_deref().map(fit_import_text),
        transaction_date: staged
            .transaction_date
            .as_deref()
            .and_then(|text| NaiveDate::parse_from_str(text, "%Y-%m-%d").ok()),
        category_id: category_id.map(core_ledger::CategoryId::from_uuid),
        forced: force,
    };
    Ok(StagedPlan::Commit(Box::new(PlannedCommit {
        staged_id,
        transaction_id,
        command,
    })))
}

impl DbWorker {
    /// Decide one staged import row: commit it to the ledger, or flag it for the Money Inbox.
    ///
    /// The checks run here against the staging rows (see the module docs); a commit is dispatched as
    /// [`WriteCommand::CommitImportedTransaction`] with `meta`, then the staged row is marked
    /// committed. `force` is the Money Inbox "import anyway". The Money Inbox is NOT rebuilt: the
    /// caller does that once after a batch ([`DbWorker::rebuild_money_inbox`]).
    ///
    /// A retry that carries the idempotency key of a commit that already ran is a replay, as it
    /// would be for any command.
    ///
    /// # Errors
    /// [`DbError::InvalidCommand`] for a staged row that does not exist or is already committed, and
    /// for a forced commit of a row the ledger cannot accept; [`DbError`] from the command or a
    /// storage failure.
    pub fn commit_staged(
        &self,
        meta: CommandMeta,
        staged: StagedTransactionId,
        force: bool,
    ) -> Result<StagedDecision, DbError> {
        let plan = {
            let guard = self.lock();
            let vault_id: Uuid = guard.conn.query_row(
                "SELECT vault_id FROM vault_metadata WHERE singleton = 1",
                [],
                |r| r.get(0),
            )?;
            if let Some(op_seq) =
                crate::find_by_idempotency_key(&guard.conn, &meta.idempotency_key)?
            {
                // A retry of a commit that already ran. If a crash left the staged row unmarked,
                // finish that bookkeeping; any other state (already marked, refused) is left alone.
                if let Ok(StagedPlan::AlreadyCommitted {
                    staged_id,
                    transaction_id,
                }) = plan_staged(&guard.conn, &vault_id, staged.as_uuid(), force)
                {
                    ingestion::mark_staged_committed(&guard.conn, staged_id, transaction_id)?;
                }
                return Ok(StagedDecision::Committed(Outcome::Replayed { op_seq }));
            }
            plan_staged(&guard.conn, &vault_id, staged.as_uuid(), force)?
        };
        match plan {
            StagedPlan::AlreadyCommitted {
                staged_id,
                transaction_id,
            } => {
                let guard = self.lock();
                ingestion::mark_staged_committed(&guard.conn, staged_id, transaction_id)?;
                Ok(StagedDecision::AlreadyCommitted)
            }
            StagedPlan::Commit(commit) => {
                let PlannedCommit {
                    staged_id,
                    transaction_id,
                    command,
                } = *commit;
                let outcome = self.dispatch(meta.clone(), command)?;
                let guard = self.lock();
                ingestion::mark_staged_committed(&guard.conn, staged_id, transaction_id)?;
                Ok(StagedDecision::Committed(outcome))
            }
            StagedPlan::Flag(flag) => {
                if !flag.already_flagged {
                    self.record_flag(&meta, &flag)?;
                }
                Ok(StagedDecision::Flagged)
            }
        }
    }

    /// Flag a staged row for the Money Inbox, with the decision that says why. Device-local: the
    /// decision names the staged row, which only this device has. One transaction, so a flagged row
    /// always has its decision. Transitional: the decision becomes a command with
    /// personal-cfo-5ymg8.6.
    fn record_flag(&self, meta: &CommandMeta, flag: &PlannedFlag) -> Result<(), DbError> {
        let guard = self.lock();
        let vault_id: Uuid = guard.conn.query_row(
            "SELECT vault_id FROM vault_metadata WHERE singleton = 1",
            [],
            |r| r.get(0),
        )?;
        let id = core_ids::DerivedIdKey::for_vault(&vault_id).content_id(
            "import.dedupe-decision",
            &[flag.staged_id.as_bytes(), b"flagged"],
        )?;
        let tx = guard.conn.unchecked_transaction()?;
        ingestion::record_dedupe_decision(
            &tx,
            id,
            &ingestion::NewDedupeDecision {
                source_batch_id: flag.batch_id,
                layer: "transaction",
                staged_transaction_id: Some(flag.staged_id),
                matched_entity_type: flag.matched.map(|_| "ledger_transaction"),
                matched_entity_id: flag.matched,
                decision: "flagged",
                reason: flag.reason,
            },
            &crate::apply::issued_at_text(meta),
        )?;
        if flag.suspected_duplicate {
            ingestion::flag_staged_duplicate(&tx, flag.staged_id)?;
        } else {
            ingestion::flag_staged_for_review(&tx, flag.staged_id)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Skip a staged row without committing it: the Money Inbox "skip" resolution (ADR 0014 §7,
    /// personal-cfo-asqy) and the silent skip of a certain connector duplicate. Marks the row
    /// `skipped` and records the decision; no ledger write, and nothing replicates. Skipping a row
    /// that is already skipped does nothing. The Money Inbox is not rebuilt.
    ///
    /// # Errors
    /// [`DbError::InvalidCommand`] for a staged row that does not exist or is already committed;
    /// [`DbError`] on a storage failure.
    pub fn skip_staged(&self, staged: StagedTransactionId) -> Result<(), DbError> {
        let guard = self.lock();
        let staged = ingestion::read_staged_transaction(&guard.conn, staged.as_uuid())?
            .ok_or_else(staged_missing)?;
        if staged.commit_status == "committed" {
            return Err(staged_cannot_be_skipped());
        }
        if staged.commit_status == "skipped" {
            return Ok(());
        }
        let batch_id: Uuid = guard.conn.query_row(
            "SELECT source_batch_id FROM source_records WHERE id = ?1",
            [staged.source_record_id],
            |r| r.get(0),
        )?;
        let vault_id: Uuid = guard.conn.query_row(
            "SELECT vault_id FROM vault_metadata WHERE singleton = 1",
            [],
            |r| r.get(0),
        )?;
        let id = core_ids::DerivedIdKey::for_vault(&vault_id).content_id(
            "import.dedupe-decision",
            &[staged.id.as_bytes(), b"skipped"],
        )?;
        let tx = guard.conn.unchecked_transaction()?;
        ingestion::mark_staged_skipped(&tx, staged.id)?;
        ingestion::record_dedupe_decision(
            &tx,
            id,
            &ingestion::NewDedupeDecision {
                source_batch_id: batch_id,
                layer: "transaction",
                staged_transaction_id: Some(staged.id),
                matched_entity_type: None,
                matched_entity_id: None,
                decision: "skipped",
                reason: "user skipped",
            },
            &Utc::now().to_rfc3339(),
        )?;
        tx.commit()?;
        Ok(())
    }
}
