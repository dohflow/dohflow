# Contributing an importer plugin

Importers turn an exported file (CSV/OFX/QFX/QIF today; more later) into typed
**staged candidates**. They never write the ledger directly — the pipeline
(`personal-cfo-cmx`) stages, dedupes, and commits what a plugin returns.

## The contract

Implement [`ImporterPlugin`](src/lib.rs) on a **stateless unit struct** and
register it at compile time:

```rust
use importer_core::{register_importer, ImporterPlugin, ParseError, ParsedBatch, ParserHints, ParserInput};
use semver::Version;

struct AcmeCsv;

impl ImporterPlugin for AcmeCsv {
    fn id(&self) -> &'static str { "acme-csv" }          // stable, unique, forever
    fn display_name(&self) -> &'static str { "ACME Bank CSV" }
    fn version(&self) -> Version { Version::new(1, 0, 0) }
    fn supported_extensions(&self) -> &'static [&'static str] { &["csv"] }
    fn detect_confidence(&self, input: &ParserInput) -> u16 { /* 0..=10000 bps */ 0 }
    fn parse(&self, input: &ParserInput, hints: &ParserHints) -> Result<ParsedBatch, ParseError> {
        // bytes in → typed staged candidates out
        todo!()
    }
}

register_importer!(AcmeCsv);
```

`register_importer!` records the plugin in a compile-time `inventory` set — there
is **no runtime registration** to wire up, and nothing to mistype. The roster is
fixed when the binary builds.

## Rules (enforced in CI)

- **No runtime dynamic loading.** `libloading`, `dlopen`, `dlsym`, `dlfcn` are
  forbidden anywhere under `crates/importers/` (ADR 0022). Registration is static.
- **No keys, DB, network, or IPC.** A plugin receives bytes and returns records —
  nothing more. Importer crates may not depend on `tokio`, `tauri`, or `rusqlite`
  (checked via the resolved dependency tree).
- **Untrusted input.** Treat every byte as hostile. Bound your own work
  (max rows, etc.) and fail with [`ParseError::LimitExceeded`] rather than
  allocating without limit — the isolation harness (`personal-cfo-hs9`) also
  enforces hard size/memory/time bounds around `parse`.
- **Never persist raw bytes.** Return `source_hash` + `normalized_json`; the
  uploaded file is shredded after parsing (ADR 0014 §4).
- **`id` + `version`** are recorded on every `parser_run`. Bump `version` when
  parsing behavior changes; never reuse or rename an `id`.

## What to return

A [`ParsedBatch`](src/lib.rs) maps onto the `ihe` staging schema:

- `accounts: Vec<ParsedAccount>` → `staged_accounts`
- each `ParsedRecord` → a `source_record` (+ an optional `ParsedTransaction` →
  `staged_transaction`, or `ParsedBalance` → `staged_balance`)
- `warnings` → surfaced to the user for the ambiguous minority

Preserve the raw date string and a confidence alongside the parsed date; emit a
`ParseWarning` for anything ambiguous rather than guessing silently.

## Checklist for a new plugin PR

- [ ] One crate under `crates/importers/<name>/`, added to the workspace `members`.
- [ ] `ImporterPlugin` impl + `register_importer!`.
- [ ] Snapshot/integration tests over real (sanitized) fixtures — negatives,
      parentheses, thousands separators, non-USD, duplicates, malformed rows.
- [ ] `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`,
      `cargo fmt --check` all green.
