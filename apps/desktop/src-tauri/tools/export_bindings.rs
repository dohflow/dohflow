//! Generate the frontend TypeScript bindings from the Rust IPC surface
//! (personal-cfo-40t).
//!
//! Run from the `src-tauri` crate directory:
//!
//! ```sh
//! cargo run --bin export_bindings
//! ```
//!
//! This writes `apps/desktop/src/bindings.ts`. The file is committed; CI
//! regenerates it and fails if the working tree diverges, so the Rust and
//! TypeScript schemas can never silently drift.

const BINDINGS_PATH: &str = "../src/bindings.ts";

fn main() {
    match app_lib::export_bindings(BINDINGS_PATH) {
        Ok(()) => println!("wrote {BINDINGS_PATH}"),
        Err(error) => {
            eprintln!("failed to export bindings: {error}");
            std::process::exit(1);
        }
    }
}
