//! Connector-connection storage (personal-cfo-gglk, ADR 0060 §1).
//!
//! One `connector_connections` row per linked aggregator connection: its IDENTITY (provider,
//! display hint, created/updated, the `deleted_at` tombstone), class 1. The provider access
//! credential (SimpleFIN: the access URL), the last refresh time and the last error live in the
//! class-3 `connector_connection_secrets` row of THIS device (personal-cfo-07u, migration 60),
//! encrypted at rest by SQLCipher like every other column and stored in the vault deliberately so
//! backup/restore round-trips the connection on this device. `connector_account_links` maps the
//! provider's external accounts (connector-core account keys) onto real accounts (class 1); the
//! per-account sync watermark is this device's own cursor in `connector_link_cursors` (class 3).
//!
//! A connection whose identity exists but has no secret row on this device (it was created on
//! another device) has no credential here: it is listed with `has_secret = false`, never
//! refreshes, and entering a token ([`DbWorker::set_connector_credential`]) inserts only the secret
//! row, never a second identity or mapping. Deleting a connection is a TOMBSTONE of the identity
//! (the fourth soft delete after SYNC-2b's three): a hard delete replicated to another device
//! would leave that device's class-3 rows pointing at a vanished identity.
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

use crate::{ActorType, CommandMeta, DbError, DbWorker, WriteCommand};

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
    /// This device's last refresh time (from its secret row; `None` without one).
    pub last_synced_at: Option<String>,
    /// This device's last refresh error (from its secret row; `None` without one).
    pub last_error: Option<String>,
    /// Whether THIS device holds a credential for the connection. `false` means the identity
    /// arrived from another device and the user must enter a token here; such a connection never
    /// refreshes.
    pub has_secret: bool,
    /// The connection's re-authentication notice as stored JSON (migration 58,
    /// `connector_attention`; personal-cfo-pxi.22), `None` when there is none. The desktop layer
    /// writes and parses it; nothing else interprets it.
    pub attention: Option<String>,
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

/// A connector link that feeds a real account, with the connection it belongs
/// to (personal-cfo-6evt: the mapping guard names the existing feed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectorFeed {
    pub connection_id: Uuid,
    /// The connection's provider (`simplefin`, `lunchflow`, …).
    pub adapter_id: String,
    /// The provider's own label for the connection, if it gave one.
    pub display_hint: Option<String>,
    pub external_id: String,
    pub external_name: Option<String>,
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
    /// The provider account's ISO 4217 currency as the provider reported it;
    /// `None` = not known yet (a link saved before migration 54, or a
    /// provider that stated none). Never assumed to be the base currency.
    pub currency: Option<String>,
}

/// The longest `connector_account_links.external_id`, in UTF-8 bytes. The link's primary key is
/// `(connection_id, external_id)`, and the canonical row encoding caps a primary key at 256
/// bytes (vault-format.md C.1.5): a uuid is a tag byte and 16 bytes (17) and a text key is a tag
/// byte, a `u32` length and the bytes (5 + len), so 256 - 17 - 5 = 234. A longer provider account
/// id is rejected with a typed error, never truncated (an adapter's account key is part of the
/// identity, ADR 0074 Addendum B). A `column_kinds` test pins this against the encoder.
pub const MAX_LINK_EXTERNAL_ID_BYTES: usize = 234;

/// The audit event recorded when a device removes the token of a connection that was deleted.
pub const CONNECTOR_SECRET_REMOVED_EVENT: &str = "connector.token_removed_connection_deleted";

/// Scrub and remove THIS device's rows for a connection: the credential is overwritten with an
/// empty value before the row is deleted, then the cursors and the notice go. The class-1 identity
/// and mapping are untouched.
pub(crate) fn scrub_local_rows(tx: &rusqlite::Transaction<'_>, id: Uuid) -> Result<bool, DbError> {
    let had_secret: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM connector_connection_secrets WHERE connection_id = ?1)",
        params![id],
        |r| r.get(0),
    )?;
    tx.execute(
        "UPDATE connector_connection_secrets SET credential = '' WHERE connection_id = ?1",
        params![id],
    )?;
    tx.execute(
        "DELETE FROM connector_connection_secrets WHERE connection_id = ?1",
        params![id],
    )?;
    tx.execute(
        "DELETE FROM connector_link_cursors WHERE connection_id = ?1",
        params![id],
    )?;
    tx.execute(
        "DELETE FROM connector_attention WHERE connection_id = ?1",
        params![id],
    )?;
    Ok(had_secret)
}

/// The metadata of a command this device issues for the connector's class-1 writes: a fresh
/// command and idempotency key, `issued_at` read once here at the command boundary.
fn local_meta(actor: ActorType, actor_id: &str) -> CommandMeta {
    CommandMeta {
        command_id: Uuid::now_v7(),
        correlation_id: Uuid::now_v7(),
        causation_id: None,
        actor_type: actor,
        actor_id: actor_id.to_owned(),
        idempotency_key: crate::new_idempotency_key(),
        issued_at: crate::IssuedAt::now(),
    }
}

impl DbWorker {
    /// Create a connection: its IDENTITY through the command bus (class 1, so it replicates),
    /// then this device's credential (class 3). The caller supplies the id so the IPC layer can
    /// return it without a read-back. A crash between the two leaves an identity without a token,
    /// which lists as needing one.
    ///
    /// # Errors
    /// [`DbError`] from the command or the credential write.
    pub fn create_connector_connection(
        &self,
        id: Uuid,
        adapter_id: &str,
        credential: &str,
        display_hint: Option<&str>,
    ) -> Result<(), DbError> {
        self.create_connector_identity(id, adapter_id, display_hint)?;
        self.set_connector_credential(id, credential)
    }

    /// Create only the connection IDENTITY (`CreateConnectorConnection`, class 1): what a replica
    /// gets when another device\'s connection is applied. It has no secret row here, lists with
    /// `has_secret = false`, never refreshes, and waits for the user to enter a token on this
    /// device ([`Self::set_connector_credential`]).
    ///
    /// # Errors
    /// [`DbError::InvalidCommand`] for an invalid adapter id or an id that belongs to another
    /// adapter; [`DbError::Sqlite`] on a write failure.
    pub fn create_connector_identity(
        &self,
        id: Uuid,
        adapter_id: &str,
        display_hint: Option<&str>,
    ) -> Result<(), DbError> {
        self.dispatch(
            local_meta(ActorType::User, "user"),
            WriteCommand::CreateConnectorConnection {
                id,
                adapter_id: adapter_id.to_owned(),
                display_hint: display_hint.map(str::to_owned),
            },
        )?;
        Ok(())
    }

    /// Enter (or replace) THIS device's credential for an existing connection: inserts only the
    /// local secret row, never a second identity or mapping. A connection created on another
    /// device has an identity here and no secret until this is called. A tombstoned or unknown
    /// connection is refused. A replaced credential clears the previous error and keeps the last
    /// refresh time.
    ///
    /// # Errors
    /// [`DbError::InvalidCommand`] for an unknown or deleted connection; [`DbError::Sqlite`] on a
    /// write failure.
    pub fn set_connector_credential(&self, id: Uuid, credential: &str) -> Result<(), DbError> {
        let now = Utc::now().to_rfc3339();
        let guard = self.lock();
        let tx = guard.conn.unchecked_transaction()?;
        let live: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM connector_connections
                             WHERE id = ?1 AND deleted_at IS NULL)",
            params![id],
            |r| r.get(0),
        )?;
        if !live {
            return Err(DbError::InvalidCommand(
                "unknown connector connection".to_owned(),
            ));
        }
        tx.execute(
            "INSERT INTO connector_connection_secrets (connection_id, credential, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT (connection_id) DO UPDATE
                SET credential = excluded.credential, last_error = NULL,
                    updated_at = excluded.updated_at",
            params![id, credential, now],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Every connection, credential omitted by construction.
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a read failure.
    pub fn connector_connections(&self) -> Result<Vec<ConnectorConnectionRow>, DbError> {
        let conn = self.read_connection()?;
        let mut stmt = conn.prepare(
            "SELECT c.id, c.adapter_id, c.display_hint, s.last_synced_at, s.last_error,
                    a.attention, c.created_at, s.connection_id IS NOT NULL
               FROM connector_connections c
               LEFT JOIN connector_connection_secrets s ON s.connection_id = c.id
               LEFT JOIN connector_attention a ON a.connection_id = c.id
              WHERE c.deleted_at IS NULL
              ORDER BY c.created_at, c.id",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(ConnectorConnectionRow {
                    id: r.get(0)?,
                    adapter_id: r.get(1)?,
                    display_hint: r.get(2)?,
                    last_synced_at: r.get(3)?,
                    last_error: r.get(4)?,
                    attention: r.get(5)?,
                    created_at: r.get(6)?,
                    has_secret: r.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Whether a live (not tombstoned) connection exists — WITHOUT selecting the credential,
    /// so existence checks never materialize the secret. Doubles as
    /// the vault-identity witness for in-flight syncs: a switched vault has
    /// no such row, so phase-3 writes abort instead of bleeding cross-vault.
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a read failure.
    pub fn connector_connection_exists(&self, id: Uuid) -> Result<bool, DbError> {
        let conn = self.read_connection()?;
        let found = conn
            .query_row(
                "SELECT 1 FROM connector_connections WHERE id = ?1 AND deleted_at IS NULL",
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

    /// This device's stored credential for one live connection, `None` when it has none (the
    /// connection came from another device, or was deleted). Callers treat the value as a secret
    /// immediately (wrap, never log).
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a read failure.
    pub fn connector_credential(&self, id: Uuid) -> Result<Option<String>, DbError> {
        self.read_connection()?
            .query_row(
                "SELECT s.credential
                   FROM connector_connection_secrets s
                   JOIN connector_connections c ON c.id = s.connection_id
                  WHERE s.connection_id = ?1 AND c.deleted_at IS NULL",
                params![id],
                |r| r.get(0),
            )
            .optional()
            .map_err(DbError::from)
    }

    /// Forget a connection (`DeleteConnectorConnection`): TOMBSTONE its identity (`deleted_at`)
    /// and remove this device\'s local rows for it (credential scrubbed first, cursors, notice).
    /// The mapping rows stay but are ignored (every reader joins the live identity), so a Sync
    /// replica never sees a vanished identity. Staged/committed data from past syncs is ledger
    /// history and is deliberately untouched. Deleting an unknown or already-deleted connection is
    /// a no-op.
    ///
    /// # Errors
    /// [`DbError`] from the command.
    pub fn delete_connector_connection(&self, id: Uuid) -> Result<(), DbError> {
        self.dispatch(
            local_meta(ActorType::User, "user"),
            WriteCommand::DeleteConnectorConnection { id },
        )?;
        Ok(())
    }

    /// The device-local cleanup AFTER a rebase swap: for every connection whose identity was
    /// tombstoned (here or, after a Sync, on another device), scrub the credential and remove this
    /// device's secret, cursors and notice, and record an audit event ("token removed:
    /// connection deleted"). Returns how many connections it cleaned. Idempotent.
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a write failure.
    pub fn scrub_tombstoned_connector_secrets(&self) -> Result<u32, DbError> {
        let guard = self.lock();
        let tx = guard.conn.unchecked_transaction()?;
        let ids: Vec<(Uuid, bool)> = {
            // Every tombstoned connection that still has ANY local row, not only those with a
            // secret: a stray cursor or notice left under a tombstone is cleaned too.
            let mut stmt = tx.prepare(
                "SELECT c.id,
                        EXISTS (SELECT 1 FROM connector_connection_secrets s
                                 WHERE s.connection_id = c.id)
                   FROM connector_connections c
                  WHERE c.deleted_at IS NOT NULL
                    AND (EXISTS (SELECT 1 FROM connector_connection_secrets s
                                  WHERE s.connection_id = c.id)
                      OR EXISTS (SELECT 1 FROM connector_link_cursors k
                                  WHERE k.connection_id = c.id)
                      OR EXISTS (SELECT 1 FROM connector_attention a
                                  WHERE a.connection_id = c.id))
                  ORDER BY c.id",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, Uuid>(0)?, r.get::<_, bool>(1)?)))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        let now = Utc::now().to_rfc3339();
        for (id, had_secret) in &ids {
            scrub_local_rows(&tx, *id)?;
            // The audit event records a removed TOKEN; a stray cursor or notice is cleaned
            // without one. A system event, not a command: it gets its own ids.
            if *had_secret {
                let event = Uuid::now_v7();
                tx.execute(
                    "INSERT INTO audit_events (id, event_type, command_id, correlation_id, created_at)
                     VALUES (?1, ?2, ?3, ?3, ?4)",
                    params![Uuid::now_v7(), CONNECTOR_SECRET_REMOVED_EVENT, event, now],
                )?;
            }
        }
        tx.commit()?;
        Ok(u32::try_from(ids.len()).unwrap_or(u32::MAX))
    }

    /// Upsert a discovered external account (`UpsertConnectorLink`). A re-discovery refreshes the
    /// display name and records the provider\'s currency when it states one (the backfill for
    /// links saved before migration 54), but never clobbers the user\'s mapping, the watermark, or
    /// a known currency with an unknown one. A discovery that would change nothing is not
    /// dispatched (every refresh would otherwise log and sync a no-op), and neither is one for a
    /// connection that is gone: an offline refresh racing a delete writes nothing.
    ///
    /// # Errors
    /// [`DbError::ReplicatedKeyTooLong`] for a key over the replicated width,
    /// [`DbError::InvalidCommand`] for a currency that is not an uppercase three-letter code,
    /// [`DbError::Sqlite`] on a write failure.
    pub fn upsert_connector_link(
        &self,
        connection_id: Uuid,
        external_id: &str,
        external_name: Option<&str>,
        currency: Option<&str>,
    ) -> Result<(), DbError> {
        if external_id.len() > MAX_LINK_EXTERNAL_ID_BYTES {
            return Err(DbError::ReplicatedKeyTooLong {
                table: "connector_account_links",
            });
        }
        let existing = {
            let conn = self.read_connection()?;
            let live: Option<bool> = conn
                .query_row(
                    "SELECT deleted_at IS NULL FROM connector_connections WHERE id = ?1",
                    params![connection_id],
                    |r| r.get(0),
                )
                .optional()?;
            if live != Some(true) {
                return Ok(());
            }
            conn.query_row(
                "SELECT external_name, currency FROM connector_account_links
                  WHERE connection_id = ?1 AND external_id = ?2",
                params![connection_id, external_id],
                |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?,
                        r.get::<_, Option<String>>(1)?,
                    ))
                },
            )
            .optional()?
        };
        if let Some((name, known_currency)) = &existing {
            if name.as_deref() == external_name
                && (currency.is_none() || known_currency.as_deref() == currency)
            {
                return Ok(());
            }
        }
        self.dispatch(
            local_meta(ActorType::Importer, "connector"),
            WriteCommand::UpsertConnectorLink {
                connection_id,
                external_id: external_id.to_owned(),
                external_name: external_name.map(str::to_owned),
                currency: currency.map(str::to_owned),
            },
        )?;
        Ok(())
    }

    /// The recorded currency of one link: `Ok(None)` when the link doesn't
    /// exist, `Ok(Some(None))` when its currency isn't known yet.
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a read failure.
    pub fn connector_link_currency(
        &self,
        connection_id: Uuid,
        external_id: &str,
    ) -> Result<Option<Option<String>>, DbError> {
        let conn = self.read_connection()?;
        let currency = conn
            .query_row(
                "SELECT l.currency
                   FROM connector_account_links l
                   JOIN connector_connections c ON c.id = l.connection_id AND c.deleted_at IS NULL
                  WHERE l.connection_id = ?1 AND l.external_id = ?2",
                params![connection_id, external_id],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()?;
        Ok(currency)
    }

    /// Map (or unmap, with `None`) an external account onto a real account
    /// (`SetConnectorLinkAccount`). Any (re)mapping clears this device\'s watermark: a
    /// newly-targeted account must receive the full history on its next sync (a stale watermark
    /// would silently skip it forever). Dedupe is scoped by account, so the re-walk commits
    /// cleanly.
    ///
    /// # Errors
    /// [`DbError::InvalidCommand`] for an unknown connection or link; [`DbError::Sqlite`] on a
    /// write failure (an account that does not exist).
    pub fn set_connector_link_account(
        &self,
        connection_id: Uuid,
        external_id: &str,
        account_id: Option<Uuid>,
    ) -> Result<(), DbError> {
        self.dispatch(
            local_meta(ActorType::User, "user"),
            WriteCommand::SetConnectorLinkAccount {
                connection_id,
                external_id: external_id.to_owned(),
                account_id: account_id.map(core_ledger::AccountId::from_uuid),
            },
        )?;
        Ok(())
    }

    /// Every link for a connection, mapping-first ordering for the UI.
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a read failure.
    pub fn connector_links(&self, connection_id: Uuid) -> Result<Vec<ConnectorLinkRow>, DbError> {
        let conn = self.read_connection()?;
        let mut stmt = conn.prepare(
            "SELECT l.connection_id, l.external_id, l.external_name, l.account_id, k.last_synced_on,
                    l.currency
               FROM connector_account_links l
               JOIN connector_connections c ON c.id = l.connection_id AND c.deleted_at IS NULL
               LEFT JOIN connector_link_cursors k
                      ON k.connection_id = l.connection_id AND k.external_id = l.external_id
              WHERE l.connection_id = ?1
              ORDER BY l.external_name, l.external_id",
        )?;
        let rows = stmt
            .query_map(params![connection_id], |r| {
                Ok(ConnectorLinkRow {
                    connection_id: r.get(0)?,
                    external_id: r.get(1)?,
                    external_name: r.get(2)?,
                    account_id: r.get(3)?,
                    last_synced_on: r.get(4)?,
                    currency: r.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Every connector link mapped onto `account_id`, across all connections
    /// (personal-cfo-6evt, ADR 0014 §3 addendum 2026-10-04: one account has one
    /// feed by default, so mapping a second one is warned about first).
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a read failure.
    pub fn connector_feeds_for_account(
        &self,
        account_id: Uuid,
    ) -> Result<Vec<ConnectorFeed>, DbError> {
        let conn = self.read_connection()?;
        let mut stmt = conn.prepare(
            "SELECT l.connection_id, c.adapter_id, c.display_hint, l.external_id, l.external_name
               FROM connector_account_links l
               JOIN connector_connections c ON c.id = l.connection_id AND c.deleted_at IS NULL
              WHERE l.account_id = ?1
              ORDER BY c.created_at, c.id, l.external_name, l.external_id",
        )?;
        let rows = stmt
            .query_map(params![account_id], |r| {
                Ok(ConnectorFeed {
                    connection_id: r.get(0)?,
                    adapter_id: r.get(1)?,
                    display_hint: r.get(2)?,
                    external_id: r.get(3)?,
                    external_name: r.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Record a sync attempt's outcome on this device's secret row. `error: None` clears a
    /// previous error (the connection is healthy again). A connection with no secret row here is
    /// a no-op.
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
            "UPDATE connector_connection_secrets
                SET last_synced_at = ?2, last_error = ?3, updated_at = ?2
              WHERE connection_id = ?1",
            params![connection_id, now, error.map(sanitize_error)],
        )?;
        Ok(())
    }

    /// Record (or replace) the connection's re-authentication notice (migration 58,
    /// `connector_attention`; personal-cfo-pxi.22). `attention_json` is the desktop layer's own
    /// small JSON object; a connection that no longer exists is a no-op (a deletion raced the
    /// refresh), never an error and never an orphan row.
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a write failure (including a value that is not valid JSON).
    pub fn set_connector_attention(
        &self,
        connection_id: Uuid,
        attention_json: &str,
    ) -> Result<(), DbError> {
        let now = Utc::now().to_rfc3339();
        let guard = self.lock();
        let tx = guard.conn.unchecked_transaction()?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM connector_connections
                             WHERE id = ?1 AND deleted_at IS NULL)",
            params![connection_id],
            |r| r.get(0),
        )?;
        if exists {
            tx.execute(
                "INSERT INTO connector_attention (connection_id, attention, updated_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT (connection_id) DO UPDATE
                    SET attention = excluded.attention, updated_at = excluded.updated_at",
                params![connection_id, attention_json, now],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Remove the connection's re-authentication notice (the next refresh was healthy). Nothing
    /// to remove is not an error.
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a write failure.
    pub fn clear_connector_attention(&self, connection_id: Uuid) -> Result<(), DbError> {
        let guard = self.lock();
        guard.conn.execute(
            "DELETE FROM connector_attention WHERE connection_id = ?1",
            params![connection_id],
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
                "INSERT INTO connector_link_cursors
                    (connection_id, external_id, last_synced_on, updated_at)
                 SELECT l.connection_id, l.external_id, ?3, ?4
                   FROM connector_account_links l
                   JOIN connector_connections c ON c.id = l.connection_id AND c.deleted_at IS NULL
                  WHERE l.connection_id = ?1 AND l.external_id = ?2
                    AND l.account_id IS NOT NULL
                 ON CONFLICT (connection_id, external_id) DO UPDATE
                    SET last_synced_on = excluded.last_synced_on,
                        updated_at = excluded.updated_at",
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
            "UPDATE connector_connection_secrets
                SET last_error = NULL, updated_at = ?2
              WHERE connection_id = ?1",
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
            "UPDATE connector_connection_secrets
                SET last_error = ?2, updated_at = ?3
              WHERE connection_id = ?1",
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

#[cfg(test)]
mod tests {
    use super::*;

    const CANARY: &str = "https://canary-user:canary-pass@bridge.invalid/simplefin";

    fn worker() -> (tempfile::TempDir, DbWorker) {
        let dir = tempfile::tempdir().unwrap();
        let worker = DbWorker::open(dir.path().join("t.vault"), "correct horse battery").unwrap();
        (dir, worker)
    }

    fn count(worker: &DbWorker, table: &str) -> i64 {
        worker
            .read_connection()
            .unwrap()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    /// Another device deleted the connection: its tombstone reaches this device as a plain
    /// `deleted_at` on the identity (a replicated class-1 write), leaving this device's secret,
    /// cursor and notice rows behind. The post-swap cleanup scrubs and removes them and audits
    /// it; the identity and mapping are untouched; running it again does nothing.
    #[test]
    fn a_connection_tombstoned_on_another_device_has_its_local_secret_scrubbed_and_audited() {
        let (_dir, worker) = worker();
        let gone = Uuid::now_v7();
        let kept = Uuid::now_v7();
        for id in [gone, kept] {
            worker
                .create_connector_connection(id, "simplefin", CANARY, None)
                .unwrap();
            worker
                .upsert_connector_link(id, "acct-1", None, None)
                .unwrap();
            worker
                .set_connector_attention(id, r#"{"kind":"relink_connection"}"#)
                .unwrap();
        }
        // The replicated tombstone (written by apply, not by this device's own delete).
        worker
            .lock()
            .conn
            .execute(
                "UPDATE connector_connections SET deleted_at = '2026-10-26T00:00:00Z' WHERE id = ?1",
                params![gone],
            )
            .unwrap();
        assert_eq!(count(&worker, "connector_connection_secrets"), 2);

        assert_eq!(worker.scrub_tombstoned_connector_secrets().unwrap(), 1);

        assert_eq!(worker.connector_credential(gone).unwrap(), None);
        assert_eq!(
            worker.connector_credential(kept).unwrap().as_deref(),
            Some(CANARY)
        );
        assert_eq!(count(&worker, "connector_connection_secrets"), 1);
        assert_eq!(count(&worker, "connector_attention"), 1);
        assert_eq!(
            count(&worker, "connector_connections"),
            2,
            "the identity stays as a tombstone"
        );
        assert_eq!(
            count(&worker, "connector_account_links"),
            2,
            "the mapping is untouched"
        );
        assert_eq!(
            worker
                .audit_event_count(CONNECTOR_SECRET_REMOVED_EVENT)
                .unwrap(),
            1
        );
        // Idempotent.
        assert_eq!(worker.scrub_tombstoned_connector_secrets().unwrap(), 0);
        assert_eq!(
            worker
                .audit_event_count(CONNECTOR_SECRET_REMOVED_EVENT)
                .unwrap(),
            1
        );
    }

    /// A refresh that races a delete advances a cursor after the tombstone landed: nothing is
    /// written under a tombstoned identity, and if a stray cursor or notice is somehow left
    /// there (written before a replicated tombstone arrived), the cleanup removes it even though
    /// no secret row exists, without recording a token removal.
    #[test]
    fn nothing_is_written_under_a_tombstone_and_a_stray_cursor_or_notice_is_cleaned() {
        let (_dir, worker) = worker();
        let id = Uuid::now_v7();
        worker
            .create_connector_connection(id, "simplefin", CANARY, None)
            .unwrap();
        worker
            .upsert_connector_link(id, "acct-1", None, None)
            .unwrap();
        // Map the link directly (no ledger account needed for this check).
        worker
            .lock()
            .conn
            .execute(
                "UPDATE connector_account_links SET account_id = NULL WHERE connection_id = ?1",
                params![id],
            )
            .unwrap();
        worker.delete_connector_connection(id).unwrap();

        // A late advance writes nothing (the link is not even mapped, and the identity is gone).
        worker
            .advance_connector_watermarks(id, "2026-10-27", &["acct-1".to_owned()])
            .unwrap();
        assert_eq!(count(&worker, "connector_link_cursors"), 0);
        assert_eq!(worker.connector_link_currency(id, "acct-1").unwrap(), None);

        // A stray cursor and notice with no secret row (planted as a replicated tombstone landing
        // after this device's writes would leave them): the cleanup removes both.
        worker
            .lock()
            .conn
            .execute_batch(&format!(
                "INSERT INTO connector_link_cursors (connection_id, external_id, last_synced_on, updated_at)
                     VALUES (x'{hex}', 'acct-1', '2026-10-27', '2026-10-27T00:00:00Z');
                 INSERT INTO connector_attention (connection_id, attention, updated_at)
                     VALUES (x'{hex}', '{{}}', '2026-10-27T00:00:00Z');",
                hex = id.as_bytes().iter().map(|b| format!("{b:02x}")).collect::<String>()
            ))
            .unwrap();
        assert_eq!(count(&worker, "connector_link_cursors"), 1);
        assert_eq!(count(&worker, "connector_connection_secrets"), 0);
        assert_eq!(worker.scrub_tombstoned_connector_secrets().unwrap(), 1);
        assert_eq!(count(&worker, "connector_link_cursors"), 0);
        assert_eq!(count(&worker, "connector_attention"), 0);
        assert_eq!(
            worker
                .audit_event_count(CONNECTOR_SECRET_REMOVED_EVENT)
                .unwrap(),
            0,
            "no token was removed, so no token-removal event"
        );
        assert_eq!(worker.scrub_tombstoned_connector_secrets().unwrap(), 0);
    }

    /// The audit event records the removal, not the credential or the connection's name.
    #[test]
    fn the_removal_audit_event_carries_no_secret() {
        let (_dir, worker) = worker();
        let id = Uuid::now_v7();
        worker
            .create_connector_connection(id, "simplefin", CANARY, Some("My secret bank"))
            .unwrap();
        worker.delete_connector_connection(id).unwrap();
        worker
            .lock()
            .conn
            .execute(
                "INSERT INTO connector_connection_secrets (connection_id, credential, updated_at)
                 VALUES (?1, ?2, '2026-10-26T00:00:00Z')",
                params![id, CANARY],
            )
            .unwrap();
        assert_eq!(worker.scrub_tombstoned_connector_secrets().unwrap(), 1);
        let dump: String = worker
            .read_connection()
            .unwrap()
            .query_row(
                "SELECT group_concat(event_type || hex(command_id) || hex(correlation_id) || created_at)
                   FROM audit_events",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            !dump.contains("canary") && !dump.contains("secret bank"),
            "{dump}"
        );
    }
}
