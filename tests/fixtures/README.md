# tests/fixtures

Shared test fixtures for the workspace — primarily the **golden synthetic
fixture vault** that integration and migration tests run against.

## Rules

- **No data is committed here.** Fixtures are generated on demand by the
  synthetic-data generator (bead `personal-cfo-9ujs`, under
  `tests/synthetic-data/`) or by per-test setup code.
- **Never commit real financial data**, real vault files, or local databases.
  `.gitignore` blocks `*.db` and `*.vault`; do not override it.
- Fixtures must be **deterministic** so snapshot and golden-vault tests are
  reproducible.

Cross-version golden vaults (vaults created by each prior pinned SQLCipher
version) are exercised by `crates/finance-kernel/tests/cross_version_vault.rs`
(`personal-cfo-7igv`) and the migration tests (`personal-cfo-c545`). Today's
single pinned engine is generated on demand by that test; when the engine is
bumped, the outgoing version's vault is captured and added to the corpus as
base64 text — never a committed `.db`. See the version-pin policy in
`docs/architecture/stack.md`.
