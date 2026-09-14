//! Shared readers for the recurring **schedule sources** that drive both the
//! Future Cash forecast ([`crate::forecast`]) and the recurring-instance
//! projection ([`crate::recurring_instances`], personal-cfo-5ie.4).
//!
//! These are the single source of truth for *which active schedules exist and
//! their base cadence/amount*. Both consumers expand the same sources through the
//! same [`pay_schedule::PaySchedule`], so a projected instance and its forecast
//! row share `(entity, date, amount)` by construction — the agreement the
//! actualization loop (ADR 0026 §9) depends on. Expansion, overrides, and sign
//! handling stay with each consumer; only the source fetch is shared, so the two
//! cannot silently drift apart (e.g. a `WHERE` clause added to one but not the
//! other).

use rusqlite::Connection;
use uuid::Uuid;

use crate::DbError;

/// Whether a schedule contributes an inflow (income) or an outflow (a recurring
/// obligation / bill). The base amount is stored positive either way; the sign is
/// implied by the kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScheduleKind {
    /// An income source — inflow (positive).
    Income,
    /// A recurring obligation — outflow (negated by the consumer). Includes loan
    /// payments; `bill_type` distinguishes them where a consumer cares.
    Obligation,
}

/// One active recurring schedule source, before occurrence expansion. `amount_minor`
/// is the base **positive** magnitude (income net pay / obligation expected amount).
/// `anchor` is the cadence anchor string (income `anchor_date`; obligation
/// `next_expected_date`, which may be absent). `freq_token`/`currency_code` are raw
/// tokens the consumer parses with the shared helpers.
pub(crate) struct ScheduleSource {
    pub id: Uuid,
    pub name: String,
    pub amount_minor: i64,
    pub currency_code: String,
    pub freq_token: String,
    pub anchor: Option<String>,
    /// The `bill_contracts.type` token, when this is an obligation backed by a bill
    /// contract (drives the forecast's loan-vs-bill `EventKind`); `None` for income.
    pub bill_type: Option<String>,
    pub kind: ScheduleKind,
    /// The obligation's pay-from account (`recurring_events.autopay_account_id`), when
    /// set — the instance-linking account gate (ADR 0047 §1). Always `None` for income.
    pub autopay_account_id: Option<Uuid>,
}

/// Every active income source (inflows). Mirrors the forecast's income query so the
/// two expand the same set in the same order.
///
/// # Errors
/// Returns [`DbError`] if the read fails.
pub(crate) fn active_income_schedules(conn: &Connection) -> Result<Vec<ScheduleSource>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, name, net_minor_units, currency, frequency, anchor_date
         FROM income_sources
         WHERE active = 1
         ORDER BY name COLLATE NOCASE, id",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(ScheduleSource {
                id: r.get(0)?,
                name: r.get(1)?,
                amount_minor: r.get(2)?,
                currency_code: r.get(3)?,
                freq_token: r.get(4)?,
                anchor: Some(r.get::<_, String>(5)?),
                bill_type: None,
                kind: ScheduleKind::Income,
                autopay_account_id: None,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Every active, in-forecast recurring obligation (outflows), joined to its bill
/// contract for the type token. Mirrors the forecast's obligation query.
///
/// # Errors
/// Returns [`DbError`] if the read fails.
pub(crate) fn active_obligation_schedules(
    conn: &Connection,
) -> Result<Vec<ScheduleSource>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT e.id, e.name, e.amount_expected_minor, e.currency, e.frequency,
                e.next_expected_date, b.type, e.autopay_account_id
         FROM recurring_events e
         LEFT JOIN bill_contracts b ON b.recurring_event_id = e.id
         WHERE e.include_in_forecast = 1 AND e.is_active = 1
         ORDER BY e.name COLLATE NOCASE, e.id",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(ScheduleSource {
                id: r.get(0)?,
                name: r.get(1)?,
                amount_minor: r.get(2)?,
                currency_code: r.get(3)?,
                freq_token: r.get(4)?,
                anchor: r.get::<_, Option<String>>(5)?,
                bill_type: r.get::<_, Option<String>>(6)?,
                kind: ScheduleKind::Obligation,
                autopay_account_id: r.get::<_, Option<Uuid>>(7)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}
