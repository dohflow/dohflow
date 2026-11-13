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
//! **Cross-device identity** (`personal-cfo-0g528.3`, ADR 0074 Addendum J). A row of a SimpleFIN batch,
//! or of a file batch whose fingerprint has section 12's form, claims an identity (`claim_for`): a
//! provider row's ledger id is its derived identity, a file row's stays the id of its batch and ordinal,
//! and the claim's ordinal is the staged row's own, the value that derives that id. When the identity is
//! already held, and the row equals what it holds or its transaction is voided, **no command is sent**:
//! only the staged row's bookkeeping is left. A row that differs carries on (a device that staged the
//! first one still flags it, as it always did) with a command that records the revision. A person's
//! "import anyway" claims nothing, and a file row whose identity is voided claims nothing either, so a
//! re-uploaded voided file keeps restoring until the restore question exists.
//!
//! **Planning runs on the writer connection.** The planner is read-only SQL, but a fresh read
//! connection costs about 75 ms to key with a passphrase, and an import decides thousands of rows. The
//! lock is held for the plan alone and released before the command is dispatched.

use chrono::NaiveDate;
use core_ledger::StagedTransactionId;
use rusqlite::{params, Connection};
use uuid::Uuid;

use crate::apply::import_identity::{changed_fields, held, Snapshot};
use crate::apply::ingestion_cmds::{
    commit_target, cross_source_reason, fit_import_text, import_date_is_valid,
    import_transaction_id, staged_already_committed, staged_cannot_be_skipped, staged_missing,
    staged_ordinal_unusable, RowProblem,
};
use crate::{
    ingestion, CommandMeta, DbError, DbWorker, ImportIdentityClaim, ImportIdentityComponents,
    Outcome, WriteCommand,
};

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
    reason: &'static str,
    matched: Option<Uuid>,
    /// Whether the row is a suspected duplicate (as opposed to a row the ledger cannot accept).
    suspected_duplicate: bool,
    /// Whether the row is already flagged, so a repeat writes nothing.
    already_flagged: bool,
    /// When the row's cross-device identity is held with different fields, the commit that records
    /// the revision (apply commits nothing and records a pending disposition): sent after the flag,
    /// so the revision is recorded whether or not this device flags the row.
    revision: Option<Box<WriteCommand>>,
}

/// The identity a staged row claims, with the ledger id its commit carries.
struct PlannedClaim {
    claim: ImportIdentityClaim,
    /// A provider row's ledger id IS its derived identity; a file row's stays the id of its batch and
    /// ordinal.
    ledger_id: Uuid,
    /// A provider row's posted date and description, read from its record the way the backfill does.
    provider: Option<(String, Option<String>)>,
}

/// The identity a staged row of a SimpleFIN batch or a file batch claims (ADR 0074 Addendum J item 2):
/// only a SimpleFIN transaction with a provider id and account key that satisfy ADR 0013-A section 9,
/// and a file row whose batch has a fingerprint in section 12's form, have one. A LunchFlow row, a
/// connector row without an id and a row under a fallback fingerprint keep the local heuristic path.
///
/// A file row whose identity is held by a **voided** transaction claims nothing: re-uploading a voided
/// file keeps restoring silently until the restore question exists (`personal-cfo-0g528.3`, S3c).
fn claim_for(
    conn: &Connection,
    vault_id: &Uuid,
    staged: &ingestion::StagedTransaction,
    batch_id: Uuid,
    ordinal: i64,
) -> Result<Option<PlannedClaim>, DbError> {
    let (source_type, connection, fingerprint, external_id, normalized): (
        String,
        Option<Uuid>,
        Option<String>,
        Option<String>,
        String,
    ) = conn.query_row(
        "SELECT sb.source_type, sb.connector_connection_id, sb.file_fingerprint, sr.external_id,
                sr.normalized_json
           FROM source_records sr JOIN source_batches sb ON sb.id = sr.source_batch_id
          WHERE sr.id = ?1 AND sb.id = ?2",
        params![staged.source_record_id, batch_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
    )?;
    let key = core_ids::DerivedIdKey::for_vault(vault_id);
    if source_type == "simplefin" {
        let (Some(connection_id), Some(provider_txn_id)) = (connection, external_id) else {
            return Ok(None);
        };
        let Some((provider_account_key, posted_date, description)) =
            crate::import_identity_backfill::parse_normalized(&normalized)
        else {
            return Ok(None);
        };
        let Ok(identity_id) =
            key.provider_txn_id(&connection_id, &provider_account_key, &provider_txn_id)
        else {
            return Ok(None);
        };
        return Ok(Some(PlannedClaim {
            claim: ImportIdentityClaim {
                identity_id,
                components: ImportIdentityComponents::Provider {
                    connection_id,
                    provider_account_key,
                    provider_txn_id,
                },
            },
            ledger_id: identity_id,
            provider: Some((posted_date, description)),
        }));
    }
    let (Some(fingerprint), Ok(row_ordinal)) = (fingerprint, u32::try_from(ordinal)) else {
        return Ok(None);
    };
    let Ok(parsed) = core_ids::FileFingerprint::parse(&fingerprint) else {
        return Ok(None);
    };
    let Ok(identity_id) = key.file_row_id(&parsed, row_ordinal) else {
        return Ok(None);
    };
    if held(conn, identity_id)?.is_some_and(|held| held.voided) {
        return Ok(None);
    }
    Ok(Some(PlannedClaim {
        claim: ImportIdentityClaim {
            identity_id,
            components: ImportIdentityComponents::File {
                file_fingerprint: fingerprint,
                row_ordinal,
            },
        },
        ledger_id: import_transaction_id(vault_id, batch_id, ordinal)?,
        provider: None,
    }))
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

/// The command that commits `staged` as the transaction `transaction_id`, claiming `claim` unless the
/// person forced the row.
fn command_for(
    conn: &Connection,
    staged: &ingestion::StagedTransaction,
    transaction_id: Uuid,
    claim: Option<&PlannedClaim>,
    force: bool,
    target: crate::apply::ingestion_cmds::CommitTarget,
) -> Result<WriteCommand, DbError> {
    // The category the source's own column named, when it names a real one (ADR 0045 §3); resolved
    // here so the command carries an id and apply looks nothing up by name.
    let category_id = match staged.imported_category.as_deref() {
        Some(name) => crate::categories::find_id_by_name(conn, name)?,
        None => None,
    };
    let account_id = staged
        .proposed_account_id
        .expect("commit_target found the account, so the row has one");
    Ok(WriteCommand::CommitImportedTransaction {
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
        identity: claim.map(|planned| planned.claim.clone()),
    })
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
    // An "import anyway" claims no identity: the person's override commits the row as it always did.
    let claim = if force {
        None
    } else {
        claim_for(conn, vault_id, &staged, batch_id, ordinal)?
    };
    let transaction_id = match &claim {
        Some(planned) => planned.ledger_id,
        None => import_transaction_id(vault_id, batch_id, ordinal)?,
    };

    // Step 0a: the identity is held already (another device's commit, a backfill, an earlier refresh).
    // A row that is equal to what the identity holds, or whose transaction the person voided, is not
    // committed and no command is sent: only the bookkeeping is left. A differing row carries on, with
    // a command that records the revision.
    let mut revision_target = None;
    let mut identity_held = false;
    if let Some(planned) = &claim {
        if let Some(held) = held(conn, planned.claim.identity_id)? {
            identity_held = true;
            match commit_target(conn, &staged)? {
                Ok(target) if import_date_is_valid(target.occurred_at) => {
                    let account = staged
                        .proposed_account_id
                        .expect("commit_target found the account, so the row has one");
                    let incoming = Snapshot::of(
                        planned.provider.clone(),
                        account,
                        &target.amount,
                        &target.occurred_at,
                    );
                    if held.voided
                        || changed_fields(&planned.claim.components, &held.snapshot, &incoming)
                            .is_empty()
                    {
                        return Ok(StagedPlan::AlreadyCommitted {
                            staged_id,
                            transaction_id: held.transaction_id,
                        });
                    }
                    revision_target = Some(target);
                }
                // A row the ledger cannot accept is planned (and flagged) the usual way, but a
                // voided identity's row is never imported again.
                _ if held.voided => {
                    return Ok(StagedPlan::AlreadyCommitted {
                        staged_id,
                        transaction_id: held.transaction_id,
                    });
                }
                _ => {}
            }
        }
    }
    let revision = match revision_target {
        Some(target) => Some(Box::new(command_for(
            conn,
            &staged,
            transaction_id,
            claim.as_ref(),
            false,
            target,
        )?)),
        None => None,
    };

    // Step 0: the transaction this row derives already exists, so there is nothing to decide.
    let exists: bool = !identity_held
        && conn.query_row(
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
            reason,
            matched,
            suspected_duplicate,
            already_flagged,
            revision: revision.clone(),
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

    let command = match revision {
        Some(command) => *command,
        None => command_for(conn, &staged, transaction_id, claim.as_ref(), force, target)?,
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
                let claimed = match &command {
                    WriteCommand::CommitImportedTransaction { identity, .. } => {
                        identity.as_ref().map(|claim| claim.identity_id)
                    }
                    _ => None,
                };
                let outcome = self.dispatch(meta.clone(), command)?;
                let guard = self.lock();
                // A row that claimed an identity is bookkept against the transaction the identity
                // holds: the one it just committed, or, when another device's commit (or an
                // earlier one) held the identity with different fields, that one.
                let committed = match claimed {
                    Some(identity_id) => guard.conn.query_row(
                        "SELECT transaction_id FROM import_identities WHERE identity_id = ?1",
                        [identity_id],
                        |r| r.get(0),
                    )?,
                    None => transaction_id,
                };
                ingestion::mark_staged_committed(&guard.conn, staged_id, committed)?;
                Ok(StagedDecision::Committed(outcome))
            }
            StagedPlan::Flag(mut flag) => {
                if !flag.already_flagged {
                    self.record_flag(&flag)?;
                }
                // The row differs from what its identity holds: the revision is recorded (apply
                // commits nothing), though this device also flags the row, as it always did.
                if let Some(revision) = flag.revision.take() {
                    self.dispatch(meta.clone(), *revision)?;
                }
                Ok(StagedDecision::Flagged)
            }
        }
    }

    /// Flag a staged row for the Money Inbox, with the reason it was flagged and the committed
    /// counterpart when there is one, both recorded on the staged row (class 3). Device-local: a flag
    /// is advisory state about a staging row, which only this device has, never a replicated decision.
    /// One statement, so a flagged row always has its reason.
    fn record_flag(&self, flag: &PlannedFlag) -> Result<(), DbError> {
        let guard = self.lock();
        if flag.suspected_duplicate {
            ingestion::flag_staged_duplicate(
                &guard.conn,
                flag.staged_id,
                flag.reason,
                flag.matched,
            )?;
        } else {
            ingestion::flag_staged_for_review(
                &guard.conn,
                flag.staged_id,
                flag.reason,
                flag.matched,
            )?;
        }
        Ok(())
    }

    /// Skip a staged row without committing it: the Money Inbox "skip" resolution (ADR 0014 §7,
    /// personal-cfo-asqy) and the silent skip of a certain connector duplicate. Marks the row
    /// `skipped`; no ledger write, and nothing replicates. No decision row is written: a `skipped`
    /// decision names no transaction, and the staged row's status is the record until
    /// personal-cfo-5ymg8.6 makes the skip a command. Skipping a row that is already skipped does
    /// nothing. The Money Inbox is not rebuilt.
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
        ingestion::mark_staged_skipped(&guard.conn, staged.id)?;
        Ok(())
    }
}
