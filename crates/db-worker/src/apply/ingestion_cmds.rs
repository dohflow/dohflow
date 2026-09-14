//! `apply_command` arms for the import/staging pipeline: source batches,
//! source records, and staged-transaction commit/skip (moved verbatim from
//! `lib.rs`).

use chrono::{DateTime, NaiveDate, Utc};
use core_ledger::{
    AccountKind, LedgerAccountId, LedgerTransaction, OperationId, Posting, SourceBatchId,
    StagedTransactionId, TransactionId,
};
use core_money::Money;
use rusqlite::{Connection, OptionalExtension};
use uuid::Uuid;

use crate::{
    currency_from_code, ensure_system_ledger_account, ingestion, money_inbox, persist_transaction,
    record_transaction_detail, CommandMeta, DbError,
};

/// Confidence (basis points) for a category prefilled from an import's own category
/// column (ADR 0045 §3). Deliberately moderate — it is a suggestion to confirm, not an
/// authoritative `user` categorization — so it reads as low-confidence in review.
const IMPORT_ALIAS_CONFIDENCE_BPS: i64 = 5000;

/// Applies [`WriteCommand::CreateSourceBatch`].
pub(crate) fn apply_create_source_batch(
    tx: &Connection,
    id: &SourceBatchId,
    source_type: &str,
    source_name: &Option<String>,
    file_fingerprint: &Option<String>,
    parser_version: &Option<String>,
) -> Result<Uuid, DbError> {
    let entity_id = {
        ingestion::create_source_batch(
            tx,
            id.as_uuid(),
            &ingestion::NewSourceBatch {
                source_type,
                source_name: source_name.as_deref(),
                file_fingerprint: file_fingerprint.as_deref(),
                parser_version: parser_version.as_deref(),
            },
        )?;
        id.as_uuid()
    };
    Ok(entity_id)
}

/// Applies [`WriteCommand::UpdateBatchState`].
pub(crate) fn apply_update_batch_state(
    tx: &Connection,
    batch_id: &SourceBatchId,
    status: &str,
    staged_count: &i64,
    committed_count: &i64,
    skipped_count: &i64,
) -> Result<Uuid, DbError> {
    let entity_id = {
        ingestion::update_batch_status(
            tx,
            batch_id.as_uuid(),
            status,
            *staged_count,
            *committed_count,
            *skipped_count,
        )?;
        batch_id.as_uuid()
    };
    Ok(entity_id)
}

/// Applies [`WriteCommand::CommitStaged`].
/// The cross-source layer's input gathering: resolves the staged row's own
/// source type and its account's ledger id, then asks the heuristic. `force`
/// (import-anyway) skips the layer exactly like the fingerprint check.
fn cross_source_reason(
    tx: &Connection,
    staged: &ingestion::StagedTransaction,
    force: &bool,
) -> Result<Option<(&'static str, Uuid)>, DbError> {
    if *force {
        return Ok(None);
    }
    let Some(account_id) = staged.proposed_account_id else {
        return Ok(None);
    };
    let Some(ledger_account_id): Option<Uuid> = tx
        .query_row(
            "SELECT ledger_account_id FROM accounts WHERE id = ?1",
            [account_id],
            |r| r.get(0),
        )
        .optional()?
    else {
        return Ok(None);
    };
    let own_source_type: String = tx.query_row(
        "SELECT sb.source_type FROM source_records sr
         JOIN source_batches sb ON sb.id = sr.source_batch_id
         WHERE sr.id = ?1",
        [staged.source_record_id],
        |r| r.get(0),
    )?;
    let posted_date: String = staged.posted_at.chars().take(10).collect();
    ingestion::cross_source_duplicate_reason(
        tx,
        account_id,
        ledger_account_id,
        staged.amount_minor,
        &posted_date,
        &own_source_type,
        staged.id,
    )
}

pub(crate) fn apply_commit_staged(
    tx: &Connection,
    meta: &CommandMeta,
    staged_transaction_id: &StagedTransactionId,
    force: &bool,
) -> Result<Uuid, DbError> {
    let entity_id = {
        let staged = ingestion::read_staged_transaction(tx, staged_transaction_id.as_uuid())?
            .ok_or_else(|| {
                DbError::InvalidCommand("staged transaction does not exist".to_owned())
            })?;
        if staged.commit_status == "committed" {
            return Err(DbError::InvalidCommand(
                "staged transaction is already committed".to_owned(),
            ));
        }

        // The batch this row belongs to (for the dedupe-decision provenance).
        let batch_id: Uuid = tx.query_row(
            "SELECT source_batch_id FROM source_records WHERE id = ?1",
            [staged.source_record_id],
            |r| r.get(0),
        )?;

        // Transaction-level dedupe (ADR 0014 §3): a fingerprint that already
        // belongs to a committed import is FLAGGED for the Money Inbox — never
        // silently dropped, and no ledger write. `force` (the Money Inbox
        // "import anyway" resolution, ADR 0014 §7) skips this check.
        let committed_or_flagged = if !*force
            && ingestion::fingerprint_already_committed(
                tx,
                &staged.txn_fingerprint,
                staged.proposed_account_id,
                staged.id,
            )? {
            ingestion::record_dedupe_decision(
                tx,
                &ingestion::NewDedupeDecision {
                    source_batch_id: batch_id,
                    layer: "transaction",
                    staged_transaction_id: Some(staged.id),
                    matched_entity_type: Some("staged_transaction"),
                    matched_entity_id: None,
                    decision: "flagged",
                    reason: "duplicate of an already-committed transaction",
                },
            )?;
            ingestion::flag_staged_duplicate(tx, staged.id)?;
            staged.id
        } else if let Some((reason, matched_txn)) = cross_source_reason(tx, &staged, force)? {
            // Cross-source heuristic (ADR 0014 §3 addendum, personal-cfo-tevp):
            // fingerprints are source-shaped and never match across sources, so
            // a CSV backfill vs a connector sync (or a manual entry) of the
            // same real-world transaction is caught on account+date+amount and
            // FLAGGED for review — same-source rows are exempt (the source
            // itself vouches distinct rows are distinct transactions).
            ingestion::record_dedupe_decision(
                tx,
                &ingestion::NewDedupeDecision {
                    source_batch_id: batch_id,
                    layer: "transaction",
                    staged_transaction_id: Some(staged.id),
                    matched_entity_type: Some("ledger_transaction"),
                    matched_entity_id: Some(matched_txn),
                    decision: "flagged",
                    reason,
                },
            )?;
            ingestion::flag_staged_duplicate(tx, staged.id)?;
            staged.id
        } else {
            // Resolve the matched account + its ledger account / currency.
            let account_id = staged.proposed_account_id.ok_or_else(|| {
                DbError::InvalidCommand(
                    "staged transaction has no matched account to commit to".to_owned(),
                )
            })?;
            let row: Option<(Uuid, String)> = tx
                .query_row(
                    "SELECT ledger_account_id, currency FROM accounts WHERE id = ?1",
                    [account_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let Some((ledger_uuid, currency_code)) = row else {
                return Err(DbError::InvalidCommand(
                    "staged transaction references an unknown account".to_owned(),
                ));
            };
            if staged.currency != currency_code {
                return Err(DbError::InvalidCommand(
                    "staged transaction currency does not match its account".to_owned(),
                ));
            }
            let amount = Money::new(staged.amount_minor, currency_from_code(&currency_code)?);
            if amount.is_zero() {
                return Err(DbError::InvalidCommand(
                    "staged transaction amount must be non-zero".to_owned(),
                ));
            }
            let occurred_at = DateTime::parse_from_rfc3339(&staged.posted_at)
                .map(|dt| dt.with_timezone(&Utc))
                .or_else(|_| {
                    NaiveDate::parse_from_str(&staged.posted_at, "%Y-%m-%d")
                        .map(|d| d.and_hms_opt(12, 0, 0).expect("noon is valid").and_utc())
                })
                .map_err(|e| {
                    DbError::InvalidCommand(format!(
                        "staged transaction has an unparseable posted_at {:?}: {e}",
                        staged.posted_at
                    ))
                })?;

            // Balance the signed amount against the sign-routed system
            // counter-account (mirrors RecordTransaction).
            let (role, kind) = if amount.minor_units() > 0 {
                ("unmatched_income", AccountKind::Income)
            } else {
                ("unmatched_expense", AccountKind::Expense)
            };
            let counter_id = ensure_system_ledger_account(tx, role, amount.currency(), kind)?;
            let txn_id = TransactionId::new();
            let txn = LedgerTransaction::new(
                txn_id,
                OperationId::from_uuid(meta.command_id),
                occurred_at,
                vec![
                    Posting::new(LedgerAccountId::from_uuid(ledger_uuid), amount),
                    Posting::new(counter_id, amount.checked_neg()?),
                ],
            )
            .map_err(|e| DbError::InvalidCommand(e.to_string()))?;
            persist_transaction(tx, &txn)?;

            // FK-strict import provenance: the committed txn ← its source_record.
            ingestion::link_provenance(
                tx,
                &ingestion::NewProvenanceLink {
                    entity_type: "ledger_transaction",
                    entity_id: txn_id.as_uuid(),
                    source_record_id: staged.source_record_id,
                    relationship: "created_from",
                },
            )?;
            ingestion::mark_staged_committed(tx, staged.id, txn_id.as_uuid())?;
            // Carry the source detail onto the committed transaction so the list
            // shows what it is, not a bare amount (personal-cfo-byxe): the
            // original description as the memo, the normalized merchant as the
            // counterparty.
            record_transaction_detail(
                tx,
                txn_id.as_uuid(),
                staged.description.as_deref(),
                staged.normalized_merchant.as_deref(),
                staged.transaction_date.as_deref(),
            )?;
            // Prefill the category from the source's own category column when it names
            // a real category (ADR 0045 §3): a reduced-confidence `import_alias`
            // categorization the user confirms or corrects (a later `user` recategorize
            // supersedes it). No name match → uncategorized (the raw category is still
            // visible in the transaction's Imported details, 4d8.24.1.4). The txn is
            // freshly minted, so OR IGNORE only guards against a double-apply.
            if let Some(name) = staged.imported_category.as_deref() {
                if let Some(category_id) = crate::categories::find_id_by_name(tx, name)? {
                    tx.execute(
                        "INSERT OR IGNORE INTO transaction_categorizations
                            (transaction_id, category_id, source, confidence_bps, assigned_at)
                         VALUES (?1, ?2, 'import_alias', ?3, ?4)",
                        rusqlite::params![
                            txn_id.as_uuid(),
                            category_id,
                            IMPORT_ALIAS_CONFIDENCE_BPS,
                            Utc::now().to_rfc3339(),
                        ],
                    )?;
                }
            }
            // A forced commit is the user's "import anyway" override of a
            // suspected duplicate; record that intent in the audit trail.
            let commit_reason = if *force {
                "user import-anyway override"
            } else {
                "unique"
            };
            ingestion::record_dedupe_decision(
                tx,
                &ingestion::NewDedupeDecision {
                    source_batch_id: batch_id,
                    layer: "transaction",
                    staged_transaction_id: Some(staged.id),
                    matched_entity_type: None,
                    matched_entity_id: None,
                    decision: "committed",
                    reason: commit_reason,
                },
            )?;
            txn_id.as_uuid()
        };

        // Refresh the Money Inbox projection so a newly-flagged duplicate gains
        // an item and a committed row drops out (ADR 0014 §7, personal-cfo-dsq).
        money_inbox::rebuild_in(tx)?;
        committed_or_flagged
    };
    Ok(entity_id)
}

/// Applies [`WriteCommand::SkipStaged`].
pub(crate) fn apply_skip_staged(
    tx: &Connection,
    staged_transaction_id: &StagedTransactionId,
) -> Result<Uuid, DbError> {
    let entity_id = {
        let staged = ingestion::read_staged_transaction(tx, staged_transaction_id.as_uuid())?
            .ok_or_else(|| {
                DbError::InvalidCommand("staged transaction does not exist".to_owned())
            })?;
        if staged.commit_status == "committed" {
            return Err(DbError::InvalidCommand(
                "a committed staged transaction cannot be skipped".to_owned(),
            ));
        }
        let batch_id: Uuid = tx.query_row(
            "SELECT source_batch_id FROM source_records WHERE id = ?1",
            [staged.source_record_id],
            |r| r.get(0),
        )?;
        ingestion::mark_staged_skipped(tx, staged.id)?;
        ingestion::record_dedupe_decision(
            tx,
            &ingestion::NewDedupeDecision {
                source_batch_id: batch_id,
                layer: "transaction",
                staged_transaction_id: Some(staged.id),
                matched_entity_type: None,
                matched_entity_id: None,
                decision: "skipped",
                reason: "user skipped",
            },
        )?;
        // The skipped row is no longer flagged, so the next rebuild drops it.
        money_inbox::rebuild_in(tx)?;
        staged.id
    };
    Ok(entity_id)
}
