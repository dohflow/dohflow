//! The data half of migration 77 (`personal-cfo-0g528.3`, ADR 0074 Addendum J item 6): `dedupe_decisions`
//! becomes a class-1 table keyed to a transaction or an identity instead of to a device-local staging
//! row, and a *flagged* decision moves onto the staged row it is about.
//!
//! The migration's SQL renames the old table to `dedupe_decisions_legacy`, creates the new one and adds
//! two class-3 columns to `staged_transactions` (`flag_reason`, `flag_matched_transaction_id`); this
//! post-step converts the rows and drops the legacy table:
//!
//! - **flagged** rows copy their reason and matched ledger transaction onto the staged row and leave
//!   the table. A flagged row whose staged row is gone is counted.
//! - **skipped** rows are dropped (counted) when they carry nothing the staged row does not: a user skip
//!   records `matched` NULL and the reason `user skipped`, and the staged row keeps `commit_status =
//!   'skipped'`. A skipped row that does carry a matched ledger transaction copies it onto the staged
//!   row's flag columns when those are empty; one that cannot be placed stays, with NULL keys, and is
//!   counted. Until `personal-cfo-5ymg8.6` makes the skip a command, `skip_staged` writes no decision.
//! - **committed** and **merged** rows stay, keyed: `transaction_id` is the staged row's
//!   `committed_transaction_id` for a decision that named a staged row, and otherwise the transaction
//!   whose derived decision id the row carries (a decision written by `CommitImportedTransaction` names
//!   no staged row; its id is a content id of the transaction and `committed`, recomputed here).
//!   `identity_id` is the identity `import_identities` holds for that transaction, when migration 76
//!   backfilled one. A committed row that cannot be keyed **stays, with NULL keys, and is counted**: a
//!   migration does not delete committed history it cannot re-key.
//!
//! Like migration 76's backfill this writes class-1 rows outside a command, so the migration is in
//! `CLASS_1_WRITING_MIGRATIONS` and refuses on a vault where Sync is armed; once Sync is user-reachable a
//! backfill of class-1 rows is a command.

use std::collections::HashMap;

use core_ids::DerivedIdKey;
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

/// What the conversion did. Counts only: no id, no text.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct ReshapeReport {
    /// Flagged decisions copied onto their staged row.
    pub flagged_moved: usize,
    /// Flagged decisions whose staged row is gone: dropped.
    pub flagged_orphans: usize,
    /// Skipped decisions that carried nothing the staged row lacks: dropped.
    pub skipped_dropped: usize,
    /// Skipped decisions whose matched transaction was copied onto the staged row's flag columns.
    pub skipped_placed: usize,
    /// Skipped decisions that could not be placed: kept, with NULL keys.
    pub skipped_kept: usize,
    /// Committed or merged decisions kept with a transaction key.
    pub keyed: usize,
    /// Of those, the ones that also got an identity key.
    pub keyed_with_identity: usize,
    /// Committed or merged decisions that could not be keyed: kept, with NULL keys.
    pub unkeyed: usize,
}

struct Legacy {
    id: Uuid,
    source_batch_id: Uuid,
    layer: String,
    staged_transaction_id: Option<Uuid>,
    matched_entity_type: Option<String>,
    matched_entity_id: Option<Uuid>,
    decision: String,
    reason: String,
    decided_at: String,
}

/// The post-step of migration 77.
pub(crate) fn migrate_up(conn: &Connection) -> rusqlite::Result<()> {
    let report = convert(conn)?;
    // Counts only (the redaction rule): what a support session needs to see that a conversion ran.
    tracing::info!(
        flagged_moved = report.flagged_moved,
        flagged_orphans = report.flagged_orphans,
        skipped_dropped = report.skipped_dropped,
        skipped_placed = report.skipped_placed,
        skipped_kept = report.skipped_kept,
        keyed = report.keyed,
        keyed_with_identity = report.keyed_with_identity,
        unkeyed = report.unkeyed,
        "dedupe decisions reshaped"
    );
    conn.execute_batch("DROP TABLE dedupe_decisions_legacy")?;
    Ok(())
}

/// Convert the legacy rows into the new table and the staged columns. Returns what it did.
pub(crate) fn convert(conn: &Connection) -> rusqlite::Result<ReshapeReport> {
    let mut report = ReshapeReport::default();
    let legacy = read_legacy(conn)?;
    if legacy.is_empty() {
        return Ok(report);
    }
    // A fresh vault has no `vault_metadata` row and no decisions; a vault with decisions has both.
    let vault_id: Option<Uuid> = conn
        .query_row(
            "SELECT vault_id FROM vault_metadata WHERE singleton = 1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    // Without the vault id a derived decision id cannot be recomputed: those rows stay unkeyed.
    let derived: HashMap<Uuid, Uuid> = match vault_id {
        Some(vault_id) => derived_decision_ids(conn, &DerivedIdKey::for_vault(&vault_id))?,
        None => HashMap::new(),
    };

    for row in legacy {
        match row.decision.as_str() {
            "flagged" => convert_flagged(conn, &row, &mut report)?,
            "skipped" => convert_skipped(conn, &row, &mut report)?,
            _ => convert_kept(conn, &row, &derived, &mut report)?,
        }
    }
    Ok(report)
}

fn read_legacy(conn: &Connection) -> rusqlite::Result<Vec<Legacy>> {
    let mut stmt = conn.prepare(
        "SELECT id, source_batch_id, layer, staged_transaction_id, matched_entity_type,
                matched_entity_id, decision, reason, decided_at
           FROM dedupe_decisions_legacy ORDER BY decided_at, id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(Legacy {
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
    rows.collect()
}

/// `derived decision id -> transaction id` for every ledger transaction that was created from a
/// source record: the id a `CommitImportedTransaction` gave its `committed` decision (a content id of
/// the transaction and the word `committed`).
fn derived_decision_ids(
    conn: &Connection,
    key: &DerivedIdKey,
) -> rusqlite::Result<HashMap<Uuid, Uuid>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT entity_id FROM source_provenance_links
          WHERE entity_type = 'ledger_transaction' AND relationship = 'created_from'",
    )?;
    let transactions = stmt
        .query_map([], |r| r.get::<_, Uuid>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut map = HashMap::with_capacity(transactions.len());
    for transaction in transactions {
        if let Ok(id) = key.content_id(
            "import.dedupe-decision",
            &[transaction.as_bytes(), b"committed"],
        ) {
            map.insert(id, transaction);
        }
    }
    Ok(map)
}

/// Whether the staged row exists, and whether its flag columns are empty.
fn staged_flag_state(conn: &Connection, staged: Uuid) -> rusqlite::Result<Option<bool>> {
    conn.query_row(
        "SELECT flag_reason IS NULL AND flag_matched_transaction_id IS NULL
           FROM staged_transactions WHERE id = ?1",
        [staged],
        |r| r.get::<_, bool>(0),
    )
    .optional()
}

fn matched_transaction(row: &Legacy) -> Option<Uuid> {
    match (row.matched_entity_type.as_deref(), row.matched_entity_id) {
        (Some("ledger_transaction"), Some(id)) => Some(id),
        _ => None,
    }
}

fn convert_flagged(
    conn: &Connection,
    row: &Legacy,
    report: &mut ReshapeReport,
) -> rusqlite::Result<()> {
    let placed = match row.staged_transaction_id {
        Some(staged) => conn.execute(
            "UPDATE staged_transactions
                SET flag_reason = ?2, flag_matched_transaction_id = ?3
              WHERE id = ?1",
            params![staged, row.reason, matched_transaction(row)],
        )?,
        None => 0,
    };
    if placed == 1 {
        report.flagged_moved += 1;
    } else {
        report.flagged_orphans += 1;
    }
    Ok(())
}

fn convert_skipped(
    conn: &Connection,
    row: &Legacy,
    report: &mut ReshapeReport,
) -> rusqlite::Result<()> {
    let state = match row.staged_transaction_id {
        Some(staged) => staged_flag_state(conn, staged)?,
        None => None,
    };
    match (state, row.matched_entity_id) {
        // Nothing the staged row does not already say: its status is `skipped`.
        (Some(_), None) => {
            report.skipped_dropped += 1;
        }
        // A matched ledger transaction: copy it onto the staged row when its flag columns are empty.
        (Some(true), Some(_)) if matched_transaction(row).is_some() => {
            conn.execute(
                "UPDATE staged_transactions
                    SET flag_reason = ?2, flag_matched_transaction_id = ?3
                  WHERE id = ?1",
                params![
                    row.staged_transaction_id,
                    row.reason,
                    matched_transaction(row)
                ],
            )?;
            report.skipped_placed += 1;
        }
        // Anything else cannot be placed: it stays, unkeyed.
        _ => {
            insert(conn, row, None, None)?;
            report.skipped_kept += 1;
        }
    }
    Ok(())
}

fn convert_kept(
    conn: &Connection,
    row: &Legacy,
    derived: &HashMap<Uuid, Uuid>,
    report: &mut ReshapeReport,
) -> rusqlite::Result<()> {
    let from_staged: Option<Uuid> = match row.staged_transaction_id {
        Some(staged) => conn
            .query_row(
                "SELECT committed_transaction_id FROM staged_transactions WHERE id = ?1",
                [staged],
                |r| r.get(0),
            )
            .optional()?
            .flatten(),
        None => None,
    };
    let transaction = from_staged.or_else(|| derived.get(&row.id).copied());
    let identity: Option<Uuid> = match transaction {
        Some(transaction) => conn
            .query_row(
                "SELECT identity_id FROM import_identities WHERE transaction_id = ?1",
                [transaction],
                |r| r.get(0),
            )
            .optional()?,
        None => None,
    };
    insert(conn, row, identity, transaction)?;
    match (transaction, identity) {
        (Some(_), Some(_)) => {
            report.keyed += 1;
            report.keyed_with_identity += 1;
        }
        (Some(_), None) => report.keyed += 1,
        (None, _) => report.unkeyed += 1,
    }
    Ok(())
}

fn insert(
    conn: &Connection,
    row: &Legacy,
    identity: Option<Uuid>,
    transaction: Option<Uuid>,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO dedupe_decisions
            (id, source_batch_id, layer, identity_id, transaction_id, matched_entity_type,
             matched_entity_id, decision, reason, decided_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            row.id,
            row.source_batch_id,
            row.layer,
            identity,
            transaction,
            row.matched_entity_type,
            row.matched_entity_id,
            row.decision,
            row.reason,
            row.decided_at,
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    //! The conversion of migration 77, driven at the schema before it (76) with rows as the engine
    //! before the reshape wrote them, then migrated.

    use super::*;
    use crate::migrations::{apply, migrate_down, MIGRATIONS};
    use crate::DbError;

    const VAULT: u128 = 0x0198_c2c1_5f0a_7c3d_8a4e_9b1d_2e3f_4a5b;
    const ACCOUNT: u128 = 0xa000;
    const LEDGER_ACCOUNT: u128 = 0xa100;
    const COUNTER: u128 = 0xa200;
    const BATCH: u128 = 0x10;

    fn uid(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn key() -> DerivedIdKey {
        DerivedIdKey::for_vault(&uid(VAULT))
    }

    /// The id `CommitImportedTransaction`'s apply gave its committed decision before the reshape.
    fn derived_decision_id(txn: u128) -> Uuid {
        key()
            .content_id(
                "import.dedupe-decision",
                &[uid(txn).as_bytes(), b"committed"],
            )
            .unwrap()
    }

    /// A vault at schema 76 with a vault id, an account and one source batch.
    fn legacy_vault() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn, &MIGRATIONS[..76]).unwrap();
        conn.execute(
            "INSERT INTO vault_metadata (singleton, vault_id, schema_version, envelope_version,
                 kdf_algorithm, kdf_memory_kib, kdf_time_cost, kdf_parallelism, created_at)
             VALUES (1, ?1, 76, 1, 'argon2id', 1, 1, 1, 't')",
            [uid(VAULT)],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO accounts (id, ledger_account_id, name, cashflow_role, normal_balance, currency)
             VALUES (?1, ?2, 'Checking', 'cash', 'debit', 'USD')",
            params![uid(ACCOUNT), uid(LEDGER_ACCOUNT)],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO source_batches (id, source_type, created_at, updated_at)
             VALUES (?1, 'csv', 't', 't')",
            [uid(BATCH)],
        )
        .unwrap();
        conn
    }

    /// A source record `n` in the batch, a committed ledger transaction `n` created from it, and (when
    /// `staged`) the staged row that committed it, `n + 0x2_0000`.
    fn committed(conn: &Connection, n: u128, staged: bool) {
        conn.execute(
            "INSERT INTO source_records (id, source_batch_id, source_hash, normalized_json, created_at)
             VALUES (?1, ?2, ?3, '{}', 't')",
            params![uid(n), uid(BATCH), format!("h{n}")],
        )
        .unwrap();
        for (account, amount) in [(LEDGER_ACCOUNT, -100), (COUNTER, 100)] {
            conn.execute(
                "INSERT INTO ledger_postings (transaction_id, ledger_account_id, minor_units, currency,
                                              currency_exponent, posting_date)
                 VALUES (?1, ?2, ?3, 'USD', 2, '2026-03-04')",
                params![uid(n), uid(account), amount],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO ledger_transactions (id, operation_id, occurred_at, currency)
             VALUES (?1, ?2, '2026-03-04T00:00:00Z', 'USD')",
            params![uid(n), uid(1)],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO source_provenance_links (id, entity_type, entity_id, source_record_id,
                                                  relationship, created_at)
             VALUES (?1, 'ledger_transaction', ?2, ?3, 'created_from', 't')",
            params![uid(n + 0x1_0000), uid(n), uid(n)],
        )
        .unwrap();
        if staged {
            stage(conn, n, "committed", Some(n));
        }
    }

    /// A staged row `n + 0x2_0000` on source record `n` (a record is created when it has none).
    fn stage(conn: &Connection, n: u128, status: &str, committed_txn: Option<u128>) {
        conn.execute(
            "INSERT OR IGNORE INTO source_records (id, source_batch_id, source_hash, normalized_json,
                                                    created_at)
             VALUES (?1, ?2, ?3, '{}', 't')",
            params![uid(n), uid(BATCH), format!("h{n}")],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO staged_transactions (id, source_record_id, proposed_account_id, posted_at,
                 amount_minor, currency, txn_fingerprint, commit_status, committed_transaction_id,
                 created_at, row_ordinal)
             VALUES (?1, ?2, ?3, '2026-03-04', -100, 'USD', ?4, ?5, ?6, 't', ?7)",
            params![
                uid(n + 0x2_0000),
                uid(n),
                uid(ACCOUNT),
                format!("fp{n}"),
                status,
                committed_txn.map(uid),
                i64::try_from(n).unwrap()
            ],
        )
        .unwrap();
    }

    #[allow(clippy::too_many_arguments)]
    fn legacy_decision(
        conn: &Connection,
        id: Uuid,
        layer: &str,
        staged: Option<u128>,
        matched: Option<(&str, Uuid)>,
        decision: &str,
        reason: &str,
        at: &str,
    ) {
        conn.execute(
            "INSERT INTO dedupe_decisions (id, source_batch_id, layer, staged_transaction_id,
                 matched_entity_type, matched_entity_id, decision, reason, decided_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                id,
                uid(BATCH),
                layer,
                staged.map(|n| uid(n + 0x2_0000)),
                matched.map(|(t, _)| t),
                matched.map(|(_, id)| id),
                decision,
                reason,
                at
            ],
        )
        .unwrap();
    }

    type Row = (
        Uuid,
        Option<Uuid>,
        Option<Uuid>,
        String,
        String,
        Option<String>,
    );

    fn decisions(conn: &Connection) -> Vec<Row> {
        let mut stmt = conn
            .prepare(
                "SELECT id, transaction_id, identity_id, decision, reason, matched_entity_type
                   FROM dedupe_decisions ORDER BY id",
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
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
    }

    fn flags(conn: &Connection) -> Vec<(Uuid, Option<String>, Option<Uuid>)> {
        let mut stmt = conn
            .prepare(
                "SELECT id, flag_reason, flag_matched_transaction_id FROM staged_transactions
                  ORDER BY id",
            )
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    /// The rows as the engine before the reshape wrote them, one of each kind.
    fn populate(conn: &Connection) {
        // Committed through `CommitImportedTransaction`: no staged id, a derived id; its transaction 1
        // has an identity from migration 76.
        committed(conn, 1, false);
        conn.execute(
            "INSERT INTO import_identities (identity_id, kind, file_fingerprint, row_ordinal,
                 transaction_id, account_id, amount_minor, currency, posted_date, created_at)
             VALUES (?1, 'file', zeroblob(32), 1, ?2, ?3, -100, 'USD', '2026-03-04', 't')",
            params![uid(0x9001), uid(1), uid(ACCOUNT)],
        )
        .unwrap();
        legacy_decision(
            conn,
            derived_decision_id(1),
            "transaction",
            None,
            None,
            "committed",
            "unique",
            "2026-03-04T01",
        );
        // Committed through the retired `CommitStaged`: names its staged row, whose
        // committed_transaction_id is the key.
        committed(conn, 2, true);
        legacy_decision(
            conn,
            uid(0x8002),
            "transaction",
            Some(2),
            None,
            "committed",
            "unique",
            "2026-03-04T02",
        );
        // A committed decision nothing can key: no staged row and an id that is no derived id.
        legacy_decision(
            conn,
            uid(0x8003),
            "transaction",
            None,
            None,
            "committed",
            "unique",
            "2026-03-04T03",
        );
        // A flagged decision moves onto its staged row.
        stage(conn, 4, "flagged", None);
        legacy_decision(
            conn,
            uid(0x8004),
            "transaction",
            Some(4),
            Some(("ledger_transaction", uid(1))),
            "flagged",
            "duplicate of an already-committed transaction",
            "2026-03-04T04",
        );
        // Skipped: nothing the staged row lacks (dropped); a matched ledger transaction (copied onto the
        // empty flag columns); a matched source batch (cannot be placed: kept); no staged row (kept).
        stage(conn, 6, "skipped", None);
        legacy_decision(
            conn,
            uid(0x8006),
            "transaction",
            Some(6),
            None,
            "skipped",
            "user skipped",
            "2026-03-04T06",
        );
        stage(conn, 7, "skipped", None);
        legacy_decision(
            conn,
            uid(0x8007),
            "transaction",
            Some(7),
            Some(("ledger_transaction", uid(2))),
            "skipped",
            "user skipped",
            "2026-03-04T07",
        );
        stage(conn, 8, "skipped", None);
        legacy_decision(
            conn,
            uid(0x8008),
            "transaction",
            Some(8),
            Some(("source_batch", uid(BATCH))),
            "skipped",
            "re-upload",
            "2026-03-04T08",
        );
        legacy_decision(
            conn,
            uid(0x8009),
            "file",
            None,
            Some(("source_batch", uid(BATCH))),
            "skipped",
            "re-upload",
            "2026-03-04T09",
        );
    }

    fn committed_row(id: Uuid, txn: Option<Uuid>, identity: Option<Uuid>) -> Row {
        (
            id,
            txn,
            identity,
            "committed".to_owned(),
            "unique".to_owned(),
            None,
        )
    }

    #[test]
    fn every_kind_of_legacy_decision_is_converted_keyed_moved_or_dropped_and_counted() {
        // The counts, from the conversion run on the migration's own SQL.
        let conn = legacy_vault();
        populate(&conn);
        // A flagged decision whose staged row is gone (only possible with foreign keys off).
        conn.execute_batch("PRAGMA foreign_keys = OFF").unwrap();
        legacy_decision(
            &conn,
            uid(0x8005),
            "transaction",
            Some(99),
            None,
            "flagged",
            "orphan",
            "2026-03-04T05",
        );
        conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();
        conn.execute_batch(MIGRATIONS[76].up).unwrap();
        assert_eq!(
            convert(&conn).unwrap(),
            ReshapeReport {
                flagged_moved: 1,
                flagged_orphans: 1,
                skipped_dropped: 1,
                skipped_placed: 1,
                skipped_kept: 2,
                keyed: 2,
                keyed_with_identity: 1,
                unkeyed: 1,
            }
        );

        // The rows, through the migration itself.
        let mut conn = legacy_vault();
        populate(&conn);
        apply(&mut conn, MIGRATIONS).unwrap();
        let mut expected = vec![
            // Keyed through its staged row's committed transaction.
            committed_row(uid(0x8002), Some(uid(2)), None),
            // Nothing can key it, so it STAYS with NULL keys (no table CHECK demands a key).
            committed_row(uid(0x8003), None, None),
            // Keyed through its derived id, and by the identity migration 76 wrote for it.
            committed_row(derived_decision_id(1), Some(uid(1)), Some(uid(0x9001))),
            // A matched source batch cannot be placed on a staged row: kept, unkeyed.
            (
                uid(0x8008),
                None,
                None,
                "skipped".to_owned(),
                "re-upload".to_owned(),
                Some("source_batch".to_owned()),
            ),
            (
                uid(0x8009),
                None,
                None,
                "skipped".to_owned(),
                "re-upload".to_owned(),
                Some("source_batch".to_owned()),
            ),
        ];
        expected.sort_by_key(|r| r.0);
        assert_eq!(decisions(&conn), expected);
        // The flag moved onto its staged row with the matched transaction; the placed skip copied its
        // reason and matched transaction onto the empty flag columns.
        let flagged: Vec<_> = flags(&conn)
            .into_iter()
            .filter(|(_, reason, _)| reason.is_some())
            .collect();
        assert_eq!(
            flagged,
            vec![
                (
                    uid(4 + 0x2_0000),
                    Some("duplicate of an already-committed transaction".to_owned()),
                    Some(uid(1))
                ),
                (
                    uid(7 + 0x2_0000),
                    Some("user skipped".to_owned()),
                    Some(uid(2))
                ),
            ]
        );
        // The legacy table is gone, and nothing names a staging row any more.
        let left: i64 = conn
            .query_row(
                "SELECT (SELECT COUNT(*) FROM sqlite_master WHERE name = 'dedupe_decisions_legacy')
                      + (SELECT COUNT(*) FROM pragma_table_info('dedupe_decisions')
                          WHERE name = 'staged_transaction_id')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(left, 0);
    }

    #[test]
    fn the_table_takes_no_flagged_row_and_asks_for_no_key() {
        let mut conn = legacy_vault();
        apply(&mut conn, MIGRATIONS).unwrap();
        let insert = |decision: &str| {
            conn.execute(
                "INSERT INTO dedupe_decisions (id, source_batch_id, layer, decision, reason, decided_at)
                 VALUES (randomblob(16), ?1, 'transaction', ?2, 'r', 't')",
                params![uid(BATCH), decision],
            )
        };
        assert!(insert("flagged").is_err(), "a flag is the staged row's");
        assert!(insert("nope").is_err());
        for decision in ["committed", "skipped", "merged"] {
            assert!(insert(decision).is_ok(), "{decision}, with NULL keys");
        }
    }

    #[test]
    fn an_armed_vault_refuses_migration_77_and_stays_at_76() {
        for sql in [
            "INSERT INTO sync_base (singleton, base_seq, state, updated_at)
             VALUES (1, 0, 'NoBase', 't')",
            "INSERT INTO sync_genesis (singleton, vault_id, membership_record, epoch_record, state,
                                       persisted_at, updated_at)
             VALUES (1, zeroblob(16), x'01', x'02', 'Persisted', 't', 't')",
        ] {
            let mut conn = legacy_vault();
            populate(&conn);
            conn.execute(sql, []).unwrap();
            let error = apply(&mut conn, MIGRATIONS).unwrap_err();
            assert!(
                matches!(
                    error,
                    DbError::MigrationRefusedSyncArmed { version: 77, .. }
                ),
                "{error:?}"
            );
            let latest: i64 = conn
                .query_row("SELECT MAX(version) FROM schema_migrations", [], |r| {
                    r.get(0)
                })
                .unwrap();
            assert_eq!(latest, 76);
            // Nothing was converted: the legacy shape and its rows are intact.
            let rows: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM dedupe_decisions WHERE staged_transaction_id IS NOT NULL",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(rows, 5);
        }
    }

    #[test]
    fn down_rebuilds_the_old_shape_and_up_again_gives_the_same_rows() {
        let mut conn = legacy_vault();
        populate(&conn);
        apply(&mut conn, MIGRATIONS).unwrap();
        let (rows, flagged) = (decisions(&conn), flags(&conn));

        assert_eq!(migrate_down(&mut conn, 76).unwrap(), 1);
        // The old shape: a flag is a decision again (its id is the staged row's), and a committed
        // row names the staged row that committed its transaction.
        let (staged_flagged, with_staged): (i64, i64) = conn
            .query_row(
                "SELECT (SELECT COUNT(*) FROM dedupe_decisions WHERE decision = 'flagged'),
                        (SELECT COUNT(*) FROM dedupe_decisions
                          WHERE decision = 'committed' AND staged_transaction_id IS NOT NULL)",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((staged_flagged, with_staged), (2, 1));

        apply(&mut conn, MIGRATIONS).unwrap();
        assert_eq!(decisions(&conn), rows, "the same decisions");
        assert_eq!(flags(&conn), flagged, "and the same flags");
    }

    /// The Money Inbox reads the staged columns now (C6), and shows what it showed before: the old
    /// generator's two subqueries (the flagged decision's reason, and its matched counterpart, else the
    /// committed row with the same fingerprint in the same account) are the oracle, run on the legacy
    /// vault before the migration; the new generator's items, after it, carry the same reason and
    /// counterpart.
    #[test]
    fn the_money_inbox_shows_the_same_reason_and_counterpart_before_and_after() {
        let mut conn = legacy_vault();
        // Transaction 1 and 2 are committed; flagged rows: one whose decision names transaction 1,
        // one whose decision names nothing (a row the ledger cannot accept), and one flagged before
        // decisions named a counterpart, found by the fingerprint of transaction 2's staged row.
        committed(&conn, 1, true);
        committed(&conn, 2, true);
        stage(&conn, 4, "flagged", None);
        legacy_decision(
            &conn,
            uid(0x8004),
            "transaction",
            Some(4),
            Some(("ledger_transaction", uid(1))),
            "flagged",
            "duplicate of an already-committed transaction",
            "2026-03-04T04",
        );
        stage(&conn, 5, "flagged", None);
        legacy_decision(
            &conn,
            uid(0x8005),
            "transaction",
            Some(5),
            None,
            "flagged",
            "not imported: the amount is zero",
            "2026-03-04T05",
        );
        stage(&conn, 9, "flagged", None);
        conn.execute(
            "UPDATE staged_transactions SET txn_fingerprint = 'fp2' WHERE id = ?1",
            [uid(9 + 0x2_0000)],
        )
        .unwrap();
        legacy_decision(
            &conn,
            uid(0x8009),
            "transaction",
            Some(9),
            None,
            "flagged",
            "same date and amount already recorded from another source",
            "2026-03-04T09",
        );

        type Seen = Vec<(Uuid, Option<String>, Option<Uuid>)>;
        let before: Seen = conn
            .prepare(
                "SELECT s.id,
                        (SELECT d.reason FROM dedupe_decisions d
                          WHERE d.staged_transaction_id = s.id AND d.decision = 'flagged'
                          ORDER BY d.decided_at DESC LIMIT 1),
                        COALESCE(
                          (SELECT d.matched_entity_id FROM dedupe_decisions d
                            JOIN ledger_transactions lt ON lt.id = d.matched_entity_id
                            WHERE d.staged_transaction_id = s.id AND d.decision = 'flagged'
                              AND d.matched_entity_type = 'ledger_transaction'
                              AND lt.voided_at IS NULL
                            ORDER BY d.decided_at DESC LIMIT 1),
                          (SELECT c.committed_transaction_id FROM staged_transactions c
                            JOIN ledger_transactions lt ON lt.id = c.committed_transaction_id
                            WHERE c.txn_fingerprint = s.txn_fingerprint
                              AND c.proposed_account_id IS s.proposed_account_id
                              AND c.commit_status = 'committed'
                              AND lt.voided_at IS NULL
                            ORDER BY c.created_at, c.id LIMIT 1))
                   FROM staged_transactions s WHERE s.commit_status = 'flagged' ORDER BY s.id",
            )
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(before.len(), 3);
        assert!(before.iter().all(|row| row.1.is_some()));
        assert_eq!(before[0].2, Some(uid(1)), "named by the decision");
        assert_eq!(before[1].2, None, "nothing to name");
        assert_eq!(before[2].2, Some(uid(2)), "found by the fingerprint");

        apply(&mut conn, MIGRATIONS).unwrap();
        crate::money_inbox::rebuild_in(&conn, "t").unwrap();
        let after: Seen = conn
            .prepare(
                "SELECT target_id, payload_json FROM money_inbox_read_model
                  WHERE item_kind = 'imported_waiting_commit' ORDER BY target_id",
            )
            .unwrap()
            .query_map([], |r| {
                let payload: String = r.get(1)?;
                let value: serde_json::Value = serde_json::from_str(&payload).unwrap();
                let reason = value["dedupe_reason"].as_str().map(str::to_owned);
                let counterpart = value["suspected_committed_txn_id"]
                    .as_str()
                    .map(|text| text.parse().unwrap());
                Ok((r.get(0)?, reason, counterpart))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(after, before, "the same items, reasons and counterparts");
    }
}
