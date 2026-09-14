//! R2 ingestion staging substrate (ADR 0008, personal-cfo-ihe).
//!
//! The low-level recording primitives over the eight staging tables every
//! importer / extractor / connector commits *through* — no source writes the
//! canonical ledger directly (ADR 0014 pipeline). A batch is parsed into
//! `source_records`, proposed as `staged_*` rows, deduped (`dedupe_decisions`),
//! and — by the commit pipeline (personal-cfo-cmx / eay / cu8, not yet built) —
//! promoted to ledger rows pinned back to their origin via `source_provenance_links`
//! (named to avoid collision with the baseline's op-log `provenance_links`, ADR 0011).
//!
//! `staged_*` rows are scratch space: [`discard_batch`] proves they are
//! TRUNCATE-safe (removing a batch touches no accounts / ledger / balances).
//!
//! Provenance is FK-strict at the application layer: [`link_provenance`] refuses
//! to pin a committed entity to a `source_record` that does not exist, because
//! `configure_conn` leaves SQLite's per-connection `foreign_keys` at its default
//! (off) — the declared `REFERENCES` document intent, this code enforces it
//! (ADR 0008 §4).
//!
//! Every primitive here is forward-built substrate: until the commit pipeline
//! wires real callers it has no non-test user, hence the module-level
//! `allow(dead_code)` (cf. `migrations::migrate_down`).
#![allow(dead_code)]

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::DbError;

// ---------------------------------------------------------------------------
// Insert inputs (borrowed; one struct per table to keep call sites readable and
// clear of `clippy::too_many_arguments`).
// ---------------------------------------------------------------------------

/// A new ingestion batch (one file import / sync). `status` starts at `parsing`.
#[derive(Debug, Clone)]
pub(crate) struct NewSourceBatch<'a> {
    pub source_type: &'a str,
    pub source_name: Option<&'a str>,
    /// Content fingerprint of the whole uploaded file (file-level dedupe). `None`
    /// for sources without a file (manual, connector sync).
    pub file_fingerprint: Option<&'a str>,
    pub parser_version: Option<&'a str>,
}

/// A parsed row / provider object. Keeps the fingerprint + extracted fields,
/// never the raw bytes (ADR 0014 §4 shred-after-parse).
#[derive(Debug, Clone)]
pub(crate) struct NewSourceRecord<'a> {
    pub source_batch_id: Uuid,
    pub external_id: Option<&'a str>,
    pub source_hash: &'a str,
    pub normalized_json: &'a str,
    pub parse_confidence_bps: Option<i64>,
}

/// One parser execution against a batch (ties to ADR 0022 bounds via `status` /
/// `limit_hit`). `started_at` is stamped on insert.
#[derive(Debug, Clone)]
pub(crate) struct NewParserRun<'a> {
    pub source_batch_id: Uuid,
    pub parser_name: &'a str,
    pub parser_version: &'a str,
    pub bytes_in: i64,
    pub records_out: i64,
    /// One of `ok` / `limit_exceeded` / `parse_error` / `timeout`.
    pub status: &'a str,
    pub limit_hit: Option<&'a str>,
    pub finished_at: Option<&'a str>,
}

/// A proposed transaction awaiting dedupe + commit. `dedupe_status` (`pending`)
/// and `commit_status` (`staged`) take their column defaults.
#[derive(Debug, Clone)]
pub(crate) struct NewStagedTransaction<'a> {
    pub source_record_id: Uuid,
    pub proposed_account_id: Option<Uuid>,
    pub posted_at: &'a str,
    /// The secondary transaction / authorization date the source carried, if any
    /// (ADR 0045). `None` for single-date sources. Descriptive — never dedupe/math.
    pub transaction_date: Option<&'a str>,
    pub amount_minor: i64,
    pub currency: &'a str,
    pub normalized_merchant: Option<&'a str>,
    pub description: Option<&'a str>,
    /// The source's own category string, if any (ADR 0045 §3) — matched to a real
    /// category at commit and recorded as an `import_alias` prefill. `None` when the
    /// source carries no category.
    pub imported_category: Option<&'a str>,
    /// date + amount + normalized merchant + account, the transaction-level
    /// dedupe key (ADR 0014 §3).
    pub txn_fingerprint: &'a str,
}

/// An external account observed in the source, to be matched to a real account.
#[derive(Debug, Clone)]
pub(crate) struct NewStagedAccount<'a> {
    pub source_batch_id: Uuid,
    pub external_name: Option<&'a str>,
    pub external_number_hash: Option<&'a str>,
    pub proposed_subtype: Option<&'a str>,
    pub matched_account_id: Option<Uuid>,
}

/// An observed ending balance. The connector sync path promotes it into
/// `balance_observations` immediately, in the staging transaction (ADR 0027
/// addendum 2026-09-02, yl53); the file-import commit path remains deferred
/// (imports flow through review gates, and an old export's balance needs an
/// age-cutoff decision first).
#[derive(Debug, Clone)]
pub(crate) struct NewStagedBalance<'a> {
    pub source_record_id: Uuid,
    pub account_ref: Option<Uuid>,
    pub observed_at: &'a str,
    pub balance_minor: i64,
    pub currency: &'a str,
}

/// A recorded dedupe outcome (replayable, never a silent drop — ADR 0014 §3).
#[derive(Debug, Clone)]
pub(crate) struct NewDedupeDecision<'a> {
    pub source_batch_id: Uuid,
    /// `file` or `transaction`.
    pub layer: &'a str,
    /// The staged transaction this decision is about (`None` for file-level).
    pub staged_transaction_id: Option<Uuid>,
    pub matched_entity_type: Option<&'a str>,
    pub matched_entity_id: Option<Uuid>,
    /// `committed` / `skipped` / `merged` / `flagged`.
    pub decision: &'a str,
    pub reason: &'a str,
}

/// A permanent link from a committed entity back to its `source_record`.
#[derive(Debug, Clone)]
pub(crate) struct NewProvenanceLink<'a> {
    /// `ledger_transaction` / `ledger_posting` / `account` / `balance_observation`.
    pub entity_type: &'a str,
    pub entity_id: Uuid,
    pub source_record_id: Uuid,
    /// `created_from` / `amended_by` / `inferred_from` / `confirmed_by` /
    /// `contradicted_by` / `superseded_by`.
    pub relationship: &'a str,
}

// ---------------------------------------------------------------------------
// Read models.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceBatch {
    pub id: Uuid,
    pub source_type: String,
    pub source_name: Option<String>,
    pub file_fingerprint: Option<String>,
    pub parser_version: Option<String>,
    pub status: String,
    pub staged_count: i64,
    pub committed_count: i64,
    pub skipped_count: i64,
    pub summary_json: Option<String>,
    pub imported_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StagedTransaction {
    pub id: Uuid,
    pub source_record_id: Uuid,
    pub proposed_account_id: Option<Uuid>,
    pub posted_at: String,
    /// The secondary transaction / authorization date (ADR 0045), `None` when absent.
    pub transaction_date: Option<String>,
    pub amount_minor: i64,
    pub currency: String,
    pub normalized_merchant: Option<String>,
    pub description: Option<String>,
    /// The source's own category string (ADR 0045 §3), `None` when absent.
    pub imported_category: Option<String>,
    pub txn_fingerprint: String,
    pub dedupe_status: String,
    pub commit_status: String,
    pub committed_transaction_id: Option<Uuid>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DedupeDecision {
    pub id: Uuid,
    pub source_batch_id: Uuid,
    pub layer: String,
    pub staged_transaction_id: Option<Uuid>,
    pub matched_entity_type: Option<String>,
    pub matched_entity_id: Option<Uuid>,
    pub decision: String,
    pub reason: String,
    pub decided_at: String,
}

// ---------------------------------------------------------------------------
// Writes.
// ---------------------------------------------------------------------------

/// Insert a `source_batches` row (`status = 'parsing'`) with the caller-minted
/// `id`, returning it. The id is minted by the command layer so the importer
/// knows the batch before attaching records (kernel command pattern).
pub(crate) fn create_source_batch(
    conn: &Connection,
    id: Uuid,
    batch: &NewSourceBatch,
) -> Result<Uuid, DbError> {
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO source_batches
            (id, source_type, source_name, file_fingerprint, parser_version,
             status, staged_count, committed_count, skipped_count, summary_json,
             imported_at, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'parsing', 0, 0, 0, NULL, NULL, ?6, ?6)",
        params![
            id,
            batch.source_type,
            batch.source_name,
            batch.file_fingerprint,
            batch.parser_version,
            now,
        ],
    )?;
    Ok(id)
}

/// Update a batch's lifecycle `status` and aggregate counts (`updated_at` is
/// re-stamped). Errors if the batch does not exist. The `status` token is
/// validated by the table CHECK.
pub(crate) fn update_batch_status(
    conn: &Connection,
    batch_id: Uuid,
    status: &str,
    staged: i64,
    committed: i64,
    skipped: i64,
) -> Result<(), DbError> {
    let n = conn.execute(
        "UPDATE source_batches
            SET status = ?2, staged_count = ?3, committed_count = ?4,
                skipped_count = ?5, updated_at = ?6
          WHERE id = ?1",
        params![
            batch_id,
            status,
            staged,
            committed,
            skipped,
            Utc::now().to_rfc3339()
        ],
    )?;
    if n == 0 {
        return Err(DbError::InvalidCommand(format!(
            "no source_batch {batch_id}"
        )));
    }
    Ok(())
}

/// Attach a `source_records` row under the caller-minted `id`, returning the
/// effective record id. **Content-hash dedupe (ADR 0008):** re-attaching the
/// same `source_hash` to the same batch is idempotent — it inserts no new row
/// and returns the existing record's id, so re-importing a file produces no
/// duplicate source_records.
pub(crate) fn insert_source_record(
    conn: &Connection,
    id: Uuid,
    rec: &NewSourceRecord,
) -> Result<Uuid, DbError> {
    if let Some(existing) = conn
        .query_row(
            "SELECT id FROM source_records WHERE source_batch_id = ?1 AND source_hash = ?2",
            params![rec.source_batch_id, rec.source_hash],
            |r| r.get::<_, Uuid>(0),
        )
        .optional()?
    {
        return Ok(existing);
    }
    conn.execute(
        "INSERT INTO source_records
            (id, source_batch_id, external_id, source_hash, normalized_json,
             parse_confidence_bps, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            id,
            rec.source_batch_id,
            rec.external_id,
            rec.source_hash,
            rec.normalized_json,
            rec.parse_confidence_bps,
            Utc::now().to_rfc3339(),
        ],
    )?;
    Ok(id)
}

/// Record a `parser_runs` row (`started_at` stamped now), returning its id.
pub(crate) fn record_parser_run(conn: &Connection, run: &NewParserRun) -> Result<Uuid, DbError> {
    let id = Uuid::now_v7();
    conn.execute(
        "INSERT INTO parser_runs
            (id, source_batch_id, parser_name, parser_version, bytes_in,
             records_out, status, limit_hit, started_at, finished_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            id,
            run.source_batch_id,
            run.parser_name,
            run.parser_version,
            run.bytes_in,
            run.records_out,
            run.status,
            run.limit_hit,
            Utc::now().to_rfc3339(),
            run.finished_at,
        ],
    )?;
    Ok(id)
}

/// Stage a proposed transaction, returning its id.
pub(crate) fn stage_transaction(
    conn: &Connection,
    txn: &NewStagedTransaction,
) -> Result<Uuid, DbError> {
    let id = Uuid::now_v7();
    conn.execute(
        "INSERT INTO staged_transactions
            (id, source_record_id, proposed_account_id, posted_at, transaction_date,
             amount_minor, currency, normalized_merchant, description, imported_category,
             txn_fingerprint, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            id,
            txn.source_record_id,
            txn.proposed_account_id,
            txn.posted_at,
            txn.transaction_date,
            txn.amount_minor,
            txn.currency,
            txn.normalized_merchant,
            txn.description,
            txn.imported_category,
            txn.txn_fingerprint,
            Utc::now().to_rfc3339(),
        ],
    )?;
    Ok(id)
}

/// Stage an external account, returning its id.
pub(crate) fn stage_account(conn: &Connection, acct: &NewStagedAccount) -> Result<Uuid, DbError> {
    let id = Uuid::now_v7();
    conn.execute(
        "INSERT INTO staged_accounts
            (id, source_batch_id, external_name, external_number_hash,
             proposed_subtype, matched_account_id, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            id,
            acct.source_batch_id,
            acct.external_name,
            acct.external_number_hash,
            acct.proposed_subtype,
            acct.matched_account_id,
            Utc::now().to_rfc3339(),
        ],
    )?;
    Ok(id)
}

/// Stage an observed balance, returning its id.
pub(crate) fn stage_balance(conn: &Connection, bal: &NewStagedBalance) -> Result<Uuid, DbError> {
    let id = Uuid::now_v7();
    conn.execute(
        "INSERT INTO staged_balances
            (id, source_record_id, account_ref, observed_at, balance_minor,
             currency, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            id,
            bal.source_record_id,
            bal.account_ref,
            bal.observed_at,
            bal.balance_minor,
            bal.currency,
            Utc::now().to_rfc3339(),
        ],
    )?;
    Ok(id)
}

/// Record a dedupe decision (`decided_at` stamped now), returning its id.
pub(crate) fn record_dedupe_decision(
    conn: &Connection,
    dec: &NewDedupeDecision,
) -> Result<Uuid, DbError> {
    let id = Uuid::now_v7();
    conn.execute(
        "INSERT INTO dedupe_decisions
            (id, source_batch_id, layer, staged_transaction_id, matched_entity_type,
             matched_entity_id, decision, reason, decided_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            id,
            dec.source_batch_id,
            dec.layer,
            dec.staged_transaction_id,
            dec.matched_entity_type,
            dec.matched_entity_id,
            dec.decision,
            dec.reason,
            Utc::now().to_rfc3339(),
        ],
    )?;
    Ok(id)
}

/// Pin a committed entity to its origin `source_record`. FK-strict at the
/// application layer (ADR 0008 §4): refuses a link to a missing source_record so
/// the audit chain never dangles.
pub(crate) fn link_provenance(
    conn: &Connection,
    link: &NewProvenanceLink,
) -> Result<Uuid, DbError> {
    let exists = conn
        .query_row(
            "SELECT 1 FROM source_records WHERE id = ?1",
            params![link.source_record_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !exists {
        return Err(DbError::InvalidCommand(format!(
            "provenance_link references missing source_record {}",
            link.source_record_id
        )));
    }
    let id = Uuid::now_v7();
    conn.execute(
        "INSERT INTO source_provenance_links
            (id, entity_type, entity_id, source_record_id, relationship, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            id,
            link.entity_type,
            link.entity_id,
            link.source_record_id,
            link.relationship,
            Utc::now().to_rfc3339(),
        ],
    )?;
    Ok(id)
}

/// Discard a batch and **all** its staged rows — atomically, child rows first.
/// TRUNCATE-safe: removes only staging data (ADR 0008 §1), never canonical
/// accounts / ledger / balances. Returns the number of rows removed.
pub(crate) fn discard_batch(conn: &Connection, batch_id: Uuid) -> Result<u64, DbError> {
    let tx = conn.unchecked_transaction()?;
    let mut removed: u64 = 0;
    removed += tx.execute(
        "DELETE FROM source_provenance_links WHERE source_record_id IN
            (SELECT id FROM source_records WHERE source_batch_id = ?1)",
        params![batch_id],
    )? as u64;
    removed += tx.execute(
        "DELETE FROM dedupe_decisions WHERE source_batch_id = ?1",
        params![batch_id],
    )? as u64;
    removed += tx.execute(
        "DELETE FROM staged_transactions WHERE source_record_id IN
            (SELECT id FROM source_records WHERE source_batch_id = ?1)",
        params![batch_id],
    )? as u64;
    removed += tx.execute(
        "DELETE FROM staged_balances WHERE source_record_id IN
            (SELECT id FROM source_records WHERE source_batch_id = ?1)",
        params![batch_id],
    )? as u64;
    removed += tx.execute(
        "DELETE FROM staged_accounts WHERE source_batch_id = ?1",
        params![batch_id],
    )? as u64;
    removed += tx.execute(
        "DELETE FROM parser_runs WHERE source_batch_id = ?1",
        params![batch_id],
    )? as u64;
    removed += tx.execute(
        "DELETE FROM source_records WHERE source_batch_id = ?1",
        params![batch_id],
    )? as u64;
    removed += tx.execute(
        "DELETE FROM source_batches WHERE id = ?1",
        params![batch_id],
    )? as u64;
    tx.commit()?;
    Ok(removed)
}

// ---------------------------------------------------------------------------
// Reads.
// ---------------------------------------------------------------------------

/// Read one batch by id (`None` if absent).
pub(crate) fn read_batch(
    conn: &Connection,
    batch_id: Uuid,
) -> Result<Option<SourceBatch>, DbError> {
    conn.query_row(
        "SELECT id, source_type, source_name, file_fingerprint, parser_version,
                status, staged_count, committed_count, skipped_count, summary_json,
                imported_at, created_at, updated_at
           FROM source_batches WHERE id = ?1",
        params![batch_id],
        |r| {
            Ok(SourceBatch {
                id: r.get(0)?,
                source_type: r.get(1)?,
                source_name: r.get(2)?,
                file_fingerprint: r.get(3)?,
                parser_version: r.get(4)?,
                status: r.get(5)?,
                staged_count: r.get(6)?,
                committed_count: r.get(7)?,
                skipped_count: r.get(8)?,
                summary_json: r.get(9)?,
                imported_at: r.get(10)?,
                created_at: r.get(11)?,
                updated_at: r.get(12)?,
            })
        },
    )
    .optional()
    .map_err(DbError::from)
}

/// All staged transactions for a batch (via their `source_records`), oldest first.
pub(crate) fn list_staged_for_batch(
    conn: &Connection,
    batch_id: Uuid,
) -> Result<Vec<StagedTransaction>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT t.id, t.source_record_id, t.proposed_account_id, t.posted_at,
                t.amount_minor, t.currency, t.normalized_merchant, t.description,
                t.txn_fingerprint, t.dedupe_status, t.commit_status,
                t.committed_transaction_id, t.created_at, t.transaction_date,
                t.imported_category
           FROM staged_transactions t
           JOIN source_records r ON r.id = t.source_record_id
          WHERE r.source_batch_id = ?1
          ORDER BY t.created_at, t.id",
    )?;
    let rows = stmt.query_map(params![batch_id], |r| {
        Ok(StagedTransaction {
            id: r.get(0)?,
            source_record_id: r.get(1)?,
            proposed_account_id: r.get(2)?,
            posted_at: r.get(3)?,
            amount_minor: r.get(4)?,
            currency: r.get(5)?,
            normalized_merchant: r.get(6)?,
            description: r.get(7)?,
            txn_fingerprint: r.get(8)?,
            dedupe_status: r.get(9)?,
            commit_status: r.get(10)?,
            committed_transaction_id: r.get(11)?,
            created_at: r.get(12)?,
            transaction_date: r.get(13)?,
            imported_category: r.get(14)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Read one staged transaction by id (`None` if absent).
pub(crate) fn read_staged_transaction(
    conn: &Connection,
    id: Uuid,
) -> Result<Option<StagedTransaction>, DbError> {
    conn.query_row(
        "SELECT id, source_record_id, proposed_account_id, posted_at, amount_minor,
                currency, normalized_merchant, description, txn_fingerprint,
                dedupe_status, commit_status, committed_transaction_id, created_at,
                transaction_date, imported_category
           FROM staged_transactions WHERE id = ?1",
        params![id],
        |r| {
            Ok(StagedTransaction {
                id: r.get(0)?,
                source_record_id: r.get(1)?,
                proposed_account_id: r.get(2)?,
                posted_at: r.get(3)?,
                amount_minor: r.get(4)?,
                currency: r.get(5)?,
                normalized_merchant: r.get(6)?,
                description: r.get(7)?,
                txn_fingerprint: r.get(8)?,
                dedupe_status: r.get(9)?,
                commit_status: r.get(10)?,
                committed_transaction_id: r.get(11)?,
                created_at: r.get(12)?,
                transaction_date: r.get(13)?,
                imported_category: r.get(14)?,
            })
        },
    )
    .optional()
    .map_err(DbError::from)
}

/// Whether `txn_fingerprint` already belongs to a *committed* staged transaction
/// other than `excluding`. The transaction-level dedupe matcher (ADR 0014 §3):
/// every import-committed transaction keeps its staged row, so the staging table
/// itself is the fingerprint index — no separate store needed.
///
/// A committed row whose ledger transaction has since been VOIDED no longer counts
/// (feedback 2026-07-03): the user deleted those transactions and re-importing the
/// same file must be able to restore them, not trip over their ghosts.
///
/// The match is scoped to `account` (the staged row's proposed account): the dedupe key
/// is date + amount + merchant + ACCOUNT, and bank ids (OFX `FITID`) are only unique per
/// account — the same movement landing in two different accounts is two transactions,
/// not a duplicate.
/// Cross-source duplicate heuristic (personal-cfo-tevp, ADR 0014 §3 addendum):
/// fingerprints are source-shaped (a CSV row hashes its description, a
/// connector row its provider id), so the exact-match layer is blind across
/// sources. This layer matches on the OBSERVABLE identity — same account,
/// same posted date, same signed amount — but only against work from a
/// DIFFERENT source type (or a manual entry): within one source, distinct
/// fingerprints mean the provider/importer itself vouches the rows are
/// distinct (two identical coffees in one sync are both real).
///
/// Returns the reason to flag with plus the matched ledger transaction (so
/// the Money Inbox review panel can show the counterpart), or `None`.
pub(crate) fn cross_source_duplicate_reason(
    conn: &Connection,
    account_id: Uuid,
    ledger_account_id: Uuid,
    amount_minor: i64,
    posted_date: &str,
    own_source_type: &str,
    excluding_staged: Uuid,
) -> Result<Option<(&'static str, Uuid)>, DbError> {
    // A committed staged row from a different source type.
    let staged_match: Option<Uuid> = conn
        .query_row(
            "SELECT st.committed_transaction_id FROM staged_transactions st
             JOIN source_records sr ON sr.id = st.source_record_id
             JOIN source_batches sb ON sb.id = sr.source_batch_id
             WHERE st.commit_status = 'committed' AND st.id != ?1
               AND st.proposed_account_id IS ?2
               AND st.amount_minor = ?3
               AND substr(st.posted_at, 1, 10) = ?4
               AND sb.source_type != ?5
               AND st.committed_transaction_id IS NOT NULL
               AND NOT EXISTS (
                 SELECT 1 FROM ledger_transactions lt
                  WHERE lt.id = st.committed_transaction_id
                    AND lt.voided_at IS NOT NULL
               )
             LIMIT 1",
            params![
                excluding_staged,
                account_id,
                amount_minor,
                posted_date,
                own_source_type
            ],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(txn) = staged_match {
        return Ok(Some((
            "same date and amount already recorded from another source",
            txn,
        )));
    }
    // A provenance-free transaction (a manual entry, transfer leg, or other
    // user-authored posting) on the same account/date/amount.
    let manual_match: Option<Uuid> = conn
        .query_row(
            "SELECT lt.id FROM ledger_postings lp
             JOIN ledger_transactions lt ON lt.id = lp.transaction_id
             WHERE lp.ledger_account_id = ?1
               AND lp.minor_units = ?2
               AND lp.posting_date = ?3
               AND lt.voided_at IS NULL
               AND NOT EXISTS (
                 SELECT 1 FROM source_provenance_links spl
                  WHERE spl.entity_type = 'ledger_transaction'
                    AND spl.entity_id = lt.id
               )
             LIMIT 1",
            params![ledger_account_id, amount_minor, posted_date],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(txn) = manual_match {
        return Ok(Some((
            "same date and amount already in this account's ledger",
            txn,
        )));
    }
    Ok(None)
}

/// Whether an exact fingerprint is already TRACKED at `account`: committed
/// (non-voided), or sitting flagged/skipped from an earlier batch. The
/// connector re-sync pre-check uses this so an unresolved (or explicitly
/// skipped) collision is not re-staged into a fresh Money Inbox item on
/// every rewind re-fetch (tevp review blocker).
pub(crate) fn fingerprint_already_tracked(
    conn: &Connection,
    txn_fingerprint: &str,
    account: Option<Uuid>,
    excluding: Uuid,
) -> Result<bool, DbError> {
    if fingerprint_already_committed(conn, txn_fingerprint, account, excluding)? {
        return Ok(true);
    }
    let pending = conn
        .query_row(
            "SELECT 1 FROM staged_transactions st
              WHERE st.txn_fingerprint = ?1 AND st.id != ?2
                AND st.proposed_account_id IS ?3
                AND st.commit_status IN ('flagged', 'skipped')",
            params![txn_fingerprint, excluding, account],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    Ok(pending)
}

pub(crate) fn fingerprint_already_committed(
    conn: &Connection,
    txn_fingerprint: &str,
    account: Option<Uuid>,
    excluding: Uuid,
) -> Result<bool, DbError> {
    let found = conn
        .query_row(
            "SELECT 1 FROM staged_transactions st
              WHERE st.txn_fingerprint = ?1 AND st.commit_status = 'committed' AND st.id != ?2
                AND st.proposed_account_id IS ?3
                AND NOT EXISTS (
                  SELECT 1 FROM ledger_transactions lt
                   WHERE lt.id = st.committed_transaction_id AND lt.voided_at IS NOT NULL
                )",
            params![txn_fingerprint, excluding, account],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    Ok(found)
}

/// Mark a staged transaction committed and link it to its ledger transaction.
pub(crate) fn mark_staged_committed(
    conn: &Connection,
    staged_id: Uuid,
    committed_transaction_id: Uuid,
) -> Result<(), DbError> {
    conn.execute(
        "UPDATE staged_transactions
            SET commit_status = 'committed', dedupe_status = 'unique',
                committed_transaction_id = ?2
          WHERE id = ?1",
        params![staged_id, committed_transaction_id],
    )?;
    Ok(())
}

/// Flag a staged transaction as a suspected duplicate — not committed, not dropped
/// (ADR 0014 §3). It surfaces in the Money Inbox for the user to skip or import.
pub(crate) fn flag_staged_duplicate(conn: &Connection, staged_id: Uuid) -> Result<(), DbError> {
    conn.execute(
        "UPDATE staged_transactions
            SET commit_status = 'flagged', dedupe_status = 'suspected_duplicate'
          WHERE id = ?1",
        params![staged_id],
    )?;
    Ok(())
}

/// Mark a staged transaction as skipped — the Money Inbox "skip" resolution
/// (ADR 0014 §7). No ledger write; the row leaves `flagged`, so the next inbox
/// rebuild drops it.
pub(crate) fn mark_staged_skipped(conn: &Connection, staged_id: Uuid) -> Result<(), DbError> {
    conn.execute(
        "UPDATE staged_transactions SET commit_status = 'skipped' WHERE id = ?1",
        params![staged_id],
    )?;
    Ok(())
}

/// Every dedupe decision recorded for a batch, oldest first (replayable).
pub(crate) fn list_dedupe_decisions_for_batch(
    conn: &Connection,
    batch_id: Uuid,
) -> Result<Vec<DedupeDecision>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, source_batch_id, layer, staged_transaction_id,
                matched_entity_type, matched_entity_id, decision, reason, decided_at
           FROM dedupe_decisions WHERE source_batch_id = ?1
          ORDER BY decided_at, id",
    )?;
    let rows = stmt.query_map(params![batch_id], |r| {
        Ok(DedupeDecision {
            id: r.get(0)?,
            source_batch_id: r.get(1)?,
            layer: r.get(2)?,
            staged_transaction_id: r.get(3)?,
            matched_entity_type: r.get(4)?,
            matched_entity_id: r.get(5)?,
            decision: r.get(6)?,
            reason: r.get(7)?,
            decided_at: r.get(8)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    /// An in-memory vault migrated to the current schema (v16 included).
    fn migrated() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::migrations::run_migrations(&mut conn).unwrap();
        conn
    }

    /// Seed one canonical `accounts` row (baseline data present at every version),
    /// so the TRUNCATE-safety test can prove a discard leaves it untouched.
    fn seed_account(conn: &Connection) {
        let id = [7u8; 16];
        let ledger = [0xABu8; 16];
        conn.execute(
            "INSERT INTO accounts
                (id, ledger_account_id, name, cashflow_role, normal_balance, currency)
             VALUES (?1, ?2, 'Checking', 'liquid_cash', 'debit', 'USD')",
            params![&id[..], &ledger[..]],
        )
        .unwrap();
    }

    fn count(conn: &Connection, table: &str) -> i64 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    fn integrity_ok(conn: &Connection) -> bool {
        let report: String = conn
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .unwrap();
        report == "ok"
    }

    #[test]
    fn migration_creates_all_eight_staging_tables() {
        let conn = migrated();
        for table in [
            "source_batches",
            "source_records",
            "parser_runs",
            "staged_transactions",
            "staged_accounts",
            "staged_balances",
            "dedupe_decisions",
            "source_provenance_links",
        ] {
            let found: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    params![table],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(found, 1, "table {table} should exist after migration v16");
        }
        assert!(integrity_ok(&conn));
    }

    /// A full batch can be staged and then discarded with no trace, and canonical
    /// `accounts` data is untouched — `staged_*` is scratch space (ADR 0008 §1).
    #[test]
    fn discarding_a_batch_is_truncate_safe() {
        let conn = migrated();
        seed_account(&conn);

        let batch = create_source_batch(
            &conn,
            Uuid::now_v7(),
            &NewSourceBatch {
                source_type: "csv",
                source_name: Some("statement.csv"),
                file_fingerprint: Some("sha256:abc"),
                parser_version: Some("csv-v1"),
            },
        )
        .unwrap();
        record_parser_run(
            &conn,
            &NewParserRun {
                source_batch_id: batch,
                parser_name: "generic-csv",
                parser_version: "1",
                bytes_in: 1024,
                records_out: 1,
                status: "ok",
                limit_hit: None,
                finished_at: Some("2026-06-24T00:00:01Z"),
            },
        )
        .unwrap();
        let record = insert_source_record(
            &conn,
            Uuid::now_v7(),
            &NewSourceRecord {
                source_batch_id: batch,
                external_id: Some("row-1"),
                source_hash: "sha256:row1",
                normalized_json: "{\"amount\":-1299}",
                parse_confidence_bps: Some(10_000),
            },
        )
        .unwrap();
        let staged = stage_transaction(
            &conn,
            &NewStagedTransaction {
                source_record_id: record,
                proposed_account_id: None,
                posted_at: "2026-06-20",
                transaction_date: None,
                amount_minor: -1299,
                currency: "USD",
                normalized_merchant: Some("coffee"),
                description: Some("CAFE"),
                imported_category: None,
                txn_fingerprint: "2026-06-20|-1299|coffee",
            },
        )
        .unwrap();
        stage_account(
            &conn,
            &NewStagedAccount {
                source_batch_id: batch,
                external_name: Some("Checking ****1234"),
                external_number_hash: Some("hash"),
                proposed_subtype: Some("checking"),
                matched_account_id: None,
            },
        )
        .unwrap();
        stage_balance(
            &conn,
            &NewStagedBalance {
                source_record_id: record,
                account_ref: None,
                observed_at: "2026-06-20",
                balance_minor: 500_000,
                currency: "USD",
            },
        )
        .unwrap();
        record_dedupe_decision(
            &conn,
            &NewDedupeDecision {
                source_batch_id: batch,
                layer: "transaction",
                staged_transaction_id: Some(staged),
                matched_entity_type: None,
                matched_entity_id: None,
                decision: "committed",
                reason: "unique",
            },
        )
        .unwrap();
        link_provenance(
            &conn,
            &NewProvenanceLink {
                entity_type: "ledger_transaction",
                entity_id: Uuid::now_v7(),
                source_record_id: record,
                relationship: "created_from",
            },
        )
        .unwrap();

        // Everything is staged.
        assert_eq!(list_staged_for_batch(&conn, batch).unwrap().len(), 1);
        for table in [
            "source_records",
            "parser_runs",
            "staged_transactions",
            "staged_accounts",
            "staged_balances",
            "dedupe_decisions",
            "source_provenance_links",
        ] {
            assert_eq!(count(&conn, table), 1, "{table} should have a staged row");
        }

        let removed = discard_batch(&conn, batch).unwrap();
        assert_eq!(removed, 8, "8 staging rows (incl. the batch) removed");

        // No staging trace remains…
        for table in [
            "source_batches",
            "source_records",
            "parser_runs",
            "staged_transactions",
            "staged_accounts",
            "staged_balances",
            "dedupe_decisions",
            "source_provenance_links",
        ] {
            assert_eq!(
                count(&conn, table),
                0,
                "{table} should be empty after discard"
            );
        }
        // …and canonical state is untouched.
        assert_eq!(
            count(&conn, "accounts"),
            1,
            "discard must not touch accounts"
        );
        assert!(integrity_ok(&conn));
        assert!(read_batch(&conn, batch).unwrap().is_none());
    }

    /// Both dedupe layers are recorded with their reason and replayable per batch.
    #[test]
    fn dedupe_decisions_are_replayable() {
        let conn = migrated();
        let batch = create_source_batch(
            &conn,
            Uuid::now_v7(),
            &NewSourceBatch {
                source_type: "csv",
                source_name: None,
                file_fingerprint: Some("sha256:file"),
                parser_version: None,
            },
        )
        .unwrap();

        record_dedupe_decision(
            &conn,
            &NewDedupeDecision {
                source_batch_id: batch,
                layer: "file",
                staged_transaction_id: None,
                matched_entity_type: Some("source_batch"),
                matched_entity_id: Some(Uuid::now_v7()),
                decision: "skipped",
                reason: "exact re-upload, already imported 2026-06-01",
            },
        )
        .unwrap();
        record_dedupe_decision(
            &conn,
            &NewDedupeDecision {
                source_batch_id: batch,
                layer: "transaction",
                staged_transaction_id: None,
                matched_entity_type: Some("ledger_transaction"),
                matched_entity_id: Some(Uuid::now_v7()),
                decision: "flagged",
                reason: "possible duplicate of committed txn",
            },
        )
        .unwrap();

        let decisions = list_dedupe_decisions_for_batch(&conn, batch).unwrap();
        assert_eq!(decisions.len(), 2);
        assert_eq!(decisions[0].layer, "file");
        assert_eq!(decisions[0].decision, "skipped");
        assert!(decisions[0].reason.contains("already imported"));
        assert_eq!(decisions[1].layer, "transaction");
        assert_eq!(decisions[1].decision, "flagged");
    }

    /// Provenance is FK-strict at the app layer: a link to a non-existent
    /// `source_record` is refused; a link to a real one succeeds (ADR 0008 §4).
    #[test]
    fn provenance_link_requires_existing_source_record() {
        let conn = migrated();
        let batch = create_source_batch(
            &conn,
            Uuid::now_v7(),
            &NewSourceBatch {
                source_type: "ofx",
                source_name: None,
                file_fingerprint: None,
                parser_version: None,
            },
        )
        .unwrap();
        let record = insert_source_record(
            &conn,
            Uuid::now_v7(),
            &NewSourceRecord {
                source_batch_id: batch,
                external_id: None,
                source_hash: "sha256:rec",
                normalized_json: "{}",
                parse_confidence_bps: None,
            },
        )
        .unwrap();

        // Dangling link → refused, nothing inserted.
        let dangling = link_provenance(
            &conn,
            &NewProvenanceLink {
                entity_type: "ledger_transaction",
                entity_id: Uuid::now_v7(),
                source_record_id: Uuid::now_v7(),
                relationship: "created_from",
            },
        );
        assert!(matches!(dangling, Err(DbError::InvalidCommand(_))));
        assert_eq!(count(&conn, "source_provenance_links"), 0);

        // Real source_record → linked.
        link_provenance(
            &conn,
            &NewProvenanceLink {
                entity_type: "ledger_transaction",
                entity_id: Uuid::now_v7(),
                source_record_id: record,
                relationship: "created_from",
            },
        )
        .unwrap();
        assert_eq!(count(&conn, "source_provenance_links"), 1);
    }

    /// The batch lifecycle status + counts move from `parsing` to a terminal
    /// state via `update_batch_status`.
    #[test]
    fn batch_status_and_counts_update() {
        let conn = migrated();
        let batch = create_source_batch(
            &conn,
            Uuid::now_v7(),
            &NewSourceBatch {
                source_type: "csv",
                source_name: None,
                file_fingerprint: None,
                parser_version: None,
            },
        )
        .unwrap();
        assert_eq!(read_batch(&conn, batch).unwrap().unwrap().status, "parsing");

        update_batch_status(&conn, batch, "committed", 10, 9, 1).unwrap();
        let b = read_batch(&conn, batch).unwrap().unwrap();
        assert_eq!(b.status, "committed");
        assert_eq!(b.staged_count, 10);
        assert_eq!(b.committed_count, 9);
        assert_eq!(b.skipped_count, 1);

        // Unknown batch → error.
        assert!(update_batch_status(&conn, Uuid::now_v7(), "committed", 0, 0, 0).is_err());
    }
}
