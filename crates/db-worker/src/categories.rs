//! Category taxonomy read model (plan §9.6, ADR 0030; personal-cfo-d3p / -bac).
//!
//! The hierarchical category taxonomy — seeded at vault create by
//! `ensure_default_categories` and managed by the CRUD commands on the bus. This
//! module exposes it for the category-management UI and the re-categorization
//! picker; it is read-only (the command apply arms own the writes).

use core_ledger::CategoryId;
use rusqlite::{params, Connection};
use uuid::Uuid;

use crate::DbError;

/// The id of the **unambiguous** non-archived category whose name matches `name`
/// case-insensitively — resolves an imported category string to a real category for
/// the `import_alias` prefill (ADR 0045 §3, personal-cfo-4d8.24.1.1).
///
/// Category names are **not** globally unique (the seeded taxonomy has "Maintenance"
/// under both Housing and Transportation, and users may add same-name categories
/// under different parents or types). When a name matches more than one non-archived
/// category we return `None` and skip the prefill rather than guess the wrong one — a
/// wrong low-confidence suggestion is worse than none, and the raw imported category
/// stays visible in the transaction's Imported details (4d8.24.1.4). An empty/blank
/// `name`, or no match, also yields `None`.
pub(crate) fn find_id_by_name(conn: &Connection, name: &str) -> Result<Option<Uuid>, DbError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    // LIMIT 2 is enough to tell "unique" (1 row) from "ambiguous" (≥2 rows) apart.
    let mut stmt = conn.prepare(
        "SELECT id FROM categories
          WHERE name = ?1 COLLATE NOCASE AND archived_at IS NULL
          LIMIT 2",
    )?;
    let ids: Vec<Uuid> = stmt
        .query_map(params![trimmed], |r| r.get::<_, Uuid>(0))?
        .collect::<Result<_, _>>()?;
    Ok(if ids.len() == 1 { Some(ids[0]) } else { None })
}

/// A category in the taxonomy (read model). `parent_id` is `None` for a top-level
/// group. `is_system` marks the seeded defaults — their identity (name/parent) is fixed,
/// but their appearance (color + icon) is user-editable (ADR 0030 amendment, kogu).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoryView {
    pub id: CategoryId,
    /// The parent category, or `None` for a top-level group.
    pub parent_id: Option<CategoryId>,
    pub name: String,
    /// `income` / `expense` / `transfer` / `adjustment`.
    pub category_type: String,
    pub icon: Option<String>,
    pub color: Option<String>,
    /// A seeded default — its identity (name/parent) is fixed, but its appearance
    /// (color + icon) is user-editable, and it can be archived (ADR 0030 amendment, kogu).
    pub is_system: bool,
    /// `deterministic` / `variable_regular` / `variable_lumpy` / `ignore_cashflow`
    /// / `income`.
    pub forecast_behavior: String,
    /// Whether the category is archived (hidden from pickers; ADR 0030 soft-delete).
    pub archived: bool,
}

/// Read the full category taxonomy, ordered by type then name (the frontend builds
/// the tree from `parent_id`).
///
/// # Errors
/// Returns [`DbError`] if the read fails.
pub(crate) fn read_views(conn: &Connection) -> Result<Vec<CategoryView>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, parent_id, name, type, icon, color, is_system, forecast_behavior,
                archived_at
         FROM categories
         ORDER BY type, name COLLATE NOCASE, id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(CategoryView {
            id: CategoryId::from_uuid(r.get::<_, Uuid>(0)?),
            parent_id: r.get::<_, Option<Uuid>>(1)?.map(CategoryId::from_uuid),
            name: r.get(2)?,
            category_type: r.get(3)?,
            icon: r.get(4)?,
            color: r.get(5)?,
            is_system: r.get::<_, i64>(6)? != 0,
            forecast_behavior: r.get(7)?,
            archived: r.get::<_, Option<String>>(8)?.is_some(),
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}
