# Definition of Done

This document is the canonical source of test, logging, and CI obligations for every feature in DohFlow. Bead acceptance criteria reference this file by path (`docs/architecture/definition-of-done.md`) rather than restating its contents.

A feature is **not done** until every applicable item below is satisfied on the golden synthetic fixture vault.

Tracked by bead [`personal-cfo-3hsx`](../../.beads/issues.jsonl).

---

## 1. Required test layers

Each feature ships with the layers that apply to it. "Applies" means the layer can meaningfully exercise the feature; if it cannot, document the reason in the feature's bead notes.

### 1.1 Unit tests

- Pure functions and small structs are covered by table-driven cases including edge cases, boundary values, and adversarial inputs.
- Use `proptest` (Rust) and fast-check style generators where invariants are obvious — money math, schedules, categorization, rules, balance sums.
- Frontend unit tests use Vitest + Testing Library for pure components and hooks.

### 1.2 Integration tests

- Exercise the feature against a **real SQLCipher fixture vault**, the **real Finance Kernel command bus**, and the **real read-model refresh path**. No mocked DB, no mocked kernel.
- Time is mocked **only via an injected `Clock`**. Wall-clock dependencies break determinism and are forbidden in test paths.
- Vault state for integration tests is set up by importing a synthetic persona fixture (`tests/fixtures/personas/<name>/`) — never by ad-hoc INSERTs.

### 1.3 End-to-end tests

- Drive the feature through the same Tauri IPC entry points the UI uses.
  - Backend-only flows: Rust-side IPC harness (bead `personal-cfo-9s1e`).
  - UI flows: Playwright/WebDriver-style harness against a built Tauri binary (bead `personal-cfo-oymv`).
- Cover at least one happy path and two failure paths (invalid input, vault locked / wrong password, etc.).
- Failure paths assert on the **typed error variant** the user sees, not on string-matched messages.

### 1.4 Snapshot tests (`insta`)

- Required for any feature that produces deterministic structured output: forecasts, agent reports, ledger projections, redacted exports.
- Snapshots are reviewed on update — drive-by snapshot acceptance is forbidden.

### 1.5 Property tests

- Required for the §20.1 invariants: ledger postings balance, splits sum to transaction total, transfer parity, money-math associativity, schedule generators (DST/timezone), categorization stability under input perturbation.

---

## 2. Required logging instrumentation

### 2.1 Structured tracing spans

- Every Tauri command, Finance Kernel command, importer, connector, agent invocation, and projection runner emits a `tracing` span with: `command_id`, `correlation_id`, `causation_id`, `actor_type`, `actor_id`, duration, outcome (success / typed error variant).
- Spans are emitted at the boundary, not deep in the call stack — one span per logical operation.

### 2.2 Redaction policy compliance

- All captured spans pass the cross-cutting redaction CI suite (`personal-cfo-q5ko`, `-64st`, `-mt6s`, `-zobt`).
- The §6.6 known-positive corpus (account numbers, balances, merchant names, paystub fields, password fragments, attachment paths, etc.) must NOT survive redaction.
- The known-negative corpus (innocuous strings, system identifiers, schema version strings) must NOT be redacted (no over-redaction).

### 2.3 No plaintext financial values in logs, anywhere

- This includes stdout, stderr, file logs, OS-level crash dumps, and any test-fixture output captured for debugging.
- WAL / SHM / temp / journal files in the vault directory must contain no plaintext financial values after any test (`personal-cfo-zxvl`).

---

## 3. Performance budgets

- Each feature with a user-perceivable budget asserts the budget in CI on the golden fixture vault.
- Defaults (override per-feature in the bead AC if unrealistic):
  - Tauri command round-trip latency: P50 < 50ms, P99 < 250ms.
  - Forecast Layer-1 (1-year horizon, 200 monthly bills, 26 paychecks): < 50ms on M1 dev.
  - Read-model rebuild on 50k transaction fixture: < 2s.
  - App cold start to "vault unlock screen visible": < 1s on dev hardware.
- Budgets are baseline-recorded; regressions ≥ 2× the baseline fail CI.

---

## 4. Cross-cutting test suites that must pass

A feature is not done until the relevant cross-cutting suites still pass:

- **Testing & quality strategy beads** that cover the whole project:
  - `personal-cfo-nijw` — Unit tests: money math, schedules, categorization, rules.
  - `personal-cfo-4lb5` — Property tests: cashflow invariants, date edge cases, split sums.
  - `personal-cfo-1z4o` — Security tests: IPC permissions, CSP, logging redaction.
  - `personal-cfo-c545` — Migration tests: upgrade/downgrade safety + fixture vaults.
- **Cross-cutting test suites** (15 in total) that exercise project-wide invariants:
  - Log correlation end-to-end (`-1de5`), mutation testing (`-2qxy`), long-running stability + memory leak (`-4hr7`), concurrency races at the kernel boundary (`-4p30`), redactor known-positive / known-negative corpus (`-64st`), pending→posted transition handling (`-eyhb`), fixture determinism + seeding policy (`-f650`), cancellation propagation through job runtime (`-gdsz`), multi-currency aggregation correctness (`-il6n`), injected `Clock` discipline (`-jui8`), chargebacks / refunds / reversals (`-l28f`), redactor fuzzing (`-mt6s`), filesystem / memory / read-only fault injection (`-rcma`), IANA timezone + DST edge cases (`-rp1r`), telemetry export redaction round-trip (`-ryjx`).

If a feature plausibly affects one of these surfaces and the suite passes, that's the signal — not a manual review.

---

## 5. CI gates (release-blocking)

Every PR runs:

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `pnpm typecheck` (frontend)
- `pnpm lint` (frontend)
- `pnpm test` (vitest)
- Tauri release build on macOS runner
- Logging redaction CI test (`personal-cfo-zobt`) — release-blocking
- WAL/SHM/temp-file plaintext-leak test (`personal-cfo-zxvl`) — release-blocking

All gates must be green before merge to `main`.

---

## 6. The real-data safety gate (one-time, pre-dogfooding)

Before any developer or user opens a real personal vault, all of these must pass on synthetic fixture vaults:

- Encrypted backup export (`personal-cfo-ef3` / `-1y3z`).
- Restore into a fresh app instance (`personal-cfo-au3`).
- Wrong-password failure test (covered by `personal-cfo-3ry` / `-fxym`).
- Logging redaction CI test (`personal-cfo-zobt`).
- Stock SQLite tools cannot open the encrypted database as plaintext (`personal-cfo-7igv`).
- No plaintext attachment preview artifacts outside the vault (`personal-cfo-bcj` / `-s5a4`).
- Migration rollback/repair test (`personal-cfo-c545`).
- Manual recovery instructions written and tested by a fresh-clone restore drill (`personal-cfo-7pfu`).

Bead `personal-cfo-w01i` ("Start synthetic-data dogfooding only") gates real-data use until all of the above are closed.

---

## 7. References

- Plan: `docs/planning/personal-finance-app-project-plan.md` §20 (testing pyramid), §6.6 (logging policy + redaction layer), §5.1.1 (real-data safety gate).
- Project profile: `docs/agent/PROJECT_PROFILE.md` (quality gates, real-data safety gate).
- Logging policy bead: `personal-cfo-2vs`.
- Testing strategy epic: `personal-cfo-56w`.

When this document changes, update the linked beads' notes so the trail is auditable.
