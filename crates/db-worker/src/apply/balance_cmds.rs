//! `apply_command` arms for balance observations (personal-cfo-5ymg8.3; ADR 0027; ADR 0074
//! decision 5). A balance observation is class 1: an account's balance as of a day, either typed by
//! the user (`manual`) or reported by a connector (`connector_sync`).
//!
//! * [`apply_record_balance_assertion`] appends a manual observation. Two assertions for one
//!   account and day are both kept (never last-wins; ADR 0027), the latest by `created_at` anchors
//!   the balance, and the rebase disposition queues a choice.
//! * [`apply_record_connector_balance`] is an UPSERT on the natural key (account, day, connector):
//!   the row is updated in place, never deleted and re-inserted, so a replicated row is never hard
//!   deleted and one device's refresh cannot leave a duplicate behind. It is **monotone in
//!   `issued_at`**: a command older than the stored row's `created_at` changes nothing, so a stale
//!   refresh from an offline device cannot overwrite a newer same-day reading. The row's id is a
//!   content id of its natural key (independent of the command), and `source_record_id` is NULL on
//!   every device: a staged source record is device-local and would fork the row's digest.

use chrono::NaiveDate;
use core_ids::DerivedIds;
use core_ledger::AccountId;
use core_money::Money;
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::{currency_from_code, CommandMeta, DbError, RejectCode};

/// The account's currency, or the refusal.
fn account_currency(tx: &Connection, account: &AccountId, amount: &Money) -> Result<(), DbError> {
    let currency: Option<String> = tx
        .query_row(
            "SELECT currency FROM accounts WHERE id = ?1",
            [account.as_uuid()],
            |r| r.get(0),
        )
        .optional()?;
    let Some(currency) = currency else {
        return Err(DbError::reject(
            RejectCode::ReferenceMissing,
            "no such account".to_owned(),
        ));
    };
    if amount.currency() != currency_from_code(&currency)? {
        return Err(DbError::reject(
            RejectCode::StateConflict,
            "assertion currency must match the account currency".to_owned(),
        ));
    }
    Ok(())
}

/// Applies [`WriteCommand::RecordBalanceAssertion`](crate::WriteCommand).
pub(crate) fn apply_record_balance_assertion(
    tx: &Connection,
    meta: &CommandMeta,
    id: &Uuid,
    account_id: &AccountId,
    amount: &Money,
    as_of: &NaiveDate,
) -> Result<Uuid, DbError> {
    account_currency(tx, account_id, amount)?;
    let taken: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM balance_observations WHERE id = ?1)",
        params![id],
        |r| r.get(0),
    )?;
    if taken {
        return Err(DbError::reject(
            RejectCode::StateConflict,
            "that balance observation id is already in use".to_owned(),
        ));
    }
    tx.execute(
        "INSERT INTO balance_observations
            (id, account_id, observed_at, balance_amount_minor, balance_currency,
             source, source_record_id, reconciliation_session_id, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'manual', NULL, NULL, ?6)",
        params![
            id,
            account_id.as_uuid(),
            as_of.to_string(),
            amount.minor_units(),
            amount.currency().code(),
            crate::apply::issued_at_text(meta),
        ],
    )?;
    Ok(account_id.as_uuid())
}

/// Applies [`WriteCommand::RecordConnectorBalance`](crate::WriteCommand).
pub(crate) fn apply_record_connector_balance(
    tx: &Connection,
    meta: &CommandMeta,
    ids: &DerivedIds,
    account_id: &AccountId,
    observed_at: &NaiveDate,
    amount: &Money,
) -> Result<Uuid, DbError> {
    account_currency(tx, account_id, amount)?;
    // A balance cannot be observed in the household's future: "today" is the household-local date
    // of the command's `issued_at`, never the wall clock, so a replica reaches the same verdict.
    if *observed_at > crate::forecast::household_today_at(tx, crate::apply::issued_at(meta))? {
        return Err(DbError::reject(
            RejectCode::StateConflict,
            "a balance cannot be observed in the future".to_owned(),
        ));
    }
    let day = observed_at.to_string();
    let at = crate::apply::issued_at_text(meta);
    // The one connector row for this account and day, whatever id it carries (rows written before
    // this command existed have random ids).
    let existing: Option<(Uuid, String)> = tx
        .query_row(
            "SELECT id, created_at FROM balance_observations
              WHERE account_id = ?1 AND observed_at = ?2 AND source = 'connector_sync'
              ORDER BY created_at DESC, id DESC LIMIT 1",
            params![account_id.as_uuid(), day],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    match existing {
        Some((id, created_at)) => {
            // Monotone: only a newer command moves the row.
            if created_at < at {
                tx.execute(
                    "UPDATE balance_observations
                        SET balance_amount_minor = ?2, balance_currency = ?3, created_at = ?4,
                            source_record_id = NULL, reconciliation_session_id = NULL
                      WHERE id = ?1",
                    params![id, amount.minor_units(), amount.currency().code(), at],
                )?;
            }
        }
        None => {
            // Under a base-dependent branch, so a content id of the natural key (no ordinal).
            let id = ids.content(
                "balance-obs.connector",
                &[account_id.as_uuid().as_bytes(), day.as_bytes()],
            )?;
            tx.execute(
                "INSERT INTO balance_observations
                    (id, account_id, observed_at, balance_amount_minor, balance_currency,
                     source, source_record_id, reconciliation_session_id, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'connector_sync', NULL, NULL, ?6)",
                params![
                    id,
                    account_id.as_uuid(),
                    day,
                    amount.minor_units(),
                    amount.currency().code(),
                    at
                ],
            )?;
        }
    }
    Ok(account_id.as_uuid())
}
