//! Connector-connection storage (personal-cfo-gglk, ADR 0060 §1).
//!
//! One `connector_connections` row per linked aggregator connection; the
//! `credential` column holds the provider access credential (SimpleFIN: the
//! access URL) — encrypted at rest by SQLCipher like every other column, and
//! stored in the vault deliberately so backup/restore round-trips the
//! connection. `connector_account_links` maps the provider's external
//! accounts (connector-core account keys) onto real accounts and carries the
//! per-account sync watermark.
//!
//! Configuration, not ledger mutations — like `settings` / `vault_metadata` /
//! `parser_runs`, everything here bypasses the `WriteCommand` bus.
//!
//! LEAK RULE: the credential never appears on the listing row type, never in
//! a log, and callers wrap it in `connector_core::Credential` semantics the
//! moment it leaves this module (the desktop layer does exactly that).

use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use uuid::Uuid;

use crate::{DbError, DbWorker};

/// Defense-in-depth floor for stored error strings (they surface in the UI
/// and Money Inbox payloads): strip control + bidi/zero-width format
/// characters and cap the length. Adapter strings arrive pre-sanitized; this
/// also covers kernel-authored messages (e.g. wrapped SQL errors).
fn sanitize_error(raw: &str) -> String {
    let forged = |c: &char| {
        c.is_control()
            || matches!(
                c,
                '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}'
            )
    };
    let cleaned: String = raw.chars().filter(|c| !forged(c)).take(300).collect();
    if raw.chars().count() > 300 {
        format!("{cleaned}…")
    } else {
        cleaned
    }
}

/// A connection row for listing — deliberately **without** the credential.
#[derive(Debug, Clone)]
pub struct ConnectorConnectionRow {
    pub id: Uuid,
    pub adapter_id: String,
    pub display_hint: Option<String>,
    pub last_synced_at: Option<String>,
    pub last_error: Option<String>,
    pub created_at: String,
}

/// One staged connector transaction: what [`crate::DbWorker::stage_sync_batch`]
/// returns per row, so the commit orchestration can pre-check certain
/// duplicates without re-reading the rows.
#[derive(Debug, Clone)]
pub struct StagedSyncTxn {
    pub staged_id: Uuid,
    pub txn_fingerprint: String,
    pub account_id: Uuid,
}

/// One external-account → real-account link, with its sync watermark.
#[derive(Debug, Clone)]
pub struct ConnectorLinkRow {
    pub connection_id: Uuid,
    /// The connector-core account key (connection-scoped provider id).
    pub external_id: String,
    pub external_name: Option<String>,
    /// The mapped real account; `None` until the user maps it.
    pub account_id: Option<Uuid>,
    /// Per-account since-watermark (`YYYY-MM-DD`), held back on
    /// retry-required provider errors.
    pub last_synced_on: Option<String>,
}

impl DbWorker {
    /// Create a connection. The caller supplies the id so the IPC layer can
    /// return it without a read-back.
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a write failure.
    pub fn create_connector_connection(
        &self,
        id: Uuid,
        adapter_id: &str,
        credential: &str,
        display_hint: Option<&str>,
    ) -> Result<(), DbError> {
        let now = Utc::now().to_rfc3339();
        let guard = self.lock();
        guard.conn.execute(
            "INSERT INTO connector_connections
                (id, adapter_id, credential, display_hint, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![id, adapter_id, credential, display_hint, now],
        )?;
        Ok(())
    }

    /// Every connection, credential omitted by construction.
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a read failure.
    pub fn connector_connections(&self) -> Result<Vec<ConnectorConnectionRow>, DbError> {
        let conn = self.read_connection()?;
        let mut stmt = conn.prepare(
            "SELECT id, adapter_id, display_hint, last_synced_at, last_error, created_at
               FROM connector_connections ORDER BY created_at, id",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(ConnectorConnectionRow {
                    id: r.get(0)?,
                    adapter_id: r.get(1)?,
                    display_hint: r.get(2)?,
                    last_synced_at: r.get(3)?,
                    last_error: r.get(4)?,
                    created_at: r.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Whether a connection row exists — WITHOUT selecting the credential
    /// column, so existence checks never materialize the secret. Doubles as
    /// the vault-identity witness for in-flight syncs: a switched vault has
    /// no such row, so phase-3 writes abort instead of bleeding cross-vault.
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a read failure.
    pub fn connector_connection_exists(&self, id: Uuid) -> Result<bool, DbError> {
        let conn = self.read_connection()?;
        let found = conn
            .query_row(
                "SELECT 1 FROM connector_connections WHERE id = ?1",
                params![id],
                |_| Ok(()),
            )
            .optional()?;
        Ok(found.is_some())
    }

    /// The provenance recorded for a sync batch: `(source_type, parser_name,
    /// parser_version)` — the adapter id + version the AC requires on
    /// source_batch/parser_run rows (personal-cfo-gglk).
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a read failure.
    pub fn source_batch_provenance(
        &self,
        batch_id: Uuid,
    ) -> Result<Option<(String, String, String)>, DbError> {
        let conn = self.read_connection()?;
        conn.query_row(
            "SELECT sb.source_type, pr.parser_name, pr.parser_version
               FROM source_batches sb
               JOIN parser_runs pr ON pr.source_batch_id = sb.id
              WHERE sb.id = ?1",
            params![batch_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(DbError::from)
    }

    /// The stored credential for one connection. Callers treat the value as a
    /// secret immediately (wrap, never log).
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a read failure.
    pub fn connector_credential(&self, id: Uuid) -> Result<Option<String>, DbError> {
        self.read_connection()?
            .query_row(
                "SELECT credential FROM connector_connections WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .optional()
            .map_err(DbError::from)
    }

    /// Forget a connection: its links first, then the row (no PRAGMA
    /// foreign-key reliance). Staged/committed data from past syncs is ledger
    /// history and is deliberately untouched.
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a write failure.
    pub fn delete_connector_connection(&self, id: Uuid) -> Result<(), DbError> {
        let guard = self.lock();
        let tx = guard.conn.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM connector_account_links WHERE connection_id = ?1",
            params![id],
        )?;
        tx.execute(
            "DELETE FROM connector_connections WHERE id = ?1",
            params![id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Upsert a discovered external account. A re-discovery refreshes the
    /// display name but never clobbers the user's mapping or the watermark.
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a write failure.
    pub fn upsert_connector_link(
        &self,
        connection_id: Uuid,
        external_id: &str,
        external_name: Option<&str>,
    ) -> Result<(), DbError> {
        let now = Utc::now().to_rfc3339();
        let guard = self.lock();
        guard.conn.execute(
            "INSERT INTO connector_account_links
                (connection_id, external_id, external_name, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?4)
             ON CONFLICT(connection_id, external_id) DO UPDATE SET
                 external_name = excluded.external_name,
                 updated_at = excluded.updated_at",
            params![connection_id, external_id, external_name, now],
        )?;
        Ok(())
    }

    /// Map (or unmap, with `None`) an external account onto a real account.
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a write failure.
    pub fn set_connector_link_account(
        &self,
        connection_id: Uuid,
        external_id: &str,
        account_id: Option<Uuid>,
    ) -> Result<(), DbError> {
        let guard = self.lock();
        let changed = guard.conn.execute(
            // Any (re)mapping clears the watermark: a newly-targeted account
            // must receive the full history on its next sync (a stale
            // watermark would silently skip it forever). Dedupe is scoped by
            // account, so the re-walk commits cleanly.
            "UPDATE connector_account_links
                SET account_id = ?3, last_synced_on = NULL, updated_at = ?4
              WHERE connection_id = ?1 AND external_id = ?2",
            params![
                connection_id,
                external_id,
                account_id,
                Utc::now().to_rfc3339()
            ],
        )?;
        if changed == 0 {
            return Err(DbError::InvalidCommand(
                "unknown connector account link".to_owned(),
            ));
        }
        Ok(())
    }

    /// Every link for a connection, mapping-first ordering for the UI.
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a read failure.
    pub fn connector_links(&self, connection_id: Uuid) -> Result<Vec<ConnectorLinkRow>, DbError> {
        let conn = self.read_connection()?;
        let mut stmt = conn.prepare(
            "SELECT connection_id, external_id, external_name, account_id, last_synced_on
               FROM connector_account_links
              WHERE connection_id = ?1
              ORDER BY external_name, external_id",
        )?;
        let rows = stmt
            .query_map(params![connection_id], |r| {
                Ok(ConnectorLinkRow {
                    connection_id: r.get(0)?,
                    external_id: r.get(1)?,
                    external_name: r.get(2)?,
                    account_id: r.get(3)?,
                    last_synced_on: r.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Record a sync attempt's outcome on the connection row. `error: None`
    /// clears a previous error (the connection is healthy again).
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a write failure.
    pub fn record_connector_sync(
        &self,
        connection_id: Uuid,
        error: Option<&str>,
    ) -> Result<(), DbError> {
        let now = Utc::now().to_rfc3339();
        let guard = self.lock();
        guard.conn.execute(
            "UPDATE connector_connections
                SET last_synced_at = ?2, last_error = ?3, updated_at = ?2
              WHERE id = ?1",
            params![connection_id, now, error.map(sanitize_error)],
        )?;
        Ok(())
    }

    /// Advance the per-account since-watermark to `synced_on` for exactly
    /// `advance_external_ids` (still requiring a mapping). The CALLER decides
    /// the set — the links that were mapped when the walk started, present in
    /// the provider's response, and not retry-held (personal-cfo-gglk review:
    /// re-deriving "mapped" here raced concurrent mapping edits, and absent
    /// accounts must keep their old watermark so their gap is re-walked).
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a write failure.
    pub fn advance_connector_watermarks(
        &self,
        connection_id: Uuid,
        synced_on: &str,
        advance_external_ids: &[String],
    ) -> Result<(), DbError> {
        let now = Utc::now().to_rfc3339();
        let guard = self.lock();
        let tx = guard.conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare(
                "UPDATE connector_account_links
                    SET last_synced_on = ?3, updated_at = ?4
                  WHERE connection_id = ?1 AND external_id = ?2
                    AND account_id IS NOT NULL",
            )?;
            for external_id in advance_external_ids {
                stmt.execute(params![connection_id, external_id, synced_on, now])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Clear a stale error WITHOUT stamping `last_synced_at`: a successful
    /// account-discovery pass proves the connection healthy but is not a sync
    /// (the badge stays "Never synced" and the unlock auto-sync keeps
    /// retrying discovery until an account is mapped).
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a write failure.
    pub fn clear_connector_error(&self, connection_id: Uuid) -> Result<(), DbError> {
        let guard = self.lock();
        guard.conn.execute(
            "UPDATE connector_connections
                SET last_error = NULL, updated_at = ?2
              WHERE id = ?1",
            params![connection_id, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    /// Record a sync FAILURE without stamping the attempt time: transient
    /// (network-class) failures must not consume the auto-sync debounce
    /// window — an offline unlock retries on the next opportunity.
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a write failure.
    pub fn record_connector_error(&self, connection_id: Uuid, error: &str) -> Result<(), DbError> {
        let guard = self.lock();
        guard.conn.execute(
            "UPDATE connector_connections
                SET last_error = ?2, updated_at = ?3
              WHERE id = ?1",
            params![
                connection_id,
                sanitize_error(error),
                Utc::now().to_rfc3339()
            ],
        )?;
        Ok(())
    }

    /// Persist the `parser_runs` provenance row for a connector sync batch:
    /// `parser_name` = adapter id, `parser_version` = adapter version — the
    /// same columns importer plugin runs record (personal-cfo-gglk, plan
    /// §8.1.3). Scratch metadata; bypasses the bus like `record_parser_run`.
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a write failure.
    pub fn record_connector_run(
        &self,
        batch_id: Uuid,
        adapter_id: &str,
        adapter_version: &str,
        records_out: usize,
    ) -> Result<(), DbError> {
        let now = Utc::now().to_rfc3339();
        let guard = self.lock();
        crate::ingestion::record_parser_run(
            &guard.conn,
            &crate::ingestion::NewParserRun {
                source_batch_id: batch_id,
                parser_name: adapter_id,
                parser_version: adapter_version,
                bytes_in: 0,
                records_out: i64::try_from(records_out).unwrap_or(i64::MAX),
                status: "ok",
                limit_hit: None,
                finished_at: Some(&now),
            },
        )?;
        Ok(())
    }
}
