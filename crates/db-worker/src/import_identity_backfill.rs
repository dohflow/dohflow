//! The migration to schema version 76 (personal-cfo-0g528.3, ADR 0074 Addendum J item 7): the guard
//! that refuses an armed vault, and the backfill of `import_identities` for what a vault imported
//! before cross-device identity existed.
//!
//! **The refusal.** A write to class-1 rows outside a command is in no log row and no envelope, so no
//! other device receives it. The migration therefore refuses, atomically and before it writes anything,
//! on a vault that holds a `sync_base` or a `sync_genesis` row ([`refuse_on_an_armed_vault`]). A vault
//! with neither is safe: the rows are ordinary class-1 state and a later genesis carries them to every
//! replica.
//!
//! **The backfill.** Without it a second device's first refresh would find no identity for a
//! transaction the first already committed and commit it again. One identity is written for every
//! committed SimpleFIN transaction whose provider id is recoverable, and one for every committed
//! file-import row whose staging row still names its ordinal. Nothing else: a row it cannot recover
//! gets no identity and stays on the heuristic path, and is counted in the [`BackfillReport`].
//!
//! The provider components come from the RECORD and its BATCH, never from `connector_account_links`
//! (which is written raw and is not one-to-one with provider keys): `connector_connection_id` of the
//! batch, `external_id` of the source record, and the `account_key` inside the record's normalized
//! payload, which is exactly what the adapter emitted then. The comparison snapshot is the record's
//! own: its posted date and description from the normalized payload, the amount and currency from the
//! committed posting, so the first refresh compares equal.
//!
//! **One identity per key.** When two committed transactions meet one key (a file voided and
//! re-imported by the old silent restore; a duplicate from before cross-device dedupe), the identity
//! points at the ACTIVE one, or when none is active at the lowest ledger id, and the rest are counted.

use core_ids::{DerivedIdKey, FileFingerprint};
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::DbError;

/// What a backfill did. Counts only: no id, no text.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct BackfillReport {
    /// Provider identities written.
    pub provider_written: usize,
    /// A committed SimpleFIN transaction whose batch has no connection id (a batch from before
    /// connection identity): not recoverable.
    pub provider_no_connection: usize,
    /// A committed SimpleFIN transaction whose record lacks a provider id or an account key, or whose
    /// strings fail ADR 0013-A section 9's byte rules, or with no readable posted date: not recoverable.
    pub provider_unrecoverable: usize,
    /// A second committed transaction for an identity already claimed: left without an identity.
    pub provider_collisions: usize,
    /// File-row identities written.
    pub file_written: usize,
    /// A committed file-import row with no fingerprint in section 12's form, or whose staging row is
    /// gone (the ordinal is device-local): not recoverable.
    pub file_unrecoverable: usize,
    /// A second committed transaction for a file row already claimed: left without an identity.
    pub file_collisions: usize,
}

/// A candidate identity before collisions are resolved.
struct Candidate {
    identity_id: Uuid,
    transaction_id: Uuid,
    active: bool,
    kind: Kind,
    account_id: Uuid,
    amount_minor: i64,
    currency: String,
    posted_date: String,
    created_at: String,
}

enum Kind {
    Provider {
        connection_id: Uuid,
        account_key: String,
        txn_id: String,
        description: Option<String>,
    },
    File {
        fingerprint: [u8; 32],
        ordinal: i64,
    },
}

/// The PRE step of migration 76: refuse on a vault where Sync is armed.
///
/// # Errors
/// [`DbError::MigrationRefusedSyncArmed`] when a `sync_base` or a `sync_genesis` row exists; the
/// migration's transaction then rolls back and the vault stays at the previous schema.
pub(crate) fn refuse_on_an_armed_vault(conn: &Connection, version: i64) -> Result<(), DbError> {
    let case = armed_case(conn)?;
    match case {
        None => Ok(()),
        Some(case) => Err(DbError::MigrationRefusedSyncArmed { version, case }),
    }
}

fn armed_case(conn: &Connection) -> rusqlite::Result<Option<&'static str>> {
    let has_row = |table: &str| -> rusqlite::Result<bool> {
        conn.query_row(&format!("SELECT EXISTS(SELECT 1 FROM {table})"), [], |r| {
            r.get(0)
        })
    };
    if has_row("sync_base")? {
        return Ok(Some("a sync_base row exists"));
    }
    if has_row("sync_genesis")? {
        return Ok(Some("a sync_genesis row exists"));
    }
    Ok(None)
}

/// The POST step of migration 76: the backfill. A fresh vault has no `vault_metadata` row yet and
/// nothing to backfill.
pub(crate) fn migrate_up(conn: &Connection) -> rusqlite::Result<()> {
    let report = backfill(conn)?;
    // Counts only (the redaction rule): what a support session needs to see that a backfill ran.
    tracing::info!(
        provider_written = report.provider_written,
        provider_no_connection = report.provider_no_connection,
        provider_unrecoverable = report.provider_unrecoverable,
        provider_collisions = report.provider_collisions,
        file_written = report.file_written,
        file_unrecoverable = report.file_unrecoverable,
        file_collisions = report.file_collisions,
        "import identities backfilled"
    );
    Ok(())
}

/// Write the identities. Returns what it did.
pub(crate) fn backfill(conn: &Connection) -> rusqlite::Result<BackfillReport> {
    let mut report = BackfillReport::default();
    let vault_id: Option<Uuid> = conn
        .query_row(
            "SELECT vault_id FROM vault_metadata WHERE singleton = 1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    let Some(vault_id) = vault_id else {
        return Ok(report);
    };
    let key = DerivedIdKey::for_vault(&vault_id);

    let mut provider = provider_candidates(conn, &key, &mut report)?;
    let mut file = file_candidates(conn, &key, &mut report)?;
    report.provider_collisions +=
        write_resolved(conn, &mut provider, &mut report.provider_written)?;
    report.file_collisions += write_resolved(conn, &mut file, &mut report.file_written)?;
    Ok(report)
}

/// The user's posting of a committed import: the account, the signed amount and the currency.
fn user_posting(
    conn: &Connection,
    transaction_id: Uuid,
) -> rusqlite::Result<Option<(Uuid, i64, String)>> {
    conn.query_row(
        "SELECT a.id, lp.minor_units, lp.currency
           FROM ledger_postings lp
           JOIN accounts a ON a.ledger_account_id = lp.ledger_account_id
          WHERE lp.transaction_id = ?1
          ORDER BY a.id LIMIT 1",
        [transaction_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .optional()
}

fn provider_candidates(
    conn: &Connection,
    key: &DerivedIdKey,
    report: &mut BackfillReport,
) -> rusqlite::Result<Vec<Candidate>> {
    let mut stmt = conn.prepare(
        "SELECT lt.id, lt.voided_at IS NULL, sr.external_id, sr.normalized_json,
                sb.connector_connection_id, sr.created_at
           FROM ledger_transactions lt
           JOIN source_provenance_links pl
             ON pl.entity_type = 'ledger_transaction' AND pl.entity_id = lt.id
            AND pl.relationship = 'created_from'
           JOIN source_records sr ON sr.id = pl.source_record_id
           JOIN source_batches sb ON sb.id = sr.source_batch_id
          WHERE sb.source_type = 'simplefin'
          ORDER BY lt.id, pl.id",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, Uuid>(0)?,
                r.get::<_, bool>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<Uuid>>(4)?,
                r.get::<_, String>(5)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut out = Vec::new();
    for (transaction_id, active, external_id, normalized, connection, created_at) in rows {
        let Some(connection_id) = connection else {
            report.provider_no_connection += 1;
            continue;
        };
        let parsed = parse_normalized(&normalized);
        let (Some(txn_id), Some((account_key, posted_date, description))) = (external_id, parsed)
        else {
            report.provider_unrecoverable += 1;
            continue;
        };
        let (Some((account_id, amount_minor, currency)), Ok(identity_id)) = (
            user_posting(conn, transaction_id)?,
            key.provider_txn_id(&connection_id, &account_key, &txn_id),
        ) else {
            report.provider_unrecoverable += 1;
            continue;
        };
        out.push(Candidate {
            identity_id,
            transaction_id,
            active,
            kind: Kind::Provider {
                connection_id,
                account_key,
                txn_id,
                description,
            },
            account_id,
            amount_minor,
            currency,
            posted_date,
            created_at,
        });
    }
    Ok(out)
}

/// `(account_key, posted_date, description)` from a SimpleFIN record's normalized payload.
fn parse_normalized(json: &str) -> Option<(String, String, Option<String>)> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let account_key = value.get("account_key")?.as_str()?.to_owned();
    let posted = value.get("posted")?.as_i64()?;
    // The adapter's own mapping of the wire's epoch seconds: a positive value, read as a UTC date.
    if posted <= 0 {
        return None;
    }
    let posted_date = chrono::DateTime::from_timestamp(posted, 0)?
        .date_naive()
        .to_string();
    let description = value
        .get("description")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    Some((account_key, posted_date, description))
}

fn file_candidates(
    conn: &Connection,
    key: &DerivedIdKey,
    report: &mut BackfillReport,
) -> rusqlite::Result<Vec<Candidate>> {
    let mut stmt = conn.prepare(
        "SELECT lt.id, lt.voided_at IS NULL, sb.file_fingerprint, st.row_ordinal,
                st.proposed_account_id, st.amount_minor, st.currency, st.posted_at, sr.created_at
           FROM ledger_transactions lt
           JOIN source_provenance_links pl
             ON pl.entity_type = 'ledger_transaction' AND pl.entity_id = lt.id
            AND pl.relationship = 'created_from'
           JOIN source_records sr ON sr.id = pl.source_record_id
           JOIN source_batches sb ON sb.id = sr.source_batch_id
      LEFT JOIN staged_transactions st
             ON st.source_record_id = sr.id AND st.committed_transaction_id = lt.id
          WHERE sb.file_fingerprint IS NOT NULL
          ORDER BY lt.id, pl.id",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, Uuid>(0)?,
                r.get::<_, bool>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<i64>>(3)?,
                r.get::<_, Option<Uuid>>(4)?,
                r.get::<_, Option<i64>>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, String>(8)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut out = Vec::new();
    for (
        transaction_id,
        active,
        fingerprint,
        ordinal,
        account,
        amount,
        currency,
        posted,
        created,
    ) in rows
    {
        let (
            Ok(fp),
            Some(ordinal),
            Some(account_id),
            Some(amount_minor),
            Some(currency),
            Some(posted_date),
        ) = (
            FileFingerprint::parse(&fingerprint),
            ordinal.filter(|o| *o >= 1),
            account,
            amount,
            currency,
            // The date a commit would give (its UTC date), not the staged text: a timestamp with an
            // offset names the UTC day.
            posted
                .as_deref()
                .and_then(crate::apply::ingestion_cmds::parse_staged_posted_at)
                .map(|at| at.date_naive().to_string()),
        )
        else {
            report.file_unrecoverable += 1;
            continue;
        };
        let Ok(ordinal_u32) = u32::try_from(ordinal) else {
            report.file_unrecoverable += 1;
            continue;
        };
        let Ok(identity_id) = key.file_row_id(&fp, ordinal_u32) else {
            report.file_unrecoverable += 1;
            continue;
        };
        out.push(Candidate {
            identity_id,
            transaction_id,
            active,
            kind: Kind::File {
                fingerprint: *fp.as_bytes(),
                ordinal,
            },
            account_id,
            amount_minor,
            currency,
            posted_date,
            created_at: created,
        });
    }
    Ok(out)
}

/// Write one identity per key: the active candidate, else the lowest transaction id. Returns how
/// many candidates lost.
fn write_resolved(
    conn: &Connection,
    candidates: &mut [Candidate],
    written: &mut usize,
) -> rusqlite::Result<usize> {
    // Active first, then the lowest transaction id (a total order: ids are unique).
    candidates.sort_by(|a, b| {
        a.identity_id
            .cmp(&b.identity_id)
            .then(b.active.cmp(&a.active))
            .then(a.transaction_id.cmp(&b.transaction_id))
    });
    let mut collisions = 0;
    let mut last: Option<Uuid> = None;
    for c in candidates.iter() {
        if last == Some(c.identity_id) {
            collisions += 1;
            continue;
        }
        last = Some(c.identity_id);
        match &c.kind {
            Kind::Provider {
                connection_id,
                account_key,
                txn_id,
                description,
            } => {
                conn.execute(
                    "INSERT INTO import_identities
                        (identity_id, kind, connection_id, provider_account_key, provider_txn_id,
                         transaction_id, account_id, amount_minor, currency, posted_date,
                         provider_description, created_at)
                     VALUES (?1, 'provider', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    params![
                        c.identity_id,
                        connection_id,
                        account_key,
                        txn_id,
                        c.transaction_id,
                        c.account_id,
                        c.amount_minor,
                        c.currency,
                        c.posted_date,
                        description,
                        c.created_at
                    ],
                )?;
            }
            Kind::File {
                fingerprint,
                ordinal,
            } => {
                conn.execute(
                    "INSERT INTO import_identities
                        (identity_id, kind, file_fingerprint, row_ordinal, transaction_id,
                         account_id, amount_minor, currency, posted_date, created_at)
                     VALUES (?1, 'file', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        c.identity_id,
                        fingerprint.as_slice(),
                        ordinal,
                        c.transaction_id,
                        c.account_id,
                        c.amount_minor,
                        c.currency,
                        c.posted_date,
                        c.created_at
                    ],
                )?;
            }
        }
        *written += 1;
    }
    Ok(collisions)
}

#[cfg(test)]
mod tests {
    //! The refusal and the backfill, driven at the schema before 76 and migrated.

    use super::*;
    use crate::migrations::{apply, migrate_down, MIGRATIONS};

    const VAULT: u128 = 0x0198_c2c1_5f0a_7c3d_8a4e_9b1d_2e3f_4a5b;
    const CONNECTION: u128 = 0x0198_c2c1_5f0a_7c3d_8a4e_9b1d_0000_0001;
    const ACCOUNT: u128 = 0xa000;
    const LEDGER_ACCOUNT: u128 = 0xa100;
    const COUNTER: u128 = 0xa200;
    const FINGERPRINT: &str =
        "sha256:2c30d6d4f9c6954bfc4f2e31460da5660e8d4aa9d893b6228eddc467516a6830";

    fn uid(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    /// A vault at schema 75 with a household vault id and one account.
    fn before() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn, &MIGRATIONS[..75]).unwrap();
        conn.execute(
            "INSERT INTO vault_metadata (singleton, vault_id, schema_version, envelope_version,
                 kdf_algorithm, kdf_memory_kib, kdf_time_cost, kdf_parallelism, created_at)
             VALUES (1, ?1, 75, 1, 'argon2id', 1, 1, 1, 't')",
            [uid(VAULT)],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO accounts (id, ledger_account_id, name, cashflow_role, normal_balance, currency)
             VALUES (?1, ?2, 'Checking', 'cash', 'debit', 'USD')",
            params![uid(ACCOUNT), uid(LEDGER_ACCOUNT)],
        )
        .unwrap();
        conn
    }

    fn batch(
        conn: &Connection,
        id: u128,
        source_type: &str,
        fp: Option<&str>,
        connection: Option<u128>,
    ) {
        conn.execute(
            "INSERT INTO source_batches (id, source_type, file_fingerprint, created_at, updated_at,
                                         connector_connection_id)
             VALUES (?1, ?2, ?3, 't', 't', ?4)",
            params![uid(id), source_type, fp, connection.map(uid)],
        )
        .unwrap();
    }

    fn record(
        conn: &Connection,
        id: u128,
        batch: u128,
        external_id: Option<&str>,
        normalized: &str,
    ) {
        conn.execute(
            "INSERT INTO source_records (id, source_batch_id, external_id, source_hash, normalized_json,
                                         created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'rec-time')",
            params![uid(id), uid(batch), external_id, format!("h{id}"), normalized],
        )
        .unwrap();
    }

    /// A committed import: the transaction, its two postings and the `created_from` link.
    fn committed(conn: &Connection, txn: u128, record_id: u128, minor: i64, voided: bool) {
        for (account, amount) in [(LEDGER_ACCOUNT, minor), (COUNTER, -minor)] {
            conn.execute(
                "INSERT INTO ledger_postings (transaction_id, ledger_account_id, minor_units, currency,
                                              currency_exponent, posting_date)
                 VALUES (?1, ?2, ?3, 'USD', 2, '2026-03-04')",
                params![uid(txn), uid(account), amount],
            )
            .unwrap();
        }
        // The header goes last: the balance trigger fires on it, once the postings are present.
        conn.execute(
            "INSERT INTO ledger_transactions (id, operation_id, occurred_at, currency, voided_at)
             VALUES (?1, ?2, '2026-03-04T00:00:00Z', 'USD', ?3)",
            params![uid(txn), uid(1), voided.then_some("v")],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO source_provenance_links (id, entity_type, entity_id, source_record_id,
                                                  relationship, created_at)
             VALUES (?1, 'ledger_transaction', ?2, ?3, 'created_from', 't')",
            params![uid(txn + 0x1_0000), uid(txn), uid(record_id)],
        )
        .unwrap();
    }

    fn staged(conn: &Connection, record_id: u128, txn: u128, ordinal: i64, minor: i64) {
        conn.execute(
            "INSERT INTO staged_transactions (id, source_record_id, proposed_account_id, posted_at,
                 amount_minor, currency, txn_fingerprint, commit_status, committed_transaction_id,
                 created_at, row_ordinal)
             VALUES (?1, ?2, ?3, '2026-03-04', ?4, 'USD', 'fp', 'committed', ?5, 't', ?6)",
            params![
                uid(txn + 0x2_0000),
                uid(record_id),
                uid(ACCOUNT),
                minor,
                uid(txn),
                ordinal
            ],
        )
        .unwrap();
    }

    fn simplefin_record(conn: &Connection, record_id: u128, batch_id: u128, txn_id: &str) {
        let json = format!(
            "{{\"account_key\":\"conn-1/acct-9\",\"txn_id\":\"{txn_id}\",\"posted\":1772582400,\"amount\":\"-12.34\",\"description\":\"COFFEE\",\"transacted_at\":null}}"
        );
        record(conn, record_id, batch_id, Some(txn_id), &json);
    }

    fn migrate(conn: &mut Connection) {
        apply(conn, MIGRATIONS).unwrap();
    }

    type IdentityRow = (Uuid, String, Uuid, i64, String, String, Option<String>);

    fn identities(conn: &Connection) -> Vec<IdentityRow> {
        let mut stmt = conn
            .prepare(
                "SELECT identity_id, kind, transaction_id, amount_minor, currency, posted_date,
                        provider_description
                   FROM import_identities ORDER BY identity_id",
            )
            .unwrap();
        stmt.query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
    }

    fn key() -> DerivedIdKey {
        DerivedIdKey::for_vault(&uid(VAULT))
    }

    #[test]
    fn a_committed_simplefin_transaction_gets_its_provider_identity_with_the_records_own_snapshot()
    {
        let mut conn = before();
        batch(&conn, 0x10, "simplefin", None, Some(CONNECTION));
        simplefin_record(&conn, 0x20, 0x10, "TXN-1");
        committed(&conn, 0x30, 0x20, -1234, false);
        migrate(&mut conn);
        let expected = key()
            .provider_txn_id(&uid(CONNECTION), "conn-1/acct-9", "TXN-1")
            .unwrap();
        assert_eq!(
            identities(&conn),
            vec![(
                expected,
                "provider".to_owned(),
                uid(0x30),
                -1234,
                "USD".to_owned(),
                // 1772582400 seconds is 2026-03-04 in UTC.
                "2026-03-04".to_owned(),
                Some("COFFEE".to_owned())
            )]
        );
        // The components, and the snapshot's account, are the record's and the batch's own.
        let (connection, account_key, txn_id, account): (Uuid, String, String, Uuid) = conn
            .query_row(
                "SELECT connection_id, provider_account_key, provider_txn_id, account_id
                   FROM import_identities",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            (connection, account_key.as_str(), txn_id.as_str(), account),
            (uid(CONNECTION), "conn-1/acct-9", "TXN-1", uid(ACCOUNT))
        );
    }

    #[test]
    fn what_cannot_be_recovered_gets_no_identity_and_is_counted() {
        let mut conn = before();
        // No connection id (a batch from before connection identity).
        batch(&conn, 0x10, "simplefin", None, None);
        simplefin_record(&conn, 0x20, 0x10, "TXN-A");
        committed(&conn, 0x30, 0x20, -100, false);
        // A record with no provider id, and one whose id breaks the byte rules.
        batch(&conn, 0x11, "simplefin", None, Some(CONNECTION));
        record(&conn, 0x21, 0x11, None, "{}");
        committed(&conn, 0x31, 0x21, -200, false);
        simplefin_record(&conn, 0x22, 0x11, "bad\u{7f}id");
        committed(&conn, 0x32, 0x22, -300, false);
        // LunchFlow is not covered (Addendum B item 11), and a manual row has no batch fingerprint.
        batch(&conn, 0x12, "lunchflow", None, Some(CONNECTION));
        simplefin_record(&conn, 0x23, 0x12, "LF-1");
        committed(&conn, 0x33, 0x23, -400, false);
        // A file row whose staging is gone, and one with a malformed fingerprint.
        batch(&conn, 0x13, "csv", Some(FINGERPRINT), None);
        record(&conn, 0x24, 0x13, None, "{}");
        committed(&conn, 0x34, 0x24, -500, false);
        batch(&conn, 0x14, "csv", Some("SHA256:nope"), None);
        record(&conn, 0x25, 0x14, None, "{}");
        committed(&conn, 0x35, 0x25, -600, false);
        staged(&conn, 0x25, 0x35, 1, -600);

        apply(&mut conn, MIGRATIONS).unwrap();
        // The step ran inside `apply`; run the report once more on a clean table to read the counts.
        conn.execute("DELETE FROM import_identities", []).unwrap();
        let report = backfill(&conn).unwrap();
        assert_eq!(
            report,
            BackfillReport {
                provider_written: 0,
                provider_no_connection: 1,
                provider_unrecoverable: 2,
                provider_collisions: 0,
                file_written: 0,
                file_unrecoverable: 2,
                file_collisions: 0,
            }
        );
        assert!(identities(&conn).is_empty());
    }

    #[test]
    fn one_identity_per_key_points_at_the_active_transaction_then_the_lowest_id() {
        let mut conn = before();
        batch(&conn, 0x10, "simplefin", None, Some(CONNECTION));
        // The same provider transaction committed three times: a voided one, an active one at a higher
        // id, and a second active one.
        for (record_id, txn, voided) in
            [(0x20, 0x30, true), (0x21, 0x31, false), (0x22, 0x32, false)]
        {
            simplefin_record(&conn, record_id, 0x10, "TXN-1");
            committed(&conn, txn, record_id, -1234, voided);
        }
        // A second provider transaction whose copies are all voided: the lowest id wins.
        for (record_id, txn) in [(0x23, 0x41), (0x24, 0x40)] {
            simplefin_record(&conn, record_id, 0x10, "TXN-2");
            committed(&conn, txn, record_id, -50, true);
        }
        migrate(&mut conn);
        let pointed: Vec<Uuid> = identities(&conn).into_iter().map(|r| r.2).collect();
        let first = key()
            .provider_txn_id(&uid(CONNECTION), "conn-1/acct-9", "TXN-1")
            .unwrap();
        let second = key()
            .provider_txn_id(&uid(CONNECTION), "conn-1/acct-9", "TXN-2")
            .unwrap();
        let target = |identity: Uuid| -> Uuid {
            conn.query_row(
                "SELECT transaction_id FROM import_identities WHERE identity_id = ?1",
                [identity],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(pointed.len(), 2);
        assert_eq!(
            target(first),
            uid(0x31),
            "the lowest ACTIVE id, not the voided one"
        );
        assert_eq!(
            target(second),
            uid(0x40),
            "all voided: the lowest ledger id"
        );
    }

    #[test]
    fn a_file_voided_and_silently_restored_has_one_identity_per_row_on_the_active_import() {
        let mut conn = before();
        // The old silent restore: a voided batch and its re-import share the fingerprint and the ordinals.
        for (batch_id, record_id, txn, voided) in
            [(0x10, 0x20, 0x30, true), (0x11, 0x21, 0x31, false)]
        {
            batch(&conn, batch_id, "csv", Some(FINGERPRINT), None);
            record(&conn, record_id, batch_id, None, "{}");
            committed(&conn, txn, record_id, -700, voided);
            staged(&conn, record_id, txn, 1, -700);
        }
        migrate(&mut conn);
        let fp = FileFingerprint::parse(FINGERPRINT).unwrap();
        let expected = key().file_row_id(&fp, 1).unwrap();
        assert_eq!(
            identities(&conn),
            vec![(
                expected,
                "file".to_owned(),
                uid(0x31),
                -700,
                "USD".to_owned(),
                "2026-03-04".to_owned(),
                None
            )]
        );
        let (account, created): (Uuid, String) = conn
            .query_row(
                "SELECT account_id, created_at FROM import_identities",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((account, created.as_str()), (uid(ACCOUNT), "rec-time"));
    }

    /// The snapshot date is the date a commit gives (the UTC day of `parse_staged_posted_at`), not the
    /// staged text: the staged column the commit path reads, with its own parse.
    #[test]
    fn a_file_rows_snapshot_date_is_the_utc_day_a_commit_would_give() {
        for (posted, expected) in [
            ("2026-03-04T23:30:00-05:00", "2026-03-05"),
            ("2026-03-04T00:30:00+05:00", "2026-03-03"),
            ("2026-03-04T12:00:00Z", "2026-03-04"),
            ("2026-03-04", "2026-03-04"),
        ] {
            let mut conn = before();
            batch(&conn, 0x10, "csv", Some(FINGERPRINT), None);
            record(&conn, 0x20, 0x10, None, "{}");
            committed(&conn, 0x30, 0x20, -700, false);
            staged(&conn, 0x20, 0x30, 1, -700);
            conn.execute("UPDATE staged_transactions SET posted_at = ?1", [posted])
                .unwrap();
            migrate(&mut conn);
            let date: String = conn
                .query_row("SELECT posted_date FROM import_identities", [], |r| {
                    r.get(0)
                })
                .unwrap();
            assert_eq!(date, expected, "{posted}");
        }
        // A staged date the commit path cannot read gives no identity (counted), as it gives no commit.
        let mut conn = before();
        batch(&conn, 0x10, "csv", Some(FINGERPRINT), None);
        record(&conn, 0x20, 0x10, None, "{}");
        committed(&conn, 0x30, 0x20, -700, false);
        staged(&conn, 0x20, 0x30, 1, -700);
        conn.execute(
            "UPDATE staged_transactions SET posted_at = 'not a date'",
            [],
        )
        .unwrap();
        migrate(&mut conn);
        assert!(identities(&conn).is_empty());
    }

    #[test]
    fn an_armed_vault_refuses_atomically_and_stays_at_the_previous_schema() {
        for (sql, case) in [
            (
                "INSERT INTO sync_base (singleton, base_seq, state, updated_at)
                 VALUES (1, 0, 'NoBase', 't')",
                "a sync_base row exists",
            ),
            (
                "INSERT INTO sync_genesis (singleton, vault_id, membership_record, epoch_record, state,
                                           persisted_at, updated_at)
                 VALUES (1, zeroblob(16), x'01', x'02', 'Persisted', 't', 't')",
                "a sync_genesis row exists",
            ),
        ] {
            let mut conn = before();
            batch(&conn, 0x10, "simplefin", None, Some(CONNECTION));
            simplefin_record(&conn, 0x20, 0x10, "TXN-1");
            committed(&conn, 0x30, 0x20, -1234, false);
            conn.execute(sql, []).unwrap();
            let error = apply(&mut conn, MIGRATIONS).unwrap_err();
            assert!(
                matches!(&error, DbError::MigrationRefusedSyncArmed { version: 76, case: c } if *c == case),
                "{error:?}"
            );
            assert!(error.to_string().contains(case));
            // Nothing was written: no table, no tracker row.
            let tables: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE name LIKE 'import_%'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(tables, 0);
            let latest: i64 = conn
                .query_row("SELECT MAX(version) FROM schema_migrations", [], |r| r.get(0))
                .unwrap();
            assert_eq!(latest, 75);
        }
    }

    #[test]
    fn a_vault_that_never_enabled_sync_backfills_and_a_fresh_one_has_nothing_to_do() {
        let mut conn = before();
        batch(&conn, 0x10, "simplefin", None, Some(CONNECTION));
        simplefin_record(&conn, 0x20, 0x10, "TXN-1");
        committed(&conn, 0x30, 0x20, -1234, false);
        migrate(&mut conn);
        assert_eq!(identities(&conn).len(), 1);
        // A second open applies nothing and writes nothing more.
        assert_eq!(apply(&mut conn, MIGRATIONS).unwrap(), 0);
        assert_eq!(identities(&conn).len(), 1);

        // A fresh vault has no `vault_metadata` row yet.
        let mut fresh = Connection::open_in_memory().unwrap();
        apply(&mut fresh, MIGRATIONS).unwrap();
        assert_eq!(identities(&fresh).len(), 0);
    }

    #[test]
    fn the_tables_check_each_kind_against_its_columns_and_the_down_never_drops_a_row() {
        let mut conn = before();
        migrate(&mut conn);
        let bad = [
            // A provider row without its components.
            "INSERT INTO import_identities (identity_id, kind, transaction_id, account_id, amount_minor,
                 currency, posted_date, created_at)
             VALUES (zeroblob(16), 'provider', zeroblob(16), zeroblob(16), 1, 'USD', 'd', 't')",
            // A provider row that also carries a file column.
            "INSERT INTO import_identities (identity_id, kind, connection_id, provider_account_key,
                 provider_txn_id, row_ordinal, transaction_id, account_id, amount_minor, currency,
                 posted_date, created_at)
             VALUES (zeroblob(16), 'provider', zeroblob(16), 'a', 't', 1, zeroblob(16), zeroblob(16),
                 1, 'USD', 'd', 't')",
            // A file row with a short fingerprint, and one with ordinal zero.
            "INSERT INTO import_identities (identity_id, kind, file_fingerprint, row_ordinal,
                 transaction_id, account_id, amount_minor, currency, posted_date, created_at)
             VALUES (zeroblob(16), 'file', zeroblob(31), 1, zeroblob(16), zeroblob(16), 1, 'USD', 'd', 't')",
            "INSERT INTO import_identities (identity_id, kind, file_fingerprint, row_ordinal,
                 transaction_id, account_id, amount_minor, currency, posted_date, created_at)
             VALUES (zeroblob(16), 'file', zeroblob(32), 0, zeroblob(16), zeroblob(16), 1, 'USD', 'd', 't')",
            // A disposition with an unknown kind, status or generation zero.
            "INSERT INTO import_dispositions (disposition_id, subject_id, kind, status, created_at)
             VALUES (zeroblob(16), zeroblob(16), 'nope', 'pending', 't')",
            "INSERT INTO import_dispositions (disposition_id, subject_id, kind, status, created_at)
             VALUES (zeroblob(16), zeroblob(16), 'skipped', 'done', 't')",
            "INSERT INTO import_dispositions (disposition_id, subject_id, kind, status, generation, created_at)
             VALUES (zeroblob(16), zeroblob(16), 'skipped', 'pending', 0, 't')",
            // An outcome on a question that is not resolved, on another kind, or one that is unknown.
            "INSERT INTO import_dispositions (disposition_id, subject_id, kind, status, outcome, created_at)
             VALUES (zeroblob(16), zeroblob(16), 'restore_question', 'pending', 'restored', 't')",
            "INSERT INTO import_dispositions (disposition_id, subject_id, kind, status, outcome, created_at)
             VALUES (zeroblob(16), zeroblob(16), 'skipped', 'resolved', 'restored', 't')",
            "INSERT INTO import_dispositions (disposition_id, subject_id, kind, status, outcome, created_at)
             VALUES (zeroblob(16), zeroblob(16), 'restore_question', 'resolved', 'nope', 't')",
            // A resolved question without an outcome.
            "INSERT INTO import_dispositions (disposition_id, subject_id, kind, status, created_at)
             VALUES (zeroblob(16), zeroblob(16), 'restore_question', 'resolved', 't')",
            // A claimed batch exactly when the outcome is import_anyway.
            "INSERT INTO import_dispositions (disposition_id, subject_id, kind, status, outcome, created_at)
             VALUES (zeroblob(16), zeroblob(16), 'restore_question', 'resolved', 'import_anyway', 't')",
            "INSERT INTO import_dispositions (disposition_id, subject_id, kind, status, outcome,
                 claimed_source_batch_id, created_at)
             VALUES (zeroblob(16), zeroblob(16), 'restore_question', 'resolved', 'restored', zeroblob(16), 't')",
            "INSERT INTO import_dispositions (disposition_id, subject_id, kind, status, outcome,
                 claimed_source_batch_id, created_at)
             VALUES (zeroblob(16), zeroblob(16), 'restore_question', 'resolved', 'import_anyway', zeroblob(15), 't')",
        ];
        for sql in bad {
            assert!(conn.execute(sql, []).is_err(), "{sql}");
        }
        // The three good shapes of a question are accepted.
        for (outcome, batch) in [
            ("restored", None),
            ("dismissed", None),
            ("import_anyway", Some(16)),
        ] {
            let sql = match batch {
                Some(n) => format!(
                    "INSERT INTO import_dispositions (disposition_id, subject_id, kind, status, outcome,
                         claimed_source_batch_id, created_at)
                     VALUES (randomblob(16), zeroblob(16), 'restore_question', 'resolved', '{outcome}',
                         zeroblob({n}), 't')"
                ),
                None => format!(
                    "INSERT INTO import_dispositions (disposition_id, subject_id, kind, status, outcome, created_at)
                     VALUES (randomblob(16), zeroblob(16), 'restore_question', 'resolved', '{outcome}', 't')"
                ),
            };
            conn.execute(&sql, []).unwrap();
        }
        conn.execute("DELETE FROM import_dispositions", []).unwrap();
        // The down drops the empty tables, and refuses (changing nothing) while either holds a row.
        conn.execute(
            "INSERT INTO import_dispositions (disposition_id, subject_id, kind, status, created_at)
             VALUES (zeroblob(16), zeroblob(16), 'skipped', 'pending', 't')",
            [],
        )
        .unwrap();
        assert!(migrate_down(&mut conn, 75).is_err());
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM import_dispositions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1, "a refused down drops no row");
        conn.execute("DELETE FROM import_dispositions", []).unwrap();
        assert_eq!(migrate_down(&mut conn, 75).unwrap(), 1);
        assert_eq!(apply(&mut conn, MIGRATIONS).unwrap(), 1, "and up again");
    }
}
