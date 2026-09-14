//! Forecast actualization (ADR 0026 §9/§17; bead personal-cfo-46jq).
//!
//! Scores each persisted forecast row against what actually happened, writing
//! `forecast_actuals`. The bridge is the recurring-instance read model
//! ([`crate::recurring_instances`]): a forecast row predicts a cash event for
//! `(source_id, date)`; the instance for that `(entity, date)` records whether the
//! occurrence was paid (linked to a realized transaction) or not. So:
//!
//! - **exact** — the occurrence is linked and the realized amount + date land within
//!   the tight band of the prediction.
//! - **matched** — linked, but the realized amount or date drifts beyond the tight
//!   band (still within the looser link tolerance, by construction).
//! - **missed** — a scheduled occurrence with no link whose date is now past.
//! - **superseded** — an older run's row for a `(entity, date)` that a newer run also
//!   predicted; the newest run's row is scored, the stale ones are not.
//!
//! Full recompute, idempotent: clears and rewrites `forecast_actuals` from canonical
//! state. Only scheduled source types (income / recurring bill / loan payment) carry
//! instances and are actualized; only rows whose date is on or before `today` are
//! resolved. Deterministic given `today` — the test seam is [`actualize_at`].

use chrono::NaiveDate;
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::forecast::parse_date;
use crate::DbError;

/// Tight amount band (bps) separating `exact` from `matched`.
const EXACT_AMOUNT_BPS: i64 = 100;
/// Absolute floor for the tight amount band, in minor units.
const EXACT_AMOUNT_FLOOR_MINOR: i64 = 100;
/// Tight date band (days) separating `exact` from `matched`.
const EXACT_DATE_DAYS: i64 = 2;

/// Source types that have recurring instances and can be actualized.
const ACTUALIZABLE_SOURCE_TYPES: &str = "('income', 'recurring_bill', 'loan_payment')";

/// One persisted prediction joined to its recurring instance.
struct Candidate {
    run_id: Uuid,
    generated_at: String,
    row_id: Uuid,
    source_id: Uuid,
    predicted_date: NaiveDate,
    predicted_amount_minor: i64,
    currency: String,
    instance_paid: bool,
    linked_transaction_id: Option<Uuid>,
}

/// Recompute `forecast_actuals` for every actualizable row dated on or before
/// `today`. Returns the number of rows written.
///
/// # Errors
/// Returns [`DbError`] if a SQLite operation fails or a stored date is malformed.
pub(crate) fn actualize(conn: &Connection, today: NaiveDate) -> Result<u64, DbError> {
    conn.execute("DELETE FROM forecast_actuals", [])?;

    let candidates = load_candidates(conn, today)?;
    let created_at = today.to_string();
    let mut count = 0u64;
    for c in &candidates {
        let superseded = is_superseded(conn, c)?;
        let (match_status, realized_date, realized_amount_minor, matched_txn) = if superseded {
            ("superseded", c.predicted_date, 0, None)
        } else if c.instance_paid {
            resolve_paid(conn, c)?
        } else {
            ("missed", c.predicted_date, 0, None)
        };

        conn.execute(
            "INSERT INTO forecast_actuals
                (id, forecast_run_id, forecast_row_id, realized_date,
                 realized_amount_minor, currency, match_status,
                 matched_transaction_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                actual_id(c.run_id, c.row_id),
                c.run_id,
                c.row_id,
                realized_date.to_string(),
                realized_amount_minor,
                c.currency,
                match_status,
                matched_txn,
                created_at,
            ],
        )?;
        count += 1;
    }
    Ok(count)
}

/// Deterministic actuals id from `(run, row)`, so a recompute is byte-identical and
/// the dedup key `(forecast_run_id, forecast_row_id)` is honored.
fn actual_id(run_id: Uuid, row_id: Uuid) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("forecast_actual:{run_id}:{row_id}").as_bytes(),
    )
}

/// Every actualizable forecast row dated `<= today`, joined to its instance.
fn load_candidates(conn: &Connection, today: NaiveDate) -> Result<Vec<Candidate>, DbError> {
    let sql = format!(
        "SELECT fr.forecast_run_id, run.generated_at, fr.id, fr.source_id, fr.date,
                fr.amount_p50_minor, rei.currency, rei.status, rei.linked_transaction_id
         FROM forecast_rows fr
         JOIN forecast_runs run ON run.id = fr.forecast_run_id
         JOIN recurring_event_instances rei
           ON rei.recurring_event_id = fr.source_id
          AND rei.scheduled_date = fr.date
         WHERE fr.source_type IN {ACTUALIZABLE_SOURCE_TYPES}
           AND fr.source_id IS NOT NULL
           AND fr.date <= ?1
         ORDER BY fr.forecast_run_id, fr.date, fr.id"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params![today.to_string()], |r| {
            Ok((
                r.get::<_, Uuid>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Uuid>(2)?,
                r.get::<_, Uuid>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, Option<Uuid>>(8)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut out = Vec::with_capacity(rows.len());
    for (run_id, generated_at, row_id, source_id, date, p50, currency, status, linked) in rows {
        out.push(Candidate {
            run_id,
            generated_at,
            row_id,
            source_id,
            predicted_date: parse_date(&date)?,
            predicted_amount_minor: p50,
            currency,
            instance_paid: status == "paid",
            linked_transaction_id: linked,
        });
    }
    Ok(out)
}

/// A row is superseded when a strictly-newer run also predicted the same
/// `(source_id, date)` — only the newest run's prediction is scored.
fn is_superseded(conn: &Connection, c: &Candidate) -> Result<bool, DbError> {
    let superseded: bool = conn.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM forecast_rows fr2
             JOIN forecast_runs run2 ON run2.id = fr2.forecast_run_id
             WHERE fr2.source_id = ?1 AND fr2.date = ?2
               AND run2.generated_at > ?3
         )",
        params![c.source_id, c.predicted_date.to_string(), c.generated_at],
        |r| r.get(0),
    )?;
    Ok(superseded)
}

/// Resolve a linked (paid) instance to `exact`/`matched` plus the realized facts.
fn resolve_paid(
    conn: &Connection,
    c: &Candidate,
) -> Result<(&'static str, NaiveDate, i64, Option<Uuid>), DbError> {
    let Some(txn) = c.linked_transaction_id else {
        // Defensive: a `paid` instance always carries a link. Treat a missing one as
        // unresolved rather than panicking.
        return Ok(("missed", c.predicted_date, 0, None));
    };
    let Some((realized_date, realized_minor)) = realized_posting(conn, txn)? else {
        return Ok(("missed", c.predicted_date, 0, None));
    };

    let tol =
        (c.predicted_amount_minor.abs() * EXACT_AMOUNT_BPS / 10_000).max(EXACT_AMOUNT_FLOOR_MINOR);
    let amount_ok = (realized_minor - c.predicted_amount_minor).abs() <= tol;
    let date_ok = (realized_date - c.predicted_date).num_days().abs() <= EXACT_DATE_DAYS;
    let status = if amount_ok && date_ok {
        "exact"
    } else {
        "matched"
    };
    Ok((status, realized_date, realized_minor, Some(txn)))
}

/// The realized `(date, signed minor units)` of a linked transaction's posting on a user
/// account. Mirrors the linking seam's collection roles (`recurring_instances`, ADR 0047
/// §1): a card-charged bill's link is a CREDIT-FACILITY posting, and scoring it as
/// `missed` because it has no liquid leg would put a permanent 100%-error row on every
/// matched occurrence (adversarial review of 4d8.25.8). Liquid postings sort first so a
/// mixed transaction keeps the pre-0047 result.
fn realized_posting(conn: &Connection, txn: Uuid) -> Result<Option<(NaiveDate, i64)>, DbError> {
    let row = conn
        .query_row(
            "SELECT lt.occurred_at, lp.minor_units
             FROM ledger_postings lp
             JOIN ledger_transactions lt ON lt.id = lp.transaction_id
             JOIN ledger_accounts la ON la.id = lp.ledger_account_id
             JOIN accounts a ON a.ledger_account_id = la.id
             WHERE lt.id = ?1
               AND a.cashflow_role IN ('liquid_cash', 'credit_facility')
               AND lt.voided_at IS NULL
             ORDER BY CASE a.cashflow_role WHEN 'liquid_cash' THEN 0 ELSE 1 END,
                      lt.occurred_at, lp.minor_units
             LIMIT 1",
            params![txn],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
        )
        .optional()?;
    match row {
        Some((occurred_at, minor)) if occurred_at.len() >= 10 => {
            Ok(Some((parse_date(&occurred_at[..10])?, minor)))
        }
        _ => Ok(None),
    }
}

/// Read the actualization rows for one run, newest realized first — the seam the
/// readiness factors + accuracy surface (A3) read.
///
/// # Errors
/// Returns [`DbError`] if the read fails.
pub(crate) fn count(conn: &Connection) -> Result<u64, DbError> {
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM forecast_actuals", [], |r| r.get(0))?;
    Ok(u64::try_from(n).unwrap_or(0))
}
