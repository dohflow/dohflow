//! `apply_command` arms for account lifecycle commands (moved verbatim
//! from `lib.rs`).

use chrono::Utc;
use core_ledger::{
    Account, AccountId, AccountKind, AccountSubtype, LedgerTransaction, OperationId, Posting,
    TransactionId,
};
use core_money::Money;
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::{
    ensure_system_ledger_account, insert_ledger_account, persist_transaction, CommandMeta, DbError,
};

/// Applies [`WriteCommand::CreateAccount`].
pub(crate) fn apply_create_account(
    tx: &Connection,
    meta: &CommandMeta,
    account: &Account,
    opening_balance: &Option<Money>,
) -> Result<Uuid, DbError> {
    let entity_id = {
        if let Some(ob) = opening_balance {
            if ob.currency() != account.currency() {
                return Err(DbError::InvalidCommand(
                    "opening balance currency does not match account currency".to_owned(),
                ));
            }
        }

        let ledger_account_id = account.ledger_account_id();
        insert_ledger_account(
            tx,
            ledger_account_id,
            account.cashflow_role().account_kind(),
            account.currency(),
        )?;

        let flags = account.flags();
        tx.execute(
            "INSERT INTO accounts (
                    id, ledger_account_id, name, cashflow_role, normal_balance, currency,
                    retirement, tax_advantaged, joint, business, active, subtype,
                    last_synced, manual_balance_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, NULL, NULL)",
            params![
                account.id().as_uuid(),
                ledger_account_id.as_uuid(),
                account.name(),
                account.cashflow_role().as_str(),
                account.normal_balance().as_str(),
                account.currency().code(),
                flags.retirement,
                flags.tax_advantaged,
                flags.joint,
                flags.business,
                flags.active,
                account.subtype().map(AccountSubtype::as_str),
            ],
        )?;

        // Opening balance is an equity posting, never a column (plan §9.1).
        if let Some(ob) = opening_balance {
            if !ob.is_zero() {
                let equity_id = ensure_system_ledger_account(
                    tx,
                    "opening_balance_equity",
                    ob.currency(),
                    AccountKind::Equity,
                )?;
                let txn = LedgerTransaction::new(
                    TransactionId::new(),
                    OperationId::from_uuid(meta.command_id),
                    Utc::now(),
                    vec![
                        Posting::new(ledger_account_id, *ob),
                        Posting::new(equity_id, ob.checked_neg()?),
                    ],
                )
                .map_err(|e| DbError::InvalidCommand(e.to_string()))?;
                persist_transaction(tx, &txn)?;
            }
        }

        account.id().as_uuid()
    };
    Ok(entity_id)
}

/// Applies [`WriteCommand::UpdateAccount`].
pub(crate) fn apply_update_account(
    tx: &Connection,
    id: &AccountId,
    name: &str,
) -> Result<Uuid, DbError> {
    let entity_id = {
        tx.execute(
            "UPDATE accounts SET name = ?1 WHERE id = ?2",
            params![name, id.as_uuid()],
        )?;
        id.as_uuid()
    };
    Ok(entity_id)
}

/// Applies [`WriteCommand::ArchiveAccount`].
pub(crate) fn apply_archive_account(tx: &Connection, id: &AccountId) -> Result<Uuid, DbError> {
    let entity_id = {
        tx.execute(
            "UPDATE accounts SET active = 0 WHERE id = ?1",
            [id.as_uuid()],
        )?;
        id.as_uuid()
    };
    Ok(entity_id)
}

/// Applies [`WriteCommand::ReinstateAccount`].
pub(crate) fn apply_reinstate_account(tx: &Connection, id: &AccountId) -> Result<Uuid, DbError> {
    let entity_id = {
        tx.execute(
            "UPDATE accounts SET active = 1 WHERE id = ?1",
            [id.as_uuid()],
        )?;
        id.as_uuid()
    };
    Ok(entity_id)
}

/// Applies [`WriteCommand::SetAccountSubtype`].
pub(crate) fn apply_set_account_subtype(
    tx: &Connection,
    id: &AccountId,
    subtype: &Option<AccountSubtype>,
) -> Result<Uuid, DbError> {
    let entity_id = {
        // Enforce the role↔subtype match here (ADR 0028): the schema CHECK only
        // constrains the token set, so look up the account's current role and
        // reject a mismatch. A missing account is an invalid command.
        let role: Option<String> = tx
            .query_row(
                "SELECT cashflow_role FROM accounts WHERE id = ?1",
                [id.as_uuid()],
                |r| r.get(0),
            )
            .optional()?;
        let Some(role) = role else {
            return Err(DbError::InvalidCommand("account does not exist".to_owned()));
        };
        if let Some(subtype) = subtype {
            if subtype.role().as_str() != role {
                return Err(DbError::InvalidCommand(format!(
                    "account subtype '{}' does not belong to this account type",
                    subtype.as_str()
                )));
            }
        }
        tx.execute(
            "UPDATE accounts SET subtype = ?1 WHERE id = ?2",
            params![subtype.map(AccountSubtype::as_str), id.as_uuid()],
        )?;
        id.as_uuid()
    };
    Ok(entity_id)
}

/// Applies [`WriteCommand::SetAccountNote`] (ADR 0044): set or clear (`None`) a free-text
/// note on an account. A blank note is stored as `NULL` (cleared).
pub(crate) fn apply_set_account_note(
    tx: &Connection,
    id: &AccountId,
    note: Option<&str>,
) -> Result<Uuid, DbError> {
    let normalized = note.map(str::trim).filter(|s| !s.is_empty());
    tx.execute(
        "UPDATE accounts SET notes = ?1 WHERE id = ?2",
        params![normalized, id.as_uuid()],
    )?;
    Ok(id.as_uuid())
}

/// Applies [`WriteCommand::SetAccountLink`] (ADR 0044 §5): link a real asset to the
/// liability that finances it, or clear the link (`None`). The link lives on the asset
/// row. Validation (roles need a lookup, so it happens here, not in the kernel):
/// the source must be a `real_asset`, the target a `loan_liability` (a home/vehicle is
/// financed by a loan, not a revolving credit card — ADR 0044 §5, personal-cfo-4d8.23.4),
/// and an account cannot link to itself.
pub(crate) fn apply_set_account_link(
    tx: &Connection,
    asset_id: &AccountId,
    liability_id: Option<&AccountId>,
) -> Result<Uuid, DbError> {
    let role_of = |id: &AccountId| -> Result<String, DbError> {
        tx.query_row(
            "SELECT cashflow_role FROM accounts WHERE id = ?1",
            [id.as_uuid()],
            |r| r.get::<_, String>(0),
        )
        .optional()?
        .ok_or_else(|| DbError::InvalidCommand("account does not exist".to_owned()))
    };

    if role_of(asset_id)? != "real_asset" {
        return Err(DbError::InvalidCommand(
            "only a property or vehicle account can be linked to a loan".to_owned(),
        ));
    }

    if let Some(liability_id) = liability_id {
        if liability_id == asset_id {
            return Err(DbError::InvalidCommand(
                "an account cannot be linked to itself".to_owned(),
            ));
        }
        if role_of(liability_id)? != "loan_liability" {
            return Err(DbError::InvalidCommand(
                "an asset can only be linked to a loan or mortgage".to_owned(),
            ));
        }
        // One-to-one (ADR 0044 §5): reject a liability already linked by a DIFFERENT
        // asset, so a liability resolves to exactly one asset (keeps the reverse lookup
        // deterministic instead of an arbitrary `LIMIT 1`).
        let already_linked: Option<Uuid> = tx
            .query_row(
                "SELECT id FROM accounts WHERE linked_account_id = ?1 AND id <> ?2 LIMIT 1",
                params![liability_id.as_uuid(), asset_id.as_uuid()],
                |r| r.get(0),
            )
            .optional()?;
        if already_linked.is_some() {
            return Err(DbError::InvalidCommand(
                "that loan is already linked to another asset".to_owned(),
            ));
        }
    }

    tx.execute(
        "UPDATE accounts SET linked_account_id = ?1 WHERE id = ?2",
        params![liability_id.map(|a| a.as_uuid()), asset_id.as_uuid()],
    )?;
    Ok(asset_id.as_uuid())
}
