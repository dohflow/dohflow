//! `apply_command` arms for Money Inbox soft actions (moved verbatim from
//! `lib.rs`).

use chrono::NaiveDate;
use rusqlite::Connection;
use uuid::Uuid;

use crate::{money_inbox, DbError};

/// Applies [`WriteCommand::SnoozeInboxItem`].
pub(crate) fn apply_snooze_inbox_item(
    tx: &Connection,
    item_id: &Uuid,
    until: &NaiveDate,
) -> Result<Uuid, DbError> {
    let entity_id = {
        money_inbox::record_snooze(tx, *item_id, &until.to_string())?;
        // Re-apply the soft-action state onto the read model.
        money_inbox::rebuild_in(tx)?;
        *item_id
    };
    Ok(entity_id)
}

/// Applies [`WriteCommand::DismissInboxItem`].
pub(crate) fn apply_dismiss_inbox_item(
    tx: &Connection,
    item_id: &Uuid,
    reason: &str,
) -> Result<Uuid, DbError> {
    let entity_id = {
        money_inbox::record_dismiss(tx, *item_id, reason)?;
        money_inbox::rebuild_in(tx)?;
        *item_id
    };
    Ok(entity_id)
}
