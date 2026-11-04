//! `apply_command` arms for the import/staging pipeline: source batches,
//! source records, and the commit of an imported transaction.
//!
//! [`apply_commit_imported_transaction`] (personal-cfo-0g528.1) is the replicable commit. It
//! carries the MATERIALIZED transaction the issuing device decided on (the read-only planner in
//! `staged_import.rs` ran the dedupe and row checks) and reads no class-3 staging row, so a replica
//! that never staged the file applies it to the same rows. [`apply_commit_staged`] and
//! [`apply_skip_staged`] are RETIRED: they stay so an operation log written before the change
//! decodes and applies as it did, and nothing produces them any more.

use chrono::{DateTime, NaiveDate, Utc};
use core_ledger::{
    AccountId, AccountKind, CategoryId, LedgerAccountId, LedgerTransaction, OperationId, Posting,
    SourceBatchId, SourceRecordId, StagedTransactionId, TransactionId,
};
use core_money::Money;
use rusqlite::{Connection, OptionalExtension};
use uuid::Uuid;

use crate::{
    currency_from_code, ensure_system_ledger_account, ingestion, money_inbox, persist_transaction,
    record_transaction_detail, CommandMeta, DbError, RejectCode,
};

/// Confidence (basis points) for a category prefilled from an import's own category
/// column (ADR 0045 §3). Deliberately moderate — it is a suggestion to confirm, not an
/// authoritative `user` categorization — so it reads as low-confidence in review.
const IMPORT_ALIAS_CONFIDENCE_BPS: i64 = 5000;

/// The longest `memo` or `counterparty` of an imported transaction, in UTF-8 bytes. A parser bounds
/// its own fields far below this; the bound exists so a payload cannot carry an unbounded string.
pub const MAX_IMPORT_TEXT_BYTES: usize = 4_096;
/// The first and last calendar year an imported transaction may be dated in.
pub const FIRST_IMPORT_YEAR: i32 = 1900;
/// See [`FIRST_IMPORT_YEAR`].
pub const LAST_IMPORT_YEAR: i32 = 2999;

/// Whether `text` is an imported memo or counterparty apply accepts. The ONE check: apply refuses a
/// commit with text that fails it, and the planner must never emit such a commit.
pub(crate) fn import_text_is_valid(text: &str) -> bool {
    text.len() <= MAX_IMPORT_TEXT_BYTES
}

/// `text` cut at a character boundary so it passes [`import_text_is_valid`].
pub(crate) fn fit_import_text(text: &str) -> String {
    let mut end = text.len().min(MAX_IMPORT_TEXT_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

/// Whether `instant` falls in the years an imported transaction may be dated in. The same check
/// bounds the planner's reading of a source date and apply's validation of the payload.
pub(crate) fn import_date_is_valid(instant: DateTime<Utc>) -> bool {
    use chrono::Datelike as _;
    (FIRST_IMPORT_YEAR..=LAST_IMPORT_YEAR).contains(&instant.year())
}

/// The ledger transaction id of an imported row: a CONTENT id of (source batch, row ordinal) under the
/// vault's derivation key. FROZEN: vaults hold transactions minted under this formula, so changing it
/// needs a new command variant. It depends on nothing about the device or the command, so the origin
/// derives the same id when it re-plans a row it already committed (a crash between the command and
/// the staged row's bookkeeping), and a re-upload (a new batch) gets new ids while two identical rows
/// in one file (two ordinals) stay distinct.
pub(crate) fn import_transaction_id(
    vault_id: &Uuid,
    source_batch_id: Uuid,
    row_ordinal: i64,
) -> Result<Uuid, DbError> {
    Ok(core_ids::DerivedIdKey::for_vault(vault_id).content_id(
        "txn.import",
        &[
            source_batch_id.as_bytes(),
            row_ordinal.to_be_bytes().as_slice(),
        ],
    )?)
}

/// Applies [`WriteCommand::CommitImportedTransaction`](crate::WriteCommand) (personal-cfo-0g528.1).
///
/// The payload is the OUTCOME of the dedupe and row checks the issuing device ran; this validates it
/// and writes it, and runs no heuristic and reads no class-3 row. The account, the category (when
/// given) and the source record must exist (`ReferenceMissing`); the amount must be non-zero, in the
/// account's currency, dated in the allowed years, with bounded text (`PayloadInvalid`). A
/// transaction that already exists (an identical re-delivery, or one the person voided) is left as
/// it is: nothing is written, so a re-import never resurrects a voided transaction. The batch the
/// dedupe decision belongs to is read off the source record.
#[allow(clippy::too_many_arguments)] // the command's fields plus its metadata
pub(crate) fn apply_commit_imported_transaction(
    tx: &Connection,
    meta: &CommandMeta,
    ids: &core_ids::DerivedIds,
    transaction_id: &TransactionId,
    account_id: &AccountId,
    source_record_id: &SourceRecordId,
    occurred_at: &DateTime<Utc>,
    amount: &Money,
    counterparty: &Option<String>,
    memo: &Option<String>,
    transaction_date: &Option<NaiveDate>,
    category_id: &Option<CategoryId>,
    forced: &bool,
) -> Result<Uuid, DbError> {
    if amount.is_zero() {
        return Err(DbError::reject(
            RejectCode::PayloadInvalid,
            "an imported transaction amount must be non-zero".to_owned(),
        ));
    }
    if [counterparty, memo]
        .into_iter()
        .flatten()
        .any(|text| !import_text_is_valid(text))
    {
        return Err(DbError::reject(
            RejectCode::PayloadInvalid,
            "an imported transaction memo or counterparty is too long".to_owned(),
        ));
    }
    if !import_date_is_valid(*occurred_at) {
        return Err(DbError::reject(
            RejectCode::PayloadInvalid,
            "an imported transaction date is outside the supported years".to_owned(),
        ));
    }
    let account: Option<(Uuid, String)> = tx
        .query_row(
            "SELECT ledger_account_id, currency FROM accounts WHERE id = ?1",
            [account_id.as_uuid()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((ledger_uuid, account_currency)) = account else {
        return Err(DbError::reject(
            RejectCode::ReferenceMissing,
            "an imported transaction names an account that does not exist".to_owned(),
        ));
    };
    if amount.currency().code() != account_currency {
        return Err(DbError::reject(
            RejectCode::PayloadInvalid,
            "an imported transaction currency differs from its account's".to_owned(),
        ));
    }
    if let Some(category) = category_id {
        let known: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM categories WHERE id = ?1)",
            [category.as_uuid()],
            |r| r.get(0),
        )?;
        if !known {
            return Err(DbError::reject(
                RejectCode::ReferenceMissing,
                "an imported transaction names a category that does not exist".to_owned(),
            ));
        }
    }
    let batch_id: Option<Uuid> = tx
        .query_row(
            "SELECT source_batch_id FROM source_records WHERE id = ?1",
            [source_record_id.as_uuid()],
            |r| r.get(0),
        )
        .optional()?;
    let Some(batch_id) = batch_id else {
        return Err(DbError::reject(
            RejectCode::ReferenceMissing,
            "an imported transaction names a source record that does not exist".to_owned(),
        ));
    };

    let already: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM ledger_transactions WHERE id = ?1)",
        [transaction_id.as_uuid()],
        |r| r.get(0),
    )?;
    if already {
        return Ok(transaction_id.as_uuid());
    }

    // Balance the signed amount against the sign-routed system counter-account (mirrors
    // RecordTransaction).
    let (role, kind) = if amount.minor_units() > 0 {
        ("unmatched_income", AccountKind::Income)
    } else {
        ("unmatched_expense", AccountKind::Expense)
    };
    let counter_id = ensure_system_ledger_account(tx, role, amount.currency(), kind)?;
    let txn = balanced_transaction(
        *transaction_id,
        meta,
        *occurred_at,
        LedgerAccountId::from_uuid(ledger_uuid),
        counter_id,
        *amount,
    )?;
    persist_transaction(tx, &txn)?;

    let now = crate::apply::issued_at_text(meta);
    // The ids below are CONTENT ids of the transaction's own id: independent of the command, so the
    // same transaction gets the same rows however many commands deliver it.
    ingestion::link_provenance(
        tx,
        ids.content(
            "import.provenance-link",
            &[transaction_id.as_uuid().as_bytes(), b"created_from"],
        )?,
        &ingestion::NewProvenanceLink {
            entity_type: "ledger_transaction",
            entity_id: transaction_id.as_uuid(),
            source_record_id: source_record_id.as_uuid(),
            relationship: "created_from",
        },
        &now,
    )?;
    // Carry the source detail onto the committed transaction so the list shows what it is, not a
    // bare amount (personal-cfo-byxe).
    let transaction_date = transaction_date.map(|date| date.to_string());
    record_transaction_detail(
        tx,
        transaction_id.as_uuid(),
        memo.as_deref(),
        counterparty.as_deref(),
        transaction_date.as_deref(),
        &now,
    )?;
    // The category the source's own column named, resolved by the issuing device (ADR 0045 §3): a
    // reduced-confidence `import_alias` the person confirms or corrects. The transaction is new, so
    // OR IGNORE only guards a double-apply.
    if let Some(category) = category_id {
        tx.execute(
            "INSERT OR IGNORE INTO transaction_categorizations
                (transaction_id, category_id, source, confidence_bps, assigned_at)
             VALUES (?1, ?2, 'import_alias', ?3, ?4)",
            rusqlite::params![
                transaction_id.as_uuid(),
                category.as_uuid(),
                IMPORT_ALIAS_CONFIDENCE_BPS,
                now
            ],
        )?;
    }
    // A forced commit is the person's "import anyway" override of a suspected duplicate; record that
    // intent in the audit trail. The decision is about the transaction, not the device-local staging
    // row, so it carries no staged id.
    ingestion::record_dedupe_decision(
        tx,
        ids.content(
            "import.dedupe-decision",
            &[transaction_id.as_uuid().as_bytes(), b"committed"],
        )?,
        &ingestion::NewDedupeDecision {
            source_batch_id: batch_id,
            layer: "transaction",
            staged_transaction_id: None,
            matched_entity_type: None,
            matched_entity_id: None,
            decision: "committed",
            reason: if *forced {
                "user import-anyway override"
            } else {
                "unique"
            },
        },
        &now,
    )?;
    Ok(transaction_id.as_uuid())
}

/// The errors the staged-row operations raise, in one place: the retired commit and skip, and the
/// worker methods that replaced them, refuse the same way. They are caller mistakes about
/// device-local state (class 3), never a verdict on a command, so they are `InvalidCommand`.
pub(crate) fn staged_missing() -> DbError {
    DbError::InvalidCommand("staged transaction does not exist".to_owned())
}

pub(crate) fn staged_already_committed() -> DbError {
    DbError::InvalidCommand("staged transaction is already committed".to_owned())
}

pub(crate) fn staged_cannot_be_skipped() -> DbError {
    DbError::InvalidCommand("a committed staged transaction cannot be skipped".to_owned())
}

/// A staged row with no row ordinal, or whose ordinal already names a transaction made from another
/// source record: the derived id cannot be trusted, so the commit stops loudly.
pub(crate) fn staged_ordinal_unusable() -> DbError {
    DbError::InvalidCommand(
        "staged transaction's row ordinal is missing or already used by another row".to_owned(),
    )
}

/// The two-posting transaction an import commits: `amount` on the user's account and its negation on
/// the sign-routed system counter-account. Shared by the commit and the retired commit, so both build
/// the transaction one way. The inputs make a failure impossible (two postings, one currency, a zero
/// sum); the constructor's error is mapped, never expected.
fn balanced_transaction(
    id: TransactionId,
    meta: &CommandMeta,
    occurred_at: DateTime<Utc>,
    account: LedgerAccountId,
    counter: LedgerAccountId,
    amount: Money,
) -> Result<LedgerTransaction, DbError> {
    LedgerTransaction::new(
        id,
        OperationId::from_uuid(meta.command_id),
        occurred_at,
        vec![
            Posting::new(account, amount),
            Posting::new(counter, amount.checked_neg()?),
        ],
    )
    .map_err(|e| DbError::InvalidCommand(e.to_string()))
}

/// Applies [`WriteCommand::CreateSourceBatch`].
#[allow(clippy::too_many_arguments)] // the command's fields plus its metadata
pub(crate) fn apply_create_source_batch(
    tx: &Connection,
    meta: &CommandMeta,
    id: &SourceBatchId,
    source_type: &str,
    source_name: &Option<String>,
    file_fingerprint: &Option<String>,
    parser_version: &Option<String>,
    connector_connection_id: &Option<Uuid>,
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
                connector_connection_id: *connector_connection_id,
            },
            &crate::apply::issued_at_text(meta),
        )?;
        id.as_uuid()
    };
    Ok(entity_id)
}

/// Applies [`WriteCommand::UpdateBatchState`].
pub(crate) fn apply_update_batch_state(
    tx: &Connection,
    meta: &CommandMeta,
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
            &crate::apply::issued_at_text(meta),
        )?;
        batch_id.as_uuid()
    };
    Ok(entity_id)
}

/// Applies [`WriteCommand::CommitStaged`].
/// The cross-source layer's input gathering: resolves the staged row's own
/// source type and its account's ledger id, then asks the heuristic. `force`
/// (import-anyway) skips the layer exactly like the fingerprint check.
pub(crate) fn cross_source_reason(
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
    let (own_source_type, own_connection): (String, Option<Uuid>) = tx.query_row(
        "SELECT sb.source_type, sb.connector_connection_id FROM source_records sr
         JOIN source_batches sb ON sb.id = sr.source_batch_id
         WHERE sr.id = ?1",
        [staged.source_record_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let posted_date: String = staged.posted_at.chars().take(10).collect();
    ingestion::cross_source_duplicate_reason(
        tx,
        account_id,
        ledger_account_id,
        staged.amount_minor,
        &posted_date,
        &own_source_type,
        own_connection,
        staged.id,
    )
}

/// Why a staged row cannot become a ledger transaction (personal-cfo-pxi.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowProblem {
    NoAccount,
    UnknownAccount,
    CurrencyMismatch,
    ZeroAmount,
    UnreadableDate,
}

impl RowProblem {
    /// The Money Inbox reason: fixed text, never the row's own values.
    pub(crate) const fn reason(self) -> &'static str {
        match self {
            Self::NoAccount => "not imported: no account was matched for this row",
            Self::UnknownAccount => "not imported: the matched account no longer exists",
            Self::CurrencyMismatch => {
                "not imported: its currency differs from the account's currency"
            }
            Self::ZeroAmount => "not imported: the amount is zero",
            Self::UnreadableDate => "not imported: its date could not be read",
        }
    }

    /// The error an explicit commit (import anyway) returns for this row.
    pub(crate) fn error(self) -> DbError {
        DbError::InvalidCommand(
            match self {
                Self::NoAccount => "staged transaction has no matched account to commit to",
                Self::UnknownAccount => "staged transaction references an unknown account",
                Self::CurrencyMismatch => "staged transaction currency does not match its account",
                Self::ZeroAmount => "staged transaction amount must be non-zero",
                Self::UnreadableDate => "staged transaction has an unparseable posted_at",
            }
            .to_owned(),
        )
    }
}

/// Where and how a valid staged row posts.
pub(crate) struct CommitTarget {
    pub(crate) ledger_uuid: Uuid,
    pub(crate) amount: Money,
    pub(crate) occurred_at: DateTime<Utc>,
}

/// Resolve the staged row's account, amount and date, or say which check it
/// fails. The outer `Result` is a storage error, which always propagates.
pub(crate) fn commit_target(
    tx: &Connection,
    staged: &ingestion::StagedTransaction,
) -> Result<Result<CommitTarget, RowProblem>, DbError> {
    let Some(account_id) = staged.proposed_account_id else {
        return Ok(Err(RowProblem::NoAccount));
    };
    let row: Option<(Uuid, String)> = tx
        .query_row(
            "SELECT ledger_account_id, currency FROM accounts WHERE id = ?1",
            [account_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((ledger_uuid, currency_code)) = row else {
        return Ok(Err(RowProblem::UnknownAccount));
    };
    if staged.currency != currency_code {
        return Ok(Err(RowProblem::CurrencyMismatch));
    }
    let amount = Money::new(staged.amount_minor, currency_from_code(&currency_code)?);
    if amount.is_zero() {
        return Ok(Err(RowProblem::ZeroAmount));
    }
    let occurred_at = DateTime::parse_from_rfc3339(&staged.posted_at)
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|_| {
            NaiveDate::parse_from_str(&staged.posted_at, "%Y-%m-%d")
                .map(|d| d.and_hms_opt(12, 0, 0).expect("noon is valid").and_utc())
        });
    let Ok(occurred_at) = occurred_at else {
        return Ok(Err(RowProblem::UnreadableDate));
    };
    Ok(Ok(CommitTarget {
        ledger_uuid,
        amount,
        occurred_at,
    }))
}

pub(crate) fn apply_commit_staged(
    tx: &Connection,
    meta: &CommandMeta,
    ids: &core_ids::DerivedIds,
    staged_transaction_id: &StagedTransactionId,
    force: &bool,
) -> Result<Uuid, DbError> {
    let entity_id = {
        let staged = ingestion::read_staged_transaction(tx, staged_transaction_id.as_uuid())?
            .ok_or_else(staged_missing)?;
        if staged.commit_status == "committed" {
            return Err(staged_already_committed());
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
        let fingerprint_match = if *force {
            None
        } else {
            ingestion::committed_fingerprint_match(
                tx,
                &staged.txn_fingerprint,
                staged.proposed_account_id,
                staged.id,
            )?
        };
        let committed_or_flagged = if let Some(matched_txn) = fingerprint_match {
            // The decision names the committed counterpart, as the
            // cross-source layer does, so review reads what was decided rather
            // than re-deriving it (personal-cfo-yl5).
            ingestion::record_dedupe_decision(
                tx,
                ids.keyed("dedupe-decision", &[staged.id.as_bytes(), b"flagged"])?,
                &ingestion::NewDedupeDecision {
                    source_batch_id: batch_id,
                    layer: "transaction",
                    staged_transaction_id: Some(staged.id),
                    matched_entity_type: Some("ledger_transaction"),
                    matched_entity_id: Some(matched_txn),
                    decision: "flagged",
                    reason: "duplicate of an already-committed transaction",
                },
                &crate::apply::issued_at_text(meta),
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
                ids.keyed("dedupe-decision", &[staged.id.as_bytes(), b"flagged"])?,
                &ingestion::NewDedupeDecision {
                    source_batch_id: batch_id,
                    layer: "transaction",
                    staged_transaction_id: Some(staged.id),
                    matched_entity_type: Some("ledger_transaction"),
                    matched_entity_id: Some(matched_txn),
                    decision: "flagged",
                    reason,
                },
                &crate::apply::issued_at_text(meta),
            )?;
            ingestion::flag_staged_duplicate(tx, staged.id)?;
            staged.id
        } else {
            let target = match commit_target(tx, &staged)? {
                Ok(target) => target,
                // A row the ledger cannot accept never aborts its batch: on the
                // pipeline path it is FLAGGED for the Money Inbox with a fixed
                // reason and no ledger write, and the batch moves on (ADR 0014
                // §2, personal-cfo-pxi.9). An explicit commit (import anyway)
                // still refuses it, and the whole command rolls back.
                Err(problem) if !*force => {
                    ingestion::record_dedupe_decision(
                        tx,
                        ids.keyed("dedupe-decision", &[staged.id.as_bytes(), b"flagged"])?,
                        &ingestion::NewDedupeDecision {
                            source_batch_id: batch_id,
                            layer: "transaction",
                            staged_transaction_id: Some(staged.id),
                            matched_entity_type: None,
                            matched_entity_id: None,
                            decision: "flagged",
                            reason: problem.reason(),
                        },
                        &crate::apply::issued_at_text(meta),
                    )?;
                    ingestion::flag_staged_for_review(tx, staged.id)?;
                    money_inbox::rebuild_in(tx, &crate::apply::issued_at_text(meta))?;
                    return Ok(staged.id);
                }
                Err(problem) => return Err(problem.error()),
            };
            let CommitTarget {
                ledger_uuid,
                amount,
                occurred_at,
            } = target;

            // Balance the signed amount against the sign-routed system
            // counter-account (mirrors RecordTransaction).
            let (role, kind) = if amount.minor_units() > 0 {
                ("unmatched_income", AccountKind::Income)
            } else {
                ("unmatched_expense", AccountKind::Expense)
            };
            let counter_id = ensure_system_ledger_account(tx, role, amount.currency(), kind)?;
            // KEYED by the staged record: this command reads class-3 staging rows to decide what
            // to commit (known non-replicable until the import reshape), so its mints must never
            // consume ordinals or depend on a count.
            let txn_id = TransactionId::from_uuid(ids.keyed(
                "txn.commit-staged",
                &[staged_transaction_id.as_uuid().as_bytes()],
            )?);
            let txn = balanced_transaction(
                txn_id,
                meta,
                occurred_at,
                LedgerAccountId::from_uuid(ledger_uuid),
                counter_id,
                amount,
            )?;
            persist_transaction(tx, &txn)?;

            // FK-strict import provenance: the committed txn ← its source_record.
            ingestion::link_provenance(
                tx,
                ids.keyed("provenance-link", &[staged.id.as_bytes(), b"created_from"])?,
                &ingestion::NewProvenanceLink {
                    entity_type: "ledger_transaction",
                    entity_id: txn_id.as_uuid(),
                    source_record_id: staged.source_record_id,
                    relationship: "created_from",
                },
                &crate::apply::issued_at_text(meta),
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
                &crate::apply::issued_at_text(meta),
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
                            crate::apply::issued_at_text(meta),
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
                ids.keyed("dedupe-decision", &[staged.id.as_bytes(), b"committed"])?,
                &ingestion::NewDedupeDecision {
                    source_batch_id: batch_id,
                    layer: "transaction",
                    staged_transaction_id: Some(staged.id),
                    matched_entity_type: None,
                    matched_entity_id: None,
                    decision: "committed",
                    reason: commit_reason,
                },
                &crate::apply::issued_at_text(meta),
            )?;
            txn_id.as_uuid()
        };

        // Refresh the Money Inbox projection so a newly-flagged duplicate gains
        // an item and a committed row drops out (ADR 0014 §7, personal-cfo-dsq).
        money_inbox::rebuild_in(tx, &crate::apply::issued_at_text(meta))?;
        committed_or_flagged
    };
    Ok(entity_id)
}

/// Applies [`WriteCommand::SkipStaged`].
pub(crate) fn apply_skip_staged(
    tx: &Connection,
    meta: &CommandMeta,
    ids: &core_ids::DerivedIds,
    staged_transaction_id: &StagedTransactionId,
) -> Result<Uuid, DbError> {
    let entity_id = {
        let staged = ingestion::read_staged_transaction(tx, staged_transaction_id.as_uuid())?
            .ok_or_else(staged_missing)?;
        if staged.commit_status == "committed" {
            return Err(staged_cannot_be_skipped());
        }
        let batch_id: Uuid = tx.query_row(
            "SELECT source_batch_id FROM source_records WHERE id = ?1",
            [staged.source_record_id],
            |r| r.get(0),
        )?;
        ingestion::mark_staged_skipped(tx, staged.id)?;
        ingestion::record_dedupe_decision(
            tx,
            ids.keyed("dedupe-decision", &[staged.id.as_bytes(), b"skipped"])?,
            &ingestion::NewDedupeDecision {
                source_batch_id: batch_id,
                layer: "transaction",
                staged_transaction_id: Some(staged.id),
                matched_entity_type: None,
                matched_entity_id: None,
                decision: "skipped",
                reason: "user skipped",
            },
            &crate::apply::issued_at_text(meta),
        )?;
        // The skipped row is no longer flagged, so the next rebuild drops it.
        money_inbox::rebuild_in(tx, &crate::apply::issued_at_text(meta))?;
        staged.id
    };
    Ok(entity_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault() -> Uuid {
        Uuid::from_u128(0x0198_c2c1_5f0a_7c3d_8a4e_9b1d_0000_0001)
    }
    fn batch(n: u128) -> Uuid {
        Uuid::from_u128(0x0198_c2c1_5f0a_7c3d_8a4e_9b1d_0000_0100 + n)
    }

    #[test]
    fn an_imported_transactions_id_is_a_pure_function_of_vault_batch_and_row() {
        let id = import_transaction_id(&vault(), batch(1), 1).unwrap();
        assert_eq!(id, import_transaction_id(&vault(), batch(1), 1).unwrap());
        // Another row of the batch, another batch (a re-upload), another vault: another id.
        assert_ne!(id, import_transaction_id(&vault(), batch(1), 2).unwrap());
        assert_ne!(id, import_transaction_id(&vault(), batch(2), 1).unwrap());
        let other_vault = Uuid::from_u128(0x0198_c2c1_5f0a_7c3d_8a4e_9b1d_0000_0002);
        assert_ne!(
            id,
            import_transaction_id(&other_vault, batch(1), 1).unwrap()
        );
        // Ordinals 1 and 256 differ in more than the last byte of a short encoding.
        assert_ne!(
            import_transaction_id(&vault(), batch(1), 1).unwrap(),
            import_transaction_id(&vault(), batch(1), 257).unwrap()
        );
    }

    /// FROZEN: vaults hold transactions minted under this formula. A change here must be a new
    /// command variant, not an edit of this vector.
    #[test]
    fn the_transaction_id_formula_is_pinned_by_a_known_vector() {
        assert_eq!(
            import_transaction_id(&vault(), batch(1), 1)
                .unwrap()
                .to_string(),
            PINNED_TRANSACTION_ID
        );
    }

    const PINNED_TRANSACTION_ID: &str = "1faa0e1c-e937-82ad-9508-ea0a9bb76003";

    #[test]
    fn the_text_fitter_cuts_at_a_character_boundary_and_what_it_returns_is_valid() {
        let long = "\u{20ac}".repeat(MAX_IMPORT_TEXT_BYTES); // three bytes each
        let cut = fit_import_text(&long);
        assert!(cut.len() <= MAX_IMPORT_TEXT_BYTES);
        assert_eq!(cut.len() % 3, 0, "never inside a character");
        assert!(import_text_is_valid(&cut));
        assert!(!import_text_is_valid(&long));
        assert_eq!(fit_import_text("short"), "short");
    }

    #[test]
    fn only_the_supported_years_are_valid_dates() {
        let at = |year: i32| {
            DateTime::parse_from_rfc3339(&format!("{year:04}-06-01T12:00:00Z"))
                .unwrap()
                .with_timezone(&Utc)
        };
        assert!(import_date_is_valid(at(FIRST_IMPORT_YEAR)));
        assert!(import_date_is_valid(at(LAST_IMPORT_YEAR)));
        assert!(!import_date_is_valid(at(FIRST_IMPORT_YEAR - 1)));
        assert!(!import_date_is_valid(at(LAST_IMPORT_YEAR + 1)));
    }
}
