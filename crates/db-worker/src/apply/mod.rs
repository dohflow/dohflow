//! Domain-grouped `apply_command` arm bodies (personal-cfo-3fdd.2).
//!
//! Each submodule holds the write logic for one command family, lifted
//! verbatim from the arm bodies in `lib.rs`. `apply_command` keeps the
//! single exhaustive match; every arm is a one-call delegation into one
//! of these `pub(crate)` functions, which return the affected entity id
//! for provenance.

pub(crate) mod accounts;
pub(crate) mod categorization_cmds;
pub(crate) mod inbox;
pub(crate) mod ingestion_cmds;
pub(crate) mod recurring;
pub(crate) mod scenario_apply;
pub(crate) mod transactions;
