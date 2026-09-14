//! Commitments projection (plan §9.8; bead personal-cfo-rxw).
//!
//! `commitments` are the forecast-facing obligation records the deterministic
//! Future Cash ledger (personal-cfo-164u) and the Money Inbox consume. The table
//! is a **derived read model**: every row is rebuildable from the canonical
//! `recurring_events` + `bill_contracts` tables, so this module is the only place
//! that writes it (command code never touches `commitments`).
//!
//! The projection is currently full-rebuild only (no incremental path): one
//! commitment per active, in-forecast recurring event, enriched by its linked
//! bill contract. The commitment **reuses its source recurring event's id**, so a
//! rebuild is deterministic — same canonical state in, byte-identical commitments
//! out (asserted by the idempotency test).

use chrono::Utc;
use core_ledger::{AccountId, CommitmentId};
use rusqlite::{params, Connection};
use uuid::Uuid;

use crate::DbError;

const PROJECTION_NAME: &str = "commitments";

/// A forecast-facing obligation, derived from a recurring event (+ its bill
/// contract). A read-model row; never hand-written. Timestamps are intentionally
/// omitted so the content checksum is stable across rebuilds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitmentView {
    /// Commitment id (equals its source recurring event's id).
    pub id: CommitmentId,
    /// Obligation kind token (e.g. `subscription`, `mortgage`).
    pub commitment_type: String,
    /// Display name (carried from the recurring event).
    pub name: String,
    /// Expected outflow per occurrence, in minor units.
    pub amount_expected_minor: Option<i64>,
    /// Opaque due-date rule JSON (carried from the bill contract).
    pub due_rule_json: Option<String>,
    /// Account the obligation is paid from, if known.
    pub payment_source_account_id: Option<AccountId>,
    /// Autopay state token (`enabled` / `disabled` / `unknown`).
    pub autopay_status: String,
    /// Which canonical entity this was projected from (`recurring_event`).
    pub source_entity_type: Option<String>,
    /// The source entity's id.
    pub source_entity_id: Option<Uuid>,
    /// Whether the obligation feeds the forecast.
    pub include_in_forecast: bool,
    /// Lifecycle token (`active` / `paused` / …).
    pub status: String,
}

/// Map a bill-contract `type` to the commitment-type token. Without a linked
/// contract the obligation is generic (`other`).
fn commitment_type_for(contract_type: Option<&str>) -> &'static str {
    match contract_type {
        Some("rent_mortgage") => "mortgage",
        Some("utility") => "utility",
        Some("insurance") => "insurance",
        Some("subscription") => "subscription",
        Some("loan_payment") => "loan_payment",
        Some("tax") => "tax_reserve",
        Some("membership") => "membership",
        Some("childcare") => "childcare",
        _ => "other",
    }
}

/// Full rebuild in its own transaction: clear `commitments` and re-derive every
/// row from the canonical `recurring_events` + `bill_contracts` tables. Returns
/// the number of rows written.
///
/// # Errors
/// Returns [`DbError`] if a SQLite operation fails.
pub(crate) fn rebuild(conn: &mut Connection) -> Result<u64, DbError> {
    let tx = conn.transaction()?;
    let count = rebuild_in(&tx)?;
    tx.commit()?;
    Ok(count)
}

/// Re-derive the projection within an existing connection or transaction (no
/// commit), so a command's apply transaction can refresh commitments atomically
/// with its write. The commitment reuses its source event's id, so the result is
/// deterministic.
///
/// # Errors
/// Returns [`DbError`] if a SQLite operation fails.
pub(crate) fn rebuild_in(conn: &Connection) -> Result<u64, DbError> {
    conn.execute("DELETE FROM commitments", [])?;

    let mut stmt = conn.prepare(
        "SELECT e.id, e.name, e.amount_expected_minor, e.autopay_account_id,
                b.type, b.due_rule_json, e.autopay_enabled
         FROM recurring_events e
         LEFT JOIN bill_contracts b ON b.recurring_event_id = e.id
         WHERE e.is_active = 1 AND e.include_in_forecast = 1
         ORDER BY e.name COLLATE NOCASE, e.id",
    )?;
    let derived = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, Uuid>(0)?,           // recurring event id (= commitment id)
                r.get::<_, String>(1)?,         // name
                r.get::<_, Option<i64>>(2)?,    // amount_expected_minor
                r.get::<_, Option<Uuid>>(3)?,   // autopay account -> payment source
                r.get::<_, Option<String>>(4)?, // bill_contract.type
                r.get::<_, Option<String>>(5)?, // due_rule_json
                r.get::<_, Option<i64>>(6)?,    // autopay_enabled (intent, ADR 0041)
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);

    let now = Utc::now().to_rfc3339();
    for (event_id, name, amount, autopay, contract_type, due_rule, autopay_enabled) in &derived {
        let commitment_type = commitment_type_for(contract_type.as_deref());
        // Autopay status reflects explicit intent (ADR 0041), not account presence.
        let autopay_status = match autopay_enabled {
            Some(1) => "enabled",
            Some(0) => "disabled",
            _ => "unknown",
        };
        conn.execute(
            "INSERT INTO commitments (
                id, commitment_type, name, amount_expected_minor, amount_confidence_bps,
                due_rule_json, payment_source_account_id, autopay_status,
                source_entity_type, source_entity_id, include_in_forecast, status,
                created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6, ?7, 'recurring_event', ?1, 1, 'active', ?8, ?8)",
            params![
                event_id,
                commitment_type,
                name,
                amount,
                due_rule,
                autopay,
                autopay_status,
                now,
            ],
        )?;
    }

    set_cursor(conn, op_log_head(conn)?, checksum(conn)?)?;
    Ok(derived.len() as u64)
}

/// Read all commitments in deterministic order.
///
/// # Errors
/// Returns [`DbError`] if the read fails.
pub(crate) fn read_rows(conn: &Connection) -> Result<Vec<CommitmentView>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, commitment_type, name, amount_expected_minor, due_rule_json,
                payment_source_account_id, autopay_status, source_entity_type,
                source_entity_id, include_in_forecast, status
         FROM commitments
         ORDER BY name COLLATE NOCASE, id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(CommitmentView {
            id: CommitmentId::from_uuid(r.get::<_, Uuid>(0)?),
            commitment_type: r.get(1)?,
            name: r.get(2)?,
            amount_expected_minor: r.get(3)?,
            due_rule_json: r.get(4)?,
            payment_source_account_id: r.get::<_, Option<Uuid>>(5)?.map(AccountId::from_uuid),
            autopay_status: r.get(6)?,
            source_entity_type: r.get(7)?,
            source_entity_id: r.get::<_, Option<Uuid>>(8)?,
            include_in_forecast: r.get::<_, i64>(9)? != 0,
            status: r.get(10)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// A stable content checksum over the commitment rows (FNV-1a over the
/// deterministically-ordered rows). Proves the rebuild is deterministic.
///
/// # Errors
/// Returns [`DbError`] if the read fails.
pub(crate) fn checksum(conn: &Connection) -> Result<u64, DbError> {
    let rows = read_rows(conn)?;
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for row in &rows {
        for byte in format!("{row:?}").bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    Ok(hash)
}

/// The current operation-log head — the watermark recorded on the projection
/// cursor (commitments reflect canonical state as of this op-log sequence).
fn op_log_head(conn: &Connection) -> Result<i64, DbError> {
    Ok(conn.query_row(
        "SELECT COALESCE(MAX(op_seq), 0) FROM operation_log",
        [],
        |r| r.get(0),
    )?)
}

/// Advance the commitments projection cursor and record the content checksum +
/// rebuild time, plus the authoritative drift-detection checksum in
/// `read_model_checksums` (mirrors `projection::set_cursor`).
fn set_cursor(conn: &Connection, op_seq: i64, content_checksum: u64) -> Result<(), DbError> {
    let now = Utc::now().to_rfc3339();
    let checksum = content_checksum as i64;
    conn.execute(
        "INSERT OR REPLACE INTO projection_cursors
            (read_model_name, last_applied_op_seq, last_rebuild_at, content_checksum)
         VALUES (?1, ?2, ?3, ?4)",
        params![PROJECTION_NAME, op_seq, now, checksum],
    )?;
    conn.execute(
        "INSERT OR REPLACE INTO read_model_checksums
            (read_model_name, current_checksum, computed_at)
         VALUES (?1, ?2, ?3)",
        params![PROJECTION_NAME, checksum, now],
    )?;
    Ok(())
}
