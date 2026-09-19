//! Compile-time [`SourcePreset`](importer_core::SourcePreset) registrations
//! for migrating from other personal-finance apps (personal-cfo-gvidg). One
//! module per source app; each is a set of hints over the generic CSV
//! importer, not a new parser — most personal-finance apps export CSV with
//! stable headers, so for most sources a preset is the whole job.
//!
//! Registering a preset here makes it discoverable via
//! [`importer_core::all_presets`]/[`importer_core::preset_by_id`] anywhere
//! this crate is linked; it carries no DB/network/IPC dependency itself
//! (pure, same rule `csv-importer`/`ofx-importer` already follow).

pub mod ynab;

#[cfg(test)]
mod harness;
