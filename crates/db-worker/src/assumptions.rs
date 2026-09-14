//! Forecast assumption-event model (ADR 0026 §4, personal-cfo-5u2).
//!
//! The assumption-event backbone the reproducible forecast pipeline applies:
//!
//! - **`forecast_assumption_events`** — the versioned input/override/scenario
//!   events a run is computed against. They are **run-independent**: an event
//!   persists across recomputes, gated by [`AssumptionStatus`], so a user
//!   override survives a re-run until it is explicitly cleared or superseded
//!   (which one run records it used via `forecast_runs.assumptions_hash`).
//! - **`forecast_dependency_edges`** — per-run attribution edges (event → row)
//!   that power per-row explanation (`forecast_rows ⋈ edges ⋈ events`) and
//!   incremental invalidation.
//! - **`forecast_dirty_ranges`** — the date-range recompute queue that drives
//!   the interactive tier.
//!
//! These are the typed vocabulary over the v10 schema; the access methods live
//! on [`crate::DbWorker`] in the `settings` direct-write shape — assumption
//! events are forecast inputs, not ledger mutations, so they bypass the
//! `WriteCommand` bus.

use chrono::NaiveDate;
use rusqlite::Row;
use uuid::Uuid;

use crate::DbError;

/// Define an enum whose variants map 1:1 to stable SQL tokens (matching the
/// `CHECK` constraints in migration v10), with `as_token` / `from_token`.
macro_rules! token_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident { $($variant:ident => $token:literal),+ $(,)? }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        $vis enum $name { $($variant),+ }

        impl $name {
            /// The stable SQL token persisted for this variant.
            #[must_use]
            $vis fn as_token(self) -> &'static str {
                match self { $(Self::$variant => $token),+ }
            }

            /// Parse a persisted SQL token, or `None` when unrecognized.
            #[must_use]
            $vis fn from_token(token: &str) -> Option<Self> {
                match token { $($token => Some(Self::$variant),)+ _ => None }
            }
        }
    };
}

token_enum! {
    /// What an assumption event adjusts.
    pub enum AssumptionKind {
        IncomeAmount => "income_amount",
        IncomeDate => "income_date",
        BillAmount => "bill_amount",
        BillDate => "bill_date",
        CardPaymentBehavior => "card_payment_behavior",
        MinimumCashFloor => "minimum_cash_floor",
        VariableSpendOverride => "variable_spend_override",
        OneTimeEvent => "one_time_event",
        InflationRate => "inflation_rate",
        ScenarioToggle => "scenario_toggle",
        Exclusion => "exclusion",
        RecurringDebtPayment => "recurring_debt_payment",
    }
}

token_enum! {
    /// Where an assumption event originated.
    pub enum AssumptionSource {
        UserOverride => "user_override",
        ModelDefault => "model_default",
        AgentProposal => "agent_proposal",
        Scheduled => "scheduled",
    }
}

token_enum! {
    /// Lifecycle of an assumption event. Only `Active` events feed a run; the
    /// rest are retained for history (overrides survive a re-run).
    pub enum AssumptionStatus {
        Active => "active",
        Cleared => "cleared",
        Superseded => "superseded",
    }
}

token_enum! {
    /// How a dependency edge relates an event to a forecast row.
    pub enum EdgeType {
        Affects => "affects",
        DerivesFrom => "derives_from",
    }
}

token_enum! {
    /// Why a date range needs recompute.
    pub enum DirtyReason {
        AssumptionChanged => "assumption_changed",
        TransactionAdded => "transaction_added",
        ScheduleChanged => "schedule_changed",
        Manual => "manual",
    }
}

/// The fields needed to record a new assumption event. On write the status is
/// set `Active` and both timestamps to "now".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAssumptionEvent {
    /// Caller-supplied id (time-ordered `Uuid::now_v7`).
    pub id: Uuid,
    pub kind: AssumptionKind,
    /// The adjusted entity type (`income_source` / `recurring_event` /
    /// `account` / `global`), or `None` for a free-standing one-off.
    pub target_entity_type: Option<String>,
    /// The adjusted entity, or `None` for a global / one-off event.
    pub target_entity_id: Option<Uuid>,
    /// Event parameters (the new amount / date / rate / …) as a JSON string.
    pub params_json: String,
    pub source: AssumptionSource,
    /// `Some` = scenario-specific; `None` = a base assumption.
    pub scenario_id: Option<Uuid>,
    /// The run during which the event was authored (provenance only — the
    /// event is not tied to that run's lifecycle).
    pub origin_run_id: Option<Uuid>,
}

/// A stored assumption event (the read model over `forecast_assumption_events`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssumptionEventView {
    pub id: Uuid,
    pub kind: AssumptionKind,
    pub target_entity_type: Option<String>,
    pub target_entity_id: Option<Uuid>,
    pub params_json: String,
    pub source: AssumptionSource,
    pub scenario_id: Option<Uuid>,
    pub status: AssumptionStatus,
    /// The event that replaced this one, when `status` is `Superseded`.
    pub superseded_by: Option<Uuid>,
    pub origin_run_id: Option<Uuid>,
    pub created_at: String,
    pub updated_at: String,
    /// The scenario this event was promoted from, when it is a base event created by
    /// applying a scenario (ADR 0055). `None` for an event the user set directly.
    ///
    /// This is the provenance that lets a bill say WHY its forecast figure differs from
    /// its stored amount — without it, an applied override is indistinguishable from one
    /// the user typed, and the two need different explanations.
    pub promoted_from_scenario_id: Option<Uuid>,
}

impl AssumptionEventView {
    /// Map a `forecast_assumption_events` row (selected in column order: id,
    /// kind, target_entity_type, target_entity_id, params_json, source,
    /// scenario_id, status, superseded_by, origin_run_id, created_at,
    /// updated_at, promoted_from_scenario_id).
    pub(crate) fn from_row(row: &Row<'_>) -> Result<Self, DbError> {
        let kind: String = row.get(1)?;
        let source: String = row.get(5)?;
        let status: String = row.get(7)?;
        Ok(Self {
            id: row.get(0)?,
            kind: AssumptionKind::from_token(&kind).ok_or_else(|| {
                DbError::InvalidCommand(format!("unknown assumption kind: {kind}"))
            })?,
            target_entity_type: row.get(2)?,
            target_entity_id: row.get(3)?,
            params_json: row.get(4)?,
            source: AssumptionSource::from_token(&source).ok_or_else(|| {
                DbError::InvalidCommand(format!("unknown assumption source: {source}"))
            })?,
            scenario_id: row.get(6)?,
            status: AssumptionStatus::from_token(&status).ok_or_else(|| {
                DbError::InvalidCommand(format!("unknown assumption status: {status}"))
            })?,
            superseded_by: row.get(8)?,
            origin_run_id: row.get(9)?,
            created_at: row.get(10)?,
            updated_at: row.get(11)?,
            promoted_from_scenario_id: row.get(12)?,
        })
    }
}

/// A per-run attribution edge: an assumption event shaped a forecast row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyEdge {
    /// Caller-supplied id (time-ordered `Uuid::now_v7`).
    pub id: Uuid,
    pub forecast_run_id: Uuid,
    pub from_event_id: Uuid,
    pub to_row_id: Uuid,
    pub edge_type: EdgeType,
}

impl DependencyEdge {
    /// Map a `forecast_dependency_edges` row (id, forecast_run_id,
    /// from_event_id, to_row_id, edge_type).
    pub(crate) fn from_row(row: &Row<'_>) -> Result<Self, DbError> {
        let edge_type: String = row.get(4)?;
        Ok(Self {
            id: row.get(0)?,
            forecast_run_id: row.get(1)?,
            from_event_id: row.get(2)?,
            to_row_id: row.get(3)?,
            edge_type: EdgeType::from_token(&edge_type).ok_or_else(|| {
                DbError::InvalidCommand(format!("unknown edge type: {edge_type}"))
            })?,
        })
    }
}

/// A date range awaiting recompute (an open `forecast_dirty_ranges` row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirtyRange {
    /// Caller-supplied id (time-ordered `Uuid::now_v7`).
    pub id: Uuid,
    pub from_date: NaiveDate,
    pub to_date: NaiveDate,
    pub reason: DirtyReason,
    /// The assumption event that dirtied the range, when applicable.
    pub triggering_event_id: Option<Uuid>,
}

impl DirtyRange {
    /// Map a `forecast_dirty_ranges` row (id, from_date, to_date, reason,
    /// triggering_event_id).
    pub(crate) fn from_row(row: &Row<'_>) -> Result<Self, DbError> {
        let from_date: String = row.get(1)?;
        let to_date: String = row.get(2)?;
        let reason: String = row.get(3)?;
        Ok(Self {
            id: row.get(0)?,
            from_date: NaiveDate::parse_from_str(&from_date, "%Y-%m-%d").map_err(|e| {
                DbError::InvalidCommand(format!("bad dirty-range from_date {from_date:?}: {e}"))
            })?,
            to_date: NaiveDate::parse_from_str(&to_date, "%Y-%m-%d").map_err(|e| {
                DbError::InvalidCommand(format!("bad dirty-range to_date {to_date:?}: {e}"))
            })?,
            reason: DirtyReason::from_token(&reason).ok_or_else(|| {
                DbError::InvalidCommand(format!("unknown dirty reason: {reason}"))
            })?,
            triggering_event_id: row.get(4)?,
        })
    }
}
