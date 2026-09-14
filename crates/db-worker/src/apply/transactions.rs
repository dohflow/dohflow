//! `apply_command` arms for transaction, tag, note, and split commands
//! (moved verbatim from `lib.rs`).

use chrono::{DateTime, NaiveDate, Utc};
use core_ledger::{
    AccountId, AccountKind, LedgerAccountId, LedgerTransaction, OperationId, Posting,
    RecurringEventId, SplitLineId, TagId, TransactionId,
};
use core_money::Money;
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::{
    currency_from_code, ensure_system_ledger_account, persist_transaction,
    reverse_and_hide_transaction, unexplained_adjustment, CommandMeta, DbError, SplitLineInput,
};

/// Applies [`WriteCommand::RecordTransaction`].
pub(crate) fn apply_record_transaction(
    tx: &Connection,
    meta: &CommandMeta,
    transaction_id: &TransactionId,
    account_id: &AccountId,
    amount: &Money,
    occurred_at: &DateTime<Utc>,
) -> Result<Uuid, DbError> {
    let entity_id = {
        if amount.is_zero() {
            return Err(DbError::InvalidCommand(
                "transaction amount must be non-zero".to_owned(),
            ));
        }

        // Resolve the account's ledger account + currency.
        let row: Option<(Uuid, String)> = tx
            .query_row(
                "SELECT ledger_account_id, currency FROM accounts WHERE id = ?1",
                [account_id.as_uuid()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((ledger_uuid, currency_code)) = row else {
            return Err(DbError::InvalidCommand(
                "transaction references an unknown account".to_owned(),
            ));
        };
        if amount.currency() != currency_from_code(&currency_code)? {
            return Err(DbError::InvalidCommand(
                "transaction currency does not match account currency".to_owned(),
            ));
        }

        // Balance the movement against a system counter-account, routed by
        // sign: money in is income, money out is expense.
        let (role, kind) = if amount.minor_units() > 0 {
            ("unmatched_income", AccountKind::Income)
        } else {
            ("unmatched_expense", AccountKind::Expense)
        };
        let counter_id = ensure_system_ledger_account(tx, role, amount.currency(), kind)?;

        let txn = LedgerTransaction::new(
            *transaction_id,
            OperationId::from_uuid(meta.command_id),
            *occurred_at,
            vec![
                Posting::new(LedgerAccountId::from_uuid(ledger_uuid), *amount),
                Posting::new(counter_id, amount.checked_neg()?),
            ],
        )
        .map_err(|e| DbError::InvalidCommand(e.to_string()))?;
        persist_transaction(tx, &txn)?;

        // The op-log entity for a manual record is the account it moved against
        // (unchanged); the transaction id is surfaced to the caller separately.
        account_id.as_uuid()
    };
    Ok(entity_id)
}

/// Applies [`WriteCommand::ConfirmObligationEarly`].
pub(crate) fn apply_confirm_obligation_early(
    tx: &Connection,
    meta: &CommandMeta,
    recurring_event_id: &RecurringEventId,
    scheduled_date: &NaiveDate,
    actual_amount: &Money,
    actual_date: &DateTime<Utc>,
    paying_account_id: &AccountId,
) -> Result<Uuid, DbError> {
    let entity_id = {
        // A zero amount is allowed — "nothing was due this cycle" (e.g. a variable bill or a
        // zero statement balance); it posts a balanced $0 transaction and still suppresses the
        // occurrence so the forecast stops projecting the nominal amount (personal-cfo-5ie.9).
        // Only a negative amount is rejected.
        if actual_amount.minor_units() < 0 {
            return Err(DbError::InvalidCommand(
                "confirmed amount can't be negative".to_owned(),
            ));
        }
        // Refuse a future-dated confirm: the posting would sit beyond the forecast's start
        // (which is date-granular), so suppressing the occurrence while its outflow is not yet
        // in the starting balance would make cash silently vanish (5ie.9 review). "Future" is
        // relative to the household-local date (ADR 0021 §1), not UTC's — the frontend's
        // matching `max={todayIso()}` guard on the date input agrees with this boundary
        // (personal-cfo-ku2hn).
        if actual_date.date_naive() > crate::forecast::household_today(tx)? {
            return Err(DbError::InvalidCommand(
                "cannot confirm a payment dated in the future".to_owned(),
            ));
        }
        let event_uuid = recurring_event_id.as_uuid();
        let sched = scheduled_date.to_string();

        // Idempotent on (recurring_event_id, scheduled_date): a second confirm of the same
        // occurrence posts no new transaction (returns the already-fulfilled entity).
        let already: Option<Uuid> = tx
            .query_row(
                "SELECT paying_account_id FROM confirmed_obligations
                     WHERE recurring_event_id = ?1 AND scheduled_date = ?2",
                params![event_uuid, sched],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(original_paying) = already {
            // Already fulfilled — no new transaction; yield the ORIGINAL paying account
            // recorded on the first confirm (not the caller's input, which a re-confirm need
            // not repeat correctly) so the op-log entity is always a real account (5ie.9 review).
            original_paying
        } else {
            // The occurrence must belong to a real recurring obligation (a bill); the payment
            // currency must match the bill's currency (as well as the paying account's below).
            let bill_currency: Option<String> = tx
                .query_row(
                    "SELECT currency FROM recurring_events WHERE id = ?1",
                    [event_uuid],
                    |r| r.get(0),
                )
                .optional()?;
            let Some(bill_currency) = bill_currency else {
                return Err(DbError::InvalidCommand(
                    "confirm references an unknown recurring bill".to_owned(),
                ));
            };
            if actual_amount.currency() != currency_from_code(&bill_currency)? {
                return Err(DbError::InvalidCommand(
                    "payment currency does not match the bill".to_owned(),
                ));
            }
            // A bill charged to a card with a billing cycle is modeled as the card PAYMENT, not a
            // per-charge liquid outflow — confirming it here would double-count (the card payment
            // still projects the charge). That path awaits mixed-role transfers (r7sb); reject it
            // so v1 stays the liquid-paid bill path only (5ie.9 review).
            let card_charged: bool = tx.query_row(
                "SELECT EXISTS(
                     SELECT 1 FROM recurring_events e
                     JOIN accounts a ON a.id = e.autopay_account_id
                     JOIN debt_terms dt ON dt.account_id = a.id
                     WHERE e.id = ?1 AND a.cashflow_role = 'credit_facility' AND a.active = 1
                       AND dt.statement_close_day IS NOT NULL AND dt.payment_due_day IS NOT NULL
                 )",
                [event_uuid],
                |r| r.get(0),
            )?;
            if card_charged {
                return Err(DbError::InvalidCommand(
                    "this bill is paid via its card's payment; confirming it early isn't \
                     supported yet"
                        .to_owned(),
                ));
            }

            // Resolve the paying account; it must be liquid (v1 pays a bill from cash), and its
            // currency must match the payment. A non-liquid payer would overstate the forecast
            // and needs the liability-crediting path (r7sb) (5ie.9 review).
            let paying: Option<(Uuid, String, String)> = tx
                .query_row(
                    "SELECT ledger_account_id, currency, cashflow_role FROM accounts WHERE id = ?1",
                    [paying_account_id.as_uuid()],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            let Some((ledger_uuid, currency_code, paying_role)) = paying else {
                return Err(DbError::InvalidCommand(
                    "confirm references an unknown paying account".to_owned(),
                ));
            };
            if paying_role != "liquid_cash" {
                return Err(DbError::InvalidCommand(
                    "an early-confirmed bill must be paid from a liquid account".to_owned(),
                ));
            }
            if actual_amount.currency() != currency_from_code(&currency_code)? {
                return Err(DbError::InvalidCommand(
                    "payment currency does not match the paying account".to_owned(),
                ));
            }

            // Post the outflow: paying account down, system expense up (mirrors an expense
            // RecordTransaction), dated when it was actually paid.
            let outflow = actual_amount.checked_neg()?;
            let counter_id = ensure_system_ledger_account(
                tx,
                "unmatched_expense",
                actual_amount.currency(),
                AccountKind::Expense,
            )?;
            let txn_id = TransactionId::new();
            let txn = LedgerTransaction::new(
                txn_id,
                OperationId::from_uuid(meta.command_id),
                *actual_date,
                vec![
                    Posting::new(LedgerAccountId::from_uuid(ledger_uuid), outflow),
                    Posting::new(counter_id, *actual_amount),
                ],
            )
            .map_err(|e| DbError::InvalidCommand(e.to_string()))?;
            persist_transaction(tx, &txn)?;

            // Record the fulfillment — the deterministic source the forecast reads to stop
            // projecting this occurrence (ADR 5ie.7). UNIQUE(event, scheduled_date) also backs
            // idempotency; the id is a v5 hash so a retry is byte-identical.
            tx.execute(
                "INSERT INTO confirmed_obligations
                    (id, recurring_event_id, scheduled_date, transaction_id, paying_account_id,
                     actual_date, actual_amount_minor, currency, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    Uuid::new_v5(
                        &Uuid::NAMESPACE_OID,
                        format!("confirmed_obligation:{event_uuid}:{sched}").as_bytes(),
                    ),
                    event_uuid,
                    sched,
                    txn_id.as_uuid(),
                    paying_account_id.as_uuid(),
                    actual_date.to_rfc3339(),
                    actual_amount.minor_units(),
                    actual_amount.currency().code(),
                    Utc::now().to_rfc3339(),
                ],
            )?;

            paying_account_id.as_uuid()
        }
    };
    Ok(entity_id)
}

/// Applies [`WriteCommand::UnconfirmObligation`].
pub(crate) fn apply_unconfirm_obligation(
    tx: &Connection,
    meta: &CommandMeta,
    recurring_event_id: &RecurringEventId,
    scheduled_date: &NaiveDate,
) -> Result<Uuid, DbError> {
    let entity_id = {
        let event_uuid = recurring_event_id.as_uuid();
        let sched = scheduled_date.to_string();
        let row: Option<(Uuid, Uuid)> = tx
            .query_row(
                "SELECT transaction_id, paying_account_id FROM confirmed_obligations
                     WHERE recurring_event_id = ?1 AND scheduled_date = ?2",
                params![event_uuid, sched],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        match row {
            // Reverse the posted payment (unless already voided) and drop the fulfillment
            // record, so the forecast projects the occurrence again.
            Some((txn_uuid, paying_uuid)) => {
                let txn_meta: Option<(String, String, Option<String>)> = tx
                    .query_row(
                        "SELECT occurred_at, currency, voided_at
                             FROM ledger_transactions WHERE id = ?1",
                        [txn_uuid],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .optional()?;
                if let Some((occurred_at_str, currency_code, None)) = txn_meta {
                    let currency = currency_from_code(&currency_code)?;
                    let occurred_at = DateTime::parse_from_rfc3339(&occurred_at_str)
                        .map(|dt| dt.with_timezone(&Utc))
                        .map_err(|e| DbError::InvalidCommand(e.to_string()))?;
                    reverse_and_hide_transaction(
                        tx,
                        txn_uuid,
                        currency,
                        occurred_at,
                        meta.command_id,
                    )?;
                }
                tx.execute(
                    "DELETE FROM confirmed_obligations
                         WHERE recurring_event_id = ?1 AND scheduled_date = ?2",
                    params![event_uuid, sched],
                )?;
                paying_uuid
            }
            // Never confirmed — idempotent no-op.
            None => event_uuid,
        }
    };
    Ok(entity_id)
}

/// Applies [`WriteCommand::ConvertUnexplainedToTransaction`].
pub(crate) fn apply_convert_unexplained_to_transaction(
    tx: &Connection,
    meta: &CommandMeta,
    account_id: &AccountId,
) -> Result<Uuid, DbError> {
    let entity_id = {
        let row: Option<(Uuid, String)> = tx
            .query_row(
                "SELECT ledger_account_id, currency FROM accounts WHERE id = ?1",
                [account_id.as_uuid()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((ledger_uuid, currency_code)) = row else {
            return Err(DbError::InvalidCommand(
                "convert references an unknown account".to_owned(),
            ));
        };
        let currency = currency_from_code(&currency_code)?;
        // The residual plug (ADR 0027 §8). Nothing to do once it is explained.
        let residual = unexplained_adjustment(tx, account_id.as_uuid(), ledger_uuid)?.unwrap_or(0);
        if residual == 0 {
            return Err(DbError::InvalidCommand(
                "nothing to convert — the balance is already explained".to_owned(),
            ));
        }
        // Date the new posting at the latest OBSERVATION — the same row the
        // residual was computed against (manual or connector-synced, ADR 0027
        // addendum). The old manual-only read here was the fifth anchor site
        // (yl53 review): against a newer connector observation it dated the
        // posting outside the plug window, stacking phantom transactions
        // while the plug never cleared.
        let observed_at: String = tx.query_row(
            "SELECT observed_at FROM balance_observations
                 WHERE account_id = ?1 AND source IN ('manual', 'connector_sync')
                 ORDER BY observed_at DESC, created_at DESC LIMIT 1",
            [account_id.as_uuid()],
            |r| r.get(0),
        )?;
        let occurred_at = NaiveDate::parse_from_str(&observed_at, "%Y-%m-%d")
            .ok()
            .and_then(|d| d.and_hms_opt(0, 0, 0))
            .map(|naive| naive.and_utc())
            .ok_or_else(|| DbError::InvalidCommand("invalid assertion date".to_owned()))?;
        let amount = Money::new(residual, currency);
        // Balance against the system counter, routed by sign (like a manual txn).
        let (role, kind) = if residual > 0 {
            ("unmatched_income", AccountKind::Income)
        } else {
            ("unmatched_expense", AccountKind::Expense)
        };
        let counter_id = ensure_system_ledger_account(tx, role, currency, kind)?;
        let txn = LedgerTransaction::new(
            TransactionId::new(),
            OperationId::from_uuid(meta.command_id),
            occurred_at,
            vec![
                Posting::new(LedgerAccountId::from_uuid(ledger_uuid), amount),
                Posting::new(counter_id, amount.checked_neg()?),
            ],
        )
        .map_err(|e| DbError::InvalidCommand(e.to_string()))?;
        persist_transaction(tx, &txn)?;

        account_id.as_uuid()
    };
    Ok(entity_id)
}

/// Applies [`WriteCommand::Transfer`].
pub(crate) fn apply_transfer(
    tx: &Connection,
    meta: &CommandMeta,
    source_account_id: &AccountId,
    dest_account_id: &AccountId,
    amount: &Money,
    occurred_at: &DateTime<Utc>,
) -> Result<Uuid, DbError> {
    let entity_id = {
        if source_account_id == dest_account_id {
            return Err(DbError::InvalidCommand(
                "a transfer needs two different accounts".to_owned(),
            ));
        }
        if amount.is_zero() || amount.minor_units() < 0 {
            return Err(DbError::InvalidCommand(
                "transfer amount must be positive".to_owned(),
            ));
        }
        // Resolve both accounts' ledger account + currency + role.
        let leg = |id: AccountId| -> Result<(Uuid, String, String), DbError> {
            tx.query_row(
                "SELECT ledger_account_id, currency, cashflow_role
                     FROM accounts WHERE id = ?1",
                [id.as_uuid()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?
            .ok_or_else(|| {
                DbError::InvalidCommand("transfer references an unknown account".to_owned())
            })
        };
        let (source_ledger, source_currency, source_role) = leg(*source_account_id)?;
        let (dest_ledger, dest_currency, dest_role) = leg(*dest_account_id)?;
        // The payer is a debit-normal asset — liquid cash, or an investment being drawn down to
        // cash (j0cg.2). Allowed role pairs:
        //   • liquid_cash      → liquid_cash                        a plain transfer
        //   • liquid_cash      → credit_facility / loan_liability   a debt payment (r7sb, ADR 0035 §3)
        //   • liquid_cash      → investment_asset                   a contribution / DCA (9h0.1)
        //   • investment_asset → liquid_cash                        a withdrawal to cash (j0cg.2)
        // An investment is only ever *liquidated to cash* — you don't pay a bill/card straight
        // from a brokerage; you move it to checking first and pay from there. So an investment
        // payer goes to liquid cash only. A non-asset payer is rejected. In every allowed case
        // the source's −amount leg reduces the payer and the +amount leg lands as cash, shrinks a
        // credit-normal liability's (negatively-stored) owed balance, or raises an investment.
        if !matches!(source_role.as_str(), "liquid_cash" | "investment_asset") {
            return Err(DbError::InvalidCommand(
                "a transfer must be paid from a liquid or investment account".to_owned(),
            ));
        }
        let dest_ok = if source_role == "investment_asset" {
            dest_role == "liquid_cash"
        } else {
            matches!(
                dest_role.as_str(),
                "liquid_cash" | "credit_facility" | "loan_liability" | "investment_asset"
            )
        };
        if !dest_ok {
            return Err(DbError::InvalidCommand(
                if source_role == "investment_asset" {
                    "an investment can only be withdrawn to a liquid account"
                } else {
                    "a transfer destination must be a liquid, liability, or investment account"
                }
                .to_owned(),
            ));
        }
        let currency = amount.currency();
        if currency != currency_from_code(&source_currency)?
            || currency != currency_from_code(&dest_currency)?
        {
            return Err(DbError::InvalidCommand(
                "transfer currency must match both accounts".to_owned(),
            ));
        }

        // Balanced double-entry (ADR 0007): the source leg is −amount (cash leaves the payer),
        // the destination leg is +amount. Between two liquid accounts that nets aggregate cash
        // to zero; to a liability the +amount reduces the owed balance (a real outflow of liquid
        // cash). No system counter-account.
        let txn = LedgerTransaction::new(
            TransactionId::new(),
            OperationId::from_uuid(meta.command_id),
            *occurred_at,
            vec![
                Posting::new(
                    LedgerAccountId::from_uuid(source_ledger),
                    amount.checked_neg()?,
                ),
                Posting::new(LedgerAccountId::from_uuid(dest_ledger), *amount),
            ],
        )
        .map_err(|e| DbError::InvalidCommand(e.to_string()))?;
        persist_transaction(tx, &txn)?;

        source_account_id.as_uuid()
    };
    Ok(entity_id)
}

/// Applies [`WriteCommand::VoidTransaction`].
pub(crate) fn apply_void_transaction(
    tx: &Connection,
    meta: &CommandMeta,
    transaction_id: &TransactionId,
) -> Result<Uuid, DbError> {
    let entity_id = {
        let txn_uuid = transaction_id.as_uuid();
        // The original must exist and not already be voided.
        let row: Option<(String, String, Option<String>)> = tx
            .query_row(
                "SELECT occurred_at, currency, voided_at
                     FROM ledger_transactions WHERE id = ?1",
                [txn_uuid],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((occurred_at_str, currency_code, voided_at)) = row else {
            return Err(DbError::InvalidCommand(
                "transaction does not exist".to_owned(),
            ));
        };
        if voided_at.is_some() {
            return Err(DbError::InvalidCommand(
                "transaction is already deleted".to_owned(),
            ));
        }
        let currency = currency_from_code(&currency_code)?;
        let occurred_at = DateTime::parse_from_rfc3339(&occurred_at_str)
            .map(|dt| dt.with_timezone(&Utc))
            .map_err(|e| DbError::InvalidCommand(e.to_string()))?;

        // Reverse it (nets to zero, dated at the original so it lands in the same assertion
        // window, ADR 0027) and hide both the original and the reversal.
        reverse_and_hide_transaction(tx, txn_uuid, currency, occurred_at, meta.command_id)?;

        txn_uuid
    };
    Ok(entity_id)
}

/// Applies [`WriteCommand::MarkReviewed`].
pub(crate) fn apply_mark_reviewed(
    tx: &Connection,
    transaction_id: &TransactionId,
    reviewed: &bool,
) -> Result<Uuid, DbError> {
    let entity_id = {
        let txn_uuid = transaction_id.as_uuid();
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM ledger_transactions WHERE id = ?1)",
            [txn_uuid],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(DbError::InvalidCommand(
                "transaction does not exist".to_owned(),
            ));
        }
        // Latest-wins explicit override (ADR 0032 §2); the default is derived from
        // import provenance at read time.
        tx.execute(
            "INSERT OR REPLACE INTO transaction_reviews
                    (transaction_id, reviewed, reviewed_at)
                 VALUES (?1, ?2, ?3)",
            params![txn_uuid, i64::from(*reviewed), Utc::now().to_rfc3339()],
        )?;
        txn_uuid
    };
    Ok(entity_id)
}

/// Applies [`WriteCommand::CreateTag`].
pub(crate) fn apply_create_tag(
    tx: &Connection,
    id: &TagId,
    name: &str,
    color: &Option<String>,
) -> Result<Uuid, DbError> {
    let entity_id = {
        // The partial unique index (active names) rejects a duplicate.
        tx.execute(
            "INSERT INTO tags (id, name, color, archived_at, created_at)
                 VALUES (?1, ?2, ?3, NULL, ?4)",
            params![id.as_uuid(), name.trim(), color, Utc::now().to_rfc3339()],
        )
        .map_err(|_| DbError::InvalidCommand("a tag with that name already exists".to_owned()))?;
        id.as_uuid()
    };
    Ok(entity_id)
}

/// Applies [`WriteCommand::SetTags`].
pub(crate) fn apply_set_tags(
    tx: &Connection,
    transaction_id: &TransactionId,
    tag_ids: &[TagId],
) -> Result<Uuid, DbError> {
    let entity_id = {
        let txn_uuid = transaction_id.as_uuid();
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM ledger_transactions WHERE id = ?1)",
            [txn_uuid],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(DbError::InvalidCommand(
                "transaction does not exist".to_owned(),
            ));
        }
        // Replace the full set: clear then re-add each (validating it exists).
        tx.execute(
            "DELETE FROM transaction_tags WHERE transaction_id = ?1",
            [txn_uuid],
        )?;
        for tag_id in tag_ids {
            let known: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM tags WHERE id = ?1)",
                [tag_id.as_uuid()],
                |r| r.get(0),
            )?;
            if !known {
                return Err(DbError::InvalidCommand("tag does not exist".to_owned()));
            }
            tx.execute(
                "INSERT OR IGNORE INTO transaction_tags (transaction_id, tag_id)
                     VALUES (?1, ?2)",
                params![txn_uuid, tag_id.as_uuid()],
            )?;
        }
        txn_uuid
    };
    Ok(entity_id)
}

/// Applies [`WriteCommand::SetNote`].
pub(crate) fn apply_set_note(
    tx: &Connection,
    transaction_id: &TransactionId,
    note: &Option<String>,
) -> Result<Uuid, DbError> {
    let entity_id = {
        let txn_uuid = transaction_id.as_uuid();
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM ledger_transactions WHERE id = ?1)",
            [txn_uuid],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(DbError::InvalidCommand(
                "transaction does not exist".to_owned(),
            ));
        }
        // Upsert the note, preserving any memo/counterparty already recorded (byxe).
        tx.execute(
            "INSERT INTO transaction_details
                    (transaction_id, memo, counterparty, note, created_at)
                 VALUES (?1, NULL, NULL, ?2, ?3)
                 ON CONFLICT(transaction_id) DO UPDATE SET note = excluded.note",
            params![txn_uuid, note.as_deref(), Utc::now().to_rfc3339()],
        )?;
        txn_uuid
    };
    Ok(entity_id)
}

/// Applies [`WriteCommand::SetSplits`].
pub(crate) fn apply_set_splits(
    tx: &Connection,
    transaction_id: &TransactionId,
    lines: &[SplitLineInput],
) -> Result<Uuid, DbError> {
    let entity_id = {
        let txn_uuid = transaction_id.as_uuid();
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM ledger_transactions
                              WHERE id = ?1 AND voided_at IS NULL)",
            [txn_uuid],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(DbError::InvalidCommand(
                "transaction does not exist or is voided".to_owned(),
            ));
        }
        // A non-empty split must partition the transaction's net user-account amount
        // exactly (ADR 0034 §3); an empty `lines` just un-splits it.
        if !lines.is_empty() {
            let (txn_minor, txn_currency): (i64, String) = tx.query_row(
                "SELECT SUM(lp.minor_units), MIN(lp.currency)
                     FROM ledger_postings lp
                     JOIN ledger_accounts la ON la.id = lp.ledger_account_id
                     JOIN accounts a ON a.ledger_account_id = la.id
                     WHERE lp.transaction_id = ?1",
                [txn_uuid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            let lines_sum: i64 = lines.iter().map(|l| l.amount.minor_units()).sum();
            if lines_sum != txn_minor {
                return Err(DbError::InvalidCommand(format!(
                    "split lines sum to {lines_sum} but the transaction amount is {txn_minor}"
                )));
            }
            if lines
                .iter()
                .any(|l| l.amount.currency().code() != txn_currency)
            {
                return Err(DbError::InvalidCommand(
                    "split line currency must match the transaction".to_owned(),
                ));
            }
        }
        // Replace the set: clear the existing lines (+ their tags), then re-insert.
        tx.execute(
            "DELETE FROM split_line_tags WHERE split_line_id IN
                    (SELECT id FROM split_lines WHERE transaction_id = ?1)",
            [txn_uuid],
        )?;
        tx.execute(
            "DELETE FROM split_lines WHERE transaction_id = ?1",
            [txn_uuid],
        )?;
        for (index, line) in lines.iter().enumerate() {
            if let Some(category_id) = line.category_id {
                let known: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM categories WHERE id = ?1)",
                    [category_id.as_uuid()],
                    |r| r.get(0),
                )?;
                if !known {
                    return Err(DbError::InvalidCommand(
                        "split line category does not exist".to_owned(),
                    ));
                }
            }
            let line_id = SplitLineId::new();
            tx.execute(
                "INSERT INTO split_lines
                        (id, transaction_id, amount_minor, currency, category_id, note, sort_order)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    line_id.as_uuid(),
                    txn_uuid,
                    line.amount.minor_units(),
                    line.amount.currency().code(),
                    line.category_id.map(|c| c.as_uuid()),
                    line.note.as_deref(),
                    i64::try_from(index).unwrap_or(i64::MAX),
                ],
            )?;
            for tag_id in &line.tag_ids {
                let known: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM tags WHERE id = ?1)",
                    [tag_id.as_uuid()],
                    |r| r.get(0),
                )?;
                if !known {
                    return Err(DbError::InvalidCommand(
                        "split line tag does not exist".to_owned(),
                    ));
                }
                tx.execute(
                    "INSERT OR IGNORE INTO split_line_tags (split_line_id, tag_id)
                         VALUES (?1, ?2)",
                    params![line_id.as_uuid(), tag_id.as_uuid()],
                )?;
            }
        }
        txn_uuid
    };
    Ok(entity_id)
}
