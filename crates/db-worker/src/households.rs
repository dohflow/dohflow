//! The household record: one `households` row per vault, its members and their profiles
//! (personal-cfo-339; ADR 0075 decision 2: the identity is the household, members are seats).
//!
//! All three tables are class 1 and are written only through `WriteCommand`s. The household and
//! the first member are SEEDED, not created by a command, with ids derived from the vault id
//! ([`core_ids::DerivedIdKey::singleton_id`]) and every column a pure function of the vault
//! (its creation time, fixed default texts), so two devices that migrate or create the same vault
//! independently hold byte-identical rows, and a replica inherits them through bootstrap instead
//! of seeding its own.
//!
//! **The local ids are for local use.** They derive from the sync-visible `vault_id`, so the
//! platform service (which must never learn them) could otherwise link records by recomputing
//! them. The platform link is `platform_household_id` / `platform_member_id` and nothing else.
//!
//! **`household_members.role` is display and ownership metadata, not access control.** Every
//! device in the membership chain holds the epoch key and can issue any command, so Sync does
//! not enforce `viewer`; nothing may rely on a role for security until an ADR says how.

use core_ids::DerivedIdKey;

use rusqlite::{params, Connection};
use uuid::Uuid;

/// The label of the household's singleton id.
pub const HOUSEHOLD_ID_LABEL: &str = "household";
/// The label of the first (primary) member's singleton id.
pub const PRIMARY_MEMBER_ID_LABEL: &str = "primary-member";
/// The household's name until the user renames it. A fixed constant, never localized at
/// migration time, so every device seeds the same bytes.
pub const DEFAULT_HOUSEHOLD_NAME: &str = "My household";
/// The first member's display name until the user renames it.
pub const DEFAULT_PRIMARY_MEMBER_NAME: &str = "Me";

/// The household's id in a vault.
#[must_use]
pub fn household_id(vault_id: &Uuid) -> Uuid {
    DerivedIdKey::for_vault(vault_id)
        .singleton_id(HOUSEHOLD_ID_LABEL)
        .expect("the label is a valid tag")
}

/// The first member's id in a vault.
#[must_use]
pub fn primary_member_id(vault_id: &Uuid) -> Uuid {
    DerivedIdKey::for_vault(vault_id)
        .singleton_id(PRIMARY_MEMBER_ID_LABEL)
        .expect("the label is a valid tag")
}

/// Insert the household and its primary member for a vault that has neither. Every column is a
/// pure function of the arguments: `created_at` is the vault's own creation time and
/// `household_timezone` its current zone, never the clock.
///
/// # Errors
/// A SQLite error, including a household that already exists.
pub(crate) fn seed(
    conn: &Connection,
    vault_id: &Uuid,
    household_timezone: &str,
    created_at: &str,
) -> rusqlite::Result<()> {
    let household = household_id(vault_id);
    conn.execute(
        "INSERT INTO households
            (id, name, household_timezone, platform_household_id, created_at, updated_at)
         VALUES (?1, ?2, ?3, NULL, ?4, ?4)",
        params![
            household,
            DEFAULT_HOUSEHOLD_NAME,
            household_timezone,
            created_at
        ],
    )?;
    conn.execute(
        "INSERT INTO household_members
            (id, household_id, display_name, role, platform_member_id, created_at, deleted_at)
         VALUES (?1, ?2, ?3, 'primary', NULL, ?4, NULL)",
        params![
            primary_member_id(vault_id),
            household,
            DEFAULT_PRIMARY_MEMBER_NAME,
            created_at
        ],
    )?;
    Ok(())
}

/// The migration to schema version 61: move an existing vault's timezone onto its new household
/// row (seeded from the vault's own id and creation time) and drop the old column. A fresh vault
/// has no `vault_metadata` row yet; its household is seeded when the row is created.
pub(crate) fn migrate_up(conn: &Connection) -> rusqlite::Result<()> {
    let existing: Option<(Uuid, String, String)> = {
        let mut stmt = conn.prepare(
            "SELECT vault_id, household_timezone, created_at FROM vault_metadata WHERE singleton = 1",
        )?;
        let mut rows = stmt.query([])?;
        match rows.next()? {
            Some(r) => Some((r.get(0)?, r.get(1)?, r.get(2)?)),
            None => None,
        }
    };
    if let Some((vault_id, timezone, created_at)) = existing {
        seed(conn, &vault_id, &timezone, &created_at)?;
    }
    conn.execute_batch("ALTER TABLE vault_metadata DROP COLUMN household_timezone;")
}

/// The role of a household member. Display and ownership metadata only, never access control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum HouseholdRole {
    /// The one live seat that owns the household; moves only by `TransferPrimary`.
    Primary,
    /// A full participant.
    Partner,
    /// A participant shown as read-only. Not enforced: Sync gives every device every command.
    Viewer,
}

impl HouseholdRole {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Partner => "partner",
            Self::Viewer => "viewer",
        }
    }
}
