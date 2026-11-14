//! The class-1b closure check before a swap (personal-cfo-bxsbz.6; ADR 0074 decision 2).
//!
//! Class-1b objects (import provenance: source batches, source records, parser runs, and the
//! provenance links that tie them to class-1 rows) are carried as artifacts under the closure rule.
//! A rebuilt vault must not be swapped in while a reference of its class-1 or class-1b state names an
//! object the vault does not hold: the person would lose the provenance of a committed row. Before
//! the swap, [`Scratch::check_class_1b_closure`] looks for every such reference. One that the scratch
//! vault cannot resolve is looked up in the **retained closure or the confirmed pin** through a
//! [`ClosureView`]; one that neither covers is [`RebaseError::ArtifactRebootstrapRequired`] with the
//! live vault untouched and the scratch vault kept as the evidence. The only way out is a
//! pull-first re-bootstrap.
//!
//! **The seam.** The retained closure and the confirmed pin are the artifact receipts' (the Sync
//! artifact store, personal-cfo-1cltt.7), which this crate does not have yet. [`ClosureView`] is the
//! question the rebase asks of them; [`NoRetainedClosure`] answers "nothing is covered" and is the
//! safe default: it can only make the check stricter.
//!
//! **What is checked.** Each reference below is read from the scratch vault, never from the live one:
//! a provenance link's source record, a source record's or parser run's batch, and the class-1 entity
//! a provenance link names (a transaction, an account or a balance observation). Attachment
//! blobs are not covered here: the `attachments` metadata is still a class-4 candidate written
//! outside the command bus (personal-cfo-bcj, SYNC-2), so there is no replicated reference to check.

use std::path::PathBuf;

use rusqlite::Connection;
use uuid::Uuid;

use super::overlay::Offending;
use super::{RebaseError, Scratch};

/// A class-1b object (or the class-1 row a provenance link needs) that a reference names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ClassOneBObject {
    /// A source batch.
    SourceBatch(Uuid),
    /// A source record.
    SourceRecord(Uuid),
    /// The class-1 entity a provenance link names, by the link's `entity_type`.
    Entity {
        /// The `source_provenance_links.entity_type` value.
        kind: &'static str,
        /// The entity's id.
        id: Uuid,
    },
}

/// What the retained closure and the confirmed pin cover. The question the rebase asks of the Sync
/// artifact store before a swap.
pub trait ClosureView {
    /// Whether the retained closure or the confirmed pin covers `object`.
    fn covers(&self, object: &ClassOneBObject) -> bool;
}

/// A view that covers nothing: the safe default until the artifact store supplies a real one.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoRetainedClosure;

impl ClosureView for NoRetainedClosure {
    fn covers(&self, _object: &ClassOneBObject) -> bool {
        false
    }
}

/// Which kind of object is missing, and how many references name one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingObjects {
    /// The kind, as a stable word (`source_record`, `source_batch`, or the entity type).
    pub kind: &'static str,
    /// The references that name a missing object, as counts and tags.
    pub rows: Offending,
}

/// The evidence of [`RebaseError::ArtifactRebootstrapRequired`]: nothing in it is a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactRebootstrapRequired {
    /// What is missing.
    pub missing: Vec<MissingObjects>,
    /// The scratch vault, kept for diagnosis.
    pub scratch: PathBuf,
}

impl ArtifactRebootstrapRequired {
    /// A sentence that names what is missing, never a value.
    #[must_use]
    pub fn describe(&self) -> String {
        let kinds: Vec<String> = self
            .missing
            .iter()
            .map(|m| format!("{} {}", m.rows.count, m.kind))
            .collect();
        format!(
            "the rebuilt vault needs objects that neither the retained closure nor the confirmed pin covers ({}): re-bootstrap",
            kinds.join(", ")
        )
    }
}

/// What a passing check found.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClosureReport {
    /// References the scratch vault could not resolve itself and the view covered.
    pub covered_by_view: usize,
}

/// `(link entity_type, the table that holds the entity)`. A posting has no id of its own (it is a row
/// of its transaction), so a `ledger_posting` link has nothing to resolve here.
const ENTITIES: &[(&str, &str)] = &[
    ("ledger_transaction", "ledger_transactions"),
    ("account", "accounts"),
    ("balance_observation", "balance_observations"),
];

impl Scratch<'_> {
    /// Check, before a swap, that every class-1b reference of the rebuilt vault resolves, or is
    /// covered by `view`. Read-only.
    ///
    /// # Errors
    /// [`RebaseError::ArtifactRebootstrapRequired`] or [`RebaseError::Db`].
    pub fn check_class_1b_closure(
        &self,
        view: &dyn ClosureView,
    ) -> Result<ClosureReport, RebaseError> {
        let guard = self.worker.lock();
        let mut report = ClosureReport::default();
        let mut missing = Vec::new();
        let mut note = |kind: &'static str, objects: Vec<ClassOneBObject>| {
            let uncovered: Vec<Vec<u8>> = objects
                .iter()
                .filter(|object| {
                    let covered = view.covers(object);
                    if covered {
                        report.covered_by_view += 1;
                    }
                    !covered
                })
                .map(|object| id_of(object).as_bytes().to_vec())
                .collect();
            if !uncovered.is_empty() {
                missing.push(MissingObjects {
                    kind,
                    rows: Offending::from_keys(&uncovered),
                });
            }
        };

        note(
            "source_record",
            unresolved(
                &guard.conn,
                "SELECT DISTINCT l.source_record_id FROM source_provenance_links l
                  WHERE NOT EXISTS (SELECT 1 FROM source_records r WHERE r.id = l.source_record_id)",
                ClassOneBObject::SourceRecord,
            )?,
        );
        note(
            "source_batch",
            unresolved(
                &guard.conn,
                "SELECT source_batch_id FROM source_records
                  WHERE NOT EXISTS (SELECT 1 FROM source_batches b WHERE b.id = source_batch_id)
                 UNION
                 SELECT source_batch_id FROM parser_runs
                  WHERE NOT EXISTS (SELECT 1 FROM source_batches b WHERE b.id = source_batch_id)",
                ClassOneBObject::SourceBatch,
            )?,
        );
        for (kind, table) in ENTITIES {
            note(
                kind,
                unresolved(
                    &guard.conn,
                    &format!(
                        "SELECT DISTINCT l.entity_id FROM source_provenance_links l
                          WHERE l.entity_type = '{kind}'
                            AND NOT EXISTS (SELECT 1 FROM {table} t WHERE t.id = l.entity_id)"
                    ),
                    |id| ClassOneBObject::Entity { kind, id },
                )?,
            );
        }

        if missing.is_empty() {
            Ok(report)
        } else {
            Err(RebaseError::ArtifactRebootstrapRequired(Box::new(
                ArtifactRebootstrapRequired {
                    missing,
                    scratch: self.path.clone(),
                },
            )))
        }
    }
}

fn id_of(object: &ClassOneBObject) -> Uuid {
    match object {
        ClassOneBObject::SourceBatch(id)
        | ClassOneBObject::SourceRecord(id)
        | ClassOneBObject::Entity { id, .. } => *id,
    }
}

fn unresolved(
    conn: &Connection,
    sql: &str,
    make: impl Fn(Uuid) -> ClassOneBObject,
) -> Result<Vec<ClassOneBObject>, rusqlite::Error> {
    let mut stmt = conn.prepare(sql)?;
    let ids = stmt
        .query_map([], |r| r.get::<_, Uuid>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ids.into_iter().map(make).collect())
}
