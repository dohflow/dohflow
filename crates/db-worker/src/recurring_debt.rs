//! Recurring extra-debt-payment overlays (personal-cfo-6wk.19).
//!
//! The read side of the `recurring_debt_payment` assumption kind: an extra monthly debt payment
//! recorded as a scenario overlay (ADR 0036 — "an extra $X/mo against debt from D"). Parsed with
//! `serde_json` (matching the hand-built write path in [`crate::forecast_events`]) and folded into
//! the projection as a recurring liquid outflow by [`crate::forecast`]. Scenario-scoped: base
//! (`scenario_id IS NULL`) overlays always apply; a scenario's only under that scenario.

use chrono::NaiveDate;
use core_money::Money;
use rusqlite::Connection;
use uuid::Uuid;

use crate::{currency_from_code, DbError};

/// A recurring monthly extra debt payment: `amount` is the positive magnitude (the fold projects
/// it as a negative outflow), recurring on `anchor_date`'s day-of-month through `end_date`
/// (`None` = through the forecast horizon).
pub(crate) struct RecurringDebtPayment {
    pub id: Uuid,
    pub amount: Money,
    pub anchor_date: NaiveDate,
    pub end_date: Option<NaiveDate>,
    pub label: String,
}

fn parse_date(s: &str) -> Result<NaiveDate, DbError> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|e| DbError::InvalidCommand(format!("bad recurring-debt date {s:?}: {e}")))
}

/// Parse a recurring extra-debt-payment's `params_json` (the read path — `serde_json`).
fn from_params(id: Uuid, params_json: &str) -> Result<RecurringDebtPayment, DbError> {
    let value: serde_json::Value = serde_json::from_str(params_json)
        .map_err(|e| DbError::InvalidCommand(format!("bad recurring-debt params: {e}")))?;
    let amount_minor = value
        .get("amount_minor")
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| DbError::InvalidCommand("recurring debt missing amount_minor".to_owned()))?;
    let currency_code = value
        .get("currency")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| DbError::InvalidCommand("recurring debt missing currency".to_owned()))?;
    let anchor_str = value
        .get("anchor_date")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| DbError::InvalidCommand("recurring debt missing anchor_date".to_owned()))?;
    let label = value
        .get("label")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let end_date = match value.get("end_date").and_then(serde_json::Value::as_str) {
        Some(s) => Some(parse_date(s)?),
        None => None,
    };
    Ok(RecurringDebtPayment {
        id,
        amount: Money::new(amount_minor, currency_from_code(currency_code)?),
        anchor_date: parse_date(anchor_str)?,
        end_date,
        label,
    })
}

/// The active recurring extra-debt-payment overlays for a run: base (`scenario_id IS NULL`) plus,
/// when `scenario` is set, that scenario's. Oldest first.
pub(crate) fn active_recurring_debt_payments(
    conn: &Connection,
    scenarios: &[Uuid],
) -> Result<Vec<RecurringDebtPayment>, DbError> {
    // Additive, not overriding: a debt-payment overlay from ANY selected scenario
    // contributes, so membership is all this needs (ADR 0059 §1 governs overrides).
    let clause = if scenarios.is_empty() {
        String::new()
    } else {
        format!(
            " OR scenario_id IN ({})",
            vec!["?"; scenarios.len()].join(", ")
        )
    };
    let mut stmt = conn.prepare(&format!(
        "SELECT id, params_json
         FROM forecast_assumption_events
         WHERE status = 'active' AND kind = 'recurring_debt_payment'
           AND (scenario_id IS NULL{clause})
         ORDER BY created_at, id"
    ))?;
    let mut out = Vec::new();
    let binds: Vec<&dyn rusqlite::ToSql> = scenarios
        .iter()
        .map(|s| s as &dyn rusqlite::ToSql)
        .collect();
    let mut rows = stmt.query(rusqlite::params_from_iter(binds))?;
    while let Some(row) = rows.next()? {
        let id: Uuid = row.get(0)?;
        let params_json: String = row.get(1)?;
        out.push(from_params(id, &params_json)?);
    }
    Ok(out)
}
