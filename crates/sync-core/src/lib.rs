//! Pure format-level building blocks for DohFlow Sync (personal-cfo-t2s4b).
//!
//! This crate has no database, no async runtime, no UI and no filesystem access: a Sync
//! server can build it without SQLCipher, and CI enforces that it stays that way (it must
//! not depend on `rusqlite`, `tokio` or `tauri`).
//!
//! * [`canonical_json`]: the strict canonical JSON codec of `docs/architecture/vault-format.md`
//!   C.1.2 (restricted RFC 8785), shared by command payloads, manifests and logical
//!   metadata.
//! * [`accepted`]: the C.10 accepted-envelope record, the service's push decision and the
//!   client's outbox binding.
//! * [`cbe`]: CBE1, the canonical binary encoding of C.1.1 (strict writer and reader).
//! * [`header`]: the C.4 clear header (the AEAD's AAD) and the `DFSY` container.
//! * [`keys`]: the exact Ed25519 and X25519 key acceptance rules of C.6.
//! * [`membership`]: the C.6 membership chain and epoch records and their reference verifier.
//! * [`producer`]: the seal-then-verify rule a client runs before it uploads an envelope.
//! * [`service`]: the checks the Sync service can make on a push without reading it.
//! * [`status`]: the replica's sync status (healthy, halted, or stopped at an invalid envelope).
//! * [`envelope`]: the envelope plaintext, its commitments and digest, and the receiver checks.

pub mod accepted;
pub mod canonical_json;
pub mod cbe;
pub mod envelope;
pub mod header;
pub mod keys;
pub mod membership;
pub mod producer;
pub mod row_digest;
pub mod service;
pub mod status;
pub mod void;
pub mod write_set;
