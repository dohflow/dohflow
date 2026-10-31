//! `apply_command` arms for Money Inbox soft actions (moved verbatim from
//! `lib.rs`).

use chrono::NaiveDate;
use core_ids::DerivedIds;
use rusqlite::Connection;
use uuid::Uuid;

use crate::{money_inbox, CommandMeta, DbError};

/// Applies [`WriteCommand::SnoozeInboxItem`].
pub(crate) fn apply_snooze_inbox_item(
    tx: &Connection,
    meta: &CommandMeta,
    ids: &mut DerivedIds,
    item_id: &Uuid,
    until: &NaiveDate,
) -> Result<Uuid, DbError> {
    let entity_id = {
        money_inbox::record_snooze(
            tx,
            ids.next("change-journal")?,
            *item_id,
            &until.to_string(),
            &crate::apply::issued_at_text(meta),
        )?;
        // Re-apply the soft-action state onto the read model.
        money_inbox::rebuild_in(tx, &crate::apply::issued_at_text(meta))?;
        *item_id
    };
    Ok(entity_id)
}

/// Applies [`WriteCommand::DismissInboxItem`].
pub(crate) fn apply_dismiss_inbox_item(
    tx: &Connection,
    meta: &CommandMeta,
    ids: &mut DerivedIds,
    item_id: &Uuid,
    reason: &str,
) -> Result<Uuid, DbError> {
    let entity_id = {
        money_inbox::record_dismiss(
            tx,
            ids.next("change-journal")?,
            *item_id,
            reason,
            &crate::apply::issued_at_text(meta),
        )?;
        money_inbox::rebuild_in(tx, &crate::apply::issued_at_text(meta))?;
        *item_id
    };
    Ok(entity_id)
}
