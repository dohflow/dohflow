//! The commit rule of `CommitImportedTransaction` for a row that claims a cross-device identity
//! (`personal-cfo-0g528.3`, ADR 0074 Addendum J items 3 and 4; `dohflow-engine-5`).
//!
//! The claim is **checked, never trusted**: apply recomputes the id from the components and refuses a
//! mismatch, binds the components to the row being committed (the source record and its batch, which
//! are class-1b rows every replica holds), looks the identity up in `import_identities` (never by
//! ledger id: transactions committed before identity keep their old ids), and computes the comparison
//! snapshot itself. Every verdict here reads the payload and class-1 and class-1b state only.
//!
//! | Situation | Result |
//! | --- | --- |
//! | No identity row | the commit proceeds, and the identity is recorded with its snapshot |
//! | Identity present, snapshot fields equal | a no-op returning the identity's transaction, one `merged` decision and an audit line |
//! | Identity present, provider fields differ | nothing is committed; one `provider_revision` disposition is recorded or its detail updated |
//! | Identity present, file fields differ | nothing is committed; one `file_row_conflict` disposition is recorded |
//! | The identity's transaction is voided | a no-op; a refresh never resurrects it |

use chrono::{DateTime, Utc};
use core_ids::{DerivedIds, FileFingerprint};
use core_ledger::{AccountId, SourceRecordId, TransactionId};
use core_money::Money;
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::{
    ingestion, CommandMeta, DbError, ImportIdentityClaim, ImportIdentityComponents, RejectCode,
};

/// The audit event recorded when two commits of one identity meet.
pub(crate) const IMPORT_MERGED_EVENT: &str = "import.identity_merged";

/// What the commit does with a claimed identity.
pub(crate) enum Resolution {
    /// No identity row: commit the transaction, then record this identity.
    Commit(Box<NewIdentity>),
    /// The identity decided the command: nothing more is written. The transaction to return.
    Settled(Uuid),
}

/// The identity to record after a commit, with the snapshot a later commit is compared against.
pub(crate) struct NewIdentity {
    id: Uuid,
    components: ImportIdentityComponents,
    snapshot: Snapshot,
}

impl NewIdentity {
    pub(crate) const fn id(&self) -> Uuid {
        self.id
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    account_id: Uuid,
    amount_minor: i64,
    currency: String,
    posted_date: String,
    provider_description: Option<String>,
}

fn invalid(message: &str) -> DbError {
    DbError::reject(RejectCode::PayloadInvalid, message.to_owned())
}

/// The fields of the claimed row's source record and batch that bind the identity to it.
struct Source {
    connection_id: Option<Uuid>,
    file_fingerprint: Option<String>,
    batch_id: Uuid,
    external_id: Option<String>,
    normalized_json: String,
}

fn read_source(tx: &Connection, source_record_id: Uuid) -> Result<Source, DbError> {
    Ok(tx.query_row(
        "SELECT sb.connector_connection_id, sb.file_fingerprint, sb.id, sr.external_id,
                sr.normalized_json
           FROM source_records sr JOIN source_batches sb ON sb.id = sr.source_batch_id
          WHERE sr.id = ?1",
        [source_record_id],
        |r| {
            Ok(Source {
                connection_id: r.get(0)?,
                file_fingerprint: r.get(1)?,
                batch_id: r.get(2)?,
                external_id: r.get(3)?,
                normalized_json: r.get(4)?,
            })
        },
    )?)
}

/// The account key and the description a SimpleFIN record's normalized payload holds (the same read
/// the migration-76 backfill makes, so the first refresh compares equal).
fn normalized_fields(json: &str) -> Option<(String, Option<String>)> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let account_key = value.get("account_key")?.as_str()?.to_owned();
    let description = value
        .get("description")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    Some((account_key, description))
}

/// Check the claim and decide what the commit does. See the module docs.
#[allow(clippy::too_many_arguments)] // the command's fields plus its metadata
pub(crate) fn resolve(
    tx: &Connection,
    meta: &CommandMeta,
    ids: &DerivedIds,
    claim: &ImportIdentityClaim,
    transaction_id: &TransactionId,
    account_id: &AccountId,
    source_record_id: &SourceRecordId,
    occurred_at: &DateTime<Utc>,
    amount: &Money,
) -> Result<Resolution, DbError> {
    let source = read_source(tx, source_record_id.as_uuid())?;
    let mut provider_description = None;

    // 1. The id is the components' derived id, and 2. the components are this row's own.
    match &claim.components {
        ImportIdentityComponents::Provider {
            connection_id,
            provider_account_key,
            provider_txn_id,
        } => {
            let derived = ids
                .provider_txn_id(connection_id, provider_account_key, provider_txn_id)
                .map_err(|_| invalid("an import identity component is not a valid provider id"))?;
            if derived != claim.identity_id {
                return Err(invalid("the import identity does not match its components"));
            }
            let (account_key, description) = normalized_fields(&source.normalized_json)
                .ok_or_else(|| invalid("the source record has no provider account key"))?;
            if source.connection_id != Some(*connection_id)
                || source.external_id.as_deref() != Some(provider_txn_id.as_str())
                || account_key != *provider_account_key
            {
                return Err(invalid(
                    "the import identity is not the identity of its source record",
                ));
            }
            // A provider row's ledger id IS its derived id, so two devices mint one transaction.
            if transaction_id.as_uuid() != derived {
                return Err(invalid(
                    "a provider transaction's id is its derived identity",
                ));
            }
            provider_description = description;
        }
        ImportIdentityComponents::File {
            file_fingerprint,
            row_ordinal,
        } => {
            let fingerprint = FileFingerprint::parse(file_fingerprint)
                .map_err(|_| invalid("an import identity names a malformed file fingerprint"))?;
            let derived = ids
                .file_row_id(&fingerprint, *row_ordinal)
                .map_err(|_| invalid("an import identity names an invalid row ordinal"))?;
            if derived != claim.identity_id {
                return Err(invalid("the import identity does not match its components"));
            }
            if source.file_fingerprint.as_deref() != Some(file_fingerprint.as_str()) {
                return Err(invalid(
                    "the import identity is not the identity of its source batch",
                ));
            }
            // The ledger id stays the (batch, ordinal) id of the upload.
            let expected = ids
                .content(
                    "txn.import",
                    &[
                        source.batch_id.as_bytes(),
                        i64::from(*row_ordinal).to_be_bytes().as_slice(),
                    ],
                )
                .map_err(|_| invalid("the row's ledger id cannot be derived"))?;
            if transaction_id.as_uuid() != expected {
                return Err(invalid(
                    "a file row's transaction id is the id of its batch and ordinal",
                ));
            }
        }
    }

    let snapshot = Snapshot {
        account_id: account_id.as_uuid(),
        amount_minor: amount.minor_units(),
        currency: amount.currency().code().to_owned(),
        posted_date: occurred_at.date_naive().to_string(),
        provider_description,
    };

    // 3. What the table holds for this identity (by identity, never by ledger id).
    let held: Option<(Uuid, Snapshot, bool)> = tx
        .query_row(
            "SELECT i.transaction_id, i.account_id, i.amount_minor, i.currency, i.posted_date,
                    i.provider_description, lt.voided_at IS NOT NULL
               FROM import_identities i
               LEFT JOIN ledger_transactions lt ON lt.id = i.transaction_id
              WHERE i.identity_id = ?1",
            [claim.identity_id],
            |r| {
                Ok((
                    r.get(0)?,
                    Snapshot {
                        account_id: r.get(1)?,
                        amount_minor: r.get(2)?,
                        currency: r.get(3)?,
                        posted_date: r.get(4)?,
                        provider_description: r.get(5)?,
                    },
                    r.get::<_, Option<bool>>(6)?.unwrap_or(false),
                ))
            },
        )
        .optional()?;
    let Some((held_transaction, held_snapshot, voided)) = held else {
        return Ok(Resolution::Commit(Box::new(NewIdentity {
            id: claim.identity_id,
            components: claim.components.clone(),
            snapshot,
        })));
    };
    // A refresh never resurrects what the person voided.
    if voided {
        return Ok(Resolution::Settled(held_transaction));
    }
    let changed = changed_fields(&claim.components, &held_snapshot, &snapshot);
    if changed.is_empty() {
        record_merged(tx, meta, ids, claim.identity_id, held_transaction, source.batch_id)?;
        return Ok(Resolution::Settled(held_transaction));
    }
    let kind = match claim.components {
        ImportIdentityComponents::Provider { .. } => "provider_revision",
        ImportIdentityComponents::File { .. } => "file_row_conflict",
    };
    record_disposition(tx, meta, ids, claim.identity_id, kind, &changed)?;
    Ok(Resolution::Settled(held_transaction))
}

/// The names of the compared fields that differ: the provider's amount, currency, posted date and
/// description, and for a file row the account, amount, currency and posted date. User-editable fields
/// are never compared.
fn changed_fields(
    components: &ImportIdentityComponents,
    held: &Snapshot,
    claimed: &Snapshot,
) -> Vec<&'static str> {
    let mut changed = Vec::new();
    let provider = matches!(components, ImportIdentityComponents::Provider { .. });
    if !provider && held.account_id != claimed.account_id {
        changed.push("account");
    }
    if held.amount_minor != claimed.amount_minor {
        changed.push("amount");
    }
    if held.currency != claimed.currency {
        changed.push("currency");
    }
    if held.posted_date != claimed.posted_date {
        changed.push("posted_date");
    }
    if provider && held.provider_description != claimed.provider_description {
        changed.push("description");
    }
    changed
}

/// Two commits of one identity met: one `merged` decision (its id is a content id of the identity and
/// `merged`, so a repeat writes nothing) and an audit line keyed by the command.
fn record_merged(
    tx: &Connection,
    meta: &CommandMeta,
    ids: &DerivedIds,
    identity_id: Uuid,
    transaction_id: Uuid,
    batch_id: Uuid,
) -> Result<(), DbError> {
    let id = ids
        .content(
            "import.dedupe-decision",
            &[identity_id.as_bytes(), b"merged"],
        )
        .map_err(|_| invalid("the merged decision's id cannot be derived"))?;
    let at = crate::apply::issued_at_text(meta);
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM dedupe_decisions WHERE id = ?1)",
        [id],
        |r| r.get(0),
    )?;
    if !exists {
        ingestion::record_dedupe_decision(
            tx,
            id,
            &ingestion::NewDedupeDecision {
                source_batch_id: batch_id,
                layer: "transaction",
                transaction_id: Some(transaction_id),
                identity_id: Some(identity_id),
                matched_entity_type: Some("ledger_transaction"),
                matched_entity_id: Some(transaction_id),
                decision: "merged",
                reason: "the same import identity was committed on another device",
            },
            &at,
        )?;
    }
    tx.execute(
        "INSERT OR IGNORE INTO audit_events (id, event_type, command_id, correlation_id, created_at)
         VALUES (?1, ?2, ?1, ?3, ?4)",
        params![
            meta.command_id,
            IMPORT_MERGED_EVENT,
            meta.correlation_id,
            at
        ],
    )?;
    Ok(())
}

/// Record one pending disposition for the identity, or update its detail (a second differing revision,
/// in command order). Never surfaced: nothing reads a pending `provider_revision` or
/// `file_row_conflict` until the resolution that Addendum B item 7 describes exists.
fn record_disposition(
    tx: &Connection,
    meta: &CommandMeta,
    ids: &DerivedIds,
    identity_id: Uuid,
    kind: &str,
    changed: &[&str],
) -> Result<(), DbError> {
    let id = ids
        .content(
            "import.disposition",
            &[identity_id.as_bytes(), kind.as_bytes()],
        )
        .map_err(|_| invalid("the disposition's id cannot be derived"))?;
    tx.execute(
        "INSERT INTO import_dispositions
             (disposition_id, subject_id, kind, status, generation, detail, created_at)
         VALUES (?1, ?2, ?3, 'pending', 1, ?4, ?5)
         ON CONFLICT(disposition_id) DO UPDATE SET detail = excluded.detail",
        params![
            id,
            identity_id,
            kind,
            changed.join(","),
            crate::apply::issued_at_text(meta)
        ],
    )?;
    Ok(())
}

/// Record the identity of a row just committed, with the snapshot a later commit is compared against.
pub(crate) fn record_identity(
    tx: &Connection,
    meta: &CommandMeta,
    transaction_id: Uuid,
    new: &NewIdentity,
) -> Result<(), DbError> {
    let s = &new.snapshot;
    let at = crate::apply::issued_at_text(meta);
    match &new.components {
        ImportIdentityComponents::Provider {
            connection_id,
            provider_account_key,
            provider_txn_id,
        } => {
            tx.execute(
                "INSERT INTO import_identities
                     (identity_id, kind, connection_id, provider_account_key, provider_txn_id,
                      transaction_id, account_id, amount_minor, currency, posted_date,
                      provider_description, created_at)
                 VALUES (?1, 'provider', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    new.id,
                    connection_id,
                    provider_account_key,
                    provider_txn_id,
                    transaction_id,
                    s.account_id,
                    s.amount_minor,
                    s.currency,
                    s.posted_date,
                    s.provider_description,
                    at
                ],
            )?;
        }
        ImportIdentityComponents::File {
            file_fingerprint,
            row_ordinal,
        } => {
            let fingerprint = FileFingerprint::parse(file_fingerprint)
                .map_err(|_| invalid("an import identity names a malformed file fingerprint"))?;
            tx.execute(
                "INSERT INTO import_identities
                     (identity_id, kind, file_fingerprint, row_ordinal, transaction_id, account_id,
                      amount_minor, currency, posted_date, created_at)
                 VALUES (?1, 'file', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    new.id,
                    fingerprint.as_bytes().as_slice(),
                    i64::from(*row_ordinal),
                    transaction_id,
                    s.account_id,
                    s.amount_minor,
                    s.currency,
                    s.posted_date,
                    at
                ],
            )?;
        }
    }
    Ok(())
}
