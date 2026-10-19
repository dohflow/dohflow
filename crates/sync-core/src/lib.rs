//! Pure format-level building blocks for DohFlow Sync (personal-cfo-t2s4b).
//!
//! This crate has no database, no async runtime, no UI and no filesystem access: a Sync
//! server can build it without SQLCipher, and CI enforces that it stays that way (it must
//! not depend on `rusqlite`, `tokio` or `tauri`).
//!
//! * [`canonical_json`]: the strict canonical JSON codec of `docs/architecture/vault-format.md`
//!   C.1.2 (restricted RFC 8785), shared by command payloads, manifests and logical
//!   metadata.

pub mod canonical_json;
