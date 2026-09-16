# Project Profile — DohFlow

This file is the project-specific contract that extends the repository-level `AGENTS.md`. Keep `AGENTS.md` universal and reusable; keep project facts, stack decisions, quality gates, and current phase constraints here.

## Project identity

- Working project name: DohFlow (pronounced "doe-flow"; ADR 0067, `docs/product/brand-direction.md`)
- Public name: DohFlow, decided 2026-09-01 (`docs/product/brand-direction.md`); the trademark, domain, and namespace checks are recorded in `docs/research/trademark-clearance-dossier.md` and `TRADEMARK.md`. The bundle identifier and machine names stay `personal-cfo` / `ai.personalcfo.desktop` (ADR 0067).
- Repository visibility: private by default. Public release requires explicit user approval plus security hardening, stable core functionality, complete public-facing docs, and external review.
- Product description: local-first personal CFO for households.
- Product thesis: build a secure, auditable, forecast-oriented household finance desktop app that answers: “How much cash will this household probably have in the future, what assumptions drive that forecast, and what risks are emerging before they become painful?”
- Product wedge: Future Cash, a forward-looking daily household liquidity forecast.
- Primary platform: macOS desktop first.
- Future platform intent: structure the codebase for later Linux and Windows support without weakening the macOS security model.
- Current phase (as of 2026-07-09): **shipping + continuous dogfooding**. The Week-8 first-playable target below was met long ago; the app is built, installed as an unsigned `/Applications/DohFlow.app` dogfooding build (`scripts/update-app.sh`), and run on real data (the real-data safety gate has passed). Work now flows from the maintainer's dogfooding review: each finding becomes a bead, then ships as one focused, adversarially-reviewed PR per turn (AGENTS.md §15). For the freshest cross-session state — what just shipped, the working rhythm, and gotchas — read the `bd` memory `handoff-2026-07-09-dogfooding-feedback-phase` (`\bd memories handoff`). The Week-8/phase framing further down is retained as historical roadmap context and still informs bead priorities.
- Execution horizon: originally a phase-gated roadmap with a hard Week 8 first-playable target and a longer private-beta/open-source path; now past first-playable and iterating via dogfooding toward MVP/beta.

## Source of truth documents

- Universal agent contract: `AGENTS.md`
- Claude wrapper: `CLAUDE.md`
- Project profile: `docs/agent/PROJECT_PROFILE.md`
- Main planning document: `docs/planning/personal-finance-app-project-plan.md`
- Tauri/Rust/React finance appendix: `docs/agent/TAURI_RUST_REACT_FINANCE_APPENDIX.md`
- Product docs directory: `docs/product/`
- Architecture docs directory: `docs/architecture/`
- Security docs directory: `docs/security/`
- ADR directory: `docs/adr/`
- Demo vault for screenshots and walkthroughs: `docs/agent/demo-vault.md`
- Beads task graph: `.beads/`, once initialized.

Do not load the full planning document into every routine coding session. Use Beads as executable task memory after plan-to-beads conversion. Reopen the full plan for architecture review, bead coverage checks, major scope decisions, or when a bead lacks required context.

Internal strategy drafts (monetization, pricing, legal-posture analysis) live outside the repo in `~/Downloads/` (owner's working copies) and never enter the tree; `.gitignore` blocks `docs/planning/*.DRAFT.md` as a backstop (`personal-cfo-o7xlf`).

## Product principles

Rank these in order when making tradeoffs:

1. Security: encrypted local-first data, minimal cloud dependency, no plaintext financial secrets, no accidental telemetry, and explicit consent before anything leaves the device.
2. Reliability: forecasts must be explainable and backtested; manual entry and import must continue working even if connectors fail.
3. Usability: simple daily check-in experience with progressive disclosure for advanced features.
4. Flexibility: support manual accounts, imports, overrides, categories, household profiles, scenarios, documents, and optional connectors over time.
5. Transparency: forecasts, categorizations, and agent reports must show assumptions and evidence.

Non-negotiables:

- The app must be useful in manual-only mode.
- Financial data must be encrypted at rest before the first useful feature ships.
- A connected account is an enhancement, not a dependency.
- Forecast rows must be inspectable and editable through explicit assumption records.
- Users must be able to override categories, recurring events, assumptions, and forecast items.
- AI must not make transactions, payments, transfers, trades, or destructive edits.
- Open-source release must not happen until the security model is documented, tested, and reviewed.

## Stack

- Desktop shell: Tauri v2.
- Trusted backend/core: Rust Finance Kernel.
- Frontend: React + TypeScript strict mode.
- Frontend build tooling: Vite.
- Frontend routing: TanStack Router or equivalent typed router.
- Frontend server/cache state: TanStack Query or equivalent.
- Frontend validation: Zod plus authoritative Rust-side validation.
- UI state: small, explicit Zustand/Jotai-style state only where needed.
- Styling: Tailwind CSS plus accessible component primitives, after license review.
- Tables: AG Grid Community or TanStack Table after license/performance review.
- Charts: prototype Recharts, Nivo, or ECharts against realistic forecast/ledger data before committing.
- Rust async: Tokio where needed; keep database writes controlled.
- Database: SQLCipher-encrypted SQLite.
- SQLite access: start with `rusqlite` plus SQLCipher bindings unless a spike proves another layer is safer.
- Persistence model: normalized relational ledger/domain tables as canonical financial state, plus immutable operation/audit log for command history, idempotency, provenance, read-model rebuilds, and future sync readiness.
- Attachments: encrypted attachment store outside the database, with encrypted metadata in the vault.
- KDF: Argon2id with per-vault salt, calibrated/versioned parameters, and rekey support.
- Platform secrets: macOS Keychain for small secrets such as wrapped vault unlock material, connector tokens where allowed, and BYOK API keys only when explicitly saved by the user.
- Biometric unlock: optional Touch ID after password unlock; password remains the primary unlock path.
- AI: none required for core app. Later AI features must be scoped report generators brokered by Rust, read-only, schema-validated, evidence-citing, and cost-capped.
- Connectors: no connector dependency for MVP. Later connector work must keep provider secrets out of the desktop app and route through user-token flows, self-hosted relay, or managed relay as explicitly designed.

## Package manager policy

- Package manager: TBD until the Tauri app is scaffolded.
- Recommendation: use `pnpm` for the frontend/workspace unless there is a specific reason to use npm, Bun, or Yarn.
- Once the package manager is chosen, commit exactly one frontend lockfile and never introduce competing lockfiles.
- Rust uses Cargo and the repository should use a Rust workspace for shared crates.
- Agents must detect the package manager from the committed lockfile or this profile before running install/build/test commands.

## Architecture boundaries

- Rust Finance Kernel is the authoritative domain layer.
- React/TypeScript is presentation and interaction only.
- Frontend must never directly access the database, vault keys, API keys, connector tokens, raw provider credentials, unrestricted filesystem paths, or financial write primitives.
- All frontend-to-backend calls go through explicit typed Tauri commands.
- Every mutating command validates permissions, vault state, schema version, idempotency key, and domain invariants in Rust.
- Importers, connector adapters, document extractors, and AI agents submit staged candidates, typed commands, or report proposals. They do not write committed financial records directly.
- Financial history is append-friendly. Use reversals, amendments, superseding records, and audit history instead of invisible destructive mutation.
- Read models are rebuildable from canonical tables plus operation/provenance records.
- Materialized read models should power dashboard, ledger, search, forecast, and reports.
- Tauri capabilities are deny-by-default and should be window/webview-specific.
- The main app window is local-only and cannot navigate to arbitrary remote URLs in production.
- Untrusted documents, imported text, SVG/HTML-like content, transaction descriptions, merchant names, connector payloads, and LLM output are hostile input.
- Untrusted document preview and agent report surfaces must not share broad write-capable IPC with the main app.

## MVP strategy

The first product is a thin vertical slice, not the full finance operating system.

MVP loop:

```text
vault -> accounts -> balances/imports -> ledger -> income/bills -> deterministic forecast
      -> forecast explanation -> reconciliation/backtest -> daily check-in
```

A feature is MVP-eligible only if it improves one of these outcomes:

- time to first useful cash forecast,
- correctness of starting balances or scheduled obligations,
- forecast explainability,
- forecast calibration/backtesting,
- safe backup/restore/dogfooding.

Explicitly defer until after the manual app is useful:

- automated bank connectors,
- cloud AI,
- full document extraction,
- investments and advanced net-worth tooling,
- advanced debt tooling,
- hosted services,
- multi-account household permissions,
- public release.

## Week 8 first-playable target

By Week 8, the developer should be using the app daily for personal cash forecasting. The app may be ugly, but it must be correct, useful, and safe enough to dogfood after the real-data safety gate passes.

Week 8 minimum scope:

- Tauri app opens.
- Encrypted vault can be created.
- Password unlock works.
- Manual accounts with balances exist.
- Manual transaction entry exists.
- Manual recurring income schedule supports one salary-style type.
- Manual recurring bills support a flat list.
- Deterministic Future Cash ledger computes daily balances from starting cash plus income minus bills.
- Minimal dashboard shows liquid cash, upcoming bills, upcoming income, and a 30-day forecast line.
- Encrypted backup to file works.

Do not add these before Week 8 unless they are needed to satisfy the safety gate or unblock the first playable:

- CSV/OFX/QFX/QIF import,
- Touch ID,
- broad category taxonomy,
- user rules,
- reconciliation,
- attachments,
- split transactions,
- merchant normalization,
- confidence bands,
- risk flags.

If Week 8 cannot be met, cut scope further rather than extending the deadline.

## Real-data safety gate

Do not use a real personal-finance vault until all of these pass on synthetic fixture vaults:

- encrypted backup export,
- restore into a fresh app instance,
- wrong-password failure test,
- log redaction test,
- normal SQLite tools cannot open the encrypted database as plaintext,
- no plaintext attachment preview artifacts outside the vault,
- basic migration rollback/repair test,
- manual recovery instructions written and tested.

Never commit real financial data, bank exports, account numbers, screenshots with private values, local vault files, database files, logs with sensitive values, `.env` files, provider tokens, API keys, secrets, or crash dumps.

## Initial schema boundary

The full data model in the plan is the target model, not the first migration set. Do not create production tables before the owning feature passes its phase gate.

MVP 0.5 / Week 8 schema profile:

- `vault_metadata`
- `households`
- `profiles`
- `accounts`
- `ledger_accounts`
- `ledger_transactions`
- `ledger_postings`
- `categories`
- `recurring_events`
- `income_sources`
- `forecast_runs`
- `forecast_rows`
- `operation_log`
- `command_idempotency_keys`
- `projection_cursors`
- `account_balance_read_model`
- `dashboard_summary_read_model`
- `cash_projection_read_model`

MVP 1 adds:

- `source_batches`
- `source_records`
- `staged_transactions`
- `dedupe_decisions`
- `provenance_links`
- `split_groups`
- `split_lines`
- `reconciliation_sessions`
- `balance_observations`

## Repository structure target

Start small, but grow toward this structure:

```text
apps/
  desktop/
    src/                  # React frontend
    src-tauri/            # Tauri/Rust app shell
crates/
  core-money/
  core-ledger/
  forecast-engine/
  categorization/
  importers/
  vault-crypto/
  agent-runtime/
  connector-core/
services/
  connector-relay/        # optional later, not MVP
docs/
  agent/
  planning/
  product/
  architecture/
  security/
  adr/
  user-guide/
tests/
  fixtures/
  synthetic-data/
  forecast-backtests/
scripts/
  generate-sample-data/
  security-checks/
  release/
.github/
  workflows/
```

Do not create empty crates/directories just to match the target layout. Create them when a bead requires them.

## Required early docs

Create these early, preferably through Beads and ADRs:

- `docs/product/vision.md`
- `docs/product/personas.md`
- `docs/product/mvp-scope.md`
- `docs/product/non-goals.md`
- `docs/architecture/system-overview.md`
- `docs/architecture/trust-boundaries.md`
- `docs/architecture/data-model.md`
- `docs/architecture/forecast-engine.md`
- `docs/architecture/operation-log-and-projections.md`
- `docs/architecture/performance-budgets.md`
- `docs/architecture/headless-cli.md`
- `docs/security/threat-model.md`
- `docs/security/encryption-design.md`
- `docs/security/ai-agent-safety.md`
- `docs/security/logging-policy.md`
- `docs/security/release-gates.md`
- `docs/adr/0001-tauri-rust-react.md`
- `docs/adr/0002-local-encrypted-vault.md`
- `docs/adr/0003-finance-kernel-boundary.md`
- `docs/adr/0011-hybrid-ledger-operation-log.md`
- `docs/adr/0004-connector-relay-boundary.md`

## Quality gates

Before meaningful code commits, run the relevant subset of these checks. If a command does not exist yet, create a bead to add it.

Rust:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Frontend, after package manager selection:

```bash
<pm> run typecheck
<pm> run lint
<pm> test
<pm> run build
```

Tauri, once scaffolded:

```bash
<pm> run tauri build
```

Security and release hardening, added progressively:

- dependency scanning for Rust and frontend,
- secret scanning with gitleaks or trufflehog,
- Semgrep or CodeQL,
- license scanning,
- SBOM generation,
- Tauri capability audit,
- CSP audit,
- production config audit for devtools, remote assets, global Tauri exposure, and IPC allowlist drift,
- golden-vault tests,
- migration tests,
- backup/restore tests,
- log-redaction tests,
- parser fuzz tests.

## Git and GitHub policy

- GitHub repositories are private by default unless the user explicitly says otherwise.
- Default branch is `main`.
- An initial bootstrap commit on `main` is acceptable when creating a brand-new empty repository.
- After bootstrap, all agent work happens on branches.
- Branch naming: `agent/<bead-id>-<short-slug>` when tied to Beads; otherwise `agent/<short-task-slug>`.
- PRs target `main`.
- Substantial work requires a PR before shipping.
- Agents must not merge PRs without explicit user approval.
- Agents must not force-push, reset, clean, delete branches, or discard changes without explicit approval under the destructive-action policy.
- Include the Bead ID in commit messages and PR descriptions when applicable.
- Push checkpoints before ending a session when safe.

### Review evidence (manual-CI project, pre-flip)

`ci.yml` (`personal-cfo-fkt5.7`) already carries real `pull_request`/`push`/
`schedule` triggers — they ship inside the go-live snapshot rather than being
added only after the flip — but every job is guarded to run only on a manual
`workflow_dispatch` or once `github.event.repository.private == false`. Until
this repository is flipped public, that guard means every job still no-ops on
a real PR, so there is **no automatic status check on a PR today** and no
green tick to rely on. Consequently, for now:

- The implementation agent runs the applicable gates above locally and records
  the exact commands and results **in the PR body**.
- The reviewer independently **reruns** the relevant gates against the candidate
  SHA whenever practical and records its own results in a **separate structured
  review comment**, not by editing the PR body. Results pasted by the
  implementation agent are a claim, not evidence.
- Any gate that could not be run is named, with the reason, by whichever agent
  could not run it.

**After the flip**, this whole section becomes historical: CI runs
automatically on every PR pushed against the (now-public) `dohflow/dohflow`,
so a real status check exists again and both local re-runs above become a
supplement to it rather than the only evidence. `personal-cfo-fkt5.9`'s
branch-protection ruleset makes the `rust`/`frontend`/`ipc-codegen`/
`security-scan`/`shell-scripts` job names required checks at that point.
- A direct push to the default branch is refused by a local pre-push guard
  (AGENTS.md §16). It is a safety bumper, not a security boundary.

## Beads policy

Use Beads as the task source of truth after initialization.

Initial plan-to-beads strategy:

1. Initialize Beads in the repo.
2. Convert the plan into actual Beads with `\bd`, not pseudo-beads in Markdown.
3. Preserve reasoning, constraints, dependencies, test obligations, and acceptance criteria in the beads.
4. Start with a complete phase map, but prioritize detailed execution beads for Phase 0, Week 8 first-playable, and MVP 1.
5. Keep later phases as lower-priority epics/backlog beads until closer to execution.
6. Run at least one bead review/polishing pass before implementation.
7. Use `\bd ready` (or `bv --robot-insights` for a graph-level view) to choose work.

Bead conventions:

- P0: safety-critical/blocking work.
- P1: Week 8 first-playable foundation and hard gates.
- P2: MVP 1 manual/import functionality.
- P3: later MVP 2/3 features.
- P4: public release, polish, future bets.
- Types: `epic`, `feature`, `task`, `bug`, `docs`, `question`.
- Labels should include domain areas such as `security`, `vault`, `architecture`, `tauri`, `rust`, `frontend`, `ledger`, `forecast`, `testing`, `docs`, `import`, `connector`, `agent-ai`, `release`.

After Beads changes:

```bash
\bd dolt commit
\bd export --include-memories -o .beads/issues.jsonl
```

There is no `bd sync` command. `dolt.auto-commit` is `on`, so `\bd dolt commit` is
usually a no-op; the export refreshes the LOCAL `.beads/issues.jsonl` file that
`scripts/backup-beads.sh` mirrors off-machine on the next push. Since ADR 0064's
2026-09-08 addendum, `.beads/` (the whole directory) is gitignored and untracked
in this repository — there is no `git add` step, and none is needed; the Dolt
database is the source of truth, backed up by the JSONL mirror and the Dolt
remote, not by a git commit in this repo.

`--include-memories` is **required**. As of bd 1.2.2 a plain `bd export` omits the
`bd remember` memories, which would silently shrink the export from 1465
records to 1320. See `AGENTS.md` §9.0.

## Multi-agent coordination policy

Single-agent work is the default, and there is **no agent mail system installed**.
MCP Agent Mail was removed on 2026-09-06 after five months of zero use; do not
reinstall it (its installer also drags in `beads_rust` and aliases `bd` to it).

Coordination, if a second agent ever runs here, is the bead graph plus git
branches: claim with `\bd update <id> --claim`, record findings with
`\bd note <id>`, one branch per bead. See `AGENTS.md` §10 — including the caveat
that the bead graph has no cross-machine sync today.

## Initial implementation priorities

Recommended first branch after bootstrap: `agent/bootstrap-beads`.

Recommended first implementation branch after bead polishing: `agent/<first-ready-bead>-foundation-scaffold`.

Do this order:

1. Add scaffold files: `AGENTS.md`, `CLAUDE.md`, this profile, `.gitignore`, `.env.example`, basic docs directories.
2. Commit and push private repo.
3. Initialize Beads.
4. Convert plan to beads.
5. Review/polish beads.
6. Create ADRs for architecture, vault, trust boundary, Finance Kernel, and hybrid persistence.
7. Scaffold Tauri v2 + React/TypeScript + Rust app.
8. Add CI baseline.
9. Build vault creation/unlock spike on synthetic data only.
10. Build deterministic Future Cash vertical slice.

## Current command placeholders

These must be filled after scaffolding.

```bash
# install frontend deps
TBD

# run desktop dev app
TBD

# format all
TBD

# lint all
TBD

# typecheck frontend
TBD

# test Rust
cargo test --workspace

# test frontend
TBD

# build desktop app
TBD
```

## Human explanation policy for this project

Because this is a security-sensitive personal finance app and the user is not a software engineer by trade, agents must explain major structural changes in plain language before implementing them. For deletions, migrations, vault/encryption changes, schema changes, build-system changes, dependency changes, Git history changes, or security boundary changes, explain:

- what is changing,
- why it is needed,
- what could go wrong,
- how it will be tested,
- how to revert or recover if needed.
