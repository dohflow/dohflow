//! Domain-grouped `apply_command` arm bodies (personal-cfo-3fdd.2).
//!
//! Each submodule holds the write logic for one command family, lifted
//! verbatim from the arm bodies in `lib.rs`. `apply_command` keeps the
//! single exhaustive match; every arm is a one-call delegation into one
//! of these `pub(crate)` functions, which return the affected entity id
//! for provenance.

pub(crate) mod accounts;
pub(crate) mod categorization_cmds;
pub(crate) mod household;
pub(crate) mod inbox;
pub(crate) mod ingestion_cmds;
pub(crate) mod recurring;
pub(crate) mod scenario_apply;
pub(crate) mod transactions;

use chrono::{DateTime, Utc};

use crate::CommandMeta;

/// The instant the command was issued. `apply` reads this, never the wall clock, so a replica
/// applying the command later stamps the same instant (ADR 0013 Addendum A; SYNC-1a). The value
/// is range-checked when the command is accepted, so this cannot fail.
pub(crate) fn issued_at(meta: &CommandMeta) -> DateTime<Utc> {
    DateTime::from_timestamp_micros(meta.issued_at.as_unix_micros())
        .expect("issued_at is range-checked by CommandMeta::validate")
}

/// [`issued_at`] as the text stored in `created_at` / `updated_at` style columns.
pub(crate) fn issued_at_text(meta: &CommandMeta) -> String {
    issued_at(meta).to_rfc3339()
}
