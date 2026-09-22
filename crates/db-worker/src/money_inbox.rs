//! Money Inbox read model (ADR 0014 §7; beads personal-cfo-dsq / -asqy).
//!
//! `money_inbox_read_model` is the single triage surface for the data-quality work
//! the import/commit pipeline could not decide automatically. Like `commitments`
//! and `transaction_display`, it is a **derived read model**: every item is
//! rebuildable from canonical state by a deterministic generator, so this module is
//! the only place that writes the table (command code never touches it directly).
//!
//! The projection is full-rebuild only (no incremental path): one item per
//! exception, ordered deterministically, with a `projection_cursors` row + a
//! `read_model_checksums` entry recorded on every rebuild.
//!
//! **Generators (one per item kind).**
//! - `imported_waiting_commit` — one item per `staged_transactions` row left
//!   `commit_status='flagged'` by the auto-commit-clean import path (a suspected
//!   duplicate / unmatched-account exception, ADR 0014 §2/§3). The item id and
//!   `surfaced_at` derive from the staged row (its id + `created_at`), so a rebuild
//!   is byte-identical. Resolution is intrinsic: import-anyway flips the row to
//!   `committed` and skip to `skipped`, so the next rebuild omits it.
//!
//! **Soft actions (ADR 0014 §7 / personal-cfo-3d3).** Snooze and dismiss don't change
//! canonical state, so they're recorded in `change_journal_entries` and re-applied to
//! the regenerated items by [`apply_change_journal`] on every rebuild — the user's
//! triage state survives. The snooze *expiry* is evaluated at read time
//! ([`crate::DbWorker::money_inbox_list`]) so this rebuild stays clock-independent.
//!
//! **Time-derived kinds** (stale balances, personal-cfo-r52x) are *not* generated here
//! — they're computed on read and merged in `money_inbox_list`, since their truth
//! depends on the wall clock and must stay out of the checksummed projection. The other
//! kinds (low-confidence categories, possible transfers, …) are deferred beads that add
//! a generator here.

use chrono::{DateTime, NaiveDate, Utc};
use rusqlite::{params, Connection};
use uuid::Uuid;

use crate::forecast_persist::json_str;
use crate::DbError;

const PROJECTION_NAME: &str = "money_inbox";

/// Default priority for the imported-waiting-commit kind. Lower = surfaced first
/// (the default sort is `priority ASC, surfaced_at ASC`); uncommitted money is
/// fairly urgent, so it sits near the top of the queue.
const PRIORITY_IMPORTED_WAITING_COMMIT: i64 = 20;

/// A single Money Inbox item — a triage row projected from canonical state. Never
/// hand-written. `payload_json` carries the kind-specific display detail (the read
/// model stays generic across all item kinds); the frontend parses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoneyInboxItem {
    /// Stable item id (for `imported_waiting_commit`, the staged transaction's id).
    pub item_id: Uuid,
    /// Item-kind token (CHECK'd in the schema; see the module list).
    pub item_kind: String,
    /// The canonical table this item points at (e.g. `staged_transactions`).
    pub target_table: String,
    /// The canonical row id this item points at.
    pub target_id: Uuid,
    /// Sort weight; lower surfaces first.
    pub priority: i64,
    /// When the item first appeared (a stable canonical timestamp).
    pub surfaced_at: String,
    /// Hidden-until timestamp set by a snooze action (`None` until snoozed).
    pub snoozed_until: Option<String>,
    /// Set when the user dismisses the item (`None` while active).
    pub dismissed_at: Option<String>,
    /// Set when the item is resolved (`None` while active).
    pub resolved_at: Option<String>,
    /// Kind-specific display detail as a JSON object string.
    pub payload_json: String,
}

/// `json_str` for an optional value; `None` becomes the JSON literal `null`.
fn json_opt(value: Option<&str>) -> String {
    value.map_or_else(|| "null".to_owned(), json_str)
}

/// Full rebuild in its own transaction: clear the read model and re-derive every
/// item from canonical state. Returns the number of items written.
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
/// commit), so the commit pipeline's apply transaction can refresh the inbox
/// atomically with the staged-row change that produced an exception. Deterministic:
/// same canonical state in, byte-identical items out.
///
/// # Errors
/// Returns [`DbError`] if a SQLite operation fails.
pub(crate) fn rebuild_in(conn: &Connection) -> Result<u64, DbError> {
    conn.execute("DELETE FROM money_inbox_read_model", [])?;
    let mut total = 0u64;
    total += generate_imported_waiting_commit(conn)?;
    // Re-apply the user's soft-action state (snooze/dismiss) recorded in the change
    // journal, so triage survives a rebuild (ADR 0014 §7, personal-cfo-3d3). Keyed
    // by stored values (the entry's `created_at` / payload date), so it stays
    // deterministic — the snooze *expiry* is evaluated at read time, not here.
    apply_change_journal(conn)?;
    set_cursor(conn, op_log_head(conn)?, checksum(conn)?)?;
    Ok(total)
}

/// Append a Money Inbox action to the audit journal (personal-cfo-3d3). `actor` is
/// the user; the kind-specific detail is `payload_json`.
fn record_change_journal(
    conn: &Connection,
    item_id: Uuid,
    entry_kind: &str,
    payload_json: &str,
) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO change_journal_entries
            (id, item_id, entry_kind, actor, payload_json, created_at)
         VALUES (?1, ?2, ?3, 'user', ?4, ?5)",
        params![
            Uuid::now_v7(),
            item_id,
            entry_kind,
            payload_json,
            Utc::now().to_rfc3339()
        ],
    )?;
    Ok(())
}

/// Record a **snooze** — hide the item until `until` (`YYYY-MM-DD`).
pub(crate) fn record_snooze(conn: &Connection, item_id: Uuid, until: &str) -> Result<(), DbError> {
    record_change_journal(
        conn,
        item_id,
        "snooze",
        &format!("{{\"until\":{}}}", json_str(until)),
    )
}

/// Record a **dismiss** with a typed `reason`.
pub(crate) fn record_dismiss(
    conn: &Connection,
    item_id: Uuid,
    reason: &str,
) -> Result<(), DbError> {
    record_change_journal(
        conn,
        item_id,
        "dismiss",
        &format!("{{\"reason\":{}}}", json_str(reason)),
    )
}

/// Re-apply soft-action state to the just-regenerated items: the latest dismiss
/// sets `dismissed_at`, the latest snooze sets `snoozed_until`. Iterating in
/// `created_at` order means a later action overrides an earlier one. Entries whose
/// item is no longer generated (e.g. it got committed) simply match no row.
fn apply_change_journal(conn: &Connection) -> Result<(), DbError> {
    let mut stmt = conn.prepare(
        "SELECT item_id, entry_kind, payload_json, created_at
         FROM change_journal_entries
         WHERE entry_kind IN ('snooze', 'dismiss')
         ORDER BY created_at, id",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, Uuid>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (item_id, kind, payload, created_at) in rows {
        match kind.as_str() {
            "dismiss" => {
                conn.execute(
                    "UPDATE money_inbox_read_model SET dismissed_at = ?2 WHERE item_id = ?1",
                    params![item_id, created_at],
                )?;
            }
            "snooze" => {
                let until: Option<String> = serde_json::from_str::<serde_json::Value>(&payload)
                    .ok()
                    .and_then(|v| v["until"].as_str().map(str::to_owned));
                if let Some(until) = until {
                    conn.execute(
                        "UPDATE money_inbox_read_model SET snoozed_until = ?2 WHERE item_id = ?1",
                        params![item_id, until],
                    )?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Generator: one item per flagged staged transaction (ADR 0014 §2/§3). Joins the
/// source batch (for the filename) + the flagged `dedupe_decisions` reason + the
/// suspected committed counterpart (same fingerprint, already committed).
fn generate_imported_waiting_commit(conn: &Connection) -> Result<u64, DbError> {
    let mut stmt = conn.prepare(
        "SELECT s.id, s.posted_at, s.amount_minor, s.currency,
                s.normalized_merchant, s.description, s.proposed_account_id,
                s.created_at, sb.source_name, a.name,
                (SELECT d.reason FROM dedupe_decisions d
                  WHERE d.staged_transaction_id = s.id AND d.decision = 'flagged'
                  ORDER BY d.decided_at DESC LIMIT 1),
                COALESCE(
                  (SELECT c.committed_transaction_id FROM staged_transactions c
                    WHERE c.txn_fingerprint = s.txn_fingerprint
                      AND c.commit_status = 'committed'
                    ORDER BY c.created_at LIMIT 1),
                  (SELECT d.matched_entity_id FROM dedupe_decisions d
                    WHERE d.staged_transaction_id = s.id AND d.decision = 'flagged'
                      AND d.matched_entity_type = 'ledger_transaction'
                    ORDER BY d.decided_at DESC LIMIT 1))
         FROM staged_transactions s
         JOIN source_records sr ON sr.id = s.source_record_id
         JOIN source_batches sb ON sb.id = sr.source_batch_id
         LEFT JOIN accounts a ON a.id = s.proposed_account_id
         WHERE s.commit_status = 'flagged'
         ORDER BY s.created_at, s.id",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, Uuid>(0)?,            // staged id (= item id, target id)
                r.get::<_, String>(1)?,          // posted_at
                r.get::<_, i64>(2)?,             // amount_minor
                r.get::<_, String>(3)?,          // currency
                r.get::<_, Option<String>>(4)?,  // normalized_merchant
                r.get::<_, Option<String>>(5)?,  // description
                r.get::<_, Option<Uuid>>(6)?,    // proposed_account_id
                r.get::<_, String>(7)?,          // created_at (= surfaced_at)
                r.get::<_, Option<String>>(8)?,  // source_name
                r.get::<_, Option<String>>(9)?,  // account name
                r.get::<_, Option<String>>(10)?, // dedupe reason
                r.get::<_, Option<Uuid>>(11)?,   // suspected committed txn id
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);

    let mut count = 0u64;
    for (
        staged_id,
        posted_at,
        amount_minor,
        currency,
        merchant,
        description,
        account_id,
        created_at,
        source_name,
        account_name,
        reason,
        counterpart_txn_id,
    ) in &rows
    {
        let account_uuid = account_id.map(|id| id.to_string());
        let counterpart = counterpart_txn_id.map(|id| id.to_string());
        // Hand-built JSON (db-worker keeps `serde_json` off the write path); fixed
        // field order keeps the projection byte-identical across rebuilds.
        let payload = format!(
            "{{\"merchant\":{},\"description\":{},\"amount_minor\":{},\"currency\":{},\
             \"posted_at\":{},\"account_id\":{},\"account_name\":{},\"source_name\":{},\
             \"dedupe_reason\":{},\"suspected_committed_txn_id\":{}}}",
            json_opt(merchant.as_deref()),
            json_opt(description.as_deref()),
            amount_minor,
            json_str(currency),
            json_str(posted_at),
            json_opt(account_uuid.as_deref()),
            json_opt(account_name.as_deref()),
            json_opt(source_name.as_deref()),
            json_opt(reason.as_deref()),
            json_opt(counterpart.as_deref()),
        );
        conn.execute(
            "INSERT INTO money_inbox_read_model (
                item_id, item_kind, target_table, target_id, priority,
                surfaced_at, snoozed_until, dismissed_at, resolved_at, payload_json
            ) VALUES (?1, 'imported_waiting_commit', 'staged_transactions', ?1, ?2,
                      ?3, NULL, NULL, NULL, ?4)",
            params![
                staged_id,
                PRIORITY_IMPORTED_WAITING_COMMIT,
                created_at,
                payload,
            ],
        )?;
        count += 1;
    }
    Ok(count)
}

/// Read the active inbox items (not dismissed, not resolved) in default sort order.
/// Snooze filtering arrives with the snooze action (a later slice); until then no
/// row carries a `snoozed_until`.
///
/// Sort weight for stale-balance items (personal-cfo-r52x) — below the
/// uncommitted-money items, which are more urgent.
const PRIORITY_STALE_BALANCE: i64 = 40;

/// Sort weight for connector-error items (personal-cfo-zfyo) — above stale
/// balances (a broken connection blocks freshness at the source) but below
/// uncommitted money.
const PRIORITY_CONNECTOR_ERROR: i64 = 30;

/// Sort weight for terminal durable-job failures (personal-cfo-ati): a failed
/// reliability job is actionable, but it must not outrank uncommitted money.
const PRIORITY_JOB_FAILURE: i64 = 25;

/// Derived-on-read generator for terminal local jobs.  Durable jobs are
/// canonical class-3 state, so this stays truthful without a separate queue
/// table or a projection write path.  Error text was sanitized at persistence;
/// this function still selects no payload/configuration column defensively.
pub(crate) fn failed_job_items(conn: &Connection) -> Result<Vec<MoneyInboxItem>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, kind, last_error, last_run_at, updated_at, attempt_count
           FROM durable_jobs
          WHERE state = 'failed' AND last_error IS NOT NULL
          ORDER BY updated_at, id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, Uuid>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, i64>(5)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, kind, reason, last_run_at, updated_at, attempts) = row?;
        let payload = format!(
            "{{\"job_kind\":{},\"reason\":{},\"last_run_at\":{},\"attempts\":{}}}",
            json_str(&kind),
            json_str(&reason),
            json_opt(last_run_at.as_deref()),
            attempts,
        );
        out.push(MoneyInboxItem {
            item_id: id,
            item_kind: "job_failure".to_owned(),
            target_table: "durable_jobs".to_owned(),
            target_id: id,
            priority: PRIORITY_JOB_FAILURE,
            surfaced_at: updated_at,
            snoozed_until: None,
            dismissed_at: None,
            resolved_at: None,
            payload_json: payload,
        });
    }
    Ok(out)
}

/// Derived-on-read generator (personal-cfo-zfyo, ADR 0014 §7 addendum
/// pattern): one item per connector connection whose last sync recorded an
/// error. Connector state is written outside the command bus (configuration,
/// like settings), so a materialized item would have no rebuild trigger —
/// derived-on-read is the only shape that stays truthful. Rate-limited syncs
/// are a HEALTHY state (ADR 0060 §5) and never write `last_error`, so they
/// never surface here. Resolution is intrinsic: the next successful sync
/// clears `last_error` (or forgetting the connection removes the row).
///
/// LEAK RULE: selects only non-secret columns — the credential is never read.
pub(crate) fn connector_error_items(conn: &Connection) -> Result<Vec<MoneyInboxItem>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, adapter_id, display_hint, last_synced_at, last_error, updated_at
           FROM connector_connections
          WHERE last_error IS NOT NULL
          ORDER BY updated_at, id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, uuid::Uuid>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, String>(5)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, adapter_id, display_hint, last_synced_at, last_error, updated_at) = row?;
        let payload = format!(
            "{{\"connection_id\":{},\"adapter_id\":{},\"display_hint\":{},\"last_synced_at\":{},\"last_error\":{}}}",
            json_str(&id.to_string()),
            json_str(&adapter_id),
            json_opt(display_hint.as_deref()),
            json_opt(last_synced_at.as_deref()),
            json_str(&last_error),
        );
        out.push(MoneyInboxItem {
            item_id: id,
            item_kind: "connector_error".to_owned(),
            target_table: "connector_connections".to_owned(),
            target_id: id,
            priority: PRIORITY_CONNECTOR_ERROR,
            surfaced_at: updated_at,
            snoozed_until: None,
            dismissed_at: None,
            resolved_at: None,
            payload_json: payload,
        });
    }
    Ok(out)
}

/// The freshness threshold (days) for an account role: liquid cash should be
/// confirmed often; debt and slower-moving assets less so (ADR 0014 §7 addendum).
fn stale_threshold_days(role: &str) -> i64 {
    match role {
        "liquid_cash" => 14,
        "credit_facility" | "loan_liability" => 30,
        _ => 60, // investment / real asset
    }
}

/// Time-derived generator (ADR 0014 §7 addendum, personal-cfo-r52x): one item per
/// active account whose latest balance observation — manual or connector-synced
/// (ADR 0027 addendum, yl53: a connector-refreshed account never goes stale) —
/// is older than the role's freshness threshold. Computed **on read** (it depends on `as_of`), so it
/// is deliberately **not** part of the materialized, checksummed read model —
/// [`crate::DbWorker::money_inbox_list`] merges it with [`read_rows`]. An account
/// that has never been asserted is left to the Forecast Readiness nudge, not
/// flagged here. Resolution is intrinsic: a fresh assertion or archiving the
/// account drops the item on the next read.
pub(crate) fn stale_balance_items(
    conn: &Connection,
    as_of: DateTime<Utc>,
) -> Result<Vec<MoneyInboxItem>, DbError> {
    let today = as_of.date_naive();
    let mut stmt = conn.prepare(
        "SELECT a.id, a.name, a.cashflow_role,
                (SELECT MAX(o.observed_at) FROM balance_observations o
                  WHERE o.account_id = a.id
                    AND o.source IN ('manual', 'connector_sync'))
         FROM accounts a
         WHERE a.active = 1
         ORDER BY a.name COLLATE NOCASE, a.id",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, Uuid>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut out = Vec::new();
    for (id, name, role, observed_opt) in rows {
        let Some(observed_str) = observed_opt else {
            continue; // never asserted → covered by Forecast Readiness, not here
        };
        let Ok(observed) = NaiveDate::parse_from_str(&observed_str, "%Y-%m-%d") else {
            continue;
        };
        let days = (today - observed).num_days();
        if days <= stale_threshold_days(&role) {
            continue;
        }
        let payload = format!(
            "{{\"account_id\":{},\"account_name\":{},\"role\":{},\"last_observed\":{},\"days_stale\":{days}}}",
            json_str(&id.to_string()),
            json_str(&name),
            json_str(&role),
            json_str(&observed_str),
        );
        out.push(MoneyInboxItem {
            item_id: id,
            item_kind: "stale_balance".to_owned(),
            target_table: "accounts".to_owned(),
            target_id: id,
            priority: PRIORITY_STALE_BALANCE,
            surfaced_at: observed_str,
            snoozed_until: None,
            dismissed_at: None,
            resolved_at: None,
            payload_json: payload,
        });
    }
    Ok(out)
}

/// Sort weight for unreviewed-transaction items — the lowest urgency (just needs a
/// look), below stale balances.
const PRIORITY_UNREVIEWED_TRANSACTION: i64 = 60;

/// Computed-on-read review items (ADR 0032 §3, personal-cfo-4d8.7): one per committed,
/// not-voided, UNREVIEWED transaction. Imported transactions default unreviewed; manual
/// ones default reviewed; the user override lives in `transaction_reviews`. Computed on
/// read like the stale-balance items (not materialized), so no projection / migration
/// change is needed — marking reviewed changes canonical state and the next read simply
/// omits the item.
pub(crate) fn unreviewed_transaction_items(
    conn: &Connection,
) -> Result<Vec<MoneyInboxItem>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT lt.id, lt.occurred_at, lp.minor_units, lp.currency,
                td.memo, td.counterparty, a.id, a.name
         FROM ledger_postings lp
         JOIN ledger_transactions lt ON lt.id = lp.transaction_id
         JOIN ledger_accounts la ON la.id = lp.ledger_account_id
         JOIN accounts a ON a.ledger_account_id = la.id
         LEFT JOIN transaction_details td ON td.transaction_id = lt.id
         LEFT JOIN transaction_reviews tr ON tr.transaction_id = lt.id
         WHERE lt.voided_at IS NULL
           AND COALESCE(tr.reviewed, CASE WHEN EXISTS(
                 SELECT 1 FROM source_provenance_links spl
                 WHERE spl.entity_type = 'ledger_transaction' AND spl.entity_id = lt.id
               ) THEN 0 ELSE 1 END) = 0
         ORDER BY lt.occurred_at DESC, lt.id",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, Uuid>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Uuid>(6)?,
                r.get::<_, String>(7)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut out = Vec::new();
    for (
        txn_id,
        occurred_at,
        amount_minor,
        currency,
        memo,
        counterparty,
        account_id,
        account_name,
    ) in rows
    {
        let payload = format!(
            "{{\"memo\":{},\"counterparty\":{},\"amount_minor\":{},\"currency\":{},\
             \"occurred_at\":{},\"account_id\":{},\"account_name\":{}}}",
            json_opt(memo.as_deref()),
            json_opt(counterparty.as_deref()),
            amount_minor,
            json_str(&currency),
            json_str(&occurred_at),
            json_str(&account_id.to_string()),
            json_str(&account_name),
        );
        out.push(MoneyInboxItem {
            item_id: txn_id,
            item_kind: "unreviewed_transaction".to_owned(),
            target_table: "ledger_transactions".to_owned(),
            target_id: txn_id,
            priority: PRIORITY_UNREVIEWED_TRANSACTION,
            surfaced_at: occurred_at,
            snoozed_until: None,
            dismissed_at: None,
            resolved_at: None,
            payload_json: payload,
        });
    }
    Ok(out)
}

/// Sort weight for low-confidence-category items (personal-cfo-j5ij/-uc95) — above the
/// generic unreviewed items (a marginal auto-categorization is more actionable than a
/// bare "needs review"), below stale balances.
const PRIORITY_LOW_CONFIDENCE_CATEGORY: i64 = 50;

/// Confidence threshold (basis points) under which an auto-categorization is queued for
/// review (ADR 0030 addendum, personal-cfo-uc95). Default 7000 = 70%. Merchant memory
/// applies at ≥6000, so the review band is the 6000–6999 slice.
pub const LOW_CONFIDENCE_CATEGORY_THRESHOLD_BPS: i64 = 7000;

/// The shared FROM + WHERE for the low-confidence-category queue: a committed, non-voided,
/// **unreviewed** transaction whose categorization is from an auto source (`rule`/`model`)
/// below `?1` confidence bps. The review predicate matches `unreviewed_transaction_items`
/// (imported = unreviewed by default unless `transaction_reviews` says otherwise).
const LOW_CONFIDENCE_FROM_WHERE: &str = "FROM ledger_postings lp
         JOIN ledger_transactions lt ON lt.id = lp.transaction_id
         JOIN ledger_accounts la ON la.id = lp.ledger_account_id
         JOIN accounts a ON a.ledger_account_id = la.id
         JOIN transaction_categorizations tc ON tc.transaction_id = lt.id
         LEFT JOIN transaction_details td ON td.transaction_id = lt.id
         LEFT JOIN transaction_reviews tr ON tr.transaction_id = lt.id
         WHERE lt.voided_at IS NULL
           AND tc.source IN ('rule', 'model')
           AND tc.confidence_bps < ?1
           AND COALESCE(tr.reviewed, CASE WHEN EXISTS(
                 SELECT 1 FROM source_provenance_links spl
                 WHERE spl.entity_type = 'ledger_transaction' AND spl.entity_id = lt.id
               ) THEN 0 ELSE 1 END) = 0";

/// Computed-on-read review items (ADR 0030 addendum, personal-cfo-j5ij/-uc95): one per
/// committed, unreviewed transaction auto-categorized below `threshold` confidence. Like the
/// unreviewed-transaction items, computed on read from canonical state (categorizations +
/// reviews), so accepting (mark reviewed) or recategorizing one drops it from the next read —
/// no projection / migration change. The payload carries the suggested category + confidence
/// so the card can show what the user is confirming.
pub(crate) fn low_confidence_category_items(
    conn: &Connection,
    threshold: i64,
) -> Result<Vec<MoneyInboxItem>, DbError> {
    let sql = format!(
        "SELECT lt.id, lt.occurred_at, lp.minor_units, lp.currency,
                td.memo, td.counterparty, a.name, tc.category_id, tc.confidence_bps
         {LOW_CONFIDENCE_FROM_WHERE}
         ORDER BY tc.confidence_bps ASC, lt.occurred_at DESC, lt.id"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params![threshold], |r| {
            Ok((
                r.get::<_, Uuid>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, Uuid>(7)?,
                r.get::<_, i64>(8)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut out = Vec::new();
    for (
        txn_id,
        occurred_at,
        amount_minor,
        currency,
        memo,
        counterparty,
        account_name,
        category_id,
        confidence_bps,
    ) in rows
    {
        let payload = format!(
            "{{\"memo\":{},\"counterparty\":{},\"amount_minor\":{},\"currency\":{},\
             \"occurred_at\":{},\"account_name\":{},\"category_id\":{},\"confidence_bps\":{}}}",
            json_opt(memo.as_deref()),
            json_opt(counterparty.as_deref()),
            amount_minor,
            json_str(&currency),
            json_str(&occurred_at),
            json_str(&account_name),
            json_str(&category_id.to_string()),
            confidence_bps,
        );
        out.push(MoneyInboxItem {
            item_id: txn_id,
            item_kind: "low_confidence_category".to_owned(),
            target_table: "ledger_transactions".to_owned(),
            target_id: txn_id,
            priority: PRIORITY_LOW_CONFIDENCE_CATEGORY,
            surfaced_at: occurred_at,
            snoozed_until: None,
            dismissed_at: None,
            resolved_at: None,
            payload_json: payload,
        });
    }
    Ok(out)
}

/// The transaction ids currently in the low-confidence-category queue (same predicate as
/// [`low_confidence_category_items`]). Backs the bulk "accept all" action, which marks each
/// reviewed via the command path.
///
/// # Errors
/// Returns [`DbError`] if the read fails.
pub(crate) fn low_confidence_category_transaction_ids(
    conn: &Connection,
    threshold: i64,
) -> Result<Vec<Uuid>, DbError> {
    let sql = format!("SELECT lt.id {LOW_CONFIDENCE_FROM_WHERE} ORDER BY lt.id");
    let mut stmt = conn.prepare(&sql)?;
    let ids = stmt
        .query_map(params![threshold], |r| r.get::<_, Uuid>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ids)
}

/// # Errors
/// Returns [`DbError`] if the read fails.
pub(crate) fn read_rows(conn: &Connection) -> Result<Vec<MoneyInboxItem>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT item_id, item_kind, target_table, target_id, priority,
                surfaced_at, snoozed_until, dismissed_at, resolved_at, payload_json
         FROM money_inbox_read_model
         WHERE dismissed_at IS NULL AND resolved_at IS NULL
         ORDER BY priority, surfaced_at, item_id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(MoneyInboxItem {
            item_id: r.get(0)?,
            item_kind: r.get(1)?,
            target_table: r.get(2)?,
            target_id: r.get(3)?,
            priority: r.get(4)?,
            surfaced_at: r.get(5)?,
            snoozed_until: r.get(6)?,
            dismissed_at: r.get(7)?,
            resolved_at: r.get(8)?,
            payload_json: r.get(9)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// A stable content checksum over the active items (FNV-1a over the
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
/// cursor (the inbox reflects canonical state as of this op-log sequence).
fn op_log_head(conn: &Connection) -> Result<i64, DbError> {
    Ok(conn.query_row(
        "SELECT COALESCE(MAX(op_seq), 0) FROM operation_log",
        [],
        |r| r.get(0),
    )?)
}

/// Advance the projection cursor and record the content checksum + rebuild time,
/// plus the authoritative drift-detection checksum in `read_model_checksums`
/// (mirrors `commitments::set_cursor` / `projection::set_cursor`).
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
