//! `apply_command` arm for the household configuration command
//! (personal-cfo-g3m.8; `docs/architecture/vault-format.md` B.2 rule 6).
//!
//! `household_timezone` sets the calendar boundary of the forecast horizon (ADR 0021
//! section 1), so it is a class-1 logical field that changes only through a logged
//! command. This is the only code that writes it after the vault's genesis seed
//! (`ensure_vault_metadata` seeds the household with the `UTC` default before any command
//! exists). The household, its members and their profiles (personal-cfo-339) follow the same
//! rule: class 1, written only here.
//!
//! The audit event's timestamp is `meta.issued_at`, not the wall clock: `apply` never
//! reads the clock (ADR 0013 Addendum A), so replaying the command elsewhere reads the
//! same instant.

use forecast_engine::Tz;
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::households::{ROLE_PARTNER, ROLE_PRIMARY};
use crate::{CommandMeta, DbError};

/// The `audit_events.event_type` recorded for every household timezone change.
pub(crate) const HOUSEHOLD_TIMEZONE_CHANGED: &str = "household_timezone_changed";

/// The command's `issued_at` as an RFC 3339 UTC timestamp, for a local audit column.
pub(crate) fn issued_at_rfc3339(meta: &CommandMeta) -> Result<String, DbError> {
    Ok(super::issued_at_text(meta))
}

/// The household's id (there is exactly one per vault).
fn household_id(tx: &Connection) -> Result<Uuid, DbError> {
    tx.query_row("SELECT id FROM households", [], |r| r.get(0))
        .optional()?
        .ok_or_else(|| DbError::InvalidCommand("the vault has no household row".to_owned()))
}

/// Applies [`WriteCommand::SetHouseholdTimezone`](crate::WriteCommand): validate the IANA
/// name and store it on the household row, and append an `audit_events` row in the same
/// transaction as the `operation_log` entry the caller writes. Returns the household id as
/// the affected entity.
///
/// A name that is not a real IANA zone is rejected here, the one gate that keeps a typo
/// or a removed zone name out of the vault, because `read_household_tz` and every caller
/// downstream of it (the whole Future Cash forecast surface, ADR 0021) fail loudly on a
/// bad value. Setting the value it already has is an accepted, logged no-op that writes no
/// class-1 row (`updated_at` moves only when the zone does).
pub(crate) fn apply_set_household_timezone(
    tx: &Connection,
    meta: &CommandMeta,
    ids: &core_ids::DerivedIds,
    timezone: &str,
) -> Result<Uuid, DbError> {
    timezone.parse::<Tz>().map_err(|e| {
        DbError::InvalidCommand(format!("invalid household timezone {timezone:?}: {e}"))
    })?;
    let household = household_id(tx)?;
    tx.execute(
        "UPDATE households SET household_timezone = ?1, updated_at = ?2
          WHERE id = ?3 AND household_timezone <> ?1",
        params![timezone, issued_at_rfc3339(meta)?, household],
    )?;
    // The audit row is device-local (class 3) and written once per command: its id is KEYED by
    // the command and the event type, and the insert ignores a repeat, so a rebase replay of the
    // same command leaves exactly one row (ADR 0073 Addendum D, rule (i)).
    tx.execute(
        "INSERT OR IGNORE INTO audit_events (id, event_type, command_id, correlation_id, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            ids.keyed("audit.household-timezone", &[])?,
            HOUSEHOLD_TIMEZONE_CHANGED,
            meta.command_id,
            meta.correlation_id,
            issued_at_rfc3339(meta)?,
        ],
    )?;
    Ok(household)
}

/// A display text: 1 to 100 characters after trimming, no control characters. Returned trimmed.
fn clean_name(what: &str, value: &str) -> Result<String, DbError> {
    let trimmed = value.trim();
    let length = char_len(trimmed);
    if length == 0 || length > 100 || trimmed.chars().any(char::is_control) {
        return Err(DbError::InvalidCommand(format!(
            "{what} must be 1 to 100 characters without control characters"
        )));
    }
    Ok(trimmed.to_owned())
}

/// The number of characters (Unicode scalar values) in `text`.
fn char_len(text: &str) -> usize {
    text.chars().map(|_| 1_usize).sum()
}

/// What a member row currently is.
struct Member {
    role: String,
    deleted: bool,
}

fn member(tx: &Connection, id: &Uuid) -> Result<Member, DbError> {
    tx.query_row(
        "SELECT role, deleted_at IS NOT NULL FROM household_members WHERE id = ?1",
        params![id],
        |r| {
            Ok(Member {
                role: r.get(0)?,
                deleted: r.get(1)?,
            })
        },
    )
    .optional()?
    .ok_or_else(|| DbError::InvalidCommand("unknown household member".to_owned()))
}

fn live_member(tx: &Connection, id: &Uuid) -> Result<Member, DbError> {
    let found = member(tx, id)?;
    if found.deleted {
        return Err(DbError::InvalidCommand(
            "that household member was removed".to_owned(),
        ));
    }
    Ok(found)
}

/// Applies [`WriteCommand::SetHouseholdName`](crate::WriteCommand).
pub(crate) fn apply_set_household_name(
    tx: &Connection,
    meta: &CommandMeta,
    name: &str,
) -> Result<Uuid, DbError> {
    let name = clean_name("the household name", name)?;
    let household = household_id(tx)?;
    tx.execute(
        "UPDATE households SET name = ?1, updated_at = ?2 WHERE id = ?3 AND name <> ?1",
        params![name, issued_at_rfc3339(meta)?, household],
    )?;
    Ok(household)
}

/// Applies [`WriteCommand::AddHouseholdMember`](crate::WriteCommand): a partner with the
/// caller-minted id.
pub(crate) fn apply_add_member(
    tx: &Connection,
    meta: &CommandMeta,
    member_id: &Uuid,
    display_name: &str,
) -> Result<Uuid, DbError> {
    let name = clean_name("a member name", display_name)?;
    let household = household_id(tx)?;
    let exists: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM household_members WHERE id = ?1)",
        params![member_id],
        |r| r.get(0),
    )?;
    if exists {
        return Err(DbError::InvalidCommand(
            "that household member id is already in use".to_owned(),
        ));
    }
    tx.execute(
        "INSERT INTO household_members
            (id, household_id, display_name, role, platform_member_id, created_at, deleted_at)
         VALUES (?1, ?2, ?3, ?4, NULL, ?5, NULL)",
        params![
            member_id,
            household,
            name,
            ROLE_PARTNER,
            issued_at_rfc3339(meta)?
        ],
    )?;
    Ok(*member_id)
}

/// Applies [`WriteCommand::RenameHouseholdMember`](crate::WriteCommand).
pub(crate) fn apply_rename_member(
    tx: &Connection,
    member_id: &Uuid,
    display_name: &str,
) -> Result<Uuid, DbError> {
    let name = clean_name("a member name", display_name)?;
    live_member(tx, member_id)?;
    tx.execute(
        "UPDATE household_members SET display_name = ?1 WHERE id = ?2 AND display_name <> ?1",
        params![name, member_id],
    )?;
    Ok(*member_id)
}

/// Applies [`WriteCommand::TransferPrimary`](crate::WriteCommand): validated against STATE (the
/// `from` member must be the live primary, the `to` member live and not primary), then the old
/// primary is demoted BEFORE the new one is promoted, because SQLite checks the one-live-primary
/// unique index per statement, not at commit.
pub(crate) fn apply_transfer_primary(
    tx: &Connection,
    from: &Uuid,
    to: &Uuid,
) -> Result<Uuid, DbError> {
    if from == to {
        return Err(DbError::InvalidCommand(
            "the primary seat cannot move to the member who holds it".to_owned(),
        ));
    }
    let current = live_member(tx, from)?;
    if current.role != ROLE_PRIMARY {
        return Err(DbError::InvalidCommand(
            "that member is not the primary".to_owned(),
        ));
    }
    let target = live_member(tx, to)?;
    if target.role == ROLE_PRIMARY {
        return Err(DbError::InvalidCommand(
            "the target member already holds the primary seat".to_owned(),
        ));
    }
    tx.execute(
        "UPDATE household_members SET role = 'partner' WHERE id = ?1",
        params![from],
    )?;
    tx.execute(
        "UPDATE household_members SET role = 'primary' WHERE id = ?1",
        params![to],
    )?;
    Ok(*to)
}

/// Applies [`WriteCommand::RemoveHouseholdMember`](crate::WriteCommand): a tombstone, never a
/// hard delete. The live primary cannot be removed; an already removed member is an accepted
/// no-op.
pub(crate) fn apply_remove_member(
    tx: &Connection,
    meta: &CommandMeta,
    member_id: &Uuid,
) -> Result<Uuid, DbError> {
    let found = member(tx, member_id)?;
    if found.deleted {
        return Ok(*member_id);
    }
    if found.role == ROLE_PRIMARY {
        return Err(DbError::InvalidCommand(
            "the primary cannot be removed; transfer the seat first".to_owned(),
        ));
    }
    tx.execute(
        "UPDATE household_members SET deleted_at = ?1 WHERE id = ?2",
        params![issued_at_rfc3339(meta)?, member_id],
    )?;
    Ok(*member_id)
}

/// Applies [`WriteCommand::RestoreHouseholdMember`](crate::WriteCommand). A live member is an
/// accepted no-op. A removed member is never the primary (the primary cannot be removed and the
/// seat moves only to a live member), so no second live primary can appear.
pub(crate) fn apply_restore_member(tx: &Connection, member_id: &Uuid) -> Result<Uuid, DbError> {
    member(tx, member_id)?;
    tx.execute(
        "UPDATE household_members SET deleted_at = NULL WHERE id = ?1 AND deleted_at IS NOT NULL",
        params![member_id],
    )?;
    Ok(*member_id)
}

/// Whether `value` is a `#RRGGBB` color.
fn is_hex_color(value: &str) -> bool {
    value.len() == 7 && value.starts_with('#') && value[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// Applies [`WriteCommand::SetMemberProfile`](crate::WriteCommand): one to four initials and a
/// `#RRGGBB` color, either optional. A profile row is never deleted (a replicated row is
/// tombstoned or kept): clearing both fields keeps the row with empty values; setting what is
/// already set is a no-op.
pub(crate) fn apply_set_member_profile(
    tx: &Connection,
    meta: &CommandMeta,
    member_id: &Uuid,
    initials: Option<&str>,
    color: Option<&str>,
) -> Result<Uuid, DbError> {
    if let Some(initials) = initials {
        let length = char_len(initials);
        if length == 0
            || length > 4
            || initials
                .chars()
                .any(|c| c.is_control() || c.is_whitespace())
        {
            return Err(DbError::InvalidCommand(
                "initials are 1 to 4 characters without spaces".to_owned(),
            ));
        }
    }
    if color.is_some_and(|c| !is_hex_color(c)) {
        return Err(DbError::InvalidCommand(
            "a profile color is #RRGGBB".to_owned(),
        ));
    }
    live_member(tx, member_id)?;
    let current: Option<(Option<String>, Option<String>)> = tx
        .query_row(
            "SELECT initials, color FROM profiles WHERE member_id = ?1",
            params![member_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    match current {
        None if initials.is_none() && color.is_none() => {}
        None => {
            tx.execute(
                "INSERT INTO profiles (member_id, initials, color, updated_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![member_id, initials, color, issued_at_rfc3339(meta)?],
            )?;
        }
        Some((old_initials, old_color))
            if old_initials.as_deref() == initials && old_color.as_deref() == color => {}
        Some(_) => {
            tx.execute(
                "UPDATE profiles SET initials = ?2, color = ?3, updated_at = ?4
                  WHERE member_id = ?1",
                params![member_id, initials, color, issued_at_rfc3339(meta)?],
            )?;
        }
    }
    Ok(*member_id)
}
