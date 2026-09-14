//! `apply_command` arms for the category tree and transaction
//! recategorization (moved verbatim from `lib.rs`).

use chrono::Utc;
use core_ledger::{CategoryId, TransactionId};
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::{
    default_forecast_behavior, require_user_category, would_create_category_cycle, DbError,
};

/// Applies [`WriteCommand::CreateCategory`].
pub(crate) fn apply_create_category(
    tx: &Connection,
    id: &CategoryId,
    parent_id: &Option<CategoryId>,
    name: &str,
    category_type: &str,
    color: &Option<String>,
    icon: &Option<String>,
) -> Result<Uuid, DbError> {
    let entity_id = {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(DbError::InvalidCommand(
                "category name must not be empty".to_owned(),
            ));
        }
        if let Some(parent) = parent_id {
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM categories WHERE id = ?1)",
                [parent.as_uuid()],
                |r| r.get(0),
            )?;
            if !exists {
                return Err(DbError::InvalidCommand(
                    "parent category does not exist".to_owned(),
                ));
            }
        }
        let now = Utc::now().to_rfc3339();
        // Always a user category; behavior derived from the type (ADR 0030). `icon` is the
        // optional emoji set at creation (personal-cfo-kogu).
        tx.execute(
            "INSERT INTO categories (
                    id, household_id, parent_id, name, type, icon, color, is_system,
                    budget_default_minor, forecast_behavior, created_at, updated_at
                ) VALUES (?1, NULL, ?2, ?3, ?4, ?5, ?6, 0, NULL, ?7, ?8, ?8)",
            params![
                id.as_uuid(),
                parent_id.map(|p| p.as_uuid()),
                trimmed,
                category_type,
                icon,
                color,
                default_forecast_behavior(category_type),
                now,
            ],
        )?;
        id.as_uuid()
    };
    Ok(entity_id)
}

/// Applies [`WriteCommand::UpdateCategory`].
pub(crate) fn apply_update_category(
    tx: &Connection,
    id: &CategoryId,
    name: &str,
    color: &Option<String>,
    icon: &Option<String>,
) -> Result<Uuid, DbError> {
    let entity_id = {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(DbError::InvalidCommand(
                "category name must not be empty".to_owned(),
            ));
        }
        // A system category's IDENTITY (name/type/parent) is immutable, but its APPEARANCE
        // (color + icon) is user-customizable (ADR 0030 amendment, personal-cfo-kogu). For a
        // system row we UPDATE color/icon only and preserve the canonical name — a submitted
        // name is ignored, so the immutability boundary can't be bypassed from the client.
        let is_system: Option<bool> = tx
            .query_row(
                "SELECT is_system FROM categories WHERE id = ?1",
                [id.as_uuid()],
                |r| r.get::<_, i64>(0).map(|v| v != 0),
            )
            .optional()?;
        let now = Utc::now().to_rfc3339();
        match is_system {
            None => {
                return Err(DbError::InvalidCommand(
                    "category does not exist".to_owned(),
                ));
            }
            Some(true) => {
                tx.execute(
                    "UPDATE categories SET color = ?2, icon = ?3, updated_at = ?4 WHERE id = ?1",
                    params![id.as_uuid(), color, icon, now],
                )?;
            }
            Some(false) => {
                tx.execute(
                    "UPDATE categories SET name = ?2, color = ?3, icon = ?4, updated_at = ?5 WHERE id = ?1",
                    params![id.as_uuid(), trimmed, color, icon, now],
                )?;
            }
        }
        id.as_uuid()
    };
    Ok(entity_id)
}

/// Applies [`WriteCommand::MoveCategory`].
pub(crate) fn apply_move_category(
    tx: &Connection,
    id: &CategoryId,
    new_parent_id: &Option<CategoryId>,
) -> Result<Uuid, DbError> {
    let entity_id = {
        // A system category's identity is fixed — it cannot be re-parented (ADR 0030;
        // only its appearance is editable, per the kogu amendment).
        require_user_category(tx, *id)?;
        if let Some(parent) = new_parent_id {
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM categories WHERE id = ?1)",
                [parent.as_uuid()],
                |r| r.get(0),
            )?;
            if !exists {
                return Err(DbError::InvalidCommand(
                    "parent category does not exist".to_owned(),
                ));
            }
            if would_create_category_cycle(tx, parent.as_uuid(), id.as_uuid())? {
                return Err(DbError::InvalidCommand(
                    "re-parenting would create a category cycle".to_owned(),
                ));
            }
        }
        let now = Utc::now().to_rfc3339();
        tx.execute(
            "UPDATE categories SET parent_id = ?2, updated_at = ?3 WHERE id = ?1",
            params![id.as_uuid(), new_parent_id.map(|p| p.as_uuid()), now],
        )?;
        id.as_uuid()
    };
    Ok(entity_id)
}

/// Applies [`WriteCommand::ArchiveCategory`].
pub(crate) fn apply_archive_category(tx: &Connection, id: &CategoryId) -> Result<Uuid, DbError> {
    let entity_id = {
        let now = Utc::now().to_rfc3339();
        let n = tx.execute(
            "UPDATE categories SET archived_at = ?2, updated_at = ?2 WHERE id = ?1",
            params![id.as_uuid(), now],
        )?;
        if n == 0 {
            return Err(DbError::InvalidCommand(
                "category does not exist".to_owned(),
            ));
        }
        id.as_uuid()
    };
    Ok(entity_id)
}

/// Applies [`WriteCommand::ReinstateCategory`].
pub(crate) fn apply_reinstate_category(tx: &Connection, id: &CategoryId) -> Result<Uuid, DbError> {
    let entity_id = {
        let now = Utc::now().to_rfc3339();
        let n = tx.execute(
            "UPDATE categories SET archived_at = NULL, updated_at = ?2 WHERE id = ?1",
            params![id.as_uuid(), now],
        )?;
        if n == 0 {
            return Err(DbError::InvalidCommand(
                "category does not exist".to_owned(),
            ));
        }
        id.as_uuid()
    };
    Ok(entity_id)
}

/// Applies [`WriteCommand::RecategorizeTransaction`].
pub(crate) fn apply_recategorize_transaction(
    tx: &Connection,
    transaction_id: &TransactionId,
    category_id: &Option<CategoryId>,
) -> Result<Uuid, DbError> {
    let entity_id = {
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM ledger_transactions WHERE id = ?1)",
            [transaction_id.as_uuid()],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(DbError::InvalidCommand(
                "transaction does not exist".to_owned(),
            ));
        }
        match category_id {
            Some(category) => {
                let known: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM categories WHERE id = ?1)",
                    [category.as_uuid()],
                    |r| r.get(0),
                )?;
                if !known {
                    return Err(DbError::InvalidCommand(
                        "category does not exist".to_owned(),
                    ));
                }
                // A manual assignment: source = user, confidence = 100% (ADR 0030).
                // Latest wins (INSERT OR REPLACE on the 1:1 transaction_id).
                tx.execute(
                    "INSERT OR REPLACE INTO transaction_categorizations
                            (transaction_id, category_id, source, confidence_bps, assigned_at)
                         VALUES (?1, ?2, 'user', 10000, ?3)",
                    params![
                        transaction_id.as_uuid(),
                        category.as_uuid(),
                        Utc::now().to_rfc3339()
                    ],
                )?;
            }
            None => {
                tx.execute(
                    "DELETE FROM transaction_categorizations WHERE transaction_id = ?1",
                    [transaction_id.as_uuid()],
                )?;
            }
        }
        transaction_id.as_uuid()
    };
    Ok(entity_id)
}
