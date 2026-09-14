//! The IPC boundary between the React frontend and the Finance Kernel
//! (personal-cfo-40t).
//!
//! - [`commands`] — the typed `#[tauri::command]` surface (plus `*_impl`
//!   functions for testing).
//! - [`dto`] — serde + `specta::Type` wire types and their conversions.
//! - [`error::IpcError`] — the single, non-leaky error type the frontend sees.

pub mod commands;
pub mod dto;
pub mod error;

pub use error::IpcError;
