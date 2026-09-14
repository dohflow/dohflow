//! Reproducible forecast-run persistence (ADR 0026 §3, personal-cfo-eqfw).
//!
//! The storage layer beneath persisted, reproducible forecasts:
//!
//! - **content-addressed input snapshots** — [`gather_inputs`] serializes the
//!   forecast's vault-state inputs (recurring obligations, income sources, active
//!   base assumption events, scenario overlays) into canonical JSON and hashes
//!   them (FNV-1a via [`crate::migrations::content_hash`]; crypto is deferred to
//!   `vhv`). [`crate::DbWorker::capture_input_snapshot`] dedups by that hash, so
//!   identical inputs map to the same snapshot id. Storing the *inputs* (not just
//!   hashes, as the baseline `63t` did) is what lets a run be reproduced.
//! - the **model registry** (`model_registry`) — versions forecast models (L1
//!   today; L2-L4 later).
//! - **run diffs** (`forecast_diffs`) — run-to-run comparisons.
//!
//! db-worker builds JSON by hand (`serde_json` is kept out of the write path),
//! so [`json_str`] does the escaping. The access methods live on
//! [`crate::DbWorker`] in the `settings` direct-write shape.

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Row};
use uuid::Uuid;

use crate::forecast::ForecastView;
use crate::{migrations, DbError};

/// A captured forecast input snapshot, content-addressed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputSnapshotView {
    pub id: Uuid,
    /// FNV-1a content address over the canonical inputs (`None` only for legacy
    /// baseline rows written before v11).
    pub content_hash: Option<String>,
    pub captured_at: String,
    /// The `operation_log` position the ledger was read at.
    pub ledger_cutoff_op_seq: Option<i64>,
    pub recurring_events_snapshot_json: Option<String>,
    pub income_sources_snapshot_json: Option<String>,
    pub manual_assumption_events_json: Option<String>,
    pub scenario_overlay_ids_json: Option<String>,
}

impl InputSnapshotView {
    /// Map a row selected as: id, content_hash, created_at, ledger_cutoff_op_seq,
    /// recurring_events_snapshot_json, income_sources_snapshot_json,
    /// manual_assumption_events_json, scenario_overlay_ids_json.
    pub(crate) fn from_row(row: &Row<'_>) -> Result<Self, DbError> {
        Ok(Self {
            id: row.get(0)?,
            content_hash: row.get(1)?,
            captured_at: row.get(2)?,
            ledger_cutoff_op_seq: row.get(3)?,
            recurring_events_snapshot_json: row.get(4)?,
            income_sources_snapshot_json: row.get(5)?,
            manual_assumption_events_json: row.get(6)?,
            scenario_overlay_ids_json: row.get(7)?,
        })
    }
}

/// A registered forecast model version (`model_registry`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRegistryView {
    pub model_id: String,
    pub version: String,
    pub parameters_json: Option<String>,
    pub created_at: String,
}

impl ModelRegistryView {
    /// Map a row selected as: model_id, version, parameters_json, created_at.
    pub(crate) fn from_row(row: &Row<'_>) -> Result<Self, DbError> {
        Ok(Self {
            model_id: row.get(0)?,
            version: row.get(1)?,
            parameters_json: row.get(2)?,
            created_at: row.get(3)?,
        })
    }
}

/// The fields needed to record a run-to-run diff. `created_at` is set on write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewForecastDiff {
    /// Caller-supplied id (time-ordered `Uuid::now_v7`).
    pub id: Uuid,
    pub forecast_run_id: Uuid,
    /// The run this one is compared against (`None` for a first run).
    pub prior_run_id: Option<Uuid>,
    pub summary_json: String,
}

/// A stored run-to-run diff (`forecast_diffs`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForecastDiffView {
    pub id: Uuid,
    pub forecast_run_id: Uuid,
    pub prior_run_id: Option<Uuid>,
    pub summary_json: String,
    pub created_at: String,
}

impl ForecastDiffView {
    /// Map a row selected as: id, forecast_run_id, prior_run_id, summary_json,
    /// created_at.
    pub(crate) fn from_row(row: &Row<'_>) -> Result<Self, DbError> {
        Ok(Self {
            id: row.get(0)?,
            forecast_run_id: row.get(1)?,
            prior_run_id: row.get(2)?,
            summary_json: row.get(3)?,
            created_at: row.get(4)?,
        })
    }
}

/// The canonical, content-addressed inputs gathered for a snapshot.
pub(crate) struct GatheredInputs {
    pub content_hash: String,
    pub ledger_cutoff_op_seq: i64,
    pub recurring_events_json: String,
    pub income_sources_json: String,
    pub manual_assumption_events_json: String,
    pub scenario_overlay_ids_json: String,
}

/// Gather the current vault-state forecast inputs into canonical JSON + a content
/// hash. Deterministic: identical vault state yields an identical `content_hash`
/// (rows are ordered by id; the ledger position and household timezone are folded
/// into the hash so a ledger change invalidates it).
pub(crate) fn gather_inputs(conn: &Connection) -> Result<GatheredInputs, DbError> {
    let op_seq: i64 = conn.query_row(
        "SELECT COALESCE(MAX(op_seq), 0) FROM operation_log",
        [],
        |r| r.get(0),
    )?;
    let tz: String = conn.query_row(
        "SELECT household_timezone FROM vault_metadata WHERE singleton = 1",
        [],
        |r| r.get(0),
    )?;
    let recurring = canonical_recurring_events(conn)?;
    let income = canonical_income_sources(conn)?;
    let assumptions = canonical_active_assumptions(conn)?;
    // No scenarios table yet (personal-cfo-0mg); base captures carry an empty set.
    let scenarios = "[]".to_owned();

    let basis = format!(
        "op_seq={op_seq}\ntz={tz}\nrecurring={recurring}\nincome={income}\nassumptions={assumptions}\nscenarios={scenarios}"
    );
    Ok(GatheredInputs {
        content_hash: migrations::content_hash(&basis),
        ledger_cutoff_op_seq: op_seq,
        recurring_events_json: recurring,
        income_sources_json: income,
        manual_assumption_events_json: assumptions,
        scenario_overlay_ids_json: scenarios,
    })
}

/// Capture a content-addressed input snapshot on `conn`, returning its id and
/// content hash. **Idempotent by content:** an identical capture returns the
/// existing row (no new write). The locking/transaction boundary is the caller's
/// — [`crate::DbWorker::capture_input_snapshot`] for a standalone capture, or the
/// persist pipeline ([`persist_run_and_rows`]) for an atomic run.
pub(crate) fn capture_snapshot(conn: &Connection) -> Result<(Uuid, String), DbError> {
    let inputs = gather_inputs(conn)?;
    if let Some(existing) = conn
        .query_row(
            "SELECT id FROM forecast_input_snapshots WHERE content_hash = ?1 LIMIT 1",
            params![inputs.content_hash],
            |r| r.get::<_, Uuid>(0),
        )
        .optional()?
    {
        return Ok((existing, inputs.content_hash));
    }

    let id = Uuid::now_v7();
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO forecast_input_snapshots
            (id, household_id, created_at, ledger_cutoff_at, included_entity_hashes_json,
             source_freshness_json, schema_version, content_hash, ledger_cutoff_op_seq,
             recurring_events_snapshot_json, income_sources_snapshot_json,
             manual_assumption_events_json, scenario_overlay_ids_json)
         VALUES (?1, NULL, ?2, ?2, '{}', NULL, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            id,
            now,
            migrations::CURRENT_VERSION,
            inputs.content_hash,
            inputs.ledger_cutoff_op_seq,
            inputs.recurring_events_json,
            inputs.income_sources_json,
            inputs.manual_assumption_events_json,
            inputs.scenario_overlay_ids_json,
        ],
    )?;
    Ok((id, inputs.content_hash))
}

/// Persist a forecast run + its rows from a computed [`ForecastView`], returning
/// the new run id. The rows are the opening liquid position followed by one row
/// per cash event carrying the running balance after it (Layer-1 is
/// deterministic, so each amount/balance is a collapsed band, p10 = p50 = p90).
///
/// Deterministic by construction: the same view + `generated_at` writes identical
/// row *data* (only the run/row ids differ) — this is what makes a run reproducible
/// from its snapshot. The caller supplies the transaction.
pub(crate) fn persist_run_and_rows(
    conn: &Connection,
    generated_at: &str,
    horizon_days: u32,
    snapshot_id: Uuid,
    content_hash: &str,
    model_id: &str,
    view: &ForecastView,
) -> Result<Uuid, DbError> {
    // Point runs at a registered model (no FK; idempotent — keeps the original row).
    conn.execute(
        "INSERT OR IGNORE INTO model_registry (model_id, version, parameters_json, created_at)
         VALUES (?1, '1', NULL, ?2)",
        params![model_id, Utc::now().to_rfc3339()],
    )?;

    let starting = view.starting_balance.minor_units();
    let (mut min_p10, mut min_p50, mut min_p90) = (starting, starting, starting);
    for day in &view.days {
        min_p10 = min_p10.min(day.closing.p10.minor_units());
        min_p50 = min_p50.min(day.closing.p50.minor_units());
        min_p90 = min_p90.min(day.closing.p90.minor_units());
    }

    let run_id = Uuid::now_v7();
    let summary = format!(
        "{{\"horizon_days\":{horizon_days},\"day_count\":{}}}",
        view.days.len()
    );
    conn.execute(
        "INSERT INTO forecast_runs
            (id, generated_at, horizon_days, starting_cash_minor, model_version,
             input_snapshot_id, assumptions_hash, deterministic_run,
             p10_min_cash_minor, p50_min_cash_minor, p90_min_cash_minor, summary_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, ?9, ?10, ?11)",
        params![
            run_id,
            generated_at,
            horizon_days,
            starting,
            model_id,
            snapshot_id,
            content_hash,
            min_p10,
            min_p50,
            min_p90,
            summary,
        ],
    )?;

    // Row 0: the opening liquid position.
    insert_row(
        conn,
        run_id,
        &view.start_date.to_string(),
        starting,
        starting,
        "starting_balance",
        None,
    )?;
    // Then one row per cash event, carrying the running balance after it.
    let mut running = starting;
    for day in &view.days {
        let date = day.date.to_string();
        for event in &day.events {
            running = running
                .checked_add(event.amount.minor_units())
                .ok_or_else(|| {
                    DbError::InvalidCommand("forecast running balance overflow".to_owned())
                })?;
            insert_row(
                conn,
                run_id,
                &date,
                event.amount.minor_units(),
                running,
                &event.kind,
                Some(event.source_event_id),
            )?;
        }
    }
    Ok(run_id)
}

/// Insert one `forecast_rows` row with a collapsed band (Layer-1 deterministic).
fn insert_row(
    conn: &Connection,
    run_id: Uuid,
    date: &str,
    amount_minor: i64,
    running_minor: i64,
    source_type: &str,
    source_id: Option<Uuid>,
) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO forecast_rows
            (id, forecast_run_id, date, amount_p10_minor, amount_p50_minor, amount_p90_minor,
             running_balance_p10_minor, running_balance_p50_minor, running_balance_p90_minor,
             source_type, source_id, computation_mode)
         VALUES (?1, ?2, ?3, ?4, ?4, ?4, ?5, ?5, ?5, ?6, ?7, 'full_batch')",
        params![
            Uuid::now_v7(),
            run_id,
            date,
            amount_minor,
            running_minor,
            source_type,
            source_id,
        ],
    )?;
    Ok(())
}

/// The in-forecast recurring obligations as a canonical JSON array (ordered by id).
fn canonical_recurring_events(conn: &Connection) -> Result<String, DbError> {
    let mut stmt = conn.prepare(
        "SELECT e.id, e.name, e.amount_expected_minor, e.currency, e.frequency,
                e.next_expected_date, b.type
         FROM recurring_events e
         LEFT JOIN bill_contracts b ON b.recurring_event_id = e.id
         WHERE e.include_in_forecast = 1 AND e.is_active = 1
         ORDER BY e.id",
    )?;
    let mut items = Vec::new();
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        let amount: i64 = r.get(2)?;
        let id = json_str(&r.get::<_, Uuid>(0)?.to_string());
        let name = json_str(&r.get::<_, String>(1)?);
        let currency = json_str(&r.get::<_, String>(3)?);
        let frequency = json_str(&r.get::<_, String>(4)?);
        let anchor = json_opt(r.get::<_, Option<String>>(5)?.as_deref());
        let bill_type = json_opt(r.get::<_, Option<String>>(6)?.as_deref());
        items.push(format!(
            "{{\"id\":{id},\"name\":{name},\"amount_expected_minor\":{amount},\"currency\":{currency},\"frequency\":{frequency},\"next_expected_date\":{anchor},\"bill_type\":{bill_type}}}"
        ));
    }
    Ok(format!("[{}]", items.join(",")))
}

/// The active income sources as a canonical JSON array (ordered by id).
fn canonical_income_sources(conn: &Connection) -> Result<String, DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, name, net_minor_units, currency, frequency, anchor_date
         FROM income_sources WHERE active = 1 ORDER BY id",
    )?;
    let mut items = Vec::new();
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        let net: i64 = r.get(2)?;
        let id = json_str(&r.get::<_, Uuid>(0)?.to_string());
        let name = json_str(&r.get::<_, String>(1)?);
        let currency = json_str(&r.get::<_, String>(3)?);
        let frequency = json_str(&r.get::<_, String>(4)?);
        let anchor = json_str(&r.get::<_, String>(5)?);
        items.push(format!(
            "{{\"id\":{id},\"name\":{name},\"net_minor_units\":{net},\"currency\":{currency},\"frequency\":{frequency},\"anchor_date\":{anchor}}}"
        ));
    }
    Ok(format!("[{}]", items.join(",")))
}

/// The active base assumption events as a canonical JSON array (ordered by id).
fn canonical_active_assumptions(conn: &Connection) -> Result<String, DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, kind, target_entity_type, target_entity_id, params_json, source
         FROM forecast_assumption_events
         WHERE status = 'active' AND scenario_id IS NULL
         ORDER BY id",
    )?;
    let mut items = Vec::new();
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        let id = json_str(&r.get::<_, Uuid>(0)?.to_string());
        let kind = json_str(&r.get::<_, String>(1)?);
        let target_type = json_opt(r.get::<_, Option<String>>(2)?.as_deref());
        let target_id = json_opt(
            r.get::<_, Option<Uuid>>(3)?
                .map(|u| u.to_string())
                .as_deref(),
        );
        let params = json_str(&r.get::<_, String>(4)?);
        let source = json_str(&r.get::<_, String>(5)?);
        items.push(format!(
            "{{\"id\":{id},\"kind\":{kind},\"target_entity_type\":{target_type},\"target_entity_id\":{target_id},\"params_json\":{params},\"source\":{source}}}"
        ));
    }
    Ok(format!("[{}]", items.join(",")))
}

/// Escape `s` into a quoted JSON string literal. db-worker builds JSON by hand
/// (`serde_json` is kept out of the write path), and content-addressing needs a
/// deterministic, correctly-escaped representation. Shared with the manual-entry
/// write path ([`crate::manual_entry`]).
pub(crate) fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `json_str` for an optional value; `None` becomes the JSON literal `null`.
fn json_opt(value: Option<&str>) -> String {
    value.map_or_else(|| "null".to_owned(), json_str)
}
