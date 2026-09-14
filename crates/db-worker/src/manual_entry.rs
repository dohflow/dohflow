//! Manual future entries (ADR 0026, personal-cfo-q6gh): user-added one-time cash
//! events that move the Future Cash projection.
//!
//! A manual entry is a `5u2` assumption event (`kind = one_time_event`, `source =
//! user_override`) whose `params_json` carries the signed amount, currency, date,
//! and label. The forecast folds the active base entries into the Layer-1
//! projection as `ManualOneOff` events ([`crate::forecast`]); edits supersede and
//! deletes clear (never silent mutation). Writes build `params_json` by hand
//! (`serde_json` stays out of the write path); reads parse it with `serde_json`.

use chrono::NaiveDate;
use core_money::Money;
use rusqlite::Connection;
use uuid::Uuid;

use crate::forecast_persist::json_str;
use crate::{currency_from_code, DbError};

/// A user-added one-time future cash event (the read model over its assumption
/// event).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualEntry {
    pub id: Uuid,
    /// Signed amount applied to the running balance (inflow +, outflow −).
    pub amount: Money,
    pub occurs_on: NaiveDate,
    pub label: String,
    /// The liquid account this flow is attributed to (from/to), if the user chose one
    /// (personal-cfo-4d8.24.3). `None` ⇒ the flow lands in the Unallocated series. This
    /// only routes the *projected* flow; a manual entry stays a forecast assumption, never
    /// a ledger transaction (ADR 0026 §12).
    pub account_id: Option<Uuid>,
    /// The committed transaction the deterministic matcher linked this entry to
    /// (ADR 0026 addendum 2026-09-02, personal-cfo-xtz5). A matched entry no
    /// longer projects — the real flow is in the balance. Derived (from
    /// `manual_entry_links`), never authored; always `None` for
    /// scenario-scoped entries.
    pub matched_transaction_id: Option<Uuid>,
    pub created_at: String,
}

/// Build the `params_json` a manual entry stores on its assumption event — by hand
/// (`serde_json` is kept out of the write path). `account_id`, when set, is written as an
/// optional key so existing entries (which lack it) stay valid.
pub(crate) fn build_params_json(
    amount: Money,
    occurs_on: NaiveDate,
    label: &str,
    account_id: Option<Uuid>,
) -> String {
    let account = match account_id {
        Some(id) => format!(",\"account_id\":{}", json_str(&id.to_string())),
        None => String::new(),
    };
    format!(
        "{{\"amount_minor\":{},\"currency\":{},\"date\":{},\"label\":{}{}}}",
        amount.minor_units(),
        json_str(amount.currency().code()),
        json_str(&occurs_on.to_string()),
        json_str(label),
        account,
    )
}

/// Parse a manual entry's `params_json` (the read path — `serde_json`).
fn from_params(id: Uuid, params_json: &str, created_at: String) -> Result<ManualEntry, DbError> {
    let value: serde_json::Value = serde_json::from_str(params_json)
        .map_err(|e| DbError::InvalidCommand(format!("bad manual-entry params: {e}")))?;
    let amount_minor = value
        .get("amount_minor")
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| DbError::InvalidCommand("manual entry missing amount_minor".to_owned()))?;
    let currency_code = value
        .get("currency")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| DbError::InvalidCommand("manual entry missing currency".to_owned()))?;
    let date_str = value
        .get("date")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| DbError::InvalidCommand("manual entry missing date".to_owned()))?;
    let label = value
        .get("label")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    // Optional (personal-cfo-4d8.24.3): entries stored before this key existed, or with no
    // account chosen, parse to `None`. A malformed value is treated as absent, never an error.
    let account_id = value
        .get("account_id")
        .and_then(serde_json::Value::as_str)
        .and_then(|s| Uuid::parse_str(s).ok());

    let currency = currency_from_code(currency_code)?;
    let occurs_on = NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
        .map_err(|e| DbError::InvalidCommand(format!("bad manual-entry date {date_str:?}: {e}")))?;
    Ok(ManualEntry {
        id,
        amount: Money::new(amount_minor, currency),
        occurs_on,
        label,
        account_id,
        created_at,
        matched_transaction_id: None,
    })
}

/// The active manual entries (status `active`, `one_time_event`) for a run: base
/// (`scenario_id IS NULL`) plus, when `scenario` is set, that scenario's. Oldest
/// first. The IPC list passes `None` (base only); the forecast passes the run's
/// scenario (personal-cfo-6zep).
pub(crate) fn active_manual_entries(
    conn: &Connection,
    scenarios: &[Uuid],
) -> Result<Vec<ManualEntry>, DbError> {
    // Additive, not overriding: a one-off event from ANY selected scenario contributes, so
    // membership is all this needs — no precedence (ADR 0059 §1 governs overrides).
    let clause = if scenarios.is_empty() {
        String::new()
    } else {
        format!(
            " OR scenario_id IN ({})",
            vec!["?"; scenarios.len()].join(", ")
        )
    };
    let mut stmt = conn.prepare(&format!(
        "SELECT e.id, e.params_json, e.created_at, l.linked_transaction_id
         FROM forecast_assumption_events e
         LEFT JOIN manual_entry_links l ON l.assumption_event_id = e.id
         WHERE e.status = 'active' AND e.kind = 'one_time_event'
           AND (e.scenario_id IS NULL{clause})
         ORDER BY e.created_at, e.id"
    ))?;
    let binds: Vec<&dyn rusqlite::ToSql> = scenarios
        .iter()
        .map(|s| s as &dyn rusqlite::ToSql)
        .collect();
    let mut out = Vec::new();
    let mut rows = stmt.query(rusqlite::params_from_iter(binds))?;
    while let Some(row) = rows.next()? {
        let id: Uuid = row.get(0)?;
        let params_json: String = row.get(1)?;
        let created_at: String = row.get(2)?;
        let matched: Option<Uuid> = row.get(3)?;
        let mut entry = from_params(id, &params_json, created_at)?;
        entry.matched_transaction_id = matched;
        out.push(entry);
    }
    Ok(out)
}

/// Rebuild the one-off entry matcher projection (ADR 0026 addendum 2026-09-02,
/// personal-cfo-xtz5). Deliberately strict v1 rules: base (non-scenario)
/// entries WITH an attributed account only; the transaction must be committed,
/// non-voided, on that account's ledger account, with the EXACT signed entry
/// amount, within ±[`crate::recurring_instances::RECURRING_MATCH_WINDOW_DAYS`]
/// days; a transaction already linked to a recurring instance is never taken,
/// and each transaction is claimed by at most one entry. Greedy nearest-date,
/// ties by transaction id then entry order — byte-stable rebuilds.
pub(crate) fn rebuild_links(conn: &Connection, today: NaiveDate) -> Result<u64, DbError> {
    use std::collections::HashSet;
    conn.execute("DELETE FROM manual_entry_links", [])?;

    let entries = active_manual_entries(conn, &[])?;
    let mut claimed: HashSet<Uuid> = HashSet::new();
    let mut linked: u64 = 0;
    // Stamped with the rebuild day, not the wall clock: rebuilds must be
    // byte-stable within a day (sibling-projection convention).
    let now = today.to_string();
    let mut stmt = conn.prepare(
        "SELECT lt.id, lp.posting_date FROM ledger_postings lp
         JOIN ledger_transactions lt ON lt.id = lp.transaction_id
         JOIN accounts a ON a.ledger_account_id = lp.ledger_account_id
         WHERE a.id = ?1
           AND lp.minor_units = ?2
           AND a.currency = ?3
           AND lt.voided_at IS NULL
           AND lp.posting_date BETWEEN ?4 AND ?5
           AND NOT EXISTS (
             SELECT 1 FROM recurring_event_instances i
              WHERE i.linked_transaction_id = lt.id
           )
         ORDER BY lp.posting_date, lt.id",
    )?;
    for entry in entries {
        let Some(account_id) = entry.account_id else {
            continue;
        };
        let window = crate::recurring_instances::RECURRING_MATCH_WINDOW_DAYS;
        let lo = (entry.occurs_on - chrono::Duration::days(window)).to_string();
        let hi = (entry.occurs_on + chrono::Duration::days(window)).to_string();
        let candidates: Vec<(Uuid, String)> = stmt
            .query_map(
                rusqlite::params![
                    account_id,
                    entry.amount.minor_units(),
                    entry.amount.currency().code(),
                    lo,
                    hi
                ],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?
            .collect::<Result<_, _>>()?;
        let best = candidates
            .iter()
            .filter(|(txn, _)| !claimed.contains(txn))
            .min_by_key(|(txn, date)| {
                let diff = NaiveDate::parse_from_str(date, "%Y-%m-%d")
                    .map(|d| (d - entry.occurs_on).num_days().abs())
                    .unwrap_or(i64::MAX);
                (diff, *txn)
            });
        if let Some((txn, date)) = best {
            conn.execute(
                "INSERT INTO manual_entry_links
                    (assumption_event_id, linked_transaction_id, matched_on, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![entry.id, txn, date, now],
            )?;
            claimed.insert(*txn);
            linked += 1;
        }
    }
    Ok(linked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_money::Currency;

    fn round_trip(json: &str) -> ManualEntry {
        from_params(Uuid::from_u128(1), json, "2026-07-08".to_owned()).unwrap()
    }

    #[test]
    fn account_id_round_trips_through_params_json() {
        let account = Uuid::from_u128(42);
        let json = build_params_json(
            Money::new(-32_000, Currency::Usd),
            NaiveDate::from_ymd_opt(2026, 8, 15).unwrap(),
            "Property tax",
            Some(account),
        );
        let entry = round_trip(&json);
        assert_eq!(entry.account_id, Some(account));
        assert_eq!(entry.amount, Money::new(-32_000, Currency::Usd));
        assert_eq!(entry.label, "Property tax");

        // No account chosen → the key is omitted and parses back to None.
        let no_account = build_params_json(
            Money::new(5_000, Currency::Usd),
            NaiveDate::from_ymd_opt(2026, 7, 31).unwrap(),
            "Gift",
            None,
        );
        assert!(!no_account.contains("account_id"));
        assert_eq!(round_trip(&no_account).account_id, None);
    }

    #[test]
    fn legacy_params_without_account_id_parse_to_none() {
        // An entry stored before the account_id key existed still parses (backward compat).
        let legacy =
            r#"{"amount_minor":-32000,"currency":"USD","date":"2026-08-15","label":"Rent"}"#;
        let entry = round_trip(legacy);
        assert_eq!(entry.account_id, None);
        assert_eq!(entry.label, "Rent");
    }
}
