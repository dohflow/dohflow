//! Domain-grouped `apply_command` arm bodies (personal-cfo-3fdd.2).
//!
//! Each submodule holds the write logic for one command family, lifted
//! verbatim from the arm bodies in `lib.rs`. `apply_command` keeps the
//! single exhaustive match; every arm is a one-call delegation into one
//! of these `pub(crate)` functions, which return the affected entity id
//! for provenance.

pub(crate) mod accounts;
pub(crate) mod balance_cmds;
pub(crate) mod categorization_cmds;
pub(crate) mod connector_cmds;
pub(crate) mod household;
pub(crate) mod inbox;
pub(crate) mod ingestion_cmds;
pub(crate) mod recurring;
pub(crate) mod scenario_apply;
pub(crate) mod transactions;

use chrono::{DateTime, Utc};

use crate::{CommandMeta, DbError};

/// Refuse a command whose `issued_at` has no calendar date. `apply_command` calls this FIRST,
/// before it touches the connection, so a command from another device (whose `issued_at` is any
/// `i64` of microseconds) fails with a deterministic `InvalidCommand` and never reaches
/// [`issued_at`], which would panic.
pub(crate) fn require_issued_at(meta: &CommandMeta) -> Result<(), DbError> {
    if DateTime::from_timestamp_micros(meta.issued_at.as_unix_micros()).is_none() {
        return Err(DbError::InvalidCommand(
            "issued_at is outside the supported date range".to_owned(),
        ));
    }
    Ok(())
}

/// The instant the command was issued. `apply` reads this, never the wall clock, so a replica
/// applying the command later stamps the same instant (ADR 0013 Addendum A; SYNC-1a).
///
/// Every `apply` function is reached only through `apply_command`, which calls
/// [`require_issued_at`] first, so the conversion cannot fail here.
pub(crate) fn issued_at(meta: &CommandMeta) -> DateTime<Utc> {
    DateTime::from_timestamp_micros(meta.issued_at.as_unix_micros())
        .expect("apply_command checks issued_at with require_issued_at before any apply function")
}

/// [`issued_at`] as the text stored in `created_at` / `updated_at` style columns.
pub(crate) fn issued_at_text(meta: &CommandMeta) -> String {
    issued_at(meta).to_rfc3339()
}
