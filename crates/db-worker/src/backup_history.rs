//! Device-local receipts for verified backup exports (personal-cfo-8qh).
//!
//! Destinations and errors are kept only inside the SQLCipher-protected vault.
//! The recorded path is evidence for retention, never authority by itself: the
//! Finance Kernel re-opens the package and verifies its manifest before a file
//! can be removed.

use rusqlite::{params, OptionalExtension};
use std::fmt;
use std::path::Path;
use uuid::Uuid;

use crate::{DbError, DbWorker};

/// Whether a verified backup was explicitly requested or scheduled locally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupHistoryKind {
    /// Created by the user's Back up now / Export backup action.
    Manual,
    /// Created by the local durable scheduled-backup job.
    Scheduled,
}

impl BackupHistoryKind {
    fn as_token(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Scheduled => "scheduled",
        }
    }

    fn parse(token: &str) -> Result<Self, DbError> {
        match token {
            "manual" => Ok(Self::Manual),
            "scheduled" => Ok(Self::Scheduled),
            other => Err(DbError::SelfTestFailed(format!(
                "invalid backup history kind {other:?}"
            ))),
        }
    }
}

/// A local receipt for one attempted backup file.
#[derive(Clone, PartialEq, Eq)]
pub struct BackupHistoryEntry {
    /// The backup manifest's stable UUID.
    pub backup_id: Uuid,
    /// The vault whose in-memory key verified this package.
    pub vault_id: Uuid,
    /// RFC 3339 UTC creation time from the manifest.
    pub created_at: String,
    /// Manual or scheduled export.
    pub kind: BackupHistoryKind,
    /// Full local path to the package; never sent to a service.
    pub destination: String,
    /// Container format version.
    pub format_version: u16,
    /// File size in bytes, or zero if no file was created.
    pub size_bytes: u64,
    /// True only after the just-written file was reopened and verified.
    pub verified: bool,
    /// Path-free reason when export or verification failed.
    pub error: Option<String>,
}

/// Paths and error text can reveal a user's folder names, so debug output is
/// deliberately limited to non-sensitive receipt metadata.
impl fmt::Debug for BackupHistoryEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BackupHistoryEntry")
            .field("backup_id", &self.backup_id)
            .field("vault_id", &self.vault_id)
            .field("created_at", &self.created_at)
            .field("kind", &self.kind)
            .field("destination", &"[REDACTED]")
            .field("format_version", &self.format_version)
            .field("size_bytes", &self.size_bytes)
            .field("verified", &self.verified)
            .field("error", &self.error.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

impl DbWorker {
    /// Insert one local backup receipt after the export/verification attempt.
    pub fn record_backup_history(&self, entry: &BackupHistoryEntry) -> Result<(), DbError> {
        chrono::DateTime::parse_from_rfc3339(&entry.created_at).map_err(|_| {
            DbError::InvalidCommand("backup creation time must be RFC 3339".to_owned())
        })?;
        if !Path::new(&entry.destination).is_absolute()
            || entry.destination.trim().is_empty()
            || entry.destination.len() > 4096
            || entry.destination.chars().any(char::is_control)
        {
            return Err(DbError::InvalidCommand(
                "backup destination is invalid".to_owned(),
            ));
        }
        if entry.format_version != 2 {
            return Err(DbError::InvalidCommand(
                "new backup history must use format version 2".to_owned(),
            ));
        }
        let size_bytes = i64::try_from(entry.size_bytes)
            .map_err(|_| DbError::InvalidCommand("backup size is out of range".to_owned()))?;
        let error = entry.error.as_deref().map(sanitize_history_error);
        if entry.verified && error.is_some() || !entry.verified && error.is_none() {
            return Err(DbError::InvalidCommand(
                "backup verification and error fields disagree".to_owned(),
            ));
        }

        let guard = self.lock();
        let stored_vault_id: Option<Uuid> = guard
            .conn
            .query_row(
                "SELECT vault_id FROM vault_metadata WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if stored_vault_id != Some(entry.vault_id) {
            return Err(DbError::InvalidCommand(
                "backup history belongs to a different vault".to_owned(),
            ));
        }
        guard.conn.execute(
            "INSERT INTO backup_history (
                backup_id, vault_id, created_at, kind, destination,
                format_version, size_bytes, verified, error
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                entry.backup_id,
                entry.vault_id,
                entry.created_at,
                entry.kind.as_token(),
                entry.destination,
                i64::from(entry.format_version),
                size_bytes,
                if entry.verified { 1i64 } else { 0i64 },
                error,
            ],
        )?;
        Ok(())
    }

    /// Receipts for one vault, newest first. The owning Kernel supplies the
    /// current vault id so a stale or mismatched row cannot seed the nudge.
    pub fn backup_history(&self, vault_id: Uuid) -> Result<Vec<BackupHistoryEntry>, DbError> {
        let conn = self.read_connection()?;
        let mut stmt = conn.prepare(
            "SELECT backup_id, vault_id, created_at, kind, destination,
                    format_version, size_bytes, verified, error
               FROM backup_history
              WHERE vault_id = ?1
              ORDER BY created_at DESC, backup_id DESC",
        )?;
        let rows = stmt.query_map(params![vault_id], |row| {
            let kind: String = row.get(3)?;
            let format_version: i64 = row.get(5)?;
            let size_bytes: i64 = row.get(6)?;
            let verified: i64 = row.get(7)?;
            Ok((
                row.get::<_, Uuid>(0)?,
                row.get::<_, Uuid>(1)?,
                row.get::<_, String>(2)?,
                kind,
                row.get::<_, String>(4)?,
                format_version,
                size_bytes,
                verified,
                row.get::<_, Option<String>>(8)?,
            ))
        })?;

        rows.map(|row| {
            let (
                backup_id,
                vault_id,
                created_at,
                kind,
                destination,
                format_version,
                size_bytes,
                verified,
                error,
            ) = row?;
            let format_version = u16::try_from(format_version)
                .map_err(|_| DbError::SelfTestFailed("invalid backup format version".to_owned()))?;
            let size_bytes = u64::try_from(size_bytes)
                .map_err(|_| DbError::SelfTestFailed("invalid backup size".to_owned()))?;
            let verified = match verified {
                0 => false,
                1 => true,
                _ => {
                    return Err(DbError::SelfTestFailed(
                        "invalid backup verification flag".to_owned(),
                    ))
                }
            };
            Ok(BackupHistoryEntry {
                backup_id,
                vault_id,
                created_at,
                kind: BackupHistoryKind::parse(&kind)?,
                destination,
                format_version,
                size_bytes,
                verified,
                error,
            })
        })
        .collect()
    }
}

fn sanitize_history_error(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|character| !character.is_control())
        .take(300)
        .collect();
    if raw.chars().count() > 300 {
        format!("{cleaned}…")
    } else {
        cleaned
    }
}
