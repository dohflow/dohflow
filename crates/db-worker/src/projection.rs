//! Transaction-display read model (plan §9.4; beads personal-cfo-9x4 + lxj).
//!
//! The read model is **derived**: rebuildable from canonical tables
//! (`ledger_transactions` + `ledger_postings` + `accounts` + `operation_log`).
//! This module is the *only* place that writes
//! `transaction_display_rows_read_model` — command code never touches it
//! (CI-enforced: no writes to that table appear in `lib.rs`).
//!
//! [`project_transaction_display`] is a pure function (no I/O) so the mapping
//! logic is testable in isolation; the runner reads canonical rows, calls it,
//! and persists the result.

use chrono::{DateTime, Utc};
use core_ledger::{AccountId, TransactionId};
use core_money::Money;
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::{currency_from_code, DbError};

const PROJECTION_NAME: &str = "transaction_display";

/// How an account's category was assigned (defaults to uncategorized until the
/// categorization subsystem lands).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CategorySource {
    /// Not categorized.
    Uncategorized,
    /// Set by the user.
    Manual,
    /// Assigned by a rule.
    Rule,
    /// Assigned by a model.
    Model,
}

impl CategorySource {
    fn as_str(self) -> &'static str {
        match self {
            CategorySource::Uncategorized => "uncategorized",
            CategorySource::Manual => "manual",
            CategorySource::Rule => "rule",
            CategorySource::Model => "model",
        }
    }

    fn from_token(token: &str) -> Result<Self, DbError> {
        match token {
            "uncategorized" => Ok(CategorySource::Uncategorized),
            "manual" => Ok(CategorySource::Manual),
            "rule" => Ok(CategorySource::Rule),
            "model" => Ok(CategorySource::Model),
            other => Err(DbError::InvalidCommand(format!(
                "unknown category_source {other}"
            ))),
        }
    }
}

/// Review state of a transaction display row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReviewStatus {
    /// Needs human review.
    NeedsReview,
    /// Reviewed by the user.
    Reviewed,
    /// Auto-cleared (no review needed).
    AutoCleared,
}

impl ReviewStatus {
    fn as_str(self) -> &'static str {
        match self {
            ReviewStatus::NeedsReview => "needs_review",
            ReviewStatus::Reviewed => "reviewed",
            ReviewStatus::AutoCleared => "auto_cleared",
        }
    }

    fn from_token(token: &str) -> Result<Self, DbError> {
        match token {
            "needs_review" => Ok(ReviewStatus::NeedsReview),
            "reviewed" => Ok(ReviewStatus::Reviewed),
            "auto_cleared" => Ok(ReviewStatus::AutoCleared),
            other => Err(DbError::InvalidCommand(format!(
                "unknown review_status {other}"
            ))),
        }
    }
}

/// Canonical input to the projection: one user-account posting plus the op-log
/// sequence of the operation that created it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionDisplayInput {
    /// The ledger transaction this posting belongs to.
    pub transaction_id: TransactionId,
    /// The user account the posting moves.
    pub account_id: AccountId,
    /// When the transaction occurred.
    pub occurred_at: DateTime<Utc>,
    /// The signed posting amount.
    pub amount: Money,
    /// Op-log sequence of the originating operation.
    pub op_seq: i64,
}

/// A user-facing transaction row (plan §9.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionDisplayRow {
    /// The ledger transaction id.
    pub transaction_id: TransactionId,
    /// The user account.
    pub account_id: AccountId,
    /// When it occurred.
    pub occurred_at: DateTime<Utc>,
    /// The signed amount from this account's perspective.
    pub amount: Money,
    /// Normalized merchant identity (none until merchant normalization lands).
    pub merchant_identity_id: Option<String>,
    /// Primary category (none until categorization lands).
    pub primary_category_id: Option<String>,
    /// Category confidence in basis points.
    pub category_confidence_bps: i64,
    /// How the category was assigned.
    pub category_source: CategorySource,
    /// Linked recurring event, if any.
    pub recurring_event_id: Option<String>,
    /// Review state.
    pub review_status: ReviewStatus,
    /// Op-log sequence that last produced this row.
    pub last_projected_op_seq: i64,
}

/// Pure projection: map canonical posting inputs to display rows. No I/O. The
/// categorization fields default to empty until those subsystems exist; this is
/// the seam where they will be filled.
#[must_use]
pub fn project_transaction_display(
    inputs: &[TransactionDisplayInput],
) -> Vec<TransactionDisplayRow> {
    inputs
        .iter()
        .map(|input| TransactionDisplayRow {
            transaction_id: input.transaction_id,
            account_id: input.account_id,
            occurred_at: input.occurred_at,
            amount: input.amount,
            merchant_identity_id: None,
            primary_category_id: None,
            category_confidence_bps: 0,
            category_source: CategorySource::Uncategorized,
            recurring_event_id: None,
            review_status: ReviewStatus::NeedsReview,
            last_projected_op_seq: input.op_seq,
        })
        .collect()
}

/// Read canonical postings against user accounts as projection inputs, optionally
/// only those from operations after `since_op_seq`.
fn read_inputs(
    conn: &Connection,
    since_op_seq: Option<i64>,
) -> Result<Vec<TransactionDisplayInput>, DbError> {
    let base = "SELECT lt.id, a.id, lt.occurred_at, lp.minor_units, lp.currency, ol.op_seq
        FROM ledger_postings lp
        JOIN ledger_transactions lt ON lt.id = lp.transaction_id
        JOIN ledger_accounts la ON la.id = lp.ledger_account_id
        JOIN accounts a ON a.ledger_account_id = la.id
        JOIN operation_log ol ON ol.command_id = lt.operation_id";
    let order = " ORDER BY ol.op_seq, lt.id, a.id";

    let mut inputs = Vec::new();
    let cutoff = since_op_seq.unwrap_or(i64::MIN);
    let sql = format!("{base} WHERE ol.op_seq > ?1{order}");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([cutoff], |r| {
        Ok((
            r.get::<_, Uuid>(0)?,
            r.get::<_, Uuid>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, i64>(5)?,
        ))
    })?;
    for row in rows {
        let (txn_id, account_id, occurred_at, minor_units, currency_code, op_seq) = row?;
        inputs.push(TransactionDisplayInput {
            transaction_id: TransactionId::from_uuid(txn_id),
            account_id: AccountId::from_uuid(account_id),
            occurred_at: parse_timestamp(&occurred_at)?,
            amount: Money::new(minor_units, currency_from_code(&currency_code)?),
            op_seq,
        });
    }
    Ok(inputs)
}

fn parse_timestamp(value: &str) -> Result<DateTime<Utc>, DbError> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| DbError::InvalidCommand(e.to_string()))
}

fn write_row(conn: &Connection, row: &TransactionDisplayRow) -> Result<(), DbError> {
    conn.execute(
        "INSERT OR REPLACE INTO transaction_display_rows_read_model (
            transaction_id, account_id, occurred_at, minor_units, currency,
            merchant_identity_id, primary_category_id, category_confidence_bps,
            category_source, recurring_event_id, review_status, last_projected_op_seq
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            row.transaction_id.as_uuid(),
            row.account_id.as_uuid(),
            row.occurred_at.to_rfc3339(),
            row.amount.minor_units(),
            row.amount.currency().code(),
            row.merchant_identity_id,
            row.primary_category_id,
            row.category_confidence_bps,
            row.category_source.as_str(),
            row.recurring_event_id,
            row.review_status.as_str(),
            row.last_projected_op_seq,
        ],
    )?;
    Ok(())
}

fn max_op_seq(conn: &Connection) -> Result<i64, DbError> {
    let max: i64 = conn.query_row(
        "SELECT COALESCE(MAX(op_seq), 0) FROM operation_log",
        [],
        |r| r.get(0),
    )?;
    Ok(max)
}

/// Advance the projection cursor and record the content checksum + rebuild time
/// (personal-cfo-0s0). Also upserts the authoritative drift-detection checksum
/// into `read_model_checksums`. The `u64` checksum is stored as an `i64`
/// bit-cast (round-trips losslessly).
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

pub(crate) fn cursor(conn: &Connection) -> Result<i64, DbError> {
    let value: Option<i64> = conn
        .query_row(
            "SELECT last_applied_op_seq FROM projection_cursors WHERE read_model_name = ?1",
            [PROJECTION_NAME],
            |r| r.get(0),
        )
        .optional()?;
    Ok(value.unwrap_or(0))
}

/// Full rebuild: clear the read model and re-derive every row from canonical
/// tables. Returns the number of rows written.
pub(crate) fn rebuild(conn: &mut Connection) -> Result<u64, DbError> {
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM transaction_display_rows_read_model", [])?;
    let inputs = read_inputs(&tx, None)?;
    let rows = project_transaction_display(&inputs);
    for row in &rows {
        write_row(&tx, row)?;
    }
    let watermark = max_op_seq(&tx)?;
    set_cursor(&tx, watermark, checksum(&tx)?)?;
    tx.commit()?;
    Ok(rows.len() as u64)
}

/// Incremental projection: derive rows only for operations after the cursor and
/// advance the cursor to the current op-log head. Returns the number of rows
/// upserted.
pub(crate) fn incremental(conn: &mut Connection) -> Result<u64, DbError> {
    let tx = conn.transaction()?;
    let since = cursor(&tx)?;
    let inputs = read_inputs(&tx, Some(since))?;
    let rows = project_transaction_display(&inputs);
    for row in &rows {
        write_row(&tx, row)?;
    }
    let watermark = max_op_seq(&tx)?;
    set_cursor(&tx, watermark, checksum(&tx)?)?;
    tx.commit()?;
    Ok(rows.len() as u64)
}

/// Read all display rows in deterministic order.
pub(crate) fn read_rows(conn: &Connection) -> Result<Vec<TransactionDisplayRow>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT transaction_id, account_id, occurred_at, minor_units, currency,
                merchant_identity_id, primary_category_id, category_confidence_bps,
                category_source, recurring_event_id, review_status, last_projected_op_seq
         FROM transaction_display_rows_read_model
         ORDER BY transaction_id, account_id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, Uuid>(0)?,
            r.get::<_, Uuid>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, Option<String>>(5)?,
            r.get::<_, Option<String>>(6)?,
            r.get::<_, i64>(7)?,
            r.get::<_, String>(8)?,
            r.get::<_, Option<String>>(9)?,
            r.get::<_, String>(10)?,
            r.get::<_, i64>(11)?,
        ))
    })?;

    let mut out = Vec::new();
    for row in rows {
        let r = row?;
        out.push(TransactionDisplayRow {
            transaction_id: TransactionId::from_uuid(r.0),
            account_id: AccountId::from_uuid(r.1),
            occurred_at: parse_timestamp(&r.2)?,
            amount: Money::new(r.3, currency_from_code(&r.4)?),
            merchant_identity_id: r.5,
            primary_category_id: r.6,
            category_confidence_bps: r.7,
            category_source: CategorySource::from_token(&r.8)?,
            recurring_event_id: r.9,
            review_status: ReviewStatus::from_token(&r.10)?,
            last_projected_op_seq: r.11,
        });
    }
    Ok(out)
}

/// A stable content checksum over the read-model rows (FNV-1a over the
/// deterministically-ordered rows). Used to prove rebuild == incremental.
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

#[cfg(test)]
mod tests {
    use super::*;
    use core_money::Currency;

    #[test]
    fn pure_projection_defaults_categorization_fields() {
        let input = TransactionDisplayInput {
            transaction_id: TransactionId::new(),
            account_id: AccountId::new(),
            occurred_at: DateTime::from_timestamp(0, 0).unwrap(),
            amount: Money::new(500, Currency::Usd),
            op_seq: 7,
        };
        let rows = project_transaction_display(std::slice::from_ref(&input));
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.amount, Money::new(500, Currency::Usd));
        assert_eq!(row.category_source, CategorySource::Uncategorized);
        assert_eq!(row.review_status, ReviewStatus::NeedsReview);
        assert_eq!(row.category_confidence_bps, 0);
        assert_eq!(row.last_projected_op_seq, 7);
        assert!(row.merchant_identity_id.is_none());
    }
}
