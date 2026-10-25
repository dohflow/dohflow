//! `apply_command` arms for income sources, recurring bills, and recurring
//! transfers (moved verbatim from `lib.rs`).

use chrono::{NaiveDate, Utc};
use core_ledger::{
    AccountId, BillContractId, CategoryId, IncomeSourceId, RecurringEventId, RecurringTransferId,
    TagId,
};
use core_money::Money;
use pay_schedule::Frequency;
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::apply::household::issued_at_rfc3339;
use crate::{commitments, currency_from_code, CommandMeta, DbError};

/// Applies [`WriteCommand::CreateRecurringTransfer`].
pub(crate) fn apply_create_recurring_transfer(
    tx: &Connection,
    id: &RecurringTransferId,
    source_account_id: &AccountId,
    dest_account_id: &AccountId,
    amount: &Money,
    frequency: &Frequency,
    anchor: &NaiveDate,
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
        // A recurring transfer moves cash from a liquid account to either another liquid account
        // (a plain transfer) or an investment (a contribution / DCA, 9h0.1). A liquid→investment
        // transfer is asymmetric: the aggregate forecast projects the source leg as a real
        // outflow (`collect_recurring_transfer_investment_outflows`), and the per-account path
        // drops the non-liquid investment leg — so the reconciliation invariant (ADR 0035 §3)
        // holds. A *recurring* debt payment stays out of here — it's modelled by the debt
        // account's own automated-payment projection (debt_terms / paying_source, ADR 0035/0036).
        let leg = |id: AccountId| -> Result<(String, String), DbError> {
            tx.query_row(
                "SELECT currency, cashflow_role FROM accounts WHERE id = ?1",
                [id.as_uuid()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| {
                DbError::InvalidCommand("transfer references an unknown account".to_owned())
            })
        };
        let (source_currency, source_role) = leg(*source_account_id)?;
        let (dest_currency, dest_role) = leg(*dest_account_id)?;
        if source_role != "liquid_cash"
            || !matches!(dest_role.as_str(), "liquid_cash" | "investment_asset")
        {
            return Err(DbError::InvalidCommand(
                    "a recurring transfer moves cash from a liquid account to a liquid or investment account"
                        .to_owned(),
                ));
        }
        if amount.currency() != currency_from_code(&source_currency)?
            || amount.currency() != currency_from_code(&dest_currency)?
        {
            return Err(DbError::InvalidCommand(
                "transfer currency must match both accounts".to_owned(),
            ));
        }
        tx.execute(
            "INSERT INTO recurring_transfers (
                    id, source_account_id, dest_account_id, amount_minor, currency,
                    frequency, anchor_date, created_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                id.as_uuid(),
                source_account_id.as_uuid(),
                dest_account_id.as_uuid(),
                amount.minor_units(),
                amount.currency().code(),
                frequency.token(),
                anchor.to_string(),
                Utc::now().to_rfc3339(),
            ],
        )?;
        id.as_uuid()
    };
    Ok(entity_id)
}

/// Whether the row `id` of `table` exists, and if so whether it is tombstoned. The table and
/// key column names are compile-time literals at every call site.
fn tombstone_state(
    tx: &Connection,
    table: &str,
    key_column: &str,
    id: Uuid,
) -> Result<Option<bool>, DbError> {
    Ok(tx
        .query_row(
            &format!("SELECT deleted_at IS NOT NULL FROM {table} WHERE {key_column} = ?1"),
            [id],
            |row| row.get::<_, bool>(0),
        )
        .optional()?)
}

/// Applies [`WriteCommand::DeleteRecurringTransfer`]: a **tombstone**, not a `DELETE`
/// (personal-cfo-egn67, ADR 0073 exceptions item 1). The row stays with `deleted_at` set to
/// the command's `issued_at` (never the wall clock), every reader skips it, and
/// [`apply_reinstate_recurring_transfer`] clears it. Deleting a row that is already
/// tombstoned is an accepted, logged no-op, so the end state is the same on every device.
pub(crate) fn apply_delete_recurring_transfer(
    tx: &Connection,
    meta: &CommandMeta,
    id: &RecurringTransferId,
) -> Result<Uuid, DbError> {
    let entity_id = {
        let updated = tx.execute(
            "UPDATE recurring_transfers SET deleted_at = ?2 WHERE id = ?1 AND deleted_at IS NULL",
            params![id.as_uuid(), issued_at_rfc3339(meta)?],
        )?;
        if updated == 0 && tombstone_state(tx, "recurring_transfers", "id", id.as_uuid())?.is_none()
        {
            return Err(DbError::InvalidCommand(
                "recurring transfer does not exist".to_owned(),
            ));
        }
        id.as_uuid()
    };
    Ok(entity_id)
}

/// Applies [`WriteCommand::ReinstateRecurringTransfer`]: clears the tombstone. Reinstating a
/// transfer that is not deleted is an accepted, logged no-op.
pub(crate) fn apply_reinstate_recurring_transfer(
    tx: &Connection,
    id: &RecurringTransferId,
) -> Result<Uuid, DbError> {
    let entity_id = {
        let updated = tx.execute(
            "UPDATE recurring_transfers SET deleted_at = NULL WHERE id = ?1 AND deleted_at IS NOT NULL",
            [id.as_uuid()],
        )?;
        if updated == 0 && tombstone_state(tx, "recurring_transfers", "id", id.as_uuid())?.is_none()
        {
            return Err(DbError::InvalidCommand(
                "recurring transfer does not exist".to_owned(),
            ));
        }
        id.as_uuid()
    };
    Ok(entity_id)
}

/// Applies [`WriteCommand::CreateIncomeSource`].
pub(crate) fn apply_create_income_source(
    tx: &Connection,
    id: &IncomeSourceId,
    name: &str,
    net_amount: &Money,
    frequency: &Frequency,
    anchor: &NaiveDate,
    deposit_account_id: &Option<AccountId>,
) -> Result<Uuid, DbError> {
    let entity_id = {
        if name.trim().is_empty() {
            return Err(DbError::InvalidCommand(
                "income source name must not be empty".to_owned(),
            ));
        }
        if net_amount.is_zero() || net_amount.minor_units() < 0 {
            return Err(DbError::InvalidCommand(
                "income source net amount must be positive".to_owned(),
            ));
        }
        // If a deposit account is named, it must exist with a matching currency.
        if let Some(account_id) = deposit_account_id {
            // Must exist, be LIQUID CASH (a paycheck lands in a cash account,
            // personal-cfo-3b8.2), and match the income currency.
            let row: Option<(String, String)> = tx
                .query_row(
                    "SELECT currency, cashflow_role FROM accounts WHERE id = ?1",
                    [account_id.as_uuid()],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            match row {
                None => {
                    return Err(DbError::InvalidCommand(
                        "income source references an unknown deposit account".to_owned(),
                    ))
                }
                Some((_, role)) if role != "liquid_cash" => {
                    return Err(DbError::InvalidCommand(
                        "income can only deposit into a cash account".to_owned(),
                    ))
                }
                Some((code, _)) if net_amount.currency() != currency_from_code(&code)? => {
                    return Err(DbError::InvalidCommand(
                        "income currency does not match the deposit account".to_owned(),
                    ))
                }
                Some(_) => {}
            }
        }

        tx.execute(
            "INSERT INTO income_sources (
                    id, name, net_minor_units, currency, frequency, anchor_date,
                    deposit_account_id, active, created_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8)",
            params![
                id.as_uuid(),
                name,
                net_amount.minor_units(),
                net_amount.currency().code(),
                frequency.token(),
                anchor.to_string(),
                deposit_account_id.map(|a| a.as_uuid()),
                Utc::now().to_rfc3339(),
            ],
        )?;

        id.as_uuid()
    };
    Ok(entity_id)
}

/// Applies [`WriteCommand::UpdateIncomeSource`].
pub(crate) fn apply_update_income_source(
    tx: &Connection,
    id: &IncomeSourceId,
    name: &str,
    net_amount: &Money,
    frequency: &Frequency,
    anchor: &NaiveDate,
    deposit_account_id: &Option<AccountId>,
) -> Result<Uuid, DbError> {
    let entity_id = {
        if name.trim().is_empty() {
            return Err(DbError::InvalidCommand(
                "income source name must not be empty".to_owned(),
            ));
        }
        if net_amount.is_zero() || net_amount.minor_units() < 0 {
            return Err(DbError::InvalidCommand(
                "income source net amount must be positive".to_owned(),
            ));
        }
        // If a deposit account is named, it must exist with a matching currency.
        if let Some(account_id) = deposit_account_id {
            // Must exist, be LIQUID CASH (a paycheck lands in a cash account,
            // personal-cfo-3b8.2), and match the income currency.
            let row: Option<(String, String)> = tx
                .query_row(
                    "SELECT currency, cashflow_role FROM accounts WHERE id = ?1",
                    [account_id.as_uuid()],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            match row {
                None => {
                    return Err(DbError::InvalidCommand(
                        "income source references an unknown deposit account".to_owned(),
                    ))
                }
                Some((_, role)) if role != "liquid_cash" => {
                    return Err(DbError::InvalidCommand(
                        "income can only deposit into a cash account".to_owned(),
                    ))
                }
                Some((code, _)) if net_amount.currency() != currency_from_code(&code)? => {
                    return Err(DbError::InvalidCommand(
                        "income currency does not match the deposit account".to_owned(),
                    ))
                }
                Some(_) => {}
            }
        }

        let updated = tx.execute(
            "UPDATE income_sources
                 SET name = ?2, net_minor_units = ?3, currency = ?4, frequency = ?5,
                     anchor_date = ?6, deposit_account_id = ?7
                 WHERE id = ?1 AND deleted_at IS NULL",
            params![
                id.as_uuid(),
                name,
                net_amount.minor_units(),
                net_amount.currency().code(),
                frequency.token(),
                anchor.to_string(),
                deposit_account_id.map(|a| a.as_uuid()),
            ],
        )?;
        if updated == 0 {
            return Err(DbError::InvalidCommand(
                "income source does not exist".to_owned(),
            ));
        }
        id.as_uuid()
    };
    Ok(entity_id)
}

/// Applies [`WriteCommand::DeleteIncomeSource`]: a tombstone (`deleted_at = issued_at`), not a
/// `DELETE`; see [`apply_delete_recurring_transfer`]. It is distinct from archiving: an archived
/// source is hidden and can be restored, a deleted one is gone from every list until reinstated.
pub(crate) fn apply_delete_income_source(
    tx: &Connection,
    meta: &CommandMeta,
    id: &IncomeSourceId,
) -> Result<Uuid, DbError> {
    let entity_id = {
        let updated = tx.execute(
            "UPDATE income_sources SET deleted_at = ?2 WHERE id = ?1 AND deleted_at IS NULL",
            params![id.as_uuid(), issued_at_rfc3339(meta)?],
        )?;
        if updated == 0 && tombstone_state(tx, "income_sources", "id", id.as_uuid())?.is_none() {
            return Err(DbError::InvalidCommand(
                "income source does not exist".to_owned(),
            ));
        }
        id.as_uuid()
    };
    Ok(entity_id)
}

/// Applies [`WriteCommand::ReinstateIncomeSource`]: clears the tombstone (the archive flag is
/// untouched). A source that is not deleted is an accepted, logged no-op.
pub(crate) fn apply_reinstate_income_source(
    tx: &Connection,
    id: &IncomeSourceId,
) -> Result<Uuid, DbError> {
    let entity_id = {
        let updated = tx.execute(
            "UPDATE income_sources SET deleted_at = NULL WHERE id = ?1 AND deleted_at IS NOT NULL",
            [id.as_uuid()],
        )?;
        if updated == 0 && tombstone_state(tx, "income_sources", "id", id.as_uuid())?.is_none() {
            return Err(DbError::InvalidCommand(
                "income source does not exist".to_owned(),
            ));
        }
        id.as_uuid()
    };
    Ok(entity_id)
}

/// Applies [`WriteCommand::ArchiveIncomeSource`].
pub(crate) fn apply_archive_income_source(
    tx: &Connection,
    id: &IncomeSourceId,
) -> Result<Uuid, DbError> {
    let entity_id = {
        let updated = tx.execute(
            "UPDATE income_sources SET active = 0, archived_at = ?2 WHERE id = ?1 AND deleted_at IS NULL",
            params![id.as_uuid(), Utc::now().to_rfc3339()],
        )?;
        if updated == 0 {
            return Err(DbError::InvalidCommand(
                "income source does not exist".to_owned(),
            ));
        }
        id.as_uuid()
    };
    Ok(entity_id)
}

/// Applies [`WriteCommand::RestoreIncomeSource`].
pub(crate) fn apply_restore_income_source(
    tx: &Connection,
    id: &IncomeSourceId,
) -> Result<Uuid, DbError> {
    let entity_id = {
        let updated = tx.execute(
            "UPDATE income_sources SET active = 1, archived_at = NULL WHERE id = ?1 AND deleted_at IS NULL",
            [id.as_uuid()],
        )?;
        if updated == 0 {
            return Err(DbError::InvalidCommand(
                "income source does not exist".to_owned(),
            ));
        }
        id.as_uuid()
    };
    Ok(entity_id)
}

/// Applies [`WriteCommand::CreateRecurringBill`].
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_create_recurring_bill(
    tx: &Connection,
    event_id: &RecurringEventId,
    contract_id: &BillContractId,
    name: &str,
    amount: &Money,
    bill_type: &str,
    frequency: &Frequency,
    anchor: &NaiveDate,
    autopay_account_id: &Option<AccountId>,
    description: &Option<String>,
    source_merchant_key: &Option<String>,
    category_id: &Option<CategoryId>,
    tag_ids: &[TagId],
) -> Result<Uuid, DbError> {
    let entity_id = {
        if name.trim().is_empty() {
            return Err(DbError::InvalidCommand(
                "recurring bill name must not be empty".to_owned(),
            ));
        }
        if amount.is_zero() || amount.minor_units() < 0 {
            return Err(DbError::InvalidCommand(
                "recurring bill amount must be positive".to_owned(),
            ));
        }
        // If an autopay account is named, it must exist with a matching currency.
        if let Some(account_id) = autopay_account_id {
            let currency: Option<String> = tx
                .query_row(
                    "SELECT currency FROM accounts WHERE id = ?1",
                    [account_id.as_uuid()],
                    |r| r.get(0),
                )
                .optional()?;
            match currency {
                None => {
                    return Err(DbError::InvalidCommand(
                        "recurring bill references an unknown autopay account".to_owned(),
                    ))
                }
                Some(code) if amount.currency() != currency_from_code(&code)? => {
                    return Err(DbError::InvalidCommand(
                        "recurring bill currency does not match the autopay account".to_owned(),
                    ))
                }
                Some(_) => {}
            }
        }

        let now = Utc::now().to_rfc3339();
        let anchor_str = anchor.to_string();
        // The schedule is stored on recurring_events; the contract also carries
        // it as an opaque due rule so the commitments projection (and 164u) can
        // generate occurrences without joining back.
        let due_rule_json = format!(
            "{{\"frequency\":\"{}\",\"anchor\":\"{}\"}}",
            frequency.token(),
            anchor_str
        );

        tx.execute(
            "INSERT INTO recurring_events (
                    id, name, source, amount_expected_minor, currency, frequency,
                    next_expected_date, autopay_account_id, include_in_forecast,
                    is_active, source_merchant_key, category_id, created_at, updated_at
                ) VALUES (?1, ?2, 'manual', ?3, ?4, ?5, ?6, ?7, 1, 1, ?8, ?9, ?10, ?10)",
            params![
                event_id.as_uuid(),
                name,
                amount.minor_units(),
                amount.currency().code(),
                frequency.token(),
                anchor_str,
                autopay_account_id.map(|a| a.as_uuid()),
                source_merchant_key
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty()),
                category_id.map(|c| c.as_uuid()),
                now,
            ],
        )?;
        tx.execute(
            "INSERT INTO bill_contracts (
                    id, name, type, expected_amount_minor, currency, cadence,
                    due_rule_json, autopay_enabled, autopay_account_id,
                    recurring_event_id, description, status, include_in_forecast,
                    created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'active', 1, ?12, ?12)",
            params![
                contract_id.as_uuid(),
                name,
                bill_type,
                amount.minor_units(),
                amount.currency().code(),
                frequency.token(),
                due_rule_json,
                // Autopay intent is unknown at creation (ADR 0041): set via SetBillAutopay,
                // which keeps this mirror consistent with recurring_events.autopay_enabled.
                None::<i64>,
                autopay_account_id.map(|a| a.as_uuid()),
                event_id.as_uuid(),
                description,
                now,
            ],
        )?;

        // Tags applied at promotion (ADR 0033 addendum, personal-cfo-4d8.24.5.1): each
        // must exist in the shared `tags` vocabulary; OR IGNORE dedupes the join PK.
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
                "INSERT OR IGNORE INTO recurring_event_tags (recurring_event_id, tag_id)
                     VALUES (?1, ?2)",
                params![event_id.as_uuid(), tag_id.as_uuid()],
            )?;
        }

        // Refresh the forecast-facing commitments projection atomically with
        // the bill write (personal-cfo-esmy / -rxw).
        commitments::rebuild_in(tx)?;

        event_id.as_uuid()
    };
    Ok(entity_id)
}

/// Applies [`WriteCommand::SetBillAutopay`].
pub(crate) fn apply_set_bill_autopay(
    tx: &Connection,
    event_id: &RecurringEventId,
    autopay: &bool,
) -> Result<Uuid, DbError> {
    let entity_id = {
        // Autopay intent (ADR 0041, personal-cfo-mc7f): authoritative on recurring_events,
        // mirrored to bill_contracts so the commitments projection reflects intent. Does not
        // touch the forecast projection.
        let flag = i64::from(*autopay);
        let now = Utc::now().to_rfc3339();
        let updated = tx.execute(
            "UPDATE recurring_events SET autopay_enabled = ?2, updated_at = ?3
                 WHERE id = ?1 AND deleted_at IS NULL",
            params![event_id.as_uuid(), flag, now],
        )?;
        if updated == 0 {
            return Err(DbError::InvalidCommand(
                "recurring bill does not exist".to_owned(),
            ));
        }
        tx.execute(
            "UPDATE bill_contracts SET autopay_enabled = ?2, updated_at = ?3
                 WHERE recurring_event_id = ?1 AND deleted_at IS NULL",
            params![event_id.as_uuid(), flag, now],
        )?;
        commitments::rebuild_in(tx)?;
        event_id.as_uuid()
    };
    Ok(entity_id)
}

/// Applies [`WriteCommand::DismissRecurringSuggestion`] (ADR 0046, personal-cfo-4d8.24.6):
/// upsert the `(merchant_key, currency)` suppression with the dismissed amount + cadence,
/// latest-dismiss-wins. Returns a fresh op-log id (the suppression is keyless).
pub(crate) fn apply_dismiss_recurring_suggestion(
    tx: &Connection,
    merchant_key: &str,
    currency: &str,
    amount_minor: i64,
    frequency: &str,
    reason: &Option<String>,
) -> Result<Uuid, DbError> {
    tx.execute(
        "INSERT INTO recurring_suggestion_suppressions
                (merchant_key, currency, amount_minor, frequency, reason, dismissed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(merchant_key, currency) DO UPDATE SET
                amount_minor = excluded.amount_minor,
                frequency    = excluded.frequency,
                reason       = excluded.reason,
                dismissed_at = excluded.dismissed_at",
        params![
            merchant_key.trim(),
            currency.trim(),
            amount_minor,
            frequency.trim(),
            reason.as_deref().map(str::trim).filter(|s| !s.is_empty()),
            Utc::now().to_rfc3339(),
        ],
    )?;
    Ok(Uuid::now_v7())
}

/// Applies [`WriteCommand::UpdateRecurringBill`].
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_update_recurring_bill(
    tx: &Connection,
    event_id: &RecurringEventId,
    name: &str,
    amount: &Money,
    bill_type: &str,
    frequency: &Frequency,
    anchor: &NaiveDate,
    autopay_account_id: &Option<AccountId>,
    description: &Option<String>,
) -> Result<Uuid, DbError> {
    let entity_id = {
        if name.trim().is_empty() {
            return Err(DbError::InvalidCommand(
                "recurring bill name must not be empty".to_owned(),
            ));
        }
        if amount.is_zero() || amount.minor_units() < 0 {
            return Err(DbError::InvalidCommand(
                "recurring bill amount must be positive".to_owned(),
            ));
        }
        // If an autopay account is named, it must exist with a matching currency.
        if let Some(account_id) = autopay_account_id {
            let currency: Option<String> = tx
                .query_row(
                    "SELECT currency FROM accounts WHERE id = ?1",
                    [account_id.as_uuid()],
                    |r| r.get(0),
                )
                .optional()?;
            match currency {
                None => {
                    return Err(DbError::InvalidCommand(
                        "recurring bill references an unknown autopay account".to_owned(),
                    ))
                }
                Some(code) if amount.currency() != currency_from_code(&code)? => {
                    return Err(DbError::InvalidCommand(
                        "recurring bill currency does not match the autopay account".to_owned(),
                    ))
                }
                Some(_) => {}
            }
        }

        let now = Utc::now().to_rfc3339();
        let anchor_str = anchor.to_string();
        let due_rule_json = format!(
            "{{\"frequency\":\"{}\",\"anchor\":\"{}\"}}",
            frequency.token(),
            anchor_str
        );

        // Rewrite the schedule (recurring_events) and the contract
        // (bill_contracts) keyed by the recurring-event id. A missing event
        // means the bill does not exist — fail before touching the contract.
        let updated = tx.execute(
            "UPDATE recurring_events
                 SET name = ?2, amount_expected_minor = ?3, currency = ?4,
                     frequency = ?5, next_expected_date = ?6, autopay_account_id = ?7,
                     updated_at = ?8
                 WHERE id = ?1 AND deleted_at IS NULL",
            params![
                event_id.as_uuid(),
                name,
                amount.minor_units(),
                amount.currency().code(),
                frequency.token(),
                anchor_str,
                autopay_account_id.map(|a| a.as_uuid()),
                now,
            ],
        )?;
        if updated == 0 {
            return Err(DbError::InvalidCommand(
                "recurring bill does not exist".to_owned(),
            ));
        }
        tx.execute(
            // Autopay intent (autopay_enabled) is owned by SetBillAutopay (ADR 0041) and left
            // untouched here — editing a bill's name/amount/schedule must not reset it.
            "UPDATE bill_contracts
                 SET name = ?2, type = ?3, expected_amount_minor = ?4, currency = ?5,
                     cadence = ?6, due_rule_json = ?7,
                     autopay_account_id = ?8, description = ?9, updated_at = ?10
                 WHERE recurring_event_id = ?1 AND deleted_at IS NULL",
            params![
                event_id.as_uuid(),
                name,
                bill_type,
                amount.minor_units(),
                amount.currency().code(),
                frequency.token(),
                due_rule_json,
                autopay_account_id.map(|a| a.as_uuid()),
                description,
                now,
            ],
        )?;

        // Refresh the forecast-facing commitments projection atomically.
        commitments::rebuild_in(tx)?;

        event_id.as_uuid()
    };
    Ok(entity_id)
}

/// Set or clear the tombstone on a bill's schedule (`recurring_events`) and its contract
/// (`bill_contracts`) and refresh the commitments projection. `at` is the command's
/// `issued_at` to tombstone, or `None` to reinstate. One function serves both directions so
/// the projection refresh has a single call site. Doing nothing because the bill is already in
/// the requested state is an accepted, logged no-op; a bill that does not exist is an error.
fn set_bill_tombstone(
    tx: &Connection,
    event_id: &RecurringEventId,
    at: Option<String>,
) -> Result<Uuid, DbError> {
    let id = event_id.as_uuid();
    let (event_sql, contract_sql) = if at.is_some() {
        (
            "UPDATE recurring_events SET deleted_at = ?2 WHERE id = ?1 AND deleted_at IS NULL",
            "UPDATE bill_contracts SET deleted_at = ?2
                 WHERE recurring_event_id = ?1 AND deleted_at IS NULL",
        )
    } else {
        (
            "UPDATE recurring_events SET deleted_at = ?2 WHERE id = ?1 AND deleted_at IS NOT NULL",
            "UPDATE bill_contracts SET deleted_at = ?2
                 WHERE recurring_event_id = ?1 AND deleted_at IS NOT NULL",
        )
    };
    let changed = tx.execute(event_sql, params![id, at])?;
    if changed == 0 {
        if tombstone_state(tx, "recurring_events", "id", id)?.is_none() {
            return Err(DbError::InvalidCommand(
                "recurring bill does not exist".to_owned(),
            ));
        }
    } else {
        tx.execute(contract_sql, params![id, at])?;
        // Refresh the forecast-facing commitments projection atomically.
        commitments::rebuild_in(tx)?;
    }
    Ok(id)
}

/// Applies [`WriteCommand::DeleteRecurringBill`]: a tombstone on the schedule and on its
/// contract, both set to the command's `issued_at`, not two `DELETE`s; see
/// [`apply_delete_recurring_transfer`]. Deleting a bill that is already tombstoned is an
/// accepted, logged no-op.
pub(crate) fn apply_delete_recurring_bill(
    tx: &Connection,
    meta: &CommandMeta,
    event_id: &RecurringEventId,
) -> Result<Uuid, DbError> {
    let id = set_bill_tombstone(tx, event_id, Some(issued_at_rfc3339(meta)?))?;
    // A link only ever sits on an active bill (personal-cfo-6wk.22): deleting the bill returns
    // the loan's own payment to the forecast, and a reinstated bill comes back unlinked.
    clear_bill_link(tx, event_id)?;
    Ok(id)
}

/// Applies [`WriteCommand::ReinstateRecurringBill`]: clears the tombstone on the schedule and
/// its contract (the archive flag is untouched) and puts the bill back in the commitments
/// projection. A bill that is not deleted is an accepted, logged no-op.
pub(crate) fn apply_reinstate_recurring_bill(
    tx: &Connection,
    event_id: &RecurringEventId,
) -> Result<Uuid, DbError> {
    set_bill_tombstone(tx, event_id, None)
}

/// Applies [`WriteCommand::ArchiveRecurringBill`].
pub(crate) fn apply_archive_recurring_bill(
    tx: &Connection,
    event_id: &RecurringEventId,
) -> Result<Uuid, DbError> {
    let entity_id = {
        let now = Utc::now().to_rfc3339();
        let updated = tx.execute(
            "UPDATE recurring_events
                 SET is_active = 0, archived_at = ?2, updated_at = ?2
                 WHERE id = ?1 AND deleted_at IS NULL",
            params![event_id.as_uuid(), now],
        )?;
        if updated == 0 {
            return Err(DbError::InvalidCommand(
                "recurring bill does not exist".to_owned(),
            ));
        }
        // Archiving drops it from the commitments projection (is_active = 0).
        commitments::rebuild_in(tx)?;
        // A link only ever sits on an active bill (personal-cfo-6wk.22): archiving the bill
        // returns the loan's own payment to the forecast, and a restored bill comes back unlinked.
        clear_bill_link(tx, event_id)?;
        event_id.as_uuid()
    };
    Ok(entity_id)
}

/// Applies [`WriteCommand::RestoreRecurringBill`].
pub(crate) fn apply_restore_recurring_bill(
    tx: &Connection,
    event_id: &RecurringEventId,
) -> Result<Uuid, DbError> {
    let entity_id = {
        let now = Utc::now().to_rfc3339();
        let updated = tx.execute(
            "UPDATE recurring_events
                 SET is_active = 1, archived_at = NULL, updated_at = ?2
                 WHERE id = ?1 AND deleted_at IS NULL",
            params![event_id.as_uuid(), now],
        )?;
        if updated == 0 {
            return Err(DbError::InvalidCommand(
                "recurring bill does not exist".to_owned(),
            ));
        }
        // Restoring returns it to the commitments projection / forecast.
        commitments::rebuild_in(tx)?;
        event_id.as_uuid()
    };
    Ok(entity_id)
}

// ---- bill <-> loan link (personal-cfo-6wk.22; ADR 0035 addendum 2026-10-25) --------------------
//
// Deliberately free of the wall clock and of `updated_at`: these commands write only the two
// link columns, so replaying one on another device produces the same row.

/// Pay-in-full philosophies project a single payoff, not a monthly payment, so a monthly bill is
/// not that payment and the link is never offered (ADR 0035 addendum A2).
const PAY_IN_FULL_PHILOSOPHIES: &[&str] = &[
    "pay_in_full",
    "pay_statement_balance",
    "pay_current_balance",
];

/// The loan's name, after checking it is a loan this bill may be linked to: an active
/// `loan_liability` account in the bill's currency with a recurring payment.
fn require_linkable_loan(
    tx: &Connection,
    loan: &AccountId,
    bill_currency: &str,
) -> Result<String, DbError> {
    /// What linking needs to know about the target account.
    struct Target {
        name: String,
        role: String,
        active: i64,
        currency: String,
        due_day: Option<i64>,
        philosophy: Option<String>,
    }
    let target: Option<Target> = tx
        .query_row(
            "SELECT a.name, a.cashflow_role, a.active, a.currency,
                    dt.payment_due_day, dt.repayment_philosophy
               FROM accounts a
               LEFT JOIN debt_terms dt ON dt.account_id = a.id
              WHERE a.id = ?1",
            [loan.as_uuid()],
            |r| {
                Ok(Target {
                    name: r.get(0)?,
                    role: r.get(1)?,
                    active: r.get(2)?,
                    currency: r.get(3)?,
                    due_day: r.get(4)?,
                    philosophy: r.get(5)?,
                })
            },
        )
        .optional()?;
    let Some(Target {
        name,
        role,
        active,
        currency,
        due_day,
        philosophy,
    }) = target
    else {
        return Err(DbError::InvalidCommand(
            "loan account does not exist".to_owned(),
        ));
    };
    if role != "loan_liability" {
        return Err(DbError::InvalidCommand(
            "a bill can only be linked to a loan account".to_owned(),
        ));
    }
    if active == 0 {
        return Err(DbError::InvalidCommand(
            "that loan account is archived".to_owned(),
        ));
    }
    if currency != bill_currency {
        return Err(DbError::InvalidCommand(
            "the bill and the loan are in different currencies".to_owned(),
        ));
    }
    if due_day.is_none() {
        return Err(DbError::InvalidCommand(
            "that loan has no payment due day set, so it has no payment to link".to_owned(),
        ));
    }
    if philosophy
        .as_deref()
        .is_some_and(|p| PAY_IN_FULL_PHILOSOPHIES.contains(&p))
    {
        return Err(DbError::InvalidCommand(
            "that loan is paid in full once, not monthly, so a bill cannot be its payment"
                .to_owned(),
        ));
    }
    Ok(name)
}

/// Applies [`WriteCommand::LinkBillToLoan`]. Linking a bill to the loan it is already linked to is
/// an accepted no-op; linking it to another loan replaces the link. Recording the link also drops a
/// "not this loan's payment" answer for the same loan (the user changed their mind).
pub(crate) fn apply_link_bill_to_loan(
    tx: &Connection,
    event_id: &RecurringEventId,
    loan: &AccountId,
) -> Result<Uuid, DbError> {
    let id = event_id.as_uuid();
    let bill: Option<(String, i64)> = tx
        .query_row(
            "SELECT currency, is_active FROM recurring_events
              WHERE id = ?1 AND deleted_at IS NULL",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((currency, is_active)) = bill else {
        return Err(DbError::InvalidCommand(
            "recurring bill does not exist".to_owned(),
        ));
    };
    if is_active == 0 {
        return Err(DbError::InvalidCommand(
            "an archived bill cannot be linked to a loan".to_owned(),
        ));
    }
    require_linkable_loan(tx, loan, &currency)?;
    let other: Option<(Uuid, String)> = tx
        .query_row(
            "SELECT id, name FROM recurring_events
              WHERE linked_loan_account_id = ?1 AND id <> ?2
                AND is_active = 1 AND deleted_at IS NULL
              ORDER BY id LIMIT 1",
            params![loan.as_uuid(), id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if let Some((bill_id, bill_name)) = other {
        return Err(DbError::LoanAlreadyLinked { bill_id, bill_name });
    }
    tx.execute(
        "UPDATE recurring_events
            SET linked_loan_account_id = ?2,
                not_payment_of_loan_account_id =
                    CASE WHEN not_payment_of_loan_account_id = ?2
                         THEN NULL ELSE not_payment_of_loan_account_id END
          WHERE id = ?1",
        params![id, loan.as_uuid()],
    )?;
    Ok(id)
}

/// Clears a bill's link, if any. Writes nothing when there is none, so the command leaves an
/// empty write set.
pub(crate) fn clear_bill_link(tx: &Connection, event_id: &RecurringEventId) -> Result<(), DbError> {
    tx.execute(
        "UPDATE recurring_events SET linked_loan_account_id = NULL
          WHERE id = ?1 AND linked_loan_account_id IS NOT NULL",
        [event_id.as_uuid()],
    )?;
    Ok(())
}

/// Clears every link to `loan` (personal-cfo-6wk.22): called by the command that archives the
/// loan, in the SAME transaction, so it is part of that command's write set and replicates. A
/// reinstated loan therefore comes back unlinked and its own payment counts again. Deterministic
/// and idempotent; the same statement is the rule a replica applies to a dangling link.
pub(crate) fn clear_links_to_loan(tx: &Connection, loan: &AccountId) -> Result<(), DbError> {
    tx.execute(
        "UPDATE recurring_events SET linked_loan_account_id = NULL
          WHERE linked_loan_account_id = ?1",
        [loan.as_uuid()],
    )?;
    Ok(())
}

/// Applies [`WriteCommand::UnlinkBillFromLoan`]. A bill that is not linked is an accepted, logged
/// no-op.
pub(crate) fn apply_unlink_bill_from_loan(
    tx: &Connection,
    event_id: &RecurringEventId,
) -> Result<Uuid, DbError> {
    let id = event_id.as_uuid();
    let exists: Option<i64> = tx
        .query_row(
            "SELECT 1 FROM recurring_events WHERE id = ?1 AND deleted_at IS NULL",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    if exists.is_none() {
        return Err(DbError::InvalidCommand(
            "recurring bill does not exist".to_owned(),
        ));
    }
    clear_bill_link(tx, event_id)?;
    Ok(id)
}

/// Applies [`WriteCommand::SetBillNotLoanPayment`]. `Some(loan)` must name an existing loan
/// account (an archived one is fine: the answer is about the pair); if the bill is linked to that
/// loan the link is dropped (the two answers contradict). `None` clears the answer.
pub(crate) fn apply_set_bill_not_loan_payment(
    tx: &Connection,
    event_id: &RecurringEventId,
    loan: &Option<AccountId>,
) -> Result<Uuid, DbError> {
    let id = event_id.as_uuid();
    let exists: Option<i64> = tx
        .query_row(
            "SELECT 1 FROM recurring_events WHERE id = ?1 AND deleted_at IS NULL",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    if exists.is_none() {
        return Err(DbError::InvalidCommand(
            "recurring bill does not exist".to_owned(),
        ));
    }
    match loan {
        Some(loan) => {
            let role: Option<String> = tx
                .query_row(
                    "SELECT cashflow_role FROM accounts WHERE id = ?1",
                    [loan.as_uuid()],
                    |r| r.get(0),
                )
                .optional()?;
            if role.as_deref() != Some("loan_liability") {
                return Err(DbError::InvalidCommand(
                    "that is not a loan account".to_owned(),
                ));
            }
            tx.execute(
                "UPDATE recurring_events
                    SET not_payment_of_loan_account_id = ?2,
                        linked_loan_account_id =
                            CASE WHEN linked_loan_account_id = ?2
                                 THEN NULL ELSE linked_loan_account_id END
                  WHERE id = ?1",
                params![id, loan.as_uuid()],
            )?;
        }
        None => {
            tx.execute(
                "UPDATE recurring_events SET not_payment_of_loan_account_id = NULL
                  WHERE id = ?1 AND not_payment_of_loan_account_id IS NOT NULL",
                [id],
            )?;
        }
    }
    Ok(id)
}
