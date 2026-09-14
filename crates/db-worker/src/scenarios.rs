//! Scenario definitions (ADR 0026 §5, personal-cfo-0mg).
//!
//! A scenario is a **named overlay** on the base forecast. Its events are ordinary
//! scenario-scoped assumption events (`forecast_assumption_events.scenario_id`, see
//! [`crate::assumptions`]) — there is no separate event store, so a scenario
//! inherits the `5u2` backbone's explanation, invalidation, and undo for free. This
//! module is just the named definition + lifecycle (`scenarios`); running a
//! scenario is `active_assumption_events(Some(id))` layered over the base.
//!
//! Access methods live on [`crate::DbWorker`] in the `settings` direct-write shape.

use rusqlite::Row;
use uuid::Uuid;

use crate::DbError;

/// A scenario's lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScenarioStatus {
    Draft,
    Active,
    Archived,
}

impl ScenarioStatus {
    /// The stable SQL token persisted for this variant.
    #[must_use]
    pub fn as_token(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Active => "active",
            Self::Archived => "archived",
        }
    }

    /// Parse a persisted SQL token, or `None` when unrecognized.
    #[must_use]
    pub fn from_token(token: &str) -> Option<Self> {
        match token {
            "draft" => Some(Self::Draft),
            "active" => Some(Self::Active),
            "archived" => Some(Self::Archived),
            _ => None,
        }
    }
}

/// The fields needed to create a scenario. `status` starts `Draft`; timestamps are
/// set on write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewScenario {
    /// Caller-supplied id (time-ordered `Uuid::now_v7`).
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    /// The forecast run this scenario forks from (for compare-vs-base).
    pub base_run_id: Option<Uuid>,
}

/// A stored scenario definition (`scenarios`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioView {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub status: ScenarioStatus,
    pub base_run_id: Option<Uuid>,
    pub created_at: String,
    pub updated_at: String,
    /// User-set expiry, `YYYY-MM-DD` (ADR 0051 §3). Past this date the scenario is
    /// treated as archived for selection; its row and events are untouched.
    pub expires_on: Option<String>,
    /// How many active overlay events this scenario carries — what the manager shows
    /// so "archived" is visibly non-destructive.
    pub event_count: u32,
    /// When this scenario's events were promoted into base (ADR 0055), or `None`.
    /// A timestamp rather than a status token, so applied-ness stays orthogonal to the
    /// draft/active/archived lifecycle — an applied scenario can still be archived.
    pub applied_at: Option<String>,
}

impl ScenarioView {
    /// Map a row selected as: id, name, description, status, base_run_id,
    /// created_at, updated_at, expires_on, event_count, applied_at.
    pub(crate) fn from_row(row: &Row<'_>) -> Result<Self, DbError> {
        let status: String = row.get(3)?;
        Ok(Self {
            id: row.get(0)?,
            name: row.get(1)?,
            description: row.get(2)?,
            status: ScenarioStatus::from_token(&status).ok_or_else(|| {
                DbError::InvalidCommand(format!("unknown scenario status: {status}"))
            })?,
            base_run_id: row.get(4)?,
            created_at: row.get(5)?,
            updated_at: row.get(6)?,
            expires_on: row.get(7)?,
            event_count: row.get::<_, i64>(8)?.try_into().unwrap_or(u32::MAX),
            applied_at: row.get(9)?,
        })
    }
}

/// Resolve the scenario a forecast run should actually apply (ADR 0051 §1, §3).
///
/// A scenario only reaches a run if it is selectable: it must still exist, must not be
/// archived, and must not have passed its expiry. Without this gate "archive" would be a
/// UI convention only — every overlay loader filters on `scenario_id` and none consults
/// the scenario's status, so an archived scenario whose events are still active would
/// fully apply the moment its id were passed in (a stale selection, a deep link, a
/// pinned query key). That is exactly why the old `delete_scenario` cleared events; with
/// events now preserved, the enforcement has to move here.
///
/// `None` in gives `None` out — the base forecast is never gated.
///
/// `today` is passed in rather than read from the clock so callers stay
/// deterministic/testable, matching the household-local `as_of` the forecast already
/// threads through.
///
/// # Errors
/// [`DbError`] on a read failure.
/// Filter a selection to the scenarios that actually apply, PRESERVING ORDER
/// (personal-cfo-4d8.27.6.4, ADR 0059 §1).
///
/// Order is the precedence, so dropping an expired or archived scenario must not reshuffle
/// the ones around it — the survivors keep their relative stacking.
///
/// # Errors
/// [`DbError`] on a read failure.
pub(crate) fn effective_scenarios(
    conn: &rusqlite::Connection,
    scenarios: &[Uuid],
    today: chrono::NaiveDate,
) -> Result<Vec<Uuid>, DbError> {
    let mut out = Vec::with_capacity(scenarios.len());
    for id in scenarios {
        if let Some(kept) = effective_scenario(conn, Some(*id), today)? {
            out.push(kept);
        }
    }
    Ok(out)
}

pub(crate) fn effective_scenario(
    conn: &rusqlite::Connection,
    scenario: Option<Uuid>,
    today: chrono::NaiveDate,
) -> Result<Option<Uuid>, DbError> {
    let Some(id) = scenario else {
        return Ok(None);
    };
    let mut stmt = conn.prepare("SELECT status, expires_on FROM scenarios WHERE id = ?1")?;
    let mut rows = stmt.query(rusqlite::params![id])?;
    let Some(row) = rows.next()? else {
        // Deleted out from under a stale selection.
        return Ok(None);
    };
    // Fails CLOSED on an unreadable status (the scenario simply is not applied): the
    // column is CHECK-constrained so this is unreachable, and refusing to apply an
    // unknown state is the safe direction. A malformed EXPIRY below deliberately fails
    // OPEN instead — there, refusing would silently disable a live plan.
    let status: String = row.get(0)?;
    if !matches!(
        ScenarioStatus::from_token(&status),
        Some(ScenarioStatus::Draft | ScenarioStatus::Active)
    ) {
        return Ok(None);
    }
    let expires_on: Option<String> = row.get(1)?;
    if let Some(expires_on) = expires_on.as_deref() {
        // A malformed date must not silently disable a scenario — ignore it and let the
        // scenario apply, which is the non-surprising failure direction.
        if let Ok(expiry) = chrono::NaiveDate::parse_from_str(expires_on, "%Y-%m-%d") {
            if today > expiry {
                return Ok(None);
            }
        }
    }
    Ok(Some(id))
}
